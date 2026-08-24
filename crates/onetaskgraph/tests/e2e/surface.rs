//! The command surface itself: what `--help` says, what `schema` emits, and what the
//! binary does when the thing it is writing to goes away.
//!
//! Nothing here reaches a source. The journeys that do are in the modules beside this
//! one, and they run against every row of the shared fixture table.

use assert_cmd::Command;
use predicates::str::contains;

/// The compiled binary, as a user's shell would find it.
fn onetaskgraph() -> Command {
    Command::cargo_bin("onetaskgraph").expect("the binary is built")
}

/// Every verb and flag the command surface owes, as `--help` must name them.
///
/// Listed here rather than asserted one at a time so that a verb added without help text
/// — or a flag renamed out from under the documentation — fails one obvious test.
const SURFACE: &[(&[&str], &[&str])] = &[
    (
        &["--help"],
        &[
            "sources", "task", "project", "label", "search", "schema", "config",
        ],
    ),
    (&["help", "sources"], &["list"]),
    (
        &["help", "task", "list"],
        &[
            "--source",
            "--label",
            "--not-label",
            "--status",
            "--project",
            "--no-project",
            "--search",
            "--in",
            "--limit",
            "--page",
            "--explain",
            "--allow-partial",
            "--json",
        ],
    ),
    (&["help", "task", "show"], &["<ID>"]),
    (&["help", "task", "deps"], &["--direction", "<ID>"]),
    (
        &["help", "project", "list"],
        &[
            "--source", "--label", "--status", "--search", "--in", "--limit",
        ],
    ),
    (&["help", "project", "show"], &["<ID>"]),
    (&["help", "project", "deps"], &["--direction", "<ID>"]),
    (&["help", "label", "list"], &["--source"]),
    (&["help", "search"], &["--in", "--kind", "<TEXT>"]),
];

#[test]
fn help_names_every_verb_and_flag_the_command_surface_owes() {
    for (arguments, expected) in SURFACE {
        let assertion = onetaskgraph().args(*arguments).assert().success();
        let rendered = String::from_utf8_lossy(&assertion.get_output().stdout).into_owned();
        for name in *expected {
            assert!(
                rendered.contains(name),
                "`onetaskgraph {}` does not mention {name}:\n{rendered}",
                arguments.join(" ")
            );
        }
    }
}

#[test]
fn a_project_list_has_no_project_filter_because_a_project_has_no_project() {
    let assertion = onetaskgraph()
        .args(["help", "project", "list"])
        .assert()
        .success();
    let rendered = String::from_utf8_lossy(&assertion.get_output().stdout).into_owned();
    assert!(!rendered.contains("--no-project"), "{rendered}");
}

#[test]
fn version_reports_the_crate_version_on_stdout_and_exits_zero() {
    let output = onetaskgraph()
        .arg("--version")
        .assert()
        .success()
        .get_output()
        .clone();

    let stdout = String::from_utf8(output.stdout).expect("utf-8");
    assert_eq!(
        stdout.trim(),
        format!("onetaskgraph {}", env!("CARGO_PKG_VERSION")),
        "--version must report the version this binary was built at"
    );
    assert!(output.stderr.is_empty(), "success stays quiet on stderr");
}

#[test]
fn help_names_the_product_and_every_verb_the_binary_answers() {
    onetaskgraph()
        .arg("--help")
        .assert()
        .success()
        .stdout(contains(
            "One interface over the ticketing systems your work lives in.",
        ))
        .stdout(contains("Usage: onetaskgraph [OPTIONS] <COMMAND>"))
        .stdout(contains("--version"))
        .stderr(predicates::str::is_empty());
}

#[test]
fn help_for_one_verb_explains_that_verb() {
    onetaskgraph()
        .args(["help", "schema"])
        .assert()
        .success()
        .stdout(contains("JSON Schema bundle"))
        .stdout(contains("Usage: onetaskgraph schema"));
}

