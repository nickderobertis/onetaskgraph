//! The live lane's board pre-flight against a loopback board holding a CI run's leftover field.
//!
//! Fields and items alike: a stamped field is always judged, and stamped items are judged when
//! the board holds more than the registered workload's items, which is when they would refuse
//! it.
//!
//! The pre-flight is the real one — `journey::preflight_board`, the call the credentialed
//! lane makes before it writes anything — recording into a real session record, which is what
//! makes it judge the board against the registered workload. What stands in for GitHub is a
//! local HTTP server holding a board of the registered size, two stamped fields more, and the
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
use onetaskgraph_github_live::{
    ARTIFACT_PREFIX, BOARD_WORKLOAD_DOCUMENT, CORE_REPOSITORY, DELETE_FIELD_DOCUMENT,
    RESIDUE_ITEMS_DOCUMENT,
};
use onetaskgraph_github_projects::DESIGN_TITLE_PREFIX;
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
/// The repository the lane writes its issues to.
const REPOSITORY: &str = "acme/work";
/// The run the leftover fields name, and the two fields it left.
const LEFTOVER_RUN: u64 = 4_242;
const LEFTOVER: &str = "otg-live-ci-4242-1-1700000000000001";
const SECOND_LEFTOVER: &str = "otg-live-ci-4242-1-1700000000000002";
/// Fields that are not a CI run's: the board's own, a machine stamp (its machine's lock
/// sweep's), and a name in the prefix that no stamp grammar reads.
const KEPT_ALWAYS: [&str; 3] = [
    "Priority",
    "otg-live-1-1-1700000000000001",
    "otg-live-ci-01-1-1",
];
/// A run still going, whose stamped items a pre-flight must leave and count.
const ACTIVE_RUN: u64 = 999;
/// What tells a re-execution of this binary which board to stand in for.
const CHILD_VARIABLE: &str = "ONETASKGRAPH_PREFLIGHT_GATE_CHILD";
/// The shared item listing as the lane's accounting sends it.
static PRICED_ITEMS: LazyLock<String> = LazyLock::new(|| {
    Accounting::with_query_prices()
        .priced_document(RESIDUE_ITEMS_DOCUMENT)
        .to_string()
});
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

type Item = (String, Option<(String, String)>, String);

