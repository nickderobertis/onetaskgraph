//! A rendering written to Linear records the digest of what Linear stores, through the binary.
//!
//! The loopback Linear is made to store every long form it is written as the Linear plugin's
//! ruling on what Linear does to Markdown on write declares — `- ` bullets come back `* `, a
//! delimiter row `| -- |`, a domain-like token linked, brackets and tildes escaped, `_y_` as
//! `*y*` — so a store recording the digest of what it *sent* would record one no read matches.
//! Every journey reads the item back through the binary and hashes its content here, never
//! taking a digest from the engine that wrote it, and reads the workspace's request log for what
//! was sent. A second mode adds one rewrite the ruling does not declare, which the store refuses
//! as `stored-content-diverged` rather than record.

use std::path::PathBuf;
use std::process::Output;

use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{
    LINEAR_UNDECLARED_REWRITE, LinearMarkdown, LinearWorkspace, document, linear_workspace_with,
};

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

fn mapping() -> Value {
    json!({
        "todo":        {"task": "Ready",    "project": "Scheduled"},
        "in-progress": {"task": "Working",  "project": "Underway"},
        "done":        {"task": "Finished", "project": "Shipped"},
        "cancelled":   {"task": "Dropped",  "project": "Abandoned"},
    })
}

/// The Markdown every rendering here holds outside code — each thing the ruling rewrites, and a
/// numbered line opening `1) ` that only the undeclared rewrite touches — and the same text
/// inside an inline code span and a fenced block, where it is kept.
const BODY: &str = "Goal: {{ goal }}\n\
\n\
- build it\n\
- ship it\n\
\n\
| Step | Owner |\n\
| --- | --- |\n\
| build | ops |\n\
\n\
Ship it to example.com once [x] is ticked, ~2 days, _soon_.\n\
1) Then review it.\n\
\n\
Kept: `- [x] _y_ example.com ~` here.\n\
\n\
```text\n\
- [x] _y_ example.com ~\n\
```\n";

/// A loader document whose one template is a heading over [`BODY`].
fn loader(reference: &str, heading: &str) -> String {
    let source = format!(
        "---\nonetaskgraph_template: 1\nvariables:\n  goal:\n    description: What it is for\n\
         ---\n# {heading}\n{BODY}"
    );
    json!({
        "reference": reference,
        "entry": "entry.md",
        "templates": [{"name": "entry.md", "source": source}],
    })
    .to_string()
}

/// `sha256:` and the hex SHA-256 of `content`, hashed here.
fn sha256(content: &str) -> String {
    let digest = Sha256::digest(content.as_bytes());
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("sha256:{hex}")
}

/// A folder of Markdown `plan`, a second one `back`, and a loopback Linear workspace `linear`
/// holding one project to file what is created there under, with a loader document for a plan
/// task, a project description and a design document.
struct Plan {
    sandbox: Sandbox,
    workspace: LinearWorkspace,
    plan: PathBuf,
    back: PathBuf,
    task: String,
    project: String,
    design: String,
}

impl Plan {
    fn new() -> Self {
        Self::caching(false)
    }

