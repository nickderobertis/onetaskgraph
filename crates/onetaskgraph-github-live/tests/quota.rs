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
