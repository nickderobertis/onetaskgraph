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
#[derive(Clone, Copy)]
pub(super) enum Workload {
    New,
    Unchanged,
    Single,
    Concurrent,
    ConcurrentRefusals,
}
impl Workload {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Unchanged => "unchanged",
            Self::Single => "single",
            Self::Concurrent | Self::ConcurrentRefusals => "concurrent",
        }
    }
    fn facts(self) -> (Vec<usize>, u64, u64, u64) {
        match self {
            Self::New => (vec![6, 2], 50_000, 500_000, 1),
            Self::Unchanged => (vec![6, 2], 50_000, 500_000, 3),
            Self::Single => (vec![24], 450_000, 500_000, 1),
            Self::Concurrent | Self::ConcurrentRefusals => (vec![24, 24, 24], 450_000, 500_000, 1),
        }
    }
}
/// The recording the ordinary test target wrote for `budget`, refused unless `budget` is a
/// budget id and the recording is that budget's own — the one reader every report shares.
fn recorded(budget: &str) -> Result<Value, String> {
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
    if value["budget"].as_str() != Some(budget) {
        return Err("telemetry belongs to a different budget".into());
    }
    Ok(value)
}

fn read(budget: &str, expected: Workload) -> Result<(f64, String), String> {
    let value = recorded(budget)?;
    validate(&value, expected)?;
    let figure = value["value"]
        .as_f64()
        .filter(|value| value.is_finite() && *value >= 0.0)
        .ok_or("missing nonnegative figure")?;
    if matches!(expected, Workload::ConcurrentRefusals)
        && (figure.fract() != 0.0 || figure > expected.facts().0.len() as f64)
    {
        return Err("refusal count must be a whole number within the recorded copy count".into());
    }
    let detail = value["detail"].as_str().ok_or("missing detail")?.to_owned();
    Ok((figure, detail))
}
fn validate(value: &Value, expected: Workload) -> Result<(), String> {
    let (counts, minimum, maximum, repetitions) = expected.facts();
    let workload = value["workload"].as_str().ok_or("missing workload")?;
    if workload != expected.label() {
        return Err("workload does not match expected shape".into());
    }
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
// Each registered command selects its workload by exact filter; telemetry cannot select it.
// llmlint: ignore-block[tests_assert_real_behavior] These five functions are report-only command entry points selected by budgets.yaml, using the landed test-harness runner convention. Ordinary tests must not report without ONEBUDGETSPEC_BUDGET_ID. The real_report_boundary journey below invokes this executable and asserts success, refusal, and whether a figure was written, including mismatched workloads; these entry points themselves are not behavioral tests.
#[test]
fn report_new() {
    report_expected(Workload::New);
}
#[test]
fn report_unchanged() {
    report_expected(Workload::Unchanged);
}
#[test]
fn report_single() {
    report_expected(Workload::Single);
}
#[test]
fn report_concurrent() {
    report_expected(Workload::Concurrent);
}
#[test]
fn report_concurrent_refusals() {
    report_expected(Workload::ConcurrentRefusals);
}
#[test]
fn report_visibility_reads() {
    let Some(budget) = selected() else {
        return;
    };
    let (value, detail) = read_visibility(&budget).unwrap_or_else(|error| refuse(&error));
    hand_over(value, &detail);
}
// llmlint: ignore-end[tests_assert_real_behavior]

/// The budget the report command was selected for, or `None` for an ordinary test run.
fn selected() -> Option<String> {
    match std::env::var("ONEBUDGETSPEC_BUDGET_ID") {
        Ok(budget) => Some(budget),
        Err(std::env::VarError::NotPresent) => None,
        Err(error) => refuse(&error.to_string()),
    }
}

/// What `visibility_cost` recorded for `budget`, refused unless it is that journey's recording
/// at that journey's workload: its figure, and the detail it reports beside it.
fn read_visibility(budget: &str) -> Result<(f64, String), String> {
    use super::visibility_cost::{WORKLOAD, WRITES};
    let value = recorded(budget)?;
    if value["workload"].as_str() != Some(WORKLOAD) || value["writes"].as_u64() != Some(WRITES) {
        return Err("workload does not match the 102-write plan copy".into());
    }
    let figure = value["value"]
        .as_u64()
        .ok_or("missing whole-number figure")?;
    // Every entry of a breakdown is a whole count: one that is not is no breakdown at all,
    // rather than an entry left out of the sum.
    let sum = |key: &str| {
        value[key]
            .as_object()
            .and_then(|counts| counts.values().map(Value::as_u64).sum::<Option<u64>>())
    };
    if sum("by_write") != Some(figure) || sum("by_read") != Some(figure) {
        return Err("the breakdown does not add up to the figure".into());
    }
    if !value["simulated_seconds"]
        .as_f64()
        .is_some_and(|seconds| seconds.is_finite() && seconds >= 0.0)
    {
        return Err("missing simulated seconds".into());
    }
    let detail = value["detail"].as_str().ok_or("missing detail")?.to_owned();
    let figure = u32::try_from(figure).map_err(|_| "figure out of range")?;
    Ok((f64::from(figure), detail))
}

fn report_expected(expected: Workload) {
    let Some(budget) = selected() else {
        return;
    };
    let (value, detail) = read(&budget, expected).unwrap_or_else(|error| refuse(&error));
    hand_over(value, &detail);
}

/// Hand one figure and its detail to onebudgetspec, failing the command when no figure was
/// written.
fn hand_over(value: f64, detail: &str) {
    // llmlint: ignore[budget_commands_measure_directly] An explicitly selected standalone report command must fail when report returns false (no destination), rather than claim success without a figure. This checks its exit-status contract only; onebudgetspec alone validates the result file and judges the figure against its threshold.
    match onebudgetspec_core::report(value, Some(detail)) {
        Ok(true) => (),
        Ok(false) => {
            refuse("no budget figure written: ONEBUDGETSPEC_RESULT must name a destination")
        }
        Err(error) => refuse(&error.to_string()),
    }
}
fn refuse(message: &str) -> ! {
    eprintln!("{message}");
    std::process::exit(1)
}

#[test]
fn the_real_report_boundary_refuses_invalid_telemetry_without_a_figure() {
    use onetaskgraph_e2e_support::common::Sandbox;
    use serde_json::json;
    let sandbox = Sandbox::new();
    let directory = sandbox.subdirectory("telemetry");
    std::fs::create_dir_all(&directory).unwrap();
    let path = onetaskgraph_e2e_support::telemetry::file_in(&directory, "recorded-assets");
    let valid = json!({"budget":"recorded-assets","value":1,"detail":"ordinary-test telemetry","workload":"single","repetitions":1,"images":[vec![480000;24]]});
    let mut cases = vec![
        (None, "recorded-assets", "no telemetry"),
        (Some("not json".into()), "recorded-assets", "expected"),
        (Some(valid.to_string()), "../escape", "invalid budget id"),
    ];
    for (field, value, message) in [
        ("budget", json!("other-budget"), "different budget"),
        ("value", json!(-1), "nonnegative figure"),
        ("value", json!("invalid"), "nonnegative figure"),
        ("detail", json!(null), "missing detail"),
        (
            "workload",
            json!("unknown"),
            "workload does not match expected shape",
        ),
        (
            "workload",
            json!("new"),
            "workload does not match expected shape",
        ),
        ("workload", json!(null), "missing workload"),
        ("repetitions", json!(3), "repetition count differs"),
        ("images", json!(null), "missing image telemetry"),
        ("images", json!([]), "record count differs"),
        ("images", json!([null]), "not an array"),
        ("images", json!([vec![480000; 23]]), "image count differs"),
        ("images", json!([vec![100000; 24]]), "image size differs"),
        ("images", json!([vec![500001; 24]]), "image size differs"),
    ] {
        let mut invalid = valid.clone();
        invalid[field] = value;
        cases.push((Some(invalid.to_string()), "recorded-assets", message));
    }
    let mut foreign_workload = valid.clone();
    foreign_workload["workload"] = json!("new");
    foreign_workload["images"] = json!([vec![480000; 6], vec![480000; 2]]);
    cases.push((
        Some(foreign_workload.to_string()),
        "recorded-assets",
        "workload does not match expected shape",
    ));
    let mut no_large = valid.clone();
    no_large["budget"] = json!("recorded-new-assets");
    no_large["workload"] = json!("new");
    no_large["images"] = json!([vec![100000; 6], vec![100000; 2]]);
    cases.push((
        Some(no_large.to_string()),
        "recorded-new-assets",
        "lacks a screenshot",
    ));
    for (recorded, budget, message) in cases {
        let _ = std::fs::remove_file(&path);
        if let Some(recorded) = recorded {
            std::fs::write(
                if budget == "../escape" {
                    path.clone()
                } else {
                    onetaskgraph_e2e_support::telemetry::file_in(&directory, budget)
                },
                recorded,
            )
            .unwrap();
        }
        let result = directory.join("result.json");
        let _ = std::fs::remove_file(&result);
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                if budget == "recorded-new-assets" {
                    "budget_runner::report_new"
                } else {
                    "budget_runner::report_single"
                },
                "--exact",
                "--nocapture",
            ])
            .env("GITHUB_ASSET_TELEMETRY_DIR", &directory)
            .env("ONEBUDGETSPEC_BUDGET_ID", budget)
            .env("ONEBUDGETSPEC_RESULT", &result)
            .output()
            .unwrap();
        assert!(
            !output.status.success(),
            "accepted invalid telemetry: {message}"
        );
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(message),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!result.exists(), "reported over an invalid workload");
    }
    std::fs::write(&path, valid.to_string()).unwrap();
    for destination in [None, Some("")] {
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args(["budget_runner::report_single", "--exact", "--nocapture"])
            .env("GITHUB_ASSET_TELEMETRY_DIR", &directory)
            .env("ONEBUDGETSPEC_BUDGET_ID", "recorded-assets")
            .env_remove("ONEBUDGETSPEC_RESULT");
        if let Some(destination) = destination {
            command.env("ONEBUDGETSPEC_RESULT", destination);
        }
        let output = command.output().unwrap();
        assert!(
            !output.status.success(),
            "accepted a missing result destination"
        );
        assert!(String::from_utf8_lossy(&output.stderr).contains("no budget figure written"));
    }
    let result = directory.join("result.json");
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["budget_runner::report_single", "--exact", "--nocapture"])
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

