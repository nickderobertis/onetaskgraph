//! The document copy round trip, against a destination that outlives the invocation.
//!
//! Every other copy journey reads its destination back through a later command, which is
//! only possible because a folder of Markdown is still there afterwards. Documents had no
//! such destination: `local-md` declares it holds none, and the in-memory source's work
//! dies with the process that held it — so a `document copy` typed at a shell could be
//! observed only through the report it printed, and "the document really landed carrying
//! every field" was proven one layer down, as a library call.
//!
//! This closes that. The destination is `document_store.py` beside this file: a peer that
//! keeps its documents in a JSON file and speaks `docs/plugin-protocol.md` over a real pipe
//! to a real second process — so the copy in one invocation is read back by the *next*
//! invocation, through the same command line a user types.
//!
//! That it is Python is deliberate twice over. It keeps a fixture out of the shipped
//! binary, where an unmeasurable spawned program would sit as permanently uncovered lines.
//! And it shares not one line with the engine's own half of the protocol, so these
//! journeys test the seam's actual claim — that a plugin can be written in another
//! language against the protocol document alone — rather than restating the engine's
//! implementation back to itself. `python3` is already what every guard under
//! `workspace:lint` runs on all three platforms, so it costs the gate no new dependency.

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
            "command": interpreter().to_string_lossy(),
            "args": [peer().to_string_lossy()],
            "settings": settings,
        },
    })
}

/// This file's peer, beside it in the source tree.
fn peer() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/e2e/document_store.py")
}

