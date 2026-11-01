//! GitHub's static validation of a GraphQL request, against the complete schema GitHub
//! publishes.
//!
//! Every GitHub double here puts each request to [`validate`] first and answers a refusal the
//! way GitHub does — HTTP 200 with an `errors` array and no `data` — so a request GitHub would
//! refuse cannot pass a test. The schema is `fixtures/github-schema.docs.graphql`, GitHub's
//! published file whole; `fixtures/README.md` holds its provenance and why it is not a
//! reduction.
//!
//! The validation is graphql-js's — the GraphQL Foundation's reference implementation, npm's
//! own `graphql-17.0.2.tgz` committed beside this file and held to npm's published integrity —
//! run by `validate.mjs` beside this file: every rule of the
//! specification's Validation section (`specifiedRules`) and, for a request, the coercion of
//! its variables (`CoerceVariableValues`, §6.1.2). It is not a Rust crate because none fits this
//! workspace's dependency policy unchanged: `apollo-compiler` brings second versions of
//! `bitflags`, `windows-sys` and `hashbrown`, `bluejay-validator` a second `unicode-width`, and
//! `graphql-tools` a licence the policy does not admit.
//!
//! It is not in the workspace's `package.json` either, because that file is an input of every
//! project in the graph: naming it there selects every crate's tests, the live lanes included,
//! for a change to this one crate's test double. `validate.mjs` unpacks the tarball into the
//! ignored `node_modules` beside it and imports it from there, holding that copy to the tarball's
//! files byte for byte on every start and unpacking it again when it differs.
//!
//! One `bun` process per clone serves every test process on the machine (see [`Validators`]):
//! building GitHub's schema takes a fifth of a second and a quarter of a gigabyte, and once per
//! test process — nextest runs each test in a process of its own — that added about a fifth to
//! the wall time of the suites these doubles serve. A test that would hold that server up for
//! the others — stopping it, or sending documents that take seconds to refuse — takes one of its
//! own instead ([`serve_privately`]).
//! A request nested past what it can recurse through is refused rather than crashing it —
//! JavaScript reports an exhausted stack as an exception — and the variables' nesting is bounded
//! here before they are sent.
//!
//! What it does not hold a request to is what only GitHub's executor knows — a node id's
//! existence, a permission, a rate or a node-count limit, a custom scalar's spelling. Its
//! messages are graphql-js's, not GitHub's wording; the shape of a refusal, an `errors` array
//! of `{message, locations}` objects and no `data`, is GraphQL's own (§7.1.2).
//!
//! Included by path from the loopback board in `onetaskgraph-e2e-support` as well as from this
//! crate's own tests, so both doubles hold to one implementation.

use serde_json::{Value, json};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

// llmlint: ignore[contracts_have_one_source_or_a_drift_gate] A dated snapshot of an external contract by design: it is pinned to the provenance in fixtures/README.md, which states its gates and why upstream freshness is not one.
/// GitHub's published GraphQL schema, whole.
pub const SCHEMA: &str = include_str!("../fixtures/github-schema.docs.graphql");

/// The deepest a request's variables may nest before they are refused unsent: far beyond any
/// value this source sends, and far short of what serializing them could exhaust.
const NESTING_LIMIT: usize = 200;

/// The protocol `validate.mjs` speaks, sent in each connection's hello; it is `PROTOCOL` there.
const PROTOCOL: u64 = 1;

/// How many times a request is put to a server before the check gives up: a server stops when it
/// is stale or idle, so a request in flight then is sent again to the one started after it.
const ATTEMPTS: usize = 4;

/// The longest one answer may take: far beyond the largest document this source sends, and a
/// bound on how long a server that has stopped answering can hold a test.
const ANSWER_WITHIN: Duration = Duration::from_secs(120);

/// The workspace root: every crate including this module sits at `crates/<name>`.
fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// This module's own directory, which holds `validate.mjs` and the tarball beside it.
///
/// Found from where this file is rather than from a crate's name, because the directory a
/// crate is built from is not always its name: `scripts/check-fixture-discrimination.sh`
/// builds this crate's tests from a copy under another name. `file!()` is this file's path as
/// cargo handed it to rustc — relative to the workspace root, the directory rustc runs in, or
/// absolute — so joining it to that root names this file whichever crate includes it.
fn here() -> PathBuf {
    let file = workspace_root().join(file!());
    file.parent()
        .expect("a source file has a parent directory")
        .to_path_buf()
}

