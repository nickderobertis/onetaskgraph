//! A `--comments` copy reads each task and its comments as one revision, at either end — driven
//! as a Rust caller drives `Engine::copy`, over a source whose detail reads can be made to
//! answer without the comments they were asked for, to carry a new revision on every request,
//! or to lose the task part way through its pages.
//!
//! The journeys in `crates/onetaskgraph-e2e/tests/e2e/copy_snapshot.rs` hold the same rule to
//! the loopback boards a person's edits race against; this is where the answers no board gives
//! on purpose are put to the engine.

use std::num::NonZeroU32;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use onetaskgraph_core::{
    ConfiguredSource, CopyAction, CopyItems, CopyRequest, CopyScope, Engine, EngineError, Filters,
    GlobalId, Paging, ProjectSelector, ResolvedSource, TaskRequest,
};
use onetaskgraph_plugin_api::{
    Capabilities, Comment, CommentBody, DependencyEdge, Direction, Health, ItemWrite, Label,
    MetadataKey, NativeId, NewComment, Page, PageRequest, Project, ProjectQuery, SecretResolver,
    SourceError, SourceName, SourcePlugin, Task, TaskDetailRead, TaskQuery, TaskRef, TaskSource,
    TaskUpdate, TaskUpdateOutcome, Visibility, WriteSupport, WriteTarget,
};
use secrecy::SecretString;
use serde_json::{Value, json};

struct NoSecrets;
impl SecretResolver for NoSecrets {
    fn get(&self, _var: &str) -> Option<SecretString> {
        None
    }
}

fn name(value: &str) -> SourceName {
    SourceName::new(value).expect("a valid source name")
}

fn id(qualified: &str) -> GlobalId {
    qualified.parse().expect("a qualified id")
}

/// How a [`Unsteady`] source answers a detail read.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Mode {
    /// As the source it wraps answers.
    Steady,
    /// Every task without the comments it was asked for.
    Commentless,
    /// Every task carrying a change token no earlier request carried.
    Shifting,
    /// Every page past the first answering that the task is gone.
    VanishingLater,
}

/// An in-memory source whose detail reads answer as [`Mode`] says, and which counts them.
struct Unsteady {
    inner: Box<dyn TaskSource>,
    mode: Arc<Mutex<Mode>>,
    detail_reads: Arc<AtomicU32>,
}

#[async_trait::async_trait]
impl TaskSource for Unsteady {
    fn kind(&self) -> &'static str {
        self.inner.kind()
    }
    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }
    fn writes(&self) -> WriteSupport {
        self.inner.writes()
    }
    async fn health(&self) -> Result<Health, SourceError> {
        self.inner.health().await
    }
    async fn visibility(&self, target: &WriteTarget<'_>) -> Result<Visibility, SourceError> {
        self.inner.visibility(target).await
    }
    async fn get_task(&self, id: &NativeId) -> Result<Option<Task>, SourceError> {
        self.inner.get_task(id).await
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
    async fn write_task(&self, write: &ItemWrite<Task>) -> Result<NativeId, SourceError> {
        self.inner.write_task(write).await
    }
    async fn delete_task(&self, id: &NativeId) -> Result<(), SourceError> {
        self.inner.delete_task(id).await
    }
    async fn set_task_metadata(
        &self,
        id: &NativeId,
        key: &MetadataKey,
        value: &Value,
    ) -> Result<Option<Task>, SourceError> {
        self.inner.set_task_metadata(id, key, value).await
    }
    async fn set_delivered_by(
        &self,
        id: &NativeId,
        delivered_by: &[TaskRef],
    ) -> Result<Option<()>, SourceError> {
        self.inner.set_delivered_by(id, delivered_by).await
    }
    async fn update_task(
        &self,
        id: &NativeId,
        update: &TaskUpdate,
    ) -> Result<Option<TaskUpdateOutcome>, SourceError> {
        self.inner.update_task(id, update).await
    }
    async fn task_comments(
        &self,
        task: &NativeId,
        page: &PageRequest,
    ) -> Result<Option<Page<Comment>>, SourceError> {
        self.inner.task_comments(task, page).await
    }
    async fn add_comment(
        &self,
        task: &NativeId,
        comment: &NewComment,
    ) -> Result<Option<Comment>, SourceError> {
        self.inner.add_comment(task, comment).await
    }
    async fn edit_comment(
        &self,
        task: &NativeId,
        comment: &NativeId,
        body: &CommentBody,
    ) -> Result<Option<Comment>, SourceError> {
        self.inner.edit_comment(task, comment, body).await
    }
    async fn delete_comment(
        &self,
        task: &NativeId,
        comment: &NativeId,
    ) -> Result<Option<NativeId>, SourceError> {
        self.inner.delete_comment(task, comment).await
    }
    async fn get_task_details(
        &self,
        ids: &[NativeId],
        comments: Option<&PageRequest>,
    ) -> Vec<Result<Option<TaskDetailRead>, SourceError>> {
        let read = self.detail_reads.fetch_add(1, Ordering::SeqCst) + 1;
        let mode = *self.mode.lock().expect("the mode");
        let later_page = comments.is_some_and(|page| page.cursor.is_some());
        self.inner
            .get_task_details(ids, comments)
            .await
            .into_iter()
            .map(|answer| {
                answer.map(|detail| match (mode, detail) {
                    (Mode::VanishingLater, Some(_)) if later_page => None,
                    (Mode::Commentless, Some(detail)) => Some(TaskDetailRead {
                        comments: None,
                        ..detail
                    }),
                    (Mode::Shifting, Some(mut detail)) => {
                        detail.task.version = Some(format!("v{read}"));
                        Some(detail)
                    }
                    (_, detail) => detail,
                })
            })
            .collect()
    }
}