    /// [`Plan::new`], with the Linear source's local cache on when `cache` says so — the store
    /// then reaches the plugin through the cache that stands in front of it.
    fn caching(cache: bool) -> Self {
        let sandbox = Sandbox::new();
        let held = json!({
            "tasks": [], "documents": [], "labels": [],
            "task_dependencies": [], "project_dependencies": [],
            "projects": [{"id": "P-1", "title": "Home", "content": "Where it lives.",
                          "status": {"category": "todo", "name": "Scheduled"}, "labels": []}],
        });
        let (mut config, workspace) =
            linear_workspace_with(&sandbox, held, TEAM_STATES, PROJECT_STATUSES);
        config["status_mapping"] = mapping();
        workspace.rewrite_markdown_on_write(LinearMarkdown::Declared);
        let plan = sandbox.subdirectory("plan");
        let back = sandbox.subdirectory("back");
        for root in [&plan, &back] {
            for kind in ["tasks", "projects", "documents"] {
                std::fs::create_dir_all(root.join(kind)).expect("a folder");
            }
        }
        let mut linear = json!({"plugin": "linear", "config": config});
        if cache {
            linear["cache"] = json!({"enabled": true, "dir": sandbox.subdirectory("cache")});
        }
        sandbox.project_document(&document(&json!({
            "linear": linear,
            "plan": {"plugin": "local-md", "config": {"root": plan}},
            "back": {"plugin": "local-md", "config": {"root": back}},
        })));
        let inputs = sandbox.subdirectory("inputs");
        let write = |name: &str, text: String| {
            let path = inputs.join(name);
            std::fs::write(&path, text).expect("a loader document");
            path.display().to_string()
        };
        let task = write("task.json", loader("host:plan-task", "Step"));
        let project = write("project.json", loader("host:plan-description", "Plan"));
        let design = write("design.json", loader("host:design-document", "Design"));
        Self {
            sandbox,
            workspace,
            plan,
            back,
            task,
            project,
            design,
        }
    }

    /// One command, and every request the workspace answered for it.
    fn run(&self, arguments: &[&str]) -> (Output, Vec<(String, Value)>) {
        let from = self.workspace.served().len();
        let output = self
            .sandbox
            .command()
            .args(arguments)
            .assert()
            .get_output()
            .clone();
        (output, self.workspace.served()[from..].to_vec())
    }

    /// One command that exits `code`, and every request it sent.
    fn exits(&self, arguments: &[&str], code: i32) -> (Output, Vec<(String, Value)>) {
        let (output, served) = self.run(arguments);
        assert_eq!(
            output.status.code(),
            Some(code),
            "`onetaskgraph {}`\n{}{}",
            arguments.join(" "),
            stdout(&output),
            stderr(&output)
        );
        (output, served)
    }

    /// What a command that had to succeed printed, trimmed.
    fn printed(&self, arguments: &[&str]) -> String {
        stdout(&self.exits(arguments, 0).0).trim().to_owned()
    }

    /// One task, project or document as `<kind> show --json` reads it.
    fn show(&self, kind: &str, id: &str) -> Value {
        let output = self.exits(&[kind, "show", id, "--json"], 0).0;
        let shown: Value = serde_json::from_str(&stdout(&output)).expect("JSON");
        shown["items"][0]["item"].clone()
    }

    /// A copy, answering its report and every request it sent.
    fn copy(&self, arguments: &[&str], code: i32) -> (Value, String, Vec<(String, Value)>) {
        let mut all = arguments.to_vec();
        all.push("--json");
        let (output, served) = self.exits(&all, code);
        let report = serde_json::from_str(&stdout(&output)).expect("a copy report");
        (report, stderr(&output), served)
    }

    /// A rendered project `goal`, a rendered task and a rendered design document filed under it,
    /// all in the folder `plan`: the task's and the document's qualified ids.
    fn rendered_plan(&self) -> (String, String) {
        self.printed(&[
            "project",
            "create",
            "plan",
            "--id",
            "goal",
            "--title",
            "Goal",
            "--template-loader",
            &self.project,
            "--var",
            "goal=Ship the plan",
            "--no-interactive",
        ]);
        let task = self.printed(&[
            "task",
            "create",
            "plan",
            "--project",
            "goal",
            "--title",
            "Build it",
            "--template-loader",
            &self.task,
            "--var",
            "goal=Build the thing",
            "--no-interactive",
        ]);
        let design = self.printed(&[
            "document",
            "create",
            "plan",
            "--project",
            "goal",
            "--title",
            "Design",
            "--template-loader",
            &self.design,
            "--var",
            "goal=Design the thing",
            "--no-interactive",
        ]);
        (task, design)
    }

