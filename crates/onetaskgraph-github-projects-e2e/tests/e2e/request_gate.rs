//! A caller's request gate over a GitHub board, through the engine the binary links: installed
//! with `Engine::with_request_gate`, asked before every request the board receives — reads,
//! writes, a retry as its own request and `Engine::quota`'s `GET /rate_limit` — and told every
//! response's own rate-limit reading, never merged with another window's — and told each
//! request it admitted ended exactly once, whether it was answered with readings or without,
//! failed in transport before any header, or was dropped in flight, and never one it refused.
//!
//! A wait holds that source's next request and nothing else; a refusal makes the source send
//! nothing more in that call, so every earlier item of a batch answers written, every later one
//! answers the refusal with nothing sent for it, and a second call naming only those lands them
//! without writing any item twice.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use onetaskgraph_core::{CommentBatch, Engine, GlobalId, UpdateBatch};
use onetaskgraph_plugin_api::{
    GatedRequest, NativeId, QuotaReading, RequestGate, SharedClock, SourceError, SourceName,
};
use serde_json::{Value, json};

use crate::common::Sandbox;
use crate::fixtures::{GitHubBoardFields, document, github_projects_with_items};

/// One task issue on the board, open at `Todo`, filed under no project.
fn issue(id: &str) -> Value {
    json!({"item":format!("ITEM-{id}"),"id":id,"type":"Issue","title":format!("Ticket {id}"),
           "body":format!("{id}, as filed"),"state":"OPEN","reason":null,"parent":null,
           "repo":"nickderobertis/onetaskgraph","status":"Todo","origin":"","labels":[]})
}

fn ticket(n: usize) -> String {
    format!("G-{n}")
}

/// The two interleaved windows of the one `graphql` resource the spike recorded answering one
/// token, each with its own reset.
fn windows() -> Vec<Value> {
    vec![
        json!({"resource":"graphql","limit":5000,"remaining":4589,"used":411,
               "reset":1_795_306_927}),
        json!({"resource":"graphql","limit":5000,"remaining":4841,"used":159,
               "reset":1_795_309_716}),
    ]
}

/// What the gate does at one ask, by its 1-based number.
#[derive(Clone)]
enum At {
    Refuse(usize),
    /// Sleep this long on the engine's clock before answering.
    Wait(usize, Duration, SharedClock),
}

/// A gate recording every ask — with how many requests the board had received when it was
/// asked — and every reading it is told.
struct Recording {
    board: GitHubBoardFields,
    asked: Mutex<Vec<(GatedRequest, usize)>>,
    told: Mutex<Vec<QuotaReading>>,
    ended: Mutex<usize>,
    at: Option<At>,
}

impl Recording {
    fn new(board: &GitHubBoardFields, at: Option<At>) -> Arc<Self> {
        Arc::new(Self {
            board: board.clone(),
            asked: Mutex::new(Vec::new()),
            told: Mutex::new(Vec::new()),
            ended: Mutex::new(0),
            at,
        })
    }

    fn asked(&self) -> Vec<(GatedRequest, usize)> {
        self.asked.lock().unwrap().clone()
    }

    fn ended(&self) -> usize {
        *self.ended.lock().unwrap()
    }
}

fn refusal(n: usize) -> SourceError {
    SourceError::Refused {
        message: format!("the caller's gate refused request {n}"),
    }
}

