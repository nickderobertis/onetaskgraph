//! The GitHub Projects live lane's own policy, which its tests and the scheduled janitor
//! both read.
//!
//! What a lane run writes is `<prefix><stamp>`, and the stamp — both forms, and what may
//! remove one — is the cross-lane contract in `onetaskgraph_live::artifact`. What is here is
//! GitHub's alone: the two prefixes the GitHub Projects lane puts in front of a stamp, how a
//! title or a label is recognised as one of them, and the repositories the lane writes to
//! and refuses. Each is declared once, because the lane's tests are not the only reader: the
//! janitor in `crates/onetaskgraph-live-janitor` recognises exactly these names in exactly
//! these repositories, and a second spelling would let the two disagree about what is
//! residue on somebody's real board. Linear's lane names its artifacts itself and reads
//! nothing here.

#![deny(missing_docs)]

use onetaskgraph_live::artifact::{CiRun, CiRunField, WIDEST_CI_STAMP, WIDEST_STAMP};

/// The prefix of every issue — and so of every board item — the GitHub Projects lane writes.
///
/// The rest of a title is a stamp, in either form, and a *document* the lane writes carries
/// that source's own design prefix in front of the whole ([`titled_stamp`] takes it off).
pub const ARTIFACT_PREFIX: &str = "onetaskgraph live cleanup ";

/// The prefix of the one repository label each GitHub Projects lane run creates.
///
/// **Short on purpose**, where the issue prefix is a readable sentence: GitHub holds a label
/// name to fifty characters, and this plus the widest stamp of either form has to fit.
pub const LABEL_PREFIX: &str = "otg-live-";

/// GitHub's longest label name, in characters.
pub const LABEL_LIMIT: usize = 50;

const _: () = assert!(LABEL_PREFIX.len() + WIDEST_STAMP <= LABEL_LIMIT);
const _: () = assert!(LABEL_PREFIX.len() + WIDEST_CI_STAMP <= LABEL_LIMIT);

/// The repository the GitHub Projects lane creates its issues in: one that exists for them.
///
/// Public, with issues enabled, and nothing else in it — so test residue, and the delete
/// rights a test credential needs, stay out of the tracker real work is filed in.
pub const SCRATCH_REPOSITORY: &str = "nickderobertis/onetaskgraph-live-scratch";

/// The core repository, which the lane wrote its issues into until it had a scratch one.
///
/// Named so it can be **refused**: the lane fails before any request when it is nominated
/// (a machine still configured with the old value says so on its next run rather than
/// writing there). It is also where `ci.yml` runs, so it is the
/// repository whose Actions runs a CI stamp's run id is read back from. A machine the lane
/// refuses fixes `GH_PROJECTS_REPOSITORY` to [`SCRATCH_REPOSITORY`] in its onetaskgraph
/// `secrets.env`, or in the environment it pushes from.
pub const CORE_REPOSITORY: &str = "nickderobertis/onetaskgraph";

/// The login owning the board both credentialed `ci.yml` steps nominate.
pub const BOARD_OWNER: &str = "nickderobertis";

/// That owner's board the lane writes to and the janitor cleans: board #1, never another.
pub const BOARD_NUMBER: u32 = 1;

/// The variable GitHub Actions sets to `true` on every run it hosts.
pub const GITHUB_ACTIONS_VARIABLE: &str = "GITHUB_ACTIONS";

/// The variable naming the GitHub Actions run, which a re-run keeps.
pub const RUN_ID_VARIABLE: &str = "GITHUB_RUN_ID";

/// The variable naming which attempt of that run this is, from 1.
pub const RUN_ATTEMPT_VARIABLE: &str = "GITHUB_RUN_ATTEMPT";

