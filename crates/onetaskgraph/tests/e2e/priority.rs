//! A task's priority, and a task's content written on its own, driven the way a user drives
//! them.
//!
//! The shared journeys run against every row of [`crate::fixtures::ROWS`]: each row's backend
//! holds the shared dataset's priorities in its own representation — a front-matter key, a
//! Linear number, a board option — and every one of them has to report the same answer, filter
//! to the same tasks, and take a narrow write the same way. The journeys after them drive what
//! only one backend has: a GitHub board's `Priority` field and the refusals it owes, and a
//! stdio plugin written before priorities, which is never handed one.

use std::path::Path;
use std::process::Output;

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::document_store::store_at;
use crate::fixtures::{
    GitHubBoardFields, ROWS, Row, SOURCE, document, empty_folder, github_projects_with_board,
    qualified,
};

/// The priority the shared dataset gives each of its tasks.
const HELD: [(&str, &str); 4] = [
    ("T-1", "high"),
    ("T-2", "none"),
    ("T-3", "urgent"),
    ("T-4", "low"),
];

/// The rows whose store outlives one invocation, so a write one command makes is what the
/// next command reads. An `in-memory` source — in process or a pipe away — holds its work in
/// the process that answered, so it is driven for its answer alone.
const PERSISTENT: [&str; 3] = ["local-md", "linear", "github-projects"];

fn run(sandbox: &Sandbox, arguments: &[&str]) -> Output {
    sandbox
        .command()
        .args(arguments)
        .assert()
        .get_output()
        .clone()
}

/// A run that had to exit `code`, quoting what it said when it did not.
fn exits(who: &str, sandbox: &Sandbox, arguments: &[&str], code: i32) -> Output {
    let output = run(sandbox, arguments);
    assert_eq!(
        output.status.code(),
        Some(code),
        "{who}: `onetaskgraph {}` exited {:?}\nstdout:\n{}\nstderr:\n{}",
        arguments.join(" "),
        output.status.code(),
        stdout(&output),
        stderr(&output)
    );
    output
}

/// A run that had to succeed, parsed as the JSON document it wrote.
fn answered(who: &str, sandbox: &Sandbox, arguments: &[&str]) -> Value {
    let output = exits(who, sandbox, arguments, 0);
    serde_json::from_str(&stdout(&output)).unwrap_or_else(|error| {
        panic!(
            "{who}: `onetaskgraph {}` wrote no JSON ({error}):\n{}",
            arguments.join(" "),
            stdout(&output)
        )
    })
}

fn host(row: &Row) -> Sandbox {
    let sandbox = Sandbox::new();
    sandbox.project_document(&row.document(&sandbox));
    sandbox
}

/// The one task a `task show --json` answered with.
fn shown(who: &str, sandbox: &Sandbox, id: &str) -> Value {
    let detail = answered(who, sandbox, &["--json", "task", "show", id]);
    detail["items"][0]["item"].clone()
}

/// Each listed task's qualified id beside its priority, in the order listed.
fn priorities(listed: &Value) -> Vec<(String, String)> {
    listed["items"]
        .as_array()
        .expect("a page of tasks")
        .iter()
        .map(|task| {
            (
                task["id"].as_str().expect("a qualified id").to_owned(),
                task["item"]["priority"]
                    .as_str()
                    .expect("every task carries its priority")
                    .to_owned(),
            )
        })
        .collect()
}

fn ours(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(id, priority)| (qualified(SOURCE, id), (*priority).to_owned()))
        .collect()
}

/// The ids a text listing names, in order.
fn listed(rendered: &str) -> Vec<String> {
    rendered
        .lines()
        .take_while(|line| !line.trim().is_empty())
        .filter_map(|line| line.split_whitespace().next())
        .map(str::to_owned)
        .collect()
}

/// `task` with the members a narrow write may not move, each named, for a member-by-member
/// comparison: everything but `priority` and `content` — the two members these verbs write —
/// and `updated_at`, which a backend moves on any write of its own accord.
fn kept(task: &Value) -> Vec<(&'static str, Value)> {
    [
        "id",
        "key",
        "title",
        "status",
        "labels",
        "project",
        "url",
        "location",
        "metadata",
        "repositories",
        "delivers",
        "delivered_by",
    ]
    .into_iter()
    .map(|member| (member, task.get(member).cloned().unwrap_or(Value::Null)))
    .collect()
}

