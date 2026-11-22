//! Closing a task as a duplicate of another, through the binary, against the loopback board.
//!
//! A caller who has decided two tickets are the same work says so, and the board then records
//! it as GitHub records a duplicate: one `closeIssue` with `stateReason: DUPLICATE` naming the
//! survivor, the card at the `cancelled` option, and the typed `duplicate` link in both issues'
//! metadata, so a later read reports the task `cancelled` and `task relations` names its
//! survivor. Every refusal is made before anything is written.

use std::process::Output;

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{GitHubBoardFields, document, github_projects_with_items};

fn issue(id: &str) -> Value {
    json!({"item":format!("ITEM-{id}"),"id":id,"type":"Issue","title":format!("Ticket {id}"),
           "body":format!("{id}, as filed"),"state":"OPEN","reason":null,"parent":null,
           "repo":"nickderobertis/onetaskgraph","status":"Todo","origin":"","labels":[]})
}

struct Setup {
    sandbox: Sandbox,
    board: GitHubBoardFields,
}

impl Setup {
    fn new() -> Self {
        let sandbox = Sandbox::new();
        let notes = sandbox.subdirectory("notes");
        std::fs::create_dir_all(notes.join("tasks")).unwrap();
        std::fs::write(
            notes.join("tasks/N-1.md"),
            "---\ntitle: Elsewhere\nstatus: todo\n---\nIn another backend.\n",
        )
        .unwrap();
        let mut items = ["U-1", "U-2", "U-3", "K-9"].map(issue).to_vec();
        let mut draft = issue("SKETCH-1");
        draft["type"] = json!("DraftIssue");
        draft["repo"] = Value::Null;
        items.push(draft);
        let (mut config, board) = github_projects_with_items(&sandbox, items);
        config["status_mapping"] = json!({"cancelled": "Cancelled", "done": "Done"});
        sandbox.project_document(&document(&json!({
            "board": {"plugin":"github-projects","config":config},
            "notes": {"plugin":"local-md","config":{"root":notes}},
        })));
        Self { sandbox, board }
    }

    fn run(&self, arguments: &[&str], stdin: Option<&str>) -> Output {
        let mut command = self.sandbox.command();
        command.args(arguments);
        if let Some(stdin) = stdin {
            command.write_stdin(stdin.to_owned());
        }
        command.assert().get_output().clone()
    }

    fn ok(&self, arguments: &[&str], stdin: Option<&str>) -> Value {
        let output = self.run(arguments, stdin);
        assert_eq!(
            output.status.code(),
            Some(0),
            "`onetaskgraph {}` failed\n{}",
            arguments.join(" "),
            stderr(&output)
        );
        serde_json::from_str(&stdout(&output)).expect("JSON")
    }

    /// Every `closeIssue` the board received, as its input.
    fn closes(&self) -> Vec<Value> {
        self.board
            .served()
            .into_iter()
            .filter(|(query, _)| query.contains("closeIssue(input:"))
            .map(|(_, variables)| variables["input"].clone())
            .collect()
    }

    fn mutations(&self) -> usize {
        self.board
            .served()
            .iter()
            .filter(|(query, _)| query.trim_start().starts_with("mutation"))
            .count()
    }

    /// The duplicate reads back closed as a duplicate of `K-9`, `cancelled`, at the card, and
    /// with its `duplicate` link read at both ends.
    fn assert_closed_as_duplicate(&self, id: &str) {
        assert_eq!(
            self.board.state(id),
            ("CLOSED".to_owned(), Some("DUPLICATE".to_owned()))
        );
        assert_eq!(self.board.duplicate_of(id).as_deref(), Some("K-9"));
        assert_eq!(self.board.status(id).as_deref(), Some("Cancelled"));
        let shown = self.ok(&["--json", "task", "show", &format!("board:{id}")], None);
        assert_eq!(
            shown["items"][0]["item"]["status"]["category"], "cancelled",
            "{shown:#}"
        );
        let relations = self.ok(
            &[
                "task",
                "relations",
                &format!("board:{id}"),
                "board:K-9",
                "--json",
            ],
            None,
        );
        let links = &relations["items"][0]["links"];
        assert_eq!(
            links,
            &json!([{"type": "duplicate", "direction": "out", "other": "board:K-9",
                      "other_end": "present"}]),
            "{relations:#}"
        );
    }
}

