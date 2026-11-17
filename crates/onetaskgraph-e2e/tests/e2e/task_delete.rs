//! Dropping a task from a plan, driven the way a user drives it: `task delete` and `project copy
//! --close-orphans`, through the compiled binary, over folders of Markdown.
//!
//! What a user sees is held here: a deleted task gone from every read of its project with its
//! file, its stored answers and its assets; a delete refused while a task depends on it, naming
//! each dependent and touching nothing; and a re-copy of the plan closing the copy the deleted
//! task left behind, while every other item — an orphan already done or cancelled among them —
//! stays byte for byte as it was. A GitHub board's half, where the close is an issue closed as
//! not planned, is in the GitHub Projects e2e suite.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::document;
use crate::machine::{bundle, validates};

/// A folder of Markdown under `sandbox` at `relative`, holding project `P` and these tasks, each
/// `(id, title, depends_on)`.
fn folder(sandbox: &Sandbox, relative: &str, tasks: &[(&str, &str, &[&str])]) -> PathBuf {
    let root = sandbox.subdirectory(relative);
    std::fs::create_dir_all(root.join("projects")).expect("the project folder");
    std::fs::create_dir_all(root.join("tasks")).expect("the task folder");
    std::fs::write(root.join("projects/P.md"), "---\ntitle: The plan\n---\n").expect("P");
    for (id, title, depends_on) in tasks {
        let depends = if depends_on.is_empty() {
            String::new()
        } else {
            format!("depends_on: [{}]\n", depends_on.join(", "))
        };
        std::fs::write(
            root.join(format!("tasks/{id}.md")),
            format!("---\ntitle: {title}\nproject: P\n{depends}---\nThe body of {id}.\n"),
        )
        .expect("a task file");
    }
    root
}

/// A sandbox configuring each `(name, root)` as a folder of Markdown.
fn configured(sandbox: &Sandbox, folders: &[(&str, &Path)]) {
    let mut sources = serde_json::Map::new();
    for (name, root) in folders {
        sources.insert(
            (*name).to_owned(),
            json!({"plugin": "local-md", "config": {"root": root}}),
        );
    }
    sandbox.project_document(&document(&Value::Object(sources)));
}

/// Every file under `root`, with its bytes, so a journey can say a folder did not change.
fn tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory).expect("a readable folder") {
            let path = entry.expect("an entry").path();
            if path.is_dir() {
                pending.push(path);
            } else {
                let bytes = std::fs::read(&path).expect("a readable file");
                files.insert(path, bytes);
            }
        }
    }
    files
}

/// The binary's whole output for one invocation.
fn run(sandbox: &Sandbox, arguments: &[&str]) -> Output {
    sandbox
        .command()
        .args(arguments)
        .assert()
        .get_output()
        .clone()
}

/// What one successful invocation printed, refusing anything but exit 0 with a silent stderr.
fn printed(sandbox: &Sandbox, arguments: &[&str]) -> String {
    let output = run(sandbox, arguments);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{arguments:?} failed\nstdout: {}\nstderr: {}",
        stdout(&output),
        stderr(&output)
    );
    assert_eq!(stderr(&output), "", "{arguments:?} wrote to stderr");
    stdout(&output)
}

/// The JSON document one successful invocation printed.
fn parsed(sandbox: &Sandbox, arguments: &[&str]) -> Value {
    let text = printed(sandbox, arguments);
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("{arguments:?}: {error}\n{text}"))
}

/// The qualified ids a `task list --json` answers.
fn listed(sandbox: &Sandbox, source: &str) -> Vec<String> {
    parsed(sandbox, &["task", "list", "--source", source, "--json"])["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|item| item["id"].as_str().expect("an id").to_owned())
        .collect()
}

/// The `(source, action, destination)` of each item a copy report answers, in its order.
fn actions(report: &Value) -> Vec<(String, String, Option<String>)> {
    report["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|item| {
            (
                item["source"].as_str().expect("a source").to_owned(),
                item["action"].as_str().expect("an action").to_owned(),
                item["destination"].as_str().map(ToOwned::to_owned),
            )
        })
        .collect()
}

/// The status category a task reads at.
fn category(sandbox: &Sandbox, id: &str) -> String {
    parsed(sandbox, &["task", "show", id, "--json"])["items"][0]["item"]["status"]["category"]
        .as_str()
        .expect("a category")
        .to_owned()
}

