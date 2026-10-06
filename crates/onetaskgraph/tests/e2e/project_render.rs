//! `project create`, `project render` and `project answers`, driven through the binary the way
//! a person or a script drives them.
//!
//! A project's description is created and regenerated from a template on the terms a task's
//! and a document's content are: it records the same `onetaskgraph.template` entry, a folder
//! of Markdown keeps its answers in the project's own file, and a copy carries the content,
//! the caller's metadata and the entry, and never the answers. The hashes are recomputed here
//! from what the binary reads back, never taken from the engine that wrote them.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{GitHubBoardFields, document, github_projects_with_board, linear_workspace};
use crate::machine::{bundle, validates};

/// A plan-description template: one required variable, one list with a default, and one
/// optional variable, with its header included from the search path.
const PLAN: &str = "---\n\
onetaskgraph_template: 1\n\
variables:\n  \
  goal:\n    \
    description: What the plan is for\n  \
  budgets:\n    \
    description: The plan-level budgets\n    \
    type: list\n    \
    default: []\n  \
  owner:\n    \
    description: Who owns it\n    \
    required: false\n\
---\n\
{% include \"header.md\" %}\n\
Goal: {{ goal }}\n\
{% for budget in budgets %}\n\
- {{ budget }}\n\
{% endfor %}\n\
{% if owner %}\n\
Owner: {{ owner }}\n\
{% endif %}\n";

const HEADER: &str = "# The plan\n";

/// A project file written by hand: a status, a label, a repository, caller metadata and a
/// dependency, every one of which a replacement keeps.
const HELD: &str = "---\n\
title: Held by hand\n\
status: in-progress\n\
labels:\n- id: budget\n  name: budget\n\
repositories:\n- github.com/acme/widgets\n\
depends_on: [upstream]\n\
metadata:\n  myapp.owner: \"ops\"\n  myapp.size: 3\n\
---\n\
Written by hand.\n";

const UPSTREAM: &str = "---\ntitle: Upstream\nstatus: todo\n---\nFirst.\n";

/// One sandbox holding a folder of Markdown called `notes`, a second one called `back`, a
/// directory of templates and — for the journeys about a hosted board — the loopback board.
struct Plan {
    sandbox: Sandbox,
    notes: PathBuf,
    back: PathBuf,
    templates: PathBuf,
    board: Option<GitHubBoardFields>,
}

impl Plan {
    fn new() -> Self {
        Self::with_board(false)
    }

    fn with_board(board: bool) -> Self {
        Self::with_hosts(board, false)
    }

    /// The plan, with the loopback board when `board` and a loopback Linear workspace called
    /// `linear` when `linear`.
    fn with_hosts(board: bool, linear: bool) -> Self {
        let sandbox = Sandbox::new();
        let notes = sandbox.subdirectory("notes");
        let back = sandbox.subdirectory("back");
        let templates = sandbox.subdirectory("templates");
        std::fs::write(templates.join("plan.md"), PLAN).expect("the template");
        std::fs::write(templates.join("header.md"), HEADER).expect("the header");
        let mut sources = json!({
            "notes": {"plugin": "local-md", "config": {"root": notes}},
            "back": {"plugin": "local-md", "config": {"root": back}},
            "frozen": {"plugin": "in-memory",
                       "config": {"capabilities": {"writes": "unsupported"}}},
        });
        let board = board.then(|| {
            let (config, fields) = github_projects_with_board(&sandbox);
            sources["board"] = json!({"plugin": "github-projects", "config": config});
            fields
        });
        if linear {
            let (config, _) = linear_workspace(
                &sandbox,
                json!({"tasks": [], "projects": [], "documents": [], "labels": [],
                       "task_dependencies": [], "project_dependencies": []}),
            );
            sources["linear"] = json!({"plugin": "linear", "config": config});
            // Each stand-in writes the one secrets file with its own credential alone.
            sandbox.secrets_file(
                "GITHUB_PROJECTS_FIXTURE_TOKEN=test-token\nLINEAR_API_KEY=fixture-key\n",
            );
        }
        sandbox.project_document(&document(&sources));
        Self {
            sandbox,
            notes,
            back,
            templates,
            board,
        }
    }

    fn template(&self) -> String {
        path(&self.templates.join("plan.md"))
    }

    fn search_path(&self) -> String {
        path(&self.templates)
    }

    fn board(&self) -> &GitHubBoardFields {
        self.board.as_ref().expect("a plan with the board")
    }

