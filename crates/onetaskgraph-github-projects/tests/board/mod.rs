//! The half of a fixture board that answers a *session*, and how one is made to disagree.
//!
//! A whole session of the live journey is the source's own reads and writes plus the
//! journey's own calls, and three of the journey's are answered from this workspace's own
//! tables and calculations rather than from any board state: the `rateLimit(dryRun: true)`
//! probe, the mutation-schema introspection, and the allowance read. That is what makes them
//! shareable — a binary with no board at all can answer them.
//!
//! Two binaries do, and they answer with **this** code rather than with two spellings of it:
//! `tests/plugin.rs`, whose board agrees with this workspace and measures what a session
//! costs, and `tests/reconciliation_gate.rs`, whose board is configured by [`Pricing`] to
//! report a price this workspace does not compute, so the journey can be watched refusing
//! one. A second spelling in the second binary would make that gate a proof about the
//! spelling rather than about the fixture.

use std::io::Read;

use serde_json::{Value, json};

/// The whole allowance a fixture board reports, in its rate-limit headers and its answers.
///
/// **Deliberately not GitHub's published hourly figure, and that is what makes the
/// assertions resting on it evidence.** A fixture that mirrored the real allowance would
/// restate GitHub's contract with nothing reconciling the two, and — worse — a report that
/// printed a number both sides already knew would pass whether or not it had read a single
/// header. This one is a board's own, so the only way a report can carry it is by having
/// read what that board sent.
pub const FIXTURE_BUDGET_LIMIT: u64 = 4_321;

/// The UTC epoch second a fixture board says its budgets come back.
///
/// One fixed number, so a refusal naming it can be asserted on rather than approximated.
pub const ALLOWANCE_RESETS_AT: u64 = 1_775_000_000;

/// What GitHub's rate-limit endpoint answers for an account with room to spare.
///
/// The shape is built by `journey::budget::documented_answer` from the names pinned in
/// `fixtures/rate-limits.json`, so a board cannot come to answer a shape the precondition no
/// longer reads. The figures are the board's rather than GitHub's published ones, for the
/// reason [`FIXTURE_BUDGET_LIMIT`] is: a fixture that restated the real allowance would let a
/// gate pass without ever having read what it was sent.
pub fn ample_allowance() -> Value {
    crate::journey::budget::documented_answer(
        FIXTURE_BUDGET_LIMIT,
        FIXTURE_BUDGET_LIMIT,
        FIXTURE_BUDGET_LIMIT,
        ALLOWANCE_RESETS_AT,
    )
}

/// What a board charges for a document, against what this workspace computes it costs.
///
/// A board answering [`Pricing::AsComputed`] agrees with this workspace by construction,
/// which is what the whole-session drive needs and exactly why that drive is not evidence
/// that a disagreement would be noticed. [`Pricing::Overstating`] is the configuration that
/// makes a board disagree, and it is the only way one does: the count and the price a board
/// reports are otherwise this workspace's own.
#[derive(Clone, Copy, Debug)]
pub enum Pricing {
    /// GitHub's own price, as this workspace computes it: what a truthful board reports.
    AsComputed,
    /// A price `by` points above what this workspace computes, which is what GitHub
    /// pricing a connection this workspace prices at nothing would look like from here.
    Overstating {
        /// How many points above the computed price this board reports.
        by: u64,
    },
}

impl Pricing {
    /// What a board configured this way reports for a document costing `computed` points.
    fn charged(self, computed: u64) -> u64 {
        match self {
            Self::AsComputed => computed,
            Self::Overstating { by } => computed.saturating_add(by),
        }
    }
}

/// The calls a session makes that no board state answers, or `None` when this is not one.
///
/// The probe is checked first: the production document it was joined to is still in the
/// text, and answering that would run a query GitHub would not have.
pub fn answer_a_stateless_session_call(query: &str, pricing: Pricing) -> Option<Value> {
    if let Some(production) = strip_probe(query) {
        return Some(probe_answer(&production, pricing));
    }
    if query.contains("__type(name:") {
        return Some(introspected(query));
    }
    if query.contains("rateLimit{") {
        return Some(json!({"rateLimit":{"cost":1,"limit":FIXTURE_BUDGET_LIMIT,
            "remaining":FIXTURE_BUDGET_LIMIT,"resetAt":"2026-01-01T00:00:00Z"}}));
    }
    None
}

