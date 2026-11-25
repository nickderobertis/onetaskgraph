//! `Engine::maps_status` as a **Rust caller** reaches it: whether a source has a name to write a
//! status category as on an item of a kind, answered from its configuration alone.
//!
//! Every source it is asked about is built by the engine from a real configuration — a folder of
//! Markdown, an in-memory source, a GitHub board and a Linear workspace pointed at a local
//! listener, and a stdio plugin that is a real child process — and the backends are watched to
//! prove the answer costs no request: the listener is never connected to, and the child is sent
//! nothing after its handshake.

use std::net::TcpListener;
use std::path::Path;

use onetaskgraph_core::{Config, Engine, EngineError};
use onetaskgraph_plugin_api::{ItemKind, SecretResolver, SourceName, StatusCategory};
use secrecy::SecretString;
use serde_json::{Value, json};

use ItemKind::{Project, Task};
use StatusCategory::{Cancelled, Done, Draft, InReview, Queued, Todo};

/// Answers every credential a source names, so a board and a Linear source build without a
/// real key: the question asks neither backend anything.
struct AnyCredential;

impl SecretResolver for AnyCredential {
    fn get(&self, _var: &str) -> Option<SecretString> {
        Some(SecretString::from("unused".to_owned()))
    }
}

fn engine_over(sources: Value) -> Engine {
    let config =
        Config::from_document(json!({ "sources": sources })).expect("a valid configuration");
    Engine::build(&config, &AnyCredential)
}

fn name(source: &str) -> SourceName {
    SourceName::new(source).expect("a usable name")
}

fn maps(engine: &Engine, source: &str, kind: ItemKind, category: StatusCategory) -> bool {
    engine
        .maps_status(&name(source), kind, category)
        .unwrap_or_else(|error| panic!("{source} answers: {error}"))
}

/// A listener nothing is expected to connect to, and the endpoint naming it.
fn unvisited() -> (TcpListener, String) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
    listener
        .set_nonblocking(true)
        .expect("a non-blocking listener");
    let endpoint = format!(
        "http://{}/graphql",
        listener.local_addr().expect("its address")
    );
    (listener, endpoint)
}

fn assert_never_visited(listener: &TcpListener, what: &str) {
    match listener.accept() {
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
        Ok((_, from)) => panic!("{what} was sent a request, from {from}"),
        Err(error) => panic!("{what}'s listener failed: {error}"),
    }
}

#[test]
fn a_folder_of_markdown_maps_what_its_status_words_read_back_as() {
    let defaulted = tempfile::tempdir().unwrap();
    let narrowed = tempfile::tempdir().unwrap();
    let folder = |root: &Path, mapping: Option<Value>| {
        let mut config = json!({"root": root});
        if let Some(mapping) = mapping {
            config["status_mapping"] = mapping;
        }
        json!({"plugin": "local-md", "config": config})
    };
    let engine = engine_over(json!({
        "defaulted": folder(defaulted.path(), None),
        "narrowed": folder(narrowed.path(), Some(json!({"todo": "todo", "finished": "done"}))),
    }));
    for kind in [Task, Project] {
        assert!(maps(&engine, "defaulted", kind, InReview), "{kind:?}");
        assert!(maps(&engine, "defaulted", kind, Queued), "{kind:?}");
        assert!(!maps(&engine, "narrowed", kind, InReview), "{kind:?}");
        assert!(maps(&engine, "narrowed", kind, Todo), "{kind:?}");
        assert!(maps(&engine, "narrowed", kind, Done), "{kind:?}");
    }
}

#[test]
fn an_in_memory_source_maps_every_category() {
    let engine = engine_over(json!({"memory": {"plugin": "in-memory", "config": {}}}));
    for category in [Draft, Todo, Queued, InReview, Done, Cancelled] {
        assert!(maps(&engine, "memory", Task, category), "{category:?}");
    }
}

