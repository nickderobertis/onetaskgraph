//! `task list --commented-since`, driven the way a user drives it: the compiled binary over
//! two folders of Markdown, in process and over the stdio plugin protocol alike.
//!
//! Each folder holds tasks commented on after the instant, never commented on, commented on
//! only before it, and one whose old comment was edited after it — so every answer below is
//! exactly one set, and a filter that kept too much or too little names the task it got wrong.

use std::process::Output;

use serde_json::{Value, json};

use crate::common::{SOURCE_BOUNDARIES, Sandbox, SourceBoundary, stderr, stdout};
use crate::fixtures::{document, empty_folder};

/// The instant every query below asks about.
const SINCE: &str = "2026-09-20T12:00:00Z";

/// Long before [`SINCE`].
const OLD: &str = "2026-09-01T09:00:00Z";

/// One task file at `status`, with a comment for each `(created_at, updated_at)` pair.
fn task(status: &str, comments: &[(&str, &str)]) -> String {
    let mut text = format!("---\ntitle: A task\nstatus: {status}\n---\nThe body.\n");
    if !comments.is_empty() {
        text.push_str("\n## Comments\n");
        for (index, (created, updated)) in comments.iter().enumerate() {
            text.push_str(&format!(
                "\n<!-- onetaskgraph:comment id=\"c-{index}\" author=\"ada\" \
                 created_at=\"{created}\" updated_at=\"{updated}\" -->\n\
                 ### ada — {created}\n\nA word.\n\n<!-- /onetaskgraph:comment -->\n"
            ));
        }
    }
    text
}

/// Two folders, `home` and `away`, configured on `boundary`.
///
/// - `home:new` — open, commented on after the instant.
/// - `home:silent` — open, never commented on.
/// - `home:old` — open, commented on and edited only before the instant.
/// - `home:edited` — shipped, commented on long before, that comment edited after the instant.
/// - `away:fresh` — shipped, commented on after the instant.
/// - `away:stale` — open, commented on only before the instant.
fn host(boundary: SourceBoundary) -> Sandbox {
    let sandbox = Sandbox::new();
    let files: [(&str, &str, String); 6] = [
        (
            "home",
            "new",
            task("todo", &[("2026-09-21T09:00:00Z", "2026-09-21T09:00:00Z")]),
        ),
        ("home", "silent", task("todo", &[])),
        ("home", "old", task("todo", &[(OLD, "2026-09-02T09:00:00Z")])),
        (
            "home",
            "edited",
            task("shipped", &[(OLD, "2026-09-25T09:00:00Z")]),
        ),
        (
            "away",
            "fresh",
            task("shipped", &[("2026-09-22T09:00:00Z", "2026-09-22T09:00:00Z")]),
        ),
        ("away", "stale", task("todo", &[(OLD, OLD)])),
    ];
    let mut sources = serde_json::Map::new();
    for folder in ["home", "away"] {
        sources.insert(
            folder.to_owned(),
            boundary.source("local-md", empty_folder(&sandbox, folder)),
        );
    }
    for (folder, name, text) in files {
        let tasks = sandbox.subdirectory(&format!("{folder}/tasks"));
        std::fs::write(tasks.join(format!("{name}.md")), text).expect("a task file");
    }
    sandbox.project_document(&document(&Value::Object(sources)));
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

/// The qualified ids `task list` answered with, sorted, for a run that had to succeed.
fn listed(sandbox: &Sandbox, filters: &[&str]) -> Vec<String> {
    let mut arguments = vec!["--json", "task", "list", "--limit", "50"];
    arguments.extend_from_slice(filters);
    let output = run(sandbox, &arguments);
    assert_eq!(
        output.status.code(),
        Some(0),
        "`onetaskgraph {}` exited {:?}\n{}",
        arguments.join(" "),
        output.status.code(),
        stderr(&output)
    );
    let answer: Value = serde_json::from_str(&stdout(&output)).expect("a JSON answer");
    let mut ids: Vec<String> = answer["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|item| item["id"].as_str().expect("a qualified id").to_owned())
        .collect();
    ids.sort();
    ids
}

#[test]
fn commented_since_keeps_exactly_the_tasks_with_a_comment_created_or_edited_since() {
    for boundary in SOURCE_BOUNDARIES {
        let sandbox = host(boundary);
        assert_eq!(
            listed(&sandbox, &["--commented-since", SINCE]),
            ["away:fresh", "home:edited", "home:new"],
            "{boundary:?}"
        );
        // The same instant in another offset is the same instant.
        assert_eq!(
            listed(&sandbox, &["--commented-since", "2026-09-20T08:00:00-04:00"]),
            ["away:fresh", "home:edited", "home:new"],
            "{boundary:?}"
        );
        assert_eq!(
            listed(&sandbox, &[]).len(),
            6,
            "{boundary:?}: no instant is no filter"
        );
    }
}

#[test]
fn commented_since_with_a_status_or_a_source_is_exactly_the_intersection() {
    for boundary in SOURCE_BOUNDARIES {
        let sandbox = host(boundary);
        assert_eq!(
            listed(&sandbox, &["--commented-since", SINCE, "--status", "todo"]),
            ["home:new"],
            "{boundary:?}"
        );
        assert_eq!(
            listed(&sandbox, &["--commented-since", SINCE, "--status", "done"]),
            ["away:fresh", "home:edited"],
            "{boundary:?}"
        );
        assert_eq!(
            listed(&sandbox, &["--commented-since", SINCE, "--source", "away"]),
            ["away:fresh"],
            "{boundary:?}"
        );
        assert_eq!(
            listed(
                &sandbox,
                &[
                    "--commented-since",
                    SINCE,
                    "--source",
                    "home",
                    "--status",
                    "done"
                ]
            ),
            ["home:edited"],
            "{boundary:?}"
        );
    }
}

#[test]
fn commented_since_is_pushed_down_to_a_folder_of_markdown_and_the_plan_says_so() {
    let sandbox = host(SourceBoundary::Direct);
    let output = run(
        &sandbox,
        &[
            "--json",
            "task",
            "list",
            "--commented-since",
            SINCE,
            "--source",
            "home",
        ],
    );
    let answer: Value = serde_json::from_str(&stdout(&output)).expect("a JSON answer");
    assert_eq!(
        answer["plan"]["per_source"][0]["pushed_down"],
        json!(["commented-since"])
    );
}

#[test]
fn an_instant_without_an_offset_or_that_does_not_parse_is_refused_naming_the_flag() {
    let sandbox = host(SourceBoundary::Direct);
    for value in ["2026-09-20T12:00:00", "yesterday", "2026-09-20"] {
        let output = run(&sandbox, &["task", "list", "--commented-since", value]);
        let said = stderr(&output);
        assert_eq!(output.status.code(), Some(2), "{value}: {said}");
        assert!(
            said.contains("--commented-since") && said.contains(value),
            "{value}: the refusal names the flag and the value: {said}"
        );
        assert!(
            said.contains("RFC 3339"),
            "{value}: the refusal says what would be accepted: {said}"
        );
        assert!(stdout(&output).is_empty(), "{value}: nothing was listed");
    }
}
