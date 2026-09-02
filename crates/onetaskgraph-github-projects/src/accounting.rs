//! What a session of requests to GitHub cost, counted rather than argued about.
//!
//! Nothing here decides anything: it records what left this crate and adds it up. The
//! reason it exists is that a query strategy cannot be chosen between without measuring
//! what each one costs, and the change that was supposed to make reads cheaper was
//! reasoned about instead of counted — and made them ten times more expensive.
//!
//! # What one record carries
//!
//! One [`Request`] per outgoing HTTP request. A GraphQL request is named by the document
//! it sent, read out of [`graphql::DOCUMENTS`] rather than from
//! a second list of names, and carries that document's worst-case node count under the
//! bindings that request actually sent — [`node_count`], the same
//! offline calculation `tests/node_count.rs` holds every document to, never a second
//! arithmetic. A REST request sends no document and has no node count, so it names the
//! endpoint it addressed instead. Both record whether they read or wrote, how they ended,
//! and the rate-limit facts that response's own headers carried.
//!
//! **Two quantities of GitHub's, kept apart by name.** `nodeCount` is the most nodes *one
//! query may return*, checked per query; it is what [`Call::Document`] carries.
//! `cost` is rate-limit points, metered per hour across everything one credential does; it
//! is what [`Spend`] is in. A document well under the node limit says nothing about the
//! second.
//!
//! # How a session's spend is arrived at, and what it is not
//!
//! Per budget, accumulated per call, from whatever that call itself makes attributable:
//!
//! - **[`Budget::Rest`] is metered in requests,** so a call is its own measure and is
//!   attributed one request ([`Basis::Counted`]).
//! - **[`Budget::Graphql`] is metered in points.** Where the request was shaped so GitHub
//!   reports its own `cost` — a document selecting `rateLimit { cost }` — that is what is
//!   attributed ([`Basis::Reported`]). Otherwise this repository's stated cost model
//!   applies: **GitHub charges at least one point for any call, so one point is
//!   attributed** ([`Basis::Modelled`]), and the report says how much of the total came
//!   that way. That is a **lower bound** and the accounting says so rather than implying a
//!   measurement: a call over a large connection really costs more, and no document this
//!   source sends today asks GitHub what.
//! - **A rate-limited refusal is attributed nothing** ([`Basis::NotRun`]), because a
//!   request GitHub refused for a rate limit did not run — the same reading of a refusal
//!   that makes retrying one safe in [`GitHubProjectsSource::graphql`](crate::GitHubProjectsSource).
//!
//! **What a session spent is never inferred by differencing a shared counter.** This
//! account is shared and rate-limited, and other work draws on the same budgets in the same
//! window, so an allowance that fell by sixty while this session made ten calls measures
//! the account rather than the session. The report gives that movement anyway — it is worth
//! seeing — and says on its face that it is the account's and not this session's.
//!
//! # Where a reader finds the report
//!
//! [`Session::report`] renders one from a snapshot, and the credentialed lane in
//! `tests/live.rs` prints it at the end of every run, passed or failed — from a `Drop`, so
//! that the run whose cost is most worth reading, the one that broke, is not the run that
//! skips it. It carries no credential, no token, no issue body and no board content: a call
//! is named by a document description this crate wrote or by an endpoint the caller spelled,
//! and everything else in it is a number.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::Mutex;

use reqwest::StatusCode;
use serde_json::Value;

use crate::{Limiter, Variables, graphql, largest_page_sizes, node_count};

/// Which of GitHub's two separately metered budgets a request drew on.
///
/// They are two because GitHub meters them separately, and a single total would hide which
/// one is close to exhaustion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Budget {
    /// The GraphQL API, metered in points.
    Graphql,
    /// The REST API, metered in requests.
    Rest,
}

impl Budget {
    /// This budget's name, as GitHub's own `x-ratelimit-resource` header spells it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Graphql => "graphql",
            Self::Rest => "rest",
        }
    }
    /// What GitHub meters this budget in.
    #[must_use]
    pub const fn unit(self) -> &'static str {
        match self {
            Self::Graphql => "points",
            Self::Rest => "requests",
        }
    }
}

/// Whether a request read or wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Mode {
    /// It asked for something.
    Read,
    /// It changed something.
    Write,
}

