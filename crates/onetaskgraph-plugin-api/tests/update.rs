//! `TaskSource::update_task` as a plugin that does not override it meets it.
//!
//! The default is what keeps the targeted update an addition: a source written before it —
//! an out-of-process plugin whose handshake predates it included — is asked through it and
//! must answer correctly. So these drive a source that implements the reads and `write_task`
//! and nothing of the update, and assert on what it was asked to write.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use onetaskgraph_plugin_api::{
    Capabilities, DependencyEdge, DependencyEndpoint, DependencyKind, DependencySupport, Direction,
    Health, ItemKind, ItemWrite, Label, MetadataKey, NativeId, Page, PageRequest, Priority,
    Project, ProjectQuery, SourceError, Status, StatusCategory, Support, Task, TaskQuery, TaskRef,
    TaskSource, TaskUpdate, TaskUpdateOutcome, UpdatedField, WriteSupport,
};
use schemars::schema_for;
use serde_json::{Value, json};

/// A writable source holding one task and its forward edges, which records every write.
struct Rewritten {
    task: Mutex<Task>,
    edges: Mutex<Vec<DependencyEdge>>,
    writes: Mutex<Vec<ItemWrite<Task>>>,
    reads: Mutex<usize>,
    /// The id every write answers, when it is not the target the write named.
    answers: Option<NativeId>,
}

impl Rewritten {
    fn holding(task: Task, edges: Vec<DependencyEdge>) -> Self {
        Self {
            task: Mutex::new(task),
            edges: Mutex::new(edges),
            writes: Mutex::new(Vec::new()),
            reads: Mutex::new(0),
            answers: None,
        }
    }

    fn writes(&self) -> Vec<ItemWrite<Task>> {
        self.writes.lock().expect("unpoisoned").clone()
    }
}

#[async_trait::async_trait]
impl TaskSource for Rewritten {
    fn kind(&self) -> &'static str {
        "rewritten"
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            projects: Support::Native,
            documents: Support::Unsupported,
            comments: Support::Unsupported,
            assets: Support::Unsupported,
            priority: Support::Native,
            filter_by_priority: Support::Unsupported,
            filter_by_comment_activity: Support::Unsupported,
            filter_by_metadata: Support::Unsupported,
            filter_by_origin: Support::Unsupported,
            orphan_tasks: Support::Native,
            filter_by_label: Support::Unsupported,
            filter_by_status: Support::Unsupported,
            search_title: Support::Unsupported,
            search_content: Support::Unsupported,
            task_dependencies: DependencySupport::ForwardOnly,
            project_dependencies: DependencySupport::ForwardOnly,
            // One edge a page, so a default that stopped after the first page would carry
            // half the edges through a rewrite.
            max_page_size: 1,
        }
    }
    async fn health(&self) -> Result<Health, SourceError> {
        Ok(Health {
            reachable: true,
            detail: None,
        })
    }
    async fn get_task(&self, id: &NativeId) -> Result<Option<Task>, SourceError> {
        *self.reads.lock().expect("unpoisoned") += 1;
        let task = self.task.lock().expect("unpoisoned").clone();
        Ok((task.id == *id).then_some(task))
    }
    async fn get_project(&self, _id: &NativeId) -> Result<Option<Project>, SourceError> {
        Ok(None)
    }
    async fn query_tasks(
        &self,
        _query: &TaskQuery,
        _page: &PageRequest,
    ) -> Result<Page<Task>, SourceError> {
        Ok(Page::last(Vec::new()))
    }
    async fn query_projects(
        &self,
        _query: &ProjectQuery,
        _page: &PageRequest,
    ) -> Result<Page<Project>, SourceError> {
        Ok(Page::last(Vec::new()))
    }
    async fn labels(&self, _page: &PageRequest) -> Result<Page<Label>, SourceError> {
        Ok(Page::last(Vec::new()))
    }
    async fn task_dependencies(
        &self,
        _id: &NativeId,
        _direction: Direction,
        page: &PageRequest,
    ) -> Result<Page<DependencyEdge>, SourceError> {
        let edges = self.edges.lock().expect("unpoisoned").clone();
        let offset: usize = page
            .cursor
            .as_ref()
            .map_or(0, |cursor| cursor.0.parse().expect("a numeric cursor"));
        let items: Vec<DependencyEdge> = edges.iter().skip(offset).take(1).cloned().collect();
        let next = (offset + 1 < edges.len())
            .then(|| onetaskgraph_plugin_api::Cursor((offset + 1).to_string()));
        Ok(Page { items, next })
    }
    async fn project_dependencies(
        &self,
        _id: &NativeId,
        _direction: Direction,
        _page: &PageRequest,
    ) -> Result<Page<DependencyEdge>, SourceError> {
        Ok(Page::last(Vec::new()))
    }
    fn writes(&self) -> WriteSupport {
        WriteSupport::Supported
    }
    async fn write_task(&self, write: &ItemWrite<Task>) -> Result<NativeId, SourceError> {
        self.writes.lock().expect("unpoisoned").push(write.clone());
        let mut held = write.item.clone();
        held.id = write.target.clone().expect("an update names its target");
        *self.task.lock().expect("unpoisoned") = held;
        *self.edges.lock().expect("unpoisoned") = write.depends_on.clone();
        Ok(self
            .answers
            .clone()
            .unwrap_or_else(|| write.target.clone().expect("an update names its target")))
    }
}

