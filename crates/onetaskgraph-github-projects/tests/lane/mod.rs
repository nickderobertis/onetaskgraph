//! The parts of the live lane that reach nothing.
//!
//! Two test targets share them, and both are ordinary tests in this crate's ordinary
//! `test` target: change this plugin and they run, change anything else and affected
//! selection does not select them. `tests/live.rs` is the journey against GitHub itself;
//! `tests/lane_shape.rs` asserts the decisions made here — which board and repository the
//! lane may write to, which artifacts a run recognises as its own, and that cleanup runs
//! whether the journey passed or failed — and reaches no network at all, which is why it
//! can assert them without a credential.

use std::future::Future;

use onetaskgraph_github_projects::DESIGN_TITLE_PREFIX;
use onetaskgraph_live::{missing, required};
use onetaskgraph_plugin_api::SecretResolver;
use secrecy::SecretString;
use serde_json::{Value, json};

pub struct LiveSecret(pub SecretString);

impl SecretResolver for LiveSecret {
    fn get(&self, variable: &str) -> Option<SecretString> {
        (variable == "GH_PROJECTS_TOKEN").then(|| self.0.clone())
    }
}

/// The write configuration this lane builds for the board it was pointed at.
///
/// Writes go by status *category*, and the board's own first Status option is the only
/// column this lane knows exists — so `todo` is pointed at it and every other
/// column-bearing category is disabled. Exactly one category writes a column, so however
/// this board spells that option, no two categories can send it the same one and the
/// source's own validation has nothing to refuse. Pointing `unknown` at that column
/// instead is the collision itself: `unknown` and `draft` map to no column by design,
/// precisely so neither can collide with a category that has one.
pub fn live_write_config(
    owner: &str,
    project_number: u32,
    repository: &str,
    status_option: &str,
) -> Value {
    json!({"owner":owner,"project_number":project_number,"repository":repository,
           "status_mapping":{"todo":status_option,"backlog":null,"in-progress":null}})
}

/// The prefix of every board item this lane writes.
///
/// The rest of a title is `<process id>-<microsecond timestamp>`, which makes one run's artifact
/// unique and makes any run's artifact recognisable to the next run.
pub const ARTIFACT_PREFIX: &str = "onetaskgraph live cleanup ";

pub fn artifact_title(process_id: u32, stamp_micros: i64) -> String {
    format!("{ARTIFACT_PREFIX}{process_id}-{stamp_micros}")
}

/// The artifact title inside one board issue's own title.
///
/// A *document* this lane writes carries [`DESIGN_TITLE_PREFIX`] in front of the title it
/// was given, because that is how this source spells a document and the source puts it
/// there rather than the caller. Cleanup reads the board's raw titles, so recognition has
/// to take that prefix off first: without this, a document a run created would be residue
/// no sweep could ever name, on somebody's real board.
fn artifact_part(title: &str) -> &str {
    title.strip_prefix(DESIGN_TITLE_PREFIX).unwrap_or(title)
}

