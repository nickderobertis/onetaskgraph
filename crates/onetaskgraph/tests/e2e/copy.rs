//! The copy verb, driven the way a user drives it.
//!
//! The round trip is the point of the whole verb, so it is proven rather than asserted:
//! a task is copied out of a destination into a folder of Markdown, the Markdown is
//! edited the way a person edits it, and the copy back updates the item it came from
//! instead of creating a second one — with every field the edit did not touch
//! byte-for-byte what it was.
//!
//! Every test here spawns the compiled binary as a subprocess and asserts on its exit
//! code, stdout and stderr. The same verb driven as a *library* call, which is the
//! consumer a command-line-only copy would strand, is proven in
//! `crates/onetaskgraph-core/tests/copy.rs`.

use std::process::Output;

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{ROWS, SOURCE, document, empty_folder, linear_block, qualified};

/// The folder every copy journey copies into, configured beside the source under test.
const NOTES: &str = "notes";

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
    let rendered = ok(sandbox, &[verb, "show", id, "--json"]);
    let response: Value = serde_json::from_str(&rendered).expect("show emits JSON");
    response["items"][0]["item"].clone()
}

/// Two Markdown folders, the first holding one task and the second empty, and the root
/// of the first.
///
/// This is the user's own flow in miniature: a folder standing in for the system their
/// team works out of, and a folder they author and edit in.
fn folders(sandbox: &Sandbox) -> std::path::PathBuf {
    let root = sandbox.subdirectory("remote");
    let tasks = root.join("tasks");
    std::fs::create_dir_all(&tasks).expect("the remote task folder");
    std::fs::write(
        tasks.join("ENG-1.md"),
        "---\ntitle: Rate-limit the sync loop\nstatus: doing\n\
         labels: [{id: L-1, name: bug}]\n\
         metadata: {caller.count: 3, caller.shape: {nested: [1, true, null]}}\n\
         repositories: [github.com/nickderobertis/onetaskgraph]\n---\nthe body\n",
    )
    .expect("the remote task");
    sandbox.project_document(&document(&json!({
        "remote": {"plugin": "local-md", "config": {
            "root": root,
            "status_mapping": {"todo": "todo", "doing": "in-progress", "shipped": "done"},
        }},
        NOTES: {"plugin": "local-md", "config": empty_folder(sandbox, NOTES)},
    })));
    root
}

#[test]
fn linear_is_a_permanent_task_destination_with_typed_metadata_and_repository_origins() {
    let sandbox = Sandbox::new();
    let root = sandbox.subdirectory("linear-task-source");
    std::fs::create_dir_all(root.join("tasks")).unwrap();
    std::fs::write(root.join("tasks/A.md"), "---\ntitle: Authored locally\nstatus: Todo\nlabels: [{id: local-bug, name: bug}]\nmetadata: {object: {a: 1}, array: [1, true], string: text, number: 3.5, boolean: true, null: null}\nrepositories: [github.com/acme/work]\n---\nvisible body\n").unwrap();
    sandbox.project_document(&document(&json!({
        "authored": {"plugin":"local-md","config":{"root":root,"status_mapping":{"Todo":"todo"}}},
        "linear": {"plugin":"linear","config":linear_block(&sandbox)},
    })));

    let first = reported(&ok(
        &sandbox,
        &["task", "copy", "authored:A", "--to", "linear", "--json"],
    ));
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].2, "created");
    let destination = first[0].1.as_str().unwrap().to_owned();
    let item = shown(&sandbox, "task", &destination);
    assert_eq!(item["title"], "Authored locally");
    assert_eq!(item["content"], "visible body");
    assert_eq!(item["status"]["name"], "Todo");
    assert_eq!(item["labels"][0]["name"], "bug");
    assert_eq!(item["repositories"], json!(["github.com/acme/work"]));
    for key in ["object", "array", "string", "number", "boolean", "null"] {
        let source = shown(&sandbox, "task", "authored:A");
        assert_eq!(
            item["metadata"][key], source["metadata"][key],
            "metadata key {key}"
        );
    }
    let second = reported(&ok(
        &sandbox,
        &["task", "copy", "authored:A", "--to", "linear", "--json"],
    ));
    assert_eq!(second[0].1, destination);
    assert!(matches!(second[0].2.as_str(), "updated" | "unchanged"));
}

