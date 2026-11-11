//! One private plan's whole road through the store to a Linear team, through the binary, counted
//! at the loopback Linear's endpoint: the plan graph created in a folder of Markdown from a plan
//! template, its first copy with its design document and that document's image, an unchanged
//! re-copy, the design-approval attestation on the copied document, and one settlement — a
//! status and one metadata key, held to the version the copy reported — of each of its four
//! nodes, sent as one `task update-many`.
//!
//! The plan is private: its three tasks name a private repository, and the team source is
//! declared private, so the public boundary is active over every write. Every phase is asserted
//! on what it leaves behind, read back through the binary, and
//! `measure_linear_requests_per_private_plan_road` records what the road sent — each phase's
//! lookup, read and mutation requests apart — for the `linear-requests-per-private-plan-road`
//! budget.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{linear_states, linear_statuses};
use crate::linear_assets::Workspace;
use crate::telemetry::record;

const TEAM_STATES: &[(&str, &str, &str)] = &[
    ("S-ready", "Ready", "unstarted"),
    ("S-working", "Working", "started"),
    ("S-finished", "Finished", "completed"),
];

const PROJECT_STATUSES: &[(&str, &str)] = &[("Scheduled", "planned"), ("Shipped", "completed")];

/// The plan template: three tasks naming the private repository, and the human node that
/// reviews them, each depending on the one before it.
const PLAN: &str = "---\n\
onetaskgraph_template: 1\n\
variables:\n  \
  goal:\n    \
    description: What the plan is for\n\
members:\n  \
  - id: road-build\n    \
    title: \"Build {{ goal }}\"\n    \
    template: {file: task.md}\n    \
    answers: {change: \"Build the service for {{ goal }}.\"}\n    \
    repositories: [github.com/acme/secret-service]\n  \
  - id: road-test\n    \
    title: \"Test {{ goal }}\"\n    \
    template: {file: task.md}\n    \
    answers: {change: \"Test the service for {{ goal }}.\"}\n    \
    depends_on: [road-build]\n    \
    repositories: [github.com/acme/secret-service]\n  \
  - id: road-ship\n    \
    title: \"Ship {{ goal }}\"\n    \
    template: {file: task.md}\n    \
    answers: {change: \"Ship the service for {{ goal }}.\"}\n    \
    depends_on: [road-test]\n    \
    repositories: [github.com/acme/secret-service]\n  \
  - id: road-review\n    \
    title: \"Review {{ goal }}\"\n    \
    template: {file: task.md}\n    \
    answers: {change: \"A person reviews the shipped service for {{ goal }}.\"}\n    \
    depends_on: [road-ship]\n    \
    metadata: {road.human: true}\n\
---\n\
The private plan for {{ goal }}.\n";

const TASK: &str = "---\n\
onetaskgraph_template: 1\n\
variables:\n  \
  change:\n    \
    description: What changes\n\
---\n\
## Change\n\n\
{{ change }}\n";

/// The four nodes, as the plan template names them.
const NODES: [&str; 4] = ["road-build", "road-test", "road-ship", "road-review"];

struct Road {
    sandbox: Sandbox,
    workspace: Workspace,
    templates: PathBuf,
    image: PathBuf,
}

impl Road {
    fn new() -> Self {
        let sandbox = Sandbox::new();
        let mut held = json!({"tasks": [], "projects": [], "documents": [], "labels": [],
                              "task_dependencies": [], "project_dependencies": []});
        held["_linear_states"] = linear_states(TEAM_STATES);
        held["_linear_project_statuses"] = linear_statuses(PROJECT_STATUSES);
        let (mut config, workspace) = Workspace::holding(&sandbox, 1, held);
        config["status_mapping"] = json!({
            "todo": {"task": "Ready", "project": "Scheduled"},
            "in-progress": {"task": "Working"},
            "done": {"task": "Finished", "project": "Shipped"},
        });
        let plans = sandbox.subdirectory("plans");
        let templates = sandbox.subdirectory("templates");
        std::fs::write(templates.join("plan.md"), PLAN).expect("a template");
        std::fs::write(templates.join("task.md"), TASK).expect("a template");
        let image = templates.join("flow.png");
        std::fs::write(&image, onetaskgraph_e2e_support::images::png(7, 60_000)).expect("an image");
        let configuration = json!({
            "sources": {
                "plans": {"plugin": "local-md", "config": {"root": plans},
                          "visibility": "private"},
                "team": {"plugin": "linear", "config": config, "visibility": "private"},
            },
            "write_policy": {
                "check_command": ["python3", "-c",
                    "import sys; sys.stdin.read(); print('{\"verdict\": \"pass\"}')"],
                "visibility_command": ["python3", "-c",
                    "import sys; sys.stdin.read(); print('{\"visibility\": \"private\"}')"],
            },
        });
        sandbox.project_document(&serde_json::to_string_pretty(&configuration).expect("YAML"));
        Self {
            sandbox,
            workspace,
            templates,
            image,
        }
    }

