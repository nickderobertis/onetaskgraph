//! The batched store calls through the binary, over a folder of Markdown and an in-memory
//! source: `task find`'s many searches, `task copy` holding each counterpart to its own
//! `--if-version <ID>=<VERSION>`, `task comment add-many`, `task update-many`, and `task update
//! --when-priority` — each answered per item, one item's failure never writing or refusing
//! another's — and each malformed batch document refused whole, naming what is wrong.
//!
//! On the folder the "other writer" is a second process writing the same task through the
//! store, unheld, between the read and the batch, and every item is read back by a later
//! command. An in-memory source's work dies with its process, so there the other writer is its
//! configuration, edited between the read and the batch — the one way its task can be at
//! another version by the time a command runs — and what the batch wrote is asserted on the
//! batch's own answer. The same calls as a Rust caller makes them are
//! `crates/onetaskgraph-core/tests/batch.rs`.

use std::path::PathBuf;
use std::process::Output;

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};

/// One task of the fixture: its id, title, status, priority, `myapp.colour` and origin.
type Fixture = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    Option<&'static str>,
);

/// Every task both sources start with. Four titles hold `batch`, more than one page of two.
const TASKS: [Fixture; 6] = [
    ("a", "Alpha batch", "todo", "high", "red", None),
    (
        "b",
        "Bravo batch",
        "todo",
        "high",
        "red",
        Some("elsewhere:B-1"),
    ),
    ("c", "Charlie batch", "in-progress", "low", "blue", None),
    ("d", "Delta", "todo", "none", "red", Some("elsewhere:B-1")),
    ("e", "Echo batch", "done", "none", "green", None),
    ("f", "Foxtrot", "todo", "none", "red", None),
];

struct Store {
    sandbox: Sandbox,
    plan: PathBuf,
    /// The in-memory source's tasks as its configuration holds them.
    memory: Vec<Value>,
}

impl Store {
    /// A folder `plan` and an in-memory source `memory` each holding [`TASKS`], an empty folder
    /// `snap` to copy into, and a page size of two.
    fn new() -> Self {
        let sandbox = Sandbox::new();
        let plan = sandbox.subdirectory("plan");
        std::fs::create_dir_all(plan.join("tasks")).expect("the task folder");
        let mut memory = Vec::new();
        for (native, title, status, priority, colour, origin) in TASKS {
            let origin_line = origin.map_or_else(String::new, |origin| {
                format!("  onetaskgraph.origin: {origin}\n")
            });
            std::fs::write(
                plan.join("tasks").join(format!("{native}.md")),
                format!(
                    "---\ntitle: {title}\nstatus: {status}\npriority: {priority}\nmetadata:\n  \
                     myapp.colour: {colour}\n{origin_line}---\nThe {native} body.\n"
                ),
            )
            .expect("a task file");
            let mut metadata = json!({"myapp.colour": colour});
            if let Some(origin) = origin {
                metadata["onetaskgraph.origin"] = json!(origin);
            }
            memory.push(json!({
                "id": native, "title": title, "labels": [], "priority": priority,
                "content": format!("The {native} body."),
                "status": {"category": status, "name": status}, "metadata": metadata,
            }));
        }
        sandbox.subdirectory("snap");
        let store = Self {
            sandbox,
            plan,
            memory,
        };
        store.configure();
        store
    }

    /// Write the configuration, the in-memory source holding `self.memory`.
    fn configure(&self) {
        let document = json!({
            "page_size": 2,
            "sources": {
                "plan": {"plugin": "local-md", "config": {"root": self.plan}},
                "snap": {"plugin": "local-md",
                         "config": {"root": self.sandbox.project().join("snap")}},
                "memory": {"plugin": "in-memory",
                           "config": {"capabilities": {"priority": "native",
                                                       "comments": "native"},
                                      "tasks": self.memory}},
            },
        });
        self.sandbox
            .project_document(&serde_json::to_string_pretty(&document).expect("JSON"));
    }

