//! How many TCP connections the binary opens to the loopback board, and what one closed under
//! it leaves.
//!
//! A connection per request is what ran the Windows runner out of ports and failed `task copy
//! plans:B plans:C --to board` against a board that was listening; see
//! `onetaskgraph_e2e_support::loopback`. Each journey here drives that command's own sequence
//! — a fresh board, then the copy — and holds the binary to the one connection it keeps alive.

use std::process::Output;

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{GitHubBoardFields, document, github_projects_with_board};

/// A fresh board configured as `board`, and a folder of Markdown as `plans` holding the two
/// tasks the failing copy named.
fn setup() -> (Sandbox, GitHubBoardFields) {
    let sandbox = Sandbox::new();
    let root = sandbox.subdirectory("plans");
    std::fs::create_dir_all(root.join("tasks")).expect("the task folder");
    for (name, title) in [("B", "Second"), ("C", "Third")] {
        std::fs::write(
            root.join(format!("tasks/{name}.md")),
            format!("---\ntitle: {title}\nstatus: Todo\n---\n{title} body\n"),
        )
        .expect("a task file");
    }
    let (config, board) = github_projects_with_board(&sandbox);
    sandbox.project_document(&document(&json!({
        "plans": {"plugin":"local-md","config":{
            "root": root,
            "status_mapping": {"Todo":"todo","Doing":"in-progress"}}},
        "board": {"plugin":"github-projects","config":config}
    })));
    (sandbox, board)
}

fn run(sandbox: &Sandbox, arguments: &[&str]) -> Output {
    sandbox
        .command()
        .args(arguments)
        .assert()
        .get_output()
        .clone()
}

/// The JSON a command printed, having had to succeed.
fn ok(sandbox: &Sandbox, arguments: &[&str]) -> Value {
    let output = run(sandbox, arguments);
    assert_eq!(
        output.status.code(),
        Some(0),
        "`onetaskgraph {}` failed\n{}",
        arguments.join(" "),
        stderr(&output)
    );
    serde_json::from_str(&stdout(&output)).expect("the command emits JSON")
}

const COPY: &[&str] = &[
    "task", "copy", "plans:B", "plans:C", "--to", "board", "--json",
];

#[test]
fn a_copy_sends_every_request_over_the_one_connection_it_keeps_alive() {
    let (sandbox, board) = setup();
    let copied = ok(&sandbox, COPY);
    let requests = board.served().len();
    assert!(
        requests > 2,
        "the copy is several requests, which is what makes the count mean anything: {requests}"
    );
    assert_eq!(
        board.connections(),
        1,
        "the copy's {requests} requests each opened a connection rather than sharing one"
    );
    // A later invocation is a new process, so a new connection, after the first one's
    // process closed its own; the board goes on answering on it.
    let b = copied["items"][0]["destination"]
        .as_str()
        .expect("the copy names where B landed");
    let shown = ok(&sandbox, &["task", "show", b, "--no-comments", "--json"]);
    assert_eq!(shown["items"][0]["item"]["title"], "Second", "{shown}");
    assert_eq!(board.connections(), 2, "one connection per invocation");
}

#[test]
fn a_connection_the_board_closes_unanswered_is_replaced_and_the_copy_lands() {
    let (sandbox, board) = setup();
    // The copy's first request, the read of what the board already holds: closing it
    // unanswered applies nothing, and the binary reads the board again its other way.
    board.drop_response_once("$originFirst:Int!");
    let copied = ok(&sandbox, COPY);
    assert_eq!(
        copied["items"].as_array().map(Vec::len),
        Some(2),
        "{copied}"
    );
    assert_eq!(
        board.connections(),
        2,
        "the connection closed under the binary is the one it stops using, and every request \
         after it shares the one that replaced it"
    );
    let b = copied["items"][0]["destination"]
        .as_str()
        .expect("the copy names where B landed");
    let shown = ok(&sandbox, &["task", "show", b, "--no-comments", "--json"]);
    assert_eq!(shown["items"][0]["item"]["title"], "Second", "{shown}");
    assert_eq!(board.connections(), 3);
}
