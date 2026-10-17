//! The shared budget telemetry recorder; readers and workload validation belong to domains.

use serde_json::json;
use std::path::{Path, PathBuf};

/// The variable `scripts/ci-job.sh` sets to `1` where the instrumented run stands in for a live
/// crate's `test` (`scripts/live-crate-test.sh`), so the journeys recording a budget run
/// instrumented rather than under `test`.
pub const TESTS_UNDER_COVERAGE: &str = "ONETASKGRAPH_TESTS_UNDER_COVERAGE";

/// The directory cargo-llvm-cov builds an instrumented run into, inside the target directory.
pub const INSTRUMENTED_TARGET: &str = "llvm-cov-target";

/// The build directory a test executable's telemetry belongs under: the one it was built into,
/// or — under [`TESTS_UNDER_COVERAGE`], when it was built into [`INSTRUMENTED_TARGET`] — the
/// plain build of the same profile, which is where the budgets' report-only runs, built plainly,
/// look for it. `None` for a path with no build directory above it.
#[must_use]
pub fn build_directory(executable: &Path, under_coverage: bool) -> Option<PathBuf> {
    let mut directory = executable.parent()?.to_path_buf();
    if directory.file_name().is_some_and(|leaf| leaf == "deps") {
        directory.pop();
    }
    if under_coverage
        && let (Some(profile), Some(instrumented)) = (directory.file_name(), directory.parent())
        && instrumented
            .file_name()
            .is_some_and(|leaf| leaf == INSTRUMENTED_TARGET)
        && let Some(target) = instrumented.parent()
    {
        return Some(target.join(profile));
    }
    Some(directory)
}

/// [`build_directory`] of the running test executable, under the environment it runs in.
#[must_use]
pub fn running_build_directory() -> Option<PathBuf> {
    let under_coverage = std::env::var_os(TESTS_UNDER_COVERAGE).is_some_and(|value| value == "1");
    build_directory(&std::env::current_exe().ok()?, under_coverage)
}

/// One file-name spelling for a budget's recorder and reader.
#[must_use]
// llmlint: ignore[invalid_states_unrepresentable] Recording callers supply literal budget IDs; domain readers validate external IDs against onebudgetspec's ID_PATTERN before resolving a filename. The report-boundary journey proves traversal is refused without a figure.
pub fn file_in(directory: &Path, budget: &str) -> PathBuf {
    directory.join(format!("{budget}.json"))
}

/// Record a figure and how the ordinary test run counted it, without judging it.
// llmlint: ignore[invalid_states_unrepresentable] The shared test recorder preserves the Linear caller's string API; its recording callers supply literal IDs, and external IDs are validated by domain readers.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_instrumented_executable_records_beside_the_plain_build_only_under_coverage() {
        let target = Path::new("/work/target");
        let instrumented = target.join("llvm-cov-target/debug/deps/e2e-0123");
        let plain = target.join("debug/deps/e2e-0123");
        assert_eq!(
            build_directory(&instrumented, true),
            Some(target.join("debug"))
        );
        assert_eq!(
            build_directory(&instrumented, false),
            Some(target.join("llvm-cov-target/debug"))
        );
        assert_eq!(build_directory(&plain, true), Some(target.join("debug")));
        assert_eq!(build_directory(&plain, false), Some(target.join("debug")));
        assert_eq!(build_directory(Path::new("/"), true), None);
    }
}
