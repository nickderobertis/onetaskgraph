//! The copy verb: one item out of one source and into another, by the rules that make a
//! second copy an update rather than a duplicate.
//!
//! Correspondence lives on the item and never in a table. A copied item carries
//! [`GlobalId::ORIGIN_KEY`], whose value is the qualified id it was copied from, and the
//! two match rules below read exactly that — so nothing here is written down outside the
//! plugin that owns the item, and the invariant this engine is built around is untouched.
//!
//! 1. **Follow the origin.** An item already carrying an origin whose source half is the
//!    destination names the destination item *directly*, and the copy updates it. This is
//!    the half that makes an edit's copy-back an update: the local file came from the
//!    remote item and knows which one.
//! 2. **Search by origin.** Otherwise the destination is scanned, one page at a time, for
//!    an item whose origin is the id being copied. Found, the copy updates it; not found,
//!    the copy creates one carrying that origin.
//!
//! A destination write is at the user's explicit request, names its destination, goes
//! through that source's own write interface into that source's own store, and is never
//! read back to answer a query. That is what makes it a write and not a cache.

use std::collections::BTreeMap;

use onetaskgraph_plugin_api::{
    DependencyEdge, DependencyEndpoint, DependencyKind, Direction, ItemKind, ItemWrite, NativeId,
    Page, PageRequest, Project, ProjectQuery, Repository, SourceError, SourceName, Task, TaskQuery,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::GlobalId;
use crate::resolve::ResolvedSource;

use super::local::ProjectSelector;
use super::{Engine, EngineError, Filters, Paging, TaskRequest};

/// A request to copy work into one configured destination.
#[derive(Debug, Clone)]
pub struct CopyRequest {
    /// The qualified items to copy, in the order they were named.
    pub items: Vec<GlobalId>,
    /// Whether those ids name tasks or projects.
    pub kind: ItemKind,
    /// The configured source to copy into — a source name, never a qualified id.
    pub destination: SourceName,
    /// For a project copy, whether the tasks in it are copied too.
    pub include_tasks: bool,
    /// How to re-establish a correspondence the two origin rules cannot find.
    pub match_by: Option<MatchBy>,
    /// Whether an origin naming nothing at the destination falls through to the search
    /// rule instead of refusing.
    pub recreate: bool,
    /// Whether to perform every read and no write.
    pub dry_run: bool,
}

/// The caller-named escape for a correspondence neither origin rule can find.
///
/// A person editing Markdown who deletes or corrupts the origin key leaves an item rule 1
/// cannot use and rule 2 cannot find, and the next copy would create a second item. This
/// is how that is re-established without hand-editing ids.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatchBy {
    /// Match the item whose title is the same.
    Title,
    /// Match the item whose value at this metadata key is the same.
    Metadata(String),
}

impl MatchBy {
    /// The spelling a caller types, `title` or any metadata key.
    #[must_use]
    pub fn parse(key: &str) -> Self {
        if key == "title" {
            Self::Title
        } else {
            Self::Metadata(key.to_owned())
        }
    }
}

/// What a copy did, one entry per item.
///
/// The same per-item outcomes reach every consumer: the machine-readable output renders
/// this, the rendered output renders this, and a Rust caller is handed it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CopyReport {
    /// One entry per item the copy considered, in the order it considered them.
    pub items: Vec<CopyOutcome>,
}

/// What happened to one item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CopyOutcome {
    /// The qualified id the item was read from.
    pub source: GlobalId,
    /// The qualified id it was written to, or `null` for a dry run that would create.
    pub destination: Option<GlobalId>,
    /// Which of the four things happened.
    pub action: CopyAction,
}

/// The four things a copy can do to one item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum CopyAction {
    /// The destination held no counterpart, so one was created.
    Created,
    /// The destination held a counterpart and it now reads as the source does.
    Updated,
    /// The destination held a counterpart that already read that way; nothing was written.
    Unchanged,
    /// The destination holds a counterpart the source no longer does. A copy never
    /// deletes, so it was left exactly as it is.
    Orphaned,
}

/// Where one item is going at the destination.
enum Target {
    /// Update the destination item with this id.
    Update(NativeId),
    /// Create one.
    Create,
}