/// Where this clone's server records its port and its starters take their lock: inside the
/// ignored `node_modules` the check already unpacks graphql-js into — or, in a process that has
/// asked for a server of its own ([`serve_privately`]), that server's scratch directory.
fn rendezvous() -> PathBuf {
    let directory = PRIVATE.get().map_or_else(
        || here().join("node_modules").join(".github-schema-server"),
        Clone::clone,
    );
    std::fs::create_dir_all(&directory).unwrap_or_else(|error| {
        panic!(
            "the GitHub schema check could not create {} ({error})",
            directory.display()
        )
    });
    directory
}

/// One connection to the server, greeted.
struct Connection {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
}

impl Connection {
    /// Connects to the server at `port` and greets it: `Ok` with the connection and the
    /// schema's digest once it answers ready, `Err` when nothing current answers there.
    fn open(port: u16) -> io::Result<(Self, String)> {
        let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        let writer = TcpStream::connect_timeout(&address, Duration::from_secs(5))?;
        writer.set_nodelay(true)?;
        writer.set_read_timeout(Some(ANSWER_WITHIN))?;
        let mut connection = Self {
            reader: BufReader::new(writer.try_clone()?),
            writer,
        };
        let ready = connection.exchange(&json!({ "hello": PROTOCOL }).to_string())?;
        match ready["schema_sha256"].as_str() {
            Some(digest) if ready["ready"] == json!(true) => Ok((connection, digest.to_owned())),
            _ => Err(io::Error::other(format!("the server answered {ready}"))),
        }
    }

    fn exchange(&mut self, line: &str) -> io::Result<Value> {
        self.writer.write_all(format!("{line}\n").as_bytes())?;
        let mut answer = String::new();
        if self.reader.read_line(&mut answer)? == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        serde_json::from_str(&answer).map_err(io::Error::other)
    }
}

/// The server this clone's check runs, shared by every test process on the machine; the
/// connections this process keeps open to it; and the answers it has given.
///
/// A process takes an idle connection from [`Validators::idle`] for each request, opening one
/// when none is idle, so its test threads' requests go to the server side by side rather than
/// in turn behind one lock. The server answers them as they arrive, keeping each document's
/// validation so one sent again with new variables costs only their coercion, and a request
/// already answered is answered from [`Validators::answers`] without being sent — the validation
/// is a function of the request and the schema alone, both fixed while a server runs.
///
/// The server records its port in [`rendezvous`]'s `endpoint.json`. A process that finds none
/// answering there takes the OS lock on `start.lock` beside it, looks once more, and starts one,
/// so concurrent first requests start one server between them. It is started in a process group
/// of its own, outside any test's, so a test that times out does not take it down; and it
/// stops by itself once idle, so nothing is left running after the tests.
struct Validators {
    idle: Mutex<Vec<Connection>>,
    schema_sha256: OnceLock<String>,
    answers: Mutex<HashMap<String, Vec<Value>>>,
}

impl Validators {
    fn kept(&self) -> MutexGuard<'_, HashMap<String, Vec<Value>>> {
        self.answers.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn idle(&self) -> MutexGuard<'_, Vec<Connection>> {
        self.idle.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// An idle connection, or a new one to the running server, which this starts if none runs.
    fn connection(&self) -> Connection {
        if let Some(connection) = self.idle().pop() {
            return connection;
        }
        let directory = rendezvous();
        let mut tried = Vec::new();
        for _ in 0..ATTEMPTS {
            match self.running(&directory) {
                Ok(connection) => return connection,
                Err(error) => tried.push(error.to_string()),
            }
            let lock = File::create(directory.join("start.lock"))
                .and_then(|lock| lock.lock().map(|()| lock))
                .expect("the GitHub schema check's start lock can be taken");
            let started = self.running(&directory).or_else(|_| self.start(&directory));
            drop(lock);
            match started {
                Ok(connection) => return connection,
                Err(error) => tried.push(error.to_string()),
            }
        }
        panic!(
            "the GitHub schema check could not be reached ({}); its server's standard error is \
             in {}",
            tried.join("; "),
            directory.join("server.log").display()
        )
    }

