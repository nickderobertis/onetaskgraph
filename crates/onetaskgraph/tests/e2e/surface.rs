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
            "sources",
            "task",
            "project",
            "label",
            "search",
            "schema",
            "config",
            "template",
            "--interactive",
            "--no-interactive",
        ],
    ),
    (&["help", "sources"], &["list", "fields"]),
    (
        &["help", "sources", "fields"],
        &["<SOURCE>", "--apply", "--json"],
    ),
    (
        &["help", "task", "list"],
        &[
            "--source",
            "--label",
            "--not-label",
            "--status",
            "--priority",
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
    (
        &["help", "task", "comment"],
        &["add", "list", "edit", "delete"],
    ),
    (
        &["help", "task", "comment", "add"],
        &["<ID>", "--body-file", "--author", "--json"],
    ),
    (&["help", "task", "comment", "list"], &["<ID>", "--json"]),
    (
        &["help", "task", "comment", "edit"],
        &["<ID>", "<COMMENT-ID>", "--body-file", "--json"],
    ),
    (
        &["help", "task", "comment", "delete"],
        &["<ID>", "<COMMENT-ID>", "--json"],
    ),
    (&["help", "task", "deps"], &["--direction", "<ID>"]),
    (&["help", "task", "priority"], &["set"]),
    (
        &["help", "task", "priority", "set"],
        &["<ID>", "<PRIORITY>"],
    ),
    (&["help", "task", "content"], &["set"]),
    (&["help", "task", "content", "set"], &["<ID>", "--file"]),
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
    (&["help", "template"], &["variables", "render"]),
    (
        &["help", "template", "variables"],
        &["<FILE>", "--search-path", "--template-loader", "--json"],
    ),
    (
        &["help", "template", "render"],
        &[
            "<FILE>",
            "--search-path",
            "--template-loader",
            "--answers",
            "--var",
            "--no-interactive",
            "--json",
        ],
    ),
    (&["help", "task"], &["create", "render", "answers"]),
    (
        &["help", "task", "create"],
        &[
            "<SOURCE>",
            "--project",
            "--title",
            "--template",
            "--search-path",
            "--template-loader",
            "--answers",
            "--var",
            "--body-file",
            "--status",
            "--label",
            "--repository",
            "--depends-on",
            "--delivers",
            "--metadata",
            "--json",
        ],
    ),
    (
        &["help", "task", "render"],
        &[
            "<ID>",
            "--template",
            "--search-path",
            "--template-loader",
            "--answers",
            "--var",
            "--unset",
            "--dry-run",
            "--json",
        ],
    ),
    (&["help", "task", "answers"], &["<ID>", "--json"]),
    (&["help", "document"], &["create", "render", "answers"]),
    (
        &["help", "document", "create"],
        &[
            "<SOURCE>",
            "--project",
            "--title",
            "--id",
            "--template",
            "--template-loader",
            "--answers",
            "--var",
            "--body-file",
            "--label",
            "--repository",
            "--metadata",
            "--json",
        ],
    ),
    (
        &["help", "document", "render"],
        &[
            "<ID>",
            "--template",
            "--template-loader",
            "--var",
            "--unset",
            "--dry-run",
        ],
    ),
    (&["help", "document", "answers"], &["<ID>", "--json"]),
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

    assert_eq!(bundle["version"], 25);
    assert!(
        bundle["roots"]["FailureDocument"].is_object(),
        "the document a failed command writes under machine output is a root"
    );
    assert_eq!(
        bundle["commands"],
        serde_json::json!([
            "schema",
            "config show",
            "sources list",
            "sources status-options",
            "sources fields",
            "task list",
            "task show",
            "task deps",
            "task copy",
            "task comment add",
            "task comment list",
            "task comment edit",
            "task comment delete",
            "task status set",
            "task priority set",
            "task content set",
            "task metadata set",
            "task update",
            "task create",
            "task render",
            "task answers",
            "project list",
            "project show",
            "project deps",
            "project copy",
            "project metadata set",
            "document list",
            "document show",
            "document copy",
            "document metadata set",
            "document create",
            "document render",
            "document answers",
            "label list",
            "search",
            "template variables",
            "template render"
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
        "StatusOptionsReport",
        "FieldsReport",
        "Priority",
        "TaskPrioritySet",
        "TaskContentSet",
        "Document",
        "Location",
        "DocumentQuery",
        "PageOfDocument",
        "QualifiedDocument",
        "QueryResponseOfQualifiedDocument",
        "CopyReport",
        "CopyOutcome",
        "CopyAction",
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
        // What `task render` and `document render`, and `task answers` and `document
        // answers`, answer with, and the provenance entry a rendered item records.
        "Regenerated",
        "TemplateAnswers",
        "TemplateProvenance",
        // What `task update` is given and answers with, the field vocabulary it reports
        // in, and the outcome a plugin answers a targeted update with.
        "TaskUpdate",
        "TaskUpdated",
        "UpdatedField",
        "TaskUpdateOutcome",
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

/// The README's synopsis entry for `command`: its `onetaskgraph <command>` line and every
/// indented line continuing it, or `None` when it has no such entry.
fn synopsis(readme: &str, command: &str) -> Option<String> {
    let head = format!("onetaskgraph {command} ");
    let mut lines = readme.lines().skip_while(|line| !line.starts_with(&head));
    let first = lines.next()?;
    let continued = lines.take_while(|line| line.starts_with(' '));
    Some(
        std::iter::once(first)
            .chain(continued)
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

#[test]
fn each_template_verb_s_readme_entry_names_the_flags_that_verb_takes() {
    // The check above finds a flag anywhere in the README, so a flag moved to the wrong verb's
    // entry passes it. For the verbs a template drives, each flag `<verb> --help` reports is
    // held to that verb's own entry — or, for a `document` verb whose entry says it takes the
    // flags of a `task` verb, to that entry too. Global flags belong to no one verb.
    let global = [
        "set",
        "page-size",
        "default-sources",
        "output",
        "json",
        "interactive",
        "no-interactive",
        "help",
    ];
    let readme = readme();
    let mut missing = Vec::new();
    for command in [
        "task create",
        "task render",
        "task answers",
        "document create",
        "document render",
        "document answers",
        "template variables",
        "template render",
    ] {
        let own = synopsis(&readme, command)
            .unwrap_or_else(|| panic!("the README has no synopsis entry for `{command}`"));
        let mut entry = own.clone();
        for borrowed in ["task create", "task render"] {
            if own.contains(&format!("of `{borrowed}`")) {
                entry.push_str(&synopsis(&readme, borrowed).unwrap_or_default());
            }
        }
        let help = String::from_utf8(
            onetaskgraph()
                .args(command.split(' '))
                .arg("--help")
                .assert()
                .success()
                .get_output()
                .stdout
                .clone(),
        )
        .expect("help is UTF-8");
        let flags = help.lines().filter_map(|line| {
            let flag = line.trim_start().trim_start_matches("-h, ");
            let name = flag.strip_prefix("--")?;
            Some(name.split([' ', '<']).next().unwrap_or(name).to_owned())
        });
        for flag in flags.filter(|flag| !global.contains(&flag.as_str())) {
            if !entry.contains(&format!("--{flag}")) {
                missing.push(format!("{command} --{flag}"));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "the README's synopsis entry for each of these verbs does not name the flag:\n  {}",
        missing.join("\n  ")
    );
}

#[test]
fn the_task_update_synopsis_names_exactly_the_flags_that_verb_takes() {
    // Held both ways, because the entry spells out every flag the verb has: one `--help`
    // reports that the entry leaves out, and one the entry names that the verb does not take,
    // each fail here. Global flags belong to no one verb and are left to the check above.
    let global = [
        "set",
        "page-size",
        "default-sources",
        "output",
        "json",
        "interactive",
        "no-interactive",
        "help",
    ];
    let entry = synopsis(&readme(), "task update")
        .expect("the README has a synopsis entry for `task update`");
    let help = String::from_utf8(
        onetaskgraph()
            .args(["task", "update", "--help"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone(),
    )
    .expect("help is UTF-8");
    let mut reported: Vec<String> = help
        .lines()
        .filter_map(|line| {
            let flag = line.trim_start().trim_start_matches("-h, ");
            let name = flag.strip_prefix("--")?;
            Some(name.split([' ', '<']).next().unwrap_or(name).to_owned())
        })
        .filter(|flag| !global.contains(&flag.as_str()))
        .collect();
    let mut named: Vec<String> = entry
        .split("--")
        .skip(1)
        .map(|rest| {
            rest.split(|c: char| !(c.is_ascii_lowercase() || c == '-'))
                .next()
                .unwrap_or_default()
                .to_owned()
        })
        .collect();
    reported.sort();
    reported.dedup();
    named.sort();
    named.dedup();
    assert_eq!(
        named, reported,
        "the README's `task update` entry and `task update --help` name different flags"
    );
}

/// The object of every ```json block of `text` that parses as one.
fn json_objects(text: &str) -> Vec<serde_json::Map<String, serde_json::Value>> {
    text.split("```json\n")
        .skip(1)
        .filter_map(|block| block.split("\n```").next())
        .filter_map(|block| serde_json::from_str::<serde_json::Value>(block).ok())
        .filter_map(|value| value.as_object().cloned())
        .collect()
}

#[test]
fn the_documents_spell_the_provenance_entry_and_the_loader_document_as_the_binary_reads_them() {
    // The README and `docs/metadata.md` each show the two shapes a caller writes against: the
    // `onetaskgraph.template` entry an item records, and the loader document a caller supplies.
    // Their keys are held to the emitted `TemplateProvenance` root and to the keys the loader
    // document is read for, so neither example can drift from what the binary does.
    let bundle: serde_json::Value = serde_json::from_slice(
        &onetaskgraph()
            .arg("schema")
            .assert()
            .success()
            .get_output()
            .stdout,
    )
    .expect("schema output is valid JSON");
    let provenance: std::collections::BTreeSet<String> =
        bundle["roots"]["TemplateProvenance"]["properties"]
            .as_object()
            .expect("the provenance root has properties")
            .keys()
            .cloned()
            .collect();
    // llmlint: ignore-block[tests_mirror_real_usage] This is a drift gate between the documents and the one place the loader document's keys are spelled; the CLI reads a loader document and ignores every other key, so no invocation can enumerate the keys it reads, and the provenance half already comes from the binary's own `schema` output.
    let loader: std::collections::BTreeSet<String> = onetaskgraph_core::LoaderDocument::KEYS
        .iter()
        .map(|key| (*key).to_owned())
        .collect();
    // llmlint: ignore-end[tests_mirror_real_usage]
    let metadata_doc = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/metadata.md"),
    )
    .expect("docs/metadata.md is readable");

    for (name, text, owes_provenance) in [
        ("README.md", readme(), false),
        ("docs/metadata.md", metadata_doc, true),
    ] {
        let objects = json_objects(&text);
        let keys = |object: &serde_json::Map<String, serde_json::Value>| {
            object
                .keys()
                .cloned()
                .collect::<std::collections::BTreeSet<String>>()
        };
        let loaders: Vec<_> = objects.iter().filter(|o| o.contains_key("entry")).collect();
        assert!(!loaders.is_empty(), "{name} shows no loader document");
        for object in loaders {
            assert_eq!(keys(object), loader, "{name}'s loader document example");
        }
        let entries: Vec<_> = objects
            .iter()
            .filter(|o| o.contains_key("answers_digest"))
            .collect();
        assert_eq!(
            !entries.is_empty(),
            owes_provenance,
            "{name}'s provenance example"
        );
        for object in entries {
            assert_eq!(keys(object), provenance, "{name}'s provenance example");
        }
    }
}

#[test]
fn the_reserved_key_inventory_names_exactly_the_keys_the_code_spells() {
    // `docs/metadata.md` lists every key of the reserved `onetaskgraph.` namespace, and counts
    // them in words. Both are held to the constants each key is spelled once as, so a key added
    // in code and not in the document — or the reverse — fails here.
    use onetaskgraph_plugin_api::{DependencyEdge, ItemKind, MetadataKey, Repository, TaskRef};
    // llmlint: ignore-block[tests_mirror_real_usage] This is a drift gate between `docs/metadata.md` and the constants each reserved key is spelled once as; the CLI refuses the whole `onetaskgraph.` namespace by prefix, so no invocation enumerates the keys, and reading the constants is what makes a key added in code without the document fail.
    let spelled: std::collections::BTreeSet<&str> = [
        Repository::METADATA_KEY,
        DependencyEdge::RECORDED_KEY,
        TaskRef::DELIVERS_KEY,
        TaskRef::DELIVERED_BY_KEY,
        ItemKind::METADATA_KEY,
        MetadataKey::TEMPLATE_KEY,
        MetadataKey::COPIES_KEY,
        onetaskgraph_core::GlobalId::ORIGIN_KEY,
    ]
    .into_iter()
    .collect();
    // llmlint: ignore-end[tests_mirror_real_usage]
    let document = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/metadata.md"),
    )
    .expect("docs/metadata.md is readable");
    let start = document
        .find("- `onetaskgraph.` belongs to this product.")
        .expect("the reserved-namespace bullet");
    let bullet = &document[start..start + document[start..].find("\n- `").unwrap()];
    let listed: std::collections::BTreeSet<&str> = bullet
        .split('`')
        .skip(1)
        .step_by(2)
        .filter(|quoted| {
            quoted.starts_with("onetaskgraph.") && quoted.len() > "onetaskgraph.".len()
        })
        .collect();
    assert_eq!(listed, spelled, "the keys docs/metadata.md lists");
    let words = [
        "", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine",
    ];
    assert!(
        bullet.contains(&format!("defines exactly {} keys", words[spelled.len()])),
        "docs/metadata.md counts {} keys in words:\n{bullet}",
        spelled.len()
    );
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

/// Command lines the README spells in full, each beside the argument — if any — whose values
/// it spells inline as `a|b|c`.
///
/// A word of [`SURFACE`] found anywhere in the README says nothing about whether the verb it
/// belongs to is documented: `set` and `fields` appear in its prose regardless. So each of
/// these is held to the README carrying `onetaskgraph <verb>` itself, and its values to the
/// exact list, in order, that the binary's help reports — a value added to the enum and not
/// the README, or dropped from one and not the other, fails here.
const README_COMMAND_LINES: &[(&[&str], Option<&str>)] = &[
    (&["sources", "fields"], None),
    (&["task", "list"], Some("--priority")),
    (&["task", "priority", "set"], Some("<PRIORITY>")),
    (&["task", "content", "set"], None),
    (&["template", "variables"], None),
    (&["template", "render"], None),
];

#[test]
fn the_readme_spells_each_command_line_and_its_values_as_the_help_does() {
    let readme = readme();
    let mut drift = Vec::new();
    for (verb, values) in README_COMMAND_LINES {
        let help = String::from_utf8_lossy(
            &onetaskgraph()
                .arg("help")
                .args(*verb)
                .assert()
                .success()
                .get_output()
                .stdout,
        )
        .into_owned();
        let command = format!("onetaskgraph {}", verb.join(" "));
        let Some(line) = readme
            .lines()
            .map(str::trim_start)
            .find(|line| line.starts_with(&command))
        else {
            drift.push(format!("no `{command}` command line"));
            continue;
        };
        let Some(argument) = values else { continue };
        let spelled = possible_values(&help, argument).join("|");
        // A flag's values sit on whichever continuation line holds the flag; a positional
        // argument's sit on the command line itself.
        let documented = if argument.starts_with("--") {
            readme.contains(&format!("{argument} {spelled}"))
        } else {
            line.contains(&spelled)
        };
        if !documented {
            drift.push(format!(
                "`{command}` does not spell {argument} as {spelled}"
            ));
        }
    }
    assert!(
        drift.is_empty(),
        "the README's command lines disagree with `--help`:\n  {}",
        drift.join("\n  ")
    );
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
    (&["help", "task", "list"], "--priority", "Priority", true),
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
    // `StatusArg`, `PriorityArg`, `FieldsArg`, `DirectionArg` and `KindArg` each mirror an
    // enum of the contract or the engine, and they exist so that deriving clap's `ValueEnum`
    // does not put clap into the plugin contract's dependencies for the sake of five flags. A
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
                "--status" | "--priority" => vec!["task", "list"],
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

#[test]
fn the_readme_names_every_word_of_the_copy_reports_via_and_link() {
    // The README lists `via` as one field of five words and `link` as one of three, because
    // a reader of a copy report sees one field rather than the two types the SDKs generate
    // for `via`. That list is held here to the words the binary's own schema emits for both,
    // so a word added to either vocabulary, or dropped from it, fails until the README says
    // so too.
    let output = onetaskgraph()
        .arg("schema")
        .assert()
        .success()
        .get_output()
        .clone();
    let bundle: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("schema output is valid JSON");
    let action = &bundle["roots"]["CopyAction"];
    let words = |schema: &serde_json::Value| -> Vec<String> {
        schema["oneOf"]
            .as_array()
            .expect("a vocabulary is a oneOf of words")
            .iter()
            .map(|word| word["const"].as_str().expect("a word").to_owned())
            .collect()
    };
    let mut via = words(&action["$defs"]["CopyVia"]);
    via.extend(words(&action["$defs"]["NoCounterpart"]));
    let link = words(&action["$defs"]["CopyLink"]);
    let actions: Vec<String> = action["oneOf"]
        .as_array()
        .expect("the actions are a oneOf")
        .iter()
        .map(|variant| {
            variant["properties"]["action"]["const"]
                .as_str()
                .expect("an action")
                .to_owned()
        })
        .collect();
    assert_eq!(via.len(), 5, "{via:?}");
    assert_eq!(link.len(), 3, "{link:?}");

    let readme = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../README.md"),
    )
    .expect("README.md is readable");
    let start = readme
        .find("`via` is one field with five words")
        .expect("the README's paragraph on `via` and `link`");
    let paragraph = &readme[start..start + readme[start..].find("\n<!--").unwrap()];
    let named: std::collections::BTreeSet<&str> = paragraph.split('`').skip(1).step_by(2).collect();
    // Every word of both vocabularies, the four actions the paragraph names them on, and the
    // five names it uses for the fields, the flag and the three generated types — and
    // nothing else, so a word this schema no longer emits cannot linger there either.
    let mut expected: std::collections::BTreeSet<&str> = via
        .iter()
        .chain(&link)
        .chain(&actions)
        .map(String::as_str)
        .collect();
    expected.extend([
        "via",
        "link",
        "--match-by",
        "CopyVia",
        "NoCounterpart",
        "CopyLink",
    ]);
    assert_eq!(named, expected, "the README's paragraph:\n{paragraph}");
}
