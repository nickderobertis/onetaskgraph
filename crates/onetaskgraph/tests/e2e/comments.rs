//! Comments on a task, driven the way a user drives them.
//!
//! Every journey here spawns the compiled binary and asserts on its exit code, stdout and
//! stderr. A row whose source keeps what it is given — a folder of Markdown, a GitHub board,
//! a Linear workspace — is driven through a whole add, list, edit and delete across separate
//! invocations, over the in-process boundary and the stdio plugin protocol alike. **The
//! in-memory rows are driven against comments seeded in their configuration instead, each
//! assertion read from the one invocation that made it, because an in-memory source's work
//! dies with the process that held it and no later command can read it back** — its
//! add-then-list within one process is proven as a library call, in
//! `crates/onetaskgraph-core/tests/comments.rs`.

use std::path::PathBuf;
use std::process::Output;

use serde_json::{Value, json};

use crate::common::{SOURCE_BOUNDARIES, Sandbox, stderr, stdout};
use crate::fixtures::{
    GITHUB_DRAFT_TASK, GitHubBoardFields, ROWS, Row, SOURCE, document, empty_folder,
    github_projects_with_board, github_projects_with_draft, qualified,
};
use onetaskgraph_plugin_api::Support;

/// The plugins whose source outlives one invocation, and the credentials each names when it
/// is hosted a process away — where §3.1 hands a plugin only what its configuration names.
const KEPT: &[(&str, &[&str])] = &[
    ("local-md", &[]),
    ("github-projects", &["GITHUB_PROJECTS_FIXTURE_TOKEN"]),
    ("linear", &["LINEAR_API_KEY"]),
];

/// The plugins whose work dies with the process that held it.
const TRANSIENT: &[&str] = &["in-memory", "subprocess"];

/// Every member a comment carries on the wire, and nothing else.
const COMMENT_MEMBERS: [&str; 6] = ["author", "body", "created_at", "id", "updated_at", "url"];

/// The task every journey comments on. Every row's source holds it.
const TASK: &str = "T-1";

fn run(sandbox: &Sandbox, arguments: &[&str], input: Option<&str>) -> Output {
    let mut command = sandbox.command();
    command.args(arguments);
    command.write_stdin(input.unwrap_or_default());
    command.assert().get_output().clone()
}

/// Standard output of a run that had to succeed, quoting stderr when it did not.
fn ok(who: &str, sandbox: &Sandbox, arguments: &[&str], input: Option<&str>) -> String {
    let output = run(sandbox, arguments, input);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{who}: `onetaskgraph {}` exited {:?}\n{}",
        arguments.join(" "),
        output.status.code(),
        stderr(&output)
    );
    stdout(&output)
}

/// Standard error of a run that had to fail, which must say what to do next.
fn refused(who: &str, sandbox: &Sandbox, arguments: &[&str], input: Option<&str>) -> String {
    let output = run(sandbox, arguments, input);
    let said = stderr(&output);
    assert_eq!(
        output.status.code(),
        Some(1),
        "{who}: `onetaskgraph {}` was expected to exit 1\n{}{said}",
        arguments.join(" "),
        stdout(&output),
    );
    assert!(
        said.contains("next:"),
        "{who}: a refusal names a next action:\n{said}"
    );
    said
}

fn parsed(who: &str, rendered: &str) -> Value {
    serde_json::from_str(rendered)
        .unwrap_or_else(|error| panic!("{who}: not one JSON document ({error}):\n{rendered}"))
}

/// A comment in exactly the shape C1 states: six members, `id` and `body` always strings,
/// every other member a string or `null`.
fn a_comment(who: &str, comment: &Value) {
    let mut members: Vec<&str> = comment
        .as_object()
        .unwrap_or_else(|| panic!("{who}: a comment is an object: {comment}"))
        .keys()
        .map(String::as_str)
        .collect();
    members.sort_unstable();
    assert_eq!(members, COMMENT_MEMBERS, "{who}: {comment}");
    assert!(
        comment["id"].as_str().is_some_and(|id| !id.is_empty()),
        "{who}: {comment}"
    );
    assert!(comment["body"].is_string(), "{who}: {comment}");
    for member in ["author", "created_at", "updated_at", "url"] {
        assert!(
            comment[member].is_string() || comment[member].is_null(),
            "{who}: `{member}` is a string or null: {comment}"
        );
    }
}

/// A file under the sandbox holding exactly `text`, for `--body-file`.
fn body_file(sandbox: &Sandbox, name: &str, text: &[u8]) -> PathBuf {
    let path = sandbox.subdirectory("bodies").join(name);
    std::fs::write(&path, text).expect("the body file");
    path
}