fn key(name: &str) -> MetadataKey {
    MetadataKey::new(name).expect("a caller key")
}

fn edge(from: &str, to: &str) -> DependencyEdge {
    DependencyEdge {
        from: DependencyEndpoint::from_native(NativeId::from(from), ItemKind::Task),
        to: DependencyEndpoint::from_native(NativeId::from(to), ItemKind::Task),
        kind: DependencyKind::Blocks,
    }
}

fn held() -> Task {
    Task {
        id: NativeId::from("T-1"),
        key: None,
        title: "Alpha engine".to_owned(),
        content: Some("the body".to_owned()),
        status: Status {
            category: StatusCategory::Todo,
            name: "todo".to_owned(),
        },
        priority: Priority::Low,
        labels: vec![Label {
            id: NativeId::from("l"),
            name: "l".to_owned(),
            color: None,
        }],
        project: Some(NativeId::from("P")),
        url: None,
        location: None,
        created_at: None,
        updated_at: None,
        metadata: BTreeMap::from([
            ("team.a".to_owned(), json!(1)),
            ("team.b".to_owned(), json!("two")),
            ("team.c".to_owned(), json!([3])),
        ]),
        repositories: Vec::new(),
        delivers: vec![TaskRef::new("T-9".to_owned()).expect("a task id")],
        delivered_by: vec![TaskRef::new("work:T-8".to_owned()).expect("a task id")],
    }
}

fn source() -> Rewritten {
    Rewritten::holding(held(), vec![edge("T-1", "T-2"), edge("T-1", "T-3")])
}

#[tokio::test]
async fn an_update_naming_only_what_the_task_holds_writes_nothing() {
    let source = source();
    let update = TaskUpdate {
        title: Some("Alpha engine".to_owned()),
        status: Some(held().status),
        priority: Some(Priority::Low),
        metadata_set: BTreeMap::from([(key("team.a"), json!(1))]),
        metadata_remove: BTreeSet::from([key("team.absent")]),
        delivers: Some(held().delivers),
        depends_on: Some(vec![edge("T-1", "T-3"), edge("T-1", "T-2")]),
        ..TaskUpdate::default()
    };
    let answer = source
        .update_task(&NativeId::from("T-1"), &update)
        .await
        .expect("an update")
        .expect("the task is held");
    assert!(source.writes().is_empty(), "{:#?}", source.writes());
    assert_eq!(
        answer,
        TaskUpdateOutcome {
            task: held(),
            written: BTreeSet::new(),
            delivers_before: held().delivers,
        }
    );
    assert_eq!(*source.reads.lock().expect("unpoisoned"), 1);
}

#[tokio::test]
async fn an_update_rewrites_the_named_fields_and_carries_everything_else_through() {
    let source = source();
    let update = TaskUpdate {
        status: Some(Status {
            category: StatusCategory::Cancelled,
            name: "failed".to_owned(),
        }),
        metadata_set: BTreeMap::from([(key("team.a"), json!({"n": 2})), (key("team.d"), json!(4))]),
        metadata_remove: BTreeSet::from([key("team.b")]),
        ..TaskUpdate::default()
    };
    let answer = source
        .update_task(&NativeId::from("T-1"), &update)
        .await
        .expect("an update")
        .expect("the task is held");
    let writes = source.writes();
    assert_eq!(writes.len(), 1, "{writes:#?}");
    let write = &writes[0];
    assert_eq!(write.target, Some(NativeId::from("T-1")));
    // Both edges, over two pages: a rewrite replaces them, so they are carried through.
    assert_eq!(
        write.depends_on,
        vec![edge("T-1", "T-2"), edge("T-1", "T-3")]
    );
    let mut expected = held();
    expected.status = Status {
        category: StatusCategory::Cancelled,
        name: "failed".to_owned(),
    };
    expected.metadata = BTreeMap::from([
        ("team.a".to_owned(), json!({"n": 2})),
        ("team.c".to_owned(), json!([3])),
        ("team.d".to_owned(), json!(4)),
    ]);
    assert_eq!(write.item, expected);
    // The answer is a read, not an echo, and names the two fields that moved.
    assert_eq!(answer.task, expected);
    assert_eq!(
        answer.written,
        BTreeSet::from([UpdatedField::Status, UpdatedField::Metadata])
    );
    assert_eq!(answer.delivers_before, held().delivers);
    assert_eq!(*source.reads.lock().expect("unpoisoned"), 2);
}

