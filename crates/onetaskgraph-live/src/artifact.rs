//! The stamp a live lane writes into every artifact it creates, and what may remove one.
//!
//! Both hosted lanes name every item, project, document and label they write
//! `<their own prefix><stamp>`. The prefix is each lane's business — GitHub's board items
//! and Linear's issues are not spelled the same way — but **the stamp, and what it
//! authorises, is one contract**, because both lanes have to answer the same question with
//! the same answer: *may this run delete that artifact?*
//!
//! # The rule
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
//! machine is never swept, because nothing here is evidence about a foreign process. An
//! interrupted hosted-check run leaves its artifacts for a person or a separate janitor to
//! clear. That direction is chosen: a leak is recoverable and a deleted live run is not.

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Run {
    host: Option<NonZeroU32>,
    process: u32,
}

impl Run {
    /// The run `process`, on the machine whose registry identity is `host`.
    #[must_use]
    pub fn vouched(host: NonZeroU32, process: u32) -> Self {
        Self {
            host: Some(host),
            process,
        }
    }

    /// The run `process`, which no registry can answer for.
    #[must_use]
    pub fn unvouched(process: u32) -> Self {
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
    pub fn process(self) -> u32 {
        self.process
    }

    /// The run `spelled` names, or `None` when it names no run.
    ///
    /// Digits only, on both halves, and both halves non-empty. A host of `0` is a run no
    /// registry vouches for rather than a run on machine zero.
    #[must_use]
    pub fn read(spelled: &str) -> Option<Self> {
        let (host, process) = spelled.split_once('-')?;
        Some(Self {
            host: NonZeroU32::new(number(host)?),
            process: number(process)?,
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

/// The `<host>-<process>-<microsecond timestamp>` every artifact name ends with.
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
    /// so the sweep passes over it rather than guessing.
    #[must_use]
    pub fn read(suffix: &str) -> Option<Self> {
        let (host, rest) = suffix.split_once('-')?;
        let (process, micros) = rest.split_once('-')?;
        Some(Self {
            run: Run {
                host: NonZeroU32::new(number(host)?),
                process: number(process)?,
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
        let process = std::process::id();
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
    pub fn registration_path(&self, process: u32) -> PathBuf {
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
    pub fn finished_runs(&self) -> Vec<u32> {
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
    pub fn take(registry: &Registry, process: u32) -> Option<Self> {
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
    finished: Vec<u32>,
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
        let stamp = Stamp::new(Run::vouched(host(41), 2533), NOW);
        assert_eq!(stamp.to_string(), format!("41-2533-{NOW}"));
        assert_eq!(Stamp::read(&stamp.to_string()), Some(stamp));
        assert_eq!(stamp.run(), Run::vouched(host(41), 2533));
        assert_eq!(stamp.run().host(), Some(host(41)));
        assert_eq!(stamp.run().process(), 2533);
        assert_eq!(stamp.micros(), NOW);
        assert_eq!(Run::read("41-2533"), Some(Run::vouched(host(41), 2533)));
        assert_eq!(Run::vouched(host(41), 2533).to_string(), "41-2533");
    }

    #[test]
    fn a_run_no_registry_vouches_for_reads_and_writes_as_one_rather_than_as_machine_zero() {
        let unvouched = Run::unvouched(2533);
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
        let stamp = Stamp::new(Run::unvouched(std::process::id()), now_micros());
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
        assert_eq!(registry.enrol(), Run::unvouched(std::process::id()));
        assert!(Registration::take(&registry, 4242).is_none());
        let sweep = registry.sweep(Run::vouched(host(7), std::process::id()), NOW, WINDOW);
        assert!(!sweep.is_orphan(Stamp::new(Run::vouched(host(7), 4242), 0)));
        // And a run nothing vouches for is nobody's orphan either, sweeping or swept.
        assert!(
            !Sweep::of(Run::unvouched(2533), NOW).is_orphan(Stamp::new(Run::unvouched(4242), 0))
        );
        let _ = fs::remove_file(&occupied);
    }

    #[test]
    fn a_run_is_over_exactly_when_its_registration_can_be_locked_again() {
        let directory = scratch("liveness");
        let registry = Registry::at(&directory);
        let live = Registration::take(&registry, 4242).expect("a registration this run can take");
        assert_eq!(
            live.run(),
            Run::vouched(registry.host().expect("an identity"), 4242)
        );
        assert!(
            !registry.finished_runs().contains(&4242),
            "a run holding its registration is not over"
        );
        // A second registration of the same run is refused: the lock is already held, and
        // that is true whichever process holds it.
        assert!(Registration::take(&registry, 4242).is_none());
        drop(live);
        assert!(
            registry.finished_runs().contains(&4242),
            "a registration nothing holds is a run the kernel says has ended"
        );
        // Whatever else is in the directory is not a registration and is never read as one.
        fs::write(directory.join("onetaskgraph-live-run-notanumber"), b"").expect("a stray file");
        fs::write(directory.join("something-else"), b"").expect("another stray file");
        assert_eq!(registry.finished_runs(), vec![4242]);
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn enrolling_twice_reports_one_run_rather_than_refusing_the_second() {
        let directory = scratch("enrol");
        let registry = Registry::at(&directory);
        let run = registry.enrol();
        assert_eq!(
            run,
            Run::vouched(registry.host().expect("an identity"), std::process::id())
        );
        assert_eq!(registry.enrol(), run);
        assert!(
            !registry.finished_runs().contains(&std::process::id()),
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
        let theirs = Registration::take(&registry, 4242).expect("a second live run");
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
            let registration = Registration::take(&registry, 4242).expect("a run that will end");
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
            Run::vouched(registry.host().expect("an identity"), 4243),
            NOW - micros(WINDOW) - 1
        )));
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_run_of_another_machine_is_never_swept_whatever_its_number() {
        let directory = scratch("foreign-host");
        let registry = Registry::at(&directory);
        let mine = registry.enrol();
        drop(Registration::take(&registry, 4242).expect("a run that has ended here"));
        let sweep = registry.sweep(mine, NOW, WINDOW);
        let stale = NOW - micros(WINDOW) - 1;
        let here = registry.host().expect("an identity");
        let elsewhere = host(here.get().wrapping_add(1).max(1));
        assert!(
            !sweep.is_orphan(Stamp::new(Run::vouched(elsewhere, 4242), stale)),
            "a process id of another machine was looked up in this machine's registry"
        );
        assert!(!sweep.is_orphan(Stamp::new(Run::unvouched(4242), stale)));
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn only_a_name_spelled_this_way_under_this_prefix_is_ever_an_orphan() {
        let directory = scratch("names");
        let registry = Registry::at(&directory);
        let mine = registry.enrol();
        let ended = {
            let registration = Registration::take(&registry, 4242).expect("a run that has ended");
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
        let widest = Stamp::new(Run::vouched(host(999_999_999), u32::MAX), u64::MAX).to_string();
        assert_eq!(widest.len(), WIDEST_STAMP);
        assert_eq!(
            Stamp::read(&widest).map(|read| read.to_string()),
            Some(widest)
        );
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
