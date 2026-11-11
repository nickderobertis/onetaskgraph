//! A plan copied into a Linear team by a store configured the way another orchestrator would
//! configure it, through the binary.
//!
//! Nothing here comes from any orchestrator's own files: the store is configuration alone —
//! three Linear team sources, each on a loopback Linear of its own, none of them a default and no
//! repository route between any two; a folder of Markdown `drafts` and one `authoring`, where the
//! plan is written; and templates of the journey's own in a directory of their own, which the
//! plan is rendered from. A dry run of the plan's copy to one named team says how large each
//! item's composed body would be, and the copy itself lands every item there and nowhere else.

use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{LinearWorkspace, document, linear_workspace};

/// The Linear team sources, each its own workspace, none a default.
const TEAMS: [&str; 3] = ["team-north", "team-south", "team-east"];

/// The one the plan is copied to.
const TEAM: &str = "team-south";

/// A project of the journey's own design: its goal, and who signs it off.
const PROJECT: &str = "---\n\
onetaskgraph_template: 1\n\
variables:\n  \
  goal:\n    \
    description: What the plan achieves\n  \
  approver:\n    \
    description: Who signs it off\n\
---\n\
# Goal\n\n\
{{ goal }}\n\n\
Signed off by {{ approver }}.\n";

/// A task of the journey's own design: the change, and how it is shown done.
const TASK: &str = "---\n\
onetaskgraph_template: 1\n\
variables:\n  \
  change:\n    \
    description: What changes\n  \
  done:\n    \
    description: How it is shown done\n\
---\n\
## Change\n\n\
{{ change }}\n\n\
## Done when\n\n\
{{ done }}\n";

/// A design document of the journey's own design: its decisions, one per line.
const DESIGN: &str = "---\n\
onetaskgraph_template: 1\n\
variables:\n  \
  decisions:\n    \
    description: What was decided\n    \
    type: list\n\
---\n\
# Decisions\n\n\
{% for decision in decisions %}- {{ decision }}\n{% endfor %}";

struct Store {
    sandbox: Sandbox,
    templates: PathBuf,
    teams: Vec<(&'static str, LinearWorkspace)>,
}

impl Store {
    fn new() -> Self {
        let sandbox = Sandbox::new();
        let empty = json!({"tasks": [], "projects": [], "documents": [], "labels": [],
            "task_dependencies": [], "project_dependencies": []});
        let mut sources = json!({
            "drafts": {"plugin": "local-md", "config": {"root": sandbox.subdirectory("drafts")}},
            "authoring": {"plugin": "local-md",
                "config": {"root": sandbox.subdirectory("authoring")}},
        });
        let mut teams = Vec::new();
        for team in TEAMS {
            let (config, workspace) = linear_workspace(&sandbox, empty.clone());
            sources[team] = json!({"plugin": "linear", "config": config});
            teams.push((team, workspace));
        }
        // Configuration alone: no `default_sources`, and no `routes` on any source.
        sandbox.project_document(&document(&sources));
        let templates = sandbox.subdirectory("house-templates");
        for (name, text) in [
            ("project.md", PROJECT),
            ("task.md", TASK),
            ("design.md", DESIGN),
        ] {
            std::fs::write(templates.join(name), text).expect("a template");
        }
        Self {
            sandbox,
            templates,
            teams,
        }
    }

    fn run(&self, arguments: &[&str]) -> Output {
        let mut command = self.sandbox.command();
        command.args(arguments);
        command.assert().get_output().clone()
    }

    fn ok(&self, arguments: &[&str]) -> String {
        let output = self.run(arguments);
        assert_eq!(
            output.status.code(),
            Some(0),
            "`onetaskgraph {}`\n{}{}",
            arguments.join(" "),
            stdout(&output),
            stderr(&output)
        );
        stdout(&output)
    }

    fn template(&self, name: &str) -> String {
        path(&self.templates.join(name))
    }

