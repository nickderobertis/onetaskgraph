//! The clock a source paces and backs off on.
//!
//! A plugin that waits — between pages to stay under a published rate, or before retrying a
//! refused request — waits on the [`Clock`] it was built with rather than on the runtime's own
//! timer, so a test can run it on simulated time: a minute of pacing then costs no real time,
//! and the figures a timing budget reports are the same on every run. The binary hands every
//! in-process source the process's one clock through [`SourcePlugin::build_with_clock`]; with
//! nothing asking for simulated time that is [`system_clock`], and every plugin behaves exactly
//! as it does on the runtime's timer.
//!
//! [`SourcePlugin::build_with_clock`]: crate::SourcePlugin::build_with_clock

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// A source of monotonic time and of waits measured against it.
pub trait Clock: Send + Sync {
    /// Monotonic time elapsed since this clock's origin.
    fn now(&self) -> std::time::Duration;
    /// Complete once `duration` has passed on this clock.
    fn sleep(
        &self,
        duration: std::time::Duration,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>>;
    /// [`sleep`](Self::sleep), for a wait whose reason is `cause` — the fixed word a plugin
    /// names its waits by, such as the limit it is pacing under.
    ///
    /// A clock that keeps a record of what waited and why — a test's simulated one — keeps the
    /// cause beside the wait; every other clock waits exactly as `sleep` does, which is what
    /// this is defaulted to.
    fn sleep_because(
        &self,
        duration: std::time::Duration,
        cause: WaitCause,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>> {
        let _ = cause;
        self.sleep(duration)
    }
}

/// The word a plugin names one kind of wait by — see [`Clock::sleep_because`].
///
/// One word: a nonempty run of printable ASCII with no whitespace, because a simulated clock
/// carries it as one word of a line. [`WaitCause::new`] refuses anything else, and it is a
/// `const fn`, so a cause declared as a constant that is not a word fails the build rather
/// than reaching a clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WaitCause(&'static str);

impl WaitCause {
    /// The cause named `word`.
    ///
    /// # Panics
    ///
    /// When `word` is not [one word](Self::is_word) — at compile time, for a constant.
    #[must_use]
    pub const fn new(word: &'static str) -> Self {
        assert!(
            Self::is_word(word),
            "a wait cause is one word of printable ASCII"
        );
        Self(word)
    }

    /// Whether `text` is a word a cause may be: nonempty printable ASCII, no whitespace.
    ///
    /// The one statement of the rule, for whatever reads a cause back off a clock's wire.
    #[must_use]
    pub const fn is_word(text: &str) -> bool {
        let bytes = text.as_bytes();
        if bytes.is_empty() {
            return false;
        }
        let mut index = 0;
        while index < bytes.len() {
            if !bytes[index].is_ascii_graphic() {
                return false;
            }
            index += 1;
        }
        true
    }

    /// The word itself.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

impl std::fmt::Display for WaitCause {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.0)
    }
}

/// The variable that puts a process of this build on a simulated clock.
///
/// Its value is `<address>/<client>`: a test's coordinator's loopback address and the
/// process's 0-based client number. Unset or empty, the process runs on [`system_clock`]. It
/// does not begin `ONETASKGRAPH_`, so the configuration's environment layer never reads it as a
/// setting. Named here, beside the clock it selects, so the binary that reads it and the test
/// harness that hands it out spell it once between them.
pub const SIMULATED_CLOCK_VARIABLE: &str = "OTG_SIMULATED_CLOCK";

/// One clock, shared by everything a process builds.
pub type SharedClock = std::sync::Arc<dyn Clock>;

/// The real clock: `tokio::time::sleep` and a monotonic `Instant`.
#[must_use]
pub fn system_clock() -> SharedClock {
    Arc::new(SystemClock {
        origin: Instant::now(),
    })
}

/// [`system_clock`]'s clock: its origin is the instant it was made.
struct SystemClock {
    origin: Instant,
}

impl Clock for SystemClock {
    fn now(&self) -> Duration {
        self.origin.elapsed()
    }

    fn sleep(&self, duration: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        Box::pin(tokio::time::sleep(duration))
    }
}

#[cfg(test)]
mod tests {
    use super::WaitCause;

    #[test]
    fn a_word_of_printable_ascii_is_a_cause_and_nothing_else_is() {
        const FILING: WaitCause = WaitCause::new("board-filing");
        assert_eq!(FILING.as_str(), "board-filing");
        assert_eq!(FILING.to_string(), "board-filing");
        for refused in [
            "",
            " ",
            "two words",
            "tab\there",
            "line\n",
            "café",
            "\u{a0}nbsp",
        ] {
            assert!(!WaitCause::is_word(refused), "{refused:?} is not one word");
        }
    }

    #[test]
    #[should_panic(expected = "a wait cause is one word of printable ASCII")]
    fn a_cause_that_is_not_one_word_is_refused_where_it_is_made() {
        let words = String::from("two words");
        let _ = WaitCause::new(Box::leak(words.into_boxed_str()));
    }
}
