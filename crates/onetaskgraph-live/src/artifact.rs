//! The stamp a live lane writes into every artifact it creates, and when one is an orphan.
//!
//! Both hosted lanes name every item, project, document and label they write
//! `<their own prefix><process id>-<microsecond timestamp>`. The prefix is each lane's
//! business — GitHub's board items and Linear's issues are not spelled the same way — but
//! **what comes after it, and what it means, is one contract**, because both lanes have to
//! answer the same question with the same answer: *may this run delete that artifact?*
//!
//! # The rule
//!
//! An artifact is this run's when it carries this run's process id, and it is an **orphan**
//! when it carries somebody else's and its own stamp is older than [`STALE_AFTER`].
//! Anything else belongs to a run that may still be alive and is left exactly where it is.
//!
//! Both halves are load-bearing and they protect different things:
//!
//! - **The process id protects the run doing the sweeping, unconditionally.** A session's
//!   own artifacts are never orphans to itself, however long that session has been going —
//!   so a run that hangs, or that a hosted API's secondary limiter parks for fifty minutes,
//!   cannot sweep its own work out from under itself when it comes back. This is what makes
//!   safety independent of [`STALE_AFTER`]'s length: whatever the window is set to, a live
//!   run's own artifacts survive its own sweep.
//! - **The stamp protects every other run,** to the extent a file's own age can. A run in
//!   another process — on this machine or on another runner — announces nothing this one
//!   can read, so how recently its artifacts were written is the only evidence there is.
//!
//! Getting the window wrong is therefore not symmetric. Too long and an interrupted run's
//! residue sits on a real board for longer before the next run clears it; too short and a
//! slow *foreign* session's artifacts are taken while it is still using them. The window is
//! chosen for the second, and the first is what it costs.
//!
//! What this module deliberately does **not** do is decide anything from a lock, a seat or
//! any other state outside the artifact itself. A removal rests on that artifact's own
//! stamp and on the identity of the run asking, and on nothing else — which is what lets
//! two sessions of one lane run side by side.

use std::fmt;
use std::time::Duration;

/// How long after its own stamp an artifact stops being a live run's and becomes an orphan.
///
/// **Six hours**, and the reasoning matters more than the number, because the next reader
/// may well want to change it:
///
/// - It has to be **comfortably longer than a session can last**, since a foreign run's
///   artifacts are protected by nothing else. A journey is minutes at the outside, but a
///   hosted API's secondary rate limiter refuses a burst for up to an hour at a time and a
///   run can meet more than one of those, so an hour is not comfortable and six is.
/// - It does **not** bound how long a live run may take without losing its own work: that
///   is the process id's job, and it holds however long the run goes on. See the module
///   documentation.
/// - What it does bound is how long an interrupted run's residue lives on a real board:
///   until the next run of that lane at least six hours later. Residue is visible and
///   cheap; another run's items disappearing mid-journey is neither.
///
/// So a longer window delays a cleanup and a shorter one risks a live run's work. Change it
/// with that trade in mind rather than to make a check faster — a check chooses its own
/// window, exactly so it never has to wait out this one.
pub const STALE_AFTER: Duration = Duration::from_secs(6 * 60 * 60);

/// The `<process id>-<microsecond timestamp>` every artifact name ends with.
///
/// One type rather than two lanes' worth of `split_once('-')`, because writing a stamp and
/// reading one back have to agree: a lane that formatted what this cannot parse would write
/// artifacts no sweep could ever recognise, on somebody's real board.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    process_id: u32,
    micros: i64,
}

impl Stamp {
    /// The stamp a run writing at `micros` puts on its artifacts.
    #[must_use]
    pub fn new(process_id: u32, micros: i64) -> Self {
        Self { process_id, micros }
    }

    /// Which process wrote the artifact carrying this stamp.
    #[must_use]
    pub fn process_id(self) -> u32 {
        self.process_id
    }

    /// When it was written, in microseconds since the epoch.
    #[must_use]
    pub fn micros(self) -> i64 {
        self.micros
    }

    /// The stamp `suffix` spells, or `None` when it spells no stamp at all.
    ///
    /// Digits only, on both halves, and both halves non-empty: `12-34` is a stamp and
    /// `12-34-56`, `-34`, `12-` and `abc-34` are not. A name whose suffix is not a stamp is
    /// not this lane's artifact, so the sweep passes over it rather than guessing.
    #[must_use]
    pub fn read(suffix: &str) -> Option<Self> {
        let (process_id, micros) = suffix.split_once('-')?;
        if [process_id, micros]
            .iter()
            .any(|part| part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()))
        {
            return None;
        }
        Some(Self {
            process_id: process_id.parse().ok()?,
            micros: micros.parse().ok()?,
        })
    }
}

impl fmt::Display for Stamp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}-{}", self.process_id, self.micros)
    }
}

/// One run's decision about which of the artifacts it can see are orphans.
///
/// Made once at the point the sweep starts and then asked of each name, so every artifact
/// in one sweep is judged against the same instant rather than against a clock that moves
/// while the sweep pages through a board.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sweep {
    run: u32,
    now_micros: i64,
    window: Duration,
}

impl Sweep {
    /// The sweep `run` makes at `now_micros`, over the window this contract declares.
    ///
    /// This is what a lane uses. [`Sweep::within`] is for a check that would otherwise have
    /// to wait out [`STALE_AFTER`] to observe anything.
    #[must_use]
    pub fn of(run: u32, now_micros: i64) -> Self {
        Self::within(run, now_micros, STALE_AFTER)
    }

