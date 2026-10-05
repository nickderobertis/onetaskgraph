//! A GitHub board's `status_mapping` scoped by item kind, through the real command-line
//! boundary against the loopback board.
//!
//! A task and a project are both issues on one board and both kinds' names are options of its
//! one `Status` field, so every journey here configures the two halves of one mapping apart
//! and holds the binary to each: which option a status is written as, which status an option
//! reads as, what `--status` keeps, what `sources fields` asks the field to hold — and that a
//! status one kind has no option for is refused on every verb that writes one before the board
//! is sent a single mutation.

use std::path::PathBuf;
use std::process::Output;

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{GitHubBoardFields, document, github_projects_with_board};

// llmlint: ignore-block[tests_mirror_real_usage] Every journey drives the compiled CLI against
// the real loopback HTTP boundary. That a refused write sends no mutation is a wire effect, and
// the fixture's received-request log is the server-side observation of it — the instrument
// `fields.rs` and `write_order.rs` read — rather than an inspection of application internals.

/// One sandbox configuring the fixture board as `board`, with `status_mapping` in place of the
/// shared one, and a folder of Markdown as `plans` holding a task `A` and a project `P`.
struct Setup {
    sandbox: Sandbox,
    board: GitHubBoardFields,
    root: PathBuf,
}

