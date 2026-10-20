//! The 40% rule: a live session that finds a budget it draws on already holding under
//! [`PAUSE_BELOW`] of its whole allowance waits for that budget's window to reset rather than
//! spending into it.
//!
//! Every session on a shared credential draws on one allowance, so a session that starts — or
//! goes on — while a budget is low takes what the work outside this repository was going to
//! spend before the window comes back. The rule is the host's, and it is the same for every
//! lane: under [`PAUSE_BELOW`] of the limit, **pause until the reset**, unless the rate the
//! budget has been spent at since its window began says it resets before this session's
//! estimate would exhaust it — then go on, because waiting would protect nothing. After a
//! pause the budget is read again, and decided again on what that read says.
//!
//! **A budget is the least of every reading of it.** One endpoint has been observed claiming a
//! GraphQL budget thousands of points fuller than the headers of real answers reported in the
//! same minutes, so no single read is trusted: a lane hands over every reading it made of a
//! budget — an allowance endpoint, the API's own rate-limit object, the headers of the last
//! real answer — and the rule decides on the one leaving the smallest share of its own limit.
//!
//! This sits beside, and before, [`affordable`]: pacing decides *when* a
//! session may draw on a budget, and the retained buffer then decides *whether* it can afford
//! to — for every budget but one the burn-rate exception let through. Under [`PAUSE_BELOW`]
//! the rule is the one decision: a budget it waits for is read again after its reset, and one
//! whose burn rate resets first is drawn on without waiting and is **not** then held to the
//! buffer, because the projection to the reset — every consumer's spend at the rate the window
//! has seen, plus this session's estimate — is what protects the other consumers until the
//! window comes back. Holding it to the buffer as well would end, or stall, the very session
//! the rule says goes on. A budget nothing could read is not paced at all — there is no
//! reading to wait on — and is refused by `affordable` exactly as before.
//!
//! **The rule has no give-up.** No count of pauses, elapsed wait or retry limit ends a session
//! whose budget is still under [`PAUSE_BELOW`]: the only ways past such a reading are its reset
//! followed by a read at or above it, or the burn-rate exception. A lane's wall-clock bound is
//! the CI job's own timeout, never a cap here — a cap would decline, or spend into, the very
//! budget the rule exists to protect.

use std::fmt;
use std::future::Future;
use std::pin::Pin;

use crate::{Allowance, Demand, Fraction, Metered, Unaffordable, affordable, still_affordable};

/// The share of a budget's whole allowance under which a live session pauses for the reset.
///
/// Forty per cent, declared once for every budget and every lane, of the **whole** allowance
/// for the reason [`RETAINED_BUFFER`](crate::RETAINED_BUFFER) is: what is protected is a fixed
/// amount, not a share of whatever happens to be left. Nothing lowers it.
pub const PAUSE_BELOW: Fraction = Fraction::new(40, 100);

/// One reading of one budget, and the read that made it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reading {
    metered: Metered,
    by: String,
    allowance: Result<Allowance, String>,
}

impl Reading {
    /// What `by` read of `metered`: an allowance, or why it could not be read.
    #[must_use]
    pub fn new(
        metered: Metered,
        by: impl Into<String>,
        allowance: Result<Allowance, String>,
    ) -> Self {
        Self {
            metered,
            by: by.into(),
            allowance,
        }
    }
    /// Which budget it is a reading of.
    #[must_use]
    pub const fn metered(&self) -> Metered {
        self.metered
    }
    /// The read that made it, as the lane names it.
    #[must_use]
    pub fn by(&self) -> &str {
        &self.by
    }
    /// What it read.
    ///
    /// # Errors
    ///
    /// Why the read could not be made.
    pub fn allowance(&self) -> Result<Allowance, &str> {
        self.allowance.as_ref().copied().map_err(String::as_str)
    }
}

