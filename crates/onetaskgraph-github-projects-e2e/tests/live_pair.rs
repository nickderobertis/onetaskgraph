//! The real main and asset executables write one complete record pair against GitHub's double.
use onetaskgraph_github_live::quota::{
    CurrentSession, EntryPoint, Phase, SessionRecord, WORKSPACE_VERSION, read_bill,
    reference_workload,
};
use std::{
    path::PathBuf,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

fn executable(package: &str, target: &str) -> PathBuf {
    let directory = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .to_owned();
    let source = if target == "e2e" {
        format!("crates/{package}/tests/e2e/main.rs")
    } else {
        format!("crates/{package}/tests/{target}.rs")
    };
    let mut candidates = Vec::new();
    for entry in std::fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|ext| ext == "d")
            && path
                .file_stem()
                .unwrap()
                .to_string_lossy()
                .starts_with(&format!("{target}-"))
            && names_source(&std::fs::read_to_string(&path).unwrap(), &source)
        {
            let binary = path.with_extension(std::env::consts::EXE_EXTENSION);
            if binary.is_file() {
                candidates.push((binary.metadata().unwrap().modified().unwrap(), binary));
            }
        }
    }
    candidates.sort();
    candidates
        .pop()
        .unwrap_or_else(|| panic!("compile {package} --test {target} before this pair"))
        .1
}

/// Whether a dep-info file lists `source`. rustc writes each path with the separator the
/// host spells it in, so on Windows the file reads `crates\<package>\tests\<target>.rs`.
fn names_source(dep_info: &str, source: &str) -> bool {
    dep_info.replace('\\', "/").contains(source)
}

#[test]
fn a_dep_info_file_names_its_source_in_either_separator() {
    let source = "crates/onetaskgraph-github-projects/tests/plugin.rs";
    let linux =
        "target/debug/deps/plugin-1.d: crates/onetaskgraph-github-projects/tests/plugin.rs\n";
    let windows = "D:\\a\\target\\debug\\deps\\plugin-1.d: crates\\onetaskgraph-github-projects\\tests\\plugin.rs\n";
    assert!(names_source(linux, source));
    assert!(names_source(windows, source));
    assert!(!names_source(
        "plugin-2.d: crates\\onetaskgraph-linear\\tests\\plugin.rs\n",
        source
    ));
}

#[test]
fn actual_entry_points_complete_one_loopback_session() {
    run_pair("none");
}

#[test]
fn later_phase_refusal_retains_the_real_owned_cleanup_in_a_partial_record() {
    run_pair("setup");
}

#[test]
fn unsupported_pricing_aliases_execute_all_contracts_but_never_hide_the_unpriced_refusal() {
    run_pair("aliases");
}

#[test]
fn ignored_actual_cost_alias_executes_the_read_only_fallback_and_refuses_a_numeric_bill() {
    run_pair("ignored-aliases");
}

#[test]
fn the_deliberate_ceiling_refusal_uses_only_the_approved_disclosed_minimum_model() {
    run_pair("page-refusal");
}

#[test]
fn a_scalar_graphql_response_with_a_date_refuses_the_entry_point_without_a_parser_panic() {
    run_pair("scalar");
}

#[test]
fn an_attachment_that_declares_core_without_an_allowance_refuses_after_owned_cleanup() {
    run_pair("attachment-core-unread");
}

#[test]
fn malformed_errors_cannot_establish_the_page_ceiling_or_inherit_its_price_model() {
    run_pair("malformed-ceiling");
}

#[test]
fn a_shared_board_smaller_than_the_measured_one_runs_and_bills_within_the_budget() {
    run_pair("board-84");
}

#[test]
fn a_task_the_search_index_reports_late_within_the_bound_is_waited_for_and_the_journey_proceeds() {
    run_pair("search-behind");
}

#[test]
fn a_task_the_search_index_never_reports_ends_the_wait_at_its_bound_naming_that_task() {
    run_pair("search-never");
}

#[test]
fn missing_or_unadmitted_board_observations_never_start_the_fixture_or_make_a_bill() {
    for scenario in ["missing-items", "missing-fields", "board-91", "board-50"] {
        run_pair(scenario);
    }
}

