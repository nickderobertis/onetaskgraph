//! `project status set` on the loopback board, through the binary.
//!
//! A project on a board is an issue holding its tasks as sub-issues — a plan's home card — and
//! its status is two halves, as a task's is: the card's `Status` option and the issue's open or
//! closed state. Every journey reads the board's request log as well as the board, because what
//! the write owes is the paired write a task status takes, for the project kind, and nothing
//! else: no title, no body, no label, no write of a sub-issue.

use std::process::Output;

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{GitHubBoardFields, document, github_projects_with_items};

/// One open issue on the board, at `status`, filed under `parent` when one is named.
fn issue(id: &str, status: &str, parent: Option<&str>) -> Value {
    json!({"item":format!("ITEM-{id}"),"id":id,"type":"Issue","title":format!("Item {id}"),
           "body":format!("{id}, as filed"),"state":"OPEN","reason":null,"parent":parent,
           "repo":"nickderobertis/onetaskgraph","status":status,"origin":"","labels":[]})
}

/// A board holding three plan homes — `H-1` and `H-2` open at `In Progress`, `H-3` closed as
/// completed at `Shipped` — each with one task sub-issue, and a task `C-1` filed under none.
/// The project kind's `done` is mapped to its own `Shipped` option, so a write that lands at it
/// is a write through the project half of the mapping.
struct Board {
    sandbox: Sandbox,
    board: GitHubBoardFields,
}

impl Board {
    fn new() -> Self {
        let sandbox = Sandbox::new();
        let items = vec![
            issue("H-1", "In Progress", None),
            issue("S-1", "Todo", Some("H-1")),
            issue("H-2", "In Progress", None),
            issue("S-2", "Todo", Some("H-2")),
            issue("H-3", "Shipped", None),
            issue("S-3", "Todo", Some("H-3")),
            issue("C-1", "Todo", None),
        ];
        let (mut config, board) = github_projects_with_items(&sandbox, items);
        board.with_status_option("OPT-in-progress", "In Progress");
        board.close("H-3", Some("COMPLETED"));
        config["status_mapping"] = json!({"todo":"Todo","in-progress":"In Progress",
                                          "done":{"task":"Done","project":"Shipped"}});
        sandbox.project_document(&document(&json!({
            "board": {"plugin":"github-projects","config":config},
        })));
        Self { sandbox, board }
    }

    fn run(&self, arguments: &[&str]) -> Output {
        self.sandbox
            .command()
            .args(arguments)
            .assert()
            .get_output()
            .clone()
    }

    /// `project status set <id> <category> --json`, asserting it exits `code`.
    fn set(&self, id: &str, category: &str, code: i32) -> Value {
        let arguments = ["--json", "project", "status", "set", id, category];
        let output = self.run(&arguments);
        assert_eq!(
            output.status.code(),
            Some(code),
            "`onetaskgraph {}`\n{}\n{}",
            arguments.join(" "),
            stdout(&output),
            stderr(&output)
        );
        serde_json::from_str(&stdout(&output)).expect("the command emits JSON")
    }

    /// Every mutation the board was sent, as its root field and the item or issue it names.
    fn mutations(&self) -> Vec<(String, Value)> {
        self.board
            .served()
            .into_iter()
            .filter(|(document, _)| document.trim_start().starts_with("mutation"))
            .map(|(document, variables)| {
                if document.contains("updateIssue(input:") {
                    let input = &variables["input"];
                    let state = input
                        .get("stateInput")
                        .map(|state| json!({"state": state["value"], "reason": state["stateReason"]}))
                        .unwrap_or(Value::Null);
                    (format!("issue {state}"), input["id"].clone())
                } else if document.contains("updateProjectV2ItemFieldValue(input:") {
                    ("option".to_owned(), variables["input"]["itemId"].clone())
                } else {
                    (document, variables)
                }
            })
            .collect()
    }

    /// One item's card and its issue's state and reason.
    fn held(&self, id: &str) -> (Option<String>, (String, Option<String>)) {
        (self.board.status(id), self.board.state(id))
    }
}

