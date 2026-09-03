//! Every branch of the budget precondition, against a loopback stand-in for GitHub.
//!
//! The precondition is the real one — `journey::budget`, the same code the credentialed
//! lane runs — and what stands in for GitHub is a local HTTP server that answers
//! `GET /rate_limit` the way GitHub's own documentation says that endpoint answers, and
//! **refuses everything else**. That refusal is what makes the first test below evidence:
//! a precondition that sent anything besides the one read it cannot do without would be
//! answered `404` and would fail here rather than quietly costing an account a request.
//!
//! No credential and no third-party API: every allowance below is one this file wrote.
//! What a stand-in cannot prove is that GitHub really answers `/rate_limit` in this shape —
//! that is GitHub's to demonstrate, and the citations in `journey::budget` are where the
//! shape came from.

use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;

use onetaskgraph_github_projects::accounting::{Accounting, Budget, Method};
use onetaskgraph_live::{RETAINED_BUFFER, Unaffordable};
use serde_json::{Value, json};

// The credentialed lane's own halves again: `journey` for the precondition under test, and
// `lane` for the session name a refusal is reported under. Most of each is for the target
// that drives it against GitHub, so what this one does not reach is the other drive's
// rather than dead code.
#[allow(dead_code)]
mod journey;
#[allow(dead_code)]
mod lane;

use journey::budget;

/// The whole allowance every stand-in below reports, for both budgets.
///
/// Deliberately not GitHub's published 5,000: a figure both sides already knew would let a
/// refusal print the right number without ever having read what it was sent.
const LIMIT: u64 = 4_321;

/// The UTC epoch second every stand-in below says its budgets come back.
const RESETS_AT: u64 = 1_775_000_000;

/// A local stand-in for GitHub's rate-limit endpoint.
///
/// `answer` is what it replies to `GET /rate_limit` with, as a status and a body; every
/// other path is a `404` and is recorded, so a caller can assert both what was asked for
/// and that nothing else was.
struct Standin {
    host: String,
    asked: Arc<Mutex<Vec<String>>>,
}

