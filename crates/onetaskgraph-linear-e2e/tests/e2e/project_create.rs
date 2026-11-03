//! A plan whose project Linear does not hold yet, copied into Linear through the binary: the
//! project is created with the plan's tasks filed under it, and a copy retried after the
//! create's answer was lost links the project its first attempt made rather than making a second.
//!
//! Every journey here drives the binary against the loopback Linear of
//! `onetaskgraph_e2e_support::fixtures`, which answers an id it does not hold with Linear's
//! errored not-found response, as Linear does.

use std::path::PathBuf;
use std::process::Output;

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{LinearWorkspace, document, linear_workspace};

/// A folder `plan` holding the project `harbor` with `tasks` tasks under it and the project
/// `lighthouse` holding none, and a Linear source `linear` holding nothing.
pub(crate) struct Setup {
    pub(crate) sandbox: Sandbox,
    pub(crate) workspace: LinearWorkspace,
}

impl Setup {
    fn new(tasks: usize) -> Self {
        let sandbox = Sandbox::new();
        let (config, workspace) = linear_workspace(
            &sandbox,
            json!({"tasks": [], "projects": [], "documents": [], "labels": [],
                   "task_dependencies": [], "project_dependencies": []}),
        );
        let plan = plan(&sandbox, "Harbor rollout", "Lighthouse survey", tasks);
        sandbox.project_document(&document(&json!({
            "linear": {"plugin": "linear", "config": config},
            "plan": {"plugin": "local-md", "config": {"root": plan}},
        })));
        Self { sandbox, workspace }
    }

    fn run(&self, arguments: &[&str]) -> (Output, Vec<(String, Value)>) {
        run(&self.sandbox, &self.workspace, arguments)
    }

    fn ok(&self, arguments: &[&str]) -> (Value, Vec<(String, Value)>) {
        let (output, served) = self.run(arguments);
        (succeeded(arguments, &output), served)
    }
}

fn run(
    sandbox: &Sandbox,
    workspace: &LinearWorkspace,
    arguments: &[&str],
) -> (Output, Vec<(String, Value)>) {
    let from = workspace.served_count();
    let output = sandbox.command().args(arguments).output().unwrap();
    (output, workspace.served_since(from))
}

/// `output`'s JSON, once it exited zero.
pub(crate) fn succeeded(arguments: &[&str], output: &Output) -> Value {
    assert_eq!(
        output.status.code(),
        Some(0),
        "`onetaskgraph {}`\n{}{}",
        arguments.join(" "),
        stdout(output),
        stderr(output)
    );
    serde_json::from_str(&stdout(output)).unwrap_or_else(|error| {
        panic!(
            "`onetaskgraph {}` printed no JSON ({error}): {}",
            arguments.join(" "),
            stdout(output)
        )
    })
}

/// A folder of Markdown under `sandbox` holding the project `harbor`, titled `harbor`, with
/// `tasks` tasks filed under it, and the project `lighthouse`, titled `lighthouse`, with none.
pub(crate) fn plan(sandbox: &Sandbox, harbor: &str, lighthouse: &str, tasks: usize) -> PathBuf {
    let root = sandbox.subdirectory("plan");
    let write = |path: &str, text: String| {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("a folder");
        std::fs::write(path, text).expect("a record");
    };
    write(
        "projects/harbor.md",
        format!("---\ntitle: {harbor}\nstatus: todo\n---\nMove the harbor cranes.\n"),
    );
    write(
        "projects/lighthouse.md",
        format!("---\ntitle: {lighthouse}\nstatus: todo\n---\nSurvey the lighthouse.\n"),
    );
    for index in 0..tasks {
        write(
            &format!("tasks/crane-{index}.md"),
            format!(
                "---\ntitle: {}\nstatus: todo\nproject: harbor\n---\nCrane step {index}.\n",
                task_title(harbor, index)
            ),
        );
    }
    root
}

/// The title of the `index`th task of a plan whose project is titled `project`.
pub(crate) fn task_title(project: &str, index: usize) -> String {
    format!("{project} crane {index}")
}

fn mutations(served: &[(String, Value)]) -> Vec<&str> {
    served
        .iter()
        .map(|(query, _)| query.as_str())
        .filter(|query| query.trim_start().starts_with("mutation"))
        .collect()
}

/// The one outcome of a copy's report for the item `source`.
pub(crate) fn outcome(report: &Value, source: &str) -> Value {
    let outcomes: Vec<&Value> = report["items"]
        .as_array()
        .expect("a report lists its items")
        .iter()
        .filter(|item| item["source"] == source)
        .collect();
    assert_eq!(outcomes.len(), 1, "one outcome for {source}: {report:#}");
    outcomes[0].clone()
}

/// Every project the source `source` holds titled `title`.
pub(crate) fn projects_titled(sandbox: &Sandbox, source: &str, title: &str) -> Vec<Value> {
    let arguments = ["--json", "project", "list", "--source", source];
    let output = sandbox.command().args(arguments).output().unwrap();
    let listed = succeeded(&arguments, &output);
    listed["items"]
        .as_array()
        .expect("a listing")
        .iter()
        .map(|row| row.get("item").unwrap_or(row).clone())
        .filter(|project| project["title"] == title)
        .collect()
}

/// The qualified id of a project a listing of the Linear source `linear` answered.
pub(crate) fn qualified(project: &Value) -> String {
    let id = project["id"].as_str().expect("an id");
    if id.contains(':') {
        id.to_owned()
    } else {
        format!("linear:{id}")
    }
}

