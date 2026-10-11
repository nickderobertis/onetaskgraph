//! A board task with no change token whose comments run past one page is copied only once two
//! full passes over its comments agree.
//!
//! The board is read in the very second its task last changed, so the plugin gives that task no
//! token (`docs/plugin-protocol.md` §4.13d), and its clock is held to that second, so a comment
//! edited between two page reads moves nothing a page read carries. What stands between that
//! edit and a copy holding the comment as it was is the full re-pass, and these journeys hold
//! the copy to it: to landing the comments as the board now holds them, or — when the task never
//! stops changing — to writing nothing for it. The requests a settling copy spent are recorded as
//! telemetry beside the recopy figures, for the source cache's budget to report.

use onetaskgraph_github_projects::graphql;
use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::copy_recopy::{part, record, spent};
use crate::fixtures::{GitHubBoardFields, document, github_projects_with_board};

/// More comments than one page of the board's, so `T-3`'s comments take two reads a pass.
const BEYOND_A_PAGE: usize = 120;

/// A board whose `T-3` holds more than a page of comments and was last changed in the second
/// the board is read in, beside an empty folder `notes`; answers with the board's handle and
/// the id of `T-3`'s first comment.
fn board() -> (Sandbox, GitHubBoardFields, String, std::path::PathBuf) {
    let sandbox = Sandbox::new();
    let (config, board) = github_projects_with_board(&sandbox);
    let notes = sandbox.subdirectory("notes");
    std::fs::create_dir_all(notes.join("tasks")).expect("the folder");
    sandbox.project_document(&document(&json!({
        "board": {"plugin": "github-projects", "config": config},
        "notes": {"plugin": "local-md", "config": {"root": notes, "status_mapping": {
            "Todo": "todo", "Doing": "in-progress", "Shipped": "done", "Done": "done"}}},
    })));
    let first = board.add_comment("T-3", "comment 0");
    for n in 1..BEYOND_A_PAGE {
        board.add_comment("T-3", &format!("comment {n}"));
    }
    board.hold_clock();
    board.read_as_it_changes(true);
    board.edit_body("T-3", "changed just now");
    (sandbox, board, first, notes)
}

/// Whether one board request is a read of a page of `T-3`'s comments past its first.
fn later_page(document: &str, variables: &Value) -> bool {
    document == graphql::ISSUE_DETAIL && variables["id"] == "T-3" && !variables["after"].is_null()
}

/// One `--comments` copy by query of the board into the folder, exiting `code`.
fn copy(sandbox: &Sandbox, code: i32) -> Value {
    let output = sandbox
        .command()
        .args([
            "task",
            "copy",
            "--to",
            "notes",
            "--source",
            "board",
            "--comments",
            "--no-link",
            "--json",
        ])
        .assert()
        .get_output()
        .clone();
    assert_eq!(output.status.code(), Some(code), "{}", stderr(&output));
    serde_json::from_str(&stdout(&output)).expect("JSON")
}

fn outcome<'a>(report: &'a Value, source: &str) -> &'a Value {
    report["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|item| item["source"] == source)
        .unwrap_or_else(|| panic!("{source} is reported: {report:#}"))
}

/// What the binary lists of one task's comments, as their bodies in order.
fn bodies(sandbox: &Sandbox, id: &str) -> Vec<Value> {
    let output = sandbox
        .command()
        .args(["task", "comment", "list", id, "--json"])
        .assert()
        .get_output()
        .clone();
    let listed: Value = serde_json::from_str(&stdout(&output)).expect("JSON");
    listed["comments"]
        .as_array()
        .expect("comments")
        .iter()
        .map(|comment| comment["body"].clone())
        .collect()
}

#[test]
fn a_comment_edited_on_an_earlier_page_between_page_reads_lands_as_it_now_reads() {
    let (sandbox, board, first, _) = board();
    // Once the copy has read T-3's second page, a person edits the comment on its first.
    let edited = first.clone();
    board.after_request(later_page, 0, move |board| {
        board.edit_comment(&edited, "comment 0, edited between pages");
    });
    let mark = board.served().len();
    let report = copy(&sandbox, 0);
    let landed = outcome(&report, "board:T-3");
    assert_eq!(landed["action"], "created", "{landed:#}");
    let copy = landed["destination"].as_str().expect("T-3 landed");
    assert_eq!(
        bodies(&sandbox, copy),
        bodies(&sandbox, "board:T-3"),
        "the folder holds T-3's comments as the board holds them now"
    );
    assert_eq!(bodies(&sandbox, copy)[0], "comment 0, edited between pages");

    // Three passes over its two pages: the one the edit fell in, one that disagreed with it,
    // and one that agreed with that.
    let served = board.served()[mark..].to_vec();
    let of_t3: Vec<(String, Value)> = served
        .iter()
        .filter(|(document, variables)| {
            variables
                .as_object()
                .is_some_and(|members| members.values().any(|value| value == "T-3"))
                && part(document, variables) != "listing"
        })
        .cloned()
        .collect();
    // Three passes over its two pages, in order: the one the edit fell in — its first page in
    // the batched read of every task — then one that disagreed with it, then one that agreed.
    // What follows them is the plugin reading T-3's edges, which the batched read carried and
    // the single-issue reads of the later passes did not; it is in the telemetry below.
    let passes: Vec<&str> = of_t3
        .iter()
        .take(6)
        .map(|(document, variables)| part(document, variables))
        .collect();
    assert_eq!(
        passes,
        [
            "detail",
            "comment_pages",
            "detail",
            "comment_pages",
            "detail",
            "comment_pages"
        ],
        "{of_t3:#?}"
    );
    assert!(
        of_t3
            .iter()
            .take(6)
            .skip(2)
            .step_by(2)
            .all(|(document, variables)| document == graphql::ISSUE_DETAIL
                && variables["after"].is_null()),
        "each later pass starts again from the first page: {of_t3:#?}"
    );
    record(
        "github-copy-unsettled-comment-passes",
        "copy-of-a-tokenless-task-with-two-comment-pages-edited-between-them",
        &spent(&of_t3),
    );
}

#[test]
fn a_task_whose_comments_never_settle_is_written_nowhere_and_reported_failed() {
    let (sandbox, board, first, notes) = board();
    for pass in 0..3 {
        let edited = first.clone();
        board.after_request(later_page, pass, move |board| {
            board.edit_comment(&edited, &format!("comment 0, edited during pass {pass}"));
        });
    }
    let report = copy(&sandbox, 4);
    let failed = outcome(&report, "board:T-3");
    assert_eq!(failed["action"], "failed", "{failed:#}");
    assert_eq!(
        failed["error"]["kind"], "changed-while-copied",
        "{failed:#}"
    );
    let written: Vec<String> = std::fs::read_dir(notes.join("tasks"))
        .expect("the folder")
        .map(|entry| std::fs::read_to_string(entry.expect("an entry").path()).expect("a file"))
        .collect();
    assert!(
        !written.iter().any(|file| file.contains("changed just now")),
        "nothing was written for T-3"
    );
    assert!(
        outcome(&report, "board:T-1")["destination"].is_string(),
        "the other tasks land: {report:#}"
    );
}
