//! The live lane's own decisions, asserted without reaching GitHub.
//!
//! Which board and repository the credentialed lane may write to, which artifacts a run
//! recognises as its own and as an interrupted earlier run's, and that cleanup runs whether the
//! journey passed or failed. None of it touches the network or reads a credential, so these
//! assertions hold on every machine — including the ones the journey beside them skips on for
//! want of a credential, which is where the decisions asserted here would otherwise go
//! unproven.

mod lane;

use lane::{
    LiveLane, LiveSecret, artifact_label, artifact_title, is_artifact_label, is_artifact_title,
    is_run_artifact_title, live_lane, live_write_config, run_then_cleanup,
};
use onetaskgraph_github_projects::DESIGN_TITLE_PREFIX;
use onetaskgraph_live::Credential;
use onetaskgraph_plugin_api::{SourceName, SourcePlugin};

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
fn residue_recovery_matches_this_lanes_own_artifacts_and_nothing_else() {
    // The stale item this lane's recovery exists for: another run's process id and timestamp.
    assert!(is_artifact_title(
        "onetaskgraph live cleanup 2533-1787816134627361"
    ));
    assert!(is_artifact_title(&artifact_title(std::process::id(), 1)));
    for foreign in [
        "AI Orchestrator plan",
        "onetaskgraph live cleanup",
        "onetaskgraph live cleanup 2533",
        "onetaskgraph live cleanup abc-1787816134627361",
        "onetaskgraph live cleanup 2533-",
        "copy of onetaskgraph live cleanup 2533-1787816134627361",
    ] {
        assert!(
            !is_artifact_title(foreign),
            "residue recovery must not match {foreign:?}"
        );
    }
    // A *document* this lane writes is titled the way this source spells one — the design
    // prefix, put there by the source rather than by the caller — so cleanup reads a title
    // the artifact prefix does not start. Recognition takes that prefix off first: without
    // it, a document a run created would be residue no sweep could ever name, on somebody's
    // real board.
    assert!(is_artifact_title(&format!(
        "{DESIGN_TITLE_PREFIX}onetaskgraph live cleanup 2533-1787816134627361"
    )));
    assert!(is_artifact_title(&format!(
        "{DESIGN_TITLE_PREFIX}{}",
        artifact_title(std::process::id(), 1)
    )));
    for foreign in [
        format!("{DESIGN_TITLE_PREFIX}AI Orchestrator plan"),
        format!("{DESIGN_TITLE_PREFIX}onetaskgraph live cleanup 2533"),
        format!("copy of {DESIGN_TITLE_PREFIX}onetaskgraph live cleanup 2533-17"),
    ] {
        assert!(
            !is_artifact_title(&foreign),
            "residue recovery must not match {foreign:?}"
        );
    }

    // A run names its own by the process id every one of its artifacts shares, so it
    // cleans up after itself without touching what an interrupted earlier run left for
    // the sweep above.
    assert!(is_run_artifact_title(
        2533,
        "onetaskgraph live cleanup 2533-17"
    ));
    assert!(is_run_artifact_title(
        2533,
        &format!("{DESIGN_TITLE_PREFIX}onetaskgraph live cleanup 2533-17")
    ));
    assert!(
        !is_run_artifact_title(
            2533,
            &format!("{DESIGN_TITLE_PREFIX}onetaskgraph live cleanup 25330-17")
        ),
        "and a document of another run is another run's, exactly as an issue is"
    );
    for other in [
        "onetaskgraph live cleanup 25330-17",
        "onetaskgraph live cleanup 253-17",
        "onetaskgraph live cleanup 12533-17",
    ] {
        assert!(
            !is_run_artifact_title(2533, other),
            "one run's cleanup must not match another run's {other:?}"
        );
    }
    // The label is residue exactly as an item is, and is recognised the same way.
    assert!(is_artifact_label("onetaskgraph-live-2533-1787816134627361"));
    assert!(is_artifact_label(&artifact_label(std::process::id(), 1)));
    for foreign in [
        "bug",
        "onetaskgraph-live-",
        "onetaskgraph-live-2533",
        "onetaskgraph-live-abc-1787816134627361",
        "onetaskgraph-live-2533-",
        "not-onetaskgraph-live-2533-1787816134627361",
    ] {
        assert!(
            !is_artifact_label(foreign),
            "label recovery must not match {foreign:?}"
        );
    }
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