fn row_of(plugin: &str) -> &'static Row {
    ROWS.iter()
        .find(|row| row.plugin == plugin)
        .unwrap_or_else(|| panic!("the shared table has a {plugin} row"))
}

/// Every row of the shared table whose source says its tasks have comments.
fn commented_rows() -> impl Iterator<Item = &'static Row> {
    ROWS.iter()
        .filter(|row| row.declared().comments == Support::Native)
}

/// A sandbox configuring `row`'s own source as `work`, on the boundary given.
fn kept(row: &Row, boundary: crate::common::SourceBoundary) -> Sandbox {
    let secrets = KEPT
        .iter()
        .find(|(plugin, _)| *plugin == row.plugin)
        .map_or(&[][..], |(_, secrets)| *secrets);
    let sandbox = Sandbox::new();
    let block = (row.fixture.block)(&sandbox);
    sandbox.project_document(&document(&json!({
        SOURCE: boundary.source_with_secrets(row.plugin, block, secrets)
    })));
    sandbox
}

fn list(who: &str, sandbox: &Sandbox, task: &str) -> Value {
    parsed(
        who,
        &ok(
            who,
            sandbox,
            &["task", "comment", "list", task, "--json"],
            None,
        ),
    )
}

#[test]
fn every_row_whose_tasks_have_comments_is_driven_by_one_of_the_comment_journeys() {
    // The journeys below split the table in two by whether a later command can read a
    // source back. A row landing in neither list would declare comments and never be driven.
    for row in commented_rows() {
        assert!(
            KEPT.iter().any(|(plugin, _)| *plugin == row.plugin) || TRANSIENT.contains(&row.plugin),
            "{}: declares comments, and no comment journey drives it",
            row.name
        );
    }
    for (plugin, _) in KEPT {
        assert_eq!(
            row_of(plugin).declared().comments,
            Support::Native,
            "{plugin}: every plugin C1 names has comments"
        );
    }
}

#[test]
fn a_comment_added_through_the_binary_is_listed_edited_and_deleted_on_every_source_that_keeps_it() {
    for (plugin, _) in KEPT {
        let row = row_of(plugin);
        for boundary in SOURCE_BOUNDARIES {
            let who = format!("{} across the {boundary:?} boundary", row.name);
            let sandbox = kept(row, boundary);
            let task = qualified(SOURCE, TASK);

            // From a file, with a heading of its own and a trailing newline — both of which a
            // body that is trimmed or re-rendered on the way through would lose.
            let first_body = "Seen again on main:\n\n## Evidence\n\n`cargo test -p x` failed\n";
            let first_file = body_file(&sandbox, "first.md", first_body.as_bytes());
            let first = parsed(
                &who,
                &ok(
                    &who,
                    &sandbox,
                    &[
                        "task",
                        "comment",
                        "add",
                        &task,
                        "--body-file",
                        first_file.to_str().expect("a UTF-8 path"),
                        "--json",
                    ],
                    None,
                ),
            );
            a_comment(&who, &first);
            assert_eq!(first["body"], first_body, "{who}: stored byte for byte");

            // From standard input, ending in a blank line.
            let second_body = "A second note, from standard input.\n\n";
            let second = parsed(
                &who,
                &ok(
                    &who,
                    &sandbox,
                    &["task", "comment", "add", &task, "--json"],
                    Some(second_body),
                ),
            );
            a_comment(&who, &second);
            assert_eq!(second["body"], second_body, "{who}: stored byte for byte");
            assert_ne!(first["id"], second["id"], "{who}");

            // Listed by a later invocation, oldest first, exactly as each add answered.
            assert_eq!(
                list(&who, &sandbox, &task),
                json!({"comments": [first.clone(), second.clone()]}),
                "{who}"
            );

            // An edit moves that comment's body and time and nothing else of it — and nothing
            // of the comment beside it.
            let edited_body = "Seen again on main, twice.\n";
            let edit_file = body_file(&sandbox, "edit.md", edited_body.as_bytes());
            let first_id = first["id"].as_str().expect("an id").to_owned();
            let edited = parsed(
                &who,
                &ok(
                    &who,
                    &sandbox,
                    &[
                        "task",
                        "comment",
                        "edit",
                        &task,
                        &first_id,
                        "--body-file",
                        edit_file.to_str().expect("a UTF-8 path"),
                        "--json",
                    ],
                    None,
                ),
            );
            a_comment(&who, &edited);
            assert_eq!(edited["body"], edited_body, "{who}");
            for member in ["id", "author", "created_at", "url"] {
                assert_eq!(
                    edited[member], first[member],
                    "{who}: `{member}` is unmoved"
                );
            }
            assert!(edited["updated_at"].is_string(), "{who}: {edited}");
            assert_eq!(
                list(&who, &sandbox, &task),
                json!({"comments": [edited.clone(), second.clone()]}),
                "{who}"
            );

            // A delete removes that comment and only that one.
            assert_eq!(
                parsed(
                    &who,
                    &ok(
                        &who,
                        &sandbox,
                        &["task", "comment", "delete", &task, &first_id, "--json"],
                        None,
                    ),
                ),
                json!({"deleted": first_id}),
                "{who}"
            );
            assert_eq!(
                list(&who, &sandbox, &task),
                json!({"comments": [second.clone()]}),
                "{who}"
            );

            // `task show` carries what is left, beside the task rather than inside it.
            let shown = parsed(
                &who,
                &ok(&who, &sandbox, &["task", "show", &task, "--json"], None),
            );
            assert_eq!(shown["comments"], json!([second.clone()]), "{who}");
            assert_eq!(shown["items"][0]["id"], json!(task), "{who}");
            let rendered = ok(&who, &sandbox, &["task", "show", &task], None);
            let after_body = rendered
                .split_once("comments: 1")
                .unwrap_or_else(|| panic!("{who}: the comments follow the task:\n{rendered}"))
                .1;
            assert!(
                after_body.contains("A second note, from standard input."),
                "{who}:\n{rendered}"
            );
            let listed = ok(&who, &sandbox, &["task", "comment", "list", &task], None);
            assert!(
                listed.contains(second["id"].as_str().expect("an id"))
                    && listed.contains("A second note, from standard input."),
                "{who}:\n{listed}"
            );

            let deleted = ok(
                &who,
                &sandbox,
                &[
                    "task",
                    "comment",
                    "delete",
                    &task,
                    second["id"].as_str().expect("an id"),
                ],
                None,
            );
            assert!(deleted.contains("deleted comment"), "{who}: {deleted}");
            assert_eq!(
                list(&who, &sandbox, &task),
                json!({"comments": []}),
                "{who}"
            );
            assert!(
                ok(&who, &sandbox, &["task", "comment", "list", &task], None)
                    .contains("no comments"),
                "{who}"
            );
        }
    }
}

