//! What a whole plan copy spends learning who can read a board declared private: the recording
//! behind the `visibility-write-requests` budget.
//!
//! The workload is the realistic one a plan copy is: one project, its 100 tasks — each
//! depending on the one before it — and its design document, 102 writes in all, copied out of
//! a private folder of Markdown onto one GitHub board declared private, so every write reads
//! the Project's visibility and the visibility of the repository its issue is created in. The
//! loopback board answers each of those reads after 0.38 simulated seconds — what one took on
//! the real API — and every other request at once, with a write's two reads overlapping, as the
//! plugin sends them together. The journey records the reads the whole copy spent, preflights
//! included, with their breakdown per write, by write kind (project, task, document) and by
//! read kind (Project, issue repository), and the simulated seconds they added, which is
//! telemetry with no threshold of its own; `budget_runner::report_visibility_reads` reports the
//! figure with that breakdown and those seconds as its detail, and runs no copy.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Output;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use onetaskgraph_e2e_support::binary;
use onetaskgraph_e2e_support::clock::SimulatedClock;
use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{document, github_projects_with_board};

/// How long one visibility read took on the real API.
const READ: Duration = Duration::from_millis(380);
/// The tasks of the plan copied.
const TASKS: usize = 100;
/// The writes the copy makes: the project, its tasks and its design document.
pub(super) const WRITES: u64 = TASKS as u64 + 2;
/// The budget this journey records for.
pub(super) const BUDGET: &str = "visibility-write-requests";
/// The workload this journey records, which the report refuses any other recording for.
pub(super) const WORKLOAD: &str = "plan-copy-1-project-100-tasks-1-document";

/// The visibility reads in flight and not yet paired, so a write's two start their delay
/// together: a read held here counts as computing, and virtual time does not move.
#[derive(Default)]
struct Pairing {
    waiting: usize,
    generation: u64,
    /// The virtual intervals the reads were delayed over, one per pair.
    intervals: Vec<(Duration, Duration)>,
}

/// A loopback in front of the fixture board, delaying each visibility read on the simulated
/// clock and passing every other request through at once.
struct VisibilityBoard {
    endpoint: String,
    pairing: Arc<(Mutex<Pairing>, Condvar)>,
}

impl VisibilityBoard {
    fn new(upstream: &str, clock: &SimulatedClock) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback listener");
        let endpoint = format!(
            "http://{}/graphql",
            listener.local_addr().expect("an address")
        );
        let upstream = upstream
            .strip_prefix("http://")
            .and_then(|rest| rest.split('/').next())
            .expect("the fixture board's address")
            .to_owned();
        let pairing = Arc::new((Mutex::new(Pairing::default()), Condvar::new()));
        let held = Arc::clone(&pairing);
        let clock = clock.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let (clock, upstream, held) = (clock.clone(), upstream.clone(), Arc::clone(&held));
                std::thread::spawn(move || serve(stream, &clock, &upstream, &held));
            }
        });
        Self { endpoint, pairing }
    }

    /// The simulated seconds the visibility reads held the copy for.
    fn seconds(&self) -> f64 {
        let pairing = self.pairing.0.lock().expect("not poisoned");
        let mut intervals = pairing.intervals.clone();
        intervals.sort_unstable();
        let mut total = Duration::ZERO;
        let mut reach = Duration::ZERO;
        for (start, end) in intervals {
            let start = start.max(reach);
            if end > start {
                total += end - start;
            }
            reach = reach.max(end);
        }
        total.as_secs_f64()
    }
}

