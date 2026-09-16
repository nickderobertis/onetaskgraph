//! `task`, `project` and `document metadata set`, driven the way a user drives them.
//!
//! Every journey spawns the compiled binary and asserts on its exit code, stdout and stderr,
//! and on what the store holds afterwards: a folder of Markdown, read back byte for byte and
//! through a later invocation, over the in-process boundary and the stdio host alike. The
//! refusals a mistaken invocation owes are proven against the Python peer of
//! `document_store.py`, which records every method it is asked for — so "refused before any
//! source was asked" is read off a file that peer never wrote.

use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::{Value, json};

use crate::common::{SOURCE_BOUNDARIES, Sandbox, stderr, stdout};
use crate::document_store::store_at;
use crate::fixtures::document;
use crate::machine::{bundle, validates};

/// The three verbs, each with the one record of its kind every journey here writes.
const VERBS: [(&str, &str); 3] = [
    ("task", "tasks/T-1.md"),
    ("project", "projects/P-1.md"),
    ("document", "documents/D-1.md"),
];

/// What each record's file holds before anything is set: flow JSON entries and a plain one,
/// a block the verb has to add to without touching.
fn held(kind: &str) -> String {
    let status = if kind == "document" {
        ""
    } else {
        "status: todo\n"
    };
    format!(
        "---\ntitle: {kind} one\n{status}metadata:\n  \"onepipeline.review\": {{\"state\":\"pending\"}}\n  myapp.note: plain words\n---\nThe body.\n"
    )
}

fn run(sandbox: &Sandbox, arguments: &[&str]) -> Output {
    sandbox
        .command()
        .args(arguments)
        .assert()
        .get_output()
        .clone()
}

/// A run that had to exit `code`, quoting what it said when it did not.
fn exits(sandbox: &Sandbox, arguments: &[&str], code: i32) -> Output {
    let output = run(sandbox, arguments);
    assert_eq!(
        output.status.code(),
        Some(code),
        "`onetaskgraph {}` exited {:?}\nstdout:\n{}\nstderr:\n{}",
        arguments.join(" "),
        output.status.code(),
        stdout(&output),
        stderr(&output)
    );
    output
}

fn parsed(output: &Output) -> Value {
    serde_json::from_str(&stdout(output))
        .unwrap_or_else(|error| panic!("not one JSON document ({error}):\n{}", stdout(output)))
}

/// A folder of Markdown holding one task, one project and one document.
fn folder(sandbox: &Sandbox) -> PathBuf {
    let root = sandbox.subdirectory("work");
    for (kind, relative) in VERBS {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().expect("a folder")).expect("a folder");
        std::fs::write(&path, held(kind)).expect("a record");
    }
    root
}

fn read(root: &Path, relative: &str) -> String {
    std::fs::read_to_string(root.join(relative)).expect("the record reads")
}