/// What a scan of the destination is looking for.
enum Wanted {
    /// An item recording this qualified id as its origin.
    Origin(String),
    /// An item whose title is this.
    Title(String),
    /// An item holding this value at this metadata key.
    Metadata(String, Value),
}

impl Wanted {
    /// Whether one destination item is the one being looked for.
    fn found(&self, title: &str, metadata: &BTreeMap<String, Value>) -> bool {
        match self {
            Self::Origin(id) => {
                metadata.get(GlobalId::ORIGIN_KEY) == Some(&Value::String(id.clone()))
            }
            Self::Title(wanted) => title == wanted,
            Self::Metadata(key, value) => metadata.get(key) == Some(value),
        }
    }
}

/// One item, read and resolved, on its way into the destination.
struct Planned {
    /// Where it came from.
    source: GlobalId,
    /// The item as its source reported it.
    item: Item,
    /// Its forward edges, as its source reported them.
    edges: Vec<DependencyEdge>,
    /// Where it is going.
    target: Target,
}

/// A task or a project, so the copy path is written once.
enum Item {
    /// A task.
    Task(Box<Task>),
    /// A project.
    Project(Box<Project>),
}

impl Item {
    fn id(&self) -> &NativeId {
        match self {
            Self::Task(task) => &task.id,
            Self::Project(project) => &project.id,
        }
    }

    fn kind(&self) -> ItemKind {
        match self {
            Self::Task(_) => ItemKind::Task,
            Self::Project(_) => ItemKind::Project,
        }
    }
}

impl Engine {
    /// Copy every item a request names into one configured destination.
    ///
    /// This is the whole of the verb, and the command line drives exactly this: a copy a
    /// Rust caller makes and a copy typed at a shell are the same call, so the two cannot
    /// answer the same copy differently.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] when the destination is not configured, cannot be built or
    /// cannot be written; when an id names nothing; when an origin names an item the
    /// destination no longer holds and `--recreate` was not given; and when the
    /// destination refuses the write — including a field or a metadata key it cannot
    /// carry, which it names rather than dropping.
    pub async fn copy(&self, request: &CopyRequest) -> Result<CopyReport, EngineError> {
        let destination = self.writable(&request.destination)?;
        let mut items = Vec::new();
        for id in &request.items {
            match request.kind {
                ItemKind::Task => {
                    items.extend(
                        self.copy_items(
                            destination,
                            request,
                            ItemKind::Task,
                            std::slice::from_ref(id),
                            None,
                        )
                        .await?,
                    );
                }
                ItemKind::Project => {
                    items.extend(self.copy_project(destination, request, id).await?);
                }
            }
        }
        Ok(CopyReport { items })
    }

    /// The destination source, once it is established it exists and can be written.
    fn writable(&self, name: &SourceName) -> Result<&ResolvedSource, EngineError> {
        let name = self.known(name)?;
        if let Some(unavailable) = self.unavailable().find(|source| source.name() == &name) {
            return Err(EngineError::DestinationUnavailable {
                name: name.to_string(),
                error: unavailable.error().clone(),
            });
        }
        let source = self
            .ready()
            .find(|source| source.name() == &name)
            .ok_or(EngineError::NoSources)?;
        if !source.source().writes().is_supported() {
            return Err(EngineError::NotWritable {
                name: name.to_string(),
                kind: source.kind().to_owned(),
            });
        }
        Ok(source)
    }

    /// Copy one project and, unless they are excluded, every task in it.
    async fn copy_project(
        &self,
        destination: &ResolvedSource,
        request: &CopyRequest,
        id: &GlobalId,
    ) -> Result<Vec<CopyOutcome>, EngineError> {
        let mut outcomes = self
            .copy_items(
                destination,
                request,
                ItemKind::Project,
                std::slice::from_ref(id),
                None,
            )
            .await?;
        if !request.include_tasks {
            return Ok(outcomes);
        }
        // `None` when a dry run would have created the project: nothing was written, so
        // there is no destination project id to file the tasks under. Every task is still
        // read and still reported, because that is what a dry run is for.
        let project = outcomes
            .first()
            .and_then(|outcome| outcome.destination.clone());
        let members = self.project_members(id).await?;
        outcomes.extend(
            self.copy_items(
                destination,
                request,
                ItemKind::Task,
                &members,
                project.as_ref().map(|project| project.native.clone()),
            )
            .await?,
        );
        if let Some(project) = project {
            outcomes.extend(
                self.orphans(destination, id, &project.native, &members)
                    .await?,
            );
        }
        Ok(outcomes)
    }

