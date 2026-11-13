//! Content-keyed attestations, driven through the binary: `task`, `project` and `document
//! attest` and `attestation check`, against a folder of Markdown, the loopback GitHub board and
//! the loopback Linear workspace.
//!
//! A record checks `valid` once written; editing the item's body, its title, a template answer,
//! a dependency edge, its repositories or a caller-owned metadata key not excluded makes it
//! `void` with `content-moved`; changing an excluded key or the item's status does not; a check
//! given another bar answers `bar-moved`; and a project copied with its documents lands copies
//! that check `valid` with the source's own records.

use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::document_store::interpreter;
use crate::fixtures::{
    LinearWorkspace, document, github_projects_with_board, linear_rate_limited, linear_workspace,
};

/// A task template: one required answer.
const TASK: &str = "---\n\
onetaskgraph_template: 1\n\
variables:\n  \
  goal:\n    \
    description: What the task is for\n\
---\n\
Goal: {{ goal }}\n";

/// A project template: one required answer.
const PLAN: &str = "---\n\
onetaskgraph_template: 1\n\
variables:\n  \
  goal:\n    \
    description: What the plan is for\n\
---\n\
The plan: {{ goal }}\n";

/// A design-document template: one required answer.
const DESIGN: &str = "---\n\
onetaskgraph_template: 1\n\
variables:\n  \
  summary:\n    \
    description: The design in a line\n\
---\n\
Design: {{ summary }}\n";

/// The kind every journey records, and the bar it records against.
const KIND: &str = "plan-review";
const BAR: &str = "bar-1";
/// The caller key every task record leaves out of its digest.
const EXCLUDED: &str = "myapp.owner";

/// A sandbox holding the folder of Markdown `notes`, the templates, and — for the journeys
/// about hosted destinations — the loopback board `board` and the loopback Linear `linear`.
pub(crate) struct Plan {
    pub(crate) sandbox: Sandbox,
    notes: PathBuf,
    templates: PathBuf,
    /// The loopback Linear workspace, for the journeys about hosted destinations.
    linear: Option<LinearWorkspace>,
}

impl Plan {
    pub(crate) fn new(hosts: bool) -> Self {
        let sandbox = Sandbox::new();
        let notes = sandbox.subdirectory("notes");
        let templates = sandbox.subdirectory("templates");
        for (name, text) in [("task.md", TASK), ("plan.md", PLAN), ("design.md", DESIGN)] {
            std::fs::write(templates.join(name), text).expect("a template");
        }
        let mut sources = json!({"notes": {"plugin": "local-md", "config": {"root": notes}}});
        let mut workspace = None;
        if hosts {
            let (board, _) = github_projects_with_board(&sandbox);
            sources["board"] = json!({"plugin": "github-projects", "config": board});
            let (linear, held) = linear_workspace(
                &sandbox,
                json!({"tasks": [], "projects": [], "documents": [], "labels": [],
                       "task_dependencies": [], "project_dependencies": []}),
            );
            sources["linear"] = json!({"plugin": "linear", "config": linear});
            workspace = Some(held);
            sandbox.secrets_file(
                "GITHUB_PROJECTS_FIXTURE_TOKEN=test-token\nLINEAR_API_KEY=fixture-key\n",
            );
        }
        sandbox.project_document(&document(&sources));
        Self {
            sandbox,
            notes,
            templates,
            linear: workspace,
        }
    }

    pub(crate) fn template(&self, name: &str) -> String {
        path(&self.templates.join(name))
    }

    fn file(&self, name: &str, text: &str) -> String {
        let file = self.sandbox.subdirectory("inputs").join(name);
        std::fs::write(&file, text).expect("an input file");
        path(&file)
    }

    pub(crate) fn run(&self, arguments: &[&str]) -> Output {
        self.sandbox
            .command()
            .args(arguments)
            .assert()
            .get_output()
            .clone()
    }