    /// A connection to the server `endpoint.json` names, if it answers ready.
    fn running(&self, directory: &std::path::Path) -> io::Result<Connection> {
        let endpoint: Value =
            serde_json::from_slice(&std::fs::read(directory.join("endpoint.json"))?)
                .map_err(io::Error::other)?;
        let port = endpoint["port"]
            .as_u64()
            .and_then(|port| u16::try_from(port).ok())
            .ok_or_else(|| io::Error::other(format!("endpoint.json holds {endpoint}")))?;
        let (connection, digest) = Connection::open(port)?;
        self.schema_sha256.get_or_init(|| digest);
        Ok(connection)
    }

    /// Starts a server, waits for it to report its port, and connects to it.
    fn start(&self, directory: &std::path::Path) -> io::Result<Connection> {
        let here = here();
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(directory.join("server.log"))?;
        let mut command = Command::new("bun");
        command
            .arg(here.join("validate.mjs"))
            .arg(here.join("../fixtures/github-schema.docs.graphql"))
            .arg("--serve")
            .arg(directory)
            // Outside the clone, so a server still idling holds no directory a test removes.
            .current_dir(std::env::temp_dir())
            // Running a script writes nothing it would otherwise cache.
            .env("BUN_RUNTIME_TRANSPILER_CACHE_PATH", "0")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(log);
        detach(&mut command);
        let mut child = spawn(&mut command).unwrap_or_else(|error| {
            panic!(
                "the GitHub schema check could not start `bun` ({error}); install the \
                 workspace's toolchain with `just bootstrap`"
            )
        });
        let output = child.stdout.take().expect("a piped standard output");
        // Reaped whenever it stops, on a thread nothing waits for: the server outlives the test
        // that started it more often than not, and this process ends without waiting for it.
        std::thread::spawn(move || child.wait());
        let mut ready = String::new();
        BufReader::new(output).read_line(&mut ready)?;
        let ready: Value = serde_json::from_str(&ready)
            .map_err(|_| io::Error::other(format!("the server started by answering {ready:?}")))?;
        if let Some(internal) = ready.get("internal") {
            panic!("the GitHub schema check did not start: {internal}");
        }
        let port = ready["port"]
            .as_u64()
            .and_then(|port| u16::try_from(port).ok())
            .ok_or_else(|| io::Error::other(format!("the server started with {ready}")))?;
        let (connection, digest) = Connection::open(port)?;
        self.schema_sha256.get_or_init(|| digest);
        Ok(connection)
    }

    fn ask(&self, request: &str) -> Vec<Value> {
        let mut failures = Vec::new();
        for _ in 0..ATTEMPTS {
            let mut connection = self.connection();
            match connection.exchange(request) {
                Ok(answer) => {
                    self.idle().push(connection);
                    if let Some(internal) = answer.get("internal") {
                        panic!("the GitHub schema check failed on {request}: {internal}");
                    }
                    return answer["errors"]
                        .as_array()
                        .cloned()
                        .expect("the GitHub schema check answers `errors`");
                }
                // The server stopped: every connection to it is closed, so none is kept.
                Err(error) => {
                    failures.push(error.to_string());
                    self.idle().clear();
                }
            }
        }
        panic!(
            "the GitHub schema check did not answer {request}: {}",
            failures.join("; ")
        )
    }

    fn answer(&self, request: String) -> Vec<Value> {
        if let Some(answer) = self.kept().get(&request) {
            return answer.clone();
        }
        let answer = self.ask(&request);
        let mut answers = self.kept();
        if answers.len() >= ANSWERS_KEPT {
            answers.clear();
        }
        answers.insert(request, answer.clone());
        answer
    }
}

/// Puts the server in a process group of its own, so the signal that ends a test's group — a
/// timeout, an interrupt — does not end the server other tests are using.
#[cfg(unix)]
fn detach(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

/// A new process group, so a console interrupt to the test's group does not reach the server.
#[cfg(windows)]
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
/// No console window for the server.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// Outside the job object the test runs in, so ending that job does not end the server.
#[cfg(windows)]
const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;

/// Puts the server in a process group of its own and, where the job the test runs in allows it,
/// outside that job, so ending the test's job does not end the server other tests are using.
#[cfg(windows)]
fn detach(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    command.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW | CREATE_BREAKAWAY_FROM_JOB);
}

