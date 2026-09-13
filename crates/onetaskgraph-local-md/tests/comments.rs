//! A task file's comments section, driven through the plugin's own methods against real files.
//!
//! The file is the contract here — a person reads and edits it by hand — so every rule
//! `COMMENTS_HEADING` documents is asserted on the bytes the plugin leaves on disk as well as
//! on what it answers.

use std::fs;
use std::path::Path;

use chrono::{DateTime, Utc};
use onetaskgraph_local_md::{COMMENT_CLOSE, COMMENTS_HEADING};
use onetaskgraph_plugin_api::{
    Comment, CommentBody, Cursor, ItemWrite, NativeId, NewComment, PageRequest, SecretResolver,
    SourceError, SourceName, SourcePlugin, Task, TaskQuery, TaskSource, TextFields, TextQuery,
};
use secrecy::SecretString;

struct NoSecrets;
impl SecretResolver for NoSecrets {
    fn get(&self, _: &str) -> Option<SecretString> {
        None
    }
}

/// The task C2 spells, byte for byte, as a copy into this folder writes one.
const TASK: &str = "---\ntitle: Ship the release\nstatus: todo\n---\nLong-form task content.\n";

/// A folder holding `files` under its root, and the source built over it.
fn folder(files: &[(&str, &str)]) -> (tempfile::TempDir, Box<dyn TaskSource>) {
    let root = tempfile::tempdir().expect("a temporary folder");
    for (relative, text) in files {
        let path = root.path().join(relative);
        fs::create_dir_all(path.parent().expect("a file under the root")).expect("its folder");
        fs::write(path, text).expect("the file");
    }
    let source = onetaskgraph_local_md::Plugin
        .build(
            &SourceName::new("notes").unwrap(),
            &serde_json::json!({"root": root.path()}),
            &NoSecrets,
        )
        .expect("source builds");
    (root, source)
}

