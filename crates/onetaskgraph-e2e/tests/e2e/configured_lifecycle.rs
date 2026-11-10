//! A plan lifecycle adopted by configuration alone: the store configured as an orchestrator
//! repository with none of this repository's own files would configure it, and driven through
//! the built binary there.
//!
//! Its destinations are three Linear team sources, each on a Linear stand-in of its own, none of
//! them a default and no repository route declared, so every destination is named by
//! configuration; its drafts are in a folder of Markdown called `drafts` and its plans in one
//! called `authoring`; and its own personas, templates and status vocabulary live in a
//! directory of its own. Through it: a plan graph created in `authoring` from that directory's
//! own project template; an attestation on one of its tasks, on the project and on a document,
//! each `valid`, then `void` with `content-moved` after an edit and `bar-moved` under another
//! bar; a project copy with its documents to one named Linear team source, whose copies check
//! `valid` with the source's own records while the other two team sources are sent nothing; and
//! a task rendered from that directory's own template whose `object` variable declares a schema,
//! accepted with a conforming answer and refused naming the variable and the JSON pointer with a
//! violating one.

use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{LinearWorkspace, document, linear_workspace};

/// The orchestrator's own plan template: three members, two templates, one depending on both.
const PLAN: &str = "---\n\
onetaskgraph_template: 1\n\
variables:\n  \
  slug:\n    \
    description: The plan's short name\n  \
  goal:\n    \
    description: What the plan delivers\n\
members:\n  \
  - id: \"{{ slug }}-spike\"\n    \
    title: \"Spike: {{ goal }}\"\n    \
    template: {file: work.md}\n    \
    answers: {persona: planner, outcome: \"what {{ goal }} costs\"}\n  \
  - id: \"{{ slug }}-review\"\n    \
    title: \"Review: {{ goal }}\"\n    \
    template: {file: review.md}\n    \
    answers: {bar: design-bar}\n  \
  - id: \"{{ slug }}-build\"\n    \
    title: \"Build: {{ goal }}\"\n    \
    template: {file: work.md}\n    \
    answers: {persona: builder, outcome: \"{{ goal }}, shipped\"}\n    \
    depends_on: [\"{{ slug }}-spike\", \"{{ slug }}-review\"]\n    \
    metadata: {crew.priority_band: high}\n\
---\n\
# {{ goal }}\n\
\n\
Planned by the crew's own lifecycle.\n";

/// A work task: who does it, and what it delivers.
const WORK: &str = "---\n\
onetaskgraph_template: 1\n\
variables:\n  \
  persona:\n    \
    description: The persona who does it\n  \
  outcome:\n    \
    description: What it delivers\n\
---\n\
Persona: {{ persona }}\n\
Outcome: {{ outcome }}\n";

/// A review task, against a named bar.
const REVIEW: &str = "---\n\
onetaskgraph_template: 1\n\
variables:\n  \
  bar:\n    \
    description: The bar reviewed against\n\
---\n\
Review against {{ bar }}.\n";

/// The crew's design-document template.
const DESIGN: &str = "---\n\
onetaskgraph_template: 1\n\
variables:\n  \
  summary:\n    \
    description: The design in a line\n\
---\n\
Design: {{ summary }}\n";

/// A brief whose `object` variable declares a schema: a budget record the crew validates where
/// it is rendered.
const BRIEF: &str = "---\n\
onetaskgraph_template: 1\n\
variables:\n  \
  budget:\n    \
    description: The budget this brief is held to\n    \
    type: object\n    \
    schema:\n      \
      type: object\n      \
      required: [id, target]\n      \
      additionalProperties: false\n      \
      properties:\n        \
        id: {type: string, pattern: \"^[a-z-]+$\"}\n        \
        target: {type: number, exclusiveMinimum: 0}\n\
---\n\
Budget {{ budget.id }}: at most {{ budget.target }}.\n";

/// The crew's own status vocabulary for its Linear teams: its tasks move through the team's
/// review states by their own names; its projects keep the workspace's.
fn vocabulary() -> Value {
    json!({
        "backlog": {"task": "Proposed", "project": "Backlog"},
        "todo": {"task": "Queued", "project": "Todo"},
        "in-progress": {"task": "In Review", "project": "Doing"},
        "done": {"task": "Done", "project": "Shipped"},
        "cancelled": "Canceled",
    })
}

/// The three Linear team sources the crew writes plans and follow-ups to.
const TEAMS: [&str; 3] = ["plans", "followups", "operations"];

