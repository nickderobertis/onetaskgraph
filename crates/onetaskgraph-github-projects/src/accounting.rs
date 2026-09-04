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
//! is named by a document description this crate wrote or by an [`Endpoint`], which is a
//! path template rather than the URL a run built, and everything else in it is a number.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::Mutex;

use serde_json::Value;

pub use reqwest::StatusCode;

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
    /// This mode's name in a report.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
        }
    }
}

/// The HTTP methods a REST call to GitHub is made with.
///
/// A closed set rather than a string: a record cannot then carry a method that is not one,
/// and a misspelling is refused by [`Method::parse`] where it happens rather than quietly
/// counted as a write. It is this crate's own enum rather than an HTTP client's, so which
/// client a caller sends its own requests with stays the caller's business.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Method {
    /// `GET`.
    Get,
    /// `HEAD`.
    Head,
    /// `POST`.
    Post,
    /// `PUT`.
    Put,
    /// `PATCH`.
    Patch,
    /// `DELETE`.
    Delete,
}

impl Method {
    /// The method `name` spells, whatever its case, or `None` when it spells none of them.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name.trim().to_ascii_uppercase().as_str() {
            "GET" => Self::Get,
            "HEAD" => Self::Head,
            "POST" => Self::Post,
            "PUT" => Self::Put,
            "PATCH" => Self::Patch,
            "DELETE" => Self::Delete,
            _ => return None,
        })
    }
    /// Its name, as HTTP spells it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Head => "HEAD",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Patch => "PATCH",
            Self::Delete => "DELETE",
        }
    }
    /// Whether it reads or writes.
    #[must_use]
    pub const fn mode(self) -> Mode {
        match self {
            Self::Get | Self::Head => Mode::Read,
            Self::Post | Self::Put | Self::Patch | Self::Delete => Mode::Write,
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
    ///
    /// The status is a [`StatusCode`] rather than a number, so a status HTTP has no room
    /// for cannot be asked about at all — and an HTTP client has already parsed one for
    /// every response it hands back, so nothing is asked of a caller that it did not have.
    #[must_use]
    // llmlint: ignore[invalid_states_unrepresentable] `budget_exhausted` is one header read as the yes-or-no it is — whether `x-ratelimit-remaining` was exactly `0` — so both of its values are meaningful and there is no third state a type could forbid. It is also the argument [`Limiter::classify`] already takes, on the line below, so giving this one wrapper its own spelling would mean two vocabularies for one header rather than one.
    pub fn of_response(status: StatusCode, budget_exhausted: bool, body: &str) -> Self {
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
/// Every figure is optional because every one of them is absent from some real response: a
/// request that never reached GitHub has no headers at all, and a refusal from an
/// intermediary carries whichever of them that intermediary felt like carrying. An absent
/// figure is reported as unknown rather than guessed at.
///
/// **These are observations of one response, so the only ways to have one are to observe a
/// response ([`RateLimit::read`]) or to have observed nothing ([`RateLimit::default`]).**
/// The fields are read through the accessors below rather than assembled: a hand-built set
/// could say the account had more remaining than its whole allowance, or name a resource
/// GitHub would never name, and a report built on it would be a measurement of nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RateLimit {
    /// `x-ratelimit-limit`: the whole allowance for this budget's window.
    limit: Option<u64>,
    /// `x-ratelimit-remaining`: what was left of it when GitHub answered.
    remaining: Option<u64>,
    /// `x-ratelimit-used`: what the *account* had spent, which is not what this session
    /// spent — other work shares the budget.
    used: Option<u64>,
    /// `x-ratelimit-reset`: the Unix second the allowance comes back.
    reset: Option<u64>,
    /// `x-ratelimit-resource`: which budget GitHub says these figures are about.
    resource: Option<String>,
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
        // The three allowance figures are one fact and are read as one. A response saying
        // more was left of a budget than the whole of it, or more of it used than it holds,
        // cannot be true of any account — and a report built on it would be a measurement of
        // nothing, which is what the type documentation above refuses. So a set that cannot
        // all be true is a budget state this response did not carry: dropped **together**,
        // because which of the three is the wrong one is not knowable from here, and
        // reported as unknown rather than repaired into a figure nothing observed. `reset`
        // and `resource` are independent of them and survive. `Allowance::read` in
        // `onetaskgraph-live` refuses the same impossibility where the gate reads it.
        let (limit, remaining, used) = (
            number("x-ratelimit-limit"),
            number("x-ratelimit-remaining"),
            number("x-ratelimit-used"),
        );
        let over_the_whole = |figure: Option<u64>| match (figure, limit) {
            (Some(figure), Some(limit)) => figure > limit,
            _ => false,
        };
        // And the same impossibility one step on, which neither figure shows on its own:
        // `used` and `remaining` are two views of one allowance — what the window has spent
        // and what is left of it — so between them they cannot exceed the whole. A response
        // saying 4,000 used and 4,000 left of 5,000 has each figure inside the limit and
        // still cannot be true of any account.
        //
        // **Only that direction is refused, and a sum falling short of the whole is kept
        // deliberately.** It accounts for less of the budget than exists rather than for
        // more of it than could, which no arithmetic forbids and which a response really
        // carries: the loopback board this crate is tested against answers `remaining: 0`
        // beside a `used` naming what that session itself spent, wherever it stands in for a
        // budget somebody else exhausted. Dropping those three would report the budget as
        // unknown, which is a worse answer than a short one — and it could not reach what
        // the report says this SESSION spent in any case, because that figure is attributed
        // per call and is never differenced out of these.
        let more_than_the_whole_between_them = match (used, remaining, limit) {
            (Some(used), Some(remaining), Some(limit)) => used.saturating_add(remaining) > limit,
            _ => false,
        };
        let readable = !over_the_whole(remaining)
            && !over_the_whole(used)
            && !more_than_the_whole_between_them;
        Self {
            limit: limit.filter(|_| readable),
            remaining: remaining.filter(|_| readable),
            used: used.filter(|_| readable),
            reset: number("x-ratelimit-reset"),
            // The one field here that is not a number, so the one that could carry a third
            // party's arbitrary bytes into a value this crate hands back. GitHub names these
            // `core`, `graphql`, `search`, `integration_manifest`; anything that is not
            // spelled like one is dropped rather than stored.
            resource: header("x-ratelimit-resource")
                .map(|value| value.trim().to_owned())
                .filter(|value| is_resource_name(value)),
        }
    }
    /// The whole allowance for this budget's window, as this response reported it.
    #[must_use]
    pub const fn limit(&self) -> Option<u64> {
        self.limit
    }
    /// What was left of it when GitHub answered.
    #[must_use]
    pub const fn remaining(&self) -> Option<u64> {
        self.remaining
    }
    /// What the **account** had spent — not what this session spent, because other work
    /// draws on the same budget in the same window.
    #[must_use]
    pub const fn used_by_the_account(&self) -> Option<u64> {
        self.used
    }
    /// The Unix second the allowance comes back.
    #[must_use]
    pub const fn reset(&self) -> Option<u64> {
        self.reset
    }
    /// Which budget GitHub said these figures were about, when it named one it spells.
    #[must_use]
    pub fn resource(&self) -> Option<&str> {
        self.resource.as_deref()
    }
    /// Whether these headers say the budget is exactly spent, which is what
    /// [`Outcome::of_response`] reads.
    #[must_use]
    pub fn exhausted(&self) -> bool {
        self.remaining == Some(0)
    }
}

/// Whether a value is spelled like one of GitHub's rate-limit resource names.
///
/// ASCII letters, digits, underscores and hyphens, and short. It is a shape rather than a
/// list because GitHub adds resources — `code_search` arrived after `search` — and a name
/// this does not recognise should be reported as GitHub sent it rather than dropped for
/// being new.
fn is_resource_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
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
///
/// **The amount and the basis are one fact, so they are settled together and read apart.**
/// Three of the four bases fix the amount outright — a call against a budget metered in
/// requests is one request, the model's lower bound is one point, and a request a rate
/// limiter refused never ran and is nothing — and only [`Basis::Reported`] carries a figure
/// of its own, GitHub's. Constructing the pair field by field would let a report say a call
/// GitHub never ran spent forty points, which is a measurement of nothing; the four
/// constructors below are the only ways to have one, and each is the invariant for its own
/// basis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Spend {
    amount: u64,
    basis: Basis,
}

