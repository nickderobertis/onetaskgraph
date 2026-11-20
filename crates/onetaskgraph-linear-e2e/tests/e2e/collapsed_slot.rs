//! Where a Linear item's metadata lives, through the real binary against the loopback Linear:
//! one collapsed block at the very end of its long form, which nobody reading the item sees until
//! they open it — and every spelling an earlier release wrote, which reads still accept, writes
//! move to the block, and `sources migrate` rewrites.
//!
//! What a person sees is read off the editor state the loopback answers each record with
//! (`linear_editor`), the ProseMirror tree Linear's editor shows, with every collapsed node closed.
//! A slot or a marker written as an HTML comment again would be a visible paragraph of its JSON
//! there, and fails these journeys.

use std::path::PathBuf;
use std::process::Output;

use onetaskgraph_e2e_support::linear_editor;
use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{
    LinearMarkdown, LinearRecord, LinearWorkspace, document, linear_workspace_with,
};

const TEAM_STATES: &[(&str, &str, &str)] = &[
    ("S-ready", "Ready", "unstarted"),
    ("S-finished", "Finished", "completed"),
];

const PROJECT_STATUSES: &[(&str, &str)] = &[("Scheduled", "planned"), ("Shipped", "completed")];

/// A value no reader of an item should see.
const SECRET: &str = "payload-value-7";

/// The caller metadata every item of these journeys carries.
fn caller() -> Value {
    json!({"caller.owner": "nick", "caller.secret": SECRET})
}

/// Every item's content: prose under a heading, as a plan task's is.
const CONTENT: &str = "Seen twice tonight.\n\n## Context\n\nA paragraph a person reads.";

/// The `Fields` section the `fielded` source shows for [`caller`], as it is sent and stored.
const FIELDS: &str = "+++ Fields\n\n* **Owner**: `nick`\n\n+++";

/// The opening of the store's block.
const BLOCK: &str = "+++ onetaskgraph metadata\n\n";

/// A workspace holding `held`, a folder of Markdown `plan`, and three sources over the workspace:
/// `linear`, configured with no `metadata_fields`, `fielded`, showing `caller.owner`, and
/// `scoped`, reading the project `P-in` alone.
struct Setup {
    sandbox: Sandbox,
    workspace: LinearWorkspace,
    plan: PathBuf,
}

impl Setup {
    fn new(held: Value) -> Self {
        let sandbox = Sandbox::new();
        let (mut config, workspace) =
            linear_workspace_with(&sandbox, held, TEAM_STATES, PROJECT_STATUSES);
        config["status_mapping"] = json!({
            "todo": {"task": "Ready", "project": "Scheduled"},
            "done": {"task": "Finished", "project": "Shipped"},
        });
        let mut fielded = config.clone();
        fielded["metadata_fields"] = json!([{"field": "Owner", "key": "caller.owner"}]);
        let mut scoped = config.clone();
        scoped["project"] = json!("P-in");
        workspace.rewrite_markdown_on_write(LinearMarkdown::Measured);
        let plan = sandbox.subdirectory("plan");
        for kind in ["tasks", "projects", "documents"] {
            std::fs::create_dir_all(plan.join(kind)).expect("a folder");
        }
        sandbox.project_document(&document(&json!({
            "linear": {"plugin": "linear", "config": config},
            "fielded": {"plugin": "linear", "config": fielded},
            "scoped": {"plugin": "linear", "config": scoped},
            "plan": {"plugin": "local-md", "config": {"root": plan}},
        })));
        Self {
            sandbox,
            workspace,
            plan,
        }
    }

    fn empty() -> Self {
        Self::new(held(Vec::new(), Vec::new(), Vec::new()))
    }

    /// One command exiting `code`, and every request the workspace answered for it.
    fn exits(&self, arguments: &[&str], code: i32) -> (Output, Vec<(String, Value)>) {
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
            Some(code),
            "`onetaskgraph {}`\n{}{}",
            arguments.join(" "),
            stdout(&output),
            stderr(&output)
        );
        (output, self.workspace.served()[from..].to_vec())
    }

    fn run(&self, arguments: &[&str]) -> (Output, Vec<(String, Value)>) {
        self.exits(arguments, 0)
    }

    /// One command's JSON answer, and its requests.
    fn answer(&self, arguments: &[&str], code: i32) -> (Value, Vec<(String, Value)>) {
        let (output, served) = self.exits(arguments, code);
        let answer = serde_json::from_str(&stdout(&output))
            .unwrap_or_else(|error| panic!("{arguments:?} wrote no JSON ({error})"));
        (answer, served)
    }

    /// One item as `<kind> show --json` reads it through `source`.
    fn show(&self, source: &str, kind: &str, id: &str) -> Value {
        let id = format!("{source}:{id}");
        let mut arguments = vec![kind, "show", id.as_str(), "--json"];
        if kind == "task" {
            arguments.push("--no-comments");
        }
        self.answer(&arguments, 0).0["items"][0]["item"].clone()
    }

    /// The text a person sees of one held record with every collapsed node closed, read off the
    /// editor state the workspace answers it with.
    fn seen(&self, kind: &str, id: &str) -> String {
        let state = self
            .workspace
            .editor_state(record(kind), id)
            .unwrap_or_else(|| panic!("{kind} {id} has an editor state"));
        let tree = linear_editor::decoded_tree(&state).expect("the state decodes");
        linear_editor::closed_text(&tree)
    }

    /// One held record's long form, as the workspace hands it back.
    fn stored(&self, kind: &str, id: &str) -> String {
        self.workspace
            .long_form(collection(kind), id)
            .unwrap_or_else(|| panic!("{kind} {id} is held"))
    }

    /// Write the plan: the project `goal`, its task `T-1` and its document `design`, each with
    /// `content` and `metadata`.
    fn author(&self, content: &str, metadata: &Value) {
        let metadata = format!("metadata: {metadata}\n");
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
                format!("---\n{front}{metadata}---\n{content}\n"),
            )
            .expect("an item");
        }
    }

    /// Copy the plan to `to` — the project with its task, then the document — answering the
    /// destination ids of the project, the task and the document, and every request sent.
    fn copy(&self, to: &str) -> ([String; 3], Vec<(String, Value)>) {
        let (project, mut served) =
            self.answer(&["project", "copy", "plan:goal", "--to", to, "--json"], 0);
        let (design, document_served) = self.answer(
            &["document", "copy", "plan:design", "--to", to, "--json"],
            0,
        );
        served.extend(document_served);
        let prefix = format!("{to}:");
        let landed = |report: &Value, source: &str| {
            report["items"]
                .as_array()
                .and_then(|items| items.iter().find(|item| item["source"] == source))
                .and_then(|item| item["destination"].as_str())
                .and_then(|id| id.strip_prefix(&prefix))
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

    /// A body file holding `text`.
    fn body(&self, name: &str, text: &str) -> String {
        let path = self.sandbox.subdirectory("bodies").join(name);
        std::fs::write(&path, text).expect("a body file");
        path.to_string_lossy().into_owned()
    }
}

/// A workspace holding exactly `tasks`, `projects` and `documents`.
fn held(tasks: Vec<Value>, projects: Vec<Value>, documents: Vec<Value>) -> Value {
    json!({
        "tasks": tasks, "projects": projects, "documents": documents, "labels": [],
        "task_dependencies": [], "project_dependencies": [],
    })
}

/// An issue at `Ready` whose description is stored as `long_form`.
fn issue(id: &str, long_form: &str) -> Value {
    json!({
        "id": id, "title": format!("Issue {id}"), "content": "",
        "status": {"name": "Ready", "category": "todo"},
        "_linear_state": {"name": "Ready", "type": "unstarted"},
        "labels": [], "priority": "none", "_linear_description": long_form,
    })
}

/// A project whose content is stored as `long_form`.
fn project(id: &str, long_form: &str) -> Value {
    json!({
        "id": id, "title": format!("Project {id}"), "content": "",
        "status": {"category": "todo", "name": "Scheduled"}, "labels": [],
        "_linear_description": long_form,
    })
}

/// A document of no project whose content is stored as `long_form`.
fn document_row(id: &str, long_form: &str) -> Value {
    json!({
        "id": id, "title": format!("Document {id}"), "content": "", "labels": [],
        "_linear_description": long_form,
    })
}

fn record(kind: &str) -> LinearRecord {
    match kind {
        "task" => LinearRecord::Task,
        "project" => LinearRecord::Project,
        _ => LinearRecord::Document,
    }
}

fn collection(kind: &str) -> &'static str {
    match kind {
        "task" => "tasks",
        "project" => "projects",
        _ => "documents",
    }
}

