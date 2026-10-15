//! What a repeat copy costs, and what it may never serve: a task unchanged at both ends since
//! it was last copied is skipped from its change tokens alone, and anything changed at either
//! end since is read and copied again.
//!
//! Every journey drives the binary against loopback boards and a folder of Markdown and holds
//! a copy to two things at once: what it reported, and what the board's own request log shows
//! it asked for. A skip that served data that had changed would pass the first and fail the
//! second's partner journey, which changes one thing at a time and requires that item, and only
//! that item, to be read again.

use std::process::Output;

use onetaskgraph_github_projects::{change_token, graphql};
use serde_json::{Value, json};

use crate::change_tokens::{TRANSFERRED_TO, far_ends};
use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{GitHubBoardFields, document, github_projects_with_board};

/// A board, a second board and a folder of Markdown, configured together.
struct Stores {
    sandbox: Sandbox,
    board: GitHubBoardFields,
    second: GitHubBoardFields,
}

impl Stores {
    fn new() -> Self {
        let sandbox = Sandbox::new();
        let (board_config, board) = github_projects_with_board(&sandbox);
        let (second_config, second) = github_projects_with_board(&sandbox);
        let notes = sandbox.subdirectory("notes");
        std::fs::create_dir_all(notes.join("tasks")).expect("the task folder");
        sandbox.project_document(&document(&json!({
            "board": {"plugin": "github-projects", "config": board_config},
            "second": {"plugin": "github-projects", "config": second_config},
            "notes": {"plugin": "local-md", "config": {"root": notes, "status_mapping": {
                "todo": "todo", "Todo": "todo", "Doing": "in-progress",
                "Shipped": "done", "Done": "done"}}},
        })));
        // A board creates no issue with labels, and a second board spells the first's own
        // `Shipped` as `Done`, so a copy of the closed T-2 never reads back as it went; these
        // journeys are about what a copy reads.
        for task in ["T-1", "T-2", "T-3", "T-4"] {
            board.set_labels(task, &[]);
        }
        board.reopen("T-2");
        board.move_card("T-2", "Todo");
        Self {
            sandbox,
            board,
            second,
        }
    }

    fn run(&self, arguments: &[&str], input: Option<&str>) -> Output {
        let mut command = self.sandbox.command();
        command.args(arguments);
        command.write_stdin(input.unwrap_or_default());
        command.assert().get_output().clone()
    }

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

    /// One copy, answering with its outcomes by source id, and every request each board was
    /// sent while it ran.
    fn copy(&self, arguments: &[&str]) -> (Vec<(String, Value)>, Served, Served) {
        let (board, second) = (self.board.served().len(), self.second.served().len());
        let mut copy = vec!["task", "copy"];
        copy.extend_from_slice(arguments);
        let report = self.json(&copy);
        let outcomes = report["items"]
            .as_array()
            .expect("items")
            .iter()
            .map(|item| {
                (
                    item["source"].as_str().expect("a source").to_owned(),
                    item.clone(),
                )
            })
            .collect();
        (
            outcomes,
            Served(self.board.served()[board..].to_vec()),
            Served(self.second.served()[second..].to_vec()),
        )
    }

    fn comments(&self, id: &str) -> Vec<Value> {
        self.json(&["task", "comment", "list", id])["comments"]
            .as_array()
            .expect("comments")
            .clone()
    }

    /// The qualified ids the task `id` depends on, in the order its source lists them.
    fn depends_on(&self, id: &str) -> Vec<String> {
        self.json(&["task", "deps", id, "--direction", "depends-on"])["items"]
            .as_array()
            .expect("items")
            .iter()
            .map(|edge| edge["to"]["id"].as_str().expect("a far end").to_owned())
            .collect()
    }

    fn metadata(&self, id: &str) -> Value {
        self.task(id)["metadata"].clone()
    }

    /// Write `key` into the metadata the destination task `id` holds, as its own store keeps
    /// it — a key nothing a caller types may set.
    fn plant(&self, id: &str, key: &str, value: &Value) {
        let (source, native) = id.split_once(':').expect("a qualified id");
        if source == "second" {
            self.second.set_metadata(native, key, value.clone());
            return;
        }
        let path = self.task(id)["location"]["path"]
            .as_str()
            .expect("a folder task's path")
            .to_owned();
        let held = std::fs::read_to_string(&path).expect("the task file");
        let planted = held.replacen(
            "\nmetadata:\n",
            &format!("\nmetadata:\n  {key}: {value}\n"),
            1,
        );
        assert_ne!(planted, held, "the task file holds metadata: {held}");
        std::fs::write(&path, planted).expect("the task file written");
    }

    fn task(&self, id: &str) -> Value {
        self.json(&["task", "show", id, "--no-comments"])["items"][0]["item"].clone()
    }
}

/// Every request one board answered during one command.
#[derive(Debug)]
struct Served(Vec<(String, Value)>);

impl Served {
    /// Each request as the plugin's own name for its document.
    fn names(&self) -> Vec<&'static str> {
        self.0.iter().map(|(document, _)| name(document)).collect()
    }

    /// The requests that name the issue `id` in their variables.
    fn naming(&self, id: &str) -> Vec<&'static str> {
        self.0
            .iter()
            .filter(|(_, variables)| names(variables, id))
            .map(|(document, _)| name(document))
            .collect()
    }

    /// Whether every request was a read.
    fn reads_only(&self) -> bool {
        self.0
            .iter()
            .all(|(document, _)| !document.trim_start().starts_with("mutation"))
    }
}