#[test]
fn every_row_reports_each_tasks_priority_in_both_renderings() {
    for row in ROWS.iter().filter(|row| row.fixture.complete_dataset) {
        let sandbox = host(row);
        let listed_tasks = answered(row.name, &sandbox, &["--json", "task", "list"]);
        assert_eq!(priorities(&listed_tasks), ours(&HELD), "{}", row.name);

        // `none` is written out rather than left absent, so a reader never has to tell the
        // two apart.
        assert_eq!(
            shown(row.name, &sandbox, &qualified(SOURCE, "T-2"))["priority"],
            "none",
            "{}",
            row.name
        );
        let human = exits(
            row.name,
            &sandbox,
            &["task", "show", &qualified(SOURCE, "T-1")],
            0,
        );
        assert!(
            stdout(&human)
                .lines()
                .any(|line| line.starts_with("priority:") && line.ends_with(" high")),
            "{}: the human rendering names the priority:\n{}",
            row.name,
            stdout(&human)
        );
        let unranked = exits(
            row.name,
            &sandbox,
            &["task", "show", &qualified(SOURCE, "T-2")],
            0,
        );
        assert!(
            !stdout(&unranked).contains("priority:"),
            "{}: a task with none set says nothing of it:\n{}",
            row.name,
            stdout(&unranked)
        );
    }
}

#[test]
fn every_row_filters_by_priority_exactly_whether_it_applies_the_filter_or_the_engine_does() {
    for row in ROWS.iter().filter(|row| row.fixture.complete_dataset) {
        let sandbox = host(row);
        let native = row.declared().filter_by_priority.is_native();
        let several = exits(
            row.name,
            &sandbox,
            &[
                "task",
                "list",
                "--priority",
                "urgent",
                "--priority",
                "high",
                "--explain",
            ],
            0,
        );
        let rendered = stdout(&several);
        assert_eq!(
            listed(&rendered),
            [qualified(SOURCE, "T-1"), qualified(SOURCE, "T-3")],
            "{}",
            row.name
        );
        let outcome = if native {
            "pushed down"
        } else {
            "applied locally"
        };
        assert!(
            rendered
                .lines()
                .map(str::trim)
                .any(|line| line.starts_with(&format!("{outcome}:")) && line.contains("priority")),
            "{}: the plan says the priority filter was {outcome}:\n{rendered}",
            row.name
        );

        let none = answered(
            row.name,
            &sandbox,
            &["--json", "task", "list", "--priority", "none"],
        );
        assert_eq!(priorities(&none), ours(&[("T-2", "none")]), "{}", row.name);
        let plan = &none["plan"]["per_source"][0];
        assert_eq!(
            plan[if native {
                "pushed_down"
            } else {
                "applied_locally"
            }],
            json!(["priority"]),
            "{}",
            row.name
        );

        // Beside other filters, every one narrows: of the tasks at urgent or high, both carry
        // `bug` and are `todo`, so both stay — and T-2 and T-4, which are at neither, do not.
        let both = answered(
            row.name,
            &sandbox,
            &[
                "--json",
                "task",
                "list",
                "--priority",
                "urgent",
                "--priority",
                "high",
                "--label",
                "bug",
                "--status",
                "todo",
            ],
        );
        assert_eq!(
            priorities(&both),
            ours(&[("T-1", "high"), ("T-3", "urgent")]),
            "{}",
            row.name
        );
    }
}

#[test]
fn a_priority_set_on_its_own_answers_what_the_source_holds_and_moves_nothing_else() {
    for row in ROWS.iter().filter(|row| row.fixture.complete_dataset) {
        let sandbox = host(row);
        let id = qualified(SOURCE, "T-1");
        let before = shown(row.name, &sandbox, &id);

        let set = answered(
            row.name,
            &sandbox,
            &["--json", "task", "priority", "set", &id, "urgent"],
        );
        assert_eq!(set, json!({"id": id, "priority": "urgent"}), "{}", row.name);
        let human = exits(
            row.name,
            &sandbox,
            &["task", "priority", "set", &id, "medium"],
            0,
        );
        assert_eq!(
            stdout(&human),
            format!("id:        {id}\npriority:  medium\n"),
            "{}",
            row.name
        );
        if !PERSISTENT.contains(&row.plugin) {
            continue;
        }
        let after = shown(row.name, &sandbox, &id);
        assert_eq!(after["priority"], "medium", "{}", row.name);
        assert_eq!(after["content"], before["content"], "{}", row.name);
        assert_eq!(kept(&after), kept(&before), "{}", row.name);

        // `none` clears it, and the next read says so.
        let cleared = answered(
            row.name,
            &sandbox,
            &["--json", "task", "priority", "set", &id, "none"],
        );
        assert_eq!(cleared["priority"], "none", "{}", row.name);
        assert_eq!(
            shown(row.name, &sandbox, &id)["priority"],
            "none",
            "{}",
            row.name
        );
        let unranked = answered(
            row.name,
            &sandbox,
            &["--json", "task", "list", "--priority", "none"],
        );
        assert_eq!(
            priorities(&unranked),
            ours(&[("T-1", "none"), ("T-2", "none")]),
            "{}",
            row.name
        );
    }
}

