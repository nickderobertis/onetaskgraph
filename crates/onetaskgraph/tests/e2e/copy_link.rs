//! The link a copy records on the item it copied, driven the way a user drives it.
//!
//! A copy records where each item landed on the item itself, at `onetaskgraph.copies`, and
//! the next copy of that item follows it by one read of the destination by id. What a
//! command line can see of that is proven here: the entry in the source's own file, over
//! the in-process boundary and the stdio plugin protocol alike, and on a GitHub board item
//! through that board's own metadata write; the rule each copy reports it was found by; and
//! the refusal a link naming nothing earns. That a copy found by its
//! link reads no page of the destination is counted where it can be — against the
//! in-memory sources in `crates/onetaskgraph-core/tests/copy_link.rs`, and against the
//! loopback GitHub board in `copy_cost.rs`.

use std::process::Output;

use serde_json::{Value, json};

use crate::common::{SOURCE_BOUNDARIES, Sandbox, SourceBoundary, stderr, stdout};
use crate::fixtures::{SOURCE, document, empty_folder, github_projects_with_board, qualified};

const NOTES: &str = "notes";

fn run(sandbox: &Sandbox, arguments: &[&str]) -> Output {
    sandbox
        .command()
        .args(arguments)
        .assert()
        .get_output()
        .clone()
}

/// The output of a run that had to exit `code`, quoting both streams when it did not.
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

/// A folder of Markdown holding every kind of record, and an empty one beside it, both
/// behind `boundary`.
///
/// Every record's `metadata:` is a block of entries or absent, which is what this folder
/// edits one key of narrowly. `T-9`'s is written on one line, which it will not edit without
/// rewriting the rest — so that one item's source cannot hold a link.
fn markdown_pair(sandbox: &Sandbox, boundary: SourceBoundary) {
    let root = sandbox.subdirectory("local-md");
    for (relative, text) in [
        (
            "tasks/T-1.md",
            "---\ntitle: Alpha engine\nstatus: todo\nmetadata:\n  caller.flags: [true, null]\n  \
             onepipeline.turn_budget: 12\n---\nthe engine core\n",
        ),
        (
            "tasks/T-9.md",
            "---\ntitle: One-line metadata\nstatus: todo\n\
             metadata: {caller.flags: [true, null]}\n---\nheld as written\n",
        ),
        (
            "projects/P-1.md",
            "---\ntitle: Engine\nstatus: doing\n---\nthe engine\n",
        ),
        (
            "documents/D-1.md",
            "---\ntitle: Alpha design\nmetadata:\n  caller.flags: [true, null]\n---\nreviewed\n",
        ),
    ] {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().expect("a folder")).expect("a folder");
        std::fs::write(path, text).expect("a record");
    }
    sandbox.project_document(&document(&json!({
        SOURCE: boundary.source("local-md", empty_folder(sandbox, "local-md")),
        NOTES: boundary.source("local-md", empty_folder(sandbox, NOTES)),
    })));
}

/// One `--json` copy of `id` of kind `verb` into the notes folder, and the one item it
/// reports as `(destination, action, via, link)`.
fn copy(sandbox: &Sandbox, verb: &str, id: &str, extra: &[&str]) -> (Value, Value, Value, Value) {
    let mut arguments = vec![verb, "copy", id, "--to", NOTES, "--json"];
    arguments.extend_from_slice(extra);
    let report = parsed(&exits(sandbox, &arguments, 0));
    let item = &report["items"][0];
    (
        item["destination"].clone(),
        item["action"].clone(),
        item["via"].clone(),
        item["link"].clone(),
    )
}

/// The README's own words, which the journeys below hold to what the binary does.
fn readme() -> String {
    std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../README.md"),
    )
    .expect("README.md is readable")
}

fn metadata(sandbox: &Sandbox, verb: &str, id: &str) -> Value {
    parsed(&exits(sandbox, &[verb, "show", id, "--json"], 0))["items"][0]["item"]["metadata"]
        .clone()
}