/// The least of every reading of `metered` among `readings`: the one leaving the smallest share
/// of its own limit, and the read that made it.
///
/// Compared as shares rather than as remainders, because two reads of one budget can report it
/// against different limits; the smaller remainder breaks a tie of shares, and the first given
/// a tie of both.
///
/// # Errors
///
/// When no reading of `metered` was answered — every reason, joined, so the refusal that
/// follows says what each read met.
pub fn least(metered: Metered, readings: &[Reading]) -> Result<(Allowance, String), String> {
    let mut least: Option<(Allowance, &str)> = None;
    let mut unread = Vec::new();
    for reading in readings.iter().filter(|reading| reading.metered == metered) {
        match reading.allowance() {
            Ok(allowance) => {
                let lower = least.is_none_or(|(held, _)| {
                    let share = u128::from(allowance.remaining()) * u128::from(held.limit().max(1));
                    let held_share =
                        u128::from(held.remaining()) * u128::from(allowance.limit().max(1));
                    share < held_share
                        || (share == held_share && allowance.remaining() < held.remaining())
                });
                if lower {
                    least = Some((allowance, reading.by()));
                }
            }
            Err(why) => unread.push(format!("{}: {why}", reading.by())),
        }
    }
    least
        .map(|(allowance, by)| (allowance, by.to_owned()))
        .ok_or_else(|| {
            if unread.is_empty() {
                format!("no read reported the {} budget at all", metered.budget())
            } else {
                unread.join("; ")
            }
        })
}

/// What the 40% rule says about one budget at one moment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pace {
    /// Draw on it now: at least [`PAUSE_BELOW`] of its limit remains, or its reset has already
    /// passed. The retained buffer then decides whether the session can afford it.
    Go,
    /// Draw on it now: under [`PAUSE_BELOW`], but the burn rate since its window began says it
    /// resets before the session's estimate would exhaust it. The rule has decided, so the
    /// retained buffer is not held against this budget at this decision.
    ResetsFirst,
    /// Wait until the UTC epoch second `until`, when its window resets, and read it again.
    Wait {
        /// The reset the reading decided on reported.
        until: u64,
    },
}

/// The 40% rule for one budget read as `allowance` at the UTC epoch second `now`, for a
/// session estimated to spend `estimated_cost` of it, whose window lasts `window` seconds.
///
/// [`Pace::Go`] when at least [`PAUSE_BELOW`] of the limit remains. Under it, the burn rate
/// since the window began — what has been used, over the time since `reset - window` — is
/// projected to the reset: when what remains still covers that projection and the session's
/// estimate, the budget resets before it would be exhausted and the answer is
/// [`Pace::ResetsFirst`]; otherwise [`Pace::Wait`] until the reset. A reset already past is never waited for — the
/// reading is behind its own window, and the next read is what tells — and a window with no
/// time behind it yet has no rate to project, so it waits.
#[must_use]
pub fn pace(allowance: Allowance, estimated_cost: u64, now: u64, window: u64) -> Pace {
    if allowance.remaining() >= PAUSE_BELOW.of(allowance.limit()) {
        return Pace::Go;
    }
    let reset = allowance.reset();
    if reset <= now {
        return Pace::Go;
    }
    let began = reset.saturating_sub(window);
    let elapsed = now.saturating_sub(began);
    if elapsed > 0 {
        let used = allowance.limit().saturating_sub(allowance.remaining());
        let to_reset = reset - now;
        // Rounded up, so a rate that only just fits is not taken to fit.
        let projected = (u128::from(used) * u128::from(to_reset)).div_ceil(u128::from(elapsed));
        if projected + u128::from(estimated_cost) < u128::from(allowance.remaining()) {
            return Pace::ResetsFirst;
        }
    }
    Pace::Wait { until: reset }
}

/// One pause a session took under the 40% rule: what it decided on, and how long it waited.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pause {
    /// The budget it paused for.
    pub metered: Metered,
    /// The reading it decided on — the least of every reading of that budget.
    pub decided_on: Allowance,
    /// Which read made that reading.
    pub by: String,
    /// The UTC epoch second it began waiting.
    pub from: u64,
    /// The UTC epoch second it waited until: the reset that reading reported.
    pub until: u64,
}

