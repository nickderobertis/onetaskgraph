//! The two traits a plugin implements, and the secret lookup it is handed.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use schemars::{JsonSchema, Schema};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    Asset, AssetName, AssetWrite, AssetsWritten, Capabilities, Classification, Comment,
    CommentBody, CommentCopy, DependencyEdge, Direction, Document, DocumentQuery,
    DocumentVersionCoverage, IssueHolding, IssueRef, ItemKind, ItemWrite, Label, Location,
    MetadataKey, MetadataRecord, Metering, NativeId, NewComment, Page, PageRequest, Priority,
    Project, ProjectQuery, ProjectVersionCoverage, Repository, SharedClock, SourceError,
    SourceName, Status, StatusCategory, StatusContradiction, Task, TaskDetailRead, TaskQuery,
    TaskRef, TaskUpdate, TaskUpdateOutcome, VersionCoverage, Visibility, WriteJournal,
    WriteSupport, WriteTarget, assetless, commentless, documentless, unwritable, unwritable_field,
    unwritable_metadata,
};

/// Whether a source is answering right now.
///
/// # Placement is an open contract question
///
/// This type lives here because [`TaskSource::health`] returns it and the trait
/// lives here: placing it in `onetaskgraph-core` would make this crate depend on
/// the engine and invert the one direction the crate split exists to establish.
/// The approved contract enumerates this crate's contents exhaustively and does
/// not name `Health`, so the enumeration and the trait as written cannot both
/// stand. Compiling forces the placement below; the resolution — add it to the
/// enumeration, or redesign `health` so no such type crosses the boundary —
/// belongs to the contract's owner, not to this crate. See `AGENTS.md`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
// llmlint: ignore[invalid_states_unrepresentable] SECOND PERMITTED REASON — this restates at a new site the justification already recorded at `Capabilities.max_page_size` (capability.rs) and `PageRequest.limit` (query.rs), and a third time in this type's own doc comment above and in AGENTS.md's "Open contract question — `Health`": `Health`'s shape is approved contract text that `TaskSource::health` returns, so an enum here would change the serialized form and the trait six undispatched nodes implement. That is the contract owner's call, not this crate's.
pub struct Health {
    /// Whether the source answered.
    ///
    /// A bare `bool` beside an untyped `detail` cannot say that an unreachable source
    /// must explain itself, or keep "reachable with a warning" apart from "reachable";
    /// an enum carrying the detail in its unreachable variant would.
    // llmlint: ignore[invalid_states_unrepresentable] SECOND PERMITTED REASON — this
    // restates at this field the justification already recorded at
    // `Capabilities.max_page_size` (capability.rs), `PageRequest.limit` (query.rs), this
    // type's own doc comment above, and AGENTS.md's "Open contract question — `Health`":
    // `Health`'s shape is approved contract text that `TaskSource::health` returns, so an
    // enum here would change the serialized form and the trait six undispatched nodes
    // implement. That is the contract owner's call, not this crate's.
    // llmlint: ignore[boundary_inputs_validated] making "unreachable with no reason given" unrepresentable means an enum here, which changes the serialized form and the trait six undispatched nodes implement. Deferred to the contract's owner — AGENTS.md, "Open contract question — `Health`".
    pub reachable: bool,
    /// What the source said, when it said anything useful.
    pub detail: Option<String>,
}

/// The task query whose [`TaskQuery::origin_matches`] is the one statement of what "copied
/// from `origin`" means, so the by-origin defaults read a project's and a document's entry
/// exactly as a task's is read.
fn origin_query(origin: &str) -> TaskQuery {
    TaskQuery {
        origin: Some(origin.to_owned()),
        ..TaskQuery::default()
    }
}

/// Refuse a page of the listing a by-origin default walks that holds more rows than it was
/// asked for, in the words the engine refuses one in: a source may return fewer and never
/// more, and a default that narrowed such a page would hide the defect behind it.
fn overran(returned: usize, asked_for: u32) -> Result<(), SourceError> {
    if returned as u64 <= u64::from(asked_for) {
        return Ok(());
    }
    Err(SourceError::Malformed {
        message: format!(
            "the source returned {returned} rows for a page of at most {asked_for}; a source \
             may return fewer than it was asked for and never more"
        ),
    })
}

/// What a by-origin default answers when the listing it walks hands back a cursor it has
/// already handed out, which would walk forever.
fn repeated_cursor() -> SourceError {
    SourceError::Malformed {
        message: "the source returned a cursor it had already handed out while its listing \
                  was walked for one origin, so the walk would never end"
            .to_owned(),
    }
}

/// The page `page` asks for of `matched`, every item a by-origin default kept: the cursor
/// it hands out is the position of the next match, so a walk of its pages neither repeats
/// nor drops one while the listing under it holds still.
fn page_of_matches<T>(matched: Vec<T>, page: &PageRequest) -> Result<Page<T>, SourceError> {
    let start = match &page.cursor {
        None => 0,
        Some(cursor) => cursor
            .0
            .strip_prefix(MATCHED_PAGE_CURSOR)
            .and_then(|position| position.parse::<usize>().ok())
            .filter(|position| *position <= matched.len())
            .ok_or_else(|| SourceError::Refused {
                message: format!(
                    "cursor {:?} was not handed out by this by-origin listing; start the walk \
                     again without one",
                    cursor.0
                ),
            })?,
    };
    let end = start.saturating_add(page.limit as usize).min(matched.len());
    let next = (end < matched.len()).then(|| crate::Cursor(format!("{MATCHED_PAGE_CURSOR}{end}")));
    Ok(Page {
        items: matched.into_iter().skip(start).take(end - start).collect(),
        next,
    })
}

const MATCHED_PAGE_CURSOR: &str = "origin:";

/// One configured source, as the engine drives it.
///
/// Dyn-compatible through `async_trait` because the engine holds
/// `Vec<Box<dyn TaskSource>>` over heterogeneous plugins.
///
/// Three rules bind every implementation, and the engine's compensation is only
/// correct while all three hold:
///
/// 1. **Apply** every predicate you declare [`Support::Native`](crate::Support::Native).
/// 2. **Ignore** every [`Support`](crate::Support)-typed predicate you declare
///    `Unsupported` — return the *wider* result set, never a narrower one.
///    Silently dropping rows for a predicate you did not declare is the one
///    failure no test above the plugin can catch.
/// 3. Never return a silently empty dependency read. Rule 2 reaches the
///    `Support`-typed *predicates* alone; a dependency read is always real, and so is a
///    document read — [`Capabilities::documents`] says whether this source has documents
///    at all, and a source that says it has none is never asked for one rather than
///    answering an empty page.
#[async_trait::async_trait]
pub trait TaskSource: Send + Sync {
    /// The plugin kind that built this source, for display and for plan output.
    fn kind(&self) -> &'static str;

    /// What this source applies itself. Read once per query by the engine.
    fn capabilities(&self) -> Capabilities;

    /// How `a`, a [`Task::version`] this source gave one of its tasks, stands to `b`, another
    /// it gave the same task: `Greater` when `a` is the later, `Less` when it is the earlier,
    /// `Equal` when nothing the token covers changed between them, and `None` when the two
    /// cannot be put in order — a value this source cannot read, or two versions its token
    /// says differ without saying which came first.
    ///
    /// A token's format and its order are the one source's that spells it, so the engine asks
    /// the source that gave both rather than reading inside either.
    ///
    /// Defaulted to equality alone: two identical tokens are one version, and any two others
    /// are unordered, which a reader takes as *changed, read it again* and never as
    /// unchanged. A source whose token says which of two versions is the later overrides it.
    fn order_versions(&self, a: &str, b: &str) -> Option<Ordering> {
        (a == b).then_some(Ordering::Equal)
    }

    /// Which parts of a task this source's [`Task::version`] vouches for, which is what the
    /// store's local cache may serve a task's parts from.
    ///
    /// Defaulted to no part, which keeps the cache from ever serving what a token was never
    /// promised to cover: a source overrides it only for the parts its token covers, as
    /// `docs/plugin-protocol.md` §4.13d states them.
    fn version_coverage(&self) -> VersionCoverage {
        VersionCoverage::none(format!(
            "the {} plugin does not declare that its change token covers a task",
            self.kind()
        ))
    }

    /// Which parts of a task a copy may take this source's unchanged [`Task::version`] as
    /// vouching for, when it decides to skip a task its destination already holds as written:
    /// the task's fields, its comments when the copy carries them, and its forward edges. A copy
    /// skips only a task whose source covers every one of those it writes.
    ///
    /// Defaulted to [`version_coverage`](Self::version_coverage). A source overrides it only to
    /// state, with its measured bound, a claim about a copy's skip that is not one the cache may
    /// serve from — as `docs/plugin-protocol.md` §4.13d does for the one plugin that does.
    fn copy_version_coverage(&self) -> VersionCoverage {
        self.version_coverage()
    }

    /// Which parts of a project this source's [`Project::version`] vouches for, on the terms
    /// of [`version_coverage`](Self::version_coverage).
    ///
    /// Defaulted to no part, for the same reason.
    fn project_version_coverage(&self) -> ProjectVersionCoverage {
        ProjectVersionCoverage::none(format!(
            "the {} plugin does not declare that its change token covers a project",
            self.kind()
        ))
    }

    /// Which parts of a document this source's [`Document::version`] vouches for, on the terms
    /// of [`version_coverage`](Self::version_coverage).
    ///
    /// Defaulted to no part, for the same reason.
    fn document_version_coverage(&self) -> DocumentVersionCoverage {
        DocumentVersionCoverage::none(format!(
            "the {} plugin does not declare that its change token covers a document",
            self.kind()
        ))
    }

    /// Tell this source the store's local cache now stands in front of it.
    ///
    /// Called exactly once, by the cache as it wraps the source, before any read and before
    /// any coverage is read — so a source whose token needs a part only a wider selection
    /// carries can widen its reads here, and declare the coverage that widening buys. Never
    /// carried over the plugin protocol, as [`version_coverage`](Self::version_coverage) is
    /// not.
    ///
    /// Defaulted to doing nothing: a source not overriding it reads exactly as it did.
    fn attach_cache(&self) {}