    /// Copy the rendered plan to Linear — the project with its task, then its design document.
    fn copy_plan(&self, design: &str, code: i32) -> (Vec<Value>, String, usize) {
        let (project, project_errors, project_served) =
            self.copy(&["project", "copy", "plan:goal", "--to", "linear"], code);
        let (document, document_errors, document_served) =
            self.copy(&["document", "copy", design, "--to", "linear"], code);
        let mut items: Vec<Value> = project["items"].as_array().expect("items").clone();
        items.extend(document["items"].as_array().expect("items").iter().cloned());
        (
            items,
            format!("{project_errors}{document_errors}"),
            project_served.len() + document_served.len(),
        )
    }
}

/// The outcome a copy report gave the item copied from `source`.
fn outcome<'a>(items: &'a [Value], source: &str) -> &'a Value {
    items
        .iter()
        .find(|item| item["source"] == source)
        .unwrap_or_else(|| panic!("the copy reports {source}: {items:#?}"))
}

/// The kind of show verb an item copied from `source` is read with.
fn kind_of(source: &str) -> &'static str {
    if source.ends_with(":goal") {
        "project"
    } else if source.contains(":design") {
        "document"
    } else {
        "task"
    }
}

/// Every mutation in `served`, by its root field.
fn mutations(served: &[(String, Value)]) -> Vec<String> {
    served
        .iter()
        .filter(|(query, _)| query.trim_start().starts_with("mutation"))
        .map(|(query, _)| {
            query
                .split_once('{')
                .and_then(|(_, rest)| rest.trim_start().split('(').next())
                .unwrap_or_default()
                .to_owned()
        })
        .collect()
}

/// Assert `item`'s content, as a read through the binary hands it back, hashes to the
/// `body_digest` its `onetaskgraph.template` entry records — and keeps the code span and the
/// fenced block of [`BODY`] byte for byte while the prose around them is Linear's.
fn verifiable(item: &Value, what: &str) {
    let content = item["content"].as_str().expect("content");
    assert_eq!(
        item["metadata"]["onetaskgraph.template"]["body_digest"],
        sha256(content),
        "{what} reads back as content whose digest its provenance records:\n{content}"
    );
    for kept in [
        "Kept: `- [x] _y_ example.com ~` here.\n",
        "```text\n- [x] _y_ example.com ~\n```",
    ] {
        assert!(
            content.contains(kept),
            "{what} keeps {kept:?} byte for byte:\n{content}"
        );
    }
    for rewritten in [
        "* build it\n* ship it\n",
        "| -- | -- |\n",
        "Ship it to [example.com](<http://example.com>) once \\[x\\] is ticked, \\~2 days, *soon*.",
    ] {
        assert!(
            content.contains(rewritten),
            "{what} holds the prose as Linear stores it, {rewritten:?}:\n{content}"
        );
    }
}

#[test]
fn a_copied_plan_reads_back_from_linear_as_the_digest_its_provenance_records() {
    for cache in [false, true] {
        let plan = Plan::caching(cache);
        let (task, design) = plan.rendered_plan();
        let (items, errors, sent) = plan.copy_plan(&design, 0);
        assert!(errors.is_empty(), "{errors}");
        if !cache {
            // Exactly the eight requests this copy sent before the store recorded what Linear
            // stores — measured with the 0.8.0 binary over this same journey. What Linear stores
            // is read off each mutation's own answer, never by a read of its own.
            // llmlint: ignore[tests_hold_no_nonfunctional_thresholds] The change's own acceptance criterion: the first copy sends exactly the requests it sent before, asserted exactly off the loopback's request log. A budget's threshold would admit the extra read this assertion exists to refuse, and the Linear budgets file is not this change's to edit.
            assert_eq!(sent, 8, "the first copy's requests, unchanged");
        }
        for source in ["plan:goal", task.as_str(), design.as_str()] {
            let landed = outcome(&items, source);
            assert_eq!(landed["action"], "created", "cache {cache}: {landed}");
            let destination = landed["destination"].as_str().expect("an id");
            verifiable(&plan.show(kind_of(source), destination), destination);
        }
    }
}

