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
/// writing there), and the janitor's legacy pass, the one thing allowed to delete in it,
/// reaches only the residue that lane left. It is also where `ci.yml` runs, so it is the
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
