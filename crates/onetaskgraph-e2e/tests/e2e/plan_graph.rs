//! A plan graph created from one project template, driven through the binary against a folder
//! of Markdown: `project create` from a template declaring `members` makes the project and
//! every member task — each rendered from its own task template with its own answers and
//! recording its provenance as `task create` does — and each member's `depends_on` as a
//! `blocks` edge.
//!
//! A member whose answer its template's schema refuses refuses the whole create, naming the
//! member, and the folder is left byte for byte as it was; `--dry-run` prints the plan and
//! writes nothing; and a project template declaring no `members` creates exactly what it
//! always did.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{document, github_projects_with_board};

/// The project template: three members, the first two from different task templates — the
/// second through a loader document — and the third depending on both.
const PLAN: &str = "---\n\
onetaskgraph_template: 1\n\
variables:\n  \
  slug:\n    \
    description: The plan's short name\n  \
  goal:\n    \
    description: What the plan is for\n  \
  steps:\n    \
    description: The release steps\n    \
    type: list\n    \
    default: [Build, Release]\n\
members:\n  \
  - id: \"{{ slug }}-design\"\n    \
    title: \"Design {{ goal }}\"\n    \
    template: {file: design-task.md}\n    \
    answers: {topic: \"{{ goal }}\"}\n  \
  - id: \"{{ slug }}-measure\"\n    \
    title: \"Measure {{ goal }}\"\n    \
    template: {loader: measure.loader.json}\n    \
    answers: {metric: \"latency of {{ goal }}\"}\n    \
    metadata: {myapp.size: 2}\n  \
  - id: \"{{ slug }}-ship\"\n    \
    title: \"Ship {{ goal }}\"\n    \
    template: {file: design-task.md}\n    \
    answers: {topic: \"shipping {{ goal }}\", steps: \"[{{ steps | join(', ') }}]\"}\n    \
    depends_on: [\"{{ slug }}-design\", \"{{ slug }}-measure\"]\n    \
    repositories: [github.com/acme/widgets]\n\
---\n\
The plan for {{ goal }}.\n";

/// The first task template: a topic and a list of steps whose schema wants each capitalised.
const DESIGN_TASK: &str = "---\n\
onetaskgraph_template: 1\n\
variables:\n  \
  topic:\n    \
    description: What the task covers\n  \
  steps:\n    \
    description: Its steps\n    \
    type: list\n    \
    default: []\n    \
    schema: {type: array, items: {type: string, pattern: \"^[A-Z]\"}}\n\
---\n\
Design: {{ topic }}\n\
{% for step in steps %}\n\
- {{ step }}\n\
{% endfor %}\n";

/// The second task template, which the loader document states.
const MEASURE_TASK: &str = "---\n\
onetaskgraph_template: 1\n\
variables:\n  \
  metric:\n    \
    description: What is measured\n\
---\n\
Measure: {{ metric }}\n";

/// A project template with no members, which creates the project alone.
const BARE: &str = "---\n\
onetaskgraph_template: 1\n\
variables:\n  \
  goal:\n    \
    description: What the plan is for\n\
---\n\
The plan for {{ goal }}.\n";

struct Fixture {
    sandbox: Sandbox,
    notes: PathBuf,
    templates: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let sandbox = Sandbox::new();
        let notes = sandbox.subdirectory("notes");
        let templates = sandbox.subdirectory("templates");
        for (name, text) in [
            ("plan.md", PLAN),
            ("design-task.md", DESIGN_TASK),
            ("measure-task.md", MEASURE_TASK),
            ("bare.md", BARE),
        ] {
            std::fs::write(templates.join(name), text).expect("a template");
        }
        std::fs::write(
            templates.join("measure.loader.json"),
            json!({"reference": "plans/measure", "entry": "measure-task.md",
                   "search_path": [templates]})
            .to_string(),
        )
        .expect("a loader document");
        sandbox.project_document(&document(&json!({
            "notes": {"plugin": "local-md", "config": {"root": notes}},
        })));
        Self {
            sandbox,
            notes,
            templates,
        }
    }

    fn template(&self, name: &str) -> String {
        path(&self.templates.join(name))
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
        serde_json::from_str(&stdout(&self.exits(&arguments, 0))).expect("JSON on stdout")
    }

    fn task(&self, id: &str) -> Value {
        self.json(&["task", "show", id])["items"][0]["item"].clone()
    }

    fn folder(&self) -> BTreeMap<PathBuf, Vec<u8>> {
        let mut found = BTreeMap::new();
        walk(&self.notes, &mut found);
        found
    }
}