    /// The same decision over a window the caller chose.
    ///
    /// A check drives this so that proving what a sweep does needs no session that runs for
    /// six hours — and so that what it proves is *independent* of the window, which is the
    /// property [`STALE_AFTER`] is chosen under rather than relied upon.
    #[must_use]
    pub fn within(run: u32, now_micros: i64, window: Duration) -> Self {
        Self {
            run,
            now_micros,
            window,
        }
    }

    /// The run this sweep is being made by, whose artifacts it never touches.
    #[must_use]
    pub fn run(self) -> u32 {
        self.run
    }

    /// Whether the artifact carrying `stamp` is an orphan this run may remove.
    ///
    /// An artifact stamped in the future is not an orphan: two machines' clocks disagree,
    /// and the direction to be wrong in is leaving somebody's work alone.
    #[must_use]
    pub fn is_orphan(self, stamp: Stamp) -> bool {
        if stamp.process_id == self.run {
            return false;
        }
        u128::try_from(self.now_micros.saturating_sub(stamp.micros))
            .is_ok_and(|age| age > self.window.as_micros())
    }

    /// Whether `name` is an orphan artifact written under `prefix`.
    ///
    /// The whole decision for a lane whose names are `prefix` then a stamp: anything that
    /// is not spelled that way belongs to somebody else entirely and is never touched.
    #[must_use]
    pub fn names_an_orphan(self, prefix: &str, name: &str) -> bool {
        name.strip_prefix(prefix)
            .and_then(Stamp::read)
            .is_some_and(|stamp| self.is_orphan(stamp))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A window a test can drive either side of without waiting for anything.
    const WINDOW: Duration = Duration::from_secs(60);

    const NOW: i64 = 1_787_816_134_627_361;

    fn micros(window: Duration) -> i64 {
        i64::try_from(window.as_micros()).expect("a test window fits in the stamp's own type")
    }

    #[test]
    fn a_stamp_reads_back_exactly_what_was_written() {
        let stamp = Stamp::new(2533, NOW);
        assert_eq!(stamp.to_string(), format!("2533-{NOW}"));
        assert_eq!(Stamp::read(&stamp.to_string()), Some(stamp));
        assert_eq!(stamp.process_id(), 2533);
        assert_eq!(stamp.micros(), NOW);
    }

    #[test]
    fn a_suffix_that_is_not_a_stamp_is_not_read_as_one() {
        for spelled in [
            "",
            "2533",
            "-2533",
            "2533-",
            "abc-17",
            "2533-abc",
            "2533-17-19",
            "2533 -17",
            "+2533-17",
            "2533--17",
            // Wider than the halves they are parsed into: a name nothing here could have
            // written, and reading it as a stamp would be reading a number that overflowed.
            "99999999999-17",
            "2533-99999999999999999999",
        ] {
            assert_eq!(Stamp::read(spelled), None, "{spelled:?} is not a stamp");
        }
    }

    #[test]
    fn a_sweep_takes_another_runs_stale_artifact_and_leaves_its_fresh_one() {
        let sweep = Sweep::within(2533, NOW, WINDOW);
        assert!(sweep.is_orphan(Stamp::new(9999, NOW - micros(WINDOW) - 1)));
        assert!(!sweep.is_orphan(Stamp::new(9999, NOW - micros(WINDOW))));
        assert!(!sweep.is_orphan(Stamp::new(9999, NOW)));
        assert_eq!(sweep.run(), 2533);
    }

    #[test]
    fn a_sweep_never_takes_its_own_runs_artifact_however_old_it_is() {
        // The live-session case: a run parked by a secondary rate limiter for longer than
        // the window comes back to find its own fixture still there. Age alone cannot say
        // this, which is why the run is part of the decision at all.
        let sweep = Sweep::within(2533, NOW, WINDOW);
        for age in [0, micros(WINDOW), micros(WINDOW) * 1_000] {
            assert!(
                !sweep.is_orphan(Stamp::new(2533, NOW - age)),
                "a run's own artifact {age} microseconds old is not an orphan to it"
            );
        }
    }

    #[test]
    fn an_artifact_stamped_in_the_future_is_left_alone() {
        let sweep = Sweep::within(2533, NOW, WINDOW);
        assert!(!sweep.is_orphan(Stamp::new(9999, NOW + micros(WINDOW) * 1_000)));
    }

    #[test]
    fn only_a_name_spelled_this_way_under_this_prefix_is_ever_an_orphan() {
        let sweep = Sweep::within(2533, NOW, WINDOW);
        let stale = NOW - micros(WINDOW) - 1;
        assert!(sweep.names_an_orphan("live ", &format!("live 9999-{stale}")));
        for foreign in [
            format!("9999-{stale}"),
            format!("copy of live 9999-{stale}"),
            "live 9999".to_owned(),
            "live ".to_owned(),
            format!("live 2533-{stale}"),
        ] {
            assert!(
                !sweep.names_an_orphan("live ", &foreign),
                "{foreign:?} is not an orphan of this run's"
            );
        }
    }

    #[test]
    fn the_declared_window_is_the_one_a_lane_sweeps_on() {
        // `Sweep::of` is what the lanes call, and what it must not do is quietly carry a
        // window of its own: the reasoning for the length is written on the constant, and a
        // second figure here would mean nothing a reader of that reasoning could act on.
        let stale = NOW - micros(STALE_AFTER) - 1;
        assert!(Sweep::of(2533, NOW).is_orphan(Stamp::new(9999, stale)));
        assert!(!Sweep::of(2533, NOW).is_orphan(Stamp::new(9999, stale + 2)));
    }
}
