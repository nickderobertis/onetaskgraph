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
//! | `orphan_tasks` | **Supported and proven.** A task document with no `project:` key belongs to none. |
//! | `filter_by_label` | **Supported and proven,** over the `labels:` key, requiring every label asked for and excluding every label refused. |
//! | `filter_by_status` | **Supported and proven,** over `status:` through this instance's own `status_mapping`. |
//! | `search_title` | **Supported and proven,** over the `title:` key. |
//! | `search_content` | **Supported and proven,** over the document body below the front matter. |
//! | `task_dependencies` | **Supported and proven,** in both directions: the reverse read scans the folder's own `depends_on` keys, which is a read of data already in hand rather than an index. |
//! | `project_dependencies` | **Supported and proven,** in both directions, the same way. |
//! | `max_page_size` | **Supported and proven.** [`MAX_PAGE_SIZE`], the largest page one folder scan returns. |
//!
//! # Where this source says an entity is
//!
//! Every task, project and document this source reports carries a `Location::Path` naming
//! the canonicalized absolute path of the file it was read from — the same path this source
//! already computes, because the configured root and every traversed path are canonicalized
//! and an identifier escaping the root is refused. That is what the location contract is
//! for on this backend: a reader holding one of these entities can print the path or read
//! the contents out for a person, knowing nothing about this plugin.
#![deny(missing_docs)]

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use onetaskgraph_plugin_api::{
    Capabilities, Cursor, DependencyEdge, DependencyEndpoint, DependencyKind, DependencySupport,
    Direction, Document, DocumentQuery, Health, ItemKind, ItemWrite, Label, LabelFilter, Location,
    NativeId, Page, PageRequest, Project, ProjectFilter, ProjectQuery, Repository, SecretResolver,
    SourceError, SourceName, SourcePlugin, Status, StatusCategory, Support, Task, TaskQuery,
    TaskSource, TextFields, TextQuery, WriteSupport,
};
use schemars::{Schema, schema_for};
use serde::{Deserialize, Serialize};

/// The registry name for this plugin.
pub const KIND: &str = "local-md";
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

