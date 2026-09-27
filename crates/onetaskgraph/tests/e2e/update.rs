//! `task update`, driven the way a user drives it.
//!
//! Every journey spawns the compiled binary and asserts on its exit code, stdout and stderr,
//! and on what the store holds afterwards: every source kind of the shared table, a folder of
//! Markdown read back byte for byte over the in-process boundary and the stdio host alike, and
//! the loopback GitHub board and Linear workspace, whose every request is observed. The engine
//! half, over `in-memory` and `local-md` as a Rust caller reaches it, is
//! `crates/onetaskgraph-core/tests/update.rs`.

use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::{Value, json};

use crate::common::{SOURCE_BOUNDARIES, Sandbox, stderr, stdout};
use crate::fixtures::{ROWS, Row, SOURCE, document, github_projects_with_board, qualified};
use crate::machine::{bundle, validates};

/// The rows whose sources outlive one invocation, so a later one reads what an update wrote.
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

fn shown(who: &str, sandbox: &Sandbox, id: &str) -> Value {
    answered(who, sandbox, &["--json", "task", "show", id])["items"][0]["item"].clone()
}

/// The fields of a task `written` names, in the wire's spelling.
fn written(answer: &Value) -> Vec<&str> {
    answer["written"]
        .as_array()
        .expect("an answer says what it wrote")
        .iter()
        .map(|field| field.as_str().expect("a field name"))
        .collect()
}

/// Every member of a task no update in this file names, as one comparable value.
fn unnamed(task: &Value) -> Vec<(&'static str, Value)> {
    [
        "id",
        "key",
        "content",
        "labels",
        "project",
        "url",
        "location",
        "repositories",
        "delivers",
        "delivered_by",
    ]
    .into_iter()
    .map(|member| (member, task.get(member).cloned().unwrap_or(Value::Null)))
    .collect()
}

fn host(row: &Row) -> Sandbox {
    let sandbox = Sandbox::new();
    sandbox.project_document(&row.document(&sandbox));
    sandbox
}

#[test]
fn every_row_updates_the_fields_it_names_and_nothing_else() {
    for row in ROWS.iter().filter(|row| row.fixture.complete_dataset) {
        let who = row.name;
        let sandbox = host(row);
        let schema = bundle(&sandbox);
        let id = qualified(SOURCE, "T-1");
        let before = shown(who, &sandbox, &id);

        let answer = answered(
            who,
            &sandbox,
            &[
                "--json",
                "task",
                "update",
                &id,
                "--title",
                "Alpha engine, renamed",
                "--status",
                "in-progress",
                "--priority",
                "low",
            ],
        );
        validates(
            &schema,
            "TaskUpdated",
            &answer,
            &format!("{who} task update"),
        );
        assert_eq!(answer["id"], id, "{who}");
        assert_eq!(written(&answer), ["title", "status", "priority"], "{who}");
        let task = &answer["task"];
        assert_eq!(task["title"], "Alpha engine, renamed", "{who}");
        assert_eq!(task["status"]["category"], "in-progress", "{who}");
        assert_eq!(task["priority"], "low", "{who}");
        assert_eq!(unnamed(task), unnamed(&before), "{who}");
        assert_eq!(task["metadata"], before["metadata"], "{who}");

        if !PERSISTENT.contains(&row.plugin) {
            continue;
        }
        // A later invocation reads what this one wrote, and every field it did not name as
        // it was.
        let after = shown(who, &sandbox, &id);
        assert_eq!(after["title"], "Alpha engine, renamed", "{who}");
        assert_eq!(after["status"]["category"], "in-progress", "{who}");
        assert_eq!(after["priority"], "low", "{who}");
        assert_eq!(unnamed(&after), unnamed(&before), "{who}");
        assert_eq!(after["metadata"], before["metadata"], "{who}");

        // The same update again finds every field already holding its value.
        let again = answered(
            who,
            &sandbox,
            &[
                "--json",
                "task",
                "update",
                &id,
                "--title",
                "Alpha engine, renamed",
                "--status",
                "in-progress",
                "--priority",
                "low",
            ],
        );
        assert!(written(&again).is_empty(), "{who}: {again}");
    }
}

