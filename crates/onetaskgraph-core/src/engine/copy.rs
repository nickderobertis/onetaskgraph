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
//! Which rule found the item decides what the copy records there. A copy that got its
//! target from rule 1 is a copy-back — the destination is the *original*, and the item
//! being copied is the one that came from it — so the destination keeps the origin it
//! already holds, holding none included. Every other copy records the id it was copied
//! from. See [`recorded`] for what stamping a copy-back's own id there costs.
//!
//! A destination write is at the user's explicit request, names its destination, goes
//! through that source's own write interface into that source's own store, and is never
//! read back to answer a query. That is what makes it a write and not a cache.

use std::collections::BTreeMap;

use onetaskgraph_plugin_api::{
    DependencyEdge, DependencyEndpoint, DependencyKind, Direction, Document, DocumentQuery,
    ItemKind, ItemWrite, NativeId, Page, PageRequest, Project, ProjectQuery, Repository,
    SourceError, SourceName, Task, TaskQuery,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::GlobalId;
use crate::resolve::ResolvedSource;

use super::local::ProjectSelector;
use super::{Engine, EngineError, Filters, LeftBehind, Paging, TaskRequest};

/// A request to copy work into one configured destination.
#[derive(Debug, Clone)]
pub struct CopyRequest {
    /// The qualified items to copy, in the order they were named.
    pub items: CopyItems,
    /// What those ids name, and what comes with them.
    pub scope: CopyScope,
    /// The configured source to copy into — a source name, never a qualified id.
    pub destination: SourceName,
    /// How to re-establish a correspondence the two origin rules cannot find.
    pub match_by: Option<MatchBy>,
    /// Whether an origin naming nothing at the destination falls through to the search
    /// rule instead of refusing.
    pub recreate: bool,
    /// Whether to perform every read and no write.
    pub dry_run: bool,
}

/// The items one copy names: at least one, because a copy naming none is not a copy.
///
/// A newtype rather than a bare `Vec`, for the reason [`Repository`] is one: the empty
/// list is not a copy of nothing, it is a caller mistake, and a type that can hold it
/// leaves every reader to decide what it means — a report with no entries, an error, a
/// silent success. None of those is better than not being able to say it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyItems(Vec<GlobalId>);

impl CopyItems {
    /// The items a caller named, or `None` when they named none.
    #[must_use]
    pub fn new(items: Vec<GlobalId>) -> Option<Self> {
        (!items.is_empty()).then_some(Self(items))
    }

    /// The items, in the order they were named.
    #[must_use]
    pub fn as_slice(&self) -> &[GlobalId] {
        &self.0
    }
}

/// What the ids a copy names are, and what travels with them.
///
/// One value rather than a kind beside a flag, because three of the four combinations
/// those two would make are real and the fourth — tasks, with the tasks of each also
/// copied — means nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyScope {
    /// The ids name tasks, and only those tasks are copied.
    Tasks,
    /// The ids name projects.
    Projects {
        /// Whether the tasks in each project are copied too.
        tasks: bool,
    },
    /// The ids name documents, and only those documents are copied.
    ///
    /// Nothing travels with a document: it takes part in no dependency graph, and it holds
    /// nothing of its own the way a project holds tasks.
    Documents,
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
///
/// `action` and `destination` are one value rather than two fields side by side: an
/// updated item without a destination id, or an orphan without one, are states this type
/// must not be able to say — the id *is* what those outcomes are about. The one outcome
/// that legitimately has none is a dry run that would create, because nothing was
/// created and there is no id to report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CopyOutcome {
    /// The qualified id the item was read from.
    pub source: GlobalId,
    /// What happened to it, and where.
    #[serde(flatten)]
    pub action: CopyAction,
}

impl CopyOutcome {
    /// The qualified id this outcome landed on, when it landed on one.
    #[must_use]
    pub fn destination(&self) -> Option<&GlobalId> {
        self.action.destination()
    }
}

/// The four things a copy can do to one item, and the id each of them is about.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "kebab-case")]
pub enum CopyAction {
    // llmlint: ignore[names_match_behavior] `created` is Contract D's serialized action
    // for both a completed create and a dry run that would create; the optional destination
    // distinguishes those cases, and renaming this public variant would break Rust callers.
    /// The destination held no counterpart, so one was created.
    Created {
        /// The id it was created under, or `null` for a dry run that would have created
        /// one — there is no id, because nothing was.
        destination: Option<GlobalId>,
    },
    /// The destination held a counterpart and it now reads as the source does.
    Updated {
        /// The item that was updated.
        destination: GlobalId,
    },
    /// The destination held a counterpart that already read that way; nothing was written.
    Unchanged {
        /// The item that already said it.
        destination: GlobalId,
    },
    /// The destination holds a counterpart the source no longer does. A copy never
    /// deletes, so it was left exactly as it is.
    Orphaned {
        /// The item that was left alone.
        destination: GlobalId,
    },
}

impl CopyAction {
    /// The qualified id this action is about, when there is one.
    #[must_use]
    pub fn destination(&self) -> Option<&GlobalId> {
        match self {
            Self::Created { destination } => destination.as_ref(),
            Self::Updated { destination }
            | Self::Unchanged { destination }
            | Self::Orphaned { destination } => Some(destination),
        }
    }

