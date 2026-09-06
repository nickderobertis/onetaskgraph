//! The live lane's own decisions, asserted without reaching GitHub.
//!
//! Which board and repository the credentialed lane may write to, which artifacts a run
//! recognises as its own and as an interrupted earlier run's, and that cleanup runs whether the
//! journey passed or failed. None of it touches the network or reads a credential, so these
//! assertions hold on every machine — including the ones the journey beside them skips on for
//! want of a credential, which is where the decisions asserted here would otherwise go
//! unproven.

mod lane;

use std::time::Duration;

use lane::{
    LiveLane, LiveSecret, SESSION_NAME, artifact_label, artifact_title, is_orphan_label,
    is_orphan_title, is_run_artifact_label, is_run_artifact_title, live_lane, live_write_config,
    rest_outcome, run_then_cleanup,
};
use onetaskgraph_github_projects::DESIGN_TITLE_PREFIX;
use onetaskgraph_github_projects::accounting::{Outcome, StatusCode};
use onetaskgraph_live::artifact::{Registration, Registry, Run, Sweep};
use onetaskgraph_live::{Credential, Exclusivity, Session};
use onetaskgraph_plugin_api::{SourceName, SourcePlugin};

/// A window these assertions drive either side of, rather than waiting out the real one.
///
/// Safety here does not rest on the window's length — what permits a removal is the owning
/// run's registration lock, and the window can only hold a removal back — so a check may
/// choose whichever window makes what it is proving visible.
const WINDOW: Duration = Duration::from_secs(60);

/// Microseconds since the epoch, far enough in that ageing a stamp cannot go negative.
const NOW: i64 = 1_787_816_134_627_361;

/// A stamp `by` wrote `windows` windows ago.
fn aged(windows: i64) -> i64 {
    NOW - windows * i64::try_from(WINDOW.as_micros()).expect("a minute fits in microseconds")
}

#[tokio::test]
async fn cleanup_runs_after_a_successful_live_journey() {
    let cleaned = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let observed = cleaned.clone();
    assert_eq!(
        run_then_cleanup(
            || async { Ok(()) },
            || async move {
                observed.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            }
        )
        .await,
        Ok(())
    );
    assert!(cleaned.load(std::sync::atomic::Ordering::SeqCst));
}

#[tokio::test]
async fn cleanup_runs_after_a_failed_live_journey() {
    let cleaned = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let observed = cleaned.clone();
    assert_eq!(
        run_then_cleanup(
            || async { Err("injected mutation failure".to_owned()) },
            || async move {
                observed.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            }
        )
        .await,
        Err("injected mutation failure".to_owned())
    );
    assert!(cleaned.load(std::sync::atomic::Ordering::SeqCst));
}

#[test]
fn a_response_this_lane_could_not_read_is_not_reported_as_one_github_answered() {
    // The session report is what says a run's calls were worth what they cost, so a `2xx`
    // carrying a body this lane could not decode has to be recorded as the refusal it was
    // — otherwise the report names a call that succeeded and produced nothing.
    let ok = StatusCode::OK;
    assert_eq!(
        rest_outcome(ok, false, "{\"id\":\"L_1\"}", true),
        Outcome::Answered,
        "a 2xx this lane read is what it says it is"
    );
    assert_eq!(
        rest_outcome(ok, false, "<html>a proxy said hello</html>", false),
        Outcome::Refused,
        "a 2xx this lane could not read answered nothing"
    );
    // The two the status alone settles are still the status's to settle: a refusal stays a
    // refusal whether or not its body parsed, and a rate-limited call still did not run.
    assert_eq!(
        rest_outcome(StatusCode::NOT_FOUND, false, "{}", true),
        Outcome::Refused
    );
    assert_eq!(
        rest_outcome(StatusCode::FORBIDDEN, true, "rate limit exceeded", false),
        Outcome::RateLimited,
        "a body that did not parse must not turn a rate-limited call into an ordinary refusal"
    );
}

