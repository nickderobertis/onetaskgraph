//! A task's assignees on a GitHub board, driven the way a user drives them, against the
//! loopback board.
//!
//! A board opts in with `config.assignees: true`: its tasks then carry every login the issue
//! holds, read on the reads it already made, and a write names one person or none, in the
//! `createIssue` or `updateIssue` it already sends — after one lookup of the logins this process
//! has not resolved. GitHub leaves a person it cannot assign off without an error, so each write
//! is held to its own answer. Every read-back is a fresh process, and every journey counts the
//! requests the board was sent; a board that has not opted in sends none of it.

use std::path::PathBuf;
use std::process::Output;

use onetaskgraph_e2e_support::{
    common::{Sandbox, stderr, stdout},
    fixtures::{GitHubBoardFields, document, github_projects_with_items},
};
use serde_json::{Value, json};

/// The issue the board seeds with three assignees.
const SEEDED: &str = "A-1";
/// Another issue seeded with the same three.
const TWIN: &str = "A-2";
/// The three people both hold.
const THREE: [&str; 3] = ["octocat", "hubot", "monalisa"];

/// One loopback board holding the shared dataset, plain open issues `A-1` to `A-12` and the
/// draft `SKETCH`, as source `board`, beside a folder of Markdown `plans` opted in to assignees.
struct Assigning {
    sandbox: Sandbox,
    board: GitHubBoardFields,
    plans: PathBuf,
    /// Where `XDG_CACHE_HOME` points, for a board configured with its cache on.
    cache_home: PathBuf,
}

impl Assigning {
    /// The board opted in to assignees when `opted`, with `cache` as its `cache` block.
    fn configured(opted: bool, cache: Option<Value>) -> Self {
        let sandbox = Sandbox::new();
        let items = (1..=12)
            .map(|n| {
                let id = format!("A-{n}");
                json!({"item": format!("ITEM-{id}"), "id": id, "type": "Issue",
                    "title": format!("Assignable {n}"), "body": format!("assignable {n}"),
                    "state": "OPEN", "reason": null, "parent": null,
                    "repo": "nickderobertis/onetaskgraph", "status": "Todo", "origin": "",
                    "labels": []})
            })
            .chain([
                json!({"item": "ITEM-SKETCH", "id": "SKETCH", "type": "DraftIssue",
                "title": "Sketch", "body": "a draft nobody converted", "state": "OPEN",
                "reason": null, "parent": null, "repo": null, "status": "Todo", "origin": "",
                "labels": []}),
            ])
            .collect();
        let (mut config, board) = github_projects_with_items(&sandbox, items);
        if opted {
            config["assignees"] = json!(true);
        }
        board.seed_assignees(SEEDED, &THREE);
        board.seed_assignees(TWIN, &THREE);
        let plans = sandbox.subdirectory("plans");
        std::fs::create_dir_all(plans.join("tasks")).expect("the task folder");
        let mut source = json!({"plugin": "github-projects", "config": config});
        if let Some(cache) = cache {
            source["cache"] = cache;
        }
        sandbox.project_document(&document(&json!({
            "board": source,
            "plans": {"plugin": "local-md", "config": {"root": plans, "assignees": true,
                "status_mapping": {"Todo": "todo", "Doing": "in-progress"}}},
        })));
        let cache_home = sandbox.subdirectory("cache-home");
        Self {
            sandbox,
            board,
            plans,
            cache_home,
        }
    }

    fn new() -> Self {
        Self::configured(true, None)
    }

    /// One task file of `plans`, assigned to `person`.
    fn plan(&self, native: &str, body: &str, person: &str) {
        std::fs::write(
            self.plans.join("tasks").join(format!("{native}.md")),
            format!(
                "---\ntitle: Plan {native}\nstatus: todo\nassignees: [{person}]\n---\n{body}\n"
            ),
        )
        .expect("a task file");
    }

    /// `onetaskgraph --json <arguments>` with `input` on standard input: what it printed and
    /// every request the board was sent for it.
    fn run(&self, arguments: &[&str], input: Option<&str>) -> (Output, Vec<(String, Value)>) {
        let mark = self.board.served_count();
        let output = self
            .sandbox
            .command()
            .env("XDG_CACHE_HOME", &self.cache_home)
            .arg("--json")
            .args(arguments)
            .write_stdin(input.unwrap_or_default().to_owned())
            .assert()
            .get_output()
            .clone();
        (output, self.board.served_since(mark))
    }

    /// [`Self::run`], exiting `code`, as the JSON it printed.
    fn exited(
        &self,
        arguments: &[&str],
        input: Option<&str>,
        code: i32,
    ) -> (Value, Vec<(String, Value)>) {
        let (output, served) = self.run(arguments, input);
        assert_eq!(
            output.status.code(),
            Some(code),
            "`onetaskgraph {}`\nstdout:\n{}\nstderr:\n{}",
            arguments.join(" "),
            stdout(&output),
            stderr(&output)
        );
        let printed = serde_json::from_str(&stdout(&output)).unwrap_or_else(|error| {
            panic!(
                "`onetaskgraph {}` printed no JSON ({error}): {}",
                arguments.join(" "),
                stdout(&output)
            )
        });
        (printed, served)
    }

    fn ok(&self, arguments: &[&str]) -> (Value, Vec<(String, Value)>) {
        self.exited(arguments, None, 0)
    }

    /// The task `id` as a fresh `task show` reads it.
    fn shown(&self, id: &str) -> Value {
        self.ok(&["task", "show", id]).0["items"][0]["item"].clone()
    }

    /// Whom a fresh process reads the task `id` assigned to.
    fn people(&self, id: &str) -> Value {
        self.shown(id)["assignees"].clone()
    }

