//! `project status set` through the binary: one project's status written on a folder of
//! Markdown — in process and over the stdio plugin protocol alike — and every refusal it owes,
//! each before anything is written.
//!
//! A folder of Markdown outlives the process, so each journey reads what landed byte for byte
//! afterwards rather than trusting the answer alone. The GitHub board's and the Linear
//! workspace's own journeys are in their plugins' e2e suites.

use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::{Value, json};

use crate::common::{SOURCE_BOUNDARIES, Sandbox, stderr, stdout};
use crate::fixtures::document;
use crate::machine::{bundle, validates};

/// A project file whose every other line a status write must leave as it is.
const PROJECT: &str = "---\ntitle: Engine\nstatus: todo\nlabels:\n  - core\nmetadata:\n  caller.note: kept as written\n---\nThe engine's plan.\n\nIts second paragraph.\n";

/// A task file in the same folder, which a project status write never touches.
const TASK: &str = "---\ntitle: One\nstatus: todo\nproject: P-1\n---\nA task of the plan.\n";

/// A folder of Markdown called `name` holding [`PROJECT`] as `P-1` and [`TASK`] as `T-1`.
fn folder(sandbox: &Sandbox, name: &str) -> PathBuf {
    let root = sandbox.subdirectory(name);
    for (relative, text) in [("projects/P-1.md", PROJECT), ("tasks/T-1.md", TASK)] {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("a folder");
        std::fs::write(path, text).expect("a file");
    }
    root
}

fn read(root: &Path, relative: &str) -> String {
    std::fs::read_to_string(root.join(relative)).expect("the file reads")
}

fn run(sandbox: &Sandbox, arguments: &[&str]) -> Output {
    sandbox
        .command()
        .args(arguments)
        .assert()
        .get_output()
        .clone()
}

/// A run that had to exit `code`, quoting what it said when it did not.
fn exits(sandbox: &Sandbox, arguments: &[&str], code: i32) -> Output {
    let output = run(sandbox, arguments);
    assert_eq!(
        output.status.code(),
        Some(code),
        "`onetaskgraph {}`\nstdout:\n{}\nstderr:\n{}",
        arguments.join(" "),
        stdout(&output),
        stderr(&output)
    );
    output
}

#[test]
fn a_project_status_set_rewrites_the_project_files_status_line_and_nothing_else() {
    for boundary in SOURCE_BOUNDARIES {
        let sandbox = Sandbox::new();
        let plan = folder(&sandbox, "plan");
        sandbox.project_document(&document(&json!({
            "plan": boundary.source("local-md", json!({"root": plan})),
        })));
        let schemas = bundle(&sandbox);

        let output = exits(
            &sandbox,
            &["--json", "project", "status", "set", "plan:P-1", "done"],
            0,
        );
        let answer: Value = serde_json::from_str(&stdout(&output)).expect("JSON");
        assert_eq!(
            answer,
            json!({"id": "plan:P-1", "status": {"category": "done", "name": "done"}}),
            "{boundary:?}"
        );
        validates(&schemas, "ProjectStatusSet", &answer, "project status set");
        assert_eq!(
            read(&plan, "projects/P-1.md"),
            PROJECT.replace("status: todo\n", "status: done\n"),
            "{boundary:?}: only the status line moved"
        );
        assert_eq!(
            read(&plan, "tasks/T-1.md"),
            TASK,
            "{boundary:?}: no task is touched"
        );
        let shown = exits(&sandbox, &["--json", "project", "show", "plan:P-1"], 0);
        let shown: Value = serde_json::from_str(&stdout(&shown)).expect("JSON");
        assert_eq!(
            shown["items"][0]["item"]["status"],
            json!({"category": "done", "name": "done"}),
            "{boundary:?}: a later invocation reads the status set"
        );

        let human = exits(
            &sandbox,
            &["project", "status", "set", "plan:P-1", "in-progress"],
            0,
        );
        assert_eq!(
            stdout(&human).trim_end(),
            "project plan:P-1 now reads in-progress (in progress)",
            "{boundary:?}"
        );
        assert_eq!(
            read(&plan, "projects/P-1.md"),
            PROJECT.replace("status: todo\n", "status: in progress\n"),
            "{boundary:?}"
        );
    }
}

#[test]
fn every_refusal_a_project_status_set_owes_exits_one_naming_why_before_any_write() {
    let sandbox = Sandbox::new();
    let plan = folder(&sandbox, "plan");
    let narrow = folder(&sandbox, "narrow");
    sandbox.project_document(&document(&json!({
        "plan": {"plugin": "local-md", "config": {"root": plan}},
        // A folder whose mapping holds no word for `queued`, which it therefore cannot write.
        "narrow": {"plugin": "local-md", "config": {"root": narrow,
            "status_mapping": {"todo": "todo", "done": "done"}}},
        "frozen": {"plugin": "in-memory", "config": {
            "capabilities": {"writes": "unsupported"},
            "projects": [{"id": "P-1", "title": "Engine", "content": null,
                          "status": {"category": "todo", "name": "Todo"}, "labels": []}]}},
    })));
    for (arguments, kind, says) in [
        (
            ["plan:P-404", "done"],
            "no-such-item",
            "no project with the id plan:P-404",
        ),
        (
            ["plan:T-1", "done"],
            "no-such-item",
            "no project with the id plan:T-1",
        ),
        (
            ["narrow:P-1", "queued"],
            "refused",
            "cannot represent the field `status`",
        ),
        (
            ["frozen:P-1", "done"],
            "not-writable",
            "source frozen cannot write a status: its plugin is in-memory",
        ),
    ] {
        let mut full = vec!["--json", "project", "status", "set"];
        full.extend(arguments);
        let output = exits(&sandbox, &full, 1);
        let failure: Value = serde_json::from_str(&stdout(&output))
            .unwrap_or_else(|error| panic!("{arguments:?}: a failure document ({error})"));
        let message = failure["failure"]["message"].as_str().unwrap_or_default();
        assert!(
            message.contains(says) && message.contains("next:"),
            "{arguments:?}: {failure:#}"
        );
        assert_eq!(
            failure["failure"]["kind"], kind,
            "{arguments:?}: {failure:#}"
        );
        assert!(
            stderr(&output).contains(says),
            "{arguments:?}: {}",
            stderr(&output)
        );
    }
    for root in [&plan, &narrow] {
        assert_eq!(
            read(root, "projects/P-1.md"),
            PROJECT,
            "nothing was written"
        );
        assert_eq!(read(root, "tasks/T-1.md"), TASK, "nothing was written");
    }
}