impl fmt::Display for Pause {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "paused {} s for the {} budget: {} reported {} of {} {} remaining, under {} of the \
             limit, so the session waited from {} until its reset at {} (UTC epoch seconds) and \
             read it again",
            self.until.saturating_sub(self.from),
            self.metered.budget(),
            self.by,
            self.decided_on.remaining(),
            self.decided_on.limit(),
            self.metered.unit(),
            PAUSE_BELOW,
            self.from,
            self.until,
        )
    }
}

/// What a lane gives the gate so that it can read a session's budgets and wait for them.
///
/// The reads are the lane's — only the API knows its allowance, and each answers it in its own
/// terms — and so is the clock, which a stand-in drives in virtual time. The rule is here.
pub trait Gauge: Send {
    /// Every reading of every budget the session draws on, made now.
    fn read(&mut self) -> Pin<Box<dyn Future<Output = Vec<Reading>> + Send + '_>>;
    /// The UTC epoch second it is now, on the clock the lane waits on.
    fn now(&self) -> u64;
    /// Complete at the UTC epoch second `until`, on that clock.
    fn wait_until(&mut self, until: u64) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;
    /// How many seconds one window of `metered` lasts, from which the burn rate is measured.
    fn window(&self, metered: Metered) -> u64;
}

/// Read every budget of `demands` — each a budget and what the session is estimated to spend
/// of it — and wait out each the 40% rule pauses for, reading again after every wait; then
/// answer one [`Demand`] per budget, on the least reading of it, with the read that made it.
///
/// Every pause is pushed onto `pauses`, in the order taken, so the session's report says what
/// it waited for. There is no limit on how many: a budget still under [`PAUSE_BELOW`] after its
/// reset is waited for again, however many resets that takes, because nothing but a reading at
/// or above it — or a burn rate that resets first — lets a session past it.
pub async fn paced(
    gauge: &mut dyn Gauge,
    demands: &[(Metered, u64)],
    pauses: &mut Vec<Pause>,
) -> Vec<(Demand, String)> {
    loop {
        let readings = gauge.read().await;
        let now = gauge.now();
        let decided: Vec<(Demand, String, Option<u64>)> = demands
            .iter()
            .map(|(metered, cost)| match least(*metered, &readings) {
                Ok((allowance, by)) => {
                    let wait = match pace(allowance, *cost, now, gauge.window(*metered)) {
                        Pace::Go | Pace::ResetsFirst => None,
                        Pace::Wait { until } => Some(until),
                    };
                    (Demand::read(*metered, *cost, allowance), by, wait)
                }
                Err(why) => (Demand::unread(*metered, *cost, why.clone()), why, None),
            })
            .collect();
        let due = decided
            .iter()
            .find_map(|(demand, by, wait)| wait.map(|until| (demand, by, until)));
        match due {
            Some((demand, by, until)) => {
                pauses.push(Pause {
                    metered: demand.metered(),
                    decided_on: demand
                        .allowance()
                        .expect("only a budget that was read is paced"),
                    by: by.clone(),
                    from: now,
                    until,
                });
                gauge.wait_until(until).await;
            }
            None => {
                return decided
                    .into_iter()
                    .map(|(demand, by, _)| (demand, by))
                    .collect();
            }
        }
    }
}

/// Whether the retained buffer decides `demand` after the 40% rule went on: every budget but
/// one whose reading the rule let through on its burn rate alone, [`Pace::ResetsFirst`] —
/// asked of the reading [`paced`] answered, at the moment it answered it, which is the reading
/// and the moment the rule decided on. A budget that was not read is always held, so
/// [`affordable`] refuses it as unread.
fn held_to_buffer(gauge: &dyn Gauge, demand: &Demand) -> bool {
    match demand.allowance() {
        Err(_) => true,
        Ok(allowance) => {
            pace(
                allowance,
                demand.estimated_cost(),
                gauge.now(),
                gauge.window(demand.metered()),
            ) != Pace::ResetsFirst
        }
    }
}