#[test]
fn a_round_trip_edit_updates_the_item_it_came_from_rather_than_duplicating_it() {
    let sandbox = Sandbox::new();
    folders(&sandbox);

    // Out of the destination and into Markdown.
    let copied = ok(&sandbox, &["task", "copy", "remote:ENG-1", "--to", NOTES]);
    assert_eq!(
        copied.split_whitespace().collect::<Vec<_>>(),
        ["remote:ENG-1", "notes:ENG-1", "created"]
    );
    let before = shown(&sandbox, "task", "remote:ENG-1");

    // Edited the way a person edits it: one field, in the file.
    let file = sandbox.project().join(NOTES).join("tasks/ENG-1.md");
    let text = std::fs::read_to_string(&file).expect("the copied Markdown is there");
    assert!(
        text.contains("onetaskgraph.origin: remote:ENG-1"),
        "the copy records where it came from:\n{text}"
    );
    std::fs::write(
        &file,
        text.replace(
            "title: Rate-limit the sync loop",
            "title: Rate-limit the sync loop, carefully",
        ),
    )
    .expect("the edit lands");

    let back = ok(
        &sandbox,
        &["task", "copy", "notes:ENG-1", "--to", "remote", "--json"],
    );
    assert_eq!(
        reported(&back),
        vec![(
            "notes:ENG-1".to_owned(),
            json!("remote:ENG-1"),
            "updated".to_owned()
        )]
    );

    // Exactly one item where there was one before: the copy back updated, and the
    // folder holds no second document.
    assert_eq!(
        ok(&sandbox, &["task", "list", "--source", "remote"])
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count(),
        1
    );

    let after = shown(&sandbox, "task", "remote:ENG-1");
    assert_eq!(
        after["title"],
        json!("Rate-limit the sync loop, carefully"),
        "the edited field changed"
    );
    // And every field the edit did not touch is byte-for-byte what it was.
    for field in [
        "id",
        "content",
        "status",
        "labels",
        "project",
        "repositories",
        "url",
        "created_at",
        "updated_at",
    ] {
        assert_eq!(
            after[field], before[field],
            "{field} survived the round trip"
        );
    }
    for key in ["caller.count", "caller.shape"] {
        assert_eq!(
            after["metadata"][key], before["metadata"][key],
            "{key} survived the round trip"
        );
    }
    // The one thing that did change is the correspondence itself, which is the mechanism
    // rather than a field of the user's: the remote item now records where its last copy
    // came from, so the next edit finds it directly.
    assert_eq!(
        after["metadata"]["onetaskgraph.origin"],
        json!("notes:ENG-1")
    );
}

#[test]
fn every_source_kind_can_be_copied_into_a_folder_of_markdown_with_its_fields_intact() {
    // A journey written once and run against every configured source kind, so no plugin
    // is proven by a suite of its own writing. The destination is a Markdown folder
    // because that is the one every source can be copied *into* today, and it is what the
    // user's own flow authors and edits in.
    for row in ROWS {
        let sandbox = Sandbox::new();
        sandbox.project_document(&row.document_with_folder(&sandbox, NOTES));
        let from = qualified(SOURCE, "T-1");

        let planned = ok(&sandbox, &["task", "copy", &from, "--to", NOTES, "--json"]);
        assert_eq!(
            reported(&planned),
            vec![(from.clone(), json!("notes:T-1"), "created".to_owned())],
            "{}",
            row.name
        );

        let source = shown(&sandbox, "task", &from);
        let copied = shown(&sandbox, "task", "notes:T-1");
        for field in ["title", "content", "status", "labels", "repositories"] {
            assert_eq!(copied[field], source[field], "{}: {field}", row.name);
        }
        // Value and JSON type alike, for every key the caller owns.
        for key in ["onepipeline.turn_budget", "caller.flags"] {
            assert_eq!(
                copied["metadata"][key], source["metadata"][key],
                "{}: {key}",
                row.name
            );
        }
        assert_eq!(
            copied["metadata"]["onetaskgraph.origin"],
            json!(from),
            "{}",
            row.name
        );
        // `url` is the destination's own and is never written.
        assert_eq!(copied["url"], Value::Null, "{}", row.name);

        // A second copy of the same item updates that one and creates nothing.
        let again = ok(&sandbox, &["task", "copy", &from, "--to", NOTES, "--json"]);
        assert_eq!(
            reported(&again),
            vec![(from.clone(), json!("notes:T-1"), "unchanged".to_owned())],
            "{}",
            row.name
        );
        assert_eq!(
            ok(&sandbox, &["task", "list", "--source", NOTES])
                .lines()
                .filter(|line| !line.trim().is_empty())
                .count(),
            1,
            "{}",
            row.name
        );
    }
}

