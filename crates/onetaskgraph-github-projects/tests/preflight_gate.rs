//! The live lane's board pre-flight against a loopback board holding a CI run's leftover field.
//!
//! The pre-flight is the real one — `journey::nominated_project_id`, the call the credentialed
//! lane makes before it writes anything — recording into a real session record, which is what
//! makes it judge the board against the registered workload. What stands in for GitHub is a
//! local HTTP server holding a board of the registered size, one stamped field more, and the
//! Actions run that stamp names.
//!
//! On 2026-10-25 a cancelled run left `otg-live-ci-40170293588-1-<micros>` on the shared
//! board, and every later journey refused it as "15 fields" until a person deleted it. So:
//! a field whose run reads `completed` — a cancelled run does — is removed and the board is
//! admitted; a field whose run is still going, or cannot be read, is left exactly where it is
//! and counted, so that board is refused as before. No credential and no third-party API.

use std::collections::BTreeMap;
use std::io::{Read, Write as _};
use std::net::TcpListener;
use std::process::Command;
use std::sync::{Arc, LazyLock, Mutex};
use std::thread;

use onetaskgraph_github_live::quota::{EntryPoint, reference_workload};
use onetaskgraph_github_live::{BOARD_WORKLOAD_DOCUMENT, CORE_REPOSITORY, DELETE_FIELD_DOCUMENT};
use onetaskgraph_github_projects::accounting::Accounting;
use serde_json::{Value, json};

// The credentialed lane's own halves: `journey` holds the pre-flight under test. Most of
// each belongs to the target that drives it against GitHub, for the reason
// `tests/sweep_gate.rs` gives.
#[allow(dead_code)]
mod journey;
#[allow(dead_code)]
mod lane;

const TOKEN: &str = "test-token";
/// The run the leftover field names: the cancelled run of 2026-10-25.
const LEFTOVER_RUN: u64 = 40_170_293_588;
const LEFTOVER: &str = "otg-live-ci-40170293588-1-1792911749637960";
/// What tells a re-execution of this binary which board to stand in for.
const CHILD_VARIABLE: &str = "ONETASKGRAPH_PREFLIGHT_GATE_CHILD";
/// The shared measurement as the lane's accounting sends it.
static PRICED_WORKLOAD: LazyLock<String> = LazyLock::new(|| {
    Accounting::with_query_prices()
        .priced_document(BOARD_WORKLOAD_DOCUMENT)
        .to_string()
});

/// The board, the run statuses GitHub reports and every request the pre-flight sent.
struct Board {
    items: u64,
    fields: Vec<(String, String)>,
    runs: BTreeMap<u64, Option<&'static str>>,
    requests: Vec<String>,
}

/// What a scenario puts on the board, and what GitHub says of the leftover field's run.
fn board(scenario: &str) -> Board {
    let mut fields = (0..14)
        .map(|n| (format!("FIELD_{n}"), format!("board field {n}")))
        .collect::<Vec<_>>();
    fields.push(("FIELD_leftover".to_owned(), LEFTOVER.to_owned()));
    let status = match scenario {
        "completed" => Some("completed"),
        "in-progress" => Some("in_progress"),
        "unreadable" => None,
        other => panic!("no scenario {other}"),
    };
    Board {
        items: 60,
        fields,
        runs: BTreeMap::from([(LEFTOVER_RUN, status)]),
        requests: Vec::new(),
    }
}

