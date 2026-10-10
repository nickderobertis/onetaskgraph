//! A targeted update of a task this source wrote itself, whatever layout its writer gave the
//! task's metadata values.
//!
//! Most tests write the record through the plugin's own writer — `write_task`,
//! `write_task_rendered`, `write_project` or `write_document` — so the file holds exactly the
//! layout this source produces: block sequences, nested mappings, and the block scalar headers
//! its writer emits for a multi-line string. The rest start from a hand-written file, to reach
//! the edits and the refusal a writer's layout never needs. Each updates through the plugin's
//! own `update_task` where it updates at all, asserting on what a read answers and on the
//! file's exact bytes: every field the update did not name reads back unchanged, and every
//! byte of it is where it was.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use onetaskgraph_plugin_api::{
    Document, ItemWrite, MetadataKey, NativeId, Priority, Project, SecretResolver, SourceError,
    SourceName, SourcePlugin, Status, StatusCategory, Task, TaskRef, TaskSource, TaskUpdate,
    UpdatedField,
};
use secrecy::SecretString;
use serde_json::{Value, json};

struct NoSecrets;
impl SecretResolver for NoSecrets {
    fn get(&self, _: &str) -> Option<SecretString> {
        None
    }
}

fn folder() -> (tempfile::TempDir, Box<dyn TaskSource>) {
    let root = tempfile::tempdir().expect("temporary notes");
    let source = onetaskgraph_local_md::Plugin
        .build(
            &SourceName::new("work").unwrap(),
            &json!({ "root": root.path() }),
            &NoSecrets,
        )
        .expect("the folder builds");
    (root, source)
}

fn key(value: &str) -> MetadataKey {
    MetadataKey::new(value).expect("a caller-owned key")
}

/// What onepipeline writes for a node with `steps`: step mappings, each ending in a
/// multi-line `task` this source's writer lays out as a block scalar — `|` for one ending in a
/// single line break, `|+` for one ending in a blank line.
fn steps(task: &str) -> Value {
    json!([
        {"id": "build", "persona": "engineer", "task": task},
        {"id": "check", "persona": "reviewer", "deps": ["build"], "task": task},
    ])
}

/// One value in each block scalar header this source's writer was seen to emit for a string —
/// `|`, `|-`, `|+`, `|2+` and `|2` — and in each shape of collection it nests them in.
fn writer_layouts() -> Vec<(&'static str, Value)> {
    let task = "## What\nWork.\n\n## Acceptance criteria\n\n- x\n";
    vec![
        ("a plain scalar", json!("plain")),
        ("a multi-line string (`|`)", json!("first\n\nthird\n")),
        (
            "a multi-line string with no final break (`|-`)",
            json!("first\nlast"),
        ),
        (
            "a multi-line string ending in a blank line (`|+`)",
            json!("kept\n\n"),
        ),
        ("a string of blank lines (`|2+`)", json!("\n\n")),
        (
            "a multi-line string led by spaces (`|2`)",
            json!("  led\nx\n"),
        ),
        ("a sequence of scalars", json!(["a", "b"])),
        ("a mapping", json!({"a": 1, "b": "two"})),
        (
            "a sequence of mappings ending in a multi-line string",
            steps(task),
        ),
        (
            "a sequence of mappings ending in a string ending in a blank line",
            steps("## What\nWork.\n\n"),
        ),
        (
            "a nested sequence and mapping",
            json!({"outer": [{"inner": {"deep": ["a", "b\nc\n"]}}], "tail": [[1, [2, "x\n\n"]]]}),
        ),
    ]
}

/// Where the value sits among the task's front matter: which metadata key holds it, the other
/// metadata key beside it, and whether a `delivers:` entry follows the `metadata:` block rather
/// than the block ending the front matter.
#[derive(Clone, Copy, Debug)]
struct Arrangement {
    value: &'static str,
    other: &'static str,
    delivers: bool,
}

