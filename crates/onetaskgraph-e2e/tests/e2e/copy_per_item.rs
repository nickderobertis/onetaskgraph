//! A task copy carries every task past one its destination cannot represent, through the
//! binary.
//!
//! Before anything is written the copy asks the destination about each task it plans, and a
//! task it refuses is answered `failed` with the destination's own reason and left out —
//! exit `4`, as for any failed item — while every other task lands as one copy, still all or
//! nothing among themselves. An edge to the task left out is carried as an edge to an item
//! outside the copy. Every other refusal still takes the whole copy back, and a project copy is
//! still one write, representability included.

use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::{Value, json};

use crate::assets::store;
use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{document, github_projects_with_items};

/// One open task issue filed under no project, its card at `status`, its body `body` with a
/// metadata slot of `slot` — every one carrying the batch this file copies by query.
fn issue(id: &str, title: &str, status: &str, body: &str, slot: Value) -> Value {
    let mut slot = slot;
    slot["caller.batch"] = json!("b-1");
    json!({"item":format!("ITEM-{id}"),"id":id,"type":"Issue","title":title,
           "body":format!("{body}\n\n<!-- onetaskgraph.metadata\n{slot}\n-->"),
           "state":"OPEN","reason":null,"parent":null,
           "repo":"nickderobertis/onetaskgraph","status":status,"origin":"","labels":[]})
}

/// A loopback board holding `left_out` beside `Q-2`, which depends on it, and `Q-3`, and a
/// folder of Markdown `notes` reading `Todo` and `Doing` — and not `Queued`.
struct BoardIntoFolder {
    sandbox: Sandbox,
    notes: PathBuf,
}

impl BoardIntoFolder {
    fn new(left_out: Value) -> Self {
        let sandbox = Sandbox::new();
        let notes = sandbox.subdirectory("notes");
        std::fs::create_dir_all(notes.join("tasks")).expect("the task folder");
        let (config, board) = github_projects_with_items(
            &sandbox,
            vec![
                left_out,
                issue(
                    "Q-2",
                    "Depends on the one left out",
                    "Todo",
                    "Second.",
                    json!({}),
                ),
                issue("Q-3", "Plain", "Todo", "Third.", json!({})),
            ],
        );
        board.block("Q-2", &["Q-1".to_owned()]);
        sandbox.project_document(&document(&json!({
            "board": {"plugin": "github-projects", "config": config},
            "notes": {"plugin": "local-md", "config": {"root": notes,
                "status_mapping": {"Todo": "todo", "Doing": "in-progress"}}},
        })));
        Self { sandbox, notes }
    }

    fn run(&self, arguments: &[&str]) -> Output {
        self.sandbox
            .command()
            .args(arguments)
            .assert()
            .get_output()
            .clone()
    }

    /// The batch copied by query into the folder, which has to exit `4`.
    fn copy(&self) -> Value {
        let output = self.run(&[
            "--json",
            "task",
            "copy",
            "--source",
            "board",
            "--metadata",
            "caller.batch=b-1",
            "--to",
            "notes",
        ]);
        assert_eq!(
            output.status.code(),
            Some(4),
            "{}\n{}",
            stdout(&output),
            stderr(&output)
        );
        assert!(
            stderr(&output).contains("board:Q-1 was not copied"),
            "{}",
            stderr(&output)
        );
        serde_json::from_str(&stdout(&output)).expect("the copy emits JSON")
    }