/// The plugin's name for one read it sends, or `"a mutation"` for a write and `"another
/// read"` for any read this list does not name.
fn name(document: &str) -> &'static str {
    [
        ("BOARD", graphql::BOARD),
        ("ISSUE_COPY_DETAILS", graphql::ISSUE_COPY_DETAILS),
        ("SEARCH_ISSUES", graphql::SEARCH_ISSUES),
        ("ISSUE", graphql::ISSUE),
        ("ISSUE_DETAIL", graphql::ISSUE_DETAIL),
        ("ISSUE_DETAILS", graphql::ISSUE_DETAILS),
        ("ISSUE_COMMENTS", graphql::ISSUE_COMMENTS),
        ("ISSUE_DEPENDENCIES", graphql::ISSUE_DEPENDENCIES),
        ("ORIGIN_LOOKUP", graphql::ORIGIN_LOOKUP),
        ("BOARD_FIELDS", graphql::BOARD_FIELDS),
        ("CREATION_CONTEXT", graphql::CREATION_CONTEXT),
        ("SUB_ISSUES", graphql::SUB_ISSUES),
        ("COMMENT_ISSUE", graphql::COMMENT_ISSUE),
    ]
    .into_iter()
    .find(|(_, held)| *held == document)
    .map_or_else(
        || {
            if document.trim_start().starts_with("mutation") {
                "a mutation"
            } else {
                "another read"
            }
        },
        |(name, _)| name,
    )
}

/// Whether a request's variables name the issue `id`, at any depth.
fn names(variables: &Value, id: &str) -> bool {
    match variables {
        Value::String(held) => held == id,
        Value::Array(items) => items.iter().any(|item| names(item, id)),
        Value::Object(members) => members.values().any(|member| names(member, id)),
        _ => false,
    }
}

/// Whether an outcome is an unchanged item decided from tokens alone.
fn skipped(outcome: &Value) -> bool {
    outcome["action"] == "unchanged" && outcome["skipped"] == true
}

/// A board's listing of its tasks: every request a listing sends, and no other.
const LISTING: [&str; 2] = ["BOARD", "SEARCH_ISSUES"];

#[test]
fn a_recopy_by_query_of_unchanged_board_items_skips_each_and_sends_nothing_but_the_listing() {
    let stores = Stores::new();
    let first = stores.board.add_comment("T-3", "the first word");
    let second = stores.board.add_comment("T-3", "the second word");
    let query = [
        "--to",
        "notes",
        "--source",
        "board",
        "--comments",
        "--no-link",
    ];
    let (landed, _, _) = stores.copy(&query);
    assert!(
        landed
            .iter()
            .all(|(_, outcome)| outcome["action"] == "created"),
        "{landed:#?}"
    );
    let note = landed
        .iter()
        .find(|(source, _)| source == "board:T-3")
        .map(|(_, outcome)| outcome["destination"].as_str().expect("an id").to_owned())
        .expect("T-3 landed");
    let origin_of: std::collections::BTreeMap<&str, &str> = landed
        .iter()
        .map(|(source, outcome)| {
            (
                outcome["destination"].as_str().expect("an id"),
                source.as_str(),
            )
        })
        .collect();
    let record = stores.metadata(&note);
    assert!(
        record["onetaskgraph.copied_version"]
            .as_str()
            .is_some_and(|token| token.starts_with(change_token::PREFIX)),
        "the copy records the source's token on the destination: {record:#}"
    );
    assert_eq!(record["onetaskgraph.copied_with_comments"], json!(true));

    let (again, served, _) = stores.copy(&query);
    assert!(
        again.iter().all(|(_, outcome)| skipped(outcome)),
        "{again:#?}"
    );
    assert!(
        served.names().iter().all(|name| LISTING.contains(name)),
        "a skipped re-copy sends the listing and nothing else: {:?}",
        served.names()
    );

    // One change at a time on one task: that task, and no other, is read and copied again.
    type Change = Box<dyn Fn(&GitHubBoardFields)>;
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
            Box::new(|board| board.edit_body("T-3", "rewritten")),
        ),
        (
            "a title edit",
            Box::new(|board| board.edit_title("T-3", "Renamed")),
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
            Box::new(|board| board.set_card_field("T-3", "Estimate", "2 days")),
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
        (
            "a dependency replaced",
            Box::new(|board| {
                board.unblock("T-3", "T-4");
                board.block("T-3", &["T-1".to_owned()]);
            }),
        ),
        (
            "filed under a project",
            Box::new(|board| board.file_under("T-3", Some("P-1"))),
        ),
        (
            "moved to another project",
            Box::new(|board| board.file_under("T-3", Some("P-2"))),
        ),
        (
            "taken out of its project",
            Box::new(|board| board.file_under("T-3", None)),
        ),
        (
            "the origin field rewritten",
            Box::new(|board| board.set_origin_field("T-3", "elsewhere:X-1")),
        ),
        (
            "a repository transfer",
            Box::new(|board| board.transfer("T-3", TRANSFERRED_TO)),
        ),
    ];
    for (kind, change) in changes {
        change(&stores.board);
        let (after, served, _) = stores.copy(&query);
        // A dependency is recorded on both of the issues it relates, so its far end is read
        // again too.
        let mut touched = vec!["board:T-3"];
        touched.extend(far_ends(kind));
        for (source, outcome) in &after {
            assert_eq!(
                skipped(outcome),
                !touched.contains(&source.as_str()),
                "{kind}: {source} is read again exactly when the change reached it: {outcome:#}"
            );
        }
        for untouched in ["T-1", "T-2", "T-4"]
            .into_iter()
            .filter(|task| !touched.contains(&format!("board:{task}").as_str()))
        {
            let reads: Vec<&str> = served
                .naming(untouched)
                .into_iter()
                .filter(|name| !LISTING.contains(name))
                .collect();
            assert!(
                reads.is_empty(),
                "{kind}: {untouched} was not read beyond the listing: {reads:?}"
            );
        }
        assert!(
            !served.naming("T-3").is_empty(),
            "{kind}: the changed task was read: {:?}",
            served.names()
        );
        let (settled, _, _) = stores.copy(&query);
        assert!(
            settled.iter().all(|(_, outcome)| skipped(outcome)),
            "{kind}: once copied, the change is skipped again: {settled:#?}"
        );
        // A skip never serves a stale edge: the copy's edges are the board's, a far end the
        // folder names by its own counterpart read as the board task it was copied from.
        let mut copied: Vec<String> = stores
            .depends_on(&note)
            .into_iter()
            .map(|far| {
                origin_of
                    .get(far.as_str())
                    .map_or(far.clone(), |id| (*id).to_owned())
            })
            .collect();
        let mut expected = stores.depends_on("board:T-3");
        copied.sort();
        expected.sort();
        assert_eq!(
            copied, expected,
            "{kind}: the folder's copy holds the board's edges"
        );
        // Nor a stale project or repository: the copy reads where the board files T-3 now.
        let (copy, board) = (stores.task(&note), stores.task("board:T-3"));
        assert_eq!(
            copy["project"].is_null(),
            board["project"].is_null(),
            "{kind}: the folder's copy is in a project exactly when the board's task is: \
             {copy:#} against {board:#}"
        );
        assert_eq!(
            copy["repositories"], board["repositories"],
            "{kind}: the folder's copy names the board task's repository"
        );
    }
    // What the folder holds is what the board holds now.
    assert_eq!(
        stores
            .comments(&note)
            .iter()
            .map(|comment| comment["body"].clone())
            .collect::<Vec<_>>(),
        stores
            .comments("board:T-3")
            .iter()
            .map(|comment| comment["body"].clone())
            .collect::<Vec<_>>()
    );
}

