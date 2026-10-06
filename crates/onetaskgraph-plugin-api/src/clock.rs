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
}

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
