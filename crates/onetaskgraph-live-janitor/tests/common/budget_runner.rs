//! Report the offline subprocess journey's telemetry; onebudgetspec alone judges it.
use std::path::PathBuf;
use std::process::Command;

/// The test target restores this recorded request count on a cache hit.
pub fn telemetry(budget: &str) -> PathBuf {
    if let Some(path) = std::env::var_os("JANITOR_BUDGET_TELEMETRY") {
        return path.into();
    }
    let executable = std::env::current_exe().expect("test executable path");
    executable
        .parent()
        .expect("test executable directory")
        .parent()
        .expect("profile directory")
        .join("telemetry/onetaskgraph-live-janitor")
        .join(format!("{budget}.json"))
}

fn refuse(reason: &str) -> ! {
    eprintln!("{reason}");
    std::process::exit(1)
}

// llmlint: ignore[tests_assert_real_behavior] This test executable is the portable budget command, inert when no budget is nominated. The subprocess journey below drives this exact entry point through onebudgetspec and asserts its reports and failures.
#[test]
fn report() {
    let budget = match std::env::var("ONEBUDGETSPEC_BUDGET_ID") {
        Err(std::env::VarError::NotPresent) => return,
        Ok(id) => id,
        other => refuse(&format!("not a janitor budget id: {other:?}")),
    };
    let grammar =
        regex::Regex::new(onebudgetspec_core::model::ID_PATTERN).expect("onebudgetspec id grammar");
    if !grammar.is_match(&budget) {
        refuse("not a budget id");
    }
    let path = telemetry(&budget);
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
    if let Err(error) = onebudgetspec_core::report(value as f64, detail) {
        refuse(&format!("cannot report telemetry: {error}"));
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
    // A successful command which reports no figure must be refused by the judge itself.
    let mut specification: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&budgets).expect("budget file"))
            .expect("budget JSON");
    specification["budgets"][0]["command"] = serde_json::json!([executable, "--list"]);
    std::fs::write(&budgets, specification.to_string()).expect("budget file");
    let output = Command::new("bun")
        .args(["run", "onebudgetspec", "check", "--json"])
        .arg(&budgets)
        .current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
        .output()
        .expect("workspace onebudgetspec starts");
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).expect("judge report");
    assert!(!output.status.success());
    assert_eq!(result["results"][0]["verdict"], "error");
    assert!(
        result["results"][0]["error"]
            .as_str()
            .expect("reason")
            .contains("empty")
    );
}
