//! Spike `spike-store-defects`: the open-item list measurement harness
//! (`spike_store_defects/measure_open_list.py`) driven offline, through the real binary and its
//! own recording proxy, against the loopback board. No request leaves the machine: the source's
//! configured `endpoint` is the loopback, and the harness forwards only there.
//!
//! What it proves is the part of the harness a live run cannot cheaply re-check: that it keeps
//! the binary's standard output and parses it into the same rows the binary lists when run
//! directly, and that an incomplete measurement exits 1 with that output still kept.

use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{GitHubBoardFields, document, github_projects_with_items};

/// One task issue on the board, filed under no project, at `status`.
fn issue(id: &str, status: &str) -> Value {
    json!({"item":format!("ITEM-{id}"),"id":id,"type":"Issue","title":format!("Ticket {id}"),
           "body":format!("Ticket {id}, as filed"),"state":"OPEN","reason":null,"parent":null,
           "repo":"nickderobertis/onetaskgraph","status":status,"origin":"","labels":[]})
}

fn harness() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("spike_store_defects/measure_open_list.py")
}

struct Setup {
    sandbox: Sandbox,
    board: GitHubBoardFields,
    out: PathBuf,
}

impl Setup {
    fn new() -> Self {
        let sandbox = Sandbox::new();
        let (config, board) = github_projects_with_items(
            &sandbox,
            vec![
                issue("H-1", "Todo"),
                issue("H-2", "Doing"),
                issue("H-3", "Todo"),
                issue("H-4", "Done"),
            ],
        );
        board.close("H-3", Some("COMPLETED"));
        sandbox.project_document(&document(&json!({
            "board": {"plugin":"github-projects","config":config}
        })));
        let out = sandbox.subdirectory("measurements").join("today.json");
        Self {
            sandbox,
            board,
            out,
        }
    }

    /// The harness, run as a person runs it, handed the loopback's token as its credential.
    fn measure(&self, extra: &[&str]) -> Output {
        let mut command = self.sandbox.subprocess("python3");
        command
            .arg(harness())
            .args(["--config-dir"])
            .arg(self.sandbox.project())
            .args(["--source", "board", "--out"])
            .arg(&self.out)
            .arg("--binary")
            .arg(onetaskgraph_e2e_support::binary())
            .args(extra)
            .env("GH_PROJECTS_TOKEN", "test-token");
        command.output().expect("python3 runs the harness")
    }

    fn written(&self, path: &Path) -> Value {
        let text = std::fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("{} was written: {error}", path.display()));
        serde_json::from_str(&text).expect("the harness writes JSON")
    }

    /// The status categories `onetaskgraph schema` names that no closure reads as: every
    /// `StatusCategory` but those its `ClosedAs` lists.
    fn open_categories(&self) -> Vec<String> {
        let output = self
            .sandbox
            .command()
            .arg("schema")
            .assert()
            .get_output()
            .clone();
        let schema: Value = serde_json::from_str(&stdout(&output)).expect("the schema bundle");
        let consts = |root: &str| -> Vec<String> {
            schema["roots"][root]["oneOf"]
                .as_array()
                .unwrap_or_else(|| panic!("{root} is an enum"))
                .iter()
                .map(|variant| variant["const"].as_str().expect("a const").to_owned())
                .collect()
        };
        let closing = consts("ClosedAs");
        consts("StatusCategory")
            .into_iter()
            .filter(|category| !closing.contains(category))
            .collect()
    }

    /// The ids the binary lists itself for the same question, run directly.
    fn listed_directly(&self, statuses: &[&str]) -> Vec<String> {
        let mut arguments = vec!["--json", "task", "list", "--all", "--source", "board"];
        for status in statuses {
            arguments.extend(["--status", status]);
        }
        let output = self
            .sandbox
            .command()
            .args(&arguments)
            .assert()
            .get_output()
            .clone();
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        let listing: Value = serde_json::from_str(&stdout(&output)).expect("JSON");
        let mut ids: Vec<String> = listing["items"]
            .as_array()
            .expect("rows")
            .iter()
            .map(|row| row["id"].as_str().expect("an id").to_owned())
            .collect();
        ids.sort();
        ids
    }
}

