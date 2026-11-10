//! A template variable of type `object` or `list` holding its answers to the JSON Schema it
//! declares, through every verb that renders: `task create` and `task render`, `project create`
//! and `project render`, `document create` and `document render`, and `template render` —
//! each driven through the binary against a folder of Markdown.
//!
//! A conforming answer renders. A violating one is refused, exit `2`, naming the variable and
//! the JSON pointer of the first violation, and nothing is written: a refused create leaves the
//! folder byte for byte as it was, and a refused render leaves the item's stored content and
//! answers as they were. The same template with its `schema` keys taken out renders the same
//! answers, conforming or not, exactly as it did before schemas existed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::document;

/// A template whose `object` variable and whose `list` variable each declare a schema.
const SCHEMED: &str = "---\n\
onetaskgraph_template: 1\n\
variables:\n  \
  plan:\n    \
    description: The plan record\n    \
    type: object\n    \
    schema:\n      \
      type: object\n      \
      required: [goal, budgets]\n      \
      properties:\n        \
        goal: {type: string, minLength: 1}\n        \
        budgets:\n          \
          type: array\n          \
          items:\n            \
            type: object\n            \
            required: [id, threshold]\n            \
            properties:\n              \
              id: {type: string}\n              \
              threshold: {type: number}\n  \
  steps:\n    \
    description: The steps, each capitalised\n    \
    type: list\n    \
    schema:\n      \
      type: array\n      \
      minItems: 1\n      \
      items: {type: string, pattern: \"^[A-Z]\"}\n\
---\n\
Goal: {{ plan.goal }}\n\
{% for budget in plan.budgets %}\n\
- {{ budget.id }} at most {{ budget.threshold }}\n\
{% endfor %}\n\
{% for step in steps %}\n\
* {{ step }}\n\
{% endfor %}\n";

/// [`SCHEMED`] with no `schema` keys: held to its types alone, as every template was before.
const UNSCHEMED: &str = "---\n\
onetaskgraph_template: 1\n\
variables:\n  \
  plan:\n    \
    description: The plan record\n    \
    type: object\n  \
  steps:\n    \
    description: The steps, each capitalised\n    \
    type: list\n\
---\n\
Goal: {{ plan.goal }}\n\
{% for budget in plan.budgets %}\n\
- {{ budget.id }} at most {{ budget.threshold }}\n\
{% endfor %}\n\
{% for step in steps %}\n\
* {{ step }}\n\
{% endfor %}\n";

/// Answers both schemas accept.
const CONFORMING: &str = "plan:\n  goal: Ship it\n  budgets:\n    - {id: speed, threshold: 7}\n    - {id: cost, threshold: 3}\nsteps: [Draft, Review]\n";

/// What [`CONFORMING`] renders to.
const CONFORMING_BODY: &str =
    "Goal: Ship it\n- speed at most 7\n- cost at most 3\n* Draft\n* Review\n";

/// The object's second budget has no threshold: refused at `/budgets/1`.
const BAD_OBJECT: &str = "plan:\n  goal: Ship it\n  budgets:\n    - {id: speed, threshold: 7}\n    - {id: cost}\nsteps: [Draft, Review]\n";

/// The list's second step is not capitalised: refused at `/1`.
const BAD_LIST: &str = "plan:\n  goal: Ship it\n  budgets:\n    - {id: speed, threshold: 7}\n    - {id: cost, threshold: 3}\nsteps: [Draft, review]\n";

/// What [`BAD_LIST`] renders to with no schema to refuse it.
const BAD_LIST_BODY: &str =
    "Goal: Ship it\n- speed at most 7\n- cost at most 3\n* Draft\n* review\n";