/// Which form of stamp the lane writes here, read from GitHub Actions' own variables.
///
/// `Ok(None)` — a machine stamp — unless `github_actions` is exactly `true`. When it is,
/// both `run_id` and `attempt` have to spell a [`CiRun`], or this refuses naming the
/// variable: **a CI artifact that does not name its run is residue nothing may ever
/// remove**, so a lane that cannot name its run must write nothing at all.
///
/// # Errors
///
/// When `github_actions` is `true` and either value is missing or outside the grammar.
pub fn ci_run(
    github_actions: Option<&str>,
    run_id: Option<&str>,
    attempt: Option<&str>,
) -> Result<Option<CiRun>, String> {
    if github_actions != Some("true") {
        return Ok(None);
    }
    let refuse = |field: CiRunField, what: String| {
        let variable = match field {
            CiRunField::RunId => RUN_ID_VARIABLE,
            CiRunField::Attempt => RUN_ATTEMPT_VARIABLE,
        };
        format!(
            "{GITHUB_ACTIONS_VARIABLE} is true, so this lane stamps every artifact with the \
             GitHub Actions run that wrote it, but {variable} {what}. It has to be a decimal \
             number of 1 to {} digits, at least 1, with no sign and no leading zero. A CI \
             artifact that does not name its run is residue nothing may ever remove, so this \
             lane writes nothing until {variable} is set the way GitHub Actions sets it",
            field.widest()
        )
    };
    let run_id = run_id.ok_or_else(|| refuse(CiRunField::RunId, "is not set".to_owned()))?;
    let attempt = attempt.ok_or_else(|| refuse(CiRunField::Attempt, "is not set".to_owned()))?;
    CiRun::read(run_id, attempt).map(Some).map_err(|field| {
        let spelled = match field {
            CiRunField::RunId => run_id,
            CiRunField::Attempt => attempt,
        };
        refuse(field, format!("is {spelled:?}"))
    })
}

/// The stamp half of an issue title the GitHub Projects lane wrote, in either form, unread.
///
/// `design_prefix` is the source's own spelling of a document — the lane's documents carry
/// it in front of [`ARTIFACT_PREFIX`] — and it is a parameter so this crate need not depend
/// on the plugin whose tests depend on it: the lane and the janitor both pass the plugin
/// crate's `DESIGN_TITLE_PREFIX`. At most one is taken off, and [`ARTIFACT_PREFIX`] must
/// follow it directly; any other title is not the lane's and gets `None`.
#[must_use]
pub fn titled_stamp<'title>(title: &'title str, design_prefix: &str) -> Option<&'title str> {
    title
        .strip_prefix(design_prefix)
        .unwrap_or(title)
        .strip_prefix(ARTIFACT_PREFIX)
}

/// The stamp half of a repository label the GitHub Projects lane created, unread.
#[must_use]
pub fn labelled_stamp(name: &str) -> Option<&str> {
    name.strip_prefix(LABEL_PREFIX)
}

/// The stamp half of a board field the GitHub Projects lane created, unread.
///
/// The lane names its scratch single-select field the way it names its label —
/// [`LABEL_PREFIX`] and a stamp — so one grammar recognises both. A board's own fields
/// (`Status`, `onetaskgraph.origin`, the built-in ones) never begin with it.
#[must_use]
pub fn fielded_stamp(name: &str) -> Option<&str> {
    name.strip_prefix(LABEL_PREFIX)
}

/// The path GitHub answers a CI stamp's run on: one workflow run of [`CORE_REPOSITORY`].
///
/// `GET /repos/{owner}/{repo}/actions/runs/{run_id}` reports the run's latest attempt, so a
/// re-run in progress reads as in progress for every attempt's artifacts.
#[must_use]
pub fn run_path(run_id: u64) -> String {
    format!("/repos/{CORE_REPOSITORY}/actions/runs/{run_id}")
}