#[test]
fn a_today_measurement_keeps_the_binarys_stdout_and_parses_the_rows_it_lists() {
    let setup = Setup::new();
    let before = setup.board.served_count();
    let output = setup.measure(&["--mode", "today"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let measured = setup.written(&setup.out);
    let sent = setup.board.served_since(before);

    // The kept stdout is exactly the listing, and it parses back to what the harness counted.
    let kept = PathBuf::from(measured["stdout_path"].as_str().expect("a kept stdout"));
    assert_eq!(kept, setup.out.with_extension("stdout.json"));
    let listing = setup.written(&kept);
    let rows = listing["items"].as_array().expect("rows");
    assert_eq!(measured["listed"], json!(rows.len()), "{measured:#}");
    assert_eq!(measured["parse_error"], Value::Null);
    assert!(measured["stdout_bytes"].as_u64().unwrap_or(0) > 0);

    // Exactly the categories the binary's schema does not close, in its own order.
    let open = setup.open_categories();
    let asked: Vec<&str> = measured["command"]
        .as_str()
        .expect("a command")
        .split(' ')
        .collect::<Vec<_>>()
        .windows(2)
        .filter(|pair| pair[0] == "--status")
        .map(|pair| pair[1])
        .collect();
    assert_eq!(asked, open, "{measured:#}");
    let mut ids: Vec<String> = measured["listed_ids"]
        .as_array()
        .expect("ids")
        .iter()
        .map(|id| id.as_str().expect("an id").to_owned())
        .collect();
    ids.sort();
    let open: Vec<&str> = open.iter().map(String::as_str).collect();
    assert_eq!(ids, setup.listed_directly(&open));
    assert!(ids.contains(&"board:H-1".to_owned()) && ids.contains(&"board:H-2".to_owned()));
    assert!(
        !ids.contains(&"board:H-3".to_owned()),
        "a closed issue is not open: {ids:?}"
    );
    assert_eq!(
        measured["listed_by_status"]["todo:Todo"],
        json!(
            ids.iter()
                .filter(|id| {
                    rows.iter().any(|row| {
                        row["id"] == json!(id) && row["item"]["status"]["name"] == "Todo"
                    })
                })
                .count()
        )
    );

    // Every request the harness recorded reached the loopback, and none of them wrote.
    let recorded = measured["summary"]["requests"].as_u64().expect("a count");
    assert_eq!(recorded as usize, sent.len(), "{measured:#}");
    assert!(
        sent.iter()
            .all(|(query, _)| !query.trim_start().starts_with("mutation")),
        "{sent:?}"
    );
    assert!(measured["summary"]["phases"]["board-items"]["requests"].as_u64() >= Some(1));
}

#[test]
fn an_incomplete_measurement_exits_1_and_still_keeps_the_binarys_stdout() {
    let setup = Setup::new();
    // A category the binary refuses: it exits 2 having listed nothing.
    let output = setup.measure(&["--mode", "today", "--status", "not-a-category"]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("incomplete"),
        "{}",
        stderr(&output)
    );
    let measured = setup.written(&setup.out);
    assert_eq!(measured["exit"], json!(2), "{measured:#}");
    assert_eq!(measured["listed"], Value::Null);
    assert!(measured["parse_error"].is_string(), "{measured:#}");
    assert!(setup.out.with_extension("stdout.json").is_file());
}

#[test]
fn a_source_that_is_no_board_is_refused_as_an_invocation_error() {
    let setup = Setup::new();
    setup.sandbox.project_document(&document(&json!({
        "board": {"plugin":"local-md","config":{"root":setup.sandbox.subdirectory("tasks")}}
    })));
    let output = setup.measure(&["--mode", "today"]);
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("is not a github-projects source"),
        "{}",
        stderr(&output)
    );
    assert!(!setup.out.exists());
}

#[test]
fn a_capture_then_a_prototype_reads_the_open_items_with_the_binarys_own_document() {
    let setup = Setup::new();
    let captured = setup.out.with_file_name("capture.json");
    let before = setup.board.served_count();
    let mut command = setup.sandbox.subprocess("python3");
    let output = command
        .arg(harness())
        .arg("--config-dir")
        .arg(setup.sandbox.project())
        .args(["--source", "board", "--mode", "capture", "--out"])
        .arg(&captured)
        .arg("--binary")
        .arg(onetaskgraph_e2e_support::binary())
        .env("GH_PROJECTS_TOKEN", "test-token")
        .output()
        .expect("python3 runs the harness");
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(
        setup.board.served_count(),
        before,
        "a capture forwards nothing"
    );
    let capture = setup.written(&captured);
    assert!(
        capture["captured"]["query"]
            .as_str()
            .expect("the board document")
            .contains("fragment Board on ProjectV2")
    );

    let output = setup.measure(&[
        "--mode",
        "prototype",
        "--captured",
        captured.to_str().expect("a UTF-8 path"),
    ]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let measured = setup.written(&setup.out);
    let sent = setup.board.served_since(before);
    assert!(
        sent.iter()
            .all(|(query, _)| query.contains("query:$itemQuery")),
        "{sent:?}"
    );
    let open = measured["counts"]["issue_open"].as_u64().expect("a count");
    assert!(open >= 3, "H-1, H-2 and H-4 are open: {measured:#}");
    assert_eq!(measured["summary"]["requests"], json!(sent.len()));
}

#[test]
fn an_env_file_that_cannot_be_read_is_an_invocation_error() {
    let setup = Setup::new();
    let missing = setup.sandbox.project().join("no-such.env");
    let mut command = setup.sandbox.subprocess("python3");
    let output = command
        .arg(harness())
        .arg("--config-dir")
        .arg(setup.sandbox.project())
        .args(["--source", "board", "--mode", "today", "--out"])
        .arg(&setup.out)
        .arg("--binary")
        .arg(onetaskgraph_e2e_support::binary())
        .arg("--env-file")
        .arg(&missing)
        .output()
        .expect("python3 runs the harness");
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("cannot read --env-file"),
        "{}",
        stderr(&output)
    );
    assert!(!setup.out.exists());
}
