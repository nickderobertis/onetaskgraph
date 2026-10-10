//! A copy into a folder of Markdown is a consistent snapshot of what it read.
//!
//! Each journey has the source change an item at one of the points a copy reads it — between
//! the listing and the item's own read, between that read and the further pages of its
//! comments, and again while the copy reads it a second time — and holds what landed to being
//! exactly one revision the source really held, as the source's own state at that revision
//! shows: never a body from one revision beside comments from another. A board and a Linear
//! workspace are changed through their loopbacks part way through a command; a folder of
//! Markdown, which has no server to wait on, is changed by `rewriting_relay.py`, a relay in
//! front of the shipped plugin host.

use std::path::{Path, PathBuf};
use std::process::Output;

use onetaskgraph_github_projects::graphql;
use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::document_store::interpreter;
use crate::fixtures::{
    GitHubBoardFields, LinearWorkspace, dataset, document, github_projects_with_board,
    linear_workspace,
};

/// More comments than one page of a hosted board's, so a task's comments take two reads.
const BEYOND_A_PAGE: usize = 120;

/// More comments than one page of a folder of Markdown's.
const BEYOND_A_FOLDER_PAGE: usize = 201;

fn run(sandbox: &Sandbox, arguments: &[&str]) -> Output {
    sandbox
        .command()
        .args(arguments)
        .write_stdin("")
        .assert()
        .get_output()
        .clone()
}

/// The JSON a run printed, having exited `code`.
fn answered(sandbox: &Sandbox, arguments: &[&str], code: i32) -> Value {
    let output = run(sandbox, arguments);
    assert_eq!(
        output.status.code(),
        Some(code),
        "`onetaskgraph {}`:\n{}",
        arguments.join(" "),
        stderr(&output)
    );
    serde_json::from_str(&stdout(&output)).expect("machine output is JSON")
}

/// What one task reads as, by a process of its own: its content and its comments' bodies,
/// oldest first.
fn revision(sandbox: &Sandbox, id: &str) -> (Value, Vec<Value>) {
    let shown = answered(sandbox, &["task", "show", id, "--json"], 0);
    (
        shown["items"][0]["item"]["content"].clone(),
        shown["comments"]
            .as_array()
            .expect("comments")
            .iter()
            .map(|comment| comment["body"].clone())
            .collect(),
    )
}

/// One copy by query into the folder `notes`, answering with its report.
fn snapshot(sandbox: &Sandbox, from: &str, code: i32) -> Value {
    answered(
        sandbox,
        &[
            "task",
            "copy",
            "--to",
            "notes",
            "--source",
            from,
            "--comments",
            "--no-link",
            "--json",
        ],
        code,
    )
}

/// The outcome the report gives one source item.
fn outcome<'a>(report: &'a Value, source: &str) -> &'a Value {
    report["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|item| item["source"] == source)
        .unwrap_or_else(|| panic!("{source} is reported: {report:#}"))
}

fn landed(report: &Value, source: &str) -> String {
    outcome(report, source)["destination"]
        .as_str()
        .unwrap_or_else(|| panic!("{source} landed: {report:#}"))
        .to_owned()
}

