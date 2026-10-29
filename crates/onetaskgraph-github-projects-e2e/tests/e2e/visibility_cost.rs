//! What a whole first copy of a mixed plan spends learning who can read a board declared
//! private: the recording behind the `visibility-reads-per-destination` budget.
//!
//! The workload is the realistic one a plan copy is: one project, its 100 tasks — each depending
//! on the one before it — and its design document, 102 writes onto one GitHub board declared
//! private, copied out of a private folder of Markdown. Half the tasks name only one of five
//! repositories the loopback serves public, ten each, and half one of five it serves private.
//! Each public task's view goes to a public board, whose writes read no visibility. The plan is
//! copied as a person copies one: `project copy`, then `document copy` of its design document —
//! two copies, each reading each destination it writes to once. The loopback answers each
//! visibility read after 0.38 simulated seconds — what one took on the real API — and every other
//! request at once. The journey records every read each copy spent, by destination, REST and
//! GraphQL apart, with every other REST request, which is telemetry with no threshold of its own;
//! `budget_runner::report_visibility_reads` reports the largest number of reads any one
//! destination received in one copy, with the rest as its detail, and runs no copy.

use std::collections::BTreeMap;
use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::process::Output;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use onetaskgraph_e2e_support::binary;
use onetaskgraph_e2e_support::clock::SimulatedClock;
use onetaskgraph_e2e_support::loopback::{self, After, Upstream};
use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{RepositoryAnswer, document, github_projects_with_board};

/// How long one visibility read took on the real API.
const READ: Duration = Duration::from_millis(380);
/// The tasks of the plan copied.
const TASKS: usize = 100;
/// The repositories of each visibility the tasks name, ten tasks to each.
const REPOSITORIES: usize = 5;
/// The writes the copy makes onto the private board: the project, its tasks and its document.
pub(super) const WRITES: u64 = TASKS as u64 + 2;
/// The budget this journey records for.
pub(super) const BUDGET: &str = "visibility-reads-per-destination";
/// The workload this journey records, which the report refuses any other recording for.
pub(super) const WORKLOAD: &str = "mixed-project-copy-101-writes-100-tasks-50-in-5-public-50-in-5-private-then-document-copy-1-write";
/// The repository the private board creates an issue in when nothing else decides.
const CONFIGURED: &str = "nickderobertis/onetaskgraph";

fn public(k: usize) -> String {
    format!("nickderobertis/open-{k}")
}

fn private(k: usize) -> String {
    format!("nickderobertis/closed-{k}")
}

/// The destinations each command in the recorded workload must read.
pub(super) fn destinations(project: bool) -> Vec<String> {
    let mut names = vec![CONFIGURED.to_owned(), "project".to_owned()];
    if project {
        names.extend((0..REPOSITORIES).flat_map(|k| [public(k), private(k)]));
    }
    names.sort_unstable();
    names
}

/// Answer a repository public exactly when it is one of the `open-` ones.
const VISIBILITY: &str = "import json,sys; r=json.load(sys.stdin)['repository']; \
     print(json.dumps({'visibility': 'public' if r.startswith('github.com/nickderobertis/open-') \
     else 'private'}))";
/// Pass every write: what is measured is where the reads go, not what a view says.
const CHECK: &str = "import sys; sys.stdin.read(); print('{\"verdict\": \"pass\"}')";

/// A loopback in front of the private board, delaying each visibility read on the simulated
/// clock and passing every other request through at once.
struct VisibilityBoard {
    endpoint: String,
    /// The virtual intervals the reads were delayed over.
    intervals: Arc<Mutex<Vec<(Duration, Duration)>>>,
    /// The client connections that actually forwarded a request to the board.
    forwarded: Arc<AtomicUsize>,
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
        let intervals = Arc::new(Mutex::new(Vec::new()));
        let held = Arc::clone(&intervals);
        let clock = clock.clone();
        // Upstream connects lazily: an accepted client may close without sending a request.
        // Count only handlers that forward, each of which keeps one connection to the board.
        let forwarded = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&forwarded);
        loopback::serve_each(listener, move || {
            let (clock, held) = (clock.clone(), Arc::clone(&held));
            let mut upstream = Upstream::new(upstream);
            let counted = Arc::clone(&counted);
            let mut first = true;
            move |sent: &loopback::Request, stream: &mut TcpStream| {
                if first {
                    counted.fetch_add(1, Ordering::SeqCst);
                    first = false;
                }
                serve(sent, stream, &clock, &mut upstream, &held)
            }
        });
        Self {
            endpoint,
            intervals,
            forwarded,
        }
    }

    fn assert_connections(&self, connections: usize) {
        assert_eq!(
            connections,
            self.forwarded.load(Ordering::SeqCst),
            "the proxy opened a connection to the board other than the one each of its own keeps"
        );
    }

    /// The simulated seconds the visibility reads held the copies for, overlapping reads once.
    fn seconds(&self) -> f64 {
        let mut intervals = self.intervals.lock().expect("not poisoned").clone();
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
    intervals: &Arc<Mutex<Vec<(Duration, Duration)>>>,
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
        let started = clock.now();
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("a runtime")
            .block_on(request.delay(READ));
        intervals
            .lock()
            .expect("not poisoned")
            .push((started, clock.now()));
    }
    match stream.write_all(&response) {
        Ok(()) => After::KeepOpen,
        Err(error) => panic!("the answer could not be written to the binary: {error}"),
    }
}