impl Spend {
    /// One request against a budget metered in requests.
    const fn counted() -> Self {
        Self {
            amount: 1,
            basis: Basis::Counted,
        }
    }
    /// What GitHub itself reported this call cost.
    const fn reported(cost: u64) -> Self {
        Self {
            amount: cost,
            basis: Basis::Reported,
        }
    }
    /// This repository's lower bound: one point, GitHub's documented minimum for any call.
    const fn modelled() -> Self {
        Self {
            amount: 1,
            basis: Basis::Modelled,
        }
    }
    /// Nothing, because a rate limiter refused the request and it never ran.
    const fn not_run() -> Self {
        Self {
            amount: 0,
            basis: Basis::NotRun,
        }
    }
    /// The amount, in that budget's own unit.
    #[must_use]
    pub const fn amount(self) -> u64 {
        self.amount
    }
    /// What makes it that amount.
    #[must_use]
    pub const fn basis(self) -> Basis {
        self.basis
    }
}

/// A REST endpoint, spelled the way GitHub's own documentation spells one.
///
/// `GET /repos/{owner}/{repo}/labels`: a method and a path template whose segments are
/// literals or `{placeholder}`s, never the URL a run built from it. That is what makes two
/// runs' reports compare line for line, and it is why this is a type with one constructor
/// rather than a string a caller fills in — a record holding
/// `https://api.github.com/repos/octo-org/board/labels?per_page=100` would put a board's
/// name, and whatever else a query string carried, into a report that promises to carry
/// neither.
///
/// **What it rules on, and what it cannot.** [`Endpoint::parse`] refuses a host, a query
/// string, a fragment, whitespace, a byte outside the small set a path template is spelled
/// from, and anything long — every way an addressed URL differs in *shape* from a template.
/// It cannot tell a template's literal segment from a filled-in one, because
/// `/repos/octo-org/board/labels` is spelled exactly like a template of literals; keeping
/// the placeholders unfilled is the caller's, and the reason `Request::rest`'s own
/// documentation says to pass the template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    method: Method,
    path: String,
    name: String,
}

