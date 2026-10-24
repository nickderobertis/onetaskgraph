//! A source that answered rate limited is sent nothing more until the instant it named.
//!
//! A hosted tracker's allowance is shared with everything else that spends its key, and a
//! request sent before the reset it asked for is one that allowance pays for only to be refused
//! again. So within one engine instance, once a source answers [`SourceError::RateLimited`]
//! carrying `retry_after_seconds`, every call to that source before that instant fails here,
//! locally and without a request, with the same `RateLimited` carrying the seconds still to
//! wait — which is how a caller re-reading a node it holds learns the reset off any later
//! failure, and why that re-read costs the source nothing. The first call at or after the
//! instant reaches the source again.
//!
//! It is implemented once, here, because every plugin's calls pass through it: the engine
//! puts each ready source behind a [`HeldOff`] as it is assembled — outside the store's own
//! cache when one is configured, so every call before the reset fails locally, as the rule
//! says, whatever would have answered it. A rate limit naming no wait is passed through and
//! remembered not at all: there is no instant to hold the source off until.
//!
//! Every method of [`TaskSource`] is forwarded, the ones that cannot reach the source (the
//! synchronous ones, and `end_command`, which only drops what the source holds) unconditionally.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

use onetaskgraph_plugin_api::{
    Asset, AssetName, AssetWrite, AssetsWritten, Capabilities, Comment, CommentBody, CommentCopy,
    DependencyEdge, Direction, Document, DocumentQuery, DocumentVersionCoverage, Health,
    IssueHolding, IssueRef, ItemKind, ItemWrite, Label, Location, MetadataKey, MetadataRecord,
    Metering, NativeId, NewComment, Page, PageRequest, Priority, Project, ProjectQuery,
    ProjectVersionCoverage, SharedClock, SourceError, SourceName, Status, StatusCategory,
    StatusContradiction, Task, TaskDetailRead, TaskQuery, TaskRef, TaskSource, TaskUpdate,
    TaskUpdateOutcome, VersionCoverage, Visibility, WriteJournal, WriteSupport, WriteTarget,
};
use serde_json::Value;

/// One source, sent nothing between a rate limit it answered and the reset that limit named.
pub(crate) struct HeldOff {
    inner: Box<dyn TaskSource>,
    name: SourceName,
    clock: SharedClock,
    /// The instant, on [`Self::clock`], before which this source is sent nothing.
    until: Mutex<Option<Duration>>,
}

impl HeldOff {
    pub(crate) fn new(name: SourceName, inner: Box<dyn TaskSource>, clock: SharedClock) -> Self {
        Self {
            inner,
            name,
            clock,
            until: Mutex::new(None),
        }
    }

    fn until(&self) -> std::sync::MutexGuard<'_, Option<Duration>> {
        self.until
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Refuse, without sending anything, a call made before the reset this source asked for,
    /// carrying the whole seconds still to wait — at least one, so a refusal never says to
    /// wait for nothing.
    fn refused(&self) -> Result<(), SourceError> {
        let mut until = self.until();
        let Some(reset) = *until else {
            return Ok(());
        };
        let now = self.clock.now();
        if now >= reset {
            *until = None;
            return Ok(());
        }
        let left = reset - now;
        let seconds = left.as_secs() + u64::from(left.subsec_nanos() > 0);
        Err(SourceError::RateLimited {
            retry_after_seconds: Some(seconds.max(1)),
            message: Some(format!(
                "not sent: source {} answered rate limited earlier in this process, and it is \
                 sent nothing before the reset it named, {} s from now; next: retry after that",
                self.name,
                seconds.max(1)
            )),
        })
    }

    /// A batched read, refused for every id it names before the reset and each answer noted
    /// after it.
    async fn each_noted(
        &self,
        ids: &[NativeId],
        reading: impl std::future::Future<Output = Vec<Result<Option<TaskDetailRead>, SourceError>>>,
    ) -> Vec<Result<Option<TaskDetailRead>, SourceError>> {
        if let Err(refused) = self.refused() {
            return ids.iter().map(|_| Err(refused.clone())).collect();
        }
        reading
            .await
            .into_iter()
            .map(|answered| self.noted(answered))
            .collect()
    }

