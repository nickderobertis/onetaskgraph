//! A task write held to the version its caller read — `--if-version` — through the binary, on
//! every write verb it holds and every source kind: a folder of Markdown, the loopback GitHub
//! board and the loopback Linear workspace, each hosted source with its local cache on and off,
//! and an in-memory source.
//!
//! Each verb is driven as a follow-up loop drives it: `task show` for the version, the write at
//! that version, which lands; then another writer — a second process writing the same task
//! through the store, unheld — moves the task, and the same write at the version read before
//! it is refused as `version-moved`, naming the version the task is now at, and the task reads
//! back exactly as the other writer left it.
//!
//! An in-memory source's work dies with its process, so no second process can move it between
//! two commands: its "other writer" is its configuration, edited between the read and the
//! write, which is the one way its task can be at another version by the time a command runs.
//! That two writes racing inside one process are serialized is proven as a library call, in
//! `crates/onetaskgraph-core/tests/held_writes.rs`.
//!
//! A folder of Markdown checks and writes as one step under the lock beside the task's file, so
//! two processes holding the same version and racing land exactly one write, which the last
//! journey here drives many times over.

use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{
    GitHubBoardFields, LinearWorkspace, dataset, document, github_projects_with_board,
    linear_default_mapping, linear_workspace,
};

/// Every destination a held write is driven against beside the folder the work starts in.
const DESTINATIONS: [&str; 3] = ["folder", "board", "linear"];

fn folder_mapping() -> Value {
    json!({"backlog": "backlog", "todo": "todo", "queued": "queued",
           "in-progress": "in-progress", "done": "done", "cancelled": "cancelled"})
}

struct Stores {
    sandbox: Sandbox,
    notes: PathBuf,
    /// Held so the loopback servers live as long as the journey.
    _board: GitHubBoardFields,
    _linear: LinearWorkspace,
}

impl Stores {
    /// A folder the work starts in, a second folder, a board and a Linear workspace — the two
    /// hosted ones with their local cache on when `cached` says so.
    fn new(cached: bool) -> Self {
        let sandbox = Sandbox::new();
        let (board_config, board) = github_projects_with_board(&sandbox);
        let (mut linear_config, linear) = linear_workspace(&sandbox, dataset());
        linear_config["status_mapping"] = linear_default_mapping();
        sandbox
            .secrets_file("GITHUB_PROJECTS_FIXTURE_TOKEN=test-token\nLINEAR_API_KEY=fixture-key\n");
        let notes = sandbox.subdirectory("notes");
        std::fs::create_dir_all(notes.join("tasks")).expect("the task folder");
        let folder = sandbox.subdirectory("folder");
        std::fs::create_dir_all(folder.join("tasks")).expect("the task folder");
        let cache = sandbox.subdirectory("cache");
        sandbox.project_document(&document(&json!({
            "board": {"plugin": "github-projects", "config": board_config,
                      "cache": {"enabled": cached, "dir": cache.join("board")}},
            "linear": {"plugin": "linear", "config": linear_config,
                       "cache": {"enabled": cached, "dir": cache.join("linear")}},
            "notes": {"plugin": "local-md",
                      "config": {"root": notes, "status_mapping": folder_mapping()}},
            "folder": {"plugin": "local-md",
                       "config": {"root": folder, "status_mapping": folder_mapping()}},
        })));
        Self {
            sandbox,
            notes,
            _board: board,
            _linear: linear,
        }
    }

    fn note(&self, id: &str, title: &str) {
        let path = self.notes.join("tasks").join(format!("{id}.md"));
        let kept = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| {
                let start = text.find("metadata:")?;
                let end = text[start..].find("\n---")? + start;
                Some(format!("{}\n", &text[start..end]))
            })
            .unwrap_or_default();
        std::fs::write(
            path,
            format!("---\ntitle: {title}\nstatus: todo\npriority: high\n{kept}---\nBody.\n"),
        )
        .expect("the task file");
    }

    fn file(&self, name: &str, text: &str) -> String {
        let path = self.sandbox.project().join(name);
        std::fs::write(&path, text).expect("a body file");
        path.to_str().expect("a path").to_owned()
    }

    fn run(&self, arguments: &[String]) -> Output {
        let mut full = vec!["--json".to_owned()];
        full.extend(arguments.iter().cloned());
        self.sandbox
            .command()
            .args(&full)
            .assert()
            .get_output()
            .clone()
    }

    fn exited(&self, arguments: &[String], code: i32) -> Value {
        let output = self.run(arguments);
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

    fn shown(&self, id: &str) -> Value {
        self.exited(&owned(&["task", "show", id]), 0)["items"][0]["item"].clone()
    }

    fn version(&self, id: &str) -> String {
        let task = self.shown(id);
        task["version"]
            .as_str()
            .unwrap_or_else(|| panic!("{id} is read with a version: {task:#}"))
            .to_owned()
    }
}

