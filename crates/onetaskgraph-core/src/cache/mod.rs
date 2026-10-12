//! The store's local cache of remote sources.
//!
//! **Opt-in, one source at a time, and keyed on nothing but the change token a source gives.**
//! A source configured with `cache.enabled` has its tasks — each one's fields, metadata,
//! comments, assets and forward edges — kept on this host, one file per task, under a directory
//! named for the remote it reads. Every process that configures the same remote shares it: the
//! command line, both SDKs and a Rust caller of the engine.
//!
//! The rule it may never break is that **it never adds a request and never serves stale data**.
//! Freshness comes only from tokens the store already holds — a caller's pin, or the version a
//! listing in the same command carried — and an entry is served only when it is at that version
//! or a later one, by the ordering of the source that spelled both. Nothing expires by age, and
//! nothing asks the source whether something changed. A part of a task the source's token does
//! not cover is never served at all.
//!
//! **Releases of different ages share one directory**, so an entry it cannot read — an entry
//! schema it does not know, bytes that do not parse, another remote's — is a miss, never a
//! failure. Each entry is written whole to a staging file and renamed into place, so a reader
//! never sees part of one, and no lock is taken.
//!
//! This is the one place the store keeps a user's work outside the plugin that owns it, and
//! only because a configuration asked it to; `AGENTS.md` names it as that invariant's one
//! exception.

mod source;

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex, MutexGuard};

use onetaskgraph_plugin_api::{
    Asset, Comment, DependencyEdge, NativeId, PartOutcome, SourceName, Task, VersionCoverage,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::config::{Config, SourceConfig};
use crate::engine::Spent;

pub(crate) use source::CachedSource;

/// The schema every entry this build writes is spelled in.
///
/// A reader that meets any other value treats the entry as a miss: this directory is shared by
/// every release on a host, so an entry a later release wrote in a shape this one does not know
/// must cost a read, never a failure or a misreading.
pub const ENTRY_SCHEMA: u32 = 1;

/// Where a source's cache lives, and what names the remote it caches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheLocation {
    /// The directory.
    pub dir: PathBuf,
    /// A digest of what names the remote — the plugin and its own identity of it.
    pub remote: String,
}

/// Where the source `config` keeps its cache, when it keeps one: `None` for a source with
/// caching off, and for one with no remote.
///
/// `home` is the directory caches default to living under — [`Config::cache_home`], or the
/// process's own when the configuration names none.
#[must_use]
pub fn location(config: &SourceConfig, home: Option<&Path>) -> Option<CacheLocation> {
    if !config.cache().enabled {
        return None;
    }
    let identity = config.plugin().plugin().remote_identity(config.config())?;
    let remote = digest(&format!("{}\n{identity}", config.plugin().as_str()));
    let dir = match &config.cache().dir {
        Some(dir) => dir.clone(),
        None => home?.join("onetaskgraph").join(&remote),
    };
    Some(CacheLocation { dir, remote })
}

/// The directory caches default to living under for a configuration that names none: the
/// process's own `$XDG_CACHE_HOME`, or `$HOME/.cache`.
#[must_use]
pub fn process_home() -> Option<PathBuf> {
    crate::config::cache_home(&crate::Environment::from_process())
}

/// Every cache a configuration names, by source, each where it lives.
#[must_use]
pub fn locations(config: &Config) -> Vec<(SourceName, CacheLocation)> {
    let home = config
        .cache_home()
        .map(Path::to_path_buf)
        .or_else(process_home);
    config
        .sources()
        .iter()
        .filter_map(|(name, source)| {
            location(source, home.as_deref()).map(|location| (name.clone(), location))
        })
        .collect()
}

/// What `cache clear` emptied: one entry per cache it reached.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CacheCleared {
    /// The caches emptied, by source, in name order.
    pub cleared: Vec<ClearedCache>,
}

/// One cache `cache clear` emptied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ClearedCache {
    /// The source whose cache it is.
    pub source: SourceName,
    /// The directory it lives in.
    // llmlint: ignore[invalid_states_unrepresentable] A filesystem path reported back as the place a cache lives; it is never parsed or acted on by a reader, and a path is not a value a narrower type describes.
    pub dir: String,
    /// How many task entries it held.
    pub entries: u64,
}

