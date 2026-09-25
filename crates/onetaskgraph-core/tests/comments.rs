//! The comment verbs are the engine's own, driven as library calls.
//!
//! Not only through the command line, deliberately: this product is exposed three ways from
//! one engine, and the Rust caller that links the crate is the consumer a CLI-only proof would
//! strand. This is also where an in-memory source's add-then-list is proven, because its work
//! lives in the process that holds it and the journeys that drive the compiled binary can only
//! read one invocation's answer — see `crates/onetaskgraph/tests/e2e/comments.rs`.

use onetaskgraph_core::{
    CommentList, Config, ConfiguredSource, DeletedComment, Engine, EngineError, GlobalId,
    ResolvedSource,
};
use onetaskgraph_plugin_api::{
    Capabilities, Comment, CommentBody, Cursor, DependencyEdge, DependencySupport, Direction,
    Health, Label, NativeId, NewComment, Page, PageRequest, Project, ProjectQuery, SecretResolver,
    SourceError, SourceName, Status, StatusCategory, Support, Task, TaskQuery, TaskSource,
    WriteSupport,
};
use secrecy::SecretString;
use serde_json::{Value, json};

struct NoSecrets;
impl SecretResolver for NoSecrets {
    fn get(&self, _var: &str) -> Option<SecretString> {
        None
    }
}

fn name(value: &str) -> SourceName {
    SourceName::new(value).expect("a valid source name")
}

fn id(qualified: &str) -> GlobalId {
    qualified.parse().expect("a qualified id")
}

fn body(text: &str) -> CommentBody {
    CommentBody::new(text).expect("a non-empty body")
}

fn task_value(native: &str) -> Value {
    json!({"id": native, "title": native, "content": null,
           "status": {"category": "todo", "name": "Todo"}, "labels": []})
}

/// Five comments on `T-1`, so a source serving two rows at a time is walked over three pages.
fn seeded() -> Value {
    Value::Array(
        (1..=5)
            .map(|n| {
                json!({"task": "T-1", "comment": {
                    "id": format!("C-{n}"), "author": null,
                    "created_at": format!("2026-09-13T15:1{n}:00Z"),
                    "updated_at": format!("2026-09-13T15:1{n}:00Z"),
                    "body": format!("comment {n}\n"), "url": null}})
            })
            .collect(),
    )
}

/// Four sources: one whose comments can be written, one whose comments can only be read, one
/// whose tasks have none, and one configured incoherently enough that it never builds.
fn engine() -> Engine {
    let sources = json!({
        "memory": {"plugin": "in-memory", "config": {
            "capabilities": {"comments": "native", "max_page_size": 2},
            "tasks": [task_value("T-1"), task_value("T-2")],
            "comments": seeded(),
        }},
        "readonly": {"plugin": "in-memory", "config": {
            "capabilities": {"comments": "native", "writes": "unsupported"},
            "tasks": [task_value("T-1")],
            "comments": seeded(),
        }},
        "bare": {"plugin": "in-memory", "config": {"tasks": [task_value("T-1")]}},
        "broken": {"plugin": "in-memory", "config": {
            "tasks": [task_value("T-1"), task_value("T-1")],
        }},
    });
    let config = Config::from_document(json!({"sources": sources})).expect("a valid configuration");
    Engine::build(&config, &NoSecrets)
}

fn bodies(list: &CommentList) -> Vec<&str> {
    list.comments
        .iter()
        .map(|comment| comment.body.as_str())
        .collect()
}

