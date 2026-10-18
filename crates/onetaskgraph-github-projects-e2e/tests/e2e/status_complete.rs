//! `task status complete` against loopback boards, through the binary.
//!
//! The verb a landing's closures reach the store through: by issue reference, naming no source,
//! so the store asks every GitHub board it configures — each once per fifty references — and
//! reads no board whole. This file holds what the GitHub source owes it: the recording behind
//! the `github-landing-closure-points` budget, at 1x and 10x, priced with the plugin's own
//! table; the batch boundary at fifty references; and the `delivers` rule running on a
//! completion exactly as it runs on any status write.
//!
//! The budget recording holds no threshold of its own: `budget_runner::report_landing_closure`
//! reads it and hands the figure to onebudgetspec, which alone judges it.

use std::path::PathBuf;
use std::process::Output;

use onetaskgraph_github_projects::worst_case_point_cost;
use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{
    GITHUB_SECOND_BOARD_TOKEN, GitHubBoardFields, document, github_number,
    github_projects_with_items,
};

/// The budget the scaled journey records for.
pub(super) const BUDGET: &str = "github-landing-closure-points";
/// The workload it records, which the report refuses any other recording for.
pub(super) const WORKLOAD: &str = "landing-closing-open-issues-two-github-boards";
/// The two scales it records: issues the landing closes, the size of the board holding them,
/// and the size of the second board, which holds none of them.
pub(super) const SCALES: [(&str, usize, usize, usize); 2] =
    [("1x", 1, 1_100, 1_100), ("10x", 10, 11_000, 1_100)];

/// The repository every fixture issue lives in.
const REPOSITORY: &str = "nickderobertis/onetaskgraph";

/// One open task issue filed under no project, its card at `status`.
fn issue(id: &str, status: &str) -> Value {
    json!({"item":format!("ITEM-{id}"),"id":id,"type":"Issue","title":format!("Task {id}"),
           "body":format!("{id}, as filed"),"state":"OPEN","reason":null,"parent":null,
           "repo":REPOSITORY,"status":status,"origin":"","labels":[]})
}

/// The reference a landing's `closes` names the issue `id` by.
fn reference(id: &str) -> String {
    format!("{REPOSITORY}#{}", github_number(id))
}

/// The id of the `n`th issue a journey names by its reference.
fn named(n: usize) -> String {
    format!("N-{n}")
}

/// The id of the `n`th filler issue a board is grown to its size with.
fn filler(n: usize) -> String {
    format!("F-{n}")
}

/// One server holding two boards over the same issues — `plans`, project 7, holding `named`
/// issues at `Todo` and enough filler to hold `size` items, and `followups`, project 8, holding
/// `second` of the filler issues and none of the named ones — configured as two GitHub sources.
struct TwoBoards {
    sandbox: Sandbox,
    board: GitHubBoardFields,
}

