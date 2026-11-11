//! The batched calls — `task find`, `task comment add-many`, `task update-many`, and `task copy`
//! held per item by `--if-version` — through the binary against the loopback board: each item
//! answered on its own, and each call sent in as few requests as GitHub allows, which the board's
//! record of every request it served says.

use std::collections::BTreeSet;
use std::path::PathBuf;

use onetaskgraph_github_projects::graphql;
use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{AliasAnswer, GitHubBoardFields, github_projects_with_items};

/// One open issue on the board, at the `Status` option `status`.
fn issue(id: &str, title: &str, body: &str, status: &str) -> Value {
    json!({"item":format!("ITEM-{id}"),"id":id,"type":"Issue","title":title,"body":body,
        "state":"OPEN","reason":null,"parent":null,"repo":"nickderobertis/onetaskgraph",
        "status":status,"origin":"","labels":[]})
}

struct Board {
    sandbox: Sandbox,
    board: GitHubBoardFields,
    notes: PathBuf,
    /// The `board` source's configuration, for a journey that builds the plugin itself.
    config: Value,
}

/// What one invocation answered: its exit code, its JSON and its standard error.
struct Answered {
    code: Option<i32>,
    json: Value,
    stderr: String,
}

impl Board {
    fn of(items: Vec<Value>) -> Self {
        let sandbox = Sandbox::new();
        let (config, board) = github_projects_with_items(&sandbox, items);
        let notes = sandbox.subdirectory("notes");
        std::fs::create_dir_all(notes.join("tasks")).expect("the folder");
        sandbox.project_document(
            &serde_json::to_string_pretty(&json!({
                "sources": {
                    "board": {"plugin": "github-projects", "config": config.clone()},
                    "notes": {"plugin": "local-md", "config": {"root": notes,
                        "status_mapping": {"Todo": "todo", "Doing": "in-progress"}}},
                },
            }))
            .expect("a document"),
        );
        Self {
            sandbox,
            board,
            notes,
            config,
        }
    }

    fn run(&self, arguments: &[&str], stdin: &str) -> Answered {
        let output = self
            .sandbox
            .command()
            .args(arguments)
            .write_stdin(stdin)
            .assert()
            .get_output()
            .clone();
        Answered {
            code: output.status.code(),
            json: serde_json::from_str(&stdout(&output)).unwrap_or(Value::Null),
            stderr: stderr(&output),
        }
    }

    /// [`Self::run`], with every request the board served for it.
    fn measured(&self, arguments: &[&str], stdin: &str) -> (Answered, Vec<(String, Value)>) {
        let mark = self.board.served_count();
        let answered = self.run(arguments, stdin);
        (answered, self.board.served_since(mark))
    }

    /// Each task's version, as `task show-many` reads it now.
    fn versions(&self, ids: &[&str]) -> Vec<String> {
        let qualified: Vec<String> = ids.iter().map(|id| format!("board:{id}")).collect();
        let mut arguments = vec!["task", "show-many", "--no-comments", "--json"];
        arguments.extend(qualified.iter().map(String::as_str));
        let shown = self.run(&arguments, "");
        assert_eq!(shown.code, Some(0), "{}", shown.stderr);
        shown.json["details"]
            .as_array()
            .unwrap_or_else(|| panic!("the details: {:#}", shown.json))
            .iter()
            .map(|detail| {
                detail["items"][0]["item"]["version"]
                    .as_str()
                    .expect("a version")
                    .to_owned()
            })
            .collect()
    }

    /// Write the folder's task `id`.
    fn author(&self, id: &str, body: &str) {
        std::fs::write(
            self.notes.join(format!("tasks/{id}.md")),
            format!("---\ntitle: Ticket {id}\nstatus: Todo\n---\n{body}\n"),
        )
        .expect("a task");
    }

    /// Copy the folder's `ids` onto the board, each recording its link, and answer the board
    /// id each landed as.
    fn copied(&self, ids: &[&str]) -> Vec<String> {
        let qualified: Vec<String> = ids.iter().map(|id| format!("notes:{id}")).collect();
        let mut arguments = vec!["task", "copy"];
        arguments.extend(qualified.iter().map(String::as_str));
        arguments.extend(["--to", "board", "--json"]);
        let copied = self.run(&arguments, "");
        assert_eq!(copied.code, Some(0), "{}", copied.stderr);
        let landed = copied.json["items"]
            .as_array()
            .expect("the items")
            .iter()
            .map(|item| {
                item["destination"]
                    .as_str()
                    .expect("a destination")
                    .trim_start_matches("board:")
                    .to_owned()
            })
            .collect();
        // Every change before a read has settled, so each read reports a version.
        self.board.pass_time(120);
        landed
    }
}

/// The documents of `served`, in order.
fn documents(served: &[(String, Value)]) -> Vec<&str> {
    served
        .iter()
        .map(|(document, _)| document.as_str())
        .collect()
}

/// Whether `document` is a mutation.
fn mutation(document: &str) -> bool {
    document.trim_start().starts_with("mutation")
}

/// The board ids every answer of a `task find` holds under `key`, each answered once.
fn found(answer: &Value, key: &str) -> BTreeSet<String> {
    let answers = answer["answers"].as_array().expect("the answers");
    let one = answers
        .iter()
        .find(|answer| answer["key"] == key)
        .unwrap_or_else(|| panic!("no answer under {key}: {answer:#}"));
    let ids: Vec<String> = one["items"]
        .as_array()
        .expect("the items")
        .iter()
        .map(|item| {
            item["id"]
                .as_str()
                .expect("an id")
                .trim_start_matches("board:")
                .to_owned()
        })
        .collect();
    let unique: BTreeSet<String> = ids.iter().cloned().collect();
    assert_eq!(
        unique.len(),
        ids.len(),
        "{key} answered a task twice: {ids:?}"
    );
    unique
}

fn set(ids: impl IntoIterator<Item = String>) -> BTreeSet<String> {
    ids.into_iter().collect()
}