#[tokio::test]
async fn a_named_edge_set_that_differs_is_written_whole() {
    let source = source();
    let update = TaskUpdate {
        depends_on: Some(vec![edge("T-1", "T-4")]),
        ..TaskUpdate::default()
    };
    let answer = source
        .update_task(&NativeId::from("T-1"), &update)
        .await
        .expect("an update")
        .expect("the task is held");
    assert_eq!(answer.written, BTreeSet::from([UpdatedField::DependsOn]));
    let writes = source.writes();
    assert_eq!(writes.len(), 1, "{writes:#?}");
    assert_eq!(writes[0].depends_on, vec![edge("T-1", "T-4")]);
    assert_eq!(writes[0].item, held(), "only the edges were named");
}

#[tokio::test]
async fn an_update_both_setting_and_removing_a_key_is_refused_before_anything_is_read() {
    let source = source();
    let update = TaskUpdate {
        title: Some("changed".to_owned()),
        metadata_set: BTreeMap::from([(key("team.a"), json!(2))]),
        metadata_remove: BTreeSet::from([key("team.a")]),
        ..TaskUpdate::default()
    };
    let error = source
        .update_task(&NativeId::from("T-1"), &update)
        .await
        .expect_err("a contradiction");
    let SourceError::Refused { message } = &error else {
        panic!("refused as {error:?}");
    };
    assert!(message.contains("team.a"), "{message}");
    assert!(source.writes().is_empty());
    assert_eq!(*source.reads.lock().expect("unpoisoned"), 0);
}

#[tokio::test]
async fn a_write_answering_another_id_is_malformed_rather_than_an_update() {
    let source = Rewritten {
        answers: Some(NativeId::from("T-9")),
        ..source()
    };
    let update = TaskUpdate {
        title: Some("changed".to_owned()),
        ..TaskUpdate::default()
    };
    let error = source
        .update_task(&NativeId::from("T-1"), &update)
        .await
        .expect_err("a write that reached another record");
    let SourceError::Malformed { message } = &error else {
        panic!("answered as {error:?}");
    };
    assert!(
        message.contains("T-1") && message.contains("T-9"),
        "{message}"
    );
    // Nothing was read back to report as the update: the one read is the one before it.
    assert_eq!(*source.reads.lock().expect("unpoisoned"), 1);
}

#[tokio::test]
async fn a_task_the_source_does_not_hold_is_none_and_nothing_is_written() {
    let source = source();
    let update = TaskUpdate {
        title: Some("changed".to_owned()),
        ..TaskUpdate::default()
    };
    let answer = source
        .update_task(&NativeId::from("T-404"), &update)
        .await
        .expect("an answer");
    assert_eq!(answer, None);
    assert!(source.writes().is_empty());
}

#[test]
fn an_update_names_on_the_wire_only_what_it_names_and_round_trips() {
    assert_eq!(
        serde_json::to_value(TaskUpdate::default()).expect("serialises"),
        json!({})
    );
    let whole = TaskUpdate {
        title: Some("t".to_owned()),
        content: Some(String::new()),
        status: Some(Status {
            category: StatusCategory::Queued,
            name: "queued".to_owned(),
        }),
        priority: Some(Priority::None),
        metadata_set: BTreeMap::from([(key("team.a"), Value::Null)]),
        metadata_remove: BTreeSet::from([key("team.b")]),
        delivers: Some(Vec::new()),
        depends_on: Some(Vec::new()),
    };
    let wire = serde_json::to_value(&whole).expect("serialises");
    assert_eq!(wire["delivers"], json!([]), "an empty list is named");
    assert_eq!(wire["metadata_set"], json!({"team.a": null}));
    let back: TaskUpdate = serde_json::from_value(wire).expect("round trips");
    assert_eq!(back, whole);
    assert!(!whole.is_empty());
    assert!(TaskUpdate::default().is_empty());
    for field in [
        UpdatedField::Title,
        UpdatedField::Content,
        UpdatedField::Status,
        UpdatedField::Priority,
        UpdatedField::Metadata,
        UpdatedField::Delivers,
        UpdatedField::DependsOn,
    ] {
        assert!(whole.names(field), "{field:?}");
        assert!(!TaskUpdate::default().names(field), "{field:?}");
    }
    assert_eq!(
        serde_json::to_value(BTreeSet::from([
            UpdatedField::DependsOn,
            UpdatedField::Title
        ]))
        .expect("serialises"),
        json!(["title", "depends-on"]),
        "kebab-case, in declaration order"
    );
    // A reserved key is refused where the update is read, as every metadata key is.
    let refused =
        serde_json::from_value::<TaskUpdate>(json!({"metadata_remove": ["onetaskgraph.origin"]}));
    assert!(refused.is_err(), "{refused:?}");
    assert!(schema_for!(TaskUpdate).as_value().is_object());
    assert!(schema_for!(TaskUpdateOutcome).as_value().is_object());
}