/// What a session was admitted on under the 40% rule, and every pause it has taken since —
/// the one implementation [`Session::open_paced`](crate::Session::open_paced) and a lane that
/// admits its own session both decide by.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Admission {
    /// One per budget, on the least reading of it, with the read that made that reading.
    admitted: Vec<(Demand, String)>,
    /// Every pause taken, in order.
    pauses: Vec<Pause>,
}

impl Admission {
    /// Admit a session drawing on `demands` — each a budget and what the session is estimated
    /// to spend of it: [`paced`] through `gauge`, then [`affordable`] on the readings decided on,
    /// but for a budget the rule let through as [`Pace::ResetsFirst`].
    ///
    /// # Errors
    ///
    /// What [`affordable`] refuses, on the least reading of each budget after every pause.
    pub async fn admit(
        gauge: &mut dyn Gauge,
        demands: &[(Metered, u64)],
    ) -> Result<Self, Unaffordable> {
        let mut pauses = Vec::new();
        let admitted = paced(gauge, demands, &mut pauses).await;
        let held: Vec<Demand> = admitted
            .iter()
            .map(|(demand, _)| demand.clone())
            .filter(|demand| held_to_buffer(gauge, demand))
            .collect();
        affordable(&held)?;
        Ok(Self { admitted, pauses })
    }

    /// Decide the session again under the same rule: [`paced`] through `gauge` with
    /// `remaining` — each budget and what the session is still estimated to spend of it — and
    /// then each budget's least reading held to [`still_affordable`] against the reading it was
    /// admitted on — but for a budget the rule let through as [`Pace::ResetsFirst`], which goes
    /// on. A budget it was not admitted on is not decided here.
    ///
    /// # Errors
    ///
    /// What [`still_affordable`] refuses, naming both readings.
    pub async fn recheck(
        &mut self,
        gauge: &mut dyn Gauge,
        remaining: &[(Metered, u64)],
    ) -> Result<(), Unaffordable> {
        let decided = paced(gauge, remaining, &mut self.pauses).await;
        for (now, carried_by) in decided {
            if !held_to_buffer(gauge, &now) {
                continue;
            }
            let Some((admitted, claimed_by)) = self
                .admitted
                .iter()
                .find(|(admitted, _)| admitted.metered() == now.metered())
            else {
                continue;
            };
            let Ok(claimed) = admitted.allowance() else {
                continue;
            };
            let owed = Demand::read(now.metered(), now.estimated_cost(), claimed);
            let carried = now.allowance().map_err(str::to_owned);
            still_affordable(&owed, claimed_by, carried, &carried_by)?;
        }
        Ok(())
    }

    /// One demand per budget, on the reading it was admitted on.
    #[must_use]
    pub fn demands(&self) -> Vec<Demand> {
        self.admitted
            .iter()
            .map(|(demand, _)| demand.clone())
            .collect()
    }

    /// The read that made the reading `metered` was admitted on.
    #[must_use]
    pub fn admitted_by(&self, metered: Metered) -> Option<&str> {
        self.admitted
            .iter()
            .find(|(demand, _)| demand.metered() == metered)
            .map(|(_, by)| by.as_str())
    }

    /// Every pause taken, at admission and at every recheck, in order.
    #[must_use]
    pub fn pauses(&self) -> &[Pause] {
        &self.pauses
    }