    /// The in-memory source's other writer: its task `native` retitled in its configuration.
    fn retitle_in_memory(&mut self, native: &str, title: &str) {
        let task = self
            .memory
            .iter_mut()
            .find(|task| task["id"] == native)
            .expect("a seeded task");
        task["title"] = json!(title);
        self.configure();
    }

    fn run(&self, arguments: &[&str], input: Option<&str>) -> Output {
        let mut command = self.sandbox.command();
        command.arg("--json").args(arguments);
        command.write_stdin(input.unwrap_or_default());
        command.assert().get_output().clone()
    }

    /// `onetaskgraph <arguments>` exiting `code`, with `input` on standard input, as JSON.
    fn exited(&self, arguments: &[&str], input: Option<&str>, code: i32) -> Value {
        let output = self.run(arguments, input);
        assert_eq!(
            output.status.code(),
            Some(code),
            "`onetaskgraph {}`\nstdout:\n{}\nstderr:\n{}",
            arguments.join(" "),
            stdout(&output),
            stderr(&output)
        );
        serde_json::from_str(&stdout(&output)).expect("the run answers JSON")
    }

    /// A batch document written to a file, its path.
    fn file(&self, name: &str, document: &Value) -> String {
        let path = self.sandbox.project().join(name);
        std::fs::write(&path, document.to_string()).expect("a batch file");
        path.to_str().expect("a path").to_owned()
    }

    fn shown(&self, id: &str) -> Value {
        self.exited(&["task", "show", id], None, 0)["items"][0]["item"].clone()
    }

    fn version(&self, id: &str) -> String {
        let task = self.shown(id);
        task["version"]
            .as_str()
            .unwrap_or_else(|| panic!("{id} is read with a version: {task:#}"))
            .to_owned()
    }

    /// The folder's other writer: a second process retitling `id`, unheld.
    fn other_writer(&self, id: &str, title: &str) {
        self.exited(&["task", "update", id, "--title", title], None, 0);
    }
}

/// Each result's `outcome`, in order.
fn outcomes(answer: &Value) -> Vec<&str> {
    answer["results"]
        .as_array()
        .expect("results")
        .iter()
        .map(|result| result["outcome"].as_str().expect("an outcome"))
        .collect()
}

/// A task's content, `myapp.colour`, status and priority, as `task show` reads them.
fn fields(task: &Value) -> Value {
    json!([
        task["content"].as_str().map(str::trim_end),
        task["metadata"]["myapp.colour"],
        task["status"]["category"],
        task["priority"],
    ])
}

/// The five searches: by title text, by a metadata value, by origin, by text and status
/// together, and one nothing matches.
fn five_searches() -> Value {
    json!({"schema_version": 1, "queries": [
        {"key": "text", "text": "batch", "in": "title"},
        {"key": "colour", "metadata": {"myapp.colour": "red"}},
        {"key": "origin", "origin": "elsewhere:B-1"},
        {"key": "open", "text": "batch", "status": ["todo", "in-progress"]},
        {"key": "nothing", "text": "no task holds this"},
    ]})
}

