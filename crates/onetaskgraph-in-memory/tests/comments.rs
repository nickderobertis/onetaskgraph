//! A task's comments on the in-memory source, driven through the real factory and trait.
//!
//! Configuration is spelled inline rather than through `tests/common`, because what these
//! tests turn on is exactly the two keys that module's fixture leaves out: the `comments`
//! declaration and the comments a source starts with.

use onetaskgraph_in_memory::Plugin;
use onetaskgraph_plugin_api::{
    Comment, CommentBody, ItemWrite, NativeId, NewComment, PageRequest, SecretResolver,
    SourceError, SourceName, SourcePlugin, TaskSource, commentless, unwritable,
};
use secrecy::SecretString;
use serde_json::{Value, json};

struct NoSecrets;
impl SecretResolver for NoSecrets {
    fn get(&self, _var: &str) -> Option<SecretString> {
        None
    }
}

/// Two tasks, the first starting with one comment, under `capabilities`.
fn configured(capabilities: Value) -> Value {
    json!({
        "capabilities": capabilities,
        "tasks": [task("T-1"), task("T-2")],
        "comments": [{
            "task": "T-1",
            "comment": {
                "id": "C-1",
                "author": "ada",
                "created_at": "2026-09-13T15:11:07Z",
                "updated_at": "2026-09-13T15:11:07Z",
                "body": "seeded\n",
                "url": null
            }
        }],
    })
}

fn task(id: &str) -> Value {
    json!({
        "id": id, "title": id, "content": null,
        "status": {"category": "todo", "name": "Todo"},
        "labels": [], "project": null, "url": null, "created_at": null, "updated_at": null
    })
}

fn build(config: &Value) -> Result<Box<dyn TaskSource>, SourceError> {
    Plugin.build(&SourceName::new("notes").unwrap(), config, &NoSecrets)
}

fn commented() -> Box<dyn TaskSource> {
    build(&configured(json!({"comments": "native"}))).expect("a coherent source")
}

fn id(text: &str) -> NativeId {
    NativeId(text.to_owned())
}

fn page(limit: u32) -> PageRequest {
    PageRequest {
        cursor: None,
        limit,
    }
}

fn new(text: &str, author: Option<&str>) -> NewComment {
    NewComment {
        body: CommentBody::new(text).unwrap(),
        author: author.map(str::to_owned),
    }
}

async fn listed(source: &dyn TaskSource, task: &str) -> Vec<Comment> {
    source
        .task_comments(&id(task), &page(50))
        .await
        .unwrap()
        .expect("the task exists")
        .items
}

#[tokio::test]
async fn comments_are_listed_added_edited_and_deleted_on_the_task_they_belong_to() {
    let source = commented();
    let seeded = listed(source.as_ref(), "T-1").await;
    assert_eq!(seeded.len(), 1);
    assert_eq!(seeded[0].id.as_str(), "C-1");
    assert!(listed(source.as_ref(), "T-2").await.is_empty());

    let added = source
        .add_comment(&id("T-1"), &new("second\n", Some("grace")))
        .await
        .unwrap()
        .expect("the task exists");
    assert_eq!(added.id.as_str(), "C-2", "the smallest id nothing holds");
    assert_eq!(added.body, "second\n");
    assert_eq!(added.author.as_deref(), Some("grace"));
    assert!(added.created_at.is_some());
    assert_eq!(added.created_at, added.updated_at);

    let edited = source
        .edit_comment(
            &id("T-1"),
            &id("C-1"),
            &CommentBody::new("seeded, corrected").unwrap(),
        )
        .await
        .unwrap()
        .expect("the comment exists");
    assert_eq!(edited.body, "seeded, corrected");
    assert_eq!(edited.created_at, seeded[0].created_at);
    assert_eq!(edited.author, seeded[0].author);
    assert!(edited.updated_at > seeded[0].updated_at);

    assert_eq!(
        listed(source.as_ref(), "T-1").await,
        vec![edited.clone(), added.clone()]
    );
    assert_eq!(
        source.delete_comment(&id("T-1"), &added.id).await.unwrap(),
        Some(added.id)
    );
    assert_eq!(listed(source.as_ref(), "T-1").await, vec![edited]);
}

