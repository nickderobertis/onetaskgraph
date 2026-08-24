//! The journeys that need more than one source at once.
//!
//! Two `in-memory` sources over the same dataset, declaring deliberately different
//! capability. That pair is what proves the three things a single source cannot: that one
//! query reaches several sources and comes back as one answer, that the answer is correct
//! by two different routes, and that one source failing leaves the others standing.

use std::process::Output;

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{NATIVE, ROWS, SCANNED, dataset, document, pair, qualified};
use serde_json::json;

/// A sandbox holding both `in-memory` sources.
fn host() -> Sandbox {
    let sandbox = Sandbox::new();
    sandbox.project_document(&pair(&sandbox));
    sandbox
}

/// Run the binary in `sandbox`.
fn run(sandbox: &Sandbox, arguments: &[&str]) -> Output {
    sandbox
        .command()
        .args(arguments)
        .assert()
        .get_output()
        .clone()
}

/// Run it and insist it succeeded.
fn ok(sandbox: &Sandbox, arguments: &[&str]) -> String {
    let output = run(sandbox, arguments);
    assert_eq!(
        output.status.code(),
        Some(0),
        "`onetaskgraph {}` exited {:?}\n{}",
        arguments.join(" "),
        output.status.code(),
        stderr(&output)
    );
    stdout(&output)
}

/// The first column of every line up to the first blank one.
fn listed(rendered: &str) -> Vec<String> {
    rendered
        .lines()
        .take_while(|line| !line.trim().is_empty())
        .filter_map(|line| line.split_whitespace().next())
        .map(str::to_owned)
        .collect()
}

#[test]
fn a_query_reaches_one_source_several_or_every_configured_one() {
    let sandbox = host();

    let one = listed(&ok(&sandbox, &["task", "list", "--source", NATIVE]));
    assert_eq!(one.len(), 4);
    assert!(one.iter().all(|id| id.starts_with(&format!("{NATIVE}:"))));

    let both = listed(&ok(
        &sandbox,
        &["task", "list", "--source", NATIVE, "--source", SCANNED],
    ));
    assert_eq!(both.len(), 8, "{both:?}");

    // Naming none addresses every configured source, and the rows interleave: the answer
    // is one merged view rather than one source's list after another's.
    let every = listed(&ok(&sandbox, &["task", "list"]));
    assert_eq!(every, both);
    assert_eq!(
        every[..2],
        [qualified(NATIVE, "T-1"), qualified(SCANNED, "T-1")]
    );
}

#[test]
fn one_query_against_two_sources_of_different_capability_gives_one_answer_and_two_plans() {
    let rendered = ok(
        &host(),
        &[
            "task",
            "list",
            "--label",
            "bug",
            "--explain",
            "--limit",
            "50",
        ],
    );

    assert_eq!(
        listed(&rendered),
        [
            qualified(NATIVE, "T-1"),
            qualified(SCANNED, "T-1"),
            qualified(NATIVE, "T-3"),
            qualified(SCANNED, "T-3"),
        ],
        "one correct answer, whatever each source could do itself:\n{rendered}"
    );

    let plan = rendered
        .split_once("plan:")
        .expect("--explain renders the plan")
        .1;
    let native = plan
        .split(SCANNED)
        .next()
        .expect("the native source's entry comes first");
    assert!(
        native.contains("pushed down: label"),
        "the source that filters by label must have it pushed down:\n{plan}"
    );
    assert!(
        plan.contains("applied locally: label"),
        "and the source that does not must have it narrowed here:\n{plan}"
    );
}

#[test]
fn an_emulated_reverse_walk_matches_a_native_one_edge_for_edge() {
    // The two sources hold the same dependency graph. One answers `depended-on-by`
    // itself; the other declares `forward-only`, so the engine scans it page by page.
    // Both must answer the same edges in the same order, and the plan must say which of
    // the two happened.
    let sandbox = host();

    let mut answers = Vec::new();
    for source in [NATIVE, SCANNED] {
        let rendered = ok(
            &sandbox,
            &[
                "task",
                "deps",
                &qualified(source, "T-2"),
                "--direction",
                "depended-on-by",
                "--explain",
            ],
        );
        let edges: Vec<String> = rendered
            .lines()
            .take_while(|line| !line.trim().is_empty())
            .map(|line| line.replace(&format!("{source}:"), ""))
            .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
            .collect();
        answers.push(edges);

        let expected = if source == NATIVE {
            "pushed down: reverse-dependencies"
        } else {
            "emulated: reverse-dependencies"
        };
        assert!(
            rendered.contains(expected),
            "{source}: the plan must say `{expected}`:\n{rendered}"
        );
    }

    assert_eq!(
        answers[0], answers[1],
        "the scanned reverse answer must match the native one edge for edge"
    );
    assert_eq!(answers[0].len(), 3);
}

