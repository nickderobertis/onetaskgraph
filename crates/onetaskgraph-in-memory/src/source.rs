//! The source itself and the factory that builds one.

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard};

use onetaskgraph_plugin_api::{
    Capabilities, Cursor, DependencyEdge, DependencyEndpoint, DependencySupport, Direction, Health,
    ItemKind, ItemWrite, Label, NativeId, Page, PageRequest, Project, ProjectFilter, ProjectQuery,
    SecretResolver, SourceError, SourceName, SourcePlugin, Task, TaskQuery, TaskSource, TextFields,
    TextQuery, WriteSupport, unwritable,
};
use schemars::{Schema, schema_for};

use crate::config::{CapabilityConfig, InMemoryConfig};
use crate::filter::{labels_match, status_matches, text_matches};

/// The plugin kind an `in-memory` source's `plugin:` field names.
pub const KIND: &str = "in-memory";

/// The factory for [`InMemorySource`].
#[derive(Debug, Clone, Copy, Default)]
pub struct Plugin;

impl SourcePlugin for Plugin {
    fn kind(&self) -> &'static str {
        KIND
    }

    fn config_schema(&self) -> Schema {
        schema_for!(InMemoryConfig)
    }

    fn build(
        &self,
        name: &SourceName,
        config: &serde_json::Value,
        _secrets: &dyn SecretResolver,
    ) -> Result<Box<dyn TaskSource>, SourceError> {
        let config: InMemoryConfig =
            serde_json::from_value(config.clone()).map_err(|error| SourceError::Config {
                message: format!("source {name}: {error}"),
            })?;
        // Shape is all serde checked; InMemorySource::new refuses the rest, and names
        // the source this configuration block belongs to.
        InMemorySource::new(config)
            .map(|source| Box::new(source) as Box<dyn TaskSource>)
            .map_err(|error| match error {
                SourceError::Config { message } => SourceError::Config {
                    message: format!("source {name}: {message}"),
                },
                other => other,
            })
    }
}

/// The work this source serves, which a write to it changes.
///
/// Separate from the capability block on purpose: what a source *declares* is fixed by
/// the configuration that built it and never moves, while the work behind it does. One
/// mutex over the work alone means a read of the declaration never contends with a write,
/// and means no lock is ever held while the declaration is being consulted.
#[derive(Debug, Default)]
struct Held {
    tasks: Vec<Task>,
    projects: Vec<Project>,
    labels: Vec<Label>,
    task_dependencies: Vec<DependencyEdge>,
    project_dependencies: Vec<DependencyEdge>,
}

/// A source that serves exactly the work it was constructed with, plus whatever has been
/// written into it since.
///
/// A write lands in this process and nowhere else, which is the whole of what an
/// in-memory source is: it holds no file, so nothing of a user's work outlives the run.
#[derive(Debug)]
pub struct InMemorySource {
    capabilities: CapabilityConfig,
    held: Mutex<Held>,
}

impl InMemorySource {
    /// Build a source over `config`, refusing one it could not serve coherently.
    ///
    /// The check belongs here rather than beside the constructor: an [`InMemoryConfig`]
    /// is deserialized straight out of a user's file, shape is all serde can check, and a
    /// duplicate id or a dangling edge makes later queries answer *wrongly* rather than
    /// loudly. Validating on the way in is what keeps an incoherent source from existing.
    pub fn new(config: InMemoryConfig) -> Result<Self, SourceError> {
        config
            .validate()
            .map_err(|message| SourceError::Config { message })?;
        Ok(Self {
            capabilities: config.capabilities,
            held: Mutex::new(Held {
                tasks: config.tasks,
                projects: config.projects,
                labels: config.labels,
                task_dependencies: config.task_dependencies,
                project_dependencies: config.project_dependencies,
            }),
        })
    }

    /// What this source declares, as its configuration set it.
    fn declared(&self) -> &CapabilityConfig {
        &self.capabilities
    }