    /// Remember the reset a rate limit named, the later of it and one already held.
    fn noted<T>(&self, answered: Result<T, SourceError>) -> Result<T, SourceError> {
        if let Err(SourceError::RateLimited {
            retry_after_seconds: Some(seconds),
            ..
        }) = &answered
            && *seconds > 0
        {
            // A wait too long for the clock to represent holds the source off for as long as
            // it can say, rather than overflowing.
            let reset = self
                .clock
                .now()
                .checked_add(Duration::from_secs(*seconds))
                .unwrap_or(Duration::MAX);
            let mut until = self.until();
            *until = Some(until.map_or(reset, |held| held.max(reset)));
        }
        answered
    }
}

#[async_trait::async_trait]
impl TaskSource for HeldOff {
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

    fn copy_version_coverage(&self) -> VersionCoverage {
        self.inner.copy_version_coverage()
    }

    fn project_version_coverage(&self) -> ProjectVersionCoverage {
        self.inner.project_version_coverage()
    }

    fn document_version_coverage(&self) -> DocumentVersionCoverage {
        self.inner.document_version_coverage()
    }

    fn attach_cache(&self) {
        self.inner.attach_cache()
    }

    fn forward_edges_version(&self, id: &NativeId) -> Option<String> {
        self.inner.forward_edges_version(id)
    }

    fn edges_version(&self, kind: ItemKind, id: &NativeId, direction: Direction) -> Option<String> {
        self.inner.edges_version(kind, id, direction)
    }

    fn reverse_edges_coverage(&self, kind: ItemKind) -> onetaskgraph_plugin_api::Coverage {
        self.inner.reverse_edges_coverage(kind)
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
        self.inner.unprime(id)
    }

    fn begin_journal(&self) {
        self.inner.begin_journal()
    }

    fn end_journal(&self) -> Option<WriteJournal> {
        self.inner.end_journal()
    }

    async fn health(&self) -> Result<Health, SourceError> {
        self.refused()?;
        self.noted(self.inner.health().await)
    }

    async fn designator(&self) -> Result<Option<onetaskgraph_plugin_api::Designator>, SourceError> {
        self.refused()?;
        self.noted(self.inner.designator().await)
    }

    async fn get_task(&self, id: &NativeId) -> Result<Option<Task>, SourceError> {
        self.refused()?;
        self.noted(self.inner.get_task(id).await)
    }

    async fn get_task_details(
        &self,
        ids: &[NativeId],
        comments: Option<&PageRequest>,
    ) -> Vec<Result<Option<TaskDetailRead>, SourceError>> {
        self.each_noted(ids, self.inner.get_task_details(ids, comments))
            .await
    }

    async fn get_task_details_for_copy(
        &self,
        ids: &[NativeId],
        comments: &PageRequest,
    ) -> Vec<Result<Option<TaskDetailRead>, SourceError>> {
        self.each_noted(ids, self.inner.get_task_details_for_copy(ids, comments))
            .await
    }

    async fn read_ahead_for_copy(&self, ids: &[NativeId]) -> Result<(), SourceError> {
        self.refused()?;
        self.noted(self.inner.read_ahead_for_copy(ids).await)
    }

    async fn read_ahead_origins_for_copy(&self, origins: &[String]) -> Result<(), SourceError> {
        self.refused()?;
        self.noted(self.inner.read_ahead_origins_for_copy(origins).await)
    }

    fn create_batch_limit(&self) -> usize {
        self.inner.create_batch_limit()
    }

    async fn write_tasks(&self, writes: &[ItemWrite<Task>]) -> Result<Vec<NativeId>, SourceError> {
        self.refused()?;
        self.noted(self.inner.write_tasks(writes).await)
    }

    fn begin_deferred_writes(&self) {
        self.inner.begin_deferred_writes()
    }

    async fn finish_writes(&self) -> Result<(), SourceError> {
        self.refused()?;
        self.noted(self.inner.finish_writes().await)
    }

    fn written_location(&self, record: MetadataRecord, id: &NativeId) -> Option<Location> {
        self.inner.written_location(record, id)
    }

    async fn get_project(&self, id: &NativeId) -> Result<Option<Project>, SourceError> {
        self.refused()?;
        self.noted(self.inner.get_project(id).await)
    }

    async fn query_tasks(
        &self,
        query: &TaskQuery,
        page: &PageRequest,
    ) -> Result<Page<Task>, SourceError> {
        self.refused()?;
        self.noted(self.inner.query_tasks(query, page).await)
    }