/// The kinds of the three items [`Setup::copy`] answers, in its order.
const KINDS: [&str; 3] = ["project", "task", "document"];

/// The root field of every request, in order: `issue`, `issueUpdate`, and so on.
fn operations(served: &[(String, Value)]) -> Vec<String> {
    served
        .iter()
        .map(|(query, _)| {
            query
                .split_once('{')
                .and_then(|(_, rest)| rest.trim_start().split(['(', '{', ' ']).next())
                .unwrap_or_default()
                .to_owned()
        })
        .collect()
}

/// The root field of every mutation, in order.
fn mutations(served: &[(String, Value)]) -> Vec<String> {
    operations(
        &served
            .iter()
            .filter(|(query, _)| query.trim_start().starts_with("mutation"))
            .cloned()
            .collect::<Vec<_>>(),
    )
}

/// How many updates of a long form the requests sent.
fn updates(served: &[(String, Value)]) -> usize {
    mutations(served)
        .iter()
        .filter(|root| ["issueUpdate", "projectUpdate", "documentUpdate"].contains(&root.as_str()))
        .count()
}

/// The metadata a stored long form ends in, holding it to exactly the agreed bytes: `content`, a
/// blank line, the `Fields` section and a blank line when `fields` says so, then the block — its
/// summary line, the sentinel paragraph after a `Fields` section, one code span holding a JSON
/// object with no backtick in it, and `+++` with nothing after it.
fn agreed_tail(stored: &str, content: &str, fields: bool) -> Value {
    let rest = stored
        .strip_prefix(content)
        .and_then(|rest| rest.strip_prefix("\n\n"))
        .unwrap_or_else(|| panic!("the content, then a blank line: {stored:?}"));
    let rest = if fields {
        rest.strip_prefix(&format!("{FIELDS}\n\n"))
            .unwrap_or_else(|| panic!("the Fields section, then a blank line: {stored:?}"))
    } else {
        rest
    };
    let rest = rest
        .strip_prefix(BLOCK)
        .unwrap_or_else(|| panic!("the block's summary line: {stored:?}"));
    let rest = if fields {
        rest.strip_prefix("`onetaskgraph.fields`\n\n")
            .unwrap_or_else(|| panic!("the sentinel paragraph: {stored:?}"))
    } else {
        rest
    };
    let json = rest
        .strip_prefix('`')
        .and_then(|rest| rest.strip_suffix("`\n\n+++"))
        .unwrap_or_else(|| panic!("one code span, then the close and nothing after: {stored:?}"));
    assert!(
        !json.contains(['`', '\n']),
        "one span on one line: {stored:?}"
    );
    assert!(!stored.contains("<!--"), "no HTML comment: {stored:?}");
    let metadata: Value = serde_json::from_str(json).expect("the span holds JSON");
    assert!(metadata.is_object(), "{metadata}");
    metadata
}

/// What a person sees holds the item's own text and none of its metadata: no key, no value, no
/// store-written HTML comment.
fn sees_no_metadata(seen: &str, what: &str) {
    assert!(
        seen.contains("A paragraph a person reads."),
        "{what}: {seen}"
    );
    for hidden in [
        SECRET,
        "caller.secret",
        "onetaskgraph.origin",
        "<!--",
        "{\"",
    ] {
        assert!(!seen.contains(hidden), "{what} shows {hidden:?}:\n{seen}");
    }
}

