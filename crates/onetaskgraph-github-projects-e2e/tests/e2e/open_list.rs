//! Listing a board's open items reads only its open items, driven through the compiled binary
//! against a generated loopback board on the simulated clock, and recorded as the telemetry of
//! the `github-open-board-list-requests` budget, which `budget_runner::report_open_list` hands
//! to onebudgetspec, the one judge of it.
//!
//! The workload is the one the budget states: 2,038 items, 562 of them open, each body about
//! 2 KB, with 7 text fields holding a value on every item. The open items include drafts,
//! items under an option no `status_mapping` names, and open issues under terminal cards; the
//! closed items include issues whose card still sits in an open column. One `task list` names
//! all six open statuses — `draft`, `backlog`, `todo`, `queued`, `in-progress` and `unknown`.
//!
//! What it holds: the listing answers exactly the set the whole board's walk answers once
//! filtered to those six categories in process — the rule the walk applied before this read
//! existed — and the board is sent nothing but pages of its own `is:open` item filter: no page
//! of its unfiltered items, no page of a closed item, and no issue-search sweep.

use std::collections::BTreeSet;
use std::process::{Output, Stdio};

use onetaskgraph_e2e_support::{
    binary,
    clock::SimulatedClock,
    common::{Sandbox, assert_spawnable_on_windows, stderr, stdout},
    fixtures::{GitHubBoardFields, document, github_projects_with_items},
};
use onetaskgraph_github_projects::graphql;
use serde_json::{Value, json};

use super::asset_board::{Board, Call};
use super::follow_up_cycle::points_of;
use crate::budget_runner::directory;

/// The budget this journey records.
pub(crate) const BUDGET: &str = "github-open-board-list-requests";
/// The workload the budget states, which its report refuses any other recording of.
pub(crate) const WORKLOAD: &str = "a generated loopback board of 2,038 items, 562 of them \
    open, bodies about 2 KB, 7 text fields; open drafts, items under an unmapped option and \
    open issues under terminal cards; one task list naming all six open statuses";
/// The phases a recording names its requests by, in order.
pub(crate) const PHASES: [&str; 4] = ["item pages", "field reads", "completion search", "other"];

/// How many items the board holds, and how many of them are open.
const ITEMS: usize = 2_038;
const OPEN: usize = 562;
/// The categories a listing of open work names.
const OPEN_CATEGORIES: [&str; 6] = [
    "draft",
    "backlog",
    "todo",
    "queued",
    "in-progress",
    "unknown",
];
/// The text fields every item holds a value of.
const TEXT_FIELDS: [&str; 7] = [
    "Area",
    "Owner",
    "Estimate",
    "Sprint",
    "Milestone",
    "Team",
    "Notes",
];

/// Whether item `index` is open: a stride coprime to [`ITEMS`] scatters exactly [`OPEN`] of
/// them across the board rather than bunching them at its head.
fn is_open(index: usize) -> bool {
    (index * 7) % ITEMS < OPEN
}

/// About 2 KB of body for item `index`.
fn body(index: usize) -> String {
    let sentence = format!("Item {index} of the grooming board, as filed by its reporter. ");
    sentence.repeat(2_048 / sentence.len() + 1)
}

/// One item of the generated board: open ones spread over every column, drafts, an option no
/// mapping names and the two terminal cards; closed ones mostly under their closure's column,
/// some still under an open column.
fn item(index: usize) -> Value {
    let id = format!("OB-{index}");
    let open = is_open(index);
    let status = match (open, index % 9) {
        (true, 0) => "Triage",
        (true, 1) => "Cancelled",
        (true, 2) => "Done",
        (true, 3) => "Backlog",
        (true, 4) => "In Progress",
        (true, 5) => "Queued",
        (true, _) => "Todo",
        (false, 0 | 1) => "Todo",
        (false, 2) => "In Progress",
        (false, 3) => "Cancelled",
        (false, _) => "Done",
    };
    let draft = open && index.is_multiple_of(25);
    let (state, reason) = match (open, status) {
        (true, _) => ("OPEN", Value::Null),
        (false, "Cancelled") => ("CLOSED", json!("NOT_PLANNED")),
        (false, _) => ("CLOSED", json!("COMPLETED")),
    };
    let texts: serde_json::Map<String, Value> = TEXT_FIELDS
        .iter()
        .map(|name| {
            (
                format!("FIELD-text-{name}"),
                json!(format!("{name} of item {index}")),
            )
        })
        .collect();
    json!({"item":format!("ITEM-{id}"),"id":id,
           "type": if draft { "DraftIssue" } else { "Issue" },
           "title":format!("Grooming item {index}"),"body":body(index),
           "state":state,"reason":reason,"parent":null,
           "repo": if draft { Value::Null } else { json!("nickderobertis/onetaskgraph") },
           "status":status,"origin":"","labels":[],"texts":texts})
}

