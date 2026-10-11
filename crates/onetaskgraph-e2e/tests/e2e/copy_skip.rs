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

use onetaskgraph_github_projects::graphql;
use serde_json::{Value, json};

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

    fn metadata(&self, id: &str) -> Value {
        self.json(&["task", "show", id, "--no-comments"])["items"][0]["item"]["metadata"].clone()
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
    let record = stores.metadata(&note);
    assert!(
        record["onetaskgraph.copied_version"]
            .as_str()
            .is_some_and(|token| token.starts_with("gh1|")),
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
    ];
    for (kind, change) in changes {
        change(&stores.board);
        let (after, served, _) = stores.copy(&query);
        for (source, outcome) in &after {
            assert_eq!(
                skipped(outcome),
                source != "board:T-3",
                "{kind}: {source} is read again exactly when it is the one changed: {outcome:#}"
            );
        }
        for untouched in ["T-1", "T-2", "T-4"] {
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