    /// A file under the sandbox, written with `text`.
    fn file(&self, name: &str, text: &str) -> String {
        let file = self.sandbox.subdirectory("inputs").join(name);
        std::fs::write(&file, text).expect("an input file");
        path(&file)
    }

    /// The file of the project `id` names in the folder `root`.
    fn project_file(root: &Path, id: &str) -> PathBuf {
        root.join("projects").join(format!("{}.md", native(id)))
    }

    /// The file of a project of `notes`, read whole.
    fn notes_file(&self, id: &str) -> String {
        std::fs::read_to_string(Self::project_file(&self.notes, id)).expect("the project file")
    }

    fn run(&self, arguments: &[&str]) -> Output {
        self.sandbox
            .command()
            .args(arguments)
            .assert()
            .get_output()
            .clone()
    }

    fn exits(&self, arguments: &[&str], code: i32) -> Output {
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

    fn json(&self, arguments: &[&str]) -> Value {
        let mut arguments = arguments.to_vec();
        arguments.push("--json");
        let output = self.exits(&arguments, 0);
        serde_json::from_str(&stdout(&output)).unwrap_or_else(|error| {
            panic!(
                "`onetaskgraph {}` wrote no JSON ({error})",
                arguments.join(" ")
            )
        })
    }

    /// One project as `project show --json` reads it.
    fn project(&self, id: &str) -> Value {
        self.json(&["project", "show", id])["items"][0]["item"].clone()
    }

    /// `project create` in `source` under `id` from the plan template, answered by
    /// `arguments`, answering with the qualified id it printed.
    fn create(&self, source: &str, id: &str, title: &str, arguments: &[&str]) -> String {
        let template = self.template();
        let search_path = self.search_path();
        let mut all = vec![
            "project",
            "create",
            source,
            "--id",
            id,
            "--title",
            title,
            "--template",
            &template,
            "--search-path",
            &search_path,
            "--no-interactive",
        ];
        all.extend_from_slice(arguments);
        stdout(&self.exits(&all, 0)).trim().to_owned()
    }

    /// `project render` of a project whose recorded template is the plan template: the
    /// chain's search path is not part of what an item records, so it is named again here.
    fn render(&self, id: &str, arguments: &[&str]) -> Output {
        let search_path = self.search_path();
        let mut all = vec![
            "project",
            "render",
            id,
            "--search-path",
            &search_path,
            "--no-interactive",
        ];
        all.extend_from_slice(arguments);
        self.run(&all)
    }

    /// [`Plan::render`], which had to succeed, as the JSON it wrote.
    fn rendered(&self, id: &str, arguments: &[&str]) -> Value {
        let mut all = arguments.to_vec();
        all.push("--json");
        let output = self.render(id, &all);
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        serde_json::from_str(&stdout(&output)).expect("project render --json writes JSON")
    }

    /// The digest `template variables` reports for the plan template.
    fn chain_digest(&self) -> Value {
        let template = self.template();
        let search_path = self.search_path();
        self.json(&[
            "template",
            "variables",
            &template,
            "--search-path",
            &search_path,
        ])["digest"]
            .clone()
    }
}

fn path(path: &Path) -> String {
    path.to_str().expect("a UTF-8 path").to_owned()
}

fn native(id: &str) -> &str {
    id.split_once(':').expect("a qualified id").1
}

fn sha256(text: &str) -> String {
    format!("sha256:{:x}", Sha256::digest(text.as_bytes()))
}

/// `value` as canonical JSON: keys sorted at every depth, no insignificant whitespace.
fn canonical(value: &Value) -> String {
    match value {
        Value::Object(entries) => {
            let sorted: BTreeMap<&String, &Value> = entries.iter().collect();
            let inner: Vec<String> = sorted
                .into_iter()
                .map(|(key, entry)| format!("{}:{}", Value::String(key.clone()), canonical(entry)))
                .collect();
            format!("{{{}}}", inner.join(","))
        }
        Value::Array(entries) => {
            let inner: Vec<String> = entries.iter().map(canonical).collect();
            format!("[{}]", inner.join(","))
        }
        scalar => scalar.to_string(),
    }
}

/// The answers block a project file ends in, byte for byte from its opening line.
fn block(file: &str) -> &str {
    let at = file
        .find("<!-- onetaskgraph:template-answers\n")
        .unwrap_or_else(|| panic!("a file with an answers block:\n{file}"));
    &file[at..]
}

/// The metadata slot at the end of one issue body, and the body before it.
fn slot(body: &str) -> (String, Value) {
    let (visible, rest) = body
        .split_once("\n\n<!-- onetaskgraph.metadata\n")
        .unwrap_or_else(|| panic!("an issue body with a metadata slot:\n{body}"));
    let encoded = rest.strip_suffix("\n-->").expect("a closed slot");
    (
        visible.to_owned(),
        serde_json::from_str(encoded).expect("the slot is JSON"),
    )
}

/// Assert that `entry` is the provenance a rendering of `content` from `template` with
/// `answers` records: the reference, the chain's digest, and both hashes recomputed here.
fn proves(entry: &Value, template: &str, digest: &Value, content: &str, answers: &Value) {
    assert_eq!(entry["template"], template, "the reference, verbatim");
    assert_eq!(&entry["digest"], digest, "the chain's digest");
    assert_eq!(
        entry["body_digest"],
        json!(sha256(content)),
        "body_digest hashes the content exactly as written"
    );
    assert_eq!(
        entry["answers_digest"],
        json!(sha256(&canonical(answers))),
        "answers_digest hashes the resolved answers as canonical JSON"
    );
}

#[test]
fn a_project_created_from_a_template_records_its_provenance_and_keeps_its_answers() {
    let plan = Plan::new();
    let id = plan.create(
        "notes",
        "plan-1",
        "The plan",
        &[
            "--var",
            "goal=Ship it",
            "--var",
            "budgets=[\"tokens: 10\", \"hours: 2\"]",
            "--status",
            "in-progress",
            "--label",
            "budget",
            "--repository",
            "github.com/acme/widgets",
            "--metadata",
            "myapp.estimate=3",
        ],
    );
    assert_eq!(id, "notes:plan-1");

    let shown = plan.json(&["project", "show", &id]);
    validates(
        &bundle(&plan.sandbox),
        "QueryResponseOfQualifiedProject",
        &shown,
        "project show --json",
    );
    let project = shown["items"][0]["item"].clone();
    let content = project["content"].as_str().expect("content").to_owned();
    assert!(
        content.starts_with("# The plan\nGoal: Ship it\n") && content.contains("- hours: 2"),
        "{content}"
    );
    assert_eq!(project["title"], "The plan");
    assert_eq!(project["status"]["category"], "in-progress");
    assert_eq!(project["labels"][0]["name"], "budget");
    assert_eq!(project["repositories"], json!(["github.com/acme/widgets"]));
    assert_eq!(project["metadata"]["myapp.estimate"], json!(3));

    // The answers are in the project's own file, after its content, and nowhere in its
    // content or its metadata.
    let answers = plan.json(&["project", "answers", &id]);
    validates(
        &bundle(&plan.sandbox),
        "TemplateAnswers",
        &answers,
        "project answers --json",
    );
    assert_eq!(
        answers,
        json!({"goal": "Ship it", "budgets": ["tokens: 10", "hours: 2"], "owner": null})
    );
    proves(
        &project["metadata"]["onetaskgraph.template"],
        &plan.template(),
        &plan.chain_digest(),
        &content,
        &answers,
    );
    let file = plan.notes_file(&id);
    assert!(
        file.contains(&format!(
            "{content}\n\n<!-- onetaskgraph:template-answers\n"
        )),
        "the block follows the content:\n{file}"
    );
    assert!(!content.contains("template-answers"));
    let yaml = stdout(&plan.exits(&["project", "answers", &id], 0));
    assert!(
        yaml.contains("goal: Ship it") && yaml.contains("- 'tokens: 10'"),
        "the answers as YAML, which `--answers` reads back:\n{yaml}"
    );

    // Regenerated from the answers it keeps, nothing changes and nothing is written.
    let unchanged = plan.rendered(&id, &[]);
    assert_eq!(unchanged["changed"], false);
    assert_eq!(
        plan.notes_file(&id),
        file,
        "an unchanged render writes nothing"
    );

    // `--json` answers exactly what `project show --json` does.
    let created = plan.json(&[
        "project",
        "create",
        "notes",
        "--id",
        "plan-2",
        "--title",
        "Second",
        "--template",
        &plan.template(),
        "--search-path",
        &plan.search_path(),
        "--var",
        "goal=Again",
        "--no-interactive",
    ]);
    assert_eq!(created, plan.json(&["project", "show", "notes:plan-2"]));
}

#[test]
fn a_project_rendered_from_a_loader_document_records_its_reference_and_both_hashes() {
    let plan = Plan::new();
    let loader = |header: &str| {
        json!({
            "reference": "$(touch pwned) host:plan-description",
            "entry": "plan.md",
            "search_path": [plan.templates],
            "templates": [{"name": "header.md", "source": header}],
        })
        .to_string()
    };
    // The loader's pair is searched after the directory, so the directory's header must go.
    std::fs::remove_file(plan.templates.join("header.md")).unwrap();
    let first = plan.file("loader.json", &loader("# From the loader\n"));
    let id = stdout(&plan.exits(
        &[
            "project",
            "create",
            "notes",
            "--id",
            "loaded",
            "--title",
            "Loaded",
            "--template-loader",
            &first,
            "--var",
            "goal=Ship it",
            "--no-interactive",
        ],
        0,
    ))
    .trim()
    .to_owned();
    let project = plan.project(&id);
    let content = project["content"].as_str().unwrap().to_owned();
    assert!(
        content.starts_with("# From the loader\nGoal: Ship it\n"),
        "{content}"
    );
    let digest =
        plan.json(&["template", "variables", "--template-loader", &first])["digest"].clone();
    let answers = plan.json(&["project", "answers", &id]);
    proves(
        &project["metadata"]["onetaskgraph.template"],
        "$(touch pwned) host:plan-description",
        &digest,
        &content,
        &answers,
    );

    // Regenerated from an edited loader: a new chain digest and both hashes moved with it.
    let edited = plan.file("edited.json", &loader("# From the edited loader\n"));
    let regenerated = plan.json(&[
        "project",
        "render",
        &id,
        "--template-loader",
        &edited,
        "--var",
        "owner=ops",
        "--no-interactive",
    ]);
    assert_eq!(regenerated["changed"], true);
    let project = plan.project(&id);
    let content = project["content"].as_str().unwrap().to_owned();
    assert_eq!(regenerated["body"], json!(content));
    let answers = plan.json(&["project", "answers", &id]);
    assert_eq!(
        answers,
        json!({"goal": "Ship it", "budgets": [], "owner": "ops"})
    );
    let digest =
        plan.json(&["template", "variables", "--template-loader", &edited])["digest"].clone();
    assert_eq!(regenerated["digest"], digest);
    proves(
        &project["metadata"]["onetaskgraph.template"],
        "$(touch pwned) host:plan-description",
        &digest,
        &content,
        &answers,
    );

    // Without a loader, a recorded reference that is not a file is refused by name, and
    // nothing was ever run.
    let refused = plan.exits(&["project", "render", &id, "--no-interactive"], 1);
    assert!(
        stderr(&refused).contains("project notes:loaded was rendered from")
            && stderr(&refused).contains("--template-loader"),
        "{}",
        stderr(&refused)
    );
    assert!(!plan.sandbox.project().join("pwned").exists());
}

#[test]
fn a_create_over_a_held_project_replaces_its_rendering_and_keeps_everything_else() {
    let plan = Plan::new();
    std::fs::create_dir_all(plan.notes.join("projects")).unwrap();
    std::fs::write(plan.notes.join("projects/held.md"), HELD).unwrap();
    std::fs::write(plan.notes.join("projects/upstream.md"), UPSTREAM).unwrap();
    let before = plan.project("notes:held");

    let id = plan.create(
        "notes",
        "held",
        "Rendered over",
        &["--var", "goal=Replace it"],
    );
    assert_eq!(
        id, "notes:held",
        "replaced under its own id, not filed beside it"
    );
    let after = plan.project(&id);
    assert_eq!(after["title"], "Rendered over");
    assert!(
        after["content"]
            .as_str()
            .unwrap()
            .contains("Goal: Replace it")
    );
    for kept in ["status", "labels", "repositories"] {
        assert_eq!(after[kept], before[kept], "{kept} is kept");
    }
    assert_eq!(after["metadata"]["myapp.owner"], "ops");
    assert_eq!(after["metadata"]["myapp.size"], json!(3));
    assert!(after["metadata"]["onetaskgraph.template"].is_object());
    assert_eq!(
        plan.json(&["project", "deps", &id])["items"][0]["to"]["id"],
        "notes:upstream",
        "its dependency is kept"
    );
    assert_eq!(
        plan.json(&["project", "answers", &id]),
        json!({"goal": "Replace it", "budgets": [], "owner": null})
    );
    let projects = std::fs::read_dir(plan.notes.join("projects"))
        .unwrap()
        .count();
    assert_eq!(projects, 2, "no second project was filed");

    // Created again with new answers and the fields it names: the rendering, the answers and
    // the provenance are replaced whole, and each field named wins over what was held.
    plan.create(
        "notes",
        "held",
        "Rendered again",
        &[
            "--var",
            "goal=Again",
            "--var",
            "owner=ops",
            "--status",
            "done",
            "--label",
            "shipped",
            "--metadata",
            "myapp.size=5",
        ],
    );
    let again = plan.project(&id);
    assert_eq!(again["status"]["category"], "done");
    assert_eq!(
        again["labels"]
            .as_array()
            .unwrap()
            .iter()
            .map(|label| label["name"].clone())
            .collect::<Vec<_>>(),
        vec![json!("shipped")]
    );
    assert_eq!(again["repositories"], before["repositories"]);
    assert_eq!(again["metadata"]["myapp.size"], json!(5));
    assert_eq!(again["metadata"]["myapp.owner"], "ops");
    let answers = plan.json(&["project", "answers", &id]);
    assert_eq!(
        answers,
        json!({"goal": "Again", "budgets": [], "owner": "ops"})
    );
    proves(
        &again["metadata"]["onetaskgraph.template"],
        &plan.template(),
        &plan.chain_digest(),
        again["content"].as_str().unwrap(),
        &answers,
    );

    // Replaced by a plain body: the provenance and the answers go with the rendering.
    let body = plan.file("plain.md", "Plain again.");
    plan.exits(
        &[
            "project",
            "create",
            "notes",
            "--id",
            "held",
            "--title",
            "Plain",
            "--body-file",
            &body,
        ],
        0,
    );
    let plain = plan.project(&id);
    assert_eq!(plain["content"], "Plain again.");
    assert!(plain["metadata"].get("onetaskgraph.template").is_none());
    assert_eq!(plain["metadata"]["myapp.owner"], "ops");
    assert!(
        !plan
            .notes_file(&id)
            .contains("onetaskgraph:template-answers")
    );
    let refused = plan.exits(&["project", "answers", &id], 1);
    assert!(
        stderr(&refused).contains("project notes:held has no stored template answers"),
        "{}",
        stderr(&refused)
    );
}

#[test]
fn a_render_keeps_every_other_field_and_writes_nothing_when_nothing_would_change() {
    let plan = Plan::new();
    std::fs::create_dir_all(plan.notes.join("projects")).unwrap();
    std::fs::write(plan.notes.join("projects/upstream.md"), UPSTREAM).unwrap();
    let id = plan.create(
        "notes",
        "plan",
        "The plan",
        &[
            "--var",
            "goal=First",
            "--status",
            "in-progress",
            "--label",
            "budget",
            "--repository",
            "github.com/acme/widgets",
            "--metadata",
            "myapp.owner=\"ops\"",
        ],
    );
    // A dependency and a second metadata key, written after the create, both survive a render.
    plan.exits(&["project", "metadata", "set", &id, "myapp.size", "3"], 0);
    let before = plan.project(&id);

    let regenerated = plan.rendered(&id, &["--var", "goal=Second", "--var", "owner=me"]);
    validates(
        &bundle(&plan.sandbox),
        "Regenerated",
        &regenerated,
        "project render --json",
    );
    assert_eq!(regenerated["changed"], true);
    assert_eq!(regenerated["id"], id);
    let after = plan.project(&id);
    assert_eq!(regenerated["body"], after["content"]);
    assert!(after["content"].as_str().unwrap().contains("Goal: Second"));
    for kept in [
        "id",
        "title",
        "status",
        "labels",
        "repositories",
        "location",
    ] {
        assert_eq!(after[kept], before[kept], "{kept} is kept");
    }
    for key in ["myapp.owner", "myapp.size"] {
        assert_eq!(
            after["metadata"][key], before["metadata"][key],
            "{key} is kept"
        );
    }
    let answers = plan.json(&["project", "answers", &id]);
    assert_eq!(
        answers,
        json!({"goal": "Second", "budgets": [], "owner": "me"})
    );
    proves(
        &after["metadata"]["onetaskgraph.template"],
        &plan.template(),
        &plan.chain_digest(),
        after["content"].as_str().unwrap(),
        &answers,
    );

    // The same answers again: nothing changes and nothing is written.
    let file = plan.notes_file(&id);
    let unchanged = plan.rendered(&id, &[]);
    assert_eq!(unchanged["changed"], false);
    assert_eq!(plan.notes_file(&id), file, "byte-identical");

    // A dry run reports a change and writes nothing.
    let dry = plan.rendered(&id, &["--var", "goal=Never written", "--dry-run"]);
    assert_eq!(dry["changed"], true);
    assert_eq!(plan.notes_file(&id), file, "a dry run writes nothing");

    // An unset answer takes its default.
    let unset = plan.rendered(&id, &["--unset", "owner"]);
    assert_eq!(unset["changed"], true);
    assert_eq!(
        plan.json(&["project", "answers", &id]),
        json!({"goal": "Second", "budgets": [], "owner": null})
    );

    // A source with no write side is refused naming the record.
    let frozen = plan.exits(
        &[
            "project",
            "create",
            "frozen",
            "--id",
            "nope",
            "--title",
            "Nope",
            "--template",
            &plan.template(),
            "--search-path",
            &plan.search_path(),
            "--var",
            "goal=Nope",
            "--no-interactive",
        ],
        1,
    );
    assert!(
        stderr(&frozen).contains("frozen") && stderr(&frozen).contains("project"),
        "{}",
        stderr(&frozen)
    );
    // And an unknown project is refused naming it.
    let missing = plan.render("notes:nowhere", &["--var", "goal=x"]);
    assert_eq!(missing.status.code(), Some(1), "{}", stderr(&missing));
    assert!(stderr(&missing).contains("nowhere"), "{}", stderr(&missing));
}

#[test]
fn a_hand_edited_project_is_regenerated_or_refused_as_a_hand_edited_document_is() {
    let plan = Plan::new();
    let id = plan.create("notes", "plan", "The plan", &["--var", "goal=First"]);
    let rendered = plan.project(&id)["content"].clone();

    // Its content edited by hand: the provenance shows it, and a render restores the rendering.
    let file = plan.notes_file(&id);
    let edited = file.replacen("Goal: First", "Goal: Edited by hand", 1);
    std::fs::write(Plan::project_file(&plan.notes, &id), &edited).unwrap();
    let hand = plan.project(&id);
    assert_ne!(
        hand["metadata"]["onetaskgraph.template"]["body_digest"],
        json!(sha256(hand["content"].as_str().unwrap())),
        "a hand edit is visible from the provenance"
    );
    let restored = plan.rendered(&id, &[]);
    assert_eq!(restored["changed"], true);
    assert_eq!(plan.project(&id)["content"], rendered);

    // Its answers edited by hand: they are no longer trusted, and the render is refused until
    // every required answer is given.
    let file = plan.notes_file(&id);
    let tampered = file.replacen("goal: First", "goal: Tampered", 1);
    assert_ne!(tampered, file);
    std::fs::write(Plan::project_file(&plan.notes, &id), &tampered).unwrap();
    let refused = plan.render(&id, &[]);
    assert_eq!(refused.status.code(), Some(2), "{}", stderr(&refused));
    assert!(
        stderr(&refused).contains("supply every required answer")
            && stderr(&refused).contains("changed after it was rendered"),
        "{}",
        stderr(&refused)
    );
    assert_eq!(plan.notes_file(&id), tampered, "nothing written");
    let given = plan.rendered(&id, &["--var", "goal=Given again"]);
    assert_eq!(given["changed"], true);

    // A provenance entry this product did not write names nothing until a template is given.
    std::fs::create_dir_all(plan.notes.join("projects")).unwrap();
    let forged = "---\ntitle: Forged\nstatus: todo\nmetadata:\n  onetaskgraph.template: by hand\n---\nWritten by hand.\n";
    std::fs::write(plan.notes.join("projects/forged.md"), forged).unwrap();
    let refused = plan.render("notes:forged", &[]);
    assert_eq!(refused.status.code(), Some(1), "{}", stderr(&refused));
    assert!(
        stderr(&refused).contains("records a template entry this product did not write"),
        "{}",
        stderr(&refused)
    );
    assert_eq!(
        std::fs::read_to_string(plan.notes.join("projects/forged.md")).unwrap(),
        forged
    );
}

#[test]
fn every_other_project_write_keeps_its_answers_block_byte_for_byte() {
    let plan = Plan::new();
    let id = plan.create(
        "notes",
        "plan",
        "The plan",
        &["--var", "goal=Kept", "--metadata", "myapp.owner=\"ops\""],
    );
    let held = block(&plan.notes_file(&id)).to_owned();

    // A metadata write.
    plan.exits(&["project", "metadata", "set", &id, "myapp.size", "3"], 0);
    let file = plan.notes_file(&id);
    assert_eq!(
        block(&file),
        held,
        "a metadata write keeps the block:\n{file}"
    );

    // Copied out into a second folder and back over it, unchanged: a copy over it.
    let out = plan.json(&["project", "copy", &id, "--to", "back"]);
    let landed = out["items"][0]["destination"].as_str().unwrap().to_owned();
    let back_file = Plan::project_file(&plan.back, &landed);
    let copied = std::fs::read_to_string(&back_file).unwrap();
    assert!(
        !copied.contains("onetaskgraph:template-answers"),
        "a copy writes no answers:\n{copied}"
    );
    let over = plan.json(&["project", "copy", &landed, "--to", "notes"]);
    assert_eq!(over["items"][0]["destination"], json!(id), "{over}");
    assert!(
        ["updated", "unchanged"].contains(&over["items"][0]["action"].as_str().unwrap()),
        "{over}"
    );
    let file = plan.notes_file(&id);
    assert_eq!(
        block(&file),
        held,
        "a copy over it keeps the block:\n{file}"
    );

    // A copy over it that writes a status.
    std::fs::write(
        &back_file,
        copied.replacen("status: todo", "status: done", 1),
    )
    .unwrap();
    let status = plan.json(&["project", "copy", &landed, "--to", "notes"]);
    assert_eq!(status["items"][0]["action"], "updated", "{status}");
    let file = plan.notes_file(&id);
    assert_eq!(plan.project(&id)["status"]["category"], "done");
    assert_eq!(
        block(&file),
        held,
        "a status write keeps the block:\n{file}"
    );
    assert_eq!(
        plan.json(&["project", "answers", &id]),
        json!({"goal": "Kept", "budgets": [], "owner": null})
    );
    assert_eq!(plan.project(&id)["metadata"]["myapp.size"], json!(3));
}

#[test]
fn a_rendered_project_copies_with_its_content_metadata_and_provenance_and_no_answers() {
    let plan = Plan::with_board(true);
    let id = plan.create(
        "notes",
        "plan",
        "The plan",
        &[
            "--var",
            "goal=Ship it",
            "--var",
            "owner=ANSWER-NEVER-COPIED-ALONE",
            "--metadata",
            "myapp.estimate=3",
        ],
    );
    let authored = plan.project(&id);
    let content = authored["content"].as_str().unwrap().to_owned();

    // Into a second folder: the content, the caller's key and the entry, and no answers block.
    let out = plan.json(&["project", "copy", &id, "--to", "back"]);
    let landed = out["items"][0]["destination"].as_str().unwrap().to_owned();
    let copy = plan.project(&landed);
    assert_eq!(copy["content"], authored["content"]);
    assert_eq!(copy["metadata"]["myapp.estimate"], json!(3));
    assert_eq!(
        copy["metadata"]["onetaskgraph.template"],
        authored["metadata"]["onetaskgraph.template"]
    );
    let file = std::fs::read_to_string(Plan::project_file(&plan.back, &landed)).unwrap();
    assert!(!file.contains("onetaskgraph:template-answers"), "{file}");
    let refused = plan.exits(&["project", "answers", &landed], 1);
    assert!(
        stderr(&refused).contains("has no stored template answers"),
        "{}",
        stderr(&refused)
    );

    // Onto the board: the issue body is the content, then a slot holding the caller's key and
    // the entry, and nothing of the answers but what the template rendered into the content.
    let out = plan.json(&["project", "copy", &id, "--to", "board"]);
    let issue = out["items"][0]["destination"].as_str().unwrap().to_owned();
    let shown = plan.project(&issue);
    assert_eq!(shown["content"], authored["content"]);
    assert_eq!(shown["metadata"]["myapp.estimate"], json!(3));
    assert_eq!(
        shown["metadata"]["onetaskgraph.template"],
        authored["metadata"]["onetaskgraph.template"]
    );
    // llmlint: ignore-block[tests_mirror_real_usage] The criterion is a property of the bytes the board stores for the issue — its body is the rendered content plus a metadata slot holding only the caller's keys and the provenance, and no answer is written anywhere else in it. The binary deliberately never shows that body: `project show` reads the slot back merged with keys the board derives, so no user-facing read can tell what the slot holds or whether an answer landed outside the content. The loopback board is the stand-in for GitHub's own store, read here as a person would read the issue on GitHub.
    let body = plan.board().body(native(&issue));
    let (visible, slot) = slot(body.as_str().expect("an issue body"));
    assert_eq!(visible, content);
    assert!(
        !slot.to_string().contains("goal"),
        "no answer in the slot: {slot}"
    );
    assert_eq!(
        slot["onetaskgraph.template"],
        authored["metadata"]["onetaskgraph.template"]
    );
    // llmlint: ignore-end[tests_mirror_real_usage]
    let refused = plan.exits(&["project", "answers", &issue], 1);
    assert!(
        stderr(&refused).contains("has no stored template answers"),
        "{}",
        stderr(&refused)
    );

    // Regenerated on the board from every required answer: the same issue, its new content and
    // a fresh entry, and nothing else about it.
    let regenerated = plan.json(&[
        "project",
        "render",
        &issue,
        "--template",
        &plan.template(),
        "--search-path",
        &plan.search_path(),
        "--var",
        "goal=On the board",
        "--no-interactive",
    ]);
    assert_eq!(regenerated["changed"], true);
    let after = plan.project(&issue);
    assert!(
        after["content"]
            .as_str()
            .unwrap()
            .contains("Goal: On the board")
    );
    assert_eq!(after["metadata"]["myapp.estimate"], json!(3));
    assert_eq!(after["title"], shown["title"]);
    assert_eq!(
        after["metadata"]["onetaskgraph.template"]["body_digest"],
        json!(sha256(after["content"].as_str().unwrap()))
    );
}

/// About ten kilobytes of JSON under one caller key — a plan's budget answers — with every
/// character a metadata slot has to carry.
fn budget_answers() -> Value {
    let budgets = (0..60)
        .map(|index| {
            json!({
                "issue": format!("plan:T-{index}"),
                "tokens": 120_000 + index,
                "note": format!(
                    "Budget {index}: \"quoted\", back\\slash, <tag> & `tick` --> naïve café — {}",
                    "x".repeat(40)
                ),
            })
        })
        .collect::<Vec<_>>();
    json!({"version": 3, "budgets": budgets})
}

#[test]
fn a_rendered_project_with_ten_kilobytes_of_metadata_copies_into_a_board_and_linear_whole() {
    let plan = Plan::with_hosts(true, true);
    let answers = budget_answers();
    let encoded = answers.to_string();
    assert!(
        (9_500..20_000).contains(&encoded.len()),
        "about ten kilobytes of JSON: {}",
        encoded.len()
    );
    let metadata = format!("onepipeline.budgets={encoded}");
    let id = plan.create(
        "notes",
        "plan",
        "The plan",
        &["--var", "goal=Ship it", "--metadata", &metadata],
    );
    let authored = plan.project(&id);
    assert_eq!(authored["metadata"]["onepipeline.budgets"], answers);

    for destination in ["board", "linear"] {
        let out = plan.json(&["project", "copy", &id, "--to", destination]);
        let landed = out["items"][0]["destination"]
            .as_str()
            .unwrap_or_else(|| panic!("a destination in {destination}: {out}"))
            .to_owned();
        let copy = plan.project(&landed);
        assert_eq!(copy["content"], authored["content"], "{destination}");
        assert_eq!(
            copy["metadata"]["onepipeline.budgets"], answers,
            "{destination} holds the whole value, never cut short"
        );
        assert_eq!(
            copy["metadata"]["onetaskgraph.template"],
            authored["metadata"]["onetaskgraph.template"],
            "{destination} carries the provenance"
        );
        let refused = plan.exits(&["project", "answers", &landed], 1);
        assert!(
            stderr(&refused).contains("has no stored template answers"),
            "{destination} keeps no answers: {}",
            stderr(&refused)
        );
        // A second copy of the same project finds what the first wrote and updates it.
        let again = plan.json(&["project", "copy", &id, "--to", destination]);
        assert_eq!(again["items"][0]["destination"], json!(landed), "{again}");
        assert_eq!(
            plan.project(&landed)["metadata"]["onepipeline.budgets"],
            answers
        );
    }
}