/// A board holding, beside the shared dataset, more than one page of each of two searches:
/// 25 release checklists, 7 deploy steps — 4 of them in progress — two follow-ups of proposal
/// p42, five tickets copied from `notes:o1` — more than a first page of its lookup — and one
/// copied from `notes:o10`.
fn searched_board() -> Board {
    let mut items: Vec<Value> = (0..25)
        .map(|n| {
            issue(
                &format!("R-{n}"),
                &format!("Release checklist {n}"),
                &format!("release step {n}"),
                "Todo",
            )
        })
        .collect();
    items.extend((0..7).map(|n| {
        issue(
            &format!("D-{n}"),
            &format!("Deploy step {n}"),
            &format!("deploy {n}"),
            if n % 2 == 0 { "Doing" } else { "Todo" },
        )
    }));
    items.extend((0..2).map(|n| issue(&format!("F-{n}"), "A follow-up", "followed up", "Todo")));
    for (id, origin) in [
        ("O-0", "notes:o1"),
        ("O-1", "notes:o1"),
        ("O-2", "notes:o1"),
        ("O-3", "notes:o1"),
        ("O-4", "notes:o1"),
        ("O-5", "notes:o10"),
    ] {
        let mut copied = issue(id, "A copied ticket", "copied", "Todo");
        copied["origin"] = json!(origin);
        items.push(copied);
    }
    let board = Board::of(items);
    for n in 0..2 {
        board.board.set_metadata(
            &format!("F-{n}"),
            "orchestrator.follow-up",
            json!({"proposal": "p42"}),
        );
    }
    board.board.pass_time(120);
    board
}

/// The five searches of the find journeys.
fn five_queries() -> String {
    json!({"schema_version": 1, "queries": [
        {"key": "releases", "text": "release"},
        {"key": "follow-ups", "metadata": {"orchestrator.follow-up/proposal": "p42"}},
        {"key": "copies", "origin": "notes:o1"},
        {"key": "deploying", "text": "deploy", "status": ["in-progress"]},
        {"key": "nothing", "text": "zebra"},
    ]})
    .to_string()
}

#[test]
fn a_find_answers_each_query_under_its_key_across_pages_in_one_request_a_round() {
    let board = searched_board();
    let (answered, served) = board.measured(
        &["task", "find", "board", "--queries", "-", "--json"],
        &five_queries(),
    );
    assert_eq!(answered.code, Some(0), "{}", answered.stderr);
    let answer = &answered.json;
    assert_eq!(
        found(answer, "releases"),
        set((0..25).map(|n| format!("R-{n}"))),
        "{answer:#}"
    );
    assert_eq!(
        found(answer, "follow-ups"),
        set((0..2).map(|n| format!("F-{n}")))
    );
    assert_eq!(
        found(answer, "copies"),
        set((0..5).map(|n| format!("O-{n}"))),
        "every carrier of the origin, and not the one of another it begins"
    );
    assert_eq!(
        found(answer, "deploying"),
        set([0, 2, 4, 6].map(|n| format!("D-{n}")))
    );
    assert_eq!(found(answer, "nothing"), BTreeSet::new());
    for each in answer["answers"].as_array().expect("answers") {
        assert_eq!(each["complete"], true, "{each}");
        assert_eq!(each["reason"], Value::Null, "{each}");
    }
    // One request a round: the five together, then the three with more than a first page
    // together, then the last page of the one with more still — one search request per
    // result page, never one per query.
    assert_eq!(
        documents(&served),
        [
            graphql::SEARCH_BATCH,
            graphql::SEARCH_BATCH,
            graphql::SEARCH_ISSUES
        ],
        "{served:#?}"
    );
    let first = &served[0].1;
    let asked: Vec<&str> = (0..onetaskgraph_github_projects::SEARCH_BATCH)
        .filter(|slot| first[format!("ask{slot}")] == true)
        .map(|slot| first[format!("search{slot}")].as_str().expect("a search"))
        .collect();
    assert_eq!(
        asked.len(),
        5,
        "every query's search in the first: {first:#}"
    );
    assert_eq!(first["origin0"], true, "{first:#}");
    assert_eq!(first["filter0"], "onetaskgraph.origin:\"notes:o1\"");
    let second = &served[1].1;
    let continuing = (0..onetaskgraph_github_projects::SEARCH_BATCH)
        .filter(|slot| second[format!("ask{slot}")] == true)
        .count();
    assert_eq!(continuing, 2, "{second:#}");
    assert_eq!(
        second["origin0"], true,
        "the origin's board half goes on: {second:#}"
    );
    // Every page of that round resumes, and together they ask one page of rows rather than a
    // page each: a round of twenty resumed searches would otherwise cost 22 points.
    let rows = |slots: usize, asked: &str, first: &str| -> u64 {
        (0..slots)
            .filter(|slot| second[format!("{asked}{slot}")] == true)
            .map(|slot| {
                second[format!("{first}{slot}")]
                    .as_u64()
                    .expect("a page size")
            })
            .sum()
    };
    let shared = rows(onetaskgraph_github_projects::SEARCH_BATCH, "ask", "first")
        + rows(
            onetaskgraph_github_projects::SEARCH_BATCH_ORIGINS,
            "origin",
            "itemsFirst",
        );
    assert!(
        shared <= u64::from(onetaskgraph_github_projects::SEARCH_PAGE_SIZE),
        "the resumed pages of one round share one page of rows, and asked {shared}: {second:#}"
    );
}

#[test]
fn a_find_refuses_an_unnarrowed_query_naming_its_key_and_reads_nothing() {
    let board = searched_board();
    let request = json!({"schema_version": 1, "queries": [
        {"key": "releases", "text": "release"},
        {"key": "everything in progress", "status": ["in-progress"]},
    ]})
    .to_string();
    let (answered, served) = board.measured(
        &["task", "find", "board", "--queries", "-", "--json"],
        &request,
    );
    assert_ne!(answered.code, Some(0), "{:#}", answered.json);
    assert!(
        answered.stderr.contains("everything in progress"),
        "{}",
        answered.stderr
    );
    assert!(served.is_empty(), "{served:#?}");
}

