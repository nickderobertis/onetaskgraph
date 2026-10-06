//! What a Linear status write costs, counted at the loopback Linear's endpoint.
//!
//! Each `measure_*` journey here is the command of one budget in
//! `crates/onetaskgraph-linear/budgets.yaml`: it measures, asserts the figure is within its
//! threshold, and — when `onebudgetspec check` runs it, which sets `ONEBUDGETSPEC_RESULT` —
//! writes the figure there. The cold figures go through the real binary, one invocation being
//! one fresh source instance; the warm ones through `onetaskgraph-core`'s own `Engine`, which
//! is what a long-lived caller — onepipeline's write-back worker — links, and where one source
//! instance makes write after write. Every workspace is Hello Patient's vocabulary, configured
//! with the per-kind mapping ai-orchestrator writes for it.
//!
//! The figures are an inner measure of headroom on a production Linear key shared by every
//! manager of a host, which no check may reach; the base's own counts, measured at commit
//! c9e75a8 on a configuration that commit completes, are recorded beside each budget.

use std::path::{Path, PathBuf};

use onetaskgraph_core::{
    ConfiguredSource, CopyItems, CopyRequest, CopyScope, Engine, GlobalId, ResolvedSource,
};
use onetaskgraph_plugin_api::{
    MetadataKey, SourceName, SourcePlugin as _, Status, StatusCategory, TaskUpdate,
};
use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{LinearWorkspace, document, linear_workspace_with};
use crate::linear_status::{PROJECT_STATUSES, TEAM_STATES, hellopatient_mapping};

/// The documents a whole project write sends of its own: the resolution when it is the first
/// write, the label lookups, the create or the rewrite, and the relation reads and writes. A
/// copy's discovery reads are none of them.
const PROJECT_WRITE: [&str; 7] = [
    onetaskgraph_linear::graphql::RESOLUTION,
    onetaskgraph_linear::graphql::PROJECT_LABEL,
    onetaskgraph_linear::graphql::PROJECT_CREATE,
    onetaskgraph_linear::graphql::PROJECT_REWRITE,
    onetaskgraph_linear::graphql::PROJECT_RELATIONS,
    onetaskgraph_linear::graphql::PROJECT_RELATION_CREATE,
    onetaskgraph_linear::graphql::PROJECT_RELATION_DELETE,
];

/// The documents a whole task write sends of its own, on the same terms.
const TASK_WRITE: [&str; 7] = [
    onetaskgraph_linear::graphql::RESOLUTION,
    onetaskgraph_linear::graphql::ISSUE_LABEL,
    onetaskgraph_linear::graphql::ISSUE_CREATE,
    onetaskgraph_linear::graphql::ISSUE_REWRITE,
    onetaskgraph_linear::graphql::ISSUE_RELATIONS,
    onetaskgraph_linear::graphql::ISSUE_RELATION_CREATE,
    onetaskgraph_linear::graphql::ISSUE_RELATION_DELETE,
];

/// The threshold `crates/onetaskgraph-linear/budgets.yaml` registers for `budget`, read out of
/// that file — so a journey and the budget it measures hold one allowance, not two.
fn threshold(budget: &str) -> usize {
    let file = include_str!("../../../onetaskgraph-linear/budgets.yaml");
    let entry = file
        .lines()
        .skip_while(|line| line.trim() != format!("- id: {budget}"))
        .skip(1)
        .take_while(|line| !line.trim_start().starts_with("- id:"));
    entry
        .filter_map(|line| line.trim().strip_prefix("threshold:"))
        .map(|value| value.trim().parse().expect("a whole number of requests"))
        .next()
        .unwrap_or_else(|| panic!("budgets.yaml registers no threshold for {budget}"))
}

/// Hand `value` to `onebudgetspec check` when it is the one running this, and hold it to the
/// budget's own threshold either way.
fn report(budget: &str, value: usize, detail: &str) {
    let threshold = threshold(budget);
    if let Some(path) = std::env::var_os("ONEBUDGETSPEC_RESULT") {
        std::fs::write(path, json!({"value": value, "detail": detail}).to_string())
            .expect("the budget result is writable");
    }
    assert!(
        value <= threshold,
        "{budget}: {value} requests, over the budget of {threshold} — {detail}"
    );
}

