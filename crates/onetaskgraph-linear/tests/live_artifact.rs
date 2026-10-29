use onetaskgraph_linear::live_artifact::{Scope, Slot, classify_retained, retained_title};

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
            Scope::Ci { team: "TEST" },
            Scope::Local {
                team: "TEST",
                stamp: "1-2-3",
            },
        ] {
            let title = retained_title(scope, slot).unwrap();
            let found = classify_retained(&title).unwrap();
            assert_eq!(found.scope, scope);
            assert_eq!(found.slot, slot);
        }
    }
}

#[test]
fn malformed_ownership_confers_no_authority() {
    let good = retained_title(Scope::Ci { team: "TEST" }, Slot::Filed).unwrap();
    for bad in [
        format!("{good} "),
        good.replace("retained-v1", "retained-v2"),
        good.replace("filed", "unknown"),
        good.replace("TEST", "test"),
        format!("{good} 1-2-3"),
    ] {
        assert!(classify_retained(&bad).is_none(), "{bad}");
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
        assert!(
            retained_title(
                Scope::Local {
                    team: "TEST",
                    stamp
                },
                Slot::Loose
            )
            .is_err()
        );
    }
}
