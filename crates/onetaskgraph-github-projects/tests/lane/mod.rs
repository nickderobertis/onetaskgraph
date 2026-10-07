//! The parts of the live lane that reach nothing.
//!
//! Two test targets share them, and both are ordinary tests in this crate's ordinary
//! `test` target: change this plugin and they run, change anything else and affected
//! selection does not select them. `tests/live.rs` is the journey against GitHub itself;
//! `tests/lane_shape.rs` asserts the decisions made here — which board and repository the
//! lane may write to, which artifacts a run recognises as its own, and that cleanup runs
//! whether the journey passed or failed — and reaches no network at all, which is why it
//! can assert them without a credential.

use std::collections::BTreeSet;
use std::future::Future;
use std::sync::Mutex;

use onetaskgraph_github_projects::DESIGN_TITLE_PREFIX;
use onetaskgraph_github_projects::accounting::{Outcome, StatusCode};
use onetaskgraph_live::artifact::{
    CORE_REPOSITORY, CiRun, CiStamp, GITHUB_ACTIONS_VARIABLE, RUN_ATTEMPT_VARIABLE,
    RUN_ID_VARIABLE, Run, SCRATCH_REPOSITORY, Stamp, Sweep, labelled_stamp, titled_stamp,
};
use onetaskgraph_live::{Credential, REQUIRED_VARIABLE, missing, required};
use onetaskgraph_plugin_api::SecretResolver;
use secrecy::SecretString;
use serde_json::{Value, json};

pub struct LiveSecret(pub SecretString);

impl SecretResolver for LiveSecret {
    fn get(&self, variable: &str) -> Option<SecretString> {
        (variable == "GH_PROJECTS_TOKEN").then(|| self.0.clone())
    }
}

/// What this lane records for one REST response, once it knows whether it could read it.
///
/// [`Outcome::of_response`] rules on the status and the rate-limit headers and leaves the
/// rest to the caller, because a success whose body the caller cannot use is a refusal only
/// the caller can see. This is that narrowing for a REST call: a `2xx` carrying something
/// this lane could not decode did not answer what was asked for, however cleanly it
/// arrived, and recording it as `Answered` would make the session report say a call
/// succeeded that produced nothing.
pub fn rest_outcome(status: StatusCode, exhausted: bool, body: &str, decoded: bool) -> Outcome {
    match Outcome::of_response(status, exhausted, body) {
        Outcome::Answered if !decoded => Outcome::Refused,
        settled => settled,
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
           "status_mapping":{"todo":status_option,"backlog":null,"queued":null,
                             "in-progress":null}})
}

/// The prefix of every issue this lane writes, and of the one label each run creates.
///
/// Declared once, in `onetaskgraph_live::artifact`, because the scheduled janitor
/// (`crates/onetaskgraph-live-janitor`) recognises exactly these names and non-test code
/// cannot import anything from here. The rest of a title is a stamp in one of two forms —
/// see [`Writer`] — and the label prefix is short because GitHub holds a label name to fifty
/// characters.
pub use onetaskgraph_live::artifact::{ARTIFACT_PREFIX, LABEL_PREFIX};

/// Which form of stamp this lane process writes, and so how its artifacts name it.
///
/// **A machine stamp outside GitHub Actions, unchanged**: the machine and process that
/// wrote it, which the lock sweep in `onetaskgraph_live::artifact` decides on. **A CI stamp
/// inside it**, `ci-<run id>-<attempt>-<micros>`: a hosted runner is a fresh machine every
/// time, so a lock there is evidence nobody can read afterwards, and what can answer for the
/// artifact later is the run it names — the janitor removes a CI-stamped artifact only once
/// GitHub reports that run `completed`. [`admit`] decides which, from `GITHUB_ACTIONS`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Writer {
    /// A run outside GitHub Actions, named by its machine and process.
    Machine(Run),
    /// A run inside GitHub Actions, named by its run id and attempt.
    Ci(CiRun),
}

