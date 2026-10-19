//! The second reading of the budget precondition, against a loopback stand-in for GitHub.
//!
//! `tests/budget_gate.rs` proves the first reading: the free `GET /rate_limit` and every way
//! it can refuse a session. This target proves what happens **after** that read has admitted
//! one — that the session is decided again on the headers its own first real call carries,
//! and declined on them where they refuse what the endpoint claimed. The journey is the real
//! one, `journey::run`, driven the way both of its drives drive it; what stands in for
//! GitHub is one local HTTP server that answers the allowance read with a whole allowance,
//! answers the session's first real call — the first contract-schema introspection document
//! — with whatever a test scripts, and refuses everything else with a `404` it records.
//!
//! That recording is what makes each decline below evidence about **artifacts** as well as
//! about the decision: a session declined on its first real call has made exactly two
//! requests, the free read and that call, and nothing that could have written anything.
//! It is also what shows the one refusal that is deliberately *not* a decline — a call that
//! never reached the host — going on to fail as itself.
//!
//! No credential and no third-party API: every allowance and every header below is one this
//! file wrote. What a stand-in cannot prove is that GitHub really disagrees with itself in
//! this way — that was observed on a real account, and `journey::budget` records where.
//!
//! One stand-in and one journey per test binary, because `journey::against` points the
//! journey at one API once: the tests below take turns on it, each scripting the stand-in
//! before its run, and each run's own allowance read is what `budget::recheck` counts from.

use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, PoisonError};
use std::thread;

use onetaskgraph_github_projects::accounting::{Accounting, Budget, Outcome, RateLimit, Request};
use onetaskgraph_live::{Allowance, RETAINED_BUFFER, Unaffordable};
use serde_json::json;

// The credentialed lane's own halves again: `journey` for the precondition, its second
// reading and the run under test, and `lane` for the session name a refusal is reported
// under. Most of each is for the target that drives the journey against GitHub, so what
// this one does not reach is the other drive's rather than dead code — the same reason
// `tests/budget_gate.rs` and `tests/plugin.rs` carry these two.
#[allow(dead_code)]
mod journey;
#[allow(dead_code)]
mod lane;

use journey::budget;

/// The whole allowance the stand-in reports, for both budgets and in every header.
///
/// Not GitHub's 5,000, for the reason `tests/budget_gate.rs` gives: a figure both sides
/// already knew would let a refusal print the right number without having read it.
const LIMIT: u64 = 4_321;

/// When the endpoint says the budgets come back.
const RESETS_AT: u64 = 1_775_000_000;

/// When the first real call's own headers say the budget comes back — deliberately not the
/// endpoint's figure, so a decline on the second reading can be seen to report the second
/// reading's reset rather than the first's.
const HEADERS_RESET_AT: u64 = 1_775_000_777;

/// The reason GitHub gives for a refused call whose primary budget is spent.
const ALREADY_EXCEEDED: &str = "API rate limit already exceeded for user ID 1.";

/// What the stand-in does with the session's first real call.
#[derive(Clone)]
enum FirstCall {
    /// Answer it with this status, these `x-ratelimit-*` lines — complete with their line
    /// endings, or none at all — and this body.
    Answer {
        status: &'static str,
        headers: String,
        body: String,
    },
    /// Close the connection without answering, which the client reports as a request that
    /// never reached the host.
    HangUp,
}

impl FirstCall {
    /// An answered call whose headers report `remaining` of [`LIMIT`] on the GraphQL budget.
    fn reporting(remaining: u64) -> Self {
        Self::Answer {
            status: "200 OK",
            headers: graphql_headers(remaining, LIMIT.saturating_sub(remaining)),
            body: json!({"data": {}}).to_string(),
        }
    }
}

/// The headers GitHub attaches to a GraphQL response, as the fixture board spells them.
fn graphql_headers(remaining: u64, used: u64) -> String {
    format!(
        "x-ratelimit-limit: {LIMIT}\r\nx-ratelimit-used: {used}\r\n\
         x-ratelimit-remaining: {remaining}\r\nx-ratelimit-reset: {HEADERS_RESET_AT}\r\n\
         x-ratelimit-resource: graphql\r\n"
    )
}

