//! What a plan and a follow-up flow write to a Linear source, driven the way a user drives it.
//!
//! Every journey spawns the compiled binary against the shared stateful fake Linear — a real
//! HTTP server whose one team carries the Hello Patient team's workflow state names and types,
//! two of type `backlog` and two of type `unstarted` among them, so a state written by name and
//! one written as the first of its type land in different places. Each asserts on the exit
//! code, stdout and stderr, and on what the workspace holds afterwards — its raw descriptions
//! byte for byte, and every request it was sent.

use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{LINEAR_TEAM_STATES, LinearWorkspace, document, linear_workspace};

/// The eight-entry mapping the Hello Patient team is configured with.
fn eng_mapping() -> Value {
    json!({
        "backlog": "Proposed",
        "draft": "Backlog",
        "todo": "Todo",
        "queued": "Queued",
        "in-progress": "In Progress",
        "unknown": "Needs Attention",
        "done": "Done",
        "cancelled": "Canceled",
    })
}

/// Every category, in the order a cycle through them never repeats one.
const CATEGORIES: [&str; 8] = [
    "draft",
    "backlog",
    "todo",
    "queued",
    "in-progress",
    "unknown",
    "done",
    "cancelled",
];

/// The state the full mapping writes `category` as.
fn named(category: &str) -> String {
    eng_mapping()[category]
        .as_str()
        .expect("every category is mapped")
        .to_owned()
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
        "`onetaskgraph {}` exited {:?}\nstdout:\n{}\nstderr:\n{}",
        arguments.join(" "),
        output.status.code(),
        stdout(&output),
        stderr(&output)
    );
    output
}

/// A run that had to succeed with one JSON document on stdout.
fn answered(sandbox: &Sandbox, arguments: &[&str]) -> Value {
    let output = exits(sandbox, arguments, 0);
    serde_json::from_str(&stdout(&output)).unwrap_or_else(|error| {
        panic!(
            "`onetaskgraph {}` wrote no JSON ({error}):\n{}",
            arguments.join(" "),
            stdout(&output)
        )
    })
}

/// One task as `task show --json` reads it back.
fn shown(sandbox: &Sandbox, id: &str) -> Value {
    answered(sandbox, &["--json", "task", "show", id])["items"][0]["item"].clone()
}

/// The native ids `task list` answers with, in order.
fn listed(sandbox: &Sandbox, arguments: &[&str]) -> Vec<String> {
    let mut full = vec!["--json", "task", "list"];
    full.extend_from_slice(arguments);
    answered(sandbox, &full)["items"]
        .as_array()
        .expect("a page of tasks")
        .iter()
        .map(|item| {
            item["id"]
                .as_str()
                .expect("a qualified id")
                .split_once(':')
                .expect("source:native")
                .1
                .to_owned()
        })
        .collect()
}

/// The status a task reads back as, as `[category, name]`.
fn status(sandbox: &Sandbox, id: &str) -> Value {
    let item = shown(sandbox, id);
    json!([item["status"]["category"], item["status"]["name"]])
}

/// The type the team gives the workflow state `name`.
fn type_of(name: &str) -> &'static str {
    LINEAR_TEAM_STATES
        .iter()
        .find(|state| state.1 == name)
        .unwrap_or_else(|| panic!("the team has no state {name}"))
        .2
}

/// One issue held at the team's workflow state `state`.
fn issue(id: &str, state: &str, extra: Value) -> Value {
    let kind = type_of(state);
    let category = match kind {
        "unstarted" => "todo",
        "started" => "in-progress",
        "completed" => "done",
        "canceled" => "cancelled",
        "backlog" => "backlog",
        _ => "unknown",
    };
    let mut row = json!({
        "id": id, "title": format!("Issue {id}"), "content": format!("The body of {id}."),
        "status": {"name": state, "category": category},
        "_linear_state": {"name": state, "type": kind},
        "labels": [], "priority": "none",
    });
    for (key, value) in extra.as_object().expect("an object") {
        row[key] = value.clone();
    }
    row
}

/// A workspace holding exactly `tasks`, `projects` and `documents`.
fn held(tasks: Vec<Value>, projects: Vec<Value>, documents: Vec<Value>) -> Value {
    json!({
        "tasks": tasks, "projects": projects, "documents": documents, "labels": [],
        "task_dependencies": [], "project_dependencies": [],
    })
}

fn project(id: &str) -> Value {
    json!({"id": id, "title": format!("Project {id}"), "content": format!("About {id}."),
           "status": {"category": "in-progress", "name": "Doing"}, "labels": []})
}

/// A folder of Markdown holding `files`, whose status words are the category words.
fn folder(sandbox: &Sandbox, name: &str, files: &[(&str, &str)]) -> PathBuf {
    let root = sandbox.subdirectory(name);
    for kind in ["tasks", "projects", "documents"] {
        std::fs::create_dir_all(root.join(kind)).expect("a folder");
    }
    for (relative, text) in files {
        std::fs::write(root.join(relative), text).expect("a file");
    }
    root
}

fn markdown(root: &Path) -> Value {
    let words = CATEGORIES
        .iter()
        .map(|category| ((*category).to_owned(), json!(category)))
        .collect::<serde_json::Map<_, _>>();
    json!({"plugin": "local-md", "config": {"root": root, "status_mapping": words}})
}

/// The Linear source configured over `config`, with `extra` beside the fake's own keys.
fn linear(config: &Value, extra: Value) -> Value {
    let mut config = config.clone();
    for (key, value) in extra.as_object().expect("an object") {
        config[key] = value.clone();
    }
    json!({"plugin": "linear", "config": config})
}

/// Every mutation the workspace answered after the `from`th request, by its root field.
fn mutations_since(workspace: &LinearWorkspace, from: usize) -> Vec<String> {
    workspace
        .served()
        .into_iter()
        .skip(from)
        .filter(|(query, _)| query.trim_start().starts_with("mutation"))
        .map(|(query, _)| {
            query
                .split_once('{')
                .and_then(|(_, rest)| rest.trim_start().split('(').next())
                .unwrap_or_default()
                .to_owned()
        })
        .collect()
}

