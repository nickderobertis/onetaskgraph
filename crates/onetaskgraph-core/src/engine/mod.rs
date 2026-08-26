//! The query engine: fan-out, capability compensation, and the plan it reports.
//!
//! Given the sources a configuration resolved to and a query, the engine addresses one,
//! several or every one of them **at once**, and decides per source and per predicate
//! whether to push the predicate down or to apply it itself. A predicate a source
//! declares [`Native`](Support::Native) is passed in the query and never re-applied. A
//! predicate it declares [`Unsupported`](Support::Unsupported) is **removed** from what
//! that source sees — the source would ignore it anyway, and leaving it in would invite
//! a source to half-apply it — and the engine narrows the wider result set in memory.
//!
//! What makes that worth the machinery is that the two are not the same plan. A source
//! with real server-side search keeps it, and a folder of Markdown beside it is
//! compensated for, and the caller can see which of the two it got: every response
//! carries a [`QueryPlan`], `--explain` renders it, and `--json` publishes it.
//!
//! Nothing here writes anything down. See [`fetch`] for the walk that makes that true.

mod fetch;
mod join;
mod local;
mod resume;

use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::sync::atomic::{AtomicU32, Ordering};

use onetaskgraph_plugin_api::{
    Capabilities, Cursor, DependencyEdge, Direction, Label, LabelFilter, NativeId, Page,
    PageRequest, Project, ProjectFilter, ProjectQuery, SecretResolver, SourceError, SourceName,
    StatusCategory, Task, TaskQuery, TextFields, TextQuery,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::GlobalId;
use crate::config::Config;
use crate::plan::{PageToken, Predicate, QueryPlan, QueryResponse, SourceFailure, SourcePlan};
use crate::resolve::{ResolvedSource, UnavailableSource, resolve_available};

use fetch::{Fetched, Stream, fits, merge, walk};
use join::join_all;
use local::{LocalProjects, LocalTasks};
pub(crate) use resume::{Owed, Resumption, StreamState};
use resume::{Resume, StreamKind};

pub use local::ProjectSelector;

/// One item, under the qualified id the engine addresses it by.
///
/// A plugin only ever deals in its own [`NativeId`]; qualifying one is the engine's job,
/// so this type is the engine's and a plugin never constructs one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Qualified<T> {
    /// `<source>:<native>`, the form a user types back at the command line.
    pub id: GlobalId,
    /// The item as its source reported it, unchanged.
    pub item: T,
}

/// One dependency edge with both ends qualified.
///
/// An end may belong to another source. The near plugin owns that qualified far id and the
/// engine reports it without resolving or fetching the far source, so it holds no index.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct QualifiedEdge {
    /// The item the edge starts at, and the one that **depends on** the other.
    ///
    /// The direction a caller asked in says which end they named, not which end the edge
    /// starts at: a forward read and the matching reverse read report the same edge.
    pub from: QualifiedEndpoint,
    /// The item the edge points at, and the one that must finish first.
    pub to: QualifiedEndpoint,
    /// What the edge means.
    pub kind: onetaskgraph_plugin_api::DependencyKind,
}

/// One typed, qualified endpoint in an engine dependency response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct QualifiedEndpoint {
    /// `<source>:<native>`, preserved when a plugin reports another source.
    pub id: GlobalId,
    /// Whether this endpoint names a task or project.
    pub kind: onetaskgraph_plugin_api::ItemKind,
}

impl std::fmt::Display for QualifiedEndpoint {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.id.fmt(formatter)
    }
}

/// One hit of a search that may cross entities.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum SearchHit {
    /// A task matched.
    Task(Qualified<Task>),
    /// A project matched.
    Project(Qualified<Project>),
}

/// Which entities a search covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum SearchKind {
    /// Tasks only.
    Tasks,
    /// Projects only.
    Projects,
    /// Both, interleaved.
    #[default]
    Both,
}

/// What one configured source is, as `sources list` reports it.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct SourceListing {
    /// The name the configuration gave it.
    pub source: SourceName,
    /// The plugin kind behind it.
    ///
    /// A `String` because the vocabulary is open, not because it was not thought about:
    /// this is the kind a source reports, and a subprocess-hosted plugin reports one
    /// arriving over the wire from a binary this workspace never compiled. No
    /// compile-time enumeration can hold that, and a newtype over the same string would
    /// only move where an unrelated value is accepted.
    // llmlint: ignore[invalid_states_unrepresentable] the reason above, and the one
    // recorded for the same field of `SourcePlan` in plan.rs: `kind` is an open
    // vocabulary a subprocess plugin extends at run time, and `SourcePlan.kind: String`
    // is approved contract text this field is rendered beside.
    pub kind: String,
    /// Whether it built, and what it can do if it did.
    #[serde(flatten)]
    pub state: SourceState,
}

/// Whether a configured source is answering.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum SourceState {
    /// The source built, and declares this.
    Available {
        /// What it applies itself.
        capabilities: Capabilities,
    },
    /// The source could not be built at all.
    Unavailable {
        /// Why not.
        error: SourceError,
    },
}

/// Which page of a result set the caller wants.
#[derive(Debug, Clone, PartialEq)]
pub struct Paging {
    /// The most items to return.
    pub limit: NonZeroU32,
    /// Where to resume, or `None` to start at the beginning.
    pub token: Option<PageToken>,
}

/// The filters every list verb shares.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Filters {
    /// Free-text search, when the caller asked for one.
    pub text: Option<TextQuery>,
    /// Label membership, by name.
    pub labels: LabelFilter,
    /// Status categories to keep. Empty means unfiltered.
    pub statuses: Vec<StatusCategory>,
}

/// A request for a page of tasks.
#[derive(Debug, Clone)]
pub struct TaskRequest {
    /// Which sources to address. Empty means the configuration's own selection.
    pub sources: Vec<SourceName>,
    /// What to keep.
    pub filters: Filters,
    /// Which project the tasks belong to.
    pub project: ProjectSelector,
    /// Which page.
    pub paging: Paging,
}

/// A request for a page of projects.
#[derive(Debug, Clone)]
pub struct ProjectRequest {
    /// Which sources to address. Empty means the configuration's own selection.
    pub sources: Vec<SourceName>,
    /// What to keep.
    pub filters: Filters,
    /// Which page.
    pub paging: Paging,
}

/// A request for a page of labels.
#[derive(Debug, Clone)]
pub struct LabelRequest {
    /// Which sources to address. Empty means the configuration's own selection.
    pub sources: Vec<SourceName>,
    /// Which page.
    pub paging: Paging,
}

/// A request for a page of search hits.
#[derive(Debug, Clone)]
pub struct SearchRequest {
    /// Which sources to address. Empty means the configuration's own selection.
    pub sources: Vec<SourceName>,
    /// What to look for, and where.
    pub text: TextQuery,
    /// Which entities to cover.
    pub kind: SearchKind,
    /// Which page.
    pub paging: Paging,
}

/// A request for a page of one item's dependency edges.
#[derive(Debug, Clone)]
pub struct DependencyRequest {
    /// The qualified item to walk from.
    pub id: GlobalId,
    /// Which way to walk.
    pub direction: Direction,
    /// Which page.
    pub paging: Paging,
}

