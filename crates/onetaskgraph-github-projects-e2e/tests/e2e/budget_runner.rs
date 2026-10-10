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
fn read(budget: &str, expected: Workload) -> Result<(f64, String), String> {
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
fn report_metadata_fields() {
    let budget = match std::env::var("ONEBUDGETSPEC_BUDGET_ID") {
        Ok(budget) => budget,
        Err(std::env::VarError::NotPresent) => return,
        Err(error) => refuse(&error.to_string()),
    };
    let (value, detail) = metadata_field_share(&budget).unwrap_or_else(|error| refuse(&error));
    // llmlint: ignore[budget_commands_measure_directly] As `report_expected`: a selected report command fails when no destination was written, and onebudgetspec alone judges the figure.
    match onebudgetspec_core::report(value, Some(&detail)) {
        Ok(true) => (),
        Ok(false) => {
            refuse("no budget figure written: ONEBUDGETSPEC_RESULT must name a destination")
        }
        Err(error) => refuse(&error.to_string()),
    }
}
// llmlint: ignore-end[tests_assert_real_behavior]

/// The workload `metadata_fields.rs` records: one follow-up copied once and re-copied twice
/// unchanged, with a projected host and without one.
pub(super) const METADATA_FIELD_WORKLOAD: &str = "metadata-field-follow-up";

/// The 10x hour of follow-up filing the budget states: first copies and unchanged re-copies
/// over the hour, and in its peak minute, when two runs' copies land together.
const BURST_HOUR: (u64, u64) = (50, 100);
const BURST_MINUTE: (u64, u64) = (10, 20);

/// One copy kind's spend as the telemetry records it.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Spent {
    operations: f64,
    requests: f64,
    points: f64,
}

fn spent(value: &Value, what: &str) -> Result<Spent, String> {
    let figure = |name: &str| {
        value[name]
            .as_u64()
            .map(|figure| figure as f64)
            .ok_or(format!("{what} is missing a whole {name} count"))
    };
    Ok(Spent {
        operations: figure("operations")?,
        requests: figure("requests")?,
        points: figure("points")?,
    })
}

