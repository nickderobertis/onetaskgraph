//! Internal, additive GitHub live-session telemetry. Contains no work data or credentials.

use serde::{Deserialize, Serialize};

/// Current internal telemetry schema, including actual and requested PNG sizes.
pub const SCHEMA_VERSION: u32 = 2;

/// One invocation's accounting; only a completed matching pair establishes a session bill.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRecord {
    /// Version of this internal representation.
    pub schema_version: u32,
    /// Job-supplied identity shared by both entry points.
    // llmlint: ignore[invalid_states_unrepresentable] The agreed wire format requires String; eligibility validation belongs to the reporter.
    pub session_id: String,
    /// Revision whose journeys ran.
    // llmlint: ignore[invalid_states_unrepresentable] The agreed wire format requires String; eligibility validation belongs to the reporter.
    pub source_revision: String,
    /// Workspace source version, distinct from the authoring CLI version.
    // llmlint: ignore[invalid_states_unrepresentable] The agreed wire format requires String; eligibility validation belongs to the reporter.
    pub workspace_version: String,
    /// Journey producing this record.
    pub invocation: EntryPoint,
    /// Numeric workload facts, shared by both records.
    pub workload: Workload,
    /// True only after successful assertions and owned cleanup.
    // llmlint: ignore[invalid_states_unrepresentable] The agreed wire format requires a bool; the reporter validates completion.
    pub complete: bool,
    /// Every executed request, including failed attempts and cleanup.
    pub calls: Vec<CallRecord>,
}

/// The two credentialed journey entry points.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntryPoint {
    /// Projects-v2 contract journey.
    Main,
    /// Task and document asset-copy journey.
    Assets,
}

/// Numeric fixture facts; an unmeasured orphan population is absent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workload {
    /// Board items before setup.
    pub initial_items: u64,
    /// Board fields before setup.
    pub initial_fields: u64,
    /// Main fixture projects.
    pub fixture_projects: u64,
    /// Main fixture tasks, including its orphan.
    pub fixture_tasks: u64,
    /// Main fixture documents.
    pub fixture_documents: u64,
    /// Actual uploaded PNG sizes, in bytes.
    pub asset_bytes: Vec<u64>,
    /// Requested PNG generator sizes; absent in older telemetry.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub asset_target_bytes: Vec<u64>,
    /// Initial eligible orphan count, only when measured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_orphans: Option<u64>,
}

/// Facts about one executed request, without variables or bodies.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallRecord {
    /// Monotonic request sequence within an invocation.
    pub sequence: u64,
    /// Work being performed.
    pub phase: Phase,
    /// SHA256 of the operation document alone.
    // llmlint: ignore[invalid_states_unrepresentable] The agreed wire format requires String; eligibility validation belongs to the reporter.
    pub operation_sha256: String,
    /// Attempt number for this operation; retries retain their phase.
    pub attempt: u32,
    /// Protocol operation kind.
    pub kind: RequestKind,
    /// Result at the actual protocol boundary.
    pub outcome: Outcome,
    /// Actual reported query price or explicit mutation model.
    pub charge: Charge,
    /// Validated primary allowance headers from this response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowance: Option<Allowance>,
}

/// Protocol kind, keeping REST separate from GraphQL points.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RequestKind {
    /// Executed GraphQL query.
    Query,
    /// Executed GraphQL mutation.
    Mutation,
    /// REST or asset request.
    Rest,
}

/// Protocol result; HTTP 200 with GraphQL errors is not success.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Outcome {
    /// Successful protocol response.
    Success,
    /// GraphQL errors in the response.
    GraphqlError,
    /// Unsuccessful HTTP status.
    HttpError,
    /// No readable protocol response.
    TransportError,
}

/// Design allocation categories, not independently judged budgets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    /// Admission and retained-buffer checks.
    Admission,
    /// Schema contracts.
    Schema,
    /// Document pricing and node-count reconciliation.
    Reconciliation,
    /// Main fixture creation.
    Setup,
    /// Capability assertions.
    Capabilities,
    /// Comment activity assertions.
    CommentActivity,
    /// Fresh change-token assertions.
    ChangeToken,
    /// Asset fixture creation.
    AssetSetup,
    /// Task asset copies and reads.
    AssetTask,
    /// Document asset copies and reads.
    AssetDocument,
    /// Owned cleanup, including failure paths.
    Cleanup,
    /// Stamp-safe recovery.
    OrphanRecovery,
    /// Additional observations.
    Observation,
}

