//! The sentinel journey: nothing of a user's work is written down anywhere.
//!
//! This is the second of the three mechanisms that hold the invariant `AGENTS.md` states
//! — `deny.toml` refuses every store, index and cache crate, and
//! `crates/onetaskgraph-core/tests/no_reuse.rs` catches an in-process cache a filesystem
//! scan cannot see. This one catches everything else, because it asserts on the
//! *observable effect* rather than on the technique: whatever a future change reaches for
//! to keep a page around, if it lands in a file, this fails and names the file.
//!
//! So `HOME`, every `XDG_*` and the temporary directory are all redirected into one tree
//! this run owns, unique strings are planted where a user's work would be, every verb is
//! driven, and the tree is compared with itself.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::common::{SOURCE_BOUNDARIES, Sandbox, SourceBoundary, stdout};
use crate::fixtures::{document, qualified};
use assert_cmd::Command;
use serde_json::json;

/// Strings that appear nowhere but in this run's source data.
///
/// Distinctive on purpose: a fragment that could occur in a path, a timestamp or a
/// clap-generated help line would make this journey fail for reasons that are not the
/// invariant.
const SENTINELS: [&str; 5] = [
    "sentinel-title-Qb7xK2",
    "sentinel-body-Vn4pR9",
    "sentinel-label-Zt6mW1",
    "sentinel-project-Hs3jL8",
    "sentinel-status-Dy5cF0",
];

/// Every file under `root`, by path, with what it held.
///
/// Contents rather than a modification time: a cache rewritten within the same clock tick
/// would leave the time unchanged, and it is what a file *holds* that this journey is
/// about.
fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut found = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if let Ok(contents) = std::fs::read(&path) {
                found.insert(path, contents);
            }
        }
    }
    found
}

/// The configuration this journey runs against: one source whose every field is a
/// sentinel.
fn planted(boundary: SourceBoundary) -> String {
    document(&json!({
        "work": boundary.source("in-memory", json!({
                "capabilities": {"max_page_size": 2},
                "tasks": [
                    {"id": "T-1", "title": SENTINELS[0], "content": SENTINELS[1],
                     "status": {"category": "todo", "name": SENTINELS[4]},
                     "labels": [{"id": "L-1", "name": SENTINELS[2]}], "project": "P-1"},
                    {"id": "T-2", "title": "second", "content": SENTINELS[1],
                     "status": {"category": "done", "name": "Shipped"}, "labels": []}
                ],
                "projects": [
                    {"id": "P-1", "title": SENTINELS[3], "content": SENTINELS[1],
                     "status": {"category": "in-progress", "name": SENTINELS[4]},
                     "labels": []},
                    {"id": "P-2", "title": "second project", "content": SENTINELS[1],
                     "status": {"category": "todo", "name": "Todo"}, "labels": []}
                ],
                "labels": [{"id": "L-1", "name": SENTINELS[2]}],
                "task_dependencies": [{"from": "T-1", "to": "T-2", "kind": "blocks"}],
                "project_dependencies": [{"from": "P-1", "to": "P-2", "kind": "blocks"}]
        }))
    }))
}

/// Every verb this binary answers, with arguments that make each of them return work.
fn every_verb() -> Vec<Vec<String>> {
    let task = qualified("work", "T-1");
    let project = qualified("work", "P-1");
    let owned = |arguments: &[&str]| arguments.iter().map(|part| (*part).to_owned()).collect();
    vec![
        owned(&["sources", "list"]),
        owned(&["task", "list"]),
        owned(&["task", "list", "--explain", "--json"]),
        owned(&["task", "list", "--label", SENTINELS[2]]),
        owned(&["task", "list", "--status", "todo"]),
        owned(&["task", "list", "--search", SENTINELS[0], "--in", "title"]),
        owned(&["task", "list", "--search", SENTINELS[1], "--in", "content"]),
        owned(&["task", "list", "--no-project"]),
        owned(&["task", "list", "--limit", "1"]),
        owned(&["task", "show", &task]),
        owned(&["task", "deps", &task]),
        vec![
            "task".to_owned(),
            "deps".to_owned(),
            qualified("work", "T-2"),
            "--direction".to_owned(),
            "depended-on-by".to_owned(),
        ],
        owned(&["project", "list"]),
        owned(&["project", "show", &project]),
        owned(&["project", "deps", &project]),
        owned(&["label", "list"]),
        owned(&["search", SENTINELS[1]]),
        owned(&["config", "show"]),
        owned(&["schema"]),
    ]
}