impl Endpoint {
    /// The endpoint `path` spells under `method`, or `None` when `path` is not spelled like
    /// a path template at all.
    ///
    /// Refusing it here is refusing it where it is written, which is the same answer
    /// [`Method::parse`] gives a method HTTP has no verb for: a caller learns at the call
    /// site rather than finding a URL in a report that was supposed to hold none.
    #[must_use]
    pub fn parse(method: Method, path: &str) -> Option<Self> {
        if !is_path_template(path) {
            return None;
        }
        Some(Self {
            method,
            path: path.to_owned(),
            name: format!("{} {path}", method.name()),
        })
    }
    /// The method it is addressed with.
    #[must_use]
    pub const fn method(&self) -> Method {
        self.method
    }
    /// The path template, without the method.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }
    /// What it is called in a report: the method and the template, as above.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// Whether `path` is spelled like one of GitHub's documented endpoint templates.
///
/// An absolute path of `/`-separated segments, each a literal of the bytes a documented
/// endpoint uses or a single `{placeholder}`. A host, a query string, a fragment, an empty
/// segment and anything over the length GitHub's own longest endpoint needs are all refused,
/// because each of them is a way an addressed URL — which carries what a run touched —
/// differs from the template it was built from.
fn is_path_template(path: &str) -> bool {
    if path.is_empty() || path.len() > 128 || !path.starts_with('/') {
        return false;
    }
    path[1..].split('/').all(|segment| {
        let placeholder = segment
            .strip_prefix('{')
            .and_then(|rest| rest.strip_suffix('}'));
        match placeholder {
            Some(name) => {
                !name.is_empty()
                    && name
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            }
            None => {
                !segment.is_empty()
                    && segment.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_' || byte == b'.'
                    })
            }
        }
    })
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
        /// The endpoint it addressed. An [`Endpoint`] rather than a string, so a record
        /// cannot hold the URL a run built — see that type for what it rules on.
        endpoint: Endpoint,
    },
}

