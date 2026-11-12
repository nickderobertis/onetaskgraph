//! A task's assignees on a folder of Markdown and in memory, driven the way a user drives them.
//!
//! A source opts in with `config.assignees: true`: its items then carry every person they hold,
//! and every write names one person or none, replacing what the item held. The realistic caller
//! routes work to a person from configuration while a person's own assignment always stands, so
//! the journeys here hold both halves: what an opted-in source reads and writes, and that a
//! source left as it was reads, writes and refuses exactly as before. Each read of a folder is a
//! fresh process; an in-memory source's work lives only as long as one invocation, so it is
//! driven for what that invocation answers.

use std::path::PathBuf;
use std::process::Output;

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};

/// Folders `work`, `copies` and `plain` — the last not opted in — and in-memory sources
/// `memory`, opted in and seeded with assigned tasks, and `bare`, not opted in.
struct Store {
    sandbox: Sandbox,
    work: PathBuf,
    copies: PathBuf,
    plain: PathBuf,
}

impl Store {
    fn new() -> Self {
        let sandbox = Sandbox::new();
        let folder = |name: &str| {
            let root = sandbox.subdirectory(name);
            std::fs::create_dir_all(root.join("tasks")).expect("a task folder");
            root
        };
        let (work, copies, plain) = (folder("work"), folder("copies"), folder("plain"));
        let status = json!({"category": "todo", "name": "todo"});
        let document = json!({
            "sources": {
                "work": {"plugin": "local-md", "config": {"root": work, "assignees": true}},
                "copies": {"plugin": "local-md", "config": {"root": copies, "assignees": true}},
                "plain": {"plugin": "local-md", "config": {"root": plain}},
                "memory": {"plugin": "in-memory", "config": {
                    "assignees": true,
                    "tasks": [
                        {"id": "m1", "title": "Memory one", "labels": [], "status": status,
                         "assignees": ["ana"]},
                        {"id": "m2", "title": "Memory pair", "labels": [], "status": status,
                         "assignees": ["ana", "ben"]},
                        {"id": "m3", "title": "Memory nobody", "labels": [], "status": status},
                    ],
                }},
                "bare": {"plugin": "in-memory", "config": {
                    "tasks": [{"id": "b1", "title": "Bare", "labels": [], "status": status}],
                }},
            },
        });
        sandbox.project_document(&serde_json::to_string_pretty(&document).expect("JSON"));
        Self {
            sandbox,
            work,
            copies,
            plain,
        }
    }

    /// Write one task file of `root`, its front matter carrying `extra` lines.
    fn task(&self, root: &std::path::Path, native: &str, title: &str, extra: &str) {
        std::fs::write(
            root.join("tasks").join(format!("{native}.md")),
            format!("---\ntitle: {title}\nstatus: todo\n{extra}---\nThe {native} body.\n"),
        )
        .expect("a task file");
    }

    fn file(&self, root: &std::path::Path, native: &str) -> String {
        std::fs::read_to_string(root.join("tasks").join(format!("{native}.md"))).expect("the file")
    }

    fn files(root: &std::path::Path) -> Vec<PathBuf> {
        let mut files: Vec<PathBuf> = std::fs::read_dir(root.join("tasks"))
            .expect("the task folder")
            .map(|entry| entry.expect("an entry").path())
            .collect();
        files.sort();
        files
    }

    fn run(&self, arguments: &[&str], input: Option<&str>) -> Output {
        let mut command = self.sandbox.command();
        command.arg("--json").args(arguments);
        command.write_stdin(input.unwrap_or_default());
        command.assert().get_output().clone()
    }

    /// `onetaskgraph <arguments>` exiting `code`, as the JSON it printed.
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

    fn ok(&self, arguments: &[&str]) -> Value {
        self.exited(arguments, None, 0)
    }

    /// The task `id` as a fresh `task show` reads it.
    fn shown(&self, id: &str) -> Value {
        self.ok(&["task", "show", id])["items"][0]["item"].clone()
    }

