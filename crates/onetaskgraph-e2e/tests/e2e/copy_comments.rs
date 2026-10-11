//! `--comments`: a copy keeps the destination's copies of a task's comments in step with the
//! source's, in every direction, by one rule.
//!
//! Every journey drives the binary against a loopback GitHub board — two, where a board is
//! copied into another — a loopback Linear workspace, and a folder of Markdown over both
//! source boundaries, and reads what landed back the way a user does: `task comment list`
//! and `task show`, and the board's own record of an issue's comments where what is asserted
//! is how the board stores one. A comment's origin names the comment where it was written,
//! however many copies it has been through.

use std::path::PathBuf;
use std::process::Output;

use serde_json::{Value, json};

use crate::common::{SOURCE_BOUNDARIES, Sandbox, SourceBoundary, stderr, stdout};
use crate::document_store::interpreter;
use crate::fixtures::{
    GITHUB_COMMENTER, GitHubBoardFields, dataset, document, github_projects_with_board,
    linear_workspace,
};

/// Every store one journey copies between, configured together.
struct Stores {
    sandbox: Sandbox,
    /// The board configured as `board`; a second one is configured as `second`.
    board: GitHubBoardFields,
    /// The folder of Markdown configured as `notes`, behind the boundary asked for.
    notes: PathBuf,
}

impl Stores {
    /// A board, a second board, a Linear workspace over the shared dataset, and an empty
    /// folder of Markdown reached across `boundary`.
    fn new(boundary: SourceBoundary) -> Self {
        let sandbox = Sandbox::new();
        let (board_config, board) = github_projects_with_board(&sandbox);
        let (second_config, _) = github_projects_with_board(&sandbox);
        let (linear_config, _) = linear_workspace(&sandbox, dataset());
        // Both loopbacks name a credential; one secrets file holds both.
        sandbox
            .secrets_file("GITHUB_PROJECTS_FIXTURE_TOKEN=test-token\nLINEAR_API_KEY=fixture-key\n");
        let notes = sandbox.subdirectory("notes");
        std::fs::create_dir_all(notes.join("tasks")).expect("the task folder");
        sandbox.project_document(&document(&json!({
            "board": {"plugin": "github-projects", "config": board_config},
            "second": {"plugin": "github-projects", "config": second_config},
            "linear": {"plugin": "linear", "config": linear_config},
            // Every status name the board and the folder's own files use, the board's own
            // `Shipped` among them.
            "notes": boundary.source(
                "local-md",
                json!({"root": notes, "status_mapping": {
                    "todo": "todo", "Todo": "todo", "Doing": "in-progress",
                    "Shipped": "done", "Done": "done"}})
            ),
        })));
        Self {
            sandbox,
            board,
            notes,
        }
    }

    fn run(&self, arguments: &[&str], input: Option<&str>) -> Output {
        let mut command = self.sandbox.command();
        command.args(arguments);
        command.write_stdin(input.unwrap_or_default());
        command.assert().get_output().clone()
    }

    /// The JSON a run printed, having exited zero.
    fn json(&self, arguments: &[&str]) -> Value {
        let mut arguments = arguments.to_vec();
        arguments.push("--json");
        let output = self.run(&arguments, None);
        assert_eq!(
            output.status.code(),
            Some(0),
            "`onetaskgraph {}` failed:\n{}",
            arguments.join(" "),
            stderr(&output)
        );
        serde_json::from_str(&stdout(&output)).expect("machine output is JSON")
    }

    /// One `--comments` copy of one task, answering with its one outcome.
    fn copy(&self, id: &str, to: &str) -> Value {
        let report = self.json(&["task", "copy", id, "--to", to, "--comments"]);
        let items = report["items"].as_array().expect("a report has items");
        assert_eq!(items.len(), 1, "{report:#}");
        items[0].clone()
    }

    /// The comments `task comment list` reports for one task.
    fn comments(&self, id: &str) -> Vec<Value> {
        self.json(&["task", "comment", "list", id])["comments"]
            .as_array()
            .expect("a comment list")
            .clone()
    }

    /// Add one comment to one task through the binary, answering with its id.
    fn comment(&self, id: &str, body: &str) -> String {
        let output = self.run(&["task", "comment", "add", id, "--json"], Some(body));
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        let added: Value = serde_json::from_str(&stdout(&output)).expect("JSON");
        added["id"].as_str().expect("a comment id").to_owned()
    }

    /// Replace one comment's body through the binary.
    fn edit(&self, id: &str, comment: &str, body: &str) {
        let output = self.run(&["task", "comment", "edit", id, comment], Some(body));
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    }

    /// Delete one comment through the binary.
    fn delete(&self, id: &str, comment: &str) {
        let output = self.run(&["task", "comment", "delete", id, comment], None);
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    }

