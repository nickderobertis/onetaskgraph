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
use crate::fixtures::{document, github_projects_with_board, linear_workspace};

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
        if hosts {
            let (board, _) = github_projects_with_board(&sandbox);
            sources["board"] = json!({"plugin": "github-projects", "config": board});
            let (linear, _) = linear_workspace(
                &sandbox,
                json!({"tasks": [], "projects": [], "documents": [], "labels": [],
                       "task_dependencies": [], "project_dependencies": []}),
            );
            sources["linear"] = json!({"plugin": "linear", "config": linear});
            sandbox.secrets_file(
                "GITHUB_PROJECTS_FIXTURE_TOKEN=test-token\nLINEAR_API_KEY=fixture-key\n",
            );
        }
        sandbox.project_document(&document(&sources));
        Self {
            sandbox,
            notes,
            templates,
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
fn an_edge_names_its_far_end_wherever_it_is_and_goes_void_when_it_moves() {
    let plan = Plan::new(false);
    let other = plan.sandbox.subdirectory("other");
    plan.sandbox.project_document(&document(&json!({
        "notes": {"plugin": "local-md", "config": {"root": plan.notes}},
        "other": {"plugin": "local-md", "config": {"root": other}},
    })));
    let moved = json!("content-moved");
    let body = plan.file("far.md", "Far.\n");

    // A task depending on a task of another configured source.
    let far = stdout(&plan.exits(
        &[
            "task",
            "create",
            "other",
            "--project",
            "elsewhere",
            "--title",
            "Upstream",
            "--body-file",
            &body,
        ],
        0,
    ))
    .trim()
    .to_owned();
    let near = stdout(&plan.exits(
        &[
            "task",
            "create",
            "notes",
            "--project",
            "plan-1",
            "--title",
            "Near",
            "--body-file",
            &body,
            "--depends-on",
            &far,
        ],
        0,
    ))
    .trim()
    .to_owned();
    plan.after(
        "task",
        &near,
        &["task", "update", &far, "--title", "Upstream, renamed"],
        "void",
        moved.clone(),
        "the far end in another source renamed",
    );
    // Its far end gone, the edge is named by its id instead: another edge.
    plan.attest("task", &near);
    let native = far.split_once(':').expect("qualified").1;
    std::fs::remove_file(other.join("tasks").join(format!("{native}.md")))
        .expect("a far end removed");
    plan.holds("task", &near, "void", moved.clone(), "the far end removed");
    plan.attest("task", &near);
    plan.holds(
        "task",
        &near,
        "valid",
        Value::Null,
        "named by its id, it holds again",
    );

    // A project depending on a project.
    let template = plan.template("plan.md");
    for (id, title) in [("upstream", "Upstream plan"), ("plan-1", "The plan")] {
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
        "void",
        moved,
        "the project it depends on renamed",
    );
}