/// What the engine refuses before it asks any source.
///
/// Distinct from a [`SourceFailure`], which is one source failing while the others
/// answer: everything here means the request itself cannot be run at all.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EngineError {
    /// A `--source` named something no configuration configures.
    #[error(
        "no source named {name:?} is configured\n\
         next: name one of the configured sources ({configured}), or add {name:?} under \
         `sources` — `onetaskgraph sources list` shows what this configuration has."
    )]
    UnknownSource {
        /// The name that was asked for.
        name: String,
        /// The names that exist, for the message.
        configured: String,
    },

    /// A `--page` token that decodes but does not belong to this query.
    #[error(
        "{message}\n\
         next: page with a token exactly as the previous page reported it, and against \
         the same configuration — or drop `--page` to start the walk again."
    )]
    Token {
        /// What the token claims that this configuration cannot honour.
        message: String,
    },

    /// Nothing at all is configured, so there is nothing to ask.
    #[error(
        "no sources are configured\n\
         next: add one under `sources` in onetaskgraph.yaml — `onetaskgraph schema` \
         prints what each plugin accepts."
    )]
    NoSources,
}

/// One configured source, in exactly one of the two states a configured source has.
///
/// A sum rather than two lists side by side: with a `ready` vector and an `unavailable`
/// one, a source appearing in both is a shape the type permits and every reader has to
/// decide about — and they would not all decide the same way, since one of them fans a
/// query out and another names failures.
pub enum ConfiguredSource {
    /// It built, and answers queries.
    Ready(ResolvedSource),
    /// It did not, and every response says so instead.
    Unavailable(UnavailableSource),
}

impl ConfiguredSource {
    /// The name the configuration gave it, whichever state it is in.
    #[must_use]
    pub fn name(&self) -> &SourceName {
        match self {
            Self::Ready(source) => source.name(),
            Self::Unavailable(source) => source.name(),
        }
    }
}

/// The sources a configuration resolved to, and the queries they answer.
pub struct Engine {
    /// Every configured source, in configured-name order, each in one state.
    sources: Vec<ConfiguredSource>,
    /// Which sources answer when a request names none.
    selection: Vec<SourceName>,
}

impl Engine {
    /// Build every source a configuration names.
    ///
    /// A source whose plugin refuses to build — a credential that is not there, a
    /// plugin whose implementation has not landed — is **not** fatal: it becomes an
    /// entry in every response's `errors`, exactly as a source that fails mid-query
    /// does, and the other sources still answer. A user with three sources and one
    /// expired token gets the other two rather than nothing.
    #[must_use]
    pub fn build(config: &Config, secrets: &dyn SecretResolver) -> Self {
        let (ready, unavailable) = resolve_available(config, secrets);
        Self::new(
            ready
                .into_iter()
                .map(ConfiguredSource::Ready)
                .chain(unavailable.into_iter().map(ConfiguredSource::Unavailable))
                .collect(),
            config.selected_sources(),
        )
    }

    /// Drive sources built elsewhere — the engine's own tests, and any caller holding a
    /// source it did not resolve from a configuration document.
    #[must_use]
    pub fn new(sources: Vec<ConfiguredSource>, selection: Vec<SourceName>) -> Self {
        Self { sources, selection }
    }

    /// Every source that built, in configured-name order.
    fn ready(&self) -> impl Iterator<Item = &ResolvedSource> {
        self.sources.iter().filter_map(|source| match source {
            ConfiguredSource::Ready(ready) => Some(ready),
            ConfiguredSource::Unavailable(_) => None,
        })
    }

    /// Every source that did not, in the same order.
    fn unavailable(&self) -> impl Iterator<Item = &UnavailableSource> {
        self.sources.iter().filter_map(|source| match source {
            ConfiguredSource::Unavailable(unavailable) => Some(unavailable),
            ConfiguredSource::Ready(_) => None,
        })
    }

    /// Every configured source, whether or not it built, in name order.
    #[must_use]
    pub fn listing(&self) -> Vec<SourceListing> {
        let mut listings: Vec<SourceListing> = self
            .ready()
            .map(|source| SourceListing {
                source: source.name().clone(),
                kind: source.kind().to_owned(),
                state: SourceState::Available {
                    capabilities: source.source().capabilities(),
                },
            })
            .chain(self.unavailable().map(|source| SourceListing {
                source: source.name().clone(),
                kind: source.kind().to_owned(),
                state: SourceState::Unavailable {
                    error: source.error().clone(),
                },
            }))
            .collect();
        listings.sort_by(|left, right| left.source.cmp(&right.source));
        listings
    }

    /// Whether this configuration has a source called `name`, built or not.
    ///
    /// A caller reading a `--project` argument needs this: `urn:project:1` is a qualified
    /// id only if `urn` is a source here, and a native id full of colons otherwise. That
    /// rule cannot be applied without knowing what is configured.
    #[must_use]
    pub fn has(&self, name: &SourceName) -> bool {
        self.sources.iter().any(|source| source.name() == name)
    }

    /// One page of tasks.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] when the request names a source nothing configures, or
    /// carries a page token this engine did not issue. One source failing is not an
    /// error: it lands in the response's `errors`.
    pub async fn tasks(
        &self,
        request: &TaskRequest,
    ) -> Result<QueryResponse<Qualified<Task>>, EngineError> {
        let mut names = self.resolve_selection(&request.sources)?;
        // A qualified project id names one project of one source, so no other source can
        // hold a task in it. Narrowing here means the plan reports the source that was
        // actually asked rather than a row of empty entries for sources that could not
        // have answered.
        if let ProjectSelector::Qualified(id) = &request.project {
            self.known(&id.source)?;
            names.retain(|name| name == &id.source);
        }
        let query = shape("task-list", &names, &(&request.filters, &request.project));
        let states = resumption(
            self,
            request.paging.token.as_ref(),
            &[StreamKind::Items],
            &query,
        )?;
        let budget = request.paging.limit.get();

        let mut answer = Answer::new();
        let (ready, starts) = walking(answer.split(self, &names), &states, StreamKind::Items);

        let shapes: Vec<TaskShape> = ready
            .iter()
            .map(|source| {
                shape_tasks(
                    &source.source().capabilities(),
                    &request.filters,
                    &project_filter(&request.project),
                )
            })
            .collect();
        let counters: Vec<AtomicU32> = ready.iter().map(|_| AtomicU32::new(0)).collect();
        let outcomes: Vec<Outcomes> = shapes.iter().map(|shape| shape.outcomes.clone()).collect();

        let walks = ready
            .iter()
            .enumerate()
            .map(|(index, source)| {
                fetch_tasks(
                    source,
                    &shapes[index],
                    &starts[index],
                    budget,
                    &counters[index],
                )
            })
            .collect();

        let streams = answer.collect(&ready, join_all(walks).await, &counters, outcomes);
        answer.finish(
            streams,
            budget,
            owed(&states),
            &query,
            |name, task: Task| Qualified {
                id: GlobalId::new(name.clone(), task.id.clone()),
                item: task,
            },
        )
    }

