//! A stateless onetaskgraph source over hand-authored Markdown files.
//!
//! # What this source declares, field by field
//!
//! One verdict per field of [`Capabilities`]. This source reads the whole folder before it
//! answers anything, so every predicate is applied here rather than pushed at a remote
//! service — which is what `Support::Native` means in the plugin contract: *the source
//! applies this predicate itself*, wherever it applies it. No *predicate* here is
//! unsupported, and none could be: a filter over files already read is a filter over files
//! already read. `documents` is not a predicate at all — it says whether this source has
//! documents in the first place, and this one does: a third folder beside the other two.
//!
//! *Proven* means a shared journey drives it against the real binary over this source's
//! own row in `crates/onetaskgraph/tests/e2e/fixtures.rs`, and
//! `every_row_declares_exactly_what_its_plugin_reports` is what keeps this list and
//! [`capabilities`](TaskSource::capabilities) from parting.
//!
//! | Field | Verdict |
//! | --- | --- |
//! | `projects` | **Supported and proven.** `projects/` is a folder of its own, and a task's `project:` key is what files it under one. |
//! | `documents` | **Supported and proven.** `documents/` is a folder of its own beside the other two, read on the same terms: recursively, with a file's path under it and without `.md` as its identifier. A document's front matter is a task's minus the two things a document is not — no `status` and no `depends_on` — and both are refused rather than ignored. |
//! | `comments` | **Supported and proven.** A task's comments are an optional trailing `## Comments` section of the task's own file — human-readable, full fidelity, never JSON — in exactly the shape [`COMMENTS_HEADING`] documents. The section is not the task's content, and nothing a copy writes into the file adds, changes or removes it. |
//! | `orphan_tasks` | **Supported and proven.** A task document with no `project:` key belongs to none. |
//! | `filter_by_label` | **Supported and proven,** over the `labels:` key, requiring every label asked for and excluding every label refused. |
//! | `filter_by_status` | **Supported and proven,** over `status:` through this instance's own `status_mapping`. |
//! | `search_title` | **Supported and proven,** over the `title:` key. |
//! | `search_content` | **Supported and proven,** over the document body below the front matter. |
//! | `task_dependencies` | **Supported and proven,** in both directions: the reverse read scans the folder's own `depends_on` keys, which is a read of data already in hand rather than an index. |
//! | `project_dependencies` | **Supported and proven,** in both directions, the same way. |
//! | `max_page_size` | **Supported and proven.** [`MAX_PAGE_SIZE`], the largest page one folder scan returns. |
//!
//! # Where this source's root is measured from
//!
//! [`LocalMdConfig::root`] may be relative, and which directory it is relative to is the
//! configuration layer that supplied it rather than anything this source decides: a
//! configuration document's relative root is resolved against the directory holding that
//! document before this plugin is built, and one from the environment layer or a flag
//! resolves against the process working directory. See [`DOCUMENT_RELATIVE_FIELDS`], and
//! "Relative paths in a configuration document" in `README.md` for the whole rule.
//!
//! # Where this source says an entity is
//!
//! Every task, project and document this source reports carries a `Location::Path` naming
//! the canonicalized absolute path of the file it was read from — the same path this source
//! already computes, because the configured root and every traversed path are canonicalized
//! and an identifier escaping the root is refused. That is what the location contract is
//! for on this backend: a reader holding one of these entities can print the path or read
//! the contents out for a person, knowing nothing about this plugin.
//!
//! # What this source writes in place
//!
//! Beside a copy's whole-file write, three narrow writes edit a file that is already there
//! and leave every byte they do not own as it was: a task's `status:` line, its
//! `delivered_by:` entry, and one entry of the `metadata:` block of a task, a project or a
//! document. All three replace the file through a staging file and a rename, so a reader
//! sees the record before the write or after it and never part of either. The last is also
//! verified by reading the edited text back before anything is written, and refuses rather
//! than reformats a block it cannot edit narrowly; [`STAGING_SUFFIX`] states its rules.
#![deny(missing_docs)]

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        RwLock, RwLockReadGuard, RwLockWriteGuard,
        atomic::{AtomicU64, Ordering},
    },
};

use chrono::{DateTime, Utc};
use onetaskgraph_plugin_api::{
    Capabilities, Comment, CommentBody, Cursor, DependencyEdge, DependencyEndpoint, DependencyKind,
    DependencySupport, Direction, Document, DocumentQuery, Health, ItemKind, ItemWrite, Label,
    LabelFilter, Location, MetadataKey, NativeId, NewComment, Page, PageRequest, Project,
    ProjectFilter, ProjectQuery, Repository, SecretResolver, SourceError, SourceName, SourcePlugin,
    Status, StatusCategory, Support, Task, TaskQuery, TaskRef, TaskSource, TextFields, TextQuery,
    WriteSupport,
};
use schemars::{Schema, schema_for};
use serde::{Deserialize, Serialize};

/// The registry name for this plugin.
pub const KIND: &str = "local-md";

/// The `config:` fields of this plugin whose value is a filesystem path, as dotted paths
/// into the block.
///
/// A relative value at one of these, **supplied by a configuration document**, is resolved
/// against the directory holding that document before this plugin is built; supplied
/// through the environment or a flag it keeps resolving against the process working
/// directory, because there is no document to rebase it on. The rule is stated once, for a
/// reader of either side, under "Relative paths in a configuration document" in
/// `README.md`.
///
/// Spelled here because a plugin's fields are the plugin's: this is what
/// [`SourcePlugin::document_relative_paths`] answers with, and
/// `document_relative_fields_are_fields_this_plugin_declares` in `tests/plugin.rs` holds
/// every name here to the configuration schema this plugin publishes.
// llmlint: ignore[invalid_states_unrepresentable] These are field names of this plugin's
// own `config:` block, and the type that would make a wrong one unrepresentable does not
// exist: every string is a syntactically valid field name, and what makes one *valid* is
// being a property of the schema `config_schema` publishes, which is a fact about this
// plugin rather than about a type. `document_relative_fields_are_fields_this_plugin_declares`
// in `tests/plugin.rs` is the executable check that holds every name here to that schema.
pub const DOCUMENT_RELATIVE_FIELDS: &[&str] = &["root"];
/// The largest page returned by a folder scan.
pub const MAX_PAGE_SIZE: u32 = 200;

/// Configuration for a Markdown folder source.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LocalMdConfig {
    /// Folder containing `tasks/`, `projects/` and `documents/`.
    pub root: PathBuf,
    /// Case-insensitive source status to normalized-category mapping.
    #[serde(default = "default_statuses")]
    pub status_mapping: BTreeMap<String, StatusCategory>,
}

/// The mapping a source with no `status_mapping:` of its own reads statuses through.
///
/// **Every normalized category's own canonical word classifies as that category**, so a
/// task that persists the word onetaskgraph itself prints — `in-progress` as readily as
/// `queued` or `done` — reads back as what it says rather than as unknown. That is a rule
/// about the whole vocabulary rather than a list somebody keeps in step:
/// `every_normalized_category_word_reads_back_as_itself` in `tests/status_mapping.rs`
/// drives it off [`StatusCategory`]'s own variants, so a category added later with no
/// word here fails there.
///
/// The display aliases sit beside those words rather than instead of them: `in progress`
/// and `doing` for `in-progress`, `canceled` for `cancelled`.
fn default_statuses() -> BTreeMap<String, StatusCategory> {
    [
        ("draft", StatusCategory::Draft),
        ("backlog", StatusCategory::Backlog),
        ("todo", StatusCategory::Todo),
        ("queued", StatusCategory::Queued),
        ("in progress", StatusCategory::InProgress),
        ("in-progress", StatusCategory::InProgress),
        ("doing", StatusCategory::InProgress),
        ("done", StatusCategory::Done),
        ("cancelled", StatusCategory::Cancelled),
        ("canceled", StatusCategory::Cancelled),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v))
    .collect()
}

/// Factory for [`LocalMdSource`].
#[derive(Debug, Clone, Copy, Default)]
pub struct Plugin;

impl SourcePlugin for Plugin {
    fn kind(&self) -> &'static str {
        KIND
    }
    fn config_schema(&self) -> Schema {
        schema_for!(LocalMdConfig)
    }
    fn document_relative_paths(&self) -> &'static [&'static str] {
        DOCUMENT_RELATIVE_FIELDS
    }
    fn build(
        &self,
        name: &SourceName,
        config: &serde_json::Value,
        _secrets: &dyn SecretResolver,
    ) -> Result<Box<dyn TaskSource>, SourceError> {
        let config: LocalMdConfig =
            serde_json::from_value(config.clone()).map_err(|e| SourceError::Config {
                message: format!("source {name}: {e}"),
            })?;
        LocalMdSource::new(config)
            .map(|s| Box::new(s.named(name.clone())) as Box<dyn TaskSource>)
            .map_err(|e| match e {
                SourceError::Config { message } => SourceError::Config {
                    message: format!("source {name}: {message}"),
                },
                other => other,
            })
    }
}

