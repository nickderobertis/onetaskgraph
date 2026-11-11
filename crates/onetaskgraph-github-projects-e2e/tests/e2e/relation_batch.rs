//! What one batch of relation calls costs a GitHub board in GraphQL rate-limit points, driven
//! through the compiled binary against the loopback board and recorded as the telemetry of the
//! `github-points-per-relation-batch` budget, which `budget_runner` hands to onebudgetspec, the
//! one judge of it.
//!
//! The workload is the one the budget states: 20 tickets on a source reading sub-issues as
//! children, each call a fresh process — one `task relations` over the 20, one `task parent
//! set-many` of 12 edges, one `task link add-many` of 30 typed links, every link type mirrored
//! natively — and then the same three calls repeated with nothing changed. Every request is
//! priced as GitHub prices it: a read at the cost the plugin's own table gives it at the page
//! sizes it bound, and a mutation document at one point.

use serde_json::{Value, json};

use super::follow_up_cycle::points_of;
use super::relations::{Board, issue};
use crate::budget_runner::directory;

/// The budget this journey records.
pub(crate) const BUDGET: &str = "github-points-per-relation-batch";
/// The workload the budget states, which its report refuses any other recording of.
pub(crate) const WORKLOAD: &str = "20 tickets, a fresh process: one task relations over the 20 \
    tickets, one task parent set-many of 12 edges, one task link add-many of 30 typed links, \
    every link type mirrored natively, then the same three calls repeated with nothing changed";
/// The calls, in the order the workload makes them.
pub(crate) const CALLS: [&str; 6] = [
    "relations",
    "parent set-many",
    "link add-many",
    "relations again",
    "parent set-many again",
    "link add-many again",
];

const TICKETS: usize = 20;

fn ticket(n: usize) -> String {
    format!("T-{n}")
}

#[test]
fn measure_github_points_per_relation_batch() {
    let board = Board::of(
        (0..TICKETS).map(|n| issue(&ticket(n), None)).collect(),
        &json!({"sub_issues": "parent",
                "link_types": {"caused-by": {"native": "relates-to"},
                               "related": {"native": "relates-to"}}}),
    );
    let ids: Vec<String> = (0..TICKETS)
        .map(|n| format!("board:{}", ticket(n)))
        .collect();
    let mut relations = vec!["task", "relations"];
    relations.extend(ids.iter().map(String::as_str));
    relations.push("--json");
    // Twelve tickets under the first twelve, three levels at most.
    let edges: Vec<Value> = (0..12)
        .map(|n| json!({"child": ids[n + 8], "parent": ids[n]}))
        .collect();
    let edges = json!({"schema_version": 1, "edges": edges}).to_string();
    // Thirty links: each ticket to the next, and every other ticket to the one after that.
    let links: Vec<Value> = (0..TICKETS)
        .map(|n| json!({"from": ids[n], "to": ids[(n + 1) % TICKETS], "type": "caused-by"}))
        .chain((0..10).map(
            |n| json!({"from": ids[2 * n], "to": ids[(2 * n + 2) % TICKETS], "type": "related"}),
        ))
        .collect();
    let links = json!({"schema_version": 1, "links": links}).to_string();
    let parents = ["task", "parent", "set-many", "--input", "-", "--json"];
    let add = ["task", "link", "add-many", "--input", "-", "--json"];

    let mut calls = Vec::new();
    for pass in 0..2 {
        let (read, served) = board.measured(&relations, "");
        assert_eq!(read.code, Some(0), "{}", read.stderr);
        calls.push(served);
        let (set, served) = board.measured(&parents, &edges);
        assert_eq!(set.code, Some(0), "{:#} {}", set.json, set.stderr);
        let expected = if pass == 0 { "written" } else { "unchanged" };
        assert!(
            set.json["results"]
                .as_array()
                .unwrap()
                .iter()
                .all(|result| result["outcome"] == expected),
            "{:#}",
            set.json
        );
        calls.push(served);
        let (added, served) = board.measured(&add, &links);
        assert_eq!(added.code, Some(0), "{:#} {}", added.json, added.stderr);
        let expected = if pass == 0 { "written" } else { "exists" };
        assert!(
            added.json["results"]
                .as_array()
                .unwrap()
                .iter()
                .all(|result| result["outcome"] == expected),
            "{:#}",
            added.json
        );
        calls.push(served);
    }
    // T-0 relates to T-1 and T-19 by `caused-by`, and to T-2 and T-18 by `related`.
    assert_eq!(board.board.relates_to(&ticket(0)).len(), 4);

    let recorded: Vec<Value> = CALLS
        .iter()
        .zip(&calls)
        .map(|(name, served)| {
            let points: u64 = served
                .iter()
                .map(|(document, variables)| points_of(document, variables))
                .sum();
            json!({"call": name, "requests": served.len(), "points": points})
        })
        .collect();
    let total: u64 = recorded
        .iter()
        .map(|call| call["points"].as_u64().unwrap())
        .sum();
    let detail = recorded
        .iter()
        .map(|call| {
            format!(
                "{} {} requests {} points",
                call["call"].as_str().unwrap(),
                call["requests"],
                call["points"]
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let directory = directory();
    std::fs::create_dir_all(&directory).expect("the telemetry directory");
    std::fs::write(
        onetaskgraph_e2e_support::telemetry::file_in(&directory, BUDGET),
        json!({"budget": BUDGET, "value": total, "workload": WORKLOAD, "calls": recorded,
               "detail": detail})
        .to_string(),
    )
    .expect("the telemetry file");
    eprintln!("{BUDGET}: {total} points; {detail}");
}
