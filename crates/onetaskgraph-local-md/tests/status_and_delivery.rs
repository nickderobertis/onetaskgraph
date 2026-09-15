//! A task file's `queued` status, its status-only write, and its two task lists.
//!
//! Every test drives the real plugin over a real folder and asserts on what a read answers
//! and on the bytes the file holds afterwards: a status-only write is a promise about the
//! file, not only about what a read reports.

use std::fs;
use std::path::Path;

use onetaskgraph_plugin_api::{
    ItemWrite, NativeId, SecretResolver, SourceError, SourceName, SourcePlugin, Status,
    StatusCategory, Task, TaskRef, TaskSource,
};
use secrecy::SecretString;
use serde_json::json;

struct NoSecrets;
impl SecretResolver for NoSecrets {
    fn get(&self, _: &str) -> Option<SecretString> {
        None
    }
}

/// A folder named `work`, holding `files` under `tasks/` and `projects/`.
fn folder(
    files: &[(&str, &str)],
    config: serde_json::Value,
) -> (tempfile::TempDir, Box<dyn TaskSource>) {
    let root = tempfile::tempdir().expect("temporary notes");
    for (relative, text) in files {
        let path = root.path().join(relative);
        fs::create_dir_all(path.parent().expect("a file has a folder")).expect("folders");
        fs::write(&path, text).expect("a file");
    }
    let mut config = config;
    config["root"] = json!(root.path());
    let source = onetaskgraph_local_md::Plugin
        .build(&SourceName::new("work").unwrap(), &config, &NoSecrets)
        .expect("the folder builds");
    (root, source)
}

fn id(value: &str) -> NativeId {
    NativeId(value.to_owned())
}

fn entry(value: &str) -> TaskRef {
    TaskRef::new(value).expect("a task id")
}

fn read(root: &tempfile::TempDir, relative: &str) -> String {
    fs::read_to_string(root.path().join(relative)).expect("the file reads")
}

fn refusal(error: SourceError) -> String {
    match error {
        SourceError::Refused { message } | SourceError::Malformed { message } => message,
        other => panic!("expected a refusal naming the problem, got {other:?}"),
    }
}

/// A task file whose front matter uses every shape the rewrite has to step around.
const RICH: &str = "---\ntitle: Alpha\nstatus: todo\nlabels: [Bug]\ndepends_on:\n  - b\n  - id: other:P-1\n    item: project\nmetadata:\n  caller.count: 3\ndelivers:\n- b\ndelivered_by: [\"plan:P-9\"]\n---\n# Alpha\n\nThe body.\n\n## Comments\n\n<!-- onetaskgraph:comment id=\"20260913T151107Z-1\" created_at=\"2026-09-13T15:11:07Z\" updated_at=\"2026-09-13T15:11:07Z\" -->\n### comment — 2026-09-13T15:11:07Z\n\nKeep me.\n\n<!-- /onetaskgraph:comment -->\n";

#[tokio::test]
async fn queued_in_any_letter_case_reads_as_queued_with_no_configuration() {
    let (_root, source) = folder(
        &[
            ("tasks/a.md", "---\nstatus: queued\n---\n"),
            ("tasks/b.md", "---\nstatus: QUEUED\n---\n"),
            ("tasks/c.md", "---\nstatus: Queued\n---\n"),
        ],
        json!({}),
    );
    for (native, word) in [("a", "queued"), ("b", "QUEUED"), ("c", "Queued")] {
        let task = source.get_task(&id(native)).await.unwrap().expect("held");
        assert_eq!(
            task.status,
            Status {
                category: StatusCategory::Queued,
                name: word.to_owned()
            }
        );
    }
}

#[tokio::test]
async fn a_status_set_rewrites_the_status_entry_and_not_one_other_byte() {
    let (root, source) = folder(
        &[("tasks/a.md", RICH), ("tasks/b.md", "---\n---\n")],
        json!({}),
    );
    for (category, word) in [
        (StatusCategory::Queued, "queued"),
        // `in progress` rather than `doing`: the category's own spelling, spoken, wins.
        (StatusCategory::InProgress, "in progress"),
        (StatusCategory::Done, "done"),
    ] {
        let answered = source.set_task_status(&id("a"), category).await.unwrap();
        assert_eq!(
            answered,
            Some(Status {
                category,
                name: word.to_owned()
            })
        );
        assert_eq!(
            read(&root, "tasks/a.md"),
            RICH.replace("status: todo", &format!("status: {word}")),
            "only the status entry moves"
        );
        let task = source.get_task(&id("a")).await.unwrap().expect("held");
        assert_eq!(task.status.category, category);
        assert_eq!(task.title, "Alpha");
        assert_eq!(task.content.as_deref(), Some("# Alpha\n\nThe body."));
        assert_eq!(task.delivers, vec![entry("b")]);
        assert_eq!(task.delivered_by, vec![entry("plan:P-9")]);
        assert_eq!(task.metadata["caller.count"], json!(3));
    }
}