/// A body file under the sandbox.
fn body(sandbox: &Sandbox, text: &str) -> String {
    let path = sandbox.subdirectory("bodies").join("body.md");
    std::fs::write(&path, text).expect("a body file");
    path.to_string_lossy().into_owned()
}

/// The workspace with an issue at every state a journey reads, and one project to file under.
fn mapped_workspace(sandbox: &Sandbox, mapping: Value) -> (Value, LinearWorkspace) {
    let (config, workspace) = linear_workspace(
        sandbox,
        held(
            vec![
                issue("L-CYCLE", "Todo", json!({})),
                issue("L-UPDATE", "Todo", json!({})),
                issue("L-TODO", "Todo", json!({})),
                issue("L-QUEUED", "Queued", json!({})),
                issue("L-PROPOSED", "Proposed", json!({})),
                issue("L-BACKLOG", "Backlog", json!({})),
                issue("L-REVIEW", "In Review", json!({})),
                issue("L-TRIAGE", "Triage", json!({})),
                issue("L-CANCELED", "Canceled", json!({})),
                issue("L-PROGRESS", "In Progress", json!({})),
                issue("L-ATTENTION", "Needs Attention", json!({})),
            ],
            vec![project("LP-1")],
            Vec::new(),
        ),
    );
    let mut extra = json!({});
    if !mapping.is_null() {
        extra["status_mapping"] = mapping;
    }
    (linear(&config, extra), workspace)
}

#[test]
fn a_mapped_linear_source_writes_every_category_at_its_named_state_by_every_write() {
    let sandbox = Sandbox::new();
    let (source, workspace) = mapped_workspace(&sandbox, eng_mapping());
    let files = CATEGORIES
        .iter()
        .map(|category| {
            (
                format!("tasks/F-{category}.md"),
                format!("---\ntitle: Copied {category}\nstatus: {category}\n---\nbody\n"),
            )
        })
        .collect::<Vec<_>>();
    let files = files
        .iter()
        .map(|(path, text)| (path.as_str(), text.as_str()))
        .collect::<Vec<_>>();
    let plan = folder(&sandbox, "plan", &files);
    sandbox.project_document(&document(
        &json!({"linear": source, "plan": markdown(&plan)}),
    ));

    // A state the mapping names reads as its category under its own name; every other state
    // reads by its type, as without a mapping.
    for (id, expected) in [
        ("linear:L-QUEUED", json!(["queued", "Queued"])),
        ("linear:L-TODO", json!(["todo", "Todo"])),
        ("linear:L-PROPOSED", json!(["backlog", "Proposed"])),
        ("linear:L-BACKLOG", json!(["draft", "Backlog"])),
        ("linear:L-REVIEW", json!(["in-progress", "In Review"])),
        ("linear:L-TRIAGE", json!(["unknown", "Triage"])),
    ] {
        assert_eq!(status(&sandbox, id), expected, "{id}");
    }

    // `--status queued` returns the issues at `Queued` and never one at `Todo`, and the
    // reverse.
    assert_eq!(
        listed(&sandbox, &["--source", "linear", "--status", "queued"]),
        ["L-QUEUED"]
    );
    assert_eq!(
        listed(&sandbox, &["--source", "linear", "--status", "todo"]),
        ["L-CYCLE", "L-UPDATE", "L-TODO"]
    );
    assert_eq!(
        listed(&sandbox, &["--source", "linear", "--status", "draft"]),
        ["L-BACKLOG"]
    );
    assert_eq!(
        listed(&sandbox, &["--source", "linear", "--status", "in-progress"]),
        ["L-REVIEW", "L-PROGRESS"],
        "in-progress is its mapped `In Progress` and `In Review`, an unmapped state of its type, \
         and never `Needs Attention`, a state of that type mapped to another status"
    );
    for (id, expected) in [
        ("linear:L-PROGRESS", json!(["in-progress", "In Progress"])),
        ("linear:L-ATTENTION", json!(["unknown", "Needs Attention"])),
    ] {
        assert_eq!(status(&sandbox, id), expected, "{id}");
    }
    assert_eq!(
        listed(&sandbox, &["--source", "linear", "--status", "unknown"]),
        ["L-TRIAGE", "L-ATTENTION"],
        "unknown is its mapped `Needs Attention` and `Triage`, which reads as it by its type"
    );

    for category in CATEGORIES {
        let state = named(category);
        let expected = json!([category, state]);

        let set = answered(
            &sandbox,
            &[
                "--json",
                "task",
                "status",
                "set",
                "linear:L-CYCLE",
                category,
            ],
        );
        assert_eq!(set["status"]["name"], json!(state), "{category}: {set}");
        assert_eq!(
            workspace.state_of("L-CYCLE").as_deref(),
            Some(state.as_str())
        );
        assert_eq!(status(&sandbox, "linear:L-CYCLE"), expected, "{category}");

        answered(
            &sandbox,
            &[
                "--json",
                "task",
                "update",
                "linear:L-UPDATE",
                "--status",
                category,
            ],
        );
        assert_eq!(
            workspace.state_of("L-UPDATE").as_deref(),
            Some(state.as_str()),
            "{category}: task update"
        );

        let title = format!("Created {category}");
        let file = body(&sandbox, "A created body.");
        answered(
            &sandbox,
            &[
                "--json",
                "task",
                "create",
                "linear",
                "--project",
                "LP-1",
                "--title",
                &title,
                "--status",
                category,
                "--body-file",
                &file,
            ],
        );
        let created = workspace
            .issue_titled(&title)
            .expect("the issue was created");
        assert_eq!(
            workspace.state_of(&created).as_deref(),
            Some(state.as_str()),
            "{category}: task create"
        );
        assert_eq!(
            status(&sandbox, &format!("linear:{created}")),
            expected,
            "{category}: task create"
        );

        let copied = answered(
            &sandbox,
            &[
                "--json",
                "task",
                "copy",
                &format!("plan:F-{category}"),
                "--to",
                "linear",
            ],
        );
        let destination = copied["items"][0]["destination"]
            .as_str()
            .expect("a destination")
            .split_once(':')
            .expect("source:native")
            .1
            .to_owned();
        assert_eq!(
            workspace.state_of(&destination).as_deref(),
            Some(state.as_str()),
            "{category}: a copy"
        );
        assert_eq!(
            status(&sandbox, &format!("linear:{destination}")),
            expected,
            "{category}: a copy"
        );
    }

    // An issue already at its category's named state is not written, by either verb.
    let from = workspace.served().len();
    answered(
        &sandbox,
        &[
            "--json",
            "task",
            "status",
            "set",
            "linear:L-QUEUED",
            "queued",
        ],
    );
    answered(
        &sandbox,
        &[
            "--json",
            "task",
            "update",
            "linear:L-QUEUED",
            "--status",
            "queued",
        ],
    );
    assert_eq!(
        mutations_since(&workspace, from),
        Vec::<String>::new(),
        "no issue update for an issue already at its named state"
    );
}

