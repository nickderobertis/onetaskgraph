//! What a dependency read costs a Linear workspace, counted at each loopback workspace, on
//! workspaces of 60 and of 600 issues; and the engine's rate-limit memory over a Linear source.
//!
//! Every read is the compiled binary on a fresh process, as one command line is, except the
//! rate-limit memory's journeys, which drive `onetaskgraph-core`'s own `Engine` — built from a
//! configuration document as the binary builds one, on a clock the journey moves by hand where
//! a reset has to pass — because the memory they prove is held for the life of one engine. The `measure_*` journey
//! records `linear-requests-per-dependency-read` — the most any one source was sent in any case,
//! at either scale — and `budget_runner::report` hands it to onebudgetspec, which alone judges it.

use std::num::NonZeroU32;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use onetaskgraph_core::{
    Completeness, ConfiguredSource, DependencyRequest, Engine, GlobalId, Paging, ResolvedSource,
};
use onetaskgraph_linear::graphql;
use onetaskgraph_plugin_api::{Direction, SourceError, SourceName, SourcePlugin as _};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{LinearWorkspace, dataset, linear_rate_limited_for, linear_workspace};
use crate::telemetry::record;

/// The two workspace sizes the budget is measured at.
const SCALES: [(&str, usize); 2] = [("1x", 60), ("10x", 600)];

/// One Linear issue at `Todo`, carrying `extra` beside its title.
fn issue(id: &str, extra: Value) -> Value {
    let mut issue = json!({"id": id, "title": format!("Issue {id}"), "content": "",
                           "status": {"category": "todo", "name": "Todo"}, "labels": []});
    issue
        .as_object_mut()
        .expect("an issue")
        .extend(extra.as_object().expect("extra keys").clone());
    issue
}

