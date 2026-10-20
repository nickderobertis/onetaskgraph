//! A sandboxed host for the journeys that drive the binary.
//!
//! Everything the configuration layer reads is real here — a real directory tree, real
//! files, a real process environment — because that is the layer under test. What the
//! sandbox does is put all of it somewhere this run owns, so a journey cannot read the
//! developer's own `~/.config/onetaskgraph` and cannot be changed by it.

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use serde_json::{Value, json};
use tempfile::TempDir;

/// The two boundaries every source-backed journey must cross.
#[derive(Clone, Copy, Debug)]
pub enum SourceBoundary {
    /// The engine builds the source in its own process.
    Direct,
    /// The engine reaches the same source through the shipped stdio host.
    Subprocess,
}

pub const SOURCE_BOUNDARIES: [SourceBoundary; 2] =
    [SourceBoundary::Direct, SourceBoundary::Subprocess];

impl SourceBoundary {
    /// One configured source, preserving the source's own plugin and configuration.
    pub fn source(self, plugin: &str, config: Value) -> Value {
        self.source_with_secrets(plugin, config, &[])
    }

    /// The same, for a source that needs a named credential to reach its backend.
    ///
    /// §3.1 forbids a plugin reading a credential out of its own environment, and the
    /// stdio host clears the child's, so a hosted source reaches only the variables its
    /// configuration *names*. An in-process source reads the engine's own resolver and
    /// needs no such list, which is why this is one method rather than two shapes of
    /// fixture: the journey says which credential the source behind the boundary needs,
    /// and each boundary does whatever getting it there takes.
    pub fn source_with_secrets(self, plugin: &str, config: Value, secrets: &[&str]) -> Value {
        match self {
            Self::Direct => json!({"plugin": plugin, "config": config}),
            Self::Subprocess => json!({
                "plugin": "subprocess",
                "config": {
                    "command": crate::source_binary(),
                    "secrets": secrets,
                    "settings": {"kind": plugin, "config": config},
                },
            }),
        }
    }

    /// The configuration path to a field in the source behind this boundary.
    pub fn config_path(self, field: &str) -> String {
        match self {
            Self::Direct => format!("sources.work.config.{field}"),
            Self::Subprocess => format!("sources.work.config.settings.config.{field}"),
        }
    }

    /// The environment spelling of [`Self::config_path`].
    pub fn config_variable(self, field: &str) -> String {
        format!(
            "ONETASKGRAPH_{}",
            self.config_path(field)
                .split('.')
                .map(|segment| segment.to_ascii_uppercase().replace('-', "_"))
                .collect::<Vec<_>>()
                .join("__")
        )
    }
}

/// The credential variables a sandboxed run must never inherit from its host.
///
/// Several journeys turn on a configured source *not* being buildable — a `linear` block
/// with no `api_key_env` falls back to `LINEAR_API_KEY` — and with that variable really
/// set those sources build, answer, and reach a real workspace.
///
/// **This is a list of what `.github/workflows/ci.yml` exports into this suite's own
/// process, not of what the plugins read.** Only what that workflow sets can be inherited
/// here, and a credential no journey turns on needs no entry — so a credential exported
/// there and missing here fails the journeys that depend on its absence, which is the
/// whole of the harm and the whole of the gate.
///
/// A journey that wants one set says so with `.env`, which lands after this removal and
/// wins. `crates/onetaskgraph-e2e/tests/ambient_credentials.rs` drives it with the variables
/// set in the parent.
// llmlint: ignore[contracts_have_one_source_or_a_drift_gate] These names are what one leg of the CI workflow exports, not a copy of the plugins' defaults, and there is nothing to derive them from: the schema `onetaskgraph schema` emits carries no default for `api_key_env` or `token_env`, and a `github-projects` source with an empty block is refused over `owner` before it names its token. The gate is the one stated above, and it is the one that fired.
pub const AMBIENT_CREDENTIALS: [&str; 2] = ["LINEAR_API_KEY", "GH_PROJECTS_TOKEN"];

/// A temporary host: a project tree to run in, and a configuration home over it.
pub struct Sandbox {
    /// Held for its drop, which removes the tree. Every path comes from `root` below.
    _directory: TempDir,
    /// The same tree, named the way the child process will name it.
    root: PathBuf,
}

/// A fresh, empty host, as [`Sandbox::new`] makes one.
impl Default for Sandbox {
    fn default() -> Self {
        Self::new()
    }
}