    /// The plan, rendered from the journey's own templates into `authoring`: a project, three
    /// tasks and a design document — and, apart, a draft in `drafts` nothing copies.
    fn author(&self) {
        self.ok(&[
            "project",
            "create",
            "authoring",
            "--id",
            "plan",
            "--title",
            "Adopt the store",
            "--template",
            &self.template("project.md"),
            "--var",
            "goal=Every plan lands in its team in one copy.",
            "--var",
            "approver=the team lead",
        ]);
        for (n, change) in [
            (1, "Configure the team sources."),
            (2, "Write the templates."),
            (3, "Copy the first plan."),
        ] {
            self.ok(&[
                "task",
                "create",
                "authoring",
                "--project",
                "plan",
                "--title",
                &format!("Step {n}"),
                "--template",
                &self.template("task.md"),
                "--var",
                &format!("change={change}"),
                "--var",
                "done=The team reads it in its own tracker.",
            ]);
        }
        self.ok(&[
            "document",
            "create",
            "authoring",
            "--project",
            "plan",
            "--title",
            "Design of the adoption",
            "--template",
            &self.template("design.md"),
            "--var",
            r#"decisions=["Configuration alone", "One copy per plan"]"#,
        ]);
        self.ok(&[
            "task",
            "create",
            "drafts",
            "--project",
            "ideas",
            "--title",
            "An idea for later",
            "--body-file",
            &path(&self.templates.join("task.md")),
        ]);
    }
}

fn path(path: &Path) -> String {
    path.to_str().expect("a UTF-8 path").to_owned()
}

#[test]
fn a_plan_copies_into_a_named_linear_team_of_a_store_configured_by_configuration_alone() {
    let store = Store::new();
    store.author();
    let copy = [
        "project",
        "copy",
        "authoring:plan",
        "--documents",
        "--to",
        TEAM,
        "--json",
    ];

    let mut dry = copy.to_vec();
    dry.push("--dry-run");
    let report: Value = serde_json::from_str(&store.ok(&dry)).expect("a copy report");
    let items = report["items"].as_array().expect("items");
    assert_eq!(
        items.len(),
        5,
        "the project, its three tasks and its document: {report}"
    );
    let mut sized = std::collections::BTreeMap::new();
    for item in items {
        assert_eq!(item["action"], json!("created"), "{item}");
        let size = item["body_size"]
            .as_u64()
            .unwrap_or_else(|| panic!("a size: {item}"));
        sized.insert(item["source"].as_str().expect("a source").to_owned(), size);
    }
    for (team, workspace) in &store.teams {
        assert!(
            workspace
                .served()
                .iter()
                .all(|(query, _)| !query.trim_start().starts_with("mutation")),
            "a dry run writes nothing to {team}"
        );
    }

    let report: Value = serde_json::from_str(&store.ok(&copy)).expect("a copy report");
    let items = report["items"].as_array().expect("items");
    assert_eq!(items.len(), 5, "{report}");
    let sources: Vec<&str> = items
        .iter()
        .map(|item| item["source"].as_str().expect("a source"))
        .collect();
    assert_eq!(sources[0], "authoring:plan");
    assert!(
        sources
            .iter()
            .any(|source| source.starts_with("authoring:") && *source != "authoring:plan")
    );
    for item in items {
        assert_eq!(item["action"], json!("created"), "{item}");
        assert!(
            item["destination"]
                .as_str()
                .is_some_and(|landed| landed.starts_with(&format!("{TEAM}:"))),
            "{item}"
        );
        assert_eq!(
            item.get("body_size"),
            None,
            "a copy that writes reports no size"
        );
    }
    for (team, workspace) in &store.teams {
        let sent = workspace.served();
        if *team == TEAM {
            assert!(
                sent.iter()
                    .any(|(query, _)| query.contains("documentCreate")),
                "the document landed in {team}"
            );
        } else {
            assert!(sent.is_empty(), "{team} was asked nothing: {sent:?}");
        }
    }
    // Each item's dry-run size is the length of the long form the team then stores for it —
    // its rendered content and the slot Linear keeps beside it — and an unchanged re-copy's dry
    // run sizes each exactly so again.
    let (_, workspace) = store
        .teams
        .iter()
        .find(|(team, _)| *team == TEAM)
        .expect("the team");
    let again: Value = serde_json::from_str(&store.ok(&dry)).expect("a copy report");
    for (index, item) in items.iter().enumerate() {
        let source = item["source"].as_str().expect("a source");
        let native = item["destination"]
            .as_str()
            .and_then(|landed| landed.strip_prefix(&format!("{TEAM}:")))
            .expect("landed in the team");
        let kind = match index {
            0 => "projects",
            4 => "documents",
            _ => "tasks",
        };
        let stored = workspace
            .long_form(kind, native)
            .unwrap_or_else(|| panic!("{source} is held as one of the team's {kind}"))
            .chars()
            .count() as u64;
        assert_eq!(sized[source], stored, "{source}: the first dry run's size");
        let resized = &again["items"][index];
        assert_eq!(resized["source"], json!(source));
        assert_eq!(resized["action"], json!("unchanged"), "{resized}");
        assert_eq!(
            resized["body_size"],
            json!(stored),
            "{source}: a re-copy's size"
        );
    }

    // Read back through the binary: every task is there, under the team's own project.
    let listed: Value =
        serde_json::from_str(&store.ok(&["task", "list", "--source", TEAM, "--json"]))
            .expect("a listing");
    let titles: Vec<&str> = listed["items"]
        .as_array()
        .expect("listed items")
        .iter()
        .map(|task| task["item"]["title"].as_str().expect("a title"))
        .collect();
    for n in 1..=3 {
        assert!(titles.contains(&format!("Step {n}").as_str()), "{titles:?}");
    }
}

/// The batched store calls against a named team of the store configured by configuration alone:
/// one `task find` of the copied plan's steps, a comment on each, and a held update of each — each
/// outcome read back through the binary, and no other team asked anything.
#[test]
fn the_batched_calls_reach_a_named_team_of_a_store_configured_by_configuration_alone() {
    let store = Store::new();
    store.author();
    store.ok(&[
        "project",
        "copy",
        "authoring:plan",
        "--documents",
        "--to",
        TEAM,
        "--json",
    ]);
    let batches = store.sandbox.subdirectory("batches");
    let write = |name: &str, document: &Value| {
        let path = batches.join(name);
        std::fs::write(&path, document.to_string()).expect("a batch");
        path.to_str().expect("a UTF-8 path").to_owned()
    };
    let queries = write(
        "find.json",
        &json!({"schema_version": 1, "queries": [
            {"key": "steps", "text": "Step", "in": "title"},
            {"key": "none", "text": "no step holds this"},
        ]}),
    );
    let found: Value =
        serde_json::from_str(&store.ok(&["task", "find", TEAM, "--queries", &queries, "--json"]))
            .expect("the answers");
    let steps = &found["answers"][0];
    assert_eq!(steps["key"], "steps");
    assert_eq!(steps["complete"], true, "{steps}");
    let items: Vec<(String, String)> = steps["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|item| {
            (
                item["id"].as_str().expect("an id").to_owned(),
                item["item"]["version"]
                    .as_str()
                    .expect("a version")
                    .to_owned(),
            )
        })
        .collect();
    assert_eq!(items.len(), 3, "{found}");
    assert_eq!(found["answers"][1]["items"], json!([]));