    /// The work behind this source.
    ///
    /// A poisoned mutex means a previous caller panicked mid-write, so what is behind it
    /// may be half a write. Saying so is the contract's [`SourceError::Unavailable`]
    /// rather than a second panic, which would take the whole query down with it.
    fn held(&self) -> Result<MutexGuard<'_, Held>, SourceError> {
        self.held.lock().map_err(|_| SourceError::Unavailable {
            message: "this source's work was left half-written by an earlier panic".to_owned(),
        })
    }

    /// Slice `items` into the page `page` asks for.
    ///
    /// The cursor is this plugin's own encoding — a decimal offset — and stays
    /// opaque to the engine, which is why it round-trips through a string.
    fn paginate<T: Clone>(&self, items: &[T], page: &PageRequest) -> Result<Page<T>, SourceError> {
        let start = match &page.cursor {
            None => 0usize,
            Some(Cursor(raw)) => {
                let offset = raw.parse::<usize>().map_err(|_| SourceError::Malformed {
                    message: format!("cursor {raw:?} was not issued by the in-memory source"),
                })?;
                // Parsing is not the same as being ours. `next` is only ever `Some` while
                // `end < items.len()`, so every cursor this source hands out addresses a row
                // that exists — an offset at or past the end is one it never issued, and
                // answering it with an empty page would look like a walk that simply ended.
                if offset >= items.len() {
                    return Err(SourceError::Malformed {
                        message: format!(
                            "cursor {raw:?} was not issued by the in-memory source; it points \
                             past the {} result(s) available",
                            items.len()
                        ),
                    });
                }
                offset
            }
        };
        // A page of no rows is not a page. Refuse it rather than quietly serving one row,
        // which would turn a caller's bug into a walk that never advances.
        if page.limit == 0 {
            return Err(SourceError::Config {
                message: "a page limit of 0 is not a page; ask for at least 1 row".to_owned(),
            });
        }
        // At least 1 by its type, so this can never narrow a page to nothing.
        let ceiling = self.declared().max_page_size.get();
        let limit = page.limit.min(ceiling) as usize;
        let end = start.saturating_add(limit).min(items.len());
        let window = items.get(start..end).unwrap_or_default().to_vec();
        let next = (end < items.len()).then(|| Cursor(end.to_string()));
        Ok(Page {
            items: window,
            next,
        })
    }

    /// Whether a task survives every predicate this source declared `Native`.
    fn task_matches(&self, task: &Task, query: &TaskQuery) -> bool {
        let declared = self.declared();

        if declared.filter_by_label.is_native() && !labels_match(&task.labels, &query.labels) {
            return false;
        }
        if declared.filter_by_status.is_native()
            && !status_matches(task.status.category, &query.statuses)
        {
            return false;
        }
        if !self.project_matches(task, &query.project) {
            return false;
        }
        self.text_survives(&task.title, task.content.as_deref(), query.text.as_ref())
    }

    /// Whether a task survives the project predicate.
    ///
    /// `Orphans` is gated on `orphan_tasks` and `Is(..)` on `projects`, because a
    /// source without projects cannot honour either and must return the wider set.
    fn project_matches(&self, task: &Task, filter: &ProjectFilter) -> bool {
        let declared = self.declared();
        match filter {
            ProjectFilter::Any => true,
            ProjectFilter::Orphans => !declared.orphan_tasks.is_native() || task.project.is_none(),
            ProjectFilter::Is(id) => {
                !declared.projects.is_native() || task.project.as_ref() == Some(id)
            }
        }
    }

    /// Whether a project survives every predicate this source declared `Native`.
    fn project_survives(&self, project: &Project, query: &ProjectQuery) -> bool {
        let declared = self.declared();

        if declared.filter_by_label.is_native() && !labels_match(&project.labels, &query.labels) {
            return false;
        }
        if declared.filter_by_status.is_native()
            && !status_matches(project.status.category, &query.statuses)
        {
            return false;
        }
        self.text_survives(
            &project.title,
            project.content.as_deref(),
            query.text.as_ref(),
        )
    }

    /// Apply the text predicate only when this source applies **every** half the
    /// query asks about.
    ///
    /// Half-applying it would break rule 2. A `title-or-content` search against a
    /// source that searches titles but not bodies would drop every row matching
    /// only in the body — a *narrower* result than the truth, which is the one
    /// failure the engine cannot compensate for. So a predicate this source
    /// cannot fully honour is ignored outright and the wider set goes back.
    fn text_survives(&self, title: &str, content: Option<&str>, query: Option<&TextQuery>) -> bool {
        let Some(query) = query else { return true };
        let declared = self.declared();
        let can_apply = match query.fields {
            TextFields::Title => declared.search_title.is_native(),
            TextFields::Content => declared.search_content.is_native(),
            TextFields::TitleOrContent => {
                declared.search_title.is_native() && declared.search_content.is_native()
            }
        };
        !can_apply || text_matches(title, content, query)
    }

    /// The edges at `id`, in `direction`, over one forward edge list.
    ///
    /// A source declaring [`DependencySupport::ForwardOnly`] refuses the reverse
    /// direction rather than answering it emptily: the engine emulates that
    /// direction itself, so a call arriving here is a bug worth naming, and rule 3
    /// of the contract forbids a silently empty dependency read.
    fn edges(
        edges: &[DependencyEdge],
        id: &NativeId,
        direction: Direction,
        support: DependencySupport,
        what: &str,
    ) -> Result<Vec<DependencyEdge>, SourceError> {
        match direction {
            Direction::DependsOn => Ok(edges
                .iter()
                .filter(|edge| &edge.from == id)
                .cloned()
                .collect()),
            Direction::DependedOnBy if support.answers_reverse() => Ok(edges
                .iter()
                .filter(|edge| &edge.to == id)
                .cloned()
                .collect()),
            Direction::DependedOnBy => Err(SourceError::Refused {
                message: format!(
                    "this source declares {what} as forward-only, so the engine emulates the \
                     reverse direction; it must not be asked for it"
                ),
            }),
        }
    }
}