fn read(root: &Path, relative: &str) -> String {
    fs::read_to_string(root.join(relative)).expect("the file reads")
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

fn body(text: &str) -> CommentBody {
    CommentBody::new(text).expect("a non-empty body")
}

/// A time as the section writes one down.
fn stamped(at: DateTime<Utc>) -> String {
    at.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// One block as a person would write it by hand, with no author.
fn block(comment: &str, at: &str, text: &str) -> String {
    format!(
        "<!-- onetaskgraph:comment id=\"{comment}\" created_at=\"{at}\" updated_at=\"{at}\" -->\n\
         ### comment — {at}\n\n{text}\n\n{COMMENT_CLOSE}\n"
    )
}

async fn add(source: &dyn TaskSource, task: &str, text: &str, author: Option<&str>) -> Comment {
    source
        .add_comment(
            &id(task),
            &NewComment {
                body: body(text),
                author: author.map(str::to_owned),
            },
        )
        .await
        .expect("the comment is added")
        .expect("the task exists")
}

async fn listed(source: &dyn TaskSource, task: &str) -> Vec<Comment> {
    source
        .task_comments(&id(task), &page(200))
        .await
        .expect("the comments read")
        .expect("the task exists")
        .items
}

#[tokio::test]
async fn an_added_comment_lands_in_exactly_the_section_shape_and_reads_back_byte_for_byte() {
    let (root, source) = folder(&[("tasks/release.md", TASK)]);
    let text = "The body, byte-for-byte: any Markdown, including its own\n\n## heading\n";

    let added = add(source.as_ref(), "release", text, Some("ada")).await;

    let at = added.created_at.expect("the time it was written");
    assert_eq!(added.updated_at, Some(at));
    assert_eq!(added.author.as_deref(), Some("ada"));
    assert_eq!(added.body, text);
    assert_eq!(
        added.id.as_str(),
        format!("{}-1", at.format("%Y%m%dT%H%M%SZ"))
    );
    let written = stamped(at);
    assert_eq!(
        read(root.path(), "tasks/release.md"),
        format!(
            "{TASK}\n{COMMENTS_HEADING}\n\n<!-- onetaskgraph:comment id=\"{}\" author=\"ada\" \
             created_at=\"{written}\" updated_at=\"{written}\" -->\n### ada — {written}\n\n{text}\n\n\
             {COMMENT_CLOSE}\n",
            added.id
        )
    );
    assert_eq!(listed(source.as_ref(), "release").await, vec![added]);
    let task = source.get_task(&id("release")).await.unwrap().unwrap();
    assert_eq!(task.content.as_deref(), Some("Long-form task content."));
}

#[tokio::test]
async fn an_edit_moves_only_that_body_and_its_time_and_deleting_the_last_removes_the_heading() {
    let (root, source) = folder(&[("tasks/release.md", TASK)]);
    let first = add(source.as_ref(), "release", "first\n", Some("ada")).await;
    let second = add(source.as_ref(), "release", "second", None).await;
    assert!(
        read(root.path(), "tasks/release.md").contains(&format!(
            "### comment — {}\n\nsecond\n\n",
            stamped(second.created_at.unwrap())
        )),
        "a comment without an author is headed `comment`"
    );

    // The section writes a time to the second, so the edit waits for the clock to pass the
    // second the comment was written in: an edit inside that same second would leave the time
    // where it was and prove nothing about it moving.
    let written = first.updated_at.expect("the time it was written");
    while Utc::now().timestamp() <= written.timestamp() {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let edited = source
        .edit_comment(&id("release"), &first.id, &body("first, corrected\n"))
        .await
        .unwrap()
        .expect("the comment exists");
    assert_eq!(edited.id, first.id);
    assert_eq!(edited.author, first.author);
    assert_eq!(edited.created_at, first.created_at);
    assert_eq!(edited.body, "first, corrected\n");
    assert!(
        edited.updated_at > first.updated_at,
        "an edit moves the time: {:?} after {:?}",
        edited.updated_at,
        first.updated_at
    );
    assert_eq!(
        listed(source.as_ref(), "release").await,
        vec![edited.clone(), second.clone()]
    );

    assert_eq!(
        source
            .delete_comment(&id("release"), &first.id)
            .await
            .unwrap(),
        Some(first.id.clone())
    );
    assert_eq!(
        listed(source.as_ref(), "release").await,
        vec![second.clone()]
    );
    assert_eq!(
        source
            .delete_comment(&id("release"), &second.id)
            .await
            .unwrap(),
        Some(second.id.clone())
    );
    assert_eq!(read(root.path(), "tasks/release.md"), TASK);
    assert!(listed(source.as_ref(), "release").await.is_empty());
}

#[tokio::test]
async fn a_body_holding_the_closing_line_is_refused_on_add_and_on_edit() {
    let (root, source) = folder(&[("tasks/release.md", TASK)]);
    let closing = format!("before\n{COMMENT_CLOSE}\nafter\n");

    let refused = source
        .add_comment(
            &id("release"),
            &NewComment {
                body: body(&closing),
                author: None,
            },
        )
        .await
        .expect_err("a body that would close its own block is refused");
    let SourceError::Refused { message } = refused else {
        panic!("refused, not {refused:?}");
    };
    assert!(message.contains(COMMENT_CLOSE), "{message}");
    assert_eq!(read(root.path(), "tasks/release.md"), TASK);

    // The marker inside a longer line closes nothing, so it is kept like any other text.
    let inline = format!("quoting {COMMENT_CLOSE} inline\n");
    let kept = add(source.as_ref(), "release", &inline, None).await;
    assert_eq!(listed(source.as_ref(), "release").await[0].body, inline);

    let before = read(root.path(), "tasks/release.md");
    let refused = source
        .edit_comment(&id("release"), &kept.id, &body(&closing))
        .await
        .expect_err("an edit to such a body is refused too");
    assert!(
        matches!(refused, SourceError::Refused { .. }),
        "{refused:?}"
    );
    assert_eq!(read(root.path(), "tasks/release.md"), before);
}

#[tokio::test]
async fn an_id_takes_the_smallest_free_number_for_its_second_and_is_never_renumbered() {
    // The minted id depends on the second the comment is written in, so the file is seeded for
    // the second the test starts in and the attempt repeated when the clock ticks between.
    for _ in 0..5 {
        let now = Utc::now();
        let stamp = now.format("%Y%m%dT%H%M%SZ").to_string();
        let at = stamped(now);
        let seeded = format!(
            "{TASK}\n{COMMENTS_HEADING}\n\n{}\n{}",
            block(&format!("{stamp}-1"), &at, "one"),
            block(&format!("{stamp}-3"), &at, "three")
        );
        let (_root, source) = folder(&[("tasks/release.md", &seeded)]);
        let added = add(source.as_ref(), "release", "two", None).await;
        if added
            .created_at
            .map(|at| at.format("%Y%m%dT%H%M%SZ").to_string())
            != Some(stamp.clone())
        {
            continue;
        }
        assert_eq!(added.id.as_str(), format!("{stamp}-2"));

        source
            .delete_comment(&id("release"), &id(&format!("{stamp}-1")))
            .await
            .unwrap()
            .expect("the seeded comment exists");
        let ids: Vec<String> = listed(source.as_ref(), "release")
            .await
            .into_iter()
            .map(|comment| comment.id.0)
            .collect();
        assert_eq!(ids, [format!("{stamp}-3"), format!("{stamp}-2")]);
        return;
    }
    panic!("the clock ticked over a second on every one of five attempts");
}

#[tokio::test]
async fn only_the_trailing_run_of_comment_blocks_is_the_section() {
    let at = "2026-09-13T15:11:07Z";
    let prose = format!(
        "---\ntitle: T\nstatus: todo\n---\nIntro.\n\n{COMMENTS_HEADING}\n\nThis heading is prose.\n"
    );
    let trailing = format!(
        "---\ntitle: T\nstatus: todo\n---\nIntro.\n\n{COMMENTS_HEADING}\n\nProse first.\n\n{COMMENTS_HEADING}\n\n{}",
        block(
            "c-1",
            at,
            &format!(
                "A body with its own\n{COMMENTS_HEADING}\n\n<!-- onetaskgraph:comment id=\"fake\" -->\nline"
            )
        )
    );
    let followed = format!(
        "---\ntitle: T\nstatus: todo\n---\nIntro.\n\n{COMMENTS_HEADING}\n\n{}\nProse after the block.\n",
        block("c-1", at, "body")
    );
    let unknown = format!(
        "---\ntitle: T\nstatus: todo\n---\nIntro.\n\n{COMMENTS_HEADING}\n\n\
         <!-- onetaskgraph:comment id=\"c-1\" colour=\"red\" created_at=\"{at}\" updated_at=\"{at}\" -->\n\
         ### comment — {at}\n\nbody\n\n{COMMENT_CLOSE}\n"
    );
    let (_root, source) = folder(&[
        ("tasks/prose.md", &prose),
        ("tasks/trailing.md", &trailing),
        ("tasks/followed.md", &followed),
        ("tasks/unknown.md", &unknown),
    ]);

    for plain in ["prose", "followed", "unknown"] {
        assert!(
            listed(source.as_ref(), plain).await.is_empty(),
            "{plain}: a heading not followed by nothing but blocks is content"
        );
        let task = source.get_task(&id(plain)).await.unwrap().unwrap();
        assert!(
            task.content.unwrap().contains(COMMENTS_HEADING),
            "{plain}: the heading stays in the content"
        );
    }

    let comments = listed(source.as_ref(), "trailing").await;
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].id.as_str(), "c-1");
    assert!(
        comments[0]
            .body
            .contains("<!-- onetaskgraph:comment id=\"fake\" -->")
    );
    let task = source.get_task(&id("trailing")).await.unwrap().unwrap();
    assert_eq!(
        task.content.as_deref(),
        Some(format!("Intro.\n\n{COMMENTS_HEADING}\n\nProse first.").as_str())
    );
}

#[tokio::test]
async fn the_section_is_not_content_so_a_content_search_never_matches_a_comment() {
    let (_root, source) = folder(&[
        ("tasks/release.md", TASK),
        (
            "tasks/other.md",
            "---\ntitle: Other\nstatus: todo\n---\nA needle in the content.\n",
        ),
    ]);
    add(source.as_ref(), "release", "a needle in a comment\n", None).await;

    for fields in [TextFields::Content, TextFields::TitleOrContent] {
        let found: Vec<String> = source
            .query_tasks(
                &TaskQuery {
                    text: Some(TextQuery {
                        terms: "needle".to_owned(),
                        fields,
                    }),
                    ..TaskQuery::default()
                },
                &page(50),
            )
            .await
            .unwrap()
            .items
            .into_iter()
            .map(|task| task.id.0)
            .collect();
        assert_eq!(found, ["other"], "{fields:?}");
    }
}

#[tokio::test]
async fn a_copy_written_over_the_task_keeps_its_section_byte_for_byte_and_adds_none() {
    let (root, source) = folder(&[("tasks/release.md", TASK)]);
    add(source.as_ref(), "release", "evidence\n", Some("ada")).await;
    let before = read(root.path(), "tasks/release.md");
    let section = &before[before.find(COMMENTS_HEADING).unwrap()..];

    let task = source.get_task(&id("release")).await.unwrap().unwrap();
    let rewritten = Task {
        content: Some("Rewritten by a copy.".to_owned()),
        ..task.clone()
    };
    source
        .write_task(&ItemWrite {
            target: Some(id("release")),
            item: rewritten.clone(),
            depends_on: Vec::new(),
        })
        .await
        .expect("the update lands");
    let after = read(root.path(), "tasks/release.md");
    assert!(after.ends_with(section), "{after}");
    assert_eq!(after.matches(COMMENTS_HEADING).count(), 1, "{after}");
    let reread = source.get_task(&id("release")).await.unwrap().unwrap();
    assert_eq!(reread.content.as_deref(), Some("Rewritten by a copy."));
    assert_eq!(listed(source.as_ref(), "release").await.len(), 1);

    // A create is a new file, and a new file has no comments to keep.
    let created = source
        .write_task(&ItemWrite {
            target: None,
            item: rewritten,
            depends_on: Vec::new(),
        })
        .await
        .expect("the create lands");
    assert!(!read(root.path(), &format!("tasks/{created}.md")).contains(COMMENTS_HEADING));
    assert!(listed(source.as_ref(), created.as_str()).await.is_empty());

    // Content that would read back as a section would turn prose into comments, so it is
    // refused and the file is left exactly as it was.
    let smuggled = Task {
        content: Some(format!(
            "Innocent.\n\n{COMMENTS_HEADING}\n\n{}",
            block("forged", "2026-09-13T15:11:07Z", "nobody wrote this")
        )),
        ..task
    };
    let refused = source
        .write_task(&ItemWrite {
            target: Some(id("release")),
            item: smuggled,
            depends_on: Vec::new(),
        })
        .await
        .expect_err("content ending in a section is refused");
    let SourceError::Refused { message } = refused else {
        panic!("refused, not {refused:?}");
    };
    assert!(message.contains("`content`"), "{message}");
    assert_eq!(read(root.path(), "tasks/release.md"), after);
}

#[tokio::test]
async fn a_comment_call_naming_no_task_or_no_comment_answers_none_and_writes_nothing() {
    let (root, source) = folder(&[("tasks/release.md", TASK)]);
    let task = id("missing");
    assert_eq!(source.task_comments(&task, &page(10)).await.unwrap(), None);
    assert_eq!(
        source
            .add_comment(
                &task,
                &NewComment {
                    body: body("x"),
                    author: None,
                },
            )
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        source
            .edit_comment(&task, &id("c"), &body("x"))
            .await
            .unwrap(),
        None
    );
    assert_eq!(source.delete_comment(&task, &id("c")).await.unwrap(), None);

    assert_eq!(
        source
            .edit_comment(&id("release"), &id("nothing"), &body("x"))
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        source
            .delete_comment(&id("release"), &id("nothing"))
            .await
            .unwrap(),
        None
    );
    assert_eq!(read(root.path(), "tasks/release.md"), TASK);
    assert!(!root.path().join("tasks/missing.md").exists());
}

#[tokio::test]
async fn an_author_is_escaped_in_its_marker_and_must_fit_on_one_heading_line() {
    let (root, source) = folder(&[("tasks/release.md", TASK)]);
    let author = "Ada \"the\" <first> & only";
    let added = add(source.as_ref(), "release", "hi", Some(author)).await;
    let written = read(root.path(), "tasks/release.md");
    assert!(
        written.contains("author=\"Ada &quot;the&quot; &lt;first&gt; &amp; only\""),
        "{written}"
    );
    assert!(written.contains(&format!("### {author} — ")), "{written}");
    assert_eq!(
        listed(source.as_ref(), "release").await[0]
            .author
            .as_deref(),
        Some(author)
    );
    assert_eq!(added.author.as_deref(), Some(author));

    for unrepresentable in ["two\nlines", "   ", "tab\there"] {
        let refused = source
            .add_comment(
                &id("release"),
                &NewComment {
                    body: body("x"),
                    author: Some(unrepresentable.to_owned()),
                },
            )
            .await
            .expect_err("an author that cannot sit on one heading line is refused");
        assert!(
            matches!(refused, SourceError::Refused { .. }),
            "{refused:?}"
        );
    }
    assert_eq!(read(root.path(), "tasks/release.md"), written);
}

#[tokio::test]
async fn comments_page_in_the_order_they_were_written() {
    let (_root, source) = folder(&[("tasks/release.md", TASK)]);
    let mut written = Vec::new();
    for text in ["one", "two", "three"] {
        written.push(add(source.as_ref(), "release", text, None).await);
    }

    let first = source
        .task_comments(&id("release"), &page(2))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first.items, written[..2]);
    let cursor: Cursor = first.next.expect("a second page");
    let second = source
        .task_comments(
            &id("release"),
            &PageRequest {
                cursor: Some(cursor),
                limit: 2,
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(second.items, written[2..]);
    assert_eq!(second.next, None);
}

#[tokio::test]
async fn a_file_whose_comments_share_an_id_is_refused_naming_the_file_and_the_id() {
    let at = "2026-09-13T15:11:07Z";
    let doubled = format!(
        "{TASK}\n{COMMENTS_HEADING}\n\n{}\n{}",
        block("same", at, "one"),
        block("same", at, "two")
    );
    let (_root, source) = folder(&[("tasks/release.md", &doubled)]);
    let refused = source
        .task_comments(&id("release"), &page(10))
        .await
        .expect_err("two comments under one id cannot be addressed one at a time");
    let SourceError::Malformed { message } = refused else {
        panic!("malformed, not {refused:?}");
    };
    assert!(message.contains("release.md"), "{message}");
    assert!(message.contains("same"), "{message}");
}