#[test]
fn a_folder_of_markdown_records_the_author_it_is_given() {
    for boundary in SOURCE_BOUNDARIES {
        let row = row_of("local-md");
        let who = format!("{} across the {boundary:?} boundary", row.name);
        let sandbox = kept(row, boundary);
        let task = qualified(SOURCE, TASK);
        let added = parsed(
            &who,
            &ok(
                &who,
                &sandbox,
                &["task", "comment", "add", &task, "--author", "ada", "--json"],
                Some("hello\n"),
            ),
        );
        assert_eq!(added["author"], "ada", "{who}");
        assert_eq!(
            list(&who, &sandbox, &task)["comments"][0]["author"],
            "ada",
            "{who}"
        );
        let rendered = ok(
            &who,
            &sandbox,
            &["task", "comment", "add", &task],
            Some("x"),
        );
        assert!(rendered.contains("comment:"), "{who}: {rendered}");
    }
}

#[test]
fn a_source_that_records_its_own_author_refuses_one_given_and_writes_nothing() {
    for plugin in ["github-projects", "linear"] {
        let row = row_of(plugin);
        for boundary in SOURCE_BOUNDARIES {
            let who = format!("{} across the {boundary:?} boundary", row.name);
            let sandbox = kept(row, boundary);
            let task = qualified(SOURCE, TASK);
            let said = refused(
                &who,
                &sandbox,
                &["task", "comment", "add", &task, "--author", "ada"],
                Some("posted under a name GitHub or Linear would not record\n"),
            );
            assert!(said.contains("author"), "{who}: {said}");
            assert_eq!(
                list(&who, &sandbox, &task),
                json!({"comments": []}),
                "{who}: nothing was posted"
            );
        }
    }
}

/// Three comments on `T-1`, the first with an author and a body of its own headings.
fn seeded_comments() -> Value {
    json!([
        {"task": TASK, "comment": {"id": "C-1", "author": "ada",
            "created_at": "2026-09-13T15:11:07Z", "updated_at": "2026-09-13T15:11:07Z",
            "body": "Seeded evidence:\n\n## heading\n", "url": null}},
        {"task": TASK, "comment": {"id": "C-2", "author": null,
            "created_at": "2026-09-13T15:12:00Z", "updated_at": "2026-09-13T15:12:00Z",
            "body": "second", "url": null}},
        {"task": TASK, "comment": {"id": "C-3", "author": "grace",
            "created_at": "2026-09-13T15:13:00Z", "updated_at": "2026-09-13T15:13:00Z",
            "body": "third\n", "url": "https://example.invalid/C-3"}}
    ])
}

