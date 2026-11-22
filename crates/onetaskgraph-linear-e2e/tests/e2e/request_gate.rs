//! A caller's request gate over a loopback Linear, through the engine the binary links
//! (`Engine::with_request_gate`): asked before every request the workspace receives, told each
//! response's own rate-limit headers, and — refusing part way through an `update-many` — leaving
//! every earlier item written, every later one answered with the refusal, and nothing sent after
//! it. Each request admitted is told ended exactly once, and the one refused never is.

use std::sync::{Arc, Mutex};

use onetaskgraph_core::{Engine, UpdateBatch};
use onetaskgraph_plugin_api::{GatedRequest, QuotaReading, RequestGate, SourceError, SourceName};
use serde_json::{Value, json};

use crate::cache::dataset;
use crate::common::Sandbox;
use crate::fixtures::{LinearWorkspace, document, linear_workspace};

/// What [`RequestGate::before_request`] answers, as `#[async_trait]` spells it.
pub(crate) type GateFuture<'future> =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), SourceError>> + Send + 'future>>;

/// A gate recording each ask with how many requests the workspace had received, refusing its
/// `refuse_at`th ask when one is named.
struct Recording {
    workspace: LinearWorkspace,
    asked: Mutex<Vec<(GatedRequest, usize)>>,
    told: Mutex<Vec<QuotaReading>>,
    ended: Mutex<usize>,
    refuse_at: Option<usize>,
}

// Spelled as `#[async_trait]` expands it, so this crate needs no dependency of its own for the
// macro: a new `Cargo.lock` entry is a global selection input that runs every project's tests.
impl RequestGate for Recording {
    fn before_request<'gate, 'request, 'future>(
        &'gate self,
        request: &'request GatedRequest,
    ) -> GateFuture<'future>
    where
        'gate: 'future,
        'request: 'future,
        Self: 'future,
    {
        let mut asked = self.asked.lock().unwrap();
        asked.push((request.clone(), self.workspace.served_count()));
        let answer = if self.refuse_at == Some(asked.len()) {
            Err(SourceError::Refused {
                message: format!("the caller's gate refused request {}", asked.len()),
            })
        } else {
            Ok(())
        };
        Box::pin(std::future::ready(answer))
    }

    fn after_response(&self, readings: &[QuotaReading]) {
        self.told.lock().unwrap().extend_from_slice(readings);
    }

    fn request_ended(&self) {
        *self.ended.lock().unwrap() += 1;
    }
}

/// The `x-ratelimit-requests-*` headers of the `n`th answer: one less left each time.
fn headers(n: u64) -> Vec<(String, String)> {
    vec![
        ("x-ratelimit-requests-limit".into(), "1500".into()),
        (
            "x-ratelimit-requests-remaining".into(),
            (1500 - n).to_string(),
        ),
        ("x-ratelimit-requests-reset".into(), "1795306927000".into()),
    ]
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

    fn engine(&self, gate: Arc<Recording>) -> Engine {
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
        Engine::build(&loaded.config, &loaded.secrets).with_request_gate(gate)
    }

    fn gate(&self, refuse_at: Option<usize>) -> Arc<Recording> {
        Arc::new(Recording {
            workspace: self.workspace.clone(),
            asked: Mutex::new(Vec::new()),
            told: Mutex::new(Vec::new()),
            ended: Mutex::new(0),
            refuse_at,
        })
    }
}

/// Two updates of each of two issues, each a new title: the first of each go in one aliased
/// write, and the second of each — an update of an issue the batch already names — are each
/// sent on their own after it, so the batch's writes span several requests.
fn updates() -> UpdateBatch {
    let updates: Vec<Value> = [
        (1, "groomed"),
        (2, "groomed"),
        (1, "regroomed"),
        (2, "regroomed"),
    ]
    .into_iter()
    .map(|(n, how)| json!({"item": format!("linear:L-{n}"), "title": format!("Issue {n}, {how}")}))
    .collect();
    serde_json::from_value(json!({"schema_version": 1, "updates": updates})).expect("a batch")
}

fn run(engine: &Engine) -> Value {
    let answer = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(engine.update_tasks(&SourceName::new("linear").unwrap(), &updates()))
        .expect("the batch answers each item");
    serde_json::to_value(answer).unwrap()
}

#[test]
fn the_gate_is_asked_before_every_request_and_told_each_response_s_headers() {
    let setup = Setup::new();
    setup
        .workspace
        .answer_with_headers((1..=50).map(headers).collect());
    let gate = setup.gate(None);
    let answer = run(&setup.engine(gate.clone()));
    for result in answer["results"].as_array().unwrap() {
        assert_eq!(result["outcome"], "written", "{answer:#}");
    }
    let asked = gate.asked.lock().unwrap().clone();
    let served = setup.workspace.served_count();
    assert_eq!(asked.len(), served, "one ask per request");
    for (at, (request, received)) in asked.iter().enumerate() {
        assert_eq!(*received, at, "ask {at} came after request {received}");
        assert_eq!(request.resource, "requests");
        assert!(!request.content_creating, "an update creates nothing");
    }
    // Each response's own reading, in order, as its headers carried it.
    let told = gate.told.lock().unwrap().clone();
    assert_eq!(
        told.iter()
            .map(|reading| reading.remaining)
            .collect::<Vec<_>>(),
        (1..=u64::try_from(served).unwrap())
            .map(|n| 1500 - n)
            .collect::<Vec<_>>()
    );
    assert!(told.iter().all(|reading| reading.resource == "requests"));
    assert_eq!(*gate.ended.lock().unwrap(), served, "one end per request");
}

#[test]
fn a_refusal_part_way_answers_every_later_item_with_it_and_sends_nothing_more() {
    // How many requests the batch sends, and from which request on its writes go.
    let measured = Setup::new();
    let gate = measured.gate(None);
    run(&measured.engine(gate.clone()));
    let total = measured.workspace.served_count();
    let first_write = measured
        .workspace
        .served()
        .iter()
        .position(|(query, _)| query.trim_start().starts_with("mutation"))
        .expect("the batch writes");
    // Refused at the request after the first write: one item has landed, the rest have not.
    let refuse_at = first_write + 2;
    assert!(refuse_at <= total, "{first_write} of {total}");

    let setup = Setup::new();
    let gate = setup.gate(Some(refuse_at));
    let answer = run(&setup.engine(gate.clone()));
    assert_eq!(
        setup.workspace.served_count(),
        refuse_at - 1,
        "nothing after it"
    );
    assert_eq!(gate.asked.lock().unwrap().len(), refuse_at);
    assert_eq!(
        *gate.ended.lock().unwrap(),
        refuse_at - 1,
        "the refused request is never ended"
    );
    let results = answer["results"].as_array().unwrap();
    let written = results
        .iter()
        .take_while(|result| result["outcome"] == "written")
        .count();
    assert!(written >= 1, "{answer:#}");
    for result in &results[written..] {
        assert_ne!(result["outcome"], "written", "{answer:#}");
        assert!(
            result["reason"]
                .as_str()
                .is_some_and(|reason| reason.contains(&format!("refused request {refuse_at}"))),
            "{answer:#}"
        );
    }
}
