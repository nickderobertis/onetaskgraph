//! The one runner of `budgets.yaml`, which every budget there names as its command.
//!
//! [`report`] measures nothing: it reads the telemetry the budget's journey recorded while this
//! target ran — the budget is the one `ONEBUDGETSPEC_BUDGET_ID` names, so no list of ids lives
//! here — and hands that figure to `onebudgetspec check` through `onebudgetspec_core::report`.
//! The comparison with the threshold is onebudgetspec's alone. In this target's ordinary run no
//! budget is named, so it reports nothing and passes. The other journey here drives it through
//! the real `onebudgetspec check`, as each budget's command does.

use std::path::Path;

use serde_json::{Value, json};

use crate::common::Sandbox;
use crate::telemetry;

#[test]
fn report() {
    let Ok(budget) = std::env::var("ONEBUDGETSPEC_BUDGET_ID") else {
        return;
    };
    let (value, detail) = telemetry::recorded(&budget).unwrap_or_else(|reason| panic!("{reason}"));
    let reported = onebudgetspec_core::report(value, detail.as_deref())
        .unwrap_or_else(|error| panic!("{budget}'s figure could not be reported: {error}"));
    assert!(
        reported,
        "ONEBUDGETSPEC_RESULT is not set, so {budget}'s figure has nowhere to go; run it through \
         `onebudgetspec check crates/onetaskgraph-linear-e2e/budgets.yaml`"
    );
}

/// A budget measured by this runner, over telemetry the caller records.
fn budget(id: &str, threshold: u32) -> Value {
    let executable = std::env::current_exe().expect("this test executable has a path");
    json!({
        "id": id, "measure": "reported", "unit": "requests", "direction": "max",
        "threshold": threshold,
        "command": [executable, "budget_runner::report", "--exact", "--nocapture"],
    })
}

#[test]
fn onebudgetspec_check_reports_what_was_recorded_and_errors_on_a_budget_nothing_recorded() {
    let sandbox = Sandbox::new();
    let recorded = sandbox.subdirectory("telemetry");
    std::fs::write(
        recorded.join("linear-requests-recorded.json"),
        json!({"value": 2, "detail": "two requests, as the journey counted them"}).to_string(),
    )
    .expect("the telemetry is writable");
    let file = sandbox.subdirectory("budgets").join("budgets.yaml");
    let budgets = json!({"schema_version": 1, "budgets": [
        budget("linear-requests-recorded", 2),
        budget("linear-requests-unrecorded", 2),
    ]});
    // JSON is YAML, so the file is written as the JSON it is.
    std::fs::write(&file, budgets.to_string()).expect("the budgets file is writable");

    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = std::process::Command::new("bun")
        .args(["run", "onebudgetspec", "check", "--json"])
        .arg(&file)
        .current_dir(&workspace)
        .env(telemetry::DIRECTORY, &recorded)
        .output()
        .expect(
            "bun runs the workspace's pinned onebudgetspec; run `just bootstrap` to install it",
        );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(3),
        "one budget errored: {stderr}"
    );
    let report: Value = serde_json::from_slice(&output.stdout).expect("a JSON report");
    let results = report["results"].as_array().expect("results");

    let within = &results[0];
    assert_eq!(within["id"], "linear-requests-recorded");
    assert_eq!(within["verdict"], "within", "{within:#}");
    assert_eq!(within["actual"], 2.0);
    assert_eq!(
        within["detail"],
        "two requests, as the journey counted them"
    );

    let errored = &results[1];
    assert_eq!(errored["id"], "linear-requests-unrecorded");
    assert_eq!(errored["verdict"], "error", "{errored:#}");
    let error = errored["error"].as_str().expect("why it errored");
    assert!(
        error.contains("linear-requests-unrecorded: no telemetry at "),
        "{error}"
    );
    assert!(
        error.contains("onetaskgraph-linear-e2e:test"),
        "it names the target that records it: {error}"
    );
}