    async fn query_projects(
        &self,
        query: &ProjectQuery,
        page: &PageRequest,
    ) -> Result<Page<Project>, SourceError> {
        self.refused()?;
        self.noted(self.inner.query_projects(query, page).await)
    }

    async fn query_projects_by_origin(
        &self,
        origin: &str,
        page: &PageRequest,
    ) -> Result<Page<Project>, SourceError> {
        self.refused()?;
        self.noted(self.inner.query_projects_by_origin(origin, page).await)
    }

    async fn labels(&self, page: &PageRequest) -> Result<Page<Label>, SourceError> {
        self.refused()?;
        self.noted(self.inner.labels(page).await)
    }

    async fn task_dependencies(
        &self,
        id: &NativeId,
        direction: Direction,
        page: &PageRequest,
    ) -> Result<Page<DependencyEdge>, SourceError> {
        self.refused()?;
        self.noted(self.inner.task_dependencies(id, direction, page).await)
    }

    async fn project_dependencies(
        &self,
        id: &NativeId,
        direction: Direction,
        page: &PageRequest,
    ) -> Result<Page<DependencyEdge>, SourceError> {
        self.refused()?;
        self.noted(self.inner.project_dependencies(id, direction, page).await)
    }

    fn writes(&self) -> WriteSupport {
        self.inner.writes()
    }

    async fn stored_form(&self, content: &str) -> Result<String, SourceError> {
        self.refused()?;
        self.noted(self.inner.stored_form(content).await)
    }

    async fn stored_status(&self, kind: ItemKind, status: &Status) -> Result<Status, SourceError> {
        self.refused()?;
        self.noted(self.inner.stored_status(kind, status).await)
    }

    fn stored_content(&self, record: MetadataRecord, id: &NativeId) -> Option<String> {
        self.inner.stored_content(record, id)
    }

    async fn write_task(&self, write: &ItemWrite<Task>) -> Result<NativeId, SourceError> {
        self.refused()?;
        self.noted(self.inner.write_task(write).await)
    }

    async fn write_project(&self, write: &ItemWrite<Project>) -> Result<NativeId, SourceError> {
        self.refused()?;
        self.noted(self.inner.write_project(write).await)
    }

    async fn check_status_write(
        &self,
        kind: ItemKind,
        category: StatusCategory,
        target: Option<&NativeId>,
    ) -> Result<(), SourceError> {
        self.refused()?;
        self.noted(self.inner.check_status_write(kind, category, target).await)
    }

    fn status_as_written(&self, kind: ItemKind, status: &Status) -> Status {
        self.inner.status_as_written(kind, status)
    }

    async fn set_task_status(
        &self,
        id: &NativeId,
        category: StatusCategory,
    ) -> Result<Option<Status>, SourceError> {
        self.refused()?;
        self.noted(self.inner.set_task_status(id, category).await)
    }

    async fn status_contradictions(&self) -> Result<Vec<StatusContradiction>, SourceError> {
        self.refused()?;
        self.noted(self.inner.status_contradictions().await)
    }

    async fn check_task_write(&self, write: &ItemWrite<Task>) -> Result<(), SourceError> {
        self.refused()?;
        self.noted(self.inner.check_task_write(write).await)
    }

    async fn issue_holdings(&self, issues: &[IssueRef]) -> Result<Vec<IssueHolding>, SourceError> {
        self.refused()?;
        self.noted(self.inner.issue_holdings(issues).await)
    }

    async fn set_task_status_reading(
        &self,
        id: &NativeId,
        category: StatusCategory,
    ) -> Result<Option<Task>, SourceError> {
        self.refused()?;
        self.noted(self.inner.set_task_status_reading(id, category).await)
    }

    async fn set_task_priority(
        &self,
        id: &NativeId,
        priority: Priority,
    ) -> Result<Option<Priority>, SourceError> {
        self.refused()?;
        self.noted(self.inner.set_task_priority(id, priority).await)
    }

    async fn set_task_content(
        &self,
        id: &NativeId,
        content: &str,
    ) -> Result<Option<()>, SourceError> {
        self.refused()?;
        self.noted(self.inner.set_task_content(id, content).await)
    }

    async fn set_delivered_by(
        &self,
        id: &NativeId,
        delivered_by: &[TaskRef],
    ) -> Result<Option<()>, SourceError> {
        self.refused()?;
        self.noted(self.inner.set_delivered_by(id, delivered_by).await)
    }