    /// One page of projects.
    ///
    /// # Errors
    ///
    /// As [`tasks`](Self::tasks).
    pub async fn projects(
        &self,
        request: &ProjectRequest,
    ) -> Result<QueryResponse<Qualified<Project>>, EngineError> {
        let names = self.resolve_selection(&request.sources)?;
        let query = shape("project-list", &names, &request.filters);
        let states = resumption(
            self,
            request.paging.token.as_ref(),
            &[StreamKind::Items],
            &query,
        )?;
        let budget = request.paging.limit.get();

        let mut answer = Answer::new();
        // A source declaring `projects: unsupported` has no project table at all, so
        // there is nothing to compensate for and nothing to ask: the predicate is
        // reported unavailable and that source contributes no rows. This is the one
        // outcome the engine cannot narrow its way out of, which is what `unavailable`
        // in the plan is for.
        let mut with_projects = Vec::new();
        for source in answer.split(self, &names) {
            if source.source().capabilities().projects.is_native() {
                with_projects.push(source);
            } else {
                answer.unreachable_predicate(source, Predicate::Project);
            }
        }
        let (ready, starts) = walking(with_projects, &states, StreamKind::Items);

        let shapes: Vec<ProjectShape> = ready
            .iter()
            .map(|source| shape_projects(&source.source().capabilities(), &request.filters))
            .collect();
        let counters: Vec<AtomicU32> = ready.iter().map(|_| AtomicU32::new(0)).collect();
        let outcomes: Vec<Outcomes> = shapes.iter().map(|shape| shape.outcomes.clone()).collect();

        let walks = ready
            .iter()
            .enumerate()
            .map(|(index, source)| {
                fetch_projects(
                    source,
                    &shapes[index],
                    &starts[index],
                    budget,
                    &counters[index],
                )
            })
            .collect();

        let streams = answer.collect(&ready, join_all(walks).await, &counters, outcomes);
        answer.finish(
            streams,
            budget,
            owed(&states),
            &query,
            |name, project: Project| Qualified {
                id: GlobalId::new(name.clone(), project.id.clone()),
                item: project,
            },
        )
    }

    /// One page of labels.
    ///
    /// # Errors
    ///
    /// As [`tasks`](Self::tasks).
    pub async fn labels(
        &self,
        request: &LabelRequest,
    ) -> Result<QueryResponse<Qualified<Label>>, EngineError> {
        let names = self.resolve_selection(&request.sources)?;
        let query = shape("label-list", &names, &());
        let states = resumption(
            self,
            request.paging.token.as_ref(),
            &[StreamKind::Items],
            &query,
        )?;
        let budget = request.paging.limit.get();

        let mut answer = Answer::new();
        let (ready, starts) = walking(answer.split(self, &names), &states, StreamKind::Items);
        let counters: Vec<AtomicU32> = ready.iter().map(|_| AtomicU32::new(0)).collect();
        let outcomes: Vec<Outcomes> = ready.iter().map(|_| Outcomes::default()).collect();

        let walks = ready
            .iter()
            .enumerate()
            .map(|(index, source)| fetch_labels(source, &starts[index], budget, &counters[index]))
            .collect();

        let streams = answer.collect(&ready, join_all(walks).await, &counters, outcomes);
        answer.finish(
            streams,
            budget,
            owed(&states),
            &query,
            |name, label: Label| Qualified {
                id: GlobalId::new(name.clone(), label.id.clone()),
                item: label,
            },
        )
    }

    /// One page of search hits, over tasks, projects, or both.
    ///
    /// # Errors
    ///
    /// As [`tasks`](Self::tasks).
    pub async fn search(
        &self,
        request: &SearchRequest,
    ) -> Result<QueryResponse<SearchHit>, EngineError> {
        let names = self.resolve_selection(&request.sources)?;
        // The streams this search reads, which is what a token resuming it may name. A
        // `--kind both` walk that has exhausted one half carries only the other, so this
        // is what a token may name rather than what it must.
        let reads: &[StreamKind] = match request.kind {
            SearchKind::Tasks => &[StreamKind::Tasks],
            SearchKind::Projects => &[StreamKind::Projects],
            SearchKind::Both => &[StreamKind::Tasks, StreamKind::Projects],
        };
        // The scope is deliberately not in the fingerprint: which streams a search covers
        // is exactly what `reads` checks below, name by name and with a message that says
        // which half a token names. Folding it in here would refuse the same mistake one
        // layer earlier and less clearly, and leave that check unreachable.
        let query = shape("search", &names, &request.text);
        let states = resumption(self, request.paging.token.as_ref(), reads, &query)?;
        let budget = request.paging.limit.get();
        let filters = Filters {
            text: Some(request.text.clone()),
            ..Filters::default()
        };

        let mut answer = Answer::new();

        // One stream per (source, entity), because a search over both entities reads two
        // result sets from each source and each has its own place to resume.
        let mut ready = Vec::new();
        let mut kinds = Vec::new();
        let mut starts = Vec::new();
        for source in answer.split(self, &names) {
            let mut streams = Vec::new();
            if matches!(request.kind, SearchKind::Tasks | SearchKind::Both) {
                streams.push(StreamKind::Tasks);
            }
            if matches!(request.kind, SearchKind::Projects | SearchKind::Both) {
                if source.source().capabilities().projects.is_native() {
                    streams.push(StreamKind::Projects);
                } else {
                    answer.unreachable_predicate(source, Predicate::Project);
                }
            }
            for stream in streams {
                if let Some(resume) = resume_at(&states, source.name(), stream) {
                    ready.push(source);
                    kinds.push(stream);
                    starts.push(resume);
                }
            }
        }

        let shapes: Vec<HitShape> = ready
            .iter()
            .zip(kinds.iter())
            .map(|(source, kind)| shape_hits(&source.source().capabilities(), &filters, *kind))
            .collect();
        let counters: Vec<AtomicU32> = ready.iter().map(|_| AtomicU32::new(0)).collect();
        let outcomes: Vec<Outcomes> = shapes.iter().map(|shape| shape.outcomes.clone()).collect();

        let walks = ready
            .iter()
            .enumerate()
            .map(|(index, source)| {
                fetch_hits(
                    source,
                    &shapes[index],
                    &starts[index],
                    budget,
                    &counters[index],
                )
            })
            .collect();

        let streams =
            answer.collect_streams(&ready, &kinds, join_all(walks).await, &counters, outcomes);
        answer.finish(
            streams,
            budget,
            owed(&states),
            &query,
            |name, found: Found| match found {
                Found::Task(task) => SearchHit::Task(Qualified {
                    id: GlobalId::new(name.clone(), task.id.clone()),
                    item: task,
                }),
                Found::Project(project) => SearchHit::Project(Qualified {
                    id: GlobalId::new(name.clone(), project.id.clone()),
                    item: project,
                }),
            },
        )
    }

    /// One task by its qualified id, or an empty page when there is no such task.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::UnknownSource`] when the id names a source nothing
    /// configures.
    pub async fn task(&self, id: &GlobalId) -> Result<QueryResponse<Qualified<Task>>, EngineError> {
        let name = self.known(&id.source)?;
        let mut answer = Answer::new();
        let selected = answer.split(self, std::slice::from_ref(&name));
        let Some(source) = selected.first() else {
            return answer.nothing();
        };
        let found = source.source().get_task(&id.native).await;
        let qualified = GlobalId::new(source.name().clone(), id.native.clone());
        answer.one(source, found, |task| Qualified {
            id: qualified,
            item: task,
        })
    }