#[test]
fn a_partly_mapped_linear_source_writes_every_category_it_leaves_out_as_an_unmapped_one_does() {
    let sandbox = Sandbox::new();
    let (source, workspace) =
        mapped_workspace(&sandbox, json!({"backlog": "Proposed", "cancelled": null}));
    sandbox.project_document(&document(&json!({ "linear": source })));

    // The mapped category lands at its named state, and the ones left out at the team's first
    // state of their type — `Todo` over `Queued`, `In Progress` over `Needs Attention`.
    for (category, state) in [
        ("backlog", "Proposed"),
        ("todo", "Todo"),
        ("in-progress", "In Progress"),
        ("done", "Done"),
        ("todo", "Todo"),
    ] {
        answered(
            &sandbox,
            &[
                "--json",
                "task",
                "status",
                "set",
                "linear:L-CYCLE",
                category,
            ],
        );
        assert_eq!(
            workspace.state_of("L-CYCLE").as_deref(),
            Some(state),
            "{category}"
        );
        answered(
            &sandbox,
            &[
                "--json",
                "task",
                "update",
                "linear:L-UPDATE",
                "--status",
                category,
            ],
        );
        assert_eq!(
            workspace.state_of("L-UPDATE").as_deref(),
            Some(state),
            "{category}: task update"
        );
    }

    // `draft`, `queued` and `unknown` are disabled as for a source with no mapping, and
    // `cancelled`, set to null, is disabled although Linear has a state of its type — each
    // refused naming the category, before any write request.
    let from = workspace.served().len();
    for (category, why) in [
        ("draft", "Linear has no workflow state of that kind"),
        ("queued", "Linear has no workflow state of that kind"),
        ("unknown", "Linear has no workflow state of that kind"),
        ("cancelled", "its status_mapping sets cancelled to null"),
    ] {
        for arguments in [
            vec!["task", "status", "set", "linear:L-CYCLE", category],
            vec!["task", "update", "linear:L-CYCLE", "--status", category],
        ] {
            let refused = exits(&sandbox, &arguments, 1);
            let said = stderr(&refused);
            assert!(
                said.contains(&format!("status to {category}")) && said.contains(why),
                "{category}: {said}"
            );
        }
    }
    // A create carries a status too, and one of a category set to null is refused the same
    // way, before the issue is written.
    let file = body(&sandbox, "A created body.");
    let refused = exits(
        &sandbox,
        &[
            "task",
            "create",
            "linear",
            "--project",
            "LP-1",
            "--title",
            "Never written",
            "--status",
            "cancelled",
            "--body-file",
            &file,
        ],
        1,
    );
    assert!(
        stderr(&refused).contains("its status_mapping sets cancelled to null"),
        "{}",
        stderr(&refused)
    );
    assert_eq!(workspace.issue_titled("Never written"), None);
    assert_eq!(mutations_since(&workspace, from), Vec::<String>::new());
    assert_eq!(workspace.state_of("L-CYCLE").as_deref(), Some("Todo"));

    // A category the mapping leaves out narrows by type, excluding the states it names; a
    // mapped one by its name and by the states of its type the mapping leaves to it.
    assert_eq!(
        listed(&sandbox, &["--source", "linear", "--status", "todo"]),
        ["L-CYCLE", "L-UPDATE", "L-TODO", "L-QUEUED"]
    );
    assert_eq!(
        listed(&sandbox, &["--source", "linear", "--status", "backlog"]),
        ["L-PROPOSED", "L-BACKLOG"]
    );
    // `cancelled`, set to null, is never written, and an issue already at a state of its type
    // still reads as it — so `--status cancelled` returns that issue rather than nothing.
    assert_eq!(
        status(&sandbox, "linear:L-CANCELED"),
        json!(["cancelled", "Canceled"])
    );
    assert_eq!(
        listed(&sandbox, &["--source", "linear", "--status", "cancelled"]),
        ["L-CANCELED"]
    );
    // `unknown`, which no type stands for, narrows to the states of a type none of the five
    // categories stand for — `Triage` here — which is what reads as it.
    assert_eq!(
        listed(&sandbox, &["--source", "linear", "--status", "unknown"]),
        ["L-TRIAGE"]
    );
}