/// A source which re-scans its canonical root for every request.
#[derive(Debug, Clone)]
pub struct LocalMdSource {
    root: PathBuf,
    /// The name this source's configuration gave it, once it is known.
    ///
    /// What tells `T-1` and `work:T-1` apart as one task in a `delivers` list. A source built
    /// without one — straight from [`LocalMdSource::new`] — recognises only a bare entry as
    /// naming a task of its own.
    name: Option<SourceName>,
    statuses: BTreeMap<String, StatusCategory>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FrontMatter {
    title: Option<String>,
    #[serde(default = "default_status")]
    status: String,
    #[serde(default)]
    labels: Vec<LabelInput>,
    project: FiledUnder,
    #[serde(default)]
    depends_on: Vec<Dependency>,
    // llmlint: ignore[invalid_states_unrepresentable, boundary_inputs_validated] `Task::url` and `Project::url` are frozen as `Option<String>` in the plugin contract, which permits source-native URL-like values; parsing here would narrow that approved boundary and is the contract owner's decision.
    url: Option<String>,
    #[serde(default)]
    metadata: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    repositories: Vec<Repository>,
    /// The tasks this one delivers, read as JSON rather than as strings so an entry that is
    /// not a task id is refused naming it rather than failing the whole front matter in
    /// serde's words.
    delivers: Option<serde_json::Value>,
    /// Every task that delivers this one, read on the terms `delivers` is.
    delivered_by: Option<serde_json::Value>,
}
/// What a front matter's `project:` key is read into, by every kind of record and by the
/// scoped read's look at that key alone — one type, so what the two accept cannot part. It is
/// the contract's own project id, which a record carries through unchanged.
type FiledUnder = Option<NativeId>;

/// The one key a scoped read looks at before it parses a record, read into the type the
/// record's own parse reads it into and ignoring every other key.
#[derive(Deserialize)]
struct Filing {
    #[serde(default)]
    project: FiledUnder,
}

fn default_status() -> String {
    "backlog".to_owned()
}

/// The front-matter keys every kind of file in this source carries.
///
/// A plain struct rather than a `#[serde(flatten)]` member of [`FrontMatter`] and
/// [`DocumentFrontMatter`]: both are `deny_unknown_fields`, serde cannot deny an unknown
/// field beside a flattened one, and denying them is exactly what makes a `status:` under
/// `documents/` a refusal rather than a shrug.
struct SharedFront {
    title: Option<String>,
    labels: Vec<LabelInput>,
    project: FiledUnder,
    url: Option<String>,
    metadata: BTreeMap<String, serde_json::Value>,
    repositories: Vec<Repository>,
}

impl FrontMatter {
    /// This front matter split into what every kind carries, and what only work does.
    fn split(self) -> (SharedFront, String, Vec<Dependency>, Delivery) {
        (
            SharedFront {
                title: self.title,
                labels: self.labels,
                project: self.project,
                url: self.url,
                metadata: self.metadata,
                repositories: self.repositories,
            },
            self.status,
            self.depends_on,
            Delivery {
                delivers: self.delivers,
                delivered_by: self.delivered_by,
            },
        )
    }
}

/// The two task lists a front matter holds, before they are read as task ids.
struct Delivery {
    delivers: Option<serde_json::Value>,
    delivered_by: Option<serde_json::Value>,
}

impl From<DocumentFrontMatter> for SharedFront {
    fn from(front: DocumentFrontMatter) -> Self {
        Self {
            title: front.title,
            labels: front.labels,
            project: front.project,
            url: front.url,
            metadata: front.metadata,
            repositories: front.repositories,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Dependency {
    // llmlint: ignore[invalid_states_unrepresentable] `NativeId` is deliberately an opaque, unvalidated string in the frozen plugin contract; this input shape preserves that contract until conversion.
    Id(String),
    Detailed {
        // llmlint: ignore[invalid_states_unrepresentable] `NativeId` is deliberately an opaque, unvalidated string in the frozen plugin contract; this input shape preserves that contract until conversion.
        id: String,
        #[serde(default)]
        kind: EdgeKind,
        /// What the far end is, when it is not the same kind of item as the near one.
        item: Option<EndpointKind>,
    },
}
/// What an expanded dependency endpoint names.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum EndpointKind {
    Task,
    Project,
}
impl From<EndpointKind> for ItemKind {
    fn from(kind: EndpointKind) -> Self {
        match kind {
            EndpointKind::Task => Self::Task,
            EndpointKind::Project => Self::Project,
        }
    }
}
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum LabelInput {
    // llmlint: ignore[invalid_states_unrepresentable] A simple label supplies both the display name and the opaque `NativeId`; the frozen contract intentionally imposes no identifier grammar.
    Name(String),
    Detailed {
        // llmlint: ignore[invalid_states_unrepresentable] `NativeId` is deliberately an opaque, unvalidated string in the frozen plugin contract; this input shape preserves that contract until conversion.
        id: String,
        name: String,
        color: Option<String>,
    },
}
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum EdgeKind {
    #[default]
    Blocks,
    Related,
}

/// The front matter of a document, which is a task's minus what a document does not have.
///
/// `deny_unknown_fields` is what makes the two omissions the contract rather than a
/// convention: a `status:` or a `depends_on:` under `documents/` is refused naming the
/// key, instead of being read and quietly dropped. A document is not work, so it has no
/// place in a status filter and none in a dependency graph.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DocumentFrontMatter {
    title: Option<String>,
    #[serde(default)]
    labels: Vec<LabelInput>,
    project: FiledUnder,
    // llmlint: ignore[invalid_states_unrepresentable, boundary_inputs_validated] `Document::url` is frozen as `Option<String>` in the plugin contract, which permits source-native URL-like values; parsing here would narrow that approved boundary and is the contract owner's decision.
    url: Option<String>,
    #[serde(default)]
    metadata: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    repositories: Vec<Repository>,
}

/// One work item's Markdown file, read: a task or a project.
///
/// A document is not one of these and has no `Entry`: it carries neither the status nor
/// the edges below, so it is read straight into the contract's own [`Document`].
struct Entry {
    common: Common,
    status: Status,
    dependencies: Vec<DependencyEdge>,
    /// Empty for a project, which delivers nothing and is delivered by nothing.
    delivers: Vec<TaskRef>,
    delivered_by: Vec<TaskRef>,
}

/// What every Markdown file of this source carries, whichever folder it is in.
struct Common {
    id: NativeId,
    title: String,
    body: Option<String>,
    labels: Vec<Label>,
    project: Option<NativeId>,
    url: Option<String>,
    location: Location,
    metadata: BTreeMap<String, serde_json::Value>,
    repositories: Vec<Repository>,
}

/// Which of this source's three folders an item lives in.
///
/// The folder **is** the discriminator, for tasks, projects and documents alike; see this
/// crate's `docs/local-md.md` for why that was chosen over a metadata marker and over a
/// distinct file extension.
#[derive(Debug, Clone, Copy)]
enum Kind {
    Task,
    Project,
    Document,
}

impl Kind {
    const fn directory(self) -> &'static str {
        match self {
            Self::Task => "tasks",
            Self::Project => "projects",
            Self::Document => "documents",
        }
    }

    /// What one item of this kind is called, for a message a user has to act on.
    const fn noun(self) -> &'static str {
        match self {
            Self::Task => "task",
            Self::Project => "project",
            Self::Document => "document",
        }
    }
}

/// The two kinds this source reads a status and a dependency list for.
///
/// Separate from [`Kind`] because a document has neither: a signature taking one of these
/// cannot be handed a document, so the branch that would have to answer *what status does
/// a document have* does not exist to be answered wrongly.
#[derive(Debug, Clone, Copy)]
enum WorkKind {
    Task,
    Project,
}

impl WorkKind {
    const fn kind(self) -> Kind {
        match self {
            Self::Task => Kind::Task,
            Self::Project => Kind::Project,
        }
    }

    /// What a dependency edge of this kind points at, at both ends.
    const fn item(self) -> ItemKind {
        match self {
            Self::Task => ItemKind::Task,
            Self::Project => ItemKind::Project,
        }
    }
}

impl LocalMdSource {
    /// Canonicalize and validate a configured source root.
    pub fn new(config: LocalMdConfig) -> Result<Self, SourceError> {
        let root = fs::canonicalize(&config.root).map_err(|e| SourceError::Config {
            message: format!("cannot canonicalize root {}: {e}", config.root.display()),
        })?;
        if !root.is_dir() {
            return Err(SourceError::Config {
                message: format!("root {} is not a directory", root.display()),
            });
        }
        Ok(Self {
            root,
            name: None,
            statuses: config
                .status_mapping
                .into_iter()
                .map(|(k, v)| (k.to_lowercase(), v))
                .collect(),
        })
    }

    /// This source, knowing the name its configuration gave it.
    #[must_use]
    pub fn named(mut self, name: SourceName) -> Self {
        self.name = Some(name);
        self
    }

    fn directory(&self, kind: Kind) -> Result<PathBuf, SourceError> {
        let path = self.root.join(kind.directory());
        if !path.exists() {
            return Ok(path);
        }
        // llmlint: ignore[changed_behavior_has_e2e] `exists` immediately above followed by
        // `canonicalize` failing is a filesystem TOCTOU race; deterministically forcing that
        // exact interval requires mocking the filesystem layer, which repository tests forbid.
        let canonical = fs::canonicalize(&path).map_err(|e| SourceError::Config {
            message: format!("cannot resolve {}: {e}", path.display()),
        })?;
        if !canonical.starts_with(&self.root) {
            return Err(SourceError::Config {
                message: format!(
                    "{} escapes configured root {}",
                    path.display(),
                    self.root.display()
                ),
            });
        }
        Ok(canonical)
    }