impl Mode {
    /// What a GraphQL document does.
    ///
    /// Every mutation this source sends creates content and no query does, so the keyword
    /// is the whole of the question — the same test [`crate::GitHubProjectsSource`] paces
    /// its own writes by.
    #[must_use]
    pub fn of_document(document: &str) -> Self {
        if crate::is_mutation(document) {
            Self::Write
        } else {
            Self::Read
        }
    }
    /// What an HTTP method does. Anything that is not a plain retrieval writes.
    #[must_use]
    pub fn of_method(method: &str) -> Self {
        match method.trim().to_ascii_uppercase().as_str() {
            "GET" | "HEAD" | "OPTIONS" => Self::Read,
            _ => Self::Write,
        }
    }
    /// This mode's name in a report.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
        }
    }
}

/// How one request ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Outcome {
    /// GitHub answered with what was asked for.
    Answered,
    /// GitHub did not answer it, for a reason that is not a rate limit — including a
    /// request that never reached GitHub at all, which carries no headers to read.
    Refused,
    /// A rate limiter refused it, so it never ran.
    RateLimited,
}

impl Outcome {
    /// How a response ended, read the way this source's own limiter reads one.
    ///
    /// `budget_exhausted` is whether `x-ratelimit-remaining` was exactly `0`, which
    /// *explains* a failing response rather than making a successful one fail. A GraphQL
    /// success carrying `errors` is a refusal only its caller can rule on, so this answers
    /// [`Outcome::Answered`] for one and the caller narrows it.
    #[must_use]
    pub fn of_response(status: u16, budget_exhausted: bool, body: &str) -> Self {
        let status = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        if Limiter::classify(status, budget_exhausted, body).is_some() {
            return Self::RateLimited;
        }
        if status.is_success() {
            Self::Answered
        } else {
            Self::Refused
        }
    }
    /// This outcome's name in a report.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Answered => "answered",
            Self::Refused => "refused",
            Self::RateLimited => "rate-limited",
        }
    }
}

/// The rate-limit facts one response's own headers carried.
///
/// Every field is optional because every one of them is absent from some real response: a
/// request that never reached GitHub has no headers at all, and a refusal from an
/// intermediary carries whichever of them that intermediary felt like carrying. An absent
/// figure is reported as unknown rather than guessed at.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RateLimit {
    /// `x-ratelimit-limit`: the whole allowance for this budget's window.
    pub limit: Option<u64>,
    /// `x-ratelimit-remaining`: what was left of it when GitHub answered.
    pub remaining: Option<u64>,
    /// `x-ratelimit-used`: what the *account* had spent, which is not what this session
    /// spent — other work shares the budget.
    pub used: Option<u64>,
    /// `x-ratelimit-reset`: the Unix second the allowance comes back.
    pub reset: Option<u64>,
    /// `x-ratelimit-resource`: which budget GitHub says these figures are about.
    pub resource: Option<String>,
}

impl RateLimit {
    /// Read the five headers GitHub carries a budget's state in.
    ///
    /// `header` is given a lower-case header name and answers that response's value for
    /// it. Taking a lookup rather than a header map is what keeps the HTTP client this
    /// crate happens to use out of its public interface, so a caller sending its own
    /// requests with its own client records into the same accounting.
    #[must_use]
    pub fn read(header: impl Fn(&str) -> Option<String>) -> Self {
        let number = |name: &str| {
            header(name)
                .as_deref()
                .map(str::trim)
                .and_then(|value| value.parse::<u64>().ok())
        };
        Self {
            limit: number("x-ratelimit-limit"),
            remaining: number("x-ratelimit-remaining"),
            used: number("x-ratelimit-used"),
            reset: number("x-ratelimit-reset"),
            resource: header("x-ratelimit-resource"),
        }
    }
    /// Whether these headers say the budget is exactly spent, which is what
    /// [`Outcome::of_response`] reads.
    #[must_use]
    pub fn exhausted(&self) -> bool {
        self.remaining == Some(0)
    }
}

/// Where one call's attributed spend came from.
///
/// It is on the record rather than folded into the number so a report can say how much of
/// a session's total is GitHub's own figure and how much is this repository's model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Basis {
    /// The budget is metered in requests, so the call is its own measure.
    Counted,
    /// GitHub reported this call's own cost, because the request asked it to.
    Reported,
    /// This repository's stated cost model: one point, GitHub's documented minimum for any
    /// call, which is a lower bound rather than a measurement.
    Modelled,
    /// A rate limiter refused the request, so it never ran and is attributed nothing.
    NotRun,
}

