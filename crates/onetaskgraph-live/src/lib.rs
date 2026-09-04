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
//! # The preconditions this crate ships
//!
//! **Exclusivity**: a test that reads and writes a shared external fixture must not run
//! concurrently with another instance of itself. Both live journeys sweep residue by
//! title before they start — that is what makes them self-healing after an interrupted run
//! — and a sweep that recognises *any* run's artifacts will delete a concurrent run's
//! in-flight items. So concurrency is a correctness problem here rather than a cost one,
//! and [`Session::open`] holds a seat for the session's name for as long as the session
//! lasts. A second instance is declined rather than allowed to race.
//!
//! **Affordability**: a live session must never be the thing that exhausts a budget the
//! work outside this repository depends on. [`affordable`] is that decision, and
//! [`RETAINED_BUFFER`] is the share of each budget's whole allowance a session may never
//! touch. The *reads* that learn an allowance belong to the lane — an allowance is a fact
//! only the API holds, and each API answers it in its own terms — but the arithmetic and
//! the buffer are here, once, so two lanes cannot come to protect two different shares.
//!
//! A session that cannot afford itself is **declined**, not failed and not passed: it is
//! the third answer above, carried by [`Declined::unaffordable`], whose cause is readable
//! as a value through [`Declined::unaffordable_because`] rather than out of its prose.
//! Nothing here waits for a budget to come back — see [`Unaffordable`].

#![deny(missing_docs)]

use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write as _};
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
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

/// How many seats this process has taken, which is the last part of a seat's own token.
///
/// The process and the thread do not tell two seats apart on their own: one thread that
/// takes a seat, has it reclaimed while it still holds it, and then takes another at the
/// same path would write the same two halves twice. What [`Seat::drop`] compares has to be
/// unique to the file, so this is counted rather than derived.
static SEATS_TAKEN: AtomicU64 = AtomicU64::new(0);

/// What that variable has to be set to for a live session to be expected.
///
/// A `const` rather than a literal in the match below because it is read from outside this
/// crate: `.github/workflows/ci.yml` sets the variable, and `scripts/check-live-lane.sh`
/// refuses a credential-carrying step that sets it to anything else — reading this constant
/// out of this file rather than restating the value, so the workflow and the parser that
/// reads it cannot come to disagree about which spelling is the demand.
pub const DEMANDED: &str = "1";

/// What it is set to where a live session is deliberately not expected.
///
/// The other half of the pair, and read from outside this crate for the same reason:
/// `.github/workflows/ci.yml` spells the fork exception as an expression that yields this
/// value on a fork pull request and [`DEMANDED`] on every other run, and
/// `scripts/check-live-lane.sh` holds it to exactly those two. An unset variable and an
/// empty one mean the same thing as this, but neither is something a workflow can be held
/// to writing down.
pub const NOT_DEMANDED: &str = "0";

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
        None | Some("") | Some(NOT_DEMANDED) => Ok(false),
        Some(DEMANDED) => Ok(true),
        Some(other) => Err(format!(
            "{REQUIRED_VARIABLE} must be {DEMANDED}, {NOT_DEMANDED} or unset, not {other:?}"
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

/// A part of a whole, as two whole numbers rather than as a floating-point share.
///
/// Two integers because a budget's retained buffer is compared against integer points and
/// integer requests, and because `0.2` is not `1/5` in binary: a share written as a float
/// would make the buffer a fraction of a point wider or narrower than the number this
/// repository says it is, differently on each budget's own scale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fraction {
    numerator: u64,
    denominator: u64,
}

impl Fraction {
    /// The fraction `numerator`/`denominator`.
    ///
    /// **Private, and that is the whole answer to a zero denominator.** The only fraction
    /// this workspace has is [`RETAINED_BUFFER`] below, declared in this file, so there is
    /// no caller to report an error to and nothing outside can ask for a fraction that is
    /// not one: the `assert!` runs when this crate is compiled rather than when a session
    /// consults it, and a second constant spelled `Fraction::new(20, 0)` would fail to
    /// build. A public constructor returning `Result` would move that from build time to
    /// run time and buy nothing, because the buffer must not be constructible at run time
    /// at all — see [`RETAINED_BUFFER`] on why nothing lowers it.
    ///
    /// # Panics
    ///
    /// When `denominator` is zero, which is not a fraction.
    const fn new(numerator: u64, denominator: u64) -> Self {
        assert!(denominator != 0, "a fraction has a non-zero denominator");
        Self {
            numerator,
            denominator,
        }
    }
    /// This fraction of `whole`, rounded **up**.
    ///
    /// Up rather than to nearest: this is what a session may not touch, and a buffer
    /// rounded down would be a share smaller than the one stated. The arithmetic is done
    /// in `u128` so a whole near `u64::MAX` cannot overflow into a buffer of nearly
    /// nothing.
    #[must_use]
    pub const fn of(self, whole: u64) -> u64 {
        let numerator = whole as u128 * self.numerator as u128;
        let denominator = self.denominator as u128;
        let rounded_up = numerator.div_ceil(denominator);
        if rounded_up > u64::MAX as u128 {
            u64::MAX
        } else {
            rounded_up as u64
        }
    }
    /// The numerator, for a message that says what share the buffer is.
    #[must_use]
    pub const fn numerator(self) -> u64 {
        self.numerator
    }
    /// The denominator.
    #[must_use]
    pub const fn denominator(self) -> u64 {
        self.denominator
    }
}

impl fmt::Display for Fraction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}/{}", self.numerator, self.denominator)
    }
}