/// Per-request primary price; hypothetical probe prices never appear here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Charge {
    /// Actual non-dryRun query response cost.
    ReportedQuery {
        /// Reported integer points.
        points: u64,
    },
    /// Explicitly priced mutation.
    Mutation {
        /// Primary points attributed to the request.
        points: u64,
        /// Evidence for this price.
        basis: MutationBasis,
    },
    /// REST detail, excluded from GraphQL totals.
    Rest,
    /// Unknown price; prevents a numeric success.
    Unavailable {
        /// Why the response did not establish its price.
        reason: String,
    },
}

/// Mutation pricing evidence; secondary prices are never primary prices.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MutationBasis {
    /// An exact primary tariff stated by GitHub.
    Documented {
        /// Documentation establishing the tariff.
        // llmlint: ignore[invalid_states_unrepresentable] The agreed wire format requires a String URL; the reporter validates the pricing basis.
        url: String,
    },
    /// Approved explicit model when no exact primary tariff is documented.
    Modelled {
        /// Documentation inspected for a primary tariff.
        // llmlint: ignore[invalid_states_unrepresentable] The agreed wire format requires a String URL; the reporter validates the pricing basis.
        url: String,
        /// Model price retained so the mutation count can be repriced.
        points_per_request: u64,
    },
}

/// Same-response primary allowance facts; reset switches invalidate cumulative deltas.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Allowance {
    /// Whole primary allowance.
    pub limit: u64,
    /// Remaining primary allowance.
    pub remaining: u64,
    /// Used primary allowance.
    pub used: u64,
    /// Reset time in Unix seconds.
    pub reset_epoch_seconds: u64,
}

/// Workspace source version, independent of each publishable crate's release version.
pub const WORKSPACE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// GitHub's primary and secondary pricing documentation, kept with the mutation model.
pub const PRICING_URL: &str =
    "https://docs.github.com/en/graphql/overview/rate-limits-and-query-limits-for-the-graphql-api";

/// Identity expected by the current invocation, rather than inferred from saved records.
pub struct CurrentSession<'a> {
    /// Job-supplied current session id.
    session_id: SessionId<'a>,
    /// Source revision being checked.
    source_revision: Revision<'a>,
    /// Workspace source version being checked.
    workspace_version: Version<'a>,
    /// Reference workload for this job.
    workload: &'a Workload,
}

struct SessionId<'a>(&'a str);
struct Revision<'a>(&'a str);
struct Version<'a>(&'a str);

impl<'a> CurrentSession<'a> {
    /// Validate the job identity before reading saved telemetry.
    ///
    /// # Errors
    /// Refuses an empty or non-printable identity, a nonhex revision, or an empty version.
    pub fn new(
        session_id: &'a str,
        source_revision: &'a str,
        workspace_version: &'a str,
        workload: &'a Workload,
    ) -> Result<Self, String> {
        if session_id.is_empty()
            || session_id.len() > 256
            || !session_id.bytes().all(|byte| byte.is_ascii_graphic())
        {
            return Err("invalid current session identity".into());
        }
        if !matches!(source_revision.len(), 40 | 64)
            || !source_revision.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err("invalid current source revision".into());
        }
        if workspace_version.is_empty()
            || !workspace_version
                .bytes()
                .all(|byte| byte.is_ascii_graphic())
        {
            return Err("invalid current workspace version".into());
        }
        Ok(Self {
            session_id: SessionId(session_id),
            source_revision: Revision(source_revision),
            workspace_version: Version(workspace_version),
            workload,
        })
    }
}

