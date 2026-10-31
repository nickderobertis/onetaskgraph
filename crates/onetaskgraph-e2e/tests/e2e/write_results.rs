//! What a copy and every task write verb report the item holds once written — `result`, read
//! off the write's own answers — and what a refused copy reports it held — `held`.
//!
//! The realistic caller records its sync from the write's own answer rather than reading the
//! item back. Each journey drives the binary from a folder of Markdown into a second folder, the
//! loopback GitHub board and the loopback Linear workspace, then reads every item back through
//! `task show` — the test's own read, not the verb's — and holds the answer to it: the status and
//! priority field for field, and the version wherever the answer gave one. A board answers no
//! version for an item it has just written, because its token waits for the item to settle.

use std::path::PathBuf;
use std::process::Output;

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{
    GitHubBoardFields, LinearWorkspace, dataset, document, github_projects_with_board,
    linear_default_mapping, linear_workspace,
};

/// Every destination, and whether a write's own answer there carries the item's version.
const DESTINATIONS: [(&str, bool); 3] = [("folder", true), ("linear", true), ("board", false)];

fn folder_mapping() -> Value {
    json!({"backlog": "backlog", "todo": "todo", "queued": "queued",
           "in-progress": "in-progress", "done": "done", "cancelled": "cancelled"})
}

struct Stores {
    sandbox: Sandbox,
    board: GitHubBoardFields,
    linear: LinearWorkspace,
    notes: PathBuf,
}

impl Stores {
    fn new() -> Self {
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
        sandbox.project_document(&document(&json!({
            "board": {"plugin": "github-projects", "config": board_config},
            "linear": {"plugin": "linear", "config": linear_config},
            "notes": {"plugin": "local-md",
                      "config": {"root": notes, "status_mapping": folder_mapping()}},
            "folder": {"plugin": "local-md",
                       "config": {"root": folder, "status_mapping": folder_mapping()}},
        })));
        Self {
            sandbox,
            board,
            linear,
            notes,
        }
    }

    fn note(&self, id: &str, title: &str, status: &str, priority: &str) {
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
            format!(
                "---\ntitle: {title}\nstatus: {status}\npriority: {priority}\n{kept}---\nBody.\n"
            ),
        )
        .expect("the task file");
    }

    fn run(&self, arguments: &[&str]) -> Output {
        self.sandbox
            .command()
            .args(arguments)
            .assert()
            .get_output()
            .clone()
    }

    fn exited(&self, arguments: &[&str], code: i32) -> Value {
        let mut full = vec!["--json"];
        full.extend(arguments);
        let output = self.run(&full);
        assert_eq!(
            output.status.code(),
            Some(code),
            "`onetaskgraph {}`\nstdout:\n{}\nstderr:\n{}",
            full.join(" "),
            stdout(&output),
            stderr(&output)
        );
        serde_json::from_str(&stdout(&output)).expect("the run answers JSON")
    }

    fn json(&self, arguments: &[&str]) -> Value {
        self.exited(arguments, 0)
    }

    fn shown(&self, id: &str) -> Value {
        self.json(&["task", "show", id])["items"][0]["item"].clone()
    }

    fn served(&self, destination: &str) -> usize {
        match destination {
            "board" => self.board.served().len(),
            "linear" => self.linear.served().len(),
            _ => 0,
        }
    }
}

/// Assert `result` reports what `task show` of `id` then answers.
fn holds_as_shown(stores: &Stores, id: &str, result: &Value, versioned: bool, what: &str) {
    let task = stores.shown(id);
    assert_eq!(
        result["status"], task["status"],
        "{what}: status of {id}: {result:#}"
    );
    assert_eq!(
        result["priority"], task["priority"],
        "{what}: priority of {id}"
    );
    let version = &result["version"];
    if versioned {
        assert!(
            version.is_string(),
            "{what}: {id} answers a version: {result:#}"
        );
    }
    if !version.is_null() {
        assert_eq!(version, &task["version"], "{what}: version of {id}");
    }
}

fn only_outcome(report: &Value) -> &Value {
    &report["items"][0]
}