    /// Every task the source holds in `project`, by qualified id.
    async fn project_members(&self, project: &GlobalId) -> Result<Vec<GlobalId>, EngineError> {
        let mut request = TaskRequest {
            sources: vec![project.source.clone()],
            filters: Filters::default(),
            project: ProjectSelector::Qualified(project.clone()),
            paging: Paging {
                limit: PROJECT_PAGE,
                token: None,
            },
        };
        let mut members = Vec::new();
        loop {
            let response = self.tasks(&request).await?;
            if let Some(failure) = response.errors.first() {
                return Err(EngineError::SourceRefused {
                    name: failure.source.to_string(),
                    error: failure.error.clone(),
                });
            }
            members.extend(response.items.into_iter().map(|task| task.id));
            match response.next {
                Some(token) => request.paging.token = Some(token),
                None => return Ok(members),
            }
        }
    }

    /// Destination tasks filed under the copied project whose origin the source no longer
    /// holds.
    ///
    /// A copy never deletes, so each is left exactly as it is and reported.
    async fn orphans(
        &self,
        destination: &ResolvedSource,
        project: &GlobalId,
        at_destination: &NativeId,
        copied: &[GlobalId],
    ) -> Result<Vec<CopyOutcome>, EngineError> {
        let mut orphans = Vec::new();
        let mut cursor = None;
        loop {
            let page: Page<Task> = destination
                .source()
                .query_tasks(&TaskQuery::default(), &request_for(destination, cursor))
                .await
                .map_err(|error| refused(destination, error))?;
            for task in &page.items {
                if task.project.as_ref() != Some(at_destination) {
                    continue;
                }
                let Some(origin) = origin_of(&task.metadata) else {
                    continue;
                };
                if origin.source != project.source || copied.contains(&origin) {
                    continue;
                }
                orphans.push(CopyOutcome {
                    source: origin,
                    destination: Some(GlobalId::new(destination.name().clone(), task.id.clone())),
                    action: CopyAction::Orphaned,
                });
            }
            match page.next {
                Some(next) => cursor = Some(next),
                None => return Ok(orphans),
            }
        }
    }

    /// Read, resolve and write every item named, then repair the edges among them.
    ///
    /// Two passes, because an edge between two items of one copy can point at a member
    /// whose destination id is not known until it has been created. The second pass runs
    /// only for the items that had one.
    async fn copy_items(
        &self,
        destination: &ResolvedSource,
        request: &CopyRequest,
        kind: ItemKind,
        items: &[GlobalId],
        project: Option<NativeId>,
    ) -> Result<Vec<CopyOutcome>, EngineError> {
        let mut planned = Vec::new();
        for id in items {
            planned.push(self.plan(destination, request, kind, id).await?);
        }

        let copied: Vec<GlobalId> = planned.iter().map(|item| item.source.clone()).collect();
        // Keyed by the qualified id's own rendering, which is what a recorded origin holds
        // anyway — making `GlobalId` orderable for one local map would put an ordering on
        // a contract type for a reason no caller of it has.
        let mut written: BTreeMap<String, NativeId> = BTreeMap::new();
        for item in &planned {
            if let Target::Update(id) = &item.target {
                written.insert(item.source.to_string(), id.clone());
            }
        }

        let mut outcomes = Vec::new();
        let mut deferred = Vec::new();
        for (index, item) in planned.iter().enumerate() {
            let edges = mapped_edges(
                &item.edges,
                &item.source.source,
                destination,
                &copied,
                &written,
            );
            if edges.iter().any(Option::is_none) {
                deferred.push(index);
            }
            let outcome = self
                .land(destination, request, item, project.clone(), &edges)
                .await?;
            if let Some(id) = &outcome.destination {
                written.insert(item.source.to_string(), id.native.clone());
            }
            outcomes.push(outcome);
        }

        if request.dry_run {
            return Ok(outcomes);
        }
        for index in deferred {
            let item = &planned[index];
            let Some(id) = outcomes[index].destination.clone() else {
                continue;
            };
            let edges = mapped_edges(
                &item.edges,
                &item.source.source,
                destination,
                &copied,
                &written,
            );
            self.write(
                destination,
                item,
                Some(id.native),
                project.clone(),
                &resolved(&edges),
            )
            .await?;
        }
        Ok(outcomes)
    }