    /// The word this action serializes as, taken from its own `Serialize`.
    ///
    /// Read back off the wire form rather than written out again in a `match`, for the
    /// reason `render::wire` gives: a second spelling of `unchanged` would be a second
    /// place for it to drift from the one a caller reads.
    #[must_use]
    pub fn name(&self) -> String {
        serde_json::to_value(self).expect("a contract enum serialises")["action"]
            .as_str()
            .expect("an internally tagged enum carries its tag")
            .to_owned()
    }
}

/// Where one item is going at the destination.
enum Target {
    /// Update the destination item with this id, reached by the rule named.
    Update {
        /// The destination item this copy updates.
        id: NativeId,
        /// Which rule found it.
        found: Found,
    },
    /// Create one.
    Create,
}

/// Which of the rules above found the destination item a copy is updating.
///
/// The two are the same instruction — update that item — and a different answer about the
/// origin, which is why the distinction is carried this far rather than dropped where it
/// is made. See [`recorded`].
#[derive(Clone, Copy, PartialEq, Eq)]
enum Found {
    /// Rule 1: the item being copied already named it, so this copy is a copy-back.
    Origin,
    /// Rule 2 or the caller's matching escape: the destination was searched for it.
    Search,
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

/// What the destination held before this copy touched one item.
///
/// Read once, in [`Engine::land`], and used three times over: to decide whether the write
/// would change anything, to repair the item's edges once the rest of the copy has landed,
/// and — if the copy cannot finish — to put the item back exactly as it was.
#[derive(Clone)]
struct Prior {
    /// The item as the destination held it.
    item: Item,
    /// Its forward edges there.
    edges: Vec<DependencyEdge>,
}

/// One item that landed with an edge whose far end was not written yet.
///
/// Held until every item of the whole copy has landed, because the far end may be in
/// another project of the same command: a copy of two projects at once is one copied set,
/// not two, and an edge across them is remapped rather than written as a foreign id.
struct Deferred {
    /// The item, as it was read and resolved.
    item: Planned,
    /// The destination project it was filed under.
    filed: Option<NativeId>,
    /// Where it landed.
    destination: NativeId,
    /// What the destination held there before, when it held anything.
    prior: Option<Prior>,
}

/// What one item's undo has to do to put the destination back.
enum Undo {
    /// The copy created it, so undoing means removing it.
    Created {
        /// Which write interface removes it.
        kind: Level,
        /// The destination id it was created under.
        id: NativeId,
    },
    /// The copy overwrote something, so undoing means writing that something back.
    ///
    /// No `kind` beside the id, unlike the variant above: what was there says which of the
    /// two write interfaces takes it back, and a second spelling of that could disagree
    /// with it.
    Updated {
        /// The destination id that was overwritten.
        id: NativeId,
        /// What was there before.
        prior: Prior,
    },
}

impl Undo {
    /// The destination id this entry is about.
    fn id(&self) -> &NativeId {
        match self {
            Self::Created { id, .. } | Self::Updated { id, .. } => id,
        }
    }