#[test]
fn a_deleted_task_is_gone_from_every_read_of_its_project_with_its_file_answers_and_assets() {
    let sandbox = Sandbox::new();
    let root = folder(&sandbox, "plans", &[("keep", "Keep", &[])]);
    configured(&sandbox, &[("plans", &root)]);
    // A task rendered from a template with an image asset, so it has stored answers and an
    // assets directory for the delete to take with it.
    let template = sandbox.project().join("template.md");
    std::fs::write(
        &template,
        "---\nonetaskgraph_template: 1\nvariables:\n  owner: {description: who owns it}\n---\n\
         owned by {{ owner }}\n![shot](./shot.png)\n",
    )
    .expect("a template");
    let image = sandbox.project().join("shot.png");
    std::fs::write(&image, b"\x89PNG\r\n\x1a\nimage bytes").expect("an image");
    let created = parsed(
        &sandbox,
        &[
            "task",
            "create",
            "plans",
            "--project",
            "P",
            "--title",
            "Drop me",
            "--template",
            template.to_str().expect("a path"),
            "--var",
            "owner=ops",
            "--asset",
            image.to_str().expect("a path"),
            "--json",
        ],
    );
    let id = created["items"][0]["id"]
        .as_str()
        .expect("an id")
        .to_owned();
    assert_eq!(id, "plans:drop-me");
    let file = root.join("tasks/drop-me.md");
    let assets = root.join("tasks/drop-me.assets");
    assert!(file.is_file() && assets.join("shot.png").is_file());
    printed(&sandbox, &["task", "answers", &id]);
    assert!(listed(&sandbox, "plans").contains(&id));
    assert!(printed(&sandbox, &["project", "graph", "plans:P"]).contains("Drop me"));

    assert_eq!(
        printed(&sandbox, &["task", "delete", &id]),
        format!("deleted task {id}\n")
    );

    assert!(!file.exists(), "the task's file is gone");
    assert!(!assets.exists(), "the task's assets are gone");
    let shown = run(&sandbox, &["task", "show", &id]);
    assert_ne!(shown.status.code(), Some(0), "{}", stdout(&shown));
    assert!(stderr(&shown).contains("no task"), "{}", stderr(&shown));
    let answers = run(&sandbox, &["task", "answers", &id]);
    assert_ne!(answers.status.code(), Some(0), "{}", stdout(&answers));
    assert_eq!(listed(&sandbox, "plans"), vec!["plans:keep".to_owned()]);
    let project = printed(&sandbox, &["project", "show", "plans:P"]);
    assert!(!project.contains("drop-me"), "{project}");
    let graph = parsed(
        &sandbox,
        &["project", "graph", "plans:P", "--format", "json"],
    );
    assert!(!graph.to_string().contains("drop-me"), "{graph}");
    assert!(!printed(&sandbox, &["project", "graph", "plans:P"]).contains("Drop me"));
    // The task it did not name is exactly as it was.
    assert_eq!(
        std::fs::read_to_string(root.join("tasks/keep.md")).expect("keep"),
        "---\ntitle: Keep\nproject: P\n---\nThe body of keep.\n"
    );
}

#[test]
fn json_output_names_the_deleted_task_in_the_schema_the_binary_emits() {
    let sandbox = Sandbox::new();
    let root = folder(&sandbox, "plans", &[("gone", "Gone", &[])]);
    configured(&sandbox, &[("plans", &root)]);
    let answer = parsed(&sandbox, &["task", "delete", "plans:gone", "--json"]);
    assert_eq!(answer, json!({"deleted": "plans:gone"}));
    validates(
        &bundle(&sandbox),
        "TaskDeleted",
        &answer,
        "task delete --json",
    );
    assert!(!root.join("tasks/gone.md").exists());
}

#[test]
fn a_task_another_task_depends_on_is_not_deleted_and_each_dependent_is_named() {
    let sandbox = Sandbox::new();
    let root = folder(
        &sandbox,
        "plans",
        &[
            ("base", "Base", &[]),
            ("walls", "Walls", &["base"]),
            ("roof", "Roof", &["base", "walls"]),
        ],
    );
    configured(&sandbox, &[("plans", &root)]);
    let before = tree(&root);

    let refused = run(&sandbox, &["task", "delete", "plans:base"]);

    assert_eq!(refused.status.code(), Some(1), "{}", stdout(&refused));
    assert_eq!(stdout(&refused), "");
    let message = stderr(&refused);
    assert!(
        message.contains("task plans:base was not deleted: 2 items of its source depend on it"),
        "{message}"
    );
    assert!(message.contains("\n  plans:walls\n"), "{message}");
    assert!(message.contains("\n  plans:roof\n"), "{message}");
    assert!(message.contains("--depends-on"), "{message}");
    assert_eq!(tree(&root), before, "a refused delete removed nothing");

    // The task nothing depends on is removable: `roof` names `walls` and `base` but nothing
    // names `roof`.
    printed(&sandbox, &["task", "delete", "plans:roof"]);
    let still = run(&sandbox, &["task", "delete", "plans:base"]);
    assert_eq!(still.status.code(), Some(1));
    let message = stderr(&still);
    assert!(
        message.contains("1 item of its source depends on it"),
        "{message}"
    );
    assert!(message.contains("plans:walls") && !message.contains("plans:roof"));
}