/// A workspace of organization `organization` holding `held` and unrelated issues after them
/// until it holds `total`.
fn workspace(
    sandbox: &Sandbox,
    organization: &str,
    mut held: Vec<Value>,
    total: usize,
) -> (Value, LinearWorkspace) {
    let start = held.len();
    held.extend((start..total).map(|at| issue(&format!("F-{at}"), json!({}))));
    let mut data = dataset();
    for (_, kept) in data.as_object_mut().expect("the dataset is an object") {
        *kept = json!([]);
    }
    data["tasks"] = json!(held);
    data["organization"] = json!(organization);
    linear_workspace(sandbox, data)
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

/// The contract's designator of the Linear workspace whose organization id is `id`.
fn designator(id: &str) -> String {
    let digest: String = Sha256::digest(format!("linear\n{id}").as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("linear/{digest}")
}

/// `onetaskgraph <arguments>` on a fresh process with this sandbox's own cache home, exiting
/// zero, and its JSON.
fn answered(sandbox: &Sandbox, arguments: &[&str]) -> Value {
    let output = sandbox
        .command()
        .env("XDG_CACHE_HOME", sandbox.subdirectory("cache"))
        .args(arguments)
        .write_stdin("")
        .assert()
        .get_output()
        .clone();
    assert!(
        output.status.success(),
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

fn since(workspace: &LinearWorkspace, from: usize) -> Vec<String> {
    workspace.served()[from..]
        .iter()
        .map(|(query, _)| query.clone())
        .collect()
}

/// One case: what each source was sent.
struct Case {
    name: String,
    per_source: Vec<(&'static str, usize)>,
    /// The complexity points the workspaces that answered spent on it, priced by the cache
    /// budget's table of Linear's measured costs; a refusal answers nothing and is priced at none.
    points: u64,
}

/// The complexity points `workspace` was charged for what it served since `from`.
fn points(workspace: &LinearWorkspace, from: usize) -> u64 {
    let served = workspace.served();
    let answered = workspace.answered();
    served[from..]
        .iter()
        .zip(&answered[from..])
        .map(|((document, variables), answer)| {
            crate::cache_budget::complexity(document, variables, answer)
        })
        .sum()
}

impl Case {
    fn most(&self) -> usize {
        self.per_source
            .iter()
            .map(|(_, sent)| *sent)
            .max()
            .unwrap_or_default()
    }
    fn render(&self) -> String {
        let each = self
            .per_source
            .iter()
            .map(|(source, sent)| format!("{source} {sent}"))
            .collect::<Vec<_>>()
            .join(", ");
        let total: usize = self.per_source.iter().map(|(_, sent)| *sent).sum();
        format!(
            "{}: {each} (total {total}, {} complexity points)",
            self.name, self.points
        )
    }
}

/// Every case on workspaces of `total` issues.
fn cases(total: usize) -> Vec<Case> {
    let relations = if total > SCALES[0].1 { 50 } else { 5 };
    let mut cases = Vec::new();

    // An issue with `relations` relations and an edge recorded in its description, both
    // directions; and a folder task recording an edge to it, read by id.
    {
        let sandbox = Sandbox::new();
        let recorded = json!({"onetaskgraph.depends_on": [{"id": "notes:T-9", "kind": "task"}]});
        // `L-R` is what `relations` issues depend on, each recording an edge of its own besides.
        let dependents: Vec<String> = (1..=relations).map(|at| format!("D-{at}")).collect();
        let mut held = vec![
            issue("L-0", json!({"metadata": recorded.clone()})),
            issue("L-R", json!({})),
        ];
        held.extend(
            dependents
                .iter()
                .map(|id| issue(id, json!({"metadata": recorded.clone()}))),
        );
        let (config, linear) = workspace(&sandbox, "org-work", held, total);
        let related: Vec<String> = (1..=relations).map(|at| format!("F-{at}")).collect();
        for far in &related {
            linear.relate("L-0", far, "blocks");
        }
        for dependent in &dependents {
            linear.relate(dependent, "L-R", "blocks");
        }
        let notes = folder(
            &sandbox,
            "notes",
            &[
                ("T-9", "title: Far end\n".to_owned()),
                (
                    "T-1",
                    "title: Ship\ndepends_on:\n  - {id: linear:L-0, item: task}\n".to_owned(),
                ),
                (
                    "T-5",
                    "title: Waits\ndepends_on:\n  - {id: linear:L-R, item: task}\n".to_owned(),
                ),
            ],
        );
        configured(
            &sandbox,
            &json!({
                "linear": {"plugin": "linear", "config": config},
                "notes": {"plugin": "local-md", "config": {"root": notes}},
            }),
        );
        let from = linear.served().len();
        let read = answered(
            &sandbox,
            &["task", "deps", "linear:L-0", "--limit", "100", "--json"],
        );
        let mut expected: Vec<(String, String)> = related
            .iter()
            .map(|far| ("linear:L-0".to_owned(), format!("linear:{far}")))
            .collect();
        expected.push(("linear:L-0".to_owned(), "notes:T-9".to_owned()));
        expected.sort();
        assert_eq!(edges(&read), expected, "every native and recorded edge");
        let sent = since(&linear, from);
        assert_eq!(
            sent,
            [graphql::ISSUE_DEPENDS_ON],
            "one request, asking for `relations` alone"
        );
        assert!(
            !sent[0].contains("inverseRelations"),
            "no reverse connection: {}",
            sent[0]
        );
        cases.push(Case {
            name: format!("depends-on of an issue with {relations} relations, recording one more"),
            per_source: vec![("linear", sent.len())],
            points: points(&linear, from),
        });

        let from = linear.served().len();
        let reverse = answered(
            &sandbox,
            &[
                "task",
                "deps",
                "linear:L-R",
                "--direction",
                "depended-on-by",
                "--limit",
                "100",
                "--json",
            ],
        );
        let mut expected: Vec<(String, String)> = dependents
            .iter()
            .map(|dependent| (format!("linear:{dependent}"), "linear:L-R".to_owned()))
            .collect();
        expected.sort();
        // Exactly the native reverse edges. A recorded edge lives on the item that depends and
        // is never written down on its far end, and the store's read scope asks no other source
        // about a reverse read: so neither `notes:T-5`'s edge into `L-R` nor the dependents' own
        // recorded edges are answered here — the forward reads above answer those.
        assert_eq!(
            edges(&reverse),
            expected,
            "every native reverse edge, and no recorded one"
        );
        let sent = since(&linear, from);
        assert_eq!(
            sent,
            [graphql::ISSUE_DEPENDED_ON_BY],
            "one request, asking for `inverseRelations` alone"
        );
        assert!(
            !sent[0].contains(" relations(") && !sent[0].contains("description"),
            "no forward connection and no long form: {}",
            sent[0]
        );
        cases.push(Case {
            name: format!("depended-on-by of an issue {relations} issues depend on"),
            per_source: vec![("linear", sent.len())],
            points: points(&linear, from),
        });

        let from = linear.served().len();
        let local = answered(&sandbox, &["task", "deps", "notes:T-1", "--json"]);
        assert_eq!(
            edges(&local),
            [("notes:T-1".to_owned(), "linear:L-0".to_owned())]
        );
        let sent = linear.served().len() - from;
        assert_eq!(sent, 0, "an edge into Linear is never followed to it");
        cases.push(Case {
            name: "a folder task recording an edge to an issue, read by id".to_owned(),
            per_source: vec![("linear", sent)],
            points: 0,
        });
    }

    // An opaque edge over two private Linear sources of two workspaces — `la` the one the edge
    // does not name, `lb` the one it does — cold, then warm.
    {
        let sandbox = Sandbox::new();
        let (first, first_workspace) = workspace(
            &sandbox,
            "org-elsewhere",
            vec![issue("L-9", json!({}))],
            total,
        );
        let (second, second_workspace) =
            workspace(&sandbox, "org-plans", vec![issue("L-9", json!({}))], total);
        let far = designator("org-plans");
        let notes = folder(
            &sandbox,
            "notes",
            &[(
                "T-2",
                format!(
                    "title: Ship\ndepends_on:\n  - {{designator: {far}, id: L-9, item: task}}\n"
                ),
            )],
        );
        configured(
            &sandbox,
            &json!({
                "notes": {"plugin": "local-md", "config": {"root": notes}, "visibility": "public"},
                "la": {"plugin": "linear", "config": first, "visibility": "private"},
                "lb": {"plugin": "linear", "config": second, "visibility": "private"},
            }),
        );
        for warmth in ["cold", "warm"] {
            let (a, b) = (
                first_workspace.served().len(),
                second_workspace.served().len(),
            );
            let read = answered(&sandbox, &["task", "deps", "notes:T-2", "--json"]);
            assert_eq!(
                edges(&read),
                [("notes:T-2".to_owned(), "lb:L-9".to_owned())]
            );
            assert!(read.get("unanswered").is_none(), "{read}");
            let (sent_a, sent_b) = (since(&first_workspace, a), since(&second_workspace, b));
            let discovery = |sent: &[String]| {
                sent.iter()
                    .filter(|query| *query == graphql::ORGANIZATION)
                    .count()
            };
            if warmth == "cold" {
                assert_eq!(
                    sent_a,
                    [graphql::ORGANIZATION],
                    "la says once which workspace"
                );
                assert_eq!(
                    sent_b,
                    [graphql::ORGANIZATION, graphql::ISSUE],
                    "lb says once which workspace, then the far end is read there"
                );
            } else {
                assert_eq!(sent_a, Vec::<String>::new(), "kept on this host");
                assert_eq!(
                    sent_b,
                    [graphql::ISSUE],
                    "kept on this host: the far end alone"
                );
            }
            cases.push(Case {
                name: format!(
                    "an opaque edge resolved {warmth}, discovery {} and {}",
                    discovery(&sent_a),
                    discovery(&sent_b)
                ),
                per_source: vec![("la", sent_a.len()), ("lb", sent_b.len())],
                points: points(&first_workspace, a) + points(&second_workspace, b),
            });
        }
    }

    // A legacy read: a copied folder task with no edges on its own record, over a private
    // source that answers rate limited and one recording the reverse edge; then a task no copy
    // landed, which asks neither.
    {
        let sandbox = Sandbox::new();
        let (limited, refusals) = linear_rate_limited_for(&sandbox, Some(3_600));
        let (holding, holding_workspace) = workspace(
            &sandbox,
            "org-vault",
            vec![issue(
                "V-1",
                json!({"metadata": {"onetaskgraph.depended_on_by":
                    {"notes:T-3": {"id": "notes:T-3", "kind": "task"}}}}),
            )],
            total,
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
                "lr": {"plugin": "linear", "config": limited, "visibility": "private"},
                "lh": {"plugin": "linear", "config": holding, "visibility": "private"},
            }),
        );
        for warmth in ["cold", "warm"] {
            let (refused, held) = (*refusals.lock().unwrap(), holding_workspace.served().len());
            let read = answered(&sandbox, &["task", "deps", "notes:T-3", "--json"]);
            assert_eq!(
                edges(&read),
                [("notes:T-3".to_owned(), "lh:V-1".to_owned())],
                "the edge the answering source records"
            );
            let unanswered = read["unanswered"].as_array().expect("an unanswered source");
            assert_eq!(unanswered.len(), 1, "{read}");
            assert_eq!(unanswered[0]["source"], "lr", "{read}");
            let sent_r = *refusals.lock().unwrap() - refused;
            let sent_h = holding_workspace.served().len() - held;
            assert_eq!(
                sent_r, 1,
                "the rate-limited source was asked once and refused"
            );
            assert_eq!(
                since(&holding_workspace, held),
                [graphql::ISSUES],
                "the holding source was asked once, by the one query naming the dependent"
            );
            cases.push(Case {
                name: format!("a legacy read {warmth}, one refusal and one edge query"),
                per_source: vec![("lr", sent_r), ("lh", sent_h)],
                points: points(&holding_workspace, held),
            });
        }
        let (refused, held) = (*refusals.lock().unwrap(), holding_workspace.served().len());
        let read = answered(&sandbox, &["task", "deps", "notes:T-4", "--json"]);
        assert_eq!(edges(&read), Vec::<(String, String)>::new());
        assert!(read.get("unanswered").is_none(), "{read}");
        let sent_r = *refusals.lock().unwrap() - refused;
        let sent_h = holding_workspace.served().len() - held;
        assert_eq!(
            (sent_r, sent_h),
            (0, 0),
            "a task no copy landed asks neither"
        );
        cases.push(Case {
            name: "an unrelated edge-free task".to_owned(),
            per_source: vec![("lr", sent_r), ("lh", sent_h)],
            points: 0,
        });
    }

    // A held task re-read while its source answered rate limited with a reset still ahead.
    let (first, again) = held_reread();
    cases.push(Case {
        name: "a held task read, then re-read before its source's reset".to_owned(),
        per_source: vec![("linear", first), ("linear, re-read", again)],
        points: 0,
    });
    cases
}

// llmlint: ignore-block[expensive_tests_stay_behind_their_own_edge] The task places this budget journey in the e2e crate's `test` target, whose telemetry its budget reads, and AGENTS.md records the aggregate coverage run repeating every crate's tests as the accepted cost of one coverage report; the module runs in about three seconds on a warm build.
#[test]
fn measure_linear_requests_per_dependency_read() {
    let measured: Vec<(usize, String)> = SCALES
        .iter()
        .map(|(scale, total)| {
            let cases = cases(*total);
            let most = cases.iter().map(Case::most).max().unwrap_or_default();
            let detail = cases
                .iter()
                .map(Case::render)
                .collect::<Vec<_>>()
                .join(", ");
            (most, format!("{scale} ({total} issues) {most}: {detail}"))
        })
        .collect();
    record(
        "linear-requests-per-dependency-read",
        measured
            .iter()
            .map(|(most, _)| *most)
            .max()
            .unwrap_or_default(),
        &measured
            .iter()
            .map(|(_, detail)| detail.as_str())
            .collect::<Vec<_>>()
            .join("; "),
    );
}
// llmlint: ignore-end[expensive_tests_stay_behind_their_own_edge]

/// An engine over one Linear source answering rate limited with a reset `reset` seconds ahead,
/// and the count of what it was sent.
fn limited_engine(sandbox: &Sandbox, reset: u64) -> (Engine, Arc<Mutex<usize>>) {
    let (config, sent) = linear_rate_limited_for(sandbox, Some(reset));
    let name = SourceName::new("linear").unwrap();
    let secrets = onetaskgraph_core::Secrets::load(onetaskgraph_core::Environment::from_pairs([(
        "LINEAR_API_KEY",
        "fixture-key",
    )]))
    .expect("an environment with no credentials file");
    let source = onetaskgraph_linear::Plugin
        .build(&name, &config, &secrets)
        .expect("the Linear source builds");
    (
        Engine::new(
            vec![ConfiguredSource::Ready(ResolvedSource::adopt(
                name.clone(),
                source,
            ))],
            vec![name],
        ),
        sent,
    )
}

/// The rate limit a failed answer carries: the seconds it names, and what it says.
fn limit(errors: &[onetaskgraph_core::SourceFailure]) -> (Option<u64>, String) {
    match &errors[0].error {
        SourceError::RateLimited {
            retry_after_seconds,
            message,
        } => (*retry_after_seconds, message.clone().unwrap_or_default()),
        other => panic!("not a rate limit: {other:?}"),
    }
}

/// What a held task's read and its re-read before the reset each sent.
fn held_reread() -> (usize, usize) {
    let sandbox = Sandbox::new();
    let (engine, sent) = limited_engine(&sandbox, 3_600);
    let id: GlobalId = "linear:L-1".parse().unwrap();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            limit(&engine.task(&id).await.expect("an answer").errors);
            let first = *sent.lock().unwrap();
            let (left, said) = limit(&engine.task(&id).await.expect("an answer").errors);
            assert!(said.contains("not sent"), "{said}");
            assert!(left.is_some_and(|left| left > 3_500), "{left:?}");
            (first, *sent.lock().unwrap() - first)
        })
}

/// Within one engine, a Linear source that answered rate limited with a reset is sent nothing
/// before that instant — every call before it fails locally, carrying the seconds left — and
/// the call after it reaches Linear again.
// llmlint: ignore-block[expensive_tests_stay_behind_their_own_edge] The task places this budget journey in the e2e crate's `test` target, whose telemetry its budget reads, and AGENTS.md records the aggregate coverage run repeating every crate's tests as the accepted cost of one coverage report; the module runs in about three seconds on a warm build.
#[test]
fn a_linear_source_that_named_a_reset_is_sent_nothing_before_it_and_reached_after_it() {
    let sandbox = Sandbox::new();
    let (engine, sent) = limited_engine(&sandbox, 2);
    let id: GlobalId = "linear:L-1".parse().unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let (wait, said) = limit(&engine.task(&id).await.expect("an answer").errors);
        assert!(wait.is_some_and(|wait| (1..=2).contains(&wait)), "{wait:?}");
        assert!(!said.contains("not sent"), "{said}");
        assert_eq!(*sent.lock().unwrap(), 1, "the refusal itself");

        let (left, said) = limit(&engine.task(&id).await.expect("an answer").errors);
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
                    source: SourceName::new("linear").unwrap(),
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
        // A batched read is refused for every id it names, and none of them is sent.
        let batched = engine
            .task_details(&[id.clone(), "linear:L-2".parse().unwrap()], false)
            .await;
        assert_eq!(batched.details.len(), 2);
        for detail in &batched.details {
            assert!(limit(&detail.response.errors).1.contains("not sent"));
        }
        assert_eq!(*sent.lock().unwrap(), 1, "the batch was not sent");

        tokio::time::sleep(std::time::Duration::from_millis(2_300)).await;
        let (_, said) = limit(&engine.task(&id).await.expect("an answer").errors);
        assert!(!said.contains("not sent"), "{said}");
        assert_eq!(
            *sent.lock().unwrap(),
            2,
            "the call after the reset reached Linear"
        );
    });
}
// llmlint: ignore-end[expensive_tests_stay_behind_their_own_edge]