/// Empty the cache of `only`, or of every source the configuration caches.
///
/// Only what this store writes there is removed — the task entries and the board entry — so a
/// `cache.dir` pointed at a directory holding anything else loses nothing else.
///
/// # Errors
///
/// A source `only` names that the configuration does not cache, and a directory that could not
/// be emptied, are refused with what to do next.
pub fn clear(config: &Config, only: Option<&SourceName>) -> Result<CacheCleared, String> {
    let all = locations(config);
    if let Some(name) = only
        && !all.iter().any(|(source, _)| source == name)
    {
        return Err(if config.sources().contains_key(name) {
            format!(
                "source {name} keeps no cache: it has no remote, or its cache.enabled is not \
                 set; next: set sources.{name}.cache.enabled, or name a source that caches"
            )
        } else {
            format!(
                "no source named {name} is configured; next: name a configured source — \
                 `onetaskgraph config show` lists them"
            )
        });
    }
    let mut cleared = Vec::new();
    for (source, location) in all {
        if only.is_some_and(|name| *name != source) {
            continue;
        }
        let entries = SourceCache::empty(&location.dir).map_err(|error| {
            format!(
                "the cache of source {source} at {} could not be emptied: {error}; next: \
                 check that directory's permissions",
                location.dir.display()
            )
        })?;
        cleared.push(ClearedCache {
            source,
            dir: location.dir.display().to_string(),
            entries,
        });
    }
    Ok(CacheCleared { cleared })
}

/// Where one read of a caching source's task was answered from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Served {
    /// From the cache, with no request.
    Cache,
    /// From the source.
    Remote,
}

/// How one read of a caching source's task used the cache: the `cache` member of its JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CacheUse {
    /// Where it was answered from.
    pub served: Served,
    /// Why the cache could not be used at all for it — a part the source's change token does
    /// not cover, or a task that carries no token — or `null` when nothing kept it from being.
    pub bypassed: Option<String>,
}

impl CacheUse {
    fn cache() -> Self {
        Self {
            served: Served::Cache,
            bypassed: None,
        }
    }

    fn remote(bypassed: Option<String>) -> Self {
        Self {
            served: Served::Remote,
            bypassed,
        }
    }
}

/// What a write that expected a version of its task came to: the `result`, `fallback` and
/// `spent` members of its JSON.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ExpectedWrite {
    /// What each mutation the write sent came to, and the version it left the task at — for a
    /// source that reports its writes' parts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<WriteResult>,
    /// That the write had to read its target first, and why — absent when it sent only its
    /// mutations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<WriteFallback>,
    /// What the write spent at the sources that meter their own requests — the fallback's
    /// read included.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spent: Option<Spent>,
}

/// What each mutation of an expected write came to, built from the mutations' own responses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WriteResult {
    /// The task's version after the write, where the last response determines it.
    pub version: Option<String>,
    /// One per mutation sent, in the order sent.
    pub parts: Vec<WritePart>,
}

/// One mutation of an expected write.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WritePart {
    /// Which part it was, in the source's words: `fields`, `issue`, `comment`.
    pub part: String,
    /// `landed`, `rejected` with the backend's error, or `unknown`.
    #[serde(flatten)]
    pub outcome: PartOutcome,
    /// Each field it set, with the value the response shows.
    pub fields: std::collections::BTreeMap<String, Value>,
}

/// That an expected write read its target before writing, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WriteFallback {
    /// Always `true`: the write made one read.
    pub read: bool,
    /// Why it could not take what it needed from the cache.
    pub reason: String,
}

/// One task as the cache keeps it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Entry {
    /// [`ENTRY_SCHEMA`].
    pub(crate) schema: u32,
    /// The digest of the remote it is from, which a reader holds it to.
    pub(crate) remote: String,
    /// The task's own id.
    pub(crate) id: NativeId,
    /// The token it was read at: `task.version`, spelled out so a reader decides on it alone.
    pub(crate) version: Option<String>,
    /// The task.
    pub(crate) task: Task,
    /// Every comment on it, oldest first — absent when the read that wrote it did not carry
    /// them whole.
    pub(crate) comments: Option<Vec<Comment>>,
    /// Its image assets, absent until a read carried them.
    pub(crate) assets: Option<Vec<Asset>>,
    /// Its forward dependency edges, absent until a read carried them.
    pub(crate) edges: Option<Vec<DependencyEdge>>,
    /// What its source said a write expecting this version could take instead of reading.
    pub(crate) context: Option<Value>,
}