#[test]
fn several_ids_in_one_command_are_one_copied_set_whose_edges_are_recreated() {
    // The ids named together *are* the set: an edge between two of them is recreated at
    // the destination, which one command per id could not do — the far end's own
    // destination id is not known until the copy that creates it has run.
    let row = &ROWS[0];
    let sandbox = Sandbox::new();
    sandbox.project_document(&row.document_with_folder(&sandbox, NOTES));
    let first = qualified(SOURCE, "T-1");
    let second = qualified(SOURCE, "T-2");

    let copied = ok(
        &sandbox,
        &["task", "copy", &first, &second, "--to", NOTES, "--json"],
    );
    assert_eq!(
        reported(&copied),
        vec![
            (first.clone(), json!("notes:T-1"), "created".to_owned()),
            (second.clone(), json!("notes:T-2"), "created".to_owned()),
        ]
    );

    let edges = ok(&sandbox, &["task", "deps", "notes:T-1", "--json"]);
    let edges: Value = serde_json::from_str(&edges).expect("deps emits JSON");
    let ends: Vec<&str> = edges["items"]
        .as_array()
        .expect("an array of edges")
        .iter()
        .map(|edge| edge["to"]["id"].as_str().expect("a qualified id"))
        .collect();
    assert!(
        ends.contains(&"notes:T-2"),
        "the far end inside the copied set is the destination's own id: {ends:?}"
    );
    assert!(
        ends.contains(&"elsewhere:P-9"),
        "a far end already naming another source is left exactly as it is: {ends:?}"
    );
}

#[test]
fn a_dry_run_reads_everything_writes_nothing_and_says_what_it_would_have_done() {
    let sandbox = Sandbox::new();
    folders(&sandbox);

    let planned = ok(
        &sandbox,
        &[
            "task",
            "copy",
            "remote:ENG-1",
            "--to",
            NOTES,
            "--dry-run",
            "--json",
        ],
    );
    assert_eq!(
        reported(&planned),
        vec![("remote:ENG-1".to_owned(), Value::Null, "created".to_owned())],
        "a dry run that would create has no destination id, because nothing was created"
    );
    assert!(
        !sandbox.project().join(NOTES).join("tasks").exists(),
        "a dry run writes nothing"
    );

    // And once something is there, a dry run over it names the id it would update.
    ok(&sandbox, &["task", "copy", "remote:ENG-1", "--to", NOTES]);
    let file = sandbox.project().join(NOTES).join("tasks/ENG-1.md");
    let before = std::fs::read(&file).expect("the copied Markdown is there");
    std::fs::write(
        &file,
        String::from_utf8(before.clone())
            .expect("UTF-8")
            .replace("title: Rate-limit", "title: Edited rate-limit"),
    )
    .expect("the edit lands");
    let edited = std::fs::read(&file).expect("the edited Markdown is there");

    let planned = ok(
        &sandbox,
        &[
            "task",
            "copy",
            "remote:ENG-1",
            "--to",
            NOTES,
            "--dry-run",
            "--json",
        ],
    );
    assert_eq!(
        reported(&planned),
        vec![(
            "remote:ENG-1".to_owned(),
            json!("notes:ENG-1"),
            "updated".to_owned()
        )]
    );
    assert_eq!(
        std::fs::read(&file).expect("the Markdown is still there"),
        edited,
        "a dry run over an item it would update still writes nothing"
    );
}

#[test]
fn a_destination_configured_with_no_write_side_exits_non_zero_naming_it_and_its_plugin() {
    let sandbox = Sandbox::new();
    sandbox.project_document(&document(&json!({
        SOURCE: {"plugin": "in-memory", "config": {"tasks": [
            {"id": "T-1", "title": "Alpha", "status": {"category": "todo", "name": "Todo"},
             "labels": []}
        ]}},
        "sealed": {"plugin": "in-memory", "config": {
            "capabilities": {"writes": "unsupported"}
        }},
    })));

    let said = refused(
        &sandbox,
        &["task", "copy", &qualified(SOURCE, "T-1"), "--to", "sealed"],
        1,
    );
    assert!(said.contains("source sealed cannot be written"), "{said}");
    assert!(said.contains("its plugin is in-memory"), "{said}");
    assert!(said.contains("sources list"), "{said}");
}