#[test]
fn a_status_set_closes_a_task_as_a_duplicate_natively_with_its_link() {
    let setup = Setup::new();
    let answer = setup.ok(
        &[
            "--json",
            "task",
            "status",
            "set",
            "board:U-1",
            "cancelled",
            "--duplicate-of",
            "board:K-9",
        ],
        None,
    );
    assert_eq!(answer["status"]["category"], "cancelled", "{answer:#}");
    assert_eq!(
        setup.closes(),
        [json!({"issueId": "U-1", "stateReason": "DUPLICATE", "duplicateIssueId": "K-9"})]
    );
    setup.assert_closed_as_duplicate("U-1");
    // A repeat closes nothing again and records the link once.
    let before = setup.closes().len();
    setup.ok(
        &[
            "--json",
            "task",
            "status",
            "set",
            "board:U-1",
            "cancelled",
            "--duplicate-of",
            "board:K-9",
        ],
        None,
    );
    assert_eq!(setup.closes().len(), before);
    setup.assert_closed_as_duplicate("U-1");
}

#[test]
fn a_task_update_and_an_update_many_close_a_task_as_a_duplicate() {
    let setup = Setup::new();
    let updated = setup.ok(
        &[
            "--json",
            "task",
            "update",
            "board:U-2",
            "--duplicate-of",
            "board:K-9",
        ],
        None,
    );
    assert!(
        updated["written"]
            .as_array()
            .unwrap()
            .contains(&json!("duplicate-of")),
        "{updated:#}"
    );
    setup.assert_closed_as_duplicate("U-2");
    let batch = json!({"schema_version": 1, "updates": [
        {"item": "board:U-3", "duplicate_of": "board:K-9"},
    ]});
    let answer = setup.ok(
        &["task", "update-many", "board", "--input", "-", "--json"],
        Some(&batch.to_string()),
    );
    assert_eq!(answer["results"][0]["outcome"], "written", "{answer:#}");
    setup.assert_closed_as_duplicate("U-3");
    assert_eq!(
        setup
            .closes()
            .iter()
            .map(|close| close["issueId"].clone())
            .collect::<Vec<_>>(),
        [json!("U-2"), json!("U-3")]
    );
}

#[test]
fn every_refusal_of_a_duplicate_close_writes_nothing() {
    let setup = Setup::new();
    for (arguments, said) in [
        (
            vec![
                "task",
                "status",
                "set",
                "board:U-1",
                "done",
                "--duplicate-of",
                "board:K-9",
            ],
            "cancelled",
        ),
        (
            vec![
                "task",
                "update",
                "board:U-1",
                "--status",
                "done",
                "--duplicate-of",
                "board:K-9",
            ],
            "cancelled",
        ),
        (
            vec![
                "task",
                "status",
                "set",
                "board:U-1",
                "cancelled",
                "--duplicate-of",
                "notes:N-1",
            ],
            "notes:N-1",
        ),
        (
            vec!["task", "update", "board:U-1", "--duplicate-of", "board:U-1"],
            "itself",
        ),
        (
            vec![
                "task",
                "update",
                "board:U-1",
                "--duplicate-of",
                "board:SKETCH-1",
            ],
            "board draft",
        ),
        (
            vec![
                "task",
                "update",
                "board:U-1",
                "--duplicate-of",
                "board:U-404",
            ],
            "holds no such issue",
        ),
    ] {
        let output = setup.run(&arguments, None);
        assert_ne!(output.status.code(), Some(0), "{arguments:?}");
        assert!(
            stderr(&output).contains(said),
            "{arguments:?}: {}",
            stderr(&output)
        );
        assert_eq!(setup.mutations(), 0, "{arguments:?} wrote something");
    }
    assert_eq!(setup.board.state("U-1").0, "OPEN");
}

#[test]
fn a_duplicate_closure_github_refuses_fails_and_a_repeat_closes_it() {
    let setup = Setup::new();
    setup.board.refuse_once("closeIssue(");
    let arguments = [
        "--json",
        "task",
        "status",
        "set",
        "board:U-1",
        "cancelled",
        "--duplicate-of",
        "board:K-9",
    ];
    let output = setup.run(&arguments, None);
    assert_ne!(output.status.code(), Some(0));
    assert_eq!(setup.board.state("U-1").0, "OPEN", "{}", stderr(&output));
    setup.ok(&arguments, None);
    setup.assert_closed_as_duplicate("U-1");
}