#[test]
fn a_repeat_copy_of_an_unchanged_plan_writes_nothing_to_linear() {
    let plan = Plan::new();
    let (_, design) = plan.rendered_plan();
    plan.copy_plan(&design, 0);
    let (project, _, project_served) =
        plan.copy(&["project", "copy", "plan:goal", "--to", "linear"], 0);
    let (document, _, document_served) =
        plan.copy(&["document", "copy", &design, "--to", "linear"], 0);
    let updates: Vec<String> = mutations(&project_served)
        .into_iter()
        .chain(mutations(&document_served))
        .filter(|root| {
            matches!(
                root.as_str(),
                "issueUpdate" | "projectUpdate" | "documentUpdate"
            )
        })
        .collect();
    assert!(
        updates.is_empty(),
        "an unchanged plan writes nothing: {updates:?}"
    );
    for item in project["items"]
        .as_array()
        .into_iter()
        .chain(document["items"].as_array())
        .flatten()
    {
        assert_eq!(item["action"], "unchanged", "{item}");
    }
}

#[test]
fn a_re_rendered_plan_updates_each_linear_item_to_a_digest_its_content_reads_as() {
    let plan = Plan::new();
    let (task, design) = plan.rendered_plan();
    plan.copy_plan(&design, 0);
    for (kind, id, loader) in [
        ("project", "plan:goal", &plan.project),
        ("task", task.as_str(), &plan.task),
        ("document", design.as_str(), &plan.design),
    ] {
        plan.exits(
            &[
                kind,
                "render",
                id,
                "--template-loader",
                loader,
                "--var",
                "goal=Changed at the source",
                "--no-interactive",
            ],
            0,
        );
    }
    let (items, errors, _) = plan.copy_plan(&design, 0);
    assert!(errors.is_empty(), "{errors}");
    for source in ["plan:goal", task.as_str(), design.as_str()] {
        let landed = outcome(&items, source);
        assert_eq!(landed["action"], "updated", "{landed}");
        let destination = landed["destination"].as_str().expect("an id");
        let item = plan.show(kind_of(source), destination);
        verifiable(&item, destination);
        assert!(
            item["content"]
                .as_str()
                .is_some_and(|content| content.contains("Goal: Changed at the source")),
            "{item}"
        );
    }

    // The update path refuses a rewrite the ruling does not declare, as the create path does.
    for (kind, id, loader) in [
        ("project", "plan:goal", &plan.project),
        ("task", task.as_str(), &plan.task),
        ("document", design.as_str(), &plan.design),
    ] {
        plan.exits(
            &[
                kind,
                "render",
                id,
                "--template-loader",
                loader,
                "--var",
                "goal=Changed once more",
                "--no-interactive",
            ],
            0,
        );
    }
    plan.workspace
        .rewrite_markdown_on_write(LinearMarkdown::Undeclared);
    let (items, errors, _) = plan.copy_plan(&design, 4);
    for source in ["plan:goal", task.as_str(), design.as_str()] {
        assert_eq!(
            outcome(&items, source)["error"]["kind"],
            "stored-content-diverged",
            "{items:#?}"
        );
    }
    assert!(
        errors.contains("stored its content other than it was sent"),
        "{errors}"
    );
}

#[test]
fn a_rewrite_the_ruling_does_not_declare_is_refused_as_stored_content_diverged() {
    for cache in [false, true] {
        refused_as_diverged(cache);
    }
}