    fn paths(&self, kind: Kind) -> Result<Vec<PathBuf>, SourceError> {
        let _replacement = replacement_reader();

        fn visit(
            root: &Path,
            dir: &Path,
            visited: &mut BTreeSet<PathBuf>,
            out: &mut Vec<PathBuf>,
        ) -> Result<(), SourceError> {
            if !dir.exists() {
                return Ok(());
            }
            if !visited.insert(dir.to_path_buf()) {
                return Err(SourceError::Config {
                    message: format!("directory cycle reaches {}", dir.display()),
                });
            }
            let entries = match fs::read_dir(dir) {
                // A folder removed since it was listed holds nothing to list.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                entries => entries.map_err(|e| SourceError::Unavailable {
                    message: format!("cannot read {}: {e}", dir.display()),
                })?,
            };
            for entry in entries {
                // llmlint: ignore[changed_behavior_has_e2e] An iterator failing after
                // `read_dir` succeeds is an OS/filesystem race that cannot be induced
                // deterministically without mocking the layer under test.
                let entry = entry.map_err(|e| SourceError::Unavailable {
                    message: format!("cannot read entry in {}: {e}", dir.display()),
                })?;
                let path = entry.path();
                // A narrow metadata write's staging file is never an item, and is skipped
                // before it is resolved: it may be renamed away between the listing and here.
                if entry
                    .file_name()
                    .to_string_lossy()
                    .ends_with(STAGING_SUFFIX)
                {
                    continue;
                }
                // Only a link is resolved to confine it. Anything else is named by the folder
                // it was listed in — already resolved and confined — and its own name, which
                // is its resolved path already. Resolving a plain file instead would ask the
                // filesystem to spell a path while a replacement is renamed over it, and
                // Windows can answer that instant with a spelling that is not under the root.
                //
                // An entry removed between the listing and here — another process deleting
                // or renaming it — is skipped: it is not a record of this folder any more,
                // and nothing about it was read to call malformed.
                // llmlint: ignore[boundary_inputs_validated, changed_behavior_has_e2e] The
                // window between classifying an entry and reading it by path is the one the
                // code this replaces had between `canonicalize` and the read, not a new one:
                // an entry swapped for a link in between is followed either way. Closing it
                // needs a no-follow open relative to the folder's handle, which `std` does not
                // offer on every platform this ships on; and a process that can swap entries
                // under the root can already write whatever a record says. Forcing that swap
                // into that instant deterministically needs a double of the filesystem, which
                // the repository's test rules forbid.
                let linked = match entry.file_type() {
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                    file_type => file_type
                        .map_err(|e| SourceError::Unavailable {
                            message: format!("cannot read {}: {e}", path.display()),
                        })?
                        .is_symlink(),
                };
                let canonical = if linked {
                    let canonical = match fs::canonicalize(&path) {
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound && vanished(&path) => {
                            continue;
                        }
                        // A link that is still there and leads nowhere is the author's to
                        // mend, so it is reported rather than skipped.
                        canonical => canonical.map_err(|e| SourceError::Malformed {
                            message: format!("{}: {e}", path.display()),
                        })?,
                    };
                    if !canonical.starts_with(root) {
                        return Err(SourceError::Config {
                            message: format!(
                                "{} escapes configured root {}",
                                path.display(),
                                root.display()
                            ),
                        });
                    }
                    canonical
                } else {
                    dir.join(entry.file_name())
                };
                if canonical.is_dir() {
                    visit(root, &canonical, visited, out)?;
                } else if canonical
                    .extension()
                    .and_then(|x| x.to_str())
                    .is_some_and(|x| x.eq_ignore_ascii_case("md"))
                {
                    if canonical
                        .strip_prefix(root)
                        .ok()
                        .and_then(Path::to_str)
                        .is_none()
                    {
                        return Err(SourceError::Malformed {
                            message: format!("{} is not a UTF-8 path", canonical.display()),
                        });
                    }
                    out.push(canonical);
                }
            }
            Ok(())
        }
        let directory = self.directory(kind)?;
        let mut paths = Vec::new();
        visit(&self.root, &directory, &mut BTreeSet::new(), &mut paths)?;
        paths.sort();
        paths.dedup();
        Ok(paths)
    }

    /// One listed file's whole text, or `None` when it has been removed since it was listed.
    fn read_listed(path: &Path) -> Result<Option<String>, SourceError> {
        let _replacement = replacement_reader();
        // llmlint: ignore[boundary_inputs_validated] This read by path is the other end of the
        // window `paths` states where it classifies the entry: the code this replaces read by
        // path after `canonicalize` and followed a swapped-in link just the same. Closing it
        // needs a no-follow open relative to the folder's handle, which `std` does not offer
        // on every platform this ships on.
        match fs::read_to_string(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            text => text.map(Some).map_err(|e| SourceError::Malformed {
                message: format!("{}: {e}", path.display()),
            }),
        }
    }

    /// One file's whole text.
    fn read_text(path: &Path) -> Result<String, SourceError> {
        let _replacement = replacement_reader();
        fs::read_to_string(path).map_err(|e| SourceError::Malformed {
            message: format!("{}: {e}", path.display()),
        })
    }

    /// The YAML front matter and the body of `text`, the contents of the file at `path`, split
    /// apart.
    fn split_text<'t>(path: &Path, text: &'t str) -> Result<(&'t str, &'t str), SourceError> {
        front_matter(text)
            .map(|(yaml, body_at)| (yaml, &text[body_at..]))
            .ok_or_else(|| unfronted(path))
    }

    /// Everything a task, a project and a document all carry, read out of one file.
    ///
    /// The location is that file's own canonical absolute path, which is what makes this
    /// backend's answer to *where is it* something a reader can act on: print the path, or
    /// read the contents out, with no knowledge of this plugin.
    fn common(
        &self,
        kind: Kind,
        path: &Path,
        body: &str,
        front: SharedFront,
    ) -> Result<Common, SourceError> {
        let base = self.directory(kind)?;
        let relative = path
            .strip_prefix(&base)
            .map_err(|_| SourceError::Malformed {
                message: format!("{} is outside {}", path.display(), base.display()),
            })?;
        let not_utf8 = || SourceError::Malformed {
            message: format!("{} is not a UTF-8 path", path.display()),
        };
        let id = relative
            .with_extension("")
            .to_str()
            .ok_or_else(not_utf8)?
            .replace('\\', "/");
        let fallback = body
            .lines()
            .find_map(|line| {
                line.strip_prefix("# ")
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
            })
            .map(str::to_owned)
            .unwrap_or_else(|| {
                relative
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            });
        let body = body.trim();
        Ok(Common {
            id: NativeId(id),
            title: front.title.unwrap_or(fallback),
            body: (!body.is_empty()).then(|| body.to_owned()),
            labels: labels_of(front.labels),
            project: front.project,
            url: front.url,
            location: Location::Path(path.to_str().ok_or_else(not_utf8)?.to_owned()),
            metadata: front.metadata,
            repositories: Repository::unique(front.repositories).map_err(|message| {
                SourceError::Malformed {
                    message: format!("{}: {message}", path.display()),
                }
            })?,
        })
    }

    fn parse(&self, kind: WorkKind, path: &Path) -> Result<Entry, SourceError> {
        self.parse_text(kind, path, &Self::read_text(path)?)
    }

    /// The work item `text` reads as, were it the contents of the file at `path`.
    fn parse_text(&self, kind: WorkKind, path: &Path, text: &str) -> Result<Entry, SourceError> {
        // Every caller canonicalizes and confines the path before parsing it. Keeping that
        // invariant explicit here makes future internal callers notice if they skip the
        // boundary check without duplicating an unreachable user-facing branch.
        debug_assert!(path.starts_with(&self.root));
        let (yaml, body) = Self::split_text(path, text)?;
        let front: FrontMatter =
            serde_norway::from_str(yaml).map_err(|e| SourceError::Malformed {
                message: format!("{}: {e}", path.display()),
            })?;
        let (shared, status, depends_on, delivery) = front.split();
        // A task's comments section is not its content: what a query searches, a copy reads
        // and `task show` prints as the body is everything above it. A project has no
        // comments, so a `## Comments` heading in one is ordinary content.
        let content = match kind {
            WorkKind::Task => sectioned(body).0,
            WorkKind::Project => body,
        };
        let common = self.common(kind.kind(), path, content, shared)?;
        let status = Status {
            category: self
                .statuses
                .get(&status.to_lowercase())
                .copied()
                .unwrap_or(StatusCategory::Unknown),
            name: status,
        };
        let from = common.id.clone();
        let item_kind = kind.item();
        // A bare `depends_on: [b]` names this source's own item, colons and all, so it
        // stays an opaque native id. The expanded form is where an author says otherwise:
        // `{id: other:P-9, item: project}` names a far end this source cannot hold, and
        // `DependencyEndpoint::new` is what validates that qualified id.
        let dependencies = depends_on
            .into_iter()
            .map(|d| match d {
                // llmlint: ignore[boundary_inputs_validated] Dependency targets use the frozen contract's deliberately opaque, unvalidated `NativeId`; rejecting a value here would narrow that public contract.
                Dependency::Id(id) => Ok(DependencyEdge {
                    from: DependencyEndpoint::from_native(from.clone(), item_kind),
                    to: DependencyEndpoint::from_native(NativeId(id), item_kind),
                    kind: DependencyKind::Blocks,
                }),
                Dependency::Detailed { id, kind, item } => Ok(DependencyEdge {
                    from: DependencyEndpoint::from_native(from.clone(), item_kind),
                    to: DependencyEndpoint::new(id, item.map_or(item_kind, Into::into)).map_err(
                        |message| SourceError::Malformed {
                            message: format!("{}: {message}", path.display()),
                        },
                    )?,
                    kind: match kind {
                        EdgeKind::Blocks => DependencyKind::Blocks,
                        EdgeKind::Related => DependencyKind::Related,
                    },
                }),
            })
            .collect::<Result<Vec<_>, SourceError>>()?;
        let (delivers, delivered_by) = match kind {
            WorkKind::Task => (
                self.task_list("delivers", &common.id, path, delivery.delivers.as_ref())?,
                self.task_list(
                    "delivered_by",
                    &common.id,
                    path,
                    delivery.delivered_by.as_ref(),
                )?,
            ),
            WorkKind::Project => {
                if delivery.delivers.is_some() || delivery.delivered_by.is_some() {
                    return Err(SourceError::Malformed {
                        message: format!(
                            "{}: `delivers` and `delivered_by` belong to a task, and this is a \
                             project; next: move them to the task that delivers",
                            path.display()
                        ),
                    });
                }
                (Vec::new(), Vec::new())
            }
        };
        Ok(Entry {
            common,
            status,
            dependencies,
            delivers,
            delivered_by,
        })
    }

    /// One of a task file's two task lists, read as task ids.
    fn task_list(
        &self,
        field: &str,
        id: &NativeId,
        path: &Path,
        value: Option<&serde_json::Value>,
    ) -> Result<Vec<TaskRef>, SourceError> {
        TaskRef::from_value(field, id, self.name.as_ref(), value).map_err(|message| {
            SourceError::Malformed {
                message: format!("{}: {message}", path.display()),
            }
        })
    }

    /// One file under `documents/`, read straight into the contract's own type.
    ///
    /// There is no `Entry` on this path because there is nothing to hold in one: a
    /// document has no status and no edges, so what a work item's parse computes for those
    /// two has nothing here to compute it from.
    fn parse_document(&self, path: &Path) -> Result<Document, SourceError> {
        self.parse_document_text(path, &Self::read_text(path)?)
    }

    /// The document `text` reads as, were it the contents of the file at `path`.
    fn parse_document_text(&self, path: &Path, text: &str) -> Result<Document, SourceError> {
        debug_assert!(path.starts_with(&self.root));
        let (yaml, body) = Self::split_text(path, text)?;
        let front: DocumentFrontMatter =
            serde_norway::from_str(yaml).map_err(|e| SourceError::Malformed {
                message: format!("{}: {e}", path.display()),
            })?;
        let common = self.common(Kind::Document, path, body, front.into())?;
        Ok(Document {
            id: common.id,
            title: common.title,
            content: common.body,
            project: common.project,
            labels: common.labels,
            url: common.url,
            location: Some(common.location),
            created_at: None,
            updated_at: None,
            metadata: common.metadata,
            repositories: common.repositories,
        })
    }

    /// Every record of `kind` that `scope` could admit, read in full.
    ///
    /// A file removed since the folder was listed is skipped. A file whose front matter
    /// files it outside `scope` is skipped before it is parsed, so a record another project
    /// holds cannot fail a query about this one; a file that cannot be shown to be outside it
    /// is parsed, and fails the read when it does not parse. `scope` is only ever a
    /// narrowing, and the caller still applies it to what comes back.
    fn parse_work_records(
        &self,
        kind: WorkKind,
        scope: &ProjectFilter,
    ) -> Result<Vec<Entry>, SourceError> {
        self.paths(kind.kind())?
            .into_iter()
            .filter_map(|path| match Self::read_listed(&path) {
                Ok(Some(text)) if outside(&text, scope) => None,
                Ok(Some(text)) => Some(self.parse_text(kind, &path, &text)),
                Ok(None) => None,
                Err(error) => Some(Err(error)),
            })
            .collect()
    }

    /// As [`Self::parse_work_records`], for the files under `documents/`.
    fn parse_document_records(&self, scope: &ProjectFilter) -> Result<Vec<Document>, SourceError> {
        self.paths(Kind::Document)?
            .into_iter()
            .filter_map(|path| match Self::read_listed(&path) {
                Ok(Some(text)) if outside(&text, scope) => None,
                Ok(Some(text)) => Some(self.parse_document_text(&path, &text)),
                Ok(None) => None,
                Err(error) => Some(Err(error)),
            })
            .collect()
    }

    /// The confined canonical path `id` names under `kind`, when this source holds one.
    fn locate(&self, kind: Kind, id: &NativeId) -> Result<Option<PathBuf>, SourceError> {
        let _replacement = replacement_reader();
        let base = self.directory(kind)?;
        let candidate = base.join(&id.0).with_extension("md");
        if !candidate.exists() {
            return Ok(None);
        }
        let canonical = fs::canonicalize(&candidate).map_err(|e| SourceError::Malformed {
            message: format!("{}: {e}", candidate.display()),
        })?;
        if !canonical.starts_with(&base) {
            return Err(SourceError::Config {
                message: format!(
                    "{} escapes configured root {}",
                    candidate.display(),
                    self.root.display()
                ),
            });
        }
        Ok(Some(canonical))
    }

    fn find(&self, kind: WorkKind, id: &NativeId) -> Result<Option<Entry>, SourceError> {
        self.locate(kind.kind(), id)?
            .map(|path| self.parse(kind, &path))
            .transpose()
    }

    fn paginate<T>(&self, items: Vec<T>, page: &PageRequest) -> Result<Page<T>, SourceError> {
        if page.limit == 0 {
            return Err(SourceError::Config {
                message: "page limit must be at least 1".to_owned(),
            });
        }
        let start = match &page.cursor {
            None => 0,
            Some(Cursor(raw)) => raw.parse::<usize>().map_err(|_| SourceError::Malformed {
                message: format!("cursor {raw:?} was not issued by local-md"),
            })?,
        };
        if start > items.len() {
            return Err(SourceError::Malformed {
                message: format!("cursor points past {} results", items.len()),
            });
        }
        let total = items.len();
        let limit = page.limit.min(MAX_PAGE_SIZE) as usize;
        let end = start.saturating_add(limit).min(items.len());
        let mut items = items;
        let tail = items.split_off(start);
        let window = tail.into_iter().take(end - start).collect();
        Ok(Page {
            items: window,
            next: (end < total).then(|| Cursor(end.to_string())),
        })
    }
}

/// The labels one file's `labels:` key names, in the order it names them.
fn labels_of(inputs: Vec<LabelInput>) -> Vec<Label> {
    inputs
        .into_iter()
        .map(|label| match label {
            LabelInput::Name(name) => Label {
                // llmlint: ignore[boundary_inputs_validated] `NativeId` deliberately accepts every upstream string in the frozen plugin contract; lowercasing the label name is this source's stable opaque-id mapping, not a validation boundary.
                id: NativeId(name.to_lowercase()),
                name,
                color: None,
            },
            LabelInput::Detailed { id, name, color } => Label {
                // llmlint: ignore[boundary_inputs_validated] `NativeId` is deliberately unvalidated and opaque in the frozen plugin contract, so this source must preserve the author's explicit id.
                id: NativeId(id),
                name,
                color,
            },
        })
        .collect()
}

fn labels_match(labels: &[Label], f: &LabelFilter) -> bool {
    let has = |n: &String| labels.iter().any(|l| l.name.eq_ignore_ascii_case(n));
    (f.any_of.is_empty() || f.any_of.iter().any(has))
        && f.all_of.iter().all(has)
        && !f.none_of.iter().any(has)
}
fn text_match(title: &str, body: Option<&str>, q: &TextQuery) -> bool {
    let t = q.terms.to_lowercase();
    match q.fields {
        TextFields::Title => title.to_lowercase().contains(&t),
        TextFields::Content => body.is_some_and(|b| b.to_lowercase().contains(&t)),
        TextFields::TitleOrContent => {
            title.to_lowercase().contains(&t) || body.is_some_and(|b| b.to_lowercase().contains(&t))
        }
    }
}