/// A rate limit that names no reset holds the source off not at all: every call is sent.
#[test]
fn a_linear_rate_limit_naming_no_reset_holds_nothing_back() {
    let sandbox = Sandbox::new();
    let (config, sent) = linear_rate_limited_for(&sandbox, None);
    let name = SourceName::new("linear").unwrap();
    let secrets = onetaskgraph_core::Secrets::load(onetaskgraph_core::Environment::from_pairs([(
        "LINEAR_API_KEY",
        "fixture-key",
    )]))
    .expect("an environment with no credentials file");
    let source = onetaskgraph_linear::Plugin
        .build(&name, &config, &secrets)
        .expect("the Linear source builds");
    let engine = Engine::new(
        vec![ConfiguredSource::Ready(ResolvedSource::adopt(
            name.clone(),
            source,
        ))],
        vec![name],
    );
    let id: GlobalId = "linear:L-1".parse().unwrap();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            for asked in 1..=2 {
                let (wait, said) = limit(&engine.task(&id).await.expect("an answer").errors);
                assert_eq!(wait, None);
                assert!(!said.contains("not sent"), "{said}");
                assert_eq!(*sent.lock().unwrap(), asked, "every call reached Linear");
            }
        });
}

/// A clock that moves only when a journey moves it, and whose every wait moves it at once.
#[derive(Default)]
struct Manual(std::sync::atomic::AtomicU64);