/// A folder of Markdown holding a task with a metadata block, the task it delivers, and a
/// second task it could deliver instead.
fn folder(sandbox: &Sandbox) -> PathBuf {
    let root = sandbox.subdirectory("work");
    std::fs::create_dir_all(root.join("tasks")).expect("a folder");
    for (id, text) in [
        ("T-1", held()),
        (
            "T-2",
            "---\ntitle: Delivered\nstatus: todo\ndelivered_by: [\"work:T-1\"]\n---\n".to_owned(),
        ),
        ("T-3", "---\ntitle: Spare\nstatus: todo\n---\n".to_owned()),
    ] {
        std::fs::write(root.join(format!("tasks/{id}.md")), text).expect("a task");
    }
    root
}

/// What `T-1`'s file holds before anything is updated.
fn held() -> String {
    "---\ntitle: One\nstatus: todo\nmetadata:\n  onepipeline.claim: r-1\n  \"onepipeline.kept\": [1]\n\
     delivers: [\"T-2\"]\n---\nThe body.\n\n## Comments\n\n<!-- onetaskgraph:comment id=\"C-1\" \
     author=\"ada\" created_at=\"2026-09-01T00:00:00Z\" updated_at=\"2026-09-01T00:00:00Z\" -->\n\
     ### ada — 2026-09-01T00:00:00Z\n\nFirst.\n\n<!-- /onetaskgraph:comment -->\n"
        .to_owned()
}

fn read(root: &Path, id: &str) -> String {
    std::fs::read_to_string(root.join(format!("tasks/{id}.md"))).expect("the task reads")
}

#[test]
fn a_folder_of_markdown_changes_by_exactly_the_entries_an_update_names() {
    for boundary in SOURCE_BOUNDARIES {
        let who = format!("{boundary:?}");
        let sandbox = Sandbox::new();
        let root = folder(&sandbox);
        sandbox.project_document(&document(&json!({
            "work": boundary.source("local-md", json!({"root": root, "status_mapping": {
                "todo": "todo", "in progress": "in-progress", "failed": "cancelled",
                "done": "done"}})),
        })));
        let schema = bundle(&sandbox);

        // A settlement: a status under a word of its own, three keys set and one removed.
        let answer = answered(
            &who,
            &sandbox,
            &[
                "--json",
                "task",
                "update",
                "work:T-1",
                "--status",
                "cancelled",
                "--status-name",
                "failed",
                "--metadata",
                r#"onepipeline.settlement={"outcome":"failed"}"#,
                "--metadata",
                "onepipeline.kept=[1]",
                "--metadata",
                r#"onepipeline.landing="none""#,
                "--remove-metadata",
                "onepipeline.claim",
            ],
        );
        validates(
            &schema,
            "TaskUpdated",
            &answer,
            &format!("{who} task update"),
        );
        assert_eq!(written(&answer), ["status", "metadata"], "{who}");
        // A folder of Markdown meters nothing, so the answer says nothing of what it spent
        // rather than claiming it spent none.
        assert!(answer.get("spent").is_none(), "{who}: {answer}");
        assert_eq!(
            answer["task"]["status"],
            json!({"category": "cancelled", "name": "failed"}),
            "{who}"
        );
        assert_eq!(
            read(&root, "T-1"),
            held()
                .replace("status: todo\n", "status: failed\n")
                .replace(
                    "  onepipeline.claim: r-1\n  \"onepipeline.kept\": [1]\n",
                    "  \"onepipeline.kept\": [1]\n  \"onepipeline.landing\": \"none\"\n  \
                     \"onepipeline.settlement\": {\"outcome\":\"failed\"}\n",
                ),
            "{who}: only the status line and the named metadata entries moved"
        );
        // The status was named, so the task it delivers was re-evaluated: a cancelled
        // deliverer releases its claim, and a delivered task already at `todo` stays there.
        assert_eq!(
            answer["delivered"],
            json!([{"ticket": "work:T-2", "deliverer": "work:T-1", "outcome": "unchanged",
                    "from": "todo"}]),
            "{who}"
        );

        // The same update again writes nothing, and the file is the very bytes it was.
        let settled = read(&root, "T-1");
        let again = answered(
            &who,
            &sandbox,
            &[
                "--json",
                "task",
                "update",
                "work:T-1",
                "--status",
                "cancelled",
                "--status-name",
                "failed",
                "--metadata",
                "onepipeline.kept=[1]",
                "--remove-metadata",
                "onepipeline.claim",
            ],
        );
        assert!(written(&again).is_empty(), "{who}: {again}");
        assert_eq!(read(&root, "T-1"), settled, "{who}");

        // One changed field among several unchanged ones is the only one reported.
        let human = exits(
            &who,
            &sandbox,
            &[
                "task",
                "update",
                "work:T-1",
                "--title",
                "One",
                "--status",
                "cancelled",
                "--status-name",
                "failed",
                "--priority",
                "high",
            ],
            0,
        );
        assert_eq!(
            stdout(&human),
            "id:       work:T-1\nwritten:  priority\nstatus:   cancelled (failed)\n\
             delivered work:T-2 by work:T-1: unchanged at todo\n",
            "{who}"
        );
    }
}