#[test]
fn one_source_failing_leaves_the_others_intact_and_costs_exit_four_unless_allowed() {
    // `linear` is registered and its source has not landed, so its factory refuses —
    // the same shape as a credential that is not there.
    let sandbox = Sandbox::new();
    let mut block = dataset();
    block["capabilities"] = json!({"max_page_size": 50});
    sandbox.project_document(&document(&json!({
        "broken": {"plugin": "linear", "config": {}},
        "work": {"plugin": ROWS[0].plugin, "config": block},
    })));

    let output = run(&sandbox, &["task", "list"]);
    assert_eq!(output.status.code(), Some(4));
    assert_eq!(
        listed(&stdout(&output)).len(),
        4,
        "the working source's results stand:\n{}",
        stdout(&output)
    );
    let complaint = stderr(&output);
    assert!(complaint.contains("broken"), "{complaint}");
    assert!(complaint.contains("--allow-partial"), "{complaint}");

    let allowed = run(&sandbox, &["task", "list", "--allow-partial"]);
    assert_eq!(
        allowed.status.code(),
        Some(0),
        "allowing a partial answer makes the same run succeed"
    );
    assert_eq!(listed(&stdout(&allowed)).len(), 4);
    assert!(
        stderr(&allowed).contains("broken"),
        "and still says which source was missing: {}",
        stderr(&allowed)
    );
}

#[test]
fn sources_list_reports_every_configured_source_and_what_it_declares() {
    let sandbox = host();
    let rendered = ok(&sandbox, &["sources", "list"]);
    assert_eq!(listed(&rendered), [NATIVE, SCANNED]);
    assert!(
        rendered.contains("both-directions") && rendered.contains("forward-only"),
        "the declared dependency support is what a user needs to read here:\n{rendered}"
    );
    assert!(rendered.contains("page <= 2"), "{rendered}");

    // A source that could not be built is listed too, with the reason.
    let broken = Sandbox::new();
    broken.project_document(&document(&json!({
        "broken": {"plugin": "linear", "config": {}},
    })));
    let listing = ok(&broken, &["sources", "list"]);
    assert!(
        listing.contains("unavailable") && listing.contains("not implemented yet"),
        "{listing}"
    );
}

#[test]
fn a_source_that_declares_nothing_native_says_so_and_the_plan_of_a_missing_source_is_empty() {
    let sandbox = Sandbox::new();
    let mut bare = dataset();
    bare["capabilities"] = json!({
        "projects": "unsupported",
        "orphan_tasks": "unsupported",
        "filter_by_label": "unsupported",
        "filter_by_status": "unsupported",
        "search_title": "unsupported",
        "search_content": "unsupported",
        "max_page_size": 3
    });
    sandbox.project_document(&document(&json!({
        "bare": {"plugin": ROWS[0].plugin, "config": bare},
        "gone": {"plugin": "linear", "config": {}},
    })));

    let listing = ok(&sandbox, &["sources", "list"]);
    assert!(
        listing.contains("native: none"),
        "a source that applies nothing itself says exactly that:\n{listing}"
    );

    // A source with no projects contributes nothing to a project query, and the plan says
    // the predicate was unavailable rather than leaving a user to guess.
    let projects = run(
        &sandbox,
        &["project", "list", "--explain", "--allow-partial"],
    );
    assert_eq!(projects.status.code(), Some(0), "{}", stderr(&projects));
    assert!(
        stdout(&projects).contains("unavailable: project"),
        "{}",
        stdout(&projects)
    );

    // And a verb addressed at a source that never built has nothing to plan at all.
    let missing = run(&sandbox, &["task", "show", "gone:T-1", "--explain"]);
    assert_eq!(missing.status.code(), Some(4));
    assert!(
        stdout(&missing).contains("(no source was addressed)"),
        "{}",
        stdout(&missing)
    );
}
