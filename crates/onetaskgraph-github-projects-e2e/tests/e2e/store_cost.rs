//! What each store operation the engine uses costs a GitHub board — a dependency read, a status
//! write and a settlement update — counted at the loopback board, on boards of 1,100 and of
//! 11,000 items.
//!
//! Every journey drives the compiled binary on a fresh process, as one command line is, except
//! the rate-limit memory's, which drives `onetaskgraph-core`'s own `Engine` because what it proves
//! is held for the life of one engine. Each `measure_*` journey records the figure of one budget in
//! `budgets.yaml` — the larger over the two scales, each scale in the recording — and
//! `budget_runner` hands it to onebudgetspec, which alone judges it.

use std::num::NonZeroU32;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use onetaskgraph_core::{
    Completeness, ConfiguredSource, DependencyRequest, Engine, GlobalId, Paging, ResolvedSource,
};
use onetaskgraph_github_projects::graphql;
use onetaskgraph_plugin_api::{
    Direction, SecretResolver, SourceError, SourceName, SourcePlugin as _,
};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use crate::budget_runner::directory;
use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{
    GITHUB_SECOND_BOARD, GitHubBoardFields, github_extra_task,
    github_projects_rate_limited_counted, github_projects_with_items,
};

/// The two board sizes every budget here is measured at.
pub(crate) const SCALES: [(&str, usize); 2] = [("1x", 1_100), ("10x", 11_000)];
/// The items the shared dataset holds before any extra task is added after it.
const SHARED: usize = 4;
/// The workloads the three budgets state, which their reports refuse any other recording of.
pub(crate) const DEPENDENCY_WORKLOAD: &str = "a board of 1,100 and of 11,000 items: an issue \
    blocked by 5 or 50 issues recording an edge in its body, both directions; a draft recording \
    an edge; a folder task recording an edge to an issue; an opaque edge resolved cold and warm \
    over two private boards; a legacy read of a copied task over a rate-limited private board \
    and one holding its reverse record; an unrelated edge-free task";
pub(crate) const STATUS_WORKLOAD: &str =
    "one status change of one issue item, fresh process, cache off, 1,100 and 11,000 items";
pub(crate) const SETTLEMENT_WORKLOAD: &str = "one status change and one metadata key on one \
    issue item, fresh process, cache off, 1,100 and 11,000 items";

/// Extra issues, `X-0` onwards, until the board holds `total` items; and `more` beside them.
fn board_of(sandbox: &Sandbox, total: usize, more: Vec<Value>) -> (Value, GitHubBoardFields) {
    let mut items: Vec<Value> = (0..total - SHARED - more.len())
        .map(|n| {
            let id = github_extra_task(n);
            json!({"item":format!("ITEM-{id}"),"id":id,"type":"Issue",
                "title":format!("Extra task {n}"),"body":format!("extra {n}"),"state":"OPEN",
                "reason":null,"parent":null,"repo":"nickderobertis/onetaskgraph",
                "status":"Todo","origin":"","labels":[]})
        })
        .collect();
    items.extend(more);
    github_projects_with_items(sandbox, items)
}

/// One board draft, `DRAFT-1`, as the dataset spells a board item.
fn draft() -> Value {
    json!({"item":"ITEM-DRAFT-1","id":"DRAFT-1","type":"DraftIssue","title":"Sketch",
        "body":"a draft","state":"OPEN","reason":null,"parent":null,"repo":null,
        "status":"Todo","origin":"","labels":[]})
}

fn folder(sandbox: &Sandbox, name: &str, tasks: &[(&str, String)]) -> PathBuf {
    let root = sandbox.subdirectory(name);
    std::fs::create_dir_all(root.join("tasks")).expect("the tasks folder");
    for (id, front) in tasks {
        std::fs::write(
            root.join("tasks").join(format!("{id}.md")),
            format!("---\n{front}---\n"),
        )
        .expect("a task file");
    }
    root
}

