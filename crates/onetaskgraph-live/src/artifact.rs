//! The stamp a live lane writes into every artifact it creates, and what may remove one.
//!
//! Both hosted lanes name every item, project, document and label they write
//! `<their own prefix><stamp>`. Linear's prefix is Linear's business; the GitHub Projects
//! lane's two — [`ARTIFACT_PREFIX`] and [`LABEL_PREFIX`] — are declared here, because that
//! lane is not the only reader of them: the scheduled janitor in
//! `crates/onetaskgraph-live-janitor` recognises the same artifacts, and two spellings of one
//! name would let the two disagree about what is residue. **The stamp, and what it
//! authorises, is one contract**, because every reader has to answer the same question
//! with the same answer: *may this be deleted?*
//!
//! # Two forms of stamp
//!
//! - A **machine stamp**, `<host>-<process>-<micros>` ([`Stamp`]), names the machine and the
//!   process that wrote it. Every run outside GitHub Actions writes one — Linear's lane
//!   everywhere — and what may remove one is the rule below.
//! - A **CI stamp**, `ci-<run id>-<attempt>-<micros>` ([`CiStamp`]), names the GitHub Actions
//!   run and attempt that wrote it. The GitHub Projects lane writes one when `GITHUB_ACTIONS`
//!   is `true` ([`CiRun::from_environment`]). A hosted runner is a fresh machine every time,
//!   so no lock on it is evidence anybody can read later; the run id is, because GitHub
//!   answers for a run by its id. What may remove a CI-stamped artifact is the janitor's
//!   rule: that run read back from GitHub as `completed`, immediately before the delete.
//!
//! The two are disjoint by their first character — a machine stamp starts with a digit and
//! a CI stamp with `ci-` — and each reader refuses the other's form, so the lock sweep below
//! can never act on a CI stamp and the janitor never acts on a machine stamp in the scratch
//! repository.
//!
//! # The rule for a machine stamp
//!
//! **A deletion is authorised by positive evidence that no live run owns the artifact, and
//! never by the artifact's age.** An artifact may be removed only when all of these hold:
//!
//! 1. It carries a [`Run`] whose **host** is the one this machine's [`Registry`] answers
//!    for. A run on another machine announces nothing this one can read, so there is no
//!    evidence about it and it is left alone — for ever, if need be.
//! 2. It is not this run's own.
//! 3. That run's **registration** is in the registry and its lock can be taken. A live run
//!    holds an exclusive lock on its registration for the whole of its life, and the
//!    operating system releases that lock when the process ends — cleanly, by a panic, or
//!    by being killed outright. So a lock this run can take is the kernel saying the run
//!    that wrote the artifact is gone. That is the authorisation.
//! 4. The artifact is older than the window the sweep was made over ([`STALE_AFTER`] for a
//!    lane). This is a **waiting period, never an authorisation**: it can only hold a
//!    removal back, so a window chosen wrong delays a cleanup and cannot take live work.
//!
//! Anything that fails any of the four belongs to a run that may still be alive, and is
//! left exactly where it is.
//!
//! # Why age cannot be the rule
//!
//! It was, and it was wrong. "Not mine, and older than a window" deletes the artifacts of a
//! *concurrent* run that has been going longer than the window — which is exactly what a
//! hung session is, and what a session parked by a hosted API's secondary rate limiter for
//! fifty minutes at a time becomes. The run doing the sweeping cannot tell that run from an
//! abandoned one by looking at its artifacts, because they look identical: old, and somebody
//! else's. The lock is what tells them apart, and it tells them apart with no clock in it at
//! all.
//!
//! # What this deliberately does not do
//!
//! It consults no seat, no lease and no lock held across a whole lane: what a removal rests
//! on is the artifact's own stamp and the registration of the run that wrote it. Two
//! sessions of one lane can therefore run side by side.
//!
//! It also reaches nothing over the network. A registration is a file on the machine the run
//! is on, so deciding what to sweep costs no API budget — which is why a renewed lease
//! against the hosted API, which would answer for a foreign machine too, is not what is here.
//!
//! **The bound that leaves, stated where it is met:** residue written by a run on *another*
//! machine is never swept by this rule, because nothing here is evidence about a foreign
//! process. A machine-stamped artifact stays the lock sweep's, on the machine that wrote
//! it, and nothing else ever removes one from the scratch repository. A run in GitHub
//! Actions writes a CI stamp instead, and an interrupted one — a pull request branch pushed
//! again cancels the run in progress — leaves its residue to the scheduled janitor
//! (`.github/workflows/live-janitor.yml`), which removes it only once GitHub reports that
//! run `completed`. That direction is chosen: a leak is recoverable and a deleted live run
//! is not.

use std::fmt;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::hash::{BuildHasher, Hasher};
use std::io::Write as _;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, SystemTime};

/// The directory live runs register themselves in, when the default is not wanted.
///
/// The default is the platform temporary directory. A check points this at a directory of
/// its own so that what it drives is a registry holding exactly the runs it put there.
pub const REGISTRY_DIRECTORY_VARIABLE: &str = "ONETASKGRAPH_LIVE_RUN_DIR";

/// How long an artifact must have existed before a sweep will consider it at all.
///
/// **Six hours, and it authorises nothing.** Read the module documentation first: what
/// permits a removal is the owning run's registration lock, which has no clock in it. This
/// is a fourth condition on top, and being a condition rather than a permission is what
/// makes its length safe to get wrong — too long delays an abandoned artifact's removal, and
/// too short removes nothing that the lock did not already say was abandoned.
///
/// So why have it at all, and why six hours:
///
/// - It is what is left if the lock ever stops being evidence. `File::try_lock` is `flock`
///   on Unix and `LockFileEx` on Windows, and there are filesystems — some network mounts
///   above all — where a lock is quietly a no-op and every registration reads as takeable.
///   On one of those, this window is the only thing standing between a sweep and a live
///   run's work, so it is set comfortably longer than a session can last: a journey is
///   minutes, a secondary rate limiter refuses a burst for up to an hour at a time, and a
///   run can meet more than one of those.
/// - What it costs is how long an interrupted run's residue lives on a real board before
///   the next run of that lane clears it. Residue is visible and cheap.
///
/// Change it with that trade in mind rather than to make a check faster — a check chooses
/// its own window through [`Sweep::within`], exactly so it never has to wait out this one.
pub const STALE_AFTER: Duration = Duration::from_secs(6 * 60 * 60);

/// The file every live run of this machine registers itself as, less the run's own number.
const REGISTRATION_PREFIX: &str = "onetaskgraph-live-run-";

/// The file this machine's own [`Registry`] identity is kept in.
const HOST_FILE: &str = "onetaskgraph-live-host";

/// The registrations this process holds open, and therefore locked, until it exits.
///
/// A run's registration lasts as long as the run, and "as long as the run" includes a run
/// killed mid-journey — which is the case the whole mechanism exists for, and the one no
/// `Drop` can serve. So the file is held here and never closed: the kernel closes it, and
/// closing it is what publishes that the run is over.
static HELD: Mutex<Vec<(PathBuf, File)>> = Mutex::new(Vec::new());