impl Basis {
    /// This basis in a report, as a phrase that says what the figure is worth.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Counted => "counted",
            Self::Reported => "reported by GitHub",
            Self::Modelled => "this repository's one-point-per-call lower bound",
            Self::NotRun => "not run",
        }
    }
}

/// What one call is attributed against its budget, and where that figure came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Spend {
    /// The amount, in that budget's own unit.
    pub amount: u64,
    /// What makes it that amount.
    pub basis: Basis,
}

/// What one request asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Call {
    /// A GraphQL request, which sends a document.
    Document {
        /// What the sender was doing, from
        /// [`graphql::DOCUMENTS`] when the document is one of
        /// this source's own, and from the caller's own name when it is not.
        name: String,
        /// The document's worst-case node count under the bindings that request sent, or
        /// `None` when the calculation could not rule on the document — which is a defect
        /// in the document rather than a cost of zero.
        node_count: Option<u64>,
    },
    /// A REST request, which sends no document and has no node count.
    Endpoint {
        /// The endpoint it addressed, spelled the way GitHub's own documentation spells
        /// one — `GET /repos/{owner}/{repo}/labels` — so two runs compare line for line
        /// and no board content reaches the report.
        endpoint: String,
    },
}

impl Call {
    /// What this call is called in a report.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Document { name, .. } => name,
            Self::Endpoint { endpoint } => endpoint,
        }
    }
    /// The budget it draws on.
    #[must_use]
    pub const fn budget(&self) -> Budget {
        match self {
            Self::Document { .. } => Budget::Graphql,
            Self::Endpoint { .. } => Budget::Rest,
        }
    }
    /// The most nodes it may return, for a GraphQL request that has such a number.
    #[must_use]
    pub const fn node_count(&self) -> Option<u64> {
        match self {
            Self::Document { node_count, .. } => *node_count,
            Self::Endpoint { .. } => None,
        }
    }
}

/// One outgoing HTTP request, described before it is sent.
///
/// [`Sending::finished`], and [`Sending::answered`] for the ordinary case, are what turn it
/// into a [`Request`]: a record with no outcome is one nobody could add up, so the outcome
/// is the step that produces the record rather than a field that might be missing.
///
/// Everything a request carries before it is sent is settled by the constructor that made
/// it, which is why there is no builder step here that could attach a reported GraphQL cost
/// to a REST call — a budget metered in requests has no such figure, and GitHub reports
/// none for one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sending {
    call: Call,
    mode: Mode,
    drawn: Drawn,
}

/// What a request draws on, and what only that kind of request can carry.
///
/// The reported cost lives inside the GraphQL arm rather than beside the call, because a
/// REST request has nowhere to have got one from: GitHub meters that budget in requests and
/// reports no per-call figure at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Drawn {
    /// The GraphQL budget, with GitHub's own reported cost for this call when the request
    /// was shaped so GitHub reported one.
    Graphql { reported_cost: Option<u64> },
    /// The REST budget, which a call is its own measure of.
    Rest,
}

impl Sending {
    /// GitHub answered it, with the rate-limit facts its response's headers carried.
    #[must_use]
    pub fn answered(self, limits: RateLimit) -> Request {
        self.finished(Outcome::Answered, limits)
    }
    /// However it ended, with the rate-limit facts its response's headers carried.
    #[must_use]
    pub fn finished(self, outcome: Outcome, limits: RateLimit) -> Request {
        let spend = match (self.drawn, outcome) {
            (_, Outcome::RateLimited) => Spend {
                amount: 0,
                basis: Basis::NotRun,
            },
            (Drawn::Rest, _) => Spend {
                amount: 1,
                basis: Basis::Counted,
            },
            (
                Drawn::Graphql {
                    reported_cost: Some(cost),
                },
                _,
            ) => Spend {
                amount: cost,
                basis: Basis::Reported,
            },
            (
                Drawn::Graphql {
                    reported_cost: None,
                },
                _,
            ) => Spend {
                amount: 1,
                basis: Basis::Modelled,
            },
        };
        Request {
            call: self.call,
            mode: self.mode,
            outcome,
            limits,
            spend,
        }
    }
}

/// What a GraphQL document this crate does not send is called until its sender names it.
const UNNAMED_DOCUMENT: &str = "talking to GitHub";

/// One outgoing HTTP request, and what it cost.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    call: Call,
    mode: Mode,
    outcome: Outcome,
    limits: RateLimit,
    spend: Spend,
}