/// The fixture's credential, and nothing of the host's: the variable the Linear source names,
/// in an environment of its own.
fn secrets() -> onetaskgraph_core::Secrets {
    onetaskgraph_core::Secrets::load(onetaskgraph_core::Environment::from_pairs([(
        "LINEAR_API_KEY",
        "fixture-key",
    )]))
    .expect("an environment with no credentials file")
}

fn issue(id: &str, state: &str, extra: Value) -> Value {
    let kind = TEAM_STATES
        .iter()
        .find(|held| held.1 == state)
        .expect("a state of the team")
        .2;
    let mut row = json!({
        "id": id, "title": format!("Issue {id}"), "content": format!("The body of {id}."),
        "status": {"name": state, "category": "unknown"},
        "_linear_state": {"name": state, "type": kind},
        "labels": [], "priority": "none",
    });
    for (key, value) in extra.as_object().expect("an object") {
        row[key] = value.clone();
    }
    row
}

fn held(tasks: Vec<Value>) -> Value {
    json!({"tasks": tasks, "projects": [], "documents": [], "labels": [],
           "task_dependencies": [], "project_dependencies": []})
}

/// Hello Patient's workspace over `tasks`, and the source configuration that reaches it.
fn hellopatient(sandbox: &Sandbox, tasks: Vec<Value>) -> (Value, LinearWorkspace) {
    let (mut config, workspace) =
        linear_workspace_with(sandbox, held(tasks), TEAM_STATES, PROJECT_STATUSES);
    config["status_mapping"] = hellopatient_mapping();
    (config, workspace)
}