#[test]
fn a_terminal_project_status_selects_the_project_kinds_option_and_then_closes_its_issue() {
    let board = Board::new();
    let sub = board.held("S-1");
    let answer = board.set("board:H-1", "done", 0);
    assert_eq!(
        answer,
        json!({"id":"board:H-1","status":{"category":"done","name":"Shipped"}})
    );
    assert_eq!(
        board.held("H-1"),
        (
            Some("Shipped".to_owned()),
            ("CLOSED".to_owned(), Some("COMPLETED".to_owned()))
        ),
        "the card at the project kind's done option, the issue closed as completed"
    );
    assert_eq!(board.held("S-1"), sub, "its sub-issue is not touched");

    let answer = board.set("board:H-2", "cancelled", 0);
    assert_eq!(
        answer,
        json!({"id":"board:H-2","status":{"category":"cancelled","name":"Cancelled"}})
    );
    assert_eq!(
        board.held("H-2"),
        (
            Some("Cancelled".to_owned()),
            ("CLOSED".to_owned(), Some("NOT_PLANNED".to_owned()))
        ),
        "the card at the cancelled option, the issue closed as not planned"
    );
    let closed = |id: &str, reason: &str| {
        (
            format!("issue {}", json!({"state":"CLOSED","reason":reason})),
            json!(id),
        )
    };
    assert_eq!(
        board.mutations(),
        vec![
            ("option".to_owned(), json!("ITEM-H-1")),
            closed("H-1", "COMPLETED"),
            ("option".to_owned(), json!("ITEM-H-2")),
            closed("H-2", "NOT_PLANNED"),
        ],
        "the option first, then the close, and nothing else"
    );
    let shown = board.run(&["--json", "project", "show", "board:H-1"]);
    let shown: Value = serde_json::from_str(&stdout(&shown)).expect("JSON");
    assert_eq!(
        shown["items"][0]["item"]["status"],
        json!({"category":"done","name":"Shipped"}),
        "a later read agrees"
    );
}

#[test]
fn an_open_project_status_reopens_a_closed_home_and_then_selects_its_option() {
    let board = Board::new();
    let answer = board.set("board:H-3", "todo", 0);
    assert_eq!(
        answer,
        json!({"id":"board:H-3","status":{"category":"todo","name":"Todo"}})
    );
    assert_eq!(
        board.held("H-3"),
        (Some("Todo".to_owned()), ("OPEN".to_owned(), None)),
        "reopened, and at the todo option"
    );
    assert_eq!(
        board.mutations(),
        vec![
            (
                format!("issue {}", json!({"state":"OPEN","reason":null})),
                json!("H-3")
            ),
            ("option".to_owned(), json!("ITEM-H-3")),
        ],
        "the reopen first, then the option"
    );

    let human = board.run(&["project", "status", "set", "board:H-1", "done"]);
    assert_eq!(human.status.code(), Some(0), "{}", stderr(&human));
    assert_eq!(
        stdout(&human).trim_end(),
        "project board:H-1 now reads done (Shipped)"
    );
}

#[test]
fn a_task_a_missing_project_and_a_category_the_project_kind_lacks_are_refused_writing_nothing() {
    let board = Board::new();
    let before: Vec<_> = ["H-1", "S-1", "C-1"]
        .iter()
        .map(|id| board.held(id))
        .collect();
    for (id, category, kind, says) in [
        (
            "board:C-1",
            "done",
            "no-such-item",
            "no project with the id board:C-1",
        ),
        (
            "board:H-404",
            "done",
            "no-such-item",
            "no project with the id board:H-404",
        ),
        (
            "board:H-1",
            "draft",
            "refused",
            "source board has no project status name for draft",
        ),
    ] {
        let failure = board.set(id, category, 1);
        assert_eq!(failure["failure"]["kind"], kind, "{id}: {failure:#}");
        let message = failure["failure"]["message"].as_str().unwrap_or_default();
        assert!(
            message.contains(says) && message.contains("next:"),
            "{id} {category}: {failure:#}"
        );
    }
    assert_eq!(board.mutations(), Vec::new(), "nothing was written");
    let after: Vec<_> = ["H-1", "S-1", "C-1"]
        .iter()
        .map(|id| board.held(id))
        .collect();
    assert_eq!(after, before);
}