#[test]
fn a_destination_that_cannot_carry_a_key_refuses_the_write_naming_the_source_and_the_keys() {
    let sandbox = Sandbox::new();
    sandbox.project_document(&document(&json!({
        SOURCE: {"plugin": "in-memory", "config": {"tasks": [
            {"id": "T-1", "title": "Alpha", "status": {"category": "todo", "name": "Todo"},
             "labels": [], "metadata": {"caller.flags": [true, null], "caller.count": 3}}
        ]}},
        "picky": {"plugin": "in-memory", "config": {
            "capabilities": {"unwritable_metadata_keys": ["caller.flags", "caller.count"]}
        }},
    })));

    let said = refused(
        &sandbox,
        &["task", "copy", &qualified(SOURCE, "T-1"), "--to", "picky"],
        1,
    );
    assert!(said.contains("source picky could not do it"), "{said}");
    assert!(said.contains("caller.count, caller.flags"), "{said}");
    assert!(
        !said.contains("dropped"),
        "the keys are named rather than dropped: {said}"
    );
}

#[test]
fn a_stale_origin_refuses_and_recreate_falls_through_to_matching_by_origin_instead() {
    let sandbox = Sandbox::new();
    let remote = folders(&sandbox);
    ok(&sandbox, &["task", "copy", "remote:ENG-1", "--to", NOTES]);

    // Somebody deletes the counterpart at the destination on purpose.
    std::fs::remove_file(remote.join("tasks/ENG-1.md")).expect("the remote document goes away");

    let said = refused(
        &sandbox,
        &["task", "copy", "notes:ENG-1", "--to", "remote"],
        1,
    );
    assert!(
        said.contains("notes:ENG-1 was copied from remote:ENG-1"),
        "{said}"
    );
    assert!(said.contains("--recreate"), "{said}");

    let created = ok(
        &sandbox,
        &[
            "task",
            "copy",
            "notes:ENG-1",
            "--to",
            "remote",
            "--recreate",
            "--json",
        ],
    );
    assert_eq!(
        reported(&created),
        vec![(
            "notes:ENG-1".to_owned(),
            json!("remote:ENG-1"),
            "created".to_owned()
        )]
    );
}

#[test]
fn an_origin_a_person_removed_creates_until_match_by_re_establishes_the_correspondence() {
    let sandbox = Sandbox::new();
    folders(&sandbox);
    ok(&sandbox, &["task", "copy", "remote:ENG-1", "--to", NOTES]);

    // A person edits the Markdown and deletes the key: neither rule can find the
    // counterpart any more, so the next copy creates a second document.
    let file = sandbox.project().join(NOTES).join("tasks/ENG-1.md");
    let text = std::fs::read_to_string(&file).expect("the copied Markdown is there");
    std::fs::write(
        &file,
        text.replace("  onetaskgraph.origin: remote:ENG-1\n", ""),
    )
    .expect("the edit lands");

    let duplicated = ok(
        &sandbox,
        &["task", "copy", "remote:ENG-1", "--to", NOTES, "--json"],
    );
    assert_eq!(
        reported(&duplicated),
        vec![(
            "remote:ENG-1".to_owned(),
            json!("notes:ENG-1-2"),
            "created".to_owned()
        )]
    );

    // The caller-named escape re-establishes it without hand-editing ids.
    std::fs::remove_file(sandbox.project().join(NOTES).join("tasks/ENG-1-2.md"))
        .expect("the duplicate goes away");
    let matched = ok(
        &sandbox,
        &[
            "task",
            "copy",
            "remote:ENG-1",
            "--to",
            NOTES,
            "--match-by",
            "title",
            "--json",
        ],
    );
    assert_eq!(
        reported(&matched),
        vec![(
            "remote:ENG-1".to_owned(),
            json!("notes:ENG-1"),
            "updated".to_owned()
        )]
    );
}