#[test]
fn an_update_keeps_the_tasks_it_delivers_in_step_only_when_it_names_them_or_its_status() {
    let sandbox = Sandbox::new();
    let root = folder(&sandbox);
    sandbox.project_document(&document(&json!({
        "work": {"plugin": "local-md", "config": {"root": root}},
    })));

    // Naming the status moves the task it delivers with it.
    let moved = answered(
        "status",
        &sandbox,
        &[
            "--json",
            "task",
            "update",
            "work:T-1",
            "--status",
            "in-progress",
        ],
    );
    assert_eq!(
        moved["delivered"],
        json!([{"ticket": "work:T-2", "deliverer": "work:T-1", "outcome": "written",
                "from": "todo", "to": "in-progress"}])
    );
    assert_eq!(
        shown("status", &sandbox, "work:T-2")["status"]["category"],
        "in-progress"
    );

    // Naming neither re-evaluates nothing.
    let quiet = answered(
        "title",
        &sandbox,
        &[
            "--json",
            "task",
            "update",
            "work:T-1",
            "--title",
            "One, renamed",
        ],
    );
    assert_eq!(quiet["delivered"], json!([]));

    // Naming the list keeps both ends in step: the new ticket gains the deliverer, and the one
    // it dropped loses it and is released.
    let swapped = answered(
        "delivers",
        &sandbox,
        &[
            "--json",
            "task",
            "update",
            "work:T-1",
            "--delivers",
            "work:T-3",
        ],
    );
    assert_eq!(written(&swapped), ["delivers"]);
    let tickets: Vec<(&str, &str)> = swapped["delivered"]
        .as_array()
        .expect("entries")
        .iter()
        .map(|entry| {
            (
                entry["ticket"].as_str().expect("a ticket"),
                entry["outcome"].as_str().expect("an outcome"),
            )
        })
        .collect();
    assert_eq!(tickets, [("work:T-3", "written"), ("work:T-2", "written")]);
    let dropped = shown("delivers", &sandbox, "work:T-2");
    assert_eq!(dropped["status"]["category"], "todo");
    assert!(dropped.get("delivered_by").is_none(), "{dropped}");
    let gained = shown("delivers", &sandbox, "work:T-3");
    assert_eq!(gained["delivered_by"], json!(["work:T-1"]));
    assert_eq!(gained["status"]["category"], "in-progress");

    // And `--no-delivers` releases it again.
    let released = answered(
        "none",
        &sandbox,
        &["--json", "task", "update", "work:T-1", "--no-delivers"],
    );
    assert_eq!(released["delivered"][0]["ticket"], "work:T-3");
    assert!(
        shown("none", &sandbox, "work:T-3")
            .get("delivered_by")
            .is_none()
    );
}