fn default_statuses() -> BTreeMap<String, StatusCategory> {
    [
        ("draft", StatusCategory::Draft),
        ("backlog", StatusCategory::Backlog),
        ("todo", StatusCategory::Todo),
        ("in progress", StatusCategory::InProgress),
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
            .map(|s| Box::new(s) as Box<dyn TaskSource>)
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
    // llmlint: ignore[invalid_states_unrepresentable] `NativeId` is deliberately an opaque, unvalidated string in the frozen plugin contract (`onetaskgraph-plugin-api/src/id.rs`); replacing this wire value with a stricter local identifier would reject values the public type expressly permits.
    project: Option<String>,
    #[serde(default)]
    depends_on: Vec<Dependency>,
    // llmlint: ignore[invalid_states_unrepresentable, boundary_inputs_validated] `Task::url` and `Project::url` are frozen as `Option<String>` in the plugin contract, which permits source-native URL-like values; parsing here would narrow that approved boundary and is the contract owner's decision.
    url: Option<String>,
    #[serde(default)]
    metadata: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    repositories: Vec<Repository>,
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
    project: Option<String>,
    url: Option<String>,
    metadata: BTreeMap<String, serde_json::Value>,
    repositories: Vec<Repository>,
}

impl FrontMatter {
    /// This front matter split into what every kind carries, and what only work does.
    fn split(self) -> (SharedFront, String, Vec<Dependency>) {
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
        )
    }
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
    // llmlint: ignore[invalid_states_unrepresentable] `NativeId` is deliberately an opaque, unvalidated string in the frozen plugin contract (`onetaskgraph-plugin-api/src/id.rs`); replacing this wire value with a stricter local identifier would reject values the public type expressly permits.
    project: Option<String>,
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
            statuses: config
                .status_mapping
                .into_iter()
                .map(|(k, v)| (k.to_lowercase(), v))
                .collect(),
        })
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
            for entry in fs::read_dir(dir).map_err(|e| SourceError::Unavailable {
                message: format!("cannot read {}: {e}", dir.display()),
            })? {
                // llmlint: ignore[changed_behavior_has_e2e] An iterator failing after
                // `read_dir` succeeds is an OS/filesystem race that cannot be induced
                // deterministically without mocking the layer under test.
                let entry = entry.map_err(|e| SourceError::Unavailable {
                    message: format!("cannot read entry in {}: {e}", dir.display()),
                })?;
                let path = entry.path();
                let canonical = fs::canonicalize(&path).map_err(|e| SourceError::Malformed {
                    message: format!("{}: {e}", path.display()),
                })?;
                if !canonical.starts_with(root) {
                    return Err(SourceError::Config {
                        message: format!(
                            "{} escapes configured root {}",
                            path.display(),
                            root.display()
                        ),
                    });
                }
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

    /// One file's YAML front matter and its body, split apart.
    fn read_split(path: &Path) -> Result<(String, String), SourceError> {
        let text = fs::read_to_string(path).map_err(|e| SourceError::Malformed {
            message: format!("{}: {e}", path.display()),
        })?;
        text.strip_prefix("---\n")
            .and_then(|rest| rest.split_once("\n---\n"))
            .or_else(|| {
                text.strip_prefix("---\r\n")
                    .and_then(|rest| rest.split_once("\r\n---\r\n"))
            })
            .map(|(yaml, body)| (yaml.to_owned(), body.to_owned()))
            .ok_or_else(|| SourceError::Malformed {
                message: format!(
                    "{}: expected YAML front matter delimited by ---",
                    path.display()
                ),
            })
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
            // llmlint: ignore[boundary_inputs_validated] Project references use the frozen contract's deliberately opaque, unvalidated `NativeId`; rejecting a value here would narrow that public contract.
            project: front.project.map(NativeId),
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
        // Both callers canonicalize and confine the path before parsing it. Keeping that
        // invariant explicit here makes future internal callers notice if they skip the
        // boundary check without duplicating an unreachable user-facing branch.
        debug_assert!(path.starts_with(&self.root));
        let (yaml, body) = Self::read_split(path)?;
        let front: FrontMatter =
            serde_norway::from_str(&yaml).map_err(|e| SourceError::Malformed {
                message: format!("{}: {e}", path.display()),
            })?;
        let (shared, status, depends_on) = front.split();
        let common = self.common(kind.kind(), path, &body, shared)?;
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
        Ok(Entry {
            common,
            status,
            dependencies,
        })
    }

    /// One file under `documents/`, read straight into the contract's own type.
    ///
    /// There is no `Entry` on this path because there is nothing to hold in one: a
    /// document has no status and no edges, so what a work item's parse computes for those
    /// two has nothing here to compute it from.
    fn parse_document(&self, path: &Path) -> Result<Document, SourceError> {
        debug_assert!(path.starts_with(&self.root));
        let (yaml, body) = Self::read_split(path)?;
        let front: DocumentFrontMatter =
            serde_norway::from_str(&yaml).map_err(|e| SourceError::Malformed {
                message: format!("{}: {e}", path.display()),
            })?;
        let common = self.common(Kind::Document, path, &body, front.into())?;
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

    fn readable_work(&self, kind: WorkKind) -> Result<Vec<Entry>, SourceError> {
        self.paths(kind.kind())?
            .into_iter()
            .filter_map(|p| self.parse(kind, &p).ok())
            .collect::<Vec<_>>()
            .pipe(Ok)
    }

    fn readable_documents(&self) -> Result<Vec<Document>, SourceError> {
        self.paths(Kind::Document)?
            .into_iter()
            .filter_map(|p| self.parse_document(&p).ok())
            .collect::<Vec<_>>()
            .pipe(Ok)
    }

    /// The confined canonical path `id` names under `kind`, when this source holds one.
    fn locate(&self, kind: Kind, id: &NativeId) -> Result<Option<PathBuf>, SourceError> {
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

trait Pipe: Sized {
    fn pipe<T>(self, f: impl FnOnce(Self) -> T) -> T {
        f(self)
    }
}
impl<T> Pipe for T {}

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
            .readable_work(WorkKind::Task)?
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
            .readable_work(WorkKind::Project)?
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
            .readable_documents()?
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
            .readable_work(WorkKind::Task)?
            .into_iter()
            .chain(self.readable_work(WorkKind::Project)?)
            .flat_map(|d| d.common.labels)
            .chain(
                self.readable_documents()?
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
        self.write_entry(
            write.target.as_ref(),
            &Outgoing::Work {
                kind: WorkKind::Task,
                status: &task.status,
                depends_on: &write.depends_on,
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
    async fn delete_task(&self, id: &NativeId) -> Result<(), SourceError> {
        self.delete_entry(Kind::Task, id)
    }
    async fn delete_project(&self, id: &NativeId) -> Result<(), SourceError> {
        self.delete_entry(Kind::Project, id)
    }
    async fn delete_document(&self, id: &NativeId) -> Result<(), SourceError> {
        self.delete_entry(Kind::Document, id)
    }
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
            .readable_work(kind)?
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
        let document = self.render(outgoing)?;
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

    /// One file's whole text, or a refusal naming the field this source cannot hold.
    fn render(&self, outgoing: &Outgoing<'_>) -> Result<String, SourceError> {
        let (status, depends_on) = match outgoing {
            Outgoing::Work {
                status, depends_on, ..
            } => (Some(*status), *depends_on),
            Outgoing::Document { .. } => (None, [].as_slice()),
        };
        if let Some(status) = status {
            let mapped = self
                .statuses
                .get(&status.name.to_lowercase())
                .copied()
                .unwrap_or(StatusCategory::Unknown);
            if mapped != status.category {
                return Err(SourceError::Refused {
                    message: format!(
                        "cannot represent the field `status`: this source reads {:?} as {}, not \
                         {}; next: map {:?} to {} under this source's status_mapping",
                        status.name,
                        category_name(mapped),
                        category_name(status.category),
                        status.name,
                        category_name(status.category),
                    ),
                });
            }
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
        };
        let yaml = serde_norway::to_string(&front).map_err(|e| SourceError::Malformed {
            message: format!("cannot render front matter for {}: {e}", outgoing.id),
        })?;
        let body = outgoing.content.unwrap_or_default().trim();
        Ok(format!("---\n{}\n---\n{body}\n", yaml.trim_end()))
    }
}