/// A folder of Markdown holding `files`.
fn folder(sandbox: &Sandbox, files: &[(&str, &str)]) -> PathBuf {
    let root = sandbox.subdirectory("plan");
    for kind in ["tasks", "projects", "documents"] {
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

/// Run the binary, which has to succeed; how many requests the workspace answered while it
/// ran, and every one of them.
fn counted(
    sandbox: &Sandbox,
    workspace: &LinearWorkspace,
    arguments: &[&str],
) -> Vec<(String, Value)> {
    let from = workspace.served().len();
    let output = sandbox
        .command()
        .args(arguments)
        .assert()
        .get_output()
        .clone();
    assert!(
        output.status.success(),
        "`onetaskgraph {}`\nstdout:\n{}\nstderr:\n{}",
        arguments.join(" "),
        stdout(&output),
        stderr(&output)
    );
    workspace.served()[from..].to_vec()
}

/// Of `served`, the requests `documents` names.
fn of(served: &[(String, Value)], documents: &[&str]) -> usize {
    served
        .iter()
        .filter(|(query, _)| documents.contains(&query.as_str()))
        .count()
}

/// The requests one project copy's `write_project` sent of its own: every request of
/// [`PROJECT_WRITE`] from the first that only a write sends — the resolution, a label lookup,
/// the create or the rewrite. What the copy read before it — its counterpart, what that holds,
/// and the dependencies it has — is the copy's own discovery, and comes first.
fn project_write(served: &[(String, Value)]) -> usize {
    let starts = &PROJECT_WRITE[..4];
    served
        .iter()
        .position(|(query, _)| starts.contains(&query.as_str()))
        .map_or(0, |first| of(&served[first..], &PROJECT_WRITE))
}

/// The requests each item write of a copy sent of its own, in the order the writes landed:
/// every request of `documents` up to and including each create.
fn per_write(served: &[(String, Value)], documents: &[&str], creates: &[&str]) -> Vec<usize> {
    let mut writes = Vec::new();
    let mut running = 0;
    for (query, _) in served {
        if documents.contains(&query.as_str()) {
            running += 1;
        }
        if creates.contains(&query.as_str()) {
            writes.push(running);
            running = 0;
        }
    }
    writes
}

#[test]
fn measure_linear_requests_per_status_write_cold() {
    let sandbox = Sandbox::new();
    let (config, workspace) = hellopatient(
        &sandbox,
        vec![
            issue("L-SET", "Todo", json!({})),
            issue("L-UPDATE", "Todo", json!({})),
        ],
    );
    let plan = folder(
        &sandbox,
        &[
            (
                "projects/alone.md",
                "---\ntitle: Alone\nstatus: todo\n---\nA project with no tasks.\n",
            ),
            (
                "projects/goal.md",
                "---\ntitle: One goal\nstatus: todo\n---\nAcross both orgs.\n",
            ),
            (
                "tasks/own.md",
                "---\ntitle: Own\nstatus: todo\nproject: goal\n\
                 repositories: [github.com/nickderobertis/lib]\n---\nStays.\n",
            ),
            (
                "tasks/pets.md",
                "---\ntitle: Pets\nstatus: todo\nproject: goal\n\
                 repositories: [github.com/petsinc/api]\n---\nRoutes.\n",
            ),
        ],
    );
    let notes = sandbox.subdirectory("notes");
    sandbox.project_document(&document(&json!({
        "patients": {"plugin": "linear", "config": config},
        "plan": markdown(&plan),
        "notes": {"plugin": "local-md", "config": {"root": notes},
                  "routes": [{"repositories": ["github.com/petsinc/*"], "to": "patients"}]},
    })));

    // A task, from a mapped state of one category to a mapped state of another, each by a
    // fresh invocation: the resolution, and the mutation.
    let set = counted(
        &sandbox,
        &workspace,
        &["task", "status", "set", "patients:L-SET", "in-progress"],
    )
    .len();
    let update = counted(
        &sandbox,
        &workspace,
        &[
            "task",
            "update",
            "patients:L-UPDATE",
            "--status",
            "in-progress",
        ],
    )
    .len();
    assert_eq!(workspace.state_of("L-SET").as_deref(), Some("In Progress"));
    assert_eq!(
        workspace.state_of("L-UPDATE").as_deref(),
        Some("In Progress")
    );

    // A project with no tasks, labels or edges, created by a copy — the requests
    // `write_project` sends of its own, beside the copy's discovery reads.
    let alone = counted(
        &sandbox,
        &workspace,
        &[
            "project",
            "copy",
            "plan:alone",
            "--to",
            "patients",
            "--no-tasks",
        ],
    );
    let project = project_write(&alone);
    // And a member project a routed copy creates, the project the copy writes there first:
    // its own requests up to and including its create.
    let routed = counted(
        &sandbox,
        &workspace,
        &["project", "copy", "plan:goal", "--to", "notes"],
    );
    let member = per_write(
        &routed,
        &PROJECT_WRITE,
        &[onetaskgraph_linear::graphql::PROJECT_CREATE],
    );
    assert_eq!(
        member.len(),
        1,
        "one member project was created: {routed:#?}"
    );
    let highest = set.max(update).max(project).max(member[0]);
    report(
        "linear-requests-per-status-write-cold",
        highest,
        &format!(
            "task status set {set}, task update --status {update}, write_project of a project \
             copy {project}, write_project of a routed member {}",
            member[0]
        ),
    );
}

#[test]
fn measure_linear_requests_per_status_write_warm() {
    // A copy creating a project and its five tasks, each create carrying a status: one
    // resolution, then one mutation per item.
    let sandbox = Sandbox::new();
    let (config, workspace) = hellopatient(&sandbox, Vec::new());
    let mut files = vec![(
        "projects/big.md".to_owned(),
        "---\ntitle: Big\nstatus: todo\n---\nFive tasks.\n".to_owned(),
    )];
    for (at, category) in ["todo", "queued", "in-progress", "done", "backlog"]
        .iter()
        .enumerate()
    {
        files.push((
            format!("tasks/t{at}.md"),
            format!("---\ntitle: Task {at}\nstatus: {category}\nproject: big\n---\nBody.\n"),
        ));
    }
    let files = files
        .iter()
        .map(|(path, text)| (path.as_str(), text.as_str()))
        .collect::<Vec<_>>();
    let plan = folder(&sandbox, &files);
    sandbox.project_document(&document(&json!({
        "patients": {"plugin": "linear", "config": config.clone()},
        "plan": markdown(&plan),
    })));
    let copied = counted(
        &sandbox,
        &workspace,
        &["project", "copy", "plan:big", "--to", "patients"],
    );
    let writes = per_write(
        &copied,
        &[&PROJECT_WRITE[..], &TASK_WRITE[..]].concat(),
        &[
            onetaskgraph_linear::graphql::PROJECT_CREATE,
            onetaskgraph_linear::graphql::ISSUE_CREATE,
        ],
    );
    assert_eq!(writes.len(), 6, "a project and five tasks: {copied:#?}");
    let created = writes[1..].iter().copied().max().unwrap_or_default();

    // In one Engine, as a long-lived caller holds one source: five status-only updates, and
    // five rewrites of a project a copy keeps in step.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    let sandbox = Sandbox::new();
    let tasks = (0..5)
        .map(|at| issue(&format!("L-{at}"), "Todo", json!({})))
        .collect();
    let (config, workspace) = hellopatient(&sandbox, tasks);
    let plan = folder(
        &sandbox,
        &[(
            "projects/kept.md",
            "---\ntitle: Kept in step\nstatus: todo\n---\nRewritten.\n",
        )],
    );
    let engine = engine(&config, Some(&plan));
    let mut updates = Vec::new();
    let mut rewrites = Vec::new();
    runtime.block_on(async {
        for (at, category) in [
            StatusCategory::InProgress,
            StatusCategory::Queued,
            StatusCategory::Done,
            StatusCategory::Backlog,
            StatusCategory::Cancelled,
        ]
        .into_iter()
        .enumerate()
        {
            let from = workspace.served().len();
            engine
                .update_task(
                    &global(&format!("patients:L-{at}")),
                    &TaskUpdate {
                        status: Some(Status {
                            category,
                            name: String::new(),
                        }),
                        ..TaskUpdate::default()
                    },
                )
                .await
                .expect("the status lands");
            updates.push(workspace.served().len() - from);
        }
        // The first copy creates the project; every one after it rewrites it.
        for category in [
            "todo",
            "queued",
            "in-progress",
            "done",
            "cancelled",
            "backlog",
        ] {
            std::fs::write(
                plan.join("projects/kept.md"),
                format!("---\ntitle: Kept in step\nstatus: {category}\n---\nRewritten.\n"),
            )
            .expect("the plan's project");
            let from = workspace.served().len();
            engine
                .copy(&CopyRequest {
                    items: CopyItems::new(vec![global("plan:kept")]).expect("one item"),
                    scope: CopyScope::Projects { tasks: false },
                    destination: SourceName::new("patients").unwrap(),
                    match_by: None,
                    recreate: false,
                    create: false,
                    dry_run: false,
                })
                .await
                .expect("the copy lands");
            rewrites.push(project_write(&workspace.served()[from..]));
        }
    });
    assert_eq!(
        updates[0], 2,
        "the first update reads the resolution: {updates:?}"
    );
    let updated = updates[1..].iter().copied().max().unwrap_or_default();
    let rewritten = rewrites[1..].iter().copied().max().unwrap_or_default();
    let highest = created.max(updated).max(rewritten);
    report(
        "linear-requests-per-status-write-warm",
        highest,
        &format!(
            "a copy's task creates after the first {:?}, status-only updates after the first \
             {:?}, project rewrites after the first create {:?}",
            &writes[1..],
            &updates[1..],
            &rewrites[1..]
        ),
    );
}

/// An Engine over the Linear source `config` configures — and the folder of Markdown at
/// `plan`, when there is one, as the source a copy reads from.
fn engine(config: &Value, plan: Option<&Path>) -> Engine {
    let name = SourceName::new("patients").unwrap();
    let linear = onetaskgraph_linear::Plugin
        .build(&name, config, &secrets())
        .expect("the Linear source builds");
    let mut sources = vec![ConfiguredSource::Ready(ResolvedSource::adopt(
        name.clone(),
        linear,
    ))];
    let mut selection = vec![name];
    if let Some(plan) = plan {
        let name = SourceName::new("plan").unwrap();
        let local = onetaskgraph_core::plugin_for("local-md")
            .expect("local-md is registered")
            .build(&name, &json!({"root": plan}), &secrets())
            .expect("the folder builds");
        sources.push(ConfiguredSource::Ready(ResolvedSource::adopt(
            name.clone(),
            local,
        )));
        selection.push(name);
    }
    Engine::new(sources, selection)
}

fn global(id: &str) -> GlobalId {
    id.parse().expect("a qualified id")
}

/// A person's text above the slot, and an unrelated key in it, which a settlement must leave
/// byte for byte.
const PERSONS_TEXT: &str = "A person's own words, edited by hand.";

/// Five settlement-shaped updates in one Engine — a status and one `onepipeline.*` key each —
/// and what each cost, every one checked to have kept the person's text and the unrelated key.
fn settlements() -> Vec<usize> {
    let sandbox = Sandbox::new();
    let tasks = (0..5)
        .map(|at| {
            issue(
                &format!("S-{at}"),
                "Todo",
                json!({"content": PERSONS_TEXT, "metadata": {"caller.unrelated": "kept"}}),
            )
        })
        .collect();
    let (config, workspace) = hellopatient(&sandbox, tasks);
    let before = (0..5)
        .map(|at| {
            workspace
                .long_form("tasks", &format!("S-{at}"))
                .expect("a description")
        })
        .collect::<Vec<_>>();
    let engine = engine(&config, None);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    let mut spent = Vec::new();
    runtime.block_on(async {
        for (at, before) in before.iter().enumerate() {
            let id = format!("S-{at}");
            let mut update = TaskUpdate {
                status: Some(Status {
                    category: StatusCategory::Done,
                    name: String::new(),
                }),
                ..TaskUpdate::default()
            };
            update.metadata_set.insert(
                MetadataKey::new("onepipeline.settlement").unwrap(),
                json!({"outcome": "landed", "turn": at}),
            );
            let from = workspace.served().len();
            engine
                .update_task(&global(&format!("patients:{id}")), &update)
                .await
                .expect("the settlement lands");
            spent.push(workspace.served().len() - from);
            let after = workspace.long_form("tasks", &id).expect("a description");
            let (text, slot) = split_slot(&after);
            let (text_before, slot_before) = split_slot(before);
            assert_eq!(text, text_before, "{id}: the person's text, byte for byte");
            assert!(text.starts_with(PERSONS_TEXT), "{id}: {after}");
            assert_eq!(slot["caller.unrelated"], slot_before["caller.unrelated"]);
            assert_eq!(slot["caller.unrelated"], "kept", "{id}: {after}");
            assert_eq!(
                slot["onepipeline.settlement"],
                json!({"outcome": "landed", "turn": at}),
                "{id}: {after}"
            );
            assert_eq!(workspace.state_of(&id).as_deref(), Some("Done"), "{id}");
        }
    });
    spent
}

/// The text above an item's trailing metadata slot, and the slot's JSON.
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
            .trim_end_matches('\n'),
    };
    (
        field[..start].to_owned(),
        serde_json::from_str(encoded).expect("the slot holds JSON"),
    )
}

