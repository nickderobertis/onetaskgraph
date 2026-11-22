//! `sources quota` against the loopback board, through the binary and through the engine it
//! links: one `GET /rate_limit` at the source's endpoint, every resource it returns reported as
//! GitHub gave it, and nothing answered from the store's cache.

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{GitHubBoardFields, document, github_projects_with_board};

/// An answer of `GET /rate_limit` whose figures are `used` of each allowance, named after the
/// instant each window resets.
fn rate_limits(used: u64) -> Value {
    json!({"resources": {
        "core": {"limit": 5000, "used": used, "remaining": 5000 - used, "reset": 1_795_306_927},
        "graphql": {"limit": 5000, "used": 2 * used, "remaining": 5000 - 2 * used,
                    "reset": 1_795_309_716},
        "search": {"limit": 30, "used": 1, "remaining": 29, "reset": 1_795_303_387},
        "code_search": {"limit": 10, "used": 0, "remaining": 10, "reset": 1_795_303_387},
    }, "rate": {"limit": 5000, "used": used, "remaining": 5000 - used, "reset": 1_795_306_927}})
}

/// What `sources quota` answers for `answer`: every resource, in GitHub's order.
fn expected(answer: &Value) -> Value {
    let resources: Vec<Value> = answer["resources"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(name, window)| {
            let resets_at = chrono::DateTime::from_timestamp(window["reset"].as_i64().unwrap(), 0)
                .unwrap()
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            json!({"resource": name, "limit": window["limit"], "remaining": window["remaining"],
                   "used": window["used"], "resets_at": resets_at})
        })
        .collect();
    json!({"source": "board", "resources": resources})
}

struct Setup {
    sandbox: Sandbox,
    board: GitHubBoardFields,
}

impl Setup {
    /// The shared loopback board, its source's cache on when `cached`.
    fn new(cached: bool) -> Self {
        let sandbox = Sandbox::new();
        let (config, board) = github_projects_with_board(&sandbox);
        let mut source = json!({"plugin":"github-projects","config":config});
        if cached {
            source["cache"] = json!({"enabled": true,
                "dir": sandbox.subdirectory("cache").to_string_lossy()});
        }
        sandbox.project_document(&document(&json!({ "board": source })));
        Self { sandbox, board }
    }

    /// `sources quota board --json`, which has to exit 0, with what the board was sent for it.
    fn quota(&self) -> (Value, Vec<String>) {
        let before = self.board.documents().len();
        let output = self
            .sandbox
            .command()
            .args(["sources", "quota", "board", "--json"])
            .assert()
            .get_output()
            .clone();
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        let answer = serde_json::from_str(&stdout(&output)).expect("the command emits JSON");
        (answer, self.board.documents()[before..].to_vec())
    }
}

#[test]
fn sources_quota_answers_every_resource_the_board_s_rate_limit_read_returns() {
    let setup = Setup::new(false);
    setup.board.serve_rate_limits(vec![rate_limits(120)]);
    let (answer, sent) = setup.quota();
    assert_eq!(answer, expected(&rate_limits(120)));
    assert_eq!(sent, ["GET /rate_limit"], "one free read and nothing else");

    // The human rendering is one row per resource.
    let output = setup
        .sandbox
        .command()
        .args(["sources", "quota", "board"])
        .assert()
        .success()
        .get_output()
        .clone();
    let text = stdout(&output);
    for resource in ["core", "graphql", "search", "code_search"] {
        assert!(
            text.lines()
                .any(|line| line.starts_with(&format!("{resource} "))),
            "{resource} has no row: {text}"
        );
    }
    assert!(
        text.contains("graphql  4760 of 5000 remaining, 240 used, resets 2026-11-22T01:08:36Z"),
        "{text}"
    );
}

#[test]
fn a_cached_source_asks_the_board_every_time_and_answers_each_reading_as_served() {
    let setup = Setup::new(true);
    setup
        .board
        .serve_rate_limits(vec![rate_limits(10), rate_limits(25)]);
    let (first, sent) = setup.quota();
    assert_eq!(first, expected(&rate_limits(10)));
    assert_eq!(sent, ["GET /rate_limit"]);
    let (second, sent) = setup.quota();
    assert_eq!(second, expected(&rate_limits(25)));
    assert_eq!(
        sent,
        ["GET /rate_limit"],
        "nothing is answered from the cache"
    );
    assert_ne!(first, second);
}

