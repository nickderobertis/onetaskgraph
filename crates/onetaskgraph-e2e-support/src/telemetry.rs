//! The shared budget telemetry recorder; readers and workload validation belong to domains.

use serde_json::json;
use std::path::{Path, PathBuf};

/// One file-name spelling for a budget's recorder and reader.
#[must_use]
pub fn file_in(directory: &Path, budget: &str) -> PathBuf {
    directory.join(format!("{budget}.json"))
}

/// Record a figure and how the ordinary test run counted it, without judging it.
pub fn record_in(directory: &Path, budget: &str, value: usize, detail: &str) {
    std::fs::create_dir_all(directory).expect("the telemetry directory is writable");
    std::fs::write(
        file_in(directory, budget),
        json!({"value": value, "detail": detail}).to_string(),
    )
    .expect("the telemetry file is writable");
}
