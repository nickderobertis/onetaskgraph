//! A `linear` source's `metadata_fields`, through the real command-line boundary against the
//! loopback Linear, which stores every long form as the recorded Linear normalisations say: the
//! collapsed `Fields` section each task, project and document shows between its content and its
//! metadata slot, in Linear's own `+++ Fields` form; every read giving back the caller's content
//! and metadata as they were; and each write sending exactly the requests the same write sends
//! with no `metadata_fields` configured.

use std::path::PathBuf;
use std::process::Output;

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{LinearMarkdown, LinearWorkspace, document, linear_workspace_with};

const TEAM_STATES: &[(&str, &str, &str)] = &[
    ("S-ready", "Ready", "unstarted"),
    ("S-working", "Working", "started"),
    ("S-finished", "Finished", "completed"),
    ("S-dropped", "Dropped", "canceled"),
];

const PROJECT_STATUSES: &[(&str, &str)] = &[
    ("Scheduled", "planned"),
    ("Underway", "started"),
    ("Shipped", "completed"),
    ("Abandoned", "canceled"),
];

/// The marker the source sets its section off by.
const MARKER: &str = "<!-- `onetaskgraph.fields` -->";

/// The metadata key every entry reads.
const KEY: &str = "orchestrator.follow-up";

/// The entries every projecting setup configures, in the order the section shows them.
fn projections() -> Value {
    json!([
        {"field": "Host", "key": KEY, "path": ["host"]},
        {"field": "Owning runs", "key": KEY, "path": ["owning_runs"]},
        {"field": "Basis", "key": KEY, "path": ["basis"]},
        {"field": "Notes", "key": KEY, "path": ["notes"]},
    ])
}

/// The follow-up record of the task's contract example: a host, two runs and a basis — with a
/// note of two lines, and its map written out of key order.
fn record() -> Value {
    json!({
        "host": "workstation-1",
        "owning_runs": ["run-a", "run-b"],
        "basis": {
            "github.com/acme/widget": "0123456789abcdef0123456789abcdef01234567",
            "github.com/acme/gadget": "fedcba9876543210fedcba9876543210fedcba98",
        },
        "notes": "first line\nsecond `tick` line",
        "untouched": 7,
    })
}

/// The section [`record`] shows, in Linear's form, as it is sent.
const SECTION: &str = "+++ Fields\n\
\n\
- **Host**: `workstation-1`\n\
- **Owning runs**:\n  - `run-a`\n  - `run-b`\n\
- **Basis**:\n  - `github.com/acme/gadget: fedcba9876543210fedcba9876543210fedcba98`\n  \
- `github.com/acme/widget: 0123456789abcdef0123456789abcdef01234567`\n\
- **Notes**:\n  - `first line`\n  - ``second `tick` line``\n\
\n\
+++";

/// The content every item carries: prose, and a caller's own `Fields` block that is not the
/// source's section.
const CONTENT: &str =
    "Seen twice tonight.\n\n<details>\n<summary>Fields</summary>\n\nMine.\n\n</details>";

/// [`CONTENT`] as it is sent to Linear: its block, as any authored block, a collapsible section.
const CONTENT_SENT: &str = "Seen twice tonight.\n\n+++ Fields\n\nMine.\n\n+++";

/// A folder of Markdown `plan` beside a loopback Linear `linear`, its source configured with
/// [`projections`] when `projecting` says so.
struct Setup {
    sandbox: Sandbox,
    workspace: LinearWorkspace,
    plan: PathBuf,
}

impl Setup {
    fn new(projecting: bool) -> Self {
        Self::configured(projecting.then(projections))
    }

    fn configured(entries: Option<Value>) -> Self {
        Self::mapped(entries, true)
    }

