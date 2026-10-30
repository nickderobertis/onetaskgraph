//! The Linear request budgets run on every CI runner the same way, Windows included.
//!
//! Windows reads no `#!` line and has no POSIX shell to hand a command to, so a budget that
//! names a script or leans on shell syntax would run on two runners and fail on the third. These
//! hold `budgets.yaml`'s commands and this project's `budgets` target to programs every runner
//! starts directly, the telemetry they read to where `test` writes it, and the reporter it
//! links to the release of the onebudgetspec that judges it.

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
    let file: Value = serde_saphyr::from_str(&manifest("budgets.yaml")).expect("budgets.yaml");
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
        .any(|pair| pair[0] == "--" && pair[1] == "budget_runner::report");
    assert!(runner, "the command runs `budget_runner::report`: {argv:?}");
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

/// The version `text` pins after `prefix`. Both manifests spell a pin as one `name = value`
/// line, so reading that line is enough and needs no parser of either format.
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

#[test]
fn the_telemetry_the_journeys_write_is_what_test_declares_and_budgets_hashes() {
    let project: Value = serde_json::from_str(&manifest("project.json")).expect("project.json");
    let targets = &project["targets"];
    assert_eq!(
        targets["test"]["outputs"],
        serde_json::json!([format!(
            "{{workspaceRoot}}/target/debug/{}",
            telemetry::BELOW_BUILD
        )]),
        "a cache hit of `test` restores the telemetry `budgets` reads"
    );
    let hashed = format!("**/{}/*.json", telemetry::BELOW_BUILD);
    assert!(
        targets["budgets"]["inputs"]
            .as_array()
            .expect("inputs")
            .contains(&serde_json::json!({"dependentTasksOutputFiles": hashed})),
        "`budgets` is keyed on the telemetry it reads: {:#}",
        targets["budgets"]
    );
    // Only the build's own directory is what `test` declares; a test's override is not. A plain
    // run writes into it — which is how this suite's journeys record the budgets, uninstrumented
    // — and so would an instrumented run standing in for `test`, so the reports, built plainly,
    // find what they recorded. cargo-llvm-cov's own variable says a run is instrumented, whatever directory it
    // builds into, so a run that moved would fail here rather than leave `budgets` nothing.
    if std::env::var_os(telemetry::DIRECTORY).is_none() {
        let instrumented = std::env::var_os("LLVM_PROFILE_FILE").is_some();
        let standing_in = onetaskgraph_e2e_support::telemetry::tests_under_coverage();
        let directory = telemetry::directory();
        if !instrumented || standing_in {
            let build = directory
                .ancestors()
                .nth(
                    std::path::Path::new(telemetry::BELOW_BUILD)
                        .components()
                        .count(),
                )
                .expect("the telemetry sits below a build directory");
            let declared =
                std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/debug");
            assert!(
                directory.ends_with(telemetry::BELOW_BUILD)
                    && std::fs::canonicalize(build).expect("the build directory exists")
                        == std::fs::canonicalize(&declared).expect("target/debug exists"),
                "the journeys record into {}, not the directory `test` declares and the \
                 reports read, below {}",
                directory.display(),
                declared.display()
            );
        } else {
            assert!(
                directory.ends_with(telemetry::BELOW_BUILD),
                "{}",
                directory.display()
            );
        }
    }
}

#[test]
fn the_query_guard_the_cache_journey_reports_is_the_threshold_onebudgetspec_enforces() {
    let file: Value = serde_saphyr::from_str(&manifest("budgets.yaml")).expect("budgets.yaml");
    let guard = file["budgets"]
        .as_array()
        .expect("a list of budgets")
        .iter()
        .find(|budget| budget["id"] == crate::cache_budget::GUARD)
        .expect("the guard is declared");
    assert_eq!(
        guard["threshold"].as_u64(),
        Some(crate::cache_budget::price::GUARD),
        "cache_budget::price::GUARD is what {} enforces",
        crate::cache_budget::GUARD
    );
    assert_eq!(guard["direction"], "max");
}