/// A source reading as [`Mode`] says, over an in-memory source configured by `config`, with
/// the handles a test turns and reads it by.
fn unsteady(named: &str, config: &Value) -> (ConfiguredSource, Arc<Mutex<Mode>>, Arc<AtomicU32>) {
    let inner = onetaskgraph_in_memory::Plugin
        .build(&name(named), config, &NoSecrets)
        .expect("the in-memory plugin builds");
    let mode = Arc::new(Mutex::new(Mode::Steady));
    let detail_reads = Arc::new(AtomicU32::new(0));
    let source = Unsteady {
        inner,
        mode: Arc::clone(&mode),
        detail_reads: Arc::clone(&detail_reads),
    };
    (
        ConfiguredSource::Ready(ResolvedSource::adopt(name(named), Box::new(source))),
        mode,
        detail_reads,
    )
}

/// An in-memory source holding one task, `T-1`, with five comments read two to a page — so
/// reading them whole takes three requests.
fn commented() -> Value {
    json!({
        "capabilities": {"comments": "native", "max_page_size": 2},
        "tasks": [{"id": "T-1", "title": "Alpha", "content": "the body",
                   "status": {"category": "todo", "name": "Todo"}, "labels": []}],
        "comments": (1..=5).map(|n| json!({"task": "T-1", "comment": {
            "id": format!("C-{n}"), "author": "ada",
            "created_at": format!("2026-09-13T15:1{n}:00Z"),
            "updated_at": format!("2026-09-13T15:1{n}:00Z"),
            "body": format!("comment {n}"), "url": null}})).collect::<Vec<_>>(),
    })
}

/// A destination holding nothing, whose comments are read two to a page too.
fn empty() -> Value {
    json!({"capabilities": {"comments": "native", "max_page_size": 2}})
}

/// A `--comments` copy of `from:T-1` into `into`, writing no link back.
fn copy() -> CopyRequest {
    CopyRequest {
        items: CopyItems::new(vec![id("from:T-1")]).expect("one item"),
        scope: CopyScope::Tasks,
        destination: name("into"),
        match_by: None,
        recreate: false,
        create: false,
        dry_run: false,
        comments: true,
        no_link: true,
    }
}

