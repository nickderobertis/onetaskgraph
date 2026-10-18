//! `task copy --status` out of the loopback board into a folder of Markdown, through the binary.
//!
//! The realistic caller is a comment snapshot: a folder configured with the board's own status
//! names, filled by a copy narrowed to the tasks that saw comment activity and are still at a
//! status somebody is deciding about. Each journey reads the board's request log, because what
//! the flag owes is as much the comments it does not read as the files it writes.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::Output;

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{GitHubBoardFields, document, github_projects_with_items};

/// One open task issue filed under no project, its card at `status`.
fn issue(id: &str, status: &str) -> Value {
    json!({"item":format!("ITEM-{id}"),"id":id,"type":"Issue","title":format!("Task {id}"),
           "body":format!("{id}, as filed"),"state":"OPEN","reason":null,"parent":null,
           "repo":"nickderobertis/onetaskgraph","status":status,"origin":"","labels":[]})
}

/// Every instant a comment on the board is stamped with is after this one.
const SINCE: &str = "2026-09-01T00:00:00Z";

/// A board whose commented tasks sit at `Proposal` (`S-1`, `S-2`), `Todo` (`S-3`), `In Progress`
/// (`S-4`) and `Done` closed as completed (`S-5`), with `S-6` closed as completed while its card
/// still reads `Todo`, and `S-7` closed for a reason that implies no category, so it reads
/// `unknown` — every one commented on — and a folder of Markdown called `snapshot` configured
/// with the board's own names.
struct Setup {
    sandbox: Sandbox,
    board: GitHubBoardFields,
    snapshot: PathBuf,
}