    /// Read one item and its forward edges, and decide where it is going.
    async fn plan(
        &self,
        destination: &ResolvedSource,
        request: &CopyRequest,
        kind: ItemKind,
        id: &GlobalId,
    ) -> Result<Planned, EngineError> {
        let source = self.readable(&id.source)?;
        let item = match kind {
            ItemKind::Task => source
                .source()
                .get_task(&id.native)
                .await
                .map_err(|error| refused(source, error))?
                .map(|task| Item::Task(Box::new(task))),
            ItemKind::Project => source
                .source()
                .get_project(&id.native)
                .await
                .map_err(|error| refused(source, error))?
                .map(|project| Item::Project(Box::new(project))),
        }
        .ok_or_else(|| EngineError::NoSuchItem { id: id.to_string() })?;
        let edges = forward_edges(source, &id.native, item.kind()).await?;
        let target = self.target(destination, request, id, &item).await?;
        Ok(Planned {
            source: id.clone(),
            item,
            edges,
            target,
        })
    }

    /// Which destination item this one corresponds to, by the two origin rules and the
    /// caller's escape.
    async fn target(
        &self,
        destination: &ResolvedSource,
        request: &CopyRequest,
        id: &GlobalId,
        item: &Item,
    ) -> Result<Target, EngineError> {
        let (title, metadata) = described(item);
        if let Some(origin) = origin_of(metadata)
            && &origin.source == destination.name()
        {
            if exists(destination, &origin.native, item.kind()).await? {
                return Ok(Target::Update(origin.native));
            }
            if !request.recreate {
                return Err(EngineError::StaleOrigin {
                    item: id.to_string(),
                    origin: origin.to_string(),
                });
            }
        }
        if let Some(found) = self
            .scan(destination, item.kind(), &Wanted::Origin(id.to_string()))
            .await?
        {
            return Ok(Target::Update(found));
        }
        let wanted = match &request.match_by {
            Some(MatchBy::Title) => Some(Wanted::Title(title.to_owned())),
            Some(MatchBy::Metadata(key)) => metadata
                .get(key)
                .map(|value| Wanted::Metadata(key.clone(), value.clone())),
            None => None,
        };
        if let Some(wanted) = wanted
            && let Some(found) = self.scan(destination, item.kind(), &wanted).await?
        {
            return Ok(Target::Update(found));
        }
        Ok(Target::Create)
    }

    /// Walk the destination one page at a time, looking for `wanted`.
    ///
    /// One page is held at a time and nothing is written down, which is the same bound
    /// every other compensation in this engine works under.
    async fn scan(
        &self,
        destination: &ResolvedSource,
        kind: ItemKind,
        wanted: &Wanted,
    ) -> Result<Option<NativeId>, EngineError> {
        let mut cursor = None;
        loop {
            let next = match kind {
                ItemKind::Task => {
                    let page = destination
                        .source()
                        .query_tasks(&TaskQuery::default(), &request_for(destination, cursor))
                        .await
                        .map_err(|error| refused(destination, error))?;
                    for task in &page.items {
                        if wanted.found(&task.title, &task.metadata) {
                            return Ok(Some(task.id.clone()));
                        }
                    }
                    page.next
                }
                ItemKind::Project => {
                    let page = destination
                        .source()
                        .query_projects(&ProjectQuery::default(), &request_for(destination, cursor))
                        .await
                        .map_err(|error| refused(destination, error))?;
                    for project in &page.items {
                        if wanted.found(&project.title, &project.metadata) {
                            return Ok(Some(project.id.clone()));
                        }
                    }
                    page.next
                }
            };
            match next {
                Some(next) => cursor = Some(next),
                None => return Ok(None),
            }
        }
    }