    /// A task file in the folder of Markdown, written as a person writes one.
    fn note(&self, id: &str, title: &str) {
        std::fs::write(
            self.notes.join(format!("tasks/{id}.md")),
            format!("---\ntitle: {title}\nstatus: todo\n---\nWritten here.\n"),
        )
        .expect("a task file");
    }

    /// The raw file of one task in the folder.
    fn file(&self, native: &str) -> String {
        std::fs::read_to_string(self.notes.join(format!("tasks/{native}.md"))).expect("a file")
    }
}

/// What a comment says and the `(task, comment id)` of its origin, if it is a copy.
type Said = (String, Option<(String, String)>);

/// What one outcome reports about the comments, as `[added, edited, deleted, unchanged]`.
fn counts(outcome: &Value) -> [u64; 4] {
    let counts = &outcome["comments"];
    ["added", "edited", "deleted", "unchanged"].map(|figure| {
        counts[figure]
            .as_u64()
            .unwrap_or_else(|| panic!("a `--comments` outcome counts {figure}: {outcome:#}"))
    })
}

/// The body and the origin `(task, comment id)` of each comment, in no order a test depends
/// on — copies land after the comments already there.
fn said(comments: &[Value]) -> Vec<Said> {
    let mut said: Vec<Said> = comments
        .iter()
        .map(|comment| {
            (
                comment["body"].as_str().expect("a body").to_owned(),
                comment.get("origin").map(|origin| {
                    (
                        origin["task"].as_str().expect("an origin task").to_owned(),
                        origin["id"].as_str().expect("an origin id").to_owned(),
                    )
                }),
            )
        })
        .collect();
    said.sort();
    said
}

fn destination(outcome: &Value) -> String {
    outcome["destination"]
        .as_str()
        .unwrap_or_else(|| panic!("the copy landed: {outcome:#}"))
        .to_owned()
}

fn native(id: &str) -> &str {
    id.split_once(':').expect("a qualified id").1
}

#[test]
fn a_board_tasks_comments_land_in_markdown_with_their_origin_and_follow_every_change() {
    for boundary in SOURCE_BOUNDARIES {
        let who = format!("{boundary:?}");
        let stores = Stores::new(boundary);
        let first = stores.board.add_comment("T-3", "the first word");
        let second = stores.board.add_comment("T-3", "the second word");
        let board_comments = stores.comments("board:T-3");

        let landed = stores.copy("board:T-3", "notes");
        assert_eq!(landed["action"], "created", "{who}: {landed:#}");
        assert_eq!(counts(&landed), [2, 0, 0, 0], "{who}");
        let note = destination(&landed);
        let copied = stores.comments(&note);
        assert_eq!(copied.len(), 2, "{who}: {copied:#?}");
        for (copy, held) in copied.iter().zip(&board_comments) {
            // Stored natively: the origin's own id, author and times, and where it is.
            assert_eq!(
                copy["origin"],
                json!({"task": "board:T-3", "id": held["id"], "author": GITHUB_COMMENTER,
                       "created_at": held["created_at"], "updated_at": held["updated_at"],
                       "url": held["url"]}),
                "{who}"
            );
            assert_eq!(copy["body"], held["body"], "{who}");
        }
        assert!(
            !stores
                .file(native(&note))
                .contains("onetaskgraph.comment_origin"),
            "{who}: a folder keeps the origin natively, never as the marker"
        );

        // Every kind of change at the source, and a comment written in the folder itself.
        stores.board.edit_comment(&first, "the first word, edited");
        stores.board.delete_comment(&second);
        let third = stores.board.add_comment("T-3", "a third word");
        stores.comment(&note, "written in the folder");
        let followed = stores.copy("board:T-3", "notes");
        assert_eq!(counts(&followed), [1, 1, 1, 0], "{who}: {followed:#}");
        assert_eq!(
            said(&stores.comments(&note)),
            said(&[
                json!({"body": "the first word, edited",
                       "origin": {"task": "board:T-3", "id": first}}),
                json!({"body": "a third word", "origin": {"task": "board:T-3", "id": third}}),
                json!({"body": "written in the folder"}),
            ]),
            "{who}: the copies follow the board, and the folder's own comment is untouched"
        );

        // Back to the board it came from: only what was written in the folder goes.
        let back = stores.copy(&note, "board");
        assert_eq!(back["action"], "updated", "{who}: {back:#}");
        assert_eq!(back["via"], "origin", "{who}");
        assert_eq!(
            counts(&back),
            [1, 0, 0, 0],
            "{who}: no board comment is echoed"
        );
        let on_board = stores.comments("board:T-3");
        let local = stores
            .comments(&note)
            .into_iter()
            .find(|comment| comment.get("origin").is_none())
            .expect("the folder's own comment");
        assert_eq!(
            said(&on_board),
            said(&[
                json!({"body": "the first word, edited"}),
                json!({"body": "a third word"}),
                json!({"body": "written in the folder",
                       "origin": {"task": note, "id": local["id"]}}),
            ]),
            "{who}: the board keeps its own comments as they were and gains the folder's, its \
             marker read back out of the body"
        );
        assert!(
            stores
                .board
                .comments_on("T-3")
                .iter()
                .any(|held| held["body"]
                    .as_str()
                    .is_some_and(|body| body.contains("onetaskgraph.comment_origin"))),
            "{who}: a board keeps the origin as the hidden marker"
        );

        // And out again: the folder's comment, now on the board, is never sent back to it.
        let again = stores.copy("board:T-3", "notes");
        assert_eq!(counts(&again), [0, 0, 0, 2], "{who}: {again:#}");
        assert_eq!(stores.comments(&note).len(), 3, "{who}");
    }
}

