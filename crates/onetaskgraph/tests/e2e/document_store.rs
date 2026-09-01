//! The document copy round trip, against a destination that outlives the invocation.
//!
//! Every other copy journey reads its destination back through a later command, which is
//! only possible because a folder of Markdown is still there afterwards. Documents had no
//! such destination: `local-md` declares it holds none, and the in-memory source's work
//! dies with the process that held it — so a `document copy` typed at a shell could be
//! observed only through the report it printed, and "the document really landed carrying
//! every field" was proven one layer down, as a library call.
//!
//! This closes that. The destination is `onetaskgraph-document-store`, a peer that keeps
//! its documents in a JSON file and speaks `docs/plugin-protocol.md` over a real pipe to a
//! real second process — so the copy in one invocation is read back by the *next*
//! invocation, through the same command line a user types.

use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{ROWS, Row, document, empty_folder, qualified};

/// The source every journey here copies out of.
const SOURCE: &str = "work";

/// The persistent destination they copy into.
const STORE: &str = "store";

fn run(sandbox: &Sandbox, arguments: &[&str]) -> Output {
    sandbox
        .command()
        .args(arguments)
        .assert()
        .get_output()
        .clone()
}

/// Standard output of a run that had to succeed, quoting stderr when it did not.
fn ok(sandbox: &Sandbox, arguments: &[&str]) -> String {
    let output = run(sandbox, arguments);
    assert_eq!(
        output.status.code(),
        Some(0),
        "`onetaskgraph {}` exited {:?}\n{}",
        arguments.join(" "),
        output.status.code(),
        stderr(&output)
    );
    stdout(&output)
}

/// Standard error of a run that had to fail with `code`.
fn refused(sandbox: &Sandbox, arguments: &[&str], code: i32) -> String {
    let output = run(sandbox, arguments);
    assert_eq!(
        output.status.code(),
        Some(code),
        "`onetaskgraph {}` was expected to exit {code}\n{}{}",
        arguments.join(" "),
        stdout(&output),
        stderr(&output)
    );
    stderr(&output)
}

/// One item of a `--json` copy report, as a comparable triple.
fn reported(rendered: &str) -> Vec<(String, Value, String)> {
    let report: Value = serde_json::from_str(rendered).expect("a copy emits JSON");
    report["items"]
        .as_array()
        .expect("a copy report carries items")
        .iter()
        .map(|item| {
            (
                item["source"].as_str().expect("a source id").to_owned(),
                item["destination"].clone(),
                item["action"].as_str().expect("an action").to_owned(),
            )
        })
        .collect()
}

/// One item of a `<verb> show --json` response.
fn shown(sandbox: &Sandbox, verb: &str, id: &str) -> Value {
    let response: Value =
        serde_json::from_str(&ok(sandbox, &[verb, "show", id, "--json"])).expect("show emits JSON");
    response["items"][0]["item"].clone()
}

/// The persistent peer, configured over `store`.
fn store_at(store: &Path, log: Option<&Path>, documents: &str) -> Value {
    let mut settings = json!({"store": store, "documents": documents});
    if let Some(log) = log {
        settings["log"] = json!(log);
    }
    json!({
        "plugin": "subprocess",
        "config": {
            "command": env!("CARGO_BIN_EXE_onetaskgraph-document-store"),
            "settings": settings,
        },
    })
}

/// The one document this suite copies, with a caller-defined key of every JSON type.
///
/// The types matter as much as the values: a destination that stringified a number, or
/// flattened a nested object, would still round-trip something recognisable, and the whole
/// promise of caller-defined metadata is that it comes back out as it went in.
fn source_document(overrides: Value) -> Value {
    let mut held = json!({
        "id": "D-1",
        "title": "Alpha design",
        "content": "the engine core, reviewed",
        "project": "P-1",
        "labels": [{"id": "L-1", "name": "spec"}],
        "location": {"url": "https://example.invalid/D-1"},
        "metadata": {
            "onepipeline.turn_budget": 12,
            "caller.flags": [true, null],
            "caller.shape": {"nested": {"depth": 2}},
            "caller.ratio": 1.5,
            "caller.note": "a string",
        },
        "repositories": ["github.com/nickderobertis/onetaskgraph"],
    });
    for (key, value) in overrides.as_object().expect("an object of overrides") {
        held[key] = value.clone();
    }
    held
}

/// The configuration these journeys run against: the source, and the persistent store.
fn planted(source: Value, store: &Path) -> String {
    document(&json!({
        SOURCE: {"plugin": "in-memory", "config": {
            "capabilities": {"documents": "native"},
            "projects": [{"id": "P-1", "title": "Engine", "content": null,
                          "status": {"category": "in-progress", "name": "Doing"}, "labels": []}],
            "documents": [source],
        }},
        STORE: store_at(store, None, "native"),
    }))
}