#[test]
fn measure_linear_requests_per_settlement_update_cold() {
    let spent = settlements();
    report(
        "linear-requests-per-settlement-update-cold",
        spent[0],
        &format!("the first of {spent:?}: the issue read, the resolution and the mutation"),
    );
}

#[test]
fn measure_linear_requests_per_settlement_update_warm() {
    let spent = settlements();
    let warm = spent[1..].iter().copied().max().unwrap_or_default();
    report(
        "linear-requests-per-settlement-update-warm",
        warm,
        &format!("every one after the first of {spent:?}: the issue read and the mutation"),
    );
}

/// The requests one copy sent besides its item writes: everything before the first request
/// only a write sends, and everything after it that no write sends.
fn discovery(served: &[(String, Value)]) -> usize {
    let writes = [&PROJECT_WRITE[..], &TASK_WRITE[..]].concat();
    let starts = [&PROJECT_WRITE[..4], &TASK_WRITE[..4]].concat();
    let first = served
        .iter()
        .position(|(query, _)| starts.contains(&query.as_str()))
        .unwrap_or(served.len());
    first + served[first..].len() - of(&served[first..], &writes)
}

/// The base this change is measured against, and what each operation cost there.
///
/// Measured at commit c9e75a8 — `chore: release v0.2.58` — through the same binary and
/// engine calls against the same loopback workspace, on a configuration that commit completes:
/// the shared workspace's team with no `status_mapping`, which that commit wrote by type and
/// read by type, and its projects at the dataset's own status names. (The `hellopatient`
/// mapping is one it could not load, and a project status it resolved by the item's own name
/// is one it could not find.)
const BASE: &str = "c9e75a8";
const BASE_TASK_SHOW: usize = 2; // ISSUE, ISSUE_COMMENTS
const BASE_PROJECT_SHOW: usize = 1; // PROJECT
const BASE_TASK_LIST_STATUS: usize = 1; // ISSUES
const BASE_PROJECT_LIST_STATUS: usize = 1; // PROJECTS
const BASE_COPY_PROJECT_ALONE: usize = 1; // PROJECTS
const BASE_COPY_ROUTED_MEMBER: usize = 1; // ISSUES
const BASE_COPY_PROJECT_AND_TASKS: usize = 6; // PROJECTS, and ISSUES for each of five tasks
const BASE_RECOPY_PROJECT: usize = 3; // PROJECTS, PROJECT, PROJECT_RELATIONS

