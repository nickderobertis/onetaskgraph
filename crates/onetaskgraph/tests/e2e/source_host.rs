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

use crate::common::Sandbox;

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
            "protocol_version": 2,
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
    assert_eq!(lines[0]["result"]["protocol_version"], 2);
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
fn a_source_with_no_documents_declares_so_and_refuses_a_document_read_over_a_real_pipe() {
    // The whole of what "honest" means for documents, driven through the shipped binary and
    // over a genuine pipe: the handshake says this source has none, and a document read that
    // arrives anyway is *refused, naming the plugin*, rather than answered with an empty
    // page. An empty page is the one wrong answer here — it is indistinguishable from a
    // source that has documents and holds none matching, and a caller cannot tell the two
    // apart afterwards.
    let mut child = source_host()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the host runs");
    let mut input = child.stdin.take().expect("stdin was piped");
    writeln!(input, "{}", handshake()).expect("the host is listening");
    for asked in [
        json!({"id": "1", "method": "get_document", "params": {"id": "D-1"}}),
        json!({"id": "2", "method": "query_documents", "params": {
            "query": {
                "text": null,
                "labels": {"any_of": [], "all_of": [], "none_of": []},
                "project": "any"
            },
            "page": {"cursor": null, "limit": 5}
        }}),
    ] {
        writeln!(input, "{asked}").expect("the host is listening");
    }
    drop(input);

    let output = child.wait_with_output().expect("the host finishes");

    assert_eq!(output.status.code(), Some(0), "the host exits cleanly");
    let answered = String::from_utf8(output.stdout).expect("responses are UTF-8");
    let lines: Vec<Value> = answered
        .lines()
        .map(|line| serde_json::from_str(line).expect("one JSON object per line"))
        .collect();
    assert_eq!(lines.len(), 3, "one answer per request: {answered}");

    // The declaration a plugin makes once, at the handshake, and the engine reads once.
    assert_eq!(
        lines[0]["result"]["capabilities"]["documents"],
        json!("unsupported"),
        "{answered}"
    );

    for refusal in &lines[1..] {
        assert!(
            refusal.get("result").is_none(),
            "a document read must not be answered at all: {refusal}"
        );
        assert_eq!(refusal["error"]["kind"], json!("refused"), "{refusal}");
        assert_eq!(
            refusal["error"]["message"],
            json!("the in-memory plugin has no documents"),
            "the refusal names the plugin behind the pipe: {refusal}"
        );
    }
}