const ARRANGEMENTS: [Arrangement; 4] = [
    Arrangement {
        value: "zzz.value",
        other: "aaa.other",
        delivers: false,
    },
    Arrangement {
        value: "zzz.value",
        other: "aaa.other",
        delivers: true,
    },
    Arrangement {
        value: "aaa.value",
        other: "zzz.other",
        delivers: false,
    },
    Arrangement {
        value: "aaa.value",
        other: "zzz.other",
        delivers: true,
    },
];

fn task(arrangement: Arrangement, value: &Value) -> Task {
    Task {
        id: NativeId::from("layout"),
        key: None,
        title: "Layout".to_owned(),
        content: Some("The body.\n".to_owned()),
        status: Status {
            category: StatusCategory::Todo,
            name: "todo".to_owned(),
        },
        priority: Priority::None,
        labels: Vec::new(),
        project: None,
        url: None,
        location: None,
        created_at: None,
        updated_at: None,
        metadata: BTreeMap::from([
            (arrangement.value.to_owned(), value.clone()),
            (arrangement.other.to_owned(), json!(1)),
        ]),
        repositories: Vec::new(),
        delivers: if arrangement.delivers {
            vec![TaskRef::new("elsewhere").expect("a task id")]
        } else {
            Vec::new()
        },
        delivered_by: Vec::new(),
        classification: Default::default(),
    }
}

/// A task holding `value` in `arrangement`, written by this source, and the file it wrote.
async fn written(
    arrangement: Arrangement,
    value: &Value,
) -> (tempfile::TempDir, Box<dyn TaskSource>, NativeId, String) {
    let (root, source) = folder();
    let id = source
        .write_task(&ItemWrite {
            target: None,
            item: task(arrangement, value),
            depends_on: Vec::new(),
        })
        .await
        .expect("the task is written");
    let file = fs::read_to_string(root.path().join(format!("tasks/{}.md", id.as_str())))
        .expect("the task file");
    (root, source, id, file)
}

/// `text` with exactly one occurrence of `from` replaced by `to`.
fn once(text: &str, from: &str, to: &str) -> String {
    assert_eq!(text.matches(from).count(), 1, "{from:?} once in:\n{text}");
    text.replacen(from, to, 1)
}

/// One targeted update of a field other than the value's, the field it writes, and the file it
/// must leave: the written one with exactly that field's bytes changed.
fn updates(arrangement: Arrangement, file: &str) -> Vec<(TaskUpdate, UpdatedField, String)> {
    let set = "  \"mmm.new\": \"x\"\n";
    // The added entry goes after the block's last entry: after the value's own lines when the
    // value is last — and after the blank lines a `|+` value keeps — else after the other key.
    let with_set = {
        let (front, body) = file[4..].split_once("\n---\n").expect("front matter");
        let front = format!("{front}\n");
        let block_end = front.find("delivers:").unwrap_or(front.len());
        format!(
            "---\n{}{set}{}---\n{body}",
            &front[..block_end],
            &front[block_end..]
        )
    };
    vec![
        (
            TaskUpdate {
                status: Some(Status {
                    category: StatusCategory::Done,
                    name: "done".to_owned(),
                }),
                ..TaskUpdate::default()
            },
            UpdatedField::Status,
            once(file, "\nstatus: todo\n", "\nstatus: done\n"),
        ),
        (
            TaskUpdate {
                title: Some("Renamed".to_owned()),
                ..TaskUpdate::default()
            },
            UpdatedField::Title,
            once(file, "---\ntitle: Layout\n", "---\ntitle: Renamed\n"),
        ),
        (
            TaskUpdate {
                content: Some("A new body.\n\nTwo paragraphs.\n".to_owned()),
                ..TaskUpdate::default()
            },
            UpdatedField::Content,
            once(
                file,
                "\n---\nThe body.\n",
                "\n---\nA new body.\n\nTwo paragraphs.\n",
            ),
        ),
        (
            TaskUpdate {
                metadata_set: BTreeMap::from([(key("mmm.new"), json!("x"))]),
                ..TaskUpdate::default()
            },
            UpdatedField::Metadata,
            with_set,
        ),
        (
            TaskUpdate {
                metadata_remove: BTreeSet::from([key(arrangement.other)]),
                ..TaskUpdate::default()
            },
            UpdatedField::Metadata,
            once(file, &format!("  {}: 1\n", arrangement.other), ""),
        ),
    ]
}