/// The registry the lanes use, from [`REGISTRY_DIRECTORY_VARIABLE`] or the temporary
/// directory.
static SHARED: LazyLock<Registry> = LazyLock::new(|| {
    Registry::at(
        &std::env::var_os(REGISTRY_DIRECTORY_VARIABLE)
            .map_or_else(std::env::temp_dir, PathBuf::from),
    )
});

/// This process's own run, registered once and reported the same way every time after.
static CURRENT: LazyLock<Run> = LazyLock::new(|| SHARED.enrol());

/// Which run wrote an artifact: the machine that can answer for it, and the process on it.
///
/// The host half is not decoration. Without it a sweep cannot tell a process id it may look
/// up in its own registry from one belonging to a machine it knows nothing about, and
/// treating the second as the first is how a foreign live run's artifacts get deleted.
///
/// A run is either **vouched for** by a registry that can be asked about it, or it is not —
/// the directory could not be written, or its identity could not be read. The two are
/// different states rather than one state with a sentinel in it, and an unvouched run is
/// never anybody's orphan, while a sweep made by one removes nothing at all. On the wire
/// they are one grammar: a host of `0` spells the second, because a stamp is digits.
///
/// # Two runs of one machine that reuse a process id are one `Run`, and that is safe
///
/// An operating system reissues process ids, so a machine can have had two runs numbered
/// alike — one long finished, one going now — and this type cannot tell them apart. Neither
/// direction of that loses work, and [`STALE_AFTER`] is why:
///
/// - The **new** run's artifacts are new, so they have not waited the window out and are
///   never orphans, whatever the registry says about the number they share.
/// - The **old** run's artifacts are protected as well, for as long as the new run holds the
///   registration they now share. They become removable when it ends, which is later than it
///   might have been and is a delay rather than a loss.
///
/// So the conflation costs a cleanup that waits, which is the direction everything here is
/// wrong in on purpose. `crates/onetaskgraph-github-projects/tests/sweep_gate.rs` drives it
/// against the real cleanup and a real re-registration of one number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Run {
    host: Option<NonZeroU32>,
    process: NonZeroU32,
}

impl Run {
    /// The run `process`, on the machine whose registry identity is `host`.
    #[must_use]
    pub fn vouched(host: NonZeroU32, process: NonZeroU32) -> Self {
        Self {
            host: Some(host),
            process,
        }
    }

    /// The run `process`, which no registry can answer for.
    #[must_use]
    pub fn unvouched(process: NonZeroU32) -> Self {
        Self {
            host: None,
            process,
        }
    }

    /// This process's run, registered in the shared registry the first time it is asked for.
    ///
    /// Registering here rather than at [`crate::Session::open`] is deliberate: this is the
    /// call that names an artifact, so a run cannot write one without first having published
    /// the lock that protects it.
    #[must_use]
    pub fn current() -> Self {
        *CURRENT
    }

    /// The machine whose registry can answer for this run, when one can.
    #[must_use]
    pub fn host(self) -> Option<NonZeroU32> {
        self.host
    }

    /// The process this run is.
    #[must_use]
    pub fn process(self) -> NonZeroU32 {
        self.process
    }

    /// The run `spelled` names, or `None` when it names no run.
    ///
    /// Digits only, on both halves, and both halves non-empty. A host of `0` is a run no
    /// registry vouches for rather than a run on machine zero; a *process* of zero is no run
    /// at all, because no operating system numbers a running process zero.
    #[must_use]
    pub fn read(spelled: &str) -> Option<Self> {
        let (host, process) = spelled.split_once('-')?;
        Some(Self {
            host: NonZeroU32::new(number(host)?),
            process: NonZeroU32::new(number(process)?)?,
        })
    }
}

impl fmt::Display for Run {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}-{}",
            self.host.map_or(0, NonZeroU32::get),
            self.process
        )
    }
}

/// The machine stamp, `<host>-<process>-<microsecond timestamp>`, that an artifact written
/// outside GitHub Actions ends with. The other form is [`CiStamp`].
///
/// One type rather than two lanes' worth of `split_once('-')`, because writing a stamp and
/// reading one back have to agree: a lane that formatted what this cannot parse would write
/// artifacts no sweep could ever recognise, on somebody's real board.
///
/// Digits and hyphens throughout, which is what lets a lane put one in a URL path unescaped.
/// The timestamp is unsigned for that reason and not for tidiness — an instant before 1970
/// would spell a second hyphen and could never be read back as the stamp it was written as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    run: Run,
    micros: u64,
}

impl Stamp {
    /// The stamp `run` puts on the artifact it writes at `micros`.
    #[must_use]
    pub fn new(run: Run, micros: u64) -> Self {
        Self { run, micros }
    }

    /// Which run wrote the artifact carrying this stamp.
    #[must_use]
    pub fn run(self) -> Run {
        self.run
    }

    /// When it was written, in microseconds since the epoch.
    #[must_use]
    pub fn micros(self) -> u64 {
        self.micros
    }

    /// The stamp `suffix` spells, or `None` when it spells no stamp at all.
    ///
    /// Three digit groups, all non-empty: `1-2-3` is a stamp and `1-2`, `1-2-3-4`, `-2-3`
    /// and `a-2-3` are not. A name whose suffix is not a stamp is not this lane's artifact,
    /// so the sweep passes over it rather than guessing. Every [`CiStamp`] is refused here —
    /// `ci` is not digits — which is what keeps the lock sweep off an artifact a hosted run
    /// wrote: no registry on any machine can answer for one.
    #[must_use]
    pub fn read(suffix: &str) -> Option<Self> {
        let (host, rest) = suffix.split_once('-')?;
        let (process, micros) = rest.split_once('-')?;
        Some(Self {
            run: Run {
                host: NonZeroU32::new(number(host)?),
                process: NonZeroU32::new(number(process)?)?,
            },
            micros: number(micros)?,
        })
    }
}

impl fmt::Display for Stamp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}-{}", self.run, self.micros)
    }
}

/// The widest a stamp can be, in characters, which is what a lane has to leave room for.
///
/// A hosted API holds a label name to a length of its own, and a lane that puts a stamp in
/// one has to know how much of that budget the stamp takes *before* it writes on somebody's
/// real repository rather than after. So the figure is declared here, where the three parts
/// are, rather than counted again by each lane: nine digits of machine identity — which is
/// what `fresh_host` bounds one to — ten of process id, twenty of microseconds, and the two
/// hyphens between them.
pub const WIDEST_STAMP: usize = 9 + 1 + 10 + 1 + 20;

/// The prefix of every issue — and so of every board item — the GitHub Projects lane writes.
///
/// The rest of a title is a stamp, in either form, and a *document* the lane writes carries
/// that source's own design prefix in front of the whole ([`titled_stamp`] takes it off).
/// Declared here rather than in the lane, because the lane's tests are not the only reader:
/// the janitor recognises exactly these titles, and a second spelling would let the two come
/// to disagree about what is residue on somebody's real board.
pub const ARTIFACT_PREFIX: &str = "onetaskgraph live cleanup ";

