//! The shared CI artifact contract through its public writers and readers.
use onetaskgraph_live::artifact::*;
use std::num::NonZeroU32;
const WINDOW: std::time::Duration = std::time::Duration::from_secs(60);
const NOW: u64 = 1_787_816_134_627_361;
fn ci(run_id: u64, attempt: u64) -> CiRun {
    CiRun::new(run_id, attempt).expect("a CI run within the grammar")
}

#[test]
fn a_ci_stamp_reads_back_exactly_what_was_written() {
    let stamp = CiStamp::new(ci(37_616_803_489, 1), 1_791_374_400_000_000)
        .expect("a stamp within the grammar");
    assert_eq!(stamp.to_string(), "ci-37616803489-1-1791374400000000");
    assert_eq!(CiStamp::read(&stamp.to_string()), Some(stamp));
    assert_eq!(stamp.run().run_id(), 37_616_803_489);
    assert_eq!(stamp.run().attempt(), 1);
    assert_eq!(stamp.micros(), 1_791_374_400_000_000);
    // The narrowest value of every field, and a stamp written now.
    let narrowest = CiStamp::new(ci(1, 1), 1).expect("ones are within the grammar");
    assert_eq!(narrowest.to_string(), "ci-1-1-1");
    assert_eq!(CiStamp::read("ci-1-1-1"), Some(narrowest));
    let now = CiStamp::new(ci(9, 2), now_micros()).expect("the clock is within the grammar");
    assert_eq!(CiStamp::read(&now.to_string()), Some(now));
}

#[test]
fn the_widest_ci_stamp_is_exactly_as_wide_as_the_widest_machine_stamp() {
    let widest = CiStamp::new(ci(99_999_999_999_999_999, 999), 9_999_999_999_999_999)
        .expect("the widest value of every field is admitted")
        .to_string();
    assert_eq!(widest.len(), 41);
    assert_eq!(widest.len(), WIDEST_CI_STAMP);
    assert_eq!(WIDEST_CI_STAMP, WIDEST_STAMP);
    assert_eq!(
        CiStamp::read(&widest).map(|read| read.to_string()),
        Some(widest)
    );
    // One past each width is no CI run and no CI stamp.
    assert_eq!(CiRun::new(100_000_000_000_000_000, 1), None);
    assert_eq!(CiRun::new(1, 1_000), None);
    assert_eq!(CiRun::new(0, 1), None);
    assert_eq!(CiRun::new(1, 0), None);
    assert_eq!(CiStamp::new(ci(1, 1), 10_000_000_000_000_000), None);
    assert_eq!(CiStamp::new(ci(1, 1), 0), None);
}

#[test]
fn a_suffix_outside_the_ci_grammar_is_not_read_as_a_ci_stamp() {
    for spelled in [
        // A zero field, and a leading zero, each of the three.
        "ci-0-1-17",
        "ci-1-0-17",
        "ci-1-1-0",
        "ci-01-1-17",
        "ci-1-01-17",
        "ci-1-1-017",
        // One digit wider than each field admits.
        "ci-100000000000000000-1-17",
        "ci-1-1000-17",
        "ci-1-1-10000000000000000",
        // A missing field, an empty one, and one too many.
        "ci-1-17",
        "ci-1--17",
        "ci--1-17",
        "ci-1-1-",
        "ci-",
        "ci",
        "ci-1-1-17-4",
        // A sign, a space, a letter, and the wrong case.
        "ci-+1-1-17",
        "ci--1-1-17",
        "ci-1-+1-17",
        "ci-1-1-+17",
        "ci- 1-1-17",
        "ci-1-1-17 ",
        "ci-1-a-17",
        "CI-1-1-17",
        "c-1-1-17",
        "",
    ] {
        assert_eq!(
            CiStamp::read(spelled),
            None,
            "{spelled:?} is not a CI stamp"
        );
    }
}

#[test]
fn each_reader_refuses_the_other_forms_stamps() {
    // Every machine stamp — of a vouched run, an unvouched one, the widest — is no CI
    // stamp, and the machine reader reads each exactly as it always did.
    for machine in [
        Stamp::new(
            Run::vouched(NonZeroU32::new(41).unwrap(), NonZeroU32::new(2533).unwrap()),
            NOW,
        ),
        Stamp::new(Run::unvouched(NonZeroU32::new(2533).unwrap()), NOW),
        Stamp::new(
            Run::vouched(
                NonZeroU32::new(999_999_999).unwrap(),
                NonZeroU32::new(u32::MAX).unwrap(),
            ),
            u64::MAX,
        ),
        Stamp::new(
            Run::vouched(NonZeroU32::new(1).unwrap(), NonZeroU32::new(1).unwrap()),
            0,
        ),
    ] {
        let spelled = machine.to_string();
        assert_eq!(
            CiStamp::read(&spelled),
            None,
            "{spelled:?} is a machine stamp"
        );
        assert_eq!(Stamp::read(&spelled), Some(machine));
    }
    // And every CI stamp is no machine stamp, so the lock sweep never acts on one.
    for spelled in [
        "ci-37616803489-1-1791374400000000",
        "ci-1-1-1",
        "ci-99999999999999999-999-9999999999999999",
    ] {
        assert!(CiStamp::read(spelled).is_some());
        assert_eq!(Stamp::read(spelled), None, "{spelled:?} is a CI stamp");
    }
    let directory =
        std::env::temp_dir().join(format!("onetaskgraph-ci-stamp-{}", std::process::id()));
    let registry = Registry::at(&directory);
    let mine = registry.enrol();
    let sweep = registry.sweep(mine, NOW, WINDOW);
    // Whatever prefix a lane names its artifacts with.
    assert!(!sweep.names_an_orphan("otg-live-", "otg-live-ci-1-1-1"));
    assert!(!sweep.names_an_orphan("artifact ", "artifact ci-1-1-1"));
    let _ = std::fs::remove_dir_all(&directory);
}