#[test]
fn every_linear_read_and_every_copys_discovery_costs_no_more_than_on_the_base() {
    let sandbox = Sandbox::new();
    let (config, workspace) =
        hellopatient(&sandbox, vec![issue("L-READ", "In Progress", json!({}))]);
    let plan = folder(
        &sandbox,
        &[
            (
                "projects/alone.md",
                "---\ntitle: Alone\nstatus: todo\n---\nA project with no tasks.\n",
            ),
            (
                "projects/goal.md",
                "---\ntitle: One goal\nstatus: todo\n---\nAcross both orgs.\n",
            ),
            (
                "tasks/own.md",
                "---\ntitle: Own\nstatus: todo\nproject: goal\n\
                 repositories: [github.com/nickderobertis/lib]\n---\nStays.\n",
            ),
            (
                "tasks/pets.md",
                "---\ntitle: Pets\nstatus: todo\nproject: goal\n\
                 repositories: [github.com/petsinc/api]\n---\nRoutes.\n",
            ),
        ],
    );
    let notes = sandbox.subdirectory("notes");
    sandbox.project_document(&document(&json!({
        "patients": {"plugin": "linear", "config": config},
        "plan": markdown(&plan),
        "notes": {"plugin": "local-md", "config": {"root": notes},
                  "routes": [{"repositories": ["github.com/petsinc/*"], "to": "patients"}]},
    })));
    let alone = counted(
        &sandbox,
        &workspace,
        &[
            "project",
            "copy",
            "plan:alone",
            "--to",
            "patients",
            "--no-tasks",
        ],
    );
    assert_eq!(
        of(&alone, &[onetaskgraph_linear::graphql::PROJECT_CREATE]),
        1
    );
    let routed = counted(
        &sandbox,
        &workspace,
        &["project", "copy", "plan:goal", "--to", "notes"],
    );

    let shown = counted(&sandbox, &workspace, &["task", "show", "patients:L-READ"]).len();
    let project_id = {
        let listed = sandbox
            .command()
            .args(["--json", "project", "list", "--source", "patients"])
            .assert()
            .get_output()
            .clone();
        let listed: Value = serde_json::from_slice(&listed.stdout).expect("a page of projects");
        listed["items"][0]["id"]
            .as_str()
            .expect("a project")
            .to_owned()
    };
    let project_shown = counted(&sandbox, &workspace, &["project", "show", &project_id]).len();
    let tasks_listed = counted(
        &sandbox,
        &workspace,
        &[
            "task",
            "list",
            "--source",
            "patients",
            "--status",
            "in-progress",
        ],
    )
    .len();
    let projects_listed = counted(
        &sandbox,
        &workspace,
        &[
            "project", "list", "--source", "patients", "--status", "todo",
        ],
    )
    .len();

    for (operation, now, base) in [
        ("task show", shown, BASE_TASK_SHOW),
        ("project show", project_shown, BASE_PROJECT_SHOW),
        ("task list --status", tasks_listed, BASE_TASK_LIST_STATUS),
        (
            "project list --status",
            projects_listed,
            BASE_PROJECT_LIST_STATUS,
        ),
        (
            "project copy --no-tasks, besides its write",
            discovery(&alone),
            BASE_COPY_PROJECT_ALONE,
        ),
        (
            "a routed copy, besides its writes",
            discovery(&routed),
            BASE_COPY_ROUTED_MEMBER,
        ),
    ] {
        assert!(
            now <= base,
            "{operation} sends {now} requests, more than the {base} it sent at {BASE}"
        );
    }

    // A copy of a project and its five tasks, and a project re-copied into one engine, besides
    // what their writes send.
    let sandbox = Sandbox::new();
    let (config, workspace) = hellopatient(&sandbox, Vec::new());
    let mut files = vec![(
        "projects/big.md".to_owned(),
        "---\ntitle: Big\nstatus: todo\n---\nFive tasks.\n".to_owned(),
    )];
    for at in 0..5 {
        files.push((
            format!("tasks/t{at}.md"),
            format!("---\ntitle: Task {at}\nstatus: todo\nproject: big\n---\nBody.\n"),
        ));
    }
    files.push((
        "projects/kept.md".to_owned(),
        "---\ntitle: Kept\nstatus: todo\n---\nRewritten.\n".to_owned(),
    ));
    let files = files
        .iter()
        .map(|(path, text)| (path.as_str(), text.as_str()))
        .collect::<Vec<_>>();
    let plan = folder(&sandbox, &files);
    sandbox.project_document(&document(&json!({
        "patients": {"plugin": "linear", "config": config.clone()},
        "plan": markdown(&plan),
    })));
    let big = counted(
        &sandbox,
        &workspace,
        &["project", "copy", "plan:big", "--to", "patients"],
    );
    assert!(
        discovery(&big) <= BASE_COPY_PROJECT_AND_TASKS,
        "a copy of a project and five tasks sends {} requests besides its writes, more than \
         the {BASE_COPY_PROJECT_AND_TASKS} at {BASE}",
        discovery(&big)
    );
    let engine = engine(&config, Some(&plan));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    runtime.block_on(async {
        for at in 0..3 {
            std::fs::write(
                plan.join("projects/kept.md"),
                format!("---\ntitle: Kept {at}\nstatus: todo\n---\nRewritten.\n"),
            )
            .expect("the plan's project");
            let from = workspace.served().len();
            engine
                .copy(&CopyRequest {
                    items: CopyItems::new(vec![global("plan:kept")]).expect("one item"),
                    scope: CopyScope::Projects { tasks: false },
                    destination: SourceName::new("patients").unwrap(),
                    match_by: None,
                    recreate: false,
                    create: false,
                    dry_run: false,
                })
                .await
                .expect("the copy lands");
            if at > 0 {
                let other = discovery(&workspace.served()[from..]);
                assert!(
                    other <= BASE_RECOPY_PROJECT,
                    "a project re-copy sends {other} requests besides its write, more than \
                     the {BASE_RECOPY_PROJECT} at {BASE}"
                );
            }
        }
    });
}