/// The prefix of the one repository label each GitHub Projects lane run creates.
///
/// **Short on purpose**, where the issue prefix is a readable sentence: GitHub holds a label
/// name to fifty characters, and this plus the widest stamp of either form has to fit —
/// which is why [`WIDEST_CI_STAMP`] is held equal to [`WIDEST_STAMP`].
pub const LABEL_PREFIX: &str = "otg-live-";

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
/// repository whose Actions runs a CI stamp's run id is read back from.
pub const CORE_REPOSITORY: &str = "nickderobertis/onetaskgraph";

/// The variable GitHub Actions sets to `true` on every run it hosts.
pub const GITHUB_ACTIONS_VARIABLE: &str = "GITHUB_ACTIONS";

/// The variable naming the GitHub Actions run, which a re-run keeps.
pub const RUN_ID_VARIABLE: &str = "GITHUB_RUN_ID";

/// The variable naming which attempt of that run this is, from 1.
pub const RUN_ATTEMPT_VARIABLE: &str = "GITHUB_RUN_ATTEMPT";

/// What every CI stamp starts with, which no machine stamp can: those start with a digit.
pub const CI_STAMP_PREFIX: &str = "ci-";

/// The most digits a CI stamp's run id may have.
const RUN_ID_DIGITS: usize = 17;

/// The most digits a CI stamp's attempt may have.
const ATTEMPT_DIGITS: usize = 3;

/// The most digits a CI stamp's time may have: microseconds since the epoch, enough until
/// the year 2286.
const CI_MICROS_DIGITS: usize = 16;

/// The widest CI stamp the grammar admits, in characters.
///
/// `ci-`, seventeen digits of run id, three of attempt, sixteen of microseconds and the two
/// hyphens between them. Held equal to [`WIDEST_STAMP`], so a lane that leaves room for one
/// form has left room for the other and [`LABEL_PREFIX`] plus either fits GitHub's limit.
pub const WIDEST_CI_STAMP: usize =
    CI_STAMP_PREFIX.len() + RUN_ID_DIGITS + 1 + ATTEMPT_DIGITS + 1 + CI_MICROS_DIGITS;

const _: () = assert!(WIDEST_CI_STAMP == WIDEST_STAMP);

/// Which GitHub Actions run, and which attempt of it, wrote an artifact.
///
/// The run id is what GitHub answers for — `GET /repos/{owner}/{repo}/actions/runs/{id}`
/// reports the run's latest attempt — so a CI-stamped artifact names the one thing that can
/// later say whether anything still owns it. The attempt is carried so that a re-run's
/// artifacts are told from the first attempt's by a reader of the title, and so that one
/// process's own cleanup recognises exactly its own stamps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CiRun {
    run_id: u64,
    attempt: u16,
}

impl CiRun {
    /// Run `run_id`, attempt `attempt`, when both are within the widths the grammar admits.
    #[must_use]
    pub fn new(run_id: u64, attempt: u64) -> Option<Self> {
        Some(Self {
            run_id: within(run_id, RUN_ID_DIGITS)?,
            attempt: u16::try_from(within(attempt, ATTEMPT_DIGITS)?).ok()?,
        })
    }

    /// The GitHub Actions run id.
    #[must_use]
    pub fn run_id(self) -> u64 {
        self.run_id
    }

    /// Which attempt of that run, from 1.
    #[must_use]
    pub fn attempt(self) -> u16 {
        self.attempt
    }

    /// Which form of stamp a lane in this environment writes, read from GitHub Actions' own
    /// variables.
    ///
    /// `Ok(None)` — a machine stamp — unless `github_actions` is exactly `true`. When it is,
    /// both `run_id` and `attempt` have to be decimal numbers of at least 1, with no sign, no
    /// leading zero and no more digits than the grammar admits, or this refuses naming the
    /// variable: **a CI artifact that does not name its run is residue nothing may ever
    /// remove**, so a lane that cannot name its run must write nothing at all.
    ///
    /// # Errors
    ///
    /// When `github_actions` is `true` and either value is missing or outside the grammar.
    pub fn from_environment(
        github_actions: Option<&str>,
        run_id: Option<&str>,
        attempt: Option<&str>,
    ) -> Result<Option<Self>, String> {
        if github_actions != Some("true") {
            return Ok(None);
        }
        let field = |variable: &str, value: Option<&str>, widest: usize| {
            let refuse = |what: String| {
                format!(
                    "{GITHUB_ACTIONS_VARIABLE} is true, so this lane stamps every artifact with \
                     the GitHub Actions run that wrote it, but {variable} {what}. It has to be a \
                     decimal number of 1 to {widest} digits, at least 1, with no sign and no \
                     leading zero. A CI artifact that does not name its run is residue nothing \
                     may ever remove, so this lane writes nothing until {variable} is set the \
                     way GitHub Actions sets it"
                )
            };
            let value = value.ok_or_else(|| refuse("is not set".to_owned()))?;
            bounded(value, widest).ok_or_else(|| refuse(format!("is {value:?}")))
        };
        let run_id = field(RUN_ID_VARIABLE, run_id, RUN_ID_DIGITS)?;
        let attempt = field(RUN_ATTEMPT_VARIABLE, attempt, ATTEMPT_DIGITS)?;
        Ok(Self::new(run_id, attempt))
    }
}

/// The `ci-<run id>-<attempt>-<micros>` an artifact written in GitHub Actions ends with.
///
/// Its characters are `c`, `i`, digits and hyphens, so a label carrying one still goes into
/// a URL path unescaped, and a person reading an issue title sees `ci-` and the run id.
/// Every field is a decimal number of at least 1 with no sign and no leading zero, so each
/// value has exactly one spelling and the reader and the writer agree on every one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CiStamp {
    run: CiRun,
    micros: u64,
}

impl CiStamp {
    /// The stamp `run` puts on an artifact it writes at `micros`, when `micros` is within the
    /// grammar: at least 1, and no more than sixteen digits.
    #[must_use]
    pub fn new(run: CiRun, micros: u64) -> Option<Self> {
        Some(Self {
            run,
            micros: within(micros, CI_MICROS_DIGITS)?,
        })
    }

    /// Which run and attempt wrote the artifact carrying this stamp.
    #[must_use]
    pub fn run(self) -> CiRun {
        self.run
    }

    /// When it was written, in microseconds since the epoch.
    #[must_use]
    pub fn micros(self) -> u64 {
        self.micros
    }

    /// The CI stamp `suffix` spells, or `None` when it spells none.
    ///
    /// Exactly `ci-` and three fields, each within its width, at least 1, with no sign and no
    /// leading zero. Every machine stamp is refused, because none starts with `ci-`.
    #[must_use]
    pub fn read(suffix: &str) -> Option<Self> {
        let mut fields = suffix.strip_prefix(CI_STAMP_PREFIX)?.split('-');
        let run_id = bounded(fields.next()?, RUN_ID_DIGITS)?;
        let attempt = bounded(fields.next()?, ATTEMPT_DIGITS)?;
        let micros = bounded(fields.next()?, CI_MICROS_DIGITS)?;
        if fields.next().is_some() {
            return None;
        }
        Self::new(CiRun::new(run_id, attempt)?, micros)
    }
}