#[test]
fn a_content_set_replaces_the_body_with_the_files_bytes_and_moves_no_other_member() {
    let body = "Why this matters now.\n\nThe second paragraph — with a dash.";
    for row in ROWS.iter().filter(|row| row.fixture.complete_dataset) {
        let sandbox = host(row);
        let file = sandbox.subdirectory("content").join("body.md");
        std::fs::write(&file, body).expect("the content file");
        let id = qualified(SOURCE, "T-1");
        let before = shown(row.name, &sandbox, &id);
        let dependencies_before = answered(
            row.name,
            &sandbox,
            &["--json", "task", "deps", &id, "--direction", "depends-on"],
        )["items"]
            .clone();

        let set = answered(
            row.name,
            &sandbox,
            &[
                "--json",
                "task",
                "content",
                "set",
                &id,
                "--file",
                file.to_str().expect("a UTF-8 path"),
            ],
        );
        assert_eq!(set, json!({"id": id}), "{}", row.name);
        let human = exits(
            row.name,
            &sandbox,
            &[
                "task",
                "content",
                "set",
                &id,
                "--file",
                file.to_str().expect("a UTF-8 path"),
            ],
            0,
        );
        assert_eq!(stdout(&human), format!("id:  {id}\n"), "{}", row.name);
        if !PERSISTENT.contains(&row.plugin) {
            continue;
        }
        let after = shown(row.name, &sandbox, &id);
        assert_eq!(after["content"], body, "{}", row.name);
        // Member by member, so a failure names what moved.
        for ((member, was), (_, now)) in kept(&before).into_iter().zip(kept(&after)) {
            assert_eq!(now, was, "{}: {member} moved", row.name);
        }
        assert_eq!(after["priority"], before["priority"], "{}", row.name);
        assert_eq!(
            answered(
                row.name,
                &sandbox,
                &["--json", "task", "deps", &id, "--direction", "depends-on"],
            )["items"],
            dependencies_before,
            "{}: the dependencies moved",
            row.name
        );

        // Whitespace at either end is content like any other: every store reads back exactly
        // the bytes the file held — trailing spaces, a run of trailing newlines, and an indented
        // first line included — whether or not it keeps a metadata block beside them.
        for exact in [
            "Ends with a newline.\n",
            "  Indented first line.\nTrailing spaces   \n\n\n",
        ] {
            std::fs::write(&file, exact).expect("the content file");
            answered(
                row.name,
                &sandbox,
                &[
                    "--json",
                    "task",
                    "content",
                    "set",
                    &id,
                    "--file",
                    file.to_str().expect("a UTF-8 path"),
                ],
            );
            let after = shown(row.name, &sandbox, &id);
            assert_eq!(after["content"], exact, "{}: {exact:?}", row.name);
            assert_eq!(kept(&after), kept(&before), "{}", row.name);
        }
    }
}

#[test]
fn a_narrow_write_to_a_source_that_cannot_take_it_is_refused_by_name() {
    let sandbox = Sandbox::new();
    let task = json!({"id": "T-1", "title": "Alpha", "content": "body",
                      "status": {"category": "todo", "name": "Todo"}, "labels": []});
    sandbox.project_document(&document(&json!({
        "frozen": {"plugin": "in-memory",
                   "config": {"capabilities": {"writes": "unsupported"}, "tasks": [task]}},
        "unranked": {"plugin": "in-memory",
                     "config": {"capabilities": {"priority": "unsupported"}, "tasks": [task]}},
    })));
    let file = sandbox.subdirectory("content").join("body.md");
    std::fs::write(&file, "new").expect("the content file");
    let path = file.to_str().expect("a UTF-8 path");

    let refused = exits(
        "frozen",
        &sandbox,
        &["task", "content", "set", "frozen:T-1", "--file", path],
        1,
    );
    assert!(
        stderr(&refused).contains("source frozen cannot write a task's content")
            && stderr(&refused).contains("next:"),
        "{}",
        stderr(&refused)
    );
    let refused = exits(
        "frozen",
        &sandbox,
        &["task", "priority", "set", "frozen:T-1", "high"],
        1,
    );
    assert!(
        stderr(&refused).contains("source frozen cannot write a priority"),
        "{}",
        stderr(&refused)
    );

    // Any value, `none` included: a source holding no priority has no field to write.
    for value in ["high", "none"] {
        let refused = answered_failure(
            &sandbox,
            &["--json", "task", "priority", "set", "unranked:T-1", value],
        );
        assert_eq!(refused["failure"]["kind"], "no-priority");
        assert_eq!(refused["failure"]["source"], "unranked");
        let message = refused["failure"]["message"].as_str().expect("a message");
        assert!(
            message.contains("source unranked cannot hold the field priority")
                && message.contains("next:"),
            "{message}"
        );
    }

    // A file that is not UTF-8 text is refused rather than repaired, before any source is
    // asked.
    let latin = sandbox.subdirectory("content").join("latin1.md");
    std::fs::write(&latin, b"caf\xe9").expect("the content file");
    let refused = exits(
        "unranked",
        &sandbox,
        &[
            "task",
            "content",
            "set",
            "unranked:T-1",
            "--file",
            latin.to_str().expect("a UTF-8 path"),
        ],
        1,
    );
    assert!(
        stderr(&refused).contains("is not UTF-8 text") && stderr(&refused).contains("next:"),
        "{}",
        stderr(&refused)
    );

    // A file that cannot be read is refused before any source is asked.
    let unreadable = exits(
        "unreadable",
        &sandbox,
        &[
            "task",
            "content",
            "set",
            "unranked:T-1",
            "--file",
            sandbox
                .subdirectory("content")
                .join("absent.md")
                .to_str()
                .expect("a UTF-8 path"),
        ],
        1,
    );
    assert!(
        stderr(&unreadable).contains("--file") && stderr(&unreadable).contains("next:"),
        "{}",
        stderr(&unreadable)
    );
}