#[test]
fn a_copy_that_adds_comments_to_an_unchanged_source_carries_them_and_then_skips() {
    let stores = Stores::new();
    stores
        .board
        .add_comment("T-1", "said before the first copy");
    let ids = ["board:T-1", "board:T-3", "--to", "notes"];
    let (first, _, _) = stores.copy(&ids);
    let note = first[0].1["destination"]
        .as_str()
        .expect("an id")
        .to_owned();
    let record = stores.metadata(&note);
    assert!(
        record["onetaskgraph.copied_version"].is_string(),
        "{record:#}"
    );
    assert!(
        record.get("onetaskgraph.copied_with_comments").is_none(),
        "{record:#}"
    );
    assert!(
        stores.comments(&note).is_empty(),
        "a copy without the flag carries none"
    );

    let with = [&ids[..], &["--comments"]].concat();
    let (second, _, _) = stores.copy(&with);
    let carried = &second[0].1;
    assert!(!skipped(carried), "{carried:#}");
    assert_eq!(carried["comments"]["added"], 1, "{carried:#}");
    assert_eq!(stores.comments(&note).len(), 1);
    assert_eq!(
        stores.metadata(&note)["onetaskgraph.copied_with_comments"],
        json!(true)
    );

    let (third, _, _) = stores.copy(&with);
    assert!(
        third.iter().all(|(_, outcome)| skipped(outcome)),
        "{third:#?}"
    );
}

#[test]
fn a_flagless_copy_between_two_comment_copies_does_not_let_the_second_skip_a_change() {
    let stores = Stores::new();
    let said = stores.board.add_comment("T-3", "the original word");
    let with = ["board:T-3", "--to", "notes", "--comments"];
    let (first, _, _) = stores.copy(&with);
    let note = first[0].1["destination"]
        .as_str()
        .expect("an id")
        .to_owned();
    let recorded = stores.metadata(&note)["onetaskgraph.copied_version"].clone();

    stores.board.edit_comment(&said, "the word, changed");
    let (flagless, _, _) = stores.copy(&["board:T-3", "--to", "notes"]);
    assert_eq!(flagless[0].1["action"], "unchanged", "{flagless:#?}");
    let record = stores.metadata(&note);
    assert_ne!(
        record["onetaskgraph.copied_version"], recorded,
        "the flagless copy advances the version it records: {record:#}"
    );
    assert!(
        record.get("onetaskgraph.copied_with_comments").is_none(),
        "and records that it carried no comments: {record:#}"
    );
    assert_eq!(
        stores.comments(&note)[0]["body"],
        "the original word",
        "a copy without the flag touches no comment"
    );

    let (carried, _, _) = stores.copy(&with);
    let outcome = &carried[0].1;
    assert!(!skipped(outcome), "{outcome:#}");
    assert_eq!(outcome["comments"]["edited"], 1, "{outcome:#}");
    assert_eq!(
        stores
            .comments(&note)
            .iter()
            .map(|comment| comment["body"].clone())
            .collect::<Vec<_>>(),
        vec![json!("the word, changed")]
    );
}

