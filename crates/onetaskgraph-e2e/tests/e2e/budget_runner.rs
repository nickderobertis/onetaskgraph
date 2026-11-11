//! The runner of `budgets.yaml`, which every budget there names as its command.
//!
//! [`record`] is what a measuring journey calls while this crate's `test` target runs: it writes
//! the figure, how it was counted and the workload it was counted on beside the build. The
//! `report_*` entries measure nothing: each reads what its journey recorded for the budget
//! `ONEBUDGETSPEC_BUDGET_ID` names, refuses a recording of another budget or workload, and hands
//! the figure to `onebudgetspec check` through `onebudgetspec_core::report`, which alone judges
//! it. In this target's ordinary run no budget is named, so they report nothing and pass.

use std::path::PathBuf;

use serde_json::{Value, json};

use crate::boundary::{BOUNDARY_BUDGET, BOUNDARY_WORKLOAD};

/// Where this crate's telemetry is written and read: beside the build, or where
/// `ONETASKGRAPH_E2E_TELEMETRY_DIR` names, which the runner's own journey sets.
fn directory() -> PathBuf {
    if let Some(directory) = std::env::var_os("ONETASKGRAPH_E2E_TELEMETRY_DIR") {
        return directory.into();
    }
    onetaskgraph_e2e_support::telemetry::running_build_directory()
        .expect("the test executable is under a build directory")
        .join("telemetry/onetaskgraph-e2e")
}

/// Record `value` for `budget`, counted on `workload` as `detail` says.
pub(crate) fn record(budget: &str, value: f64, detail: &str, workload: &str) {
    let directory = directory();
    std::fs::create_dir_all(&directory).expect("the telemetry directory is writable");
    std::fs::write(
        onetaskgraph_e2e_support::telemetry::file_in(&directory, budget),
        json!({"budget": budget, "value": value, "detail": detail, "workload": workload})
            .to_string(),
    )
    .expect("the telemetry file is writable");
}

/// What the journey recorded for `budget`, refused unless `budget` is a budget id and the
/// recording is that budget's own, of `workload`: its figure and its detail.
fn recorded(budget: &str, workload: &str) -> Result<(f64, String), String> {
    if !regex::Regex::new(onebudgetspec_core::model::ID_PATTERN)
        .expect("onebudgetspec's id pattern compiles")
        .is_match(budget)
    {
        return Err("invalid budget id".into());
    }
    let path = onetaskgraph_e2e_support::telemetry::file_in(&directory(), budget);
    let text = std::fs::read_to_string(&path).map_err(|error| {
        format!(
            "no telemetry at {}: {error}; run `onetaskgraph-e2e:test` first",
            path.display()
        )
    })?;
    let value: Value = serde_json::from_str(&text).map_err(|error| error.to_string())?;
    if value["budget"].as_str() != Some(budget) {
        return Err("telemetry belongs to a different budget".into());
    }
    if value["workload"].as_str() != Some(workload) {
        return Err(format!("the recording is not of the {workload}"));
    }
    let figure = value["value"]
        .as_f64()
        .filter(|value| value.is_finite() && *value >= 0.0)
        .ok_or("missing nonnegative figure")?;
    let detail = value["detail"].as_str().ok_or("missing detail")?.to_owned();
    Ok((figure, detail))
}

fn refuse(message: &str) -> ! {
    eprintln!("{message}");
    std::process::exit(1)
}

// llmlint: ignore-block[tests_assert_real_behavior] A report-only command entry point budgets.yaml selects, which reports nothing without ONEBUDGETSPEC_BUDGET_ID — the harness-runner convention this repository's e2e crates register their budgets with. `the_boundary_report_hands_over_the_recorded_figure_and_refuses_another_workload` below drives it as a process and asserts what it hands over and refuses.
// llmlint: ignore-block[budget_commands_measure_directly] The task registering `boundary-invocations-per-public-copy` names this entry as its command. It reads only the telemetry `boundary::batch` recorded during this crate's ordinary test run, refuses a recording of another budget or workload, and hands the figure to onebudgetspec, which alone judges it.
#[test]
fn report_boundary() {
    let budget = match std::env::var("ONEBUDGETSPEC_BUDGET_ID") {
        Ok(budget) => budget,
        Err(std::env::VarError::NotPresent) => return,
        Err(error) => refuse(&error.to_string()),
    };
    if budget != BOUNDARY_BUDGET {
        refuse(&format!("{budget} is not the budget this entry reports"));
    }
    let (value, detail) =
        recorded(&budget, BOUNDARY_WORKLOAD).unwrap_or_else(|error| refuse(&error));
    match onebudgetspec_core::report(value, Some(&detail)) {
        Ok(true) => {}
        Ok(false) => {
            refuse("no budget figure written: ONEBUDGETSPEC_RESULT must name a destination")
        }
        Err(error) => refuse(&error.to_string()),
    }
}
// llmlint: ignore-end[budget_commands_measure_directly]
// llmlint: ignore-end[tests_assert_real_behavior]

#[test]
fn the_boundary_report_hands_over_the_recorded_figure_and_refuses_another_workload() {
    let sandbox = crate::common::Sandbox::new();
    let directory = sandbox.subdirectory("telemetry");
    let run = |recorded: Option<Value>| {
        let path = onetaskgraph_e2e_support::telemetry::file_in(&directory, BOUNDARY_BUDGET);
        let _ = std::fs::remove_file(&path);
        if let Some(recorded) = recorded {
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(&path, recorded.to_string()).unwrap();
        }
        let result = directory.join("result.json");
        let _ = std::fs::remove_file(&result);
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["budget_runner::report_boundary", "--exact", "--nocapture"])
            .env("ONETASKGRAPH_E2E_TELEMETRY_DIR", &directory)
            .env("ONEBUDGETSPEC_BUDGET_ID", BOUNDARY_BUDGET)
            .env("ONEBUDGETSPEC_RESULT", &result)
            .output()
            .unwrap();
        (output, std::fs::read_to_string(&result).ok())
    };
    let valid = json!({"budget": BOUNDARY_BUDGET, "value": 3.0, "detail": "recorded",
        "workload": BOUNDARY_WORKLOAD});
    let with = |field: &str, value: Value| {
        let mut changed = valid.clone();
        changed[field] = value;
        Some(changed)
    };
    for (recorded, message) in [
        (None, "no telemetry"),
        (with("workload", json!("a task copy")), "is not of the"),
        (with("budget", json!("another-budget")), "different budget"),
        (with("value", json!(-1.0)), "missing nonnegative figure"),
        (with("detail", json!(null)), "missing detail"),
    ] {
        let (output, written) = run(recorded);
        assert!(!output.status.success(), "accepted {message}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(message),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(written, None, "reported a figure over {message}");
    }
    let (output, written) = run(Some(valid));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let written: Value = serde_json::from_str(&written.expect("a figure")).unwrap();
    assert_eq!(written["value"], json!(3.0), "{written}");
}
