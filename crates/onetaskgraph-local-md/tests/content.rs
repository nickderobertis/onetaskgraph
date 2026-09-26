//! A task's content replaced on its own.
//!
//! Every test drives the real plugin over a real folder and asserts both on what a later
//! read answers, member by member, and on the bytes the file holds: the front matter and the
//! comments section are promises about the file, not only about what a read reports.

use std::fs;

use onetaskgraph_plugin_api::{
    Direction, NativeId, PageRequest, Priority, SecretResolver, SourceError, SourceName,
    SourcePlugin, StatusCategory, Task, TaskSource,
};
use secrecy::SecretString;
use serde_json::json;

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

fn page() -> PageRequest {
    PageRequest {
        limit: 200,
        cursor: None,
    }
}

fn refusal(error: SourceError) -> String {
    match error {
        SourceError::Refused { message } => message,
        other => panic!("expected a refusal naming the problem, got {other:?}"),
    }
}

/// Every member of the front matter this source reads, and a comments section.
const FRONT: &str = "---\ntitle: Alpha\nstatus: in progress\npriority: urgent\nlabels: [Bug, {id: ui, name: UI, color: '#00f'}]\nproject: launch\ndepends_on:\n  - b\n  - id: other:P-1\n    item: project\nmetadata:\n  caller.count: 3\n  caller.shape: {nested: [true, null]}\nrepositories: [github.com/acme/work]\ndelivers:\n- b\ndelivered_by: [\"plan:P-9\"]\n---\n";
const BODY: &str = "# Alpha\n\nThe body.\n\n";
const SECTION: &str = "## Comments\n\n<!-- onetaskgraph:comment id=\"20260913T151107Z-1\" author=\"ada\" created_at=\"2026-09-13T15:11:07Z\" updated_at=\"2026-09-13T15:11:07Z\" -->\n### ada — 2026-09-13T15:11:07Z\n\nKeep me.\n\n<!-- /onetaskgraph:comment -->\n";

/// Assert that `after` is `before` in every member but its content, one member at a time,
/// so a failure names the member that moved.
fn every_member_but_content_kept(before: &Task, after: &Task) {
    assert_eq!(after.id, before.id, "id");
    assert_eq!(after.key, before.key, "key");
    assert_eq!(after.title, before.title, "title");
    assert_eq!(after.status, before.status, "status");
    assert_eq!(after.priority, before.priority, "priority");
    assert_eq!(after.labels, before.labels, "labels");
    assert_eq!(after.project, before.project, "project");
    assert_eq!(after.url, before.url, "url");
    assert_eq!(after.location, before.location, "location");
    assert_eq!(after.metadata, before.metadata, "metadata");
    assert_eq!(after.repositories, before.repositories, "repositories");
    assert_eq!(after.delivers, before.delivers, "delivers");
    assert_eq!(after.delivered_by, before.delivered_by, "delivered_by");
}

#[tokio::test]
async fn a_content_set_replaces_the_body_and_keeps_the_front_matter_and_the_comments() {
    let original = format!("{FRONT}{BODY}{SECTION}");
    let (root, source) = folder(&[("tasks/a.md", &original)]);
    let before = source.get_task(&id("a")).await.unwrap().expect("held");
    // The fixture really does carry every member this asserts is kept.
    assert_eq!(before.status.category, StatusCategory::InProgress);
    assert_eq!(before.priority, Priority::Urgent);
    assert_eq!(before.labels.len(), 2);
    assert_eq!(before.metadata["caller.count"], json!(3));
    assert_eq!(before.repositories.len(), 1);
    assert_eq!(before.delivers.len(), 1);
    assert_eq!(before.delivered_by.len(), 1);
    let comments = source
        .task_comments(&id("a"), &page())
        .await
        .unwrap()
        .expect("held");
    let edges = source
        .task_dependencies(&id("a"), Direction::DependsOn, &page())
        .await
        .unwrap();
    assert_eq!(edges.items.len(), 2);

    for content in [
        "# Alpha\n\nA new body, on\nthree\nlines.",
        "# Alpha\r\n\r\nWindows line endings\r\nthroughout.",
        "# Alpha\n\nÜnïcödé — 日本語 ✓ and an emoji-free rest.",
        "# Alpha\n\nIt carries its own `## Comments` heading\n\n## Comments\n\nfollowed by prose.",
    ] {
        assert_eq!(
            source.set_task_content(&id("a"), content).await.unwrap(),
            Some(())
        );
        let after = source.get_task(&id("a")).await.unwrap().expect("held");
        assert_eq!(after.content.as_deref(), Some(content), "the content");
        every_member_but_content_kept(&before, &after);
        let text = read(&root, "tasks/a.md");
        assert_eq!(
            text,
            format!("{FRONT}{content}\n\n{SECTION}"),
            "the front matter and the section are the bytes they were, and the content is \
             the bytes given"
        );
        assert_eq!(
            source.task_comments(&id("a"), &page()).await.unwrap(),
            Some(comments.clone()),
            "the comments"
        );
        assert_eq!(
            source
                .task_dependencies(&id("a"), Direction::DependsOn, &page())
                .await
                .unwrap(),
            edges,
            "the dependencies"
        );
    }
}