    /// Write one planned item, or say what a dry run would have done.
    async fn land(
        &self,
        destination: &ResolvedSource,
        request: &CopyRequest,
        item: &Planned,
        project: Option<NativeId>,
        edges: &[Option<DependencyEdge>],
    ) -> Result<CopyOutcome, EngineError> {
        let target = match &item.target {
            Target::Update(id) => Some(id.clone()),
            Target::Create => None,
        };
        let project = match (&item.item, project) {
            (Item::Task(task), None) => self.counterpart(destination, item, task).await?,
            (Item::Task(_), filed) => filed,
            (Item::Project(_), _) => None,
        };
        let edges = resolved(edges);
        if let Some(id) = &target
            && !self
                .changes(destination, item, id, &project, &edges)
                .await?
        {
            return Ok(CopyOutcome {
                source: item.source.clone(),
                destination: Some(GlobalId::new(destination.name().clone(), id.clone())),
                action: CopyAction::Unchanged,
            });
        }
        let action = if target.is_some() {
            CopyAction::Updated
        } else {
            CopyAction::Created
        };
        if request.dry_run {
            return Ok(CopyOutcome {
                source: item.source.clone(),
                destination: target.map(|id| GlobalId::new(destination.name().clone(), id)),
                action,
            });
        }
        let written = self
            .write(destination, item, target, project, &edges)
            .await?;
        Ok(CopyOutcome {
            source: item.source.clone(),
            destination: Some(GlobalId::new(destination.name().clone(), written)),
            action,
        })
    }

    /// The destination project this task's own project corresponds to, when there is one.
    ///
    /// A task copied on its own keeps its source's project id when the destination holds
    /// no counterpart: the field is opaque to this engine, and dropping it would lose
    /// what the source said.
    async fn counterpart(
        &self,
        destination: &ResolvedSource,
        item: &Planned,
        task: &Task,
    ) -> Result<Option<NativeId>, EngineError> {
        let Some(project) = &task.project else {
            return Ok(None);
        };
        let qualified = GlobalId::new(item.source.source.clone(), project.clone());
        let found = self
            .scan(
                destination,
                ItemKind::Project,
                &Wanted::Origin(qualified.to_string()),
            )
            .await?;
        Ok(Some(found.unwrap_or_else(|| project.clone())))
    }

    /// Whether writing this item would change what the destination already holds.
    async fn changes(
        &self,
        destination: &ResolvedSource,
        item: &Planned,
        target: &NativeId,
        project: &Option<NativeId>,
        edges: &[DependencyEdge],
    ) -> Result<bool, EngineError> {
        let held = match &item.item {
            Item::Task(_) => destination
                .source()
                .get_task(target)
                .await
                .map_err(|error| refused(destination, error))?
                .map(|task| Item::Task(Box::new(task))),
            Item::Project(_) => destination
                .source()
                .get_project(target)
                .await
                .map_err(|error| refused(destination, error))?
                .map(|project| Item::Project(Box::new(project))),
        };
        let Some(held) = held else {
            return Ok(true);
        };
        let outgoing = outgoing(item, target.clone(), project.clone());
        if !same(&held, &outgoing) {
            return Ok(true);
        }
        let at_destination = forward_edges(destination, target, item.item.kind()).await?;
        Ok(!same_edges(&at_destination, edges))
    }

    /// Hand one item to the destination's own write interface.
    async fn write(
        &self,
        destination: &ResolvedSource,
        item: &Planned,
        target: Option<NativeId>,
        project: Option<NativeId>,
        edges: &[DependencyEdge],
    ) -> Result<NativeId, EngineError> {
        let suggested = target.clone().unwrap_or_else(|| item.item.id().clone());
        match outgoing(item, suggested, project) {
            Item::Task(task) => destination
                .source()
                .write_task(&ItemWrite {
                    target,
                    item: *task,
                    depends_on: edges.to_vec(),
                })
                .await
                .map_err(|error| refused(destination, error)),
            Item::Project(project) => destination
                .source()
                .write_project(&ItemWrite {
                    target,
                    item: *project,
                    depends_on: edges.to_vec(),
                })
                .await
                .map_err(|error| refused(destination, error)),
        }
    }

