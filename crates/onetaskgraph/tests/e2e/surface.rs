//! The command surface itself: what `--help` says, what `schema` emits, and what the
//! binary does when the thing it is writing to goes away.
//!
//! Nothing here reaches a source. The journeys that do are in the modules beside this
//! one, and they run against every row of the shared fixture table.

use assert_cmd::Command;
use predicates::str::contains;

use crate::common::Sandbox;

fn onetaskgraph() -> Command {
    Command::new(env!("CARGO_BIN_EXE_onetaskgraph"))
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

    assert_eq!(bundle["version"], 5);
    assert_eq!(
        bundle["commands"],
        serde_json::json!([
            "schema",
            "config show",
            "sources list",
            "task list",
            "task show",
            "task deps",
            "project list",
            "project show",
            "project deps",
            "label list",
            "search"
        ])
    );

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
        "SourceListings",
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

    let output = StdCommand::new(env!("CARGO_BIN_EXE_onetaskgraph"))
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

/// `--project` and `--no-project` ask for opposite things, so asking for both is refused.
///
/// A task is in a project or it is in none, and a filter cannot keep both sets at once. The
/// two flags are declared mutually exclusive, and a declaration nothing invokes is a
/// declaration that can be dropped in a refactor without anything noticing — so this drives
/// the pair the way a user types them and asserts the invocation exit code and the
/// diagnostic naming both flags, rather than trusting the attribute.
#[test]
fn asking_for_a_project_and_for_no_project_at_once_is_refused_as_a_bad_invocation() {
    onetaskgraph()
        .args(["task", "list", "--project", "P-1", "--no-project"])
        .assert()
        .failure()
        .code(2)
        .stderr(contains("--project"))
        .stderr(contains("cannot be used with"))
        .stderr(contains("--no-project"));
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

    let mut child = StdCommand::new(env!("CARGO_BIN_EXE_onetaskgraph"))
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

/// Wait until `path` can actually be executed.
///
/// The tests of one target run as threads of one process, and a copy made by one thread
/// races every other thread's spawn: `fs::copy` holds a write descriptor, `fork` puts a
/// copy of it in the child's table, and the kernel refuses `execve` on a file anything
/// has open for writing — `ETXTBSY` — before close-on-exec ever gets a chance to run.
/// Nothing in this test's own code can prevent that, because the descriptor belongs to a
/// different test; the window is a few microseconds wide and closes on its own.
///
/// So this waits for it rather than failing the suite over a race it does not own, and
/// fails loudly if the file is still not executable after a wait no real one would need.
fn runnable(path: &std::path::Path) {
    let mut last = None;
    for _ in 0..200 {
        match std::process::Command::new(path).arg("--version").output() {
            Ok(_) => return,
            Err(error) => {
                last = Some(error);
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
    }
    panic!(
        "the copied binary would not run after two seconds: {}",
        last.expect("the loop ran at least once")
    );
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
    std::fs::copy(env!("CARGO_BIN_EXE_onetaskgraph"), &renamed).expect("the binary copies");
    runnable(&renamed);

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

/// Each command-line vocabulary, the contract root it mirrors, and whether the two spell
/// their values the same way.
///
/// `--status` and `--direction` deliberately borrow the contract's own spellings, so
/// those two must agree value for value. `--in` and `--kind` deliberately do not — a user
/// types `--in both`, not `--in title-or-content`, and `--kind task`, not `--kind tasks` —
/// so for those the reconciliation is that the command line can name **as many** things as
/// the contract has, which is what an added variant breaks.
const VOCABULARIES: &[(&[&str], &str, &str, bool)] = &[
    (
        &["help", "task", "list"],
        "--status",
        "StatusCategory",
        true,
    ),
    (&["help", "task", "deps"], "--direction", "Direction", true),
    (&["help", "search"], "--in", "TextFields", false),
    (&["help", "search"], "--kind", "SearchKind", false),
];

/// The values clap says `flag` takes, read out of this help text.
///
/// From the help rather than from the enum itself, because a test target cannot reach a
/// binary crate's own modules — and because what a user can actually type is what the
/// help says, which makes reading it the stronger of the two anyway.
fn possible_values(help: &str, flag: &str) -> Vec<String> {
    let after = help
        .split_once(flag)
        .unwrap_or_else(|| panic!("`{flag}` is not in this help text:\n{help}"))
        .1;
    let block = after
        .split_once("Possible values:")
        .unwrap_or_else(|| panic!("`{flag}` prints no possible values:\n{help}"))
        .1;
    let values: Vec<String> = block
        .lines()
        .map(str::trim)
        .skip_while(|line| line.is_empty())
        .take_while(|line| line.starts_with("- "))
        .map(|line| {
            line.trim_start_matches("- ")
                .split_once(':')
                .expect("clap writes `- value: description`")
                .0
                .to_owned()
        })
        .collect();
    assert!(!values.is_empty(), "`{flag}` lists no values:\n{help}");
    values
}

#[test]
fn the_command_line_accepts_exactly_the_vocabularies_the_contract_declares() {
    // `StatusArg`, `FieldsArg`, `DirectionArg` and `KindArg` each mirror an enum of the
    // contract or the engine, and they exist so that deriving clap's `ValueEnum` does not
    // put clap into the plugin contract's dependencies for the sake of four flags. A
    // mirror drifts: add a status category upstream and nothing here stops compiling,
    // nothing fails, and the command line simply cannot name it any more.
    //
    // So the two are reconciled against the schema bundle this binary emits, which is
    // generated from the contract types themselves and is therefore the one document that
    // cannot disagree with them.
    let sandbox = Sandbox::new();
    let bundle: serde_json::Value = serde_json::from_slice(
        &sandbox
            .command()
            .arg("schema")
            .assert()
            .success()
            .get_output()
            .stdout,
    )
    .expect("the bundle is JSON");

    for (arguments, flag, root, same_spelling) in VOCABULARIES {
        let help = String::from_utf8_lossy(
            &onetaskgraph()
                .args(*arguments)
                .assert()
                .success()
                .get_output()
                .stdout,
        )
        .into_owned();
        let accepted = possible_values(&help, flag);

        // Each variant carries its own doc comment into the bundle, so schemars writes a
        // `oneOf` of `const`s rather than a bare `enum` — which is the shape a generator
        // wants and is where the values are.
        let mut declared: Vec<String> = bundle["roots"][root]["oneOf"]
            .as_array()
            .unwrap_or_else(|| panic!("the bundle's {root} root declares no values"))
            .iter()
            .map(|variant| {
                variant["const"]
                    .as_str()
                    .unwrap_or_else(|| panic!("{root} declares a variant with no value"))
                    .to_owned()
            })
            .collect();

        assert_eq!(
            accepted.len(),
            declared.len(),
            "`{flag}` accepts {accepted:?} while the contract's {root} declares \
             {declared:?}. A variant added to one and not the other is a value a user \
             can never name — add it to the mirror in crates/onetaskgraph/src/cli.rs."
        );

        if *same_spelling {
            let mut accepted = accepted;
            accepted.sort();
            declared.sort();
            assert_eq!(
                accepted, declared,
                "`{flag}` borrows {root}'s own spellings, so the two must agree value for \
                 value"
            );
        }
    }
}

#[test]
fn every_value_the_help_advertises_is_one_the_command_line_actually_takes() {
    // The other half, and it reads the help rather than the bundle on purpose: the test
    // above has already established that the help's vocabulary and the contract's are the
    // same set, so what is left to prove is that each value the help prints is one the
    // running binary accepts rather than merely advertises. Reading the bundle again here
    // would prove the same equality twice and this property not at all.
    let sandbox = Sandbox::new();
    sandbox.project_document(&crate::fixtures::ROWS[0].document(&sandbox));

    for (arguments, flag, _, _) in VOCABULARIES {
        let help = String::from_utf8_lossy(
            &onetaskgraph()
                .args(*arguments)
                .assert()
                .success()
                .get_output()
                .stdout,
        )
        .into_owned();
        for value in possible_values(&help, flag) {
            let verb: Vec<&str> = match *flag {
                "--status" => vec!["task", "list"],
                "--direction" => vec!["task", "deps", "work:T-1"],
                _ => vec!["search", "alpha"],
            };
            sandbox
                .command()
                .args(verb)
                .args([*flag, value.as_str()])
                .assert()
                .success();
        }
    }
}