impl Request {
    /// Begin recording one GraphQL request carrying `document` under `variables`.
    ///
    /// The name comes from [`graphql::DOCUMENTS`] — the
    /// inventory, not a second list — and the node count from
    /// [`node_count`] under the page sizes `variables` really binds, so
    /// a page read with a caller's smaller limit is counted at that limit rather than at
    /// the worst case. Any page-size variable the request leaves unbound keeps the largest
    /// value this source could send it, which is the worst case for exactly that document.
    ///
    /// `otherwise` names a document the inventory does not hold, which is how a caller's own
    /// calls — a schema introspection, a residue sweep — are named beside this source's;
    /// `None` leaves such a document under the placeholder, and a document the inventory
    /// does hold keeps its entry's name either way.
    ///
    /// `reported_cost` is GitHub's own `cost` for **this** call, from a response to a
    /// request shaped to report one. A `rateLimit(dryRun: true)` probe reports what some
    /// other document would cost and never what this call spent, so its figure does not
    /// belong here.
    #[must_use]
    pub fn graphql(
        document: &str,
        variables: &Value,
        otherwise: Option<&str>,
        reported_cost: Option<u64>,
    ) -> Sending {
        let name = document_name(document);
        Sending {
            call: Call::Document {
                name: match (name, otherwise) {
                    (UNNAMED_DOCUMENT, Some(otherwise)) => otherwise.to_owned(),
                    (name, _) => name.to_owned(),
                },
                node_count: node_count(document, &bindings(variables)).ok(),
            },
            mode: Mode::of_document(document),
            drawn: Drawn::Graphql { reported_cost },
        }
    }
    /// Begin recording one REST request against `endpoint`.
    ///
    /// `endpoint` is the endpoint rather than the URL that was built from it — `GET
    /// /repos/{owner}/{repo}/labels`, not the repository and label a run happened to name —
    /// because a report is compared between runs and carries no board content.
    #[must_use]
    pub fn rest(method: &str, endpoint: &str) -> Sending {
        Sending {
            call: Call::Endpoint {
                endpoint: format!("{} {endpoint}", method.trim().to_ascii_uppercase()),
            },
            mode: Mode::of_method(method),
            drawn: Drawn::Rest,
        }
    }
    /// What was called.
    #[must_use]
    pub const fn call(&self) -> &Call {
        &self.call
    }
    /// What it is called in a report.
    #[must_use]
    pub fn name(&self) -> &str {
        self.call.name()
    }
    /// Whether it read or wrote.
    #[must_use]
    pub const fn mode(&self) -> Mode {
        self.mode
    }
    /// How it ended.
    #[must_use]
    pub const fn outcome(&self) -> Outcome {
        self.outcome
    }
    /// The budget it drew on.
    #[must_use]
    pub const fn budget(&self) -> Budget {
        self.call.budget()
    }
    /// The most nodes it may return, for a GraphQL request that has such a number.
    #[must_use]
    pub const fn node_count(&self) -> Option<u64> {
        self.call.node_count()
    }
    /// What it is attributed against its budget, and where that figure came from.
    #[must_use]
    pub const fn spend(&self) -> Spend {
        self.spend
    }
    /// The rate-limit facts this response's own headers carried.
    #[must_use]
    pub const fn rate_limit(&self) -> &RateLimit {
        &self.limits
    }
}

/// What this source calls the document it just sent, from the inventory rather than a copy.
fn document_name(document: &str) -> &str {
    graphql::DOCUMENTS
        .iter()
        .find(|(known, _)| *known == document)
        .map_or(UNNAMED_DOCUMENT, |(_, doing)| *doing)
}

/// The page sizes one request really bound, over the largest this source can send.
///
/// Starting from [`largest_page_sizes`] rather than from nothing is what keeps a document
/// countable when the request leaves one of its page-size variables out: the calculation
/// refuses a `first:` it has no binding for, and the worst case is the honest answer for a
/// size nobody narrowed. A variable no `first:` references — a project number, a page
/// cursor — is ignored by the calculation, so passing every integer through costs nothing.
fn bindings(variables: &Value) -> Variables {
    let mut bindings = largest_page_sizes();
    if let Some(bound) = variables.as_object() {
        for (name, value) in bound {
            if let Some(size) = value.as_u64().and_then(|size| u32::try_from(size).ok()) {
                bindings.insert(name.clone(), size);
            }
        }
    }
    bindings
}

