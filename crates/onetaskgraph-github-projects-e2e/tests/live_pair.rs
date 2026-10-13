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
            && std::fs::read_to_string(&path).unwrap().contains(&source)
        {
            let binary = path.with_extension("");
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

#[test]
fn actual_entry_points_complete_one_loopback_session() {
    let sandbox = onetaskgraph_e2e_support::common::Sandbox::new();
    let directory = sandbox.subdirectory("live-pair");
    let records = directory.join("session");
    std::fs::create_dir(&records).unwrap();
    let ready = directory.join("ready");
    let fixture_log = std::fs::File::create(directory.join("fixture.log")).unwrap();
    let mut fixture = Command::new(executable("onetaskgraph-github-projects", "plugin"))
        .args(["live_pair_fixture_child", "--exact", "--nocapture"])
        .env("GITHUB_LIVE_FIXTURE_READY", &ready)
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
    assert!(bill.query_points > 0 && bill.modelled_mutations > 0 && bill.rest_requests > 0);
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
        for phase in phases {
            assert!(
                record.calls.iter().any(|call| call.phase == phase),
                "missing {phase:?}"
            );
        }
    }
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
    assert!(
        reported["detail"]
            .as_str()
            .unwrap()
            .contains("Modelled mutation points=")
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