impl From<Run> for Writer {
    fn from(run: Run) -> Self {
        Self::Machine(run)
    }
}

impl Writer {
    /// This process, outside GitHub Actions: the machine stamp it has always written.
    pub fn this_machine() -> Self {
        Self::Machine(Run::current())
    }

    /// The stamp this writer puts on an artifact it writes at `micros`.
    ///
    /// A CI stamp's time is at least 1 and at most sixteen digits, which every clock reading
    /// after 1970 and before the year 2286 is; [`check_clock`](Self::check_clock) is what a
    /// run asks before it writes anything, so this never refuses a stamp half-way through.
    pub fn stamp(self, micros: u64) -> String {
        match self {
            Self::Machine(run) => Stamp::new(run, micros).to_string(),
            Self::Ci(run) => CiStamp::new(run, micros)
                .map(|stamp| stamp.to_string())
                .unwrap_or_else(|| {
                    panic!("{micros} is not a time a CI stamp can carry; see Writer::check_clock")
                }),
        }
    }

    /// Whether every stamp from `micros` to `micros + span` can be written, asked once
    /// before a run writes anything.
    pub fn check_clock(self, micros: u64, span: u64) -> Result<(), String> {
        match self {
            Self::Machine(_) => Ok(()),
            Self::Ci(run) => [micros, micros.saturating_add(span)]
                .into_iter()
                .all(|at| CiStamp::new(run, at).is_some())
                .then_some(())
                .ok_or_else(|| {
                    format!(
                        "the clock reads {micros} microseconds since the epoch, which a CI \
                         stamp cannot carry; fix this machine's clock"
                    )
                }),
        }
    }

    /// Everything a stamp of this writer spells before its time: the part of a title a title
    /// search for this run's own artifacts names.
    pub fn stamp_prefix(self) -> String {
        match self {
            Self::Machine(run) => format!("{run}-"),
            Self::Ci(run) => format!("ci-{}-{}-", run.run_id(), run.attempt()),
        }
    }
}

pub fn artifact_title(writer: impl Into<Writer>, stamp_micros: u64) -> String {
    format!("{ARTIFACT_PREFIX}{}", writer.into().stamp(stamp_micros))
}

/// The stamp inside one board issue's own title, when it is spelled the way this lane
/// spells one.
///
/// A *document* this lane writes carries [`DESIGN_TITLE_PREFIX`] in front of the title it
/// was given, because that is how this source spells a document and the source puts it
/// there rather than the caller. Cleanup reads the board's raw titles, so recognition takes
/// that prefix off first — `onetaskgraph_live::artifact::titled_stamp` is that grammar,
/// shared with the janitor.
fn artifact_stamp(title: &str) -> Option<&str> {
    titled_stamp(title, DESIGN_TITLE_PREFIX)
}

/// Whether a board item is an artifact `sweep` may remove.
///
/// **Never anything a live run owns, and that is the whole of what this decides.** What
/// permits a removal is positive evidence that the run which wrote the artifact has ended —
/// its registration lock, which the kernel releases when that process does — and never how
/// old the artifact is. The rule and the window are `onetaskgraph_live::artifact`'s, so this
/// lane and Linear's cannot come to sweep on two different ones. A CI stamp is never one:
/// the machine reader refuses it, so a hosted run's residue is the janitor's alone.
pub fn is_orphan_title(sweep: &Sweep, title: &str) -> bool {
    artifact_stamp(title)
        .and_then(Stamp::read)
        .is_some_and(|stamp| sweep.is_orphan(stamp))
}

/// Whether a board item is one the machine run `run` wrote.
///
/// Every artifact of one run carries that run, so a run names its own for cleanup without
/// touching one another run is still using or one an interrupted earlier run left for
/// [`is_orphan_title`] to sweep.
pub fn is_run_artifact_title(run: Run, title: &str) -> bool {
    artifact_stamp(title)
        .and_then(Stamp::read)
        .is_some_and(|stamp| stamp.run() == run)
}