#[test]
fn a_markdown_tasks_comments_land_on_a_board_with_their_origin_and_follow_every_change() {
    for boundary in SOURCE_BOUNDARIES {
        let who = format!("{boundary:?}");
        let stores = Stores::new(boundary);
        stores.note("A", "Written in the folder");
        let kept = stores.comment("notes:A", "kept as written");
        let edited = stores.comment("notes:A", "about to change");
        let removed = stores.comment("notes:A", "about to go");
        let local = stores.comments("notes:A");

        let landed = stores.copy("notes:A", "board");
        assert_eq!(landed["action"], "created", "{who}: {landed:#}");
        assert_eq!(counts(&landed), [3, 0, 0, 0], "{who}");
        let issue = destination(&landed);
        let on_board = stores.comments(&issue);
        for (copy, held) in on_board.iter().zip(&local) {
            assert_eq!(
                copy["body"], held["body"],
                "{who}: the marker is read back out"
            );
            assert_eq!(
                copy["origin"],
                json!({"task": "notes:A", "id": held["id"], "author": held["author"],
                       "created_at": held["created_at"], "updated_at": held["updated_at"],
                       "url": held["url"]}),
                "{who}"
            );
            assert_eq!(
                copy["author"], GITHUB_COMMENTER,
                "{who}: GitHub signs every comment"
            );
        }

        stores.board.add_comment(native(&issue), "a board comment");
        stores.edit("notes:A", &edited, "changed in the folder");
        stores.delete("notes:A", &removed);
        let followed = stores.copy("notes:A", "board");
        assert_eq!(counts(&followed), [0, 1, 1, 1], "{who}: {followed:#}");
        assert_eq!(
            said(&stores.comments(&issue)),
            said(&[
                json!({"body": "kept as written", "origin": {"task": "notes:A", "id": kept}}),
                json!({"body": "changed in the folder",
                       "origin": {"task": "notes:A", "id": edited}}),
                json!({"body": "a board comment"}),
            ]),
            "{who}: the board's own comment is never edited or deleted"
        );
    }
}

#[test]
fn an_empty_source_comment_is_never_copied_and_one_emptied_takes_its_copy_with_it() {
    let stores = Stores::new(SourceBoundary::Direct);
    let kept = stores.board.add_comment("T-3", "said and kept");
    let cleared = stores.board.add_comment("T-3", "about to be cleared");
    // A comment with nothing in it — an attachment, or a body a person cleared on the
    // service's own page — is one no destination this product writes would take.
    stores.board.add_comment("T-3", "");

    let landed = stores.copy("board:T-3", "notes");
    assert_eq!(counts(&landed), [2, 0, 0, 0], "{landed:#}");
    let note = destination(&landed);

    stores.board.edit_comment(&cleared, "");
    let followed = stores.copy("board:T-3", "notes");
    assert_eq!(counts(&followed), [0, 0, 1, 1], "{followed:#}");
    assert_eq!(
        said(&stores.comments(&note)),
        said(&[json!({"body": "said and kept",
                      "origin": {"task": "board:T-3", "id": kept}})]),
        "the copy of a comment its source emptied is gone, and nothing empty was written"
    );
}