    /// Which of the destination's three write interfaces this entry belongs to.
    ///
    /// An id alone does not identify a destination item: nothing stops a destination
    /// numbering its tasks and its projects in one namespace, and a local-Markdown store
    /// filing `alpha.md` under both is the ordinary case rather than the contrived one.
    /// So this pairs with `id` wherever one entry has to be told from another.
    fn kind(&self) -> Level {
        match self {
            Self::Created { kind, .. } => *kind,
            // Read off what was there, for the reason the variant carries no `kind` of
            // its own: two spellings of one fact can disagree, and this one cannot.
            Self::Updated { prior, .. } => prior.item.level(),
        }
    }
}

/// Everything one copy has written, in the order it wrote it, so a copy that cannot finish
/// can undo its own writes.
///
/// This is not state the engine keeps: it lives for the length of one `copy` call and is
/// dropped with it, so the invariant that nothing of a user's work is written down outside
/// the plugin that owns it is untouched.
#[derive(Default)]
struct Journal {
    /// One entry per destination item this copy first touched, in that order.
    entries: Vec<Undo>,
}

impl Journal {
    /// Record what has to happen to put one destination item back.
    ///
    /// The *first* entry for an id is the one that matters and later ones are dropped: an
    /// item written twice — once as it lands, once when its edges are repaired — was only
    /// ever one thing before this copy started, and that is what undoing it restores.
    fn record(&mut self, entry: Undo) {
        if self
            .entries
            .iter()
            .any(|held| held.kind() == entry.kind() && held.id() == entry.id())
        {
            return;
        }
        self.entries.push(entry);
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

/// A task, a project or a document, so the copy path is written once.
#[derive(Clone)]
enum Item {
    /// A task.
    Task(Box<Task>),
    /// A project.
    Project(Box<Project>),
    /// A document.
    Document(Box<Document>),
}

impl Item {
    fn id(&self) -> &NativeId {
        match self {
            Self::Task(task) => &task.id,
            Self::Project(project) => &project.id,
            Self::Document(document) => &document.id,
        }
    }

    fn level(&self) -> Level {
        match self {
            Self::Task(_) => Level::Task,
            Self::Project(_) => Level::Project,
            Self::Document(_) => Level::Document,
        }
    }
}

/// Which of a destination's three read-and-write interfaces one item belongs to.
///
/// Deliberately not [`ItemKind`]: that enum names what a *dependency endpoint* points at,
/// and the contract gives it no document variant because nothing may point at a document.
/// This one names which pair of methods reads and writes an item, which is a different
/// question with a third answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Level {
    /// `get_task`, `write_task`, `delete_task`.
    Task,
    /// `get_project`, `write_project`, `delete_project`.
    Project,
    /// `get_document`, `write_document`, `delete_document`.
    Document,
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
    /// Returns [`EngineError`] when the destination is not configured, cannot be built,
    /// cannot be written, or — for a document copy — declares it has no documents; when an
    /// id names nothing; when an origin names an item the
    /// destination no longer holds and `--recreate` was not given; and when the
    /// destination refuses the write — including a field or a metadata key it cannot
    /// carry, which it names rather than dropping.
    pub async fn copy(&self, request: &CopyRequest) -> Result<CopyReport, EngineError> {
        let destination = self.writable(&request.destination)?;
        // Before anything is read, and from the declaration rather than from a failed
        // write: a destination that says it has no documents has nowhere to put one.
        if request.scope == CopyScope::Documents {
            documentary(destination)?;
        }
        let mut journal = Journal::default();
        match self.copy_all(destination, request, &mut journal).await {
            Ok(report) => Ok(report),
            Err(error) => Err(self.undo(destination, journal, error).await),
        }
    }

    /// The copy itself, with everything it writes recorded so a failure can be undone.
    ///
    /// The ids named together are **one** copied set, and that is what makes an edge
    /// between any two of them a real edge at the destination: a copy of two projects at
    /// once knows that a task in the first depends on a task in the second, and a task
    /// knows that the project it belongs to is being created beside it. Copying them one
    /// at a time could not, and wrote the far end as the id it had at its *source* — a
    /// dangling reference to somewhere the destination has never heard of.
    async fn copy_all(
        &self,
        destination: &ResolvedSource,
        request: &CopyRequest,
        journal: &mut Journal,
    ) -> Result<CopyReport, EngineError> {
        // Keyed by the qualified id's own rendering, which is what a recorded origin holds
        // anyway — making `GlobalId` orderable for one local map would put an ordering on
        // a contract type for a reason no caller of it has.
        let mut written: BTreeMap<String, NativeId> = BTreeMap::new();
        let mut deferred: Vec<Deferred> = Vec::new();
        // The whole copied set, established before anything is written. For a project
        // copy that means reading every named project's membership first: the set is the
        // whole request rather than one project of it.
        let mut membership = Vec::new();
        let mut copied = Vec::new();
        match request.scope {
            CopyScope::Tasks | CopyScope::Documents => {
                copied.extend(request.items.as_slice().iter().cloned());
            }
            CopyScope::Projects { tasks } => {
                for id in request.items.as_slice() {
                    let members = if tasks {
                        self.project_members(id).await?
                    } else {
                        Vec::new()
                    };
                    copied.push(id.clone());
                    copied.extend(members.iter().cloned());
                    membership.push((id.clone(), members));
                }
            }
        }
        let items = match request.scope {
            CopyScope::Tasks | CopyScope::Documents => {
                self.copy_items(
                    destination,
                    request,
                    match request.scope {
                        CopyScope::Documents => Level::Document,
                        _ => Level::Task,
                    },
                    request.items.as_slice(),
                    None,
                    &copied,
                    &mut written,
                    &mut deferred,
                    journal,
                )
                .await?
            }
            CopyScope::Projects { tasks } => {
                let mut items = Vec::new();
                for (id, members) in &membership {
                    items.extend(
                        self.copy_project(
                            destination,
                            request,
                            id,
                            members,
                            tasks,
                            &copied,
                            &mut written,
                            &mut deferred,
                            journal,
                        )
                        .await?,
                    );
                }
                items
            }
        };
        self.repair(destination, request, &copied, &written, deferred, journal)
            .await?;
        Ok(CopyReport { items })
    }

    /// Write every deferred item again, now that every destination id is known.
    ///
    /// This is the second half of the two passes an edge between two items of one copy
    /// needs: the far end's destination id does not exist until it has been created, so
    /// the item that points at it lands first without that edge and is completed here.
    /// It runs once for the whole request rather than once per project, because a far end
    /// may be in a project this copy has not reached yet.
    async fn repair(
        &self,
        destination: &ResolvedSource,
        request: &CopyRequest,
        copied: &[GlobalId],
        written: &BTreeMap<String, NativeId>,
        deferred: Vec<Deferred>,
        journal: &mut Journal,
    ) -> Result<(), EngineError> {
        if request.dry_run {
            return Ok(());
        }
        for entry in deferred {
            let edges = mapped_edges(
                &entry.item.edges,
                &entry.item.source.source,
                destination,
                copied,
                written,
            );
            self.write(
                destination,
                &entry.item,
                Some(entry.destination),
                entry.filed,
                &resolved(&edges),
                entry.prior,
                journal,
            )
            .await?;
        }
        Ok(())
    }

    /// Put the destination back the way this copy found it, then report why it failed.
    ///
    /// Undone in reverse, and an item this copy created is removed rather than restored —
    /// the entry recording what it looked like a moment after creation is not a state
    /// anybody asked for. When the destination cannot take one of them back, the refusal
    /// says so and names what is still there, because a user told "the copy failed" about
    /// a destination that is not as they left it will copy again over a tree nobody
    /// described.
    async fn undo(
        &self,
        destination: &ResolvedSource,
        journal: Journal,
        error: EngineError,
    ) -> EngineError {
        let created: Vec<(Level, &NativeId)> = journal
            .entries
            .iter()
            .filter_map(|entry| match entry {
                Undo::Created { kind, id } => Some((*kind, id)),
                Undo::Updated { .. } => None,
            })
            .collect();
        // The ids and the refusal are one value rather than two, because they are one
        // fact: an item is only left behind because the destination refused to take it
        // back, so the first refusal carries the first id and neither half can be
        // recorded without the other.
        let mut unrestored: Option<(LeftBehind, SourceError)> = None;
        for entry in journal.entries.iter().rev() {
            let outcome = match entry {
                Undo::Created { kind, id } => remove(destination, *kind, id).await,
                Undo::Updated { id, prior, .. } if !created.contains(&(prior.item.level(), id)) => {
                    restore(destination, id, prior).await
                }
                Undo::Updated { .. } => Ok(()),
            };
            if let Err(problem) = outcome {
                let id = GlobalId::new(destination.name().clone(), entry.id().clone());
                match &mut unrestored {
                    Some((left_behind, _)) => left_behind.push(id),
                    None => unrestored = Some((LeftBehind::new(id), problem)),
                }
            }
        }
        match unrestored {
            None => error,
            Some((left_behind, refusal)) => EngineError::CopyNotUndone {
                error: Box::new(error),
                left_behind,
                refusal,
            },
        }
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
    // llmlint: ignore[suppressions_justified] Five of these are the copy's own running
    // state — the copied set, the ids written so far, the items held back for repair and
    // the undo journal — and every one of them is shared across the whole request rather
    // than per project, which is the defect this signature exists to close. Bundling them
    // into a context struct would put a lifetime and a borrow split around state that is
    // threaded through three call sites and read nowhere else.
    #[allow(clippy::too_many_arguments)]
    async fn copy_project(
        &self,
        destination: &ResolvedSource,
        request: &CopyRequest,
        id: &GlobalId,
        members: &[GlobalId],
        tasks: bool,
        copied: &[GlobalId],
        written: &mut BTreeMap<String, NativeId>,
        deferred: &mut Vec<Deferred>,
        journal: &mut Journal,
    ) -> Result<Vec<CopyOutcome>, EngineError> {
        // On a repeat copy, compare the project with its final remapped edges before the
        // first pass temporarily rewrites it. This preserves an `unchanged` outcome when
        // the project and every copied member already have counterparts.
        let project_plan = self.plan(destination, request, Level::Project, id).await?;
        let mut known = BTreeMap::new();
        if let Target::Update { id: target, .. } = &project_plan.target {
            known.insert(id.to_string(), target.clone());
        }
        for member in members {
            let member_plan = self.plan(destination, request, Level::Task, member).await?;
            if let Target::Update { id: target, .. } = member_plan.target {
                known.insert(member.to_string(), target);
            }
        }
        let project_was_unchanged = if let Target::Update { id: target, .. } = &project_plan.target
        {
            let edges = mapped_edges(&project_plan.edges, &id.source, destination, copied, &known);
            let held = self.prior(destination, Level::Project, target).await?;
            !edges.iter().any(Option::is_none)
                && !changes(
                    held.as_ref(),
                    &project_plan,
                    target,
                    &None,
                    &resolved(&edges),
                )
        } else {
            false
        };
        let mut outcomes = self
            .copy_items(
                destination,
                request,
                Level::Project,
                std::slice::from_ref(id),
                None,
                copied,
                written,
                deferred,
                journal,
            )
            .await?;
        if !tasks {
            return Ok(outcomes);
        }
        // `None` when a dry run would have created the project: nothing was written, so
        // there is no destination project id to file the tasks under. Every task is still
        // read and still reported, because that is what a dry run is for.
        let project = outcomes.first().and_then(CopyOutcome::destination).cloned();
        let task_outcomes = self
            .copy_items(
                destination,
                request,
                Level::Task,
                members,
                project.as_ref().map(|project| project.native.clone()),
                copied,
                written,
                deferred,
                journal,
            )
            .await?;
        outcomes.extend(task_outcomes);
        if let Some(project) = project {
            if project_was_unchanged {
                outcomes[0].action = CopyAction::Unchanged {
                    destination: project.clone(),
                };
            }
            outcomes.extend(
                self.orphans(destination, id, &project.native, members)
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
                    action: CopyAction::Orphaned {
                        destination: GlobalId::new(destination.name().clone(), task.id.clone()),
                    },
                });
            }
            match page.next {
                Some(next) => cursor = Some(next),
                None => return Ok(orphans),
            }
        }
    }