impl Sandbox {
    /// An empty host — no documents, no credentials file.
    pub fn new() -> Self {
        let directory = TempDir::new().expect("a temporary directory");
        let root = resolved(directory.path());
        std::fs::create_dir_all(root.join("project")).expect("the project tree");
        std::fs::create_dir_all(root.join("home/onetaskgraph")).expect("the config home");
        Self {
            _directory: directory,
            root,
        }
    }

    /// The directory a command runs in.
    pub fn project(&self) -> PathBuf {
        self.root.join("project")
    }

    /// The configuration home, as `XDG_CONFIG_HOME`.
    pub fn config_home(&self) -> PathBuf {
        self.root.join("home")
    }

    /// Write `onetaskgraph.yaml` at the top of the project tree.
    pub fn project_document(&self, text: &str) -> PathBuf {
        write(self.project().join("onetaskgraph.yaml"), text)
    }

    /// Write the user-level document under the configuration home.
    pub fn user_document(&self, text: &str) -> PathBuf {
        write(self.config_home().join("onetaskgraph/config.yaml"), text)
    }

    /// Write the credentials file under the configuration home.
    pub fn secrets_file(&self, text: &str) -> PathBuf {
        write(self.config_home().join("onetaskgraph/secrets.env"), text)
    }

    /// Create a directory under the project tree and return it.
    pub fn subdirectory(&self, relative: &str) -> PathBuf {
        let path = self.project().join(relative);
        std::fs::create_dir_all(&path).expect("a directory under the project tree");
        path
    }

    /// The binary, running in the project tree with this sandbox's configuration home.
    pub fn command(&self) -> Command {
        self.command_in(&self.project())
    }

    /// The binary, running in `directory` with this sandbox's configuration home.
    ///
    /// The ambient `ONETASKGRAPH_` variables are removed rather than the whole
    /// environment cleared: a coverage run hands the child `LLVM_PROFILE_FILE`, and
    /// clearing it would silently stop attributing this binary's lines to its crate.
    /// [`AMBIENT_CREDENTIALS`] goes with them, for the reason recorded there.
    pub fn command_in(&self, directory: &Path) -> Command {
        let mut command = Command::from_std(self.subprocess(crate::binary()));
        command.current_dir(directory);
        command
    }

    /// A raw subprocess for pipe and shell journeys, with the same isolation as `command`.
    pub fn subprocess(&self, executable: impl AsRef<std::ffi::OsStr>) -> std::process::Command {
        let mut command = std::process::Command::new(executable);
        for (name, _) in std::env::vars() {
            if name.starts_with("ONETASKGRAPH_") {
                command.env_remove(name);
            }
        }
        for name in AMBIENT_CREDENTIALS {
            command.env_remove(name);
        }
        // A run is on the real clock unless a journey puts it on a simulated one.
        command.env_remove(onetaskgraph_plugin_api::SIMULATED_CLOCK_VARIABLE);
        command
            .current_dir(self.project())
            .env("XDG_CONFIG_HOME", self.config_home())
            .env_remove("HOME");
        command
    }

    /// Put a directory where a file belongs, under the configuration home.
    ///
    /// A file this run cannot read, without needing a permission a test user may not
    /// be able to set and without a `chmod` that means nothing on Windows: a directory
    /// exists as far as a read is concerned, and reading one fails everywhere.
    pub fn unreadable(&self, relative: &str) -> PathBuf {
        let path = self.config_home().join(relative);
        std::fs::create_dir_all(&path).expect("a directory in the file's place");
        path
    }
}

/// The temporary root under the name the binary will report for it.
///
/// A journey that asserts on the document's path compares two names for one file: the
/// one this sandbox built and the one the child derived from its own `current_dir()`.
/// The operating system resolves symbolic links out of the second, so on a host whose
/// temporary directory is reached through one — macOS, where `/var` is a link to
/// `/private/var` — the two names differ and every such assertion fails there and
/// nowhere else. Resolving here makes the sandbox's name for the tree the child's name
/// for it, so the comparison is about the layer under test rather than about the host.
#[cfg(not(windows))]
fn resolved(path: &Path) -> PathBuf {
    path.canonicalize().expect("the temporary root resolves")
}

/// On Windows, the name `TempDir` returns is already the one the child reports.
///
/// There is no symlinked temporary root to resolve there, and `canonicalize` would
/// answer with a `\\?\` verbatim path — a third name for the tree, carried by neither
/// side of the comparison above.
#[cfg(windows)]
fn resolved(path: &Path) -> PathBuf {
    path.to_path_buf()
}

/// Write `text` to `path`, creating what it needs, and return the path.
fn write(path: PathBuf, text: &str) -> PathBuf {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("the directory holding the file");
    }
    std::fs::write(&path, text).expect("the file is written");
    path
}

