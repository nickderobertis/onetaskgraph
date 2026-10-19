//! A ticket write onto a GitHub board never lands half-done, through the binary.
//!
//! A follow-up ticket once reached a board with no `Status` and no `Host` field, and its
//! withdrawal closed the issue as not planned with no `Cancelled` card. Each journey here makes
//! the loopback board refuse one write of a create or a copy after the item is placed — the
//! batched field write whole, its `Status` alias, or the projected `Host` alias after `Status`
//! landed — and holds the board to holding no trace of the attempt: no item for it, no issue it
//! opened left open, and the failure reported. And a `done` or `cancelled` write whose option
//! selection is refused leaves the issue open, because the option is written before the close.

use std::path::PathBuf;
use std::process::Output;

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{GitHubBoardFields, document, github_projects_with_board};

/// How one write after the item is placed is refused.
#[derive(Debug, Clone, Copy)]
enum Fault {
    /// The batched field write, refused whole: nothing of it lands.
    FieldsWhole,
    /// Its `Status` alias, after every alias before it landed.
    Status,
    /// Its projected `Host` alias, after the `Status` alias before it landed.
    Host,
}

impl Fault {
    fn arm(self, board: &GitHubBoardFields) {
        match self {
            Self::FieldsWhole => board.refuse_once("updateProjectV2ItemFieldValue(input:$input)"),
            Self::Status => board.fail_field_write("FIELD-status"),
            Self::Host => board.fail_field_write("FIELD-text-Host"),
        }
    }
}

/// The loopback board projecting `orchestrator.follow-up` at `host` onto its `Host` field, and
/// a folder of Markdown holding the follow-up `T-1`.
struct Setup {
    sandbox: Sandbox,
    board: GitHubBoardFields,
    root: PathBuf,
}

impl Setup {
    fn new() -> Self {
        let sandbox = Sandbox::new();
        let root = sandbox.subdirectory("notes");
        std::fs::create_dir_all(root.join("tasks")).expect("the task folder");
        std::fs::write(
            root.join("tasks/T-1.md"),
            "---\ntitle: Disk filling on the build host\nstatus: todo\nmetadata: \
             {orchestrator.follow-up: {run: r-1, host: build-7}}\n---\nSeen twice tonight.\n",
        )
        .expect("the follow-up");
        std::fs::write(root.join("body.md"), "A new ticket.\n").expect("a body file");
        let (mut config, board) = github_projects_with_board(&sandbox);
        config["metadata_fields"] =
            json!([{"field": "Host", "key": "orchestrator.follow-up", "path": ["host"]}]);
        board.with_text_field("Host");
        sandbox.project_document(&document(&json!({
            "notes": {"plugin": "local-md", "config": {"root": root}},
            "board": {"plugin": "github-projects", "config": config},
        })));
        Self {
            sandbox,
            board,
            root,
        }
    }

    fn run(&self, arguments: &[&str]) -> Output {
        self.sandbox
            .command()
            .args(arguments)
            .assert()
            .get_output()
            .clone()
    }

    /// The ids of every issue a `createIssue` this board answered opened.
    fn opened(&self) -> Vec<String> {
        // A created issue is named `ISSUE-<n>` in the order the board created them.
        (1..=self
            .board
            .served()
            .into_iter()
            .filter(|(document, _)| document.contains("createIssue(input:"))
            .count())
            .map(|n| format!("ISSUE-{n}"))
            .collect()
    }

    /// That nothing of an attempt is left: every issue it opened is gone from the board and
    /// from the repository, and the board's tasks are the ones it started with.
    fn left_no_trace(&self, what: &str, tasks_before: &[String]) {
        let opened = self.opened();
        assert!(!opened.is_empty(), "{what}: the attempt opened an issue");
        for issue in &opened {
            assert_eq!(
                self.board.status(issue),
                None,
                "{what}: the board holds no item for {issue}"
            );
            assert!(
                !self.board.holds_issue(issue),
                "{what}: the repository holds no issue {issue}"
            );
        }
        assert_eq!(self.tasks(), tasks_before, "{what}: the board's tasks");
    }