    let comments = write(
        "comments.json",
        &json!({"schema_version": 1, "comments": items.iter().map(|(id, _)| json!({
            "item": id, "body": "Picked up by the team."})).collect::<Vec<_>>()}),
    );
    let added: Value = serde_json::from_str(&store.ok(&[
        "task", "comment", "add-many", TEAM, "--input", &comments, "--json",
    ]))
    .expect("the results");
    for result in added["results"].as_array().expect("results") {
        assert_eq!(result["outcome"], "written", "{result}");
    }
    let updates = write(
        "updates.json",
        &json!({"schema_version": 1, "updates": items.iter().map(|(id, version)| json!({
            "item": id, "if_version": version, "status": "in-progress",
            "metadata": {"adoption.picked": true}})).collect::<Vec<_>>()}),
    );
    let updated: Value = serde_json::from_str(&store.ok(&[
        "task",
        "update-many",
        TEAM,
        "--input",
        &updates,
        "--json",
    ]))
    .expect("the results");
    for result in updated["results"].as_array().expect("results") {
        assert_eq!(result["outcome"], "written", "{result}");
    }
    for (id, _) in &items {
        let shown: Value =
            serde_json::from_str(&store.ok(&["task", "show", id, "--json"])).expect("a task");
        let task = &shown["items"][0]["item"];
        assert_eq!(task["status"]["category"], "in-progress", "{task}");
        assert_eq!(task["metadata"]["adoption.picked"], true, "{task}");
        let comments = shown["items"][0]["comments"]
            .as_array()
            .or_else(|| shown["comments"].as_array())
            .cloned()
            .unwrap_or_default();
        assert!(
            comments
                .iter()
                .any(|comment| comment["body"] == "Picked up by the team."),
            "{shown}"
        );
    }
    for (team, workspace) in &store.teams {
        if *team != TEAM {
            assert!(workspace.served().is_empty(), "{team} was asked nothing");
        }
    }
}