/// Where the destination keeps its documents, under this run's own sandbox.
fn store_path(sandbox: &Sandbox) -> PathBuf {
    sandbox.subdirectory("store").join("documents.json")
}

#[test]
fn a_document_copy_creates_at_a_persistent_destination_and_a_second_copy_updates_that_one() {
    let sandbox = Sandbox::new();
    let store = store_path(&sandbox);
    sandbox.project_document(&planted(source_document(json!({})), &store));

    // One invocation writes.
    let created = reported(&ok(
        &sandbox,
        &[
            "document",
            "copy",
            &qualified(SOURCE, "D-1"),
            "--to",
            STORE,
            "--json",
        ],
    ));
    assert_eq!(
        created,
        vec![(
            qualified(SOURCE, "D-1"),
            json!(qualified(STORE, "D-1")),
            "created".to_owned()
        )]
    );

    // A *different* invocation reads it back — which is the whole point of this file, and
    // which no in-memory destination could have answered.
    let landed = shown(&sandbox, "document", &qualified(STORE, "D-1"));
    assert_eq!(landed["title"], json!("Alpha design"));
    assert_eq!(landed["content"], json!("the engine core, reviewed"));
    assert_eq!(landed["labels"][0]["name"], json!("spec"));
    assert_eq!(landed["project"], json!("P-1"));
    assert_eq!(
        landed["repositories"],
        json!(["github.com/nickderobertis/onetaskgraph"])
    );
    // Every caller-defined key, with its JSON type intact — a number as a number, a list
    // holding two different types, a nested object, a float and a string.
    assert_eq!(landed["metadata"]["onepipeline.turn_budget"], json!(12));
    assert_eq!(landed["metadata"]["caller.flags"], json!([true, null]));
    assert_eq!(
        landed["metadata"]["caller.shape"],
        json!({"nested": {"depth": 2}})
    );
    assert_eq!(landed["metadata"]["caller.ratio"], json!(1.5));
    assert_eq!(landed["metadata"]["caller.note"], json!("a string"));
    // And the origin the copy recorded, which is how the second copy finds this one.
    assert_eq!(
        landed["metadata"]["onetaskgraph.origin"],
        json!(qualified(SOURCE, "D-1"))
    );
    // Where the *source* holds a document says nothing about where the destination does,
    // so the location is the destination's own and this copy did not write one.
    assert_eq!(landed["location"], json!(null));

    // Now the source changes, the way an author editing a document changes it.
    sandbox.project_document(&planted(
        source_document(json!({
            "title": "Alpha design, revised",
            "metadata": {
                "onepipeline.turn_budget": 20,
                "caller.flags": [true, null],
                "caller.shape": {"nested": {"depth": 2}},
                "caller.ratio": 1.5,
                "caller.note": "a string",
            },
        })),
        &store,
    ));

    let updated = reported(&ok(
        &sandbox,
        &[
            "document",
            "copy",
            &qualified(SOURCE, "D-1"),
            "--to",
            STORE,
            "--json",
        ],
    ));
    assert_eq!(
        updated,
        vec![(
            qualified(SOURCE, "D-1"),
            json!(qualified(STORE, "D-1")),
            "updated".to_owned()
        )],
        "a second copy updates the document already there"
    );

    // Exactly one where there was one before, in a later invocation still.
    let listed = ok(&sandbox, &["document", "list", "--source", STORE]);
    assert_eq!(
        listed
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count(),
        1,
        "the second copy updated rather than adding a duplicate:\n{listed}"
    );

    let revised = shown(&sandbox, "document", &qualified(STORE, "D-1"));
    assert_eq!(revised["title"], json!("Alpha design, revised"));
    assert_eq!(revised["metadata"]["onepipeline.turn_budget"], json!(20));
    // Every field the edit did not touch is byte-for-byte what it was.
    for untouched in ["content", "labels", "project", "repositories", "location"] {
        assert_eq!(
            revised[untouched], landed[untouched],
            "the update rewrote {untouched}, which the edit did not touch"
        );
    }
    for untouched in [
        "caller.flags",
        "caller.shape",
        "caller.ratio",
        "caller.note",
        "onetaskgraph.origin",
    ] {
        assert_eq!(
            revised["metadata"][untouched], landed["metadata"][untouched],
            "the update rewrote the metadata key {untouched}, which the edit did not touch"
        );
    }
}

