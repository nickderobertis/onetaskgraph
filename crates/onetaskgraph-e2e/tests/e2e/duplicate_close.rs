//! Closing a task as a duplicate of another, through the binary, on a folder of Markdown — and
//! the refusal a stdio plugin whose handshake does not declare duplicate closes is owed.
//!
//! A folder has no native duplicate, so a duplicate close there is `cancelled` and the typed
//! `duplicate` link the store records at both ends; `task relations` then names the survivor.
//! A GitHub board's and a Linear source's native closes are their own suites'.

use std::path::Path;

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::document_store::store_at;
use crate::fixtures::document;

fn ok(sandbox: &Sandbox, arguments: &[&str]) -> Value {
    let output = sandbox
        .command()
        .args(arguments)
        .assert()
        .get_output()
        .clone();
    assert_eq!(
        output.status.code(),
        Some(0),
        "`onetaskgraph {}` failed\n{}",
        arguments.join(" "),
        stderr(&output)
    );
    serde_json::from_str(&stdout(&output)).expect("JSON")
}

/// A folder of Markdown holding `T-1`, `T-2` and `T-3`, at `todo`.
fn folder(sandbox: &Sandbox) -> std::path::PathBuf {
    let root = sandbox.subdirectory("plan");
    std::fs::create_dir_all(root.join("tasks")).unwrap();
    for n in 1..=3 {
        std::fs::write(
            root.join(format!("tasks/T-{n}.md")),
            format!("---\ntitle: Task {n}\nstatus: todo\n---\nTask {n}.\n"),
        )
        .unwrap();
    }
    sandbox.project_document(&document(&json!({
        "plan": {"plugin": "local-md", "config": {"root": root}},
    })));
    root
}

fn read(root: &Path, id: &str) -> String {
    std::fs::read_to_string(root.join(format!("tasks/{id}.md"))).unwrap()
}

#[test]
fn a_folder_closes_a_task_cancelled_with_its_duplicate_link_at_both_ends() {
    let sandbox = Sandbox::new();
    let root = folder(&sandbox);
    let answer = ok(
        &sandbox,
        &[
            "--json",
            "task",
            "status",
            "set",
            "plan:T-1",
            "cancelled",
            "--duplicate-of",
            "plan:T-2",
        ],
    );
    assert_eq!(answer["status"]["category"], "cancelled", "{answer:#}");
    let shown = ok(&sandbox, &["--json", "task", "show", "plan:T-1"]);
    assert_eq!(shown["items"][0]["item"]["status"]["category"], "cancelled");
    let relations = ok(
        &sandbox,
        &["task", "relations", "plan:T-1", "plan:T-2", "--json"],
    );
    assert_eq!(
        relations["items"][0]["links"],
        json!([{"type": "duplicate", "direction": "out", "other": "plan:T-2",
                "other_end": "present"}]),
        "{relations:#}"
    );
    assert_eq!(
        relations["items"][1]["links"],
        json!([{"type": "duplicate", "direction": "in", "other": "plan:T-1",
                "other_end": "present"}]),
        "{relations:#}"
    );
    // A repeat writes nothing: both files stay byte for byte.
    let (one, two) = (read(&root, "T-1"), read(&root, "T-2"));
    ok(
        &sandbox,
        &[
            "--json",
            "task",
            "update",
            "plan:T-1",
            "--duplicate-of",
            "plan:T-2",
        ],
    );
    assert_eq!((read(&root, "T-1"), read(&root, "T-2")), (one, two));
    // And `update-many` closes another the same way.
    let batch = json!({"schema_version": 1, "updates": [
        {"item": "plan:T-3", "status": "cancelled", "duplicate_of": "plan:T-2"},
    ]});
    let output = sandbox
        .command()
        .args(["task", "update-many", "plan", "--input", "-", "--json"])
        .write_stdin(batch.to_string())
        .assert()
        .success()
        .get_output()
        .clone();
    let answer: Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(answer["results"][0]["outcome"], "written", "{answer:#}");
    let relations = ok(&sandbox, &["task", "relations", "plan:T-3", "--json"]);
    assert_eq!(relations["items"][0]["links"][0]["type"], "duplicate");
    assert!(read(&root, "T-3").contains("status: cancelled"));
}

#[test]
fn a_refused_duplicate_close_writes_nothing_to_a_folder() {
    let sandbox = Sandbox::new();
    let root = folder(&sandbox);
    let before: Vec<String> = (1..=3).map(|n| read(&root, &format!("T-{n}"))).collect();
    for (arguments, said) in [
        (
            vec![
                "task",
                "status",
                "set",
                "plan:T-1",
                "done",
                "--duplicate-of",
                "plan:T-2",
            ],
            "cancelled",
        ),
        (
            vec![
                "task",
                "update",
                "plan:T-1",
                "--status",
                "in-progress",
                "--duplicate-of",
                "plan:T-2",
            ],
            "cancelled",
        ),
        (
            vec![
                "task",
                "update",
                "plan:T-1",
                "--duplicate-of",
                "elsewhere:T-2",
            ],
            "elsewhere:T-2",
        ),
    ] {
        let output = sandbox
            .command()
            .args(&arguments)
            .assert()
            .get_output()
            .clone();
        assert_ne!(output.status.code(), Some(0), "{arguments:?}");
        assert!(
            stderr(&output).contains(said),
            "{arguments:?}: {}",
            stderr(&output)
        );
    }
    let after: Vec<String> = (1..=3).map(|n| read(&root, &format!("T-{n}"))).collect();
    assert_eq!(after, before, "nothing was written");
}

#[test]
fn a_stdio_peer_not_declaring_duplicate_closes_is_refused_the_field_before_anything_is_sent() {
    let sandbox = Sandbox::new();
    let store = sandbox.subdirectory("store").join("documents.json");
    std::fs::write(&store, r#"{"documents": []}"#).unwrap();
    let log = sandbox.subdirectory("store").join("asked.log");
    sandbox.project_document(&document(&json!({
        "peer": store_at(&store, Some(&log), "native"),
    })));
    let output = sandbox
        .command()
        .args(["task", "update", "peer:T-1", "--duplicate-of", "peer:T-2"])
        .assert()
        .get_output()
        .clone();
    assert_ne!(output.status.code(), Some(0));
    assert!(
        stderr(&output).contains("duplicate_closes") && stderr(&output).contains("duplicate_of"),
        "{}",
        stderr(&output)
    );
    assert_eq!(
        std::fs::read_to_string(&log)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        ["initialize"],
        "the peer was sent nothing"
    );
}
