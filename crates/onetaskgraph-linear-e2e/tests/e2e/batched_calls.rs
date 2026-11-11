//! The batched store calls on a Linear team, through the binary against the loopback Linear, each
//! held to what it answers and to the requests the loopback was sent: `task find` answered in one
//! aliased search per round of pages, a page Linear refuses ending that search alone; held copies,
//! comments and multi-field updates written each on its own terms, in one aliased request each,
//! with exactly the item Linear refuses sent alone; a project's tasks created in one request and
//! an unchanged re-copy of it reading no item on its own; a settlement held to a version a cache
//! holds sent as one request; and the team's vocabulary read once for two source instances.

use std::path::PathBuf;

use onetaskgraph_core::{ConfiguredSource, Engine, GlobalId, ResolvedSource};
use onetaskgraph_plugin_api::{SourceName, SourcePlugin as _, TaskUpdate};
use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{LinearWorkspace, document, linear_workspace};

/// One issue held before anything runs.
fn issue(id: &str, title: &str, status: (&str, &str), extra: Value) -> Value {
    let mut issue = json!({"id": id, "title": title, "content": format!("The body of {id}."),
        "status": {"category": status.0, "name": status.1}, "labels": []});
    for (key, value) in extra.as_object().into_iter().flatten() {
        issue[key] = value.clone();
    }
    issue
}

fn todo() -> (&'static str, &'static str) {
    ("todo", "Todo")
}

struct Team {
    sandbox: Sandbox,
    workspace: LinearWorkspace,
    notes: PathBuf,
    batches: PathBuf,
}

/// What one command printed and how it exited, and what the loopback was sent while it ran.
struct Ran {
    code: Option<i32>,
    json: Value,
    sent: Vec<(String, Value)>,
}

impl Team {
    fn holding(tasks: Vec<Value>, cache: bool) -> Self {
        let sandbox = Sandbox::new();
        let held = json!({"tasks": tasks, "projects": [], "documents": [], "labels": [],
                          "task_dependencies": [], "project_dependencies": []});
        let (config, workspace) = linear_workspace(&sandbox, held);
        let notes = sandbox.subdirectory("notes");
        std::fs::create_dir_all(notes.join("tasks")).expect("the tasks folder");
        std::fs::create_dir_all(notes.join("projects")).expect("the projects folder");
        let mut team = json!({"plugin": "linear", "config": config});
        if cache {
            team["cache"] = json!({"enabled": true, "dir": sandbox.subdirectory("cache")});
        }
        let mut configuration: Value = serde_json::from_str(&document(&json!({
            "team": team,
            "notes": {"plugin": "local-md", "config": {"root": notes}},
        })))
        .expect("a configuration document");
        configuration["page_size"] = json!(2);
        sandbox.project_document(&configuration.to_string());
        let batches = sandbox.subdirectory("batches");
        Self {
            sandbox,
            workspace,
            notes,
            batches,
        }
    }

    fn run(&self, arguments: &[&str]) -> Ran {
        let mark = self.workspace.served_count();
        let output = self
            .sandbox
            .command()
            .args(arguments)
            .arg("--json")
            .write_stdin("")
            .assert()
            .get_output()
            .clone();
        let json = serde_json::from_str(&stdout(&output)).unwrap_or_else(|error| {
            panic!(
                "`onetaskgraph {}` wrote no JSON ({error}):\n{}{}",
                arguments.join(" "),
                stdout(&output),
                stderr(&output)
            )
        });
        Ran {
            code: output.status.code(),
            json,
            sent: self.workspace.served_since(mark),
        }
    }

    fn ok(&self, arguments: &[&str]) -> Ran {
        let ran = self.run(arguments);
        assert_eq!(ran.code, Some(0), "`{}`: {}", arguments.join(" "), ran.json);
        ran
    }

    fn batch(&self, name: &str, document: &Value) -> String {
        let path = self.batches.join(name);
        std::fs::write(&path, document.to_string()).expect("a batch");
        path.to_str().expect("a UTF-8 path").to_owned()
    }

    fn version(&self, id: &str) -> String {
        self.ok(&["task", "show", id]).json["items"][0]["item"]["version"]
            .as_str()
            .expect("a version")
            .to_owned()
    }

    fn task(&self, id: &str) -> Value {
        self.ok(&["task", "show", id]).json["items"][0]["item"].clone()
    }

    fn note(&self, id: &str, front: &str, body: &str) {
        std::fs::write(
            self.notes.join("tasks").join(format!("{id}.md")),
            format!("---\n{front}---\n{body}"),
        )
        .expect("a task file");
    }
}