/// The generated board behind the timing proxy, on the simulated clock.
struct Setup {
    sandbox: Sandbox,
    /// The loopback board itself, held so it keeps serving for as long as the journey runs.
    _github: GitHubBoardFields,
    board: Board,
    clock: SimulatedClock,
}

impl Setup {
    fn new() -> Self {
        let sandbox = Sandbox::new();
        let clock = SimulatedClock::start(1);
        let (mut config, github) =
            github_projects_with_items(&sandbox, (0..ITEMS).map(item).collect());
        github.with_status_option("OPT-in-progress", "In Progress");
        github.with_status_option("OPT-backlog", "Backlog");
        github.with_status_option("OPT-queued", "Queued");
        // An option a person added that no mapping names: an open item under it reads
        // `unknown`.
        github.with_status_option("OPT-triage", "Triage");
        for name in TEXT_FIELDS {
            github.with_text_field(name);
        }
        let board = Board::new(config["endpoint"].as_str().unwrap(), &clock, 0, None);
        board.measure_latency();
        config["endpoint"] = json!(board.endpoint);
        config["status_mapping"] = json!({"backlog":"Backlog","todo":"Todo",
            "in-progress":"In Progress","queued":"Queued","done":"Done","cancelled":"Cancelled"});
        sandbox.project_document(&document(&json!({
            "board": {"plugin":"github-projects","config":config}
        })));
        Self {
            sandbox,
            _github: github,
            board,
            clock,
        }
    }

    fn run(&self, arguments: &[&str]) -> Output {
        let mut command = self.sandbox.subprocess(binary());
        command
            .args(arguments)
            .envs(self.clock.client_env(0))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        assert_spawnable_on_windows(&command);
        command.spawn().unwrap().wait_with_output().unwrap()
    }

    /// One command, its answer, every request the board received for it, and the simulated
    /// seconds it took.
    fn measured(&self, arguments: &[&str]) -> (Value, Vec<Call>, f64) {
        let before = self.board.calls().len();
        let started = self.clock.now();
        let lease = self.clock.lease(0);
        let output = self.run(arguments);
        lease.wait_for_detach();
        drop(lease);
        assert_eq!(
            output.status.code(),
            Some(0),
            "`onetaskgraph {}` failed\n{}",
            arguments.join(" "),
            stderr(&output)
        );
        let answer = serde_json::from_str(&stdout(&output)).expect("the command emits JSON");
        let seconds = (self.clock.now() - started).as_secs_f64();
        (answer, self.board.calls_since(before), seconds)
    }
}

/// The document and variables one call sent.
fn sent(call: &Call) -> (String, Value) {
    let request: Value = serde_json::from_slice(&call.bytes).expect("a GraphQL request");
    (
        request["query"].as_str().expect("a query").to_owned(),
        request["variables"].clone(),
    )
}

/// Which phase of a board listing one request is.
fn phase(document: &str) -> &'static str {
    if document == graphql::OPEN_ITEMS || document == graphql::BOARD {
        "item pages"
    } else if document == graphql::BOARD_FIELDS {
        "field reads"
    } else if document == graphql::SEARCH_ISSUES {
        "completion search"
    } else {
        "other"
    }
}

/// The ids of the tasks a listing answered.
fn listed(answer: &Value) -> BTreeSet<String> {
    answer["items"]
        .as_array()
        .expect("a listing")
        .iter()
        .map(|row| row["id"].as_str().expect("an id").to_owned())
        .collect()
}