#[test]
fn a_mapped_state_the_team_lacks_and_one_state_mapped_twice_are_refused() {
    let sandbox = Sandbox::new();
    let (source, workspace) = mapped_workspace(&sandbox, json!({"queued": "Shipping"}));
    sandbox.project_document(&document(&json!({ "linear": source })));
    let from = workspace.served().len();
    let refused = exits(
        &sandbox,
        &["task", "status", "set", "linear:L-TODO", "queued"],
        1,
    );
    let said = stderr(&refused);
    assert!(
        said.contains("\"Shipping\"")
            && said.contains("team FIX")
            && said.contains("status queued"),
        "{said}"
    );
    assert_eq!(mutations_since(&workspace, from), Vec::<String>::new());

    let sandbox = Sandbox::new();
    let (source, _) = mapped_workspace(&sandbox, json!({"todo": "Todo", "queued": "todo"}));
    sandbox.project_document(&document(&json!({ "linear": source })));
    let refused = run(&sandbox, &["task", "list", "--source", "linear"]);
    assert_ne!(refused.status.code(), Some(0), "{}", stdout(&refused));
    let said = stderr(&refused);
    assert!(
        said.contains("status_mapping sends both todo and queued"),
        "{said}"
    );
}

#[test]
fn sources_fields_reports_each_mapped_state_on_a_linear_team_and_refuses_to_apply() {
    let sandbox = Sandbox::new();
    let (source, workspace) = mapped_workspace(
        &sandbox,
        json!({"queued": "Queued", "todo": "Todo", "done": "Shipped", "draft": null}),
    );
    sandbox.project_document(&document(&json!({ "linear": source })));

    let report = answered(&sandbox, &["--json", "sources", "fields", "linear"]);
    assert_eq!(
        report,
        json!({"source": "linear", "team": "FIX", "states": [
            {"category": "todo", "state": "Todo", "present": true, "type": "unstarted"},
            {"category": "queued", "state": "Queued", "present": true, "type": "unstarted"},
            {"category": "done", "state": "Shipped", "present": false},
        ]})
    );
    let text = stdout(&exits(&sandbox, &["sources", "fields", "linear"], 0));
    assert!(
        text.contains("linear: queued -> Queued: present on team FIX (unstarted)")
            && text.contains("linear: done -> Shipped: missing from team FIX"),
        "{text}"
    );

    // A source whose mapping names no state reports so, in both renderings.
    let sandbox_unmapped = Sandbox::new();
    let (unmapped, _) = mapped_workspace(&sandbox_unmapped, Value::Null);
    sandbox_unmapped.project_document(&document(&json!({ "linear": unmapped })));
    assert_eq!(
        answered(
            &sandbox_unmapped,
            &["--json", "sources", "fields", "linear"]
        ),
        json!({"source": "linear", "team": "FIX", "states": []})
    );
    assert_eq!(
        stdout(&exits(
            &sandbox_unmapped,
            &["sources", "fields", "linear"],
            0
        ))
        .trim_end(),
        "linear: status_mapping names no workflow state of team FIX"
    );

    let from = workspace.served().len();
    let refused = exits(&sandbox, &["sources", "fields", "linear", "--apply"], 1);
    let said = stderr(&refused);
    assert!(
        said.contains("--apply") && said.contains("workflow states are team settings"),
        "{said}"
    );
    assert_eq!(
        workspace.served().len(),
        from,
        "nothing was asked of Linear"
    );
}

/// The workspace a scoped source reads: two projects of the team, an issue and a document in
/// each, and one issue in none.
fn scoped_workspace(sandbox: &Sandbox) -> (Value, LinearWorkspace) {
    // LP-1 is the counterpart of the plan's project PX, so a copy of PX updates the one
    // project a scoped source holds.
    let mut scope = project("LP-1");
    scope["metadata"] = json!({"onetaskgraph.origin": "plan:PX"});
    linear_workspace(
        sandbox,
        held(
            vec![
                issue("I-IN", "Todo", json!({"project": "LP-1"})),
                issue("I-OUT", "Todo", json!({"project": "LP-2"})),
                issue("I-NONE", "Todo", json!({})),
            ],
            vec![scope, project("LP-2")],
            vec![
                json!({"id": "D-IN", "title": "In", "content": "in", "project": "LP-1", "labels": []}),
                json!({"id": "D-OUT", "title": "Out", "content": "out", "project": "LP-2", "labels": []}),
            ],
        ),
    )
}

fn ids(answer: &Value) -> Vec<String> {
    answer["items"]
        .as_array()
        .expect("a page")
        .iter()
        .map(|item| item["id"].as_str().expect("an id").to_owned())
        .collect()
}