#[test]
fn a_refused_page_of_one_query_cuts_that_answer_short_and_completes_the_others() {
    let board = searched_board();
    // The release search's first page is answered and its second refused at its own alias.
    board.board.refuse_search_page("\"release\"", 1);
    let (answered, served) = board.measured(
        &["task", "find", "board", "--queries", "-", "--json"],
        &five_queries(),
    );
    assert_eq!(answered.code, Some(4), "{}", answered.stderr);
    let answers = answered.json["answers"].as_array().expect("answers");
    let releases = answers
        .iter()
        .find(|answer| answer["key"] == "releases")
        .expect("the releases");
    assert_eq!(releases["complete"], false, "{releases:#}");
    let reason = releases["reason"].as_str().expect("a reason");
    assert!(reason.contains("page 2"), "{reason}");
    assert!(reason.contains("releases"), "{reason}");
    assert_eq!(
        found(&answered.json, "releases").len(),
        onetaskgraph_github_projects::SEARCH_BATCH_FIRST_PAGE as usize,
        "what its first page answered"
    );
    for key in ["follow-ups", "copies", "deploying", "nothing"] {
        let answer = answers
            .iter()
            .find(|answer| answer["key"] == key)
            .expect("an answer");
        assert_eq!(answer["complete"], true, "{answer:#}");
    }
    assert_eq!(
        found(&answered.json, "deploying"),
        set([0, 2, 4, 6].map(|n| format!("D-{n}")))
    );
    assert_eq!(
        documents(&served),
        [graphql::SEARCH_BATCH, graphql::SEARCH_BATCH],
        "the refused search is walked no further: {served:#?}"
    );
}

#[test]
fn a_copy_holding_two_items_writes_the_one_at_its_version_and_refuses_the_moved_one() {
    let board = Board::of(Vec::new());
    for id in ["c0", "c1", "c2", "c3"] {
        board.author(id, &format!("first {id}"));
    }
    let landed = board.copied(&["c0", "c1", "c2", "c3"]);
    let ids: Vec<&str> = landed.iter().map(String::as_str).collect();
    let versions = board.versions(&ids);
    for id in ["c0", "c1", "c2", "c3"] {
        let path = board.notes.join(format!("tasks/{id}.md"));
        let text = std::fs::read_to_string(&path).expect("the task");
        std::fs::write(
            &path,
            text.replace(&format!("first {id}"), &format!("second {id}")),
        )
        .expect("an edit");
    }
    // Another writer moves the second ticket after its version was read.
    board.board.edit_body(ids[1], "a person's own words");
    board.board.pass_time(120);
    let moved_to = board.versions(&[ids[1]]).remove(0);
    let held = [
        format!("notes:c0={}", versions[0]),
        format!("notes:c1={}", versions[1]),
    ];
    let copied = board.run(
        &[
            "task",
            "copy",
            "notes:c0",
            "notes:c1",
            "notes:c2",
            "notes:c3",
            "--to",
            "board",
            "--if-version",
            &held[0],
            "--if-version",
            &held[1],
            "--json",
        ],
        "",
    );
    assert_eq!(copied.code, Some(4), "{}", copied.stderr);
    let items = copied.json["items"].as_array().expect("items");
    for (at, item) in items.iter().enumerate() {
        if at == 1 {
            assert_eq!(item["action"], "failed", "{item:#}");
            assert_eq!(item["error"]["kind"], "version-moved", "{item:#}");
            assert_eq!(item["held"]["version"], json!(moved_to), "{item:#}");
        } else {
            assert_eq!(item["action"], "updated", "{item:#}");
        }
    }
    for (at, id) in ids.iter().enumerate() {
        let body = board.board.body(id);
        let body = body.as_str().expect("a body");
        if at == 1 {
            assert_eq!(
                body, "a person's own words",
                "left as the other writer left it"
            );
        } else {
            assert!(body.starts_with(&format!("second c{at}")), "{body}");
        }
    }
}

#[test]
fn comments_added_together_hold_their_versions_in_one_read_and_one_mutation() {
    let board = Board::of(
        (0..4)
            .map(|n| issue(&format!("A-{n}"), "Asked", "a question", "Todo"))
            .collect(),
    );
    board.board.pass_time(120);
    let versions = board.versions(&["A-0", "A-1", "A-2", "A-3"]);
    // Another writer comments on the second after its version was read.
    board
        .board
        .add_comment_by("A-1", "a person got there first", "ada");
    board.board.pass_time(120);
    let moved_to = board.versions(&["A-1"]).remove(0);
    let batch = json!({"schema_version": 1, "comments": [
        {"item": "board:A-0", "body": "answer 0", "if_version": versions[0]},
        {"item": "board:A-1", "body": "answer 1", "if_version": versions[1]},
        {"item": "board:A-2", "body": "answer 2"},
        {"item": "board:A-3", "body": "answer 3"},
    ]});
    let (answered, served) = board.measured(
        &[
            "task", "comment", "add-many", "board", "--input", "-", "--json",
        ],
        &batch.to_string(),
    );
    assert_eq!(answered.code, Some(4), "{}", answered.stderr);
    let results = answered.json["results"].as_array().expect("results");
    for (at, result) in results.iter().enumerate() {
        if at == 1 {
            assert_eq!(result["outcome"], "version-moved", "{result:#}");
            assert_eq!(result["current_version"], json!(moved_to), "{result:#}");
        } else {
            assert_eq!(result["outcome"], "written", "{result:#}");
            assert!(result["comment"]["id"].is_string(), "{result:#}");
        }
    }
    for n in [0, 2, 3] {
        let bodies: Vec<Value> = board
            .board
            .comments_on(&format!("A-{n}"))
            .iter()
            .map(|comment| comment["body"].clone())
            .collect();
        assert_eq!(bodies, [json!(format!("answer {n}"))]);
    }
    let bodies: Vec<Value> = board
        .board
        .comments_on("A-1")
        .iter()
        .map(|comment| comment["body"].clone())
        .collect();
    assert_eq!(bodies, [json!("a person got there first")]);
    // One read deciding every hold, and the four comments in one mutation request.
    assert_eq!(
        documents(&served),
        [graphql::ISSUE_DETAILS, graphql::ADD_COMMENTS],
        "{served:#?}"
    );
}

/// The four tickets of the update journeys: `U-2` holds a `Low` priority, the rest none.
fn ticket_board() -> Board {
    let board = Board::of(
        (0..4)
            .map(|n| {
                issue(
                    &format!("U-{n}"),
                    "A ticket",
                    &format!("ticket {n}"),
                    "Todo",
                )
            })
            .collect(),
    );
    board.board.assign_priority("U-2", Some("Low"));
    board.board.pass_time(120);
    board
}