/// `row`'s own configuration with [`seeded_comments`] in it, wherever that row keeps its
/// in-memory source's settings.
fn seeded(row: &Row, sandbox: &Sandbox) -> Value {
    let mut block = (row.fixture.block)(sandbox);
    let settings = if row.plugin == "subprocess" {
        &mut block["settings"]
    } else {
        &mut block
    };
    settings["comments"] = seeded_comments();
    json!({"plugin": row.plugin, "config": block})
}

#[test]
fn an_in_memory_source_lists_edits_and_deletes_the_comments_it_holds_in_one_invocation_each() {
    let held: Vec<Value> = seeded_comments()
        .as_array()
        .expect("a list")
        .iter()
        .map(|seed| seed["comment"].clone())
        .collect();
    for row in ROWS.iter().filter(|row| TRANSIENT.contains(&row.plugin)) {
        let who = row.name;
        let sandbox = Sandbox::new();
        sandbox.project_document(&document(&json!({ SOURCE: seeded(row, &sandbox) })));
        let task = qualified(SOURCE, TASK);

        // Listed oldest first and byte for byte — across more than one page on the row that
        // serves two rows at a time.
        let listed = list(who, &sandbox, &task);
        assert_eq!(listed, json!({"comments": held.clone()}), "{who}");
        for comment in &held {
            a_comment(who, comment);
        }

        let added = parsed(
            who,
            &ok(
                who,
                &sandbox,
                &[
                    "task", "comment", "add", &task, "--author", "hopper", "--json",
                ],
                Some("Added from standard input.\n\n## and a heading\n"),
            ),
        );
        a_comment(who, &added);
        assert_eq!(
            added["body"], "Added from standard input.\n\n## and a heading\n",
            "{who}"
        );
        assert_eq!(added["author"], "hopper", "{who}");
        assert_eq!(added["id"], "C-4", "{who}");

        let edit = body_file(&sandbox, "edit.md", b"Seeded evidence, corrected.\n");
        let edited = parsed(
            who,
            &ok(
                who,
                &sandbox,
                &[
                    "task",
                    "comment",
                    "edit",
                    &task,
                    "C-1",
                    "--body-file",
                    edit.to_str().expect("a UTF-8 path"),
                    "--json",
                ],
                None,
            ),
        );
        assert_eq!(edited["body"], "Seeded evidence, corrected.\n", "{who}");
        for member in ["id", "author", "created_at", "url"] {
            assert_eq!(
                edited[member], held[0][member],
                "{who}: `{member}` is unmoved"
            );
        }
        assert_ne!(edited["updated_at"], held[0]["updated_at"], "{who}");

        assert_eq!(
            parsed(
                who,
                &ok(
                    who,
                    &sandbox,
                    &["task", "comment", "delete", &task, "C-2", "--json"],
                    None,
                ),
            ),
            json!({"deleted": "C-2"}),
            "{who}"
        );

        let shown = parsed(
            who,
            &ok(who, &sandbox, &["task", "show", &task, "--json"], None),
        );
        assert_eq!(shown["comments"], json!(held.clone()), "{who}");
    }
}

#[test]
fn task_show_says_nothing_about_comments_for_a_source_whose_tasks_have_none() {
    let row = row_of("in-memory");
    let sandbox = Sandbox::new();
    let mut block = (row.fixture.block)(&sandbox);
    block["capabilities"]
        .as_object_mut()
        .expect("a capability block")
        .remove("comments");
    sandbox.project_document(&document(&json!({
        SOURCE: {"plugin": row.plugin, "config": block}
    })));
    let task = qualified(SOURCE, TASK);

    let shown = parsed(
        row.name,
        &ok(row.name, &sandbox, &["task", "show", &task, "--json"], None),
    );
    assert!(
        shown.get("comments").is_none(),
        "absent rather than an empty list, which would read as a source holding none: {shown}"
    );
    let rendered = ok(row.name, &sandbox, &["task", "show", &task], None);
    assert!(!rendered.contains("comments"), "{rendered}");

    // And a source that has comments but none on this task says so, both ways.
    let local = row_of("local-md");
    let sandbox = kept(local, crate::common::SourceBoundary::Direct);
    let shown = parsed(
        local.name,
        &ok(
            local.name,
            &sandbox,
            &["task", "show", &task, "--json"],
            None,
        ),
    );
    assert_eq!(shown["comments"], json!([]), "{shown}");
    assert!(ok(local.name, &sandbox, &["task", "show", &task], None).contains("comments: none"));
}

