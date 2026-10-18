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
// llmlint: ignore-block[budget_commands_measure_directly] Every budget of this crate selects its own report-only entry here, the harness-runner convention it landed with (report_visibility_reads, report_metadata_fields), and the task registering this one names that pattern. It reads only the telemetry the `status_complete` journey recorded, refuses a recording of another workload, and hands the figure to onebudgetspec, which alone judges it.
#[test]
fn report_landing_closure() {
    let Some(budget) = selected() else {
        return;
    };
    let (value, detail) = read_landing_closure(&budget).unwrap_or_else(|error| refuse(&error));
    hand_over(value, &detail);
}
// llmlint: ignore-end[budget_commands_measure_directly]
// llmlint: ignore-block[budget_commands_measure_directly] Every budget of this crate selects its own report-only entry here, the harness-runner convention it landed with (report_visibility_reads, report_metadata_fields), and the task registering this one names that pattern. It reads only the telemetry the journeys recorded, refuses a recording of another workload, and hands the figure to onebudgetspec, which alone judges it.
#[test]
fn report_cache_points() {
    let Some(budget) = selected() else {
        return;
    };
    let (value, detail) = read_cache_points(&budget).unwrap_or_else(|error| refuse(&error));
    hand_over(value, &detail);
}
// llmlint: ignore-end[budget_commands_measure_directly]
#[test]
fn report_metadata_fields() {
    let Some(budget) = selected() else {
        return;
    };
    let (value, detail) = metadata_field_share(&budget).unwrap_or_else(|error| refuse(&error));
    hand_over(value, &detail);
}
// llmlint: ignore-end[tests_assert_real_behavior]

/// The workload `metadata_fields.rs` records: one follow-up copied once and re-copied twice
/// unchanged, with a projected host and without one.
pub(super) const METADATA_FIELD_WORKLOAD: &str = "metadata-field-follow-up";

/// The 10x hour of follow-up filing the budget states: first copies and unchanged re-copies
/// over the hour, and in its peak minute, when two runs' copies land together.
const BURST_HOUR: (u64, u64) = (50, 100);
const BURST_MINUTE: (u64, u64) = (10, 20);