/// One held update of `id` setting content, a metadata key, a status and, while it holds none,
/// a priority.
fn triage(id: &str, version: Option<&String>) -> Value {
    json!({"item": format!("board:{id}"), "if_version": version,
           "body": format!("{id} triaged"), "status": "in-progress",
           "priority": "high", "when_priority": ["none"],
           "metadata": {"orchestrator.triage": {"by": "orchestrator"}}})
}

#[test]
fn updates_written_together_hold_their_versions_and_guards_and_write_each_whole() {
    let board = ticket_board();
    let versions = board.versions(&["U-0", "U-1", "U-2", "U-3"]);
    // Another writer edits the second after its version was read.
    board.board.edit_body("U-1", "a person's own words");
    board.board.pass_time(120);
    let moved_to = board.versions(&["U-1"]).remove(0);
    let batch = json!({"schema_version": 1, "updates": [
        triage("U-0", Some(&versions[0])),
        triage("U-1", Some(&versions[1])),
        triage("U-2", Some(&versions[2])),
        triage("U-3", None),
    ]});
    let (answered, served) = board.measured(
        &["task", "update-many", "board", "--input", "-", "--json"],
        &batch.to_string(),
    );
    assert_eq!(answered.code, Some(4), "{}", answered.stderr);
    let outcomes: Vec<&str> = answered.json["results"]
        .as_array()
        .expect("results")
        .iter()
        .map(|result| result["outcome"].as_str().expect("an outcome"))
        .collect();
    assert_eq!(
        outcomes,
        ["written", "version-moved", "held", "written"],
        "{:#}",
        answered.json
    );
    assert_eq!(
        answered.json["results"][1]["current_version"],
        json!(moved_to)
    );
    // Every field of a written update changed; nothing of a refused one did.
    for id in ["U-0", "U-3"] {
        let body = board.board.body(id);
        let body = body.as_str().expect("a body");
        assert!(body.starts_with(&format!("{id} triaged")), "{body}");
        assert!(body.contains("orchestrator.triage"), "{body}");
        assert_eq!(board.board.status(id).as_deref(), Some("Doing"));
        assert_eq!(board.board.priority(id).as_deref(), Some("High"));
    }
    assert_eq!(board.board.body("U-1"), json!("a person's own words"));
    assert_eq!(board.board.status("U-1").as_deref(), Some("Todo"));
    assert_eq!(board.board.priority("U-1"), None);
    assert_eq!(board.board.body("U-2"), json!("ticket 2"));
    assert_eq!(board.board.status("U-2").as_deref(), Some("Todo"));
    assert_eq!(board.board.priority("U-2").as_deref(), Some("Low"));
    // Read once, together; then every field write in one request and every issue update in
    // one more — never a request per ticket. The issue updates' own answer shows each ticket
    // whole, so the version each holds after its write needs no read after it.
    let mutations: Vec<&str> = documents(&served)
        .into_iter()
        .filter(|document| mutation(document))
        .collect();
    assert_eq!(
        mutations,
        [graphql::WRITE_TASKS, graphql::WRITE_TASKS],
        "{served:#?}"
    );
    assert_eq!(
        documents(&served)
            .into_iter()
            .filter(|document| *document == graphql::ISSUE_DETAILS)
            .count(),
        1,
        "{served:#?}"
    );
    assert!(
        !documents(&served).contains(&graphql::ISSUE),
        "no ticket read on its own: {served:#?}"
    );
}

#[test]
fn a_guarded_update_writes_content_metadata_and_priority_only_while_the_guard_holds() {
    let board = ticket_board();
    let content = board.notes.join("content.md");
    std::fs::write(&content, "rewritten").expect("the content");
    let content = content.to_str().expect("a path");
    let update = |id: &str| {
        board.run(
            &[
                "task",
                "update",
                &format!("board:{id}"),
                "--body-file",
                content,
                "--metadata",
                "orchestrator.triage={\"by\":\"orchestrator\"}",
                "--priority",
                "high",
                "--when-priority",
                "none",
                "--json",
            ],
            "",
        )
    };
    let written = update("U-0");
    assert_eq!(written.code, Some(0), "{}", written.stderr);
    let body = board.board.body("U-0");
    let body = body.as_str().expect("a body");
    assert!(body.starts_with("rewritten"), "{body}");
    assert!(body.contains("orchestrator.triage"), "{body}");
    assert_eq!(board.board.priority("U-0").as_deref(), Some("High"));
    let held = update("U-2");
    assert_eq!(held.code, Some(0), "{}", held.stderr);
    assert_eq!(held.json["guard"]["outcome"], "held", "{:#}", held.json);
    assert_eq!(held.json["guard"]["priority_condition"]["held"], "low");
    assert_eq!(held.json["written"], json!([]));
    assert_eq!(board.board.body("U-2"), json!("ticket 2"));
    assert_eq!(board.board.priority("U-2").as_deref(), Some("Low"));
}