impl Standin {
    fn serving(answer: (&'static str, String)) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a stand-in listener");
        let host = format!("http://{}", listener.local_addr().unwrap());
        let asked = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&asked);
        thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = stream.expect("a stand-in connection");
                let mut request = [0_u8; 8192];
                let read = stream.read(&mut request).expect("a stand-in request");
                let head = String::from_utf8_lossy(&request[..read]).into_owned();
                let line = head.lines().next().unwrap_or_default().to_owned();
                let mut parts = line.split(' ');
                let called = format!(
                    "{} {}",
                    parts.next().unwrap_or_default(),
                    parts.next().unwrap_or_default()
                );
                recorded.lock().unwrap().push(called.clone());
                let (status, body) = if called == "GET /rate_limit" {
                    (answer.0, answer.1.clone())
                } else {
                    (
                        "404 Not Found",
                        r#"{"message":"this stand-in answers the allowance read alone"}"#
                            .to_owned(),
                    )
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n\
                     x-ratelimit-limit: {LIMIT}\r\nx-ratelimit-used: 0\r\n\
                     x-ratelimit-remaining: {LIMIT}\r\nx-ratelimit-resource: core\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        Self { host, asked }
    }
    /// A stand-in reporting `graphql` points and `rest` requests left of `LIMIT` each.
    ///
    /// The answer is built by the same `documented_answer` the pin gates, rather than
    /// spelled out again here: a stand-in that restated GitHub's shape independently would
    /// prove the parser against a second guess at that contract instead of against the one
    /// `fixtures/rate-limits.json` records.
    fn with(graphql_remaining: u64, rest_remaining: u64) -> Self {
        Self::serving((
            "200 OK",
            budget::documented_answer(LIMIT, graphql_remaining, rest_remaining, RESETS_AT)
                .to_string(),
        ))
    }
    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
}

/// Everything the precondition asks for and everything it decided, from one drive of it.
async fn decide(standin: &Standin) -> (Accounting, Result<(), onetaskgraph_live::Declined>) {
    let into = Accounting::new();
    let decided = budget::precondition("test-token", &standin.host, &into).await;
    (into, decided)
}

/// The estimate this session is sized at, which every case below reads rather than restates.
///
/// It is what `journey::budget::estimate` derives from the branch's own per-call record — no
/// credential, no call to GitHub — and taking it from there is what makes the remainders
/// below move with the session instead of pinning a figure that would go stale.
fn estimated(budget: Budget) -> u64 {
    budget::estimate()
        .get(&budget)
        .copied()
        .expect("this session draws on both of GitHub's budgets")
}

#[tokio::test]
async fn a_budget_with_room_starts_the_session_after_exactly_one_allowance_read() {
    let standin = Standin::with(LIMIT, LIMIT);
    let (into, decided) = decide(&standin).await;
    decided.expect("an untouched allowance affords this session and its buffer");

    // The gate's own traffic: one read, and nothing else. The stand-in would have answered
    // anything else with a 404, so this is the bound rather than a description of it.
    assert_eq!(standin.asked(), vec!["GET /rate_limit".to_owned()]);

    // Recorded like any other request, so what the gate itself cost is in every session
    // report rather than assumed — with the rate-limit facts its response carried.
    let session = into.snapshot();
    assert_eq!(session.total_requests(), 1);
    let read = &session.requests()[0];
    assert_eq!(read.name(), "GET /rate_limit");
    assert_eq!(read.budget(), Budget::Rest);
    assert_eq!(read.rate_limit().limit(), Some(LIMIT));
    assert_eq!(read.rate_limit().remaining(), Some(LIMIT));

    // And the estimate the decision was made on is on the report beside the consumption.
    for budget in [Budget::Graphql, Budget::Rest] {
        assert_eq!(session.estimated(budget), Some(estimated(budget)));
    }
    let report = session.report();
    assert!(
        report.contains(&format!(
            "a precondition estimated {} points before this session started",
            estimated(Budget::Graphql)
        )),
        "{report}"
    );
    assert!(
        report.contains(&format!(
            "a precondition estimated {} requests before this session started",
            estimated(Budget::Rest)
        )),
        "{report}"
    );
}

#[tokio::test]
async fn a_remainder_that_would_dip_into_the_retained_buffer_does_not_start() {
    // One short of what the session and the buffer need together, against the GraphQL
    // budget: this is the case the whole gate exists for.
    let buffer = RETAINED_BUFFER.of(LIMIT);
    let remaining = estimated(Budget::Graphql) + buffer - 1;
    let standin = Standin::with(remaining, LIMIT);
    let (_, decided) = decide(&standin).await;
    let declined = decided.expect_err("a session that would dip into the buffer must not run");

    // Read as a value rather than out of the prose: which budget was short, that budget's
    // limit, what remained, the estimated cost, the retained buffer, and when it resets.
    let cause = declined
        .unaffordable_because()
        .expect("a budget refusal carries the decision it was made on");
    assert_eq!(
        cause,
        &Unaffordable::Short {
            budget: "graphql".to_owned(),
            unit: "points".to_owned(),
            limit: LIMIT,
            remaining,
            estimated_cost: estimated(Budget::Graphql),
            retained_buffer: buffer,
            reset: RESETS_AT,
        }
    );
    // And in the line a person reads an hour later, which leads with the run not having
    // happened rather than with the cause.
    let message = declined.message();
    assert!(message.contains("DID NOT RUN"), "{message}");
    assert!(
        message.contains("not a test failure in the code under test"),
        "{message}"
    );
    for figure in [
        LIMIT,
        remaining,
        estimated(Budget::Graphql),
        buffer,
        RESETS_AT,
    ] {
        assert!(message.contains(&figure.to_string()), "{figure}: {message}");
    }
    // It says when the budget comes back and stops there: nothing waits for it.
    assert!(message.contains("nothing here waits for it"), "{message}");
}

#[tokio::test]
async fn two_budgets_where_only_one_is_short_decline_naming_that_one() {
    // GraphQL has room to spare; the REST budget is one request short of its buffer.
    let buffer = RETAINED_BUFFER.of(LIMIT);
    let rest_remaining = estimated(Budget::Rest) + buffer - 1;
    let standin = Standin::with(LIMIT, rest_remaining);
    let (_, decided) = decide(&standin).await;
    let declined = decided.expect_err("one short budget is enough to decline");
    let cause = declined
        .unaffordable_because()
        .expect("a budget refusal carries the decision it was made on");
    assert_eq!(cause.budget(), "rest");
    assert!(cause.reason().contains("requests"), "{}", cause.reason());

    // The same board with that one request back afford it, so what declined above really is
    // the REST budget and not something both stand-ins share.
    let (_, allowed) = decide(&Standin::with(LIMIT, rest_remaining + 1)).await;
    allowed.expect("one more request is what that budget was short of");
}

#[tokio::test]
async fn an_allowance_read_the_stand_in_refuses_does_not_start_and_says_which_read_failed() {
    for (status, body) in [
        ("503 Service Unavailable", json!({"message":"unavailable"})),
        // Answered, but with no object for the budget being asked about: an allowance that
        // is not there is not one to assume either.
        ("200 OK", json!({"resources":{"search":{"limit":30}}})),
    ] {
        let standin = Standin::serving((status, body.to_string()));
        let (into, decided) = decide(&standin).await;
        let declined = decided.expect_err("an unread allowance is not an affordable one");
        let cause = declined
            .unaffordable_because()
            .expect("an unread budget carries the read that was not answered");
        let Unaffordable::Unread { budget, why } = cause else {
            panic!("an unanswered read is unread rather than short: {cause:?}");
        };
        assert_eq!(budget, "graphql");
        assert!(why.contains("/rate_limit"), "{why}");
        assert!(declined.message().contains("DID NOT RUN"), "{status}");
        // The read that failed is still recorded, so a session report says the gate asked.
        assert_eq!(into.snapshot().total_requests(), 1);
    }
}

#[tokio::test]
async fn an_allowance_read_that_reaches_nothing_at_all_does_not_start() {
    // A host nothing is listening on: the send itself fails, which carries no headers and
    // no body to read a budget out of.
    let nowhere = TcpListener::bind("127.0.0.1:0").expect("a port to close");
    let host = format!("http://{}", nowhere.local_addr().unwrap());
    drop(nowhere);
    let into = Accounting::new();
    let declined = budget::precondition("test-token", &host, &into)
        .await
        .expect_err("a read that reached nothing leaves the budget unknown");
    assert!(matches!(
        declined.unaffordable_because(),
        Some(Unaffordable::Unread { .. })
    ));
    assert!(declined.message().contains("DID NOT RUN"));
}

#[tokio::test]
async fn the_allowance_read_matches_its_pinned_artifact() {
    // The endpoint, the method, the two resource objects and the three fields this
    // precondition reads are **GitHub's contract and not this repository's decisions**, and
    // this source restates every one of them. Restating an external contract without a gate
    // is how the restatement quietly stops being true, so
    // `fixtures/rate-limits.json` is the pin — recorded with its date and page in the README
    // beside it — and this reconciles the two **both ways**: a name the precondition reads
    // that nothing pinned, and a pinned name it no longer reads, each fail here.
    let pinned: Value =
        serde_json::from_str(include_str!("fixtures/rate-limits.json")).expect("the pin parses");
    let allowance = &pinned["allowance"];

    assert_eq!(
        allowance["endpoint"].as_str(),
        Some(budget::ALLOWANCE_ENDPOINT),
        "the precondition calls an endpoint fixtures/rate-limits.json does not pin"
    );
    assert_eq!(
        allowance["method"].as_str().and_then(Method::parse),
        Some(budget::ALLOWANCE_METHOD),
        "the precondition addresses that endpoint with a method the pin does not record"
    );

    // Both ways over the two budgets this session draws on: `resources.rest` in the pin is
    // GitHub's `core`, and pretending the two names agree would read the wrong figures.
    let pinned_resources = allowance["resources"]
        .as_object()
        .expect("the pin records one resource name per budget");
    for budget in [Budget::Graphql, Budget::Rest] {
        assert_eq!(
            pinned_resources.get(budget.name()).and_then(Value::as_str),
            Some(budget::resource_of(budget)),
            "the {} budget is read out of a resources object the pin does not record",
            budget.name()
        );
    }
    assert_eq!(
        pinned_resources.len(),
        2,
        "the pin records a resource this precondition does not read a budget out of"
    );

    let pinned_fields: Vec<&str> = allowance["fields"]
        .as_array()
        .expect("the pin records the fields of a budget's object")
        .iter()
        .map(|field| field.as_str().expect("each pinned field is a string"))
        .collect();
    assert_eq!(
        pinned_fields,
        budget::ALLOWANCE_FIELDS,
        "the fields this precondition reads and the fields the pin records have parted"
    );

    // And the pin has teeth rather than being a second list nobody consults: the read is
    // driven against an answer missing each pinned field in turn, and every one of them
    // leaves the budget UNREAD rather than assumed. A field the parser stopped needing
    // would start affording a session on an allowance it never read.
    for absent in budget::ALLOWANCE_FIELDS {
        let mut answer = budget::documented_answer(LIMIT, LIMIT, LIMIT, RESETS_AT);
        answer["resources"][budget::GRAPHQL_RESOURCE]
            .as_object_mut()
            .expect("the answer carries the graphql budget")
            .remove(absent);
        let standin = Standin::serving(("200 OK", answer.to_string()));
        let (_, decided) = decide(&standin).await;
        let declined = decided.expect_err(&format!(
            "an allowance missing {absent} must not be assumed"
        ));
        let cause = declined.unaffordable_because().expect("a budget refusal");
        let Unaffordable::Unread { budget, why } = cause else {
            panic!("an allowance missing {absent} is unread rather than short: {cause:?}");
        };
        assert_eq!(budget, "graphql");
        assert!(why.contains(absent), "{absent} is not named in {why}");
    }
}

#[tokio::test]
async fn an_allowance_reporting_more_left_than_it_holds_is_not_one_to_decide_on() {
    // A third party answering `remaining` above `limit` is answering something that cannot
    // be true, and a gate that took it would compute a buffer from one figure and compare
    // it against a remainder from another. Unread rather than short: this session has no
    // allowance for that budget.
    let standin = Standin::serving((
        "200 OK",
        budget::documented_answer(LIMIT, LIMIT + 1, LIMIT, RESETS_AT).to_string(),
    ));
    let (_, decided) = decide(&standin).await;
    let declined = decided.expect_err("an impossible allowance is not an affordable one");
    let cause = declined.unaffordable_because().expect("a budget refusal");
    let Unaffordable::Unread { budget, why } = cause else {
        panic!("an impossible allowance is unread rather than short: {cause:?}");
    };
    assert_eq!(budget, "graphql");
    assert!(why.contains("cannot both be true"), "{why}");
}

#[tokio::test]
async fn a_rest_budget_the_answer_omits_declines_although_the_graphql_one_read() {
    // The REST budget is decided after the GraphQL one, so an answer that carries a whole
    // GraphQL allowance and no `core` object is the only way to reach that branch — and
    // without it a session would start on a budget it never read.
    let mut answer = budget::documented_answer(LIMIT, LIMIT, LIMIT, RESETS_AT);
    answer["resources"]
        .as_object_mut()
        .expect("the answer carries a resources object")
        .remove(budget::REST_RESOURCE);
    let standin = Standin::serving(("200 OK", answer.to_string()));
    let (_, decided) = decide(&standin).await;
    let declined = decided.expect_err("a budget with no object is one this session did not read");
    let cause = declined.unaffordable_because().expect("a budget refusal");
    assert_eq!(cause.budget(), "rest");
    assert!(
        cause.reason().contains(budget::REST_RESOURCE),
        "{}",
        cause.reason()
    );
}

#[tokio::test]
async fn the_estimate_is_derived_from_the_branchs_own_record_of_the_session() {
    // Derived offline from the record beside this crate: this test makes no call to GitHub
    // and reads no credential, and the estimate moves with the session rather than with an
    // edit, because that record is what the session-cost test rewrites.
    let record = include_str!("fixtures/session-cost.txt");
    let rows: Vec<(u64, &str)> = record
        .lines()
        .skip_while(|line| line.trim() != "requests per call")
        .skip(1)
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let mut fields = line.split_whitespace();
            let requests: u64 = fields.next().unwrap().parse().unwrap();
            let _nodes = fields.next().unwrap();
            (requests, line.split_whitespace().nth(2).unwrap())
        })
        .collect();
    let rest_calls: u64 = rows
        .iter()
        .filter(|(_, first)| Method::parse(first).is_some())
        .map(|(requests, _)| requests)
        .sum();
    let calls: u64 = rows.iter().map(|(requests, _)| requests).sum();
    let estimate = budget::estimate();

    // REST is metered in requests, so its estimate is exactly the REST calls the record
    // holds — the model rather than a coincidence, and reading them off the record here is
    // what would catch the two parting.
    assert_eq!(estimate[&Budget::Rest], rest_calls);
    assert!(rest_calls > 0 && rest_calls < calls);

    // GraphQL is metered in points, and **node count is not that estimate**: the session's
    // 1,757,301 worst-case nodes are two orders of magnitude above the points it is sized
    // at, which is the whole reason the model divides by the smallest page size before it
    // divides by GitHub's hundred.
    let graphql = estimate[&Budget::Graphql];
    let nodes: u64 = record
        .lines()
        .skip_while(|line| line.trim() != "requests per call")
        .skip(1)
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| line.split_whitespace().nth(1)?.parse::<u64>().ok())
        .sum();
    assert!(
        graphql * 100 < nodes,
        "{graphql} points against {nodes} nodes"
    );
    // And conservative where it is unsure: comfortably above the one-point-per-call lower
    // bound the accounting attributes, which is what a gate must never size itself from.
    assert!(
        graphql > calls - rest_calls,
        "{graphql} points against {} GraphQL calls",
        calls - rest_calls
    );
}

