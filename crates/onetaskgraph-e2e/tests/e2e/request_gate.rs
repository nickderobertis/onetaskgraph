//! A caller's request gate over a folder of Markdown and over a plugin served on the stdio
//! protocol, through the engine the binary links (`Engine::with_request_gate`).
//!
//! A folder asks the gate once before each record it writes, naming the write `local`, and is
//! told no reading after one; the engine asks once before each method it sends a stdio peer.
//! Each request admitted is told ended exactly once, and one refused never is. A
//! refusal part way through an `update-many` leaves exactly the earlier records written, answers
//! every later item with the refusal, and sends nothing after it. A GitHub board's and a Linear
//! source's gate are their own suites'.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use onetaskgraph_core::{CommentBatch, Engine, UpdateBatch};
use onetaskgraph_plugin_api::{GatedRequest, QuotaReading, RequestGate, SourceError, SourceName};
use serde_json::{Value, json};

use crate::common::Sandbox;
use crate::document_store::interpreter;
use crate::fixtures::document;

/// A gate recording what it is asked, refusing its `refuse_at`th ask when one is named, and
/// counting the requests it admitted that ended and the most it ever held in flight at once.
struct Recording {
    asked: Mutex<Vec<GatedRequest>>,
    told: Mutex<Vec<QuotaReading>>,
    /// Requests ended, requests in flight, and the most in flight at once.
    ended: Mutex<(usize, usize, usize)>,
    refuse_at: Option<usize>,
}

impl Recording {
    fn new(refuse_at: Option<usize>) -> Arc<Self> {
        Arc::new(Self {
            asked: Mutex::new(Vec::new()),
            told: Mutex::new(Vec::new()),
            ended: Mutex::new((0, 0, 0)),
            refuse_at,
        })
    }

    /// How many admitted requests ended, and the most that were in flight at once.
    fn ended(&self) -> (usize, usize) {
        let (ended, in_flight, most) = *self.ended.lock().unwrap();
        assert_eq!(in_flight, 0, "a request admitted never ended");
        (ended, most)
    }
}

#[async_trait::async_trait]
impl RequestGate for Recording {
    async fn before_request(&self, request: &GatedRequest) -> Result<(), SourceError> {
        let mut asked = self.asked.lock().unwrap();
        asked.push(request.clone());
        if self.refuse_at == Some(asked.len()) {
            return Err(SourceError::Refused {
                message: format!("the caller's gate refused request {}", asked.len()),
            });
        }
        let mut ended = self.ended.lock().unwrap();
        ended.1 += 1;
        ended.2 = ended.2.max(ended.1);
        Ok(())
    }

    fn after_response(&self, readings: &[QuotaReading]) {
        self.told.lock().unwrap().extend_from_slice(readings);
    }

    fn request_ended(&self) {
        let mut ended = self.ended.lock().unwrap();
        ended.0 += 1;
        ended.1 = ended
            .1
            .checked_sub(1)
            .expect("ended a request never admitted");
    }
}

/// A folder of Markdown holding five tasks at `todo`.
fn folder(sandbox: &Sandbox) -> PathBuf {
    let root = sandbox.subdirectory("plan");
    std::fs::create_dir_all(root.join("tasks")).expect("the tasks folder");
    for n in 1..=5 {
        std::fs::write(
            root.join(format!("tasks/T-{n}.md")),
            format!("---\ntitle: Task {n}\nstatus: todo\n---\nTask {n}, as filed.\n"),
        )
        .expect("a task");
    }
    root
}

/// The engine the binary would build from the sandbox's configuration.
fn engine(sandbox: &Sandbox) -> Engine {
    let environment = onetaskgraph_core::Environment::from_pairs([(
        "XDG_CONFIG_HOME",
        sandbox.config_home().to_string_lossy().into_owned(),
    )]);
    let loaded = onetaskgraph_core::config::load(
        &sandbox.project(),
        &environment,
        &onetaskgraph_core::config::Layer::default(),
    )
    .expect("the configuration loads");
    Engine::build(&loaded.config, &loaded.secrets)
}

/// An update of every task, each to `in-progress` with a body of its own.
fn updates(source: &str) -> UpdateBatch {
    let updates: Vec<Value> = (1..=5)
        .map(|n| {
            json!({"item": format!("{source}:T-{n}"), "status": "in-progress",
                        "body": format!("Task {n}, started.")})
        })
        .collect();
    serde_json::from_value(json!({"schema_version": 1, "updates": updates})).expect("a batch")
}