#[test]
fn a_batch_answers_a_missing_task_and_a_refused_mutation_each_on_its_own() {
    let board = Board::of(
        (0..4)
            .map(|n| {
                issue(
                    &format!("M-{n}"),
                    "A ticket",
                    &format!("ticket {n}"),
                    "Todo",
                )
            })
            .collect(),
    );
    board.board.pass_time(120);
    let comments = json!({"schema_version": 1, "comments": [
        {"item": "board:M-0", "body": "noted 0"},
        {"item": "board:NOPE-1", "body": "noted nowhere"},
        {"item": "board:M-1", "body": "noted 1"},
        {"item": "board:M-3", "body": "noted 3"},
        {"item": "board:M-2", "body": "noted 2"},
    ]});
    board.board.reject_mutation_of("M-3");
    let added = board.run(
        &[
            "task", "comment", "add-many", "board", "--input", "-", "--json",
        ],
        &comments.to_string(),
    );
    assert_eq!(added.code, Some(4), "{}", added.stderr);
    let results = added.json["results"].as_array().expect("results");
    for (at, named) in [(1, "NOPE-1"), (3, "M-3")] {
        assert_eq!(results[at]["outcome"], "error", "{:#}", results[at]);
        let reason = results[at]["reason"].as_str().expect("a reason");
        assert!(reason.contains(named), "{reason}");
    }
    for at in [0, 2, 4] {
        assert_eq!(results[at]["outcome"], "written", "{:#}", results[at]);
    }
    assert!(board.board.comments_on("M-3").is_empty());
    for n in 0..3 {
        assert_eq!(board.board.comments_on(&format!("M-{n}")).len(), 1);
    }

    board.board.reject_mutation_of("M-3");
    let updates = json!({"schema_version": 1, "updates": [
        {"item": "board:M-0", "body": "M-0 settled", "status": "in-progress"},
        {"item": "board:NOPE-1", "body": "nowhere", "status": "in-progress"},
        {"item": "board:M-1", "body": "M-1 settled", "status": "in-progress"},
        {"item": "board:M-3", "body": "M-3 settled", "status": "in-progress"},
        {"item": "board:M-2", "body": "M-2 settled", "status": "in-progress"},
    ]});
    let updated = board.run(
        &["task", "update-many", "board", "--input", "-", "--json"],
        &updates.to_string(),
    );
    assert_eq!(updated.code, Some(4), "{}", updated.stderr);
    let results = updated.json["results"].as_array().expect("results");
    for (at, named) in [(1, "NOPE-1"), (3, "M-3")] {
        assert_eq!(results[at]["outcome"], "error", "{:#}", results[at]);
        let reason = results[at]["reason"].as_str().expect("a reason");
        assert!(reason.contains(named), "{reason}");
    }
    for (at, id) in [(0, "M-0"), (2, "M-1"), (4, "M-2")] {
        assert_eq!(results[at]["outcome"], "written", "{:#}", results[at]);
        let body = board.board.body(id);
        assert!(
            body.as_str()
                .is_some_and(|body| body.starts_with(&format!("{id} settled"))),
            "{body}"
        );
        assert_eq!(board.board.status(id).as_deref(), Some("Doing"));
    }
    // The refused ticket's field write was refused, so nothing of its update was sent.
    assert_eq!(board.board.body("M-3"), json!("ticket 3"));
    assert_eq!(board.board.status("M-3").as_deref(), Some("Todo"));
}

#[test]
fn an_unchanged_recopy_of_five_decides_every_item_from_one_batched_read() {
    let board = Board::of(Vec::new());
    let ids = ["r0", "r1", "r2", "r3", "r4"];
    for id in ids {
        board.author(id, &format!("ticket {id}"));
    }
    board.copied(&ids);
    let qualified: Vec<String> = ids.iter().map(|id| format!("notes:{id}")).collect();
    let mut arguments = vec!["task", "copy"];
    arguments.extend(qualified.iter().map(String::as_str));
    arguments.extend(["--to", "board", "--json"]);
    let (copied, served) = board.measured(&arguments, "");
    assert_eq!(copied.code, Some(0), "{}", copied.stderr);
    assert!(
        copied.json["items"]
            .as_array()
            .expect("items")
            .iter()
            .all(|item| item["action"] == "unchanged"),
        "{:#}",
        copied.json
    );
    assert_eq!(
        documents(&served),
        [graphql::ISSUE_COPY_DETAILS],
        "{served:#?}"
    );
}

#[test]
fn two_updates_of_one_ticket_are_each_written_on_what_the_one_before_left() {
    let board = ticket_board();
    let batch = json!({"schema_version": 1, "updates": [
        {"item": "board:U-0", "body": "U-0 first"},
        {"item": "board:U-0", "status": "in-progress",
         "metadata": {"orchestrator.seen": true}},
        triage("U-1", None),
    ]});
    let answered = board.run(
        &["task", "update-many", "board", "--input", "-", "--json"],
        &batch.to_string(),
    );
    assert_eq!(answered.code, Some(0), "{}", answered.stderr);
    let results = answered.json["results"].as_array().expect("results");
    for result in results {
        assert_eq!(result["outcome"], "written", "{result:#}");
    }
    // The second update of U-0 kept the body the first wrote, and added its own fields.
    let body = board.board.body("U-0");
    let body = body.as_str().expect("a body");
    assert!(body.starts_with("U-0 first"), "{body}");
    assert!(body.contains("orchestrator.seen"), "{body}");
    assert_eq!(board.board.status("U-0").as_deref(), Some("Doing"));
    // Each written update answers the version its ticket holds after the batch's writes — both
    // of U-0's the one its later update left — and once the board has settled, it is the one a
    // read gives.
    let reported: Vec<&str> = results
        .iter()
        .map(|result| {
            result["version"]
                .as_str()
                .expect("a version after the write")
        })
        .collect();
    assert_eq!(reported[0], reported[1], "{results:#?}");
    board.board.pass_time(120);
    assert_eq!(board.versions(&["U-0", "U-1"]), [reported[1], reported[2]]);
    let body = board.board.body("U-1");
    assert!(
        body.as_str()
            .is_some_and(|body| body.starts_with("U-1 triaged")),
        "{body}"
    );
}

/// `task update` of `id` retitled, held by `--if-version` to `version`.
fn held_retitle(board: &Board, id: &str, version: &str) -> Answered {
    board.run(
        &[
            "task",
            "update",
            &format!("board:{id}"),
            "--title",
            "Retitled under a hold",
            "--if-version",
            version,
            "--json",
        ],
        "",
    )
}