#[test]
fn every_landed_outcome_reports_what_task_show_then_reads() {
    for (destination, versioned) in DESTINATIONS {
        let stores = Stores::new();
        stores.note("A", "Task A", "todo", "high");
        let copy = ["task", "copy", "notes:A", "--to", destination];

        let created = stores.json(&copy);
        let outcome = only_outcome(&created);
        assert_eq!(outcome["action"], "created", "{destination}: {created:#}");
        let landed = outcome["destination"].as_str().expect("an id").to_owned();
        holds_as_shown(&stores, &landed, &outcome["result"], versioned, destination);
        assert!(outcome["result"].get("parts").is_none(), "{outcome:#}");

        stores.note("A", "Task A, revised", "queued", "low");
        let updated = stores.json(&copy);
        let outcome = only_outcome(&updated);
        assert_eq!(outcome["action"], "updated", "{destination}: {updated:#}");
        holds_as_shown(&stores, &landed, &outcome["result"], versioned, destination);

        // Nothing changed: what the read before the copy held, at the version it read.
        let before = stores.served(destination);
        let unchanged = stores.json(&copy);
        let outcome = only_outcome(&unchanged);
        assert_eq!(
            outcome["action"], "unchanged",
            "{destination}: {unchanged:#}"
        );
        let task = stores.shown(&landed);
        assert_eq!(outcome["result"]["status"], task["status"]);
        assert_eq!(outcome["result"]["priority"], task["priority"]);
        assert_eq!(
            outcome["result"]["version"], task["version"],
            "{destination}"
        );
        let _ = before;

        // A status the destination kept, and a priority the guard kept.
        stores.json(&["task", "priority", "set", &landed, "urgent"]);
        stores.note("A", "Task A, again", "in-progress", "medium");
        let kept = stores.json(&[
            "task",
            "copy",
            "notes:A",
            "--to",
            destination,
            "--status-policy",
            "preserve-destination",
            "--when-destination-priority",
            "none",
        ]);
        let outcome = only_outcome(&kept);
        assert_eq!(outcome["action"], "updated", "{destination}: {kept:#}");
        assert_eq!(outcome["result"]["status"]["category"], "queued");
        assert_eq!(outcome["result"]["priority"], "urgent");
        holds_as_shown(&stores, &landed, &outcome["result"], versioned, destination);

        // A dry run reports no result.
        stores.note("A", "Task A, dry", "todo", "low");
        let dry = stores.json(&["task", "copy", "notes:A", "--to", destination, "--dry-run"]);
        assert!(only_outcome(&dry).get("result").is_none(), "{dry:#}");
    }
}

#[test]
fn a_refused_copy_reports_what_the_counterpart_held_and_one_never_read_reports_nothing() {
    for (destination, _) in DESTINATIONS {
        let stores = Stores::new();
        stores.note("A", "Task A", "todo", "high");
        let landed = only_outcome(&stores.json(&["task", "copy", "notes:A", "--to", destination]))
            ["destination"]
            .as_str()
            .expect("an id")
            .to_owned();
        stores.note("A", "Task A, revised", "queued", "low");
        let report = stores.exited(
            &[
                "task",
                "copy",
                "notes:A",
                "--to",
                destination,
                "--when-destination-status",
                "done",
            ],
            4,
        );
        let outcome = only_outcome(&report);
        assert_eq!(outcome["error"]["kind"], "destination-status", "{report:#}");
        assert!(outcome.get("result").is_none(), "{outcome:#}");
        let task = stores.shown(&landed);
        let held = &outcome["held"];
        assert_eq!(held["status"], task["status"], "{destination}: {held:#}");
        assert_eq!(held["priority"], task["priority"], "{destination}");
        assert_eq!(held["version"], task["version"], "{destination}");

        // A target that does not name the task back: refused whole, after the read.
        stores.note("B", "Task B", "todo", "none");
        let failure = stores.exited(
            &[
                "task",
                "copy",
                "notes:B",
                "--to",
                destination,
                "--target-task",
                &landed,
            ],
            1,
        );
        assert_eq!(
            failure["failure"]["kind"], "target-task-origin",
            "{failure:#}"
        );
        assert_eq!(failure["held"]["status"], task["status"], "{failure:#}");
        assert_eq!(failure["held"]["priority"], task["priority"], "{failure:#}");

        // A stale link names nothing there, so there was nothing to hold.
        let missing = format!("{destination}:no-such-item");
        let failure = stores.exited(
            &[
                "task",
                "copy",
                "notes:B",
                "--to",
                destination,
                "--target-task",
                &missing,
            ],
            1,
        );
        assert!(failure.get("held").is_none(), "{failure:#}");
    }
}

#[test]
fn every_task_write_verb_answers_what_the_task_then_holds() {
    for (destination, versioned) in DESTINATIONS {
        let stores = Stores::new();
        stores.note("A", "Task A", "todo", "high");
        let landed = only_outcome(&stores.json(&["task", "copy", "notes:A", "--to", destination]))
            ["destination"]
            .as_str()
            .expect("an id")
            .to_owned();
        let content = stores.sandbox.project().join("content.md");
        std::fs::write(&content, "Rewritten.\n").expect("content");
        let content = content.to_str().expect("a path").to_owned();
        for (verb, arguments) in [
            (
                "status set",
                vec!["task", "status", "set", &landed, "queued"],
            ),
            (
                "priority set",
                vec!["task", "priority", "set", &landed, "low"],
            ),
            (
                "content set",
                vec!["task", "content", "set", &landed, "--file", &content],
            ),
            (
                "metadata set",
                vec![
                    "task",
                    "metadata",
                    "set",
                    &landed,
                    "example.sync",
                    "\"done\"",
                ],
            ),
            (
                "update",
                vec!["task", "update", &landed, "--title", "Task A, updated"],
            ),
        ] {
            let answer = stores.json(&arguments);
            holds_as_shown(
                &stores,
                &landed,
                &answer["result"],
                versioned,
                &format!("{destination} {verb}"),
            );
        }
        let body = stores.sandbox.project().join("comment.md");
        std::fs::write(&body, "A word.\n").expect("a comment");
        let added = stores.json(&[
            "task",
            "comment",
            "add",
            &landed,
            "--body-file",
            body.to_str().expect("a path"),
        ]);
        assert_eq!(added["item"], landed.as_str(), "{destination}: {added:#}");
        assert!(
            added.get("body").is_some(),
            "the comment's own members stay: {added:#}"
        );
    }
}