/// `n` spelled as the protocol document spells a figure: with a comma between each three digits.
fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

#[test]
fn the_protocols_price_table_and_budget_thresholds_are_the_ones_the_budgets_use() {
    use crate::cache_budget::price as p;
    let protocol = manifest("../../docs/plugin-protocol.md");
    let page = onetaskgraph_linear::change_token::PAGE as u64;
    let projects = u64::from(onetaskgraph_linear::PROJECTS_CACHED_PAGE);
    let task_pages = p::RELATIONS_PAGE + p::INVERSE_RELATIONS_PAGE + p::COMMENTS_PAGE;
    let at = |at_100: u64, first: u64| (at_100 * first).div_ceil(100);
    let row = p::ISSUES_ROW + task_pages;
    let project_row =
        p::PROJECTS_ROW + p::PROJECT_RELATIONS_PAGE + p::PROJECT_INVERSE_RELATIONS_PAGE;
    for line in [
        format!(
            "| `ISSUES`, at 100 rows | {} + {} a row: {} | {} + {row} a row: **{}** |",
            p::ISSUES_BASE,
            p::ISSUES_ROW,
            grouped(p::ISSUES_BASE + p::ISSUES_ROW * 100),
            p::ISSUES_BASE,
            grouped(p::ISSUES_BASE + row * 100)
        ),
        format!("| `ISSUE` | {} | {} |", p::ISSUE, p::ISSUE + task_pages),
        format!(
            "| `ISSUE_DETAIL`, at `last:100` | {} | {} |",
            p::ISSUE_DETAIL,
            p::ISSUE_DETAIL + p::RELATIONS_PAGE + p::INVERSE_RELATIONS_PAGE
        ),
        format!(
            "| `ISSUE_RELATIONS` | {}, at `first:100` | {}, at `first:{page}` |",
            p::RELATIONS_AT_100,
            at(p::RELATIONS_CACHED_AT_100, page) + p::COMMENTS_PAGE
        ),
        format!(
            "| `PROJECTS` | {} a row asked for: {} at 100 | {project_row} a row asked for, at most \
             {projects}: **{}** |",
            p::PROJECTS_ROW,
            grouped(p::PROJECTS_ROW * 100),
            grouped(project_row * projects)
        ),
        format!(
            "| `PROJECT` | {} | {} |",
            p::PROJECT,
            p::PROJECT + p::PROJECT_RELATIONS_PAGE + p::PROJECT_INVERSE_RELATIONS_PAGE
        ),
        format!(
            "| `PROJECT_RELATIONS` | {}, at `first:100` | {}, at `first:{page}` |",
            p::RELATIONS_AT_100,
            at(p::RELATIONS_CACHED_AT_100, page)
        ),
    ] {
        assert!(
            protocol.contains(&line),
            "docs/plugin-protocol.md §4.13e states `{line}`, the price the hour budget uses"
        );
    }

    let file: Value = serde_saphyr::from_str(&manifest("budgets.yaml")).expect("budgets.yaml");
    let threshold = |id: &str| {
        file["budgets"]
            .as_array()
            .expect("a list of budgets")
            .iter()
            .find(|budget| budget["id"] == id)
            .and_then(|budget| budget["threshold"].as_u64())
            .unwrap_or_else(|| panic!("{id} has a whole threshold"))
    };
    let mut ids: Vec<&str> = crate::cache_budget::ABSOLUTE
        .iter()
        .map(|(id, _, _)| *id)
        .collect();
    ids.push(crate::cache_budget::GUARD);
    for id in ids {
        let stated = protocol
            .find(&format!("`{id}`"))
            .map(|at| &protocol[at..(at + id.len() + 80).min(protocol.len())])
            .unwrap_or_else(|| panic!("docs/plugin-protocol.md names `{id}`"));
        assert!(
            stated.contains(&grouped(threshold(id))),
            "docs/plugin-protocol.md states `{id}`'s threshold, {}, beside it: {stated}",
            grouped(threshold(id))
        );
    }
}