/// [`a_rewrite_the_ruling_does_not_declare_is_refused_as_stored_content_diverged`], with the
/// Linear source's cache on or off.
fn refused_as_diverged(cache: bool) {
    let plan = Plan::caching(cache);
    let (task, design) = plan.rendered_plan();
    plan.workspace
        .rewrite_markdown_on_write(LinearMarkdown::Undeclared);
    let (items, errors, _) = plan.copy_plan(&design, 4);
    let (written, stored) = LINEAR_UNDECLARED_REWRITE;
    for source in ["plan:goal", task.as_str(), design.as_str()] {
        let failed = outcome(&items, source);
        assert_eq!(failed["action"], "failed", "{failed}");
        assert_eq!(
            failed["error"]["kind"], "stored-content-diverged",
            "{failed}"
        );
        assert_eq!(failed["error"]["source"], "linear", "{failed}");
        let message = failed["error"]["message"].as_str().expect("a message");
        // The item is written, under an id the message names; the digest it records is still
        // that of what was sent, so its stored content does not hash to it.
        let destination = message
            .split_whitespace()
            .next()
            .expect("the message opens with the item's id");
        assert!(destination.starts_with("linear:"), "{message}");
        for named in [
            "first at line 12".to_owned(),
            format!("sent:   \"{written}Then review it.\""),
            format!("stored: \"{stored}Then review it.\""),
            "no declared rule of source linear explains the difference".to_owned(),
            "Ruling: what Linear does to Markdown on write, declared".to_owned(),
        ] {
            assert!(message.contains(&named), "names {named:?}: {message}");
        }
        assert!(errors.contains(destination), "{errors}");
        let item = plan.show(kind_of(source), destination);
        assert_ne!(
            item["metadata"]["onetaskgraph.template"]["body_digest"],
            sha256(item["content"].as_str().expect("content")),
            "{item}"
        );
        // Refused, so no link names the item as this copy's counterpart.
        let copied = plan.show(kind_of(source), source);
        assert!(
            copied["metadata"]["onetaskgraph.copies"]["linear"].is_null(),
            "cache {cache}: {copied}"
        );
    }
}

#[test]
fn a_rendering_edited_by_hand_keeps_its_provenance_unrestamped_on_the_way_to_linear() {
    let plan = Plan::new();
    let (task, _) = plan.rendered_plan();
    let file = plan
        .plan
        .join("tasks")
        .join(format!("{}.md", task.trim_start_matches("plan:")));
    let held = std::fs::read_to_string(&file).expect("the task file");
    std::fs::write(
        &file,
        held.replacen("- ship it\n", "- ship it\n- edited by hand\n", 1),
    )
    .expect("a hand edit");
    let recorded = plan.show("task", &task)["metadata"]["onetaskgraph.template"].clone();
    let (copied, errors, _) = plan.copy(&["task", "copy", &task, "--to", "linear"], 0);
    assert!(errors.is_empty(), "{errors}");
    let destination = copied["items"][0]["destination"]
        .as_str()
        .expect("an id")
        .to_owned();
    let item = plan.show("task", &destination);
    assert_eq!(
        item["metadata"]["onetaskgraph.template"], recorded,
        "a hand edit's provenance is carried as it was, never laundered"
    );
    let content = item["content"].as_str().expect("content");
    assert!(content.contains("* edited by hand\n"), "{content}");
    assert_ne!(recorded["body_digest"], sha256(content));
}