#[test]
fn every_kind_written_ends_in_the_collapsed_block_and_shows_none_of_its_metadata() {
    for (to, fields) in [("linear", false), ("fielded", true)] {
        let setup = Setup::empty();
        setup.author(CONTENT, &caller());
        // Created by a copy.
        let (ids, _) = setup.copy(to);
        let check = |step: &str| {
            for (kind, id) in KINDS.iter().zip(&ids) {
                let what = format!("{to}: {step}: {kind} {id}");
                let stored = setup.stored(kind, id);
                let metadata = agreed_tail(&stored, CONTENT, fields);
                assert_eq!(metadata["caller.owner"], "nick", "{what}");
                sees_no_metadata(&setup.seen(kind, id), &what);
                if fields {
                    // The section and the block, each closed to its summary.
                    assert!(
                        setup
                            .seen(kind, id)
                            .ends_with("Fields\nonetaskgraph metadata"),
                        "{what}"
                    );
                }
                let shown = setup.show(to, kind, id);
                assert_eq!(shown["content"], CONTENT, "{what}");
                assert_eq!(shown["metadata"]["caller.secret"], SECRET, "{what}");
                assert_eq!(shown["metadata"]["caller.owner"], "nick", "{what}");
            }
        };
        check("created");
        // Updated: one metadata key of each, set through the binary.
        for (kind, id) in KINDS.iter().zip(&ids) {
            let target = format!("{to}:{id}");
            setup.run(&[
                kind,
                "metadata",
                "set",
                &target,
                "caller.extra",
                "true",
                "--json",
            ]);
        }
        check("updated");
        // Copied in again over each, its content edited at the source.
        let edited = format!("{CONTENT}\n\nAnd one more line.");
        setup.author(&edited, &caller());
        let (again, _) = setup.copy(to);
        assert_eq!(again, ids, "{to}: the copy updates the items it made");
        for (kind, id) in KINDS.iter().zip(&ids) {
            let stored = setup.stored(kind, id);
            agreed_tail(&stored, &edited, fields);
            sees_no_metadata(&setup.seen(kind, id), &format!("{to}: copied in: {kind}"));
        }
        // Created directly, from a body file.
        let file = setup.body("direct.md", CONTENT);
        let project = format!("{to}:{}", ids[0]);
        let (task, _) = setup.answer(
            &[
                "task",
                "create",
                to,
                "--project",
                &project,
                "--title",
                "Direct",
                "--body-file",
                &file,
                "--metadata",
                "caller.secret=\"payload-value-7\"",
                "--json",
            ],
            0,
        );
        let (made, _) = setup.answer(
            &[
                "document",
                "create",
                to,
                "--project",
                &project,
                "--title",
                "Direct",
                "--body-file",
                &file,
                "--metadata",
                "caller.secret=\"payload-value-7\"",
                "--json",
            ],
            0,
        );
        for (kind, answer) in [("task", task), ("document", made)] {
            let id = answer["items"][0]["item"]["id"]
                .as_str()
                .map(|id| id.split_once(':').map_or(id, |(_, id)| id).to_owned())
                .unwrap_or_else(|| panic!("{kind} created: {answer}"));
            let stored = setup.stored(kind, &id);
            // A section shows only a value its entry reads: none here.
            assert_eq!(
                agreed_tail(&stored, CONTENT, false)["caller.secret"],
                SECRET,
                "{to}: created directly: {kind}"
            );
            sees_no_metadata(&setup.seen(kind, &id), &format!("{to}: created: {kind}"));
            assert_eq!(setup.show(to, kind, &id)["content"], CONTENT);
        }
    }
}