/// A well-formed recording broken one way at a time, each with the refusal it must meet:
/// another workload, a count that is missing or not a whole number, and re-copies that are
/// absent, not a list, or not the two the workload makes.
fn malformed_metadata_recordings(valid: Value) -> Vec<(Value, Result<f64, &'static str>)> {
    use serde_json::json;
    let with = |pointer: &str, value: Value| {
        let mut changed = valid.clone();
        *changed
            .pointer_mut(pointer)
            .expect("the recording has that member") = value;
        changed
    };
    vec![
        (
            with("/workload", json!("another-workload")),
            Err("workload does not match"),
        ),
        (
            with("/first/with/operations", Value::Null),
            Err("a first copy with the projection is missing a whole operations count"),
        ),
        (
            with("/first/without/requests", json!(5.5)),
            Err("a first copy without the projection is missing a whole requests count"),
        ),
        (
            with("/unchanged/with/0/points", json!("one")),
            Err("an unchanged re-copy with the projection is missing a whole points count"),
        ),
        (
            with("/unchanged/without", Value::Null),
            Err("two unchanged re-copies without the projection"),
        ),
        (
            with("/unchanged/with", json!({"operations": 0})),
            Err("two unchanged re-copies with the projection"),
        ),
        (
            with(
                "/unchanged/with",
                json!([{"operations": 0, "requests": 1, "points": 1}]),
            ),
            Err("two unchanged re-copies with the projection"),
        ),
    ]
}

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
    let value = recorded(budget)?;
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
    ]
    .into_iter()
    .chain(malformed_metadata_recordings(recorded(
        (3, 2),
        (0, 0),
        (5, 1),
    ))) {
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

/// What `status_complete` recorded for `budget`, refused unless it is that journey's recording
/// at its workload with both scales measured, each source's reads and writes adding up to that
/// scale's points, and the figure the 10x scale's.
fn read_landing_closure(budget: &str) -> Result<(f64, String), String> {
    landing_closure_of(&recorded(budget)?)
}

/// [`read_landing_closure`]'s judgement of one recording, `value`.
fn landing_closure_of(value: &Value) -> Result<(f64, String), String> {
    use super::status_complete::{SCALES, WORKLOAD};
    if value["workload"].as_str() != Some(WORKLOAD) {
        return Err("workload does not match one landing's closures over two boards".into());
    }
    let figure = value["value"]
        .as_u64()
        .ok_or("missing whole-number figure")?;
    for (scale, issues, board, second) in SCALES {
        let recorded = &value["scales"][scale];
        if recorded["issues"].as_u64() != Some(issues as u64)
            || recorded["board"].as_u64() != Some(board as u64)
            || recorded["second_board"].as_u64() != Some(second as u64)
        {
            return Err(format!(
                "the {scale} scale is not the workload the budget states"
            ));
        }
        let points = recorded["points"]
            .as_u64()
            .ok_or_else(|| format!("the {scale} scale carries no points"))?;
        let sources = recorded["sources"]
            .as_object()
            .filter(|sources| sources.len() == 2)
            .ok_or_else(|| format!("the {scale} scale does not break down two sources"))?;
        let mut sum = 0;
        for spent in sources.values() {
            for key in ["reads", "read_points", "writes", "write_points"] {
                spent[key]
                    .as_u64()
                    .ok_or_else(|| format!("the {scale} scale's {key} is not a count"))?;
            }
            sum += spent["read_points"].as_u64().unwrap_or(0)
                + spent["write_points"].as_u64().unwrap_or(0);
        }
        if sum != points {
            return Err(format!(
                "the {scale} breakdown does not add up to its points"
            ));
        }
        if scale == "10x" && points != figure {
            return Err("the figure is not the 10x scale's".into());
        }
    }
    let detail = value["detail"].as_str().ok_or("missing detail")?.to_owned();
    let figure = u32::try_from(figure).map_err(|_| "figure out of range")?;
    Ok((f64::from(figure), detail))
}

/// A recording `landing_closure_of` accepts: both scales at the workload the budget states, each
/// source spending one read point, the 10x scale's figure the recording's.
fn landing_closure_recording() -> Value {
    use super::status_complete::{SCALES, WORKLOAD};
    let mut scales = serde_json::Map::new();
    for (scale, issues, board, second) in SCALES {
        let spent = serde_json::json!({"reads":1,"read_points":1,"writes":0,"write_points":0});
        scales.insert(
            scale.to_owned(),
            serde_json::json!({"issues":issues,"board":board,"second_board":second,"points":2,
                               "sources":{"plans":spent,"followups":spent}}),
        );
    }
    serde_json::json!({"workload":WORKLOAD,"value":2,"scales":scales,"detail":"2 points"})
}

#[test]
fn a_landing_closure_recording_is_refused_unless_it_is_that_journeys_whole_measurement() {
    assert_eq!(
        landing_closure_of(&landing_closure_recording()),
        Ok((2.0, "2 points".to_owned()))
    );
    let bent = |bend: &dyn Fn(&mut Value)| {
        let mut recording = landing_closure_recording();
        bend(&mut recording);
        landing_closure_of(&recording).expect_err("a bent recording is refused")
    };
    for (bend, says) in [
        (
            Box::new(|value: &mut Value| value["workload"] = "another".into())
                as Box<dyn Fn(&mut Value)>,
            "workload does not match",
        ),
        (
            Box::new(|value: &mut Value| value["scales"]["1x"]["issues"] = 2.into()),
            "the 1x scale is not the workload the budget states",
        ),
        (
            Box::new(|value: &mut Value| {
                value["scales"]["10x"]["sources"]
                    .as_object_mut()
                    .unwrap()
                    .remove("plans");
            }),
            "the 10x scale does not break down two sources",
        ),
        (
            Box::new(|value: &mut Value| {
                value["scales"]["1x"]["sources"]["plans"]["write_points"] = "1".into();
            }),
            "the 1x scale's write_points is not a count",
        ),
        (
            Box::new(|value: &mut Value| value["scales"]["10x"]["points"] = 3.into()),
            "the 10x breakdown does not add up to its points",
        ),
        (
            Box::new(|value: &mut Value| value["value"] = 1.into()),
            "the figure is not the 10x scale's",
        ),
        (
            Box::new(|value: &mut Value| value["value"] = "2".into()),
            "missing whole-number figure",
        ),
        (
            Box::new(|value: &mut Value| {
                value.as_object_mut().unwrap().remove("detail");
            }),
            "missing detail",
        ),
    ] {
        let refused = bent(&*bend);
        assert!(refused.contains(says), "{refused} does not say {says}");
    }
}

/// What `cache_budget` recorded for `budget`, refused unless it is that journey's recording at
/// its workload with every sequence measured in both modes, and its figure the largest
/// difference among them.
fn read_cache_points(budget: &str) -> Result<(f64, String), String> {
    use super::cache_budget::{KINDS, SCALES, WORKLOAD, sequences_of};
    let value = recorded(budget)?;
    if value["workload"].as_str() != Some(WORKLOAD) {
        return Err("workload does not match the board workload at 1x and 10x".into());
    }
    let figure = value["value"]
        .as_i64()
        .ok_or("missing whole-number figure")?;
    // A point count is a whole number of points spent, never negative, and small enough that
    // the difference of two cannot overflow.
    let difference = |measured: &Value, what: &str| -> Result<i64, String> {
        let points = |mode: &str| {
            measured[mode]["points"]
                .as_u64()
                .and_then(|points| i64::try_from(points).ok())
        };
        let (Some(on), Some(off)) = (points("on"), points("off")) else {
            return Err(format!(
                "{what} is not measured in both modes as whole point counts"
            ));
        };
        on.checked_sub(off)
            .ok_or_else(|| format!("{what}'s point counts are out of range"))
    };
    let mut largest = i64::MIN;
    for (scale, _) in SCALES {
        let at = &value["scales"][scale];
        for kind in KINDS {
            for sequence in sequences_of(kind) {
                largest = largest.max(difference(
                    &at["kinds"][kind][sequence],
                    &format!("{scale} {kind} {sequence}"),
                )?);
            }
        }
        largest = largest.max(difference(&at["total"], &format!("the {scale} total"))?);
    }
    if largest != figure {
        return Err("the figure is not the largest difference the recording holds".into());
    }
    let detail = value["detail"].as_str().ok_or("missing detail")?.to_owned();
    let figure = i32::try_from(figure).map_err(|_| "the figure is out of range")?;
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

#[test]
fn the_cache_report_takes_the_whole_matrix_and_refuses_any_cell_or_figure_it_cannot_vouch_for() {
    use super::cache_budget::{BUDGET, KINDS, SCALES, WORKLOAD, sequences_of};
    use onetaskgraph_e2e_support::common::Sandbox;
    use serde_json::json;
    let sandbox = Sandbox::new();
    let directory = sandbox.subdirectory("cache-points");
    // Every cell spends 10 points off and 7 on, so the largest difference is a 3-point saving
    // — except each scale's total, which sums them and saves more.
    let cell = |on: u64, off: u64| json!({"on": {"points": on}, "off": {"points": off}});
    let mut scales = serde_json::Map::new();
    for (scale, _) in SCALES {
        let mut kinds = serde_json::Map::new();
        for kind in KINDS {
            let cells: serde_json::Map<String, Value> = sequences_of(kind)
                .iter()
                .map(|sequence| ((*sequence).to_owned(), cell(7, 10)))
                .collect();
            kinds.insert(kind.to_owned(), Value::Object(cells));
        }
        scales.insert(
            scale.to_owned(),
            json!({"kinds": kinds, "total": cell(140, 200)}),
        );
    }
    let valid = json!({"budget": BUDGET, "value": -3, "detail": "recorded", "workload": WORKLOAD,
                       "scales": scales});
    let run = |recorded: Option<Value>| {
        let path = onetaskgraph_e2e_support::telemetry::file_in(&directory, BUDGET);
        let _ = std::fs::remove_file(&path);
        if let Some(recorded) = recorded {
            std::fs::write(&path, recorded.to_string()).unwrap();
        }
        let result = directory.join("result.json");
        let _ = std::fs::remove_file(&result);
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "budget_runner::report_cache_points",
                "--exact",
                "--nocapture",
            ])
            .env("GITHUB_ASSET_TELEMETRY_DIR", &directory)
            .env("ONEBUDGETSPEC_BUDGET_ID", BUDGET)
            .env("ONEBUDGETSPEC_RESULT", &result)
            .output()
            .unwrap();
        (output, std::fs::read_to_string(&result).ok())
    };
    let with = |pointer: &str, value: Value| {
        let mut changed = valid.clone();
        *changed.pointer_mut(pointer).expect("the member") = value;
        Some(changed)
    };
    let without = |pointer: &str, member: &str| {
        let mut changed = valid.clone();
        changed
            .pointer_mut(pointer)
            .and_then(Value::as_object_mut)
            .expect("the object")
            .remove(member);
        Some(changed)
    };
    // A cell spending past what one budget can report, its figure made the largest so the range
    // is what refuses it.
    let mut huge = valid.clone();
    huge["scales"]["10x"]["kinds"]["tasks"]["cold"] = cell(u64::from(u32::MAX) + 10, 10);
    huge["value"] = json!(u64::from(u32::MAX));
    for (recorded, message) in [
        (None, "no telemetry"),
        (
            with("/budget", json!("github-copy-recopy-unchanged")),
            "a different budget",
        ),
        (
            with("/workload", json!("board-1-task")),
            "workload does not match",
        ),
        (
            without("/scales/1x/kinds/asset-bytes", "render-after-render"),
            "1x asset-bytes render-after-render is not measured in both modes",
        ),
        (
            without("/scales/10x", "total"),
            "the 10x total is not measured in both modes",
        ),
        (
            with("/scales/1x/kinds/documents/cold/on/points", json!(-1)),
            "1x documents cold is not measured in both modes",
        ),
        (
            with("/scales/1x/kinds/documents/cold/off/points", json!(2.5)),
            "1x documents cold is not measured in both modes",
        ),
        (with("/value", json!(0)), "not the largest difference"),
        (with("/value", json!(null)), "missing whole-number figure"),
        (with("/value", json!(-3.5)), "missing whole-number figure"),
        (with("/detail", json!(null)), "missing detail"),
        (Some(huge), "the figure is out of range"),
    ] {
        let (output, written) = run(recorded);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "accepted: {message}");
        assert!(stderr.contains(message), "{message}: {stderr}");
        assert_eq!(
            written.as_deref().unwrap_or_default(),
            "",
            "reported a figure over {message}"
        );
    }
    let (output, written) = run(Some(valid));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_str(&written.expect("a result")).expect("JSON");
    assert_eq!(
        result["value"],
        json!(-3.0),
        "a saving is the negative figure it is"
    );
}