    /// A configured source that built, for reading an item out of.
    fn readable(&self, name: &SourceName) -> Result<&ResolvedSource, EngineError> {
        let name = self.known(name)?;
        if let Some(unavailable) = self.unavailable().find(|source| source.name() == &name) {
            return Err(EngineError::SourceRefused {
                name: name.to_string(),
                error: unavailable.error().clone(),
            });
        }
        self.ready()
            .find(|source| source.name() == &name)
            .ok_or(EngineError::NoSources)
    }
}

/// How many tasks of a project are read at once while walking it.
const PROJECT_PAGE: std::num::NonZeroU32 = std::num::NonZeroU32::new(50).expect("50 is not zero");

/// One page request against `source`, at the largest page it will serve.
fn request_for(
    source: &ResolvedSource,
    cursor: Option<onetaskgraph_plugin_api::Cursor>,
) -> PageRequest {
    PageRequest {
        cursor,
        limit: source.source().capabilities().max_page_size.max(1),
    }
}

/// One source failing while a copy was mid-flight.
fn refused(source: &ResolvedSource, error: SourceError) -> EngineError {
    EngineError::SourceRefused {
        name: source.name().to_string(),
        error,
    }
}

/// Every forward edge at one item, walked to exhaustion one page at a time.
async fn forward_edges(
    source: &ResolvedSource,
    id: &NativeId,
    kind: ItemKind,
) -> Result<Vec<DependencyEdge>, EngineError> {
    let mut edges = Vec::new();
    let mut cursor = None;
    loop {
        let page = match kind {
            ItemKind::Task => {
                source
                    .source()
                    .task_dependencies(id, Direction::DependsOn, &request_for(source, cursor))
                    .await
            }
            ItemKind::Project => {
                source
                    .source()
                    .project_dependencies(id, Direction::DependsOn, &request_for(source, cursor))
                    .await
            }
        }
        .map_err(|error| refused(source, error))?;
        edges.extend(page.items);
        match page.next {
            Some(next) => cursor = Some(next),
            None => return Ok(edges),
        }
    }
}

/// The origin one item records, when it records a usable one.
fn origin_of(metadata: &BTreeMap<String, Value>) -> Option<GlobalId> {
    metadata
        .get(GlobalId::ORIGIN_KEY)?
        .as_str()?
        .parse::<GlobalId>()
        .ok()
}

/// The title and metadata of either kind of item.
fn described(item: &Item) -> (&str, &BTreeMap<String, Value>) {
    match item {
        Item::Task(task) => (&task.title, &task.metadata),
        Item::Project(project) => (&project.title, &project.metadata),
    }
}

/// The item as the destination should hold it.
///
/// `url`, `created_at` and `updated_at` are the destination's own and are never written.
/// The two reserved keys this product encodes typed fields under are removed, because
/// those fields travel as themselves — leaving the encoding beside them would have the
/// destination hold one thing twice, and disagree with itself the moment one changed.
fn outgoing(item: &Planned, id: NativeId, project: Option<NativeId>) -> Item {
    let origin = item.source.to_string();
    match &item.item {
        Item::Task(task) => Item::Task(Box::new(Task {
            id,
            url: None,
            created_at: None,
            updated_at: None,
            project,
            metadata: carried(&task.metadata, &origin),
            ..(**task).clone()
        })),
        Item::Project(project) => Item::Project(Box::new(Project {
            id,
            url: None,
            created_at: None,
            updated_at: None,
            metadata: carried(&project.metadata, &origin),
            ..(**project).clone()
        })),
    }
}

/// The metadata a copy carries: the caller's own keys untouched, and the origin recorded.
fn carried(metadata: &BTreeMap<String, Value>, origin: &str) -> BTreeMap<String, Value> {
    let mut carried = metadata.clone();
    carried.remove(Repository::METADATA_KEY);
    carried.remove(DependencyEdge::RECORDED_KEY);
    carried.insert(
        GlobalId::ORIGIN_KEY.to_owned(),
        Value::String(origin.to_owned()),
    );
    carried
}