fn serve(
    mut stream: TcpStream,
    clock: &SimulatedClock,
    upstream: &str,
    pairing: &Arc<(Mutex<Pairing>, Condvar)>,
) {
    let request = clock.request(0);
    let raw = read_request(&mut stream);
    let text = String::from_utf8_lossy(&raw);
    let body = text.split_once("\r\n\r\n").map_or("", |(_, body)| body);
    let visibility = text.starts_with("GET /repos/")
        || serde_json::from_str::<Value>(body).is_ok_and(|request| {
            request["query"] == onetaskgraph_github_projects::graphql::PROJECT_VISIBILITY
        });
    let mut connection = TcpStream::connect(upstream).expect("the fixture board");
    connection.write_all(&raw).expect("forwarded");
    let mut response = Vec::new();
    connection.read_to_end(&mut response).expect("answered");
    if visibility {
        let (lock, paired) = &**pairing;
        let mut state = lock.lock().expect("not poisoned");
        state.waiting += 1;
        if state.waiting == 2 {
            state.waiting = 0;
            state.generation += 1;
            paired.notify_all();
        } else {
            let generation = state.generation;
            let (held, _) = paired
                .wait_timeout_while(state, Duration::from_secs(5), |state| {
                    state.generation == generation
                })
                .expect("not poisoned");
            state = held;
            if state.generation == generation {
                // Alone after all: it is delayed alone.
                state.waiting = 0;
            }
        }
        drop(state);
        let started = clock.now();
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("a runtime")
            .block_on(request.delay(READ));
        lock.lock()
            .expect("not poisoned")
            .intervals
            .push((started, clock.now()));
        stream.write_all(&response).expect("the answer");
        return;
    }
    stream.write_all(&response).expect("the answer");
    drop(request);
}

/// One whole HTTP request, its head and its declared body.
fn read_request(stream: &mut TcpStream) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 8192];
    let head = loop {
        let count = stream.read(&mut chunk).expect("a request");
        assert!(count > 0, "a request ended before its headers");
        bytes.extend_from_slice(&chunk[..count]);
        if let Some(at) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break at + 4;
        }
    };
    let headers = String::from_utf8_lossy(&bytes[..head]).to_ascii_lowercase();
    let length = headers
        .lines()
        .find_map(|line| line.strip_prefix("content-length: "))
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    while bytes.len() < head + length {
        let count = stream.read(&mut chunk).expect("a request body");
        assert!(count > 0, "a request ended before its body");
        bytes.extend_from_slice(&chunk[..count]);
    }
    // The fixture board answers one request per connection and closes it.
    let text = String::from_utf8_lossy(&bytes[..head])
        .replace("connection: keep-alive", "connection: close");
    let mut forwarded = text.into_bytes();
    forwarded.extend_from_slice(&bytes[head..]);
    forwarded
}

fn author(root: &std::path::Path) {
    let write = |path: &str, text: String| {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("a folder");
        std::fs::write(path, text).expect("a record");
    };
    write(
        "projects/plan.md",
        "---\ntitle: Plan\nstatus: todo\n---\nThe plan.\n".to_owned(),
    );
    for index in 0..TASKS {
        let depends = if index == 0 {
            String::new()
        } else {
            format!("depends_on: [t-{:03}]\n", index - 1)
        };
        write(
            &format!("tasks/t-{index:03}.md"),
            format!(
                "---\ntitle: Task {index:03}\nstatus: todo\nproject: plan\n{depends}---\n\
                 Step {index} of the plan.\n"
            ),
        );
    }
    write(
        "documents/design.md",
        "---\ntitle: Plan design\nproject: plan\n---\nHow the plan fits together.\n".to_owned(),
    );
}

fn copy(sandbox: &Sandbox, clock: &SimulatedClock, kind: &str, id: &str) -> Output {
    let output = sandbox
        .subprocess(binary())
        .args([kind, "copy", id, "--to", "board", "--json"])
        .envs(clock.client_env(0))
        .output()
        .expect("the binary runs");
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}\n{}",
        stdout(&output),
        stderr(&output)
    );
    output
}

