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

use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::process::Output;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use onetaskgraph_e2e_support::binary;
use onetaskgraph_e2e_support::clock::SimulatedClock;
use onetaskgraph_e2e_support::loopback::{self, After, Upstream};
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
    /// The pairs of reads that were in flight together, one per write that sent its two at once.
    generation: u64,
    /// The reads that waited for a partner and none arrived: a write that sent them one after
    /// the other.
    alone: u64,
    /// The virtual intervals the reads were delayed over, one per pair.
    intervals: Vec<(Duration, Duration)>,
}

/// A loopback in front of the fixture board, delaying each visibility read on the simulated
/// clock and passing every other request through at once.
struct VisibilityBoard {
    endpoint: String,
    pairing: Arc<(Mutex<Pairing>, Condvar)>,
    /// The connections the binary opened to this proxy.
    accepted: Arc<AtomicUsize>,
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
            .and_then(|address| address.parse().ok())
            .expect("the fixture board's address");
        let pairing = Arc::new((Mutex::new(Pairing::default()), Condvar::new()));
        let held = Arc::clone(&pairing);
        let clock = clock.clone();
        // Each connection the binary keeps alive to this proxy keeps one of its own to the board
        // behind it, and connections are answered side by side, so a write's two visibility
        // reads, sent together on two connections, are both held here at once.
        let accepted = loopback::serve_each(listener, move || {
            let (clock, held) = (clock.clone(), Arc::clone(&held));
            let mut upstream = Upstream::new(upstream);
            move |sent: &loopback::Request, stream: &mut TcpStream| {
                serve(sent, stream, &clock, &mut upstream, &held)
            }
        });
        Self {
            endpoint,
            pairing,
            accepted,
        }
    }

    /// How many pairs of reads were in flight together, and how many reads arrived alone.
    fn overlap(&self) -> (u64, u64) {
        let pairing = self.pairing.0.lock().expect("not poisoned");
        (pairing.generation, pairing.alone)
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
    sent: &loopback::Request,
    stream: &mut TcpStream,
    clock: &SimulatedClock,
    upstream: &mut Upstream,
    pairing: &Arc<(Mutex<Pairing>, Condvar)>,
) -> After {
    let request = clock.request(0);
    let visibility = sent.line().starts_with("GET /repos/")
        || serde_json::from_slice::<Value>(sent.body()).is_ok_and(|request| {
            request["query"] == onetaskgraph_github_projects::graphql::PROJECT_VISIBILITY
        });
    let response = upstream.forward(&sent.bytes());
    if response.is_empty() {
        // Unanswered, which `Upstream` has said why on standard error: the binary is told the
        // same way, by its connection closing.
        return After::Close;
    }
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
                state.alone += 1;
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
    }
    match stream.write_all(&response) {
        Ok(()) => After::KeepOpen,
        Err(error) => panic!("the answer could not be written to the binary: {error}"),
    }
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
/// read before a write's first mutation is that write's — and the reads before a request
/// creating several issues together are theirs, two to each, in the order it creates them.
fn attributed(served: &[(String, Value)]) -> Vec<(&'static str, &'static str)> {
    let kind_of = |title: &str| {
        if title == "Plan" {
            "project"
        } else if title.starts_with("Task ") {
            "task"
        } else {
            "document"
        }
    };
    let mut reads = Vec::new();
    let mut pending = Vec::new();
    for (document, variables) in served {
        if document == onetaskgraph_github_projects::graphql::PROJECT_VISIBILITY {
            pending.push("project");
        } else if document.starts_with("GET /repos/") {
            pending.push("repository");
        } else if document == onetaskgraph_github_projects::graphql::CREATE_ISSUES {
            let created: Vec<&str> = (0..onetaskgraph_github_projects::MUTATION_BATCH)
                .filter(|slot| variables[format!("send{slot}")] == true)
                .map(|slot| {
                    variables[format!("create{slot}")]["title"]
                        .as_str()
                        .unwrap_or_default()
                })
                .collect();
            let each = pending.len() / created.len().max(1);
            for title in created {
                reads.extend(
                    pending
                        .drain(..each.min(pending.len()))
                        .map(|read| (kind_of(title), read)),
                );
            }
        } else if document.contains("createIssue(input:$input)") {
            let title = variables["input"]["title"].as_str().unwrap_or_default();
            reads.extend(pending.drain(..).map(|read| (kind_of(title), read)));
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
    let creates: usize = served
        .iter()
        .map(|(document, variables)| {
            if document == onetaskgraph_github_projects::graphql::CREATE_ISSUES {
                (0..onetaskgraph_github_projects::MUTATION_BATCH)
                    .filter(|slot| variables[format!("send{slot}")] == true)
                    .count()
            } else {
                usize::from(document.contains("createIssue(input:$input)"))
            }
        })
        .sum();
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
    // What the copy spent is onebudgetspec's to judge, and the reads each write sends are the
    // boundary journeys'; what this journey holds is how they are sent: each write's two
    // together, both in flight at once, and never one after the other.
    let (paired, alone) = front.overlap();
    assert!(paired > 0, "the copy read who can read the board");
    assert_eq!(alone, 0, "a read waited for its partner and none came");
    assert_eq!(
        board.connections(),
        front.accepted.load(Ordering::SeqCst),
        "the proxy opened a connection to the board other than the one each of its own keeps"
    );
}