#[test]
fn every_refusal_an_update_owes_names_the_problem_and_writes_nothing() {
    let sandbox = Sandbox::new();
    let root = folder(&sandbox);
    sandbox.project_document(&document(&json!({
        "work": {"plugin": "local-md", "config": {"root": root}},
    })));
    let before = read(&root, "T-1");

    let empty = exits("empty", &sandbox, &["task", "update", "work:T-1"], 2);
    assert!(
        stderr(&empty).contains("names no field to write"),
        "{}",
        stderr(&empty)
    );

    let both = exits(
        "both",
        &sandbox,
        &[
            "task",
            "update",
            "work:T-1",
            "--metadata",
            "onepipeline.claim=\"r-2\"",
            "--remove-metadata",
            "onepipeline.claim",
        ],
        1,
    );
    assert!(
        stderr(&both).contains("both sets and removes the metadata key onepipeline.claim"),
        "{}",
        stderr(&both)
    );

    let missing = exits(
        "missing",
        &sandbox,
        &["task", "update", "work:T-9", "--title", "x"],
        1,
    );
    assert!(
        stderr(&missing).contains("no task with the id work:T-9"),
        "{}",
        stderr(&missing)
    );

    let unqualified = exits(
        "bare",
        &sandbox,
        &["task", "update", "T-1", "--title", "x"],
        1,
    );
    assert!(
        stderr(&unqualified).contains("next:"),
        "{}",
        stderr(&unqualified)
    );

    let reserved = exits(
        "reserved",
        &sandbox,
        &[
            "task",
            "update",
            "work:T-1",
            "--remove-metadata",
            "onetaskgraph.origin",
        ],
        1,
    );
    assert!(
        stderr(&reserved).contains("which this product owns"),
        "{}",
        stderr(&reserved)
    );

    let conflicting = exits(
        "conflict",
        &sandbox,
        &[
            "task",
            "update",
            "work:T-1",
            "--delivers",
            "work:T-3",
            "--no-delivers",
        ],
        2,
    );
    assert!(
        stderr(&conflicting).contains("cannot be used with"),
        "{}",
        stderr(&conflicting)
    );

    assert_eq!(read(&root, "T-1"), before, "no refusal wrote anything");
}

/// Every request the board served since `from`, by what it was: the document's first word
/// and, for a mutation, its operation.
fn requests_since(board: &crate::fixtures::GitHubBoardFields, from: usize) -> Vec<String> {
    board.served()[from..]
        .iter()
        .map(|(query, _)| {
            let body = query.trim_start();
            if let Some(rest) = body.strip_prefix("mutation") {
                let after = rest
                    .split_once('{')
                    .map_or(rest, |(_, after)| after)
                    .trim_start();
                after
                    .split(|c: char| !c.is_alphanumeric())
                    .next()
                    .unwrap_or_default()
                    .to_owned()
            } else {
                "read".to_owned()
            }
        })
        .collect()
}