impl TwoBoards {
    fn new(named_issues: usize, size: usize, second: usize) -> Self {
        let sandbox = Sandbox::new();
        // The shared dataset's nine items are on the first board already.
        let fill = size.saturating_sub(named_issues + 9);
        let mut items: Vec<Value> = (1..=named_issues)
            .map(|n| issue(&named(n), "Todo"))
            .collect();
        items.extend((0..fill).map(|n| issue(&filler(n), "Todo")));
        let (plans, board) = github_projects_with_items(&sandbox, items);
        for n in 0..second.min(fill) {
            board.put_on_second_board(&filler(n), Some("Todo"));
        }
        sandbox.project_document(&document(&json!({
            "plans": {"plugin":"github-projects","config":plans},
            "followups": {"plugin":"github-projects","config":board.second_board()},
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

    /// `task status complete` for `ids`, naming no source, as machine output — asserting it
    /// exits `code`.
    fn complete(&self, ids: &[String], code: i32) -> Value {
        let mut arguments = vec![
            "--json".to_owned(),
            "task".to_owned(),
            "status".to_owned(),
            "complete".to_owned(),
        ];
        for id in ids {
            arguments.push("--issue".to_owned());
            arguments.push(reference(id));
        }
        let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
        let output = self.run(&arguments);
        assert_eq!(
            output.status.code(),
            Some(code),
            "`onetaskgraph task status complete`\n{}\n{}",
            stdout(&output),
            stderr(&output)
        );
        serde_json::from_str(&stdout(&output)).expect("the command emits JSON")
    }
}

/// What one source's requests came to: reads and their points, writes and their points.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Spent {
    reads: u64,
    read_points: u64,
    writes: u64,
    write_points: u64,
}

impl Spent {
    fn points(self) -> u64 {
        self.read_points + self.write_points
    }

    fn json(self) -> Value {
        json!({"reads": self.reads, "read_points": self.read_points,
               "writes": self.writes, "write_points": self.write_points})
    }
}

/// What the requests one source sent cost, each priced with the plugin's own table.
fn spent(served: &[(String, Value)]) -> Spent {
    let mut spent = Spent {
        reads: 0,
        read_points: 0,
        writes: 0,
        write_points: 0,
    };
    for (document, _) in served {
        let points = worst_case_point_cost(document).expect("a document the plugin prices");
        if document.trim_start().starts_with("mutation") {
            spent.writes += 1;
            spent.write_points += points;
        } else {
            spent.reads += 1;
            spent.read_points += points;
        }
    }
    spent
}

/// That `served` holds no read of a board's items, no search and no project read — every
/// read it holds is one of the named issues.
fn reads_only_the_named_issues(served: &[(String, Value)], who: &str) {
    for (document, _) in served {
        if document.trim_start().starts_with("mutation") {
            continue;
        }
        assert_eq!(
            document.as_str(),
            onetaskgraph_github_projects::graphql::ISSUES_BY_REFERENCE,
            "{who}: every read is of the named issues"
        );
    }
}

#[test]
fn a_landing_closing_issues_on_one_of_two_boards_writes_only_those_issues_cards() {
    let mut recorded = serde_json::Map::new();
    let mut figure = 0;
    for (scale, closed, size, second) in SCALES {
        let boards = TwoBoards::new(closed, size, second);
        let ids: Vec<String> = (1..=closed).map(named).collect();
        let answer = boards.complete(&ids, 0);
        assert_eq!(answer["unsupported"], json!([]), "{scale}: {answer:#}");
        let items = answer["items"].as_array().expect("items");
        assert_eq!(
            items.len(),
            2 * closed,
            "{scale}: one per source per reference"
        );
        // Sources in configuration order — `followups` before `plans` — and references in the
        // order given.
        for (at, id) in ids.iter().enumerate() {
            assert_eq!(
                items[at],
                json!({"issue":reference(id),"source":"followups","task":null,"from":null,
                       "to":null,"action":"absent"}),
                "{scale}"
            );
            assert_eq!(
                items[closed + at],
                json!({"issue":reference(id),"source":"plans","task":format!("plans:{id}"),
                       "from":"Todo","to":{"category":"done","name":"Done"},
                       "action":"updated"}),
                "{scale}"
            );
            assert_eq!(boards.board.status(id).as_deref(), Some("Done"));
            assert_eq!(
                boards.board.state(id),
                ("CLOSED".to_owned(), Some("COMPLETED".to_owned()))
            );
        }
        let plans = boards.board.served_to("test-token");
        let followups = boards.board.served_to(GITHUB_SECOND_BOARD_TOKEN);
        reads_only_the_named_issues(&plans, "plans");
        reads_only_the_named_issues(&followups, "followups");
        let (plans, followups) = (spent(&plans), spent(&followups));
        assert_eq!(plans.reads, 1, "{scale}: one read of the named issues");
        assert_eq!(followups.reads, 1, "{scale}: one read of the named issues");
        assert_eq!(
            followups.writes, 0,
            "{scale}: the second board holds none of them"
        );
        assert_eq!(
            plans.writes,
            2 * closed as u64,
            "{scale}: a card write and a close per issue"
        );
        let points = plans.points() + followups.points();
        recorded.insert(
            scale.to_owned(),
            json!({"issues": closed, "board": size, "second_board": second,
                   "points": points,
                   "sources": {"plans": plans.json(), "followups": followups.json()}}),
        );
        if scale == "10x" {
            figure = points;
        }
    }
    let detail = format!(
        "one `task status complete` naming no source, two GitHub boards configured, priced \
         with the plugin's table: {}",
        Value::Object(recorded.clone())
    );
    let directory = super::budget_runner::directory();
    std::fs::create_dir_all(&directory).expect("the telemetry directory");
    std::fs::write(
        onetaskgraph_e2e_support::telemetry::file_in(&directory, BUDGET),
        serde_json::to_vec(&json!({
            "budget": BUDGET,
            "value": figure,
            "detail": detail,
            "workload": WORKLOAD,
            "scales": Value::Object(recorded),
        }))
        .expect("JSON"),
    )
    .expect("the telemetry is written");
}

#[test]
fn a_completion_reads_fifty_references_at_a_time_and_answers_each_in_the_order_given() {
    for (count, reads) in [(50, 1), (51, 2), (120, 3)] {
        let sandbox = Sandbox::new();
        let mut items: Vec<Value> = (1..=count).map(|n| issue(&named(n), "Todo")).collect();
        items.extend((0..1_100 - count - 9).map(|n| issue(&filler(n), "Todo")));
        let (config, board) = github_projects_with_items(&sandbox, items);
        sandbox.project_document(&document(&json!({
            "plans": {"plugin":"github-projects","config":config},
        })));
        // Given in reverse, so the order of the answer is the order given and not the board's.
        let ids: Vec<String> = (1..=count).rev().map(named).collect();
        let mut arguments = vec!["--json", "task", "status", "complete", "--issue"];
        let references: Vec<String> = ids.iter().map(|id| reference(id)).collect();
        arguments.extend(references.iter().map(String::as_str));
        let output = sandbox
            .command()
            .args(&arguments)
            .assert()
            .get_output()
            .clone();
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        let answer: Value = serde_json::from_str(&stdout(&output)).expect("JSON");
        let items = answer["items"].as_array().expect("items");
        assert_eq!(items.len(), count, "{count}: one per reference");
        for (item, id) in items.iter().zip(&ids) {
            assert_eq!(item["issue"], json!(reference(id)), "{count}");
            assert_eq!(item["action"], "updated", "{count}: {item:#}");
            assert_eq!(board.status(id).as_deref(), Some("Done"), "{count}: {id}");
        }
        let served = board.served();
        reads_only_the_named_issues(&served, "plans");
        let batches: Vec<usize> = served
            .iter()
            .filter(|(document, _)| {
                document.as_str() == onetaskgraph_github_projects::graphql::ISSUES_BY_REFERENCE
            })
            .map(|(_, variables)| {
                // A short batch binds its spare slots to its last reference again.
                (0..onetaskgraph_github_projects::REFERENCE_BATCH)
                    .map(|slot| variables[format!("number{slot}")].as_u64())
                    .collect::<std::collections::BTreeSet<_>>()
                    .len()
            })
            .collect();
        let wanted: Vec<usize> = match count {
            50 => vec![50],
            51 => vec![50, 1],
            _ => vec![50, 50, 20],
        };
        assert_eq!(batches.len(), reads, "{count}: {batches:?}");
        assert_eq!(batches, wanted, "{count}");
    }
}

/// A board whose task `N-1`, at `In Progress`, delivers the ticket `tickets:T-1` of a folder of
/// Markdown — delivered by `others` too — with the issue closed as completed when `closed` says.
struct Delivering {
    sandbox: Sandbox,
    board: GitHubBoardFields,
    tickets: PathBuf,
}

impl Delivering {
    fn new(closed: bool, others: &[&str]) -> Self {
        Self::with_delivers(closed, others, &["tickets:T-1"])
    }

    /// As [`Delivering::new`], with `N-1` delivering every task of `delivers`.
    fn with_delivers(closed: bool, others: &[&str], delivers: &[&str]) -> Self {
        let sandbox = Sandbox::new();
        let tickets = sandbox.subdirectory("tickets");
        std::fs::create_dir_all(tickets.join("tasks")).expect("the ticket folder");
        let mut delivered_by = vec!["plans:N-1".to_owned()];
        delivered_by.extend(others.iter().map(|other| format!("tickets:{other}")));
        std::fs::write(
            tickets.join("tasks/T-1.md"),
            format!(
                "---\ntitle: The ticket\nstatus: in progress\ndelivered_by: {}\n---\nAsked for.\n",
                serde_json::to_string(&delivered_by).expect("a list")
            ),
        )
        .expect("the ticket");
        for other in others {
            std::fs::write(
                tickets.join(format!("tasks/{other}.md")),
                "---\ntitle: Another deliverer\nstatus: in progress\ndelivers: [T-1]\n---\n",
            )
            .expect("the other deliverer");
        }
        let mut deliverer = issue("N-1", "In Progress");
        deliverer["body"] = json!(format!(
            "The deliverer.\n\n<!-- onetaskgraph.metadata\n{}\n-->",
            json!({"onetaskgraph.delivers": delivers})
        ));
        let (mut config, board) = github_projects_with_items(&sandbox, vec![deliverer]);
        board.with_status_option("OPT-in-progress", "In Progress");
        config["status_mapping"] = json!({"todo":"Todo","in-progress":"In Progress"});
        if closed {
            board.close("N-1", Some("COMPLETED"));
        }
        sandbox.project_document(&document(&json!({
            "plans": {"plugin":"github-projects","config":config},
            "tickets": {"plugin":"local-md","config":{"root":tickets}},
        })));
        Self {
            sandbox,
            board,
            tickets,
        }
    }

    /// The ticket's status as the folder reads it.
    fn ticket(&self) -> Value {
        let output = self
            .sandbox
            .command()
            .args(["--json", "task", "show", "tickets:T-1", "--no-comments"])
            .assert()
            .get_output()
            .clone();
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        let shown: Value = serde_json::from_str(&stdout(&output)).expect("JSON");
        shown["items"][0]["item"]["status"]["category"].clone()
    }

    fn complete(&self) -> Value {
        self.complete_exiting(0)
    }

    /// The completion of `N-1`, asserting it exits `code`.
    fn complete_exiting(&self, code: i32) -> Value {
        let output = self
            .sandbox
            .command()
            .args([
                "--json",
                "task",
                "status",
                "complete",
                "--issue",
                &reference("N-1"),
            ])
            .assert()
            .get_output()
            .clone();
        assert_eq!(
            output.status.code(),
            Some(code),
            "{}\n{}",
            stdout(&output),
            stderr(&output)
        );
        serde_json::from_str(&stdout(&output)).expect("JSON")
    }
}

/// The one item a completion over one board answered for `N-1`.
fn only(answer: &Value) -> &Value {
    let items = answer["items"].as_array().expect("items");
    let plans: Vec<&Value> = items
        .iter()
        .filter(|item| item["source"] == "plans")
        .collect();
    assert_eq!(plans.len(), 1, "{answer:#}");
    plans[0]
}

#[test]
fn completing_an_issue_closed_while_its_card_read_in_progress_finishes_the_ticket_it_delivers() {
    let setup = Delivering::new(true, &[]);
    assert_eq!(setup.ticket(), "in-progress");
    let answer = setup.complete();
    let item = only(&answer);
    assert_eq!(item["action"], "updated", "{answer:#}");
    assert_eq!(item["from"], "In Progress");
    assert_eq!(item["delivered"][0]["ticket"], "tickets:T-1", "{answer:#}");
    assert_eq!(setup.board.status("N-1").as_deref(), Some("Done"));
    assert_eq!(
        setup.board.state("N-1"),
        ("CLOSED".to_owned(), Some("COMPLETED".to_owned())),
        "the issue's state and reason are as they were"
    );
    assert!(
        !setup
            .board
            .served()
            .iter()
            .any(|(document, _)| document.contains("updateIssue(input:")),
        "an issue already closed as completed is not closed again"
    );
    assert_eq!(
        setup.ticket(),
        "done",
        "its only deliverer is done, so it is"
    );
}

#[test]
fn completing_an_open_issue_in_progress_closes_it_and_finishes_the_ticket_it_delivers() {
    let setup = Delivering::new(false, &[]);
    let answer = setup.complete();
    assert_eq!(only(&answer)["action"], "updated", "{answer:#}");
    assert_eq!(setup.board.status("N-1").as_deref(), Some("Done"));
    assert_eq!(
        setup.board.state("N-1"),
        ("CLOSED".to_owned(), Some("COMPLETED".to_owned()))
    );
    assert_eq!(setup.ticket(), "done");
}

#[test]
fn a_ticket_another_deliverer_is_still_working_on_reads_what_the_delivers_rule_gives_the_pair() {
    let setup = Delivering::new(true, &["T-2"]);
    let answer = setup.complete();
    assert_eq!(only(&answer)["action"], "updated", "{answer:#}");
    assert_eq!(setup.board.status("N-1").as_deref(), Some("Done"));
    // One deliverer done and one in progress settle at in progress, which the ticket holds.
    assert_eq!(
        onetaskgraph_core::settled(&[
            onetaskgraph_plugin_api::StatusCategory::Done,
            onetaskgraph_plugin_api::StatusCategory::InProgress
        ]),
        Some(onetaskgraph_plugin_api::StatusCategory::InProgress)
    );
    assert_eq!(
        setup.ticket(),
        "in-progress",
        "not done: T-2 is still working"
    );
    assert!(
        std::fs::read_to_string(setup.tickets.join("tasks/T-1.md"))
            .expect("the ticket")
            .contains("status: in progress")
    );
}

#[test]
fn a_delivered_task_that_cannot_be_kept_in_step_is_answered_failed_and_exits_four() {
    let setup = Delivering::with_delivers(true, &[], &["tickets:T-1", "nowhere:T-9"]);
    let answer = setup.complete_exiting(4);
    let item = only(&answer);
    assert_eq!(
        item["action"], "updated",
        "the deliverer's own write landed: {answer:#}"
    );
    let delivered = item["delivered"].as_array().expect("delivered entries");
    let failed: Vec<&Value> = delivered
        .iter()
        .filter(|entry| entry["outcome"] == "failed")
        .collect();
    assert_eq!(failed.len(), 1, "{answer:#}");
    assert_eq!(failed[0]["ticket"], "nowhere:T-9");
    assert_eq!(failed[0]["deliverer"], "plans:N-1");
    assert!(
        failed[0]["failure"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("nowhere")),
        "the failure names the source nothing configures: {answer:#}"
    );
    assert_eq!(setup.board.status("N-1").as_deref(), Some("Done"));
    assert_eq!(setup.ticket(), "done", "the ticket it can keep in step is");
}

/// Fill the cache's entry of `board:T-1` at the version its listing carries, hold it to being a
/// hit, and answer that version.
fn cached_t1(cached: &crate::cache::Cached) -> String {
    let held = crate::cache_kinds::listed(cached, "task", "board:T-1");
    crate::cache_kinds::show(cached, "task", "board:T-1", Some(&held));
    let (_, sent) = crate::cache_kinds::show(cached, "task", "board:T-1", Some(&held));
    assert_eq!(sent, Vec::<String>::new(), "the entry is filled");
    held
}

/// That a read of `board:T-1` pinned to `held` reaches the board after a card write, and reads
/// it done.
fn read_afresh_after_the_write(cached: &crate::cache::Cached, held: &str) {
    let (shown, sent) = crate::cache_kinds::show(cached, "task", "board:T-1", Some(held));
    assert!(
        !sent.is_empty(),
        "the entry the write could change is out of service, so the pinned read is the board's"
    );
    assert_eq!(
        shown["items"][0]["item"]["status"],
        json!({"category":"done","name":"Done"}),
        "{shown:#}"
    );
    assert_ne!(crate::cache_kinds::version(&shown), held);
}

#[test]
fn a_completion_on_a_cached_board_takes_the_written_tasks_entry_out_of_service() {
    let cached = crate::cache::Cached::new();
    let held = cached_t1(&cached);
    let (answer, _) = cached.ok(&[
        "--json",
        "task",
        "status",
        "complete",
        "--issue",
        &reference("T-1"),
    ]);
    assert_eq!(answer["items"][0]["action"], "updated", "{answer:#}");
    read_afresh_after_the_write(&cached, &held);
}

#[test]
fn a_reconcile_on_a_cached_board_takes_the_written_tasks_entry_out_of_service() {
    let cached = crate::cache::Cached::new();
    cached.board.close("T-1", Some("COMPLETED"));
    let held = cached_t1(&cached);
    let (answer, _) = cached.ok(&["--json", "task", "status", "reconcile", "--source", "board"]);
    assert!(
        answer["items"]
            .as_array()
            .expect("items")
            .iter()
            .any(|item| item["source"] == "board:T-1" && item["action"] == "updated"),
        "{answer:#}"
    );
    read_afresh_after_the_write(&cached, &held);
}

#[test]
fn a_private_task_on_a_board_not_declared_private_is_refused_by_both_verbs_and_never_written() {
    let sandbox = Sandbox::new();
    let private = |id: &str| {
        let mut item = issue(id, "Todo");
        item["body"] = json!(format!(
            "Not for everyone.\n\n<!-- onetaskgraph.metadata\n{}\n-->",
            json!({"onetaskgraph.classification": "private"})
        ));
        item
    };
    let (config, board) =
        github_projects_with_items(&sandbox, vec![private("N-1"), private("N-2")]);
    board.reopen("T-2");
    board.close("N-2", Some("COMPLETED"));
    sandbox.project_document(&document(&json!({
        "plans": {"plugin":"github-projects","config":config},
    })));
    let run = |arguments: &[&str]| -> Value {
        let output = sandbox
            .command()
            .args(arguments)
            .assert()
            .get_output()
            .clone();
        assert_eq!(
            output.status.code(),
            Some(4),
            "`onetaskgraph {}` reports the refusal as a failed item\n{}\n{}",
            arguments.join(" "),
            stdout(&output),
            stderr(&output)
        );
        serde_json::from_str(&stdout(&output)).expect("JSON")
    };
    let mutations = || {
        board
            .served()
            .into_iter()
            .filter(|(document, _)| document.trim_start().starts_with("mutation"))
            .count()
    };

    let completed = run(&[
        "--json",
        "task",
        "status",
        "complete",
        "--issue",
        &reference("N-1"),
    ]);
    let item = only(&completed);
    assert_eq!(item["action"], "failed", "{completed:#}");
    assert_eq!(item["task"], "plans:N-1", "{completed:#}");
    assert!(
        item["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("private")),
        "the refusal says why: {completed:#}"
    );

    let reconciled = run(&["--json", "task", "status", "reconcile", "--source", "plans"]);
    let items = reconciled["items"].as_array().expect("items");
    assert_eq!(items.len(), 1, "{reconciled:#}");
    assert_eq!(items[0]["source"], "plans:N-2");
    assert_eq!(items[0]["action"], "failed", "{reconciled:#}");
    assert!(
        items[0]["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("private")),
        "{reconciled:#}"
    );

    assert_eq!(mutations(), 0, "neither verb wrote anything");
    assert_eq!(board.status("N-1").as_deref(), Some("Todo"));
    assert_eq!(board.state("N-1").0, "OPEN");
    assert_eq!(board.status("N-2").as_deref(), Some("Todo"));
}