    fn json(&self, arguments: &[&str], code: i32) -> Value {
        let mut command = self.sandbox.command();
        command
            .envs(self.workspace.clock.client_env(0))
            .args(arguments)
            .arg("--json")
            .write_stdin("");
        let output = command.assert().get_output().clone();
        assert_eq!(
            output.status.code(),
            Some(code),
            "`onetaskgraph {}`\n{}{}",
            arguments.join(" "),
            stdout(&output),
            stderr(&output)
        );
        serde_json::from_str(&stdout(&output)).expect("JSON on stdout")
    }

    fn author(&self) {
        self.json(
            &[
                "project",
                "create",
                "plans",
                "--id",
                "road",
                "--title",
                "The private road",
                "--template",
                &path(&self.templates.join("plan.md")),
                "--var",
                "goal=the ledger",
                "--classification",
                "private",
                "--no-interactive",
            ],
            0,
        );
        let body = self.sandbox.subdirectory("bodies").join("design.md");
        std::fs::write(
            &body,
            "# Design\n\nThe service ships behind a flag.\n\n![The flow](./flow.png)\n",
        )
        .expect("a body");
        self.json(
            &[
                "document",
                "create",
                "plans",
                "--id",
                "road-design",
                "--project",
                "road",
                "--title",
                "Design of the ledger service",
                "--body-file",
                &path(&body),
                "--asset",
                &path(&self.image),
            ],
            0,
        );
    }
}

fn path(path: &Path) -> String {
    path.to_str().expect("a UTF-8 path").to_owned()
}

/// What one phase of the road sent the team: its lookups of the team's own vocabulary, its
/// reads, its mutations, and its other requests (asset transfers), apart.
#[derive(Default, Clone, Copy)]
pub(crate) struct Phase {
    pub(crate) lookups: usize,
    pub(crate) reads: usize,
    pub(crate) mutations: usize,
    /// Image bytes sent to, or read back from, the team's storage.
    pub(crate) transfers: usize,
}

impl Phase {
    pub(crate) fn total(self) -> usize {
        self.lookups + self.reads + self.mutations + self.transfers
    }

    /// `served` classified by the root fields each document asks for.
    pub(crate) fn of(served: &[(String, Value)]) -> Self {
        let mut phase = Self::default();
        for (query, _) in served {
            let text = query.trim_start();
            if text.starts_with("mutation") {
                phase.mutations += 1;
            } else if is_lookup(text) {
                phase.lookups += 1;
            } else {
                phase.reads += 1;
            }
        }
        phase
    }

    pub(crate) fn render(self, name: &str) -> String {
        format!(
            "{name} {} ({} lookup, {} read, {} mutation, {} asset transfer)",
            self.total(),
            self.lookups,
            self.reads,
            self.mutations,
            self.transfers
        )
    }
}

/// Whether a query reads only the team's own vocabulary — the organization, the team, its
/// workflow states, its project statuses, its labels — rather than any item.
pub(crate) fn is_lookup(query: &str) -> bool {
    let items = [
        "issue(",
        "issues(",
        "project(",
        "projects(",
        "document(",
        "documents(",
        "comments",
        "relations",
        "originIssues",
        "searchIssues",
    ];
    !items.iter().any(|item| query.contains(item))
}

