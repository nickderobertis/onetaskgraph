//! Every malformed answer a source can give the closure methods and the write check is refused,
//! through the binary, against the Python store over a real pipe.
//!
//! A source hosted over the plugin protocol is written by someone else, so what it answers is
//! input: a contradiction list naming one task twice, a holdings list that does not answer each
//! reference once and in order, a status write that reads back another category, and a write
//! check answered with something other than an object. Each journey hands the engine one of
//! those and holds it to refusing before it writes what the answer would have it write.

use std::path::PathBuf;
use std::process::Output;

use serde_json::{Value, json};

use crate::assets::store;
use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::document;

/// A folder of Markdown `plan` holding `A` (`Alpha`) and `B` (`Beta`), both copied into the
/// Python store `store` before any journey begins.
struct Stored {
    sandbox: Sandbox,
    plan: PathBuf,
    at: PathBuf,
    log: PathBuf,
}

impl Stored {
    fn new() -> Self {
        let sandbox = Sandbox::new();
        let plan = sandbox.subdirectory("plan");
        std::fs::create_dir_all(plan.join("tasks")).expect("the task folder");
        for (id, title) in [("A", "Alpha"), ("B", "Beta")] {
            std::fs::write(
                plan.join(format!("tasks/{id}.md")),
                format!("---\ntitle: {title}\nstatus: todo\n---\n{title}.\n"),
            )
            .expect("a task");
        }
        let at = sandbox.subdirectory("store").join("store.json");
        let log = sandbox.subdirectory("store").join("log.jsonl");
        let stored = Self {
            sandbox,
            plan,
            at,
            log,
        };
        stored.configure(json!({}));
        let copied = stored.run(&["task", "copy", "--source", "plan", "--to", "store"]);
        assert_eq!(copied.status.code(), Some(0), "{}", stderr(&copied));
        stored
    }

    /// Configure the store with `settings`, from here on.
    fn configure(&self, settings: Value) {
        self.sandbox.project_document(&document(&json!({
            "plan": {"plugin": "local-md", "config": {"root": self.plan}},
            "store": store(&self.at, &self.log, settings),
        })));
    }

    fn run(&self, arguments: &[&str]) -> Output {
        self.sandbox
            .command()
            .args(arguments)
            .assert()
            .get_output()
            .clone()
    }

    /// The store's own id for the task titled `title`.
    fn id_of(&self, title: &str) -> String {
        let held: Value =
            serde_json::from_str(&std::fs::read_to_string(&self.at).expect("the store"))
                .expect("the store");
        held["tasks"]
            .as_array()
            .expect("tasks")
            .iter()
            .find(|task| task["title"] == title)
            .and_then(|task| task["id"].as_str())
            .unwrap_or_else(|| panic!("the store holds {title}: {held:#}"))
            .to_owned()
    }

    /// Every status write the store was sent.
    fn status_writes(&self) -> Vec<Value> {
        std::fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).expect("a logged line"))
            .filter(|line| line["method"] == "set_task_status")
            .collect()
    }

    /// `arguments` with `--json`, asserting it exits `code`, as its machine output.
    fn json(&self, arguments: &[&str], code: i32) -> (Value, String) {
        let mut full = vec!["--json"];
        full.extend(arguments);
        let output = self.run(&full);
        assert_eq!(
            output.status.code(),
            Some(code),
            "`onetaskgraph {}`\n{}\n{}",
            full.join(" "),
            stdout(&output),
            stderr(&output)
        );
        (
            serde_json::from_str(&stdout(&output)).expect("JSON"),
            stderr(&output),
        )
    }
}

/// The two references every completion below names.
const ISSUES: [&str; 4] = ["--issue", "acme/work#1", "--issue", "acme/work#2"];

#[test]
fn a_contradiction_list_naming_one_task_twice_refuses_the_reconcile_writing_nothing() {
    let stored = Stored::new();
    let alpha = stored.id_of("Alpha");
    stored.configure(json!({"closure": {"contradictions": [alpha, alpha]}}));
    let (failure, _) = stored.json(&["task", "status", "reconcile", "--source", "store"], 1);
    let message = failure["failure"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains(&format!("answered task {alpha} more than once")),
        "{failure:#}"
    );
    assert_eq!(failure["failure"]["source"], "store", "{failure:#}");
    assert!(stored.status_writes().is_empty(), "no card was written");
}

