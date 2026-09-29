//! A task list narrowed by comment activity over a folder of Markdown: the comments section
//! each task file already holds is the only evidence, because a task here carries no
//! `updated_at` of its own.
//!
//! Every test drives the real plugin over a real folder.

use std::fs;

use chrono::{DateTime, Utc};
use onetaskgraph_plugin_api::{
    CommentBody, NativeId, NewComment, PageRequest, SecretResolver, SourceName, SourcePlugin,
    StatusCategory, Support, TaskQuery, TaskSource,
};
use secrecy::SecretString;
use serde_json::json;

struct NoSecrets;
impl SecretResolver for NoSecrets {
    fn get(&self, _: &str) -> Option<SecretString> {
        None
    }
}

/// The instant every query below asks about.
const SINCE: &str = "2026-09-20T12:00:00Z";

fn since() -> DateTime<Utc> {
    SINCE.parse().expect("an RFC 3339 instant")
}

/// One task file at `status`, with a comment for each `(created_at, updated_at)` pair.
fn task(status: &str, comments: &[(&str, &str)]) -> String {
    let mut text = format!("---\ntitle: A task\nstatus: {status}\n---\nThe body.\n");
    if !comments.is_empty() {
        text.push_str("\n## Comments\n");
        for (index, (created, updated)) in comments.iter().enumerate() {
            text.push_str(&format!(
                "\n<!-- onetaskgraph:comment id=\"c-{index}\" author=\"ada\" \
                 created_at=\"{created}\" updated_at=\"{updated}\" -->\n\
                 ### ada — {created}\n\nA word.\n\n<!-- /onetaskgraph:comment -->\n"
            ));
        }
    }
    text
}

/// Six tasks: commented after the instant, never commented, commented only before it, an old
/// comment edited after it, a comment exactly at it, and an old comment beside a new one — the
/// edited one done and every other open.
fn folder() -> (tempfile::TempDir, Box<dyn TaskSource>) {
    let root = tempfile::tempdir().expect("temporary notes");
    let tasks = root.path().join("tasks");
    fs::create_dir_all(&tasks).expect("the tasks folder");
    let old = "2026-09-01T09:00:00Z";
    for (name, text) in [
        (
            "new",
            task("todo", &[("2026-09-21T09:00:00Z", "2026-09-21T09:00:00Z")]),
        ),
        ("silent", task("todo", &[])),
        ("old", task("todo", &[(old, "2026-09-02T09:00:00Z")])),
        ("edited", task("done", &[(old, "2026-09-25T09:00:00Z")])),
        ("boundary", task("todo", &[(SINCE, SINCE)])),
        (
            "second",
            task(
                "todo",
                &[(old, old), ("2026-09-22T09:00:00Z", "2026-09-22T09:00:00Z")],
            ),
        ),
    ] {
        fs::write(tasks.join(format!("{name}.md")), text).expect("a task file");
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

async fn listed(source: &dyn TaskSource, query: &TaskQuery) -> Vec<String> {
    let mut ids: Vec<String> = source
        .query_tasks(
            query,
            &PageRequest {
                limit: 200,
                cursor: None,
            },
        )
        .await
        .expect("answers")
        .items
        .iter()
        .map(|task| task.id.to_string())
        .collect();
    ids.sort();
    ids
}

fn commented_since() -> TaskQuery {
    TaskQuery {
        commented_since: Some(since()),
        ..TaskQuery::default()
    }
}

#[tokio::test]
async fn a_task_is_kept_exactly_when_a_comment_was_created_or_edited_at_or_after_the_instant() {
    let (_root, source) = folder();
    assert_eq!(
        source.capabilities().filter_by_comment_activity,
        Support::Native
    );
    assert_eq!(
        listed(source.as_ref(), &commented_since()).await,
        ["boundary", "edited", "new", "second"],
        "the uncommented task and the one commented only before the instant are dropped"
    );
    assert_eq!(
        listed(source.as_ref(), &TaskQuery::default()).await,
        ["boundary", "edited", "new", "old", "second", "silent"],
        "no instant is no filter"
    );
}

#[tokio::test]
async fn combined_with_a_status_filter_the_answer_is_exactly_the_intersection() {
    let (_root, source) = folder();
    let with = |statuses: Vec<StatusCategory>| TaskQuery {
        statuses,
        ..commented_since()
    };
    assert_eq!(
        listed(source.as_ref(), &with(vec![StatusCategory::Todo])).await,
        ["boundary", "new", "second"]
    );
    assert_eq!(
        listed(source.as_ref(), &with(vec![StatusCategory::Done])).await,
        ["edited"]
    );
}

#[tokio::test]
async fn a_comment_written_or_removed_through_the_source_moves_the_answer() {
    let (_root, source) = folder();
    let silent = NativeId::from("silent");
    source
        .add_comment(
            &silent,
            &NewComment {
                author: None,
                body: CommentBody::new("Now it has one.").expect("a body"),
            },
        )
        .await
        .expect("adds it");
    assert_eq!(
        listed(source.as_ref(), &commented_since()).await,
        ["boundary", "edited", "new", "second", "silent"],
        "a comment added now was created after the instant"
    );
    source
        .delete_comment(&NativeId::from("new"), &NativeId::from("c-0"))
        .await
        .expect("removes it");
    assert_eq!(
        listed(source.as_ref(), &commented_since()).await,
        ["boundary", "edited", "second", "silent"],
        "a comment deleted before the query is not a match"
    );
}