    fn update_many(&self, source: &str, updates: &Value, code: i32) -> Value {
        let batch = json!({"schema_version": 1, "updates": updates}).to_string();
        self.exited(
            &["task", "update-many", source, "--input", "-"],
            Some(&batch),
            code,
        )
    }
}

/// The people `task` holds, as its JSON carries them.
fn people(task: &Value) -> Value {
    task["assignees"].clone()
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

/// The one outcome of a copy for the item copied from `source`.
fn outcome<'a>(report: &'a Value, source: &str) -> &'a Value {
    report["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|item| item["source"] == source)
        .unwrap_or_else(|| panic!("no outcome for {source}: {report}"))
}

#[test]
fn a_task_created_with_one_assignee_is_read_back_by_show_list_and_find() {
    let store = Store::new();
    let body = store.sandbox.project().join("body.md");
    std::fs::write(&body, "Route me.\n").expect("a body");
    let created = store.ok(&[
        "task",
        "create",
        "work",
        "--project",
        "p",
        "--title",
        "Routed work",
        "--body-file",
        body.to_str().expect("a path"),
        "--assignee",
        "ana",
    ]);
    let id = created["task"]["id"]
        .as_str()
        .map_or_else(|| created["id"].to_string(), str::to_owned);
    assert!(
        store
            .file(&store.work, "routed-work")
            .contains("assignees:\n- ana\n"),
        "{}",
        store.file(&store.work, "routed-work")
    );
    assert_eq!(
        people(&store.shown("work:routed-work")),
        json!(["ana"]),
        "{id}"
    );

    let listed = store.ok(&["task", "list", "--source", "work"]);
    assert_eq!(people(&listed["items"][0]["item"]), json!(["ana"]));

    let queries = json!({"schema_version": 1,
                         "queries": [{"key": "routed", "text": "Routed", "in": "title"}]})
    .to_string();
    let found = store.exited(
        &["task", "find", "work", "--queries", "-"],
        Some(&queries),
        0,
    );
    assert_eq!(
        people(&found["answers"][0]["items"][0]["item"]),
        json!(["ana"])
    );

    // In memory, each seeded task reads as it was configured: one, both, nobody.
    for (id, held) in [
        ("memory:m1", json!(["ana"])),
        ("memory:m2", json!(["ana", "ben"])),
        ("memory:m3", json!([])),
    ] {
        assert_eq!(people(&store.shown(id)), held, "{id}");
    }
    let listed = store.ok(&["task", "list", "--source", "memory"]);
    let held: Vec<Value> = listed["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|item| people(&item["item"]))
        .collect();
    assert_eq!(held, [json!(["ana"]), json!(["ana", "ben"]), json!([])]);
    let queries = json!({"schema_version": 1,
                         "queries": [{"key": "pair", "text": "pair", "in": "title"}]})
    .to_string();
    let found = store.exited(
        &["task", "find", "memory", "--queries", "-"],
        Some(&queries),
        0,
    );
    assert_eq!(
        people(&found["answers"][0]["items"][0]["item"]),
        json!(["ana", "ben"])
    );
}

#[test]
fn naming_assignee_twice_is_a_usage_error_and_writes_nothing() {
    let store = Store::new();
    let output = store.run(
        &[
            "task",
            "create",
            "work",
            "--project",
            "p",
            "--title",
            "Twice",
            "--assignee",
            "ana",
            "--assignee",
            "ben",
        ],
        None,
    );
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("--assignee"),
        "{}",
        stderr(&output)
    );
    assert!(Store::files(&store.work).is_empty());
}

#[test]
fn assignee_set_replaces_the_assignee_and_naming_none_unassigns() {
    let store = Store::new();
    store.task(&store.work, "w1", "One", "assignees: [ana]\n");
    let set = store.ok(&["task", "assignee", "set", "work:w1", "ben"]);
    assert_eq!(set["assignees"], json!(["ben"]));
    assert_eq!(people(&store.shown("work:w1")), json!(["ben"]));
    let set = store.ok(&["task", "assignee", "set", "work:w1"]);
    assert_eq!(set["assignees"], json!([]));
    assert_eq!(people(&store.shown("work:w1")), json!([]));
    assert!(!store.file(&store.work, "w1").contains("assignees"));

    // In memory, the one invocation answers what it holds after the write.
    let set = store.ok(&["task", "assignee", "set", "memory:m2", "cara"]);
    assert_eq!(set["assignees"], json!(["cara"]));
    let set = store.ok(&["task", "assignee", "set", "memory:m1"]);
    assert_eq!(set["assignees"], json!([]));
}

