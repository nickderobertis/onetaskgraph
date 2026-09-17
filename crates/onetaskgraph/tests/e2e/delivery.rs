//! `task status set`, the `delivers` relation, and the rule that keeps a delivered task in step
//! with its deliverers — driven the way a user drives them.
//!
//! Every journey spawns the compiled binary and asserts on its exit code, stdout and stderr,
//! and on what a later invocation reads back. The sources are ones that outlive a process —
//! folders of Markdown, and the GitHub fixture board served over loopback HTTP — because every
//! claim here is about what the store holds after a command has exited.

use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::{Value, json};

use crate::common::{SOURCE_BOUNDARIES, Sandbox, SourceBoundary, stderr, stdout};
use crate::fixtures::{GitHubBoardFields, document, github_projects_with_board};
use crate::machine::{bundle, validates};

/// The credential the GitHub fixture board's source names, which a hosted plugin is handed.
const BOARD_TOKEN: &[&str] = &["GITHUB_PROJECTS_FIXTURE_TOKEN"];

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

fn parsed(output: &Output) -> Value {
    let text = stdout(output);
    serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("not one JSON document ({error}):\n{text}"))
}

/// A folder of Markdown under the sandbox, holding `files`.
fn folder(sandbox: &Sandbox, name: &str, files: &[(&str, &str)]) -> PathBuf {
    let root = sandbox.subdirectory(name);
    for kind in ["tasks", "projects"] {
        std::fs::create_dir_all(root.join(kind)).expect("a folder");
    }
    for (relative, text) in files {
        std::fs::write(root.join(relative), text).expect("a file");
    }
    root
}

fn markdown(root: &Path) -> Value {
    json!({"plugin": "local-md", "config": {"root": root}})
}

fn read(root: &Path, relative: &str) -> String {
    std::fs::read_to_string(root.join(relative)).expect("the file reads")
}

/// One task as `task show --json` reads it back.
fn show(sandbox: &Sandbox, id: &str) -> Value {
    let output = exits(id, sandbox, &["task", "show", id, "--json"], 0);
    parsed(&output)
}

fn item(sandbox: &Sandbox, id: &str) -> Value {
    show(sandbox, id)["items"][0]["item"].clone()
}

fn category(sandbox: &Sandbox, id: &str) -> String {
    item(sandbox, id)["status"]["category"]
        .as_str()
        .expect("a category")
        .to_owned()
}