/// A board's context, as the cache keeps it.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct BoardEntry {
    schema: u32,
    remote: String,
    context: Value,
}

/// A write expected on one task at one version, while the engine runs it.
#[derive(Debug, Clone)]
pub(crate) struct Expectation {
    pub(crate) id: NativeId,
    pub(crate) version: String,
    /// Whether the source took what the cache kept, and why a read would follow if not —
    /// decided at the first call touching the task, so every read the write's own verb makes
    /// of it first is answered from what was primed too.
    pub(crate) primed: Option<(bool, String)>,
}

/// Everything one command has learnt about a caching source's tasks, dropped when it ends.
#[derive(Debug, Default)]
struct Command {
    /// Versions callers pinned reads to.
    pins: HashMap<NativeId, String>,
    /// Versions a listing in this command carried.
    listed: HashMap<NativeId, String>,
    /// How the last read of each task used the cache.
    used: HashMap<NativeId, CacheUse>,
    /// Versions read from the source in this command, which assets and edges read next are
    /// recorded under.
    read: HashMap<NativeId, String>,
    /// Entries served in this command, whose assets and edges are served with them.
    served: HashMap<NativeId, Entry>,
    /// The write being expected, while it runs.
    expected: Option<Expectation>,
    /// What each expected write came to, until the engine takes it.
    reports: HashMap<NativeId, ExpectedWrite>,
    /// The last expected write's report, until anything takes it — for a write that failed.
    last: Option<ExpectedWrite>,
}

/// One source's cache: the directory, and what the running command knows about it.
#[derive(Debug)]
pub struct SourceCache {
    dir: PathBuf,
    remote: String,
    coverage: VersionCoverage,
    command: Mutex<Command>,
}

/// Distinguishes the staging files one process writes.
static STAGED: AtomicU64 = AtomicU64::new(0);

impl SourceCache {
    /// The cache at `location`, for a source whose token covers `coverage`.
    pub(crate) fn new(location: CacheLocation, coverage: VersionCoverage) -> Arc<Self> {
        Arc::new(Self {
            dir: location.dir,
            remote: location.remote,
            coverage,
            command: Mutex::new(Command::default()),
        })
    }

    /// The directory this cache lives in.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn command(&self) -> MutexGuard<'_, Command> {
        // What a command learnt is an optimisation: a lock poisoned by a panic elsewhere is
        // read as it stands rather than failing a read over it.
        self.command
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Pin the next reads of `id` in this command to `version`.
    pub(crate) fn pin(&self, id: &NativeId, version: &str) {
        self.command().pins.insert(id.clone(), version.to_owned());
    }

    /// How the last read of `id` in this command used the cache.
    pub(crate) fn used(&self, id: &NativeId) -> Option<CacheUse> {
        self.command().used.get(id).cloned()
    }

    /// Forget everything this command learnt.
    pub(crate) fn end_command(&self) {
        *self.command() = Command::default();
    }

    /// Expect the next write of `id` to find it at `version`.
    pub(crate) fn expect(&self, id: &NativeId, version: &str) {
        let mut command = self.command();
        command.reports.remove(id);
        command.expected = Some(Expectation {
            id: id.clone(),
            version: version.to_owned(),
            primed: None,
        });
    }

    /// Stop expecting, answering with what the expected writes of `id` came to.
    pub(crate) fn take_expected(&self, id: &NativeId) -> Option<ExpectedWrite> {
        let mut command = self.command();
        if command
            .expected
            .as_ref()
            .is_some_and(|expected| expected.id == *id)
        {
            command.expected = None;
        }
        command.last = None;
        command.reports.remove(id)
    }

    /// What the last expected write came to, when nothing has taken it.
    pub(crate) fn take_last(&self) -> Option<ExpectedWrite> {
        let mut command = self.command();
        let last = command.last.take()?;
        command.reports.clear();
        Some(last)
    }

    /// Record what the expected write of `id` came to, ending the expectation: a later write
    /// of the same task in this command — an undo, a comment the copy carries — is a plain one.
    pub(crate) fn report(&self, id: &NativeId, report: ExpectedWrite) {
        let mut command = self.command();
        command.expected = None;
        command.last = Some(report.clone());
        command.reports.insert(id.clone(), report);
    }