#[test]
fn refusal_report_rejects_fractional_and_impossible_counts() {
    use onetaskgraph_e2e_support::common::Sandbox;
    use serde_json::json;
    let sandbox = Sandbox::new();
    let directory = sandbox.subdirectory("counts");
    std::fs::create_dir_all(&directory).unwrap();
    for (figure, accepted) in [(0.5, false), (4.0, false), (3.0, true)] {
        let recorded = json!({"budget":"recorded-count","value":figure,"detail":"observed refusals","workload":"concurrent","repetitions":1,"images":[vec![480000;24],vec![480000;24],vec![480000;24]]});
        std::fs::write(
            onetaskgraph_e2e_support::telemetry::file_in(&directory, "recorded-count"),
            recorded.to_string(),
        )
        .unwrap();
        let result = directory.join("result.json");
        let _ = std::fs::remove_file(&result);
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "budget_runner::report_concurrent_refusals",
                "--exact",
                "--nocapture",
            ])
            .env("GITHUB_ASSET_TELEMETRY_DIR", &directory)
            .env("ONEBUDGETSPEC_BUDGET_ID", "recorded-count")
            .env("ONEBUDGETSPEC_RESULT", &result)
            .output()
            .unwrap();
        assert_eq!(
            output.status.success(),
            accepted,
            "figure {figure}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(result.exists(), accepted);
        if !accepted {
            assert!(String::from_utf8_lossy(&output.stderr).contains("refusal count"));
        }
    }
}

