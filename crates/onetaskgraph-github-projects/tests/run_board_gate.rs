//! Live sessions side by side on one GitHub account, against a loopback stand-in for it.
//!
//! Every session here is a real process — this test binary started again as a child — and
//! runs the lane's own session lifecycle: `journey::run_board::copy` of the nominated board,
//! `journey::preflight_board` of the copy, the origin field through `ensure_origin_field`,
//! its writes and its cursor walk on its own board, and `journey::run_board::delete`. A real
//! process each, because that is what a run is, and because the namespace guard is held per
//! process exactly as it is on a runner.
//!
//! The stand-in is one account: the nominated template, every copy a session makes, and the
//! limits GitHub holds an account to, enforced the way GitHub enforces them — an hourly
//! GraphQL allowance of 5,000 points (one per request, GitHub's minimum price), an hourly
//! cap of 500 content-creating requests, and 50 fields on a board. A request past a limit is
//! refused as GitHub refuses it, and the refusal is counted.
//!
//! What is proven: two sessions of each entry point at once each pass, each working only on
//! its own board, and neither naming, changing or removing the other's; eight runs at once —
//! each a main and an asset session, every one spending its registered design allocation
//! (`journey::budget::session_design_points`) — are none of them refused for any of those
//! limits; and a session that names a board outside its namespace is refused before the
//! request leaves it. GitHub's per-minute content-creation limit is not enforced here: a real
//! session's creations are spread across its whole duration, a quarter of an hour of settle
//! waits, and the plugin backs off on that limit rather than failing.

use std::collections::BTreeMap;
use std::io::{Read, Write as _};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

use onetaskgraph_github_live::quota::EntryPoint;
use onetaskgraph_github_live::{
    ADD_DRAFT_DOCUMENT, BOARD_WORKLOAD_DOCUMENT, RUN_BOARDS_DOCUMENT, TEMPLATE_DRAFT_TITLE,
    TEMPLATE_DRAFTS, TEMPLATE_ITEMS_DOCUMENT, run_board_stamp,
};
use serde_json::{Value, json};

#[allow(dead_code)]
mod journey;
#[allow(dead_code)]
mod lane;

const OWNER: &str = "octo-org";
const TEMPLATE: u32 = 1;
const REPOSITORY: &str = "acme/work";
/// GitHub's hourly GraphQL allowance for one account.
const POINTS_PER_HOUR: u64 = 5_000;
/// GitHub's hourly cap on content-creating requests.
const CREATIONS_PER_HOUR: u64 = 500;
/// GitHub's cap on a board's fields.
const FIELDS_PER_BOARD: usize = 50;
/// The template's fields, as many as the registered workload measured.
const TEMPLATE_FIELDS: usize = 14;
/// Content-creating requests a session sends besides its copy: the main journey's issues,
/// their board items, comments, label and scratch field (`fixtures/session-cost.txt`), and
/// the asset journey's two issues and their items.
const MAIN_CREATIONS: u64 = 24;
const ASSET_CREATIONS: u64 = 4;

#[derive(Clone, Debug)]
struct Board {
    id: String,
    number: u32,
    title: String,
    /// `(id, title)` of each draft item.
    items: Vec<(String, String)>,
    /// `(id, name)` of each field.
    fields: Vec<(String, String)>,
}

#[derive(Default)]
struct Account {
    boards: BTreeMap<u32, Board>,
    serial: u64,
    points: u64,
    creations: u64,
    /// Every request: the bearer token it carried, its document and its variables.
    seen: Vec<(String, String, Value)>,
    /// Every request refused for a limit, and which.
    refused: Vec<String>,
}

impl Account {
    fn new() -> Self {
        let mut account = Self::default();
        let fields = (0..TEMPLATE_FIELDS)
            .map(|n| {
                let name = if n == 0 {
                    "Status".to_owned()
                } else {
                    format!("Field {n}")
                };
                (format!("F_template_{n}"), name)
            })
            .collect();
        let items = (0..TEMPLATE_DRAFTS)
            .map(|n| (format!("DI_template_{n}"), TEMPLATE_DRAFT_TITLE.to_owned()))
            .collect();
        account.boards.insert(
            TEMPLATE,
            Board {
                id: "PVT_template".into(),
                number: TEMPLATE,
                title: "Plans".into(),
                items,
                fields,
            },
        );
        account
    }