    fn update_many(&self, updates: &Value, code: i32) -> (Value, Vec<(String, Value)>) {
        let batch = json!({"schema_version": 1, "updates": updates}).to_string();
        self.exited(
            &["task", "update-many", "board", "--input", "-"],
            Some(&batch),
            code,
        )
    }
}

/// The mutations among `served`.
fn writes(served: &[(String, Value)]) -> Vec<&(String, Value)> {
    served
        .iter()
        .filter(|(document, _)| document.trim_start().starts_with("mutation"))
        .collect()
}

/// The person lookups among `served`.
fn lookups(served: &[(String, Value)]) -> usize {
    served
        .iter()
        .filter(|(document, _)| document.contains(":user(login:"))
        .count()
}

/// Whether any request of `served` sends an assignee write.
fn assigns(served: &[(String, Value)]) -> bool {
    served
        .iter()
        .any(|(_, variables)| variables.to_string().contains("assigneeIds"))
}

/// Each result's `outcome`, in order.
fn outcomes(answer: &Value) -> Vec<&str> {
    answer["results"]
        .as_array()
        .unwrap_or_else(|| panic!("results: {answer}"))
        .iter()
        .map(|result| result["outcome"].as_str().expect("an outcome"))
        .collect()
}

/// The one outcome of a copy.
fn only_outcome(report: &Value) -> &Value {
    let items = report["items"]
        .as_array()
        .unwrap_or_else(|| panic!("items: {report}"));
    assert_eq!(items.len(), 1, "{report}");
    &items[0]
}

#[test]
fn an_issue_holding_three_people_reads_back_with_all_three_and_one_set_by_login_reads_back() {
    let store = Assigning::new();
    let (shown, served) = store.ok(&["task", "show", "board:A-1"]);
    assert_eq!(
        shown["items"][0]["item"]["assignees"],
        json!(THREE),
        "{shown}"
    );
    assert!(
        served
            .iter()
            .all(|(document, _)| !document.contains(":user(")),
        "a read looks nobody up"
    );
    // Listed, the same three ride the listing; an unassigned issue reads as empty.
    let (listed, _) = store.ok(&["task", "list", "--source", "board", "--all"]);
    let rows = listed["items"].as_array().expect("rows");
    let row = |id: &str| {
        rows.iter()
            .find(|row| row["id"] == id)
            .unwrap_or_else(|| panic!("{id} listed: {listed}"))["item"]
            .clone()
    };
    assert_eq!(row("board:A-1")["assignees"], json!(THREE));
    assert_eq!(row("board:A-3")["assignees"], json!([]));

    // One person by login, read back from a fresh process.
    let (set, served) = store.ok(&["task", "assignee", "set", "board:A-3", "hubot"]);
    assert_eq!(set["assignees"], json!(["hubot"]), "{set}");
    assert_eq!(writes(&served).len(), 1, "{served:#?}");
    assert_eq!(store.people("board:A-3"), json!(["hubot"]));
    assert_eq!(store.board.assignees_of("A-3"), ["hubot"]);
}

#[test]
fn a_set_replaces_every_person_the_issue_held_and_naming_none_clears_it_in_one_write_each() {
    let store = Assigning::new();
    let (set, served) = store.ok(&["task", "assignee", "set", "board:A-1", "defunkt"]);
    assert_eq!(set["assignees"], json!(["defunkt"]), "{set}");
    let written = writes(&served);
    assert_eq!(written.len(), 1, "exactly one write: {served:#?}");
    // The read the update makes, the lookup, and the write: nothing else.
    assert_eq!(served.len(), 3, "{served:#?}");
    assert!(written[0].0.contains("updateIssue("), "{}", written[0].0);
    assert_eq!(
        written[0].1["input"]["assigneeIds"],
        json!(["U_defunkt"]),
        "{written:#?}"
    );
    assert_eq!(
        lookups(&served),
        1,
        "one lookup of a person not yet resolved"
    );
    // Every one of the three earlier people is gone.
    assert_eq!(store.people("board:A-1"), json!(["defunkt"]));

    let (cleared, served) = store.ok(&["task", "assignee", "set", "board:A-1"]);
    assert_eq!(cleared["assignees"], json!([]), "{cleared}");
    let written = writes(&served);
    assert_eq!(written.len(), 1, "exactly one write: {served:#?}");
    assert_eq!(written[0].1["input"]["assigneeIds"], json!([]));
    assert_eq!(lookups(&served), 0, "unassigning names nobody to look up");
    assert_eq!(served.len(), 2, "the read and the write: {served:#?}");
    assert_eq!(store.people("board:A-1"), json!([]));
}

#[test]
fn a_person_refused_unknown_or_left_off_is_refused_for_that_item_and_the_others_are_written() {
    let store = Assigning::new();
    store.board.refuse_assigning("mojombo");
    store.board.leave_off_when_assigned("pat");
    let (answer, served) = store.update_many(
        &json!([
            {"item": "board:A-3", "title": "Nobody knows", "assignee": "no-such-person-x"},
            {"item": "board:A-4", "title": "Refused", "assignee": "mojombo"},
            {"item": "board:A-5", "title": "Left off", "assignee": "pat"},
            {"item": "board:A-6", "title": "Assigned", "assignee": "hubot"},
        ]),
        4,
    );
    assert_eq!(
        outcomes(&answer),
        ["error", "error", "error", "written"],
        "{answer}"
    );
    let reason = |at: usize| answer["results"][at]["reason"].to_string();
    for (at, person) in [(0, "no-such-person-x"), (1, "mojombo"), (2, "pat")] {
        assert!(reason(at).contains(person), "{person}: {}", reason(at));
    }
    assert!(reason(2).contains("left"), "{}", reason(2));
    // All four people are looked up in one request.
    assert_eq!(lookups(&served), 1, "{served:#?}");
    assert_eq!(store.people("board:A-6"), json!(["hubot"]));
    for id in ["board:A-3", "board:A-4", "board:A-5"] {
        assert_eq!(store.people(id), json!([]), "{id}");
    }
    assert_eq!(store.shown("board:A-6")["title"], "Assigned");
}