fn answer(board: &Mutex<Board>, method: &str, path: &str, body: Option<Value>) -> (u16, Value) {
    let mut board = board.lock().unwrap();
    if method == "POST" && path == "/graphql" {
        let body = body.expect("a GraphQL request");
        let query = body["query"].as_str().unwrap().to_owned();
        board.requests.push(query.clone());
        // The lane prices every query it sends, so what arrives is the measurement priced.
        if query == *PRICED_WORKLOAD {
            assert_eq!(body["variables"], json!({"owner":"octo-org","number":7}));
            let nodes = board
                .fields
                .iter()
                .map(|(id, name)| json!({"id":id,"name":name}))
                .collect::<Vec<_>>();
            return (
                200,
                json!({"data":{"repositoryOwner":{"projectV2":{"id":"PVT_board",
                    "items":{"totalCount":board.items},
                    "fields":{"totalCount":nodes.len(),"nodes":nodes,
                              "pageInfo":{"hasNextPage":false}}}}}}),
            );
        }
        assert_eq!(query, DELETE_FIELD_DOCUMENT, "an unexpected document");
        let field = body["variables"]["input"]["fieldId"]
            .as_str()
            .unwrap()
            .to_owned();
        board.fields.retain(|(id, _)| *id != field);
        return (
            200,
            json!({"data":{"deleteProjectV2Field":{"projectV2Field":{"id":field}}}}),
        );
    }
    board.requests.push(format!("{method} {path}"));
    // GitHub's single-run read, `GET /repos/{owner}/{repo}/actions/runs/{run_id}`, which the
    // janitor's `tests/fixtures/rest-operations.json` pins; `status` is the one field read.
    let run = path
        .strip_prefix(&format!("/repos/{CORE_REPOSITORY}/actions/runs/"))
        .and_then(|run| run.parse::<u64>().ok())
        .filter(|_| method == "GET")
        .unwrap_or_else(|| panic!("an unexpected request: {method} {path}"));
    match board.runs.get(&run).copied().flatten() {
        Some(status) => (200, json!({"id":run,"status":status})),
        None => (500, json!({"message":"Server Error"})),
    }
}

fn serve(board: Arc<Mutex<Board>>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a stand-in listener");
    let host = format!("http://{}", listener.local_addr().unwrap());
    thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.expect("a stand-in connection");
            let (method, path, body) = read_request(&mut stream);
            let (status, payload) = answer(&board, &method, &path, body);
            let payload = payload.to_string();
            let resource = if path == "/graphql" {
                "graphql"
            } else {
                "core"
            };
            let response = format!(
                "HTTP/1.1 {status} Stand-in\r\nContent-Type: application/json\r\n\
                 x-ratelimit-limit: 5000\r\nx-ratelimit-used: 1\r\n\
                 x-ratelimit-remaining: 4999\r\nx-ratelimit-resource: {resource}\r\n\
                 x-ratelimit-reset: 1775000000\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                payload.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    host
}

fn read_request(stream: &mut impl Read) -> (String, String, Option<Value>) {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 4096];
    let end = loop {
        let count = stream.read(&mut chunk).expect("a stand-in request");
        assert!(count > 0, "the request ended before its headers");
        bytes.extend_from_slice(&chunk[..count]);
        if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break end + 4;
        }
    };
    let headers = String::from_utf8_lossy(&bytes[..end]).into_owned();
    assert!(
        headers
            .to_ascii_lowercase()
            .contains(&format!("authorization: bearer {TOKEN}")),
        "{headers}"
    );
    let length = headers
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length: ")?
                .trim()
                .parse::<usize>()
                .ok()
        })
        .unwrap_or(0);
    while bytes.len() < end + length {
        let count = stream.read(&mut chunk).expect("a stand-in body");
        assert!(count > 0, "the request ended before its body");
        bytes.extend_from_slice(&chunk[..count]);
    }
    let mut line = headers.lines().next().unwrap_or_default().split(' ');
    let method = line.next().unwrap_or_default().to_owned();
    let path = line.next().unwrap_or_default().to_owned();
    let body = (length > 0).then(|| serde_json::from_slice(&bytes[end..end + length]).unwrap());
    (method, path, body)
}

/// Re-executes this binary as the pre-flight of one recorded session against `scenario`.
fn preflight(scenario: &str) -> (bool, String, String) {
    let directory = tempfile::tempdir().unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["preflight_child", "--exact", "--nocapture"])
        .env(CHILD_VARIABLE, scenario)
        .env("GITHUB_LIVE_SESSION_DIR", directory.path())
        .env("GITHUB_LIVE_SESSION_ID", "preflight-gate")
        .env("GITHUB_LIVE_SOURCE_REVISION", "a".repeat(40))
        .output()
        .unwrap();
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