/// The mean of the unchanged re-copies the telemetry records for one side.
fn unchanged(value: &Value, side: &str) -> Result<Spent, String> {
    let recopies = value["unchanged"][side]
        .as_array()
        .filter(|recopies| recopies.len() == 2)
        .ok_or(format!(
            "the workload records two unchanged re-copies {side} the projection"
        ))?;
    let each = recopies
        .iter()
        .map(|recopy| {
            spent(
                recopy,
                &format!("an unchanged re-copy {side} the projection"),
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mean = |pick: fn(&Spent) -> f64| each.iter().map(pick).sum::<f64>() / each.len() as f64;
    Ok(Spent {
        operations: mean(|spent| spent.operations),
        requests: mean(|spent| spent.requests),
        points: mean(|spent| spent.points),
    })
}

/// The larger of the two shares of GitHub's content-creation allowance projecting one field
/// adds to the stated burst, in percent, and the breakdown it is reported with.
pub(super) fn metadata_field_share(budget: &str) -> Result<(f64, String), String> {
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
    if value["workload"].as_str() != Some(METADATA_FIELD_WORKLOAD) {
        return Err("workload does not match expected shape".into());
    }
    let first_with = spent(&value["first"]["with"], "a first copy with the projection")?;
    let first_without = spent(
        &value["first"]["without"],
        "a first copy without the projection",
    )?;
    let unchanged_with = unchanged(&value, "with")?;
    let unchanged_without = unchanged(&value, "without")?;
    for (kind, with, without) in [
        ("first copy", first_with, first_without),
        ("unchanged re-copy", unchanged_with, unchanged_without),
    ] {
        if with.requests != without.requests {
            return Err(format!(
                "a {kind} sends {} HTTP requests with the projection and {} without it; the \
                 projection must ride a request the copy already sends",
                with.requests, without.requests
            ));
        }
        if with.operations < without.operations {
            return Err(format!(
                "a {kind} records fewer operations with the projection"
            ));
        }
    }
    let added_first = first_with.operations - first_without.operations;
    let added_unchanged = unchanged_with.operations - unchanged_without.operations;
    let added = |(first, recopies): (u64, u64)| {
        first as f64 * added_first + recopies as f64 * added_unchanged
    };
    let minute_writes = added(BURST_MINUTE);
    let hour_writes = added(BURST_HOUR);
    let per_minute = onetaskgraph_github_projects::CONTENT_CREATION_PER_MINUTE as f64;
    let per_hour = onetaskgraph_github_projects::CONTENT_CREATION_PER_HOUR as f64;
    let minute_share = 100.0 * minute_writes / per_minute;
    let hour_share = 100.0 * hour_writes / per_hour;
    let row = |kind: &str, with: Spent, without: Spent| {
        format!(
            "{kind}: {} operations, {} requests, {} points with metadata_fields; {} \
             operations, {} requests, {} points without",
            with.operations,
            with.requests,
            with.points,
            without.operations,
            without.requests,
            without.points
        )
    };
    // Points are the primary GraphQL allowance's unit, which `GET /rate_limit` reports per
    // token; what projecting adds to it over the burst hour is reported beside the shares.
    let hour_points = BURST_HOUR.0 as f64 * (first_with.points - first_without.points)
        + BURST_HOUR.1 as f64 * (unchanged_with.points - unchanged_without.points);
    let detail = format!(
        "peak minute: {minute_writes} added writes of {per_minute} a minute = {minute_share}%; \
         hour: {hour_writes} added writes of {per_hour} an hour = {hour_share}%; hour: \
         {hour_points} added modelled primary points against the token's hourly GraphQL points \
         allowance; burst {} first copies and {} unchanged re-copies an hour, {} and {} in its \
         peak minute; {}; {}",
        BURST_HOUR.0,
        BURST_HOUR.1,
        BURST_MINUTE.0,
        BURST_MINUTE.1,
        row("first copy", first_with, first_without),
        row("unchanged re-copy", unchanged_with, unchanged_without),
    );
    Ok((minute_share.max(hour_share), detail))
}

#[test]
fn the_metadata_field_share_is_the_larger_of_the_minute_and_the_hour() {
    use onetaskgraph_e2e_support::common::Sandbox;
    use serde_json::json;
    let sandbox = Sandbox::new();
    let directory = sandbox.subdirectory("metadata-telemetry");
    std::fs::create_dir_all(&directory).unwrap();
    let spent = |operations: u64, requests: u64| json!({"operations": operations, "requests": requests, "points": requests});
    let recorded = |first: (u64, u64), unchanged: (u64, u64), requests: (u64, u64)| {
        json!({"budget": "recorded-share", "detail": "observed",
               "workload": METADATA_FIELD_WORKLOAD,
               "first": {"with": spent(first.0, requests.0), "without": spent(first.1, 5)},
               "unchanged": {"with": [spent(unchanged.0, requests.1), spent(unchanged.0, requests.1)],
                             "without": [spent(unchanged.1, 1), spent(unchanged.1, 1)]}})
    };
    for (telemetry, expected) in [
        // One added write per first copy and none per re-copy: 10 / 80 against 50 / 500.
        (recorded((3, 2), (0, 0), (5, 1)), Ok(12.5)),
        // One per re-copy too: 30 / 80 against 150 / 500.
        (recorded((3, 2), (1, 0), (5, 1)), Ok(37.5)),
        (recorded((3, 2), (0, 0), (6, 1)), Err("must ride a request")),
        (recorded((3, 2), (0, 0), (5, 2)), Err("must ride a request")),
        (recorded((1, 2), (0, 0), (5, 1)), Err("fewer operations")),
    ] {
        std::fs::write(
            onetaskgraph_e2e_support::telemetry::file_in(&directory, "recorded-share"),
            telemetry.to_string(),
        )
        .unwrap();
        let result = directory.join("result.json");
        let _ = std::fs::remove_file(&result);
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "budget_runner::report_metadata_fields",
                "--exact",
                "--nocapture",
            ])
            .env("GITHUB_ASSET_TELEMETRY_DIR", &directory)
            .env("ONEBUDGETSPEC_BUDGET_ID", "recorded-share")
            .env("ONEBUDGETSPEC_RESULT", &result)
            .output()
            .unwrap();
        let said = String::from_utf8_lossy(&output.stderr);
        match expected {
            Ok(share) => {
                assert!(output.status.success(), "{said}");
                let reported: Value =
                    serde_json::from_str(&std::fs::read_to_string(&result).unwrap()).unwrap();
                assert_eq!(reported["value"].as_f64(), Some(share), "{reported}");
                let detail = reported["detail"].as_str().unwrap_or_default();
                for named in [
                    "peak minute",
                    "of 80 a minute",
                    "of 500 an hour",
                    "first copy",
                ] {
                    assert!(detail.contains(named), "{named}: {detail}");
                }
            }
            Err(message) => {
                assert!(!output.status.success(), "accepted: {message}");
                assert!(said.contains(message), "{message}: {said}");
                assert!(!result.exists());
            }
        }
    }
}

fn report_expected(expected: Workload) {
    let budget = match std::env::var("ONEBUDGETSPEC_BUDGET_ID") {
        Ok(budget) => budget,
        Err(std::env::VarError::NotPresent) => return,
        Err(error) => refuse(&error.to_string()),
    };
    let (value, detail) = read(&budget, expected).unwrap_or_else(|error| refuse(&error));
    // llmlint: ignore[budget_commands_measure_directly] An explicitly selected standalone report command must fail when report returns false (no destination), rather than claim success without a figure. This checks its exit-status contract only; onebudgetspec alone validates the result file and judges the figure against its threshold.
    match onebudgetspec_core::report(value, Some(&detail)) {
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