impl Manual {
    fn advance(&self, seconds: u64) {
        self.0.fetch_add(
            seconds.saturating_mul(1_000),
            std::sync::atomic::Ordering::SeqCst,
        );
    }
}

impl onetaskgraph_plugin_api::Clock for Manual {
    fn now(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.0.load(std::sync::atomic::Ordering::SeqCst))
    }
    fn sleep(
        &self,
        duration: std::time::Duration,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>> {
        self.0.fetch_add(
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX),
            std::sync::atomic::Ordering::SeqCst,
        );
        Box::pin(async {})
    }
}

/// An engine built from a configuration document, as the binary builds one, over a loopback
/// Linear workspace holding `L-1`, `L-2` and `L-3`, on a clock the journey moves by hand.
fn held_engine() -> (Sandbox, LinearWorkspace, Engine, Arc<Manual>) {
    let sandbox = Sandbox::new();
    let held = ["L-1", "L-2", "L-3"]
        .iter()
        .map(|id| issue(id, json!({})))
        .collect();
    let (config, linear) = workspace(&sandbox, "org-work", held, 3);
    sandbox.project_document(
        &serde_json::to_string_pretty(&json!({"sources": {
            "linear": {"plugin": "linear", "config": config}}}))
        .expect("a document"),
    );
    let environment = onetaskgraph_core::Environment::from_pairs([
        (
            "XDG_CONFIG_HOME",
            sandbox.config_home().to_string_lossy().into_owned(),
        ),
        (
            "XDG_CACHE_HOME",
            sandbox.subdirectory("cache").to_string_lossy().into_owned(),
        ),
    ]);
    let loaded = onetaskgraph_core::config::load(
        &sandbox.project(),
        &environment,
        &onetaskgraph_core::config::Layer::default(),
    )
    .expect("the configuration loads");
    let clock = Arc::new(Manual::default());
    let shared: onetaskgraph_plugin_api::SharedClock = clock.clone();
    let engine = Engine::build_with_clock(&loaded.config, &loaded.secrets, &shared);
    (sandbox, linear, engine, clock)
}