/// The repository task `index` names: the even ones a public repository, the odd ones a private
/// one, ten tasks to each repository.
fn named(index: usize) -> String {
    let k = (index / 2) % REPOSITORIES;
    if index.is_multiple_of(2) {
        public(k)
    } else {
        private(k)
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
                "---\ntitle: Task {index:03}\nstatus: todo\nproject: plan\n\
                 repositories: [github.com/{}]\n{depends}---\nStep {index} of the plan.\n",
                named(index)
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

/// What one copy sent the private board, counted.
struct Spent {
    /// The visibility reads each destination received: `project` for the board's.
    by_destination: BTreeMap<String, u64>,
    rest: u64,
    graphql: u64,
    /// Every REST request that was not a visibility read.
    other_rest: u64,
    creates: u64,
}

fn spent(served: &[(String, Value)]) -> Spent {
    let mut by_destination = BTreeMap::new();
    let (mut rest, mut graphql, mut other_rest, mut creates) = (0, 0, 0, 0);
    for (document, variables) in served {
        if document == onetaskgraph_github_projects::graphql::PROJECT_VISIBILITY {
            graphql += 1;
            *by_destination.entry("project".to_owned()).or_insert(0) += 1;
        } else if let Some(repository) = document.strip_prefix("GET /repos/") {
            rest += 1;
            *by_destination.entry(repository.to_owned()).or_insert(0) += 1;
        } else if document.starts_with("GET ")
            || document.starts_with("POST /")
            || document.starts_with("PUT ")
        {
            other_rest += 1;
        } else if document == onetaskgraph_github_projects::graphql::CREATE_ISSUES {
            creates += (0..onetaskgraph_github_projects::MUTATION_BATCH)
                .filter(|slot| variables[format!("send{slot}")] == true)
                .count() as u64;
        } else if document.contains("createIssue(input:$input)") {
            creates += 1;
        }
    }
    Spent {
        by_destination,
        rest,
        graphql,
        other_rest,
        creates,
    }
}

#[test]
fn a_mixed_plans_project_and_document_copies_record_visibility_reads_by_destination() {
    let clock = SimulatedClock::start(1);
    let sandbox = Sandbox::new();
    let root = sandbox.subdirectory("plan");
    author(&root);
    let (mut config, board) = github_projects_with_board(&sandbox);
    for k in 0..REPOSITORIES {
        board.set_repository_visibility(&public(k), RepositoryAnswer::Public);
    }
    let front = VisibilityBoard::new(config["endpoint"].as_str().expect("an endpoint"), &clock);
    // Some HTTP stacks open a speculative connection and close it without a request.
    drop(
        TcpStream::connect(
            front
                .endpoint
                .strip_prefix("http://")
                .unwrap()
                .trim_end_matches("/graphql"),
        )
        .expect("an unused client connection"),
    );
    config["endpoint"] = json!(front.endpoint);
    // The public board the views go to: a loopback of its own, filing in a public repository.
    let (mut site_config, site) = github_projects_with_board(&sandbox);
    site_config["repository"] = json!(public(0));
    site.use_repository(&public(0));
    let mut configured: Value = serde_json::from_str(&document(&json!({
        "plan": {"plugin": "local-md", "config": {"root": root}, "visibility": "private"},
        "board": {"plugin": "github-projects", "config": config, "visibility": "private",
                  "routes": [{"repositories": (0..REPOSITORIES)
                      .map(|k| format!("github.com/{}", public(k)))
                      .collect::<Vec<_>>(), "to": "site"}]},
        "site": {"plugin": "github-projects", "config": site_config, "visibility": "public"},
    })))
    .expect("a configuration document");
    configured["write_policy"] = json!({
        "visibility_command": ["python3", "-c", VISIBILITY],
        "check_command": ["python3", "-c", CHECK],
    });
    sandbox.project_document(&configured.to_string());
    let lease = clock.lease(0);
    let mut copies = Vec::new();
    for (kind, id) in [("project", "plan:plan"), ("document", "plan:design")] {
        let before = board.served().len();
        copy(&sandbox, &clock, kind, id);
        lease.wait_for_detach();
        copies.push((format!("{kind} copy"), spent(&board.served()[before..])));
    }
    drop(lease);
    assert_eq!(
        copies.iter().map(|(_, spent)| spent.creates).sum::<u64>(),
        WRITES,
        "the workload is the 102 writes it names"
    );
    // Exactly the reads each copy owes: the project copy the board, each repository a task
    // names and the configured one its project's issue is in; the document copy the board and
    // that one repository.
    let wanted = destinations(true);
    let (_, project_copy) = &copies[0];
    assert_eq!(
        project_copy
            .by_destination
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        wanted
    );
    let (_, document_copy) = &copies[1];
    assert_eq!(
        document_copy
            .by_destination
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        destinations(false)
    );
    // The public board is not declared private, so nothing reads who can read it.
    assert!(
        site.served().iter().all(|(document, _)| {
            document != onetaskgraph_github_projects::graphql::PROJECT_VISIBILITY
                && !document.starts_with("GET /repos/")
        }),
        "the public board was asked who can read it"
    );
    let figure = copies
        .iter()
        .flat_map(|(_, spent)| spent.by_destination.values().copied())
        .max()
        .expect("a read");
    let total: u64 = copies
        .iter()
        .map(|(_, spent)| spent.rest + spent.graphql)
        .sum();
    let rest: u64 = copies.iter().map(|(_, spent)| spent.rest).sum();
    let graphql: u64 = copies.iter().map(|(_, spent)| spent.graphql).sum();
    let other_rest: u64 = copies.iter().map(|(_, spent)| spent.other_rest).sum();
    let distinct = copies
        .iter()
        .flat_map(|(_, spent)| spent.by_destination.keys())
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    let seconds = front.seconds();
    let recorded: Vec<Value> = copies
        .iter()
        .map(|(copy, spent)| {
            json!({
                "copy": copy,
                "reads": spent.rest + spent.graphql,
                "rest": spent.rest,
                "graphql": spent.graphql,
                "other_rest": spent.other_rest,
                "by_destination": spent.by_destination,
            })
        })
        .collect();
    let per_copy = copies
        .iter()
        .map(|(copy, spent)| {
            format!(
                "{copy}: {} reads of {} destinations ({} GraphQL, {} REST), {}",
                spent.rest + spent.graphql,
                spent.by_destination.len(),
                spent.graphql,
                spent.rest,
                spent
                    .by_destination
                    .iter()
                    .map(|(destination, reads)| format!("{destination} {reads}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    let directory = super::budget_runner::directory();
    std::fs::create_dir_all(&directory).expect("the telemetry directory");
    std::fs::write(
        onetaskgraph_e2e_support::telemetry::file_in(&directory, BUDGET),
        serde_json::to_vec(&json!({
            "budget": BUDGET,
            "value": figure,
            "detail": format!(
                "at most {figure} visibility read(s) of one destination in one copy, over \
                 {WRITES} writes onto a board declared private: {total} reads in all of \
                 {distinct} distinct destinations, {graphql} GraphQL and {rest} REST, and \
                 {other_rest} other REST requests; {per_copy}; the reads added {seconds:.2} \
                 simulated seconds at 0.38 s each"
            ),
            "workload": WORKLOAD,
            "writes": WRITES,
            "total": total,
            "distinct": distinct,
            "rest": rest,
            "graphql": graphql,
            "other_rest": other_rest,
            "copies": recorded,
            "simulated_seconds": seconds,
        }))
        .expect("JSON"),
    )
    .expect("the telemetry is written");
    front.assert_connections(board.connections());
}

/// An extra upstream connection must still be detected even with an unused client socket.
#[test]
#[should_panic(expected = "the proxy opened a connection to the board other than")]
fn an_extra_board_connection_is_refused() {
    let sandbox = Sandbox::new();
    let (config, board) = github_projects_with_board(&sandbox);
    let clock = SimulatedClock::start(1);
    let endpoint = config["endpoint"].as_str().expect("an endpoint");
    let front = VisibilityBoard::new(endpoint, &clock);
    let address = |endpoint: &str| {
        endpoint
            .strip_prefix("http://")
            .and_then(|rest| rest.split('/').next())
            .expect("a loopback address")
            .parse()
            .expect("a socket address")
    };
    drop(TcpStream::connect(address(&front.endpoint)).expect("an unused client connection"));
    // Bypass the proxy just as an erroneous extra upstream connection would.
    let mut extra = Upstream::new(address(endpoint));
    assert!(!extra.forward(b"GET /repos/nickderobertis/onetaskgraph HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer test-token\r\n\r\n").is_empty());
    front.assert_connections(board.connections());
}