/// A run that had to fail, parsed as the failure document it wrote under machine output.
fn answered_failure(sandbox: &Sandbox, arguments: &[&str]) -> Value {
    let output = exits("refusal", sandbox, arguments, 1);
    serde_json::from_str(&stdout(&output)).expect("a failure document")
}

/// The text of the one task file a folder of Markdown holds.
fn only_task_file(root: &Path) -> String {
    let files: Vec<_> = std::fs::read_dir(root.join("tasks"))
        .expect("the folder's tasks")
        .map(|entry| entry.expect("an entry").path())
        .collect();
    assert_eq!(files.len(), 1, "{files:?}");
    std::fs::read_to_string(&files[0]).expect("the copied file")
}

/// One Markdown task file, in a folder of its own.
fn markdown_task(root: &Path, id: &str, front: &str, body: &str) {
    let path = root.join("tasks").join(format!("{id}.md"));
    std::fs::create_dir_all(path.parent().expect("a folder")).expect("the folder");
    std::fs::write(path, format!("---\n{front}\n---\n{body}\n")).expect("the task file");
}

#[test]
fn a_copy_carries_priority_on_create_and_on_update_including_back_to_none() {
    for row in ROWS.iter().filter(|row| row.fixture.complete_dataset) {
        let sandbox = Sandbox::new();
        sandbox.project_document(&row.document_with_folder(&sandbox, "folder"));
        let copied = answered(
            row.name,
            &sandbox,
            &[
                "--json",
                "task",
                "copy",
                &qualified(SOURCE, "T-3"),
                "--to",
                "folder",
            ],
        );
        let landed = copied["items"][0]["destination"]
            .as_str()
            .unwrap_or_else(|| panic!("{}: a destination id: {copied}", row.name))
            .to_owned();
        assert_eq!(
            shown(row.name, &sandbox, &landed)["priority"],
            "urgent",
            "{}",
            row.name
        );
        if !PERSISTENT.contains(&row.plugin) {
            continue;
        }
        for (now, file) in [("low", "priority: low"), ("none", "")] {
            answered(
                row.name,
                &sandbox,
                &[
                    "--json",
                    "task",
                    "priority",
                    "set",
                    &qualified(SOURCE, "T-3"),
                    now,
                ],
            );
            answered(
                row.name,
                &sandbox,
                &[
                    "--json",
                    "task",
                    "copy",
                    &qualified(SOURCE, "T-3"),
                    "--to",
                    "folder",
                ],
            );
            assert_eq!(
                shown(row.name, &sandbox, &landed)["priority"],
                now,
                "{}",
                row.name
            );
            let text = only_task_file(&sandbox.subdirectory("folder"));
            let line = text.lines().find(|line| line.starts_with("priority:"));
            assert_eq!(
                line.unwrap_or_default(),
                file,
                "{}: the file holds its priority, and no key for none:\n{text}",
                row.name
            );
        }
    }
}

/// A board configured as the shared row is, beside a folder of Markdown holding one task,
/// so a copy into the board creates an item rather than matching one of the dataset's.
fn notes_and_board(front: &str) -> (Sandbox, GitHubBoardFields) {
    let sandbox = Sandbox::new();
    let (board, fields) = github_projects_with_board(&sandbox);
    let notes = sandbox.subdirectory("notes");
    markdown_task(&notes, "N-1", front, "A task of the notes.");
    sandbox.project_document(&document(&json!({
        "notes": {"plugin": "local-md", "config": empty_folder(&sandbox, "notes")},
        "board": {"plugin": "github-projects", "config": board},
    })));
    (sandbox, fields)
}