#[test]
fn schema_emits_a_bundle_covering_every_contract_root_and_plugin_config() {
    // Both SDKs are generated from this document, so it is emitted by the running
    // binary rather than committed: the schema and the types that serialise are
    // the same types and cannot drift.
    let output = onetaskgraph()
        .arg("schema")
        .assert()
        .success()
        .get_output()
        .clone();
    assert!(output.stderr.is_empty(), "success stays quiet on stderr");

    let bundle: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("schema output is valid JSON");

    assert_eq!(bundle["version"], 3);

    let roots = bundle["roots"].as_object().expect("roots is an object");
    for root in [
        "Task",
        "Project",
        "Label",
        "Capabilities",
        "TaskQuery",
        "ProjectQuery",
        "PageOfTask",
        "SourceError",
        "GlobalId",
        "QueryPlan",
        "SourcePlan",
        "Predicate",
        "QualifiedTask",
        "QueryResponseOfQualifiedTask",
        "SearchHit",
        "SourceListing",
        "PageToken",
        // `config show --json` emits an EffectiveConfig, so an SDK is generated
        // against it from here like every other machine-readable output.
        "EffectiveConfig",
        "Setting",
        "Origin",
        "SecretsReport",
    ] {
        let schema = &roots[root];
        assert!(schema.is_object(), "the bundle is missing {root}");
        // Each root is a self-describing JSON Schema document, which is what a
        // generator needs in order to emit a model from it.
        assert!(
            schema["$schema"].is_string(),
            "{root} is not a self-describing schema document"
        );
    }

    let plugins = bundle["plugin_config"]
        .as_object()
        .expect("plugin_config is an object");
    for kind in ["github-projects", "in-memory", "linear", "local-md"] {
        assert!(
            plugins[kind].is_object(),
            "the registry can name {kind}, so the bundle must carry its config schema"
        );
    }
}

#[test]
fn schema_output_is_stable_across_runs_so_a_generator_can_diff_it() {
    let first = onetaskgraph()
        .arg("schema")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let second = onetaskgraph()
        .arg("schema")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(first, second, "the bundle must not vary between runs");
}

#[test]
fn help_documents_the_exit_codes_a_caller_scripts_against() {
    onetaskgraph()
        .arg("--help")
        .assert()
        .success()
        .stdout(contains("Exit codes: `0` on success"))
        .stdout(contains("`2` when the invocation itself was wrong"))
        .stdout(contains("`4` when a query succeeded for some sources"));
}

/// `/dev/full` accepts a write and then fails it with ENOSPC, which is the one portable
/// way to make the real binary's stdout fail deterministically. It is Linux-only, so this
/// journey is too; the same two failure paths are covered in-process on every platform by
/// the unit tests beside `emit_schema`.
#[cfg(target_os = "linux")]
#[test]
fn a_failed_write_to_stdout_exits_one_and_names_the_problem_on_stderr() {
    use std::fs::OpenOptions;
    use std::process::{Command as StdCommand, Stdio};

    let full = OpenOptions::new()
        .write(true)
        .open("/dev/full")
        .expect("/dev/full exists on Linux");

    let output = StdCommand::new(assert_cmd::cargo::cargo_bin("onetaskgraph"))
        .arg("schema")
        .stdout(Stdio::from(full))
        .stderr(Stdio::piped())
        .output()
        .expect("the binary runs");

    // 1, not 2: the invocation was correct and the run failed. A caller scripting around
    // this has to be able to tell those apart.
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("onetaskgraph: could not write the schema bundle"),
        "{stderr}"
    );
    assert!(!stderr.contains("panicked"), "{stderr}");
}

#[test]
fn an_unknown_verb_exits_non_zero_and_names_the_problem_on_stderr() {
    onetaskgraph()
        .arg("teleport")
        .assert()
        .failure()
        .code(2)
        .stderr(contains("unrecognized subcommand"))
        .stderr(contains("--help"));
}

#[test]
fn an_unknown_flag_exits_non_zero_and_names_the_problem_on_stderr() {
    onetaskgraph()
        .args(["schema", "--everything"])
        .assert()
        .failure()
        .code(2)
        .stderr(contains("unexpected argument"));
}

#[test]
fn no_verb_at_all_exits_non_zero_and_points_at_help() {
    onetaskgraph()
        .assert()
        .failure()
        .code(2)
        .stderr(contains("Usage: onetaskgraph"));
}