#[test]
fn task_find_answers_each_key_with_exactly_its_matches_across_pages() {
    let store = Store::new();
    let file = store.file("queries.json", &five_searches());
    for source in ["plan", "memory"] {
        // From a file on the folder and from standard input on the in-memory source.
        let answer = if source == "plan" {
            store.exited(&["task", "find", source, "--queries", &file], None, 0)
        } else {
            let input = five_searches().to_string();
            store.exited(&["task", "find", source, "--queries", "-"], Some(&input), 0)
        };
        let found: Vec<(String, Vec<String>, bool, Value)> = answer["answers"]
            .as_array()
            .expect("answers")
            .iter()
            .map(|answer| {
                let mut ids: Vec<String> = answer["items"]
                    .as_array()
                    .expect("items")
                    .iter()
                    .map(|item| item["id"].as_str().expect("an id").to_owned())
                    .collect();
                ids.sort();
                (
                    answer["key"].as_str().expect("a key").to_owned(),
                    ids,
                    answer["complete"] == json!(true),
                    answer["reason"].clone(),
                )
            })
            .collect();
        let expected = |key: &str, natives: &[&str]| {
            (
                key.to_owned(),
                natives
                    .iter()
                    .map(|native| format!("{source}:{native}"))
                    .collect::<Vec<_>>(),
                true,
                Value::Null,
            )
        };
        assert_eq!(
            found,
            vec![
                expected("text", &["a", "b", "c", "e"]),
                expected("colour", &["a", "b", "d", "f"]),
                expected("origin", &["b", "d"]),
                expected("open", &["a", "b", "c"]),
                expected("nothing", &[]),
            ],
            "{source}: each key holds exactly its matches, across pages of two"
        );

        let unnarrowed = json!({"schema_version": 1, "queries": [
            {"key": "named", "text": "batch"}, {"key": "everything", "status": ["todo"]},
        ]})
        .to_string();
        let output = store.run(
            &["task", "find", source, "--queries", "-"],
            Some(&unnarrowed),
        );
        assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
        assert!(
            stderr(&output).contains("\"everything\""),
            "the refusal names the key: {}",
            stderr(&output)
        );
        let refused: Value = serde_json::from_str(&stdout(&output)).expect("a failure document");
        assert_eq!(refused["failure"]["kind"], "unnarrowed-query");
    }
}

#[test]
fn task_copy_holds_each_counterpart_named_to_its_own_version() {
    let store = Store::new();
    let ids = ["plan:a", "plan:b", "plan:c", "plan:d"];
    let mut first = vec!["task", "copy"];
    first.extend(ids);
    first.extend(["--to", "snap"]);
    let created = store.exited(&first, None, 0);
    let counterpart = |source: &str| -> String {
        created["items"]
            .as_array()
            .expect("items")
            .iter()
            .find(|item| item["source"] == source)
            .and_then(|item| item["destination"].as_str())
            .unwrap_or_else(|| panic!("{source} was created: {created:#}"))
            .to_owned()
    };
    let read_a = store.version(&counterpart("plan:a"));
    let read_b = store.version(&counterpart("plan:b"));
    // Another writer moves `b`'s counterpart after it was read, and every source task changes.
    store.other_writer(&counterpart("plan:b"), "Bravo, edited at snap");
    let moved_to = store.version(&counterpart("plan:b"));
    assert_ne!(moved_to, read_b);
    let left = store.shown(&counterpart("plan:b"));
    for id in ids {
        let title = format!(
            "{}, revised",
            store.shown(id)["title"].as_str().expect("a title")
        );
        store.exited(&["task", "update", id, "--title", &title], None, 0);
    }

    let hold_a = format!("plan:a={read_a}");
    let hold_b = format!("plan:b={read_b}");
    let mut second = first.clone();
    second.extend(["--if-version", &hold_a, "--if-version", &hold_b]);
    let copied = store.exited(&second, None, 4);
    for item in copied["items"].as_array().expect("items") {
        let source = item["source"].as_str().expect("a source");
        let shown = store.shown(&counterpart(source));
        if source == "plan:b" {
            assert_eq!(item["action"], "failed", "{item:#}");
            assert_eq!(item["error"]["kind"], "version-moved", "{item:#}");
            assert_eq!(item["held"]["version"], json!(moved_to), "{item:#}");
            assert_eq!(
                shown, left,
                "the moved counterpart is as the other writer left it"
            );
        } else {
            assert_eq!(item["action"], "updated", "{source}: {item:#}");
            let title = shown["title"].as_str().expect("a title");
            assert!(title.ends_with(", revised"), "{source}: {title}");
        }
    }
}

