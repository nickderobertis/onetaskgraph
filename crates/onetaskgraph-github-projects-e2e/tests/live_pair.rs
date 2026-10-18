//! The real main and asset executables write one complete record pair against GitHub's double.
use onetaskgraph_github_live::quota::{
    CurrentSession, EntryPoint, Phase, SessionRecord, WORKSPACE_VERSION, read_bill,
    reference_workload,
};
use std::{
    io::{Read as _, Write as _},
    net::{Shutdown, TcpListener, TcpStream},
    path::PathBuf,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

/// A loopback relay in front of the fixture for one entry point: every byte it carries goes
/// both ways unchanged, and it notes when it first accepted a connection and when it last
/// carried a byte, so which entry point spoke to the fixture when is read off the wire.
struct Relay {
    origin: String,
    first: Arc<Mutex<Option<Instant>>>,
    last: Arc<Mutex<Option<Instant>>>,
}

impl Relay {
    fn to(upstream: &str) -> Self {
        let upstream = upstream
            .strip_prefix("http://")
            .expect("the fixture listens on plain HTTP")
            .to_owned();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let first = Arc::new(Mutex::new(None));
        let last = Arc::new(Mutex::new(None));
        let (accepted, carried) = (first.clone(), last.clone());
        std::thread::spawn(move || {
            for client in listener.incoming() {
                let Ok(client) = client else { return };
                accepted.lock().unwrap().get_or_insert_with(Instant::now);
                // A fixture already gone refuses the connection, as it would unrelayed.
                let Ok(server) = TcpStream::connect(&upstream) else {
                    continue;
                };
                for (mut from, mut into) in [
                    (client.try_clone().unwrap(), server.try_clone().unwrap()),
                    (server, client),
                ] {
                    let carried = carried.clone();
                    std::thread::spawn(move || {
                        let mut buffer = [0; 16 * 1024];
                        while let Ok(read) = from.read(&mut buffer) {
                            if read == 0 || into.write_all(&buffer[..read]).is_err() {
                                break;
                            }
                            *carried.lock().unwrap() = Some(Instant::now());
                        }
                        let _ = into.shutdown(Shutdown::Write);
                    });
                }
            }
        });
        Self {
            origin,
            first,
            last,
        }
    }
}

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
fn an_asset_entry_point_started_before_main_waits_for_its_record_then_completes_the_session() {
    run_pair("assets-first");
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
    // Each entry point reaches the fixture through a relay of its own: main for everything it
    // sends, and the asset entry point for its REST requests, the first of which is the
    // allowance read that follows its wait. Its GraphQL goes straight to the fixture, whose
    // attachment URLs name the origin it is reached at.
    let relays = [Relay::to(host), Relay::to(host)];
    let entry_point = |package: &str, test: &str| {
        let assets = package == "onetaskgraph-github-projects-e2e";
        let relay = &relays[usize::from(assets)];
        let graphql = if assets {
            endpoint.clone()
        } else {
            format!("{}/graphql", relay.origin)
        };
        let mut command = Command::new(executable(package, "live"));
        command
            .args([test, "--exact", "--nocapture"])
            .env("GITHUB_LIVE_SELECTION", "run")
            .env("GITHUB_LIVE_TEST_GRAPHQL", graphql)
            .env("GITHUB_LIVE_TEST_REST", &relay.origin)
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
            .env("GITHUB_RUN_ATTEMPT", "2");
        command
    };
    // With `assets-first` the asset entry point starts before main, as Nx may start it, and is
    // waiting on main's record once it has reserved its own; main then runs beside it.
    let mut waiting = None;
    if scenario == "assets-first" {
        waiting = Some(
            entry_point(
                "onetaskgraph-github-projects-e2e",
                "disposable_task_and_document_asset_copies",
            )
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
        );
        let reserved = Instant::now();
        while !records.join("assets.json").exists() {
            assert!(reserved.elapsed() < Duration::from_secs(60));
            std::thread::sleep(Duration::from_millis(10));
        }
    }
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
        let output = if package == "onetaskgraph-github-projects-e2e"
            && let Some(asset) = waiting.take()
        {
            asset.wait_with_output().unwrap()
        } else {
            entry_point(package, test).output().unwrap()
        };
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
        assert!(said.contains("18 documents agreed"));
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
        let probes: Vec<_> = main
            .calls
            .iter()
            .filter(|call| call.phase == Phase::Reconciliation)
            .collect();
        assert_eq!(probes.len(), 21);
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
        // Main ended without completing its record, and its own marker says so: an asset entry
        // point waiting on it refuses at once, naming that, rather than at the end of its bound.
        assert!(records.join("main.ended").exists());
        let asset = entry_point(
            "onetaskgraph-github-projects-e2e",
            "disposable_task_and_document_asset_copies",
        )
        .env("GITHUB_LIVE_MAIN_WAIT_SECONDS", "600")
        .output()
        .unwrap();
        let said = String::from_utf8_lossy(&asset.stderr);
        assert!(!asset.status.success(), "{said}");
        assert!(
            said.contains("the main entry point ended without completing it"),
            "{said}"
        );
        assert!(relays[1].first.lock().unwrap().is_none(), "{said}");
        let workload = reference_workload();
        let current =
            CurrentSession::new("entry-point-pair", &revision, WORKSPACE_VERSION, &workload)
                .unwrap();
        assert!(read_bill(&records, &current).is_err());
        return;
    }
    for output in &outputs {
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
    // Each entry point reports what each of its phases took, live, past the harness's capture:
    // the one place a credentialed run's log says where the session's time went. The figures
    // are the session's own: its phases' requests add up to every call its record holds, and
    // its clock only moves forward, each phase within the time the session had taken by then.
    for ((output, phases), record) in outputs
        .iter()
        .zip([
            &["Admission", "Setup", "ChangeToken", "Cleanup"][..],
            &["AssetSetup", "AssetTask", "AssetDocument"][..],
        ])
        .zip(["main.json", "assets.json"])
    {
        let said = String::from_utf8_lossy(&output.stderr);
        let timings: Vec<(String, f64, usize, f64)> = said
            .lines()
            .filter_map(|line| line.strip_prefix("live-session timing: "))
            .map(|line| {
                let (phase, rest) = line.split_once(" took ").expect("a phase and its time");
                let (took, rest) = rest.split_once("s and ").expect("its seconds");
                let (requests, rest) = rest
                    .split_once(" requests (session at ")
                    .expect("its requests");
                let at = rest.strip_suffix("s)").expect("the session's clock");
                (
                    phase.rsplit(' ').next().unwrap().to_owned(),
                    took.parse().expect("seconds"),
                    requests.parse().expect("a request count"),
                    at.parse().expect("seconds"),
                )
            })
            .collect();
        for phase in phases {
            assert!(
                timings.iter().any(|(timed, ..)| timed == phase),
                "no timing line for {phase}: {said}"
            );
        }
        let recorded: SessionRecord =
            serde_json::from_slice(&std::fs::read(records.join(record)).unwrap()).unwrap();
        assert_eq!(
            timings
                .iter()
                .map(|(_, _, requests, _)| requests)
                .sum::<usize>(),
            recorded.calls.len(),
            "the phases' requests are every call {record} holds: {said}"
        );
        let mut clock = 0.0;
        for (phase, took, _, at) in &timings {
            assert!(
                *took >= 0.0 && *took <= *at + 0.05,
                "{phase} took {took}s by {at}s"
            );
            assert!(
                *at >= clock,
                "the session's clock went back to {at}s at {phase}"
            );
            clock = *at;
        }
    }
    // An asset entry point started first sends nothing until main has sent everything: its
    // first request is a REST one, through its relay.
    if scenario == "assets-first" {
        let main_last = relays[0]
            .last
            .lock()
            .unwrap()
            .expect("main reached the fixture");
        let asset_first = relays[1]
            .first
            .lock()
            .unwrap()
            .expect("assets reached the fixture");
        assert!(
            asset_first >= main_last,
            "the asset entry point connected {:?} before main's last byte",
            main_last - asset_first
        );
    }
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