fn source_file(sandbox: &Sandbox, verb: &str, id: &str) -> std::path::PathBuf {
    sandbox
        .subdirectory("local-md")
        .join(format!("{verb}s"))
        .join(format!("{id}.md"))
}

#[test]
fn every_kind_copied_out_of_a_folder_records_its_link_and_the_next_copy_follows_it() {
    for boundary in SOURCE_BOUNDARIES {
        let sandbox = Sandbox::new();
        markdown_pair(&sandbox, boundary);

        for (verb, id, extra) in [
            ("task", "T-1", &[][..]),
            ("project", "P-1", &["--no-tasks"][..]),
            ("document", "D-1", &[][..]),
        ] {
            let from = qualified(SOURCE, id);
            let into = qualified(NOTES, id);
            let before = std::fs::read_to_string(source_file(&sandbox, verb, id))
                .expect("the source record");

            assert_eq!(
                copy(&sandbox, verb, &from, extra),
                (
                    json!(into),
                    json!("created"),
                    json!("created"),
                    json!("recorded")
                ),
                "{boundary:?}: {verb} {id}"
            );
            // The link is in the source's own file, and a later invocation reads it back.
            let after = std::fs::read_to_string(source_file(&sandbox, verb, id))
                .expect("the source record");
            assert!(
                after.contains("onetaskgraph.copies") && after.contains(&into),
                "{boundary:?}: the source file records where {id} landed:\n{after}"
            );
            assert_ne!(before, after, "{boundary:?}: {verb} {id}");
            let recorded = metadata(&sandbox, verb, &from)["onetaskgraph.copies"].clone();
            assert_eq!(recorded, json!({NOTES: into}), "{boundary:?}: {verb} {id}");
            // The README shows a link by example, and this is the link that example is of.
            if verb == "task" {
                let example = format!("`{recorded}`").replace("\":\"", "\": \"");
                assert!(
                    readme().contains(&example),
                    "the README's example of a link is not what a copy records: {example}"
                );
            }
            if verb != "project" {
                assert_eq!(
                    metadata(&sandbox, verb, &from)["caller.flags"],
                    json!([true, null]),
                    "{boundary:?}: {verb} {id}"
                );
            }
            // And it never travels: the copy records its origin and no link of its own.
            let landed = metadata(&sandbox, verb, &into);
            assert_eq!(landed["onetaskgraph.origin"], json!(from), "{boundary:?}");
            assert!(
                landed.get("onetaskgraph.copies").is_none(),
                "{boundary:?}: {verb} {id} carried the link: {landed:#}"
            );

            assert_eq!(
                copy(&sandbox, verb, &from, extra),
                (
                    json!(into),
                    json!("unchanged"),
                    json!("link"),
                    json!("unchanged")
                ),
                "{boundary:?}: the next copy of {verb} {id} follows its link"
            );
        }

        let path = source_file(&sandbox, "task", "T-1");
        let edited = std::fs::read_to_string(&path)
            .expect("the source task")
            .replace("title: Alpha engine", "title: Alpha engine, edited");
        std::fs::write(&path, edited).expect("an edit");
        assert_eq!(
            copy(&sandbox, "task", &qualified(SOURCE, "T-1"), &[]),
            (
                json!(qualified(NOTES, "T-1")),
                json!("updated"),
                json!("link"),
                json!("unchanged")
            ),
            "{boundary:?}"
        );
    }
}