/// Each delivered entry as `ticket deliverer outcome from->to`, for a comparison a reader can
/// read.
fn entries_of(answer: &Value) -> Vec<String> {
    answer["delivered"]
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .map(|entry| {
                    let mut line = format!(
                        "{} {} {}",
                        entry["ticket"].as_str().expect("a ticket"),
                        entry["deliverer"].as_str().expect("a deliverer"),
                        entry["outcome"].as_str().expect("an outcome"),
                    );
                    if let Some(from) = entry["from"].as_str() {
                        line.push_str(&format!(" {from}"));
                    }
                    if let Some(to) = entry["to"].as_str() {
                        line.push_str(&format!("->{to}"));
                    }
                    line
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The GitHub fixture board as a source called `name`, on the boundary given.
fn board_source(sandbox: &Sandbox, boundary: SourceBoundary) -> (Value, GitHubBoardFields) {
    let (config, board) = github_projects_with_board(sandbox);
    (
        boundary.source_with_secrets("github-projects", config, BOARD_TOKEN),
        board,
    )
}

/// Every mutation the board served after `from`, with the variables it was sent.
fn mutations(board: &GitHubBoardFields, from: usize) -> Vec<(String, Value)> {
    board
        .served()
        .into_iter()
        .skip(from)
        .filter(|(document, _)| document.trim_start().starts_with("mutation"))
        .map(|(document, variables)| {
            let operation = [
                "updateProjectV2ItemFieldValue",
                "updateIssue",
                "createIssue",
                "addProjectV2ItemById",
            ]
            .into_iter()
            .find(|name| document.contains(&format!("{name}(input:$input)")))
            .unwrap_or("another mutation");
            (operation.to_owned(), variables)
        })
        .collect()
}

/// A task file whose front matter carries every shape a status-only write must step around.
const RICH: &str = "---\ntitle: Alpha\nstatus: todo\nlabels: [bug]\nproject: P-1\nmetadata:\n  caller.count: 3\ndepends_on: [T-2]\ndelivers: [T-2]\ndelivered_by: [\"plan:P-9\"]\n---\nThe body.\n\n## Comments\n\n<!-- onetaskgraph:comment id=\"20260913T151107Z-1\" created_at=\"2026-09-13T15:11:07Z\" updated_at=\"2026-09-13T15:11:07Z\" -->\n### comment — 2026-09-13T15:11:07Z\n\nKeep me.\n\n<!-- /onetaskgraph:comment -->\n";

#[test]
fn a_status_set_on_local_markdown_rewrites_the_status_and_nothing_else() {
    for boundary in SOURCE_BOUNDARIES {
        let who = format!("{boundary:?}");
        let sandbox = Sandbox::new();
        let root = folder(
            &sandbox,
            "work",
            &[
                ("tasks/T-1.md", RICH),
                (
                    "tasks/T-2.md",
                    "---\ntitle: Beta\nstatus: done\ndelivered_by: [\"work:T-1\"]\n---\n",
                ),
                ("projects/P-1.md", "---\ntitle: Engine\nstatus: todo\n---\n"),
            ],
        );
        sandbox.project_document(&document(&json!({
            "work": boundary.source("local-md", json!({"root": root})),
        })));
        let before = show(&sandbox, "work:T-1");
        let deps_before = parsed(&exits(
            &who,
            &sandbox,
            &["task", "deps", "work:T-1", "--json"],
            0,
        ))["items"]
            .clone();
        let beta = read(&root, "tasks/T-2.md");

        let set = parsed(&exits(
            &who,
            &sandbox,
            &["task", "status", "set", "work:T-1", "queued", "--json"],
            0,
        ));
        assert_eq!(set["id"], "work:T-1", "{who}");
        assert_eq!(
            set["status"],
            json!({"category": "queued", "name": "queued"}),
            "{who}"
        );
        assert_eq!(
            entries_of(&set),
            ["work:T-2 work:T-1 left done"],
            "{who}: {set:#}"
        );
        validates(
            &bundle(&sandbox),
            "TaskStatusSet",
            &set,
            "task status set --json",
        );

        assert_eq!(
            read(&root, "tasks/T-1.md"),
            RICH.replace("status: todo", "status: queued"),
            "{who}: the record is byte-identical apart from its status"
        );
        assert_eq!(
            read(&root, "tasks/T-2.md"),
            beta,
            "{who}: a task left alone is untouched"
        );

        let mut expected = before.clone();
        expected["items"][0]["item"]["status"] = json!({"category": "queued", "name": "queued"});
        assert_eq!(
            show(&sandbox, "work:T-1"),
            expected,
            "{who}: everything else reads back unchanged"
        );
        assert_eq!(
            expected["items"][0]["item"]["delivers"],
            json!(["work:T-2"]),
            "{who}: task show prints every entry qualified"
        );
        assert_eq!(
            expected["items"][0]["item"]["delivered_by"],
            json!(["plan:P-9"])
        );
        assert_eq!(expected["comments"][0]["body"], "Keep me.");
        let deps_after = parsed(&exits(
            &who,
            &sandbox,
            &["task", "deps", "work:T-1", "--json"],
            0,
        ))["items"]
            .clone();
        assert_eq!(
            deps_after, deps_before,
            "{who}: the dependencies read back unchanged"
        );
        assert_eq!(
            deps_after.as_array().map(Vec::len),
            Some(1),
            "{who}: {deps_after:#}"
        );

        // The same, said in words.
        let rendered = stdout(&exits(
            &who,
            &sandbox,
            &["task", "status", "set", "work:T-1", "in-progress"],
            0,
        ));
        assert!(
            rendered.contains("in-progress (in progress)"),
            "{who}: {rendered}"
        );
        assert!(
            rendered.contains("delivered work:T-2 by work:T-1: left at done"),
            "{who}: {rendered}"
        );
        let shown = stdout(&exits(&who, &sandbox, &["task", "show", "work:T-1"], 0));
        assert!(
            shown
                .lines()
                .any(|line| line.starts_with("delivers:") && line.ends_with("work:T-2")),
            "{who}: {shown}"
        );
        assert!(
            shown
                .lines()
                .any(|line| line.starts_with("delivered by:") && line.ends_with("plan:P-9")),
            "{who}: {shown}"
        );
        let listed = stdout(&exits(&who, &sandbox, &["task", "list"], 0));
        assert!(
            listed.contains("delivers work:T-2; delivered by plan:P-9"),
            "{who}: {listed}"
        );
        let listed = parsed(&exits(&who, &sandbox, &["task", "list", "--json"], 0));
        let alpha = listed["items"]
            .as_array()
            .expect("items")
            .iter()
            .find(|task| task["id"] == "work:T-1")
            .expect("listed");
        assert_eq!(alpha["item"]["delivers"], json!(["work:T-2"]), "{who}");
        let beta = listed["items"]
            .as_array()
            .expect("items")
            .iter()
            .find(|task| task["id"] == "work:T-2")
            .expect("listed");
        assert_eq!(beta["item"]["delivered_by"], json!(["work:T-1"]), "{who}");
    }
}

#[test]
fn a_status_set_with_no_deliveries_says_so_in_text() {
    let sandbox = Sandbox::new();
    let root = folder(
        &sandbox,
        "work",
        &[("tasks/T-1.md", "---\ntitle: One\nstatus: todo\n---\n")],
    );
    sandbox.project_document(&document(&json!({
        "work": {"plugin":"local-md", "config":{"root":root}}
    })));
    let output = exits(
        "a task with no deliveries",
        &sandbox,
        &["task", "status", "set", "work:T-1", "queued"],
        0,
    );
    assert!(stdout(&output).contains("delivered: none"));
}

#[test]
fn a_status_set_on_a_github_board_sends_only_the_option_update_or_the_close_or_reopen() {
    for boundary in SOURCE_BOUNDARIES {
        let who = format!("{boundary:?}");
        let sandbox = Sandbox::new();
        let (work, board) = board_source(&sandbox, boundary);
        sandbox.project_document(&document(&json!({"work": work})));
        let before = item(&sandbox, "work:T-3");
        let body = sandbox.subdirectory("bodies").join("evidence.md");
        std::fs::write(&body, "Seen on main.\n").expect("a comment body");
        exits(
            &who,
            &sandbox,
            &[
                "task",
                "comment",
                "add",
                "work:T-3",
                "--body-file",
                body.to_str().expect("a UTF-8 path"),
                "--json",
            ],
            0,
        );
        let comments_before = show(&sandbox, "work:T-3")["comments"].clone();
        let deps_before = parsed(&exits(
            &who,
            &sandbox,
            &["task", "deps", "work:T-3", "--json"],
            0,
        ))["items"]
            .clone();

        let steps: [(&str, &str, Value, Vec<&str>); 4] = [
            (
                "work:T-3",
                "queued",
                json!({"category": "queued", "name": "Queued"}),
                vec!["updateProjectV2ItemFieldValue"],
            ),
            (
                "work:T-3",
                "done",
                json!({"category": "done", "name": "Queued"}),
                vec!["updateIssue"],
            ),
            (
                "work:T-3",
                "cancelled",
                json!({"category": "cancelled", "name": "Queued"}),
                vec!["updateIssue"],
            ),
            (
                "work:T-2",
                "in-progress",
                json!({"category": "in-progress", "name": "Doing"}),
                vec!["updateIssue", "updateProjectV2ItemFieldValue"],
            ),
        ];
        for (task, wanted, status, sent) in steps {
            let from = board.served().len();
            let set = parsed(&exits(
                &who,
                &sandbox,
                &["task", "status", "set", task, wanted, "--json"],
                0,
            ));
            assert_eq!(set["status"], status, "{who} {task} {wanted}");
            let served = mutations(&board, from);
            assert_eq!(
                served
                    .iter()
                    .map(|(name, _)| name.as_str())
                    .collect::<Vec<_>>(),
                sent,
                "{who} {task} {wanted}: {served:#?}"
            );
            for (name, variables) in &served {
                let input = variables["input"].as_object().expect("an input object");
                for field in ["title", "body", "labelIds"] {
                    assert!(
                        !input.contains_key(field),
                        "{who}: {name} carried {field}: {variables}"
                    );
                }
                if *name == "updateIssue" {
                    let expected = match wanted {
                        "done" => json!({"value": "CLOSED", "stateReason": "COMPLETED"}),
                        "cancelled" => json!({"value": "CLOSED", "stateReason": "NOT_PLANNED"}),
                        _ => json!({"value": "OPEN"}),
                    };
                    assert_eq!(input["stateInput"], expected, "{who} {task} {wanted}");
                }
            }
            assert_eq!(
                item(&sandbox, task)["status"],
                status,
                "{who}: task show reads it back"
            );
        }
        let after = item(&sandbox, "work:T-3");
        for field in [
            "title",
            "content",
            "labels",
            "metadata",
            "project",
            "delivers",
            "delivered_by",
        ] {
            assert_eq!(after[field], before[field], "{who}: {field} is unchanged");
        }
        assert_eq!(
            show(&sandbox, "work:T-3")["comments"],
            comments_before,
            "{who}: comments are unchanged"
        );
        assert_eq!(
            comments_before.as_array().map(Vec::len),
            Some(1),
            "{who}: {comments_before:#}"
        );
        let deps_after = parsed(&exits(
            &who,
            &sandbox,
            &["task", "deps", "work:T-3", "--json"],
            0,
        ))["items"]
            .clone();
        assert_eq!(deps_after, deps_before, "{who}: dependencies are unchanged");
        assert!(
            !deps_before.as_array().expect("edges").is_empty(),
            "{who}: {deps_before:#}"
        );
    }
}

#[test]
fn every_status_set_refusal_names_the_problem_and_the_next_action() {
    let sandbox = Sandbox::new();
    let work = folder(
        &sandbox,
        "work",
        &[("tasks/T-1.md", "---\ntitle: One\nstatus: todo\n---\n")],
    );
    let (config, _board) = github_projects_with_board(&sandbox);
    sandbox.project_document(&document(&json!({
        "work": {"plugin": "local-md", "config": {"root": work, "status_mapping": {"todo": "todo"}}},
        "frozen": {"plugin": "in-memory", "config": {
            "capabilities": {"writes": "unsupported"},
            "tasks": [{"id": "T-1", "title": "One", "content": null,
                       "status": {"category": "todo", "name": "Todo"}, "labels": []}]}},
        "board": {"plugin": "github-projects", "config": config},
    })));
    for (arguments, message) in [
        (vec!["T-1", "done"], "\"T-1\" is not a qualified id"),
        (
            vec!["nowhere:T-1", "done"],
            "no source named \"nowhere\" is configured",
        ),
        (vec!["work:T-404", "done"], "no task with the id work:T-404"),
        (
            vec!["work:T-1", "queued"],
            "cannot represent the field `status`: this source reads \"queued\" as unknown, not queued",
        ),
        (
            vec!["board:T-3", "draft"],
            "status draft is disabled for source board",
        ),
        (
            vec!["frozen:T-1", "done"],
            "source frozen cannot write a status: its plugin is in-memory",
        ),
    ] {
        let mut full = vec!["task", "status", "set"];
        full.extend(&arguments);
        let output = exits(&arguments.join(" "), &sandbox, &full, 1);
        let said = stderr(&output);
        assert!(
            said.contains(message) && said.contains("next:"),
            "{arguments:?}: {said}"
        );
        full.push("--json");
        let failed = parsed(&exits(&arguments.join(" "), &sandbox, &full, 1));
        assert!(
            failed["failure"]["message"]
                .as_str()
                .is_some_and(|text| text.contains(message)),
            "{arguments:?}: {failed:#}"
        );
        validates(
            &bundle(&sandbox),
            "FailureDocument",
            &failed,
            "a refused status set",
        );
    }
    assert_eq!(
        read(&work, "tasks/T-1.md"),
        "---\ntitle: One\nstatus: todo\n---\n"
    );
    // A category the vocabulary has no word for is refused as the invocation it is.
    exits(
        "an unknown category",
        &sandbox,
        &["task", "status", "set", "work:T-1", "later"],
        2,
    );
}

#[test]
fn a_list_naming_its_own_task_repeating_one_or_holding_no_task_id_is_refused_by_name() {
    let sandbox = Sandbox::new();
    let work = folder(
        &sandbox,
        "work",
        &[
            ("tasks/SELF.md", "---\ndelivers: [SELF]\n---\n"),
            (
                "tasks/TWICE.md",
                "---\ndelivers: [SELF, \"work:SELF\"]\n---\n",
            ),
            ("tasks/NUMBER.md", "---\ndelivered_by: [3]\n---\n"),
        ],
    );
    let (config, _board) = github_projects_with_board(&sandbox);
    let into = folder(&sandbox, "into", &[]);
    sandbox.project_document(&document(&json!({
        "work": markdown(&work),
        "into": markdown(&into),
        "board": {"plugin": "github-projects", "config": config},
    })));
    for (id, message) in [
        (
            "work:SELF",
            "delivers on task SELF names SELF, which is that task itself",
        ),
        (
            "work:TWICE",
            "delivers on task TWICE names work:SELF more than once",
        ),
        (
            "work:NUMBER",
            "delivered_by on task NUMBER holds 3, which is not a task id",
        ),
    ] {
        let output = run(&sandbox, &["task", "show", id]);
        assert_ne!(output.status.code(), Some(0), "{id}");
        assert!(
            stderr(&output).contains(message),
            "{id}: {}",
            stderr(&output)
        );
    }

    // A write is refused too, naming the field, before the destination holds anything: this
    // one names its own copy once it lands.
    let plan = folder(
        &sandbox,
        "plan",
        &[(
            "tasks/A.md",
            "---\ntitle: A\nstatus: todo\ndelivers: [\"into:A\"]\n---\n",
        )],
    );
    let mut sources: Value = serde_json::from_str(
        &std::fs::read_to_string(sandbox.project().join("onetaskgraph.yaml"))
            .expect("the document"),
    )
    .expect("JSON");
    sources["sources"]["plan"] = markdown(&plan);
    sandbox.project_document(&serde_json::to_string(&sources).expect("renders"));
    let output = exits(
        "a self-naming copy",
        &sandbox,
        &["task", "copy", "plan:A", "--to", "into"],
        1,
    );
    assert!(
        stderr(&output).contains("cannot represent the field `delivers`: delivers on task A names into:A, which is that task itself"),
        "{}",
        stderr(&output)
    );
    assert!(!into.join("tasks/A.md").exists());
}

/// The plan every copy journey below starts from: a project whose task `A` delivers its
/// sibling `B` and a ticket in another folder, and records a `delivered_by` of its own that
/// a copy must never carry.
fn a_plan(sandbox: &Sandbox) -> (PathBuf, PathBuf, PathBuf) {
    let plan = folder(
        sandbox,
        "plan",
        &[
            ("projects/P.md", "---\ntitle: Plan\nstatus: todo\n---\n"),
            (
                "tasks/A.md",
                "---\ntitle: A\nstatus: queued\nproject: P\ndelivers: [B, \"tickets:T-1\"]\ndelivered_by: [\"x:Y-1\"]\n---\nDo A.\n",
            ),
            (
                "tasks/B.md",
                "---\ntitle: B\nstatus: queued\nproject: P\n---\nDo B.\n",
            ),
        ],
    );
    let tickets = folder(
        sandbox,
        "tickets",
        &[(
            "tasks/T-1.md",
            "---\ntitle: Ticket\nstatus: todo\n---\nThe ticket body.\n",
        )],
    );
    let board = folder(sandbox, "board", &[]);
    sandbox.project_document(&document(&json!({
        "plan": markdown(&plan),
        "tickets": markdown(&tickets),
        "board": markdown(&board),
    })));
    (plan, tickets, board)
}

#[test]
fn a_copy_rewrites_members_carries_the_rest_qualified_and_keeps_the_back_reference_in_step() {
    let sandbox = Sandbox::new();
    let (plan, tickets, _board) = a_plan(&sandbox);

    let copied = parsed(&exits(
        "the first copy",
        &sandbox,
        &["project", "copy", "plan:P", "--to", "board", "--json"],
        0,
    ));
    validates(
        &bundle(&sandbox),
        "CopyReport",
        &copied,
        "project copy --json",
    );
    assert_eq!(copied["delivers_rewritten"], 1, "{copied:#}");
    assert_eq!(
        entries_of(&copied),
        [
            "board:B board:A unchanged queued",
            "tickets:T-1 board:A written todo->queued"
        ],
        "{copied:#}"
    );
    let landed = item(&sandbox, "board:A");
    assert_eq!(landed["delivers"], json!(["board:B", "tickets:T-1"]));
    assert!(
        landed.get("delivered_by").is_none(),
        "a copy never takes delivered_by: {landed:#}"
    );
    assert_eq!(
        item(&sandbox, "board:B")["delivered_by"],
        json!(["board:A"])
    );
    let ticket = item(&sandbox, "tickets:T-1");
    assert_eq!(ticket["delivered_by"], json!(["board:A"]));
    assert_eq!(ticket["status"]["category"], "queued");
    assert_eq!(
        read(&tickets, "tasks/T-1.md"),
        "---\ntitle: Ticket\nstatus: queued\ndelivered_by: [\"board:A\"]\n---\nThe ticket body.\n",
        "maintaining the back-reference changes nothing else on the ticket"
    );

    // A total replacement of B keeps the delivered_by the store gave it.
    std::fs::write(
        plan.join("tasks/B.md"),
        "---\ntitle: B, retitled\nstatus: queued\nproject: P\n---\nDo B.\n",
    )
    .expect("an edit");
    let again = parsed(&exits(
        "a second copy",
        &sandbox,
        &["project", "copy", "plan:P", "--to", "board", "--json"],
        0,
    ));
    let replaced = item(&sandbox, "board:B");
    assert_eq!(replaced["title"], "B, retitled", "{again:#}");
    assert_eq!(replaced["delivered_by"], json!(["board:A"]));
    assert_eq!(
        entries_of(&again),
        [
            "board:B board:A unchanged queued",
            "tickets:T-1 board:A unchanged queued"
        ],
        "a copy that leaves the deliverer unchanged re-evaluates all the same"
    );

    // A ticket whose status drifted is written back by the next copy, and says so in words.
    std::fs::write(
        tickets.join("tasks/T-1.md"),
        "---\ntitle: Ticket\nstatus: todo\ndelivered_by: [\"board:A\"]\n---\nThe ticket body.\n",
    )
    .expect("a drift");
    let rendered = stdout(&exits(
        "a copy in words",
        &sandbox,
        &["project", "copy", "plan:P", "--to", "board"],
        0,
    ));
    assert!(rendered.contains("delivers: 1 rewritten"), "{rendered}");
    assert!(
        rendered.contains("delivered tickets:T-1 by board:A: written from todo to queued"),
        "{rendered}"
    );
    assert_eq!(category(&sandbox, "tickets:T-1"), "queued");

    // Dropping the ticket from A's delivers removes the back-reference and releases it.
    std::fs::write(
        plan.join("tasks/A.md"),
        "---\ntitle: A\nstatus: queued\nproject: P\ndelivers: [B]\n---\nDo A.\n",
    )
    .expect("an edit");
    let dropped = parsed(&exits(
        "a copy that drops the ticket",
        &sandbox,
        &["project", "copy", "plan:P", "--to", "board", "--json"],
        0,
    ));
    assert_eq!(
        entries_of(&dropped),
        [
            "board:B board:A unchanged queued",
            "tickets:T-1 board:A written queued->todo"
        ],
        "{dropped:#}"
    );
    assert_eq!(
        read(&tickets, "tasks/T-1.md"),
        "---\ntitle: Ticket\nstatus: todo\n---\nThe ticket body.\n"
    );
}

/// One journey of the rule through `task status set`: deliverers in `plan`, one ticket in
/// `tickets`, each hand-authored, and the categories the ticket reads afterwards.
struct Rule<'a> {
    /// The ticket's front matter status and delivered_by.
    ticket: &'a str,
    /// Every other deliverer already named, as `(native id, status)`.
    others: &'a [(&'a str, &'a str)],
    /// The category `plan:P` is set to.
    set: &'a str,
    /// What the ticket's entry says.
    said: &'a str,
    /// The category the ticket reads afterwards.
    after: &'a str,
}

fn drive(rule: &Rule<'_>) {
    let who = format!(
        "P set {} with ticket `{}` beside {:?}",
        rule.set, rule.ticket, rule.others
    );
    let sandbox = Sandbox::new();
    let mut files = vec![(
        "tasks/P.md".to_owned(),
        "---\ntitle: P\nstatus: todo\ndelivers: [\"tickets:T\"]\n---\n".to_owned(),
    )];
    for (native, status) in rule.others {
        files.push((
            format!("tasks/{native}.md"),
            format!("---\ntitle: {native}\nstatus: {status}\ndelivers: [\"tickets:T\"]\n---\n"),
        ));
    }
    let borrowed: Vec<(&str, &str)> = files
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let plan = folder(&sandbox, "plan", &borrowed);
    let tickets = folder(
        &sandbox,
        "tickets",
        &[(
            "tasks/T.md",
            &format!("---\ntitle: T\n{}\n---\n", rule.ticket),
        )],
    );
    sandbox.project_document(&document(
        &json!({"plan": markdown(&plan), "tickets": markdown(&tickets)}),
    ));
    let set = parsed(&exits(
        &who,
        &sandbox,
        &["task", "status", "set", "plan:P", rule.set, "--json"],
        0,
    ));
    assert_eq!(
        entries_of(&set),
        [format!("tickets:T plan:P {}", rule.said)],
        "{who}: {set:#}"
    );
    assert_eq!(category(&sandbox, "tickets:T"), rule.after, "{who}");
}

#[test]
fn a_delivered_task_follows_one_deliverer_through_queued_in_progress_and_done() {
    let sandbox = Sandbox::new();
    let plan = folder(
        &sandbox,
        "plan",
        &[(
            "tasks/P.md",
            "---\ntitle: P\nstatus: todo\ndelivers: [\"tickets:T\"]\n---\n",
        )],
    );
    let tickets = folder(
        &sandbox,
        "tickets",
        &[("tasks/T.md", "---\ntitle: T\nstatus: todo\n---\n")],
    );
    sandbox.project_document(&document(
        &json!({"plan": markdown(&plan), "tickets": markdown(&tickets)}),
    ));
    for (set, said) in [
        ("queued", "tickets:T plan:P written todo->queued"),
        (
            "in-progress",
            "tickets:T plan:P written queued->in-progress",
        ),
        ("done", "tickets:T plan:P written in-progress->done"),
    ] {
        let answer = parsed(&exits(
            set,
            &sandbox,
            &["task", "status", "set", "plan:P", set, "--json"],
            0,
        ));
        assert_eq!(entries_of(&answer), [said]);
        assert_eq!(category(&sandbox, "tickets:T"), set);
    }
    assert_eq!(
        item(&sandbox, "tickets:T")["delivered_by"],
        json!(["plan:P"])
    );
}

#[test]
fn a_release_returns_the_ticket_to_todo_and_draft_or_backlog_writes_nothing() {
    for (set, said, after) in [
        ("todo", "written in-progress->todo", "todo"),
        ("cancelled", "written in-progress->todo", "todo"),
        ("unknown", "written in-progress->todo", "todo"),
        ("draft", "unchanged in-progress", "in-progress"),
        ("backlog", "unchanged in-progress", "in-progress"),
    ] {
        drive(&Rule {
            ticket: "status: in progress\ndelivered_by: [\"plan:P\"]",
            others: &[],
            set,
            said,
            after,
        });
    }
}

#[test]
fn two_deliverers_decide_the_ticket_by_the_result_rule() {
    for (other, set, after) in [
        ("in progress", "cancelled", "in-progress"),
        ("in progress", "todo", "in-progress"),
        ("todo", "queued", "queued"),
        ("todo", "done", "todo"),
        ("queued", "done", "queued"),
        ("backlog", "done", "todo"),
        ("cancelled", "cancelled", "todo"),
        ("done", "done", "done"),
        ("cancelled", "done", "done"),
    ] {
        let said = if after == "queued" {
            "written todo->queued".to_owned()
        } else if after == "todo" {
            "unchanged todo".to_owned()
        } else {
            format!("written todo->{after}")
        };
        drive(&Rule {
            ticket: "status: todo\ndelivered_by: [\"plan:O\"]",
            others: &[("O", other)],
            set,
            said: &said,
            after,
        });
    }
}

#[test]
fn a_ticket_at_draft_backlog_unknown_done_or_cancelled_is_left_as_it_is() {
    for (word, held) in [
        ("draft", "draft"),
        ("backlog", "backlog"),
        ("someday", "unknown"),
        ("done", "done"),
        ("cancelled", "cancelled"),
    ] {
        drive(&Rule {
            ticket: &format!("status: {word}"),
            others: &[],
            set: "in-progress",
            said: &format!("left {held}"),
            after: held,
        });
    }
}

#[test]
fn a_deliverer_that_no_longer_exists_is_pruned_and_one_nothing_configures_fails_the_run() {
    let sandbox = Sandbox::new();
    let plan = folder(
        &sandbox,
        "plan",
        &[(
            "tasks/P.md",
            "---\ntitle: P\nstatus: todo\ndelivers: [\"tickets:T\", \"tickets:U\"]\n---\n",
        )],
    );
    let tickets = folder(
        &sandbox,
        "tickets",
        &[
            (
                "tasks/T.md",
                "---\ntitle: T\nstatus: todo\ndelivered_by: [\"plan:GONE\", \"plan:P\"]\n---\n",
            ),
            (
                "tasks/U.md",
                "---\ntitle: U\nstatus: todo\ndelivered_by: [\"elsewhere:X-1\"]\n---\n",
            ),
        ],
    );
    sandbox.project_document(&document(
        &json!({"plan": markdown(&plan), "tickets": markdown(&tickets)}),
    ));
    let output = exits(
        "a deliverer gone and one unconfigured",
        &sandbox,
        &["task", "status", "set", "plan:P", "in-progress", "--json"],
        4,
    );
    let answer = parsed(&output);
    validates(
        &bundle(&sandbox),
        "TaskStatusSet",
        &answer,
        "a partial status set",
    );
    let entries = answer["delivered"].as_array().expect("entries");
    assert_eq!(entries[0]["outcome"], "written", "{answer:#}");
    assert_eq!(entries[0]["pruned"], json!(["plan:GONE"]));
    assert_eq!(
        item(&sandbox, "tickets:T")["delivered_by"],
        json!(["plan:P"])
    );
    assert_eq!(category(&sandbox, "tickets:T"), "in-progress");

    assert_eq!(entries[1]["outcome"], "failed", "{answer:#}");
    assert_eq!(entries[1]["from"], "todo");
    assert_eq!(entries[1]["failure"]["kind"], "unknown-source");
    assert!(entries[1].get("pruned").is_none());
    assert_eq!(
        item(&sandbox, "tickets:U")["delivered_by"],
        json!(["elsewhere:X-1", "plan:P"])
    );
    assert_eq!(category(&sandbox, "tickets:U"), "todo");
    assert_eq!(
        category(&sandbox, "plan:P"),
        "in-progress",
        "the deliverer's own write landed"
    );
    assert!(
        stderr(&output).contains("task tickets:U could not be kept in step with plan:P"),
        "{}",
        stderr(&output)
    );

    // Pruned alone, with nothing failed, is a complete run.
    std::fs::write(
        plan.join("tasks/P.md"),
        "---\ntitle: P\nstatus: todo\ndelivers: [\"tickets:T\"]\n---\n",
    )
    .expect("an edit");
    std::fs::write(
        tickets.join("tasks/T.md"),
        "---\ntitle: T\nstatus: todo\ndelivered_by: [\"plan:GONE\"]\n---\n",
    )
    .expect("an edit");
    let answer = parsed(&exits(
        "pruned alone",
        &sandbox,
        &["task", "status", "set", "plan:P", "queued", "--json"],
        0,
    ));
    assert_eq!(answer["delivered"][0]["pruned"], json!(["plan:GONE"]));
    assert_eq!(
        item(&sandbox, "tickets:T")["delivered_by"],
        json!(["plan:P"])
    );
}

#[test]
fn a_dropped_ticket_is_released_over_whatever_deliverers_remain() {
    for (ticket, other, said, after) in [
        ("queued", None, "written queued->todo", "todo"),
        ("in progress", None, "written in-progress->todo", "todo"),
        ("todo", None, "unchanged todo", "todo"),
        ("draft", None, "left draft", "draft"),
        ("backlog", None, "left backlog", "backlog"),
        ("someday", None, "left unknown", "unknown"),
        ("done", None, "left done", "done"),
        ("cancelled", None, "left cancelled", "cancelled"),
        (
            "in progress",
            Some("in progress"),
            "unchanged in-progress",
            "in-progress",
        ),
    ] {
        let who = format!("a {ticket} ticket beside {other:?}");
        let sandbox = Sandbox::new();
        let plan = folder(
            &sandbox,
            "plan",
            &[("tasks/D.md", "---\ntitle: D\nstatus: in progress\n---\n")],
        );
        let mut named = vec!["\"board:D\""];
        let mut others = Vec::new();
        if let Some(status) = other {
            named.push("\"plan:O\"");
            others.push((
                "tasks/O.md".to_owned(),
                format!("---\ntitle: O\nstatus: {status}\ndelivers: [\"tickets:T\"]\n---\n"),
            ));
        }
        for (relative, text) in &others {
            std::fs::write(plan.join(relative), text).expect("another deliverer");
        }
        let tickets = folder(
            &sandbox,
            "tickets",
            &[(
                "tasks/T.md",
                &format!(
                    "---\ntitle: T\nstatus: {ticket}\ndelivered_by: [{}]\n---\n",
                    named.join(", ")
                ),
            )],
        );
        let board = folder(
            &sandbox,
            "board",
            &[(
                "tasks/D.md",
                "---\ntitle: D\nstatus: in progress\nmetadata:\n  onetaskgraph.origin: plan:D\ndelivers: [\"tickets:T\"]\n---\n",
            )],
        );
        sandbox.project_document(&document(&json!({
            "plan": markdown(&plan), "tickets": markdown(&tickets), "board": markdown(&board),
        })));
        let copied = parsed(&exits(
            &who,
            &sandbox,
            &["task", "copy", "plan:D", "--to", "board", "--json"],
            0,
        ));
        assert_eq!(
            entries_of(&copied),
            [format!("tickets:T board:D {said}")],
            "{who}: {copied:#}"
        );
        assert_eq!(category(&sandbox, "tickets:T"), after, "{who}");
        let remaining = item(&sandbox, "tickets:T")["delivered_by"].clone();
        assert_eq!(
            remaining,
            if other.is_some() {
                json!(["plan:O"])
            } else {
                Value::Null
            },
            "{who}: the back-reference is gone"
        );
    }
}

#[test]
fn a_markdown_task_keeps_a_github_ticket_in_step_through_a_copy_and_status_sets() {
    let sandbox = Sandbox::new();
    let plan = folder(
        &sandbox,
        "plan",
        &[
            ("projects/P.md", "---\ntitle: Plan\nstatus: todo\n---\n"),
            (
                "tasks/A.md",
                "---\ntitle: A\nstatus: queued\nproject: P\ndelivers: [\"gh:T-3\"]\n---\n",
            ),
        ],
    );
    let notes = folder(&sandbox, "notes", &[]);
    let (config, board) = github_projects_with_board(&sandbox);
    sandbox.project_document(&document(&json!({
        "plan": markdown(&plan),
        "notes": markdown(&notes),
        "gh": {"plugin": "github-projects", "config": config},
    })));
    let body = board.body("T-3");

    let from = board.served().len();
    let copied = parsed(&exits(
        "the copy",
        &sandbox,
        &["project", "copy", "plan:P", "--to", "notes", "--json"],
        0,
    ));
    assert_eq!(
        entries_of(&copied),
        ["gh:T-3 notes:A written todo->queued"],
        "{copied:#}"
    );
    let served = mutations(&board, from);
    assert_eq!(
        served
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        ["updateIssue", "updateProjectV2ItemFieldValue"],
        "{served:#?}"
    );
    let slot = format!(
        "{}\n\n<!-- onetaskgraph.metadata\n{{\"onetaskgraph.delivered_by\":[\"notes:A\"]}}\n-->",
        body.as_str().expect("a body")
    );
    assert_eq!(
        board.body("T-3"),
        json!(slot),
        "the body changed only inside its metadata block"
    );
    let ticket = item(&sandbox, "gh:T-3");
    assert_eq!(
        ticket["status"],
        json!({"category": "queued", "name": "Queued"})
    );
    assert_eq!(ticket["delivered_by"], json!(["notes:A"]));
    assert!(
        ticket["metadata"]
            .get("onetaskgraph.delivered_by")
            .is_none(),
        "{ticket:#}"
    );
    assert_eq!(ticket["content"], "unrelated");

    let moved = parsed(&exits(
        "in progress",
        &sandbox,
        &["task", "status", "set", "notes:A", "in-progress", "--json"],
        0,
    ));
    assert_eq!(
        entries_of(&moved),
        ["gh:T-3 notes:A written queued->in-progress"]
    );
    assert_eq!(
        item(&sandbox, "gh:T-3")["status"],
        json!({"category": "in-progress", "name": "Doing"})
    );

    let from = board.served().len();
    let finished = parsed(&exits(
        "done",
        &sandbox,
        &["task", "status", "set", "notes:A", "done", "--json"],
        0,
    ));
    assert_eq!(
        entries_of(&finished),
        ["gh:T-3 notes:A written in-progress->done"]
    );
    let closed = mutations(&board, from);
    assert_eq!(closed.len(), 1, "{closed:#?}");
    assert_eq!(
        closed[0].1["input"]["stateInput"],
        json!({"value": "CLOSED", "stateReason": "COMPLETED"})
    );
    assert_eq!(category(&sandbox, "gh:T-3"), "done");
}

#[test]
fn a_ticket_the_destination_refuses_is_failed_while_the_deliverers_write_lands() {
    let sandbox = Sandbox::new();
    let plan = folder(
        &sandbox,
        "plan",
        &[(
            "tasks/P.md",
            "---\ntitle: P\nstatus: todo\ndelivers: [\"gh:T-3\"]\n---\n",
        )],
    );
    let (config, board) = github_projects_with_board(&sandbox);
    board.without_option("Queued");
    sandbox.project_document(&document(&json!({
        "plan": markdown(&plan),
        "gh": {"plugin": "github-projects", "config": config},
    })));
    let output = exits(
        "a board lacking Queued",
        &sandbox,
        &["task", "status", "set", "plan:P", "queued", "--json"],
        4,
    );
    let answer = parsed(&output);
    validates(
        &bundle(&sandbox),
        "TaskStatusSet",
        &answer,
        "a status set whose ticket failed",
    );
    assert_eq!(answer["status"]["category"], "queued");
    let entry = &answer["delivered"][0];
    assert_eq!(entry["outcome"], "failed", "{answer:#}");
    assert_eq!(entry["from"], "todo");
    assert_eq!(entry["failure"]["class"], "refused");
    assert_eq!(entry["failure"]["source"], "gh");
    assert!(
        entry["failure"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("\"Queued\"")),
        "{entry:#}"
    );
    assert!(entry.get("to").is_none());
    assert_eq!(
        category(&sandbox, "plan:P"),
        "queued",
        "the deliverer's own write reads back"
    );
    assert_eq!(category(&sandbox, "gh:T-3"), "todo");
    let said = stderr(&output);
    assert!(
        said.contains("task gh:T-3 could not be kept in step with plan:P")
            && said.contains("next:"),
        "{said}"
    );

    let rendered = stdout(&exits(
        "in words",
        &sandbox,
        &["task", "status", "set", "plan:P", "queued"],
        4,
    ));
    assert!(
        rendered.contains("delivered gh:T-3 by plan:P: failed at todo:"),
        "{rendered}"
    );
}

#[test]
fn a_github_task_holds_its_delivers_in_its_metadata_block_and_reports_them_qualified() {
    let sandbox = Sandbox::new();
    let plan = folder(
        &sandbox,
        "plan",
        &[(
            "tasks/A.md",
            "---\ntitle: A\nstatus: todo\ndelivers: [\"gh:T-3\", \"tickets:T-1\"]\n---\nDo A.\n",
        )],
    );
    let tickets = folder(
        &sandbox,
        "tickets",
        &[("tasks/T-1.md", "---\ntitle: Ticket\nstatus: todo\n---\n")],
    );
    let (config, board) = github_projects_with_board(&sandbox);
    sandbox.project_document(&document(&json!({
        "plan": markdown(&plan),
        "tickets": markdown(&tickets),
        "gh": {"plugin": "github-projects", "config": config},
    })));
    let copied = parsed(&exits(
        "the copy onto the board",
        &sandbox,
        &["task", "copy", "plan:A", "--to", "gh", "--json"],
        0,
    ));
    let landed = copied["items"][0]["destination"]
        .as_str()
        .expect("a destination id")
        .to_owned();
    let native = landed.strip_prefix("gh:").expect("a board id");
    assert_eq!(
        entries_of(&copied),
        [
            format!("gh:T-3 {landed} unchanged todo"),
            format!("tickets:T-1 {landed} unchanged todo")
        ],
        "{copied:#}"
    );

    let held = board.body(native);
    assert!(
        held.as_str().is_some_and(
            |body| body.starts_with("Do A.\n\n<!-- onetaskgraph.metadata\n")
                && body.contains("\"onetaskgraph.delivers\":[\"gh:T-3\",\"tickets:T-1\"]")
        ),
        "the board keeps the list in the issue's metadata block: {held}"
    );
    let task = item(&sandbox, &landed);
    assert_eq!(task["delivers"], json!(["gh:T-3", "tickets:T-1"]));
    let metadata = task["metadata"].as_object().expect("metadata");
    assert!(
        !metadata.contains_key("onetaskgraph.delivers")
            && !metadata.contains_key("onetaskgraph.delivered_by"),
        "neither key is free metadata: {task:#}"
    );
    let ticket = item(&sandbox, "gh:T-3");
    assert_eq!(ticket["delivered_by"], json!([landed]));
    assert!(
        !ticket["metadata"]
            .as_object()
            .is_some_and(|held| held.contains_key("onetaskgraph.delivered_by"))
    );
    assert_eq!(
        item(&sandbox, "tickets:T-1")["delivered_by"],
        json!([landed])
    );
}