#[test]
fn a_copy_never_touches_a_copy_another_task_carried_onto_its_destination() {
    for boundary in SOURCE_BOUNDARIES {
        let who = format!("{boundary:?}");
        let stores = Stores::new(boundary);
        stores.note("A", "Written in the folder");
        let gone = stores.comment("notes:A", "about to go");
        let issue = destination(&stores.copy("notes:A", "board"));
        // A copy of a comment of a task this copy has nothing to do with, carried onto the same
        // issue by some other copy and marked as every copy onto a board is.
        let elsewhere = json!({"task": "linear:L-9", "id": "C-77", "author": "grace",
                               "created_at": null, "updated_at": null, "url": null});
        stores.board.add_comment(
            native(&issue),
            &format!(
                "carried from elsewhere\n\n<!-- onetaskgraph.comment_origin `{elsewhere}` -->"
            ),
        );
        stores.delete("notes:A", &gone);
        let new = stores.comment("notes:A", "a new word");

        let followed = stores.copy("notes:A", "board");
        assert_eq!(counts(&followed), [1, 0, 1, 0], "{who}: {followed:#}");
        assert_eq!(
            said(&stores.comments(&issue)),
            said(&[
                json!({"body": "carried from elsewhere",
                       "origin": {"task": "linear:L-9", "id": "C-77"}}),
                json!({"body": "a new word", "origin": {"task": "notes:A", "id": new}}),
            ]),
            "{who}: the copy of another task's comment is neither edited nor deleted"
        );
    }
}

#[test]
fn linear_and_markdown_carry_each_others_comments_both_ways() {
    let stores = Stores::new(SourceBoundary::Direct);
    let first = stores.comment("linear:T-3", "said in Linear");
    let gone = stores.comment("linear:T-3", "soon gone");
    let landed = stores.copy("linear:T-3", "notes");
    assert_eq!(counts(&landed), [2, 0, 0, 0], "{landed:#}");
    let note = destination(&landed);
    let held = stores.comments("linear:T-3");
    for (copy, origin) in stores.comments(&note).iter().zip(&held) {
        assert_eq!(copy["origin"]["task"], "linear:T-3");
        assert_eq!(copy["origin"]["id"], origin["id"]);
        assert_eq!(copy["origin"]["author"], origin["author"]);
        assert_eq!(copy["origin"]["url"], origin["url"]);
    }
    stores.edit("linear:T-3", &first, "said in Linear, then edited");
    stores.delete("linear:T-3", &gone);
    let followed = stores.copy("linear:T-3", "notes");
    assert_eq!(counts(&followed), [0, 1, 1, 0], "{followed:#}");
    assert_eq!(
        said(&stores.comments(&note)),
        said(&[json!({"body": "said in Linear, then edited",
                      "origin": {"task": "linear:T-3", "id": first}})]),
    );

    stores.note("B", "For Linear");
    let mine = stores.comment("notes:B", "from the folder");
    let landed = stores.copy("notes:B", "linear");
    assert_eq!(counts(&landed), [1, 0, 0, 0], "{landed:#}");
    let issue = destination(&landed);
    assert_eq!(
        said(&stores.comments(&issue)),
        said(&[json!({"body": "from the folder", "origin": {"task": "notes:B", "id": mine}})]),
        "the marker Linear keeps is read back out of the body"
    );
    stores.edit("notes:B", &mine, "from the folder, edited");
    assert_eq!(counts(&stores.copy("notes:B", "linear")), [0, 1, 0, 0]);
    stores.delete("notes:B", &mine);
    assert_eq!(counts(&stores.copy("notes:B", "linear")), [0, 0, 1, 0]);
    assert!(stores.comments(&issue).is_empty());
}

#[test]
fn a_board_tasks_comments_land_on_another_board_and_in_linear_and_follow_every_change() {
    for to in ["second", "linear"] {
        let stores = Stores::new(SourceBoundary::Direct);
        // A board creates no issue with labels, and this journey is about the comments.
        stores.board.set_labels("T-3", &[]);
        let first = stores.board.add_comment("T-3", "on the first board");
        let gone = stores.board.add_comment("T-3", "soon gone");
        let landed = stores.copy("board:T-3", to);
        assert_eq!(counts(&landed), [2, 0, 0, 0], "{to}: {landed:#}");
        let copied = destination(&landed);
        let held = stores.comments("board:T-3");
        for (copy, origin) in stores.comments(&copied).iter().zip(&held) {
            assert_eq!(copy["body"], origin["body"], "{to}");
            assert_eq!(
                copy["origin"],
                json!({"task": "board:T-3", "id": origin["id"], "author": GITHUB_COMMENTER,
                       "created_at": origin["created_at"], "updated_at": origin["updated_at"],
                       "url": origin["url"]}),
                "{to}"
            );
        }
        stores
            .board
            .edit_comment(&first, "on the first board, edited");
        stores.board.delete_comment(&gone);
        let followed = stores.copy("board:T-3", to);
        assert_eq!(counts(&followed), [0, 1, 1, 0], "{to}: {followed:#}");
        assert_eq!(
            said(&stores.comments(&copied)),
            said(&[json!({"body": "on the first board, edited",
                          "origin": {"task": "board:T-3", "id": first}})]),
            "{to}"
        );
    }
}