#[tokio::test]
async fn a_targeted_update_of_another_field_leaves_each_writer_layout_byte_for_byte() {
    for (layout, value) in writer_layouts() {
        for arrangement in ARRANGEMENTS {
            let (_, _, _, file) = written(arrangement, &value).await;
            for (update, field, expected) in updates(arrangement, &file) {
                let (root, source, id, _) = written(arrangement, &value).await;
                let before = source.get_task(&id).await.unwrap().expect("held");
                assert_eq!(
                    before.metadata[arrangement.value], value,
                    "{layout}, {arrangement:?}: the task this source wrote reads back as written"
                );
                let outcome = source
                    .update_task(&id, &update)
                    .await
                    .unwrap_or_else(|error| {
                        panic!("{layout}, {arrangement:?}, {field:?}: refused: {error:?}\n{file}")
                    })
                    .expect("held");
                assert_eq!(
                    outcome.written,
                    BTreeSet::from([field]),
                    "{layout}, {arrangement:?}"
                );
                let after =
                    fs::read_to_string(root.path().join(format!("tasks/{}.md", id.as_str())))
                        .expect("the task file");
                assert_eq!(
                    after, expected,
                    "{layout}, {arrangement:?}, {field:?}: exactly the named field's bytes moved"
                );
                let read = source.get_task(&id).await.unwrap().expect("held");
                assert_eq!(read, outcome.task, "the answer is the read");
                assert_eq!(
                    read.metadata[arrangement.value], value,
                    "{layout}, {arrangement:?}, {field:?}"
                );
                let mut wanted = before.clone();
                wanted.title = read.title.clone();
                wanted.status = read.status.clone();
                wanted.content = read.content.clone();
                wanted.metadata = read.metadata.clone();
                assert_eq!(
                    read, wanted,
                    "{layout}, {arrangement:?}, {field:?}: nothing unnamed moved"
                );
            }
        }
    }
}

#[tokio::test]
async fn a_rendered_task_ending_in_a_kept_block_scalar_reads_back_as_written_and_updates() {
    // A rendered write lays its answers block after the front matter, which is the other
    // branch of the writer that closes it.
    let kept = steps("## What\nWork.\n\n");
    let answers = BTreeMap::from([("goal".to_owned(), json!("Ship it"))]);
    let (root, source) = folder();
    let id = source
        .write_task_rendered(
            &ItemWrite {
                target: None,
                item: task(ARRANGEMENTS[0], &kept),
                depends_on: Vec::new(),
            },
            &answers,
        )
        .await
        .expect("the task is written");
    let path = root.path().join(format!("tasks/{}.md", id.as_str()));
    let file = fs::read_to_string(&path).expect("the task file");
    assert!(file.contains("task: |+\n"), "{file}");
    let before = source.get_task(&id).await.unwrap().expect("held");
    assert_eq!(before.metadata["zzz.value"], kept, "{file}");

    let outcome = source
        .update_task(
            &id,
            &TaskUpdate {
                title: Some("Renamed".to_owned()),
                metadata_remove: BTreeSet::from([key("aaa.other")]),
                ..TaskUpdate::default()
            },
        )
        .await
        .expect("the update lands")
        .expect("held");
    assert_eq!(
        outcome.written,
        BTreeSet::from([UpdatedField::Title, UpdatedField::Metadata])
    );
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        once(
            &once(&file, "---\ntitle: Layout\n", "---\ntitle: Renamed\n"),
            "  aaa.other: 1\n",
            ""
        )
    );
    assert_eq!(outcome.task.metadata["zzz.value"], kept);
    assert_eq!(
        source.task_template_answers(&id).await.unwrap(),
        Some(answers),
        "the answers block is where it was"
    );
}