/// The Python interpreter, by absolute path.
///
/// Absolute because the engine spawns a plugin with a cleared environment (§3.1, so a
/// plugin cannot read credentials from its own), which leaves the child with no `PATH` to
/// resolve a bare command against. Found here, where there is still an environment to look
/// in, rather than assumed — and a host without one fails saying so, because `python3` is
/// what every guard under `workspace:lint` already runs.
fn interpreter() -> PathBuf {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let names = if cfg!(windows) {
        ["python3.exe", "python.exe"].as_slice()
    } else {
        ["python3", "python"].as_slice()
    };
    for directory in std::env::split_paths(&path) {
        for name in names {
            let candidate = directory.join(name);
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    panic!(
        "no {names:?} on PATH; this suite's document destination is a Python peer, and \
         python3 is already what every guard under `workspace:lint` runs"
    )
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
fn a_peer_that_cannot_parse_what_it_was_handed_refuses_in_its_own_words() {
    // The recovery path of the seam, driven the way a user meets it: a peer answers a
    // request it cannot parse with `malformed` (§5) rather than dying, so the engine
    // reports one named source as having failed and leaves the rest of the answer intact.
    // Here the peer's own store is the thing it cannot parse — a file somebody replaced.
    let sandbox = Sandbox::new();
    let store = store_path(&sandbox);
    sandbox.project_document(&planted(source_document(json!({})), &store));
    std::fs::create_dir_all(store.parent().expect("the store has a directory"))
        .expect("the store directory");
    std::fs::write(&store, "[\"not a store\"]").expect("the store file");

    // Exit 4: a query that lost a source and was not asked to accept a partial answer.
    let complaint = refused(&sandbox, &["document", "list", "--source", STORE], 4);
    assert!(
        complaint.contains("is not a store"),
        "the peer's own words reach the user:\n{complaint}"
    );

    // The other source still answers, and the run says which one could not.
    let partial = run(
        &sandbox,
        &[
            "document",
            "list",
            "--source",
            SOURCE,
            "--source",
            STORE,
            "--allow-partial",
        ],
    );
    assert_eq!(partial.status.code(), Some(0), "{}", stderr(&partial));
    assert!(
        stdout(&partial).contains(&qualified(SOURCE, "D-1")),
        "one source failing leaves the other's documents intact:\n{}",
        stdout(&partial)
    );
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

/// One conversation with the peer over real pipes: the request lines in, the responses out.
///
/// Spawned directly rather than through the engine, and only here. Every journey above
/// drives the peer the way a user reaches it, because that is what a destination is for —
/// but a *malformed* request is precisely what a correct engine never sends, so a line the
/// peer owes a refusal to has no other way to arrive. Everything else about the seam is
/// the real one: a second process, a real pipe, one JSON object per line, and end-of-file
/// on standard input to close the connection (§1).
fn converse(requests: &[Value]) -> Vec<Value> {
    let mut child = std::process::Command::new(interpreter())
        .arg(peer())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("the peer spawns");
    {
        use std::io::Write as _;
        let mut input = child.stdin.take().expect("the peer's standard input");
        for request in requests {
            writeln!(input, "{request}").expect("a request line reaches the peer");
        }
    }
    let output = child.wait_with_output().expect("the peer exits");
    // A peer that answers a request it cannot parse is a peer that is still *there*: the
    // one failure §5 exists to prevent is the interpreter dying into the pipe, which the
    // engine can only report as a plugin that closed its output.
    assert!(
        output.status.success(),
        "the peer must answer and exit 0 rather than dying: {:?}\n{}",
        output.status.code(),
        stderr(&output)
    );
    stdout(&output)
        .lines()
        .map(|line| serde_json::from_str(line).expect("every response line is JSON"))
        .collect()
}

/// The handshake (§3) that tells the peer where its store is.
fn hello(store: &Path) -> Value {
    json!({
        "id": "0",
        "method": "initialize",
        "params": {
            "protocol_version": 2,
            "source_name": STORE,
            "config": {"store": store},
        },
    })
}

/// A `write_document` creating what `item` describes.
fn creating(id: &str, item: Value) -> Value {
    json!({
        "id": id,
        "method": "write_document",
        "params": {"write": {"target": null, "item": item, "depends_on": []}},
    })
}

/// A `query_documents` over `query`, asking for one full page.
fn querying(id: &str, query: Value) -> Value {
    json!({
        "id": id,
        "method": "query_documents",
        "params": {"query": query, "page": {"cursor": null, "limit": 50}},
    })
}

#[test]
fn the_peer_answers_a_well_formed_document_conversation_over_real_stdio() {
    // The shapes every refusal below is measured against: a handshake, a write, a query
    // carrying each member `DocumentQuery` has, and a read of the document that landed.
    let sandbox = Sandbox::new();
    let store = store_path(&sandbox);
    let answers = converse(&[
        hello(&store),
        creating("1", source_document(json!({}))),
        querying(
            "2",
            json!({
                "text": {"terms": "alpha", "fields": "title-or-content"},
                "labels": {"any_of": ["spec"], "all_of": [], "none_of": ["wontfix"]},
                "project": {"is": "P-1"},
            }),
        ),
        json!({"id": "3", "method": "get_document", "params": {"id": "D-1"}}),
    ]);

    let addressed: Vec<&str> = answers
        .iter()
        .map(|answer| {
            answer["id"]
                .as_str()
                .expect("every response echoes a string id")
        })
        .collect();
    assert_eq!(
        addressed,
        ["0", "1", "2", "3"],
        "every request is answered, and each response echoes its own id: {answers:?}"
    );
    for answer in &answers {
        assert_eq!(
            answer["error"],
            json!(null),
            "a well-formed request is not refused: {answer}"
        );
    }

    assert_eq!(answers[0]["result"]["kind"], json!("document-store"));
    assert_eq!(
        answers[0]["result"]["capabilities"]["documents"],
        json!("native"),
        "the peer declares it has documents: {}",
        answers[0]
    );
    assert_eq!(
        answers[1]["result"]["id"],
        json!("D-1"),
        "a create answers with the id this source now holds it under: {}",
        answers[1]
    );
    assert_eq!(
        answers[2]["result"]["items"],
        json!([source_document(json!({}))]),
        "the query returns the document whole, every metadata type intact: {}",
        answers[2]
    );
    assert_eq!(answers[2]["result"]["next"], json!(null));
    assert_eq!(
        answers[3]["result"]["document"],
        source_document(json!({})),
        "and so does a read by id: {}",
        answers[3]
    );
}

#[test]
fn the_peer_refuses_malformed_protocol_input_rather_than_raising_into_the_pipe() {
    // Every one of these is a shape the protocol does not have, and each is owed the same
    // answer: `malformed` (§5), naming what arrived, on a connection that carries on. A
    // Python exception here would reach the engine as a plugin that closed its output,
    // which says nothing about what was wrong with the request.
    let sandbox = Sandbox::new();
    let store = store_path(&sandbox);
    let refusals: Vec<(Value, &str)> = vec![
        (
            json!({"id": "m1", "method": "query_documents"}),
            "`params`, present even when empty",
        ),
        (
            json!({"id": "m2", "method": "query_documents", "params": []}),
            "`params`, present even when empty",
        ),
        (
            json!({"id": "m3", "method": 7, "params": {}}),
            "names its method as a string",
        ),
        (
            querying("m4", json!({"text": {"terms": 5, "fields": "title"}})),
            "search terms must be a string",
        ),
        (
            querying(
                "m5",
                json!({"text": {"terms": "alpha", "fields": "headings"}}),
            ),
            "search fields must be one of title, content, title-or-content",
        ),
        (
            querying("m6", json!({"labels": {"any_of": [7]}})),
            "any_of label name must be a string",
        ),
        (
            querying("m7", json!({"project": {"is": 7}})),
            "project filter must be",
        ),
        (
            json!({"id": "m8", "method": "query_documents", "params": {
                "query": {}, "page": {"cursor": 7, "limit": 50}}}),
            "page cursor must be a string or null",
        ),
        (
            json!({"id": "m9", "method": "query_documents", "params": {
                "query": {}, "page": {"cursor": null, "limit": "many"}}}),
            "page limit must be an integer",
        ),
        (
            json!({"id": "m10", "method": "write_document", "params": {
                "write": {"target": 7, "item": source_document(json!({}))}}}),
            "write target must be a native id or null",
        ),
        (
            creating("m11", source_document(json!({"title": 7}))),
            "needs a title that must be a string",
        ),
        (
            creating("m12", source_document(json!({"labels": [{"id": "L-1"}]}))),
            "label name must be a string",
        ),
        (
            creating(
                "m13",
                source_document(json!({"location": {"url": "u", "path": "p"}})),
            ),
            "location must be",
        ),
        (
            creating("m14", source_document(json!({"repositories": [7]}))),
            "repository origin must be a string",
        ),
        // The falsey ones, which are their own case: `[]`, `{}`, `0` and `false` are all
        // falsey in Python, so a member defaulted with `or` reads `false` as "none of
        // these" and accepts a shape the protocol does not have.
        (
            creating("m15", source_document(json!({"labels": false}))),
            "labels must be a list",
        ),
        (
            creating("m16", source_document(json!({"metadata": 0}))),
            "metadata must be an object",
        ),
        (
            creating("m17", source_document(json!({"repositories": false}))),
            "repositories must be a list",
        ),
        (
            querying("m18", json!({"labels": false})),
            "label filter must be an object",
        ),
        (
            querying("m19", json!({"labels": {"any_of": false}})),
            "any_of label filter must be a list",
        ),
    ];

    let mut sent = vec![hello(&store)];
    sent.extend(refusals.iter().map(|(request, _)| request.clone()));
    // A line the protocol gives no address to answer at: `id` is a string (§2), so this
    // one is dropped rather than answered — and the request after it still is, which is
    // what says the connection survived being handed one.
    sent.push(json!({"id": 7, "method": "get_document", "params": {"id": "D-1"}}));
    sent.push(querying("survivor", json!({})));
    let answers = converse(&sent);

    for (index, (request, expected)) in refusals.iter().enumerate() {
        let answer = &answers[index + 1];
        assert_eq!(
            answer["id"], request["id"],
            "each refusal is addressed to the request that earned it: {answer}"
        );
        assert_eq!(
            answer["error"]["kind"],
            json!("malformed"),
            "{request} is malformed, not a failure of the source: {answer}"
        );
        let message = answer["error"]["message"]
            .as_str()
            .expect("a malformed error carries a message a person reads");
        assert!(
            message.contains(expected),
            "the refusal has to say what arrived; wanted {expected:?} in: {message}"
        );
        assert_eq!(
            answer["result"],
            json!(null),
            "a refused request answers with an error and nothing else: {answer}"
        );
    }

    let addressed: Vec<&str> = answers
        .iter()
        .map(|answer| answer["id"].as_str().expect("a string id"))
        .collect();
    assert_eq!(
        addressed.last(),
        Some(&"survivor"),
        "the connection answers after every refusal: {answers:?}"
    );
    assert!(
        !addressed.contains(&"7") && addressed.len() == refusals.len() + 2,
        "a line whose id is not a string is dropped rather than answered: {addressed:?}"
    );
    assert_eq!(
        answers.last().expect("the survivor")["result"]["items"],
        json!([]),
        "and nothing a refused write named was persisted: {:?}",
        answers.last()
    );
    assert!(
        !store.exists(),
        "a refused write leaves the store as it found it: {store:?}"
    );
}

#[test]
fn the_peer_refuses_a_store_holding_something_that_is_not_a_document() {
    // The other half of the same boundary. A store file is as much untrusted input as a
    // request line — this one was edited by hand — and a peer that handed a half-shaped
    // document back would report its own defect as the engine's.
    let sandbox = Sandbox::new();
    let store = store_path(&sandbox);
    std::fs::create_dir_all(store.parent().expect("the store has a directory"))
        .expect("the store directory");
    std::fs::write(
        &store,
        json!({"documents": [{"id": "D-1", "title": ["not", "a", "title"]}]}).to_string(),
    )
    .expect("the store file");

    let answers = converse(&[
        hello(&store),
        json!({"id": "1", "method": "get_document", "params": {"id": "D-1"}}),
    ]);
    assert_eq!(answers[1]["error"]["kind"], json!("malformed"));
    let message = answers[1]["error"]["message"]
        .as_str()
        .expect("a message a person reads");
    assert!(
        message.contains("document 0") && message.contains("needs a title"),
        "the refusal names which entry of the store and what is wrong with it: {message}"
    );
}