#[tokio::test]
async fn a_task_already_in_the_category_is_left_byte_for_byte_word_and_all() {
    let original = "---\nstatus: Doing\n---\nbody\n";
    let (root, source) = folder(&[("tasks/a.md", original)], json!({}));
    let answered = source
        .set_task_status(&id("a"), StatusCategory::InProgress)
        .await
        .unwrap();
    assert_eq!(
        answered,
        Some(Status {
            category: StatusCategory::InProgress,
            name: "Doing".to_owned()
        })
    );
    assert_eq!(read(&root, "tasks/a.md"), original);
}

#[tokio::test]
async fn a_task_with_no_status_entry_gains_one_and_a_windows_file_keeps_its_line_endings() {
    let (root, source) = folder(
        &[
            ("tasks/bare.md", "---\ntitle: Bare\n---\nbody\n"),
            (
                "tasks/crlf.md",
                "---\r\ntitle: Crlf\r\nstatus: todo\r\n---\r\nbody\r\n",
            ),
        ],
        json!({}),
    );
    source
        .set_task_status(&id("bare"), StatusCategory::Todo)
        .await
        .unwrap();
    assert_eq!(
        read(&root, "tasks/bare.md"),
        "---\ntitle: Bare\nstatus: todo\n---\nbody\n"
    );
    source
        .set_task_status(&id("crlf"), StatusCategory::Queued)
        .await
        .unwrap();
    assert_eq!(
        read(&root, "tasks/crlf.md"),
        "---\r\ntitle: Crlf\r\nstatus: queued\r\n---\r\nbody\r\n"
    );
}

#[tokio::test]
async fn a_category_this_folder_cannot_read_back_is_refused_in_the_words_a_copy_uses() {
    let original = "---\nstatus: todo\n---\n";
    let (root, source) = folder(
        &[("tasks/a.md", original)],
        json!({"status_mapping": {"todo": "todo"}}),
    );
    let refused = refusal(
        source
            .set_task_status(&id("a"), StatusCategory::Queued)
            .await
            .expect_err("no word here reads back as queued"),
    );
    let copied = refusal(
        source
            .write_task(&ItemWrite {
                target: Some(id("a")),
                item: Task {
                    status: Status {
                        category: StatusCategory::Queued,
                        name: "queued".to_owned(),
                    },
                    ..source.get_task(&id("a")).await.unwrap().expect("held")
                },
                depends_on: Vec::new(),
            })
            .await
            .expect_err("a copy of that status is refused too"),
    );
    assert_eq!(refused, copied, "one refusal, in one wording");
    assert!(
        refused.contains("this source reads \"queued\" as unknown, not queued"),
        "{refused}"
    );
    assert_eq!(read(&root, "tasks/a.md"), original, "nothing was written");
}

#[tokio::test]
async fn a_status_set_or_a_delivered_by_write_naming_no_task_answers_none() {
    let (_root, source) = folder(&[("projects/p.md", "---\nstatus: todo\n---\n")], json!({}));
    assert_eq!(
        source
            .set_task_status(&id("p"), StatusCategory::Done)
            .await
            .unwrap(),
        None,
        "a project is not a task"
    );
    assert_eq!(
        source.set_delivered_by(&id("gone"), &[]).await.unwrap(),
        None
    );
}

#[tokio::test]
async fn both_lists_are_read_as_stored_and_a_bad_entry_is_refused_naming_the_task_and_the_entry() {
    let (_root, source) = folder(
        &[
            (
                "tasks/a.md",
                "---\ndelivers: [b, \"other:X-1\"]\ndelivered_by: [\"plan:P-9\"]\n---\n",
            ),
            ("tasks/number.md", "---\ndelivers: [3]\n---\n"),
            ("tasks/itself.md", "---\ndelivers: [\"work:itself\"]\n---\n"),
            (
                "tasks/twice.md",
                "---\ndelivered_by: [b, \"work:b\"]\n---\n",
            ),
            ("tasks/blank.md", "---\ndelivers: [\"\"]\n---\n"),
            ("projects/p.md", "---\ndelivers: [a]\n---\n"),
        ],
        json!({}),
    );
    let task = source.get_task(&id("a")).await.unwrap().expect("held");
    assert_eq!(task.delivers, vec![entry("b"), entry("other:X-1")]);
    assert_eq!(task.delivered_by, vec![entry("plan:P-9")]);

    for (native, says) in [
        (
            "number",
            "delivers on task number holds 3, which is not a task id",
        ),
        (
            "itself",
            "delivers on task itself names work:itself, which is that task itself",
        ),
        (
            "twice",
            "delivered_by on task twice names work:b more than once",
        ),
        (
            "blank",
            "delivers on task blank holds \"\", which is not a task id",
        ),
    ] {
        let message = refusal(source.get_task(&id(native)).await.expect_err("refused"));
        assert!(message.contains(says), "{native}: {message}");
    }
    let message = refusal(source.get_project(&id("p")).await.expect_err("refused"));
    assert!(
        message.contains("belong to a task, and this is a project"),
        "{message}"
    );
}

