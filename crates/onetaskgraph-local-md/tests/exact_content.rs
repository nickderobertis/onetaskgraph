//! Every path that stores an item's content stores it byte for byte, however it ends.
//!
//! A rendered item's provenance records the SHA-256 of its content, so a write that moves one
//! trailing newline reads back as a hand edit nobody made. Between them the tests drive the
//! real plugin over a real folder, through every write a copy, a create, a rendering and a
//! content set make, with content ending in no newline, in one and in two, and read it back.

use std::collections::BTreeMap;
use std::fs;

use onetaskgraph_plugin_api::{
    CommentBody, Document, ItemWrite, MetadataKey, NativeId, NewComment, Priority, Project,
    SecretResolver, SourceName, SourcePlugin, Status, StatusCategory, Task, TaskSource,
};
use secrecy::SecretString;
use serde_json::{Value, json};

struct NoSecrets;
impl SecretResolver for NoSecrets {
    fn get(&self, _: &str) -> Option<SecretString> {
        None
    }
}

/// The three endings, on content with interior structure a trim would also leave alone.
const ENDINGS: [&str; 3] = [
    "# Goal\n\n- two\n  lines",
    "# Goal\n\n- two\n  lines\n",
    "# Goal\n\n- two\n  lines\n\n",
];

fn folder() -> (tempfile::TempDir, Box<dyn TaskSource>) {
    let root = tempfile::tempdir().expect("temporary notes");
    let source = onetaskgraph_local_md::Plugin
        .build(
            &SourceName::new("work").unwrap(),
            &json!({ "root": root.path() }),
            &NoSecrets,
        )
        .expect("the folder builds");
    (root, source)
}

fn id(value: &str) -> NativeId {
    NativeId(value.to_owned())
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
    Task {
        id: id("alpha"),
        key: None,
        title: "Alpha".to_owned(),
        content: Some(content.to_owned()),
        status: Status {
            category: StatusCategory::Todo,
            name: "todo".to_owned(),
        },
        priority: Priority::None,
        labels: Vec::new(),
        project: None,
        url: None,
        location: None,
        created_at: None,
        updated_at: None,
        metadata: BTreeMap::from([(MetadataKey::TEMPLATE_KEY.to_owned(), provenance())]),
        repositories: Vec::new(),
        delivers: Vec::new(),
        delivered_by: Vec::new(),
    }
}

fn document(content: &str) -> Document {
    Document {
        id: id("design"),
        title: "Design".to_owned(),
        content: Some(content.to_owned()),
        project: None,
        labels: Vec::new(),
        url: None,
        location: None,
        created_at: None,
        updated_at: None,
        metadata: BTreeMap::new(),
        repositories: Vec::new(),
    }
}

fn project(content: &str) -> Project {
    Project {
        id: id("launch"),
        title: "Launch".to_owned(),
        content: Some(content.to_owned()),
        status: Status {
            category: StatusCategory::Todo,
            name: "todo".to_owned(),
        },
        labels: Vec::new(),
        url: None,
        location: None,
        created_at: None,
        updated_at: None,
        metadata: BTreeMap::new(),
        repositories: Vec::new(),
    }
}

fn write<T>(target: Option<&NativeId>, item: T) -> ItemWrite<T> {
    ItemWrite {
        target: target.cloned(),
        item,
        depends_on: Vec::new(),
    }
}

async fn task_content(source: &dyn TaskSource, native: &NativeId) -> String {
    source
        .get_task(native)
        .await
        .unwrap()
        .expect("the task")
        .content
        .expect("content")
}

async fn document_content(source: &dyn TaskSource, native: &NativeId) -> String {
    source
        .get_document(native)
        .await
        .unwrap()
        .expect("the document")
        .content
        .expect("content")
}