pub fn artifact_label(writer: impl Into<Writer>, stamp_micros: u64) -> String {
    format!("{LABEL_PREFIX}{}", writer.into().stamp(stamp_micros))
}

/// Whether a repository label is an artifact `sweep` may remove. See [`is_orphan_title`].
pub fn is_orphan_label(sweep: &Sweep, name: &str) -> bool {
    sweep.names_an_orphan(LABEL_PREFIX, name)
}

/// Whether a repository label is one the machine run `run` created.
pub fn is_run_artifact_label(run: Run, name: &str) -> bool {
    labelled_stamp(name)
        .and_then(Stamp::read)
        .is_some_and(|stamp| stamp.run() == run)
}

/// What one lane process has written, which is what its own end-of-run cleanup removes.
///
/// **By its whole stamp, so two processes of one attempt never remove each other's.** A
/// machine stamp names the process, so a machine writer recognises every stamp carrying its
/// run, exactly as it always has. A CI stamp names only the run and the attempt, which two
/// processes of one attempt share, so a CI writer recognises only the stamps it issued
/// itself: every title and label is named through [`Own::title`] and [`Own::label`], which
/// record the stamp before anything carrying it is written.
#[derive(Debug)]
pub struct Own {
    writer: Writer,
    issued: Mutex<BTreeSet<u64>>,
}

impl Own {
    pub fn new(writer: impl Into<Writer>) -> Self {
        Self {
            writer: writer.into(),
            issued: Mutex::new(BTreeSet::new()),
        }
    }

    /// Which form of stamp this process writes.
    pub fn writer(&self) -> Writer {
        self.writer
    }

    fn issue(&self, micros: u64) {
        self.issued
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(micros);
    }

    /// The title of an artifact this process writes at `micros`, recorded as its own.
    pub fn title(&self, micros: u64) -> String {
        self.issue(micros);
        artifact_title(self.writer, micros)
    }

    /// The name of a label this process creates at `micros`, recorded as its own.
    pub fn label(&self, micros: u64) -> String {
        self.issue(micros);
        artifact_label(self.writer, micros)
    }

    fn owns_stamp(&self, spelled: &str) -> bool {
        match self.writer {
            Writer::Machine(run) => Stamp::read(spelled).is_some_and(|stamp| stamp.run() == run),
            Writer::Ci(run) => CiStamp::read(spelled).is_some_and(|stamp| {
                stamp.run() == run
                    && self
                        .issued
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .contains(&stamp.micros())
            }),
        }
    }

    /// Whether a board issue's title is one this process wrote.
    pub fn owns_title(&self, title: &str) -> bool {
        artifact_stamp(title).is_some_and(|stamp| self.owns_stamp(stamp))
    }

    /// Whether a repository label is one this process created.
    pub fn owns_label(&self, name: &str) -> bool {
        labelled_stamp(name).is_some_and(|stamp| self.owns_stamp(stamp))
    }
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
        token: Credential,
        owner: String,
        project_number: u32,
        repository: String,
    },
    Skip(String),
}

/// The session this lane opens against GitHub, by the name its refusals use.
pub const SESSION_NAME: &str = "GitHub Projects";

// llmlint: ignore-block[contracts_have_one_source_or_a_drift_gate] GitHub publishes this grammar as prose in its own UI rather than as an artifact anything here could read, so there is nothing offline to derive these two from or reconcile them against. What stands in for the gate is stated on each function and holds in both directions: this filter may only ever be NARROWER than GitHub's spelling, being a floor on what reaches a URL rather than a mirror of what GitHub accepts, and a divergence either way fails the credentialed lane in the required check — a name GitHub would spell but this refuses fails here, naming the variable and the value, before a credential is spent, and a name this keeps that GitHub does not spell is answered by GitHub with a 404. Neither drift is silent, which is the failure this rule exists to prevent.

