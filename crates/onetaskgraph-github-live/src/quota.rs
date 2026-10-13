//! Internal, additive GitHub live-session telemetry. Contains no work data or credentials.

use serde::{Deserialize, Serialize};

/// One invocation's accounting; only a completed matching pair establishes a session bill.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRecord {
    /// Version of this internal representation.
    pub schema_version: u32,
    /// Job-supplied identity shared by both entry points.
    // llmlint: ignore[invalid_states_unrepresentable] The planner-owned additive telemetry agreement requires a String here. Pair validation belongs to the reporter before any numeric success; changing this field to a newtype would amend that frozen agreement.
    pub session_id: String,
    /// Revision whose journeys ran.
    // llmlint: ignore[invalid_states_unrepresentable] The planner-owned wire agreement requires String and exact revision matching across the pair. This representation records identity; eligibility validation belongs to the reporter.
    pub source_revision: String,
    /// Workspace source version, distinct from the authoring CLI version.
    // llmlint: ignore[invalid_states_unrepresentable] The planner-owned wire agreement requires String and exact workspace-version matching across the pair. A version newtype would change the agreed Rust representation.
    pub workspace_version: String,
    /// Journey producing this record.
    pub invocation: EntryPoint,
    /// Numeric workload facts, shared by both records.
    pub workload: Workload,
    /// True only after successful assertions and owned cleanup.
    // llmlint: ignore[invalid_states_unrepresentable] The planner-owned wire agreement explicitly requires a completion bool. Writers set it only after assertions and cleanup; the reporter refuses false records. A completion enum would amend the agreement.
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
    /// Requested PNG sizes, in bytes.
    pub asset_bytes: Vec<u64>,
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
    // llmlint: ignore[invalid_states_unrepresentable] The planner-owned wire agreement requires String for the document digest. Writers compute SHA256 without retaining variables or bodies; reader eligibility validates records before reporting.
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
        // llmlint: ignore[invalid_states_unrepresentable] The planner-owned pricing-basis agreement requires a String URL. Writers supply the fixed public GitHub rate-limit documentation URL, never request URLs or credentials; a newtype would amend the agreement.
        url: String,
    },
    /// Approved explicit model when no exact primary tariff is documented.
    Modelled {
        /// Documentation inspected for a primary tariff.
        // llmlint: ignore[invalid_states_unrepresentable] The planner-owned pricing-basis agreement requires a String URL. Writers supply the fixed public GitHub rate-limit documentation URL, never request URLs or credentials; a newtype would amend the agreement.
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

/// GitHub's primary and secondary pricing documentation, kept with the mutation model.
pub const PRICING_URL: &str =
    "https://docs.github.com/en/graphql/overview/rate-limits-and-query-limits-for-the-graphql-api";

/// Identity expected by the current invocation, rather than inferred from saved records.
pub struct CurrentSession<'a> {
    /// Job-supplied current session id.
    pub session_id: &'a str,
    /// Source revision being checked.
    pub source_revision: &'a str,
    /// Workspace source version being checked.
    pub workspace_version: &'a str,
    /// Reference workload for this job.
    pub workload: &'a Workload,
}

/// An attributed bill with corroboration kept separate from its price.
#[derive(Debug, PartialEq, Eq)]
pub struct Bill {
    /// Actual query prices plus explicit mutation prices; excludes REST.
    pub points: u64,
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
    if records.len() != 2 || current.session_id.is_empty() {
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
        mutations: 0,
        graphql_requests: 0,
        account_movement: None,
        movement_flagged: false,
        phases: Vec::new(),
    };
    let mut allowances = Vec::new();
    for record in [main, assets] {
        if record.schema_version != 1
            || record.session_id != current.session_id
            || record.source_revision != current.source_revision
            || record.workspace_version != current.workspace_version
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
            let points = match (&call.kind, &call.charge) {
                (RequestKind::Rest, Charge::Rest) => 0,
                (RequestKind::Query, Charge::ReportedQuery { points }) => {
                    result.graphql_requests += 1;
                    *points
                }
                (RequestKind::Mutation, Charge::Mutation { points, basis }) => {
                    match basis {
                        MutationBasis::Documented { url } if url == PRICING_URL => {}
                        MutationBasis::Modelled {
                            url,
                            points_per_request,
                        } if url == PRICING_URL && points == points_per_request => {}
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
            if let Some(allowance) = call.allowance {
                if allowance.used.checked_add(allowance.remaining) != Some(allowance.limit) {
                    return Err("inconsistent allowance facts".into());
                }
                if call.kind != RequestKind::Rest {
                    allowances.push(allowance);
                }
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
