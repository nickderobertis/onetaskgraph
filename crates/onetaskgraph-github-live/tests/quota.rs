//! The internal versioned record has one serde definition and a checked-in wire golden.
use onetaskgraph_github_live::quota::{Charge, SessionRecord};

#[test]
fn current_schema_round_trips_without_inventing_unknown_orphans_or_allowance() {
    let golden: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/session-v3.json")).unwrap();
    let record: SessionRecord = serde_json::from_value(golden.clone()).unwrap();
    assert_eq!(
        record.schema_version,
        onetaskgraph_github_live::quota::SCHEMA_VERSION
    );
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
        serde_json::from_str(include_str!("fixtures/session-v3.json")).unwrap();
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
        serde_json::from_str(include_str!("fixtures/session-v3.json")).unwrap();
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
        &CurrentSession::new(
            &expected.session_id,
            &expected.source_revision,
            &expected.workspace_version,
            &expected.workload,
        )?,
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

#[test]
fn reader_refuses_malformed_request_and_allowance_facts() {
    use onetaskgraph_github_live::quota::{Charge, CurrentSession, MutationBasis, RequestKind};
    let good = completed_pair();
    let identity = &good[0];
    for (id, revision, version) in [
        ("", identity.source_revision.as_str(), "0.6.0"),
        ("job\n42", identity.source_revision.as_str(), "0.6.0"),
        ("job", "bad", "0.6.0"),
        ("job", "gggggggggggggggggggggggggggggggggggggggg", "0.6.0"),
        ("job", identity.source_revision.as_str(), ""),
        ("job", identity.source_revision.as_str(), "0.6.0\n"),
    ] {
        assert!(CurrentSession::new(id, revision, version, &identity.workload).is_err());
    }
    for mutate in [
        |record: &mut SessionRecord| {
            record.schema_version = onetaskgraph_github_live::quota::SCHEMA_VERSION + 1
        },
        |record: &mut SessionRecord| record.calls.clear(),
        |record: &mut SessionRecord| record.calls[0].sequence = 2,
        |record: &mut SessionRecord| record.calls[0].attempt = 0,
        |record: &mut SessionRecord| record.calls[0].operation_sha256.clear(),
        |record: &mut SessionRecord| record.calls[0].operation_sha256 = "g".repeat(64),
        |record: &mut SessionRecord| record.calls[0].kind = RequestKind::Rest,
        |record: &mut SessionRecord| {
            record.calls[0].charge = Charge::ReportedQuery { points: u64::MAX }
        },
        |record: &mut SessionRecord| record.calls[0].allowance.as_mut().unwrap().used += 1,
        |record: &mut SessionRecord| record.calls[0].allowance.as_mut().unwrap().used = u64::MAX,
        |record: &mut SessionRecord| {
            record.calls[0].kind = RequestKind::Mutation;
            record.calls[0].charge = Charge::Mutation {
                points: 1,
                basis: MutationBasis::Documented {
                    url: "wrong".into(),
                },
            };
        },
        |record: &mut SessionRecord| {
            record.calls[0].kind = RequestKind::Mutation;
            record.calls[0].charge = Charge::Mutation {
                points: 1,
                basis: MutationBasis::Modelled {
                    url: "wrong".into(),
                    points_per_request: 1,
                },
            };
        },
        |record: &mut SessionRecord| {
            record.calls[0].kind = RequestKind::Mutation;
            record.calls[0].charge = Charge::Mutation {
                points: 2,
                basis: MutationBasis::Modelled {
                    url: onetaskgraph_github_live::quota::PRICING_URL.into(),
                    points_per_request: 1,
                },
            };
        },
    ] {
        let mut pair = good.clone();
        mutate(&mut pair[0]);
        assert!(read_pair(&pair).is_err());
    }
}

#[test]
fn mutation_models_rest_exclusion_and_phase_aggregation_survive_serialization() {
    use onetaskgraph_github_live::quota::{Charge, MutationBasis, PRICING_URL, Phase, RequestKind};
    let mut pair = completed_pair();
    for (index, basis) in [
        MutationBasis::Modelled {
            url: PRICING_URL.into(),
            points_per_request: 1,
        },
        MutationBasis::Documented {
            url: PRICING_URL.into(),
        },
    ]
    .into_iter()
    .enumerate()
    {
        let call = &mut pair[0].calls[index];
        call.kind = RequestKind::Mutation;
        call.charge = Charge::Mutation { points: 1, basis };
    }
    let mut rest = pair[1].calls[4].clone();
    rest.sequence = 6;
    rest.kind = RequestKind::Rest;
    rest.charge = Charge::Rest;
    rest.allowance.as_mut().unwrap().reset_epoch_seconds += 1;
    pair[1].calls.push(rest);
    // Exercise the wire boundary instead of handing only in-memory structs to the reader.
    let saved = serde_json::to_string(&pair).unwrap();
    let records: Vec<SessionRecord> = serde_json::from_str(&saved).unwrap();
    let result = read_pair(&records).unwrap();
    assert_eq!(result.points, 14);
    assert_eq!(result.mutations, 2);
    assert_eq!(result.query_points, 12);
    assert_eq!(result.modelled_mutation_points, 1);
    assert_eq!(result.modelled_mutations, 1);
    assert_eq!(result.documented_mutation_points, 1);
    assert_eq!(result.documented_mutations, 1);
    assert_eq!(result.rest_requests, 1);
    assert_eq!(result.graphql_requests, 14);
    assert_eq!(result.account_movement, Some(0));
    assert_eq!(
        result
            .phases
            .iter()
            .find(|(phase, _)| *phase == Phase::Admission),
        Some(&(Phase::Admission, 2))
    );
    assert_eq!(
        result.phases.iter().map(|(_, points)| points).sum::<u64>(),
        result.points
    );
}

#[test]
fn corroboration_handles_absence_limits_regressions_and_flag_boundary() {
    let mut pair = completed_pair();
    let last = pair[1]
        .calls
        .last_mut()
        .unwrap()
        .allowance
        .as_mut()
        .unwrap();
    last.used = 528;
    last.remaining = 4472;
    let result = read_pair(&pair).unwrap();
    assert_eq!(result.account_movement, Some(28));
    assert!(!result.movement_flagged);
    let last = pair[1]
        .calls
        .last_mut()
        .unwrap()
        .allowance
        .as_mut()
        .unwrap();
    last.used += 1;
    last.remaining -= 1;
    assert!(read_pair(&pair).unwrap().movement_flagged);
    for mutate in [
        |record: &mut SessionRecord| record.calls[2].allowance = None,
        |record: &mut SessionRecord| {
            let allowance = record.calls[2].allowance.as_mut().unwrap();
            allowance.limit += 1;
            allowance.remaining += 1;
        },
        |record: &mut SessionRecord| {
            let allowance = record.calls[2].allowance.as_mut().unwrap();
            allowance.used -= 1;
            allowance.remaining += 1;
        },
    ] {
        let mut changed = completed_pair();
        mutate(&mut changed[0]);
        let result = read_pair(&changed).unwrap();
        if changed[0].calls[2].allowance.is_none() {
            assert_eq!(result.account_movement, Some(0));
        } else {
            assert_eq!(result.account_movement, None);
        }
    }
    for record in &mut pair {
        for call in &mut record.calls {
            call.allowance = None;
        }
    }
    assert_eq!(read_pair(&pair).unwrap().account_movement, None);
}

#[test]
fn current_identity_boundaries_and_every_main_phase_are_validated() {
    use onetaskgraph_github_live::quota::{CurrentSession, bill};
    let pair = completed_pair();
    let identity = &pair[0];
    assert!(
        CurrentSession::new(
            &"x".repeat(257),
            &identity.source_revision,
            &identity.workspace_version,
            &identity.workload
        )
        .is_err()
    );
    let long_id = "x".repeat(256);
    let long_revision = "a".repeat(64);
    let current = CurrentSession::new(
        &long_id,
        &long_revision,
        &identity.workspace_version,
        &identity.workload,
    )
    .unwrap();
    let mut long_pair = pair.clone();
    for record in &mut long_pair {
        record.session_id = long_id.clone();
        record.source_revision = long_revision.clone();
    }
    assert!(bill(&long_pair, &current).is_ok());
    for invocation in 0..pair.len() {
        for index in 0..pair[invocation].calls.len() {
            let mut missing = pair.clone();
            missing[invocation].calls.remove(index);
            for (index, call) in missing[invocation].calls.iter_mut().enumerate() {
                call.sequence = index as u64 + 1;
            }
            assert!(
                read_pair(&missing)
                    .unwrap_err()
                    .contains("required journey phase")
            );
        }
    }
}

#[test]
fn failed_attempts_and_successful_retries_all_contribute_their_own_prices() {
    use onetaskgraph_github_live::quota::{
        Charge, MutationBasis, Outcome, PRICING_URL, RequestKind,
    };
    let mut pair = completed_pair();
    for outcome in [
        Outcome::GraphqlError,
        Outcome::HttpError,
        Outcome::TransportError,
    ] {
        let mut failed = pair[0].calls[0].clone();
        failed.sequence = pair[0].calls.len() as u64 + 1;
        failed.kind = RequestKind::Mutation;
        failed.outcome = outcome;
        failed.charge = Charge::Mutation {
            points: 1,
            basis: MutationBasis::Modelled {
                url: PRICING_URL.into(),
                points_per_request: 1,
            },
        };
        pair[0].calls.push(failed.clone());
        failed.sequence += 1;
        failed.attempt = 2;
        failed.outcome = Outcome::Success;
        pair[0].calls.push(failed);
    }
    let wire = serde_json::to_string(&pair).unwrap();
    let result = read_pair(&serde_json::from_str::<Vec<SessionRecord>>(&wire).unwrap()).unwrap();
    assert_eq!(result.points, 20);
    assert_eq!(result.mutations, 6);
    assert_eq!(result.graphql_requests, 20);
    // Queries without a response cost refuse numeric reporting even after recovery.
    pair[0].calls[0].outcome = Outcome::TransportError;
    pair[0].calls[0].charge = Charge::Unavailable {
        reason: "transport failure".into(),
    };
    assert!(read_pair(&pair).is_err());
}

#[test]
fn file_backed_invocations_and_reader_refuse_partial_stale_duplicate_and_corrupt_pairs() {
    use onetaskgraph_github_live::quota::{CurrentSession, InvocationWriter, read_bill};
    let directory = tempfile::tempdir().unwrap();
    let pair = completed_pair();
    let expected = &pair[0];
    let current = CurrentSession::new(
        &expected.session_id,
        &expected.source_revision,
        &expected.workspace_version,
        &expected.workload,
    )
    .unwrap();
    assert!(read_bill(directory.path(), &current).is_err());
    let mut writers = Vec::new();
    for record in &pair {
        let mut initial = record.clone();
        initial.calls.clear();
        let mut writer = InvocationWriter::start(directory.path(), initial.clone()).unwrap();
        writer.observe_initial_board(91, 15).unwrap();
        let filename = match record.invocation {
            onetaskgraph_github_live::quota::EntryPoint::Main => "main.json",
            onetaskgraph_github_live::quota::EntryPoint::Assets => "assets.json",
        };
        let observed: SessionRecord =
            serde_json::from_slice(&std::fs::read(directory.path().join(filename)).unwrap())
                .unwrap();
        assert_eq!(observed.workload.initial_items, 91);
        assert_eq!(observed.workload.initial_fields, 15);
        writer.observe_initial_board(90, 14).unwrap();
        assert!(InvocationWriter::start(directory.path(), initial).is_err());
        for call in &record.calls {
            let mut call = call.clone();
            call.sequence = 999; // The writer, rather than a caller, owns sequence assignment.
            writer.record(call).unwrap();
        }
        assert!(read_bill(directory.path(), &current).is_err());
        writers.push(writer);
    }
    writers[0].complete().unwrap();
    assert!(read_bill(directory.path(), &current).is_err());
    writers[1].complete().unwrap();
    assert_eq!(
        read_bill(directory.path(), &current).unwrap(),
        read_pair(&pair).unwrap()
    );
    assert!(writers[0].complete().is_err());
    assert!(writers[0].observe_initial_board(91, 15).is_err());
    assert!(writers[0].record(pair[0].calls[0].clone()).is_err());
    let stale = CurrentSession::new(
        "other-job",
        &expected.source_revision,
        &expected.workspace_version,
        &expected.workload,
    )
    .unwrap();
    assert!(read_bill(directory.path(), &stale).is_err());
    std::fs::copy(
        directory.path().join("main.json"),
        directory.path().join("duplicate.json"),
    )
    .unwrap();
    assert!(read_bill(directory.path(), &current).is_err());
    std::fs::remove_file(directory.path().join("duplicate.json")).unwrap();
    std::fs::write(directory.path().join("assets.json"), "{ truncated").unwrap();
    assert!(read_bill(directory.path(), &current).is_err());
    std::fs::remove_file(directory.path().join("assets.json")).unwrap();
    assert!(read_bill(directory.path(), &current).is_err());
    assert!(read_bill(&directory.path().join("missing"), &current).is_err());
    let mut invalid = pair[0].clone();
    invalid.calls.clear();
    invalid.schema_version = onetaskgraph_github_live::quota::SCHEMA_VERSION + 1;
    assert!(InvocationWriter::start(directory.path(), invalid).is_err());
    assert!(InvocationWriter::start(directory.path(), pair[0].clone()).is_err());
    let mut initial = pair[0].clone();
    initial.calls.clear();
    assert!(InvocationWriter::start(&directory.path().join("missing"), initial).is_err());
}

#[test]
fn asset_admission_requires_the_current_complete_and_priced_main_file() {
    use onetaskgraph_github_live::quota::{CurrentSession, require_current_main};
    let good = completed_pair()[0].clone();
    let current = CurrentSession::new(
        &good.session_id,
        &good.source_revision,
        &good.workspace_version,
        &good.workload,
    )
    .unwrap();
    let directory = tempfile::tempdir().unwrap();
    assert!(require_current_main(directory.path(), &current).is_err());
    std::fs::write(
        directory.path().join("main.json"),
        serde_json::to_vec(&good).unwrap(),
    )
    .unwrap();
    require_current_main(directory.path(), &current).unwrap();
    for change in [
        |record: &mut SessionRecord| record.complete = false,
        |record: &mut SessionRecord| record.session_id.push('x'),
        |record: &mut SessionRecord| record.source_revision.push('x'),
        |record: &mut SessionRecord| record.workspace_version.push('x'),
        |record: &mut SessionRecord| record.workload.asset_bytes.clear(),
        |record: &mut SessionRecord| {
            record.invocation = onetaskgraph_github_live::quota::EntryPoint::Assets
        },
        |record: &mut SessionRecord| {
            record.calls.pop();
        },
        |record: &mut SessionRecord| {
            record.calls[0].charge = Charge::Unavailable {
                reason: "missing actual query price".into(),
            }
        },
    ] {
        let mut record = good.clone();
        change(&mut record);
        std::fs::write(
            directory.path().join("main.json"),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
        assert!(require_current_main(directory.path(), &current).is_err());
    }
}

#[test]
fn absent_asset_targets_round_trip_without_inventing_an_old_field() {
    let mut json: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/session-v3.json")).unwrap();
    json["workload"]
        .as_object_mut()
        .unwrap()
        .remove("asset_target_bytes");
    let record: onetaskgraph_github_live::quota::SessionRecord =
        serde_json::from_value(json).unwrap();
    assert!(record.workload.asset_target_bytes.is_empty());
    assert!(
        serde_json::to_value(record).unwrap()["workload"]
            .get("asset_target_bytes")
            .is_none()
    );
}

#[test]
fn only_one_pinned_deliberate_refusal_can_use_the_approved_minimum_model() {
    use onetaskgraph_github_live::quota::{
        Outcome, PAGE_SIZE_PROBE_DOCUMENT, PRICING_URL, Phase, operation_sha256,
    };
    let mut pair = completed_pair();
    let call = &mut pair[0].calls[4];
    call.charge = Charge::ModelledPageSizeRefusal {
        points: 1,
        url: PRICING_URL.to_owned(),
    };
    call.operation_sha256 = operation_sha256(PAGE_SIZE_PROBE_DOCUMENT);
    call.outcome = Outcome::GraphqlError;
    let bill = read_pair(&pair).unwrap();
    assert_eq!(bill.modelled_page_refusals, 1);
    assert_eq!(bill.modelled_page_refusal_points, 1);
    for invalidate in [
        |call: &mut onetaskgraph_github_live::quota::CallRecord| call.outcome = Outcome::HttpError,
        |call: &mut onetaskgraph_github_live::quota::CallRecord| call.phase = Phase::Schema,
        |call: &mut onetaskgraph_github_live::quota::CallRecord| {
            call.operation_sha256 = "b".repeat(64)
        },
        |call: &mut onetaskgraph_github_live::quota::CallRecord| {
            call.charge = Charge::ModelledPageSizeRefusal {
                points: 2,
                url: PRICING_URL.to_owned(),
            }
        },
        |call: &mut onetaskgraph_github_live::quota::CallRecord| {
            call.charge = Charge::ModelledPageSizeRefusal {
                points: 1,
                url: "https://example.test".into(),
            }
        },
    ] {
        let mut invalid = pair.clone();
        invalidate(&mut invalid[0].calls[4]);
        assert!(read_pair(&invalid).is_err());
    }
    let mut duplicate = pair.clone();
    let mut call = duplicate[0].calls[4].clone();
    call.sequence = duplicate[0].calls.len() as u64 + 1;
    duplicate[0].calls.push(call);
    assert!(read_pair(&duplicate).is_err());
}