#[test]
fn a_chained_copy_names_the_first_origin_and_a_copy_back_from_either_hop_echoes_nothing() {
    // The board, then the folder, then the second board — which creates no issue with
    // labels, and this journey is about the comments.
    let stores = Stores::new(SourceBoundary::Direct);
    stores.board.set_labels("T-3", &[]);
    let said_first = stores.board.add_comment("T-3", "said on the board");
    let held = stores.comments("board:T-3")[0].clone();
    let note = destination(&stores.copy("board:T-3", "notes"));
    let written = stores.comment(&note, "written in the folder");
    let hop = stores.copy(&note, "second");
    assert_eq!(counts(&hop), [2, 0, 0, 0], "{hop:#}");
    let far = destination(&hop);
    let at_far = stores.comments(&far);
    let first_origin = at_far
        .iter()
        .find(|comment| comment["body"] == "said on the board")
        .expect("the board's comment, two hops on")["origin"]
        .clone();
    assert_eq!(
        first_origin,
        json!({"task": "board:T-3", "id": said_first, "author": GITHUB_COMMENTER,
               "created_at": held["created_at"], "updated_at": held["updated_at"],
               "url": held["url"]}),
        "the second hop names where the comment was written, never the folder"
    );
    assert_eq!(
        at_far
            .iter()
            .find(|comment| comment["body"] == "written in the folder")
            .expect("the folder's comment")["origin"]["task"],
        json!(note)
    );

    // Back from the first hop to the board: only the folder's comment.
    let back = stores.copy(&note, "board");
    assert_eq!(counts(&back), [1, 0, 0, 0], "{back:#}");
    // Back from the second hop to the folder it came from: only what was written there.
    stores.comment(&far, "written on the second board");
    let back = stores.copy(&far, "notes");
    assert_eq!(back["via"], "origin", "{back:#}");
    assert_eq!(counts(&back), [1, 0, 0, 1], "{back:#}");
    assert_eq!(
        said(&stores.comments(&note)),
        said(&[
            json!({"body": "said on the board", "origin": {"task": "board:T-3", "id": said_first}}),
            json!({"body": "written in the folder"}),
            json!({"body": "written on the second board",
                   "origin": {"task": far, "id": stores.comments(&far).iter()
                       .find(|comment| comment["body"] == "written on the second board")
                       .expect("the second board's own")["id"]}}),
        ]),
        "{written}: nothing of the folder's own is echoed back into it"
    );

    // The board, then the second board, then the folder: the folder names the first board.
    let stores = Stores::new(SourceBoundary::Direct);
    stores.board.set_labels("T-3", &[]);
    let origin = stores.board.add_comment("T-3", "said on the board");
    let middle = destination(&stores.copy("board:T-3", "second"));
    let last = destination(&stores.copy(&middle, "notes"));
    let at_last = stores.comments(&last);
    assert_eq!(at_last.len(), 1, "{at_last:#?}");
    assert_eq!(at_last[0]["origin"]["task"], "board:T-3");
    assert_eq!(at_last[0]["origin"]["id"], json!(origin));
    assert_eq!(at_last[0]["origin"]["author"], GITHUB_COMMENTER);
    // Back from the second hop to the second board: nothing but what the folder wrote.
    stores.comment(&last, "written at the end");
    let back = stores.copy(&last, "second");
    assert_eq!(counts(&back), [1, 0, 0, 1], "{back:#}");
    assert_eq!(stores.comments(&middle).len(), 2);
}

#[test]
fn a_project_copy_with_comments_carries_each_of_its_tasks_comments() {
    for boundary in SOURCE_BOUNDARIES {
        let who = format!("{boundary:?}");
        let stores = Stores::new(boundary);
        let said_first = stores.board.add_comment("T-1", "on the first task");
        let report = stores.json(&[
            "project",
            "copy",
            "board:P-1",
            "--to",
            "notes",
            "--comments",
        ]);
        let items = report["items"].as_array().expect("items");
        let project = items
            .iter()
            .find(|item| item["source"] == "board:P-1")
            .expect("the project");
        assert!(
            project.get("comments").is_none(),
            "{who}: a project has no comments to count: {project:#}"
        );
        let task = items
            .iter()
            .find(|item| item["source"] == "board:T-1")
            .expect("the task with a comment");
        assert_eq!(counts(task), [1, 0, 0, 0], "{who}: {task:#}");
        let quiet = items
            .iter()
            .find(|item| item["source"] == "board:T-2")
            .expect("the task without one");
        assert_eq!(counts(quiet), [0, 0, 0, 0], "{who}: {quiet:#}");
        assert_eq!(
            said(&stores.comments(&destination(task))),
            said(&[json!({"body": "on the first task",
                          "origin": {"task": "board:T-1", "id": said_first}})]),
            "{who}"
        );
        // Without the flag the same copy reports no comments at all.
        let flagless = stores.json(&["project", "copy", "board:P-1", "--to", "notes"]);
        assert!(
            flagless["items"]
                .as_array()
                .expect("items")
                .iter()
                .all(|item| item.get("comments").is_none()),
            "{who}: {flagless:#}"
        );
    }
}

