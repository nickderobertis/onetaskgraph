//! The simulated clock, run with real client processes.
//!
//! Each client here is a process of its own — this test binary re-run as
//! [`clock_client_process_entry`] — attached through [`SimulatedClock::client_env`] and the
//! same `onetaskgraph_core::process_clock` the binary settles its clock with, so what these
//! prove about virtual time is what a journey whose spawned binary paces on it observes. The
//! last two drive the compiled binary itself.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use onetaskgraph_core::Environment;

use crate::common::clock::SimulatedClock;
use crate::common::images;
use crate::common::{Sandbox, SourceBoundary, one_source, stderr, stdout};

/// The test-only variable naming which scenario a client process runs.
const SCENARIO: &str = "CLOCK_CLIENT_SCENARIO";
/// The test-only variable naming the loopback endpoint a client process sends its request to.
const ENDPOINT: &str = "CLOCK_CLIENT_ENDPOINT";
/// The size of the body a client process sends, in bytes.
const BODY: &str = "CLOCK_CLIENT_BODY";
/// What a client process prefixes each virtual time it reports with.
const REPORT: &str = "clock-report ";

/// The entry point of the client processes the tests below spawn; run on its own, it has no
/// scenario and returns.
///
/// A scenario is a short script over the process clock — sleep, compute, send one request —
/// that reports the virtual time it reads at each step on standard output.
#[test]
fn clock_client_process_entry() {
    let Ok(scenario) = std::env::var(SCENARIO) else {
        return;
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    runtime.block_on(async {
        let clock = onetaskgraph_core::process_clock(&Environment::from_process())
            .expect("the process clock attaches");
        let report = |label: &str| println!("{REPORT}{label} {}", clock.now().as_nanos());
        for step in scenario.split(',') {
            match step.split_once(':') {
                Some(("sleep", seconds)) => {
                    clock
                        .sleep(Duration::from_secs(seconds.parse().expect("seconds")))
                        .await;
                    report(step);
                }
                Some(("compute", millis)) => {
                    let until = Instant::now() + Duration::from_millis(millis.parse().expect("ms"));
                    while Instant::now() < until {
                        std::hint::spin_loop();
                    }
                    report(step);
                }
                _ if step == "request" => {
                    send_request();
                    report(step);
                }
                Some(("request-while-sleeping", seconds)) => {
                    // One request sent from another thread while this one sleeps on the
                    // clock: the handler takes the request, the sleep is registered, and only
                    // then is the body sent. The client counts as waiting throughout, so only
                    // the handler's own computing can hold virtual time while the body is
                    // received.
                    let (taken, held) = std::sync::mpsc::channel();
                    let (go, going) = std::sync::mpsc::channel::<()>();
                    let sending = std::thread::spawn(move || {
                        send_request_after(|| {
                            let _ = taken.send(());
                            going.recv().expect("the sleep is registered");
                        });
                    });
                    held.recv().expect("the handler took the request");
                    let sleeping =
                        clock.sleep(Duration::from_secs(seconds.parse().expect("seconds")));
                    // The coordinator answers this only once it has read the sleep before it,
                    // so the sleep is registered by the time the body leaves.
                    let _ = clock.now();
                    go.send(()).expect("the sender is waiting");
                    sleeping.await;
                    sending.join().expect("the request completes");
                    report(step);
                }
                _ => panic!("unknown scenario step {step:?}"),
            }
        }
    });
}

/// Send one request of the configured body to the configured endpoint and wait for its answer.
fn send_request() {
    send_request_after(|| ());
}

/// [`send_request`], calling `taken` once the handler has taken the request and before the
/// body is sent.
fn send_request_after(taken: impl FnOnce()) {
    let size: usize = std::env::var(BODY).map_or(0, |size| size.parse().expect("a size"));
    let body = if size == 0 {
        Vec::new()
    } else {
        images::png(7, size)
    };
    let mut stream = TcpStream::connect(std::env::var(ENDPOINT).expect("an endpoint"))
        .expect("the endpoint accepts");
    let mut ack = [0_u8; 1];
    stream
        .read_exact(&mut ack)
        .expect("the handler takes the request");
    taken();
    stream
        .write_all(&u32::try_from(body.len()).expect("fits").to_be_bytes())
        .and_then(|()| stream.write_all(&body))
        .expect("the request is sent");
    let mut answer = [0_u8; 1];
    stream.read_exact(&mut answer).expect("the answer arrives");
}

/// One client process running `scenario` on `clock` as client `client`, its request — if it
/// sends one — going to `endpoint`.
fn client(
    clock: &SimulatedClock,
    client: usize,
    scenario: &str,
    endpoint: Option<&str>,
    body: usize,
) -> std::process::Child {
    let mut command = Command::new(std::env::current_exe().expect("this test binary"));
    command
        .args([
            "--exact",
            "clock::clock_client_process_entry",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(SCENARIO, scenario)
        .env(BODY, body.to_string())
        .envs(clock.client_env(client))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    if let Some(endpoint) = endpoint {
        command.env(ENDPOINT, endpoint);
    }
    command.spawn().expect("the client process starts")
}

/// The virtual times one client reported, each beside the step it reported after.
type Reports = Vec<(String, Duration)>;

/// The virtual times a finished client reported, by step.
fn reported(output: &Output) -> Reports {
    assert!(
        output.status.success(),
        "the client process failed: {}\n{}",
        stdout(output),
        stderr(output)
    );
    stdout(output)
        .lines()
        .filter_map(|line| line.split_once(REPORT).map(|(_, rest)| rest))
        .map(|rest| {
            let (label, nanos) = rest.rsplit_once(' ').expect("a label and a time");
            (
                label.to_owned(),
                Duration::from_nanos(nanos.parse().expect("nanoseconds")),
            )
        })
        .collect()
}

/// A loopback endpoint for client `client` that holds each request's answer for `hold` of
/// virtual time, reporting the virtual time at which each body had been received whole.
///
/// Its wire: once it has taken a request it writes one byte; the client then sends a
/// four-byte big-endian length and the body, and the endpoint answers with one byte.
fn endpoint(
    clock: &SimulatedClock,
    client: usize,
    hold: Duration,
) -> (String, std::sync::mpsc::Receiver<(usize, Duration)>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback listener");
    let address = listener.local_addr().expect("its address").to_string();
    let (sender, received) = std::sync::mpsc::channel();
    let clock = clock.clone();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime");
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let request = clock.request(client);
            stream.write_all(&[0]).expect("the request is taken");
            let mut length = [0_u8; 4];
            stream.read_exact(&mut length).expect("the length");
            let mut body = vec![0; u32::from_be_bytes(length) as usize];
            stream.read_exact(&mut body).expect("the body");
            let _ = sender.send((body.len(), clock.now()));
            runtime.block_on(request.delay(hold));
            stream.write_all(&[1]).expect("the answer");
            drop(request);
        }
    });
    (address, received)
}

