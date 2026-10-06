//! Where the Linear request budgets' telemetry lives: one JSON file per budget,
//! `{"value": <requests>, "detail": "<how they were counted>"}`, named after the budget's id.
//!
//! The `measure_*` journeys of the `e2e` target write it while that target runs, and
//! `tests/budgets/main.rs` reads it back for `onebudgetspec check`. The directory is
//! `<target>/<profile>/telemetry/onetaskgraph-linear-e2e`, found from the running test
//! executable the way `onetaskgraph_e2e_support::binary` finds the binary, so the writer and
//! the reader — two test executables of one build — agree on it whatever the target directory
//! is. It is never committed, and it is the `test` target's declared output, so a cache hit of
//! that target restores what its last run recorded.

use std::path::PathBuf;

/// The file `budget`'s telemetry is recorded in.
pub fn file(budget: &str) -> PathBuf {
    let executable = std::env::current_exe().expect("the running test executable has a path");
    let mut directory = executable
        .parent()
        .expect("a test executable sits inside a build directory")
        .to_path_buf();
    if directory.file_name().is_some_and(|leaf| leaf == "deps") {
        directory.pop();
    }
    directory
        .join("telemetry")
        .join("onetaskgraph-linear-e2e")
        .join(format!("{budget}.json"))
}

/// The figure and the detail `budget`'s journey recorded, or why there is none to read.
pub fn recorded(budget: &str) -> Result<(f64, Option<String>), String> {
    let path = file(budget);
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
    let value = recorded["value"].as_f64().ok_or_else(|| {
        format!(
            "{budget}: {} records no numeric `value`: {text}",
            path.display()
        )
    })?;
    Ok((value, recorded["detail"].as_str().map(str::to_owned)))
}