/// Report the current complete live pair without making any GitHub request.
// llmlint: ignore[tests_assert_real_behavior] This report-only command is selected by budgets.yaml; live_session_report_boundary invokes this executable and checks its result and refusal paths.
#[test]
fn report_live_session() {
    let Some(budget) = selected() else {
        return;
    };
    // llmlint: ignore-block[budget_commands_measure_directly] The task requires this named report-only entry point; checking its SDK selector prevents a foreign budget id from receiving a live-session figure. The function measures only saved telemetry, with no threshold comparison.
    if budget != "github-live-session-points" {
        refuse("live-session reporter selected for a different budget");
    }
    // llmlint: ignore-end[budget_commands_measure_directly]
    let (value, detail) = live_session_figure()
        .unwrap_or_else(|error| refuse(&format!("live-session measurement unavailable: {error}")));
    hand_over(value, &detail);
}

fn live_session_figure() -> Result<(f64, String), String> {
    use onetaskgraph_github_live::quota::{
        CurrentSession, PRICING_URL, WORKSPACE_VERSION, read_bill, reference_workload,
    };
    let directory =
        std::env::var_os("GITHUB_LIVE_SESSION_DIR").ok_or("no current session directory")?;
    let id = std::env::var("GITHUB_LIVE_SESSION_ID").map_err(|error| error.to_string())?;
    let revision =
        std::env::var("GITHUB_LIVE_SOURCE_REVISION").map_err(|error| error.to_string())?;
    let workload = reference_workload();
    let current = CurrentSession::new(&id, &revision, WORKSPACE_VERSION, &workload)?;
    let bill = read_bill(std::path::Path::new(&directory), &current)?;
    // The SDK accepts f64. Refuse rather than round an integer outside its exact range.
    let figure =
        u32::try_from(bill.points).map_err(|_| "session figure out of exact reporting range")?;
    let phases = bill
        .phases
        .iter()
        .map(|(phase, points)| format!("{phase:?}={points}"))
        .collect::<Vec<_>>()
        .join(", ");
    // A smaller shared board bills less; say so rather than let its figure read as the
    // registered workload's measurement.
    let board = if bill.initial_items == workload.initial_items {
        format!(
            "initial board items={} (the registered workload)",
            bill.initial_items
        )
    } else {
        format!(
            "initial board items={} (fewer than the registered workload's {}; not its measurement)",
            bill.initial_items, workload.initial_items
        )
    };
    let detail = format!(
        "session_id={id}; source_revision={revision}; workspace_version={WORKSPACE_VERSION}; {board}; reported query points={}; Modelled mutation points={} ({} requests); Documented mutation points={} ({} requests); mutation count={}; pricing URL={PRICING_URL}; GraphQL requests={}; REST requests={} (excluded from points); phases: {phases}; corroborating account movement={:?}; concurrent movement flagged={}; actual asset bytes={:?}; requested PNG bytes={:?}; Modelled deliberate page-size refusal points={} ({} requests)",
        bill.query_points,
        bill.modelled_mutation_points,
        bill.modelled_mutations,
        bill.documented_mutation_points,
        bill.documented_mutations,
        bill.mutations,
        bill.graphql_requests,
        bill.rest_requests,
        bill.account_movement,
        bill.movement_flagged,
        workload.asset_bytes,
        workload.asset_target_bytes,
        bill.modelled_page_refusal_points,
        bill.modelled_page_refusals
    );
    Ok((f64::from(figure), detail))
}