fn owned(arguments: &[&str]) -> Vec<String> {
    arguments
        .iter()
        .map(|argument| (*argument).to_owned())
        .collect()
}

/// The verbs a held write is driven through, by name.
const VERBS: [&str; 8] = [
    "status set",
    "priority set",
    "content set",
    "metadata set",
    "update",
    "comment add",
    "comment edit",
    "copy",
];

/// The arguments of `verb` writing its `round`-th value to `task` — `comment` the comment an
/// edit edits — held to `version`.
fn held_write(
    stores: &Stores,
    verb: &str,
    task: &str,
    destination: &str,
    comment: &str,
    round: usize,
    version: &str,
) -> Vec<String> {
    let mut arguments = match verb {
        "status set" => owned(&[
            "task",
            "status",
            "set",
            task,
            ["queued", "in-progress"][round % 2],
        ]),
        "priority set" => owned(&[
            "task",
            "priority",
            "set",
            task,
            ["low", "urgent"][round % 2],
        ]),
        "content set" => {
            let file = stores.file("content.md", &format!("Content, round {round}.\n"));
            owned(&["task", "content", "set", task, "--file", &file])
        }
        "metadata set" => owned(&[
            "task",
            "metadata",
            "set",
            task,
            "followup.round",
            &round.to_string(),
        ]),
        "update" => owned(&[
            "task",
            "update",
            task,
            "--title",
            &format!("Updated, round {round}"),
        ]),
        "comment add" => {
            let file = stores.file("comment.md", &format!("A follow-up, round {round}.\n"));
            owned(&["task", "comment", "add", task, "--body-file", &file])
        }
        "comment edit" => {
            let file = stores.file("comment.md", &format!("Edited, round {round}.\n"));
            owned(&[
                "task",
                "comment",
                "edit",
                task,
                comment,
                "--body-file",
                &file,
            ])
        }
        "copy" => {
            stores.note("A", &format!("Task A, round {round}"));
            owned(&["task", "copy", "notes:A", "--to", destination])
        }
        other => panic!("no verb {other}"),
    };
    arguments.extend(owned(&["--if-version", version]));
    arguments
}

/// Drive every verb held to a version against `destination`: a write at the version read lands,
/// and one at a version another writer moved the task past is refused and writes nothing.
fn every_verb_is_held(stores: &Stores, destination: &str, what: &str) {
    stores.note("A", "Task A");
    let task = stores.exited(&owned(&["task", "copy", "notes:A", "--to", destination]), 0)["items"]
        [0]["destination"]
        .as_str()
        .expect("the copy names what it created")
        .to_owned();
    let seed = stores.file("seed.md", "The comment an edit edits.\n");
    let added = stores.exited(
        &owned(&["task", "comment", "add", &task, "--body-file", &seed]),
        0,
    );
    let comment = added["id"]
        .as_str()
        .unwrap_or_else(|| panic!("{what}: the comment's id: {added:#}"))
        .to_owned();
    for (index, verb) in VERBS.iter().enumerate() {
        let who = format!("{what}: {verb}");
        let read = stores.version(&task);
        let landed = held_write(stores, verb, &task, destination, &comment, 2 * index, &read);
        let answer = stores.exited(&landed, 0);
        let written = stores.version(&task);
        // A Linear task's token is its `updatedAt` and its facets, which a comment moves
        // neither of; every other write here moves the token it is held to.
        if !(destination == "linear" && verb.starts_with("comment")) {
            assert_ne!(
                written, read,
                "{who}: the write landed and moved the version: {answer:#}"
            );
        }

        // Another writer, unheld, moves the task after this caller's read of it.
        let other = format!("Moved by another writer before {verb}");
        stores.exited(&owned(&["task", "update", &task, "--title", &other]), 0);
        let moved = stores.version(&task);
        assert_ne!(moved, written, "{who}: the other writer moved the version");
        let before = stores.shown(&task);

        let stale = held_write(
            stores,
            verb,
            &task,
            destination,
            &comment,
            2 * index + 1,
            &written,
        );
        let refused = stores.exited(&stale, 1);
        assert_eq!(
            refused["failure"]["kind"], "version-moved",
            "{who}: {refused:#}"
        );
        assert_eq!(
            refused["held"]["version"], moved,
            "{who}: the refusal names the version the task is at: {refused:#}"
        );
        let message = refused["failure"]["message"].as_str().unwrap_or_default();
        assert!(
            message.contains(&moved) && message.contains(&written),
            "{who}: {message}"
        );
        let after = stores.shown(&task);
        assert_eq!(
            after, before,
            "{who}: the refused write left the task as the other writer left it"
        );
        assert_eq!(after["title"], other, "{who}");
    }
}