#[test]
fn a_guarded_update_many_writes_where_its_people_are_held_and_answers_held_with_no_assignee_write_otherwise()
 {
    let store = Assigning::new();
    let (answer, served) = store.update_many(
        &json!([
            {"item": "board:A-1", "assignee": "defunkt",
             "when_assignees": ["monalisa", "octocat", "hubot"]},
            {"item": "board:A-2", "assignee": "defunkt", "when_assignees": ["octocat"]},
        ]),
        0,
    );
    assert_eq!(outcomes(&answer), ["written", "held"], "{answer}");
    let named: Vec<&Value> = served
        .iter()
        .filter(|(_, variables)| variables.to_string().contains("assigneeIds"))
        .map(|(_, variables)| variables)
        .collect();
    assert_eq!(named.len(), 1, "{served:#?}");
    assert!(
        !named[0].to_string().contains("\"A-2\""),
        "nothing assigns the held item: {}",
        named[0]
    );
    assert_eq!(store.people("board:A-1"), json!(["defunkt"]));
    assert_eq!(store.people("board:A-2"), json!(THREE));
}

#[test]
fn a_guarded_copy_writes_the_sources_assignee_only_while_the_destination_holds_what_it_names() {
    let store = Assigning::new();
    store.plan("p1", "first", "octocat");
    let (report, _) = store.ok(&["task", "copy", "plans:p1", "--to", "board"]);
    let copied = only_outcome(&report)["destination"]
        .as_str()
        .unwrap_or_else(|| panic!("a destination: {report}"))
        .to_owned();
    assert_eq!(store.people(&copied), json!(["octocat"]));
    // Somebody reassigns it on the board.
    store.ok(&["task", "assignee", "set", &copied, "hubot"]);

    // The guard names exactly whom it holds: the source's assignee is written, with the body.
    store.plan("p1", "second", "octocat");
    let (report, met) = store.ok(&[
        "task",
        "copy",
        "plans:p1",
        "--to",
        "board",
        "--when-destination-assignees",
        "hubot",
    ]);
    let item = only_outcome(&report);
    assert_eq!(item["action"], "updated", "{report}");
    assert_eq!(
        item["assignee_condition"],
        json!({"held": ["hubot"], "met": true})
    );
    assert_eq!(store.people(&copied), json!(["octocat"]));
    assert!(assigns(&met), "{met:#?}");

    // Not met: the body is written as before, and no request assigns anyone.
    store.plan("p1", "third", "monalisa");
    let (report, unmet) = store.ok(&[
        "task",
        "copy",
        "plans:p1",
        "--to",
        "board",
        "--when-destination-assignees",
        "hubot",
    ]);
    let item = only_outcome(&report);
    assert_eq!(item["action"], "updated", "{report}");
    assert_eq!(
        item["assignee_condition"],
        json!({"held": ["octocat"], "met": false})
    );
    assert!(!assigns(&unmet), "{unmet:#?}");
    assert_eq!(lookups(&unmet), 0);
    assert_eq!(store.people(&copied), json!(["octocat"]));
    assert!(
        store.shown(&copied)["content"]
            .as_str()
            .is_some_and(|content| content.contains("third")),
        "the body was still written"
    );

    // The assignee rode the write the body already made: the met copy sent exactly the writes
    // the unmet one did, and the one lookup of the person it named.
    assert_eq!(
        writes(&met).len(),
        writes(&unmet).len(),
        "met: {met:#?}\nunmet: {unmet:#?}"
    );
    assert_eq!(writes(&met).len(), 1, "one updateIssue: {met:#?}");
    assert_eq!(lookups(&met), 1);
    assert_eq!(
        (met.len(), unmet.len()),
        (4, 3),
        "met: {met:#?}\nunmet: {unmet:#?}"
    );
}

