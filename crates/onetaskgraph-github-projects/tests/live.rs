//! Structural and residue-free write verification against GitHub's real Projects v2 API.
//!
//! This target is the credential's half alone: it points [`journey::enter`] at GitHub with
//! the process environment, which decides whether the lane may run and under which stamp,
//! opens the one session that may hold the credential, and hands the whole journey to
//! [`journey::run`]. The journey itself is shared, because the same code
//! is driven a second time against this crate's loopback fixture board — with no credential
//! — to count what one session costs. Two spellings of one journey would measure two
//! journeys.

use std::env;

use onetaskgraph_live::{Exclusivity, Session};

// The stand-ins that prove the budget precondition build their answers from `journey`'s
// own pinned names, and this target reaches none of them: it points the journey at GitHub,
// which needs no stand-in. So what it does not use is the other drives' rather than dead
// code — the same reason `tests/plugin.rs` carries this.
#[allow(dead_code)]
mod journey;
// Likewise `lane`: this target names only `SESSION_NAME`, and its other helpers are the
// admission, stamp and cleanup the fixture drives and `tests/lane_shape.rs` exercise.
#[allow(dead_code)]
mod lane;

use lane::SESSION_NAME;

#[tokio::test]
async fn real_projects_v2_contract_writes_and_leaves_no_residue() {
    journey::against(journey::Endpoints::github());
    // `Shared`: this lane takes no seat. Every artifact it writes carries this process's own
    // stamp, its cleanup removes only those, and what it recovers of an interrupted run's is
    // decided by that artifact's own stamp — so two sessions of this lane cannot reach each
    // other's work and there is nothing left for a seat to protect. What it never protected
    // is the case that remains: the hosted check runs on three platforms and a file on one
    // runner excludes nothing on another. The precondition that can decline this lane is the
    // budget one, inside `journey::run`, which `scripts/check-budget-decline.sh` drives
    // through to a red check without a credential. A session that is refused did not run and
    // did not pass, and says so.
    journey::enter(&|variable| env::var(variable).ok(), |token| {
        Session::open(SESSION_NAME, token, Exclusivity::Shared)
            .unwrap_or_else(|declined| declined.refuse())
    })
    .await
    .unwrap_or_else(|error| panic!("the GitHub Projects live lane cannot run: {error}"));
}