/// An attributed bill with corroboration kept separate from its price.
#[derive(Debug, PartialEq, Eq)]
pub struct Bill {
    /// Actual query prices plus explicit mutation prices; excludes REST.
    pub points: u64,
    /// Points reported by executed query responses.
    pub query_points: u64,
    /// Mutation points attributed under the approved explicit model.
    pub modelled_mutation_points: u64,
    /// Mutation requests attributed under the approved explicit model.
    pub modelled_mutations: u64,
    /// Mutation points attributed at an exact documented primary tariff.
    pub documented_mutation_points: u64,
    /// Mutation requests attributed at an exact documented primary tariff.
    pub documented_mutations: u64,
    /// REST requests retained as detail, excluded from GraphQL points.
    pub rest_requests: u64,
    /// Requests attributed at a mutation price, so the model can be repriced.
    pub mutations: u64,
    /// Executed GraphQL requests, including retries and refusals.
    pub graphql_requests: u64,
    /// Same-window account movement; absent if any reset switches internally.
    pub account_movement: Option<u64>,
    /// Movement exceeds attribution by more than the number of GraphQL requests.
    pub movement_flagged: bool,
    /// Points for each phase, independent of the budget threshold.
    pub phases: Vec<(Phase, u64)>,
}

/// Validate a complete current pair and attribute only per-response prices.
///
/// This does not compare a threshold. Account movement is corroboration alone: concurrent
/// callers are never subtracted, and an internal reset invalidates the whole delta.
///
/// # Errors
/// Refuses stale, duplicate, partial or malformed records and unavailable GraphQL prices.
pub fn bill(records: &[SessionRecord], current: &CurrentSession<'_>) -> Result<Bill, String> {
    if records.len() != 2 || current.session_id.0.is_empty() {
        return Err("a current session requires exactly two invocation records".into());
    }
    let main = records
        .iter()
        .find(|record| record.invocation == EntryPoint::Main);
    let assets = records
        .iter()
        .find(|record| record.invocation == EntryPoint::Assets);
    let (Some(main), Some(assets)) = (main, assets) else {
        return Err("a current session requires both Main and Assets".into());
    };
    let mut result = Bill {
        points: 0,
        query_points: 0,
        modelled_mutation_points: 0,
        modelled_mutations: 0,
        documented_mutation_points: 0,
        documented_mutations: 0,
        rest_requests: 0,
        mutations: 0,
        graphql_requests: 0,
        account_movement: None,
        movement_flagged: false,
        phases: Vec::new(),
    };
    let mut allowances = Vec::new();
    for record in [main, assets] {
        validate_record(record, current)?;
        for call in &record.calls {
            let points = match (&call.kind, &call.charge) {
                (RequestKind::Rest, Charge::Rest) => {
                    result.rest_requests += 1;
                    0
                }
                (RequestKind::Query, Charge::ReportedQuery { points }) => {
                    result.graphql_requests += 1;
                    result.query_points = result
                        .query_points
                        .checked_add(*points)
                        .ok_or("query points overflow")?;
                    *points
                }
                (RequestKind::Mutation, Charge::Mutation { points, basis }) => {
                    match basis {
                        MutationBasis::Documented { url } if url == PRICING_URL => {
                            result.documented_mutations += 1;
                            result.documented_mutation_points = result
                                .documented_mutation_points
                                .checked_add(*points)
                                .ok_or("documented mutation points overflow")?;
                        }
                        MutationBasis::Modelled {
                            url,
                            points_per_request,
                        } if url == PRICING_URL && points == points_per_request => {
                            result.modelled_mutations += 1;
                            result.modelled_mutation_points = result
                                .modelled_mutation_points
                                .checked_add(*points)
                                .ok_or("modelled mutation points overflow")?;
                        }
                        _ => return Err("invalid mutation pricing basis".into()),
                    }
                    result.graphql_requests += 1;
                    result.mutations += 1;
                    *points
                }
                (_, Charge::Unavailable { reason }) => {
                    return Err(format!("live-session accounting unavailable: {reason}"));
                }
                _ => return Err("request kind and pricing basis disagree".into()),
            };
            result.points = result
                .points
                .checked_add(points)
                .ok_or("session points overflow")?;
            if let Some((_, total)) = result
                .phases
                .iter_mut()
                .find(|(phase, _)| *phase == call.phase)
            {
                *total = total.checked_add(points).ok_or("phase points overflow")?;
            } else {
                result.phases.push((call.phase, points));
            }
            if let Some(allowance) = call.allowance
                && call.kind != RequestKind::Rest
            {
                allowances.push(allowance);
            }
        }
    }
    if let (Some(first), Some(last)) = (allowances.first(), allowances.last())
        && allowances.iter().all(|allowance| {
            allowance.reset_epoch_seconds == first.reset_epoch_seconds
                && allowance.limit == first.limit
        })
        && allowances
            .windows(2)
            .all(|pair| pair[1].used >= pair[0].used)
    {
        result.account_movement = last.used.checked_sub(first.used);
    }
    result.movement_flagged = result
        .account_movement
        .is_some_and(|movement| movement.saturating_sub(result.points) > result.graphql_requests);
    Ok(result)
}

