//! Every path that stores an item's content stores it byte for byte, however it ends.
//!
//! A rendered item's provenance records the SHA-256 of its content, so a write that moves one
//! trailing newline reads back as a hand edit nobody made. Each test drives the real source
//! through the trait, through each write a copy, a create, a rendering and a content set make,
//! with content ending in no newline, in one and in two, and reads it back.

use std::collections::BTreeMap;

use onetaskgraph_in_memory::{InMemoryConfig, InMemorySource};
use onetaskgraph_plugin_api::{
    Document, ItemWrite, MetadataKey, NativeId, Project, Task, TaskSource,
};
use serde_json::{Value, json};

/// The three endings, on content with interior structure a trim would also leave alone.
const ENDINGS: [&str; 3] = [
    "# Goal\n\n- two\n  lines",
    "# Goal\n\n- two\n  lines\n",
    "# Goal\n\n- two\n  lines\n\n",
];

fn source() -> InMemorySource {
    let config: InMemoryConfig = serde_json::from_value(json!({
        "capabilities": {"writes": "supported", "documents": "native"},
    }))
    .expect("a configuration");
    InMemorySource::new(config).expect("a coherent source")
}

fn answers() -> BTreeMap<String, Value> {
    serde_json::from_value(json!({"goal": "Ship it"})).unwrap()
}

/// The entry a rendering write carries. A plugin stores it as it stores any metadata value and
/// never reads its shape, which is the engine's, so an opaque value is all a write here needs.
fn provenance() -> Value {
    json!({"rendered": "by this test"})
}

fn task(content: &str) -> Task {
    serde_json::from_value(json!({
        "id": "T-1", "title": "Alpha", "content": content,
        "status": {"category": "todo", "name": "Todo"}, "labels": [],
        "metadata": {MetadataKey::TEMPLATE_KEY: provenance()},
    }))
    .expect("a task")
}

fn document(content: &str) -> Document {
    serde_json::from_value(
        json!({"id": "D-1", "title": "Design", "content": content, "labels": []}),
    )
    .expect("a document")
}

fn project(content: &str) -> Project {
    serde_json::from_value(json!({
        "id": "P-1", "title": "Launch", "content": content,
        "status": {"category": "todo", "name": "Todo"}, "labels": [],
    }))
    .expect("a project")
}

fn write<T>(target: Option<&NativeId>, item: T) -> ItemWrite<T> {
    ItemWrite {
        target: target.cloned(),
        item,
        depends_on: Vec::new(),
    }
}

async fn task_content(source: &InMemorySource, native: &NativeId) -> String {
    source
        .get_task(native)
        .await
        .unwrap()
        .unwrap()
        .content
        .unwrap()
}

async fn document_content(source: &InMemorySource, native: &NativeId) -> String {
    source
        .get_document(native)
        .await
        .unwrap()
        .unwrap()
        .content
        .unwrap()
}

#[tokio::test]
async fn every_task_write_keeps_the_content_exactly() {
    for content in ENDINGS {
        let source = source();
        let plain = source
            .write_task(&write(None, task(content)))
            .await
            .unwrap();
        assert_eq!(task_content(&source, &plain).await, content, "create");
        let rendered = source
            .write_task_rendered(&write(None, task(content)), &answers())
            .await
            .unwrap();
        assert_eq!(
            task_content(&source, &rendered).await,
            content,
            "rendered create"
        );
        for again in ENDINGS {
            source
                .write_task(&write(Some(&plain), task(again)))
                .await
                .unwrap();
            assert_eq!(task_content(&source, &plain).await, again, "update");
            source
                .write_task_rendered(&write(Some(&rendered), task(again)), &answers())
                .await
                .unwrap();
            assert_eq!(
                task_content(&source, &rendered).await,
                again,
                "rendered update"
            );
            source
                .set_task_rendering(&rendered, again, &provenance(), &answers())
                .await
                .unwrap()
                .expect("the task");
            assert_eq!(task_content(&source, &rendered).await, again, "rendering");
            source
                .set_task_content(&plain, again)
                .await
                .unwrap()
                .expect("the task");
            assert_eq!(task_content(&source, &plain).await, again, "content set");
        }
    }
}

#[tokio::test]
async fn every_document_and_project_write_keeps_the_content_exactly() {
    for content in ENDINGS {
        let source = source();
        let plain = source
            .write_document(&write(None, document(content)))
            .await
            .unwrap();
        assert_eq!(document_content(&source, &plain).await, content, "create");
        let rendered = source
            .write_document_rendered(&write(None, document(content)), &answers())
            .await
            .unwrap();
        assert_eq!(
            document_content(&source, &rendered).await,
            content,
            "rendered create"
        );
        let launch = source
            .write_project(&write(None, project(content)))
            .await
            .unwrap();
        for again in ENDINGS {
            source
                .write_document(&write(Some(&plain), document(again)))
                .await
                .unwrap();
            assert_eq!(document_content(&source, &plain).await, again, "update");
            source
                .write_document_rendered(&write(Some(&rendered), document(again)), &answers())
                .await
                .unwrap();
            assert_eq!(
                document_content(&source, &rendered).await,
                again,
                "rendered update"
            );
            source
                .set_document_rendering(&rendered, again, &provenance(), &answers())
                .await
                .unwrap()
                .expect("the document");
            assert_eq!(
                document_content(&source, &rendered).await,
                again,
                "rendering"
            );
            source
                .write_project(&write(Some(&launch), project(again)))
                .await
                .unwrap();
            assert_eq!(
                source
                    .get_project(&launch)
                    .await
                    .unwrap()
                    .unwrap()
                    .content
                    .as_deref(),
                Some(again),
                "project update"
            );
        }
    }
}

/// What a drift check hashes is what a read answers, so a hand edit of an interior line of a
/// copied rendering has to read back as the edit — neither normalised away nor lost.
#[tokio::test]
async fn an_interior_edit_of_a_copied_rendering_reads_back_as_the_edit() {
    for content in ENDINGS {
        let source = source();
        let copied = source
            .write_task(&write(None, task(content)))
            .await
            .unwrap();
        let edited = content.replacen("- two\n", "- three\n", 1);
        source
            .set_task_content(&copied, &edited)
            .await
            .unwrap()
            .expect("the task");
        let read = task_content(&source, &copied).await;
        assert_eq!(read, edited);
        assert_ne!(read, content, "the edit is visible to a drift check");
    }
}