#[test]
fn an_unknown_or_unqualified_id_is_refused_and_removes_nothing() {
    let sandbox = Sandbox::new();
    let root = folder(&sandbox, "plans", &[("only", "Only", &[])]);
    configured(&sandbox, &[("plans", &root)]);
    let before = tree(&root);
    for (id, says) in [
        ("plans:nope", "no task with the id plans:nope"),
        ("nope", "nope"),
        ("elsewhere:only", "elsewhere"),
    ] {
        let refused = run(&sandbox, &["task", "delete", id]);
        assert_ne!(refused.status.code(), Some(0), "{id}: {}", stdout(&refused));
        assert_eq!(stdout(&refused), "", "{id}");
        assert!(
            stderr(&refused).contains(says),
            "{id}: {}",
            stderr(&refused)
        );
        assert!(
            stderr(&refused).contains("next:"),
            "{id}: {}",
            stderr(&refused)
        );
    }
    assert_eq!(tree(&root), before);
}

#[test]
fn a_task_of_a_source_with_no_write_side_is_not_deleted() {
    let sandbox = Sandbox::new();
    sandbox.project_document(&document(&json!({
        "frozen": {"plugin": "in-memory", "config": {
            "capabilities": {"writes": "unsupported"},
            "tasks": [{"id": "T-1", "title": "Kept", "labels": [],
                       "status": {"category": "todo", "name": "Todo"}}]}}
    })));
    let refused = run(&sandbox, &["task", "delete", "frozen:T-1", "--json"]);
    assert_eq!(refused.status.code(), Some(1), "{}", stderr(&refused));
    let failure: Value = serde_json::from_str(&stdout(&refused)).expect("a failure document");
    assert_eq!(failure["failure"]["kind"], "not-writable", "{failure}");
    assert!(
        stderr(&refused).contains(
            "source frozen cannot delete a task: its plugin is in-memory, which has no write side"
        ),
        "{}",
        stderr(&refused)
    );
}

/// A plan of four tasks copied into `copies`, then three of them deleted at the source: `open`
/// left as the copy wrote it, `finished` set `done` and `abandoned` set `cancelled` at the
/// destination first. Answers the two folders.
fn dropped_plan(sandbox: &Sandbox) -> (PathBuf, PathBuf) {
    let plans = folder(
        sandbox,
        "plans",
        &[
            ("stay", "Stay", &[]),
            ("open", "Open", &[]),
            ("finished", "Finished", &[]),
            ("abandoned", "Abandoned", &[]),
        ],
    );
    let copies = sandbox.subdirectory("copies");
    configured(sandbox, &[("plans", &plans), ("copies", &copies)]);
    printed(sandbox, &["project", "copy", "plans:P", "--to", "copies"]);
    printed(
        sandbox,
        &["task", "status", "set", "copies:finished", "done"],
    );
    printed(
        sandbox,
        &["task", "status", "set", "copies:abandoned", "cancelled"],
    );
    for id in ["plans:open", "plans:finished", "plans:abandoned"] {
        printed(sandbox, &["task", "delete", id]);
    }
    (plans, copies)
}

