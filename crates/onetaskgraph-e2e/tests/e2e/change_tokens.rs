//! A task's change token, as every read of the binary reports it.
//!
//! The token is the source's promise that nothing in a task changed: its title, body, status,
//! priority, labels, board fields, metadata, edges and comments. So each journey here changes
//! one task of a loopback board by exactly one of those kinds — the way a person changes it on
//! GitHub, through the board's own state — and holds the token to moving to a later one, by
//! the store's own ordering, while every task the change did not touch keeps the token it had.
//! `docs/plugin-protocol.md` §4.13d states what each plugin's token keys on; the loopback
//! board models exactly those three parts.

use std::collections::BTreeMap;

use onetaskgraph_core::GlobalId;
use onetaskgraph_core::version::{VersionOrder, Versioned, compare};
use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{GitHubBoardFields, ROWS, SOURCE, document, github_projects_with_board};

/// The JSON a run printed, having exited zero.
fn answered(sandbox: &Sandbox, arguments: &[&str]) -> Value {
    let output = sandbox
        .command()
        .args(arguments)
        .assert()
        .get_output()
        .clone();
    assert_eq!(
        output.status.code(),
        Some(0),
        "`onetaskgraph {}` failed:\n{}",
        arguments.join(" "),
        stderr(&output)
    );
    serde_json::from_str(&stdout(&output)).expect("machine output is JSON")
}

/// Every task `task list` reports, by qualified id, with the token it carries.
fn listed(sandbox: &Sandbox, extra: &[&str]) -> BTreeMap<String, Option<String>> {
    let mut arguments = vec!["task", "list", "--json", "--limit", "100"];
    arguments.extend_from_slice(extra);
    answered(sandbox, &arguments)["items"]
        .as_array()
        .expect("a list answers with items")
        .iter()
        .map(|task| {
            (
                task["id"].as_str().expect("a qualified id").to_owned(),
                task["item"]["version"].as_str().map(str::to_owned),
            )
        })
        .collect()
}

/// A board configured as the source `board`, with its handle.
fn board() -> (Sandbox, GitHubBoardFields) {
    let sandbox = Sandbox::new();
    let (config, board) = github_projects_with_board(&sandbox);
    sandbox.project_document(&document(&json!({
        "board": {"plugin": "github-projects", "config": config}
    })));
    (sandbox, board)
}

/// One change a person makes to one task on the board.
type Change = Box<dyn Fn(&GitHubBoardFields)>;

#[test]
fn every_change_kind_moves_the_changed_tasks_token_to_a_later_one_and_no_other() {
    let (sandbox, board) = board();
    // Two comments to edit and to delete, made before anything is read.
    let first = board.add_comment("T-3", "the first word");
    let second = board.add_comment("T-3", "the second word");
    let changes: Vec<(&str, Change)> = vec![
        (
            "a new comment",
            Box::new(|board| {
                board.add_comment("T-3", "a third word");
            }),
        ),
        (
            "an edited comment",
            Box::new(move |board| board.edit_comment(&first, "the first word, edited")),
        ),
        (
            "a deleted comment",
            Box::new(move |board| board.delete_comment(&second)),
        ),
        (
            "a body edit",
            Box::new(|board| board.edit_body("T-3", "a body a person rewrote")),
        ),
        (
            "a title edit",
            Box::new(|board| board.edit_title("T-3", "Gamma, renamed")),
        ),
        (
            "a status change",
            Box::new(|board| board.move_card("T-3", "Doing")),
        ),
        (
            "a priority change",
            Box::new(|board| board.assign_priority("T-3", Some("Low"))),
        ),
        (
            "a label change",
            Box::new(|board| board.set_labels("T-3", &["chore"])),
        ),
        (
            "a project field change",
            Box::new(|board| board.set_card_field("T-3", "Estimate", "3 days")),
        ),
        (
            "a metadata change",
            Box::new(|board| board.set_metadata("T-3", "caller.note", json!("noted"))),
        ),
        (
            "a dependency added",
            Box::new(|board| board.block("T-3", &["T-4".to_owned()])),
        ),
        (
            "a dependency removed",
            Box::new(|board| board.unblock("T-3", "T-2")),
        ),
    ];
    let changed = GlobalId::from_str_checked("board:T-3");
    for (kind, change) in changes {
        let before = listed(&sandbox, &["--source", "board"]);
        change(&board);
        let after = listed(&sandbox, &["--source", "board"]);
        let old = before["board:T-3"]
            .clone()
            .unwrap_or_else(|| panic!("{kind}: a board task carries a token before: {before:?}"));
        let new = after["board:T-3"]
            .clone()
            .unwrap_or_else(|| panic!("{kind}: a board task carries a token after: {after:?}"));
        assert_ne!(old, new, "{kind}: two equal tokens across a change");
        assert_eq!(
            compare(
                Versioned {
                    item: &changed,
                    version: &new
                },
                Versioned {
                    item: &changed,
                    version: &old
                }
            ),
            VersionOrder::Later,
            "{kind}: the store orders the token after the change as the later"
        );
        for (id, token) in &before {
            if id != "board:T-3" {
                assert_eq!(
                    &after[id], token,
                    "{kind}: {id} was not touched, so its token is the one it had"
                );
            }
        }
    }
}