#[test]
fn a_project_scoped_linear_source_reads_and_writes_that_project_alone() {
    let sandbox = Sandbox::new();
    let (config, workspace) = scoped_workspace(&sandbox);
    let plan = folder(
        &sandbox,
        "plan",
        &[
            (
                "tasks/F-1.md",
                "---\ntitle: From the plan\nstatus: todo\n---\nbody\n",
            ),
            (
                "projects/PX.md",
                "---\ntitle: The scope's own\nstatus: todo\n---\nbody\n",
            ),
            (
                "projects/PY.md",
                "---\ntitle: Another project\nstatus: todo\n---\nbody\n",
            ),
            ("documents/D-1.md", "---\ntitle: A plan note\n---\nnote\n"),
        ],
    );
    sandbox.project_document(&document(&json!({
        "scoped": linear(&config, json!({"project": "LP-1"})),
        "wide": linear(&config, json!({})),
        "plan": markdown(&plan),
    })));

    assert_eq!(
        ids(&answered(
            &sandbox,
            &["--json", "task", "list", "--source", "scoped"]
        )),
        ["scoped:I-IN"]
    );
    assert_eq!(
        ids(&answered(
            &sandbox,
            &["--json", "project", "list", "--source", "scoped"]
        )),
        ["scoped:LP-1"]
    );
    assert_eq!(
        ids(&answered(
            &sandbox,
            &["--json", "document", "list", "--source", "scoped"]
        )),
        ["scoped:D-IN"]
    );
    // A read by id of anything filed elsewhere is no item of this source, and a document
    // query naming another project holds nothing.
    for (verb, id) in [
        ("task", "scoped:I-OUT"),
        ("project", "scoped:LP-2"),
        ("document", "scoped:D-OUT"),
    ] {
        exits(&sandbox, &[verb, "show", id], 1);
    }
    assert_eq!(
        ids(&answered(
            &sandbox,
            &[
                "--json",
                "document",
                "list",
                "--source",
                "scoped",
                "--project",
                "LP-2"
            ]
        )),
        Vec::<String>::new()
    );

    // Without the key, the same workspace reads team-wide.
    assert_eq!(
        ids(&answered(
            &sandbox,
            &["--json", "task", "list", "--source", "wide"]
        )),
        ["wide:I-IN", "wide:I-OUT", "wide:I-NONE"]
    );
    assert_eq!(
        ids(&answered(
            &sandbox,
            &["--json", "project", "list", "--source", "wide"]
        )),
        ["wide:LP-1", "wide:LP-2"]
    );

    // A task copied in with no project is placed in the scope.
    let copied = answered(
        &sandbox,
        &["--json", "task", "copy", "plan:F-1", "--to", "scoped"],
    );
    let destination = copied["items"][0]["destination"]
        .as_str()
        .expect("a destination")
        .split_once(':')
        .expect("source:native")
        .1
        .to_owned();
    assert_eq!(workspace.project_of(&destination).as_deref(), Some("LP-1"));
    assert!(
        ids(&answered(
            &sandbox,
            &["--json", "task", "list", "--source", "scoped"]
        ))
        .contains(&format!("scoped:{destination}")),
        "the copy reads back through the scoped source"
    );

    // One created in the scope lands there; one naming another project is refused naming both.
    let file = body(&sandbox, "A created body.");
    answered(
        &sandbox,
        &[
            "--json",
            "task",
            "create",
            "scoped",
            "--project",
            "LP-1",
            "--title",
            "Scoped",
            "--body-file",
            &file,
        ],
    );
    let created = workspace.issue_titled("Scoped").expect("created");
    assert_eq!(workspace.project_of(&created).as_deref(), Some("LP-1"));
    let from = workspace.served().len();
    let refused = exits(
        &sandbox,
        &[
            "task",
            "create",
            "scoped",
            "--project",
            "LP-2",
            "--title",
            "Elsewhere",
            "--body-file",
            &file,
        ],
        1,
    );
    let said = stderr(&refused);
    assert!(said.contains("LP-1") && said.contains("LP-2"), "{said}");
    assert_eq!(mutations_since(&workspace, from), Vec::<String>::new());

    // A document copied in with no project is filed under the scope; one created naming
    // another project is refused naming both.
    let copied = answered(
        &sandbox,
        &["--json", "document", "copy", "plan:D-1", "--to", "scoped"],
    );
    let destination = copied["items"][0]["destination"]
        .as_str()
        .expect("a destination")
        .split_once(':')
        .expect("source:native")
        .1
        .to_owned();
    assert_eq!(
        workspace.document_project(&destination).as_deref(),
        Some("LP-1")
    );
    let from = workspace.served().len();
    let refused = exits(
        &sandbox,
        &[
            "document",
            "create",
            "scoped",
            "--project",
            "LP-2",
            "--title",
            "Elsewhere",
            "--body-file",
            &file,
        ],
        1,
    );
    let said = stderr(&refused);
    assert!(said.contains("LP-1") && said.contains("LP-2"), "{said}");
    assert_eq!(mutations_since(&workspace, from), Vec::<String>::new());
    answered(
        &sandbox,
        &[
            "--json",
            "document",
            "create",
            "scoped",
            "--project",
            "LP-1",
            "--title",
            "Scoped note",
            "--body-file",
            &file,
        ],
    );
    let created = workspace.document_titled("Scoped note").expect("created");
    assert_eq!(
        workspace.document_project(&created).as_deref(),
        Some("LP-1")
    );

    // The one project it holds is updated by a copy of its counterpart; any other project is
    // refused naming the scope, writing nothing.
    let updated = answered(
        &sandbox,
        &["--json", "project", "copy", "plan:PX", "--to", "scoped"],
    );
    assert_eq!(
        updated["items"][0]["destination"], "scoped:LP-1",
        "{updated:#}"
    );
    let from = workspace.served().len();
    let refused = exits(
        &sandbox,
        &["project", "copy", "plan:PY", "--to", "scoped"],
        1,
    );
    let said = stderr(&refused);
    assert!(said.contains("scoped to the Linear project LP-1"), "{said}");
    assert_eq!(mutations_since(&workspace, from), Vec::<String>::new());
}

/// An issue, a project and a document each carrying prose with its own spacing and a slot
/// closed the way Linear hands one back.
fn slotted_workspace(sandbox: &Sandbox) -> (Value, LinearWorkspace) {
    let held_slot = "Prose a person wrote,\n\n   spaced their own way.  \n\n<!-- onetaskgraph.metadata\n{\"caller.kept\":[1,{\"deep\":true}]}\n\\-->";
    let mut task = issue("I-M", "Todo", json!({"project": "LP-1"}));
    task["_linear_description"] = json!(held_slot);
    let mut item = project("LP-1");
    item["_linear_description"] = json!(held_slot);
    linear_workspace(
        sandbox,
        held(
            vec![task],
            vec![item],
            vec![
                json!({"id": "D-M", "title": "A document", "content": "", "project": "LP-1",
                        "labels": [], "_linear_description": held_slot}),
            ],
        ),
    )
}

/// `field` split at its trailing slot: the bytes above it, and the slot's parsed JSON.
///
/// Either spelling: the one-line code span a source writes, and the multi-line one a
/// workspace seeded before it holds.
fn split_slot(field: &str) -> (String, Value) {
    let start = field
        .rfind("<!-- onetaskgraph.metadata")
        .expect("the field ends in a slot");
    let slot = &field[start + "<!-- onetaskgraph.metadata".len()..];
    let encoded = match slot.strip_prefix(" `") {
        Some(span) => span.trim_end().trim_end_matches("` -->"),
        None => slot
            .trim_start_matches('\n')
            .trim_end_matches("-->")
            .trim_end_matches('\\')
            .trim_end_matches('\n'),
    };
    (
        field[..start].to_owned(),
        serde_json::from_str(encoded).expect("the slot holds JSON"),
    )
}

