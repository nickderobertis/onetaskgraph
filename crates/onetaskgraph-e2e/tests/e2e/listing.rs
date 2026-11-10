//! `task list --all`: every page of a listing at once, against the loopback GitHub board holding
//! more items than three pages, and refused, naming the token, against a board that repeats a
//! cursor rather than walking it forever.

use std::collections::BTreeSet;

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{GitHubBoardFields, document, github_projects_with_board};

fn run(sandbox: &Sandbox, arguments: &[&str], code: i32) -> Value {
    let mut full = vec!["--json"];
    full.extend(arguments);
    let output = sandbox.command().args(&full).assert().get_output().clone();
    assert_eq!(
        output.status.code(),
        Some(code),
        "`onetaskgraph {}`\nstdout:\n{}\nstderr:\n{}",
        full.join(" "),
        stdout(&output),
        stderr(&output)
    );
    serde_json::from_str(&stdout(&output)).expect("the run answers JSON")
}

/// A board holding `extra` more tasks than the shared dataset, each a person's issue.
fn crowded(extra: usize) -> (Sandbox, GitHubBoardFields) {
    let sandbox = Sandbox::new();
    let (config, board) = github_projects_with_board(&sandbox);
    sandbox.secrets_file("GITHUB_PROJECTS_FIXTURE_TOKEN=test-token\n");
    sandbox.project_document(&document(&json!({
        "board": {"plugin": "github-projects", "config": config},
    })));
    let template = board
        .issues()
        .into_iter()
        .find(|issue| issue["parent"].is_null() && issue["type"] == "Issue")
        .expect("an issue to copy");
    let mut added = Vec::new();
    for n in 0..extra {
        let mut issue = template.clone();
        issue["id"] = json!(format!("MANY-{n}"));
        issue["item"] = json!(format!("ITEM-MANY-{n}"));
        issue["title"] = json!(format!("Many {n}"));
        issue["body"] = json!("");
        added.push(issue);
    }
    board.add_items(added);
    (sandbox, board)
}

fn ids(answer: &Value) -> Vec<String> {
    answer["items"]
        .as_array()
        .expect("a list")
        .iter()
        .map(|task| task["id"].as_str().expect("an id").to_owned())
        .collect()
}

#[test]
fn every_page_of_a_board_listing_more_than_three_pages_long_is_answered_once() {
    let (sandbox, _board) = crowded(12);
    let every = run(
        &sandbox,
        &["task", "list", "--source", "board", "--all", "--limit", "3"],
        0,
    );
    let listed = ids(&every);
    let unique: BTreeSet<&String> = listed.iter().collect();
    assert_eq!(unique.len(), listed.len(), "each item once: {listed:?}");
    assert!(
        listed.len() > 9,
        "more than three pages of three: {listed:?}"
    );
    assert!(every["next"].is_null());
}

#[test]
fn a_board_repeating_its_cursor_is_refused_rather_than_walked_forever() {
    // More items than one of the board's own pages holds, so its read reaches a second page.
    let (sandbox, board) = crowded(120);
    board.stall_the_items_cursor();
    let output = sandbox
        .command()
        .args(["--json", "task", "list", "--source", "board", "--all"])
        .timeout(std::time::Duration::from_secs(60))
        .assert()
        .get_output()
        .clone();
    assert_ne!(output.status.code(), Some(0), "{}", stdout(&output));
    let answer: Value = serde_json::from_str(&stdout(&output)).expect("JSON");
    let said = format!("{answer}");
    assert!(
        said.contains("did not advance") || said.contains("repeated-page"),
        "the repeated cursor is named: {answer:#}\n{}",
        stderr(&output)
    );
}
