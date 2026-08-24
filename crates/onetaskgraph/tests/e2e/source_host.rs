//! The hidden command on the main program: the plugin side of the stdio protocol.
//!
//! Every journey in this suite already drives it, because the shared fixture table
//! configures a source through it — but always as a child of the engine, where the engine
//! decides what it is sent and never lets it fail. These journeys drive it directly, the
//! way a plugin author reading `docs/plugin-protocol.md` beside it would: one line in, one
//! line out, and what it does when it cannot write the line it owes.

use std::io::Write;
use std::process::{Command, Stdio};

use serde_json::{Value, json};

/// The shipped host, ready to be given a connection on its standard input.
fn source_host() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_onetaskgraph"));
    command.args(["plugin-serve", "in-memory"]);
    command
}

#[test]
fn plugin_serve_is_hidden_from_normal_help() {
    let output = Command::new(env!("CARGO_BIN_EXE_onetaskgraph"))
        .arg("--help")
        .output()
        .expect("the main command runs");

    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).expect("help is UTF-8");
    assert!(
        !help.contains("plugin-serve"),
        "hidden command leaked: {help}"
    );
}

#[test]
fn plugin_serve_refuses_a_source_this_build_does_not_have() {
    let output = Command::new(env!("CARGO_BIN_EXE_onetaskgraph"))
        .args(["plugin-serve", "missing"])
        .output()
        .expect("the main command runs");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let complaint = String::from_utf8(output.stderr).expect("diagnostic is UTF-8");
    assert!(
        complaint.contains("no plugin of this build is called \"missing\""),
        "{complaint}"
    );
    assert!(complaint.contains("in-memory"), "{complaint}");
}

/// One `initialize` request over a source of one task.
fn handshake() -> Value {
    json!({
        "id": "0",
        "method": "initialize",
        "params": {
            "protocol_version": 1,
            "engine": {"name": "onetaskgraph", "version": "0.1.0"},
            "source_name": "work",
            "config": {"tasks": [{
                    "id": "T-1", "title": "Alpha",
                    "status": {"category": "todo", "name": "Todo"}, "labels": []
                }]},
            "secrets": {}
        }
    })
}

#[test]
fn the_shipped_host_answers_a_connection_on_its_standard_input_and_exits_zero() {
    let mut child = source_host()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the host runs");
    let mut input = child.stdin.take().expect("stdin was piped");
    writeln!(input, "{}", handshake()).expect("the host is listening");
    writeln!(
        input,
        "{}",
        json!({"id": "1", "method": "labels", "params": {"page": {"limit": 5}}})
    )
    .expect("the host is listening");
    // Closing standard input is step 4 of §1.2: the host finishes what it has and exits.
    drop(input);

    let output = child.wait_with_output().expect("the host finishes");

    assert_eq!(output.status.code(), Some(0), "the host exits cleanly");
    let answered = String::from_utf8(output.stdout).expect("responses are UTF-8");
    let lines: Vec<Value> = answered
        .lines()
        .map(|line| serde_json::from_str(line).expect("one JSON object per line"))
        .collect();
    assert_eq!(lines.len(), 2, "one answer per request: {answered}");
    assert_eq!(lines[0]["id"], "0");
    assert_eq!(lines[0]["result"]["protocol_version"], 1);
    assert_eq!(lines[0]["result"]["kind"], "in-memory");
    assert_eq!(lines[1]["id"], "1");
    assert!(lines[1]["result"]["items"].is_array(), "{answered}");
    assert!(
        String::from_utf8_lossy(&output.stderr).is_empty(),
        "a successful call writes nothing to standard error (§1)"
    );
}

#[test]
fn the_shipped_host_reports_malformed_plugin_settings_on_the_wire() {
    let mut asked = handshake();
    asked["params"]["config"] = json!({"tasks": "not a task list"});

    let mut child = source_host()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("the host runs");
    let mut input = child.stdin.take().expect("stdin was piped");
    writeln!(input, "{asked}").expect("the host is listening");
    drop(input);

    let output = child.wait_with_output().expect("the host finishes");

    assert_eq!(output.status.code(), Some(0));
    let answered = String::from_utf8(output.stdout).expect("UTF-8");
    let refusal: Value = serde_json::from_str(answered.trim()).expect("one JSON object");
    assert_eq!(refusal["id"], "0");
    assert_eq!(refusal["error"]["kind"], "config");
    assert!(
        refusal["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("source work") && message.contains("sequence")),
        "{answered}"
    );
}

#[test]
fn a_protocol_version_the_shipped_host_does_not_know_is_refused_and_it_exits_zero() {
    let mut asked = handshake();
    asked["params"]["protocol_version"] = json!(2);

    let mut child = source_host()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("the host runs");
    let mut input = child.stdin.take().expect("stdin was piped");
    writeln!(input, "{asked}").expect("the host is listening");
    drop(input);

    let output = child.wait_with_output().expect("the host finishes");

    // §6.2: it names both versions and then exits `0`. The refusal is the answer, not a
    // crash, so the engine gets a sentence to report rather than a dead pipe to guess at.
    assert_eq!(output.status.code(), Some(0));
    let answered = String::from_utf8(output.stdout).expect("UTF-8");
    let refusal: Value = serde_json::from_str(answered.trim()).expect("one JSON object");
    assert_eq!(refusal["error"]["kind"], "config");
    let message = refusal["error"]["message"]
        .as_str()
        .expect("a message to read");
    assert!(
        message.contains("version 2") && message.contains("version 1"),
        "{message}"
    );
}

/// `/dev/full` accepts a write and then fails it with ENOSPC, which is the one portable
/// way to make a real program's standard output fail deterministically. It is Linux-only,
/// so this journey is too — the same branch is a two-line `match` whose other arm every
/// other journey in this suite takes.
#[cfg(target_os = "linux")]
#[test]
fn a_host_that_cannot_write_its_answer_exits_one_and_says_so_on_standard_error() {
    use std::fs::OpenOptions;

    let full = OpenOptions::new()
        .write(true)
        .open("/dev/full")
        .expect("/dev/full exists on Linux");

    let mut child = source_host()
        .stdin(Stdio::piped())
        .stdout(Stdio::from(full))
        .stderr(Stdio::piped())
        .spawn()
        .expect("the host runs");
    let mut input = child.stdin.take().expect("stdin was piped");
    writeln!(input, "{}", handshake()).expect("the host is listening");
    drop(input);

    let output = child.wait_with_output().expect("the host finishes");

    assert_eq!(
        output.status.code(),
        Some(1),
        "a stream it cannot write is a failure, not a usage mistake"
    );
    let complaint = String::from_utf8_lossy(&output.stderr);
    assert!(
        complaint.contains("onetaskgraph:"),
        "the program names itself: {complaint}"
    );
    assert!(!complaint.contains("panicked"), "{complaint}");
}