#[test]
fn the_engine_answers_the_same_readings_as_a_library_call() {
    let setup = Setup::new(false);
    setup.board.serve_rate_limits(vec![rate_limits(7)]);
    let environment = onetaskgraph_core::Environment::from_pairs([(
        "XDG_CONFIG_HOME",
        setup.sandbox.config_home().to_string_lossy().into_owned(),
    )]);
    let loaded = onetaskgraph_core::config::load(
        &setup.sandbox.project(),
        &environment,
        &onetaskgraph_core::config::Layer::default(),
    )
    .expect("the configuration loads");
    let engine = onetaskgraph_core::Engine::build(&loaded.config, &loaded.secrets);
    let board = onetaskgraph_plugin_api::SourceName::new("board").expect("a name");
    let readings = tokio::runtime::Runtime::new()
        .expect("a runtime")
        .block_on(engine.quota(&board))
        .expect("the board answers its quota");
    assert_eq!(
        serde_json::to_value(onetaskgraph_core::SourceQuota {
            source: board,
            resources: readings,
        })
        .unwrap(),
        expected(&rate_limits(7))
    );
    // And the command line answers exactly that.
    let (answer, _) = setup.quota();
    assert_eq!(answer, expected(&rate_limits(7)));
}

#[test]
fn an_unknown_source_is_refused_naming_it() {
    let setup = Setup::new(false);
    let output = setup
        .sandbox
        .command()
        .args(["sources", "quota", "elsewhere", "--json"])
        .assert()
        .get_output()
        .clone();
    assert_ne!(output.status.code(), Some(0));
    assert!(stderr(&output).contains("elsewhere"), "{}", stderr(&output));
    assert_eq!(setup.board.documents().len(), 0, "nothing was sent");
}

#[test]
fn a_rate_limit_read_github_refuses_fails_the_command_naming_why() {
    for (status, body, said) in [
        (
            401,
            r#"{"message":"Bad credentials"}"#,
            "rejected source board's credential",
        ),
        (
            403,
            r#"{"message":"Resource not accessible by personal access token"}"#,
            "credential reading GET /rate_limit with HTTP 403",
        ),
        (500, r#"{"message":"boom"}"#, "GitHub answered HTTP 500"),
        (
            200,
            "<html>unicorn</html>",
            "could not read its token's rate limits from GET /rate_limit",
        ),
        (
            200,
            r#"{"rate":{"limit":5000}}"#,
            "does not read as rate limits",
        ),
        (
            200,
            r#"{"resources":{"core":{"limit":5000,"used":1,"reset":1}}}"#,
            "no whole-number remaining",
        ),
        (
            200,
            r#"{"resources":{"core":{"limit":5000,"used":1,"remaining":4999,
                "reset":18446744073709551615}}}"#,
            "which is no instant",
        ),
    ] {
        let setup = Setup::new(false);
        setup.board.refuse_rate_limit_read(status, body);
        let output = setup
            .sandbox
            .command()
            .args(["sources", "quota", "board", "--json"])
            .assert()
            .get_output()
            .clone();
        assert_ne!(output.status.code(), Some(0), "{status} {body}");
        assert!(
            stderr(&output).contains(said),
            "{status}: {}",
            stderr(&output)
        );
        assert_eq!(setup.board.documents(), ["GET /rate_limit"]);
    }
}

#[test]
fn a_rate_limit_read_that_never_reaches_github_fails_naming_the_transport() {
    let sandbox = Sandbox::new();
    let (mut config, _board) = github_projects_with_board(&sandbox);
    // A port nothing listens on: the read cannot be sent at all.
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = closed.local_addr().unwrap();
    drop(closed);
    config["endpoint"] = json!(format!("http://{address}/graphql"));
    sandbox.project_document(&document(&json!({
        "board": {"plugin":"github-projects","config":config},
    })));
    let output = sandbox
        .command()
        .args(["sources", "quota", "board", "--json"])
        .assert()
        .get_output()
        .clone();
    assert_ne!(output.status.code(), Some(0));
    assert!(
        stderr(&output).contains("could not read its token's rate limits from GET /rate_limit"),
        "{}",
        stderr(&output)
    );
}