#[test]
fn an_edit_at_the_destination_is_restored_and_a_comment_written_there_stops_one_skip() {
    for to in ["notes", "second"] {
        let stores = Stores::new();
        stores.board.add_comment("T-3", "said on the board");
        let query = ["--to", to, "--source", "board", "--comments", "--no-link"];
        let (landed, _, _) = stores.copy(&query);
        let copy = landed
            .iter()
            .find(|(source, _)| source == "board:T-3")
            .map(|(_, outcome)| outcome["destination"].as_str().expect("an id").to_owned())
            .expect("T-3 landed");
        let copied = stores.comments(&copy)[0]["id"]
            .as_str()
            .expect("an id")
            .to_owned();

        // A copied comment edited where it landed: restored to the source's words.
        let output = stores.run(
            &["task", "comment", "edit", &copy, &copied],
            Some("tampered"),
        );
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        let (restored, served, _) = stores.copy(&query);
        let outcome = &restored
            .iter()
            .find(|(source, _)| source == "board:T-3")
            .expect("T-3")
            .1;
        assert!(!skipped(outcome), "{to}: {outcome:#}");
        assert_eq!(outcome["comments"]["edited"], 1, "{to}: {outcome:#}");
        assert!(
            !served.naming("T-3").is_empty(),
            "{to}: the source was read again"
        );
        assert_eq!(
            stores.comments(&copy)[0]["body"],
            "said on the board",
            "{to}"
        );

        // Deleted where it landed: written again.
        let output = stores.run(&["task", "comment", "delete", &copy, &copied], None);
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        let (rewritten, _, _) = stores.copy(&query);
        let outcome = &rewritten
            .iter()
            .find(|(source, _)| source == "board:T-3")
            .expect("T-3")
            .1;
        assert_eq!(outcome["comments"]["added"], 1, "{to}: {outcome:#}");
        assert_eq!(stores.comments(&copy).len(), 1, "{to}");

        // A comment written where it landed: no skip, nothing touched, and then skips again.
        let output = stores.run(&["task", "comment", "add", &copy], Some("written here"));
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        let (noticed, _, found) = stores.copy(&query);
        let outcome = &noticed
            .iter()
            .find(|(source, _)| source == "board:T-3")
            .expect("T-3")
            .1;
        assert_eq!(outcome["action"], "unchanged", "{to}: {outcome:#}");
        assert_eq!(outcome["skipped"], false, "{to}: {outcome:#}");
        assert!(
            stores
                .comments(&copy)
                .iter()
                .any(|comment| comment["body"] == "written here"),
            "{to}: the destination's own comment stays"
        );
        let (again, board, second) = stores.copy(&query);
        assert!(
            again.iter().all(|(_, outcome)| skipped(outcome)),
            "{to}: {again:#?}"
        );
        assert!(
            board.names().iter().all(|name| LISTING.contains(name)),
            "{to}: the source is sent its listing alone: {:?}",
            board.names()
        );
        if to == "second" {
            // The destination check is made over the reads the copy makes anyway to find each
            // counterpart and read what it holds, and asks for nothing of its own: a skipped
            // re-copy writes nothing and reads nothing the copy before it — which read the
            // same counterparts and did not skip — did not read too.
            assert!(second.reads_only(), "{:?}", second.names());
            let mut read: Vec<&str> = found
                .0
                .iter()
                .filter(|(document, _)| !document.trim_start().starts_with("mutation"))
                .map(|(document, _)| name(document))
                .collect();
            for asked in second.names() {
                let at = read
                    .iter()
                    .position(|held| *held == asked)
                    .unwrap_or_else(|| {
                        panic!("a skipped re-copy read {asked} beyond what a copy reads: {read:?}")
                    });
                read.remove(at);
            }
        }
    }
}

#[test]
fn no_link_writes_nothing_to_the_source_and_the_next_copy_finds_the_counterpart() {
    let stores = Stores::new();
    let (landed, served, _) = stores.copy(&["board:T-3", "--to", "notes", "--no-link"]);
    let outcome = &landed[0].1;
    assert_eq!(outcome["action"], "created", "{outcome:#}");
    assert_eq!(outcome["link"], "skipped", "{outcome:#}");
    assert!(served.reads_only(), "{:?}", served.names());
    for again in [
        vec!["board:T-3", "--to", "notes", "--no-link"],
        vec!["board:T-3", "--to", "notes"],
    ] {
        let (found, _, _) = stores.copy(&again);
        let outcome = &found[0].1;
        assert_ne!(outcome["action"], "created", "{outcome:#}");
        assert_eq!(outcome["via"], "scan", "found by its origin: {outcome:#}");
        assert_eq!(outcome["destination"], landed[0].1["destination"]);
    }
}

