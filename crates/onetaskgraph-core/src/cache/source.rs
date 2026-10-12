//! A caching source: the plugin's own source, answering from the cache what a pin vouches for.
//!
//! The engine wraps every source configured with `cache.enabled` in one of these, so every read
//! path the engine has — `task show`, `show-many`, a comment listing and every read a copy
//! makes — meets the cache at one seam, and no verb decides for itself whether an entry is
//! fresh. Everything is forwarded to the plugin's own source except:
//!
//! - **A listing** is always sent, and each task it carries pins the reads of it that follow in
//!   the same command.
//! - **A read by id** is answered from the cache when it is pinned — by a caller, by a listing,
//!   or by a write expecting a version — and the entry is at that version or a later one, by the
//!   source's own ordering. Every other read is sent, and what it answered is written to the
//!   entry. A part of a task the source's token does not cover is never served.
//! - **A write** is sent, and leaves the entry unserveable — unless it was expected, when the
//!   source is primed with what the entry kept so it need not read first, and the entry is
//!   advanced to what the write's own last response showed whole, when it showed it.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use onetaskgraph_plugin_api::{
    Asset, AssetName, AssetWrite, AssetsWritten, Capabilities, Comment, CommentBody, CommentCopy,
    DependencyEdge, Direction, Document, DocumentQuery, Health, ItemKind, ItemWrite, Label,
    MetadataKey, Metering, NativeId, NewComment, Page, PageRequest, PartOutcome, Priority, Project,
    ProjectQuery, SourceError, Status, StatusCategory, Task, TaskDetailRead, TaskQuery, TaskRef,
    TaskSource, TaskUpdate, TaskUpdateOutcome, VersionCoverage, Visibility, WriteJournal,
    WriteSupport, WriteTarget,
};
use serde_json::Value;

use super::{
    CacheUse, ENTRY_SCHEMA, Entry, ExpectedWrite, Priming, SourceCache, WriteFallback, WritePart,
    WriteResult,
};

/// One write's future, boxed so one helper can run any of the trait's writes.
type Writing<'a, T> = Pin<Box<dyn Future<Output = Result<T, SourceError>> + Send + 'a>>;

/// One batched detail read's future.
type Reading<'a> =
    Pin<Box<dyn Future<Output = Vec<Result<Option<TaskDetailRead>, SourceError>>> + Send + 'a>>;

/// Why a read could not be answered from the cache.
enum Miss {
    /// Nothing pins it: an unpinned read goes to the source, as it always did.
    Unpinned,
    /// A part it needs is one the source's token does not cover.
    Bypassed(String),
    /// No entry at the pinned version or a later one.
    Behind,
}

/// The plugin's own source, behind the cache of it.
pub(crate) struct CachedSource {
    inner: Box<dyn TaskSource>,
    cache: Arc<SourceCache>,
}

impl CachedSource {
    /// `inner`, behind `cache`.
    pub(crate) fn new(inner: Box<dyn TaskSource>, cache: Arc<SourceCache>) -> Self {
        Self { inner, cache }
    }

    /// The version reads of `id` are pinned to in this command, if any: a caller's pin, a
    /// write expecting it, or the version a listing carried.
    fn pin(&self, id: &NativeId) -> Option<String> {
        let command = self.cache.command();
        command
            .pins
            .get(id)
            .or_else(|| {
                command
                    .expected
                    .as_ref()
                    .filter(|expected| expected.id == *id)
                    .map(|expected| &expected.version)
            })
            .or_else(|| command.listed.get(id))
            .cloned()
    }

    /// Whether `held` is at `pin` or a later version of the same task.
    fn fresh(&self, held: &str, pin: &str) -> bool {
        matches!(
            self.inner.order_versions(held, pin),
            Some(Ordering::Equal | Ordering::Greater)
        )
    }

    /// The entry a read of `id` may be answered from: pinned, covered, and at the pin or later.
    fn servable(&self, id: &NativeId, comments: bool) -> Result<Entry, Miss> {
        let pin = self.pin(id).ok_or(Miss::Unpinned)?;
        if let Some(reason) = self.cache.coverage.fields.reason() {
            return Err(Miss::Bypassed(reason.to_owned()));
        }
        if comments && let Some(reason) = self.cache.coverage.comments.reason() {
            return Err(Miss::Bypassed(reason.to_owned()));
        }
        let entry = self.cache.read(id).ok_or(Miss::Behind)?;
        let fresh = entry.version().is_some_and(|held| self.fresh(held, &pin));
        if fresh && (!comments || entry.comments.is_some()) {
            Ok(entry)
        } else {
            Err(Miss::Behind)
        }
    }

    /// Record that the read of `id` was answered from `entry`.
    fn served(&self, entry: &Entry) {
        let mut command = self.cache.command();
        command
            .used
            .insert(entry.task.id.clone(), CacheUse::cache());
        command.served.insert(entry.task.id.clone(), entry.clone());
    }

    /// Record that the read of `id` went to the source, for `miss`, and answered `version`.
    fn missed(&self, id: &NativeId, miss: &Miss, version: Option<&str>) {
        let bypassed = match miss {
            Miss::Bypassed(reason) => Some(reason.clone()),
            Miss::Unpinned | Miss::Behind => self
                .cache
                .coverage
                .fields
                .reason()
                .map(str::to_owned)
                .or_else(|| {
                    version.is_none().then(|| {
                        "the source gave this task no change token, so the cache cannot \
                         vouch for any copy of it"
                            .to_owned()
                    })
                }),
        };
        let mut command = self.cache.command();
        command.used.insert(id.clone(), CacheUse::remote(bypassed));
        command.served.remove(id);
        match version {
            Some(version) => command.read.insert(id.clone(), version.to_owned()),
            None => command.read.remove(id),
        };
    }

