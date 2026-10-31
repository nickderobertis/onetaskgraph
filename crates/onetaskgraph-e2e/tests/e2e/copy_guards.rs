//! A task copy that guards its own writes on what the destination holds:
//! `--when-destination-status`, `--otherwise` and `--when-destination-priority`.
//!
//! The realistic caller syncs a folder of Markdown onto a tracker a person also edits. Its copy
//! may move a card only while the card is still where the sync left it, and may write a priority
//! only while nobody has set one by hand. Each journey drives the binary from a folder of
//! Markdown into the loopback GitHub board and into the loopback Linear workspace, reads the
//! destination back through `task show`, and counts the requests each destination was sent: a
//! guard is decided on the read the copy already makes, so it costs nothing.

use std::path::PathBuf;
use std::process::Output;

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{
    GitHubBoardFields, LinearWorkspace, dataset, document, github_projects_with_board,
    linear_default_mapping, linear_workspace,
};

/// The two hosted destinations every journey here runs against.
const DESTINATIONS: [&str; 2] = ["board", "linear"];

/// The categories the folder's files spell, each under its own name.
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
        Self::caching(false)
    }

    /// The stores, the board's source keeping its local cache when `cache` is set.
    fn caching(cache: bool) -> Self {
        let sandbox = Sandbox::new();
        let cache_dir = sandbox.subdirectory("board-cache");
        let (board_config, board) = github_projects_with_board(&sandbox);
        let (mut linear_config, linear) = linear_workspace(&sandbox, dataset());
        linear_config["status_mapping"] = linear_default_mapping();
        sandbox
            .secrets_file("GITHUB_PROJECTS_FIXTURE_TOKEN=test-token\nLINEAR_API_KEY=fixture-key\n");
        let notes = sandbox.subdirectory("notes");
        std::fs::create_dir_all(notes.join("tasks")).expect("the task folder");
        sandbox.project_document(&document(&json!({
            "board": {"plugin": "github-projects", "config": board_config,
                      "cache": {"enabled": cache, "dir": cache_dir}},
            "linear": {"plugin": "linear", "config": linear_config},
            "notes": {"plugin": "local-md",
                      "config": {"root": notes, "status_mapping": folder_mapping()}},
        })));
        Self {
            sandbox,
            board,
            linear,
            notes,
        }
    }

    /// Write one task of the folder, keeping any link an earlier copy recorded in it.
    fn note(&self, id: &str, title: &str, status: &str, priority: &str, body: &str) {
        let path = self.notes.join("tasks").join(format!("{id}.md"));
        let kept = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| {
                let start = text.find("metadata:")?;
                let end = text[start..].find("\n---")? + start;
                Some(text[start..end].to_owned())
            })
            .map(|metadata| format!("{metadata}\n"))
            .unwrap_or_default();
        let priority = if priority == "none" {
            String::new()
        } else {
            format!("priority: {priority}\n")
        };
        std::fs::write(
            path,
            format!("---\ntitle: {title}\nstatus: {status}\n{priority}{kept}---\n{body}\n"),
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

    /// The JSON a run printed, having exited `code`.
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

    /// The one task `id` reads as.
    fn shown(&self, id: &str) -> Value {
        self.json(&["task", "show", id])["items"][0]["item"].clone()
    }

    /// How many requests `destination` has been sent.
    fn served(&self, destination: &str) -> usize {
        match destination {
            "board" => self.board.served().len(),
            _ => self.linear.served().len(),
        }
    }

    /// Copy each of `ids` from the folder into `destination`, answering each one's counterpart.
    fn seed(&self, destination: &str, ids: &[&str]) -> Vec<String> {
        let mut arguments = vec!["task", "copy"];
        let qualified: Vec<String> = ids.iter().map(|id| format!("notes:{id}")).collect();
        arguments.extend(qualified.iter().map(String::as_str));
        arguments.extend(["--to", destination]);
        let copied = self.json(&arguments);
        copied["items"]
            .as_array()
            .expect("an outcome per task")
            .iter()
            .map(|item| item["destination"].as_str().expect("created").to_owned())
            .collect()
    }
}

