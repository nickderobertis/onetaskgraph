//! `task status reconcile` against the loopback board, through the binary.
//!
//! An on-demand tool: a person runs it to bring a board's cards in line after closures made
//! outside the stack. Every journey reads the board's request log, because what the verb owes is
//! as much what it does not send — no read of the board's unfiltered `items`, no write of an
//! issue's state — as what it writes.

use std::process::Output;
use std::time::Duration;

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{GitHubBoardFields, document, github_projects_with_items};

/// One item as the board holds it: its id, its card's option, and its issue's state and reason.
type Held = (String, Option<String>, (String, Option<String>));

/// One open task issue on the board, filed under no project, its card at `status`.
fn issue(id: &str, status: Option<&str>) -> Value {
    json!({"item":format!("ITEM-{id}"),"id":id,"type":"Issue","title":format!("Task {id}"),
           "body":format!("{id}, as filed"),"state":"OPEN","reason":null,"parent":null,
           "repo":"nickderobertis/onetaskgraph","status":status,"origin":"","labels":[]})
}

/// A board configured as `board` with `Proposal`, `Todo`, `Queued`, `In Progress`, `Done` and
/// `Cancelled` mapped, holding `items` — each closed afterwards as `closures` says, on the
/// fixture, the way GitHub records a closure.
struct Setup {
    sandbox: Sandbox,
    board: GitHubBoardFields,
}