/// The board, the run statuses GitHub reports and every request the pre-flight sent.
struct Board {
    /// Each item: its id, the issue behind it and that issue's repository (none for a
    /// draft), and its title.
    items: Vec<Item>,
    /// Each field as GitHub answers it under the measurement's two fragments.
    fields: Vec<Value>,
    more_fields: bool,
    answer_nodes: bool,
    /// The board's node id as the measurement answers it.
    board_id: &'static str,
    /// Which of the measurement's two counts it answers.
    answer_item_count: bool,
    answer_field_count: bool,
    /// A run's status, or `None` for a read GitHub fails.
    runs: BTreeMap<u64, Option<&'static str>>,
    removal: Removal,
    /// How many items one page of the artifact lookup answers, and how it is answered.
    item_page: usize,
    item_fault: Option<&'static str>,
    item_removal: Removal,
    issue_removal: Removal,
    issues_deleted: Vec<String>,
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
    let drafts = |count: usize| {
        (0..count)
            .map(|n| -> Item {
                (
                    format!("PVTI_draft_{n}"),
                    None,
                    "existing board note".to_owned(),
                )
            })
            .collect::<Vec<_>>()
    };
    let mut board = Board {
        items: drafts(60),
        fields,
        more_fields: false,
        answer_nodes: true,
        board_id: "PVT_board",
        answer_item_count: true,
        answer_field_count: true,
        runs: BTreeMap::from([(LEFTOVER_RUN, Some("completed"))]),
        removal: Removal::Confirmed,
        item_page: 100,
        item_fault: None,
        item_removal: Removal::Confirmed,
        issue_removal: Removal::Confirmed,
        issues_deleted: Vec::new(),
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
        // 80 of the board's own; 10 issues a completed run left, a document among them; 3 a run
        // still going left; and, under the completed run's stamp, a draft and an issue of
        // another repository, which are nobody's residue: 95 raw, 85 once the 10 are gone.
        "items-completed"
        | "items-paged"
        | "items-no-cursor"
        | "items-no-page-info"
        | "items-stuck-cursor"
        | "items-bad-id"
        | "items-empty-cursor"
        | "items-no-content"
        | "items-no-typename"
        | "items-no-repository"
        | "items-no-connection"
        | "items-no-nodes"
        | "items-endless"
        | "item-removal-refused"
        | "item-removal-unconfirmed"
        | "issue-removal-refused" => {
            board.items = drafts(79);
            // An item whose content this credential cannot see, answered as `null`.
            board
                .items
                .push(("PVTI_hidden".to_owned(), None, "(hidden)".to_owned()));
            board.items.extend(stamped_items(LEFTOVER_RUN, 10));
            board.items.extend(stamped_items(ACTIVE_RUN, 3));
            board.items.extend(not_residue());
            board.runs.insert(ACTIVE_RUN, Some("in_progress"));
            match scenario {
                "items-paged" | "items-stuck-cursor" => board.item_page = 40,
                "items-endless" => board.item_page = 1,
                "item-removal-refused" => board.item_removal = Removal::Refused,
                "item-removal-unconfirmed" => board.item_removal = Removal::NamingAnother,
                "issue-removal-refused" => board.issue_removal = Removal::Refused,
                _ => {}
            }
            board.item_fault = match scenario {
                "items-no-cursor" => Some("no-cursor"),
                "items-no-page-info" => Some("no-page-info"),
                "items-stuck-cursor" => Some("stuck-cursor"),
                "items-empty-cursor" => Some("empty-cursor"),
                "items-no-content" => Some("no-content"),
                "items-no-typename" => Some("no-typename"),
                "items-no-repository" => Some("no-repository"),
                "items-no-connection" => Some("no-connection"),
                "items-no-nodes" => Some("no-nodes"),
                "items-endless" => Some("endless"),
                "items-bad-id" => {
                    board.items.push((
                        "PVTI_bad".to_owned(),
                        Some((String::new(), REPOSITORY.to_owned())),
                        format!("{ARTIFACT_PREFIX}ci-{LEFTOVER_RUN}-1-1"),
                    ));
                    None
                }
                _ => None,
            };
        }
        // 82 of the board's own and 10 a run still going, unreadable or undecodable left: 92,
        // all counted.
        "items-active" | "items-unreadable" | "items-unknown-status" => {
            board.items = drafts(82);
            board.items.extend(stamped_items(ACTIVE_RUN, 10));
            board.runs.insert(
                ACTIVE_RUN,
                match scenario {
                    "items-active" => Some("in_progress"),
                    "items-unreadable" => None,
                    _ => Some("cancelled"),
                },
            );
        }
        "more-fields" => board.more_fields = true,
        "malformed-field" => board.fields.push(json!({"id":"FIELD_bad","name":7})),
        "empty-id" => board
            .fields
            .push(json!({"id":"","name":"otg-live-ci-1-1-1"})),
        "control-id" => board.fields.push(json!({"id":"FIELD\nbad","name":"x"})),
        "missing-nodes" => board.answer_nodes = false,
        "empty-board-id" => board.board_id = "",
        "control-board-id" => board.board_id = "PVT\nboard",
        "no-item-count" => board.answer_item_count = false,
        "no-field-count" => board.answer_field_count = false,
        "removal-refused" => board.removal = Removal::Refused,
        "removal-names-another" => board.removal = Removal::NamingAnother,
        other => panic!("no scenario {other}"),
    }
    board
}

/// `count` issues the lane wrote under `run`'s CI stamp, the first of them a document.
fn stamped_items(run: u64, count: usize) -> Vec<Item> {
    (0..count)
        .map(|n| {
            let design = if n == 0 { DESIGN_TITLE_PREFIX } else { "" };
            (
                format!("PVTI_{run}_{n}"),
                Some((format!("I_{run}_{n}"), REPOSITORY.to_owned())),
                format!(
                    "{design}{ARTIFACT_PREFIX}ci-{run}-1-{}",
                    1_700_000_000_000_100 + n
                ),
            )
        })
        .collect()
}

/// A draft and an issue of another repository, each titled under the completed run's stamp.
fn not_residue() -> Vec<Item> {
    let title = |n: u64| {
        format!(
            "{ARTIFACT_PREFIX}ci-{LEFTOVER_RUN}-1-{}",
            1_700_000_000_000_000 + n
        )
    };
    vec![
        ("PVTI_stamped_draft".to_owned(), None, title(1)),
        (
            "PVTI_foreign".to_owned(),
            Some(("I_foreign".to_owned(), "other/repository".to_owned())),
            title(2),
        ),
    ]
}