/// Each outcome of a copy by its source id: its action and the whole entry.
fn outcome<'a>(report: &'a Value, source: &str) -> &'a Value {
    report["items"]
        .as_array()
        .expect("an items list")
        .iter()
        .find(|item| item["source"] == source)
        .unwrap_or_else(|| panic!("an outcome for {source}: {report:#}"))
}

/// Four tasks at `todo`, `done`, `backlog` and `cancelled` copied into `destination`, then each
/// moved to `queued` and retitled in the folder: every counterpart now reads the category it was
/// created at, and the next copy would write `queued` over it.
fn seeded(destination: &str) -> (Stores, Vec<String>) {
    let stores = Stores::new();
    for (id, status) in [
        ("A", "todo"),
        ("B", "done"),
        ("C", "backlog"),
        ("D", "cancelled"),
    ] {
        stores.note(
            id,
            &format!("Task {id}"),
            status,
            "high",
            "As first written.",
        );
    }
    let landed = stores.seed(destination, &["A", "B", "C", "D"]);
    for id in ["A", "B", "C", "D"] {
        stores.note(
            id,
            &format!("Task {id}, revised"),
            "queued",
            "low",
            "Revised.",
        );
    }
    (stores, landed)
}

const STATUS_GUARD: [&str; 6] = [
    "--when-destination-status",
    "backlog",
    "--when-destination-status",
    "cancelled",
    "--otherwise",
    "refuse",
];

#[test]
fn a_status_guard_writes_only_the_counterparts_at_a_named_category_and_refuses_the_rest() {
    for destination in DESTINATIONS {
        let (stores, landed) = seeded(destination);
        let mut arguments = vec![
            "task",
            "copy",
            "notes:A",
            "notes:B",
            "notes:C",
            "notes:D",
            "--to",
            destination,
        ];
        arguments.extend(STATUS_GUARD);
        let report = stores.exited(&arguments, 4);
        for (id, held, met) in [
            ("notes:A", "todo", false),
            ("notes:B", "done", false),
            ("notes:C", "backlog", true),
            ("notes:D", "cancelled", true),
        ] {
            let item = outcome(&report, id);
            assert_eq!(
                item["status_condition"],
                json!({"held": held, "met": met}),
                "{destination} {id}: {item:#}"
            );
            if met {
                assert_eq!(item["action"], "updated", "{destination} {id}: {item:#}");
            } else {
                assert_eq!(item["action"], "failed", "{destination} {id}: {item:#}");
                let failure = &item["error"];
                assert_eq!(failure["kind"], "destination-status", "{failure:#}");
                let message = failure["message"].as_str().expect("a message");
                assert!(
                    message.contains(id)
                        && message.contains(held)
                        && message.contains("backlog, cancelled"),
                    "{destination}: names the item, its category and the set: {message}"
                );
            }
        }
        // The refused counterparts were written nothing; the others carry the source.
        for (index, held) in [(0, "todo"), (1, "done")] {
            let task = stores.shown(&landed[index]);
            assert_eq!(task["status"]["category"], held, "{destination}: {task:#}");
            assert!(
                !task["title"].as_str().unwrap().contains("revised"),
                "{destination}: {task:#}"
            );
        }
        for index in [2, 3] {
            let task = stores.shown(&landed[index]);
            assert_eq!(
                task["status"]["category"], "queued",
                "{destination}: {task:#}"
            );
            assert!(task["title"].as_str().unwrap().ends_with("revised"));
        }
    }
}

#[test]
fn otherwise_preserve_destination_copies_the_rest_and_keeps_the_status() {
    for destination in DESTINATIONS {
        let (stores, landed) = seeded(destination);
        let report = stores.json(&[
            "task",
            "copy",
            "notes:A",
            "--to",
            destination,
            "--when-destination-status",
            "backlog",
            "--otherwise",
            "preserve-destination",
        ]);
        let item = outcome(&report, "notes:A");
        assert_eq!(item["action"], "updated", "{destination}: {item:#}");
        assert_eq!(
            item["status_condition"],
            json!({"held": "todo", "met": false})
        );
        let task = stores.shown(&landed[0]);
        assert_eq!(
            task["status"]["category"], "todo",
            "{destination}: {task:#}"
        );
        assert_eq!(task["title"], "Task A, revised", "{destination}: {task:#}");
        assert_eq!(task["priority"], "low", "{destination}: {task:#}");
    }
}