#[test]
fn copying_a_project_carries_its_tasks_and_reports_one_the_source_no_longer_holds() {
    let sandbox = Sandbox::new();
    let root = sandbox.subdirectory("remote");
    for (kind, id, front) in [
        (
            "projects",
            "P-1",
            "title: Engine\nstatus: doing\ndepends_on: [{id: T-1, item: task}]",
        ),
        ("tasks", "T-1", "title: Alpha\nstatus: todo\nproject: P-1"),
        ("tasks", "T-2", "title: Beta\nstatus: todo\nproject: P-1"),
    ] {
        let path = root.join(kind).join(format!("{id}.md"));
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("the folder");
        std::fs::write(path, format!("---\n{front}\n---\nbody\n")).expect("the document");
    }
    sandbox.project_document(&document(&json!({
        "remote": {"plugin": "local-md", "config": {
            "root": root,
            "status_mapping": {"todo": "todo", "doing": "in-progress"},
        }},
        NOTES: {"plugin": "local-md", "config": empty_folder(&sandbox, NOTES)},
    })));

    // A dry run of a project the destination does not hold yet still reads every task in
    // it and reports what each would have got — there is simply no destination id for any
    // of them, because nothing was written and the project they would be filed under does
    // not exist.
    let planned = ok(
        &sandbox,
        &[
            "project",
            "copy",
            "remote:P-1",
            "--to",
            NOTES,
            "--dry-run",
            "--json",
        ],
    );
    assert_eq!(
        reported(&planned),
        vec![
            ("remote:P-1".to_owned(), Value::Null, "created".to_owned()),
            ("remote:T-1".to_owned(), Value::Null, "created".to_owned()),
            ("remote:T-2".to_owned(), Value::Null, "created".to_owned()),
        ]
    );
    assert!(
        !sandbox.project().join(NOTES).join("tasks").exists()
            && !sandbox.project().join(NOTES).join("projects").exists(),
        "a dry run writes nothing"
    );

    let copied = ok(
        &sandbox,
        &["project", "copy", "remote:P-1", "--to", NOTES, "--json"],
    );
    assert_eq!(
        reported(&copied),
        vec![
            (
                "remote:P-1".to_owned(),
                json!("notes:P-1"),
                "created".to_owned()
            ),
            (
                "remote:T-1".to_owned(),
                json!("notes:T-1"),
                "created".to_owned()
            ),
            (
                "remote:T-2".to_owned(),
                json!("notes:T-2"),
                "created".to_owned()
            ),
        ]
    );
    // Each copied task is filed under the destination project rather than the source's.
    assert_eq!(
        shown(&sandbox, "task", "notes:T-1")["project"],
        json!("P-1")
    );
    let dependencies: Value =
        serde_json::from_str(&ok(&sandbox, &["project", "deps", "notes:P-1", "--json"]))
            .expect("project dependencies emit JSON");
    assert!(
        dependencies["items"]
            .as_array()
            .expect("dependency items")
            .contains(&json!({
                "from": {"id": "notes:P-1", "kind": "project"},
                "to": {"id": "notes:T-1", "kind": "task"},
                "kind": "blocks"
            })),
        "the copied project edge is recreated between destination items: {dependencies:#}"
    );

    // A second copy matches each task independently and duplicates nothing.
    let again = ok(
        &sandbox,
        &["project", "copy", "remote:P-1", "--to", NOTES, "--json"],
    );
    assert!(
        reported(&again)
            .iter()
            .all(|(_, _, action)| action == "unchanged"),
        "{again}"
    );

    // A destination item the source no longer holds is left alone and reported.
    std::fs::remove_file(root.join("tasks/T-2.md")).expect("the source drops a task");
    let orphaned = ok(
        &sandbox,
        &["project", "copy", "remote:P-1", "--to", NOTES, "--json"],
    );
    assert!(
        reported(&orphaned).contains(&(
            "remote:T-2".to_owned(),
            json!("notes:T-2"),
            "orphaned".to_owned()
        )),
        "{orphaned}"
    );
    assert_eq!(shown(&sandbox, "task", "notes:T-2")["title"], json!("Beta"));

    let alone = ok(
        &sandbox,
        &[
            "project",
            "copy",
            "remote:P-1",
            "--to",
            NOTES,
            "--no-tasks",
            "--json",
        ],
    );
    assert_eq!(reported(&alone).len(), 1);
}

#[test]
fn a_copy_that_cannot_run_at_all_exits_non_zero_with_a_suggested_next_action() {
    let sandbox = Sandbox::new();
    folders(&sandbox);

    for (arguments, expected) in [
        (
            vec!["task", "copy", "remote:absent", "--to", NOTES],
            "no item with the id remote:absent",
        ),
        (
            vec!["task", "copy", "remote:ENG-1", "--to", "nowhere"],
            "no source named \"nowhere\" is configured",
        ),
        (
            vec!["task", "copy", "ENG-1", "--to", NOTES],
            "is not a qualified id",
        ),
        (
            vec!["task", "copy", "remote:ENG-1", "--to", "NOT A NAME"],
            "--to NOT A NAME",
        ),
    ] {
        let said = refused(&sandbox, &arguments, 1);
        assert!(said.contains(expected), "{arguments:?}: {said}");
        assert!(said.contains("next:"), "{arguments:?}: {said}");
    }
}
