//! What this lane's cleanup removes, and what it must leave, against a loopback workspace.
//!
//! The cleanup is the real one — `cleanup::remove_artifacts` and `cleanup::sweep_orphans`,
//! the same code the credentialed lane runs — driven through the same [`cleanup::against`]
//! seam the lane is pointed at Linear through. What stands in for Linear is a local HTTP
//! server holding issues, projects, documents and labels, and one behaviour that is half the
//! reason this file exists: an entity another deleter takes between the listing and the
//! delete is refused, the way Linear refuses a delete of what is no longer there.
//!
//! No credential and no third-party API: every entity below is one this file wrote.
//!
//! # What is proven here, and why a stand-in can prove it
//!
//! This lane used to clear residue at startup with a predicate that recognised **any** run's
//! artifacts, so a session starting beside another one deleted that one's in-flight issues;
//! the rule that replaced it recognised any run's artifacts that were old enough, which is
//! the same failure against a run that is merely slow. Both are decisions this lane makes
//! about names it can see, so a workspace it can see is enough to settle them. What a
//! stand-in cannot settle is how Linear spells a refusal — and nothing here depends on that
//! spelling, which is the point: the cleanup asks the workspace what is left rather than
//! reading the refusal.

use std::io::{Read, Write as _};
use std::net::TcpListener;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, LazyLock, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use onetaskgraph_live::artifact::{Registration, Registry, Run, Sweep};
use serde_json::{Value, json};

// Most of `cleanup` belongs to the target that drives it against Linear, so what this one
// does not reach is that drive's rather than dead code.
#[allow(dead_code)]
mod cleanup;

use cleanup::{RESIDUE, Residue, artifact_label, artifact_title, is_this_runs};

/// The credential every drive below sends, which no server but this file's own ever sees.
const KEY: &str = "test-key";

/// The window these drives sweep on.
///
/// **A check chooses its own, and that is a property of the rule rather than a shortcut.**
/// Safety does not rest on the window's length — what permits a removal is the owning run's
/// registration lock, and the window can only hold a removal back — so proving what a sweep
/// does needs no session that runs for the length of the real one. See
/// `onetaskgraph_live::artifact::STALE_AFTER` for the length the lane uses and why.
const WINDOW: Duration = Duration::from_secs(60);

/// How long a drive below will wait for a real second process to reach a state.
const PATIENCE: Duration = Duration::from_secs(30);

/// The variable that tells a re-execution of this binary it is the live run beside a sweep,
/// and where to say so once it has registered.
const CHILD_VARIABLE: &str = "ONETASKGRAPH_LINEAR_SWEEP_GATE_CHILD";

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
/// this machine is neither consulted nor disturbed. Left behind when this binary ends, for
/// the same reason a registration is: what closes an open registration is the process
/// exiting.
struct Runs {
    directory: PathBuf,
    registry: Registry,
    mine: Run,
}