/// One whole request off `stream` — its head and as much body as its `Content-Length` says —
/// so what it asks for is read whatever the client's writes split it into.
fn whole_request(stream: &mut std::net::TcpStream) -> String {
    let mut held = Vec::new();
    let mut chunk = [0_u8; 16384];
    loop {
        let read = stream.read(&mut chunk).expect("a stand-in request");
        held.extend_from_slice(&chunk[..read]);
        let text = String::from_utf8_lossy(&held).into_owned();
        let Some(end) = text.find("\r\n\r\n") else {
            if read == 0 {
                return text;
            }
            continue;
        };
        let length = text[..end]
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .and_then(|length| length.trim().parse::<usize>().ok())
            })
            .unwrap_or(0);
        if read == 0 || held.len() >= end + 4 + length {
            return text;
        }
    }
}

/// What the stand-in answers every path it does not serve with.
fn refused() -> (&'static str, String, String) {
    (
        "404 Not Found",
        String::new(),
        json!({"message": "this stand-in answers the allowance reads and the first real call alone"})
            .to_string(),
    )
}

/// The one stand-in this binary drives the journey against.
struct Standin {
    host: String,
    /// What the next run's first real call is answered with — taken by that call, so every
    /// later GraphQL document of the run is refused like any other path.
    first_call: Arc<Mutex<Option<FirstCall>>>,
    asked: Arc<Mutex<Vec<String>>>,
}

/// How the stand-in records the read of GraphQL's own rateLimit object, which it answers
/// with the whole allowance and never counts as a session's first real call.
const OBJECT_READ: &str = "POST /graphql rateLimit";

impl Standin {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a stand-in listener");
        let host = format!("http://{}", listener.local_addr().unwrap());
        let first_call = Arc::new(Mutex::new(None));
        let asked = Arc::new(Mutex::new(Vec::new()));
        let (scripted, recorded) = (Arc::clone(&first_call), Arc::clone(&asked));
        thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = stream.expect("a stand-in connection");
                let head = whole_request(&mut stream);
                let line = head.lines().next().unwrap_or_default().to_owned();
                let mut parts = line.split(' ');
                let mut called = format!(
                    "{} {}",
                    parts.next().unwrap_or_default(),
                    parts.next().unwrap_or_default()
                );
                if called == "POST /graphql" && head.contains(budget::RATE_LIMIT_OBJECT) {
                    called = OBJECT_READ.to_owned();
                }
                recorded.lock().unwrap().push(called.clone());
                // The endpoint and the rateLimit object claim the whole allowance of both
                // budgets every time; only the FIRST real GraphQL call of a run is answered as
                // scripted, and every later one is refused like any other path, so a journey
                // that got past the re-check goes no further than that.
                let (status, headers, body) = match called.as_str() {
                    "GET /rate_limit" => (
                        "200 OK",
                        format!(
                            "x-ratelimit-limit: {LIMIT}\r\nx-ratelimit-used: 0\r\n\
                             x-ratelimit-remaining: {LIMIT}\r\nx-ratelimit-resource: core\r\n"
                        ),
                        budget::documented_answer(LIMIT, LIMIT, LIMIT, RESETS_AT).to_string(),
                    ),
                    OBJECT_READ => (
                        "200 OK",
                        format!(
                            "x-ratelimit-limit: {LIMIT}\r\nx-ratelimit-used: 0\r\n\
                             x-ratelimit-remaining: {LIMIT}\r\nx-ratelimit-reset: {RESETS_AT}\r\n\
                             x-ratelimit-resource: graphql\r\n"
                        ),
                        json!({"data": {"rateLimit": {"limit": LIMIT, "remaining": LIMIT,
                            "resetAt": chrono::DateTime::from_timestamp(
                                i64::try_from(RESETS_AT).unwrap(), 0).unwrap().to_rfc3339()}}})
                        .to_string(),
                    ),
                    "POST /graphql" => match scripted.lock().unwrap().take() {
                        Some(FirstCall::Answer {
                            status,
                            headers,
                            body,
                        }) => (status, headers, body),
                        Some(FirstCall::HangUp) => continue,
                        None => refused(),
                    },
                    _ => refused(),
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n{headers}\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        Self {
            host,
            first_call,
            asked,
        }
    }
}

static STANDIN: LazyLock<Standin> = LazyLock::new(|| {
    let standin = Standin::start();
    journey::against(journey::Endpoints {
        graphql: format!("{}/graphql", standin.host),
        rest_host: standin.host.clone(),
        source: Some(json!({"endpoint": format!("{}/graphql", standin.host)})),
        settle: std::time::Duration::ZERO,
    });
    standin
});