#[test]
fn a_hand_edited_file_holding_two_reads_both_and_a_held_update_is_decided_on_the_whole_set() {
    let store = Store::new();
    store.task(&store.work, "w1", "Pair", "assignees: [ana, ben]\n");
    store.task(&store.work, "w2", "Other pair", "assignees: [ana, ben]\n");
    assert_eq!(people(&store.shown("work:w1")), json!(["ana", "ben"]));

    let answer = store.update_many(
        "work",
        &json!([
            {"item": "work:w1", "assignee": "cara", "when_assignees": ["ben", "ana"]},
            {"item": "work:w2", "assignee": "cara", "when_assignees": ["ana"]},
        ]),
        0,
    );
    assert_eq!(outcomes(&answer), ["written", "held"], "{answer}");
    // Written: the one person named replaces both.
    assert_eq!(people(&store.shown("work:w1")), json!(["cara"]));
    // Held: a guard naming only one of the two people it holds writes nothing.
    assert_eq!(people(&store.shown("work:w2")), json!(["ana", "ben"]));
    assert!(
        store
            .file(&store.work, "w2")
            .contains("assignees: [ana, ben]\n")
    );

    // `null` unassigns, and `[]` guards an unassigned task.
    let answer = store.update_many(
        "work",
        &json!([{"item": "work:w1", "assignee": null, "when_assignees": ["cara"]}]),
        0,
    );
    assert_eq!(outcomes(&answer), ["written"], "{answer}");
    let answer = store.update_many(
        "work",
        &json!([
            {"item": "work:w1", "assignee": "dan", "when_assignees": []},
            {"item": "work:w2", "assignee": "dan", "when_assignees": []},
        ]),
        0,
    );
    assert_eq!(outcomes(&answer), ["written", "held"], "{answer}");
    assert_eq!(people(&store.shown("work:w1")), json!(["dan"]));

    // In memory, the same guard on the whole set.
    let answer = store.update_many(
        "memory",
        &json!([
            {"item": "memory:m2", "assignee": "eve", "when_assignees": ["ana"]},
            {"item": "memory:m1", "assignee": "eve", "when_assignees": ["ana"]},
        ]),
        0,
    );
    assert_eq!(outcomes(&answer), ["held", "written"], "{answer}");
}