impl Setup {
    fn new() -> Self {
        let sandbox = Sandbox::new();
        let snapshot = sandbox.subdirectory("snapshot");
        std::fs::create_dir_all(snapshot.join("tasks")).expect("the snapshot's task folder");
        let items = vec![
            issue("S-1", "Proposal"),
            issue("S-2", "Proposal"),
            issue("S-3", "Todo"),
            issue("S-4", "In Progress"),
            issue("S-5", "Done"),
            issue("S-6", "Todo"),
            issue("S-7", "Todo"),
        ];
        let (mut config, board) = github_projects_with_items(&sandbox, items);
        board.with_status_option("OPT-in-progress", "In Progress");
        board.with_status_option("OPT-proposal", "Proposal");
        config["status_mapping"] =
            json!({"backlog":"Proposal","todo":"Todo","in-progress":"In Progress"});
        board.close("S-5", Some("COMPLETED"));
        board.close("S-6", Some("COMPLETED"));
        board.close("S-7", Some("REOPENED"));
        for id in ["S-1", "S-2", "S-3", "S-4", "S-5", "S-6", "S-7"] {
            board.add_comment(id, &format!("a word about {id}"));
        }
        sandbox.project_document(&document(&json!({
            "board": {"plugin":"github-projects","config":config},
            "snapshot": {"plugin":"local-md","config":{"root":snapshot,"status_mapping":{
                "Proposal":"backlog","Todo":"todo","In Progress":"in-progress","Done":"done",
                "Cancelled":"cancelled"}}}
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

    /// The machine output of a copy, asserting it exits zero.
    fn copy(&self, arguments: &[&str]) -> Value {
        let mut full = vec!["--json", "task", "copy"];
        full.extend(arguments);
        full.extend(["--to", "snapshot"]);
        let output = self.run(&full);
        assert_eq!(
            output.status.code(),
            Some(0),
            "`onetaskgraph {}`\n{}\n{}",
            full.join(" "),
            stdout(&output),
            stderr(&output)
        );
        serde_json::from_str(&stdout(&output)).expect("the copy emits JSON")
    }

    /// The board ids a copy's answer names, each with its action.
    fn answered(answer: &Value) -> Vec<(String, String)> {
        answer["items"]
            .as_array()
            .unwrap_or_else(|| panic!("an items list: {answer:#}"))
            .iter()
            .map(|item| {
                (
                    item["source"]
                        .as_str()
                        .and_then(|id| id.strip_prefix("board:"))
                        .expect("a board id")
                        .to_owned(),
                    item["action"].as_str().expect("an action").to_owned(),
                )
            })
            .collect()
    }

    /// The title of every task the snapshot holds.
    fn copied(&self) -> BTreeSet<String> {
        std::fs::read_dir(self.snapshot.join("tasks"))
            .expect("the snapshot's task folder")
            .map(|entry| {
                let text =
                    std::fs::read_to_string(entry.expect("an entry").path()).expect("a task file");
                text.lines()
                    .find_map(|line| line.strip_prefix("title: "))
                    .expect("a title")
                    .trim_matches('"')
                    .to_owned()
            })
            .collect()
    }

    /// Every issue whose comments the board was asked for.
    fn comments_read(&self) -> BTreeSet<String> {
        self.board
            .served()
            .into_iter()
            .filter(|(document, _)| document.contains("comments(first:$first"))
            .flat_map(|(_, variables)| {
                variables
                    .as_object()
                    .expect("variables")
                    .iter()
                    .filter(|(name, _)| *name == "id" || name.starts_with("id"))
                    .filter_map(|(_, value)| value.as_str().map(str::to_owned))
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// The query each board-scoped issue search was sent with.
    fn searches(&self) -> Vec<String> {
        self.board
            .served()
            .into_iter()
            .filter(|(document, _)| document.contains("search(query:$search"))
            .filter_map(|(_, variables)| variables["search"].as_str().map(str::to_owned))
            .collect()
    }
}

/// The titles the board's tasks `ids` carry.
fn titles(ids: &[&str]) -> BTreeSet<String> {
    ids.iter().map(|id| format!("Task {id}")).collect()
}

/// The set of ids `ids` spells.
fn set(ids: &[&str]) -> BTreeSet<String> {
    ids.iter().map(|id| (*id).to_owned()).collect()
}

#[test]
fn a_comment_snapshot_narrowed_to_backlog_copies_only_the_proposals_and_reads_no_other_comments() {
    let setup = Setup::new();
    let answer = setup.copy(&[
        "--source",
        "board",
        "--commented-since",
        SINCE,
        "--status",
        "backlog",
    ]);
    let answered: Vec<(String, String)> = Setup::answered(&answer);
    assert_eq!(
        answered,
        vec![
            ("S-1".to_owned(), "created".to_owned()),
            ("S-2".to_owned(), "created".to_owned())
        ],
        "only the Proposal items are named: {answer:#}"
    );
    assert_eq!(setup.copied(), titles(&["S-1", "S-2"]));
    assert_eq!(
        setup.comments_read(),
        set(&["S-1", "S-2"]),
        "no other item's comments are read"
    );
    let searches = setup.searches();
    assert!(!searches.is_empty(), "the board was searched");
    for search in &searches {
        assert!(
            search.split_whitespace().any(|word| word == "is:open"),
            "a search naming only open categories asks for open issues: {search}"
        );
    }
}

#[test]
fn a_snapshot_of_two_open_categories_copies_both_and_still_searches_open_issues_alone() {
    let setup = Setup::new();
    let answer = setup.copy(&[
        "--source",
        "board",
        "--commented-since",
        SINCE,
        "--status",
        "backlog",
        "--status",
        "todo",
    ]);
    let named: BTreeSet<String> = Setup::answered(&answer)
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    assert_eq!(named, set(&["S-1", "S-2", "S-3"]), "{answer:#}");
    assert_eq!(setup.copied(), titles(&["S-1", "S-2", "S-3"]));
    // The comment activity is confirmed from the comments of every task at those statuses —
    // the shared dataset's two todo tasks among them — and of no task at another.
    assert_eq!(
        setup.comments_read(),
        set(&["S-1", "S-2", "S-3", "T-1", "T-3"])
    );
    for search in setup.searches() {
        assert!(
            search.split_whitespace().any(|word| word == "is:open"),
            "{search}"
        );
    }
}

#[test]
fn a_snapshot_naming_done_searches_every_issue_and_copies_what_its_closure_says_is_done() {
    let setup = Setup::new();
    let answer = setup.copy(&[
        "--source",
        "board",
        "--commented-since",
        SINCE,
        "--status",
        "backlog",
        "--status",
        "done",
    ]);
    let named: BTreeSet<String> = Setup::answered(&answer)
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    assert_eq!(
        named,
        set(&["S-1", "S-2", "S-5", "S-6"]),
        "the Proposal items, the one at Done, and the one closed as completed at Todo: \
         {answer:#}"
    );
    assert_eq!(setup.copied(), titles(&["S-1", "S-2", "S-5", "S-6"]));
    // And the shared dataset's one task closed as completed, which reads done too.
    assert_eq!(
        setup.comments_read(),
        set(&["S-1", "S-2", "S-5", "S-6", "T-2"])
    );
    let searches = setup.searches();
    assert!(!searches.is_empty());
    for search in searches {
        assert!(
            !search.split_whitespace().any(|word| word == "is:open"),
            "a search naming done asks for closed issues too: {search}"
        );
    }
}

#[test]
fn ids_beside_status_copy_the_one_at_it_and_skip_the_others_reading_none_of_their_comments() {
    let setup = Setup::new();
    let answer = setup.copy(&["board:S-1", "board:S-3", "board:S-6", "--status", "backlog"]);
    let items = answer["items"].as_array().expect("items");
    assert_eq!(items.len(), 3, "{answer:#}");
    assert_eq!(items[0]["source"], "board:S-1");
    assert_eq!(items[0]["action"], "created");
    assert_eq!(
        items[1],
        json!({"source":"board:S-3","action":"skipped",
               "reason":"status todo is not one of backlog"})
    );
    assert_eq!(
        items[2],
        json!({"source":"board:S-6","action":"skipped",
               "reason":"status done is not one of backlog"})
    );
    assert_eq!(setup.copied(), titles(&["S-1"]));
    assert!(
        setup.comments_read().is_subset(&set(&["S-1"])),
        "neither skipped task's comments are read: {:?}",
        setup.comments_read()
    );
    let human = setup.run(&[
        "task",
        "copy",
        "board:S-3",
        "--status",
        "backlog",
        "--to",
        "snapshot",
    ]);
    assert_eq!(human.status.code(), Some(0), "{}", stderr(&human));
    assert!(
        stdout(&human).contains("skipped")
            && stdout(&human).contains("status todo is not one of backlog"),
        "{}",
        stdout(&human)
    );
}

#[test]
fn a_listing_of_unknown_finds_the_issue_closed_for_no_category_and_searches_every_issue() {
    let setup = Setup::new();
    let output = setup.run(&[
        "--json",
        "task",
        "list",
        "--source",
        "board",
        "--commented-since",
        SINCE,
        "--status",
        "unknown",
    ]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let listed: Value = serde_json::from_str(&stdout(&output)).expect("JSON");
    let ids: BTreeSet<String> = listed["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|item| item["id"].as_str().expect("an id").to_owned())
        .collect();
    assert!(
        ids.contains("board:S-7"),
        "a closed issue reads unknown when its reason implies no category: {listed:#}"
    );
    let searches = setup.searches();
    assert!(!searches.is_empty(), "the board was searched");
    for search in searches {
        assert!(
            !search.split_whitespace().any(|word| word == "is:open"),
            "a search naming unknown asks for closed issues too: {search}"
        );
    }
}
