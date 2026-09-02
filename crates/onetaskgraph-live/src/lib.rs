//! The one gate a live test passes through before it may reach a real API.
//!
//! Every live journey in this workspace — `crates/onetaskgraph-linear/tests/live.rs`,
//! `crates/onetaskgraph-github-projects/tests/live.rs` — is an ordinary test in an
//! ordinary `test` target, selected by the ordinary affected selection. What makes them
//! ordinary is that nothing about *running* them is special; what is left that has to be
//! decided once, rather than once per lane, is **whether a session may start at all**.
//! That decision is here, and it is the only way to hold a live credential:
//! [`Session::open`] takes the credential in and hands it back only once every
//! precondition has run. A precondition added here therefore governs every path by which
//! these tests reach a real API rather than some of them, which is the whole reason this
//! crate exists as a crate rather than as two copies of the same twenty lines.
//!
//! # The three answers, and why the third is not either of the others
//!
//! - **Run.** [`Session::open`] returned, the seat is held, and the lane may reach its API.
//! - **Skip.** A credential or a nomination the lane needs was not given, and nobody said
//!   one was expected. [`missing`] reports it with its reason and the lane returns without
//!   asserting anything. This is a contributor with no keys, and it is a pull request from
//!   a fork, which the host gives no secrets.
//! - **Declined.** The lane could have run, and a precondition refused it. It tested
//!   nothing, so it is *not* a pass — [`Declined::refuse`] panics, which fails the test,
//!   the target and the required check that runs it. Its message says the session did not
//!   run and why, so a reader is not sent debugging a defect that is really a refusal.
//!
//! `ONETASKGRAPH_LIVE_REQUIRED=1` turns a skip into a failure, which is what keeps the
//! required lane from passing green merely because a credential went missing where one was
//! expected. See [`required`] and [`missing`].
//!
//! # The precondition this crate ships
//!
//! **Exclusivity**: a test that reads and writes a shared external fixture must not run
//! concurrently with another instance of itself. Both live journeys sweep residue by
//! title before they start — that is what makes them self-healing after an interrupted run
//! — and a sweep that recognises *any* run's artifacts will delete a concurrent run's
//! in-flight items. So concurrency is a correctness problem here rather than a cost one,
//! and [`Session::open`] holds a seat for the session's name for as long as the session
//! lasts. A second instance is declined rather than allowed to race.

#![deny(missing_docs)]

use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write as _};
use std::path::{Path, PathBuf};
use std::process;
use std::time::{Duration, SystemTime};

/// The directory the session seats live in, when the default is not wanted.
///
/// The default is the platform temporary directory. A caller sets this to prove what a
/// contended seat does without touching a directory anything else on the machine shares —
/// `scripts/check-live-decline.sh` does exactly that.
pub const SEAT_DIRECTORY_VARIABLE: &str = "ONETASKGRAPH_LIVE_SEAT_DIR";

/// The variable that says a live session is expected here.
pub const REQUIRED_VARIABLE: &str = "ONETASKGRAPH_LIVE_REQUIRED";

/// How long a seat file may go untouched before it is treated as an interrupted run's.
///
/// A live session is minutes at the outside, so an hour cannot be a session still going.
/// Without this a process killed between taking its seat and releasing it would decline
/// every later run on that machine for ever, which is a worse failure than the race the
/// seat exists to prevent — CI gets a fresh runner each time and would never notice, and
/// the contributor whose run was interrupted would.
const SEAT_IS_STALE_AFTER: Duration = Duration::from_secs(60 * 60);

/// Whether a live session is expected here, from `ONETASKGRAPH_LIVE_REQUIRED`.
///
/// `1` demands one; `0`, the empty string and an absent variable all leave the lane free
/// to skip. Anything else is a misconfiguration rather than a quiet not-required: a value
/// nobody can read is the shape in which "the credentialed lane silently stopped running"
/// arrives.
///
/// # Errors
///
/// When the value is neither `1`, `0`, empty nor absent.
// llmlint: ignore[invalid_states_unrepresentable] The answer really is two-valued and every `bool` is one of the two, so there is no unrepresentable state a `Demand` enum would remove. What this function exists to make unrepresentable is the *third* reading — "a value nobody can parse quietly means not-required" — and it does that by returning `Err`, not by the shape of its success. Its one caller passes the answer straight to `missing` below, which takes no other boolean.
pub fn required(raw: Option<&str>) -> Result<bool, String> {
    match raw.map(str::trim) {
        None | Some("") | Some("0") => Ok(false),
        Some("1") => Ok(true),
        Some(other) => Err(format!(
            "{REQUIRED_VARIABLE} must be 1, 0 or unset, not {other:?}"
        )),
    }
}