/// `sources` as a configuration document, with a write policy that passes everything — a store
/// declaring a visibility is held to the public boundary, which nothing here is about.
fn configured(sandbox: &Sandbox, sources: &Value) {
    sandbox.project_document(
        &serde_json::to_string_pretty(&json!({
            "sources": sources,
            "write_policy": {
                "check_command": ["python3", "-c",
                    "import sys; sys.stdin.read(); print('{\"verdict\": \"pass\"}')"],
                "visibility_command": ["python3", "-c",
                    "import sys; sys.stdin.read(); print('{\"visibility\": \"public\"}')"],
            },
        }))
        .expect("a document"),
    );
}

/// The contract's designator of a GitHub board whose project node id is `id`.
fn designator(id: &str) -> String {
    let digest: String = Sha256::digest(format!("github-projects\n{id}").as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("github-projects/{digest}")
}

/// `onetaskgraph <arguments>` on a fresh process with this sandbox's own cache home, exiting
/// `code`, and its JSON.
fn answered(sandbox: &Sandbox, arguments: &[&str], code: i32) -> Value {
    let output = sandbox
        .command()
        .env("XDG_CACHE_HOME", sandbox.subdirectory("cache"))
        .args(arguments)
        .write_stdin("")
        .assert()
        .get_output()
        .clone();
    assert_eq!(
        output.status.code(),
        Some(code),
        "`onetaskgraph {}`:\n{}\n{}",
        arguments.join(" "),
        stdout(&output),
        stderr(&output)
    );
    serde_json::from_str(&stdout(&output)).expect("machine output is JSON")
}

/// The `(from, to)` of every edge a dependency read answered, sorted.
fn edges(read: &Value) -> Vec<(String, String)> {
    let mut edges: Vec<(String, String)> = read["items"]
        .as_array()
        .expect("the edges")
        .iter()
        .map(|edge| {
            (
                edge["from"]["id"].as_str().expect("an id").to_owned(),
                edge["to"]["id"].as_str().expect("an id").to_owned(),
            )
        })
        .collect();
    edges.sort();
    edges
}

fn since(board: &GitHubBoardFields, from: usize) -> Vec<String> {
    board.served()[from..]
        .iter()
        .map(|(query, _)| query.clone())
        .collect()
}

/// The GraphQL points `sent` costs, by the plugin's own worst-case price of each document.
fn points(sent: &[String]) -> u64 {
    sent.iter()
        .map(|query| {
            onetaskgraph_github_projects::worst_case_point_cost(query)
                .expect("a document the plugin prices")
        })
        .sum()
}

/// One case of a dependency read: what each source was sent, and the points the boards that
/// answered spent on it — a refusal is priced at nothing.
struct Case {
    name: String,
    per_source: Vec<(&'static str, usize)>,
    points: u64,
}

impl Case {
    fn most(&self) -> usize {
        self.per_source
            .iter()
            .map(|(_, sent)| *sent)
            .max()
            .unwrap_or_default()
    }
    fn total(&self) -> usize {
        self.per_source.iter().map(|(_, sent)| *sent).sum()
    }
    fn render(&self) -> String {
        let each = self
            .per_source
            .iter()
            .map(|(source, sent)| format!("{source} {sent}"))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "{}: {each} (total {}, {} points)",
            self.name,
            self.total(),
            self.points
        )
    }
}

/// Write `budget`'s recording: the larger of each scale's figure, each scale's detail, and the
/// workload it was measured at.
fn record(budget: &str, workload: &str, scales: &[(&str, usize, String)]) {
    let value = scales
        .iter()
        .map(|(_, figure, _)| *figure)
        .max()
        .unwrap_or_default();
    let detail = scales
        .iter()
        .map(|(scale, figure, detail)| format!("{scale} {figure}: {detail}"))
        .collect::<Vec<_>>()
        .join("; ");
    let by_scale: serde_json::Map<String, Value> = scales
        .iter()
        .map(|(scale, figure, detail)| {
            (
                (*scale).to_owned(),
                json!({"value": figure, "detail": detail}),
            )
        })
        .collect();
    let directory = directory();
    std::fs::create_dir_all(&directory).expect("the telemetry directory");
    std::fs::write(
        onetaskgraph_e2e_support::telemetry::file_in(&directory, budget),
        json!({"budget": budget, "value": value, "workload": workload, "scales": by_scale,
               "detail": detail})
        .to_string(),
    )
    .expect("the telemetry file");
}

/// The dependency read's cases on a board of `total` items: the most any one source was sent
/// in any case, and every case.
fn dependency_cases(total: usize) -> (usize, String) {
    let blockers = if total > SCALES[0].1 { 50 } else { 5 };
    let mut cases = Vec::new();

    // An issue blocked by `blockers` issues, recording an edge to a folder task in its body;
    // and a draft recording the same.
    {
        let sandbox = Sandbox::new();
        let (block, board) = board_of(&sandbox, total, vec![draft()]);
        let near = github_extra_task(0);
        let blocking: Vec<String> = (1..=blockers).map(github_extra_task).collect();
        board.block(&near, &blocking);
        let recorded = json!([{"id": "notes:T-9", "kind": "task"}]);
        board.set_metadata(&near, "onetaskgraph.depends_on", recorded.clone());
        board.set_metadata("DRAFT-1", "onetaskgraph.depends_on", recorded);
        let notes = folder(
            &sandbox,
            "notes",
            &[
                ("T-9", "title: Far end\n".to_owned()),
                (
                    "T-1",
                    format!("title: Ship\ndepends_on:\n  - {{id: board:{near}, item: task}}\n"),
                ),
            ],
        );
        configured(
            &sandbox,
            &json!({
                "board": {"plugin": "github-projects", "config": block},
                "notes": {"plugin": "local-md", "config": {"root": notes}},
            }),
        );
        let from = board.served().len();
        let read = answered(
            &sandbox,
            &[
                "task",
                "deps",
                &format!("board:{near}"),
                "--limit",
                "100",
                "--json",
            ],
            0,
        );
        let mut expected: Vec<(String, String)> = blocking
            .iter()
            .map(|blocker| (format!("board:{near}"), format!("board:{blocker}")))
            .collect();
        expected.push((format!("board:{near}"), "notes:T-9".to_owned()));
        expected.sort();
        assert_eq!(edges(&read), expected, "every native and recorded edge");
        let sent = since(&board, from);
        assert_eq!(
            sent,
            [graphql::ISSUE_DEPENDS_ON],
            "what it depends on is one request, asking for `blockedBy` alone"
        );
        cases.push(Case {
            name: format!("depends-on of an issue blocked by {blockers}, recording one more"),
            per_source: vec![("board", sent.len())],
            points: points(&sent),
        });

        let from = board.served().len();
        let reverse = answered(
            &sandbox,
            &[
                "task",
                "deps",
                &format!("board:{}", blocking[0]),
                "--direction",
                "depended-on-by",
                "--limit",
                "100",
                "--json",
            ],
            0,
        );
        assert_eq!(
            edges(&reverse),
            [(format!("board:{near}"), format!("board:{}", blocking[0]))]
        );
        let sent = since(&board, from);
        assert_eq!(
            sent,
            [graphql::ISSUE_DEPENDED_ON_BY],
            "what depends on it is one request, asking for `blocking` alone"
        );
        cases.push(Case {
            name: "depended-on-by of one of its blockers".to_owned(),
            per_source: vec![("board", sent.len())],
            points: points(&sent),
        });

        let from = board.served().len();
        let drafted = answered(&sandbox, &["task", "deps", "board:DRAFT-1", "--json"], 0);
        assert_eq!(
            edges(&drafted),
            [("board:DRAFT-1".to_owned(), "notes:T-9".to_owned())]
        );
        let sent = since(&board, from);
        assert_eq!(
            sent,
            [graphql::ISSUE_DEPENDS_ON],
            "a draft's recorded edges ride on the same query: no draft read, no board read"
        );
        cases.push(Case {
            name: "depends-on of a draft recording one edge".to_owned(),
            per_source: vec![("board", sent.len())],
            points: points(&sent),
        });

        let from = board.served().len();
        let local = answered(&sandbox, &["task", "deps", "notes:T-1", "--json"], 0);
        assert_eq!(
            edges(&local),
            [("notes:T-1".to_owned(), format!("board:{near}"))]
        );
        let sent = board.served().len() - from;
        assert_eq!(sent, 0, "an edge into the board is never followed to it");
        cases.push(Case {
            name: "a folder task recording an edge to an issue, read by id".to_owned(),
            per_source: vec![("board", sent)],
            points: 0,
        });
    }

    // An opaque edge over two private boards: `pa`, board 8 of one server, which is not the
    // tracker the edge names, and `pb`, board 7 of another, which is — cold, then warm.
    {
        let sandbox = Sandbox::new();
        let (first, first_board) = board_of(&sandbox, total, Vec::new());
        let (second, second_board) = board_of(&sandbox, total, Vec::new());
        let near = github_extra_task(3);
        let mut other = first;
        other["project_number"] = json!(GITHUB_SECOND_BOARD);
        let far = designator("PVT-board");
        let notes = folder(
            &sandbox,
            "notes",
            &[(
                "T-2",
                format!(
                    "title: Ship\ndepends_on:\n  - {{designator: {far}, id: {near}, item: task}}\n"
                ),
            )],
        );
        configured(
            &sandbox,
            &json!({
                "notes": {"plugin": "local-md", "config": {"root": notes}, "visibility": "public"},
                "pa": {"plugin": "github-projects", "config": other, "visibility": "private"},
                "pb": {"plugin": "github-projects", "config": second, "visibility": "private"},
            }),
        );
        for warmth in ["cold", "warm"] {
            let (a, b) = (first_board.served().len(), second_board.served().len());
            let read = answered(&sandbox, &["task", "deps", "notes:T-2", "--json"], 0);
            assert_eq!(
                edges(&read),
                [("notes:T-2".to_owned(), format!("pb:{near}"))]
            );
            assert!(read.get("unanswered").is_none(), "{read}");
            let (sent_a, sent_b) = (since(&first_board, a), since(&second_board, b));
            let discovery = |sent: &[String]| {
                sent.iter()
                    .filter(|query| query.contains("boardFields:repositoryOwner"))
                    .count()
            };
            if warmth == "cold" {
                assert_eq!(
                    discovery(&sent_a),
                    1,
                    "pa says once which board it is: {sent_a:?}"
                );
                assert_eq!(sent_a.len(), 1, "and is asked nothing else: {sent_a:?}");
                assert_eq!(
                    discovery(&sent_b),
                    1,
                    "pb says once which board it is: {sent_b:?}"
                );
                assert_eq!(
                    sent_b.len(),
                    2,
                    "then the far end is read there: {sent_b:?}"
                );
            } else {
                assert_eq!(sent_a, Vec::<String>::new(), "kept on this host");
                assert_eq!(discovery(&sent_b), 0, "kept on this host: {sent_b:?}");
                assert_eq!(sent_b.len(), 1, "the far end alone: {sent_b:?}");
            }
            cases.push(Case {
                name: format!(
                    "an opaque edge resolved {warmth}, discovery {} and {}",
                    discovery(&sent_a),
                    discovery(&sent_b)
                ),
                per_source: vec![("pa", sent_a.len()), ("pb", sent_b.len())],
                points: points(&sent_a) + points(&sent_b),
            });
        }
    }

    // A legacy read: a copied folder task with no edges on its own record, over a private
    // board that answers rate limited and one recording the reverse edge; then a task no copy
    // landed, which asks neither.
    {
        let sandbox = Sandbox::new();
        let (limited, refusals) = github_projects_rate_limited_counted(&sandbox, Some(3_600));
        let holder = github_extra_task(5);
        let (holding, holding_board) = board_of(&sandbox, total, Vec::new());
        holding_board.set_metadata(
            &holder,
            "onetaskgraph.depended_on_by",
            json!({"notes:T-3": {"id": "notes:T-3", "kind": "task"}}),
        );
        let notes = folder(
            &sandbox,
            "notes",
            &[
                (
                    "T-3",
                    "title: Copied\nmetadata:\n  onetaskgraph.copied_destination_version: sha256:00\n"
                        .to_owned(),
                ),
                ("T-4", "title: Unrelated\n".to_owned()),
            ],
        );
        configured(
            &sandbox,
            &json!({
                "notes": {"plugin": "local-md", "config": {"root": notes}, "visibility": "public"},
                "pl": {"plugin": "github-projects", "config": limited, "visibility": "private"},
                "ph": {"plugin": "github-projects", "config": holding, "visibility": "private"},
            }),
        );
        for warmth in ["cold", "warm"] {
            let (refused, held) = (*refusals.lock().unwrap(), holding_board.served().len());
            let read = answered(&sandbox, &["task", "deps", "notes:T-3", "--json"], 0);
            assert_eq!(
                edges(&read),
                [("notes:T-3".to_owned(), format!("ph:{holder}"))],
                "the edge the answering board records"
            );
            let unanswered = read["unanswered"].as_array().expect("an unanswered source");
            assert_eq!(unanswered.len(), 1, "{read}");
            assert_eq!(unanswered[0]["source"], "pl", "{read}");
            let sent_l = *refusals.lock().unwrap() - refused;
            let asked = since(&holding_board, held);
            let sent_h = asked.len();
            assert_eq!(
                sent_l, 1,
                "the rate-limited board was asked once and refused"
            );
            assert!(sent_h >= 1, "the holding board was asked");
            cases.push(Case {
                name: format!("a legacy read {warmth}, one refusal and one edge query"),
                per_source: vec![("pl", sent_l), ("ph", sent_h)],
                points: points(&asked),
            });
        }
        let (refused, held) = (*refusals.lock().unwrap(), holding_board.served().len());
        let read = answered(&sandbox, &["task", "deps", "notes:T-4", "--json"], 0);
        assert_eq!(edges(&read), Vec::<(String, String)>::new());
        assert!(read.get("unanswered").is_none(), "{read}");
        let sent_l = *refusals.lock().unwrap() - refused;
        let sent_h = holding_board.served().len() - held;
        assert_eq!(
            (sent_l, sent_h),
            (0, 0),
            "a task no copy landed asks neither"
        );
        cases.push(Case {
            name: "an unrelated edge-free task".to_owned(),
            per_source: vec![("pl", sent_l), ("ph", sent_h)],
            points: 0,
        });
    }

    let most = cases.iter().map(Case::most).max().unwrap_or_default();
    let detail = cases
        .iter()
        .map(Case::render)
        .collect::<Vec<_>>()
        .join(", ");
    (most, detail)
}

// llmlint: ignore-block[expensive_tests_stay_behind_their_own_edge] The task places this budget journey in the e2e crate's `test` target, whose telemetry its budget reads, and AGENTS.md records the aggregate coverage run repeating every crate's tests as the accepted cost of one coverage report; the module runs in about three seconds on a warm build.
#[test]
fn measure_github_requests_per_dependency_read() {
    let scales: Vec<(&str, usize, String)> = SCALES
        .iter()
        .map(|(scale, total)| {
            let (most, detail) = dependency_cases(*total);
            (
                *scale,
                most,
                format!("{total} items, most to one source {most}: {detail}"),
            )
        })
        .collect();
    record(
        "github-requests-per-dependency-read",
        DEPENDENCY_WORKLOAD,
        &scales,
    );
}
// llmlint: ignore-end[expensive_tests_stay_behind_their_own_edge]

/// A status write and a settlement on a fresh process over a board of `total` items: what
/// each sent, every one checked to have landed and moved nothing else.
fn writes(total: usize) -> (usize, usize, String, String) {
    let sandbox = Sandbox::new();
    let (block, board) = board_of(&sandbox, total, Vec::new());
    configured(
        &sandbox,
        &json!({"board": {"plugin": "github-projects", "config": block}}),
    );
    let (status, settled) = (github_extra_task(0), github_extra_task(1));
    let before = (
        board.item(&status).expect("held"),
        board.item(&settled).expect("held"),
    );

    let from = board.served().len();
    answered(
        &sandbox,
        &[
            "task",
            "status",
            "set",
            &format!("board:{status}"),
            "in-progress",
            "--json",
        ],
        0,
    );
    let status_sent = since(&board, from);
    let after = board.item(&status).expect("held");
    assert_eq!(after["status"], "Doing", "the mapped option");
    for unchanged in ["title", "body", "state"] {
        assert_eq!(after[unchanged], before.0[unchanged], "{unchanged}");
    }

    let from = board.served().len();
    answered(
        &sandbox,
        &[
            "task",
            "update",
            &format!("board:{settled}"),
            "--status",
            "done",
            "--metadata",
            r#"onepipeline.settlement={"outcome":"landed"}"#,
            "--json",
        ],
        0,
    );
    let settlement_sent = since(&board, from);
    let after = board.item(&settled).expect("held");
    assert_eq!(after["status"], "Done");
    assert_eq!(after["title"], before.1["title"]);
    let body = after["body"].as_str().expect("a body");
    assert!(
        body.starts_with("extra 1"),
        "the person's text is kept: {body}"
    );
    assert!(
        body.contains(r#""onepipeline.settlement":{"outcome":"landed"}"#),
        "the key is stored: {body}"
    );
    let name = |sent: &[String]| {
        sent.iter()
            .map(|query| {
                graphql::DOCUMENTS
                    .iter()
                    .find(|(document, _)| document == query)
                    .map_or("another", |(_, said)| said)
                    .to_owned()
            })
            .collect::<Vec<_>>()
    };
    (
        status_sent.len(),
        settlement_sent.len(),
        format!("{:?}, {} points", name(&status_sent), points(&status_sent)),
        format!(
            "{:?}, {} points",
            name(&settlement_sent),
            points(&settlement_sent)
        ),
    )
}

// llmlint: ignore-block[expensive_tests_stay_behind_their_own_edge] The task places this budget journey in the e2e crate's `test` target, whose telemetry its budget reads, and AGENTS.md records the aggregate coverage run repeating every crate's tests as the accepted cost of one coverage report; the module runs in about three seconds on a warm build.
#[test]
fn measure_github_requests_per_status_write_and_settlement() {
    let measured: Vec<(&str, usize, usize, String, String)> = SCALES
        .iter()
        .map(|(scale, total)| {
            let (status, settlement, said, settled) = writes(*total);
            (*scale, status, settlement, said, settled)
        })
        .collect();
    record(
        "github-requests-per-status-write",
        STATUS_WORKLOAD,
        &measured
            .iter()
            .map(|(scale, status, _, said, _)| (*scale, *status, said.clone()))
            .collect::<Vec<_>>(),
    );
    record(
        "github-requests-per-settlement-update",
        SETTLEMENT_WORKLOAD,
        &measured
            .iter()
            .map(|(scale, _, settlement, _, settled)| (*scale, *settlement, settled.clone()))
            .collect::<Vec<_>>(),
    );
}
// llmlint: ignore-end[expensive_tests_stay_behind_their_own_edge]

struct Secrets;
impl SecretResolver for Secrets {
    fn get(&self, variable: &str) -> Option<secrecy::SecretString> {
        (variable == "GITHUB_PROJECTS_FIXTURE_TOKEN").then(|| "test-token".into())
    }
}

/// Within one engine, a board that answered rate limited with a reset is sent nothing before
/// that instant — every call before it fails locally, carrying the seconds left — and the call
/// after it reaches the board again.
// llmlint: ignore-block[expensive_tests_stay_behind_their_own_edge] The task places this budget journey in the e2e crate's `test` target, whose telemetry its budget reads, and AGENTS.md records the aggregate coverage run repeating every crate's tests as the accepted cost of one coverage report; the module runs in about three seconds on a warm build.
#[test]
fn a_board_that_named_a_reset_is_sent_nothing_before_it_and_reached_after_it() {
    let sandbox = Sandbox::new();
    let (block, sent): (Value, Arc<Mutex<usize>>) =
        github_projects_rate_limited_counted(&sandbox, Some(2));
    let name = SourceName::new("board").unwrap();
    let source = onetaskgraph_github_projects::Plugin
        .build(&name, &block, &Secrets)
        .expect("the board builds");
    let engine = Engine::new(
        vec![ConfiguredSource::Ready(ResolvedSource::adopt(
            name.clone(),
            source,
        ))],
        vec![name],
    );
    let id: GlobalId = "board:I-1".parse().unwrap();
    let limit = |answer: &[onetaskgraph_core::SourceFailure]| match &answer[0].error {
        SourceError::RateLimited {
            retry_after_seconds,
            message,
        } => (*retry_after_seconds, message.clone().unwrap_or_default()),
        other => panic!("not a rate limit: {other:?}"),
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let first = engine.task(&id).await.expect("an answer");
        let (wait, _) = limit(&first.errors);
        assert!(wait.is_some_and(|wait| (1..=2).contains(&wait)), "{wait:?}");
        assert_eq!(*sent.lock().unwrap(), 1, "the refusal itself");

        let again = engine.task(&id).await.expect("an answer");
        let (left, said) = limit(&again.errors);
        assert!(left.is_some_and(|left| (1..=2).contains(&left)), "{left:?}");
        assert!(said.contains("not sent"), "{said}");
        let edges = engine
            .task_dependencies(&DependencyRequest {
                id: id.clone(),
                direction: Direction::DependsOn,
                completeness: Completeness::default(),
                paging: Paging {
                    limit: NonZeroU32::new(10).unwrap(),
                    token: None,
                },
            })
            .await;
        let failed = match edges {
            Ok(answer) => answer.errors,
            Err(onetaskgraph_core::EngineError::SourceFailed { error, .. }) => {
                vec![onetaskgraph_core::SourceFailure {
                    source: SourceName::new("board").unwrap(),
                    error,
                }]
            }
            Err(other) => panic!("{other:?}"),
        };
        assert!(limit(&failed).1.contains("not sent"));
        assert_eq!(
            *sent.lock().unwrap(),
            1,
            "a held node's re-read costs no request"
        );
        // A write is held back as a read is: refused here, and never sent.
        let write = engine
            .update_task(
                &id,
                &onetaskgraph_plugin_api::TaskUpdate {
                    status: Some(onetaskgraph_plugin_api::Status {
                        category: onetaskgraph_plugin_api::StatusCategory::Done,
                        name: String::new(),
                    }),
                    ..onetaskgraph_plugin_api::TaskUpdate::default()
                },
            )
            .await;
        match write {
            Err(onetaskgraph_core::EngineError::SourceFailed {
                error: SourceError::RateLimited { message, .. },
                ..
            }) => assert!(message.unwrap_or_default().contains("not sent")),
            other => panic!("a write during the hold: {other:?}"),
        }
        assert_eq!(*sent.lock().unwrap(), 1, "the write was not sent");

        tokio::time::sleep(std::time::Duration::from_millis(2_300)).await;
        let after = engine.task(&id).await.expect("an answer");
        assert!(!limit(&after.errors).1.contains("not sent"));
        assert_eq!(
            *sent.lock().unwrap(),
            2,
            "the call after the reset reached the board"
        );
    });
}
// llmlint: ignore-end[expensive_tests_stay_behind_their_own_edge]

/// A board held off by the rate-limit memory is sent nothing before its reset even for a read
/// its cache could answer: the memory stands in front of every call, as its rule says. The same
/// pinned read is served from the cache, refused unsent during the hold, and served from the
/// cache again after the reset.
// llmlint: ignore-block[expensive_tests_stay_behind_their_own_edge] The task places this budget journey in the e2e crate's `test` target, whose telemetry its budget reads, and AGENTS.md records the aggregate coverage run repeating every crate's tests as the accepted cost of one coverage report; the module runs in about three seconds on a warm build.
#[test]
fn a_held_board_refuses_even_a_read_its_cache_holds_and_serves_it_after_the_reset() {
    let sandbox = Sandbox::new();
    let (mut block, board) = board_of(&sandbox, SHARED + 2, Vec::new());
    block["pacing"]["retry_budget_ms"] = json!(0);
    let cache = sandbox.subdirectory("cache");
    sandbox.project_document(
        &serde_json::to_string_pretty(&json!({"sources": {"board": {
            "plugin": "github-projects", "config": block, "cache": {"enabled": true}}}}))
        .expect("a document"),
    );
    let environment = onetaskgraph_core::Environment::from_pairs([
        (
            "XDG_CONFIG_HOME",
            sandbox.config_home().to_string_lossy().into_owned(),
        ),
        ("XDG_CACHE_HOME", cache.to_string_lossy().into_owned()),
    ]);
    let loaded = onetaskgraph_core::config::load(
        &sandbox.project(),
        &environment,
        &onetaskgraph_core::config::Layer::default(),
    )
    .expect("the configuration loads");
    let engine = Engine::build(&loaded.config, &loaded.secrets);
    let held: GlobalId = format!("board:{}", github_extra_task(0)).parse().unwrap();
    let other: GlobalId = format!("board:{}", github_extra_task(1)).parse().unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let first = engine.task(&held).await.expect("an answer");
        let pin = first.items[0]
            .item
            .version
            .clone()
            .expect("a read the cache keeps carries its version");
        let pinned = |engine: &Engine| engine.pin(&held, &pin);
        pinned(&engine);
        let before = board.served().len();
        let hit = engine.task(&held).await.expect("an answer");
        assert!(hit.errors.is_empty(), "{:?}", hit.errors);
        assert_eq!(
            board.served().len(),
            before,
            "a pinned read the cache holds sends nothing"
        );

        board.rate_limit_next(2);
        let limited = engine.task(&other).await.expect("an answer");
        assert!(matches!(
            limited.errors[0].error,
            SourceError::RateLimited {
                retry_after_seconds: Some(1..=2),
                ..
            }
        ));
        let before = board.served().len();
        pinned(&engine);
        let refused = engine.task(&held).await.expect("an answer");
        match &refused.errors[0].error {
            SourceError::RateLimited { message, .. } => {
                assert!(message.as_deref().unwrap_or_default().contains("not sent"));
            }
            other => panic!("not held off: {other:?}"),
        }
        assert_eq!(board.served().len(), before, "nothing sent during the hold");

        tokio::time::sleep(std::time::Duration::from_millis(2_300)).await;
        pinned(&engine);
        let after = engine.task(&held).await.expect("an answer");
        assert!(after.errors.is_empty(), "{:?}", after.errors);
        assert_eq!(after.items[0].item.version.as_deref(), Some(pin.as_str()));
        assert_eq!(
            board.served().len(),
            before,
            "served from the cache after the reset"
        );
    });
}
// llmlint: ignore-end[expensive_tests_stay_behind_their_own_edge]
