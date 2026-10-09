//! A Linear source declared private, verified once per source instance inside the read a cold
//! write already sends.
//!
//! A Linear workspace is readable by its members alone and nothing makes it readable by anybody
//! else, so what is verified is that the source's own credential really reaches its workspace —
//! `organization{id}` in the team and status resolution every write needs anyway. Every journey
//! spawns the binary against the loopback Linear workspace and asserts on the requests it
//! answered: a declared source spends exactly what an undeclared one does, and a declaration it
//! cannot verify is refused before any mutation.

use std::path::PathBuf;
use std::process::Output;

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{LinearWorkspace, document, linear_workspace_with};

const TEAM_STATES: &[(&str, &str, &str)] = &[
    ("S-icebox", "Icebox", "backlog"),
    ("S-sketch", "Sketch", "backlog"),
    ("S-ready", "Ready", "unstarted"),
    ("S-up-next", "Up Next", "unstarted"),
    ("S-working", "Working", "started"),
    ("S-stuck", "Stuck", "started"),
    ("S-finished", "Finished", "completed"),
    ("S-dropped", "Dropped", "canceled"),
];

const PROJECT_STATUSES: &[(&str, &str)] = &[
    ("Someday", "backlog"),
    ("Concept", "backlog"),
    ("Scheduled", "planned"),
    ("Committed", "planned"),
    ("Underway", "started"),
    ("On Hold", "started"),
    ("Shipped", "completed"),
    ("Abandoned", "canceled"),
];

fn mapping() -> Value {
    json!({
        "backlog":     {"task": "Icebox",   "project": "Someday"},
        "draft":       {"task": "Sketch",   "project": "Concept"},
        "todo":        {"task": "Ready",    "project": "Scheduled"},
        "queued":      {"task": "Up Next",  "project": "Committed"},
        "in-progress": {"task": "Working",  "project": "Underway"},
        "unknown":     {"task": "Stuck",    "project": "On Hold"},
        "done":        {"task": "Finished", "project": "Shipped"},
        "cancelled":   {"task": "Dropped",  "project": "Abandoned"},
    })
}

/// A private folder `plan`, holding one project of `tasks` private tasks, and a Linear source
/// declared `visibility` — `None` for one that declares nothing.
struct Setup {
    sandbox: Sandbox,
    workspace: LinearWorkspace,
}

impl Setup {
    fn new(visibility: Option<&str>, dataset: Value, team: bool, tasks: usize) -> Self {
        let sandbox = Sandbox::new();
        let (mut config, workspace) =
            linear_workspace_with(&sandbox, dataset, TEAM_STATES, PROJECT_STATUSES);
        config["status_mapping"] = mapping();
        if !team {
            config.as_object_mut().expect("a block").remove("team");
        }
        let plan = plan(&sandbox, tasks);
        let mut linear = json!({"plugin": "linear", "config": config});
        let mut folder = json!({"plugin": "local-md", "config": {"root": plan}});
        if let Some(visibility) = visibility {
            linear["visibility"] = json!(visibility);
            folder["visibility"] = json!("private");
        }
        sandbox.project_document(&document(&json!({"linear": linear, "plan": folder})));
        Self { sandbox, workspace }
    }

    fn run(&self, arguments: &[&str]) -> (Output, Vec<(String, Value)>) {
        let from = self.workspace.served().len();
        let output = self
            .sandbox
            .command()
            .args(arguments)
            .assert()
            .get_output()
            .clone();
        (output, self.workspace.served()[from..].to_vec())
    }

    fn ok(&self, arguments: &[&str]) -> Vec<(String, Value)> {
        let (output, served) = self.run(arguments);
        assert_eq!(
            output.status.code(),
            Some(0),
            "`onetaskgraph {}`\n{}{}",
            arguments.join(" "),
            stdout(&output),
            stderr(&output)
        );
        served
    }
}

fn plan(sandbox: &Sandbox, tasks: usize) -> PathBuf {
    let root = sandbox.subdirectory("plan");
    let write = |path: &str, text: String| {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("a folder");
        std::fs::write(path, text).expect("a record");
    };
    write(
        "projects/goal.md",
        "---\ntitle: Goal\nstatus: todo\nclassification: private\n---\nThe goal.\n".to_owned(),
    );
    for index in 0..tasks {
        write(
            &format!("tasks/t-{index}.md"),
            format!(
                "---\ntitle: Task {index}\nstatus: todo\nproject: goal\n\
                 classification: private\n---\nStep {index}.\n"
            ),
        );
    }
    root
}