/// Whether `login` may be filled into the `{owner}` of this lane's REST endpoint templates.
///
/// Letters, digits and hyphens, no hyphen at either end and none doubled, at most the 39
/// characters GitHub accepted when this was written on 2026-09-03.
///
/// **This is a floor, not a mirror of GitHub's grammar.** What it must never do is keep a
/// value that would address something other than the account it names; being narrower than
/// GitHub costs a refusal that names the variable and the value, which is loud and cheap.
/// So GitHub loosening its spelling cannot make this wrong, only strict — and GitHub is
/// still the authority on whether such an account exists, which it answers with a 404.
fn is_login(login: &str) -> bool {
    !login.is_empty()
        && login.len() <= 39
        && !login.starts_with('-')
        && !login.ends_with('-')
        && !login.contains("--")
        && login
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
}

/// Whether `name` may be filled into the `{repo}` of this lane's REST endpoint templates.
///
/// Letters, digits, hyphens, underscores and dots, at most the 100 characters GitHub
/// accepted when this was written on 2026-09-03, and neither of the two relative path
/// segments. The same floor as [`is_login`], and the same reason.
fn is_repository_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 100
        && name != "."
        && name != ".."
        && name.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
}
// llmlint: ignore-end[contracts_have_one_source_or_a_drift_gate]

/// Decides whether this lane may run, and against which board.
///
/// The board comes from `GH_PROJECTS_OWNER` and `GH_PROJECTS_NUMBER`, and the repository this
/// source creates its issues in comes from `GH_PROJECTS_REPOSITORY`, or the lane does not run.
/// Nothing here asks GitHub which project was updated most recently, for the viewer or for any
/// organization: a credentialed lane that writes and deletes must reach only a board somebody
/// nominated by name, and that requirement — rather than any cleanup — is what keeps it off a
/// board nobody nominated.
///
/// **A nomination that reaches a URL is held to the grammar of what it names.** The two
/// halves of `GH_PROJECTS_REPOSITORY` are filled into the `{owner}` and `{repo}` of this
/// lane's own REST endpoint templates, so a value carrying `?`, `#`, a space or a second
/// path segment would not point at the repository it appears to name — it would address
/// something else, with a credential, and the endpoint the session report named would not
/// be the one that was called. Both halves are therefore refused here unless GitHub itself
/// would spell them that way, which is before anything is sent rather than after. The board
/// is not checked here because it reaches no URL: `GH_PROJECTS_OWNER` and
/// `GH_PROJECTS_NUMBER` are bound as GraphQL variables, and the production boundary this
/// lane drives validates them.
///
/// **The core repository is refused outright.** The lane's issues go into
/// `nickderobertis/onetaskgraph-live-scratch`, a repository that exists for them; a value
/// naming `nickderobertis/onetaskgraph` — however it is cased or padded — is a machine that
/// was never repointed, and it fails here, before a session is opened or a request sent,
/// naming what to change and where, rather than writing test residue into the tracker real
/// work is filed in. See [`refuse_the_core_repository`].
///
/// `ONETASKGRAPH_LIVE_REQUIRED=1` turns a skip into
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
    refuse_the_core_repository(repository)?;
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
    // `Credential::new` is what decides a token is usable, rather than a second reading of
    // "empty" here: the session this lane opens takes one of those and nothing else.
    let Some(token) = Credential::new(token) else {
        return skip("GH_PROJECTS_TOKEN is empty");
    };
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
        .is_none_or(|(owner, name)| !is_login(owner) || !is_repository_name(name))
    {
        return Err(format!(
            "GH_PROJECTS_REPOSITORY must be spelled owner/name, in GitHub's own spelling of \
             each — an owner of letters, digits and single inner hyphens, and a name of \
             letters, digits, hyphens, underscores and dots — not {repository:?}"
        ));
    }
    Ok(LiveLane::Run {
        token,
        owner: owner.to_owned(),
        project_number: number,
        repository: repository.to_owned(),
    })
}