/// The share of a budget's **whole allowance** a live session may never touch.
///
/// Twenty per cent, declared once for every budget and every lane. Of the *allowance*
/// rather than of what happens to be left: a share of the remainder shrinks precisely when
/// protection matters most, and what this exists to guarantee is that a fixed amount is
/// always there for the work outside this repository. It is a [`Fraction`] rather than a
/// count of points or requests so that it means the same thing on a budget metered in
/// points and on one metered in requests, and so that it cannot drift into two numbers.
///
/// Nothing lowers it: it is a constant, and no environment variable, command-line option
/// or configuration file reads it or replaces it. A session that cannot fit under it does
/// not run.
pub const RETAINED_BUFFER: Fraction = Fraction::new(20, 100);

/// A budget, and what it is metered in — one fact rather than two fields.
///
/// **Paired once, by the one place that knows both.** A budget's name and its unit are not
/// independent: GitHub's GraphQL budget is metered in points and its REST one in requests,
/// and a value that carried them separately would let a refusal say the GraphQL budget was
/// short of requests, three types away from where the pair was decided. So they travel
/// together, from [`Demand`] into [`Unaffordable`], and nothing downstream can transpose or
/// re-spell either.
///
/// `&'static str` rather than `String` for the same reason: a budget's name is a constant
/// of the lane that draws on it — GitHub's are `Budget::name` and `Budget::unit` on a closed
/// enum — and never a value read off a response, so nothing a third party sends can become
/// one.
// llmlint: ignore-block[invalid_states_unrepresentable] This crate is the gate every lane opens, and the set of budgets is the LANE's, not this crate's: a closed enum here would have to name GitHub's `graphql` and `core` and Linear's next one, which is the dependency this crate exists not to have. So the pairing is made unrepresentable where both halves are known — `journey::budget::metered` in `onetaskgraph-github-projects`, total over that crate's own closed `Budget` enum and the only production call of this constructor — and what travels from there is the settled pair rather than two strings a `Demand` could transpose. That transposition was the state this type was introduced to remove, and it is removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Metered {
    budget: &'static str,
    unit: &'static str,
}

impl Metered {
    /// The budget `budget`, metered in `unit`.
    ///
    /// **The pair is decided by the lane, because the budgets are the lane's.** The one
    /// production caller is `journey::budget::metered` in `onetaskgraph-github-projects`,
    /// which derives both halves from that crate's own closed `Budget` enum, so no pairing
    /// this workspace makes can be transposed or re-spelled.
    #[must_use]
    pub const fn new(budget: &'static str, unit: &'static str) -> Self {
        Self { budget, unit }
    }
    // llmlint: ignore-end[invalid_states_unrepresentable]
    /// Which budget.
    #[must_use]
    pub const fn budget(self) -> &'static str {
        self.budget
    }
    /// What it is metered in.
    #[must_use]
    pub const fn unit(self) -> &'static str {
        self.unit
    }
}

/// One budget's primary allowance, as the account itself reported it.
///
/// **Read rather than assembled**, for the reason the accounting's own `RateLimit` is: a
/// hand-built allowance could say more remained than the whole allowance, and a gate that
/// decided on one would be protecting nothing. [`Allowance::read`] is the only way to have
/// one, and its caller is whichever lane made the read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Allowance {
    limit: u64,
    remaining: u64,
    reset: u64,
}

impl Allowance {
    /// The allowance an API just reported: its whole size, what is left, and the UTC epoch
    /// second the window resets.
    ///
    /// `None` when more is reported remaining than the whole allowance, which is not an
    /// allowance any API could truthfully report. That state is refused here rather than
    /// carried, because [`affordable`] would answer it by comparing a buffer computed from
    /// one figure against a remainder from another — a decision made on numbers that cannot
    /// both be true, which is worse than no decision. A caller that meets it has a budget
    /// it did not read, and [`Demand::unread`] is what that is.
    #[must_use]
    pub const fn read(limit: u64, remaining: u64, reset: u64) -> Option<Self> {
        if remaining > limit {
            return None;
        }
        Some(Self {
            limit,
            remaining,
            reset,
        })
    }
    /// The whole allowance for this budget's window.
    #[must_use]
    pub const fn limit(self) -> u64 {
        self.limit
    }
    /// What is left of it.
    #[must_use]
    pub const fn remaining(self) -> u64 {
        self.remaining
    }
    /// The UTC epoch second the window resets.
    #[must_use]
    pub const fn reset(self) -> u64 {
        self.reset
    }
}