/// One journey at a time: the journey's accounting and endpoint are one per binary, so two
/// runs interleaving would record into each other's session.
static ONE_RUN_AT_A_TIME: Mutex<()> = Mutex::new(());

/// What one drive of the journey produced: the panic it ended in, and what the stand-in
/// was asked during it.
struct Drive {
    panic: Box<dyn std::any::Any + Send>,
    asked: Vec<String>,
}

impl Drive {
    fn message(&self) -> String {
        self.panic
            .downcast_ref::<String>()
            .cloned()
            .unwrap_or_else(|| panic!("the run carries its message: {:?}", self.panic))
    }
}

/// Drive the whole journey against the stand-in with the first real call answered as
/// `first_call`, holding the turn for as long as the run lasts.
///
/// Every drive here ends in a panic — a decline, or the schema verification refusing the
/// stand-in's empty answer once the session has gone on past the re-check — and the panic is
/// caught on the run's own thread so one test can read the outcome rather than only fail
/// on it. The guard is returned so the caller's assertions about `asked` are made before the
/// next test's run can add to it.
fn drive(first_call: FirstCall) -> (Drive, MutexGuard<'static, ()>) {
    let turn = take_the_turn();
    let standin = &*STANDIN;
    *standin.first_call.lock().unwrap() = Some(first_call);
    let before = standin.asked.lock().unwrap().len();
    let panic = thread::spawn(|| {
        block_on(journey::run(journey::Nomination {
            token: "test-token".to_owned(),
            owner: "octo-org".to_owned(),
            project_number: 7,
            repository: "acme/work".to_owned(),
            writer: lane::Writer::this_machine(),
        }));
    })
    .join()
    .expect_err("every drive of this stand-in ends in a panic, declined or failed");
    let asked = standin.asked.lock().unwrap()[before..].to_vec();
    (Drive { panic, asked }, turn)
}

/// Take the turn on the stand-in, whichever test last held it and however that test ended.
fn take_the_turn() -> MutexGuard<'static, ()> {
    ONE_RUN_AT_A_TIME
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

/// Run one future to completion on a runtime of its own.
///
/// The tests below are synchronous so they can hold the turn across everything they send
/// to the stand-in: every request it records is attributed to whichever run is in progress,
/// so a precondition read from another test interleaving with a drive would be read as that
/// drive's own traffic.
fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime for the journey")
        .block_on(future)
}

/// The GraphQL estimate the session is sized at, read from the same record the gate reads.
fn estimated() -> u64 {
    budget::estimate()[&Budget::Graphql]
}

/// Both admission reads — the free `GET /rate_limit` and GraphQL's own `rateLimit` object —
/// and the first real call, and nothing else: what a session declined on its second reading
/// has sent.
fn the_admission_reads_and_one_real_call() -> Vec<String> {
    vec![
        "GET /rate_limit".to_owned(),
        OBJECT_READ.to_owned(),
        "POST /graphql".to_owned(),
    ]
}

/// [`the_admission_reads_and_one_real_call`], and the two reads a recheck under the 40% rule makes
/// of the budgets again before it decides on the least of every reading.
fn and_the_recheck_reads() -> Vec<String> {
    let mut asked = the_admission_reads_and_one_real_call();
    asked.extend(["GET /rate_limit".to_owned(), OBJECT_READ.to_owned()]);
    asked
}

#[test]
fn a_journey_whose_first_call_carries_less_than_the_endpoint_claimed_does_not_run() {
    // The observed shape: the endpoint claimed the whole allowance, and the headers on the
    // session's own first real call reported one point short of the session and its buffer.
    let buffer = RETAINED_BUFFER.of(LIMIT);
    let estimate = budget::remaining_design_points(onetaskgraph_github_live::quota::Phase::Schema);
    let remaining = estimate + buffer - 1;
    let (drive, _turn) = drive(FirstCall::reporting(remaining));

    // Declined, not failed: the line leads with the run not having happened, and it is
    // not the schema verification reporting a drift.
    let message = drive.message();
    assert!(message.contains("DID NOT RUN"), "{message}");
    assert!(
        message.contains("not a test failure in the code under test"),
        "{message}"
    );
    assert!(!message.contains("mutation schema drifted"), "{message}");
    // Which reading declined it, and what each reading answered.
    for figure in [
        "the allowance read GET /rate_limit".to_owned(),
        format!("claimed {LIMIT} of {LIMIT} points remaining"),
        format!(
            "headers of contract schema introspection while retaining {estimate} points of remaining design demand"
        ),
        format!("reported {remaining} of {LIMIT} remaining"),
        format!("estimated to spend {estimate}"),
        format!("retained buffer is {buffer}"),
        // The second reading's own reset, which is the window that reading was about.
        format!("resets at {HEADERS_RESET_AT}"),
        "nothing here waits for it".to_owned(),
    ] {
        assert!(
            message.contains(&figure),
            "{figure} is missing from {message}"
        );
    }
    // The admission's reads and the one real call, and nothing else — the answer's own headers
    // already decline it, so the recheck reads nothing more — so nothing was written and there
    // is no partial artifact for anything to leave behind.
    assert_eq!(drive.asked, the_admission_reads_and_one_real_call());

    if std::env::var_os(budget::FOLLOW_THROUGH).is_some() {
        std::panic::resume_unwind(drive.panic);
    }
}

