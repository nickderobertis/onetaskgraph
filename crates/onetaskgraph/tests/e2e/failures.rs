//! Failure and recovery: every way a user can get this wrong, and what they are told.
//!
//! Each one exits non-zero, names the exact problem on standard error, and says what to
//! do about it. A message that says only what went wrong leaves the user where they
//! started.

use std::process::Output;

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{ROWS, dataset, document, qualified};
use serde_json::json;

/// A sandbox holding one working `in-memory` source called `work`.
fn host() -> Sandbox {
    let sandbox = Sandbox::new();
    let mut block = dataset();
    block["capabilities"] = json!({"max_page_size": 50});
    sandbox.project_document(&document(
        &json!({"work": {"plugin": ROWS[0].plugin, "config": block}}),
    ));
    sandbox
}

/// Run the binary and return what it did.
fn run(sandbox: &Sandbox, arguments: &[&str]) -> Output {
    sandbox
        .command()
        .args(arguments)
        .assert()
        .get_output()
        .clone()
}

/// Assert this run failed, said `problem`, and suggested `next`.
fn refused(output: &Output, problem: &str, next: &str) {
    assert_ne!(
        output.status.code(),
        Some(0),
        "this must not succeed:\n{}",
        stdout(output)
    );
    let complaint = stderr(output);
    assert!(
        complaint.contains(problem),
        "the message does not name the problem ({problem}):\n{complaint}"
    );
    assert!(
        complaint.contains(next),
        "the message suggests no next action ({next}):\n{complaint}"
    );
}

#[test]
fn a_source_name_nothing_configures_is_refused_with_the_names_that_exist() {
    let output = run(&host(), &["task", "list", "--source", "elsewhere"]);
    refused(&output, "elsewhere", "sources list");
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn a_configuration_that_will_not_parse_is_refused_where_it_is_written() {
    let sandbox = Sandbox::new();
    sandbox.project_document("sources:\n  work:\n   plugin: [this is not a plugin name\n");
    let output = run(&sandbox, &["task", "list"]);
    refused(&output, "not valid YAML", "correct the syntax");
}

#[test]
fn a_setting_no_plugin_answers_to_is_refused_by_the_key_that_names_it() {
    let sandbox = Sandbox::new();
    sandbox.project_document(&document(
        &json!({"work": {"plugin": "jira", "config": {}}}),
    ));
    let output = run(&sandbox, &["task", "list"]);
    refused(&output, "sources.work.plugin", "use one of");
}

#[test]
fn an_id_that_names_nothing_is_refused_and_says_where_to_look() {
    let sandbox = host();

    let missing = run(&sandbox, &["task", "show", &qualified("work", "NOPE")]);
    refused(&missing, "no task with that id", "task list");
    assert_eq!(missing.status.code(), Some(1));

    let no_project = run(&sandbox, &["project", "show", &qualified("work", "NOPE")]);
    refused(&no_project, "no project with that id", "project list");

    // Unqualified, which is a different mistake and gets a different message.
    let unqualified = run(&sandbox, &["task", "show", "T-1"]);
    refused(&unqualified, "is not a qualified id", "sources list");

    // And an id whose source is not configured at all.
    let elsewhere = run(&sandbox, &["task", "show", "elsewhere:T-1"]);
    refused(&elsewhere, "elsewhere", "sources list");
}

#[test]
fn a_source_that_cannot_answer_exits_four_and_names_itself() {
    let sandbox = Sandbox::new();
    sandbox.project_document(&document(
        &json!({"gone": {"plugin": "linear", "config": {}}}),
    ));

    let output = run(&sandbox, &["task", "list"]);
    assert_eq!(output.status.code(), Some(4));
    refused(&output, "gone", "--allow-partial");
}

#[test]
fn a_page_token_this_engine_did_not_issue_is_refused_rather_than_restarting_the_walk() {
    let output = run(&host(), &["task", "list", "--page", "not-a-token"]);
    refused(&output, "not issued by this engine", "previous page");
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn a_page_of_no_rows_is_refused_as_the_typing_mistake_it_is() {
    let output = run(&host(), &["task", "list", "--limit", "0"]);
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("invalid value '0' for '--limit"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn a_source_name_that_could_not_name_anything_is_refused_for_being_unusable() {
    // Distinct from a name nothing configures: this one could never be a source name at
    // all, because the `ONETASKGRAPH_SOURCES__<NAME>__` mapping would be ambiguous if it
    // were.
    let output = run(&host(), &["task", "list", "--source", "Work_One"]);
    refused(&output, "--source Work_One", "sources list");
}

#[test]
fn a_configuration_with_no_sources_at_all_says_what_to_add() {
    let sandbox = Sandbox::new();
    sandbox.project_document("page_size: 10\n");
    refused(
        &run(&sandbox, &["task", "list"]),
        "no sources",
        "onetaskgraph.yaml",
    );
}
