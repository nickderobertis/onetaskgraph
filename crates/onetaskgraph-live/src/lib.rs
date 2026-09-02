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
    pub fn refuse(self) -> ! {
        panic!("{}", self.message())
    }
}

impl fmt::Display for Declined {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message())
    }
}

/// A live session in progress: the credential, and the seat that says it is the only one.
///
/// Held for as long as the lane is reaching its API. Dropping it releases the seat, so a
/// lane that ends — passed, failed or panicked — leaves nothing behind for the next run to
/// be declined by.
#[derive(Debug)]
pub struct Session {
    name: String,
    credential: String,
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
    pub fn open(name: &str, credential: String) -> Result<Self, Declined> {
        let directory = std::env::var_os(SEAT_DIRECTORY_VARIABLE)
            .map_or_else(std::env::temp_dir, PathBuf::from);
        Self::open_in(&directory, name, credential)
    }

    /// [`Session::open`], against a named directory rather than the default one.
    ///
    /// # Errors
    ///
    /// As [`Session::open`].
    pub fn open_in(directory: &Path, name: &str, credential: String) -> Result<Self, Declined> {
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
    pub fn credential(&self) -> &str {
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
                let stale = fs::metadata(&path)
                    .and_then(|metadata| metadata.modified())
                    .is_ok_and(|modified| {
                        SystemTime::now()
                            .duration_since(modified)
                            .is_ok_and(|age| age > SEAT_IS_STALE_AFTER)
                    });
                if !stale {
                    return Err(format!(
                        "another instance of it is already running against the same shared \
                         fixture, and two of them would delete each other's in-flight items — \
                         each sweeps residue by title before it starts, and that sweep \
                         recognises any run's artifacts. Its seat is {}; wait for that run to \
                         finish, or delete that file if no run holds it",
                        path.display()
                    ));
                }
                // An interrupted run's seat, older than any session lasts. Reclaiming it is
                // the self-healing half: `create_new` again rather than a plain create, so
                // two runs reclaiming at once still leave exactly one holder.
                let _ = fs::remove_file(&path);
                Self::create(&path).map_err(|problem| {
                    format!(
                        "its seat {} was left by an interrupted run and could not be reclaimed: \
                         {problem}. Delete that file, then re-run",
                        path.display()
                    )
                })
            }
            Err(problem) => Err(format!(
                "its seat {} could not be taken: {problem}. Make that directory writable, or \
                 point {SEAT_DIRECTORY_VARIABLE} at one that is",
                path.display()
            )),
        }
    }

    /// Create the seat file, failing when one is already there.
    fn create(path: &Path) -> std::io::Result<Self> {
        let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
        // Who holds it, for the reader of a refusal that names this file. Best effort: the
        // seat is the file's existence, not its contents, so a failed write must not hand
        // the seat to a second run.
        let _ = writeln!(file, "held by process {}", std::process::id());
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
    use super::*;

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
        let held = Session::open_in(&directory, "GitHub Projects", "live-token".to_owned())
            .expect("the first instance takes the seat");
        assert_eq!(held.credential(), "live-token");
        assert_eq!(held.name(), "GitHub Projects");
        assert!(held.seat_path().exists());
        assert!(
            held.seat_path()
                .ends_with("onetaskgraph-live-github-projects.seat"),
            "{}",
            held.seat_path().display()
        );

        let declined = Session::open_in(&directory, "GitHub Projects", "live-token".to_owned())
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
        Session::open_in(&directory, "GitHub Projects", "live-token".to_owned())
            .expect("the seat is free once the run that held it ended");
    }

    #[test]
    fn an_interrupted_runs_seat_is_reclaimed_rather_than_declining_for_ever() {
        let directory = scratch();
        let path = directory.join("onetaskgraph-live-linear.seat");
        fs::write(&path, "held by process 1\n").expect("a seat file is writable");
        let long_ago = SystemTime::now() - SEAT_IS_STALE_AFTER - Duration::from_secs(60);
        fs::File::open(&path)
            .expect("the seat file opens")
            .set_modified(long_ago)
            .expect("its modification time is settable");

        let reclaimed = Session::open_in(&directory, "Linear", "live-key".to_owned())
            .expect("a seat older than any session lasts is an interrupted run's");
        assert_eq!(reclaimed.seat_path(), path);
    }

    #[test]
    fn a_seat_directory_that_does_not_exist_declines_rather_than_running_unguarded() {
        let directory = scratch().join("absent");
        let declined = Session::open_in(&directory, "Linear", "live-key".to_owned())
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
        let session = Session::open("onetaskgraph default seat probe", "unused".to_owned())
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
    fn a_session_name_becomes_one_readable_file_name() {
        assert_eq!(slug("GitHub Projects"), "github-projects");
        assert_eq!(slug("Linear"), "linear");
        assert_eq!(slug("  //  "), "session");
        assert_eq!(slug("a  b"), "a-b");
    }
}