#[test]
fn a_task_or_a_comment_that_is_not_there_is_refused_by_name_on_every_row() {
    for row in commented_rows() {
        let who = row.name;
        let sandbox = Sandbox::new();
        let block = (row.fixture.block)(&sandbox);
        let secrets = KEPT
            .iter()
            .find(|(plugin, _)| *plugin == row.plugin)
            .map_or(&[][..], |(_, secrets)| *secrets);
        sandbox.project_document(&document(&json!({
            SOURCE: crate::common::SourceBoundary::Direct
                .source_with_secrets(row.plugin, block, secrets)
        })));
        let missing = qualified(SOURCE, "T-404");
        for arguments in [
            vec!["task", "comment", "list", missing.as_str()],
            vec!["task", "comment", "add", missing.as_str()],
            vec!["task", "comment", "delete", missing.as_str(), "C-1"],
        ] {
            let said = refused(who, &sandbox, &arguments, Some("a body\n"));
            assert!(
                said.contains(&format!("no task with the id {missing}")),
                "{who}: {said}"
            );
        }

        let task = qualified(SOURCE, TASK);
        for arguments in [
            vec!["task", "comment", "edit", task.as_str(), "NO-SUCH-COMMENT"],
            vec![
                "task",
                "comment",
                "delete",
                task.as_str(),
                "NO-SUCH-COMMENT",
            ],
        ] {
            let said = refused(who, &sandbox, &arguments, Some("a body\n"));
            assert!(
                said.contains(&format!(
                    "task {task} has no comment with the id NO-SUCH-COMMENT"
                )),
                "{who}: {said}"
            );
        }
    }
}

#[test]
fn an_empty_or_unreadable_body_is_refused_before_anything_is_written() {
    let row = row_of("local-md");
    let who = row.name;
    let sandbox = kept(row, crate::common::SourceBoundary::Direct);
    let task = qualified(SOURCE, TASK);

    let said = refused(who, &sandbox, &["task", "comment", "add", &task], Some(""));
    assert!(said.contains("is empty"), "{said}");
    let empty = body_file(&sandbox, "empty.md", b"");
    let said = refused(
        who,
        &sandbox,
        &[
            "task",
            "comment",
            "add",
            &task,
            "--body-file",
            empty.to_str().expect("a UTF-8 path"),
        ],
        None,
    );
    assert!(said.contains("is empty"), "{said}");
    let binary = body_file(&sandbox, "binary.md", &[0xff, 0xfe, 0x00]);
    let said = refused(
        who,
        &sandbox,
        &[
            "task",
            "comment",
            "add",
            &task,
            "--body-file",
            binary.to_str().expect("a UTF-8 path"),
        ],
        None,
    );
    assert!(said.contains("not UTF-8"), "{said}");
    let said = refused(
        who,
        &sandbox,
        &[
            "task",
            "comment",
            "add",
            &task,
            "--body-file",
            "no/such/file.md",
        ],
        None,
    );
    assert!(said.contains("could not read it"), "{said}");
    assert_eq!(list(who, &sandbox, &task), json!({"comments": []}));

    let added = parsed(
        who,
        &ok(
            who,
            &sandbox,
            &["task", "comment", "add", &task, "--json"],
            Some("kept\n"),
        ),
    );
    let said = refused(
        who,
        &sandbox,
        &[
            "task",
            "comment",
            "edit",
            &task,
            added["id"].as_str().expect("an id"),
        ],
        Some(""),
    );
    assert!(said.contains("is empty"), "{said}");
    assert_eq!(list(who, &sandbox, &task)["comments"][0]["body"], "kept\n");
}

#[test]
fn a_stdio_plugin_whose_handshake_says_nothing_of_comments_is_refused_before_anything_is_read() {
    // The document store beside this module was written before there were comments, so its
    // handshake does not mention them — which the protocol reads as a source without any.
    // Its log records every method it is asked for, so "refused before anything is read"
    // is the log holding the handshake and nothing after it.
    let sandbox = Sandbox::new();
    let store = sandbox.subdirectory("store");
    let log = store.join("asked.log");
    sandbox.project_document(&document(&json!({
        "store": {"plugin": "subprocess", "config": {
            "command": crate::document_store::interpreter().to_string_lossy(),
            "args": [crate::document_store::peer().to_string_lossy()],
            "settings": {"store": store.join("store.json"), "log": log},
        }},
    })));
    let task = qualified("store", "D-1");
    for arguments in [
        vec!["task", "comment", "list", task.as_str()],
        vec!["task", "comment", "add", task.as_str()],
        vec!["task", "comment", "edit", task.as_str(), "C-1"],
        vec!["task", "comment", "delete", task.as_str(), "C-1"],
    ] {
        let said = refused("document-store", &sandbox, &arguments, Some("a body\n"));
        assert!(
            said.contains("source store has no comments: its plugin is document-store"),
            "{} names the source and its plugin:\n{said}",
            arguments.join(" ")
        );
        let asked = std::fs::read_to_string(&log).unwrap_or_default();
        assert!(
            asked.lines().all(|method| method == "initialize"),
            "{}: nothing but the handshake reached the plugin:\n{asked}",
            arguments.join(" ")
        );
    }
}