#[test]
fn every_task_write_verb_held_to_a_version_lands_at_it_and_is_refused_once_it_moved() {
    for cached in [false, true] {
        for destination in DESTINATIONS {
            let stores = Stores::new(cached);
            if destination != "folder" {
                // The hosted source really keeps its cache when this run says it does: a read of
                // one reports how it used it.
                stores.note("Z", "Probe");
                let copied =
                    stores.exited(&owned(&["task", "copy", "notes:Z", "--to", destination]), 0);
                let probe = copied["items"][0]["destination"]
                    .as_str()
                    .expect("an id")
                    .to_owned();
                let shown = stores.exited(&owned(&["task", "show", &probe]), 0);
                assert_eq!(
                    shown["cache"].is_object(),
                    cached,
                    "{destination}: {shown:#}"
                );
            }
            every_verb_is_held(
                &stores,
                destination,
                &format!("{destination}, cache {cached}"),
            );
        }
    }
}

#[test]
fn a_held_write_on_a_task_nothing_holds_is_refused_as_any_write_of_it_is() {
    let stores = Stores::new(false);
    stores.note("A", "Task A");
    let failure = stores.exited(
        &owned(&[
            "task",
            "priority",
            "set",
            "notes:no-such-task",
            "low",
            "--if-version",
            "md1|whatever",
        ]),
        1,
    );
    assert_ne!(failure["failure"]["kind"], "version-moved", "{failure:#}");
}

#[test]
fn a_held_copy_names_one_task() {
    let stores = Stores::new(false);
    stores.note("A", "Task A");
    stores.note("B", "Task B");
    let failure = stores.exited(
        &owned(&[
            "task",
            "copy",
            "notes:A",
            "notes:B",
            "--to",
            "folder",
            "--if-version",
            "md1|whatever",
        ]),
        1,
    );
    assert_eq!(
        failure["failure"]["kind"], "if-version-one-task",
        "{failure:#}"
    );
}

/// An in-memory source holding one task with one comment, `title` its title — the copy of
/// `folder:A`, which a copy of that task finds by its origin — beside the folder.
fn memory(sandbox: &Sandbox, title: &str) {
    sandbox.project_document(&document(&json!({
        "memory": {"plugin": "in-memory", "config": {
            "capabilities": {"comments": "native", "priority": "native"},
            "tasks": [{"id": "T-1", "title": title, "content": "Body.",
                       "status": {"category": "todo", "name": "todo"}, "priority": "high",
                       "labels": [], "metadata": {"onetaskgraph.origin": "folder:A"}}],
            "comments": [{"task": "T-1", "comment": {"id": "C-1", "author": "ada",
                "created_at": "2026-09-13T15:11:07Z", "updated_at": "2026-09-13T15:11:07Z",
                "body": "Seeded.\n", "url": null}}],
        }},
        "folder": {"plugin": "local-md", "config": {
            "root": sandbox.subdirectory("folder"), "status_mapping": folder_mapping()}},
    })));
}