impl Setup {
    fn new(status_mapping: Value) -> Self {
        let sandbox = Sandbox::new();
        let root = sandbox.subdirectory("plans");
        std::fs::create_dir_all(root.join("projects")).expect("the project folder");
        std::fs::create_dir_all(root.join("tasks")).expect("the task folder");
        std::fs::write(
            root.join("projects/P.md"),
            "---\ntitle: A plan\nstatus: Doing\n---\nthe plan\n",
        )
        .expect("the project");
        std::fs::write(
            root.join("tasks/A.md"),
            "---\ntitle: A step\nstatus: Todo\n---\nthe step\n",
        )
        .expect("the task");
        std::fs::write(root.join("body.md"), "a new task\n").expect("a body file");
        let (mut config, board) = github_projects_with_board(&sandbox);
        config["status_mapping"] = status_mapping;
        sandbox.project_document(&document(&json!({
            "plans": {"plugin":"local-md","config":{
                "root": root,
                "status_mapping": {"Todo":"todo","Doing":"in-progress","Queued":"queued"}}},
            "board": {"plugin":"github-projects","config":config}
        })));
        Self {
            sandbox,
            board,
            root,
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

    fn json(&self, arguments: &[&str]) -> Value {
        let output = self.run(arguments);
        assert_eq!(
            output.status.code(),
            Some(0),
            "`onetaskgraph {}` failed\n{}",
            arguments.join(" "),
            stderr(&output)
        );
        serde_json::from_str(&stdout(&output)).expect("the command emits JSON")
    }

    /// Every mutation the board has received.
    fn mutations(&self) -> Vec<(String, Value)> {
        self.board
            .served()
            .into_iter()
            .filter(|(query, _)| query.trim_start().starts_with("mutation"))
            .collect()
    }

    /// The ids `list` printed, in order.
    fn ids(&self, arguments: &[&str]) -> Vec<String> {
        self.json(arguments)["items"]
            .as_array()
            .expect("a page")
            .iter()
            .map(|item| item["id"].as_str().expect("a qualified id").to_owned())
            .collect()
    }
}

#[test]
fn a_status_its_kind_has_no_option_for_is_refused_on_every_verb_before_any_mutation() {
    // Each: the mapping, the command, the kind and category it writes, and what else the
    // refusal must name — why the kind has no option, or the option the board lacks.
    let cases: Vec<(Value, Vec<&str>, &str, &str, &str)> = vec![
        // Unconfigured, with no shipped default.
        (
            json!({"todo":"Todo","in-progress":"Doing"}),
            vec![
                "task",
                "create",
                "board",
                "--project",
                "P-1",
                "--title",
                "New",
                "--status",
                "draft",
                "--body-file",
                "BODY",
            ],
            "task",
            "draft",
            "does not name draft",
        ),
        // Disabled for every kind.
        (
            json!({"todo":"Todo","in-progress":"Doing","backlog":null}),
            vec!["task", "status", "set", "board:T-1", "backlog"],
            "task",
            "backlog",
            "sets backlog to null",
        ),
        // Named for the other kind only — the shipped `Cancelled` is not the fallback.
        (
            json!({"todo":"Todo","in-progress":"Doing","cancelled":{"project":"Cancelled"}}),
            vec!["task", "update", "board:T-1", "--status", "cancelled"],
            "task",
            "cancelled",
            "names cancelled for the other kind only",
        ),
        (
            json!({"todo":{"project":"Todo"},"in-progress":"Doing"}),
            vec!["task", "copy", "plans:A", "--to", "board"],
            "task",
            "todo",
            "names todo for the other kind only",
        ),
        (
            json!({"todo":"Todo","in-progress":{"task":"Doing"}}),
            vec!["project", "copy", "plans:P", "--to", "board", "--no-tasks"],
            "project",
            "in-progress",
            "names in-progress for the other kind only",
        ),
        // A mapped option the board's `Status` field lacks.
        (
            json!({"todo":{"task":"Ready","project":"Todo"},"in-progress":"Doing"}),
            vec![
                "task",
                "create",
                "board",
                "--project",
                "P-1",
                "--title",
                "New",
                "--status",
                "todo",
                "--body-file",
                "BODY",
            ],
            "task",
            "todo",
            "\"Ready\"",
        ),
        (
            json!({"todo":{"task":"Ready","project":"Todo"},"in-progress":"Doing"}),
            vec!["task", "status", "set", "board:T-3", "todo"],
            "task",
            "todo",
            "\"Ready\"",
        ),
        (
            json!({"todo":"Todo","in-progress":{"task":"Doing","project":"Active"}}),
            vec!["project", "copy", "plans:P", "--to", "board", "--no-tasks"],
            "project",
            "in-progress",
            "\"Active\"",
        ),
    ];
    for (mapping, command, kind, category, why) in cases {
        let setup = Setup::new(mapping.clone());
        let body = setup.root.join("body.md");
        let command: Vec<&str> = command
            .into_iter()
            .map(|argument| {
                if argument == "BODY" {
                    body.to_str().expect("a UTF-8 path")
                } else {
                    argument
                }
            })
            .collect();
        let output = setup.run(&command);
        let said = stderr(&output);
        assert_ne!(
            output.status.code(),
            Some(0),
            "{mapping}: `onetaskgraph {}` was meant to be refused\n{}",
            command.join(" "),
            stdout(&output)
        );
        for part in [
            "board",
            &format!("{kind} status"),
            category,
            &format!("status_mapping.{category}.{kind}"),
            why,
        ] {
            assert!(
                said.contains(part),
                "{mapping}: `onetaskgraph {}` was refused without naming {part:?}:\n{said}",
                command.join(" ")
            );
        }
        assert_eq!(
            setup.mutations(),
            Vec::<(String, Value)>::new(),
            "{mapping}: `onetaskgraph {}` sent a mutation before refusing",
            command.join(" ")
        );
    }
}

#[test]
fn each_kind_is_written_read_and_narrowed_through_its_own_half_of_the_mapping() {
    // The board's `Todo` is a task's `todo` and a project's `queued`, and its `Doing` is a
    // project's `in-progress` and no task status at all.
    let setup = Setup::new(json!({
        "todo": {"task": "Todo"},
        "queued": {"project": "Todo"},
        "in-progress": {"project": "Doing"},
    }));
    let statuses = |list: &[&str]| -> Vec<(String, Value)> {
        setup.json(list)["items"]
            .as_array()
            .expect("a page")
            .iter()
            .map(|item| {
                (
                    item["id"].as_str().expect("an id").to_owned(),
                    item["item"]["status"].clone(),
                )
            })
            .collect()
    };
    assert_eq!(
        statuses(&["--json", "task", "list", "--source", "board"]),
        [
            (
                "board:T-1".to_owned(),
                json!({"category": "todo", "name": "Todo"})
            ),
            // Closed as completed: the state decides, whatever the option.
            (
                "board:T-2".to_owned(),
                json!({"category": "done", "name": "Shipped"})
            ),
            (
                "board:T-3".to_owned(),
                json!({"category": "todo", "name": "Todo"})
            ),
            // `Doing` is no task category here, so it is `unknown` under its own name.
            (
                "board:T-4".to_owned(),
                json!({"category": "unknown", "name": "Doing"})
            ),
        ]
    );
    assert_eq!(
        statuses(&["--json", "project", "list", "--source", "board"]),
        [
            (
                "board:P-1".to_owned(),
                json!({"category": "in-progress", "name": "Doing"})
            ),
            (
                "board:P-2".to_owned(),
                json!({"category": "queued", "name": "Todo"})
            ),
        ]
    );
    for (status, tasks, projects) in [
        ("todo", vec!["board:T-1", "board:T-3"], vec![]),
        ("queued", vec![], vec!["board:P-2"]),
        ("in-progress", vec![], vec!["board:P-1"]),
        ("unknown", vec!["board:T-4"], vec![]),
        ("done", vec!["board:T-2"], vec![]),
    ] {
        assert_eq!(
            setup.ids(&[
                "--json", "task", "list", "--source", "board", "--status", status
            ]),
            tasks,
            "tasks at {status}"
        );
        assert_eq!(
            setup.ids(&[
                "--json", "project", "list", "--source", "board", "--status", status
            ]),
            projects,
            "projects at {status}"
        );
    }

    // A task set to `todo` lands on `Todo`, the option a project reads as `queued`, and is
    // read back as the task's own category; a project copied in at `queued` lands on that
    // same option and reads back as `queued`.
    let set = setup.json(&["--json", "task", "status", "set", "board:T-4", "todo"]);
    assert_eq!(
        set["status"],
        json!({"category": "todo", "name": "Todo"}),
        "{set}"
    );
    std::fs::write(
        setup.root.join("projects/P.md"),
        "---\ntitle: A plan\nstatus: Queued\n---\nthe plan\n",
    )
    .expect("the project");
    setup.json(&[
        "--json",
        "project",
        "copy",
        "plans:P",
        "--to",
        "board",
        "--no-tasks",
    ]);
    let queued = statuses(&[
        "--json", "project", "list", "--source", "board", "--status", "queued",
    ]);
    assert_eq!(queued.len(), 2, "P-2 and the copy: {queued:?}");
    assert!(
        queued
            .iter()
            .all(|(_, status)| *status == json!({"category": "queued", "name": "Todo"})),
        "{queued:?}"
    );
}

#[test]
fn sources_fields_names_each_kinds_missing_options_and_apply_adds_both_keeping_every_id() {
    let setup = Setup::new(json!({
        "todo": "Todo", "in-progress": "Doing",
        "draft": {"task": "Idea"},
        "queued": {"task": "Queued", "project": "Waiting"},
        "done": {"project": "Shipped"},
    }));
    let before = setup.board.status_options();

    let planned = setup.json(&["--json", "sources", "fields", "board"]);
    let status = &planned["fields"][0];
    assert_eq!(status["field"], "Status");
    assert_eq!(status["missing"], json!(["Idea", "Waiting"]));
    assert_eq!(
        status["kinds"],
        json!([{"kind": "task", "missing": ["Idea"]},
               {"kind": "project", "missing": ["Waiting"]}])
    );
    assert_eq!(status["outcome"], "planned");
    assert!(
        planned["fields"][1].get("kinds").is_none(),
        "Priority carries no kinds: {}",
        planned["fields"][1]
    );
    let text = setup.run(&["sources", "fields", "board"]);
    assert_eq!(
        stdout(&text),
        "board: missing configured Status options: task: Idea; project: Waiting\n\
         board: missing configured Priority options: none\n"
    );
    assert!(setup.mutations().is_empty(), "a plan writes nothing");

    let applied = setup.run(&["sources", "fields", "board", "--apply"]);
    assert_eq!(applied.status.code(), Some(0), "{}", stderr(&applied));
    assert_eq!(
        stdout(&applied),
        "board: added and verified Status options: task: Idea; project: Waiting\n\
         board: missing configured Priority options: none\n"
    );
    let sent = setup.mutations();
    assert_eq!(sent.len(), 1, "one whole-list update: {sent:?}");
    let options = sent[0].1["input"]["singleSelectOptions"]
        .as_array()
        .expect("the whole option list")
        .clone();
    for old in &before {
        assert!(
            options.contains(old),
            "every existing option goes back with its id: {old} in {options:?}"
        );
    }
    let after = setup.board.status_options();
    for old in &before {
        assert!(after.contains(old), "{old} kept its id: {after:?}");
    }
    for added in ["Idea", "Waiting"] {
        assert!(
            after.iter().any(|option| option["name"] == added),
            "{added} was added: {after:?}"
        );
    }

    // The board now holds both kinds' options: nothing is missing, and a task's `draft` lands
    // on the option only a task maps.
    let again = setup.json(&["--json", "sources", "fields", "board"]);
    assert_eq!(again["fields"][0]["missing"], json!([]));
    assert!(again["fields"][0].get("kinds").is_none(), "{again}");
    let set = setup.json(&["--json", "task", "status", "set", "board:T-3", "draft"]);
    assert_eq!(
        set["status"],
        json!({"category": "draft", "name": "Idea"}),
        "{set}"
    );
}