#[test]
fn a_link_naming_nothing_refuses_naming_both_ids_until_recreate_records_it_again() {
    for boundary in SOURCE_BOUNDARIES {
        let sandbox = Sandbox::new();
        markdown_pair(&sandbox, boundary);
        let from = qualified(SOURCE, "T-1");
        copy(&sandbox, "task", &from, &[]);

        // Somebody deletes the copy at the destination on purpose.
        std::fs::remove_file(sandbox.subdirectory(NOTES).join("tasks/T-1.md"))
            .expect("the copy goes away");

        let output = exits(&sandbox, &["task", "copy", &from, "--to", NOTES], 1);
        let said = stderr(&output);
        assert!(
            said.contains(&format!("{from} was last copied to notes:T-1")),
            "{boundary:?}: {said}"
        );
        assert!(said.contains("--recreate"), "{boundary:?}: {said}");
        let failure = parsed(&exits(
            &sandbox,
            &["task", "copy", &from, "--to", NOTES, "--json"],
            1,
        ));
        assert_eq!(
            failure["failure"]["kind"],
            json!("stale-link"),
            "{boundary:?}"
        );
        // And the README names the refusal by the kind the binary reports it under.
        let rules = readme();
        let rules = &rules[rules
            .find("These rules find the counterpart, in this order:")
            .expect("the README's rules")..];
        assert!(
            rules.contains(&format!(
                "the failure kind\n   `{}`",
                failure["failure"]["kind"].as_str().expect("a kind")
            )),
            "the README's rule for a link naming nothing does not name the kind the binary reports"
        );
        assert!(
            !sandbox.subdirectory(NOTES).join("tasks/T-1.md").exists(),
            "{boundary:?}: nothing was created"
        );

        // A folder names what it creates after the source's id, so the new item has the id
        // the old one had and the link already names it: nothing to write there. The core
        // suite's own journey recreates under a new id and sees the link rewritten.
        assert_eq!(
            copy(&sandbox, "task", &from, &["--recreate"]),
            (
                json!(qualified(NOTES, "T-1")),
                json!("created"),
                json!("created"),
                json!("unchanged")
            ),
            "{boundary:?}"
        );
        assert_eq!(
            metadata(&sandbox, "task", &from)["onetaskgraph.copies"],
            json!({NOTES: qualified(NOTES, "T-1")}),
            "{boundary:?}"
        );
    }
}

#[test]
fn a_copy_back_is_found_by_its_origin_and_records_nothing_on_either_side() {
    // The pair is found from either side by one read by id: forward by the link, and back
    // by the origin the copy carries — which records nothing new, so the copy-back writes
    // nothing to the item it copied.
    let sandbox = Sandbox::new();
    markdown_pair(&sandbox, SOURCE_BOUNDARIES[0]);
    let from = qualified(SOURCE, "T-1");
    let into = qualified(NOTES, "T-1");
    copy(&sandbox, "task", &from, &[]);
    let copied = sandbox.subdirectory(NOTES).join("tasks/T-1.md");
    let held = std::fs::read_to_string(&copied).expect("the copy");

    let back = parsed(&exits(
        &sandbox,
        &["task", "copy", &into, "--to", SOURCE, "--json"],
        0,
    ));
    assert_eq!(
        (
            &back["items"][0]["destination"],
            &back["items"][0]["action"],
            &back["items"][0]["via"],
            &back["items"][0]["link"],
        ),
        (
            &json!(from),
            &json!("unchanged"),
            &json!("origin"),
            &json!("unchanged")
        ),
        "{back:#}"
    );
    assert_eq!(
        std::fs::read_to_string(&copied).expect("the copy"),
        held,
        "the copy-back wrote nothing on the item it copied"
    );
}

#[test]
fn a_dry_run_reports_the_rule_and_leaves_link_out() {
    let sandbox = Sandbox::new();
    markdown_pair(&sandbox, SOURCE_BOUNDARIES[0]);
    let from = qualified(SOURCE, "T-1");
    let before = std::fs::read_to_string(source_file(&sandbox, "task", "T-1")).expect("a task");

    let report = parsed(&exits(
        &sandbox,
        &["task", "copy", &from, "--to", NOTES, "--dry-run", "--json"],
        0,
    ));
    assert_eq!(report["items"][0]["via"], json!("created"), "{report:#}");
    assert!(report["items"][0].get("link").is_none(), "{report:#}");
    assert_eq!(
        std::fs::read_to_string(source_file(&sandbox, "task", "T-1")).expect("a task"),
        before
    );
}