    /// Write what a read of `task` from the source answered, keeping what the entry already
    /// held at the very same version and the read did not carry.
    fn remember(&self, task: &Task, comments: Option<Vec<Comment>>) {
        let held = self
            .cache
            .read(&task.id)
            .filter(|held| held.version().is_some() && held.version() == task.version.as_deref());
        let (assets, edges, kept) = match held {
            Some(held) => (held.assets, held.edges, held.comments),
            None => (None, None, None),
        };
        self.cache.write(&Entry {
            schema: ENTRY_SCHEMA,
            remote: self.cache.remote.clone(),
            task: task.clone(),
            comments: comments.or(kept),
            assets,
            edges,
            context: self.inner.write_context(&task.id),
        });
    }

    /// Add `assets` or `edges` to the entry of `id`, when the read that answered them followed
    /// a read of the task from the source at the entry's own version in this command.
    fn complete(&self, id: &NativeId, assets: Option<&[Asset]>, edges: Option<&[DependencyEdge]>) {
        let read = self.cache.command().read.get(id).cloned();
        let Some(read) = read else {
            return;
        };
        let Some(mut entry) = self.cache.read(id) else {
            return;
        };
        if entry.version() != Some(read.as_str()) {
            return;
        }
        if let Some(assets) = assets {
            entry.assets = Some(assets.to_vec());
        }
        if let Some(edges) = edges {
            entry.edges = Some(edges.to_vec());
        }
        self.cache.write(&entry);
    }

    /// The entry `id` was served from in this command, or one a pin vouches for.
    fn held(&self, id: &NativeId) -> Option<Entry> {
        let served = self.cache.command().served.get(id).cloned();
        served.or_else(|| self.servable(id, false).ok())
    }