    /// [`Setup::configured`], with an empty `status_mapping` unless `mapped` says so.
    fn mapped(entries: Option<Value>, mapped: bool) -> Self {
        let sandbox = Sandbox::new();
        let held = json!({
            "tasks": [], "documents": [], "labels": [], "projects": [],
            "task_dependencies": [], "project_dependencies": [],
        });
        let (mut config, workspace) =
            linear_workspace_with(&sandbox, held, TEAM_STATES, PROJECT_STATUSES);
        if mapped {
            config["status_mapping"] = json!({
                "todo": {"task": "Ready", "project": "Scheduled"},
                "done": {"task": "Finished", "project": "Shipped"},
            });
        } else {
            config["status_mapping"] = json!({});
        }
        // `bare` reads the same workspace with no `metadata_fields` at all.
        let bare = config.clone();
        if let Some(entries) = entries {
            config["metadata_fields"] = entries;
        }
        workspace.rewrite_markdown_on_write(LinearMarkdown::Measured);
        let plan = sandbox.subdirectory("plan");
        for kind in ["tasks", "projects", "documents"] {
            std::fs::create_dir_all(plan.join(kind)).expect("a folder");
        }
        sandbox.project_document(&document(&json!({
            "linear": {"plugin": "linear", "config": config},
            "bare": {"plugin": "linear", "config": bare},
            "plan": {"plugin": "local-md", "config": {"root": plan}},
        })));
        Self {
            sandbox,
            workspace,
            plan,
        }
    }

    /// Write the plan: the project `goal`, its task `T-1` and its document `design`, each
    /// carrying `record` and [`CONTENT`].
    fn author(&self, record: &Value) {
        let metadata = format!("metadata: {}\n", json!({KEY: record}));
        for (path, front) in [
            ("projects/goal.md", "title: Goal\nstatus: todo\n"),
            (
                "tasks/T-1.md",
                "title: Build it\nstatus: todo\nproject: goal\n",
            ),
            ("documents/design.md", "title: Design\nproject: goal\n"),
        ] {
            std::fs::write(
                self.plan.join(path),
                format!("---\n{front}{metadata}---\n{CONTENT}\n"),
            )
            .expect("an item");
        }
    }

    /// One command that has to succeed, and every request the workspace answered for it.
    fn run(&self, arguments: &[&str]) -> (Output, Vec<(String, Value)>) {
        let from = self.workspace.served().len();
        let output = self
            .sandbox
            .command()
            .args(arguments)
            .assert()
            .get_output()
            .clone();
        assert_eq!(
            output.status.code(),
            Some(0),
            "`onetaskgraph {}`\n{}{}",
            arguments.join(" "),
            stdout(&output),
            stderr(&output)
        );
        (output, self.workspace.served()[from..].to_vec())
    }

    /// Copy the plan to Linear — the project with its task, then the document — answering the
    /// Linear ids of the project, the task and the document, and every request sent.
    fn copy(&self) -> ([String; 3], Vec<(String, Value)>) {
        let (project, mut served) =
            self.run(&["project", "copy", "plan:goal", "--to", "linear", "--json"]);
        let (design, document_served) = self.run(&[
            "document",
            "copy",
            "plan:design",
            "--to",
            "linear",
            "--json",
        ]);
        served.extend(document_served);
        let landed = |output: &Output, source: &str| {
            let report: Value = serde_json::from_str(&stdout(output)).expect("a copy report");
            report["items"]
                .as_array()
                .and_then(|items| items.iter().find(|item| item["source"] == source))
                .and_then(|item| item["destination"].as_str())
                .and_then(|id| id.strip_prefix("linear:"))
                .unwrap_or_else(|| panic!("{source} landed: {report}"))
                .to_owned()
        };
        (
            [
                landed(&project, "plan:goal"),
                landed(&project, "plan:T-1"),
                landed(&design, "plan:design"),
            ],
            served,
        )
    }

    /// One item as `<kind> show --json` reads it.
    fn show(&self, kind: &str, id: &str) -> Value {
        self.show_from("linear", kind, id)
    }

    /// One item as `<kind> show --json` reads it through the source `source`.
    fn show_from(&self, source: &str, kind: &str, id: &str) -> Value {
        let id = format!("{source}:{id}");
        let mut arguments = vec![kind, "show", id.as_str(), "--json"];
        if kind == "task" {
            arguments.push("--no-comments");
        }
        let (output, _) = self.run(&arguments);
        let shown: Value = serde_json::from_str(&stdout(&output)).expect("JSON");
        shown["items"][0]["item"].clone()
    }
}

