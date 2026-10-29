use onetaskgraph_linear::live_artifact::{Scope, Slot, classify_retained, retained_title};
use onetaskgraph_live::artifact::{Run, Stamp};
use std::num::NonZeroU32;

#[test]
fn retained_identity_round_trips_without_overlapping_scopes() {
    for slot in [
        Slot::Filed,
        Slot::Loose,
        Slot::FollowUp,
        Slot::Assets,
        Slot::Parent,
    ] {
        for scope in [
            Scope::ci("TEST").unwrap(),
            Scope::local("TEST", "1-2-3").unwrap(),
        ] {
            let title = retained_title(scope, slot).unwrap();
            let found = classify_retained(&title).unwrap();
            assert_eq!(found.scope, scope);
            assert_eq!(found.slot, slot);
            assert_eq!(found.scope.team(), "TEST");
            assert_eq!(found.scope.is_ci(), found.scope.local_stamp().is_none());
        }
    }
}

#[test]
fn malformed_ownership_confers_no_authority() {
    let good = retained_title(Scope::ci("TEST").unwrap(), Slot::Filed).unwrap();
    for bad in [
        format!("{good} "),
        good.replace("retained-v1", "retained-v2"),
        good.replace("filed", "unknown"),
        good.replace("TEST", "test"),
        format!("{good} 1-2-3"),
    ] {
        assert!(classify_retained(&bad).is_none(), "{bad}");
    }
    for team in ["", "test", "TEST team", "TEST\n", "TÉST"] {
        assert!(Scope::ci(team).is_err());
        assert!(Scope::local(team, "1-2-3").is_err());
    }
    for stamp in [
        "ci-1-2-3",
        "+1-2-3",
        "1-+2-3",
        "0-2-3",
        "1-0-3",
        "1-2-0",
        "01-2-3",
        "1-2-3-4",
        "4294967296-2-3",
    ] {
        assert!(Scope::local("TEST", stamp).is_err());
    }
}

#[test]
fn local_grammar_matches_canonical_positive_vouched_live_stamps_at_every_boundary() {
    // Stamp deliberately also admits unvouched host zero, timestamp zero and padded
    // decimal input. Retention requires positive ownership and its exact canonical display.
    let runs = [
        "",
        "0",
        "1",
        "01",
        "4294967295",
        "4294967296",
        "18446744073709551615",
        "+1",
        "-1",
        "a",
    ];
    let times = [
        "",
        "0",
        "1",
        "01",
        "18446744073709551615",
        "18446744073709551616",
        "+1",
        "-1",
        "a",
    ];
    for host in runs {
        for process in runs {
            for micros in times {
                let spelling = format!("{host}-{process}-{micros}");
                let expected = Stamp::read(&spelling).is_some_and(|stamp| {
                    stamp.run().host().is_some()
                        && stamp.micros() > 0
                        && stamp.to_string() == spelling
                });
                assert_eq!(
                    Scope::local("TEST", &spelling).is_ok(),
                    expected,
                    "{spelling}"
                );
                let title =
                    format!("onetaskgraph live cleanup retained-v1 local TEST filed {spelling}");
                assert_eq!(classify_retained(&title).is_some(), expected, "{spelling}");
            }
        }
    }
    for host in [1, u32::MAX] {
        for process in [1, u32::MAX] {
            for micros in [1, u64::MAX] {
                let stamp = Stamp::new(
                    Run::vouched(
                        NonZeroU32::new(host).unwrap(),
                        NonZeroU32::new(process).unwrap(),
                    ),
                    micros,
                )
                .to_string();
                assert!(Scope::local("TEST", &stamp).is_ok(), "{stamp}");
            }
        }
    }
}