#[async_trait::async_trait]
impl TaskSource for LocalMdSource {
    fn kind(&self) -> &'static str {
        KIND
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            projects: Support::Native,
            documents: Support::Native,
            comments: Support::Native,
            orphan_tasks: Support::Native,
            filter_by_label: Support::Native,
            filter_by_status: Support::Native,
            search_title: Support::Native,
            search_content: Support::Native,
            task_dependencies: DependencySupport::BothDirections,
            project_dependencies: DependencySupport::BothDirections,
            max_page_size: MAX_PAGE_SIZE,
        }
    }
    fn writes(&self) -> WriteSupport {
        WriteSupport::Supported
    }
    async fn health(&self) -> Result<Health, SourceError> {
        self.paths(Kind::Task)?;
        self.paths(Kind::Project)?;
        self.paths(Kind::Document)?;
        Ok(Health {
            reachable: true,
            detail: Some(format!("reading Markdown under {}", self.root.display())),
        })
    }
    async fn get_task(&self, id: &NativeId) -> Result<Option<Task>, SourceError> {
        Ok(self.find(WorkKind::Task, id)?.map(task))
    }
    async fn get_project(&self, id: &NativeId) -> Result<Option<Project>, SourceError> {
        Ok(self.find(WorkKind::Project, id)?.map(project))
    }
    async fn query_tasks(&self, q: &TaskQuery, p: &PageRequest) -> Result<Page<Task>, SourceError> {
        let items = self
            .parse_work_records(WorkKind::Task, &q.project)?
            .into_iter()
            .map(task)
            .filter(|t| {
                labels_match(&t.labels, &q.labels)
                    && (q.statuses.is_empty() || q.statuses.contains(&t.status.category))
                    && match &q.project {
                        ProjectFilter::Any => true,
                        ProjectFilter::Orphans => t.project.is_none(),
                        ProjectFilter::Is(id) => t.project.as_ref() == Some(id),
                    }
                    && q.text
                        .as_ref()
                        .is_none_or(|x| text_match(&t.title, t.content.as_deref(), x))
            })
            .collect();
        self.paginate(items, p)
    }
    async fn query_projects(
        &self,
        q: &ProjectQuery,
        p: &PageRequest,
    ) -> Result<Page<Project>, SourceError> {
        let items = self
            .parse_work_records(WorkKind::Project, &ProjectFilter::Any)?
            .into_iter()
            .map(project)
            .filter(|x| {
                labels_match(&x.labels, &q.labels)
                    && (q.statuses.is_empty() || q.statuses.contains(&x.status.category))
                    && q.text
                        .as_ref()
                        .is_none_or(|z| text_match(&x.title, x.content.as_deref(), z))
            })
            .collect();
        self.paginate(items, p)
    }
    /// Every document under `documents/`, narrowed by every predicate `q` carries.
    ///
    /// The same three a task query carries minus the status filter, because a document has
    /// no status for one to compare against.
    async fn query_documents(
        &self,
        q: &DocumentQuery,
        p: &PageRequest,
    ) -> Result<Page<Document>, SourceError> {
        let items = self
            .parse_document_records(&q.project)?
            .into_iter()
            .filter(|d| {
                labels_match(&d.labels, &q.labels)
                    && match &q.project {
                        ProjectFilter::Any => true,
                        ProjectFilter::Orphans => d.project.is_none(),
                        ProjectFilter::Is(id) => d.project.as_ref() == Some(id),
                    }
                    && q.text
                        .as_ref()
                        .is_none_or(|x| text_match(&d.title, d.content.as_deref(), x))
            })
            .collect();
        self.paginate(items, p)
    }
    async fn get_document(&self, id: &NativeId) -> Result<Option<Document>, SourceError> {
        self.locate(Kind::Document, id)?
            .map(|path| self.parse_document(&path))
            .transpose()
    }
    async fn labels(&self, p: &PageRequest) -> Result<Page<Label>, SourceError> {
        let mut seen = BTreeSet::new();
        // Documents too: a label a document carries is a label of this source, and reading
        // one more folder that is already on disk is the same read as the other two.
        let mut items: Vec<Label> = self
            .parse_work_records(WorkKind::Task, &ProjectFilter::Any)?
            .into_iter()
            .chain(self.parse_work_records(WorkKind::Project, &ProjectFilter::Any)?)
            .flat_map(|d| d.common.labels)
            .chain(
                self.parse_document_records(&ProjectFilter::Any)?
                    .into_iter()
                    .flat_map(|d| d.labels),
            )
            .filter(|l| seen.insert(l.name.to_lowercase()))
            .collect();
        items.sort_by(|left, right| left.id.0.cmp(&right.id.0));
        self.paginate(items, p)
    }
    async fn task_dependencies(
        &self,
        id: &NativeId,
        d: Direction,
        p: &PageRequest,
    ) -> Result<Page<DependencyEdge>, SourceError> {
        self.edges(WorkKind::Task, id, d, p)
    }
    async fn project_dependencies(
        &self,
        id: &NativeId,
        d: Direction,
        p: &PageRequest,
    ) -> Result<Page<DependencyEdge>, SourceError> {
        self.edges(WorkKind::Project, id, d, p)
    }
    async fn write_task(&self, write: &ItemWrite<Task>) -> Result<NativeId, SourceError> {
        let task = &write.item;
        let near = write.target.as_ref().unwrap_or(&task.id);
        for (field, list) in [
            ("delivers", &task.delivers),
            ("delivered_by", &task.delivered_by),
        ] {
            self.representable_list(field, near, list)?;
        }
        self.write_entry(
            write.target.as_ref(),
            &Outgoing::Work {
                kind: WorkKind::Task,
                status: &task.status,
                depends_on: &write.depends_on,
                delivers: &task.delivers,
                delivered_by: &task.delivered_by,
                fields: Fields {
                    id: &task.id,
                    title: &task.title,
                    content: task.content.as_deref(),
                    labels: &task.labels,
                    project: task.project.as_ref(),
                    metadata: &task.metadata,
                    repositories: &task.repositories,
                },
            },
        )
    }
    async fn write_project(&self, write: &ItemWrite<Project>) -> Result<NativeId, SourceError> {
        let project = &write.item;
        self.write_entry(
            write.target.as_ref(),
            &Outgoing::Work {
                kind: WorkKind::Project,
                status: &project.status,
                depends_on: &write.depends_on,
                delivers: &[],
                delivered_by: &[],
                fields: Fields {
                    id: &project.id,
                    title: &project.title,
                    content: project.content.as_deref(),
                    labels: &project.labels,
                    project: None,
                    metadata: &project.metadata,
                    repositories: &project.repositories,
                },
            },
        )
    }
    /// One file under `documents/`, on exactly the terms a task lands under `tasks/`.
    ///
    /// `write.depends_on` reaches nothing here, which is the contract rather than an
    /// omission: nothing may point at a document, so [`Outgoing::Document`] has nowhere to
    /// carry an edge and no status to disagree with this folder's mapping.
    // llmlint: ignore[boundary_inputs_validated] `ItemWrite` carries `depends_on` for all three kinds and the frozen contract says nothing about a document's being empty, so a non-empty one is not an input this plugin may rule on. `in-memory`, the reference implementation of this method, ignores it for the same recorded reason; refusing here would make this the one source that rejects a call every other source accepts, which is a change to the contract rather than to this plugin and is its owner's to make.
    async fn write_document(&self, write: &ItemWrite<Document>) -> Result<NativeId, SourceError> {
        let document = &write.item;
        self.write_entry(
            write.target.as_ref(),
            &Outgoing::Document {
                fields: Fields {
                    id: &document.id,
                    title: &document.title,
                    content: document.content.as_deref(),
                    labels: &document.labels,
                    project: document.project.as_ref(),
                    metadata: &document.metadata,
                    repositories: &document.repositories,
                },
            },
        )
    }
    /// Rewrite the one `status:` entry of the task's front matter, and nothing else.
    ///
    /// A task already in `category` is left byte for byte as it is, word and all: a rewrite
    /// would respell a status the file already holds. Otherwise the word written is one this
    /// folder's `status_mapping` reads back as `category` — the category's own spelling when
    /// the mapping has it, else the first word that maps there — and a category the mapping
    /// reaches with no word is refused in the words a copy of that status is.
    async fn set_task_status(
        &self,
        id: &NativeId,
        category: StatusCategory,
    ) -> Result<Option<Status>, SourceError> {
        let Some(path) = self.locate(Kind::Task, id)? else {
            return Ok(None);
        };
        let entry = self.parse(WorkKind::Task, &path)?;
        if entry.status.category == category {
            return Ok(Some(entry.status));
        }
        let word = self.word_for(category);
        let status = Status {
            category,
            name: word,
        };
        self.representable_status(&status)?;
        // A string always renders as a YAML scalar.
        let rendered = serde_norway::to_string(&status.name).expect("a status word renders");
        self.rewrite_front_entry(&path, "status", Some(rendered.trim_end()))?;
        Ok(Some(status))
    }

    /// Rewrite the one `delivered_by:` entry of the task's front matter, and nothing else —
    /// removing it when the list is empty.
    async fn set_delivered_by(
        &self,
        id: &NativeId,
        delivered_by: &[TaskRef],
    ) -> Result<Option<()>, SourceError> {
        self.representable_list("delivered_by", id, delivered_by)?;
        let Some(path) = self.locate(Kind::Task, id)? else {
            return Ok(None);
        };
        self.parse(WorkKind::Task, &path)?;
        // A list of strings always renders as JSON, which is a YAML flow sequence.
        let rendered = (!delivered_by.is_empty())
            .then(|| serde_json::to_string(delivered_by).expect("a list of task ids renders"));
        self.rewrite_front_entry(&path, "delivered_by", rendered.as_deref())?;
        Ok(Some(()))
    }

    /// Edit the one entry for `key` in the task's front-matter `metadata:` block, and nothing
    /// else; see [`STAGING_SUFFIX`] for how the file is replaced and what is refused.
    async fn set_task_metadata(
        &self,
        id: &NativeId,
        key: &MetadataKey,
        value: &serde_json::Value,
    ) -> Result<Option<Task>, SourceError> {
        self.set_metadata(
            Kind::Task,
            id,
            key,
            value,
            |path, text| self.parse_text(WorkKind::Task, path, text).map(task),
            |task| &mut task.metadata,
        )
    }

    /// As [`set_task_metadata`](TaskSource::set_task_metadata), for a file under `projects/`.
    async fn set_project_metadata(
        &self,
        id: &NativeId,
        key: &MetadataKey,
        value: &serde_json::Value,
    ) -> Result<Option<Project>, SourceError> {
        self.set_metadata(
            Kind::Project,
            id,
            key,
            value,
            |path, text| self.parse_text(WorkKind::Project, path, text).map(project),
            |project| &mut project.metadata,
        )
    }

    /// As [`set_task_metadata`](TaskSource::set_task_metadata), for a file under `documents/`,
    /// verified against a document's own front matter.
    async fn set_document_metadata(
        &self,
        id: &NativeId,
        key: &MetadataKey,
        value: &serde_json::Value,
    ) -> Result<Option<Document>, SourceError> {
        self.set_metadata(
            Kind::Document,
            id,
            key,
            value,
            |path, text| self.parse_document_text(path, text),
            |document| &mut document.metadata,
        )
    }
    async fn delete_task(&self, id: &NativeId) -> Result<(), SourceError> {
        self.delete_entry(Kind::Task, id)
    }
    async fn delete_project(&self, id: &NativeId) -> Result<(), SourceError> {
        self.delete_entry(Kind::Project, id)
    }
    async fn delete_document(&self, id: &NativeId) -> Result<(), SourceError> {
        self.delete_entry(Kind::Document, id)
    }
    async fn task_comments(
        &self,
        task: &NativeId,
        page: &PageRequest,
    ) -> Result<Option<Page<Comment>>, SourceError> {
        let Some(file) = self.task_file(task)? else {
            return Ok(None);
        };
        self.paginate(file.comments, page).map(Some)
    }
    async fn add_comment(
        &self,
        task: &NativeId,
        comment: &NewComment,
    ) -> Result<Option<Comment>, SourceError> {
        if let Some(author) = &comment.author {
            representable_author(author)?;
        }
        representable_body(&comment.body)?;
        let Some(mut file) = self.task_file(task)? else {
            return Ok(None);
        };
        let now = this_second();
        let added = Comment {
            id: minted_comment_id(now, &file.comments),
            author: comment.author.clone(),
            created_at: Some(now),
            updated_at: Some(now),
            body: comment.body.as_str().to_owned(),
            url: None,
        };
        file.comments.push(added.clone());
        file.rewrite()?;
        Ok(Some(added))
    }
    async fn edit_comment(
        &self,
        task: &NativeId,
        comment: &NativeId,
        body: &CommentBody,
    ) -> Result<Option<Comment>, SourceError> {
        representable_body(body)?;
        let Some(mut file) = self.task_file(task)? else {
            return Ok(None);
        };
        let Some(edited) = file.comments.iter_mut().find(|held| &held.id == comment) else {
            return Ok(None);
        };
        edited.body = body.as_str().to_owned();
        edited.updated_at = Some(this_second());
        let edited = edited.clone();
        file.rewrite()?;
        Ok(Some(edited))
    }
    async fn delete_comment(
        &self,
        task: &NativeId,
        comment: &NativeId,
    ) -> Result<Option<NativeId>, SourceError> {
        let Some(mut file) = self.task_file(task)? else {
            return Ok(None);
        };
        let Some(at) = file.comments.iter().position(|held| &held.id == comment) else {
            return Ok(None);
        };
        file.comments.remove(at);
        file.rewrite()?;
        Ok(Some(comment.clone()))
    }
}