#[tokio::test]
async fn a_project_and_a_document_ending_in_a_block_scalar_read_back_as_written() {
    for value in [steps("## What\nWork.\n"), steps("## What\nWork.\n\n")] {
        let metadata = BTreeMap::from([("zzz.value".to_owned(), value.clone())]);
        let (_root, source) = folder();
        let project = source
            .write_project(&ItemWrite {
                target: None,
                item: Project {
                    id: NativeId::from("launch"),
                    title: "Launch".to_owned(),
                    content: None,
                    status: Status {
                        category: StatusCategory::Todo,
                        name: "todo".to_owned(),
                    },
                    labels: Vec::new(),
                    url: None,
                    location: None,
                    created_at: None,
                    updated_at: None,
                    metadata: metadata.clone(),
                    repositories: Vec::new(),
                    classification: Default::default(),
                },
                depends_on: Vec::new(),
            })
            .await
            .expect("the project is written");
        let document = source
            .write_document(&ItemWrite {
                target: None,
                item: Document {
                    id: NativeId::from("design"),
                    title: "Design".to_owned(),
                    content: None,
                    project: None,
                    labels: Vec::new(),
                    url: None,
                    location: None,
                    created_at: None,
                    updated_at: None,
                    metadata: metadata.clone(),
                    repositories: Vec::new(),
                    classification: Default::default(),
                },
                depends_on: Vec::new(),
            })
            .await
            .expect("the document is written");
        let project = source.get_project(&project).await.unwrap().expect("held");
        assert_eq!(project.metadata, metadata);
        let document = source.get_document(&document).await.unwrap().expect("held");
        assert_eq!(document.metadata, metadata);
    }
}

#[tokio::test]
async fn an_update_naming_the_key_that_holds_steps_replaces_or_removes_it() {
    let held = steps("## What\nWork.\n\n## Acceptance criteria\n\n- x\n");
    let replacement = steps("## What\nOther work.\n\n");
    for arrangement in ARRANGEMENTS {
        let (_root, source, id, _) = written(arrangement, &held).await;
        let before = source.get_task(&id).await.unwrap().expect("held");

        let set = source
            .update_task(
                &id,
                &TaskUpdate {
                    metadata_set: BTreeMap::from([(key(arrangement.value), replacement.clone())]),
                    ..TaskUpdate::default()
                },
            )
            .await
            .unwrap_or_else(|error| panic!("{arrangement:?}: {error:?}"))
            .expect("held");
        assert_eq!(set.written, BTreeSet::from([UpdatedField::Metadata]));
        let mut wanted = before.clone();
        wanted
            .metadata
            .insert(arrangement.value.to_owned(), replacement.clone());
        assert_eq!(source.get_task(&id).await.unwrap(), Some(wanted.clone()));

        let removed = source
            .update_task(
                &id,
                &TaskUpdate {
                    metadata_remove: BTreeSet::from([key(arrangement.value)]),
                    ..TaskUpdate::default()
                },
            )
            .await
            .unwrap_or_else(|error| panic!("{arrangement:?}: {error:?}"))
            .expect("held");
        assert_eq!(removed.written, BTreeSet::from([UpdatedField::Metadata]));
        wanted.metadata.remove(arrangement.value);
        assert_eq!(source.get_task(&id).await.unwrap(), Some(wanted));
    }

    for arrangement in ARRANGEMENTS {
        let (_root, source, id, _) = written(arrangement, &held).await;
        let mut wanted = source.get_task(&id).await.unwrap().expect("held");
        source
            .update_task(
                &id,
                &TaskUpdate {
                    metadata_remove: BTreeSet::from([key(arrangement.value)]),
                    ..TaskUpdate::default()
                },
            )
            .await
            .unwrap_or_else(|error| panic!("{arrangement:?}: {error:?}"))
            .expect("held");
        wanted.metadata.remove(arrangement.value);
        assert_eq!(source.get_task(&id).await.unwrap(), Some(wanted));
    }
}