fn live_report_pair(points: u64) -> Vec<onetaskgraph_github_live::quota::SessionRecord> {
    use onetaskgraph_github_live::quota::{
        CallRecord, Charge, EntryPoint, MutationBasis, Outcome, PRICING_URL, Phase, RequestKind,
        SessionRecord, WORKSPACE_VERSION, operation_sha256, reference_workload,
    };
    let phases = [
        (
            EntryPoint::Main,
            vec![
                Phase::Admission,
                Phase::Schema,
                Phase::Reconciliation,
                Phase::Setup,
                Phase::Capabilities,
                Phase::CommentActivity,
                Phase::ChangeToken,
                Phase::Cleanup,
                Phase::OrphanRecovery,
            ],
        ),
        (
            EntryPoint::Assets,
            vec![
                Phase::Admission,
                Phase::AssetSetup,
                Phase::AssetTask,
                Phase::AssetDocument,
                Phase::Cleanup,
            ],
        ),
    ];
    let mut pair: Vec<_> = phases
        .into_iter()
        .map(|(invocation, phases)| SessionRecord {
            schema_version: onetaskgraph_github_live::quota::SCHEMA_VERSION,
            session_id: "current-test-session".into(),
            source_revision: "a".repeat(40),
            workspace_version: WORKSPACE_VERSION.into(),
            invocation,
            workload: reference_workload(),
            complete: true,
            calls: phases
                .into_iter()
                .enumerate()
                .map(|(index, phase)| CallRecord {
                    sequence: u64::try_from(index + 1).unwrap(),
                    phase,
                    operation_sha256: operation_sha256("query{rateLimit{cost}}"),
                    attempt: 1,
                    kind: RequestKind::Query,
                    outcome: Outcome::Success,
                    charge: Charge::ReportedQuery { points: 1 },
                    allowance: None,
                })
                .collect(),
        })
        .collect();
    pair[0].calls[0].charge = Charge::ReportedQuery {
        points: points - 13,
    };
    pair[0].calls[3].kind = RequestKind::Mutation;
    pair[0].calls[3].charge = Charge::Mutation {
        points: 1,
        basis: MutationBasis::Modelled {
            url: PRICING_URL.into(),
            points_per_request: 1,
        },
    };
    pair[1].calls[3].kind = RequestKind::Mutation;
    pair[1].calls[3].charge = Charge::Mutation {
        points: 1,
        basis: MutationBasis::Documented {
            url: PRICING_URL.into(),
        },
    };
    let mut rest = pair[1].calls[4].clone();
    rest.sequence = 6;
    rest.kind = RequestKind::Rest;
    rest.charge = Charge::Rest;
    pair[1].calls.push(rest);
    pair
}