/// How many of `sent` are a write — each alias of a batched one its own.
fn mutations(sent: &[(String, Value)]) -> usize {
    sent.iter()
        .filter(|(query, _)| query.trim_start().starts_with("mutation"))
        .count()
}

/// The ids of a find answer's items, in order.
fn found(answer: &Value) -> Vec<String> {
    answer["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|item| item["id"].as_str().expect("an id").to_owned())
        .collect()
}

fn searches() -> Vec<Value> {
    let mut tasks = Vec::new();
    for (at, letter) in ["A", "B", "C"].iter().enumerate() {
        let status = if at == 2 { ("done", "Shipped") } else { todo() };
        tasks.push(issue(
            &format!("W-{at}"),
            &format!("Widget {letter}"),
            status,
            json!({}),
        ));
        tasks.push(issue(
            &format!("M-{at}"),
            &format!("Gadget {letter}"),
            todo(),
            json!({"metadata": {"team.kind": "alpha"}}),
        ));
        tasks.push(issue(
            &format!("O-{at}"),
            &format!("Copied {letter}"),
            todo(),
            json!({"metadata": {"onetaskgraph.origin": "notes:src"}}),
        ));
    }
    for at in 0..10 {
        tasks.push(issue(
            &format!("F-{at}"),
            &format!("Other {at}"),
            todo(),
            json!({}),
        ));
    }
    tasks
}

fn five_queries() -> Value {
    json!({"schema_version": 1, "queries": [
        {"key": "by-text", "text": "Widget", "in": "title"},
        {"key": "by-metadata", "metadata": {"team.kind": "alpha"}},
        {"key": "by-origin", "origin": "notes:src"},
        {"key": "by-text-and-status", "text": "Widget", "status": ["done"]},
        {"key": "matching-nothing", "text": "nothing holds this"},
    ]})
}

#[test]
fn five_searches_are_answered_together_one_request_per_round_of_pages() {
    let team = Team::holding(searches(), false);
    let queries = team.batch("find.json", &five_queries());
    let ran = team.ok(&["task", "find", "team", "--queries", &queries]);
    let answers = ran.json["answers"].as_array().expect("answers");
    let by_key = |key: &str| {
        answers
            .iter()
            .find(|answer| answer["key"] == key)
            .unwrap_or_else(|| panic!("{key} is answered: {}", ran.json))
    };
    let mut expected = [
        ("by-text", vec!["team:W-0", "team:W-1", "team:W-2"]),
        ("by-metadata", vec!["team:M-0", "team:M-1", "team:M-2"]),
        ("by-origin", vec!["team:O-0", "team:O-1", "team:O-2"]),
        ("by-text-and-status", vec!["team:W-2"]),
        ("matching-nothing", vec![]),
    ];
    for (key, ids) in &mut expected {
        let answer = by_key(key);
        let mut held = found(answer);
        held.sort();
        ids.sort_unstable();
        assert_eq!(held, *ids, "{key}: {answer}");
        assert_eq!(answer["complete"], true, "{answer}");
        assert_eq!(answer["reason"], Value::Null, "{answer}");
    }
    // Three of the searches hold more than a page of two: two rounds, each one request carrying
    // every search still walking — never one request per search.
    let searched: Vec<&(String, Value)> = ran
        .sent
        .iter()
        .filter(|(query, _)| query.contains("issues("))
        .collect();
    assert_eq!(searched.len(), 2, "one request per round: {searched:#?}");
    assert_eq!(
        onetaskgraph_linear::graphql::unaliased(&searched[0].0).map(|(_, count)| count),
        Some(5),
        "the first round carries all five"
    );
}

#[test]
fn a_search_narrowed_by_nothing_is_refused_by_its_key_reading_nothing() {
    let team = Team::holding(searches(), false);
    let queries = team.batch(
        "find.json",
        &json!({"schema_version": 1, "queries": [
            {"key": "fine", "text": "Widget"},
            {"key": "everything", "status": ["todo"]},
        ]}),
    );
    let ran = team.run(&["task", "find", "team", "--queries", &queries]);
    assert_eq!(ran.code, Some(1), "{}", ran.json);
    assert_eq!(ran.json["failure"]["kind"], "unnarrowed-query");
    assert!(
        ran.json["failure"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("\"everything\"")),
        "{}",
        ran.json
    );
    assert!(ran.sent.is_empty(), "nothing is read: {:?}", ran.sent);
}

#[test]
fn a_refused_later_page_ends_that_search_alone() {
    let team = Team::holding(searches(), false);
    // The plugin narrows by a metadata value as its spelling in the issue's slot.
    team.workspace.refuse_later_pages_of("alpha");
    let queries = team.batch("find.json", &five_queries());
    let ran = team.run(&["task", "find", "team", "--queries", &queries]);
    assert_eq!(ran.code, Some(4), "{}", ran.json);
    for answer in ran.json["answers"].as_array().expect("answers") {
        if answer["key"] == "by-metadata" {
            assert_eq!(answer["complete"], false, "{answer}");
            assert!(
                answer["reason"]
                    .as_str()
                    .is_some_and(|reason| reason.contains("page 2")),
                "the reason names the refused page: {answer}"
            );
            assert_eq!(found(answer).len(), 2, "the first page stands: {answer}");
        } else {
            assert_eq!(answer["complete"], true, "{answer}");
        }
    }
}

#[test]
fn a_search_answering_a_cursor_it_already_followed_is_stopped_there_alone() {
    let team = Team::holding(searches(), false);
    // Every later page of the metadata search answers the cursor it was asked from.
    team.workspace.stall_later_pages_of("alpha");
    let queries = team.batch("find.json", &five_queries());
    let ran = team.run(&["task", "find", "team", "--queries", &queries]);
    assert_eq!(ran.code, Some(4), "{}", ran.json);
    for answer in ran.json["answers"].as_array().expect("answers") {
        if answer["key"] == "by-metadata" {
            assert_eq!(answer["complete"], false, "{answer}");
            assert!(
                answer["reason"].as_str().is_some_and(
                    |reason| reason.contains("page 2") && reason.contains("already followed")
                ),
                "the reason names the page and its repeated cursor: {answer}"
            );
            // Both pages' rows stand: the second's own rows were read, and only its cursor
            // was not followed.
            assert_eq!(found(answer).len(), 3, "both pages' rows stand: {answer}");
        } else {
            assert_eq!(answer["complete"], true, "{answer}");
        }
    }
}

fn writable() -> Vec<Value> {
    (1..=4)
        .map(|at| {
            let priority = if at == 3 { "high" } else { "low" };
            issue(
                &format!("U-{at}"),
                &format!("Update {at}"),
                todo(),
                json!({"priority": priority}),
            )
        })
        .collect()
}

#[test]
fn held_comments_and_updates_are_written_each_on_its_own_terms_in_one_request_each() {
    let team = Team::holding(writable(), false);
    let versions: Vec<String> = (1..=4)
        .map(|at| team.version(&format!("team:U-{at}")))
        .collect();
    // Another writer moves U-2 after its version was read.
    team.workspace
        .retitle("tasks", "U-2", "Another writer's title");

    let comments = team.batch(
        "comments.json",
        &json!({"schema_version": 1, "comments": (1..=4).map(|at| json!({
            "item": format!("team:U-{at}"), "body": format!("A note on {at}."),
            "if_version": versions[at - 1]})).collect::<Vec<_>>()}),
    );
    let ran = team.run(&["task", "comment", "add-many", "team", "--input", &comments]);
    assert_eq!(ran.code, Some(4), "{}", ran.json);
    let results = ran.json["results"].as_array().expect("results");
    for (at, result) in results.iter().enumerate() {
        if at == 1 {
            assert_eq!(result["outcome"], "version-moved", "{result}");
            assert!(result["current_version"].is_string(), "{result}");
        } else {
            assert_eq!(result["outcome"], "written", "{result}");
            assert!(result["comment"]["id"].is_string(), "{result}");
        }
    }
    assert_eq!(
        mutations(&ran.sent),
        1,
        "the three comments go in one request: {:#?}",
        ran.sent
    );
    assert_eq!(ran.sent.len(), 2, "one read deciding every hold, one write");

    let versions: Vec<String> = (1..=4)
        .map(|at| team.version(&format!("team:U-{at}")))
        .collect();
    team.workspace
        .retitle("tasks", "U-2", "Another writer's title, again");
    let updates = team.batch(
        "updates.json",
        &json!({"schema_version": 1, "updates": (1..=4).map(|at| json!({
            "item": format!("team:U-{at}"), "if_version": versions[at - 1],
            "body": format!("New body {at}.\n"), "status": "in-progress",
            "priority": "urgent", "when_priority": ["low"],
            "metadata": {"team.state": format!("updated-{at}")}})).collect::<Vec<_>>()}),
    );
    let ran = team.run(&["task", "update-many", "team", "--input", &updates]);
    assert_eq!(ran.code, Some(4), "{}", ran.json);
    let outcomes: Vec<&str> = ran.json["results"]
        .as_array()
        .expect("results")
        .iter()
        .map(|result| result["outcome"].as_str().expect("an outcome"))
        .collect();
    assert_eq!(
        outcomes,
        ["written", "version-moved", "held", "written"],
        "{}",
        ran.json
    );
    assert_eq!(
        mutations(&ran.sent),
        1,
        "the two written updates go in one request: {:#?}",
        ran.sent
    );
    for at in [1, 4] {
        let task = team.task(&format!("team:U-{at}"));
        assert_eq!(task["content"], format!("New body {at}.\n"), "{task}");
        assert_eq!(task["status"]["category"], "in-progress", "{task}");
        assert_eq!(task["priority"], "urgent", "{task}");
        assert_eq!(task["metadata"]["team.state"], format!("updated-{at}"));
    }
    for at in [2, 3] {
        let task = team.task(&format!("team:U-{at}"));
        assert_eq!(task["content"], format!("The body of U-{at}."), "{task}");
        assert_eq!(task["status"]["category"], "todo", "{task}");
        assert!(task["metadata"].get("team.state").is_none(), "{task}");
    }
    assert_eq!(
        team.task("team:U-2")["title"],
        "Another writer's title, again",
        "the moved task is left as the other writer left it"
    );
}

#[test]
fn later_updates_of_one_issue_are_each_written_on_what_the_earlier_one_left() {
    let team = Team::holding(writable(), false);
    // U-1 is at `low` priority.
    let before = team.version("team:U-1");
    let updates = team.batch(
        "updates.json",
        &json!({"schema_version": 1, "updates": [
            {"item": "team:U-1", "if_version": before, "priority": "high",
             "metadata": {"team.first": true}},
            {"item": "team:U-2", "title": "Beside them"},
            // Held to the version the first update moved the issue past.
            {"item": "team:U-1", "if_version": before, "body": "Overwritten.\n"},
            // Guarded on the priority the first update replaced.
            {"item": "team:U-1", "priority": "urgent", "when_priority": ["low"]},
            // Guarded on the priority it now holds, and keeping the key the first one set.
            {"item": "team:U-1", "body": "Settled.\n", "when_priority": ["high"],
             "metadata": {"team.last": true}},
        ]}),
    );
    let ran = team.run(&["task", "update-many", "team", "--input", &updates]);
    assert_eq!(ran.code, Some(4), "{}", ran.json);
    let outcomes: Vec<&str> = ran.json["results"]
        .as_array()
        .expect("results")
        .iter()
        .map(|result| result["outcome"].as_str().expect("an outcome"))
        .collect();
    assert_eq!(
        outcomes,
        ["written", "written", "version-moved", "held", "written"],
        "{}",
        ran.json
    );
    let task = team.task("team:U-1");
    assert_eq!(task["content"], "Settled.\n", "{task}");
    assert_eq!(task["priority"], "high", "{task}");
    assert_eq!(task["metadata"]["team.first"], true, "{task}");
    assert_eq!(task["metadata"]["team.last"], true, "{task}");
    assert_eq!(team.task("team:U-2")["title"], "Beside them");
}

#[test]
fn a_guarded_update_writes_all_three_only_while_the_guard_holds() {
    let team = Team::holding(writable(), false);
    let body = team.batches.join("body.md");
    std::fs::write(&body, "Guarded body.\n").expect("a body");
    let path = body.to_str().expect("a path");
    let held = team.ok(&[
        "task",
        "update",
        "team:U-3",
        "--body-file",
        path,
        "--metadata",
        "team.guarded=true",
        "--priority",
        "urgent",
        "--when-priority",
        "low",
    ]);
    assert_eq!(held.json["guard"]["outcome"], "held", "{}", held.json);
    assert_eq!(mutations(&held.sent), 0);
    let task = team.task("team:U-3");
    assert_eq!(task["priority"], "high");
    assert!(task["metadata"].get("team.guarded").is_none());

    let written = team.ok(&[
        "task",
        "update",
        "team:U-1",
        "--body-file",
        path,
        "--metadata",
        "team.guarded=true",
        "--priority",
        "urgent",
        "--when-priority",
        "low",
    ]);
    assert_eq!(
        written.json["guard"]["outcome"], "written",
        "{}",
        written.json
    );
    let task = team.task("team:U-1");
    assert_eq!(task["content"], "Guarded body.\n");
    assert_eq!(task["priority"], "urgent");
    assert_eq!(task["metadata"]["team.guarded"], true);
}

#[test]
fn a_missing_item_and_a_refused_one_fail_alone_and_the_rest_are_written() {
    let team = Team::holding(writable(), false);
    team.workspace.refuse_writes_of("U-4");
    let updates = team.batch(
        "updates.json",
        &json!({"schema_version": 1, "updates": [
            {"item": "team:U-1", "title": "One"},
            {"item": "team:U-2", "title": "Two"},
            {"item": "team:NOPE", "title": "Nothing"},
            {"item": "team:U-3", "title": "Three"},
            {"item": "team:U-4", "title": "Four"},
        ]}),
    );
    let ran = team.run(&["task", "update-many", "team", "--input", &updates]);
    assert_eq!(ran.code, Some(4), "{}", ran.json);
    let results = ran.json["results"].as_array().expect("results");
    for (at, title) in [(0, "One"), (1, "Two"), (3, "Three")] {
        assert_eq!(results[at]["outcome"], "written", "{}", results[at]);
        let id = results[at]["item"].as_str().expect("an item");
        assert_eq!(team.task(id)["title"], title);
    }
    for at in [2, 4] {
        assert_eq!(results[at]["outcome"], "error", "{}", results[at]);
        let item = results[at]["item"].as_str().expect("an item");
        assert!(
            results[at]["reason"]
                .as_str()
                .is_some_and(|reason| reason.contains(item.trim_start_matches("team:"))),
            "the reason names {item}: {}",
            results[at]
        );
    }
    // Linear refuses the batch at U-4's alias, landing nothing: U-4 alone, the rest together.
    let writes: Vec<usize> = ran
        .sent
        .iter()
        .filter(|(query, _)| query.trim_start().starts_with("mutation"))
        .map(|(query, _)| {
            onetaskgraph_linear::graphql::unaliased(query).map_or(1, |(_, count)| count)
        })
        .collect();
    assert_eq!(writes, [4, 1, 3], "{:#?}", ran.sent);

    let comments = team.batch(
        "comments.json",
        &json!({"schema_version": 1, "comments": [
            {"item": "team:U-1", "body": "One."},
            {"item": "team:NOPE", "body": "Nothing."},
            {"item": "team:U-2", "body": "Two."},
            {"item": "team:U-3", "body": "Three."},
        ]}),
    );
    let ran = team.run(&["task", "comment", "add-many", "team", "--input", &comments]);
    assert_eq!(ran.code, Some(4), "{}", ran.json);
    let outcomes: Vec<&str> = ran.json["results"]
        .as_array()
        .expect("results")
        .iter()
        .map(|result| result["outcome"].as_str().expect("an outcome"))
        .collect();
    assert_eq!(outcomes, ["written", "error", "written", "written"]);
    assert!(
        ran.json["results"][1]["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("NOPE"))
    );
}

#[test]
fn held_copies_write_every_item_but_the_one_another_writer_moved() {
    let team = Team::holding(Vec::new(), false);
    for at in 1..=4 {
        team.note(
            &format!("C-{at}"),
            "title: Copied\nstatus: todo\n",
            &format!("First {at}.\n"),
        );
    }
    let first = team.ok(&[
        "task",
        "copy",
        "notes:C-1",
        "notes:C-2",
        "notes:C-3",
        "notes:C-4",
        "--to",
        "team",
    ]);
    let landed = |report: &Value, source: &str| -> (String, String) {
        let item = report["items"]
            .as_array()
            .expect("items")
            .iter()
            .find(|item| item["source"] == source)
            .unwrap_or_else(|| panic!("{source}: {report}"))
            .clone();
        (
            item["destination"]
                .as_str()
                .expect("a destination")
                .to_owned(),
            item["result"]["version"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
        )
    };
    let (c1, v1) = landed(&first.json, "notes:C-1");
    let (c2, v2) = landed(&first.json, "notes:C-2");
    for at in 1..=4 {
        team.note(
            &format!("C-{at}"),
            "title: Copied\nstatus: todo\n",
            &format!("Second {at}.\n"),
        );
    }
    team.workspace.retitle(
        "tasks",
        c2.trim_start_matches("team:"),
        "Another writer's title",
    );
    // C-1's counterpart is still at the version the first copy answered with.
    assert_eq!(team.version(&c1), v1);
    let held_1 = format!("notes:C-1={v1}");
    let held_2 = format!("notes:C-2={v2}");
    let ran = team.run(&[
        "task",
        "copy",
        "notes:C-1",
        "notes:C-2",
        "notes:C-3",
        "notes:C-4",
        "--to",
        "team",
        "--if-version",
        &held_1,
        "--if-version",
        &held_2,
    ]);
    assert_eq!(ran.code, Some(4), "{}", ran.json);
    let items = ran.json["items"].as_array().expect("items");
    for item in items {
        if item["source"] == "notes:C-2" {
            assert_eq!(item["action"], "failed", "{item}");
            assert_eq!(item["error"]["kind"], "version-moved", "{item}");
            assert!(item["held"]["version"].is_string(), "{item}");
        } else {
            assert_eq!(item["action"], "updated", "{item}");
        }
    }
    let moved = team.task(&c2);
    assert_eq!(moved["title"], "Another writer's title");
    assert_eq!(moved["content"], "First 2.", "{moved}");
    assert_eq!(team.task(&c1)["content"], "Second 1.");
}

#[test]
fn a_five_task_project_is_created_in_one_request_and_re_copied_reading_no_item_alone() {
    let team = Team::holding(Vec::new(), false);
    std::fs::write(
        team.notes.join("projects").join("P.md"),
        "---\ntitle: The plan\nstatus: todo\n---\nThe plan.\n",
    )
    .expect("a project");
    for at in 1..=5 {
        team.note(
            &format!("T-{at}"),
            "title: Task {at}\nstatus: todo\nproject: P\n"
                .replace("{at}", &at.to_string())
                .as_str(),
            &format!("What task {at} does.\n"),
        );
    }
    let copy = ["project", "copy", "notes:P", "--to", "team"];
    let first = team.ok(&copy);
    let creates: Vec<usize> = first
        .sent
        .iter()
        .filter(|(query, _)| query.contains("issueCreate("))
        .map(|(query, _)| {
            onetaskgraph_linear::graphql::unaliased(query).map_or(1, |(_, count)| count)
        })
        .collect();
    assert_eq!(
        creates,
        [5],
        "the five creates go in one request: {:#?}",
        first.sent
    );
    for at in 1..=5 {
        let landed = first.json["items"]
            .as_array()
            .expect("items")
            .iter()
            .find(|item| item["source"] == format!("notes:T-{at}"))
            .expect("the task landed")["destination"]
            .as_str()
            .expect("a destination")
            .to_owned();
        let task = team.task(&landed);
        assert_eq!(task["title"], format!("Task {at}"), "{task}");
        assert_eq!(task["content"], format!("What task {at} does."), "{task}");
        assert_eq!(task["status"]["category"], "todo", "{task}");
        let edges = team.ok(&["task", "deps", &landed]).json;
        let ends: Vec<&str> = edges["items"]
            .as_array()
            .expect("edges")
            .iter()
            .map(|edge| edge["to"]["id"].as_str().expect("an end"))
            .collect();
        assert!(
            ends.is_empty(),
            "task {at} depends on nothing, as at its source: {ends:?}"
        );
    }

    let again = team.ok(&copy);
    for item in again.json["items"].as_array().expect("items") {
        assert_eq!(item["action"], "unchanged", "{item}");
    }
    assert_eq!(mutations(&again.sent), 0, "{:#?}", again.sent);
    let alone: Vec<&String> = again
        .sent
        .iter()
        .map(|(query, _)| query)
        .filter(|query| {
            [
                onetaskgraph_linear::graphql::ISSUE,
                onetaskgraph_linear::graphql::ISSUE_DEPENDS_ON,
                onetaskgraph_linear::graphql::ISSUE_DETAIL,
            ]
            .contains(&query.as_str())
        })
        .collect();
    assert!(alone.is_empty(), "no task is read on its own: {alone:#?}");
}

/// A settlement — a status and a metadata key — of a task whose version the caller holds, sent
/// through a long-lived engine whose source's cache holds the task at that version and has read
/// the team's vocabulary already: one request, the update, and no read of the task first.
#[test]
fn a_settlement_at_a_version_the_caller_holds_is_one_request() {
    let team = Team::holding(writable(), true);
    let environment = onetaskgraph_core::Environment::from_pairs([
        (
            "XDG_CONFIG_HOME",
            team.sandbox.config_home().to_string_lossy().into_owned(),
        ),
        (
            "XDG_CACHE_HOME",
            team.sandbox
                .subdirectory("cache")
                .to_string_lossy()
                .into_owned(),
        ),
        ("LINEAR_API_KEY", "fixture-key".to_owned()),
    ]);
    let loaded = onetaskgraph_core::config::load(
        &team.sandbox.project(),
        &environment,
        &onetaskgraph_core::config::Layer::default(),
    )
    .expect("the configuration loads");
    let engine = Engine::build(&loaded.config, &loaded.secrets);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    let settle = |key: &str| {
        let mut update = TaskUpdate {
            status: Some(onetaskgraph_plugin_api::Status {
                category: onetaskgraph_plugin_api::StatusCategory::Done,
                name: "done".to_owned(),
            }),
            ..TaskUpdate::default()
        };
        update.metadata_set.insert(
            onetaskgraph_plugin_api::MetadataKey::new(key).unwrap(),
            json!(true),
        );
        update
    };
    // An earlier settlement in the same engine has read the team's vocabulary.
    let earlier: GlobalId = "team:U-2".parse().unwrap();
    runtime
        .block_on(engine.update_task(&earlier, &settle("team.settled")))
        .expect("the earlier settlement lands");
    runtime
        .block_on(engine.end_command())
        .expect("the command ends");
    let task: GlobalId = "team:U-1".parse().unwrap();
    let read = runtime
        .block_on(engine.task(&task))
        .expect("the task reads");
    let version = read.items[0].item.version.clone().expect("a version");
    // A read pinned to that version fills the cache's entry, as a caller holding it has read it.
    engine.pin(&task, &version);
    runtime
        .block_on(engine.task(&task))
        .expect("the task reads");
    runtime
        .block_on(engine.end_command())
        .expect("the command ends");
    let mark = team.workspace.served_count();
    engine.pin(&task, &version);
    engine.expect(&task, &version).expect("the cache holds it");
    runtime
        .block_on(engine.update_task(&task, &settle("team.settled")))
        .expect("the settlement lands");
    let sent = team.workspace.served_since(mark);
    assert_eq!(sent.len(), 1, "the settlement alone: {sent:#?}");
    assert_eq!(mutations(&sent), 1);
    let held = team.task("team:U-1");
    assert_eq!(held["status"]["category"], "done");
    assert_eq!(held["metadata"]["team.settled"], true);
}

/// Two Linear sources over one team in one process — two instances of one tracker — resolve the
/// team's vocabulary once between them, across a write through each.
#[test]
fn two_instances_of_one_tracker_read_its_vocabulary_once_across_two_writes() {
    let sandbox = Sandbox::new();
    let held = json!({"tasks": writable(), "projects": [], "documents": [], "labels": [],
                      "task_dependencies": [], "project_dependencies": []});
    let (config, workspace) = linear_workspace(&sandbox, held);
    let secrets = crate::linear_budget::secrets();
    let mut sources = Vec::new();
    let mut selection = Vec::new();
    for name in ["north", "south"] {
        let name = SourceName::new(name).unwrap();
        let source = onetaskgraph_linear::Plugin
            .build(&name, &config, &secrets)
            .expect("the Linear source builds");
        sources.push(ConfiguredSource::Ready(ResolvedSource::adopt(
            name.clone(),
            source,
        )));
        selection.push(name);
    }
    let engine = Engine::new(sources, selection);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    let mark = workspace.served_count();
    for (source, id) in [("north", "U-1"), ("south", "U-2")] {
        let task: GlobalId = format!("{source}:{id}").parse().unwrap();
        let update = TaskUpdate {
            status: Some(onetaskgraph_plugin_api::Status {
                category: onetaskgraph_plugin_api::StatusCategory::Done,
                name: "done".to_owned(),
            }),
            title: Some(format!("Settled {id}")),
            ..TaskUpdate::default()
        };
        runtime
            .block_on(engine.update_task(&task, &update))
            .expect("the update lands");
    }
    let resolutions = workspace
        .served_since(mark)
        .iter()
        .filter(|(query, _)| query.contains("teams(first:2"))
        .count();
    assert_eq!(resolutions, 1, "{:#?}", workspace.served_since(mark));
}

/// Two Linear sources over one team in one process share the ids of the labels their writes
/// name: the first create looks both labels up in one request and the second looks up neither;
/// a create Linear refuses drops what the tracker holds, so the next create, through the other
/// instance, looks them up again rather than sending an id that may be what was refused.
#[test]
fn two_instances_of_one_tracker_share_label_ids_until_a_refused_write_drops_them() {
    use onetaskgraph_core::{Body, TaskCreate};
    let sandbox = Sandbox::new();
    let held = json!({"tasks": writable(), "documents": [], "labels": [],
                      "projects": [{"id": "P-1", "title": "Plans", "content": "The plans.",
                                    "status": {"category": "todo", "name": "Todo"},
                                    "labels": []}],
                      "task_dependencies": [], "project_dependencies": []});
    let (config, workspace) = linear_workspace(&sandbox, held);
    let secrets = crate::linear_budget::secrets();
    let mut sources = Vec::new();
    let mut selection = Vec::new();
    for name in ["east", "west"] {
        let name = SourceName::new(name).unwrap();
        let source = onetaskgraph_linear::Plugin
            .build(&name, &config, &secrets)
            .expect("the Linear source builds");
        sources.push(ConfiguredSource::Ready(ResolvedSource::adopt(
            name.clone(),
            source,
        )));
        selection.push(name);
    }
    let engine = Engine::new(sources, selection);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    let create = |source: &str, title: &str| TaskCreate {
        source: SourceName::new(source).unwrap(),
        project: onetaskgraph_plugin_api::NativeId::from("P-1"),
        title: title.to_owned(),
        body: Body::plain("A labelled task."),
        status: None,
        labels: vec!["triage".to_owned(), "ops".to_owned()],
        repositories: Vec::new(),
        depends_on: Vec::new(),
        delivers: Vec::new(),
        metadata: std::collections::BTreeMap::new(),
        assets: Vec::new(),
        classification: Default::default(),
    };
    let lookups = |mark: usize| -> usize {
        workspace
            .served_since(mark)
            .iter()
            .filter(|(query, _)| query.contains("issueLabels("))
            .count()
    };

    let mark = workspace.served_count();
    runtime
        .block_on(engine.create_task(&create("east", "First")))
        .expect("the first create lands");
    assert_eq!(lookups(mark), 1, "both labels in one lookup");
    let mark = workspace.served_count();
    runtime
        .block_on(engine.create_task(&create("west", "Second")))
        .expect("the second create lands");
    assert_eq!(lookups(mark), 0, "the other instance's ids are used");

    workspace.refuse_creates_titled("Refused");
    runtime
        .block_on(engine.create_task(&create("west", "Refused")))
        .expect_err("Linear refuses the create");
    let mark = workspace.served_count();
    let created = runtime
        .block_on(engine.create_task(&create("east", "Third")))
        .expect("the third create lands");
    assert_eq!(lookups(mark), 1, "looked up again after the refusal");
    let mut labels: Vec<String> = created
        .task
        .item
        .labels
        .iter()
        .map(|label| label.name.clone())
        .collect();
    labels.sort();
    assert_eq!(labels, ["ops", "triage"]);
}

#[test]
fn a_comment_linear_refuses_in_a_batch_fails_alone_and_every_other_lands_once() {
    let team = Team::holding(writable(), false);
    team.workspace.refuse_writes_of("U-2");
    let comments = team.batch(
        "comments.json",
        &json!({"schema_version": 1, "comments": (1..=4).map(|at| json!({
            "item": format!("team:U-{at}"), "body": format!("A note on {at}.")
        })).collect::<Vec<_>>()}),
    );
    let ran = team.run(&["task", "comment", "add-many", "team", "--input", &comments]);
    assert_eq!(ran.code, Some(4), "{}", ran.json);
    let outcomes: Vec<&str> = ran.json["results"]
        .as_array()
        .expect("results")
        .iter()
        .map(|result| result["outcome"].as_str().expect("an outcome"))
        .collect();
    assert_eq!(
        outcomes,
        ["written", "error", "written", "written"],
        "{}",
        ran.json
    );
    for at in 1..=4 {
        let listed = team.ok(&["task", "comment", "list", &format!("team:U-{at}")]);
        let bodies: Vec<&str> = listed.json["comments"]
            .as_array()
            .expect("comments")
            .iter()
            .filter_map(|comment| comment["body"].as_str())
            .collect();
        let expected: Vec<String> = if at == 2 {
            Vec::new()
        } else {
            vec![format!("A note on {at}.")]
        };
        assert_eq!(bodies, expected, "U-{at}");
    }
}

#[test]
fn a_guarded_update_is_held_to_the_version_its_guard_read_and_refused_once_it_moves() {
    let team = Team::holding(writable(), false);
    // Another writer retitles U-1 straight after the guard has read it.
    team.workspace.after_request(
        |_, variables| variables["id"] == "U-1",
        0,
        |workspace| workspace.retitle("tasks", "U-1", "Another writer's title"),
    );
    let ran = team.run(&[
        "task",
        "update",
        "team:U-1",
        "--title",
        "Guarded title",
        "--when-priority",
        "low",
    ]);
    assert_eq!(ran.code, Some(1), "{}", ran.json);
    assert_eq!(ran.json["failure"]["kind"], "version-moved", "{}", ran.json);
    assert_eq!(mutations(&ran.sent), 0, "nothing was written");
    assert_eq!(team.task("team:U-1")["title"], "Another writer's title");
}