/// What a lane does about an input it was not given: skip with the reason, or fail.
///
/// `Ok(reason)` is the skip. `Err(reason)` is the same reason with what demanded the
/// input, for the run where one was expected — a fork pull request receives no secrets and
/// skips, and a trusted run that reaches the lane without one is a misconfiguration.
///
/// # Errors
///
/// When `required` is true, so the absent input is a failure rather than a skip.
// llmlint: ignore[invalid_states_unrepresentable, live_tier_compiles_and_requires_credential] Its `required` is the answer [`required`] above already validated and the only boolean here, so there is no pair a caller could transpose and no third state to represent. And its `Ok` — the skip — is reached only when `required` is false, which is the run where no credential was ever expected: a contributor with no keys, and a fork pull request, to which GitHub supplies no secrets at all. The run where one IS expected sets `ONETASKGRAPH_LIVE_REQUIRED=1`, which `.github/workflows/ci.yml` sets on every run but that one, so this same call returns the `Err` the credential rule asks for. Failing unconditionally would add a demand nowhere; it would only fail every outside contribution for a secret its author cannot have.
pub fn missing(required: bool, session: &str, reason: impl Into<String>) -> Result<String, String> {
    let reason = reason.into();
    if required {
        return Err(format!(
            "{reason}, and {REQUIRED_VARIABLE}=1 requires the {session} live session to run"
        ));
    }
    Ok(reason)
}

/// A session that could have run and did not, and the reason no test result covers it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declined {
    session: String,
    reason: String,
}

impl Declined {
    /// The line a reader sees, which says the tests did not run before it says why.
    ///
    /// The wording leads with the outcome rather than with the cause on purpose: a quota
    /// that was short, or a seat another run holds, is not a defect in the code under
    /// test, and somebody scanning a red check has to be able to tell those apart in the
    /// first line rather than in the third.
    #[must_use]
    pub fn message(&self) -> String {
        format!(
            "the {} live session DID NOT RUN and tested nothing: {}. This is not a test \
             failure in the code under test.",
            self.session, self.reason
        )
    }

    /// End this run as what it is: something that did not happen, and so did not pass.
    ///
    /// A panic, because a panic is what the required check reads as a failure. A run that
    /// never happened must conclude neither success nor anything branch protection accepts
    /// in place of success — reaching the default branch on a signal that never ran is how
    /// a query GitHub refuses outright got in.
    ///
    /// # Panics
    ///
    /// Always. That is what it is for.
    // llmlint: ignore[no_panics_on_recoverable_errors] There is nowhere to propagate to: the caller is a `#[test]`, and a test that returned early instead would be reported as passed — a run that never happened concluding as success is the exact hole this arrangement exists to close, since branch protection accepts nothing in place of success either. Failing is what a cargo test harness has instead of a third conclusion. `Session::open` returns the `Result` precisely so a caller with something better to do can do it; this is the answer for the one that has not, and its message leads with the tests not having run, so the failure is not read as a defect in the code under test.
    pub fn refuse(self) -> ! {
        panic!("{}", self.message())
    }
}

impl fmt::Display for Declined {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message())
    }
}

/// A credential a session can actually be opened with.
///
/// A type rather than a `String`, because "the session is open" has to mean the lane may
/// reach its API — and a blank credential cannot. That is the common case rather than a
/// pathological one: a host expands a secret it does not have to the empty string, so the
/// absent credential and the blank one arrive here spelled the same way, and a `String`
/// field would let a session hold one and hand it to a lane that then reads GitHub's
/// refusal as a defect in the code under test. [`Credential::new`] is the only way to make
/// one, so a session that exists has a usable credential.
#[derive(Clone, PartialEq, Eq)]
pub struct Credential(String);

impl Credential {
    /// The credential `raw` carries, or `None` when it carries none.
    ///
    /// Blank is none: a variable set to spaces is a variable nobody set. The value is kept
    /// exactly as it was given rather than trimmed — what an API accepts is the API's
    /// business, and a credential this crate had quietly edited would be a worse failure
    /// than one it refused.
    #[must_use]
    pub fn new(raw: impl Into<String>) -> Option<Self> {
        let raw = raw.into();
        (!raw.trim().is_empty()).then_some(Self(raw))
    }