#[test]
fn a_sixty_second_sleep_completes_after_exactly_sixty_virtual_seconds_and_no_real_wait() {
    let clock = SimulatedClock::start(1);
    let started = Instant::now();
    let output = client(&clock, 0, "sleep:60", None, 0)
        .wait_with_output()
        .expect("the client finishes");
    let elapsed = started.elapsed();
    assert_eq!(
        reported(&output),
        vec![("sleep:60".to_owned(), Duration::from_secs(60))]
    );
    assert_eq!(clock.now(), Duration::from_secs(60));
    assert!(
        elapsed < Duration::from_secs(1),
        "a sixty-second sleep took {elapsed:?} of real time"
    );
}

#[test]
fn a_handlers_delay_holds_its_answer_by_exactly_the_virtual_time_asked() {
    let clock = SimulatedClock::start(1);
    let (address, received) = endpoint(&clock, 0, Duration::from_secs(7));
    let output = client(&clock, 0, "request,sleep:3", Some(&address), 0)
        .wait_with_output()
        .expect("the client finishes");
    assert_eq!(
        reported(&output),
        vec![
            ("request".to_owned(), Duration::from_secs(7)),
            ("sleep:3".to_owned(), Duration::from_secs(10)),
        ]
    );
    assert_eq!(
        received.recv().expect("the handler saw the request"),
        (0, Duration::ZERO)
    );
}

