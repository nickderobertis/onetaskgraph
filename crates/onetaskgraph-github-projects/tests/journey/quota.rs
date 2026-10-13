//! Bridges the existing request ledger to the one internal session wire contract.
use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex},
};

use onetaskgraph_github_live::quota::{
    Allowance, CallRecord, Charge, EntryPoint, InvocationWriter, MutationBasis, Outcome,
    PRICING_URL, Phase, RequestKind, SessionRecord, WORKSPACE_VERSION, Workload, operation_sha256,
};
use onetaskgraph_github_projects::accounting::{Accounting, Budget, Mode, ResponseOutcome};

struct Invocation {
    ledger: Arc<Accounting>,
    writer: InvocationWriter,
    phase: Phase,
    written: usize,
    modelled_page_refusals: BTreeSet<usize>,
}

static CURRENT: Mutex<Option<Invocation>> = Mutex::new(None);

/// Keeps failure-path requests in the partial record, including an unwinding journey.
pub struct Scope;

impl Drop for Scope {
    fn drop(&mut self) {
        if let Err(error) = flush() {
            use std::io::Write as _;
            let _ = writeln!(
                std::io::stderr(),
                "live-session telemetry unavailable: {error}"
            );
        }
    }
}

/// Start a recording only when the job supplies its current identity and fresh directory.
/// Credentialless ordinary tests have no session recording, rather than a numeric zero.
pub fn start(
    invocation: EntryPoint,
    ledger: Arc<Accounting>,
    workload: Workload,
) -> Result<Scope, String> {
    let directory = match std::env::var_os("GITHUB_LIVE_SESSION_DIR") {
        Some(directory) => std::path::PathBuf::from(directory),
        None => return Ok(Scope),
    };
    let record = SessionRecord {
        schema_version: onetaskgraph_github_live::quota::SCHEMA_VERSION,
        session_id: std::env::var("GITHUB_LIVE_SESSION_ID").map_err(|error| error.to_string())?,
        source_revision: std::env::var("GITHUB_LIVE_SOURCE_REVISION")
            .map_err(|error| error.to_string())?,
        workspace_version: WORKSPACE_VERSION.to_owned(),
        invocation,
        workload,
        complete: false,
        calls: Vec::new(),
    };
    let mut current = CURRENT.lock().map_err(|error| error.to_string())?;
    if current.is_some() {
        return Err("only one live invocation may record in this process".into());
    }
    let writer = InvocationWriter::start(&directory, record)?;
    *current = Some(Invocation {
        ledger,
        writer,
        phase: Phase::Admission,
        written: 0,
        modelled_page_refusals: BTreeSet::new(),
    });
    Ok(Scope)
}

/// Flush the previous phase's already-executed requests before changing its attribution.
pub fn phase(phase: Phase) -> Result<(), String> {
    let mut current = CURRENT.lock().map_err(|error| error.to_string())?;
    if let Some(current) = current.as_mut() {
        current.flush()?;
        current.phase = phase;
    }
    Ok(())
}

/// Complete only after the entry point's assertions and owned cleanup have succeeded.
pub fn complete() -> Result<(), String> {
    let mut current = CURRENT.lock().map_err(|error| error.to_string())?;
    if let Some(current) = current.as_mut() {
        current.flush()?;
        current.writer.complete()?;
    }
    Ok(())
}

fn flush() -> Result<(), String> {
    let mut current = CURRENT.lock().map_err(|error| error.to_string())?;
    if let Some(current) = current.as_mut() {
        current.flush()?;
    }
    Ok(())
}