// llmlint: ignore-block[contracts_have_one_source_or_a_drift_gate] GitHub publishes no machine-readable vocabulary of a run's `status` (the OpenAPI schema types it as a bare string), so there is no artifact to derive this from. The six words are pinned with their provenance below, the janitor's offline journeys derive theirs from this one declaration, and the drift gate is the janitor itself: a word GitHub adds is undecodable, fails the scheduled run red naming that word, and deletes nothing.
/// A workflow run's `status`, as GitHub documents it. Anything else is undecodable, and a
/// reader names it, which is how a word GitHub adds is noticed.
///
/// The six statuses of the `status` values GitHub lists for "List workflow runs for a
/// workflow" (<https://docs.github.com/en/rest/actions/workflow-runs?apiVersion=2022-11-28>,
/// read 2026-10-07); the rest of that list — `action_required`, `cancelled`, `failure`,
/// `neutral`, `skipped`, `stale`, `success`, `timed_out` — are conclusions, which a run's
/// `status` never holds. A cancelled run therefore reads `completed`, as a succeeded or a
/// failed one does. This is the one declaration: the janitor, the live lane's board
/// pre-flight and the janitor's offline journeys all read it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStatus {
    /// The run is over, whatever its conclusion: nothing of it is still writing.
    Completed,
    /// Waiting for a runner.
    Queued,
    /// Running.
    InProgress,
    /// Waiting on a deployment protection rule.
    Waiting,
    /// Requested and not yet queued.
    Requested,
    /// Pending on a concurrency group.
    Pending,
}
impl RunStatus {
    /// Every status a run that has not finished can have.
    pub const ACTIVE: [Self; 5] = [
        Self::Queued,
        Self::InProgress,
        Self::Waiting,
        Self::Requested,
        Self::Pending,
    ];

    /// GitHub's spelling, in a run's `status`.
    #[must_use]
    pub fn spelling(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Queued => "queued",
            Self::InProgress => "in_progress",
            Self::Waiting => "waiting",
            Self::Requested => "requested",
            Self::Pending => "pending",
        }
    }

    /// The status a run read answered with, or `None` when it carries none this declares.
    #[must_use]
    pub fn read(value: &serde_json::Value) -> Option<Self> {
        let spelled = value.get("status")?.as_str()?;
        std::iter::once(Self::Completed)
            .chain(Self::ACTIVE)
            .find(|status| status.spelling() == spelled)
    }
}
// llmlint: ignore-end[contracts_have_one_source_or_a_drift_gate]

/// Deletes one board field, answering with the id of the field deleted.
///
/// Sent by the janitor, by the live lane's board pre-flight and by the lane's own scratch
/// field removal; validated against the plugin's pinned schema by the janitor's
/// `tests/schema.rs`. The two fragments are the two field kinds the lane ever creates.
pub const DELETE_FIELD_DOCUMENT: &str = "mutation($input:DeleteProjectV2FieldInput!){deleteProjectV2Field(input:$input){projectV2Field{... on ProjectV2Field{id} ... on ProjectV2SingleSelectField{id}}}}";

/// One page of a board's items, with what the live lane's pre-flight needs to tell a CI run's
/// leftover issue from everything else: the item's id, and for an issue its id, title and
/// repository.
///
/// Declared here, beside [`BOARD_WORKLOAD_DOCUMENT`], so the janitor's `tests/schema.rs` holds
/// it to the pinned schema.
pub const RESIDUE_ITEMS_DOCUMENT: &str = "query($id:ID!,$after:String){node(id:$id){... on ProjectV2{items(first:100,after:$after){nodes{id content{__typename ... on Issue{id title repository{nameWithOwner}}}} pageInfo{hasNextPage endCursor}}}}}";

/// The live lane's measurement of the nominated board before it writes anything: its id,
/// how many items and fields it holds, and every field's id and name, so a field a finished
/// run left behind can be told from the board's own before the count is judged.
///
/// Declared here rather than in the lane so the janitor's `tests/schema.rs` holds it to the
/// pinned schema beside the janitor's own documents. GitHub caps a board at fewer than a
/// hundred fields, so one page holds them all and `hasNextPage` is read to prove it.
pub const BOARD_WORKLOAD_DOCUMENT: &str = "query LiveBoardWorkload($owner:String!,$number:Int!){repositoryOwner(login:$owner){... on ProjectV2Owner{projectV2(number:$number){id items(first:1){totalCount} fields(first:100){totalCount nodes{... on ProjectV2Field{id name} ... on ProjectV2SingleSelectField{id name}} pageInfo{hasNextPage}}}}}}";

/// Internal session accounting shared by the two GitHub journeys and their reporter.
pub mod quota;
