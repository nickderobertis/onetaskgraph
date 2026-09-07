//! A board that prices a document differently from this workspace, and the run it fails.
//!
//! The whole-session drive in `tests/plugin.rs` answers every `rateLimit(dryRun: true)` probe
//! with what this workspace computes, so it agrees by construction — which is exactly why it
//! is not evidence that the reconciliation would notice a disagreement. This points the same
//! journey at a board configured to report a price this workspace does not compute, over real
//! HTTP through the journey's own send path, and watches the run fail naming both figures.
//! GitHub is the authority on its own pricing, and what this proves is that this repository
//! really reads GitHub's answer rather than its own.
//!
//! The board is the fixture's own: `board::answer_a_stateless_session_call` is the code
//! `tests/plugin.rs` answers these calls with, under a different [`Pricing`]. A second
//! spelling here would make this a proof about the spelling.
//!
//! **One journey per test binary, which is why this is a binary of its own.** A journey is
//! pointed at one API by a `OnceLock` and reports one session's cost into one accounting, so
//! the truthful drive and this one cannot share a process. The truthful half is the drive in
//! `tests/plugin.rs`, where all seven read documents reconcile over the same HTTP path.
//!
//! No credential and no third-party API: everything below is answered by a loopback listener
//! this file starts.

use std::io::Write as _;
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;

use onetaskgraph_github_projects::accounting::Mode;
use onetaskgraph_github_projects::{graphql, worst_case_point_cost};
use serde_json::json;

// The board's shared answers, and the credentialed lane's own halves again. What this file
// reaches of `board` it reaches all of, so that one carries no suppression; most of
// `journey` and `lane` is for the drives that reach a board's state or GitHub itself, so
// what this one does not reach is those drives' rather than dead code.
mod board;
#[allow(dead_code)]
mod journey;
#[allow(dead_code)]
mod lane;

use board::{FIXTURE_BUDGET_LIMIT, Pricing};

/// How many points above the computed price the board below reports.
///
/// One is the smallest disagreement there is, which is the point: a check that only noticed a
/// large one would pass over the drift this exists to catch.
const OVERSTATED_BY: u64 = 1;

/// A loopback board answering the calls a run makes before it reaches a board's own state.
///
/// That is the allowance read the budget precondition makes over REST, the mutation-schema
/// introspection, and the reconciliation's own allowance reads and probes — all of them from
/// [`board`], under the `pricing` given. Anything else is refused with a GraphQL error naming
/// the document: the run under test is meant to end at the reconciliation, and a board that
/// answered its way past that would hide that it had not.
///
/// Every document it is asked is recorded, so a test can assert the probe really arrived over
/// the wire rather than that a failure happened for some other reason.
fn serving(pricing: Pricing, asked: &Arc<Mutex<Vec<String>>>) -> journey::Endpoints {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a fixture listener");
    let host = format!("http://{}", listener.local_addr().unwrap());
    let recorded = Arc::clone(asked);
    thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.expect("a fixture connection");
            let (method, path, body) = board::read_http_request(&mut stream);
            let (status, payload) = match (method.as_str(), path.as_str()) {
                ("GET", "/rate_limit") => ("200 OK", board::ample_allowance().to_string()),
                ("POST", "/graphql") => {
                    let request = body.expect("a GraphQL request carries a body");
                    let query = request["query"].as_str().expect("a GraphQL document");
                    graphql_parser::parse_query::<String>(query).expect("a valid GraphQL document");
                    recorded.lock().unwrap().push(query.to_owned());
                    let answered = board::answer_a_stateless_session_call(query, pricing);
                    let body = match answered {
                        Some(data) => json!({ "data": data }),
                        None => json!({"errors":[{"message":
                            format!("this board answers a session's own calls and nothing else: {query}")}]}),
                    };
                    ("200 OK", body.to_string())
                }
                _ => (
                    "404 Not Found",
                    json!({"message":format!("this board does not answer {method} {path}")})
                        .to_string(),
                ),
            };
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n\
                 x-ratelimit-limit: {FIXTURE_BUDGET_LIMIT}\r\n\
                 x-ratelimit-used: 1\r\n\
                 x-ratelimit-remaining: {}\r\n\
                 x-ratelimit-resource: graphql\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                FIXTURE_BUDGET_LIMIT - 1,
                payload.len()
            );
            stream.write_all(response.as_bytes()).expect("a response");
        }
    });
    journey::Endpoints {
        graphql: format!("{host}/graphql"),
        rest_host: host,
        source: None,
    }
}

