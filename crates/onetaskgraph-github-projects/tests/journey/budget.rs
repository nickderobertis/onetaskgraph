//! The precondition a live session passes before it spends any of the account's allowance.
//!
//! A live run must never be the thing that exhausts a budget the work outside this
//! repository depends on. So before this journey does any of the work it exists to do, it
//! asks GitHub what the account has left and refuses to start unless every budget it will
//! draw on can pay for the session **and** still keep the share
//! [`onetaskgraph_live::RETAINED_BUFFER`] holds back. The arithmetic and that share live in
//! the gate crate, once for every lane; what is here is what only this lane knows — which
//! read learns an allowance, and what this session is estimated to cost.
//!
//! # The read, and why it does not spend what it is protecting
//!
//! One request: `GET /rate_limit`. GitHub's own documentation of that endpoint says, in a
//! note directly under its title, *"Accessing this endpoint does not count against your
//! REST API rate limit"*, and the same page documents its answer as carrying one object per
//! budget — `resources.core` for *"all non-search-related resources in the REST API"* and
//! `resources.graphql` for *"your rate limit status for the GraphQL API"*, each with
//! `limit`, `used`, `remaining` and `reset`. So **one free call answers both budgets this
//! session draws on**, and it cannot spend GraphQL points at all, because it is not a
//! GraphQL call.
//!
//! The alternative GitHub offers — the `rateLimit` object in the GraphQL schema — is
//! deliberately **not** used. It carries no published statement that it is unmetered, and
//! GitHub's own guidance on that page points the other way: *"When possible, you should use
//! the rate limit response headers instead of querying the API to check your rate limit."*
//! A gate that could not cite a basis for its own read being free would be a gate that
//! might itself tip a nearly-exhausted account over.
//!
//! **Freeness is not established here by reading twice and comparing.** This account is
//! shared, so another consumer moving the allowance between two reads would condemn a sound
//! gate, and two equal readings would not show either read was uncharged either. What is
//! done instead is what the criterion above asks: the published statement is cited, and the
//! gate's own read is recorded in the accounting like every other request — so what the
//! gate itself cost appears in every session report, and a read that began costing would
//! show there. `reconcile_node_counts` in the module beside this one carries the
//! credentialed observation alongside: it records what the account's allowance read before
//! and after a stretch of the run, which is an observation rather than a verdict.
//!
//! # The cost model, and the rule it rests on
//!
//! GitHub publishes, on *Rate limits and query limits for the GraphQL API*:
//!
//! 1. *"Add up the number of requests needed to fulfill each unique connection in the call.
//!    Assume every request will reach the first or last argument limits."*
//! 2. *"Divide the number by 100 and round the result to the nearest whole number to get
//!    the final aggregate point value."*
//!
//! with the note that *"The minimum point value of a call to the GraphQL API is 1"*. Call
//! that aggregate `A`; a call costs `max(1, round(A / 100))` points. The REST API is
//! metered in requests instead, so a REST call is its own measure.
//!
//! **`A` is not the node count, and the node count cannot stand in for it.** A connection
//! resolved `R` times with a page size of `P` needs `R` requests and returns `R × P` nodes,
//! so the two differ by the page size — GitHub's own worked example returns 305,100 nodes
//! and costs 51 points, where the node count divided by 100 would have said 3,051. What is
//! true is the bound that follows from the same identity: summed over a call's connections,
//! `A = Σ nodes / first ≤ nodes / min(first)`. Every `first:` this source binds is one of
//! the three page sizes [`largest_page_sizes`] names, the smallest of which is read out of
//! that function below rather than copied, so:
//!
//! ```text
//! points(one call) ≤ 1 + ceil(nodes / (smallest page size × 100))
//! ```
//!
//! **Where it is unsure, it estimates high.** `ceil` rather than GitHub's "round to the
//! nearest"; the leading `1` over a formula that has no such term, which also absorbs the
//! one place the bound is not airtight — a caller asking for a page *smaller* than the
//! smallest size this source's own constants bind, where one connection can exceed
//! `nodes / smallest` by under one request, and no document here has more than six
//! connections. And the bound itself is reached only by a call whose every connection sits
//! at the smallest page size, which none of this source's do: `reading the board` really
//! aggregates about 5,200 requests where this bound allows 26,015.
//!
//! **How a reader checks it against GitHub.** Two ways, both already here. Offline: take
//! any document from `graphql::DOCUMENTS`, count the requests each of its connections needs
//! under [`largest_page_sizes`] by GitHub's step 1, and compare `max(1, round(A / 100))`
//! with what [`points`] gives for that call's node count. Against GitHub itself: the
//! credentialed lane records GitHub's own reported `cost` for every call it makes, and
//! [`accounting::Session::report`] now prints this estimate beside what the session was
//! attributed — so a run says how far the model was from GitHub's figures rather than
//! asking anyone to trust it.
//!
//! # Where the estimate comes from
//!
//! `tests/fixtures/session-cost.txt`, the branch's own per-call record of the reduced
//! session, read at compile time. Nothing here calls GitHub to size the session and nothing
//! needs a credential: the estimate is arithmetic over that record. It also follows the
//! session without an edit — the record is a golden that the session-cost test rewrites when
//! what a session sends changes, and the estimate is recomputed from it.