#[test]
fn an_assignee_costs_a_create_and_a_five_item_update_many_no_write_and_one_lookup() {
    let store = Assigning::new();
    let body = store.sandbox.project().join("body.md");
    std::fs::write(&body, "Assigned at birth.\n").expect("a body");
    let body = body.to_str().expect("a path");
    let create = |title: &str, person: Option<&str>| {
        let mut arguments = vec![
            "task",
            "create",
            "board",
            "--project",
            "P-1",
            "--title",
            title,
            "--body-file",
            body,
        ];
        if let Some(person) = person {
            arguments.extend(["--assignee", person]);
        }
        store.ok(&arguments)
    };
    let (plain, without) = create("Plain create", None);
    let (assigned, with) = create("Assigned create", Some("monalisa"));
    assert_eq!(
        writes(&with).len(),
        writes(&without).len(),
        "with: {with:#?}\nwithout: {without:#?}"
    );
    // `createIssue`, `addProjectV2ItemById`, the `Status` field and `addSubIssue` either way.
    assert_eq!(writes(&with).len(), 4, "{with:#?}");
    assert_eq!(lookups(&with), 1);
    assert_eq!(lookups(&without), 0);
    assert_eq!((with.len(), without.len()), (8, 7));
    let id = |created: &Value| {
        created["task"]["id"]
            .as_str()
            .or_else(|| created["id"].as_str())
            .or_else(|| created["items"][0]["id"].as_str())
            .unwrap_or_else(|| panic!("a created id: {created}"))
            .to_owned()
    };
    assert_eq!(store.people(&id(&assigned)), json!(["monalisa"]));
    assert_eq!(store.people(&id(&plain)), json!([]));

    // Five items each moved to `Doing` with a metadata key, once without an assignee and once
    // with: the same writes, and one lookup however many items name the people.
    let five = |ids: std::ops::RangeInclusive<u32>, people: Option<[&str; 2]>| {
        let updates: Vec<Value> = ids
            .enumerate()
            .map(|(at, n)| {
                let mut update = json!({"item": format!("board:A-{n}"), "status": "in-progress",
                    "metadata": {"myapp.note": format!("note {n}")}});
                if let Some(people) = people {
                    update["assignee"] = json!(people[at % 2]);
                }
                update
            })
            .collect();
        store.update_many(&json!(updates), 0)
    };
    let (answer, without) = five(3..=7, None);
    assert_eq!(outcomes(&answer), ["written"; 5], "{answer}");
    let (answer, with) = five(8..=12, Some(["hubot", "defunkt"]));
    assert_eq!(outcomes(&answer), ["written"; 5], "{answer}");
    assert_eq!(
        writes(&with).len(),
        writes(&without).len(),
        "with: {with:#?}\nwithout: {without:#?}"
    );
    // The five items' field writes in one request, then their five issue updates in one.
    assert_eq!(writes(&with).len(), 2, "{with:#?}");
    assert_eq!(lookups(&with), 1, "{with:#?}");
    assert_eq!(lookups(&without), 0);
    assert_eq!((with.len(), without.len()), (4, 3));
    for n in 8..=12 {
        let person = if (n - 8) % 2 == 0 { "hubot" } else { "defunkt" };
        assert_eq!(
            store.people(&format!("board:A-{n}")),
            json!([person]),
            "A-{n}"
        );
    }
}

#[test]
fn an_assignee_only_change_moves_the_token_and_a_pinned_cached_read_never_serves_the_old_people() {
    let store = Assigning::configured(true, Some(json!({"enabled": true})));
    let (first, _) = store.ok(&["task", "show", "board:A-1"]);
    let before = first["items"][0]["item"]["version"]
        .as_str()
        .unwrap_or_else(|| panic!("a version: {first}"))
        .to_owned();
    assert_eq!(first["items"][0]["item"]["assignees"], json!(THREE));

    let (set, _) = store.ok(&["task", "assignee", "set", "board:A-1", "defunkt"]);
    let (after, _) = store.ok(&["task", "show", "board:A-1"]);
    let token = after["items"][0]["item"]["version"]
        .as_str()
        .unwrap_or_else(|| panic!("a version: {after}"))
        .to_owned();
    assert_ne!(token, before, "an assignee-only change moves the token");
    if let Some(answered) = set["result"]["version"].as_str() {
        assert_eq!(answered, token, "{set}");
    }

    for pin in [&token, &before] {
        let (pinned, _) = store.ok(&["task", "show", "board:A-1", "--pin", pin]);
        assert_eq!(
            pinned["items"][0]["item"]["assignees"],
            json!(["defunkt"]),
            "pinned to {pin}: {pinned}"
        );
    }
    // Pinned to the new token again, it is answered from the cache, with the new people.
    let (pinned, served) = store.ok(&["task", "show", "board:A-1", "--pin", &token]);
    assert_eq!(served, Vec::<(String, Value)>::new(), "{pinned}");
    assert_eq!(pinned["items"][0]["item"]["assignees"], json!(["defunkt"]));
}

#[test]
fn a_board_not_opted_in_refuses_every_assignee_write_unsent_and_never_asks_for_assignees() {
    let store = Assigning::configured(false, None);
    store.plan("p1", "first", "octocat");
    let body = store.sandbox.project().join("body.md");
    std::fs::write(&body, "Nobody.\n").expect("a body");
    let body = body.to_str().expect("a path");
    let batch = json!({"schema_version": 1,
        "updates": [{"item": "board:A-1", "assignee": "hubot"}]})
    .to_string();
    for (arguments, input) in [
        (
            vec![
                "task",
                "create",
                "board",
                "--project",
                "P-1",
                "--title",
                "T",
                "--body-file",
                body,
                "--assignee",
                "hubot",
            ],
            None,
        ),
        (vec!["task", "assignee", "set", "board:A-1", "hubot"], None),
        (
            vec!["task", "update-many", "board", "--input", "-"],
            Some(batch.as_str()),
        ),
        (
            vec![
                "task",
                "copy",
                "plans:p1",
                "--to",
                "board",
                "--when-destination-assignees",
                "hubot",
            ],
            None,
        ),
    ] {
        let (output, served) = store.run(&arguments, input);
        let said = format!("{}{}", stdout(&output), stderr(&output));
        assert_ne!(output.status.code(), Some(0), "{arguments:?}: {said}");
        assert!(said.contains("assignees"), "{arguments:?}: {said}");
        assert_eq!(served, Vec::new(), "{arguments:?} reached the board");
    }
    // Its reads are the reads it always made: nothing asks for anyone's assignees.
    let (shown, _) = store.ok(&["task", "show", "board:A-1"]);
    assert!(
        shown["items"][0]["item"].get("assignees").is_none(),
        "{shown}"
    );
    store.ok(&["task", "list", "--source", "board"]);
    assert!(
        store
            .board
            .served()
            .iter()
            .all(|(document, variables)| !document.contains("assignee")
                && !variables.to_string().contains("assignee")),
        "a board not opted in asked for assignees"
    );
}