#[test]
fn close_orphans_closes_an_open_orphan_and_leaves_every_other_item_as_it_was() {
    let sandbox = Sandbox::new();
    let (_, copies) = dropped_plan(&sandbox);
    let before = tree(&copies);
    let orphaned = |id: &str| {
        (
            format!("plans:{id}"),
            "orphaned".to_owned(),
            Some(format!("copies:{id}")),
        )
    };
    let unchanged = |id: &str| {
        (
            format!("plans:{id}"),
            "unchanged".to_owned(),
            Some(format!("copies:{id}")),
        )
    };
    let copy = ["project", "copy", "plans:P", "--to", "copies", "--json"];

    // Without the flag nothing is written to any orphan, and each is reported as before.
    let flagless = parsed(&sandbox, &copy);
    assert_eq!(
        actions(&flagless),
        vec![
            unchanged("P"),
            unchanged("stay"),
            orphaned("abandoned"),
            orphaned("finished"),
            orphaned("open"),
        ]
    );
    assert_eq!(
        tree(&copies),
        before,
        "a copy without the flag closed nothing"
    );
    validates(
        &bundle(&sandbox),
        "CopyReport",
        &flagless,
        "project copy --json",
    );

    // A dry run reports the close and writes nothing.
    let closing = [&copy[..], &["--close-orphans"]].concat();
    let dry = parsed(&sandbox, &[&closing[..], &["--dry-run"]].concat());
    let closed = (
        "plans:open".to_owned(),
        "closed".to_owned(),
        Some("copies:open".to_owned()),
    );
    let expected = vec![
        unchanged("P"),
        unchanged("stay"),
        orphaned("abandoned"),
        orphaned("finished"),
        closed.clone(),
    ];
    assert_eq!(actions(&dry), expected);
    assert_eq!(tree(&copies), before, "a dry run closed nothing");

    // The copy closes the open orphan alone, as `task status set … cancelled` would.
    let report = parsed(&sandbox, &closing);
    assert_eq!(actions(&report), expected);
    assert_eq!(
        report["items"][4],
        json!({"source": "plans:open", "action": "closed", "destination": "copies:open"})
    );
    validates(
        &bundle(&sandbox),
        "CopyReport",
        &report,
        "project copy --json",
    );
    assert_eq!(category(&sandbox, "copies:open"), "cancelled");
    let after = tree(&copies);
    let open = copies.join("tasks/open.md");
    for (path, bytes) in &before {
        if path != &open {
            assert_eq!(after.get(path), Some(bytes), "{} changed", path.display());
        }
    }
    assert_ne!(
        after.get(&open),
        before.get(&open),
        "the open orphan was closed"
    );

    // Now cancelled, it is an orphan like the others, and a second close writes nothing.
    let again = tree(&copies);
    let repeat = parsed(&sandbox, &closing);
    assert_eq!(actions(&repeat)[4], orphaned("open"));
    assert_eq!(tree(&copies), again);
}

#[test]
fn close_orphans_in_the_human_rendering_reads_closed() {
    let sandbox = Sandbox::new();
    let (_, copies) = dropped_plan(&sandbox);
    let human = printed(
        &sandbox,
        &[
            "project",
            "copy",
            "plans:P",
            "--to",
            "copies",
            "--close-orphans",
        ],
    );
    let row = human
        .lines()
        .find(|line| line.starts_with("plans:open"))
        .unwrap_or_else(|| panic!("no row for the open orphan:\n{human}"));
    assert_eq!(
        row.split_whitespace().collect::<Vec<_>>(),
        ["plans:open", "copies:open", "closed"]
    );
    let finished = human
        .lines()
        .find(|line| line.starts_with("plans:finished"))
        .expect("a row for the finished orphan");
    assert!(finished.ends_with("orphaned"), "{human}");
    assert!(
        copies.join("tasks/open.md").is_file(),
        "a close removes nothing"
    );
}

#[test]
fn close_orphans_beside_a_copy_that_does_not_walk_every_task_is_refused_before_anything_is_read() {
    let sandbox = Sandbox::new();
    let (plans, copies) = dropped_plan(&sandbox);
    let (plans_before, copies_before) = (tree(&plans), tree(&copies));
    // `nowhere` is no configured source: a refusal that read anything would name it instead.
    for narrowing in [
        &["--member", "nowhere:stay"][..],
        &["--member", "nowhere:stay", "--members-only"][..],
        &["--no-tasks"][..],
    ] {
        let arguments = [
            &[
                "project",
                "copy",
                "nowhere:P",
                "--to",
                "copies",
                "--close-orphans",
            ][..],
            narrowing,
        ]
        .concat();
        let refused = run(&sandbox, &arguments);
        assert_eq!(refused.status.code(), Some(2), "{arguments:?}");
        assert_eq!(stdout(&refused), "", "{arguments:?}");
        let message = stderr(&refused);
        assert!(
            message.contains("'--close-orphans' cannot be used with"),
            "{arguments:?}: {message}"
        );
        assert!(!message.contains("nowhere"), "{arguments:?}: {message}");
    }
    assert_eq!(tree(&plans), plans_before);
    assert_eq!(tree(&copies), copies_before);
}