    /// The folder's task files, by name.
    fn files(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.notes.join("tasks"))
            .expect("the task folder")
            .map(|entry| {
                entry
                    .expect("an entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        names
    }

    /// One copied task's file.
    fn file(&self, name: &str) -> String {
        std::fs::read_to_string(self.notes.join("tasks").join(name)).expect("a task file")
    }
}

/// That the copy left `Q-1` out with the folder's `reason`, wrote the other two, and wrote
/// `Q-2`'s edge to `Q-1` as an edge to an item outside the copy.
fn left_out_alone(case: &BoardIntoFolder, answer: &Value, reason: &str) {
    let items = answer["items"].as_array().expect("items");
    let source = |item: &Value| item["source"].as_str().unwrap_or_default().to_owned();
    let failed: Vec<&Value> = items
        .iter()
        .filter(|item| item["action"] == "failed")
        .collect();
    assert_eq!(failed.len(), 1, "{answer:#}");
    assert_eq!(source(failed[0]), "board:Q-1");
    assert_eq!(failed[0]["error"]["class"], "refused", "{answer:#}");
    assert!(
        failed[0]["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains(reason)),
        "the destination's own reason: {answer:#}"
    );
    let mut created: Vec<String> = items
        .iter()
        .filter(|item| item["action"] == "created")
        .map(source)
        .collect();
    created.sort();
    assert_eq!(created, ["board:Q-2", "board:Q-3"], "{answer:#}");
    assert_eq!(
        case.files(),
        ["Q-2.md", "Q-3.md"],
        "nothing of Q-1 was written"
    );
    let depending = case.file("Q-2.md");
    let edges: Vec<&str> = depending
        .lines()
        .skip_while(|line| !line.starts_with("depends_on:"))
        .skip(1)
        .take_while(|line| line.starts_with("- ") || line.starts_with("  "))
        .filter_map(|line| line.trim_start_matches("- ").trim().strip_prefix("id: "))
        .collect();
    assert_eq!(
        edges,
        ["board:Q-1"],
        "its one edge names the task left out where it is, outside the copy, and no copy of \
         it here, which there is not:\n{depending}"
    );
}

#[test]
fn a_task_whose_status_the_destination_cannot_hold_is_left_out_and_the_rest_are_copied() {
    let case = BoardIntoFolder::new(issue(
        "Q-1",
        "Queued somewhere",
        "Queued",
        "First.",
        json!({}),
    ));
    let answer = case.copy();
    left_out_alone(&case, &answer, "cannot represent the field `status`");
}

#[test]
fn a_task_whose_content_the_destination_cannot_hold_is_left_out_and_the_rest_are_copied() {
    // Content ending in what a folder of Markdown reads as a task's own comments section.
    let content = "First.\n\n## Comments\n\n<!-- onetaskgraph:comment id=\"c-1\" \
                   created_at=\"2026-09-13T15:11:07Z\" updated_at=\"2026-09-13T15:11:07Z\" -->\n\
                   ### comment — 2026-09-13T15:11:07Z\n\nKeep me.\n\n<!-- /onetaskgraph:comment -->";
    let case = BoardIntoFolder::new(issue("Q-1", "Ends in comments", "Todo", content, json!({})));
    let answer = case.copy();
    left_out_alone(&case, &answer, "cannot represent the field `content`");
}

#[test]
fn a_task_whose_delivers_the_destination_cannot_hold_is_left_out_and_the_rest_are_copied() {
    // It delivers the very task the folder would land it as, which a task cannot.
    let case = BoardIntoFolder::new(issue(
        "Q-1",
        "Delivers itself",
        "Todo",
        "First.",
        json!({"onetaskgraph.delivers": ["notes:Q-1"]}),
    ));
    let answer = case.copy();
    left_out_alone(&case, &answer, "cannot represent the field `delivers`");
}

#[test]
fn a_project_copy_holding_a_task_the_destination_cannot_hold_still_writes_nothing() {
    let case = BoardIntoFolder::new(issue(
        "Q-1",
        "Queued somewhere",
        "Queued",
        "First.",
        json!({}),
    ));
    let projects = case.notes.join("projects");
    std::fs::create_dir_all(&projects).expect("the project folder");
    // `P-1` holds `T-2`, closed as completed in `Shipped`, which this folder cannot read as done.
    let output = case.run(&["project", "copy", "board:P-1", "--to", "notes"]);
    assert_eq!(output.status.code(), Some(1), "{}", stdout(&output));
    assert!(
        stderr(&output).contains("cannot represent the field `status`"),
        "{}",
        stderr(&output)
    );
    assert!(case.files().is_empty(), "no task was written");
    assert!(
        std::fs::read_dir(&projects)
            .expect("the project folder")
            .next()
            .is_none(),
        "no project was written"
    );
}

/// A folder of Markdown `plan` holding `A`, `B` and `Z`, copied into the Python store over a real
/// pipe with `settings`; answers the copy's output and the lines the store logged.
fn into_the_store(settings: Value) -> (Sandbox, Output, Vec<Value>, PathBuf) {
    let into = PlanAndStore::new(&[], settings);
    let output = into.copy();
    let logged = logged(&into.log);
    (into.sandbox, output, logged, into.at)
}

/// A folder of Markdown `plan` holding `A` (`Alpha`), `B` (`Beta`) and `Z` (`Zeta`) — each with
/// the front matter `extra` gives it — beside the Python store, configured with `settings`.
struct PlanAndStore {
    sandbox: Sandbox,
    plan: PathBuf,
    at: PathBuf,
    log: PathBuf,
}

impl PlanAndStore {
    fn new(extra: &[(&str, &str)], settings: Value) -> Self {
        let sandbox = Sandbox::new();
        let plan = sandbox.subdirectory("plan");
        std::fs::create_dir_all(plan.join("tasks")).expect("the task folder");
        for (id, title) in [("A", "Alpha"), ("B", "Beta"), ("Z", "Zeta")] {
            let more = extra
                .iter()
                .filter(|(of, _)| *of == id)
                .map(|(_, line)| format!("{line}\n"))
                .collect::<String>();
            std::fs::write(
                plan.join(format!("tasks/{id}.md")),
                format!("---\ntitle: {title}\nstatus: todo\n{more}---\n{title}.\n"),
            )
            .expect("a task");
        }
        let at = sandbox.subdirectory("store").join("store.json");
        let log = sandbox.subdirectory("store").join("log.jsonl");
        let into = Self {
            sandbox,
            plan,
            at,
            log,
        };
        into.configure(settings);
        into
    }

    /// Configure the store with `settings`, from here on.
    fn configure(&self, settings: Value) {
        self.sandbox.project_document(&document(&json!({
            "plan": {"plugin": "local-md", "config": {"root": self.plan}},
            "store": store(&self.at, &self.log, settings),
        })));
    }

    /// Every task of the folder copied by query into the store.
    fn copy(&self) -> Output {
        self.sandbox
            .command()
            .args([
                "--json", "task", "copy", "--source", "plan", "--to", "store",
            ])
            .assert()
            .get_output()
            .clone()
    }

    /// The task the store holds under `title`, as it holds it.
    fn held_task(&self, title: &str) -> Value {
        let held: Value =
            serde_json::from_str(&std::fs::read_to_string(&self.at).expect("the store"))
                .expect("the store");
        held["tasks"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|task| task["title"] == title)
            .cloned()
            .unwrap_or_else(|| panic!("the store holds {title}: {held:#}"))
    }
}

/// Every line the store logged, in order.
fn logged(log: &Path) -> Vec<Value> {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).expect("a logged line"))
        .collect()
}

/// The titles of the tasks the store holds.
fn held(at: &Path) -> Vec<String> {
    let held: Value =
        serde_json::from_str(&std::fs::read_to_string(at).unwrap_or_else(|_| "{}".into()))
            .expect("the store");
    let mut titles: Vec<String> = held["tasks"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|task| task["title"].as_str().unwrap_or_default().to_owned())
        .collect();
    titles.sort();
    titles
}

#[test]
fn every_task_is_asked_about_before_the_first_write_and_the_one_refused_is_left_out() {
    let (_sandbox, output, logged, at) =
        into_the_store(json!({"write_checks": true, "unrepresentable": ["Zeta"]}));
    assert_eq!(output.status.code(), Some(4), "{}", stderr(&output));
    let methods: Vec<String> = logged
        .iter()
        .map(|line| line["method"].as_str().unwrap_or_default().to_owned())
        .collect();
    let first_write = methods
        .iter()
        .position(|method| method == "write_task")
        .expect("a task was written");
    let checks: Vec<&Value> = logged
        .iter()
        .filter(|line| line["method"] == "check_task_write")
        .collect();
    assert_eq!(
        checks
            .iter()
            .map(|line| line["title"].as_str().unwrap_or_default())
            .collect::<Vec<_>>(),
        ["Alpha", "Beta", "Zeta"],
        "each planned task asked about once, in query order: {methods:?}"
    );
    assert!(
        methods[..first_write]
            .iter()
            .filter(|method| *method == "check_task_write")
            .count()
            == 3,
        "every check came before the first write: {methods:?}"
    );
    let answer: Value = serde_json::from_str(&stdout(&output)).expect("JSON");
    assert_eq!(answer["items"][2]["source"], "plan:Z");
    assert_eq!(answer["items"][2]["action"], "failed");
    assert!(
        answer["items"][2]["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("cannot represent the field `status` of Zeta")),
        "{answer:#}"
    );
    assert_eq!(held(&at), ["Alpha", "Beta"]);
}

#[test]
fn a_task_accepted_at_the_check_and_refused_at_its_write_takes_the_whole_copy_back() {
    let (_sandbox, output, logged, at) =
        into_the_store(json!({"write_checks": true, "refused_at_write": ["Zeta"]}));
    assert_eq!(output.status.code(), Some(1), "{}", stdout(&output));
    assert!(
        stderr(&output).contains("refused at its write"),
        "{}",
        stderr(&output)
    );
    assert!(
        logged.iter().any(|line| line["method"] == "delete_task"),
        "what landed was taken back: {logged:?}"
    );
    assert!(
        held(&at).is_empty(),
        "the store holds none of the copy's tasks"
    );
}

#[test]
fn a_refusal_of_another_kind_at_the_check_writes_nothing() {
    let (_sandbox, output, logged, at) =
        into_the_store(json!({"write_checks": true, "check_unavailable": ["Zeta"]}));
    assert_eq!(output.status.code(), Some(1), "{}", stdout(&output));
    assert!(
        stderr(&output).contains("the store is not answering for Zeta"),
        "{}",
        stderr(&output)
    );
    assert!(
        !logged.iter().any(|line| line["method"] == "write_task"),
        "nothing was written: {logged:?}"
    );
    assert!(held(&at).is_empty());
}

/// The sources and actions a copy's machine output answers, in order.
fn actions(output: &Output) -> Vec<(String, String)> {
    let answer: Value = serde_json::from_str(&stdout(output)).expect("JSON");
    answer["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|item| {
            (
                item["source"].as_str().unwrap_or_default().to_owned(),
                item["action"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect()
}

#[test]
fn two_tasks_the_destination_refuses_are_both_left_out_and_the_rest_are_copied() {
    let (_sandbox, output, _logged, at) =
        into_the_store(json!({"write_checks": true, "unrepresentable": ["Alpha", "Zeta"]}));
    assert_eq!(output.status.code(), Some(4), "{}", stderr(&output));
    assert_eq!(
        actions(&output),
        [
            ("plan:A".to_owned(), "failed".to_owned()),
            ("plan:B".to_owned(), "created".to_owned()),
            ("plan:Z".to_owned(), "failed".to_owned()),
        ]
    );
    assert!(
        stderr(&output).contains("plan:A was not copied")
            && stderr(&output).contains("plan:Z was not copied"),
        "{}",
        stderr(&output)
    );
    assert_eq!(held(&at), ["Beta"]);
}

#[test]
fn a_counterpart_the_destination_refuses_to_update_is_left_as_it_was() {
    let into = PlanAndStore::new(&[], json!({"write_checks": true}));
    let first = into.copy();
    assert_eq!(first.status.code(), Some(0), "{}", stderr(&first));
    let before = into.held_task("Zeta");
    // Each task changes at its source; the store then refuses Zeta's new form.
    for (id, title) in [("A", "Alpha"), ("B", "Beta"), ("Z", "Zeta")] {
        let path = into.plan.join(format!("tasks/{id}.md"));
        let text = std::fs::read_to_string(&path).expect("a task");
        std::fs::write(&path, text.replace(&format!("{title}.\n"), "Edited.\n")).expect("an edit");
    }
    into.configure(json!({"write_checks": true, "unrepresentable": ["Zeta"]}));
    let second = into.copy();
    assert_eq!(second.status.code(), Some(4), "{}", stderr(&second));
    assert_eq!(
        actions(&second),
        [
            ("plan:A".to_owned(), "updated".to_owned()),
            ("plan:B".to_owned(), "updated".to_owned()),
            ("plan:Z".to_owned(), "failed".to_owned()),
        ]
    );
    assert_eq!(
        into.held_task("Zeta"),
        before,
        "the refused task's counterpart is exactly as the first copy left it"
    );
    assert!(
        into.held_task("Alpha")["content"]
            .as_str()
            .is_some_and(|content| content.starts_with("Edited.")),
        "{:#}",
        into.held_task("Alpha")
    );
}

#[test]
fn a_task_delivering_one_the_destination_refuses_carries_it_as_a_task_outside_the_copy() {
    // `A` delivers `Z`, which is in progress — a status the folder copied into has no name for.
    let sandbox = Sandbox::new();
    let plan = sandbox.subdirectory("plan");
    let notes = sandbox.subdirectory("notes");
    for root in [&plan, &notes] {
        std::fs::create_dir_all(root.join("tasks")).expect("a task folder");
    }
    for (id, front) in [
        ("A", "title: Alpha\nstatus: todo\ndelivers: [Z]"),
        ("B", "title: Beta\nstatus: todo"),
        (
            "Z",
            "title: Zeta\nstatus: in progress\ndelivered_by: [plan:A]",
        ),
    ] {
        std::fs::write(
            plan.join(format!("tasks/{id}.md")),
            format!("---\n{front}\n---\n{id}.\n"),
        )
        .expect("a task");
    }
    sandbox.project_document(&document(&json!({
        "plan": {"plugin": "local-md", "config": {"root": plan}},
        "notes": {"plugin": "local-md", "config": {"root": notes,
            "status_mapping": {"Todo": "todo", "Done": "done"}}},
    })));
    let output = sandbox
        .command()
        .args([
            "--json", "task", "copy", "--source", "plan", "--to", "notes",
        ])
        .assert()
        .get_output()
        .clone();
    assert_eq!(output.status.code(), Some(4), "{}", stderr(&output));
    assert_eq!(
        actions(&output),
        [
            ("plan:A".to_owned(), "created".to_owned()),
            ("plan:B".to_owned(), "created".to_owned()),
            ("plan:Z".to_owned(), "failed".to_owned()),
        ]
    );
    let shown = sandbox
        .command()
        .args(["--json", "task", "list", "--source", "notes"])
        .assert()
        .get_output()
        .clone();
    let listed: Value = serde_json::from_str(&stdout(&shown)).expect("JSON");
    let alpha = listed["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|item| item["item"]["title"] == "Alpha")
        .unwrap_or_else(|| panic!("Alpha landed: {listed:#}"));
    assert_eq!(
        alpha["item"]["delivers"],
        json!(["plan:Z"]),
        "the entry names the task left out where it is, and no copy of it, which there is \
         not: {alpha:#}"
    );
    assert_eq!(
        std::fs::read_dir(notes.join("tasks"))
            .expect("the folder")
            .count(),
        2,
        "nothing of Zeta was written"
    );
}

#[test]
fn a_dry_run_reports_the_task_the_destination_refuses_failed_and_writes_nothing() {
    let into = PlanAndStore::new(
        &[],
        json!({"write_checks": true, "unrepresentable": ["Zeta"]}),
    );
    let output = into
        .sandbox
        .command()
        .args([
            "--json",
            "task",
            "copy",
            "--source",
            "plan",
            "--to",
            "store",
            "--dry-run",
        ])
        .assert()
        .get_output()
        .clone();
    assert_eq!(output.status.code(), Some(4), "{}", stderr(&output));
    assert_eq!(
        actions(&output),
        [
            ("plan:A".to_owned(), "created".to_owned()),
            ("plan:B".to_owned(), "created".to_owned()),
            ("plan:Z".to_owned(), "failed".to_owned()),
        ]
    );
    assert!(
        stderr(&output).contains("plan:Z was not copied"),
        "{}",
        stderr(&output)
    );
    let answer: Value = serde_json::from_str(&stdout(&output)).expect("JSON");
    for at in [0, 1] {
        assert_eq!(
            answer["items"][at]["destination"],
            Value::Null,
            "a dry run lands nowhere: {answer:#}"
        );
    }
    assert!(
        logged(&into.log)
            .iter()
            .all(|line| line["method"] == "check_task_write"),
        "a dry run asks the checks and writes nothing: {:?}",
        logged(&into.log)
    );
    assert!(held(&into.at).is_empty(), "the store holds nothing");
}
