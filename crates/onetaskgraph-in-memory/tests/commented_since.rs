//! The in-memory source's comment-activity filter — declared, applied over the comments it
//! holds beside each task, and ignored when a configuration declares it unsupported — driven
//! through the real trait and the plugin's own configuration reader.

use chrono::{DateTime, Utc};
use onetaskgraph_in_memory::Plugin;
use onetaskgraph_plugin_api::{
    NativeId, PageRequest, SecretResolver, SourceName, SourcePlugin, StatusCategory, Support,
    TaskQuery, TaskSource,
};
use secrecy::SecretString;
use serde_json::{Value, json};

struct NoSecrets;
impl SecretResolver for NoSecrets {
    fn get(&self, _var: &str) -> Option<SecretString> {
        None
    }
}

/// The instant every query below asks about.
const SINCE: &str = "2026-09-20T12:00:00Z";

fn since() -> DateTime<Utc> {
    SINCE.parse().expect("an RFC 3339 instant")
}

fn comment(id: &str, task: &str, created: &str, updated: &str) -> Value {
    json!({"task": task, "comment": {"id": id, "author": "ada", "created_at": created,
           "updated_at": updated, "body": "a word", "url": null}})
}

/// Five tasks: one commented after the instant, one never commented, one commented and left
/// alone before it, one whose old comment was edited after it, and one commented exactly at
/// it — the first four open, the edited one done.
fn source(capabilities: Value) -> Box<dyn TaskSource> {
    let task = |id: &str, category: &str| {
        json!({"id": id, "title": id, "content": null,
               "status": {"category": category, "name": category}, "labels": []})
    };
    let mut declared = json!({"comments": "native"});
    declared
        .as_object_mut()
        .expect("an object")
        .extend(capabilities.as_object().expect("an object").clone());
    Plugin
        .build(
            &SourceName::new("work").expect("a name"),
            &json!({
                "capabilities": declared,
                "tasks": [
                    task("T-new", "todo"),
                    task("T-silent", "todo"),
                    task("T-old", "todo"),
                    task("T-edited", "done"),
                    task("T-boundary", "todo"),
                ],
                "comments": [
                    comment("c-1", "T-new", "2026-09-21T09:00:00Z", "2026-09-21T09:00:00Z"),
                    comment("c-2", "T-old", "2026-09-01T09:00:00Z", "2026-09-02T09:00:00Z"),
                    comment("c-3", "T-edited", "2026-09-01T09:00:00Z", "2026-09-25T09:00:00Z"),
                    comment("c-4", "T-boundary", SINCE, SINCE),
                ]
            }),
            &NoSecrets,
        )
        .expect("builds")
}

async fn listed(source: &dyn TaskSource, query: &TaskQuery) -> Vec<String> {
    source
        .query_tasks(
            query,
            &PageRequest {
                cursor: None,
                limit: 50,
            },
        )
        .await
        .expect("answers")
        .items
        .iter()
        .map(|task| task.id.to_string())
        .collect()
}

fn commented_since() -> TaskQuery {
    TaskQuery {
        commented_since: Some(since()),
        ..TaskQuery::default()
    }
}

#[tokio::test]
async fn a_task_is_kept_exactly_when_a_comment_was_created_or_edited_at_or_after_the_instant() {
    let source = source(json!({}));
    assert_eq!(
        source.capabilities().filter_by_comment_activity,
        Support::Native
    );
    assert_eq!(
        listed(source.as_ref(), &commented_since()).await,
        ["T-new", "T-edited", "T-boundary"],
        "the uncommented task and the one commented only before the instant are dropped"
    );
    assert_eq!(
        listed(source.as_ref(), &TaskQuery::default()).await,
        ["T-new", "T-silent", "T-old", "T-edited", "T-boundary"],
        "no instant is no filter"
    );
}

#[tokio::test]
async fn combined_with_a_status_filter_the_answer_is_exactly_the_intersection() {
    let source = source(json!({}));
    let with = |statuses: Vec<StatusCategory>| TaskQuery {
        statuses,
        ..commented_since()
    };
    assert_eq!(
        listed(source.as_ref(), &with(vec![StatusCategory::Todo])).await,
        ["T-new", "T-boundary"]
    );
    assert_eq!(
        listed(source.as_ref(), &with(vec![StatusCategory::Done])).await,
        ["T-edited"]
    );
}

#[tokio::test]
async fn a_comment_deleted_before_the_query_is_not_a_match() {
    let source = source(json!({}));
    source
        .delete_comment(&NativeId::from("T-new"), &NativeId::from("c-1"))
        .await
        .expect("removes it");
    assert_eq!(
        listed(source.as_ref(), &commented_since()).await,
        ["T-edited", "T-boundary"]
    );
}

#[tokio::test]
async fn a_source_declaring_the_filter_unsupported_returns_the_wider_set() {
    let ignoring = source(json!({"filter_by_comment_activity": "unsupported"}));
    assert_eq!(
        ignoring.capabilities().filter_by_comment_activity,
        Support::Unsupported
    );
    assert_eq!(
        listed(ignoring.as_ref(), &commented_since()).await,
        ["T-new", "T-silent", "T-old", "T-edited", "T-boundary"]
    );
}
