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
fn said(comments: &[Value]) -> Vec<(String, Option<(String, String)>)> {
    let mut said: Vec<(String, Option<(String, String)>)> = comments
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