impl fmt::Display for CiStamp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{CI_STAMP_PREFIX}{}-{}-{}",
            self.run.run_id, self.run.attempt, self.micros
        )
    }
}

/// The stamp half of an issue title the GitHub Projects lane wrote, in either form, unread.
///
/// `design_prefix` is the source's own spelling of a document — the lane's documents carry
/// it in front of [`ARTIFACT_PREFIX`] — and it is a parameter because this crate depends on
/// nothing of the workspace: the lane and the janitor both pass the plugin crate's
/// `DESIGN_TITLE_PREFIX`. At most one is taken off, and [`ARTIFACT_PREFIX`] must follow it
/// directly; any other title is not the lane's and gets `None`.
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

/// `value`, when it is at least 1 and spelled in no more than `widest` digits.
fn within(value: u64, widest: usize) -> Option<u64> {
    (value > 0 && value.to_string().len() <= widest).then_some(value)
}

/// `spelled` as a CI stamp field: at most `widest` digits, at least 1, no leading zero.
fn bounded(spelled: &str, widest: usize) -> Option<u64> {
    if spelled.len() > widest || spelled.starts_with('0') {
        return None;
    }
    number::<u64>(spelled).and_then(|value| within(value, widest))
}

/// The instant a stamp written now would carry.
///
/// Here rather than in each lane, so both stamp on one clock and neither has to convert a
/// signed timestamp into the unsigned one a stamp can spell. A clock set before 1970 stamps
/// at zero, which is an instant no sweep will ever mistake for a live run's work.
#[must_use]
pub fn now_micros() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |since| {
            u64::try_from(since.as_micros()).unwrap_or(u64::MAX)
        })
}

/// `spelled` as a number, when it is digits and nothing else and fits.
///
/// `str::parse` alone accepts a leading `+` and, for a signed type, a leading `-` — neither
/// of which anything here writes, and both of which would let one field of a stamp swallow
/// the separator before the next.
fn number<T: std::str::FromStr>(spelled: &str) -> Option<T> {
    if spelled.is_empty() || !spelled.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    spelled.parse().ok()
}

/// Where the live runs of one machine say they are alive, and the identity of that machine.
///
/// One directory holding one identity file and one file per run. Nothing in it is read for
/// its contents except the identity: what a registration says is said by whether its lock
/// can be taken, which is a fact the kernel keeps rather than one a killed process could
/// have failed to write.
#[derive(Debug)]
pub struct Registry {
    directory: PathBuf,
    host: Option<NonZeroU32>,
}

impl Registry {
    /// The registry kept in `directory`, creating it and its identity if they are not there.
    ///
    /// A directory that cannot be made, or an identity that can be neither read nor written,
    /// leaves this registry with no identity — which is it saying it can answer for nothing,
    /// and which makes every sweep made through it remove nothing.
    #[must_use]
    pub fn at(directory: &Path) -> Self {
        let host = if fs::create_dir_all(directory).is_ok() {
            host_of(&directory.join(HOST_FILE))
        } else {
            None
        };
        Self {
            directory: directory.to_owned(),
            host,
        }
    }

    /// The registry the lanes use, from [`REGISTRY_DIRECTORY_VARIABLE`] or the temporary
    /// directory.
    #[must_use]
    pub fn shared() -> &'static Self {
        &SHARED
    }

    /// This machine's identity as this registry knows it, when it has one.
    #[must_use]
    pub fn host(&self) -> Option<NonZeroU32> {
        self.host
    }

    /// Register this process as a live run, and report the run it is.
    ///
    /// The registration is held for the rest of the process's life. Calling this again
    /// returns the same run rather than a second registration.
    #[must_use]
    pub fn enrol(&self) -> Run {
        let process = this_process();
        let path = self.registration_path(process);
        let mut held = HELD.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(host) = self.host
            && held.iter().any(|(registered, _)| *registered == path)
        {
            // Already registered here, and the lock this call could not take is this
            // process's own. A second registration of one run is not a second run.
            return Run::vouched(host, process);
        }
        match Registration::take(self, process) {
            Some(registration) => {
                let run = registration.run();
                held.push((path, registration.into_file()));
                run
            }
            // Nothing here could vouch for this run, so it stamps its artifacts as vouched
            // for by nobody — which is what keeps every other run's sweep off them for ever.
            None => Run::unvouched(process),
        }
    }

    /// Where the run `process` registers itself in this registry.
    #[must_use]
    pub fn registration_path(&self, process: NonZeroU32) -> PathBuf {
        self.directory
            .join(format!("{REGISTRATION_PREFIX}{process}"))
    }

    /// The runs of this machine the kernel says are over.
    ///
    /// A registration whose lock this call can take belonged to a process that no longer
    /// exists, whatever it was doing when it stopped. One whose lock is held is a live run.
    /// One this call cannot even open is neither: no evidence, so it is not reported, and a
    /// sweep therefore leaves its artifacts alone.
    ///
    /// **A shared lock, because this is a question rather than a claim.** An exclusive one
    /// would be refused by exactly the same runs and would, for the instant it was held,
    /// refuse a run that was registering right then — which would leave that run unable to
    /// say who it is. Two sweeps asking at once do not refuse each other either.
    #[must_use]
    pub fn finished_runs(&self) -> Vec<NonZeroU32> {
        let Ok(entries) = fs::read_dir(&self.directory) else {
            return Vec::new();
        };
        let mut finished = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(process) = name
                .to_str()
                .and_then(|name| name.strip_prefix(REGISTRATION_PREFIX))
                .and_then(number::<u32>)
                .and_then(NonZeroU32::new)
            else {
                continue;
            };
            if let Ok(file) = OpenOptions::new().read(true).write(true).open(entry.path())
                && file.try_lock_shared().is_ok()
            {
                let _ = file.unlock();
                finished.push(process);
            }
        }
        finished
    }

    /// The sweep `run` makes at `now_micros` over `window`, decided against this registry.
    #[must_use]
    pub fn sweep(&self, run: Run, now_micros: u64, window: Duration) -> Sweep {
        Sweep {
            run,
            now_micros,
            window,
            finished: if run.host().is_some() && run.host() == self.host {
                self.finished_runs()
            } else {
                // A run this registry does not answer for gets no evidence from it, and a
                // sweep with no evidence removes nothing.
                Vec::new()
            },
        }
    }
}

/// One live run's claim on its own artifacts, held as a lock the kernel releases when the
/// process ends.
///
/// A lane takes one through [`Registry::enrol`] and never sees this type. A check takes one
/// for a nominated run id, which is how it stands a second live run beside the one sweeping
/// without waiting for anything: the lock is real, and a sweep cannot tell it from a lock a
/// separate process holds because there is nothing to tell apart.
#[derive(Debug)]
pub struct Registration {
    run: Run,
    file: File,
}