#[test]
fn a_written_update_answers_the_version_it_left_and_a_write_held_to_it_goes_ahead() {
    let board = ticket_board();
    let before = board.versions(&["U-0", "U-1", "U-2", "U-3"]);
    // Two updates written together, each held to the version read before it.
    let batch = json!({"schema_version": 1, "updates": [
        triage("U-0", Some(&before[0])),
        triage("U-3", Some(&before[3])),
    ]});
    let batched = board.run(
        &["task", "update-many", "board", "--input", "-", "--json"],
        &batch.to_string(),
    );
    assert_eq!(batched.code, Some(0), "{}", batched.stderr);
    let results = batched.json["results"].as_array().expect("results");
    let reported: Vec<String> = results
        .iter()
        .map(|result| {
            assert_eq!(result["outcome"], "written", "{result:#}");
            result["version"]
                .as_str()
                .unwrap_or_else(|| panic!("a version after the write: {result:#}"))
                .to_owned()
        })
        .collect();
    assert_ne!(
        reported[0], before[0],
        "never the version read before the write"
    );
    assert_ne!(
        reported[1], before[3],
        "never the version read before the write"
    );
    // One update whose last write is its issue's, and one writing two board fields alone.
    let single = |id: &str, arguments: &[&str]| {
        let target = format!("board:{id}");
        let mut command = vec!["task", "update", target.as_str()];
        command.extend(arguments);
        command.push("--json");
        let written = board.run(&command, "");
        assert_eq!(written.code, Some(0), "{}", written.stderr);
        let version = written.json["result"]["version"]
            .as_str()
            .unwrap_or_else(|| panic!("a version after the write: {:#}", written.json))
            .to_owned();
        assert_eq!(
            written.json["task"]["version"],
            json!(version),
            "the task answered carries the version the write left: {:#}",
            written.json
        );
        version
    };
    let retitled = single("U-1", &["--title", "A retitled ticket"]);
    assert_ne!(
        retitled, before[1],
        "never the version read before the write"
    );
    let moved = single("U-2", &["--status", "in-progress", "--priority", "high"]);
    assert_ne!(moved, before[2], "never the version read before the write");

    // Once the board has settled, each is the version a read gives.
    board.board.pass_time(120);
    assert_eq!(
        board.versions(&["U-0", "U-3", "U-1", "U-2"]),
        [reported[0].as_str(), &reported[1], &retitled, &moved]
    );
    // A write held to a reported version goes ahead; one held to the version read before the
    // write is refused, writing nothing.
    for (id, version) in [("U-0", &reported[0]), ("U-1", &retitled), ("U-2", &moved)] {
        let held = held_retitle(&board, id, version);
        assert_eq!(held.code, Some(0), "{id}: {}", held.stderr);
        assert_eq!(held.json["task"]["title"], "Retitled under a hold", "{id}");
    }
    let stale = held_retitle(&board, "U-3", &before[3]);
    assert_ne!(stale.code, Some(0), "{:#}", stale.json);
    assert_eq!(
        stale.json["failure"]["kind"], "version-moved",
        "{:#}",
        stale.json
    );
    assert_eq!(stale.json["held"]["version"], json!(reported[1]));
}

#[test]
fn a_later_update_of_one_ticket_is_held_against_what_the_earlier_one_left() {
    let board = ticket_board();
    let before = board.versions(&["U-0"]).remove(0);
    let batch = json!({"schema_version": 1, "updates": [
        {"item": "board:U-0", "if_version": before, "priority": "high"},
        // Held to the version the earlier update of this batch moved the ticket past.
        {"item": "board:U-0", "if_version": before, "body": "U-0 overwritten"},
        // Guarded on the priority the earlier update replaced.
        {"item": "board:U-0", "priority": "low", "when_priority": ["none"]},
        // Guarded on the priority it now holds.
        {"item": "board:U-0", "body": "U-0 settled", "when_priority": ["high"]},
    ]});
    let answered = board.run(
        &["task", "update-many", "board", "--input", "-", "--json"],
        &batch.to_string(),
    );
    assert_eq!(answered.code, Some(4), "{}", answered.stderr);
    let outcomes: Vec<&str> = answered.json["results"]
        .as_array()
        .expect("results")
        .iter()
        .map(|result| result["outcome"].as_str().expect("an outcome"))
        .collect();
    assert_eq!(
        outcomes,
        ["written", "version-moved", "held", "written"],
        "{:#}",
        answered.json
    );
    assert_eq!(board.board.priority("U-0").as_deref(), Some("High"));
    let body = board.board.body("U-0");
    assert!(
        body.as_str()
            .is_some_and(|body| body.starts_with("U-0 settled")),
        "{body}"
    );
}

/// Two tickets updated together, the second's write answered as `answer` bends its `alias`: the
/// second's update fails naming why and the first's lands; the second's issue update is sent
/// only when its field write was answered whole.
fn two_updates_with_one_answer_bent(alias: &'static str, answer: AliasAnswer) -> (Board, Answered) {
    let board = ticket_board();
    board.board.bend_batch_alias(alias, answer);
    let batch = json!({"schema_version": 1, "updates": [
        {"item": "board:U-0", "body": "U-0 moved", "status": "in-progress"},
        {"item": "board:U-1", "body": "U-1 moved", "status": "in-progress"},
    ]});
    let answered = board.run(
        &["task", "update-many", "board", "--input", "-", "--json"],
        &batch.to_string(),
    );
    assert_eq!(answered.code, Some(4), "{}", answered.stderr);
    let results = answered.json["results"].as_array().expect("results");
    assert_eq!(results[0]["outcome"], "written", "{:#}", answered.json);
    assert_eq!(results[1]["outcome"], "error", "{:#}", answered.json);
    assert_eq!(board.board.status("U-0").as_deref(), Some("Doing"));
    assert!(
        board
            .board
            .body("U-0")
            .as_str()
            .is_some_and(|body| body.starts_with("U-0 moved"))
    );
    (board, answered)
}

#[test]
fn a_field_write_answered_without_its_alias_fails_that_ticket_alone_and_sends_it_no_issue_update() {
    let (board, answered) = two_updates_with_one_answer_bent("field1", AliasAnswer::Missing);
    let reason = answered.json["results"][1]["reason"]
        .as_str()
        .unwrap_or_default();
    assert!(reason.contains("board:U-1"), "{reason}");
    // The field write was made; the issue update behind it was never sent.
    assert_eq!(board.board.body("U-1"), json!("ticket 1"));
}

#[test]
fn an_issue_update_answered_for_another_issue_fails_that_ticket_alone() {
    let (_, answered) = two_updates_with_one_answer_bent("update1", AliasAnswer::NamingAnother);
    let reason = answered.json["results"][1]["reason"]
        .as_str()
        .unwrap_or_default();
    assert!(reason.contains("with another issue"), "{reason}");
}