#[test]
fn a_copy_carries_the_assignee_refuses_an_item_holding_two_and_guards_a_recopy() {
    let store = Store::new();
    store.task(&store.work, "w1", "Single", "assignees: [ana]\n");
    store.task(&store.work, "w2", "Pair", "assignees: [ana, ben]\n");
    let report = store.exited(
        &["task", "copy", "work:w1", "work:w2", "--to", "copies"],
        None,
        4,
    );
    let landed = outcome(&report, "work:w1");
    assert_eq!(landed["action"], "created", "{report}");
    let copied = landed["destination"]
        .as_str()
        .expect("a destination")
        .to_owned();
    assert_eq!(people(&store.shown(&copied)), json!(["ana"]));
    let refused = outcome(&report, "work:w2");
    assert_eq!(refused["action"], "failed", "{report}");
    let message = refused.to_string();
    assert!(
        message.contains("work:w2") && message.contains("2 assignees"),
        "{message}"
    );
    assert_eq!(Store::files(&store.copies).len(), 1);

    // A person assigns the copy to somebody else: a plain re-copy keeps their assignment.
    store.ok(&["task", "assignee", "set", &copied, "dan"]);
    store.task(&store.work, "w1", "Single, renamed", "assignees: [ana]\n");
    let report = store.ok(&["task", "copy", "work:w1", "--to", "copies"]);
    let item = outcome(&report, "work:w1");
    assert_eq!(item["action"], "updated", "{report}");
    assert!(item.get("assignee_condition").is_none(), "{report}");
    assert_eq!(people(&store.shown(&copied)), json!(["dan"]));

    // Naming what it holds, the source's assignee is written over it.
    let report = store.ok(&[
        "task",
        "copy",
        "work:w1",
        "--to",
        "copies",
        "--when-destination-assignees",
        "dan",
    ]);
    let item = outcome(&report, "work:w1");
    assert_eq!(item["action"], "updated", "{report}");
    assert_eq!(
        item["assignee_condition"],
        json!({"held": ["dan"], "met": true})
    );
    assert_eq!(people(&store.shown(&copied)), json!(["ana"]));

    // Naming another set, it keeps its own.
    store.ok(&["task", "assignee", "set", &copied, "eve"]);
    let report = store.ok(&[
        "task",
        "copy",
        "work:w1",
        "--to",
        "copies",
        "--when-destination-assignees",
        "dan",
    ]);
    let item = outcome(&report, "work:w1");
    assert_eq!(
        item["assignee_condition"],
        json!({"held": ["eve"], "met": false})
    );
    assert_eq!(people(&store.shown(&copied)), json!(["eve"]));

    // `none` alone names an unassigned task, and beside a person is refused before anything.
    let output = store.run(
        &[
            "task",
            "copy",
            "work:w1",
            "--to",
            "copies",
            "--when-destination-assignees",
            "none",
            "--when-destination-assignees",
            "eve",
        ],
        None,
    );
    assert_ne!(output.status.code(), Some(0), "{}", stdout(&output));
    assert_eq!(people(&store.shown(&copied)), json!(["eve"]));
}

#[test]
fn a_held_assignee_set_whose_version_moved_answers_version_moved() {
    let store = Store::new();
    store.task(&store.work, "w1", "Held", "assignees: [ana]\n");
    let read = store.shown("work:w1")["version"]
        .as_str()
        .expect("a version")
        .to_owned();
    store.ok(&["task", "update", "work:w1", "--title", "Moved on"]);
    let output = store.run(
        &[
            "task",
            "assignee",
            "set",
            "work:w1",
            "ben",
            "--if-version",
            &read,
        ],
        None,
    );
    assert_ne!(output.status.code(), Some(0), "{}", stdout(&output));
    let failure: Value = serde_json::from_str(&stdout(&output)).expect("a failure document");
    assert_eq!(failure["failure"]["kind"], "version-moved", "{failure}");
    assert_eq!(people(&store.shown("work:w1")), json!(["ana"]));

    // At the version it holds, the same write lands.
    let now = store.shown("work:w1")["version"]
        .as_str()
        .expect("a version")
        .to_owned();
    store.ok(&[
        "task",
        "assignee",
        "set",
        "work:w1",
        "ben",
        "--if-version",
        &now,
    ]);
    assert_eq!(people(&store.shown("work:w1")), json!(["ben"]));
}

#[test]
fn an_assignee_only_change_moves_the_change_token_and_no_pin_reads_it_stale() {
    let store = Store::new();
    store.task(&store.work, "w1", "Tokened", "assignees: [ana]\n");
    let before = store.shown("work:w1")["version"]
        .as_str()
        .expect("a version")
        .to_owned();
    store.ok(&["task", "assignee", "set", "work:w1", "ben"]);
    let after = store.shown("work:w1")["version"]
        .as_str()
        .expect("a version")
        .to_owned();
    assert_ne!(before, after, "an assignee-only change moves the token");
    for pin in [&after, &before] {
        let shown =
            store.ok(&["task", "show", "work:w1", "--pin", pin])["items"][0]["item"].clone();
        assert_eq!(people(&shown), json!(["ben"]), "pinned to {pin}");
        assert_eq!(shown["version"], after.as_str(), "pinned to {pin}");
    }
}

