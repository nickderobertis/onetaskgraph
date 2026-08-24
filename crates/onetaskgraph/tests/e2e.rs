//! Every journey this binary answers, driven the way a user drives it.
//!
//! Each test spawns the compiled binary as a subprocess and asserts on its exit
//! code, stdout and stderr — never an in-process `run()` call, and nothing about
//! the process is mocked. The journey table in `AGENTS.md` grows as verbs land;
//! these are the ones the binary answers today.

use assert_cmd::Command;
use predicates::str::contains;

/// The compiled binary, as a user's shell would find it.
fn onetaskgraph() -> Command {
    Command::cargo_bin("onetaskgraph").expect("the binary is built")
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
        .stdout(contains("schema"))
        .stdout(contains("config"))
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

    assert_eq!(bundle["version"], 2);

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
        "QueryResponseOfTask",
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
        .stdout(contains("`2` when the invocation itself was wrong"));
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