    fn next(&mut self, prefix: &str) -> String {
        self.serial += 1;
        format!("{prefix}_{}", self.serial)
    }

    fn by_id(&mut self, id: &str) -> Option<&mut Board> {
        self.boards.values_mut().find(|board| board.id == id)
    }

    fn answer(&mut self, token: &str, document: &str, variables: &Value) -> Value {
        self.seen
            .push((token.to_owned(), document.to_owned(), variables.clone()));
        self.points += 1;
        if self.points > POINTS_PER_HOUR {
            self.refused.push(format!("points: {document}"));
            return json!({"errors":[{"type":"RATE_LIMITED","message":"API rate limit already exceeded for user ID 1."}]});
        }
        let input = &variables["input"];
        let creates = [
            "LiveCopyBoard(",
            "LiveTemplateDraft(",
            "createProjectV2Field(",
        ];
        if creates.iter().any(|name| document.contains(name)) {
            self.creations += 1;
            if self.creations > CREATIONS_PER_HOUR {
                self.refused.push(format!("creations: {document}"));
                return json!({"errors":[{"message":"You have exceeded a secondary rate limit and have been temporarily blocked from content creation."}]});
            }
        }
        if document.contains("query LiveTemplate(") {
            let board = self
                .boards
                .get(&number(variables))
                .map(|board| json!({"id":board.id}));
            return json!({"data":{"repositoryOwner":{"id":"U_octo","projectV2":board}}});
        }
        if document.contains("mutation LiveCopyBoard(") {
            let Some(template) = self.by_id(input["projectId"].as_str().unwrap()).cloned() else {
                return json!({"errors":[{"message":"Could not resolve to a ProjectV2"}]});
            };
            let number = self.boards.keys().max().unwrap() + 1;
            let id = format!("PVT_{number}");
            let items = if input["includeDraftIssues"] == json!(true) {
                template
                    .items
                    .iter()
                    .map(|(_, title)| (self.next("DI"), title.clone()))
                    .collect()
            } else {
                Vec::new()
            };
            let fields = template
                .fields
                .iter()
                .map(|(_, name)| (self.next("F"), name.clone()))
                .collect();
            let title = input["title"].as_str().unwrap().to_owned();
            self.boards.insert(
                number,
                Board {
                    id: id.clone(),
                    number,
                    title: title.clone(),
                    items,
                    fields,
                },
            );
            return json!({"data":{"copyProjectV2":{"projectV2":{"id":id,"number":number,"title":title}}}});
        }
        if document.contains("query LiveRunBoardItems(")
            || document.contains("query LiveBoardWorkload(")
        {
            let Some(board) = self.boards.get(&number(variables)) else {
                return json!({"data":{"repositoryOwner":{"projectV2":null}}});
            };
            let nodes: Vec<_> = board
                .fields
                .iter()
                .map(|(id, name)| json!({"id":id,"name":name}))
                .collect();
            return json!({"data":{"repositoryOwner":{"projectV2":{"id":board.id,
                "items":{"totalCount":board.items.len()},
                "fields":{"totalCount":nodes.len(),"nodes":nodes,"pageInfo":{"hasNextPage":false}}}}}});
        }
        if document.contains("on ProjectV2{fields(first:100") {
            let board = self.by_id(variables["id"].as_str().unwrap()).unwrap();
            let nodes: Vec<_> = board
                .fields
                .iter()
                .map(|(id, name)| json!({"id":id,"name":name}))
                .collect();
            return json!({"data":{"node":{"fields":{"nodes":nodes,"pageInfo":{"hasNextPage":false,"endCursor":null}}}}});
        }
        if document.contains("createProjectV2Field(") {
            let id = self.next("F");
            let board = self.by_id(input["projectId"].as_str().unwrap()).unwrap();
            if board.fields.len() >= FIELDS_PER_BOARD {
                let number = board.number;
                self.refused.push(format!("fields on #{number}"));
                return json!({"errors":[{"message":"Projects can have at most 50 fields"}]});
            }
            let board = self.by_id(input["projectId"].as_str().unwrap()).unwrap();
            let name = input["name"].as_str().unwrap().to_owned();
            board.fields.push((id.clone(), name.clone()));
            return json!({"data":{"createProjectV2Field":{"projectV2Field":{"id":id,"name":name}}}});
        }
        if document.contains("mutation LiveTemplateDraft(") {
            let id = self.next("DI");
            let board = self.by_id(input["projectId"].as_str().unwrap()).unwrap();
            board
                .items
                .push((id.clone(), input["title"].as_str().unwrap().to_owned()));
            return json!({"data":{"addProjectV2DraftIssue":{"projectItem":{"id":id}}}});
        }
        if document.contains("query LiveTemplateItems(") {
            let board = self.by_id(variables["id"].as_str().unwrap()).unwrap();
            let start: usize = variables["after"]
                .as_str()
                .map_or(0, |after| after.parse().unwrap());
            let nodes: Vec<_> = board.items.iter().skip(start).take(100)
                .map(|(id, title)| json!({"id":id,"content":{"__typename":"DraftIssue","title":title}}))
                .collect();
            let next = start + nodes.len();
            return json!({"data":{"node":{"items":{"nodes":nodes,"pageInfo":{
                "hasNextPage":next < board.items.len(),"endCursor":next.to_string()}}}}});
        }
        if document.contains("query LiveRunBoards(") {
            let nodes: Vec<_> = self
                .boards
                .values()
                .map(|board| json!({"id":board.id,"number":board.number,"title":board.title}))
                .collect();
            return json!({"data":{"repositoryOwner":{"projectsV2":{"nodes":nodes,"pageInfo":{"hasNextPage":false,"endCursor":null}}}}});
        }
        if document.contains("mutation LiveDeleteBoard(") {
            let id = input["projectId"].as_str().unwrap().to_owned();
            let Some(number) = self
                .boards
                .values()
                .find(|board| board.id == id)
                .map(|board| board.number)
            else {
                return json!({"errors":[{"message":"Could not resolve to a ProjectV2"}]});
            };
            assert_ne!(number, TEMPLATE, "the template was deleted");
            self.boards.remove(&number);
            return json!({"data":{"deleteProjectV2":{"projectV2":{"id":id}}}});
        }
        json!({"errors":[{"message":format!("the stand-in does not answer {document}")}]})
    }
}