#[async_trait::async_trait]
impl RequestGate for Recording {
    async fn before_request(&self, request: &GatedRequest) -> Result<(), SourceError> {
        let n = {
            let mut asked = self.asked.lock().unwrap();
            asked.push((request.clone(), self.board.served_count()));
            asked.len()
        };
        match &self.at {
            Some(At::Refuse(at)) if n == *at => Err(refusal(n)),
            Some(At::Wait(at, length, clock)) if n == *at => {
                clock
                    .sleep_because(
                        *length,
                        onetaskgraph_plugin_api::WaitCause::new("caller-gate"),
                    )
                    .await;
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn after_response(&self, readings: &[QuotaReading]) {
        self.told.lock().unwrap().extend_from_slice(readings);
    }

    fn request_ended(&self) {
        *self.ended.lock().unwrap() += 1;
    }
}

struct Setup {
    sandbox: Sandbox,
    board: GitHubBoardFields,
}

impl Setup {
    /// A board holding `tickets` tickets, every answer carrying one of [`windows`] in turn.
    fn new(tickets: usize) -> Self {
        let setup = Self::without_readings(tickets);
        setup.board.answer_from_windows(windows());
        setup
    }

    /// A board holding `tickets` tickets whose answers carry no rate-limit header at all.
    fn without_readings(tickets: usize) -> Self {
        let sandbox = Sandbox::new();
        let (config, board) =
            github_projects_with_items(&sandbox, (0..tickets).map(|n| issue(&ticket(n))).collect());
        sandbox.project_document(&document(&json!({
            "board": {"plugin":"github-projects","config":config},
        })));
        Self { sandbox, board }
    }

    /// The same, its source sent to `endpoint` instead of the board.
    fn at_endpoint(tickets: usize, endpoint: &str) -> Self {
        let sandbox = Sandbox::new();
        let (mut config, board) =
            github_projects_with_items(&sandbox, (0..tickets).map(|n| issue(&ticket(n))).collect());
        config["endpoint"] = json!(endpoint);
        sandbox.project_document(&document(&json!({
            "board": {"plugin":"github-projects","config":config},
        })));
        Self { sandbox, board }
    }

    fn engine(&self) -> Engine {
        let environment = onetaskgraph_core::Environment::from_pairs([(
            "XDG_CONFIG_HOME",
            self.sandbox.config_home().to_string_lossy().into_owned(),
        )]);
        let loaded = onetaskgraph_core::config::load(
            &self.sandbox.project(),
            &environment,
            &onetaskgraph_core::config::Layer::default(),
        )
        .expect("the configuration loads");
        Engine::build(&loaded.config, &loaded.secrets)
    }
}

fn board() -> SourceName {
    SourceName::new("board").expect("a name")
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime")
}

/// One update per ticket in `tickets`: a status, a priority and a metadata key, each a write.
fn updates(tickets: impl Iterator<Item = usize>) -> UpdateBatch {
    let updates: Vec<Value> = tickets
        .map(|n| {
            json!({"item": format!("board:{}", ticket(n)), "body": format!("{} triaged", ticket(n)),
                   "status": "in-progress",
                   "metadata": {"orchestrator.triage": {"pass": 1}}})
        })
        .collect();
    serde_json::from_value(json!({"schema_version": 1, "updates": updates})).expect("a batch")
}

/// Every outcome of an update batch, in order.
fn outcomes(answer: &onetaskgraph_core::TasksUpdated) -> Vec<String> {
    serde_json::to_value(answer).unwrap()["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|result| result["outcome"].as_str().unwrap().to_owned())
        .collect()
}

/// Every document the board received from `from` on that writes `ticket`.
fn writes_of(board: &GitHubBoardFields, from: usize, ticket: &str) -> usize {
    board.served()[from..]
        .iter()
        .filter(|(query, variables)| {
            query.trim_start().starts_with("mutation")
                && variables.to_string().contains(&format!("\"{ticket}\""))
        })
        .count()
}

#[test]
fn the_gate_is_asked_before_every_request_of_an_update_many_and_a_comment_add_many() {
    let setup = Setup::new(100);
    let gate = Recording::new(&setup.board, None);
    let engine = setup.engine().with_request_gate(gate.clone());
    let runtime = runtime();
    let answer = runtime
        .block_on(engine.update_tasks(&board(), &updates(0..100)))
        .expect("the batch runs");
    assert!(outcomes(&answer).iter().all(|outcome| outcome == "written"));
    let added = runtime
        .block_on(engine.add_comments(&board(), &comments()))
        .expect("the comments are added");
    assert_eq!(
        serde_json::to_value(&added).unwrap()["results"]
            .as_array()
            .unwrap()
            .len(),
        60
    );

    // Asked once before each request the board received, in the order it received them.
    let asked = gate.asked();
    let served = setup.board.served();
    assert_eq!(asked.len(), served.len(), "one ask per request");
    for (at, (_, received)) in asked.iter().enumerate() {
        assert_eq!(*received, at, "ask {at} came after request {received}");
    }
    // Content-creating exactly for the comment creations, and every one names its resource.
    for ((request, _), (query, _)) in asked.iter().zip(&served) {
        assert_eq!(request.resource, "graphql");
        assert_eq!(
            request.content_creating,
            query.contains("addComment("),
            "{query}"
        );
    }
    assert!(asked.iter().any(|(request, _)| request.content_creating));
    // And each is priced as GitHub's published rules price the request the board received, at
    // the page sizes it bound.
    for ((request, _), (query, variables)) in asked.iter().zip(&served) {
        let mut bound = onetaskgraph_github_projects::largest_page_sizes();
        for (name, value) in variables.as_object().into_iter().flatten() {
            if let Some(size) = value.as_u64().and_then(|size| u32::try_from(size).ok()) {
                bound.insert(name.clone(), size);
            }
        }
        let priced = onetaskgraph_github_projects::point_cost(query, &bound).expect("priced");
        assert_eq!(request.cost_hint, Some(priced), "{query}");
    }
    // Every response's own reading, each window with its own reset, never merged.
    let told = gate.told.lock().unwrap().clone();
    assert_eq!(told.len(), served.len(), "one reading per response");
    let resets: std::collections::BTreeSet<i64> = told
        .iter()
        .map(|reading| reading.resets_at.timestamp())
        .collect();
    assert_eq!(
        resets,
        std::collections::BTreeSet::from([1_795_306_927, 1_795_309_716])
    );
    for (at, reading) in told.iter().enumerate() {
        let window = &windows()[at % 2];
        assert_eq!(reading.remaining, window["remaining"].as_u64().unwrap());
        assert_eq!(reading.used, window["used"].as_u64().unwrap());
        assert_eq!(
            reading.resets_at.timestamp(),
            window["reset"].as_i64().unwrap()
        );
    }
    eprintln!(
        "an update-many of 100 and a comment add-many of 60 sent {} requests",
        served.len()
    );
}

#[test]
fn with_no_gate_installed_the_board_receives_the_same_requests() {
    let gated = Setup::new(100);
    let gate = Recording::new(&gated.board, None);
    let runtime = runtime();
    runtime
        .block_on(
            gated
                .engine()
                .with_request_gate(gate)
                .update_tasks(&board(), &updates(0..100)),
        )
        .expect("the batch runs");
    let plain = Setup::new(100);
    runtime
        .block_on(plain.engine().update_tasks(&board(), &updates(0..100)))
        .expect("the batch runs");
    assert_eq!(gated.board.served(), plain.board.served());
}

/// The 60 comments the comment batch adds, round the board's tickets.
fn comments() -> CommentBatch {
    let comments: Vec<Value> = (0..60)
        .map(|n| json!({"item": format!("board:{}", ticket(n % 100)), "body": format!("note {n}")}))
        .collect();
    serde_json::from_value(json!({"schema_version": 1, "comments": comments})).unwrap()
}

/// Every item of a batch's answer that was written, and every one answered with the gate's
/// refusal of request `n`, by position.
fn split(answer: &Value, n: usize) -> (Vec<usize>, Vec<usize>, Vec<usize>) {
    let results = answer["results"].as_array().unwrap();
    let written: Vec<usize> = (0..results.len())
        .filter(|at| results[*at]["outcome"] == "written")
        .collect();
    let refused: Vec<usize> = (0..results.len())
        .filter(|at| {
            results[*at]["reason"].as_str().is_some_and(|reason| {
                reason.contains(&format!("the caller's gate refused request {n}"))
            })
        })
        .collect();
    let other: Vec<usize> = (0..results.len())
        .filter(|at| !written.contains(at) && !refused.contains(at))
        .collect();
    (written, refused, other)
}

#[test]
fn a_refusal_at_the_thirtieth_request_stops_the_call_and_a_second_call_lands_the_rest() {
    // The update-many of 100 and then the comment add-many of 60, as one caller sends them
    // through one engine, the 30th request of the two refused: the update-many answering every
    // item written and the comments answering some refused is what places it inside the second.
    let runtime = runtime();
    let setup = Setup::new(100);
    let gate = Recording::new(&setup.board, Some(At::Refuse(30)));
    let engine = setup.engine().with_request_gate(gate.clone());
    let updated = runtime
        .block_on(engine.update_tasks(&board(), &updates(0..100)))
        .expect("the batch runs");
    assert!(
        outcomes(&updated)
            .iter()
            .all(|outcome| outcome == "written")
    );
    let added = runtime
        .block_on(engine.add_comments(&board(), &comments()))
        .expect("the comment batch answers each item");
    // Nothing reaches the board after the refusal: the 30th request was never sent.
    assert_eq!(setup.board.served_count(), 29);
    assert_eq!(gate.asked().len(), 30, "the gate is asked nothing more");
    let (written, refused, other) = split(&serde_json::to_value(&added).unwrap(), 30);
    assert_eq!(other, Vec::<usize>::new(), "{added:#?}");
    assert!(!refused.is_empty() && written.iter().all(|at| refused.iter().all(|r| at < r)));
    // Every comment answered written is on its ticket once, and no refused one is anywhere.
    let landed = |n: usize| {
        setup
            .board
            .comments_on(&ticket(n % 100))
            .iter()
            .filter(|comment| comment["body"] == format!("note {n}"))
            .count()
    };
    for n in &written {
        assert_eq!(landed(*n), 1, "note {n}");
    }
    for n in &refused {
        assert_eq!(landed(*n), 0, "note {n}");
    }
    // A second call naming only the comments not answered written adds each once.
    let again: Vec<Value> = refused
        .iter()
        .map(|n| json!({"item": format!("board:{}", ticket(n % 100)), "body": format!("note {n}")}))
        .collect();
    let again: CommentBatch =
        serde_json::from_value(json!({"schema_version": 1, "comments": again})).unwrap();
    runtime
        .block_on(setup.engine().add_comments(&board(), &again))
        .expect("the rest are added");
    for n in 0..60 {
        assert_eq!(landed(n), 1, "note {n}");
    }
    eprintln!(
        "refused at request 30: {} comments written, {} refused",
        written.len(),
        refused.len()
    );
}

#[test]
fn a_refusal_part_way_through_an_update_many_answers_every_later_item_refused() {
    let runtime = runtime();
    let setup = Setup::new(100);
    let gate = Recording::new(&setup.board, Some(At::Refuse(12)));
    let engine = setup.engine().with_request_gate(gate.clone());
    let answer = runtime
        .block_on(engine.update_tasks(&board(), &updates(0..100)))
        .expect("the batch answers each item");
    assert_eq!(
        setup.board.served_count(),
        11,
        "nothing is sent after the refusal"
    );
    assert_eq!(gate.asked().len(), 12);
    let (written, refused, other) = split(&serde_json::to_value(&answer).unwrap(), 12);
    eprintln!(
        "refused at request 12: {} written, {} refused, {} otherwise: {:?}",
        written.len(),
        refused.len(),
        other.len(),
        other
    );
    assert!(!refused.is_empty());
    // A second call naming only the items not answered written lands them, and no item already
    // written is written again.
    let from = setup.board.served_count();
    let rest: Vec<usize> = (0..100).filter(|at| !written.contains(at)).collect();
    let again = runtime
        .block_on(
            setup
                .engine()
                .update_tasks(&board(), &updates(rest.iter().copied())),
        )
        .expect("the second call runs");
    assert!(outcomes(&again).iter().all(|outcome| outcome == "written"));
    for at in &written {
        assert_eq!(writes_of(&setup.board, from, &ticket(*at)), 0, "{at}");
    }
    // Every ticket ends at the one card `in-progress` names, none left at `Todo`.
    let cards: std::collections::BTreeSet<Option<String>> =
        (0..100).map(|at| setup.board.status(&ticket(at))).collect();
    assert_eq!(cards.len(), 1, "{cards:?}");
    assert_ne!(cards.into_iter().next().flatten().as_deref(), Some("Todo"));
}

#[test]
fn a_read_a_retry_and_a_quota_read_are_each_asked_and_a_refusal_of_each_sends_nothing() {
    let setup = Setup::new(1);
    let id = GlobalId {
        source: board(),
        native: NativeId(ticket(0)),
    };
    let runtime = runtime();
    // The first request answered 429, so the read is sent again: two asks, two requests.
    setup.board.rate_limit_next(1);
    let gate = Recording::new(&setup.board, None);
    let engine = setup.engine().with_request_gate(gate.clone());
    runtime
        .block_on(engine.task(&id))
        .expect("the task is read");
    runtime.block_on(engine.quota(&board())).expect("a reading");
    let documents = setup.board.documents();
    assert_eq!(gate.asked().len(), documents.len());
    assert_eq!(
        documents.last().map(String::as_str),
        Some("GET /rate_limit")
    );
    assert_eq!(gate.asked().last().unwrap().0.resource, "core");
    assert!(
        documents.len() >= 3,
        "a read, its retry and the quota read: {documents:?}"
    );
    assert_eq!(
        documents[0], documents[1],
        "the retry is the same request again"
    );

    // Refused at each in turn: the board receives nothing for it, and the call answers it.
    for (what, refuse_at) in [("the read", 1), ("the retry", 2)] {
        let setup = Setup::new(1);
        setup.board.rate_limit_next(1);
        let gate = Recording::new(&setup.board, Some(At::Refuse(refuse_at)));
        let engine = setup.engine().with_request_gate(gate.clone());
        let answer = runtime.block_on(engine.task(&id));
        let said = match answer {
            Ok(response) => serde_json::to_value(&response).unwrap()["errors"].to_string(),
            Err(error) => error.to_string(),
        };
        assert!(
            said.contains(&format!("refused request {refuse_at}")),
            "{what}: {said}"
        );
        assert_eq!(setup.board.served_count(), refuse_at - 1, "{what}");
    }
    let setup = Setup::new(1);
    let gate = Recording::new(&setup.board, Some(At::Refuse(1)));
    let engine = setup.engine().with_request_gate(gate);
    let refused = runtime
        .block_on(engine.quota(&board()))
        .expect_err("the quota read is refused");
    assert!(
        refused.to_string().contains("refused request 1"),
        "{refused}"
    );
    assert!(setup.board.documents().is_empty(), "nothing was sent");
}

#[test]
fn a_wait_holds_its_own_source_s_request_and_no_other_source_s() {
    // Two boards, one source each, on one engine.
    let sandbox = Sandbox::new();
    let (first, first_board) = github_projects_with_items(&sandbox, vec![issue("A-1")]);
    let (second, second_board) = github_projects_with_items(&sandbox, vec![issue("B-1")]);
    sandbox.project_document(&document(&json!({
        "first": {"plugin":"github-projects","config":first},
        "second": {"plugin":"github-projects","config":second},
    })));
    let setup = Setup {
        sandbox,
        board: first_board.clone(),
    };
    /// Holds the first request it is asked about until it is released.
    struct Holding {
        held: tokio::sync::Notify,
        released: tokio::sync::Notify,
        asks: Mutex<usize>,
    }
    #[async_trait::async_trait]
    impl RequestGate for Holding {
        async fn before_request(&self, _request: &GatedRequest) -> Result<(), SourceError> {
            let first = {
                let mut asks = self.asks.lock().unwrap();
                *asks += 1;
                *asks == 1
            };
            if first {
                self.held.notify_one();
                self.released.notified().await;
            }
            Ok(())
        }
        fn after_response(&self, _readings: &[QuotaReading]) {}
    }
    let gate = Arc::new(Holding {
        held: tokio::sync::Notify::new(),
        released: tokio::sync::Notify::new(),
        asks: Mutex::new(0),
    });
    let engine = setup.engine().with_request_gate(gate.clone());
    let a = GlobalId {
        source: SourceName::new("first").unwrap(),
        native: NativeId("A-1".into()),
    };
    let b = GlobalId {
        source: SourceName::new("second").unwrap(),
        native: NativeId("B-1".into()),
    };
    let runtime = runtime();
    let (first, second_served_while_held) = runtime.block_on(async {
        tokio::join!(engine.task(&a), async {
            gate.held.notified().await;
            // The first source's gate is holding: the second source's request goes through.
            let read = engine.task(&b).await.expect("the second source answers");
            assert_eq!(read.items.len(), 1);
            let served = (first_board.served_count(), second_board.served_count());
            gate.released.notify_one();
            served
        })
    });
    first.expect("the first source answers once released");
    assert_eq!(
        second_served_while_held.0, 0,
        "the first board received nothing while its gate held"
    );
    assert!(second_served_while_held.1 >= 1, "the second board answered");
    assert!(
        first_board.served_count() >= 1,
        "and the first, once released"
    );
}

#[test]
fn a_wait_at_the_thirtieth_request_holds_it_on_the_simulated_clock() {
    use super::asset_board::Board as Timed;
    use onetaskgraph_e2e_support::clock::SimulatedClock;
    let clock = SimulatedClock::start(1);
    let sandbox = Sandbox::new();
    let (mut config, github) =
        github_projects_with_items(&sandbox, (0..100).map(|n| issue(&ticket(n))).collect());
    let timed = Timed::new(config["endpoint"].as_str().unwrap(), &clock, 0, None);
    config["endpoint"] = json!(timed.endpoint);
    sandbox.project_document(&document(&json!({
        "board": {"plugin":"github-projects","config":config},
    })));
    let setup = Setup {
        sandbox,
        board: github,
    };
    let environment = onetaskgraph_core::Environment::from_pairs(clock.client_env(0));
    let attached = onetaskgraph_core::process_clock(&environment).expect("the clock attaches");
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
    let wait = Duration::from_secs(600);
    let gate = Recording::new(&setup.board, Some(At::Wait(30, wait, attached.clone())));
    let engine = Engine::build_with_clock(&loaded.config, &loaded.secrets, &attached)
        .with_request_gate(gate.clone());
    let runtime = runtime();
    let answer = runtime
        .block_on(engine.update_tasks(&board(), &updates(0..100)))
        .expect("the batch runs");
    assert!(outcomes(&answer).iter().all(|outcome| outcome == "written"));
    runtime
        .block_on(engine.add_comments(&board(), &comments()))
        .expect("the comments are added");
    let calls = timed.calls();
    assert!(calls.len() >= 30, "{}", calls.len());
    // The board received nothing between the 29th request's answer and the end of the wait:
    // the 30th arrived the whole wait after the 29th was answered.
    assert!(
        calls[29].at >= calls[28].answered + wait,
        "the 30th request arrived at {:?}, the 29th answered at {:?}",
        calls[29].at,
        calls[28].answered
    );
    assert!(calls[28].at < calls[28].answered + wait);
}

#[test]
fn a_private_board_s_repository_visibility_read_is_asked_and_its_refusal_sends_it_not() {
    for refusing in [false, true] {
        let sandbox = Sandbox::new();
        let (config, github) = github_projects_with_items(&sandbox, vec![issue(&ticket(1))]);
        // Declared private, the board reads each destination's visibility before a write: the
        // repository's over REST.
        sandbox.project_document(&document(&json!({
            "board": {"plugin":"github-projects","config":config,"visibility":"private"},
        })));
        let setup = Setup {
            sandbox,
            board: github,
        };
        struct Visibility {
            asked: Mutex<Vec<GatedRequest>>,
            refusing: bool,
        }
        #[async_trait::async_trait]
        impl RequestGate for Visibility {
            async fn before_request(&self, request: &GatedRequest) -> Result<(), SourceError> {
                self.asked.lock().unwrap().push(request.clone());
                if self.refusing && request.resource == "core" {
                    return Err(SourceError::Refused {
                        message: "the caller's gate refused the visibility read".into(),
                    });
                }
                Ok(())
            }
            fn after_response(&self, _readings: &[QuotaReading]) {}
        }
        let gate = Arc::new(Visibility {
            asked: Mutex::new(Vec::new()),
            refusing,
        });
        let engine = setup.engine().with_request_gate(gate.clone());
        let id = GlobalId {
            source: board(),
            native: NativeId(ticket(1)),
        };
        let written = runtime().block_on(
            engine.set_task_status(&id, onetaskgraph_plugin_api::StatusCategory::InProgress),
        );
        let asked = gate.asked.lock().unwrap().clone();
        assert!(
            asked
                .iter()
                .any(|request| request.resource == "core" && !request.content_creating),
            "the repository read was asked about: {asked:?}"
        );
        let reads = setup
            .board
            .documents()
            .iter()
            .filter(|document| document.starts_with("GET /repos/"))
            .count();
        if refusing {
            let said = written.expect_err("the write is refused").to_string();
            assert!(said.contains("refused the visibility read"), "{said}");
            assert_eq!(reads, 0, "the repository was not read");
            assert_eq!(setup.board.status(&ticket(1)).as_deref(), Some("Todo"));
        } else {
            written.expect("the write lands");
            assert!(reads >= 1);
        }
    }
}

fn ticket_id(n: usize) -> GlobalId {
    GlobalId {
        source: board(),
        native: NativeId(ticket(n)),
    }
}

#[test]
fn every_request_answered_with_readings_or_without_ends_once_and_a_refused_one_never() {
    let runtime = runtime();
    for (what, setup) in [
        ("with readings", Setup::new(5)),
        ("without readings", Setup::without_readings(5)),
    ] {
        let gate = Recording::new(&setup.board, None);
        let engine = setup.engine().with_request_gate(gate.clone());
        let answer = runtime
            .block_on(engine.update_tasks(&board(), &updates(0..5)))
            .expect("the batch runs");
        assert!(outcomes(&answer).iter().all(|outcome| outcome == "written"));
        runtime.block_on(engine.quota(&board())).expect("a reading");
        let asked = gate.asked().len();
        // The batch's reads and writes and the quota read, at the least.
        assert!(asked >= 3, "{what}: {asked}");
        assert_eq!(asked, setup.board.documents().len(), "{what}");
        assert_eq!(gate.ended(), asked, "{what}: one end per admitted request");
        let told = gate.told.lock().unwrap().len();
        if what == "with readings" {
            assert!(told >= asked - 1, "{what}: {told}");
        } else {
            assert_eq!(told, 0, "{what}: no reading was told");
        }
    }

    // Refused at the second ask: one admitted and ended, the second never ended.
    let setup = Setup::new(5);
    let gate = Recording::new(&setup.board, Some(At::Refuse(2)));
    let engine = setup.engine().with_request_gate(gate.clone());
    runtime
        .block_on(engine.update_tasks(&board(), &updates(0..5)))
        .expect("the batch answers each item");
    assert_eq!(gate.asked().len(), 2);
    assert_eq!(setup.board.served_count(), 1);
    assert_eq!(gate.ended(), 1, "the refused request is never ended");
}

#[test]
fn a_request_failing_in_transport_before_any_header_ends_once() {
    // A port nothing listens on: the connection is refused before any byte of an answer.
    let unbound = std::net::TcpListener::bind("127.0.0.1:0").expect("a port");
    let endpoint = format!("http://{}/graphql", unbound.local_addr().unwrap());
    drop(unbound);
    let setup = Setup::at_endpoint(1, &endpoint);
    let gate = Recording::new(&setup.board, None);
    let engine = setup.engine().with_request_gate(gate.clone());
    let runtime = runtime();
    let answer = runtime.block_on(engine.task(&ticket_id(0)));
    let said = match answer {
        Ok(response) => serde_json::to_value(&response).unwrap()["errors"].to_string(),
        Err(error) => error.to_string(),
    };
    assert!(said.contains("request failed"), "{said}");
    let quota = runtime.block_on(engine.quota(&board()));
    assert!(quota.is_err(), "the quota read fails in transport too");
    let asked = gate.asked().len();
    assert!(
        asked >= 2,
        "the read and the quota read were asked: {asked}"
    );
    assert_eq!(gate.ended(), asked, "each failed request ended once");
    assert!(gate.told.lock().unwrap().is_empty());
}

#[test]
fn a_request_dropped_while_in_flight_ends_once() {
    // A listener that takes the request and never answers it.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a port");
    let endpoint = format!("http://{}/graphql", listener.local_addr().unwrap());
    let (accepted, on_accept) = tokio::sync::oneshot::channel::<()>();
    std::thread::spawn(move || {
        let (stream, _) = listener.accept().expect("the request's connection");
        let _ = accepted.send(());
        // Held open, unanswered, until the test process ends.
        std::thread::sleep(Duration::from_secs(600));
        drop(stream);
    });
    let setup = Setup::at_endpoint(1, &endpoint);
    let gate = Recording::new(&setup.board, None);
    let engine = setup.engine().with_request_gate(gate.clone());
    let id = ticket_id(0);
    runtime().block_on(async {
        tokio::select! {
            answer = engine.task(&id) => panic!("the request was answered: {answer:?}"),
            accepted = on_accept => accepted.expect("the request reached the listener"),
        }
        // The read's future is dropped here, its request in flight.
    });
    assert_eq!(gate.asked().len(), 1);
    assert_eq!(gate.ended(), 1, "the dropped request ended once");
}

#[test]
fn a_repository_visibility_read_failing_in_transport_ends_once() {
    use onetaskgraph_e2e_support::loopback::{self, After, Upstream};
    use std::sync::atomic::{AtomicUsize, Ordering};
    let sandbox = Sandbox::new();
    let (mut config, github) = github_projects_with_items(&sandbox, vec![issue(&ticket(1))]);
    // In front of the board: every request forwarded but a repository read, whose connection
    // is closed with nothing answered — a transport failure before any header.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a port");
    let board_address: std::net::SocketAddr = config["endpoint"]
        .as_str()
        .and_then(|endpoint| endpoint.strip_prefix("http://"))
        .and_then(|rest| rest.split('/').next())
        .and_then(|address| address.parse().ok())
        .expect("the board's address");
    config["endpoint"] = json!(format!("http://{}/graphql", listener.local_addr().unwrap()));
    let dropped = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&dropped);
    loopback::serve_each(listener, move || {
        let mut upstream = Upstream::new(board_address);
        let counted = Arc::clone(&counted);
        move |sent: &loopback::Request, stream: &mut std::net::TcpStream| {
            use std::io::Write;
            if sent.line().starts_with("GET /repos/") {
                counted.fetch_add(1, Ordering::SeqCst);
                return After::Close;
            }
            let response = upstream.forward(&sent.bytes());
            if response.is_empty() {
                return After::Close;
            }
            stream.write_all(&response).expect("the answer is relayed");
            After::KeepOpen
        }
    });
    sandbox.project_document(&document(&json!({
        "board": {"plugin":"github-projects","config":config,"visibility":"private"},
    })));
    let setup = Setup {
        sandbox,
        board: github,
    };
    let gate = Recording::new(&setup.board, None);
    let engine = setup.engine().with_request_gate(gate.clone());
    let written = runtime().block_on(engine.set_task_status(
        &ticket_id(1),
        onetaskgraph_plugin_api::StatusCategory::InProgress,
    ));
    let said = written
        .expect_err("an unreadable visibility refuses the write")
        .to_string();
    assert!(said.contains("could not read whether repository"), "{said}");
    assert!(
        dropped.load(Ordering::SeqCst) >= 1,
        "the repository was read"
    );
    let asked = gate.asked();
    assert!(
        asked.iter().any(|(request, _)| request.resource == "core"),
        "the repository read was asked about: {asked:?}"
    );
    assert_eq!(
        gate.ended(),
        asked.len(),
        "every admitted request ended once, the failed repository read among them"
    );
    assert_eq!(setup.board.status(&ticket(1)).as_deref(), Some("Todo"));
}