    /// One project by its qualified id, or an empty page when there is no such project.
    ///
    /// # Errors
    ///
    /// As [`task`](Self::task).
    pub async fn project(
        &self,
        id: &GlobalId,
    ) -> Result<QueryResponse<Qualified<Project>>, EngineError> {
        let name = self.known(&id.source)?;
        let mut answer = Answer::new();
        let selected = answer.split(self, std::slice::from_ref(&name));
        let Some(source) = selected.first() else {
            return answer.nothing();
        };
        let found = source.source().get_project(&id.native).await;
        let qualified = GlobalId::new(source.name().clone(), id.native.clone());
        answer.one(source, found, |project| Qualified {
            id: qualified,
            item: project,
        })
    }

    /// One page of a task's dependency edges.
    ///
    /// # Errors
    ///
    /// As [`task`](Self::task), plus [`EngineError::Token`] for a page token this engine
    /// did not issue.
    pub async fn task_dependencies(
        &self,
        request: &DependencyRequest,
    ) -> Result<QueryResponse<QualifiedEdge>, EngineError> {
        self.dependencies(request, Entity::Task).await
    }

    /// One page of a project's dependency edges.
    ///
    /// # Errors
    ///
    /// As [`task_dependencies`](Self::task_dependencies).
    pub async fn project_dependencies(
        &self,
        request: &DependencyRequest,
    ) -> Result<QueryResponse<QualifiedEdge>, EngineError> {
        self.dependencies(request, Entity::Project).await
    }

    /// Both dependency verbs, which differ only in which of a source's two edge sets
    /// they read and which of its two declarations governs the reverse direction.
    async fn dependencies(
        &self,
        request: &DependencyRequest,
        entity: Entity,
    ) -> Result<QueryResponse<QualifiedEdge>, EngineError> {
        let name = self.known(&request.id.source)?;
        let query = shape(
            "dependencies",
            std::slice::from_ref(&name),
            &(entity, &request.id.native, request.direction),
        );
        let states = resumption(
            self,
            request.paging.token.as_ref(),
            &[StreamKind::Items],
            &query,
        )?;
        let budget = request.paging.limit.get();

        let mut answer = Answer::new();
        let (ready, starts) = walking(
            answer.split(self, std::slice::from_ref(&name)),
            &states,
            StreamKind::Items,
        );
        let Some(source) = ready.first() else {
            return answer.nothing();
        };

        let capabilities = source.source().capabilities();
        let support = match entity {
            Entity::Task => capabilities.task_dependencies,
            Entity::Project => capabilities.project_dependencies,
        };
        // `DependencySupport` has no unsupported variant on purpose: a dependency read is
        // answered natively or emulated by the scan below, never abandoned and never
        // silently empty.
        let emulating = request.direction == Direction::DependedOnBy && !support.answers_reverse();
        let mut outcomes = Outcomes::default();
        if request.direction == Direction::DependedOnBy {
            if emulating {
                outcomes.record(Predicate::ReverseDependencies, Outcome::Emulated);
            } else {
                outcomes.record(Predicate::ReverseDependencies, Outcome::PushedDown);
            }
        }

        let counters = vec![AtomicU32::new(0)];
        let walked = fetch_edges(
            source,
            &request.id.native,
            request.direction,
            entity,
            emulating,
            &starts[0],
            budget,
            &counters[0],
        )
        .await;

        let streams = answer.collect(&ready, vec![walked], &counters, vec![outcomes]);
        answer.finish(
            streams,
            budget,
            owed(&states),
            &query,
            |name, edge: DependencyEdge| QualifiedEdge {
                from: qualify_endpoint(name, edge.from),
                to: qualify_endpoint(name, edge.to),
                kind: edge.kind,
            },
        )
    }

    /// The names a request addresses: the ones it gave, or the configuration's own.
    fn resolve_selection(&self, asked: &[SourceName]) -> Result<Vec<SourceName>, EngineError> {
        if asked.is_empty() {
            if self.selection.is_empty() {
                return Err(EngineError::NoSources);
            }
            return Ok(self.selection.clone());
        }
        asked.iter().map(|name| self.known(name)).collect()
    }

    /// `name` when this configuration has a source called that.
    fn known(&self, name: &SourceName) -> Result<SourceName, EngineError> {
        if self.has(name) {
            return Ok(name.clone());
        }
        if self.sources.is_empty() {
            return Err(EngineError::NoSources);
        }
        Err(EngineError::UnknownSource {
            name: name.to_string(),
            configured: self
                .listing()
                .iter()
                .map(|listing| listing.source.to_string())
                .collect::<Vec<_>>()
                .join(", "),
        })
    }
}

fn qualify_endpoint(
    source: &SourceName,
    endpoint: onetaskgraph_plugin_api::DependencyEndpoint,
) -> QualifiedEndpoint {
    let kind = endpoint.kind;
    let is_qualified = endpoint.is_qualified();
    let endpoint_id = endpoint.into_id();
    QualifiedEndpoint {
        id: if is_qualified {
            endpoint_id
                .parse()
                .expect("plugin-api validates qualified dependency endpoints")
        } else {
            GlobalId::new(source.clone(), NativeId(endpoint_id))
        },
        kind,
    }
}

/// Which of a source's two dependency graphs a request walks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Entity {
    /// Task dependencies.
    Task,
    /// Project dependencies.
    Project,
}

/// A search hit before it is qualified.
enum Found {
    /// A task matched.
    Task(Task),
    /// A project matched.
    Project(Project),
}

/// What happened to one predicate against one source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    /// Applied by the source itself.
    PushedDown,
    /// Applied by the engine over a wider result set.
    AppliedLocally,
    /// Answered by a bounded scan of the source.
    Emulated,
    /// Neither side could answer it, so this source contributed nothing for it.
    Unavailable,
}

/// What happened to each predicate against one source.
///
/// Keyed by predicate, one outcome each, because those are the only states there are: a
/// predicate the source applied was not also applied here, and one nobody could answer
/// was not also pushed down. The four lists [`SourcePlan`] carries are this map fanned
/// out at the boundary — held *as* four lists a predicate could sit in all four at once,
/// and four contradictory claims about one predicate is the one thing the part of the
/// answer whose whole job is to say which of them is true must not be able to say.
///
/// A `BTreeMap` rather than a `HashMap` so the lists come out in one order and two runs
/// of a query render the same plan.
#[derive(Debug, Clone, Default, PartialEq)]
struct Outcomes(BTreeMap<Predicate, Outcome>);

impl Outcomes {
    /// Record what happened to one predicate, replacing whatever was recorded before.
    ///
    /// Replacing rather than refusing: shaping a query decides each predicate once, and a
    /// second decision about the same one is the later one — there is no case here where
    /// both were meant to stand.
    fn record(&mut self, predicate: Predicate, outcome: Outcome) {
        self.0.insert(predicate, outcome);
    }

    /// Record the same outcome for several predicates, as a text search does for the two
    /// fields it covers.
    fn record_all(&mut self, predicates: impl IntoIterator<Item = Predicate>, outcome: Outcome) {
        for predicate in predicates {
            self.record(predicate, outcome);
        }
    }

    /// The predicates this outcome befell, in the map's own stable order.
    fn with(&self, outcome: Outcome) -> Vec<Predicate> {
        self.0
            .iter()
            .filter(|(_, recorded)| **recorded == outcome)
            .map(|(predicate, _)| *predicate)
            .collect()
    }
}

/// The query one source sees, and the predicates left to the engine.
struct TaskShape {
    /// What the source is asked.
    pushed: TaskQuery,
    /// What the engine narrows afterwards.
    local: LocalTasks,
    /// What to report.
    outcomes: Outcomes,
}

