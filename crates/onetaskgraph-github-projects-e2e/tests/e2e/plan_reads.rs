//! What reading one plan on a board costs, and finding every copy of one plan there: each read
//! through the real binary against the loopback board of `plan_copy`, at 2,000 items and again
//! at 20,000, held to never paging the board's items or its issue search and to sending the same
//! requests at both sizes — what the plan holds sets the cost, not what the board has gathered.
//!
//! Approval on the host reads a plan's records with `task list --project`, `document list
//! --project`, `project show` and `document show`, and finds a plan's copies with `project list
//! --origin`; before, the last of those was a whole-board issue search, and stopped seeing past
//! GitHub's first 1,000 results.

use super::asset_board::Call;
use super::plan_copy::{LARGE, Plan, SMALL, TASKS, counterparts, never_walks, tally};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

/// The four reads of one plan, by what each is, with what each printed and sent.
type Reads = BTreeMap<&'static str, (Value, Vec<Call>)>;

/// The generated plan copied onto a board of `items` other items, then read the four ways
/// approval reads it.
fn read_the_plan(items: usize) -> (Plan, BTreeMap<String, String>, Reads) {
    let plan = Plan::new(items, Vec::new());
    let (project, document, _) = plan.copy();
    let at = counterparts(&project, &document);
    let project = format!("board:{}", at["P"]);
    let document = format!("board:{}", at["D"]);
    let mut reads = Reads::new();
    for (what, arguments) in [
        (
            "task list --project",
            vec![
                "task",
                "list",
                "--source",
                "board",
                "--project",
                &project,
                "--json",
            ],
        ),
        (
            "document list --project",
            vec![
                "document",
                "list",
                "--source",
                "board",
                "--project",
                &project,
                "--json",
            ],
        ),
        ("project show", vec!["project", "show", &project, "--json"]),
        (
            "document show",
            vec!["document", "show", &document, "--json"],
        ),
    ] {
        reads.insert(what, plan.measured(&arguments));
    }
    (plan, at, reads)
}