#[test]
fn an_item_its_folder_cannot_edit_narrowly_is_copied_and_reported_unrecorded() {
    for boundary in SOURCE_BOUNDARIES {
        let sandbox = Sandbox::new();
        markdown_pair(&sandbox, boundary);
        let from = qualified(SOURCE, "T-9");
        let before = std::fs::read_to_string(source_file(&sandbox, "task", "T-9")).expect("T-9");

        assert_eq!(
            copy(&sandbox, "task", &from, &[]),
            (
                json!(qualified(NOTES, "T-9")),
                json!("created"),
                json!("created"),
                json!("unrecorded")
            ),
            "{boundary:?}"
        );
        assert_eq!(
            std::fs::read_to_string(source_file(&sandbox, "task", "T-9")).expect("T-9"),
            before,
            "{boundary:?}: the file is left exactly as it was written"
        );
        // With no link, the next copy finds its counterpart the way it always has.
        assert_eq!(
            copy(&sandbox, "task", &from, &[]),
            (
                json!(qualified(NOTES, "T-9")),
                json!("unchanged"),
                json!("scan"),
                json!("unrecorded")
            ),
            "{boundary:?}"
        );
    }
}

#[test]
fn a_copy_found_by_its_title_or_by_searching_reports_so_and_records_the_link() {
    let sandbox = Sandbox::new();
    markdown_pair(&sandbox, SOURCE_BOUNDARIES[0]);
    let from = qualified(SOURCE, "T-1");
    let into = qualified(NOTES, "T-1");
    // A person's own item of the same title, written before any copy and naming no origin.
    let theirs = sandbox.subdirectory(NOTES).join("tasks/T-1.md");
    std::fs::create_dir_all(theirs.parent().expect("a folder")).expect("a folder");
    std::fs::write(
        &theirs,
        "---\ntitle: Alpha engine\nstatus: todo\n---\ntheirs\n",
    )
    .expect("their item");

    let (destination, action, via, link) = copy(&sandbox, "task", &from, &["--match-by", "title"]);
    assert_eq!(
        (destination, action, via, link),
        (
            json!(into),
            json!("updated"),
            json!("match"),
            json!("recorded")
        )
    );
    assert_eq!(
        metadata(&sandbox, "task", &from)["onetaskgraph.copies"],
        json!({NOTES: into})
    );

    // With the link taken off again, the next copy finds the item by the origin the first one
    // wrote there, and records the link it no longer has.
    let path = source_file(&sandbox, "task", "T-1");
    let linked = std::fs::read_to_string(&path).expect("the source task");
    let unlinked: String = linked
        .lines()
        .filter(|line| !line.contains("onetaskgraph.copies"))
        .map(|line| format!("{line}\n"))
        .collect();
    assert_ne!(linked, unlinked, "the link was one line of the file");
    std::fs::write(&path, unlinked).expect("the link removed");
    assert_eq!(
        copy(&sandbox, "task", &from, &[]),
        (
            json!(into),
            json!("unchanged"),
            json!("scan"),
            json!("recorded")
        )
    );
    assert_eq!(
        metadata(&sandbox, "task", &from)["onetaskgraph.copies"],
        json!({NOTES: into})
    );
}

