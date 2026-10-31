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
//! One `bun` process per test process holds the schema, built once, and answers one JSON line
//! per request; requests from every thread are put to it in turn. A request nested past what
//! it can recurse through is refused rather than crashing it — JavaScript reports an exhausted
//! stack as an exception — and the variables' nesting is bounded here before they are sent.
//!
//! What it does not hold a request to is what only GitHub's executor knows — a node id's
//! existence, a permission, a rate or a node-count limit, a custom scalar's spelling. Its
//! messages are graphql-js's, not GitHub's wording; the shape of a refusal, an `errors` array
//! of `{message, locations}` objects and no `data`, is GraphQL's own (§7.1.2).
//!
//! Included by path from the loopback board in `onetaskgraph-e2e-support` as well as from this
//! crate's own tests, so both doubles hold to one implementation.

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Mutex, OnceLock};

// llmlint: ignore[contracts_have_one_source_or_a_drift_gate] A dated snapshot of an external contract by design: it is pinned to the provenance in fixtures/README.md, which states its gates and why upstream freshness is not one.
/// GitHub's published GraphQL schema, whole.
pub const SCHEMA: &str = include_str!("../fixtures/github-schema.docs.graphql");

/// The deepest a request's variables may nest before they are refused unsent: far beyond any
/// value this source sends, and far short of what serializing them could exhaust.
const NESTING_LIMIT: usize = 200;

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

/// The running validator: `validate.mjs` under `bun`, with the pipes it is spoken to over.
struct Validator {
    // Held so the process lives as long as this one; it ends on its own when its standard
    // input closes with this process.
    _child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    /// The SHA-256, in lowercase hex, of the schema file the process built.
    schema_sha256: String,
}

impl Validator {
    fn start() -> Self {
        let root = workspace_root();
        let here = here();
        let mut child = Command::new("bun")
            .arg(here.join("validate.mjs"))
            .arg(here.join("../fixtures/github-schema.docs.graphql"))
            .current_dir(&root)
            // Running a script writes nothing it would otherwise cache.
            .env("BUN_RUNTIME_TRANSPILER_CACHE_PATH", "0")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap_or_else(|error| {
                panic!(
                    "the GitHub schema check could not start `bun` ({error}); install the \
                     workspace's toolchain with `just bootstrap`"
                )
            });
        let input = child.stdin.take().expect("a piped standard input");
        let output = BufReader::new(child.stdout.take().expect("a piped standard output"));
        let mut validator = Self {
            _child: child,
            input,
            output,
            schema_sha256: String::new(),
        };
        let ready = validator.read();
        assert_eq!(
            ready["ready"],
            json!(true),
            "the GitHub schema check did not start: {ready}"
        );
        validator.schema_sha256 = ready["schema_sha256"]
            .as_str()
            .expect("the GitHub schema check reports the schema's digest")
            .to_owned();
        validator
    }

    fn read(&mut self) -> Value {
        let mut line = String::new();
        let read = self
            .output
            .read_line(&mut line)
            .expect("the GitHub schema check's answer is readable");
        assert!(
            read > 0,
            "the GitHub schema check ended without answering; its standard error above says why"
        );
        serde_json::from_str(&line).expect("the GitHub schema check answers one JSON line")
    }

    fn ask(&mut self, request: &Value) -> Vec<Value> {
        let mut line = request.to_string();
        line.push('\n');
        self.input
            .write_all(line.as_bytes())
            .and_then(|()| self.input.flush())
            .expect("the GitHub schema check takes a request");
        let answer = self.read();
        if let Some(internal) = answer.get("internal") {
            panic!("the GitHub schema check failed on {request}: {internal}");
        }
        answer["errors"]
            .as_array()
            .cloned()
            .expect("the GitHub schema check answers `errors`")
    }
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
    });
    validator().ask(&request)
}

/// The one validator of this test process, started on first use.
fn validator() -> std::sync::MutexGuard<'static, Validator> {
    static VALIDATOR: OnceLock<Mutex<Validator>> = OnceLock::new();
    VALIDATOR
        .get_or_init(|| Mutex::new(Validator::start()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Starts this process's validator now, if it has not started, rather than inside the first
/// request a double answers: building GitHub's schema takes a fraction of a second, and a
/// double whose test times its answers on the real clock would otherwise count that against
/// whichever request came first. Each double calls it before it starts listening.
pub fn warm() {
    drop(validator());
}

/// The SHA-256, in lowercase hex, of the schema file the validator built — [`SCHEMA`]'s bytes,
/// read from the same path.
pub fn schema_sha256() -> String {
    validator().schema_sha256.clone()
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