#[test]
fn a_board_update_is_one_read_one_body_update_and_one_status_write() {
    let sandbox = Sandbox::new();
    let (config, board) = github_projects_with_board(&sandbox);
    sandbox.project_document(&document(&json!({
        SOURCE: {"plugin": "github-projects", "config": config}
    })));
    let id = qualified(SOURCE, "T-1");
    let before = shown("board", &sandbox, &id);

    let from = board.served().len();
    let answer = answered(
        "board",
        &sandbox,
        &[
            "--json",
            "task",
            "update",
            &id,
            "--status",
            "in-progress",
            "--metadata",
            r#"onepipeline.settlement={"outcome":"landed"}"#,
            "--metadata",
            r#"onepipeline.landing="merged""#,
            "--metadata",
            r#"onepipeline.change_url="https://example.invalid/pull/1""#,
            "--remove-metadata",
            "caller.flags",
        ],
    );
    assert_eq!(
        requests_since(&board, from),
        ["read", "updateIssue", "updateProjectV2ItemFieldValue"],
        "one read of the item, one body update and one status write — nothing else"
    );
    assert_eq!(written(&answer), ["status", "metadata"]);
    // The board meters its own requests, and the answer carries what this one spent.
    assert_eq!(answer["spent"]["requests"], 3, "{answer}");
    let after = shown("board", &sandbox, &id);
    assert_eq!(
        after["status"],
        json!({"category": "in-progress", "name": "Doing"})
    );
    assert_eq!(after["metadata"]["onepipeline.landing"], "merged");
    assert_eq!(after["metadata"]["onepipeline.turn_budget"], 12);
    assert!(after["metadata"].get("caller.flags").is_none(), "{after}");
    assert_eq!(unnamed(&after), unnamed(&before));
    assert_eq!(after["priority"], before["priority"]);
    assert_eq!(after["title"], before["title"]);

    // Naming only what it holds is one read and no mutation at all.
    let from = board.served().len();
    let unchanged = answered(
        "board",
        &sandbox,
        &[
            "--json",
            "task",
            "update",
            &id,
            "--status",
            "in-progress",
            "--metadata",
            r#"onepipeline.landing="merged""#,
            "--remove-metadata",
            "caller.flags",
        ],
    );
    assert!(written(&unchanged).is_empty(), "{unchanged}");
    assert_eq!(requests_since(&board, from), ["read"]);

    // A terminal status named by a word of its own lands on its mapped option and closes,
    // and a changed priority is one field write beside it.
    let from = board.served().len();
    let failed = answered(
        "board",
        &sandbox,
        &[
            "--json",
            "task",
            "update",
            &id,
            "--status",
            "cancelled",
            "--status-name",
            "failed",
            "--priority",
            "urgent",
        ],
    );
    assert_eq!(
        requests_since(&board, from),
        [
            "read",
            "updateProjectV2ItemFieldValue",
            "updateIssue",
            "updateProjectV2ItemFieldValue"
        ]
    );
    assert_eq!(
        failed["task"]["status"],
        json!({"category": "cancelled", "name": "Cancelled"})
    );
    assert_eq!(board.priority("T-1").as_deref(), Some("Urgent"));

    // New visible content goes out in the one body update, the metadata block kept after it.
    let file = sandbox.subdirectory("bodies").join("body.md");
    std::fs::write(&file, "Rewritten by an update.").expect("a body");
    let from = board.served().len();
    let rewritten = answered(
        "board",
        &sandbox,
        &[
            "--json",
            "task",
            "update",
            &id,
            "--body-file",
            file.to_str().expect("a UTF-8 path"),
            "--metadata",
            r#"onepipeline.landing="reverted""#,
        ],
    );
    assert_eq!(written(&rewritten), ["content", "metadata"]);
    assert_eq!(requests_since(&board, from), ["read", "updateIssue"]);
    let body = board.body("T-1");
    assert!(
        body.as_str().is_some_and(
            |body| body.starts_with("Rewritten by an update.\n\n<!-- onetaskgraph.metadata\n")
        ),
        "{body}"
    );
    let after = shown("board", &sandbox, &id);
    assert_eq!(after["content"], "Rewritten by an update.");
    assert_eq!(after["metadata"]["onepipeline.landing"], "reverted");
    assert_eq!(after["metadata"]["onepipeline.turn_budget"], 12);
}

#[test]
fn a_linear_update_sends_one_issue_update_of_what_differs() {
    let row = ROWS
        .iter()
        .find(|row| row.plugin == "linear")
        .expect("the table has a Linear row");
    let sandbox = host(row);
    let id = qualified(SOURCE, "T-1");
    let before = shown("linear", &sandbox, &id);

    let answer = answered(
        "linear",
        &sandbox,
        &[
            "--json",
            "task",
            "update",
            &id,
            "--status",
            "done",
            "--metadata",
            r#"onepipeline.settlement={"outcome":"landed"}"#,
            "--remove-metadata",
            "caller.flags",
        ],
    );
    assert_eq!(written(&answer), ["status", "metadata"]);
    let after = shown("linear", &sandbox, &id);
    assert_eq!(after["status"], json!({"category": "done", "name": "Done"}));
    assert_eq!(
        after["metadata"]["onepipeline.settlement"],
        json!({"outcome": "landed"})
    );
    assert_eq!(after["metadata"]["onepipeline.turn_budget"], 12);
    assert!(after["metadata"].get("caller.flags").is_none(), "{after}");
    assert_eq!(unnamed(&after), unnamed(&before));

    // Named dependencies replace the issue's own, relations and recorded far ends alike, and a
    // later invocation reads the replacement.
    let replaced = answered(
        "linear",
        &sandbox,
        &[
            "--json",
            "task",
            "update",
            &id,
            "--depends-on",
            &qualified(SOURCE, "T-3"),
        ],
    );
    assert_eq!(written(&replaced), ["depends-on"]);
    let far: Vec<String> = answered("linear", &sandbox, &["--json", "task", "deps", &id])["items"]
        .as_array()
        .expect("a page of edges")
        .iter()
        .map(|edge| edge["to"]["id"].as_str().expect("a far end").to_owned())
        .collect();
    assert_eq!(far, [qualified(SOURCE, "T-3")]);

    // A list Linear cannot carry is refused by name.
    let refused = exits(
        "linear",
        &sandbox,
        &[
            "task",
            "update",
            &id,
            "--delivers",
            &qualified(SOURCE, "T-3"),
        ],
        1,
    );
    assert!(
        stderr(&refused).contains("cannot carry delivers"),
        "{}",
        stderr(&refused)
    );
}