/// The token `task list` reports for one task now.
fn token(sandbox: &Sandbox, source: &str, id: &str) -> Value {
    answered(
        sandbox,
        &[
            "task", "list", "--source", source, "--json", "--limit", "100",
        ],
        0,
    )["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|task| task["id"] == id)
        .expect("the task is listed")["item"]["version"]
        .clone()
}

fn recorded(sandbox: &Sandbox, id: &str) -> Value {
    answered(sandbox, &["task", "show", id, "--no-comments", "--json"], 0)["items"][0]["item"]
        ["metadata"]["onetaskgraph.copied_version"]
        .clone()
}

/// A board configured as `board` and an empty folder as `notes`.
fn board() -> (Sandbox, GitHubBoardFields) {
    let sandbox = Sandbox::new();
    let (config, board) = github_projects_with_board(&sandbox);
    let notes = sandbox.subdirectory("notes");
    std::fs::create_dir_all(notes.join("tasks")).expect("the folder");
    sandbox.project_document(&document(&json!({
        "board": {"plugin": "github-projects", "config": config},
        "notes": {"plugin": "local-md", "config": {"root": notes, "status_mapping": {
            "Todo": "todo", "Doing": "in-progress", "Shipped": "done", "Done": "done"}}},
    })));
    for task in ["T-1", "T-2", "T-3", "T-4"] {
        board.set_labels(task, &[]);
    }
    (sandbox, board)
}

/// Whether one board request is the read of `id`'s first page of comments: its detail read,
/// alone or in a batch.
fn first_page_of(document: &str, variables: &Value, id: &str) -> bool {
    let naming = |variables: &Value| {
        variables
            .as_object()
            .is_some_and(|members| members.values().any(|value| value == id))
    };
    (document == graphql::ISSUE_DETAILS && variables["comments"] == true && naming(variables))
        || (document == graphql::ISSUE_DETAIL
            && variables["id"] == id
            && variables["after"].is_null())
}

#[test]
fn a_board_task_changed_after_its_listing_lands_at_the_revision_its_read_found() {
    let (sandbox, board) = board();
    board.add_comment("T-3", "said before");
    board.after_request(
        |document, _| document == graphql::BOARD || document == graphql::SEARCH_ISSUES,
        0,
        |board| board.edit_body("T-3", "edited after the listing"),
    );
    let report = snapshot(&sandbox, "board", 0);
    let copy = landed(&report, "board:T-3");
    assert_eq!(revision(&sandbox, &copy), revision(&sandbox, "board:T-3"));
    assert_eq!(
        recorded(&sandbox, &copy),
        token(&sandbox, "board", "board:T-3"),
        "the record names the revision the copy read, never the listing's"
    );
}

#[test]
fn a_board_task_changed_between_its_pages_is_read_again_once_and_lands_whole() {
    type Change = Box<dyn FnOnce(&GitHubBoardFields) + Send>;
    let cases: Vec<(&str, Box<dyn Fn(&str) -> Change>)> = vec![
        (
            "a comment edited",
            Box::new(|first: &str| {
                let first = first.to_owned();
                Box::new(move |board: &GitHubBoardFields| board.edit_comment(&first, "edited"))
            }),
        ),
        (
            "a comment added",
            Box::new(|_: &str| {
                Box::new(|board: &GitHubBoardFields| {
                    board.add_comment("T-3", "added part way");
                })
            }),
        ),
        (
            "the body edited",
            Box::new(|_: &str| {
                Box::new(|board: &GitHubBoardFields| board.edit_body("T-3", "a new body"))
            }),
        ),
    ];
    for (kind, change) in cases {
        let (sandbox, board) = board();
        let first = board.add_comment("T-3", "comment 0");
        for n in 1..BEYOND_A_PAGE {
            board.add_comment("T-3", &format!("comment {n}"));
        }
        board.after_request(
            |document, variables| first_page_of(document, variables, "T-3"),
            0,
            change(&first),
        );
        let mark = board.served().len();
        let report = snapshot(&sandbox, "board", 0);
        assert_eq!(
            outcome(&report, "board:T-3")["action"],
            "created",
            "{kind}: {report:#}"
        );
        let reads = board.served()[mark..]
            .iter()
            .filter(|(document, variables)| first_page_of(document, variables, "T-3"))
            .count();
        assert_eq!(reads, 2, "{kind}: the item is read once, then once again");
        let copy = landed(&report, "board:T-3");
        assert_eq!(
            revision(&sandbox, &copy),
            revision(&sandbox, "board:T-3"),
            "{kind}: the later revision, whole"
        );
        assert_eq!(
            recorded(&sandbox, &copy),
            token(&sandbox, "board", "board:T-3"),
            "{kind}"
        );
    }
}

#[test]
fn a_board_task_that_changes_again_while_it_is_read_again_is_written_nowhere() {
    let (sandbox, board) = board();
    for n in 0..BEYOND_A_PAGE {
        board.add_comment("T-3", &format!("comment {n}"));
    }
    board.after_request(
        |document, variables| first_page_of(document, variables, "T-3"),
        0,
        |board| board.edit_body("T-3", "the first change"),
    );
    board.after_request(
        |document, variables| first_page_of(document, variables, "T-3"),
        1,
        |board| board.edit_body("T-3", "the second change"),
    );
    let mark = board.served().len();
    let report = snapshot(&sandbox, "board", 4);
    let failed = outcome(&report, "board:T-3");
    assert_eq!(failed["action"], "failed", "{report:#}");
    assert_eq!(
        failed["error"]["kind"], "changed-while-copied",
        "{failed:#}"
    );
    assert!(failed.get("destination").is_none(), "{failed:#}");
    let reads = board.served()[mark..]
        .iter()
        .filter(|(document, variables)| first_page_of(document, variables, "T-3"))
        .count();
    assert_eq!(reads, 2, "read once, then once again, and no more");
    for other in ["board:T-1", "board:T-2", "board:T-4"] {
        assert_eq!(
            outcome(&report, other)["action"],
            "created",
            "{other} lands: {report:#}"
        );
    }
    let held = answered(
        &sandbox,
        &["task", "list", "--source", "notes", "--json"],
        0,
    );
    assert_eq!(
        held["items"].as_array().expect("items").len(),
        3,
        "nothing was written for the item that would not hold still: {held:#}"
    );
}

#[test]
fn a_copy_into_a_folder_is_a_snapshot_nothing_but_another_copy_changes() {
    let (sandbox, board) = board();
    let said = board.add_comment("T-3", "as it was");
    let report = snapshot(&sandbox, "board", 0);
    let copy = landed(&report, "board:T-3");
    let file = sandbox
        .project()
        .join("notes/tasks")
        .join(format!("{}.md", copy.split_once(':').expect("qualified").1));
    let before = std::fs::read(&file).expect("the copied file");

    board.edit_comment(&said, "as it is now");
    board.edit_body("T-3", "a later body");
    // Another process reads the board's changed items.
    revision(&sandbox, "board:T-3");
    answered(
        &sandbox,
        &["task", "list", "--source", "board", "--json"],
        0,
    );
    assert_eq!(
        std::fs::read(&file).expect("the copied file"),
        before,
        "reading the board writes nothing into the folder"
    );

    snapshot(&sandbox, "board", 0);
    assert_ne!(std::fs::read(&file).expect("the copied file"), before);
    assert_eq!(revision(&sandbox, &copy), revision(&sandbox, "board:T-3"));
}

/// A Linear workspace over the shared dataset whose `T-3` holds more than one page of
/// comments, configured as `linear`, beside an empty folder as `notes`.
fn linear() -> (Sandbox, LinearWorkspace) {
    let sandbox = Sandbox::new();
    let mut held = dataset();
    held["comments"] = Value::Array(
        (0..BEYOND_A_PAGE)
            .map(|n| {
                json!({"task": "T-3", "comment": {
                    "id": format!("C-{n}"), "body": format!("comment {n}"),
                    "created_at": format!("2026-08-01T{:02}:{:02}:00Z", n / 60, n % 60),
                    "updated_at": format!("2026-08-01T{:02}:{:02}:00Z", n / 60, n % 60),
                    "author": "a person"}})
            })
            .collect(),
    );
    let (config, workspace) = linear_workspace(&sandbox, held);
    let notes = sandbox.subdirectory("notes");
    std::fs::create_dir_all(notes.join("tasks")).expect("the folder");
    sandbox.project_document(&document(&json!({
        "linear": {"plugin": "linear", "config": config},
        "notes": {"plugin": "local-md", "config": {"root": notes, "status_mapping": {
            "Todo": "todo", "Doing": "in-progress", "Shipped": "done", "Done": "done",
            "Backlog": "backlog"}}},
    })));
    (sandbox, workspace)
}

#[test]
fn a_linear_task_changed_at_each_point_a_copy_reads_it_lands_as_one_revision() {
    type Change = Box<dyn FnOnce(&LinearWorkspace) + Send>;
    let listing = |document: &str, _: &Value| document == onetaskgraph_linear::graphql::ISSUES;
    let first_page = |document: &str, variables: &Value| {
        document == onetaskgraph_linear::graphql::ISSUE_DETAIL
            && variables["id"] == "T-3"
            && variables["before"].is_null()
    };
    type Point = fn(&str, &Value) -> bool;
    let cases: Vec<(&str, Point, Change, bool)> = vec![
        (
            "after its listing, the body edited",
            listing,
            Box::new(|workspace| workspace.edit_description("T-3", "edited after the listing")),
            true,
        ),
        (
            "between its pages, the oldest comment edited",
            first_page,
            Box::new(|workspace| workspace.edit_comment("C-0", "the oldest, edited")),
            false,
        ),
        (
            "between its pages, a comment added",
            first_page,
            Box::new(|workspace| {
                workspace.add_comment("T-3", "added part way");
            }),
            false,
        ),
        (
            "between its pages, the body edited",
            first_page,
            Box::new(|workspace| workspace.edit_description("T-3", "a new body")),
            true,
        ),
    ];
    for (kind, point, change, later) in cases {
        let (sandbox, workspace) = linear();
        let before = revision(&sandbox, "linear:T-3");
        workspace.after_request(point, 0, change);
        let report = snapshot(&sandbox, "linear", 0);
        let copy = landed(&report, "linear:T-3");
        let after = revision(&sandbox, "linear:T-3");
        let written = revision(&sandbox, &copy);
        assert!(
            written == before || written == after,
            "{kind}: one revision Linear held, never a mix: {written:?}"
        );
        if later {
            assert_eq!(written, after, "{kind}: the change was seen and read whole");
        }
    }
}

/// A folder of Markdown configured as `src` behind `rewriting_relay.py`, whose one task `A`
/// holds `comments` comments and is rewritten to `rewritten` once the host has answered
/// `after`, beside an empty folder as `notes`.
fn relayed(after: &str, comments: usize, rewritten: &str) -> (Sandbox, PathBuf) {
    let sandbox = Sandbox::new();
    let source = sandbox.subdirectory("src");
    std::fs::create_dir_all(source.join("tasks")).expect("the folder");
    let file = source.join("tasks/A.md");
    std::fs::write(&file, task_file("as it was", comments)).expect("the task file");
    let notes = sandbox.subdirectory("notes");
    std::fs::create_dir_all(notes.join("tasks")).expect("the folder");
    let relay = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/e2e/rewriting_relay.py");
    sandbox.project_document(&document(&json!({
        "src": {"plugin": "subprocess", "config": {
            "command": interpreter().to_string_lossy(),
            "args": [relay.to_string_lossy()],
            "settings": {
                "host": onetaskgraph_e2e_support::source_binary(), "after": after, "skip": 0,
                "file": file, "text": rewritten,
                "kind": "local-md", "config": {"root": source},
            },
        }},
        "notes": {"plugin": "local-md", "config": {"root": notes}},
    })));
    (sandbox, file)
}

/// A task file with this body and this many comments, each a second after the last.
fn task_file(body: &str, comments: usize) -> String {
    let mut text = format!("---\ntitle: A\nstatus: todo\n---\n{body}\n\n## Comments\n\n");
    for n in 0..comments {
        let at = format!(
            "2026-09-13T{:02}:{:02}:{:02}Z",
            n / 3600,
            n / 60 % 60,
            n % 60
        );
        text.push_str(&format!(
            "<!-- onetaskgraph:comment id=\"c{n}\" created_at=\"{at}\" updated_at=\"{at}\" -->\n\
             ### comment — {at}\n\ncomment {n}\n\n<!-- /onetaskgraph:comment -->\n"
        ));
        if n + 1 < comments {
            text.push('\n');
        }
    }
    text
}

#[test]
fn a_markdown_task_rewritten_at_each_point_a_copy_reads_it_lands_as_the_revision_read() {
    for (point, comments) in [
        ("query_tasks", 3),
        ("get_task_detail", BEYOND_A_FOLDER_PAGE),
    ] {
        let rewritten = task_file("rewritten part way", comments + 1);
        let (sandbox, file) = relayed(point, comments, &rewritten);
        let report = snapshot(&sandbox, "src", 0);
        assert_eq!(
            std::fs::read_to_string(&file).expect("the file"),
            rewritten,
            "{point}: the relay rewrote the file during the copy"
        );
        let copy = landed(&report, "src:A");
        assert_eq!(
            revision(&sandbox, &copy),
            revision(&sandbox, "src:A"),
            "{point}: the copy holds the file as it was rewritten, whole"
        );
        assert_eq!(
            recorded(&sandbox, &copy),
            token(&sandbox, "src", "src:A"),
            "{point}"
        );
    }
}