/// The `Priority` option ids each field write sent, in order.
fn priority_writes(board: &GitHubBoardFields) -> Vec<String> {
    board
        .served()
        .into_iter()
        .filter_map(|(query, variables)| {
            if query.contains("clearProjectV2ItemFieldValue(input:$input)") {
                return Some("clear".to_owned());
            }
            (query.contains("updateProjectV2ItemFieldValue(input:$input)")
                && variables["input"]["fieldId"] == "FIELD-priority")
                .then(|| {
                    variables["input"]["value"]["singleSelectOptionId"]
                        .as_str()
                        .expect("an option id")
                        .to_owned()
                })
        })
        .collect()
}

#[test]
fn a_copy_into_a_board_selects_the_mapped_option_and_clears_it_for_none() {
    let (sandbox, board) = notes_and_board("title: Notes task\nstatus: todo\npriority: urgent");
    let copied = answered(
        "board",
        &sandbox,
        &["--json", "task", "copy", "notes:N-1", "--to", "board"],
    );
    let landed = copied["items"][0]["destination"]
        .as_str()
        .expect("a destination id")
        .to_owned();
    let native = landed.split_once(':').expect("qualified").1.to_owned();
    assert_eq!(board.priority(&native).as_deref(), Some("Urgent"));
    assert_eq!(priority_writes(&board), ["OPT-p-urgent"]);
    assert_eq!(shown("board", &sandbox, &landed)["priority"], "urgent");

    for (now, option, sent) in [("low", Some("Low"), "OPT-p-low"), ("none", None, "clear")] {
        answered(
            "notes",
            &sandbox,
            &["--json", "task", "priority", "set", "notes:N-1", now],
        );
        answered(
            "board",
            &sandbox,
            &["--json", "task", "copy", "notes:N-1", "--to", "board"],
        );
        assert_eq!(board.priority(&native).as_deref(), option, "{now}");
        assert_eq!(
            priority_writes(&board).last().map(String::as_str),
            Some(sent)
        );
        assert_eq!(shown("board", &sandbox, &landed)["priority"], now);
    }
}

#[test]
fn a_board_field_write_reaches_the_priority_field_and_a_missing_option_or_field_is_refused() {
    let sandbox = Sandbox::new();
    let (config, board) = github_projects_with_board(&sandbox);
    sandbox.project_document(&document(&json!({
        SOURCE: {"plugin": "github-projects", "config": config}
    })));
    let id = qualified(SOURCE, "T-2");

    answered(
        "board",
        &sandbox,
        &["--json", "task", "priority", "set", &id, "medium"],
    );
    assert_eq!(board.priority("T-2").as_deref(), Some("Medium"));
    assert_eq!(priority_writes(&board), ["OPT-p-medium"]);
    // Clearing a priority an item does not hold sends nothing; one it holds is cleared.
    answered(
        "board",
        &sandbox,
        &["--json", "task", "priority", "set", &id, "none"],
    );
    assert_eq!(board.priority("T-2"), None);
    let before = board.served().len();
    answered(
        "board",
        &sandbox,
        &["--json", "task", "priority", "set", &id, "none"],
    );
    assert_eq!(priority_writes(&board), ["OPT-p-medium", "clear"]);
    assert!(
        board.served()[before..]
            .iter()
            .all(|(query, _)| !query.contains("mutation")),
        "a clear of nothing is not sent"
    );

    // The answer is what the board holds afterwards, read back, not the value asked for: a
    // write the board answers as landed and does not keep is reported as it stands.
    board.drop_priority_writes();
    let unkept = answered(
        "board",
        &sandbox,
        &[
            "--json",
            "task",
            "priority",
            "set",
            &qualified(SOURCE, "T-1"),
            "low",
        ],
    );
    assert_eq!(unkept["priority"], "high");
    assert_eq!(board.priority("T-1").as_deref(), Some("High"));

    // An option the board lacks is refused by name, pointing at the verb that adds it.
    board.without_priority_option("Urgent");
    let refused = exits(
        "board",
        &sandbox,
        &["task", "priority", "set", &id, "urgent"],
        1,
    );
    let said = stderr(&refused);
    assert!(
        said.contains("\"Urgent\"")
            && said.contains("does not have it")
            && said.contains("onetaskgraph sources fields work --apply"),
        "{said}"
    );
    board.without_priority_field();
    let refused = exits(
        "board",
        &sandbox,
        &["task", "priority", "set", &id, "low"],
        1,
    );
    let said = stderr(&refused);
    assert!(
        said.contains("this board has no Priority field")
            && said.contains("onetaskgraph sources fields work --apply"),
        "{said}"
    );
    assert!(
        !board
            .documents()
            .iter()
            .any(|query| query.contains("createProjectV2Field")
                || query.contains("updateProjectV2Field(")),
        "a read or a write never creates a field or an option"
    );
}

