//! The measure every clock-driven runner holds its spawn to, against the spawns it exists for:
//! the 48,024 ids that failed Windows with `os error 206`, and the command that replaced them.
use onetaskgraph_e2e_support::common::{
    WINDOWS_SPAWN_LIMIT, assert_spawnable_on_windows, spawn_lengths,
};
use std::process::Command;

/// `task show-many` of `ids` ids, as the points journey once spelled 2,001 batched reads.
fn show_many(ids: usize) -> Command {
    let mut command = Command::new("onetaskgraph");
    command.args(["task", "show-many"]);
    command.args((0..ids).map(|n| format!("board:ISSUE-QUERY-{n}")));
    command.arg("--json");
    command
}

#[test]
fn the_command_line_that_failed_windows_measures_past_its_bound() {
    let lengths = spawn_lengths(&show_many(24 * 2_001));
    assert!(lengths.command_line > WINDOWS_SPAWN_LIMIT, "{lengths:?}");
    let refused = std::panic::catch_unwind(|| assert_spawnable_on_windows(&show_many(24 * 2_001)))
        .expect_err("refused");
    let message = refused
        .downcast_ref::<String>()
        .expect("a formatted refusal");
    assert!(message.contains("os error 206"), "{message}");
}

#[test]
fn the_command_line_counts_every_argument_its_separator_and_its_quotes() {
    // `"onetaskgraph" "task" "comment" "list" "board:T-3" "--json"`.
    let mut command = Command::new("onetaskgraph");
    command.args(["task", "comment", "list", "board:T-3", "--json"]);
    assert_eq!(spawn_lengths(&command).command_line, 59);
    assert_spawnable_on_windows(&command);
    // A quote or a backslash may cost an escape of its own.
    let mut escaped = Command::new("x");
    escaped.arg(r#"a"b\c"#);
    assert_eq!(spawn_lengths(&escaped).command_line, 3 + 1 + 5 + 2 + 2);
}

#[test]
fn an_environment_variable_past_the_bound_is_refused_and_a_removed_one_is_not_counted() {
    let mut command = Command::new("onetaskgraph");
    command.env(
        "ONETASKGRAPH_TOO_LONG",
        "x".repeat(WINDOWS_SPAWN_LIMIT - "ONETASKGRAPH_TOO_LONG=".len()),
    );
    // `NAME=value` at the bound itself leaves no unit for its terminating null.
    assert_eq!(
        spawn_lengths(&command).longest_variable,
        WINDOWS_SPAWN_LIMIT
    );
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            assert_spawnable_on_windows(&command);
        }))
        .is_err()
    );
    let before = spawn_lengths(&command).environment_block;
    command.env_remove("ONETASKGRAPH_TOO_LONG");
    let after = spawn_lengths(&command);
    assert_eq!(before - after.environment_block, WINDOWS_SPAWN_LIMIT + 1);
    assert_spawnable_on_windows(&command);
}

/// This test executable, asked to run no test of its own, with a quoted argument of spaces
/// padding its command line to `length` as [`spawn_lengths`] measures it.
#[cfg(windows)]
fn this_executable_measured_at(length: usize) -> Command {
    let mut command = Command::new(std::env::current_exe().expect("this test's executable"));
    command.arg("--exact");
    let measured = spawn_lengths(&command).command_line;
    // ` "<padding>"`: a separator, two quotes and the padding, which holds a space so Windows
    // quotes it exactly as it is measured.
    let padding = length - measured - 3;
    command.arg(" ".repeat(padding));
    assert_eq!(spawn_lengths(&command).command_line, length);
    command
}

#[cfg(windows)]
#[test]
fn windows_spawns_a_command_line_measured_a_unit_inside_its_bound() {
    let mut command = this_executable_measured_at(WINDOWS_SPAWN_LIMIT - 1);
    assert_spawnable_on_windows(&command);
    let status = command
        .stdout(std::process::Stdio::null())
        .status()
        .expect("a command line the measure admits is one Windows spawns");
    assert!(status.success(), "{status:?}");
}

#[cfg(windows)]
#[test]
fn windows_refuses_a_command_line_past_its_bound_as_the_measure_does() {
    // The measure over-counts only the program's backslashes and `--exact`'s quotes, so the
    // line Windows is handed is at least that much shorter: two units past the bound once
    // those are added back.
    let program = std::env::current_exe().expect("this test's executable");
    let over = program.to_string_lossy().matches('\\').count() + 2;
    let mut command = this_executable_measured_at(WINDOWS_SPAWN_LIMIT + 1 + over);
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            assert_spawnable_on_windows(&command);
        }))
        .is_err()
    );
    let refused = command
        .stdout(std::process::Stdio::null())
        .status()
        .expect_err("Windows refuses a command line past its bound");
    assert_eq!(refused.raw_os_error(), Some(206), "{refused}");
}