#[cfg(not(any(unix, windows)))]
fn detach(_command: &mut Command) {}

/// Spawns `command`; on Windows, inside the test's job when that job refuses a breakaway, in
/// which case a server outlives only the tests whose job it was started in and is started again
/// after.
fn spawn(command: &mut Command) -> io::Result<std::process::Child> {
    let spawned = command.spawn();
    #[cfg(windows)]
    if spawned.is_err() {
        use std::os::windows::process::CommandExt;
        return command
            .creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW)
            .spawn();
    }
    spawned
}

/// Every error GitHub's static validation answers `document`, sent with `operation_name` and
/// `variables`, with; empty when GitHub would run it. `variables` of `None` skips their
/// coercion, for a document whose values a caller supplies later.
fn errors(document: &str, operation_name: Option<&str>, variables: Option<&Value>) -> Vec<Value> {
    if variables.is_some_and(|variables| nesting(variables) > NESTING_LIMIT) {
        return vec![
            json!({"message": format!("Variables nest deeper than {NESTING_LIMIT} levels")}),
        ];
    }
    let request = json!({
        "query": document,
        "operationName": operation_name,
        "variables": variables,
        "coerce": variables.is_some(),
    })
    .to_string();
    validators().answer(request)
}

/// The most answers [`Validators`] keeps before it starts again from none: far more distinct
/// requests than one test process sends, and a bound on what an adversarial test can make it
/// hold.
const ANSWERS_KEPT: usize = 16_384;

/// This process's way to the server, connected on first use.
static VALIDATORS: OnceLock<Validators> = OnceLock::new();

/// The scratch directory of this process's own server, once it has asked for one.
static PRIVATE: OnceLock<PathBuf> = OnceLock::new();

/// A server of this process's own, in place of the one every test process shares, until it is
/// dropped: for a test whose requests would hold the shared one up for every other process —
/// one that stops a server, or that sends documents each taking seconds to refuse. The server
/// is single-threaded, so either would add those seconds to whatever another test's double is
/// answering at the time, and a test timing its answers on the real clock would count them.
///
/// Taken before the process's first request, and once per process; nextest runs each test in a
/// process of its own. Dropping it stops the server and removes its directory.
#[must_use]
pub struct PrivateServer(());

/// Gives this process a server of its own; see [`PrivateServer`].
pub fn serve_privately() -> PrivateServer {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    let directory = std::env::temp_dir().join(format!(
        "onetaskgraph-github-schema-{}-{nanos}",
        std::process::id()
    ));
    assert!(
        VALIDATORS.get().is_none() && PRIVATE.set(directory).is_ok(),
        "a private GitHub schema check is asked for before this process's first request, once"
    );
    PrivateServer(())
}