#[test]
fn a_priority_field_whose_options_are_not_a_list_is_refused_as_malformed_rather_than_missing() {
    let sandbox = Sandbox::new();
    let (config, board) = github_projects_with_board(&sandbox);
    sandbox.project_document(&document(&json!({
        SOURCE: {"plugin": "github-projects", "config": config}
    })));
    let id = qualified(SOURCE, "T-2");
    answered(
        "board",
        &sandbox,
        &["--json", "task", "priority", "set", &id, "medium"],
    );
    assert_eq!(board.priority("T-2").as_deref(), Some("Medium"));
    answered(
        "board",
        &sandbox,
        &["--json", "task", "priority", "set", &id, "none"],
    );
    assert_eq!(board.priority("T-2"), None);

    // An answer this product cannot read is not a board lacking the option: telling the user
    // to add an option the board may well have would send them to fix the wrong thing. Both
    // places the field's definition is read from are held to it — the board's field list, for
    // an item holding no value, and the item's own value, for one holding a value already.
    board.malform_priority_options();
    for (task, held) in [("T-2", None), ("T-1", Some("High"))] {
        let before = board.served().len();
        let refused = run(
            &sandbox,
            &["task", "priority", "set", &qualified(SOURCE, task), "low"],
        );
        let said = stderr(&refused);
        assert!(
            !refused.status.success()
                && said.contains("GitHub Priority field options is not an array")
                && !said.contains("does not have it")
                && !said.contains("sources fields"),
            "{task}: exited {:?}\n{said}",
            refused.status.code()
        );
        assert!(
            board.served()[before..]
                .iter()
                .all(|(query, _)| !query.contains("mutation")),
            "{task}: nothing is written on a response that could not be read"
        );
        assert_eq!(board.priority(task).as_deref(), held, "{task}");
    }
}

#[test]
fn an_option_the_mapping_does_not_name_is_that_tasks_error_rather_than_a_level_or_none() {
    let sandbox = Sandbox::new();
    let (config, board) = github_projects_with_board(&sandbox);
    sandbox.project_document(&document(&json!({
        SOURCE: {"plugin": "github-projects", "config": config}
    })));
    board.with_priority_option("OPT-p-someday", "Someday");
    board.assign_priority("T-3", Some("Someday"));

    let shown_task = exits(
        "board",
        &sandbox,
        &["task", "show", &qualified(SOURCE, "T-3")],
        4,
    );
    let said = stderr(&shown_task);
    assert!(
        said.contains("\"Someday\"") && said.contains("priority_mapping does not name"),
        "{said}"
    );
    // Its siblings read as they always did.
    assert_eq!(
        shown("board", &sandbox, &qualified(SOURCE, "T-1"))["priority"],
        "high"
    );
    // Matched case-insensitively: a board spelling a mapped option another way reads as it.
    board.with_priority_option("OPT-p-shout", "URGENT");
    board.assign_priority("T-3", Some("URGENT"));
    assert_eq!(
        shown("board", &sandbox, &qualified(SOURCE, "T-3"))["priority"],
        "urgent"
    );
}

#[test]
fn a_priority_mapping_is_refused_for_an_unknown_level_or_two_levels_naming_one_option() {
    // An unknown level is refused where the configuration is read, against the plugin's own
    // published schema, and exits 1; a mapping the schema admits and the plugin cannot hold
    // leaves the source unbuilt, which a listing reports as a source that could not answer.
    for (mapping, code, said) in [
        (
            json!({"critical": "Urgent"}),
            1,
            "sources.work.config.priority_mapping: {\"critical\":\"Urgent\"} is not valid",
        ),
        (
            json!({"urgent": "High"}),
            4,
            "sends both urgent and high to the board option \"High\"",
        ),
        (
            json!({"low": "HIGH"}),
            4,
            "sends both high and low to the board option \"HIGH\"",
        ),
        (json!({"medium": " "}), 4, "cannot be blank"),
    ] {
        let sandbox = Sandbox::new();
        let (mut config, _board) = github_projects_with_board(&sandbox);
        config["priority_mapping"] = mapping.clone();
        sandbox.project_document(&document(&json!({
            SOURCE: {"plugin": "github-projects", "config": config}
        })));
        let refused = exits("board", &sandbox, &["task", "list"], code);
        assert!(
            stderr(&refused).contains(said),
            "{mapping}: {}",
            stderr(&refused)
        );
    }
}