/// The heading line that opens a task file's comments section.
///
/// # The comments section, rule by rule
///
/// A task's comments are an **optional trailing section of the task's own file**, after its
/// body, human-readable and at full fidelity:
///
/// ```markdown
/// ## Comments
///
/// <!-- onetaskgraph:comment id="20260913T151107Z-1" author="ada" created_at="2026-09-13T15:11:07Z" updated_at="2026-09-13T15:11:07Z" -->
/// ### ada — 2026-09-13T15:11:07Z
///
/// The body, byte-for-byte: any Markdown, including its own `##` headings.
///
/// <!-- /onetaskgraph:comment -->
/// ```
///
/// - The section is this heading line followed by one or more comment blocks, and nothing but
///   blank lines between and after them to the end of the file. A `## Comments` heading
///   anywhere else, or one followed by anything that is not a comment block, is ordinary task
///   content. A task with no comments has no section: removing the last removes the heading.
/// - A block opens with the marker line [`COMMENT_OPEN`] carrying `id`, `author` when one was
///   given, `created_at` and `updated_at`, each double-quoted with `&`, `"`, `<` and `>`
///   written as the four XML entities. A human heading follows — `### <author> — <created_at>`,
///   or `### comment — <created_at>` without an author — then a blank line, the body verbatim,
///   a blank line, and [`COMMENT_CLOSE`] on a line of its own. The marker is the source of
///   truth; the heading is for a person and is regenerated on every write.
/// - A body containing a line that is exactly [`COMMENT_CLOSE`] is refused rather than
///   escaped, because escaping it would store a body other than the one written.
/// - A comment id is minted as `<created_at in UTC as YYYYMMDDTHHMMSSZ>-<n>`, with `n` the
///   smallest positive integer not already an id in that file, and is never renumbered.
/// - The section is not the task's content: every read reports content as the body above it,
///   so a content search never matches a comment. A copy written into the file keeps its
///   existing section byte for byte, and content that would itself read as a section is
///   refused rather than turned into comments nobody wrote.
///
/// `docs/local-md.md` describes the same section for a person; this is its one executable
/// source.
pub const COMMENTS_HEADING: &str = "## Comments";

/// The start of the marker line that opens one comment block.
pub const COMMENT_OPEN: &str = "<!-- onetaskgraph:comment ";

/// The line that closes one comment block.
pub const COMMENT_CLOSE: &str = "<!-- /onetaskgraph:comment -->";

/// The end of the marker line that opens one comment block.
const MARKER_END: &str = " -->";

/// One task file, read for its comments: where its content ends and what its section holds.
struct TaskFile {
    path: PathBuf,
    /// The whole file, exactly as it was read.
    text: String,
    /// Where the body begins: everything before this is the front matter and its delimiters.
    body_at: usize,
    /// Where the comments section begins, or the end of the file when there is none.
    section_at: usize,
    comments: Vec<Comment>,
}

impl TaskFile {
    /// Write the file back with `comments` as its section, touching nothing above it.
    ///
    /// The one blank line that separates a section from the content is added with the section
    /// and removed with it, so a task that gains and then loses its only comment reads back
    /// with the content it had.
    fn rewrite(&self) -> Result<(), SourceError> {
        let content = &self.text[self.body_at..self.section_at];
        let mut written = String::with_capacity(self.text.len() + 256);
        written.push_str(&self.text[..self.body_at]);
        if self.comments.is_empty() {
            match content.strip_suffix("\n\n") {
                Some(above) => {
                    written.push_str(above);
                    written.push('\n');
                }
                None => written.push_str(content),
            }
        } else {
            written.push_str(content);
            while !written.ends_with("\n\n") {
                written.push('\n');
            }
            written.push_str(&rendered_section(&self.comments));
        }
        fs::write(&self.path, written).map_err(|e| SourceError::Unavailable {
            message: format!("cannot write {}: {e}", self.path.display()),
        })
    }
}

impl LocalMdSource {
    /// The task file `id` names, read for its comments, or `None` when there is no such task.
    ///
    /// The task is read exactly as [`get_task`](TaskSource::get_task) reads it first, so a
    /// file that is not a readable task is refused for the reason that read gives rather than
    /// written into.
    fn task_file(&self, id: &NativeId) -> Result<Option<TaskFile>, SourceError> {
        let Some(path) = self.locate(Kind::Task, id)? else {
            return Ok(None);
        };
        self.parse(WorkKind::Task, &path)?;
        let text = fs::read_to_string(&path).map_err(|e| SourceError::Malformed {
            message: format!("{}: {e}", path.display()),
        })?;
        let (_, body_at) = front_matter(&text).ok_or_else(|| unfronted(&path))?;
        let (content, comments) = sectioned(&text[body_at..]);
        let section_at = body_at + content.len();
        let comments = comments.unwrap_or_default();
        let mut seen = BTreeSet::new();
        if let Some(repeated) = comments.iter().find(|held| !seen.insert(&held.id)) {
            return Err(SourceError::Malformed {
                message: format!(
                    "{}: two comments share the id {}; next: give one of them an id of its own \
                     in its marker line, so an edit or a delete names one comment",
                    path.display(),
                    repeated.id
                ),
            });
        }
        Ok(Some(TaskFile {
            path,
            text,
            body_at,
            section_at,
            comments,
        }))
    }
}

/// Where one file's YAML front matter is, and the byte its body begins at.
fn front_matter(text: &str) -> Option<(&str, usize)> {
    for (open, close) in [("---\n", "\n---\n"), ("---\r\n", "\r\n---\r\n")] {
        if let Some(rest) = text.strip_prefix(open) {
            return rest
                .find(close)
                .map(|at| (&rest[..at], open.len() + at + close.len()));
        }
    }
    None
}

/// `text` with its front matter's top-level `key` entry replaced by `key: value`, or removed
/// when `value` is `None`, and every other byte exactly as it was — or `None` when `text` has
/// no front matter.
///
/// An entry is its `key:` line and every line after it that continues it: an indented line,
/// or a `- ` sequence item. An absent key is added as the last line of the front matter. The
/// line ending written is the one the file already uses.
fn with_front_entry(text: &str, key: &str, value: Option<&str>) -> Option<String> {
    let (open, newline) = if text.starts_with("---\r\n") {
        ("---\r\n", "\r\n")
    } else {
        ("---\n", "\n")
    };
    let (yaml, _) = front_matter(text)?;
    let start = open.len();
    let end = start + yaml.len();
    let mut lines: Vec<(usize, usize)> = Vec::new();
    let mut at = start;
    while at < end {
        let next = text[at..end].find('\n').map_or(end, |found| at + found + 1);
        lines.push((at, next));
        at = next;
    }
    let prefix = format!("{key}:");
    let written = value.map(|value| format!("{key}: {value}"));
    let Some(index) = lines
        .iter()
        .position(|&(from, _)| text[from..].starts_with(&prefix))
    else {
        let Some(written) = written else {
            return Some(text.to_owned());
        };
        let separator = if end > start { newline } else { "" };
        return Some(format!(
            "{}{separator}{written}{}",
            &text[..end],
            &text[end..]
        ));
    };
    let mut last = index;
    while let Some(&(from, to)) = lines.get(last + 1) {
        let line = text[from..to].trim_end_matches(['\r', '\n']);
        if line.starts_with([' ', '\t']) || line == "-" || line.starts_with("- ") {
            last += 1;
        } else {
            break;
        }
    }
    let from = lines[index].0;
    let to = lines[last].1;
    let ended = text[..to].ends_with('\n');
    Some(match written {
        Some(written) => format!(
            "{}{written}{}{}",
            &text[..from],
            if ended { newline } else { "" },
            &text[to..]
        ),
        // The entry was the last line, so the line ending before it goes with it.
        None if !ended && from > start => {
            let before = text[..from].strip_suffix(newline).unwrap_or(&text[..from]);
            format!("{before}{}", &text[to..])
        }
        None => format!("{}{}", &text[..from], &text[to..]),
    })
}

/// Whether the entry at `path`, which could not be resolved, is gone from its folder rather
/// than a link that leads nowhere.
fn vanished(path: &Path) -> bool {
    fs::symlink_metadata(path).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
}

/// Whether the file whose contents are `text` provably files its record outside `scope`.
///
/// Read far more leniently than a record is: only the front matter's `project` key, through
/// [`Filing`], ignoring every other key. A file with no front matter names no project. Front
/// matter whose `project` key [`Filing`] cannot read — or that is not YAML at all — is not
/// provably outside, so the caller parses it in full and it fails as itself.
fn outside(text: &str, scope: &ProjectFilter) -> bool {
    let wanted = match scope {
        ProjectFilter::Any => return false,
        ProjectFilter::Orphans => None,
        ProjectFilter::Is(id) => Some(id),
    };
    let project = match front_matter(text) {
        None => None,
        Some((yaml, _)) => match serde_norway::from_str::<Filing>(yaml) {
            Ok(filing) => filing.project,
            Err(_) => return false,
        },
    };
    project.as_ref() != wanted
}

/// The refusal for a file with no front matter this source can find.
fn unfronted(path: &Path) -> SourceError {
    SourceError::Malformed {
        message: format!(
            "{}: expected YAML front matter delimited by ---",
            path.display()
        ),
    }
}

/// A task body split into its content and its trailing comments section, when it has one.
///
/// Every line that is exactly [`COMMENTS_HEADING`] is a candidate, earliest first, and the
/// first from which the rest of the body reads as comment blocks to its end is the section.
/// Earliest first is what keeps a body's own `## Comments` line — inside a comment, or in
/// content followed by prose — from being read as the section: a comment's body is consumed
/// whole up to its closing line, and a heading followed by prose is not a run of blocks.
fn sectioned(body: &str) -> (&str, Option<Vec<Comment>>) {
    let mut at = 0;
    while at <= body.len() {
        let line_end = body[at..].find('\n').map_or(body.len(), |end| at + end);
        if &body[at..line_end] == COMMENTS_HEADING
            && let Some(comments) = section(&body[at..])
        {
            return (&body[..at], Some(comments));
        }
        if line_end == body.len() {
            break;
        }
        at = line_end + 1;
    }
    (body, None)
}