#[test]
fn live_session_report_boundary() {
    use onetaskgraph_e2e_support::common::Sandbox;
    use onetaskgraph_github_live::quota::{Charge, Phase};
    let sandbox = Sandbox::new();
    let directory = sandbox.subdirectory("live-session");
    std::fs::create_dir_all(&directory).unwrap();
    let result = directory.join("sdk-result");
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
    assert!(budget.labels.iter().any(|label| label == "github"));
    let boundary = budget.threshold.to_string().parse::<u64>().unwrap();
    let good = live_report_pair(boundary);
    let mut cases = vec![(good.clone(), None), (live_report_pair(boundary + 1), None)];
    for change in [
        |pair: &mut Vec<onetaskgraph_github_live::quota::SessionRecord>| {
            pair.pop();
        },
        |pair: &mut Vec<onetaskgraph_github_live::quota::SessionRecord>| pair[1].complete = false,
        |pair: &mut Vec<onetaskgraph_github_live::quota::SessionRecord>| {
            pair[1].session_id.push('x')
        },
        |pair: &mut Vec<onetaskgraph_github_live::quota::SessionRecord>| {
            pair[1].source_revision = "b".repeat(40)
        },
        |pair: &mut Vec<onetaskgraph_github_live::quota::SessionRecord>| {
            pair[1].workspace_version.push('x')
        },
        |pair: &mut Vec<onetaskgraph_github_live::quota::SessionRecord>| {
            pair[1].workload.asset_bytes.pop().map(|_| ()).unwrap()
        },
        |pair: &mut Vec<onetaskgraph_github_live::quota::SessionRecord>| {
            pair[1].calls.retain(|call| call.phase != Phase::Cleanup)
        },
        |pair: &mut Vec<onetaskgraph_github_live::quota::SessionRecord>| {
            pair[0].calls[0].charge = Charge::Unavailable {
                reason: "missing actual price".into(),
            }
        },
        |pair: &mut Vec<onetaskgraph_github_live::quota::SessionRecord>| pair[1] = pair[0].clone(),
    ] {
        let mut pair = good.clone();
        change(&mut pair);
        cases.push((pair, Some("live-session measurement unavailable")));
    }
    for (pair, failure) in cases {
        for file in ["main.json", "assets.json", "sdk-result"] {
            let _ = std::fs::remove_file(directory.join(file));
        }
        for (index, record) in pair.iter().enumerate() {
            std::fs::write(
                directory.join(if index == 0 {
                    "main.json"
                } else {
                    "assets.json"
                }),
                serde_json::to_vec(record).unwrap(),
            )
            .unwrap();
        }
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "budget_runner::report_live_session",
                "--exact",
                "--nocapture",
            ])
            .env("GITHUB_LIVE_SESSION_DIR", &directory)
            .env("GITHUB_LIVE_SESSION_ID", "current-test-session")
            .env("GITHUB_LIVE_SOURCE_REVISION", "a".repeat(40))
            .env("ONEBUDGETSPEC_BUDGET_ID", "github-live-session-points")
            .env("ONEBUDGETSPEC_RESULT", &result)
            .output()
            .unwrap();
        let said = String::from_utf8_lossy(&output.stderr);
        if let Some(message) = failure {
            assert!(!output.status.success(), "{said}");
            assert!(said.contains(message), "{said}");
            assert!(!result.exists());
        } else {
            assert!(output.status.success(), "{said}");
            let reported: Value = serde_json::from_slice(&std::fs::read(&result).unwrap()).unwrap();
            assert_eq!(
                reported["value"].as_f64(),
                Some(match pair[0].calls[0].charge {
                    Charge::ReportedQuery { points } =>
                        f64::from(u32::try_from(points + 13).unwrap()),
                    _ => unreachable!(),
                })
            );
            let verdict = onebudgetspec_core::judge(
                budget.direction,
                budget.threshold,
                reported["value"].as_f64().unwrap(),
            )
            .0;
            let expected = if matches!(pair[0].calls[0].charge, Charge::ReportedQuery { points } if points + 13 == boundary)
            {
                onebudgetspec_core::report::Verdict::Within
            } else {
                onebudgetspec_core::report::Verdict::Over
            };
            assert_eq!(verdict, expected);
            let detail = reported["detail"].as_str().unwrap();
            for fact in [
                "Modelled mutation points=1 (1 requests)",
                "Documented mutation points=1 (1 requests)",
                "mutation count=2",
                "REST requests=1",
                "AssetDocument=1",
                "Cleanup=2",
                "workspace_version=",
                "source_revision=",
                "session_id=current-test-session",
                "initial board items=90 (the registered workload)",
            ] {
                assert!(detail.contains(fact), "{fact}: {detail}");
            }
        }
    }
}