/// A configuration with one `in-memory` source called `work`, on either boundary.
pub fn one_source(boundary: SourceBoundary) -> String {
    serde_json::to_string(&json!({
        "sources": {
            "work": boundary.source("in-memory", json!({
                "capabilities": {"max_page_size": 20}
            }))
        }
    }))
    .expect("a one-source document")
}

/// Windows' bound, in UTF-16 units, on the command line `CreateProcessW` takes and on any one
/// variable of a child's environment, as Microsoft documents both — `CreateProcessW`'s
/// `lpCommandLine` and "Environment Variables" — and the block holding them all has no bound of
/// its own. Its gate is the Windows lane, which spawns every command this measures: were it set
/// past what Windows takes, a spawn it passes would fail there with `os error 206`.
pub const WINDOWS_SPAWN_LIMIT: usize = 32_767;

/// What spawning `command` hands a Windows child, in UTF-16 units: the command line at its
/// longest quoting, its longest environment variable as `NAME=value`, and the whole block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpawnLengths {
    pub command_line: usize,
    pub longest_variable: usize,
    pub environment_block: usize,
}

/// Measure what `command` would hand its child: its program and arguments, each quoted with
/// every quote and backslash escaped as the most a Windows command line can spell them, and
/// the environment this process passes on with the command's own settings and removals laid
/// over it. `Command` does not say whether `env_clear` was called on it, so a command that
/// clears its environment is measured with this process's still in it: the figure is then an
/// over-count, never an under-count, and every command this harness spawns inherits it.
#[must_use]
pub fn spawn_lengths(command: &std::process::Command) -> SpawnLengths {
    let units = |text: &std::ffi::OsStr| text.to_string_lossy().encode_utf16().count();
    let quoted = |text: &std::ffi::OsStr| {
        let escapes = text
            .to_string_lossy()
            .chars()
            .filter(|c| matches!(c, '"' | '\\'))
            .count();
        units(text) + escapes + 2
    };
    let command_line = std::iter::once(command.get_program())
        .chain(command.get_args())
        .map(|piece| quoted(piece) + 1)
        .sum::<usize>()
        .saturating_sub(1);
    let mut environment: std::collections::BTreeMap<std::ffi::OsString, std::ffi::OsString> =
        std::env::vars_os().collect();
    for (name, value) in command.get_envs() {
        match value {
            Some(value) => environment.insert(name.to_owned(), value.to_owned()),
            None => environment.remove(name),
        };
    }
    let variables: Vec<usize> = environment
        .iter()
        .map(|(name, value)| units(name) + 1 + units(value))
        .collect();
    SpawnLengths {
        command_line,
        longest_variable: variables.iter().copied().max().unwrap_or(0),
        environment_block: variables.iter().map(|variable| variable + 1).sum::<usize>() + 1,
    }
}

/// Refuse a spawn Windows would refuse with `os error 206`, naming what is too long — so a
/// journey that outgrows the bound fails on every platform rather than on Windows alone.
pub fn assert_spawnable_on_windows(command: &std::process::Command) {
    let lengths = spawn_lengths(command);
    assert!(
        lengths.command_line <= WINDOWS_SPAWN_LIMIT
            && lengths.longest_variable <= WINDOWS_SPAWN_LIMIT,
        "this spawn hands its child {lengths:?}, past the {WINDOWS_SPAWN_LIMIT} UTF-16 units \
         Windows allows a command line or one environment variable, so `spawn` fails there with \
         os error 206; next: reach the same work through fewer or shorter arguments"
    );
}

/// Standard output as text, failing the test rather than the assertion when it is not
/// UTF-8 — a binary that emitted bytes here would be a different bug entirely.
pub fn stdout(output: &std::process::Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("stdout is UTF-8")
}

/// Standard error as text.
pub fn stderr(output: &std::process::Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("stderr is UTF-8")
}

/// `value` with every `updated_at` and every `version` taken out, at any depth.
///
/// What a source's own clock says rather than what an item holds: a hosted board moves an
/// item's time, and the change token built on it, for every write it takes — the write that
/// puts an item back after a copy could not finish included — so two reads of an item a
/// command left as it found it agree on everything else, and on these only when nothing wrote
/// to it at all.
#[must_use]
pub fn unclocked(value: &Value) -> Value {
    match value {
        Value::Object(members) => Value::Object(
            members
                .iter()
                .filter(|(key, _)| key.as_str() != "updated_at" && key.as_str() != "version")
                .map(|(key, member)| (key.clone(), unclocked(member)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(unclocked).collect()),
        scalar => scalar.clone(),
    }
}