/// The comments `text` holds when it is exactly a section, or `None` when it is not one.
fn section(text: &str) -> Option<Vec<Comment>> {
    let mut rest = text.strip_prefix(COMMENTS_HEADING)?.strip_prefix('\n')?;
    let mut comments = Vec::new();
    loop {
        rest = rest.trim_start_matches('\n');
        if rest.is_empty() {
            return (!comments.is_empty()).then_some(comments);
        }
        let (comment, after) = block(rest)?;
        comments.push(comment);
        rest = after;
    }
}

/// One comment block at the start of `text`, and what follows it.
fn block(text: &str) -> Option<(Comment, &str)> {
    let (marker, rest) = text.split_once('\n')?;
    let attributes = attributes(
        marker
            .strip_prefix(COMMENT_OPEN)?
            .strip_suffix(MARKER_END)?,
    )?;
    let (heading, rest) = rest.split_once('\n')?;
    heading.strip_prefix("### ")?;
    let rest = rest.strip_prefix('\n')?;
    let closing = closing_line(rest)?;
    let body = rest[..closing].strip_suffix("\n\n")?;
    let after = &rest[closing + COMMENT_CLOSE.len()..];
    let after = match after.strip_prefix('\n') {
        Some(after) => after,
        None if after.is_empty() => after,
        None => return None,
    };
    if body.is_empty() {
        return None;
    }
    let mut id = None;
    let mut author = None;
    let mut created_at = None;
    let mut updated_at = None;
    for (name, value) in attributes {
        let slot = match name {
            "id" => &mut id,
            "author" => &mut author,
            "created_at" => &mut created_at,
            "updated_at" => &mut updated_at,
            _ => return None,
        };
        if slot.replace(value).is_some() {
            return None;
        }
    }
    let timestamp = |value: String| {
        DateTime::parse_from_rfc3339(&value)
            .ok()
            .map(|at| at.with_timezone(&Utc))
    };
    let id = id.filter(|id| !id.is_empty())?;
    Some((
        Comment {
            id: NativeId(id),
            author,
            created_at: Some(timestamp(created_at?)?),
            updated_at: Some(timestamp(updated_at?)?),
            body: body.to_owned(),
            url: None,
        },
        after,
    ))
}

/// The byte a line that is exactly [`COMMENT_CLOSE`] starts at, if `text` has one.
fn closing_line(text: &str) -> Option<usize> {
    let mut at = 0;
    loop {
        let found = at + text[at..].find(COMMENT_CLOSE)?;
        let starts_line = found == 0 || text.as_bytes()[found - 1] == b'\n';
        let end = found + COMMENT_CLOSE.len();
        let ends_line = end == text.len() || text.as_bytes()[end] == b'\n';
        if starts_line && ends_line {
            return Some(found);
        }
        at = found + 1;
    }
}

/// A marker line's `name="value"` pairs, unescaped, or `None` when it is not a list of them.
fn attributes(text: &str) -> Option<Vec<(&str, String)>> {
    let mut pairs = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let (name, after) = rest.split_once("=\"")?;
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
        {
            return None;
        }
        let (value, after) = after.split_once('"')?;
        pairs.push((name, unescaped(value)));
        rest = match after.strip_prefix(' ') {
            Some(after) if !after.is_empty() => after,
            Some(_) => return None,
            None if after.is_empty() => after,
            None => return None,
        };
    }
    Some(pairs)
}

/// An attribute value as a marker line writes it.
fn escaped(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// An attribute value as a marker line wrote it, read back.
fn unescaped(value: &str) -> String {
    value
        .replace("&quot;", "\"")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// A timestamp as the section spells one: RFC 3339 in UTC, to the second.
fn stamped(at: &DateTime<Utc>) -> String {
    at.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// The section holding `comments`, in the order given.
fn rendered_section(comments: &[Comment]) -> String {
    let mut section = format!("{COMMENTS_HEADING}\n");
    for comment in comments {
        section.push('\n');
        section.push_str(&rendered_block(comment));
    }
    section
}

/// One comment block, marker first and closing line last.
fn rendered_block(comment: &Comment) -> String {
    let mut marker = format!("{COMMENT_OPEN}id=\"{}\"", escaped(comment.id.as_str()));
    if let Some(author) = &comment.author {
        marker.push_str(&format!(" author=\"{}\"", escaped(author)));
    }
    let created = comment.created_at.as_ref().map(stamped).unwrap_or_default();
    let updated = comment.updated_at.as_ref().map(stamped).unwrap_or_default();
    marker.push_str(&format!(
        " created_at=\"{}\" updated_at=\"{}\"{MARKER_END}",
        escaped(&created),
        escaped(&updated)
    ));
    let who = comment.author.as_deref().unwrap_or("comment");
    format!(
        "{marker}\n### {who} — {created}\n\n{}\n\n{COMMENT_CLOSE}\n",
        comment.body
    )
}

/// `<created_at in UTC as YYYYMMDDTHHMMSSZ>-<n>`, for the smallest positive `n` not already an
/// id among `comments`.
fn minted_comment_id(at: DateTime<Utc>, comments: &[Comment]) -> NativeId {
    let stamp = at.format("%Y%m%dT%H%M%SZ");
    (1_u64..)
        .map(|n| NativeId(format!("{stamp}-{n}")))
        .find(|candidate| !comments.iter().any(|held| &held.id == candidate))
        .expect("an unbounded counter eventually clears a finite set of ids")
}

/// Now, to the second — the precision the section writes a time down in, so a comment reads
/// back with exactly the times it was written with.
fn this_second() -> DateTime<Utc> {
    let now = Utc::now();
    DateTime::from_timestamp(now.timestamp(), 0).unwrap_or(now)
}

/// Refuse a body the section cannot hold byte for byte.
fn representable_body(body: &CommentBody) -> Result<(), SourceError> {
    if body.as_str().split('\n').any(|line| line == COMMENT_CLOSE) {
        return Err(SourceError::Refused {
            message: format!(
                "cannot represent this comment body: it contains a line that is exactly \
                 {COMMENT_CLOSE}, which closes a comment in a task file and would end this one \
                 early; next: change that line"
            ),
        });
    }
    Ok(())
}

/// Refuse an author the section cannot hold on one heading line.
fn representable_author(author: &str) -> Result<(), SourceError> {
    if author.trim().is_empty() || author.chars().any(char::is_control) {
        return Err(SourceError::Refused {
            message: format!(
                "cannot represent the author {author:?}: an author here is written on one \
                 heading line, so it must say something and hold no line break or control \
                 character; next: give the name on one line"
            ),
        });
    }
    Ok(())
}
impl LocalMdSource {
    fn edges(
        &self,
        kind: WorkKind,
        id: &NativeId,
        d: Direction,
        p: &PageRequest,
    ) -> Result<Page<DependencyEdge>, SourceError> {
        let edges = self
            .parse_work_records(kind, &ProjectFilter::Any)?
            .into_iter()
            .flat_map(|x| x.dependencies)
            .filter(|e| match d {
                Direction::DependsOn => &e.from == id,
                Direction::DependedOnBy => &e.to == id,
            })
            .collect();
        self.paginate(edges, p)
    }
}
fn task(d: Entry) -> Task {
    Task {
        id: d.common.id,
        title: d.common.title,
        content: d.common.body,
        status: d.status,
        labels: d.common.labels,
        project: d.common.project,
        url: d.common.url,
        location: Some(d.common.location),
        created_at: None,
        updated_at: None,
        metadata: d.common.metadata,
        repositories: d.common.repositories,
        delivers: d.delivers,
        delivered_by: d.delivered_by,
    }
}
fn project(d: Entry) -> Project {
    Project {
        id: d.common.id,
        title: d.common.title,
        content: d.common.body,
        status: d.status,
        labels: d.common.labels,
        url: d.common.url,
        location: Some(d.common.location),
        created_at: None,
        updated_at: None,
        metadata: d.common.metadata,
        repositories: d.common.repositories,
    }
}

/// One item on its way into a Markdown file, and what its own kind carries.
///
/// The write path is written once over this rather than three times over the three
/// contract types — and it is an enum rather than one struct with optional members because
/// a document has no status and no edges while a task has both. Neither *a document with a
/// status* nor *a task without one* is a value this type can hold, so the write path has no
/// such case to get wrong.
enum Outgoing<'a> {
    /// A task or a project.
    Work {
        kind: WorkKind,
        status: &'a Status,
        depends_on: &'a [DependencyEdge],
        /// Empty for a project.
        delivers: &'a [TaskRef],
        /// Empty for a project.
        delivered_by: &'a [TaskRef],
        fields: Fields<'a>,
    },
    /// A document, which takes part in no dependency graph and has no status.
    Document { fields: Fields<'a> },
}

/// What every item on its way out carries, whichever kind it is.
struct Fields<'a> {
    id: &'a NativeId,
    title: &'a str,
    content: Option<&'a str>,
    labels: &'a [Label],
    project: Option<&'a NativeId>,
    metadata: &'a BTreeMap<String, serde_json::Value>,
    repositories: &'a [Repository],
}

impl<'a> Outgoing<'a> {
    /// Which of this source's folders this item is filed in.
    const fn kind(&self) -> Kind {
        match self {
            Self::Work { kind, .. } => kind.kind(),
            Self::Document { .. } => Kind::Document,
        }
    }

    const fn fields(&self) -> &Fields<'a> {
        match self {
            Self::Work { fields, .. } | Self::Document { fields } => fields,
        }
    }
}

/// The front matter this source writes, which is the subset of [`FrontMatter`] a copy
/// carries: `url` is the destination's own and is never written.
///
/// `status` is omitted entirely for a document, so what lands under `documents/` is a
/// [`DocumentFrontMatter`] — which refuses that key — rather than a task's front matter
/// with one value left blank.
#[derive(Serialize)]
struct WrittenFrontMatter {
    title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    labels: Vec<WrittenLabel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    project: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    depends_on: Vec<WrittenDependency>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    metadata: BTreeMap<String, serde_json::Value>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    repositories: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    delivers: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    delivered_by: Vec<String>,
}

#[derive(Serialize)]
struct WrittenLabel {
    id: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    color: Option<String>,
}

/// Always the expanded form, so the level and the kind of every far end are written down
/// rather than left to the shorthand's defaults.
#[derive(Serialize)]
struct WrittenDependency {
    id: String,
    kind: &'static str,
    item: &'static str,
}

/// This vocabulary's own spelling of a category, for a message a user has to act on.
fn category_name(category: StatusCategory) -> &'static str {
    match category {
        StatusCategory::Draft => "draft",
        StatusCategory::Backlog => "backlog",
        StatusCategory::Todo => "todo",
        StatusCategory::Queued => "queued",
        StatusCategory::InProgress => "in-progress",
        StatusCategory::Done => "done",
        StatusCategory::Cancelled => "cancelled",
        StatusCategory::Unknown => "unknown",
    }
}

/// The relative path, without its extension, a created document is filed under.
///
/// A native id is opaque and a path is not, so every character a path gives meaning to is
/// replaced rather than obeyed: `..` cannot be spelled, a separator cannot escape the
/// configured root, and a dot cannot make `a.b` and `a` name the same file once `.md` is
/// appended.
fn document_stem(id: &NativeId) -> Result<String, SourceError> {
    let parts: Vec<String> = id
        .as_str()
        .split(['/', '\\'])
        .map(|part| {
            part.chars()
                .map(|character| {
                    if character.is_ascii_alphanumeric() || matches!(character, '_' | '-') {
                        character
                    } else {
                        '-'
                    }
                })
                .collect::<String>()
        })
        .filter(|part| !part.trim_matches('-').is_empty())
        .collect();
    if parts.is_empty() {
        return Err(SourceError::Refused {
            message: format!(
                "{id} has no character a file name can be made of; next: copy it under an id \
                 carrying at least one letter, digit, underscore or dash"
            ),
        });
    }
    Ok(parts.join("/"))
}