impl Registration {
    /// Register `process` in `registry`, or `None` when it could not be registered.
    ///
    /// `None` covers both a registry that could not be written and a registration whose
    /// lock is already held — including by this very process, which is what a second
    /// registration of one run is.
    #[must_use]
    pub fn take(registry: &Registry, process: NonZeroU32) -> Option<Self> {
        let host = registry.host?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(registry.registration_path(process))
            .ok()?;
        match file.try_lock() {
            Ok(()) => Some(Self {
                run: Run::vouched(host, process),
                file,
            }),
            Err(TryLockError::WouldBlock | TryLockError::Error(_)) => None,
        }
    }

    /// The run this registration is for.
    #[must_use]
    pub fn run(&self) -> Run {
        self.run
    }

    /// Give up the guard and keep the lock, by keeping the file open.
    fn into_file(self) -> File {
        self.file
    }
}

/// One run's decision about which of the artifacts it can see no live run owns.
///
/// Made once at the point the sweep starts and then asked of each name, so every artifact in
/// one sweep is judged against the same reading of the registry and the same instant, rather
/// than against a clock and a directory that both move while the sweep pages through a board.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sweep {
    run: Run,
    now_micros: u64,
    window: Duration,
    /// The runs of this machine the registry says are over — the only ones this sweep may
    /// remove anything of.
    finished: Vec<NonZeroU32>,
}

impl Sweep {
    /// The sweep `run` makes at `now_micros`, over the window this contract declares.
    ///
    /// This is what a lane uses. [`Sweep::within`] is for a check that would otherwise have
    /// to wait out [`STALE_AFTER`] to observe anything.
    #[must_use]
    pub fn of(run: Run, now_micros: u64) -> Self {
        Self::within(run, now_micros, STALE_AFTER)
    }

    /// The same decision over a window the caller chose.
    ///
    /// A check drives this so that proving what a sweep does needs no session that runs for
    /// six hours — and so that what it proves is *independent* of the window, which is the
    /// property [`STALE_AFTER`] is chosen under rather than relied upon.
    #[must_use]
    pub fn within(run: Run, now_micros: u64, window: Duration) -> Self {
        Registry::shared().sweep(run, now_micros, window)
    }

    /// The run this sweep is being made by, whose artifacts it never touches.
    #[must_use]
    pub fn run(&self) -> Run {
        self.run
    }

    /// Whether the artifact carrying `stamp` is one no live run owns.
    ///
    /// All four conditions of the module documentation, in the order that makes what is
    /// missing readable: the wrong machine and this run's own are refused outright, then the
    /// registry has to say the owning run is over, and only then does age come into it.
    ///
    /// An artifact stamped in the future is never removed: two machines' clocks disagree, and
    /// the direction to be wrong in is leaving somebody's work alone. That falls out of the
    /// saturating subtraction rather than being a case of its own.
    ///
    /// The registry was read once, when this sweep was made, and a run may have started or
    /// ended since — including one reusing a process id this reading called finished. The
    /// window is what covers that: an artifact of a run that started after this reading is
    /// newer than this reading, so it has not waited the window out. See [`Run`].
    #[must_use]
    pub fn is_orphan(&self, stamp: Stamp) -> bool {
        if self.run.host.is_none() || stamp.run.host != self.run.host {
            return false;
        }
        if stamp.run.process == self.run.process {
            return false;
        }
        if !self.finished.contains(&stamp.run.process) {
            return false;
        }
        u128::from(self.now_micros.saturating_sub(stamp.micros)) > self.window.as_micros()
    }

    /// Whether `name` is an artifact under `prefix` that no live run owns.
    ///
    /// The whole decision for a lane whose names are `prefix` then a stamp: anything that is
    /// not spelled that way belongs to somebody else entirely and is never touched.
    #[must_use]
    pub fn names_an_orphan(&self, prefix: &str, name: &str) -> bool {
        name.strip_prefix(prefix)
            .and_then(Stamp::read)
            .is_some_and(|stamp| self.is_orphan(stamp))
    }
}

