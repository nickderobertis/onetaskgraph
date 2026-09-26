//! A task file's `priority:` key: how it reads, how a copy writes it, how a priority-only
//! write rewrites it, and how a query filters by it.
//!
//! Every test drives the real plugin over a real folder and asserts on what a read answers
//! and on the bytes the file holds afterwards: a priority-only write is a promise about the
//! file, not only about what a read reports.

use std::fs;

use onetaskgraph_plugin_api::{
    ItemWrite, NativeId, PageRequest, Priority, SecretResolver, SourceError, SourceName,
    SourcePlugin, Status, StatusCategory, Task, TaskQuery, TaskSource,
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
    fs::create_dir_all(root.path().join("tasks")).expect("the tasks folder");
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

fn malformed(error: SourceError) -> String {
    match error {
        SourceError::Malformed { message } => message,
        other => panic!("expected the file to be reported malformed, got {other:?}"),
    }
}

async fn ids(source: &dyn TaskSource, priorities: Vec<Priority>) -> Vec<String> {
    let mut ids: Vec<String> = source
        .query_tasks(
            &TaskQuery {
                priorities,
                ..TaskQuery::default()
            },
            &page(),
        )
        .await
        .expect("the folder answers")
        .items
        .into_iter()
        .map(|task| task.id.0)
        .collect();
    ids.sort();
    ids
}

/// A task file whose front matter uses every shape the rewrite has to step around, and a
/// comments section after its body.
const RICH: &str = "---\ntitle: Alpha\nstatus: todo\nlabels: [Bug]\ndepends_on:\n  - b\n  - id: other:P-1\n    item: project\nmetadata:\n  caller.count: 3\ndelivers:\n- b\ndelivered_by: [\"plan:P-9\"]\n---\n# Alpha\n\nThe body.\n\n## Comments\n\n<!-- onetaskgraph:comment id=\"20260913T151107Z-1\" created_at=\"2026-09-13T15:11:07Z\" updated_at=\"2026-09-13T15:11:07Z\" -->\n### comment — 2026-09-13T15:11:07Z\n\nKeep me.\n\n<!-- /onetaskgraph:comment -->\n";

#[tokio::test]
async fn an_absent_key_reads_none_and_every_value_reads_as_itself() {
    let mut files = vec![(
        "tasks/absent.md".to_owned(),
        "---\ntitle: A\n---\n".to_owned(),
    )];
    for priority in Priority::ALL {
        files.push((
            format!("tasks/{priority}.md"),
            format!("---\ntitle: {priority}\npriority: {priority}\n---\n"),
        ));
    }
    let files: Vec<(&str, &str)> = files
        .iter()
        .map(|(path, text)| (path.as_str(), text.as_str()))
        .collect();
    let (_root, source) = folder(&files);
    assert_eq!(
        source
            .get_task(&id("absent"))
            .await
            .unwrap()
            .expect("held")
            .priority,
        Priority::None
    );
    for priority in Priority::ALL {
        let task = source
            .get_task(&id(priority.as_str()))
            .await
            .unwrap()
            .expect("held");
        assert_eq!(task.priority, priority);
    }
}

#[tokio::test]
async fn a_value_that_is_not_a_priority_makes_the_file_malformed_naming_the_key() {
    for (value, named) in [
        ("critical", "\"critical\""),
        ("3", "3"),
        ("High", "\"High\""),
        // An explicit null is a value that is not a priority, not the key being absent.
        ("null", "null"),
        ("~", "null"),
    ] {
        let (_root, source) = folder(&[(
            "tasks/a.md",
            &format!("---\ntitle: A\npriority: {value}\n---\n"),
        )]);
        for message in [
            malformed(
                source
                    .get_task(&id("a"))
                    .await
                    .expect_err("a read of the file is refused"),
            ),
            malformed(
                source
                    .query_tasks(&TaskQuery::default(), &page())
                    .await
                    .expect_err("a listing holding the file is refused"),
            ),
        ] {
            assert!(message.contains("a.md"), "{message}");
            assert!(message.contains("`priority`"), "{message}");
            assert!(message.contains(named), "{message}");
            assert!(
                message.contains("none, urgent, high, medium or low"),
                "{message}"
            );
        }
    }
}

#[tokio::test]
async fn a_project_carrying_a_priority_is_refused_rather_than_read_and_dropped() {
    for value in ["high", "null"] {
        let (_root, source) = folder(&[(
            "projects/p.md",
            &format!("---\ntitle: P\npriority: {value}\n---\n"),
        )]);
        let message = malformed(
            source
                .get_project(&id("p"))
                .await
                .expect_err("a project has no priority"),
        );
        assert!(
            message.contains("p.md") && message.contains("`priority` belongs to a task"),
            "{value}: {message}"
        );
    }
}

#[tokio::test]
async fn the_filter_keeps_exactly_the_tasks_holding_a_value_asked_for() {
    let (_root, source) = folder(&[
        ("tasks/absent.md", "---\ntitle: A\n---\n"),
        ("tasks/explicit.md", "---\ntitle: E\npriority: none\n---\n"),
        ("tasks/urgent.md", "---\ntitle: U\npriority: urgent\n---\n"),
        ("tasks/high.md", "---\ntitle: H\npriority: high\n---\n"),
        ("tasks/low.md", "---\ntitle: L\npriority: low\n---\n"),
    ]);
    let source = source.as_ref();
    assert_eq!(
        ids(source, Vec::new()).await,
        ["absent", "explicit", "high", "low", "urgent"]
    );
    assert_eq!(
        ids(source, vec![Priority::None]).await,
        ["absent", "explicit"]
    );
    assert_eq!(ids(source, vec![Priority::Urgent]).await, ["urgent"]);
    assert_eq!(
        ids(source, vec![Priority::High, Priority::Low]).await,
        ["high", "low"]
    );
    assert_eq!(
        ids(source, vec![Priority::Medium]).await,
        Vec::<String>::new()
    );
}

/// A task a copy would write, holding `priority`.
fn outgoing(priority: Priority) -> Task {
    Task {
        id: id("copied"),
        key: None,
        title: "Copied".to_owned(),
        content: Some("Copied body.".to_owned()),
        status: Status {
            category: StatusCategory::Todo,
            name: "todo".to_owned(),
        },
        priority,
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

#[tokio::test]
async fn a_copy_writes_the_key_only_for_a_priority_that_is_not_none() {
    let (root, source) = folder(&[]);

    // Created with a priority: the key is written, and reads back.
    let created = source
        .write_task(&ItemWrite {
            target: None,
            item: outgoing(Priority::High),
            depends_on: Vec::new(),
        })
        .await
        .unwrap();
    let path = format!("tasks/{created}.md");
    assert_eq!(
        read(&root, &path),
        "---\ntitle: Copied\nstatus: todo\npriority: high\n---\nCopied body.\n"
    );
    let task = source.get_task(&created).await.unwrap().expect("held");
    assert_eq!(task.priority, Priority::High);

    // Updated with another: the key moves with it.
    source
        .write_task(&ItemWrite {
            target: Some(created.clone()),
            item: outgoing(Priority::Low),
            depends_on: Vec::new(),
        })
        .await
        .unwrap();
    assert_eq!(
        source.get_task(&created).await.unwrap().unwrap().priority,
        Priority::Low
    );
    assert!(read(&root, &path).contains("\npriority: low\n"));

    // Updated with none: no key at all, which is the file a copy wrote before priorities.
    source
        .write_task(&ItemWrite {
            target: Some(created.clone()),
            item: outgoing(Priority::None),
            depends_on: Vec::new(),
        })
        .await
        .unwrap();
    assert_eq!(
        read(&root, &path),
        "---\ntitle: Copied\nstatus: todo\n---\nCopied body.\n"
    );
    assert_eq!(
        source.get_task(&created).await.unwrap().unwrap().priority,
        Priority::None
    );

    // Created with none: no key either.
    let bare = source
        .write_task(&ItemWrite {
            target: None,
            item: outgoing(Priority::None),
            depends_on: Vec::new(),
        })
        .await
        .unwrap();
    assert!(
        !read(&root, &format!("tasks/{bare}.md")).contains("priority"),
        "`none` writes no key"
    );
}

#[tokio::test]
async fn a_priority_set_rewrites_the_priority_entry_and_not_one_other_byte() {
    let (root, source) = folder(&[("tasks/a.md", RICH)]);
    let before = source.get_task(&id("a")).await.unwrap().expect("held");
    let with = |word: &str| {
        RICH.replacen(
            "delivered_by: [\"plan:P-9\"]\n---\n",
            &format!("delivered_by: [\"plan:P-9\"]\npriority: {word}\n---\n"),
            1,
        )
    };

    // Added as the last entry of the front matter, then replaced in place.
    for priority in [Priority::High, Priority::Urgent, Priority::Low] {
        let answered = source.set_task_priority(&id("a"), priority).await.unwrap();
        assert_eq!(answered, Some(priority));
        assert_eq!(
            read(&root, "tasks/a.md"),
            with(priority.as_str()),
            "only the priority entry moves"
        );
        let after = source.get_task(&id("a")).await.unwrap().expect("held");
        assert_eq!(
            after,
            Task {
                priority,
                ..before.clone()
            }
        );
    }

    // Cleared: the line goes, and the file is byte for byte the one it was.
    let answered = source
        .set_task_priority(&id("a"), Priority::None)
        .await
        .unwrap();
    assert_eq!(answered, Some(Priority::None));
    assert_eq!(read(&root, "tasks/a.md"), RICH);
    assert_eq!(source.get_task(&id("a")).await.unwrap(), Some(before));
}

#[tokio::test]
async fn a_priority_entry_in_the_middle_is_replaced_and_removed_where_it_stands() {
    let (root, source) = folder(&[
        (
            "tasks/lf.md",
            "---\ntitle: A\npriority: low\nstatus: todo\n---\nbody\n",
        ),
        (
            "tasks/crlf.md",
            "---\r\ntitle: A\r\npriority: low\r\nstatus: todo\r\n---\r\nbody\r\n",
        ),
    ]);
    source
        .set_task_priority(&id("lf"), Priority::Medium)
        .await
        .unwrap();
    assert_eq!(
        read(&root, "tasks/lf.md"),
        "---\ntitle: A\npriority: medium\nstatus: todo\n---\nbody\n"
    );
    source
        .set_task_priority(&id("lf"), Priority::None)
        .await
        .unwrap();
    assert_eq!(
        read(&root, "tasks/lf.md"),
        "---\ntitle: A\nstatus: todo\n---\nbody\n"
    );
    source
        .set_task_priority(&id("crlf"), Priority::Urgent)
        .await
        .unwrap();
    assert_eq!(
        read(&root, "tasks/crlf.md"),
        "---\r\ntitle: A\r\npriority: urgent\r\nstatus: todo\r\n---\r\nbody\r\n"
    );
    source
        .set_task_priority(&id("crlf"), Priority::None)
        .await
        .unwrap();
    assert_eq!(
        read(&root, "tasks/crlf.md"),
        "---\r\ntitle: A\r\nstatus: todo\r\n---\r\nbody\r\n"
    );
}

#[tokio::test]
async fn setting_the_priority_a_task_already_holds_writes_nothing() {
    // An explicit `none` is kept rather than removed: nothing about the task changes, so
    // nothing about the file does.
    let explicit = "---\ntitle: A\npriority: none\n---\nbody\n";
    let held = "---\ntitle: B\npriority:   high   # keep\n---\nbody\n";
    let (root, source) = folder(&[("tasks/a.md", explicit), ("tasks/b.md", held)]);
    assert_eq!(
        source
            .set_task_priority(&id("a"), Priority::None)
            .await
            .unwrap(),
        Some(Priority::None)
    );
    assert_eq!(read(&root, "tasks/a.md"), explicit);
    assert_eq!(
        source
            .set_task_priority(&id("b"), Priority::High)
            .await
            .unwrap(),
        Some(Priority::High)
    );
    assert_eq!(read(&root, "tasks/b.md"), held);
}

#[tokio::test]
async fn a_priority_set_on_a_task_this_folder_does_not_hold_answers_none() {
    let (root, source) = folder(&[("tasks/a.md", "---\ntitle: A\n---\n")]);
    assert_eq!(
        source
            .set_task_priority(&id("missing"), Priority::High)
            .await
            .unwrap(),
        None
    );
    assert!(!root.path().join("tasks/missing.md").exists());
}