    /// Read, resolve and write every item named, holding back the ones whose edges are
    /// not resolvable yet.
    ///
    /// An edge between two items of one copy can point at a member whose destination id
    /// does not exist until it has been created, so the item that points at it lands
    /// without that edge and is handed to `deferred`. [`Engine::repair`] finishes it once
    /// the *whole* request has landed — not once this call has, because the far end may
    /// be in another project of the same command.
    // llmlint: ignore[suppressions_justified] The same running state `copy_project` threads,
    // for the same reason: it belongs to one `copy` call and is shared across every item of
    // it, and a struct around it would add a borrow split for no reader's benefit.
    #[allow(clippy::too_many_arguments)]
    async fn copy_items(
        &self,
        destination: &ResolvedSource,
        request: &CopyRequest,
        kind: Level,
        items: &[GlobalId],
        project: Option<NativeId>,
        copied: &[GlobalId],
        written: &mut BTreeMap<String, NativeId>,
        deferred: &mut Vec<Deferred>,
        journal: &mut Journal,
    ) -> Result<Vec<CopyOutcome>, EngineError> {
        let mut planned = Vec::new();
        for id in items {
            planned.push(self.plan(destination, request, kind, id).await?);
        }

        for item in &planned {
            if let Target::Update { id, .. } = &item.target {
                written.insert(item.source.to_string(), id.clone());
            }
        }

        // Resolved once per item, and used by both passes: the repair pass writes the
        // same item again, and re-deriving this there could file it somewhere else.
        let mut filed = Vec::new();
        for item in &planned {
            filed.push(self.filed(destination, item, project.clone()).await?);
        }

        let mut outcomes = Vec::new();
        let mut unresolved = Vec::new();
        let mut priors = Vec::new();
        for (index, item) in planned.iter().enumerate() {
            let edges = mapped_edges(
                &item.edges,
                &item.source.source,
                destination,
                copied,
                written,
            );
            if edges.iter().any(Option::is_none) {
                unresolved.push(index);
            }
            let (outcome, prior) = self
                .land(
                    destination,
                    request,
                    item,
                    filed[index].clone(),
                    &edges,
                    journal,
                )
                .await?;
            if let Some(id) = outcome.destination() {
                written.insert(item.source.to_string(), id.native.clone());
            }
            outcomes.push(outcome);
            priors.push(prior);
        }

        if !request.dry_run {
            for (index, item) in planned.into_iter().enumerate() {
                if !unresolved.contains(&index) {
                    continue;
                }
                // Every item a copy that is not a dry run lands has a destination id: the
                // one outcome without one is a dry run that would have created, and this
                // block does not run for a dry run.
                let id = outcomes[index]
                    .destination()
                    .expect("a copy that writes lands every item it planned")
                    .clone();
                deferred.push(Deferred {
                    item,
                    filed: filed[index].clone(),
                    destination: id.native,
                    prior: priors[index].clone(),
                });
            }
        }
        Ok(outcomes)
    }

