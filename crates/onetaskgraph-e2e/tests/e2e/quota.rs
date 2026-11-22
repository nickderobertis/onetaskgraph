//! `sources quota` on sources that are not a hosted board: a folder of Markdown, which has
//! nothing to report, and a plugin served over the stdio protocol, which reports what its
//! handshake declares it can — and, declaring nothing, is never sent the method. A GitHub
//! board's reading is the GitHub Projects suite's (`quota.rs` in
//! `onetaskgraph-github-projects-e2e`), and a Linear source's the Linear suite's.

use std::path::Path;

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::document_store::store_at;
use crate::fixtures::document;

/// `sources quota <source> --json`, exiting 0: its JSON answer.
fn quota(sandbox: &Sandbox, source: &str) -> Value {
    let output = sandbox
        .command()
        .args(["sources", "quota", source, "--json"])
        .assert()
        .get_output()
        .clone();
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}{}",
        stdout(&output),
        stderr(&output)
    );
    serde_json::from_str(&stdout(&output)).expect("a quota reading")
}

/// Every method the peer whose log is `log` was asked for, in order.
fn asked(log: &Path) -> Vec<String> {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn a_folder_of_markdown_reports_no_reading_through_the_binary_and_the_engine() {
    let sandbox = Sandbox::new();
    let plan = sandbox.subdirectory("plan");
    std::fs::create_dir_all(plan.join("tasks")).expect("a folder");
    sandbox.project_document(&document(&json!({
        "plan": {"plugin": "local-md", "config": {"root": plan}},
    })));
    assert_eq!(
        quota(&sandbox, "plan"),
        json!({"source": "plan", "resources": []})
    );
    let output = sandbox
        .command()
        .args(["sources", "quota", "plan"])
        .assert()
        .success()
        .get_output()
        .clone();
    assert_eq!(stdout(&output), "plan: no quota reading to report\n");

    // The engine the binary links answers the same, as a library call.
    let environment = onetaskgraph_core::Environment::from_pairs([(
        "XDG_CONFIG_HOME",
        sandbox.config_home().to_string_lossy().into_owned(),
    )]);
    let loaded = onetaskgraph_core::config::load(
        &sandbox.project(),
        &environment,
        &onetaskgraph_core::config::Layer::default(),
    )
    .expect("the configuration loads");
    let engine = onetaskgraph_core::Engine::build(&loaded.config, &loaded.secrets);
    let name = onetaskgraph_plugin_api::SourceName::new("plan").expect("a name");
    let readings = tokio::runtime::Runtime::new()
        .expect("a runtime")
        .block_on(engine.quota(&name))
        .expect("a folder answers");
    assert!(readings.is_empty());

    // A source nothing configures is refused naming it, before anything runs.
    let output = sandbox
        .command()
        .args(["sources", "quota", "elsewhere"])
        .assert()
        .failure()
        .get_output()
        .clone();
    assert!(stderr(&output).contains("elsewhere"), "{}", stderr(&output));
}

#[test]
fn a_stdio_peer_whose_handshake_lacks_quota_reports_none_and_is_never_sent_it() {
    let sandbox = Sandbox::new();
    let store = sandbox.subdirectory("store").join("documents.json");
    std::fs::write(&store, r#"{"documents": []}"#).expect("a store");
    let log = sandbox.subdirectory("store").join("asked.log");
    sandbox.project_document(&document(&json!({
        "peer": store_at(&store, Some(&log), "native"),
    })));
    assert_eq!(
        quota(&sandbox, "peer"),
        json!({"source": "peer", "resources": []})
    );
    assert_eq!(
        asked(&log),
        ["initialize"],
        "the peer was sent nothing more"
    );
}

#[test]
fn a_stdio_peer_declaring_quota_is_sent_it_once_and_its_readings_are_answered() {
    let sandbox = Sandbox::new();
    let store = sandbox.subdirectory("store").join("documents.json");
    std::fs::write(&store, r#"{"documents": []}"#).expect("a store");
    let log = sandbox.subdirectory("store").join("asked.log");
    let readings = json!([
        {"resource": "requests", "limit": 1500, "remaining": 1480, "used": 20,
         "resets_at": "2026-11-22T00:22:07Z"},
        {"resource": "complexity", "limit": 250000, "remaining": 200000, "used": 50000,
         "resets_at": "2026-11-22T01:08:36Z"},
    ]);
    let mut peer = store_at(&store, Some(&log), "native");
    peer["config"]["settings"]["quota"] = readings.clone();
    sandbox.project_document(&document(&json!({ "peer": peer })));
    assert_eq!(
        quota(&sandbox, "peer"),
        json!({"source": "peer", "resources": readings})
    );
    assert_eq!(asked(&log), ["initialize", "quota"]);
}

#[test]
fn the_shipped_host_forwards_quota_to_the_plugin_it_serves() {
    let sandbox = Sandbox::new();
    let root = sandbox.subdirectory("plan");
    std::fs::create_dir_all(root.join("tasks")).expect("a folder");
    let log = sandbox.subdirectory("peer").join("sent.log");
    sandbox.project_document(&document(&json!({
        "peer": crate::request_gate::hosted(&root, &log),
    })));
    // `plugin-serve local-md` declares `quota` and answers what the folder reports: nothing.
    assert_eq!(
        quota(&sandbox, "peer"),
        json!({"source": "peer", "resources": []})
    );
    assert_eq!(asked(&log), ["initialize", "quota"]);
}

#[test]
fn a_peer_answering_a_reading_it_cannot_have_read_is_refused_as_malformed() {
    let sandbox = Sandbox::new();
    let store = sandbox.subdirectory("store").join("documents.json");
    std::fs::write(&store, r#"{"documents": []}"#).expect("a store");
    let mut peer = store_at(&store, None, "native");
    // A reading naming no limit, remainder, use or reset.
    peer["config"]["settings"]["quota"] = json!([{"resource": "requests"}]);
    sandbox.project_document(&document(&json!({ "peer": peer })));
    let output = sandbox
        .command()
        .args(["sources", "quota", "peer", "--json"])
        .assert()
        .get_output()
        .clone();
    assert_ne!(output.status.code(), Some(0), "{}", stdout(&output));
    // The failure document, and no reading.
    let answer: Value = serde_json::from_str(&stdout(&output)).expect("a failure document");
    assert_eq!(answer["failure"]["kind"], "malformed", "{answer:#}");
    assert!(answer.get("resources").is_none(), "{answer:#}");
}