#[test]
fn comments_a_source_can_read_but_not_write_are_listed_and_every_write_is_refused() {
    let row = row_of("in-memory");
    let sandbox = Sandbox::new();
    let mut source = seeded(row, &sandbox);
    source["config"]["capabilities"]["writes"] = json!("unsupported");
    sandbox.project_document(&document(&json!({ SOURCE: source })));
    let task = qualified(SOURCE, TASK);

    assert_eq!(
        list(row.name, &sandbox, &task)["comments"]
            .as_array()
            .map(Vec::len),
        Some(3)
    );
    for arguments in [
        vec!["task", "comment", "add", task.as_str()],
        vec!["task", "comment", "edit", task.as_str(), "C-1"],
        vec!["task", "comment", "delete", task.as_str(), "C-1"],
    ] {
        let said = refused(row.name, &sandbox, &arguments, Some("a body\n"));
        assert!(
            said.contains("source work cannot be written: its plugin is in-memory"),
            "{said}"
        );
    }
}

#[test]
fn a_github_draft_issue_is_refused_for_every_comment_verb_and_still_shows() {
    for boundary in SOURCE_BOUNDARIES {
        let who = format!("github-projects draft across the {boundary:?} boundary");
        let sandbox = Sandbox::new();
        let config = github_projects_with_draft(&sandbox);
        sandbox.project_document(&document(&json!({
            SOURCE: boundary.source_with_secrets(
                "github-projects",
                config,
                &["GITHUB_PROJECTS_FIXTURE_TOKEN"],
            )
        })));
        let draft = qualified(SOURCE, GITHUB_DRAFT_TASK);

        for arguments in [
            vec!["task", "comment", "list", draft.as_str()],
            vec!["task", "comment", "add", draft.as_str()],
            vec!["task", "comment", "edit", draft.as_str(), "C-1"],
            vec!["task", "comment", "delete", draft.as_str(), "C-1"],
        ] {
            let said = refused(&who, &sandbox, &arguments, Some("a body\n"));
            assert!(
                said.contains("draft"),
                "{who}: `{}` says the item is a draft:\n{said}",
                arguments.join(" ")
            );
        }

        // A draft is still a task a person can read: it carries no comments to show, which
        // is not a source failing.
        let shown = parsed(
            &who,
            &ok(&who, &sandbox, &["task", "show", &draft, "--json"], None),
        );
        assert_eq!(shown["items"][0]["id"], json!(draft), "{who}");
        assert!(shown.get("comments").is_none(), "{who}: {shown}");
        assert_eq!(shown["errors"], json!([]), "{who}: {shown}");
    }
}

/// The documents a GitHub board is sent when a comment is read or written, as the plugin
/// itself spells them — so a copy that reached a comment at either end cannot hide it behind
/// a differently spelled request.
const GITHUB_COMMENT_DOCUMENTS: [&str; 5] = [
    onetaskgraph_github_projects::graphql::ISSUE_COMMENTS,
    onetaskgraph_github_projects::graphql::COMMENT_ISSUE,
    onetaskgraph_github_projects::graphql::ADD_COMMENT,
    onetaskgraph_github_projects::graphql::UPDATE_COMMENT,
    onetaskgraph_github_projects::graphql::DELETE_COMMENT,
];

/// The comment documents `board` received after its first `since` documents.
fn comment_calls(board: &GitHubBoardFields, since: usize) -> Vec<String> {
    board.documents()[since..]
        .iter()
        .filter(|document| GITHUB_COMMENT_DOCUMENTS.contains(&document.as_str()))
        .cloned()
        .collect()
}