struct Crew {
    sandbox: Sandbox,
    own: PathBuf,
    teams: Vec<(&'static str, LinearWorkspace)>,
}

impl Crew {
    fn new() -> Self {
        let sandbox = Sandbox::new();
        let own = sandbox.subdirectory("crew");
        let templates = own.join("templates");
        std::fs::create_dir_all(&templates).expect("the crew's templates");
        std::fs::create_dir_all(own.join("personas")).expect("the crew's personas");
        for (name, text) in [
            ("plan.md", PLAN),
            ("work.md", WORK),
            ("review.md", REVIEW),
            ("design.md", DESIGN),
            ("brief.md", BRIEF),
        ] {
            std::fs::write(templates.join(name), text).expect("a template");
        }
        for persona in ["planner", "builder"] {
            std::fs::write(
                own.join("personas").join(format!("{persona}.md")),
                format!("# The {persona}\n"),
            )
            .expect("a persona");
        }
        let mut sources = json!({
            "drafts": {"plugin": "local-md", "config": {"root": sandbox.subdirectory("drafts")}},
            "authoring": {"plugin": "local-md",
                          "config": {"root": sandbox.subdirectory("authoring")}},
        });
        let mut teams = Vec::new();
        for team in TEAMS {
            let (mut config, workspace) = linear_workspace(
                &sandbox,
                json!({"tasks": [], "projects": [], "documents": [], "labels": [],
                       "task_dependencies": [], "project_dependencies": []}),
            );
            config["status_mapping"] = vocabulary();
            sources[team] = json!({"plugin": "linear", "config": config});
            teams.push((team, workspace));
        }
        // No `default_sources` and no `routes`: every destination is named where it is used.
        sandbox.project_document(&document(&sources));
        Self {
            sandbox,
            own,
            teams,
        }
    }

    fn template(&self, name: &str) -> String {
        path(&self.own.join("templates").join(name))
    }

    fn input(&self, name: &str, text: &str) -> String {
        let file = self.sandbox.subdirectory("inputs").join(name);
        std::fs::write(&file, text).expect("an input");
        path(&file)
    }