    /// Read one item and its forward edges, and decide where it is going.
    async fn plan(
        &self,
        destination: &ResolvedSource,
        request: &CopyRequest,
        kind: Level,
        id: &GlobalId,
    ) -> Result<Planned, EngineError> {
        let source = self.readable(&id.source)?;
        if kind == Level::Document {
            documentary(source)?;
        }
        let item = match kind {
            Level::Task => source
                .source()
                .get_task(&id.native)
                .await
                .map_err(|error| refused(source, error))?
                .map(|task| Item::Task(Box::new(task))),
            Level::Project => source
                .source()
                .get_project(&id.native)
                .await
                .map_err(|error| refused(source, error))?
                .map(|project| Item::Project(Box::new(project))),
            Level::Document => source
                .source()
                .get_document(&id.native)
                .await
                .map_err(|error| refused(source, error))?
                .map(|document| Item::Document(Box::new(document))),
        }
        .ok_or_else(|| EngineError::NoSuchItem { id: id.to_string() })?;
        let edges = forward_edges(source, &id.native, item.level()).await?;
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
            if exists(destination, &origin.native, item.level()).await? {
                return Ok(Target::Update {
                    id: origin.native,
                    found: Found::Origin,
                });
            }
            if !request.recreate {
                return Err(EngineError::StaleOrigin {
                    item: id.to_string(),
                    origin: origin.to_string(),
                });
            }
        }
        if let Some(found) = self
            .scan(destination, item.level(), &Wanted::Origin(id.to_string()))
            .await?
        {
            return Ok(Target::Update {
                id: found,
                found: Found::Search,
            });
        }
        let wanted = match &request.match_by {
            Some(MatchBy::Title) => Some(Wanted::Title(title.to_owned())),
            Some(MatchBy::Metadata(key)) => metadata
                .get(key)
                .map(|value| Wanted::Metadata(key.clone(), value.clone())),
            None => None,
        };
        if let Some(wanted) = wanted
            && let Some(found) = self.scan(destination, item.level(), &wanted).await?
        {
            return Ok(Target::Update {
                id: found,
                found: Found::Search,
            });
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
        kind: Level,
        wanted: &Wanted,
    ) -> Result<Option<NativeId>, EngineError> {
        let mut cursor = None;
        loop {
            let next = match kind {
                Level::Task => {
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
                Level::Project => {
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
                Level::Document => {
                    let page = destination
                        .source()
                        .query_documents(
                            &DocumentQuery::default(),
                            &request_for(destination, cursor),
                        )
                        .await
                        .map_err(|error| refused(destination, error))?;
                    for document in &page.items {
                        if wanted.found(&document.title, &document.metadata) {
                            return Ok(Some(document.id.clone()));
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
    ///
    /// Answers with what the destination held there beforehand as well, which is what
    /// makes an item written twice restorable to what it was rather than to what this
    /// copy's first pass left.
    async fn land(
        &self,
        destination: &ResolvedSource,
        request: &CopyRequest,
        item: &Planned,
        project: Option<NativeId>,
        edges: &[Option<DependencyEdge>],
        journal: &mut Journal,
    ) -> Result<(CopyOutcome, Option<Prior>), EngineError> {
        let target = match &item.target {
            Target::Update { id, .. } => Some(id.clone()),
            Target::Create => None,
        };
        // One read of the destination item, used to decide whether the write changes
        // anything and — if the copy cannot finish — to put that item back.
        let prior = match &target {
            Some(id) => self.prior(destination, item.item.level(), id).await?,
            None => None,
        };
        let edges = resolved(edges);
        let qualified = |native: NativeId| GlobalId::new(destination.name().clone(), native);
        if let Some(id) = &target
            && !changes(prior.as_ref(), item, id, &project, &edges)
        {
            return Ok((
                CopyOutcome {
                    source: item.source.clone(),
                    action: CopyAction::Unchanged {
                        destination: qualified(id.clone()),
                    },
                },
                prior,
            ));
        }
        if request.dry_run {
            return Ok((
                CopyOutcome {
                    source: item.source.clone(),
                    action: match target {
                        Some(id) => CopyAction::Updated {
                            destination: qualified(id),
                        },
                        // Null only here: nothing was created, so there is no id to report.
                        None => CopyAction::Created { destination: None },
                    },
                },
                prior,
            ));
        }
        let updating = target.is_some();
        let written = qualified(
            self.write(
                destination,
                item,
                target,
                project,
                &edges,
                prior.clone(),
                journal,
            )
            .await?,
        );
        Ok((
            CopyOutcome {
                source: item.source.clone(),
                action: if updating {
                    CopyAction::Updated {
                        destination: written,
                    }
                } else {
                    CopyAction::Created {
                        destination: Some(written),
                    }
                },
            },
            prior,
        ))
    }

    /// Which destination project this item is filed under, when it is filed at all.
    ///
    /// A task copied as part of a project copy is filed under that project's counterpart,
    /// which the copy has just established. A task copied on its own has to find it.
    async fn filed(
        &self,
        destination: &ResolvedSource,
        item: &Planned,
        project: Option<NativeId>,
    ) -> Result<Option<NativeId>, EngineError> {
        match (&item.item, project) {
            (Item::Task(task), None) => {
                self.counterpart(destination, item, task.project.as_ref())
                    .await
            }
            (Item::Document(document), None) => {
                self.counterpart(destination, item, document.project.as_ref())
                    .await
            }
            (Item::Task(_) | Item::Document(_), filed) => Ok(filed),
            (Item::Project(_), _) => Ok(None),
        }
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
        project: Option<&NativeId>,
    ) -> Result<Option<NativeId>, EngineError> {
        let Some(project) = project else {
            return Ok(None);
        };
        let qualified = GlobalId::new(item.source.source.clone(), project.clone());
        let found = self
            .scan(
                destination,
                Level::Project,
                &Wanted::Origin(qualified.to_string()),
            )
            .await?;
        Ok(Some(found.unwrap_or_else(|| project.clone())))
    }

    /// What the destination holds at one id, item and forward edges together.
    ///
    /// One read for both purposes it serves — deciding whether a write changes anything,
    /// and putting the item back if the copy cannot finish — because a second read of the
    /// same item is a second round trip against a hosted destination for nothing.
    async fn prior(
        &self,
        destination: &ResolvedSource,
        kind: Level,
        id: &NativeId,
    ) -> Result<Option<Prior>, EngineError> {
        let held = match kind {
            Level::Task => destination
                .source()
                .get_task(id)
                .await
                .map_err(|error| refused(destination, error))?
                .map(|task| Item::Task(Box::new(task))),
            Level::Project => destination
                .source()
                .get_project(id)
                .await
                .map_err(|error| refused(destination, error))?
                .map(|project| Item::Project(Box::new(project))),
            Level::Document => destination
                .source()
                .get_document(id)
                .await
                .map_err(|error| refused(destination, error))?
                .map(|document| Item::Document(Box::new(document))),
        };
        let Some(item) = held else {
            return Ok(None);
        };
        let edges = forward_edges(destination, id, kind).await?;
        Ok(Some(Prior { item, edges }))
    }

    /// Hand one item to the destination's own write interface, recording how to take it
    /// back.
    // llmlint: ignore[suppressions_justified] A write is the item, where it is going, what
    // it is filed under, its edges, what was there before and the journal that records how
    // to put it back. Each is a distinct decision made by a different part of the copy, and
    // grouping them would only move the argument list to a constructor.
    #[allow(clippy::too_many_arguments)]
    async fn write(
        &self,
        destination: &ResolvedSource,
        item: &Planned,
        target: Option<NativeId>,
        project: Option<NativeId>,
        edges: &[DependencyEdge],
        prior: Option<Prior>,
        journal: &mut Journal,
    ) -> Result<NativeId, EngineError> {
        let created_kind = item.item.level();
        let suggested = target.clone().unwrap_or_else(|| item.item.id().clone());
        // Settled before the journal takes `prior`, and from that same read: what the
        // destination holds at the origin key is what a copy-back leaves there.
        let origin = recorded(item, prior.as_ref());
        // Recorded *before* the write rather than after it. A destination's own write is
        // several calls — `docs/plugin-protocol.md` §4.9 — and one of them failing leaves
        // the ones before it applied. No source can put those back, because only this
        // journal holds what was there; recorded after a successful write, an update that
        // stopped part way was the one way a copy could end and leave the destination
        // altered. A restore of an item the write never reached rewrites what is already
        // there, which costs one mutation and is what "either complete or it never
        // happened" is worth.
        if let (Some(id), Some(prior)) = (target.clone(), prior) {
            journal.record(Undo::Updated { id, prior });
        }
        let landed = match outgoing(item, suggested, project, &origin) {
            Item::Task(task) => destination
                .source()
                .write_task(&ItemWrite {
                    target: target.clone(),
                    item: *task,
                    depends_on: edges.to_vec(),
                })
                .await
                .map_err(|error| refused(destination, error))?,
            Item::Project(project) => destination
                .source()
                .write_project(&ItemWrite {
                    target: target.clone(),
                    item: *project,
                    depends_on: edges.to_vec(),
                })
                .await
                .map_err(|error| refused(destination, error))?,
            // No edges, and that is the contract: a document takes part in no dependency
            // graph, so there is nothing here for `depends_on` to carry.
            Item::Document(document) => destination
                .source()
                .write_document(&ItemWrite {
                    target: target.clone(),
                    item: *document,
                    depends_on: Vec::new(),
                })
                .await
                .map_err(|error| refused(destination, error))?,
        };
        // A created item can only be journalled here: its id is what the write answers
        // with. A create that fails leaves nothing behind — §4.9 makes taking the item
        // back the source's own duty, because a write that refused must not leave an item
        // nobody asked for.
        if target.is_none() {
            journal.record(Undo::Created {
                kind: created_kind,
                id: landed.clone(),
            });
        }
        Ok(landed)
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

/// Whether writing this item would change what the destination already holds.
///
/// A free function over the state already read rather than a method that reads it again:
/// the same answer is wanted where the item is landed and where a repeat copy of a project
/// decides whether it settled, and a second read there is a second round trip for nothing.
fn changes(
    held: Option<&Prior>,
    item: &Planned,
    target: &NativeId,
    project: &Option<NativeId>,
    edges: &[DependencyEdge],
) -> bool {
    let Some(held) = held else {
        return true;
    };
    let outgoing = outgoing(
        item,
        target.clone(),
        project.clone(),
        &recorded(item, Some(held)),
    );
    !same(&held.item, &outgoing) || !same_edges(&held.edges, edges)
}

/// Remove one item this copy created, through the destination's own write interface.
async fn remove(
    destination: &ResolvedSource,
    kind: Level,
    id: &NativeId,
) -> Result<(), SourceError> {
    match kind {
        Level::Task => destination.source().delete_task(id).await,
        Level::Project => destination.source().delete_project(id).await,
        Level::Document => destination.source().delete_document(id).await,
    }
}

/// Write one item back exactly as the destination held it before this copy.
async fn restore(
    destination: &ResolvedSource,
    id: &NativeId,
    prior: &Prior,
) -> Result<(), SourceError> {
    match &prior.item {
        Item::Task(task) => destination
            .source()
            .write_task(&ItemWrite {
                target: Some(id.clone()),
                item: (**task).clone(),
                depends_on: prior.edges.clone(),
            })
            .await
            .map(|_| ()),
        Item::Project(project) => destination
            .source()
            .write_project(&ItemWrite {
                target: Some(id.clone()),
                item: (**project).clone(),
                depends_on: prior.edges.clone(),
            })
            .await
            .map(|_| ()),
        Item::Document(document) => destination
            .source()
            .write_document(&ItemWrite {
                target: Some(id.clone()),
                item: (**document).clone(),
                depends_on: Vec::new(),
            })
            .await
            .map(|_| ()),
    }
}

/// Refuse a document copy addressed to a source that declares it has none.
///
/// Read off the declaration rather than by asking, which is what "not asked" means: the
/// engine learned at the handshake that this source holds no documents, so it refuses
/// naming the source and its plugin instead of sending a read that would be refused there.
/// Applied at both ends of a copy — a source with no documents holds nothing to copy out,
/// and a destination with none has nowhere to put one.
fn documentary(source: &ResolvedSource) -> Result<(), EngineError> {
    if source.source().capabilities().documents.is_native() {
        return Ok(());
    }
    Err(EngineError::NoDocuments {
        name: source.name().to_string(),
        kind: source.kind().to_owned(),
    })
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
    kind: Level,
) -> Result<Vec<DependencyEdge>, EngineError> {
    // A document has no edges to walk, and asking for them would mean asking a source for
    // a graph the contract says nothing may point into.
    if kind == Level::Document {
        return Ok(Vec::new());
    }
    let mut edges = Vec::new();
    let mut cursor = None;
    loop {
        let page = match kind {
            Level::Task | Level::Document => {
                source
                    .source()
                    .task_dependencies(id, Direction::DependsOn, &request_for(source, cursor))
                    .await
            }
            Level::Project => {
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
        Item::Document(document) => (&document.title, &document.metadata),
    }
}

/// The item as the destination should hold it.
///
/// `url`, `location`, `created_at` and `updated_at` are the destination's own and are
/// never written — where the *source* holds an item says nothing about where the
/// destination does, which is why a copied document does not arrive claiming the path or
/// the link its source reported. The two reserved keys this product encodes typed fields
/// under are removed, because those fields travel as themselves — leaving the encoding
/// beside them would have the destination hold one thing twice, and disagree with itself
/// the moment one changed.
fn outgoing(item: &Planned, id: NativeId, project: Option<NativeId>, origin: &Origin) -> Item {
    match &item.item {
        Item::Task(task) => Item::Task(Box::new(Task {
            id,
            url: None,
            location: None,
            created_at: None,
            updated_at: None,
            project,
            metadata: carried(&task.metadata, origin),
            ..(**task).clone()
        })),
        Item::Project(project) => Item::Project(Box::new(Project {
            id,
            url: None,
            location: None,
            created_at: None,
            updated_at: None,
            metadata: carried(&project.metadata, origin),
            ..(**project).clone()
        })),
        Item::Document(document) => Item::Document(Box::new(Document {
            id,
            url: None,
            location: None,
            created_at: None,
            updated_at: None,
            project,
            metadata: carried(&document.metadata, origin),
            ..(**document).clone()
        })),
    }
}

/// The metadata a copy carries: the caller's own keys untouched, and the origin settled.
///
/// The key is removed before it is settled rather than overwritten, because the item being
/// copied carries an origin of its own and [`Origin::Keeps`] must not let it through.
fn carried(metadata: &BTreeMap<String, Value>, origin: &Origin) -> BTreeMap<String, Value> {
    let mut carried = metadata.clone();
    carried.remove(Repository::METADATA_KEY);
    carried.remove(DependencyEdge::RECORDED_KEY);
    carried.remove(GlobalId::ORIGIN_KEY);
    let held = match origin {
        Origin::Records(id) => Some(Value::String(id.to_string())),
        Origin::Keeps(held) => held.clone(),
    };
    if let Some(held) = held {
        carried.insert(GlobalId::ORIGIN_KEY.to_owned(), held);
    }
    carried
}

/// What one landed item records at [`GlobalId::ORIGIN_KEY`].
enum Origin {
    /// The qualified id this item was copied from, as the id type rather than as its
    /// spelling: the key holds a [`GlobalId`] and nothing else may be recorded there.
    Records(GlobalId),
    /// Whatever the destination already holds there — `None` when it holds nothing, which
    /// is written as the key being absent rather than as a null.
    ///
    /// A [`Value`] and not a [`GlobalId`], because this variant does not interpret what it
    /// carries: it is the destination's own metadata entry, held for the length of one
    /// write and put back exactly as it was read. Parsing it would turn a value a
    /// destination holds and this engine cannot read into a value this engine deletes,
    /// which is the opposite of what keeping it means.
    Keeps(Option<Value>),
}

/// Which of the two a copy of this item does.
///
/// A copy that reached its target by rule 1 is a copy-back: the item being copied names
/// the destination item, so the destination is the *original* and the id being copied
/// belongs to the copy that came out of it. Recording that id there would overwrite the
/// original's own provenance — and with it the correspondence every later copy from the
/// source it was authored in depends on. That copy would then match nothing and create a
/// second item beside the one it meant to update, which is the whole failure: nothing is
/// reported, and whoever reads that board now has two. So a copy-back leaves the
/// destination's origin exactly as the destination holds it, absent included, and every
/// other copy records the id it was copied from.
fn recorded(item: &Planned, held: Option<&Prior>) -> Origin {
    if let Target::Update {
        found: Found::Origin,
        ..
    } = &item.target
    {
        return Origin::Keeps(
            held.and_then(|held| described(&held.item).1.get(GlobalId::ORIGIN_KEY).cloned()),
        );
    }
    Origin::Records(item.source.clone())
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
        // No status, because a document has none; no edges, because it is in no graph.
        (Item::Document(held), Item::Document(outgoing)) => {
            held.title == outgoing.title
                && held.content == outgoing.content
                && held.labels == outgoing.labels
                && held.project == outgoing.project
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
    kind: Level,
) -> Result<bool, EngineError> {
    let found = match kind {
        Level::Task => destination
            .source()
            .get_task(id)
            .await
            .map_err(|error| refused(destination, error))?
            .is_some(),
        Level::Project => destination
            .source()
            .get_project(id)
            .await
            .map_err(|error| refused(destination, error))?
            .is_some(),
        Level::Document => destination
            .source()
            .get_document(id)
            .await
            .map_err(|error| refused(destination, error))?
            .is_some(),
    };
    Ok(found)
}
