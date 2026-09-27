//! `task create`, `document create`, `task render`, `document render`, `task answers` and
//! `document answers`, driven through the binary the way a person or a script drives them.
//!
//! Every journey runs against a folder of Markdown, which keeps the answers an item was
//! rendered from in its own file, and — where the criterion is about a hosted board — the
//! loopback GitHub Projects board, which keeps none. The hashes a created item records are
//! recomputed here from what the binary reads back, never taken from the engine that wrote
//! them.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use crate::common::{Sandbox, SourceBoundary, stderr, stdout};
use crate::fixtures::{GitHubBoardFields, document, github_projects_with_board};
use crate::machine::{bundle, validates};

/// A task template: one required variable, one list with a default, one optional variable it
/// renders and one — `notes` — it never renders, which is where a large answer goes to prove
/// no answer reaches an issue except through the content.
const TASK: &str = "---\n\
onetaskgraph_template: 1\n\
variables:\n  \
  goal:\n    \
    description: What the task is for\n  \
  steps:\n    \
    description: How it is done\n    \
    type: list\n    \
    default: []\n  \
  owner:\n    \
    description: Who does it\n    \
    required: false\n  \
  notes:\n    \
    description: Background this template keeps and never renders\n    \
    type: text\n    \
    required: false\n\
---\n\
{% include \"header.md\" %}\n\
Goal: {{ goal }}\n\
{% for step in steps %}\n\
- {{ step }}\n\
{% endfor %}\n\
{% if owner %}\n\
Owner: {{ owner }}\n\
{% endif %}\n";

const HEADER: &str = "# The task\n";