#[test]
fn driving_every_verb_writes_nothing_of_a_users_work_anywhere() {
    for boundary in SOURCE_BOUNDARIES {
        let sandbox = Sandbox::new();
        // Instrumented binaries write LLVM coverage data on exit. Keep that tool-owned
        // output outside the observed tree so the assertion below retains its literal
        // meaning: every file created under the sandbox is an engine write.
        let coverage = tempfile::tempdir().expect("a directory for coverage runtime output");
        let document = sandbox.project_document(&planted(boundary));
        let root = document
            .parent()
            .and_then(Path::parent)
            .expect("the project tree sits under the sandbox root")
            .to_path_buf();

        // Everywhere a program conventionally writes, redirected into this one tree. A cache
        // put anywhere a running process could reach lands here rather than on the machine.
        let mut homes = Vec::new();
        for relative in ["home", "cache", "data", "state", "runtime", "tmp"] {
            let path = root.join(relative);
            std::fs::create_dir_all(&path).expect("a sandboxed directory");
            homes.push(path);
        }

        let before = snapshot(&root);
        assert!(
            before
                .values()
                .any(|contents| { String::from_utf8_lossy(contents).contains(SENTINELS[0]) }),
            "the sentinels must actually be planted, or this journey proves nothing"
        );

        let mut answered = 0;
        for arguments in every_verb() {
            let mut command = Command::new(env!("CARGO_BIN_EXE_onetaskgraph"));
            for (name, _) in std::env::vars() {
                if name.starts_with("ONETASKGRAPH_") {
                    command.env_remove(name);
                }
            }
            let output = command
                .current_dir(sandbox.project())
                .env("HOME", &homes[0])
                .env("XDG_CONFIG_HOME", sandbox.config_home())
                .env("XDG_CACHE_HOME", &homes[1])
                .env("XDG_DATA_HOME", &homes[2])
                .env("XDG_STATE_HOME", &homes[3])
                .env("XDG_RUNTIME_DIR", &homes[4])
                .env("TMPDIR", &homes[5])
                .env("TEMP", &homes[5])
                .env("TMP", &homes[5])
                .env(
                    "LLVM_PROFILE_FILE",
                    coverage.path().join("onetaskgraph-%p-%m.profraw"),
                )
                .args(&arguments)
                .assert()
                .success()
                .get_output()
                .clone();
            // Not a vacuous pass: each verb has to have answered with something.
            assert!(
                !stdout(&output).trim().is_empty(),
                "`onetaskgraph {}` printed nothing, so this journey drove no work through it",
                arguments.join(" ")
            );
            answered += 1;
        }
        assert_eq!(answered, every_verb().len());

        let after = snapshot(&root);
        let mut offences = Vec::new();
        for (path, contents) in &after {
            let text = String::from_utf8_lossy(contents);
            match before.get(path) {
                Some(original) if original == contents => {}
                Some(_) => offences.push(format!("{} changed during the run", path.display())),
                None => {
                    let held: Vec<&str> = SENTINELS
                        .iter()
                        .copied()
                        .filter(|sentinel| text.contains(sentinel))
                        .collect();
                    offences.push(format!(
                        "{} was created during the run{}",
                        path.display(),
                        if held.is_empty() {
                            String::new()
                        } else {
                            format!(", holding {}", held.join(", "))
                        }
                    ));
                }
            }
        }

        assert!(
            offences.is_empty(),
            "the engine writes nothing down. These files say otherwise:\n  {}",
            offences.join("\n  ")
        );
    }
}