/// `GlobalId::from_str`, for an id this module spells itself.
trait Checked {
    fn from_str_checked(text: &str) -> Self;
}

impl Checked for GlobalId {
    fn from_str_checked(text: &str) -> Self {
        text.parse().expect("a qualified id")
    }
}

#[test]
fn every_read_carries_the_token_its_source_gives_and_linear_gives_none() {
    for row in ROWS.iter().filter(|row| row.fixture.complete_dataset) {
        let who = row.name;
        let sandbox = Sandbox::new();
        sandbox.project_document(&row.document(&sandbox));
        let tokened = matches!(row.plugin, "github-projects" | "local-md");
        let listed = listed(&sandbox, &[]);
        assert!(!listed.is_empty(), "{who}: the dataset holds tasks");
        for (id, token) in &listed {
            assert_eq!(
                token.is_some(),
                tokened,
                "{who}: {id} carries a token exactly when its source gives one"
            );
            let shown = answered(&sandbox, &["task", "show", id, "--json"]);
            assert_eq!(
                shown["items"][0]["item"]["version"].as_str(),
                token.as_deref(),
                "{who}: `task show` reports the token `task list` did"
            );
        }
        let ids: Vec<&str> = listed.keys().map(String::as_str).collect();
        let mut arguments = vec!["task", "show-many", "--json"];
        arguments.extend(&ids);
        let many = answered(&sandbox, &arguments);
        for (detail, id) in many["details"]
            .as_array()
            .expect("details")
            .iter()
            .zip(&ids)
        {
            assert_eq!(
                detail["items"][0]["item"]["version"].as_str(),
                listed[*id].as_deref(),
                "{who}: `task show-many` reports the token `task list` did for {id}"
            );
        }
    }
}

#[test]
fn a_task_kept_for_its_comment_activity_carries_its_token() {
    // A board, whose search names the issues updated since the instant, and a folder of
    // Markdown, which reads each task's own comments.
    let (sandbox, board) = board();
    board.add_comment("T-1", "recent activity");
    let since = "2026-09-01T00:00:00Z";
    let kept = listed(&sandbox, &["--source", "board", "--commented-since", since]);
    assert_eq!(
        kept.keys().collect::<Vec<_>>(),
        vec!["board:T-1"],
        "{kept:?}"
    );
    assert!(kept["board:T-1"].is_some(), "{kept:?}");

    let sandbox = Sandbox::new();
    let root = sandbox.subdirectory("notes");
    std::fs::create_dir_all(root.join("tasks")).unwrap();
    std::fs::write(
        root.join("tasks/A.md"),
        "---\ntitle: A\nstatus: todo\n---\nBody.\n\n## Comments\n\n<!-- onetaskgraph:comment id=\"c1\" created_at=\"2026-09-13T15:11:07Z\" updated_at=\"2026-09-13T15:11:07Z\" -->\n### comment — 2026-09-13T15:11:07Z\n\nRecent.\n\n<!-- /onetaskgraph:comment -->\n",
    )
    .unwrap();
    sandbox.project_document(&document(&json!({
        SOURCE: {"plugin": "local-md", "config": {"root": root}}
    })));
    let kept = listed(&sandbox, &["--commented-since", since]);
    assert!(
        kept["work:A"]
            .as_deref()
            .is_some_and(|token| token.starts_with("md1|")),
        "{kept:?}"
    );
}