#[test]
fn an_in_memory_task_held_to_a_version_lands_at_it_and_is_refused_at_another() {
    let sandbox = Sandbox::new();
    let folder = sandbox.subdirectory("folder");
    std::fs::create_dir_all(folder.join("tasks")).expect("the task folder");
    std::fs::write(
        folder.join("tasks").join("A.md"),
        "---\ntitle: Task A\nstatus: todo\n---\nBody.\n",
    )
    .expect("a task");
    let body = sandbox.project().join("body.md");
    std::fs::write(&body, "Written.\n").expect("a body");
    let body = body.to_str().expect("a path").to_owned();
    let run = |arguments: &[&str], code: i32| -> Value {
        let mut full = vec!["--json"];
        full.extend(arguments);
        let output = sandbox.command().args(&full).assert().get_output().clone();
        assert_eq!(
            output.status.code(),
            Some(code),
            "`onetaskgraph {}`\n{}\n{}",
            full.join(" "),
            stdout(&output),
            stderr(&output)
        );
        serde_json::from_str(&stdout(&output)).expect("JSON")
    };
    let verbs: [&[&str]; 8] = [
        &["task", "status", "set", "memory:T-1", "queued"],
        &["task", "priority", "set", "memory:T-1", "low"],
        &["task", "content", "set", "memory:T-1", "--file", &body],
        &[
            "task",
            "metadata",
            "set",
            "memory:T-1",
            "followup.round",
            "1",
        ],
        &["task", "update", "memory:T-1", "--title", "Updated"],
        &["task", "comment", "add", "memory:T-1", "--body-file", &body],
        &[
            "task",
            "comment",
            "edit",
            "memory:T-1",
            "C-1",
            "--body-file",
            &body,
        ],
        &["task", "copy", "folder:A", "--to", "memory"],
    ];
    for verb in verbs {
        let who = verb.join(" ");
        memory(&sandbox, "As configured");
        let read = run(&["task", "show", "memory:T-1"], 0)["items"][0]["item"]["version"]
            .as_str()
            .expect("an in-memory task is read with a version")
            .to_owned();
        assert!(read.starts_with("mem1|"), "{read}");
        let mut held = verb.to_vec();
        held.extend(["--if-version", &read]);
        run(&held, 0);

        // Another writer: the configuration this source serves, changed after the read.
        memory(&sandbox, "Changed since the read");
        let moved = run(&["task", "show", "memory:T-1"], 0)["items"][0]["item"]["version"]
            .as_str()
            .expect("a version")
            .to_owned();
        assert_ne!(moved, read, "{who}");
        let refused = run(&held, 1);
        assert_eq!(
            refused["failure"]["kind"], "version-moved",
            "{who}: {refused:#}"
        );
        assert_eq!(refused["held"]["version"], moved, "{who}: {refused:#}");
    }
}

/// Two processes, each writing the folder's task held to the same version, started together.
fn race(sandbox: &Sandbox, version: &str, round: usize) -> [(Option<i32>, String); 2] {
    let spawn = |priority: &str| {
        sandbox
            .subprocess(onetaskgraph_e2e_support::binary())
            .args([
                "--json",
                "task",
                "priority",
                "set",
                "folder:T",
                priority,
                "--if-version",
                version,
            ])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("the binary starts")
    };
    let (first, second) = if round.is_multiple_of(2) {
        (spawn("low"), spawn("urgent"))
    } else {
        (spawn("urgent"), spawn("low"))
    };
    [first, second].map(|child| {
        let output = child.wait_with_output().expect("the binary finishes");
        (output.status.code(), stdout(&output))
    })
}

fn folder_version(sandbox: &Sandbox) -> String {
    let output = sandbox
        .command()
        .args(["--json", "task", "show", "folder:T"])
        .assert()
        .success()
        .get_output()
        .clone();
    let shown: Value = serde_json::from_str(&stdout(&output)).expect("JSON");
    shown["items"][0]["item"]["version"]
        .as_str()
        .expect("a version")
        .to_owned()
}

fn priority_of(path: &Path) -> String {
    std::fs::read_to_string(path)
        .expect("the task file")
        .lines()
        .find_map(|line| line.strip_prefix("priority: ").map(str::to_owned))
        .unwrap_or_default()
}

#[test]
fn two_writes_held_to_one_version_of_a_folder_task_never_both_land() {
    let sandbox = Sandbox::new();
    let folder = sandbox.subdirectory("folder");
    std::fs::create_dir_all(folder.join("tasks")).expect("the task folder");
    let file = folder.join("tasks").join("T.md");
    sandbox.project_document(&document(&json!({
        "folder": {"plugin": "local-md", "config": {"root": folder}},
    })));
    for round in 0..24 {
        std::fs::write(
            &file,
            "---\ntitle: Raced\nstatus: todo\npriority: high\n---\nBody.\n",
        )
        .expect("the task file");
        let version = folder_version(&sandbox);
        let answers = race(&sandbox, &version, round);
        let landed: Vec<&(Option<i32>, String)> = answers
            .iter()
            .filter(|(code, _)| *code == Some(0))
            .collect();
        assert_eq!(
            landed.len(),
            1,
            "round {round}: exactly one lands: {answers:#?}"
        );
        let refused = answers
            .iter()
            .find(|(code, _)| *code != Some(0))
            .expect("the other is refused");
        assert_eq!(refused.0, Some(1), "round {round}: {answers:#?}");
        let failure: Value = serde_json::from_str(&refused.1).expect("a failure document");
        assert_eq!(failure["failure"]["kind"], "version-moved", "round {round}");
        // What the file holds is the one write that landed, never a mix of the two.
        let answered: Value = serde_json::from_str(&landed[0].1).expect("an answer");
        assert_eq!(
            priority_of(&file),
            answered["priority"].as_str().expect("a priority"),
            "round {round}: {answered:#}"
        );
        assert_eq!(
            failure["held"]["version"],
            folder_version(&sandbox),
            "round {round}"
        );
    }
}