/// The task file `text`, updated by `update`: the file it leaves, and the task a read reports.
async fn updated(text: &str, update: TaskUpdate) -> (String, Task) {
    let (root, source) = folder();
    let path = root.path().join("tasks/a.md");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, text).unwrap();
    let id = NativeId::from("a");
    source
        .update_task(&id, &update)
        .await
        .unwrap_or_else(|error| panic!("{text:?}: refused: {error:?}"))
        .expect("held");
    let task = source.get_task(&id).await.unwrap().expect("held");
    (fs::read_to_string(&path).unwrap(), task)
}

#[tokio::test]
async fn a_top_level_entry_is_edited_around_a_block_scalar_and_as_one() {
    // Removing the entry after a kept scalar leaves the scalar's blank line where it was.
    let (file, task) = updated(
        "---\ntitle: T\nmetadata:\n  a.b: |+\n    x\n\npriority: high\n---\n",
        TaskUpdate {
            priority: Some(Priority::None),
            ..TaskUpdate::default()
        },
    )
    .await;
    assert_eq!(file, "---\ntitle: T\nmetadata:\n  a.b: |+\n    x\n\n---\n");
    assert_eq!(task.metadata["a.b"], json!("x\n\n"));

    // A title that is itself a block scalar, with a blank line inside it and the blank lines
    // it keeps after it, is replaced whole.
    for title in [
        "---\ntitle: |\n  a\n\n  b\nstatus: todo\n---\n",
        "---\ntitle: |+\n  a\n\nstatus: todo\n---\n",
    ] {
        let (file, task) = updated(
            title,
            TaskUpdate {
                title: Some("Renamed".to_owned()),
                ..TaskUpdate::default()
            },
        )
        .await;
        assert_eq!(
            file, "---\ntitle: Renamed\nstatus: todo\n---\n",
            "{title:?}"
        );
        assert_eq!(task.title, "Renamed");
    }

    // A Windows file ending in a block scalar reads it with its line break, and an edit around
    // it keeps every line ending the file has.
    let crlf = "---\r\ntitle: T\r\nmetadata:\r\n  a.b: |\r\n    x\r\n---\r\n";
    let (file, task) = updated(
        crlf,
        TaskUpdate {
            title: Some("Renamed".to_owned()),
            metadata_set: BTreeMap::from([(key("c.d"), json!(1))]),
            ..TaskUpdate::default()
        },
    )
    .await;
    assert_eq!(
        file,
        "---\r\ntitle: Renamed\r\nmetadata:\r\n  a.b: |\r\n    x\r\n  \"c.d\": 1\r\n---\r\n"
    );
    assert_eq!(task.metadata["a.b"], json!("x\n"));
}

#[tokio::test]
async fn an_edit_that_cannot_be_made_in_place_is_still_refused_and_one_that_can_is_not() {
    // A comment in column 0 ends the `metadata:` block this source can see, while YAML reads on
    // past it: an entry added to the block this source sees would read back as another value
    // of `myapp.b` than the one named. The title of the same file is an ordinary edit.
    let text =
        "---\ntitle: T\nstatus: todo\nmetadata:\n  myapp.a: 1\n# a note\n  myapp.b: 2\n---\n";
    let (root, source) = folder();
    let path = root.path().join("tasks/a.md");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, text).unwrap();
    let id = NativeId::from("a");

    let error = source
        .update_task(
            &id,
            &TaskUpdate {
                metadata_set: BTreeMap::from([(key("myapp.b"), json!(3))]),
                ..TaskUpdate::default()
            },
        )
        .await
        .expect_err("the edit cannot be made in place");
    let SourceError::Refused { message } = &error else {
        panic!("refused as {error:?}");
    };
    assert!(
        message.contains(
            "cannot update this task without changing more than the update names: the edited \
             file would read back otherwise"
        ),
        "{message}"
    );
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        text,
        "the file as it was"
    );

    let outcome = source
        .update_task(
            &id,
            &TaskUpdate {
                title: Some("Renamed".to_owned()),
                ..TaskUpdate::default()
            },
        )
        .await
        .expect("the title is written in place")
        .expect("held");
    assert_eq!(outcome.written, BTreeSet::from([UpdatedField::Title]));
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        text.replace("title: T\n", "title: Renamed\n")
    );
}