/// Whether the destination already reads exactly as this copy would leave it.
///
/// The destination's own `url` and timestamps are excluded because a copy never writes
/// them, so a difference there is not one this copy would close.
fn same(held: &Item, outgoing: &Item) -> bool {
    match (held, outgoing) {
        (Item::Task(held), Item::Task(outgoing)) => {
            held.title == outgoing.title
                && held.content == outgoing.content
                && held.status == outgoing.status
                && held.labels == outgoing.labels
                && held.project == outgoing.project
                && held.metadata == outgoing.metadata
                && held.repositories == outgoing.repositories
        }
        (Item::Project(held), Item::Project(outgoing)) => {
            held.title == outgoing.title
                && held.content == outgoing.content
                && held.status == outgoing.status
                && held.labels == outgoing.labels
                && held.metadata == outgoing.metadata
                && held.repositories == outgoing.repositories
        }
        _ => false,
    }
}

/// Whether the destination's forward edges already say what this copy would write.
fn same_edges(held: &[DependencyEdge], outgoing: &[DependencyEdge]) -> bool {
    let ends = |edges: &[DependencyEdge]| {
        let mut ends: Vec<(String, ItemKind, DependencyKind)> = edges
            .iter()
            .map(|edge| (edge.to.id().to_owned(), edge.to.kind, edge.kind))
            .collect();
        ends.sort_by(|left, right| left.0.cmp(&right.0));
        ends
    };
    ends(held) == ends(outgoing)
}

/// Each read edge as the destination should record it, or `None` when its far end is a
/// member of this copy whose destination id is not known yet.
fn mapped_edges(
    edges: &[DependencyEdge],
    origin: &SourceName,
    destination: &ResolvedSource,
    copied: &[GlobalId],
    written: &BTreeMap<String, NativeId>,
) -> Vec<Option<DependencyEdge>> {
    edges
        .iter()
        .map(|edge| {
            let far = GlobalId::new(origin.clone(), NativeId(edge.to.id().to_owned()));
            let id = if let Some(native) = names(&edge.to, destination.name()) {
                // A far end already qualified to the destination's own source is that
                // source's own item, so it is written the way that source names its own:
                // unqualified. Leaving it qualified would have the destination hold an
                // edge into itself written as if it left, which is the one spelling the
                // reserved key exists to keep for edges that really do.
                Some(native)
            } else if edge.to.is_qualified() || origin == destination.name() {
                // Already naming a source of its own, or a copy inside one source where
                // the far end's own id is the destination's id.
                Some(edge.to.id().to_owned())
            } else if copied.contains(&far) {
                written.get(&far.to_string()).map(|native| native.0.clone())
            } else {
                Some(far.to_string())
            }?;
            DependencyEndpoint::new(id, edge.to.kind)
                .ok()
                .map(|to| DependencyEdge {
                    from: edge.from.clone(),
                    to,
                    kind: edge.kind,
                })
        })
        .collect()
}

/// The native id a qualified endpoint names at `destination`, when it names one there.
fn names(endpoint: &DependencyEndpoint, destination: &SourceName) -> Option<String> {
    if !endpoint.is_qualified() {
        return None;
    }
    let id: GlobalId = endpoint.id().parse().ok()?;
    (&id.source == destination).then_some(id.native.0)
}

/// The edges that could be resolved, which is every one of them on the second pass.
fn resolved(edges: &[Option<DependencyEdge>]) -> Vec<DependencyEdge> {
    edges.iter().flatten().cloned().collect()
}

/// Whether the destination holds an item with this id.
async fn exists(
    destination: &ResolvedSource,
    id: &NativeId,
    kind: ItemKind,
) -> Result<bool, EngineError> {
    let found = match kind {
        ItemKind::Task => destination
            .source()
            .get_task(id)
            .await
            .map_err(|error| refused(destination, error))?
            .is_some(),
        ItemKind::Project => destination
            .source()
            .get_project(id)
            .await
            .map_err(|error| refused(destination, error))?
            .is_some(),
    };
    Ok(found)
}