impl LocalMdSource {
    /// Create or update one file, answering with the id it is filed under.
    fn write_entry(
        &self,
        target: Option<&NativeId>,
        outgoing: &Outgoing<'_>,
    ) -> Result<NativeId, SourceError> {
        let kind = outgoing.kind();
        let (id, path) = match target {
            Some(target) => (target.clone(), self.existing(kind, target)?),
            None => self.unused(kind, outgoing.fields().id)?,
        };
        let mut document = self.render(outgoing)?;
        // An update of a task keeps the comments section the file already has, byte for byte:
        // a copy writes the task, and nothing a copy does adds, changes or removes a comment.
        // Only a file can hold a section: anything else at that path is left for the write
        // below to report. A file that cannot be read is refused rather than overwritten,
        // because overwriting it would drop whatever comments it holds unread.
        if let (Some(_), Kind::Task) = (target, kind)
            && path.is_file()
        {
            let existing = fs::read_to_string(&path).map_err(|e| SourceError::Unavailable {
                message: format!(
                    "cannot read {} to keep its comments before writing it: {e}",
                    path.display()
                ),
            })?;
            if let Some((_, body_at)) = front_matter(&existing) {
                let body = &existing[body_at..];
                let (content, comments) = sectioned(body);
                if comments.is_some() {
                    document.push('\n');
                    document.push_str(&body[content.len()..]);
                }
            }
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| SourceError::Unavailable {
                message: format!("cannot create {}: {e}", parent.display()),
            })?;
        }
        fs::write(&path, document).map_err(|e| SourceError::Unavailable {
            message: format!("cannot write {}: {e}", path.display()),
        })?;
        Ok(id)
    }

    /// Remove one item, so a copy that could not finish leaves this folder as it was.
    ///
    /// An id naming no file is not an error: it is already gone, which is the state this
    /// asks for. `existing` refuses that case because an *update* of a missing item is a
    /// caller mistake, and this is not one.
    fn delete_entry(&self, kind: Kind, id: &NativeId) -> Result<(), SourceError> {
        let path = match self.existing(kind, id) {
            Ok(path) => path,
            Err(SourceError::Refused { .. }) => return Ok(()),
            Err(other) => return Err(other),
        };
        fs::remove_file(&path).map_err(|e| SourceError::Unavailable {
            message: format!("cannot remove {}: {e}", path.display()),
        })
    }

    /// The path of the item `id` names in that folder, refusing when there is no such file.
    fn existing(&self, kind: Kind, id: &NativeId) -> Result<PathBuf, SourceError> {
        let base = self.directory(kind)?;
        let candidate = base.join(&id.0).with_extension("md");
        if !candidate.exists() {
            return Err(SourceError::Refused {
                message: format!(
                    "{id} names no {} here; next: copy with --recreate to create one \
                     instead of updating",
                    kind.noun()
                ),
            });
        }
        let canonical = fs::canonicalize(&candidate).map_err(|e| SourceError::Malformed {
            message: format!("{}: {e}", candidate.display()),
        })?;
        if !canonical.starts_with(&base) {
            return Err(SourceError::Config {
                message: format!(
                    "{} escapes configured root {}",
                    candidate.display(),
                    self.root.display()
                ),
            });
        }
        Ok(canonical)
    }

    /// A path nothing in that folder occupies, and the id it will be read back under.
    fn unused(&self, kind: Kind, id: &NativeId) -> Result<(NativeId, PathBuf), SourceError> {
        let base = self.directory(kind)?;
        let stem = document_stem(id)?;
        for attempt in 1..=1_000_u32 {
            let candidate = if attempt == 1 {
                stem.clone()
            } else {
                format!("{stem}-{attempt}")
            };
            let path = base.join(format!("{candidate}.md"));
            if !path.exists() {
                return Ok((NativeId(candidate), path));
            }
        }
        Err(SourceError::Refused {
            message: format!(
                "every name from {stem} to {stem}-1000 is taken under {}; next: tidy that \
                 folder, or copy into a source whose ids this one does not already hold",
                base.display()
            ),
        })
    }

    /// Refuse a status this folder would read back as another category, in the one wording a
    /// copy and a status write share.
    fn representable_status(&self, status: &Status) -> Result<(), SourceError> {
        let mapped = self
            .statuses
            .get(&status.name.to_lowercase())
            .copied()
            .unwrap_or(StatusCategory::Unknown);
        if mapped == status.category {
            return Ok(());
        }
        Err(SourceError::Refused {
            message: format!(
                "cannot represent the field `status`: this source reads {:?} as {}, not {}; \
                 next: map {:?} to {} under this source's status_mapping",
                status.name,
                category_name(mapped),
                category_name(status.category),
                status.name,
                category_name(status.category),
            ),
        })
    }

    /// Refuse a task list naming its own task or one task twice, naming the field.
    fn representable_list(
        &self,
        field: &str,
        near: &NativeId,
        list: &[TaskRef],
    ) -> Result<(), SourceError> {
        TaskRef::listed(field, near, self.name.as_ref(), list.to_vec())
            .map(|_| ())
            .map_err(|message| SourceError::Refused {
                message: format!("cannot represent the field `{field}`: {message}"),
            })
    }

    /// The word this folder writes a status in `category` as.
    ///
    /// The category's own spelling when the mapping reads that word as it — `queued`, or
    /// `in progress` for `in-progress` — else the first word the mapping sends there, else the
    /// category's own spelling, which [`Self::representable_status`] then refuses by name.
    ///
    /// The spoken spelling is preferred over the hyphenated one where the mapping holds
    /// both, which the default mapping now does: `in-progress` is there so a task that
    /// persists the canonical word reads back as that category, and what a person reads in
    /// a file this source writes stays `in progress`. Stating the preference is what keeps
    /// that from resting on the order a `BTreeMap` happens to hold two words in.
    fn word_for(&self, category: StatusCategory) -> String {
        let spelled = category_name(category);
        let spoken = spelled.replace('-', " ");
        let words: Vec<&String> = self
            .statuses
            .iter()
            .filter(|(_, held)| **held == category)
            .map(|(word, _)| word)
            .collect();
        words
            .iter()
            .find(|word| word.as_str() == spoken)
            .or_else(|| words.iter().find(|word| word.as_str() == spelled))
            .or_else(|| words.first())
            .map_or_else(|| spelled.to_owned(), |word| (*word).clone())
    }

    /// Replace one top-level entry of a task file's front matter, leaving every other byte
    /// of the file as it was, and refuse a result this source could not read back.
    ///
    /// The file is replaced the way a metadata write replaces one — a staging file and a
    /// rename, by the rules [`STAGING_SUFFIX`] states — so a reader sees the task as it was
    /// or as it is now, and never part of either.
    fn rewrite_front_entry(
        &self,
        path: &Path,
        key: &str,
        value: Option<&str>,
    ) -> Result<(), SourceError> {
        let text = fs::read_to_string(path).map_err(|e| SourceError::Malformed {
            message: format!("{}: {e}", path.display()),
        })?;
        let rewritten = with_front_entry(&text, key, value).ok_or_else(|| unfronted(path))?;
        let Some((yaml, _)) = front_matter(&rewritten) else {
            return Err(unfronted(path));
        };
        serde_norway::from_str::<FrontMatter>(yaml).map_err(|e| SourceError::Malformed {
            message: format!(
                "{}: rewriting `{key}` would leave front matter this source cannot read: {e}; \
                 next: tidy that entry of the file by hand",
                path.display()
            ),
        })?;
        replace_atomically(path, &rewritten)
    }

    /// One file's whole text, or a refusal naming the field this source cannot hold.
    fn render(&self, outgoing: &Outgoing<'_>) -> Result<String, SourceError> {
        let is_task = matches!(
            outgoing,
            Outgoing::Work {
                kind: WorkKind::Task,
                ..
            }
        );
        let (status, depends_on, delivers, delivered_by) = match outgoing {
            Outgoing::Work {
                status,
                depends_on,
                delivers,
                delivered_by,
                ..
            } => (Some(*status), *depends_on, *delivers, *delivered_by),
            Outgoing::Document { .. } => (None, [].as_slice(), [].as_slice(), [].as_slice()),
        };
        if let Some(status) = status {
            self.representable_status(status)?;
        }
        let outgoing = outgoing.fields();
        let front = WrittenFrontMatter {
            title: outgoing.title.to_owned(),
            status: status.map(|status| status.name.clone()),
            labels: outgoing
                .labels
                .iter()
                .map(|label| WrittenLabel {
                    id: label.id.0.clone(),
                    name: label.name.clone(),
                    color: label.color.clone(),
                })
                .collect(),
            project: outgoing.project.map(|id| id.0.clone()),
            depends_on: depends_on
                .iter()
                .map(|edge| WrittenDependency {
                    id: edge.to.id().to_owned(),
                    kind: match edge.kind {
                        DependencyKind::Blocks => "blocks",
                        DependencyKind::Related => "related",
                    },
                    item: match edge.to.kind {
                        ItemKind::Task => "task",
                        ItemKind::Project => "project",
                    },
                })
                .collect(),
            metadata: outgoing.metadata.clone(),
            repositories: outgoing
                .repositories
                .iter()
                .map(|repository| repository.as_str().to_owned())
                .collect(),
            delivers: delivers.iter().map(ToString::to_string).collect(),
            delivered_by: delivered_by.iter().map(ToString::to_string).collect(),
        };
        let yaml = serde_norway::to_string(&front).map_err(|e| SourceError::Malformed {
            message: format!("cannot render front matter for {}: {e}", outgoing.id),
        })?;
        let body = outgoing.content.unwrap_or_default().trim();
        // Content that would itself read back as a comments section is refused: writing it
        // would turn part of a task's content into comments nobody wrote, which is the one
        // thing a copy may never do to a comment.
        if is_task && sectioned(&format!("{body}\n")).1.is_some() {
            return Err(SourceError::Refused {
                message: format!(
                    "cannot represent the field `content` of {}: it ends in a `{COMMENTS_HEADING}` \
                     section of comment blocks, which this source reads as the task's comments \
                     rather than its content; next: change that heading in the content being \
                     copied",
                    outgoing.id
                ),
            });
        }
        Ok(format!("---\n{}\n---\n{body}\n", yaml.trim_end()))
    }
}

/// The suffix of the staging file a narrow metadata write replaces a record through.
///
/// # A narrow metadata write, rule by rule
///
/// [`set_task_metadata`](TaskSource::set_task_metadata), and its project and document
/// siblings, edit **exactly one entry** of the record's front-matter `metadata:` block and
/// leave every other byte of the file as it was, line endings included:
///
/// - The entry is written on one line as `"<key>": <compact JSON>` at the block's own entry
///   indent. An entry already there for the key — whatever shape it was written in: a flow
///   value, a plain `key: value`, a nested block mapping, a block scalar with blank lines in
///   it, an indentless sequence — is replaced whole; a missing one is added after the block's
///   last entry; a file with no `metadata:` block gains one, entries indented two spaces, as
///   the last entry of its front matter. A key matches when its decoded spelling, quoted or
///   plain, is the key's own.
/// - A key already holding the value is no write at all: the file is not opened for writing
///   and no staging file is made.
/// - The edited text is read back before anything is written, through the reader a query of
///   that kind uses, and must be the record as it was with that one key set. When it is not,
///   or when the block cannot be edited narrowly — a one-line `metadata: {…}`, a tab, an entry
///   this source cannot find the key of, a key written twice — the write is refused naming
///   the file, and the file is left as it was. Nothing is ever reformatted.
/// - The new bytes go to a staging file beside the record, named
///   `.<file name>.<process id>-<counter>` followed by this suffix, which is then renamed over
///   the record, so a reader sees the old file or the new one and never part of either. A
///   file ending in this suffix is never listed as a task, a project or a document, and a
///   staging file whose write or rename fails is removed.
pub const STAGING_SUFFIX: &str = ".onetaskgraph-staging";

/// How many staging files this process has named, so two writes in it never share one.
static STAGED: AtomicU64 = AtomicU64::new(0);
static REPLACEMENTS: RwLock<()> = RwLock::new(());