#[test]
fn task_comment_add_many_answers_each_comment_on_its_own() {
    let mut store = Store::new();
    // The folder: `b` moved by a second process after it was read.
    let stale = store.version("plan:b");
    store.other_writer("plan:b", "Bravo, moved");
    let current = store.version("plan:c");
    let batch = json!({"schema_version": 1, "comments": [
        {"item": "plan:a", "body": "On a."},
        {"item": "plan:b", "body": "On b.", "if_version": stale},
        {"item": "plan:c", "body": "On c.", "if_version": current},
        {"item": "plan:d", "body": "On d."},
    ]});
    let file = store.file("comments.json", &batch);
    let added = store.exited(
        &["task", "comment", "add-many", "plan", "--input", &file],
        None,
        4,
    );
    assert_eq!(
        outcomes(&added),
        ["written", "version-moved", "written", "written"]
    );
    assert_eq!(
        added["results"][1]["current_version"],
        json!(store.version("plan:b"))
    );
    for (at, native, body) in [(0, "a", "On a."), (2, "c", "On c."), (3, "d", "On d.")] {
        let listed = store.exited(
            &["task", "comment", "list", &format!("plan:{native}")],
            None,
            0,
        );
        assert_eq!(listed["comments"].as_array().map(Vec::len), Some(1));
        assert_eq!(
            listed["comments"][0]["id"],
            added["results"][at]["comment"]["id"]
        );
        assert_eq!(
            listed["comments"][0]["body"].as_str().map(str::trim_end),
            Some(body)
        );
    }
    let untouched = store.exited(&["task", "comment", "list", "plan:b"], None, 0);
    assert_eq!(untouched["comments"], json!([]));

    // The in-memory source: `b` moved by its configuration after it was read.
    let stale = store.version("memory:b");
    store.retitle_in_memory("b", "Bravo, moved");
    let batch = json!({"schema_version": 1, "comments": [
        {"item": "memory:a", "body": "On a."},
        {"item": "memory:b", "body": "On b.", "if_version": stale},
        {"item": "memory:c", "body": "On c.", "if_version": store.version("memory:c")},
        {"item": "memory:d", "body": "On d."},
    ]})
    .to_string();
    let added = store.exited(
        &["task", "comment", "add-many", "memory", "--input", "-"],
        Some(&batch),
        4,
    );
    assert_eq!(
        outcomes(&added),
        ["written", "version-moved", "written", "written"]
    );
    assert_eq!(
        added["results"][1]["current_version"],
        json!(store.version("memory:b"))
    );
    for at in [0, 2, 3] {
        assert!(
            added["results"][at]["comment"]["id"].is_string(),
            "{added:#}"
        );
    }
}

/// An update of `item` setting content, metadata, status and a priority guarded by `high` and
/// `none`.
fn every_field(item: &str, if_version: Option<&str>) -> Value {
    let mut update = json!({
        "item": item, "body": "Rewritten.\n", "metadata": {"myapp.colour": "black"},
        "status": "done", "priority": "low", "when_priority": ["high", "none"],
    });
    if let Some(version) = if_version {
        update["if_version"] = json!(version);
    }
    update
}