#[test]
fn the_lane_takes_its_board_and_repository_only_from_the_names_it_is_given() {
    assert_eq!(
        live_lane(
            Some("live-token"),
            Some("nickderobertis"),
            Some("1"),
            Some("acme/work"),
            None
        ),
        Ok(LiveLane::Run {
            token: Credential::new("live-token").expect("a placeholder credential is not blank"),
            owner: "nickderobertis".to_owned(),
            project_number: 1,
            repository: "acme/work".to_owned(),
        })
    );
}

#[test]
fn the_lanes_write_configuration_is_accepted_whatever_the_board_calls_its_first_column() {
    // The lane discovers this option name from the board at run time, so its configuration
    // has to be accepted whichever name comes back — including each name a shipped default
    // already claims, which is where pointing `unknown` at the column collided.
    for option in ["Todo", "Backlog", "In Progress", "Ready for review"] {
        onetaskgraph_github_projects::Plugin
            .build(
                &SourceName::new("github-live").unwrap(),
                &live_write_config("nickderobertis", 1, "acme/work", option),
                &LiveSecret("live-token".into()),
            )
            .unwrap_or_else(|error| {
                panic!(
                    "the live write configuration was refused for a board whose first Status \
                     option is {option:?}: {error}"
                )
            });
    }
}

#[test]
fn an_unnamed_board_skips_the_lane_and_says_which_two_names_it_needs() {
    let Ok(LiveLane::Skip(reason)) =
        live_lane(Some("live-token"), None, None, Some("acme/work"), None)
    else {
        panic!("a credential without a named board must skip rather than discover one");
    };
    assert!(
        reason.contains("GH_PROJECTS_OWNER") && reason.contains("GH_PROJECTS_NUMBER"),
        "the skip must name both variables, not just report a skip: {reason}"
    );
}

#[test]
fn an_unnamed_board_fails_the_lane_when_the_live_tier_is_required() {
    let error = live_lane(Some("live-token"), None, None, Some("acme/work"), Some("1"))
        .expect_err("ONETASKGRAPH_LIVE_REQUIRED=1 must turn the unnamed-board skip into a failure");
    assert!(
        error.contains("GH_PROJECTS_OWNER")
            && error.contains("GH_PROJECTS_NUMBER")
            && error.contains("ONETASKGRAPH_LIVE_REQUIRED"),
        "the failure must name both variables and what demanded them: {error}"
    );
}

#[test]
fn an_absent_credential_keeps_its_own_skip_or_fail_pairing() {
    let Ok(LiveLane::Skip(reason)) = live_lane(
        None,
        Some("nickderobertis"),
        Some("1"),
        Some("acme/work"),
        None,
    ) else {
        panic!("an absent credential must skip");
    };
    assert!(reason.contains("GH_PROJECTS_TOKEN"), "{reason}");
    let error = live_lane(
        None,
        Some("nickderobertis"),
        Some("1"),
        Some("acme/work"),
        Some("1"),
    )
    .expect_err("ONETASKGRAPH_LIVE_REQUIRED=1 must turn the absent-credential skip into a failure");
    assert!(error.contains("GH_PROJECTS_TOKEN"), "{error}");
    let Ok(LiveLane::Skip(empty)) = live_lane(
        Some("  "),
        Some("nickderobertis"),
        Some("1"),
        Some("acme/work"),
        None,
    ) else {
        panic!("an empty credential must skip");
    };
    assert!(empty.contains("GH_PROJECTS_TOKEN"), "{empty}");
}

#[test]
fn half_a_board_and_an_unusable_number_are_misconfigurations_rather_than_skips() {
    for (owner, number) in [
        (Some("nickderobertis"), None),
        (None, Some("1")),
        (Some("nickderobertis"), Some("0")),
        (Some("nickderobertis"), Some("not-a-number")),
        (Some("nickderobertis"), Some("4294967295")),
    ] {
        live_lane(Some("live-token"), owner, number, Some("acme/work"), None).expect_err(&format!(
            "GH_PROJECTS_OWNER={owner:?} with GH_PROJECTS_NUMBER={number:?} must fail rather than \
             skip or select a board"
        ));
    }
}