    /// The change token the response that answered this source's last
    /// [`Direction::DependsOn`] read of task `id` in this process spelled, or `None` when it
    /// spelled none.
    ///
    /// What lets the store's cache keep a task's forward edges by the version they were read
    /// at when nothing else in the command read the task: those edges are served only to a pin
    /// that version is at or after, and never vouch for the task's own fields. Synchronous, and
    /// never carried over the plugin protocol, as [`version_coverage`](Self::version_coverage)
    /// is not.
    ///
    /// Defaulted to `None`: a source whose dependency read spells no token keeps its edges
    /// remote unless a read of the task itself vouched for them.
    fn forward_edges_version(&self, id: &NativeId) -> Option<String> {
        let _ = id;
        None
    }

    /// The change token the response that answered this source's last dependency read of the
    /// `kind` item `id` in `direction`, from its first page, spelled — or `None` when it spelled
    /// none — on the terms of [`forward_edges_version`](Self::forward_edges_version), which is
    /// what it answers for a task's [`Direction::DependsOn`].
    ///
    /// What lets the store's cache keep a project's forward edges, and either kind's reverse
    /// ones, by the version they were read at: a project's forward edges are served only where
    /// [`project_version_coverage`](Self::project_version_coverage) covers them, reverse ones only
    /// where [`reverse_edges_coverage`](Self::reverse_edges_coverage) covers that kind's, and each
    /// only to a pin that version is at or after. Synchronous, and never
    /// carried over the plugin protocol.
    ///
    /// Defaulted to [`forward_edges_version`](Self::forward_edges_version) for a task's forward
    /// edges and to `None` for every other read, which keeps those remote.
    fn edges_version(&self, kind: ItemKind, id: &NativeId, direction: Direction) -> Option<String> {
        match (kind, direction) {
            (ItemKind::Task, Direction::DependsOn) => self.forward_edges_version(id),
            _ => None,
        }
    }

    /// Whether this source's token for a `kind` item — [`Task::version`] or
    /// [`Project::version`] — vouches for its reverse edges, what a [`Direction::DependedOnBy`]
    /// read answers, so the store's cache may serve them.
    ///
    /// A reverse edge is recorded on another item, so a token vouches for one only where it
    /// carries a part that moves whenever one is added to or removed from that other item —
    /// which no part of an item's own fields does. Never carried over the plugin protocol.
    ///
    /// Defaulted to not covering them, with that reason.
    fn reverse_edges_coverage(&self, kind: ItemKind) -> crate::Coverage {
        crate::Coverage::Uncovered {
            reason: format!(
                "the {} plugin does not declare that its change token covers a {}'s reverse \
                 edges, which are recorded on the items that depend on it",
                self.kind(),
                match kind {
                    ItemKind::Task => "task",
                    ItemKind::Project => "project",
                }
            ),
        }
    }

    /// What this source holds about task `id` — the item it read last in this process — that
    /// a later write expecting that version of it could take instead of reading it again, or
    /// `None` when it holds nothing of the kind.
    ///
    /// Opaque to the engine, which keeps it beside the cached task and hands it back through
    /// [`prime_write`](Self::prime_write). Defaulted to `None`: a source whose writes read
    /// nothing first has nothing to keep.
    fn write_context(&self, id: &NativeId) -> Option<serde_json::Value> {
        let _ = id;
        None
    }

    /// What this source holds about the container every task of it shares — a board's field
    /// and option ids — that a write could take instead of reading it, or `None`.
    ///
    /// Defaulted to `None`, on the terms of [`write_context`](Self::write_context).
    fn board_context(&self) -> Option<serde_json::Value> {
        None
    }

    /// Hold `context`, which [`write_context`](Self::write_context) gave for task `id` at a
    /// version the caller expects it still at, and `board`, which
    /// [`board_context`](Self::board_context) gave, so the next write of `id` takes them
    /// instead of reading — answering whether it took them.
    ///
    /// A context this source cannot read is not taken, and the write then reads as it always
    /// does. Defaulted to taking nothing.
    fn prime_write(
        &self,
        id: &NativeId,
        context: &serde_json::Value,
        board: Option<&serde_json::Value>,
    ) -> bool {
        let _ = (id, context, board);
        false
    }

    /// What this source read in this process about where it creates — a container's field
    /// and option ids, the node ids of the repositories it files in — that a create in a
    /// later process could take instead of reading it, or `None`.
    ///
    /// Immutable placement knowledge only: it holds no item's title, content or membership,
    /// and nothing that authorizes a versioned write. Defaulted to `None`.
    fn placement_context(&self) -> Option<serde_json::Value> {
        None
    }

    /// Take `context`, which [`placement_context`](Self::placement_context) gave in an
    /// earlier process, so the next create reads none of it — answering whether it took it.
    /// A context this source cannot read is not taken. Defaulted to taking nothing.
    fn prime_placement(&self, context: &serde_json::Value) -> bool {
        let _ = context;
        false
    }

    /// The ids — never the title or the content — a later process needs to write task `id`,
    /// which this process wrote, without reading it first, or `None`. Defaulted to `None`.
    fn written_context(&self, id: &NativeId) -> Option<serde_json::Value> {
        let _ = id;
        None
    }

    /// Take `task`, which the caller asserts this source holds exactly as it last wrote it,
    /// with `context`, which [`written_context`](Self::written_context) gave, as the record
    /// of `id` — so the next [`update_task`](Self::update_task) of it sends only its
    /// mutations. Answers whether it took them; a context it cannot read is not taken, and
    /// the update then reads as it always does. Defaulted to taking nothing.
    fn prime_written(
        &self,
        id: &NativeId,
        context: &serde_json::Value,
        task: &crate::Task,
    ) -> bool {
        let _ = (id, context, task);
        false
    }

    /// Drop whatever [`prime_write`](Self::prime_write) took for `id`, and the board context
    /// with it, so the next write of it reads afresh.
    fn unprime(&self, id: &NativeId) {
        let _ = id;
    }

    /// Start recording every request this source sends, for [`end_journal`](Self::end_journal).
    ///
    /// The engine opens one journal around one write at a time. Defaulted to recording
    /// nothing.
    fn begin_journal(&self) {}

    /// Stop recording, answering with every request sent since
    /// [`begin_journal`](Self::begin_journal) — or `None` for a source that records none.
    fn end_journal(&self) -> Option<WriteJournal> {
        None
    }

    /// Whether the source is answering right now.
    ///
    /// # Errors
    ///
    /// Returns a [`SourceError`] when the check itself could not be made.
    async fn health(&self) -> Result<Health, SourceError>;

    /// The [`Designator`](crate::Designator) naming the tracker this source reads, built by
    /// [`Designator::of_backend`](crate::Designator::of_backend) over the backend's own opaque
    /// identifier of itself — or `None` for a backend with none, whose configuration may
    /// declare a `public_alias` instead.
    ///
    /// At most one request. The engine keeps the answer per host, so a source is asked once.
    /// The default is `None`.
    ///
    /// # Errors
    ///
    /// Returns a [`SourceError`] when the backend could not be asked.
    async fn designator(&self) -> Result<Option<crate::Designator>, SourceError> {
        Ok(None)
    }

    /// Fetch one task by its native id, or `None` when there is no such task.
    ///
    /// # Errors
    ///
    /// Returns a [`SourceError`] when the source could not answer.
    async fn get_task(&self, id: &NativeId) -> Result<Option<Task>, SourceError>;

    /// Fetch several tasks by their native ids, each with the first page of its comments
    /// when `comments` names the page to read — one answer per id, in the order they were
    /// given.
    ///
    /// Each answer is what [`get_task`](Self::get_task) answers for that id, so a task that
    /// is not there is `Ok(None)` and a read that failed fails that id alone. Its
    /// [`TaskDetailRead::comments`] is what [`task_comments`](Self::task_comments) answers
    /// for `comments`. The engine passes `comments` only to a source declaring
    /// [`Capabilities::comments`](crate::Capabilities::comments) native.
    ///
    /// Defaulted to exactly those calls, item by item, so a source that implements nothing
    /// here answers as it always did. A source that can read many items — or an item and its
    /// comments — in fewer requests than that overrides it, which is the whole reason it
    /// exists: a caller holding twenty ids pays for one batch rather than forty reads.
    async fn get_task_details(
        &self,
        ids: &[NativeId],
        comments: Option<&PageRequest>,
    ) -> Vec<Result<Option<TaskDetailRead>, SourceError>> {
        let mut read = Vec::with_capacity(ids.len());
        for id in ids {
            read.push(match self.get_task(id).await {
                Ok(Some(task)) => {
                    let comments = match comments {
                        Some(page) => Some(self.task_comments(id, page).await),
                        None => None,
                    };
                    Ok(Some(TaskDetailRead { task, comments }))
                }
                Ok(None) => Ok(None),
                Err(error) => Err(error),
            });
        }
        read
    }

    /// Fetch one project by its native id, or `None` when there is no such project.
    ///
    /// # Errors
    ///
    /// Returns a [`SourceError`] when the source could not answer.
    async fn get_project(&self, id: &NativeId) -> Result<Option<Project>, SourceError>;

    /// One page of the tasks matching `query`.
    ///
    /// # Errors
    ///
    /// Returns a [`SourceError`] when the source could not answer.
    async fn query_tasks(
        &self,
        query: &TaskQuery,
        page: &PageRequest,
    ) -> Result<Page<Task>, SourceError>;

    /// One page of the projects matching `query`.
    ///
    /// # Errors
    ///
    /// Returns a [`SourceError`] when the source could not answer.
    async fn query_projects(
        &self,
        query: &ProjectQuery,
        page: &PageRequest,
    ) -> Result<Page<Project>, SourceError>;