#[test]
fn a_draft_reads_as_assigned_to_nobody_and_an_assignee_written_to_it_is_refused_unsent() {
    let store = Assigning::new();
    // A draft holds no comments, which `task show` reports beside the item, exiting 4.
    let (shown, _) = store.exited(&["task", "show", "board:SKETCH"], None, 4);
    assert_eq!(shown["items"][0]["item"]["assignees"], json!([]), "{shown}");
    let (output, served) = store.run(&["task", "assignee", "set", "board:SKETCH", "hubot"], None);
    let said = format!("{}{}", stdout(&output), stderr(&output));
    assert_ne!(output.status.code(), Some(0), "{said}");
    assert!(said.contains("draft"), "{said}");
    assert!(
        writes(&served).is_empty(),
        "nothing is written: {served:#?}"
    );
    assert_eq!(lookups(&served), 0, "nobody is looked up for a draft");
}

#[test]
fn a_failed_assignment_sends_no_second_lookup_for_its_person_within_the_same_call() {
    // Two updates of one task in one call are written one after the other, the second on what
    // the first left: a later write of the same call, whose lookups say whether the call looked
    // its person up once.
    let store = Assigning::new();
    let twice = |id: &str, person: &str| {
        json!([
            {"item": format!("board:{id}"), "assignee": person},
            {"item": format!("board:{id}"), "title": "Asked again", "assignee": person},
        ])
    };
    // A person the board assigns is looked up once: the second write finds them resolved.
    let (answer, kept) = store.update_many(&twice("A-3", "hubot"), 0);
    assert_eq!(outcomes(&answer), ["written", "written"], "{answer}");
    assert_eq!(lookups(&kept), 1, "{kept:#?}");

    // A person the board leaves off fails the first write, which forgets them only once the
    // call ends: the second write of the same call is sent with the id the call's one lookup
    // found, and fails the same way.
    store.board.leave_off_when_assigned("pat");
    let (answer, failed) = store.update_many(&twice("A-4", "pat"), 4);
    assert_eq!(outcomes(&answer), ["error", "error"], "{answer}");
    assert_eq!(lookups(&failed), 1, "{failed:#?}");
    assert_eq!(store.people("board:A-4"), json!([]));
}

#[test]
fn a_write_answered_without_the_people_it_left_assigned_is_refused_as_malformed() {
    let store = Assigning::new();
    store.board.answer_next_assignment_without_assignees();
    let (output, served) = store.run(&["task", "assignee", "set", "board:A-3", "hubot"], None);
    let said = format!("{}{}", stdout(&output), stderr(&output));
    assert_ne!(output.status.code(), Some(0), "{said}");
    assert!(said.contains("without the people"), "{said}");
    assert_eq!(writes(&served).len(), 1, "{served:#?}");
}

#[test]
fn a_replacement_github_answers_keeping_a_person_it_did_not_name_is_refused_naming_them() {
    let store = Assigning::new();
    store.board.keep_when_replaced("octocat");
    let (output, _) = store.run(&["task", "assignee", "set", "board:A-1", "defunkt"], None);
    let said = format!("{}{}", stdout(&output), stderr(&output));
    assert_ne!(output.status.code(), Some(0), "{said}");
    assert!(said.contains("kept octocat"), "{said}");
    assert_eq!(store.board.assignees_of("A-1"), ["defunkt", "octocat"]);
}

#[test]
fn a_create_github_answers_assigned_otherwise_takes_the_new_issue_back_on_either_create_path() {
    let store = Assigning::new();
    store.board.leave_off_when_assigned("pat");
    let deletes = |served: &[(String, Value)]| {
        writes(served)
            .iter()
            .filter(|(document, _)| document.contains("deleteIssue"))
            .count()
    };
    // `task create`: the issue is created on no board and filed after.
    let body = store.sandbox.project().join("body.md");
    std::fs::write(&body, "Left off.\n").expect("a body");
    let (output, served) = store.run(
        &[
            "task",
            "create",
            "board",
            "--project",
            "P-1",
            "--title",
            "Created for pat",
            "--body-file",
            body.to_str().expect("a path"),
            "--assignee",
            "pat",
        ],
        None,
    );
    let said = format!("{}{}", stdout(&output), stderr(&output));
    assert_ne!(output.status.code(), Some(0), "{said}");
    assert!(said.contains("pat"), "{said}");
    assert_eq!(deletes(&served), 1, "{served:#?}");
    assert!(!store.board.holds_titled("Created for pat"));

    // A copy: the issue is created filed on the board by its `createIssue`.
    store.plan("p9", "copied", "pat");
    let (output, served) = store.run(&["task", "copy", "plans:p9", "--to", "board"], None);
    let said = format!("{}{}", stdout(&output), stderr(&output));
    assert_ne!(output.status.code(), Some(0), "{said}");
    assert!(said.contains("pat"), "{said}");
    assert!(
        served
            .iter()
            .any(|(document, _)| document.contains("createIssue(input:$input)")),
        "the copy created its issue alone: {served:#?}"
    );
    assert_eq!(deletes(&served), 1, "{served:#?}");
    assert!(!store.board.holds_titled("Plan p9"));
}

#[test]
fn twenty_two_unseen_people_are_resolved_in_one_lookup_that_answers_each_of_them() {
    // Twenty people GitHub does not know, then two it knows: one lookup asks for all twenty-two
    // and answers the last two as well as the first.
    let store = Assigning::new();
    let mut updates: Vec<Value> = (0..20)
        .map(
            |n| json!({"item": format!("board:A-{}", n % 12 + 1), "assignee": format!("ghost{n}")}),
        )
        .collect();
    updates.push(json!({"item": "board:A-1", "assignee": "octocat"}));
    updates.push(json!({"item": "board:A-2", "assignee": "hubot"}));
    let (answer, served) = store.update_many(&Value::Array(updates), 4);
    assert_eq!(lookups(&served), 1, "{served:#?}");
    let asked = served
        .iter()
        .find(|(document, _)| document.contains(":user(login:"))
        .map(|(_, variables)| variables.as_object().map_or(0, serde_json::Map::len))
        .unwrap_or_default();
    assert_eq!(asked, 22, "the one lookup names every one of them");
    let outcomes = outcomes(&answer);
    assert_eq!(&outcomes[..20], ["error"; 20], "{answer}");
    assert_eq!(&outcomes[20..], ["written", "written"], "{answer}");
    let reason = answer["results"][0]["reason"].as_str().expect("a reason");
    assert!(reason.contains("ghost0"), "{reason}");
    assert_eq!(store.people("board:A-1"), json!(["octocat"]));
    assert_eq!(store.people("board:A-2"), json!(["hubot"]));
}