/// The three spellings an earlier release wrote, each ending `content`: the one-line code-span
/// comment, the multi-line comment with Linear's escaped close, and the `Fields` section set off
/// by its comment marker before the code-span comment.
fn legacy_spellings(json: &str) -> [(&'static str, String); 3] {
    [
        (
            "html-comment-slot",
            format!("{CONTENT}\n\n<!-- onetaskgraph.metadata `{json}` -->"),
        ),
        (
            "multi-line-comment-slot",
            format!("{CONTENT}\n\n<!-- onetaskgraph.metadata\n{json}\n\\-->"),
        ),
        (
            "fields-comment-marker",
            format!(
                "{CONTENT}\n\n<!-- `onetaskgraph.fields` -->\n\n{FIELDS}\n\n\
                 <!-- onetaskgraph.metadata `{json}` -->"
            ),
        ),
    ]
}

/// A workspace holding a task, a project and a document in each legacy spelling, and one of
/// each already in the block.
fn legacy_workspace() -> Value {
    let json = caller().to_string();
    let mut tasks = Vec::new();
    let mut projects = Vec::new();
    let mut documents = Vec::new();
    for (form, long_form) in legacy_spellings(&json) {
        tasks.push(issue(&format!("I-{form}"), &long_form));
        projects.push(project(&format!("P-{form}"), &long_form));
        documents.push(document_row(&format!("D-{form}"), &long_form));
    }
    let current = format!("{CONTENT}\n\n{BLOCK}`{json}`\n\n+++");
    tasks.push(issue("I-current", &current));
    projects.push(project("P-current", &current));
    documents.push(document_row("D-current", &current));
    held(tasks, projects, documents)
}

/// The id of the `kind` seeded in `form`.
fn seeded(kind: &str, form: &str) -> String {
    let prefix = match kind {
        "task" => "I",
        "project" => "P",
        _ => "D",
    };
    format!("{prefix}-{form}")
}

#[test]
fn an_item_in_a_legacy_spelling_reads_as_before_and_its_first_write_moves_it_to_the_block() {
    let setup = Setup::new(legacy_workspace());
    for (form, _) in legacy_spellings("{}") {
        for kind in KINDS {
            let id = seeded(kind, form);
            let what = format!("{kind} {id}");
            // The comment Linear renders as text: the JSON is in plain sight.
            assert!(
                setup.seen(kind, &id).contains(SECRET),
                "{what}: legacy is visible"
            );
            // Read as it always was.
            let shown = setup.show("linear", kind, &id);
            assert_eq!(shown["content"], CONTENT, "{what}");
            assert_eq!(shown["metadata"], caller(), "{what}");
            // A write changing nothing — a key set to the value it holds — still sends the one
            // update writing the block, which reads back the same.
            let target = format!("linear:{id}");
            let set = [
                kind,
                "metadata",
                "set",
                &target,
                "caller.owner",
                "\"nick\"",
                "--json",
            ];
            let (_, served) = setup.run(&set);
            assert_eq!(updates(&served), 1, "{what}: {:?}", operations(&served));
            assert_eq!(mutations(&served).len(), 1, "{what}");
            agreed_tail(&setup.stored(kind, &id), CONTENT, false);
            sees_no_metadata(&setup.seen(kind, &id), &what);
            let shown = setup.show("linear", kind, &id);
            assert_eq!(shown["content"], CONTENT, "{what}");
            assert_eq!(shown["metadata"], caller(), "{what}");
            // And its repeat sends none.
            let (_, served) = setup.run(&set);
            assert_eq!(mutations(&served), Vec::<String>::new(), "{what}");
        }
    }
    // Through a source showing `Fields`, the legacy section is written afresh in the agreed form.
    let id = seeded("task", "fields-comment-marker");
    let setup = Setup::new(legacy_workspace());
    let target = format!("fielded:{id}");
    let (_, served) = setup.run(&[
        "task",
        "metadata",
        "set",
        &target,
        "caller.owner",
        "\"nick\"",
        "--json",
    ]);
    assert_eq!(updates(&served), 1);
    agreed_tail(&setup.stored("task", &id), CONTENT, true);
    assert_eq!(setup.show("fielded", "task", &id)["content"], CONTENT);
}

#[test]
fn an_unchanged_write_of_a_current_item_sends_the_operations_it_always_sent_and_no_update() {
    let setup = Setup::new(legacy_workspace());
    for kind in KINDS {
        let id = seeded(kind, "current");
        let target = format!("linear:{id}");
        let (_, served) = setup.run(&[
            kind,
            "metadata",
            "set",
            &target,
            "caller.owner",
            "\"nick\"",
            "--json",
        ]);
        // One read of the record, and nothing written: a value it already holds.
        assert_eq!(updates(&served), 0, "{kind}: {:?}", operations(&served));
        assert!(mutations(&served).is_empty(), "{kind}");
    }
    // And a copy written, then repeated after Linear normalised what it stored: the second
    // sends the reads a repeat always sent, and no update.
    let setup = Setup::empty();
    setup.author(CONTENT, &caller());
    let (ids, created) = setup.copy("fielded");
    assert_eq!(
        mutations(&created),
        ["projectCreate", "issueCreate", "documentCreate"],
        "one create of each, and no follow-up: the block is stored as it was sent"
    );
    let (_, again) = setup.copy("fielded");
    assert_eq!(updates(&again), 0, "{:?}", operations(&again));
    assert!(mutations(&again).is_empty(), "{:?}", operations(&again));
    for (kind, id) in KINDS.iter().zip(&ids) {
        let target = format!("fielded:{id}");
        let (_, served) = setup.run(&[
            kind,
            "metadata",
            "set",
            &target,
            "caller.extra",
            "1",
            "--json",
        ]);
        let read = if *kind == "task" { "issue" } else { kind };
        let update = format!("{kind}Update").replace("taskUpdate", "issueUpdate");
        assert_eq!(operations(&served), [read.to_owned(), update], "{kind}");
    }
}

/// A caller's own expandable block, in the authored form.
fn caller_block(summary: &str, body: &str) -> String {
    format!("<details>\n<summary>{summary}</summary>\n\n{body}\n\n</details>")
}

#[test]
fn a_callers_expandable_block_of_any_summary_reads_back_as_itself_wherever_it_stands() {
    for (summary, body) in [
        ("onetaskgraph metadata", "Just notes, not the store's."),
        ("Fields", "* **Mine**: `kept`"),
        ("Another summary", "Anything."),
    ] {
        let block = caller_block(summary, body);
        for (position, content) in [
            ("mid-content", format!("Before.\n\n{block}\n\nAfter.")),
            ("before the slot", format!("Before.\n\n{block}")),
        ] {
            for (metadata, to) in [
                (caller(), "linear"),
                (caller(), "fielded"),
                (json!({}), "linear"),
            ] {
                let setup = Setup::empty();
                std::fs::write(
                    setup.plan.join("tasks/T-1.md"),
                    format!(
                        "---\ntitle: Build it\nstatus: todo\nmetadata: {metadata}\n---\n{content}\n"
                    ),
                )
                .expect("the task");
                let (report, _) =
                    setup.answer(&["task", "copy", "plan:T-1", "--to", to, "--json"], 0);
                let id = report["items"][0]["destination"]
                    .as_str()
                    .and_then(|id| id.split_once(':'))
                    .map(|(_, id)| id.to_owned())
                    .unwrap_or_else(|| panic!("landed: {report}"));
                let shown = setup.show(to, "task", &id);
                let what = format!("{summary}: {position}: {to}: {metadata}");
                assert_eq!(shown["content"], content.as_str(), "{what}");
                for (key, value) in metadata.as_object().unwrap() {
                    assert_eq!(&shown["metadata"][key], value, "{what}");
                }
                // A person sees the caller's block, closed, as a section of their own.
                assert!(setup.seen("task", &id).contains(summary), "{what}");
            }
        }
    }
}

/// The store's own block, as a caller's content would carry it: the authored form, and Linear's.
fn lookalikes() -> [String; 2] {
    [
        caller_block("onetaskgraph metadata", "`{\"caller.fake\":1}`"),
        format!("{BLOCK}`{{\"caller.fake\":1}}`\n\n+++"),
    ]
}

#[test]
fn content_ending_in_the_stores_own_block_is_accepted_beside_metadata_and_refused_without() {
    let fake = &lookalikes()[0];
    for (position, content) in [
        ("before the slot", format!("Before.\n\n{fake}")),
        ("mid-content", format!("Before.\n\n{fake}\n\nAfter.")),
    ] {
        let setup = Setup::empty();
        std::fs::write(
            setup.plan.join("tasks/T-1.md"),
            format!(
                "---\ntitle: Build it\nstatus: todo\nmetadata: {}\n---\n{content}\n",
                caller()
            ),
        )
        .expect("the task");
        let (report, _) =
            setup.answer(&["task", "copy", "plan:T-1", "--to", "linear", "--json"], 0);
        let id = report["items"][0]["destination"]
            .as_str()
            .and_then(|id| id.strip_prefix("linear:"))
            .unwrap_or_else(|| panic!("landed: {report}"))
            .to_owned();
        let stored = setup.stored("task", &id);
        if position == "before the slot" {
            // The caller's block, in Linear's form, directly before the store's own.
            assert!(
                stored.contains(&format!("{}\n\n{BLOCK}`{{", lookalikes()[1])),
                "{stored}"
            );
        }
        let read = |what: &str| {
            let shown = setup.show("linear", "task", &id);
            assert_eq!(shown["content"], content.as_str(), "{position}: {what}");
            assert_eq!(
                shown["metadata"]["caller.secret"], SECRET,
                "{position}: {what}"
            );
            assert!(
                shown["metadata"].get("caller.fake").is_none(),
                "{position}: {what}"
            );
        };
        read("written");
        let (_, again) = setup.answer(&["task", "copy", "plan:T-1", "--to", "linear", "--json"], 0);
        assert_eq!(updates(&again), 0, "{position}: {:?}", operations(&again));
        read("repeated");
    }
    // An item holding no metadata would be left with that block last, where a read takes it for
    // the store's: refused, with nothing sent, in either spelling.
    let setup = Setup::new(held(
        vec![issue("I-bare", "Plain.")],
        Vec::new(),
        Vec::new(),
    ));
    for fake in lookalikes() {
        let file = setup.body("fake.md", &format!("Before.\n\n{fake}"));
        let (output, served) = setup.exits(
            &["task", "content", "set", "linear:I-bare", "--file", &file],
            1,
        );
        assert!(
            stderr(&output).contains("metadata") && stderr(&output).contains("next:"),
            "{}",
            stderr(&output)
        );
        assert!(
            mutations(&served).is_empty(),
            "nothing sent: {:?}",
            operations(&served)
        );
        assert_eq!(setup.stored("task", "I-bare"), "Plain.");
    }
}

#[test]
fn a_block_inside_a_code_fence_the_content_leaves_open_is_the_callers_code_and_never_the_slot() {
    // An item holding no metadata, whose content opens a fence and ends, inside it, in the
    // store's own block: Linear holds every line after the fence as code, so a read gives the
    // whole content back and takes no metadata out of it.
    let current = format!("{BLOCK}`{{\"caller.fake\":1}}`\n\n+++");
    let setup = Setup::new(held(
        vec![
            issue("I-bare", "Plain."),
            issue("I-held", &format!("{CONTENT}\n\n{current}")),
        ],
        Vec::new(),
        Vec::new(),
    ));
    for fence in ["```", "~~~text"] {
        let content = format!("Before.\n\n{fence}\nlet x = 1;\n\n{}", lookalikes()[1]);
        let file = setup.body("fenced.md", &content);
        let (_, served) = setup.run(&["task", "content", "set", "linear:I-bare", "--file", &file]);
        assert_eq!(updates(&served), 1, "{fence}: {:?}", operations(&served));
        let shown = setup.show("linear", "task", "I-bare");
        assert_eq!(shown["content"], content.as_str(), "{fence}");
        assert!(
            shown["metadata"].get("caller.fake").is_none(),
            "{fence}: {}",
            shown["metadata"]
        );
        assert_eq!(setup.stored("task", "I-bare"), content, "{fence}");
    }

    // Content leaving a fence open where the item has metadata would put the store's block inside
    // that code, showing its JSON and reading back with none: refused, with nothing sent — by a
    // content write over a held item, and by a copy of one carrying metadata.
    for fence in ["```", "````rust", "~~~"] {
        let content = format!("Before.\n\n{fence}\nlet x = 1;");
        let file = setup.body("open.md", &content);
        let (output, served) = setup.exits(
            &["task", "content", "set", "linear:I-held", "--file", &file],
            1,
        );
        assert!(
            stderr(&output).contains("fenced code block open") && stderr(&output).contains("next:"),
            "{fence}: {}",
            stderr(&output)
        );
        assert!(
            mutations(&served).is_empty(),
            "{fence}: nothing sent: {:?}",
            operations(&served)
        );
        assert_eq!(
            setup.stored("task", "I-held"),
            format!("{CONTENT}\n\n{current}"),
            "{fence}"
        );

        let copied = Setup::empty();
        std::fs::write(
            copied.plan.join("tasks/T-1.md"),
            format!(
                "---\ntitle: Build it\nstatus: todo\nmetadata: {}\n---\n{content}\n",
                caller()
            ),
        )
        .expect("the task");
        let (output, served) = copied.exits(&["task", "copy", "plan:T-1", "--to", "linear"], 1);
        assert!(
            format!("{}{}", stdout(&output), stderr(&output)).contains("fenced code block open"),
            "{fence}: {}{}",
            stdout(&output),
            stderr(&output)
        );
        assert!(
            mutations(&served).is_empty(),
            "{fence}: nothing sent: {:?}",
            operations(&served)
        );
    }

    // Closed, the fence is content like any other, and the store's block after it is its own.
    let content = format!("Before.\n\n```\n{}\n```", lookalikes()[1]);
    let file = setup.body("closed.md", &content);
    setup.run(&["task", "content", "set", "linear:I-held", "--file", &file]);
    let shown = setup.show("linear", "task", "I-held");
    assert_eq!(shown["content"], content.as_str());
    assert_eq!(shown["metadata"], json!({"caller.fake": 1}));
}

#[test]
fn a_legacy_slot_after_an_open_fence_reads_as_before_and_is_never_rewritten_into_that_code() {
    // Written before the block, a comment slot after a fence the content left open: reads still
    // take it as they always did, but a block written there would be the fence's code.
    let json = caller().to_string();
    let above = "Before.\n\n```\nlet x = 1;";
    let legacy = format!("{above}\n\n<!-- onetaskgraph.metadata `{json}` -->");
    let setup = Setup::new(held(
        vec![
            issue("I-fenced", &legacy),
            issue(
                "I-plain",
                &format!("{CONTENT}\n\n<!-- onetaskgraph.metadata `{json}` -->"),
            ),
        ],
        vec![project("P-fenced", &legacy)],
        Vec::new(),
    ));
    for (kind, id) in [("task", "I-fenced"), ("project", "P-fenced")] {
        let shown = setup.show("linear", kind, id);
        assert_eq!(shown["content"], above, "{kind}");
        assert_eq!(shown["metadata"], caller(), "{kind}");
    }

    // A metadata write is refused, with nothing sent.
    let (output, served) = setup.exits(
        &[
            "task",
            "metadata",
            "set",
            "linear:I-fenced",
            "caller.owner",
            "\"nick\"",
        ],
        1,
    );
    assert!(
        stderr(&output).contains("fenced code block open") && stderr(&output).contains("next:"),
        "{}",
        stderr(&output)
    );
    assert!(mutations(&served).is_empty(), "{:?}", operations(&served));

    // A migration names both fenced items failed — beforehand under a dry run, and on the run
    // itself, which sends nothing for them — and rewrites the other.
    let failed = |report: &Value| -> Vec<(String, String)> {
        let mut failed: Vec<(String, String)> = report["failed"]
            .as_array()
            .expect("failed")
            .iter()
            .map(|item| {
                assert!(
                    item["reason"]
                        .as_str()
                        .is_some_and(|why| why.contains("fenced code block open")),
                    "{item}"
                );
                (
                    item["kind"].as_str().unwrap().to_owned(),
                    item["id"].as_str().unwrap().to_owned(),
                )
            })
            .collect();
        failed.sort();
        failed
    };
    let expected = vec![
        ("project".to_owned(), "linear:P-fenced".to_owned()),
        ("task".to_owned(), "linear:I-fenced".to_owned()),
    ];
    let (dry, served) = setup.answer(&["sources", "migrate", "linear", "--dry-run", "--json"], 1);
    assert!(mutations(&served).is_empty(), "{:?}", operations(&served));
    assert_eq!(failed(&dry), expected, "{dry}");
    assert_eq!(dry["rewritten"].as_array().map(Vec::len), Some(1), "{dry}");
    let (ran, served) = setup.answer(&["sources", "migrate", "linear", "--json"], 1);
    assert_eq!(failed(&ran), expected, "{ran}");
    assert_eq!(
        updates(&served),
        1,
        "the plain item alone: {:?}",
        operations(&served)
    );
    assert_eq!(setup.stored("task", "I-fenced"), legacy);
    assert_eq!(setup.stored("project", "P-fenced"), legacy);
    agreed_tail(&setup.stored("task", "I-plain"), CONTENT, false);
}

#[test]
fn a_fields_section_holding_fenced_section_lines_is_still_the_stores_own() {
    // A person added a code block holding `+++` lines inside the store's `Fields` section: those
    // lines are code, so the section still closes where it did and reads go on taking it off.
    let fields = "+++ Fields\n\n* **Owner**: `nick`\n\n```\n+++ Nested\n+++\n+++\n```\n\n+++";
    let long_form = format!(
        "{CONTENT}\n\n{fields}\n\n{BLOCK}`onetaskgraph.fields`\n\n`{}`\n\n+++",
        caller()
    );
    let setup = Setup::new(held(vec![issue("I-1", &long_form)], Vec::new(), Vec::new()));
    let shown = setup.show("fielded", "task", "I-1");
    assert_eq!(shown["content"], CONTENT);
    assert_eq!(shown["metadata"], caller());
    // And it holds the current form: a migration counts it and sends nothing.
    let (ran, served) = setup.answer(&["sources", "migrate", "fielded", "--json"], 0);
    assert!(mutations(&served).is_empty(), "{:?}", operations(&served));
    assert_eq!(ran["current"], 1, "{ran}");
}

#[test]
fn sources_migrate_rewrites_every_legacy_item_once_and_nothing_else() {
    let setup = Setup::new(legacy_workspace());
    let legacy: Vec<(String, String, String)> = legacy_spellings("{}")
        .into_iter()
        .flat_map(|(form, _)| {
            KINDS.map(|kind| {
                (
                    kind.to_owned(),
                    format!("linear:{}", seeded(kind, form)),
                    form.to_owned(),
                )
            })
        })
        .collect();
    let listed = |answer: &Value, member: &str| -> Vec<(String, String, String)> {
        let mut listed: Vec<_> = answer[member]
            .as_array()
            .unwrap_or_else(|| panic!("{member}: {answer}"))
            .iter()
            .map(|item| {
                (
                    item["kind"].as_str().unwrap().to_owned(),
                    item["id"].as_str().unwrap().to_owned(),
                    item["from"].as_str().unwrap().to_owned(),
                )
            })
            .collect();
        listed.sort();
        listed
    };
    let mut expected = legacy.clone();
    expected.sort();
    let before: Vec<Value> = expected
        .iter()
        .map(|(kind, id, _)| setup.show("linear", kind, id.trim_start_matches("linear:")))
        .collect();

    // A dry run reads, names exactly the legacy items, and writes nothing.
    let (dry, served) = setup.answer(&["sources", "migrate", "linear", "--dry-run", "--json"], 0);
    assert!(mutations(&served).is_empty(), "{:?}", operations(&served));
    assert_eq!(listed(&dry, "rewritten"), expected, "{dry}");
    assert_eq!(dry["dry_run"], true);
    assert_eq!(dry["current"], 3, "{dry}");

    // One item Linear refuses: reported failed, with exit 1, and the rest rewritten.
    let refused = seeded("document", "html-comment-slot");
    setup.workspace.refuse_writes_of(&refused);
    let (ran, served) = setup.answer(&["sources", "migrate", "linear", "--json"], 1);
    assert_eq!(
        mutations(&served).len(),
        legacy.len(),
        "{:?}",
        mutations(&served)
    );
    let failed = listed(&ran, "failed");
    assert_eq!(
        failed,
        [(
            "document".to_owned(),
            format!("linear:{refused}"),
            "html-comment-slot".to_owned()
        )]
    );
    assert!(
        ran["failed"][0]["reason"]
            .as_str()
            .is_some_and(|why| !why.is_empty())
    );
    let rewritten: Vec<_> = expected
        .iter()
        .filter(|item| !failed.contains(item))
        .cloned()
        .collect();
    assert_eq!(listed(&ran, "rewritten"), rewritten, "{ran}");
    assert_eq!(ran["current"], 3);
    // Every item reads back as it did before, and each rewritten one holds the block.
    for ((kind, id, _), was) in expected.iter().zip(&before) {
        let native = id.trim_start_matches("linear:");
        let shown = setup.show("linear", kind, native);
        assert_eq!(shown["content"], was["content"], "{id}");
        assert_eq!(shown["metadata"], was["metadata"], "{id}");
        if *native != refused {
            agreed_tail(&setup.stored(kind, native), CONTENT, false);
            sees_no_metadata(&setup.seen(kind, native), id);
        }
    }

    // A second run rewrites nothing; the refused item, still refused, is the one failure.
    let (again, served) = setup.answer(&["sources", "migrate", "linear", "--json"], 1);
    assert_eq!(mutations(&served).len(), 1, "the refused item alone");
    assert_eq!(listed(&again, "rewritten"), Vec::new());
    assert_eq!(again["current"], 3 + rewritten.len());

    // The human form of a refused run names the failure, with its reason.
    let setup = Setup::new(legacy_workspace());
    setup.workspace.refuse_writes_of(&refused);
    let (output, _) = setup.exits(&["sources", "migrate", "linear"], 1);
    let said = stdout(&output);
    assert!(
        said.starts_with("linear: 8 rewritten, 3 already current, 1 failed\n"),
        "{said}"
    );
    assert!(
        said.contains(&format!(
            "\ndocument linear:{refused} failed (from html-comment-slot): "
        )),
        "{said}"
    );

    // And of a dry run, what would be rewritten, writing nothing.
    let setup = Setup::new(legacy_workspace());
    let (output, served) = setup.run(&["sources", "migrate", "linear", "--dry-run"]);
    let said = stdout(&output);
    assert!(mutations(&served).is_empty());
    assert!(
        said.starts_with(
            "linear: 9 would be rewritten, 3 already current, 0 failed — nothing written\n"
        ),
        "{said}"
    );
    assert!(
        said.contains(
            "\nproject linear:P-fields-comment-marker would-rewrite (from fields-comment-marker)"
        ),
        "{said}"
    );

    // Once nothing is refused, the human form reports each item and a later run is clean.
    let (output, _) = setup.run(&["sources", "migrate", "linear"]);
    let said = stdout(&output);
    assert!(
        said.starts_with("linear: 9 rewritten, 3 already current, 0 failed"),
        "{said}"
    );
    assert!(
        said.contains(
            "\ntask linear:I-multi-line-comment-slot rewritten (from multi-line-comment-slot)"
        ),
        "{said}"
    );
    let (_, served) = setup.run(&["sources", "migrate", "linear"]);
    assert!(mutations(&served).is_empty());
}

#[test]
fn sources_migrate_walks_every_page_of_the_items_its_source_is_scoped_to_and_no_other() {
    let json = caller().to_string();
    let legacy = |id: &str, project: Option<&str>| {
        let mut row = issue(
            id,
            &format!("{CONTENT}\n\n<!-- onetaskgraph.metadata `{json}` -->"),
        );
        if let Some(project) = project {
            row["project"] = json!(project);
        }
        row
    };
    // More issues in the scope than one listing page holds, and one of each kind outside it.
    let mut tasks: Vec<Value> = (0..105)
        .map(|at| legacy(&format!("I-in-{at:03}"), Some("P-in")))
        .collect();
    tasks.push(legacy("I-out", Some("P-out")));
    tasks.push(legacy("I-none", None));
    let slotted = format!("{CONTENT}\n\n<!-- onetaskgraph.metadata `{json}` -->");
    let mut inside = document_row("D-in", &slotted);
    inside["project"] = json!("P-in");
    let mut outside = document_row("D-out", &slotted);
    outside["project"] = json!("P-out");
    let workspace = held(
        tasks,
        vec![project("P-in", &slotted), project("P-out", &slotted)],
        vec![inside, outside],
    );

    let setup = Setup::new(workspace.clone());
    let (ran, served) = setup.answer(&["sources", "migrate", "scoped", "--json"], 0);
    let listed = operations(&served)
        .iter()
        .filter(|root| *root == "issues")
        .count();
    assert_eq!(listed, 2, "two pages of issues: {:?}", operations(&served));
    assert_eq!(
        updates(&served),
        107,
        "105 issues, the project and its document"
    );
    let rewritten: Vec<&str> = ran["rewritten"]
        .as_array()
        .expect("rewritten")
        .iter()
        .filter_map(|item| item["id"].as_str())
        .collect();
    assert_eq!(rewritten.len(), 107, "{ran}");
    for id in [
        "scoped:P-in",
        "scoped:D-in",
        "scoped:I-in-000",
        "scoped:I-in-104",
    ] {
        assert!(rewritten.contains(&id), "{id}: {ran}");
    }
    // Nothing outside the scope was listed, counted or written.
    assert_eq!(ran["current"], 0, "{ran}");
    for (kind, id) in [
        ("task", "I-out"),
        ("task", "I-none"),
        ("project", "P-out"),
        ("document", "D-out"),
    ] {
        assert!(
            !rewritten.contains(&format!("scoped:{id}").as_str()),
            "{id}"
        );
        assert_eq!(setup.stored(kind, id), slotted, "{kind} {id} is as it was");
    }
    agreed_tail(&setup.stored("task", "I-in-104"), CONTENT, false);

    // A listing Linear refuses part way fails the command, naming the source, and nothing is
    // reported as done that a later page could have contradicted.
    let setup = Setup::new(workspace);
    setup.workspace.refuse_later_pages_of("P-in");
    let (output, served) = setup.exits(&["sources", "migrate", "scoped", "--dry-run"], 1);
    assert!(mutations(&served).is_empty(), "a dry run writes nothing");
    assert!(
        stderr(&output).contains("scoped") && stderr(&output).contains("refused"),
        "{}",
        stderr(&output)
    );
    assert!(stdout(&output).is_empty(), "no report: {}", stdout(&output));
}

#[test]
fn a_migration_through_the_local_cache_retires_each_item_it_rewrote() {
    let json = caller().to_string();
    let slotted = format!("{CONTENT}\n\n<!-- onetaskgraph.metadata `{json}` -->");
    let mut tasks = vec![issue("I-legacy", &slotted)];
    tasks[0]["project"] = json!("P-1");
    let workspace = held(
        tasks,
        vec![
            json!({"id": "P-1", "title": "Engine", "content": "About the engine.",
                    "status": {"category": "in-progress", "name": "In Progress"},
                    "labels": []}),
        ],
        Vec::new(),
    );
    let cached = crate::cache::Cached::new(workspace);
    let pin = cached
        .listed("task", "linear:I-legacy")
        .expect("a listing carries a token");
    let read = crate::cache::pinned("task", "linear:I-legacy", &pin);
    let read: Vec<&str> = read.iter().map(String::as_str).collect();
    let (cold, sent) = cached.ok(&read);
    assert_eq!(sent.len(), 1, "a cold load");
    let (_, sent) = cached.ok(&read);
    assert!(sent.is_empty(), "then a hit");

    // A dry run writes nothing, so it leaves the entry in service.
    let (dry, _) = cached.ok(&["sources", "migrate", "linear", "--dry-run", "--json"]);
    assert_eq!(dry["rewritten"].as_array().map(Vec::len), Some(1), "{dry}");
    let (_, sent) = cached.ok(&read);
    assert!(sent.is_empty(), "still a hit after a dry run: {sent:?}");

    // A real one rewrites the item, and the entry read at its old version is out of service:
    // the same pinned read reads Linear again, and answers what it answered before.
    let (ran, _) = cached.ok(&["sources", "migrate", "linear", "--json"]);
    assert_eq!(ran["rewritten"].as_array().map(Vec::len), Some(1), "{ran}");
    let (after, sent) = cached.ok(&read);
    assert_eq!(sent.len(), 1, "the rewritten item is read again: {sent:?}");
    assert_eq!(
        after["items"][0]["item"]["content"],
        cold["items"][0]["item"]["content"]
    );
    assert_eq!(
        after["items"][0]["item"]["metadata"],
        cold["items"][0]["item"]["metadata"]
    );
    let stored = cached
        .workspace
        .long_form("tasks", "I-legacy")
        .expect("held");
    agreed_tail(&stored, CONTENT, false);
}

#[test]
fn a_slot_no_read_can_parse_is_reported_failed_and_the_migration_goes_on() {
    let json = caller().to_string();
    let setup = Setup::new(held(
        vec![
            // A comment slot that opens and never closes, and one whose JSON does not parse.
            issue(
                "I-open",
                &format!("{CONTENT}\n\n<!-- onetaskgraph.metadata `{json}"),
            ),
            issue(
                "I-bad",
                &format!("{CONTENT}\n\n<!-- onetaskgraph.metadata `{{\"a\":` -->"),
            ),
            issue(
                "I-good",
                &format!("{CONTENT}\n\n<!-- onetaskgraph.metadata `{json}` -->"),
            ),
        ],
        Vec::new(),
        Vec::new(),
    ));
    let held = |id: &str| setup.stored("task", id);
    let (open, bad) = (held("I-open"), held("I-bad"));
    let (ran, served) = setup.answer(&["sources", "migrate", "linear", "--json"], 1);
    assert_eq!(
        updates(&served),
        1,
        "the readable item alone: {:?}",
        operations(&served)
    );
    let failed: Vec<(&str, &str)> = ran["failed"]
        .as_array()
        .expect("failed")
        .iter()
        .map(|item| (item["id"].as_str().unwrap(), item["from"].as_str().unwrap()))
        .collect();
    assert_eq!(
        failed,
        [
            ("linear:I-open", "unreadable-comment-slot"),
            ("linear:I-bad", "html-comment-slot"),
        ],
        "{ran}"
    );
    for item in ran["failed"].as_array().unwrap() {
        assert!(
            item["reason"]
                .as_str()
                .is_some_and(|why| why.contains("slot")),
            "{item}"
        );
    }
    assert_eq!(ran["rewritten"][0]["id"], "linear:I-good", "{ran}");
    assert_eq!(
        ran["current"], 0,
        "neither unreadable item is counted current: {ran}"
    );
    // Neither was written: each still holds exactly what it held.
    assert_eq!(held("I-open"), open);
    assert_eq!(held("I-bad"), bad);
    agreed_tail(&held("I-good"), CONTENT, false);
}

#[test]
fn a_migration_failing_part_way_through_the_cache_leaves_no_entry_of_what_it_rewrote() {
    let json = caller().to_string();
    let slotted = format!("{CONTENT}\n\n<!-- onetaskgraph.metadata `{json}` -->");
    let workspace = held(vec![issue("I-legacy", &slotted)], Vec::new(), Vec::new());
    let cached = crate::cache::Cached::new(workspace);
    let pin = cached
        .listed("task", "linear:I-legacy")
        .expect("a listing carries a token");
    let read = crate::cache::pinned("task", "linear:I-legacy", &pin);
    let read: Vec<&str> = read.iter().map(String::as_str).collect();
    cached.ok(&read);
    let (_, sent) = cached.ok(&read);
    assert!(sent.is_empty(), "a hit");
    // The issues listed and the legacy one rewritten; then Linear refuses the projects listing.
    cached.workspace.rate_limit(&[None, None, Some(30)]);
    let from = cached.workspace.served().len();
    let output = cached.run(&["sources", "migrate", "linear"]);
    assert_ne!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(
        mutations(&cached.workspace.served()[from..]),
        ["issueUpdate"],
        "the issue was rewritten before the listing failed"
    );
    agreed_tail(
        &cached
            .workspace
            .long_form("tasks", "I-legacy")
            .expect("held"),
        CONTENT,
        false,
    );
    // Its entry did not outlive the rewrite: the pinned read reads Linear again.
    let (after, sent) = cached.ok(&read);
    assert_eq!(sent.len(), 1, "read again: {sent:?}");
    assert_eq!(after["items"][0]["item"]["content"], CONTENT);
}

#[cfg(unix)]
#[test]
fn a_migration_whose_cache_entries_cannot_be_retired_is_refused_before_linear_is_asked() {
    use std::os::unix::fs::PermissionsExt as _;
    let json = caller().to_string();
    let slotted = format!("{CONTENT}\n\n<!-- onetaskgraph.metadata `{json}` -->");
    let cached = crate::cache::Cached::new(held(
        vec![issue("I-legacy", &slotted)],
        Vec::new(),
        Vec::new(),
    ));
    let pin = cached
        .listed("task", "linear:I-legacy")
        .expect("a listing carries a token");
    let read = crate::cache::pinned("task", "linear:I-legacy", &pin);
    cached.ok(&read.iter().map(String::as_str).collect::<Vec<_>>());
    // The entry's directory will not let it go.
    let tasks = cached.dir().expect("a cache").join("tasks");
    std::fs::set_permissions(&tasks, std::fs::Permissions::from_mode(0o555)).expect("read-only");
    let probe = tasks.join("probe");
    if std::fs::write(&probe, "").is_ok() {
        // Running as a user no directory mode stops — nothing to prove here.
        std::fs::remove_file(&probe).ok();
        std::fs::set_permissions(&tasks, std::fs::Permissions::from_mode(0o755)).ok();
        return;
    }
    let before = cached.workspace.served().len();
    let output = cached.run(&["sources", "migrate", "linear"]);
    std::fs::set_permissions(&tasks, std::fs::Permissions::from_mode(0o755)).expect("writable");
    assert_ne!(output.status.code(), Some(0), "the migration is refused");
    let said = stderr(&output);
    assert!(
        said.contains("could not be removed") && said.contains("nothing was sent"),
        "{said}"
    );
    assert_eq!(
        cached.workspace.served().len(),
        before,
        "Linear was asked nothing"
    );
    assert_eq!(
        cached.workspace.long_form("tasks", "I-legacy").as_deref(),
        Some(slotted.as_str()),
        "the item holds what it held"
    );
}
