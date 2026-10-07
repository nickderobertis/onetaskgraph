//! The workflow's real executable against the paginated loopback GitHub peer.
#[path = "common/budget_runner.rs"]
mod budget_runner;

#[test]
fn offline_cleanup_journeys() {
    let result = std::process::Command::new("python3")
        .env("JANITOR_BUDGET_TELEMETRY", budget_runner::telemetry())
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/journey.py"))
        .env(
            "JANITOR_BINARY",
            env!("CARGO_BIN_EXE_onetaskgraph-live-janitor"),
        )
        .output()
        .expect("Python loopback journey starts");
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}
