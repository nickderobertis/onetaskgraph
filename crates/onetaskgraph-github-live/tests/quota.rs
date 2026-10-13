//! The internal versioned record has one serde definition and a checked-in wire golden.
use onetaskgraph_github_live::quota::{Charge, SessionRecord};

#[test]
fn version_one_round_trips_without_inventing_unknown_orphans_or_allowance() {
    let golden: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/session-v1.json")).unwrap();
    let record: SessionRecord = serde_json::from_value(golden.clone()).unwrap();
    assert_eq!(record.schema_version, 1);
    assert_eq!(record.workload.initial_orphans, None);
    assert_eq!(record.calls[1].allowance, None);
    assert!(matches!(record.calls[1].charge, Charge::Mutation { .. }));
    assert_eq!(serde_json::to_value(&record).unwrap(), golden);
    let decoded: SessionRecord =
        serde_json::from_str(&serde_json::to_string(&record).unwrap()).unwrap();
    assert_eq!(decoded, record);
}

#[test]
fn invalid_prices_and_unknown_outcomes_are_refused_by_the_shared_reader() {
    let golden: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/session-v1.json")).unwrap();
    for invalid in [serde_json::json!(-1), serde_json::json!(1.5)] {
        let mut value = golden.clone();
        value["calls"][0]["charge"]["ReportedQuery"]["points"] = invalid;
        assert!(serde_json::from_value::<SessionRecord>(value).is_err());
    }
    let mut value = golden;
    value["calls"][0]["outcome"] = serde_json::json!("Http200");
    assert!(serde_json::from_value::<SessionRecord>(value).is_err());
}

fn completed_pair() -> Vec<SessionRecord> {
    use onetaskgraph_github_live::quota::{EntryPoint, Phase};
    let mut main: SessionRecord =
        serde_json::from_str(include_str!("fixtures/session-v1.json")).unwrap();
    main.complete = true;
    let query = main.calls[0].clone();
    main.calls = [
        Phase::Admission,
        Phase::Schema,
        Phase::Reconciliation,
        Phase::Setup,
        Phase::Capabilities,
        Phase::CommentActivity,
        Phase::ChangeToken,
        Phase::Cleanup,
        Phase::OrphanRecovery,
    ]
    .into_iter()
    .enumerate()
    .map(|(index, phase)| {
        let mut call = query.clone();
        call.sequence = index as u64 + 1;
        call.phase = phase;
        call
    })
    .collect();
    let mut assets = main.clone();
    assets.invocation = EntryPoint::Assets;
    assets.calls = [
        Phase::Admission,
        Phase::AssetSetup,
        Phase::AssetTask,
        Phase::AssetDocument,
        Phase::Cleanup,
    ]
    .into_iter()
    .enumerate()
    .map(|(index, phase)| {
        let mut call = query.clone();
        call.sequence = index as u64 + 1;
        call.phase = phase;
        call
    })
    .collect();
    vec![main, assets]
}

fn read_pair(records: &[SessionRecord]) -> Result<onetaskgraph_github_live::quota::Bill, String> {
    use onetaskgraph_github_live::quota::{CurrentSession, bill};
    let expected = completed_pair().remove(0);
    bill(
        records,
        &CurrentSession {
            session_id: &expected.session_id,
            source_revision: &expected.source_revision,
            workspace_version: &expected.workspace_version,
            workload: &expected.workload,
        },
    )
}

#[test]
fn complete_pair_attributes_response_prices_and_keeps_account_activity_separate() {
    let mut pair = completed_pair();
    pair[1].calls[4].allowance.as_mut().unwrap().used = 900;
    pair[1].calls[4].allowance.as_mut().unwrap().remaining = 4100;
    let result = read_pair(&pair).unwrap();
    assert_eq!(result.points, 14);
    assert_eq!(result.account_movement, Some(400));
    assert!(result.movement_flagged);
    // A reset in the middle invalidates even matching first/last windows.
    pair[0].calls[4]
        .allowance
        .as_mut()
        .unwrap()
        .reset_epoch_seconds += 1;
    let result = read_pair(&pair).unwrap();
    assert_eq!(result.points, 14);
    assert_eq!(result.account_movement, None);
    assert!(!result.movement_flagged);
}

#[test]
fn partial_stale_duplicate_missing_price_and_missing_cleanup_records_refuse_a_bill() {
    use onetaskgraph_github_live::quota::Charge;
    let good = completed_pair();
    assert!(read_pair(&good).is_ok());
    assert!(read_pair(&good[..1]).is_err());
    assert!(read_pair(&[good[0].clone(), good[0].clone()]).is_err());
    for mutate in [
        |record: &mut SessionRecord| record.complete = false,
        |record: &mut SessionRecord| record.session_id.push('x'),
        |record: &mut SessionRecord| record.source_revision.push('x'),
        |record: &mut SessionRecord| record.workspace_version.push('x'),
        |record: &mut SessionRecord| record.workload.asset_bytes.clear(),
        |record: &mut SessionRecord| {
            record.calls[0].charge = Charge::Unavailable {
                reason: "no actual cost".into(),
            }
        },
        |record: &mut SessionRecord| {
            record.calls.pop();
        },
    ] {
        let mut pair = good.clone();
        mutate(&mut pair[1]);
        assert!(read_pair(&pair).is_err());
    }
}