/// A folder task `notes:A` with three comments, copied once onto the board, then changed in
/// the folder so the next copy deletes one copy, edits one and adds two: every kind of step.
/// Answers with the issue it landed as, the board's copy of the edited comment, and what the
/// board's comments said after the first copy.
fn changed_after_one_copy(stores: &Stores) -> (String, String, Vec<Said>) {
    stores.note("A", "Written in the folder");
    stores.comment("notes:A", "kept as written");
    let edited = stores.comment("notes:A", "about to change");
    let removed = stores.comment("notes:A", "about to go");
    let issue = destination(&stores.copy("notes:A", "board"));
    let held = stores.comments(&issue);
    let edited_copy = held
        .iter()
        .find(|comment| comment["origin"]["id"] == edited.as_str())
        .and_then(|comment| comment["id"].as_str())
        .expect("the board holds a copy of the edited comment")
        .to_owned();
    // Deleted before the additions, and usually in the same second: a folder once minted the
    // smallest number free in that second, which handed the first addition the deleted
    // comment's id, and this copy then edited the deleted comment's copy instead of deleting
    // it and adding one. Kept in this order as the regression.
    stores.edit("notes:A", &edited, "changed in the folder");
    stores.delete("notes:A", &removed);
    stores.comment("notes:A", "a new word");
    stores.comment("notes:A", "a second new word");
    (issue, edited_copy, said(&held))
}

#[test]
fn a_comments_copy_that_fails_part_way_takes_back_every_comment_write_or_names_what_is_left() {
    for undo_refused in [false, true] {
        let who = if undo_refused {
            "undo refused"
        } else {
            "undone"
        };
        let stores = Stores::new(SourceBoundary::Direct);
        let (issue, _, before) = changed_after_one_copy(&stores);
        // The deletion, the edit and the first addition land; the second addition is refused.
        stores.board.refuse_after("addComment(", 1);
        if undo_refused {
            // And taking the edit back is refused too: the forward edit is answered first.
            stores.board.refuse_after("updateIssueComment(", 1);
        }
        let output = stores.run(
            &["task", "copy", "notes:A", "--to", "board", "--comments"],
            None,
        );
        assert_ne!(output.status.code(), Some(0), "{who}: the copy failed");
        let refusal = stderr(&output);
        assert!(refusal.contains("addComment"), "{who}: {refusal}");
        let after = said(&stores.comments(&issue));
        if undo_refused {
            assert!(
                refusal.contains("could not be undone") && refusal.contains(&issue),
                "{who}: the refusal names the task still holding what the copy wrote: {refusal}"
            );
            assert!(
                after
                    .iter()
                    .any(|(body, origin)| body == "changed in the folder" && origin.is_some()),
                "{who}: the edit the destination would not take back is still there: {after:#?}"
            );
            assert!(
                !after.iter().any(|(body, _)| body == "a new word"),
                "{who}: every other write was still taken back: {after:#?}"
            );
        } else {
            assert!(!refusal.contains("could not be undone"), "{who}: {refusal}");
            assert_eq!(
                after, before,
                "{who}: the deleted copy is back, the edit reverted and the addition removed"
            );
        }
    }
}

#[test]
fn a_comments_copy_whose_destination_copy_vanishes_before_its_edit_writes_it_afresh() {
    let stores = Stores::new(SourceBoundary::Direct);
    let (issue, edited_copy, _) = changed_after_one_copy(&stores);
    // A person deletes the copy about to be edited, between the copy's plan and its edit:
    // deletions are written first, so once the board has answered the first one.
    stores.board.after_request(
        |document, _| document.contains("deleteIssueComment("),
        0,
        move |board| board.delete_comment(&edited_copy),
    );
    let followed = stores.copy("notes:A", "board");
    assert_eq!(counts(&followed), [2, 1, 1, 1], "{followed:#}");
    let local = stores.comments("notes:A");
    let origin_of = |body: &str| {
        let held = local
            .iter()
            .find(|comment| comment["body"] == body)
            .unwrap_or_else(|| panic!("the folder holds {body:?}"));
        json!({"task": "notes:A", "id": held["id"]})
    };
    assert_eq!(
        said(&stores.comments(&issue)),
        said(&[
            json!({"body": "kept as written", "origin": origin_of("kept as written")}),
            json!({"body": "changed in the folder", "origin": origin_of("changed in the folder")}),
            json!({"body": "a new word", "origin": origin_of("a new word")}),
            json!({"body": "a second new word", "origin": origin_of("a second new word")}),
        ]),
        "the vanished copy is written again, once, saying what its origin says now"
    );
}

