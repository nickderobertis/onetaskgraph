//! Dropping a task from a plan copied onto a GitHub board: `task delete` at the source, then
//! `project copy --close-orphans` closing the board issue the dropped task left behind as not
//! planned — against the loopback fixture board, counted on every request it was sent and on
//! every item it holds.

use std::collections::BTreeMap;
use std::path::PathBuf;

use onetaskgraph_github_projects::graphql;
use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{GitHubBoardFields, document, github_projects_with_board};

/// A folder of Markdown holding project `P` and four tasks, beside the fixture board it is
/// copied into.
struct Plan {
    sandbox: Sandbox,
    board: GitHubBoardFields,
}

/// The plan's tasks: `stay` remains in it, and the other three are dropped from it once copied —
/// `open` left as the copy wrote it, `finished` moved to done and `abandoned` to cancelled on the
/// board first.
const TASKS: [&str; 4] = ["stay", "open", "finished", "abandoned"];

impl Plan {
    fn new() -> Self {
        let sandbox = Sandbox::new();
        let root: PathBuf = sandbox.subdirectory("plans");
        std::fs::create_dir_all(root.join("projects")).expect("the project folder");
        std::fs::create_dir_all(root.join("tasks")).expect("the task folder");
        std::fs::write(root.join("projects/P.md"), "---\ntitle: The plan\n---\n").expect("P");
        for task in TASKS {
            std::fs::write(
                root.join(format!("tasks/{task}.md")),
                format!(
                    "---\ntitle: Task {task}\nstatus: todo\nproject: P\n---\nThe {task} task.\n"
                ),
            )
            .expect("a task");
        }
        let (config, board) = github_projects_with_board(&sandbox);
        sandbox.project_document(&document(&json!({
            "plans": {"plugin": "local-md", "config": {"root": root}},
            "board": {"plugin": "github-projects", "config": config}
        })));
        Self { sandbox, board }
    }

    /// One command, which had to exit `code`: what it printed, and every request the board
    /// served for it.
    fn run(&self, arguments: &[&str], code: i32) -> (String, String, Vec<(String, Value)>) {
        let before = self.board.served().len();
        let output = self
            .sandbox
            .command()
            .args(arguments)
            .assert()
            .get_output()
            .clone();
        assert_eq!(
            output.status.code(),
            Some(code),
            "`onetaskgraph {}`\n{}{}",
            arguments.join(" "),
            stdout(&output),
            stderr(&output)
        );
        let served = self.board.served()[before..].to_vec();
        (stdout(&output), stderr(&output), served)
    }

    fn json(&self, arguments: &[&str]) -> (Value, Vec<(String, Value)>) {
        let (out, _, served) = self.run(arguments, 0);
        (
            serde_json::from_str(&out).expect("a command emits JSON"),
            served,
        )
    }

    /// Every item the board holds for the plan, by its board id, exactly as it holds it.
    fn held(&self, at: &BTreeMap<String, String>) -> BTreeMap<String, Value> {
        at.values()
            .map(|native| {
                (
                    native.clone(),
                    self.board.item(native).expect("the board holds the item"),
                )
            })
            .collect()
    }
}

/// The mutations of the board's documents: any of them sent is a write.
fn writes(document: &str) -> bool {
    [
        graphql::UPDATE_ISSUE,
        graphql::UPDATE_DRAFT,
        graphql::UPDATE_FIELD,
        graphql::UPDATE_FIELDS,
        graphql::WRITE_FIELD_VALUES,
        graphql::CLEAR_FIELD,
        graphql::CLOSE_ISSUES,
        graphql::UPDATE_ISSUES,
        graphql::DELETE_ISSUE,
        graphql::CREATE_ISSUE,
        graphql::CREATE_ISSUES,
    ]
    .contains(&document)
}

/// What a report says of each item, by its source id: its action and its destination.
fn said(report: &Value) -> BTreeMap<String, (String, Option<String>)> {
    report["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|item| {
            (
                item["source"].as_str().expect("a source").to_owned(),
                (
                    item["action"].as_str().expect("an action").to_owned(),
                    item["destination"].as_str().map(ToOwned::to_owned),
                ),
            )
        })
        .collect()
}

/// The plan copied onto the board, its `finished` and `abandoned` copies moved to done and
/// cancelled there, and every task but `stay` deleted at the source: the board id of each
/// record's counterpart, by its id in the folder.
fn dropped() -> (Plan, BTreeMap<String, String>) {
    let plan = Plan::new();
    let (copied, _) = plan.json(&["project", "copy", "plans:P", "--to", "board", "--json"]);
    let at: BTreeMap<String, String> = said(&copied)
        .into_iter()
        .map(|(source, (action, destination))| {
            assert_eq!(action, "created", "{copied:#}");
            (
                source.trim_start_matches("plans:").to_owned(),
                destination
                    .expect("a destination")
                    .trim_start_matches("board:")
                    .to_owned(),
            )
        })
        .collect();
    plan.run(
        &[
            "task",
            "status",
            "set",
            &format!("board:{}", at["finished"]),
            "done",
        ],
        0,
    );
    plan.run(
        &[
            "task",
            "status",
            "set",
            &format!("board:{}", at["abandoned"]),
            "cancelled",
        ],
        0,
    );
    for task in ["open", "finished", "abandoned"] {
        let (out, _, served) = plan.run(&["task", "delete", &format!("plans:{task}")], 0);
        assert_eq!(out, format!("deleted task plans:{task}\n"));
        assert!(
            served.is_empty(),
            "a delete at the source sent the board {served:?}"
        );
    }
    (plan, at)
}