/// Which write each visibility read was spent on, in the order the board served them: every
/// read before a write's first mutation is that write's.
fn attributed(served: &[(String, Value)]) -> Vec<(&'static str, &'static str)> {
    let mut reads = Vec::new();
    let mut pending = Vec::new();
    for (document, variables) in served {
        if document == onetaskgraph_github_projects::graphql::PROJECT_VISIBILITY {
            pending.push("project");
        } else if document.starts_with("GET /repos/") {
            pending.push("repository");
        } else if document.contains("createIssue(input:$input)") {
            let title = variables["input"]["title"].as_str().unwrap_or_default();
            let kind = if title == "Plan" {
                "project"
            } else if title.starts_with("Task ") {
                "task"
            } else {
                "document"
            };
            reads.extend(pending.drain(..).map(|read| (kind, read)));
        }
    }
    assert!(
        pending.is_empty(),
        "every read was spent on a write: {pending:?}"
    );
    reads
}

#[test]
fn a_whole_plan_copy_spends_two_visibility_reads_per_write() {
    let clock = SimulatedClock::start(1);
    let sandbox = Sandbox::new();
    let root = sandbox.subdirectory("plan");
    author(&root);
    let (mut config, board) = github_projects_with_board(&sandbox);
    let front = VisibilityBoard::new(config["endpoint"].as_str().expect("an endpoint"), &clock);
    config["endpoint"] = json!(front.endpoint);
    sandbox.project_document(&document(&json!({
        "plan": {"plugin": "local-md", "config": {"root": root}, "visibility": "private"},
        "board": {"plugin": "github-projects", "config": config, "visibility": "private"},
    })));
    let before = board.served().len();
    let lease = clock.lease(0);
    copy(&sandbox, &clock, "project", "plan:plan");
    lease.wait_for_detach();
    copy(&sandbox, &clock, "document", "plan:design");
    lease.wait_for_detach();
    drop(lease);
    let served = board.served()[before..].to_vec();
    let creates = served
        .iter()
        .filter(|(document, _)| document.contains("createIssue(input:$input)"))
        .count();
    assert_eq!(
        creates as u64, WRITES,
        "the workload is the 102 writes it names"
    );
    let reads = attributed(&served);
    let count = |kind: Option<&str>, read: Option<&str>| {
        reads
            .iter()
            .filter(|(of, by)| {
                kind.is_none_or(|kind| kind == *of) && read.is_none_or(|read| read == *by)
            })
            .count() as u64
    };
    let total = count(None, None);
    let seconds = front.seconds();
    let by_write = json!({
        "project": count(Some("project"), None),
        "task": count(Some("task"), None),
        "document": count(Some("document"), None),
    });
    let by_read = json!({
        "project": count(None, Some("project")),
        "repository": count(None, Some("repository")),
    });
    let directory = super::budget_runner::directory();
    std::fs::create_dir_all(&directory).expect("the telemetry directory");
    std::fs::write(
        onetaskgraph_e2e_support::telemetry::file_in(&directory, BUDGET),
        serde_json::to_vec(&json!({
            "budget": BUDGET,
            "value": total,
            "detail": format!(
                "{total} visibility requests over {WRITES} writes ({:.2} per write): by write \
                 {by_write}; by read {by_read}; the reads added {seconds:.2} simulated seconds \
                 at 0.38 s each, a write's two overlapping",
                total as f64 / WRITES as f64
            ),
            "workload": WORKLOAD,
            "writes": WRITES,
            "by_write": by_write,
            "by_read": by_read,
            "simulated_seconds": seconds,
        }))
        .expect("JSON"),
    )
    .expect("the telemetry is written");
    // What the budget holds the copy to is onebudgetspec's to judge; what this journey holds
    // is the shape of what it spent: both reads, and only those two, for every write.
    assert_eq!(by_read["project"], json!(WRITES), "{by_read}");
    assert_eq!(by_read["repository"], json!(WRITES), "{by_read}");
    assert_eq!(by_write, json!({"project": 2, "task": 200, "document": 2}));
    assert!(
        (seconds - WRITES as f64 * READ.as_secs_f64()).abs() < 1e-6,
        "a write's two reads overlap: {seconds}"
    );
}