static RUNS: LazyLock<Runs> = LazyLock::new(|| {
    let directory = std::env::temp_dir().join(format!(
        "onetaskgraph-linear-sweep-gate-{}",
        std::process::id()
    ));
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
    let registration = Registration::take(&RUNS.registry, process(40_000 + offset))
        .expect("a run that this check then ends");
    let run = registration.run();
    drop(registration);
    run
}

/// A run's process id, which no operating system numbers zero.
fn process(number: u32) -> NonZeroU32 {
    NonZeroU32::new(number).expect("a process id this check names is never zero")
}

/// The sweep this run makes at `now`, decided against the registry above.
fn sweep_at(now: u64) -> Sweep {
    RUNS.registry.sweep(RUNS.mine, now, WINDOW)
}

/// A REAL second process, holding a live registration in the same registry.
///
/// This is what a concurrent session is, and nothing smaller proves the property: the whole
/// question is whether a sweep can tell a run that is still going from one that is over, and
/// a description of a live run is not one.
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
            "a_run_of_this_binary_is_registered_and_the_registry_says_it_is_live",
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
            process(child.id()),
        );
        // Waited for by a file that child writes AFTER it has registered, rather than by
        // asking the registry: asking means taking the very lock the child is trying to
        // take, and losing that race would leave it unable to say who it is.
        let started = Instant::now();
        while !ready_marker(process(child.id())).exists() {
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
        while !RUNS
            .registry
            .finished_runs()
            .contains(&process(self.child.id()))
        {
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

/// A run of this binary registers itself, and the registry it joined says it is live.
///
/// **That is the assertion in both of this test's roles, and it is the whole premise of
/// every drive above.** Ordinarily it is a check of its own: what a sweep decides on is the
/// registration a live run holds, so a run that did not register — or that registered and
/// read back as over — would make every one of those drives prove nothing.
///
/// [`LiveRunBeside`] then re-executes this binary at exactly this test with the registry to
/// join named, which is how a sweep is driven beside a run in another process. In that role
/// it makes the same assertion, says it has registered, and stays alive to be swept at.
#[test]
fn a_run_of_this_binary_is_registered_and_the_registry_says_it_is_live() {
    let run = Run::current();
    assert!(
        run.host().is_some(),
        "the registry this run was pointed at could not vouch for it"
    );
    assert!(
        !Registry::shared().finished_runs().contains(&run.process()),
        "the registry reported a run that is running as one that has ended"
    );
    let Some(directory) = std::env::var_os(CHILD_VARIABLE) else {
        return;
    };
    std::fs::write(
        PathBuf::from(directory).join(format!("ready-{}", run.process())),
        b"",
    )
    .expect("saying that this run has registered");
    // Alive until the drive beside this kills it, and not for ever if that drive never does.
    thread::sleep(PATIENCE * 4);
}

/// The file the live run beside a sweep writes once it has registered.
fn ready_marker(process: NonZeroU32) -> PathBuf {
    RUNS.directory.join(format!("ready-{process}"))
}

/// One entity the stand-in holds: which of the four kinds it is, its id and its name.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Entity {
    kind: &'static str,
    id: String,
    name: String,
}

/// Everything the stand-in holds, and the one race it can be told to stage.
#[derive(Default)]
struct Workspace {
    entities: Vec<Entity>,
    /// Ids another deleter takes the moment this workspace lists the kind they belong to.
    ///
    /// The race in one switch: the cleanup asked what was there, was told, and by the time
    /// its delete arrives the entity is gone. Staged rather than waited for, because a race
    /// nobody can stage is a race nobody can prove either way.
    vanishing: Vec<String>,
    /// Ids this workspace refuses to delete and goes on listing.
    ///
    /// The other refusal, and the one the tolerance must NOT swallow: a delete that stays
    /// refused while the entity is still there is residue left behind, and a cleanup that
    /// reported success would leave it in somebody's real workspace with nothing said.
    immortal: Vec<String>,
    /// What this workspace has already refused a delete for, so a drive can assert the
    /// tolerance was exercised rather than that nothing was ever deleted.
    refused: Vec<String>,
}

impl Workspace {
    /// Take everything `vanishing` names, as another deleter would.
    fn let_the_race_happen(&mut self, kind: &str) {
        let taken: Vec<String> = self
            .entities
            .iter()
            .filter(|entity| entity.kind == kind && self.vanishing.contains(&entity.id))
            .map(|entity| entity.id.clone())
            .collect();
        self.entities.retain(|entity| !taken.contains(&entity.id));
    }

    fn names(&self) -> Vec<String> {
        sorted(
            self.entities
                .iter()
                .map(|entity| entity.name.clone())
                .collect(),
        )
    }
}

/// The one stand-in this binary drives, since a lane is pointed at one API per binary.
static STANDIN: LazyLock<Arc<Mutex<Workspace>>> = LazyLock::new(|| {
    let workspace = Arc::new(Mutex::new(Workspace::default()));
    let host = serve(Arc::clone(&workspace));
    cleanup::against(&format!("{host}/graphql"));
    workspace
});

/// One drive at a time: the workspace is one shared fixture and every drive sweeps all of it.
///
/// **The standard library's, and it is what a drive holds for its whole length.** Each
/// `#[tokio::test]` is a current-thread runtime of its own on a test thread of its own, so a
/// guard that blocks excludes the other drives without ever blocking the runtime it is held
/// in — and it excludes them whichever runtime they are waiting in, which is the half a
/// runtime-aware guard here left to chance. What that chance cost was a drive whose fixture
/// another drive replaced underneath it, which then passed or failed on the wrong workspace.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

/// The workspace, planted for one drive and nobody else's, for as long as this is held.
///
/// Every reading of the workspace goes through this rather than through the fixture, so a
/// drive cannot read one it does not hold: what the type system says here is the whole of
/// why the exclusion above can be relied on.
struct Drive {
    _exclusive: std::sync::MutexGuard<'static, ()>,
}

impl Drive {
    /// Take the workspace, planted with `entities`, for the length of one drive.
    fn plant(entities: Vec<Entity>, vanishing: Vec<String>, immortal: Vec<String>) -> Self {
        // A drive that failed poisoned nothing of consequence: the next one replants the
        // workspace outright, and reporting a poisoned lock instead would hide the failure
        // that caused it.
        let exclusive = ONE_AT_A_TIME
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *STANDIN
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Workspace {
            entities,
            vanishing,
            immortal,
            refused: Vec::new(),
        };
        Self {
            _exclusive: exclusive,
        }
    }

    /// What the workspace holds now, as sorted names.
    fn left(&self) -> Vec<String> {
        STANDIN
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .names()
    }

    /// Every delete this workspace refused, in the order it refused them.
    fn refused(&self) -> Vec<String> {
        STANDIN
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .refused
            .clone()
    }
}

/// One artifact of `run`, of every kind this lane writes, planted under its own naming.
fn artifacts_of(run: Run, micros: u64, tag: &str) -> Vec<Entity> {
    RESIDUE
        .iter()
        .enumerate()
        .map(|(index, residue)| Entity {
            kind: residue.kind,
            id: format!("{tag}-{index}"),
            name: name_of(residue, run, micros),
        })
        .collect()
}

/// What `run` calls one artifact of this kind, by the lane's own naming.
fn name_of(residue: &Residue, run: Run, micros: u64) -> String {
    if residue.prefix == cleanup::LABEL_PREFIX {
        artifact_label(run, micros)
    } else {
        artifact_title(run, micros)
    }
}

fn sorted(mut names: Vec<String>) -> Vec<String> {
    names.sort();
    names
}

#[tokio::test]
async fn a_sweep_leaves_a_concurrent_live_runs_artifacts_however_old_they_are() {
    // The property the age rule got wrong, driven against the real cleanup. The run beside
    // this one is a REAL second process holding a REAL registration lock, and its artifacts
    // are stamped ten windows ago — which is what a hung session looks like, and what a
    // session Linear's own rate limiter has parked becomes. Nothing about their age is
    // different from an abandoned run's; the lock is the whole of the difference.
    let mut beside = LiveRunBeside::start();
    let ended = ended_run(1);
    let fresh_ended = ended_run(2);
    let theirs = artifacts_of(beside.run, aged(10), "theirs");
    let mine = artifacts_of(RUNS.mine, aged(10), "mine");
    let stale = artifacts_of(ended, aged(2), "stale");
    let fresh = artifacts_of(fresh_ended, NOW, "fresh");
    let nobodys = Entity {
        kind: "issue",
        id: "nobodys".to_owned(),
        name: "AI Orchestrator plan".to_owned(),
    };
    let planted_entities = [
        theirs.clone(),
        mine.clone(),
        stale.clone(),
        fresh.clone(),
        vec![nobodys.clone()],
    ]
    .concat();
    let drive = Drive::plant(planted_entities, vec![], vec![]);

    cleanup::sweep_orphans(KEY, &sweep_at(NOW))
        .await
        .expect("an orphan sweep over a workspace it can read succeeds");

    let survived = |entities: &[Entity]| -> Vec<String> {
        entities.iter().map(|entity| entity.name.clone()).collect()
    };
    assert_eq!(
        drive.left(),
        sorted(
            [
                survived(&theirs),
                survived(&mine),
                survived(&fresh),
                vec![nobodys.name.clone()],
            ]
            .concat()
        ),
        "the sweep took an artifact a live run owns, or left one nothing owns"
    );

    // And the second half of the same property: once that run really ends, the very same
    // artifacts — not one microsecond older in the sweep's eyes, since it is made at the
    // same instant — become the residue this sweep recovers. Nothing changed but the run.
    beside.end();
    cleanup::sweep_orphans(KEY, &sweep_at(NOW))
        .await
        .expect("a second sweep over the same workspace succeeds");
    let remaining = drive.left();
    for gone in survived(&theirs) {
        assert!(
            !remaining.contains(&gone),
            "an ended run's {gone:?} was not recovered: {remaining:?}"
        );
    }
    assert_eq!(
        remaining,
        sorted([survived(&mine), survived(&fresh), vec![nobodys.name]].concat()),
        "the second sweep took more than the run that had ended"
    );
}

#[tokio::test]
async fn an_artifact_another_deleter_took_first_leaves_the_cleanup_successful() {
    // The workspace answers the listing and then the entity is gone — swept by another run,
    // removed by hand, whatever. Linear refuses the delete that follows, and treating that
    // refusal as a failure once killed a whole journey over an issue that had already gone.
    let ended = ended_run(3);
    let racing = artifacts_of(ended, aged(2), "racing");
    let ids: Vec<String> = racing.iter().map(|entity| entity.id.clone()).collect();
    let drive = Drive::plant(racing, ids.clone(), vec![]);

    cleanup::sweep_orphans(KEY, &sweep_at(NOW))
        .await
        .expect("a delete of what has already gone is the outcome the delete was asking for");

    assert!(drive.left().is_empty(), "{:?}", drive.left());
    // And the tolerance was really exercised: the workspace refused every delete rather than
    // this having passed because nothing was ever asked of it.
    assert_eq!(
        sorted(drive.refused()),
        sorted(ids),
        "the workspace did not refuse the deletes this case is about"
    );
}

#[tokio::test]
async fn this_runs_own_cleanup_removes_everything_it_wrote_and_nothing_else() {
    // The other half of the arrangement: the sweep above recovers an ended run's, and this
    // removes this run's own — whether the journey passed or failed, which is
    // `run_then_cleanup`'s and is asserted below.
    let ended = ended_run(4);
    let mine = artifacts_of(RUNS.mine, NOW, "mine");
    let also_mine = artifacts_of(RUNS.mine, NOW + 1, "also-mine");
    let theirs = artifacts_of(ended, NOW, "theirs");
    let drive = Drive::plant(
        [mine.clone(), also_mine.clone(), theirs.clone()].concat(),
        vec![],
        vec![],
    );

    let run = RUNS.mine;
    cleanup::remove_artifacts(KEY, &|prefix, name| is_this_runs(run, prefix, name))
        .await
        .expect("this run's own cleanup over a workspace it can read succeeds");

    assert_eq!(
        drive.left(),
        sorted(theirs.iter().map(|entity| entity.name.clone()).collect()),
        "this run's cleanup left its own work, or took a run's that is not it"
    );
}

#[tokio::test]
async fn residue_a_delete_never_takes_fails_the_cleanup_rather_than_passing_quietly() {
    // The other side of the tolerance, and the side that has to stay sharp. A delete refused
    // for something that has already GONE is the outcome the delete was asking for; a delete
    // refused for something still there is residue left in somebody's real workspace, and a
    // cleanup that reported success would leave it there with nothing said.
    let ended = ended_run(5);
    let stuck = artifacts_of(ended, aged(2), "stuck");
    let ids: Vec<String> = stuck.iter().map(|entity| entity.id.clone()).collect();
    let names: Vec<String> = stuck.iter().map(|entity| entity.name.clone()).collect();
    let drive = Drive::plant(stuck, vec![], ids.clone());

    let refusal = cleanup::sweep_orphans(KEY, &sweep_at(NOW))
        .await
        .expect_err("a sweep that left residue behind has not swept");

    // Every kind this lane writes is named, because residue of any of them is residue the
    // next run inherits.
    for kind in RESIDUE.iter().map(|residue| residue.kind) {
        assert!(
            refusal.contains(kind),
            "the refusal has to name the {kind} it could not remove: {refusal}"
        );
    }
    assert_eq!(drive.left(), sorted(names));
}

#[tokio::test]
async fn cleanup_runs_whether_the_journey_passed_or_failed() {
    let ran = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    for journey in [Ok(()), Err("the journey failed".to_owned())] {
        let failed = journey.is_err();
        let observed = std::sync::Arc::clone(&ran);
        let outcome = cleanup::run_then_cleanup(
            || async { journey },
            || async move {
                observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            },
        )
        .await;
        assert_eq!(outcome.is_err(), failed, "{outcome:?}");
    }
    assert_eq!(
        ran.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "cleanup has to run after a journey that failed as well as one that passed"
    );
}

/// A local stand-in for the workspace this lane's cleanup reaches.
///
/// Every answer below is derived from `cleanup::RESIDUE` — the same table the cleanup itself
/// listed and deleted through — so a stand-in and a lane cannot come to disagree about which
/// document lists what. Every document it does not recognise is answered with a GraphQL
/// error and nothing changes, so a cleanup that started asking something else would fail
/// here rather than pass quietly.
fn serve(workspace: Arc<Mutex<Workspace>>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a stand-in listener");
    let host = format!("http://{}", listener.local_addr().unwrap());
    thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.expect("a stand-in connection");
            let body = read_request(&mut stream);
            let payload = graphql(&workspace, &body).to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                payload.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    host
}

/// The stand-in's answer to one GraphQL document.
fn graphql(workspace: &Mutex<Workspace>, body: &str) -> Value {
    let request: Value = match serde_json::from_str(body) {
        Ok(request) => request,
        Err(problem) => return errors(&format!("unreadable request: {problem}")),
    };
    let query = request.get("query").and_then(Value::as_str).unwrap_or("");
    let variables = request.get("variables").cloned().unwrap_or(Value::Null);
    let mut held = workspace
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(residue) = RESIDUE.iter().find(|residue| residue.listing == query) {
        return list(&mut held, residue, &variables);
    }
    if let Some(residue) = RESIDUE.iter().find(|residue| residue.delete == query) {
        return delete(&mut held, residue, &variables);
    }
    errors(&format!(
        "this stand-in was asked something it does not answer: {query}"
    ))
}

/// The page of `residue` this workspace holds, narrowed the way that listing narrows.
fn list(workspace: &mut Workspace, residue: &Residue, variables: &Value) -> Value {
    let prefix = variables.get("prefix").and_then(Value::as_str);
    if residue.narrowed && prefix != Some(residue.prefix) {
        return errors(&format!(
            "the {} listing narrowed by {prefix:?} rather than by its own prefix",
            residue.kind
        ));
    }
    let after = variables.get("after").and_then(Value::as_str);
    let matching: Vec<&Entity> = workspace
        .entities
        .iter()
        .filter(|entity| entity.kind == residue.kind)
        .filter(|entity| !residue.narrowed || entity.name.starts_with(residue.prefix))
        .collect();
    // One node per page, so every drive above walks a connection to exhaustion rather than
    // reading one page and stopping — which is what the cleanup's own cursor guard is for.
    let from = after.map_or(0, |cursor| {
        matching
            .iter()
            .position(|entity| entity.id == cursor)
            .map_or(matching.len(), |at| at + 1)
    });
    let node = matching.get(from);
    let nodes: Vec<Value> = node
        .map(|entity| json!({"id": entity.id, residue.name_field: entity.name}))
        .into_iter()
        .collect();
    let page = json!({"data": {residue.connection: {
        "nodes": nodes,
        "pageInfo": {
            "hasNextPage": from + 1 < matching.len(),
            "endCursor": node.map(|entity| entity.id.clone()),
        },
    }}});
    // The race, staged the moment this listing has named them: the cleanup was told what is
    // there, and another deleter takes it before the delete arrives. Staged after the answer
    // rather than before it, because a thing that vanished before the listing that names it
    // is not the race — it is an empty listing.
    workspace.let_the_race_happen(residue.kind);
    page
}

/// Delete what `variables.id` names, or refuse because it is no longer there.
fn delete(workspace: &mut Workspace, residue: &Residue, variables: &Value) -> Value {
    let Some(id) = variables.get("id").and_then(Value::as_str) else {
        return errors("a delete arrived with no id");
    };
    if workspace.immortal.iter().any(|held| held == id) {
        workspace.refused.push(id.to_owned());
        return errors(&format!("this workspace will not part with {id}"));
    }
    let held = workspace
        .entities
        .iter()
        .position(|entity| entity.id == id && entity.kind == residue.kind);
    let Some(at) = held else {
        // Exactly what Linear does with a delete of what has already gone, and the outcome
        // the delete was asking for.
        workspace.refused.push(id.to_owned());
        return errors(&format!("Entity not found: {id}"));
    };
    workspace.entities.remove(at);
    // `confirm` is a JSON pointer into the mutation's own answer, so the answer is built
    // from it rather than from a second spelling of the same field name.
    let mut answer = json!({});
    let mut at = &mut answer;
    let path: Vec<&str> = residue
        .confirm
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    for (depth, part) in path.iter().enumerate() {
        let value = if depth + 1 == path.len() {
            Value::Bool(true)
        } else {
            json!({})
        };
        at[*part] = value;
        at = &mut at[*part];
    }
    json!({ "data": answer })
}

fn errors(message: &str) -> Value {
    json!({"errors":[{"message":message}]})
}

/// The body of one HTTP request, once its headers have said how long it is.
fn read_request(stream: &mut std::net::TcpStream) -> String {
    let mut raw = Vec::new();
    let mut buffer = [0_u8; 1024];
    loop {
        let read = stream.read(&mut buffer).expect("a stand-in request");
        if read == 0 {
            break;
        }
        raw.extend_from_slice(&buffer[..read]);
        let text = String::from_utf8_lossy(&raw).into_owned();
        let Some(head) = text.split("\r\n\r\n").next() else {
            continue;
        };
        let length: usize = head
            .lines()
            .find_map(|line| {
                line.strip_prefix("Content-Length: ")
                    .or_else(|| line.strip_prefix("content-length: "))
            })
            .and_then(|value| value.trim().parse().ok())
            .unwrap_or(0);
        if text.contains("\r\n\r\n") && text.len() >= head.len() + 4 + length {
            return text[head.len() + 4..].to_owned();
        }
    }
    String::new()
}
