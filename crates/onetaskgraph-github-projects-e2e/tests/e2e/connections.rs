//! How many TCP connections the binary opens to the loopback board, and what one closed under
//! it leaves; `onetaskgraph_e2e_support::loopback` says why that count matters. Each journey
//! drives the sequence of the copy that failed on Windows — a fresh board, then `task copy
//! plans:B plans:C --to board` — and holds the binary to the one connection it keeps alive.

use std::process::Output;

use onetaskgraph_e2e_support::loopback;
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

#[test]
fn a_change_made_part_way_through_a_copy_sees_the_connection_the_copy_holds() {
    let (sandbox, board) = setup();
    let seen = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let recorded = std::sync::Arc::clone(&seen);
    board.after_request(
        |_, _| true,
        0,
        move |during| {
            recorded.store(during.connections(), std::sync::atomic::Ordering::SeqCst);
        },
    );
    ok(&sandbox, COPY);
    assert_eq!(
        seen.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the board a change is handed counts the connection the copy's first request came on"
    );
}

/// A board configured in `sandbox` at a loopback server answering every request with
/// `answer`.
fn board_answering(
    sandbox: &Sandbox,
    answer: impl FnMut(&loopback::Request, &mut std::net::TcpStream) -> loopback::After + Send + 'static,
) -> Value {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a loopback listener");
    let endpoint = format!(
        "http://{}/graphql",
        listener.local_addr().expect("an address")
    );
    loopback::serve_with(listener, answer);
    let mut board = crate::fixtures::github_projects_unreachable(sandbox);
    board["endpoint"] = Value::String(endpoint);
    board
}

/// What a command reading `board`, configured in `sandbox`, said on standard error, having had
/// to fail.
fn unreached(sandbox: &Sandbox, board: &Value) -> String {
    sandbox.project_document(&document(&json!({
        "board": {"plugin":"github-projects","config":board}
    })));
    let output = run(sandbox, &["task", "list", "--source", "board", "--json"]);
    assert_ne!(output.status.code(), Some(0), "{}", stdout(&output));
    stderr(&output)
}

#[test]
fn a_connection_closed_before_its_answer_is_named_beneath_the_failed_send() {
    let sandbox = Sandbox::new();
    let board = board_answering(&sandbox, |_, _| loopback::After::Close);
    let said = unreached(&sandbox, &board);
    assert!(
        said.contains("GitHub GraphQL request failed: error sending request for url")
            && said.contains("connection closed before message completed"),
        "the failure names the client's error and the cause beneath it: {said}"
    );
}

#[test]
fn an_answer_cut_short_is_named_beneath_the_failed_read() {
    let sandbox = Sandbox::new();
    let board = board_answering(&sandbox, |_, stream| {
        use std::io::Write;
        // Ten bytes of the hundred it declares, and then the connection ends.
        let _ = stream.write_all(
            b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 100\r\n\r\n{\"data\":{",
        );
        loopback::After::Close
    });
    let said = unreached(&sandbox, &board);
    assert!(
        said.contains("GitHub GraphQL response could not be read: error decoding response body:"),
        "the failure names the cause beneath the client's error: {said}"
    );
}

#[test]
fn a_refused_connection_carries_the_operating_systems_own_error() {
    let sandbox = Sandbox::new();
    let board = crate::fixtures::github_projects_unreachable(&sandbox);
    let said = unreached(&sandbox, &board);
    assert!(
        said.contains("error sending request for url") && said.contains("(os error "),
        "the failure names the socket error the operating system gave: {said}"
    );
}
