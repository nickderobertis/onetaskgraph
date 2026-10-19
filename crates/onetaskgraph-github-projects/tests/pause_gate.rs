//! The 40% rule, against a loopback stand-in for GitHub on a virtual clock.
//!
//! A live session that finds a budget it draws on holding under 40% of its limit waits for
//! that budget's reset — unless the burn rate since the window began says it resets before
//! the session would exhaust it — and reads it again; at admission and at every recheck. What
//! is driven here is the gate itself, `onetaskgraph_live::Session::open_paced` and
//! `Session::recheck`, through this lane's own reads, `journey::budget::GitHubGauge`: `GET
//! /rate_limit`, GraphQL's own `rateLimit` object and the headers of that object's answer. The
//! stand-in reports the GraphQL budget differently on each of the three in every case, so only
//! the least of them is under or over 40% as the case needs, and it reports the whole
//! allowance again once the virtual clock has reached the reset — the window coming back.
//!
//! The clock is virtual: a wait completes at once and moves `now` by exactly what was waited,
//! so a pause of half an hour costs no real time and `budget::epoch_now` says how long the
//! session waited. No credential and no third-party API.

use std::io::{Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Duration;

use onetaskgraph_github_projects::accounting::{Accounting, Budget};
use onetaskgraph_live::{Credential, Exclusivity, Metered, PAUSE_BELOW, Session};
use serde_json::json;

// The credentialed lane's own halves again: `journey` for the gauge, the admission and the
// clock under test, and `lane` for the session name they report under. Most of each is for the
// target that drives the journey against GitHub, so what this one does not reach is the other
// drive's rather than dead code — the same reason `tests/budget_gate.rs` and
// `tests/recheck_gate.rs` carry these two.
#[allow(dead_code)]
mod journey;
#[allow(dead_code)]
mod lane;

use journey::budget;

/// The whole allowance of every budget the stand-in reports.
const LIMIT: u64 = 5_000;

/// What this session is estimated to spend of the GraphQL budget.
const ESTIMATE: u64 = 100;

/// The UTC epoch second the virtual clock starts at.
const STARTS_AT: u64 = 1_800_000_000;

/// A clock whose time moves only by what is waited on it.
#[derive(Default)]
struct Virtual {
    waited: Mutex<Duration>,
}

impl onetaskgraph_plugin_api::Clock for Virtual {
    fn now(&self) -> Duration {
        *self.waited.lock().unwrap()
    }

    fn sleep(
        &self,
        duration: Duration,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>> {
        *self.waited.lock().unwrap() += duration;
        Box::pin(std::future::ready(()))
    }
}

/// The one virtual clock every session in this binary waits on, put in place once.
static CLOCK: LazyLock<()> = LazyLock::new(|| {
    budget::wait_on(Arc::new(Virtual::default()), STARTS_AT);
});

/// One case at a time: the clock is one per binary, so a case's waits are its own only while
/// no other case is waiting beside it.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

fn take_the_turn() -> MutexGuard<'static, ()> {
    LazyLock::force(&CLOCK);
    ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What the stand-in reports of the GraphQL budget until its reset, on each of the three reads.
#[derive(Clone, Copy)]
struct Reported {
    /// `GET /rate_limit`'s `resources.graphql.remaining`.
    endpoint: u64,
    /// The rateLimit object's `remaining`.
    object: u64,
    /// The `x-ratelimit-remaining` header of the object's answer.
    headers: u64,
    /// The UTC epoch second all three say the budget resets.
    reset: u64,
}

/// The stand-in: `GET /rate_limit` and the rateLimit object, each reporting `reported` until
/// the virtual clock reaches its reset and the whole allowance from then on.
struct Standin {
    host: String,
    reported: Arc<Mutex<Reported>>,
    asked: Arc<Mutex<Vec<String>>>,
    /// Whether the budget stays where it is past every reset, as one another consumer keeps
    /// drawing down would — reporting the same remainder, against the next window's reset.
    stuck: Arc<std::sync::atomic::AtomicBool>,
}

impl Standin {
    fn start(reported: Reported) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a stand-in listener");
        let host = format!("http://{}", listener.local_addr().unwrap());
        let reported = Arc::new(Mutex::new(reported));
        let asked = Arc::new(Mutex::new(Vec::new()));
        let stuck = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (script, recorded) = (Arc::clone(&reported), Arc::clone(&asked));
        let held_down = Arc::clone(&stuck);
        thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = stream.expect("a stand-in connection");
                let request = whole_request(&mut stream);
                let line = request.lines().next().unwrap_or_default().to_owned();
                let mut parts = line.split(' ');
                let called = format!(
                    "{} {}",
                    parts.next().unwrap_or_default(),
                    parts.next().unwrap_or_default()
                );
                recorded.lock().unwrap().push(called.clone());
                let now = budget::epoch_now();
                let held = *script.lock().unwrap();
                // Past its reset the window has come back whole, and a new one has begun.
                let (scripted, reset) =
                    if now >= held.reset && held_down.load(std::sync::atomic::Ordering::SeqCst) {
                        let reset = now + 1_800;
                        (Reported { reset, ..held }, reset)
                    } else if now >= held.reset {
                        (
                            Reported {
                                endpoint: LIMIT,
                                object: LIMIT,
                                headers: LIMIT,
                                reset: now + 3_600,
                            },
                            now + 3_600,
                        )
                    } else {
                        (held, held.reset)
                    };
                let (status, headers, body) = match called.as_str() {
                    "GET /rate_limit" => (
                        "200 OK",
                        format!(
                            "x-ratelimit-limit: {LIMIT}\r\nx-ratelimit-remaining: {LIMIT}\r\n\
                             x-ratelimit-used: 0\r\nx-ratelimit-resource: core\r\n"
                        ),
                        budget::documented_answer(LIMIT, scripted.endpoint, LIMIT, reset)
                            .to_string(),
                    ),
                    "POST /graphql" if request.contains(budget::RATE_LIMIT_OBJECT) => (
                        "200 OK",
                        format!(
                            "x-ratelimit-limit: {LIMIT}\r\nx-ratelimit-remaining: {}\r\n\
                             x-ratelimit-used: {}\r\nx-ratelimit-reset: {reset}\r\n\
                             x-ratelimit-resource: graphql\r\n",
                            scripted.headers,
                            LIMIT - scripted.headers
                        ),
                        json!({"data": {"rateLimit": {
                            "limit": LIMIT,
                            "remaining": scripted.object,
                            "resetAt": chrono::DateTime::from_timestamp(
                                i64::try_from(reset).unwrap(), 0).unwrap().to_rfc3339(),
                        }}})
                        .to_string(),
                    ),
                    _ => ("404 Not Found", String::new(), "{}".to_owned()),
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
            reported,
            asked,
            stuck,
        }
    }

    /// Keep the budget where it is past every reset from now on.
    fn hold_down(&self) {
        self.stuck.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Report `reported` from now on — the bucket moving under a session already admitted.
    fn report(&self, reported: Reported) {
        *self.reported.lock().unwrap() = reported;
    }

    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
}

/// One whole request off `stream`: its head, and its body to its `Content-Length`.
fn whole_request(stream: &mut TcpStream) -> String {
    let mut held = Vec::new();
    let mut chunk = [0_u8; 16_384];
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

/// The two reads one decision under the rule makes of the budgets.
const ONE_READ: [&str; 2] = ["GET /rate_limit", "POST /graphql"];

fn graphql() -> Metered {
    budget::metered(Budget::Graphql)
}

fn demands() -> [(Metered, u64); 2] {
    [(graphql(), ESTIMATE), (budget::metered(Budget::Rest), 1)]
}

fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime")
        .block_on(future)
}

/// Open a session against `standin` through this lane's gauge, recording into `into`.
fn open(standin: &Standin, into: &Accounting) -> Session {
    let mut gauge = budget::GitHubGauge::new("test-token", &standin.host, into);
    block_on(Session::open_paced(
        "pause-gate",
        Credential::new("test-token").unwrap(),
        Exclusivity::Shared,
        &mut gauge,
        &demands(),
    ))
    .unwrap_or_else(|declined| panic!("{}", declined.message()))
}

/// Recheck `session` against `standin`.
fn recheck(session: &mut Session, standin: &Standin, into: &Accounting) {
    let mut gauge = budget::GitHubGauge::new("test-token", &standin.host, into);
    block_on(session.recheck(&mut gauge, &[(graphql(), ESTIMATE)]))
        .unwrap_or_else(|declined| panic!("{}", declined.message()));
}

/// Thirty per cent, under the rule's forty, as one of three readings that disagree: the
/// endpoint and the object both report more than forty per cent and only the least — `low`,
/// on whichever read `on` names — is under it.
fn only_one_under(low: u64, on: &str, reset: u64) -> Reported {
    let high = PAUSE_BELOW.of(LIMIT) + 900;
    Reported {
        endpoint: if on == "endpoint" { low } else { high },
        object: if on == "object" { low } else { high + 200 },
        headers: if on == "headers" { low } else { high + 400 },
        reset,
    }
}

/// The read each stand-in field is named after, as the gauge names it.
fn named(on: &str) -> String {
    match on {
        "endpoint" => budget::allowance_read(),
        "object" => budget::RATE_LIMIT_OBJECT_READ.to_owned(),
        _ => format!("the headers of {}", budget::RATE_LIMIT_OBJECT_READ),
    }
}

#[test]
fn admission_under_forty_per_cent_waits_for_the_reset_decided_on_the_least_reading() {
    let _turn = take_the_turn();
    // Every one of the three reads in turn is the least, and only it is under the rule.
    for on in ["endpoint", "object", "headers"] {
        let started = budget::epoch_now();
        // Thirty per cent left, half an hour before the reset, 3,500 used in the half hour
        // since the window began: at that rate it is gone long before the reset.
        let reset = started + 1_800;
        let low = LIMIT * 30 / 100;
        let standin = Standin::start(only_one_under(low, on, reset));
        let into = Accounting::new();
        let session = open(&standin, &into);

        assert_eq!(
            budget::epoch_now(),
            reset,
            "{on}: waited until the reported reset"
        );
        let pauses = session.pauses();
        assert_eq!(pauses.len(), 1, "{on}: {pauses:?}");
        assert_eq!(pauses[0].metered, graphql());
        assert_eq!(
            pauses[0].decided_on.remaining(),
            low,
            "{on}: decided on the least"
        );
        assert_eq!(pauses[0].by, named(on), "{on}");
        assert_eq!((pauses[0].from, pauses[0].until), (started, reset));
        // Read once, then read again after the wait.
        assert_eq!(standin.asked(), [ONE_READ, ONE_READ].concat(), "{on}");
        let report = session.report();
        assert!(
            report.contains("paused 1800 s for the graphql budget"),
            "{report}"
        );
        assert!(report.contains(&named(on)), "{report}");
        assert!(
            report.contains(&format!("{low} of {LIMIT} points remaining")),
            "{report}"
        );
    }
}

#[test]
fn admission_under_forty_per_cent_goes_on_when_the_burn_rate_resets_first() {
    let _turn = take_the_turn();
    for on in ["endpoint", "object", "headers"] {
        let started = budget::epoch_now();
        // Thirty per cent left a minute before the reset, 3,500 used over the 59 minutes since
        // the window began: about 60 more by the reset at that rate, and the 100 this session
        // needs, are well under the 1,500 left — so it resets before it would run out.
        let low = LIMIT * 30 / 100;
        let standin = Standin::start(only_one_under(low, on, started + 60));
        let into = Accounting::new();
        let session = open(&standin, &into);

        assert_eq!(budget::epoch_now(), started, "{on}: no wait at all");
        assert!(session.pauses().is_empty(), "{on}: {:?}", session.pauses());
        assert_eq!(standin.asked(), ONE_READ, "{on}: read once, and not again");
        assert!(
            session.report().contains("took no pause"),
            "{}",
            session.report()
        );
    }
}

#[test]
fn admission_at_or_above_forty_per_cent_goes_on_at_once() {
    let _turn = take_the_turn();
    let started = budget::epoch_now();
    // The least of the three is exactly forty per cent; the others are fuller still.
    let at = PAUSE_BELOW.of(LIMIT);
    let standin = Standin::start(Reported {
        endpoint: at + 1_000,
        object: at + 500,
        headers: at,
        reset: started + 1_800,
    });
    let into = Accounting::new();
    let session = open(&standin, &into);
    assert_eq!(budget::epoch_now(), started);
    assert!(session.pauses().is_empty(), "{:?}", session.pauses());
    assert_eq!(standin.asked(), ONE_READ);
}

#[test]
fn a_recheck_finding_a_bucket_under_forty_per_cent_pauses_under_the_same_rule() {
    let _turn = take_the_turn();
    for on in ["endpoint", "object", "headers"] {
        let started = budget::epoch_now();
        // Admitted on readings all above forty per cent.
        let standin = Standin::start(Reported {
            endpoint: 4_000,
            object: 3_800,
            headers: 3_600,
            reset: started + 3_000,
        });
        let into = Accounting::new();
        let mut session = open(&standin, &into);
        assert!(session.pauses().is_empty());

        // Then the bucket is drawn down while the session runs: only one read says so.
        let reset = started + 1_800;
        let low = LIMIT * 25 / 100;
        standin.report(only_one_under(low, on, reset));
        recheck(&mut session, &standin, &into);

        assert_eq!(
            budget::epoch_now(),
            reset,
            "{on}: the recheck waited until the reset"
        );
        let pauses = session.pauses();
        assert_eq!(pauses.len(), 1, "{on}: {pauses:?}");
        assert_eq!(pauses[0].decided_on.remaining(), low, "{on}");
        assert_eq!(pauses[0].by, named(on), "{on}");
        // Admission's read, the recheck's, and the recheck's again after its wait.
        assert_eq!(
            standin.asked(),
            [ONE_READ, ONE_READ, ONE_READ].concat(),
            "{on}"
        );
        assert!(
            session.report().contains("paused 1800 s"),
            "{}",
            session.report()
        );
    }
}

#[test]
fn a_recheck_under_forty_per_cent_goes_on_when_the_burn_rate_resets_first() {
    let _turn = take_the_turn();
    for on in ["endpoint", "object", "headers"] {
        let started = budget::epoch_now();
        let standin = Standin::start(Reported {
            endpoint: 4_000,
            object: 3_800,
            headers: 3_600,
            reset: started + 3_000,
        });
        let into = Accounting::new();
        let mut session = open(&standin, &into);
        // Under forty per cent a minute before the reset, at a rate that leaves it short of
        // empty by then.
        standin.report(only_one_under(LIMIT * 25 / 100, on, started + 60));
        recheck(&mut session, &standin, &into);
        assert_eq!(budget::epoch_now(), started, "{on}: no wait");
        assert!(session.pauses().is_empty(), "{on}: {:?}", session.pauses());
        assert_eq!(standin.asked(), [ONE_READ, ONE_READ].concat(), "{on}");
    }
}

#[test]
fn the_lanes_own_admission_pauses_under_the_same_rule() {
    // `journey::budget::precondition`, which the credentialed journey opens every session
    // through, decides by the same admission as `Session::open_paced`.
    let _turn = take_the_turn();
    let started = budget::epoch_now();
    let reset = started + 1_800;
    let standin = Standin::start(only_one_under(LIMIT * 30 / 100, "headers", reset));
    let into = Accounting::new();
    let admitted = block_on(budget::precondition("test-token", &standin.host, &into))
        .unwrap_or_else(|declined| panic!("{}", declined.message()));
    assert_eq!(budget::epoch_now(), reset);
    assert_eq!(admitted.pauses().len(), 1);
    assert!(
        admitted.report().contains("paused 1800 s"),
        "{}",
        admitted.report()
    );
    assert_eq!(standin.asked(), [ONE_READ, ONE_READ].concat());
}

#[test]
fn the_lanes_own_recheck_pauses_on_a_real_answers_headers_and_decides_after_the_reset() {
    // `journey::budget::recheck_remaining`, which every phase of the credentialed journey
    // calls: admitted above forty per cent, then the session's own last real answer carries
    // headers under it — the least of every reading — so the recheck waits for that answer's
    // reset, reads the budget again, and decides on what it reads after the window came back.
    let _turn = take_the_turn();
    let started = budget::epoch_now();
    let standin = Standin::start(Reported {
        endpoint: 4_000,
        object: 3_800,
        headers: 3_600,
        reset: started + 3_000,
    });
    let into = Accounting::new();
    let admitted = block_on(budget::precondition("test-token", &standin.host, &into))
        .unwrap_or_else(|declined| panic!("{}", declined.message()));
    assert!(admitted.pauses().is_empty());

    let reset = started + 1_800;
    let low = LIMIT * 20 / 100;
    into.record(
        onetaskgraph_github_projects::accounting::Request::graphql(
            "query { __typename }",
            &json!({}),
            Some("a real answer"),
            None,
        )
        .finished(
            onetaskgraph_github_projects::accounting::Outcome::Answered,
            onetaskgraph_github_projects::accounting::RateLimit::read(|name| match name {
                "x-ratelimit-limit" => Some(LIMIT.to_string()),
                "x-ratelimit-remaining" => Some(low.to_string()),
                "x-ratelimit-used" => Some((LIMIT - low).to_string()),
                "x-ratelimit-reset" => Some(reset.to_string()),
                "x-ratelimit-resource" => Some("graphql".to_owned()),
                _ => None,
            }),
        ),
    );
    block_on(budget::recheck_remaining(&admitted, &into, ESTIMATE))
        .unwrap_or_else(|declined| panic!("{}", declined.message()));

    assert_eq!(
        budget::epoch_now(),
        reset,
        "waited until the answer's reset"
    );
    let pauses = admitted.pauses();
    assert_eq!(pauses.len(), 1, "{pauses:?}");
    assert_eq!(pauses[0].decided_on.remaining(), low);
    assert!(
        pauses[0].by.contains("headers of a real answer"),
        "{}",
        pauses[0].by
    );
    // Admission's read, the recheck's, and the recheck's again after its wait.
    assert_eq!(standin.asked(), [ONE_READ, ONE_READ, ONE_READ].concat());
    assert!(
        admitted.report().contains("paused 1800 s"),
        "{}",
        admitted.report()
    );
}

#[test]
fn a_budget_that_never_comes_back_is_waited_for_a_bounded_number_of_times_then_declined() {
    // Ten per cent, half an hour from its reset, and still ten per cent after every reset: the
    // session pauses `MOST_PAUSES` times, reading again after each, and then the retained
    // buffer decides on the last reading — which it cannot afford, so it does not run.
    let _turn = take_the_turn();
    let started = budget::epoch_now();
    let low = LIMIT / 10;
    let standin = Standin::start(Reported {
        endpoint: low,
        object: low,
        headers: low,
        reset: started + 1_800,
    });
    standin.hold_down();
    let into = Accounting::new();
    let mut gauge = budget::GitHubGauge::new("test-token", &standin.host, &into);
    let declined = block_on(Session::open_paced(
        "pause-gate",
        Credential::new("test-token").unwrap(),
        Exclusivity::Shared,
        &mut gauge,
        &demands(),
    ))
    .expect_err("a budget that never comes back is not one this session can afford");
    let pauses = u64::try_from(onetaskgraph_live::pace::MOST_PAUSES).unwrap();
    assert_eq!(
        budget::epoch_now(),
        started + pauses * 1_800,
        "bounded waiting"
    );
    assert_eq!(
        standin.asked().len(),
        ONE_READ.len() * (onetaskgraph_live::pace::MOST_PAUSES + 1),
        "read again after every pause, and no more"
    );
    assert!(
        matches!(
            declined.unaffordable_because(),
            Some(onetaskgraph_live::Unaffordable::Short { remaining, .. }) if *remaining == low
        ),
        "{}",
        declined.message()
    );
}