    /// Answer `ids` from the cache where a pin vouches for them, and the rest with one read of
    /// the source through `read`, in the order asked.
    async fn details<'a>(
        &'a self,
        ids: &[NativeId],
        page: Option<&PageRequest>,
        edges: bool,
        read: impl FnOnce(Vec<NativeId>) -> Reading<'a>,
    ) -> Vec<Result<Option<TaskDetailRead>, SourceError>> {
        let mut answers: Vec<Option<Result<Option<TaskDetailRead>, SourceError>>> =
            Vec::with_capacity(ids.len());
        let mut unread = Vec::new();
        let mut misses = Vec::new();
        for id in ids {
            let usable = self.servable(id, page.is_some()).and_then(|entry| {
                let fits = page.is_none_or(|page| {
                    entry
                        .comments
                        .as_ref()
                        .is_some_and(|comments| comments.len() <= page.limit as usize)
                });
                // A detail read is followed by a read of the task's assets, and a copy's by its
                // edges — and by its assets' bytes only when its content names one, which the
                // cache does not keep: an entry missing what follows would send a request the
                // uncached read carried for nothing, so it is not served.
                let follows = if edges {
                    // A source primed for a write expecting this task answers its edges from what
                    // it was primed with.
                    (entry.edges.is_some() || self.primed_for(id))
                        && onetaskgraph_plugin_api::asset_references(
                            entry.task.content.as_deref().unwrap_or_default(),
                        )
                        .is_empty()
                } else {
                    entry.assets.is_some()
                };
                if fits && follows {
                    Ok(entry)
                } else {
                    Err(Miss::Behind)
                }
            });
            match usable {
                Ok(entry) => {
                    self.served(&entry);
                    let comments = page.map(|_| {
                        Ok(Some(Page {
                            items: entry.comments.clone().unwrap_or_default(),
                            next: None,
                        }))
                    });
                    answers.push(Some(Ok(Some(TaskDetailRead {
                        task: entry.task,
                        comments,
                    }))));
                }
                Err(miss) => {
                    answers.push(None);
                    unread.push(id.clone());
                    misses.push(miss);
                }
            }
        }
        if !unread.is_empty() {
            let asked = unread.clone();
            let mut read = read(asked).await;
            // llmlint: ignore-block[changed_behavior_has_e2e] A source answering another number of details than it was asked for breaks the trait's own contract, and no source this build ships can: it is answered exactly as `Engine::task_details` answers the same miscount — every id of the read refused as malformed, none shifted onto another's slot — which the engine's own `ShortDetails` test proves. No real interface reaches this branch to drive it end to end.
            if read.len() != unread.len() {
                let message = format!(
                    "the source answered {} task details for the {} ids it was asked for",
                    read.len(),
                    unread.len()
                );
                read = unread
                    .iter()
                    .map(|_| {
                        Err(SourceError::Malformed {
                            message: message.clone(),
                        })
                    })
                    .collect();
            }
            // llmlint: ignore-end[changed_behavior_has_e2e]
            for ((id, miss), answer) in unread.iter().zip(&misses).zip(read) {
                match &answer {
                    // Recorded only for the task it is about: an answer naming another task is
                    // the engine's to refuse, and nothing of it reaches this task's entry.
                    Ok(Some(detail)) if detail.task.id == *id => {
                        let comments = match &detail.comments {
                            Some(Ok(Some(page))) if page.next.is_none() => Some(page.items.clone()),
                            _ => None,
                        };
                        self.missed(id, miss, detail.task.version.as_deref());
                        self.remember(&detail.task, comments);
                    }
                    _ => self.missed(id, miss, None),
                }
                if let Some(slot) = answers.iter_mut().find(|slot| slot.is_none()) {
                    *slot = Some(answer);
                }
            }
        }
        answers.into_iter().flatten().collect()
    }

    /// Whether the source was primed for a write expecting `id`.
    fn primed_for(&self, id: &NativeId) -> bool {
        self.cache
            .command()
            .expected
            .as_ref()
            .is_some_and(|expected| {
                expected.id == *id && expected.primed.as_ref().is_some_and(Priming::primed)
            })
    }

    /// Prime the source with what the cache kept for `id`, once, when a write expecting it
    /// is under way: whether it was primed, and why a read follows when it was not.
    fn prime_expected(&self, id: &NativeId) -> Option<Priming> {
        let version = {
            let command = self.cache.command();
            let expected = command
                .expected
                .as_ref()
                .filter(|expected| expected.id == *id)?;
            if let Some(primed) = &expected.primed {
                return Some(primed.clone());
            }
            expected.version.clone()
        };
        let entry = self.cache.read(id);
        let decided = match &entry {
            None => Priming::Unprimed {
                reason: "the cache holds no entry for this task, so its target was read".to_owned(),
            },
            Some(entry)
                if !entry
                    .version()
                    .is_some_and(|held| self.fresh(held, &version)) =>
            {
                Priming::Unprimed {
                    reason: "the cache holds this task only at an older version than the one \
                             expected, so its target was read"
                        .to_owned(),
                }
            }
            Some(entry) => match &entry.context {
                Some(context) => {
                    let board = self.cache.board();
                    if self.inner.prime_write(id, context, board.as_ref()) {
                        Priming::Primed {
                            reason: "the cache did not hold all the write needed — the board's \
                                     field ids — so it was read"
                                .to_owned(),
                        }
                    } else {
                        Priming::Unprimed {
                            reason: "the source could not take what the cache kept for this \
                                     task, so its target was read"
                                .to_owned(),
                        }
                    }
                }
                None => Priming::Unprimed {
                    reason: "the cache kept nothing a write of this task could take instead of \
                             reading it, so its target was read"
                        .to_owned(),
                },
            },
        };
        let mut command = self.cache.command();
        if let Some(expected) = command
            .expected
            .as_mut()
            .filter(|expected| expected.id == *id)
        {
            expected.primed = Some(decided.clone());
        }
        Some(decided)
    }

    /// Run one write of task `id` through `write`: plainly, leaving its entry unserveable, or —
    /// when the engine expects it — primed with what the entry kept, journaled, and sent once
    /// more after one read if a primed id turned out stale.
    async fn write<'a, T>(
        &'a self,
        id: Option<&NativeId>,
        write: impl Fn() -> Writing<'a, T>,
    ) -> Result<T, SourceError> {
        let expected = id.and_then(|id| {
            let command = self.cache.command();
            command
                .expected
                .as_ref()
                .filter(|expected| expected.id == *id)
                .map(|expected| expected.version.clone())
        });
        let (Some(id), Some(_)) = (id, expected) else {
            let written = write().await;
            if let Some(id) = id {
                self.cache.invalidate(id);
            }
            return written;
        };
        let before = self.inner.metering().await.ok().flatten();
        let priming = self
            .prime_expected(id)
            .unwrap_or_else(|| Priming::Unprimed {
                reason: "the write was not expected when it began, so its target was read"
                    .to_owned(),
            });
        let primed = priming.primed();
        let mut reason = priming.reason().to_owned();
        self.inner.begin_journal();
        let mut written = write().await;
        let mut journal = self.inner.end_journal();
        if written.is_err() && primed && journal.as_ref().is_some_and(WriteJournal::stale) {
            // One read, then the refused part once more: the read refreshes what the cache
            // kept, so the same stale id cannot be sent again.
            let refused = journal
                .as_ref()
                .and_then(|journal| {
                    journal.mutations().find_map(|entry| match &entry.outcome {
                        PartOutcome::Rejected { error, stale: true } => Some(error.clone()),
                        _ => None,
                    })
                })
                .unwrap_or_default();
            reason = format!(
                "an id the cache kept was stale — the source refused it: {refused} — so the \
                 target was read and the refused part sent once more"
            );
            self.inner.unprime(id);
            self.cache.drop_board();
            self.inner.begin_journal();
            written = write().await;
            if let (Some(first), Some(again)) = (journal.as_mut(), self.inner.end_journal()) {
                first.requests.extend(again.requests);
            }
        }
        if let Some(board) = self.inner.board_context() {
            self.cache.write_board(board);
        }
        let after = self.inner.metering().await.ok().flatten();
        // Settled before the source lets go of what it was primed with: the entry takes the
        // record the write's own response left behind.
        let report = self.settle(id, journal.as_ref(), reason, before, after);
        self.inner.unprime(id);
        self.cache.report(id, report);
        written
    }

    /// Advance the entry of `id` to what `journal`'s last mutation showed whole, or leave it
    /// unserveable; and answer what the write came to.
    fn settle(
        &self,
        id: &NativeId,
        journal: Option<&WriteJournal>,
        reason: String,
        before: Option<Metering>,
        after: Option<Metering>,
    ) -> ExpectedWrite {
        let spent = crate::engine::spent_between(&[before], &[after]);
        let Some(journal) = journal else {
            // A source that records no journal wrote as it always does, and says nothing about
            // what the write left behind.
            self.cache.invalidate(id);
            return ExpectedWrite {
                result: None,
                fallback: None,
                spent,
            };
        };
        let last = journal.mutations().last();
        let landed_last = last.filter(|entry| entry.outcome == PartOutcome::Landed);
        // Taken only when it is this write's own task: a response showing another task says
        // nothing about this one, whose entry is then left unserveable.
        let whole = landed_last
            .and_then(|entry| entry.shown.whole())
            .filter(|whole| whole.task().id == *id);
        match whole {
            Some(whole) => {
                self.cache.write(&Entry {
                    schema: ENTRY_SCHEMA,
                    remote: self.cache.remote.clone(),
                    task: whole.task().clone(),
                    comments: Some(whole.comments().to_vec()),
                    assets: Some(whole.assets().to_vec()),
                    edges: whole.edges().map(<[DependencyEdge]>::to_vec),
                    context: self.inner.write_context(id),
                });
                let mut command = self.cache.command();
                command.served.remove(id);
                command.read.remove(id);
            }
            _ => self.cache.invalidate(id),
        }
        ExpectedWrite {
            result: Some(WriteResult {
                version: landed_last.and_then(|entry| entry.shown.version().map(str::to_owned)),
                parts: journal
                    .mutations()
                    .map(|entry| WritePart {
                        part: entry.part.clone(),
                        outcome: entry.outcome.clone(),
                        fields: entry.fields.clone(),
                    })
                    .collect(),
            }),
            fallback: journal
                .read()
                .then_some(WriteFallback { read: true, reason }),
            spent,
        }
    }
}