#[test]
fn close_orphans_closes_the_dropped_tasks_open_issue_as_not_planned_and_nothing_else() {
    let (plan, at) = dropped();
    let before = plan.held(&at);
    let finished = &at["finished"];
    let abandoned = &at["abandoned"];
    assert_eq!(before[finished]["state"], "CLOSED");
    assert_eq!(before[abandoned]["reason"], "NOT_PLANNED");
    let expect = |open: &str| -> BTreeMap<String, (String, Option<String>)> {
        [
            ("P", "unchanged"),
            ("stay", "unchanged"),
            ("open", open),
            ("finished", "orphaned"),
            ("abandoned", "orphaned"),
        ]
        .into_iter()
        .map(|(source, action)| {
            (
                format!("plans:{source}"),
                (action.to_owned(), Some(format!("board:{}", at[source]))),
            )
        })
        .collect()
    };
    let copy = ["project", "copy", "plans:P", "--to", "board", "--json"];

    // Without the flag, every orphan is reported and left as it was, and nothing is written.
    let (flagless, served) = plan.json(&copy);
    assert_eq!(said(&flagless), expect("orphaned"));
    assert!(
        !served.iter().any(|(document, _)| writes(document)),
        "a copy without --close-orphans wrote: {served:?}"
    );
    assert_eq!(plan.held(&at), before);

    // A dry run reports the close and writes nothing.
    let closing = [&copy[..], &["--close-orphans"]].concat();
    let (dry, served) = plan.json(&[&closing[..], &["--dry-run"]].concat());
    assert_eq!(said(&dry), expect("closed"));
    assert!(
        !served.iter().any(|(document, _)| writes(document)),
        "a dry run wrote: {served:?}"
    );
    assert_eq!(plan.held(&at), before);

    // The copy closes the open orphan alone: the card at `Cancelled` and the issue closed as
    // not planned, as `task status set … cancelled` leaves it.
    let (report, served) = plan.json(&closing);
    assert_eq!(said(&report), expect("closed"));
    let open = &at["open"];
    let closed = report["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["source"] == "plans:open")
        .unwrap();
    assert_eq!(
        closed,
        &json!({"source": "plans:open", "action": "closed", "destination": format!("board:{open}")})
    );
    let after = plan.held(&at);
    assert_eq!(after[open]["state"], "CLOSED", "{:#}", after[open]);
    assert_eq!(after[open]["reason"], "NOT_PLANNED", "{:#}", after[open]);
    assert_eq!(after[open]["status"], "Cancelled", "{:#}", after[open]);
    assert_eq!(after[open]["title"], before[open]["title"]);
    assert_eq!(after[open]["body"], before[open]["body"]);
    for (native, item) in &before {
        if native != open {
            assert_eq!(&after[native], item, "{native} changed");
        }
    }
    let written: Vec<&Value> = served
        .iter()
        .filter(|(document, _)| writes(document))
        .map(|(_, variables)| variables)
        .collect();
    assert!(!written.is_empty(), "the close wrote nothing");
    assert!(
        written
            .iter()
            .all(|variables| variables.to_string().contains(open.as_str())),
        "a write named another item than the one closed: {written:?}"
    );
    let (shown, _) = plan.json(&["task", "show", &format!("board:{open}"), "--json"]);
    assert_eq!(
        shown["items"][0]["item"]["status"],
        json!({"category": "cancelled", "name": "Cancelled"})
    );

    // Closed now, it is an orphan like the others, and a second close writes nothing.
    let (again, served) = plan.json(&closing);
    assert_eq!(said(&again), expect("orphaned"));
    assert!(!served.iter().any(|(document, _)| writes(document)));
    assert_eq!(plan.held(&at), after);
}

#[test]
fn close_orphans_beside_a_copy_that_does_not_walk_every_task_is_refused_before_anything_is_read() {
    let (plan, at) = dropped();
    let before = plan.held(&at);
    for narrowing in [
        &["--member", "plans:stay"][..],
        &["--member", "plans:stay", "--members-only"][..],
        &["--no-tasks"][..],
    ] {
        let arguments = [
            &[
                "project",
                "copy",
                "plans:P",
                "--to",
                "board",
                "--close-orphans",
            ][..],
            narrowing,
        ]
        .concat();
        let (out, err, served) = plan.run(&arguments, 2);
        assert_eq!(out, "", "{arguments:?}");
        assert!(
            err.contains("'--close-orphans' cannot be used with"),
            "{arguments:?}: {err}"
        );
        assert!(served.is_empty(), "{arguments:?} sent the board {served:?}");
    }
    assert_eq!(plan.held(&at), before);
}