#[test]
fn a_metadata_set_moves_only_the_slot_and_a_render_only_the_body_and_its_provenance_on_linear() {
    let sandbox = Sandbox::new();
    let (config, workspace) = slotted_workspace(&sandbox);
    let plan = folder(&sandbox, "plan", &[]);
    sandbox.project_document(&document(&json!({
        "linear": linear(&config, json!({})),
        "plan": markdown(&plan),
    })));

    for (verb, kind, id) in [
        ("task", "tasks", "I-M"),
        ("project", "projects", "LP-1"),
        ("document", "documents", "D-M"),
    ] {
        let before = workspace.long_form(kind, id).expect("held");
        let (above_before, slot_before) = split_slot(&before);
        let from = workspace.served().len();
        let answer = answered(
            &sandbox,
            &[
                "--json",
                verb,
                "metadata",
                "set",
                &format!("linear:{id}"),
                "myapp.review",
                r#"{"approved":true}"#,
            ],
        );
        assert_eq!(
            answer["value"],
            json!({"approved": true}),
            "{verb}: {answer}"
        );
        let after = workspace.long_form(kind, id).expect("held");
        let (above_after, slot_after) = split_slot(&after);
        assert_eq!(
            above_after, above_before,
            "{verb}: every byte above the slot"
        );
        let mut expected = slot_before.clone();
        expected["myapp.review"] = json!({"approved": true});
        assert_eq!(slot_after, expected, "{verb}: the slot gained one key");
        assert_eq!(
            mutations_since(&workspace, from).len(),
            1,
            "{verb}: one write"
        );

        // The same value again is a write that changes nothing, and sends none.
        let from = workspace.served().len();
        answered(
            &sandbox,
            &[
                "--json",
                verb,
                "metadata",
                "set",
                &format!("linear:{id}"),
                "myapp.review",
                r#"{"approved":true}"#,
            ],
        );
        assert_eq!(mutations_since(&workspace, from), Vec::<String>::new());
    }

    // A rendering replaces the body and the provenance together, and nothing else.
    let template = sandbox.subdirectory("templates").join("note.md");
    std::fs::write(&template, "A rendered body.\n").expect("a template");
    let template = template.to_string_lossy().into_owned();
    for (verb, kind, id) in [("task", "tasks", "I-M"), ("document", "documents", "D-M")] {
        let (_, slot_before) = split_slot(&workspace.long_form(kind, id).expect("held"));
        let regenerated = answered(
            &sandbox,
            &[
                "--json",
                verb,
                "render",
                &format!("linear:{id}"),
                "--template",
                &template,
                "--no-interactive",
            ],
        );
        assert_eq!(regenerated["changed"], true, "{verb}: {regenerated}");
        let after = workspace.long_form(kind, id).expect("held");
        let (above_after, slot_after) = split_slot(&after);
        assert_eq!(above_after, "A rendered body.\n\n\n", "{verb}: the body");
        let mut kept = slot_after.clone();
        let provenance = kept
            .as_object_mut()
            .expect("a slot")
            .remove("onetaskgraph.template")
            .expect("the provenance is recorded");
        assert_eq!(provenance["template"], json!(template), "{verb}");
        assert_eq!(kept, slot_before, "{verb}: every other slot entry kept");
    }

    // A copy out of Linear records where the item landed on the Linear item itself.
    let copied = answered(
        &sandbox,
        &["--json", "task", "copy", "linear:I-M", "--to", "plan"],
    );
    assert_eq!(copied["items"][0]["link"], "recorded", "{copied:#}");
    let (_, slot) = split_slot(&workspace.long_form("tasks", "I-M").expect("held"));
    assert_eq!(
        slot["onetaskgraph.copies"],
        json!({"plan": copied["items"][0]["destination"]})
    );
    // And the next copy follows it.
    let again = answered(
        &sandbox,
        &["--json", "task", "copy", "linear:I-M", "--to", "plan"],
    );
    assert_eq!(again["items"][0]["via"], "link", "{again:#}");
}