impl Invocation {
    fn flush(&mut self) -> Result<(), String> {
        let snapshot = self.ledger.snapshot();
        for (index, request) in snapshot.requests().iter().enumerate().skip(self.written) {
            let kind = match (request.budget(), request.mode()) {
                (Budget::Rest, _) => RequestKind::Rest,
                (Budget::Graphql, Mode::Read) => RequestKind::Query,
                (Budget::Graphql, Mode::Write) => RequestKind::Mutation,
            };
            let charge = match kind {
                RequestKind::Rest => Charge::Rest,
                RequestKind::Mutation => Charge::Mutation {
                    points: onetaskgraph_github_live::quota::MODELLED_MUTATION_POINTS,
                    basis: MutationBasis::Modelled {
                        url: PRICING_URL.to_owned(),
                        points_per_request:
                            onetaskgraph_github_live::quota::MODELLED_MUTATION_POINTS,
                    },
                },
                RequestKind::Query => match request.reported_query_cost() {
                    Some(points) => Charge::ReportedQuery { points },
                    None if self.modelled_page_refusals.contains(&index) => {
                        Charge::ModelledPageSizeRefusal {
                            points: 1,
                            url: PRICING_URL.to_owned(),
                        }
                    }
                    None => Charge::Unavailable {
                        reason: "query response did not report its actual primary price".into(),
                    },
                },
            };
            let outcome = match request.response_outcome() {
                ResponseOutcome::Success => Outcome::Success,
                ResponseOutcome::GraphqlError => Outcome::GraphqlError,
                ResponseOutcome::HttpError => Outcome::HttpError,
                ResponseOutcome::TransportError => Outcome::TransportError,
            };
            let limits = request.rate_limit();
            let allowance = match (
                limits.limit(),
                limits.remaining(),
                limits.used_by_the_account(),
                limits.reset(),
            ) {
                (Some(limit), Some(remaining), Some(used), Some(reset_epoch_seconds)) => {
                    Some(Allowance {
                        limit,
                        remaining,
                        used,
                        reset_epoch_seconds,
                    })
                }
                _ => None,
            };
            let document = request.document().unwrap_or_else(|| request.name());
            self.writer.record(CallRecord {
                sequence: 0,
                phase: self.phase,
                operation_sha256: operation_sha256(document),
                attempt: request.attempt(),
                kind,
                outcome,
                charge,
                allowance,
            })?;
            self.written += 1;
        }
        Ok(())
    }
}

/// Retain actual board facts, then refuse a board the registered workload does not admit:
/// more items than it was measured on, fewer than the journey needs, or other fields.
pub fn observe_initial_board(items: Option<u64>, fields: Option<u64>) -> Result<(), String> {
    let mut current = CURRENT.lock().map_err(|error| error.to_string())?;
    if let Some(current) = current.as_mut() {
        let items = items.ok_or("initial board item count was not reported")?;
        let fields = fields.ok_or("initial board field count was not reported")?;
        current.writer.observe_initial_board(items, fields)?;
        let expected = onetaskgraph_github_live::quota::reference_workload();
        let minimum = onetaskgraph_github_live::quota::MINIMUM_INITIAL_ITEMS;
        if !(minimum..=expected.initial_items).contains(&items) || fields != expected.initial_fields
        {
            return Err(format!(
                "initial board workload is {items} items and {fields} fields; registered workload requires {minimum} to {} items and {} fields",
                expected.initial_items, expected.initial_fields
            ));
        }
    }
    Ok(())
}

/// Mark only the last, not-yet-flushed request whose caller confirmed first=101.
/// The response and operation are independently checked here; no variables are persisted.
pub fn model_last_page_size_refusal() -> Result<(), String> {
    use onetaskgraph_github_live::quota::PAGE_SIZE_PROBE_DOCUMENT;
    let mut current = CURRENT.lock().map_err(|error| error.to_string())?;
    if let Some(current) = current.as_mut() {
        let snapshot = current.ledger.snapshot();
        let index = snapshot
            .requests()
            .len()
            .checked_sub(1)
            .ok_or("no page-size request was recorded")?;
        let request = &snapshot.requests()[index];
        if index < current.written
            || current.phase != Phase::Capabilities
            || request.document() != Some(PAGE_SIZE_PROBE_DOCUMENT)
            || request.response_outcome() != ResponseOutcome::GraphqlError
            || request.reported_query_cost().is_some()
        {
            return Err(
                "only the unpriced deliberate page-size GraphQL refusal can be modelled".into(),
            );
        }
        current.modelled_page_refusals.insert(index);
    }
    Ok(())
}