#[async_trait::async_trait]
impl TaskSource for InMemorySource {
    fn kind(&self) -> &'static str {
        KIND
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::from(self.declared())
    }

    async fn health(&self) -> Result<Health, SourceError> {
        let held = self.held()?;
        Ok(Health {
            reachable: true,
            detail: Some(format!(
                "{} task(s), {} project(s) held in memory",
                held.tasks.len(),
                held.projects.len()
            )),
        })
    }

    async fn get_task(&self, id: &NativeId) -> Result<Option<Task>, SourceError> {
        Ok(self.held()?.tasks.iter().find(|t| &t.id == id).cloned())
    }

    async fn get_project(&self, id: &NativeId) -> Result<Option<Project>, SourceError> {
        Ok(self.held()?.projects.iter().find(|p| &p.id == id).cloned())
    }

    async fn query_tasks(
        &self,
        query: &TaskQuery,
        page: &PageRequest,
    ) -> Result<Page<Task>, SourceError> {
        let matched: Vec<Task> = self
            .held()?
            .tasks
            .iter()
            .filter(|task| self.task_matches(task, query))
            .cloned()
            .collect();
        self.paginate(&matched, page)
    }

    /// A source declaring `projects: unsupported` has no project table at all,
    /// and the "wider set" of a table that does not exist is the empty one — so
    /// this short-circuits rather than ignoring the predicate the way rule 2 asks
    /// of a *filter*.
    async fn query_projects(
        &self,
        query: &ProjectQuery,
        page: &PageRequest,
    ) -> Result<Page<Project>, SourceError> {
        if !self.declared().projects.is_native() {
            return Ok(Page::last(Vec::new()));
        }
        let matched: Vec<Project> = self
            .held()?
            .projects
            .iter()
            .filter(|project| self.project_survives(project, query))
            .cloned()
            .collect();
        self.paginate(&matched, page)
    }

    async fn labels(&self, page: &PageRequest) -> Result<Page<Label>, SourceError> {
        let labels = self.held()?.labels.clone();
        self.paginate(&labels, page)
    }

    async fn task_dependencies(
        &self,
        id: &NativeId,
        direction: Direction,
        page: &PageRequest,
    ) -> Result<Page<DependencyEdge>, SourceError> {
        let edges = Self::edges(
            &self.held()?.task_dependencies,
            id,
            direction,
            self.declared().task_dependencies,
            "task dependencies",
        )?;
        self.paginate(&edges, page)
    }

    async fn project_dependencies(
        &self,
        id: &NativeId,
        direction: Direction,
        page: &PageRequest,
    ) -> Result<Page<DependencyEdge>, SourceError> {
        let edges = Self::edges(
            &self.held()?.project_dependencies,
            id,
            direction,
            self.declared().project_dependencies,
            "project dependencies",
        )?;
        self.paginate(&edges, page)
    }

    fn writes(&self) -> WriteSupport {
        self.declared().writes
    }

    async fn write_task(&self, write: &ItemWrite<Task>) -> Result<NativeId, SourceError> {
        self.writable(&write.item.metadata)?;
        let mut held = self.held()?;
        let id = match &write.target {
            Some(target) => {
                let position = position_of(held.tasks.iter().map(|task| &task.id), target)
                    .ok_or_else(|| missing(target, "task"))?;
                held.tasks[position] = Task {
                    id: target.clone(),
                    ..write.item.clone()
                };
                target.clone()
            }
            None => {
                let id = unused(held.tasks.iter().map(|task| &task.id), &write.item.id);
                held.tasks.push(Task {
                    id: id.clone(),
                    ..write.item.clone()
                });
                id
            }
        };
        held.adopt_labels(&write.item.labels);
        let edges = rooted(&write.depends_on, &id, ItemKind::Task);
        replace_edges(&mut held.task_dependencies, &id, edges);
        Ok(id)
    }

    async fn write_project(&self, write: &ItemWrite<Project>) -> Result<NativeId, SourceError> {
        self.writable(&write.item.metadata)?;
        let mut held = self.held()?;
        let id = match &write.target {
            Some(target) => {
                let position = position_of(held.projects.iter().map(|project| &project.id), target)
                    .ok_or_else(|| missing(target, "project"))?;
                held.projects[position] = Project {
                    id: target.clone(),
                    ..write.item.clone()
                };
                target.clone()
            }
            None => {
                let id = unused(
                    held.projects.iter().map(|project| &project.id),
                    &write.item.id,
                );
                held.projects.push(Project {
                    id: id.clone(),
                    ..write.item.clone()
                });
                id
            }
        };
        held.adopt_labels(&write.item.labels);
        let edges = rooted(&write.depends_on, &id, ItemKind::Project);
        replace_edges(&mut held.project_dependencies, &id, edges);
        Ok(id)
    }
}