    /// The credential itself, for the call that sends it.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Credential {
    /// Redacted, and its length instead: this type is held by [`Session`], which derives
    /// `Debug`, and a panic message that printed a live credential would put it in a log.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Credential(<redacted, {} bytes>)", self.0.len())
    }
}

/// A live session in progress: the credential, and the seat that says it is the only one.
///
/// Held for as long as the lane is reaching its API. Dropping it releases the seat, so a
/// lane that ends — passed, failed or panicked — leaves nothing behind for the next run to
/// be declined by.
#[derive(Debug)]
pub struct Session {
    // llmlint: ignore[invalid_states_unrepresentable] The name is not input: each lane names its own session with a `const` of its own, there are two in this workspace, and neither reaches a user or a file a user writes. The state a newtype would remove — two names whose seats slug alike — is a spurious *decline*, which fails loudly with both names in the message rather than letting two runs race; and `slug` is total, with its empty and punctuation-only cases pinned by this crate's own tests. A validated type here would be ceremony over two constants.
    name: String,
    credential: Credential,
    seat: Seat,
}

impl Session {
    /// Open a session, or say why it did not run.
    ///
    /// The credential goes in and comes back out of [`Session::credential`], so a lane
    /// cannot reach its API without having passed every precondition first.
    ///
    /// # Errors
    ///
    /// When a precondition refuses. Today that is exclusivity alone: another instance of
    /// this session is already running against the same shared fixture.
    pub fn open(name: &str, credential: Credential) -> Result<Self, Declined> {
        let directory = std::env::var_os(SEAT_DIRECTORY_VARIABLE)
            .map_or_else(std::env::temp_dir, PathBuf::from);
        Self::open_in(&directory, name, credential)
    }

    /// [`Session::open`], against a named directory rather than the default one.
    ///
    /// # Errors
    ///
    /// As [`Session::open`].
    pub fn open_in(directory: &Path, name: &str, credential: Credential) -> Result<Self, Declined> {
        let seat = Seat::take(directory, name).map_err(|reason| Declined {
            session: name.to_owned(),
            reason,
        })?;
        Ok(Self {
            name: name.to_owned(),
            credential,
            seat,
        })
    }

    /// The credential this session was opened with.
    #[must_use]
    pub fn credential(&self) -> &Credential {
        &self.credential
    }

    /// What this session is called, as its seat and its refusals name it.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Where this session's seat is held.
    #[must_use]
    pub fn seat_path(&self) -> &Path {
        &self.seat.path
    }
}

/// One session's place, held as a file nothing else may create while it exists.
#[derive(Debug)]
struct Seat {
    path: PathBuf,
}

impl Seat {
    /// Take the seat for `name`, or say why this run may not have it.
    fn take(directory: &Path, name: &str) -> Result<Self, String> {
        let path = directory.join(format!("onetaskgraph-live-{}.seat", slug(name)));
        match Self::create(&path) {
            Ok(seat) => Ok(seat),
            Err(held) if held.kind() == ErrorKind::AlreadyExists => {
                if !Self::is_stale(&path) {
                    return Err(Self::already_running(&path));
                }
                Self::reclaim(&path)
            }
            Err(problem) => Err(format!(
                "its seat {} could not be taken: {problem}. Make that directory writable, or \
                 point {SEAT_DIRECTORY_VARIABLE} at one that is",
                path.display()
            )),
        }
    }

    /// The refusal a run gets when a live session still holds this seat.
    fn already_running(path: &Path) -> String {
        format!(
            "another instance of it is already running against the same shared fixture, and \
             two of them would delete each other's in-flight items — each sweeps residue by \
             title before it starts, and that sweep recognises any run's artifacts. Its seat \
             is {}; wait for that run to finish, or delete that file if no run holds it",
            path.display()
        )
    }

    /// Whether the file at `path` was last touched longer ago than any session lasts.
    ///
    /// A file that is not there is not stale — it is absent, which is a different answer and
    /// belongs to whoever asked.
    fn is_stale(path: &Path) -> bool {
        fs::metadata(path)
            .and_then(|metadata| metadata.modified())
            .is_ok_and(|modified| {
                SystemTime::now()
                    .duration_since(modified)
                    .is_ok_and(|age| age > SEAT_IS_STALE_AFTER)
            })
    }