#[test]
fn a_guarded_copy_sends_exactly_the_requests_of_the_same_copy_without_the_guard() {
    for destination in DESTINATIONS {
        let mut counts = Vec::new();
        for guarded in [false, true] {
            let (stores, _) = seeded(destination);
            let mut arguments = vec!["task", "copy", "notes:C", "--to", destination];
            if guarded {
                arguments.extend([
                    "--when-destination-status",
                    "backlog",
                    "--when-destination-priority",
                    "high",
                ]);
            }
            let before = stores.served(destination);
            stores.json(&arguments);
            counts.push(stores.served(destination) - before);
        }
        assert_eq!(counts[0], counts[1], "{destination}: {counts:?}");
    }
}

#[test]
fn a_create_under_a_guard_is_made_as_without_it_and_a_dry_run_reports_each_decision() {
    for destination in DESTINATIONS {
        let (stores, landed) = seeded(destination);
        stores.note("E", "Task E", "queued", "urgent", "New.");
        let mut arguments = vec![
            "task",
            "copy",
            "notes:A",
            "notes:C",
            "notes:E",
            "--to",
            destination,
            "--dry-run",
            "--when-destination-priority",
            "low",
        ];
        arguments.extend(STATUS_GUARD);
        let before = stores.served(destination);
        let report = stores.exited(&arguments, 4);
        let refused = outcome(&report, "notes:A");
        assert_eq!(refused["error"]["kind"], "destination-status");
        assert_eq!(
            refused["priority_condition"],
            json!({"held": "high", "met": false})
        );
        let kept = outcome(&report, "notes:C");
        assert_eq!(kept["action"], "updated");
        assert_eq!(
            kept["status_condition"],
            json!({"held": "backlog", "met": true})
        );
        let created = outcome(&report, "notes:E");
        assert_eq!(created["action"], "created", "{created:#}");
        assert!(created.get("status_condition").is_none(), "{created:#}");
        assert!(created.get("priority_condition").is_none(), "{created:#}");
        assert!(stores.served(destination) >= before);
        // Nothing was written: the counterpart still reads as first copied.
        assert_eq!(stores.shown(&landed[2])["title"], "Task C");
        // The real copy creates E with the source's priority, guard or no guard.
        arguments.retain(|argument| *argument != "--dry-run");
        let report = stores.exited(&arguments, 4);
        let created = outcome(&report, "notes:E");
        let task = stores.shown(created["destination"].as_str().expect("an id"));
        assert_eq!(task["priority"], "urgent", "{destination}: {task:#}");
        assert_eq!(
            task["status"]["category"], "queued",
            "{destination}: {task:#}"
        );
    }
}

#[test]
fn a_priority_guard_keeps_a_priority_a_person_set_and_writes_every_other_field() {
    for destination in DESTINATIONS {
        let (stores, landed) = seeded(destination);
        // A person sets two counterparts' priorities by hand.
        stores.json(&["task", "priority", "set", &landed[0], "urgent"]);
        stores.json(&["task", "priority", "set", &landed[1], "medium"]);
        stores.json(&["task", "priority", "set", &landed[2], "none"]);
        let report = stores.json(&[
            "task",
            "copy",
            "notes:A",
            "notes:B",
            "notes:C",
            "--to",
            destination,
            "--when-destination-priority",
            "medium",
            "--when-destination-priority",
            "none",
        ]);
        for (index, id, held, met, now) in [
            (0, "notes:A", "urgent", false, "urgent"),
            (1, "notes:B", "medium", true, "low"),
            (2, "notes:C", "none", true, "low"),
        ] {
            let item = outcome(&report, id);
            assert_eq!(item["action"], "updated", "{destination} {id}: {item:#}");
            assert_eq!(
                item["priority_condition"],
                json!({"held": held, "met": met})
            );
            assert!(item.get("status_condition").is_none(), "{item:#}");
            let task = stores.shown(&landed[index]);
            assert_eq!(task["priority"], now, "{destination} {id}: {task:#}");
            assert_eq!(task["status"]["category"], "queued", "{destination} {id}");
            assert!(task["title"].as_str().unwrap().ends_with("revised"));
            assert_eq!(task["content"].as_str().map(str::trim), Some("Revised."));
        }
        // The next copy of the unchanged source without the guard writes its priority.
        let report = stores.json(&["task", "copy", "notes:A", "--to", destination]);
        assert_eq!(
            outcome(&report, "notes:A")["action"],
            "updated",
            "{destination} {report:#}"
        );
        assert_eq!(stores.shown(&landed[0])["priority"], "low");
    }
}