fn run_pair(scenario: &str) {
    let decline = scenario == "setup";
    let aliases = matches!(scenario, "aliases" | "ignored-aliases");
    let sandbox = onetaskgraph_e2e_support::common::Sandbox::new();
    let directory = sandbox.subdirectory("live-pair");
    let records = directory.join("session");
    std::fs::create_dir(&records).unwrap();
    let ready = directory.join("ready");
    let fixture_log = std::fs::File::create(directory.join("fixture.log")).unwrap();
    let mut fixture = Command::new(executable("onetaskgraph-github-projects", "plugin"))
        .args(["live_pair_fixture_child", "--exact", "--nocapture"])
        .env("GITHUB_LIVE_FIXTURE_READY", &ready)
        .env("GITHUB_LIVE_FIXTURE_DECLINE", scenario)
        .stdin(Stdio::piped())
        .stdout(fixture_log.try_clone().unwrap())
        .stderr(fixture_log)
        .spawn()
        .unwrap();
    let started = Instant::now();
    while !ready.exists() {
        assert!(
            fixture.try_wait().unwrap().is_none(),
            "{}",
            std::fs::read_to_string(directory.join("fixture.log")).unwrap()
        );
        assert!(started.elapsed() < Duration::from_secs(30));
        std::thread::sleep(Duration::from_millis(10));
    }
    let endpoint = std::fs::read_to_string(&ready).unwrap();
    let host = endpoint.strip_suffix("/graphql").unwrap();
    let revision = "a".repeat(40);
    let mut outputs = Vec::new();
    for (package, test) in [
        (
            "onetaskgraph-github-projects",
            "real_projects_v2_contract_writes_and_leaves_no_residue",
        ),
        (
            "onetaskgraph-github-projects-e2e",
            "disposable_task_and_document_asset_copies",
        ),
    ] {
        let output = Command::new(executable(package, "live"))
            .args([test, "--exact", "--nocapture"])
            .env("GITHUB_LIVE_SELECTION", "run")
            .env("GITHUB_LIVE_TEST_GRAPHQL", &endpoint)
            .env("GITHUB_LIVE_TEST_REST", host)
            .env("GH_PROJECTS_TOKEN", "test-token")
            .env("GH_PROJECTS_OWNER", "octo-org")
            .env("GH_PROJECTS_NUMBER", "7")
            .env("GH_PROJECTS_REPOSITORY", "acme/work")
            .env("ONETASKGRAPH_LIVE_REQUIRED", "1")
            .env("GITHUB_LIVE_SESSION_DIR", &records)
            .env("GITHUB_LIVE_SESSION_ID", "entry-point-pair")
            .env("GITHUB_LIVE_SOURCE_REVISION", &revision)
            .env("GITHUB_ACTIONS", "true")
            .env("GITHUB_RUN_ID", "37616803489")
            .env("GITHUB_RUN_ATTEMPT", "2")
            .output()
            .unwrap();
        let success = output.status.success();
        outputs.push(output);
        if !success {
            break;
        }
    }
    drop(fixture.stdin.take());
    let fixture_status = fixture.wait().unwrap();
    let population = scenario
        .strip_prefix("board-")
        .map(|items| items.parse::<u64>().unwrap());
    if scenario.starts_with("missing-") || matches!(population, Some(91 | 50)) {
        assert_eq!(outputs.len(), 1);
        assert!(!outputs[0].status.success());
        let said = String::from_utf8_lossy(&outputs[0].stderr);
        assert!(
            said.contains(&if let Some(items) = population {
                format!(
                    "initial board workload is {items} items and 14 fields; registered workload requires {} to {} items and 14 fields",
                    onetaskgraph_github_live::quota::MINIMUM_INITIAL_ITEMS,
                    reference_workload().initial_items
                )
            } else if scenario == "missing-items" {
                "initial board item count was not reported".to_owned()
            } else {
                "initial board field count was not reported".to_owned()
            }),
            "{said}"
        );
        assert!(fixture_status.success());
        let main: SessionRecord =
            serde_json::from_slice(&std::fs::read(records.join("main.json")).unwrap()).unwrap();
        assert!(!main.complete);
        assert!(
            !main
                .calls
                .iter()
                .any(|call| call.kind == onetaskgraph_github_live::quota::RequestKind::Mutation)
        );
        if let Some(items) = population {
            assert_eq!(main.workload.initial_items, items);
        }
        assert!(!records.join("assets.json").exists());
        return;
    }
    if scenario == "search-never" {
        assert_eq!(outputs.len(), 1);
        assert!(!outputs[0].status.success());
        let said = String::from_utf8_lossy(&outputs[0].stderr);
        // How long the wait really took is the stand-in's to measure, against
        // `journey::STAND_IN_FIXTURE_WAIT`'s 1.5-second deadline, so its verdict comes first.
        assert!(
            fixture_status.success(),
            "{}",
            std::fs::read_to_string(directory.join("fixture.log")).unwrap()
        );
        assert!(
            said.contains("the live fixture never became readable: after waiting ")
                && said.contains(
                    "s against a deadline of 1.5s, GitHub's issue search for titles \
                     onetaskgraph live cleanup ci-37616803489-2-*"
                )
                && said.contains("a search still unanswered at the deadline was abandoned"),
            "{said}"
        );
        assert!(
            said.contains("never reported the task \"onetaskgraph live cleanup ci-37616803489-2-")
                && said.contains(
                    "which a read by its own id finds on the board, so the index was behind on it"
                ),
            "{said}"
        );
        assert_eq!(
            said.matches("which a read by its own id").count(),
            1,
            "only the one task the search left out is named: {said}"
        );
        let main: SessionRecord =
            serde_json::from_slice(&std::fs::read(records.join("main.json")).unwrap()).unwrap();
        assert!(!main.complete);
        assert!(main.calls.iter().any(|call| call.phase == Phase::Cleanup));
        assert!(!records.join("assets.json").exists());
        return;
    }
    if scenario == "malformed-ceiling" {
        assert_eq!(outputs.len(), 1);
        assert!(!outputs[0].status.success());
        assert!(
            String::from_utf8_lossy(&outputs[0].stderr).contains("invalid GraphQL errors envelope")
        );
        assert!(fixture_status.success());
        let main: SessionRecord =
            serde_json::from_slice(&std::fs::read(records.join("main.json")).unwrap()).unwrap();
        assert!(!main.complete);
        let refused = main
            .calls
            .iter()
            .find(|call| {
                call.phase == Phase::Capabilities
                    && matches!(
                        call.charge,
                        onetaskgraph_github_live::quota::Charge::Unavailable { .. }
                    )
            })
            .unwrap();
        assert_eq!(
            refused.outcome,
            onetaskgraph_github_live::quota::Outcome::TransportError
        );
        assert!(main.calls.iter().any(|call| call.phase == Phase::Cleanup));
        return;
    }
    if scenario == "attachment-core-unread" {
        assert_eq!(outputs.len(), 2);
        assert!(outputs[0].status.success());
        assert!(!outputs[1].status.success());
        assert!(
            String::from_utf8_lossy(&outputs[1].stderr)
                .contains("no allowance this session could read")
        );
        assert!(
            fixture_status.success(),
            "{}",
            std::fs::read_to_string(directory.join("fixture.log")).unwrap()
        );
        let assets: SessionRecord =
            serde_json::from_slice(&std::fs::read(records.join("assets.json")).unwrap()).unwrap();
        assert!(!assets.complete);
        assert!(assets.calls.iter().any(|call| call.phase == Phase::Cleanup));
        let workload = reference_workload();
        let current =
            CurrentSession::new("entry-point-pair", &revision, WORKSPACE_VERSION, &workload)
                .unwrap();
        assert!(read_bill(&records, &current).is_err());
        return;
    }
    if scenario == "scalar" {
        assert_eq!(outputs.len(), 1);
        assert!(!outputs[0].status.success());
        let said = String::from_utf8_lossy(&outputs[0].stderr);
        assert!(said.contains("response must be an object"), "{said}");
        assert!(fixture_status.success());
        let main: SessionRecord =
            serde_json::from_slice(&std::fs::read(records.join("main.json")).unwrap()).unwrap();
        assert!(!main.complete);
        assert_eq!(
            main.calls.last().unwrap().outcome,
            onetaskgraph_github_live::quota::Outcome::TransportError
        );
        assert!(matches!(
            main.calls.last().unwrap().charge,
            onetaskgraph_github_live::quota::Charge::Unavailable { .. }
        ));
        return;
    }
    if aliases {
        assert_eq!(
            outputs.len(),
            2,
            "main: {}; fixture: {}",
            String::from_utf8_lossy(&outputs[0].stderr),
            std::fs::read_to_string(directory.join("fixture.log")).unwrap()
        );
        assert!(
            outputs[0].status.success(),
            "{}",
            String::from_utf8_lossy(&outputs[0].stderr)
        );
        let said = String::from_utf8_lossy(&outputs[0].stderr);
        assert!(said.contains("using executed read-only reconciliation"));
        // Every read document of `graphql::DOCUMENTS` is reconciled, the closed-items read of
        // `task status reconcile` and the read by issue reference of `task status complete` among
        // them.
        assert!(said.contains("22 documents agreed"), "{said}");
        assert!(!outputs[1].status.success());
        assert!(
            String::from_utf8_lossy(&outputs[1].stderr)
                .contains("query response did not report its actual primary price")
        );
        assert!(
            fixture_status.success(),
            "{}",
            std::fs::read_to_string(directory.join("fixture.log")).unwrap()
        );
        let main: SessionRecord =
            serde_json::from_slice(&std::fs::read(records.join("main.json")).unwrap()).unwrap();
        // The phase opens with its recheck, which reads the budgets again under the 40% rule
        // with the two reads the session's admission made first; what is counted here is what
        // follows, the reconciliation's own.
        let budget_reads: Vec<&str> = main.calls[..2]
            .iter()
            .map(|call| call.operation_sha256.as_str())
            .collect();
        let phase: Vec<_> = main
            .calls
            .iter()
            .filter(|call| call.phase == Phase::Reconciliation)
            .collect();
        let rechecked = phase
            .iter()
            .take(budget_reads.len())
            .take_while(|call| budget_reads.contains(&call.operation_sha256.as_str()))
            .count();
        let probes: Vec<_> = phase[rechecked..].to_vec();
        assert_eq!(probes.len(), 25);
        assert_eq!(
            probes[1].outcome,
            if scenario == "aliases" {
                onetaskgraph_github_live::quota::Outcome::GraphqlError
            } else {
                onetaskgraph_github_live::quota::Outcome::Success
            }
        );
        assert_eq!(probes[2].attempt, 2);
        assert!(probes.iter().any(|call| matches!(
            call.charge,
            onetaskgraph_github_live::quota::Charge::Unavailable { .. }
        )));
        let workload = reference_workload();
        let current =
            CurrentSession::new("entry-point-pair", &revision, WORKSPACE_VERSION, &workload)
                .unwrap();
        assert!(read_bill(&records, &current).is_err());
        return;
    }
    if decline {
        assert_eq!(
            outputs.len(),
            1,
            "fixture: {}; main: {}",
            std::fs::read_to_string(directory.join("fixture.log")).unwrap(),
            String::from_utf8_lossy(&outputs[0].stderr)
        );
        assert!(!outputs[0].status.success());
        let said = String::from_utf8_lossy(&outputs[0].stderr);
        assert!(said.contains("remaining design demand"), "{said}");
        assert!(
            fixture_status.success(),
            "{}",
            std::fs::read_to_string(directory.join("fixture.log")).unwrap()
        );
        let main: SessionRecord =
            serde_json::from_slice(&std::fs::read(records.join("main.json")).unwrap()).unwrap();
        assert!(!main.complete);
        assert!(main.calls.iter().any(|call| call.phase == Phase::Cleanup));
        assert!(
            main.calls
                .iter()
                .any(|call| call.phase == Phase::OrphanRecovery)
        );
        assert!(
            !main
                .calls
                .iter()
                .any(|call| call.phase == Phase::Capabilities)
        );
        assert!(!records.join("assets.json").exists());
        let workload = reference_workload();
        let current =
            CurrentSession::new("entry-point-pair", &revision, WORKSPACE_VERSION, &workload)
                .unwrap();
        assert!(read_bill(&records, &current).is_err());
        return;
    }
    for output in outputs {
        assert!(
            output.status.success(),
            "stdout: {}\nstderr: {}\nfixture: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
            std::fs::read_to_string(directory.join("fixture.log")).unwrap()
        );
    }
    assert!(
        fixture_status.success(),
        "{}",
        std::fs::read_to_string(directory.join("fixture.log")).unwrap()
    );
    let workload = reference_workload();
    let current =
        CurrentSession::new("entry-point-pair", &revision, WORKSPACE_VERSION, &workload).unwrap();
    let bill = read_bill(&records, &current).unwrap();
    assert_eq!(
        bill.initial_items,
        population.unwrap_or(workload.initial_items)
    );
    assert_eq!(
        bill.modelled_page_refusals,
        u64::from(scenario == "page-refusal")
    );
    assert_eq!(
        bill.modelled_page_refusal_points,
        u64::from(scenario == "page-refusal")
    );
    assert!(bill.query_points > 0 && bill.modelled_mutations > 0 && bill.rest_requests > 0);
    // Every token the change-token leg asserts on is the plugin's own read by id.
    let token_read_hash = onetaskgraph_github_live::quota::operation_sha256(
        &onetaskgraph_github_projects::accounting::Accounting::with_query_prices()
            .priced_document(onetaskgraph_github_projects::graphql::ISSUE),
    );
    let mut reported_query_points = 0;
    let mut fresh_token_reads = 0;
    for (file, invocation, phases) in [
        (
            "main.json",
            EntryPoint::Main,
            vec![
                Phase::Schema,
                Phase::Reconciliation,
                Phase::CommentActivity,
                Phase::ChangeToken,
                Phase::Cleanup,
                Phase::OrphanRecovery,
            ],
        ),
        (
            "assets.json",
            EntryPoint::Assets,
            vec![Phase::AssetTask, Phase::AssetDocument, Phase::Cleanup],
        ),
    ] {
        let record: SessionRecord =
            serde_json::from_slice(&std::fs::read(records.join(file)).unwrap()).unwrap();
        assert_eq!(record.invocation, invocation);
        assert!(record.complete);
        for call in &record.calls {
            if let onetaskgraph_github_live::quota::Charge::ReportedQuery { points } = call.charge {
                reported_query_points += points;
            }
            if call.operation_sha256 == token_read_hash && call.phase == Phase::ChangeToken {
                fresh_token_reads += 1;
                assert_eq!(
                    call.charge,
                    onetaskgraph_github_live::quota::Charge::ReportedQuery { points: 1 },
                    "every token read must retain the loopback board's actual reported cost"
                );
            }
            if let onetaskgraph_github_live::quota::Charge::Mutation {
                points,
                basis:
                    onetaskgraph_github_live::quota::MutationBasis::Modelled {
                        points_per_request, ..
                    },
            } = &call.charge
            {
                assert_eq!(
                    *points,
                    onetaskgraph_github_live::quota::MODELLED_MUTATION_POINTS
                );
                assert_eq!(
                    *points_per_request,
                    onetaskgraph_github_live::quota::MODELLED_MUTATION_POINTS
                );
            }
        }
        for phase in phases {
            assert!(
                record.calls.iter().any(|call| call.phase == phase),
                "missing {phase:?}"
            );
        }
    }
    assert!(
        fresh_token_reads > 0,
        "the actual lane must poll fresh tokens"
    );
    assert_eq!(bill.query_points, reported_query_points);
    let result = directory.join("reported");
    let report = || {
        Command::new(executable("onetaskgraph-github-projects-e2e", "e2e"))
            .args([
                "budget_runner::report_live_session",
                "--exact",
                "--nocapture",
            ])
            .env("GITHUB_LIVE_SESSION_DIR", &records)
            .env("GITHUB_LIVE_SESSION_ID", "entry-point-pair")
            .env("GITHUB_LIVE_SOURCE_REVISION", &revision)
            .env("ONEBUDGETSPEC_BUDGET_ID", "github-live-session-points")
            .env("ONEBUDGETSPEC_RESULT", &result)
            .env_remove("GH_PROJECTS_TOKEN")
            .output()
            .unwrap()
    };
    let output = report();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let reported: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&result).unwrap()).unwrap();
    assert_eq!(
        reported["value"].as_f64(),
        Some(f64::from(u32::try_from(bill.points).unwrap()))
    );
    let detail = reported["detail"].as_str().unwrap();
    assert!(detail.contains("Modelled mutation points="));
    // A smaller board's bill names that board and never reads as the measured workload's.
    assert!(
        detail.contains(&if bill.initial_items == workload.initial_items {
            format!("initial board items={} (the registered workload)", bill.initial_items)
        } else {
            format!(
                "initial board items={} (fewer than the registered workload's {}; not its measurement)",
                bill.initial_items, workload.initial_items
            )
        }),
        "{detail}"
    );
    let configuration = onebudgetspec_core::load(
        &[PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("budgets.yaml")],
        false,
    )
    .unwrap();
    let budget = configuration
        .files()
        .iter()
        .flat_map(|file| &file.contents.budgets)
        .find(|budget| budget.id == "github-live-session-points")
        .unwrap();
    assert_eq!(
        onebudgetspec_core::judge(
            budget.direction,
            budget.threshold,
            reported["value"].as_f64().unwrap()
        )
        .0,
        onebudgetspec_core::report::Verdict::Within,
        "the registered budget judges this session's bill within its threshold"
    );
    std::fs::remove_file(&result).unwrap();
    // Removing real cleanup evidence must make the same reader refuse numeric success.
    let mut assets: SessionRecord =
        serde_json::from_slice(&std::fs::read(records.join("assets.json")).unwrap()).unwrap();
    assets.complete = false;
    std::fs::write(
        records.join("assets.json"),
        serde_json::to_vec(&assets).unwrap(),
    )
    .unwrap();
    assert!(read_bill(&records, &current).is_err());
    let output = report();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("live-session measurement unavailable")
    );
    assert!(!result.exists());
}