    fn exits(&self, arguments: &[&str], code: i32) -> Output {
        let output = self
            .sandbox
            .command()
            .args(arguments)
            .assert()
            .get_output()
            .clone();
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

    fn json(&self, arguments: &[&str], code: i32) -> Value {
        let mut arguments = arguments.to_vec();
        arguments.push("--json");
        serde_json::from_str(&stdout(&self.exits(&arguments, code))).expect("JSON on stdout")
    }

    fn attest(&self, noun: &str, id: &str) -> Value {
        self.json(
            &[
                noun,
                "attest",
                id,
                "--kind",
                "plan-review",
                "--by",
                "reviewer",
                "--bar",
                "bar-1",
            ],
            0,
        )["record"]
            .clone()
    }

    /// The check of `noun ID` under `bar`, held to its exit code.
    fn check(&self, noun: &str, id: &str, bar: &str) -> Value {
        let arguments = [
            noun,
            "attestation",
            "check",
            id,
            "--kind",
            "plan-review",
            "--bar",
            bar,
        ];
        let mut with_json = arguments.to_vec();
        with_json.push("--json");
        let output = self
            .sandbox
            .command()
            .args(&with_json)
            .assert()
            .get_output()
            .clone();
        let checked: Value = serde_json::from_str(&stdout(&output))
            .unwrap_or_else(|_| panic!("a check answers JSON: {}", stderr(&output)));
        let expected = if checked["state"] == "valid" { 0 } else { 3 };
        assert_eq!(output.status.code(), Some(expected), "{checked}");
        checked
    }

    /// The requests the Linear team source `team` was sent.
    fn sent_to(&self, team: &str) -> usize {
        self.teams
            .iter()
            .find(|(name, _)| *name == team)
            .expect("a team source")
            .1
            .served_count()
    }
}

fn path(path: &Path) -> String {
    path.to_str().expect("a UTF-8 path").to_owned()
}

#[test]
fn a_crew_configured_only_by_its_own_files_runs_a_plan_lifecycle_through_the_store() {
    let crew = Crew::new();

    // A plan graph in `authoring`, from the crew's own project template.
    let plan = crew.template("plan.md");
    let created = stdout(&crew.exits(
        &[
            "project",
            "create",
            "authoring",
            "--id",
            "launch",
            "--title",
            "Launch the widget",
            "--template",
            &plan,
            "--var",
            "slug=launch",
            "--var",
            "goal=the widget",
            "--no-interactive",
        ],
        0,
    ));
    assert_eq!(
        created.lines().collect::<Vec<_>>(),
        [
            "authoring:launch",
            "authoring:launch-spike",
            "authoring:launch-review",
            "authoring:launch-build",
        ]
    );
    let build = "authoring:launch-build";
    let shown = crew.json(&["task", "show", build], 0)["items"][0]["item"].clone();
    assert_eq!(
        shown["content"],
        "Persona: builder\nOutcome: the widget, shipped\n"
    );
    let mut edges: Vec<String> = crew.json(&["task", "deps", build], 0)["items"]
        .as_array()
        .expect("edges")
        .iter()
        .map(|edge| edge["to"]["id"].as_str().unwrap().to_owned())
        .collect();
    edges.sort();
    assert_eq!(edges, ["authoring:launch-review", "authoring:launch-spike"]);
    let design = crew.template("design.md");
    let doc = stdout(&crew.exits(
        &[
            "document",
            "create",
            "authoring",
            "--project",
            "launch",
            "--id",
            "launch-design",
            "--title",
            "The widget's design",
            "--template",
            &design,
            "--var",
            "summary=One widget, one board",
            "--no-interactive",
        ],
        0,
    ))
    .trim()
    .to_owned();

    // Attestations on a task, the project and the document: valid, then void after an edit,
    // and bar-moved under another bar.
    let project = "authoring:launch";
    let edits: [(&str, &str, Vec<&str>); 3] = [
        (
            "task",
            build,
            vec![
                "task",
                "metadata",
                "set",
                build,
                "crew.priority_band",
                "\"low\"",
            ],
        ),
        (
            "project",
            project,
            vec![
                "project",
                "metadata",
                "set",
                project,
                "crew.owner",
                "\"ops\"",
            ],
        ),
        (
            "document",
            &doc,
            vec!["document", "metadata", "set", &doc, "crew.owner", "\"ops\""],
        ),
    ];
    for (noun, id, edit) in &edits {
        crew.attest(noun, id);
        assert_eq!(
            crew.check(noun, id, "bar-1")["state"],
            "valid",
            "{noun} {id}"
        );
        crew.exits(edit, 0);
        let moved = crew.check(noun, id, "bar-1");
        assert_eq!(
            (&moved["state"], &moved["reason"]),
            (&json!("void"), &json!("content-moved")),
            "{noun} {id} after an edit"
        );
        crew.attest(noun, id);
        let bar = crew.check(noun, id, "bar-2");
        assert_eq!(
            (&bar["state"], &bar["reason"]),
            (&json!("void"), &json!("bar-moved")),
            "{noun} {id} under another bar"
        );
    }
    let records: Vec<(&str, String, Value)> = edits
        .iter()
        .map(|(noun, id, _)| (*noun, (*id).to_owned(), crew.attest(noun, id)))
        .collect();

    // A project copy with its documents to one named Linear team source.
    let before: Vec<usize> = TEAMS.iter().map(|team| crew.sent_to(team)).collect();
    let copied = crew.json(&["project", "copy", project, "--to", "plans"], 0);
    let documents = crew.json(&["document", "copy", &doc, "--to", "plans"], 0);
    let landed = |report: &Value, source: &str| -> String {
        report["items"]
            .as_array()
            .expect("outcomes")
            .iter()
            .find(|outcome| outcome["source"] == source)
            .and_then(|outcome| outcome["destination"].as_str())
            .unwrap_or_else(|| panic!("{source} landed: {report}"))
            .to_owned()
    };
    for (noun, id, record) in &records {
        let copy = if *noun == "document" {
            landed(&documents, id)
        } else {
            landed(&copied, id)
        };
        assert!(
            copy.starts_with("plans:"),
            "{copy} is in the named team source"
        );
        let checked = crew.check(noun, &copy, "bar-1");
        assert_eq!(
            (&checked["state"], &checked["record"]),
            (&json!("valid"), record),
            "the copy of {id} holds the source's record, and it holds"
        );
    }
    assert!(
        crew.sent_to("plans") > before[0],
        "the named team source was written"
    );
    assert_eq!(
        (crew.sent_to("followups"), crew.sent_to("operations")),
        (before[1], before[2]),
        "no team source the copy did not name was sent anything"
    );

    // A brief rendered in `drafts` from the crew's own schema'd template.
    let brief = crew.template("brief.md");
    let good = crew.input("good.yaml", "budget: {id: approval-seconds, target: 7}\n");
    let bad = crew.input("bad.yaml", "budget: {id: approval-seconds, target: -1}\n");
    let accepted = stdout(&crew.exits(
        &[
            "task",
            "create",
            "drafts",
            "--project",
            "briefs",
            "--title",
            "Approval budget",
            "--template",
            &brief,
            "--answers",
            &good,
            "--no-interactive",
        ],
        0,
    ))
    .trim()
    .to_owned();
    let accepted =
        crew.json(&["task", "show", &accepted], 0)["items"][0]["item"]["content"].clone();
    assert_eq!(accepted, "Budget approval-seconds: at most 7.\n");
    let refused = crew.exits(
        &[
            "task",
            "create",
            "drafts",
            "--project",
            "briefs",
            "--title",
            "Bad budget",
            "--template",
            &brief,
            "--answers",
            &bad,
            "--no-interactive",
        ],
        2,
    );
    let message = stderr(&refused);
    assert!(
        message.contains(
            "the answer to \"budget\" does not satisfy its schema at JSON pointer \"/target\""
        ),
        "{message}"
    );
    let briefs = crew.json(&["task", "list", "--source", "drafts"], 0)["items"]
        .as_array()
        .expect("tasks")
        .len();
    assert_eq!(briefs, 1, "the refused brief wrote nothing");
}