#[test]
fn a_closed_stdout_never_panics_however_the_race_lands() {
    // A user pipes `onetaskgraph schema | head -1`; the downstream end closes
    // early. The binary must report it, not panic and not exit zero having
    // written a truncated bundle.
    use std::io::Read as _;
    use std::process::{Command as StdCommand, Stdio};

    let mut child = StdCommand::new(assert_cmd::cargo::cargo_bin("onetaskgraph"))
        .arg("schema")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the binary starts");

    drop(child.stdout.take().expect("stdout is piped"));

    let mut stderr = String::new();
    child
        .stderr
        .take()
        .expect("stderr is piped")
        .read_to_string(&mut stderr)
        .expect("stderr reads");
    let status = child.wait().expect("the process exits");

    // Either the write landed before the reader vanished (success) or it failed,
    // in which case the binary says so and exits non-zero — never a panic.
    assert!(
        !stderr.contains("panicked"),
        "a closed pipe must not panic: {stderr}"
    );
    if !status.success() {
        assert!(
            stderr.contains("onetaskgraph: could not write the schema bundle"),
            "{stderr}"
        );
    }
}

#[test]
fn help_names_the_product_however_the_executable_on_disk_is_named() {
    // Windows appends `.exe` to the file, and clap takes its usage line from argv[0]
    // unless told otherwise — so without a pinned `bin_name` the help there would name
    // `onetaskgraph.exe`, a command no document tells a user to type. Copying the real
    // binary under another file name reproduces that condition on every platform.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let renamed = dir.path().join(format!(
        "onetaskgraph-under-another-name{}",
        std::env::consts::EXE_SUFFIX
    ));
    std::fs::copy(assert_cmd::cargo::cargo_bin("onetaskgraph"), &renamed)
        .expect("the binary copies");

    Command::new(&renamed)
        .arg("--help")
        .assert()
        .success()
        .stdout(contains("Usage: onetaskgraph [OPTIONS] <COMMAND>"));

    Command::new(&renamed)
        .args(["help", "schema"])
        .assert()
        .success()
        .stdout(contains("Usage: onetaskgraph schema"));
}

/// The README section that documents the command surface, read from the repository.
///
/// A path relative to this crate's manifest rather than the working directory, because a
/// test's working directory is the crate root and the document is two levels above it.
fn readme() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../README.md")
        .canonicalize()
        .expect("the README sits at the repository root");
    std::fs::read_to_string(path).expect("the README is readable")
}

#[test]
fn the_readme_documents_the_command_surface_this_binary_actually_has() {
    // The README spells the verbs, the flags and the exit codes a second time, for the
    // person deciding whether to install this at all. A second spelling drifts, and the
    // one that drifts is always the prose — so it is reconciled here against the binary's
    // own help rather than against a list somebody has to remember to update.
    let readme = readme();
    let mut missing = Vec::new();

    for (arguments, expected) in SURFACE {
        for name in *expected {
            // `<ID>` and `<TEXT>` are clap's placeholders, spelled in prose in the README.
            if name.starts_with('<') {
                continue;
            }
            if !readme.contains(name) {
                missing.push(format!("{} — {name}", arguments.join(" ")));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "the README does not document part of the surface `--help` reports:\n  {}",
        missing.join("\n  ")
    );

    // And the exit codes, which are the part a script depends on. `--help` is the
    // binary's own statement of them; every code it names has to appear in the README's
    // table, and the table must not invent one the binary does not use.
    let help = String::from_utf8_lossy(
        &onetaskgraph()
            .arg("--help")
            .assert()
            .success()
            .get_output()
            .stdout,
    )
    .into_owned();
    let documented: Vec<&str> = ["`0`", "`1`", "`2`", "`4`"]
        .into_iter()
        .filter(|code| help.contains(code))
        .collect();
    assert_eq!(
        documented.len(),
        4,
        "`--help` no longer names every exit code:\n{help}"
    );
    for code in documented {
        let row = format!("| {code} |");
        assert!(
            readme.contains(&row),
            "the README's exit-code table has no row for {code}"
        );
    }
    for invented in ["| `3` |", "| `5` |"] {
        assert!(
            !readme.contains(invented),
            "the README documents {invented}, which this binary never exits with"
        );
    }
}