/// As [`TaskShape`], for projects.
struct ProjectShape {
    /// What the source is asked.
    pushed: ProjectQuery,
    /// What the engine narrows afterwards.
    local: LocalProjects,
    /// What to report.
    outcomes: Outcomes,
}

/// As [`TaskShape`], for one entity's half of a search.
struct HitShape {
    /// Which entity this stream reads.
    stream: StreamKind,
    /// What a task stream asks.
    tasks: TaskQuery,
    /// What a project stream asks.
    projects: ProjectQuery,
    /// What the engine narrows afterwards, for tasks.
    local_tasks: LocalTasks,
    /// What the engine narrows afterwards, for projects.
    local_projects: LocalProjects,
    /// What to report.
    outcomes: Outcomes,
}

/// The plan and the failures a response carries, accumulated as the verb runs.
///
/// One type rather than three parallel vectors threaded through every verb, because the
/// rule they enforce together is one rule: a source that fails contributes an error and
/// still leaves every other source's results standing.
struct Answer {
    /// One entry per source the engine addressed, merged by source at the end.
    plans: Vec<SourcePlan>,
    /// Every source that could not answer.
    errors: Vec<SourceFailure>,
}

impl Answer {
    fn new() -> Self {
        Self {
            plans: Vec::new(),
            errors: Vec::new(),
        }
    }

    /// The selected sources that built, recording the ones that did not as failures.
    ///
    /// A source that never built is reported and skipped rather than fatal: that is the
    /// same rule as a source failing mid-query, applied one step earlier.
    fn split<'a>(&mut self, engine: &'a Engine, names: &[SourceName]) -> Vec<&'a ResolvedSource> {
        let mut selected = Vec::new();
        for name in names {
            match engine.sources.iter().find(|source| source.name() == name) {
                Some(ConfiguredSource::Ready(source)) => selected.push(source),
                Some(ConfiguredSource::Unavailable(source)) => {
                    self.errors.push(source.failure());
                }
                None => {}
            }
        }
        selected
    }

    /// Record that a source could answer nothing for `predicate`.
    fn unreachable_predicate(&mut self, source: &ResolvedSource, predicate: Predicate) {
        let mut outcomes = Outcomes::default();
        outcomes.record(predicate, Outcome::Unavailable);
        self.plans.push(plan_for(source, outcomes, 0));
    }

    /// Turn each source's walk into a stream, keeping a failed source's failure.
    fn collect<T>(
        &mut self,
        ready: &[&ResolvedSource],
        walked: Vec<Result<Fetched<T>, SourceError>>,
        counters: &[AtomicU32],
        outcomes: Vec<Outcomes>,
    ) -> Vec<Stream<T>> {
        let kinds = vec![StreamKind::Items; ready.len()];
        self.collect_streams(ready, &kinds, walked, counters, outcomes)
    }

    /// As [`collect`](Self::collect), where a source may contribute more than one stream.
    fn collect_streams<T>(
        &mut self,
        ready: &[&ResolvedSource],
        kinds: &[StreamKind],
        walked: Vec<Result<Fetched<T>, SourceError>>,
        counters: &[AtomicU32],
        outcomes: Vec<Outcomes>,
    ) -> Vec<Stream<T>> {
        let mut streams = Vec::new();
        for (index, result) in walked.into_iter().enumerate() {
            let source = ready[index];
            let pages = counters[index].load(Ordering::Relaxed);
            self.plans
                .push(plan_for(source, outcomes[index].clone(), pages));
            match result {
                Ok(fetched) => streams.push(Stream {
                    source: source.name().clone(),
                    kind: kinds[index],
                    fetched,
                }),
                // A stream that failed leaves the token, so a walk always terminates: a
                // source failing on every page would otherwise page forever.
                Err(error) => self.errors.push(SourceFailure {
                    source: source.name().clone(),
                    error,
                }),
            }
        }
        streams
    }

    /// The response for a verb that reads exactly one item from exactly one source.
    fn one<T, U>(
        mut self,
        source: &ResolvedSource,
        found: Result<Option<T>, SourceError>,
        qualify: impl FnOnce(T) -> U,
    ) -> Result<QueryResponse<U>, EngineError> {
        self.plans.push(plan_for(source, Outcomes::default(), 1));
        let items = match found {
            Ok(Some(item)) => vec![qualify(item)],
            Ok(None) => Vec::new(),
            Err(error) => {
                self.errors.push(SourceFailure {
                    source: source.name().clone(),
                    error,
                });
                Vec::new()
            }
        };
        Ok(QueryResponse {
            items,
            next: None,
            plan: QueryPlan {
                per_source: merge_plans(self.plans),
            },
            errors: self.errors,
        })
    }

    /// The response for a verb with nothing left to ask.
    fn nothing<U>(self) -> Result<QueryResponse<U>, EngineError> {
        Ok(QueryResponse {
            items: Vec::new(),
            next: None,
            plan: QueryPlan {
                per_source: merge_plans(self.plans),
            },
            errors: self.errors,
        })
    }

    /// Merge the streams into the caller's page and mint the token that resumes it.
    ///
    /// `first` is the stream the token being resumed says is owed the next row, so the
    /// round-robin picks up where the previous page stopped rather than restarting.
    fn finish<T, U>(
        self,
        streams: Vec<Stream<T>>,
        budget: u32,
        first: Option<&Owed>,
        query: &str,
        qualify: impl Fn(&SourceName, T) -> U,
    ) -> Result<QueryResponse<U>, EngineError> {
        let (rows, states, owed) = merge(streams, budget, first);
        let next = (!states.is_empty()).then(|| PageToken::encode(query, owed, &states));
        Ok(QueryResponse {
            items: rows
                .into_iter()
                .map(|(name, item)| qualify(&name, item))
                .collect(),
            next,
            plan: QueryPlan {
                per_source: merge_plans(self.plans),
            },
            errors: self.errors,
        })
    }
}

/// One source's plan entry: the outcomes fanned out into the four lists the contract's
/// [`SourcePlan`] carries, each in one order so two runs read the same.
fn plan_for(source: &ResolvedSource, outcomes: Outcomes, pages: u32) -> SourcePlan {
    SourcePlan {
        source: source.name().clone(),
        kind: source.kind().to_owned(),
        pushed_down: outcomes.with(Outcome::PushedDown),
        applied_locally: outcomes.with(Outcome::AppliedLocally),
        emulated: outcomes.with(Outcome::Emulated),
        unavailable: outcomes.with(Outcome::Unavailable),
        pages_fetched: pages,
    }
}

/// One entry per source, however many streams that source contributed.
///
/// `search --kind both` reads two streams from each source, and a plan is per source:
/// two entries for one name would say the engine addressed it twice.
fn merge_plans(plans: Vec<SourcePlan>) -> Vec<SourcePlan> {
    let mut merged: Vec<SourcePlan> = Vec::new();
    for plan in plans {
        if let Some(existing) = merged
            .iter_mut()
            .find(|existing| existing.source == plan.source)
        {
            existing.pushed_down.extend(plan.pushed_down);
            existing.applied_locally.extend(plan.applied_locally);
            existing.emulated.extend(plan.emulated);
            existing.unavailable.extend(plan.unavailable);
            existing.pages_fetched = existing.pages_fetched.saturating_add(plan.pages_fetched);
            for list in [
                &mut existing.pushed_down,
                &mut existing.applied_locally,
                &mut existing.emulated,
                &mut existing.unavailable,
            ] {
                list.sort_unstable();
                list.dedup();
            }
        } else {
            merged.push(plan);
        }
    }
    merged
}