/// One sandbox: a folder of Markdown called `notes`, both templates and the answers files.
struct Fixture {
    sandbox: Sandbox,
    notes: PathBuf,
    inputs: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let sandbox = Sandbox::new();
        let notes = sandbox.subdirectory("notes");
        let inputs = sandbox.subdirectory("inputs");
        for (name, text) in [
            ("schemed.md", SCHEMED),
            ("unschemed.md", UNSCHEMED),
            ("conforming.yaml", CONFORMING),
            ("bad-object.yaml", BAD_OBJECT),
            ("bad-list.yaml", BAD_LIST),
        ] {
            std::fs::write(inputs.join(name), text).expect("an input file");
        }
        sandbox.project_document(&document(&json!({
            "notes": {"plugin": "local-md", "config": {"root": notes}},
        })));
        Self {
            sandbox,
            notes,
            inputs,
        }
    }

    fn input(&self, name: &str) -> String {
        self.inputs
            .join(name)
            .to_str()
            .expect("a UTF-8 path")
            .to_owned()
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

    /// Every file under the folder, by path, with its bytes.
    fn folder(&self) -> BTreeMap<PathBuf, Vec<u8>> {
        let mut found = BTreeMap::new();
        walk(&self.notes, &mut found);
        found
    }

    /// Run `arguments`, which a schema must refuse: exit `2`, the variable and the pointer
    /// named on standard error, nothing on standard output, and the folder as it was.
    fn refused(&self, arguments: &[&str], variable: &str, pointer: &str) {
        let before = self.folder();
        let output = self.exits(arguments, 2);
        let message = stderr(&output);
        assert!(
            message.contains(&format!(
                "the answer to {variable:?} does not satisfy its schema"
            )),
            "the refusal names the variable {variable}: {message}"
        );
        assert!(
            message.contains(&format!("at JSON pointer {pointer:?}")),
            "the refusal names the JSON pointer {pointer}: {message}"
        );
        assert!(stdout(&output).is_empty(), "a refusal answers nothing");
        assert_eq!(self.folder(), before, "a refused write writes nothing");
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

#[test]
fn template_render_holds_each_answer_to_its_variables_schema() {
    let fixture = Fixture::new();
    let schemed = fixture.input("schemed.md");
    let render = |answers: &str| {
        let answers = fixture.input(answers);
        vec![
            "template".to_owned(),
            "render".to_owned(),
            schemed.clone(),
            "--answers".to_owned(),
            answers,
            "--no-interactive".to_owned(),
        ]
    };
    let conforming = render("conforming.yaml");
    let conforming: Vec<&str> = conforming.iter().map(String::as_str).collect();
    assert_eq!(
        stdout(&fixture.exits(&conforming, 0)),
        CONFORMING_BODY,
        "a conforming answer renders"
    );
    for (answers, variable, pointer) in [
        ("bad-object.yaml", "plan", "/budgets/1"),
        ("bad-list.yaml", "steps", "/1"),
    ] {
        let arguments = render(answers);
        let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
        fixture.refused(&arguments, variable, pointer);
    }

    // `template variables` reports each declared schema, so a caller can read what it holds
    // answers to.
    let described = fixture.json(&["template", "variables", &schemed]);
    assert_eq!(described["variables"][0]["name"], "plan");
    assert_eq!(
        described["variables"][0]["schema"]["required"],
        json!(["goal", "budgets"])
    );
    assert_eq!(described["variables"][1]["schema"]["minItems"], 1);
}

#[test]
fn a_template_with_no_schema_renders_every_answer_of_its_type_as_before() {
    let fixture = Fixture::new();
    let unschemed = fixture.input("unschemed.md");
    for (answers, body) in [
        ("conforming.yaml", CONFORMING_BODY),
        ("bad-list.yaml", BAD_LIST_BODY),
    ] {
        let answers = fixture.input(answers);
        let output = fixture.exits(
            &[
                "template",
                "render",
                &unschemed,
                "--answers",
                &answers,
                "--no-interactive",
            ],
            0,
        );
        assert_eq!(stdout(&output), body);
    }
    let described = fixture.json(&["template", "variables", &unschemed]);
    assert!(
        described["variables"]
            .as_array()
            .expect("variables")
            .iter()
            .all(|variable| variable.get("schema").is_none()),
        "no schema is reported where none is declared: {described}"
    );

    // And a task created from it with the answers the schema refuses lands as before.
    let answers = fixture.input("bad-list.yaml");
    let id = stdout(&fixture.exits(
        &[
            "task",
            "create",
            "notes",
            "--project",
            "P-1",
            "--title",
            "Unschemed",
            "--template",
            &unschemed,
            "--answers",
            &answers,
            "--no-interactive",
        ],
        0,
    ))
    .trim()
    .to_owned();
    assert_eq!(Kind::Task.content(&fixture, &id), BAD_LIST_BODY);
}

/// The create and render verbs of one kind of item, as `task`, `project` or `document`.
#[derive(Clone, Copy)]
enum Kind {
    Task,
    Project,
    Document,
}

impl Kind {
    fn noun(self) -> &'static str {
        match self {
            Self::Task => "task",
            Self::Project => "project",
            Self::Document => "document",
        }
    }

    /// `<noun> create notes …` from `template` answered by `answers`.
    fn create(self, template: &str, answers: &str) -> Vec<String> {
        let mut arguments: Vec<String> = vec![self.noun().into(), "create".into(), "notes".into()];
        match self {
            Self::Task => arguments.extend(["--project".into(), "P-1".into()]),
            Self::Project => arguments.extend(["--id".into(), "plan-1".into()]),
            Self::Document => arguments.extend([
                "--project".into(),
                "P-1".into(),
                "--id".into(),
                "design".into(),
            ]),
        }
        arguments.extend([
            "--title".into(),
            "Schemed".into(),
            "--template".into(),
            template.into(),
            "--answers".into(),
            answers.into(),
            "--no-interactive".into(),
        ]);
        arguments
    }

    /// The item's content as `show --json` reads it.
    fn content(self, fixture: &Fixture, id: &str) -> Value {
        fixture.json(&[self.noun(), "show", id])["items"][0]["item"]["content"].clone()
    }
}

/// One kind's whole journey: a create refused for each violation, writing nothing; a
/// conforming create; a render refused for each violation, leaving the stored content and
/// answers as they were; and a conforming render.
fn holds_answers_to_schemas(kind: Kind) {
    let fixture = Fixture::new();
    let schemed = fixture.input("schemed.md");
    let noun = kind.noun();
    for (answers, variable, pointer) in [
        ("bad-object.yaml", "plan", "/budgets/1"),
        ("bad-list.yaml", "steps", "/1"),
    ] {
        let arguments = kind.create(&schemed, &fixture.input(answers));
        let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
        fixture.refused(&arguments, variable, pointer);
    }

    let arguments = kind.create(&schemed, &fixture.input("conforming.yaml"));
    let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
    let id = stdout(&fixture.exits(&arguments, 0)).trim().to_owned();
    assert_eq!(kind.content(&fixture, &id), CONFORMING_BODY);
    let answers = fixture.json(&[noun, "answers", &id]);

    for (file, variable, pointer) in [
        ("bad-object.yaml", "plan", "/budgets/1"),
        ("bad-list.yaml", "steps", "/1"),
    ] {
        let given = fixture.input(file);
        fixture.refused(
            &[noun, "render", &id, "--answers", &given, "--no-interactive"],
            variable,
            pointer,
        );
        assert_eq!(kind.content(&fixture, &id), CONFORMING_BODY);
        assert_eq!(fixture.json(&[noun, "answers", &id]), answers);
    }

    // A conforming render lands.
    let changed = fixture.sandbox.subdirectory("inputs").join("changed.yaml");
    std::fs::write(&changed, CONFORMING.replace("Ship it", "Ship it twice")).expect("answers");
    let changed = changed.to_str().expect("a UTF-8 path").to_owned();
    fixture.exits(
        &[
            noun,
            "render",
            &id,
            "--answers",
            &changed,
            "--no-interactive",
        ],
        0,
    );
    assert_eq!(
        kind.content(&fixture, &id),
        CONFORMING_BODY.replace("Ship it", "Ship it twice")
    );
}

#[test]
fn task_create_and_task_render_hold_answers_to_their_schemas() {
    holds_answers_to_schemas(Kind::Task);
}

#[test]
fn project_create_and_project_render_hold_answers_to_their_schemas() {
    holds_answers_to_schemas(Kind::Project);
}

#[test]
fn document_create_and_document_render_hold_answers_to_their_schemas() {
    holds_answers_to_schemas(Kind::Document);
}

#[test]
fn a_schema_that_does_not_compile_or_types_a_scalar_is_refused_naming_its_key() {
    let fixture = Fixture::new();
    for (name, declaration, key) in [
        (
            "scalar.md",
            "  goal:\n    description: g\n    schema: {minLength: 1}\n",
            "variables.goal.schema",
        ),
        (
            "broken.md",
            "  plan:\n    description: p\n    type: object\n    schema: {type: no-such-type}\n",
            "variables.plan.schema",
        ),
        (
            "remote.md",
            "  plan:\n    description: p\n    type: object\n    schema: {$ref: \"https://example.com/plan.schema.json\"}\n",
            "variables.plan.schema",
        ),
        (
            "default.md",
            "  steps:\n    description: s\n    type: list\n    default: [lower]\n    schema: {items: {pattern: \"^[A-Z]\"}}\n",
            "variables.steps.default",
        ),
    ] {
        let file = fixture.sandbox.subdirectory("inputs").join(name);
        std::fs::write(
            &file,
            format!("---\nonetaskgraph_template: 1\nvariables:\n{declaration}---\nbody\n"),
        )
        .expect("a template");
        let file = file.to_str().expect("a UTF-8 path").to_owned();
        let output = fixture.exits(&["template", "variables", &file], 1);
        assert!(
            stderr(&output).contains(&format!("front matter key `{key}`")),
            "the refusal names {key}: {}",
            stderr(&output)
        );
    }
}

#[test]
fn a_nearer_declaration_gives_its_own_schema_or_none() {
    let fixture = Fixture::new();
    let inputs = fixture.sandbox.subdirectory("inputs");
    // The base declares `steps` with the capitalised-steps schema; one child redeclares it with
    // no schema, another with a schema of its own.
    std::fs::write(
        inputs.join("base.md"),
        "---\nonetaskgraph_template: 1\nvariables:\n  steps:\n    description: s\n    type: list\n    schema: {type: array, items: {type: string, pattern: \"^[A-Z]\"}}\n---\n{% for step in steps %}* {{ step }}\n{% endfor %}",
    )
    .expect("a template");
    std::fs::write(
        inputs.join("loose.md"),
        "---\nonetaskgraph_template: 1\nvariables:\n  steps:\n    description: s\n    type: list\n---\n{% extends \"base.md\" %}\n",
    )
    .expect("a template");
    std::fs::write(
        inputs.join("short.md"),
        "---\nonetaskgraph_template: 1\nvariables:\n  steps:\n    description: s\n    type: list\n    schema: {type: array, maxItems: 1}\n---\n{% extends \"base.md\" %}\n",
    )
    .expect("a template");
    let search = fixture.input("");
    let render = |template: &str, steps: &str| {
        let template = fixture.input(template);
        fixture.run(&[
            "template",
            "render",
            &template,
            "--search-path",
            &search,
            "--var",
            steps,
            "--no-interactive",
        ])
    };
    // The base's own schema refuses a lower-case step.
    assert_eq!(
        render("base.md", "steps=[Draft, review]").status.code(),
        Some(2)
    );
    // Redeclared without one, the same answer renders.
    let loose = render("loose.md", "steps=[Draft, review]");
    assert_eq!(loose.status.code(), Some(0), "{}", stderr(&loose));
    assert_eq!(stdout(&loose), "* Draft\n* review\n");
    // Redeclared with its own, that schema decides: two capitalised steps are refused, at "".
    let short = render("short.md", "steps=[Draft, Review]");
    assert_eq!(short.status.code(), Some(2));
    assert!(
        stderr(&short)
            .contains("the answer to \"steps\" does not satisfy its schema at JSON pointer \"\""),
        "{}",
        stderr(&short)
    );
    assert_eq!(stdout(&render("short.md", "steps=[lower]")), "* lower\n");
}

#[test]
fn a_default_its_schema_accepts_renders_when_no_answer_is_given() {
    let fixture = Fixture::new();
    let file = fixture.sandbox.subdirectory("inputs").join("defaulted.md");
    std::fs::write(
        &file,
        "---\nonetaskgraph_template: 1\nvariables:\n  steps:\n    description: s\n    type: list\n    default: [Draft, Review]\n    schema: {type: array, items: {type: string, pattern: \"^[A-Z]\"}}\n---\n{% for step in steps %}* {{ step }}\n{% endfor %}",
    )
    .expect("a template");
    let file = file.to_str().expect("a UTF-8 path").to_owned();
    let output = fixture.exits(&["template", "render", &file, "--no-interactive"], 0);
    assert_eq!(stdout(&output), "* Draft\n* Review\n");
}

#[test]
fn a_boolean_schema_accepts_every_answer_or_refuses_every_one_writing_nothing() {
    let fixture = Fixture::new();
    let inputs = fixture.sandbox.subdirectory("inputs");
    for (name, schema) in [("open.md", "true"), ("closed.md", "false")] {
        std::fs::write(
            inputs.join(name),
            format!(
                "---\nonetaskgraph_template: 1\nvariables:\n  plan:\n    description: p\n    type: object\n    schema: {schema}\n---\nGoal: {{{{ plan.goal }}}}\n"
            ),
        )
        .expect("a template");
    }
    let open = fixture.input("open.md");
    let closed = fixture.input("closed.md");
    let output = fixture.exits(
        &[
            "template",
            "render",
            &open,
            "--var",
            "plan={goal: anything}",
            "--no-interactive",
        ],
        0,
    );
    assert_eq!(stdout(&output), "Goal: anything\n");
    fixture.refused(
        &[
            "task",
            "create",
            "notes",
            "--project",
            "P-1",
            "--title",
            "Closed",
            "--template",
            &closed,
            "--var",
            "plan={goal: anything}",
            "--no-interactive",
        ],
        "plan",
        "",
    );
}