/// The far ends of a task's forward edges, as `task deps` reports them.
fn depends_on(sandbox: &Sandbox, id: &str) -> Vec<String> {
    answered("deps", sandbox, &["--json", "task", "deps", id])["items"]
        .as_array()
        .expect("a page of edges")
        .iter()
        .map(|edge| edge["to"]["id"].as_str().expect("a far end").to_owned())
        .collect()
}

#[test]
fn named_dependencies_replace_the_tasks_own_and_none_clears_them() {
    let sandbox = Sandbox::new();
    let root = folder(&sandbox);
    sandbox.project_document(&document(&json!({
        "work": {"plugin": "local-md", "config": {"root": root}},
    })));
    assert!(depends_on(&sandbox, "work:T-1").is_empty());

    let answer = answered(
        "depends-on",
        &sandbox,
        &[
            "--json",
            "task",
            "update",
            "work:T-1",
            "--depends-on",
            "work:T-3",
            "--depends-on",
            "work:T-2",
        ],
    );
    assert_eq!(written(&answer), ["depends-on"]);
    assert_eq!(
        answer["delivered"],
        json!([]),
        "neither status nor delivers was named"
    );
    assert_eq!(depends_on(&sandbox, "work:T-1"), ["work:T-3", "work:T-2"]);
    assert!(
        read(&root, "T-1").contains(
            "depends_on: [{\"id\":\"T-3\",\"kind\":\"blocks\",\"item\":\"task\"},\
             {\"id\":\"T-2\",\"kind\":\"blocks\",\"item\":\"task\"}]\n"
        ),
        "{}",
        read(&root, "T-1")
    );

    // The same set, in another order, is the set the task holds: nothing is written.
    let again = answered(
        "same set",
        &sandbox,
        &[
            "--json",
            "task",
            "update",
            "work:T-1",
            "--depends-on",
            "work:T-2",
            "--depends-on",
            "work:T-3",
        ],
    );
    assert!(written(&again).is_empty(), "{again}");

    let cleared = answered(
        "none",
        &sandbox,
        &["--json", "task", "update", "work:T-1", "--no-depends-on"],
    );
    assert_eq!(written(&cleared), ["depends-on"]);
    assert!(depends_on(&sandbox, "work:T-1").is_empty());
    assert_eq!(
        read(&root, "T-1"),
        held(),
        "clearing them took the entry back out"
    );
}

#[test]
fn a_body_file_that_cannot_be_read_as_text_is_refused_before_anything_is_asked() {
    let sandbox = Sandbox::new();
    let root = folder(&sandbox);
    sandbox.project_document(&document(&json!({
        "work": {"plugin": "local-md", "config": {"root": root}},
    })));
    let bodies = sandbox.subdirectory("bodies");
    let absent = bodies.join("absent.md");
    let refused = exits(
        "absent",
        &sandbox,
        &[
            "task",
            "update",
            "work:T-1",
            "--body-file",
            absent.to_str().expect("a UTF-8 path"),
        ],
        1,
    );
    let said = stderr(&refused);
    assert!(
        said.contains("--body-file")
            && said.contains("could not read it")
            && said.contains("next:"),
        "{said}"
    );

    let binary = bodies.join("binary.md");
    std::fs::write(&binary, [0xff, 0xfe, 0x00]).expect("a file that is not UTF-8");
    let refused = exits(
        "not text",
        &sandbox,
        &[
            "task",
            "update",
            "work:T-1",
            "--body-file",
            binary.to_str().expect("a UTF-8 path"),
        ],
        1,
    );
    let said = stderr(&refused);
    assert!(
        said.contains("is not UTF-8 text") && said.contains("next:"),
        "{said}"
    );
    assert_eq!(read(&root, "T-1"), held(), "a refused body wrote nothing");

    // A readable one replaces the content alone, the comments below it kept byte for byte.
    let good = bodies.join("good.md");
    std::fs::write(&good, "A new body.").expect("a body");
    let answer = answered(
        "good",
        &sandbox,
        &[
            "--json",
            "task",
            "update",
            "work:T-1",
            "--body-file",
            good.to_str().expect("a UTF-8 path"),
        ],
    );
    assert_eq!(written(&answer), ["content"]);
    assert_eq!(
        read(&root, "T-1"),
        held().replace("The body.\n", "A new body.\n")
    );
}

