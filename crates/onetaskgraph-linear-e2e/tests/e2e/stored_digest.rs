//! A rendering written to Linear records the digest of what Linear stores, through the binary.
//!
//! The loopback Linear is made to store every long form it is written as the Linear plugin's
//! ruling on what Linear does to Markdown on write declares — `- ` bullets come back `* `, a
//! delimiter row `| -- |`, a domain-like token linked, brackets and tildes escaped, `_y_` as
//! `*y*`, a list item's next lines run on under it, a fence and a lone image set apart by blank
//! lines — so a store recording the digest of what it *sent* would record one no read matches.
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

/// A folder of Markdown `plan` and a loopback Linear workspace `linear`
/// holding one project to file what is created there under, with a loader document for a plan
/// task, a project description and a design document.
struct Plan {
    sandbox: Sandbox,
    workspace: LinearWorkspace,
    plan: PathBuf,
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
        Self::configured(if cache {
            Hosting::Cached
        } else {
            Hosting::InProcess
        })
    }

    /// [`Plan::new`], with the Linear source reached as `hosting` says.
    fn configured(hosting: Hosting) -> Self {
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
        for kind in ["tasks", "projects", "documents"] {
            std::fs::create_dir_all(plan.join(kind)).expect("a folder");
        }
        let linear = match hosting {
            Hosting::InProcess => json!({"plugin": "linear", "config": config}),
            Hosting::Cached => json!({
                "plugin": "linear",
                "config": config,
                "cache": {"enabled": true, "dir": sandbox.subdirectory("cache")},
            }),
            // The Linear plugin of this build served by the binary's own stdio host, so every
            // request the store makes of it crosses the plugin protocol over a real pipe.
            Hosting::Stdio => json!({
                "plugin": "subprocess",
                "config": {
                    "command": onetaskgraph_e2e_support::binary(),
                    "args": ["plugin-serve", "linear"],
                    "secrets": ["LINEAR_API_KEY"],
                    "settings": config,
                },
            }),
        };
        sandbox.project_document(&document(&json!({
            "linear": linear,
            "plan": {"plugin": "local-md", "config": {"root": plan}},
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

/// How the store reaches the Linear plugin.
#[derive(Debug, Clone, Copy)]
enum Hosting {
    /// Built in process, as `plugin: linear` configures it.
    InProcess,
    /// Built in process behind the source's local cache.
    Cached,
    /// Spawned and spoken to over the stdio plugin protocol, as `plugin: subprocess` running
    /// `onetaskgraph plugin-serve linear` configures it.
    Stdio,
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

/// The Markdown the Linear plugin's live probe sent real Linear in CI run 39361674659, and two
/// images on adjacent lines as the asset journey of that run wrote them.
const OBSERVED_SENT: &str = "- bullet\n| a | b |\n| --- | --- |\nsee https://example.com/a.\n\
example.com and \"caller.live\"\n[x] done ~5 _y_\nsay \\\"hi\\\" a \\d b\n-->\n```\n\
- [x] _y_ example.com ~\n```\nkept `- [x] _y_ example.com ~` span\n\n\
![alt 0](https://uploads.linear.app/a)\n![alt 1](https://uploads.linear.app/b)\n";

/// What real Linear stored of [`OBSERVED_SENT`], quoted from the logs of that run and of CI run
/// 39375395042: every line after the bullet run on under it, a blank line either side of the
/// fence — and of the second image, the empty line the asset journey's refusal quoted at line 2
/// — and nothing after the second image, where the later run's refusal quoted the content as
/// ending at that line.
const OBSERVED_STORED: &str = concat!(
    "* bullet\n  | a | b |\n  | --- | --- |\n",
    "  see [https://example.com/a](<https://example.com/a>).\n",
    "  [example.com](<http://example.com>) and \"[caller.live](<http://caller.live>)\"\n",
    "  \\[x\\] done \\~5 *y*\n  say \"hi\" a \\\\d b\n  \\-->\n\n```\n- [x] _y_ example.com ~\n```\n\n",
    "kept `- [x] _y_ example.com ~` span\n\n",
    "![alt 0](https://uploads.linear.app/a)\n\n![alt 1](https://uploads.linear.app/b)",
);

#[test]
fn a_rendering_holding_what_real_linear_rewrote_is_copied_as_linear_stores_it() {
    let plan = Plan::new();
    plan.rendered_plan();
    let inputs = plan.sandbox.subdirectory("observed");
    let source = format!(
        "---\nonetaskgraph_template: 1\nvariables:\n  goal:\n    description: What it is for\n\
         ---\nGoal: {{{{ goal }}}}\n\n{OBSERVED_SENT}"
    );
    let path = inputs.join("observed.json");
    std::fs::write(
        &path,
        json!({
            "reference": "host:observed",
            "entry": "entry.md",
            "templates": [{"name": "entry.md", "source": source}],
        })
        .to_string(),
    )
    .expect("a loader document");
    let task = plan.printed(&[
        "task",
        "create",
        "plan",
        "--project",
        "goal",
        "--title",
        "Observed",
        "--template-loader",
        &path.display().to_string(),
        "--var",
        "goal=Hold what Linear did",
        "--no-interactive",
    ]);
    let (report, errors, _) = plan.copy(&["task", "copy", &task, "--to", "linear"], 0);
    assert!(errors.is_empty(), "{errors}");
    let items = report["items"].as_array().expect("items");
    let landed = outcome(items, &task);
    assert_eq!(landed["action"], "created", "{landed}");
    let item = plan.show("task", landed["destination"].as_str().expect("an id"));
    let content = item["content"].as_str().expect("content");
    assert_eq!(
        content,
        format!("Goal: Hold what Linear did\n\n{OBSERVED_STORED}"),
        "the copy sends, and the loopback stores, exactly what real Linear stored"
    );
    assert_eq!(
        item["metadata"]["onetaskgraph.template"]["body_digest"],
        sha256(content),
        "{item}"
    );
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
    for hosting in [Hosting::InProcess, Hosting::Cached] {
        refused_as_diverged(hosting);
    }
}

/// [`a_rewrite_the_ruling_does_not_declare_is_refused_as_stored_content_diverged`], with the
/// Linear source reached as `hosting` says.
fn refused_as_diverged(hosting: Hosting) {
    let plan = Plan::configured(hosting);
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
            "{hosting:?}: {copied}"
        );
    }
}

/// The Linear plugin served over the stdio plugin protocol declares its rewrite, and says what
/// Linear stored, across the pipe: a copied plan reads back as the digest it records, a repeat
/// copy of it writes nothing, and a rewrite the ruling does not declare is refused as it is in
/// process.
#[test]
fn over_the_stdio_protocol_linear_stores_what_the_copy_records_and_refuses_what_it_cannot_explain()
{
    let plan = Plan::configured(Hosting::Stdio);
    let (task, design) = plan.rendered_plan();
    let (items, errors, _) = plan.copy_plan(&design, 0);
    assert!(errors.is_empty(), "{errors}");
    for source in ["plan:goal", task.as_str(), design.as_str()] {
        let landed = outcome(&items, source);
        assert_eq!(landed["action"], "created", "{landed}");
        let destination = landed["destination"].as_str().expect("an id");
        verifiable(&plan.show(kind_of(source), destination), destination);
    }

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
        "an unchanged plan writes nothing over the pipe: {updates:?}"
    );
    for item in project["items"]
        .as_array()
        .into_iter()
        .chain(document["items"].as_array())
        .flatten()
    {
        assert_eq!(item["action"], "unchanged", "{item}");
    }

    refused_as_diverged(Hosting::Stdio);
}

/// An item that depends on another of the same copy lands first without that edge and is
/// written again once its far end exists; that second write's answer is compared exactly as the
/// first's was, so a rewrite the ruling does not declare, applied only to it, is still refused.
#[test]
fn a_repair_write_linear_stores_otherwise_is_refused_as_stored_content_diverged() {
    let plan = Plan::new();
    let (written, stored) = LINEAR_UNDECLARED_REWRITE;
    let tasks = plan.plan.join("tasks");
    std::fs::write(
        tasks.join("A.md"),
        format!(
            "---\ntitle: Waits on its upstream\nstatus: todo\ndepends_on: [B]\n---\n\
             Step one.\n{written}Then review it.\n"
        ),
    )
    .expect("a task file");
    std::fs::write(
        tasks.join("B.md"),
        "---\ntitle: Upstream\nstatus: todo\n---\nDone first.\n",
    )
    .expect("a task file");
    // Linear applies the undeclared rewrite from the moment it has created the second issue —
    // after every first write of this copy and before the repair of the first.
    plan.workspace.after_request(
        |query, _| query.contains("issueCreate("),
        1,
        |workspace| workspace.rewrite_markdown_on_write(LinearMarkdown::Undeclared),
    );
    let (copied, errors, served) =
        plan.copy(&["task", "copy", "plan:A", "plan:B", "--to", "linear"], 4);
    let items = copied["items"].as_array().expect("items");
    assert_eq!(
        mutations(&served)
            .iter()
            .filter(|name| name.as_str() == "issueUpdate")
            .count(),
        1,
        "the first task is repaired once, after both were created: {served:#?}"
    );
    let landed = outcome(items, "plan:B");
    assert_eq!(landed["action"], "created", "{landed}");
    let failed = outcome(items, "plan:A");
    assert_eq!(failed["action"], "failed", "{failed}");
    assert_eq!(
        failed["error"]["kind"], "stored-content-diverged",
        "{failed}"
    );
    let message = failed["error"]["message"].as_str().expect("a message");
    let destination = message
        .split_whitespace()
        .next()
        .expect("the message opens with the item's id");
    assert!(destination.starts_with("linear:"), "{message}");
    for named in [
        "first at line 2".to_owned(),
        format!("sent:   \"{written}Then review it.\""),
        format!("stored: \"{stored}Then review it.\""),
    ] {
        assert!(message.contains(&named), "names {named:?}: {message}");
    }
    assert!(errors.contains(destination), "{errors}");
    // Refused, so no link names it as this copy's counterpart, while its upstream's does.
    assert!(
        plan.show("task", "plan:A")["metadata"]["onetaskgraph.copies"]["linear"].is_null(),
        "the refused task records no counterpart"
    );
    assert!(
        !plan.show("task", "plan:B")["metadata"]["onetaskgraph.copies"]["linear"].is_null(),
        "the task that landed records its counterpart"
    );
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

/// A create of `body` on the Linear source, with no template and no metadata, so no slot
/// follows the content: the command's output and the issue the workspace holds under `title`.
fn plain_create(plan: &Plan, title: &str, body: &str, code: i32) -> (Output, String) {
    let path = plan
        .sandbox
        .subdirectory("plain")
        .join(format!("{title}.md"));
    std::fs::write(&path, body).expect("a body");
    let (output, _) = plan.exits(
        &[
            "task",
            "create",
            "linear",
            "--project",
            "P-1",
            "--title",
            title,
            "--body-file",
            &path.display().to_string(),
            "--json",
        ],
        code,
    );
    let held = plan.workspace.issue_titled(title).expect("the issue");
    (output, held)
}

/// A field ending in a table keeps the table's last line break, as CI run 39375395042 observed
/// of real Linear, and the ruling declares it: the create sends it, the workspace holds it, and a
/// read hands it back, so nothing is removed between what Linear stores and what is compared.
#[test]
fn a_plain_create_ending_in_a_table_sends_and_reads_back_its_line_break() {
    let plan = Plan::new();
    let stored = "Owners:\n\n| Step | Owner |\n| -- | -- |\n";
    let (created, held) = plan_create_table(&plan);
    let id = serde_json::from_str::<Value>(&stdout(&created)).expect("JSON")["items"][0]["id"]
        .as_str()
        .expect("an id")
        .to_owned();
    assert_eq!(
        plan.workspace.long_form("tasks", &held).as_deref(),
        Some(stored)
    );
    assert_eq!(plan.show("task", &id)["content"], stored);
}

fn plan_create_table(plan: &Plan) -> (Output, String) {
    plain_create(
        plan,
        "Ends in a table",
        "Owners:\n\n\n| Step | Owner |\n| --- | --- |",
        0,
    )
}

/// A line break Linear stores that no declared rule explains is refused, never taken off: the
/// workspace holds it, the create exits naming the line, and a read hands the byte back.
#[test]
fn a_trailing_line_break_no_rule_declares_is_refused_rather_than_removed() {
    let plan = Plan::new();
    plan.workspace
        .rewrite_markdown_on_write(LinearMarkdown::TrailingLineBreak);
    let (refused, held) = plain_create(&plan, "Ends in prose", "Owners: done", 1);
    let failure: Value = serde_json::from_str(&stdout(&refused)).expect("a failure document");
    assert_eq!(
        failure["failure"]["kind"], "stored-content-diverged",
        "{failure}"
    );
    let message = stderr(&refused);
    assert!(
        message.contains("first at line 2")
            && message.contains("sent:   no line 2")
            && message.contains("stored: \"\""),
        "{message}"
    );
    assert_eq!(
        plan.workspace.long_form("tasks", &held).as_deref(),
        Some("Owners: done\n")
    );
    assert_eq!(
        plan.show("task", &format!("linear:{held}"))["content"],
        "Owners: done\n",
        "a read hands back what Linear stores"
    );
}

/// A rendering ending in a table, copied to Linear with its slot after it: the table keeps its
/// line break, the read content hashes to the digest the provenance records, and a second copy
/// of the unchanged rendering writes nothing.
#[test]
fn a_rendering_ending_in_a_table_copies_to_linear_verifiably_and_recopies_as_nothing() {
    let plan = Plan::new();
    let path = plan.sandbox.subdirectory("table").join("table.json");
    let source = "---\nonetaskgraph_template: 1\nvariables:\n  goal:\n    description: What it \
                  is for\n---\nGoal: {{ goal }}\n\n| Step | Owner |\n| --- | --- |\n| build | \
                  ops |\n";
    std::fs::write(
        &path,
        json!({
            "reference": "host:table",
            "entry": "entry.md",
            "templates": [{"name": "entry.md", "source": source}],
        })
        .to_string(),
    )
    .expect("a loader document");
    plan.rendered_plan();
    let task = plan.printed(&[
        "task",
        "create",
        "plan",
        "--project",
        "goal",
        "--title",
        "Tabled",
        "--template-loader",
        &path.display().to_string(),
        "--var",
        "goal=End in a table",
        "--no-interactive",
    ]);
    let (report, errors, _) = plan.copy(&["task", "copy", &task, "--to", "linear"], 0);
    assert!(errors.is_empty(), "{errors}");
    let landed = outcome(report["items"].as_array().expect("items"), &task);
    assert_eq!(landed["action"], "created", "{landed}");
    let destination = landed["destination"].as_str().expect("an id").to_owned();
    let item = plan.show("task", &destination);
    let content = item["content"].as_str().expect("content");
    assert!(
        content.ends_with("| Step | Owner |\n| -- | -- |\n| build | ops |\n"),
        "{content:?}"
    );
    assert_eq!(
        item["metadata"]["onetaskgraph.template"]["body_digest"],
        sha256(content),
        "{item}"
    );
    let (again, _, served) = plan.copy(&["task", "copy", &task, "--to", "linear"], 0);
    assert_eq!(
        outcome(again["items"].as_array().expect("items"), &task)["action"],
        "unchanged",
        "{again}"
    );
    assert!(
        !mutations(&served).iter().any(|root| root == "issueUpdate"),
        "an unchanged table writes nothing: {:?}",
        mutations(&served)
    );
}