#[test]
fn a_dry_run_follows_a_recorded_link_and_writes_neither_side() {
    let sandbox = Sandbox::new();
    markdown_pair(&sandbox, SOURCE_BOUNDARIES[0]);
    let from = qualified(SOURCE, "T-1");
    let notes = sandbox.subdirectory(NOTES);

    // Before any copy: nothing is created at the destination and no link is written.
    parsed(&exits(
        &sandbox,
        &["task", "copy", &from, "--to", NOTES, "--dry-run", "--json"],
        0,
    ));
    assert!(
        !notes.join("tasks/T-1.md").exists(),
        "a dry run created the copy"
    );

    // After one: the dry run finds the copy by the link and touches neither file.
    copy(&sandbox, "task", &from, &[]);
    let source = source_file(&sandbox, "task", "T-1");
    let edited = std::fs::read_to_string(&source)
        .expect("the source task")
        .replace("title: Alpha engine", "title: Alpha engine, edited");
    std::fs::write(&source, &edited).expect("an edit");
    let copied = std::fs::read_to_string(notes.join("tasks/T-1.md")).expect("the copy");

    let report = parsed(&exits(
        &sandbox,
        &["task", "copy", &from, "--to", NOTES, "--dry-run", "--json"],
        0,
    ));
    let item = &report["items"][0];
    assert_eq!(
        (&item["action"], &item["via"]),
        (&json!("updated"), &json!("link")),
        "{report:#}"
    );
    assert!(item.get("link").is_none(), "{report:#}");
    assert_eq!(
        std::fs::read_to_string(&source).expect("the source task"),
        edited
    );
    assert_eq!(
        std::fs::read_to_string(notes.join("tasks/T-1.md")).expect("the copy"),
        copied
    );
}

#[test]
fn a_board_item_copied_out_records_its_link_on_the_board_and_the_next_copy_follows_it() {
    for boundary in SOURCE_BOUNDARIES {
        let sandbox = Sandbox::new();
        let authored = sandbox.subdirectory("authored");
        std::fs::create_dir_all(authored.join("tasks")).expect("a folder");
        std::fs::write(
            authored.join("tasks/A.md"),
            "---\ntitle: Board-born task\nstatus: Todo\n---\nfiled on the board\n",
        )
        .expect("a task");
        let (config, board) = github_projects_with_board(&sandbox);
        sandbox.project_document(&document(&json!({
            "authored": {"plugin":"local-md","config":{
                "root": authored,
                "status_mapping": {"Todo":"todo","Doing":"in-progress","Shipped":"done"}}},
            "board": boundary.source_with_secrets(
                "github-projects",
                config,
                &["GITHUB_PROJECTS_FIXTURE_TOKEN"],
            ),
            NOTES: {"plugin": "local-md", "config": empty_folder(&sandbox, NOTES)},
        })));

        // An issue on the board, whose own origin names the folder it came from — not notes.
        let filed = parsed(&exits(
            &sandbox,
            &["task", "copy", "authored:A", "--to", "board", "--json"],
            0,
        ));
        let issue = filed["items"][0]["destination"]
            .as_str()
            .expect("a board id")
            .to_owned();

        let copy_out = ["task", "copy", issue.as_str(), "--to", NOTES, "--json"];
        let first = parsed(&exits(&sandbox, &copy_out, 0));
        let item = &first["items"][0];
        assert_eq!(
            (&item["action"], &item["via"], &item["link"]),
            (&json!("created"), &json!("created"), &json!("recorded")),
            "{boundary:?}: {first:#}"
        );
        let landed = item["destination"].as_str().expect("a notes id").to_owned();

        // The link went through the board's own metadata write, into the issue's body …
        assert!(
            board.served().iter().any(|(document, variables)| {
                document.contains("updateIssue")
                    && variables.to_string().contains("onetaskgraph.copies")
            }),
            "{boundary:?}: no issue update carried the link"
        );
        // … and a later invocation reads it back from the board.
        assert_eq!(
            metadata(&sandbox, "task", &issue)["onetaskgraph.copies"],
            json!({NOTES: landed}),
            "{boundary:?}"
        );

        let second = parsed(&exits(&sandbox, &copy_out, 0));
        let item = &second["items"][0];
        assert_eq!(
            (&item["destination"], &item["via"], &item["link"]),
            (&json!(landed), &json!("link"), &json!("unchanged")),
            "{boundary:?}: {second:#}"
        );
    }
}