impl Drop for PrivateServer {
    fn drop(&mut self) {
        stop();
        // The server has stopped listening; on Windows its log stays open until it has exited.
        let directory = rendezvous();
        let stopped = std::time::Instant::now();
        while std::fs::remove_dir_all(&directory).is_err() && directory.exists() {
            if stopped.elapsed() > Duration::from_secs(10) {
                eprintln!(
                    "the private GitHub schema check's directory {} could not be removed",
                    directory.display()
                );
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

/// Stops the server this process's checks use, if one is running, and waits until it no longer
/// listens: a client greeting it with a protocol it does not speak is answered `stale`, and the
/// server stops.
pub fn stop() {
    validators().idle().clear();
    let Ok(endpoint) = std::fs::read(rendezvous().join("endpoint.json")) else {
        return;
    };
    let Some(port) = serde_json::from_slice::<Value>(&endpoint)
        .ok()
        .and_then(|endpoint| endpoint["port"].as_u64())
        .and_then(|port| u16::try_from(port).ok())
    else {
        return;
    };
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    if let Ok(mut stream) = TcpStream::connect_timeout(&address, Duration::from_secs(5)) {
        let _ = stream.write_all(b"{\"hello\":0}\n");
        let _ = stream.set_read_timeout(Some(ANSWER_WITHIN));
        let _ = BufReader::new(stream).read_line(&mut String::new());
    }
    let asked = std::time::Instant::now();
    while TcpStream::connect_timeout(&address, Duration::from_secs(1)).is_ok()
        && asked.elapsed() < ANSWER_WITHIN
    {
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn validators() -> &'static Validators {
    VALIDATORS.get_or_init(|| {
        let validators = Validators {
            idle: Mutex::new(Vec::new()),
            schema_sha256: OnceLock::new(),
            answers: Mutex::new(HashMap::new()),
        };
        let connection = validators.connection();
        validators.idle().push(connection);
        validators
    })
}

/// Connects this process to the server now, starting it if none runs, rather than inside the
/// first request a double answers: building GitHub's schema takes a fraction of a second, and a
/// double whose test times its answers on the real clock would otherwise count that against
/// whichever request came first. Each double calls it before it starts listening.
pub fn warm() {
    validators();
}

/// The SHA-256, in lowercase hex, of the schema file the server built — [`SCHEMA`]'s bytes,
/// read from the same path.
pub fn schema_sha256() -> String {
    validators()
        .schema_sha256
        .get()
        .expect("a connected server reported the schema's digest")
        .clone()
}

/// The port and process id of the server this clone's checks share, as it records them,
/// connecting this process to it first.
pub fn server() -> (u16, u64) {
    validators();
    let endpoint: Value = serde_json::from_slice(
        &std::fs::read(rendezvous().join("endpoint.json")).expect("a running server's endpoint"),
    )
    .expect("the endpoint is JSON");
    (
        endpoint["port"]
            .as_u64()
            .and_then(|port| u16::try_from(port).ok())
            .expect("the endpoint names a port"),
        endpoint["pid"].as_u64().expect("the endpoint names a pid"),
    )
}

/// How deeply `value` nests, measured without recursion so that no depth of it can exhaust
/// the stack before it is refused.
fn nesting(value: &Value) -> usize {
    let mut deepest = 0;
    let mut pending = vec![(value, 1)];
    while let Some((value, depth)) = pending.pop() {
        deepest = deepest.max(depth);
        match value {
            Value::Array(items) => pending.extend(items.iter().map(|item| (item, depth + 1))),
            Value::Object(fields) => pending.extend(fields.values().map(|item| (item, depth + 1))),
            _ => {}
        }
    }
    deepest
}

fn messages(errors: Vec<Value>) -> Result<(), Vec<String>> {
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors
            .iter()
            .map(|error| error["message"].as_str().unwrap_or_default().to_owned())
            .collect())
    }
}

/// Whether GitHub's static validation accepts `document` sent with `variables`; `Err` carries
/// each message it would answer, in order.
pub fn validate(document: &str, variables: &Value) -> Result<(), Vec<String>> {
    messages(errors(document, None, Some(variables)))
}

/// [`validate`] of the document alone: every rule but the coercion of the variables' values,
/// for a document whose values a caller supplies later.
pub fn validate_document(document: &str) -> Result<(), Vec<String>> {
    messages(errors(document, None, None))
}

/// `body`, a GraphQL request's JSON — `query`, and `operationName` and `variables` when given —
/// put to GitHub's static validation: `Some` refusal body when GitHub would refuse it, in
/// GraphQL's error shape with no `data`, and `None` when it would run it or when `body` is not
/// a GraphQL request.
#[must_use]
pub fn refuse(body: &Value) -> Option<Value> {
    let document = body.get("query")?.as_str()?;
    let operation_name = match body.get("operationName") {
        None | Some(Value::Null) => None,
        Some(Value::String(name)) => Some(name.as_str()),
        Some(other) => {
            return Some(json!({"errors": [{
                "message": format!("operationName must be a string, not {other}")
            }]}));
        }
    };
    let empty = Value::Object(serde_json::Map::new());
    let variables = body
        .get("variables")
        .filter(|value| !value.is_null())
        .unwrap_or(&empty);
    let errors = errors(document, operation_name, Some(variables));
    (!errors.is_empty()).then(|| json!({ "errors": errors }))
}
