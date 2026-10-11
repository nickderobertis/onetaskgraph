//! A copied comment's origin, and the hidden marker that carries it where a source cannot
//! store it natively — driven through the public surface a plugin author meets: the marker's
//! own two methods, serde, and the trait's default `write_comment_copy` over a source that
//! keeps exactly what it is handed.

use std::sync::Mutex;

use chrono::{TimeZone as _, Utc};
use onetaskgraph_plugin_api::{
    Capabilities, Comment, CommentBody, CommentCopy, CommentOrigin, DependencyEdge,
    DependencySupport, Direction, Health, Label, NativeId, NewComment, Page, PageRequest, Project,
    ProjectQuery, SourceError, Support, Task, TaskQuery, TaskSource,
};

fn origin() -> CommentOrigin {
    CommentOrigin {
        task: "board:I_kw1".to_owned(),
        id: NativeId("IC_7".to_owned()),
        author: Some("ada `the` <reviewer>".to_owned()),
        created_at: Some(Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 1).unwrap()),
        updated_at: Some(Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 2).unwrap()),
        url: Some("https://example.invalid/I_kw1#IC_7".to_owned()),
    }
}

fn held(body: String) -> Comment {
    Comment {
        id: NativeId("C-1".to_owned()),
        author: Some("fixture-owner".to_owned()),
        created_at: None,
        updated_at: None,
        body,
        url: None,
        origin: None,
    }
}

#[test]
fn a_marked_body_reads_back_as_the_body_it_was_and_the_origin_it_carried() {
    for body in [
        "Seen again on main.\n",
        "no trailing newline",
        "```\n<!-- not a marker -->\n```\n",
        "two\n\nparagraphs\n\n",
    ] {
        let marked = origin().mark(body);
        assert_ne!(marked, body, "the marker is appended");
        assert!(
            !marked.contains("<reviewer>") && !marked.contains("`the`"),
            "no value can close the marker early: {marked}"
        );
        let read = held(marked).unmarked();
        assert_eq!(read.body, body, "the body comes back byte for byte");
        assert_eq!(read.origin, Some(origin()));
    }
}

#[test]
fn a_body_with_no_marker_or_one_this_product_did_not_write_is_left_as_text() {
    let plain = held("Just a comment.".to_owned()).unmarked();
    assert_eq!(plain.body, "Just a comment.");
    assert_eq!(plain.origin, None);

    let forged = "Quoting: <!-- onetaskgraph.comment_origin `{\"task\":1}` -->";
    let read = held(forged.to_owned()).unmarked();
    assert_eq!(read.body, forged, "a marker that does not parse stays text");
    assert_eq!(read.origin, None);

    for task in ["", "bare", ":T-1", "notes:", "not a source:T-1"] {
        let unqualified = CommentOrigin {
            task: task.to_owned(),
            ..origin()
        }
        .mark("first");
        let read = held(unqualified.clone()).unmarked();
        assert_eq!(
            read.body, unqualified,
            "{task:?}: a marker naming no qualified task stays text"
        );
        assert_eq!(read.origin, None);
    }

    let mid = format!("{}\n\nand then more prose", origin().mark("first"));
    let read = held(mid.clone()).unmarked();
    assert_eq!(read.body, mid, "only a marker that ends the body is one");
}

#[test]
fn a_comment_that_already_names_its_origin_is_not_read_again() {
    let mut comment = held(origin().mark("body"));
    let other = CommentOrigin {
        id: NativeId("other".to_owned()),
        ..origin()
    };
    comment.origin = Some(other.clone());
    let read = comment.clone().unmarked();
    assert_eq!(read, comment);
}

#[test]
fn a_native_origin_naming_no_qualified_task_is_dropped_rather_than_trusted() {
    for task in ["", "I_kw1", "board:", ":I_kw1", "not a source:I_kw1"] {
        let mut comment = held("said here".to_owned());
        comment.origin = Some(CommentOrigin {
            task: task.to_owned(),
            ..origin()
        });
        let read = comment.unmarked();
        assert_eq!(read.origin, None, "{task:?}: no origin is taken on trust");
        assert_eq!(read.body, "said here", "{task:?}");
    }
}

#[test]
fn origin_is_left_off_the_wire_when_absent_and_round_trips_when_present() {
    let plain = serde_json::to_value(held("x".to_owned())).unwrap();
    assert!(plain.get("origin").is_none(), "{plain}");
    let back: Comment = serde_json::from_value(plain).unwrap();
    assert_eq!(back.origin, None);

    let mut copied = held("x".to_owned());
    copied.origin = Some(origin());
    let wire = serde_json::to_value(&copied).unwrap();
    assert_eq!(wire["origin"]["task"], "board:I_kw1");
    assert_eq!(wire["origin"]["id"], "IC_7");
    assert_eq!(wire["origin"]["created_at"], "2026-09-01T00:00:01Z");
    let back: Comment = serde_json::from_value(wire).unwrap();
    assert_eq!(back, copied);
}