/// One budget a session will draw on: what it will cost, and what the account has left.
///
/// The allowance is a `Result` because **an allowance the session could not read is not an
/// allowance it may assume**. A budget whose read the API did not answer is unknown, and an
/// unknown budget is not an affordable one — so the two cases are one field with two
/// constructors rather than an `Option` a caller could forget to check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Demand {
    metered: Metered,
    estimated_cost: u64,
    allowance: Result<Allowance, String>,
}

impl Demand {
    /// A budget whose allowance was read: this session is estimated to spend
    /// `estimated_cost` of `metered`, and `allowance` is what the account has left of it.
    #[must_use]
    pub const fn read(metered: Metered, estimated_cost: u64, allowance: Allowance) -> Self {
        Self {
            metered,
            estimated_cost,
            allowance: Ok(allowance),
        }
    }
    /// A budget whose allowance the API did not answer, and why.
    #[must_use]
    pub fn unread(metered: Metered, estimated_cost: u64, why: impl Into<String>) -> Self {
        Self {
            metered,
            estimated_cost,
            allowance: Err(why.into()),
        }
    }
    /// Which budget, and what it is metered in.
    #[must_use]
    pub const fn metered(&self) -> Metered {
        self.metered
    }
    /// Which budget.
    #[must_use]
    pub const fn budget(&self) -> &'static str {
        self.metered.budget()
    }
    /// What that budget is metered in.
    #[must_use]
    pub const fn unit(&self) -> &'static str {
        self.metered.unit()
    }
    /// What this session is estimated to spend against it.
    #[must_use]
    pub const fn estimated_cost(&self) -> u64 {
        self.estimated_cost
    }
    /// What the account has left of it, or why that could not be read.
    ///
    /// # Errors
    ///
    /// Carries the lane's own description of the read that was not answered.
    pub fn allowance(&self) -> Result<Allowance, &str> {
        self.allowance.as_ref().copied().map_err(String::as_str)
    }
}

/// Why a session that could have run did not: the account cannot afford it.
///
/// It is an enum of two named cases rather than a message, because a run that declined has
/// to be told from a run that failed by something reading the outcome rather than by
/// reading prose. Every figure the decision was made on is on it.
///
/// **Neither case waits.** There is no sleep, no poll and no retry here or in anything that
/// consults it: a refusal naming a rate limit while the account's own reported budget still
/// shows room is GitHub's secondary limiter, which nothing reports and every further attempt
/// extends. What a declined run does is say when the budget resets and stop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unaffordable {
    /// The API did not answer this budget's allowance, so it is not one to assume.
    Unread {
        /// Which budget went unread, and what it is metered in.
        metered: Metered,
        /// The read that was not answered, as the lane that made it describes it.
        why: String,
    },
    /// Starting would leave less of this budget than the retained buffer.
    Short {
        /// Which budget is short, and what it is metered in.
        metered: Metered,
        /// Its whole allowance.
        limit: u64,
        /// What was left of it when the allowance was read.
        remaining: u64,
        /// What this session is estimated to spend against it.
        estimated_cost: u64,
        /// What [`RETAINED_BUFFER`] of `limit` comes to.
        retained_buffer: u64,
        /// The UTC epoch second that budget's window resets.
        reset: u64,
    },
}

impl Unaffordable {
    /// Which budget refused the session, and what it is metered in.
    #[must_use]
    pub const fn metered(&self) -> Metered {
        match self {
            Self::Unread { metered, .. } | Self::Short { metered, .. } => *metered,
        }
    }
    /// Which budget refused the session.
    #[must_use]
    pub const fn budget(&self) -> &'static str {
        self.metered().budget()
    }
    /// The reason, spelled out with every figure the decision was made on.
    #[must_use]
    pub fn reason(&self) -> String {
        match self {
            Self::Unread { metered, why } => format!(
                "the {} budget's allowance could not be read, and an allowance this \
                 session could not read is not one it may assume: {why}. Re-run once that \
                 read is answered",
                metered.budget(),
            ),
            Self::Short {
                metered,
                limit,
                remaining,
                estimated_cost,
                retained_buffer,
                reset,
            } => format!(
                "the account cannot afford it against the {} budget and still keep the \
                 retained buffer. That budget's limit is {limit} {}, {remaining} remained, \
                 this session is estimated to spend {estimated_cost}, and the retained buffer \
                 is {retained_buffer} ({RETAINED_BUFFER} of the allowance) — which leaves {} \
                 where {retained_buffer} is owed. That budget resets at {reset} (UTC epoch \
                 seconds); nothing here waits for it, so re-run after that",
                metered.budget(),
                metered.unit(),
                remaining.saturating_sub(*estimated_cost),
            ),
        }
    }
}

