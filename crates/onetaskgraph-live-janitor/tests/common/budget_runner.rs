//! Report the offline subprocess journey's telemetry; onebudgetspec alone judges it.
use std::path::PathBuf;
use std::process::Command;

/// The test target restores this recorded request count on a cache hit.
pub fn telemetry() -> PathBuf {
    if let Some(path) = std::env::var_os("JANITOR_BUDGET_TELEMETRY") {
        return path.into();
    }
    let executable = std::env::current_exe().expect("test executable path");
    executable
        .parent()
        .expect("test executable directory")
        .parent()
        .expect("profile directory")
        .join("telemetry/onetaskgraph-live-janitor/live-janitor-requests-per-run.json")
}

fn refuse(reason: &str) -> ! {
    eprintln!("{reason}");
    std::process::exit(1)
}

// llmlint: ignore[tests_assert_real_behavior] This test executable is the portable budget command, inert when no budget is nominated. The subprocess journey below drives this exact entry point through onebudgetspec and asserts its reports and failures.
#[test]
fn report() {
    match std::env::var("ONEBUDGETSPEC_BUDGET_ID") {
        Err(std::env::VarError::NotPresent) => return,
        Ok(id) if id == "live-janitor-requests-per-run" => {}
        other => refuse(&format!("not a janitor budget id: {other:?}")),
    }
    let path = telemetry();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        refuse(&format!(
            "no telemetry at {} ({error}); run onetaskgraph-live-janitor:test first",
            path.display()
        ))
    });
    let record: serde_json::Value = serde_json::from_str(&text)
        .unwrap_or_else(|error| refuse(&format!("invalid telemetry JSON: {error}")));
    let value = record["value"]
        .as_u64()
        .unwrap_or_else(|| refuse("telemetry value must be a whole number of requests"));
    let detail = match &record["detail"] {
        serde_json::Value::Null => None,
        serde_json::Value::String(detail) => Some(detail.as_str()),
        _ => refuse("telemetry detail must be a string"),
    };
    match onebudgetspec_core::report(value as f64, detail) {
        Ok(true) => {}
        Ok(false) => refuse("ONEBUDGETSPEC_RESULT is absent; run through onebudgetspec check"),
        Err(error) => refuse(&format!("cannot report telemetry: {error}")),
    }
}

#[test]
fn budget_judge_accepts_counts_and_refuses_excess_or_missing_telemetry() {
    let directory = tempfile::tempdir().expect("scratch directory");
    let telemetry = directory.path().join("record.json");
    let budgets = directory.path().join("budgets.yaml");
    let executable = std::env::current_exe().expect("test executable");
    std::fs::write(
        &budgets,
        serde_json::json!({"schema_version": 1, "budgets": [{
            "id": "live-janitor-requests-per-run", "measure": "reported",
            "unit": "requests", "direction": "max", "threshold": 250,
            "command": [executable, "budget_runner::report", "--exact", "--nocapture"]
        }]})
        .to_string(),
    )
    .expect("budget file");
    for (record, verdict) in [
        (Some("{\"value\": 200}"), "within"),
        (Some("{\"value\": 251}"), "over"),
        (Some("{\"value\": -1}"), "error"),
        (Some("invalid"), "error"),
        (None, "error"),
    ] {
        if let Some(record) = record {
            std::fs::write(&telemetry, record).expect("telemetry file");
        } else {
            std::fs::remove_file(&telemetry).expect("remove telemetry");
        }
        let output = Command::new("bun")
            .args(["run", "onebudgetspec", "check", "--json"])
            .arg(&budgets)
            .current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
            .env("JANITOR_BUDGET_TELEMETRY", &telemetry)
            .output()
            .expect("workspace onebudgetspec starts");
        let result: serde_json::Value = serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|error| panic!("{error}: {}", String::from_utf8_lossy(&output.stderr)));
        assert_eq!(result["results"][0]["verdict"], verdict, "{result:#}");
        assert_eq!(output.status.success(), verdict == "within");
    }
}