#[tokio::test]
async fn a_comment_added_is_listed_edited_and_deleted_by_the_same_engine() {
    let engine = engine();
    let task = id("memory:T-1");

    let added = engine
        .add_comment(
            &task,
            &NewComment {
                body: body("Added.\n\n## with a heading\n"),
                author: Some("ada".to_owned()),
            },
        )
        .await
        .expect("the comment is added");
    assert_eq!(added.id.as_str(), "C-6");
    assert_eq!(added.author.as_deref(), Some("ada"));

    // Oldest first, walked to the end of the source's two-row pages.
    let listed = engine.comments(&task).await.expect("the comments read");
    assert_eq!(
        bodies(&listed),
        [
            "comment 1\n",
            "comment 2\n",
            "comment 3\n",
            "comment 4\n",
            "comment 5\n",
            "Added.\n\n## with a heading\n"
        ]
    );
    assert_eq!(listed.comments.last(), Some(&added));

    let edited = engine
        .edit_comment(&task, &added.id, &body("Added, then edited.\n"))
        .await
        .expect("the comment is edited");
    assert_eq!(edited.id, added.id);
    assert_eq!(edited.created_at, added.created_at);
    assert_eq!(
        engine.comments(&task).await.unwrap().comments.last(),
        Some(&edited)
    );

    let deleted = engine
        .delete_comment(&task, &NativeId::from("C-3"))
        .await
        .expect("the comment is deleted");
    assert_eq!(
        deleted,
        DeletedComment {
            deleted: NativeId::from("C-3")
        }
    );
    let remaining = engine.comments(&task).await.unwrap();
    assert_eq!(remaining.comments.len(), 5);
    assert!(
        remaining
            .comments
            .iter()
            .all(|comment| comment.id.as_str() != "C-3")
    );

    let detail = engine.task_detail(&task).await.expect("the task reads");
    assert_eq!(detail.response.items.len(), 1);
    assert!(detail.response.errors.is_empty());
    assert_eq!(detail.comments, Some(remaining.comments));
}

#[tokio::test]
async fn a_task_detail_carries_comments_only_for_a_task_found_on_a_source_that_has_them() {
    let engine = engine();

    let bare = engine.task_detail(&id("bare:T-1")).await.unwrap();
    assert_eq!(bare.response.items.len(), 1);
    assert_eq!(bare.comments, None);

    let missing = engine.task_detail(&id("memory:T-404")).await.unwrap();
    assert!(missing.response.items.is_empty());
    assert_eq!(missing.comments, None);

    let none = engine.task_detail(&id("memory:T-2")).await.unwrap();
    assert_eq!(none.comments, Some(Vec::new()));

    // Serialized, the list sits beside the response rather than inside it, and is absent —
    // not empty — where the source has no comments.
    let rendered = serde_json::to_value(&none).unwrap();
    assert_eq!(rendered["comments"], json!([]));
    assert!(rendered["items"].is_array());
    assert!(
        serde_json::to_value(&bare)
            .unwrap()
            .get("comments")
            .is_none()
    );
}

#[tokio::test]
async fn every_refusal_names_what_was_not_there_and_says_what_to_do_next() {
    let engine = engine();
    let new = || NewComment {
        body: body("x"),
        author: None,
    };
    let refusals: Vec<(EngineError, &str)> = vec![
        (
            engine.comments(&id("bare:T-1")).await.unwrap_err(),
            "source bare has no comments: its plugin is in-memory",
        ),
        (
            engine
                .add_comment(&id("bare:T-1"), &new())
                .await
                .unwrap_err(),
            "source bare has no comments",
        ),
        (
            engine
                .add_comment(&id("readonly:T-1"), &new())
                .await
                .unwrap_err(),
            "source readonly cannot be written",
        ),
        (
            engine
                .edit_comment(&id("readonly:T-1"), &NativeId::from("C-1"), &body("x"))
                .await
                .unwrap_err(),
            "source readonly cannot be written",
        ),
        (
            engine
                .add_comment(&id("memory:T-404"), &new())
                .await
                .unwrap_err(),
            "no task with the id memory:T-404",
        ),
        (
            engine.comments(&id("memory:T-404")).await.unwrap_err(),
            "no task with the id memory:T-404",
        ),
        (
            engine
                .edit_comment(&id("memory:T-1"), &NativeId::from("C-999"), &body("x"))
                .await
                .unwrap_err(),
            "task memory:T-1 has no comment with the id C-999",
        ),
        (
            engine
                .delete_comment(&id("memory:T-404"), &NativeId::from("C-1"))
                .await
                .unwrap_err(),
            "no task with the id memory:T-404",
        ),
        (
            engine.comments(&id("nowhere:T-1")).await.unwrap_err(),
            "no source named \"nowhere\"",
        ),
        (
            engine.comments(&id("broken:T-1")).await.unwrap_err(),
            "source broken could not be built",
        ),
    ];
    for (refusal, says) in refusals {
        let message = refusal.to_string();
        assert!(message.contains(says), "{message}");
        assert!(message.contains("next:"), "{message}");
    }

    // A source whose comments can only be read still answers the read.
    assert_eq!(
        engine
            .comments(&id("readonly:T-1"))
            .await
            .unwrap()
            .comments
            .len(),
        5
    );
}