#[test]
fn a_board_with_no_priority_mapping_holds_no_priority_and_sends_what_it_always_sent() {
    let sandbox = Sandbox::new();
    let (mut config, board) = github_projects_with_board(&sandbox);
    config
        .as_object_mut()
        .expect("a config block")
        .remove("priority_mapping");
    let notes = sandbox.subdirectory("notes");
    markdown_task(
        &notes,
        "N-1",
        "title: Ranked\nstatus: todo\npriority: high",
        "Body.",
    );
    markdown_task(&notes, "N-2", "title: Unranked\nstatus: todo", "Body.");
    sandbox.project_document(&document(&json!({
        SOURCE: {"plugin": "github-projects", "config": config},
        "notes": {"plugin": "local-md", "config": empty_folder(&sandbox, "notes")},
    })));

    let listed_sources = answered("board", &sandbox, &["--json", "sources", "list"]);
    let work = listed_sources
        .as_array()
        .expect("a listing")
        .iter()
        .find(|source| source["source"] == SOURCE)
        .expect("the board is listed");
    assert_eq!(work["capabilities"]["priority"], "unsupported", "{work}");
    // Every task reads as `none`, whatever the board's own field holds.
    let tasks = answered(
        "board",
        &sandbox,
        &["--json", "task", "list", "--source", SOURCE],
    );
    assert_eq!(
        priorities(&tasks),
        ours(&[
            ("T-1", "none"),
            ("T-2", "none"),
            ("T-3", "none"),
            ("T-4", "none")
        ])
    );

    // A copy carrying a priority is refused before the board is asked anything.
    let asked = board.served().len();
    let refused = answered_failure(
        &sandbox,
        &["--json", "task", "copy", "notes:N-1", "--to", SOURCE],
    );
    assert_eq!(refused["failure"]["kind"], "no-priority");
    let message = refused["failure"]["message"].as_str().expect("a message");
    assert!(
        message.contains("source work cannot hold the field priority")
            && message.contains("notes:N-1's priority high")
            && message.contains("priority_mapping"),
        "{message}"
    );
    assert_eq!(
        board.served().len(),
        asked,
        "the destination was not touched"
    );
    let refused = exits(
        "board",
        &sandbox,
        &["task", "priority", "set", &qualified(SOURCE, "T-1"), "low"],
        1,
    );
    assert!(stderr(&refused).contains("cannot hold the field priority"));
    assert_eq!(board.served().len(), asked, "the source was not asked");

    // A copy carrying `none` writes exactly as it did before priorities existed.
    answered(
        "board",
        &sandbox,
        &["--json", "task", "copy", "notes:N-2", "--to", SOURCE],
    );
    let inventory = onetaskgraph_github_projects::graphql::DOCUMENTS;
    for (query, variables) in board.served() {
        assert!(
            query != onetaskgraph_github_projects::graphql::CLEAR_FIELD
                && query != onetaskgraph_github_projects::graphql::CREATE_FIELD,
            "an instance with no priority_mapping sent a priority document: {query}"
        );
        assert!(
            inventory.iter().any(|(document, _)| *document == query),
            "a document this source does not inventory: {query}"
        );
        assert_ne!(
            variables["input"]["fieldId"], "FIELD-priority",
            "an instance with no priority_mapping wrote the Priority field"
        );
    }
}

#[test]
fn metadata_then_content_and_content_then_metadata_both_read_back_on_one_board_item() {
    let sandbox = Sandbox::new();
    let (config, board) = github_projects_with_board(&sandbox);
    sandbox.project_document(&document(&json!({
        SOURCE: {"plugin": "github-projects", "config": config}
    })));
    let file = sandbox.subdirectory("content").join("body.md");
    let path = file.to_str().expect("a UTF-8 path").to_owned();
    let id = qualified(SOURCE, "T-1");

    // Metadata first, then content: the slot the first wrote survives the second.
    answered(
        "board",
        &sandbox,
        &[
            "--json",
            "task",
            "metadata",
            "set",
            &id,
            "myapp.estimate",
            "3",
        ],
    );
    std::fs::write(&file, "Rewritten after the metadata.").expect("the content file");
    answered(
        "board",
        &sandbox,
        &["--json", "task", "content", "set", &id, "--file", &path],
    );
    let task = shown("board", &sandbox, &id);
    assert_eq!(task["content"], "Rewritten after the metadata.");
    assert_eq!(task["metadata"]["myapp.estimate"], 3);
    // And the caller's keys the dataset gave it are still there.
    assert_eq!(task["metadata"]["onepipeline.turn_budget"], 12);
    let body = board.body("T-1");
    assert!(
        body.as_str().is_some_and(|body| body
            .starts_with("Rewritten after the metadata.\n\n<!-- onetaskgraph.metadata\n")),
        "{body}"
    );

    // Content first, then metadata: the content the first wrote survives the second.
    std::fs::write(&file, "Rewritten before the metadata.").expect("the content file");
    answered(
        "board",
        &sandbox,
        &["--json", "task", "content", "set", &id, "--file", &path],
    );
    answered(
        "board",
        &sandbox,
        &[
            "--json",
            "task",
            "metadata",
            "set",
            &id,
            "myapp.estimate",
            "5",
        ],
    );
    let task = shown("board", &sandbox, &id);
    assert_eq!(task["content"], "Rewritten before the metadata.");
    assert_eq!(task["metadata"]["myapp.estimate"], 5);
    assert_eq!(task["metadata"]["onepipeline.turn_budget"], 12);
    assert_eq!(
        task["priority"], "high",
        "a body write leaves the Priority field"
    );
}

