//! What a Linear source reports of its key's rate-limit windows: what its last response's
//! headers said, and nothing before its first.
//!
//! A fresh `sources quota` process has made no Linear call, so through the binary it answers no
//! resources (manager ruling `a4`). That a reading follows the last response is proven in one
//! process, through the engine the binary links: one request to the loopback, then
//! `Engine::quota`, then another request whose answer carries other figures.

use serde_json::{Value, json};

use crate::cache::dataset;
use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{LinearWorkspace, document, linear_workspace};

/// The `x-ratelimit-*` headers of one answer: `requests` and `complexity` windows with that
/// much of each left.
fn headers(requests: u64, complexity: u64) -> Vec<(String, String)> {
    [
        ("x-ratelimit-requests-limit", "1500".to_owned()),
        ("x-ratelimit-requests-remaining", requests.to_string()),
        ("x-ratelimit-requests-reset", "1795306927000".to_owned()),
        ("x-ratelimit-complexity-limit", "250000".to_owned()),
        ("x-ratelimit-complexity-remaining", complexity.to_string()),
        ("x-ratelimit-complexity-reset", "1795309716000".to_owned()),
    ]
    .into_iter()
    .map(|(name, value)| (name.to_owned(), value))
    .collect()
}

/// What those headers read as.
fn readings(requests: u64, complexity: u64) -> Value {
    json!([
        {"resource": "requests", "limit": 1500, "remaining": requests, "used": 1500 - requests,
         "resets_at": "2026-11-22T00:22:07Z"},
        {"resource": "complexity", "limit": 250000, "remaining": complexity,
         "used": 250000 - complexity, "resets_at": "2026-11-22T01:08:36Z"},
    ])
}

struct Setup {
    sandbox: Sandbox,
    workspace: LinearWorkspace,
}

impl Setup {
    fn new() -> Self {
        let sandbox = Sandbox::new();
        let (config, workspace) = linear_workspace(&sandbox, dataset());
        sandbox.project_document(&document(&json!({
            "linear": {"plugin": "linear", "config": config},
        })));
        Self { sandbox, workspace }
    }

    fn engine(&self) -> onetaskgraph_core::Engine {
        let environment = onetaskgraph_core::Environment::from_pairs([
            (
                "XDG_CONFIG_HOME",
                self.sandbox.config_home().to_string_lossy().into_owned(),
            ),
            ("LINEAR_API_KEY", "fixture-key".to_owned()),
        ]);
        let loaded = onetaskgraph_core::config::load(
            &self.sandbox.project(),
            &environment,
            &onetaskgraph_core::config::Layer::default(),
        )
        .expect("the configuration loads");
        onetaskgraph_core::Engine::build(&loaded.config, &loaded.secrets)
    }
}

#[test]
fn a_fresh_process_has_received_no_response_and_reports_no_reading() {
    let setup = Setup::new();
    let output = setup
        .sandbox
        .command()
        .args(["sources", "quota", "linear", "--json"])
        .assert()
        .get_output()
        .clone();
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let answer: Value = serde_json::from_str(&stdout(&output)).expect("JSON");
    assert_eq!(answer, json!({"source": "linear", "resources": []}));
    assert!(
        setup.workspace.served().is_empty(),
        "asking costs Linear no request"
    );
}

#[test]
fn the_engine_reports_exactly_what_the_last_response_carried() {
    let setup = Setup::new();
    let engine = setup.engine();
    let linear = onetaskgraph_plugin_api::SourceName::new("linear").expect("a name");
    let id = |native: &str| onetaskgraph_core::GlobalId {
        source: linear.clone(),
        native: onetaskgraph_plugin_api::NativeId(native.to_owned()),
    };
    let runtime = tokio::runtime::Runtime::new().expect("a runtime");
    // Before any request, nothing.
    assert!(
        runtime
            .block_on(engine.quota(&linear))
            .expect("a reading")
            .is_empty()
    );
    // One read, its answer carrying the windows' figures.
    setup
        .workspace
        .answer_with_headers(vec![headers(1490, 249_000)]);
    let before = setup.workspace.served().len();
    runtime
        .block_on(engine.task(&id("L-1")))
        .expect("the task is read");
    let sent = setup.workspace.served().len() - before;
    assert_eq!(sent, 1, "the read is one request");
    let read = runtime.block_on(engine.quota(&linear)).expect("a reading");
    assert_eq!(
        serde_json::to_value(&read).unwrap(),
        readings(1490, 249_000)
    );
    assert_eq!(
        setup.workspace.served().len() - before,
        sent,
        "the reading itself sends nothing"
    );
    // A later answer carrying other figures is what is reported next.
    setup
        .workspace
        .answer_with_headers(vec![headers(1470, 230_000)]);
    runtime
        .block_on(engine.task(&id("L-2")))
        .expect("the task is read");
    let read = runtime.block_on(engine.quota(&linear)).expect("a reading");
    assert_eq!(
        serde_json::to_value(&read).unwrap(),
        readings(1470, 230_000)
    );
}
