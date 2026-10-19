//! A closed issue reads as its closure says, through the binary, against the loopback board.
//!
//! Whichever way an issue closes on GitHub — a person closing it, a change request merged with
//! a closing line, a landing's issue-closing line — every reader of it sees only its state and
//! the reason it was closed with, and its card stays where it was. So each journey here builds
//! a board of open issues, closes each one on the fixture the way GitHub records a closure, and
//! then reads it with nothing written in between: `task show`, `task list --status` and a
//! `task copy` into a folder of Markdown configured with the board's own status names all have
//! to agree that an issue closed as completed at `Todo` is done.

use std::path::PathBuf;
use std::process::Output;

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{GitHubBoardFields, document, github_projects_with_items};

/// A board of open issues, each at the `Status` option named, and a folder of Markdown called
/// `snapshot` whose mapping reads the board's own option names.
struct Setup {
    sandbox: Sandbox,
    board: GitHubBoardFields,
    snapshot: PathBuf,
}

/// One open task issue on the board, filed under no project, at `status`.
fn open_issue(id: &str, title: &str, status: &str) -> Value {
    json!({"item":format!("ITEM-{id}"),"id":id,"type":"Issue","title":title,
           "body":format!("{title}, as filed"),"state":"OPEN","reason":null,"parent":null,
           "repo":"nickderobertis/onetaskgraph","status":status,"origin":"","labels":[]})
}

impl Setup {
    fn new(items: Vec<Value>) -> Self {
        let sandbox = Sandbox::new();
        let snapshot = sandbox.subdirectory("snapshot");
        std::fs::create_dir_all(snapshot.join("tasks")).expect("the snapshot's task folder");
        let (mut config, board) = github_projects_with_items(&sandbox, items);
        board.with_status_option("OPT-in-progress", "In Progress");
        config["status_mapping"] = json!({"todo":"Todo","in-progress":"In Progress"});
        sandbox.project_document(&document(&json!({
            "board": {"plugin":"github-projects","config":config},
            "snapshot": {"plugin":"local-md","config":{"root":snapshot,"status_mapping":{
                "Todo":"todo","In Progress":"in-progress","Done":"done",
                "Cancelled":"cancelled","Shipped":"done"}}}
        })));
        Self {
            sandbox,
            board,
            snapshot,
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

    fn json(&self, arguments: &[&str]) -> Value {
        let output = self.run(arguments);
        assert_eq!(
            output.status.code(),
            Some(0),
            "`onetaskgraph {}` failed\n{}",
            arguments.join(" "),
            stderr(&output)
        );
        serde_json::from_str(&stdout(&output)).expect("the command emits JSON")
    }

    /// The status `task show` reads for one board task.
    fn shown(&self, id: &str) -> Value {
        self.json(&["--json", "task", "show", &format!("board:{id}")])["items"][0]["item"]["status"]
            .clone()
    }

    /// The board tasks `task list --status <category>` keeps, by native id.
    fn listed(&self, category: &str) -> Vec<String> {
        self.json(&[
            "--json", "task", "list", "--source", "board", "--status", category,
        ])["items"]
            .as_array()
            .expect("a page")
            .iter()
            .map(|item| {
                item["id"]
                    .as_str()
                    .and_then(|id| id.strip_prefix("board:"))
                    .expect("a board id")
                    .to_owned()
            })
            .collect()
    }

    /// Copy one board task into the snapshot, answering the `status:` line of the file it
    /// landed in.
    fn copied_status(&self, id: &str) -> String {
        let copied = self.json(&[
            "--json",
            "task",
            "copy",
            &format!("board:{id}"),
            "--to",
            "snapshot",
        ]);
        let landed = copied["items"][0]["destination"]
            .as_str()
            .and_then(|id| id.strip_prefix("snapshot:"))
            .unwrap_or_else(|| panic!("{id} landed in the snapshot: {copied:#}"))
            .to_owned();
        let text = std::fs::read_to_string(self.snapshot.join(format!("tasks/{landed}.md")))
            .expect("the copied task's file");
        text.lines()
            .find_map(|line| line.strip_prefix("status: "))
            .unwrap_or_else(|| panic!("{id}'s copy has a status:\n{text}"))
            .trim_matches('"')
            .to_owned()
    }
}

#[test]
fn an_issue_closed_as_completed_at_todo_reads_and_copies_as_done_named_by_the_done_option() {
    let setup = Setup::new(vec![
        open_issue("C-1", "Landed with a reason", "Todo"),
        open_issue("C-2", "Landed before reasons", "Todo"),
        open_issue("C-6", "Still open", "Todo"),
    ]);
    setup.board.close("C-1", Some("COMPLETED"));
    setup.board.close("C-2", None);

    for id in ["C-1", "C-2"] {
        assert_eq!(
            setup.shown(id),
            json!({"category":"done","name":"Done"}),
            "{id}: closed as completed, so done, named by the board's `done` option"
        );
        assert_eq!(
            setup.board.status(id).as_deref(),
            Some("Todo"),
            "{id}: unwritten"
        );
    }
    assert_eq!(
        setup.shown("C-6"),
        json!({"category":"todo","name":"Todo"}),
        "an open issue reads exactly as before"
    );
    let done = setup.listed("done");
    let todo = setup.listed("todo");
    for id in ["C-1", "C-2"] {
        assert!(done.contains(&id.to_owned()), "{id} is done: {done:?}");
        assert!(!todo.contains(&id.to_owned()), "{id} is not todo: {todo:?}");
    }
    assert!(todo.contains(&"C-6".to_owned()), "{todo:?}");
    assert!(!done.contains(&"C-6".to_owned()), "{done:?}");

    assert_eq!(setup.copied_status("C-1"), "Done");
    assert_eq!(setup.copied_status("C-2"), "Done");
    assert_eq!(setup.copied_status("C-6"), "Todo");
    assert_eq!(
        setup.board.state("C-1"),
        ("CLOSED".to_owned(), Some("COMPLETED".to_owned())),
        "nothing wrote the issue"
    );
}

#[test]
fn an_issue_closed_as_not_planned_or_duplicate_in_progress_reads_and_copies_as_cancelled() {
    let setup = Setup::new(vec![
        open_issue("C-3", "Dropped", "In Progress"),
        open_issue("C-4", "Duplicated", "In Progress"),
    ]);
    setup.board.close("C-3", Some("NOT_PLANNED"));
    setup.board.close("C-4", Some("DUPLICATE"));

    let cancelled = setup.listed("cancelled");
    let in_progress = setup.listed("in-progress");
    for id in ["C-3", "C-4"] {
        assert_eq!(
            setup.shown(id),
            json!({"category":"cancelled","name":"Cancelled"}),
            "{id}: named by the board's `cancelled` option"
        );
        assert!(cancelled.contains(&id.to_owned()), "{cancelled:?}");
        assert!(!in_progress.contains(&id.to_owned()), "{in_progress:?}");
        assert_eq!(setup.copied_status(id), "Cancelled");
        assert_eq!(
            setup.board.status(id).as_deref(),
            Some("In Progress"),
            "{id}: its card is not written"
        );
    }
}

#[test]
fn an_issue_closed_as_completed_in_an_option_the_mapping_does_not_name_keeps_that_name() {
    let setup = Setup::new(vec![open_issue("C-5", "Shipped already", "Shipped")]);
    setup.board.close("C-5", Some("COMPLETED"));
    assert_eq!(
        setup.shown("C-5"),
        json!({"category":"done","name":"Shipped"})
    );
    assert!(setup.listed("done").contains(&"C-5".to_owned()));
    assert_eq!(setup.copied_status("C-5"), "Shipped");
}