/// How [`Misbehaving`] answers a comment read.
#[derive(Clone, Copy)]
enum Answer {
    /// Hands back the cursor it was given, so a walk that trusted it would never end.
    RepeatedCursor,
    /// Returns more rows than the page asked for.
    Overlong,
    /// Fails outright, and fails a task read of `T-gone`.
    Failure,
    /// Holds the task for the first page and not the second.
    Vanishing,
    /// Declines to read comments on this task, as a source does for an item that cannot
    /// carry any — a GitHub draft.
    Refusal,
}

/// A source that breaks one rule of the contract on cue, which no real plugin does on demand.
struct Misbehaving(Answer);

fn a_task(native: &str) -> Task {
    Task {
        id: NativeId::from(native),
        key: None,
        title: native.to_owned(),
        content: None,
        status: Status {
            category: StatusCategory::Todo,
            name: "Todo".to_owned(),
        },
        labels: Vec::new(),
        project: None,
        url: None,
        location: None,
        created_at: None,
        updated_at: None,
        metadata: Default::default(),
        repositories: Vec::new(),
        delivers: Vec::new(),
        delivered_by: Vec::new(),
    }
}

fn a_comment(native: &str) -> Comment {
    Comment {
        id: NativeId::from(native),
        author: None,
        created_at: None,
        updated_at: None,
        body: "x".to_owned(),
        url: None,
    }
}

#[async_trait::async_trait]
impl TaskSource for Misbehaving {
    fn kind(&self) -> &'static str {
        "misbehaving"
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            projects: Support::Native,
            documents: Support::Unsupported,
            comments: Support::Native,
            orphan_tasks: Support::Native,
            filter_by_label: Support::Native,
            filter_by_status: Support::Native,
            search_title: Support::Native,
            search_content: Support::Native,
            task_dependencies: DependencySupport::BothDirections,
            project_dependencies: DependencySupport::BothDirections,
            max_page_size: 1,
        }
    }
    fn writes(&self) -> WriteSupport {
        WriteSupport::Supported
    }
    async fn health(&self) -> Result<Health, SourceError> {
        Ok(Health {
            reachable: true,
            detail: None,
        })
    }
    async fn get_task(&self, id: &NativeId) -> Result<Option<Task>, SourceError> {
        match (self.0, id.as_str()) {
            (Answer::Failure, "T-gone") => Err(SourceError::Unavailable {
                message: "the task read failed".to_owned(),
            }),
            _ => Ok(Some(a_task(id.as_str()))),
        }
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
        _page: &PageRequest,
    ) -> Result<Page<DependencyEdge>, SourceError> {
        Ok(Page::last(Vec::new()))
    }
    async fn project_dependencies(
        &self,
        _id: &NativeId,
        _direction: Direction,
        _page: &PageRequest,
    ) -> Result<Page<DependencyEdge>, SourceError> {
        Ok(Page::last(Vec::new()))
    }
    async fn task_comments(
        &self,
        _task: &NativeId,
        page: &PageRequest,
    ) -> Result<Option<Page<Comment>>, SourceError> {
        match self.0 {
            Answer::RepeatedCursor => Ok(Some(Page {
                items: Vec::new(),
                next: Some(page.cursor.clone().unwrap_or(Cursor("again".to_owned()))),
            })),
            Answer::Overlong => Ok(Some(Page::last(vec![a_comment("C-1"), a_comment("C-2")]))),
            Answer::Failure => Err(SourceError::Unavailable {
                message: "the comment read failed".to_owned(),
            }),
            Answer::Vanishing => Ok(page.cursor.is_none().then(|| Page {
                items: vec![a_comment("C-1")],
                next: Some(Cursor("1".to_owned())),
            })),
            Answer::Refusal => Err(SourceError::Refused {
                message: "this item is a draft, which has no comments".to_owned(),
            }),
        }
    }
    async fn add_comment(
        &self,
        _task: &NativeId,
        _comment: &NewComment,
    ) -> Result<Option<Comment>, SourceError> {
        Err(SourceError::Refused {
            message: "this source refuses every comment it is given".to_owned(),
        })
    }
    async fn edit_comment(
        &self,
        _task: &NativeId,
        _comment: &NativeId,
        _body: &CommentBody,
    ) -> Result<Option<Comment>, SourceError> {
        Ok(None)
    }
    async fn delete_comment(
        &self,
        _task: &NativeId,
        _comment: &NativeId,
    ) -> Result<Option<NativeId>, SourceError> {
        Err(SourceError::Unavailable {
            message: "the delete failed".to_owned(),
        })
    }
}