#[test]
fn copies_across_a_source_without_the_setting_carry_no_assignee_and_are_not_refused_for_it() {
    let store = Store::new();
    // Out of an opted-in source holding an assigned item: written with no `assignees` key.
    let report = store.ok(&["task", "copy", "memory:m1", "--to", "plain"]);
    assert_eq!(
        outcome(&report, "memory:m1")["action"],
        "created",
        "{report}"
    );
    let files = Store::files(&store.plain);
    assert_eq!(files.len(), 1);
    let stored = std::fs::read_to_string(&files[0]).expect("the copy");
    assert!(!stored.contains("assignees"), "{stored}");
    let native = files[0]
        .file_stem()
        .and_then(|stem| stem.to_str())
        .expect("a stem")
        .to_owned();
    assert!(
        store
            .shown(&format!("plain:{native}"))
            .get("assignees")
            .is_none()
    );

    // Into an opted-in source from one without the setting: created unassigned...
    store.task(&store.plain, "p1", "Plain", "");
    let report = store.ok(&["task", "copy", "plain:p1", "--to", "copies"]);
    let copied = outcome(&report, "plain:p1")["destination"]
        .as_str()
        .expect("a destination")
        .to_owned();
    assert_eq!(people(&store.shown(&copied)), json!([]));
    // ...and an existing item's assignees left as they were.
    store.ok(&["task", "assignee", "set", &copied, "fay"]);
    store.task(&store.plain, "p1", "Plain, renamed", "");
    let report = store.ok(&["task", "copy", "plain:p1", "--to", "copies"]);
    assert_eq!(
        outcome(&report, "plain:p1")["action"],
        "updated",
        "{report}"
    );
    let shown = store.shown(&copied);
    assert_eq!(shown["title"], "Plain, renamed");
    assert_eq!(people(&shown), json!(["fay"]));
}

#[test]
fn a_source_without_the_setting_refuses_every_assignee_write_and_carries_no_key() {
    let store = Store::new();
    store.task(&store.plain, "p1", "Plain", "");
    let untouched = store.file(&store.plain, "p1");
    let refused = |arguments: &[&str], input: Option<&str>| {
        let output = store.run(arguments, input);
        assert_ne!(output.status.code(), Some(0), "{}", stdout(&output));
        let said = format!("{}{}", stdout(&output), stderr(&output));
        assert!(
            said.contains("no-assignees") && said.contains("assignees: true"),
            "`onetaskgraph {}` names the setting: {said}",
            arguments.join(" ")
        );
    };
    for source in ["plain", "bare"] {
        refused(
            &[
                "task",
                "create",
                source,
                "--project",
                "p",
                "--title",
                "New",
                "--assignee",
                "ana",
            ],
            None,
        );
    }
    refused(&["task", "assignee", "set", "plain:p1", "ana"], None);
    refused(&["task", "assignee", "set", "plain:p1"], None);
    refused(&["task", "assignee", "set", "bare:b1", "ana"], None);
    refused(
        &[
            "task",
            "copy",
            "memory:m1",
            "--to",
            "plain",
            "--when-destination-assignees",
            "ana",
        ],
        None,
    );
    for update in [
        json!({"item": "plain:p1", "assignee": "ana"}),
        json!({"item": "plain:p1", "assignee": null}),
        json!({"item": "plain:p1", "title": "Retitled", "when_assignees": []}),
    ] {
        let answer = store.update_many("plain", &json!([update]), 4);
        assert_eq!(outcomes(&answer), ["error"], "{answer}");
        let reason = answer["results"][0]["reason"].as_str().expect("a reason");
        assert!(
            reason.contains("plain") && reason.contains("assignees: true"),
            "{reason}"
        );
    }
    assert_eq!(store.file(&store.plain, "p1"), untouched);
    assert_eq!(Store::files(&store.plain).len(), 1);
    assert!(store.shown("plain:p1").get("assignees").is_none());
    assert!(store.shown("bare:b1").get("assignees").is_none());
    let listed = store.ok(&["task", "list", "--source", "bare"]);
    assert!(listed["items"][0]["item"].get("assignees").is_none());
}

