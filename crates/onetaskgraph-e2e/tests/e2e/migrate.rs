//! `sources migrate` on a source whose plugin has only ever written one stored form: a folder of
//! Markdown, and a plugin served over the stdio protocol that declares no such write. Each is
//! reported as holding nothing to migrate, and is sent nothing. A Linear source's migration is
//! the Linear suite's (`collapsed_slot.rs` in `onetaskgraph-linear-e2e`).

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::document_store::store_at;
use crate::fixtures::document;

/// Every file under `root`, by its path, with its bytes.
fn tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut files = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory).expect("a readable folder") {
            let path = entry.expect("an entry").path();
            if path.is_dir() {
                pending.push(path);
            } else {
                files.insert(
                    path.to_string_lossy().into_owned(),
                    std::fs::read(&path).expect("a readable file"),
                );
            }
        }
    }
    files
}

/// `sources migrate <source>`, with `extra` after it, exiting 0: its JSON answer.
fn migrated(sandbox: &Sandbox, source: &str, extra: &[&str]) -> Value {
    let output = sandbox
        .command()
        .args(["sources", "migrate", source, "--json"])
        .args(extra)
        .assert()
        .get_output()
        .clone();
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}{}",
        stdout(&output),
        stderr(&output)
    );
    serde_json::from_str(&stdout(&output)).expect("a migration report")
}

/// What a source with nothing to migrate answers.
fn nothing(source: &str, dry_run: bool) -> Value {
    json!({"source": source, "dry_run": dry_run, "rewritten": [], "current": 0, "failed": []})
}

#[test]
fn a_folder_of_markdown_has_nothing_to_migrate_and_is_written_nothing() {
    let sandbox = Sandbox::new();
    let plan = sandbox.subdirectory("plan");
    for kind in ["tasks", "projects", "documents"] {
        std::fs::create_dir_all(plan.join(kind)).expect("a folder");
    }
    // An item whose content quotes the comment slot a Linear source once wrote: a folder of
    // Markdown keeps its metadata in front matter, so that text is content and nothing to move.
    std::fs::write(
        plan.join("tasks/T-1.md"),
        "---\ntitle: Build it\nstatus: todo\nmetadata: {\"caller.key\": 1}\n---\n\
         Quoting <!-- onetaskgraph.metadata `{}` -->\n",
    )
    .expect("a task");
    sandbox.project_document(&document(&json!({
        "plan": {"plugin": "local-md", "config": {"root": plan}},
    })));
    let before = tree(&plan);
    assert_eq!(
        migrated(&sandbox, "plan", &["--dry-run"]),
        nothing("plan", true)
    );
    assert_eq!(migrated(&sandbox, "plan", &[]), nothing("plan", false));
    assert_eq!(tree(&plan), before, "nothing in the folder moved");
    // And the human form says so in one line.
    let output = sandbox
        .command()
        .args(["sources", "migrate", "plan"])
        .assert()
        .success()
        .get_output()
        .clone();
    assert_eq!(
        stdout(&output),
        "plan: 0 rewritten, 0 already current, 0 failed\n"
    );
    // A source nothing configures is refused naming the configured ones, before anything runs.
    let output = sandbox
        .command()
        .args(["sources", "migrate", "elsewhere"])
        .assert()
        .failure()
        .get_output()
        .clone();
    assert!(
        stderr(&output).contains("elsewhere") && stderr(&output).contains("plan"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn a_stdio_plugin_declaring_no_migration_is_never_asked_and_sent_no_write() {
    let sandbox = Sandbox::new();
    let store = sandbox.subdirectory("store").join("documents.json");
    std::fs::write(
        &store,
        serde_json::to_string_pretty(&json!({"documents": [{
            "id": "D-1", "title": "Design", "content": "Body.\n\n<!-- onetaskgraph.metadata `{}` -->",
            "labels": [], "metadata": {"caller.key": 1},
        }]}))
        .expect("JSON"),
    )
    .expect("a store");
    let log = sandbox.subdirectory("log").join("methods.log");
    sandbox.project_document(&document(&json!({
        "store": store_at(&store, Some(&log), "native"),
    })));
    let before = std::fs::read(&store).expect("the store");
    assert_eq!(
        migrated(&sandbox, "store", &["--dry-run"]),
        nothing("store", true)
    );
    assert_eq!(migrated(&sandbox, "store", &[]), nothing("store", false));
    assert_eq!(
        std::fs::read(&store).expect("the store"),
        before,
        "the store holds what it held"
    );
    // The peer was started, and asked nothing but its handshake, once per command: no read, no
    // write, and no method its handshake did not declare.
    let asked = std::fs::read_to_string(&log).expect("the peer's log");
    assert_eq!(
        asked.lines().collect::<Vec<_>>(),
        ["initialize", "initialize"],
        "{asked}"
    );
}