fn validate_record(record: &SessionRecord, current: &CurrentSession<'_>) -> Result<(), String> {
    if record.schema_version != SCHEMA_VERSION
        || record.session_id != current.session_id.0
        || record.source_revision != current.source_revision.0
        || record.workspace_version != current.workspace_version.0
        || record.workload != *current.workload
        || !record.complete
        || record.calls.is_empty()
    {
        return Err("incomplete, stale or mismatched live-session record".into());
    }
    let required: &[Phase] = match record.invocation {
        EntryPoint::Main => &[
            Phase::Admission,
            Phase::Schema,
            Phase::Reconciliation,
            Phase::Setup,
            Phase::Capabilities,
            Phase::CommentActivity,
            Phase::ChangeToken,
            Phase::Cleanup,
            Phase::OrphanRecovery,
        ],
        EntryPoint::Assets => &[
            Phase::Admission,
            Phase::AssetSetup,
            Phase::AssetTask,
            Phase::AssetDocument,
            Phase::Cleanup,
        ],
    };
    if required
        .iter()
        .any(|phase| !record.calls.iter().any(|call| call.phase == *phase))
    {
        return Err("live-session record is missing a required journey phase".into());
    }
    for (index, call) in record.calls.iter().enumerate() {
        if call.sequence != u64::try_from(index).map_err(|error| error.to_string())? + 1
            || call.attempt == 0
            || call.operation_sha256.len() != 64
            || !call
                .operation_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err("invalid live-session request identity".into());
        }
        if let Some(allowance) = call.allowance
            && allowance.used.checked_add(allowance.remaining) != Some(allowance.limit)
        {
            return Err("inconsistent allowance facts".into());
        }
        match (&call.kind, &call.charge) {
            (RequestKind::Query, Charge::ReportedQuery { .. })
            | (RequestKind::Rest, Charge::Rest) => (),
            (RequestKind::Mutation, Charge::Mutation { points, basis }) => match basis {
                MutationBasis::Documented { url } if url == PRICING_URL => (),
                MutationBasis::Modelled {
                    url,
                    points_per_request,
                } if url == PRICING_URL && points == points_per_request => (),
                _ => return Err("invalid mutation pricing basis".into()),
            },
            (_, Charge::Unavailable { reason }) => {
                return Err(format!("live-session accounting unavailable: {reason}"));
            }
            _ => return Err("request kind and pricing basis disagree".into()),
        }
    }
    Ok(())
}

/// Confirm the current main contribution before the asset entry point spends anything.
/// This reads the same wire and eligibility checks as the whole-session reporter.
///
/// # Errors
/// Refuses a missing, stale, partial or unpriced main contribution.
pub fn require_current_main(
    directory: &std::path::Path,
    current: &CurrentSession<'_>,
) -> Result<(), String> {
    let bytes = std::fs::read(directory.join("main.json")).map_err(|error| error.to_string())?;
    let record: SessionRecord =
        serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    if record.invocation != EntryPoint::Main {
        return Err("main telemetry belongs to another invocation".into());
    }
    validate_record(&record, current)
}

/// File-backed invocation recording, reserved in a fresh job directory before requests run.
/// A crash leaves a partial record; the reporter cannot treat it as numeric success.
pub struct InvocationWriter {
    file: std::fs::File,
    record: SessionRecord,
}