#[test]
fn closing_an_orphan_keeps_the_tasks_it_delivers_in_step() {
    // The open orphan delivers a ticket the destination holds of its own: closing the orphan is
    // the write `task status set … cancelled` makes, so the ticket is re-evaluated against it and
    // the copy reports that beside the close.
    let sandbox = Sandbox::new();
    let (_, copies) = dropped_plan(&sandbox);
    let ticket = parsed(
        &sandbox,
        &[
            "task",
            "create",
            "copies",
            "--project",
            "P",
            "--title",
            "Ticket",
            "--json",
        ],
    )["items"][0]["id"]
        .as_str()
        .expect("an id")
        .to_owned();
    printed(
        &sandbox,
        &[
            "task",
            "update",
            "copies:open",
            "--delivers",
            &ticket,
            "--json",
        ],
    );

    let report = parsed(
        &sandbox,
        &[
            "project",
            "copy",
            "plans:P",
            "--to",
            "copies",
            "--close-orphans",
            "--json",
        ],
    );

    assert_eq!(actions(&report)[4].1, "closed", "{report:#}");
    let delivered = report["delivered"]
        .as_array()
        .expect("what the close kept in step");
    assert!(
        delivered
            .iter()
            .any(|entry| entry["ticket"] == ticket.as_str() && entry["deliverer"] == "copies:open"),
        "{report:#}"
    );
    assert_eq!(category(&sandbox, "copies:open"), "cancelled");
    assert!(copies.join("tasks/open.md").is_file());
}

#[test]
fn close_orphans_closes_a_dropped_tasks_copy_in_the_member_a_route_filed_it_under() {
    // `notes` routes widgetco work to `team`: the plan's home lands in `notes` beside its
    // library task, and its two widgetco tasks in a member project in `team`. Dropping one of
    // those, the re-copy walks the member too and closes the copy there.
    let sandbox = Sandbox::new();
    let plan = sandbox.subdirectory("plan");
    let notes = sandbox.subdirectory("notes");
    let team = sandbox.subdirectory("team");
    for (kind, id, front) in [
        ("projects", "goal", "title: One goal"),
        (
            "tasks",
            "lib",
            "title: Library\nproject: goal\nrepositories: [github.com/nickderobertis/lib]",
        ),
        (
            "tasks",
            "app",
            "title: App\nproject: goal\nrepositories: [github.com/widgetco/app]",
        ),
        (
            "tasks",
            "site",
            "title: Site\nproject: goal\nrepositories: [github.com/widgetco/site]",
        ),
    ] {
        std::fs::create_dir_all(plan.join(kind)).expect("a folder");
        std::fs::write(
            plan.join(kind).join(format!("{id}.md")),
            format!("---\n{front}\n---\nBody of {id}.\n"),
        )
        .expect("a record");
    }
    sandbox.project_document(&document(&json!({
        "plan": {"plugin": "local-md", "config": {"root": plan}},
        "notes": {
            "plugin": "local-md",
            "config": {"root": notes},
            "routes": [{"repositories": ["github.com/widgetco/*"], "to": "team"}],
        },
        "team": {"plugin": "local-md", "config": {"root": team}},
    })));
    let first = parsed(
        &sandbox,
        &["project", "copy", "plan:goal", "--to", "notes", "--json"],
    );
    let site = first["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|item| item["source"] == "plan:site")
        .expect("the site task")["destination"]
        .as_str()
        .expect("where it landed")
        .to_owned();
    assert!(site.starts_with("team:"), "{first:#}");
    printed(&sandbox, &["task", "delete", "plan:site"]);
    let notes_before = tree(&notes);

    let report = parsed(
        &sandbox,
        &[
            "project",
            "copy",
            "plan:goal",
            "--to",
            "notes",
            "--close-orphans",
            "--json",
        ],
    );

    let closed: Vec<&Value> = report["items"]
        .as_array()
        .expect("items")
        .iter()
        .filter(|item| item["action"] == "closed")
        .collect();
    assert_eq!(closed.len(), 1, "{report:#}");
    assert_eq!(closed[0]["source"], "plan:site");
    assert_eq!(closed[0]["destination"], site.as_str());
    assert_eq!(category(&sandbox, &site), "cancelled");
    assert_eq!(
        tree(&notes),
        notes_before,
        "nothing in the home's source moved"
    );
}