    async fn set_task_metadata(
        &self,
        id: &NativeId,
        key: &MetadataKey,
        value: &Value,
    ) -> Result<Option<Task>, SourceError> {
        self.refused()?;
        self.noted(self.inner.set_task_metadata(id, key, value).await)
    }

    async fn update_task(
        &self,
        id: &NativeId,
        update: &TaskUpdate,
    ) -> Result<Option<TaskUpdateOutcome>, SourceError> {
        self.refused()?;
        self.noted(self.inner.update_task(id, update).await)
    }

    async fn set_project_status(
        &self,
        id: &NativeId,
        category: StatusCategory,
    ) -> Result<Option<Status>, SourceError> {
        self.refused()?;
        self.noted(self.inner.set_project_status(id, category).await)
    }

    async fn set_project_metadata(
        &self,
        id: &NativeId,
        key: &MetadataKey,
        value: &Value,
    ) -> Result<Option<Project>, SourceError> {
        self.refused()?;
        self.noted(self.inner.set_project_metadata(id, key, value).await)
    }

    async fn set_document_metadata(
        &self,
        id: &NativeId,
        key: &MetadataKey,
        value: &Value,
    ) -> Result<Option<Document>, SourceError> {
        self.refused()?;
        self.noted(self.inner.set_document_metadata(id, key, value).await)
    }

    async fn delete_task(&self, id: &NativeId) -> Result<(), SourceError> {
        self.refused()?;
        self.noted(self.inner.delete_task(id).await)
    }

    async fn delete_project(&self, id: &NativeId) -> Result<(), SourceError> {
        self.refused()?;
        self.noted(self.inner.delete_project(id).await)
    }

    async fn get_document(&self, id: &NativeId) -> Result<Option<Document>, SourceError> {
        self.refused()?;
        self.noted(self.inner.get_document(id).await)
    }

    async fn query_documents(
        &self,
        query: &DocumentQuery,
        page: &PageRequest,
    ) -> Result<Page<Document>, SourceError> {
        self.refused()?;
        self.noted(self.inner.query_documents(query, page).await)
    }

    async fn query_documents_by_origin(
        &self,
        origin: &str,
        page: &PageRequest,
    ) -> Result<Page<Document>, SourceError> {
        self.refused()?;
        self.noted(self.inner.query_documents_by_origin(origin, page).await)
    }

    async fn write_document(&self, write: &ItemWrite<Document>) -> Result<NativeId, SourceError> {
        self.refused()?;
        self.noted(self.inner.write_document(write).await)
    }

    async fn delete_document(&self, id: &NativeId) -> Result<(), SourceError> {
        self.refused()?;
        self.noted(self.inner.delete_document(id).await)
    }

    async fn task_comments(
        &self,
        task: &NativeId,
        page: &PageRequest,
    ) -> Result<Option<Page<Comment>>, SourceError> {
        self.refused()?;
        self.noted(self.inner.task_comments(task, page).await)
    }

    async fn add_comment(
        &self,
        task: &NativeId,
        comment: &NewComment,
    ) -> Result<Option<Comment>, SourceError> {
        self.refused()?;
        self.noted(self.inner.add_comment(task, comment).await)
    }

    async fn edit_comment(
        &self,
        task: &NativeId,
        comment: &NativeId,
        body: &CommentBody,
    ) -> Result<Option<Comment>, SourceError> {
        self.refused()?;
        self.noted(self.inner.edit_comment(task, comment, body).await)
    }

    async fn delete_comment(
        &self,
        task: &NativeId,
        comment: &NativeId,
    ) -> Result<Option<NativeId>, SourceError> {
        self.refused()?;
        self.noted(self.inner.delete_comment(task, comment).await)
    }

    async fn write_comment_copy(
        &self,
        task: &NativeId,
        existing: Option<&NativeId>,
        copy: &CommentCopy,
    ) -> Result<Option<Comment>, SourceError> {
        self.refused()?;
        self.noted(self.inner.write_comment_copy(task, existing, copy).await)
    }

    fn keeps_template_answers(&self) -> bool {
        self.inner.keeps_template_answers()
    }

    async fn task_template_answers(
        &self,
        id: &NativeId,
    ) -> Result<Option<BTreeMap<String, Value>>, SourceError> {
        self.refused()?;
        self.noted(self.inner.task_template_answers(id).await)
    }

