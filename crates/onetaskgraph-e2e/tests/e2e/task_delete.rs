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
use crate::document_store::interpreter;
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
    // those, the re-copy walks the member too and closes the copy there — found by its origin,
    // and again with the plan declared private, by the record the delete left on the plan.
    for private in [false, true] {
        routed_dropped_task_closes(private);
    }
}

fn routed_dropped_task_closes(private: bool) {
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
    let mut configuration: Value = serde_json::from_str(&document(&json!({
        "plan": {"plugin": "local-md", "config": {"root": plan}},
        "notes": {
            "plugin": "local-md",
            "config": {"root": notes},
            "routes": [{"repositories": ["github.com/widgetco/*"], "to": "team"}],
        },
        "team": {"plugin": "local-md", "config": {"root": team}},
    })))
    .expect("a document");
    if private {
        configuration["sources"]["plan"]["visibility"] = json!("private");
        configuration["write_policy"] = passing_write_policy();
    }
    sandbox.project_document(&serde_json::to_string_pretty(&configuration).expect("renders"));
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
    let origin =
        parsed(&sandbox, &["task", "show", &site, "--json"])["items"][0]["item"]["metadata"]
            .get("onetaskgraph.origin")
            .cloned();
    assert_eq!(origin.is_none(), private, "{origin:?}");
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

/// A `write_policy` whose check passes every write and which answers every repository public: a
/// store declaring a visibility is held to the public boundary, and what is under test beside it
/// is the record a delete leaves.
fn passing_write_policy() -> Value {
    json!({
        "check_command": [
            interpreter(), "-c",
            "import sys; sys.stdin.read(); print('{\"verdict\": \"pass\"}')"
        ],
        "visibility_command": [
            interpreter(), "-c",
            "import sys; sys.stdin.read(); print('{\"visibility\": \"public\"}')"
        ],
    })
}

/// A folder declared private, `drafts`, holding project `P` and these tasks, copied once into a
/// folder that is not, `public`: a copy that withholds provenance, so no copy it lands names an
/// origin, and a dropped task's copy is found by what `task delete` records on `P`. A second
/// folder that is not, `mirror`, is configured beside it and copied into only when a journey
/// asks.
struct PrivatePlan {
    sandbox: Sandbox,
    drafts: PathBuf,
    public: PathBuf,
    /// Each draft task's copy in `public`, by the task's qualified id.
    landed: BTreeMap<String, String>,
    /// `P`'s copy in `public`.
    project: String,
}

impl PrivatePlan {
    fn new(tasks: &[(&str, &str, &[&str])]) -> Self {
        let sandbox = Sandbox::new();
        let drafts = folder(&sandbox, "drafts", tasks);
        let public = sandbox.subdirectory("public");
        let mirror = sandbox.subdirectory("mirror");
        let mut configuration: Value = serde_json::from_str(&document(&json!({
            "drafts": {"plugin": "local-md", "config": {"root": drafts}, "visibility": "private"},
            "public": {"plugin": "local-md", "config": {"root": public}},
            "mirror": {"plugin": "local-md", "config": {"root": mirror}}
        })))
        .expect("a document");
        configuration["write_policy"] = passing_write_policy();
        sandbox.project_document(&serde_json::to_string_pretty(&configuration).expect("renders"));
        let first = parsed(&sandbox, &Self::COPY);
        let project = actions(&first)
            .into_iter()
            .find(|(source, _, _)| source == "drafts:P")
            .and_then(|(_, _, destination)| destination)
            .expect("P's copy");
        let landed = actions(&first)
            .into_iter()
            .filter(|(source, _, _)| source != "drafts:P")
            .map(|(source, action, destination)| {
                assert_eq!(action, "created", "{first:#}");
                (source, destination.expect("a destination"))
            })
            .collect();
        Self {
            sandbox,
            drafts,
            public,
            landed,
            project,
        }
    }

    const COPY: [&'static str; 6] = ["project", "copy", "drafts:P", "--to", "public", "--json"];

    /// One copy of the plan with these flags besides, which had to succeed.
    fn copy(&self, flags: &[&str]) -> Value {
        parsed(&self.sandbox, &[&Self::COPY[..], flags].concat())
    }

    /// What `P` records at `onetaskgraph.deleted_copies`, as `project show` reads it.
    fn record(&self) -> Value {
        parsed(&self.sandbox, &["project", "show", "drafts:P", "--json"])["items"][0]["item"]
            ["metadata"]["onetaskgraph.deleted_copies"]
            .clone()
    }

    /// The record `P` holds of `tasks`' copies, each as its delete recorded it.
    fn recording(&self, tasks: &[&str]) -> Value {
        let mut record = serde_json::Map::new();
        for task in tasks {
            let (at, native) = self.landed[*task].split_once(':').expect("a qualified id");
            record
                .entry(at)
                .or_insert_with(|| json!({}))
                .as_object_mut()
                .expect("an object")
                .insert(native.to_owned(), json!(task));
        }
        Value::Object(record)
    }
}

/// What a copy report says of every item but those left `unchanged`.
fn moved(report: &Value) -> Vec<(String, String, Option<String>)> {
    actions(report)
        .into_iter()
        .filter(|(_, action, _)| action != "unchanged")
        .collect()
}

#[test]
fn a_private_folders_deleted_task_closes_its_copy_that_carries_no_origin_at_the_next_closing_copy()
{
    // The delete changes nothing but the private folder; a copy without `--close-orphans` and a
    // closing dry run leave the dropped task's copy and the record alone; the closing copy
    // cancels that copy alone and takes the record's entry out; and the copy after it says and
    // writes nothing of the dropped task.
    let plan = PrivatePlan::new(&[("stay", "Stay", &[]), ("open", "Open", &[])]);
    let copied = plan.landed["drafts:open"].clone();
    assert_eq!(
        parsed(&plan.sandbox, &["task", "show", &copied, "--json"])["items"][0]["item"]["metadata"]
            .get("onetaskgraph.origin"),
        None,
        "the copy names no private origin"
    );
    let published = tree(&plan.public);
    assert_eq!(
        printed(&plan.sandbox, &["task", "delete", "drafts:open"]),
        "deleted task drafts:open\n"
    );
    let recorded = plan.recording(&["drafts:open"]);
    assert_eq!(plan.record(), recorded);
    assert_eq!(
        tree(&plan.public),
        published,
        "a delete reached the public folder"
    );

    // Neither a copy without the flag nor a closing dry run writes anything or the record; only
    // the dry run, which reads the record, says anything of the dropped task.
    assert_eq!(moved(&plan.copy(&[])), vec![]);
    let closed = (
        "drafts:open".to_owned(),
        "closed".to_owned(),
        Some(copied.clone()),
    );
    assert_eq!(
        moved(&plan.copy(&["--close-orphans", "--dry-run"])),
        vec![closed.clone()]
    );
    assert_eq!(tree(&plan.public), published);
    assert_eq!(plan.record(), recorded);

    // The closing copy cancels the copy alone and takes its entry out of the record.
    let report = plan.copy(&["--close-orphans"]);
    validates(
        &bundle(&plan.sandbox),
        "CopyReport",
        &report,
        "project copy --json",
    );
    assert_eq!(moved(&report), vec![closed]);
    assert_eq!(category(&plan.sandbox, &copied), "cancelled");
    let after = tree(&plan.public);
    let changed: Vec<&PathBuf> = published
        .iter()
        .filter(|(path, bytes)| after.get(*path) != Some(*bytes))
        .map(|(path, _)| path)
        .collect();
    assert_eq!(changed.len(), 1, "one copy changed: {changed:?}");
    assert_eq!(plan.record(), json!({}));
    let shown = parsed(&plan.sandbox, &["project", "show", &plan.project, "--json"]);
    assert_eq!(
        shown["items"][0]["item"]["metadata"].get("onetaskgraph.deleted_copies"),
        None,
        "no copy carries the record to the destination's project"
    );

    // With nothing left on record, the next closing copy says and writes nothing of it.
    assert_eq!(moved(&plan.copy(&["--close-orphans"])), vec![]);
    assert_eq!(tree(&plan.public), after);
}

#[test]
fn a_recorded_copy_already_finished_or_landed_again_is_taken_off_the_record_and_not_closed() {
    // Two dropped tasks: `finished`'s copy a person already set done, which is reported
    // `orphaned` and left as it is, and `again`'s copy, which a new draft task of the same title
    // takes over by `--match-by title` in the very copy that would close it — so it serves a task
    // the source still holds and is not closed. Both entries go from the record.
    let plan = PrivatePlan::new(&[
        ("stay", "Stay", &[]),
        ("finished", "Finished", &[]),
        ("again", "Again", &[]),
        ("vanished", "Vanished", &[]),
    ]);
    let finished = plan.landed["drafts:finished"].clone();
    let again = plan.landed["drafts:again"].clone();
    printed(&plan.sandbox, &["task", "status", "set", &finished, "done"]);
    let dropped = ["drafts:finished", "drafts:again", "drafts:vanished"];
    for task in dropped {
        printed(&plan.sandbox, &["task", "delete", task]);
    }
    assert_eq!(plan.record(), plan.recording(&dropped));
    // A third, `vanished`, had its copy deleted by a person as well: nothing is filed there to
    // close or report, and its entry goes too.
    printed(
        &plan.sandbox,
        &["task", "delete", &plan.landed["drafts:vanished"]],
    );
    std::fs::write(
        plan.drafts.join("tasks/renewed.md"),
        "---\ntitle: Again\nproject: P\n---\nThe body of again.\n",
    )
    .expect("a new draft task");
    let finished_before = tree(&plan.public);

    let report = plan.copy(&["--close-orphans", "--match-by", "title"]);
    let said = moved(&report);
    assert!(
        said.contains(&(
            "drafts:finished".to_owned(),
            "orphaned".to_owned(),
            Some(finished.clone())
        )),
        "{report:#}"
    );
    assert!(
        said.iter()
            .all(|(source, _, _)| source != "drafts:vanished"),
        "{report:#}"
    );
    assert!(
        said.iter()
            .all(|(source, action, destination)| action != "closed"
                && (source != "drafts:renewed" || destination.as_ref() == Some(&again))),
        "{report:#}"
    );
    assert_eq!(category(&plan.sandbox, &finished), "done");
    assert_ne!(category(&plan.sandbox, &again), "cancelled");
    let finished_file = |tree: &BTreeMap<PathBuf, Vec<u8>>| {
        tree.iter()
            .find(|(path, _)| path.ends_with(format!("{}.md", finished.split_once(':').unwrap().1)))
            .map(|(_, bytes)| bytes.clone())
    };
    assert_eq!(
        finished_file(&tree(&plan.public)),
        finished_file(&finished_before)
    );
    assert_eq!(plan.record(), json!({}));
}

#[cfg(unix)]
fn permit(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .expect("the path's permissions");
}

#[cfg(unix)]
#[test]
fn a_delete_that_cannot_record_or_cannot_remove_leaves_the_task_and_the_record_as_they_were() {
    // A project file that cannot be read, and a project folder the record cannot be written
    // into, each refuse the delete before anything is removed; a task folder the file cannot be
    // removed from refuses it after the record was written, which puts the record back. Then the
    // delete goes through.
    let plan = PrivatePlan::new(&[("stay", "Stay", &[]), ("open", "Open", &[])]);
    for (path, mode, restore, what) in [
        ("projects/P.md", 0o000, 0o644, "the project's read"),
        ("projects", 0o555, 0o755, "the record"),
        ("tasks", 0o555, 0o755, "the removal"),
    ] {
        let before = tree(&plan.drafts);
        permit(&plan.drafts.join(path), mode);
        let refused = run(&plan.sandbox, &["task", "delete", "drafts:open"]);
        permit(&plan.drafts.join(path), restore);
        assert_ne!(
            refused.status.code(),
            Some(0),
            "{what}: {}",
            stdout(&refused)
        );
        assert_eq!(stdout(&refused), "", "{what}");
        assert!(
            stderr(&refused).contains("next:"),
            "{what}: {}",
            stderr(&refused)
        );
        assert!(
            listed(&plan.sandbox, "drafts").contains(&"drafts:open".to_owned()),
            "{what}"
        );
        let record = plan.record();
        assert!(
            record.is_null() || record == json!({}),
            "{what}: the record names nothing: {record}"
        );
        if path != "tasks" {
            assert_eq!(tree(&plan.drafts), before, "{what}: nothing moved");
        }
    }
    printed(&plan.sandbox, &["task", "delete", "drafts:open"]);
    assert_eq!(plan.record(), plan.recording(&["drafts:open"]));
}

#[cfg(unix)]
#[test]
fn a_close_the_destination_refuses_keeps_its_entry_for_the_next_closing_copy() {
    // The public folder refuses the close: the copy reports it `failed` and exits 4, and the
    // entry stays, so the next closing copy closes it and takes the entry out.
    let plan = PrivatePlan::new(&[("stay", "Stay", &[]), ("open", "Open", &[])]);
    let copied = plan.landed["drafts:open"].clone();
    printed(&plan.sandbox, &["task", "delete", "drafts:open"]);
    let recorded = plan.record();
    let tasks = plan.public.join("tasks");
    permit(&tasks, 0o555);
    let refused = run(
        &plan.sandbox,
        &[&PrivatePlan::COPY[..], &["--close-orphans"]].concat(),
    );
    permit(&tasks, 0o755);
    assert_eq!(refused.status.code(), Some(4), "{}", stderr(&refused));
    let report: Value = serde_json::from_str(&stdout(&refused)).expect("the report");
    assert_eq!(
        moved(&report)
            .into_iter()
            .map(|(source, action, _)| (source, action))
            .collect::<Vec<_>>(),
        vec![("drafts:open".to_owned(), "failed".to_owned())]
    );
    assert_ne!(category(&plan.sandbox, &copied), "cancelled");
    assert_eq!(plan.record(), recorded, "a refused close keeps its entry");

    let closing = plan.copy(&["--close-orphans"]);
    assert_eq!(
        moved(&closing),
        vec![(
            "drafts:open".to_owned(),
            "closed".to_owned(),
            Some(copied.clone())
        )]
    );
    assert_eq!(category(&plan.sandbox, &copied), "cancelled");
    assert_eq!(plan.record(), json!({}));
}

#[test]
fn a_task_filed_under_a_project_its_source_does_not_hold_is_deleted_recording_nothing() {
    // No project copy walks a project the source does not hold, so there is no record for a
    // closing copy to read: the task copied on its own is deleted, and nothing is recorded on
    // the plan's project or reaches the public folder.
    let plan = PrivatePlan::new(&[("stay", "Stay", &[])]);
    std::fs::write(
        plan.drafts.join("tasks/lost.md"),
        "---\ntitle: Lost\nproject: Ghost\n---\nThe body of lost.\n",
    )
    .expect("a task under a project the folder does not hold");
    let copied = parsed(
        &plan.sandbox,
        &["task", "copy", "drafts:lost", "--to", "public", "--json"],
    );
    assert_eq!(actions(&copied)[0].1, "created", "{copied:#}");
    let published = tree(&plan.public);
    assert_eq!(
        printed(&plan.sandbox, &["task", "delete", "drafts:lost"]),
        "deleted task drafts:lost\n"
    );
    assert!(!listed(&plan.sandbox, "drafts").contains(&"drafts:lost".to_owned()));
    assert!(plan.record().is_null(), "{}", plan.record());
    assert_eq!(tree(&plan.public), published);
}

#[test]
fn a_recorded_copy_moved_to_another_project_is_left_as_it_is_and_taken_off_the_record() {
    // A person refiled the dropped task's copy under another project of the public folder: the
    // closing copy reads the project's copy alone, so it neither reports nor closes the moved
    // copy, which stays byte for byte, and the entry goes.
    let plan = PrivatePlan::new(&[("stay", "Stay", &[]), ("open", "Open", &[])]);
    let copied = plan.landed["drafts:open"].clone();
    printed(&plan.sandbox, &["task", "delete", "drafts:open"]);
    std::fs::write(
        plan.public.join("projects/elsewhere.md"),
        "---\ntitle: Elsewhere\n---\n",
    )
    .expect("another project");
    let file = plan.public.join(format!(
        "tasks/{}.md",
        copied.split_once(':').expect("an id").1
    ));
    let (_, project) = plan.project.split_once(':').expect("an id");
    let text = std::fs::read_to_string(&file).expect("the copy's file");
    let moved_text = text.replacen(&format!("project: {project}\n"), "project: elsewhere\n", 1);
    assert_ne!(moved_text, text, "the copy's file names its project");
    std::fs::write(&file, &moved_text).expect("the copy refiled");

    let report = plan.copy(&["--close-orphans"]);
    assert_eq!(moved(&report), vec![], "{report:#}");
    assert_eq!(
        std::fs::read_to_string(&file).expect("the copy"),
        moved_text
    );
    assert_eq!(plan.record(), json!({}));
}

#[test]
fn a_hand_edited_record_keeps_its_well_formed_entries_through_the_next_delete() {
    // The record is the engine's, so an entry a person mangled is left out of what is read and
    // of what the next delete writes back, beside every well-formed entry: the mangled one is
    // never closed, and the well-formed one still is.
    let plan = PrivatePlan::new(&[("stay", "Stay", &[]), ("open", "Open", &[])]);
    let project = plan.drafts.join("projects/P.md");
    let held = std::fs::read_to_string(&project).expect("P's file");
    let edited = held.replacen(
        "\nmetadata:\n",
        "\nmetadata:\n  onetaskgraph.deleted_copies:\n    public:\n      stay: 7\n      also: not \
         a qualified id\n    listed: [drafts:x]\n",
        1,
    );
    assert_ne!(edited, held, "P records its copy's link under `metadata`");
    std::fs::write(&project, edited).expect("a hand-edited record");
    printed(&plan.sandbox, &["task", "delete", "drafts:open"]);
    assert_eq!(plan.record(), plan.recording(&["drafts:open"]));
    let report = plan.copy(&["--close-orphans"]);
    assert_eq!(
        moved(&report),
        vec![(
            "drafts:open".to_owned(),
            "closed".to_owned(),
            Some(plan.landed["drafts:open"].clone())
        )]
    );
    assert_ne!(
        category(&plan.sandbox, &plan.landed["drafts:stay"]),
        "cancelled"
    );
}

#[test]
fn a_closing_copy_into_one_destination_leaves_another_destinations_entry_for_its_own() {
    // The plan is copied into two public folders, so the dropped task's delete records a copy in
    // each. The closing copy into `public` closes that copy and takes out that entry alone; the
    // copy in `mirror` stays open and its entry stays, until the closing copy into `mirror`.
    let plan = PrivatePlan::new(&[("stay", "Stay", &[]), ("open", "Open", &[])]);
    let mirrored = parsed(
        &plan.sandbox,
        &["project", "copy", "drafts:P", "--to", "mirror", "--json"],
    );
    let in_mirror = actions(&mirrored)
        .into_iter()
        .find(|(source, _, _)| source == "drafts:open")
        .and_then(|(_, _, destination)| destination)
        .expect("open's copy in the mirror");
    printed(&plan.sandbox, &["task", "delete", "drafts:open"]);
    let (_, in_public) = plan.landed["drafts:open"].split_once(':').expect("an id");
    let (_, mirrored_native) = in_mirror.split_once(':').expect("an id");
    assert_eq!(
        plan.record(),
        json!({"mirror": {mirrored_native: "drafts:open"}, "public": {in_public: "drafts:open"}})
    );
    let mirror_before = tree(&plan.sandbox.subdirectory("mirror"));

    plan.copy(&["--close-orphans"]);
    assert_eq!(
        category(&plan.sandbox, &plan.landed["drafts:open"]),
        "cancelled"
    );
    assert_eq!(
        plan.record(),
        json!({"mirror": {mirrored_native: "drafts:open"}})
    );
    assert_eq!(tree(&plan.sandbox.subdirectory("mirror")), mirror_before);
    assert_ne!(category(&plan.sandbox, &in_mirror), "cancelled");

    let closing = parsed(
        &plan.sandbox,
        &[
            "project",
            "copy",
            "drafts:P",
            "--to",
            "mirror",
            "--close-orphans",
            "--json",
        ],
    );
    assert_eq!(
        moved(&closing),
        vec![(
            "drafts:open".to_owned(),
            "closed".to_owned(),
            Some(in_mirror.clone())
        )]
    );
    assert_eq!(category(&plan.sandbox, &in_mirror), "cancelled");
    assert_eq!(plan.record(), json!({}));
}