/// Two GitHub boards over the shared dataset — `work` to copy out of and `board` to copy into —
/// and a handle on what each board is sent.
///
/// A board refuses to create an item carrying labels, and every item of the shared dataset
/// carries one, so the task and the project copied between the two are written onto `work`
/// first out of a folder of Markdown that labels neither: `notes:N-1`, filed under
/// `notes:NP-1`, and `notes:N-2`, filed under nothing — a task copied on its own has to be
/// one, because a destination refuses a task filed under a project it does not hold. Then
/// `work`'s copies of both tasks and `board`'s own `T-1` each get a comment, so both ends of
/// every copy below hold tasks with comments.
struct Boards {
    sandbox: Sandbox,
    from: GitHubBoardFields,
    into: GitHubBoardFields,
    /// `work`'s own id for the task filed under [`Self::project`].
    task: String,
    /// `work`'s own id for the task filed under nothing.
    orphan: String,
    /// `work`'s own id for the project it is filed under.
    project: String,
}

fn two_commented_boards(who: &str) -> Boards {
    let sandbox = Sandbox::new();
    let (from_config, from) = github_projects_with_board(&sandbox);
    let (into_config, into) = github_projects_with_board(&sandbox);
    let notes = empty_folder(&sandbox, "notes");
    let folder = sandbox.subdirectory("notes");
    std::fs::create_dir_all(folder.join("tasks")).expect("the folder's tasks");
    std::fs::create_dir_all(folder.join("projects")).expect("the folder's projects");
    std::fs::write(
        folder.join("projects/NP-1.md"),
        "---\ntitle: Unlabelled project\nstatus: todo\n---\na project with no labels\n",
    )
    .expect("the folder's project");
    std::fs::write(
        folder.join("tasks/N-1.md"),
        "---\ntitle: Unlabelled task\nstatus: todo\nproject: NP-1\n---\na task with no labels\n",
    )
    .expect("the folder's task");
    std::fs::write(
        folder.join("tasks/N-2.md"),
        "---\ntitle: Unfiled task\nstatus: todo\n---\na task in no project\n",
    )
    .expect("the folder's unfiled task");
    sandbox.project_document(&document(&json!({
        SOURCE: {"plugin": "github-projects", "config": from_config},
        "board": {"plugin": "github-projects", "config": into_config},
        "notes": {"plugin": "local-md", "config": notes},
    })));

    let written = parsed(
        who,
        &ok(
            who,
            &sandbox,
            &["project", "copy", "notes:NP-1", "--to", SOURCE, "--json"],
            None,
        ),
    );
    let landed = |source: &str| {
        written["items"]
            .as_array()
            .expect("a copy reports its items")
            .iter()
            .find(|item| item["source"] == source)
            .and_then(|item| item["destination"].as_str())
            .unwrap_or_else(|| panic!("{who}: {source} landed on `work`: {written}"))
            .to_owned()
    };
    let (project, task) = (landed("notes:NP-1"), landed("notes:N-1"));
    let unfiled = parsed(
        who,
        &ok(
            who,
            &sandbox,
            &["task", "copy", "notes:N-2", "--to", SOURCE, "--json"],
            None,
        ),
    );
    let orphan = unfiled["items"][0]["destination"]
        .as_str()
        .unwrap_or_else(|| panic!("{who}: notes:N-2 landed on `work`: {unfiled}"))
        .to_owned();

    for commented in [&task, &orphan] {
        ok(
            who,
            &sandbox,
            &["task", "comment", "add", commented],
            Some("evidence on the source\n"),
        );
    }
    ok(
        who,
        &sandbox,
        &["task", "comment", "add", &qualified("board", TASK)],
        Some("evidence on the destination\n"),
    );
    Boards {
        sandbox,
        from,
        into,
        task,
        orphan,
        project,
    }
}

/// Assert a copy made no comment call at either board, left the comments of `source_task` and
/// of `board:T-1` as they were, and carried no comment of `source_task` onto what it wrote.
fn a_copy_touched_no_comment(who: &str, boards: &Boards, source_task: &str, arguments: &[&str]) {
    let sandbox = &boards.sandbox;
    let before = (
        list(who, sandbox, source_task),
        list(who, sandbox, &qualified("board", TASK)),
    );
    let (from_seen, into_seen) = (boards.from.documents().len(), boards.into.documents().len());

    let report = parsed(who, &ok(who, sandbox, arguments, None));

    assert_eq!(
        comment_calls(&boards.from, from_seen),
        Vec::<String>::new(),
        "{who}: the copy read or wrote a comment at its source"
    );
    assert_eq!(
        comment_calls(&boards.into, into_seen),
        Vec::<String>::new(),
        "{who}: the copy read or wrote a comment at its destination"
    );
    assert_eq!(
        (
            list(who, sandbox, source_task),
            list(who, sandbox, &qualified("board", TASK)),
        ),
        before,
        "{who}: both ends' comments are as they were"
    );
    let items = report["items"]
        .as_array()
        .expect("a copy reports its items");
    assert!(!items.is_empty(), "{who}: {report}");
    for item in items {
        // Only a task written can carry a comment; a project or a document has none to carry.
        if item["source"] != json!(source_task) {
            continue;
        }
        let destination = item["destination"].as_str().expect("a copy that wrote");
        assert_eq!(
            list(who, sandbox, destination),
            json!({"comments": []}),
            "{who}: {destination} carries no comment of the task it was copied from"
        );
    }
}