impl Call {
    /// What this call is called in a report.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Document { name, .. } => name,
            Self::Endpoint { endpoint } => endpoint.name(),
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
            (_, Outcome::RateLimited) => Spend::not_run(),
            (Drawn::Rest, _) => Spend::counted(),
            (
                Drawn::Graphql {
                    reported_cost: Some(cost),
                },
                _,
            ) => Spend::reported(cost),
            (
                Drawn::Graphql {
                    reported_cost: None,
                },
                _,
            ) => Spend::modelled(),
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
                // llmlint: ignore[invalid_states_unrepresentable] `None` here is not a missing invariant, it is the honest reading of a document this calculation could not rule on — and the state is reachable, because this constructor is public and `otherwise` exists for a caller's OWN documents (a schema introspection, a residue sweep) which no test of this source's inventory sweeps. Refusing to build the record would lose the request from the accounting entirely, which is strictly worse than recording a call whose node count is unknown. It is never totalled as zero: `Session::total_node_count` filter-maps it out, the report prints the total "over N GraphQL requests that have one", and the by-document section lists only those. Every document this source itself sends is held countable by `tests/node_count.rs`.
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
    /// because a report is compared between runs and carries no board content. It is an
    /// [`Endpoint`] rather than a string for that reason: what a report may hold is settled
    /// where the endpoint is written, not here.
    #[must_use]
    pub fn rest(endpoint: Endpoint) -> Sending {
        Sending {
            mode: endpoint.method().mode(),
            call: Call::Endpoint { endpoint },
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
    estimates: Mutex<BTreeMap<Budget, u64>>,
}

impl Accounting {
    /// An accounting with nothing in it.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
    /// Record what a caller estimated this session would spend against `budget`, before
    /// it started.
    ///
    /// An estimate is not an observation and is kept apart from one everywhere below: it is
    /// what a precondition decided on, and the point of carrying it here is that a report
    /// can put it beside what the session really spent, so a reader sees how far the model
    /// was from GitHub's own figures rather than being told to trust it. The last estimate
    /// recorded for a budget is the one reported, because a precondition decides once.
    pub fn estimate(&self, budget: Budget, cost: u64) {
        self.estimates
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(budget, cost);
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
            estimates: self
                .estimates
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
    estimates: BTreeMap<Budget, u64>,
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
    /// What a precondition estimated this session would spend against `budget`, before it
    /// started, when one estimated anything at all.
    #[must_use]
    pub fn estimated(&self, budget: Budget) -> Option<u64> {
        self.estimates.get(&budget).copied()
    }
    /// What this session is **attributed** against `budget`, summed per call.
    ///
    /// Attributed rather than spent, and the difference is the point: only a call GitHub
    /// itself reported a cost for is a measurement. A call against a budget metered in
    /// requests is its own measure, and every other GraphQL call carries [`Basis::Modelled`]
    /// — one point, GitHub's documented minimum, which is a **lower bound**. So this is what
    /// the session can account for, never a claim about what GitHub charged; [`Self::budgets`]
    /// is where the same total comes apart by basis, and that is what says how much of it is
    /// measured.
    #[must_use]
    pub fn attributed(&self, budget: Budget) -> u64 {
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
        // Seeded from the estimates first: a budget a precondition sized this session
        // against and the session then never reached is worth reporting as exactly that,
        // rather than vanishing from the report that is supposed to compare the two.
        for (budget, estimated) in &self.estimates {
            touched
                .entry(*budget)
                .or_insert_with(|| BudgetReport::of(*budget))
                .estimated = Some(*estimated);
        }
        for request in &self.requests {
            let budget = request.budget();
            touched
                .entry(budget)
                .or_insert_with(|| BudgetReport::of(budget))
                .record(request);
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
///
/// **Every figure here is a total over requests that really drew on this budget, so the only
/// way to have one is to add those requests up.** [`Session::budgets`] is that, and
/// [`BudgetReport::record`] is where a request joins one: the request count, the spend, its
/// three attributions and the account's own readings all move together, from the same record,
/// so a report cannot say it summarises nine requests while its attributions add up to four,
/// or carry a REST budget's figures under [`Budget::Graphql`]. The fields are read through
/// the accessors below for exactly the reason [`RateLimit`]'s are: a hand-assembled set of
/// totals would be a measurement of nothing, and this whole accounting exists because a
/// number nobody measured was argued about instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetReport {
    budget: Budget,
    estimated: Option<u64>,
    requests: usize,
    attributed: u64,
    reported: u64,
    modelled: u64,
    counted: u64,
    not_run: usize,
    limit: Option<u64>,
    used_by_the_account: Option<u64>,
    remaining_first_seen: Option<u64>,
    remaining_last_seen: Option<u64>,
}

impl BudgetReport {
    /// A report of `budget` with nothing added to it yet.
    const fn of(budget: Budget) -> Self {
        Self {
            budget,
            estimated: None,
            requests: 0,
            attributed: 0,
            reported: 0,
            modelled: 0,
            counted: 0,
            not_run: 0,
            limit: None,
            used_by_the_account: None,
            remaining_first_seen: None,
            remaining_last_seen: None,
        }
    }
    /// Add one of this budget's requests, and everything its response said about it.
    ///
    /// The one place these totals move, which is what makes them agree with each other and
    /// with the requests they are over.
    fn record(&mut self, request: &Request) {
        self.requests += 1;
        self.attributed = self.attributed.saturating_add(request.spend.amount);
        match request.spend.basis {
            Basis::Reported => {
                self.reported = self.reported.saturating_add(request.spend.amount);
            }
            Basis::Modelled => {
                self.modelled = self.modelled.saturating_add(request.spend.amount);
            }
            Basis::Counted => {
                self.counted = self.counted.saturating_add(request.spend.amount);
            }
            // Attributed nothing, and counted as one of the requests that ran into the
            // limiter. Its headers are still read below — a refusal for a spent budget is
            // the response whose figures say most about that budget's state.
            Basis::NotRun => self.not_run += 1,
        }
        if let Some(limit) = request.limits.limit() {
            self.limit = Some(limit);
        }
        if let Some(used) = request.limits.used_by_the_account() {
            self.used_by_the_account = Some(used);
        }
        if let Some(remaining) = request.limits.remaining() {
            self.remaining_first_seen.get_or_insert(remaining);
            self.remaining_last_seen = Some(remaining);
        }
    }
    /// Which budget.
    #[must_use]
    pub const fn budget(&self) -> Budget {
        self.budget
    }
    /// What a precondition estimated this session would spend against it before it
    /// started, when one estimated anything at all.
    ///
    /// It is an estimate rather than a measurement, and the report says so where it prints
    /// it: what makes it worth carrying is that a reader can see how far the model was from
    /// what the session really spent.
    #[must_use]
    pub const fn estimated(&self) -> Option<u64> {
        self.estimated
    }
    /// How many of this session's requests drew on it.
    #[must_use]
    pub const fn requests(&self) -> usize {
        self.requests
    }
    /// What this session itself is **attributed** against it, summed per call.
    ///
    /// Attributed rather than spent, for the reason [`Session::attributed`] gives: some of
    /// this figure is [`Basis::Modelled`], GitHub's documented one-point minimum, which is a
    /// lower bound and not a measurement. The four figures below are the same total split by
    /// basis, which is what says how much of it GitHub itself reported.
    #[must_use]
    pub const fn attributed(&self) -> u64 {
        self.attributed
    }
    /// How much of that GitHub itself reported.
    #[must_use]
    pub const fn reported(&self) -> u64 {
        self.reported
    }
    /// How much of it is this repository's one-point-per-call lower bound.
    #[must_use]
    pub const fn modelled(&self) -> u64 {
        self.modelled
    }
    /// How much of it is a count of requests against a budget metered in requests.
    #[must_use]
    pub const fn counted(&self) -> u64 {
        self.counted
    }
    /// How many of its requests a rate limiter refused, so they never ran and are
    /// attributed nothing.
    #[must_use]
    pub const fn not_run(&self) -> usize {
        self.not_run
    }
    /// The whole allowance, as GitHub's own headers reported it.
    #[must_use]
    pub const fn limit(&self) -> Option<u64> {
        self.limit
    }
    /// What the **account** had spent, as GitHub's own headers reported it. Not this
    /// session's spend: other work draws on the same budget in the same window.
    #[must_use]
    pub const fn used_by_the_account(&self) -> Option<u64> {
        self.used_by_the_account
    }
    /// The allowance remaining when this session's first request against this budget was
    /// answered.
    #[must_use]
    pub const fn remaining_first_seen(&self) -> Option<u64> {
        self.remaining_first_seen
    }
    /// The allowance remaining when its last one was.
    #[must_use]
    pub const fn remaining_last_seen(&self) -> Option<u64> {
        self.remaining_last_seen
    }
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
            "  this session sent {} requests and is attributed {} {} — measured only where \
             GitHub reported it: {} {}, {} {}, {} {}; {} {}",
            self.requests,
            self.attributed,
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
        if let Some(estimated) = self.estimated {
            let _ = writeln!(
                lines,
                "  a precondition estimated {estimated} {} before this session started, \
                 against the {} attributed above",
                self.budget.unit(),
                self.attributed,
            );
        }
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