/// Each item's outcome, and whether its reason is the gate's refusal of request `n`.
fn answered(answer: &onetaskgraph_core::TasksUpdated, n: usize) -> Vec<(String, bool)> {
    serde_json::to_value(answer).unwrap()["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|result| {
            (
                result["outcome"].as_str().unwrap().to_owned(),
                result["reason"]
                    .as_str()
                    .is_some_and(|reason| reason.contains(&format!("refused request {n}"))),
            )
        })
        .collect()
}

/// Which of the folder's five tasks were written.
fn started(root: &Path) -> Vec<bool> {
    (1..=5)
        .map(|n| {
            std::fs::read_to_string(root.join(format!("tasks/T-{n}.md")))
                .unwrap()
                .contains("started")
        })
        .collect()
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

#[test]
fn a_folder_asks_the_gate_once_before_each_record_it_writes() {
    let sandbox = Sandbox::new();
    let root = folder(&sandbox);
    sandbox.project_document(&document(&json!({
        "plan": {"plugin": "local-md", "config": {"root": root}},
    })));
    let gate = Recording::new(None);
    let engine = engine(&sandbox).with_request_gate(gate.clone());
    let plan = SourceName::new("plan").unwrap();
    let answer = runtime()
        .block_on(engine.update_tasks(&plan, &updates("plan")))
        .expect("the batch runs");
    assert!(
        answered(&answer, 0)
            .iter()
            .all(|(outcome, _)| outcome == "written")
    );
    let asked = gate.asked.lock().unwrap().clone();
    assert_eq!(asked.len(), 5, "one ask per record written: {asked:?}");
    assert!(asked.iter().all(|request| request.resource == "local"));
    assert!(
        gate.told.lock().unwrap().is_empty(),
        "a folder reports no reading"
    );
    assert_eq!(started(&root), [true; 5]);
    assert_eq!(
        gate.ended(),
        (5, 1),
        "each record write ended once, one at a time"
    );
}

#[test]
fn a_refusal_in_a_folder_leaves_exactly_the_earlier_records_written() {
    let sandbox = Sandbox::new();
    let root = folder(&sandbox);
    sandbox.project_document(&document(&json!({
        "plan": {"plugin": "local-md", "config": {"root": root}},
    })));
    let gate = Recording::new(Some(3));
    let engine = engine(&sandbox).with_request_gate(gate.clone());
    let plan = SourceName::new("plan").unwrap();
    let answer = runtime()
        .block_on(engine.update_tasks(&plan, &updates("plan")))
        .expect("the batch answers each item");
    let answered = answered(&answer, 3);
    assert_eq!(answered[0].0, "written");
    assert_eq!(answered[1].0, "written");
    for (outcome, refused) in &answered[2..] {
        assert_ne!(outcome, "written");
        assert!(refused, "{answered:?}");
    }
    assert_eq!(started(&root), [true, true, false, false, false]);
    assert_eq!(
        gate.asked.lock().unwrap().len(),
        3,
        "nothing is asked after the refusal"
    );
    assert_eq!(gate.ended().0, 2, "the refused write is never ended");
}

/// A peer serving the folder at `root` over the stdio protocol, every method it is sent
/// logged to `log`.
pub(crate) fn hosted(root: &Path, log: &Path) -> Value {
    let proxy = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/e2e/logging_peer.py");
    json!({"plugin": "subprocess", "config": {
        "command": interpreter().to_string_lossy(),
        "args": [proxy.to_string_lossy(), log.to_string_lossy(),
                 onetaskgraph_e2e_support::binary().to_string_lossy(), "plugin-serve", "local-md"],
        "settings": {"root": root},
    }})
}

/// Every method the peer was sent after its handshake, which is sent as the engine is built.
fn sent(log: &Path) -> Vec<String> {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .filter(|method| *method != "initialize")
        .map(str::to_owned)
        .collect()
}

#[test]
fn the_engine_asks_once_before_each_method_it_sends_a_stdio_peer_and_stops_at_a_refusal() {
    for refuse_at in [None, Some(3)] {
        let sandbox = Sandbox::new();
        let root = folder(&sandbox);
        let log = sandbox.subdirectory("peer").join("sent.log");
        sandbox.project_document(&document(&json!({ "peer": hosted(&root, &log) })));
        let gate = Recording::new(refuse_at);
        let engine = engine(&sandbox).with_request_gate(gate.clone());
        let peer = SourceName::new("peer").unwrap();
        let answer = runtime()
            .block_on(engine.update_tasks(&peer, &updates("peer")))
            .expect("the batch answers each item");
        let asked = gate.asked.lock().unwrap().clone();
        let methods = sent(&log);
        match refuse_at {
            None => {
                assert!(
                    answered(&answer, 0)
                        .iter()
                        .all(|(outcome, _)| outcome == "written")
                );
                // One ask per method, and never one without the method sent after it.
                assert_eq!(asked.len(), methods.len(), "{methods:?}");
                assert!(asked.iter().all(|request| request.resource == "stdio"));
                assert_eq!(started(&root), [true; 5]);
                assert_eq!(gate.ended(), (asked.len(), 1), "each method ended once");
            }
            Some(n) => {
                // Every method asked for before the refusal was sent, and none after it.
                assert_eq!(asked.len(), n);
                assert_eq!(methods.len(), n - 1, "{methods:?}");
                assert_eq!(gate.ended().0, n - 1, "the refused method is never ended");
                let answered = answered(&answer, n);
                let written = answered
                    .iter()
                    .filter(|(outcome, _)| outcome == "written")
                    .count();
                assert!(
                    answered[written..].iter().all(|(_, refused)| *refused),
                    "{answered:?}"
                );
                assert_eq!(
                    started(&root),
                    (1..=5).map(|at| at <= written).collect::<Vec<_>>()
                );
            }
        }
    }
}

/// A comment added to each of the first two tasks.
fn comments(source: &str) -> CommentBatch {
    serde_json::from_value(json!({"schema_version": 1, "comments": [
        {"item": format!("{source}:T-1"), "body": "first note"},
        {"item": format!("{source}:T-2"), "body": "second note"},
    ]}))
    .expect("a batch")
}

#[test]
fn a_comment_add_is_asked_as_content_creating_and_an_update_is_not() {
    for hosted_peer in [false, true] {
        let sandbox = Sandbox::new();
        let root = folder(&sandbox);
        let log = sandbox.subdirectory("peer").join("sent.log");
        let (name, config) = if hosted_peer {
            ("peer", hosted(&root, &log))
        } else {
            (
                "plan",
                json!({"plugin": "local-md", "config": {"root": root}}),
            )
        };
        sandbox.project_document(&document(&json!({ name: config })));
        let gate = Recording::new(None);
        let engine = engine(&sandbox).with_request_gate(gate.clone());
        let source = SourceName::new(name).unwrap();
        let runtime = runtime();
        runtime
            .block_on(engine.update_tasks(&source, &updates(name)))
            .expect("the batch runs");
        let updating = gate.asked.lock().unwrap().len();
        runtime
            .block_on(engine.add_comments(&source, &comments(name)))
            .expect("the comments are added");
        let asked = gate.asked.lock().unwrap().clone();
        assert!(
            asked[..updating]
                .iter()
                .all(|request| !request.content_creating),
            "{name}: an update creates nothing: {asked:?}"
        );
        let adding = &asked[updating..];
        // A folder asks once per comment it writes; a peer once per method, `add_comment` among
        // them, and exactly those are content-creating.
        let created = adding
            .iter()
            .filter(|request| request.content_creating)
            .count();
        assert_eq!(created, 2, "{name}: {adding:?}");
        if hosted_peer {
            let methods: Vec<String> = sent(&log).into_iter().skip(updating).collect();
            for (request, method) in adding.iter().zip(&methods) {
                assert_eq!(
                    request.content_creating,
                    method == "add_comment",
                    "{method}"
                );
            }
        } else {
            assert!(adding.iter().all(|request| request.content_creating));
        }
        assert!(adding.iter().all(|request| request.cost_hint.is_none()));
        assert_eq!(
            gate.ended(),
            (asked.len(), 1),
            "{name}: each request ended once"
        );
        for n in 1..=2 {
            let file = std::fs::read_to_string(root.join(format!("tasks/T-{n}.md"))).unwrap();
            assert!(file.contains("note"), "{name}: T-{n}: {file}");
        }
    }
}