#[test]
fn a_linear_ticket_is_delivered_from_another_source_through_its_mapped_states() {
    let sandbox = Sandbox::new();
    let (config, workspace) = linear_workspace(
        &sandbox,
        held(
            vec![
                issue("L-T", "Todo", json!({})),
                issue("L-P", "Proposed", json!({})),
                issue("L-B", "Backlog", json!({})),
            ],
            Vec::new(),
            Vec::new(),
        ),
    );
    let plan = folder(
        &sandbox,
        "plan",
        &[
            (
                "tasks/D-1.md",
                "---\ntitle: Deliverer\nstatus: todo\n---\nbody\n",
            ),
            (
                "tasks/D-2.md",
                "---\ntitle: Idle deliverer\nstatus: todo\n---\nbody\n",
            ),
        ],
    );
    sandbox.project_document(&document(&json!({
        "linear": linear(&config, json!({"status_mapping": eng_mapping()})),
        "plan": markdown(&plan),
    })));

    // The relation is set on the deliverer, and the store keeps the ticket's own side in its
    // slot.
    answered(
        &sandbox,
        &[
            "--json",
            "task",
            "update",
            "plan:D-1",
            "--delivers",
            "linear:L-T",
        ],
    );
    assert_eq!(
        shown(&sandbox, "linear:L-T")["delivered_by"],
        json!(["plan:D-1"])
    );
    let (_, slot) = split_slot(&workspace.long_form("tasks", "L-T").expect("held"));
    assert_eq!(slot["onetaskgraph.delivered_by"], json!(["plan:D-1"]));

    for (deliverer, ticket) in [
        ("queued", "Queued"),
        ("in-progress", "In Progress"),
        ("done", "Done"),
    ] {
        let set = answered(
            &sandbox,
            &["--json", "task", "status", "set", "plan:D-1", deliverer],
        );
        assert_eq!(set["delivered"][0]["outcome"], "written", "{set:#}");
        assert_eq!(
            workspace.state_of("L-T").as_deref(),
            Some(ticket),
            "a {deliverer} deliverer"
        );
        assert_eq!(status(&sandbox, "linear:L-T"), json!([deliverer, ticket]));
    }

    // A ticket at `Proposed` or `Backlog` is left alone however its deliverer moves.
    answered(
        &sandbox,
        &[
            "--json",
            "task",
            "update",
            "plan:D-2",
            "--delivers",
            "linear:L-P",
            "--delivers",
            "linear:L-B",
        ],
    );
    let set = answered(
        &sandbox,
        &["--json", "task", "status", "set", "plan:D-2", "in-progress"],
    );
    assert_eq!(
        set["delivered"]
            .as_array()
            .expect("entries")
            .iter()
            .map(|entry| entry["outcome"].clone())
            .collect::<Vec<_>>(),
        [json!("left"), json!("left")],
        "{set:#}"
    );
    assert_eq!(workspace.state_of("L-P").as_deref(), Some("Proposed"));
    assert_eq!(workspace.state_of("L-B").as_deref(), Some("Backlog"));

    // A Linear task's own `delivers` is held in its slot under the reserved key.
    answered(
        &sandbox,
        &[
            "--json",
            "task",
            "update",
            "linear:L-P",
            "--delivers",
            "plan:D-2",
        ],
    );
    let (_, slot) = split_slot(&workspace.long_form("tasks", "L-P").expect("held"));
    assert_eq!(slot["onetaskgraph.delivers"], json!(["plan:D-2"]));
    assert_eq!(
        shown(&sandbox, "linear:L-P")["delivers"],
        json!(["plan:D-2"])
    );
}

/// The issues a follow-up search tells apart: one that matches every predicate, one whose
/// prose carries every searched phrase outside its slot, and one whose slot holds other values.
fn searched_workspace(sandbox: &Sandbox) -> (Value, LinearWorkspace) {
    let slot = |json: &str| format!("\n\n<!-- onetaskgraph.metadata\n{json}\n-->");
    let mut matching = issue(
        "S-MATCH",
        "Todo",
        json!({"title": "Alpha follow-up", "priority": "high"}),
    );
    matching["_linear_description"] = json!(format!(
        "The body.{}",
        slot(
            r#"{"caller.key":"v","onetaskgraph.origin":"plan:ORIG-1","orchestrator.follow-up":{"root_cause":"stale-cache"}}"#
        )
    ));
    let mut prose = issue(
        "S-PROSE",
        "Todo",
        json!({"title": "Second", "priority": "low"}),
    );
    prose["_linear_description"] = json!(
        "Quotes \"caller.key\":\"v\", \"onetaskgraph.origin\":\"plan:ORIG-1\" and \
         \"root_cause\":\"stale-cache\" in its prose, and the word zebra."
    );
    let mut other = issue(
        "S-OTHER",
        "Todo",
        json!({"title": "Third", "priority": "low"}),
    );
    other["_linear_description"] = json!(format!(
        "Another body.{}",
        slot(
            r#"{"caller.key":"w","onetaskgraph.origin":"plan:ORIG-2","orchestrator.follow-up":{"root_cause":"other"}}"#
        )
    ));
    // A slot in the multi-line spelling, spaced by hand: readable, and so a match.
    let mut legacy = issue("S-LEGACY", "Todo", json!({"title": "Fourth"}));
    legacy["_linear_description"] = json!(
        "Legacy body.\n\n<!-- onetaskgraph.metadata\n{\"caller.key\": \"v\", \
         \"onetaskgraph.origin\": \"plan:ORIG-1\"}\n-->"
    );
    let mut workspace = held(vec![matching, prose, other, legacy], Vec::new(), Vec::new());
    workspace["comments"] = json!([
        {"task": "S-MATCH", "comment": {"id": "C-1", "body": "recent",
            "created_at": "2026-09-01T00:00:00Z", "updated_at": "2026-10-01T12:00:00Z"}},
        {"task": "S-OTHER", "comment": {"id": "C-2", "body": "old",
            "created_at": "2026-09-01T00:00:00Z", "updated_at": "2026-09-01T00:00:00Z"}},
    ]);
    linear_workspace(sandbox, workspace)
}

#[test]
fn the_follow_up_searches_are_pushed_to_linear_and_return_exactly_what_matches() {
    let sandbox = Sandbox::new();
    let (config, workspace) = searched_workspace(&sandbox);
    let plan = folder(&sandbox, "plan", &[]);
    sandbox.project_document(&document(&json!({
        "linear": linear(&config, json!({})),
        "plan": markdown(&plan),
    })));
    for (arguments, predicate, expected) in [
        (
            vec!["--metadata", "caller.key=v"],
            "metadata",
            vec!["S-MATCH", "S-LEGACY"],
        ),
        (
            vec![
                "--metadata",
                "orchestrator.follow-up/root_cause=stale-cache",
            ],
            "metadata",
            vec!["S-MATCH"],
        ),
        (
            vec!["--origin", "plan:ORIG-1"],
            "origin",
            vec!["S-MATCH", "S-LEGACY"],
        ),
        (vec!["--priority", "high"], "priority", vec!["S-MATCH"]),
        (
            vec!["--commented-since", "2026-10-01T00:00:00Z"],
            "commented-since",
            vec!["S-MATCH"],
        ),
        (
            vec!["--search", "ALPHA", "--in", "title"],
            "search-title",
            vec!["S-MATCH"],
        ),
        (
            vec!["--search", "zebra", "--in", "content"],
            "search-content",
            vec!["S-PROSE"],
        ),
        (
            vec!["--search", "stale-cache", "--in", "content"],
            "search-content",
            vec!["S-PROSE"],
        ),
    ] {
        let mut full = vec!["--json", "task", "list", "--source", "linear"];
        full.extend_from_slice(&arguments);
        let from = workspace.served().len();
        let answer = answered(&sandbox, &full);
        assert_eq!(
            ids(&answer),
            expected
                .iter()
                .map(|id| format!("linear:{id}"))
                .collect::<Vec<_>>(),
            "{arguments:?}"
        );
        assert!(
            answer["plan"]["per_source"][0]["pushed_down"]
                .as_array()
                .expect("a plan")
                .contains(&json!(predicate)),
            "{arguments:?}: {:#}",
            answer["plan"]
        );
        // Linear was asked for the narrowing, not for the whole team.
        let filter = workspace
            .served()
            .into_iter()
            .skip(from)
            .find(|(query, _)| query == onetaskgraph_linear::graphql::ISSUES)
            .expect("one issue read")
            .1["filter"]
            .to_string();
        let member = match predicate {
            "metadata" | "origin" => "\"description\":{\"contains\"",
            "priority" => "\"priority\":{\"in\"",
            "commented-since" => "\"comments\":{\"some\"",
            "search-title" => "\"title\":{\"containsIgnoreCase\"",
            _ => "\"description\":{\"containsIgnoreCase\"",
        };
        assert!(filter.contains(member), "{arguments:?}: {filter}");
    }
}

