//! Structural and residue-free write verification against GitHub's real Projects v2 API.
//!
//! This target is the credential's half alone: it decides whether the lane may run, opens the
//! one session that may hold the credential, and hands the whole journey to
//! [`journey::run`] pointed at GitHub. The journey itself is shared, because the same code
//! is driven a second time against this crate's loopback fixture board — with no credential
//! — to count what one session costs. Two spellings of one journey would measure two
//! journeys.

use std::env;

use onetaskgraph_live::Session;

mod journey;
mod lane;

use lane::{LiveLane, SESSION_NAME, live_lane};

#[tokio::test]
async fn real_projects_v2_contract_writes_and_leaves_no_residue() {
    // llmlint: ignore-block[live_tier_compiles_and_requires_credential,tests_assert_real_behavior] An absent credential
    // skips only where none was expected — a contributor with no keys, and a fork pull
    // request, which the host gives no secrets. `ONETASKGRAPH_LIVE_REQUIRED=1`, which
    // .github/workflows/ci.yml sets on the one lane the credentials reach, turns every skip
    // below into the failure this rule asks for. The skip branch asserting nothing about
    // GitHub is the point rather than a gap: there was no run to assert about, and a
    // stand-in asserted against instead would be a green result for a journey that never
    // reached the API. What that branch owes is a printed reason, and it prints one.
    let lane = live_lane(
        env::var("GH_PROJECTS_TOKEN").ok().as_deref(),
        // llmlint: ignore[contracts_have_one_source_or_a_drift_gate] .github/workflows/ci.yml spells these three names too, and the drift gate is the lane's own refusal: that workflow sets ONETASKGRAPH_LIVE_REQUIRED=1 on the lane it hands the credential to, so a name spelled differently on either side fails the required check naming the variable rather than skipping green.
        env::var("GH_PROJECTS_OWNER").ok().as_deref(),
        env::var("GH_PROJECTS_NUMBER").ok().as_deref(),
        env::var("GH_PROJECTS_REPOSITORY").ok().as_deref(),
        env::var(onetaskgraph_live::REQUIRED_VARIABLE)
            .ok()
            .as_deref(),
    )
    .unwrap_or_else(|error| panic!("the GitHub Projects live lane cannot run: {error}"));
    let (token, owner, project_number, repository) = match lane {
        LiveLane::Run {
            token,
            owner,
            project_number,
            repository,
        } => (token, owner, project_number, repository),
        LiveLane::Skip(reason) => {
            // Straight to the process's stderr, the way the session report is: the test
            // harness captures `eprintln!` and discards it for every test that passed, and
            // a skip is a pass. So a reader of this target's ordinary output can see that
            // the live session did not run, and why, without knowing to go looking.
            journey::say(&format!("skipped live GitHub Projects journey: {reason}"));
            return;
        }
    };
    // llmlint: ignore-end[live_tier_compiles_and_requires_credential,tests_assert_real_behavior]
    // The one gate: nothing below may reach GitHub until the session is open, because the
    // token below is the one this returns rather than the one the lane read. A session that
    // is refused did not run and did not pass, and says so.
    let session = Session::open(SESSION_NAME, token).unwrap_or_else(|declined| declined.refuse());
    journey::against(journey::Endpoints::github());
    journey::run(journey::Nomination {
        token: session.credential().expose().to_owned(),
        owner,
        project_number,
        repository,
    })
    .await;
}