/// Whether every budget can afford this session and still keep its retained buffer.
///
/// A session starts only when, for **every** demand, the remaining allowance minus that
/// session's estimated cost is at least [`RETAINED_BUFFER`] of that budget's **whole**
/// allowance. A budget whose allowance was not read never affords anything.
///
/// The demands are decided in the order given and the first that refuses is the one
/// reported, so a session drawing on two budgets where one is short declines naming that
/// one rather than both.
///
/// # Errors
///
/// When a budget's allowance could not be read, or when starting would dip into its
/// retained buffer.
pub fn affordable(demands: &[Demand]) -> Result<(), Unaffordable> {
    for demand in demands {
        let allowance = match demand.allowance() {
            Ok(allowance) => allowance,
            Err(why) => {
                return Err(Unaffordable::Unread {
                    metered: demand.metered,
                    why: why.to_owned(),
                });
            }
        };
        let retained_buffer = RETAINED_BUFFER.of(allowance.limit());
        // Saturating rather than checked: a session estimated to cost more than the whole
        // remainder leaves nothing, which is under any buffer, and that is the answer.
        let left = allowance.remaining().saturating_sub(demand.estimated_cost);
        if left < retained_buffer {
            return Err(Unaffordable::Short {
                metered: demand.metered,
                limit: allowance.limit(),
                remaining: allowance.remaining(),
                estimated_cost: demand.estimated_cost,
                retained_buffer,
                reset: allowance.reset(),
            });
        }
    }
    Ok(())
}

/// A session that could have run and did not, and the reason no test result covers it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declined {
    session: String,
    reason: String,
    // llmlint: ignore[invalid_states_unrepresentable] `None` is not a missing cause: it is
    // every decline this crate makes for a reason that is not a budget — the seat another
    // instance holds, and a seat directory nothing can write to — and those carry their
    // whole reason in the line above. A cause enum with a variant per precondition would
    // put the seat's prose into a shape nothing reads structurally, to remove a state that
    // is meaningful. Boxed because `Declined` is the `Err` of `Session::open`, and an
    // enum carrying seven figures inline would make every ordinary `Ok` that size too.
    cause: Option<Box<Unaffordable>>,
}

impl Declined {
    /// A session refused because the account cannot afford it.
    ///
    /// The one way to build one from outside this crate, and it takes the decision rather
    /// than a sentence: what makes a declined run tellable from a failed one is that the
    /// budget, its limit, what remained, the estimate, the buffer and the reset are on the
    /// value, and [`Declined::unaffordable_because`] hands them back without anybody
    /// parsing the message.
    #[must_use]
    pub fn unaffordable(session: &str, cause: Unaffordable) -> Self {
        Self {
            session: session.to_owned(),
            reason: cause.reason(),
            cause: Some(Box::new(cause)),
        }
    }

    /// What this session is called.
    #[must_use]
    pub fn session(&self) -> &str {
        &self.session
    }