#[test]
fn a_task_copy_neither_reads_nor_writes_a_comment_at_either_end() {
    let who = "task copy between two GitHub boards";
    let boards = two_commented_boards(who);
    let task = boards.orphan.clone();
    a_copy_touched_no_comment(
        who,
        &boards,
        &task,
        &["task", "copy", &task, "--to", "board", "--json"],
    );
}

#[test]
fn a_project_copy_neither_reads_nor_writes_a_comment_at_either_end() {
    let who = "project copy between two GitHub boards";
    let boards = two_commented_boards(who);
    let (project, task) = (boards.project.clone(), boards.task.clone());
    a_copy_touched_no_comment(
        who,
        &boards,
        &task,
        &["project", "copy", &project, "--to", "board", "--json"],
    );
}

#[test]
fn a_document_copy_neither_reads_nor_writes_a_comment_at_either_end() {
    // `D-3` is the one document of the shared board that carries no labels.
    let who = "document copy between two GitHub boards";
    let boards = two_commented_boards(who);
    let task = boards.task.clone();
    a_copy_touched_no_comment(
        who,
        &boards,
        &task,
        &[
            "document",
            "copy",
            &qualified(SOURCE, "D-3"),
            "--to",
            "board",
            "--json",
        ],
    );
}

#[test]
fn a_copy_into_a_markdown_task_keeps_its_comments_section_byte_for_byte() {
    // The folder already holds the copied task, recorded as coming from `work:T-1`, with a
    // comment section a person left on it. The copy updates that file from a board whose own
    // `T-1` has a comment too — and neither the folder's comment nor the board's may move.
    let who = "task copy from a GitHub board into a folder of Markdown";
    let sandbox = Sandbox::new();
    let (from, from_board) = github_projects_with_board(&sandbox);
    let notes = empty_folder(&sandbox, "notes");
    let tasks = sandbox.subdirectory("notes/tasks");
    let section = "## Comments\n\n\
        <!-- onetaskgraph:comment id=\"20260913T151107Z-1\" author=\"ada\" \
        created_at=\"2026-09-13T15:11:07Z\" updated_at=\"2026-09-13T15:11:07Z\" -->\n\
        ### ada — 2026-09-13T15:11:07Z\n\nkept across the copy\n\n\
        <!-- /onetaskgraph:comment -->\n";
    let file = tasks.join("T-1.md");
    std::fs::write(
        &file,
        format!(
            "---\ntitle: Alpha engine\nstatus: Todo\nmetadata:\n  onetaskgraph.origin: \
             work:T-1\n---\nwritten before the copy\n\n{section}"
        ),
    )
    .expect("the folder's task");
    sandbox.project_document(&document(&json!({
        SOURCE: {"plugin": "github-projects", "config": from},
        "notes": {"plugin": "local-md", "config": notes},
    })));
    ok(
        who,
        &sandbox,
        &["task", "comment", "add", &qualified(SOURCE, TASK)],
        Some("evidence the copy must not carry\n"),
    );
    let seen = from_board.documents().len();

    let report = parsed(
        who,
        &ok(
            who,
            &sandbox,
            &[
                "task",
                "copy",
                &qualified(SOURCE, TASK),
                "--to",
                "notes",
                "--json",
            ],
            None,
        ),
    );
    assert_eq!(report["items"][0]["action"], "updated", "{who}: {report}");
    assert_eq!(
        comment_calls(&from_board, seen),
        Vec::<String>::new(),
        "{who}: the copy read a comment at its source"
    );

    let written = std::fs::read_to_string(&file).expect("the folder's task");
    assert!(
        written.ends_with(section),
        "{who}: the section is kept byte for byte:\n{written}"
    );
    assert!(
        !written.contains("written before the copy"),
        "{who}: the content above it was updated:\n{written}"
    );
    assert!(
        !written.contains("evidence the copy must not carry"),
        "{who}: no source comment entered the file:\n{written}"
    );
    let listed = list(who, &sandbox, &qualified("notes", TASK));
    assert_eq!(
        listed["comments"].as_array().map(|comments| comments
            .iter()
            .map(|c| c["body"].clone())
            .collect::<Vec<_>>()),
        Some(vec![json!("kept across the copy")]),
        "{who}: {listed}"
    );
}