#[test]
fn a_person_a_failed_write_named_is_looked_up_again_by_the_next_call_and_not_before() {
    // One engine is one process; `end_command` is where one of its calls ends and the next
    // begins.
    let store = Assigning::new();
    store.board.leave_off_when_assigned("pat");
    let environment = onetaskgraph_core::Environment::from_pairs([
        (
            "XDG_CONFIG_HOME",
            store.sandbox.config_home().to_string_lossy().into_owned(),
        ),
        (
            "XDG_CACHE_HOME",
            store.cache_home.to_string_lossy().into_owned(),
        ),
    ]);
    let loaded = onetaskgraph_core::config::load(
        &store.sandbox.project(),
        &environment,
        &onetaskgraph_core::config::Layer::default(),
    )
    .expect("the configuration loads");
    let engine = onetaskgraph_core::Engine::build(&loaded.config, &loaded.secrets);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    let assign = |id: &str, person: &str| {
        let mark = store.board.served_count();
        let id: onetaskgraph_core::GlobalId = id.parse().expect("an id");
        let answer = runtime.block_on(engine.set_task_assignee(&id, Some(person)));
        (answer, lookups(&store.board.served_since(mark)))
    };
    let (failed, looked) = assign("board:A-4", "pat");
    assert!(failed.is_err(), "pat is left off: {failed:?}");
    assert_eq!(looked, 1, "the call resolves pat");
    let (failed, looked) = assign("board:A-5", "pat");
    assert!(failed.is_err(), "{failed:?}");
    assert_eq!(looked, 0, "the same call does not look pat up again");
    runtime
        .block_on(engine.end_command())
        .expect("the call ends");
    let (failed, looked) = assign("board:A-6", "pat");
    assert!(failed.is_err(), "{failed:?}");
    assert_eq!(looked, 1, "the next call looks pat up afresh");
    // A person whose write landed is kept across calls.
    let (written, looked) = assign("board:A-7", "hubot");
    assert!(written.is_ok(), "{written:?}");
    assert_eq!(looked, 1);
    runtime
        .block_on(engine.end_command())
        .expect("the call ends");
    let (written, looked) = assign("board:A-8", "hubot");
    assert!(written.is_ok(), "{written:?}");
    assert_eq!(looked, 0, "hubot stays resolved");
}

/// Apply `change` to every `assignees` connection `data` holds, wherever it is.
fn every_connection(data: &mut Value, change: &dyn Fn(&mut Value)) {
    match data {
        Value::Object(members) => {
            for (key, member) in members.iter_mut() {
                if key == "assignees" {
                    change(member);
                } else {
                    every_connection(member, change);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                every_connection(item, change);
            }
        }
        _ => {}
    }
}

/// The failure kind a command that failed printed: its failure document's, or the first error
/// a read reported its source failed with.
fn failure_kind(output: &Output) -> String {
    let printed: Value = serde_json::from_str(&stdout(output)).unwrap_or(Value::Null);
    printed["failure"]["kind"]
        .as_str()
        .or_else(|| printed["errors"][0]["error"]["kind"].as_str())
        .unwrap_or_default()
        .to_owned()
}

#[test]
fn a_read_answered_with_a_malformed_assignee_connection_is_refused_never_read_as_nobody() {
    let store = Assigning::new();
    for (what, bend) in [
        (
            "no connection",
            Box::new(|data: &mut Value| {
                if let Some(node) = data.get_mut("node").and_then(Value::as_object_mut) {
                    node.remove("assignees");
                }
            }) as Box<dyn FnMut(&mut Value) + Send>,
        ),
        (
            "nodes not a list",
            Box::new(|data: &mut Value| {
                every_connection(data, &|connection| connection["nodes"] = json!("octocat"));
            }),
        ),
        (
            "a person without a login",
            Box::new(|data: &mut Value| {
                every_connection(data, &|connection| {
                    connection["nodes"] = json!([{"id": "U_octocat"}]);
                });
            }),
        ),
    ] {
        store.board.bend_next_answer("node(id:$id)", bend);
        let (output, _) = store.run(&["task", "show", &format!("board:{TWIN}")], None);
        assert_ne!(output.status.code(), Some(0), "{what}: {}", stdout(&output));
        assert_eq!(
            failure_kind(&output),
            "malformed",
            "{what}: {}",
            stdout(&output)
        );
    }
    // Answered as it should be, the same read holds every person.
    assert_eq!(store.people(&format!("board:{TWIN}")), json!(THREE));
}

#[test]
fn a_lookup_answered_without_an_id_or_without_its_not_found_error_is_refused_as_malformed() {
    let store = Assigning::new();
    for (what, bend) in [
        (
            "a user without an id",
            Box::new(|data: &mut Value| data["u0"] = json!({"login": "defunkt"}))
                as Box<dyn FnMut(&mut Value) + Send>,
        ),
        (
            "another user than asked for",
            Box::new(|data: &mut Value| data["u0"] = json!({"id": "U_hubot", "login": "hubot"})),
        ),
        (
            "no answer and no error",
            Box::new(|data: &mut Value| {
                if let Some(members) = data.as_object_mut() {
                    members.remove("u0");
                }
            }),
        ),
    ] {
        store.board.bend_next_answer(":user(login:", bend);
        let (output, served) =
            store.run(&["task", "assignee", "set", "board:A-5", "defunkt"], None);
        assert_ne!(output.status.code(), Some(0), "{what}: {}", stdout(&output));
        assert_eq!(
            failure_kind(&output),
            "malformed",
            "{what}: {}",
            stdout(&output)
        );
        assert!(
            writes(&served).is_empty(),
            "{what}: nothing is written: {served:#?}"
        );
    }
    assert_eq!(store.people("board:A-5"), json!([]));
}

#[test]
fn a_write_answered_with_a_person_missing_an_id_or_login_is_refused_and_the_call_looks_nobody_up_twice()
 {
    // Updates of one task in one call are written one after the other, each on what the one
    // before left: the first lands but is answered malformed, the second unassigns, and the
    // third assigns the same person again — a later write of the same call, which the call's one
    // lookup still answers.
    let store = Assigning::new();
    store
        .board
        .bend_next_answer("updateIssue(", |data: &mut Value| {
            every_connection(data, &|connection| {
                connection["nodes"] = json!([{"id": "U_hubot"}]);
            });
        });
    let (answer, served) = store.update_many(
        &json!([
            {"item": "board:A-6", "assignee": "hubot"},
            {"item": "board:A-6", "assignee": null},
            {"item": "board:A-6", "assignee": "hubot"},
        ]),
        4,
    );
    assert_eq!(
        outcomes(&answer),
        ["error", "written", "written"],
        "{answer}"
    );
    let reason = answer["results"][0]["reason"].as_str().expect("a reason");
    assert!(reason.contains("no id or no login"), "{reason}");
    assert_eq!(
        lookups(&served),
        1,
        "the call's one lookup answers its later writes too: {served:#?}"
    );
    assert_eq!(store.people("board:A-6"), json!(["hubot"]));
}

/// Set the board's `config.assignees` to `opted`, keeping every other setting of the store.
fn opting(store: &Assigning, opted: bool) {
    let path = store.sandbox.project().join("onetaskgraph.yaml");
    let mut document: Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("the project document"))
            .expect("JSON");
    document["sources"]["board"]["config"]["assignees"] = json!(opted);
    store.sandbox.project_document(&document.to_string());
}