/// GitHub refusing a mutation the credential may not make.
fn refusal(mutation: &str) -> Value {
    json!({"data":{mutation:null},
           "errors":[{"message":"Resource not accessible by personal access token"}]})
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
            let mut fields = json!({"totalCount":board.fields.len(),"nodes":board.fields,
                                    "pageInfo":{"hasNextPage":board.more_fields}});
            if !board.answer_nodes {
                fields.as_object_mut().unwrap().remove("nodes");
            }
            if !board.answer_field_count {
                fields.as_object_mut().unwrap().remove("totalCount");
            }
            let items = if board.answer_item_count {
                json!({"totalCount":board.items.len()})
            } else {
                json!({})
            };
            return (
                200,
                json!({"data":{"repositoryOwner":{"projectV2":{"id":board.board_id,
                    "items":items,"fields":fields}}}}),
            );
        }
        // The pre-flight's listing of the board's items, the shared document priced.
        if query == *PRICED_ITEMS {
            assert_eq!(body["variables"]["id"], json!("PVT_board"));
            // The cursor is the offset of the page asked for.
            let offset = body["variables"]["after"]
                .as_str()
                .map_or(0, |after| after.parse::<usize>().unwrap());
            let end = (offset + board.item_page).min(board.items.len());
            let nodes = board.items[offset.min(end)..end]
                .iter()
                .map(|(id, issue, title)| match issue {
                    _ if id == "PVTI_hidden" => json!({"id":id,"content":null}),
                    Some((issue, repository)) => json!({"id":id,
                        "content":{"__typename":"Issue","id":issue,"title":title,
                                   "repository":{"nameWithOwner":repository}}}),
                    // A draft answers only its type: neither fragment selects from one.
                    None => json!({"id":id,"content":{"__typename":"DraftIssue"}}),
                })
                .collect::<Vec<_>>();
            // An endless board always says there is a page more, past its last item.
            let end = if board.item_fault == Some("endless") {
                offset + 1
            } else {
                end
            };
            let more = board.item_fault == Some("endless") || end < board.items.len();
            let mut items = json!({"nodes":nodes,"pageInfo":{"hasNextPage":more,
            "endCursor":if board.item_fault == Some("stuck-cursor") && offset > 0 {
                json!(offset.to_string())
            } else {
                json!(end.to_string())
            }}});
            match board.item_fault {
                Some("no-cursor") => items["pageInfo"] = json!({"hasNextPage":true}),
                Some("no-page-info") => {
                    items.as_object_mut().unwrap().remove("pageInfo");
                }
                Some("no-connection") => return (200, json!({"data":{"node":{}}})),
                Some("no-nodes") => {
                    items.as_object_mut().unwrap().remove("nodes");
                }
                Some("empty-cursor") => {
                    items["pageInfo"] = json!({"hasNextPage":true,"endCursor":""});
                }
                // The first node is one of the board's own drafts, answered malformed.
                Some("no-content") => items["nodes"][0] = json!({"id":"PVTI_draft_0"}),
                Some("no-typename") => items["nodes"][0]["content"] = json!({"title":"x"}),
                Some("no-repository") => {
                    items["nodes"][0]["content"] =
                        json!({"__typename":"Issue","id":"I_x","title":"existing board note"});
                }
                _ => {}
            }
            return (200, json!({"data":{"node":{"items":items}}}));
        }
        if query.contains("deleteProjectV2Item(input:$input)") {
            assert_eq!(body["variables"]["input"]["projectId"], json!("PVT_board"));
            let item = body["variables"]["input"]["itemId"].clone();
            return match board.item_removal {
                Removal::Refused => (200, refusal("deleteProjectV2Item")),
                Removal::NamingAnother => (
                    200,
                    json!({"data":{"deleteProjectV2Item":{"deletedItemId":"PVTI_draft_0"}}}),
                ),
                Removal::Confirmed => {
                    board.items.retain(|(id, _, _)| json!(id) != item);
                    (
                        200,
                        json!({"data":{"deleteProjectV2Item":{"deletedItemId":item}}}),
                    )
                }
            };
        }
        if query.contains("deleteIssue(input:$input)") {
            if board.issue_removal == Removal::Refused {
                return (200, refusal("deleteIssue"));
            }
            let issue = body["variables"]["input"]["issueId"]
                .as_str()
                .unwrap()
                .to_owned();
            board.issues_deleted.push(issue);
            return (
                200,
                json!({"data":{"deleteIssue":{"repository":{"id":"R_scratch"}}}}),
            );
        }
        assert_eq!(query, DELETE_FIELD_DOCUMENT, "an unexpected document");
        let field = body["variables"]["input"]["fieldId"]
            .as_str()
            .unwrap()
            .to_owned();
        return match board.removal {
            Removal::Refused => (200, refusal("deleteProjectV2Field")),
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

// llmlint: ignore[tests_assert_real_behavior] This subprocess helper is driven by the tests below, which assert on what it printed and how it exited; an ordinary test run omits its selector and starts no stand-in.
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
    let measured = journey::preflight_board(TOKEN, "octo-org", 7, REPOSITORY).await;
    let board = board.lock().unwrap();
    println!("measured: {measured:?}");
    println!(
        "requests: {}",
        serde_json::to_string(&board.requests).unwrap()
    );
    println!(
        "issues deleted: {}",
        serde_json::to_string(&board.issues_deleted).unwrap()
    );
    println!(
        "items left: {}",
        serde_json::to_string(&board.items.iter().map(|(_, _, t)| t).collect::<Vec<_>>()).unwrap()
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

fn issues_deleted(stdout: &str) -> Vec<String> {
    let line = stdout
        .lines()
        .find_map(|line| line.strip_prefix("issues deleted: "))
        .unwrap_or_else(|| panic!("the child printed no deleted issues: {stdout}"));
    serde_json::from_str(line).unwrap()
}

fn items_left(stdout: &str) -> Vec<String> {
    let line = stdout
        .lines()
        .find_map(|line| line.strip_prefix("items left: "))
        .unwrap_or_else(|| panic!("the child printed no items: {stdout}"));
    serde_json::from_str(line).unwrap()
}

/// How many of the requests sent were the lane's artifact lookup or an item or issue removal.
fn item_requests(stdout: &str) -> usize {
    requests(stdout)
        .iter()
        .filter(|request| {
            request.contains("on ProjectV2{items(first:100")
                || request.contains("deleteProjectV2Item(")
                || request.contains("deleteIssue(")
        })
        .count()
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
    // Within the registered workload's items, the board is not listed at all.
    assert_eq!(item_requests(&stdout), 0);
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
        (
            "malformed-field",
            "a field without a usable id and a name",
            0,
        ),
        ("empty-id", "a field without a usable id and a name", 0),
        ("control-id", "a field without a usable id and a name", 0),
        ("missing-nodes", "field names were not reported", 0),
        ("empty-board-id", "answered an unusable id", 0),
        ("control-board-id", "answered an unusable id", 0),
        (
            "no-item-count",
            "initial board item count was not reported",
            0,
        ),
        (
            "no-field-count",
            "initial board field count was not reported",
            0,
        ),
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
        // Nothing past the failure: the second field is never removed.
        assert!(
            fields_left(&stdout).contains(&SECOND_LEFTOVER.to_owned()),
            "{scenario}"
        );
    }
}

#[test]
fn a_measurement_that_cannot_be_judged_is_refused_before_anything_is_read_or_removed() {
    for scenario in [
        "empty-board-id",
        "control-board-id",
        "no-item-count",
        "no-field-count",
    ] {
        let (succeeded, stdout, stderr) = preflight(scenario);
        assert!(succeeded, "{scenario}: {stdout}\n{stderr}");
        // The measurement is the only request: no run is read and nothing is removed.
        assert_eq!(
            requests(&stdout),
            std::slice::from_ref(&*PRICED_WORKLOAD),
            "{scenario}"
        );
        assert_eq!(fields_left(&stdout).len(), 15, "{scenario}");
    }
}

#[test]
fn items_a_completed_run_left_past_the_registered_workload_are_removed_and_the_board_is_admitted() {
    let (succeeded, stdout, stderr) = preflight("items-completed");
    assert!(succeeded, "{stdout}\n{stderr}");
    assert!(
        stdout.contains(r#"measured: Ok("PVT_board")"#),
        "{stdout}\n{stderr}"
    );
    let left = items_left(&stdout);
    assert_eq!(left.len(), 85, "{left:?}");
    // The item this credential cannot see is left as it was, and the rest is still processed.
    assert!(left.contains(&"(hidden)".to_owned()));
    for (_, _, title) in stamped_items(LEFTOVER_RUN, 10) {
        assert!(!left.contains(&title), "{title}");
        assert!(
            stderr.contains(&format!(
                "board pre-flight removed item {title} and its issue: residue of run \
                 {LEFTOVER_RUN}, which reads completed"
            )),
            "{title}: {stderr}"
        );
    }
    for (_, _, title) in stamped_items(ACTIVE_RUN, 3) {
        assert!(left.contains(&title), "{title}");
        assert!(
            stderr.contains(&format!(
                "board pre-flight kept item {title}: run {ACTIVE_RUN} reads in_progress"
            )),
            "{title}: {stderr}"
        );
    }
    for (_, _, title) in not_residue() {
        assert!(left.contains(&title), "{title}");
        assert!(!stderr.contains(&title), "{title}: {stderr}");
    }
    let sent = requests(&stdout);
    // One read per run before each kind of removal, never per item: the leftover run also
    // left two fields, so its run is read once before those and once before its items.
    for (run, reads) in [(LEFTOVER_RUN, 2), (ACTIVE_RUN, 1)] {
        let read = format!("GET /repos/{CORE_REPOSITORY}/actions/runs/{run}");
        assert_eq!(sent.iter().filter(|r| **r == read).count(), reads, "{run}");
    }
    let count = |needle: &str| sent.iter().filter(|r| r.contains(needle)).count();
    assert_eq!(count("deleteProjectV2Item("), 10);
    // Each removed item's own issue, and no other.
    assert_eq!(
        issues_deleted(&stdout),
        stamped_items(LEFTOVER_RUN, 10)
            .into_iter()
            .map(|(_, issue, _)| issue.unwrap().0)
            .collect::<Vec<_>>()
    );
}

#[test]
fn a_board_listed_over_several_pages_is_recovered_whole() {
    let (succeeded, stdout, stderr) = preflight("items-paged");
    assert!(succeeded, "{stdout}\n{stderr}");
    assert!(
        stdout.contains(r#"measured: Ok("PVT_board")"#),
        "{stdout}\n{stderr}"
    );
    assert_eq!(items_left(&stdout).len(), 85);
    assert_eq!(issues_deleted(&stdout).len(), 10);
    // 95 items at 40 a page.
    let lookups = requests(&stdout)
        .iter()
        .filter(|r| r.contains("on ProjectV2{items(first:100"))
        .count();
    assert_eq!(lookups, 3);
}

#[test]
fn an_item_list_it_cannot_read_whole_or_a_removal_github_does_not_confirm_stops_there() {
    for (scenario, refusal, item_removals, issue_removals) in [
        ("items-no-cursor", "has no advancing cursor", 0, 0),
        ("items-stuck-cursor", "has no advancing cursor", 0, 0),
        (
            "items-no-page-info",
            "did not say whether it was complete",
            0,
            0,
        ),
        ("items-empty-cursor", "has no advancing cursor", 0, 0),
        ("items-no-connection", "returned no items connection", 0, 0),
        ("items-no-nodes", "nodes is not an array", 0, 0),
        ("items-endless", "did not end within 100 pages", 0, 0),
        ("items-no-content", "an item without content", 0, 0),
        ("items-no-typename", "content without its type", 0, 0),
        (
            "items-no-repository",
            "an issue without its repository",
            0,
            0,
        ),
        ("items-bad-id", "a stamped issue without usable ids", 0, 0),
        ("item-removal-refused", "Resource not accessible", 1, 0),
        ("item-removal-unconfirmed", "did not confirm removing", 1, 0),
        ("issue-removal-refused", "Resource not accessible", 1, 1),
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
        let sent = requests(&stdout);
        let count = |needle: &str| sent.iter().filter(|r| r.contains(needle)).count();
        assert_eq!(count("deleteProjectV2Item("), item_removals, "{scenario}");
        assert_eq!(count("deleteIssue("), issue_removals, "{scenario}");
        assert!(issues_deleted(&stdout).is_empty(), "{scenario}");
        assert!(
            !stderr.contains("board pre-flight removed item"),
            "{scenario}"
        );
    }
}

#[test]
fn items_no_completed_run_is_shown_to_own_stay_counted_and_the_board_is_refused() {
    for (scenario, why) in [
        ("items-active", "reads in_progress"),
        ("items-unreadable", "could not be read"),
        ("items-unknown-status", "answered no status this lane knows"),
    ] {
        let (succeeded, stdout, stderr) = preflight(scenario);
        assert!(succeeded, "{scenario}: {stdout}\n{stderr}");
        assert!(
            stdout.contains(
                "measured: Err(\"initial board workload is 92 items and 14 fields; registered \
                 workload requires 51 to 90 items and 14 fields\")"
            ),
            "{scenario}: {stdout}\n{stderr}"
        );
        assert_eq!(items_left(&stdout).len(), 92, "{scenario}");
        for (_, _, title) in stamped_items(ACTIVE_RUN, 10) {
            assert!(
                stderr.contains(&format!(
                    "board pre-flight kept item {title}: run {ACTIVE_RUN} {why}"
                )),
                "{scenario}: {title}: {stderr}"
            );
        }
        assert!(
            !requests(&stdout)
                .iter()
                .any(|r| r.contains("deleteProjectV2Item(") || r.contains("deleteIssue(")),
            "{scenario}"
        );
    }
}