/// The kinds of the three items [`Setup::copy`] answers, as their verbs and the workspace's
/// tables name them.
const KINDS: [(&str, &str); 3] = [
    ("project", "projects"),
    ("task", "tasks"),
    ("document", "documents"),
];

/// Every long form the requests sent, in order.
fn long_forms_sent(served: &[(String, Value)]) -> Vec<String> {
    fn walk(value: &Value, into: &mut Vec<String>) {
        match value {
            Value::Object(object) => {
                for (key, held) in object {
                    match held {
                        Value::String(body) if key == "description" || key == "content" => {
                            into.push(body.clone());
                        }
                        other => walk(other, into),
                    }
                }
            }
            Value::Array(items) => items.iter().for_each(|item| walk(item, into)),
            _ => {}
        }
    }
    let mut sent = Vec::new();
    for (query, variables) in served {
        if query.trim_start().starts_with("mutation") {
            walk(variables, &mut sent);
        }
    }
    sent
}

/// The updates of a long form among the requests: every mutation updating an issue, a project
/// or a document.
fn updates(served: &[(String, Value)]) -> usize {
    served
        .iter()
        .filter(|(query, _)| {
            query.trim_start().starts_with("mutation")
                && ["issueUpdate", "projectUpdate", "documentUpdate"]
                    .iter()
                    .any(|root| query.contains(root))
        })
        .count()
}

/// What `long_form` holds between its content and its metadata slot.
fn between<'a>(long_form: &'a str, content: &str) -> &'a str {
    let (above, _) = long_form
        .split_once("<!-- onetaskgraph.metadata `")
        .unwrap_or_else(|| panic!("a slot: {long_form}"));
    above
        .strip_prefix(content)
        .unwrap_or_else(|| panic!("the content first: {long_form}"))
}

#[test]
fn every_kind_is_stored_with_its_fields_section_and_read_back_without_it() {
    let setup = Setup::new(true);
    setup.author(&record());
    let (ids, served) = setup.copy();
    // Each item's long form is sent with the section between the content and the slot,
    // exactly as the contract renders it, in Linear's form.
    let sent = long_forms_sent(&served);
    let sectioned: Vec<_> = sent.iter().filter(|form| form.contains(MARKER)).collect();
    assert_eq!(sectioned.len(), 3, "{sent:#?}");
    for form in sectioned {
        assert_eq!(
            between(form, CONTENT_SENT),
            format!("\n\n{MARKER}\n\n{SECTION}\n\n"),
            "{form}"
        );
    }
    for ((kind, table), id) in KINDS.iter().zip(&ids) {
        let stored = setup
            .workspace
            .long_form(table, id)
            .unwrap_or_else(|| panic!("{kind} {id} is held"));
        // Linear keeps the section a collapsible section, its bullets as Linear spells them.
        assert_eq!(
            between(&stored, CONTENT_SENT),
            format!("\n\n{MARKER}\n\n{}\n\n", SECTION.replace("- ", "* ")),
            "{kind}: {stored}"
        );
        // A read is the caller's content — its own Fields block included — and its metadata.
        let shown = setup.show(kind, id);
        assert_eq!(shown["content"], CONTENT, "{kind}: {shown}");
        assert_eq!(shown["metadata"][KEY], record(), "{kind}: {shown}");
        // A source configured with no `metadata_fields` reads the same item the same way.
        assert_eq!(setup.show_from("bare", kind, id), shown, "{kind}");
    }

    // An unchanged repeat sends no update of any long form, though Linear rewrote the section.
    let (_, again) = setup.copy();
    assert_eq!(updates(&again), 0, "{again:#?}");
}