#[test]
fn an_update_that_lands_with_a_delivered_task_out_of_reach_exits_four_and_says_which() {
    let sandbox = Sandbox::new();
    let root = sandbox.subdirectory("work");
    std::fs::create_dir_all(root.join("tasks")).expect("a folder");
    std::fs::write(
        root.join("tasks/P.md"),
        "---\ntitle: Parent\nstatus: todo\ndelivers: [\"nowhere:T-9\"]\n---\n",
    )
    .expect("a task");
    sandbox.project_document(&document(&json!({
        "work": {"plugin": "local-md", "config": {"root": root}},
    })));

    let output = exits(
        "partial",
        &sandbox,
        &[
            "--json",
            "task",
            "update",
            "work:P",
            "--status",
            "in-progress",
        ],
        4,
    );
    let said = stderr(&output);
    assert!(
        said.contains("nowhere:T-9 could not be kept in step with work:P")
            && said.contains("the write itself landed"),
        "{said}"
    );
    let answer: Value = serde_json::from_str(&stdout(&output)).expect("the whole answer");
    assert_eq!(written(&answer), ["status"]);
    assert_eq!(answer["delivered"][0]["outcome"], "failed");
    assert_eq!(answer["delivered"][0]["failure"]["kind"], "unknown-source");
    // The update itself landed.
    assert_eq!(
        shown("partial", &sandbox, "work:P")["status"]["category"],
        "in-progress"
    );
}

#[test]
fn removing_the_only_metadata_key_takes_the_block_with_it_and_a_doubled_key_is_refused() {
    let sandbox = Sandbox::new();
    let root = sandbox.subdirectory("work");
    std::fs::create_dir_all(root.join("tasks")).expect("a folder");
    let sole = "---\ntitle: Sole\nstatus: todo\nmetadata:\n  onepipeline.claim: r-1\n---\nBody.\n";
    let doubled =
        "---\ntitle: Doubled\nstatus: todo\nmetadata:\n  a.b: 1\n  \"a.b\": 2\n---\nBody.\n";
    std::fs::write(root.join("tasks/S.md"), sole).expect("a task");
    std::fs::write(root.join("tasks/D.md"), doubled).expect("a task");
    sandbox.project_document(&document(&json!({
        "work": {"plugin": "local-md", "config": {"root": root}},
    })));

    let answer = answered(
        "sole",
        &sandbox,
        &[
            "--json",
            "task",
            "update",
            "work:S",
            "--remove-metadata",
            "onepipeline.claim",
        ],
    );
    assert_eq!(written(&answer), ["metadata"]);
    assert_eq!(
        read(&root, "S"),
        "---\ntitle: Sole\nstatus: todo\n---\nBody.\n",
        "the block went with its only entry, and nothing else moved"
    );
    assert_eq!(shown("sole", &sandbox, "work:S")["metadata"], json!({}));

    let refused = exits(
        "doubled",
        &sandbox,
        &["task", "update", "work:D", "--remove-metadata", "a.b"],
        1,
    );
    let said = stderr(&refused);
    assert!(
        said.contains("cannot remove the metadata key `a.b`")
            && said.contains("more than once")
            && said.contains("next:"),
        "{said}"
    );
    assert_eq!(read(&root, "D"), doubled, "a refused removal wrote nothing");
}