#[test]
fn a_journey_whose_first_call_is_refused_for_a_rate_limit_is_declined_rather_than_failed() {
    // What was observed on a real account, header for header: refused with the allowance
    // already exceeded, `remaining: 0`, and more used than the whole allowance — a set the
    // accounting refuses to record as figures, so what the re-check has is the refusal.
    let (drive, _turn) = drive(FirstCall::Answer {
        status: "403 Forbidden",
        headers: graphql_headers(0, LIMIT + 42),
        body: json!({"message": ALREADY_EXCEEDED}).to_string(),
    });
    let message = drive.message();
    assert!(message.contains("DID NOT RUN"), "{message}");
    assert!(!message.contains("mutation schema drifted"), "{message}");
    // The refusal is named as the rate-limited refusal it was, not as an ordinary one —
    // which is what reading the raw header buys — and no figure is invented for it.
    assert!(
        message.contains("it was refused for a rate limit"),
        "{message}"
    );
    assert!(
        message.contains("carried no allowance this session could read"),
        "{message}"
    );
    assert!(
        message.contains(&format!("claimed {LIMIT} of {LIMIT} points remaining")),
        "{message}"
    );
    // With nothing carried to say when that budget comes back, it is the first reading's
    // reset that is reported, and the message says whose it is.
    assert!(
        message.contains(&format!(
            "By the first reading that budget resets at {RESETS_AT}"
        )),
        "{message}"
    );
    assert_eq!(drive.asked, the_admission_reads_and_one_real_call());
}

#[test]
fn a_first_call_refused_with_the_budget_still_showing_room_is_declined_as_the_secondary_limiter() {
    // A refusal naming a rate limit while the headers still show a whole allowance is the
    // secondary limiter, which nothing reports and every further attempt extends: the
    // figures afford the session and the refusal declines it anyway, and it does not retry.
    let (drive, _turn) = drive(FirstCall::Answer {
        status: "403 Forbidden",
        headers: graphql_headers(LIMIT - 1, 1),
        body: json!({"message": "You have exceeded a secondary rate limit."}).to_string(),
    });
    let message = drive.message();
    assert!(message.contains("DID NOT RUN"), "{message}");
    assert!(
        message.contains(&format!(
            "it was refused for a rate limit, its headers reporting {} of {LIMIT} remaining",
            LIMIT - 1
        )),
        "{message}"
    );
    assert_eq!(drive.asked, the_admission_reads_and_one_real_call());
}

#[test]
fn an_answered_first_call_carrying_no_readable_allowance_declines_rather_than_assuming_the_claim() {
    // Two ways an ANSWER can carry no allowance this session may decide on: none of the
    // headers at all, and the observed shape on a call the budget still allowed — nothing
    // remaining beside more used than the whole allowance, which the accounting refuses to
    // record as figures. GitHub attaches these headers to every answer, so neither is an
    // answer the session can say it affords itself on.
    for headers in [String::new(), graphql_headers(0, LIMIT + 42)] {
        let (drive, _turn) = drive(FirstCall::Answer {
            status: "200 OK",
            headers: headers.clone(),
            body: json!({"data": {}}).to_string(),
        });
        let message = drive.message();
        assert!(message.contains("DID NOT RUN"), "{headers:?}: {message}");
        assert!(
            message.contains(
                "it was answered and its headers carried no allowance this session could read"
            ),
            "{headers:?}: {message}"
        );
        assert!(message.contains("not one it may assume"), "{message}");
        assert_eq!(drive.asked, the_admission_reads_and_one_real_call());
    }
}