#[test]
fn items_created_and_rendered_in_linear_record_the_digest_of_what_linear_stores() {
    let plan = Plan::new();
    let created = [
        (
            "task",
            plan.printed(&[
                "task",
                "create",
                "linear",
                "--project",
                "P-1",
                "--title",
                "Built in Linear",
                "--template-loader",
                &plan.task,
                "--var",
                "goal=Built in Linear",
                "--no-interactive",
            ]),
            &plan.task,
        ),
        (
            "project",
            plan.printed(&[
                "project",
                "create",
                "linear",
                "--id",
                "planned-in-linear",
                "--title",
                "Planned in Linear",
                "--template-loader",
                &plan.project,
                "--var",
                "goal=Planned in Linear",
                "--no-interactive",
            ]),
            &plan.project,
        ),
        (
            "document",
            plan.printed(&[
                "document",
                "create",
                "linear",
                "--project",
                "P-1",
                "--title",
                "Designed in Linear",
                "--template-loader",
                &plan.design,
                "--var",
                "goal=Designed in Linear",
                "--no-interactive",
            ]),
            &plan.design,
        ),
    ];
    for (kind, id, loader) in &created {
        verifiable(&plan.show(kind, id), id);
        plan.exits(
            &[
                kind,
                "render",
                id,
                "--template-loader",
                loader,
                "--var",
                "goal=Rendered again",
                "--no-interactive",
            ],
            0,
        );
        let item = plan.show(kind, id);
        verifiable(&item, id);
        assert!(
            item["content"]
                .as_str()
                .is_some_and(|content| content.contains("Goal: Rendered again")),
            "{item}"
        );
    }

    plan.workspace
        .rewrite_markdown_on_write(LinearMarkdown::Undeclared);
    for (kind, id, loader) in &created {
        let (refused, _) = plan.exits(
            &[
                kind,
                "render",
                id,
                "--template-loader",
                loader,
                "--var",
                "goal=Never verifiable",
                "--no-interactive",
                "--json",
            ],
            1,
        );
        let failure: Value = serde_json::from_str(&stdout(&refused)).expect("a failure document");
        assert_eq!(
            failure["failure"]["kind"], "stored-content-diverged",
            "{kind} render: {failure}"
        );
    }
    for (kind, create) in [
        (
            "task",
            vec![
                "task",
                "create",
                "linear",
                "--project",
                "P-1",
                "--title",
                "Refused",
            ],
        ),
        (
            "project",
            vec![
                "project", "create", "linear", "--id", "refused", "--title", "Refused",
            ],
        ),
        (
            "document",
            vec![
                "document",
                "create",
                "linear",
                "--project",
                "P-1",
                "--title",
                "Refused",
            ],
        ),
    ] {
        let loader = match kind {
            "task" => &plan.task,
            "project" => &plan.project,
            _ => &plan.design,
        };
        let mut arguments = create;
        arguments.extend([
            "--template-loader",
            loader,
            "--var",
            "goal=Never verifiable",
            "--no-interactive",
            "--json",
        ]);
        let (refused, _) = plan.exits(&arguments, 1);
        let failure: Value = serde_json::from_str(&stdout(&refused)).expect("a failure document");
        assert_eq!(
            failure["failure"]["kind"], "stored-content-diverged",
            "{kind} create: {failure}"
        );
        assert!(
            stderr(&refused).contains("first at line 12"),
            "{}",
            stderr(&refused)
        );
    }
}

#[test]
fn a_copy_into_a_folder_of_markdown_writes_the_rendering_and_its_provenance_as_it_read_them() {
    let plan = Plan::new();
    let (task, design) = plan.rendered_plan();
    for (kind, source) in [("task", task.as_str()), ("document", design.as_str())] {
        let read = plan.show(kind, source);
        let (copied, errors, served) = plan.copy(&[kind, "copy", source, "--to", "back"], 0);
        assert!(errors.is_empty(), "{errors}");
        assert!(
            served.is_empty(),
            "a folder copy reaches no Linear: {served:#?}"
        );
        let destination = copied["items"][0]["destination"].as_str().expect("an id");
        let landed = plan.show(kind, destination);
        assert_eq!(
            landed["content"], read["content"],
            "a folder of Markdown declares no rewrite, so the bytes are the source's"
        );
        assert_eq!(
            landed["metadata"]["onetaskgraph.template"],
            read["metadata"]["onetaskgraph.template"]
        );
        assert_eq!(
            landed["metadata"]["onetaskgraph.template"]["body_digest"],
            sha256(landed["content"].as_str().expect("content"))
        );
        let file = plan
            .back
            .join(format!("{kind}s"))
            .join(format!("{}.md", destination.trim_start_matches("back:")));
        let written = std::fs::read_to_string(&file).expect("the copied file");
        assert!(
            written.contains("- build it\n- ship it\n| Step | Owner |\n")
                || written.contains("- build it\n- ship it\n\n| Step | Owner |\n| --- | --- |\n"),
            "{written}"
        );
    }
}
