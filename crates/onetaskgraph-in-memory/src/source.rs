//! The source itself and the factory that builds one.

use onetaskgraph_plugin_api::{
    Capabilities, Cursor, DependencyEdge, DependencySupport, Direction, Health, Label, NativeId,
    Page, PageRequest, Project, ProjectFilter, ProjectQuery, SecretResolver, SourceError,
    SourceName, SourcePlugin, Task, TaskQuery, TaskSource, TextFields, TextQuery,
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

/// A source that serves exactly the work it was constructed with.
#[derive(Debug, Clone)]
pub struct InMemorySource {
    config: InMemoryConfig,
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
        Ok(Self { config })
    }

    /// What this source declares, as its configuration set it.
    fn declared(&self) -> &CapabilityConfig {
        &self.config.capabilities
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
        Ok(Health {
            reachable: true,
            detail: Some(format!(
                "{} task(s), {} project(s) held in memory",
                self.config.tasks.len(),
                self.config.projects.len()
            )),
        })
    }

    async fn get_task(&self, id: &NativeId) -> Result<Option<Task>, SourceError> {
        Ok(self.config.tasks.iter().find(|t| &t.id == id).cloned())
    }

    async fn get_project(&self, id: &NativeId) -> Result<Option<Project>, SourceError> {
        Ok(self.config.projects.iter().find(|p| &p.id == id).cloned())
    }

    async fn query_tasks(
        &self,
        query: &TaskQuery,
        page: &PageRequest,
    ) -> Result<Page<Task>, SourceError> {
        let matched: Vec<Task> = self
            .config
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
            .config
            .projects
            .iter()
            .filter(|project| self.project_survives(project, query))
            .cloned()
            .collect();
        self.paginate(&matched, page)
    }

    async fn labels(&self, page: &PageRequest) -> Result<Page<Label>, SourceError> {
        self.paginate(&self.config.labels, page)
    }

    async fn task_dependencies(
        &self,
        id: &NativeId,
        direction: Direction,
        page: &PageRequest,
    ) -> Result<Page<DependencyEdge>, SourceError> {
        let edges = Self::edges(
            &self.config.task_dependencies,
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
            &self.config.project_dependencies,
            id,
            direction,
            self.declared().project_dependencies,
            "project dependencies",
        )?;
        self.paginate(&edges, page)
    }
}