/// The fields a field-scoped write must leave as they were: every member of `task show` but
/// the version, the field written and the time a hosted source stamps the write with — and the
/// field's reserved metadata spelling, which a Linear read also reports among the metadata.
fn untouched(task: &Value, written: &str) -> Value {
    let mut task = task.clone();
    let fields = task.as_object_mut().expect("a task is an object");
    for member in ["version", "updated_at", written] {
        fields.remove(member);
    }
    if let Some(metadata) = fields.get_mut("metadata").and_then(Value::as_object_mut) {
        metadata.remove(&format!("onetaskgraph.{written}"));
    }
    task
}

#[test]
fn a_guarded_priority_set_writes_only_over_a_priority_it_names_and_answers_held_otherwise() {
    for destination in DESTINATIONS {
        let stores = Stores::new(false);
        stores.note("A", "Task A");
        let task =
            stores.exited(&owned(&["task", "copy", "notes:A", "--to", destination]), 0)["items"][0]
                ["destination"]
                .as_str()
                .expect("an id")
                .to_owned();
        let before = stores.shown(&task);
        assert_eq!(before["priority"], "high", "{destination}");

        // Held: the task is at `high`, which the guard does not name.
        let held = stores.exited(
            &owned(&[
                "task",
                "priority",
                "set",
                &task,
                "low",
                "--when-priority",
                "none",
                "--when-priority",
                "medium",
            ]),
            0,
        );
        assert_eq!(held["outcome"], "held", "{destination}: {held:#}");
        assert_eq!(held["priority"], "high", "{destination}: {held:#}");
        assert_eq!(
            held["priority_condition"],
            json!({"held": "high", "met": false}),
            "{destination}"
        );
        assert_eq!(
            stores.shown(&task),
            before,
            "{destination}: nothing was written"
        );

        // Written: the guard names the priority the task holds.
        let written = stores.exited(
            &owned(&[
                "task",
                "priority",
                "set",
                &task,
                "low",
                "--when-priority",
                "high",
            ]),
            0,
        );
        assert_eq!(written["outcome"], "written", "{destination}: {written:#}");
        assert_eq!(written["priority"], "low", "{destination}");
        assert_eq!(
            written["priority_condition"],
            json!({"held": "high", "met": true}),
            "{destination}"
        );
        assert_eq!(stores.shown(&task)["priority"], "low", "{destination}");

        // `none` names a task with no priority.
        stores.exited(&owned(&["task", "priority", "set", &task, "none"]), 0);
        let cleared = stores.exited(
            &owned(&[
                "task",
                "priority",
                "set",
                &task,
                "medium",
                "--when-priority",
                "none",
            ]),
            0,
        );
        assert_eq!(cleared["outcome"], "written", "{destination}: {cleared:#}");
        assert_eq!(stores.shown(&task)["priority"], "medium", "{destination}");

        // Held to a version another writer moved the task past: refused, the task unchanged.
        let read = stores.version(&task);
        stores.exited(&owned(&["task", "update", &task, "--title", "Moved"]), 0);
        let moved = stores.shown(&task);
        let refused = stores.exited(
            &owned(&[
                "task",
                "priority",
                "set",
                &task,
                "urgent",
                "--when-priority",
                "medium",
                "--if-version",
                &read,
            ]),
            1,
        );
        assert_eq!(
            refused["failure"]["kind"], "version-moved",
            "{destination}: {refused:#}"
        );
        assert_eq!(
            refused["held"]["version"], moved["version"],
            "{destination}"
        );
        assert_eq!(
            stores.shown(&task),
            moved,
            "{destination}: nothing was written"
        );
        // And a guard answering `held` at a stale version is refused all the same.
        let stale_held = stores.exited(
            &owned(&[
                "task",
                "priority",
                "set",
                &task,
                "urgent",
                "--when-priority",
                "low",
                "--if-version",
                &read,
            ]),
            1,
        );
        assert_eq!(
            stale_held["failure"]["kind"], "version-moved",
            "{destination}"
        );
    }
}