#[tokio::test]
async fn a_written_task_carries_both_lists_and_refuses_one_naming_itself_or_one_task_twice() {
    let (root, source) = folder(&[], json!({}));
    let task = |delivers: Vec<TaskRef>, delivered_by: Vec<TaskRef>| Task {
        id: id("a"),
        title: "Alpha".to_owned(),
        content: None,
        status: Status {
            category: StatusCategory::Todo,
            name: "todo".to_owned(),
        },
        labels: Vec::new(),
        project: None,
        url: None,
        location: None,
        created_at: None,
        updated_at: None,
        metadata: Default::default(),
        repositories: Vec::new(),
        delivers,
        delivered_by,
    };
    let landed = source
        .write_task(&ItemWrite {
            target: None,
            item: task(vec![entry("b"), entry("gh:I_1")], vec![entry("plan:P-1")]),
            depends_on: Vec::new(),
        })
        .await
        .expect("written");
    let text = read(&root, &format!("tasks/{landed}.md"));
    assert!(text.contains("delivers:\n- b\n- gh:I_1\n"), "{text}");
    assert!(text.contains("delivered_by:\n- plan:P-1\n"), "{text}");
    let back = source.get_task(&landed).await.unwrap().expect("held");
    assert_eq!(back.delivers, vec![entry("b"), entry("gh:I_1")]);
    assert_eq!(back.delivered_by, vec![entry("plan:P-1")]);

    for (delivers, says) in [
        (vec![entry("a")], "names a, which is that task itself"),
        (
            vec![entry("b"), entry("work:b")],
            "names work:b more than once",
        ),
    ] {
        let message = refusal(
            source
                .write_task(&ItemWrite {
                    target: Some(landed.clone()),
                    item: task(delivers, Vec::new()),
                    depends_on: Vec::new(),
                })
                .await
                .expect_err("refused"),
        );
        assert!(
            message.contains("cannot represent the field `delivers`") && message.contains(says),
            "{message}"
        );
    }
}

#[tokio::test]
async fn delivered_by_is_rewritten_alone_added_when_absent_and_removed_when_emptied() {
    let (root, source) = folder(
        &[
            ("tasks/a.md", RICH),
            (
                "tasks/last.md",
                "---\ntitle: Last\ndelivered_by:\n- \"x:1\"\n---\nbody\n",
            ),
            ("tasks/none.md", "---\ntitle: None\n---\n"),
        ],
        json!({}),
    );
    source
        .set_delivered_by(&id("a"), &[entry("plan:P-9"), entry("gh:I_2")])
        .await
        .unwrap()
        .expect("held");
    assert_eq!(
        read(&root, "tasks/a.md"),
        RICH.replace(
            "delivered_by: [\"plan:P-9\"]",
            "delivered_by: [\"plan:P-9\",\"gh:I_2\"]"
        )
    );

    source
        .set_delivered_by(&id("last"), &[])
        .await
        .unwrap()
        .expect("held");
    assert_eq!(
        read(&root, "tasks/last.md"),
        "---\ntitle: Last\n---\nbody\n"
    );

    source
        .set_delivered_by(&id("none"), &[entry("plan:P-1")])
        .await
        .unwrap()
        .expect("held");
    assert_eq!(
        read(&root, "tasks/none.md"),
        "---\ntitle: None\ndelivered_by: [\"plan:P-1\"]\n---\n"
    );

    let message = refusal(
        source
            .set_delivered_by(&id("none"), &[entry("work:none")])
            .await
            .expect_err("a task is not delivered by itself"),
    );
    assert!(message.contains("which is that task itself"), "{message}");
    assert!(Path::new(&root.path().join("tasks/none.md")).exists());
}