/// The reset a rate-limited answer names, or `None` for an answer that is not one; and
/// whether the engine refused it without sending it.
fn held_back(errors: &[onetaskgraph_core::SourceFailure]) -> Option<(Option<u64>, bool)> {
    match &errors.first()?.error {
        SourceError::RateLimited {
            retry_after_seconds,
            message,
        } => Some((
            *retry_after_seconds,
            message.as_deref().unwrap_or_default().contains("not sent"),
        )),
        _ => None,
    }
}

/// Through the engine the binary builds, on a hand-moved clock, over loopback Linear: a rate
/// limit Linear answers inside a batched read starts the hold, keeping the later of the two
/// resets that batch is answered with; a later call sends nothing and is told the seconds
/// left; ending the command during the hold reaches the source and is not refused; and the
/// first call at the reset reaches Linear and is answered.
#[test]
fn a_rate_limit_inside_a_batch_holds_linear_off_until_its_later_reset() {
    let (_sandbox, linear, engine, clock) = held_engine();
    let id = |native: &str| -> GlobalId { format!("linear:{native}").parse().unwrap() };
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            linear.rate_limit(&[None, Some(30), Some(10)]);
            let batch = engine
                .task_details(&[id("L-1"), id("L-2"), id("L-3")], false)
                .await;
            assert_eq!(batch.details[0].response.items.len(), 1, "L-1 is answered");
            assert_eq!(
                held_back(&batch.details[1].response.errors),
                Some((Some(30), false)),
                "Linear refused L-2, naming 30 seconds"
            );
            assert_eq!(
                held_back(&batch.details[2].response.errors),
                Some((Some(10), false)),
                "and L-3, naming 10"
            );
            let sent = linear.served().len();
            assert_eq!(sent, 3, "the batch sent each of its reads");

            clock.advance(15);
            let later = engine.task(&id("L-1")).await.expect("an answer");
            assert_eq!(
                held_back(&later.errors),
                Some((Some(15), true)),
                "refused here, with the later reset's 15 seconds left"
            );
            assert_eq!(
                linear.served().len(),
                sent,
                "a call during the hold sends nothing"
            );

            engine
                .end_command()
                .await
                .expect("ending a command during the hold reaches the source");
            assert_eq!(linear.served().len(), sent, "and sends nothing either");

            clock.advance(15);
            let after = engine.task(&id("L-1")).await.expect("an answer");
            assert!(after.errors.is_empty(), "{:?}", after.errors);
            assert_eq!(after.items.len(), 1, "answered at the reset");
            assert_eq!(
                linear.served().len(),
                sent + 1,
                "the call at the reset reached Linear"
            );
        });
}

