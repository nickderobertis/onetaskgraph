//! The in-memory source's narrow metadata writes, driven through the real trait for a task, a
//! project and a document.

use onetaskgraph_in_memory::{InMemoryConfig, InMemorySource};
use onetaskgraph_plugin_api::{MetadataKey, NativeId, SourceError, TaskSource};
use serde_json::{Value, json};

fn source(capabilities: Value) -> InMemorySource {
    let config: InMemoryConfig = serde_json::from_value(json!({
        "capabilities": capabilities,
        "projects": [
            {"id": "P-1", "title": "Plan", "content": "why",
             "status": {"category": "in-progress", "name": "Building"}, "labels": [],
             "metadata": {"myapp.review": "pending", "other.kept": [1, 2]}}
        ],
        "tasks": [
            {"id": "T-1", "title": "Build", "content": "body",
             "status": {"category": "queued", "name": "Queued"}, "labels": [],
             "project": "P-1",
             "metadata": {"myapp.review": "pending", "other.kept": [1, 2]},
             "delivers": ["T-2"]},
            {"id": "T-2", "title": "Ticket", "content": null,
             "status": {"category": "todo", "name": "Todo"}, "labels": []}
        ],
        "documents": [
            {"id": "D-1", "title": "Design", "content": "text", "labels": [],
             "project": "P-1",
             "metadata": {"myapp.review": "pending", "other.kept": [1, 2]}}
        ]
    }))
    .expect("a configuration");
    InMemorySource::new(config).expect("a coherent source")
}

fn writable() -> InMemorySource {
    source(json!({"writes": "supported", "documents": "native"}))
}

fn key(value: &str) -> MetadataKey {
    MetadataKey::new(value).expect("a caller key")
}

fn id(value: &str) -> NativeId {
    NativeId(value.to_owned())
}

/// Every field of the record as JSON, with the metadata taken out, so "nothing else changed"
/// is one comparison.
fn without_metadata(record: &impl serde::Serialize) -> (Value, Value) {
    let mut value = serde_json::to_value(record).expect("a record serializes");
    let metadata = value
        .as_object_mut()
        .expect("a record is an object")
        .remove("metadata")
        .unwrap_or(Value::Null);
    (value, metadata)
}

#[tokio::test]
async fn a_task_key_is_added_and_replaced_and_nothing_else_moves() {
    let source = writable();
    let before = source.get_task(&id("T-1")).await.unwrap().expect("held");
    let (fields_before, _) = without_metadata(&before);

    let added = source
        .set_task_metadata(&id("T-1"), &key("myapp.approved"), &json!({"by": "nick"}))
        .await
        .expect("written")
        .expect("held");
    let replaced = source
        .set_task_metadata(&id("T-1"), &key("myapp.review"), &json!(true))
        .await
        .expect("written")
        .expect("held");

    let reread = source.get_task(&id("T-1")).await.unwrap().expect("held");
    assert_eq!(replaced, reread, "the answer is what a later read reports");
    assert_eq!(added.metadata["myapp.approved"], json!({"by": "nick"}));
    let (fields_after, metadata_after) = without_metadata(&reread);
    assert_eq!(fields_after, fields_before);
    assert_eq!(
        metadata_after,
        json!({"myapp.approved": {"by": "nick"}, "myapp.review": true, "other.kept": [1, 2]})
    );
    // Metadata is not status: the task it delivers is exactly as it was.
    let ticket = source.get_task(&id("T-2")).await.unwrap().expect("held");
    assert_eq!(ticket.status.name, "Todo");
    assert!(ticket.delivered_by.is_empty());
}

#[tokio::test]
async fn a_project_key_is_added_and_replaced_and_nothing_else_moves() {
    let source = writable();
    let before = source.get_project(&id("P-1")).await.unwrap().expect("held");
    let (fields_before, _) = without_metadata(&before);

    source
        .set_project_metadata(&id("P-1"), &key("myapp.approved"), &json!(null))
        .await
        .expect("written")
        .expect("held");
    let answered = source
        .set_project_metadata(&id("P-1"), &key("myapp.review"), &json!(["a", 1]))
        .await
        .expect("written")
        .expect("held");

    let reread = source.get_project(&id("P-1")).await.unwrap().expect("held");
    assert_eq!(answered, reread);
    let (fields_after, metadata_after) = without_metadata(&reread);
    assert_eq!(fields_after, fields_before);
    assert_eq!(
        metadata_after,
        json!({"myapp.approved": null, "myapp.review": ["a", 1], "other.kept": [1, 2]})
    );
}