use std::collections::BTreeMap;

use onetaskgraph_github_projects::accounting::{Accounting, Budget, Endpoint, Method};
use onetaskgraph_github_projects::largest_page_sizes;
use onetaskgraph_live::{Allowance, Declined, Demand, Unaffordable, affordable};
use serde_json::Value;

use crate::lane::SESSION_NAME;

/// The one endpoint this precondition calls, as GitHub's documentation spells it.
pub const ALLOWANCE_ENDPOINT: &str = "/rate_limit";

/// How that call is named in the reason a budget went unread.
///
/// It names the endpoint, because what a declined run owes a reader is *which read failed*
/// — and the session report already names the call by its own endpoint template, so this is
/// the one place the two would otherwise disagree.
pub const ALLOWANCE_READ: &str = "the allowance read GET /rate_limit";

/// The branch's own per-call record of the reduced session.
///
/// Read at compile time so deriving the estimate needs no credential, no call to GitHub and
/// no file at run time — and so that a session which stops costing what this record says
/// fails the session-cost test rather than quietly moving the gate.
const SESSION_RECORD: &str = include_str!("../fixtures/session-cost.txt");

/// The line the per-call rows of that record begin after.
const PER_CALL: &str = "requests per call";

/// The budget GitHub's `/rate_limit` answer reports a session's REST calls under.
///
/// `core` is what that endpoint calls *"all non-search-related resources in the REST API"*,
/// which is what every REST call this lane makes is one of. It is spelled here rather than
/// taken from [`Budget::name`] because the two names are GitHub's own and are different:
/// the `x-ratelimit-resource` header the accounting reads and this endpoint's object key
/// do not have to agree, and pretending they do would read the wrong budget's figures.
const REST_RESOURCE: &str = "core";

/// The budget that answer reports GraphQL points under.
const GRAPHQL_RESOURCE: &str = "graphql";

/// One row of the record: how many calls, and their worst-case nodes between them.
struct Row {
    requests: u64,
    nodes: u64,
    budget: Budget,
}

/// The smallest page size any `first:` in this source's documents is bound to.
///
/// Read out of [`largest_page_sizes`] rather than copied, so a page-size constant that
/// changes moves this bound with it instead of leaving a stale divisor behind.
fn smallest_page_size() -> u64 {
    largest_page_sizes()
        .values()
        .copied()
        .min()
        .map(u64::from)
        .expect("this source binds at least one page size")
}

/// What one row of the record costs against the GraphQL budget, in points.
///
/// `1 + ceil(nodes / (smallest page size × 100))` per call, which the module documentation
/// derives from GitHub's published formula, summed over the row's calls. The record gives
/// one node total for the whole row rather than one per call, and
/// `Σ (1 + ceil(nᵢ/D)) ≤ k + Σ (nᵢ/D + 1) = 2k + N/D ≤ 2k + ceil(N/D)`, so the row is
/// charged `2 × requests + ceil(nodes / D)` — an upper bound on the per-call sum however
/// the row's nodes were distributed between its calls.
fn points(requests: u64, nodes: u64) -> u64 {
    let divisor = smallest_page_size().saturating_mul(100);
    2_u64
        .saturating_mul(requests)
        .saturating_add(nodes.div_ceil(divisor))
}

/// Which budget a recorded call drew on, from the name the record carries it under.
///
/// A REST call is named by its endpoint — `GET /repos/{owner}/{repo}/labels` — so a name
/// that is a method and a path template is one, and everything else is a GraphQL document's
/// own description. Both halves are decided by the accounting's own parsers rather than by a
/// second reading of the same spelling.
fn budget_of(name: &str) -> Budget {
    let endpoint = name.split_once(' ').and_then(|(method, path)| {
        Method::parse(method).and_then(|method| Endpoint::parse(method, path))
    });
    if endpoint.is_some() {
        Budget::Rest
    } else {
        Budget::Graphql
    }
}