fn walk(directory: &Path, found: &mut BTreeMap<PathBuf, Vec<u8>>) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries {
        let path = entry.expect("a directory entry").path();
        if path.is_dir() {
            walk(&path, found);
        } else {
            found.insert(path.clone(), std::fs::read(&path).expect("a file"));
        }
    }
}

fn path(path: &Path) -> String {
    path.to_str().expect("a UTF-8 path").to_owned()
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

/// The arguments of `project create notes --id plan-1 --title …` from `template`, `extra` after.
fn create_arguments(template: &str, extra: &[&str]) -> Vec<String> {
    let mut arguments: Vec<String> = [
        "project",
        "create",
        "notes",
        "--id",
        "plan-1",
        "--title",
        "The widgets plan",
        "--template",
        template,
        "--var",
        "slug=w",
        "--var",
        "goal=widgets",
        "--no-interactive",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    arguments.extend(extra.iter().map(|argument| (*argument).to_owned()));
    arguments
}

fn borrowed(arguments: &[String]) -> Vec<&str> {
    arguments.iter().map(String::as_str).collect()
}

#[test]
fn a_project_template_with_members_creates_the_project_its_tasks_and_their_edges() {
    let fixture = Fixture::new();
    let arguments = create_arguments(&fixture.template("plan.md"), &[]);
    let created = stdout(&fixture.exits(&borrowed(&arguments), 0));
    let ids: Vec<&str> = created.lines().collect();
    assert_eq!(
        ids,
        [
            "notes:plan-1",
            "notes:w-design",
            "notes:w-measure",
            "notes:w-ship"
        ],
        "the project, then each member in the order it was created"
    );

    let project = fixture.json(&["project", "show", "notes:plan-1"])["items"][0]["item"].clone();
    assert_eq!(project["title"], "The widgets plan");
    assert_eq!(project["content"], "The plan for widgets.\n");

    let design_task = fixture.template("design-task.md");
    let expected: [(&str, &str, &str, Value, &str); 3] = [
        (
            "notes:w-design",
            "Design widgets",
            "Design: widgets\n",
            json!({"topic": "widgets", "steps": []}),
            design_task.as_str(),
        ),
        (
            "notes:w-measure",
            "Measure widgets",
            "Measure: latency of widgets\n",
            json!({"metric": "latency of widgets"}),
            "plans/measure",
        ),
        (
            "notes:w-ship",
            "Ship widgets",
            "Design: shipping widgets\n- Build\n- Release\n",
            json!({"topic": "shipping widgets", "steps": ["Build", "Release"]}),
            design_task.as_str(),
        ),
    ];
    for (id, title, content, answers, template) in &expected {
        let task = fixture.task(id);
        assert_eq!(task["title"], *title, "{id}");
        assert_eq!(task["content"], *content, "{id}");
        assert_eq!(task["project"], "plan-1", "{id} is filed under the project");
        // Its answers are stored beside it, as `task create` stores them.
        assert_eq!(&fixture.json(&["task", "answers", id]), answers, "{id}");
        // Its provenance names its own template, and both hashes are its own.
        let provenance = &task["metadata"]["onetaskgraph.template"];
        assert_eq!(provenance["template"], *template, "{id}");
        assert_eq!(provenance["body_digest"], json!(sha256(content)), "{id}");
        assert_eq!(
            provenance["answers_digest"],
            json!(sha256(&canonical(answers))),
            "{id}"
        );
        // So a provenance read accepts it: regenerating it from its recorded template and
        // answers changes nothing.
        // A loader document's reference is never turned into a location, so that member is
        // regenerated from the document named again, as `task create` from one would be.
        let search = path(&fixture.templates);
        let loader = fixture.template("measure.loader.json");
        let mut render = vec!["task", "render", id, "--dry-run", "--no-interactive"];
        if *template == "plans/measure" {
            render.extend(["--template-loader", loader.as_str()]);
        } else {
            render.extend(["--search-path", search.as_str()]);
        }
        let regenerated = fixture.json(&render);
        assert_eq!(regenerated["changed"], false, "{id}: {regenerated}");
    }
    assert_eq!(fixture.task("notes:w-measure")["metadata"]["myapp.size"], 2);
    assert_eq!(
        fixture.task("notes:w-ship")["repositories"],
        json!(["github.com/acme/widgets"])
    );

    // The dependency walk shows exactly the declared edges.
    let edges = |id: &str, direction: &str| -> Vec<(String, String)> {
        fixture.json(&["task", "deps", id, "--direction", direction])["items"]
            .as_array()
            .expect("edges")
            .iter()
            .map(|edge| {
                (
                    edge["from"]["id"].as_str().unwrap().to_owned(),
                    edge["to"]["id"].as_str().unwrap().to_owned(),
                )
            })
            .collect()
    };
    let mut shipping = edges("notes:w-ship", "depends-on");
    shipping.sort();
    assert_eq!(
        shipping,
        [
            ("notes:w-ship".to_owned(), "notes:w-design".to_owned()),
            ("notes:w-ship".to_owned(), "notes:w-measure".to_owned()),
        ]
    );
    assert!(edges("notes:w-design", "depends-on").is_empty());
    assert!(edges("notes:w-measure", "depends-on").is_empty());
    assert_eq!(
        edges("notes:w-design", "depended-on-by"),
        [("notes:w-ship".to_owned(), "notes:w-design".to_owned())]
    );
    let listed = fixture.json(&["task", "list", "--project", "notes:plan-1"])["items"]
        .as_array()
        .expect("tasks")
        .len();
    assert_eq!(
        listed, 3,
        "the project holds its three members and nothing else"
    );
}

#[test]
fn a_member_whose_answer_its_schema_refuses_refuses_the_whole_create_writing_nothing() {
    let fixture = Fixture::new();
    let before = fixture.folder();
    // `steps` reaches the ship member's `steps`, whose schema wants each capitalised.
    let arguments = create_arguments(
        &fixture.template("plan.md"),
        &["--var", "steps=[Build, release]"],
    );
    let output = fixture.exits(&borrowed(&arguments), 2);
    let message = stderr(&output);
    assert!(
        message.contains("member \"w-ship\""),
        "the refusal names the member: {message}"
    );
    assert!(
        message
            .contains("the answer to \"steps\" does not satisfy its schema at JSON pointer \"/1\""),
        "and the variable and pointer: {message}"
    );
    assert!(stdout(&output).is_empty());
    assert_eq!(
        fixture.folder(),
        before,
        "no project and no task of it is written"
    );
    let held = fixture.json(&["task", "list", "--source", "notes"])["items"]
        .as_array()
        .expect("tasks")
        .len();
    assert_eq!(held, 0);
}

#[test]
fn a_dry_run_prints_the_project_and_its_members_and_writes_nothing() {
    let fixture = Fixture::new();
    let before = fixture.folder();
    let arguments = create_arguments(&fixture.template("plan.md"), &["--dry-run", "--json"]);
    let planned: Value =
        serde_json::from_str(&stdout(&fixture.exits(&borrowed(&arguments), 0))).expect("JSON");
    assert_eq!(fixture.folder(), before, "a dry run writes nothing");
    assert_eq!(
        (
            &planned["source"],
            &planned["id"],
            &planned["title"],
            &planned["content"]
        ),
        (
            &json!("notes"),
            &json!("plan-1"),
            &json!("The widgets plan"),
            &json!("The plan for widgets.\n")
        )
    );
    let members = planned["members"].as_array().expect("members");
    let ids: Vec<&str> = members
        .iter()
        .map(|member| member["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["w-design", "w-measure", "w-ship"]);
    assert_eq!(members[2]["depends_on"], json!(["w-design", "w-measure"]));
    assert_eq!(members[1]["template"], "plans/measure");
    assert_eq!(members[1]["content"], "Measure: latency of widgets\n");
    assert_eq!(members[1]["metadata"], json!({"myapp.size": 2}));
    // The text rendering says the same, one line per member.
    let text = create_arguments(&fixture.template("plan.md"), &["--dry-run"]);
    let said = stdout(&fixture.exits(&borrowed(&text), 0));
    assert!(
        said.starts_with("would create project notes:plan-1"),
        "{said}"
    );
    assert!(said.contains("w-ship \"Ship widgets\""), "{said}");
    assert_eq!(fixture.folder(), before);
}

#[test]
fn a_project_template_without_members_creates_the_project_alone_as_before() {
    let fixture = Fixture::new();
    let bare = fixture.template("bare.md");
    let output = fixture.exits(
        &[
            "project",
            "create",
            "notes",
            "--id",
            "plan-1",
            "--title",
            "Bare",
            "--template",
            &bare,
            "--var",
            "goal=widgets",
            "--no-interactive",
        ],
        0,
    );
    assert_eq!(
        stdout(&output),
        "notes:plan-1\n",
        "one line, the project's id, as before"
    );
    let files: Vec<PathBuf> = fixture.folder().into_keys().collect();
    assert_eq!(files, [fixture.notes.join("projects").join("plan-1.md")]);
}

#[test]
fn a_member_graph_that_does_not_hold_together_is_refused_before_anything_is_written() {
    let fixture = Fixture::new();
    let before = fixture.folder();
    for (name, members, says) in [
        (
            "cycle.md",
            "  - {id: a, title: A, template: {file: design-task.md}, answers: {topic: t}, depends_on: [b]}\n  \
               - {id: b, title: B, template: {file: design-task.md}, answers: {topic: t}, depends_on: [a]}\n",
            "cycle",
        ),
        (
            "unknown.md",
            "  - {id: a, title: A, template: {file: design-task.md}, answers: {topic: t}, depends_on: [z]}\n",
            "which no member is",
        ),
        (
            "twice.md",
            "  - {id: a, title: A, template: {file: design-task.md}, answers: {topic: t}}\n  \
               - {id: a, title: B, template: {file: design-task.md}, answers: {topic: t}}\n",
            "name the member \"a\" twice",
        ),
        (
            "itself.md",
            "  - {id: a, title: A, template: {file: design-task.md}, answers: {topic: t}, depends_on: [a]}\n",
            "depend on itself",
        ),
        (
            "empty.md",
            "  - {id: \"{{ '' }}\", title: A, template: {file: design-task.md}, answers: {topic: t}}\n",
            "renders an empty `id`",
        ),
        (
            "missing.md",
            "  - {id: a, title: A, template: {file: no-such.md}, answers: {topic: t}}\n",
            "no-such.md",
        ),
    ] {
        let file = fixture.templates.join(name);
        std::fs::write(
            &file,
            format!("---\nonetaskgraph_template: 1\nmembers:\n{members}---\nPlan\n"),
        )
        .expect("a template");
        let output = fixture.exits(
            &[
                "project",
                "create",
                "notes",
                "--id",
                "plan-1",
                "--title",
                "P",
                "--template",
                &path(&file),
                "--no-interactive",
            ],
            1,
        );
        assert!(
            stderr(&output).contains(says),
            "{name}: {}",
            stderr(&output)
        );
        assert_eq!(fixture.folder(), before, "{name}: nothing written");
    }
}

#[test]
fn members_declared_by_a_template_the_project_template_names_are_refused() {
    let fixture = Fixture::new();
    std::fs::write(
        fixture.templates.join("inner.md"),
        "---\nonetaskgraph_template: 1\nmembers:\n  - {id: a, title: A, template: {file: design-task.md}, answers: {topic: t}}\n---\n{% block body %}Inner{% endblock %}\n",
    )
    .expect("a template");
    let search = path(&fixture.templates);
    // Named by a literal include, by `extends`, and by an include whose name is an expression
    // the answers decide.
    for (name, body, extra) in [
        (
            "included.md",
            "---\nonetaskgraph_template: 1\n---\n{% include \"inner.md\" %}\n",
            None,
        ),
        (
            "extended.md",
            "---\nonetaskgraph_template: 1\n---\n{% extends \"inner.md\" %}\n",
            None,
        ),
        (
            "named.md",
            "---\nonetaskgraph_template: 1\nvariables:\n  part:\n    description: p\n    default: header\n---\n{% include part ~ \".md\" ignore missing %}\n",
            Some("part=inner"),
        ),
    ] {
        let outer = fixture.templates.join(name);
        std::fs::write(&outer, body).expect("a template");
        let before = fixture.folder();
        let outer = path(&outer);
        let mut arguments = vec![
            "project",
            "create",
            "notes",
            "--id",
            "plan-1",
            "--title",
            "P",
            "--template",
            outer.as_str(),
            "--search-path",
            search.as_str(),
            "--no-interactive",
        ];
        if let Some(answer) = extra {
            arguments.extend(["--var", answer]);
        }
        let output = fixture.exits(&arguments, 1);
        let message = stderr(&output);
        assert!(
            message.contains("inner.md") && message.contains("only the rendered project template"),
            "{name}: {message}"
        );
        assert_eq!(fixture.folder(), before, "{name}: nothing written");
    }
}

#[test]
fn a_member_that_cannot_be_written_names_itself_and_what_already_landed() {
    let fixture = Fixture::new();
    // No task can be written: a file stands where the folder keeps its tasks. The project is
    // written first, and lands.
    std::fs::write(fixture.notes.join("tasks"), "not a directory").expect("a file in the way");
    let arguments = create_arguments(&fixture.template("plan.md"), &[]);
    let output = fixture.exits(&borrowed(&arguments), 1);
    let message = stderr(&output);
    assert!(
        message.contains("member w-design of the plan could not be created"),
        "the failure names the first member, the one that failed: {message}"
    );
    assert!(
        message.contains("already created: notes:plan-1\n"),
        "and what landed before it: {message}"
    );
    // What landed is left as it landed.
    let project = fixture.json(&["project", "show", "notes:plan-1"])["items"][0]["item"].clone();
    assert_eq!(project["title"], "The widgets plan");
}

#[test]
fn a_member_with_a_reserved_metadata_key_or_a_malformed_repository_is_refused_naming_it() {
    let fixture = Fixture::new();
    let before = fixture.folder();
    for (name, extra, says) in [
        (
            "reserved.md",
            "    metadata: {onetaskgraph.origin: x}\n",
            "invalid-metadata-key",
        ),
        (
            "repository.md",
            "    repositories: [not a repository]\n",
            "repository not a repository",
        ),
    ] {
        let file = fixture.templates.join(name);
        std::fs::write(
            &file,
            format!(
                "---\nonetaskgraph_template: 1\nmembers:\n  - id: a\n    title: A\n    template: {{file: design-task.md}}\n    answers: {{topic: t}}\n{extra}---\nPlan\n"
            ),
        )
        .expect("a template");
        let output = fixture.exits(
            &[
                "project",
                "create",
                "notes",
                "--id",
                "plan-1",
                "--title",
                "P",
                "--template",
                &path(&file),
                "--no-interactive",
                "--json",
            ],
            1,
        );
        let failure: Value = serde_json::from_str(&stdout(&output)).expect("a failure document");
        let message = failure["failure"]["message"].as_str().unwrap_or_default();
        assert!(
            message.contains("member \"a\" of the project template"),
            "{name}: {message}"
        );
        assert!(
            message.contains(says) || failure["failure"]["kind"] == says,
            "{name}: {failure}"
        );
        assert_eq!(fixture.folder(), before, "{name}: nothing written");
    }
}

#[test]
fn a_project_template_from_a_loader_finds_its_members_beside_the_loader() {
    let fixture = Fixture::new();
    // The loader document sits in a directory of its own, beside the member templates; the
    // project template it states is registered inline.
    let crew = fixture.sandbox.subdirectory("crew");
    std::fs::write(crew.join("design-task.md"), DESIGN_TASK).expect("a member template");
    std::fs::write(crew.join("measure-task.md"), MEASURE_TASK).expect("a member template");
    std::fs::write(
        crew.join("measure.loader.json"),
        json!({"reference": "crew/measure", "entry": "measure-task.md", "search_path": [crew]})
            .to_string(),
    )
    .expect("a loader document");
    let loader = crew.join("plan.loader.json");
    std::fs::write(
        &loader,
        json!({"reference": "crew/plan", "entry": "plan.md", "templates": [{"name": "plan.md", "source": PLAN}]})
            .to_string(),
    )
    .expect("a loader document");
    let output = fixture.exits(
        &[
            "project",
            "create",
            "notes",
            "--id",
            "plan-1",
            "--title",
            "Loaded",
            "--template-loader",
            &path(&loader),
            "--var",
            "slug=w",
            "--var",
            "goal=widgets",
            "--no-interactive",
        ],
        0,
    );
    assert_eq!(stdout(&output).lines().count(), 4, "{}", stdout(&output));
    assert_eq!(
        fixture.task("notes:w-measure")["metadata"]["onetaskgraph.template"]["template"],
        "crew/measure"
    );

    // Read from standard input, a loader has no directory of its own: its search path is where
    // members are found.
    let piped = json!({"reference": "crew/plan", "entry": "plan.md", "search_path": [crew],
        "templates": [{"name": "plan.md", "source": PLAN}]})
    .to_string();
    let output = fixture
        .sandbox
        .command()
        .args([
            "project",
            "create",
            "notes",
            "--id",
            "plan-2",
            "--title",
            "Piped",
            "--template-loader",
            "-",
            "--var",
            "slug=p",
            "--var",
            "goal=gadgets",
            "--no-interactive",
        ])
        .write_stdin(piped)
        .assert()
        .get_output()
        .clone();
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(fixture.task("notes:p-ship")["title"], "Ship gadgets");
}

#[test]
fn on_a_board_that_issues_its_own_ids_each_edge_names_the_id_the_board_gave() {
    let fixture = Fixture::new();
    let (board, _) = github_projects_with_board(&fixture.sandbox);
    fixture.sandbox.project_document(&document(&json!({
        "notes": {"plugin": "local-md", "config": {"root": fixture.notes}},
        "board": {"plugin": "github-projects", "config": board},
    })));
    // The plan without its repository: a board files a member's issue in its own repository.
    let without = PLAN.replace("    repositories: [github.com/acme/widgets]\n", "");
    assert_ne!(without, PLAN);
    std::fs::write(fixture.templates.join("board-plan.md"), without).expect("a template");
    let template = fixture.template("board-plan.md");
    let output = fixture.exits(
        &[
            "project",
            "create",
            "board",
            "--id",
            "plan-1",
            "--title",
            "On the board",
            "--template",
            &template,
            "--var",
            "slug=w",
            "--var",
            "goal=widgets",
            "--no-interactive",
        ],
        0,
    );
    let ids: Vec<String> = stdout(&output).lines().map(str::to_owned).collect();
    assert_eq!(ids.len(), 4, "the project and three members: {ids:?}");
    for member in &ids[1..] {
        assert!(member.starts_with("board:"), "{member}");
        assert!(
            !member.contains("w-"),
            "the board issued {member}, not the offered id"
        );
    }
    let (design, measure, ship) = (&ids[1], &ids[2], &ids[3]);
    assert_eq!(fixture.task(ship)["title"], "Ship widgets");
    let mut depended: Vec<String> = fixture.json(&["task", "deps", ship])["items"]
        .as_array()
        .expect("edges")
        .iter()
        .map(|edge| edge["to"]["id"].as_str().unwrap().to_owned())
        .collect();
    depended.sort();
    let mut expected = vec![design.clone(), measure.clone()];
    expected.sort();
    assert_eq!(depended, expected, "the edges name what the board issued");
}

#[test]
fn a_later_member_the_board_refuses_leaves_the_members_before_it_and_names_them() {
    let fixture = Fixture::new();
    let (board, _) = github_projects_with_board(&fixture.sandbox);
    fixture.sandbox.project_document(&document(&json!({
        "notes": {"plugin": "local-md", "config": {"root": fixture.notes}},
        "board": {"plugin": "github-projects", "config": board},
    })));
    // The ship member names a repository of another owner than the board's project issue,
    // which a board refuses only once the design and measure members have landed.
    let template = fixture.template("plan.md");
    let output = fixture.exits(
        &[
            "project",
            "create",
            "board",
            "--id",
            "plan-1",
            "--title",
            "On the board",
            "--template",
            &template,
            "--var",
            "slug=w",
            "--var",
            "goal=widgets",
            "--no-interactive",
        ],
        1,
    );
    let message = stderr(&output);
    assert!(
        message.contains("member w-ship of the plan could not be created"),
        "{message}"
    );
    let landed = message
        .lines()
        .find_map(|line| line.strip_prefix("already created: "))
        .expect("what landed is named");
    let landed: Vec<&str> = landed.split(", ").collect();
    assert_eq!(
        landed.len(),
        3,
        "the project and the two members before it: {landed:?}"
    );
    for (id, title) in landed[1..]
        .iter()
        .zip(["Design widgets", "Measure widgets"])
    {
        assert_eq!(
            fixture.task(id)["title"],
            title,
            "{id} is left as it landed"
        );
    }
}

#[test]
fn a_structured_member_answer_is_rendered_at_every_depth_and_stored_as_its_value() {
    let fixture = Fixture::new();
    std::fs::write(
        fixture.templates.join("brief.md"),
        "---\nonetaskgraph_template: 1\nvariables:\n  budget:\n    description: b\n    type: object\n    schema: {type: object, required: [id, steps]}\n---\n{{ budget.id }}: {{ budget.steps | join(', ') }}\n",
    )
    .expect("a member template");
    let plan = fixture.templates.join("structured.md");
    std::fs::write(
        &plan,
        "---\nonetaskgraph_template: 1\nvariables:\n  goal: {description: g}\nmembers:\n  - id: brief\n    title: Brief\n    template: {file: brief.md}\n    answers:\n      budget: {id: \"{{ goal }}-seconds\", steps: [\"measure {{ goal }}\", fixed], limit: 7}\n---\nPlan\n",
    )
    .expect("a project template");
    fixture.exits(
        &[
            "project",
            "create",
            "notes",
            "--id",
            "plan-1",
            "--title",
            "P",
            "--template",
            &path(&plan),
            "--var",
            "goal=approval",
            "--no-interactive",
        ],
        0,
    );
    assert_eq!(
        fixture.task("notes:brief")["content"],
        "approval-seconds: measure approval, fixed\n"
    );
    assert_eq!(
        fixture.json(&["task", "answers", "notes:brief"]),
        json!({"budget": {"id": "approval-seconds", "steps": ["measure approval", "fixed"], "limit": 7}})
    );
}