#[test]
fn a_priority_guard_beside_preserve_destination_keeps_both_and_a_refused_item_gets_nothing() {
    for destination in DESTINATIONS {
        let (stores, landed) = seeded(destination);
        stores.json(&["task", "priority", "set", &landed[0], "urgent"]);
        stores.json(&[
            "task",
            "copy",
            "notes:A",
            "--to",
            destination,
            "--status-policy",
            "preserve-destination",
            "--when-destination-priority",
            "low",
        ]);
        let task = stores.shown(&landed[0]);
        assert_eq!(task["priority"], "urgent", "{destination}: {task:#}");
        assert_eq!(
            task["status"]["category"], "todo",
            "{destination}: {task:#}"
        );
        assert_eq!(task["title"], "Task A, revised");

        // Refused for its status, an item is written nothing whatever its priority.
        stores.note("B", "Task B, again", "queued", "low", "Again.");
        let mut arguments = vec![
            "task",
            "copy",
            "notes:B",
            "--to",
            destination,
            "--when-destination-priority",
            "high",
        ];
        arguments.extend(STATUS_GUARD);
        let report = stores.exited(&arguments, 4);
        assert_eq!(
            outcome(&report, "notes:B")["error"]["kind"],
            "destination-status"
        );
        let task = stores.shown(&landed[1]);
        assert_eq!(task["title"], "Task B", "{destination}: {task:#}");
        assert_eq!(task["priority"], "high", "{destination}: {task:#}");
    }
}

#[test]
fn a_status_guard_beside_preserve_destination_and_otherwise_alone_are_refused_before_any_request() {
    for destination in DESTINATIONS {
        let (stores, _) = seeded(destination);
        let before = stores.served(destination);
        for (extra, kind) in [
            (
                vec![
                    "--when-destination-status",
                    "todo",
                    "--status-policy",
                    "preserve-destination",
                ],
                "status-condition-preserved",
            ),
            (vec!["--otherwise", "refuse"], "otherwise-without-status"),
        ] {
            let mut arguments = vec!["task", "copy", "notes:A", "--to", destination];
            arguments.extend(extra);
            let failure = stores.exited(&arguments, 1);
            assert_eq!(failure["failure"]["kind"], kind, "{failure:#}");
            assert!(
                failure["failure"]["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("next:")),
                "{failure:#}"
            );
        }
        assert_eq!(stores.served(destination), before, "{destination}");
    }
}

#[test]
fn a_project_copy_refuses_a_guard_before_anything_is_read() {
    let stores = Stores::new();
    let output = stores.run(&[
        "project",
        "copy",
        "notes:P",
        "--to",
        "board",
        "--when-destination-status",
        "todo",
    ]);
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
}

#[test]
fn under_expect_the_status_guard_is_decided_on_the_cached_counterpart_with_no_read_of_it() {
    let stores = Stores::caching(true);
    stores.note("A", "Task A", "todo", "high", "As first written.");
    let landed = stores.seed("board", &["A"]);
    // The read that fills the cache, and the version the caller then knows.
    let version = stores.shown(&landed[0])["version"]
        .as_str()
        .expect("a board task carries its version")
        .to_owned();
    stores.note("A", "Task A, revised", "queued", "low", "Revised.");
    let before = stores.board.served();
    let report = stores.exited(
        &[
            "task",
            "copy",
            "notes:A",
            "--to",
            "board",
            "--expect",
            &version,
            "--when-destination-status",
            "backlog",
        ],
        4,
    );
    let item = outcome(&report, "notes:A");
    assert_eq!(item["error"]["kind"], "destination-status", "{item:#}");
    assert_eq!(
        item["status_condition"],
        json!({"held": "todo", "met": false})
    );
    let sent = &stores.board.served()[before.len()..];
    assert!(sent.is_empty(), "the board was sent nothing: {sent:#?}");
    assert_eq!(stores.shown(&landed[0])["title"], "Task A");
}