// This lock carries no data that a panic could leave inconsistent: its only purpose is to
// keep readers outside the Windows interval in which an atomic replacement changes handles.
// Keep synchronizing after a panic instead of turning every later read and write into an error.
fn replacement_reader() -> RwLockReadGuard<'static, ()> {
    REPLACEMENTS
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn replacement_writer() -> RwLockWriteGuard<'static, ()> {
    REPLACEMENTS
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Why a narrow metadata write could not be made, and what to do about it.
struct Unnarrow {
    reason: String,
    next: &'static str,
}

impl Unnarrow {
    /// The refusal naming `path` and `key`.
    fn refusal(self, path: &Path, key: &MetadataKey) -> SourceError {
        SourceError::Refused {
            message: format!(
                "{}: cannot set the metadata key `{key}` without changing anything else: {}; \
                 next: {}",
                path.display(),
                self.reason,
                self.next
            ),
        }
    }
}

/// The next action for a `metadata:` block this source cannot find its way around.
const TIDY_BLOCK: &str = "write the file's `metadata:` as a block mapping indented with spaces, \
                          one `key: value` entry to a line, and set the key again";

impl LocalMdSource {
    /// Set `key` to `value` in the metadata of the record `id` names under `kind`, by the rules
    /// [`STAGING_SUFFIX`] states.
    ///
    /// `read` is the reader a query uses for that kind — which is what makes the verification
    /// a read of the right front matter — and `metadata` reaches the map a record holds.
    fn set_metadata<R: PartialEq>(
        &self,
        kind: Kind,
        id: &NativeId,
        key: &MetadataKey,
        value: &serde_json::Value,
        read: impl Fn(&Path, &str) -> Result<R, SourceError>,
        metadata: impl Fn(&mut R) -> &mut BTreeMap<String, serde_json::Value>,
    ) -> Result<Option<R>, SourceError> {
        let Some(path) = self.locate(kind, id)? else {
            return Ok(None);
        };
        let text = Self::read_text(&path)?;
        let mut record = read(&path, &text)?;
        if metadata(&mut record).get(key.as_str()) == Some(value) {
            return Ok(Some(record));
        }
        let written = compact(value);
        let alone = serde_norway::from_str::<BTreeMap<String, serde_json::Value>>(&format!(
            "value: {written}"
        ));
        if alone
            .ok()
            .and_then(|mut read| read.remove("value"))
            .as_ref()
            != Some(value)
        {
            return Err(Unnarrow {
                reason: format!(
                    "the value {written}, written as JSON, does not read back through this \
                     source's YAML as the same value"
                ),
                next: "set a value that reads back as itself, such as one without a control or \
                       line-separator character in a string",
            }
            .refusal(&path, key));
        }
        let edited = with_metadata_entry(&text, key.as_str(), value)
            .map_err(|unnarrow| unnarrow.refusal(&path, key))?;
        metadata(&mut record).insert(key.as_str().to_owned(), value.clone());
        let reread = read(&path, &edited).map_err(|error| {
            Unnarrow {
                reason: format!("the edited front matter would not read back: {error}"),
                next: TIDY_BLOCK,
            }
            .refusal(&path, key)
        })?;
        if reread != record {
            return Err(Unnarrow {
                reason: "the edited file would read back as more than that one key changed"
                    .to_owned(),
                next: TIDY_BLOCK,
            }
            .refusal(&path, key));
        }
        replace_atomically(&path, &edited)?;
        read(&path, &Self::read_text(&path)?).map(Some)
    }
}

/// `value` as compact JSON.
fn compact(value: &serde_json::Value) -> String {
    // A `serde_json::Value` always serializes: its map keys are strings.
    serde_json::to_string(value).expect("a JSON value renders")
}

/// Replace the file at `path` with `text` through a staging file beside it and a rename.
fn replace_atomically(path: &Path, text: &str) -> Result<(), SourceError> {
    let unavailable = |e: std::io::Error| SourceError::Unavailable {
        message: format!("cannot write {}: {e}", path.display()),
    };
    let _replacement = replacement_writer();
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let directory = path.parent().unwrap_or(Path::new("."));
    let staging = directory.join(format!(
        ".{name}.{}-{}{STAGING_SUFFIX}",
        std::process::id(),
        STAGED.fetch_add(1, Ordering::Relaxed)
    ));
    let permissions = fs::metadata(path).map_err(unavailable)?.permissions();
    let written = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staging)?;
        file.write_all(text.as_bytes())?;
        file.set_permissions(permissions)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&staging, path)
    })();
    written.map_err(|e| {
        // The staging file may never have been created; either way none is left behind.
        let _ = fs::remove_file(&staging);
        unavailable(e)
    })
}

/// One line of a front matter: where it starts and ends in the file, the end including its
/// line ending when it has one.
#[derive(Clone, Copy)]
struct Line {
    from: usize,
    to: usize,
}

/// `text` with the entry for `key` in its front matter's `metadata:` block set to `value`,
/// by the rules [`STAGING_SUFFIX`] states, or why that cannot be done narrowly.
fn with_metadata_entry(
    text: &str,
    key: &str,
    value: &serde_json::Value,
) -> Result<String, Unnarrow> {
    let (open, newline) = if text.starts_with("---\r\n") {
        ("---\r\n", "\r\n")
    } else {
        ("---\n", "\n")
    };
    // The one caller has already read a record out of this very text, which needs front matter.
    let (yaml, _) = front_matter(text).expect("a record was read from this text");
    let start = open.len();
    let end = start + yaml.len();
    let mut lines = Vec::new();
    let mut at = start;
    while at < end {
        let next = text[at..end].find('\n').map_or(end, |found| at + found + 1);
        lines.push(Line { from: at, to: next });
        at = next;
    }
    let content = |line: Line| text[line.from..line.to].trim_end_matches(['\r', '\n']);
    let written_key = serde_json::to_string(key).expect("a string renders");
    let entry = |indent: usize| format!("{}{written_key}: {}", " ".repeat(indent), compact(value));
    // A new line after `line`: that line's own ending ends it, or — for the last line of the
    // front matter, whose ending is the closing delimiter's — a new ending goes before it.
    let after = |line: Line, written: &str| {
        if text[..line.to].ends_with('\n') {
            format!("{}{written}{newline}{}", &text[..line.to], &text[line.to..])
        } else {
            format!("{}{newline}{written}{}", &text[..line.to], &text[line.to..])
        }
    };
    // Front matter holding `metadata` twice does not read at all, so the first is the one.
    let Some(block) = lines.iter().position(|&line| {
        !content(line).starts_with([' ', '\t'])
            && entry_key(content(line)).is_some_and(|(found, _)| found == "metadata")
    }) else {
        let written = format!("metadata:{newline}{}", entry(2));
        let separator = if end > start { newline } else { "" };
        return Ok(format!(
            "{}{separator}{written}{}",
            &text[..end],
            &text[end..]
        ));
    };
    let heading = content(lines[block]);
    let (_, colon) = entry_key(heading).expect("the block's own line has a key");
    let inline = heading[colon..].trim();
    if !inline.is_empty() && !inline.starts_with('#') {
        return Err(Unnarrow {
            reason: format!(
                "its `metadata` is written on one line as `{inline}` rather than as a block of \
                 entries"
            ),
            next: TIDY_BLOCK,
        });
    }
    let body: Vec<(usize, Line)> = lines
        .iter()
        .copied()
        .enumerate()
        .skip(block + 1)
        .take_while(|&(_, line)| {
            let line = content(line);
            line.trim().is_empty() || line.starts_with([' ', '\t'])
        })
        .collect();
    let indent_of = |line: &str| line.len() - line.trim_start_matches(' ').len();
    let entry_indent = body
        .iter()
        .map(|&(_, line)| content(line))
        .find(|line| !line.trim().is_empty() && !line.trim_start().starts_with('#'))
        .map(indent_of)
        .unwrap_or(2);
    // Each entry of the block: its decoded key, and its first and last line.
    let mut entries: Vec<(String, usize, usize)> = Vec::new();
    for &(index, line) in &body {
        let line = content(line);
        if line.trim().is_empty() {
            continue;
        }
        let indent = indent_of(line);
        let rest = &line[indent..];
        let number = index + 2;
        if rest.starts_with('\t') && indent <= entry_indent {
            return Err(Unnarrow {
                reason: format!("line {number} of the file is indented with a tab"),
                next: TIDY_BLOCK,
            });
        }
        let continues = indent > entry_indent
            || (indent == entry_indent && (rest == "-" || rest.starts_with("- ")));
        if continues && let Some(last) = entries.last_mut() {
            last.2 = index;
        } else if rest.starts_with('#') {
            // A comment belonging to no entry.
        } else if indent != entry_indent {
            return Err(Unnarrow {
                reason: format!(
                    "line {number} of the file is not indented as the other entries of its \
                     `metadata:` block"
                ),
                next: TIDY_BLOCK,
            });
        } else if let Some((found, _)) = entry_key(rest) {
            entries.push((found, index, index));
        } else {
            return Err(Unnarrow {
                reason: format!(
                    "line {number} of the file is not an entry this source can read the key of"
                ),
                next: TIDY_BLOCK,
            });
        }
    }
    let matching: Vec<&(String, usize, usize)> =
        entries.iter().filter(|(found, ..)| found == key).collect();
    match (matching.as_slice(), entries.last()) {
        ([], None) => Ok(after(lines[block], &entry(entry_indent))),
        ([], Some(&(_, _, last))) => Ok(after(lines[last], &entry(entry_indent))),
        (&[&(_, first, last)], _) => {
            let (from, to) = (lines[first].from, lines[last].to);
            let ending = if text[..to].ends_with('\n') {
                newline
            } else {
                ""
            };
            Ok(format!(
                "{}{}{ending}{}",
                &text[..from],
                entry(entry_indent),
                &text[to..]
            ))
        }
        _ => Err(Unnarrow {
            reason: format!("its `metadata:` block holds `{key}` more than once"),
            next: "remove all but one of those entries",
        }),
    }
}

/// The decoded key a mapping entry `line` starts with — double-quoted, single-quoted or
/// plain — and the byte just after the `:` that ends it, or `None` when `line` does not start
/// with a key this source can decode.
fn entry_key(line: &str) -> Option<(String, usize)> {
    let (key, after) = if let Some(rest) = line.strip_prefix('"') {
        let mut escaped = false;
        let close = rest
            .char_indices()
            .find_map(|(at, character)| match (escaped, character) {
                (true, _) => {
                    escaped = false;
                    None
                }
                (false, '\\') => {
                    escaped = true;
                    None
                }
                (false, '"') => Some(at),
                (false, _) => None,
            })?;
        let end = 1 + close + 1;
        (serde_json::from_str::<String>(&line[..end]).ok()?, end)
    } else if let Some(rest) = line.strip_prefix('\'') {
        let mut key = String::new();
        let mut characters = rest.char_indices().peekable();
        let close = loop {
            match characters.next()? {
                (_, '\'') if characters.peek().is_some_and(|&(_, next)| next == '\'') => {
                    characters.next();
                    key.push('\'');
                }
                (at, '\'') => break at,
                (_, character) => key.push(character),
            }
        };
        (key, 1 + close + 1)
    } else {
        if line.starts_with([
            '[', '{', '&', '*', '!', '|', '>', '%', '@', '`', '?', '#', ',',
        ]) || line == "-"
            || line.starts_with("- ")
        {
            return None;
        }
        let colon = line.match_indices(':').map(|(at, _)| at).find(|&at| {
            line[at + 1..]
                .chars()
                .next()
                .is_none_or(|next| next == ' ' || next == '\t')
        })?;
        (line[..colon].trim_end().to_owned(), colon)
    };
    let rest = &line[after..];
    let colon = after + (rest.len() - rest.trim_start_matches(' ').len());
    let tail = line[colon..].strip_prefix(':')?;
    tail.chars()
        .next()
        .is_none_or(|next| next == ' ' || next == '\t')
        .then_some((key, colon + 1))
}