    async fn document_template_answers(
        &self,
        id: &NativeId,
    ) -> Result<Option<BTreeMap<String, Value>>, SourceError> {
        self.refused()?;
        self.noted(self.inner.document_template_answers(id).await)
    }

    async fn project_template_answers(
        &self,
        id: &NativeId,
    ) -> Result<Option<BTreeMap<String, Value>>, SourceError> {
        self.refused()?;
        self.noted(self.inner.project_template_answers(id).await)
    }

    async fn write_task_rendered(
        &self,
        write: &ItemWrite<Task>,
        answers: &BTreeMap<String, Value>,
    ) -> Result<NativeId, SourceError> {
        self.refused()?;
        self.noted(self.inner.write_task_rendered(write, answers).await)
    }

    async fn write_document_rendered(
        &self,
        write: &ItemWrite<Document>,
        answers: &BTreeMap<String, Value>,
    ) -> Result<NativeId, SourceError> {
        self.refused()?;
        self.noted(self.inner.write_document_rendered(write, answers).await)
    }

    async fn write_project_rendered(
        &self,
        write: &ItemWrite<Project>,
        answers: &BTreeMap<String, Value>,
    ) -> Result<NativeId, SourceError> {
        self.refused()?;
        self.noted(self.inner.write_project_rendered(write, answers).await)
    }

    async fn set_task_rendering(
        &self,
        id: &NativeId,
        content: &str,
        provenance: &Value,
        answers: &BTreeMap<String, Value>,
    ) -> Result<Option<()>, SourceError> {
        self.refused()?;
        self.noted(
            self.inner
                .set_task_rendering(id, content, provenance, answers)
                .await,
        )
    }

    async fn set_document_rendering(
        &self,
        id: &NativeId,
        content: &str,
        provenance: &Value,
        answers: &BTreeMap<String, Value>,
    ) -> Result<Option<()>, SourceError> {
        self.refused()?;
        self.noted(
            self.inner
                .set_document_rendering(id, content, provenance, answers)
                .await,
        )
    }

    async fn set_project_rendering(
        &self,
        id: &NativeId,
        content: &str,
        provenance: &Value,
        answers: &BTreeMap<String, Value>,
    ) -> Result<Option<()>, SourceError> {
        self.refused()?;
        self.noted(
            self.inner
                .set_project_rendering(id, content, provenance, answers)
                .await,
        )
    }

    async fn task_assets(&self, id: &NativeId) -> Result<Vec<Asset>, SourceError> {
        self.refused()?;
        self.noted(self.inner.task_assets(id).await)
    }

    async fn document_assets(&self, id: &NativeId) -> Result<Vec<Asset>, SourceError> {
        self.refused()?;
        self.noted(self.inner.document_assets(id).await)
    }

    async fn task_asset(
        &self,
        id: &NativeId,
        name: &AssetName,
    ) -> Result<Option<Vec<u8>>, SourceError> {
        self.refused()?;
        self.noted(self.inner.task_asset(id, name).await)
    }

    async fn document_asset(
        &self,
        id: &NativeId,
        name: &AssetName,
    ) -> Result<Option<Vec<u8>>, SourceError> {
        self.refused()?;
        self.noted(self.inner.document_asset(id, name).await)
    }

    async fn write_task_with_assets(
        &self,
        write: &ItemWrite<Task>,
        answers: Option<&BTreeMap<String, Value>>,
        assets: &AssetWrite,
    ) -> Result<AssetsWritten, SourceError> {
        self.refused()?;
        self.noted(
            self.inner
                .write_task_with_assets(write, answers, assets)
                .await,
        )
    }

    async fn write_document_with_assets(
        &self,
        write: &ItemWrite<Document>,
        answers: Option<&BTreeMap<String, Value>>,
        assets: &AssetWrite,
    ) -> Result<AssetsWritten, SourceError> {
        self.refused()?;
        self.noted(
            self.inner
                .write_document_with_assets(write, answers, assets)
                .await,
        )
    }

    async fn set_task_rendering_with_assets(
        &self,
        id: &NativeId,
        content: &str,
        provenance: &Value,
        answers: &BTreeMap<String, Value>,
        assets: &AssetWrite,
    ) -> Result<Option<AssetsWritten>, SourceError> {
        self.refused()?;
        self.noted(
            self.inner
                .set_task_rendering_with_assets(id, content, provenance, answers, assets)
                .await,
        )
    }