    /// Take over an interrupted run's seat, with at most one run reclaiming at a time.
    ///
    /// The exclusion is the point rather than a refinement. Reclaiming is delete-then-create,
    /// and two runs that had each read the same stale seat would otherwise interleave into
    /// each deleting the *other's* freshly created seat and each believing it held one —
    /// which is two concurrent sessions against one shared external fixture, the very thing
    /// the seat exists to prevent. So a reclaim is done under a claim file taken with
    /// `create_new`, and the staleness is read again inside it: a run that arrives with an
    /// out-of-date reading finds the seat fresh under the claim and is declined instead of
    /// deleting a live run's seat.
    ///
    /// llmlint: ignore-block[changed_behavior_has_e2e] Reclaiming is the branch that lets the session open, and a session that opens is a journey that reaches the real API on its next line — so driving this one through either live journey means spending a live session against a third party to prove a decision about a file's age, on every run of the required check, and cannot be arranged at all without a credential. The half that CAN be driven through the real journey binary is, in `scripts/check-live-decline.sh`: a seat a live run still holds declines, and the run that was declined leaves it where it found it. What is left is the age comparison, the claim and the re-creation, which this crate's own tests drive against a real seat file whose modification time is really in the past and against real concurrent reclaimers.
    fn reclaim(path: &Path) -> Result<Self, String> {
        let claim = path.with_extension("reclaim");
        if Self::hold(&claim).is_err() {
            return Err(format!(
                "another run is reclaiming the seat {} an interrupted run left, so exactly \
                 one of the two of you gets it. Re-run once that one has finished",
                path.display()
            ));
        }
        let reclaimed = if Self::is_stale(path) {
            let _ = fs::remove_file(path);
            Self::create(path).map_err(|problem| {
                format!(
                    "its seat {} was left by an interrupted run and this run did not get it: \
                     {problem}. Delete that file, then re-run",
                    path.display()
                )
            })
        } else {
            // Somebody reclaimed it while this run was deciding, and what is there now is
            // theirs. Reading it again under the claim is what stops this run deleting it.
            Err(Self::already_running(path))
        };
        let _ = fs::remove_file(&claim);
        reclaimed
    }
    // llmlint: ignore-end[changed_behavior_has_e2e]

    /// Take the claim file, or fail because another run holds it.
    ///
    /// A claim a killed run left behind goes stale on the same clock the seat does, so a
    /// reclaim interrupted between its two halves does not decline every later run for ever.
    fn hold(claim: &Path) -> std::io::Result<()> {
        // Not a `Seat`: that type releases its file on drop, and this one has to outlive the
        // reclaim it guards. `reclaim` removes it on every path out of itself instead.
        let create = || {
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(claim)
                .map(drop)
        };
        match create() {
            Ok(()) => Ok(()),
            Err(taken) if taken.kind() == ErrorKind::AlreadyExists && Self::is_stale(claim) => {
                let _ = fs::remove_file(claim);
                create()
            }
            Err(problem) => Err(problem),
        }
    }

    /// Create the seat file, failing when one is already there.
    fn create(path: &Path) -> std::io::Result<Self> {
        let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
        // Who holds it, for the reader of a refusal that names this file. Best effort: the
        // seat is the file's existence, not its contents, so a failed write must not hand
        // the seat to a second run.
        let _ = writeln!(file, "held by process {}", process::id());
        Ok(Self {
            path: path.to_owned(),
        })
    }
}

impl Drop for Seat {
    fn drop(&mut self) {
        // Best effort by construction: a seat nobody could remove is reclaimed by the next
        // run as an interrupted one's, which is the case this ignores in favour of.
        let _ = fs::remove_file(&self.path);
    }
}