#[test]
fn listing_every_open_status_reads_only_the_board_s_open_items() {
    let setup = Setup::new();
    let mut arguments = vec!["--json", "task", "list", "--source", "board", "--all"];
    for category in OPEN_CATEGORIES {
        arguments.extend(["--status", category]);
    }
    let (answer, calls, seconds) = setup.measured(&arguments);

    // The board was sent pages of its own open filter and nothing else.
    let mut phases: Vec<(&str, usize, u64)> = PHASES.iter().map(|name| (*name, 0, 0)).collect();
    for call in &calls {
        let (document, variables) = sent(call);
        assert_eq!(
            document,
            graphql::OPEN_ITEMS,
            "a listing of open work sent {}",
            phase(&document)
        );
        assert_eq!(variables["filter"], "is:open", "{variables}");
        let points = points_of(&document, &variables);
        let at = phases
            .iter_mut()
            .find(|(name, _, _)| *name == phase(&document))
            .expect("a phase");
        at.1 += 1;
        at.2 += points;
    }

    // Exactly what the whole board's walk answers once filtered to the same categories.
    let (whole, walked, _) =
        setup.measured(&["--json", "task", "list", "--source", "board", "--all"]);
    assert!(
        walked.iter().any(|call| sent(call).0 == graphql::BOARD),
        "the comparison walks the whole board"
    );
    let expected: BTreeSet<String> = whole["items"]
        .as_array()
        .expect("a listing")
        .iter()
        .filter(|row| {
            OPEN_CATEGORIES.contains(&row["item"]["status"]["category"].as_str().unwrap())
        })
        .map(|row| row["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(listed(&answer), expected);
    // And its generated items are the ones the generator planted open: every one but those
    // under a terminal card, which read as their card. The fixture board's own shared tasks
    // are listed beside them by the same rule.
    let planted: BTreeSet<String> = (0..ITEMS)
        .filter(|index| is_open(*index))
        .filter(|index| !matches!(index % 9, 1 | 2))
        .map(|index| format!("board:OB-{index}"))
        .collect();
    let generated: BTreeSet<String> = listed(&answer)
        .into_iter()
        .filter(|id| id.starts_with("board:OB-"))
        .collect();
    assert_eq!(generated, planted);
    let shown = |id: &str| -> Value {
        answer["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["id"] == id)
            .unwrap_or_else(|| panic!("{id} is listed"))["item"]
            .clone()
    };
    // A draft, an item under the unmapped option, and an open issue under an open column, each
    // as the whole walk reads it; an open issue under a terminal card is out.
    assert_eq!(shown("board:OB-0")["status"]["category"], "unknown");
    let draft = (0..ITEMS)
        .find(|index| is_open(*index) && index.is_multiple_of(25) && !matches!(index % 9, 1 | 2))
        .unwrap();
    assert!(listed(&answer).contains(&format!("board:OB-{draft}")));
    let terminal = (0..ITEMS)
        .find(|index| is_open(*index) && index % 9 == 1)
        .unwrap();
    assert!(!listed(&answer).contains(&format!("board:OB-{terminal}")));

    let requests = calls.len();
    let points: u64 = phases.iter().map(|(_, _, points)| points).sum();
    let detail = format!(
        "{}; {points} points, {seconds:.1} simulated seconds",
        phases
            .iter()
            .map(|(name, count, points)| format!("{name} {count} requests {points} points"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let directory = directory();
    std::fs::create_dir_all(&directory).expect("the telemetry directory");
    std::fs::write(
        onetaskgraph_e2e_support::telemetry::file_in(&directory, BUDGET),
        json!({"budget": BUDGET, "value": requests, "workload": WORKLOAD,
               "phases": phases.iter().map(|(name, count, points)|
                   json!({"phase": name, "requests": count, "points": points}))
                   .collect::<Vec<_>>(),
               "points": points, "seconds": seconds, "detail": detail})
        .to_string(),
    )
    .expect("the telemetry file");
    eprintln!("{BUDGET}: {requests} requests; {detail}");
}