    pub(crate) fn exits(&self, arguments: &[&str], code: i32) -> Output {
        let output = self.run(arguments);
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

    pub(crate) fn json(&self, arguments: &[&str]) -> Value {
        self.json_exiting(arguments, 0)
    }

    pub(crate) fn json_exiting(&self, arguments: &[&str], code: i32) -> Value {
        let mut arguments = arguments.to_vec();
        arguments.push("--json");
        let output = self.exits(&arguments, code);
        serde_json::from_str(&stdout(&output)).unwrap_or_else(|error| {
            panic!(
                "`onetaskgraph {}` wrote no JSON ({error})",
                arguments.join(" ")
            )
        })
    }

    /// `<noun> attest ID --kind plan-review --by ada --bar bar-1`, a task's leaving out
    /// [`EXCLUDED`], answering the record written.
    pub(crate) fn attest(&self, noun: &str, id: &str) -> Value {
        let mut arguments = vec![
            noun, "attest", id, "--kind", KIND, "--by", "ada", "--bar", BAR,
        ];
        if noun == "task" {
            arguments.extend(["--exclude", EXCLUDED]);
        }
        let attested = self.json(&arguments);
        assert_eq!(attested["id"], id);
        assert_eq!(attested["record"]["kind"], KIND);
        assert_eq!(attested["record"]["bar"], BAR);
        attested["record"].clone()
    }

    /// `<noun> attestation check ID --kind plan-review [--bar BAR]`, held to its exit code: `0`
    /// exactly when it answers `valid`, `3` otherwise.
    pub(crate) fn check(&self, noun: &str, id: &str, bar: Option<&str>) -> Value {
        let mut arguments = vec![noun, "attestation", "check", id, "--kind", KIND];
        if let Some(bar) = bar {
            arguments.extend(["--bar", bar]);
        }
        let mut with_json = arguments.clone();
        with_json.push("--json");
        let output = self.run(&with_json);
        let checked: Value = serde_json::from_str(&stdout(&output))
            .unwrap_or_else(|error| panic!("a check answers JSON ({error}): {}", stderr(&output)));
        let expected = if checked["state"] == "valid" { 0 } else { 3 };
        assert_eq!(
            output.status.code(),
            Some(expected),
            "a check answering {checked} exits {expected}: {}",
            stderr(&output)
        );
        assert_eq!(checked["kind"], KIND);
        checked
    }

    /// Hold `noun ID`'s check to `state` and `reason`.
    pub(crate) fn holds(&self, noun: &str, id: &str, state: &str, reason: Value, why: &str) {
        let checked = self.check(noun, id, Some(BAR));
        assert_eq!(
            (&checked["state"], &checked["reason"]),
            (&json!(state), &reason),
            "{noun} {id}: {why}: {checked}"
        );
    }

    /// Record a fresh attestation on `noun ID`, check it valid, run `edit`, and hold the check
    /// afterwards to `state`/`reason`.
    fn after(&self, noun: &str, id: &str, edit: &[&str], state: &str, reason: Value, why: &str) {
        self.attest(noun, id);
        self.holds(noun, id, "valid", Value::Null, "freshly recorded");
        self.exits(edit, 0);
        self.holds(noun, id, state, reason, why);
    }

    /// Edit the file of the record `id` names under `directory` of the folder by hand,
    /// replacing the first `from` with `to`.
    fn hand_edit(&self, directory: &str, id: &str, from: &str, to: &str) {
        let native = id.split_once(':').expect("qualified").1;
        let file = self.notes.join(directory).join(format!("{native}.md"));
        let text = std::fs::read_to_string(&file).expect("the record's file");
        let edited = text.replacen(from, to, 1);
        assert_ne!(edited, text, "{from:?} is in {}", file.display());
        std::fs::write(&file, edited).expect("an edit");
    }

    /// The project, its three tasks and its design document, each rendered from a template
    /// but `measure`, in `source`; answering their ids.
    fn seed(&self, source: &str) -> Seeded {
        let (plan, task, design) = (
            self.template("plan.md"),
            self.template("task.md"),
            self.template("design.md"),
        );
        let project = stdout(&self.exits(
            &[
                "project",
                "create",
                source,
                "--id",
                "plan-1",
                "--title",
                "The plan",
                "--template",
                &plan,
                "--var",
                "goal=Ship it",
                "--no-interactive",
            ],
            0,
        ))
        .trim()
        .to_owned();
        let native = project.split_once(':').expect("qualified").1.to_owned();
        let create = |title: &str, extra: &[&str]| {
            let mut arguments = vec![
                "task",
                "create",
                source,
                "--project",
                &native,
                "--title",
                title,
                "--no-interactive",
            ];
            arguments.extend_from_slice(extra);
            stdout(&self.exits(&arguments, 0)).trim().to_owned()
        };
        let body = self.file("measure.md", "Measure it.\n");
        let build = create(
            "Build",
            &[
                "--template",
                &task,
                "--var",
                "goal=Build it",
                "--metadata",
                "myapp.size=1",
            ],
        );
        let measure = create("Measure", &["--body-file", &body]);
        let ship = create(
            "Ship",
            &[
                "--template",
                &task,
                "--var",
                "goal=Ship it",
                "--depends-on",
                &build,
                "--metadata",
                "myapp.size=1",
                "--metadata",
                "myapp.owner=\"ops\"",
            ],
        );
        let design = stdout(&self.exits(
            &[
                "document",
                "create",
                source,
                "--project",
                &native,
                "--id",
                "design",
                "--title",
                "Design",
                "--template",
                &design,
                "--var",
                "summary=Small",
                "--metadata",
                "myapp.size=1",
                "--no-interactive",
            ],
            0,
        ))
        .trim()
        .to_owned();
        Seeded {
            project,
            build,
            measure,
            ship,
            design,
        }
    }
}

/// The ids [`Plan::seed`] wrote.
struct Seeded {
    project: String,
    build: String,
    measure: String,
    ship: String,
    design: String,
}

fn path(path: &Path) -> String {
    path.to_str().expect("a UTF-8 path").to_owned()
}

#[test]
fn a_task_record_goes_void_exactly_when_its_authored_content_moves() {
    let plan = Plan::new(false);
    let seeded = plan.seed("notes");
    let ship = seeded.ship.as_str();
    let moved = json!("content-moved");
    let search = path(&plan.templates);
    let body = plan.file("body.md", "Goal: shipped by hand\n");

    // An absent kind is reported absent, exit 3, with no record.
    let absent = plan.check("task", ship, None);
    assert_eq!(
        (&absent["state"], &absent["reason"], &absent["record"]),
        (&json!("absent"), &Value::Null, &Value::Null)
    );

    let record = plan.attest("task", ship);
    assert_eq!(record["by"], "ada");
    assert_eq!(record["excludes"], json!([EXCLUDED]));
    let digest = record["content_digest"].as_str().expect("a digest");
    assert!(
        digest.starts_with("sha256:") && digest.len() == 71,
        "{digest}"
    );
    assert!(record["at"].as_str().is_some_and(|at| at.ends_with('Z')));
    let checked = plan.check("task", ship, Some(BAR));
    assert_eq!(checked["state"], "valid");
    assert_eq!(
        checked["record"], record,
        "the check answers the record held"
    );
    // Without --bar the bar is not checked; with another it is `bar-moved`.
    assert_eq!(plan.check("task", ship, None)["state"], "valid");
    let other = plan.check("task", ship, Some("bar-2"));
    assert_eq!(
        (&other["state"], &other["reason"]),
        (&json!("void"), &json!("bar-moved"))
    );

    plan.after(
        "task",
        ship,
        &["task", "content", "set", ship, "--file", &body],
        "void",
        moved.clone(),
        "a body edit",
    );
    plan.after(
        "task",
        ship,
        &["task", "update", ship, "--title", "Ship it now"],
        "void",
        moved.clone(),
        "a title edit",
    );
    plan.after(
        "task",
        ship,
        &[
            "task",
            "render",
            ship,
            "--search-path",
            &search,
            "--var",
            "goal=Ship it twice",
            "--no-interactive",
        ],
        "void",
        moved.clone(),
        "a template answer edit",
    );
    plan.after(
        "task",
        ship,
        &[
            "task",
            "update",
            ship,
            "--depends-on",
            &seeded.build,
            "--depends-on",
            &seeded.measure,
        ],
        "void",
        moved.clone(),
        "a dependency edge added",
    );
    plan.after(
        "task",
        ship,
        &["task", "metadata", "set", ship, "myapp.size", "2"],
        "void",
        moved.clone(),
        "a caller key edit",
    );
    plan.after(
        "task",
        ship,
        &["task", "metadata", "set", ship, EXCLUDED, "\"dev\""],
        "valid",
        Value::Null,
        "an excluded key edit",
    );
    plan.after(
        "task",
        ship,
        &["task", "status", "set", ship, "done"],
        "valid",
        Value::Null,
        "a status change",
    );

    // A repository edited by hand in the task's own file.
    plan.attest("task", ship);
    plan.hand_edit(
        "tasks",
        ship,
        "status:",
        "repositories:\n- github.com/acme/widgets\nstatus:",
    );
    plan.holds("task", ship, "void", moved, "a repository added");

    // A second kind is its own record: recording it leaves the first's as it was.
    let first = plan.attest("task", ship);
    plan.exits(
        &[
            "task",
            "attest",
            ship,
            "--kind",
            "design-approval",
            "--by",
            "bo",
            "--bar",
            "x",
            "--note",
            "looks right",
        ],
        0,
    );
    let both = plan.check("task", ship, Some(BAR));
    assert_eq!((&both["state"], &both["record"]), (&json!("valid"), &first));
    let shown = plan.json(&["task", "show", ship]);
    let held = &shown["items"][0]["item"]["metadata"]["onetaskgraph.attestations"];
    assert_eq!(held[KIND], first);
    assert_eq!(held["design-approval"]["note"], "looks right");
}

#[test]
fn a_project_and_a_document_record_go_void_exactly_when_their_authored_content_moves() {
    let plan = Plan::new(false);
    let seeded = plan.seed("notes");
    let moved = json!("content-moved");
    let search = path(&plan.templates);
    let template = plan.template("plan.md");
    let project = seeded.project.as_str();
    let doc = seeded.design.as_str();

    plan.after(
        "project",
        project,
        &[
            "project",
            "render",
            project,
            "--search-path",
            &search,
            "--var",
            "goal=Ship it twice",
            "--no-interactive",
        ],
        "void",
        moved.clone(),
        "a template answer edit, which moves the body",
    );
    plan.after(
        "project",
        project,
        &[
            "project",
            "create",
            "notes",
            "--id",
            "plan-1",
            "--title",
            "The renamed plan",
            "--template",
            &template,
            "--var",
            "goal=Ship it twice",
            "--no-interactive",
        ],
        "void",
        moved.clone(),
        "a title edit",
    );
    plan.after(
        "project",
        project,
        &[
            "project",
            "create",
            "notes",
            "--id",
            "plan-1",
            "--title",
            "The renamed plan",
            "--template",
            &template,
            "--var",
            "goal=Ship it twice",
            "--repository",
            "github.com/acme/widgets",
            "--no-interactive",
        ],
        "void",
        moved.clone(),
        "a repository added",
    );
    plan.after(
        "project",
        project,
        &["project", "metadata", "set", project, "myapp.size", "2"],
        "void",
        moved.clone(),
        "a caller key edit",
    );
    plan.after(
        "project",
        project,
        &["project", "status", "set", project, "in-progress"],
        "valid",
        Value::Null,
        "a status change",
    );
    plan.attest("project", project);
    assert_eq!(
        plan.check("project", project, Some("bar-2"))["reason"],
        "bar-moved"
    );

    plan.after(
        "document",
        doc,
        &[
            "document",
            "render",
            doc,
            "--search-path",
            &search,
            "--var",
            "summary=Larger",
            "--no-interactive",
        ],
        "void",
        moved.clone(),
        "a template answer edit",
    );
    // A document's title and repositories are edited by hand in its own file: `document
    // create --id` replaces a document whole, its records with it.
    plan.attest("document", doc);
    plan.hand_edit("documents", doc, "title: Design", "title: The design");
    plan.holds(
        "document",
        doc,
        "void",
        moved.clone(),
        "a title edited by hand",
    );
    plan.attest("document", doc);
    plan.hand_edit(
        "documents",
        doc,
        "metadata:",
        "repositories:\n- github.com/acme/widgets\nmetadata:",
    );
    plan.holds(
        "document",
        doc,
        "void",
        moved.clone(),
        "a repository added by hand",
    );
    plan.after(
        "document",
        doc,
        &["document", "metadata", "set", doc, "myapp.size", "2"],
        "void",
        moved,
        "a caller key edit",
    );
    plan.attest("document", doc);
    assert_eq!(
        plan.check("document", doc, Some("bar-2"))["reason"],
        "bar-moved"
    );

    // An excluded key on a document leaves its record standing.
    plan.exits(
        &[
            "document",
            "attest",
            doc,
            "--kind",
            KIND,
            "--by",
            "ada",
            "--bar",
            BAR,
            "--exclude",
            "myapp.size",
        ],
        0,
    );
    plan.exits(&["document", "metadata", "set", doc, "myapp.size", "3"], 0);
    plan.holds(
        "document",
        doc,
        "valid",
        Value::Null,
        "an excluded key edit",
    );
}

#[test]
fn every_task_of_a_project_is_checked_one_entry_each() {
    let plan = Plan::new(false);
    let seeded = plan.seed("notes");
    plan.attest("task", &seeded.build);
    plan.attest("task", &seeded.ship);
    let checked = plan.json_exiting(
        &[
            "project",
            "attestation",
            "check",
            &seeded.project,
            "--kind",
            KIND,
            "--tasks",
            "--bar",
            BAR,
        ],
        3,
    );
    assert_eq!(checked["project"], json!(seeded.project));
    let mut states: Vec<(String, String)> = checked["tasks"]
        .as_array()
        .expect("one entry per task")
        .iter()
        .map(|entry| {
            (
                entry["id"].as_str().unwrap().to_owned(),
                entry["state"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    states.sort();
    let mut expected = vec![
        (seeded.build.clone(), "valid".to_owned()),
        (seeded.measure.clone(), "absent".to_owned()),
        (seeded.ship.clone(), "valid".to_owned()),
    ];
    expected.sort();
    assert_eq!(states, expected);
    plan.attest("task", &seeded.measure);
    let all = plan.json(&[
        "project",
        "attestation",
        "check",
        &seeded.project,
        "--kind",
        KIND,
        "--tasks",
    ]);
    assert!(
        all["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .all(|entry| entry["state"] == "valid"),
        "{all}"
    );
}

#[test]
fn an_attestation_held_to_a_version_is_refused_at_another_and_bad_input_is_refused_by_flag() {
    let plan = Plan::new(false);
    let seeded = plan.seed("notes");
    let refused = plan.exits(
        &[
            "task",
            "attest",
            &seeded.ship,
            "--kind",
            KIND,
            "--by",
            "ada",
            "--bar",
            BAR,
            "--if-version",
            "not-the-version",
        ],
        1,
    );
    assert!(
        stderr(&refused).contains("not-the-version"),
        "{}",
        stderr(&refused)
    );
    assert_eq!(
        plan.check("task", &seeded.ship, None)["state"],
        "absent",
        "nothing was written"
    );
    for (flag, value) in [("--kind", "two words"), ("--by", ""), ("--bar", "")] {
        let mut arguments = vec![
            "task",
            "attest",
            &seeded.ship,
            "--kind",
            KIND,
            "--by",
            "ada",
            "--bar",
            BAR,
        ];
        let at = arguments
            .iter()
            .position(|argument| *argument == flag)
            .unwrap();
        arguments[at + 1] = value;
        let output = plan.exits(&arguments, 1);
        assert!(
            stderr(&output).contains(flag),
            "{flag}: {}",
            stderr(&output)
        );
    }
    let reserved = plan.exits(
        &[
            "task",
            "attest",
            &seeded.ship,
            "--kind",
            KIND,
            "--by",
            "ada",
            "--bar",
            BAR,
            "--exclude",
            "onetaskgraph.origin",
        ],
        1,
    );
    assert!(
        stderr(&reserved).contains("--exclude"),
        "{}",
        stderr(&reserved)
    );
    let unknown = plan.exits(
        &[
            "task",
            "attestation",
            "check",
            "notes:no-such-task",
            "--kind",
            KIND,
        ],
        1,
    );
    assert!(
        stderr(&unknown).contains("no-such-task"),
        "{}",
        stderr(&unknown)
    );
}

#[test]
fn records_survive_a_project_copy_with_its_documents_and_go_void_where_they_landed() {
    let plan = Plan::new(true);
    let seeded = plan.seed("notes");
    let records: Vec<(&str, &str, Value)> = [
        ("project", seeded.project.as_str()),
        ("task", seeded.build.as_str()),
        ("task", seeded.ship.as_str()),
        ("document", seeded.design.as_str()),
    ]
    .into_iter()
    .map(|(noun, id)| (noun, id, plan.attest(noun, id)))
    .collect();

    for destination in ["board", "linear"] {
        let copied = plan.json(&["project", "copy", &seeded.project, "--to", destination]);
        let landed = |source: &str| -> String {
            copied["items"]
                .as_array()
                .expect("outcomes")
                .iter()
                .find(|outcome| outcome["source"] == source)
                .and_then(|outcome| outcome["destination"].as_str())
                .unwrap_or_else(|| panic!("{source} landed in {destination}: {copied}"))
                .to_owned()
        };
        let documents = plan.json(&["document", "copy", &seeded.design, "--to", destination]);
        let design = documents["items"][0]["destination"]
            .as_str()
            .unwrap_or_else(|| panic!("the design landed in {destination}: {documents}"))
            .to_owned();
        for (noun, id, record) in &records {
            let copy = if *noun == "document" {
                design.clone()
            } else {
                landed(id)
            };
            let checked = plan.check(noun, &copy, Some(BAR));
            assert_eq!(
                (&checked["state"], &checked["record"]),
                (&json!("valid"), record),
                "{destination}: the copy of {id}, {copy}, holds the source's record and it holds"
            );
        }

        // Where they landed, the records go void as they do where they were written.
        let ship = landed(&seeded.ship);
        let build = landed(&seeded.build);
        let measure = landed(&seeded.measure);
        let moved = json!("content-moved");
        let search = path(&plan.templates);
        plan.after(
            "task",
            &ship,
            &["task", "update", &ship, "--title", "Ship it now"],
            "void",
            moved.clone(),
            &format!("{destination}: a title edit"),
        );
        plan.after(
            "task",
            &ship,
            &["task", "metadata", "set", &ship, "myapp.size", "5"],
            "void",
            moved.clone(),
            &format!("{destination}: a caller key edit"),
        );
        plan.after(
            "task",
            &ship,
            &["task", "metadata", "set", &ship, EXCLUDED, "\"dev\""],
            "valid",
            Value::Null,
            &format!("{destination}: an excluded key edit"),
        );
        plan.after(
            "task",
            &ship,
            &["task", "status", "set", &ship, "in-progress"],
            "valid",
            Value::Null,
            &format!("{destination}: a status change"),
        );
        let body = plan.file("hosted.md", "Goal: shipped on the host\n");
        plan.after(
            "task",
            &ship,
            &["task", "content", "set", &ship, "--file", &body],
            "void",
            moved.clone(),
            &format!("{destination}: a body edit"),
        );
        plan.after(
            "task",
            &ship,
            &[
                "task",
                "render",
                &ship,
                "--template",
                &plan.template("task.md"),
                "--search-path",
                &search,
                "--var",
                "goal=Ship it again",
                "--no-interactive",
            ],
            "void",
            moved.clone(),
            &format!("{destination}: a template answer edit"),
        );
        plan.after(
            "task",
            &ship,
            &[
                "task",
                "update",
                &ship,
                "--depends-on",
                &build,
                "--depends-on",
                &measure,
            ],
            "void",
            moved.clone(),
            &format!("{destination}: a dependency edge added"),
        );
        plan.attest("task", &ship);
        assert_eq!(
            plan.check("task", &ship, Some("bar-2"))["reason"],
            "bar-moved"
        );

        let project = landed(&seeded.project);
        plan.after(
            "project",
            &project,
            &["project", "metadata", "set", &project, "myapp.size", "5"],
            "void",
            moved.clone(),
            &format!("{destination}: a project's caller key edit"),
        );
        plan.after(
            "project",
            &project,
            &[
                "project",
                "render",
                &project,
                "--template",
                &plan.template("plan.md"),
                "--var",
                "goal=Again",
                "--no-interactive",
            ],
            "void",
            moved.clone(),
            &format!("{destination}: a project's template answer edit"),
        );
        plan.after(
            "document",
            &design,
            &["document", "metadata", "set", &design, "myapp.size", "5"],
            "void",
            moved.clone(),
            &format!("{destination}: a document's caller key edit"),
        );
        plan.after(
            "document",
            &design,
            &[
                "document",
                "render",
                &design,
                "--template",
                &plan.template("design.md"),
                "--var",
                "summary=Again",
                "--no-interactive",
            ],
            "void",
            moved,
            &format!("{destination}: a document's template answer edit"),
        );
        plan.attest("document", &design);
        assert_eq!(
            plan.check("document", &design, Some("bar-2"))["reason"],
            "bar-moved"
        );
    }
}

#[test]
fn an_attestation_held_to_the_version_the_item_is_at_is_recorded() {
    let plan = Plan::new(false);
    let seeded = plan.seed("notes");
    // A folder of Markdown gives a task a version and a document none; a board document's is
    // held in the GitHub Projects suite's approval journey.
    let (noun, id) = ("task", seeded.ship.as_str());
    let version = plan.json(&[noun, "show", id])["items"][0]["item"]["version"]
        .as_str()
        .unwrap_or_else(|| panic!("{noun} {id} reports a version"))
        .to_owned();
    let attested = plan.json(&[
        noun,
        "attest",
        id,
        "--kind",
        KIND,
        "--by",
        "ada",
        "--bar",
        BAR,
        "--if-version",
        &version,
    ]);
    assert_eq!(attested["record"]["kind"], KIND, "{noun}");
    assert_eq!(plan.check(noun, id, Some(BAR))["state"], "valid", "{noun}");
}

#[test]
fn an_edge_names_its_far_end_by_identity_wherever_it_is_never_by_title() {
    let plan = Plan::new(false);
    let other = plan.sandbox.subdirectory("other");
    plan.sandbox.project_document(&document(&json!({
        "notes": {"plugin": "local-md", "config": {"root": plan.notes}},
        "other": {"plugin": "local-md", "config": {"root": other}},
    })));
    let moved = json!("content-moved");
    let body = plan.file("far.md", "Far.\n");
    let create = |source: &str, title: &str, extra: &[&str]| {
        let mut arguments = vec![
            "task",
            "create",
            source,
            "--project",
            "elsewhere",
            "--title",
            title,
            "--body-file",
            &body,
        ];
        arguments.extend_from_slice(extra);
        stdout(&plan.exits(&arguments, 0)).trim().to_owned()
    };

    // A task depending on a task of another configured source.
    let far = create("other", "Upstream", &[]);
    let twin = create("other", "Upstream", &[]);
    let near = create("notes", "Near", &["--depends-on", &far]);
    // Renaming the far end leaves it the same item: the record holds.
    plan.after(
        "task",
        &near,
        &["task", "update", &far, "--title", "Upstream, renamed"],
        "valid",
        Value::Null,
        "the far end in another source renamed",
    );
    // Pointing the edge at another item of that source voids it, whatever the titles.
    plan.exits(&["task", "update", &far, "--title", "Upstream"], 0);
    plan.after(
        "task",
        &near,
        &["task", "update", &near, "--depends-on", &twin],
        "void",
        moved.clone(),
        "the edge moved to another item of the same title",
    );
    // A far end gone from its store is still named by the id the edge records: the same edge.
    plan.attest("task", &near);
    let native = twin.split_once(':').expect("qualified").1;
    std::fs::remove_file(other.join("tasks").join(format!("{native}.md")))
        .expect("a far end removed");
    plan.holds(
        "task",
        &near,
        "valid",
        Value::Null,
        "the far end removed, the edge unchanged",
    );

    // A project depending on a project.
    let template = plan.template("plan.md");
    for (id, title) in [
        ("upstream", "Upstream plan"),
        ("lookalike", "Upstream plan"),
        ("plan-1", "The plan"),
    ] {
        plan.exits(
            &[
                "project",
                "create",
                "notes",
                "--id",
                id,
                "--title",
                title,
                "--template",
                &template,
                "--var",
                "goal=g",
                "--no-interactive",
            ],
            0,
        );
    }
    plan.hand_edit(
        "projects",
        "notes:plan-1",
        "title: The plan",
        "title: The plan\ndepends_on: [upstream]",
    );
    let edges = plan.json(&["project", "deps", "notes:plan-1"])["items"].clone();
    assert_eq!(edges[0]["to"]["id"], "notes:upstream", "{edges}");
    plan.after(
        "project",
        "notes:plan-1",
        &[
            "project",
            "create",
            "notes",
            "--id",
            "upstream",
            "--title",
            "Upstream plan, renamed",
            "--template",
            &template,
            "--var",
            "goal=g",
            "--no-interactive",
        ],
        "valid",
        Value::Null,
        "the project it depends on renamed",
    );
    plan.attest("project", "notes:plan-1");
    plan.hand_edit(
        "projects",
        "notes:plan-1",
        "depends_on: [upstream]",
        "depends_on: [lookalike]",
    );
    plan.holds(
        "project",
        "notes:plan-1",
        "void",
        moved,
        "the edge moved to another project of the title it once had",
    );
}

#[test]
fn a_person_reads_each_answer_in_text_and_excludes_are_recorded_once_each_in_order() {
    let plan = Plan::new(false);
    let seeded = plan.seed("notes");
    let ship = seeded.ship.as_str();
    let said = stdout(&plan.exits(
        &[
            "task",
            "attest",
            ship,
            "--kind",
            KIND,
            "--by",
            "ada",
            "--bar",
            BAR,
            "--exclude",
            "myapp.zeta",
            "--exclude",
            "myapp.alpha",
            "--exclude",
            "myapp.zeta",
        ],
        0,
    ));
    assert!(
        said.starts_with(&format!("{ship}: {KIND} recorded by ada at "))
            && said.contains(&format!("against bar {BAR} (content sha256:")),
        "{said}"
    );
    let held = plan.check("task", ship, None);
    assert_eq!(
        held["record"]["excludes"],
        json!(["myapp.alpha", "myapp.zeta"])
    );

    let valid = plan.exits(&["task", "attestation", "check", ship, "--kind", KIND], 0);
    assert_eq!(stdout(&valid), format!("{KIND}: valid\n"));
    let moved = plan.exits(
        &[
            "task",
            "attestation",
            "check",
            ship,
            "--kind",
            KIND,
            "--bar",
            "bar-2",
        ],
        3,
    );
    assert_eq!(stdout(&moved), format!("{KIND}: void (bar-moved)\n"));
    let absent = plan.exits(
        &[
            "task",
            "attestation",
            "check",
            ship,
            "--kind",
            "never-recorded",
        ],
        3,
    );
    assert_eq!(stdout(&absent), "never-recorded: absent\n");
    let tasks = plan.exits(
        &[
            "project",
            "attestation",
            "check",
            &seeded.project,
            "--kind",
            KIND,
            "--tasks",
        ],
        3,
    );
    let lines: Vec<String> = stdout(&tasks).lines().map(str::to_owned).collect();
    assert!(
        lines.contains(&format!("{ship}\t{KIND}: valid"))
            && lines.contains(&format!("{}\t{KIND}: absent", seeded.measure)),
        "{lines:?}"
    );

    // A project with no task checks nothing, and that is no failure.
    let template = plan.template("plan.md");
    plan.exits(
        &[
            "project",
            "create",
            "notes",
            "--id",
            "empty",
            "--title",
            "Empty",
            "--template",
            &template,
            "--var",
            "goal=g",
            "--no-interactive",
        ],
        0,
    );
    let empty = plan.exits(
        &[
            "project",
            "attestation",
            "check",
            "notes:empty",
            "--kind",
            KIND,
            "--tasks",
        ],
        0,
    );
    assert_eq!(stdout(&empty), "notes:empty: no task to check\n");
    let json = plan.json(&[
        "project",
        "attestation",
        "check",
        "notes:empty",
        "--kind",
        KIND,
        "--tasks",
    ]);
    assert_eq!(json["tasks"], json!([]));
}

#[test]
fn a_write_answered_with_another_record_than_the_one_written_is_refused() {
    let plan = Plan::new(true);
    let seeded = plan.seed("notes");
    let copied = plan.json(&["project", "copy", &seeded.project, "--to", "linear"]);
    let ship = copied["items"]
        .as_array()
        .expect("outcomes")
        .iter()
        .find(|outcome| outcome["source"] == seeded.ship.as_str())
        .and_then(|outcome| outcome["destination"].as_str())
        .expect("the task landed")
        .to_owned();
    // Linear answers the metadata write with the record's digest changed: what it says it holds
    // is not what was written.
    plan.linear.as_ref().expect("the workspace").bend_answers(
        |query, _| query.contains("issueUpdate("),
        1,
        |_, body| {
            for answered in body
                .as_object_mut()
                .into_iter()
                .flat_map(|data| data.values_mut())
            {
                let held = &mut answered["issue"]["description"];
                let Some(text) = held.as_str().map(str::to_owned) else {
                    continue;
                };
                let marker = "\"content_digest\":\"sha256:";
                if let Some(at) = text.find(marker) {
                    let start = at + marker.len();
                    *held = json!(format!(
                        "{}{}{}",
                        &text[..start],
                        "0".repeat(64),
                        &text[start + 64..]
                    ));
                }
            }
        },
    );
    let refused = plan.exits(
        &[
            "task", "attest", &ship, "--kind", KIND, "--by", "ada", "--bar", BAR,
        ],
        1,
    );
    assert!(
        stderr(&refused).contains(&format!(
            "does not hold the {KIND} attestation it was just written"
        )),
        "{}",
        stderr(&refused)
    );
}

#[test]
fn a_far_end_in_a_source_nothing_configures_is_named_by_its_id() {
    let plan = Plan::new(false);
    let seeded = plan.seed("notes");
    let ship = seeded.measure.as_str();
    // An edge to an item of a source this configuration does not name: no source can read it,
    // so the digest names it by its id, and the record holds while that id does.
    plan.hand_edit(
        "tasks",
        ship,
        "status:",
        "depends_on:\n- id: elsewhere:far-1\n  kind: blocks\n  item: task\nstatus:",
    );
    plan.attest("task", ship);
    plan.holds(
        "task",
        ship,
        "valid",
        Value::Null,
        "an unread far end, named by its id",
    );
    plan.hand_edit("tasks", ship, "elsewhere:far-1", "elsewhere:far-2");
    plan.holds(
        "task",
        ship,
        "void",
        json!("content-moved"),
        "the far end's id moved",
    );
}

#[test]
fn a_stored_record_this_product_cannot_read_is_absent_and_another_kind_still_checks() {
    let plan = Plan::new(false);
    let seeded = plan.seed("notes");
    let measure = seeded.measure.as_str();
    plan.attest("task", measure);
    // A person edits the entry: one kind's record loses its digest, and another kind's is filed
    // under the wrong key.
    let native = measure.split_once(':').expect("qualified").1;
    let file = plan.notes.join("tasks").join(format!("{native}.md"));
    let text = std::fs::read_to_string(&file).expect("the task file");
    let start = text
        .find("\"onetaskgraph.attestations\": ")
        .expect("the entry");
    let line_end = start + text[start..].find('\n').expect("one line");
    let edited = format!(
        "{}\"onetaskgraph.attestations\": {}{}",
        &text[..start],
        json!({
            "broken": {"kind": "broken", "by": "ada"},
            "misfiled": {"kind": "other", "by": "ada", "at": "2026-11-10T00:00:00Z",
                         "content_digest": format!("sha256:{}", "0".repeat(64)), "bar": "b"},
            KIND: serde_json::from_str::<Value>(
                &text[start + "\"onetaskgraph.attestations\": ".len()..line_end]
            )
            .expect("the entry is JSON")[KIND],
        }),
        &text[line_end..]
    );
    std::fs::write(&file, edited).expect("an edit");
    for kind in ["broken", "misfiled"] {
        let output = plan.exits(
            &["task", "attestation", "check", measure, "--kind", kind],
            3,
        );
        assert_eq!(stdout(&output), format!("{kind}: absent\n"));
    }
    plan.holds(
        "task",
        measure,
        "valid",
        Value::Null,
        "a readable kind beside them",
    );

    // Recording a kind — replacing one held and adding another — writes that kind alone: the
    // entries this product cannot read stay in the file exactly as the person left them.
    let unreadable = |text: &str| -> (Value, Value) {
        let start = text
            .find("\"onetaskgraph.attestations\": ")
            .expect("the entry")
            + "\"onetaskgraph.attestations\": ".len();
        let line_end = start + text[start..].find('\n').expect("one line");
        let entry: Value = serde_json::from_str(&text[start..line_end]).expect("JSON");
        (entry["broken"].clone(), entry["misfiled"].clone())
    };
    let before = unreadable(&std::fs::read_to_string(&file).expect("the task file"));
    plan.attest("task", measure);
    plan.json(&[
        "task",
        "attest",
        measure,
        "--kind",
        "design-approval",
        "--by",
        "ada",
        "--bar",
        BAR,
    ]);
    let after = std::fs::read_to_string(&file).expect("the task file");
    assert_eq!(
        unreadable(&after),
        before,
        "the unreadable entries are kept as stored"
    );
    for kind in ["broken", "misfiled"] {
        plan.exits(
            &["task", "attestation", "check", measure, "--kind", kind],
            3,
        );
    }
    for kind in [KIND, "design-approval"] {
        let checked = plan.json(&[
            "task",
            "attestation",
            "check",
            measure,
            "--kind",
            kind,
            "--bar",
            BAR,
        ]);
        assert_eq!(checked["state"], "valid", "{kind}: {checked}");
    }
}

#[test]
fn a_project_attestation_held_to_a_version_lands_at_it_and_is_refused_at_another() {
    // A folder of Markdown gives a project no version; a board and Linear each give one.
    let plan = Plan::new(true);
    let seeded = plan.seed("notes");
    for destination in ["board", "linear"] {
        let copied = plan.json(&["project", "copy", &seeded.project, "--to", destination]);
        let project = copied["items"]
            .as_array()
            .expect("outcomes")
            .iter()
            .find(|outcome| outcome["source"] == seeded.project.as_str())
            .and_then(|outcome| outcome["destination"].as_str())
            .unwrap_or_else(|| panic!("the project landed in {destination}: {copied}"))
            .to_owned();
        let version = plan.json(&["project", "show", &project])["items"][0]["item"]["version"]
            .as_str()
            .unwrap_or_else(|| panic!("{destination}: project {project} reports a version"))
            .to_owned();
        let attest = |at: &str| {
            vec![
                "project".to_owned(),
                "attest".to_owned(),
                project.clone(),
                "--kind".to_owned(),
                KIND.to_owned(),
                "--by".to_owned(),
                "ada".to_owned(),
                "--bar".to_owned(),
                BAR.to_owned(),
                "--if-version".to_owned(),
                at.to_owned(),
            ]
        };
        let stale = attest("not-the-version");
        let stale: Vec<&str> = stale.iter().map(String::as_str).collect();
        let refused = plan.exits(&stale, 1);
        assert!(
            stderr(&refused).contains("not-the-version"),
            "{destination}: {}",
            stderr(&refused)
        );
        assert_eq!(
            plan.json(&["project", "show", &project])["items"][0]["item"]["version"],
            json!(version),
            "{destination}: the refusal wrote nothing"
        );
        assert_eq!(
            plan.check("project", &project, None)["state"],
            "absent",
            "{destination}: the refusal recorded nothing"
        );
        let held = attest(&version);
        let held: Vec<&str> = held.iter().map(String::as_str).collect();
        let mut held = held;
        held.push("--json");
        let attested: Value =
            serde_json::from_str(&stdout(&plan.exits(&held, 0))).expect("the answer is JSON");
        assert_eq!(
            attested["record"]["kind"], KIND,
            "{destination}: {attested}"
        );
        assert_eq!(
            plan.check("project", &project, Some(BAR))["state"],
            "valid",
            "{destination}"
        );
    }
}

#[test]
fn every_label_an_attestation_refuses_is_refused_by_its_flag_writing_nothing() {
    let plan = Plan::new(false);
    let seeded = plan.seed("notes");
    let ship = seeded.ship.as_str();
    let long = "x".repeat(201);
    for (flag, value) in [
        ("--kind", long.as_str()),
        ("--by", "two\nlines"),
        ("--by", long.as_str()),
        ("--bar", long.as_str()),
        ("--bar", "two\nlines"),
        ("--note", "   "),
    ] {
        let mut arguments = vec![ship, "--kind", KIND, "--by", "ada", "--bar", BAR];
        match arguments.iter().position(|argument| *argument == flag) {
            Some(at) => arguments[at + 1] = value,
            None => arguments.extend([flag, value]),
        }
        let mut full = vec!["task", "attest"];
        full.extend(arguments);
        let refused = plan.exits(&full, 1);
        assert!(
            stderr(&refused).contains(flag),
            "{flag}: {}",
            stderr(&refused)
        );
        assert_eq!(
            plan.check("task", ship, None)["state"],
            "absent",
            "{flag}: nothing written"
        );
    }
    let refused = plan.exits(
        &[
            "task",
            "attestation",
            "check",
            ship,
            "--kind",
            KIND,
            "--bar",
            " ",
        ],
        1,
    );
    assert!(stderr(&refused).contains("--bar"), "{}", stderr(&refused));
}

#[test]
fn an_opaque_far_end_no_configured_tracker_is_is_named_by_its_tracker_and_id() {
    let plan = Plan::new(false);
    let seeded = plan.seed("notes");
    let measure = seeded.measure.as_str();
    // A far end named by a tracker's designator, as a copy between trackers records one, that
    // no configured source is.
    let far = format!("linear/{}", "ab".repeat(32));
    plan.hand_edit(
        "tasks",
        measure,
        "status:",
        &format!("depends_on:\n  - {{designator: {far}, id: L-9, item: task}}\nstatus:"),
    );
    plan.attest("task", measure);
    plan.holds(
        "task",
        measure,
        "valid",
        Value::Null,
        "an opaque far end, named by its id",
    );
    plan.hand_edit("tasks", measure, "id: L-9", "id: L-10");
    plan.holds(
        "task",
        measure,
        "void",
        json!("content-moved"),
        "the opaque far end moved",
    );
    // The same id in another tracker is another item.
    plan.attest("task", measure);
    plan.hand_edit(
        "tasks",
        measure,
        &far,
        &format!("linear/{}", "cd".repeat(32)),
    );
    plan.holds(
        "task",
        measure,
        "void",
        json!("content-moved"),
        "the opaque far end's tracker moved",
    );
}

#[test]
fn a_dependency_swapped_for_another_task_of_the_same_title_voids_the_record() {
    let plan = Plan::new(false);
    let body = plan.file("same.md", "Same.\n");
    let create = |title: &str, extra: &[&str]| {
        let mut arguments = vec![
            "task",
            "create",
            "notes",
            "--project",
            "plan-1",
            "--title",
            title,
            "--body-file",
            &body,
        ];
        arguments.extend_from_slice(extra);
        stdout(&plan.exits(&arguments, 0)).trim().to_owned()
    };
    let first = create("Same title", &[]);
    let second = create("Same title", &[]);
    assert_ne!(first, second, "two tasks under one title");
    let near = create("Near", &["--depends-on", &first]);
    plan.after(
        "task",
        &near,
        &["task", "update", &near, "--depends-on", &second],
        "void",
        json!("content-moved"),
        "the dependency is now another task that happens to share the first's title",
    );
}

#[test]
fn a_task_record_with_edges_survives_two_hops_of_copying() {
    let plan = Plan::new(true);
    let seeded = plan.seed("notes");
    let record = plan.attest("task", &seeded.ship);
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
    // notes → board, then the board's copy → Linear: every far end's origin chain leads back
    // to the folder's own task.
    let first = plan.json(&["project", "copy", &seeded.project, "--to", "board"]);
    let on_board = landed(&first, &seeded.project);
    let second = plan.json(&["project", "copy", &on_board, "--to", "linear"]);
    let ship = landed(&second, &landed(&first, &seeded.ship));
    let checked = plan.check("task", &ship, Some(BAR));
    assert_eq!(
        (&checked["state"], &checked["record"]),
        (&json!("valid"), &record),
        "two hops from where it was recorded, the record still holds"
    );
}

#[test]
fn deleting_a_copy_between_a_far_end_and_its_root_leaves_the_record_valid() {
    // notes → middle → last, three folders of Markdown. Ship depends on Build at every hop, and
    // the record recorded in `notes` travels to `last` with the copies. The copy of Build in
    // `middle` is then deleted, as a person deletes a file: the copy in `last` names Build's
    // root itself, so the edge — and every record over it — reads as it did.
    let plan = Plan::new(false);
    let middle = plan.sandbox.subdirectory("middle");
    let last = plan.sandbox.subdirectory("last");
    plan.sandbox.project_document(&document(&json!({
        "notes": {"plugin": "local-md", "config": {"root": plan.notes}},
        "middle": {"plugin": "local-md", "config": {"root": middle}},
        "last": {"plugin": "local-md", "config": {"root": last}},
    })));
    let seeded = plan.seed("notes");
    let record = plan.attest("task", &seeded.ship);
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
    let first = plan.json(&["project", "copy", &seeded.project, "--to", "middle"]);
    let second = plan.json(&[
        "project",
        "copy",
        &landed(&first, &seeded.project),
        "--to",
        "last",
    ]);
    let build_in_middle = landed(&first, &seeded.build);
    let build_in_last = landed(&second, &build_in_middle);
    let ship_in_last = landed(&second, &landed(&first, &seeded.ship));
    let metadata =
        |id: &str| plan.json(&["task", "show", id])["items"][0]["item"]["metadata"].clone();
    assert_eq!(
        (
            &metadata(&build_in_last)["onetaskgraph.origin"],
            &metadata(&build_in_last)["onetaskgraph.lineage"],
        ),
        (&json!(build_in_middle), &json!(seeded.build)),
        "the second hop records where it came from and the root that came from"
    );
    assert_eq!(
        metadata(&build_in_middle).get("onetaskgraph.lineage"),
        None,
        "the first hop's origin is its root, so it records no lineage beside it"
    );
    plan.holds(
        "task",
        &ship_in_last,
        "valid",
        Value::Null,
        "two hops from where it was recorded",
    );
    let copied_here = {
        let checked = plan.check("task", &ship_in_last, Some(BAR));
        assert_eq!(checked["record"], record, "the record the copies carried");
        checked
    };

    let native = build_in_middle.split_once(':').expect("qualified").1;
    let file = middle.join("tasks").join(format!("{native}.md"));
    std::fs::remove_file(&file).unwrap_or_else(|error| panic!("{}: {error}", file.display()));
    plan.exits(&["task", "show", &build_in_middle], 1);

    let after = plan.check("task", &ship_in_last, Some(BAR));
    assert_eq!(
        after, copied_here,
        "with the copy between it and its root deleted, the record reads exactly as it did"
    );
    plan.holds(
        "task",
        &seeded.ship,
        "valid",
        Value::Null,
        "the record where it was recorded",
    );
    // And a record made where the chain is broken agrees with one made where it is whole.
    let recorded_after = plan.attest("task", &ship_in_last);
    assert_eq!(
        recorded_after["content_digest"], record["content_digest"],
        "the content Ship's copy holds names Build's root as Ship does in `notes`"
    );
}

#[test]
fn a_copy_hands_on_the_root_its_source_records_as_the_source_holds_it() {
    // Every branch of what a copy records at `onetaskgraph.lineage`, through the binary over
    // three folders of Markdown whose task files are written by hand.
    let plan = Plan::new(false);
    let middle = plan.sandbox.subdirectory("middle");
    let last = plan.sandbox.subdirectory("last");
    plan.sandbox.project_document(&document(&json!({
        "notes": {"plugin": "local-md", "config": {"root": plan.notes}},
        "middle": {"plugin": "local-md", "config": {"root": middle}},
        "last": {"plugin": "local-md", "config": {"root": last}},
    })));
    let tasks = plan.notes.join("tasks");
    std::fs::create_dir_all(&tasks).expect("the tasks folder");
    let write = |id: &str, front: &str| {
        std::fs::write(
            tasks.join(format!("{id}.md")),
            format!("---\ntitle: {id}\nstatus: todo\n{front}---\nBody.\n"),
        )
        .expect("a task file");
    };
    write(
        "rooted",
        "metadata:\n  onetaskgraph.lineage: root:R-1\n  onetaskgraph.origin: elsewhere:E-1\n",
    );
    write(
        "own-root",
        "metadata:\n  onetaskgraph.lineage: notes:own-root\n",
    );
    write(
        "garbled-root",
        "metadata:\n  onetaskgraph.lineage: not a qualified id\n  onetaskgraph.origin: notes:x\n",
    );
    write(
        "on-garbled",
        "depends_on:\n- id: garbled-root\n  kind: blocks\n  item: task\n",
    );
    write("plain", "");
    let copy = |ids: &[&str], to: &str| -> Value {
        let mut arguments = vec!["task", "copy"];
        arguments.extend_from_slice(ids);
        arguments.extend(["--to", to]);
        plan.json(&arguments)
    };
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
    let provenance = |id: &str| {
        let metadata = plan.json(&["task", "show", id])["items"][0]["item"]["metadata"].clone();
        (
            metadata.get("onetaskgraph.origin").cloned(),
            metadata.get("onetaskgraph.lineage").cloned(),
        )
    };

    // A lineage the source records outranks the origin beside it, and survives another hop.
    let rooted = landed(&copy(&["notes:rooted"], "middle"), "notes:rooted");
    assert_eq!(
        provenance(&rooted),
        (Some(json!("notes:rooted")), Some(json!("root:R-1"))),
        "the source's lineage, not its origin, is the root handed on"
    );
    let onward = landed(&copy(&[&rooted], "last"), &rooted);
    assert_eq!(
        provenance(&onward),
        (Some(json!(rooted)), Some(json!("root:R-1"))),
        "and the next hop hands the same root on"
    );

    // A source naming itself as its root is the origin its copy records: nothing beside it.
    let own = landed(&copy(&["notes:own-root"], "middle"), "notes:own-root");
    assert_eq!(
        provenance(&own),
        (Some(json!("notes:own-root")), None),
        "a lineage equal to the copied item's own id is not recorded"
    );

    // A lineage that is no qualified id is carried as it is held, never replaced by the origin
    // beside it, so the far end is exactly as refusable where it lands as where it was written.
    let report = copy(&["notes:on-garbled", "notes:garbled-root"], "middle");
    let garbled = landed(&report, "notes:garbled-root");
    assert_eq!(
        provenance(&garbled),
        (
            Some(json!("notes:garbled-root")),
            Some(json!("not a qualified id"))
        ),
        "the malformed root is carried verbatim"
    );
    for near in ["notes:on-garbled", &landed(&report, "notes:on-garbled")] {
        let refused = plan.exits(
            &[
                "task", "attest", near, "--kind", KIND, "--by", "ada", "--bar", BAR,
            ],
            1,
        );
        assert!(
            stderr(&refused).contains("records the onetaskgraph.lineage \"not a qualified id\""),
            "{near}: {}",
            stderr(&refused)
        );
    }

    // A copy back keeps the destination's own origin, and its own lineage with it.
    let copied = landed(&copy(&["notes:plain"], "middle"), "notes:plain");
    write("plain", "metadata:\n  onetaskgraph.lineage: kept:K-1\n");
    let back = copy(&[&copied], "notes");
    assert_eq!(
        landed(&back, &copied),
        "notes:plain",
        "the copy back: {back}"
    );
    assert_eq!(
        provenance("notes:plain"),
        (None, Some(json!("kept:K-1"))),
        "the original keeps its own provenance whole"
    );
}

#[test]
fn a_far_end_whose_lineage_cannot_be_named_refuses_the_attestation_writing_nothing() {
    let plan = Plan::new(false);
    plan.sandbox.project_document(&document(&json!({
        "notes": {"plugin": "local-md", "config": {"root": plan.notes}},
        "broken": {"plugin": "local-md", "config": {"root": "/nonexistent/onetaskgraph/root"}},
    })));
    let tasks = plan.notes.join("tasks");
    std::fs::create_dir_all(&tasks).expect("the tasks folder");
    let write = |id: &str, front: &str| {
        std::fs::write(
            tasks.join(format!("{id}.md")),
            format!("---\ntitle: {id}\nstatus: todo\n{front}---\nBody.\n"),
        )
        .expect("a task file");
    };
    // A task recording a lineage that is not a qualified id, beside an origin that is: the
    // lineage is what names the root, so the origin does not stand in for it.
    write(
        "garbled-root",
        "metadata:\n  onetaskgraph.lineage: not a qualified id\n  onetaskgraph.origin: notes:x\n",
    );
    // A task recording an origin that is not a qualified id.
    write(
        "garbled",
        "metadata:\n  onetaskgraph.origin: not a qualified id\n",
    );
    let near = |name: &str, far: &str| {
        write(
            name,
            &format!("depends_on:\n- id: {far}\n  kind: blocks\n  item: task\n"),
        );
        format!("notes:{name}")
    };
    for (near, says) in [
        (
            near("on-garbled-root", "garbled-root"),
            "records the onetaskgraph.lineage \"not a qualified id\"",
        ),
        (near("on-garbled", "garbled"), "which is not a qualified id"),
        (near("on-broken", "broken:x"), "broken"),
    ] {
        let before = std::fs::read_to_string(
            tasks.join(format!("{}.md", near.split_once(':').expect("qualified").1)),
        )
        .expect("the near task");
        let refused = plan.exits(
            &[
                "task", "attest", &near, "--kind", KIND, "--by", "ada", "--bar", BAR,
            ],
            1,
        );
        assert!(
            stderr(&refused).contains(says),
            "{near}: {}",
            stderr(&refused)
        );
        let after = std::fs::read_to_string(
            tasks.join(format!("{}.md", near.split_once(':').expect("qualified").1)),
        )
        .expect("the near task");
        assert_eq!(after, before, "{near}: nothing written");
        let check = plan.exits(&["task", "attestation", "check", &near, "--kind", KIND], 1);
        assert!(stderr(&check).contains(says), "{near}: {}", stderr(&check));
    }
}

#[test]
fn a_document_check_on_a_source_with_no_documents_is_refused_naming_it() {
    let plan = Plan::new(false);
    plan.sandbox.project_document(&document(&json!({
        "notes": {"plugin": "local-md", "config": {"root": plan.notes}},
        "memo": {"plugin": "in-memory", "config": {"capabilities": {"documents": "unsupported"}}},
    })));
    for arguments in [
        vec![
            "document",
            "attestation",
            "check",
            "memo:D-1",
            "--kind",
            KIND,
        ],
        vec![
            "document", "attest", "memo:D-1", "--kind", KIND, "--by", "ada", "--bar", BAR,
        ],
    ] {
        let refused = plan.exits(&arguments, 1);
        assert!(stderr(&refused).contains("memo"), "{}", stderr(&refused));
        assert!(
            stderr(&refused).contains("document"),
            "{}",
            stderr(&refused)
        );
    }
}

/// `project attestation check PROJECT --kind plan-review --tasks --bar bar-1`: each task's
/// state and reason, by id, sorted.
fn task_states(plan: &Plan, project: &str) -> Vec<(String, Value, Value)> {
    let arguments = [
        "project",
        "attestation",
        "check",
        project,
        "--kind",
        KIND,
        "--tasks",
        "--bar",
        BAR,
        "--json",
    ];
    let output = plan.run(&arguments);
    let checked: Value = serde_json::from_str(&stdout(&output)).unwrap_or_else(|error| {
        panic!(
            "a project check answers JSON ({error}): {}",
            stderr(&output)
        )
    });
    // It exits 0 exactly when every task's record is `valid`, and 3 otherwise.
    let all_valid = checked["tasks"]
        .as_array()
        .is_some_and(|tasks| tasks.iter().all(|task| task["state"] == "valid"));
    assert_eq!(
        output.status.code(),
        Some(if all_valid { 0 } else { 3 }),
        "a project check answering {checked}: {}",
        stderr(&output)
    );
    let mut states: Vec<(String, Value, Value)> = checked["tasks"]
        .as_array()
        .unwrap_or_else(|| panic!("one entry per task: {checked}"))
        .iter()
        .map(|entry| {
            (
                entry["id"].as_str().expect("an id").to_owned(),
                entry["state"].clone(),
                entry["reason"].clone(),
            )
        })
        .collect();
    states.sort_by(|left, right| left.0.cmp(&right.0));
    states
}

/// `states`, in the order [`task_states`] answers them in.
fn sorted(mut states: Vec<(String, Value, Value)>) -> Vec<(String, Value, Value)> {
    states.sort_by(|left, right| left.0.cmp(&right.0));
    states
}

/// A `valid` entry of [`task_states`].
fn valid(id: &str) -> (String, Value, Value) {
    (id.to_owned(), json!("valid"), Value::Null)
}

/// Where the outcome copying `source` landed, in a copy's report.
fn landed(report: &Value, source: &str) -> String {
    report["items"]
        .as_array()
        .expect("outcomes")
        .iter()
        .find(|outcome| outcome["source"] == source)
        .and_then(|outcome| outcome["destination"].as_str())
        .unwrap_or_else(|| panic!("{source} landed: {report}"))
        .to_owned()
}

/// `project copy FROM --to TO --documents --no-interactive`, answering its report.
fn copy_plan(plan: &Plan, from: &str, to: &str) -> Value {
    plan.json(&[
        "project",
        "copy",
        from,
        "--to",
        to,
        "--documents",
        "--no-interactive",
    ])
}

/// The planning lifecycle a host configures this store with: `source` (the folder of
/// Markdown under [`Plan`]'s `notes`) declared private, where plans are authored and reviewed;
/// two loopback boards `plans` and `shipped`, a folder `published` and a loopback Linear
/// `linear`, each declared public — so a copy out of `source` records no origin naming it on
/// any of them — and a `write_policy` whose check passes every write.
fn private_authoring(source: &str) -> Plan {
    let mut plan = Plan::new(false);
    let (board, _) = github_projects_with_board(&plan.sandbox);
    let (second, _) = github_projects_with_board(&plan.sandbox);
    let (linear, held) = linear_workspace(
        &plan.sandbox,
        json!({"tasks": [], "projects": [], "documents": [], "labels": [],
               "task_dependencies": [], "project_dependencies": []}),
    );
    plan.linear = Some(held);
    let published = plan.sandbox.subdirectory("published");
    let mut configured: Value = serde_json::from_str(&document(&json!({
        source: {"plugin": "local-md", "visibility": "private", "config": {"root": plan.notes}},
        "plans": {"plugin": "github-projects", "visibility": "public", "config": board},
        "shipped": {"plugin": "github-projects", "visibility": "public", "config": second},
        "published": {"plugin": "local-md", "visibility": "public",
                      "config": {"root": published}},
        "linear": {"plugin": "linear", "visibility": "public", "config": linear},
    })))
    .expect("a document");
    configured["write_policy"] = json!({
        "check_command": [
            interpreter(), "-c", "import sys; sys.stdin.read(); print('{\"verdict\": \"pass\"}')"
        ],
        "visibility_command": [
            interpreter(), "-c",
            "import sys; sys.stdin.read(); print('{\"visibility\": \"public\"}')"
        ],
    });
    plan.sandbox
        .project_document(&serde_json::to_string_pretty(&configured).expect("a document"));
    plan.sandbox
        .secrets_file("GITHUB_PROJECTS_FIXTURE_TOKEN=test-token\nLINEAR_API_KEY=fixture-key\n");
    plan
}

#[test]
fn a_reviewed_plan_copied_onto_boards_keeps_every_review_valid_until_its_content_moves() {
    // Two tasks reviewed where they are authored, B depending on A, and C beside them; the
    // project copied with its documents onto a board and checked there; copied on from that
    // board onto a second, which gives A a second new identity; copied onto a folder and into
    // Linear; then B's body edited and the project copied over the board's items again.
    let plan = private_authoring("authoring");
    let project = "authoring:launch";
    plan.exits(
        &[
            "project",
            "create",
            "authoring",
            "--id",
            "launch",
            "--title",
            "Launch",
        ],
        0,
    );
    let create = |title: &str, text: &str, extra: &[&str]| -> String {
        let file = plan.file(&format!("{title}.md"), text);
        let mut arguments = vec![
            "task",
            "create",
            "authoring",
            "--project",
            "launch",
            "--title",
            title,
            "--body-file",
            &file,
            "--no-interactive",
        ];
        arguments.extend_from_slice(extra);
        stdout(&plan.exits(&arguments, 0)).trim().to_owned()
    };
    let a = create("A", "Do A.\n", &[]);
    let b = create("B", "Do B after A.\n", &["--depends-on", &a]);
    let c = create("C", "Do C.\n", &[]);
    plan.attest("task", &a);
    plan.attest("task", &b);
    let reviewed = |ids: [&str; 3]| {
        sorted(vec![
            valid(ids[0]),
            valid(ids[1]),
            (ids[2].to_owned(), json!("absent"), Value::Null),
        ])
    };
    assert_eq!(
        task_states(&plan, project),
        reviewed([&a, &b, &c]),
        "where they were reviewed"
    );

    let first = copy_plan(&plan, project, "plans");
    let on_board = landed(&first, project);
    let (board_a, board_b, board_c) = (landed(&first, &a), landed(&first, &b), landed(&first, &c));
    assert_eq!(
        task_states(&plan, &on_board),
        reviewed([&board_a, &board_b, &board_c]),
        "on the board, B's copy — whose edge names A's copy — holds its review"
    );

    // Copied on from the board, A's copy gets a second new identity: B's edge still names A.
    let chained = copy_plan(&plan, &on_board, "shipped");
    let shipped = landed(&chained, &on_board);
    let (shipped_b, shipped_c) = (landed(&chained, &board_b), landed(&chained, &board_c));
    assert_eq!(
        task_states(&plan, &shipped),
        reviewed([&landed(&chained, &board_a), &shipped_b, &shipped_c]),
        "two copies from where it was reviewed"
    );

    // A folder and Linear, each declared public, hold the reviews the same way.
    for destination in ["published", "linear"] {
        let copied = copy_plan(&plan, project, destination);
        assert_eq!(
            task_states(&plan, &landed(&copied, project)),
            reviewed([
                &landed(&copied, &a),
                &landed(&copied, &b),
                &landed(&copied, &c)
            ]),
            "{destination}: B's copy holds its review"
        );
    }

    // On a copy, an edge added, removed or pointed at another item still voids the review.
    let moved = json!("content-moved");
    plan.after(
        "task",
        &shipped_b,
        &["task", "update", &shipped_b, "--depends-on", &shipped_c],
        "void",
        moved.clone(),
        "the copy's edge pointed at another item",
    );
    plan.after(
        "task",
        &shipped_b,
        &["task", "update", &shipped_b, "--no-depends-on"],
        "void",
        moved.clone(),
        "the copy's edge removed",
    );
    plan.after(
        "task",
        &shipped_b,
        &["task", "update", &shipped_b, "--depends-on", &shipped_c],
        "void",
        moved.clone(),
        "an edge added to the copy",
    );

    // B's body edited where it is authored, and the project copied again over the board's
    // items: B's review is void there, A's still holds.
    let edited = plan.file("b-edited.md", "Do B after A, twice.\n");
    plan.exits(&["task", "content", "set", &b, "--file", &edited], 0);
    let again = copy_plan(&plan, project, "plans");
    assert_eq!(landed(&again, &a), board_a, "the re-copy updates A's copy");
    assert_eq!(landed(&again, &b), board_b, "the re-copy updates B's copy");
    assert_eq!(
        task_states(&plan, &on_board),
        sorted(vec![
            valid(&board_a),
            (board_b.clone(), json!("void"), moved.clone()),
            (board_c.clone(), json!("absent"), Value::Null),
        ]),
        "B's body moved and A's did not"
    );

    // Where they are authored, an edge added, removed or pointed elsewhere voids it too.
    plan.after(
        "task",
        &b,
        &["task", "update", &b, "--depends-on", &c],
        "void",
        moved.clone(),
        "the edge pointed at another item",
    );
    plan.after(
        "task",
        &b,
        &["task", "update", &b, "--no-depends-on"],
        "void",
        moved.clone(),
        "the edge removed",
    );
    plan.after(
        "task",
        &b,
        &["task", "update", &b, "--depends-on", &a, "--depends-on", &c],
        "void",
        moved,
        "an edge added",
    );
}

#[test]
fn a_record_the_previous_release_wrote_reads_valid_and_travels_through_a_copy() {
    // `notes:a` and `notes:b`, B depending on A, each attested by the released v0.14.14 binary
    // (fixtures/attestations/v0.14.14). B's edge names a far end no copy made, so its digest is
    // the one that release computed — with `notes` undeclared, and declared private.
    let placed = |plan: &Plan| {
        for (directory, name, text) in [
            (
                "projects",
                "launch.md",
                include_str!("fixtures/attestations/v0.14.14/launch.md"),
            ),
            (
                "tasks",
                "a.md",
                include_str!("fixtures/attestations/v0.14.14/a.md"),
            ),
            (
                "tasks",
                "b.md",
                include_str!("fixtures/attestations/v0.14.14/b.md"),
            ),
        ] {
            let held = plan.notes.join(directory);
            std::fs::create_dir_all(&held).expect("the folder's directory");
            std::fs::write(held.join(name), text).expect("a recorded file");
        }
    };
    let plain = Plan::new(false);
    placed(&plain);
    assert_eq!(
        task_states(&plain, "notes:launch"),
        sorted(vec![valid("notes:a"), valid("notes:b")]),
        "the previous release's records, read by this one"
    );

    let plan = private_authoring("notes");
    placed(&plan);
    assert_eq!(
        task_states(&plan, "notes:launch"),
        sorted(vec![valid("notes:a"), valid("notes:b")]),
        "the previous release's records, with their source declared private"
    );
    let copied = copy_plan(&plan, "notes:launch", "plans");
    assert_eq!(
        task_states(&plan, &landed(&copied, "notes:launch")),
        sorted(vec![
            valid(&landed(&copied, "notes:a")),
            valid(&landed(&copied, "notes:b"))
        ]),
        "copied onto a board, the previous release's records hold"
    );
}

#[test]
fn the_item_a_private_copy_was_made_from_is_found_only_when_its_link_says_so_unambiguously() {
    // Every answer the private side's correspondence gives, through the binary over folders of
    // Markdown: `authoring` and `vault` declared private, beside a private Linear workspace that
    // answers every request rate limited and one whose credential is absent; `published` and
    // `onward` declared public. Authoring's `a` records `vault:root-a` as its root, and `b`
    // depends on it; project `downstream` depends on project `upstream`, listed after sixty
    // other projects so the one linking to its copy is past the first page.
    let plan = Plan::new(false);
    let vault = plan.sandbox.subdirectory("vault");
    let published = plan.sandbox.subdirectory("published");
    let onward = plan.sandbox.subdirectory("onward");
    let (limited, _) = linear_rate_limited(&plan.sandbox);
    // A private workspace whose credential is not there at all.
    let (mut keyless, _) = linear_rate_limited(&plan.sandbox);
    keyless["api_key_env"] = json!("ONETASKGRAPH_TEST_NO_SUCH_KEY");
    let mut configured: Value = serde_json::from_str(&document(&json!({
        "authoring": {"plugin": "local-md", "visibility": "private",
                      "config": {"root": plan.notes}},
        "secret": {"plugin": "linear", "visibility": "private", "config": limited},
        "keyless": {"plugin": "linear", "visibility": "private", "config": keyless},
        "vault": {"plugin": "local-md", "visibility": "private", "config": {"root": vault}},
        "published": {"plugin": "local-md", "visibility": "public",
                      "config": {"root": published}},
        "onward": {"plugin": "local-md", "visibility": "public", "config": {"root": onward}},
    })))
    .expect("a document");
    configured["write_policy"] = json!({
        "check_command": [
            interpreter(), "-c", "import sys; sys.stdin.read(); print('{\"verdict\": \"pass\"}')"
        ],
        "visibility_command": [
            interpreter(), "-c",
            "import sys; sys.stdin.read(); print('{\"visibility\": \"public\"}')"
        ],
    });
    plan.sandbox
        .project_document(&serde_json::to_string_pretty(&configured).expect("a document"));
    plan.sandbox.secrets_file("LINEAR_API_KEY=fixture-key\n");
    let write = |root: &Path, relative: &str, text: &str| {
        let file = root.join(relative);
        std::fs::create_dir_all(file.parent().expect("a directory")).expect("the directory");
        std::fs::write(file, text).expect("a file");
    };
    write(
        &plan.notes,
        "projects/launch.md",
        "---\ntitle: Launch\nstatus: todo\n---\n",
    );
    for n in 0..60 {
        write(
            &plan.notes,
            &format!("projects/filler-{n:02}.md"),
            &format!("---\ntitle: Filler {n}\nstatus: todo\n---\n"),
        );
    }
    write(
        &plan.notes,
        "projects/upstream.md",
        "---\ntitle: Upstream\nstatus: todo\n---\n",
    );
    write(
        &plan.notes,
        "projects/downstream.md",
        "---\ntitle: Downstream\nstatus: todo\ndepends_on: [{id: upstream, item: project}]\n---\n",
    );
    let a_file = "---\ntitle: A\nstatus: todo\nproject: launch\n\
                  metadata:\n  \"onetaskgraph.origin\": \"vault:root-a\"\n---\nDo A.\n";
    write(&plan.notes, "tasks/a.md", a_file);
    write(
        &plan.notes,
        "tasks/b.md",
        "---\ntitle: B\nstatus: todo\nproject: launch\ndepends_on: [a]\n---\nDo B.\n",
    );
    plan.attest("task", "authoring:b");
    plan.attest("project", "authoring:downstream");
    plan.holds(
        "task",
        "authoring:b",
        "valid",
        Value::Null,
        "where it was reviewed",
    );

    // The copy of `a` names no root: `authoring:a` links to it — past a private source that
    // cannot be asked — and the root `authoring:a` records is what B's copy names.
    let first = copy_plan(&plan, "authoring:launch", "published");
    let (copy_a, copy_b) = (landed(&first, "authoring:a"), landed(&first, "authoring:b"));
    plan.holds(
        "task",
        &copy_b,
        "valid",
        Value::Null,
        "by the root of the private item its far end was copied from",
    );
    // Copied on, A's copy's copy records A's copy, which is read once for the root behind it.
    let second = copy_plan(&plan, &landed(&first, "authoring:launch"), "onward");
    let onward_b = landed(&second, &copy_b);
    plan.holds("task", &onward_b, "valid", Value::Null, "two copies on");

    // A second private item linking to A's copy is no confident answer: the edge names the
    // copy itself, and the record goes void until it is gone.
    let copy_native = copy_a.split_once(':').expect("qualified").1;
    write(
        &vault,
        "tasks/decoy.md",
        &format!(
            "---\ntitle: Decoy\nstatus: todo\n\
             metadata:\n  \"onetaskgraph.copies\": {{\"published\": \"{copy_a}\"}}\n---\n"
        ),
    );
    plan.holds(
        "task",
        &copy_b,
        "void",
        json!("content-moved"),
        "two private items link to its far end",
    );
    std::fs::remove_file(vault.join("tasks/decoy.md")).expect("the decoy goes");
    plan.holds("task", &copy_b, "valid", Value::Null, "one link again");

    // A root that is no qualified id is refused, never guessed — on the private item found,
    // and on the copy read for the root behind it.
    let refused = |id: &str, naming: &str, why: &str| {
        let output = plan.run(&["task", "attestation", "check", id, "--kind", KIND]);
        assert_ne!(output.status.code(), Some(0), "{why}: {}", stdout(&output));
        assert!(
            stderr(&output).contains(naming) && stderr(&output).contains("not a qualified id"),
            "{why}: {:?}\n{}\n{}",
            output.status.code(),
            stdout(&output),
            stderr(&output)
        );
    };
    // `a`'s file now holds the link the copy recorded beside its root: it is edited in place.
    let linked = std::fs::read_to_string(plan.notes.join("tasks/a.md")).expect("a's file");
    write(
        &plan.notes,
        "tasks/a.md",
        &linked.replacen("vault:root-a", "nowhere", 1),
    );
    refused(
        &copy_b,
        "authoring:a",
        "the private item records a malformed root",
    );
    write(&plan.notes, "tasks/a.md", &linked);
    plan.holds("task", &copy_b, "valid", Value::Null, "its root restored");
    let held = published.join("tasks").join(format!("{copy_native}.md"));
    let text = std::fs::read_to_string(&held).expect("A's copy");
    std::fs::write(
        &held,
        text.replacen(
            "metadata:\n",
            "metadata:\n  \"onetaskgraph.origin\": \"nowhere\"\n",
            1,
        ),
    )
    .expect("an edit");
    refused(
        &onward_b,
        &copy_a,
        "the recorded root records a malformed root",
    );
    std::fs::write(&held, text).expect("restored");

    // A project's edge. The copy writes `downstream` without its edge to a project outside it,
    // so the edge is given to the copy by hand, naming the copy of `upstream` — which names no
    // root, and which `authoring:upstream` links to.
    let upstream = landed(
        &copy_plan(&plan, "authoring:upstream", "published"),
        "authoring:upstream",
    );
    let downstream = landed(
        &copy_plan(&plan, "authoring:downstream", "published"),
        "authoring:downstream",
    );
    plan.holds(
        "project",
        &downstream,
        "void",
        json!("content-moved"),
        "the copy holds no edge",
    );
    let file = published.join("projects/downstream.md");
    let text = std::fs::read_to_string(&file).expect("downstream's copy");
    let upstream_native = upstream.split_once(':').expect("qualified").1;
    std::fs::write(
        &file,
        text.replacen(
            "status: todo\n",
            &format!("status: todo\ndepends_on: [{{id: {upstream_native}, item: project}}]\n"),
            1,
        ),
    )
    .expect("an edge");
    plan.holds(
        "project",
        &downstream,
        "valid",
        Value::Null,
        "its edge names the copy of the project it depended on",
    );
    // A second project of the same private source linking to that copy — on the page before
    // the one `authoring:upstream` is listed on — is no confident answer either.
    write(
        &plan.notes,
        "projects/aa-decoy.md",
        &format!(
            "---\ntitle: Decoy\nstatus: todo\n\
             metadata:\n  \"onetaskgraph.copies\": {{\"published\": \"{upstream}\"}}\n---\n"
        ),
    );
    plan.holds(
        "project",
        &downstream,
        "void",
        json!("content-moved"),
        "two projects link to its far end",
    );
    std::fs::remove_file(plan.notes.join("projects/aa-decoy.md")).expect("the decoy goes");

    // Copied on, the copy of `upstream`'s copy records it, and it is read for the root behind
    // it; a read of it that fails fails the check.
    let onward_upstream = landed(&copy_plan(&plan, &upstream, "onward"), &upstream);
    let onward_downstream = landed(&copy_plan(&plan, &downstream, "onward"), &downstream);
    let file = onward.join(format!(
        "projects/{}.md",
        onward_downstream.split_once(':').expect("qualified").1
    ));
    let text = std::fs::read_to_string(&file).expect("downstream's onward copy");
    let onward_native = onward_upstream.split_once(':').expect("qualified").1;
    assert!(
        text.contains("depends_on:") && text.contains(onward_native),
        "a copy between public sources carries the edge to the onward copy of `upstream`: {text}"
    );
    plan.holds(
        "project",
        &onward_downstream,
        "valid",
        Value::Null,
        "two copies on, by the root behind the project its far end records",
    );
    let upstream_file = published.join(format!("projects/{upstream_native}.md"));
    let upstream_text = std::fs::read_to_string(&upstream_file).expect("upstream's copy");
    std::fs::write(&upstream_file, "---\ntitle: [unclosed\n---\n").expect("a broken file");
    let unread = plan.run(&[
        "project",
        "attestation",
        "check",
        &onward_downstream,
        "--kind",
        KIND,
    ]);
    assert_ne!(
        unread.status.code(),
        Some(0),
        "the recorded project unreadable: {}",
        stdout(&unread)
    );
    assert!(
        stderr(&unread).contains("published"),
        "the failure names the source holding it: {}",
        stderr(&unread)
    );
    std::fs::write(&upstream_file, upstream_text).expect("restored");

    // A far end in a public source that nothing private links to is named by its own id, so a
    // record taken over it holds; a private source failing for any other reason than being
    // unreachable fails the check rather than reading as no link.
    write(
        &published,
        "tasks/p.md",
        "---\ntitle: P\nstatus: todo\nproject: launch\n\
         metadata:\n  \"onetaskgraph.origin\": \"vault:root-p\"\n---\nDo P.\n",
    );
    write(
        &published,
        "tasks/q.md",
        "---\ntitle: Q\nstatus: todo\nproject: launch\ndepends_on: [p, n]\n---\nDo Q.\n",
    );
    write(
        &published,
        "tasks/n.md",
        "---\ntitle: N\nstatus: todo\nproject: launch\n---\nDo N.\n",
    );
    plan.attest("task", "published:q");
    plan.holds(
        "task",
        "published:q",
        "valid",
        Value::Null,
        "a far end no private item links to",
    );
    write(&vault, "tasks/broken.md", "---\ntitle: [unclosed\n---\n");
    let broken = plan.run(&[
        "task",
        "attestation",
        "check",
        "published:q",
        "--kind",
        KIND,
    ]);
    assert_ne!(
        broken.status.code(),
        Some(0),
        "a private source failing: {}",
        stdout(&broken)
    );
    assert!(
        stderr(&broken).contains("vault"),
        "the failure names the private source: {}",
        stderr(&broken)
    );
    std::fs::remove_file(vault.join("tasks/broken.md")).expect("the broken file goes");

    // Copied on, Q's copy's far end records `published:p`, which is read for the root it
    // records in turn — `vault:root-p` — and a read of it that fails fails the check.
    let tasks = plan.json(&[
        "task",
        "copy",
        "published:p",
        "published:n",
        "published:q",
        "--to",
        "onward",
    ]);
    let onward_q = landed(&tasks, "published:q");
    plan.holds(
        "task",
        &onward_q,
        "valid",
        Value::Null,
        "by the root its far end's origin records",
    );
    let p_file = published.join("tasks/p.md");
    let p_text = std::fs::read_to_string(&p_file).expect("p's file");
    std::fs::write(&p_file, "---\ntitle: [unclosed\n---\n").expect("a broken file");
    let unread = plan.run(&["task", "attestation", "check", &onward_q, "--kind", KIND]);
    assert_ne!(
        unread.status.code(),
        Some(0),
        "the recorded root unreadable: {}",
        stdout(&unread)
    );
    assert!(
        stderr(&unread).contains("published"),
        "the failure names the source holding it: {}",
        stderr(&unread)
    );
    std::fs::write(&p_file, p_text).expect("restored");
    plan.holds(
        "task",
        &onward_q,
        "valid",
        Value::Null,
        "its far end's origin restored",
    );
}