#[test]
fn a_repositories_update_replaces_the_list_alone_and_clears_it() {
    for destination in DESTINATIONS {
        let stores = Stores::new(false);
        stores.note("A", "Task A");
        let task =
            stores.exited(&owned(&["task", "copy", "notes:A", "--to", destination]), 0)["items"][0]
                ["destination"]
                .as_str()
                .expect("an id")
                .to_owned();
        stores.exited(
            &owned(&[
                "task",
                "metadata",
                "set",
                &task,
                "followup.kept",
                "\"as it was\"",
            ]),
            0,
        );
        let before = stores.shown(&task);

        let named = ["github.com/acme/widgets", "github.com/acme/gadgets"];
        let updated = stores.exited(
            &owned(&[
                "task",
                "update",
                &task,
                "--repository",
                named[0],
                "--repository",
                named[1],
            ]),
            0,
        );
        assert_eq!(
            updated["written"],
            json!(["repositories"]),
            "{destination}: {updated:#}"
        );
        let after = stores.shown(&task);
        assert_eq!(
            after["repositories"],
            json!(named),
            "{destination}: {after:#}"
        );
        assert_eq!(
            untouched(&after, "repositories"),
            untouched(&before, "repositories"),
            "{destination}: title, body, status, priority and metadata read back unchanged"
        );

        // Held to a version another writer moved the task past: refused, the list unchanged.
        let read = stores.version(&task);
        stores.exited(&owned(&["task", "update", &task, "--title", "Moved"]), 0);
        let moved = stores.shown(&task);
        let refused = stores.exited(
            &owned(&[
                "task",
                "update",
                &task,
                "--no-repositories",
                "--if-version",
                &read,
            ]),
            1,
        );
        assert_eq!(
            refused["failure"]["kind"], "version-moved",
            "{destination}: {refused:#}"
        );
        assert_eq!(
            stores.shown(&task),
            moved,
            "{destination}: nothing was written"
        );

        // Cleared, at the version the task is at.
        let current = stores.version(&task);
        stores.exited(
            &owned(&[
                "task",
                "update",
                &task,
                "--no-repositories",
                "--if-version",
                &current,
            ]),
            0,
        );
        let cleared = stores.shown(&task);
        assert_eq!(
            cleared["repositories"],
            json!([]),
            "{destination}: {cleared:#}"
        );
        assert_eq!(
            untouched(&cleared, "repositories"),
            untouched(&moved, "repositories"),
            "{destination}"
        );
    }
}

#[test]
fn an_in_memory_task_takes_a_guarded_priority_and_a_repositories_update() {
    let sandbox = Sandbox::new();
    std::fs::create_dir_all(sandbox.subdirectory("folder").join("tasks")).expect("a folder");
    memory(&sandbox, "As configured");
    let run = |arguments: &[&str], code: i32| -> Value {
        let mut full = vec!["--json"];
        full.extend(arguments);
        let output = sandbox.command().args(&full).assert().get_output().clone();
        assert_eq!(
            output.status.code(),
            Some(code),
            "{}\n{}",
            stdout(&output),
            stderr(&output)
        );
        serde_json::from_str(&stdout(&output)).expect("JSON")
    };
    let held = run(
        &[
            "task",
            "priority",
            "set",
            "memory:T-1",
            "low",
            "--when-priority",
            "none",
        ],
        0,
    );
    assert_eq!(held["outcome"], "held", "{held:#}");
    assert_eq!(held["priority"], "high");
    let written = run(
        &[
            "task",
            "priority",
            "set",
            "memory:T-1",
            "low",
            "--when-priority",
            "high",
        ],
        0,
    );
    assert_eq!(written["outcome"], "written", "{written:#}");
    assert_eq!(written["priority"], "low");

    let updated = run(
        &[
            "task",
            "update",
            "memory:T-1",
            "--repository",
            "github.com/acme/widgets",
        ],
        0,
    );
    assert_eq!(
        updated["task"]["repositories"],
        json!(["github.com/acme/widgets"])
    );
    assert_eq!(updated["task"]["title"], "As configured");
    assert_eq!(updated["written"], json!(["repositories"]));
    let read = run(&["task", "show", "memory:T-1"], 0)["items"][0]["item"]["version"]
        .as_str()
        .expect("a version")
        .to_owned();
    memory(&sandbox, "Changed since the read");
    let refused = run(
        &[
            "task",
            "update",
            "memory:T-1",
            "--no-repositories",
            "--if-version",
            &read,
        ],
        1,
    );
    assert_eq!(refused["failure"]["kind"], "version-moved", "{refused:#}");
    let stale = run(
        &[
            "task",
            "priority",
            "set",
            "memory:T-1",
            "low",
            "--when-priority",
            "high",
            "--if-version",
            &read,
        ],
        1,
    );
    assert_eq!(stale["failure"]["kind"], "version-moved", "{stale:#}");
}