#[test]
fn task_update_many_writes_every_field_of_an_update_or_none_of_them() {
    let mut store = Store::new();
    let changed = json!(["Rewritten.", "black", "done", "low"]);

    // The folder: `b` moved by a second process after it was read, `c` at an unlisted priority.
    let read_a = store.version("plan:a");
    let stale = store.version("plan:b");
    store.other_writer("plan:b", "Bravo, moved");
    let left_b = store.shown("plan:b");
    let left_c = store.shown("plan:c");
    let batch = json!({"schema_version": 1, "updates": [
        every_field("plan:a", Some(&read_a)),
        every_field("plan:b", Some(&stale)),
        every_field("plan:c", None),
        every_field("plan:d", None),
    ]});
    let file = store.file("updates.json", &batch);
    let updated = store.exited(&["task", "update-many", "plan", "--input", &file], None, 4);
    assert_eq!(
        outcomes(&updated),
        ["written", "version-moved", "held", "written"]
    );
    assert_eq!(
        updated["results"][1]["current_version"],
        json!(store.version("plan:b"))
    );
    for (at, native) in [(0, "a"), (3, "d")] {
        let shown = store.shown(&format!("plan:{native}"));
        assert_eq!(fields(&shown), changed, "{native}: every field landed");
        assert_eq!(updated["results"][at]["version"], shown["version"]);
    }
    assert_eq!(
        store.shown("plan:b"),
        left_b,
        "nothing of b, as the other writer left it"
    );
    assert_eq!(
        store.shown("plan:c"),
        left_c,
        "nothing of c, which the guard held"
    );

    // The in-memory source: `b` moved by its configuration; what was written is the answer's.
    let read_a = store.version("memory:a");
    let stale = store.version("memory:b");
    store.retitle_in_memory("b", "Bravo, moved");
    let batch = json!({"schema_version": 1, "updates": [
        every_field("memory:a", Some(&read_a)),
        every_field("memory:b", Some(&stale)),
        every_field("memory:c", None),
        every_field("memory:d", None),
    ]})
    .to_string();
    let updated = store.exited(
        &["task", "update-many", "memory", "--input", "-"],
        Some(&batch),
        4,
    );
    assert_eq!(
        outcomes(&updated),
        ["written", "version-moved", "held", "written"]
    );
    assert_eq!(
        updated["results"][1]["current_version"],
        json!(store.version("memory:b"))
    );
    for at in [0, 3] {
        let version = updated["results"][at]["version"]
            .as_str()
            .expect("a version");
        assert!(version.starts_with("mem1|"), "{updated:#}");
    }
    assert_ne!(updated["results"][0]["version"], json!(read_a));
}

#[test]
fn a_batch_naming_a_task_that_does_not_exist_writes_every_other_item() {
    let store = Store::new();
    for source in ["plan", "memory"] {
        let item = |native: &str| format!("{source}:{native}");
        let comments = json!({"schema_version": 1, "comments": [
            {"item": item("a"), "body": "One."},
            {"item": item("nowhere"), "body": "Lost."},
            {"item": item("c"), "body": "Three."},
            {"item": item("d"), "body": "Four."},
        ]})
        .to_string();
        let added = store.exited(
            &["task", "comment", "add-many", source, "--input", "-"],
            Some(&comments),
            4,
        );
        assert_eq!(outcomes(&added), ["written", "error", "written", "written"]);
        let reason = added["results"][1]["reason"].as_str().expect("a reason");
        assert!(reason.contains(&item("nowhere")), "{source}: {reason}");

        let updates = json!({"schema_version": 1, "updates": [
            {"item": item("a"), "title": "One, updated"},
            {"item": item("c"), "title": "Three, updated"},
            {"item": item("nowhere"), "title": "Lost"},
            {"item": item("d"), "title": "Four, updated"},
        ]})
        .to_string();
        let updated = store.exited(
            &["task", "update-many", source, "--input", "-"],
            Some(&updates),
            4,
        );
        assert_eq!(
            outcomes(&updated),
            ["written", "written", "error", "written"]
        );
        let reason = updated["results"][2]["reason"].as_str().expect("a reason");
        assert!(reason.contains(&item("nowhere")), "{source}: {reason}");
    }
    for (native, title) in [
        ("a", "One, updated"),
        ("c", "Three, updated"),
        ("d", "Four, updated"),
    ] {
        assert_eq!(store.shown(&format!("plan:{native}"))["title"], title);
        let listed = store.exited(
            &["task", "comment", "list", &format!("plan:{native}")],
            None,
            0,
        );
        assert_eq!(listed["comments"].as_array().map(Vec::len), Some(1));
    }
}