// llmlint: ignore[tests_assert_real_behavior] This subprocess helper is driven by the three tests below, which assert on what it printed and how it exited; an ordinary test run omits its selector and starts no stand-in.
#[tokio::test]
async fn preflight_child() {
    let Ok(scenario) = std::env::var(CHILD_VARIABLE) else {
        return;
    };
    let board = Arc::new(Mutex::new(board(&scenario)));
    let host = serve(Arc::clone(&board));
    journey::against(journey::Endpoints {
        graphql: format!("{host}/graphql"),
        rest_host: host,
        source: None,
        settle: std::time::Duration::ZERO,
        fixture_wait: journey::STAND_IN_FIXTURE_WAIT,
    });
    let _recording = journey::quota::start(
        EntryPoint::Main,
        Arc::clone(&journey::SESSION),
        reference_workload(),
    )
    .unwrap();
    let measured = journey::nominated_project_id(TOKEN, "octo-org", 7).await;
    let board = board.lock().unwrap();
    println!("measured: {measured:?}");
    println!(
        "requests: {}",
        serde_json::to_string(&board.requests).unwrap()
    );
    println!(
        "fields left: {}",
        serde_json::to_string(&board.fields.iter().map(|(_, n)| n).collect::<Vec<_>>()).unwrap()
    );
}

fn fields_left(stdout: &str) -> Vec<String> {
    let line = stdout
        .lines()
        .find_map(|line| line.strip_prefix("fields left: "))
        .unwrap_or_else(|| panic!("the child printed no fields: {stdout}"));
    serde_json::from_str(line).unwrap()
}

fn requests(stdout: &str) -> Vec<String> {
    let line = stdout
        .lines()
        .find_map(|line| line.strip_prefix("requests: "))
        .unwrap_or_else(|| panic!("the child printed no requests: {stdout}"));
    serde_json::from_str(line).unwrap()
}

#[test]
fn a_field_a_completed_run_left_is_removed_and_the_board_is_admitted() {
    let (succeeded, stdout, stderr) = preflight("completed");
    assert!(succeeded, "{stdout}\n{stderr}");
    assert!(
        stdout.contains(r#"measured: Ok("PVT_board")"#),
        "{stdout}\n{stderr}"
    );
    assert!(
        stderr.contains(&format!(
            "board pre-flight removed field {LEFTOVER}: residue of run {LEFTOVER_RUN}, which \
             reads completed"
        )),
        "{stderr}"
    );
    assert!(!fields_left(&stdout).contains(&LEFTOVER.to_owned()));
    assert_eq!(fields_left(&stdout).len(), 14);
    assert_eq!(
        requests(&stdout),
        [
            PRICED_WORKLOAD.clone(),
            format!("GET /repos/{CORE_REPOSITORY}/actions/runs/{LEFTOVER_RUN}"),
            DELETE_FIELD_DOCUMENT.to_owned(),
        ]
    );
}

#[test]
fn a_field_an_active_or_unreadable_run_holds_is_kept_counted_and_refused() {
    for (scenario, why) in [
        ("in-progress", "reads in_progress".to_owned()),
        ("unreadable", "could not be read".to_owned()),
    ] {
        let (succeeded, stdout, stderr) = preflight(scenario);
        assert!(succeeded, "{scenario}: {stdout}\n{stderr}");
        assert!(
            stdout.contains(
                "measured: Err(\"initial board workload is 60 items and 15 fields; registered \
                 workload requires 51 to 90 items and 14 fields\")"
            ),
            "{scenario}: {stdout}\n{stderr}"
        );
        assert!(
            stderr.contains(&format!(
                "board pre-flight kept field {LEFTOVER}: run {LEFTOVER_RUN} {why}"
            )),
            "{scenario}: {stderr}"
        );
        assert!(fields_left(&stdout).contains(&LEFTOVER.to_owned()));
        assert_eq!(fields_left(&stdout).len(), 15);
        assert!(
            !requests(&stdout)
                .iter()
                .any(|request| request == DELETE_FIELD_DOCUMENT),
            "{scenario}"
        );
    }
}