#[test]
fn a_field_write_answered_for_another_item_fails_that_ticket_alone() {
    let (board, answered) = two_updates_with_one_answer_bent("field1", AliasAnswer::NamingAnother);
    let reason = answered.json["results"][1]["reason"]
        .as_str()
        .unwrap_or_default();
    assert!(reason.contains("board:U-1"), "{reason}");
    // Its issue update is sent only once its field writes landed as asked.
    assert_eq!(board.board.body("U-1"), json!("ticket 1"));
}

#[test]
fn a_batched_comment_answered_missing_or_about_another_issue_fails_that_comment_alone() {
    for (answer, said) in [
        (AliasAnswer::Missing, "comment1"),
        (AliasAnswer::NamingAnother, "about another issue"),
    ] {
        let board = ticket_board();
        board.board.bend_batch_alias("comment1", answer);
        let batch = json!({"schema_version": 1, "comments": [
            {"item": "board:U-0", "body": "A note on 0."},
            {"item": "board:U-1", "body": "A note on 1."},
            {"item": "board:U-2", "body": "A note on 2."},
        ]});
        let answered = board.run(
            &[
                "task", "comment", "add-many", "board", "--input", "-", "--json",
            ],
            &batch.to_string(),
        );
        assert_eq!(answered.code, Some(4), "{}", answered.stderr);
        let results = answered.json["results"].as_array().expect("results");
        let outcomes: Vec<&str> = results
            .iter()
            .map(|result| result["outcome"].as_str().expect("an outcome"))
            .collect();
        assert_eq!(outcomes, ["written", "error", "written"], "{answer:?}");
        let reason = results[1]["reason"].as_str().unwrap_or_default();
        assert!(reason.contains(said), "{answer:?}: {reason}");
        for (at, id) in [(0, "board:U-0"), (2, "board:U-2")] {
            let listed = board.run(&["task", "comment", "list", id, "--json"], "");
            let bodies: Vec<&str> = listed.json["comments"]
                .as_array()
                .expect("comments")
                .iter()
                .filter_map(|comment| comment["body"].as_str())
                .collect();
            assert_eq!(bodies, [format!("A note on {at}.")], "{answer:?}");
        }
    }
}

#[test]
fn a_batched_write_answered_beside_an_error_naming_it_is_reported_as_possibly_done() {
    let board = ticket_board();
    board
        .board
        .error_beside_alias_once("update1", "Something went wrong reading the issue");
    let updates = json!({"schema_version": 1, "updates": [
        {"item": "board:U-0", "body": "U-0 moved"},
        {"item": "board:U-1", "body": "U-1 moved"},
    ]});
    let updated = board.run(
        &["task", "update-many", "board", "--input", "-", "--json"],
        &updates.to_string(),
    );
    board
        .board
        .error_beside_alias_once("comment1", "Something went wrong reading the comment");
    let comments = json!({"schema_version": 1, "comments": [
        {"item": "board:U-2", "body": "A note on 2."},
        {"item": "board:U-3", "body": "A note on 3."},
    ]});
    let commented = board.run(
        &[
            "task", "comment", "add-many", "board", "--input", "-", "--json",
        ],
        &comments.to_string(),
    );
    for answered in [&updated, &commented] {
        assert_eq!(answered.code, Some(4), "{}", answered.stderr);
        let results = answered.json["results"].as_array().expect("results");
        assert_eq!(results[0]["outcome"], "written", "{:#}", answered.json);
        assert_eq!(results[1]["outcome"], "error", "{:#}", answered.json);
        let reason = results[1]["reason"].as_str().unwrap_or_default();
        assert!(
            reason.contains("beside an error naming it") && reason.contains("may have been done"),
            "{reason}"
        );
    }
}

#[test]
fn a_batched_priority_clear_lands_and_one_answered_missing_fails_that_ticket_alone() {
    let board = ticket_board();
    // U-2 holds `Low`; give U-1 the same, so the batch clears two.
    board.board.assign_priority("U-1", Some("Low"));
    board.board.bend_batch_alias("clear1", AliasAnswer::Missing);
    let batch = json!({"schema_version": 1, "updates": [
        {"item": "board:U-1", "priority": "none"},
        {"item": "board:U-2", "priority": "none"},
    ]});
    let answered = board.run(
        &["task", "update-many", "board", "--input", "-", "--json"],
        &batch.to_string(),
    );
    assert_eq!(answered.code, Some(4), "{}", answered.stderr);
    let outcomes: Vec<&str> = answered.json["results"]
        .as_array()
        .expect("results")
        .iter()
        .map(|result| result["outcome"].as_str().expect("an outcome"))
        .collect();
    assert_eq!(outcomes, ["written", "error"], "{:#}", answered.json);
    assert_eq!(board.board.priority("U-1"), None, "the first clear landed");
    let reason = answered.json["results"][1]["reason"]
        .as_str()
        .unwrap_or_default();
    assert!(reason.contains("clear1"), "{reason}");
}

#[test]
fn a_second_comment_held_to_the_version_the_first_moved_is_refused() {
    let board = ticket_board();
    let version = board.versions(&["U-0"]).remove(0);
    let batch = json!({"schema_version": 1, "comments": [
        {"item": "board:U-0", "body": "First.", "if_version": version},
        {"item": "board:U-0", "body": "Second.", "if_version": version},
        {"item": "board:U-1", "body": "Beside them."},
    ]});
    let answered = board.run(
        &[
            "task", "comment", "add-many", "board", "--input", "-", "--json",
        ],
        &batch.to_string(),
    );
    assert_eq!(answered.code, Some(4), "{}", answered.stderr);
    let outcomes: Vec<&str> = answered.json["results"]
        .as_array()
        .expect("results")
        .iter()
        .map(|result| result["outcome"].as_str().expect("an outcome"))
        .collect();
    assert_eq!(
        outcomes,
        ["written", "version-moved", "written"],
        "{:#}",
        answered.json
    );
    let listed = board.run(&["task", "comment", "list", "board:U-0", "--json"], "");
    let bodies: Vec<&str> = listed.json["comments"]
        .as_array()
        .expect("comments")
        .iter()
        .filter_map(|comment| comment["body"].as_str())
        .collect();
    assert_eq!(bodies, ["First."]);
}