#[test]
fn a_task_version_is_left_off_the_wire_when_absent() {
    let task: Task = serde_json::from_value(serde_json::json!({
        "id": "T-1", "title": "t", "content": null, "status": {"category": "todo", "name": "todo"},
        "labels": [], "project": null, "url": null, "created_at": null, "updated_at": null
    }))
    .unwrap();
    assert_eq!(
        task.version, None,
        "a plugin that sends none is read as giving none"
    );
    assert!(
        serde_json::to_value(&task)
            .unwrap()
            .get("version")
            .is_none()
    );
    let versioned = Task {
        version: Some("gh1:x".to_owned()),
        ..task
    };
    assert_eq!(
        serde_json::to_value(&versioned).unwrap()["version"],
        "gh1:x"
    );
}

/// A source whose comments are exactly what it was last handed: no marker of its own, so the
/// trait's default is the only thing standing between a copy and a body carrying it.
#[derive(Default)]
struct Keeping {
    comments: Mutex<Vec<Comment>>,
}

#[async_trait::async_trait]
impl TaskSource for Keeping {
    fn kind(&self) -> &'static str {
        "keeping"
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            projects: Support::Unsupported,
            documents: Support::Unsupported,
            comments: Support::Native,
            assets: Support::Unsupported,
            priority: Support::Unsupported,
            filter_by_priority: Support::Unsupported,
            filter_by_comment_activity: Support::Unsupported,
            filter_by_metadata: Support::Unsupported,
            filter_by_origin: Support::Unsupported,
            orphan_tasks: Support::Native,
            filter_by_label: Support::Unsupported,
            filter_by_status: Support::Unsupported,
            search_title: Support::Unsupported,
            search_content: Support::Unsupported,
            task_dependencies: DependencySupport::ForwardOnly,
            project_dependencies: DependencySupport::ForwardOnly,
            max_page_size: 25,
        }
    }
    async fn health(&self) -> Result<Health, SourceError> {
        Ok(Health {
            reachable: true,
            detail: None,
        })
    }
    async fn get_task(&self, _id: &NativeId) -> Result<Option<Task>, SourceError> {
        Ok(None)
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
    async fn add_comment(
        &self,
        _task: &NativeId,
        comment: &NewComment,
    ) -> Result<Option<Comment>, SourceError> {
        let mut held = self.comments.lock().unwrap();
        let added = Comment {
            id: NativeId(format!("C-{}", held.len() + 1)),
            author: Some("signed-in".to_owned()),
            created_at: None,
            updated_at: None,
            body: comment.body.as_str().to_owned(),
            url: None,
            origin: None,
        };
        held.push(added.clone());
        Ok(Some(added))
    }
    async fn edit_comment(
        &self,
        _task: &NativeId,
        comment: &NativeId,
        body: &CommentBody,
    ) -> Result<Option<Comment>, SourceError> {
        let mut held = self.comments.lock().unwrap();
        let Some(found) = held.iter_mut().find(|held| held.id == *comment) else {
            return Ok(None);
        };
        found.body = body.as_str().to_owned();
        Ok(Some(found.clone()))
    }
}

#[tokio::test]
async fn the_default_comment_copy_writes_the_marker_and_answers_without_it() {
    let source = Keeping::default();
    let task = NativeId("T-1".to_owned());
    let copy = CommentCopy {
        body: CommentBody::new("Copied text.\n").unwrap(),
        origin: origin(),
    };
    let written = source
        .write_comment_copy(&task, None, &copy)
        .await
        .unwrap()
        .expect("the task is there");
    assert_eq!(written.body, "Copied text.\n");
    assert_eq!(written.origin, Some(origin()));
    let stored = source.comments.lock().unwrap()[0].body.clone();
    assert_eq!(
        stored,
        origin().mark("Copied text.\n"),
        "the source holds the marker"
    );

    let edited = CommentCopy {
        body: CommentBody::new("Edited at the origin.").unwrap(),
        origin: CommentOrigin {
            updated_at: Some(Utc.with_ymd_and_hms(2026, 9, 2, 0, 0, 0).unwrap()),
            ..origin()
        },
    };
    let written = source
        .write_comment_copy(&task, Some(&written.id), &edited)
        .await
        .unwrap()
        .expect("the comment is there");
    assert_eq!(written.body, "Edited at the origin.");
    assert_eq!(written.origin, Some(edited.origin.clone()));
    assert_eq!(
        source.comments.lock().unwrap().len(),
        1,
        "an edit adds nothing"
    );

    let missing = source
        .write_comment_copy(&task, Some(&NativeId("gone".to_owned())), &edited)
        .await
        .unwrap();
    assert_eq!(missing, None);
}

#[tokio::test]
async fn the_default_comment_copy_refuses_an_origin_naming_no_task_and_writes_nothing() {
    let source = Keeping::default();
    for task in ["", "I_kw1", "board:", "not a source:I_kw1"] {
        let copy = CommentCopy {
            body: CommentBody::new("Copied text.").unwrap(),
            origin: CommentOrigin {
                task: task.to_owned(),
                ..origin()
            },
        };
        let refused = source
            .write_comment_copy(&NativeId("T-1".to_owned()), None, &copy)
            .await
            .expect_err("an origin read back as none is never written");
        assert!(
            matches!(&refused, SourceError::Malformed { message } if message.contains("qualified task id")),
            "{task:?}: {refused:?}"
        );
    }
    assert!(source.comments.lock().unwrap().is_empty());
}