/// Every task the source `source` holds under `project`.
pub(crate) fn tasks_under(sandbox: &Sandbox, source: &str, project: &str) -> Vec<Value> {
    let arguments = [
        "--json",
        "task",
        "list",
        "--source",
        source,
        "--project",
        project,
    ];
    let output = sandbox.command().args(arguments).output().unwrap();
    let listed = succeeded(&arguments, &output);
    listed["items"]
        .as_array()
        .expect("a listing")
        .iter()
        .map(|row| row.get("item").unwrap_or(row).clone())
        .collect()
}

/// The titles of `tasks`, sorted.
pub(crate) fn titles(tasks: &[Value]) -> Vec<String> {
    let mut titles: Vec<String> = tasks
        .iter()
        .map(|task| task["title"].as_str().expect("a title").to_owned())
        .collect();
    titles.sort();
    titles
}

#[test]
fn a_new_project_is_created_with_its_tasks_and_a_dry_run_and_a_no_tasks_copy_too() {
    let setup = Setup::new(2);
    let copy = ["--json", "project", "copy", "plan:harbor", "--to", "linear"];

    // A dry run reads everything and writes nothing.
    let dry = [&copy[..], &["--dry-run"]].concat();
    let (report, served) = setup.ok(&dry);
    assert_eq!(
        outcome(&report, "plan:harbor")["action"],
        "created",
        "{report:#}"
    );
    assert!(
        mutations(&served).is_empty(),
        "a dry run writes nothing: {:#?}",
        mutations(&served)
    );
    assert!(projects_titled(&setup.sandbox, "linear", "Harbor rollout").is_empty());

    // The copy itself: one project, with both tasks under it.
    let (report, served) = setup.ok(&copy);
    let landed = outcome(&report, "plan:harbor");
    assert_eq!(landed["action"], "created", "{report:#}");
    let created = landed["destination"].as_str().expect("where it landed");
    assert!(
        mutations(&served).contains(&onetaskgraph_linear::graphql::PROJECT_CREATE),
        "{:#?}",
        mutations(&served)
    );
    let held = projects_titled(&setup.sandbox, "linear", "Harbor rollout");
    assert_eq!(held.len(), 1, "{held:#?}");
    assert_eq!(qualified(&held[0]), created, "{held:#?}");
    assert_eq!(
        titles(&tasks_under(&setup.sandbox, "linear", created)),
        vec![
            task_title("Harbor rollout", 0),
            task_title("Harbor rollout", 1)
        ]
    );

    // A project copied without its tasks is created all the same, and nothing else is.
    let bare = [
        "--json",
        "project",
        "copy",
        "plan:lighthouse",
        "--to",
        "linear",
        "--no-tasks",
    ];
    let (report, served) = setup.ok(&bare);
    let landed = outcome(&report, "plan:lighthouse");
    assert_eq!(landed["action"], "created", "{report:#}");
    let lighthouse = landed["destination"].as_str().expect("where it landed");
    assert!(
        !mutations(&served).contains(&onetaskgraph_linear::graphql::ISSUE_CREATE),
        "{:#?}",
        mutations(&served)
    );
    let held = projects_titled(&setup.sandbox, "linear", "Lighthouse survey");
    assert_eq!(held.len(), 1, "{held:#?}");
    assert_eq!(qualified(&held[0]), lighthouse);
    assert!(tasks_under(&setup.sandbox, "linear", lighthouse).is_empty());
}

#[test]
fn a_project_create_whose_answer_was_lost_is_linked_by_the_retry_and_made_once() {
    let setup = Setup::new(2);
    let copy = ["--json", "project", "copy", "plan:harbor", "--to", "linear"];
    // Linear applies the create and its answer never reaches the binary, and nor does the
    // answer to the read the plugin makes at once for what that create left: so this attempt
    // fails, and only a second command can find the project it made.
    setup
        .workspace
        .lose_answer_once(onetaskgraph_linear::graphql::PROJECT_CREATE);
    setup
        .workspace
        .lose_answer_once(onetaskgraph_linear::graphql::PROJECT);
    let (output, served) = setup.run(&copy);
    assert_ne!(
        output.status.code(),
        Some(0),
        "a copy whose create was not answered does not succeed: {}",
        stdout(&output)
    );
    assert!(
        mutations(&served).contains(&onetaskgraph_linear::graphql::PROJECT_CREATE),
        "{:#?}",
        mutations(&served)
    );
    let made = projects_titled(&setup.sandbox, "linear", "Harbor rollout");
    assert_eq!(
        made.len(),
        1,
        "the first attempt's create landed: {made:#?}"
    );
    let made = qualified(&made[0]);

    let (report, served) = setup.ok(&copy);
    let landed = outcome(&report, "plan:harbor");
    assert_eq!(
        landed["destination"], made,
        "the retry links the project its first attempt made: {report:#}"
    );
    assert!(
        !mutations(&served).contains(&onetaskgraph_linear::graphql::PROJECT_CREATE),
        "the retry creates no second project: {:#?}",
        mutations(&served)
    );
    let held = projects_titled(&setup.sandbox, "linear", "Harbor rollout");
    assert_eq!(held.len(), 1, "one project for the copy: {held:#?}");
    let key = held[0]["metadata"][onetaskgraph_plugin_api::MetadataKey::CREATE_KEY].clone();
    assert!(
        key.is_string(),
        "it carries the copy's create key: {held:#?}"
    );
    assert_eq!(
        titles(&tasks_under(&setup.sandbox, "linear", &made)),
        vec![
            task_title("Harbor rollout", 0),
            task_title("Harbor rollout", 1)
        ],
        "each task once, under that project"
    );
    // And each task once in the whole team, not merely under it.
    let arguments = ["--json", "task", "list", "--source", "linear"];
    let output = setup.sandbox.command().args(arguments).output().unwrap();
    let listed = succeeded(&arguments, &output);
    assert_eq!(
        listed["items"].as_array().map(Vec::len),
        Some(2),
        "{listed:#}"
    );
}
