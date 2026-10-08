//! Where the Linear request budgets' telemetry lives: one JSON file per budget,
//! `{"value": <requests>, "detail": "<how they were counted>"}`, named after the budget's id.
//!
//! The `measure_*` journeys write it with [`record`] while the `e2e` target runs, and
//! `budget_runner::report` reads it back with [`recorded`] for `onebudgetspec check`. The
//! directory is `<target>/<profile>/telemetry/onetaskgraph-linear-e2e`, found from the running
//! test executable the way `onetaskgraph_e2e_support::binary` finds the binary, so the writer and
//! the reader agree on it whatever the target directory is. It is never committed, and it is the
//! `test` target's declared output, so a cache hit of that target restores what its last run
//! recorded. [`DIRECTORY`] names another, so a test can drive the runner over telemetry of its
//! own without touching what the journeys recorded.

use std::path::{Path, PathBuf};

use serde_json::json;

/// The variable that, when set, names the telemetry directory in place of the build's.
pub const DIRECTORY: &str = "LINEAR_BUDGET_TELEMETRY_DIR";

/// The directory's path below the build directory: what the `test` target declares as its
/// output and the `budgets` target hashes.
pub const BELOW_BUILD: &str = "telemetry/onetaskgraph-linear-e2e";

/// The directory the telemetry is in: the one [`DIRECTORY`] names, or the build's own.
pub fn directory() -> PathBuf {
    if let Some(directory) = std::env::var_os(DIRECTORY) {
        return PathBuf::from(directory);
    }
    let executable = std::env::current_exe().expect("the running test executable has a path");
    let mut directory = executable
        .parent()
        .expect("a test executable sits inside a build directory")
        .to_path_buf();
    if directory.file_name().is_some_and(|leaf| leaf == "deps") {
        directory.pop();
    }
    directory.join(Path::new(BELOW_BUILD))
}

/// One spelling of the file name for the recorder and the reader alike, so a runner test that
/// plants a file through it is read exactly where a journey's figure would be.
pub fn file_in(directory: &Path, budget: &str) -> PathBuf {
    directory.join(format!("{budget}.json"))
}

/// Record `value` as `budget`'s telemetry: the figure and how it was reached, and no judgement
/// of it.
pub fn record(budget: &str, value: usize, detail: &str) {
    record_in(&directory(), budget, value, detail);
}

/// Takes its directory so the runner's tests record into a scratch tree of their own rather than
/// over the figures the journeys left in the build directory.
pub fn record_in(directory: &Path, budget: &str, value: usize, detail: &str) {
    std::fs::create_dir_all(directory).expect("the telemetry directory is writable");
    std::fs::write(
        file_in(directory, budget),
        json!({"value": value, "detail": detail}).to_string(),
    )
    .expect("the telemetry file is writable");
}

/// Record an asset workload beside its observed figure; the analyser validates its weight.
pub fn record_assets(budget: &str, value: f64, sizes: &[usize], copies: usize) {
    let directory = directory();
    std::fs::create_dir_all(&directory).expect("the telemetry directory is writable");
    std::fs::write(
        file_in(&directory, budget),
        json!({
            "value": value, "detail": "counted at the Linear loopback in simulated time",
            "workload": {"sizes": sizes, "copies": copies}
        })
        .to_string(),
    )
    .expect("the telemetry file is writable");
}

/// The figure and the detail `budget`'s journey recorded, or why there is none to read.
///
/// `budget` comes from the environment, so it is held to onebudgetspec's own grammar for an id
/// before it names a file: nothing else can reach outside the telemetry directory.
pub fn recorded(budget: &str) -> Result<(f64, Option<String>), String> {
    let id = regex::Regex::new(onebudgetspec_core::model::ID_PATTERN)
        .expect("onebudgetspec's id pattern compiles");
    if !id.is_match(budget) {
        return Err(format!(
            "{budget:?} is not a budget id ({}); name one of budgets.yaml's",
            onebudgetspec_core::model::ID_PATTERN
        ));
    }
    let path = file_in(&directory(), budget);
    let text = std::fs::read_to_string(&path).map_err(|error| {
        format!(
            "{budget}: no telemetry at {} ({error}); the journey that measures it records it \
             while the onetaskgraph-linear-e2e `test` target runs, so run \
             `scripts/nx.sh run onetaskgraph-linear-e2e:test` and re-run, or restore that \
             journey's `record(\"{budget}\", …)` in tests/e2e/linear_budget.rs",
            path.display()
        )
    })?;
    let recorded: serde_json::Value = serde_json::from_str(&text)
        .map_err(|error| format!("{budget}: {} is not JSON ({error})", path.display()))?;
    let value = if budget.starts_with("linear-asset-") || budget.starts_with("linear-concurrent-") {
        let concurrent = budget.ends_with("copies-refused");
        let large = concurrent || budget.ends_with("copy-seconds");
        let expected = if concurrent {
            72
        } else if large {
            24
        } else {
            8
        };
        let copies = if concurrent || budget.ends_with("unchanged-asset") {
            3
        } else {
            1
        };
        let sizes = recorded["workload"]["sizes"]
            .as_array()
            .ok_or_else(|| format!("{budget}: missing asset workload"))?;
        let minimum = if large { 450_000 } else { 50_000 };
        if sizes.len() != expected
            || recorded["workload"]["copies"] != copies
            || sizes.iter().any(|size| {
                size.as_u64()
                    .is_none_or(|size| !(minimum..=500_000).contains(&size))
            })
            || !sizes
                .iter()
                .any(|size| size.as_u64().is_some_and(|size| size >= 450_000))
        {
            return Err(format!(
                "{budget}: asset workload differs: expected {expected} images, {copies} copies, each {minimum}..=500000 bytes, including a 450000..=500000 byte image"
            ));
        }
        recorded["value"]
            .as_f64()
            .filter(|value| value.is_finite() && *value >= 0.0)
            .ok_or_else(|| format!("{budget}: missing non-negative observed value"))?
    } else {
        recorded["value"].as_u64().ok_or_else(|| {
            format!(
                "{budget}: {} records no `value` that is a whole number of requests: {text}",
                path.display()
            )
        })? as f64
    };
    let detail = match &recorded["detail"] {
        serde_json::Value::Null => None,
        serde_json::Value::String(detail) => Some(detail.clone()),
        other => {
            return Err(format!(
                "{budget}: {} records a `detail` that is not a string: {other}",
                path.display()
            ));
        }
    };
    Ok((value, detail))
}