#[tokio::test]
async fn a_document_key_is_added_and_replaced_and_nothing_else_moves() {
    let source = writable();
    let before = source
        .get_document(&id("D-1"))
        .await
        .unwrap()
        .expect("held");
    let (fields_before, _) = without_metadata(&before);

    source
        .set_document_metadata(&id("D-1"), &key("myapp.approved"), &json!(2.5))
        .await
        .expect("written")
        .expect("held");
    let answered = source
        .set_document_metadata(&id("D-1"), &key("myapp.review"), &json!("approved"))
        .await
        .expect("written")
        .expect("held");

    let reread = source
        .get_document(&id("D-1"))
        .await
        .unwrap()
        .expect("held");
    assert_eq!(answered, reread);
    let (fields_after, metadata_after) = without_metadata(&reread);
    assert_eq!(fields_after, fields_before);
    assert_eq!(
        metadata_after,
        json!({"myapp.approved": 2.5, "myapp.review": "approved", "other.kept": [1, 2]})
    );
}

#[tokio::test]
async fn setting_the_held_value_changes_nothing() {
    let source = writable();
    let task = source.get_task(&id("T-1")).await.unwrap();
    let project = source.get_project(&id("P-1")).await.unwrap();
    let document = source.get_document(&id("D-1")).await.unwrap();
    let pending = json!("pending");
    let review = key("myapp.review");

    assert_eq!(
        source
            .set_task_metadata(&id("T-1"), &review, &pending)
            .await
            .unwrap(),
        task
    );
    assert_eq!(
        source
            .set_project_metadata(&id("P-1"), &review, &pending)
            .await
            .unwrap(),
        project
    );
    assert_eq!(
        source
            .set_document_metadata(&id("D-1"), &review, &pending)
            .await
            .unwrap(),
        document
    );
    assert_eq!(source.get_task(&id("T-1")).await.unwrap(), task);
    assert_eq!(source.get_project(&id("P-1")).await.unwrap(), project);
    assert_eq!(source.get_document(&id("D-1")).await.unwrap(), document);
}

#[tokio::test]
async fn a_missing_id_answers_not_found() {
    let source = writable();
    let review = key("myapp.review");
    let value = json!(1);
    assert!(
        source
            .set_task_metadata(&id("T-9"), &review, &value)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        source
            .set_project_metadata(&id("T-1"), &review, &value)
            .await
            .unwrap()
            .is_none(),
        "a task's id names no project"
    );
    assert!(
        source
            .set_document_metadata(&id("D-9"), &review, &value)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn a_source_without_a_write_side_or_without_the_key_refuses_and_holds_what_it_held() {
    let read_only = source(json!({"writes": "unsupported", "documents": "native"}));
    let review = key("myapp.review");
    let refused = read_only
        .set_task_metadata(&id("T-1"), &review, &json!(1))
        .await
        .expect_err("no write side");
    assert_eq!(
        refused,
        SourceError::Refused {
            message: "the in-memory plugin cannot be written".to_owned()
        }
    );

    let keyless = source(json!({
        "writes": "supported", "documents": "native",
        "unwritable_metadata_keys": ["myapp.review"]
    }));
    let refused = keyless
        .set_project_metadata(&id("P-1"), &review, &json!(1))
        .await
        .expect_err("a key it cannot carry");
    assert!(
        matches!(&refused, SourceError::Refused { message } if message.contains("myapp.review")),
        "{refused:?}"
    );
    let held = keyless
        .get_project(&id("P-1"))
        .await
        .unwrap()
        .expect("held");
    assert_eq!(held.metadata["myapp.review"], json!("pending"));

    let documentless = InMemorySource::new(
        serde_json::from_value(json!({"capabilities": {"writes": "supported"}}))
            .expect("a configuration"),
    )
    .expect("a coherent source");
    let refused = documentless
        .set_document_metadata(&id("D-1"), &review, &json!(1))
        .await
        .expect_err("no documents");
    assert_eq!(
        refused,
        SourceError::Refused {
            message: "the in-memory plugin has no documents".to_owned()
        }
    );
}
