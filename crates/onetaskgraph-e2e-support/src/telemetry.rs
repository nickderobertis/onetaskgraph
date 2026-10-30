//! The shared budget telemetry recorder; readers and workload validation belong to domains.

use serde_json::json;
use std::path::{Path, PathBuf};

/// The variable that narrows a live crate's `test` to its credentialed journey
/// (`scripts/live-crate-test.sh`) where an instrumented run stands in for the rest, so the
/// journeys recording a budget run instrumented. No job sets it since the live and e2e suites
/// run uninstrumented (coverage:uninstrumented); it is read for any caller that does.
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

/// Whether [`TESTS_UNDER_COVERAGE`] is set: `1`, or unset or empty, exactly as
/// `scripts/live-crate-test.sh` reads it (a test below holds the two to one reading).
///
/// # Errors
///
/// Any other value, named, because it is neither of the two modes.
pub fn under_coverage(value: Option<&std::ffi::OsStr>) -> Result<bool, String> {
    match value {
        None => Ok(false),
        Some(value) if value.is_empty() => Ok(false),
        Some(value) if value == "1" => Ok(true),
        Some(value) => Err(format!(
            "{TESTS_UNDER_COVERAGE} is {value:?}; it is 1 or unset (scripts/live-crate-test.sh)"
        )),
    }
}

/// [`under_coverage`] of the running test's environment.
///
/// # Panics
///
/// When the variable is set to anything but `1`: a test cannot tell which build its telemetry
/// belongs beside, so it stops rather than record it in the wrong one.
#[must_use]
pub fn tests_under_coverage() -> bool {
    // llmlint: ignore[no_panics_on_recoverable_errors] Test support: a misconfigured run has nothing to recover to, and the panic names the variable and its two values.
    under_coverage(std::env::var_os(TESTS_UNDER_COVERAGE).as_deref())
        .unwrap_or_else(|problem| panic!("{problem}"))
}

/// [`build_directory`] of the running test executable, under the environment it runs in.
#[must_use]
pub fn running_build_directory() -> Option<PathBuf> {
    build_directory(&std::env::current_exe().ok()?, tests_under_coverage())
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

    #[test]
    fn the_coverage_mode_is_one_or_unset_and_refuses_anything_else() {
        assert_eq!(under_coverage(None), Ok(false));
        assert_eq!(under_coverage(Some("".as_ref())), Ok(false));
        assert_eq!(under_coverage(Some("1".as_ref())), Ok(true));
        for value in ["0", "true", "yes", " 1"] {
            let problem = under_coverage(Some(value.as_ref())).expect_err(value);
            assert!(problem.contains(TESTS_UNDER_COVERAGE), "{problem}");
            assert!(problem.contains("1 or unset"), "{problem}");
        }
    }

    // The shell reader is the variable's other consumer: the same three arms, `1`, empty or
    // unset (`${…:-}`), and a refusal of anything else.
    #[test]
    fn the_shell_reader_reads_the_coverage_mode_the_same_way() {
        let script = include_str!("../../../scripts/live-crate-test.sh");
        let arms = format!(
            "case \"${{{TESTS_UNDER_COVERAGE}:-}}\" in\n  1) run_journey ;;\n  \"\") ;;\n  *)"
        );
        assert!(
            script.contains(&arms),
            "scripts/live-crate-test.sh no longer reads {TESTS_UNDER_COVERAGE} as `1`, empty or unset, refusing the rest"
        );
    }
}