    async fn set_document_rendering_with_assets(
        &self,
        id: &NativeId,
        content: &str,
        provenance: &Value,
        answers: &BTreeMap<String, Value>,
        assets: &AssetWrite,
    ) -> Result<Option<AssetsWritten>, SourceError> {
        self.refused()?;
        self.noted(
            self.inner
                .set_document_rendering_with_assets(id, content, provenance, answers, assets)
                .await,
        )
    }

    async fn metering(&self) -> Result<Option<Metering>, SourceError> {
        self.refused()?;
        self.noted(self.inner.metering().await)
    }

    async fn end_command(&self) -> Result<(), SourceError> {
        self.inner.end_command().await
    }

    async fn visibility(&self, target: &WriteTarget<'_>) -> Result<Visibility, SourceError> {
        self.refused()?;
        self.noted(self.inner.visibility(target).await)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, AtomicU64, Ordering as Atomic};

    use onetaskgraph_plugin_api::{
        Capabilities, Clock, DependencySupport, Health, Label, NativeId, Page, PageRequest,
        Project, ProjectQuery, SourceError, SourceName, Support, Task, TaskQuery, TaskSource,
    };

    use super::HeldOff;

    /// A clock that moves only when a test moves it.
    #[derive(Default)]
    struct Manual(AtomicU64);

    impl Manual {
        fn advance(&self, millis: u64) {
            self.0.fetch_add(millis, Atomic::SeqCst);
        }
    }

    impl Clock for Manual {
        fn now(&self) -> std::time::Duration {
            std::time::Duration::from_millis(self.0.load(Atomic::SeqCst))
        }
        fn sleep(
            &self,
            duration: std::time::Duration,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>> {
            self.advance(u64::try_from(duration.as_millis()).unwrap_or(u64::MAX));
            Box::pin(async {})
        }
    }

    /// A source that counts every call it receives and answers each with the next of
    /// `answers`, an empty answer once they run out.
    struct Scripted {
        calls: Arc<AtomicU32>,
        answers: std::sync::Mutex<Vec<Option<SourceError>>>,
        ended: Arc<AtomicU32>,
    }

    impl Scripted {
        fn next(&self) -> Result<(), SourceError> {
            self.calls.fetch_add(1, Atomic::SeqCst);
            let mut answers = self.answers.lock().unwrap();
            if answers.is_empty() {
                return Ok(());
            }
            answers.remove(0).map_or(Ok(()), Err)
        }
    }

    #[async_trait::async_trait]
    impl TaskSource for Scripted {
        fn kind(&self) -> &'static str {
            "scripted"
        }
        fn capabilities(&self) -> Capabilities {
            Capabilities {
                projects: Support::Native,
                documents: Support::Unsupported,
                comments: Support::Unsupported,
                assets: Support::Unsupported,
                priority: Support::Unsupported,
                filter_by_priority: Support::Unsupported,
                filter_by_comment_activity: Support::Unsupported,
                filter_by_metadata: Support::Unsupported,
                filter_by_origin: Support::Unsupported,
                reconcile_status: Support::Unsupported,
                complete_by_issue: Support::Unsupported,
                orphan_tasks: Support::Native,
                filter_by_label: Support::Native,
                filter_by_status: Support::Native,
                search_title: Support::Native,
                search_content: Support::Native,
                task_dependencies: DependencySupport::BothDirections,
                project_dependencies: DependencySupport::BothDirections,
                max_page_size: 10,
            }
        }
        async fn end_command(&self) -> Result<(), SourceError> {
            self.ended.fetch_add(1, Atomic::SeqCst);
            Ok(())
        }
        async fn health(&self) -> Result<Health, SourceError> {
            self.next()?;
            Ok(Health {
                reachable: true,
                detail: None,
            })
        }
        async fn get_task(&self, _id: &NativeId) -> Result<Option<Task>, SourceError> {
            self.next().map(|()| None)
        }
        async fn get_project(&self, _id: &NativeId) -> Result<Option<Project>, SourceError> {
            self.next().map(|()| None)
        }
        async fn query_tasks(
            &self,
            _query: &TaskQuery,
            _page: &PageRequest,
        ) -> Result<Page<Task>, SourceError> {
            self.next().map(|()| Page::last(Vec::new()))
        }
        async fn query_projects(
            &self,
            _query: &ProjectQuery,
            _page: &PageRequest,
        ) -> Result<Page<Project>, SourceError> {
            self.next().map(|()| Page::last(Vec::new()))
        }
        async fn labels(&self, _page: &PageRequest) -> Result<Page<Label>, SourceError> {
            self.next().map(|()| Page::last(Vec::new()))
        }
        async fn task_dependencies(
            &self,
            _id: &NativeId,
            _direction: onetaskgraph_plugin_api::Direction,
            _page: &PageRequest,
        ) -> Result<Page<onetaskgraph_plugin_api::DependencyEdge>, SourceError> {
            self.next().map(|()| Page::last(Vec::new()))
        }
        async fn project_dependencies(
            &self,
            _id: &NativeId,
            _direction: onetaskgraph_plugin_api::Direction,
            _page: &PageRequest,
        ) -> Result<Page<onetaskgraph_plugin_api::DependencyEdge>, SourceError> {
            self.next().map(|()| Page::last(Vec::new()))
        }
    }

