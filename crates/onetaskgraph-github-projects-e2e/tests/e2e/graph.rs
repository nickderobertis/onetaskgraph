//! `project graph` over a GitHub Projects board: what it costs, and that a board failing part
//! of a read prints no part of a graph.
//!
//! Driven through the compiled binary against the loopback board of
//! `onetaskgraph_e2e_support::fixtures`, which records every request it serves — the same
//! board `copy_cost.rs` counts a copy's requests through. The contract's own bytes are held
//! over folders of Markdown in the engine's suite; what is this plugin's to prove is that the
//! edges ride on the listing, so a plan's requests grow with the pages of its tasks rather than
//! with its tasks or its edges.

use std::process::Output;

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{GitHubBoardFields, PlanIssue, document, github_projects_with_plan};

/// The project issue every plan below files its tasks under.
const PLAN: &str = "PLAN-1";

/// How many sub-issues one page of GitHub's `subIssues` holds as this source asks for them.
const LISTING_PAGE: usize = onetaskgraph_github_projects::MAX_PAGE_SIZE as usize;

/// How many of an issue's blockers one listing carries beside it: `$nestedFirst`.
const CARRIED_BLOCKERS: usize = 50;

/// A plan of `count` tasks, most with one or two edges: each task after the first is blocked by
/// the one before it, every third by the one before that too, and every tenth by nothing.
fn tasks(count: usize) -> Vec<PlanIssue> {
    (0..count)
        .map(|index| {
            let mut blocked_by = Vec::new();
            if index % 10 != 0 {
                blocked_by.push(format!("S-{:03}", index - 1));
                if index % 3 == 0 && index >= 2 {
                    blocked_by.push(format!("S-{:03}", index - 2));
                }
            }
            PlanIssue {
                id: format!("S-{index:03}"),
                title: format!("Step {index:03}"),
                blocked_by,
            }
        })
        .collect()
}

/// A sandbox configuring the board holding `plan` as source `board`.
fn hosted(plan: &[PlanIssue]) -> (Sandbox, GitHubBoardFields) {
    let sandbox = Sandbox::new();
    let (config, board) = github_projects_with_plan(&sandbox, PLAN, plan);
    sandbox.project_document(&document(&json!({
        "board": {"plugin": "github-projects", "config": config}
    })));
    (sandbox, board)
}

/// One `project graph` invocation and every request the board served for it.
fn graph(sandbox: &Sandbox, board: &GitHubBoardFields, extra: &[&str]) -> (Output, Vec<String>) {
    let before = board.documents().len();
    let mut arguments = vec!["project", "graph", "board:PLAN-1"];
    arguments.extend_from_slice(extra);
    let output = sandbox
        .command()
        .args(&arguments)
        .assert()
        .get_output()
        .clone();
    (output, board.documents()[before..].to_vec())
}

/// The requests one successful `project graph` cost, and the nodes and edges it printed.
fn measured(sandbox: &Sandbox, board: &GitHubBoardFields, extra: &[&str]) -> (usize, Value) {
    let mut arguments = vec!["--format", "json"];
    arguments.extend_from_slice(extra);
    let (output, served) = graph(sandbox, board, &arguments);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let printed: Value = serde_json::from_str(&stdout(&output)).expect("one JSON document");
    (served.len(), printed)
}

/// What each request of a run was, by the one operation in it that names it.
fn named(served: &[String]) -> Vec<&'static str> {
    served
        .iter()
        .map(|query| {
            if query.contains("subIssues(first:$first") {
                "subIssues page"
            } else if query.contains("blocking(first:$first") {
                "dependency read"
            } else if query == onetaskgraph_github_projects::graphql::ISSUE {
                "issue read"
            } else {
                "other"
            }
        })
        .collect()
}