    /// Every task the board holds, by its qualified id.
    fn tasks(&self) -> Vec<String> {
        let output = self.run(&["--json", "task", "list", "--source", "board"]);
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        let listed: Value = serde_json::from_str(&stdout(&output)).expect("JSON");
        listed["items"]
            .as_array()
            .expect("items")
            .iter()
            .map(|item| item["id"].as_str().expect("an id").to_owned())
            .collect()
    }
}

/// A `task create` onto the board with `fault` armed: refused, reported, and leaving no item.
fn create_leaves_no_trace(fault: Fault) {
    let setup = Setup::new();
    let before = setup.tasks();
    fault.arm(&setup.board);
    let body = setup.root.join("body.md");
    let output = setup.run(&[
        "task",
        "create",
        "board",
        "--project",
        "P-1",
        "--title",
        "A new ticket",
        "--status",
        "todo",
        "--body-file",
        body.to_str().expect("a UTF-8 path"),
        "--metadata",
        "orchestrator.follow-up={\"host\":\"build-7\"}",
    ]);
    assert_ne!(
        output.status.code(),
        Some(0),
        "{fault:?}: the create was refused\n{}",
        stdout(&output)
    );
    assert!(
        stderr(&output).contains("Something went wrong while executing your query"),
        "{fault:?}: the failure is reported: {}",
        stderr(&output)
    );
    setup.left_no_trace(&format!("{fault:?} create"), &before);
}

/// A `task copy` onto the board with `fault` armed: rolled back whole, reported, and leaving
/// no item and no link to one.
fn copy_leaves_no_trace(fault: Fault) {
    let setup = Setup::new();
    let before = setup.tasks();
    fault.arm(&setup.board);
    let output = setup.run(&["task", "copy", "notes:T-1", "--to", "board"]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "{fault:?}: the copy was refused\n{}",
        stdout(&output)
    );
    assert!(
        stderr(&output).contains("Something went wrong while executing your query"),
        "{fault:?}: the failure is reported: {}",
        stderr(&output)
    );
    setup.left_no_trace(&format!("{fault:?} copy"), &before);
    assert!(
        !std::fs::read_to_string(setup.root.join("tasks/T-1.md"))
            .expect("the follow-up")
            .contains("onetaskgraph.copies"),
        "{fault:?}: no link to an item that is gone"
    );
}

/// `task status set board:T-1 <category>` with its option selection refused.
fn terminal_status_leaves_the_issue_open(category: &str) {
    let setup = Setup::new();
    setup
        .board
        .refuse_once("updateProjectV2ItemFieldValue(input:$input)");
    let output = setup.run(&["task", "status", "set", "board:T-1", category]);
    assert_ne!(output.status.code(), Some(0), "{category}: refused");
    assert_eq!(
        setup.board.state("T-1"),
        ("OPEN".to_owned(), None),
        "{category}: an issue whose option was refused is not closed"
    );
    assert_eq!(
        setup.board.status("T-1").as_deref(),
        Some("Todo"),
        "{category}: its card is where it was"
    );
    assert!(
        !setup
            .board
            .served()
            .iter()
            .any(|(document, _)| document.contains("updateIssue(input:")),
        "{category}: no close was sent"
    );
}

#[test]
fn a_task_create_whose_field_write_is_refused_whole_leaves_no_item_behind() {
    create_leaves_no_trace(Fault::FieldsWhole);
}

#[test]
fn a_task_create_whose_status_write_is_refused_leaves_no_item_behind() {
    create_leaves_no_trace(Fault::Status);
}

#[test]
fn a_task_create_whose_projected_field_is_refused_after_its_status_leaves_no_item_behind() {
    create_leaves_no_trace(Fault::Host);
}

#[test]
fn a_task_copy_whose_field_write_is_refused_whole_is_rolled_back() {
    copy_leaves_no_trace(Fault::FieldsWhole);
}

#[test]
fn a_task_copy_whose_status_write_is_refused_is_rolled_back() {
    copy_leaves_no_trace(Fault::Status);
}

#[test]
fn a_task_copy_whose_projected_field_is_refused_after_its_status_is_rolled_back() {
    copy_leaves_no_trace(Fault::Host);
}

#[test]
fn a_done_write_whose_option_selection_is_refused_leaves_the_issue_open() {
    terminal_status_leaves_the_issue_open("done");
}

#[test]
fn a_cancelled_write_whose_option_selection_is_refused_leaves_the_issue_open() {
    terminal_status_leaves_the_issue_open("cancelled");
}
