//! A Linear source's statuses by item kind, driven the way a user drives them.
//!
//! Every journey spawns the compiled binary against the shared stateful fake Linear — a real
//! HTTP server holding one team's workflow states and the workspace's project statuses — and
//! asserts on the exit code, stdout and stderr, and on what the workspace holds afterwards:
//! which state an issue is at, which status a project is at, the names its vocabularies hold,
//! and every request it was sent.
//!
//! The vocabulary is Hello Patient's: its project statuses as `projectStatuses` answered on
//! 2026-10-05, and the team states this host maps, with `Triage` and `In Review` beside them
//! so an unmapped name has something to read. The mapping is the one ai-orchestrator configures
//! for it.

use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{LinearWorkspace, document, linear_workspace_with};

/// The `hellopatient` source's `status_mapping`, exactly as ai-orchestrator writes it.
pub(crate) fn hellopatient_mapping() -> Value {
    json!({
        "backlog":     {"task": "Proposed",        "project": "Proposal"},
        "draft":       {"task": "Backlog",         "project": "Idea"},
        "todo":        {"task": "Todo",            "project": "Planned"},
        "queued":      {"task": "Queued",          "project": "Accepted"},
        "in-progress": "In Progress",
        "unknown":     {"task": "Needs Attention", "project": "Blocked"},
        "done":        {"task": "Done",            "project": "Completed"},
        "cancelled":   "Canceled",
    })
}

/// The team's workflow states: the eight the mapping names, and `Triage` and `In Review`.
pub(crate) const TEAM_STATES: &[(&str, &str, &str)] = &[
    ("S-proposed", "Proposed", "backlog"),
    ("S-backlog", "Backlog", "backlog"),
    ("S-todo", "Todo", "unstarted"),
    ("S-queued", "Queued", "unstarted"),
    ("S-in-progress", "In Progress", "started"),
    ("S-needs-attention", "Needs Attention", "started"),
    ("S-done", "Done", "completed"),
    ("S-canceled", "Canceled", "canceled"),
    ("S-triage", "Triage", "triage"),
    ("S-in-review", "In Review", "started"),
];

/// Hello Patient's project statuses, in Linear's order.
pub(crate) const PROJECT_STATUSES: &[(&str, &str)] = &[
    ("Idea", "backlog"),
    ("Proposal", "backlog"),
    ("Backlog", "backlog"),
    ("Discovery", "planned"),
    ("Planned", "planned"),
    ("Accepted", "planned"),
    ("Requirements Gathering", "started"),
    ("In Design", "started"),
    ("PRD Review", "started"),
    ("In Progress", "started"),
    ("Blocked", "started"),
    ("Maintenance", "completed"),
    ("Completed", "completed"),
    ("Canceled", "canceled"),
];

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