/// The sources still walking, with where each picks up.
///
/// A token names every stream that has more to give, so a source **absent** from one has
/// been exhausted and is not asked again. Without that, the second page of a walk would
/// restart every finished source from its first row.
fn walking<'a>(
    selected: Vec<&'a ResolvedSource>,
    states: &Option<Resumption>,
    kind: StreamKind,
) -> (Vec<&'a ResolvedSource>, Vec<Resume>) {
    let mut ready = Vec::new();
    let mut starts = Vec::new();
    for source in selected {
        if let Some(resume) = resume_at(states, source.name(), kind) {
            ready.push(source);
            starts.push(resume);
        }
    }
    (ready, starts)
}

/// A fingerprint of everything about a query that decides which rows it returns, and in
/// what order — the verb, the sources it addresses, and every filter it carries.
///
/// Written from the request's own [`Debug`] rather than field by field, and that is the
/// point: a filter added to `Filters` next year joins the fingerprint by existing. A
/// hand-written canonical form would keep compiling with the new field missing, and the
/// tokens it minted would silently stop distinguishing the queries that differ by it —
/// which is the whole failure this exists to prevent, reintroduced quietly.
///
/// Hashed rather than carried whole so a token stays a thing a person can paste. This is
/// not a signature and there is nothing secret in a token — see [`PageToken`]. It detects
/// a caller resuming the wrong walk, which is a mistake rather than an attack, so FNV-1a
/// is enough and needs no dependency the supply-chain gate would then have to weigh.
///
/// [`Debug`] output is not promised to be stable across compiler releases, and that is
/// survivable here: a token outstanding across a rebuild is refused with the message
/// above rather than honoured wrongly, which is the safe direction to fail in.
fn shape(verb: &str, sources: &[SourceName], filters: &impl std::fmt::Debug) -> String {
    let names: Vec<&str> = sources.iter().map(SourceName::as_str).collect();
    fingerprint(&format!("{verb}|{names:?}|{filters:?}"))
}