/// Every task the source `named` holds, and the bodies of each one's comments, read through
/// the engine's own verbs.
async fn held(engine: &Engine, named: &str) -> Vec<(String, Vec<String>)> {
    let tasks = engine
        .tasks(&TaskRequest {
            sources: vec![name(named)],
            filters: Filters::default(),
            project: ProjectSelector::Any,
            priorities: Vec::new(),
            commented_since: None,
            metadata: Vec::new(),
            origin: None,
            include_members: false,
            paging: Paging {
                limit: NonZeroU32::new(50).expect("a non-zero limit"),
                token: None,
            },
        })
        .await
        .expect("the list verb answers")
        .items;
    let mut held = Vec::new();
    for task in tasks {
        let comments = engine
            .comments(&task.id)
            .await
            .expect("the comments read")
            .comments;
        held.push((
            task.item.title,
            comments.into_iter().map(|comment| comment.body).collect(),
        ));
    }
    held
}

#[tokio::test]
async fn a_source_answering_a_read_without_the_comments_asked_for_is_refused_and_nothing_lands() {
    let (from, mode, _) = unsteady("from", &commented());
    let (into, _, _) = unsteady("into", &empty());
    let engine = Engine::new(vec![from, into], vec![name("from"), name("into")]);
    *mode.lock().unwrap() = Mode::Commentless;

    let Err(refused) = engine.copy(&copy()).await else {
        panic!("a read without its comments is not a task with none");
    };
    assert!(
        refused.to_string().contains("comments without them"),
        "{refused}"
    );
    assert!(held(&engine, "into").await.is_empty());
}

#[tokio::test]
async fn a_source_task_changing_on_every_request_is_read_again_once_and_then_refused() {
    let (from, mode, detail_reads) = unsteady("from", &commented());
    let (into, _, _) = unsteady("into", &empty());
    let engine = Engine::new(vec![from, into], vec![name("from"), name("into")]);
    *mode.lock().unwrap() = Mode::Shifting;

    let report = engine.copy(&copy()).await.expect("the copy runs");
    let CopyAction::Failed { error } = &report.items[0].action else {
        panic!("a task that never holds still is not copied: {report:?}");
    };
    assert!(
        error
            .message()
            .contains("changed while it was being copied, twice"),
        "{error:?}"
    );
    // The first page beside the task and the second, which disagree; then the same two again,
    // once — and no third attempt.
    assert_eq!(detail_reads.load(Ordering::SeqCst), 4);
    assert!(held(&engine, "into").await.is_empty());
}

#[tokio::test]
async fn a_source_task_gone_part_way_through_its_comment_pages_is_no_such_task() {
    let (from, mode, _) = unsteady("from", &commented());
    let (into, _, _) = unsteady("into", &empty());
    let engine = Engine::new(vec![from, into], vec![name("from"), name("into")]);
    *mode.lock().unwrap() = Mode::VanishingLater;

    let Err(refused) = engine.copy(&copy()).await else {
        panic!("a task gone while it was read is not copied");
    };
    assert!(
        matches!(&refused, EngineError::NoSuchItem { id } if id == "from:T-1"),
        "{refused:?}"
    );
    assert!(held(&engine, "into").await.is_empty());
}

#[tokio::test]
async fn a_destination_misreading_its_copy_is_refused_and_every_copied_comment_stays() {
    for (misreads, refusal) in [
        (Mode::Commentless, "comments without them"),
        (Mode::Shifting, "changed while the copy read it, twice"),
    ] {
        let (from, _, _) = unsteady("from", &commented());
        let (into, mode, _) = unsteady("into", &empty());
        let engine = Engine::new(vec![from, into], vec![name("from"), name("into")]);
        engine.copy(&copy()).await.expect("the first copy lands");
        let landed = held(&engine, "into").await;
        assert_eq!(landed[0].1.len(), 5, "{misreads:?}: {landed:?}");

        *mode.lock().unwrap() = misreads;
        let Err(refused) = engine.copy(&copy()).await else {
            panic!("{misreads:?}: a destination read that cannot be trusted is not acted on");
        };
        assert!(
            refused.to_string().contains(refusal),
            "{misreads:?}: {refused}"
        );
        assert_eq!(
            held(&engine, "into").await,
            landed,
            "{misreads:?}: no copied comment was taken for gone"
        );
    }
}