/// Two clients: one sleeps ten seconds at once; the other computes, sends a 500 KB body to a
/// handler that holds its answer two seconds, then sleeps five. Every virtual time each
/// reports, and the time at which the handler had the body whole.
fn two_clients() -> (Reports, Reports, (usize, Duration)) {
    let clock = SimulatedClock::start(2);
    let (address, received) = endpoint(&clock, 1, Duration::from_secs(2));
    let sleeper = client(&clock, 0, "sleep:10", None, 0);
    let worker = client(
        &clock,
        1,
        "compute:300,request,sleep:5",
        Some(&address),
        500_000,
    );
    let worker = worker.wait_with_output().expect("the worker finishes");
    let sleeper = sleeper.wait_with_output().expect("the sleeper finishes");
    (
        reported(&sleeper),
        reported(&worker),
        received.recv().expect("the handler saw the request"),
    )
}

#[test]
fn virtual_time_waits_for_every_client_and_handler_to_be_waiting_before_it_advances() {
    let started = Instant::now();
    let (sleeper, worker, body) = two_clients();
    // The sleeper waited from the start, but nothing advanced while the worker computed for
    // 300 ms of real time, nor while the handler received a 500 KB body: had it, the sleeper's
    // ten seconds would have passed first and every one of the worker's times would be later.
    assert_eq!(
        worker,
        vec![
            ("compute:300".to_owned(), Duration::ZERO),
            ("request".to_owned(), Duration::from_secs(2)),
            ("sleep:5".to_owned(), Duration::from_secs(7)),
        ]
    );
    let (received, at) = body;
    assert!((475_000..=525_000).contains(&received), "{received} bytes");
    assert_eq!(at, Duration::ZERO);
    // Once both were waiting it advanced, to each wake-up in turn.
    assert_eq!(
        sleeper,
        vec![("sleep:10".to_owned(), Duration::from_secs(10))]
    );
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[test]
fn a_handler_receiving_a_body_holds_virtual_time_while_its_client_sleeps() {
    let clock = SimulatedClock::start(1);
    let (address, received) = endpoint(&clock, 0, Duration::from_secs(2));
    let output = client(
        &clock,
        0,
        "request-while-sleeping:10",
        Some(&address),
        500_000,
    )
    .wait_with_output()
    .expect("the client finishes");
    // The client's ten-second sleep was pending throughout, so had the handler's receiving
    // not held virtual time, the body would have been whole only after it had passed.
    let (bytes, at) = received.recv().expect("the handler saw the request");
    assert!((475_000..=525_000).contains(&bytes), "{bytes} bytes");
    assert_eq!(at, Duration::ZERO);
    assert_eq!(
        reported(&output),
        vec![(
            "request-while-sleeping:10".to_owned(),
            Duration::from_secs(10)
        )]
    );
}

#[test]
fn two_runs_of_one_scenario_report_the_same_virtual_times() {
    assert_eq!(two_clients(), two_clients());
}

#[test]
fn the_binary_given_a_clients_environment_attaches_as_that_client_and_without_it_to_nothing() {
    let sandbox = Sandbox::new();
    sandbox.project_document(&one_source(SourceBoundary::Direct));

    let clock = SimulatedClock::start(2);
    let attached = sandbox
        .command()
        .envs(clock.client_env(1))
        .args(["sources", "list"])
        .output()
        .expect("the binary runs");
    assert!(attached.status.success(), "{}", stderr(&attached));
    assert_eq!(clock.attached(), vec![1]);

    let unattached = SimulatedClock::start(1);
    let output = sandbox
        .command()
        .args(["sources", "list"])
        .output()
        .expect("the binary runs");
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), stdout(&attached));
    assert!(unattached.attached().is_empty());
}

#[test]
fn the_binary_refuses_a_simulated_clock_it_cannot_read_or_reach_naming_the_variable() {
    let sandbox = Sandbox::new();
    sandbox.project_document(&one_source(SourceBoundary::Direct));
    let clock = SimulatedClock::start(1);
    let (variable, _) = clock.client_env(0).remove(0);
    for (value, problem) in [
        ("not-an-address", "is not `<address>/<client>`"),
        ("127.0.0.1:1/0", "could not be reached"),
    ] {
        let output = sandbox
            .command()
            .env(&variable, value)
            .args(["sources", "list"])
            .output()
            .expect("the binary runs");
        assert_eq!(output.status.code(), Some(1), "{value}");
        let said = stderr(&output);
        assert!(said.contains(&variable) && said.contains(problem), "{said}");
    }
}