    fn task_path(&self, id: &NativeId) -> PathBuf {
        self.dir
            .join("tasks")
            .join(format!("{}.json", digest(id.as_str())))
    }

    fn board_path(&self) -> PathBuf {
        self.dir.join("board.json")
    }

    /// The entry for `id`, or `None` when there is none this build can read.
    pub(crate) fn read(&self, id: &NativeId) -> Option<Entry> {
        let bytes = std::fs::read(self.task_path(id)).ok()?;
        let value: Value = serde_json::from_slice(&bytes).ok()?;
        if value.get("schema").and_then(Value::as_u64) != Some(u64::from(ENTRY_SCHEMA)) {
            return None;
        }
        let entry: Entry = serde_json::from_value(value).ok()?;
        (entry.remote == self.remote && entry.id == *id).then_some(entry)
    }

    /// Write `entry`, whole, in place of whatever was there.
    ///
    /// A cache that cannot be written is a cache that is not used: the read that produced the
    /// entry has its answer either way, so a failure here is swallowed rather than failing it.
    pub(crate) fn write(&self, entry: &Entry) {
        let Ok(bytes) = serde_json::to_vec(entry) else {
            return;
        };
        let _ = self.write_atomically(&self.task_path(&entry.id), &bytes);
    }

    /// Make the entry for `id` unserveable: a write whose response could not vouch for what
    /// the task now holds.
    pub(crate) fn invalidate(&self, id: &NativeId) {
        let _ = std::fs::remove_file(self.task_path(id));
        let mut command = self.command();
        command.served.remove(id);
        command.read.remove(id);
    }

    /// The board context, when one is kept.
    pub(crate) fn board(&self) -> Option<Value> {
        let bytes = std::fs::read(self.board_path()).ok()?;
        let entry: BoardEntry = serde_json::from_slice(&bytes).ok()?;
        (entry.schema == ENTRY_SCHEMA && entry.remote == self.remote).then_some(entry.context)
    }

    /// Keep `context` as the board's.
    pub(crate) fn write_board(&self, context: Value) {
        let entry = BoardEntry {
            schema: ENTRY_SCHEMA,
            remote: self.remote.clone(),
            context,
        };
        if let Ok(bytes) = serde_json::to_vec(&entry) {
            let _ = self.write_atomically(&self.board_path(), &bytes);
        }
    }

    /// Drop the board context, so the next write needing it reads it.
    pub(crate) fn drop_board(&self) {
        let _ = std::fs::remove_file(self.board_path());
    }

    /// Stage `bytes` beside `path` and rename them into place, so no reader ever opens part of
    /// them: a rename within one directory replaces the old file whole.
    fn write_atomically(&self, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        let directory = path.parent().unwrap_or(&self.dir);
        std::fs::create_dir_all(directory)?;
        let staged = directory.join(format!(
            ".staged-{}-{}",
            std::process::id(),
            STAGED.fetch_add(1, AtomicOrdering::Relaxed)
        ));
        let written = (|| {
            let mut file = std::fs::File::create(&staged)?;
            file.write_all(bytes)?;
            file.sync_all()?;
            std::fs::rename(&staged, path)
        })();
        if written.is_err() {
            let _ = std::fs::remove_file(&staged);
        }
        written
    }

    /// Remove every entry under `dir`, answering how many task entries there were.
    fn empty(dir: &Path) -> std::io::Result<u64> {
        let tasks = dir.join("tasks");
        let entries = match std::fs::read_dir(&tasks) {
            Ok(listed) => listed
                .filter_map(Result::ok)
                .filter(|entry| entry.file_name().to_string_lossy().ends_with(".json"))
                .count() as u64,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
            Err(error) => return Err(error),
        };
        match std::fs::remove_dir_all(&tasks) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        match std::fs::remove_file(dir.join("board.json")) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        Ok(entries)
    }
}

/// The lower-case hex SHA-256 of `text`.
fn digest(text: &str) -> String {
    let mut hex = String::with_capacity(64);
    for byte in Sha256::digest(text.as_bytes()) {
        // Writing to a `String` never fails.
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}