#[test]
fn a_task_of_a_source_without_the_setting_is_stored_and_read_exactly_as_before_assignees_existed() {
    // The committed fixtures are the stored forms the release before this one wrote, recorded
    // with the relation journeys: a folder's file after a copy and an update, and an in-memory
    // task as `task show` reads it, its change token included. Neither source opts in here.
    let sandbox = Sandbox::new();
    let copies = sandbox.subdirectory("copies");
    let status = json!({"category": "todo", "name": "todo"});
    sandbox.project_document(
        &serde_json::to_string_pretty(&json!({"sources": {
            "copies": {"plugin": "local-md", "config": {"root": copies}},
            "memory": {"plugin": "in-memory", "config": {"tasks": [
                {"id": "m3", "title": "Memory other", "labels": [],
                 "content": "Unrelated.", "status": status},
            ]}},
        }}))
        .expect("JSON"),
    );
    let ok = |arguments: &[&str]| {
        let output = sandbox
            .command()
            .arg("--json")
            .args(arguments)
            .assert()
            .get_output()
            .clone();
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        serde_json::from_str::<Value>(&stdout(&output)).expect("JSON")
    };
    ok(&["task", "copy", "memory:m3", "--to", "copies"]);
    let files = Store::files(&copies);
    assert_eq!(files.len(), 1, "{files:?}");
    let native = files[0]
        .file_stem()
        .and_then(|stem| stem.to_str())
        .expect("a stem")
        .to_owned();
    ok(&[
        "task",
        "update",
        &format!("copies:{native}"),
        "--title",
        "Memory other, updated",
    ]);
    let stored = std::fs::read_to_string(&files[0]).expect("the copy");
    assert_eq!(
        stored.replace(&native, "<native>"),
        include_str!("fixtures/relations/unrelated-local-md.md"),
    );
    let shown = ok(&["task", "show", "memory:m3"])["items"][0]["item"].clone();
    assert_eq!(
        serde_json::to_string_pretty(&shown).expect("JSON") + "\n",
        include_str!("fixtures/relations/unrelated-in-memory.json"),
    );
}

#[test]
fn a_person_reading_a_terminal_is_told_every_assignee_and_nothing_for_a_source_without_them() {
    let store = Store::new();
    store.task(&store.work, "w1", "Pair", "assignees: [ana, ben]\n");
    store.task(&store.plain, "p1", "Plain", "assignees: [ana]\n");
    let text = |arguments: &[&str]| {
        let output = store
            .sandbox
            .command()
            .args(arguments)
            .assert()
            .get_output()
            .clone();
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        stdout(&output)
    };
    let shown = text(&["task", "show", "work:w1"]);
    assert!(
        shown
            .lines()
            .any(|line| line.starts_with("assignees:") && line.ends_with("ana, ben")),
        "{shown}"
    );
    // A folder that has not opted in prints the lines it always printed, whatever its file says.
    let shown = text(&["task", "show", "plain:p1"]);
    assert!(!shown.contains("assignees"), "{shown}");

    let set = text(&["task", "assignee", "set", "work:w1", "cara"]);
    assert!(
        set.lines()
            .any(|line| line.starts_with("assignees:") && line.ends_with("cara")),
        "{set}"
    );
    let set = text(&["task", "assignee", "set", "work:w1"]);
    assert!(
        set.lines()
            .any(|line| line.starts_with("assignees:") && line.ends_with("none")),
        "{set}"
    );
}

#[test]
fn an_in_memory_source_configured_with_assignees_it_has_not_opted_in_to_is_refused_at_load() {
    let sandbox = Sandbox::new();
    let status = json!({"category": "todo", "name": "todo"});
    sandbox.project_document(
        &json!({"sources": {"bare": {"plugin": "in-memory", "config": {"tasks": [
            {"id": "b1", "title": "Bare", "labels": [], "status": status, "assignees": ["ana"]},
        ]}}}})
        .to_string(),
    );
    let output = sandbox
        .command()
        .args(["task", "show", "bare:b1"])
        .assert()
        .get_output()
        .clone();
    assert_ne!(output.status.code(), Some(0), "{}", stdout(&output));
    let said = stderr(&output);
    assert!(
        said.contains("b1") && said.contains("assignees: true"),
        "names the task and the setting: {said}"
    );
}

