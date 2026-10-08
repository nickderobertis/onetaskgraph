//! The shared budget telemetry recorder; readers and workload validation belong to domains.

use serde_json::json;
use std::path::{Path, PathBuf};

/// One file-name spelling for a budget's recorder and reader.
#[must_use]
// llmlint: ignore[invalid_states_unrepresentable] This preserves the existing Linear test recorder's string API and behavior, as this task explicitly requires. Recording callers pass literal budget IDs; both domain readers validate environment-provided IDs against onebudgetspec's ID_PATTERN before calling this filename helper. The GitHub report-boundary journey proves traversal is refused without a figure.
pub fn file_in(directory: &Path, budget: &str) -> PathBuf {
    directory.join(format!("{budget}.json"))
}

/// Record a figure and how the ordinary test run counted it, without judging it.
// llmlint: ignore[invalid_states_unrepresentable] This is the existing test-only recorder moved unchanged with a re-export, per this task's compatibility requirement. Its callers supply literal IDs; environment input reaches domain readers that validate onebudgetspec's ID_PATTERN before resolving a filename. Changing this signature to a newtype would change the inherited recorder API rather than just share it.
pub fn record_in(directory: &Path, budget: &str, value: usize, detail: &str) {
    // llmlint: ignore[no_panics_on_recoverable_errors] This test-only recorder intentionally fails the journey if it cannot persist its evidence. No production call uses it.
    std::fs::create_dir_all(directory).expect("the telemetry directory is writable");
    // llmlint: ignore-block[no_panics_on_recoverable_errors] A failed telemetry write must fail the recording test rather than leave a successful journey with missing budget evidence. This preserves the existing test recorder contract; it is not a production error boundary.
    std::fs::write(
        file_in(directory, budget),
        json!({"value": value, "detail": detail}).to_string(),
    )
    .expect("the telemetry file is writable");
    // llmlint: ignore-end[no_panics_on_recoverable_errors]
}
