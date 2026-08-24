//! The journeys that need more than one source at once.
//!
//! Two `in-memory` sources over the same dataset, declaring deliberately different
//! capability. That pair is what proves the three things a single source cannot: that one
//! query reaches several sources and comes back as one answer, that the answer is correct
//! by two different routes, and that one source failing leaves the others standing.

use std::process::Output;

use crate::common::{SOURCE_BOUNDARIES, Sandbox, SourceBoundary, stderr, stdout};
use crate::fixtures::{NATIVE, ROWS, SCANNED, dataset, document, pair_at, qualified};
use serde_json::json;

/// A sandbox holding both `in-memory` sources.
fn host(boundary: SourceBoundary) -> Sandbox {
    let sandbox = Sandbox::new();
    sandbox.project_document(&pair_at(&sandbox, boundary));
    sandbox
}

fn run(sandbox: &Sandbox, arguments: &[&str]) -> Output {
    sandbox
        .command()
        .args(arguments)
        .assert()
        .get_output()
        .clone()
}

/// Quotes stderr on failure, for the reason `journeys::ok` carries.
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
    for boundary in SOURCE_BOUNDARIES {
        let sandbox = host(boundary);

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
}

#[test]
fn one_query_against_two_sources_of_different_capability_gives_one_answer_and_two_plans() {
    for boundary in SOURCE_BOUNDARIES {
        let rendered = ok(
            &host(boundary),
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
}

#[test]
fn an_emulated_reverse_walk_matches_a_native_one_edge_for_edge() {
    for boundary in SOURCE_BOUNDARIES {
        // The two sources hold the same dependency graph. One answers `depended-on-by`
        // itself; the other declares `forward-only`, so the engine scans it page by page.
        // Both must answer the same edges in the same order, and the plan must say which of
        // the two happened.
        let sandbox = host(boundary);

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
}

#[test]
fn one_source_failing_leaves_the_others_intact_and_costs_exit_four_unless_allowed() {
    for boundary in SOURCE_BOUNDARIES {
        // Linear is deliberately configured without its credential.
        let sandbox = Sandbox::new();
        let mut block = dataset();
        block["capabilities"] = json!({"max_page_size": 50});
        sandbox.project_document(&document(&json!({
            "broken": {"plugin": "linear", "config": {}},
            "work": boundary.source(ROWS[0].plugin, block),
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
}

#[test]
fn sources_list_reports_every_configured_source_and_what_it_declares() {
    for boundary in SOURCE_BOUNDARIES {
        let sandbox = host(boundary);
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
            listing.contains("unavailable") && listing.contains("LINEAR_API_KEY"),
            "{listing}"
        );
    }
}

#[test]
fn a_source_that_declares_nothing_native_says_so_and_the_plan_of_a_missing_source_is_empty() {
    for boundary in SOURCE_BOUNDARIES {
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
            "bare": boundary.source(ROWS[0].plugin, bare),
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
}

#[test]
fn a_walk_across_sources_returns_one_order_whatever_page_size_the_caller_chose() {
    for boundary in SOURCE_BOUNDARIES {
        // Rows come back one from each source in turn, and a page boundary can fall in the
        // middle of one of those turns — which is exactly what a limit that is not a multiple
        // of the number of sources does. `--limit 3` over two sources is the case: it ends
        // its page having taken two rows from the first source and one from the second, with
        // the second source's turn still owed. A next page that began its turns at the first
        // source again would hand that source two rows in a row, and the walk would come back
        // in an order no page size but its own produces.
        //
        // So the assertion is not merely that a walk terminates and loses nothing: it is that
        // every page size returns the *same* sequence, which is the order the README
        // documents. The single-source paging journey cannot see this — with one source every
        // turn is a whole round.
        let sandbox = host(boundary);
        let whole = listed(&ok(&sandbox, &["task", "list", "--limit", "50"]));
        assert_eq!(whole.len(), 8, "the pair serves eight rows between them");

        for limit in ["1", "2", "3", "5", "7"] {
            let mut walked = Vec::new();
            let mut token: Option<String> = None;
            for step in 0..12 {
                let mut arguments = vec!["task", "list", "--limit", limit];
                if let Some(page) = &token {
                    arguments.push("--page");
                    arguments.push(page);
                }
                let rendered = ok(&sandbox, &arguments);
                walked.extend(listed(&rendered));
                token = rendered
                    .lines()
                    .find_map(|line| line.strip_prefix("next page: --page "))
                    .map(str::to_owned);
                if token.is_none() {
                    break;
                }
                assert!(
                    step < 11,
                    "an eight-row walk at --limit {limit} must terminate"
                );
            }
            assert_eq!(
                walked, whole,
                "walking at --limit {limit} must return the same rows in the same order as \
             one page does"
            );
        }
    }
}

#[test]
fn an_emulated_reverse_walk_pages_to_the_same_answer_the_native_one_gives() {
    for boundary in SOURCE_BOUNDARIES {
        // The most delicate path this engine has, walked one page at a time: the scanned
        // source answers `depended-on-by` by reading its items page by page — two at a time,
        // which is its declared ceiling — and collecting each item's forward edges. An outer
        // page can therefore yield more matching edges than the caller's limit, and what
        // happens to the surplus is the whole question: held over it would be the caching
        // this product does not do, dropped it would lose edges, and re-scanned from the top
        // it would repeat them.
        //
        // So the assertion is that every page size walks to the same answer the source that
        // answers reverse dependencies *natively* gives in one page — edge for edge and in
        // the same order.
        let sandbox = host(boundary);
        let native = listed(&ok(
            &sandbox,
            &[
                "task",
                "deps",
                &qualified(NATIVE, "T-2"),
                "--direction",
                "depended-on-by",
            ],
        ));
        assert_eq!(native.len(), 3, "three tasks depend on T-2");

        for limit in ["1", "2", "3"] {
            let mut walked = Vec::new();
            let mut token: Option<String> = None;
            for step in 0..8 {
                let mut arguments = vec![
                    "task",
                    "deps",
                    &"",
                    "--direction",
                    "depended-on-by",
                    "--limit",
                    limit,
                ];
                let id = qualified(SCANNED, "T-2");
                arguments[2] = &id;
                if let Some(page) = &token {
                    arguments.push("--page");
                    arguments.push(page);
                }
                let rendered = ok(&sandbox, &arguments);
                walked.extend(listed(&rendered));
                token = rendered
                    .lines()
                    .find_map(|line| line.strip_prefix("next page: --page "))
                    .map(str::to_owned);
                if token.is_none() {
                    break;
                }
                assert!(
                    step < 7,
                    "a three-edge walk at --limit {limit} must terminate"
                );
            }

            // The two sources hold the same graph under different names, so compare the
            // native ids the rows are qualified with.
            let scanned: Vec<String> = walked
                .iter()
                .map(|id| id.replace(&format!("{SCANNED}:"), ""))
                .collect();
            let expected: Vec<String> = native
                .iter()
                .map(|id| id.replace(&format!("{NATIVE}:"), ""))
                .collect();
            assert_eq!(
                scanned, expected,
                "walking the emulated reverse answer at --limit {limit} must give the native \
             answer edge for edge"
            );
        }
    }
}