/// Refuses a nomination of the core repository, which this lane no longer writes to.
///
/// Trimmed and compared without regard to case, because GitHub resolves an owner and a
/// repository name that way: `  NickDeRobertis/OneTaskGraph ` addresses the same tracker.
/// The message names the variable, the value refused, the repository to nominate instead,
/// and the two places a machine sets it — which is how a development machine that was never
/// repointed announces itself on its next run instead of quietly writing to the core
/// repository.
///
/// # Errors
///
/// When `repository` names [`CORE_REPOSITORY`].
pub fn refuse_the_core_repository(repository: Option<&str>) -> Result<(), String> {
    match repository {
        Some(named) if named.trim().eq_ignore_ascii_case(CORE_REPOSITORY) => Err(format!(
            "GH_PROJECTS_REPOSITORY is {named:?}, which names the core repository \
             {CORE_REPOSITORY}, where real work is filed: this lane no longer writes there. \
             Nominate {SCRATCH_REPOSITORY}, the repository that exists for this lane's \
             issues — set GH_PROJECTS_REPOSITORY={SCRATCH_REPOSITORY} in this machine's \
             onetaskgraph secrets.env (~/.config/onetaskgraph/secrets.env), or in the \
             environment it runs or pushes from (on an orchestration host, that host's \
             ai-orchestrator .env)"
        )),
        _ => Ok(()),
    }
}

/// Whether the lane may run, against which board, and under which stamp — every variable it
/// reads, read through `read` by name.
#[derive(Debug, PartialEq, Eq)]
pub enum Admission {
    /// Run, writing under `writer`.
    Run {
        token: Credential,
        owner: String,
        project_number: u32,
        repository: String,
        writer: Writer,
    },
    /// Skip, for the reason given.
    Skip(String),
}

/// The lane's whole decision before it opens a session: [`live_lane`] over the nominations,
/// then which stamp it writes.
///
/// One function rather than the steps spelled again in each caller, because `tests/live.rs`
/// drives it against GitHub and `tests/lane_entry.rs` drives the same entry against a
/// loopback stand-in that counts what reaches it. Nothing here sends a request.
///
/// The stamp is decided only once the lane will run: a CI stamp under `GITHUB_ACTIONS=true`,
/// carrying `GITHUB_RUN_ID` and `GITHUB_RUN_ATTEMPT`, and the machine stamp otherwise. A run
/// in GitHub Actions that cannot name its run is refused naming the variable, because a CI
/// artifact that does not name its run is residue nothing may ever remove.
///
/// # Errors
///
/// A misconfiguration, as [`live_lane`] and `CiRun::from_environment` report one.
pub fn admit(read: &dyn Fn(&str) -> Option<String>) -> Result<Admission, String> {
    let lane = live_lane(
        read("GH_PROJECTS_TOKEN").as_deref(),
        // llmlint: ignore[contracts_have_one_source_or_a_drift_gate] .github/workflows/ci.yml spells these three names too, and the drift gate is the lane's own refusal: that workflow sets ONETASKGRAPH_LIVE_REQUIRED=1 on the lane it hands the credential to, so a name spelled differently on either side fails the required check naming the variable rather than skipping green.
        read("GH_PROJECTS_OWNER").as_deref(),
        read("GH_PROJECTS_NUMBER").as_deref(),
        read("GH_PROJECTS_REPOSITORY").as_deref(),
        read(REQUIRED_VARIABLE).as_deref(),
    )?;
    let (token, owner, project_number, repository) = match lane {
        LiveLane::Run {
            token,
            owner,
            project_number,
            repository,
        } => (token, owner, project_number, repository),
        LiveLane::Skip(reason) => return Ok(Admission::Skip(reason)),
    };
    let writer = match CiRun::from_environment(
        read(GITHUB_ACTIONS_VARIABLE).as_deref(),
        read(RUN_ID_VARIABLE).as_deref(),
        read(RUN_ATTEMPT_VARIABLE).as_deref(),
    )? {
        Some(run) => Writer::Ci(run),
        None => Writer::this_machine(),
    };
    Ok(Admission::Run {
        token,
        owner,
        project_number,
        repository,
        writer,
    })
}
