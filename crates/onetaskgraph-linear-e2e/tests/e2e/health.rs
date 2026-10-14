//! A source's `health`, read through the one public entry point that reaches it: the stdio
//! plugin host, `onetaskgraph-source`, answering a `health` request (`docs/plugin-protocol.md`
//! §4.3).
//!
//! Nothing in the engine, the command line or either SDK asks a source for its health. The host
//! builds the plugin it serves bare — its `initialize` carries no cache setting, and the engine
//! keeps no cache of a hosted source — so the store's cache never stands in front of this read.
//! This drives the real host serving the Linear plugin against the loopback Linear, with the
//! user's cache settings for that source turned on and then off, and holds every health read to
//! reaching Linear, at the same count both ways, with nothing written where a cache would be.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

use onetaskgraph_linear::graphql;
use serde_json::{Value, json};

use crate::cache::{dataset, walk};
use crate::common::Sandbox;
use crate::fixtures::linear_workspace;

/// What two `health` requests to one hosted Linear plugin answered, and every request the
/// workspace was sent by the host — with the cache settings for that source on or off.
fn health_twice(cache: bool) -> (Vec<Value>, Vec<String>, Vec<std::path::PathBuf>) {
    let sandbox = Sandbox::new();
    let (config, workspace) = linear_workspace(&sandbox, dataset());
    let cache_home = sandbox.subdirectory("cache-home");
    let mut host = Command::new(onetaskgraph_e2e_support::source_binary());
    host.env("XDG_CACHE_HOME", &cache_home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if cache {
        host.env("ONETASKGRAPH_SOURCES__LINEAR__CACHE__ENABLED", "true")
            .env(
                "ONETASKGRAPH_SOURCES__LINEAR__CACHE__DIR",
                cache_home.join("explicit"),
            );
    }
    let mut host = host.spawn().expect("the plugin host starts");
    let mut stdin = host.stdin.take().expect("its standard input");
    let requests = [
        json!({"id": "0", "method": "initialize", "params": {
            "protocol_version": 2,
            "engine": {"name": "onetaskgraph", "version": env!("CARGO_PKG_VERSION")},
            "source_name": "linear",
            "config": {"kind": "linear", "config": config},
            "secrets": {"LINEAR_API_KEY": "fixture-key"},
        }}),
        json!({"id": "1", "method": "health", "params": {}}),
        json!({"id": "2", "method": "health", "params": {}}),
    ];
    for request in requests {
        writeln!(stdin, "{request}").expect("a request is written");
    }
    drop(stdin);
    let answers: Vec<Value> = BufReader::new(host.stdout.take().expect("its standard output"))
        .lines()
        .map(|line| serde_json::from_str(&line.expect("a line")).expect("a line of JSON"))
        .collect();
    host.wait().expect("the host exits");
    let sent = workspace
        .served()
        .into_iter()
        .map(|(document, _)| document)
        .collect();
    (answers, sent, walk(&cache_home))
}

#[test]
fn every_health_read_through_the_plugin_host_reaches_linear_with_the_cache_on_or_off() {
    let [on, off] = [true, false].map(health_twice);
    for (mode, (answers, sent, cached)) in [("on", &on), ("off", &off)] {
        let handshake = answers
            .iter()
            .find(|answer| answer["id"] == "0")
            .unwrap_or_else(|| panic!("cache {mode}: no handshake answer: {answers:?}"));
        assert!(
            handshake.get("error").is_none(),
            "cache {mode}: {handshake}"
        );
        for id in ["1", "2"] {
            let answer = answers
                .iter()
                .find(|answer| answer["id"] == id)
                .unwrap_or_else(|| panic!("cache {mode}: no answer to health {id}: {answers:?}"));
            assert_eq!(
                answer["result"]["reachable"], true,
                "cache {mode}: health {id} answered {answer}"
            );
        }
        assert_eq!(
            sent.iter()
                .filter(|document| *document == graphql::VIEWER)
                .count(),
            2,
            "cache {mode}: each health read is its own request to Linear: {sent:?}"
        );
        assert!(
            cached.is_empty(),
            "cache {mode}: nothing is kept for a health read: {cached:?}"
        );
    }
    assert_eq!(
        on.1, off.1,
        "the host sends Linear the same requests whatever the cache settings say"
    );
}
