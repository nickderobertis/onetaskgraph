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

#[test]
fn pool_members_round_trip_and_stay_distinct_from_each_other_and_the_pre_pool_set() {
    use onetaskgraph_linear::live_artifact::POOL_SIZE;
    let legacy = retained_title(Scope::ci("TEST").unwrap(), Slot::Filed).unwrap();
    let mut seen = std::collections::BTreeSet::from([legacy.clone()]);
    for index in 1..=POOL_SIZE {
        let scope = Scope::pooled("TEST", index).unwrap();
        let title = retained_title(scope, Slot::Filed).unwrap();
        assert!(seen.insert(title.clone()), "{title} repeats");
        let found = classify_retained(&title).unwrap();
        assert_eq!(
            (found.scope, found.scope.pool_index()),
            (scope, Some(index))
        );
        assert!(found.scope.is_ci());
    }
    assert_eq!(classify_retained(&legacy).unwrap().scope.pool_index(), None);
    for index in [0, POOL_SIZE + 1] {
        assert!(Scope::pooled("TEST", index).is_err());
    }
    for bad in ["pool-0", "pool-01", "pool-17", "pool-", "pool-x", "7"] {
        assert!(
            classify_retained(&format!("{legacy} {bad}")).is_none(),
            "{bad}"
        );
    }
}

#[test]
fn lease_titles_round_trip_and_malformed_ones_confer_no_lease() {
    use onetaskgraph_linear::live_artifact::{
        Lease, LeaseHolder, LeaseLane, classify_lease, is_reserved_title, lease_title,
        lease_title_prefix,
    };
    for holder in [
        LeaseHolder::Held("1-2-3"),
        LeaseHolder::Held("0-2-3"),
        LeaseHolder::Released,
    ] {
        for lane in [LeaseLane::Plugin, LeaseLane::Assets] {
            let lease = Lease {
                team: "TEST",
                lane,
                index: 16,
                generation: 41,
                holder,
            };
            let title = lease_title(lease).unwrap();
            assert!(title.starts_with(&lease_title_prefix("TEST", lane)));
            assert_eq!(classify_lease(&title), Some(lease));
            assert!(is_reserved_title(&title));
            assert!(classify_retained(&title).is_none());
        }
    }
    let good = lease_title(Lease {
        team: "TEST",
        lane: LeaseLane::Plugin,
        index: 3,
        generation: 1,
        holder: LeaseHolder::Held("1-2-3"),
    })
    .unwrap();
    for bad in [
        format!("{good} "),
        good.replace("gen-1", "gen-0"),
        good.replace("gen-1", "gen-01"),
        good.replace("pool-3", "pool-17"),
        good.replace("plugin", "other"),
        good.replace("1-2-3", "ci-1-2-3"),
        good.replace("lease-v1", "lease-v2"),
    ] {
        assert!(classify_lease(&bad).is_none(), "{bad}");
    }
    assert!(
        lease_title(Lease {
            generation: 0,
            ..classify_lease(&good).unwrap()
        })
        .is_err()
    );
}