    /// One page of the projects whose [`TaskQuery::ORIGIN_KEY`] entry is a string equal to
    /// `origin`, exactly — the projects copied from that qualified id, and none other.
    ///
    /// `origin` is compared byte for byte, on the terms [`TaskQuery::origin`] states, and is
    /// never constructed or interpreted here. Zero, one or several projects may match; which
    /// of several a caller wants is the caller's question.
    ///
    /// Defaulted to [`query_projects`](Self::query_projects) unfiltered, walked to its end on
    /// every call and narrowed here, so a source that implements nothing answers exactly what
    /// its own listing holds; the default's cursor is a position among the matches. A source
    /// that can ask its store for one origin overrides it, which is the whole reason it
    /// exists: a caller looking for the copies of one plan pays for those copies rather than
    /// for every project the store holds.
    ///
    /// # Errors
    ///
    /// Returns a [`SourceError`] when the source could not answer, and
    /// [`SourceError::Refused`] for a cursor the default did not hand out.
    async fn query_projects_by_origin(
        &self,
        origin: &str,
        page: &PageRequest,
    ) -> Result<Page<Project>, SourceError> {
        let wanted = origin_query(origin);
        let mut matched = Vec::new();
        let mut cursor: Option<crate::Cursor> = None;
        // Every cursor the listing handed out, so one cycling through several is caught as
        // one repeating the last is.
        let mut visited: Vec<String> = Vec::new();
        loop {
            let listed = self
                .query_projects(
                    &ProjectQuery::default(),
                    &PageRequest {
                        cursor: cursor.clone(),
                        limit: page.limit,
                    },
                )
                .await?;
            overran(listed.items.len(), page.limit)?;
            matched.extend(
                listed
                    .items
                    .into_iter()
                    .filter(|project| wanted.origin_matches(&project.metadata)),
            );
            cursor = match listed.next {
                None => break,
                Some(next) if visited.contains(&next.0) => return Err(repeated_cursor()),
                Some(next) => {
                    visited.push(next.0.clone());
                    Some(next)
                }
            };
        }
        page_of_matches(matched, page)
    }

    /// One page of every label this source knows.
    ///
    /// # Errors
    ///
    /// Returns a [`SourceError`] when the source could not answer.
    async fn labels(&self, page: &PageRequest) -> Result<Page<Label>, SourceError>;

    /// One page of the task dependency edges at `id`, in `direction`.
    ///
    /// # Errors
    ///
    /// Returns a [`SourceError`] when the source could not answer.
    async fn task_dependencies(
        &self,
        id: &NativeId,
        direction: Direction,
        page: &PageRequest,
    ) -> Result<Page<DependencyEdge>, SourceError>;

    /// One page of the project dependency edges at `id`, in `direction`.
    ///
    /// # Errors
    ///
    /// Returns a [`SourceError`] when the source could not answer.
    async fn project_dependencies(
        &self,
        id: &NativeId,
        direction: Direction,
        page: &PageRequest,
    ) -> Result<Page<DependencyEdge>, SourceError>;

    /// Whether this source can be written through at all.
    ///
    /// Defaulted to [`WriteSupport::Unsupported`], which is what keeps this a read
    /// interface for every source that has nothing to write into: one that cannot be
    /// written needs no edit and keeps working. Read before a write is attempted, so a
    /// copy naming such a source as its destination is refused before anything is read.
    fn writes(&self) -> WriteSupport {
        WriteSupport::Unsupported
    }

    /// The status this source's backend holds once an item of `kind` is written at `status`:
    /// the name it records for the category, where it names a category itself rather than
    /// keeping the name it was given.
    ///
    /// What a copy compares a destination item's status with, so an item whose category it
    /// already holds reads as unchanged rather than as differing by a name no write could
    /// change. Defaulted to `status` as given: a backend that keeps the name it is written
    /// declares nothing.
    ///
    /// Asynchronous and fallible because a source hosted over the stdio plugin protocol asks
    /// the plugin that declares it; an in-process source answers at once and never fails.
    ///
    /// # Errors
    ///
    /// A [`SourceError`] when the source could not say what its backend holds — never the
    /// status as given in its place.
    async fn stored_status(&self, kind: ItemKind, status: &Status) -> Result<Status, SourceError> {
        let _ = kind;
        Ok(status.clone())
    }

    /// The content the backend answered it now holds for the `record` `id`, as the response
    /// to this instance's latest write of that item's content reported it — taken, so a second
    /// call answers `None` until the next such write.
    ///
    /// The content is as a read reports content — the metadata slot, and whatever separates
    /// it from the content, removed — and it is read off the write's own response, never by a
    /// request of its own. It is what the engine takes as the record of what the item holds: a
    /// backend may rewrite the Markdown it is written, keeping its rendered meaning, and where
    /// this differs from what was sent the engine records each fingerprint it keeps over this
    /// content instead — a rendering's `body_digest`, a copy's
    /// [`MetadataKey::COPIED_DESTINATION_VERSION_KEY`] — and writes it back once, beside them,
    /// as its own request. That follow-up write's own answer is then compared with this
    /// content byte for byte, and a backend that does not hold its own stored content stable
    /// is refused as `stored-content-diverged`. Defaulted to `None`: a backend that stores what
    /// it is given says nothing, and nothing is compared or written again.
    fn stored_content(&self, record: MetadataRecord, id: &NativeId) -> Option<String> {
        let _ = (record, id);
        None
    }

    /// Create or update one task, answering with the native id the destination holds it
    /// under.
    ///
    /// A source declaring [`WriteSupport::Supported`] owes three things here. It refuses,
    /// naming the field, anything it cannot represent rather than dropping it — including
    /// a metadata key it cannot carry, which it names. It writes every other field it was
    /// given. And it never creates when [`ItemWrite::target`] names an item it does not
    /// hold.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::Refused`] when this source has no write side, when a field
    /// or a metadata key cannot be represented, or when `target` names nothing here; and
    /// whatever else the source could not do the write for.
    async fn write_task(&self, write: &ItemWrite<Task>) -> Result<NativeId, SourceError> {
        let _ = write;
        Err(unwritable(self.kind()))
    }

    /// Create or update one project, on exactly the terms of
    /// [`write_task`](Self::write_task).
    ///
    /// # Errors
    ///
    /// As [`write_task`](Self::write_task).
    async fn write_project(&self, write: &ItemWrite<Project>) -> Result<NativeId, SourceError> {
        let _ = write;
        Err(unwritable(self.kind()))
    }

    /// Whether this source can represent every field of `write` — its status, its content, the
    /// tasks it delivers and every other field a [`write_task`](Self::write_task) of it would
    /// refuse as "cannot represent the field …" — asked before anything is written and changing
    /// nothing.
    ///
    /// A task copy asks it of every task it plans before its first write, and leaves out each
    /// task this refuses — answered `failed`, with this refusal as its reason — writing the rest
    /// as one copy. It answers what the write itself would refuse in the same words; and it is a
    /// promise only about representability, so a write may still fail for any other reason.
    ///
    /// Defaulted to `Ok(())`: a source that does not answer it in advance still refuses such a
    /// write in its write, which a copy then takes back whole, exactly as before.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::Refused`] naming the field this source cannot represent; and
    /// whatever else kept it from finding out.
    async fn check_task_write(&self, write: &ItemWrite<Task>) -> Result<(), SourceError> {
        let _ = write;
        Ok(())
    }

    /// Whether a write of an item of `kind` at `category` — over the item `target` names, or
    /// creating one when it is `None` — would have a status to write, asked before the write
    /// and changing nothing.
    ///
    /// A caller that has to put an item back when a later write of it fails — a copy, whose
    /// journal records what an item held before overwriting it — asks this first, so a status
    /// this source has no name for is refused while nothing has been recorded or written, and
    /// nothing has to be put back. It answers what the write itself would refuse a status
    /// with, in the same words, and sends no request a write of that status would not have
    /// sent anyway: what it reads to answer, it holds for the write that follows.
    ///
    /// Defaulted to `Ok(())`: a source that does not answer it in advance still refuses the
    /// status in its write, exactly as before.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::Refused`] naming the kind, the category and what is missing
    /// when this source has no name for that status; and whatever else kept it from finding
    /// out.
    async fn check_status_write(
        &self,
        kind: ItemKind,
        category: StatusCategory,
        target: Option<&NativeId>,
    ) -> Result<(), SourceError> {
        let _ = (kind, category, target);
        Ok(())
    }

    /// The status an item of `kind` reads back as once this source has written `status` for
    /// it, asked without writing or reading anything.
    ///
    /// A source that keeps a status's name as it was given answers `status` itself. One that
    /// writes a category to a name of its own — a board's `Status` option the category maps
    /// to — answers that name, so a caller comparing what it would write with what the source
    /// holds does not read a difference of spelling alone as a change: a copy then leaves an
    /// item that already reads as the source does unwritten.
    ///
    /// Defaulted to `status` itself, which is what every source keeping names verbatim reads
    /// back, and what a caller compared against before this was asked.
    fn status_as_written(&self, kind: ItemKind, status: &Status) -> Status {
        let _ = kind;
        status.clone()
    }