/// The ids and titles of a listing's or a show's items.
fn shown(answer: &Value) -> BTreeSet<(String, String)> {
    answer["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            (
                row["item"]["id"].as_str().unwrap().to_owned(),
                row["item"]["title"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

#[test]
fn a_plans_records_are_read_from_the_project_and_cost_the_same_on_any_board() {
    let (_small_plan, small_at, small) = read_the_plan(SMALL);
    for (what, (answer, calls)) in &small {
        never_walks(what, calls);
        let expected: BTreeSet<(String, String)> = match *what {
            "task list --project" => (1..=TASKS)
                .map(|n| {
                    (
                        small_at[&format!("T-{n}")].clone(),
                        format!("Step {n} of the plan"),
                    )
                })
                .collect(),
            "project show" => BTreeSet::from([(
                small_at["P"].clone(),
                "Copy the plan in a fraction of the time".to_owned(),
            )]),
            // A document reads back by its own title: the prefix that makes the issue one is
            // the board's spelling, and not part of it.
            _ => BTreeSet::from([(small_at["D"].clone(), "Design of the plan copy".to_owned())]),
        };
        assert_eq!(shown(answer), expected, "{what}");
    }
    let content = small["document show"].0["items"][0]["item"]["content"]
        .as_str()
        .unwrap();
    for n in 1..=TASKS {
        assert!(
            content.contains(&format!(
                "https://example.invalid/{}",
                small_at[&format!("T-{n}")]
            )),
            "the document names T-{n}'s counterpart"
        );
    }

    let (_large_plan, _, large) = read_the_plan(LARGE);
    for (what, (_, calls)) in &large {
        never_walks(what, calls);
        assert_eq!(
            tally(calls),
            tally(&small[what].1),
            "{what} sent different requests on a board of {SMALL} and of {LARGE}"
        );
    }
}

/// The origin every case below asks for.
const ORIGIN: &str = "plans:ORIGIN-1";

/// A project issue of the board's, stamped as a copy of `origin`.
fn stamped(id: &str, origin: &str, project: bool) -> Value {
    let mut slot = json!({"onetaskgraph.origin": origin});
    if project {
        slot["onetaskgraph.item_kind"] = json!("project");
    }
    json!({"item":format!("ITEM-{id}"),"id":id,"type":"Issue",
        "title":format!("Stamped plan {id}"),
        "body":format!("A copy.\n\n<!-- onetaskgraph.metadata\n{slot}\n-->"),
        "state":"OPEN","reason":null,"parent":null,
        "repo":"nickderobertis/onetaskgraph","status":"Todo","origin":origin,"labels":[]})
}

/// A board of `items` other items holding `copies` projects stamped with [`ORIGIN`], beside
/// projects stamped with origins that only resemble it and a task stamped with it exactly.
fn stamped_board(items: usize, copies: usize) -> (Plan, BTreeSet<String>) {
    let mut extra: Vec<Value> = (0..copies)
        .map(|n| stamped(&format!("COPY-{n}"), ORIGIN, true))
        .collect();
    extra.extend([
        stamped("NEAR-1", "plans:ORIGIN-10", true),
        stamped("NEAR-2", "other:ORIGIN-1", true),
        stamped("NEAR-3", "plans:ORIGIN-", true),
        stamped("TASK-1", ORIGIN, false),
    ]);
    let plan = Plan::new(items, extra);
    let expected = (0..copies).map(|n| format!("COPY-{n}")).collect();
    (plan, expected)
}

/// The ids of a project listing's rows.
fn ids(answer: &Value) -> BTreeSet<String> {
    answer["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["item"]["id"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn project_list_by_origin_lists_exactly_its_copies_and_costs_the_same_on_any_board() {
    for copies in [0, 1, 2] {
        let mut tallies = Vec::new();
        for items in [SMALL, LARGE] {
            let (plan, expected) = stamped_board(items, copies);
            let (answer, calls) = plan.measured(&[
                "project", "list", "--source", "board", "--origin", ORIGIN, "--json",
            ]);
            assert_eq!(ids(&answer), expected, "{copies} copies on {items}");
            // No page of the board's items, and none of its issue search either: the one
            // lookup sent is the origin's own.
            never_walks("a listing by origin", &calls);
            tallies.push(tally(&calls));

            // The same rows, in the same shape, as `project list` prints for the same projects
            // — in JSON and in the human rendering alike — and `project list` without the
            // origin still lists every one of them.
            let (searched, _) = plan.measured(&[
                "project",
                "list",
                "--source",
                "board",
                "--search",
                "Stamped plan",
                "--in",
                "title",
                "--json",
            ]);
            for row in answer["items"].as_array().unwrap() {
                assert!(
                    searched["items"].as_array().unwrap().contains(row),
                    "{row:#} is not a row `project list` prints"
                );
            }
            assert!(
                expected.is_subset(&ids(&searched)),
                "`project list` without --origin lists every copy"
            );
            let lease = plan.clock.lease(0);
            let by_origin = success_text(
                &plan.run(&["project", "list", "--source", "board", "--origin", ORIGIN]),
            );
            lease.wait_for_detach();
            let listed = success_text(&plan.run(&[
                "project",
                "list",
                "--source",
                "board",
                "--search",
                "Stamped plan",
                "--in",
                "title",
            ]));
            lease.wait_for_detach();
            drop(lease);
            let rows: Vec<&str> = listed.lines().collect();
            for id in &expected {
                let row = by_origin
                    .lines()
                    .find(|line| line.contains(id.as_str()))
                    .unwrap_or_else(|| panic!("{id} has no row in\n{by_origin}"));
                assert!(rows.contains(&row), "{row:?} is not a row of\n{listed}");
            }
            for decoy in ["NEAR-1", "NEAR-2", "NEAR-3", "TASK-1"] {
                assert!(
                    !by_origin.contains(decoy),
                    "{decoy} is listed in\n{by_origin}"
                );
            }
        }
        assert_eq!(
            tallies[0], tallies[1],
            "{copies} copies: the listing by origin sent different requests on a board of \
             {SMALL} and of {LARGE}"
        );
    }
}

/// What a command printed to standard output, once it exited 0.
fn success_text(output: &std::process::Output) -> String {
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}
