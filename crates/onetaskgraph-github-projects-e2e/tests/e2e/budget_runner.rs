//! Reports ordinary-test telemetry, validating the workload before handing it to onebudgetspec.
use serde_json::Value;
use std::path::PathBuf;

pub(super) fn directory() -> PathBuf {
    if let Some(directory) = std::env::var_os("GITHUB_ASSET_TELEMETRY_DIR") {
        return directory.into();
    }
    let executable = std::env::current_exe().unwrap();
    let mut directory = executable.parent().unwrap().to_path_buf();
    if directory.file_name().is_some_and(|name| name == "deps") {
        directory.pop();
    }
    directory.join("telemetry/onetaskgraph-github-projects-e2e")
}
fn read(budget: &str) -> Result<(f64, String), String> {
    if !regex::Regex::new(onebudgetspec_core::model::ID_PATTERN)
        .unwrap()
        .is_match(budget)
    {
        return Err("invalid budget id".into());
    }
    let path = onetaskgraph_e2e_support::telemetry::file_in(&directory(), budget);
    let text = std::fs::read_to_string(&path).map_err(|error| {
        format!(
            "no telemetry at {}: {error}; run the domain test target",
            path.display()
        )
    })?;
    let value: Value = serde_json::from_str(&text).map_err(|error| error.to_string())?;
    validate(&value)?;
    let figure = value["value"]
        .as_f64()
        .filter(|value| value.is_finite() && *value >= 0.0)
        .ok_or("missing nonnegative figure")?;
    let detail = value["detail"].as_str().ok_or("missing detail")?.to_owned();
    Ok((figure, detail))
}
fn validate(value: &Value) -> Result<(), String> {
    let (counts, minimum, maximum, repetitions) = match value["workload"].as_str() {
        Some("new") => (vec![6, 2], 50_000, 500_000, 1),
        Some("unchanged") => (vec![6, 2], 50_000, 500_000, 3),
        Some("single") => (vec![24], 450_000, 500_000, 1),
        Some("concurrent") => (vec![24, 24, 24], 450_000, 500_000, 1),
        _ => return Err("missing or unknown workload".into()),
    };
    if value["repetitions"].as_u64() != Some(repetitions) {
        return Err("workload repetition count differs".into());
    }
    let copies = value["images"]
        .as_array()
        .ok_or("missing image telemetry")?;
    if copies.len() != counts.len() {
        return Err("workload record count differs".into());
    }
    for (copy, count) in copies.iter().zip(counts) {
        let sizes = copy.as_array().ok_or("image sizes are not an array")?;
        if sizes.len() != count {
            return Err("workload image count differs".into());
        }
        if sizes.iter().any(|size| {
            !size
                .as_u64()
                .is_some_and(|size| (minimum..=maximum).contains(&size))
        }) {
            return Err("workload image size differs".into());
        }
        if !sizes.iter().any(|size| {
            size.as_u64()
                .is_some_and(|size| (450_000..=500_000).contains(&size))
        }) {
            return Err("workload lacks a screenshot of about 500 KB".into());
        }
    }
    Ok(())
}
#[test]
fn report() {
    let budget = match std::env::var("ONEBUDGETSPEC_BUDGET_ID") {
        Ok(budget) => budget,
        Err(std::env::VarError::NotPresent) => return,
        Err(error) => refuse(&error.to_string()),
    };
    let (value, detail) = read(&budget).unwrap_or_else(|error| refuse(&error));
    match onebudgetspec_core::report(value, Some(&detail)) {
        Ok(true) => (),
        Ok(false) => refuse("ONEBUDGETSPEC_RESULT is missing"),
        Err(error) => refuse(&error.to_string()),
    }
}
fn refuse(message: &str) -> ! {
    eprintln!("{message}");
    std::process::exit(1)
}

#[test]
fn the_real_report_boundary_refuses_missing_short_or_small_workloads_without_a_figure() {
    use onetaskgraph_e2e_support::common::Sandbox;
    use serde_json::json;
    let sandbox = Sandbox::new();
    let directory = sandbox.subdirectory("telemetry");
    std::fs::create_dir_all(&directory).unwrap();
    let path = onetaskgraph_e2e_support::telemetry::file_in(&directory, "recorded-assets");
    for recorded in [
        None,
        Some(
            json!({"value":1,"detail":"ordinary test telemetry","workload":"single","repetitions":1,"images":[vec![480000;23]]}),
        ),
        Some(
            json!({"value":1,"detail":"ordinary test telemetry","workload":"single","repetitions":1,"images":[vec![100000;24]]}),
        ),
    ] {
        if let Some(recorded) = recorded {
            std::fs::write(&path, recorded.to_string()).unwrap();
        }
        let result = directory.join("result.json");
        let _ = std::fs::remove_file(&result);
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["budget_runner::report", "--exact", "--nocapture"])
            .env("GITHUB_ASSET_TELEMETRY_DIR", &directory)
            .env("ONEBUDGETSPEC_BUDGET_ID", "recorded-assets")
            .env("ONEBUDGETSPEC_RESULT", &result)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(!result.exists(), "reported over an invalid workload");
    }
    std::fs::write(&path,json!({"value":1,"detail":"ordinary-test telemetry","workload":"single","repetitions":1,"images":[vec![480000;24]]}).to_string()).unwrap();
    let result = directory.join("result.json");
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["budget_runner::report", "--exact", "--nocapture"])
        .env("GITHUB_ASSET_TELEMETRY_DIR", &directory)
        .env("ONEBUDGETSPEC_BUDGET_ID", "recorded-assets")
        .env("ONEBUDGETSPEC_RESULT", &result)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(result.is_file());
}

#[test]
fn nx_tracks_the_reporter_telemetry_directory() {
    let project: Value = serde_json::from_str(include_str!("../../project.json")).unwrap();
    let actual = directory();
    let suffix = "telemetry/onetaskgraph-github-projects-e2e";
    assert!(actual.ends_with(suffix));
    assert_eq!(
        project["targets"]["test"]["outputs"][0],
        format!("{{workspaceRoot}}/target/debug/{suffix}")
    );
    assert_eq!(
        project["targets"]["budgets"]["inputs"][2]["dependentTasksOutputFiles"],
        format!("**/{suffix}/*.json")
    );
}