/// This machine's registry identity, read from `path` or written there if it has none.
///
/// `None` when it can be neither read nor written, which is this machine declining to answer
/// for any run rather than answering wrongly.
fn host_of(path: &Path) -> Option<NonZeroU32> {
    for _ in 0..8 {
        if let Some(host) = read_host(path) {
            return Some(host);
        }
        // `create_new`, so two processes arriving together cannot each write an identity and
        // leave the machine with two. The loser reads the winner's on the next turn.
        if let Ok(mut file) = OpenOptions::new().write(true).create_new(true).open(path) {
            let host = fresh_host();
            if writeln!(file, "{host}").and_then(|()| file.flush()).is_ok() {
                return Some(host);
            }
            // An identity half-written is one nothing can read, and leaving it there would
            // make this machine hostless for ever rather than for this call.
            let _ = fs::remove_file(path);
            return None;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    None
}

/// The identity `path` holds, when it holds one that means anything.
fn read_host(path: &Path) -> Option<NonZeroU32> {
    NonZeroU32::new(number(fs::read_to_string(path).ok()?.trim())?)
}

/// This process, as a run identity.
///
/// `std::process::id` is a `u32` and a run is not: no operating system numbers a running
/// process zero — it is the scheduler on Unix and the idle process on Windows — so a `Run`
/// carrying one would be a state nothing could ever be in.
fn this_process() -> NonZeroU32 {
    NonZeroU32::new(std::process::id()).expect("no operating system numbers a running process zero")
}

/// An identity for a machine that has none yet.
///
/// It has to differ between machines and it never has to be secret, so it is a hash of the
/// two things that differ — when this process started asking and which process it is — under
/// `RandomState`, whose seed is itself random per process. Bounded to nine digits so that a
/// whole stamp still fits inside a hosted API's own limit on a label name, and forced
/// non-zero because zero is how the wire grammar spells "no registry answers here".
fn fresh_host() -> NonZeroU32 {
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos()),
    );
    hasher.write_u32(std::process::id());
    NonZeroU32::new(u32::try_from(hasher.finish() % 999_999_937).unwrap_or(1) | 1)
        .expect("an identity forced odd is never zero")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A window a test can drive either side of without waiting for anything.
    const WINDOW: Duration = Duration::from_secs(60);

    const NOW: u64 = 1_787_816_134_627_361;

    /// A machine identity written out by hand, for the names this file spells itself.
    fn host(number: u32) -> NonZeroU32 {
        NonZeroU32::new(number).expect("a test host identity is never zero")
    }

    /// The same for a run's process id, which no operating system numbers zero either.
    fn process(number: u32) -> NonZeroU32 {
        NonZeroU32::new(number).expect("a test process id is never zero")
    }

    /// A registry directory of this test's own, so what it holds is what this test put there.
    fn scratch(what: &str) -> PathBuf {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let directory = std::env::temp_dir().join(format!(
            "onetaskgraph-live-artifact-{}-{what}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&directory);
        directory
    }

    fn micros(window: Duration) -> u64 {
        u64::try_from(window.as_micros()).expect("a test window fits in the stamp's own type")
    }

    #[test]
    fn a_stamp_reads_back_exactly_what_was_written() {
        let stamp = Stamp::new(Run::vouched(host(41), process(2533)), NOW);
        assert_eq!(stamp.to_string(), format!("41-2533-{NOW}"));
        assert_eq!(Stamp::read(&stamp.to_string()), Some(stamp));
        assert_eq!(stamp.run(), Run::vouched(host(41), process(2533)));
        assert_eq!(stamp.run().host(), Some(host(41)));
        assert_eq!(stamp.run().process(), process(2533));
        assert_eq!(stamp.micros(), NOW);
        assert_eq!(
            Run::read("41-2533"),
            Some(Run::vouched(host(41), process(2533)))
        );
        assert_eq!(Run::vouched(host(41), process(2533)).to_string(), "41-2533");
    }

    #[test]
    fn a_run_no_registry_vouches_for_reads_and_writes_as_one_rather_than_as_machine_zero() {
        let unvouched = Run::unvouched(process(2533));
        assert_eq!(unvouched.host(), None);
        assert_eq!(unvouched.to_string(), "0-2533");
        assert_eq!(Run::read("0-2533"), Some(unvouched));
        let stamp = Stamp::new(unvouched, NOW);
        assert_eq!(Stamp::read(&stamp.to_string()), Some(stamp));
    }

    #[test]
    fn a_suffix_that_is_not_a_stamp_is_not_read_as_one() {
        for spelled in [
            "",
            "2533",
            "41-2533",
            "-41-2533",
            "41--2533",
            "41-2533-",
            "a-41-2533",
            "41-a-2533",
            "41-2533-a",
            "41-2533-17-19",
            "41 -2533-17",
            "+41-2533-17",
            // A signed spelling of the timestamp, which nothing here can write: the stamp's
            // own clock is unsigned, so an instant that would need a second hyphen does not
            // exist rather than being written and then unreadable.
            "41-2533--17",
            // Wider than the halves they are parsed into: a name nothing here could have
            // written, and reading it as a stamp would be reading a number that overflowed.
            "99999999999-2533-17",
            "41-99999999999-17",
            "41-2533-99999999999999999999999",
        ] {
            assert_eq!(Stamp::read(spelled), None, "{spelled:?} is not a stamp");
        }
        for spelled in ["", "41", "-41", "41-", "a-41", "41-a", "41-2533-17"] {
            assert_eq!(Run::read(spelled), None, "{spelled:?} is not a run");
        }
    }

    #[test]
    fn a_stamp_written_now_is_one_this_grammar_can_read_back() {
        let stamp = Stamp::new(Run::unvouched(process(std::process::id())), now_micros());
        assert_eq!(Stamp::read(&stamp.to_string()), Some(stamp));
        assert!(
            stamp.micros() > NOW,
            "the clock is not before this file was"
        );
    }

    #[test]
    fn one_machine_keeps_one_identity_however_many_registries_read_it() {
        let directory = scratch("identity");
        let first = Registry::at(&directory);
        let second = Registry::at(&directory);
        assert!(
            first.host().is_some(),
            "a writable directory has an identity"
        );
        assert_eq!(first.host(), second.host());
        // And a second machine's is a different one, which is what makes a foreign run's
        // artifacts recognisable as foreign.
        assert_ne!(
            first.host(),
            Registry::at(&scratch("identity-elsewhere")).host()
        );
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_registry_that_cannot_be_written_answers_for_nothing() {
        // A directory that cannot be made, because a file is already at that path. The
        // machine then has no identity, so it vouches for no run and its sweeps take nothing.
        let occupied = scratch("occupied");
        fs::create_dir_all(occupied.parent().expect("a temporary directory")).expect("a parent");
        fs::write(&occupied, b"not a directory").expect("a file in the way");
        let registry = Registry::at(&occupied.join("registry"));
        assert_eq!(registry.host(), None);
        assert_eq!(
            registry.enrol(),
            Run::unvouched(process(std::process::id()))
        );
        assert!(Registration::take(&registry, process(4242)).is_none());
        let sweep = registry.sweep(
            Run::vouched(host(7), process(std::process::id())),
            NOW,
            WINDOW,
        );
        assert!(!sweep.is_orphan(Stamp::new(Run::vouched(host(7), process(4242)), 0)));
        // And a run nothing vouches for is nobody's orphan either, sweeping or swept.
        assert!(
            !Sweep::of(Run::unvouched(process(2533)), NOW)
                .is_orphan(Stamp::new(Run::unvouched(process(4242)), 0))
        );
        let _ = fs::remove_file(&occupied);
    }

    #[test]
    fn a_run_is_over_exactly_when_its_registration_can_be_locked_again() {
        let directory = scratch("liveness");
        let registry = Registry::at(&directory);
        let live =
            Registration::take(&registry, process(4242)).expect("a registration this run can take");
        assert_eq!(
            live.run(),
            Run::vouched(registry.host().expect("an identity"), process(4242))
        );
        assert!(
            !registry.finished_runs().contains(&process(4242)),
            "a run holding its registration is not over"
        );
        // A second registration of the same run is refused: the lock is already held, and
        // that is true whichever process holds it.
        assert!(Registration::take(&registry, process(4242)).is_none());
        drop(live);
        assert!(
            registry.finished_runs().contains(&process(4242)),
            "a registration nothing holds is a run the kernel says has ended"
        );
        // Whatever else is in the directory is not a registration and is never read as one.
        fs::write(directory.join("onetaskgraph-live-run-notanumber"), b"").expect("a stray file");
        fs::write(directory.join("something-else"), b"").expect("another stray file");
        assert_eq!(registry.finished_runs(), vec![process(4242)]);
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn enrolling_twice_reports_one_run_rather_than_refusing_the_second() {
        let directory = scratch("enrol");
        let registry = Registry::at(&directory);
        let run = registry.enrol();
        assert_eq!(
            run,
            Run::vouched(
                registry.host().expect("an identity"),
                process(std::process::id())
            )
        );
        assert_eq!(registry.enrol(), run);
        assert!(
            !registry
                .finished_runs()
                .contains(&process(std::process::id())),
            "this process is registered here and is not over"
        );
        // Deliberately not removed: the registration is held for the life of the process, so
        // the directory is left for the operating system to clear.
    }

    #[test]
    fn a_sweep_leaves_a_live_runs_artifacts_however_old_they_are() {
        // The case the age rule got wrong: a concurrent run that has been going longer than
        // the window — a hung session, or one a secondary rate limiter parked — is
        // indistinguishable from an abandoned one by age alone. Its lock is what tells them
        // apart, and it says nothing about time.
        let directory = scratch("live-run");
        let registry = Registry::at(&directory);
        let mine = registry.enrol();
        let theirs = Registration::take(&registry, process(4242)).expect("a second live run");
        let sweep = registry.sweep(mine, NOW, WINDOW);
        for age in [0, micros(WINDOW), micros(WINDOW) * 100_000] {
            assert!(
                !sweep.is_orphan(Stamp::new(theirs.run(), NOW - age)),
                "a live run's artifact {age} microseconds old was taken"
            );
            assert!(
                !sweep.is_orphan(Stamp::new(mine, NOW - age)),
                "this run's own artifact {age} microseconds old was taken"
            );
        }
        assert_eq!(sweep.run(), mine);
        drop(theirs);
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_sweep_takes_an_ended_runs_artifact_once_it_has_waited_the_window_out() {
        let directory = scratch("ended-run");
        let registry = Registry::at(&directory);
        let mine = registry.enrol();
        let ended = {
            let registration =
                Registration::take(&registry, process(4242)).expect("a run that will end");
            registration.run()
        };
        let stamp = |age: u64| Stamp::new(ended, NOW - age);
        let sweep = registry.sweep(mine, NOW, WINDOW);
        assert!(sweep.is_orphan(stamp(micros(WINDOW) + 1)));
        // The window is a waiting period and only that: it holds a removal back and it
        // never authorises one.
        assert!(!sweep.is_orphan(stamp(micros(WINDOW))));
        assert!(!sweep.is_orphan(stamp(0)));
        // Nor is an artifact stamped in the future, whose clock disagrees with this one's.
        assert!(!sweep.is_orphan(Stamp::new(ended, NOW + micros(WINDOW) * 100)));
        // A run this registry never heard of is no evidence at all, ended or not.
        assert!(!sweep.is_orphan(Stamp::new(
            Run::vouched(registry.host().expect("an identity"), process(4243)),
            NOW - micros(WINDOW) - 1
        )));
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_run_of_another_machine_is_never_swept_whatever_its_number() {
        let directory = scratch("foreign-host");
        let registry = Registry::at(&directory);
        let mine = registry.enrol();
        drop(Registration::take(&registry, process(4242)).expect("a run that has ended here"));
        let sweep = registry.sweep(mine, NOW, WINDOW);
        let stale = NOW - micros(WINDOW) - 1;
        let here = registry.host().expect("an identity");
        let elsewhere = host(here.get().wrapping_add(1).max(1));
        assert!(
            !sweep.is_orphan(Stamp::new(Run::vouched(elsewhere, process(4242)), stale)),
            "a process id of another machine was looked up in this machine's registry"
        );
        assert!(!sweep.is_orphan(Stamp::new(Run::unvouched(process(4242)), stale)));
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn only_a_name_spelled_this_way_under_this_prefix_is_ever_an_orphan() {
        let directory = scratch("names");
        let registry = Registry::at(&directory);
        let mine = registry.enrol();
        let ended = {
            let registration =
                Registration::take(&registry, process(4242)).expect("a run that has ended");
            registration.run()
        };
        let sweep = registry.sweep(mine, NOW, WINDOW);
        let stale = NOW - micros(WINDOW) - 1;
        assert!(sweep.names_an_orphan("live ", &format!("live {}-{stale}", ended)));
        for foreign in [
            format!("{ended}-{stale}"),
            format!("copy of live {ended}-{stale}"),
            format!("live {ended}"),
            "live ".to_owned(),
            format!("live {mine}-{stale}"),
        ] {
            assert!(
                !sweep.names_an_orphan("live ", &foreign),
                "{foreign:?} is not an orphan of this run's"
            );
        }
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_process_id_reissued_to_a_new_run_costs_a_delay_and_never_a_deletion() {
        // The operating system's own reuse of a number, made real: one run registers and
        // ends, and a second registers under the very same number. The two are one `Run` to
        // this contract, and neither loses work — see the note on that type.
        let directory = scratch("reuse");
        let registry = Registry::at(&directory);
        let mine = registry.enrol();
        let reused = {
            let first = Registration::take(&registry, process(4242)).expect("the first run");
            first.run()
        };
        let stale = NOW - micros(WINDOW) - 1;
        assert!(
            registry
                .sweep(mine, NOW, WINDOW)
                .is_orphan(Stamp::new(reused, stale)),
            "the first run really has ended, which is what the second one then takes over"
        );
        let _second = Registration::take(&registry, process(4242)).expect("the second run");
        let sweep = registry.sweep(mine, NOW, WINDOW);
        assert!(
            !sweep.is_orphan(Stamp::new(reused, NOW)),
            "the run holding that number now is live, and its own artifacts are new"
        );
        assert!(
            !sweep.is_orphan(Stamp::new(reused, stale)),
            "the ended run's residue waits for the one that took its number, rather than \
             being taken while that one is going"
        );
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn no_stamp_is_wider_than_the_room_a_lane_is_told_to_leave() {
        // Both halves of the promise `WIDEST_STAMP` makes. The identity really is bounded to
        // nine digits, whatever the hash it comes from produced; and the widest name the
        // three parts can spell together is no wider than the figure a lane budgets for.
        for _ in 0..1_000 {
            assert!(
                fresh_host().get() <= 999_999_999,
                "a machine identity has to fit in the nine digits WIDEST_STAMP allows it"
            );
        }
        let widest =
            Stamp::new(Run::vouched(host(999_999_999), process(u32::MAX)), u64::MAX).to_string();
        assert_eq!(widest.len(), WIDEST_STAMP);
        assert_eq!(
            Stamp::read(&widest).map(|read| read.to_string()),
            Some(widest)
        );
    }

    /// A CI run written out by hand, for the stamps this file spells itself.
    fn ci(run_id: u64, attempt: u64) -> CiRun {
        CiRun::new(run_id, attempt).expect("a CI run within the grammar")
    }

    #[test]
    fn a_ci_stamp_reads_back_exactly_what_was_written() {
        let stamp = CiStamp::new(ci(37_616_803_489, 1), 1_791_374_400_000_000)
            .expect("a stamp within the grammar");
        assert_eq!(stamp.to_string(), "ci-37616803489-1-1791374400000000");
        assert_eq!(CiStamp::read(&stamp.to_string()), Some(stamp));
        assert_eq!(stamp.run().run_id(), 37_616_803_489);
        assert_eq!(stamp.run().attempt(), 1);
        assert_eq!(stamp.micros(), 1_791_374_400_000_000);
        // The narrowest value of every field, and a stamp written now.
        let narrowest = CiStamp::new(ci(1, 1), 1).expect("ones are within the grammar");
        assert_eq!(narrowest.to_string(), "ci-1-1-1");
        assert_eq!(CiStamp::read("ci-1-1-1"), Some(narrowest));
        let now = CiStamp::new(ci(9, 2), now_micros()).expect("the clock is within the grammar");
        assert_eq!(CiStamp::read(&now.to_string()), Some(now));
    }

    #[test]
    fn the_widest_ci_stamp_is_exactly_as_wide_as_the_widest_machine_stamp() {
        let widest = CiStamp::new(ci(99_999_999_999_999_999, 999), 9_999_999_999_999_999)
            .expect("the widest value of every field is admitted")
            .to_string();
        assert_eq!(widest.len(), 41);
        assert_eq!(widest.len(), WIDEST_CI_STAMP);
        assert_eq!(WIDEST_CI_STAMP, WIDEST_STAMP);
        assert_eq!(
            CiStamp::read(&widest).map(|read| read.to_string()),
            Some(widest)
        );
        assert!(LABEL_PREFIX.len() + WIDEST_CI_STAMP <= 50);
        assert!(
            ARTIFACT_PREFIX.len() + WIDEST_CI_STAMP <= 256,
            "an issue title GitHub accepts"
        );
        // One past each width is no CI run and no CI stamp.
        assert_eq!(CiRun::new(100_000_000_000_000_000, 1), None);
        assert_eq!(CiRun::new(1, 1_000), None);
        assert_eq!(CiRun::new(0, 1), None);
        assert_eq!(CiRun::new(1, 0), None);
        assert_eq!(CiStamp::new(ci(1, 1), 10_000_000_000_000_000), None);
        assert_eq!(CiStamp::new(ci(1, 1), 0), None);
    }

    #[test]
    fn a_suffix_outside_the_ci_grammar_is_not_read_as_a_ci_stamp() {
        for spelled in [
            // A zero field, and a leading zero, each of the three.
            "ci-0-1-17",
            "ci-1-0-17",
            "ci-1-1-0",
            "ci-01-1-17",
            "ci-1-01-17",
            "ci-1-1-017",
            // One digit wider than each field admits.
            "ci-100000000000000000-1-17",
            "ci-1-1000-17",
            "ci-1-1-10000000000000000",
            // A missing field, an empty one, and one too many.
            "ci-1-17",
            "ci-1--17",
            "ci--1-17",
            "ci-1-1-",
            "ci-",
            "ci",
            "ci-1-1-17-4",
            // A sign, a space, a letter, and the wrong case.
            "ci-+1-1-17",
            "ci--1-1-17",
            "ci-1-+1-17",
            "ci-1-1-+17",
            "ci- 1-1-17",
            "ci-1-1-17 ",
            "ci-1-a-17",
            "CI-1-1-17",
            "c-1-1-17",
            "",
        ] {
            assert_eq!(
                CiStamp::read(spelled),
                None,
                "{spelled:?} is not a CI stamp"
            );
        }
    }

    #[test]
    fn each_reader_refuses_the_other_forms_stamps() {
        // Every machine stamp — of a vouched run, an unvouched one, the widest — is no CI
        // stamp, and the machine reader reads each exactly as it always did.
        for machine in [
            Stamp::new(Run::vouched(host(41), process(2533)), NOW),
            Stamp::new(Run::unvouched(process(2533)), NOW),
            Stamp::new(Run::vouched(host(999_999_999), process(u32::MAX)), u64::MAX),
            Stamp::new(Run::vouched(host(1), process(1)), 0),
        ] {
            let spelled = machine.to_string();
            assert_eq!(
                CiStamp::read(&spelled),
                None,
                "{spelled:?} is a machine stamp"
            );
            assert_eq!(Stamp::read(&spelled), Some(machine));
        }
        // And every CI stamp is no machine stamp, so the lock sweep never acts on one.
        for spelled in [
            "ci-37616803489-1-1791374400000000",
            "ci-1-1-1",
            "ci-99999999999999999-999-9999999999999999",
        ] {
            assert!(CiStamp::read(spelled).is_some());
            assert_eq!(Stamp::read(spelled), None, "{spelled:?} is a CI stamp");
        }
        let directory = scratch("ci-is-no-orphan");
        let registry = Registry::at(&directory);
        let mine = registry.enrol();
        let sweep = registry.sweep(mine, NOW, WINDOW);
        assert!(!sweep.names_an_orphan(LABEL_PREFIX, "otg-live-ci-1-1-1"));
        assert!(!sweep.names_an_orphan(ARTIFACT_PREFIX, "onetaskgraph live cleanup ci-1-1-1"));
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_lane_in_github_actions_names_its_run_or_refuses_naming_the_variable() {
        let read = |actions: Option<&str>, id: Option<&str>, attempt: Option<&str>| {
            CiRun::from_environment(actions, id, attempt)
        };
        assert_eq!(
            read(Some("true"), Some("37616803489"), Some("2")),
            Ok(Some(ci(37_616_803_489, 2)))
        );
        // Outside GitHub Actions — not set, or anything but `true` — a machine stamp, whatever
        // the other two say.
        for actions in [None, Some("false"), Some(""), Some("TRUE"), Some("1")] {
            assert_eq!(read(actions, None, None), Ok(None), "{actions:?}");
            assert_eq!(read(actions, Some("0"), Some("x")), Ok(None), "{actions:?}");
        }
        for (id, attempt, named) in [
            (None, Some("1"), RUN_ID_VARIABLE),
            (Some(""), Some("1"), RUN_ID_VARIABLE),
            (Some("abc"), Some("1"), RUN_ID_VARIABLE),
            (Some("0"), Some("1"), RUN_ID_VARIABLE),
            (Some("012"), Some("1"), RUN_ID_VARIABLE),
            (Some("-12"), Some("1"), RUN_ID_VARIABLE),
            (Some("100000000000000000"), Some("1"), RUN_ID_VARIABLE),
            (Some("12"), None, RUN_ATTEMPT_VARIABLE),
            (Some("12"), Some("one"), RUN_ATTEMPT_VARIABLE),
            (Some("12"), Some("0"), RUN_ATTEMPT_VARIABLE),
            (Some("12"), Some("1000"), RUN_ATTEMPT_VARIABLE),
        ] {
            let refusal = read(Some("true"), id, attempt)
                .expect_err("a CI run that cannot name itself writes nothing");
            assert!(
                refusal.contains(named) && refusal.contains(GITHUB_ACTIONS_VARIABLE),
                "{id:?} {attempt:?}: {refusal}"
            );
        }
    }

    #[test]
    fn a_title_or_label_names_a_stamp_only_in_the_lanes_exact_spelling() {
        let design = "DESIGN: ";
        assert_eq!(
            titled_stamp("onetaskgraph live cleanup ci-1-1-1", design),
            Some("ci-1-1-1")
        );
        assert_eq!(
            titled_stamp("DESIGN: onetaskgraph live cleanup 4-5-6", design),
            Some("4-5-6")
        );
        for foreign in [
            "onetaskgraph live cleanup",
            "copy of onetaskgraph live cleanup ci-1-1-1",
            "DESIGN: DESIGN: onetaskgraph live cleanup ci-1-1-1",
            "copy of DESIGN: onetaskgraph live cleanup ci-1-1-1",
            "an ordinary feature",
        ] {
            assert_eq!(titled_stamp(foreign, design), None, "{foreign:?}");
        }
        assert_eq!(labelled_stamp("otg-live-ci-1-1-1"), Some("ci-1-1-1"));
        assert_eq!(labelled_stamp("bug"), None);
    }

    #[test]
    fn the_declared_window_is_the_one_a_lane_sweeps_on() {
        // `Sweep::of` and `Sweep::within` are what the lanes and their checks call, and both
        // go to the shared registry rather than carrying a registry or a window of their
        // own. The reasoning for the length is written on the constant, and a second figure
        // here would mean nothing a reader of that reasoning could act on.
        let run = Run::current();
        assert_eq!(run, Run::current());
        assert_eq!(Sweep::of(run, NOW).run(), run);
        assert_eq!(Sweep::of(run, NOW), Sweep::within(run, NOW, STALE_AFTER));
        assert_ne!(Sweep::of(run, NOW), Sweep::within(run, NOW, WINDOW));
        // Whatever else the shared registry holds, this run is live in it and its own
        // artifacts are its own — which is the property that holds at any window.
        assert!(!Sweep::of(run, NOW).is_orphan(Stamp::new(run, 0)));
    }
}
