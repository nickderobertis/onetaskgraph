//! A Linear source with its local cache on: Linear gives its tasks no change token, so the
//! cache is written and never served, and every read says why it went to Linear.

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{document, linear_workspace};
use crate::linear::{eng_mapping, held, issue, linear};

#[test]
fn a_linear_source_with_its_cache_on_reads_linear_every_time_and_says_why() {
    let sandbox = Sandbox::new();
    let (config, workspace) = linear_workspace(
        &sandbox,
        held(
            vec![issue("L-1", "Todo", json!({}))],
            Vec::new(),
            Vec::new(),
        ),
    );
    let mut source = linear(&config, json!({"status_mapping": eng_mapping()}));
    source["cache"] = json!({"enabled": true});
    sandbox.project_document(&document(&json!({ "linear": source })));
    let cache_home = sandbox.subdirectory("cache-home");
    let run = |arguments: &[&str]| -> (Value, usize) {
        let before = workspace.served().len();
        let output = sandbox
            .command()
            .env("XDG_CACHE_HOME", &cache_home)
            .args(arguments)
            .assert()
            .get_output()
            .clone();
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        (
            serde_json::from_str(&stdout(&output)).expect("JSON"),
            workspace.served().len() - before,
        )
    };
    for _ in 0..2 {
        let (shown, sent) = run(&["task", "show", "linear:L-1", "--pin", "anything", "--json"]);
        assert!(sent > 0, "a Linear read always reaches Linear");
        assert_eq!(shown["cache"]["served"], "remote", "{shown}");
        let reason = shown["cache"]["bypassed"].as_str().expect("a reason");
        assert!(reason.contains("linear"), "{reason}");
        assert!(shown["items"][0]["item"].get("version").is_none());
    }
    let (listed, sent) = run(&["task", "comment", "list", "linear:L-1", "--json"]);
    assert!(sent > 0);
    assert_eq!(listed["cache"]["served"], "remote");
    assert!(listed["cache"]["bypassed"].is_string());
    // The entry was still written, under the default cache home.
    let root = cache_home.join("onetaskgraph");
    let written: usize = std::fs::read_dir(&root)
        .expect("a cache directory")
        .filter_map(Result::ok)
        .map(|dir| std::fs::read_dir(dir.path().join("tasks")).map_or(0, Iterator::count))
        .sum();
    assert!(written >= 1, "the cache is written, and never served");
}