    fn limited(seconds: Option<u64>) -> Option<SourceError> {
        Some(SourceError::RateLimited {
            retry_after_seconds: seconds,
            message: Some("the backend's own words".to_owned()),
        })
    }

    fn held(answers: Vec<Option<SourceError>>) -> (HeldOff, Arc<AtomicU32>, Arc<Manual>) {
        let (source, calls, clock, _) = held_ending(answers);
        (source, calls, clock)
    }

    /// [`held`], and the count of `end_command` calls the source received.
    fn held_ending(
        answers: Vec<Option<SourceError>>,
    ) -> (HeldOff, Arc<AtomicU32>, Arc<Manual>, Arc<AtomicU32>) {
        let calls = Arc::new(AtomicU32::new(0));
        let ended = Arc::new(AtomicU32::new(0));
        let clock = Arc::new(Manual::default());
        let source = HeldOff::new(
            SourceName::new("tracker").unwrap(),
            Box::new(Scripted {
                calls: Arc::clone(&calls),
                answers: std::sync::Mutex::new(answers),
                ended: Arc::clone(&ended),
            }),
            clock.clone(),
        );
        (source, calls, clock, ended)
    }

    fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(future)
    }

    fn waits(answered: Result<Option<Task>, SourceError>) -> Option<u64> {
        match answered {
            Err(SourceError::RateLimited {
                retry_after_seconds,
                ..
            }) => retry_after_seconds,
            other => panic!("not a rate limit: {other:?}"),
        }
    }

    #[test]
    fn a_source_that_named_a_reset_is_sent_nothing_before_it_and_reached_after_it() {
        let (source, calls, clock) = held(vec![limited(Some(30))]);
        let id = NativeId::from("T-1");
        block_on(async {
            assert_eq!(waits(source.get_task(&id).await), Some(30));
            assert_eq!(calls.load(Atomic::SeqCst), 1, "the refusal itself was sent");

            clock.advance(10_400);
            let refused = source.get_task(&id).await;
            assert_eq!(
                calls.load(Atomic::SeqCst),
                1,
                "nothing sent before the reset"
            );
            let message = format!("{}", refused.as_ref().unwrap_err());
            assert!(message.contains("not sent"), "{message}");
            assert!(message.contains("tracker"), "{message}");
            assert_eq!(
                waits(refused),
                Some(20),
                "the whole seconds left, rounded up"
            );
            // Every call, not only the kind refused: a listing, and a batched read, each id.
            assert!(matches!(
                source
                    .labels(&PageRequest {
                        limit: 5,
                        cursor: None
                    })
                    .await,
                Err(SourceError::RateLimited {
                    retry_after_seconds: Some(20),
                    ..
                })
            ));
            let batched = source
                .get_task_details(&[id.clone(), id.clone()], None)
                .await;
            assert_eq!(batched.len(), 2);
            assert!(batched.iter().all(|read| matches!(
                read,
                Err(SourceError::RateLimited {
                    retry_after_seconds: Some(20),
                    ..
                })
            )));
            assert_eq!(calls.load(Atomic::SeqCst), 1);

            clock.advance(19_000);
            assert_eq!(waits(source.get_task(&id).await), Some(1), "never zero");
            clock.advance(600);
            assert_eq!(source.get_task(&id).await.unwrap(), None, "at the reset");
            assert_eq!(
                calls.load(Atomic::SeqCst),
                2,
                "the call at the reset was sent"
            );
        });
    }

    #[test]
    fn a_rate_limit_naming_no_wait_holds_nothing_back() {
        let (source, calls, _) = held(vec![limited(None), limited(Some(0))]);
        let id = NativeId::from("T-1");
        block_on(async {
            assert_eq!(waits(source.get_task(&id).await), None);
            assert_eq!(waits(source.get_task(&id).await), Some(0));
            assert_eq!(source.get_task(&id).await.unwrap(), None);
            assert_eq!(calls.load(Atomic::SeqCst), 3, "every call was sent");
        });
    }

    #[test]
    fn a_shorter_reset_never_pulls_a_longer_one_in_and_other_failures_hold_nothing_back() {
        let (source, calls, clock) = held(vec![
            Some(SourceError::Unavailable {
                message: "down".to_owned(),
            }),
            limited(Some(60)),
        ]);
        let id = NativeId::from("T-1");
        block_on(async {
            assert!(matches!(
                source.get_task(&id).await,
                Err(SourceError::Unavailable { .. })
            ));
            assert_eq!(waits(source.get_task(&id).await), Some(60));
            // A second limit arriving through `noted` while one is held keeps the later reset.
            source
                .noted::<()>(Err(limited(Some(5)).unwrap()))
                .unwrap_err();
            clock.advance(30_000);
            assert_eq!(waits(source.get_task(&id).await), Some(30));
            assert_eq!(calls.load(Atomic::SeqCst), 2);
        });
    }

    #[test]
    fn a_rate_limit_answered_inside_a_batch_starts_the_hold() {
        let (source, calls, _) = held(vec![None, limited(Some(40))]);
        let ids = [NativeId::from("T-1"), NativeId::from("T-2")];
        block_on(async {
            let batched = source.get_task_details(&ids, None).await;
            assert!(batched[0].is_ok());
            assert!(matches!(
                batched[1],
                Err(SourceError::RateLimited {
                    retry_after_seconds: Some(40),
                    ..
                })
            ));
            assert_eq!(calls.load(Atomic::SeqCst), 2);
            assert_eq!(waits(source.get_task(&ids[0]).await), Some(40));
            assert_eq!(calls.load(Atomic::SeqCst), 2, "held by the batch's answer");
        });
    }

    #[test]
    fn a_reset_too_far_for_the_clock_holds_the_source_off_without_overflowing() {
        let (source, calls, clock) = held(vec![limited(Some(u64::MAX))]);
        let id = NativeId::from("T-1");
        block_on(async {
            clock.advance(5_000);
            assert_eq!(waits(source.get_task(&id).await), Some(u64::MAX));
            clock.advance(3_600_000);
            assert!(waits(source.get_task(&id).await).is_some_and(|left| left > 1_000_000));
            assert_eq!(calls.load(Atomic::SeqCst), 1);
        });
    }

    #[test]
    fn ending_a_command_reaches_the_source_during_a_hold() {
        let (source, calls, _, ended) = held_ending(vec![limited(Some(30))]);
        block_on(async {
            waits(source.get_task(&NativeId::from("T-1")).await);
            source
                .end_command()
                .await
                .expect("dropping what it holds is no request");
            assert_eq!(ended.load(Atomic::SeqCst), 1, "it reached the source");
            assert_eq!(calls.load(Atomic::SeqCst), 1);
        });
    }

    /// Every method of `TaskSource` is forwarded here, and nothing else: a method added to the
    /// trait with a default would otherwise be answered by that default, bypassing both the
    /// source and the hold.
    #[test]
    fn every_method_of_the_trait_is_forwarded() {
        fn methods(text: &str, start: &str) -> std::collections::BTreeSet<String> {
            let from = text.find(start).expect("the block");
            let block = &text[from..from + text[from..].find("\n}\n").expect("its end")];
            block
                .lines()
                .filter_map(|line| {
                    let line = line.strip_prefix("    ")?;
                    let line = line.strip_prefix("async ").unwrap_or(line);
                    let name = line.strip_prefix("fn ")?;
                    Some(name[..name.find(['(', '<'])?].to_owned())
                })
                .collect()
        }
        let declared = methods(
            include_str!("../../../onetaskgraph-plugin-api/src/source.rs"),
            "pub trait TaskSource",
        );
        let forwarded = methods(include_str!("held_off.rs"), "impl TaskSource for HeldOff");
        assert_eq!(declared, forwarded);
    }
}