/// Whether a board item is one this lane wrote, in this run or in an earlier one.
pub fn is_artifact_title(title: &str) -> bool {
    let Some(suffix) = artifact_part(title).strip_prefix(ARTIFACT_PREFIX) else {
        return false;
    };
    let Some((process_id, stamp_micros)) = suffix.split_once('-') else {
        return false;
    };
    [process_id, stamp_micros]
        .iter()
        .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

/// Whether a board item is one *this* run wrote.
///
/// Every artifact of one run carries this process's id, so a run names its own for
/// cleanup without touching one an interrupted earlier run left for [`is_artifact_title`]
/// to sweep.
pub fn is_run_artifact_title(process_id: u32, title: &str) -> bool {
    is_artifact_title(title)
        && artifact_part(title).starts_with(&format!("{ARTIFACT_PREFIX}{process_id}-"))
}

/// The prefix of the one repository label this lane creates.
///
/// A label this lane created is residue exactly as an issue is, so it is named the way
/// board items are — this process's id and a timestamp — and swept the same way before a
/// run starts. The grammar [`is_artifact_label`] accepts is letters, digits and hyphens
/// only, which is what lets the cleanup below name one in a URL path unescaped.
pub const LABEL_PREFIX: &str = "onetaskgraph-live-";

pub fn artifact_label(process_id: u32, stamp_micros: i64) -> String {
    format!("{LABEL_PREFIX}{process_id}-{stamp_micros}")
}

/// Whether a repository label is one this lane created, in this run or in an earlier one.
pub fn is_artifact_label(name: &str) -> bool {
    let Some(suffix) = name.strip_prefix(LABEL_PREFIX) else {
        return false;
    };
    let Some((process_id, stamp_micros)) = suffix.split_once('-') else {
        return false;
    };
    [process_id, stamp_micros]
        .iter()
        .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

pub async fn run_then_cleanup<J, JF, C, CF>(journey: J, cleanup: C) -> Result<(), String>
where
    J: FnOnce() -> JF,
    JF: Future<Output = Result<(), String>>,
    C: FnOnce() -> CF,
    CF: Future<Output = Result<(), String>>,
{
    let journey_result = journey().await;
    let cleanup_result = cleanup().await;
    match (journey_result, cleanup_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(journey), Ok(())) => Err(journey),
        (Ok(()), Err(cleanup)) => Err(format!("live cleanup failed: {cleanup}")),
        (Err(journey), Err(cleanup)) => Err(format!(
            "{journey}; additionally, live cleanup failed: {cleanup}"
        )),
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum LiveLane {
    Run {
        token: String,
        owner: String,
        project_number: u32,
        repository: String,
    },
    Skip(String),
}

/// The session this lane opens against GitHub, by the name its seat and its refusals use.
pub const SESSION_NAME: &str = "GitHub Projects";

/// Decides whether this lane may run, and against which board.
///
/// The board comes from `GH_PROJECTS_OWNER` and `GH_PROJECTS_NUMBER`, and the repository this
/// source creates its issues in comes from `GH_PROJECTS_REPOSITORY`, or the lane does not run.
/// Nothing here asks GitHub which project was updated most recently, for the viewer or for any
/// organization: a credentialed lane that writes and deletes must reach only a board somebody
/// nominated by name, and that requirement — rather than any cleanup — is what keeps it off a
/// board nobody nominated. `ONETASKGRAPH_LIVE_REQUIRED=1` turns a skip into
/// a failure, the same pairing an absent credential already has. `Err` is a misconfiguration,
/// which fails whether or not the lane is required.
///
/// The skip-or-fail pairing itself is [`onetaskgraph_live::missing`], so this lane and
/// Linear's answer an absent input the same way rather than in two dialects. What this
/// function decides is only *which* names are needed; whether a session may then start is
/// [`onetaskgraph_live::Session::open`]'s, and the token below is unusable until it has.
pub fn live_lane(
    token: Option<&str>,
    owner: Option<&str>,
    project_number: Option<&str>,
    repository: Option<&str>,
    live_required: Option<&str>,
) -> Result<LiveLane, String> {
    let live_required = required(live_required)?;
    let skip = |reason: &str| -> Result<LiveLane, String> {
        Ok(LiveLane::Skip(missing(
            live_required,
            SESSION_NAME,
            reason,
        )?))
    };
    // llmlint: ignore[live_tier_compiles_and_requires_credential] An absent credential
    // skips rather than fails only where no credential was expected — a contributor with no
    // keys, and a pull request from a fork, which the host gives no secrets. The run where
    // one *is* expected sets `ONETASKGRAPH_LIVE_REQUIRED=1`, which turns every skip below
    // into the failure this rule asks for, and .github/workflows/ci.yml sets it on the one
    // lane the credentials reach.
    let Some(token) = token else {
        return skip("GH_PROJECTS_TOKEN is not set");
    };
    if token.trim().is_empty() {
        return skip("GH_PROJECTS_TOKEN is empty");
    }
    let owner = owner.map(str::trim).filter(|owner| !owner.is_empty());
    let project_number = project_number
        .map(str::trim)
        .filter(|number| !number.is_empty());
    let (owner, project_number) = match (owner, project_number) {
        (Some(owner), Some(project_number)) => (owner, project_number),
        (None, None) => {
            return skip(
                "GH_PROJECTS_OWNER and GH_PROJECTS_NUMBER are not set, and this lane writes only \
                 to the board those two name rather than discovering one",
            );
        }
        (owner, _) => {
            return Err(format!(
                "GH_PROJECTS_OWNER and GH_PROJECTS_NUMBER name one board together: {} is missing",
                if owner.is_some() {
                    "GH_PROJECTS_NUMBER"
                } else {
                    "GH_PROJECTS_OWNER"
                }
            ));
        }
    };
    let number = project_number
        .parse::<u32>()
        .ok()
        .filter(|number| *number > 0 && *number <= i32::MAX as u32)
        .ok_or_else(|| {
            format!("GH_PROJECTS_NUMBER must be a positive GraphQL Int, not {project_number:?}")
        })?;
    let Some(repository) = repository
        .map(str::trim)
        .filter(|repository| !repository.is_empty())
    else {
        return skip(
            "GH_PROJECTS_REPOSITORY is not set, and this lane creates its artifact as an issue \
             in the repository that name gives rather than discovering one",
        );
    };
    if repository
        .split_once('/')
        .is_none_or(|(owner, name)| owner.is_empty() || name.is_empty() || name.contains('/'))
    {
        return Err(format!(
            "GH_PROJECTS_REPOSITORY must be spelled owner/name, not {repository:?}"
        ));
    }
    Ok(LiveLane::Run {
        token: token.to_owned(),
        owner: owner.to_owned(),
        project_number: number,
        repository: repository.to_owned(),
    })
}