/// The name `mapping` gives `category` for `kind`, if it gives one.
fn named(mapping: &Value, category: &str, kind: &str) -> Option<String> {
    match &mapping[category] {
        Value::String(name) => Some(name.clone()),
        Value::Object(names) => names.get(kind).and_then(Value::as_str).map(str::to_owned),
        _ => None,
    }
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

/// A run that had to fail, whatever its non-zero code; its stderr.
fn failed(sandbox: &Sandbox, arguments: &[&str]) -> String {
    let output = run(sandbox, arguments);
    assert!(
        !output.status.success(),
        "`onetaskgraph {}` succeeded where it had to be refused\nstdout:\n{}",
        arguments.join(" "),
        stdout(&output)
    );
    stderr(&output)
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

/// What `<verb> show --json` reads one item back as, as `[category, name]`.
fn status(sandbox: &Sandbox, verb: &str, id: &str) -> Value {
    let item = answered(sandbox, &["--json", verb, "show", id])["items"][0]["item"].clone();
    json!([item["status"]["category"], item["status"]["name"]])
}

/// The native ids `<verb> list --status <category>` answers with over the `linear` source.
fn listed(sandbox: &Sandbox, verb: &str, category: &str) -> Vec<String> {
    let mut ids = answered(
        sandbox,
        &[
            "--json", verb, "list", "--source", "linear", "--status", category, "--limit", "100",
        ],
    )["items"]
        .as_array()
        .expect("a page")
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
        .collect::<Vec<_>>();
    ids.sort();
    ids
}

/// One issue held at the workflow state `name` of `states`.
fn issue(id: &str, name: &str, states: &[(&str, &str, &str)], extra: Value) -> Value {
    let (_, _, kind) = states
        .iter()
        .find(|state| state.1 == name)
        .unwrap_or_else(|| panic!("the team has no state {name}"));
    let mut row = json!({
        "id": id, "title": format!("Issue {id}"), "content": format!("The body of {id}."),
        "status": {"name": name, "category": "unknown"},
        "_linear_state": {"name": name, "type": kind},
        "labels": [], "priority": "none",
    });
    for (key, value) in extra.as_object().expect("an object") {
        row[key] = value.clone();
    }
    row
}

/// One project held at the project status `name` of `statuses`.
fn project(id: &str, name: &str, statuses: &[(&str, &str)], extra: Value) -> Value {
    let (_, kind) = statuses
        .iter()
        .find(|status| status.0 == name)
        .unwrap_or_else(|| panic!("the workspace has no project status {name}"));
    let mut row = json!({
        "id": id, "title": format!("Project {id}"), "content": format!("About {id}."),
        "status": {"name": name, "category": "unknown",
                   "_linear_status": {"name": name, "type": kind}},
        "labels": [],
    });
    for (key, value) in extra.as_object().expect("an object") {
        row[key] = value.clone();
    }
    row
}

/// A workspace holding exactly `tasks` and `projects`.
fn held(tasks: Vec<Value>, projects: Vec<Value>) -> Value {
    json!({
        "tasks": tasks, "projects": projects, "documents": [], "labels": [],
        "task_dependencies": [], "project_dependencies": [],
    })
}

/// The Linear source configured over `config`, with `extra` beside the fake's own keys.
fn linear(config: &Value, extra: Value) -> Value {
    let mut config = config.clone();
    for (key, value) in extra.as_object().expect("an object") {
        config[key] = value.clone();
    }
    json!({"plugin": "linear", "config": config})
}

/// A folder of Markdown holding `files`, whose status words are the category words.
fn folder(sandbox: &Sandbox, name: &str, files: &[(String, String)]) -> PathBuf {
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

/// A body file under the sandbox.
fn body(sandbox: &Sandbox) -> String {
    let path = sandbox.subdirectory("bodies").join("body.md");
    std::fs::write(&path, "A created body.").expect("a body file");
    path.to_string_lossy().into_owned()
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

/// The native half of the destination a copy report names for its first item.
fn destination(report: &Value) -> String {
    report["items"][0]["destination"]
        .as_str()
        .unwrap_or_else(|| panic!("a destination: {report:#}"))
        .split_once(':')
        .expect("source:native")
        .1
        .to_owned()
}

#[test]
fn every_category_is_written_and_read_back_for_both_kinds_under_its_mapped_name() {
    let sandbox = Sandbox::new();
    let mapping = hellopatient_mapping();
    let (config, workspace) = linear_workspace_with(
        &sandbox,
        held(
            vec![
                issue("L-CYCLE", "Todo", TEAM_STATES, json!({})),
                issue("L-UPDATE", "Todo", TEAM_STATES, json!({})),
            ],
            // The counterpart of the plan's project `cycle`, so each copy of it updates this.
            vec![project(
                "LP-CYCLE",
                "Planned",
                PROJECT_STATUSES,
                json!({"metadata": {"onetaskgraph.origin": "plan:cycle"}}),
            )],
        ),
        TEAM_STATES,
        PROJECT_STATUSES,
    );
    let mut files = Vec::new();
    for category in CATEGORIES {
        files.push((
            format!("tasks/F-{category}.md"),
            format!("---\ntitle: Copied {category}\nstatus: {category}\n---\nbody\n"),
        ));
        files.push((
            format!("projects/PR-{category}.md"),
            format!("---\ntitle: Project {category}\nstatus: {category}\n---\nbody\n"),
        ));
    }
    let plan = folder(&sandbox, "plan", &files);
    sandbox.project_document(&document(&json!({
        "linear": linear(&config, json!({"status_mapping": mapping})),
        "plan": markdown(&plan),
    })));
    let file = body(&sandbox);

    for category in CATEGORIES {
        let task = named(&mapping, category, "task").expect("every category names a task");
        let project_name =
            named(&mapping, category, "project").expect("every category names a project");
        let task_read = json!([category, task]);
        let project_read = json!([category, project_name]);

        // A task, by every verb that sets its status.
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
        assert_eq!(set["status"]["name"], json!(task), "{category}: {set}");
        assert_eq!(
            workspace.state_of("L-CYCLE"),
            Some(task.clone()),
            "{category}"
        );
        assert_eq!(status(&sandbox, "task", "linear:L-CYCLE"), task_read);

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
            workspace.state_of("L-UPDATE"),
            Some(task.clone()),
            "{category}"
        );
        assert_eq!(status(&sandbox, "task", "linear:L-UPDATE"), task_read);

        let title = format!("Created {category}");
        answered(
            &sandbox,
            &[
                "--json",
                "task",
                "create",
                "linear",
                "--project",
                "LP-CYCLE",
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
            workspace.state_of(&created),
            Some(task.clone()),
            "{category}"
        );
        assert_eq!(
            status(&sandbox, "task", &format!("linear:{created}")),
            task_read
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
        let landed = destination(&copied);
        assert_eq!(
            workspace.state_of(&landed),
            Some(task.clone()),
            "{category}"
        );
        assert_eq!(
            status(&sandbox, "task", &format!("linear:{landed}")),
            task_read
        );

        // A project, created and updated, through the project side of the mapping.
        let copied = answered(
            &sandbox,
            &[
                "--json",
                "project",
                "copy",
                &format!("plan:PR-{category}"),
                "--to",
                "linear",
                "--no-tasks",
            ],
        );
        let created = destination(&copied);
        assert_eq!(
            workspace.status_of(&created),
            Some(project_name.clone()),
            "{category}: a project created"
        );
        assert_eq!(
            status(&sandbox, "project", &format!("linear:{created}")),
            project_read
        );

        std::fs::write(
            plan.join("projects/cycle.md"),
            format!("---\ntitle: The cycle\nstatus: {category}\n---\nbody\n"),
        )
        .expect("the plan's project");
        let copied = answered(
            &sandbox,
            &[
                "--json",
                "project",
                "copy",
                "plan:cycle",
                "--to",
                "linear",
                "--no-tasks",
            ],
        );
        assert_eq!(destination(&copied), "LP-CYCLE", "{copied:#}");
        assert_eq!(
            workspace.status_of("LP-CYCLE"),
            Some(project_name.clone()),
            "{category}: a project updated"
        );
        assert_eq!(status(&sandbox, "project", "linear:LP-CYCLE"), project_read);
    }
}

#[test]
fn every_status_write_a_linear_source_has_no_name_for_is_refused_before_any_mutation() {
    let sandbox = Sandbox::new();
    // `draft` is not mentioned, `cancelled` is disabled, `queued` names a task alone and
    // `backlog` a project alone, and `done` names a state and a status Linear does not have.
    let mapping = json!({
        "todo": {"task": "Todo", "project": "Planned"},
        "queued": {"task": "Queued"},
        "backlog": {"project": "Proposal"},
        "cancelled": null,
        "done": {"task": "Shipping", "project": "Shipped"},
    });
    let (config, workspace) = linear_workspace_with(
        &sandbox,
        held(
            vec![issue("L-1", "Todo", TEAM_STATES, json!({}))],
            vec![project(
                "LP-1",
                "Planned",
                PROJECT_STATUSES,
                json!({"metadata": {"onetaskgraph.origin": "plan:existing"}}),
            )],
        ),
        TEAM_STATES,
        PROJECT_STATUSES,
    );
    let mut files = Vec::new();
    for category in ["draft", "cancelled", "queued", "backlog", "done"] {
        files.push((
            format!("tasks/F-{category}.md"),
            format!("---\ntitle: Refused {category}\nstatus: {category}\n---\nbody\n"),
        ));
        files.push((
            format!("projects/PR-{category}.md"),
            format!("---\ntitle: Refused project {category}\nstatus: {category}\n---\nbody\n"),
        ));
    }
    let plan = folder(&sandbox, "plan", &files);
    sandbox.project_document(&document(&json!({
        "linear": linear(&config, json!({"status_mapping": mapping})),
        "unmapped": linear(&config, json!({"status_mapping": {}})),
        "plan": markdown(&plan),
    })));
    let file = body(&sandbox);

    // What each refusal has to name beside the source, the kind and the category.
    let task_cases = [
        ("draft", "status_mapping.draft.task"),
        ("cancelled", "status_mapping.cancelled.task"),
        ("backlog", "status_mapping.backlog.task"),
        ("done", "\"Shipping\""),
    ];
    let project_cases = [
        ("draft", "status_mapping.draft.project"),
        ("cancelled", "status_mapping.cancelled.project"),
        ("queued", "status_mapping.queued.project"),
        ("done", "\"Shipped\""),
    ];
    let check = |said: &str, kind: &str, category: &str, names: &str, verb: &str| {
        assert!(
            said.contains("source linear")
                && said.contains(&format!("{kind} status"))
                && said.contains(category)
                && said.contains(names),
            "{verb} of a {kind} at {category}: {said}"
        );
    };
    for (category, names) in task_cases {
        for (verb, arguments) in [
            (
                "task create",
                vec![
                    "task",
                    "create",
                    "linear",
                    "--project",
                    "LP-1",
                    "--title",
                    "Never written",
                    "--status",
                    category,
                    "--body-file",
                    &file,
                ],
            ),
            (
                "task copy",
                vec![
                    "task",
                    "copy",
                    &format!("plan:F-{category}"),
                    "--to",
                    "linear",
                ],
            ),
            (
                "task status set",
                vec!["task", "status", "set", "linear:L-1", category],
            ),
            (
                "task update",
                vec!["task", "update", "linear:L-1", "--status", category],
            ),
        ] {
            let from = workspace.served().len();
            let said = failed(&sandbox, &arguments);
            check(&said, "task", category, names, verb);
            assert_eq!(
                mutations_since(&workspace, from),
                Vec::<String>::new(),
                "{verb}"
            );
        }
    }
    for (category, names) in project_cases {
        // A project created, and one updated: `plan:existing` is the counterpart of `LP-1`.
        std::fs::write(
            plan.join("projects/existing.md"),
            format!("---\ntitle: Existing\nstatus: {category}\n---\nbody\n"),
        )
        .expect("the plan's project");
        // Created: the write is refused before any mutation, and there is nothing to undo.
        let from = workspace.served().len();
        let said = failed(
            &sandbox,
            &[
                "project",
                "copy",
                &format!("plan:PR-{category}"),
                "--to",
                "linear",
                "--no-tasks",
            ],
        );
        check(&said, "project", category, names, "project create");
        assert_eq!(
            mutations_since(&workspace, from),
            Vec::<String>::new(),
            "{category}"
        );
        // Updated: refused before the copy records an overwrite or sends anything, so there is
        // nothing for its undo to put back and no mutation at all.
        let from = workspace.served().len();
        let said = failed(
            &sandbox,
            &[
                "project",
                "copy",
                "plan:existing",
                "--to",
                "linear",
                "--no-tasks",
            ],
        );
        check(&said, "project", category, names, "project update");
        assert_eq!(
            mutations_since(&workspace, from),
            Vec::<String>::new(),
            "{category}: no write and no restore"
        );
    }
    assert_eq!(workspace.state_of("L-1").as_deref(), Some("Todo"));
    assert_eq!(workspace.status_of("LP-1").as_deref(), Some("Planned"));

    // A source with no mapping at all has no name for anything: Linear has no built-in names.
    let from = workspace.served().len();
    let said = failed(&sandbox, &["task", "status", "set", "unmapped:L-1", "todo"]);
    assert!(
        said.contains("source unmapped")
            && said.contains("task status name for todo")
            && said.contains("status_mapping.todo.task"),
        "{said}"
    );
    let said = failed(
        &sandbox,
        &[
            "project",
            "copy",
            "plan:PR-done",
            "--to",
            "unmapped",
            "--no-tasks",
        ],
    );
    assert!(
        said.contains("source unmapped")
            && said.contains("project status name for done")
            && said.contains("status_mapping.done.project"),
        "{said}"
    );
    assert_eq!(mutations_since(&workspace, from), Vec::<String>::new());
}

#[test]
fn a_copy_refused_after_it_overwrote_a_project_puts_that_project_back() {
    // The project lands first and really changes `LP-1`; its task's status has no name, so
    // the copy stops there, and its undo writes `LP-1` back as it held it.
    let sandbox = Sandbox::new();
    let (config, workspace) = linear_workspace_with(
        &sandbox,
        held(
            Vec::new(),
            vec![project(
                "LP-1",
                "Planned",
                PROJECT_STATUSES,
                json!({"metadata": {"onetaskgraph.origin": "plan:existing"}}),
            )],
        ),
        TEAM_STATES,
        PROJECT_STATUSES,
    );
    let plan = folder(
        &sandbox,
        "plan",
        &[
            (
                "projects/existing.md".to_owned(),
                "---\ntitle: Renamed\nstatus: done\n---\nbody\n".to_owned(),
            ),
            (
                "tasks/stuck.md".to_owned(),
                "---\ntitle: Stuck\nstatus: draft\nproject: existing\n---\nbody\n".to_owned(),
            ),
        ],
    );
    sandbox.project_document(&document(&json!({
        "linear": linear(&config, json!({"status_mapping": {
            "todo": {"task": "Todo", "project": "Planned"},
            "done": {"task": "Done", "project": "Completed"},
        }})),
        "plan": markdown(&plan),
    })));
    let from = workspace.served().len();
    let said = failed(
        &sandbox,
        &["project", "copy", "plan:existing", "--to", "linear"],
    );
    assert!(
        said.contains("source linear")
            && said.contains("task status")
            && said.contains("status_mapping.draft.task"),
        "{said}"
    );
    let statuses = workspace.served()[from..]
        .iter()
        .filter(|(query, _)| query.trim_start().starts_with("mutation"))
        .map(|(_, variables)| variables["input"]["statusId"].clone())
        .collect::<Vec<_>>();
    assert_eq!(
        statuses,
        [json!("Completed"), json!("Planned")],
        "the project's write, then its restore — and no write of the refused task"
    );
    assert_eq!(workspace.status_of("LP-1").as_deref(), Some("Planned"));
}

/// Unmapped names of every `WorkflowState.type` and every `ProjectStatusType`.
const UNMAPPED_STATES: &[(&str, &str, &str)] = &[
    ("S-icebox", "Icebox", "backlog"),
    ("S-ready", "Ready", "unstarted"),
    ("S-merged", "Merged", "completed"),
    ("S-wont-fix", "Won't Fix", "canceled"),
    ("S-duplicate", "Duplicate", "duplicate"),
];
const UNMAPPED_STATUSES: &[(&str, &str)] = &[("On Hold", "paused"), ("Abandoned", "canceled")];

#[test]
fn an_unmapped_name_reads_as_unknown_and_status_returns_exactly_what_reads_as_each_category() {
    let sandbox = Sandbox::new();
    let mapping = hellopatient_mapping();
    let states: Vec<(&str, &str, &str)> =
        TEAM_STATES.iter().chain(UNMAPPED_STATES).copied().collect();
    let statuses: Vec<(&str, &str)> = PROJECT_STATUSES
        .iter()
        .chain(UNMAPPED_STATUSES)
        .copied()
        .collect();
    // One issue at every state and one project at every status.
    let tasks = states
        .iter()
        .enumerate()
        .map(|(at, (_, name, _))| issue(&format!("I-{at:02}"), name, &states, json!({})))
        .collect();
    let projects = statuses
        .iter()
        .enumerate()
        .map(|(at, (name, _))| project(&format!("P-{at:02}"), name, &statuses, json!({})))
        .collect();
    let (config, _) = linear_workspace_with(&sandbox, held(tasks, projects), &states, &statuses);
    sandbox.project_document(&document(&json!({
        "linear": linear(&config, json!({"status_mapping": mapping})),
    })));

    // What each item reads as, by its kind's half of the mapping, and nothing else: a name
    // that half does not name is `unknown` under its own name, whatever its type.
    let mut expected: std::collections::BTreeMap<(&str, &str), Vec<String>> =
        std::collections::BTreeMap::new();
    for (at, (_, name, kind)) in states.iter().enumerate() {
        let category = CATEGORIES
            .iter()
            .find(|category| named(&mapping, category, "task").as_deref() == Some(*name))
            .copied()
            .unwrap_or("unknown");
        let id = format!("I-{at:02}");
        assert_eq!(
            status(&sandbox, "task", &format!("linear:{id}")),
            json!([category, name]),
            "an issue at {name}, of type {kind}"
        );
        expected.entry(("task", category)).or_default().push(id);
    }
    for (at, (name, kind)) in statuses.iter().enumerate() {
        let category = CATEGORIES
            .iter()
            .find(|category| named(&mapping, category, "project").as_deref() == Some(*name))
            .copied()
            .unwrap_or("unknown");
        let id = format!("P-{at:02}");
        assert_eq!(
            status(&sandbox, "project", &format!("linear:{id}")),
            json!([category, name]),
            "a project at {name}, of type {kind}"
        );
        expected.entry(("project", category)).or_default().push(id);
    }
    for (_, name, _) in UNMAPPED_STATES.iter().chain(&TEAM_STATES[8..]) {
        assert!(
            expected[&("task", "unknown")].iter().any(|id| status(
                &sandbox,
                "task",
                &format!("linear:{id}")
            )[1] == json!(name)),
            "{name} reads as unknown"
        );
    }

    // `--status` returns exactly what reads as each category, for each kind, `unknown` among
    // them — narrowed by Linear, over the names each kind's mapping gives.
    for verb in ["task", "project"] {
        for category in CATEGORIES {
            let mut wanted = expected.get(&(verb, category)).cloned().unwrap_or_default();
            wanted.sort();
            assert_eq!(
                listed(&sandbox, verb, category),
                wanted,
                "{verb} --status {category}"
            );
        }
    }
}

#[test]
fn sources_fields_reports_both_kinds_and_apply_creates_every_missing_name_once() {
    let sandbox = Sandbox::new();
    // Every category names a task and a project missing from the workspace; `draft` names one
    // bare name for both.
    let mut mapping = json!({"draft": "Fresh Draft"});
    for (category, word) in [
        ("backlog", "Backlog"),
        ("todo", "Todo"),
        ("queued", "Queued"),
        ("in-progress", "Progress"),
        ("unknown", "Unknown"),
        ("done", "Done"),
        ("cancelled", "Cancelled"),
    ] {
        mapping[category] = json!({"task": format!("T {word}"), "project": format!("P {word}")});
    }
    let (config, workspace) = linear_workspace_with(
        &sandbox,
        held(Vec::new(), Vec::new()),
        TEAM_STATES,
        PROJECT_STATUSES,
    );
    sandbox.project_document(&document(&json!({
        "linear": linear(&config, json!({"status_mapping": mapping})),
        // Present names: one of the type its category is created as, and one of another type,
        // which is reported and left exactly as it is.
        "present": linear(&config, json!({"status_mapping": {
            "todo": {"task": "Todo", "project": "Planned"},
            "queued": {"task": "Done", "project": "Completed"},
        }})),
    })));
    let types = |category: &str| match category {
        "backlog" | "draft" => ("backlog", "backlog"),
        "todo" | "queued" => ("unstarted", "planned"),
        "in-progress" | "unknown" => ("started", "started"),
        "done" => ("completed", "completed"),
        _ => ("canceled", "canceled"),
    };
    let row = |kind: &str, category: &str, present: bool, created: bool| {
        let name = named(&mapping, category, kind).expect("named");
        let expected = if kind == "task" {
            types(category).0
        } else {
            types(category).1
        };
        let mut row = json!({"kind": kind, "category": category, "name": name,
                             "present": present, "expected_type": expected});
        if present {
            row["type"] = json!(expected);
        }
        if created {
            row["created"] = json!(true);
        }
        row
    };
    let order = [
        "draft",
        "backlog",
        "todo",
        "queued",
        "in-progress",
        "done",
        "cancelled",
        "unknown",
    ];
    let rows = |present: bool, created: bool| {
        ["task", "project"]
            .iter()
            .flat_map(|kind| {
                order
                    .iter()
                    .map(move |category| row(kind, category, present, created))
            })
            .collect::<Vec<_>>()
    };

    // A plan: every name missing, and nothing written.
    let from = workspace.served().len();
    let report = answered(&sandbox, &["--json", "sources", "fields", "linear"]);
    assert_eq!(
        report,
        json!({"source": "linear", "team": "FIX", "names": rows(false, false)})
    );
    let text = stdout(&exits(&sandbox, &["sources", "fields", "linear"], 0));
    assert!(
        text.contains("linear: task draft -> Fresh Draft: missing: no workflow state of team FIX has that name")
            && text.contains("linear: project draft -> Fresh Draft: missing: no project status of this workspace has that name"),
        "{text}"
    );
    assert_eq!(mutations_since(&workspace, from), Vec::<String>::new());

    // Applied: each missing name created once, of its category's type, in its own vocabulary;
    // the bare name in both.
    let states_before = workspace.states();
    let statuses_before = workspace.project_statuses();
    let applied = answered(
        &sandbox,
        &["--json", "sources", "fields", "linear", "--apply"],
    );
    assert_eq!(
        applied,
        json!({"source": "linear", "team": "FIX", "names": rows(true, true)})
    );
    let created_states = workspace.states()[states_before.len()..].to_vec();
    let created_statuses = workspace.project_statuses()[statuses_before.len()..].to_vec();
    assert_eq!(
        created_states,
        order
            .iter()
            .map(|category| (
                named(&mapping, category, "task").unwrap(),
                types(category).0.to_owned()
            ))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        created_statuses,
        order
            .iter()
            .map(|category| (
                named(&mapping, category, "project").unwrap(),
                types(category).1.to_owned()
            ))
            .collect::<Vec<_>>()
    );
    assert_eq!(workspace.states()[..states_before.len()], states_before[..]);
    // Each in Linear's neutral grey, and each project status after the workspace's last, in
    // the order it was created.
    let creates = workspace
        .served()
        .into_iter()
        .filter(|(query, _)| {
            query == onetaskgraph_linear::graphql::WORKFLOW_STATE_CREATE
                || query == onetaskgraph_linear::graphql::PROJECT_STATUS_CREATE
        })
        .map(|(_, variables)| variables["input"].clone())
        .collect::<Vec<_>>();
    assert_eq!(creates.len(), 16, "{creates:#?}");
    assert!(
        creates.iter().all(|input| input["color"] == "#95a2b3"),
        "{creates:#?}"
    );
    let positions = creates
        .iter()
        .filter_map(|input| input.get("position").and_then(Value::as_f64))
        .collect::<Vec<_>>();
    let last = PROJECT_STATUSES.len() as f64;
    assert_eq!(
        positions,
        (1..=8).map(|at| last + f64::from(at)).collect::<Vec<_>>(),
        "after the workspace's last, at {last}"
    );
    assert_eq!(
        workspace.project_statuses()[..statuses_before.len()],
        statuses_before[..]
    );
    let text = stdout(&exits(&sandbox, &["sources", "fields", "linear"], 0));
    assert!(
        text.contains(
            "linear: task done -> T Done: present as a workflow state of team FIX (completed)"
        ),
        "{text}"
    );

    // A second apply finds every name and creates nothing.
    let from = workspace.served().len();
    let again = answered(
        &sandbox,
        &["--json", "sources", "fields", "linear", "--apply"],
    );
    assert_eq!(
        again,
        json!({"source": "linear", "team": "FIX", "names": rows(true, false)})
    );
    assert_eq!(mutations_since(&workspace, from), Vec::<String>::new());

    // A present name of another type is reported with its type and left as it is.
    let from = workspace.served().len();
    let vocabulary = (workspace.states(), workspace.project_statuses());
    let present = answered(
        &sandbox,
        &["--json", "sources", "fields", "present", "--apply"],
    );
    assert_eq!(
        present["names"],
        json!([
            {"kind": "task", "category": "todo", "name": "Todo", "present": true,
             "type": "unstarted", "expected_type": "unstarted"},
            {"kind": "task", "category": "queued", "name": "Done", "present": true,
             "type": "completed", "expected_type": "unstarted"},
            {"kind": "project", "category": "todo", "name": "Planned", "present": true,
             "type": "planned", "expected_type": "planned"},
            {"kind": "project", "category": "queued", "name": "Completed", "present": true,
             "type": "completed", "expected_type": "planned"},
        ])
    );
    assert_eq!(mutations_since(&workspace, from), Vec::<String>::new());
    assert_eq!(
        (workspace.states(), workspace.project_statuses()),
        vocabulary
    );
    let text = stdout(&exits(&sandbox, &["sources", "fields", "present"], 0));
    assert!(
        text.contains("present: task queued -> Done: present as a workflow state of team FIX (completed; queued is created as unstarted, and this one is left as it is)"),
        "{text}"
    );
}

#[test]
fn an_apply_linear_refuses_part_way_names_what_it_created_and_a_rerun_creates_the_rest() {
    let sandbox = Sandbox::new();
    let mapping = json!({
        "todo": {"task": "First"},
        "queued": {"task": "Second"},
        "done": {"task": "Third", "project": "Finished"},
    });
    let (config, workspace) = linear_workspace_with(
        &sandbox,
        held(Vec::new(), Vec::new()),
        TEAM_STATES,
        PROJECT_STATUSES,
    );
    sandbox.project_document(&document(&json!({
        "linear": linear(&config, json!({"status_mapping": mapping})),
    })));
    workspace.refuse_create("Second");

    let output = run(
        &sandbox,
        &["--json", "sources", "fields", "linear", "--apply"],
    );
    assert!(!output.status.success(), "{}", stdout(&output));
    let said = stderr(&output);
    assert!(
        said.contains("the create of the task status \"Second\" failed")
            && said.contains("You do not have permission")
            && said.contains("--apply"),
        "{said}"
    );
    // The report first, then the failure document `--json` prints for a refusal.
    let printed = stdout(&output);
    let report: Value = serde_json::Deserializer::from_str(&printed)
        .into_iter::<Value>()
        .next()
        .expect("the report is printed")
        .expect("the report is JSON");
    let created = |report: &Value| {
        report["names"]
            .as_array()
            .expect("names")
            .iter()
            .filter(|row| row["created"] == true)
            .map(|row| {
                format!(
                    "{} {}",
                    row["kind"].as_str().unwrap(),
                    row["name"].as_str().unwrap()
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(created(&report), ["task First"], "{report:#}");
    assert_eq!(
        report["refused"]["kind"], "task",
        "the report names the refused create: {report:#}"
    );
    assert_eq!(report["refused"]["name"], "Second");
    let names =
        |held: Vec<(String, String)>| held.into_iter().map(|(name, _)| name).collect::<Vec<_>>();
    assert!(names(workspace.states()).contains(&"First".to_owned()));
    assert!(!names(workspace.states()).contains(&"Second".to_owned()));
    assert!(!names(workspace.states()).contains(&"Third".to_owned()));

    workspace.allow_create();
    let again = answered(
        &sandbox,
        &["--json", "sources", "fields", "linear", "--apply"],
    );
    assert_eq!(
        created(&again),
        ["task Second", "task Third", "project Finished"],
        "only what was still missing: {again:#}"
    );
    assert!(again.get("refused").is_none());
}

#[test]
fn a_project_scoped_source_reports_and_creates_project_names_and_writes_its_own_project_alone() {
    let sandbox = Sandbox::new();
    let (config, workspace) = linear_workspace_with(
        &sandbox,
        held(
            vec![issue(
                "I-IN",
                "Todo",
                TEAM_STATES,
                json!({"project": "LP-1"}),
            )],
            vec![
                project(
                    "LP-1",
                    "Planned",
                    PROJECT_STATUSES,
                    json!({"metadata": {"onetaskgraph.origin": "plan:own"}}),
                ),
                project(
                    "LP-2",
                    "Planned",
                    PROJECT_STATUSES,
                    json!({"metadata": {"onetaskgraph.origin": "plan:other"}}),
                ),
            ],
        ),
        TEAM_STATES,
        PROJECT_STATUSES,
    );
    let plan = folder(
        &sandbox,
        "plan",
        &[
            (
                "projects/own.md".to_owned(),
                "---\ntitle: Own\nstatus: done\n---\nbody\n".to_owned(),
            ),
            (
                "projects/other.md".to_owned(),
                "---\ntitle: Other\nstatus: done\n---\nbody\n".to_owned(),
            ),
            (
                "projects/new.md".to_owned(),
                "---\ntitle: New\nstatus: done\n---\nbody\n".to_owned(),
            ),
        ],
    );
    sandbox.project_document(&document(&json!({
        "scoped": linear(&config, json!({"project": "LP-1",
            "status_mapping": {"todo": "Todo", "done": "Wrapped Up"}})),
        "plan": markdown(&plan),
    })));

    // Its project names are checked against the workspace's project statuses, a bare name in
    // both vocabularies, exactly as for a source with no `project`.
    let report = answered(&sandbox, &["--json", "sources", "fields", "scoped"]);
    assert_eq!(
        report["names"],
        json!([
            {"kind": "task", "category": "todo", "name": "Todo", "present": true,
             "type": "unstarted", "expected_type": "unstarted"},
            {"kind": "task", "category": "done", "name": "Wrapped Up", "present": false,
             "expected_type": "completed"},
            {"kind": "project", "category": "todo", "name": "Todo", "present": false,
             "expected_type": "planned"},
            {"kind": "project", "category": "done", "name": "Wrapped Up", "present": false,
             "expected_type": "completed"},
        ])
    );
    let from = workspace.served().len();
    answered(
        &sandbox,
        &["--json", "sources", "fields", "scoped", "--apply"],
    );
    assert_eq!(
        mutations_since(&workspace, from),
        [
            "workflowStateCreate",
            "projectStatusCreate",
            "projectStatusCreate"
        ]
    );
    assert!(
        workspace
            .states()
            .contains(&("Wrapped Up".into(), "completed".into()))
    );
    assert!(
        workspace
            .project_statuses()
            .contains(&("Todo".into(), "planned".into()))
    );
    assert!(
        workspace
            .project_statuses()
            .contains(&("Wrapped Up".into(), "completed".into()))
    );

    // Its own project's status is written as the name its project mapping gives.
    answered(
        &sandbox,
        &[
            "--json",
            "project",
            "copy",
            "plan:own",
            "--to",
            "scoped",
            "--no-tasks",
        ],
    );
    assert_eq!(workspace.status_of("LP-1").as_deref(), Some("Wrapped Up"));
    assert_eq!(
        status(&sandbox, "project", "scoped:LP-1"),
        json!(["done", "Wrapped Up"])
    );

    // A new project and another project are refused, naming the scope, before any mutation.
    for id in ["plan:new", "plan:other"] {
        let from = workspace.served().len();
        let said = failed(
            &sandbox,
            &["project", "copy", id, "--to", "scoped", "--no-tasks"],
        );
        assert!(
            said.contains("scoped to the Linear project LP-1"),
            "{id}: {said}"
        );
        assert_eq!(
            mutations_since(&workspace, from),
            Vec::<String>::new(),
            "{id}"
        );
    }
    assert_eq!(workspace.status_of("LP-2").as_deref(), Some("Planned"));
}

#[test]
fn a_malformed_status_mapping_is_refused_when_the_configuration_loads_naming_the_part() {
    for plugin in ["linear", "github-projects"] {
        for (mapping, said) in [
            (
                json!({"done": {}}),
                "status_mapping.done is an empty object, which maps no kind; to disable done \
                 for every kind, write null",
            ),
            (
                json!({"done": {"task": "Done", "epic": "Done"}}),
                "status_mapping.done names \"epic\", which is not an item kind",
            ),
            (
                json!({"done": {"task": null}}),
                "status_mapping.done.task is null",
            ),
            (
                json!({"done": {"project": " "}}),
                "status_mapping.done.project is blank",
            ),
            (
                json!({"doing": "Doing"}),
                "status_mapping names \"doing\", which is not a status category",
            ),
        ] {
            let sandbox = Sandbox::new();
            sandbox.project_document(&document(&json!({
                "work": {"plugin": plugin, "config": {"status_mapping": mapping}},
            })));
            let refused = failed(&sandbox, &["task", "list", "--source", "work"]);
            assert!(
                refused.contains("sources.work.config.status_mapping") && refused.contains(said),
                "{plugin} {mapping}: {refused}"
            );
        }
    }
}

#[test]
fn an_apply_linear_refuses_a_project_status_for_names_it_and_a_rerun_creates_it() {
    // A workspace whose key may create workflow states and not this project status.
    let sandbox = Sandbox::new();
    let (config, workspace) = linear_workspace_with(
        &sandbox,
        held(Vec::new(), Vec::new()),
        TEAM_STATES,
        PROJECT_STATUSES,
    );
    sandbox.project_document(&document(&json!({
        "linear": linear(&config, json!({"status_mapping": {
            "done": {"task": "Wrapped Up", "project": "Closing"},
        }})),
    })));
    workspace.refuse_create("Closing");
    let refused = run(
        &sandbox,
        &["--json", "sources", "fields", "linear", "--apply"],
    );
    assert!(!refused.status.success(), "{}", stdout(&refused));
    let said = stderr(&refused);
    assert!(
        said.contains("the create of the project status \"Closing\" failed"),
        "{said}"
    );
    let printed = stdout(&refused);
    let report: Value = serde_json::Deserializer::from_str(&printed)
        .into_iter::<Value>()
        .next()
        .expect("the report is printed")
        .expect("the report is JSON");
    assert_eq!(report["refused"]["kind"], "project", "{report:#}");
    assert_eq!(
        report["names"][0]["created"], true,
        "the state was created first"
    );
    assert!(report["names"][1].get("created").is_none(), "{report:#}");
    assert!(
        !workspace
            .project_statuses()
            .iter()
            .any(|(name, _)| name == "Closing")
    );

    workspace.allow_create();
    let from = workspace.served().len();
    let again = answered(
        &sandbox,
        &["--json", "sources", "fields", "linear", "--apply"],
    );
    assert_eq!(mutations_since(&workspace, from), ["projectStatusCreate"]);
    assert_eq!(again["names"][1]["created"], true, "{again:#}");
    assert!(
        workspace
            .project_statuses()
            .contains(&("Closing".to_owned(), "completed".to_owned()))
    );
}

#[test]
fn an_apply_whose_create_linear_answers_other_than_asked_fails_beside_what_it_created() {
    // Each create after the first answered under a type its category does not derive, under
    // another name, or with no state or status at all: none is reported created, nor held as
    // the mapping's.
    for (kind, name, how, said) in [
        (
            "task",
            "Second",
            "other-type",
            "of type unstarted with \"Second\" of type triage",
        ),
        (
            "task",
            "Second",
            "other-name",
            "of type unstarted with \"Second (renamed)\" of type unstarted",
        ),
        (
            "project",
            "Closing",
            "other-name",
            "of type completed with \"Closing (renamed)\" of type completed",
        ),
        (
            "task",
            "Second",
            "no-payload",
            "missing workflowStateCreate.workflowState",
        ),
        (
            "project",
            "Closing",
            "other-type",
            "of type completed with \"Closing\" of type paused",
        ),
        (
            "project",
            "Closing",
            "no-payload",
            "missing projectStatusCreate.status",
        ),
    ] {
        let sandbox = Sandbox::new();
        let (config, workspace) = linear_workspace_with(
            &sandbox,
            held(Vec::new(), Vec::new()),
            TEAM_STATES,
            PROJECT_STATUSES,
        );
        sandbox.project_document(&document(&json!({
            "linear": linear(&config, json!({"status_mapping": {
                "todo": {"task": "First"},
                "queued": {"task": "Second"},
                "done": {"project": "Closing"},
            }})),
        })));
        workspace.misanswer_create(name, how);
        let failed = run(
            &sandbox,
            &["--json", "sources", "fields", "linear", "--apply"],
        );
        assert!(!failed.status.success(), "{how}: {}", stdout(&failed));
        let stderr = stderr(&failed);
        assert!(
            stderr.contains(&format!(
                "the create of the {kind} status \"{name}\" failed"
            )) && stderr.contains(said),
            "{how}: {stderr}"
        );
        let printed = stdout(&failed);
        let report: Value = serde_json::Deserializer::from_str(&printed)
            .into_iter::<Value>()
            .next()
            .expect("the report is printed")
            .expect("the report is JSON");
        assert_eq!(report["refused"]["kind"], kind, "{how}: {report:#}");
        assert_eq!(report["refused"]["name"], name, "{how}: {report:#}");
        let created = report["names"]
            .as_array()
            .expect("names")
            .iter()
            .filter(|row| row["created"] == true)
            .map(|row| row["name"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        let before = if kind == "task" {
            vec!["First"]
        } else {
            vec!["First", "Second"]
        };
        assert_eq!(
            created, before,
            "{how}: only what landed as asked: {report:#}"
        );
        assert!(
            workspace
                .states()
                .contains(&("First".to_owned(), "unstarted".to_owned())),
            "{how}: the create before it landed"
        );
        if how == "other-name" {
            // What Linear named otherwise is not the mapping's name: a fresh read still finds
            // the asked name absent from that kind's vocabulary.
            let reread = answered(&sandbox, &["--json", "sources", "fields", "linear"]);
            let row = reread["names"]
                .as_array()
                .expect("names")
                .iter()
                .find(|row| row["kind"] == kind && row["name"] == name)
                .unwrap_or_else(|| panic!("{how}: a row for {name}: {reread:#}"))
                .clone();
            assert_eq!(row["present"], false, "{how}: {reread:#}");
        }
    }
}

#[test]
fn sources_fields_says_in_words_what_it_found_and_what_it_created() {
    let sandbox = Sandbox::new();
    let (config, workspace) = linear_workspace_with(
        &sandbox,
        held(Vec::new(), Vec::new()),
        TEAM_STATES,
        PROJECT_STATUSES,
    );
    sandbox.project_document(&document(&json!({
        "linear": linear(&config, json!({"status_mapping": {"done": "Wrapped Up"}})),
        "unmapped": linear(&config, json!({"status_mapping": {}})),
    })));
    assert_eq!(
        stdout(&exits(&sandbox, &["sources", "fields", "unmapped"], 0)).trim_end(),
        "unmapped: status_mapping names no status"
    );
    let applied = stdout(&exits(
        &sandbox,
        &["sources", "fields", "linear", "--apply"],
        0,
    ));
    assert_eq!(
        applied.trim_end().lines().collect::<Vec<_>>(),
        [
            "linear: task done -> Wrapped Up: created as a workflow state of team FIX (completed)",
            "linear: project done -> Wrapped Up: created as a project status of this workspace \
             (completed)",
        ]
    );
    assert!(
        workspace
            .project_statuses()
            .contains(&("Wrapped Up".to_owned(), "completed".to_owned()))
    );
}

#[test]
fn unknown_is_every_item_of_a_kind_its_mapping_names_nothing_for() {
    let sandbox = Sandbox::new();
    let (config, _) = linear_workspace_with(
        &sandbox,
        held(
            vec![
                issue("I-TODO", "Todo", TEAM_STATES, json!({})),
                issue("I-TRIAGE", "Triage", TEAM_STATES, json!({})),
            ],
            vec![
                project("P-PLANNED", "Planned", PROJECT_STATUSES, json!({})),
                project("P-IDEA", "Idea", PROJECT_STATUSES, json!({})),
            ],
        ),
        TEAM_STATES,
        PROJECT_STATUSES,
    );
    sandbox.project_document(&document(&json!({
        // No mapping at all, and one naming a task's statuses alone.
        "unmapped": linear(&config, json!({"status_mapping": {}})),
        "tasks-only": linear(&config, json!({"status_mapping": {"todo": {"task": "Todo"}}})),
    })));
    let listed = |source: &str, verb: &str, category: &str| {
        let mut ids = answered(
            &sandbox,
            &[
                "--json", verb, "list", "--source", source, "--status", category,
            ],
        )["items"]
            .as_array()
            .expect("a page")
            .iter()
            .map(|item| item["id"].as_str().expect("an id").to_owned())
            .collect::<Vec<_>>();
        ids.sort();
        ids
    };
    assert_eq!(
        listed("unmapped", "task", "unknown"),
        ["unmapped:I-TODO", "unmapped:I-TRIAGE"]
    );
    assert_eq!(
        listed("unmapped", "project", "unknown"),
        ["unmapped:P-IDEA", "unmapped:P-PLANNED"]
    );
    assert_eq!(listed("unmapped", "task", "todo"), Vec::<String>::new());
    assert_eq!(listed("tasks-only", "task", "todo"), ["tasks-only:I-TODO"]);
    assert_eq!(
        listed("tasks-only", "task", "unknown"),
        ["tasks-only:I-TRIAGE"]
    );
    assert_eq!(
        listed("tasks-only", "project", "unknown"),
        ["tasks-only:P-IDEA", "tasks-only:P-PLANNED"]
    );
    assert_eq!(
        listed("tasks-only", "project", "todo"),
        Vec::<String>::new()
    );
}