impl InMemorySource {
    /// Refuse a write this source's configuration says it cannot take.
    ///
    /// Both refusals name what a caller has to change: the plugin, when there is no write
    /// side at all, and every key this source cannot carry rather than the first — someone
    /// correcting a document wants the whole list, not one round trip per key.
    fn writable(&self, metadata: &BTreeMap<String, serde_json::Value>) -> Result<(), SourceError> {
        if !self.declared().writes.is_supported() {
            return Err(unwritable(KIND));
        }
        // Sorted rather than in the order the configuration happened to list them, so the
        // message a caller reads does not reshuffle when the document is reordered.
        let mut refused: Vec<&str> = self
            .declared()
            .unwritable_metadata_keys
            .iter()
            .filter(|key| metadata.contains_key(key.as_str()))
            .map(String::as_str)
            .collect();
        refused.sort_unstable();
        if refused.is_empty() {
            return Ok(());
        }
        Err(SourceError::Refused {
            message: format!(
                "cannot carry the metadata key(s) {}; next: remove them from the item being \
                 copied, or copy into a source that holds them",
                refused.join(", ")
            ),
        })
    }
}

/// Where `id` sits among the ids given, or `None` when it sits nowhere.
fn position_of<'a>(ids: impl Iterator<Item = &'a NativeId>, id: &NativeId) -> Option<usize> {
    ids.enumerate()
        .find(|(_, held)| *held == id)
        .map(|(position, _)| position)
}

/// The refusal for a `target` this source does not hold.
fn missing(id: &NativeId, what: &str) -> SourceError {
    SourceError::Refused {
        message: format!(
            "{id} names no {what} this source holds; next: copy with --recreate to create one \
             instead of updating"
        ),
    }
}

/// `wanted` when nothing holds it, or the first `wanted-N` that is free.
///
/// A destination decides its own ids: the id an item was read under at its source is a
/// suggestion, and taking it verbatim when something else already answers to it would make
/// which item a lookup returns arbitrary.
fn unused<'a>(ids: impl Iterator<Item = &'a NativeId>, wanted: &NativeId) -> NativeId {
    let taken: Vec<&NativeId> = ids.collect();
    if !taken.contains(&wanted) {
        return wanted.clone();
    }
    (2_u32..)
        .map(|attempt| NativeId(format!("{wanted}-{attempt}")))
        .find(|candidate| !taken.contains(&candidate))
        .expect("an unbounded suffix eventually clears a finite set of ids")
}

/// The written edges, with this source's own id as the near end of each.
fn rooted(depends_on: &[DependencyEdge], near: &NativeId, kind: ItemKind) -> Vec<DependencyEdge> {
    depends_on
        .iter()
        .map(|edge| DependencyEdge {
            from: DependencyEndpoint::from_native(near.clone(), kind),
            to: edge.to.clone(),
            kind: edge.kind,
        })
        .collect()
}

/// Replace every forward edge at `near` with `edges`.
///
/// A write says what an item depends on now, so an edge the copy no longer carries is one
/// the source no longer has — leaving it would make a second copy of an item whose
/// dependency was removed keep depending on it.
fn replace_edges(held: &mut Vec<DependencyEdge>, near: &NativeId, edges: Vec<DependencyEdge>) {
    held.retain(|edge| &edge.from != near);
    held.extend(edges);
}

impl Held {
    /// Learn any label a written item carries that this source did not already know.
    ///
    /// Keyed by id, because that is what `labels` answers with and what a duplicate would
    /// make ambiguous.
    fn adopt_labels(&mut self, labels: &[Label]) {
        for label in labels {
            if !self.labels.iter().any(|held| held.id == label.id) {
                self.labels.push(label.clone());
            }
        }
    }
}
