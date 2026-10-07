//! The GitHub lane's artifact names through their public readers.
use onetaskgraph_github_live::*;
use onetaskgraph_live::artifact::{CiRun, CiStamp, WIDEST_CI_STAMP, WIDEST_STAMP};

fn ci(run_id: u64, attempt: u64) -> CiRun {
    CiRun::new(run_id, attempt).expect("a CI run within the grammar")
}

#[test]
fn the_widest_stamp_of_either_form_fits_a_github_label_and_title() {
    let widest = CiStamp::new(
        CiRun::new(99_999_999_999_999_999, 999).expect("the widest run is admitted"),
        9_999_999_999_999_999,
    )
    .expect("the widest stamp is admitted")
    .to_string();
    let label = format!("{LABEL_PREFIX}{widest}");
    assert_eq!(labelled_stamp(&label), Some(widest.as_str()));
    assert!(label.len() <= LABEL_LIMIT);
    assert!(LABEL_PREFIX.len() + WIDEST_STAMP.max(WIDEST_CI_STAMP) <= LABEL_LIMIT);
    assert!(
        ARTIFACT_PREFIX.len() + WIDEST_CI_STAMP <= 256,
        "an issue title GitHub accepts"
    );
}

#[test]
fn a_title_or_label_names_a_stamp_only_with_the_callers_prefix() {
    // The caller supplies its document prefix; the plugin lane tests its own constant.
    let design = "document: ";
    assert_eq!(
        titled_stamp("onetaskgraph live cleanup ci-1-1-1", design),
        Some("ci-1-1-1")
    );
    assert_eq!(
        titled_stamp("document: onetaskgraph live cleanup 4-5-6", design),
        Some("4-5-6")
    );
    for foreign in [
        "onetaskgraph live cleanup",
        "copy of onetaskgraph live cleanup ci-1-1-1",
        "document: document: onetaskgraph live cleanup ci-1-1-1",
        "copy of document: onetaskgraph live cleanup ci-1-1-1",
        "an ordinary feature",
    ] {
        assert_eq!(titled_stamp(foreign, design), None, "{foreign:?}");
    }
    assert_eq!(labelled_stamp("otg-live-ci-1-1-1"), Some("ci-1-1-1"));
    assert_eq!(labelled_stamp("bug"), None);
}

#[test]
fn the_lane_writes_to_a_repository_it_does_not_refuse() {
    assert_ne!(
        SCRATCH_REPOSITORY.to_ascii_lowercase(),
        CORE_REPOSITORY.to_ascii_lowercase()
    );
    for repository in [SCRATCH_REPOSITORY, CORE_REPOSITORY] {
        assert_eq!(
            repository.split_once('/').map(|(owner, _)| owner),
            Some(BOARD_OWNER)
        );
    }
    assert_eq!(BOARD_NUMBER, 1);
}

#[test]
fn a_lane_in_github_actions_names_its_run_or_refuses_naming_the_variable() {
    let read = |actions: Option<&str>, id: Option<&str>, attempt: Option<&str>| {
        ci_run(actions, id, attempt)
    };
    assert_eq!(
        read(Some("true"), Some("37616803489"), Some("2")),
        Ok(Some(ci(37_616_803_489, 2)))
    );
    // Outside GitHub Actions — not set, or anything but `true` — a machine stamp, whatever
    // the other two say.
    for actions in [None, Some("false"), Some(""), Some("TRUE"), Some("1")] {
        assert_eq!(read(actions, None, None), Ok(None), "{actions:?}");
        assert_eq!(read(actions, Some("0"), Some("x")), Ok(None), "{actions:?}");
    }
    for (id, attempt, named) in [
        (None, Some("1"), RUN_ID_VARIABLE),
        (Some(""), Some("1"), RUN_ID_VARIABLE),
        (Some("abc"), Some("1"), RUN_ID_VARIABLE),
        (Some("0"), Some("1"), RUN_ID_VARIABLE),
        (Some("012"), Some("1"), RUN_ID_VARIABLE),
        (Some("-12"), Some("1"), RUN_ID_VARIABLE),
        (Some("100000000000000000"), Some("1"), RUN_ID_VARIABLE),
        (Some("12"), None, RUN_ATTEMPT_VARIABLE),
        (Some("12"), Some("one"), RUN_ATTEMPT_VARIABLE),
        (Some("12"), Some("0"), RUN_ATTEMPT_VARIABLE),
        (Some("12"), Some("1000"), RUN_ATTEMPT_VARIABLE),
    ] {
        let refusal = read(Some("true"), id, attempt)
            .expect_err("a CI run that cannot name itself writes nothing");
        assert!(
            refusal.contains(named) && refusal.contains(GITHUB_ACTIONS_VARIABLE),
            "{id:?} {attempt:?}: {refusal}"
        );
    }
}