impl Setup {
    fn new(items: Vec<Value>, closures: &[(&str, Option<&str>)], pacing_ms: u64) -> Self {
        let sandbox = Sandbox::new();
        let (mut config, board) = github_projects_with_items(&sandbox, items);
        board.with_status_option("OPT-in-progress", "In Progress");
        board.with_status_option("OPT-proposal", "Proposal");
        // The shared dataset's one closed task sits in an option this mapping does not name;
        // reopened, so every closed item on this board is one a journey here put there.
        board.reopen("T-2");
        for (id, reason) in closures {
            board.close(id, *reason);
        }
        config["status_mapping"] =
            json!({"backlog":"Proposal","todo":"Todo","in-progress":"In Progress"});
        config["pacing"] = json!({"min_mutation_interval_ms": pacing_ms});
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

    /// The machine output of `arguments`, asserting it exits `code`.
    fn json(&self, arguments: &[&str], code: i32) -> Value {
        let output = self.run(arguments);
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

    /// Every item's card and its issue's state, by id.
    fn snapshot(&self, ids: &[&str]) -> Vec<Held> {
        ids.iter()
            .map(|id| {
                (
                    (*id).to_owned(),
                    self.board.status(id),
                    self.board.state(id),
                )
            })
            .collect()
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

    /// Every board read of `items` this run sent, each with the filter it carried — and that
    /// it sent no read of the board's unfiltered `items` at all.
    fn item_reads_are_closed_filtered(&self) {
        let reads: Vec<(String, Value)> = self
            .board
            .served()
            .into_iter()
            .filter(|(query, _)| query.contains("items(first:"))
            .collect();
        assert!(!reads.is_empty(), "the reconcile read the board");
        for (query, variables) in &reads {
            assert_eq!(
                query.as_str(),
                onetaskgraph_github_projects::graphql::CLOSED_ITEMS,
                "every board read of items is the closed-filtered one"
            );
            assert_eq!(variables["filter"], "is:closed", "{variables}");
        }
        assert!(
            self.board.board_item_reads().is_empty(),
            "no read of the board's unfiltered items"
        );
    }
}

/// The ids of a reconcile's answer, and what each came to.
fn listed(answer: &Value) -> Vec<(String, Value, Value, String)> {
    answer["items"]
        .as_array()
        .unwrap_or_else(|| panic!("an items list: {answer:#}"))
        .iter()
        .map(|item| {
            (
                item["source"].as_str().expect("a qualified id").to_owned(),
                item["from"].clone(),
                item["to"].clone(),
                item["action"].as_str().expect("an action").to_owned(),
            )
        })
        .collect()
}

/// The board every reconcile journey below reads: open items, closed items in line, closed
/// items at contradicting options, a closed item with no option, and one closed for a reason
/// that implies no category (`R-11`), which is never listed.
fn mixed() -> (Vec<Value>, Vec<(&'static str, Option<&'static str>)>) {
    (
        vec![
            issue("R-1", Some("Todo")),
            issue("R-2", Some("In Progress")),
            issue("R-3", Some("Done")),
            issue("R-4", Some("Cancelled")),
            issue("R-5", Some("Todo")),
            issue("R-6", Some("In Progress")),
            issue("R-7", Some("Queued")),
            issue("R-8", Some("Proposal")),
            issue("R-9", Some("Proposal")),
            issue("R-10", None),
            issue("R-11", Some("Todo")),
        ],
        vec![
            ("R-3", Some("COMPLETED")),
            ("R-4", Some("NOT_PLANNED")),
            ("R-5", Some("COMPLETED")),
            ("R-6", Some("COMPLETED")),
            ("R-7", Some("COMPLETED")),
            ("R-8", Some("NOT_PLANNED")),
            ("R-9", Some("DUPLICATE")),
            ("R-10", Some("COMPLETED")),
            ("R-11", Some("REOPENED")),
        ],
    )
}

const ALL: [&str; 11] = [
    "R-1", "R-2", "R-3", "R-4", "R-5", "R-6", "R-7", "R-8", "R-9", "R-10", "R-11",
];

/// What every reconcile of [`mixed`] owes: these items, from these options, to these statuses.
fn expected(action: &str) -> Vec<(String, Value, Value, String)> {
    [
        ("R-5", json!("Todo"), "done", "Done"),
        ("R-6", json!("In Progress"), "done", "Done"),
        ("R-7", json!("Queued"), "done", "Done"),
        ("R-8", json!("Proposal"), "cancelled", "Cancelled"),
        ("R-9", json!("Proposal"), "cancelled", "Cancelled"),
        ("R-10", Value::Null, "done", "Done"),
    ]
    .into_iter()
    .map(|(id, from, category, name)| {
        (
            format!("board:{id}"),
            from,
            json!({"category":category,"name":name}),
            action.to_owned(),
        )
    })
    .collect()
}

#[test]
fn a_reconcile_moves_exactly_each_contradicting_card_to_the_option_its_closure_implies() {
    let (items, closures) = mixed();
    let setup = Setup::new(items, &closures, 0);
    let before = setup.snapshot(&ALL);

    let answer = setup.json(
        &["--json", "task", "status", "reconcile", "--source", "board"],
        0,
    );
    assert_eq!(listed(&answer), expected("updated"), "{answer:#}");
    assert!(
        answer["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item.get("error").is_none()),
        "{answer:#}"
    );

    for (id, card, state) in &before {
        let wanted = match id.as_str() {
            "R-5" | "R-6" | "R-7" | "R-10" => Some("Done".to_owned()),
            "R-8" | "R-9" => Some("Cancelled".to_owned()),
            _ => card.clone(),
        };
        assert_eq!(setup.board.status(id), wanted, "{id}'s card");
        assert_eq!(
            &setup.board.state(id),
            state,
            "{id}'s issue state and reason"
        );
    }
    assert_eq!(
        setup.mutations(),
        vec!["updateProjectV2ItemFieldValue"; 6],
        "one card write per contradicting item, and no issue written"
    );
    setup.item_reads_are_closed_filtered();

    // Nothing is left out of line, so a second run lists nothing and writes nothing.
    let again = setup.json(
        &["--json", "task", "status", "reconcile", "--source", "board"],
        0,
    );
    assert_eq!(again, json!({"items": []}));
    assert_eq!(setup.mutations().len(), 6);

    let human = setup.run(&["task", "status", "reconcile", "--source", "board"]);
    assert!(
        stdout(&human).contains("0 updated, 0 would update, 0 failed"),
        "{}",
        stdout(&human)
    );
}

#[test]
fn a_dry_run_lists_the_same_items_as_would_update_and_leaves_the_board_unchanged() {
    let (items, closures) = mixed();
    let setup = Setup::new(items, &closures, 0);
    let before = setup.snapshot(&ALL);
    let answer = setup.json(
        &[
            "--json",
            "task",
            "status",
            "reconcile",
            "--source",
            "board",
            "--dry-run",
        ],
        0,
    );
    assert_eq!(listed(&answer), expected("would-update"), "{answer:#}");
    assert_eq!(setup.snapshot(&ALL), before);
    assert!(setup.mutations().is_empty(), "a dry run writes nothing");
    setup.item_reads_are_closed_filtered();

    let human = setup.run(&[
        "task",
        "status",
        "reconcile",
        "--source",
        "board",
        "--dry-run",
    ]);
    let said = stdout(&human);
    assert!(
        said.contains("board:R-5: Todo -> Done (done): would-update")
            && said.contains("board:R-10: no option -> Done (done): would-update")
            && said.contains("0 updated, 6 would update, 0 failed"),
        "{said}"
    );
}

#[test]
fn an_item_whose_write_is_refused_is_listed_failed_and_the_others_are_written() {
    let (items, closures) = mixed();
    let setup = Setup::new(items, &closures, 0);
    setup
        .board
        .refuse_once("updateProjectV2ItemFieldValue(input:$input)");
    let answer = setup.json(
        &["--json", "task", "status", "reconcile", "--source", "board"],
        4,
    );
    let mut wanted = expected("updated");
    wanted[0].3 = "failed".to_owned();
    assert_eq!(listed(&answer), wanted, "{answer:#}");
    let failed = &answer["items"][0];
    assert_eq!(failed["error"]["class"], "refused", "{failed:#}");
    assert!(
        failed["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("updateProjectV2ItemFieldValue")),
        "{failed:#}"
    );
    assert_eq!(setup.board.status("R-5").as_deref(), Some("Todo"));
    for id in ["R-6", "R-7", "R-10"] {
        assert_eq!(setup.board.status(id).as_deref(), Some("Done"), "{id}");
    }

    // The same refusal as a person reads it: the report on stdout, the failure on stderr.
    let (items, closures) = mixed();
    let human = Setup::new(items, &closures, 0);
    human
        .board
        .refuse_once("updateProjectV2ItemFieldValue(input:$input)");
    let output = human.run(&["task", "status", "reconcile", "--source", "board"]);
    assert_eq!(output.status.code(), Some(4), "{}", stderr(&output));
    let said = stdout(&output);
    assert!(
        said.contains("board:R-5: Todo -> Done (done): failed — ")
            && said.contains("board:R-6: In Progress -> Done (done): updated")
            && said.contains("board:R-8: Proposal -> Cancelled (cancelled): updated")
            && said.contains("5 updated, 0 would update, 1 failed"),
        "{said}"
    );
    assert!(
        stderr(&output).contains("onetaskgraph: board:R-5 was not reconciled: ")
            && stderr(&output).contains("updateProjectV2ItemFieldValue"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn a_closed_filter_that_lets_open_items_through_writes_and_lists_only_the_closed_ones() {
    let (items, closures) = mixed();
    let setup = Setup::new(items, &closures, 0);
    setup.board.leak_open_items_through_the_closed_filter();
    let answer = setup.json(
        &["--json", "task", "status", "reconcile", "--source", "board"],
        0,
    );
    assert_eq!(listed(&answer), expected("updated"), "{answer:#}");
    for id in ["R-1", "R-2", "T-1", "T-3", "T-4", "P-1", "P-2"] {
        assert_eq!(
            setup.board.state(id).0,
            "OPEN",
            "{id} stays open, its card untouched"
        );
    }
    assert_eq!(setup.board.status("R-1").as_deref(), Some("Todo"));
    assert_eq!(setup.board.status("R-2").as_deref(), Some("In Progress"));
    assert_eq!(setup.mutations().len(), 6);
}

#[test]
fn a_reconcile_paces_every_card_write_by_the_sources_configured_interval() {
    let ids: Vec<String> = (1..=20).map(|n| format!("R-{n}")).collect();
    let items = ids.iter().map(|id| issue(id, Some("Todo"))).collect();
    let closures: Vec<(&str, Option<&str>)> = ids
        .iter()
        .map(|id| (id.as_str(), Some("COMPLETED")))
        .collect();
    let interval = Duration::from_millis(150);
    let setup = Setup::new(
        items,
        &closures,
        u64::try_from(interval.as_millis()).unwrap(),
    );
    let answer = setup.json(
        &["--json", "task", "status", "reconcile", "--source", "board"],
        0,
    );
    assert_eq!(answer["items"].as_array().map(Vec::len), Some(20));
    for id in &ids {
        assert_eq!(setup.board.status(id).as_deref(), Some("Done"), "{id}");
    }
    let writes: Vec<std::time::Instant> = setup
        .board
        .served_when()
        .into_iter()
        .filter(|(query, ..)| query.contains("updateProjectV2ItemFieldValue(input:"))
        .map(|(.., at)| at)
        .collect();
    assert_eq!(writes.len(), 20, "all twenty are written");
    for pair in writes.windows(2) {
        let gap = pair[1].duration_since(pair[0]);
        assert!(
            gap >= interval,
            "a card write followed the one before it by {gap:?}, under the configured {interval:?}"
        );
    }
}

impl Setup {
    /// The failure document of a run refused whole, asserting it exits 1 and sent no mutation.
    fn refused(&self, arguments: &[&str]) -> Value {
        let output = self.run(arguments);
        assert_eq!(
            output.status.code(),
            Some(1),
            "`onetaskgraph {}` is refused whole\n{}\n{}",
            arguments.join(" "),
            stdout(&output),
            stderr(&output)
        );
        assert!(
            self.mutations().is_empty(),
            "a refused reconcile writes nothing"
        );
        serde_json::from_str(&stdout(&output)).expect("a failure document")
    }
}

/// `count` items, each at `Todo` and closed as completed.
fn closed_at_todo(count: usize) -> Setup {
    let ids: Vec<String> = (1..=count).map(|n| format!("R-{n}")).collect();
    let items = ids.iter().map(|id| issue(id, Some("Todo"))).collect();
    let closures: Vec<(&str, Option<&str>)> = ids
        .iter()
        .map(|id| (id.as_str(), Some("COMPLETED")))
        .collect();
    Setup::new(items, &closures, 0)
}

#[test]
fn a_board_whose_closed_items_cannot_be_read_refuses_the_reconcile_whole_writing_nothing() {
    let (items, closures) = mixed();
    let setup = Setup::new(items, &closures, 0);
    let before = setup.snapshot(&ALL);
    setup.board.refuse_once("closedItems:repositoryOwner(");
    let failure = setup.refused(&["--json", "task", "status", "reconcile", "--source", "board"]);
    assert!(
        failure["failure"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("board")),
        "the refusal names the source: {failure:#}"
    );
    assert_eq!(setup.snapshot(&ALL), before);
}

#[test]
fn a_reconcile_walks_every_page_of_the_closed_items() {
    let setup = closed_at_todo(150);
    let answer = setup.json(
        &[
            "--json",
            "task",
            "status",
            "reconcile",
            "--source",
            "board",
            "--dry-run",
        ],
        0,
    );
    let items = answer["items"].as_array().expect("items");
    assert_eq!(
        items.len(),
        150,
        "every closed item of both pages is listed"
    );
    assert!(
        items.iter().all(|item| item["action"] == "would-update"),
        "{answer:#}"
    );
    let pages = setup
        .board
        .served()
        .into_iter()
        .filter(|(query, _)| query == onetaskgraph_github_projects::graphql::CLOSED_ITEMS)
        .count();
    assert_eq!(pages, 2, "a page of 100 and a page of 50");
    setup.item_reads_are_closed_filtered();
}

#[test]
fn a_closed_items_cursor_that_does_not_advance_refuses_the_reconcile_whole_writing_nothing() {
    let setup = closed_at_todo(150);
    setup.board.stall_the_closed_items_cursor();
    let failure = setup.refused(&["--json", "task", "status", "reconcile", "--source", "board"]);
    assert!(
        failure["failure"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("cursor")),
        "the refusal says the paging did not advance: {failure:#}"
    );
    assert_eq!(setup.board.status("R-1").as_deref(), Some("Todo"));
}