#[async_trait::async_trait]
impl TaskSource for CachedSource {
    fn kind(&self) -> &'static str {
        self.inner.kind()
    }

    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }

    fn order_versions(&self, a: &str, b: &str) -> Option<Ordering> {
        self.inner.order_versions(a, b)
    }

    fn version_coverage(&self) -> VersionCoverage {
        self.inner.version_coverage()
    }

    fn write_context(&self, id: &NativeId) -> Option<Value> {
        self.inner.write_context(id)
    }

    fn board_context(&self) -> Option<Value> {
        self.inner.board_context()
    }

    fn prime_write(&self, id: &NativeId, context: &Value, board: Option<&Value>) -> bool {
        self.inner.prime_write(id, context, board)
    }

    fn unprime(&self, id: &NativeId) {
        self.inner.unprime(id);
    }

    fn begin_journal(&self) {
        self.inner.begin_journal();
    }

    fn end_journal(&self) -> Option<WriteJournal> {
        self.inner.end_journal()
    }

    async fn health(&self) -> Result<Health, SourceError> {
        self.inner.health().await
    }

    async fn get_task(&self, id: &NativeId) -> Result<Option<Task>, SourceError> {
        self.prime_expected(id);
        match self.servable(id, false) {
            Ok(entry) => {
                self.served(&entry);
                Ok(Some(entry.task))
            }
            Err(miss) => {
                let read = self.inner.get_task(id).await;
                match &read {
                    // Recorded only for the task it is about, as a batched detail read is.
                    Ok(Some(task)) if task.id == *id => {
                        self.missed(id, &miss, task.version.as_deref());
                        self.remember(task, None);
                    }
                    _ => self.missed(id, &miss, None),
                }
                read
            }
        }
    }

    async fn get_task_details(
        &self,
        ids: &[NativeId],
        comments: Option<&PageRequest>,
    ) -> Vec<Result<Option<TaskDetailRead>, SourceError>> {
        for id in ids {
            self.prime_expected(id);
        }
        self.details(ids, comments, false, |unread| {
            Box::pin(async move {
                // One task with its comments is read the way a copy reads it — the same one
                // request — so what the source keeps of it carries its `blockedBy` too, and a
                // write expecting it later reads none of its edges either.
                match (unread.as_slice(), comments) {
                    ([_], Some(page)) => self.inner.get_task_details_for_copy(&unread, page).await,
                    _ => self.inner.get_task_details(&unread, comments).await,
                }
            })
        })
        .await
    }

    async fn get_task_details_for_copy(
        &self,
        ids: &[NativeId],
        comments: &PageRequest,
    ) -> Vec<Result<Option<TaskDetailRead>, SourceError>> {
        for id in ids {
            self.prime_expected(id);
        }
        self.details(ids, Some(comments), true, |unread| {
            Box::pin(async move {
                self.inner
                    .get_task_details_for_copy(&unread, comments)
                    .await
            })
        })
        .await
    }

    async fn get_project(&self, id: &NativeId) -> Result<Option<Project>, SourceError> {
        self.inner.get_project(id).await
    }

    async fn query_tasks(
        &self,
        query: &TaskQuery,
        page: &PageRequest,
    ) -> Result<Page<Task>, SourceError> {
        let listed = self.inner.query_tasks(query, page).await?;
        let mut command = self.cache.command();
        for task in &listed.items {
            if let Some(version) = &task.version {
                command.listed.insert(task.id.clone(), version.clone());
            }
        }
        drop(command);
        Ok(listed)
    }

    async fn query_projects(
        &self,
        query: &ProjectQuery,
        page: &PageRequest,
    ) -> Result<Page<Project>, SourceError> {
        self.inner.query_projects(query, page).await
    }

    async fn labels(&self, page: &PageRequest) -> Result<Page<Label>, SourceError> {
        self.inner.labels(page).await
    }

    async fn task_dependencies(
        &self,
        id: &NativeId,
        direction: Direction,
        page: &PageRequest,
    ) -> Result<Page<DependencyEdge>, SourceError> {
        self.prime_expected(id);
        if direction == Direction::DependsOn && page.cursor.is_none() {
            if let Some(edges) = self
                .held(id)
                .and_then(|entry| entry.edges)
                .filter(|edges| edges.len() <= page.limit as usize)
            {
                return Ok(Page {
                    items: edges,
                    next: None,
                });
            }
            let read = self.inner.task_dependencies(id, direction, page).await?;
            if read.next.is_none() {
                self.complete(id, None, Some(&read.items));
            }
            return Ok(read);
        }
        self.inner.task_dependencies(id, direction, page).await
    }

    async fn project_dependencies(
        &self,
        id: &NativeId,
        direction: Direction,
        page: &PageRequest,
    ) -> Result<Page<DependencyEdge>, SourceError> {
        self.inner.project_dependencies(id, direction, page).await
    }

    fn writes(&self) -> WriteSupport {
        self.inner.writes()
    }

    async fn write_task(&self, write: &ItemWrite<Task>) -> Result<NativeId, SourceError> {
        self.write(write.target.as_ref(), || {
            Box::pin(self.inner.write_task(write))
        })
        .await
    }

    async fn write_project(&self, write: &ItemWrite<Project>) -> Result<NativeId, SourceError> {
        self.inner.write_project(write).await
    }

    async fn check_status_write(
        &self,
        kind: ItemKind,
        category: StatusCategory,
        target: Option<&NativeId>,
    ) -> Result<(), SourceError> {
        if let Some(target) = target {
            self.prime_expected(target);
        }
        self.inner.check_status_write(kind, category, target).await
    }

    async fn set_task_status(
        &self,
        id: &NativeId,
        category: StatusCategory,
    ) -> Result<Option<Status>, SourceError> {
        self.write(Some(id), || {
            Box::pin(self.inner.set_task_status(id, category))
        })
        .await
    }

    async fn set_task_status_reading(
        &self,
        id: &NativeId,
        category: StatusCategory,
    ) -> Result<Option<Task>, SourceError> {
        // An expected write reads the task it answers with from the cache, at the version
        // expected, rather than letting the source read it first: the two calls a source's own
        // answer is defined as, each through this cache.
        let expected = self
            .cache
            .command()
            .expected
            .as_ref()
            .is_some_and(|expected| expected.id == *id);
        if expected && self.servable(id, false).is_ok() {
            let Some(mut task) = self.get_task(id).await? else {
                return Ok(None);
            };
            let Some(status) = self.set_task_status(id, category).await? else {
                return Ok(None);
            };
            task.status = status;
            return Ok(Some(task));
        }
        self.write(Some(id), || {
            Box::pin(self.inner.set_task_status_reading(id, category))
        })
        .await
    }

    async fn set_task_priority(
        &self,
        id: &NativeId,
        priority: Priority,
    ) -> Result<Option<Priority>, SourceError> {
        self.write(Some(id), || {
            Box::pin(self.inner.set_task_priority(id, priority))
        })
        .await
    }

    async fn set_task_content(
        &self,
        id: &NativeId,
        content: &str,
    ) -> Result<Option<()>, SourceError> {
        self.write(Some(id), || {
            Box::pin(self.inner.set_task_content(id, content))
        })
        .await
    }

    async fn set_delivered_by(
        &self,
        id: &NativeId,
        delivered_by: &[TaskRef],
    ) -> Result<Option<()>, SourceError> {
        self.write(Some(id), || {
            Box::pin(self.inner.set_delivered_by(id, delivered_by))
        })
        .await
    }

    async fn set_task_metadata(
        &self,
        id: &NativeId,
        key: &MetadataKey,
        value: &Value,
    ) -> Result<Option<Task>, SourceError> {
        self.write(Some(id), || {
            Box::pin(self.inner.set_task_metadata(id, key, value))
        })
        .await
    }

    async fn update_task(
        &self,
        id: &NativeId,
        update: &TaskUpdate,
    ) -> Result<Option<TaskUpdateOutcome>, SourceError> {
        self.write(Some(id), || Box::pin(self.inner.update_task(id, update)))
            .await
    }

    async fn set_project_metadata(
        &self,
        id: &NativeId,
        key: &MetadataKey,
        value: &Value,
    ) -> Result<Option<Project>, SourceError> {
        self.inner.set_project_metadata(id, key, value).await
    }

    async fn set_document_metadata(
        &self,
        id: &NativeId,
        key: &MetadataKey,
        value: &Value,
    ) -> Result<Option<Document>, SourceError> {
        self.inner.set_document_metadata(id, key, value).await
    }

    async fn delete_task(&self, id: &NativeId) -> Result<(), SourceError> {
        self.write(Some(id), || Box::pin(self.inner.delete_task(id)))
            .await
    }

    async fn delete_project(&self, id: &NativeId) -> Result<(), SourceError> {
        self.inner.delete_project(id).await
    }

    async fn get_document(&self, id: &NativeId) -> Result<Option<Document>, SourceError> {
        self.inner.get_document(id).await
    }

    async fn query_documents(
        &self,
        query: &DocumentQuery,
        page: &PageRequest,
    ) -> Result<Page<Document>, SourceError> {
        self.inner.query_documents(query, page).await
    }

    async fn write_document(&self, write: &ItemWrite<Document>) -> Result<NativeId, SourceError> {
        self.inner.write_document(write).await
    }

    async fn delete_document(&self, id: &NativeId) -> Result<(), SourceError> {
        self.inner.delete_document(id).await
    }

    async fn task_comments(
        &self,
        task: &NativeId,
        page: &PageRequest,
    ) -> Result<Option<Page<Comment>>, SourceError> {
        if page.cursor.is_none()
            && let Ok(entry) = self.servable(task, true)
            && let Some(comments) = entry
                .comments
                .clone()
                .filter(|comments| comments.len() <= page.limit as usize)
        {
            self.served(&entry);
            return Ok(Some(Page {
                items: comments,
                next: None,
            }));
        }
        self.inner.task_comments(task, page).await
    }

    async fn add_comment(
        &self,
        task: &NativeId,
        comment: &NewComment,
    ) -> Result<Option<Comment>, SourceError> {
        self.write(Some(task), || {
            Box::pin(self.inner.add_comment(task, comment))
        })
        .await
    }

    async fn edit_comment(
        &self,
        task: &NativeId,
        comment: &NativeId,
        body: &CommentBody,
    ) -> Result<Option<Comment>, SourceError> {
        self.write(Some(task), || {
            Box::pin(self.inner.edit_comment(task, comment, body))
        })
        .await
    }

    async fn delete_comment(
        &self,
        task: &NativeId,
        comment: &NativeId,
    ) -> Result<Option<NativeId>, SourceError> {
        self.write(Some(task), || {
            Box::pin(self.inner.delete_comment(task, comment))
        })
        .await
    }

    async fn write_comment_copy(
        &self,
        task: &NativeId,
        existing: Option<&NativeId>,
        copy: &CommentCopy,
    ) -> Result<Option<Comment>, SourceError> {
        self.write(Some(task), || {
            Box::pin(self.inner.write_comment_copy(task, existing, copy))
        })
        .await
    }

    fn keeps_template_answers(&self) -> bool {
        self.inner.keeps_template_answers()
    }

    async fn task_template_answers(
        &self,
        id: &NativeId,
    ) -> Result<Option<BTreeMap<String, Value>>, SourceError> {
        self.inner.task_template_answers(id).await
    }

    async fn document_template_answers(
        &self,
        id: &NativeId,
    ) -> Result<Option<BTreeMap<String, Value>>, SourceError> {
        self.inner.document_template_answers(id).await
    }

    async fn project_template_answers(
        &self,
        id: &NativeId,
    ) -> Result<Option<BTreeMap<String, Value>>, SourceError> {
        self.inner.project_template_answers(id).await
    }

    async fn write_task_rendered(
        &self,
        write: &ItemWrite<Task>,
        answers: &BTreeMap<String, Value>,
    ) -> Result<NativeId, SourceError> {
        self.write(write.target.as_ref(), || {
            Box::pin(self.inner.write_task_rendered(write, answers))
        })
        .await
    }

    async fn write_document_rendered(
        &self,
        write: &ItemWrite<Document>,
        answers: &BTreeMap<String, Value>,
    ) -> Result<NativeId, SourceError> {
        self.inner.write_document_rendered(write, answers).await
    }

    async fn write_project_rendered(
        &self,
        write: &ItemWrite<Project>,
        answers: &BTreeMap<String, Value>,
    ) -> Result<NativeId, SourceError> {
        self.inner.write_project_rendered(write, answers).await
    }

    async fn set_task_rendering(
        &self,
        id: &NativeId,
        content: &str,
        provenance: &Value,
        answers: &BTreeMap<String, Value>,
    ) -> Result<Option<()>, SourceError> {
        self.write(Some(id), || {
            Box::pin(
                self.inner
                    .set_task_rendering(id, content, provenance, answers),
            )
        })
        .await
    }

    async fn set_document_rendering(
        &self,
        id: &NativeId,
        content: &str,
        provenance: &Value,
        answers: &BTreeMap<String, Value>,
    ) -> Result<Option<()>, SourceError> {
        self.inner
            .set_document_rendering(id, content, provenance, answers)
            .await
    }

    async fn set_project_rendering(
        &self,
        id: &NativeId,
        content: &str,
        provenance: &Value,
        answers: &BTreeMap<String, Value>,
    ) -> Result<Option<()>, SourceError> {
        self.inner
            .set_project_rendering(id, content, provenance, answers)
            .await
    }

    async fn task_assets(&self, id: &NativeId) -> Result<Vec<Asset>, SourceError> {
        self.prime_expected(id);
        if let Some(assets) = self.held(id).and_then(|entry| entry.assets) {
            return Ok(assets);
        }
        let read = self.inner.task_assets(id).await?;
        self.complete(id, Some(&read), None);
        Ok(read)
    }

    async fn document_assets(&self, id: &NativeId) -> Result<Vec<Asset>, SourceError> {
        self.inner.document_assets(id).await
    }

    async fn task_asset(
        &self,
        id: &NativeId,
        name: &AssetName,
    ) -> Result<Option<Vec<u8>>, SourceError> {
        self.inner.task_asset(id, name).await
    }

    async fn document_asset(
        &self,
        id: &NativeId,
        name: &AssetName,
    ) -> Result<Option<Vec<u8>>, SourceError> {
        self.inner.document_asset(id, name).await
    }

    async fn write_task_with_assets(
        &self,
        write: &ItemWrite<Task>,
        answers: Option<&BTreeMap<String, Value>>,
        assets: &AssetWrite,
    ) -> Result<AssetsWritten, SourceError> {
        self.write(write.target.as_ref(), || {
            Box::pin(self.inner.write_task_with_assets(write, answers, assets))
        })
        .await
    }

    async fn write_document_with_assets(
        &self,
        write: &ItemWrite<Document>,
        answers: Option<&BTreeMap<String, Value>>,
        assets: &AssetWrite,
    ) -> Result<AssetsWritten, SourceError> {
        self.inner
            .write_document_with_assets(write, answers, assets)
            .await
    }

    async fn set_task_rendering_with_assets(
        &self,
        id: &NativeId,
        content: &str,
        provenance: &Value,
        answers: &BTreeMap<String, Value>,
        assets: &AssetWrite,
    ) -> Result<Option<AssetsWritten>, SourceError> {
        self.write(Some(id), || {
            Box::pin(
                self.inner
                    .set_task_rendering_with_assets(id, content, provenance, answers, assets),
            )
        })
        .await
    }

    async fn set_document_rendering_with_assets(
        &self,
        id: &NativeId,
        content: &str,
        provenance: &Value,
        answers: &BTreeMap<String, Value>,
        assets: &AssetWrite,
    ) -> Result<Option<AssetsWritten>, SourceError> {
        self.inner
            .set_document_rendering_with_assets(id, content, provenance, answers, assets)
            .await
    }

    async fn metering(&self) -> Result<Option<Metering>, SourceError> {
        self.inner.metering().await
    }

    async fn end_command(&self) -> Result<(), SourceError> {
        self.cache.end_command();
        self.inner.end_command().await
    }

    async fn visibility(&self, target: &WriteTarget<'_>) -> Result<Visibility, SourceError> {
        self.inner.visibility(target).await
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use onetaskgraph_in_memory::{InMemoryConfig, InMemorySource};
    use onetaskgraph_plugin_api::{JournalEntry, RequestKind, Shown, StatusCategory, WholeTask};

    use super::super::CacheLocation;
    use super::*;

    /// An in-memory source whose read by id answers `answer`, whatever id it was asked for.
    struct Answering {
        inner: InMemorySource,
        answer: Task,
    }

    #[async_trait::async_trait]
    impl TaskSource for Answering {
        fn kind(&self) -> &'static str {
            self.inner.kind()
        }
        fn capabilities(&self) -> Capabilities {
            self.inner.capabilities()
        }
        async fn health(&self) -> Result<Health, SourceError> {
            self.inner.health().await
        }
        async fn get_task(&self, _id: &NativeId) -> Result<Option<Task>, SourceError> {
            Ok(Some(self.answer.clone()))
        }
        async fn get_project(&self, id: &NativeId) -> Result<Option<Project>, SourceError> {
            self.inner.get_project(id).await
        }
        async fn query_tasks(
            &self,
            query: &TaskQuery,
            page: &PageRequest,
        ) -> Result<Page<Task>, SourceError> {
            self.inner.query_tasks(query, page).await
        }
        async fn query_projects(
            &self,
            query: &ProjectQuery,
            page: &PageRequest,
        ) -> Result<Page<Project>, SourceError> {
            self.inner.query_projects(query, page).await
        }
        async fn labels(&self, page: &PageRequest) -> Result<Page<Label>, SourceError> {
            self.inner.labels(page).await
        }
        async fn task_dependencies(
            &self,
            id: &NativeId,
            direction: Direction,
            page: &PageRequest,
        ) -> Result<Page<DependencyEdge>, SourceError> {
            self.inner.task_dependencies(id, direction, page).await
        }
        async fn project_dependencies(
            &self,
            id: &NativeId,
            direction: Direction,
            page: &PageRequest,
        ) -> Result<Page<DependencyEdge>, SourceError> {
            self.inner.project_dependencies(id, direction, page).await
        }
    }

    fn id(value: &str) -> NativeId {
        NativeId::from(value)
    }

    fn task(of: &str, version: &str) -> Task {
        Task {
            version: Some(version.to_owned()),
            id: id(of),
            key: None,
            title: format!("Task {of}"),
            content: None,
            status: Status {
                category: StatusCategory::Todo,
                name: "Todo".to_owned(),
            },
            priority: Priority::None,
            labels: Vec::new(),
            project: None,
            url: None,
            location: None,
            created_at: None,
            updated_at: None,
            metadata: BTreeMap::new(),
            repositories: Vec::new(),
            delivers: Vec::new(),
            delivered_by: Vec::new(),
            classification: Default::default(),
        }
    }

    /// A caching source over a source whose every read by id answers `answer`, in `dir`.
    fn cached(dir: &std::path::Path, answer: Task) -> (CachedSource, Arc<SourceCache>) {
        let cache = SourceCache::new(
            CacheLocation {
                dir: dir.to_path_buf(),
                remote: "remote".to_owned(),
            },
            VersionCoverage::whole(),
        );
        let inner = Answering {
            inner: InMemorySource::new(InMemoryConfig::default()).expect("an empty source"),
            answer,
        };
        (CachedSource::new(Box::new(inner), cache.clone()), cache)
    }

    #[tokio::test]
    async fn a_read_answered_with_another_task_writes_no_entry() {
        let dir = tempfile::tempdir().expect("a directory");
        let (source, cache) = cached(dir.path(), task("T-2", "v2"));
        source.get_task(&id("T-1")).await.expect("the read");
        assert!(cache.read(&id("T-1")).is_none());
        assert!(cache.read(&id("T-2")).is_none());
        assert!(!cache.command().read.contains_key(&id("T-1")));
    }

    #[tokio::test]
    async fn a_read_answered_with_its_own_task_writes_its_entry() {
        let dir = tempfile::tempdir().expect("a directory");
        let (source, cache) = cached(dir.path(), task("T-1", "v1"));
        source.get_task(&id("T-1")).await.expect("the read");
        assert_eq!(
            cache.read(&id("T-1")).and_then(|entry| entry.task.version),
            Some("v1".to_owned())
        );
    }

    fn journal(shown: Task) -> WriteJournal {
        WriteJournal {
            requests: vec![JournalEntry {
                part: "issue".to_owned(),
                kind: RequestKind::Mutation,
                outcome: PartOutcome::Landed,
                fields: BTreeMap::new(),
                shown: Shown::Whole {
                    task: Box::new(
                        WholeTask::new(shown, Vec::new(), Vec::new(), None).expect("a version"),
                    ),
                },
            }],
        }
    }

    #[test]
    fn a_write_whose_response_shows_another_task_advances_no_entry() {
        let dir = tempfile::tempdir().expect("a directory");
        let (source, cache) = cached(dir.path(), task("T-1", "v1"));
        cache.write(&Entry {
            schema: ENTRY_SCHEMA,
            remote: "remote".to_owned(),
            task: task("T-1", "v1"),
            comments: Some(Vec::new()),
            assets: None,
            edges: None,
            context: None,
        });
        let written = source.settle(
            &id("T-1"),
            Some(&journal(task("T-2", "v2"))),
            String::new(),
            None,
            None,
        );
        assert!(
            cache.read(&id("T-1")).is_none(),
            "this task's entry is left unserveable"
        );
        assert!(
            cache.read(&id("T-2")).is_none(),
            "nothing is written for the other task"
        );
        assert_eq!(written.result.map(|result| result.parts.len()), Some(1));
    }

    #[test]
    fn a_write_whose_response_shows_its_own_task_whole_advances_its_entry() {
        let dir = tempfile::tempdir().expect("a directory");
        let (source, cache) = cached(dir.path(), task("T-1", "v1"));
        source.settle(
            &id("T-1"),
            Some(&journal(task("T-1", "v2"))),
            String::new(),
            None,
            None,
        );
        assert_eq!(
            cache.read(&id("T-1")).and_then(|entry| entry.task.version),
            Some("v2".to_owned())
        );
    }

    /// The entry the tests below start from: `T-1` at `v1`, served.
    fn held_at_v1(cache: &SourceCache) {
        cache.write(&Entry {
            schema: ENTRY_SCHEMA,
            remote: "remote".to_owned(),
            task: task("T-1", "v1"),
            comments: Some(Vec::new()),
            assets: None,
            edges: None,
            context: None,
        });
        assert!(cache.read(&id("T-1")).is_some(), "the entry starts served");
    }

    /// Another process's view of the same cache directory.
    fn another_process(dir: &std::path::Path) -> Arc<SourceCache> {
        SourceCache::new(
            CacheLocation {
                dir: dir.to_path_buf(),
                remote: "remote".to_owned(),
            },
            VersionCoverage::whole(),
        )
    }

    /// Sets a path's mode for as long as it lives, and gives the old one back.
    #[cfg(unix)]
    struct Mode {
        path: std::path::PathBuf,
        was: u32,
    }

    #[cfg(unix)]
    impl Mode {
        fn set(path: &std::path::Path, mode: u32) -> Self {
            use std::os::unix::fs::PermissionsExt as _;
            let was = std::fs::metadata(path)
                .expect("the path")
                .permissions()
                .mode();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
                .expect("the mode");
            Self {
                path: path.to_path_buf(),
                was,
            }
        }
    }

    #[cfg(unix)]
    impl Drop for Mode {
        fn drop(&mut self) {
            use std::os::unix::fs::PermissionsExt as _;
            let _ = std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(self.was));
        }
    }

    /// Make the entries' directory refuse every removal, rename and new file, answering `None`
    /// on a host where that does not take — a superuser's — after saying so.
    #[cfg(unix)]
    fn refusing(dir: &std::path::Path) -> Option<Mode> {
        refusing_in(&dir.join("tasks"))
    }

    /// Make `directory` refuse every removal, rename and new file, as [`refusing`] does.
    #[cfg(unix)]
    fn refusing_in(directory: &std::path::Path) -> Option<Mode> {
        let mode = Mode::set(directory, 0o555);
        if std::fs::File::create(directory.join("probe")).is_ok() {
            eprintln!(
                "skipped: this host lets a read-only directory be written, so no removal can be \
                 made to fail here"
            );
            return None;
        }
        Some(mode)
    }

    fn entry_file(dir: &std::path::Path) -> std::path::PathBuf {
        dir.join("tasks")
            .join(format!("{}.json", super::super::digest("T-1")))
    }

    #[cfg(unix)]
    #[test]
    fn an_invalidation_the_filesystem_will_not_remove_still_serves_the_old_entry_to_no_one() {
        let dir = tempfile::tempdir().expect("a directory");
        let (source, cache) = cached(dir.path(), task("T-1", "v1"));
        held_at_v1(&cache);
        let Some(_refusing) = refusing(dir.path()) else {
            return;
        };
        // A response showing another task cannot vouch for this one, so it invalidates.
        source.settle(
            &id("T-1"),
            Some(&journal(task("T-2", "v2"))),
            String::new(),
            None,
            None,
        );
        assert!(
            entry_file(dir.path()).exists(),
            "the removal was refused, so the file is still there"
        );
        assert!(cache.read(&id("T-1")).is_none(), "this process serves it");
        assert!(
            another_process(dir.path()).read(&id("T-1")).is_none(),
            "another process serves it"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_write_the_filesystem_will_not_take_leaves_the_old_entry_served_to_no_one() {
        let dir = tempfile::tempdir().expect("a directory");
        let (source, cache) = cached(dir.path(), task("T-1", "v1"));
        held_at_v1(&cache);
        let refused = refusing(dir.path());
        if refused.is_none() {
            return;
        }
        // A response showing the task whole at `v2`, whose entry cannot be written.
        source.settle(
            &id("T-1"),
            Some(&journal(task("T-1", "v2"))),
            String::new(),
            None,
            None,
        );
        assert!(cache.read(&id("T-1")).is_none(), "this process serves `v1`");
        assert!(
            another_process(dir.path()).read(&id("T-1")).is_none(),
            "another process serves `v1`"
        );
        // Once the directory takes writes again, the next one lands and is served.
        drop(refused);
        source.settle(
            &id("T-1"),
            Some(&journal(task("T-1", "v3"))),
            String::new(),
            None,
            None,
        );
        assert_eq!(
            another_process(dir.path())
                .read(&id("T-1"))
                .and_then(|entry| entry.task.version),
            Some("v3".to_owned())
        );
    }

    #[cfg(unix)]
    #[test]
    fn an_entry_whose_own_mode_refuses_writes_is_still_taken_out_of_service() {
        let dir = tempfile::tempdir().expect("a directory");
        let (_, cache) = cached(dir.path(), task("T-1", "v1"));
        held_at_v1(&cache);
        let _read_only = Mode::set(&entry_file(dir.path()), 0o444);
        let Some(_refusing) = refusing(dir.path()) else {
            return;
        };
        cache.invalidate(&id("T-1"));
        assert!(
            another_process(dir.path()).read(&id("T-1")).is_none(),
            "another process serves it"
        );
    }

    #[cfg(unix)]
    #[test]
    fn an_entry_nothing_can_remove_or_empty_is_withheld_from_this_process_until_rewritten() {
        let dir = tempfile::tempdir().expect("a directory");
        let (_, cache) = cached(dir.path(), task("T-1", "v1"));
        held_at_v1(&cache);
        let refused = refusing(dir.path());
        if refused.is_none() {
            return;
        }
        let path = entry_file(dir.path());
        // Removal is refused by the directory, and emptying by a filesystem that will not
        // write the file at all.
        cache.retire_with(&path, |_| {
            Err(std::io::Error::from(std::io::ErrorKind::ReadOnlyFilesystem))
        });
        assert!(path.exists(), "the file is as it was");
        assert!(
            cache.read(&id("T-1")).is_none(),
            "this process withholds it from itself"
        );
        // A write of its own landing there gives it back.
        drop(refused);
        cache.write(&Entry {
            schema: ENTRY_SCHEMA,
            remote: "remote".to_owned(),
            task: task("T-1", "v2"),
            comments: Some(Vec::new()),
            assets: None,
            edges: None,
            context: None,
        });
        assert_eq!(
            cache.read(&id("T-1")).and_then(|entry| entry.task.version),
            Some("v2".to_owned()),
            "the withholding process serves what it wrote"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_board_context_the_filesystem_will_not_replace_or_drop_is_served_to_no_one() {
        let dir = tempfile::tempdir().expect("a directory");
        let (_, cache) = cached(dir.path(), task("T-1", "v1"));
        cache.write_board(serde_json::json!({"option": "stale"}));
        assert!(cache.board().is_some(), "the board starts served");
        {
            let Some(_refusing) = refusing_in(dir.path()) else {
                return;
            };
            cache.write_board(serde_json::json!({"option": "fresh"}));
            assert!(
                dir.path().join("board.json").exists(),
                "the file is still there"
            );
            assert!(
                cache.board().is_none(),
                "the replaced context is not served"
            );
            assert!(
                another_process(dir.path()).board().is_none(),
                "nor to another process"
            );
        }
        cache.write_board(serde_json::json!({"option": "stale"}));
        let Some(_refusing) = refusing_in(dir.path()) else {
            return;
        };
        cache.drop_board();
        assert!(cache.board().is_none(), "the dropped context is not served");
        assert!(
            another_process(dir.path()).board().is_none(),
            "nor to another process"
        );
    }

    /// Runs on every host; on Windows, where a file's read-only attribute refuses its removal,
    /// it is what drives clearing that attribute before the file is emptied.
    #[test]
    fn an_entry_marked_read_only_is_still_taken_out_of_service() {
        let dir = tempfile::tempdir().expect("a directory");
        let (_, cache) = cached(dir.path(), task("T-1", "v1"));
        held_at_v1(&cache);
        let path = entry_file(dir.path());
        let mut permissions = std::fs::metadata(&path).expect("the entry").permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&path, permissions).expect("read-only");
        cache.invalidate(&id("T-1"));
        assert!(
            another_process(dir.path()).read(&id("T-1")).is_none(),
            "another process serves it"
        );
    }
}