/// The variable that asks the journey decline below to be followed through to its
/// conclusion instead of only asserted.
///
/// Unset — which is every ordinary run — the test asserts the outcome and passes. Set, it
/// re-raises the very panic the decline made, so the target fails and `cargo test` exits
/// non-zero. That is the second half of what this branch owes: a run that declined for want
/// of budget must leave the required check concluding something branch protection accepts
/// neither as success nor in place of it. `scripts/check-budget-decline.sh` is what sets it
/// and reads the conclusion.
const FOLLOW_THROUGH: &str = "ONETASKGRAPH_BUDGET_DECLINE_FOLLOW_THROUGH";

#[test]
fn a_journey_the_account_cannot_afford_does_not_run_and_says_which_budget_was_short() {
    // The whole journey, driven the way both of its drives drive it, against a stand-in
    // whose GraphQL budget is one point short of this session plus its retained buffer.
    // Nothing below the precondition is reached: the decline is the session's first act.
    let buffer = RETAINED_BUFFER.of(LIMIT);
    let remaining = estimated(Budget::Graphql) + buffer - 1;
    let standin = Standin::with(remaining, LIMIT);
    journey::against(journey::Endpoints {
        // Both point at the stand-in, which answers the allowance read and 404s everything
        // else, so a journey that somehow got past the gate would fail here rather than
        // reach GitHub.
        graphql: format!("{}/graphql", standin.host),
        rest_host: standin.host.clone(),
        source: Some(json!({"endpoint": format!("{}/graphql", standin.host)})),
    });
    let declined = thread::spawn(move || {
        // Its own runtime on its own thread, so the panic `Declined::refuse` makes can be
        // caught and read rather than only observed as a failing test — which is what lets
        // one test assert the outcome and, under the variable above, follow it through.
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime for the declined journey")
            .block_on(journey::run(journey::Nomination {
                token: "test-token".to_owned(),
                owner: "octo-org".to_owned(),
                project_number: 7,
                repository: "acme/work".to_owned(),
            }));
    })
    .join()
    .expect_err("a journey the account cannot afford must not run");

    let message = declined
        .downcast_ref::<String>()
        .cloned()
        .unwrap_or_else(|| panic!("the decline carries its message: {declined:?}"));
    assert!(message.contains("DID NOT RUN"), "{message}");
    assert!(
        message.contains("not a test failure in the code under test"),
        "{message}"
    );
    assert!(message.contains("graphql"), "{message}");
    for figure in [
        LIMIT,
        remaining,
        estimated(Budget::Graphql),
        buffer,
        RESETS_AT,
    ] {
        assert!(message.contains(&figure.to_string()), "{figure}: {message}");
    }
    // One request, and it is the allowance read: a declined session spends the account
    // nothing beyond the read it declined on, and it does not retry, poll or wait.
    assert_eq!(standin.asked(), vec!["GET /rate_limit".to_owned()]);

    if std::env::var_os(FOLLOW_THROUGH).is_some() {
        std::panic::resume_unwind(declined);
    }
}
