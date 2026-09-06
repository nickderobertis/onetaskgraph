//! What this lane's cleanup removes, and what it must leave, against a loopback board.
//!
//! The cleanup is the real one — `journey::sweep_orphans` and `journey::remove_live_state`,
//! the same code the credentialed lane runs — driven through the same [`journey::Endpoints`]
//! indirection the lane is pointed at GitHub through. What stands in for GitHub is a local
//! HTTP server holding a board, a repository's labels, and one behaviour that is the whole
//! reason this file exists: an item another deleter takes between the listing and the delete
//! is refused, the way GitHub refuses a delete of what is no longer there.
//!
//! No credential and no third-party API: every board item below is one this file wrote.
//!
//! # What is proven here, and why a stand-in can prove it
//!
//! A run used to clear residue at startup with a predicate that recognised **any** run's
//! artifacts, so a session starting beside another one deleted that one's in-flight items,
//! and a delete refused for an item that had already gone failed the whole journey. Both of
//! those are decisions this lane makes about what it can see, so a board it can see is
//! enough to settle them. What a stand-in cannot settle is how GitHub spells a refusal —
//! and nothing here depends on that spelling, which is the point: the cleanup asks the board
//! what is left rather than reading the refusal.

use std::io::{Read, Write as _};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, LazyLock, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use onetaskgraph_live::artifact::{Registration, Registry, Run, Sweep};
use onetaskgraph_live::{Credential, Exclusivity, Session};
use serde_json::{Value, json};

// The credentialed lane's own halves again: `journey` for the cleanup under test, and `lane`
// for the naming every artifact below carries. Most of each belongs to the target that
// drives it against GitHub, so what this one does not reach is that drive's rather than dead
// code — the same reason `tests/budget_gate.rs` carries this.
#[allow(dead_code)]
mod journey;
#[allow(dead_code)]
mod lane;

use lane::{SESSION_NAME, artifact_label, artifact_title};

/// The board, the repository and the credential every drive below is pointed at.
const BOARD: &str = "PVT_board";
const REPOSITORY: &str = "acme/work";
const TOKEN: &str = "test-token";

/// The window these drives sweep on.
///
/// **A check chooses its own, and that is a property of the rule rather than a shortcut.**
/// Safety does not rest on the window's length — what permits a removal is the owning run's
/// registration lock, and the window can only hold a removal back — so proving what a sweep
/// does needs no session that runs for the length of the real one. See
/// `onetaskgraph_live::artifact::STALE_AFTER` for the length the lanes use and why.
const WINDOW: Duration = Duration::from_secs(60);

/// How long a drive below will wait for a real second process to reach a state.
const PATIENCE: Duration = Duration::from_secs(30);

/// The variable that tells a re-execution of this binary it is the live run beside a sweep,
/// and where to say so once it has registered.
const CHILD_VARIABLE: &str = "ONETASKGRAPH_SWEEP_GATE_CHILD";

/// The instant every sweep below is made at, in microseconds since the epoch.
///
/// Fixed rather than read from the clock, so what each artifact's age *is* relative to the
/// window is decided by this file and not by how long the suite took to get here.
const NOW: u64 = 1_787_816_134_627_361;

/// A stamp written `windows` windows before [`NOW`].
fn aged(windows: u64) -> u64 {
    NOW - windows * u64::try_from(WINDOW.as_micros()).expect("a minute fits in microseconds")
}

/// The registry every drive below decides against, and this process's own run in it.
///
/// A directory of this check's own rather than the machine's, so what the registry holds is
/// exactly the runs this file put there — and so a real live session of some other lane on
/// this machine is neither consulted nor disturbed.
///
/// Left behind when this binary ends, for the same reason a registration is: the registration
/// this process holds is an open file, and what closes it is the process exiting. It is a
/// handful of empty files under the platform's own temporary directory.
struct Runs {
    directory: PathBuf,
    registry: Registry,
    mine: Run,
}