/// The road, walked once, asserted on, and what each phase sent.
pub(crate) fn walk() -> Vec<(&'static str, Phase)> {
    let road = Road::new();
    road.author();
    let workspace = &road.workspace.held;
    let storage = &road.workspace;
    let transfers = || {
        let (_, sent, verified) = storage.counts();
        sent + verified
    };
    let mut phases = Vec::new();
    let mut mark = workspace.served_count();
    let mut moved = transfers();
    let mut phase =
        |name: &'static str, phases: &mut Vec<(&'static str, Phase)>, mark: &mut usize| {
            let sent = workspace.served_since(*mark);
            if std::env::var_os("ROAD_DEBUG").is_some() {
                for (query, _) in &sent {
                    let head: String = query
                        .split_whitespace()
                        .take(4)
                        .collect::<Vec<_>>()
                        .join(" ");
                    eprintln!("ROOT {name}: {}", &head[..head.len().min(120)]);
                }
            }
            *mark = workspace.served_count();
            let mut measured = Phase::of(&sent);
            let now = transfers();
            measured.transfers = now - moved;
            moved = now;
            phases.push((name, measured));
        };

    // The first copy, with the design document and its image.
    let copied = road.json(
        &[
            "project",
            "copy",
            "plans:road",
            "--documents",
            "--to",
            "team",
        ],
        0,
    );
    phase("first copy", &mut phases, &mut mark);
    let items = copied["items"].as_array().expect("items");
    assert_eq!(
        items.len(),
        6,
        "the project, four tasks and the document: {copied}"
    );
    for item in items {
        assert_eq!(item["action"], "created", "{item}");
    }
    let landed = |source: &str| -> (String, Value) {
        let item = items
            .iter()
            .find(|item| item["source"] == json!(source))
            .unwrap_or_else(|| panic!("{source} was copied: {copied}"));
        (
            item["destination"]
                .as_str()
                .expect("a destination")
                .to_owned(),
            item.clone(),
        )
    };

    // An unchanged re-copy writes nothing.
    let again = road.json(
        &[
            "project",
            "copy",
            "plans:road",
            "--documents",
            "--to",
            "team",
        ],
        0,
    );
    phase("unchanged re-copy", &mut phases, &mut mark);
    for item in again["items"].as_array().expect("items") {
        assert_eq!(item["action"], "unchanged", "{item}");
    }
    assert!(
        phases.last().expect("a phase").1.mutations == 0,
        "an unchanged re-copy sends no mutation"
    );

    // The design approval, on the copied design document.
    let (design, _) = landed("plans:road-design");
    let attested = road.json(
        &[
            "document",
            "attest",
            &design,
            "--kind",
            "design-approval",
            "--by",
            "reviewer",
            "--bar",
            "design-bar-1",
        ],
        0,
    );
    phase("design approval", &mut phases, &mut mark);
    assert_eq!(attested["record"]["kind"], "design-approval", "{attested}");

    // Each node settled once, held to the version it was copied at.
    let mut updates = Vec::new();
    for (at, node) in NODES.iter().enumerate() {
        let (id, _) = landed(&format!("plans:{node}"));
        let shown = road.json(&["task", "show", &id], 0);
        let version = shown["items"][0]["item"]["version"].clone();
        updates.push(json!({
            "item": id,
            "if_version": version,
            "status": if at < 2 { "done" } else { "in-progress" },
            "metadata": {"road.settlement": format!("settled-{at}")},
        }));
    }
    let input = road
        .sandbox
        .subdirectory("batches")
        .join("settlements.json");
    std::fs::write(
        &input,
        json!({"schema_version": 1, "updates": updates}).to_string(),
    )
    .expect("a batch");
    mark = workspace.served_count();
    let settled = road.json(
        &["task", "update-many", "team", "--input", &path(&input)],
        0,
    );
    phase("settlements", &mut phases, &mut mark);
    for (at, result) in settled["results"]
        .as_array()
        .expect("results")
        .iter()
        .enumerate()
    {
        assert_eq!(result["outcome"], "written", "{result}");
        let shown = road.json(&["task", "show", result["item"].as_str().unwrap()], 0);
        let task = &shown["items"][0]["item"];
        assert_eq!(
            task["status"]["category"],
            if at < 2 { "done" } else { "in-progress" },
            "{task}"
        );
        assert_eq!(task["metadata"]["road.settlement"], format!("settled-{at}"));
    }
    phases
}

#[test]
fn a_private_plan_copied_again_writes_nothing_until_its_source_is_edited() {
    let road = Road::new();
    road.author();
    let workspace = &road.workspace.held;
    let copy = [
        "project",
        "copy",
        "plans:road",
        "--documents",
        "--to",
        "team",
    ];
    road.json(&copy, 0);
    for pass in ["second", "third"] {
        let mark = workspace.served_count();
        let again = road.json(&copy, 0);
        for item in again["items"].as_array().expect("items") {
            assert_eq!(item["action"], "unchanged", "the {pass} copy: {item}");
        }
        let mutations: Vec<String> = workspace
            .served_since(mark)
            .into_iter()
            .map(|(query, _)| query)
            .filter(|query| query.trim_start().starts_with("mutation"))
            .collect();
        assert_eq!(
            mutations,
            Vec::<String>::new(),
            "the {pass} copy writes nothing"
        );
    }
    // An edit to the source after the copy linked it is still copied.
    let body = road.sandbox.subdirectory("bodies").join("edited.md");
    std::fs::write(&body, "## Change\n\nBuild it twice as fast.\n").expect("a body");
    road.json(
        &[
            "task",
            "content",
            "set",
            "plans:road-build",
            "--file",
            &path(&body),
        ],
        0,
    );
    let edited = road.json(&copy, 0);
    let build = edited["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|item| item["source"] == "plans:road-build")
        .expect("the edited task");
    assert_eq!(build["action"], "updated", "{edited}");
    let shown = road.json(&["task", "show", build["destination"].as_str().unwrap()], 0);
    assert!(
        shown["items"][0]["item"]["content"]
            .as_str()
            .is_some_and(|content| content.contains("twice as fast")),
        "{shown}"
    );
}

// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] The budget's own journey, in this crate's `test` target whose telemetry the budget reads, as every `measure_*` journey here is.
#[test]
fn measure_linear_requests_per_private_plan_road() {
    let phases = walk();
    let total: usize = phases.iter().map(|(_, phase)| phase.total()).sum();
    let detail = phases
        .iter()
        .map(|(name, phase)| phase.render(name))
        .collect::<Vec<_>>()
        .join("; ");
    record(
        "linear-requests-per-private-plan-road",
        total,
        &format!("{total} requests over the road: {detail}"),
    );
}
