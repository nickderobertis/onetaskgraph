//! What a repeat copy by query of a board's narrowed tasks into a folder of Markdown costs,
//! recorded as telemetry for the source cache's budget to report in its breakdown.
//!
//! The workload is the one a caller keeping a local snapshot of its follow-ups runs: a board of
//! eighty tasks, twenty of which hold a metadata value and were commented on since an instant —
//! each one to five times — copied with their comments into a folder, and the folder kept in step
//! by copying again. The listing is read at five rows a page, and the board's search answers
//! pages of five, so it spans four pages. Two
//! re-copies are measured: one with nothing changed on the board, which may send nothing but the
//! listing, and one after five of the twenty changed, which may read only those five — each with
//! its first page of comments in the same request, batched as the board's detail read batches —
//! and no unchanged one. Both are asserted here, and each is written to this crate's telemetry
//! directory with its listing, detail and comment-page requests and the points each spent. This
//! suite records those figures and holds none of them to a ceiling: a ceiling on them belongs to
//! a budget in this crate's `budgets.yaml`.

use onetaskgraph_github_projects::{graphql, worst_case_point_cost};
use serde_json::{Value, json};

use crate::budget_runner::directory;
use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{GitHubBoardFields, github_extra_task, github_projects_with_tasks};

/// The extra tasks beside the dataset's four, so the board holds eighty.
const EXTRA: usize = 76;
/// How many of them the query names.
const MATCHING: usize = 20;
/// How many of those change before the second measured copy.
const CHANGED: usize = 5;
/// The rows a page of the listing holds, so twenty rows span four pages.
const PAGE: u32 = 5;

/// The narrowing the copy and the list share.
const NARROWING: [&str; 6] = [
    "--source",
    "board",
    "--commented-since",
    "2026-09-01T00:00:00Z",
    "--metadata",
    "orchestrator.follow-up/root_cause=stale-cache",
];

/// What one request was, for the breakdown: listing, detail or a further comment page.
fn part(document: &str, variables: &Value) -> &'static str {
    if document == graphql::SEARCH_ISSUES || document == graphql::BOARD {
        "listing"
    } else if document == graphql::ISSUE_DETAILS
        || document == graphql::ISSUE_COPY_DETAILS
        || (document == graphql::ISSUE_DETAIL && variables["after"].is_null())
    {
        "detail"
    } else if document == graphql::ISSUE_DETAIL || document == graphql::ISSUE_COMMENTS {
        "comment_pages"
    } else {
        "other"
    }
}

/// The requests and points one command spent, in total and by part.
fn spent(served: &[(String, Value)]) -> Value {
    let mut breakdown = serde_json::Map::new();
    let (mut requests, mut points) = (0u64, 0u64);
    for (document, variables) in served {
        let cost =
            worst_case_point_cost(document).expect("every document this source sends prices");
        requests += 1;
        points += cost;
        let entry = breakdown
            .entry(part(document, variables).to_owned())
            .or_insert_with(|| json!({"requests": 0, "points": 0}));
        entry["requests"] = json!(entry["requests"].as_u64().unwrap_or(0) + 1);
        entry["points"] = json!(entry["points"].as_u64().unwrap_or(0) + cost);
    }
    json!({"requests": requests, "points": points, "breakdown": breakdown})
}

/// Write one measurement where this crate's telemetry lives.
fn record(name: &str, workload: &str, measured: &Value) {
    let directory = directory();
    std::fs::create_dir_all(&directory).expect("the telemetry directory");
    let mut written = measured.clone();
    written["workload"] = json!(workload);
    std::fs::write(
        directory.join(format!("{name}.json")),
        serde_json::to_string_pretty(&written).expect("telemetry renders"),
    )
    .expect("the telemetry file");
}

struct Plan {
    sandbox: Sandbox,
    board: GitHubBoardFields,
}

impl Plan {
    fn new() -> Self {
        let sandbox = Sandbox::new();
        let (config, board) = github_projects_with_tasks(&sandbox, EXTRA);
        let notes = sandbox.subdirectory("notes");
        std::fs::create_dir_all(notes.join("tasks")).expect("the folder");
        sandbox.project_document(
            &serde_json::to_string_pretty(&json!({
                "page_size": PAGE,
                "sources": {
                    "board": {"plugin": "github-projects", "config": config},
                    "notes": {"plugin": "local-md", "config": {"root": notes}},
                },
            }))
            .expect("a document"),
        );
        // GitHub's search answers a page of fewer rows than asked for; this one answers a
        // page of the listing's size, so twenty rows take four search requests.
        board.answer_search_pages_of(PAGE as usize);
        for n in 0..MATCHING {
            let task = github_extra_task(n);
            board.set_metadata(
                &task,
                "orchestrator.follow-up",
                json!({"root_cause": "stale-cache"}),
            );
            for comment in 0..=n % 5 {
                board.add_comment(&task, &format!("follow-up note {comment}"));
            }
        }
        // Commented on, and not a follow-up: the metadata narrowing leaves it out.
        board.add_comment(&github_extra_task(MATCHING), "unrelated activity");
        // The shared dataset's own follow-up, with no comments, is another root cause here, so
        // exactly twenty tasks match: a candidate the other narrowing keeps and no comment ever
        // touched has no copy to confirm it from and is read on every copy to be ruled out.
        board.set_metadata(
            "T-4",
            "orchestrator.follow-up",
            json!({"root_cause": "a different one"}),
        );
        Self { sandbox, board }
    }