/// The requests one step of a journey sent, under its name.
type Step = (&'static str, Vec<(String, Value)>);

/// Every write a journey makes, from the copy that creates the plan on, of one setup: each
/// step's requests, and the three items' ids.
fn every_write(setup: &Setup) -> (Vec<Step>, [String; 3]) {
    setup.author(&record());
    let (ids, created) = setup.copy();
    let mut steps = vec![("a create", created)];
    steps.push(("an unchanged copy", setup.copy().1));
    let mut moved = record();
    moved["host"] = json!("workstation-2");
    let moved = moved.to_string();
    for (kind, id) in KINDS.iter().map(|(kind, _)| *kind).zip(&ids) {
        let target = format!("linear:{id}");
        steps.push((
            "metadata set",
            setup
                .run(&[kind, "metadata", "set", &target, KEY, &moved, "--json"])
                .1,
        ));
    }
    let task = format!("linear:{}", ids[1]);
    let mut updated = record();
    updated["owning_runs"] = json!(["run-c"]);
    let update = format!("{KEY}={updated}");
    steps.push((
        "task update",
        setup
            .run(&["task", "update", &task, "--metadata", &update, "--json"])
            .1,
    ));
    let comment = setup.plan.join("comment.md");
    std::fs::write(&comment, "Seen again.\n").expect("a comment body");
    steps.push((
        "comment add",
        setup
            .run(&[
                "task",
                "comment",
                "add",
                &task,
                "--body-file",
                comment.to_str().unwrap(),
                "--json",
            ])
            .1,
    ));
    let empty = json!({"host": "", "owning_runs": [], "basis": {}, "notes": null}).to_string();
    steps.push((
        "metadata set, every value empty",
        setup
            .run(&["task", "metadata", "set", &task, KEY, &empty, "--json"])
            .1,
    ));
    (steps, ids)
}

#[test]
fn every_write_sends_the_requests_it_sends_without_metadata_fields_and_keeps_the_section_in_step() {
    let projecting = Setup::new(true);
    let (with, ids) = every_write(&projecting);
    let (without, _) = every_write(&Setup::new(false));
    for ((named, with), (_, without)) in with.iter().zip(&without) {
        assert_eq!(
            with.len(),
            without.len(),
            "{named}: {with:#?}\nagainst\n{without:#?}"
        );
    }
    let step = |named: &str| -> Vec<&Vec<(String, Value)>> {
        with.iter()
            .filter(|(at, _)| *at == named)
            .map(|(_, served)| served)
            .collect()
    };
    assert_eq!(updates(step("an unchanged copy")[0]), 0);
    // A metadata set moving a projected value sends one update, whose long form shows the
    // moved value, and the stored section follows.
    for ((kind, table), (id, served)) in KINDS.iter().zip(ids.iter().zip(step("metadata set"))) {
        assert_eq!(updates(served), 1, "{kind}");
        let sent = long_forms_sent(served);
        assert_eq!(sent.len(), 1, "{kind}: {sent:#?}");
        assert!(
            sent[0].contains(&format!(
                "{MARKER}\n\n+++ Fields\n\n- **Host**: `workstation-2`\n"
            )),
            "{kind}: {}",
            sent[0]
        );
        // The task is written again by the steps after; the project and the document are not.
        if *kind == "task" {
            continue;
        }
        let stored = projecting.workspace.long_form(table, id).unwrap();
        assert!(
            stored.contains("* **Host**: `workstation-2`\n"),
            "{kind}: {stored}"
        );
        let shown = projecting.show(kind, id);
        assert_eq!(shown["content"], CONTENT, "{kind}: {shown}");
        assert_eq!(shown["metadata"][KEY]["host"], "workstation-2", "{kind}");
    }
    let updated = long_forms_sent(step("task update")[0]);
    assert!(
        updated
            .iter()
            .any(|form| form.contains("- **Owning runs**:\n  - `run-c`\n- **Basis**:")),
        "{updated:#?}"
    );
    // A comment carries no slot and gets no section.
    let comment = step("comment add")[0];
    let bodies: Vec<&Value> = comment
        .iter()
        .filter(|(query, _)| query.contains("commentCreate"))
        .map(|(_, variables)| &variables["input"]["body"])
        .collect();
    assert_eq!(bodies, [&json!("Seen again.\n")], "{comment:#?}");
    // Every value empty: no bullet, so no section, and the content and slot as before.
    let stored = projecting.workspace.long_form("tasks", &ids[1]).unwrap();
    assert!(!stored.contains(MARKER), "{stored}");
    assert!(!stored.contains("Fields\n\n* **"), "{stored}");
    assert_eq!(
        between(&stored, CONTENT_SENT),
        "\n\n",
        "the content, a blank line and the slot: {stored}"
    );
    let shown = projecting.show("task", &ids[1]);
    assert_eq!(shown["content"], CONTENT);
}

#[test]
fn every_shape_no_section_can_show_is_refused_before_any_mutation_naming_what_it_found() {
    for (runs, found) in [
        (json!(true), "a boolean"),
        (json!(7), "a number"),
        (json!(["run-a", 2]), "an array whose element 1 is a number"),
        (
            json!(["run-a", ["run-b"]]),
            "an array whose element 1 is an array",
        ),
        (
            json!({"b": "x", "a": {"c": "d"}}),
            "an object whose value at key \"a\" is an object",
        ),
        (
            json!({"a": "x", "b": null}),
            "an object whose value at key \"b\" is null",
        ),
    ] {
        let setup = Setup::new(true);
        std::fs::write(
            setup.plan.join("tasks/T-1.md"),
            format!(
                "---\ntitle: Build it\nstatus: todo\nmetadata: {}\n---\nSeen.\n",
                json!({KEY: {"owning_runs": runs}})
            ),
        )
        .unwrap();
        let output = setup
            .sandbox
            .command()
            .args(["task", "copy", "plan:T-1", "--to", "linear"])
            .assert()
            .failure()
            .get_output()
            .clone();
        let said = stderr(&output);
        for named in [
            "source linear",
            "\"orchestrator.follow-up\"",
            "[\"owning_runs\"]",
            "\"Owning runs\"",
            found,
        ] {
            assert!(said.contains(named), "{runs}: {named}: {said}");
        }
        assert!(
            !setup
                .workspace
                .served()
                .iter()
                .any(|(query, _)| query.trim_start().starts_with("mutation")),
            "{runs}: refused before any mutation"
        );
    }
}

#[test]
fn sources_fields_reports_each_entry_as_carried_in_the_section_and_creates_nothing() {
    let mut entries = projections();
    entries
        .as_array_mut()
        .unwrap()
        .push(json!({"field": "Team", "key": "team.name"}));
    let setup = Setup::configured(Some(entries));
    let (output, served) = setup.run(&["--json", "sources", "fields", "linear", "--apply"]);
    let report: Value = serde_json::from_str(&stdout(&output)).expect("a fields report");
    assert_eq!(
        report["metadata_fields"],
        json!([
            {"field": "Host", "key": KEY, "path": ["host"], "carried": "section"},
            {"field": "Owning runs", "key": KEY, "path": ["owning_runs"], "carried": "section"},
            {"field": "Basis", "key": KEY, "path": ["basis"], "carried": "section"},
            {"field": "Notes", "key": KEY, "path": ["notes"], "carried": "section"},
            {"field": "Team", "key": "team.name", "carried": "section"},
        ])
    );
    assert!(
        !served
            .iter()
            .any(|(query, _)| query.trim_start().starts_with("mutation")),
        "nothing to create"
    );
    let (output, _) = setup.run(&["sources", "fields", "linear"]);
    for line in [
        "linear: Host for orchestrator.follow-up at host is carried in each item's Fields \
         section; nothing to create\n",
        "linear: Team for team.name is carried in each item's Fields section; nothing to create\n",
    ] {
        assert!(stdout(&output).contains(line), "{line}{}", stdout(&output));
    }
}

/// The task `T-1` alone, carrying `content` and `record`, copied to Linear: its Linear id and
/// the long form its create sent.
fn copied_task(setup: &Setup, content: &str, record: &Value) -> (String, String) {
    std::fs::write(
        setup.plan.join("tasks/T-1.md"),
        format!(
            "---\ntitle: Build it\nstatus: todo\nmetadata: {}\n---\n{content}\n",
            json!({KEY: record})
        ),
    )
    .expect("the task");
    let (output, served) = setup.run(&["task", "copy", "plan:T-1", "--to", "linear", "--json"]);
    let report: Value = serde_json::from_str(&stdout(&output)).expect("a copy report");
    let id = report["items"][0]["destination"]
        .as_str()
        .and_then(|id| id.strip_prefix("linear:"))
        .unwrap_or_else(|| panic!("a Linear destination: {report}"))
        .to_owned();
    let sent = long_forms_sent(&served)
        .into_iter()
        .next()
        .expect("the create's long form");
    (id, sent)
}

#[test]
fn every_value_is_one_code_span_whatever_backticks_spaces_or_line_breaks_it_holds() {
    let setup = Setup::new(true);
    let record = json!({
        "host": "`edge`",
        "owning_runs": ["", "two\nlines", "ends with `"],
        "basis": {"k": "v`"},
        "notes": " spaced ",
    });
    let (id, sent) = copied_task(&setup, "Prose.", &record);
    assert_eq!(
        between(&sent, "Prose."),
        format!(
            "\n\n{MARKER}\n\n+++ Fields\n\n\
             - **Host**: `` `edge` ``\n\
             - **Owning runs**:\n  - ` `\n  - `two lines`\n  - `` ends with ` ``\n\
             - **Basis**:\n  - `` k: v` ``\n\
             - **Notes**: `  spaced  `\n\
             \n+++\n\n"
        ),
        "{sent}"
    );
    let shown = setup.show("task", &id);
    assert_eq!(shown["content"], "Prose.");
    assert_eq!(shown["metadata"][KEY], record);
}

#[test]
fn a_caller_quoting_the_marker_keeps_every_byte_of_it_with_a_section_or_without() {
    // The marker quoted inline, and alone on a line before prose rather than a block: neither is
    // a section of the source's, so both read back as written.
    // So is one opening a line with a block's opening after it on that same line.
    let content = format!(
        "Quoting {MARKER} inline.\n\n{MARKER}\n\nJust prose after it.\n\n{MARKER} +++ Fields\n\n\
         More prose."
    );
    let setup = Setup::new(true);
    let (id, sent) = copied_task(&setup, &content, &record());
    assert!(sent.contains("\n\n+++ Fields\n\n- **Host**"), "{sent}");
    assert_eq!(setup.show("task", &id)["content"], content.as_str());
    let empty = json!({"host": ""}).to_string();
    let target = format!("linear:{id}");
    setup.run(&["task", "metadata", "set", &target, KEY, &empty, "--json"]);
    let stored = setup.workspace.long_form("tasks", &id).unwrap();
    assert!(!stored.contains("* **Host**"), "no section: {stored}");
    assert_eq!(
        stored.matches(MARKER).count(),
        3,
        "the caller's three, and no fourth: {stored}"
    );
    assert_eq!(setup.show("task", &id)["content"], content.as_str());
    assert_eq!(
        setup.show_from("bare", "task", &id)["content"],
        content.as_str()
    );
}

#[test]
fn sources_fields_reports_the_section_entries_of_a_source_mapping_no_status() {
    let setup = Setup::mapped(Some(json!([{"field": "Team", "key": "team.name"}])), false);
    let (output, served) = setup.run(&["sources", "fields", "linear", "--apply"]);
    assert_eq!(
        stdout(&output),
        "linear: status_mapping names no status\n\
         linear: Team for team.name is carried in each item's Fields section; nothing to create\n"
    );
    assert!(
        !served
            .iter()
            .any(|(query, _)| query.trim_start().starts_with("mutation")),
        "nothing to create"
    );
}

#[test]
fn a_field_name_holding_markup_is_shown_as_written_and_keeps_the_section_whole() {
    let setup = Setup::configured(Some(json!([
        {"field": "Owner *of* <it>_now_", "key": KEY, "path": ["host"]},
    ])));
    let (id, sent) = copied_task(&setup, "Prose.", &json!({"host": "h"}));
    assert_eq!(
        between(&sent, "Prose."),
        format!(
            "\n\n{MARKER}\n\n+++ Fields\n\n- **Owner \\*of\\* \\<it\\>\\_now\\_**: `h`\n\n+++\n\n"
        ),
        "{sent}"
    );
    assert_eq!(setup.show("task", &id)["content"], "Prose.");
    // A name on two lines could not stand on one bullet's line: refused as the configuration
    // is read, before any request.
    let refused = Setup::configured(Some(json!([{"field": "Ho\nst", "key": KEY}])));
    let output = refused
        .sandbox
        .command()
        .args(["task", "list", "--source", "linear"])
        .assert()
        .failure()
        .get_output()
        .clone();
    assert!(
        stderr(&output).contains("line break"),
        "{}",
        stderr(&output)
    );
    assert!(refused.workspace.served().is_empty());
}

#[test]
fn the_entries_both_sources_refuse_a_linear_source_refuses_at_configuration_read() {
    let host = json!({"field": "Host", "key": KEY, "path": ["host"]});
    for (entries, said) in [
        (json!([{"field": " ", "key": KEY}]), "names a blank field"),
        (json!([{"field": "Host", "key": ""}]), "names a blank key"),
        (
            json!([{"field": "Host", "key": "onetaskgraph.origin"}]),
            "reserved \"onetaskgraph.\" prefix",
        ),
        (
            json!([host, {"field": "HOST", "key": "team.machine"}]),
            "names the same field as metadata_fields[0]",
        ),
    ] {
        let setup = Setup::configured(Some(entries.clone()));
        let output = setup
            .sandbox
            .command()
            .args(["task", "list", "--source", "linear"])
            .assert()
            .failure()
            .get_output()
            .clone();
        let refused = stderr(&output);
        for named in ["source linear", "metadata_fields[", said] {
            assert!(refused.contains(named), "{entries}: {named}: {refused}");
        }
        assert!(setup.workspace.served().is_empty(), "{entries}: no request");
    }
}

#[test]
fn a_section_a_person_edited_is_never_read_and_is_written_afresh_by_the_next_write() {
    let setup = Setup::new(true);
    let (id, _) = copied_task(&setup, "Prose.", &record());
    // A person edits the section's text in Linear, leaving the rest of the description as it is.
    let held = setup.workspace.long_form("tasks", &id).unwrap();
    let edited = held.replace(
        "* **Host**: `workstation-1`",
        "* **Host**: `edited by hand`",
    );
    assert_ne!(edited, held, "the section holds the host: {held}");
    setup.workspace.edit_description(&id, &edited);
    assert!(
        setup
            .workspace
            .long_form("tasks", &id)
            .unwrap()
            .contains("edited by hand"),
        "the person's edit is held"
    );
    // A read is the caller's content and the slot's metadata, as before the edit.
    let shown = setup.show("task", &id);
    assert_eq!(shown["content"], "Prose.");
    assert_eq!(shown["metadata"][KEY], record());
    // An unchanged copy sends no update for it.
    let (_, unchanged) = setup.run(&["task", "copy", "plan:T-1", "--to", "linear", "--json"]);
    assert_eq!(updates(&unchanged), 0, "{unchanged:#?}");
    // The next write of the long form writes the section afresh from the slot.
    let mut moved = record();
    moved["host"] = json!("workstation-2");
    let target = format!("linear:{id}");
    let (_, served) = setup.run(&[
        "task",
        "metadata",
        "set",
        &target,
        KEY,
        &moved.to_string(),
        "--json",
    ]);
    let sent = long_forms_sent(&served);
    assert_eq!(sent.len(), 1, "{sent:#?}");
    assert!(
        sent[0].contains("- **Host**: `workstation-2`") && !sent[0].contains("edited by hand"),
        "{}",
        sent[0]
    );
    let stored = setup.workspace.long_form("tasks", &id).unwrap();
    assert!(!stored.contains("edited by hand"), "{stored}");
    assert_eq!(setup.show("task", &id)["content"], "Prose.");
}