fn number(variables: &Value) -> u32 {
    u32::try_from(variables["number"].as_u64().unwrap()).unwrap()
}

/// The stand-in, serving GraphQL and `GET /rate_limit` until the test ends.
fn serve(account: Arc<Mutex<Account>>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let account = Arc::clone(&account);
            thread::spawn(move || answer(stream, &account));
        }
    });
    origin
}

fn answer(mut stream: TcpStream, account: &Mutex<Account>) {
    let mut bytes = Vec::new();
    let mut buffer = [0; 16 * 1024];
    let (head, body) = loop {
        let Ok(read) = stream.read(&mut buffer) else {
            return;
        };
        if read == 0 {
            return;
        }
        bytes.extend_from_slice(&buffer[..read]);
        let Some(split) = bytes.windows(4).position(|window| window == b"\r\n\r\n") else {
            continue;
        };
        let head = String::from_utf8_lossy(&bytes[..split]).into_owned();
        let length: usize = head
            .lines()
            .find_map(|line| {
                line.split_once(':')
                    .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                    .map(|(_, value)| value.trim().parse().unwrap())
            })
            .unwrap_or(0);
        if bytes.len() >= split + 4 + length {
            break (head, bytes[split + 4..split + 4 + length].to_vec());
        }
    };
    let token = head
        .lines()
        .find_map(|line| {
            line.split_once(':')
                .filter(|(name, _)| name.eq_ignore_ascii_case("authorization"))
                .map(|(_, value)| value.trim().trim_start_matches("Bearer ").to_owned())
        })
        .unwrap_or_default();
    let reply = if head.starts_with("GET /rate_limit") {
        let account = account.lock().unwrap();
        let remaining = POINTS_PER_HOUR.saturating_sub(account.points);
        json!({"resources":{"core":{"limit":5000,"remaining":5000,"reset":4_000_000_000_u64,"used":0},
            "graphql":{"limit":POINTS_PER_HOUR,"remaining":remaining,"reset":4_000_000_000_u64,"used":account.points}}})
    } else {
        let request: Value = serde_json::from_slice(&body).unwrap();
        account.lock().unwrap().answer(
            &token,
            request["query"].as_str().unwrap_or_default(),
            &request["variables"],
        )
    };
    let reply = reply.to_string();
    let _ = write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
        reply.len()
    );
}

