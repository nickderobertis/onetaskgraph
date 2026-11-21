//! Spike `spike-store-defects`, defect 1: `task status set <id> cancelled` reports success on an
//! open issue whose card already sits in `Cancelled`, and leaves the issue open.
//!
//! The suspected cause is the no-op shortcut in `set_status`
//! (`crates/onetaskgraph-github-projects/src/lib.rs`): an open issue's category is read from its
//! card alone, so an open issue under a terminal card already "is" the target category and the
//! write returns before the close is sent. These journeys drive the real binary against the
//! loopback board and assert what a person asking for the status expects — the issue closed
//! with the reason the category implies; the open-column case is the control. Which of them
//! fail on which commit is the spike report's to record.
//!
//! Throwaway: kept on the spike branch so the `store-fixes` node can start from a failing
//! reproduction, never landed as is.

use std::process::Output;

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{GitHubBoardFields, document, github_projects_with_items};

/// One task issue on the board, filed under no project, at `status`.
fn issue(id: &str, status: &str) -> Value {
    json!({"item":format!("ITEM-{id}"),"id":id,"type":"Issue","title":format!("Ticket {id}"),
           "body":format!("Ticket {id}, as filed"),"state":"OPEN","reason":null,"parent":null,
           "repo":"nickderobertis/onetaskgraph","status":status,"origin":"","labels":[]})
}

struct Setup {
    sandbox: Sandbox,
    board: GitHubBoardFields,
}

impl Setup {
    fn new(items: Vec<Value>) -> Self {
        let sandbox = Sandbox::new();
        let (mut config, board) = github_projects_with_items(&sandbox, items);
        board.with_status_option("OPT-in-progress", "In Progress");
        // Every category the journeys write names its option, so no shipped default is in play.
        config["status_mapping"] = json!({"backlog":"Backlog","todo":"Todo",
            "in-progress":"In Progress","queued":"Queued","done":"Done","cancelled":"Cancelled"});
        sandbox.project_document(&document(&json!({
            "board": {"plugin":"github-projects","config":config}
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

    /// `task status set board:<id> <category>`, which has to exit 0, answering its JSON.
    fn set(&self, id: &str, category: &str) -> Value {
        let arguments = [
            "--json",
            "task",
            "status",
            "set",
            &format!("board:{id}"),
            category,
        ];
        let output = self.run(&arguments);
        assert_eq!(
            output.status.code(),
            Some(0),
            "`onetaskgraph {}` failed\n{}",
            arguments.join(" "),
            stderr(&output)
        );
        serde_json::from_str(&stdout(&output)).expect("the command emits JSON")
    }

    /// Every mutation the board received, by its root field.
    fn mutations(&self) -> Vec<String> {
        self.board
            .served()
            .into_iter()
            .filter(|(query, _)| query.trim_start().starts_with("mutation"))
            .map(|(query, _)| {
                ["updateProjectV2ItemFieldValue", "updateIssue"]
                    .into_iter()
                    .find(|root| query.contains(&format!("{root}(input:")))
                    .unwrap_or("another mutation")
                    .to_owned()
            })
            .collect()
    }
}

#[test]
fn setting_cancelled_on_an_open_issue_already_under_a_cancelled_card_closes_it_as_not_planned() {
    let setup = Setup::new(vec![issue("S-1", "Cancelled")]);
    let answer = setup.set("S-1", "cancelled");
    eprintln!("answer: {answer:#}\nmutations: {:?}", setup.mutations());
    assert_eq!(
        setup.board.state("S-1"),
        ("CLOSED".to_owned(), Some("NOT_PLANNED".to_owned())),
        "the command answered {answer:#} and sent {:?}, but the issue is still open",
        setup.mutations()
    );
}

#[test]
fn setting_done_on_an_open_issue_already_under_a_done_card_closes_it_as_completed() {
    let setup = Setup::new(vec![issue("S-2", "Done")]);
    let answer = setup.set("S-2", "done");
    eprintln!("answer: {answer:#}\nmutations: {:?}", setup.mutations());
    assert_eq!(
        setup.board.state("S-2"),
        ("CLOSED".to_owned(), Some("COMPLETED".to_owned())),
        "the command answered {answer:#} and sent {:?}, but the issue is still open",
        setup.mutations()
    );
}

#[test]
fn setting_an_open_column_on_a_closed_issue_already_under_that_card_reopens_it() {
    let setup = Setup::new(vec![issue("S-3", "Todo"), issue("S-4", "In Progress")]);
    setup.board.close("S-3", Some("NOT_PLANNED"));
    setup.board.close("S-4", Some("COMPLETED"));
    let todo = setup.set("S-3", "todo");
    let doing = setup.set("S-4", "in-progress");
    eprintln!(
        "answers: {todo:#} {doing:#}\nmutations: {:?}",
        setup.mutations()
    );
    assert_eq!(setup.board.state("S-3").0, "OPEN", "{todo:#}");
    assert_eq!(setup.board.status("S-3").as_deref(), Some("Todo"));
    assert_eq!(setup.board.state("S-4").0, "OPEN", "{doing:#}");
    assert_eq!(setup.board.status("S-4").as_deref(), Some("In Progress"));
}

/// The safety net the plan might have relied on: `task status reconcile` reaches closed items
/// only, so it reports nothing for an open issue under a terminal card and writes nothing.
/// This one asserts today's behaviour, as evidence that reconcile cannot catch defect 1.
#[test]
fn reconcile_does_not_reach_an_open_issue_under_a_terminal_card() {
    let setup = Setup::new(vec![issue("S-5", "Cancelled"), issue("S-6", "Done")]);
    let output = setup.run(&["--json", "task", "status", "reconcile", "--source", "board"]);
    let answer: Value = serde_json::from_str(&stdout(&output)).expect("the command emits JSON");
    eprintln!("reconcile: {answer:#}");
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    // The fixture board's own closed items (`T-2` among them) may be reconciled; neither
    // open issue planted here is listed.
    let listed: Vec<&str> = answer["items"]
        .as_array()
        .expect("a list")
        .iter()
        .filter_map(|item| item["source"].as_str())
        .collect();
    assert!(
        !listed.contains(&"board:S-5") && !listed.contains(&"board:S-6"),
        "{answer:#}"
    );
    assert_eq!(setup.board.state("S-5").0, "OPEN");
    assert_eq!(setup.board.state("S-6").0, "OPEN");
}