#[test]
fn a_project_copy_holding_a_task_assigned_to_two_is_refused_whole_writing_nothing() {
    let store = Store::new();
    std::fs::create_dir_all(store.work.join("projects")).expect("a project folder");
    std::fs::write(
        store.work.join("projects").join("plan.md"),
        "---\ntitle: Plan\nstatus: todo\n---\nThe plan.\n",
    )
    .expect("a project file");
    store.task(
        &store.work,
        "w1",
        "Single",
        "project: plan\nassignees: [ana]\n",
    );
    store.task(
        &store.work,
        "w2",
        "Pair",
        "project: plan\nassignees: [ana, ben]\n",
    );
    let output = store.run(&["project", "copy", "work:plan", "--to", "copies"], None);
    assert_ne!(output.status.code(), Some(0), "{}", stdout(&output));
    let said = format!("{}{}", stdout(&output), stderr(&output));
    assert!(
        said.contains("work:w2") && said.contains("2 assignees"),
        "names the task and how many it holds: {said}"
    );
    assert!(
        Store::files(&store.copies).is_empty(),
        "no task was written"
    );
    assert!(
        !store.copies.join("projects").exists()
            || std::fs::read_dir(store.copies.join("projects"))
                .expect("the project folder")
                .next()
                .is_none(),
        "no project was written"
    );
}

#[test]
fn a_copy_guard_names_the_whole_set_in_any_order_and_none_alone_names_an_unassigned_task() {
    let store = Store::new();
    store.task(&store.work, "w1", "Guarded", "assignees: [ana]\n");
    let report = store.ok(&["task", "copy", "work:w1", "--to", "copies"]);
    let copied = outcome(&report, "work:w1")["destination"]
        .as_str()
        .expect("a destination")
        .to_owned();
    let native = copied.trim_start_matches("copies:").to_owned();
    let copy_guarded = |people: &[&str]| {
        let mut arguments = vec!["task", "copy", "work:w1", "--to", "copies"];
        for person in people {
            arguments.extend(["--when-destination-assignees", person]);
        }
        store.ok(&arguments)
    };
    // The counterpart holds two people a person gave it.
    let file = store.file(&store.copies, &native);
    let held = file.replace("assignees:\n- ana\n", "assignees: [ben, cara]\n");
    assert_ne!(held, file, "{file}");
    std::fs::write(
        store.copies.join("tasks").join(format!("{native}.md")),
        held,
    )
    .expect("the counterpart");

    // A subset of them is not the set it holds: it keeps both.
    let report = copy_guarded(&["ben"]);
    assert_eq!(
        outcome(&report, "work:w1")["assignee_condition"],
        json!({"held": ["ben", "cara"], "met": false})
    );
    assert_eq!(people(&store.shown(&copied)), json!(["ben", "cara"]));
    // `none` names an unassigned task, which it is not.
    let report = copy_guarded(&["none"]);
    assert_eq!(
        outcome(&report, "work:w1")["assignee_condition"]["met"],
        false
    );
    assert_eq!(people(&store.shown(&copied)), json!(["ben", "cara"]));
    // The whole set, in another order, is met: the source's one person replaces both.
    let report = copy_guarded(&["cara", "ben"]);
    assert_eq!(
        outcome(&report, "work:w1")["assignee_condition"],
        json!({"held": ["ben", "cara"], "met": true})
    );
    assert_eq!(people(&store.shown(&copied)), json!(["ana"]));

    // Unassigned, `none` alone is met and the source's person is written.
    store.ok(&["task", "assignee", "set", &copied]);
    store.ok(&["task", "assignee", "set", "work:w1", "dan"]);
    let report = copy_guarded(&["none"]);
    assert_eq!(
        outcome(&report, "work:w1")["assignee_condition"],
        json!({"held": [], "met": true})
    );
    assert_eq!(people(&store.shown(&copied)), json!(["dan"]));
}
