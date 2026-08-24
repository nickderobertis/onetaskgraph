//! Failure and recovery: every way a user can get this wrong, and what they are told.
//!
//! Each one exits non-zero, names the exact problem on standard error, and says what to
//! do about it. A message that says only what went wrong leaves the user where they
//! started.

use std::process::Output;

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{ROWS, dataset, document, qualified};
use serde_json::{Value, json};

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

/// A page token whose streams are `streams` and whose query is the one a plain
/// `task list` over `sandbox` really carries.
///
/// The query half is lifted from a token the binary itself minted rather than computed
/// here: the fingerprint is the engine's, a test that recomputed it would be asserting
/// against its own arithmetic, and one that drifted would pass for the wrong reason.
/// These journeys are about the *streams* half, so they need the other half to be real.
fn token_over(sandbox: &Sandbox, streams: &Value) -> String {
    let minted: Value = serde_json::from_str(&stdout(&run(
        sandbox,
        &["task", "list", "--limit", "1", "--json"],
    )))
    .expect("--json emits a response");
    let carried = minted["next"]
        .as_str()
        .expect("one page of four leaves more");
    let decoded: Value = serde_json::from_slice(&unhex(carried)).expect("a token holds JSON");

    let document = serde_json::to_string(&json!({
        "query": decoded["query"].clone(),
        "streams": streams,
    }))
    .expect("a resume document renders");
    document
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The bytes a lower-case hex token spells.
fn unhex(raw: &str) -> Vec<u8> {
    raw.as_bytes()
        .chunks(2)
        .map(|pair| {
            u8::from_str_radix(std::str::from_utf8(pair).expect("hex is ascii"), 16)
                .expect("a token is hex")
        })
        .collect()
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
fn a_page_token_this_engine_did_not_write_is_refused_rather_than_restarting_the_walk() {
    let output = run(&host(), &["task", "list", "--page", "not-a-token"]);
    refused(
        &output,
        "not a page token this engine writes",
        "previous page",
    );
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn a_well_shaped_token_this_configuration_cannot_honour_is_refused_for_what_it_says() {
    // The dangerous case, and the one a malformed string does not reach: a token that
    // decodes perfectly and asks for something this configuration has no answer for.
    // Silently ignoring any of these would resume half a walk and look like a short page.
    //
    // Each carries the real query fingerprint of the walk it is handed to, so what is
    // under test is the streams it names rather than the check that it belongs to this
    // query at all — that one has a journey of its own below.
    let sandbox = host();
    for (streams, problem) in [
        (
            json!([{"source": "elsewhere", "stream": "items"}]),
            "does not have",
        ),
        (
            json!([{"source": "work", "stream": "items", "skip": 999}]),
            "serves at most",
        ),
        (
            json!([
                {"source": "work", "stream": "items"},
                {"source": "work", "stream": "items", "skip": 1},
            ]),
            "two places to resume",
        ),
        // The one that would otherwise exit 0 with an empty page: a token a `search`
        // minted, handed to `task list`, which reads neither of the streams it names.
        (
            json!([{"source": "work", "stream": "tasks"}]),
            "this command does not read",
        ),
    ] {
        let token = token_over(&sandbox, &streams);
        let output = run(&sandbox, &["task", "list", "--page", &token]);
        assert_eq!(
            output.status.code(),
            Some(1),
            "{streams} must be refused:\n{}",
            stdout(&output)
        );
        refused(&output, problem, "the same configuration");
    }
}

#[test]
fn a_token_owing_the_next_row_to_a_stream_it_does_not_resume_is_refused() {
    // The stream owed the next row decides where the page picks its turns up. One naming
    // a stream the token does not resume is a value from outside with no reading — the
    // merge would quietly begin at the first stream instead, which is an answer in an
    // order the token did not ask for.
    let sandbox = host();
    let minted: Value = serde_json::from_str(&stdout(&run(
        &sandbox,
        &["task", "list", "--limit", "1", "--json"],
    )))
    .expect("--json emits a response");
    let carried = minted["next"]
        .as_str()
        .expect("one page of four leaves more");
    let mut document: Value = serde_json::from_slice(&unhex(carried)).expect("a token holds JSON");
    document["owed"] = json!({"source": "elsewhere", "stream": "items"});

    let forged: String = serde_json::to_string(&document)
        .expect("a resume document renders")
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();

    let output = run(&sandbox, &["task", "list", "--page", &forged]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "a token owing a stream it does not resume must be refused:\n{}",
        stdout(&output)
    );
    refused(&output, "does not resume", "the same configuration");
}

#[test]
fn a_token_from_one_query_is_refused_by_another_rather_than_resuming_it_somewhere_else() {
    // Every cursor in a token is an offset into the result set *one* query produced. Hand
    // one to a different query and each source picks up at a position that meant
    // something in a walk the caller is no longer doing — and what comes back is real
    // rows at exit zero, with nothing about the answer to say it is arbitrary. That is
    // the worst shape this can take, so it is refused rather than served.
    let sandbox = host();
    let minted: Value = serde_json::from_str(&stdout(&run(
        &sandbox,
        &["task", "list", "--label", "bug", "--limit", "1", "--json"],
    )))
    .expect("--json emits a response");
    let token = minted["next"].as_str().expect("two tasks carry that label");

    // The same query resumes, so what follows refuses a mismatch rather than every token.
    let same = run(
        &sandbox,
        &[
            "task", "list", "--label", "bug", "--limit", "1", "--page", token,
        ],
    );
    assert_eq!(
        same.status.code(),
        Some(0),
        "the walk it came from must still resume:\n{}",
        stderr(&same)
    );

    for arguments in [
        vec!["task", "list", "--label", "core", "--limit", "1"],
        vec!["task", "list", "--limit", "1"],
        vec![
            "task", "list", "--label", "bug", "--status", "todo", "--limit", "1",
        ],
        vec![
            "task", "list", "--label", "bug", "--search", "alpha", "--limit", "1",
        ],
        vec!["project", "list", "--limit", "1"],
        vec!["label", "list", "--limit", "1"],
    ] {
        let mut with_token = arguments.clone();
        with_token.extend(["--page", token]);
        let output = run(&sandbox, &with_token);
        assert_eq!(
            output.status.code(),
            Some(1),
            "`{}` must not resume a token another query minted:\n{}",
            arguments.join(" "),
            stdout(&output)
        );
        refused(&output, "written by a different query", "drop `--page`");
    }
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