/// What a board answers a `rateLimit(dryRun: true)` probe about `production` with.
///
/// The node count is this workspace's own and so, under [`Pricing::AsComputed`], is the
/// price: GitHub is the authority on both, and the credentialed lane is where they are
/// really reconciled. What a board reporting something else does to a run is
/// `tests/reconciliation_gate.rs`.
pub fn probe_answer(production: &str, pricing: Pricing) -> Value {
    let computed = onetaskgraph_github_projects::worst_case_point_cost(production)
        .expect("a priceable production document");
    json!({"rateLimit":{"cost":pricing.charged(computed),
        "nodeCount":onetaskgraph_github_projects::worst_case_node_count(production)
            .expect("a countable production document"),
        "limit":FIXTURE_BUDGET_LIMIT,"remaining":FIXTURE_BUDGET_LIMIT}})
}

/// The production half of a `rateLimit(dryRun: true)` probe, or `None` when this is not one.
pub fn strip_probe(query: &str) -> Option<String> {
    const PROBE: &str = "rateLimit(dryRun:true){cost nodeCount limit remaining} ";
    query.contains(PROBE).then(|| query.replace(PROBE, ""))
}

/// GitHub's mutation surface, as the journey's own contract tables spell it.
///
/// The journey asks for the whole contract in one document, as one aliased `__type` root
/// field per type, so this answers every alias that document carries rather than one type.
fn introspected(query: &str) -> Value {
    let mut answered = serde_json::Map::new();
    for selected in query.split("__type(name:\"").skip(1) {
        let name = selected
            .split_once('"')
            .map(|(name, _)| name)
            .expect("an introspected type name");
        answered.insert(name.to_owned(), introspected_type_members(name));
    }
    assert!(
        !answered.is_empty(),
        "an introspection document selecting no type: {query}"
    );
    Value::Object(answered)
}

/// One introspected type's members, as the selection for its kind spells them.
fn introspected_type_members(name: &str) -> Value {
    if name == "Mutation" {
        let fields = crate::journey::MUTATION_CONTRACT
            .iter()
            .map(|(field, input, payload)| {
                json!({"name":field,"type":{"name":null,"ofType":{"name":payload}},
                       "args":[{"name":"input","type":{"name":null,"ofType":{"name":input}}}]})
            })
            .collect::<Vec<_>>();
        return json!({ "fields": fields });
    }
    let (_, input, expected) = crate::journey::MUTATION_TYPES
        .iter()
        .find(|(held, _, _)| *held == name)
        .unwrap_or_else(|| panic!("the journey asked about a type it does not name: {name}"));
    let declared = crate::journey::mutation_field_types(name);
    let mut fields = declared
        .iter()
        .map(|(field, signature)| json!({"name":field,"type":introspected_type(signature)}))
        .collect::<Vec<_>>();
    for field in *expected {
        if !declared.iter().any(|(held, _)| held == field) {
            fields.push(json!({"name":field,"type":introspected_type("String")}));
        }
    }
    let selection = if *input { "inputFields" } else { "fields" };
    json!({ selection: fields })
}

/// One type signature, in the nesting GitHub's introspection answers it in.
fn introspected_type(signature: &str) -> Value {
    match signature.strip_suffix('!') {
        Some(inner) => json!({"kind":"NON_NULL","name":null,"ofType":introspected_type(inner)}),
        None => json!({"kind":"SCALAR","name":signature,"ofType":null}),
    }
}

/// One REST request: its method, its path with any query string, and its body if it sent one.
pub fn read_http_request(stream: &mut impl Read) -> (String, String, Option<Value>) {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 4096];
    let header_end = loop {
        let count = stream.read(&mut chunk).expect("a fixture request");
        assert!(count > 0, "the request ended before its headers");
        bytes.extend_from_slice(&chunk[..count]);
        if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break end + 4;
        }
    };
    let headers = String::from_utf8_lossy(&bytes[..header_end]).into_owned();
    assert!(headers.contains("authorization: Bearer test-token"));
    let mut request_line = headers.lines().next().expect("a request line").split(' ');
    let method = request_line.next().expect("a method").to_owned();
    let path = request_line.next().expect("a path").to_owned();
    let length = headers
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length: ")
                .and_then(|value| value.parse::<usize>().ok())
        })
        .unwrap_or_default();
    while bytes.len() - header_end < length {
        let count = stream.read(&mut chunk).expect("a request body");
        assert!(count > 0, "the request ended before its declared body");
        bytes.extend_from_slice(&chunk[..count]);
    }
    let body = (length > 0).then(|| {
        serde_json::from_slice(&bytes[header_end..header_end + length]).expect("body JSON")
    });
    (method, path, body)
}
