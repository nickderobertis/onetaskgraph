//! The Linear request budgets run on every CI runner the same way, Windows included.
//!
//! Windows reads no `#!` line and has no POSIX shell to hand a command to, so a budget that
//! names a script or leans on shell syntax would run on two runners and fail on the third. These
//! hold `budgets.yaml`'s commands and this project's `budgets` target to programs every runner
//! starts directly, the runner they name to the failures it owes, and the reporter it links to
//! the release of the onebudgetspec that judges it.

use std::path::Path;

use serde_json::Value;

use crate::telemetry;

/// Programs that are a shell, or that hand their arguments to one.
const SHELLS: &[&str] = &[
    "bash",
    "sh",
    "dash",
    "zsh",
    "ksh",
    "fish",
    "pwsh",
    "powershell",
    "cmd",
    "env",
];

/// Suffixes that name a script, which Windows will not start by its `#!` line.
const SCRIPTS: &[&str] = &[".sh", ".bash", ".zsh", ".ps1", ".bat", ".cmd"];

/// Characters only a shell gives a meaning to: expansion, pipes, lists, redirection, globs.
const SHELL_SYNTAX: &[char] = &['$', '|', '&', ';', '<', '>', '`', '*', '?', '~', '(', ')'];

/// Why `word`, one word of a command, would not run the same on every runner.
fn unportable(word: &str) -> Option<String> {
    let program = word.to_ascii_lowercase();
    if SHELLS.contains(&program.trim_end_matches(".exe")) {
        return Some(format!("`{word}` is a shell"));
    }
    if let Some(suffix) = SCRIPTS.iter().find(|suffix| program.ends_with(*suffix)) {
        return Some(format!("`{word}` names a {suffix} script"));
    }
    if let Some(syntax) = word.chars().find(|c| SHELL_SYNTAX.contains(c)) {
        return Some(format!("`{word}` carries the shell syntax `{syntax}`"));
    }
    None
}

fn manifest(relative: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

#[test]
fn every_budget_names_the_one_runner_and_no_shell() {
    let file: Value = serde_norway::from_str(&manifest("budgets.yaml")).expect("budgets.yaml");
    let budgets = file["budgets"].as_array().expect("a list of budgets");
    assert!(!budgets.is_empty(), "budgets.yaml declares no budget");
    let first = &budgets[0]["command"];
    for budget in budgets {
        let id = budget["id"].as_str().expect("an id");
        let command = budget["command"].as_array().expect("an argv");
        assert_eq!(
            &budget["command"], first,
            "{id}: every budget names the one runner, which reads ONEBUDGETSPEC_BUDGET_ID"
        );
        let program = command[0].as_str().expect("a program");
        assert!(
            !program.contains(['/', '\\']),
            "{id}: `{program}` is a path, which Windows resolves only to an .exe"
        );
        for word in command {
            let word = word.as_str().expect("a word");
            assert_eq!(unportable(word), None, "{id}: {command:?}");
        }
    }
    let argv = first.as_array().expect("an argv");
    let runner = argv
        .windows(2)
        .any(|pair| pair[0] == "--test" && pair[1] == "budgets");
    assert!(runner, "the command runs the `budgets` runner: {argv:?}");
}

#[test]
fn the_budgets_target_runs_onebudgetspec_alone_and_is_cached_on_no_credential() {
    let project: Value = serde_json::from_str(&manifest("project.json")).expect("project.json");
    let target = &project["targets"]["budgets"];
    assert_eq!(
        target["options"]["commands"],
        Value::Null,
        "one command: {target:#}"
    );
    let command = target["options"]["command"].as_str().expect("one command");
    let words = command.split_whitespace().collect::<Vec<_>>();
    for word in &words {
        assert_eq!(unportable(word), None, "{command}");
    }
    assert!(
        command.contains("onebudgetspec check crates/onetaskgraph-linear-e2e/budgets.yaml"),
        "{command}"
    );
    assert_eq!(target["cache"], true, "{target:#}");
    let inputs = target["inputs"].as_array().expect("inputs");
    assert!(
        inputs.iter().all(|input| input.get("env").is_none()),
        "the figures read nothing of the host, so no variable is a key: {inputs:#?}"
    );
    assert!(
        target["dependsOn"]
            .as_array()
            .is_some_and(|tasks| tasks.contains(&Value::from("test"))),
        "it analyses what `test` recorded: {target:#}"
    );
    let check = project["targets"]["check"]["dependsOn"]
        .as_array()
        .expect("check's dependencies");
    assert!(check.contains(&Value::from("budgets")), "{check:?}");
}

#[test]
fn a_budget_whose_journey_recorded_nothing_is_refused_naming_the_target_that_records_it() {
    let refused = telemetry::recorded("linear-requests-never-recorded")
        .expect_err("nothing recorded this budget");
    assert!(
        refused.starts_with("linear-requests-never-recorded: no telemetry at "),
        "{refused}"
    );
    assert!(
        refused.contains("scripts/nx.sh run onetaskgraph-linear-e2e:test"),
        "{refused}"
    );
}

#[test]
fn a_budget_id_that_is_not_one_names_no_file() {
    for id in ["../escape", "/abs", "Upper", "", "a/b"] {
        let refused = telemetry::recorded(id).expect_err("not an id");
        assert!(refused.contains("is not a budget id"), "{id:?}: {refused}");
    }
}

/// The version `text` gives after `prefix`, on the one line that starts with it.
fn pinned<'a>(text: &'a str, prefix: &str) -> &'a str {
    let line = text
        .lines()
        .find_map(|line| line.trim().strip_prefix(prefix))
        .unwrap_or_else(|| panic!("no line starts with {prefix}"));
    line.trim_matches(|c: char| c == '"' || c == ',' || c == '=' || c.is_whitespace())
}

#[test]
fn the_reporter_is_the_release_of_the_onebudgetspec_that_judges_it() {
    let workspace = manifest("../../Cargo.toml");
    let packages = manifest("../../package.json");
    let cargo = pinned(&workspace, "onebudgetspec-core = ");
    let npm = pinned(&packages, "\"@onebudgetspec/cli\": ");
    assert_eq!(
        cargo, npm,
        "Cargo.toml's onebudgetspec-core and package.json's @onebudgetspec/cli are one release: \
         one release workflow publishes both, so move them together"
    );
}
