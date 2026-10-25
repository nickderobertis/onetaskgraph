//! The live lane's board pre-flight against a loopback board holding a CI run's leftover field.
//!
//! The pre-flight is the real one — `journey::preflight_board`, the call the credentialed
//! lane makes before it writes anything — recording into a real session record, which is what
//! makes it judge the board against the registered workload. What stands in for GitHub is a
//! local HTTP server holding a board of the registered size, one stamped field more, and the
//! Actions run that stamp names.
//!
//! A field whose run reads `completed` — a cancelled run does — is removed and the board is
//! admitted; any other field is left exactly where it is and counted, so a board holding a
//! field of a run still going is refused as before. No credential and no third-party API.

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
/// The run the leftover fields name, and the two fields it left.
const LEFTOVER_RUN: u64 = 40_170_293_588;
const LEFTOVER: &str = "otg-live-ci-40170293588-1-1792911749637960";
const SECOND_LEFTOVER: &str = "otg-live-ci-40170293588-1-1792911749637999";
/// Fields that are not a CI run's: the board's own, a machine stamp (its machine's lock
/// sweep's), and a name in the prefix that no stamp grammar reads.
const KEPT_ALWAYS: [&str; 3] = [
    "Priority",
    "otg-live-1-1-1792911749637960",
    "otg-live-ci-01-1-1",
];
/// What tells a re-execution of this binary which board to stand in for.
const CHILD_VARIABLE: &str = "ONETASKGRAPH_PREFLIGHT_GATE_CHILD";
/// The shared measurement as the lane's accounting sends it.
static PRICED_WORKLOAD: LazyLock<String> = LazyLock::new(|| {
    Accounting::with_query_prices()
        .priced_document(BOARD_WORKLOAD_DOCUMENT)
        .to_string()
});

/// How a removal is answered.
#[derive(Clone, Copy, PartialEq)]
enum Removal {
    Confirmed,
    Refused,
    NamingAnother,
}

/// The board, the run statuses GitHub reports and every request the pre-flight sent.
struct Board {
    items: u64,
    /// Each field as GitHub answers it under the measurement's two fragments.
    fields: Vec<Value>,
    more_fields: bool,
    /// A run's status, or `None` for a read GitHub fails.
    runs: BTreeMap<u64, Option<&'static str>>,
    removal: Removal,
    requests: Vec<String>,
}