#[test]
fn an_unnamed_repository_skips_the_lane_and_a_malformed_one_is_a_misconfiguration() {
    // A lane that creates issues names the repository it creates them in, for the reason it
    // names the board: a credentialed write must not land somewhere nobody nominated.
    let Ok(LiveLane::Skip(reason)) = live_lane(
        Some("live-token"),
        Some("nickderobertis"),
        Some("1"),
        None,
        None,
    ) else {
        panic!("an unnamed repository must skip rather than discover one");
    };
    assert!(reason.contains("GH_PROJECTS_REPOSITORY"), "{reason}");
    let required = live_lane(
        Some("live-token"),
        Some("nickderobertis"),
        Some("1"),
        None,
        Some("1"),
    )
    .expect_err("ONETASKGRAPH_LIVE_REQUIRED=1 turns that skip into a failure");
    assert!(required.contains("GH_PROJECTS_REPOSITORY"), "{required}");
    for malformed in ["nameless", "acme/", "/work", "acme/work/extra"] {
        live_lane(
            Some("live-token"),
            Some("nickderobertis"),
            Some("1"),
            Some(malformed),
            None,
        )
        .expect_err(&format!("GH_PROJECTS_REPOSITORY={malformed:?} must fail"));
    }
}

#[test]
fn a_repository_nomination_that_would_address_something_else_is_refused_before_anything_is_sent() {
    // Both halves are filled into this lane's REST endpoint templates, so a character that
    // is significant in a URL does not name a repository — it redirects a credentialed
    // call, and the endpoint the session report names is then not the one that was made.
    // Each of these splits on a slash and would have passed the owner/name check alone.
    for redirecting in [
        "acme/work?per_page=1",
        "acme/work#fragment",
        "acme/work%2f..",
        "acme/..",
        "acme/.",
        "acme/wo rk",
        "acme/{repo}",
        "-acme/work",
        "acme-/work",
        "ac--me/work",
        "ac me/work",
        "acme./work",
    ] {
        let refusal = live_lane(
            Some("live-token"),
            Some("nickderobertis"),
            Some("1"),
            Some(redirecting),
            None,
        )
        .expect_err(&format!(
            "GH_PROJECTS_REPOSITORY={redirecting:?} reaches a URL, so it must be refused rather \
             than sent"
        ));
        assert!(
            refusal.contains("GH_PROJECTS_REPOSITORY") && refusal.contains(redirecting),
            "the refusal names the variable and the value given: {refusal}"
        );
    }
    // The nomination CI actually sets still passes, so the grammar refuses a redirect
    // rather than the real board's own repository.
    assert!(matches!(
        live_lane(
            Some("live-token"),
            Some("nickderobertis"),
            Some("1"),
            Some("nickderobertis/onetaskgraph"),
            None,
        ),
        Ok(LiveLane::Run { .. })
    ));
}

/// A registry of this check's own, holding one run that has ended and one that is live.
///
/// Real registrations rather than a description of them: `ended` was taken and given up, so
/// the kernel reports its lock free, and `live` is still held. That is the whole of the
/// evidence a sweep decides on, and it is the same evidence whichever process holds the lock
/// — flock and `LockFileEx` both refuse a second handle in the process that already has one.
struct Runs {
    directory: std::path::PathBuf,
    registry: Registry,
    mine: Run,
    ended: Run,
    live: Run,
    _held: Registration,
}

impl Runs {
    fn take() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "onetaskgraph-lane-shape-runs-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        let registry = Registry::at(&directory);
        let mine = registry.enrol();
        let ended = Registration::take(&registry, 2533).expect("a run that will end");
        let ended = ended.run();
        drop(Registration::take(&registry, 2533));
        let held = Registration::take(&registry, 9998).expect("a run that is still going");
        let live = held.run();
        Self {
            directory,
            registry,
            mine,
            ended,
            live,
            _held: held,
        }
    }

    fn sweep(&self, now: i64) -> Sweep {
        self.registry.sweep(self.mine, now, WINDOW)
    }
}