    /// The budget decision that refused it, when a budget is what refused it.
    ///
    /// `None` for every other precondition — a seat another instance holds, a seat
    /// directory nothing can write to — whose whole reason is in [`Declined::message`].
    #[must_use]
    pub fn unaffordable_because(&self) -> Option<&Unaffordable> {
        self.cause.as_deref()
    }

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
            cause: None,
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
    /// What this run wrote into the file it created, which is what tells that file from the
    /// next one at the same path.
    ///
    /// [`Seat::drop`] is the one reader and why it needs one is written there. A whole
    /// number of the process, the thread and this process's own count of seats taken,
    /// rather than a modification time: two files created either side of a reclaim can share
    /// a timestamp on a filesystem whose resolution is coarse, and cannot share this.
    ///
    /// `None` when the write did not land — the one case the drop falls back to removing
    /// unconditionally, for the reason given there.
    token: Option<String>,
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
        Self::stale_at(path).is_some()
    }

    /// When the file at `path` was last touched, if it is there and stale.
    ///
    /// The number is nanoseconds since the epoch, and it is what tells one stale file from
    /// the next one at the same path: creating a file moves its modification time, so a
    /// reading of a file that has since been replaced no longer matches what is there. That
    /// is what [`Seat::hold`] elects on, so a run acting on an out-of-date reading is
    /// refused rather than removing a file another run has just created.
    fn stale_at(path: &Path) -> Option<u128> {
        let modified = fs::metadata(path)
            .and_then(|metadata| metadata.modified())
            .ok()?;
        let age = SystemTime::now().duration_since(modified).ok()?;
        if age <= SEAT_IS_STALE_AFTER {
            return None;
        }
        Some(
            modified
                .duration_since(SystemTime::UNIX_EPOCH)
                .ok()?
                .as_nanos(),
        )
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
    ///
    /// **Recovering that stale claim is elected, and the election is named for the reading
    /// it was made on.** Delete-then-create loses here the same way it loses one level up,
    /// and not narrowly: two runs that read one stale claim each remove what the other has
    /// just created and each believe they hold it, which puts two runs into `reclaim` at
    /// once and undoes the whole of what the claim is for.
    ///
    /// So a run may only remove the claim while holding [`Seat::elect`]'s file for that
    /// claim's own modification time, and only while the claim still carries it. Creating a
    /// file moves that time, so a run whose reading has gone out of date competes for a
    /// path nobody else wants, finds the claim is no longer the one it read, and is refused
    /// — rather than removing a claim the run that owns it has just created. Nothing here
    /// ever empties the claim's path for another run to walk into.
    fn hold(claim: &Path) -> std::io::Result<()> {
        // Not a `Seat`: that type releases its file on drop, and this one has to outlive the
        // reclaim it guards. `reclaim` removes it on every path out of itself instead.
        let create = || Self::create_only(claim);
        match create() {
            Ok(()) => Ok(()),
            Err(taken) if taken.kind() == ErrorKind::AlreadyExists => {
                let Some(stale_at) = Self::stale_at(claim) else {
                    return Err(taken);
                };
                let election = claim.with_extension(format!("recover-{stale_at}"));
                Self::elect(&election)?;
                let held = if Self::stale_at(claim) == Some(stale_at) {
                    let _ = fs::remove_file(claim);
                    create()
                } else {
                    // The claim was replaced while this run was deciding, so what is there
                    // is somebody else's and removing it is the one thing this must not do.
                    Err(taken)
                };
                let _ = fs::remove_file(&election);
                held
            }
            Err(problem) => Err(problem),
        }
    }

    /// Win the election at `path`, or fail because another run holds it.
    ///
    /// An election a killed run left behind goes stale on the same clock everything else
    /// here does, so one process dying inside a recovery does not stop every later run
    /// recovering that claim. It is carried away rather than deleted, because a move cannot
    /// be made twice — a rename whose source is already gone fails — so of any number of
    /// runs finding it stale exactly one gets to stand again, where a delete would let them
    /// all.
    fn elect(path: &Path) -> std::io::Result<()> {
        match Self::create_only(path) {
            Ok(()) => Ok(()),
            Err(taken) if taken.kind() == ErrorKind::AlreadyExists && Self::is_stale(path) => {
                let aside = path.with_extension(format!(
                    "abandoned-{}-{:?}",
                    process::id(),
                    thread::current().id()
                ));
                fs::rename(path, &aside)?;
                let _ = fs::remove_file(&aside);
                Self::create_only(path)
            }
            Err(problem) => Err(problem),
        }
    }

    /// Create `path`, failing when something is already there.
    fn create_only(path: &Path) -> std::io::Result<()> {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map(drop)
    }

    /// Create the seat file, failing when one is already there.
    fn create(path: &Path) -> std::io::Result<Self> {
        let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
        // Who holds it, for the reader of a refusal that names this file — and, in the same
        // line, which run holds it, for [`Seat::drop`]. Best effort: the seat is the file's
        // existence, not its contents, so a failed write must not hand the seat to a second
        // run.
        let token = format!(
            "held by process {} thread {:?} seat {}",
            process::id(),
            thread::current().id(),
            SEATS_TAKEN.fetch_add(1, Ordering::Relaxed)
        );
        Ok(Self {
            path: path.to_owned(),
            token: writeln!(file, "{token}").ok().map(|()| token),
        })
    }
}

impl Drop for Seat {
    fn drop(&mut self) {
        // **Only the file this run took.** A session that outlives `SEAT_IS_STALE_AFTER` has
        // its seat reclaimed by a later run, which removes this one's file and creates its
        // own at the same path — and an unconditional remove here would then delete *that*
        // run's seat and let a third run in beside it, which is the two concurrent sessions
        // against one shared fixture the seat exists to prevent. So the file there is this
        // run's only while it still carries what this run wrote into it, which is the same
        // identity test `hold` makes of a claim one level down.
        let ours = match &self.token {
            // Nothing was written, so this cannot tell the two apart. It removes, because
            // the failure that leaves is the worse one: a seat nobody releases declines
            // every run on this machine until it goes stale an hour later, where the case
            // above needs a session to have already run for that long.
            None => true,
            Some(token) => {
                fs::read_to_string(&self.path).is_ok_and(|held| held.trim_end() == token)
            }
        };
        // Best effort by construction: a seat nobody could remove is reclaimed by the next
        // run as an interrupted one's, which is the case this ignores in favour of.
        if ours {
            let _ = fs::remove_file(&self.path);
        }
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
        age_out_of_the_window(path);
    }