#[test]
fn an_entry_cached_before_the_board_opted_in_is_read_again_rather_than_served_without_people() {
    // The board's cache is on and it has not opted in: a pinned read fills an entry, which the
    // same pin then serves with no request.
    let store = Assigning::configured(false, Some(json!({"enabled": true})));
    let (first, _) = store.ok(&["task", "show", &format!("board:{SEEDED}")]);
    let token = first["items"][0]["item"]["version"]
        .as_str()
        .unwrap_or_else(|| panic!("a version: {first}"))
        .to_owned();
    assert!(
        first["items"][0]["item"].get("assignees").is_none(),
        "{first}"
    );
    let (cached, served) = store.ok(&["task", "show", &format!("board:{SEEDED}"), "--pin", &token]);
    assert_eq!(
        served,
        Vec::<(String, Value)>::new(),
        "served from the cache: {cached}"
    );

    // Opted in, the same pin is read from GitHub — the entry holds nobody's people — and
    // answers every person the issue holds.
    opting(&store, true);
    let (opted, served) = store.ok(&["task", "show", &format!("board:{SEEDED}"), "--pin", &token]);
    assert!(!served.is_empty(), "the entry was not served: {opted}");
    assert_eq!(
        opted["items"][0]["item"]["assignees"],
        json!(THREE),
        "{opted}"
    );

    // Opted out again, the entry the opted-in read wrote is not served either, and no answer
    // carries an `assignees` key.
    let (again, _) = store.ok(&["task", "show", &format!("board:{SEEDED}")]);
    let token = again["items"][0]["item"]["version"]
        .as_str()
        .unwrap_or_else(|| panic!("a version: {again}"))
        .to_owned();
    opting(&store, false);
    let (plain, served) = store.ok(&["task", "show", &format!("board:{SEEDED}"), "--pin", &token]);
    assert!(!served.is_empty(), "the entry was not served: {plain}");
    assert!(
        plain["items"][0]["item"].get("assignees").is_none(),
        "{plain}"
    );
}