#[test]
fn a_status_write_reading_back_another_category_is_answered_failed_not_updated() {
    let stored = Stored::new();
    let alpha = stored.id_of("Alpha");
    stored.configure(json!({"closure": {"contradictions": [alpha], "reads_back": "todo"}}));
    let (answer, _) = stored.json(&["task", "status", "reconcile", "--source", "store"], 4);
    let item = &answer["items"][0];
    assert_eq!(item["action"], "failed", "{answer:#}");
    assert!(
        item["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("to done read back as todo")),
        "{answer:#}"
    );

    stored.configure(json!({"closure": {"holdings": "due", "reads_back": "todo"}}));
    let (answer, _) = stored.json(
        &["task", "status", "complete", "--source", "store"]
            .iter()
            .copied()
            .chain(ISSUES)
            .collect::<Vec<_>>(),
        4,
    );
    let actions: Vec<&str> = answer["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|item| item["action"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(actions, ["failed", "failed"], "{answer:#}");
}

#[test]
fn a_task_removed_before_its_status_write_is_answered_failed_and_the_others_are_written() {
    let stored = Stored::new();
    let (alpha, beta) = (stored.id_of("Alpha"), stored.id_of("Beta"));
    let gone = format!("store:{alpha}");
    let removed = |answer: &Value| {
        let item = &answer["items"][0];
        assert_eq!(item["action"], "failed", "{answer:#}");
        assert_eq!(item["error"]["kind"], "no-such-item", "{answer:#}");
        assert!(
            item["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains(&format!("no task with the id {gone}"))),
            "{answer:#}"
        );
        assert_eq!(answer["items"][1]["action"], "updated", "{answer:#}");
    };

    stored.configure(json!({"closure": {"contradictions": [alpha, beta], "removed": [alpha]}}));
    let (answer, _) = stored.json(&["task", "status", "reconcile", "--source", "store"], 4);
    removed(&answer);
    assert_eq!(answer["items"][0]["source"], gone, "{answer:#}");
    assert_eq!(
        answer["items"][1]["source"],
        format!("store:{beta}"),
        "{answer:#}"
    );

    stored.configure(json!({"closure": {"holdings": "due", "removed": [alpha]}}));
    let (answer, _) = stored.json(
        &["task", "status", "complete", "--source", "store"]
            .iter()
            .copied()
            .chain(ISSUES)
            .collect::<Vec<_>>(),
        4,
    );
    removed(&answer);
    assert_eq!(answer["items"][0]["task"], gone, "{answer:#}");
    let targets: Vec<String> = stored
        .status_writes()
        .iter()
        .map(|write| write["target"].as_str().unwrap_or_default().to_owned())
        .collect();
    assert_eq!(
        targets,
        [alpha.as_str(), beta.as_str(), alpha.as_str(), beta.as_str()],
        "each task was asked once per verb"
    );
}

#[test]
fn a_holdings_answer_that_does_not_name_each_reference_once_in_order_fails_every_reference() {
    for (mode, says) in [
        ("missing", "answered 1 holding(s) for 2 reference(s)"),
        ("extra", "answered 3 holding(s) for 2 reference(s)"),
        (
            "reordered",
            "answered acme/work#2 where acme/work#1 was asked about",
        ),
        (
            "renamed",
            "answered acme/other#1 where acme/work#1 was asked about",
        ),
    ] {
        let stored = Stored::new();
        stored.configure(json!({"closure": {"holdings": mode}}));
        let (answer, _) = stored.json(
            &["task", "status", "complete", "--source", "store"]
                .iter()
                .copied()
                .chain(ISSUES)
                .collect::<Vec<_>>(),
            4,
        );
        let items = answer["items"].as_array().expect("items");
        assert_eq!(
            items
                .iter()
                .map(|item| (
                    item["issue"].as_str().unwrap_or_default(),
                    item["action"].as_str().unwrap_or_default()
                ))
                .collect::<Vec<_>>(),
            [("acme/work#1", "failed"), ("acme/work#2", "failed")],
            "{mode}: {answer:#}"
        );
        for item in items {
            assert!(
                item["error"]["message"]
                    .as_str()
                    .is_some_and(|message| message.contains(says)),
                "{mode}: {item:#}"
            );
        }
        assert!(
            stored.status_writes().is_empty(),
            "{mode}: nothing was written"
        );
    }
}

#[test]
fn a_write_check_answered_with_something_other_than_an_object_refuses_the_copy_writing_nothing() {
    let stored = Stored::new();
    let before = std::fs::read_to_string(&stored.at).expect("the store");
    std::fs::write(
        stored.plan.join("tasks/C.md"),
        "---\ntitle: Gamma\nstatus: todo\n---\nGamma.\n",
    )
    .expect("a new task");
    stored.configure(json!({"write_checks": true, "check_answer": "list"}));
    let output = stored.run(&["task", "copy", "--source", "plan", "--to", "store"]);
    assert_eq!(output.status.code(), Some(1), "{}", stdout(&output));
    assert!(
        stderr(&output)
            .contains("answered check_task_write with [], where §4.9b asks for an object"),
        "{}",
        stderr(&output)
    );
    assert_eq!(
        std::fs::read_to_string(&stored.at).expect("the store"),
        before,
        "the store holds exactly what it held"
    );
}