#[test]
fn task_update_when_priority_writes_content_metadata_and_priority_only_while_it_holds() {
    let store = Store::new();
    let body = store.sandbox.project().join("guarded.md");
    std::fs::write(&body, "Guarded.\n").expect("a body file");
    let body = body.to_str().expect("a path");
    for source in ["plan", "memory"] {
        let update = |native: &str, code| {
            store.exited(
                &[
                    "task",
                    "update",
                    &format!("{source}:{native}"),
                    "--body-file",
                    body,
                    "--metadata",
                    "myapp.colour=\"black\"",
                    "--priority",
                    "urgent",
                    "--when-priority",
                    "high",
                    "--when-priority",
                    "none",
                ],
                None,
                code,
            )
        };
        let before = store.shown(&format!("{source}:c"));
        let held = update("c", 0);
        assert_eq!(held["guard"]["outcome"], "held", "{source}: {held:#}");
        assert_eq!(held["guard"]["priority_condition"]["held"], "low");
        assert_eq!(held["written"], json!([]));
        assert_eq!(
            fields(&held["task"]),
            fields(&before),
            "{source}: nothing of c"
        );
        if source == "plan" {
            assert_eq!(store.shown("plan:c"), before, "the folder's c is as it was");
        }

        let written = update("a", 0);
        assert_eq!(
            written["guard"]["outcome"], "written",
            "{source}: {written:#}"
        );
        let mut fields_written: Vec<&str> = written["written"]
            .as_array()
            .expect("written")
            .iter()
            .map(|field| field.as_str().expect("a field"))
            .collect();
        fields_written.sort_unstable();
        assert_eq!(
            fields_written,
            ["content", "metadata", "priority"],
            "{source}"
        );
        let landed = json!(["Guarded.", "black", "todo", "urgent"]);
        assert_eq!(fields(&written["task"]), landed, "{source}: {written:#}");
        if source == "plan" {
            assert_eq!(fields(&store.shown("plan:a")), landed);
        }
    }
}

impl Store {
    /// `onetaskgraph <arguments>` in its human rendering, exiting `code`: its standard output
    /// and standard error.
    fn human(&self, arguments: &[&str], input: &str, code: i32) -> (String, String) {
        let mut command = self.sandbox.command();
        command.args(arguments);
        command.write_stdin(input);
        let output = command.assert().get_output().clone();
        assert_eq!(
            output.status.code(),
            Some(code),
            "`onetaskgraph {}`\nstdout:\n{}\nstderr:\n{}",
            arguments.join(" "),
            stdout(&output),
            stderr(&output)
        );
        (stdout(&output), stderr(&output))
    }

    /// Every file of the folder, its bytes, so a refusal is proven to have written nothing.
    fn folder(&self) -> Vec<(String, Vec<u8>)> {
        let mut files: Vec<(String, Vec<u8>)> = std::fs::read_dir(self.plan.join("tasks"))
            .expect("the task folder")
            .map(|entry| {
                let path = entry.expect("an entry").path();
                (
                    path.display().to_string(),
                    std::fs::read(&path).expect("a task file"),
                )
            })
            .collect();
        files.sort();
        files
    }
}

#[test]
fn a_malformed_batch_document_is_refused_whole_naming_what_is_wrong_and_writing_nothing() {
    let store = Store::new();
    let before = store.folder();
    let find = ["task", "find", "plan", "--queries", "-"];
    let comments = ["task", "comment", "add-many", "plan", "--input", "-"];
    let updates = ["task", "update-many", "plan", "--input", "-"];
    let refusals: [(&[&str], String, &[&str]); 9] = [
        (
            &find,
            "{not json".to_owned(),
            &["is not a FindRequest", "next:"],
        ),
        (
            &find,
            json!({"schema_version": 2, "queries": []}).to_string(),
            &["schema_version is 2", "reads 1", "next:"],
        ),
        (
            &find,
            json!({"schema_version": 1, "queries": [{"key": "k", "colour": "red"}]}).to_string(),
            &["unknown field `colour`", "next:"],
        ),
        (
            &find,
            json!({"schema_version": 1, "queries": [
                {"key": "k", "text": "batch"}, {"key": "k", "text": "Delta"}]})
            .to_string(),
            &["the key \"k\" names two queries", "next:"],
        ),
        (
            &find,
            json!({"schema_version": 1, "queries": [
                {"key": "wide", "status": ["todo"]}]})
            .to_string(),
            &["query \"wide\" is narrowed by none", "next:"],
        ),
        (
            &find,
            json!({"schema_version": 1, "queries": [
                {"key": "deep", "metadata": {"myapp.colour//shade": "red"}}]})
            .to_string(),
            &["query \"deep\"", "a metadata location is", "next:"],
        ),
        (
            &comments,
            json!({"schema_version": 3, "comments": [{"item": "plan:a", "body": "One."}]})
                .to_string(),
            &["schema_version is 3", "next:"],
        ),
        (
            &updates,
            json!({"schema_version": 1, "updates": [{"item": "plan:a", "colour": "red"}]})
                .to_string(),
            &["unknown field `colour`", "next:"],
        ),
        (
            &updates,
            json!({"updates": [{"item": "plan:a", "title": "Lost"}]}).to_string(),
            &["schema_version", "next:"],
        ),
    ];
    for (arguments, input, said) in refusals {
        let (out, err) = store.human(arguments, &input, 1);
        assert!(out.is_empty(), "{input}: {out}");
        for words in said {
            assert!(err.contains(words), "{input}: {words:?} not in {err}");
        }
    }
    let missing = store.sandbox.project().join("missing.json");
    let missing = missing.to_str().expect("a path");
    let (_, err) = store.human(&["task", "find", "plan", "--queries", missing], "", 1);
    assert!(err.contains("could not read it"), "{err}");
    assert!(err.contains("next: name a readable file"), "{err}");
    assert_eq!(store.folder(), before, "no refusal wrote a byte");
}