#[test]
fn a_cross_source_edge_is_established_both_ways_and_never_taken_from_another_team_or_project() {
    for scope in [None, Some("LP-1")] {
        let sandbox = Sandbox::new();
        // A decoy in another team carries the far end's origin, and — for a scoped source —
        // so does one of this team filed in another project. Neither may stand for it. (To a
        // source with no `project`, an issue of its own team in any project is its own, so the
        // second decoy is only planted where it is one.)
        let decoy_slot =
            "Decoy.\n\n<!-- onetaskgraph.metadata\n{\"onetaskgraph.origin\":\"plan:F-1\"}\n-->";
        let mut other_team = issue("X-TEAM", "Todo", json!({"team": "OTHER"}));
        other_team["_linear_description"] = json!(decoy_slot);
        let mut other_project = issue("X-PROJECT", "Todo", json!({"project": "LP-2"}));
        other_project["_linear_description"] = json!(decoy_slot);
        let mut tasks = vec![
            other_team,
            issue("L-1", "Todo", json!({"project": "LP-1"})),
            issue("L-2", "Todo", json!({"project": "LP-1"})),
        ];
        if scope.is_some() {
            tasks.insert(1, other_project);
        }
        let (config, workspace) = linear_workspace(
            &sandbox,
            held(tasks, vec![project("LP-1"), project("LP-2")], Vec::new()),
        );
        let extra = scope.map_or_else(|| json!({}), |scope| json!({"project": scope}));
        let linear_source = linear(&config, extra);
        let plan = folder(
            &sandbox,
            "plan",
            &[
                (
                    "tasks/F-1.md",
                    "---\ntitle: Upstream\nstatus: todo\n---\nbody\n",
                ),
                (
                    "tasks/F-2.md",
                    "---\ntitle: Waits on upstream\nstatus: todo\ndepends_on: [F-1]\n---\nbody\n",
                ),
            ],
        );
        sandbox.project_document(&document(&json!({
            "linear": linear_source,
            "plan": markdown(&plan),
        })));
        // L-2 depends on L-1, natively.
        answered(
            &sandbox,
            &[
                "--json",
                "task",
                "update",
                "linear:L-2",
                "--depends-on",
                "linear:L-1",
            ],
        );

        // Linear → plan: a copy whose far end stayed behind records it, and the decoys
        // carrying that far end's origin are not taken for it.
        let copied = answered(
            &sandbox,
            &["--json", "task", "copy", "plan:F-2", "--to", "linear"],
        );
        let near = copied["items"][0]["destination"]
            .as_str()
            .expect("a destination")
            .to_owned();
        let far: Vec<Value> = answered(&sandbox, &["--json", "task", "deps", &near])["items"]
            .as_array()
            .expect("edges")
            .iter()
            .map(|edge| edge["to"]["id"].clone())
            .collect();
        assert_eq!(far, [json!("plan:F-1")], "{scope:?}");
        for decoy in ["X-TEAM", "X-PROJECT"] {
            assert!(
                !workspace
                    .long_form("tasks", near.split_once(':').expect("qualified").1)
                    .expect("held")
                    .contains(decoy),
                "{scope:?}: {decoy}"
            );
        }

        // plan → Linear: a copy out of Linear keeps its far end in Linear.
        let copied = answered(
            &sandbox,
            &["--json", "task", "copy", "linear:L-2", "--to", "plan"],
        );
        let near = copied["items"][0]["destination"]
            .as_str()
            .expect("a destination")
            .to_owned();
        let far: Vec<Value> = answered(&sandbox, &["--json", "task", "deps", &near])["items"]
            .as_array()
            .expect("edges")
            .iter()
            .map(|edge| edge["to"]["id"].clone())
            .collect();
        assert_eq!(far, [json!("linear:L-1")], "{scope:?}");

        // Once the far end itself is copied in, the next copy relates the two natively —
        // to the copy in this team and project, never to a decoy.
        let upstream = answered(
            &sandbox,
            &["--json", "task", "copy", "plan:F-1", "--to", "linear"],
        );
        let upstream = upstream["items"][0]["destination"].clone();
        let again = answered(
            &sandbox,
            &["--json", "task", "copy", "plan:F-2", "--to", "linear"],
        );
        let near = again["items"][0]["destination"]
            .as_str()
            .expect("a destination")
            .to_owned();
        let far: Vec<Value> = answered(&sandbox, &["--json", "task", "deps", &near])["items"]
            .as_array()
            .expect("edges")
            .iter()
            .map(|edge| edge["to"]["id"].clone())
            .collect();
        assert_eq!(far, [upstream], "{scope:?}");
    }
}