#[test]
fn one_key_of_a_task_a_project_and_a_document_is_set_and_nothing_else_moves() {
    for boundary in SOURCE_BOUNDARIES {
        let sandbox = Sandbox::new();
        let root = folder(&sandbox);
        sandbox.project_document(&document(&json!({
            "work": boundary.source("local-md", json!({"root": root})),
        })));
        let schema = bundle(&sandbox);

        for (kind, relative) in VERBS {
            let who = format!("{boundary:?} {kind}");
            let id = format!(
                "work:{}",
                if kind == "task" {
                    "T-1"
                } else if kind == "project" {
                    "P-1"
                } else {
                    "D-1"
                }
            );
            let before =
                parsed(&exits(&sandbox, &[kind, "show", &id, "--json"], 0))["items"][0]["item"]
                    .clone();

            // A new key is added to the block, on one line, and every other byte stays.
            let answer = parsed(&exits(
                &sandbox,
                &[
                    kind,
                    "metadata",
                    "set",
                    &id,
                    "myapp.review",
                    r#"{"approved": true, "by": ["nick"]}"#,
                    "--json",
                ],
                0,
            ));
            validates(
                &schema,
                "MetadataSet",
                &answer,
                &format!("{who} metadata set"),
            );
            let file = std::fs::canonicalize(root.join(relative)).expect("the record resolves");
            assert_eq!(
                answer,
                json!({"id": id, "key": "myapp.review", "value": {"approved": true, "by": ["nick"]},
                       "location": {"path": file}}),
                "{who}"
            );
            assert_eq!(
                read(&root, relative),
                held(kind).replace(
                    "  myapp.note: plain words\n",
                    "  myapp.note: plain words\n  \"myapp.review\": {\"approved\":true,\"by\":[\"nick\"]}\n"
                ),
                "{who}: the file changed only by the one entry added"
            );

            // An existing key is replaced where it is, and a value may begin with a hyphen.
            let replaced = parsed(&exits(
                &sandbox,
                &[
                    kind,
                    "metadata",
                    "set",
                    &id,
                    "onepipeline.review",
                    "-1.5",
                    "--json",
                ],
                0,
            ));
            assert_eq!(replaced["value"], json!(-1.5), "{who}");
            assert_eq!(
                read(&root, relative),
                held(kind)
                    .replace("  \"onepipeline.review\": {\"state\":\"pending\"}\n", "  \"onepipeline.review\": -1.5\n")
                    .replace(
                        "  myapp.note: plain words\n",
                        "  myapp.note: plain words\n  \"myapp.review\": {\"approved\":true,\"by\":[\"nick\"]}\n"
                    ),
                "{who}: the file changed only by the one entry replaced"
            );

            // A later invocation reads the record back with both keys and every other field
            // exactly as it was.
            let after =
                parsed(&exits(&sandbox, &[kind, "show", &id, "--json"], 0))["items"][0]["item"]
                    .clone();
            let mut expected = before.clone();
            expected["metadata"]["myapp.review"] = json!({"approved": true, "by": ["nick"]});
            expected["metadata"]["onepipeline.review"] = json!(-1.5);
            assert_eq!(after, expected, "{who}");

            // The held value again writes nothing, and says so in words a person reads.
            let unchanged = read(&root, relative);
            let modified = std::fs::metadata(root.join(relative))
                .and_then(|metadata| metadata.modified())
                .expect("a modification time");
            let text = stdout(&exits(
                &sandbox,
                &[
                    kind,
                    "metadata",
                    "set",
                    &id,
                    "myapp.note",
                    "\"plain words\"",
                ],
                0,
            ));
            assert!(
                text.contains("id:")
                    && text.contains(&id)
                    && text.contains("key:")
                    && text.contains("myapp.note")
                    && text.contains("value:")
                    && text.contains("\"plain words\"")
                    && text.contains("location:")
                    && text.contains("path "),
                "{who}: {text}"
            );
            assert_eq!(read(&root, relative), unchanged, "{who}");
            assert_eq!(
                std::fs::metadata(root.join(relative))
                    .and_then(|metadata| metadata.modified())
                    .expect("a modification time"),
                modified,
                "{who}: setting the held value is no write at all"
            );
        }
    }
}