impl InvocationWriter {
    /// Reserve this invocation's file without replacing an old or duplicate recording.
    ///
    /// # Errors
    /// Refuses an existing file, invalid identity, or an unwritable directory.
    pub fn start(directory: &std::path::Path, mut record: SessionRecord) -> Result<Self, String> {
        CurrentSession::new(
            &record.session_id,
            &record.source_revision,
            &record.workspace_version,
            &record.workload,
        )?;
        if record.schema_version != SCHEMA_VERSION || !record.calls.is_empty() {
            return Err("a new invocation must start with the current schema and no calls".into());
        }
        record.complete = false;
        let filename = match record.invocation {
            EntryPoint::Main => "main.json",
            EntryPoint::Assets => "assets.json",
        };
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join(filename))
            .map_err(|error| error.to_string())?;
        let mut writer = Self { file, record };
        writer.persist()?;
        Ok(writer)
    }

    /// Retain one executed call, assigning its sequence in this invocation.
    ///
    /// # Errors
    /// Refuses a write after completion or a failed telemetry write.
    pub fn record(&mut self, mut call: CallRecord) -> Result<(), String> {
        if self.record.complete {
            return Err("cannot append to a completed invocation".into());
        }
        call.sequence =
            u64::try_from(self.record.calls.len()).map_err(|error| error.to_string())? + 1;
        self.record.calls.push(call);
        self.persist()
    }

    /// Mark completion only after every assertion and owned cleanup succeeded.
    ///
    /// # Errors
    /// Refuses completion twice or a failed telemetry write.
    pub fn complete(&mut self) -> Result<(), String> {
        if self.record.complete {
            return Err("invocation already completed".into());
        }
        self.record.complete = true;
        self.persist()
    }

    /// Retain the initial board facts observed before creating the invocation's fixtures.
    ///
    /// # Errors
    /// Refuses an observation after completion or a failed telemetry write.
    pub fn observe_initial_board(&mut self, items: u64, fields: u64) -> Result<(), String> {
        if self.record.complete {
            return Err("cannot change a completed invocation's workload".into());
        }
        self.record.workload.initial_items = items;
        self.record.workload.initial_fields = fields;
        self.persist()
    }

    fn persist(&mut self) -> Result<(), String> {
        use std::io::{Seek, Write};
        let bytes = serde_json::to_vec(&self.record).map_err(|error| error.to_string())?;
        self.file.rewind().map_err(|error| error.to_string())?;
        self.file.set_len(0).map_err(|error| error.to_string())?;
        self.file
            .write_all(&bytes)
            .map_err(|error| error.to_string())?;
        self.file.sync_all().map_err(|error| error.to_string())
    }
}

/// Read the current directory's records through the shared serde contract and validate them.
/// This performs no network requests and compares no budget threshold.
///
/// # Errors
/// Refuses missing, duplicate, corrupt, stale, partial or unavailable telemetry.
pub fn read_bill(
    directory: &std::path::Path,
    current: &CurrentSession<'_>,
) -> Result<Bill, String> {
    let mut records = Vec::new();
    for entry in std::fs::read_dir(directory).map_err(|error| error.to_string())? {
        let path = entry.map_err(|error| error.to_string())?.path();
        if path
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            let bytes = std::fs::read(&path).map_err(|error| error.to_string())?;
            records.push(serde_json::from_slice(&bytes).map_err(|error| error.to_string())?);
        }
    }
    bill(&records, current)
}

/// Digest of an operation document alone, without retaining it or its variable values.
#[must_use]
pub fn operation_sha256(document: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(document.as_bytes()))
}

/// The nominated workload required for the whole-lane budget, with unknown initial orphans.
#[must_use]
pub fn reference_workload() -> Workload {
    Workload {
        initial_items: 90,
        initial_fields: 14,
        fixture_projects: 2,
        fixture_tasks: 3,
        fixture_documents: 1,
        asset_bytes: vec![74_666, 479_959, 74_666, 479_959],
        asset_target_bytes: vec![75_000, 480_000, 75_000, 480_000],
        initial_orphans: None,
    }
}

/// Read only the selection already computed by the repository's existing policy.
/// No value means a direct test invocation, which remains selected.
///
/// # Errors
/// Refuses malformed or non-Unicode propagated selection rather than silently skipping.
pub fn live_selected() -> Result<bool, String> {
    match std::env::var("GITHUB_LIVE_SELECTION") {
        Ok(value) if value == "run" => Ok(true),
        Ok(value) if value == "not-selected" => Ok(false),
        Err(std::env::VarError::NotPresent) => Ok(true),
        _ => Err("GITHUB_LIVE_SELECTION must be run or not-selected".into()),
    }
}