#[test]
fn a_comments_dry_run_reports_what_it_would_write_and_changes_neither_end() {
    let stores = Stores::new(SourceBoundary::Direct);
    let (issue, _, _) = changed_after_one_copy(&stores);
    let folder = stores.file("A");
    let on_board = stores.comments(&issue);
    let sent = stores.board.served().len();

    let report = stores.json(&[
        "task",
        "copy",
        "notes:A",
        "--to",
        "board",
        "--comments",
        "--dry-run",
    ]);
    let planned = &report["items"][0];
    assert_eq!(counts(planned), [2, 1, 1, 1], "{report:#}");
    assert!(
        !stores.board.served()[sent..]
            .iter()
            .any(|(document, _)| document.trim_start().starts_with("mutation")),
        "a dry run sends the board no write"
    );
    assert_eq!(
        stores.comments(&issue),
        on_board,
        "the board's comments are as they were"
    );
    assert_eq!(
        stores.file("A"),
        folder,
        "the folder's file is byte for byte as it was"
    );

    let copied = stores.copy("notes:A", "board");
    assert_eq!(
        counts(&copied),
        counts(planned),
        "the copy writes exactly what its dry run reported"
    );
}

#[test]
fn a_comments_copy_into_a_source_holding_no_comments_is_refused_before_anything_is_read() {
    let sandbox = Sandbox::new();
    let (board_config, board) = github_projects_with_board(&sandbox);
    sandbox.secrets_file("GITHUB_PROJECTS_FIXTURE_TOKEN=test-token\n");
    let notes = sandbox.subdirectory("notes");
    std::fs::create_dir_all(notes.join("tasks")).expect("the task folder");
    sandbox.project_document(&document(&json!({
        "board": {"plugin": "github-projects", "config": board_config},
        "memory": {"plugin": "in-memory", "config": {}},
        // A folder whose tasks hold comments, routing some tasks to a source whose hold none.
        "notes": {
            "plugin": "local-md",
            "config": {"root": notes},
            "routes": [{"repositories": ["github.com/widgetco/*"], "to": "memory"}],
        },
    })));
    for (verb, item, to) in [
        ("task", "board:T-3", "memory"),
        ("task", "board:T-3", "notes"),
        ("project", "board:P-1", "notes"),
    ] {
        let who = format!("{verb} copy into {to}");
        let sent = board.served().len();
        let mut command = sandbox.command();
        command.args([verb, "copy", item, "--to", to, "--comments"]);
        let output = command.assert().get_output().clone();
        assert_ne!(output.status.code(), Some(0), "{who}");
        let refusal = stderr(&output);
        assert!(
            refusal.contains("--comments cannot carry comments into memory")
                && refusal.contains("in-memory")
                && refusal.contains("next:"),
            "{who}: names the source that holds none, its plugin and what to do: {refusal}"
        );
        assert_eq!(
            board.served().len(),
            sent,
            "{who}: refused before the board was asked anything"
        );
        assert_eq!(
            std::fs::read_dir(notes.join("tasks"))
                .expect("the folder")
                .count(),
            0,
            "{who}: nothing was written"
        );
    }
}

/// A folder of Markdown configured as `old` behind the relay, which strips `task_details` and
/// `comment_copies` from the host's handshake so it reads as a plugin written before either
/// existed, and logs every method the engine sends it — beside the board, as `board`.
fn older_peer() -> (Stores, PathBuf) {
    let sandbox = Sandbox::new();
    let (board_config, board) = github_projects_with_board(&sandbox);
    sandbox.secrets_file("GITHUB_PROJECTS_FIXTURE_TOKEN=test-token\n");
    let old = sandbox.subdirectory("old");
    std::fs::create_dir_all(old.join("tasks")).expect("the task folder");
    let log = sandbox.project().join("methods.log");
    let relay =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/e2e/rewriting_relay.py");
    sandbox.project_document(&document(&json!({
        "board": {"plugin": "github-projects", "config": board_config},
        "old": {"plugin": "subprocess", "config": {
            "command": interpreter().to_string_lossy(),
            "args": [relay.to_string_lossy()],
            "settings": {
                "host": onetaskgraph_e2e_support::source_binary(),
                "forget": ["task_details", "comment_copies"],
                "log": log,
                "kind": "local-md",
                "config": {"root": old, "status_mapping": {
                    "todo": "todo", "Todo": "todo", "Doing": "in-progress",
                    "Shipped": "done", "Done": "done"}},
            },
        }},
    })));
    (
        Stores {
            sandbox,
            board,
            notes: old,
        },
        log,
    )
}