#[test]
fn a_github_board_maps_its_mapping_over_its_shipped_defaults_and_is_sent_nothing() {
    let (listener, endpoint) = unvisited();
    let board = |mapping: Value| {
        json!({"plugin": "github-projects", "config": {
            "owner": "someone", "project_number": 1, "endpoint": endpoint,
            "status_mapping": mapping,
        }})
    };
    let engine = engine_over(json!({
        "reviewing": board(json!({"in-review": {"task": "In Review"}, "queued": null})),
        "plain": board(json!({})),
    }));
    // Mapped for the one kind it names, and for no other.
    assert!(maps(&engine, "reviewing", Task, InReview));
    assert!(!maps(&engine, "reviewing", Project, InReview));
    // Set to `null`.
    assert!(!maps(&engine, "reviewing", Task, Queued));
    // Shipped defaults stand for every category the mapping does not mention, and none is
    // shipped for `in-review` or `draft`.
    assert!(maps(&engine, "reviewing", Task, Todo));
    assert!(maps(&engine, "plain", Task, Queued));
    assert!(maps(&engine, "plain", Project, Done));
    assert!(!maps(&engine, "plain", Task, InReview));
    assert!(!maps(&engine, "plain", Task, Draft));
    assert_never_visited(&listener, "the board");
}

#[test]
fn a_linear_source_maps_exactly_its_mapping_and_is_sent_nothing() {
    let (listener, endpoint) = unvisited();
    let engine = engine_over(json!({"team": {"plugin": "linear", "config": {
        "team": "ENG", "endpoint": endpoint,
        "status_mapping": {
            "in-review": {"task": "In Review"},
            "todo": "Todo",
            "cancelled": null,
        },
    }}}));
    assert!(maps(&engine, "team", Task, InReview));
    assert!(!maps(&engine, "team", Project, InReview));
    assert!(maps(&engine, "team", Task, Todo));
    assert!(maps(&engine, "team", Project, Todo));
    assert!(!maps(&engine, "team", Task, Cancelled));
    // Linear has no built-in names: a category the mapping leaves out has none.
    assert!(!maps(&engine, "team", Task, Done));
    assert_never_visited(&listener, "Linear");
}

#[cfg(unix)]
#[test]
fn a_stdio_plugin_maps_what_its_handshake_lists_and_is_sent_nothing_more() {
    let heard = tempfile::tempdir().unwrap();
    let log = heard.path().join("requests");
    let peer = |statuses: &str| {
        format!(
            r#"read -r _request
printf '%s\n' '{{"id":"0","result":{{"protocol_version":2,"kind":"peer","capabilities":{{"projects":"native","orphan_tasks":"native","filter_by_label":"native","filter_by_status":"native","search_title":"native","search_content":"native","task_dependencies":"both-directions","project_dependencies":"both-directions","max_page_size":50}},"writes":"supported","task_updates":true,"statuses":{statuses}}}}}'
while read -r line; do printf '%s\n' "$line" >> "$0"; done"#
        )
    };
    let earlier =
        peer(r#"["draft","backlog","todo","queued","in-progress","done","cancelled","unknown"]"#);
    let current = peer(
        r#"["draft","backlog","todo","queued","in-progress","in-review","done","cancelled","unknown"]"#,
    );
    let engine = engine_over(json!({
        "earlier": {"plugin": "subprocess", "config": {
            "command": "/bin/sh", "args": ["-c", earlier, log],
        }},
        "current": {"plugin": "subprocess", "config": {
            "command": "/bin/sh", "args": ["-c", current, log],
        }},
    }));
    assert!(!maps(&engine, "earlier", Task, InReview));
    assert!(maps(&engine, "earlier", Task, Queued));
    assert!(maps(&engine, "current", Task, InReview));
    assert!(maps(&engine, "current", Project, InReview));
    drop(engine);
    let sent = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        sent.is_empty(),
        "a peer was sent more than its handshake: {sent}"
    );
}

#[test]
fn a_source_nothing_configures_is_refused_by_name() {
    let engine = engine_over(json!({"memory": {"plugin": "in-memory", "config": {}}}));
    let error = engine
        .maps_status(&name("elsewhere"), Task, InReview)
        .expect_err("nothing configures it");
    assert!(
        matches!(&error, EngineError::UnknownSource { name, .. } if name == "elsewhere"),
        "{error:?}"
    );
}