/// One sandbox holding a folder of Markdown called `notes`, a second one called `back`, and a
/// directory of templates — plus, for the journeys about a hosted board, the loopback board.
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
        let sandbox = Sandbox::new();
        let notes = sandbox.subdirectory("notes");
        let back = sandbox.subdirectory("back");
        let templates = sandbox.subdirectory("templates");
        std::fs::write(templates.join("task.md"), TASK).expect("the template");
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
        path(&self.templates.join("task.md"))
    }

    fn search_path(&self) -> String {
        path(&self.templates)
    }

    fn board(&self) -> &GitHubBoardFields {
        self.board.as_ref().expect("a plan with the board")
    }

    /// A file under the sandbox's project tree, written with `text`.
    fn file(&self, name: &str, text: &str) -> String {
        let file = self.sandbox.subdirectory("inputs").join(name);
        std::fs::write(&file, text).expect("an input file");
        path(&file)
    }

    fn task_file(&self, id: &str) -> PathBuf {
        let native = id.split_once(':').expect("a qualified id").1;
        self.notes.join("tasks").join(format!("{native}.md"))
    }

    fn run(&self, arguments: &[&str]) -> Output {
        self.sandbox
            .command()
            .args(arguments)
            .assert()
            .get_output()
            .clone()
    }

    fn run_with(&self, arguments: &[&str], input: &str) -> Output {
        self.sandbox
            .command()
            .args(arguments)
            .write_stdin(input.to_owned())
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

    /// One task as `task show --json` reads it.
    fn task(&self, id: &str) -> Value {
        self.json(&["task", "show", id])["items"][0]["item"].clone()
    }

    /// Create a task in `source` from the task template, answered by `arguments`.
    fn create(&self, source: &str, title: &str, arguments: &[&str]) -> String {
        let template = self.template();
        let search_path = self.search_path();
        let mut all = vec![
            "task",
            "create",
            source,
            "--project",
            "P-1",
            "--title",
            title,
            "--template",
            &template,
            "--search-path",
            &search_path,
            "--no-interactive",
        ];
        all.extend_from_slice(arguments);
        let output = self.exits(&all, 0);
        stdout(&output).trim().to_owned()
    }

    /// `task render` of a task whose recorded template is the task template: the chain's
    /// search path is not part of what an item records, so it is named again here.
    fn render(&self, id: &str, arguments: &[&str]) -> Output {
        let search_path = self.search_path();
        let mut all = vec![
            "task",
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
        serde_json::from_str(&stdout(&output)).expect("task render --json writes JSON")
    }

    /// What `template render` prints for the task template and `arguments`: the body a fresh
    /// render gives.
    fn fresh(&self, arguments: &[&str]) -> String {
        let template = self.template();
        let search_path = self.search_path();
        let mut all = vec![
            "template",
            "render",
            &template,
            "--search-path",
            &search_path,
            "--no-interactive",
        ];
        all.extend_from_slice(arguments);
        stdout(&self.exits(&all, 0))
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

fn native(id: &str) -> &str {
    id.split_once(':').expect("a qualified id").1
}

#[test]
fn a_task_created_from_a_template_records_its_provenance_and_reads_back_every_field() {
    let plan = Plan::new();
    let answers = plan.file("answers.yaml", "steps: [build, publish]\nowner: ada\n");
    let delivered = plan.create("notes", "Delivered later", &["--var", "goal=Deliver it"]);

    let created = plan.json(&[
        "task",
        "create",
        "notes",
        "--project",
        "P-1",
        "--title",
        "Ship the release",
        "--template",
        &plan.template(),
        "--search-path",
        &plan.search_path(),
        "--answers",
        &answers,
        "--var",
        "goal=Ship it",
        "--status",
        "in-progress",
        "--label",
        "release",
        "--label",
        "urgent",
        "--repository",
        "github.com/acme/ship",
        "--depends-on",
        "notes:first",
        "--delivers",
        &delivered,
        "--metadata",
        "myapp.estimate=3",
        "--metadata",
        "myapp.shape={\"nested\":[true,null]}",
        "--no-interactive",
    ]);
    validates(
        &bundle(&plan.sandbox),
        "TaskDetail",
        &created,
        "task create --json",
    );
    let id = created["items"][0]["id"]
        .as_str()
        .expect("a qualified id")
        .to_owned();
    let task = plan.task(&id);
    assert_eq!(
        created["items"][0]["item"], task,
        "`task show --json`, exactly"
    );

    // Every field the create was given reads back.
    assert_eq!(task["title"], "Ship the release");
    assert_eq!(task["project"], "P-1");
    assert_eq!(task["status"]["category"], "in-progress");
    assert_eq!(
        task["labels"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| l["name"].clone())
            .collect::<Vec<_>>(),
        vec![json!("release"), json!("urgent")]
    );
    assert_eq!(task["repositories"], json!(["github.com/acme/ship"]));
    assert_eq!(task["delivers"], json!([delivered]));
    assert_eq!(task["metadata"]["myapp.estimate"], json!(3));
    assert_eq!(
        task["metadata"]["myapp.shape"],
        json!({"nested": [true, null]})
    );
    let deps = plan.json(&["task", "deps", &id]);
    assert_eq!(deps["items"][0]["to"]["id"], "notes:first");
    // …and the task it delivers is kept in step with it.
    assert_eq!(plan.task(&delivered)["delivered_by"], json!([id]));

    // The content is a fresh render of the answers, and the provenance's hashes are the
    // content's and the resolved answers' own.
    let content = task["content"].as_str().expect("content").to_owned();
    assert_eq!(
        content,
        plan.fresh(&["--answers", &answers, "--var", "goal=Ship it"])
    );
    let provenance = &task["metadata"]["onetaskgraph.template"];
    validates(
        &bundle(&plan.sandbox),
        "TemplateProvenance",
        provenance,
        "the recorded provenance",
    );
    assert_eq!(
        provenance["template"],
        path(&std::path::absolute(plan.templates.join("task.md")).unwrap())
    );
    assert_eq!(provenance["body_digest"], json!(sha256(&content)));
    let stored = plan.json(&["task", "answers", &id]);
    validates(
        &bundle(&plan.sandbox),
        "TemplateAnswers",
        &stored,
        "task answers --json",
    );
    assert_eq!(
        stored,
        json!({"goal": "Ship it", "steps": ["build", "publish"], "owner": "ada", "notes": null}),
        "the resolved answers, defaults applied"
    );
    assert_eq!(
        provenance["answers_digest"],
        json!(sha256(&canonical(&stored)))
    );
    let digest = plan.json(&[
        "template",
        "variables",
        &plan.template(),
        "--search-path",
        &plan.search_path(),
    ])["digest"]
        .clone();
    assert_eq!(provenance["digest"], digest);

    // The answers are in neither the content nor the metadata, and are printed exactly.
    assert!(!content.contains("onetaskgraph:template-answers"));
    assert!(task["metadata"].get("goal").is_none());
    let yaml = stdout(&plan.exits(&["task", "answers", &id], 0));
    assert_eq!(
        yaml,
        "goal: Ship it\nnotes: null\nowner: ada\nsteps:\n- build\n- publish\n"
    );
    assert!(
        std::fs::read_to_string(plan.task_file(&id))
            .unwrap()
            .ends_with(&format!("<!-- onetaskgraph:template-answers\n{yaml}-->\n"))
    );
}

#[test]
fn a_create_whose_delivered_task_cannot_be_kept_in_step_lands_and_exits_four() {
    let plan = Plan::new();
    let output = plan.run(&[
        "task",
        "create",
        "notes",
        "--project",
        "P-1",
        "--title",
        "Delivers elsewhere",
        "--body-file",
        &plan.file("body.md", "x"),
        "--delivers",
        "elsewhere:T-9",
    ]);
    assert_eq!(output.status.code(), Some(4), "{}", stderr(&output));
    let id = stdout(&output).trim().to_owned();
    assert_eq!(
        id, "notes:delivers-elsewhere",
        "the create itself landed and says so"
    );
    assert_eq!(plan.task(&id)["delivers"], json!(["elsewhere:T-9"]));
    assert!(
        stderr(&output).contains(&format!(
            "task elsewhere:T-9 could not be kept in step with {id}"
        )) && stderr(&output).contains("the write itself landed"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn a_task_created_from_a_plain_body_records_no_provenance_and_stores_no_answers() {
    let plan = Plan::new();
    let body = plan.file("body.md", "Written by hand.");
    let from_file = stdout(&plan.exits(
        &[
            "task",
            "create",
            "notes",
            "--project",
            "P-1",
            "--title",
            "By hand",
            "--body-file",
            &body,
        ],
        0,
    ))
    .trim()
    .to_owned();
    let from_stdin = stdout(&plan.run_with(
        &[
            "task",
            "create",
            "notes",
            "--project",
            "P-1",
            "--title",
            "Piped",
        ],
        "Piped in.",
    ))
    .trim()
    .to_owned();

    for (id, content) in [(&from_file, "Written by hand."), (&from_stdin, "Piped in.")] {
        let task = plan.task(id);
        assert_eq!(task["content"], content);
        assert!(
            task["metadata"].get("onetaskgraph.template").is_none(),
            "{task}"
        );
        let refused = plan.exits(&["task", "answers", id], 1);
        assert!(
            stderr(&refused).contains(&format!("task {id} has no stored template answers")),
            "{}",
            stderr(&refused)
        );
    }
}

#[test]
fn a_long_template_reference_is_recorded_whole() {
    let plan = Plan::new();
    let reference = "r".repeat(1000);
    let loader = plan.file(
        "loader.json",
        &json!({
            "reference": reference,
            "entry": "task.md",
            "search_path": [plan.templates],
        })
        .to_string(),
    );
    let id = stdout(&plan.exits(
        &[
            "task",
            "create",
            "notes",
            "--project",
            "P-1",
            "--title",
            "Long",
            "--template-loader",
            &loader,
            "--var",
            "goal=Record it",
            "--no-interactive",
        ],
        0,
    ))
    .trim()
    .to_owned();
    assert_eq!(
        plan.task(&id)["metadata"]["onetaskgraph.template"]["template"],
        json!(reference)
    );
}

#[test]
fn a_reserved_metadata_key_and_an_unwritable_source_are_refused_and_nothing_is_written() {
    let plan = Plan::new();
    let reserved = plan.exits(
        &[
            "task",
            "create",
            "notes",
            "--project",
            "P-1",
            "--title",
            "Forged",
            "--template",
            &plan.template(),
            "--search-path",
            &plan.search_path(),
            "--var",
            "goal=x",
            "--metadata",
            "onetaskgraph.origin=\"board:T-1\"",
            "--no-interactive",
        ],
        1,
    );
    assert!(
        stderr(&reserved).contains("\"onetaskgraph.origin\"")
            && stderr(&reserved).contains("namespace"),
        "{}",
        stderr(&reserved)
    );
    assert!(!plan.notes.join("tasks").exists(), "nothing written");

    // Each value the binary parses is refused by name before any source is built.
    for (flags, named) in [
        (
            &["--repository", "not a repository"][..],
            "--repository not a repository",
        ),
        (
            &["--metadata", "myapp.when=tomorrow"][..],
            "the value is not JSON",
        ),
        (&["--metadata", "no-namespace=1"][..], "has no namespace"),
    ] {
        let mut arguments = vec![
            "task",
            "create",
            "notes",
            "--project",
            "P-1",
            "--title",
            "Refused",
            "--body-file",
        ];
        let body = plan.file("body.md", "x");
        arguments.push(&body);
        arguments.extend_from_slice(flags);
        let refused = plan.exits(&arguments, 1);
        assert!(
            stderr(&refused).contains(named),
            "{named}: {}",
            stderr(&refused)
        );
    }
    let binary = plan.sandbox.subdirectory("inputs").join("binary.md");
    std::fs::write(&binary, [0xff, 0xfe, 0x00]).unwrap();
    let not_text = plan.exits(
        &[
            "task",
            "create",
            "notes",
            "--project",
            "P-1",
            "--title",
            "Bytes",
            "--body-file",
            &path(&binary),
        ],
        1,
    );
    assert!(
        stderr(&not_text).contains("is not UTF-8 text"),
        "{}",
        stderr(&not_text)
    );
    assert!(!plan.notes.join("tasks").exists(), "nothing written");

    let elsewhere = plan.exits(
        &[
            "task",
            "create",
            "notes",
            "--project",
            "back:P-1",
            "--title",
            "Misfiled",
            "--body-file",
            &plan.file("body.md", "x"),
        ],
        1,
    );
    assert!(
        stderr(&elsewhere).contains("that project is in source back"),
        "{}",
        stderr(&elsewhere)
    );
    assert!(!plan.notes.join("tasks").exists(), "nothing written");
    let own = stdout(&plan.exits(
        &[
            "task",
            "create",
            "notes",
            "--project",
            "notes:P-1",
            "--title",
            "Filed",
            "--body-file",
            &plan.file("body.md", "x"),
        ],
        0,
    ))
    .trim()
    .to_owned();
    assert_eq!(
        plan.task(&own)["project"],
        "P-1",
        "its own source's prefix is dropped"
    );

    let unwritable = plan.exits(
        &[
            "task",
            "create",
            "frozen",
            "--project",
            "P-1",
            "--title",
            "Frozen",
            "--body-file",
            &plan.file("body.md", "x"),
        ],
        1,
    );
    assert!(
        stderr(&unwritable).contains("source frozen cannot create a task"),
        "{}",
        stderr(&unwritable)
    );
    assert_eq!(
        plan.json(&["task", "list", "--source", "frozen"])["items"],
        json!([])
    );
    let document = plan.exits(
        &[
            "document",
            "create",
            "frozen",
            "--project",
            "P-1",
            "--title",
            "Frozen",
            "--body-file",
            &plan.file("body.md", "x"),
        ],
        1,
    );
    assert!(
        stderr(&document).contains("source frozen cannot create a document"),
        "{}",
        stderr(&document)
    );
}

#[test]
fn a_source_with_no_documents_or_no_write_side_refuses_before_writing_anything() {
    let sandbox = Sandbox::new();
    let templates = sandbox.subdirectory("templates");
    std::fs::write(templates.join("task.md"), TASK).unwrap();
    std::fs::write(templates.join("header.md"), HEADER).unwrap();
    let digest = format!("sha256:{}", "0".repeat(64));
    let recorded = json!({
        "template": path(&std::path::absolute(templates.join("task.md")).unwrap()),
        "digest": digest, "body_digest": digest, "answers_digest": digest,
    });
    sandbox.project_document(&document(&json!({
        // Writable, and declaring no documents: the in-memory default.
        "plain": {"plugin": "in-memory", "config": {}},
        "frozen": {"plugin": "in-memory", "config": {
            "capabilities": {"writes": "unsupported"},
            "tasks": [{"id": "T-1", "title": "Held", "content": "As it was.",
                       "status": {"category": "todo", "name": "Todo"}, "labels": [],
                       "metadata": {"onetaskgraph.template": recorded}}],
        }},
    })));
    let run = |arguments: &[&str]| {
        sandbox
            .command()
            .args(arguments)
            .assert()
            .get_output()
            .clone()
    };
    let body = sandbox.subdirectory("inputs").join("body.md");
    std::fs::write(&body, "x").unwrap();

    let documentless = run(&[
        "document",
        "create",
        "plain",
        "--project",
        "P-1",
        "--title",
        "Design",
        "--body-file",
        &path(&body),
    ]);
    assert_eq!(
        documentless.status.code(),
        Some(1),
        "{}",
        stderr(&documentless)
    );
    assert!(
        stderr(&documentless).contains("source plain has no documents"),
        "{}",
        stderr(&documentless)
    );

    let unwritable = run(&[
        "task",
        "render",
        "frozen:T-1",
        "--search-path",
        &path(&templates),
        "--var",
        "goal=x",
        "--no-interactive",
    ]);
    assert_eq!(unwritable.status.code(), Some(1), "{}", stderr(&unwritable));
    assert!(
        stderr(&unwritable).contains("source frozen cannot write a task's rendering"),
        "{}",
        stderr(&unwritable)
    );
    // The render itself happened — a dry run is what that source can be asked for.
    let dry = run(&[
        "task",
        "render",
        "frozen:T-1",
        "--search-path",
        &path(&templates),
        "--var",
        "goal=x",
        "--dry-run",
        "--no-interactive",
        "--json",
    ]);
    assert_eq!(dry.status.code(), Some(0), "{}", stderr(&dry));
    let shown: Value =
        serde_json::from_str(&stdout(&run(&["task", "show", "frozen:T-1", "--json"]))).unwrap();
    assert_eq!(shown["items"][0]["item"]["content"], "As it was.");
}

#[test]
fn a_board_task_from_a_template_holds_its_content_and_provenance_and_no_answer() {
    let plan = Plan::with_board(true);
    let notes = format!("BACKGROUND-{}", "n".repeat(30_000));
    let answers = plan.file(
        "big.yaml",
        &format!("notes: {}\nsteps: [build]\n", json!(notes)),
    );
    let id = plan.create(
        "board",
        "Ship on the board",
        &[
            "--answers",
            &answers,
            "--var",
            "goal=Ship it",
            "--metadata",
            "myapp.estimate=3",
            // The repository the board's issue is created in, which the board derives rather
            // than records: every key left in the slot is then the caller's or the
            // provenance, besides the board's own kind marker.
            "--repository",
            "github.com/nickderobertis/onetaskgraph",
        ],
    );

    let task = plan.task(&id);
    let content = task["content"].as_str().unwrap().to_owned();
    // llmlint: ignore-block[tests_mirror_real_usage] The criterion is a property of the bytes the board stores for the issue — its body is the rendered content plus a metadata slot holding only the caller's keys and the provenance, and no answer is written anywhere else in it. The binary deliberately never shows that body: `task show` reads the slot back merged with keys the board derives, so no user-facing read can tell what the slot holds or whether an answer landed outside the content. The loopback board is the stand-in for GitHub's own store, read here as a person would read the issue on GitHub.
    let body = plan.board().body(native(&id));
    let body = body.as_str().expect("an issue body");
    let (visible, slot) = slot(body);
    assert_eq!(visible, content, "the issue body is the rendered content");
    assert_eq!(
        slot.as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        vec![
            "myapp.estimate",
            "onetaskgraph.item_kind",
            "onetaskgraph.template"
        ],
        "the slot holds the caller's keys, the provenance and the board's own kind marker"
    );
    assert_eq!(
        slot["onetaskgraph.template"]["body_digest"],
        json!(sha256(&content))
    );
    assert!(!body.contains("BACKGROUND-"), "no answer reaches the issue");
    // llmlint: ignore-end[tests_mirror_real_usage]
    assert_eq!(
        plan.json(&["task", "comment", "list", &id])["comments"],
        json!([]),
        "and no comment carries one"
    );
    let refused = plan.exits(&["task", "answers", &id], 1);
    assert!(
        stderr(&refused).contains(&format!("task {id} has no stored template answers")),
        "{}",
        stderr(&refused)
    );
    assert_eq!(task["title"], "Ship on the board");
    assert_eq!(task["metadata"]["myapp.estimate"], json!(3));
}

#[test]
fn a_copy_carries_provenance_and_never_answers_and_a_regenerate_updates_the_same_issue() {
    let plan = Plan::with_board(true);
    let notes = format!("BACKGROUND-{}", "n".repeat(30_000));
    let answers = plan.file("big.yaml", &format!("notes: {}\n", json!(notes)));
    let id = plan.create(
        "notes",
        "Authored locally",
        &[
            "--answers",
            &answers,
            "--var",
            "goal=Ship it",
            "--metadata",
            "myapp.estimate=3",
            "--repository",
            "github.com/nickderobertis/onetaskgraph",
        ],
    );
    let authored = plan.task(&id);

    let copied = plan.json(&["task", "copy", &id, "--to", "board"]);
    let issue = copied["items"][0]["destination"]
        .as_str()
        .unwrap()
        .to_owned();
    // llmlint: ignore-block[tests_mirror_real_usage] The criterion is a property of the bytes the board stores for the issue — its body is the rendered content plus a metadata slot holding only the caller's keys and the provenance, and no answer is written anywhere else in it. The binary deliberately never shows that body: `task show` reads the slot back merged with keys the board derives, so no user-facing read can tell what the slot holds or whether an answer landed outside the content. The loopback board is the stand-in for GitHub's own store, read here as a person would read the issue on GitHub.
    let body = plan.board().body(native(&issue));
    let (visible, slot) = slot(body.as_str().unwrap());
    assert_eq!(visible, authored["content"].as_str().unwrap());
    assert_eq!(
        slot["onetaskgraph.template"],
        authored["metadata"]["onetaskgraph.template"]
    );
    assert_eq!(
        slot.as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        vec![
            "myapp.estimate",
            "onetaskgraph.item_kind",
            "onetaskgraph.template"
        ],
        "the caller's keys and the provenance, besides the board's own kind marker"
    );
    assert!(
        !body.as_str().unwrap().contains("BACKGROUND-"),
        "no answer in the issue"
    );
    // llmlint: ignore-end[tests_mirror_real_usage]
    assert_eq!(
        plan.json(&["task", "comment", "list", &issue])["comments"],
        json!([]),
        "no comment carries one"
    );

    // Regenerated locally, then copied again: the same issue, updated.
    plan.rendered(&id, &["--var", "goal=Ship it again"]);
    let again = plan.json(&["task", "copy", &id, "--to", "board"]);
    assert_eq!(again["items"][0]["action"], "updated");
    assert_eq!(again["items"][0]["destination"], json!(issue));
    assert!(
        plan.task(&issue)["content"]
            .as_str()
            .unwrap()
            .contains("Goal: Ship it again")
    );

    // And back out of the board into a second folder: the file there holds no answers.
    let back = plan.json(&["task", "copy", &issue, "--to", "back"]);
    let landed = back["items"][0]["destination"].as_str().unwrap().to_owned();
    let file = std::fs::read_to_string(
        plan.back
            .join("tasks")
            .join(format!("{}.md", native(&landed))),
    )
    .unwrap();
    assert!(!file.contains("onetaskgraph:template-answers"), "{file}");
    assert!(
        file.contains("onetaskgraph.template"),
        "the provenance travels:\n{file}"
    );
    assert_eq!(
        plan.exits(&["task", "answers", &landed], 1).status.code(),
        Some(1)
    );
}

#[test]
fn documents_are_created_on_both_kinds_of_source_and_their_answers_read_where_kept() {
    let plan = Plan::with_board(true);
    for source in ["notes", "board"] {
        let template = plan.template();
        let search_path = plan.search_path();
        let mut arguments = vec![
            "document",
            "create",
            source,
            "--project",
            "P-1",
            "--title",
            "Design",
            "--template",
            &template,
            "--search-path",
            &search_path,
            "--var",
            "goal=Design it",
            "--metadata",
            "myapp.kind=\"design\"",
            "--no-interactive",
        ];
        // A board files what it creates with no labels, and refuses one rather than drop it.
        if source == "notes" {
            arguments.extend(["--label", "design"]);
        }
        let created = plan.json(&arguments);
        validates(
            &bundle(&plan.sandbox),
            "QueryResponseOfQualifiedDocument",
            &created,
            "document create --json",
        );
        let id = created["items"][0]["id"].as_str().unwrap().to_owned();
        let shown = plan.json(&["document", "show", &id]);
        assert_eq!(created, shown, "`document show --json`, exactly");
        let document = &shown["items"][0]["item"];
        assert_eq!(document["title"], "Design");
        assert_eq!(document["project"], "P-1");
        if source == "notes" {
            assert_eq!(document["labels"][0]["name"], "design");
        }
        assert_eq!(document["metadata"]["myapp.kind"], "design");
        let content = document["content"].as_str().unwrap();
        assert_eq!(
            document["metadata"]["onetaskgraph.template"]["body_digest"],
            json!(sha256(content))
        );
        let answers = plan.run(&["document", "answers", &id, "--json"]);
        if source == "notes" {
            assert_eq!(answers.status.code(), Some(0), "{}", stderr(&answers));
            let stored: Value = serde_json::from_str(&stdout(&answers)).unwrap();
            assert_eq!(stored["goal"], "Design it");
        } else {
            assert_eq!(answers.status.code(), Some(1));
            assert!(
                stderr(&answers).contains(&format!("document {id} has no stored template answers"))
            );
        }

        // A dry run reports the change it would make and writes nothing, on either kind.
        let before = plan.json(&["document", "show", &id]);
        let files = snapshot(&plan.notes);
        let dry = plan.json(&[
            "document",
            "render",
            &id,
            "--template",
            &plan.template(),
            "--search-path",
            &plan.search_path(),
            "--var",
            "goal=Design it again",
            "--no-interactive",
            "--dry-run",
        ]);
        assert_eq!(dry["changed"], true, "{dry:#}");
        assert_ne!(
            dry["body"], before["items"][0]["item"]["content"],
            "the dry run rendered the new answer"
        );
        assert_eq!(
            plan.json(&["document", "show", &id]),
            before,
            "a dry run wrote"
        );
        assert_eq!(snapshot(&plan.notes), files, "a dry run changed the folder");

        // Regenerated in place, on either kind.
        let regenerated = plan.json(&[
            "document",
            "render",
            &id,
            "--template",
            &plan.template(),
            "--search-path",
            &plan.search_path(),
            "--var",
            "goal=Design it again",
            "--no-interactive",
        ]);
        validates(
            &bundle(&plan.sandbox),
            "Regenerated",
            &regenerated,
            "document render --json",
        );
        assert_eq!(regenerated["changed"], true);
        let after = plan.json(&["document", "show", &id]);
        assert_eq!(after["items"][0]["item"]["content"], regenerated["body"]);
        assert_eq!(after["items"][0]["id"], json!(id), "the same document");
    }

    // With --id naming a document the folder holds, a create replaces it.
    let body = plan.file("memo.md", "First.");
    let replace = |title: &str| {
        stdout(&plan.exits(
            &[
                "document",
                "create",
                "notes",
                "--project",
                "P-1",
                "--title",
                title,
                "--id",
                "memo",
                "--body-file",
                &body,
            ],
            0,
        ))
        .trim()
        .to_owned()
    };
    assert_eq!(replace("Memo"), "notes:memo");
    assert_eq!(replace("Memo, replaced"), "notes:memo");
    assert_eq!(
        plan.json(&["document", "show", "notes:memo"])["items"][0]["item"]["title"],
        "Memo, replaced"
    );

    // On the board too, under the id the board gave it: the same issue, and no new one.
    let create = |title: &str, id: Option<&str>| {
        let mut arguments = vec![
            "document",
            "create",
            "board",
            "--project",
            "P-1",
            "--title",
            title,
            "--body-file",
            &body,
        ];
        if let Some(id) = id {
            arguments.extend(["--id", id]);
        }
        stdout(&plan.exits(&arguments, 0)).trim().to_owned()
    };
    let first = create("Board memo", None);
    let held = |plan: &Plan| plan.json(&["document", "list", "--source", "board"])["items"].clone();
    let before = held(&plan);
    assert_eq!(create("Board memo, replaced", Some(native(&first))), first);
    assert_eq!(
        held(&plan).as_array().map(Vec::len),
        before.as_array().map(Vec::len),
        "a replacement created a document"
    );
    assert_eq!(
        plan.json(&["document", "show", &first])["items"][0]["item"]["title"],
        "Board memo, replaced"
    );
}

#[test]
fn a_regenerate_overlays_answers_keeps_every_other_field_and_writes_nothing_when_unchanged() {
    let plan = Plan::new();
    let id = plan.create(
        "notes",
        "Regenerated",
        &[
            "--var",
            "goal=Ship it",
            "--var",
            "owner=ada",
            "--var",
            "steps=[build]",
            "--status",
            "queued",
            "--label",
            "release",
            "--metadata",
            "myapp.estimate=3",
        ],
    );
    let before = plan.task(&id);

    // A changed --var: a fresh render of the overlaid answers, and the same chain digest.
    let regenerated = plan.rendered(&id, &["--var", "goal=Ship it twice"]);
    validates(
        &bundle(&plan.sandbox),
        "Regenerated",
        &regenerated,
        "task render --json",
    );
    assert_eq!(regenerated["changed"], true);
    assert_eq!(
        regenerated["body"],
        json!(plan.fresh(&[
            "--var",
            "goal=Ship it twice",
            "--var",
            "owner=ada",
            "--var",
            "steps=[build]"
        ]))
    );
    assert_eq!(
        regenerated["digest"], before["metadata"]["onetaskgraph.template"]["digest"],
        "no chain source changed"
    );
    let after = plan.task(&id);
    assert_eq!(after["content"], regenerated["body"]);
    assert_eq!(
        regenerated["body_digest"],
        json!(sha256(after["content"].as_str().unwrap()))
    );
    for field in [
        "id",
        "title",
        "status",
        "priority",
        "labels",
        "project",
        "repositories",
        "delivers",
        "delivered_by",
        "location",
    ] {
        assert_eq!(after[field], before[field], "{field} is kept");
    }
    assert_eq!(
        after["metadata"]["myapp.estimate"],
        json!(3),
        "other metadata is kept"
    );

    // --unset drops the optional answer.
    let unset = plan.rendered(&id, &["--unset", "owner"]);
    assert!(
        !unset["body"].as_str().unwrap().contains("Owner:"),
        "{unset}"
    );
    assert_eq!(plan.json(&["task", "answers", &id])["owner"], Value::Null);

    // Unsetting a required answer with no default leaves it unanswered: refused, writing nothing.
    let file = std::fs::read(plan.task_file(&id)).unwrap();
    let refused = plan.render(&id, &["--unset", "goal"]);
    assert_eq!(refused.status.code(), Some(2), "{}", stderr(&refused));
    let message = stderr(&refused);
    assert!(
        message.contains("supply every required answer to regenerate")
            && message.contains(
                ": goal unanswered, and the stored answers were used and do not \
                                answer them"
            ),
        "{message}"
    );
    assert_eq!(
        std::fs::read(plan.task_file(&id)).unwrap(),
        file,
        "a refused regenerate writes nothing"
    );

    // Unsetting a name the template does not declare is refused naming it, writing nothing.
    let refused = plan.render(&id, &["--unset", "gaol"]);
    assert_eq!(refused.status.code(), Some(2), "{}", stderr(&refused));
    let message = stderr(&refused);
    assert!(
        message.contains("--unset gaol: the template declares no variable of that name")
            && message.contains("goal, steps, owner, notes"),
        "{message}"
    );
    assert_eq!(
        std::fs::read(plan.task_file(&id)).unwrap(),
        file,
        "a refused unset writes nothing"
    );

    let file = std::fs::read(plan.task_file(&id)).unwrap();
    let unchanged = plan.rendered(&id, &[]);
    assert_eq!(unchanged["changed"], false);
    assert_eq!(
        std::fs::read(plan.task_file(&id)).unwrap(),
        file,
        "byte-identical"
    );

    let dry = plan.rendered(&id, &["--var", "goal=Never written", "--dry-run"]);
    assert_eq!(dry["changed"], true);
    assert!(dry["body"].as_str().unwrap().contains("Never written"));
    assert_eq!(
        std::fs::read(plan.task_file(&id)).unwrap(),
        file,
        "a dry run writes nothing"
    );
    let text = stdout(&plan.render(&id, &["--var", "goal=Never", "--dry-run"]));
    assert!(text.contains("dry run: nothing written"), "{text}");

    // A hand edit is detectable from the provenance, and a regenerate restores the rendering.
    let hand = plan.file("hand.md", "Edited by hand.");
    plan.exits(&["task", "content", "set", &id, "--file", &hand], 0);
    let edited = plan.task(&id);
    assert_ne!(
        edited["metadata"]["onetaskgraph.template"]["body_digest"],
        json!(sha256("Edited by hand."))
    );
    let restored = plan.rendered(&id, &[]);
    assert_eq!(restored["changed"], true);
    assert_eq!(plan.task(&id)["content"], unchanged["body"]);
}

#[test]
fn the_digest_changes_when_and_only_when_a_chain_source_changes() {
    let plan = Plan::new();
    let id = plan.create("notes", "Chained", &["--var", "goal=Ship it"]);
    let recorded = plan.task(&id)["metadata"]["onetaskgraph.template"]["digest"].clone();

    let same = plan.rendered(&id, &["--var", "goal=Other answers"]);
    assert_eq!(same["digest"], recorded, "answers are not a chain source");

    std::fs::write(plan.templates.join("header.md"), "# The task, retitled\n").unwrap();
    let changed = plan.rendered(&id, &[]);
    assert_ne!(changed["digest"], recorded, "an included file changed");
    assert!(
        changed["body"]
            .as_str()
            .unwrap()
            .starts_with("# The task, retitled\n")
    );
}

#[test]
fn answers_out_of_step_with_the_provenance_need_every_required_answer() {
    let plan = Plan::new();
    let id = plan.create(
        "notes",
        "Out of step",
        &["--var", "goal=Ship it", "--var", "owner=ada"],
    );
    let file = plan.task_file(&id);
    let edited = std::fs::read_to_string(&file)
        .unwrap()
        .replace("goal: Ship it", "goal: Forged");
    std::fs::write(&file, &edited).unwrap();

    let refused = plan.render(&id, &["--var", "owner=bob"]);
    assert_eq!(refused.status.code(), Some(2), "{}", stderr(&refused));
    let message = stderr(&refused);
    assert!(
        message.contains("supply every required answer"),
        "{message}"
    );
    assert!(message.contains(": goal unanswered"), "{message}");
    assert!(
        message.contains("answers_digest"),
        "says why the stored answers were not used"
    );
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        edited,
        "nothing written"
    );

    let regenerated = plan.rendered(&id, &["--var", "goal=Ship it"]);
    assert!(
        regenerated["body"]
            .as_str()
            .unwrap()
            .contains("Goal: Ship it")
    );
    assert_eq!(plan.json(&["task", "answers", &id])["goal"], "Ship it");
}

#[test]
fn a_board_item_regenerates_from_every_required_answer_and_keeps_its_issue() {
    let plan = Plan::with_board(true);
    let id = plan.create(
        "board",
        "On the board",
        &[
            "--var",
            "goal=Ship it",
            "--var",
            "owner=ada",
            "--metadata",
            r#"myapp.keep={"n": 1}"#,
        ],
    );
    let listed = |plan: &Plan| {
        plan.json(&["task", "list", "--source", "board"])["items"]
            .as_array()
            .map(Vec::len)
    };
    let before = listed(&plan);

    // A board keeps no answers, so none can be the base.
    let refused = plan.exits(
        &[
            "task",
            "render",
            &id,
            "--template",
            &plan.template(),
            "--search-path",
            &plan.search_path(),
            "--var",
            "owner=bob",
            "--no-interactive",
        ],
        2,
    );
    let message = stderr(&refused);
    assert!(
        message.contains("supply every required answer") && message.contains("goal"),
        "{message}"
    );
    assert!(message.contains("holds no stored answers"), "{message}");

    let regenerated = plan.json(&[
        "task",
        "render",
        &id,
        "--template",
        &plan.template(),
        "--search-path",
        &plan.search_path(),
        "--var",
        "goal=Ship it again",
        "--no-interactive",
    ]);
    assert_eq!(regenerated["id"], json!(id), "the same issue");
    assert_eq!(listed(&plan), before, "no new board item");
    let task = plan.task(&id);
    assert_eq!(task["content"], regenerated["body"]);
    assert_eq!(
        task["metadata"]["onetaskgraph.template"]["body_digest"],
        regenerated["body_digest"]
    );
    // The provenance entry is the one metadata entry a regenerate moves: the caller's own stays.
    assert_eq!(task["metadata"]["myapp.keep"], json!({"n": 1}), "{task:#}");
}

#[test]
fn a_task_with_no_provenance_and_no_template_is_refused() {
    let plan = Plan::new();
    let id = stdout(&plan.exits(
        &[
            "task",
            "create",
            "notes",
            "--project",
            "P-1",
            "--title",
            "Plain",
            "--body-file",
            &plan.file("b.md", "x"),
        ],
        0,
    ))
    .trim()
    .to_owned();
    let refused = plan.render(&id, &[]);
    assert_eq!(refused.status.code(), Some(1));
    assert!(
        stderr(&refused).contains("records no template")
            && stderr(&refused).contains("--template-loader"),
        "{}",
        stderr(&refused)
    );
}

#[test]
fn a_hand_written_answers_block_round_trips_and_every_other_write_keeps_it() {
    let plan = Plan::new();
    let text = "---\ntitle: By hand\nstatus: todo\n---\nThe content.\n\n<!-- onetaskgraph:template-answers\ngoal: By hand\nsteps:\n- one\n-->\n";
    std::fs::create_dir_all(plan.notes.join("tasks")).unwrap();
    std::fs::write(plan.notes.join("tasks/hand.md"), text).unwrap();

    assert_eq!(plan.task("notes:hand")["content"], "The content.");
    assert_eq!(plan.task("notes:hand")["metadata"], json!({}));
    assert_eq!(
        stdout(&plan.exits(&["task", "answers", "notes:hand"], 0)),
        "goal: By hand\nsteps:\n- one\n"
    );
    assert_eq!(
        std::fs::read_to_string(plan.notes.join("tasks/hand.md")).unwrap(),
        text,
        "reads write nothing"
    );

    plan.exits(&["task", "status", "set", "notes:hand", "done"], 0);
    plan.exits(
        &[
            "task",
            "comment",
            "add",
            "notes:hand",
            "--body-file",
            &plan.file("c.md", "A comment."),
        ],
        0,
    );
    let after = std::fs::read_to_string(plan.notes.join("tasks/hand.md")).unwrap();
    assert!(
        after.contains("The content.\n\n<!-- onetaskgraph:template-answers\ngoal: By hand\nsteps:\n- one\n-->\n\n## Comments\n"),
        "{after}"
    );
    assert_eq!(
        plan.json(&["task", "answers", "notes:hand"]),
        json!({"goal": "By hand", "steps": ["one"]})
    );
}

#[test]
fn a_provenance_entry_this_product_did_not_write_names_nothing_until_a_template_is_given() {
    let plan = Plan::new();
    let text = "---\ntitle: Forged\nstatus: todo\nmetadata:\n  onetaskgraph.template: by hand\n---\nWritten by hand.\n";
    std::fs::create_dir_all(plan.notes.join("tasks")).unwrap();
    let file = plan.notes.join("tasks/forged.md");
    std::fs::write(&file, text).unwrap();

    let refused = plan.render("notes:forged", &[]);
    assert_eq!(refused.status.code(), Some(1), "{}", stderr(&refused));
    assert!(
        stderr(&refused).contains("records a template entry this product did not write")
            && stderr(&refused).contains("--template-loader"),
        "{}",
        stderr(&refused)
    );
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        text,
        "nothing written"
    );

    // Given a template and every required answer, it renders, and records a fresh entry.
    let regenerated = plan.rendered(
        "notes:forged",
        &["--template", &plan.template(), "--var", "goal=Reclaimed"],
    );
    assert!(
        regenerated["body"]
            .as_str()
            .unwrap()
            .contains("Goal: Reclaimed")
    );
    assert_eq!(
        plan.task("notes:forged")["metadata"]["onetaskgraph.template"]["body_digest"],
        regenerated["body_digest"]
    );
    // Without every required answer, the refusal says why no stored answers were used.
    std::fs::write(&file, text).unwrap();
    let partial = plan.render("notes:forged", &["--template", &plan.template()]);
    assert_eq!(partial.status.code(), Some(2), "{}", stderr(&partial));
    assert!(
        stderr(&partial).contains("supply every required answer")
            && stderr(&partial).contains("nothing trusted says which stored answers are its"),
        "{}",
        stderr(&partial)
    );
}

#[test]
fn a_malformed_answers_block_is_refused_naming_the_file_and_is_never_rewritten() {
    let plan = Plan::new();
    let text = "---\ntitle: Broken\nstatus: todo\n---\nThe content.\n\n<!-- onetaskgraph:template-answers\ngoal: [unclosed\n-->\n";
    std::fs::create_dir_all(plan.notes.join("tasks")).unwrap();
    let file = plan.notes.join("tasks/broken.md");
    std::fs::write(&file, text).unwrap();

    // The task itself reads: the block is not its content either way.
    assert_eq!(plan.task("notes:broken")["content"], "The content.");
    let refused = plan.exits(&["task", "answers", "notes:broken"], 1);
    assert!(
        stderr(&refused).contains("broken.md: the stored template answers are not a YAML mapping")
            && stderr(&refused).contains("next: correct the block"),
        "{}",
        stderr(&refused)
    );
    let render = plan.render(
        "notes:broken",
        &["--template", &plan.template(), "--var", "goal=x"],
    );
    assert_eq!(render.status.code(), Some(1), "{}", stderr(&render));
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        text,
        "never rewritten"
    );
}

// A directory's and a file's permissions are Unix's to refuse a write with.
#[cfg(unix)]
#[test]
fn a_write_made_to_fail_leaves_the_previous_file_and_no_staging_file() {
    use std::os::unix::fs::PermissionsExt as _;

    let plan = Plan::new();
    let id = plan.create("notes", "Guarded", &["--var", "goal=Ship it"]);
    let tasks = plan.notes.join("tasks");
    let file = plan.task_file(&id);
    let before = std::fs::read(&file).unwrap();
    let listing = || {
        let mut names: Vec<String> = std::fs::read_dir(&tasks)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    };
    let files = listing();

    for (target, mode) in [(&file, 0o444), (&tasks, 0o555)] {
        std::fs::set_permissions(target, std::fs::Permissions::from_mode(mode)).unwrap();
        let refused = plan.render(&id, &["--var", "goal=Never lands"]);
        let create = plan.run(&[
            "task",
            "create",
            "notes",
            "--project",
            "P-1",
            "--title",
            "Never lands",
            "--template",
            &plan.template(),
            "--search-path",
            &plan.search_path(),
            "--var",
            "goal=x",
            "--no-interactive",
        ]);
        let restore = if target.is_dir() { 0o755 } else { 0o644 };
        std::fs::set_permissions(target, std::fs::Permissions::from_mode(restore)).unwrap();

        assert_eq!(refused.status.code(), Some(1), "{}", stderr(&refused));
        assert!(
            stderr(&refused).contains("cannot write"),
            "{}",
            stderr(&refused)
        );
        assert_eq!(std::fs::read(&file).unwrap(), before, "byte-identical");
        if target.is_dir() {
            assert_eq!(create.status.code(), Some(1), "{}", stderr(&create));
        }
        assert_eq!(
            listing()
                .into_iter()
                .filter(|name| !name.starts_with("never-lands"))
                .collect::<Vec<_>>(),
            files,
            "no staging file is left behind"
        );
    }
}

// A document's rendering is written by the same one write; held to the same rule.
#[cfg(unix)]
#[test]
fn a_document_rendering_made_to_fail_leaves_the_previous_file_and_no_staging_file() {
    use std::os::unix::fs::PermissionsExt as _;

    let plan = Plan::new();
    plan.exits(
        &[
            "document",
            "create",
            "notes",
            "--project",
            "P-1",
            "--title",
            "Guarded",
            "--id",
            "guarded",
            "--template",
            &plan.template(),
            "--search-path",
            &plan.search_path(),
            "--var",
            "goal=Design it",
            "--no-interactive",
        ],
        0,
    );
    let documents = plan.notes.join("documents");
    let file = documents.join("guarded.md");
    let before = std::fs::read(&file).unwrap();

    for (target, mode, restore) in [(&file, 0o444, 0o644), (&documents, 0o555, 0o755)] {
        std::fs::set_permissions(target, std::fs::Permissions::from_mode(mode)).unwrap();
        let refused = plan.run(&[
            "document",
            "render",
            "notes:guarded",
            "--search-path",
            &plan.search_path(),
            "--var",
            "goal=Never lands",
            "--no-interactive",
        ]);
        std::fs::set_permissions(target, std::fs::Permissions::from_mode(restore)).unwrap();

        assert_eq!(refused.status.code(), Some(1), "{}", stderr(&refused));
        assert!(
            stderr(&refused).contains("cannot write"),
            "{}",
            stderr(&refused)
        );
        assert_eq!(std::fs::read(&file).unwrap(), before, "byte-identical");
        let names: Vec<String> = std::fs::read_dir(&documents)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            vec!["guarded.md".to_owned()],
            "no staging file is left behind"
        );
    }
}

#[test]
fn a_loader_document_on_standard_input_supplies_the_template() {
    let plan = Plan::new();
    let document = json!({
        "reference": "caller:piped",
        "entry": "task.md",
        "search_path": [plan.templates],
    })
    .to_string();
    let created = plan.run_with(
        &[
            "task",
            "create",
            "notes",
            "--project",
            "P-1",
            "--title",
            "Piped loader",
            "--template-loader",
            "-",
            "--var",
            "goal=From a pipe",
            "--no-interactive",
        ],
        &document,
    );
    assert_eq!(created.status.code(), Some(0), "{}", stderr(&created));
    let id = stdout(&created).trim().to_owned();
    let task = plan.task(&id);
    assert_eq!(
        task["metadata"]["onetaskgraph.template"]["template"],
        "caller:piped"
    );
    assert!(
        task["content"]
            .as_str()
            .unwrap()
            .contains("Goal: From a pipe")
    );

    let rendered = plan.run_with(
        &[
            "task",
            "render",
            &id,
            "--template-loader",
            "-",
            "--var",
            "goal=Piped again",
            "--no-interactive",
            "--json",
        ],
        &document,
    );
    assert_eq!(rendered.status.code(), Some(0), "{}", stderr(&rendered));
    let regenerated: Value = serde_json::from_str(&stdout(&rendered)).unwrap();
    assert!(
        regenerated["body"]
            .as_str()
            .unwrap()
            .contains("Goal: Piped again")
    );
}

#[test]
fn a_loader_document_supplies_the_template_and_its_reference_is_recorded_and_never_resolved() {
    let plan = Plan::new();
    let loader = |base: &str| {
        json!({
            "reference": "$(touch pwned) caller:task/default",
            "entry": "task.md",
            "search_path": [plan.templates],
            "templates": [{"name": "header.md", "source": base}],
            "caller_layers": ["ignored"],
        })
    };
    // The loader's pair is searched after the directory, so the directory's header must go.
    std::fs::remove_file(plan.templates.join("header.md")).unwrap();
    let first = plan.file("loader.json", &loader("# From the loader\n").to_string());
    let id = stdout(&plan.exits(
        &[
            "task",
            "create",
            "notes",
            "--project",
            "P-1",
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
    let task = plan.task(&id);
    assert_eq!(
        task["metadata"]["onetaskgraph.template"]["template"], "$(touch pwned) caller:task/default",
        "the reference, verbatim"
    );
    assert!(
        task["content"]
            .as_str()
            .unwrap()
            .starts_with("# From the loader\n")
    );

    // An edited document is picked up: the changed pair changes the chain.
    let edited = plan.file(
        "edited.json",
        &loader("# From the edited loader\n").to_string(),
    );
    let regenerated = plan.json(&[
        "task",
        "render",
        &id,
        "--template-loader",
        &edited,
        "--no-interactive",
    ]);
    assert_ne!(
        regenerated["digest"],
        task["metadata"]["onetaskgraph.template"]["digest"]
    );
    assert!(
        regenerated["body"]
            .as_str()
            .unwrap()
            .starts_with("# From the edited loader\n")
    );

    // Without a loader, a recorded reference that is not a file is refused, naming it — and
    // nothing was ever run.
    let refused = plan.exits(&["task", "render", &id, "--no-interactive"], 1);
    assert!(
        stderr(&refused).contains("\"$(touch pwned) caller:task/default\"")
            && stderr(&refused).contains("--template-loader"),
        "{}",
        stderr(&refused)
    );
    assert!(!plan.sandbox.project().join("pwned").exists());

    // `template variables` and `template render` take one in place of a file.
    let variables = plan.json(&["template", "variables", "--template-loader", &edited]);
    assert_eq!(variables["digest"], regenerated["digest"]);
    assert_eq!(
        variables["variables"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["name"].clone())
            .collect::<Vec<_>>(),
        vec![
            json!("goal"),
            json!("steps"),
            json!("owner"),
            json!("notes")
        ]
    );
    let rendered = stdout(&plan.exits(
        &[
            "template",
            "render",
            "--template-loader",
            &edited,
            "--var",
            "goal=Ship it",
            "--no-interactive",
        ],
        0,
    ));
    assert!(
        rendered.starts_with("# From the edited loader\nGoal: Ship it\n"),
        "{rendered}"
    );
}

#[test]
fn a_loader_document_that_cannot_be_used_is_refused_by_name_and_writes_nothing() {
    let plan = Plan::new();
    let computed = plan.json(&[
        "template",
        "variables",
        &plan.template(),
        "--search-path",
        &plan.search_path(),
    ])["digest"]
        .as_str()
        .unwrap()
        .to_owned();
    let stated = format!("sha256:{}", "0".repeat(64));
    for (document, named) in [
        ("{not json".to_owned(), "is not JSON".to_owned()),
        (json!({"entry": "task.md"}).to_string(), "`reference` is missing".to_owned()),
        (json!({"reference": "r"}).to_string(), "`entry` is missing".to_owned()),
        (
            json!({"reference": "r", "entry": "task.md", "search_path": [plan.templates.join("missing")]}).to_string(),
            "missing".to_owned(),
        ),
        (
            json!({"reference": "r", "entry": "task.md", "search_path": [plan.templates], "digest": stated}).to_string(),
            format!("states the digest {stated}, and the chain it names computes {computed}"),
        ),
    ] {
        let loader = plan.file("bad.json", &document);
        let refused = plan.exits(
            &["task", "create", "notes", "--project", "P-1", "--title", "Refused", "--template-loader", &loader, "--var", "goal=x", "--no-interactive"],
            1,
        );
        assert!(stderr(&refused).contains(&named), "{document}: {}", stderr(&refused));
        assert!(!plan.notes.join("tasks").exists(), "{document}: nothing written");
    }

    let both = plan.exits(
        &[
            "template",
            "render",
            "--template-loader",
            "-",
            "--answers",
            "-",
            "--no-interactive",
        ],
        2,
    );
    assert!(
        stderr(&both).contains("both read standard input"),
        "{}",
        stderr(&both)
    );
    let create = plan.exits(
        &[
            "task",
            "create",
            "notes",
            "--project",
            "P-1",
            "--title",
            "Both",
            "--template-loader",
            "-",
            "--answers",
            "-",
            "--no-interactive",
        ],
        2,
    );
    assert!(
        stderr(&create).contains("both read standard input"),
        "{}",
        stderr(&create)
    );
    assert!(!plan.notes.join("tasks").exists());
}

/// Every file under `root`, by its path relative to it, with its bytes.
fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, directory: &Path, into: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in std::fs::read_dir(directory).expect("a readable directory") {
            let entry = entry.expect("a readable entry").path();
            if entry.is_dir() {
                walk(root, &entry, into);
            } else {
                let relative = entry.strip_prefix(root).expect("under the root").to_owned();
                into.insert(relative, std::fs::read(&entry).expect("a readable file"));
            }
        }
    }
    let mut files = BTreeMap::new();
    walk(root, root, &mut files);
    files
}

#[test]
fn a_source_behind_the_stdio_protocol_refuses_every_template_operation_by_name() {
    // `hosted` is the very folder `notes` is, reached through the protocol's reference host:
    // what `notes` writes in-process is there for `hosted` to be asked about, and a refusal
    // that wrote anything would show in the one folder both name.
    let plan = Plan::new();
    let notes = path(&plan.notes);
    plan.sandbox.project_document(&document(&json!({
        "notes": {"plugin": "local-md", "config": {"root": notes}},
        "hosted": SourceBoundary::Subprocess.source("local-md", json!({"root": notes})),
    })));
    let template = plan.template();
    let search_path = plan.search_path();
    let refused = |arguments: &[&str], operation: &str| {
        let output = plan.exits(arguments, 1);
        let said = stderr(&output);
        assert!(
            said.contains("source hosted could not do it")
                && said.contains(&format!("does not carry {operation}"))
                && said.contains("stdio plugin protocol")
                && said.contains("nothing was written"),
            "`onetaskgraph {}` did not name the source and the operation:\n{said}",
            arguments.join(" ")
        );
    };

    // Neither create from a template lands anything, not even the content without its answers.
    for (kind, operation) in [
        ("task", "a task create from a template"),
        ("document", "a document create from a template"),
    ] {
        refused(
            &[
                kind,
                "create",
                "hosted",
                "--project",
                "P-1",
                "--title",
                "Hosted",
                "--template",
                &template,
                "--search-path",
                &search_path,
                "--var",
                "goal=Hosted",
                "--no-interactive",
            ],
            operation,
        );
    }
    assert!(
        snapshot(&plan.notes).is_empty(),
        "a refused create wrote {:?}",
        snapshot(&plan.notes).keys().collect::<Vec<_>>()
    );

    // The refusal is about the template operations alone: a plain body still crosses.
    let plain = stdout(&plan.exits(
        &[
            "task",
            "create",
            "hosted",
            "--project",
            "P-1",
            "--title",
            "Plain",
            "--body-file",
            &plan.file("plain.md", "Plain."),
        ],
        0,
    ))
    .trim()
    .to_owned();
    assert_eq!(plain, "hosted:plain");
    let memo = stdout(&plan.exits(
        &[
            "document",
            "create",
            "hosted",
            "--project",
            "P-1",
            "--title",
            "Memo",
            "--body-file",
            &plan.file("memo.md", "Memo."),
        ],
        0,
    ))
    .trim()
    .to_owned();
    assert_eq!(memo, "hosted:memo");
    assert_eq!(
        plan.json(&["document", "show", "notes:memo"])["items"][0]["item"]["content"],
        "Memo."
    );

    // An item rendered in-process, with its provenance and its answers in its file, is refused
    // every read of its answers and every regenerate through the host, a dry run included.
    let task = plan.create("notes", "Rendered", &["--var", "goal=First"]);
    let task = format!("hosted:{}", native(&task));
    let document = plan.json(&[
        "document",
        "create",
        "notes",
        "--project",
        "P-1",
        "--title",
        "Design",
        "--template",
        &template,
        "--search-path",
        &search_path,
        "--var",
        "goal=First",
        "--no-interactive",
    ]);
    let document = format!(
        "hosted:{}",
        native(document["items"][0]["id"].as_str().expect("a document id"))
    );
    let before = snapshot(&plan.notes);
    for (kind, id) in [("task", &task), ("document", &document)] {
        let answers = format!("a {kind}'s stored template answers");
        refused(&[kind, "answers", id], &answers);
        refused(&[kind, "answers", id, "--json"], &answers);
        for dry_run in [&[][..], &["--dry-run"][..]] {
            let mut render = vec![
                kind,
                "render",
                id.as_str(),
                "--template",
                &template,
                "--search-path",
                &search_path,
                "--var",
                "goal=Second",
                "--no-interactive",
            ];
            render.extend_from_slice(dry_run);
            refused(&render, &answers);
        }
    }
    assert_eq!(
        snapshot(&plan.notes),
        before,
        "a refused answers read or regenerate changed the folder"
    );
}

#[test]
fn a_regenerate_writes_back_an_answers_block_deleted_by_hand() {
    let plan = Plan::new();
    let id = plan.create("notes", "Restored", &["--var", "goal=Ship it"]);
    let file = plan.task_file(&id);
    let authored = std::fs::read_to_string(&file).unwrap();
    let open = authored
        .find("<!-- onetaskgraph:template-answers")
        .expect("the file holds its answers block");
    let close = open + authored[open..].find("-->\n").expect("the block closes") + "-->\n".len();
    // The block and the blank lines framing it, so the file is a plain task file and its
    // content reads back unchanged.
    let deleted = format!(
        "{}\n\n{}",
        authored[..open].trim_end_matches('\n'),
        &authored[close..]
    );
    let content = plan.task(&id)["content"].clone();
    std::fs::write(&file, &deleted).unwrap();
    assert_eq!(plan.task(&id)["content"], content, "only the block is gone");
    plan.exits(&["task", "answers", &id], 1);

    // The same answers give the same content and provenance: only the block differs, and a
    // dry run says so without writing it.
    let dry = plan.rendered(&id, &["--var", "goal=Ship it", "--dry-run"]);
    assert_eq!(dry["changed"], true, "{dry:#}");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), deleted);

    let restored = plan.rendered(&id, &["--var", "goal=Ship it"]);
    assert_eq!(restored["changed"], true, "{restored:#}");
    let written = std::fs::read_to_string(&file).unwrap();
    assert!(
        written.ends_with(&authored[open..close]),
        "the block the create wrote is back, byte for byte:\n{written}"
    );
    assert_eq!(plan.json(&["task", "answers", &id])["goal"], "Ship it");
    assert_eq!(plan.task(&id)["content"], "# The task\nGoal: Ship it\n");
}

/// A template whose rendering ends in no newline; [`ENDINGS`] names what each journey appends.
const EXACT: &str = "---\n\
onetaskgraph_template: 1\n\
variables:\n  \
  goal:\n    \
    description: What the task is for\n\
---\n\
# {{ goal }}\n\n- two\n  lines";

/// How a rendering may end: in no newline, in one and in two.
const ENDINGS: [&str; 3] = ["", "\n", "\n\n"];

impl Plan {
    /// The [`EXACT`] template with `ending` after it, written under the templates directory.
    fn exact_template(&self, ending: &str) -> String {
        let file = self.templates.join(format!("exact-{}.md", ending.len()));
        std::fs::write(&file, format!("{EXACT}{ending}")).expect("the template");
        path(&file)
    }

    /// Create a task in `source` from `template`, answering `goal`.
    fn create_exact(&self, source: &str, template: &str, goal: &str) -> String {
        let search_path = self.search_path();
        let answer = format!("goal={goal}");
        stdout(&self.exits(
            &[
                "task",
                "create",
                source,
                "--project",
                "P-1",
                "--title",
                "Exact",
                "--template",
                template,
                "--search-path",
                &search_path,
                "--var",
                &answer,
                "--no-interactive",
            ],
            0,
        ))
        .trim()
        .to_owned()
    }

    /// The two comparisons a drift check makes of the task `id`, rendered from `template` with
    /// `goal`: whether the SHA-256 of the content `task show` returns is the `body_digest` its
    /// provenance records, and whether it is the `body_digest` a dry-run `task render` with the
    /// original answer reports.
    fn drift(&self, id: &str, template: &str, goal: &str) -> (bool, bool, String) {
        let task = self.task(id);
        let content = task["content"].as_str().unwrap_or_default().to_owned();
        let recorded = &task["metadata"]["onetaskgraph.template"]["body_digest"];
        let search_path = self.search_path();
        let answer = format!("goal={goal}");
        let dry = self.json(&[
            "task",
            "render",
            id,
            "--template",
            template,
            "--search-path",
            &search_path,
            "--var",
            &answer,
            "--no-interactive",
            "--dry-run",
        ]);
        let hash = json!(sha256(&content));
        (*recorded == hash, dry["body_digest"] == hash, content)
    }

    /// Assert both drift comparisons report the task `id` as matching its rendering, which is
    /// `expected`.
    fn matches_rendering(&self, id: &str, template: &str, goal: &str, expected: &str, how: &str) {
        let (recorded, rendered, content) = self.drift(id, template, goal);
        assert_eq!(content, expected, "{how}: the content, byte for byte");
        assert!(
            recorded,
            "{how}: `task show` content hashes to the recorded body_digest"
        );
        assert!(
            rendered,
            "{how}: a dry-run render reports the content's hash"
        );
    }

    /// Replace one interior line of the task `id`'s content by hand, and assert both drift
    /// comparisons report it as edited.
    fn edited_by_hand(&self, id: &str, template: &str, goal: &str, how: &str) {
        let content = self.task(id)["content"].as_str().unwrap().to_owned();
        let edited = content.replacen("- two\n", "- three\n", 1);
        assert_ne!(edited, content, "{how}: the interior line is there to edit");
        let hand = self.file("hand-edit.md", &edited);
        self.exits(&["task", "content", "set", id, "--file", &hand], 0);
        let (recorded, rendered, now) = self.drift(id, template, goal);
        assert_eq!(now, edited, "{how}: the edit is stored as written");
        assert!(!recorded, "{how}: the edit no longer hashes to body_digest");
        assert!(!rendered, "{how}: nor to what a render gives");
    }
}

#[test]
fn a_rendering_copied_between_folders_matches_it_and_an_interior_edit_does_not() {
    for ending in ENDINGS {
        for verb in ["task", "project"] {
            let how = format!("{verb} copy of a rendering ending {ending:?}");
            let plan = Plan::new();
            std::fs::create_dir_all(plan.notes.join("projects")).unwrap();
            std::fs::write(
                plan.notes.join("projects/P-1.md"),
                "---\ntitle: Plan\nstatus: todo\n---\nWhy.\n",
            )
            .unwrap();
            let template = plan.exact_template(ending);
            let expected = format!("# Ship\n\n- two\n  lines{ending}");
            let id = plan.create_exact("notes", &template, "Ship");
            plan.matches_rendering(&id, &template, "Ship", &expected, &how);

            match verb {
                "task" => plan.json(&["task", "copy", &id, "--to", "back"]),
                _ => plan.json(&["project", "copy", "notes:P-1", "--to", "back"]),
            };
            let copied = plan.json(&["task", "list", "--source", "back"])["items"]
                .as_array()
                .unwrap()
                .iter()
                .find(|entry| entry["item"]["title"] == "Exact")
                .unwrap_or_else(|| panic!("{how}: the copy is in `back`"))["id"]
                .as_str()
                .unwrap()
                .to_owned();
            plan.matches_rendering(&copied, &template, "Ship", &expected, &how);
            plan.edited_by_hand(&copied, &template, "Ship", &how);
        }
    }
}

#[test]
fn a_rendering_on_the_board_matches_it_through_every_write_and_an_interior_edit_does_not() {
    for ending in ENDINGS {
        let plan = Plan::with_board(true);
        let template = plan.exact_template(ending);
        let expected = |goal: &str| format!("# {goal}\n\n- two\n  lines{ending}");

        // A plain create, from a body file holding the rendering's bytes.
        let body = plan.file("plain.md", &expected("Plain"));
        let plain = stdout(&plan.exits(
            &[
                "task",
                "create",
                "board",
                "--project",
                "P-1",
                "--title",
                "Plain",
                "--body-file",
                &body,
            ],
            0,
        ))
        .trim()
        .to_owned();
        assert_eq!(
            plan.task(&plain)["content"],
            json!(expected("Plain")),
            "plain create ending {ending:?}"
        );

        // A rendered create, then a rendering write over it.
        let rendered = plan.create_exact("board", &template, "Ship");
        let how = format!("rendered create ending {ending:?}");
        plan.matches_rendering(&rendered, &template, "Ship", &expected("Ship"), &how);
        plan.rendered(
            &rendered,
            &["--template", &template, "--var", "goal=Ship again"],
        );
        let how = format!("rendering write ending {ending:?}");
        plan.matches_rendering(
            &rendered,
            &template,
            "Ship again",
            &expected("Ship again"),
            &how,
        );

        // A copy from a folder onto the board, then a copy of a new rendering over it.
        let authored = plan.create_exact("notes", &template, "Copied");
        let copied =
            plan.json(&["task", "copy", &authored, "--to", "board"])["items"][0]["destination"]
                .as_str()
                .unwrap()
                .to_owned();
        let how = format!("copy onto the board ending {ending:?}");
        plan.matches_rendering(&copied, &template, "Copied", &expected("Copied"), &how);
        plan.rendered(&authored, &["--var", "goal=Copied again"]);
        let again = plan.json(&["task", "copy", &authored, "--to", "board"]);
        assert_eq!(again["items"][0]["action"], "updated");
        let how = format!("copy over the board's copy ending {ending:?}");
        plan.matches_rendering(
            &copied,
            &template,
            "Copied again",
            &expected("Copied again"),
            &how,
        );

        // A hand edit of an interior line, through a content write, is an edit; the rendering's
        // own bytes written back the same way match again.
        plan.edited_by_hand(&copied, &template, "Copied again", &how);
        let restored = plan.file("restored.md", &expected("Copied again"));
        plan.exits(&["task", "content", "set", &copied, "--file", &restored], 0);
        let how = format!("content set ending {ending:?}");
        plan.matches_rendering(
            &copied,
            &template,
            "Copied again",
            &expected("Copied again"),
            &how,
        );
    }
}