#[tokio::test]
async fn a_task_or_a_comment_that_is_not_there_answers_none() {
    let source = commented();
    let missing = id("T-404");
    assert_eq!(
        source.task_comments(&missing, &page(10)).await.unwrap(),
        None
    );
    assert_eq!(
        source.add_comment(&missing, &new("x", None)).await.unwrap(),
        None
    );
    let body = CommentBody::new("x").unwrap();
    assert_eq!(
        source
            .edit_comment(&missing, &id("C-1"), &body)
            .await
            .unwrap(),
        None
    );
    // A comment on another task is a comment this task does not have.
    assert_eq!(
        source
            .edit_comment(&id("T-2"), &id("C-1"), &body)
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        source.delete_comment(&id("T-2"), &id("C-1")).await.unwrap(),
        None
    );
    assert_eq!(listed(source.as_ref(), "T-1").await.len(), 1);
}

#[tokio::test]
async fn a_source_declaring_no_comments_refuses_every_comment_call_in_the_contracts_words() {
    let source = build(&json!({"tasks": [task("T-1")]})).expect("a source without comments");
    let body = CommentBody::new("x").unwrap();
    let refusals = [
        source
            .task_comments(&id("T-1"), &page(10))
            .await
            .map(|_| ()),
        source
            .add_comment(&id("T-1"), &new("x", None))
            .await
            .map(|_| ()),
        source
            .edit_comment(&id("T-1"), &id("C-1"), &body)
            .await
            .map(|_| ()),
        source
            .delete_comment(&id("T-1"), &id("C-1"))
            .await
            .map(|_| ()),
    ];
    for refusal in refusals {
        assert_eq!(refusal, Err(commentless("in-memory")));
    }
}

#[tokio::test]
async fn comments_a_source_cannot_write_are_read_and_every_write_is_refused() {
    let source = build(&configured(
        json!({"comments": "native", "writes": "unsupported"}),
    ))
    .expect("a read-only source with comments");
    assert_eq!(listed(source.as_ref(), "T-1").await.len(), 1);
    let body = CommentBody::new("x").unwrap();
    assert_eq!(
        source.add_comment(&id("T-1"), &new("x", None)).await,
        Err(unwritable("in-memory"))
    );
    assert_eq!(
        source.edit_comment(&id("T-1"), &id("C-1"), &body).await,
        Err(unwritable("in-memory"))
    );
    assert_eq!(
        source.delete_comment(&id("T-1"), &id("C-1")).await,
        Err(unwritable("in-memory"))
    );
    assert_eq!(listed(source.as_ref(), "T-1").await[0].body, "seeded\n");
}

#[tokio::test]
async fn comments_page_in_the_order_they_were_written() {
    let source = commented();
    for text in ["two", "three"] {
        source
            .add_comment(&id("T-1"), &new(text, None))
            .await
            .unwrap();
    }
    let first = source
        .task_comments(&id("T-1"), &page(2))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        first
            .items
            .iter()
            .map(|c| c.id.as_str())
            .collect::<Vec<_>>(),
        ["C-1", "C-2"]
    );
    let second = source
        .task_comments(
            &id("T-1"),
            &PageRequest {
                cursor: first.next,
                limit: 2,
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        second
            .items
            .iter()
            .map(|c| c.body.as_str())
            .collect::<Vec<_>>(),
        ["three"]
    );
    assert_eq!(second.next, None);
}

#[tokio::test]
async fn a_deleted_task_takes_its_comments_so_a_task_recreated_under_its_id_has_none() {
    let source = commented();
    let recreated = source.get_task(&id("T-1")).await.unwrap().unwrap();
    source.delete_task(&id("T-1")).await.unwrap();
    assert_eq!(
        source.task_comments(&id("T-1"), &page(10)).await.unwrap(),
        None
    );

    source
        .write_task(&ItemWrite {
            target: None,
            item: recreated,
            depends_on: Vec::new(),
        })
        .await
        .expect("the task is written again");
    assert!(listed(source.as_ref(), "T-1").await.is_empty());
}

#[test]
fn a_configuration_whose_comments_cannot_be_reached_is_refused_naming_each_problem() {
    let mut orphaned = configured(json!({"comments": "native"}));
    orphaned["comments"][0]["task"] = json!("T-404");
    let Err(SourceError::Config { message }) = build(&orphaned) else {
        panic!("a comment on a task nothing holds is refused");
    };
    assert!(
        message.contains("comment C-1 is on task T-404"),
        "{message}"
    );

    let mut doubled = configured(json!({"comments": "native"}));
    let again = doubled["comments"][0].clone();
    doubled["comments"].as_array_mut().unwrap().push(again);
    let Err(SourceError::Config { message }) = build(&doubled) else {
        panic!("two comments under one id are refused");
    };
    assert!(message.contains("share the id C-1"), "{message}");

    let Err(SourceError::Config { message }) = build(&configured(json!({}))) else {
        panic!("comments under a source declaring none are refused");
    };
    assert!(
        message.contains("`capabilities.comments: native`"),
        "{message}"
    );
}