#[tokio::test]
async fn a_plain_task_create_and_update_the_copy_verb_makes_keep_the_content_exactly() {
    for content in ENDINGS {
        let (root, source) = folder();
        let created = source
            .write_task(&write(None, task(content)))
            .await
            .unwrap();
        assert_eq!(task_content(&*source, &created).await, content, "create");
        let file = fs::read_to_string(root.path().join("tasks/alpha.md")).unwrap();
        assert!(
            file.ends_with(&format!("---\n{content}\n")),
            "the file holds the content and one line break after it:\n{file:?}"
        );

        // An update over a file that already has a comments section, which a copy keeps.
        source
            .add_comment(
                &created,
                &NewComment {
                    body: CommentBody::new("Keep me.".to_owned()).unwrap(),
                    author: None,
                },
            )
            .await
            .unwrap();
        for again in ENDINGS {
            source
                .write_task(&write(Some(&created), task(again)))
                .await
                .unwrap();
            assert_eq!(task_content(&*source, &created).await, again, "update");
        }
        let comments = source
            .task_comments(
                &created,
                &onetaskgraph_plugin_api::PageRequest {
                    limit: 10,
                    cursor: None,
                },
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(comments.items.len(), 1, "the section is still its comments");
    }
}

#[tokio::test]
async fn a_rendered_task_create_rendering_write_and_content_set_keep_the_content_exactly() {
    for content in ENDINGS {
        let (_root, source) = folder();
        let created = source
            .write_task_rendered(&write(None, task(content)), &answers())
            .await
            .unwrap();
        assert_eq!(task_content(&*source, &created).await, content, "create");
        for again in ENDINGS {
            source
                .write_task_rendered(&write(Some(&created), task(again)), &answers())
                .await
                .unwrap();
            assert_eq!(
                task_content(&*source, &created).await,
                again,
                "rendered update"
            );
            source
                .set_task_rendering(&created, again, &provenance(), &answers())
                .await
                .unwrap()
                .expect("the task");
            assert_eq!(task_content(&*source, &created).await, again, "rendering");
            source
                .set_task_content(&created, again)
                .await
                .unwrap()
                .expect("the task");
            assert_eq!(task_content(&*source, &created).await, again, "content set");
        }
        assert_eq!(
            source.task_template_answers(&created).await.unwrap(),
            Some(answers())
        );
    }
}

#[tokio::test]
async fn every_document_and_project_write_keeps_the_content_exactly() {
    for content in ENDINGS {
        let (_root, source) = folder();
        let plain = source
            .write_document(&write(None, document(content)))
            .await
            .unwrap();
        assert_eq!(document_content(&*source, &plain).await, content, "create");
        let rendered = source
            .write_document_rendered(&write(None, document(content)), &answers())
            .await
            .unwrap();
        assert_eq!(
            document_content(&*source, &rendered).await,
            content,
            "rendered create"
        );
        let launch = source
            .write_project(&write(None, project(content)))
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
            Some(content),
            "project create"
        );
        for again in ENDINGS {
            source
                .write_document(&write(Some(&plain), document(again)))
                .await
                .unwrap();
            assert_eq!(document_content(&*source, &plain).await, again, "update");
            source
                .write_document_rendered(&write(Some(&rendered), document(again)), &answers())
                .await
                .unwrap();
            assert_eq!(
                document_content(&*source, &rendered).await,
                again,
                "rendered update"
            );
            source
                .set_document_rendering(&rendered, again, &provenance(), &answers())
                .await
                .unwrap()
                .expect("the document");
            assert_eq!(
                document_content(&*source, &rendered).await,
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

fn refuses_content<T: std::fmt::Debug>(result: &Result<T, onetaskgraph_plugin_api::SourceError>) {
    assert!(
        matches!(
            result,
            Err(onetaskgraph_plugin_api::SourceError::Refused { message })
                if message.contains("`content`")
        ),
        "{result:?}"
    );
}

fn files(root: &tempfile::TempDir) -> BTreeMap<String, Vec<u8>> {
    let mut found = BTreeMap::new();
    let mut folders = vec![root.path().to_path_buf()];
    while let Some(folder) = folders.pop() {
        for entry in fs::read_dir(&folder).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                folders.push(path);
            } else {
                let name = path
                    .strip_prefix(root.path())
                    .unwrap()
                    .display()
                    .to_string();
                found.insert(name, fs::read(&path).unwrap());
            }
        }
    }
    found
}

#[tokio::test]
async fn a_plain_write_whose_content_would_read_back_otherwise_is_refused_and_writes_nothing() {
    let (root, source) = folder();
    let task_id = source
        .write_task(&write(None, task(ENDINGS[1])))
        .await
        .unwrap();
    let document_id = source
        .write_document(&write(None, document(ENDINGS[1])))
        .await
        .unwrap();
    let project_id = source
        .write_project(&write(None, project(ENDINGS[1])))
        .await
        .unwrap();
    let before = files(&root);

    // Content ending in what a task or a document reads as its stored answers would read back
    // without that block, and content ending in a lone carriage return without it.
    let forged = format!(
        "Text.\n\n{}\nforged: true\n{}\n",
        onetaskgraph_local_md::ANSWERS_OPEN,
        onetaskgraph_local_md::ANSWERS_CLOSE
    );
    for content in [forged.as_str(), "Ends in a lone carriage return\r"] {
        refuses_content(&source.write_task(&write(None, task(content))).await);
        refuses_content(
            &source
                .write_task(&write(Some(&task_id), task(content)))
                .await,
        );
        refuses_content(&source.write_document(&write(None, document(content))).await);
        refuses_content(
            &source
                .write_document(&write(Some(&document_id), document(content)))
                .await,
        );
    }
    // A project keeps no answers, so only the carriage return cannot be held.
    let lone = "Ends in a lone carriage return\r";
    refuses_content(&source.write_project(&write(None, project(lone))).await);
    refuses_content(
        &source
            .write_project(&write(Some(&project_id), project(lone)))
            .await,
    );

    assert_eq!(files(&root), before, "no file is created or changed");
    assert_eq!(task_content(&*source, &task_id).await, ENDINGS[1]);
    assert_eq!(document_content(&*source, &document_id).await, ENDINGS[1]);
}