fn held() -> Value {
    json!({"tasks": [], "projects": [], "documents": [], "labels": [],
           "task_dependencies": [], "project_dependencies": []})
}

fn mutations(served: &[(String, Value)]) -> usize {
    served
        .iter()
        .filter(|(query, _)| query.trim_start().starts_with("mutation"))
        .count()
}

fn resolutions(served: &[(String, Value)]) -> usize {
    served
        .iter()
        .filter(|(query, _)| query == onetaskgraph_linear::graphql::RESOLUTION)
        .count()
}

#[test]
fn a_private_declaration_is_verified_inside_the_cold_read_and_spends_no_request() {
    // The same project copy — a project and three tasks, each create carrying a status — into a
    // source declared private and into one declaring nothing, which the inactive store writes
    // exactly as before. The private items need the declaration; the public copy of the same
    // plan, minus their classification, is what an undeclared source is sent.
    let declared = Setup::new(Some("private"), held(), true, 3);
    let spent = declared.ok(&["project", "copy", "plan:goal", "--to", "linear"]);
    assert_eq!(
        resolutions(&spent),
        1,
        "verified once per instance: {spent:#?}"
    );
    assert_eq!(mutations(&spent), 4, "one create per item: {spent:#?}");

    let undeclared = Setup::new(None, held(), true, 3);
    for index in 0..3 {
        let path = undeclared
            .sandbox
            .project()
            .join(format!("plan/tasks/t-{index}.md"));
        let text = std::fs::read_to_string(&path).expect("a record");
        std::fs::write(&path, text.replace("classification: private\n", "")).expect("written");
    }
    let path = undeclared.sandbox.project().join("plan/projects/goal.md");
    let text = std::fs::read_to_string(&path).expect("a record");
    std::fs::write(&path, text.replace("classification: private\n", "")).expect("written");
    let baseline = undeclared.ok(&["project", "copy", "plan:goal", "--to", "linear"]);
    assert_eq!(
        spent.len(),
        baseline.len(),
        "a declared source spends exactly what an undeclared one does\ndeclared: \
         {spent:#?}\nundeclared: {baseline:#?}"
    );
}

#[test]
fn a_warm_write_to_a_declared_source_sends_nothing_more() {
    let setup = Setup::new(Some("private"), held(), true, 1);
    setup.ok(&["project", "copy", "plan:goal", "--to", "linear"]);
    // A status write by a fresh invocation is one resolution and one mutation, declared or not:
    // the verification rode on the resolution.
    let listed: Value = serde_json::from_str(&stdout(
        &setup
            .run(&["task", "list", "--source", "linear", "--json"])
            .0,
    ))
    .expect("a listing");
    let id = listed["items"][0]["id"]
        .as_str()
        .unwrap_or_else(|| panic!("a copied task: {listed:#}"))
        .to_owned();
    let served = setup.ok(&["task", "status", "set", &id, "done"]);
    assert_eq!(served.len(), 2, "{served:#?}");
    assert_eq!(resolutions(&served), 1);
    assert_eq!(mutations(&served), 1);
}

#[test]
fn a_declaration_that_cannot_be_verified_is_refused_before_any_mutation() {
    // A workspace whose resolution names no organization, and a source with no team, which
    // cannot send the resolution at all.
    let mut withheld = held();
    withheld["_linear_organization"] = Value::Null;
    for (dataset, team, said) in [
        (withheld, true, "named no workspace"),
        (held(), false, "cannot verify that it is private"),
    ] {
        let setup = Setup::new(Some("private"), dataset, team, 1);
        let (output, served) = setup.run(&["task", "copy", "plan:t-0", "--to", "linear", "--json"]);
        assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
        let failure: Value = serde_json::from_str(&stdout(&output)).expect("a failure document");
        assert_eq!(
            failure["failure"]["kind"], "visibility-unreadable",
            "{failure:#}"
        );
        assert!(
            failure["failure"]["message"]
                .as_str()
                .is_some_and(|message| message.contains(said)),
            "{failure:#}"
        );
        assert_eq!(mutations(&served), 0, "{served:#?}");
    }
}