    /// Move a file's modification time outside the window any session lasts.
    ///
    /// What moves in a real run that outlives the window is the clock, not the file; moving
    /// the file's own time back is how a test reaches the same comparison without waiting an
    /// hour. It leaves the contents alone, so a seat aged this way still says which run
    /// wrote it.
    ///
    /// **The handle is opened for writing, and on Windows that is what makes this work at
    /// all.** `SetFileTime` needs `FILE_WRITE_ATTRIBUTES`, which a read-only handle does not
    /// carry, so `File::open` — read access alone — makes `set_modified` fail there with
    /// `Access is denied.` (os error 5) while succeeding on Linux and macOS, whose
    /// `futimens` accepts any descriptor the caller may write through. That is a
    /// platform difference in the *helper*, not in the seat logic, and it failed all seven
    /// seat-lease tests on `check (windows-latest)` while every one of them passed on the
    /// other two. `write(true)` alone is the fix: it adds no `create` and no `truncate`, so
    /// the seat's contents — the run id a reclaim reads — are exactly what they were.
    ///
    /// What makes a write handle safe to ask for here is that nothing holds one: `Seat`
    /// stores a path and a token, and [`Seat::create`] drops the handle it wrote through
    /// before returning. A Windows open for writing against a file some other handle has
    /// open without `FILE_SHARE_WRITE` is refused as a sharing violation, so a `Seat` that
    /// kept its file open would trade this platform's `Access is denied.` for that one —
    /// and this helper, not the seat logic, is where that would surface.
    fn age_out_of_the_window(path: &Path) {
        let long_ago = SystemTime::now() - SEAT_IS_STALE_AFTER - Duration::from_secs(60);
        fs::OpenOptions::new()
            .write(true)
            .open(path)
            .expect("the file opens for writing")
            .set_modified(long_ago)
            .expect("its modification time is settable");
        // Read the time back rather than trusting the call: a platform that accepted the
        // request and moved nothing would leave every test below asserting a reclaim
        // against a seat still inside the window, which reads as the seat logic being
        // wrong rather than as this helper having done nothing.
        let aged = fs::metadata(path)
            .expect("the aged file is readable")
            .modified()
            .expect("its modification time is readable");
        assert!(
            aged.elapsed().expect("the aged time is in the past") > SEAT_IS_STALE_AFTER,
            "ageing left the seat inside the stale window, so nothing below tests a reclaim"
        );
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
    fn a_displaced_holder_does_not_release_the_seat_the_run_that_replaced_it_holds() {
        // A session that outlives the stale window has its seat reclaimed by a later run,
        // and then finishes. What it must not do on the way out is take the replacement's
        // seat with it: that would let a third run open beside a live one, which is the two
        // concurrent sessions against one shared fixture the seat exists to prevent.
        let directory = scratch();
        let displaced = Session::open_in(&directory, "GitHub Projects", credential("live-token"))
            .expect("the first instance takes the seat");
        let seat = displaced.seat_path().to_owned();
        age_out_of_the_window(&seat);

        let replacement = Session::open_in(&directory, "GitHub Projects", credential("live-token"))
            .expect("a seat older than any session lasts is reclaimed");
        assert_eq!(replacement.seat_path(), seat);
        let taken_by_the_replacement =
            fs::read_to_string(&seat).expect("the reclaimed seat is readable");

        drop(displaced);

        assert!(
            seat.exists(),
            "the displaced run released a seat that was no longer its own"
        );
        assert_eq!(
            fs::read_to_string(&seat).expect("the reclaimed seat is still readable"),
            taken_by_the_replacement,
            "the seat there is the replacement's, and nothing this run did replaced it"
        );
        let declined = Session::open_in(&directory, "GitHub Projects", credential("live-token"))
            .expect_err("a third instance must be declined while the replacement runs");
        assert!(
            declined.message().contains("already running"),
            "{}",
            declined.message()
        );

        // And the replacement's own release is untouched by any of it: it wrote what is
        // there, so it is the run that takes it away.
        drop(replacement);
        assert!(!seat.exists(), "the run that took the seat releases it");
    }

    #[test]
    fn a_holder_that_could_not_write_its_token_still_releases_the_seat_it_took() {
        // `Seat::create` takes the seat by creating the file and then writes what tells this
        // run's file from the next one at that path. The write is best effort — the seat is
        // the file's existence, not its contents — so a run whose write did not land holds a
        // seat it cannot identify, which is the `None` token this drives.
        //
        // The choice made there is that such a run removes anyway, and this is the reason:
        // a seat nobody releases declines every run on the machine until it goes stale an
        // hour later. So the whole of the tokenless drop is here — what it buys, and what it
        // costs — rather than only the half that reads well.
        let directory = scratch();
        let path = directory.join("onetaskgraph-live-github-projects.seat");
        Seat::create_only(&path).expect("the seat file is creatable");
        let tokenless = Seat {
            path: path.clone(),
            token: None,
        };
        drop(tokenless);
        assert!(
            !path.exists(),
            "a run that could not write its token left a seat nobody releases"
        );

        // What it buys: the next run opens instead of being declined for an hour.
        let next = Session::open_in(&directory, "GitHub Projects", credential("live-token"))
            .expect("the seat a tokenless holder released is free");
        assert_eq!(next.seat_path(), path);

        // And what it costs, stated rather than assumed: a tokenless holder cannot tell this
        // run's file from a replacement's, so a second one dropping now takes the live
        // session's seat with it. That is the trade the comment there makes — the worse
        // failure is the seat nobody releases — and it is pinned here so changing the trade
        // means changing this test rather than discovering it later.
        drop(Seat {
            path: path.clone(),
            token: None,
        });
        assert!(
            !path.exists(),
            "a tokenless holder is documented as removing whatever is at its path"
        );
        // The session itself is unharmed and its own drop is still best effort.
        assert_eq!(next.credential().expose(), "live-token");
        drop(next);
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
    fn an_election_an_interrupted_recovery_left_does_not_decline_every_later_run_for_ever() {
        // One level further down than the test above: a run killed inside the recovery of a
        // stale claim leaves the election behind too. That must not become the file nobody
        // can ever get past — the claim under it would then be unrecoverable for ever, which
        // is the failure staleness exists to prevent, one level in.
        let directory = scratch();
        let path = directory.join("onetaskgraph-live-linear.seat");
        let claim = path.with_extension("reclaim");
        interrupted_runs_file(&path);
        interrupted_runs_file(&claim);
        let election = claim.with_extension(format!(
            "recover-{}",
            Seat::stale_at(&claim).expect("the planted claim really is stale")
        ));
        interrupted_runs_file(&election);

        let reclaimed = Session::open_in(&directory, "Linear", credential("live-key"))
            .expect("an election older than any session lasts is an interrupted recovery's");
        assert_eq!(reclaimed.seat_path(), path);
        assert!(
            !election.exists(),
            "a finished recovery leaves no election for the next run to stand against"
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
    fn two_runs_recovering_one_interrupted_reclaims_claim_leave_exactly_one_holder() {
        // The race one level down from the test above, and the one a delete-then-create
        // recovery loses every time rather than narrowly: a run killed midway through a
        // reclaim leaves BOTH a stale seat and a stale claim, and every run that then reads
        // that claim as stale would remove it and create its own — so each removes the
        // other's, each takes the claim, and each goes on to reclaim the seat. Two sessions
        // against one shared external fixture is exactly what the claim exists to prevent,
        // so the claim's own recovery has to be as exclusive as the claim is.
        //
        // Rounds, rather than one, because the two halves of a delete-then-create recovery
        // are microseconds apart, so a round of this only sometimes lands the interleaving:
        // against a delete-then-create recovery one round in forty or so ends with two
        // holders, and a test of one round would pass while the defect was there. Election
        // is exclusive by the filesystem rather than by timing, so it holds every round and
        // costs a fifth of a second; two hundred rounds is what makes the failing direction
        // a witness rather than a coin toss.
        let directory = scratch();
        let path = directory.join("onetaskgraph-live-github-projects.seat");
        let claim = path.with_extension("reclaim");
        for round in 1..=200 {
            interrupted_runs_file(&path);
            interrupted_runs_file(&claim);

            let start = Arc::new(Barrier::new(32));
            let holders: Vec<_> = (0..32)
                .map(|_| {
                    let directory = directory.clone();
                    let start = Arc::clone(&start);
                    std::thread::spawn(move || {
                        start.wait();
                        Session::open_in(&directory, "GitHub Projects", credential("live-token"))
                            .ok()
                    })
                })
                .collect();
            let held: Vec<_> = holders
                .into_iter()
                .filter_map(|thread| thread.join().expect("no recovering thread panicked"))
                .collect();
            assert_eq!(
                held.len(),
                1,
                "round {round}: {} runs recovered one interrupted reclaim's claim at once, \
                 so that many sessions would have written to the same shared fixture",
                held.len()
            );
            assert!(
                held[0].seat_path().exists(),
                "round {round}: the winner really holds a seat"
            );
            assert!(
                !claim.exists(),
                "round {round}: a finished reclaim leaves no claim for the next run to wait \
                 behind"
            );
        }
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
            cause: None,
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

    /// The budget the two lanes' own units are spelled after, for the tests below.
    const GRAPHQL: Metered = Metered::new("graphql", "points");
    /// The other one a session drawing on both meters separately.
    const REST: Metered = Metered::new("rest", "requests");

    fn graphql(estimated_cost: u64, limit: u64, remaining: u64) -> Demand {
        Demand::read(
            GRAPHQL,
            estimated_cost,
            Allowance::read(limit, remaining, 1_775_000_000)
                .expect("this test's allowance leaves no more than the whole of it"),
        )
    }

    #[test]
    fn the_retained_buffer_is_one_share_of_the_allowance_for_every_budget() {
        assert_eq!(RETAINED_BUFFER.numerator(), 20);
        assert_eq!(RETAINED_BUFFER.denominator(), 100);
        assert_eq!(RETAINED_BUFFER.to_string(), "20/100");
        // Of the allowance, on each budget's own scale, and rounded up so the share held
        // back is never narrower than the one this repository states.
        assert_eq!(RETAINED_BUFFER.of(5_000), 1_000);
        assert_eq!(RETAINED_BUFFER.of(30), 6);
        assert_eq!(RETAINED_BUFFER.of(1), 1);
        assert_eq!(RETAINED_BUFFER.of(0), 0);
        // A whole near the top of the range must not overflow into a buffer of nearly
        // nothing, which is the arithmetic failure a gate would never notice.
        assert_eq!(RETAINED_BUFFER.of(u64::MAX), u64::MAX / 5);
    }

    #[test]
    fn a_budget_with_room_for_the_session_and_the_buffer_starts() {
        // 5,000 allowance, 1,000 buffer, 1,932 estimated: 3,000 remaining leaves 1,068.
        assert_eq!(affordable(&[graphql(1_932, 5_000, 3_000)]), Ok(()));
        // Exactly the buffer is still affordable: what is retained is what may not be
        // *dipped into*, and landing on it does not.
        assert_eq!(affordable(&[graphql(1_932, 5_000, 2_932)]), Ok(()));
        assert_eq!(affordable(&[]), Ok(()));
    }

    #[test]
    fn a_budget_whose_remainder_would_dip_into_the_buffer_does_not_start() {
        let short = affordable(&[graphql(1_932, 5_000, 2_931)])
            .expect_err("one point under the buffer is under the buffer");
        assert_eq!(short.budget(), "graphql");
        let Unaffordable::Short {
            metered,
            limit,
            remaining,
            estimated_cost,
            retained_buffer,
            reset,
            ..
        } = &short
        else {
            panic!("a budget with a read allowance is short rather than unread: {short:?}");
        };
        assert_eq!(
            (
                metered.unit(),
                *limit,
                *remaining,
                *estimated_cost,
                *retained_buffer,
                *reset
            ),
            ("points", 5_000, 2_931, 1_932, 1_000, 1_775_000_000)
        );
        // Twenty per cent of the ALLOWANCE and not of what is left: on what remains, the
        // buffer would be 587 and this session would fit.
        assert!(RETAINED_BUFFER.of(*remaining) < *retained_buffer);
        let reason = short.reason();
        for figure in [
            "graphql",
            "points",
            "5000",
            "2931",
            "1932",
            "1000",
            "1775000000",
        ] {
            assert!(reason.contains(figure), "{figure} is missing from {reason}");
        }
        assert!(!reason.contains("wait for"), "{reason}");
    }

    #[test]
    fn an_estimate_larger_than_the_whole_remainder_does_not_start_rather_than_wrapping() {
        let short = affordable(&[graphql(9_999, 5_000, 10)])
            .expect_err("a session costing more than is left leaves nothing");
        assert!(
            short.reason().contains("which leaves 0"),
            "{}",
            short.reason()
        );
    }

    #[test]
    fn two_budgets_where_one_is_short_decline_naming_that_one() {
        let rest = Demand::read(
            REST,
            5,
            Allowance::read(5_000, 4_999, 1_775_000_060).expect("4,999 is under 5,000"),
        );
        let short = affordable(&[graphql(1_932, 5_000, 2_000), rest.clone()])
            .expect_err("the graphql budget cannot afford this session");
        assert_eq!(short.budget(), "graphql");
        assert_eq!(affordable(&[rest]), Ok(()));
    }

    #[test]
    fn an_allowance_reporting_more_left_than_it_holds_cannot_be_made_at_all() {
        assert_eq!(Allowance::read(5_000, 5_001, 1), None);
        assert_eq!(
            Allowance::read(5_000, 5_000, 1).map(Allowance::remaining),
            Some(5_000),
            "an untouched allowance is not an impossible one"
        );
        assert_eq!(Allowance::read(0, 0, 1).map(Allowance::limit), Some(0));
    }

    #[test]
    fn an_allowance_the_session_could_not_read_is_not_one_it_may_assume() {
        let unread = affordable(&[Demand::unread(
            REST,
            5,
            "GET /rate_limit failed with HTTP 503",
        )])
        .expect_err("an unknown budget is not an affordable one");
        assert_eq!(
            unread,
            Unaffordable::Unread {
                metered: REST,
                why: "GET /rate_limit failed with HTTP 503".to_owned(),
            }
        );
        assert!(unread.reason().contains("GET /rate_limit"), "{unread:?}");
    }

    #[test]
    fn a_session_the_account_cannot_afford_declines_rather_than_failing_or_passing() {
        let short = affordable(&[graphql(1_932, 5_000, 2_000)]).expect_err("2,000 is short");
        let declined = Declined::unaffordable("GitHub Projects", short.clone());
        assert_eq!(declined.session(), "GitHub Projects");
        // Read as a value rather than out of the prose: that is what tells a run that did
        // not happen from a run that failed, for something reading the outcome.
        assert_eq!(declined.unaffordable_because(), Some(&short));
        let message = declined.message();
        assert!(message.contains("DID NOT RUN"), "{message}");
        assert!(
            message.contains("not a test failure in the code under test"),
            "{message}"
        );
        assert!(message.contains(&short.reason()), "{message}");
    }

    #[test]
    fn a_decline_for_any_other_reason_carries_no_budget_to_read() {
        let directory = scratch();
        let _held = Session::open_in(&directory, "Linear", credential("live-key"))
            .expect("the first instance takes the seat");
        let declined = Session::open_in(&directory, "Linear", credential("live-key"))
            .expect_err("a second instance is declined");
        assert_eq!(declined.unaffordable_because(), None);
    }

    #[test]
    fn a_session_name_becomes_one_readable_file_name() {
        assert_eq!(slug("GitHub Projects"), "github-projects");
        assert_eq!(slug("Linear"), "linear");
        assert_eq!(slug("  //  "), "session");
        assert_eq!(slug("a  b"), "a-b");
    }
}