#[test]
fn a_plans_requests_grow_with_its_listing_pages_and_not_with_its_tasks_or_edges() {
    // Three listing pages, then a fourth of further tasks, each with edges.
    let three = 2 * LISTING_PAGE + 50;
    let four = three + LISTING_PAGE;
    let mut costs = Vec::new();
    for count in [three, four] {
        let plan = tasks(count);
        let edges: usize = plan.iter().map(|task| task.blocked_by.len()).sum();
        assert!(
            plan.iter()
                .filter(|task| !task.blocked_by.is_empty())
                .count()
                * 10
                >= count * 8,
            "most tasks have an edge"
        );
        let (sandbox, board) = hosted(&plan);
        for grouping in [&[][..], &["--group-by", "orchestrator.unit"][..]] {
            let (output, served) = graph(&sandbox, &board, grouping);
            assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
            let pages = count.div_ceil(LISTING_PAGE);
            // The project's own issue, read once to say it is a project, and one request per
            // listing page: every edge rode on the page that listed its task.
            let mut expected = vec!["issue read"];
            expected.extend(std::iter::repeat_n("subIssues page", pages));
            assert_eq!(named(&served), expected, "{count} tasks {grouping:?}");
            costs.push(served.len());

            let (_, printed) = measured(&sandbox, &board, grouping);
            assert_eq!(printed["nodes"].as_array().unwrap().len(), count);
            assert_eq!(printed["edges"].as_array().unwrap().len(), edges);
        }
    }
    assert_eq!(costs, [4, 4, 5, 5]);
    // A listing page more — a hundred tasks, nearly two hundred edges — is one request more.
    assert!(costs[2] - costs[0] <= 1 && costs[3] - costs[1] <= 1);
}

#[test]
fn a_board_refusing_a_later_listing_page_prints_no_part_of_the_graph() {
    for format in ["mermaid", "json"] {
        let (sandbox, board) = hosted(&tasks(LISTING_PAGE + 20));
        board.refuse_after("subIssues(first:$first", 1);
        let (output, served) = graph(&sandbox, &board, &["--format", format]);
        assert_ne!(output.status.code(), Some(0));
        assert_eq!(stdout(&output), "", "a graph was printed in part");
        assert!(
            stderr(&output).contains("subIssues(first:$first"),
            "the source's error is not reported: {}",
            stderr(&output)
        );
        // The first page landed and the second was the one refused.
        let listing = named(&served)
            .into_iter()
            .filter(|name| *name == "subIssues page")
            .count();
        assert!(listing >= 2, "{:?}", named(&served));
    }
}

#[test]
fn a_board_refusing_a_dependency_read_after_the_listing_prints_no_part_of_the_graph() {
    for format in ["mermaid", "json"] {
        // One task blocked by more issues than a listing carries beside it, so its edges are
        // read on their own after the listing — and that read is refused.
        let mut plan = tasks(CARRIED_BLOCKERS + 10);
        let crowded = (0..=CARRIED_BLOCKERS)
            .map(|index| format!("S-{index:03}"))
            .collect();
        plan.push(PlanIssue {
            id: "S-999".into(),
            title: "Blocked by many".into(),
            blocked_by: crowded,
        });
        let (sandbox, board) = hosted(&plan);
        board.refuse_once("blocking(first:$first");
        let (output, served) = graph(&sandbox, &board, &["--format", format]);
        assert_ne!(output.status.code(), Some(0));
        assert_eq!(stdout(&output), "", "a graph was printed in part");
        assert!(
            stderr(&output).contains("blocking(first:$first"),
            "the source's error is not reported: {}",
            stderr(&output)
        );
        let names = named(&served);
        let refused = names
            .iter()
            .position(|name| *name == "dependency read")
            .expect("the dependency read was sent");
        assert!(
            names[..refused].contains(&"subIssues page"),
            "the listing had landed before the refused read: {names:?}"
        );

        // And the same board, refusing nothing, draws the crowded task's every edge.
        let (sandbox, board) = hosted(&plan);
        let (_, printed) = measured(&sandbox, &board, &[]);
        let crowded_edges = printed["edges"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|edge| edge["to"] == "board:S-999")
            .count();
        assert_eq!(crowded_edges, CARRIED_BLOCKERS + 1);
    }
}