/// What one child session reports on its standard output, last line.
#[derive(Debug, serde::Deserialize)]
struct Report {
    board: u32,
    walked: usize,
    own: usize,
}

/// Starts one session as a child process of this binary, against `origin`.
fn session(
    origin: &str,
    entry: EntryPoint,
    token: &str,
    run_id: u64,
    extra: &[(&str, &str)],
) -> std::process::Child {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "session_child",
            "--exact",
            "--nocapture",
            "--test-threads",
            "1",
        ])
        .env("RUN_BOARD_GATE_ORIGIN", origin)
        .env(
            "RUN_BOARD_GATE_ENTRY",
            if entry == EntryPoint::Main {
                "main"
            } else {
                "assets"
            },
        )
        .env("RUN_BOARD_GATE_TOKEN", token)
        .env("RUN_BOARD_GATE_RUN", run_id.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (name, value) in extra {
        command.env(name, value);
    }
    command.spawn().unwrap()
}

fn finish(child: std::process::Child) -> Result<Report, String> {
    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() {
        return Err(format!(
            "{stdout}\n{}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let line = stdout
        .lines()
        .find_map(|line| {
            line.split_once("run-board-gate report: ")
                .map(|(_, report)| report)
        })
        .ok_or_else(|| format!("no report: {stdout}"))?;
    Ok(serde_json::from_str(line).unwrap())
}

/// Every board number and id a request names.
fn named(variables: &Value, boards: &[(String, u32)]) -> Vec<u32> {
    let text = variables.to_string();
    let mut found: Vec<u32> = boards
        .iter()
        .filter(|(id, _)| text.contains(&format!("\"{id}\"")))
        .map(|(_, number)| *number)
        .collect();
    if let Some(number) = variables.get("number").and_then(Value::as_u64) {
        found.push(u32::try_from(number).unwrap());
    }
    found
}

/// The child: one whole session of `entry`'s lifecycle on the account at `origin`.
#[tokio::test]
async fn session_child() {
    let Ok(origin) = std::env::var("RUN_BOARD_GATE_ORIGIN") else {
        return;
    };
    journey::against(
        journey::Endpoints::loopback(&format!("{origin}/graphql"), &format!("{origin}/")).unwrap(),
    );
    let token = std::env::var("RUN_BOARD_GATE_TOKEN").unwrap();
    let setup = match std::env::var("RUN_BOARD_GATE_ENTRY").unwrap().as_str() {
        "template-dry-run" => Some(journey::run_board::TemplateSetup::DryRun),
        "template-fill" => Some(journey::run_board::TemplateSetup::Fill),
        _ => None,
    };
    if let Some(setup) = setup {
        let send = journey::run_board::main_sender(&token);
        for line in journey::run_board::setup_template(&send, OWNER, TEMPLATE, setup)
            .await
            .unwrap()
        {
            println!("run-board-gate template: {line}");
        }
        return;
    }
    let entry = if std::env::var("RUN_BOARD_GATE_ENTRY").unwrap() == "main" {
        EntryPoint::Main
    } else {
        EntryPoint::Assets
    };
    let run = onetaskgraph_live::artifact::CiRun::read(
        &std::env::var("RUN_BOARD_GATE_RUN").unwrap(),
        "1",
    )
    .unwrap();
    let writer = lane::Writer::Ci(run);
    let stamp = writer.stamp(onetaskgraph_live::artifact::now_micros());
    let send = journey::run_board::main_sender(&token);
    let board = journey::run_board::copy(&send, OWNER, TEMPLATE, &stamp)
        .await
        .unwrap_or_else(|error| panic!("copy: {error}"));
    let guard = journey::run_board::Guard::new(&token, &board);
    let outcome = async {
        let project_id = journey::preflight_board(&token, OWNER, board.number, REPOSITORY).await?;
        let fields = journey::writable_fields(&token, &project_id).await?;
        journey::ensure_origin_field(&token, &project_id, &fields).await?;
        let creations = if entry == EntryPoint::Main {
            MAIN_CREATIONS
        } else {
            ASSET_CREATIONS
        };
        for n in 0..creations {
            send(
                ADD_DRAFT_DOCUMENT,
                "session write",
                json!({"input":{"projectId":project_id,"title":format!("{stamp} item {n}")}}),
            )
            .await?;
        }
        // The deliberate breach, when asked for: the template, then another run's board.
        if std::env::var_os("RUN_BOARD_GATE_TRESPASS").is_some() {
            for (what, variables) in [
                ("the template", json!({"owner":OWNER,"number":TEMPLATE})),
                ("another run's board", {
                    let listed = send(
                        RUN_BOARDS_DOCUMENT,
                        "boards",
                        json!({"owner":OWNER,"query":"onetaskgraph live board","after":null}),
                    )
                    .await?;
                    let other = listed["data"]["repositoryOwner"]["projectsV2"]["nodes"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|node| {
                            node["number"] != json!(board.number)
                                && node["number"] != json!(TEMPLATE)
                                && node["title"]
                                    .as_str()
                                    .is_some_and(|title| run_board_stamp(title).is_some())
                        })
                        .ok_or("no other run board to trespass on")?
                        .clone();
                    json!({"owner":OWNER,"number":other["number"]})
                }),
            ] {
                let refused = send(BOARD_WORKLOAD_DOCUMENT, "trespass", variables).await;
                let refusal = refused.err().ok_or(format!("naming {what} was sent"))?;
                if !refusal.contains("refused before sending") {
                    return Err(format!("naming {what} failed otherwise: {refusal}"));
                }
                println!("run-board-gate refused: {what}");
            }
        }
        // Spend the rest of this entry point's registered design allocation: what a whole
        // session draws from the account's allowance, at one point a request.
        let spent = journey::SESSION.snapshot().total_requests() as u64;
        let design = journey::budget::session_design_points(entry);
        for _ in spent..design.saturating_sub(4) {
            send(
                BOARD_WORKLOAD_DOCUMENT,
                "session read",
                json!({"owner":OWNER,"number":board.number}),
            )
            .await?;
        }
        // The cursor walk over its own board: the template's drafts and its own writes alone.
        let mut after = Value::Null;
        let mut titles = Vec::new();
        loop {
            let page = send(
                TEMPLATE_ITEMS_DOCUMENT,
                "walk",
                json!({"id":project_id,"after":after}),
            )
            .await?;
            let items = &page["data"]["node"]["items"];
            for node in items["nodes"].as_array().unwrap() {
                titles.push(node["content"]["title"].as_str().unwrap().to_owned());
            }
            if items["pageInfo"]["hasNextPage"] != json!(true) {
                break;
            }
            after = items["pageInfo"]["endCursor"].clone();
        }
        let own = titles
            .iter()
            .filter(|title| title.starts_with(&stamp))
            .count();
        let foreign: Vec<_> = titles
            .iter()
            .filter(|title| !title.starts_with(&stamp) && *title != TEMPLATE_DRAFT_TITLE)
            .collect();
        if !foreign.is_empty() || own != usize::try_from(creations).unwrap() {
            return Err(format!(
                "the walk saw {own} own items and these others: {foreign:?}"
            ));
        }
        Ok::<_, String>(titles.len())
    }
    .await;
    let deleted = journey::run_board::delete(&send, &board).await;
    guard.deleted();
    let walked = outcome.unwrap_or_else(|error| panic!("session: {error}"));
    deleted.unwrap();
    let own = if entry == EntryPoint::Main {
        MAIN_CREATIONS
    } else {
        ASSET_CREATIONS
    };
    println!(
        "run-board-gate report: {}",
        json!({"board":board.number,"walked":walked,"own":own})
    );
}

fn assert_isolated(account: &Account, tokens: &[String], reports: &[Report]) {
    let mut boards: Vec<(String, u32)> = Vec::new();
    // Each session's board, from its own copy's answer: the board numbers it reported.
    for report in reports {
        boards.push((format!("PVT_{}", report.board), report.board));
        assert_eq!(
            report.walked,
            usize::try_from(TEMPLATE_DRAFTS).unwrap() + report.own
        );
    }
    boards.push(("PVT_template".into(), TEMPLATE));
    let numbers: std::collections::BTreeSet<_> =
        reports.iter().map(|report| report.board).collect();
    assert_eq!(numbers.len(), reports.len(), "two sessions shared a board");
    for (token, report) in tokens.iter().zip(reports) {
        for (_, document, variables) in account.seen.iter().filter(|(sender, ..)| sender == token) {
            for board in named(variables, &boards) {
                let template_read = board == TEMPLATE
                    && (document.contains("LiveTemplate(") || document.contains("LiveCopyBoard("));
                assert!(
                    board == report.board || template_read,
                    "session {token} named board #{board} in {document}"
                );
            }
        }
    }
    // Every copy is gone, and the template is as it was.
    assert_eq!(
        account.boards.keys().copied().collect::<Vec<_>>(),
        vec![TEMPLATE]
    );
    let template = &account.boards[&TEMPLATE];
    assert_eq!(
        template.items.len(),
        usize::try_from(TEMPLATE_DRAFTS).unwrap()
    );
    assert_eq!(template.fields.len(), TEMPLATE_FIELDS);
    assert_eq!(
        account.refused,
        Vec::<String>::new(),
        "a session was refused for a limit"
    );
}

#[test]
fn two_concurrent_sessions_of_each_entry_point_pass_on_boards_of_their_own() {
    let account = Arc::new(Mutex::new(Account::new()));
    let origin = serve(Arc::clone(&account));
    let sessions = [
        (EntryPoint::Main, "main-a", 101),
        (EntryPoint::Main, "main-b", 102),
        (EntryPoint::Assets, "assets-a", 101),
        (EntryPoint::Assets, "assets-b", 102),
    ];
    let children: Vec<_> = sessions
        .iter()
        .map(|(entry, token, run)| session(&origin, *entry, token, *run, &[]))
        .collect();
    let reports: Vec<_> = children
        .into_iter()
        .map(|child| finish(child).unwrap_or_else(|error| panic!("{error}")))
        .collect();
    let tokens: Vec<_> = sessions
        .iter()
        .map(|(_, token, _)| (*token).to_owned())
        .collect();
    assert_isolated(&account.lock().unwrap(), &tokens, &reports);
}

#[test]
fn eight_runs_at_once_stay_inside_every_account_and_board_limit() {
    let account = Arc::new(Mutex::new(Account::new()));
    let origin = serve(Arc::clone(&account));
    let sessions: Vec<_> = (0..8u64)
        .flat_map(|run| {
            [
                (EntryPoint::Main, format!("main-{run}"), 200 + run),
                (EntryPoint::Assets, format!("assets-{run}"), 200 + run),
            ]
        })
        .collect();
    let children: Vec<_> = sessions
        .iter()
        .map(|(entry, token, run)| session(&origin, *entry, token, *run, &[]))
        .collect();
    let reports: Vec<_> = children
        .into_iter()
        .map(|child| finish(child).unwrap_or_else(|error| panic!("{error}")))
        .collect();
    let tokens: Vec<_> = sessions.iter().map(|(_, token, _)| token.clone()).collect();
    let account = account.lock().unwrap();
    assert_isolated(&account, &tokens, &reports);
    // What the eight runs drew, against what GitHub leaves an account and the share no
    // session may touch: every run's whole design allocation fits beside seven others.
    let design = journey::budget::session_design_points(EntryPoint::Main)
        + journey::budget::session_design_points(EntryPoint::Assets);
    let usable = POINTS_PER_HOUR - onetaskgraph_live::RETAINED_BUFFER.of(POINTS_PER_HOUR);
    assert!(
        8 * design <= usable,
        "{} points for eight runs against {usable}",
        8 * design
    );
    assert!(
        account.points <= usable,
        "{} points spent against {usable}",
        account.points
    );
    assert!(account.creations <= CREATIONS_PER_HOUR);
}

#[test]
fn a_session_naming_a_board_outside_its_run_is_refused_before_sending() {
    let account = Arc::new(Mutex::new(Account::new()));
    let origin = serve(Arc::clone(&account));
    // A session whose board stays up while the trespasser runs: its walk waits on nothing,
    // so it is started first and the trespasser finds it listed.
    let other = Command::new(std::env::current_exe().unwrap())
        .args(["session_child", "--exact", "--nocapture"])
        .env("RUN_BOARD_GATE_ORIGIN", &origin)
        .env("RUN_BOARD_GATE_ENTRY", "main")
        .env("RUN_BOARD_GATE_TOKEN", "bystander")
        .env("RUN_BOARD_GATE_RUN", "301")
        .output()
        .unwrap();
    assert!(
        other.status.success(),
        "{}",
        String::from_utf8_lossy(&other.stderr)
    );
    // The bystander's board is gone by now; seed one as a live run's copy would stand.
    let seeded = {
        let mut account = account.lock().unwrap();
        let number = account.boards.keys().max().unwrap() + 1;
        account.boards.insert(
            number,
            Board {
                id: format!("PVT_{number}"),
                number,
                title: onetaskgraph_github_live::run_board_title("ci-302-1-1"),
                items: Vec::new(),
                fields: Vec::new(),
            },
        );
        number
    };
    let child = session(
        &origin,
        EntryPoint::Assets,
        "trespasser",
        303,
        &[("RUN_BOARD_GATE_TRESPASS", "1")],
    );
    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("run-board-gate refused: the template"),
        "{stdout}"
    );
    assert!(
        stdout.contains("run-board-gate refused: another run's board"),
        "{stdout}"
    );
    // Neither request reached the account, and the other run's board is as it stood.
    let account = account.lock().unwrap();
    let trespasses: Vec<_> = account
        .seen
        .iter()
        .filter(|(token, document, variables)| {
            token == "trespasser"
                && !document.contains("LiveTemplate(")
                && !document.contains("LiveCopyBoard(")
                && named(
                    variables,
                    &[
                        ("PVT_template".into(), TEMPLATE),
                        (format!("PVT_{seeded}"), seeded),
                    ],
                )
                .iter()
                .any(|board| *board == TEMPLATE || *board == seeded)
        })
        .collect();
    assert_eq!(
        trespasses,
        Vec::<&(String, String, Value)>::new(),
        "a refused request reached the account"
    );
    assert!(account.boards.contains_key(&seeded));
}

fn template_setup(origin: &str, action: &str) -> String {
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["session_child", "--exact", "--nocapture"])
        .env("RUN_BOARD_GATE_ORIGIN", origin)
        .env("RUN_BOARD_GATE_ENTRY", action)
        .env("RUN_BOARD_GATE_TOKEN", "operator")
        .env("RUN_BOARD_GATE_RUN", "1")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    stdout
}

#[test]
fn the_template_setup_dry_runs_without_writing_then_fills_once() {
    let account = Arc::new(Mutex::new(Account::new()));
    account
        .lock()
        .unwrap()
        .boards
        .get_mut(&TEMPLATE)
        .unwrap()
        .items
        .truncate(10);
    let origin = serve(Arc::clone(&account));
    let writes = |account: &Account| {
        account
            .seen
            .iter()
            .filter(|(_, document, _)| document.starts_with("mutation"))
            .count()
    };
    let said = template_setup(&origin, "template-dry-run");
    assert!(said.contains("template #1 holds 10 draft items"), "{said}");
    assert!(
        said.contains(&format!("would add {} draft items", TEMPLATE_DRAFTS - 10)),
        "{said}"
    );
    assert_eq!(writes(&account.lock().unwrap()), 0, "a dry run wrote");
    let said = template_setup(&origin, "template-fill");
    assert!(
        said.contains(&format!("added {} draft items", TEMPLATE_DRAFTS - 10)),
        "{said}"
    );
    let drafts = account.lock().unwrap().boards[&TEMPLATE].items.len();
    assert_eq!(drafts, usize::try_from(TEMPLATE_DRAFTS).unwrap());
    let before = writes(&account.lock().unwrap());
    let said = template_setup(&origin, "template-fill");
    assert!(said.contains("nothing to add"), "{said}");
    assert_eq!(
        writes(&account.lock().unwrap()),
        before,
        "a second fill wrote"
    );
    // A template already past the registered workload is reported, never trimmed.
    {
        let mut account = account.lock().unwrap();
        let template = account.boards.get_mut(&TEMPLATE).unwrap();
        for n in 0..40 {
            template
                .items
                .push((format!("DI_extra_{n}"), TEMPLATE_DRAFT_TITLE.to_owned()));
        }
    }
    let said = template_setup(&origin, "template-fill");
    assert!(said.contains("remove draft items by hand"), "{said}");
    assert_eq!(writes(&account.lock().unwrap()), before);
}