/// A session's name as a filename: lowercase, with every run of anything else one hyphen.
fn slug(name: &str) -> String {
    let mut slug = String::with_capacity(name.len());
    for character in name.chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let trimmed = slug.trim_matches('-');
    if trimmed.is_empty() {
        "session".to_owned()
    } else {
        trimmed.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier};

    use super::*;

    /// Plant a seat file that was last touched longer ago than any session lasts.
    fn interrupted_runs_file(path: &Path) {
        fs::write(path, "held by an interrupted run\n").expect("the file is writable");
        let long_ago = SystemTime::now() - SEAT_IS_STALE_AFTER - Duration::from_secs(60);
        fs::File::open(path)
            .expect("the file opens")
            .set_modified(long_ago)
            .expect("its modification time is settable");
    }

    fn credential(raw: &str) -> Credential {
        Credential::new(raw).expect("this test's placeholder credential is not blank")
    }

    fn scratch() -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "onetaskgraph-live-seats-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        fs::create_dir_all(&directory).expect("the scratch seat directory is creatable");
        directory
    }

    #[test]
    fn the_demand_reads_as_one_of_three_things_and_never_as_a_quiet_no() {
        assert_eq!(required(None), Ok(false));
        assert_eq!(required(Some("")), Ok(false));
        assert_eq!(required(Some(" 0 ")), Ok(false));
        assert_eq!(required(Some("1")), Ok(true));
        for unusable in ["yes", "true", "2", "on"] {
            let error = required(Some(unusable))
                .expect_err("an unreadable demand must fail rather than mean not-required");
            assert!(
                error.contains(REQUIRED_VARIABLE) && error.contains(unusable),
                "{error}"
            );
        }
    }

    #[test]
    fn an_absent_input_skips_unless_one_was_expected() {
        assert_eq!(
            missing(false, "Linear", "LINEAR_API_KEY is not set"),
            Ok("LINEAR_API_KEY is not set".to_owned())
        );
        let error = missing(true, "Linear", "LINEAR_API_KEY is not set")
            .expect_err("a demanded session must fail rather than skip");
        assert!(
            error.contains("LINEAR_API_KEY")
                && error.contains(REQUIRED_VARIABLE)
                && error.contains("Linear"),
            "{error}"
        );
    }

    #[test]
    fn one_session_holds_its_seat_and_a_second_instance_is_declined() {
        let directory = scratch();
        let held = Session::open_in(&directory, "GitHub Projects", credential("live-token"))
            .expect("the first instance takes the seat");
        assert_eq!(held.credential().expose(), "live-token");
        assert_eq!(held.name(), "GitHub Projects");
        assert!(held.seat_path().exists());
        assert!(
            held.seat_path()
                .ends_with("onetaskgraph-live-github-projects.seat"),
            "{}",
            held.seat_path().display()
        );

        let declined = Session::open_in(&directory, "GitHub Projects", credential("live-token"))
            .expect_err("a second instance against the same fixture must be declined");
        let message = declined.message();
        assert!(message.contains("DID NOT RUN"), "{message}");
        assert!(message.contains("GitHub Projects"), "{message}");
        assert!(message.contains("already running"), "{message}");
        assert!(message.contains("delete that file"), "{message}");
        assert!(
            message.contains("not a test failure in the code under test"),
            "{message}"
        );
        assert_eq!(message, declined.to_string());

        let seat = held.seat_path().to_owned();
        drop(held);
        assert!(!seat.exists(), "a finished session releases its seat");
        Session::open_in(&directory, "GitHub Projects", credential("live-token"))
            .expect("the seat is free once the run that held it ended");
    }

    #[test]
    fn an_interrupted_runs_seat_is_reclaimed_rather_than_declining_for_ever() {
        let directory = scratch();
        let path = directory.join("onetaskgraph-live-linear.seat");
        interrupted_runs_file(&path);

        let reclaimed = Session::open_in(&directory, "Linear", credential("live-key"))
            .expect("a seat older than any session lasts is an interrupted run's");
        assert_eq!(reclaimed.seat_path(), path);
    }

    #[test]
    fn a_run_already_reclaiming_the_seat_declines_the_next_one_rather_than_racing_it() {
        // The interleaving the thread test can only sometimes produce, produced on purpose:
        // one run is between reading the seat as stale and writing its own, and a second run
        // reads the same stale seat. Without the claim both delete the other's and both hold.
        let directory = scratch();
        let path = directory.join("onetaskgraph-live-github-projects.seat");
        interrupted_runs_file(&path);
        fs::write(
            path.with_extension("reclaim"),
            "held by the run reclaiming it\n",
        )
        .expect("the claim is writable");

        let declined = Session::open_in(&directory, "GitHub Projects", credential("live-token"))
            .expect_err("a second reclaimer must be declined rather than delete the first's seat");
        let message = declined.message();
        assert!(message.contains("DID NOT RUN"), "{message}");
        assert!(message.contains("is reclaiming"), "{message}");
    }

    #[test]
    fn a_claim_an_interrupted_reclaim_left_does_not_decline_every_later_run_for_ever() {
        let directory = scratch();
        let path = directory.join("onetaskgraph-live-linear.seat");
        let claim = path.with_extension("reclaim");
        interrupted_runs_file(&path);
        interrupted_runs_file(&claim);

        let reclaimed = Session::open_in(&directory, "Linear", credential("live-key"))
            .expect("a claim older than any session lasts is an interrupted reclaim's");
        assert_eq!(reclaimed.seat_path(), path);
        assert!(
            !claim.exists(),
            "a finished reclaim leaves no claim for the next run to wait behind"
        );
    }

    #[test]
    fn two_runs_reclaiming_one_interrupted_seat_leave_exactly_one_holder() {
        // The race a delete-then-create reclaim loses: both runs find the seat stale, each
        // removes what the other just wrote, and both believe they hold it — which is two
        // concurrent sessions against one shared external fixture, deleting each other's
        // in-flight items. Real threads against a real seat file, because the interleaving
        // is the whole of what is under test.
        let directory = scratch();
        let path = directory.join("onetaskgraph-live-github-projects.seat");
        interrupted_runs_file(&path);

        let start = Arc::new(Barrier::new(8));
        let holders: Vec<_> = (0..8)
            .map(|_| {
                let directory = directory.clone();
                let start = Arc::clone(&start);
                std::thread::spawn(move || {
                    start.wait();
                    Session::open_in(&directory, "GitHub Projects", credential("live-token")).ok()
                })
            })
            .collect();
        let held: Vec<_> = holders
            .into_iter()
            .filter_map(|thread| thread.join().expect("no reclaiming thread panicked"))
            .collect();
        assert_eq!(
            held.len(),
            1,
            "{} runs reclaimed one interrupted seat at once, so that many sessions would \
             have written to the same shared fixture",
            held.len()
        );
        assert!(
            held[0].seat_path().exists(),
            "the winner really holds a seat"
        );
    }

    #[test]
    fn a_seat_directory_that_does_not_exist_declines_rather_than_running_unguarded() {
        let directory = scratch().join("absent");
        let declined = Session::open_in(&directory, "Linear", credential("live-key"))
            .expect_err("a seat that cannot be taken must decline rather than run");
        let message = declined.message();
        assert!(message.contains("DID NOT RUN"), "{message}");
        assert!(message.contains(SEAT_DIRECTORY_VARIABLE), "{message}");
    }

    #[test]
    fn the_default_seat_directory_is_the_platform_temporary_one_unless_named() {
        // SAFETY-of-the-suite note: this is the one test that reads the process
        // environment, and it only reads it.
        let named = std::env::var_os(SEAT_DIRECTORY_VARIABLE);
        let session = Session::open("onetaskgraph default seat probe", credential("unused"))
            .expect("nothing else holds this probe's seat");
        let expected = named.map_or_else(std::env::temp_dir, PathBuf::from);
        assert_eq!(session.seat_path().parent(), Some(expected.as_path()));
    }

    #[test]
    #[should_panic(expected = "DID NOT RUN")]
    fn a_declined_session_ends_the_run_rather_than_returning_to_it() {
        Declined {
            session: "GitHub Projects".to_owned(),
            reason: "the account cannot afford it".to_owned(),
        }
        .refuse()
    }

    #[test]
    fn a_credential_a_session_could_not_use_cannot_be_made_at_all() {
        for blank in ["", " ", "\t\n"] {
            assert!(
                Credential::new(blank).is_none(),
                "a host expands a secret it does not have to {blank:?}, which no session may open on"
            );
        }
        let credential = credential("  ghp_padded  ");
        assert_eq!(
            credential.expose(),
            "  ghp_padded  ",
            "the value is handed back exactly as it was given rather than edited"
        );
        assert_eq!(
            format!("{credential:?}"),
            "Credential(<redacted, 14 bytes>)",
            "a debug rendering must not put a live credential in a log"
        );
        assert!(
            !format!(
                "{:?}",
                Session::open_in(&scratch(), "Redaction", credential).unwrap()
            )
            .contains("ghp_padded"),
            "nor may the session that holds it"
        );
    }

    #[test]
    fn a_session_name_becomes_one_readable_file_name() {
        assert_eq!(slug("GitHub Projects"), "github-projects");
        assert_eq!(slug("Linear"), "linear");
        assert_eq!(slug("  //  "), "session");
        assert_eq!(slug("a  b"), "a-b");
    }
}