#[test]
fn content_that_would_read_back_as_a_boards_metadata_is_refused_and_the_body_left_alone() {
    let sandbox = Sandbox::new();
    let (config, board) = github_projects_with_board(&sandbox);
    sandbox.project_document(&document(&json!({
        SOURCE: {"plugin": "github-projects", "config": config}
    })));
    let file = sandbox.subdirectory("content").join("body.md");
    let path = file.to_str().expect("a UTF-8 path").to_owned();
    let lookalike = "Notes.\n\n<!-- onetaskgraph.metadata\n{\"caller.number\":1}\n-->";
    std::fs::write(&file, lookalike).expect("the content file");

    // T-3 has no metadata slot, so the block would become one.
    let before = board.body("T-3");
    let refused = exits(
        "board",
        &sandbox,
        &[
            "task",
            "content",
            "set",
            &qualified(SOURCE, "T-3"),
            "--file",
            &path,
        ],
        1,
    );
    assert!(
        stderr(&refused).contains("reads as its own metadata slot")
            && stderr(&refused).contains("next:"),
        "{}",
        stderr(&refused)
    );
    assert_eq!(board.body("T-3"), before, "nothing was sent");

    // T-1 has one, which stays the slot: the block is content there, and reads back as it.
    answered(
        "board",
        &sandbox,
        &[
            "--json",
            "task",
            "content",
            "set",
            &qualified(SOURCE, "T-1"),
            "--file",
            &path,
        ],
    );
    let task = shown("board", &sandbox, &qualified(SOURCE, "T-1"));
    assert_eq!(task["content"], lookalike);
    assert_eq!(task["metadata"]["onepipeline.turn_budget"], 12);
    assert!(task["metadata"].get("caller.number").is_none());
}

#[test]
fn a_stdio_plugin_written_before_priorities_is_never_handed_one_or_a_content_write() {
    let sandbox = Sandbox::new();
    let store = sandbox.subdirectory("store").join("documents.json");
    let log = sandbox.subdirectory("store").join("asked.log");
    let notes = sandbox.subdirectory("notes");
    markdown_task(
        &notes,
        "N-1",
        "title: Ranked\nstatus: todo\npriority: high",
        "Body.",
    );
    sandbox.project_document(&document(&json!({
        "store": store_at(&store, Some(&log), "native"),
        "notes": {"plugin": "local-md", "config": empty_folder(&sandbox, "notes")},
    })));
    let asked = || -> Vec<String> {
        std::fs::read_to_string(&log)
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    };

    let refused = answered_failure(
        &sandbox,
        &["--json", "task", "copy", "notes:N-1", "--to", "store"],
    );
    assert_eq!(refused["failure"]["kind"], "no-priority");
    assert_eq!(refused["failure"]["source"], "store");
    assert_eq!(
        asked(),
        ["initialize"],
        "nothing but the handshake reached it"
    );

    let refused = exits(
        "store",
        &sandbox,
        &["task", "priority", "set", "store:X-1", "none"],
        1,
    );
    assert!(stderr(&refused).contains("source store cannot hold the field priority"));
    let file = sandbox.subdirectory("content").join("body.md");
    std::fs::write(&file, "x").expect("the content file");
    let refused = exits(
        "store",
        &sandbox,
        &[
            "task",
            "content",
            "set",
            "store:X-1",
            "--file",
            file.to_str().expect("a UTF-8 path"),
        ],
        1,
    );
    assert!(
        stderr(&refused).contains("cannot write a task's content on its own"),
        "{}",
        stderr(&refused)
    );
    assert!(
        asked().iter().all(|method| method == "initialize"),
        "{:?}",
        asked()
    );
}