/// FNV-1a over `text`, as sixteen hex digits.
fn fingerprint(text: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// The stream a token says is owed the next row, or `None` when it says none is.
///
/// A fresh query has no token and every token whose last round ended evenly carries no
/// such stream, so `None` is the common case and means "begin at the first stream".
fn owed(document: &Option<Resumption>) -> Option<&Owed> {
    document.as_ref()?.owed.as_ref()
}

/// Where one stream picks up, or `None` when the token says it is finished.
fn resume_at(states: &Option<Resumption>, source: &SourceName, kind: StreamKind) -> Option<Resume> {
    match states {
        None => Some(Resume::default()),
        Some(document) => document
            .streams
            .iter()
            .find(|state| &state.source == source && state.stream == kind)
            .map(|state| state.resume.clone()),
    }
}

/// Read a caller's page token against the sources this configuration has.
///
/// [`PageToken::parse`] has already established that the string is this engine's own
/// resume document — that is structural and happens where the caller's string enters.
/// What it cannot establish is that the document belongs *here*, because only the engine
/// knows which sources are configured and what page ceiling each one declares. So the
/// three things a token this engine wrote is always true of are checked here:
///
/// 1. every stream it names belongs to a configured source, so a token carried over from
///    another configuration is refused rather than quietly resuming half a walk;
/// 2. every stream it names is one **this verb reads**. A token minted by
///    `search --kind both` carries task and project streams, and `task list` reads
///    neither; without this it would find nothing to resume, drop every source, and
///    answer with an empty page and a zero exit — a wrong answer that looks like an
///    exhausted walk, which is the worst shape this failure could take;
/// 3. no stream appears twice, because a walk has one place to pick up per stream;
/// 4. no `skip` reaches a source's declared page ceiling, because the engine's `skip` is
///    an index among the surviving rows of one source page and can never reach it.
///
/// 5. the stream owed the next row, when the document names one, is a stream the
///    document also resumes.
///
/// A sixth thing needs no check here: at most one stream is owed the next row, because
/// [`Resumption`] holds that as one optional stream rather than as a flag on each of
/// them, so a document naming two has no spelling.
///
/// None of this is a security boundary and a page token is not a credential: nothing in
/// one is secret, and a forged cursor is handed straight back to the source that would
/// have issued it and refused there. What it buys is that a stale or hand-edited token
/// fails saying so, instead of silently returning a page from somewhere else in the walk.
fn resumption(
    engine: &Engine,
    token: Option<&PageToken>,
    reads: &[StreamKind],
    query: &str,
) -> Result<Option<Resumption>, EngineError> {
    let Some(document) = token.map(PageToken::decode) else {
        return Ok(None);
    };

    // Every cursor below is an offset into the result set *one* query produced. Handed to
    // a different one — the same verb with another `--label`, another `--search`, another
    // `--direction` — each source picks up at a position that meant something in a walk
    // the caller is no longer doing, and the rows that come back are real rows at exit
    // zero. Nothing about that answer says it is arbitrary, which is what makes it worth
    // refusing rather than serving.
    if document.query != query {
        return Err(EngineError::Token {
            message: "this page token was written by a different query — resume the walk it \
                      came from, or drop --page to start this one from the beginning"
                .to_owned(),
        });
    }
    let states = &document.streams;

    let mut seen: Vec<(&SourceName, StreamKind)> = Vec::new();
    for state in states {
        if !reads.contains(&state.stream) {
            return Err(EngineError::Token {
                message: format!(
                    "this page token resumes {}, which this command does not read — it \
                     was written by a different query",
                    state.stream.describe()
                ),
            });
        }
        let ceiling = engine
            .ready()
            .find(|source| source.name() == &state.source)
            .map(ceiling);
        if ceiling.is_none() && !engine.has(&state.source) {
            return Err(EngineError::Token {
                message: format!(
                    "this page token resumes a source called {:?}, which this \
                     configuration does not have",
                    state.source.as_str()
                ),
            });
        }
        if let Some(ceiling) = ceiling
            && state.resume.skip >= ceiling
        {
            return Err(EngineError::Token {
                message: format!(
                    "this page token resumes {} rows into a page of source {:?}, which \
                     serves at most {ceiling}",
                    state.resume.skip,
                    state.source.as_str()
                ),
            });
        }
        if seen.contains(&(&state.source, state.stream)) {
            return Err(EngineError::Token {
                message: format!(
                    "this page token gives source {:?} two places to resume from",
                    state.source.as_str()
                ),
            });
        }
        seen.push((&state.source, state.stream));
    }

    // The stream owed the next row has to be one of the streams this document resumes.
    // Ignoring a stray one would be harmless in its effect — the merge would start at the
    // first stream instead — but it would be a value from outside accepted without a
    // reading, and the next thing to depend on it would inherit that.
    if let Some(owed) = &document.owed
        && !document
            .streams
            .iter()
            .any(|state| state.source == owed.source && state.stream == owed.stream)
    {
        return Err(EngineError::Token {
            message: format!(
                "this page token owes the next row to a stream it does not resume, \
                 {:?}'s {}",
                owed.source.as_str(),
                owed.stream.describe()
            ),
        });
    }

    Ok(Some(document))
}

/// Which project a task must belong to, as a source sees it.
///
/// A qualified id becomes a plain native one because by the time this runs the selection
/// holds only that id's own source — so there is no "some other source" case to get
/// wrong, and none to leave untested.
fn project_filter(selector: &ProjectSelector) -> ProjectFilter {
    match selector {
        ProjectSelector::Any => ProjectFilter::Any,
        ProjectSelector::Orphans => ProjectFilter::Orphans,
        ProjectSelector::Native(id) => ProjectFilter::Is(id.clone()),
        ProjectSelector::Qualified(id) => ProjectFilter::Is(id.native.clone()),
    }
}

/// The predicates one text query is made of.
fn text_predicates(fields: TextFields) -> Vec<Predicate> {
    match fields {
        TextFields::Title => vec![Predicate::SearchTitle],
        TextFields::Content => vec![Predicate::SearchContent],
        TextFields::TitleOrContent => vec![Predicate::SearchTitle, Predicate::SearchContent],
    }
}

/// Whether a source searches **every** field this query names.
///
/// Every, not any: a `title-or-content` search pushed to a source that searches only
/// titles would come back missing every row that matches in the body alone — a narrower
/// result than the truth, which is the one thing compensation cannot repair. So a
/// half-capable source is not asked at all and the engine searches both fields itself.
fn searches_natively(capabilities: &Capabilities, fields: TextFields) -> bool {
    match fields {
        TextFields::Title => capabilities.search_title.is_native(),
        TextFields::Content => capabilities.search_content.is_native(),
        TextFields::TitleOrContent => {
            capabilities.search_title.is_native() && capabilities.search_content.is_native()
        }
    }
}

/// Split a task query between the source and the engine.
fn shape_tasks(
    capabilities: &Capabilities,
    filters: &Filters,
    project: &ProjectFilter,
) -> TaskShape {
    let mut pushed = TaskQuery::default();
    let mut local = LocalTasks::default();
    let mut outcomes = Outcomes::default();

    if !filters.labels.is_empty() {
        if capabilities.filter_by_label.is_native() {
            pushed.labels = filters.labels.clone();
            outcomes.record(Predicate::Label, Outcome::PushedDown);
        } else {
            local.labels = Some(filters.labels.clone());
            outcomes.record(Predicate::Label, Outcome::AppliedLocally);
        }
    }
    if !filters.statuses.is_empty() {
        if capabilities.filter_by_status.is_native() {
            pushed.statuses.clone_from(&filters.statuses);
            outcomes.record(Predicate::Status, Outcome::PushedDown);
        } else {
            local.statuses.clone_from(&filters.statuses);
            outcomes.record(Predicate::Status, Outcome::AppliedLocally);
        }
    }
    if let Some(text) = &filters.text {
        let predicates = text_predicates(text.fields);
        if searches_natively(capabilities, text.fields) {
            pushed.text = Some(text.clone());
            outcomes.record_all(predicates, Outcome::PushedDown);
        } else {
            local.text = Some(text.clone());
            outcomes.record_all(predicates, Outcome::AppliedLocally);
        }
    }
    match project {
        ProjectFilter::Any => {}
        ProjectFilter::Orphans => {
            if capabilities.orphan_tasks.is_native() {
                pushed.project = ProjectFilter::Orphans;
                outcomes.record(Predicate::Project, Outcome::PushedDown);
            } else {
                local.project = Some(ProjectFilter::Orphans);
                outcomes.record(Predicate::Project, Outcome::AppliedLocally);
            }
        }
        ProjectFilter::Is(id) => {
            if capabilities.projects.is_native() {
                pushed.project = ProjectFilter::Is(id.clone());
                outcomes.record(Predicate::Project, Outcome::PushedDown);
            } else {
                local.project = Some(ProjectFilter::Is(id.clone()));
                outcomes.record(Predicate::Project, Outcome::AppliedLocally);
            }
        }
    }

    TaskShape {
        pushed,
        local,
        outcomes,
    }
}

/// Split a project query between the source and the engine.
fn shape_projects(capabilities: &Capabilities, filters: &Filters) -> ProjectShape {
    let mut pushed = ProjectQuery::default();
    let mut local = LocalProjects::default();
    let mut outcomes = Outcomes::default();

    if !filters.labels.is_empty() {
        if capabilities.filter_by_label.is_native() {
            pushed.labels = filters.labels.clone();
            outcomes.record(Predicate::Label, Outcome::PushedDown);
        } else {
            local.labels = Some(filters.labels.clone());
            outcomes.record(Predicate::Label, Outcome::AppliedLocally);
        }
    }
    if !filters.statuses.is_empty() {
        if capabilities.filter_by_status.is_native() {
            pushed.statuses.clone_from(&filters.statuses);
            outcomes.record(Predicate::Status, Outcome::PushedDown);
        } else {
            local.statuses.clone_from(&filters.statuses);
            outcomes.record(Predicate::Status, Outcome::AppliedLocally);
        }
    }
    if let Some(text) = &filters.text {
        let predicates = text_predicates(text.fields);
        if searches_natively(capabilities, text.fields) {
            pushed.text = Some(text.clone());
            outcomes.record_all(predicates, Outcome::PushedDown);
        } else {
            local.text = Some(text.clone());
            outcomes.record_all(predicates, Outcome::AppliedLocally);
        }
    }

    ProjectShape {
        pushed,
        local,
        outcomes,
    }
}

/// Split one entity's half of a search between the source and the engine.
fn shape_hits(capabilities: &Capabilities, filters: &Filters, stream: StreamKind) -> HitShape {
    match stream {
        StreamKind::Projects => {
            let shaped = shape_projects(capabilities, filters);
            HitShape {
                stream,
                tasks: TaskQuery::default(),
                projects: shaped.pushed,
                local_tasks: LocalTasks::default(),
                local_projects: shaped.local,
                outcomes: shaped.outcomes,
            }
        }
        StreamKind::Items | StreamKind::Tasks => {
            let shaped = shape_tasks(capabilities, filters, &ProjectFilter::Any);
            HitShape {
                stream,
                tasks: shaped.pushed,
                projects: ProjectQuery::default(),
                local_tasks: shaped.local,
                local_projects: LocalProjects::default(),
                outcomes: shaped.outcomes,
            }
        }
    }
}

/// How large a page to ask a source for.
///
/// Exactly what is needed when every predicate went down, and the source's own ceiling
/// when the engine is narrowing — because a compensating walk cannot know how many rows
/// of a page will survive, and asking for the caller's limit would turn one filtered
/// page into a page per surviving row.
fn page_size(compensating: bool, budget: u32, ceiling: u32) -> u32 {
    if compensating {
        ceiling
    } else {
        budget.min(ceiling)
    }
}

/// The largest page this source will serve, never zero.
fn ceiling(source: &ResolvedSource) -> u32 {
    source.source().capabilities().max_page_size.max(1)
}

/// Walk one source's tasks, narrowing whatever it did not apply itself.
async fn fetch_tasks(
    source: &ResolvedSource,
    shape: &TaskShape,
    start: &Resume,
    budget: u32,
    calls: &AtomicU32,
) -> Result<Fetched<Task>, SourceError> {
    let compensating = shape.local != LocalTasks::default();
    walk(
        start,
        budget,
        page_size(compensating, budget, ceiling(source)),
        |task| shape.local.keeps(task),
        |cursor, limit| async move {
            calls.fetch_add(1, Ordering::Relaxed);
            let request = PageRequest { cursor, limit };
            source.source().query_tasks(&shape.pushed, &request).await
        },
    )
    .await
}

/// Walk one source's projects, narrowing whatever it did not apply itself.
async fn fetch_projects(
    source: &ResolvedSource,
    shape: &ProjectShape,
    start: &Resume,
    budget: u32,
    calls: &AtomicU32,
) -> Result<Fetched<Project>, SourceError> {
    let compensating = shape.local != LocalProjects::default();
    walk(
        start,
        budget,
        page_size(compensating, budget, ceiling(source)),
        |project| shape.local.keeps(project),
        |cursor, limit| async move {
            calls.fetch_add(1, Ordering::Relaxed);
            let request = PageRequest { cursor, limit };
            source
                .source()
                .query_projects(&shape.pushed, &request)
                .await
        },
    )
    .await
}

/// Walk one source's labels. There is no predicate to compensate for.
async fn fetch_labels(
    source: &ResolvedSource,
    start: &Resume,
    budget: u32,
    calls: &AtomicU32,
) -> Result<Fetched<Label>, SourceError> {
    walk(
        start,
        budget,
        page_size(false, budget, ceiling(source)),
        |_| true,
        |cursor, limit| async move {
            calls.fetch_add(1, Ordering::Relaxed);
            let request = PageRequest { cursor, limit };
            source.source().labels(&request).await
        },
    )
    .await
}

/// Walk one entity's half of a search.
async fn fetch_hits(
    source: &ResolvedSource,
    shape: &HitShape,
    start: &Resume,
    budget: u32,
    calls: &AtomicU32,
) -> Result<Fetched<Found>, SourceError> {
    let ceiling = ceiling(source);
    match shape.stream {
        StreamKind::Projects => {
            let compensating = shape.local_projects != LocalProjects::default();
            walk(
                start,
                budget,
                page_size(compensating, budget, ceiling),
                |found| match found {
                    Found::Project(project) => shape.local_projects.keeps(project),
                    Found::Task(_) => true,
                },
                |cursor, limit| async move {
                    calls.fetch_add(1, Ordering::Relaxed);
                    let request = PageRequest { cursor, limit };
                    let page = source
                        .source()
                        .query_projects(&shape.projects, &request)
                        .await?;
                    Ok(Page {
                        items: page.items.into_iter().map(Found::Project).collect(),
                        next: page.next,
                    })
                },
            )
            .await
        }
        StreamKind::Items | StreamKind::Tasks => {
            let compensating = shape.local_tasks != LocalTasks::default();
            walk(
                start,
                budget,
                page_size(compensating, budget, ceiling),
                |found| match found {
                    Found::Task(task) => shape.local_tasks.keeps(task),
                    Found::Project(_) => true,
                },
                |cursor, limit| async move {
                    calls.fetch_add(1, Ordering::Relaxed);
                    let request = PageRequest { cursor, limit };
                    let page = source.source().query_tasks(&shape.tasks, &request).await?;
                    Ok(Page {
                        items: page.items.into_iter().map(Found::Task).collect(),
                        next: page.next,
                    })
                },
            )
            .await
        }
    }
}

/// One page of an item's forward edges.
async fn forward_edges(
    source: &ResolvedSource,
    entity: Entity,
    id: &NativeId,
    request: &PageRequest,
) -> Result<Page<DependencyEdge>, SourceError> {
    match entity {
        Entity::Task => {
            source
                .source()
                .task_dependencies(id, Direction::DependsOn, request)
                .await
        }
        Entity::Project => {
            source
                .source()
                .project_dependencies(id, Direction::DependsOn, request)
                .await
        }
    }
}

/// Walk one item's dependency edges, emulating the reverse direction when the source
/// only reports forward ones.
///
/// The emulation is the bounded page-by-page scan the contract describes: a page of the
/// source's items, each asked for its own forward edges, keeping the ones that point at
/// `native`. It is indexless by construction — nothing is retained between pages beyond
/// the caller's own page — which is why a source that cannot walk backwards costs a scan
/// rather than a stored reverse index.
#[expect(
    clippy::too_many_arguments,
    reason = "every argument is one axis of one walk — the source, the item, the \
              direction, which of its two graphs, whether the reverse is emulated, where \
              to resume, how many rows to return and where to count calls. Grouping them \
              into a struct would name the same eight values one indirection further from \
              the loop that reads them."
)]
async fn fetch_edges(
    source: &ResolvedSource,
    native: &NativeId,
    direction: Direction,
    entity: Entity,
    emulating: bool,
    start: &Resume,
    budget: u32,
    calls: &AtomicU32,
) -> Result<Fetched<DependencyEdge>, SourceError> {
    let ceiling = ceiling(source);
    if !emulating {
        return walk(
            start,
            budget,
            page_size(false, budget, ceiling),
            |_| true,
            |cursor, limit| async move {
                calls.fetch_add(1, Ordering::Relaxed);
                let request = PageRequest { cursor, limit };
                match entity {
                    Entity::Task => {
                        source
                            .source()
                            .task_dependencies(native, direction, &request)
                            .await
                    }
                    Entity::Project => {
                        source
                            .source()
                            .project_dependencies(native, direction, &request)
                            .await
                    }
                }
            },
        )
        .await;
    }

    walk(
        start,
        budget,
        ceiling,
        |_| true,
        |cursor, limit| async move {
            calls.fetch_add(1, Ordering::Relaxed);
            let request = PageRequest { cursor, limit };
            let (ids, next) = match entity {
                Entity::Task => {
                    let page = source
                        .source()
                        .query_tasks(&TaskQuery::default(), &request)
                        .await?;
                    let ids: Vec<NativeId> = page.items.into_iter().map(|task| task.id).collect();
                    (ids, page.next)
                }
                Entity::Project => {
                    let page = source
                        .source()
                        .query_projects(&ProjectQuery::default(), &request)
                        .await?;
                    let ids: Vec<NativeId> =
                        page.items.into_iter().map(|project| project.id).collect();
                    (ids, page.next)
                }
            };

            let mut edges = Vec::new();
            for id in ids {
                let mut inner: Option<Cursor> = None;
                loop {
                    calls.fetch_add(1, Ordering::Relaxed);
                    let request = PageRequest {
                        cursor: inner.clone(),
                        limit,
                    };
                    let page = forward_edges(source, entity, &id, &request).await?;
                    // The inner half of the same bound the walk holds on its own pages:
                    // this scan keeps every matching edge of one source page, so a source
                    // that overruns here overruns the engine's memory just as surely.
                    fits(page.items.len(), limit)?;
                    edges.extend(page.items.into_iter().filter(|edge| &edge.to == native));
                    if page.next.is_some() && page.next == inner {
                        return Err(SourceError::Malformed {
                            message: "the source returned the cursor it was given while its \
                                      forward edges were being scanned, so the scan would \
                                      never end"
                                .to_owned(),
                        });
                    }
                    match page.next {
                        Some(cursor) => inner = Some(cursor),
                        None => break,
                    }
                }
            }

            Ok(Page { items: edges, next })
        },
    )
    .await
}