    /// Set the status of one task this source holds, and change nothing else about it,
    /// answering with the status as this source now reads it — or `None` when this source
    /// holds no such task.
    ///
    /// The category lands where this source's own mapping sends it, exactly as a
    /// [`write_task`](Self::write_task) of a task in that category would: a category this
    /// source has disabled is refused in the words a write of it is refused with. Title,
    /// content, labels, metadata, dependencies, [`Task::delivers`], [`Task::delivered_by`],
    /// project and comments are left exactly as they are.
    ///
    /// Defaulted to [`unwritable_field`], which is what keeps this an addition rather than a
    /// break: a source that cannot write a status on its own needs no edit and refuses by
    /// saying so. A source declaring [`WriteSupport::Unsupported`] is never asked.
    ///
    /// [`Task::delivers`]: crate::Task::delivers
    /// [`Task::delivered_by`]: crate::Task::delivered_by
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::Refused`] when this source cannot write a status, or cannot
    /// write this one; and whatever else the source could not do the write for.
    async fn set_task_status(
        &self,
        id: &NativeId,
        category: StatusCategory,
    ) -> Result<Option<Status>, SourceError> {
        let _ = (id, category);
        Err(unwritable_field(self.kind(), "status"))
    }

    /// Set the status of one project this source holds, and change nothing else about it,
    /// answering with the status as this source now reads it — or `None` when this source
    /// holds no such project.
    ///
    /// The category lands where this source's own mapping sends it for a project, exactly as a
    /// [`write_project`](Self::write_project) of a project in that category would: a category
    /// this source has disabled is refused in the words a write of it is refused with. Title,
    /// content, labels, metadata, repositories, dependencies and the project's tasks are left
    /// exactly as they are. A project delivers no task, so nothing is kept in step after it.
    ///
    /// Defaulted to the refusal [`set_task_status`](Self::set_task_status) defaults to,
    /// [`unwritable_field`] naming `status`, which is what keeps this an addition rather than a
    /// break: a source that does not implement it needs no edit. A source declaring
    /// [`WriteSupport::Unsupported`] is never asked.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::Refused`] when this source cannot write a project's status, or
    /// cannot write this one; and whatever else the source could not do the write for.
    async fn set_project_status(
        &self,
        id: &NativeId,
        category: StatusCategory,
    ) -> Result<Option<Status>, SourceError> {
        let _ = (id, category);
        Err(unwritable_field(self.kind(), "status"))
    }

    /// Set the status of one task this source holds, as [`set_task_status`](Self::set_task_status)
    /// does, and answer with the whole task as this source now reads it — its
    /// [`Task::delivers`] among what a caller keeping delivered tasks in step needs — or `None`
    /// when this source holds no such task.
    ///
    /// Defaulted to exactly the two calls a caller would otherwise make: [`get_task`] first,
    /// answering `None` with nothing written when it does, then `set_task_status`, the status
    /// it answers put on the task read. A source whose status write answers the whole task in
    /// the same round trip overrides it to save the read.
    ///
    /// [`Task::delivers`]: crate::Task::delivers
    /// [`get_task`]: Self::get_task
    ///
    /// # Errors
    ///
    /// As [`get_task`](Self::get_task) and [`set_task_status`](Self::set_task_status).
    async fn set_task_status_reading(
        &self,
        id: &NativeId,
        category: StatusCategory,
    ) -> Result<Option<Task>, SourceError> {
        let Some(mut task) = self.get_task(id).await? else {
            return Ok(None);
        };
        let Some(status) = self.set_task_status(id, category).await? else {
            return Ok(None);
        };
        task.status = status;
        Ok(Some(task))
    }

    /// Every task this source holds whose card contradicts its issue's closure, each with the
    /// status that brings it in line — what `task status reconcile` writes, through
    /// [`set_task_status`](Self::set_task_status), one task at a time.
    ///
    /// A task needs it when its issue is closed with a reason that reads `done` or
    /// `cancelled` and its card does not sit at the option this source's mapping gives that
    /// category. An open task, a closure that reads `unknown` and a card already in line are
    /// never answered. Changes nothing.
    ///
    /// Asked only of a source declaring
    /// [`Capabilities::reconcile_status`](crate::Capabilities::reconcile_status); defaulted to a
    /// refusal saying so, which no caller holding to that declaration ever receives.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::Refused`] when this source answers no such question; and whatever
    /// kept it from reading its items.
    async fn status_contradictions(&self) -> Result<Vec<StatusContradiction>, SourceError> {
        Err(SourceError::Refused {
            message: format!(
                "a {} source's status is its closure, so nothing needs reconciling",
                self.kind()
            ),
        })
    }

    /// For each issue in `issues`, in order and once each, the task this source holds for it
    /// and what moving it to `done` comes to — what `task status complete` writes, through
    /// [`set_task_status`](Self::set_task_status), for each one answered
    /// [`Completion::Due`](crate::Completion::Due).
    ///
    /// The read is proportional to `issues` and never to the size of what this source holds,
    /// and an empty `issues` reads nothing. Changes nothing.
    ///
    /// Asked only of a source declaring
    /// [`Capabilities::complete_by_issue`](crate::Capabilities::complete_by_issue); defaulted
    /// to a refusal saying so, which no caller holding to that declaration ever receives.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::Refused`] when this source answers no such question; and whatever
    /// kept it from reading the issues.
    async fn issue_holdings(&self, issues: &[IssueRef]) -> Result<Vec<IssueHolding>, SourceError> {
        let _ = issues;
        Err(SourceError::Refused {
            message: format!("a {} source holds no items by issue reference", self.kind()),
        })
    }

    /// Set the priority of one task this source holds, and change nothing else about it,
    /// answering with the priority as this source now reads it — or `None` when this source
    /// holds no such task.
    ///
    /// [`Priority::None`] clears the priority. Title, content, status, labels, metadata,
    /// repositories, dependencies and comments are left exactly as they are. Nothing about
    /// the task's status moves, so the engine re-evaluates no delivered task after it.
    ///
    /// Defaulted to [`unwritable_field`], which is what keeps this an addition rather than a
    /// break. A source declaring [`WriteSupport::Unsupported`], or declaring
    /// [`Capabilities::priority`] unsupported, is never asked.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::Refused`] when this source cannot write a priority, or cannot
    /// write this one — a board with no option for it, naming the option; and whatever else
    /// the source could not do the write for.
    async fn set_task_priority(
        &self,
        id: &NativeId,
        priority: Priority,
    ) -> Result<Option<Priority>, SourceError> {
        let _ = (id, priority);
        Err(unwritable_field(self.kind(), "priority"))
    }

    /// Replace the content of one task this source holds with `content`, byte for byte, and
    /// change nothing else about it — or answer `None` when this source holds no such task.
    ///
    /// The content is [`Task::content`] exactly as this source reports it: what a later read
    /// answers there is `content`. Where the source keeps something else inside the same
    /// backend field — a metadata block in an issue body — that is kept as it was, and so is
    /// every other member: title, status, priority, labels, metadata, repositories,
    /// dependencies, project and comments. Nothing about the task's status moves, so the
    /// engine re-evaluates no delivered task after it.
    ///
    /// Defaulted to [`unwritable_field`] on exactly the terms of
    /// [`set_task_status`](Self::set_task_status). A source declaring
    /// [`WriteSupport::Unsupported`] is never asked.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::Refused`] when this source cannot write a task's content on its
    /// own, or cannot represent this one; and whatever else the source could not do the write
    /// for.
    async fn set_task_content(
        &self,
        id: &NativeId,
        content: &str,
    ) -> Result<Option<()>, SourceError> {
        let _ = (id, content);
        Err(unwritable_field(self.kind(), "content"))
    }

    /// Replace the [`Task::delivered_by`] of one task this source holds, and change nothing
    /// else about it — or answer `None` when this source holds no such task.
    ///
    /// Every entry is a qualified id, and the list is the whole of it: what the task held
    /// there before is replaced, not merged. It is the store's to keep in step — the engine
    /// calls this whenever it writes a task's [`Task::delivers`] — and nothing a person types
    /// reaches it directly.
    ///
    /// Defaulted to [`unwritable_field`] on exactly the terms of
    /// [`set_task_status`](Self::set_task_status).
    ///
    /// [`Task::delivers`]: crate::Task::delivers
    /// [`Task::delivered_by`]: crate::Task::delivered_by
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::Refused`] when this source cannot hold the list, and whatever
    /// else it could not do the write for.
    async fn set_delivered_by(
        &self,
        id: &NativeId,
        delivered_by: &[TaskRef],
    ) -> Result<Option<()>, SourceError> {
        let _ = (id, delivered_by);
        Err(unwritable_field(self.kind(), "delivered_by"))
    }

    /// Set one key of the metadata of one task this source holds, and change nothing else
    /// about it, answering with the task as this source reads it back after the write — or
    /// `None` when this source holds no such task.
    ///
    /// The key is added when the task does not hold it and replaced when it does; every other
    /// metadata key, and every other field of the task, is left exactly as it was. A `value`
    /// the task already holds under `key` is a write that changes nothing, and a source owes
    /// it no write at all. The answer is a read, not an echo: what the engine reports as the
    /// value is what the returned task holds under `key`.
    ///
    /// Nothing about the task's status or its [`Task::delivers`] moves, so the engine
    /// re-evaluates no delivered task after this write.
    ///
    /// Defaulted to [`unwritable_metadata`] on exactly the terms of
    /// [`set_task_status`](Self::set_task_status): a source that cannot write one key on its
    /// own needs no edit and refuses by saying so. A source declaring
    /// [`WriteSupport::Unsupported`] is never asked.
    ///
    /// [`Task::delivers`]: crate::Task::delivers
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::Refused`] when this source cannot write one key of a task's
    /// metadata on its own, or cannot write this one without changing something else; and
    /// whatever else the source could not do the write for.
    async fn set_task_metadata(
        &self,
        id: &NativeId,
        key: &MetadataKey,
        value: &Value,
    ) -> Result<Option<Task>, SourceError> {
        let _ = (id, key, value);
        Err(unwritable_metadata(self.kind(), MetadataRecord::Task))
    }

    /// Apply a targeted update to one task this source holds — every field `update` names, and
    /// nothing else — answering with a [`TaskUpdateOutcome`], or `None` when this source holds
    /// no such task.
    ///
    /// The outcome is everything the engine reports about the call, so it reads nothing of its
    /// own: the task as this source reads it once the update landed — from the one read it made
    /// and what it then wrote, where its backend answers a write with what it holds — the
    /// fields it actually wrote, and the `delivers` the task held before.
    ///
    /// A field already holding the requested value is sent no write, and an update in which
    /// nothing differs sends no write at all. Every field `update` leaves unnamed — labels,
    /// repositories, project, [`Task::delivered_by`] and comments always among them — is left
    /// exactly as it is. An update naming one metadata key both to set and to remove is refused
    /// before anything is written, in the words of [`TaskUpdate::consistent`].
    ///
    /// Defaulted to [`TaskUpdate::rewrite`]: read the task, apply the update, and — only when
    /// anything differs — [`write_task`](Self::write_task) it with its `target` set. That keeps
    /// this an addition rather than a break, and a source that predates it behaves correctly and
    /// is merely not minimal; one whose backend can take a field on its own owes an override
    /// that sends only what differs. A source declaring [`WriteSupport::Unsupported`] is never
    /// asked.
    ///
    /// [`Task::delivered_by`]: crate::Task::delivered_by
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::Refused`] when the update contradicts itself, when a field it
    /// names cannot be represented here — a category this source has disabled included — and
    /// whatever else the source could not do the write for.
    async fn update_task(
        &self,
        id: &NativeId,
        update: &TaskUpdate,
    ) -> Result<Option<TaskUpdateOutcome>, SourceError> {
        update.rewrite(self, id).await
    }

    /// Set one key of the metadata of one project this source holds, on exactly the terms of
    /// [`set_task_metadata`](Self::set_task_metadata).
    ///
    /// # Errors
    ///
    /// As [`set_task_metadata`](Self::set_task_metadata).
    async fn set_project_metadata(
        &self,
        id: &NativeId,
        key: &MetadataKey,
        value: &Value,
    ) -> Result<Option<Project>, SourceError> {
        let _ = (id, key, value);
        Err(unwritable_metadata(self.kind(), MetadataRecord::Project))
    }

    /// Set one key of the metadata of one document this source holds, on exactly the terms of
    /// [`set_task_metadata`](Self::set_task_metadata).
    ///
    /// A source declaring [`Capabilities::documents`] unsupported is never asked, exactly as
    /// it is never asked for a document read.
    ///
    /// # Errors
    ///
    /// As [`set_task_metadata`](Self::set_task_metadata).
    async fn set_document_metadata(
        &self,
        id: &NativeId,
        key: &MetadataKey,
        value: &Value,
    ) -> Result<Option<Document>, SourceError> {
        let _ = (id, key, value);
        Err(unwritable_metadata(self.kind(), MetadataRecord::Document))
    }

    /// Remove one task this destination holds, so a copy that could not finish can put
    /// the destination back the way it found it.
    ///
    /// This is not a verb of the product: nothing a user types deletes anything, and a
    /// copy never deletes an item it did not itself create in the run that is failing.
    /// It exists because a copy is either complete or it never happened — a half-written
    /// project has to be run again, and the re-run is the mutation burst that trips a
    /// hosted destination's rate limiter. Undoing this run's own creates is what removes
    /// that retry at source.
    ///
    /// A source declaring [`WriteSupport::Supported`] owes a real implementation, for the
    /// reason it owes [`write_task`](Self::write_task) one: the engine will create items
    /// there, so it has to be able to remove the ones it created. An `id` naming nothing
    /// is **not** an error — the item is already gone, which is the state this asks for.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::Refused`] when this source has no write side, and whatever
    /// else the source could not remove the item for.
    async fn delete_task(&self, id: &NativeId) -> Result<(), SourceError> {
        let _ = id;
        Err(unwritable(self.kind()))
    }

    /// Remove one project this destination holds, on exactly the terms of
    /// [`delete_task`](Self::delete_task).
    ///
    /// # Errors
    ///
    /// As [`delete_task`](Self::delete_task).
    async fn delete_project(&self, id: &NativeId) -> Result<(), SourceError> {
        let _ = id;
        Err(unwritable(self.kind()))
    }

    /// Fetch one document by its native id, or `None` when there is no such document.
    ///
    /// Defaulted to [`documentless`], which is what keeps documents an addition rather
    /// than a break: a source with none needs no edit, keeps working, and says so in the
    /// same words every other document-free source does. A source that has documents
    /// declares [`Support::Native`](crate::Support::Native) for
    /// [`Capabilities::documents`] and owes a real implementation here, because that
    /// declaration is what makes the engine ask.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::Refused`] when this source has no documents, and whatever
    /// else the source could not answer for.
    async fn get_document(&self, id: &NativeId) -> Result<Option<Document>, SourceError> {
        let _ = id;
        Err(documentless(self.kind()))
    }

    /// One page of the documents matching `query`.
    ///
    /// Defaulted on exactly the terms of [`get_document`](Self::get_document). A source
    /// with no documents refuses rather than answering an empty page: an empty page reads
    /// as a source that has documents and holds none matching, which is the one wrong
    /// answer this method can give.
    ///
    /// # Errors
    ///
    /// As [`get_document`](Self::get_document).
    async fn query_documents(
        &self,
        query: &DocumentQuery,
        page: &PageRequest,
    ) -> Result<Page<Document>, SourceError> {
        let _ = (query, page);
        Err(documentless(self.kind()))
    }

    /// One page of the documents whose [`TaskQuery::ORIGIN_KEY`] entry is a string equal to
    /// `origin`, exactly, on the terms of
    /// [`query_projects_by_origin`](Self::query_projects_by_origin).
    ///
    /// Defaulted to [`query_documents`](Self::query_documents) unfiltered, walked and narrowed
    /// as that default is — so a source with no documents refuses exactly as its
    /// `query_documents` does.
    ///
    /// # Errors
    ///
    /// As [`query_documents`](Self::query_documents), and [`SourceError::Refused`] for a
    /// cursor the default did not hand out.
    async fn query_documents_by_origin(
        &self,
        origin: &str,
        page: &PageRequest,
    ) -> Result<Page<Document>, SourceError> {
        let wanted = origin_query(origin);
        let mut matched = Vec::new();
        let mut cursor: Option<crate::Cursor> = None;
        // Every cursor the listing handed out, so one cycling through several is caught as
        // one repeating the last is.
        let mut visited: Vec<String> = Vec::new();
        loop {
            let listed = self
                .query_documents(
                    &DocumentQuery::default(),
                    &PageRequest {
                        cursor: cursor.clone(),
                        limit: page.limit,
                    },
                )
                .await?;
            overran(listed.items.len(), page.limit)?;
            matched.extend(
                listed
                    .items
                    .into_iter()
                    .filter(|document| wanted.origin_matches(&document.metadata)),
            );
            cursor = match listed.next {
                None => break,
                Some(next) if visited.contains(&next.0) => return Err(repeated_cursor()),
                Some(next) => {
                    visited.push(next.0.clone());
                    Some(next)
                }
            };
        }
        page_of_matches(matched, page)
    }

    /// Create or update one document, on exactly the terms of
    /// [`write_task`](Self::write_task).
    ///
    /// # Errors
    ///
    /// As [`write_task`](Self::write_task).
    async fn write_document(&self, write: &ItemWrite<Document>) -> Result<NativeId, SourceError> {
        let _ = write;
        Err(unwritable(self.kind()))
    }

    /// Remove one document this destination holds, on exactly the terms of
    /// [`delete_task`](Self::delete_task).
    ///
    /// # Errors
    ///
    /// As [`delete_task`](Self::delete_task).
    async fn delete_document(&self, id: &NativeId) -> Result<(), SourceError> {
        let _ = id;
        Err(unwritable(self.kind()))
    }

    /// One page of the comments on `task`, oldest first, or `None` when this source holds
    /// no such task.
    ///
    /// Defaulted to [`commentless`], which is what keeps comments an addition rather than a
    /// break: a source with none needs no edit and keeps working. A source whose tasks have
    /// comments declares [`Support::Native`](crate::Support::Native) for
    /// [`Capabilities::comments`] and owes a real implementation of all four comment methods,
    /// because that declaration is what makes the engine ask.
    ///
    /// "No such task" is `None` rather than an error, exactly as it is for
    /// [`get_task`](Self::get_task); a task that exists and has no comments is an empty page.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::Refused`] when this source has no comments, and whatever else
    /// the source could not answer for.
    async fn task_comments(
        &self,
        task: &NativeId,
        page: &PageRequest,
    ) -> Result<Option<Page<Comment>>, SourceError> {
        let _ = (task, page);
        Err(commentless(self.kind()))
    }

    /// [`get_task_details`](Self::get_task_details) as a copy reads the tasks it carries: each
    /// with the first page of its comments `comments` names — and, where the same request can
    /// carry them, what this source needs to answer [`task_dependencies`](Self::task_dependencies)
    /// for it without another, since a copy reads every task's edges next.
    ///
    /// Defaulted to `get_task_details(ids, Some(comments))`, which is exactly what it answers.
    /// A source whose detail read is priced by what it selects overrides it with a read that
    /// also carries the edges, so a copy of a changed task costs its detail read and nothing
    /// more; one whose reads are free has no reason to.
    async fn get_task_details_for_copy(
        &self,
        ids: &[NativeId],
        comments: &PageRequest,
    ) -> Vec<Result<Option<TaskDetailRead>, SourceError>> {
        self.get_task_details(ids, Some(comments)).await
    }

    /// Read the items `ids` names — tasks, projects and documents alike, whatever kind each
    /// is — together, as a copy reads the counterparts it is about to write, and hold each for
    /// this command so that the next [`get_task`](Self::get_task),
    /// [`get_project`](Self::get_project) or [`get_document`](Self::get_document) of it is
    /// answered from that read, once, instead of being sent on its own.
    ///
    /// What is held is exactly what that read by id would have answered — the item's fields,
    /// its edges, its asset manifest and its change token — and only until the next read of
    /// it, a write naming it, or [`end_command`](Self::end_command), whichever comes first, so
    /// a read answered from it is never staler than the read-before-write it stands in for.
    ///
    /// Defaulted to doing nothing, so every read is sent as it always was: a source whose
    /// reads are free has no reason to override it, and one priced per request overrides it
    /// with a batched read. An id the batched read finds nothing at is either held as absent —
    /// its next read answers `None`, exactly as that read would have — or not held, and its own
    /// read answers as it always does.
    ///
    /// # Errors
    ///
    /// Returns a [`SourceError`] when the batched read could not be made. Nothing is then
    /// held, and each item's own read answers as it would have.
    async fn read_ahead_for_copy(&self, ids: &[NativeId]) -> Result<(), SourceError> {
        let _ = ids;
        Ok(())
    }

    /// Ask, together, which items were copied from each of `origins` — the qualified ids a copy
    /// is about to look for at this destination by the `onetaskgraph.origin` each item records
    /// — and hold each answer
    /// for this command, so that the next query for the items copied from one of them (a
    /// [`TaskQuery::origin`] task query, [`query_projects_by_origin`](Self::query_projects_by_origin)
    /// or [`query_documents_by_origin`](Self::query_documents_by_origin)) is answered from it,
    /// once, instead of being sent on its own. A source may carry in the same read whatever its
    /// creating an item would read first, since a copy finding nothing creates — and its
    /// [`designator`](Self::designator), which a copy writing edges asks next: a designator
    /// carried in that read answers the next [`designator`](Self::designator) of the command
    /// without a request of its own.
    ///
    /// What is held is exactly what those queries would have answered, and only until the next
    /// write or [`end_command`](Self::end_command); a designator, which no write moves, until
    /// [`end_command`](Self::end_command). An origin is held as having no counterpart only when
    /// the read answered it whole: one whose answer the read could not see the end of is left
    /// to its own query. Defaulted to doing nothing, so each query is sent as it always was.
    ///
    /// # Errors
    ///
    /// Returns a [`SourceError`] when the read could not be made. Nothing is then held, and each
    /// query answers as it would have.
    async fn read_ahead_origins_for_copy(&self, origins: &[String]) -> Result<(), SourceError> {
        let _ = origins;
        Ok(())
    }

    /// Who may read each member of the project `project` — every task and document filed under
    /// it — as the classification and repositories each one's record holds, read by whatever
    /// this source holds for this command rather than by its whole records: what a write the
    /// public boundary screens reads of a project before it is held to it.
    ///
    /// `None` is no such read, and is the default: the engine then reads the members as it
    /// always has, through [`query_tasks`](Self::query_tasks) narrowed to the project and
    /// [`query_documents`](Self::query_documents), so every source answers exactly as before. A
    /// source answers `Some` only with every member — a page that says there is more is no
    /// answer, and is left to that path — and never with fewer members than those queries
    /// would have listed. A source may hold such a page from a read it made beside another,
    /// such as the one [`read_ahead_for_copy`](Self::read_ahead_for_copy) names the project to.
    ///
    /// # Errors
    ///
    /// Returns a [`SourceError`] when the read could not be made.
    async fn member_classifications(
        &self,
        project: &NativeId,
    ) -> Result<Option<Vec<(Classification, Vec<Repository>)>>, SourceError> {
        let _ = project;
        Ok(None)
    }

    /// How many tasks this source creates in one [`write_tasks`](Self::write_tasks): more than
    /// one only for a source that creates several in one request.
    ///
    /// A copy hands such a source the tasks it creates together, and names each one's edges to
    /// the others in a write after they all exist. Defaulted to one, which has a copy write every
    /// task on its own, exactly as before.
    fn create_batch_limit(&self) -> usize {
        1
    }

    /// Create every task of `writes` — each [`ItemWrite::target`] `None` — answering with the
    /// native ids they were created under, in order.
    ///
    /// All of them or none: a source that cannot create every one takes back each it did create
    /// before refusing, on the terms §4.9 of the protocol puts on [`write_task`](Self::write_task).
    /// Defaulted to creating them one at a time through [`write_task`](Self::write_task), taking
    /// back those created when a later one is refused.
    ///
    /// # Errors
    ///
    /// Refused, before anything is written, when a write names an existing target; otherwise
    /// whatever [`write_task`](Self::write_task) of one of them refuses with.
    // llmlint: ignore[invalid_states_unrepresentable] Approved by the contract's owner as an additive method taking `ItemWrite<Task>` exactly as `write_task` does; a create-only input type would be a new contract type. A write naming an existing target is refused below before anything is written, so rollback only ever takes back what this call created.
    async fn write_tasks(&self, writes: &[ItemWrite<Task>]) -> Result<Vec<NativeId>, SourceError> {
        if let Some(write) = writes.iter().find(|write| write.target.is_some()) {
            return Err(SourceError::Refused {
                message: format!(
                    "write_tasks creates tasks, and the write of {} names an existing target; \
                     next: update it with write_task",
                    write.item.id
                ),
            });
        }
        let mut created = Vec::with_capacity(writes.len());
        for write in writes {
            match self.write_task(write).await {
                Ok(id) => created.push(id),
                Err(error) => {
                    // The caller is never handed these ids, so one this source will not take
                    // back is named here or by nobody.
                    let mut left: Vec<&str> = Vec::new();
                    for id in &created {
                        if self.delete_task(id).await.is_err() {
                            left.push(id.0.as_str());
                        }
                    }
                    if left.is_empty() {
                        return Err(error);
                    }
                    return Err(SourceError::Refused {
                        message: format!(
                            "{error}; this source then refused to take back {}, which this \
                             write created and which it still holds; next: delete {} before \
                             writing again",
                            left.join(", "),
                            if left.len() == 1 { "it" } else { "them" }
                        ),
                    });
                }
            }
        }
        Ok(created)
    }

    /// From here until [`finish_writes`](Self::finish_writes), a source may hold back the part of
    /// each write that can land later — a created item's fields and edges — and send it together
    /// with the others' when that is called.
    ///
    /// A caller asks for this only when it will call `finish_writes` before it reads what it
    /// wrote from anywhere but this source, or reports it written. Defaulted to holding nothing
    /// back, so every write lands whole as it is made.
    fn begin_deferred_writes(&self) {}

    /// Send whatever this source has held back since [`begin_deferred_writes`](Self::begin_deferred_writes),
    /// and hold nothing back after it.
    ///
    /// All or nothing for the items it created: a source that cannot finish them takes them back
    /// before refusing, so a caller undoing its writes finds them gone. Defaulted to doing
    /// nothing, since nothing was held.
    ///
    /// # Errors
    ///
    /// Whatever sending what was held back was refused with.
    async fn finish_writes(&self) -> Result<(), SourceError> {
        Ok(())
    }

    /// Where an item this source wrote in the current command is, when the write's own answer
    /// said so: `record` names which kind of item `id` is.
    ///
    /// Answered only from what a write already returned — never by a request — so a caller may
    /// ask it after every write for free. A caller treats `None` as not knowing, and reads the
    /// item when it needs its location. Defaulted to `None`.
    fn written_location(&self, record: MetadataRecord, id: &NativeId) -> Option<Location> {
        let _ = (record, id);
        None
    }

    /// Add one comment to `task`, answering with the comment as the source now holds it, or
    /// `None` when this source holds no such task.
    ///
    /// The body is stored byte for byte. A source that records the author itself refuses a
    /// [`NewComment::author`] rather than dropping it, naming why; a source that cannot
    /// represent the body refuses it, naming why, rather than escaping it into something
    /// else.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::Refused`] when this source has no comments or cannot be
    /// written, when it cannot record what it was given, and whatever else it could not do
    /// the write for.
    async fn add_comment(
        &self,
        task: &NativeId,
        comment: &NewComment,
    ) -> Result<Option<Comment>, SourceError> {
        let _ = (task, comment);
        Err(commentless(self.kind()))
    }

    /// Replace the body of the comment `comment` on `task`, answering with the comment as the
    /// source now holds it, or `None` when this source holds no such task or that task has no
    /// such comment.
    ///
    /// Only the body and the time it last changed move: the id, the author and the time it
    /// was written are the comment's own.
    ///
    /// # Errors
    ///
    /// As [`add_comment`](Self::add_comment).
    async fn edit_comment(
        &self,
        task: &NativeId,
        comment: &NativeId,
        body: &CommentBody,
    ) -> Result<Option<Comment>, SourceError> {
        let _ = (task, comment, body);
        Err(commentless(self.kind()))
    }

    /// Remove the comment `comment` from `task`, answering with the id it removed, or `None`
    /// when this source holds no such task or that task has no such comment.
    ///
    /// Unlike [`delete_task`](Self::delete_task), this *is* a verb of the product — a person
    /// removes a comment they posted — so a comment that is not there is reported as `None`
    /// for the engine to refuse by name, rather than treated as already gone.
    ///
    /// # Errors
    ///
    /// As [`add_comment`](Self::add_comment).
    async fn delete_comment(
        &self,
        task: &NativeId,
        comment: &NativeId,
    ) -> Result<Option<NativeId>, SourceError> {
        let _ = (task, comment);
        Err(commentless(self.kind()))
    }

    /// Write one comment a copy carries onto `task`: a new one when `existing` is `None`, and
    /// otherwise the comment `existing`, which a copy wrote before, made to say `copy` — or
    /// `None` when this source holds no such task or no such comment.
    ///
    /// The answer reports [`Comment::origin`] as `copy.origin`, however the source keeps it.
    ///
    /// **Defaulted to the hidden marker**, which is right for every source that records a
    /// comment's author and times itself: the body is written through
    /// [`add_comment`](Self::add_comment) or [`edit_comment`](Self::edit_comment) with
    /// [`CommentOrigin::mark`](crate::CommentOrigin::mark) appended, and read back
    /// [`unmarked`](Comment::unmarked). A source that can store the origin natively — and so
    /// takes the origin's own id, author and times for the copy — overrides this.
    ///
    /// # Errors
    ///
    /// As [`add_comment`](Self::add_comment).
    async fn write_comment_copy(
        &self,
        task: &NativeId,
        existing: Option<&NativeId>,
        copy: &CommentCopy,
    ) -> Result<Option<Comment>, SourceError> {
        let body = CommentBody::new(copy.origin.qualified()?.mark(copy.body.as_str()))?;
        let written = match existing {
            Some(comment) => self.edit_comment(task, comment, &body).await?,
            None => {
                self.add_comment(task, &NewComment { body, author: None })
                    .await?
            }
        };
        Ok(written.map(Comment::unmarked))
    }

    /// Whether this source keeps template answers beside its items at all.
    ///
    /// What lets a regenerate tell an item whose answers went missing — a block deleted by
    /// hand, which the regenerate writes back — from an item of a source that never keeps any,
    /// where a missing answer is no difference. Defaulted to `false`, as
    /// [`task_template_answers`](Self::task_template_answers) is defaulted to `None`; a source
    /// that keeps answers answers `true` here.
    fn keeps_template_answers(&self) -> bool {
        false
    }

    /// The template answers the task `id` was last rendered from, as this source keeps them
    /// beside the task — or `None` when it keeps none for it, or holds no such task.
    ///
    /// Keeping answers is a source's own choice, never an obligation: a source whose items
    /// are one record in a hosted system has no room beside the item that is not the item, and
    /// answers written into its content or its metadata would duplicate what the content
    /// already says. Defaulted to `None`, which is what keeps this an addition rather than a
    /// break. A source that keeps them owes three things: they are in neither
    /// [`Task::content`] nor [`Task::metadata`], they are written only by
    /// [`write_task_rendered`](Self::write_task_rendered) and
    /// [`set_task_rendering`](Self::set_task_rendering), and they come back here exactly as
    /// they were written, JSON types intact.
    ///
    /// # Errors
    ///
    /// Returns a [`SourceError`] when the source could not answer, a record whose answers it
    /// cannot read included.
    async fn task_template_answers(
        &self,
        id: &NativeId,
    ) -> Result<Option<BTreeMap<String, Value>>, SourceError> {
        let _ = id;
        Ok(None)
    }

    /// The template answers the document `id` was last rendered from, on exactly the terms of
    /// [`task_template_answers`](Self::task_template_answers).
    ///
    /// # Errors
    ///
    /// As [`task_template_answers`](Self::task_template_answers).
    async fn document_template_answers(
        &self,
        id: &NativeId,
    ) -> Result<Option<BTreeMap<String, Value>>, SourceError> {
        let _ = id;
        Ok(None)
    }

    /// The template answers the project `id` was last rendered from, on exactly the terms of
    /// [`task_template_answers`](Self::task_template_answers).
    ///
    /// # Errors
    ///
    /// As [`task_template_answers`](Self::task_template_answers).
    async fn project_template_answers(
        &self,
        id: &NativeId,
    ) -> Result<Option<BTreeMap<String, Value>>, SourceError> {
        let _ = id;
        Ok(None)
    }

    /// Create or update one task exactly as [`write_task`](Self::write_task) does, keeping
    /// `answers` — the answers its content was rendered from — beside it in the same write
    /// where this source keeps answers at all.
    ///
    /// The task arrives carrying its provenance under [`MetadataKey::TEMPLATE_KEY`] like any
    /// other metadata entry. Defaulted to [`write_task`](Self::write_task) alone: a source that
    /// keeps no answers writes the task and nothing beside it, which is the whole of what it
    /// owes. A source that keeps them writes the task and the answers together, so a reader
    /// never finds one without the other.
    ///
    /// # Errors
    ///
    /// As [`write_task`](Self::write_task).
    async fn write_task_rendered(
        &self,
        write: &ItemWrite<Task>,
        answers: &BTreeMap<String, Value>,
    ) -> Result<NativeId, SourceError> {
        let _ = answers;
        self.write_task(write).await
    }

    /// Create or update one document, on exactly the terms of
    /// [`write_task_rendered`](Self::write_task_rendered).
    ///
    /// # Errors
    ///
    /// As [`write_document`](Self::write_document).
    async fn write_document_rendered(
        &self,
        write: &ItemWrite<Document>,
        answers: &BTreeMap<String, Value>,
    ) -> Result<NativeId, SourceError> {
        let _ = answers;
        self.write_document(write).await
    }

    /// Create or update one project, on exactly the terms of
    /// [`write_task_rendered`](Self::write_task_rendered).
    ///
    /// # Errors
    ///
    /// As [`write_project`](Self::write_project).
    async fn write_project_rendered(
        &self,
        write: &ItemWrite<Project>,
        answers: &BTreeMap<String, Value>,
    ) -> Result<NativeId, SourceError> {
        let _ = answers;
        self.write_project(write).await
    }

    /// Replace one task's rendering — its content, byte for byte, its
    /// [`MetadataKey::TEMPLATE_KEY`] entry, set to `provenance`, and the answers it keeps
    /// beside the task where it keeps any — in one write, and change nothing else about it;
    /// or answer `None` when this source holds no such task.
    ///
    /// The content is [`Task::content`] exactly as a later read reports it, on the terms of
    /// [`set_task_content`](Self::set_task_content). Title, status, priority, labels, every
    /// other metadata entry, repositories, dependencies, [`Task::delivers`],
    /// [`Task::delivered_by`], project and comments are left exactly as they are. The write is
    /// one write: a reader sees the task as it was or as it is now, never a new content beside
    /// the old provenance or the old answers.
    ///
    /// Defaulted to [`unwritable_field`] on exactly the terms of
    /// [`set_task_status`](Self::set_task_status). A source declaring
    /// [`WriteSupport::Unsupported`] is never asked.
    ///
    /// [`Task::delivers`]: crate::Task::delivers
    /// [`Task::delivered_by`]: crate::Task::delivered_by
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::Refused`] when this source cannot replace a task's rendering on
    /// its own, or cannot represent this one; and whatever else the source could not do the
    /// write for.
    async fn set_task_rendering(
        &self,
        id: &NativeId,
        content: &str,
        provenance: &Value,
        answers: &BTreeMap<String, Value>,
    ) -> Result<Option<()>, SourceError> {
        let _ = (id, content, provenance, answers);
        Err(unwritable_field(self.kind(), "rendering"))
    }

    /// Replace one document's rendering, on exactly the terms of
    /// [`set_task_rendering`](Self::set_task_rendering). A source declaring
    /// [`Capabilities::documents`] unsupported is never asked.
    ///
    /// # Errors
    ///
    /// As [`set_task_rendering`](Self::set_task_rendering).
    async fn set_document_rendering(
        &self,
        id: &NativeId,
        content: &str,
        provenance: &Value,
        answers: &BTreeMap<String, Value>,
    ) -> Result<Option<()>, SourceError> {
        let _ = (id, content, provenance, answers);
        Err(SourceError::Refused {
            message: format!(
                "the {} plugin cannot write a document's rendering on its own",
                self.kind()
            ),
        })
    }

    /// Replace one project's rendering, on exactly the terms of
    /// [`set_task_rendering`](Self::set_task_rendering): its content, its
    /// [`MetadataKey::TEMPLATE_KEY`] entry and the answers kept beside it change, and its
    /// title, status, labels, every other metadata entry, repositories and dependencies are
    /// left exactly as they are.
    ///
    /// # Errors
    ///
    /// As [`set_task_rendering`](Self::set_task_rendering).
    async fn set_project_rendering(
        &self,
        id: &NativeId,
        content: &str,
        provenance: &Value,
        answers: &BTreeMap<String, Value>,
    ) -> Result<Option<()>, SourceError> {
        let _ = (id, content, provenance, answers);
        Err(SourceError::Refused {
            message: format!(
                "the {} plugin cannot write a project's rendering on its own",
                self.kind()
            ),
        })
    }

    /// The image assets the task `id` holds, or none when it holds none or this source holds
    /// no such task.
    ///
    /// Defaulted to none, which is what keeps assets an addition rather than a break: a source
    /// that keeps no assets needs no edit and reports every record as holding none. A source
    /// declaring [`Capabilities::assets`] native answers what it holds for the record, in any
    /// order — the engine orders them by the content's first reference.
    ///
    /// # Errors
    ///
    /// Returns a [`SourceError`] when the source could not answer.
    async fn task_assets(&self, id: &NativeId) -> Result<Vec<Asset>, SourceError> {
        let _ = id;
        Ok(Vec::new())
    }

    /// The image assets the document `id` holds, on exactly the terms of
    /// [`task_assets`](Self::task_assets).
    ///
    /// # Errors
    ///
    /// As [`task_assets`](Self::task_assets).
    async fn document_assets(&self, id: &NativeId) -> Result<Vec<Asset>, SourceError> {
        let _ = id;
        Ok(Vec::new())
    }

    /// The bytes of the asset `name` the task `id` holds, or `None` when it holds no such asset
    /// or this source holds no such task.
    ///
    /// Defaulted to `None`, on the terms of [`task_assets`](Self::task_assets).
    ///
    /// # Errors
    ///
    /// Returns a [`SourceError`] when the source could not answer.
    async fn task_asset(
        &self,
        id: &NativeId,
        name: &AssetName,
    ) -> Result<Option<Vec<u8>>, SourceError> {
        let _ = (id, name);
        Ok(None)
    }

    /// The bytes of the asset `name` the document `id` holds, on exactly the terms of
    /// [`task_asset`](Self::task_asset).
    ///
    /// # Errors
    ///
    /// As [`task_asset`](Self::task_asset).
    async fn document_asset(
        &self,
        id: &NativeId,
        name: &AssetName,
    ) -> Result<Option<Vec<u8>>, SourceError> {
        let _ = (id, name);
        Ok(None)
    }

    /// Create or update one task carrying image assets, in one write: the task as
    /// [`write_task`](Self::write_task) writes it — or, with `answers`, as
    /// [`write_task_rendered`](Self::write_task_rendered) does — and `assets` as the record's
    /// whole asset set, answering with where it landed and the content as stored.
    ///
    /// Only a source declaring [`Capabilities::assets`] native is asked, and the engine has
    /// already checked that every reference in the content names one of `assets` and every one
    /// of `assets` is referenced. A source that keeps the bytes beside the record stores each
    /// payload under its name, removes every asset the record held that `assets` does not
    /// name, and answers the content unchanged. A source that serves them at a URL uploads each
    /// payload that carries bytes, reuses the URL [`AssetWrite::recorded_assets`] records for
    /// each that does not, and writes the record through
    /// [`serve_asset_references`](crate::serve_asset_references) — so its references point at
    /// those URLs, its [`MetadataKey::ASSETS_KEY`] records each `{sha256, url}`, and a
    /// rendering's `body_digest` stays true of what landed — answering the content it wrote.
    ///
    /// Defaulted to [`assetless`], which is what a source with no assets answers.
    ///
    /// # Errors
    ///
    /// As [`write_task`](Self::write_task), and [`SourceError::Refused`] from a source that
    /// keeps no assets.
    async fn write_task_with_assets(
        &self,
        write: &ItemWrite<Task>,
        answers: Option<&BTreeMap<String, Value>>,
        assets: &AssetWrite,
    ) -> Result<AssetsWritten, SourceError> {
        let _ = (write, answers, assets);
        Err(assetless(self.kind()))
    }

    /// Create or update one document carrying image assets, on exactly the terms of
    /// [`write_task_with_assets`](Self::write_task_with_assets).
    ///
    /// # Errors
    ///
    /// As [`write_task_with_assets`](Self::write_task_with_assets).
    async fn write_document_with_assets(
        &self,
        write: &ItemWrite<Document>,
        answers: Option<&BTreeMap<String, Value>>,
        assets: &AssetWrite,
    ) -> Result<AssetsWritten, SourceError> {
        let _ = (write, answers, assets);
        Err(assetless(self.kind()))
    }

    /// Replace one task's rendering as [`set_task_rendering`](Self::set_task_rendering) does,
    /// and its asset set as [`write_task_with_assets`](Self::write_task_with_assets) does, in
    /// one write; or answer `None` when this source holds no such task.
    ///
    /// Defaulted to [`assetless`].
    ///
    /// # Errors
    ///
    /// As [`set_task_rendering`](Self::set_task_rendering), and [`SourceError::Refused`] from a
    /// source that keeps no assets.
    async fn set_task_rendering_with_assets(
        &self,
        id: &NativeId,
        content: &str,
        provenance: &Value,
        answers: &BTreeMap<String, Value>,
        assets: &AssetWrite,
    ) -> Result<Option<AssetsWritten>, SourceError> {
        let _ = (id, content, provenance, answers, assets);
        Err(assetless(self.kind()))
    }

    /// Replace one document's rendering and its asset set, on exactly the terms of
    /// [`set_task_rendering_with_assets`](Self::set_task_rendering_with_assets).
    ///
    /// # Errors
    ///
    /// As [`set_task_rendering_with_assets`](Self::set_task_rendering_with_assets).
    async fn set_document_rendering_with_assets(
        &self,
        id: &NativeId,
        content: &str,
        provenance: &Value,
        answers: &BTreeMap<String, Value>,
        assets: &AssetWrite,
    ) -> Result<Option<AssetsWritten>, SourceError> {
        let _ = (id, content, provenance, answers, assets);
        Err(assetless(self.kind()))
    }

    /// What this source has sent to its backend since it was built and what that spent, or
    /// `None` when it does not meter its own requests.
    ///
    /// Defaulted to `None`, which is what keeps metering an addition rather than a break: a
    /// source that does not count its requests needs no edit, and is reported as not
    /// metering rather than as having spent nothing. A source that answers owes a running
    /// total — see [`Metering`] — because what one command spent is read as the difference
    /// between two readings.
    ///
    /// # Errors
    ///
    /// Returns a [`SourceError`] when the reading itself could not be taken. A caller
    /// reports such a source as not metering; what a command cost is never a reason for the
    /// command to fail.
    async fn metering(&self) -> Result<Option<Metering>, SourceError> {
        Ok(None)
    }

    /// End one command: drop everything held that a person can change.
    ///
    /// A command is one unit of work — one invocation of the binary, or whatever a caller
    /// holding one engine for many units of work calls between them. Within a command a
    /// source may hold what it read and reuse it; across this call it may not. After it
    /// returns, a source answers no read and bases no write on item content, statuses, board
    /// contents or search results it held before the call. It may keep only identifiers and
    /// vocabulary that stay valid in normal use — a repository's node id, a team's workflow
    /// states — and only where a lookup that misses one re-reads rather than refusing.
    ///
    /// Defaulted to doing nothing, which is right for a source that holds nothing between
    /// calls or holds only what the rule above lets it keep. A source that holds more owes
    /// an override. Never called by the engine on its own: the binary's one command is its
    /// whole process, so only a caller that outlives a command calls it.
    ///
    /// # Errors
    ///
    /// Returns a [`SourceError`] when the source could not be sure it dropped what it held —
    /// ordinarily a hosted plugin that did not answer. A caller should not go on to the next
    /// command on that source's earlier reads.
    async fn end_command(&self) -> Result<(), SourceError> {
        Ok(())
    }

    /// Who can read where `target` lands, read live from the backend.
    ///
    /// This is the destination's reality, which the engine holds a declaration against
    /// before it lets a write through: a source a configuration declares private receives an
    /// item that may only be written somewhere private only while this answers
    /// [`Visibility::Private`]. The engine asks before the write it is about. A source must
    /// not answer from anything it read for an earlier write unless its backend's visibility
    /// cannot change in between — or the read was taken while the engine held this source's
    /// reads for one operation ([`hold_visibility`](Self::hold_visibility)), which a copy
    /// does so each destination is read once for the whole copy rather than once per write.
    ///
    /// Defaulted to [`Visibility::Unknown`], which is the answer of a source that cannot say
    /// and is never treated as private. A source on this machine answers `Private` — what it
    /// writes stays on the host — and a hosted source reads its backend.
    ///
    /// # Errors
    ///
    /// Returns a [`SourceError`] when the backend could not be asked or would not answer —
    /// a credential lacking the scope the read needs among them. An unreadable visibility is
    /// never guessed: the engine refuses the write.
    async fn visibility(&self, target: &WriteTarget<'_>) -> Result<Visibility, SourceError> {
        let _ = target;
        Ok(Visibility::Unknown)
    }

    /// The repository a create of `target` would file its issue in, as a normalized origin,
    /// by the same rule the create itself applies — or `None` for a source that files no
    /// issue in a repository.
    ///
    /// Read-only: it writes nothing, and reads only what that rule reads — for a source whose
    /// rule looks at the project a new item is filed under, that project, and only when
    /// `target` names one. It answers where a *create* would file, so an
    /// [`WriteTarget::Existing`] target, which no create makes, answers `None`.
    ///
    /// Defaulted to `None`, the answer of every source that files no issue in a repository.
    ///
    /// # Errors
    ///
    /// Returns a [`SourceError`] when the create would be refused — refused as the create
    /// would refuse it — or the read the rule needs failed.
    async fn creation_repository(
        &self,
        target: &WriteTarget<'_>,
    ) -> Result<Option<Repository>, SourceError> {
        let _ = target;
        Ok(None)
    }

    /// Read a create's visibility under a project this copy will create or update.
    ///
    /// `project` names the destination project, including a create whose id does not exist
    /// yet. Backends whose placement depends on its repository predict that repository
    /// with the same rule as the actual create. Other backends use `visibility` unchanged.
    ///
    /// # Errors
    /// Returns the same failures as [`visibility`](Self::visibility).
    async fn visibility_with_project(
        &self,
        target: &WriteTarget<'_>,
        project: &WriteTarget<'_>,
    ) -> Result<Visibility, SourceError> {
        let _ = project;
        self.visibility(target).await
    }

    /// Told, once the configuration is read, who the configuration declares can read this
    /// source: [`Visibility::Private`] for a source declared `visibility: private`,
    /// [`Visibility::Public`] for one declared public, and [`Visibility::Unknown`] for one
    /// that declares nothing.
    ///
    /// A declaration, never the backend's reality, which [`visibility`](Self::visibility)
    /// reads. A source whose backend places a new item somewhere decided per item may place
    /// it differently on a source declared private — a GitHub board files an issue whose one
    /// named repository reads public in a private repository instead — and must place it
    /// exactly as before on every other. Defaulted to ignoring it, which is what every source
    /// that places an item the same way whoever may read it does.
    fn declare_visibility(&self, declared: Visibility) {
        let _ = declared;
    }

    /// Run an operation with isolated visibility reads, including when operations overlap.
    ///
    /// The default executes the future unchanged. Backends that hold visibility reads may
    /// scope them to this future rather than sharing them with another operation.
    fn scope_visibility<'a>(
        &'a self,
        operation: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
        operation
    }

    /// Begin one engine operation's hold on what [`visibility`](Self::visibility) reads: from
    /// now until the matching [`release_visibility`](Self::release_visibility), a source may
    /// answer each destination from its first read in that hold rather than reading it again.
    ///
    /// Holds nest: only the release matching the outermost hold ends it, and that release
    /// drops everything read. A copy holds every source for the length of the copy, so a
    /// destination whose visibility changes during one copy is seen by the next. Defaulted to
    /// doing nothing, which is right for a source that reads nothing to answer.
    fn hold_visibility(&self) {}

    /// End the hold [`hold_visibility`](Self::hold_visibility) began, dropping every
    /// visibility read in it once the outermost hold ends. Defaulted to doing nothing.
    fn release_visibility(&self) {}
}

/// The factory that turns one configuration block into a live [`TaskSource`].
///
/// Having the compile-time registry and the subprocess seam be the same shape is
/// the whole reason this is a trait rather than a free function.
pub trait SourcePlugin: Send + Sync + 'static {
    /// The name a configuration document's `plugin:` field names.
    fn kind(&self) -> &'static str;

    /// The JSON Schema for this plugin's own `config:` block.
    fn config_schema(&self) -> Schema;

    /// Build a live source from one configuration block.
    ///
    /// `name` is the configured source's name, for error messages only — a
    /// plugin never learns it for any other purpose.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::Config`] when `config` is not valid for this
    /// plugin, or [`SourceError::Auth`] when a named credential is absent.
    fn build(
        &self,
        name: &SourceName,
        config: &serde_json::Value,
        secrets: &dyn SecretResolver,
    ) -> Result<Box<dyn TaskSource>, SourceError>;

    /// Build a live source from one configuration block, handing it the clock it paces and
    /// backs off on.
    ///
    /// The binary builds every in-process source through this, with the process's one
    /// [`SharedClock`] — [`system_clock`](crate::system_clock) unless a test asked for
    /// simulated time. Defaulted to [`build`](Self::build), which ignores the clock: a plugin
    /// that never waits needs no edit, and one that does overrides this and routes its waits
    /// through `clock`.
    ///
    /// # Errors
    ///
    /// As [`build`](Self::build).
    fn build_with_clock(
        &self,
        name: &SourceName,
        config: &serde_json::Value,
        secrets: &dyn SecretResolver,
        clock: SharedClock,
    ) -> Result<Box<dyn TaskSource>, SourceError> {
        let _ = clock;
        self.build(name, config, secrets)
    }

    /// The fields of this plugin's `config:` block that name a filesystem path, as dotted
    /// paths into that block.
    ///
    /// A relative value at one of these, **supplied by a configuration document**, is
    /// resolved against the directory holding that document before [`Self::build`] sees it;
    /// supplied through the environment or a flag it keeps resolving against the process
    /// working directory, because there is no document to rebase it on. A plugin is handed
    /// values and no origins, so this declaration is the only way it can say which of its
    /// own fields that rule reaches.
    ///
    /// Defaulted to none, which is what keeps this an addition rather than a break: a
    /// plugin whose block holds no path needs no edit, and a caller asks every plugin
    /// rather than keeping a table of which ones answer.
    // llmlint: ignore[invalid_states_unrepresentable] The identity of a configuration field
    // is a name, and no type can make a wrong one unrepresentable here: every string is a
    // syntactically valid dotted path, so a newtype would validate nothing and would only
    // move where a name that is not a field of *this* plugin is accepted. What decides that
    // is whether the name is a property of the schema `config_schema` publishes — a
    // per-plugin fact no shared type can hold — so the gate is per plugin and executable:
    // `document_relative_fields_are_fields_this_plugin_declares` in
    // `onetaskgraph-local-md/tests/plugin.rs`, which a plugin adding a declaration owes its
    // own copy of.
    fn document_relative_paths(&self) -> &'static [&'static str] {
        &[]
    }

    /// What names the remote store a source built from `config` reads — a board's owner and
    /// number, a Linear endpoint and team — or `None` for a source with no remote.
    ///
    /// The store's local cache of a remote source lives under a digest of this, so every
    /// process configuring the same remote shares one cache and no two remotes share one.
    /// Defaulted to `None`, which is what a source keeping its work on this machine — a
    /// folder of Markdown — answers: it has nothing to cache, and its cache setting is
    /// ignored. A `config` this plugin would refuse answers `None` too.
    fn remote_identity(&self, config: &serde_json::Value) -> Option<String> {
        let _ = config;
        None
    }
}

/// How a plugin reads the credential its configuration names.
///
/// A configuration document never carries a credential value, only the name of
/// the environment variable holding it.
pub trait SecretResolver: Send + Sync {
    /// The value of `var`, or `None` when nothing defines it.
    ///
    /// The returned value is never logged and never appears in `Debug` output.
    fn get(&self, var: &str) -> Option<SecretString>;
}