#[tokio::test]
async fn content_ending_in_a_blank_line_needs_no_separator_before_the_section() {
    let (root, source) = folder(&[("tasks/a.md", &format!("{FRONT}{BODY}{SECTION}"))]);
    let content = "# Alpha\n\nEnds with a blank line.\n\n";
    source.set_task_content(&id("a"), content).await.unwrap();
    assert_eq!(
        read(&root, "tasks/a.md"),
        format!("{FRONT}{content}{SECTION}")
    );
}

#[tokio::test]
async fn a_file_with_no_comments_holds_exactly_the_front_matter_and_the_content() {
    let (root, source) = folder(&[("tasks/a.md", &format!("{FRONT}{BODY}"))]);
    let before = source.get_task(&id("a")).await.unwrap().expect("held");
    for content in [
        "# Alpha\n\nWith a trailing newline.\n",
        "# Alpha\r\n\r\nWith a CRLF one.\r\n",
        "# Alpha\n\nWith none at all.",
    ] {
        source.set_task_content(&id("a"), content).await.unwrap();
        assert_eq!(read(&root, "tasks/a.md"), format!("{FRONT}{content}"));
        let after = source.get_task(&id("a")).await.unwrap().expect("held");
        // This source reports every task's content with the whitespace around it trimmed,
        // so a trailing line ending is in the file and not in what a read answers.
        assert_eq!(after.content.as_deref(), Some(content.trim_end()));
        every_member_but_content_kept(&before, &after);
    }
    // Emptied: the task reads as having no content, and the front matter is untouched.
    source.set_task_content(&id("a"), "").await.unwrap();
    assert_eq!(read(&root, "tasks/a.md"), FRONT);
    let after = source.get_task(&id("a")).await.unwrap().expect("held");
    assert_eq!(after.content, None);
    every_member_but_content_kept(&before, &after);
}

#[tokio::test]
async fn a_windows_file_keeps_its_front_matter_byte_for_byte() {
    let front = "---\r\ntitle: Crlf\r\nstatus: todo\r\npriority: low\r\n---\r\n";
    let (root, source) = folder(&[("tasks/a.md", &format!("{front}old\r\n"))]);
    source
        .set_task_content(&id("a"), "new\r\ncontent")
        .await
        .unwrap();
    assert_eq!(read(&root, "tasks/a.md"), format!("{front}new\r\ncontent"));
    let task = source.get_task(&id("a")).await.unwrap().expect("held");
    assert_eq!(task.content.as_deref(), Some("new\r\ncontent"));
    assert_eq!(task.priority, Priority::Low);
}

#[tokio::test]
async fn content_that_would_read_as_a_comments_section_is_refused_and_nothing_is_written() {
    let original = format!("{FRONT}{BODY}");
    let (root, source) = folder(&[("tasks/a.md", &original)]);
    let message = refusal(
        source
            .set_task_content(&id("a"), &format!("# Alpha\n\n{SECTION}"))
            .await
            .expect_err("the content would become comments nobody wrote"),
    );
    assert!(
        message.contains("a.md")
            && message.contains("`content`")
            && message.contains("## Comments"),
        "{message}"
    );
    assert_eq!(read(&root, "tasks/a.md"), original);
}

#[tokio::test]
async fn content_that_would_retitle_a_task_with_no_title_key_is_refused() {
    let original = "---\nstatus: todo\n---\n# Headed\n\nbody\n";
    let (root, source) = folder(&[("tasks/a.md", original)]);
    let message = refusal(
        source
            .set_task_content(&id("a"), "# Renamed\n\nbody")
            .await
            .expect_err("the title comes from the heading"),
    );
    assert!(
        message.contains("\"Headed\"")
            && message.contains("\"Renamed\"")
            && message.contains("`title:`"),
        "{message}"
    );
    assert_eq!(read(&root, "tasks/a.md"), original);

    // Content keeping the heading is fine, because the title does not move.
    source
        .set_task_content(&id("a"), "# Headed\n\nanother body")
        .await
        .unwrap();
    let task = source.get_task(&id("a")).await.unwrap().expect("held");
    assert_eq!(task.title, "Headed");
    assert_eq!(task.content.as_deref(), Some("# Headed\n\nanother body"));
}

#[tokio::test]
async fn a_content_set_on_a_task_this_folder_does_not_hold_answers_none() {
    let (root, source) = folder(&[("tasks/a.md", "---\ntitle: A\n---\n")]);
    assert_eq!(
        source
            .set_task_content(&id("missing"), "body")
            .await
            .unwrap(),
        None
    );
    assert!(!root.path().join("tasks/missing.md").exists());
}