#[test]
fn a_copy_by_query_copies_what_task_list_answers_and_reads_each_tasks_comments_once() {
    let stores = Stores::new();
    for task in ["T-1", "T-3"] {
        stores.board.set_metadata(
            task,
            "orchestrator.follow-up",
            json!({"root_cause": "stale-cache"}),
        );
        stores.board.add_comment(task, "recent activity");
    }
    stores.board.set_metadata(
        "T-4",
        "orchestrator.follow-up",
        json!({"root_cause": "stale-cache"}),
    );
    let narrowing = [
        "--source",
        "board",
        "--commented-since",
        "2026-09-01T00:00:00Z",
        "--metadata",
        "orchestrator.follow-up/root_cause=stale-cache",
    ];
    let listed: Vec<String> = stores.json(&[&["task", "list"][..], &narrowing].concat())["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|task| task["id"].as_str().expect("an id").to_owned())
        .collect();
    assert_eq!(listed, vec!["board:T-1", "board:T-3"]);
    let (copied, served, _) = stores.copy(
        &[
            &["--to", "notes", "--comments", "--no-link"][..],
            &narrowing,
        ]
        .concat(),
    );
    assert_eq!(
        copied
            .iter()
            .map(|(source, _)| source.clone())
            .collect::<Vec<_>>(),
        listed,
        "a copy by query copies exactly what the list answers"
    );
    for task in ["T-1", "T-3", "T-4"] {
        let comment_reads = served
            .0
            .iter()
            .filter(|(document, variables)| {
                names(variables, task)
                    && (document == graphql::ISSUE_COMMENTS
                        || document == graphql::ISSUE_DETAIL
                        || (document == graphql::ISSUE_DETAILS && variables["comments"] == true))
            })
            .count();
        assert!(
            comment_reads <= 1,
            "{task}'s comments were read {comment_reads} times: {:?}",
            served.names()
        );
    }

    let output = stores.run(
        &[
            "task",
            "copy",
            "board:T-1",
            "--to",
            "notes",
            "--source",
            "board",
        ],
        None,
    );
    assert_eq!(
        output.status.code(),
        Some(2),
        "ids and a query together are refused"
    );
    assert!(stderr(&output).contains("--source"), "{}", stderr(&output));
}

#[test]
fn a_copy_by_query_matching_nothing_succeeds_reading_only_the_listing_and_writes_nothing() {
    let stores = Stores::new();
    let (copied, served, second) = stores.copy(&[
        "--to",
        "second",
        "--comments",
        "--source",
        "board",
        "--metadata",
        "orchestrator.follow-up/root_cause=nothing-says-this",
    ]);
    assert!(copied.is_empty(), "{copied:?}");
    assert!(
        served.names().iter().all(|name| LISTING.contains(name)),
        "the source was sent its listing and nothing else: {:?}",
        served.names()
    );
    assert!(
        second.0.is_empty(),
        "the destination was not asked anything: {:?}",
        second.names()
    );
}

#[test]
fn a_copy_narrowed_by_comment_activity_without_comments_copies_the_right_tasks_and_no_comment() {
    let stores = Stores::new();
    for task in ["T-1", "T-3"] {
        stores.board.add_comment(task, "recent activity");
    }
    let narrowing = [
        "--source",
        "board",
        "--commented-since",
        "2026-09-01T00:00:00Z",
    ];
    let listed: Vec<String> = stores.json(&[&["task", "list"][..], &narrowing].concat())["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|task| task["id"].as_str().expect("an id").to_owned())
        .collect();
    assert_eq!(listed, vec!["board:T-1", "board:T-3"]);

    let copy = [&["--to", "notes", "--no-link"][..], &narrowing].concat();
    let (copied, _, _) = stores.copy(&copy);
    let sources: Vec<&str> = copied.iter().map(|(source, _)| source.as_str()).collect();
    assert_eq!(sources, listed, "exactly what the list answers");
    for (source, outcome) in &copied {
        assert!(
            outcome.get("comments").is_none(),
            "{source}: a copy without --comments reports no comments: {outcome:#}"
        );
        let landed = outcome["destination"].as_str().expect("a destination");
        assert!(
            stores.comments(landed).is_empty(),
            "{source}: no comment of the board's is carried without --comments"
        );
    }

    // A comment written at the destination is the destination's own, and a copy without
    // --comments leaves it exactly as it is.
    let landed = copied[0].1["destination"]
        .as_str()
        .expect("a destination")
        .to_owned();
    let output = stores.run(&["task", "comment", "add", &landed], Some("written here"));
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    stores.board.add_comment("T-1", "more activity");
    let (again, _, _) = stores.copy(&copy);
    assert_eq!(again.len(), 2, "{again:?}");
    let held = stores.comments(&landed);
    assert_eq!(held.len(), 1, "{held:#?}");
    assert_eq!(held[0]["body"], "written here");
    assert!(held[0].get("origin").is_none(), "{held:#?}");
}

#[test]
fn a_copy_by_query_one_of_whose_sources_fails_is_refused_and_writes_nothing_anywhere() {
    let sandbox = Sandbox::new();
    let (board_config, board) = github_projects_with_board(&sandbox);
    let notes = sandbox.subdirectory("notes");
    std::fs::create_dir_all(notes.join("tasks")).expect("the task folder");
    sandbox.project_document(&document(&json!({
        "board": {"plugin": "github-projects", "config": board_config},
        // Every status the board uses, so the half that answered could land if it were let.
        "notes": {"plugin": "local-md", "config": {"root": notes, "status_mapping": {
            "Todo": "todo", "Doing": "in-progress", "Shipped": "done", "Done": "done"}}},
        // A plugin whose process cannot be started: its listing fails, the board's does not.
        "dead": {"plugin": "subprocess", "config": {
            "command": sandbox.subdirectory("nowhere").join("no-such-peer")}},
    })));
    let mark = board.served().len();
    let output = sandbox
        .command()
        .args([
            "task",
            "copy",
            "--to",
            "notes",
            "--source",
            "board",
            "--source",
            "dead",
            "--comments",
            "--json",
        ])
        .write_stdin("")
        .assert()
        .get_output()
        .clone();
    assert_ne!(output.status.code(), Some(0), "{}", stdout(&output));
    assert!(stderr(&output).contains("dead"), "{}", stderr(&output));
    let written: Vec<_> = std::fs::read_dir(notes.join("tasks"))
        .expect("the task folder")
        .collect();
    assert!(
        written.is_empty(),
        "nothing landed from the half of the query that answered: {written:?}"
    );
    let served = Served(board.served()[mark..].to_vec());
    assert!(
        served.reads_only(),
        "nothing was written to the board: {:?}",
        served.names()
    );
}

/// `value` as canonical JSON: keys sorted at every depth, no insignificant whitespace.
fn canonical(value: &Value) -> String {
    match value {
        Value::Object(entries) => {
            let sorted: std::collections::BTreeMap<&String, &Value> = entries.iter().collect();
            let inner: Vec<String> = sorted
                .into_iter()
                .map(|(key, entry)| format!("{}:{}", Value::String(key.clone()), canonical(entry)))
                .collect();
            format!("{{{}}}", inner.join(","))
        }
        Value::Array(entries) => {
            let inner: Vec<String> = entries.iter().map(canonical).collect();
            format!("[{}]", inner.join(","))
        }
        scalar => scalar.to_string(),
    }
}

/// The digest `docs/metadata.md` documents for `onetaskgraph.copied_destination_version`,
/// recomputed from what the binary reads back of the destination task `id` — its comments
/// counted when `with_comments`.
fn documented_digest(stores: &Stores, id: &str, with_comments: bool) -> String {
    let mut excluded = RECORD_KEYS.to_vec();
    excluded.push("onetaskgraph.depended_on_by");
    digest_without(stores, id, with_comments, &excluded)
}

/// The three keys recording a copy, which the digest never counts.
const RECORD_KEYS: [&str; 3] = [
    "onetaskgraph.copied_version",
    "onetaskgraph.copied_with_comments",
    "onetaskgraph.copied_destination_version",
];

/// The documented digest's formula over the destination task `id`, its `metadata` without
/// each of `excluded`.
fn digest_without(stores: &Stores, id: &str, with_comments: bool, excluded: &[&str]) -> String {
    let item = stores.json(&["task", "show", id, "--no-comments"])["items"][0]["item"].clone();
    let mut metadata = item["metadata"].as_object().expect("metadata").clone();
    for key in excluded {
        metadata.remove(*key);
    }
    // Each forward edge's far end as the destination itself reports it: its native id for an
    // item of the destination, and its qualified id for one of another source.
    let own = format!("{}:", id.split_once(':').expect("a qualified id").0);
    let mut edges: Vec<Value> = stores.json(&["task", "deps", id])["items"]
        .as_array()
        .expect("edges")
        .iter()
        .map(|edge| {
            let far = edge["to"]["id"].as_str().expect("a far end");
            json!([
                far.strip_prefix(own.as_str()).unwrap_or(far),
                edge["to"]["kind"],
                edge["kind"]
            ])
        })
        .collect();
    edges.sort_by_key(canonical);
    assert!(
        item.get("delivers").is_none(),
        "this journey's task delivers nothing, so `delivers` digests as empty: {item:#}"
    );
    let mut fields = json!({
        "title": item["title"],
        "content": item["content"],
        "status": item["status"],
        "priority": item["priority"],
        "labels": item["labels"],
        "project": item["project"],
        "metadata": metadata,
        "repositories": item["repositories"],
        "classification": item.get("classification").cloned().unwrap_or(json!("public")),
        "delivers": [],
        "edges": edges,
    });
    if with_comments {
        let mut entries: Vec<Value> = stores
            .comments(id)
            .iter()
            .map(|comment| match comment.get("origin") {
                Some(origin) => json!({"origin": [origin["task"], origin["id"]],
                                       "body": comment["body"]}),
                None => json!({"id": comment["id"], "body": comment["body"]}),
            })
            .collect();
        entries.sort_by_key(canonical);
        fields["comments"] = Value::Array(entries);
    }
    format!(
        "sha256:{:x}",
        <sha2::Sha256 as sha2::Digest>::digest(canonical(&fields).as_bytes())
    )
}

#[test]
fn the_recorded_destination_digest_is_the_documented_digest_of_the_destination_as_read() {
    for to in ["notes", "second"] {
        let stores = Stores::new();
        stores.board.add_comment("T-3", "said on the board");
        stores
            .board
            .set_metadata("T-3", "caller.note", json!({"nested": [1, true, null]}));
        let narrowed = |comments: bool| {
            let mut query = vec!["--to", to, "--source", "board", "--no-link"];
            if comments {
                query.push("--comments");
            }
            let (landed, _, _) = stores.copy(&query);
            landed
                .into_iter()
                .find(|(source, _)| source == "board:T-3")
                .map(|(_, outcome)| outcome["destination"].as_str().expect("an id").to_owned())
                .expect("T-3 landed")
        };

        // A `--comments` copy: the digest counts the comments as the destination holds them.
        let copy = narrowed(true);
        let recorded = stores.metadata(&copy)["onetaskgraph.copied_destination_version"].clone();
        assert_eq!(
            recorded,
            json!(documented_digest(&stores, &copy, true)),
            "{to}: the record is the documented digest of the destination as it reads"
        );
        // The digest is over the task as read, not over the source: a native comment at the
        // destination moves it, which is what makes the next copy read again.
        assert_ne!(
            json!(documented_digest(&stores, &copy, false)),
            recorded,
            "{to}: the comments are inside the digest"
        );

        // The dependents a private task records are kept in step after the copy, so they are
        // outside the digest: one held at the destination when the copy records it leaves the
        // record what the formula without them says.
        let depended_on_by = json!({"board:T-1": {"id": "board:T-1", "kind": "task"}});
        stores.plant(&copy, "onetaskgraph.depended_on_by", &depended_on_by);

        // A copy without `--comments` takes `copied_with_comments` away, and records a digest
        // that leaves the comments out.
        assert_eq!(narrowed(false), copy);
        let metadata = stores.metadata(&copy);
        assert_eq!(
            metadata["onetaskgraph.depended_on_by"], depended_on_by,
            "{to}: the copy keeps the store's own record"
        );
        assert_ne!(
            metadata["onetaskgraph.copied_destination_version"],
            json!(digest_without(&stores, &copy, false, &RECORD_KEYS)),
            "{to}: counting the record would have moved the digest"
        );
        assert!(
            metadata.get("onetaskgraph.copied_with_comments").is_none(),
            "{to}: {metadata:#}"
        );
        assert_eq!(
            metadata["onetaskgraph.copied_destination_version"],
            json!(documented_digest(&stores, &copy, false)),
            "{to}: without --comments the digest has no comments in it"
        );
    }
}

#[test]
fn two_edits_inside_one_second_are_never_served_from_a_skip() {
    let stores = Stores::new();
    let said = stores.board.add_comment("T-3", "said once");
    let query = [
        "--to",
        "notes",
        "--source",
        "board",
        "--comments",
        "--no-link",
    ];
    // A person editing T-3 while the copy reads it: every change in one second, and the board
    // read in that same second — so a change after the read moves neither of GitHub's times,
    // and the comment count stays where it was.
    stores.board.hold_clock();
    stores.board.read_as_it_changes(true);
    stores.board.edit_body("T-3", "the first word");
    let (first, _, _) = stores.copy(&query);
    let landed = |outcomes: &[(String, Value)]| {
        outcomes
            .iter()
            .find(|(source, _)| source == "board:T-3")
            .map(|(_, outcome)| outcome.clone())
            .expect("T-3 is copied")
    };
    let copy = landed(&first)["destination"]
        .as_str()
        .expect("T-3 landed")
        .to_owned();
    stores.board.edit_body("T-3", "the second word");
    stores.board.edit_comment(&said, "said twice");

    let (again, _, _) = stores.copy(&query);
    let outcome = landed(&again);
    assert!(
        !skipped(&outcome),
        "a task changed in the second it was read is read again: {outcome:#}"
    );
    let shown = stores.json(&["task", "show", &copy, "--no-comments"]);
    assert_eq!(
        shown["items"][0]["item"]["content"], "the second word",
        "the destination holds the latest body"
    );
    assert_eq!(
        stores
            .comments(&copy)
            .iter()
            .map(|comment| comment["body"].clone())
            .collect::<Vec<_>>(),
        vec![json!("said twice")],
        "and the latest comment"
    );

    // Once the board has settled the token is given again, and a copy after the one that
    // records it skips as before.
    stores.board.release_clock();
    stores.board.read_as_it_changes(false);
    let (recorded, _, _) = stores.copy(&query);
    assert!(!skipped(&landed(&recorded)), "{recorded:#?}");
    let (settled, served, _) = stores.copy(&query);
    assert!(skipped(&landed(&settled)), "{settled:#?}");
    assert!(
        served.naming("T-3").is_empty(),
        "a settled, unchanged task costs nothing beyond the listing: {:?}",
        served.names()
    );
}

#[test]
fn a_board_read_without_the_servers_clock_gives_no_token_and_is_never_skipped() {
    let stores = Stores::new();
    let versions = |stores: &Stores| -> Vec<Value> {
        stores.json(&["task", "list", "--source", "board"])["items"]
            .as_array()
            .expect("items")
            .iter()
            .map(|task| task["item"]["version"].clone())
            .collect()
    };
    assert!(
        versions(&stores).iter().all(Value::is_string),
        "a settled board read with its Date gives every issue a token"
    );
    stores.board.serve_without_date(true);
    assert!(
        versions(&stores).iter().all(Value::is_null),
        "without the server's clock nothing says the read came after the last change"
    );
    let query = [
        "--to",
        "notes",
        "--source",
        "board",
        "--comments",
        "--no-link",
    ];
    stores.copy(&query);
    let (again, _, _) = stores.copy(&query);
    assert!(
        again.iter().all(|(_, outcome)| !skipped(outcome)),
        "a task with no token is read and compared: {again:#?}"
    );
}

#[test]
fn a_destination_recorded_under_the_tokens_older_spelling_is_read_again_once_and_then_skipped() {
    let stores = Stores::new();
    let query = [
        "--to",
        "notes",
        "--source",
        "board",
        "--comments",
        "--no-link",
    ];
    let (landed, _, _) = stores.copy(&query);
    let note = landed
        .iter()
        .find(|(source, _)| source == "board:T-3")
        .map(|(_, outcome)| outcome["destination"].as_str().expect("an id").to_owned())
        .expect("T-3 landed");
    // What the builds before this one recorded: `gh2`, before the facets — 0.6.0's — and `gh1`,
    // before the relationship count and the repository too, each spelled from this token's own
    // parts. A copy skips only on the recorded token being the listed one, and this build orders
    // neither spelling, so such a record is never skipped on, and the read it costs is what
    // brings it up to date.
    let recorded = stores.metadata(&note)["onetaskgraph.copied_version"]
        .as_str()
        .expect("the copy records the source's token")
        .to_owned();
    let parts: Vec<&str> = recorded
        .strip_prefix(change_token::PREFIX)
        .expect("a token of this build")
        .splitn(6, '|')
        .collect();
    assert_eq!(parts.len(), 6, "a GitHub token's six parts: {recorded}");
    let gh2 = format!("gh2|{}|{}", parts[..4].join("|"), parts[5]);
    let gh1 = format!("gh1|{}", parts[..3].join("|"));
    let file = stores
        .sandbox
        .subdirectory("notes")
        .join("tasks")
        .join(format!(
            "{}.md",
            note.strip_prefix("notes:").expect("a folder id")
        ));
    for older in [gh2, gh1] {
        let text = std::fs::read_to_string(&file).expect("the copy's file");
        assert_eq!(text.matches(&recorded).count(), 1, "{text}");
        std::fs::write(&file, text.replace(&recorded, &older)).expect("the copy's file");

        let (again, served, _) = stores.copy(&query);
        for (source, outcome) in &again {
            assert_eq!(
                skipped(outcome),
                source != "board:T-3",
                "{source} is read again exactly when its record is a token this build never \
                 gives, here {older}: {outcome:#}"
            );
        }
        assert!(
            !served.naming("T-3").is_empty(),
            "the task recorded under {older} was read: {:?}",
            served.names()
        );
        assert_eq!(
            stores.metadata(&note)["onetaskgraph.copied_version"],
            json!(recorded),
            "the copy records the token this build gives"
        );
        let (settled, served, _) = stores.copy(&query);
        assert!(
            settled.iter().all(|(_, outcome)| skipped(outcome)),
            "{settled:#?}"
        );
        assert!(
            served.names().iter().all(|name| LISTING.contains(name)),
            "and the next copy skips it on its listing alone: {:?}",
            served.names()
        );
    }
}

/// The bound `docs/plugin-protocol.md` §4.13d states for a board's skip: a far end's own kind
/// flipping moves nothing of the task that depends on it, so a skipping recopy keeps the edge as
/// it was, and the next copy that reads the task carries it.
#[test]
fn a_far_end_turned_into_a_project_is_carried_by_the_next_copy_that_reads_the_task_after_it() {
    let stores = Stores::new();
    stores.board.block("T-4", &["T-3".to_owned()]);
    let query = ["--to", "notes", "--source", "board", "--no-link"];
    let (landed, _, _) = stores.copy(&query);
    let note = landed
        .iter()
        .find(|(source, _)| source == "board:T-4")
        .map(|(_, outcome)| outcome["destination"].as_str().expect("an id").to_owned())
        .expect("T-4 landed");
    let outcome = |copied: &[(String, Value)]| {
        copied
            .iter()
            .find(|(source, _)| source == "board:T-4")
            .map(|(_, outcome)| outcome.clone())
            .expect("T-4 is copied")
    };
    // What the folder's copy of T-4 says of the far end that was T-3.
    let far_end = |stores: &Stores| {
        stores.json(&["task", "deps", &note, "--direction", "depends-on"])["items"]
            .as_array()
            .expect("items")
            .iter()
            .map(|edge| edge["to"].clone())
            .find(|to| to["id"].as_str().is_some_and(|id| id.ends_with(":T-3")))
            .expect("T-4 depends on T-3")
    };
    assert_eq!(far_end(&stores)["kind"], "task");

    // T-3 gains a sub-issue, so it is a project; nothing of T-4 moves on GitHub.
    let held = stores.task("board:T-4")["version"].clone();
    stores.board.file_under("T-1", Some("T-3"));
    assert_eq!(
        stores.task("board:T-4")["version"],
        held,
        "T-4's own token is unmoved"
    );
    let source_kind =
        stores.json(&["task", "deps", "board:T-4", "--direction", "depends-on"])["items"]
            .as_array()
            .expect("items")
            .iter()
            .find(|edge| edge["to"]["id"] == "board:T-3")
            .map(|edge| edge["to"]["kind"].clone());
    assert_eq!(
        source_kind,
        Some(json!("project")),
        "the board reads T-3 as a project"
    );

    let (flipped, _, _) = stores.copy(&query);
    assert!(skipped(&outcome(&flipped)), "{flipped:#?}");
    assert_eq!(
        far_end(&stores)["kind"],
        "task",
        "the bound: a skipping recopy does not carry a far end's kind flip alone"
    );

    stores
        .board
        .edit_body("T-4", "edited, so the next copy reads it");
    let (read, _, _) = stores.copy(&query);
    assert!(!skipped(&outcome(&read)), "{read:#?}");
    assert_eq!(
        far_end(&stores),
        json!({"id": "board:T-3", "kind": "project"}),
        "the next copy that reads T-4 carries what its far end has become"
    );
}