/// Every request one session sent, and what each cost.
///
/// It is on this crate's ordinary code path — [`crate::GitHubProjectsSource`] records into
/// one at the single place a request leaves the crate, with no environment variable, no
/// feature and no build configuration to know about, because an instrument nobody switches
/// on measures nothing. It is constructible and recordable-into from outside the crate for
/// the other half of the same reason: a caller making its own calls beside this source's —
/// the credentialed lane verifying a schema, sweeping residue, cleaning up — accounts for
/// the whole session rather than for this source's share of it.
#[derive(Debug, Default)]
pub struct Accounting {
    requests: Mutex<Vec<Request>>,
}

impl Accounting {
    /// An accounting with nothing in it.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
    /// Record one request.
    pub fn record(&self, request: Request) {
        // A poisoned lock costs the accounting, never the work: a panic elsewhere must not
        // turn measuring the session into a second failure on top of the first.
        self.requests
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(request);
    }
    /// A snapshot of what has been recorded so far: a value to hold and compare, never a
    /// live borrow of this accounting.
    #[must_use]
    pub fn snapshot(&self) -> Session {
        Session {
            requests: self
                .requests
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone(),
        }
    }
}

/// One session's requests, as a value a caller can hold, compare and report on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Session {
    requests: Vec<Request>,
}

impl Session {
    /// The requests it holds, in the order they were recorded.
    #[must_use]
    pub fn requests(&self) -> &[Request] {
        &self.requests
    }
    /// How many requests this session sent.
    #[must_use]
    pub fn total_requests(&self) -> usize {
        self.requests.len()
    }
    /// The nodes every GraphQL request that has a node count may return, added up.
    #[must_use]
    pub fn total_node_count(&self) -> u64 {
        self.requests
            .iter()
            .filter_map(Request::node_count)
            .fold(0, u64::saturating_add)
    }
    /// What this session spent against `budget`, attributed per call.
    #[must_use]
    pub fn spent(&self, budget: Budget) -> u64 {
        self.requests
            .iter()
            .filter(|request| request.budget() == budget)
            .fold(0, |total, request| {
                total.saturating_add(request.spend.amount)
            })
    }
    /// Every budget this session touched, in a stable order.
    #[must_use]
    pub fn budgets(&self) -> Vec<BudgetReport> {
        let mut touched: BTreeMap<Budget, BudgetReport> = BTreeMap::new();
        for request in &self.requests {
            let budget = request.budget();
            let report = touched.entry(budget).or_insert_with(|| BudgetReport {
                budget,
                requests: 0,
                spent: 0,
                reported: 0,
                modelled: 0,
                counted: 0,
                not_run: 0,
                limit: None,
                used_by_the_account: None,
                remaining_first_seen: None,
                remaining_last_seen: None,
            });
            report.requests += 1;
            report.spent = report.spent.saturating_add(request.spend.amount);
            match request.spend.basis {
                Basis::Reported => {
                    report.reported = report.reported.saturating_add(request.spend.amount);
                }
                Basis::Modelled => {
                    report.modelled = report.modelled.saturating_add(request.spend.amount);
                }
                Basis::Counted => {
                    report.counted = report.counted.saturating_add(request.spend.amount);
                }
                Basis::NotRun => report.not_run += 1,
            }
            if let Some(limit) = request.limits.limit {
                report.limit = Some(limit);
            }
            if let Some(used) = request.limits.used {
                report.used_by_the_account = Some(used);
            }
            if let Some(remaining) = request.limits.remaining {
                report.remaining_first_seen.get_or_insert(remaining);
                report.remaining_last_seen = Some(remaining);
            }
        }
        touched.into_values().collect()
    }
    /// The session report: what a person puts two runs of side by side.
    #[must_use]
    pub fn report(&self) -> String {
        let mut report = String::from("github-projects session accounting\n");
        let count = |mode: Mode| {
            self.requests
                .iter()
                .filter(|request| request.mode == mode)
                .count()
        };
        let ended = |outcome: Outcome| {
            self.requests
                .iter()
                .filter(|request| request.outcome == outcome)
                .count()
        };
        let _ = writeln!(
            report,
            "requests {}: {} {}, {} {}; {} {}, {} {}, {} {}",
            self.total_requests(),
            count(Mode::Read),
            Mode::Read.name(),
            count(Mode::Write),
            Mode::Write.name(),
            ended(Outcome::Answered),
            Outcome::Answered.name(),
            ended(Outcome::Refused),
            Outcome::Refused.name(),
            ended(Outcome::RateLimited),
            Outcome::RateLimited.name(),
        );
        report.push_str("requests by call\n");
        let mut by_call: BTreeMap<&str, (usize, u64, usize)> = BTreeMap::new();
        for request in &self.requests {
            let entry = by_call.entry(request.name()).or_insert((0, 0, 0));
            entry.0 += 1;
            if let Some(nodes) = request.node_count() {
                entry.1 = entry.1.saturating_add(nodes);
                entry.2 += 1;
            }
        }
        for (name, (requests, _, _)) in &by_call {
            let _ = writeln!(report, "  {name:<52}{requests:>6}");
        }
        let counted = self
            .requests
            .iter()
            .filter(|request| request.node_count().is_some())
            .count();
        let _ = writeln!(
            report,
            "node count {} over {counted} GraphQL requests that have one",
            self.total_node_count(),
        );
        report.push_str("node count by document\n");
        for (name, (_, nodes, requests)) in by_call.iter().filter(|(_, (_, _, with))| *with > 0) {
            let counted = format!(
                "{requests} {}",
                if *requests == 1 {
                    "request"
                } else {
                    "requests"
                }
            );
            let _ = writeln!(report, "  {name:<52}{counted:>14}{nodes:>12} nodes");
        }
        for budget in self.budgets() {
            report.push_str(&budget.render());
        }
        report
    }
}

