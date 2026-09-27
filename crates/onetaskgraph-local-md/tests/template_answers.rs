//! The template answers a task or a document stores in its own file, and the rendering write
//! that replaces its content, its provenance and those answers together.
//!
//! Every test drives the real plugin over a real folder and asserts both on what a later read
//! answers and on the bytes the file holds: the block is a promise about the file — in neither
//! the content nor the metadata, kept byte for byte by every other write — not only about
//! what a read reports.

use std::collections::BTreeMap;
use std::fs;

use onetaskgraph_local_md::{ANSWERS_CLOSE, ANSWERS_OPEN};
use onetaskgraph_plugin_api::{
    CommentBody, Document, ItemWrite, MetadataKey, NativeId, NewComment, Priority, SecretResolver,
    SourceError, SourceName, SourcePlugin, Status, StatusCategory, Task, TaskSource,
};
use secrecy::SecretString;
use serde_json::{Value, json};

struct NoSecrets;
impl SecretResolver for NoSecrets {
    fn get(&self, _: &str) -> Option<SecretString> {
        None
    }
}

/// A folder named `work`, holding `files`.
fn folder(files: &[(&str, &str)]) -> (tempfile::TempDir, Box<dyn TaskSource>) {
    let root = tempfile::tempdir().expect("temporary notes");
    for (relative, text) in files {
        let path = root.path().join(relative);
        fs::create_dir_all(path.parent().expect("a file has a folder")).expect("folders");
        fs::write(&path, text).expect("a file");
    }
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

fn read(root: &tempfile::TempDir, relative: &str) -> String {
    fs::read_to_string(root.path().join(relative)).expect("the file reads")
}

/// Every file name in one folder, so a test can say no staging file was left there.
fn names(root: &tempfile::TempDir, folder: &str) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(root.path().join(folder))
        .expect("the folder lists")
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn answers(value: Value) -> BTreeMap<String, Value> {
    serde_json::from_value(value).expect("answers are a mapping")
}

fn provenance(template: &str) -> Value {
    json!({
        "template": template,
        "digest": format!("sha256:{}", "a".repeat(64)),
        "body_digest": format!("sha256:{}", "b".repeat(64)),
        "answers_digest": format!("sha256:{}", "c".repeat(64)),
    })
}

/// A task file with a comments section and a hand-written answers block above it.
const HAND_WRITTEN: &str = "---\ntitle: Alpha\nstatus: todo\nmetadata:\n  caller.count: 3\n---\nThe rendered goal.\n\n<!-- onetaskgraph:template-answers\ngoal: Ship it\nsteps:\n- build\n- publish\nlimit: 3\n-->\n\n## Comments\n\n<!-- onetaskgraph:comment id=\"20260913T151107Z-1\" author=\"ada\" created_at=\"2026-09-13T15:11:07Z\" updated_at=\"2026-09-13T15:11:07Z\" -->\n### ada — 2026-09-13T15:11:07Z\n\nKeep me.\n\n<!-- /onetaskgraph:comment -->\n";

/// The block exactly as [`HAND_WRITTEN`] holds it.
const HAND_BLOCK: &str = "<!-- onetaskgraph:template-answers\ngoal: Ship it\nsteps:\n- build\n- publish\nlimit: 3\n-->\n";

fn task(title: &str, content: &str, metadata: Value) -> Task {
    Task {
        id: id(title),
        key: None,
        title: title.to_owned(),
        content: Some(content.to_owned()),
        status: Status {
            category: StatusCategory::Todo,
            name: "todo".to_owned(),
        },
        priority: Priority::None,
        labels: Vec::new(),
        project: Some(id("launch")),
        url: None,
        location: None,
        created_at: None,
        updated_at: None,
        metadata: serde_json::from_value(metadata).unwrap(),
        repositories: Vec::new(),
        delivers: Vec::new(),
        delivered_by: Vec::new(),
    }
}

#[tokio::test]
async fn a_hand_written_block_is_in_neither_the_content_nor_the_metadata_and_reads_as_answers() {
    let (_root, source) = folder(&[("tasks/a.md", HAND_WRITTEN)]);

    let read = source.get_task(&id("a")).await.unwrap().expect("the task");
    assert_eq!(read.content.as_deref(), Some("The rendered goal."));
    assert_eq!(read.metadata, answers(json!({"caller.count": 3})));
    assert_eq!(
        source.task_template_answers(&id("a")).await.unwrap(),
        Some(answers(
            json!({"goal":"Ship it","steps":["build","publish"],"limit":3})
        )),
        "the answers come back with their JSON types intact"
    );
    let comments = source
        .task_comments(
            &id("a"),
            &onetaskgraph_plugin_api::PageRequest {
                limit: 10,
                cursor: None,
            },
        )
        .await
        .unwrap()
        .expect("the task's comments");
    assert_eq!(
        comments.items.len(),
        1,
        "the section is still read as comments"
    );
}

#[tokio::test]
async fn every_other_write_keeps_a_hand_written_block_byte_for_byte() {
    let (root, source) = folder(&[("tasks/a.md", HAND_WRITTEN)]);

    source
        .set_task_status(&id("a"), StatusCategory::Done)
        .await
        .unwrap();
    source
        .set_task_metadata(
            &id("a"),
            &MetadataKey::new("caller.more").unwrap(),
            &json!(true),
        )
        .await
        .unwrap();
    source
        .set_task_content(&id("a"), "A new goal, by hand.")
        .await
        .unwrap();
    source
        .add_comment(
            &id("a"),
            &NewComment {
                body: CommentBody::new("Another.".to_owned()).unwrap(),
                author: None,
            },
        )
        .await
        .unwrap();

    let text = read(&root, "tasks/a.md");
    assert!(
        text.contains(&format!(
            "A new goal, by hand.\n\n{HAND_BLOCK}\n## Comments\n"
        )),
        "the block sits between the new content and the section, as it was:\n{text}"
    );
    let task = source.get_task(&id("a")).await.unwrap().unwrap();
    assert_eq!(task.content.as_deref(), Some("A new goal, by hand."));
    assert_eq!(
        source.task_template_answers(&id("a")).await.unwrap(),
        Some(answers(
            json!({"goal":"Ship it","steps":["build","publish"],"limit":3})
        ))
    );
}

#[tokio::test]
async fn a_rendered_create_stores_the_content_exactly_and_the_answers_after_it() {
    let (root, source) = folder(&[]);
    let written = answers(json!({"goal":"Ship it","count":2,"notes":null}));
    let item = task(
        "Alpha",
        "# Goal\n\nShip it\n",
        json!({
            MetadataKey::TEMPLATE_KEY: provenance("/templates/task.md"),
            "caller.kept": "yes",
        }),
    );

    let native = source
        .write_task_rendered(
            &ItemWrite {
                target: None,
                item,
                depends_on: Vec::new(),
            },
            &written,
        )
        .await
        .unwrap();

    let text = read(&root, &format!("tasks/{}.md", native.0));
    assert!(
        text.ends_with(&format!(
            "---\n# Goal\n\nShip it\n\n\n{ANSWERS_OPEN}\ncount: 2\ngoal: Ship it\nnotes: null\n{ANSWERS_CLOSE}\n"
        )),
        "the content keeps its trailing newline, then the block:\n{text}"
    );
    let back = source.get_task(&native).await.unwrap().unwrap();
    assert_eq!(back.content.as_deref(), Some("# Goal\n\nShip it\n"));
    assert_eq!(
        back.metadata[MetadataKey::TEMPLATE_KEY],
        provenance("/templates/task.md")
    );
    assert!(
        !back.metadata.keys().any(|key| key == "goal"),
        "no answer is metadata"
    );
    assert_eq!(
        source.task_template_answers(&native).await.unwrap(),
        Some(written)
    );
}

#[tokio::test]
async fn a_plain_write_stores_no_answers_and_a_copy_over_a_rendered_file_leaves_none() {
    let (root, source) = folder(&[("tasks/a.md", HAND_WRITTEN)]);

    let plain = source
        .write_task(&ItemWrite {
            target: None,
            item: task("Plain", "Just a body", json!({})),
            depends_on: Vec::new(),
        })
        .await
        .unwrap();
    assert_eq!(source.task_template_answers(&plain).await.unwrap(), None);

    source
        .write_task(&ItemWrite {
            target: Some(id("a")),
            item: task("Alpha", "Copied content", json!({})),
            depends_on: Vec::new(),
        })
        .await
        .unwrap();
    let text = read(&root, "tasks/a.md");
    assert!(
        !text.contains(ANSWERS_OPEN),
        "a copy carries no answers:\n{text}"
    );
    assert!(
        text.contains("## Comments"),
        "and keeps the comments, as a copy does"
    );
    assert_eq!(source.task_template_answers(&id("a")).await.unwrap(), None);
}

#[tokio::test]
async fn a_rendering_replaces_content_provenance_and_answers_and_nothing_else() {
    let (root, source) = folder(&[("tasks/a.md", HAND_WRITTEN)]);
    let before = source.get_task(&id("a")).await.unwrap().unwrap();
    let section = &HAND_WRITTEN[HAND_WRITTEN.find("## Comments").unwrap()..];

    let written = answers(json!({"goal":"Ship it again","steps":["build"]}));
    source
        .set_task_rendering(
            &id("a"),
            "The goal, rendered again.\n",
            &provenance("/t.md"),
            &written,
        )
        .await
        .unwrap()
        .expect("the task is there");

    let text = read(&root, "tasks/a.md");
    assert!(
        text.starts_with("---\ntitle: Alpha\nstatus: todo\nmetadata:\n  caller.count: 3\n"),
        "the front matter is edited by the one entry:\n{text}"
    );
    assert!(
        text.ends_with(section),
        "the comments section byte for byte:\n{text}"
    );
    let after = source.get_task(&id("a")).await.unwrap().unwrap();
    assert_eq!(
        after.content.as_deref(),
        Some("The goal, rendered again.\n")
    );
    let mut metadata = before.metadata.clone();
    metadata.insert(MetadataKey::TEMPLATE_KEY.to_owned(), provenance("/t.md"));
    assert_eq!(after.metadata, metadata);
    assert_eq!(
        Task {
            content: before.content.clone(),
            metadata: before.metadata.clone(),
            ..after
        },
        before,
        "every other member is as it was"
    );
    assert_eq!(
        source.task_template_answers(&id("a")).await.unwrap(),
        Some(written.clone())
    );

    // The same rendering again is no write at all — even of a file this process may not write.
    let unchanged = read(&root, "tasks/a.md");
    read_only(&root.path().join("tasks/a.md"));
    source
        .set_task_rendering(
            &id("a"),
            "The goal, rendered again.\n",
            &provenance("/t.md"),
            &written,
        )
        .await
        .unwrap()
        .expect("the task is there");
    assert_eq!(read(&root, "tasks/a.md"), unchanged);
}

#[cfg(unix)]
fn read_only(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt as _;
    let mode = if path.is_dir() { 0o555 } else { 0o444 };
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

#[cfg(unix)]
fn writable(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt as _;
    let mode = if path.is_dir() { 0o755 } else { 0o644 };
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

#[cfg(not(unix))]
fn read_only(path: &std::path::Path) {
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(path, permissions).unwrap();
}

#[cfg(not(unix))]
fn writable(path: &std::path::Path) {
    let mut permissions = fs::metadata(path).unwrap().permissions();
    #[allow(
        clippy::permissions_set_readonly_false,
        reason = "restoring a test file"
    )]
    permissions.set_readonly(false);
    fs::set_permissions(path, permissions).unwrap();
}

#[tokio::test]
async fn a_rendering_of_a_file_it_cannot_write_leaves_the_file_and_no_staging_file() {
    let (root, source) = folder(&[("tasks/a.md", HAND_WRITTEN)]);
    let file = root.path().join("tasks/a.md");
    read_only(&file);

    let error = source
        .set_task_rendering(&id("a"), "New.", &provenance("/t.md"), &answers(json!({})))
        .await
        .expect_err("an unwritable file is refused");
    writable(&file);

    assert!(
        matches!(error, SourceError::Unavailable { .. }),
        "{error:?}"
    );
    assert_eq!(read(&root, "tasks/a.md"), HAND_WRITTEN, "byte-identical");
    assert_eq!(names(&root, "tasks"), vec!["a.md".to_owned()]);
}

// A directory's permissions do not stop a Windows rename, so the folder half is Unix's.
#[cfg(unix)]
#[tokio::test]
async fn a_rendering_or_a_rendered_create_in_a_folder_it_cannot_write_writes_nothing() {
    let (root, source) = folder(&[("tasks/a.md", HAND_WRITTEN)]);
    let tasks = root.path().join("tasks");
    read_only(&tasks);

    let rendering = source
        .set_task_rendering(&id("a"), "New.", &provenance("/t.md"), &answers(json!({})))
        .await;
    let create = source
        .write_task_rendered(
            &ItemWrite {
                target: None,
                item: task("Beta", "New.", json!({})),
                depends_on: Vec::new(),
            },
            &answers(json!({"goal": "x"})),
        )
        .await;
    writable(&tasks);

    assert!(
        rendering.is_err() && create.is_err(),
        "{rendering:?} {create:?}"
    );
    assert_eq!(read(&root, "tasks/a.md"), HAND_WRITTEN, "byte-identical");
    assert_eq!(
        names(&root, "tasks"),
        vec!["a.md".to_owned()],
        "and nothing staged"
    );
    assert!(
        !names(&root, "tasks")
            .iter()
            .any(|name| name.ends_with(onetaskgraph_local_md::STAGING_SUFFIX))
    );
}

#[tokio::test]
async fn content_that_would_read_back_as_an_answers_block_is_refused_where_none_follows_it() {
    let plain = "---\ntitle: Plain\nstatus: todo\n---\nBody.\n";
    let (root, source) = folder(&[("tasks/a.md", HAND_WRITTEN), ("tasks/plain.md", plain)]);
    let forged = format!("Text.\n\n{ANSWERS_OPEN}\nforged: true\n{ANSWERS_CLOSE}\n");

    // With no real block after it, a block-shaped ending would read back as answers nobody
    // gave, so it is refused and the file is left as it was.
    let refused = source.set_task_content(&id("plain"), &forged).await;
    assert!(
        matches!(refused, Err(SourceError::Refused { .. })),
        "{refused:?}"
    );
    assert_eq!(read(&root, "tasks/plain.md"), plain);
    assert_eq!(
        source.task_template_answers(&id("plain")).await.unwrap(),
        None
    );

    // Above a real block — one a rendering write puts there, or one already in the file — it
    // is content like any other: the real block is the last thing above the section.
    let created = source
        .write_task_rendered(
            &ItemWrite {
                target: None,
                item: task("Beta", &forged, json!({})),
                depends_on: Vec::new(),
            },
            &answers(json!({"real": 1})),
        )
        .await
        .expect("a rendered create writes its own block last");
    assert_eq!(
        source
            .get_task(&created)
            .await
            .unwrap()
            .unwrap()
            .content
            .as_deref(),
        Some(forged.as_str())
    );
    assert_eq!(
        source.task_template_answers(&created).await.unwrap(),
        Some(answers(json!({"real": 1})))
    );
    source
        .set_task_content(&id("a"), &forged)
        .await
        .unwrap()
        .expect("content above a real block");
    assert_eq!(
        source
            .get_task(&id("a"))
            .await
            .unwrap()
            .unwrap()
            .content
            .as_deref(),
        Some(forged.as_str())
    );
    assert_eq!(
        source.task_template_answers(&id("a")).await.unwrap(),
        Some(answers(
            json!({"goal":"Ship it","steps":["build","publish"],"limit":3})
        ))
    );
}

#[tokio::test]
async fn a_rendering_that_would_retitle_an_untitled_task_is_refused() {
    let untitled = "---\nstatus: todo\n---\n# Old heading\n\nBody.\n";
    let (root, source) = folder(&[("tasks/a.md", untitled)]);

    let error = source
        .set_task_rendering(
            &id("a"),
            "# New heading\n",
            &provenance("/t.md"),
            &answers(json!({})),
        )
        .await
        .expect_err("a retitle is refused");

    let SourceError::Refused { message } = error else {
        panic!("a refusal");
    };
    assert!(message.contains("no `title:`"), "{message}");
    assert_eq!(read(&root, "tasks/a.md"), untitled);
}

#[tokio::test]
async fn documents_store_answers_and_take_a_rendering_on_the_same_terms() {
    let (root, source) = folder(&[]);
    let written = answers(json!({"summary":"Why"}));
    let document = Document {
        id: id("design"),
        title: "Design".to_owned(),
        content: Some("Why.\n".to_owned()),
        project: Some(id("launch")),
        labels: Vec::new(),
        url: None,
        location: None,
        created_at: None,
        updated_at: None,
        metadata: answers(json!({MetadataKey::TEMPLATE_KEY: provenance("/d.md")})),
        repositories: Vec::new(),
    };
    let native = source
        .write_document_rendered(
            &ItemWrite {
                target: None,
                item: document,
                depends_on: Vec::new(),
            },
            &written,
        )
        .await
        .unwrap();
    assert_eq!(native, id("design"));
    assert_eq!(
        source.document_template_answers(&native).await.unwrap(),
        Some(written)
    );
    assert_eq!(
        source
            .get_document(&native)
            .await
            .unwrap()
            .unwrap()
            .content
            .as_deref(),
        Some("Why.\n")
    );

    let again = answers(json!({"summary":"Why not"}));
    source
        .set_document_rendering(&native, "Why not.\n", &provenance("/d.md"), &again)
        .await
        .unwrap()
        .expect("the document is there");
    let read_back = source.get_document(&native).await.unwrap().unwrap();
    assert_eq!(read_back.content.as_deref(), Some("Why not.\n"));
    assert_eq!(
        source.document_template_answers(&native).await.unwrap(),
        Some(again)
    );
    assert!(read(&root, "documents/design.md").contains("summary: Why not\n-->\n"));
    assert_eq!(
        source
            .set_document_rendering(
                &id("nothing"),
                "x",
                &provenance("/d.md"),
                &answers(json!({}))
            )
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn a_project_keeps_no_answers_and_a_block_in_one_is_its_content() {
    let text = "---\ntitle: Launch\nstatus: todo\n---\nPlan.\n\n<!-- onetaskgraph:template-answers\ngoal: x\n-->\n";
    let (_root, source) = folder(&[("projects/launch.md", text)]);

    let project = source.get_project(&id("launch")).await.unwrap().unwrap();
    assert!(project.content.unwrap().contains(ANSWERS_OPEN));
}