#[test]
fn the_visibility_report_refuses_a_missing_or_foreign_recording_without_a_figure() {
    use onetaskgraph_e2e_support::common::Sandbox;
    use serde_json::json;
    let sandbox = Sandbox::new();
    let directory = sandbox.subdirectory("visibility");
    let budget = super::visibility_cost::BUDGET;
    let valid = json!({"budget":budget,"value":204,"detail":"recorded","workload":super::visibility_cost::WORKLOAD,
        "writes":102,"by_write":{"project":2,"task":200,"document":2},
        "by_read":{"project":102,"repository":102},"simulated_seconds":38.76});
    let run = |recorded: Option<Value>| {
        let path = onetaskgraph_e2e_support::telemetry::file_in(&directory, budget);
        let _ = std::fs::remove_file(&path);
        if let Some(recorded) = recorded {
            std::fs::write(&path, recorded.to_string()).unwrap();
        }
        let result = directory.join("result.json");
        let _ = std::fs::remove_file(&result);
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "budget_runner::report_visibility_reads",
                "--exact",
                "--nocapture",
            ])
            .env("GITHUB_ASSET_TELEMETRY_DIR", &directory)
            .env("ONEBUDGETSPEC_BUDGET_ID", budget)
            .env("ONEBUDGETSPEC_RESULT", &result)
            .output()
            .unwrap();
        (output, result.exists())
    };
    let mut foreign = valid.clone();
    foreign["workload"] = json!("plan-copy-1-project-10-tasks-1-document");
    let mut fewer = valid.clone();
    fewer["writes"] = json!(12);
    let mut unbalanced = valid.clone();
    unbalanced["by_read"] = json!({"project":102,"repository":101});
    // An entry that is no count, beside entries that still add up to the figure on their own.
    let mut uncounted = valid.clone();
    uncounted["by_write"] = json!({"project":2,"task":200,"document":2,"note":"extra"});
    let with = |field: &str, value: Value| {
        let mut changed = valid.clone();
        changed[field] = value;
        Some(changed)
    };
    // A figure past what one budget can report, its breakdown made to add up so the range is
    // what refuses it.
    let mut huge = valid.clone();
    huge["value"] = json!(u64::from(u32::MAX) + 1);
    huge["by_write"] = json!({"task": u64::from(u32::MAX) + 1});
    huge["by_read"] = json!({"project": u64::from(u32::MAX) + 1});
    for (recorded, message) in [
        (None, "no telemetry"),
        (Some(foreign), "workload does not match"),
        (Some(fewer), "workload does not match"),
        (Some(unbalanced), "does not add up"),
        (Some(uncounted), "does not add up"),
        (with("value", json!(null)), "missing whole-number figure"),
        (with("value", json!(204.5)), "missing whole-number figure"),
        (
            with("simulated_seconds", json!(null)),
            "missing simulated seconds",
        ),
        (
            with("simulated_seconds", json!(-1.0)),
            "missing simulated seconds",
        ),
        (with("detail", json!(null)), "missing detail"),
        (Some(huge), "figure out of range"),
    ] {
        let (output, written) = run(recorded);
        assert!(!output.status.success(), "accepted: {message}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(message),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!written, "reported a figure over {message}");
    }
    let (output, written) = run(Some(valid));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(written, "a valid recording is reported");
}
