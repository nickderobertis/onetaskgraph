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
        key: None,
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

#[tokio::test]
async fn emptying_a_list_the_file_never_held_or_one_between_other_keys_touches_nothing_else() {
    let untouched = "---\ntitle: Plain\nstatus: todo\n---\nbody\n";
    let middle = "---\ntitle: Middle\ndelivered_by:\n  - \"plan:P-1\"\n  - \"plan:P-2\"\nstatus: todo\n---\nbody\n";
    let (root, source) = folder(
        &[("tasks/plain.md", untouched), ("tasks/middle.md", middle)],
        json!({}),
    );
    source
        .set_delivered_by(&id("plain"), &[])
        .await
        .unwrap()
        .expect("held");
    assert_eq!(
        read(&root, "tasks/plain.md"),
        untouched,
        "nothing to remove, nothing written"
    );
    source
        .set_delivered_by(&id("middle"), &[])
        .await
        .unwrap()
        .expect("held");
    assert_eq!(
        read(&root, "tasks/middle.md"),
        "---\ntitle: Middle\nstatus: todo\n---\nbody\n",
        "an entry between two others goes with its own continuation lines alone"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_record_the_filesystem_will_not_let_this_source_write_is_reported_rather_than_lost() {
    use std::os::unix::fs::PermissionsExt as _;
    let original = "---\ntitle: Locked\nstatus: todo\n---\n";
    let (root, source) = folder(&[("tasks/locked.md", original)], json!({}));
    // The write replaces the file through a staging file beside it and a rename, so what it
    // needs is a folder it may add a file to and rename one in.
    let tasks = root.path().join("tasks");
    fs::set_permissions(&tasks, fs::Permissions::from_mode(0o555)).expect("read-only");
    let refused = source
        .set_task_status(&id("locked"), StatusCategory::Done)
        .await;
    fs::set_permissions(&tasks, fs::Permissions::from_mode(0o755)).expect("writable again");
    // A process running as root writes through the mode bits, which is the one host where
    // this refusal cannot be provoked; everywhere else it is reported, and nothing is lost.
    match refused {
        Err(SourceError::Unavailable { message }) => {
            assert!(message.starts_with("cannot write "), "{message}");
            assert!(message.contains("locked.md"), "{message}");
            assert_eq!(read(&root, "tasks/locked.md"), original);
            assert_eq!(
                listing(root.path()),
                [
                    std::path::PathBuf::from("tasks"),
                    std::path::PathBuf::from("tasks/locked.md")
                ]
                .into(),
                "no staging file is left behind"
            );
        }
        Ok(_) if std::env::var("USER").as_deref() == Ok("root") => {}
        other => panic!("expected the write to be reported unavailable, got {other:?}"),
    }
}

/// Every path under `root`, relative to it, directories included.
fn listing(root: &Path) -> std::collections::BTreeSet<std::path::PathBuf> {
    fn visit(root: &Path, dir: &Path, out: &mut std::collections::BTreeSet<std::path::PathBuf>) {
        for entry in fs::read_dir(dir).expect("a folder lists") {
            let path = entry.expect("an entry").path();
            out.insert(path.strip_prefix(root).unwrap().to_path_buf());
            if path.is_dir() {
                visit(root, &path, out);
            }
        }
    }
    let mut out = std::collections::BTreeSet::new();
    visit(root, root, &mut out);
    out
}

/// Two sources over one folder: one sets `tasks/a.md`'s status and then its `delivered_by`
/// over and over, while the other lists and reads it on a thread of its own, and every read
/// must be the whole record — its title, its body to the last line, and a status and a list
/// those writes could have left there.
///
/// Both writes share one test rather than one each: the plugin holds every replacement apart
/// from every read in its process, so two such tests running side by side queue behind each
/// other's writes and take many times as long as the two writes interleaved here.
#[test]
fn a_reader_never_sees_a_record_part_written_by_a_status_set_or_a_delivered_by_write() {
    // A long body makes a non-atomic write's window wide enough to land in.
    let body = "Prose that makes the file long enough to be caught half written.\n".repeat(400);
    let text = format!("---\ntitle: Busy\nstatus: todo\n---\n{body}The last line.\n");
    let (root, writer) = folder(&[("tasks/a.md", &text)], json!({}));
    let reader = onetaskgraph_local_md::Plugin
        .build(
            &SourceName::new("work").unwrap(),
            &json!({ "root": root.path() }),
            &NoSecrets,
        )
        .expect("a second source over the same folder");
    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let reading = {
        let done = std::sync::Arc::clone(&done);
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap();
            let whole = |task: &Task| {
                assert_eq!(task.title, "Busy");
                assert!(
                    task.content
                        .as_deref()
                        .is_some_and(|content| content.ends_with("The last line.")),
                    "the body was cut short"
                );
                assert!(
                    matches!(
                        task.status.category,
                        StatusCategory::Todo | StatusCategory::Done
                    ),
                    "{:?}",
                    task.status
                );
                assert!(task.delivered_by.len() <= 1, "{:?}", task.delivered_by);
            };
            let mut reads = 0_u32;
            while !done.load(std::sync::atomic::Ordering::SeqCst) || reads == 0 {
                let task = runtime
                    .block_on(reader.get_task(&id("a")))
                    .expect("the record always reads")
                    .expect("the record is always there");
                whole(&task);
                let listed = runtime
                    .block_on(reader.query_tasks(
                        &onetaskgraph_plugin_api::TaskQuery::default(),
                        &onetaskgraph_plugin_api::PageRequest {
                            limit: 200,
                            cursor: None,
                        },
                    ))
                    .expect("the folder always lists");
                assert_eq!(listed.items.len(), 1, "the record is always listed, once");
                whole(&listed.items[0]);
                reads += 1;
            }
            reads
        })
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    for round in 1..=150 {
        let category = if round % 2 == 0 {
            StatusCategory::Todo
        } else {
            StatusCategory::Done
        };
        let status = runtime
            .block_on(writer.set_task_status(&id("a"), category))
            .unwrap()
            .expect("held");
        assert_eq!(status.category, category);
        let delivered_by = if round % 2 == 0 {
            Vec::new()
        } else {
            vec![entry(&format!("plan:P-{round}"))]
        };
        runtime
            .block_on(writer.set_delivered_by(&id("a"), &delivered_by))
            .unwrap()
            .expect("held");
    }
    done.store(true, std::sync::atomic::Ordering::SeqCst);
    let reads = reading
        .join()
        .expect("the reader never saw a broken record");
    assert!(reads > 0);
    assert_eq!(
        listing(root.path()),
        [
            std::path::PathBuf::from("tasks"),
            std::path::PathBuf::from("tasks/a.md")
        ]
        .into()
    );
}