/// The destination each copied item landed at, by the item it was copied from.
fn landed(report: &Value, source: &str) -> String {
    report["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|item| item["source"] == source)
        .and_then(|item| item["destination"].as_str())
        .unwrap_or_else(|| panic!("{source} landed nowhere: {report}"))
        .to_owned()
}

#[test]
fn tasks_created_together_resolve_every_person_in_one_lookup_whatever_case_names_them() {
    let store = Assigning::new();
    store.plan("p1", "one", "octocat");
    store.plan("p2", "two", "HuBot");
    store.plan("p3", "three", "hubot");
    let (report, served) = store.ok(&[
        "task", "copy", "plans:p1", "plans:p2", "plans:p3", "--to", "board",
    ]);
    // One lookup, naming each person once however the plan spelled them.
    let lookups: Vec<&Value> = served
        .iter()
        .filter(|(document, _)| document.contains(":user(login:"))
        .map(|(_, variables)| variables)
        .collect();
    assert_eq!(lookups.len(), 1, "{served:#?}");
    assert_eq!(
        lookups[0].as_object().map_or(0, serde_json::Map::len),
        2,
        "{}",
        lookups[0]
    );
    // One document creates all three, each with its person's id.
    let creates: Vec<&Value> = served
        .iter()
        .filter(|(document, _)| document.contains("createIssue(input:$create2)"))
        .map(|(_, variables)| variables)
        .collect();
    assert_eq!(
        creates.len(),
        1,
        "the three are created together: {served:#?}"
    );
    for (at, id) in [(0, "U_octocat"), (1, "U_hubot"), (2, "U_hubot")] {
        assert_eq!(
            creates[0][format!("create{at}")]["assigneeIds"],
            json!([id]),
            "{}",
            creates[0]
        );
    }
    for (plan, person) in [
        ("plans:p1", "octocat"),
        ("plans:p2", "hubot"),
        ("plans:p3", "hubot"),
    ] {
        assert_eq!(
            store.people(&landed(&report, plan)),
            json!([person]),
            "{plan}"
        );
    }
    // Asked for the person it holds, in another case, an issue is written nothing.
    let hubot = landed(&report, "plans:p3");
    let (set, served) = store.ok(&["task", "assignee", "set", &hubot, "HUBOT"]);
    assert_eq!(set["assignees"], json!(["hubot"]), "{set}");
    assert!(writes(&served).is_empty(), "{served:#?}");
}

#[test]
fn tasks_created_together_are_all_taken_back_when_github_leaves_one_person_off() {
    let store = Assigning::new();
    store.board.leave_off_when_assigned("pat");
    store.plan("p1", "one", "octocat");
    store.plan("p2", "two", "pat");
    let (output, served) = store.run(
        &["task", "copy", "plans:p1", "plans:p2", "--to", "board"],
        None,
    );
    let said = format!("{}{}", stdout(&output), stderr(&output));
    assert_ne!(output.status.code(), Some(0), "{said}");
    assert!(said.contains("pat"), "{said}");
    assert_eq!(
        served
            .iter()
            .filter(|(document, _)| document.contains(":user(login:"))
            .count(),
        1,
        "{served:#?}"
    );
    assert!(
        served
            .iter()
            .any(|(document, _)| document.contains("createIssue(input:$create1)")),
        "the two were created together: {served:#?}"
    );
    // Neither issue is left on the board: the copy took back both it created.
    assert!(!store.board.holds_titled("Plan p1"), "{said}");
    assert!(!store.board.holds_titled("Plan p2"), "{said}");
}

#[test]
fn a_create_github_assigns_otherwise_and_will_not_take_back_answers_the_assignment_and_stays() {
    // On either create path: GitHub leaves pat off the issue it creates, and then refuses the
    // delete that would take it back. What is answered is why the write stopped — pat left off —
    // and the issue is still there, which a person then has to remove.
    let store = Assigning::new();
    store.board.leave_off_when_assigned("pat");
    let deleted = |served: &[(String, Value)]| {
        served
            .iter()
            .any(|(document, _)| document.contains("deleteIssue(input:$"))
    };

    // `task create`: the issue is created on no board and filed after.
    store.board.refuse_once("deleteIssue(input:$");
    let body = store.sandbox.project().join("kept.md");
    std::fs::write(&body, "Kept.\n").expect("a body");
    let (output, served) = store.run(
        &[
            "task",
            "create",
            "board",
            "--project",
            "P-1",
            "--title",
            "Kept for pat",
            "--body-file",
            body.to_str().expect("a path"),
            "--assignee",
            "pat",
        ],
        None,
    );
    let said = format!("{}{}", stdout(&output), stderr(&output));
    assert_ne!(output.status.code(), Some(0), "{said}");
    assert!(
        said.contains("left pat off"),
        "the assignment's refusal: {said}"
    );
    assert!(deleted(&served), "the take-back was asked for: {served:#?}");
    assert!(store.board.holds_titled("Kept for pat"), "{said}");

    // A copy: the issue is created filed on the board by its `createIssue`.
    store.board.refuse_once("deleteIssue(input:$");
    store.plan("p8", "kept", "pat");
    let (output, served) = store.run(&["task", "copy", "plans:p8", "--to", "board"], None);
    let said = format!("{}{}", stdout(&output), stderr(&output));
    assert_ne!(output.status.code(), Some(0), "{said}");
    assert!(
        said.contains("left pat off"),
        "the assignment's refusal: {said}"
    );
    assert!(deleted(&served), "the take-back was asked for: {served:#?}");
    assert!(store.board.holds_titled("Plan p8"), "{said}");
}

#[test]
fn a_batch_whose_shared_lookup_fails_looks_each_person_up_again_and_writes_every_update() {
    // The lookup a batch makes for every person it names is answered malformed, once: each
    // update then looks its own person up and is written — through the batched path for two
    // updates, and on its own for one.
    let store = Assigning::new();
    let broken = |data: &mut Value| data["u0"] = json!({"login": "octocat"});
    store.board.bend_next_answer(":user(login:", broken);
    let (answer, served) = store.update_many(
        &json!([
            {"item": "board:A-7", "assignee": "octocat", "title": "Seven"},
            {"item": "board:A-8", "assignee": "hubot", "title": "Eight"},
        ]),
        0,
    );
    assert_eq!(outcomes(&answer), ["written", "written"], "{answer}");
    assert!(
        lookups(&served) > 1,
        "each person was looked up again: {served:#?}"
    );
    assert_eq!(store.people("board:A-7"), json!(["octocat"]));
    assert_eq!(store.people("board:A-8"), json!(["hubot"]));

    store.board.bend_next_answer(":user(login:", broken);
    let (answer, served) =
        store.update_many(&json!([{"item": "board:A-9", "assignee": "octocat"}]), 0);
    assert_eq!(outcomes(&answer), ["written"], "{answer}");
    assert_eq!(
        lookups(&served),
        2,
        "the shared lookup, then its own: {served:#?}"
    );
    assert_eq!(store.people("board:A-9"), json!(["octocat"]));
}