/// The methods the engine sent the older peer since the log was last taken, and empty it.
fn taken(log: &PathBuf) -> Vec<String> {
    let sent = std::fs::read_to_string(log).unwrap_or_default();
    std::fs::write(log, "").expect("the log");
    sent.lines().map(str::to_owned).collect()
}

#[test]
fn an_older_peer_is_sent_comment_copies_as_marked_comments_and_read_in_two_calls() {
    let (stores, log) = older_peer();
    let first = stores.board.add_comment("T-3", "the first word");
    let second = stores.board.add_comment("T-3", "the second word");

    // Into the older peer: each copy goes as an ordinary comment carrying the marker.
    let landed = stores.copy("board:T-3", "old");
    assert_eq!(counts(&landed), [2, 0, 0, 0], "{landed:#}");
    let task = destination(&landed);
    let sent = taken(&log);
    for never in ["write_comment_copy", "get_task_detail"] {
        assert!(
            !sent.iter().any(|method| method == never),
            "a peer that did not declare it is never sent {never}: {sent:?}"
        );
    }
    assert!(
        sent.iter().any(|method| method == "add_comment"),
        "{sent:?}"
    );
    assert!(
        stores
            .file(native(&task))
            .contains("onetaskgraph.comment_origin"),
        "the origin travels as the marker in the body the peer stores"
    );
    assert_eq!(
        said(&stores.comments(&task)),
        said(&[
            json!({"body": "the first word", "origin": {"task": "board:T-3", "id": first}}),
            json!({"body": "the second word", "origin": {"task": "board:T-3", "id": second}}),
        ]),
        "and is read back out of it as the origin, the body as the board wrote it"
    );

    // An edit at the board edits the copy through the peer's own edit, marker kept.
    stores.board.edit_comment(&first, "the first word, edited");
    taken(&log);
    let followed = stores.copy("board:T-3", "old");
    assert_eq!(counts(&followed), [0, 1, 0, 1], "{followed:#}");
    let sent = taken(&log);
    assert!(
        sent.iter().any(|method| method == "edit_comment"),
        "{sent:?}"
    );
    assert!(
        !sent.iter().any(|method| method == "write_comment_copy"),
        "{sent:?}"
    );
    assert_eq!(
        said(&stores.comments(&task)),
        said(&[
            json!({"body": "the first word, edited",
                   "origin": {"task": "board:T-3", "id": first}}),
            json!({"body": "the second word", "origin": {"task": "board:T-3", "id": second}}),
        ])
    );

    // Out of the older peer: its task and its comments are read in two calls, and only the
    // comment written there goes back — none of the board's own is echoed.
    let local = stores.comment(&task, "written behind the older peer");
    taken(&log);
    let back = stores.copy(&task, "board");
    assert_eq!(back["destination"], "board:T-3", "{back:#}");
    assert_eq!(counts(&back)[..3], [1, 0, 0], "{back:#}");
    let sent = taken(&log);
    assert!(
        sent.iter().any(|method| method == "get_task")
            && sent.iter().any(|method| method == "task_comments")
            && !sent.iter().any(|method| method == "get_task_detail"),
        "the task and its comments are read through the two methods the peer answers: {sent:?}"
    );
    assert!(
        said(&stores.comments("board:T-3")).contains(&(
            "written behind the older peer".to_owned(),
            Some((task.clone(), local))
        )),
        "the peer's own comment lands on the board with its origin"
    );
}

#[test]
fn the_human_report_counts_each_tasks_comments_and_says_which_were_skipped() {
    let stores = Stores::new(SourceBoundary::Direct);
    // In the board's own wording, `Todo`: a status compares by its name as well as its
    // category, so a folder's `todo` would read as changed on every copy and never skip.
    std::fs::write(
        stores.notes.join("tasks/A.md"),
        "---\ntitle: Written in the folder\nstatus: Todo\n---\nWritten here.\n",
    )
    .expect("a task file");
    for said in ["one", "two", "three"] {
        stores.comment("notes:A", said);
    }
    let copy = || {
        let output = stores.run(
            // No link written back, so the folder's file — and its token — stay as they were.
            &[
                "task",
                "copy",
                "notes:A",
                "--to",
                "board",
                "--comments",
                "--no-link",
            ],
            None,
        );
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        let said = stdout(&output);
        said.lines()
            .find(|line| line.starts_with("notes:A"))
            .unwrap_or_else(|| panic!("notes:A has a row:\n{said}"))
            .to_owned()
    };
    let first = copy();
    assert!(
        first.contains("created")
            && first.contains("comments: 3 added, 0 edited, 0 deleted, 0 unchanged"),
        "{first}"
    );
    let again = copy();
    assert!(
        again.contains("unchanged (skipped)")
            && again.contains("comments: 0 added, 0 edited, 0 deleted, 3 unchanged"),
        "a task decided from its change tokens alone says it was skipped: {again}"
    );
}