#[test]
fn each_batched_verb_renders_one_line_per_item_and_refuses_a_bad_item_alone() {
    let store = Store::new();
    let (found, _) = store.human(
        &["task", "find", "plan", "--queries", "-"],
        &json!({"schema_version": 1, "queries": [
            {"key": "open", "text": "batch", "status": ["in-progress"]},
            {"key": "nothing", "text": "no task holds this"},
        ]})
        .to_string(),
        0,
    );
    assert_eq!(
        found,
        "open: 1 task(s)\n  plan:c  Charlie batch\nnothing: 0 task(s)\n"
    );

    let (added, _) = store.human(
        &["task", "comment", "add-many", "plan", "--input", "-"],
        &json!({"schema_version": 1, "comments": [
            {"item": "plan:a", "body": ""},
            {"item": "plan:a", "body": "Kept."},
        ]})
        .to_string(),
        4,
    );
    let lines: Vec<&str> = added.lines().collect();
    assert_eq!(lines.len(), 2, "{added}");
    assert!(
        lines[0].starts_with("plan:a  error  ") && lines[0].contains("cannot be empty"),
        "{added}"
    );
    assert!(lines[1].starts_with("plan:a  written  comment "), "{added}");
    let listed = store.exited(&["task", "comment", "list", "plan:a"], None, 0);
    let bodies: Vec<&str> = listed["comments"]
        .as_array()
        .expect("comments")
        .iter()
        .map(|comment| comment["body"].as_str().expect("a body").trim_end())
        .collect();
    assert_eq!(bodies, ["Kept."], "the empty comment was never sent");

    let (updated, _) = store.human(
        &["task", "update-many", "plan", "--input", "-"],
        &json!({"schema_version": 1, "updates": [
            {"item": "plan:a"},
            {"item": "plan:a", "metadata": {"onetaskgraph.origin": "elsewhere:X"}},
            {"item": "plan:a", "title": "Alpha, renamed"},
        ]})
        .to_string(),
        4,
    );
    let lines: Vec<&str> = updated.lines().collect();
    assert_eq!(lines.len(), 3, "{updated}");
    assert!(
        lines[0].starts_with("plan:a  error  ") && lines[0].contains("names no field"),
        "{updated}"
    );
    assert!(
        lines[1].starts_with("plan:a  error  ") && lines[1].contains("`onetaskgraph.` namespace"),
        "{updated}"
    );
    let version = store.version("plan:a");
    assert_eq!(lines[2], format!("plan:a  written  version {version}"));
    let task = store.shown("plan:a");
    assert_eq!(task["title"], "Alpha, renamed");
    assert!(
        task["metadata"].get("onetaskgraph.origin").is_none(),
        "the reserved key was never written: {task:#}"
    );
}
