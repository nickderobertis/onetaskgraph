//! A stateless onetaskgraph source over hand-authored Markdown files.
#![deny(missing_docs)]

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use onetaskgraph_plugin_api::{
    Capabilities, Cursor, DependencyEdge, DependencyKind, DependencySupport, Direction, Health,
    Label, LabelFilter, NativeId, Page, PageRequest, Project, ProjectFilter, ProjectQuery,
    SecretResolver, SourceError, SourceName, SourcePlugin, Status, StatusCategory, Support, Task,
    TaskQuery, TaskSource, TextFields, TextQuery,
};
use schemars::{Schema, schema_for};
use serde::Deserialize;

/// The registry name for this plugin.
pub const KIND: &str = "local-md";
/// The largest page returned by a folder scan.
pub const MAX_PAGE_SIZE: u32 = 200;

/// Configuration for a Markdown folder source.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LocalMdConfig {
    /// Folder containing `tasks/` and `projects/`.
    pub root: PathBuf,
    /// Case-insensitive source status to normalized-category mapping.
    #[serde(default = "default_statuses")]
    pub status_mapping: BTreeMap<String, StatusCategory>,
}

fn default_statuses() -> BTreeMap<String, StatusCategory> {
    [
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
}
fn default_status() -> String {
    "todo".to_owned()
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
    },
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

struct Document {
    id: NativeId,
    title: String,
    body: Option<String>,
    status: Status,
    labels: Vec<Label>,
    project: Option<NativeId>,
    dependencies: Vec<DependencyEdge>,
    url: Option<String>,
}

#[derive(Debug, Clone, Copy)]
enum DocumentKind {
    Task,
    Project,
}

impl DocumentKind {
    const fn directory(self) -> &'static str {
        match self {
            Self::Task => "tasks",
            Self::Project => "projects",
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

    fn directory(&self, kind: DocumentKind) -> Result<PathBuf, SourceError> {
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

    fn paths(&self, kind: DocumentKind) -> Result<Vec<PathBuf>, SourceError> {
        fn visit(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), SourceError> {
            if !dir.exists() {
                return Ok(());
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
                    visit(root, &canonical, out)?;
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
        visit(&self.root, &directory, &mut paths)?;
        paths.sort();
        paths.dedup();
        Ok(paths)
    }

    fn parse(&self, kind: DocumentKind, path: &Path) -> Result<Document, SourceError> {
        // Both callers canonicalize and confine the path before parsing it. Keeping that
        // invariant explicit here makes future internal callers notice if they skip the
        // boundary check without duplicating an unreachable user-facing branch.
        debug_assert!(path.starts_with(&self.root));
        let text = fs::read_to_string(path).map_err(|e| SourceError::Malformed {
            message: format!("{}: {e}", path.display()),
        })?;
        let (yaml, body) = text
            .strip_prefix("---\n")
            .and_then(|rest| rest.split_once("\n---\n"))
            .ok_or_else(|| SourceError::Malformed {
                message: format!(
                    "{}: expected YAML front matter delimited by ---",
                    path.display()
                ),
            })?;
        let front: FrontMatter =
            serde_norway::from_str(yaml).map_err(|e| SourceError::Malformed {
                message: format!("{}: {e}", path.display()),
            })?;
        let base = self.directory(kind)?;
        let relative = path
            .strip_prefix(&base)
            .map_err(|_| SourceError::Malformed {
                message: format!("{} is outside {}", path.display(), base.display()),
            })?;
        let id = relative
            .with_extension("")
            .to_str()
            .ok_or_else(|| SourceError::Malformed {
                message: format!("{} is not a UTF-8 path", path.display()),
            })?
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
        let title = front.title.unwrap_or(fallback);
        let body = body.trim();
        let labels = front
            .labels
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
            .collect();
        let status = Status {
            category: self
                .statuses
                .get(&front.status.to_lowercase())
                .copied()
                .unwrap_or(StatusCategory::Unknown),
            name: front.status,
        };
        let from = NativeId(id.clone());
        let dependencies = front
            .depends_on
            .into_iter()
            .map(|d| {
                let (to, kind) = match d {
                    Dependency::Id(id) => (id, DependencyKind::Blocks),
                    Dependency::Detailed { id, kind } => (
                        id,
                        match kind {
                            EdgeKind::Blocks => DependencyKind::Blocks,
                            EdgeKind::Related => DependencyKind::Related,
                        },
                    ),
                };
                DependencyEdge {
                    from: from.clone(),
                    // llmlint: ignore[boundary_inputs_validated] Dependency targets use the frozen contract's deliberately opaque, unvalidated `NativeId`; rejecting a value here would narrow that public contract.
                    to: NativeId(to),
                    kind,
                }
            })
            .collect();
        Ok(Document {
            id: NativeId(id),
            title,
            body: (!body.is_empty()).then(|| body.to_owned()),
            status,
            labels,
            // llmlint: ignore[boundary_inputs_validated] Project references use the frozen contract's deliberately opaque, unvalidated `NativeId`; rejecting a value here would narrow that public contract.
            project: front.project.map(NativeId),
            dependencies,
            url: front.url,
        })
    }

    fn readable_documents(&self, kind: DocumentKind) -> Result<Vec<Document>, SourceError> {
        self.paths(kind)?
            .into_iter()
            .filter_map(|p| self.parse(kind, &p).ok())
            .collect::<Vec<_>>()
            .pipe(Ok)
    }
    fn find(&self, kind: DocumentKind, id: &NativeId) -> Result<Option<Document>, SourceError> {
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
        self.parse(kind, &canonical).map(Some)
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
    async fn health(&self) -> Result<Health, SourceError> {
        self.paths(DocumentKind::Task)?;
        self.paths(DocumentKind::Project)?;
        Ok(Health {
            reachable: true,
            detail: Some(format!("reading Markdown under {}", self.root.display())),
        })
    }
    async fn get_task(&self, id: &NativeId) -> Result<Option<Task>, SourceError> {
        Ok(self.find(DocumentKind::Task, id)?.map(task))
    }
    async fn get_project(&self, id: &NativeId) -> Result<Option<Project>, SourceError> {
        Ok(self.find(DocumentKind::Project, id)?.map(project))
    }
    async fn query_tasks(&self, q: &TaskQuery, p: &PageRequest) -> Result<Page<Task>, SourceError> {
        let items = self
            .readable_documents(DocumentKind::Task)?
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
            .readable_documents(DocumentKind::Project)?
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
    async fn labels(&self, p: &PageRequest) -> Result<Page<Label>, SourceError> {
        let mut seen = BTreeSet::new();
        let mut items: Vec<Label> = self
            .readable_documents(DocumentKind::Task)?
            .into_iter()
            .chain(self.readable_documents(DocumentKind::Project)?)
            .flat_map(|d| d.labels)
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
        self.edges(DocumentKind::Task, id, d, p)
    }
    async fn project_dependencies(
        &self,
        id: &NativeId,
        d: Direction,
        p: &PageRequest,
    ) -> Result<Page<DependencyEdge>, SourceError> {
        self.edges(DocumentKind::Project, id, d, p)
    }
}
impl LocalMdSource {
    fn edges(
        &self,
        kind: DocumentKind,
        id: &NativeId,
        d: Direction,
        p: &PageRequest,
    ) -> Result<Page<DependencyEdge>, SourceError> {
        let edges = self
            .readable_documents(kind)?
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
fn task(d: Document) -> Task {
    Task {
        id: d.id,
        title: d.title,
        content: d.body,
        status: d.status,
        labels: d.labels,
        project: d.project,
        url: d.url,
        created_at: None,
        updated_at: None,
    }
}
fn project(d: Document) -> Project {
    Project {
        id: d.id,
        title: d.title,
        content: d.body,
        status: d.status,
        labels: d.labels,
        url: d.url,
        created_at: None,
        updated_at: None,
    }
}