#[test]
fn a_first_call_that_never_reached_the_host_fails_as_itself_rather_than_declining() {
    // The connection is closed without an answer, so the call carries no headers and was
    // not refused for a rate limit: that is no evidence about the budget, and reporting it
    // as a decline would say the code under test is not at fault about a journey that
    // could not reach its host. The session goes on with the endpoint's reading, and what
    // it runs into is the schema verification FAILING on the call that could not be made.
    let (drive, _turn) = drive(FirstCall::HangUp);
    let message = drive.message();
    assert!(message.contains("contract schema drifted"), "{message}");
    assert!(message.contains("could not reach GitHub"), "{message}");
    assert!(!message.contains("DID NOT RUN"), "{message}");
    // The stand-in recorded the call before hanging up on it, and the journey did not
    // retry it: one free read, one real call.
    assert_eq!(drive.asked, the_admission_reads_and_one_real_call());
}

#[test]
fn a_first_call_carrying_the_allowance_the_endpoint_claimed_lets_the_session_go_on() {
    // The headers agree with the endpoint, less the point that call itself cost. The
    // session goes on past the re-check: the stand-in refuses the next introspection
    // document, so what it goes on to is the schema verification FAILING — a failure, in
    // the code under test's own words, not a decline — and the stand-in was asked more
    // than the two requests a declined session sends.
    let (drive, _turn) = drive(FirstCall::reporting(LIMIT - 1));
    let message = drive.message();
    assert!(message.contains("contract schema drifted"), "{message}");
    assert!(!message.contains("DID NOT RUN"), "{message}");
    assert!(
        drive.asked.len() > 3,
        "a session the second reading affords goes on past its first real call: {:?}",
        drive.asked
    );
    assert_eq!(
        drive.asked[..3],
        the_admission_reads_and_one_real_call()[..]
    );
}

/// A recorded first real call, for the cases below that need no journey to reach.
fn recorded_first_call(outcome: Outcome, headers: &[(&str, &str)]) -> Request {
    let headers: Vec<(String, String)> = headers
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    Request::graphql(
        "query { __typename }",
        &json!({}),
        Some("contract schema introspection"),
        None,
    )
    .finished(
        outcome,
        RateLimit::read(|name| {
            headers
                .iter()
                .find(|(held, _)| held == name)
                .map(|(_, value)| value.clone())
        }),
    )
}

/// The pre-check admitted into `into` against the stand-in, which claims the whole allowance.
fn admitted_on_the_whole_allowance(into: &Accounting) -> budget::Admitted {
    block_on(budget::precondition("test-token", &STANDIN.host, into))
        .expect("a whole allowance admits this session on the first reading")
}

#[test]
fn the_second_reading_is_decided_on_the_call_recorded_right_after_the_allowance_read() {
    // Two sessions' worth of records in one accounting, which is what a stand-in driving
    // the journey several times in one binary produces: the re-check counts from its own
    // allowance read, so it reads the call that followed IT and not the first one ever
    // recorded — and not the one recorded last, either.
    let _turn = take_the_turn();
    let into = Accounting::new();
    let admitted = admitted_on_the_whole_allowance(&into);
    into.record(recorded_first_call(
        Outcome::Answered,
        &[
            ("x-ratelimit-limit", &LIMIT.to_string()),
            ("x-ratelimit-remaining", "0"),
            ("x-ratelimit-used", &LIMIT.to_string()),
            ("x-ratelimit-resource", "graphql"),
        ],
    ));
    let declined = block_on(budget::recheck(&admitted, &into))
        .expect_err("nothing left on the second reading declines the session");
    let Some(Unaffordable::Contradicted(readings)) = declined.unaffordable_because() else {
        panic!("a second reading that refuses is a contradiction: {declined:?}");
    };
    assert_eq!(readings.metered, budget::metered(Budget::Graphql));
    assert_eq!(
        readings.claimed,
        Allowance::read(LIMIT, LIMIT, RESETS_AT).expect("the claim")
    );
    assert_eq!(readings.claimed_by, budget::allowance_read());
    // The headers carried no reset, so the reading's is the window the claim named.
    assert_eq!(
        readings.carried,
        Ok(Allowance::read(LIMIT, 0, RESETS_AT).expect("nothing left"))
    );
    assert_eq!(
        readings.carried_by,
        budget::first_call_headers("contract schema introspection")
    );
    assert_eq!(readings.estimated_cost, estimated());
    assert_eq!(readings.retained_buffer, RETAINED_BUFFER.of(LIMIT));

    // A second admission into the same accounting reads the call after ITS read, however
    // much the record already held: the whole allowance again, this time carried.
    let again = admitted_on_the_whole_allowance(&into);
    into.record(recorded_first_call(
        Outcome::Answered,
        &[
            ("x-ratelimit-limit", &LIMIT.to_string()),
            ("x-ratelimit-remaining", &(LIMIT - 1).to_string()),
            ("x-ratelimit-used", "1"),
            ("x-ratelimit-resource", "graphql"),
        ],
    ));
    block_on(budget::recheck(&again, &into)).expect("the call after the second read affords it");
    // And the first admission, re-checked now, still reads the call that followed its own
    // read rather than the latest one.
    assert!(block_on(budget::recheck(&admitted, &into)).is_err());
}