static RUNS: LazyLock<Runs> = LazyLock::new(|| {
    let directory =
        std::env::temp_dir().join(format!("onetaskgraph-sweep-gate-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    let registry = Registry::at(&directory);
    let mine = registry.enrol();
    assert!(
        mine.host().is_some(),
        "a registry this check can write is what every drive below decides against"
    );
    Runs {
        directory,
        registry,
        mine,
    }
});

/// A run of this machine that has ENDED: really registered, and its registration really
/// given up, which is the state the kernel leaves behind when a process dies.
fn ended_run(offset: u32) -> Run {
    let registration = Registration::take(&RUNS.registry, 40_000 + offset)
        .expect("a run that this check then ends");
    let run = registration.run();
    // Given up here, so the lock is free — the same thing the kernel does for a process that
    // has died, and the only state that authorises a removal.
    drop(registration);
    run
}

/// The sweep this run makes at `now`, decided against the registry above.
fn sweep_at(now: u64) -> Sweep {
    RUNS.registry.sweep(RUNS.mine, now, WINDOW)
}

/// A REAL second process, holding a live registration in the same registry.
///
/// This is what a concurrent session is, and nothing smaller proves the property: the whole
/// question is whether a sweep can tell a run that is still going from one that is over, and
/// a description of a live run is not one. So this binary is re-executed with
/// [`CHILD_VARIABLE`] set, it registers itself and waits, and the sweep beside it decides on
/// the lock that child really holds.
struct LiveRunBeside {
    child: Child,
    run: Run,
}

impl LiveRunBeside {
    fn start() -> Self {
        let child = Command::new(
            std::env::current_exe().expect("the path of the test binary being re-executed"),
        )
        .args([
            "--exact",
            "a_re_execution_of_this_binary_is_the_live_run_beside_a_sweep",
        ])
        .env(CHILD_VARIABLE, &RUNS.directory)
        .env(
            onetaskgraph_live::artifact::REGISTRY_DIRECTORY_VARIABLE,
            &RUNS.directory,
        )
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("a second process for the live run beside this sweep");
        let run = Run::vouched(
            RUNS.mine
                .host()
                .expect("the registry every drive here decides against"),
            child.id(),
        );
        // Waited for by a file that child writes AFTER it has registered, rather than by
        // asking the registry: asking means taking the very lock the child is trying to
        // take, and losing that race would leave it unable to say who it is.
        let started = Instant::now();
        while !ready_marker(child.id()).exists() {
            assert!(
                started.elapsed() < PATIENCE,
                "the live run beside this sweep never registered"
            );
            thread::sleep(Duration::from_millis(20));
        }
        Self { child, run }
    }

    /// End that run the way an interrupted one ends, and wait until the kernel has said so.
    fn end(&mut self) {
        // A process this drive started itself, by the handle it started it with.
        let _ = self.child.kill();
        let _ = self.child.wait();
        let started = Instant::now();
        while !RUNS.registry.finished_runs().contains(&self.child.id()) {
            assert!(
                started.elapsed() < PATIENCE,
                "the kernel never released the ended run's registration"
            );
            thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for LiveRunBeside {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The other half of [`LiveRunBeside`]: what a re-execution of this binary does.
///
/// An ordinary run of this target reaches the early return and asserts nothing, because
/// there is no sweep beside it to be the live run for.
#[test]
fn a_re_execution_of_this_binary_is_the_live_run_beside_a_sweep() {
    let Some(directory) = std::env::var_os(CHILD_VARIABLE) else {
        return;
    };
    let run = Run::current();
    assert!(
        run.host().is_some(),
        "the registry this run was pointed at could not vouch for it"
    );
    std::fs::write(
        PathBuf::from(directory).join(format!("ready-{}", run.process())),
        b"",
    )
    .expect("saying that this run has registered");
    // Alive until the drive beside this kills it, and not for ever if that drive never does.
    thread::sleep(PATIENCE * 4);
}

/// The file the live run beside a sweep writes once it has registered.
fn ready_marker(process: u32) -> PathBuf {
    RUNS.directory.join(format!("ready-{process}"))
}

/// Everything the stand-in holds, and the one race it can be told to stage.
///
/// Three stores, because this lane writes to three and its cleanup has to leave none of them
/// with residue: the board holds items, the repository holds the issues behind them, and the
/// repository holds the labels.
#[derive(Default)]
struct Board {
    /// Board items as `(item id, the issue behind it, title)`.
    items: Vec<(String, Option<String>, String)>,
    /// The repository's own issues, which outlive the board items pointing at them.
    issues: Vec<String>,
    /// The repository's own label names.
    labels: Vec<String>,
    /// Item ids another deleter takes the moment this board lists its items, and label names
    /// another deleter takes the moment it lists its labels.
    ///
    /// The race in one switch: the cleanup asked what was there, was told, and by the time
    /// its delete arrives the thing is gone. Staged rather than waited for, because a race
    /// nobody can stage is a race nobody can prove either way. Two lists rather than one
    /// because the two surfaces are listed at different moments, and a thing that vanished
    /// before the listing that names it is not the race — it is an empty listing.
    vanishing_items: Vec<String>,
    vanishing_labels: Vec<String>,
    /// What this board has already refused a delete for, so a test can assert the tolerance
    /// was exercised rather than that nothing was ever deleted.
    refused: Vec<String>,
}

impl Board {
    /// Take everything `vanishing_items` names off the board, as another deleter would.
    fn let_the_item_race_happen(&mut self) {
        let taken = std::mem::take(&mut self.vanishing_items);
        self.items.retain(|(id, _, _)| !taken.contains(id));
        self.issues
            .retain(|issue| !taken.iter().any(|item| issue_of(item) == *issue));
        self.vanishing_items = taken;
    }

    /// The same for the repository's labels.
    fn let_the_label_race_happen(&mut self) {
        let taken = std::mem::take(&mut self.vanishing_labels);
        self.labels.retain(|name| !taken.contains(name));
        self.vanishing_labels = taken;
    }

    fn holds_item(&self, id: &str) -> bool {
        self.items.iter().any(|(item, _, _)| item == id)
    }

    fn holds_issue(&self, id: &str) -> bool {
        self.issues.iter().any(|issue| issue == id)
    }

    fn holds_label(&self, name: &str) -> bool {
        self.labels.iter().any(|held| held == name)
    }

    fn titles(&self) -> Vec<String> {
        sorted(
            self.items
                .iter()
                .map(|(_, _, title)| title.clone())
                .collect(),
        )
    }

    fn label_names(&self) -> Vec<String> {
        sorted(self.labels.clone())
    }
}

/// The issue a vanishing board item takes with it, by the naming every drive below uses.
///
/// Another deleter of this lane's artifacts removes the issue behind an item the same way
/// this lane's own cleanup does, so a staged race has to take both.
fn issue_of(item: &str) -> String {
    item.replace("PVTI_", "I_")
}

/// The one stand-in this binary drives, since a journey is pointed at one API per binary.
static STANDIN: LazyLock<Arc<Mutex<Board>>> = LazyLock::new(|| {
    let board = Arc::new(Mutex::new(Board::default()));
    let host = serve(Arc::clone(&board));
    journey::against(journey::Endpoints {
        graphql: format!("{host}/graphql"),
        rest_host: host,
        source: None,
    });
    board
});

/// One drive at a time: the board is one shared fixture and every test below sweeps all of it.
///
/// Tokio's rather than the standard library's, because the guard is held across the awaits
/// of the drive it is serialising — which is the whole of what it is for, and what a
/// blocking guard would deadlock a runtime on.
static ONE_AT_A_TIME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Take the board, planted with `items` and `labels`, for the length of one drive.
async fn planted(
    items: Vec<(&str, Option<&str>, String)>,
    labels: Vec<String>,
    vanishing_items: Vec<String>,
    vanishing_labels: Vec<String>,
) -> tokio::sync::MutexGuard<'static, ()> {
    let held = ONE_AT_A_TIME.lock().await;
    // A test that failed poisoned nothing of consequence: the next one replants the board
    // outright, and reporting a poisoned lock instead would hide the failure that caused it.
    let mut board = STANDIN
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let items: Vec<_> = items
        .into_iter()
        .map(|(item, issue, title)| (item.to_owned(), issue.map(str::to_owned), title))
        .collect();
    *board = Board {
        issues: items
            .iter()
            .filter_map(|(_, issue, _)| issue.clone())
            .collect(),
        items,
        labels,
        vanishing_items,
        vanishing_labels,
        refused: Vec::new(),
    };
    held
}

/// What the board holds now, as sorted titles and sorted label names.
fn left() -> (Vec<String>, Vec<String>) {
    let board = STANDIN
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    (board.titles(), board.label_names())
}

/// The session a lane holds while it is reaching its API, opened the way this lane opens it.
///
/// Held across every sweep below, so what those sweeps run beside is a session that is
/// really open rather than a description of one.
fn session_in_flight() -> Session {
    Session::open(
        SESSION_NAME,
        Credential::new(TOKEN).expect("a placeholder credential is not blank"),
        Exclusivity::Shared,
    )
    .unwrap_or_else(|declined| {
        panic!(
            "this lane takes no seat, so nothing may decline it here: {}",
            declined.message()
        )
    })
}

#[tokio::test]
async fn a_sweep_leaves_a_concurrent_live_runs_artifacts_however_old_they_are() {
    // The property the age rule got wrong, driven against the real cleanup. The run beside
    // this one is a REAL second process holding a REAL registration lock, and its artifacts
    // are stamped ten windows ago — which is what a hung session looks like, and what a
    // session a secondary rate limiter has parked for the best part of an hour becomes.
    // Nothing about their age is different from an abandoned run's; the lock is the whole of
    // the difference, and the sweep decides on it.
    let mut beside = LiveRunBeside::start();
    let theirs = artifact_title(beside.run, aged(10));
    let their_label = artifact_label(beside.run, aged(10));
    let mine = artifact_title(RUNS.mine, aged(10));
    let my_label = artifact_label(RUNS.mine, aged(10));
    let ended = ended_run(1);
    let stale = artifact_title(ended, aged(2));
    let stale_label = artifact_label(ended, aged(2));
    let fresh_ended = ended_run(2);
    let fresh = artifact_title(fresh_ended, NOW);
    let fresh_label = artifact_label(fresh_ended, NOW);
    let _one_at_a_time = planted(
        vec![
            ("PVTI_theirs", Some("I_theirs"), theirs.clone()),
            ("PVTI_mine", Some("I_mine"), mine.clone()),
            ("PVTI_stale", Some("I_stale"), stale.clone()),
            ("PVTI_fresh", Some("I_fresh"), fresh.clone()),
            ("PVTI_nobodys", None, "AI Orchestrator plan".to_owned()),
        ],
        vec![
            their_label.clone(),
            my_label.clone(),
            stale_label.clone(),
            fresh_label.clone(),
            "bug".to_owned(),
        ],
        vec![],
        vec![],
    )
    .await;
    let _in_flight = session_in_flight();

    journey::sweep_orphans(TOKEN, BOARD, REPOSITORY, &sweep_at(NOW))
        .await
        .expect("an orphan sweep over a board it can read succeeds");

    let (titles, labels) = left();
    assert_eq!(
        titles,
        sorted(vec![
            theirs.clone(),
            mine,
            fresh,
            "AI Orchestrator plan".to_owned()
        ]),
        "the sweep took an artifact a live run owns"
    );
    assert_eq!(
        labels,
        sorted(vec![
            their_label.clone(),
            my_label,
            fresh_label,
            "bug".to_owned()
        ]),
        "the label sweep took a label a live run owns"
    );
    assert!(
        !titles.contains(&stale) && !labels.contains(&stale_label),
        "the sweep left the residue of a run that has ended, which is what it exists to recover"
    );

    // And the second half of the same property: once that run really ends, the very same
    // artifacts — not one microsecond older in the sweep's eyes, since it is made at the
    // same instant — become the residue this sweep recovers. Nothing changed but the run.
    beside.end();
    journey::sweep_orphans(TOKEN, BOARD, REPOSITORY, &sweep_at(NOW))
        .await
        .expect("a second sweep over the same board succeeds");
    let (titles, labels) = left();
    assert!(
        !titles.contains(&theirs) && !labels.contains(&their_label),
        "an ended run's artifacts were not recovered: {titles:?} {labels:?}"
    );
}

#[tokio::test]
async fn an_artifact_another_deleter_took_first_leaves_the_cleanup_successful() {
    // The board answers the listing and then the item is gone — swept by another run,
    // removed by hand, whatever. GitHub refuses the delete that follows, and treating that
    // refusal as a failure once killed a whole journey over an item that had already gone.
    let ended = ended_run(3);
    let stale = artifact_title(ended, aged(2));
    let stale_label = artifact_label(ended, aged(2));
    let _one_at_a_time = planted(
        vec![("PVTI_racing", Some("I_racing"), stale.clone())],
        vec![stale_label.clone()],
        vec!["PVTI_racing".to_owned()],
        vec![stale_label.clone()],
    )
    .await;
    let _in_flight = session_in_flight();

    journey::sweep_orphans(TOKEN, BOARD, REPOSITORY, &sweep_at(NOW))
        .await
        .expect("a delete of what has already gone is the outcome the delete was asking for");

    let (titles, labels) = left();
    assert!(
        titles.is_empty() && labels.is_empty(),
        "{titles:?} {labels:?}"
    );
    // And the tolerance was really exercised: the board refused both deletes rather than
    // this having passed because nothing was ever asked of it.
    let refused = STANDIN
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .refused
        .clone();
    assert_eq!(
        sorted(refused),
        sorted(vec!["PVTI_racing".to_owned(), stale_label]),
        "the board did not refuse the deletes this case is about"
    );
}

#[tokio::test]
async fn this_runs_own_cleanup_removes_everything_it_wrote_and_nothing_else() {
    // The other half of the arrangement: the sweep above recovers an ended run's, and this
    // removes this run's own — whether the journey passed or failed, which is
    // `run_then_cleanup`'s and is asserted in `lane_shape.rs`.
    let mine = artifact_title(RUNS.mine, NOW);
    let also_mine = artifact_title(RUNS.mine, NOW + 1);
    let my_label = artifact_label(RUNS.mine, NOW);
    let ended = ended_run(4);
    let theirs = artifact_title(ended, NOW);
    let their_label = artifact_label(ended, NOW);
    let _one_at_a_time = planted(
        vec![
            ("PVTI_mine", Some("I_mine"), mine),
            ("PVTI_mine_draft", None, also_mine),
            ("PVTI_theirs", Some("I_theirs"), theirs.clone()),
        ],
        vec![my_label, their_label.clone(), "bug".to_owned()],
        vec![],
        vec![],
    )
    .await;
    let _in_flight = session_in_flight();

    journey::remove_live_state(TOKEN, BOARD, REPOSITORY, RUNS.mine, false)
        .await
        .expect("this run's own cleanup over a board it can read succeeds");

    let (titles, labels) = left();
    assert_eq!(titles, vec![theirs]);
    assert_eq!(labels, sorted(vec![their_label, "bug".to_owned()]));
    // The issue behind the board item goes with it: taking an item off the board leaves the
    // issue in the repository, and this lane's claim is that it leaves no residue anywhere.
    let board = STANDIN
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(
        !board.holds_issue("I_mine"),
        "the issue behind this run's board item is still in the repository"
    );
    assert!(
        board.holds_issue("I_theirs"),
        "this run's cleanup took another run's issue with it"
    );
}

fn sorted(mut names: Vec<String>) -> Vec<String> {
    names.sort();
    names
}

/// A local stand-in for the board and the repository this lane's cleanup reaches.
///
/// One listener for both, because the cleanup makes GraphQL and REST calls and
/// [`journey::Endpoints`] is what says where each goes. Every path it does not recognise is
/// answered `404` and nothing on the board changes, so a cleanup that started calling
/// something else would fail here rather than pass quietly.
fn serve(board: Arc<Mutex<Board>>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a stand-in listener");
    let host = format!("http://{}", listener.local_addr().unwrap());
    thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.expect("a stand-in connection");
            let (method, path, body) = read_request(&mut stream);
            let (status, payload) = if method == "POST" && path == "/graphql" {
                graphql(&board, &body.expect("a GraphQL document"))
            } else {
                rest(&board, &method, &path)
            };
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n\
                 x-ratelimit-limit: 5000\r\nx-ratelimit-used: 1\r\n\
                 x-ratelimit-remaining: 4999\r\nx-ratelimit-resource: core\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                payload.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    host
}

/// What this board answers one GraphQL document with.
fn graphql(board: &Arc<Mutex<Board>>, request: &Value) -> (&'static str, String) {
    let query = request["query"].as_str().expect("a GraphQL document");
    let input = request
        .pointer("/variables/input")
        .cloned()
        .unwrap_or(Value::Null);
    let mut board = board
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if query.contains("on ProjectV2{items(first:100,after:$after)") {
        assert_eq!(
            request.pointer("/variables/id").and_then(Value::as_str),
            Some(BOARD)
        );
        let nodes = board
            .items
            .iter()
            .map(|(item, issue, title)| match issue {
                Some(issue) => json!({"id":item,
                    "content":{"__typename":"Issue","id":issue,"title":title}}),
                None => json!({"id":item,"content":{"title":title}}),
            })
            .collect::<Vec<_>>();
        // The listing answered; whatever this board was told would vanish, vanishes now.
        board.let_the_item_race_happen();
        return answered(json!({"node":{"items":{"nodes":nodes,
            "pageInfo":{"hasNextPage":false,"endCursor":null}}}}));
    }
    if query.contains("deleteProjectV2Item(input:$input)") {
        let item = input["itemId"]
            .as_str()
            .expect("a board item id")
            .to_owned();
        if !board.holds_item(&item) {
            board.refused.push(item.clone());
            return gone(&item);
        }
        board.items.retain(|(held, _, _)| *held != item);
        return answered(json!({"deleteProjectV2Item":{"deletedItemId":item}}));
    }
    if query.contains("deleteIssue(input:$input)") {
        let issue = input["issueId"].as_str().expect("an issue id").to_owned();
        if !board.holds_issue(&issue) {
            board.refused.push(issue.clone());
            return gone(&issue);
        }
        board.issues.retain(|held| *held != issue);
        return answered(json!({"deleteIssue":{"repository":{"id":"REPO_1"}}}));
    }
    (
        "400 Bad Request",
        json!({"errors":[{"message":format!("this stand-in answers no such document: {query}")}]})
            .to_string(),
    )
}

/// What this board answers one REST call with.
fn rest(board: &Arc<Mutex<Board>>, method: &str, path: &str) -> (&'static str, String) {
    let mut board = board
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let listing = format!("/repos/{REPOSITORY}/labels");
    if method == "GET" && path.starts_with(&format!("{listing}?")) {
        let names = board
            .labels
            .iter()
            .map(|name| json!({"name":name}))
            .collect::<Vec<_>>();
        board.let_the_label_race_happen();
        return ("200 OK", Value::Array(names).to_string());
    }
    if method == "DELETE"
        && let Some(name) = path.strip_prefix(&format!("{listing}/"))
    {
        if !board.holds_label(name) {
            board.refused.push(name.to_owned());
            return (
                "404 Not Found",
                json!({"message":"Not Found",
                       "documentation_url":"https://docs.github.com/rest/issues/labels"})
                .to_string(),
            );
        }
        board.labels.retain(|held| held != name);
        return ("204 No Content", String::new());
    }
    (
        "404 Not Found",
        json!({"message":format!("this stand-in answers no such endpoint: {method} {path}")})
            .to_string(),
    )
}

fn answered(data: Value) -> (&'static str, String) {
    ("200 OK", json!({ "data": data }).to_string())
}

/// GitHub's own answer to a mutation naming a node that is no longer there.
///
/// The shape rather than the sentence is what matters: a `200` carrying errors, which is how
/// GraphQL reports a refusal and how `journey`'s own transport reads one. Nothing in the
/// cleanup reads the message — it asks the board what is left instead — so what this pins is
/// that a refusal is answered at all, not a spelling this repository would have to keep up
/// with.
fn gone(id: &str) -> (&'static str, String) {
    (
        "200 OK",
        json!({"data":{},"errors":[{"type":"NOT_FOUND",
            "message":format!("Could not resolve to a node with the global id of '{id}'.")}]})
        .to_string(),
    )
}

/// One HTTP request off the wire, as `(method, path, JSON body)`.
fn read_request(stream: &mut impl Read) -> (String, String, Option<Value>) {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        let count = stream.read(&mut chunk).expect("a stand-in request");
        assert!(count > 0, "the request ended before its headers");
        bytes.extend_from_slice(&chunk[..count]);
        if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    let header_end = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("a header terminator")
        + 4;
    let headers = String::from_utf8_lossy(&bytes[..header_end]).into_owned();
    // Every call this lane makes carries the credential the session handed it, and a
    // stand-in that answered an unauthenticated one would prove less than it looks.
    assert!(
        headers.contains(&format!("authorization: Bearer {TOKEN}")),
        "{headers}"
    );
    let mut request = headers.lines().next().unwrap_or_default().split(' ');
    let method = request.next().unwrap_or_default().to_owned();
    let path = request.next().unwrap_or_default().to_owned();
    let length = headers
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length: ")
                .and_then(|value| value.trim().parse::<usize>().ok())
        })
        .unwrap_or_default();
    while bytes.len() - header_end < length {
        let count = stream.read(&mut chunk).expect("a stand-in request body");
        assert!(count > 0, "the request ended before its declared body");
        bytes.extend_from_slice(&chunk[..count]);
    }
    let body = (length > 0).then(|| {
        serde_json::from_slice(&bytes[header_end..header_end + length]).expect("request JSON")
    });
    (method, path, body)
}