/// The per-call rows of the record.
fn rows(record: &str) -> Vec<Row> {
    record
        .lines()
        .skip_while(|line| line.trim() != PER_CALL)
        .skip(1)
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let mut fields = line.split_whitespace();
            let requests = fields
                .next()
                .and_then(|count| count.parse().ok())
                .unwrap_or_else(|| panic!("the record's row {line:?} begins with a call count"));
            let nodes = fields
                .next()
                .and_then(|nodes| nodes.parse().ok())
                .unwrap_or_else(|| panic!("the record's row {line:?} carries a node count"));
            let name = fields.collect::<Vec<_>>().join(" ");
            assert!(!name.is_empty(), "the record's row {line:?} names its call");
            Row {
                requests,
                nodes,
                budget: budget_of(&name),
            }
        })
        .collect()
}

/// What this session is estimated to spend against each budget it draws on.
///
/// Points for GraphQL, requests for REST, which is what GitHub meters each in. Derived from
/// the record alone: no credential, no call to GitHub, and no figure taken from anywhere but
/// the branch's own measurement of the session that will run.
#[must_use]
pub fn estimate() -> BTreeMap<Budget, u64> {
    let mut estimated = BTreeMap::from([(Budget::Graphql, 0), (Budget::Rest, 0)]);
    for row in rows(SESSION_RECORD) {
        let cost = match row.budget {
            // Metered in requests, so a call is its own measure and the row is its count.
            Budget::Rest => row.requests,
            Budget::Graphql => points(row.requests, row.nodes),
        };
        *estimated.entry(row.budget).or_default() += cost;
    }
    estimated
}

/// The account's allowance for every budget, from GitHub's own rate-limit endpoint.
///
/// Recorded into `into` like any other request, which is what makes the gate's own cost
/// visible in the session report rather than assumed.
async fn read_allowance(token: &str, rest_host: &str, into: &Accounting) -> Result<Value, String> {
    let endpoint = Endpoint::parse(Method::Get, ALLOWANCE_ENDPOINT)
        .expect("GitHub's rate-limit endpoint is spelled like an endpoint template");
    let sent = reqwest::Client::new()
        .get(format!("{rest_host}{ALLOWANCE_ENDPOINT}"))
        .header("user-agent", "onetaskgraph-live-test")
        .header("accept", "application/vnd.github+json")
        .bearer_auth(token)
        .send()
        .await;
    super::record_rest_response(into, &endpoint, sent, ALLOWANCE_READ).await
}

/// One budget's allowance out of that answer, or why it is not there to read.
fn allowance_of(answered: &Result<Value, String>, resource: &str) -> Result<Allowance, String> {
    let answer = answered.as_ref().map_err(Clone::clone)?;
    let reported = answer
        .pointer(&format!("/resources/{resource}"))
        .ok_or_else(|| {
            format!(
                "GET {ALLOWANCE_ENDPOINT} answered without a resources.{resource} object, so this \
                 budget's allowance is unknown"
            )
        })?;
    let field = |name: &str| {
        reported
            .get(name)
            .and_then(Value::as_u64)
            .ok_or_else(|| format!("GET {ALLOWANCE_ENDPOINT} reported no {resource} {name}"))
    };
    Ok(Allowance::read(
        field("limit")?,
        field("remaining")?,
        field("reset")?,
    ))
}

/// Ask GitHub what the account has left, and decide whether this session may start.
///
/// **This is a session's first request and its only one before the decision**: one
/// `GET /rate_limit`, which answers every budget this session draws on. What comes back is
/// turned into one [`Demand`] per budget — with the estimate above and the allowance that
/// budget's own object reported — and handed to [`affordable`], which is where the
/// [`onetaskgraph_live::RETAINED_BUFFER`] arithmetic lives.
///
/// The estimates are recorded into `into` before the read, so the report of a session that
/// does start carries them beside what it really spent.
///
/// # Errors
///
/// When any budget's allowance could not be read, or when starting would dip into that
/// budget's retained buffer. Both are a [`Declined`]: a run that did not happen, which is
/// neither a pass nor a failing assertion. Nothing here sleeps, polls or retries.
pub async fn precondition(token: &str, rest_host: &str, into: &Accounting) -> Result<(), Declined> {
    let estimated = estimate();
    for (budget, cost) in &estimated {
        into.estimate(*budget, *cost);
    }
    let answered = read_allowance(token, rest_host, into).await;
    let demand = |budget: Budget, resource: &str| {
        let cost = estimated.get(&budget).copied().unwrap_or_default();
        match allowance_of(&answered, resource) {
            Ok(allowance) => Demand::read(budget.name(), budget.unit(), cost, allowance),
            Err(why) => Demand::unread(budget.name(), budget.unit(), cost, why),
        }
    };
    affordable(&[
        demand(Budget::Graphql, GRAPHQL_RESOURCE),
        demand(Budget::Rest, REST_RESOURCE),
    ])
    .map_err(|cause: Unaffordable| Declined::unaffordable(SESSION_NAME, cause))
}