#[test]
fn every_metadata_set_refusal_is_a_failure_document_naming_its_cause() {
    let sandbox = Sandbox::new();
    let root = folder(&sandbox);
    let store = sandbox.project().join("store.json");
    std::fs::write(&store, r#"{"documents": []}"#).expect("a store");
    let log = sandbox.project().join("asked.log");
    sandbox.project_document(&document(&json!({
        "work": {"plugin": "local-md", "config": {"root": root}},
        "frozen": {"plugin": "in-memory", "config": {"capabilities": {"writes": "unsupported", "documents": "native"}}},
        "store": store_at(&store, Some(&log), "native"),
    })));
    let schema = bundle(&sandbox);

    for (kind, _) in VERBS {
        for (id, failure_kind, message) in [
            (
                "nowhere:X-1".to_owned(),
                "unknown-source",
                "no source named \"nowhere\" is configured".to_owned(),
            ),
            (
                "frozen:X-1".to_owned(),
                "not-writable",
                format!(
                    "source frozen cannot write a {kind}'s metadata: its plugin is in-memory, which has no write side"
                ),
            ),
            (
                "work:X-404".to_owned(),
                "no-such-item",
                format!("no {kind} with the id work:X-404"),
            ),
        ] {
            let who = format!("{kind} metadata set {id}");
            let output = exits(
                &sandbox,
                &[
                    kind,
                    "metadata",
                    "set",
                    &id,
                    "myapp.review",
                    "true",
                    "--json",
                ],
                1,
            );
            let failed = parsed(&output);
            validates(&schema, "FailureDocument", &failed, &who);
            assert_eq!(failed["failure"]["kind"], failure_kind, "{who}: {failed:#}");
            let said = failed["failure"]["message"].as_str().expect("a message");
            assert!(
                said.contains(&message) && said.contains("next:"),
                "{who}: {said}"
            );
            assert!(stderr(&output).contains(&message), "{who}");
        }
    }
    for (kind, relative) in VERBS {
        assert_eq!(
            read(&root, relative),
            held(kind),
            "a refusal writes nothing"
        );
    }

    // A stdio plugin whose handshake does not declare the metadata writes is refused in the
    // contract's own words, and is never sent the method.
    let output = exits(
        &sandbox,
        &[
            "document",
            "metadata",
            "set",
            "store:D-1",
            "myapp.review",
            "true",
            "--json",
        ],
        1,
    );
    let failed = parsed(&output);
    validates(&schema, "FailureDocument", &failed, "an undeclared plugin");
    assert_eq!(failed["failure"]["kind"], "refused", "{failed:#}");
    assert!(
        failed["failure"]["message"]
            .as_str()
            .is_some_and(|said| said.contains(
                "the document-store plugin cannot write a document's metadata on its own"
            )),
        "{failed:#}"
    );
    // Every invocation starts every configured source, so the peer's record holds one
    // handshake per run above — and nothing else.
    let asked = std::fs::read_to_string(&log).expect("the peer was started");
    assert!(
        !asked.is_empty() && asked.lines().all(|method| method == "initialize"),
        "the peer was asked for more than its handshake: {asked:?}"
    );
}

#[test]
fn a_mistaken_id_key_or_value_is_refused_before_any_source_is_asked() {
    let sandbox = Sandbox::new();
    let store = sandbox.project().join("store.json");
    std::fs::write(
        &store,
        r#"{"documents": [{"id": "D-1", "title": "Design", "content": null, "labels": []}]}"#,
    )
    .expect("a store");
    let log = sandbox.project().join("asked.log");
    sandbox.project_document(&document(&json!({
        "store": store_at(&store, Some(&log), "native"),
    })));

    for (kind, _) in VERBS {
        for (id, key, value, why) in [
            (
                "D-1",
                "myapp.review",
                "true",
                "\"D-1\" is not a qualified id",
            ),
            ("store:D-1", "review", "true", "has no namespace"),
            ("store:D-1", "myapp..review", "true", "has an empty segment"),
            ("store:D-1", ".review", "true", "has an empty segment"),
            (
                "store:D-1",
                "onetaskgraph.origin",
                "true",
                "which this product owns",
            ),
            (
                "store:D-1",
                "onetaskgraph.delivers",
                "[]",
                "which this product owns",
            ),
            ("store:D-1", "myapp.review", "yes", "is not JSON"),
            ("store:D-1", "myapp.review", "2026-01-01", "is not JSON"),
            (
                "store:D-1",
                "myapp.review",
                "{\"a\": 1} {\"b\": 2}",
                "is not JSON",
            ),
        ] {
            let who = format!("{kind} metadata set {id} {key} {value}");
            let output = exits(
                &sandbox,
                &[kind, "metadata", "set", id, key, value, "--json"],
                1,
            );
            let failed = parsed(&output);
            let said = failed["failure"]["message"].as_str().expect("a message");
            assert!(
                said.contains(why) && said.contains("next:"),
                "{who}: {said}"
            );
            assert!(stderr(&output).contains(why), "{who}");
            assert!(
                !log.exists(),
                "{who}: the source was started, and asked for {:?}",
                std::fs::read_to_string(&log)
            );
        }
    }

    // The peer really does record being asked, so its silence above is evidence. It is a
    // Python peer writing in text mode, so on Windows each record ends `\r\n`: read lines.
    exits(&sandbox, &["document", "show", "store:D-1", "--json"], 0);
    let asked = std::fs::read_to_string(&log).expect("the peer was started");
    assert_eq!(
        asked.lines().next(),
        Some("initialize"),
        "the peer's first record is its handshake: {asked:?}"
    );
}

/// A write that fails after its staging file was created — here because the file-size limit
/// the binary runs under stops the staging write partway — is a failure document, and leaves
/// the record byte for byte and no staging file beside it.
///
/// The limit is set by the shell that `exec`s the binary, with the signal a file-size overrun
/// raises ignored, so the write sees `EFBIG` and the binary reports it. Every record here is
/// far below the limit and the value set is far above it, so only the staging write can meet
/// it. Unix only: Windows has no per-process file-size limit to set.
///
/// The limit reaches every file the process writes, so under a coverage run it would also
/// truncate the profile LLVM's runtime writes on exit, and a truncated profile in the run's
/// own pool fails the whole merge. That profile goes to a directory of its own instead.
#[cfg(unix)]
#[test]
fn a_write_that_fails_after_staging_leaves_the_record_and_no_staging_file() {
    let sandbox = Sandbox::new();
    let root = folder(&sandbox);
    sandbox.project_document(&document(&json!({
        "work": {"plugin": "local-md", "config": {"root": root}},
    })));
    let listed = |root: &Path| {
        let mut names: Vec<PathBuf> = VERBS
            .iter()
            .flat_map(|(_, relative)| {
                let folder = root.join(relative).parent().expect("a folder").to_owned();
                std::fs::read_dir(&folder)
                    .expect("the folder lists")
                    .map(|entry| entry.expect("an entry").path())
                    .collect::<Vec<_>>()
            })
            .collect();
        names.sort();
        names
    };
    let before = listed(&root);
    let oversized = serde_json::to_string(&"x".repeat(64 * 1024)).expect("a JSON string");
    let truncated_profiles = tempfile::tempdir().expect("a directory for truncated profiles");

    for (kind, relative) in VERBS {
        let id = format!(
            "work:{}",
            relative
                .rsplit('/')
                .next()
                .and_then(|name| name.strip_suffix(".md"))
                .expect("a record name")
        );
        let output = std::process::Command::new("sh")
            .args([
                "-c",
                r#"ulimit -f 16 && trap '' XFSZ && exec "$0" "$@""#,
                env!("CARGO_BIN_EXE_onetaskgraph"),
                kind,
                "metadata",
                "set",
                &id,
                "myapp.review",
                &oversized,
                "--json",
            ])
            .current_dir(sandbox.project())
            .env("XDG_CONFIG_HOME", sandbox.config_home())
            .env_remove("HOME")
            .env(
                "LLVM_PROFILE_FILE",
                truncated_profiles.path().join("onetaskgraph-%p.profraw"),
            )
            .output()
            .expect("the shell runs");
        let who = format!("{kind} metadata set {id}");
        assert_eq!(
            output.status.code(),
            Some(1),
            "{who}\nstdout:\n{}\nstderr:\n{}",
            stdout(&output),
            stderr(&output)
        );
        let failed = parsed(&output);
        let said = failed["failure"]["message"].as_str().expect("a message");
        assert!(said.contains("cannot write "), "{who}: {said}");
        assert_eq!(read(&root, relative), held(kind), "{who}");
    }
    assert_eq!(listed(&root), before, "a staging file was left behind");
}