/// Through the same engine: a rate limit naming a wait of nothing holds nothing back, and one
/// naming a reset past what the clock can represent holds the source off — far past any time
/// the clock is moved to — without overflowing.
#[test]
fn a_reset_of_nothing_holds_nothing_and_an_oversized_one_holds_without_overflowing() {
    let (_sandbox, linear, engine, clock) = held_engine();
    let id: GlobalId = "linear:L-1".parse().unwrap();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            linear.rate_limit(&[Some(0)]);
            let refused = engine.task(&id).await.expect("an answer");
            assert_eq!(held_back(&refused.errors), Some((Some(0), false)));
            let answered = engine.task(&id).await.expect("an answer");
            assert_eq!(
                answered.items.len(),
                1,
                "a wait of nothing holds nothing back"
            );
            assert_eq!(linear.served().len(), 2);

            // An hour into the engine's life, so the reset lies past what the clock's
            // `Duration` can hold, not merely at its edge.
            clock.advance(3_600);
            linear.rate_limit(&[Some(u64::MAX)]);
            let refused = engine.task(&id).await.expect("an answer");
            assert_eq!(held_back(&refused.errors), Some((Some(u64::MAX), false)));
            clock.advance(10 * 365 * 24 * 3_600);
            let later = engine.task(&id).await.expect("an answer");
            let (left, unsent) = held_back(&later.errors).expect("still held off");
            assert!(unsent, "refused here");
            assert!(left.is_some_and(|left| left > 1_000_000_000), "{left:?}");
            assert_eq!(linear.served().len(), 3, "nothing sent ten years on");
        });
}