#[test]
fn a_first_call_refused_for_something_other_than_a_rate_limit_is_decided_on_its_headers() {
    // A refusal that is not a rate limit — a credential GitHub rejected, an outage — carries
    // the account's headers like any response, and those are what decide. Where they afford
    // the session, the re-check lets the refusal surface as the failure it is rather than
    // dressing it as a budget the session did not have; where they do not, the session is
    // declined on them like any other.
    let _turn = take_the_turn();
    let into = Accounting::new();
    let admitted = admitted_on_the_whole_allowance(&into);
    into.record(recorded_first_call(
        Outcome::Refused,
        &[
            ("x-ratelimit-limit", &LIMIT.to_string()),
            ("x-ratelimit-remaining", &(LIMIT - 1).to_string()),
            ("x-ratelimit-used", "1"),
            ("x-ratelimit-reset", &HEADERS_RESET_AT.to_string()),
            ("x-ratelimit-resource", "graphql"),
        ],
    ));
    block_on(budget::recheck(&admitted, &into))
        .expect("headers with room let the refusal be a refusal");

    let admitted = admitted_on_the_whole_allowance(&into);
    into.record(recorded_first_call(
        Outcome::Refused,
        &[
            ("x-ratelimit-limit", &LIMIT.to_string()),
            ("x-ratelimit-remaining", "0"),
            ("x-ratelimit-used", &LIMIT.to_string()),
            ("x-ratelimit-resource", "graphql"),
        ],
    ));
    assert!(
        block_on(budget::recheck(&admitted, &into)).is_err(),
        "headers reporting nothing left decline the session whatever refused the call"
    );

    // And a refusal carrying no figures at all is no evidence about the budget: the session
    // goes on, and that refusal is its own failure to report.
    let admitted = admitted_on_the_whole_allowance(&into);
    into.record(recorded_first_call(Outcome::Refused, &[]));
    block_on(budget::recheck(&admitted, &into))
        .expect("a refusal that says nothing about the budget does not decline on it");
}

#[test]
fn a_reported_first_query_price_retires_demand_before_the_header_recheck() {
    let buffer = RETAINED_BUFFER.of(LIMIT);
    let initial = budget::remaining_design_points(onetaskgraph_github_live::quota::Phase::Schema);
    for (remaining, declined) in [(buffer + initial - 1, false), (buffer + initial - 2, true)] {
        let (drive, _turn) = drive(FirstCall::Answer {
            status: "200 OK",
            headers: graphql_headers(remaining, LIMIT - remaining),
            // The price is real response data; the absent schema fields deliberately make
            // an affordable run continue to the next schema request, before any fixture write.
            body: json!({"data":{"rateLimit":{"cost":1}}}).to_string(),
        });
        let message = drive.message();
        assert_eq!(message.contains("DID NOT RUN"), declined, "{message}");
        if declined {
            assert!(message.contains(&format!("estimated to spend {}", initial - 1)));
        } else {
            assert!(
                message.contains("GitHub contract schema drifted"),
                "{message}"
            );
        }
        // Declined on the answer's own headers with no further read; afforded, the recheck
        // reads the budgets again before the session goes on.
        let mut expected = if declined {
            the_admission_reads_and_one_real_call()
        } else {
            and_the_recheck_reads()
        };
        if !declined {
            expected.push("POST /graphql".into());
        }
        assert_eq!(drive.asked, expected);
    }
}