#[test]
fn a_guard_reads_as_held_or_written_to_a_person_and_metadata_off_a_task_takes_no_version() {
    let stores = Stores::new(false);
    stores.note("A", "Task A");
    let text = |arguments: &[&str], code: i32| -> String {
        let output = stores
            .sandbox
            .command()
            .args(arguments)
            .assert()
            .get_output()
            .clone();
        assert_eq!(output.status.code(), Some(code), "{}", stderr(&output));
        stdout(&output)
    };
    let held = text(
        &[
            "task",
            "priority",
            "set",
            "notes:A",
            "low",
            "--when-priority",
            "none",
        ],
        0,
    );
    assert!(held.contains("outcome:") && held.contains("held"), "{held}");
    assert!(held.contains("high"), "the priority it keeps: {held}");
    let written = text(
        &[
            "task",
            "priority",
            "set",
            "notes:A",
            "low",
            "--when-priority",
            "high",
        ],
        0,
    );
    assert!(written.contains("written"), "{written}");
    let plain = text(&["task", "priority", "set", "notes:A", "medium"], 0);
    assert!(!plain.contains("outcome:"), "no guard, no outcome: {plain}");

    // A project's and a document's metadata are written without a version to hold, so naming one
    // is refused before anything is written.
    let projects = stores.notes.join("projects");
    std::fs::create_dir_all(&projects).expect("a folder");
    std::fs::write(
        projects.join("P.md"),
        "---\ntitle: Plan\nstatus: todo\n---\n",
    )
    .expect("a project");
    let before = std::fs::read(projects.join("P.md")).expect("the project");
    let refused = stores.exited(
        &owned(&[
            "project",
            "metadata",
            "set",
            "notes:P",
            "followup.k",
            "1",
            "--if-version",
            "md1|x",
        ]),
        1,
    );
    assert_eq!(
        refused["failure"]["kind"], "if-version-one-task",
        "{refused:#}"
    );
    assert_eq!(
        std::fs::read(projects.join("P.md")).expect("the project"),
        before
    );
    let documents = stores.notes.join("documents");
    std::fs::create_dir_all(&documents).expect("a folder");
    std::fs::write(documents.join("D.md"), "---\ntitle: Design\n---\nprose\n").expect("a document");
    let refused = stores.exited(
        &owned(&[
            "document",
            "metadata",
            "set",
            "notes:D",
            "followup.k",
            "1",
            "--if-version",
            "md1|x",
        ]),
        1,
    );
    assert_eq!(
        refused["failure"]["kind"], "if-version-one-task",
        "{refused:#}"
    );
}

/// A source hosted a process away over the stdio plugin protocol — the in-memory plugin
/// served by `plugin-serve` — holding one task titled `title`.
fn hosted(sandbox: &Sandbox, title: &str) {
    sandbox.project_document(&document(&json!({
        "hosted": {"plugin": "subprocess", "config": {
            "command": onetaskgraph_e2e_support::binary(),
            "args": ["plugin-serve", "in-memory"],
            "settings": {
                "capabilities": {"priority": "native"},
                "tasks": [{"id": "T-1", "title": title, "labels": [], "priority": "high",
                           "status": {"category": "todo", "name": "todo"}}],
            },
        }},
    })));
}

#[test]
fn a_write_held_across_the_stdio_plugin_protocol_lands_at_its_version_and_is_refused_at_another() {
    let sandbox = Sandbox::new();
    let run = |arguments: &[&str], code: i32| -> Value {
        let mut full = vec!["--json"];
        full.extend(arguments);
        let output = sandbox.command().args(&full).assert().get_output().clone();
        assert_eq!(
            output.status.code(),
            Some(code),
            "`onetaskgraph {}`\n{}\n{}",
            full.join(" "),
            stdout(&output),
            stderr(&output)
        );
        serde_json::from_str(&stdout(&output)).expect("JSON")
    };
    hosted(&sandbox, "As served");
    let read = run(&["task", "show", "hosted:T-1"], 0)["items"][0]["item"]["version"]
        .as_str()
        .expect("a hosted task is read with its version")
        .to_owned();
    let landed = run(
        &[
            "task",
            "priority",
            "set",
            "hosted:T-1",
            "low",
            "--if-version",
            &read,
        ],
        0,
    );
    assert_eq!(landed["priority"], "low", "{landed:#}");

    // Another writer: what the peer serves, changed after the read.
    hosted(&sandbox, "Changed since the read");
    let moved = run(&["task", "show", "hosted:T-1"], 0)["items"][0]["item"]["version"]
        .as_str()
        .expect("a version")
        .to_owned();
    let refused = run(
        &[
            "task",
            "update",
            "hosted:T-1",
            "--title",
            "Stale",
            "--if-version",
            &read,
        ],
        1,
    );
    assert_eq!(refused["failure"]["kind"], "version-moved", "{refused:#}");
    assert_eq!(refused["held"]["version"], moved, "{refused:#}");
}