impl Drop for Runs {
    /// Best effort: the registration this check still holds is an open file, which Windows
    /// will not let a directory be removed under. Leaving it is what the platform's own
    /// temporary directory is for.
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

#[test]
fn a_sweep_takes_an_ended_runs_artifacts_and_leaves_every_live_runs_alone() {
    let runs = Runs::take();
    let sweep = runs.sweep(NOW);
    let (ended, live, mine) = (runs.ended, runs.live, runs.mine);
    // What the sweep exists for: a run the kernel says has ended, whose artifact has waited
    // the window out.
    assert!(is_orphan_title(&sweep, &artifact_title(ended, aged(2))));
    // And the things it must never take. A fresh artifact of an ended run has not waited the
    // window out; an artifact of a run that is STILL GOING is that run's however old it is,
    // which is what a hung or rate-limited session looks like and what age alone cannot
    // protect; one of THIS run is this run's whatever its age; and a title that is not
    // spelled the way this lane spells one belongs to somebody else entirely.
    for left in [
        artifact_title(ended, NOW),
        artifact_title(live, aged(1_000)),
        artifact_title(live, NOW),
        artifact_title(mine, aged(1_000)),
        artifact_title(mine, NOW),
        artifact_title(Run::new(runs.registry.host(), 4242), aged(1_000)),
        artifact_title(
            Run::new(runs.registry.host().wrapping_add(1), 2533),
            aged(1_000),
        ),
        "AI Orchestrator plan".to_owned(),
        "onetaskgraph live cleanup".to_owned(),
        "onetaskgraph live cleanup 2533".to_owned(),
        format!("onetaskgraph live cleanup abc-{}", aged(2)),
        "onetaskgraph live cleanup 9999-".to_owned(),
        format!("copy of {}", artifact_title(ended, aged(2))),
    ] {
        assert!(
            !is_orphan_title(&sweep, &left),
            "a sweep must not take {left:?}"
        );
    }

    // A *document* this lane writes is titled the way this source spells one — the design
    // prefix, put there by the source rather than by the caller — so cleanup reads a title
    // the artifact prefix does not start. Recognition takes that prefix off first: without
    // it, a document a run created would be residue no sweep could ever name, on somebody's
    // real board.
    assert!(is_orphan_title(
        &sweep,
        &format!("{DESIGN_TITLE_PREFIX}{}", artifact_title(ended, aged(2)))
    ));
    assert!(
        !is_orphan_title(
            &sweep,
            &format!("{DESIGN_TITLE_PREFIX}{}", artifact_title(mine, aged(2)))
        ),
        "and this run's own document is this run's, exactly as its issue is"
    );
    for foreign in [
        format!("{DESIGN_TITLE_PREFIX}AI Orchestrator plan"),
        format!("{DESIGN_TITLE_PREFIX}onetaskgraph live cleanup 9999"),
        format!(
            "copy of {DESIGN_TITLE_PREFIX}{}",
            artifact_title(ended, aged(2))
        ),
    ] {
        assert!(
            !is_orphan_title(&sweep, &foreign),
            "a sweep must not take {foreign:?}"
        );
    }

    // The label is residue exactly as an item is, and is decided the same way.
    assert!(is_orphan_label(&sweep, &artifact_label(ended, aged(2))));
    for left in [
        artifact_label(ended, NOW),
        artifact_label(live, aged(1_000)),
        artifact_label(mine, aged(1_000)),
        "bug".to_owned(),
        "otg-live-".to_owned(),
        "otg-live-9999".to_owned(),
        format!("otg-live-abc-{}", aged(2)),
        format!("not-{}", artifact_label(ended, aged(2))),
    ] {
        assert!(
            !is_orphan_label(&sweep, &left),
            "a label sweep must not take {left:?}"
        );
    }
}

#[test]
fn a_label_this_lane_writes_fits_inside_the_limit_github_holds_one_to() {
    // Fifty characters, and a stamp now names the machine as well as the process — which is
    // why the label prefix is the short one. A label GitHub refuses is an artifact this lane
    // cannot write at all, and it would fail on somebody's real repository rather than here.
    let widest = artifact_label(Run::new(u32::MAX, u32::MAX), i64::MAX);
    assert!(
        widest.len() <= 50,
        "the widest label this lane could write is {} characters: {widest}",
        widest.len()
    );
}

#[test]
fn a_run_names_its_own_artifacts_and_no_other_runs() {
    // Teardown's half: a run removes everything it wrote, whether its assertions passed or
    // failed, by the run every one of its artifacts carries.
    let mine = Run::new(41, 2533);
    assert!(is_run_artifact_title(mine, &artifact_title(mine, 17)));
    assert!(is_run_artifact_title(
        mine,
        &format!("{DESIGN_TITLE_PREFIX}{}", artifact_title(mine, 17))
    ));
    assert!(is_run_artifact_label(mine, &artifact_label(mine, 17)));
    for other in [
        artifact_title(Run::new(41, 25330), 17),
        artifact_title(Run::new(41, 253), 17),
        artifact_title(Run::new(41, 12533), 17),
        // The same process id on another machine is another run, and this is the one a
        // process id alone could not tell apart.
        artifact_title(Run::new(42, 2533), 17),
        format!(
            "{DESIGN_TITLE_PREFIX}{}",
            artifact_title(Run::new(41, 25330), 17)
        ),
        "onetaskgraph live cleanup 2533".to_owned(),
        "AI Orchestrator plan".to_owned(),
    ] {
        assert!(
            !is_run_artifact_title(mine, &other),
            "one run's cleanup must not match {other:?}"
        );
    }
    for other in [
        artifact_label(Run::new(41, 25330), 17),
        artifact_label(Run::new(42, 2533), 17),
        "otg-live-2533".to_owned(),
        "bug".to_owned(),
    ] {
        assert!(
            !is_run_artifact_label(mine, &other),
            "one run's label cleanup must not match {other:?}"
        );
    }
}

#[test]
fn a_second_session_of_this_lane_opens_beside_the_first_rather_than_being_declined() {
    // The seat this lane used to take is gone, and this is what that means: two sessions of
    // it on one machine no longer exclude one another. They cannot reach each other's work —
    // every artifact carries its own run's process id and the sweep above is decided on
    // that — and a seat never excluded the case that is left, which is two hosted runners.
    let held = Session::open(
        SESSION_NAME,
        Credential::new("live-token").expect("a placeholder credential is not blank"),
        Exclusivity::Shared,
    )
    .unwrap_or_else(|declined| {
        panic!(
            "this lane takes no seat, so nothing may decline it here: {}",
            declined.message()
        )
    });
    assert_eq!(held.seat_path(), None, "this lane took a seat");
    let beside = Session::open(
        SESSION_NAME,
        Credential::new("live-token").expect("a placeholder credential is not blank"),
        Exclusivity::Shared,
    )
    .unwrap_or_else(|declined| {
        panic!(
            "a second session of this lane was declined while the first was open: {}",
            declined.message()
        )
    });
    assert_eq!(beside.seat_path(), None, "this lane took a seat");
}

#[test]
fn an_unreadable_live_tier_demand_is_a_misconfiguration() {
    for unusable in ["yes", "true", "2", "on"] {
        live_lane(
            Some("live-token"),
            Some("nickderobertis"),
            Some("1"),
            Some("acme/work"),
            Some(unusable),
        )
        .expect_err(&format!(
            "ONETASKGRAPH_LIVE_REQUIRED={unusable:?} must fail rather than quietly mean not-required"
        ));
    }
    assert_eq!(
        live_lane(Some("live-token"), None, None, Some("acme/work"), Some("0")),
        live_lane(Some("live-token"), None, None, Some("acme/work"), None),
        "0 and unset both mean the lane may skip"
    );
}