#[test]
fn a_find_wider_than_one_request_is_split_across_requests_answering_every_query() {
    let board = searched_board();
    // Sixteen searches and six origin lookups: twenty-two search slots and six origin slots,
    // more of each than one request carries.
    let mut queries: Vec<Value> = (0..16)
        .map(|n| json!({"key": format!("deploy-{n}"), "text": "deploy"}))
        .collect();
    let origins = [
        "notes:o1",
        "notes:o10",
        "notes:x1",
        "notes:x2",
        "notes:x3",
        "notes:x4",
    ];
    queries.extend(
        origins
            .iter()
            .map(|origin| json!({"key": format!("from-{origin}"), "origin": origin})),
    );
    let (answered, served) = board.measured(
        &["task", "find", "board", "--queries", "-", "--json"],
        &json!({"schema_version": 1, "queries": queries}).to_string(),
    );
    assert_eq!(answered.code, Some(0), "{}", answered.stderr);
    let answer = &answered.json;
    for n in 0..16 {
        assert_eq!(
            found(answer, &format!("deploy-{n}")),
            set((0..7).map(|n| format!("D-{n}"))),
            "deploy-{n}"
        );
    }
    assert_eq!(
        found(answer, "from-notes:o1"),
        set((0..5).map(|n| format!("O-{n}")))
    );
    assert_eq!(found(answer, "from-notes:o10"), set(["O-5".to_owned()]));
    for origin in &origins[2..] {
        assert_eq!(found(answer, &format!("from-{origin}")), BTreeSet::new());
    }
    for each in answer["answers"].as_array().expect("answers") {
        assert_eq!(each["complete"], true, "{each}");
    }
    let batches: Vec<&Value> = served
        .iter()
        .filter(|(query, _)| query == graphql::SEARCH_BATCH)
        .map(|(_, variables)| variables)
        .collect();
    assert!(batches.len() >= 2, "split across requests: {served:#?}");
    for variables in batches {
        let searches = (0..onetaskgraph_github_projects::SEARCH_BATCH)
            .filter(|slot| variables[format!("ask{slot}")] == true)
            .count();
        let lookups = (0..onetaskgraph_github_projects::SEARCH_BATCH_ORIGINS)
            .filter(|slot| variables[format!("origin{slot}")] == true)
            .count();
        assert!(searches <= onetaskgraph_github_projects::SEARCH_BATCH);
        assert!(lookups <= onetaskgraph_github_projects::SEARCH_BATCH_ORIGINS);
    }
}

/// The fixture board's token, for a journey that builds the plugin itself.
struct FixtureToken;

impl onetaskgraph_plugin_api::SecretResolver for FixtureToken {
    fn get(&self, variable: &str) -> Option<secrecy::SecretString> {
        (variable == "GITHUB_PROJECTS_FIXTURE_TOKEN").then(|| "test-token".into())
    }
}

/// A comment batch carries no author through the command line, so a Rust caller handing the
/// plugin one is what reaches this: that comment is refused before anything is sent for it,
/// and its neighbour lands.
#[test]
fn a_batched_comment_handed_an_author_is_refused_alone_and_its_neighbour_lands() {
    use onetaskgraph_plugin_api::{
        BatchAnswer, BatchComment, CommentBody, NativeId, NewComment, SourceName, SourcePlugin as _,
    };
    let board = ticket_board();
    let source = onetaskgraph_github_projects::Plugin
        .build(
            &SourceName::new("board").unwrap(),
            &board.config,
            &FixtureToken,
        )
        .expect("the plugin builds");
    let comment = |author: Option<&str>, body: &str| BatchComment {
        task: NativeId("U-0".to_owned()),
        comment: NewComment {
            body: CommentBody::new(body).expect("a body"),
            author: author.map(str::to_owned),
        },
        if_version: None,
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    let answers = runtime
        .block_on(source.add_comments(&[comment(Some("Ada"), "Signed."), comment(None, "Plain.")]));
    assert!(
        matches!(&answers[0], Err(onetaskgraph_plugin_api::SourceError::Refused { message })
            if message.contains("\"Ada\"")),
        "{:?}",
        answers[0]
    );
    assert!(
        matches!(&answers[1], Ok(BatchAnswer::Written(_))),
        "{:?}",
        answers[1]
    );
    let listed = board.run(&["task", "comment", "list", "board:U-0", "--json"], "");
    let bodies: Vec<&str> = listed.json["comments"]
        .as_array()
        .expect("comments")
        .iter()
        .filter_map(|comment| comment["body"].as_str())
        .collect();
    assert_eq!(bodies, ["Plain."]);
}

/// The version a written update answers is the command's own, as every record of a write this
/// source holds is: `written_task` answers it until the command ends and nothing after, and the
/// next command's write answers the version that write left, which a settled read then gives.
#[test]
fn a_written_version_is_answered_for_the_command_that_wrote_it_and_no_longer() {
    use onetaskgraph_plugin_api::{NativeId, SourceName, SourcePlugin as _, TaskUpdate};
    let board = ticket_board();
    let source = onetaskgraph_github_projects::Plugin
        .build(
            &SourceName::new("board").unwrap(),
            &board.config,
            &FixtureToken,
        )
        .expect("the plugin builds");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    let id = NativeId("U-0".to_owned());
    let retitled = |title: &str| {
        let update = TaskUpdate {
            title: Some(title.to_owned()),
            ..TaskUpdate::default()
        };
        runtime
            .block_on(source.update_task(&id, &update))
            .expect("the update lands")
            .expect("a task of this board")
    };
    let read = board.versions(&["U-0"]).remove(0);

    let first = retitled("First");
    let written = first
        .task
        .version
        .clone()
        .expect("the version the write left");
    assert_ne!(written, read, "never the version read before the write");
    let answered = runtime
        .block_on(source.written_task(&id))
        .expect("the write's own record");
    assert_eq!(answered.version.as_deref(), Some(written.as_str()));

    runtime
        .block_on(source.end_command())
        .expect("the command ends");
    assert_eq!(
        runtime.block_on(source.written_task(&id)),
        None,
        "nothing of the last command's write is answered in the next"
    );

    board.board.pass_time(120);
    let second = retitled("Second");
    let again = second
        .task
        .version
        .clone()
        .expect("the version the write left");
    assert_ne!(
        again, written,
        "the next command's write answers its own version"
    );
    board.board.pass_time(120);
    assert_eq!(board.versions(&["U-0"]), [again]);
}