#[test]
fn a_document_copy_into_a_destination_with_no_documents_reads_nothing_from_the_source_first() {
    // The refusal is answered from the declaration the handshake carried, so the proof it
    // owes is not only its own wording: the source has to have been asked *nothing*. This
    // source records every method it is asked for, and afterwards the record holds the
    // handshake and nothing else.
    let sandbox = Sandbox::new();
    let store = store_path(&sandbox);
    let log = sandbox.subdirectory("store").join("asked.log");
    std::fs::write(
        &store,
        json!({"documents": [source_document(json!({}))]}).to_string(),
    )
    .expect("the source's own store");

    sandbox.project_document(&document(&json!({
        SOURCE: store_at(&store, Some(&log), "native"),
        "notes": {"plugin": "local-md", "config": empty_folder(&sandbox, "notes")},
    })));

    let complaint = refused(
        &sandbox,
        &[
            "document",
            "copy",
            &qualified(SOURCE, "D-1"),
            "--to",
            "notes",
        ],
        1,
    );
    assert!(
        complaint.contains("notes")
            && complaint.contains("local-md")
            && complaint.contains("has no documents"),
        "the refusal names the destination and its plugin:\n{complaint}"
    );

    let asked: Vec<String> = std::fs::read_to_string(&log)
        .expect("the source recorded what it was asked")
        .lines()
        .map(str::to_owned)
        .collect();
    assert_eq!(
        asked,
        ["initialize"],
        "the copy was refused before the source was read: it was asked {asked:?}"
    );

    // And nothing was written at the destination either.
    let notes = sandbox.project().join("notes");
    let written: Vec<PathBuf> = walk(&notes);
    assert!(
        written.is_empty(),
        "a refused copy writes nothing: {written:?}"
    );
}

/// This row's own source, with the persistent store beside it as the destination.
fn row_with_store(row: &Row, sandbox: &Sandbox, store: &Path) -> String {
    document(&json!({
        SOURCE: {"plugin": row.plugin, "config": (row.fixture.block)(sandbox)},
        STORE: store_at(store, None, "native"),
    }))
}

#[test]
fn every_document_bearing_row_copies_into_a_persistent_destination_and_is_matched_not_duplicated() {
    // The journey above proves the round trip in full against one source. This proves the
    // half that has to hold for *every* source kind: a document copied out of it lands at a
    // destination that is still there afterwards, reads back as the source reported it, and
    // is matched rather than duplicated when the same copy is run again.
    for row in ROWS
        .iter()
        .filter(|row| row.declared().documents.is_native())
    {
        let sandbox = Sandbox::new();
        let store = store_path(&sandbox);
        sandbox.project_document(&row_with_store(row, &sandbox, &store));

        let created = reported(&ok(
            &sandbox,
            &[
                "document",
                "copy",
                &qualified(SOURCE, "D-1"),
                "--to",
                STORE,
                "--json",
            ],
        ));
        assert_eq!(
            created,
            vec![(
                qualified(SOURCE, "D-1"),
                json!(qualified(STORE, "D-1")),
                "created".to_owned()
            )],
            "{}",
            row.name
        );

        // A later invocation, against the file the first one wrote.
        let landed = shown(&sandbox, "document", &qualified(STORE, "D-1"));
        let source = shown(&sandbox, "document", &qualified(SOURCE, "D-1"));
        for field in ["title", "content", "labels", "project", "repositories"] {
            assert_eq!(
                landed[field], source[field],
                "{}: the destination holds {field} as the source reported it",
                row.name
            );
        }
        for key in ["onepipeline.turn_budget", "caller.flags"] {
            assert_eq!(
                landed["metadata"][key], source["metadata"][key],
                "{}: the destination holds the metadata key {key} with its JSON type intact",
                row.name
            );
        }

        let again = reported(&ok(
            &sandbox,
            &[
                "document",
                "copy",
                &qualified(SOURCE, "D-1"),
                "--to",
                STORE,
                "--json",
            ],
        ));
        assert_eq!(
            again[0].1,
            json!(qualified(STORE, "D-1")),
            "{}: the second copy found the first one's document",
            row.name
        );
        assert!(
            matches!(again[0].2.as_str(), "updated" | "unchanged"),
            "{}: a second copy is not a second create: {:?}",
            row.name,
            again[0]
        );
        let listed = ok(&sandbox, &["document", "list", "--source", STORE]);
        assert_eq!(
            listed
                .lines()
                .filter(|line| !line.trim().is_empty())
                .count(),
            1,
            "{}: exactly one where there was one before:\n{listed}",
            row.name
        );
    }
}

/// Every file under `root`, however deep.
fn walk(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else {
                found.push(path);
            }
        }
    }
    found
}