    /// One line per pause, each naming the session `name`, or one line saying it took none.
    #[must_use]
    pub fn report(&self, name: &str) -> String {
        if self.pauses.is_empty() {
            return format!(
                "the {name} live session took no pause: every budget it drew on held at least \
                 {PAUSE_BELOW} of its limit, or reset before the session would exhaust it"
            );
        }
        self.pauses
            .iter()
            .map(|pause| format!("the {name} live session {pause}"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GRAPHQL: Metered = Metered::new("graphql", "points");

    fn allowance(remaining: u64, reset: u64) -> Allowance {
        Allowance::read(5_000, remaining, reset).unwrap()
    }

    #[test]
    fn at_or_above_forty_per_cent_a_budget_is_drawn_on_at_once() {
        assert_eq!(pace(allowance(2_000, 2_000), 100, 1_000, 3_600), Pace::Go);
        assert_eq!(pace(allowance(4_999, 2_000), 4_000, 1_000, 3_600), Pace::Go);
    }

    #[test]
    fn under_forty_per_cent_the_session_waits_for_the_reset_unless_the_burn_resets_first() {
        // 3,500 used over the 2,600 s since the window began at 400: the 1,000 s to the reset
        // at that rate is 1,347 more, which with an estimate of 100 is under the 1,500 left —
        // it resets first.
        assert_eq!(
            pace(allowance(1_500, 4_000), 100, 3_000, 3_600),
            Pace::ResetsFirst
        );
        // The same reading 10 s into its window has burnt 3,500 in 10 s: it would be gone
        // long before the reset, so the session waits for it.
        assert_eq!(
            pace(allowance(1_500, 4_000), 100, 410, 3_600),
            Pace::Wait { until: 4_000 }
        );
        // A reset already past is not waited for.
        assert_eq!(pace(allowance(10, 900), 100, 1_000, 3_600), Pace::Go);
    }

    #[test]
    fn a_burn_that_resets_first_goes_on_without_waiting_even_where_the_estimate_breaches_the_buffer()
     {
        // The credentialed lane's cleanup on CI: 462 of 5,000, 4,538 used over the 3,500 s
        // since the window began at 500, so the 100 s to the reset at that rate is 130 more,
        // and with an estimate of 123 still under the 462 left — it resets first. 462 less 123
        // leaves 339 where the retained buffer is 1,000, and the rule goes on regardless.
        assert_eq!(
            pace(allowance(462, 4_100), 123, 4_000, 3_600),
            Pace::ResetsFirst
        );
    }

    /// A gauge whose budget stays low: every read reports ten per cent left, resetting a minute
    /// after now, until `recovers_after` waits have passed — then the whole allowance. Each wait
    /// moves now to the reset asked for; past `wait_forever_after` waits, a wait never completes,
    /// so a poll shows the decision still waiting rather than made.
    struct Low {
        now: u64,
        reads: usize,
        waits: Vec<u64>,
        recovers_after: usize,
        wait_forever_after: usize,
    }

    impl Low {
        fn new(recovers_after: usize, wait_forever_after: usize) -> Self {
            Self {
                now: 1_000,
                reads: 0,
                waits: Vec::new(),
                recovers_after,
                wait_forever_after,
            }
        }
    }

    impl Gauge for Low {
        fn read(&mut self) -> Pin<Box<dyn Future<Output = Vec<Reading>> + Send + '_>> {
            self.reads += 1;
            let remaining = if self.waits.len() >= self.recovers_after {
                5_000
            } else {
                500
            };
            let reading = Reading::new(GRAPHQL, "low", Ok(allowance(remaining, self.now + 60)));
            Box::pin(std::future::ready(vec![reading]))
        }
        fn now(&self) -> u64 {
            self.now
        }
        fn wait_until(&mut self, until: u64) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
            if self.waits.len() >= self.wait_forever_after {
                return Box::pin(std::future::pending());
            }
            self.waits.push(until);
            self.now = until;
            Box::pin(std::future::ready(()))
        }
        fn window(&self, _: Metered) -> u64 {
            // A window that began a moment ago, so the burn rate never says it resets first.
            61
        }
    }

    /// Poll a future once, with no runtime: its value when every await it reached was ready.
    fn poll_once<T>(future: impl Future<Output = T>) -> std::task::Poll<T> {
        let mut future = std::pin::pin!(future);
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        future.as_mut().poll(&mut context)
    }

    /// Drive a future whose every await is ready, with no runtime.
    fn ready<T>(future: impl Future<Output = T>) -> T {
        match poll_once(future) {
            std::task::Poll::Ready(value) => value,
            std::task::Poll::Pending => panic!("every await here is ready"),
        }
    }

    /// Far more resets than the four pauses the rule was once capped at.
    const MANY: usize = 50;

    #[test]
    fn a_budget_that_stays_under_forty_per_cent_is_waited_for_however_many_resets_it_takes() {
        // Still under forty per cent after every one of MANY resets: no decision is made — the
        // session is still waiting, neither declined nor let through.
        let mut gauge = Low::new(usize::MAX, MANY);
        let mut pauses = Vec::new();
        assert!(
            poll_once(paced(&mut gauge, &[(GRAPHQL, 100)], &mut pauses)).is_pending(),
            "a budget under forty per cent is never decided on"
        );
        assert_eq!(
            pauses.len(),
            MANY + 1,
            "paused once more, for the next reset"
        );
        assert_eq!(gauge.waits.len(), MANY);
        assert_eq!(gauge.reads, MANY + 1, "read again after every pause");
        let mut gauge = Low::new(usize::MAX, MANY);
        assert!(
            poll_once(Admission::admit(&mut gauge, &[(GRAPHQL, 100)])).is_pending(),
            "admission is not declined while the budget is under forty per cent"
        );
        // The same recovering after MANY resets: admitted on the reading after the last one.
        let mut gauge = Low::new(MANY, usize::MAX);
        let admitted = ready(Admission::admit(&mut gauge, &[(GRAPHQL, 100)]))
            .unwrap_or_else(|declined| panic!("{declined:?}"));
        assert_eq!(admitted.pauses().len(), MANY);
        assert_eq!(gauge.reads, MANY + 1, "read again after every pause");
        assert_eq!(
            gauge.waits,
            (1..=MANY)
                .map(|n| 1_000 + 60 * u64::try_from(n).unwrap())
                .collect::<Vec<_>>(),
            "each wait is until the reset the reading reported"
        );
        assert_eq!(
            admitted.demands()[0].allowance().map(Allowance::remaining),
            Ok(5_000)
        );
    }

    #[test]
    fn a_recheck_that_stays_under_forty_per_cent_is_waited_for_however_many_resets_it_takes() {
        // Admitted at the whole allowance, then a recheck that finds it under forty per cent
        // through MANY resets: still waiting, never ended.
        let mut gauge = Low::new(0, usize::MAX);
        let mut admitted = ready(Admission::admit(&mut gauge, &[(GRAPHQL, 100)])).unwrap();
        let mut gauge = Low::new(usize::MAX, MANY);
        assert!(
            poll_once(admitted.recheck(&mut gauge, &[(GRAPHQL, 100)])).is_pending(),
            "a recheck under forty per cent does not end the session"
        );
        assert_eq!(gauge.waits.len(), MANY);
        assert_eq!(gauge.reads, MANY + 1, "read again after every pause");
        // And recovering after MANY resets, the recheck goes on.
        let mut admitted = ready(Admission::admit(
            &mut Low::new(0, usize::MAX),
            &[(GRAPHQL, 100)],
        ))
        .unwrap();
        let mut gauge = Low::new(MANY, usize::MAX);
        ready(admitted.recheck(&mut gauge, &[(GRAPHQL, 100)]))
            .unwrap_or_else(|declined| panic!("{declined:?}"));
        assert_eq!(admitted.pauses().len(), MANY);
        assert_eq!(gauge.reads, MANY + 1);
    }

    #[test]
    fn the_least_reading_is_the_smallest_share_of_its_own_limit() {
        let readings = vec![
            Reading::new(GRAPHQL, "endpoint", Ok(allowance(4_800, 9))),
            Reading::new(
                GRAPHQL,
                "headers",
                Ok(Allowance::read(1_000, 300, 9).unwrap()),
            ),
            Reading::new(GRAPHQL, "object", Ok(allowance(1_600, 9))),
            Reading::new(GRAPHQL, "broken", Err("no answer".into())),
            Reading::new(
                Metered::new("core", "requests"),
                "rest",
                Ok(allowance(1, 9)),
            ),
        ];
        let (least, by) = least(GRAPHQL, &readings).unwrap();
        assert_eq!((least.remaining(), by.as_str()), (300, "headers"));
        assert!(super::least(Metered::new("search", "requests"), &readings).is_err());
    }
}