#[test]
fn a_protocol_version_the_shipped_host_does_not_know_is_refused_and_it_exits_zero() {
    let mut asked = handshake();
    asked["params"]["protocol_version"] = json!(3);

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
        message.contains("version 3") && message.contains("version 2"),
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

/// One plugin of this build, hosted over a fixture the shared journeys already use.
///
/// The credential travels in the handshake rather than through `sandbox`, which is here
/// only because the fixtures take one; the listener each starts outlives it.
struct Hosted {
    kind: &'static str,
    config: Value,
    secrets: Value,
}

impl Hosted {
    /// Both sources whose recorded tail is reachable by cursor, each over a `T-1` that
    /// records one far end in a source nothing configures.
    fn every_kind(sandbox: &Sandbox) -> Vec<Self> {
        let far = json!([{"id": "elsewhere:P-9", "kind": "project"}]);
        vec![
            Self {
                kind: "linear",
                config: crate::fixtures::linear_recording(sandbox, far.clone()),
                secrets: json!({"LINEAR_API_KEY": "fixture-key"}),
            },
            Self {
                kind: "github-projects",
                config: crate::fixtures::github_projects_recording(sandbox, far),
                secrets: json!({"GITHUB_PROJECTS_FIXTURE_TOKEN": "test-token"}),
            },
        ]
    }

    fn connection(&self) -> (Command, Value) {
        let mut command = Command::new(env!("CARGO_BIN_EXE_onetaskgraph"));
        command.args(["plugin-serve", self.kind]);
        let handshake = json!({
            "id": "0",
            "method": "initialize",
            "params": {
                "protocol_version": 2,
                "engine": {"name": "onetaskgraph", "version": "0.1.0"},
                "source_name": "work",
                "config": self.config,
                "secrets": self.secrets
            }
        });
        (command, handshake)
    }
}

fn answers(mut command: Command, handshake: &Value, requests: &[Value]) -> Vec<Value> {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the host runs");
    let mut input = child.stdin.take().expect("stdin was piped");
    writeln!(input, "{handshake}").expect("the host is listening");
    for request in requests {
        writeln!(input, "{request}").expect("the host is listening");
    }
    drop(input);
    let output = child.wait_with_output().expect("the host finishes");
    assert_eq!(output.status.code(), Some(0), "the host exits cleanly");
    let answered = String::from_utf8(output.stdout).expect("responses are UTF-8");
    let lines: Vec<Value> = answered
        .lines()
        .map(|line| serde_json::from_str(line).expect("one JSON object per line"))
        .collect();
    assert_eq!(lines.len(), requests.len() + 1, "{answered}");
    assert_eq!(lines[0]["result"]["protocol_version"], 2, "{answered}");
    lines[1..].to_vec()
}

fn resumed(direction: &str, cursor: &str) -> Value {
    json!({
        "id": "1",
        "method": "task_dependencies",
        "params": {
            "id": "T-1",
            "direction": direction,
            "page": {"cursor": cursor, "limit": 50}
        }
    })
}

fn refusal(hosted: &Hosted, request: Value) -> String {
    let (command, handshake) = hosted.connection();
    let answered = answers(command, &handshake, &[request]);
    let answer = &answered[0];
    assert!(
        answer.get("result").is_none(),
        "{}: this must not be answered: {answer}",
        hosted.kind
    );
    answer["error"]["message"]
        .as_str()
        .unwrap_or_else(|| panic!("{}: an error carries a message: {answer}", hosted.kind))
        .to_owned()
}

#[test]
fn the_shipped_host_refuses_a_recorded_cursor_no_walk_of_its_own_reported() {
    // The reserved key holds forward edges and nothing else: the reverse of a recorded
    // edge is derived from the far end, never written down on the near item. So the tail
    // cursor a forward walk reports is one no reverse walk can be resuming, and serving it
    // would answer "what depends on T-1" with what T-1 depends on. A cursor under that
    // namespace which resumes nothing at all is the other way to get this wrong.
    //
    // The engine cannot reach either state — a dependency query's fingerprint carries its
    // direction, and the engine only ever replays a cursor a source itself reported — so
    // these journeys drive the host directly, the way a peer on the protocol does.
    let sandbox = Sandbox::new();
    let cursor = "onetaskgraph.depends_on:0";
    for hosted in Hosted::every_kind(&sandbox) {
        let reversed = refusal(&hosted, resumed("depended-on-by", cursor));
        assert!(reversed.contains(cursor), "{}: {reversed}", hosted.kind);
        assert!(
            reversed.contains("reverse dependency read"),
            "{}: {reversed}",
            hosted.kind
        );

        let unreadable = refusal(&hosted, resumed("depends-on", "onetaskgraph.depends_on:x"));
        assert!(
            unreadable.contains("is not a recorded-edge cursor"),
            "{}: {unreadable}",
            hosted.kind
        );

        // The same cursor in the direction that reported it still answers, so both
        // refusals are about the request rather than about a cursor this host cannot read.
        let (command, handshake) = hosted.connection();
        let answered = answers(command, &handshake, &[resumed("depends-on", cursor)]);
        let items = &answered[0]["result"]["items"];
        assert_eq!(items[0]["from"]["id"], "T-1", "{}: {items}", hosted.kind);
        assert_eq!(
            items[0]["to"]["id"], "elsewhere:P-9",
            "{}: {items}",
            hosted.kind
        );
    }
}

/// One `initialize` request over a source holding a task and the project it belongs to.
///
/// `capabilities` is the plugin's own block, which is how a removal is made to fail on
/// cue: `undeletable_ids` names what this source will not take back.
fn removal_handshake(capabilities: Value) -> Value {
    json!({
        "id": "0",
        "method": "initialize",
        "params": {
            "protocol_version": 2,
            "engine": {"name": "onetaskgraph", "version": "0.1.0"},
            "source_name": "work",
            "config": {
                "tasks": [{
                    "id": "T-1", "title": "Alpha",
                    "status": {"category": "todo", "name": "Todo"},
                    "labels": [], "project": "P-1"
                }],
                "projects": [{
                    "id": "P-1", "title": "Engine",
                    "status": {"category": "todo", "name": "Todo"}, "labels": []
                }],
                "capabilities": capabilities
            },
            "secrets": {}
        }
    })
}

#[test]
fn the_shipped_host_removes_a_task_and_a_project_and_they_are_gone_afterwards() {
    // §4.10, the half a copy that cannot finish depends on: undoing this engine's own
    // writes goes through the plugin, so over a subprocess-hosted source it goes over the
    // wire. Both removals are read back through the same connection, because a host that
    // answered `{}` and removed nothing would look identical from the answer alone.
    let answered = answers(
        source_host(),
        &removal_handshake(json!({})),
        &[
            json!({"id": "1", "method": "delete_task", "params": {"id": "T-1"}}),
            json!({"id": "2", "method": "get_task", "params": {"id": "T-1"}}),
            json!({"id": "3", "method": "delete_project", "params": {"id": "P-1"}}),
            json!({"id": "4", "method": "get_project", "params": {"id": "P-1"}}),
        ],
    );

    assert_eq!(answered[0]["result"], json!({}), "{answered:?}");
    assert_eq!(answered[1]["result"]["task"], Value::Null, "{answered:?}");
    assert_eq!(answered[2]["result"], json!({}), "{answered:?}");
    assert_eq!(
        answered[3]["result"]["project"],
        Value::Null,
        "{answered:?}"
    );
}

#[test]
fn a_removal_the_hosted_source_refuses_crosses_the_wire_as_its_own_reason() {
    // The destination that will not take an item back is what a copy reports as still
    // there, so its reason has to survive the boundary rather than becoming a transport
    // failure — and the item has to still be there afterwards.
    let answered = answers(
        source_host(),
        &removal_handshake(json!({"undeletable_ids": ["T-1"]})),
        &[
            json!({"id": "1", "method": "delete_task", "params": {"id": "T-1"}}),
            json!({"id": "2", "method": "get_task", "params": {"id": "T-1"}}),
        ],
    );

    assert_eq!(answered[0]["error"]["kind"], "refused", "{answered:?}");
    let message = answered[0]["error"]["message"]
        .as_str()
        .expect("a message to read");
    assert!(
        message.contains("will not remove T-1"),
        "the plugin's own reason names the item: {message}"
    );
    assert_eq!(answered[1]["result"]["task"]["id"], "T-1", "{answered:?}");
}