/// One budget a session drew on, with its own figures kept apart from the account's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetReport {
    /// Which budget.
    pub budget: Budget,
    /// How many of this session's requests drew on it.
    pub requests: usize,
    /// What this session itself spent against it, attributed per call.
    pub spent: u64,
    /// How much of that GitHub itself reported.
    pub reported: u64,
    /// How much of it is this repository's one-point-per-call lower bound.
    pub modelled: u64,
    /// How much of it is a count of requests against a budget metered in requests.
    pub counted: u64,
    /// How many of its requests a rate limiter refused, so they never ran and are
    /// attributed nothing.
    pub not_run: usize,
    /// The whole allowance, as GitHub's own headers reported it.
    pub limit: Option<u64>,
    /// What the **account** had spent, as GitHub's own headers reported it. Not this
    /// session's spend: other work draws on the same budget in the same window.
    pub used_by_the_account: Option<u64>,
    /// The allowance remaining when this session's first request against this budget was
    /// answered.
    pub remaining_first_seen: Option<u64>,
    /// The allowance remaining when its last one was.
    pub remaining_last_seen: Option<u64>,
}

impl BudgetReport {
    /// How far the **account's** remaining allowance fell while this session ran.
    ///
    /// A fall rather than a movement, and the difference is not pedantry: an allowance that
    /// *rose* is the hourly window having reset mid-session, which is not a negative spend
    /// and answers zero here. Either way it is not this session's spend, and the report says
    /// so where it prints it — this account is shared, so the difference between two
    /// readings of a shared counter measures the account.
    #[must_use]
    pub fn account_allowance_fall(&self) -> Option<u64> {
        Some(
            self.remaining_first_seen?
                .saturating_sub(self.remaining_last_seen?),
        )
    }
    /// This budget's lines of the session report.
    #[must_use]
    pub fn render(&self) -> String {
        let unknown = |value: Option<u64>| {
            value.map_or_else(|| "not reported".to_owned(), |value| value.to_string())
        };
        let mut lines = format!(
            "budget {}, metered in {}\n",
            self.budget.name(),
            self.budget.unit()
        );
        let _ = writeln!(
            lines,
            "  this session sent {} requests and spent {} {}: {} {}, {} {}, {} {}; {} {}",
            self.requests,
            self.spent,
            self.budget.unit(),
            self.reported,
            Basis::Reported.name(),
            self.modelled,
            Basis::Modelled.name(),
            self.counted,
            Basis::Counted.name(),
            self.not_run,
            Basis::NotRun.name(),
        );
        let _ = writeln!(
            lines,
            "  the account: limit {}, {} used, {} remaining when this session finished",
            unknown(self.limit),
            unknown(self.used_by_the_account),
            unknown(self.remaining_last_seen),
        );
        let _ = writeln!(
            lines,
            "  the account's remaining allowance fell {} while this session ran; that is the \
             account's own consumption and not this session's spend, because other work \
             draws on the same budget in the same window",
            unknown(self.account_allowance_fall()),
        );
        lines
    }
}