/// A board of the registered size — 60 items, 14 fields of its own, one of them of a kind
/// the measurement selects nothing of — and two fields one CI run left.
fn board(scenario: &str) -> Board {
    let mut fields = (0..10)
        .map(|n| json!({"id":format!("FIELD_{n}"),"name":format!("board field {n}")}))
        .chain(
            KEPT_ALWAYS
                .iter()
                .enumerate()
                .map(|(n, name)| json!({"id":format!("FIELD_kept_{n}"),"name":name})),
        )
        .chain([json!({})])
        .collect::<Vec<_>>();
    fields.push(json!({"id":"FIELD_leftover","name":LEFTOVER}));
    fields.push(json!({"id":"FIELD_leftover_2","name":SECOND_LEFTOVER}));
    let mut board = Board {
        items: 60,
        fields,
        more_fields: false,
        runs: BTreeMap::from([(LEFTOVER_RUN, Some("completed"))]),
        removal: Removal::Confirmed,
        requests: Vec::new(),
    };
    // A run's status by scenario; `None` is a read GitHub fails.
    let status = match scenario {
        "in-progress" => Some("in_progress"),
        "unreadable" => None,
        // A conclusion, which a run's `status` never holds: undecodable, so nothing is shown.
        "unknown-status" => Some("cancelled"),
        _ => Some("completed"),
    };
    board.runs.insert(LEFTOVER_RUN, status);
    match scenario {
        "completed" | "in-progress" | "unreadable" | "unknown-status" => {}
        "more-fields" => board.more_fields = true,
        "malformed-field" => board.fields.push(json!({"id":"FIELD_bad","name":7})),
        "removal-refused" => board.removal = Removal::Refused,
        "removal-names-another" => board.removal = Removal::NamingAnother,
        other => panic!("no scenario {other}"),
    }
    board
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
            return (
                200,
                json!({"data":{"repositoryOwner":{"projectV2":{"id":"PVT_board",
                    "items":{"totalCount":board.items},
                    "fields":{"totalCount":board.fields.len(),"nodes":board.fields,
                              "pageInfo":{"hasNextPage":board.more_fields}}}}}}),
            );
        }
        assert_eq!(query, DELETE_FIELD_DOCUMENT, "an unexpected document");
        let field = body["variables"]["input"]["fieldId"]
            .as_str()
            .unwrap()
            .to_owned();
        return match board.removal {
            Removal::Refused => (
                200,
                json!({"data":{"deleteProjectV2Field":null},
                       "errors":[{"message":"Resource not accessible by personal access token"}]}),
            ),
            Removal::NamingAnother => (
                200,
                json!({"data":{"deleteProjectV2Field":{"projectV2Field":{"id":"FIELD_0"}}}}),
            ),
            Removal::Confirmed => {
                board.fields.retain(|held| held["id"] != json!(field));
                (
                    200,
                    json!({"data":{"deleteProjectV2Field":{"projectV2Field":{"id":field}}}}),
                )
            }
        };
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
    let measured = journey::preflight_board(TOKEN, "octo-org", 7).await;
    let board = board.lock().unwrap();
    println!("measured: {measured:?}");
    println!(
        "requests: {}",
        serde_json::to_string(&board.requests).unwrap()
    );
    println!(
        "fields left: {}",
        serde_json::to_string(
            &board
                .fields
                .iter()
                .filter_map(|field| field.get("name").and_then(Value::as_str))
                .collect::<Vec<_>>()
        )
        .unwrap()
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

fn removals(stdout: &str) -> usize {
    requests(stdout)
        .iter()
        .filter(|request| *request == DELETE_FIELD_DOCUMENT)
        .count()
}

#[test]
fn every_field_a_completed_run_left_is_removed_and_the_board_is_admitted() {
    let (succeeded, stdout, stderr) = preflight("completed");
    assert!(succeeded, "{stdout}\n{stderr}");
    assert!(
        stdout.contains(r#"measured: Ok("PVT_board")"#),
        "{stdout}\n{stderr}"
    );
    for leftover in [LEFTOVER, SECOND_LEFTOVER] {
        assert!(
            stderr.contains(&format!(
                "board pre-flight removed field {leftover}: residue of run {LEFTOVER_RUN}, \
                 which reads completed"
            )),
            "{stderr}"
        );
        assert!(!fields_left(&stdout).contains(&leftover.to_owned()));
    }
    // Every field that is not a CI run's is left untouched and never read about.
    for kept in KEPT_ALWAYS {
        assert!(fields_left(&stdout).contains(&kept.to_owned()), "{kept}");
        assert!(!stderr.contains(kept), "{kept}: {stderr}");
    }
    // One read for the run however many fields it left, then one removal per field.
    assert_eq!(
        requests(&stdout),
        [
            PRICED_WORKLOAD.clone(),
            format!("GET /repos/{CORE_REPOSITORY}/actions/runs/{LEFTOVER_RUN}"),
            DELETE_FIELD_DOCUMENT.to_owned(),
            DELETE_FIELD_DOCUMENT.to_owned(),
        ]
    );
}

#[test]
fn a_field_no_completed_run_is_shown_to_own_is_kept_counted_and_refused() {
    for (scenario, why) in [
        ("in-progress", "reads in_progress"),
        ("unreadable", "could not be read"),
        ("unknown-status", "answered no status this lane knows"),
    ] {
        let (succeeded, stdout, stderr) = preflight(scenario);
        assert!(succeeded, "{scenario}: {stdout}\n{stderr}");
        assert!(
            stdout.contains(
                "measured: Err(\"initial board workload is 60 items and 16 fields; registered \
                 workload requires 51 to 90 items and 14 fields\")"
            ),
            "{scenario}: {stdout}\n{stderr}"
        );
        for leftover in [LEFTOVER, SECOND_LEFTOVER] {
            assert!(
                stderr.contains(&format!(
                    "board pre-flight kept field {leftover}: run {LEFTOVER_RUN} {why}"
                )),
                "{scenario}: {stderr}"
            );
        }
        assert_eq!(fields_left(&stdout).len(), 15, "{scenario}");
        assert_eq!(removals(&stdout), 0, "{scenario}");
    }
}

#[test]
fn a_field_list_it_cannot_read_whole_or_a_removal_github_does_not_confirm_fails() {
    for (scenario, refusal, removals_sent) in [
        ("more-fields", "did not come back on one page", 0),
        ("malformed-field", "a field without a string id and name", 0),
        ("removal-refused", "Resource not accessible", 1),
        (
            "removal-names-another",
            "did not confirm removing field FIELD_leftover",
            1,
        ),
    ] {
        let (succeeded, stdout, stderr) = preflight(scenario);
        assert!(succeeded, "{scenario}: {stdout}\n{stderr}");
        let measured = stdout
            .lines()
            .find(|line| line.starts_with("measured: "))
            .unwrap_or_default();
        assert!(
            measured.starts_with("measured: Err(") && measured.contains(refusal),
            "{scenario}: {measured}"
        );
        assert_eq!(removals(&stdout), removals_sent, "{scenario}");
        // Nothing past the failure: neither the second field nor a run read is reached.
        assert!(
            fields_left(&stdout).contains(&SECOND_LEFTOVER.to_owned()),
            "{scenario}"
        );
    }
}