/// The first document the reconciliation asks GitHub about, from the inventory.
///
/// Read out of `graphql::DOCUMENTS` rather than named here: the reconciliation walks that
/// list and skips the mutations, so this is the document a mispricing board is first caught
/// on — and it stays that whatever is added to the list later.
fn first_document_asked_about() -> (&'static str, &'static str) {
    *graphql::DOCUMENTS
        .iter()
        .find(|(document, _)| Mode::of_document(document) == Mode::Read)
        .expect("this source sends at least one query document")
}

/// A run against a board that prices a document differently fails, naming both figures.
///
/// It drives `journey::run` — the whole entry point the credentialed lane drives — so what
/// refuses is the real reconciliation reading a real answer off a real HTTP response, rather
/// than a verdict handed a value this test built. The journey reports every failure by
/// panicking, so the run is joined for its panic and the message is what is asserted on.
#[test]
fn a_board_that_prices_a_document_differently_fails_the_run_naming_both_figures() {
    let asked = Arc::new(Mutex::new(Vec::new()));
    journey::against(serving(Pricing::Overstating { by: OVERSTATED_BY }, &asked));
    let (document, doing) = first_document_asked_about();
    let ours = worst_case_point_cost(document).expect("a priceable document");

    // Its own thread and its own runtime, because the journey's failure is a panic: joining
    // the thread is what turns that into a value this test can read, and a panic on the
    // test's own thread could only be asserted on by not asserting at all.
    let refusal = thread::spawn(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime for the journey")
            .block_on(journey::run(journey::Nomination {
                token: "test-token".to_owned(),
                owner: "octo-org".to_owned(),
                project_number: 7,
                repository: "acme/work".to_owned(),
            }));
    })
    .join()
    .expect_err("a run against a board that misprices a document does not finish");
    let refusal = panicked_with(&refusal);

    // GitHub's own figure and this workspace's, both named, and the document they are about.
    assert!(refusal.contains(doing), "{refusal}");
    assert!(
        refusal.contains(&format!("at {} points", ours + OVERSTATED_BY)),
        "{refusal}"
    );
    assert!(refusal.contains(&format!("computes {ours}")), "{refusal}");
    assert!(refusal.contains("GitHub is the authority"), "{refusal}");

    // And it really came off the wire: the board was asked that document, joined to the probe
    // that carries the price. A run that failed before sending it would satisfy every
    // assertion above if the message were built from the inventory alone.
    let asked = asked.lock().unwrap();
    assert!(
        asked.iter().any(|sent| {
            sent.contains("rateLimit(dryRun:true)")
                && board::strip_probe(sent).as_deref() == Some(document)
        }),
        "the board was never asked about {doing}; it answered {asked:?}"
    );
}

/// What a joined thread's panic said, whichever way the payload was spelled.
fn panicked_with(payload: &Box<dyn std::any::Any + Send>) -> String {
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| {
            payload
                .downcast_ref::<&str>()
                .map(|held| (*held).to_owned())
        })
        .unwrap_or_else(|| "the journey panicked with a payload this test cannot read".to_owned())
}

/// The board answers the same probe truthfully under the pricing the session drive uses.
///
/// One journey per binary, so the truthful *run* is `tests/plugin.rs`'s; what is left to hold
/// here is that the configuration above is the only difference between the two — a board
/// whose answers disagreed for some other reason would fail the test above for the wrong
/// reason.
#[test]
fn the_same_board_answers_that_document_as_this_workspace_prices_it() {
    let (document, _) = first_document_asked_about();
    let ours = worst_case_point_cost(document).expect("a priceable document");
    let truthful = board::probe_answer(document, Pricing::AsComputed);
    let overstated = board::probe_answer(document, Pricing::Overstating { by: OVERSTATED_BY });
    assert_eq!(truthful.pointer("/rateLimit/cost"), Some(&json!(ours)));
    assert_eq!(
        overstated.pointer("/rateLimit/cost"),
        Some(&json!(ours + OVERSTATED_BY))
    );
    assert_eq!(
        truthful.pointer("/rateLimit/nodeCount"),
        overstated.pointer("/rateLimit/nodeCount"),
        "only the price is what a mispricing board changes"
    );
}