    /// One copy by query, its report and every request the board was sent for it.
    fn copy(&self) -> (Value, Vec<(String, Value)>) {
        let before = self.board.served().len();
        let mut arguments = vec![
            "task",
            "copy",
            "--to",
            "notes",
            "--comments",
            "--no-link",
            "--json",
        ];
        arguments.extend(NARROWING);
        let output = self
            .sandbox
            .command()
            .args(&arguments)
            .assert()
            .get_output()
            .clone();
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        (
            serde_json::from_str(&stdout(&output)).expect("JSON"),
            self.board.served()[before..].to_vec(),
        )
    }
}

/// Whether a request names the issue `id` in its variables.
fn names(variables: &Value, id: &str) -> bool {
    variables
        .as_object()
        .is_some_and(|members| members.values().any(|value| value == id))
}

#[test]
fn a_recopy_by_query_reads_only_the_listing_and_then_only_what_changed() {
    let plan = Plan::new();
    let (first, _) = plan.copy();
    let items = first["items"].as_array().expect("items");
    assert_eq!(items.len(), MATCHING, "{first:#}");
    assert!(items.iter().all(|item| item["action"] == "created"));

    let (unchanged, served) = plan.copy();
    let reported = unchanged["items"].as_array().expect("items");
    assert_eq!(
        reported.len(),
        MATCHING,
        "every match is reported: {unchanged:#}"
    );
    assert!(
        reported
            .iter()
            .all(|item| item["action"] == "unchanged" && item["skipped"] == true),
        "{unchanged:#}"
    );
    let measured = spent(&served);
    assert!(
        served
            .iter()
            .all(|(document, variables)| part(document, variables) == "listing"),
        "nothing beyond the listing: {measured:#}"
    );
    // The listing walks its pages in order: the first from the start, each later one from the
    // cursor the one before it ended at, none twice.
    let cursors: Vec<&Value> = served
        .iter()
        .filter(|(document, _)| document == graphql::SEARCH_ISSUES)
        .map(|(_, variables)| &variables["after"])
        .collect();
    assert!(
        cursors.first().is_some_and(|first| first.is_null()),
        "the listing starts at its first page: {cursors:?}"
    );
    let resumed = &cursors[1..];
    assert!(
        !resumed.is_empty() && resumed.iter().all(|cursor| cursor.is_string()),
        "twenty matches at five a page are more than one page, each resumed by its cursor: \
         {cursors:?}"
    );
    assert!(
        resumed
            .iter()
            .enumerate()
            .all(|(at, cursor)| !resumed[..at].contains(cursor)),
        "no page is read twice: {cursors:?}"
    );
    record(
        "github-copy-recopy-unchanged",
        "copy-by-query-80-tasks-20-matching-unchanged",
        &measured,
    );

    let changed: Vec<String> = (0..CHANGED).map(github_extra_task).collect();
    for task in &changed {
        plan.board.add_comment(task, "a later word");
    }
    let (after, served) = plan.copy();
    for item in after["items"].as_array().expect("items") {
        let source = item["source"].as_str().expect("a source");
        let was_changed = changed.iter().any(|task| source == format!("board:{task}"));
        assert_eq!(
            item["skipped"] == true,
            !was_changed,
            "{source}: read again exactly when it changed: {item:#}"
        );
    }
    for n in CHANGED..MATCHING {
        let task = github_extra_task(n);
        assert!(
            !served.iter().any(|(document, variables)| {
                part(document, variables) != "listing" && names(variables, &task)
            }),
            "{task} did not change and was not read"
        );
    }
    let measured = spent(&served);
    assert!(
        measured["breakdown"].get("comment_pages").is_none(),
        "no item's comments outran its first page: {measured:#}"
    );
    assert!(
        measured["breakdown"].get("other").is_none(),
        "no item cost a request beyond its share of the batched read: {measured:#}"
    );
    assert!(
        served.iter().any(|(document, variables)| {
            document == graphql::ISSUE_COPY_DETAILS
                && changed.iter().all(|task| names(variables, task))
        }),
        "the five changed items were read together, each with its first comment page and its \
         edges, in one batched read: {measured:#}"
    );
    record(
        "github-copy-recopy-five-changed",
        "copy-by-query-80-tasks-20-matching-5-changed",
        &measured,
    );
}