fn misbehaving(answer: Answer) -> Engine {
    Engine::new(
        vec![ConfiguredSource::Ready(ResolvedSource::adopt(
            name("odd"),
            Box::new(Misbehaving(answer)),
        ))],
        vec![name("odd")],
    )
}

#[tokio::test]
async fn a_walk_refuses_a_repeated_cursor_and_an_overlong_page_rather_than_trusting_them() {
    let repeated = misbehaving(Answer::RepeatedCursor)
        .comments(&id("odd:T-1"))
        .await
        .unwrap_err()
        .to_string();
    assert!(
        repeated.contains("returned the cursor it was given"),
        "{repeated}"
    );

    let overlong = misbehaving(Answer::Overlong)
        .comments(&id("odd:T-1"))
        .await
        .unwrap_err()
        .to_string();
    assert!(overlong.contains("never more"), "{overlong}");

    // A task that is gone part way through a walk is gone.
    let vanished = misbehaving(Answer::Vanishing)
        .comments(&id("odd:T-1"))
        .await
        .unwrap_err();
    assert!(
        matches!(vanished, EngineError::NoSuchTask { .. }),
        "{vanished}"
    );
}

#[tokio::test]
async fn a_source_that_fails_a_comment_call_is_named_and_a_task_detail_keeps_the_task() {
    let engine = misbehaving(Answer::Failure);
    let task = id("odd:T-1");

    let failed = engine.comments(&task).await.unwrap_err();
    assert!(
        matches!(&failed, EngineError::SourceFailed { name, .. } if name == "odd"),
        "{failed}"
    );
    let added = engine
        .add_comment(
            &task,
            &NewComment {
                body: body("x"),
                author: None,
            },
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(added.contains("refuses every comment"), "{added}");
    let deleted = engine
        .delete_comment(&task, &NativeId::from("C-1"))
        .await
        .unwrap_err()
        .to_string();
    assert!(deleted.contains("the delete failed"), "{deleted}");

    // An edit the source answered with nothing, on a task the source then cannot read, reports
    // the read that failed rather than guessing which of the two was missing.
    let unknowable = engine
        .edit_comment(&id("odd:T-gone"), &NativeId::from("C-1"), &body("x"))
        .await
        .unwrap_err()
        .to_string();
    assert!(unknowable.contains("the task read failed"), "{unknowable}");

    let detail = engine.task_detail(&task).await.unwrap();
    assert_eq!(detail.response.items.len(), 1);
    assert_eq!(detail.comments, None);
    assert_eq!(detail.response.errors.len(), 1);
    assert!(
        detail.response.errors[0]
            .error
            .to_string()
            .contains("the comment read failed")
    );
}

#[tokio::test]
async fn a_task_detail_reports_a_comment_read_its_source_refuses_as_that_sources_failure() {
    let engine = misbehaving(Answer::Refusal);
    let task = id("odd:T-1");

    // Asked for directly, the refusal is the answer, named as the source's own.
    let refused = engine.comments(&task).await.unwrap_err().to_string();
    assert!(refused.contains("which has no comments"), "{refused}");

    // Asked for beside the task, the task is still reported and the refusal is a failure of
    // that source rather than an absence nobody explains: an omitted list would read exactly
    // like a source without comments.
    let detail = engine.task_detail(&task).await.unwrap();
    assert_eq!(detail.response.items.len(), 1);
    assert_eq!(detail.comments, None);
    assert_eq!(detail.response.errors.len(), 1);
    assert!(
        detail.response.errors[0]
            .error
            .to_string()
            .contains("which has no comments")
    );
}