#[test]
fn a_write_expecting_a_version_and_held_to_it_takes_both_on_a_cached_board() {
    let stores = Stores::new(true);
    stores.note("A", "Task A");
    let task = stores.exited(&owned(&["task", "copy", "notes:A", "--to", "board"]), 0)["items"][0]
        ["destination"]
        .as_str()
        .expect("an id")
        .to_owned();
    let read = stores.version(&task);
    let both = stores.exited(
        &owned(&[
            "task",
            "priority",
            "set",
            &task,
            "low",
            "--expect",
            &read,
            "--if-version",
            &read,
        ]),
        0,
    );
    assert_eq!(both["priority"], "low", "{both:#}");
    assert!(
        both["result"]["parts"].is_array(),
        "an expected write still reports its parts: {both:#}"
    );

    // Moved by another writer: the held version refuses the write the expectation would make.
    let written = stores.version(&task);
    stores.exited(&owned(&["task", "update", &task, "--title", "Moved"]), 0);
    let moved = stores.shown(&task);
    let refused = stores.exited(
        &owned(&[
            "task",
            "priority",
            "set",
            &task,
            "urgent",
            "--expect",
            &written,
            "--if-version",
            &written,
        ]),
        1,
    );
    assert_eq!(refused["failure"]["kind"], "version-moved", "{refused:#}");
    assert_eq!(refused["held"]["version"], moved["version"]);
    assert_eq!(stores.shown(&task), moved, "nothing was written");
}

#[test]
fn a_held_copy_carrying_comments_checks_its_first_write_and_lands_the_rest() {
    for destination in DESTINATIONS {
        let stores = Stores::new(false);
        stores.note("A", "Task A");
        let copy = |version: &str| {
            owned(&[
                "task",
                "copy",
                "notes:A",
                "--to",
                destination,
                "--comments",
                "--if-version",
                version,
            ])
        };
        let task =
            stores.exited(&owned(&["task", "copy", "notes:A", "--to", destination]), 0)["items"][0]
                ["destination"]
                .as_str()
                .expect("an id")
                .to_owned();
        let comments = |id: &str| stores.exited(&owned(&["task", "comment", "list", id]), 0);

        // At the counterpart's version: the task and every comment after it land. A note is
        // rewritten whole, so its comment is added after it.
        stores.note("A", "Task A, revised");
        let comment = stores.file("comment.md", "A comment the copy carries.\n");
        stores.exited(
            &owned(&["task", "comment", "add", "notes:A", "--body-file", &comment]),
            0,
        );
        let read = stores.version(&task);
        stores.exited(&copy(&read), 0);
        let carried = comments(&task);
        assert!(
            carried["comments"]
                .as_array()
                .expect("a list")
                .iter()
                .any(|held| held["body"]
                    .as_str()
                    .is_some_and(|body| body.contains("A comment the copy carries."))),
            "{destination}: {carried:#}"
        );

        // At a version another writer moved it past: nothing lands, comments included.
        let written = stores.version(&task);
        stores.exited(&owned(&["task", "update", &task, "--title", "Moved"]), 0);
        stores.exited(
            &owned(&["task", "update", "notes:A", "--title", "Task A, again"]),
            0,
        );
        let second = stores.file("second.md", "A second comment.\n");
        stores.exited(
            &owned(&["task", "comment", "add", "notes:A", "--body-file", &second]),
            0,
        );
        let before = comments(&task);
        let refused = stores.exited(&copy(&written), 1);
        assert_eq!(
            refused["failure"]["kind"], "version-moved",
            "{destination}: {refused:#}"
        );
        assert_eq!(stores.shown(&task)["title"], "Moved", "{destination}");
        assert_eq!(
            comments(&task)["comments"],
            before["comments"],
            "{destination}: no comment was carried"
        );
    }
}
