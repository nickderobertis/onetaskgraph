//! The in-memory source's `queued` status, its status-only write, and a task's two lists,
//! driven through the real trait.

use onetaskgraph_in_memory::{InMemoryConfig, InMemorySource};
use onetaskgraph_plugin_api::{
    ItemWrite, NativeId, PageRequest, SourceError, Status, StatusCategory, Task, TaskQuery,
    TaskRef, TaskSource,
};
use serde_json::{Value, json};

fn source(config: Value) -> Result<InMemorySource, SourceError> {
    let config: InMemoryConfig = serde_json::from_value(config).expect("a configuration");
    InMemorySource::new(config)
}

fn work(writes: &str) -> InMemorySource {
    source(json!({
        "capabilities": {"writes": writes},
        "tasks": [
            {"id": "T-1", "title": "Claimed", "content": "body",
             "status": {"category": "queued", "name": "Queued"}, "labels": [],
             "metadata": {"caller.count": 3},
             "delivers": ["T-2", "other:X-1"], "delivered_by": ["plan:P-1"]},
            {"id": "T-2", "title": "Ready", "content": null,
             "status": {"category": "todo", "name": "Todo"}, "labels": []}
        ]
    }))
    .expect("a coherent source")
}

fn id(value: &str) -> NativeId {
    NativeId(value.to_owned())
}

fn entry(value: &str) -> TaskRef {
    TaskRef::new(value).expect("a task id")
}

fn refused(error: SourceError) -> String {
    match error {
        SourceError::Refused { message } | SourceError::Config { message } => message,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

async fn with_statuses(source: &InMemorySource, statuses: &[StatusCategory]) -> Vec<String> {
    let query = TaskQuery {
        statuses: statuses.to_vec(),
        ..TaskQuery::default()
    };
    source
        .query_tasks(
            &query,
            &PageRequest {
                cursor: None,
                limit: 50,
            },
        )
        .await
        .expect("answered")
        .items
        .into_iter()
        .map(|task| task.id.0)
        .collect()
}

#[tokio::test]
async fn a_queued_task_is_stored_and_filtered_as_queued() {
    let source = work("supported");
    assert_eq!(
        with_statuses(&source, &[StatusCategory::Queued]).await,
        ["T-1"]
    );
    assert_eq!(
        with_statuses(&source, &[StatusCategory::Todo]).await,
        ["T-2"]
    );
    let held = source.get_task(&id("T-1")).await.unwrap().expect("held");
    assert_eq!(held.delivers, vec![entry("T-2"), entry("other:X-1")]);
    assert_eq!(held.delivered_by, vec![entry("plan:P-1")]);
}

#[tokio::test]
async fn a_status_set_moves_the_status_alone_and_keeps_a_name_the_category_already_has() {
    let source = work("supported");
    let before = source.get_task(&id("T-1")).await.unwrap().expect("held");

    let kept = source
        .set_task_status(&id("T-1"), StatusCategory::Queued)
        .await
        .unwrap();
    assert_eq!(
        kept,
        Some(Status {
            category: StatusCategory::Queued,
            name: "Queued".to_owned()
        })
    );

    let moved = source
        .set_task_status(&id("T-1"), StatusCategory::InProgress)
        .await
        .unwrap();
    assert_eq!(
        moved,
        Some(Status {
            category: StatusCategory::InProgress,
            name: "in-progress".to_owned()
        })
    );
    let after = source.get_task(&id("T-1")).await.unwrap().expect("held");
    assert_eq!(
        after,
        Task {
            status: moved.expect("answered"),
            ..before
        },
        "nothing but the status moved"
    );
    assert_eq!(
        source
            .set_task_status(&id("T-9"), StatusCategory::Done)
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn delivered_by_is_replaced_whole_and_nothing_else_moves() {
    let source = work("supported");
    let before = source.get_task(&id("T-2")).await.unwrap().expect("held");
    source
        .set_delivered_by(&id("T-2"), &[entry("work:T-1"), entry("plan:P-2")])
        .await
        .unwrap()
        .expect("held");
    let after = source.get_task(&id("T-2")).await.unwrap().expect("held");
    assert_eq!(
        after,
        Task {
            delivered_by: vec![entry("work:T-1"), entry("plan:P-2")],
            ..before
        }
    );
    assert_eq!(
        source.set_delivered_by(&id("T-9"), &[]).await.unwrap(),
        None
    );
    let message = refused(
        source
            .set_delivered_by(&id("T-2"), &[entry("T-2")])
            .await
            .expect_err("a task is not delivered by itself"),
    );
    assert!(
        message.contains("names T-2, which is that task itself"),
        "{message}"
    );
}

#[tokio::test]
async fn a_source_with_no_write_side_refuses_both_narrow_writes() {
    let source = work("unsupported");
    for message in [
        refused(
            source
                .set_task_status(&id("T-1"), StatusCategory::Done)
                .await
                .expect_err("refused"),
        ),
        refused(
            source
                .set_delivered_by(&id("T-1"), &[])
                .await
                .expect_err("refused"),
        ),
    ] {
        assert_eq!(message, "the in-memory plugin cannot be written");
    }
}

#[tokio::test]
async fn a_list_naming_its_own_task_or_one_task_twice_is_refused_in_configuration_and_on_write() {
    let message = refused(
        source(json!({"tasks": [
            {"id": "T-1", "title": "Self", "content": null,
             "status": {"category": "todo", "name": "Todo"}, "labels": [],
             "delivers": ["T-1"]},
            {"id": "T-2", "title": "Twice", "content": null,
             "status": {"category": "todo", "name": "Todo"}, "labels": [],
             "delivered_by": ["plan:P-1", "plan:P-1"]}
        ]}))
        .expect_err("incoherent"),
    );
    assert!(
        message.contains("delivers on task T-1 names T-1, which is that task itself"),
        "{message}"
    );
    assert!(
        message.contains("delivered_by on task T-2 names plan:P-1 more than once"),
        "{message}"
    );

    let source = work("supported");
    let held = source.get_task(&id("T-2")).await.unwrap().expect("held");
    let message = refused(
        source
            .write_task(&ItemWrite {
                target: Some(id("T-2")),
                item: Task {
                    delivers: vec![entry("T-2")],
                    ..held
                },
                depends_on: Vec::new(),
            })
            .await
            .expect_err("refused"),
    );
    assert!(
        message.contains("cannot represent the field `delivers`"),
        "{message}"
    );
}
