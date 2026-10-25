//! A realistic plan copied into Linear, and items created and rendered there, through the binary,
//! against a loopback Linear that rewrites every long form as spike `spike-linear-stored-content`
//! measured real Linear rewriting it.
//!
//! The plan is one project, five tasks and one design document holding two images on adjacent
//! lines, each rendered in a folder of Markdown from a template composed of the committed
//! per-rewrite samples (`linear_rewrites`). Every journey reads what it asserts on back through
//! the binary, hashes it here, and reads the loopback's request log for what was sent: each item
//! the loopback rewrote takes exactly one follow-up write and no read after either write, and a
//! copy or a render of something unchanged at both ends writes nothing.
//!
//! The realistic first copy's request figure is what `measure_linear_requests_per_plan_copy` in
//! `linear_budget` records for the `linear-requests-per-plan-copy` budget.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::Output;

use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use crate::common::{stderr, stdout};
use crate::fixtures::LinearMarkdown;
use crate::linear_assets::Journey;
use crate::linear_rewrites::{SAMPLES, sample};

/// `sha256:` and the hex SHA-256 of `content`, hashed here.
fn sha256(content: &str) -> String {
    let digest = Sha256::digest(content.as_bytes());
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("sha256:{hex}")
}

/// The long-form mutations, by their root field.
const CONTENT_WRITES: [&str; 6] = [
    "issueCreate",
    "issueUpdate",
    "projectCreate",
    "projectUpdate",
    "documentCreate",
    "documentUpdate",
];

/// One GraphQL request the loopback answered, with its answer.
struct Request {
    query: String,
    variables: Value,
    answer: Value,
}

impl Request {
    /// Its first root field, an aliased one by its alias.
    fn root(&self) -> String {
        self.query
            .split_once('{')
            .and_then(|(_, rest)| {
                rest.trim_start()
                    .split(['(', ' ', '{', ':'])
                    .next()
                    .map(str::to_owned)
            })
            .unwrap_or_default()
    }

    fn mutation(&self) -> bool {
        self.query.trim_start().starts_with("mutation")
    }

    /// The Linear id a long-form mutation wrote: the one it named, or the one it created.
    fn written(&self) -> Option<String> {
        let root = self.root();
        if !self.mutation() || !CONTENT_WRITES.contains(&root.as_str()) {
            return None;
        }
        self.variables["id"]
            .as_str()
            .map(str::to_owned)
            .or_else(|| {
                let payload = ["issue", "project", "document"]
                    .into_iter()
                    .find_map(|payload| self.answer["data"][&root][payload]["id"].as_str());
                payload.map(str::to_owned)
            })
    }
}

/// Everything one command sent the loopback Linear: its GraphQL requests, and its asset requests
/// by kind.
struct Sent {
    requests: Vec<Request>,
    uploads: usize,
    puts: usize,
    gets: usize,
}

impl Sent {
    /// Every request, GraphQL and asset alike.
    fn total(&self) -> usize {
        self.requests.len() + self.uploads + self.puts + self.gets
    }

    /// Every long-form write, by the Linear id it wrote, in the order sent.
    fn writes(&self) -> Vec<String> {
        self.requests.iter().filter_map(Request::written).collect()
    }

    /// How many long-form writes named `id`.
    fn writes_of(&self, id: &str) -> usize {
        self.writes()
            .iter()
            .filter(|written| *written == id)
            .count()
    }

    /// The roots of every request sent before the first mutation.
    fn before_the_first_write(&self) -> Vec<String> {
        self.requests
            .iter()
            .take_while(|request| !request.mutation())
            .map(Request::root)
            .collect()
    }

    /// Every query naming, in its text or its variables, an item a write of this command wrote
    /// before it — a read after a write.
    fn reads_after_a_write(&self) -> Vec<String> {
        let mut written: BTreeSet<String> = BTreeSet::new();
        let mut reads = Vec::new();
        for request in &self.requests {
            if let Some(id) = request.written() {
                written.insert(id);
                continue;
            }
            if request.mutation() {
                continue;
            }
            let named = request.variables.to_string();
            if written
                .iter()
                .any(|id| named.contains(id.as_str()) || request.query.contains(id.as_str()))
            {
                reads.push(request.root());
            }
        }
        reads
    }

    /// The roots of every request, in the order sent.
    fn roots(&self) -> Vec<String> {
        self.requests.iter().map(Request::root).collect()
    }

    /// The roots of every mutation.
    fn mutations(&self) -> Vec<String> {
        self.requests
            .iter()
            .filter(|request| request.mutation())
            .map(Request::root)
            .collect()
    }
}

/// A folder of Markdown `notes` holding the realistic plan, and the loopback Linear `dest`.
pub(crate) struct Plan {
    journey: Journey,
    inputs: PathBuf,
    /// The five tasks' qualified ids, in the order they were created.
    tasks: Vec<String>,
    /// The design document's qualified id.
    design: String,
}

/// A loader document whose one template is `body` under a `goal` variable.
fn loader(reference: &str, body: &str) -> String {
    let source = format!(
        "---\nonetaskgraph_template: 1\nvariables:\n  goal:\n    description: What it is for\n\
         ---\n{body}"
    );
    json!({
        "reference": reference,
        "entry": "entry.md",
        "templates": [{"name": "entry.md", "source": source}],
    })
    .to_string()
}

/// The samples, `from` of them on, each set apart by a blank line.
fn samples(from: usize) -> String {
    SAMPLES
        .iter()
        .skip(from)
        .map(|name| sample(name))
        .collect::<Vec<_>>()
        .join("\n\n")
}

impl Plan {
    /// The plan, rendered in `notes`, with the loopback Linear storing long forms as `markdown`.
    pub(crate) fn new(markdown: LinearMarkdown) -> Self {
        let journey = Journey::new();
        journey.workspace.held.rewrite_markdown_on_write(markdown);
        let inputs = journey.sandbox.subdirectory("plan-inputs");
        let write = |name: &str, text: String| {
            let path = inputs.join(name);
            std::fs::write(&path, text).unwrap();
            path.to_string_lossy().into_owned()
        };
        let project = write(
            "plan.json",
            loader(
                "host:plan-description",
                &format!("# Plan\n\nGoal: {{{{ goal }}}}\n\n{}\n", samples(4)),
            ),
        );
        let task = write(
            "plan-task.json",
            loader(
                "host:plan-task",
                &format!("# Plan task\n\nGoal: {{{{ goal }}}}\n\n{}\n", samples(0)),
            ),
        );
        let design = write(
            "design.json",
            loader(
                "host:design-document",
                &format!(
                    "# Design\n\nGoal: {{{{ goal }}}}\n\n{}\n\n### Screens\n{}\n",
                    samples(0),
                    sample("adjacent-images.md")
                ),
            ),
        );
        for image in ["first.png", "second.png"] {
            std::fs::copy(
                format!(
                    "{}/tests/fixtures/linear_rewrites/{image}",
                    env!("CARGO_MANIFEST_DIR")
                ),
                inputs.join(image),
            )
            .unwrap();
        }
        let mut plan = Self {
            journey,
            inputs: inputs.clone(),
            tasks: Vec::new(),
            design: String::new(),
        };
        plan.printed(&[
            "project",
            "create",
            "notes",
            "--id",
            "plan",
            "--title",
            "Plan",
            "--template-loader",
            &project,
            "--var",
            "goal=Ship the plan",
            "--no-interactive",
        ]);
        for index in 0..5 {
            let title = format!("Plan task {index}");
            let goal = format!("goal=Do part {index}");
            let id = plan.printed(&[
                "task",
                "create",
                "notes",
                "--project",
                "plan",
                "--title",
                &title,
                "--template-loader",
                &task,
                "--var",
                &goal,
                "--no-interactive",
            ]);
            plan.tasks.push(id);
        }
        let first = inputs.join("first.png").to_string_lossy().into_owned();
        let second = inputs.join("second.png").to_string_lossy().into_owned();
        plan.design = plan.printed(&[
            "document",
            "create",
            "notes",
            "--project",
            "plan",
            "--title",
            "Design",
            "--template-loader",
            &design,
            "--var",
            "goal=Design the plan",
            "--asset",
            &first,
            "--asset",
            &second,
            "--no-interactive",
        ]);
        plan
    }

    /// One command, and everything it sent the loopback Linear.
    fn run(&self, arguments: &[&str]) -> (Output, Sent) {
        let workspace = &self.journey.workspace;
        let from = workspace.held.served().len();
        let (uploads, puts, gets) = workspace.counts();
        let output = self
            .journey
            .sandbox
            .command()
            .envs(workspace.clock.client_env(0))
            .args(arguments)
            .output()
            .unwrap();
        let served = workspace.held.served();
        let answered = workspace.held.answered();
        let (uploads_after, puts_after, gets_after) = workspace.counts();
        let requests = served[from..]
            .iter()
            .zip(&answered[from..])
            .map(|((query, variables), answer)| Request {
                query: query.clone(),
                variables: variables.clone(),
                answer: answer.clone(),
            })
            .collect();
        (
            output,
            Sent {
                requests,
                uploads: uploads_after - uploads,
                puts: puts_after - puts,
                gets: gets_after - gets,
            },
        )
    }

    /// A command that has to succeed: what it printed, trimmed.
    fn printed(&self, arguments: &[&str]) -> String {
        let (output, _) = self.run(arguments);
        assert!(
            output.status.success(),
            "`onetaskgraph {}`\n{}{}",
            arguments.join(" "),
            stdout(&output),
            stderr(&output)
        );
        stdout(&output).trim().to_owned()
    }

    /// A command that has to succeed, answering its JSON and what it sent.
    fn json(&self, arguments: &[&str]) -> (Value, Sent) {
        let mut all = vec!["--json"];
        all.extend_from_slice(arguments);
        let (output, sent) = self.run(&all);
        assert!(
            output.status.success(),
            "`onetaskgraph {}`\n{}{}",
            all.join(" "),
            stdout(&output),
            stderr(&output)
        );
        (serde_json::from_str(&stdout(&output)).unwrap(), sent)
    }

    /// One item as `<kind> show --json` reads it.
    fn show(&self, kind: &str, id: &str) -> Value {
        self.json(&[kind, "show", id]).0["items"][0]["item"].clone()
    }

    /// The project copy, then the document copy: each report and what each sent.
    fn copy_plan(&self) -> ((Value, Sent), (Value, Sent)) {
        (
            self.json(&["project", "copy", "notes:plan", "--to", "dest"]),
            self.json(&["document", "copy", &self.design, "--to", "dest"]),
        )
    }

    /// Where the item copied from `source` landed, by the report `report`, unqualified.
    fn landed(report: &Value, source: &str) -> String {
        report["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["source"] == source)
            .and_then(|item| item["destination"].as_str())
            .unwrap_or_else(|| panic!("{source} landed: {report}"))
            .split_once(':')
            .unwrap()
            .1
            .to_owned()
    }

    /// Every source id of the plan with its kind.
    fn items(&self) -> Vec<(&'static str, String)> {
        std::iter::once(("project", "notes:plan".to_owned()))
            .chain(self.tasks.iter().map(|task| ("task", task.clone())))
            .chain(std::iter::once(("document", self.design.clone())))
            .collect()
    }

    /// The path of the folder's file for the item `id` of `kind`.
    fn file(&self, kind: &str, id: &str) -> PathBuf {
        self.journey
            .sandbox
            .subdirectory("notes")
            .join(format!("{kind}s"))
            .join(format!("{}.md", id.trim_start_matches("notes:")))
    }
}

/// What the realistic plan's first copy into an empty Linear sent, as the budget reports it.
pub(crate) struct FirstCopy {
    pub(crate) requests: usize,
    pub(crate) detail: String,
}

/// The realistic plan copied into an empty loopback Linear that rewrites every long form, then
/// copied again unchanged, then after one task's source is edited, then after the document's.
///
/// Every item lands; each records a `body_digest` that is the SHA-256 of its content read back
/// afterwards, and the copy record on all three kinds; each item the loopback rewrote takes
/// exactly one follow-up write and no read after either write; the project copy sends exactly
/// one request before its first write; the document copy reads no destination project; the two
/// images take one `fileUpload` and one PUT each and no GET. An unchanged repeat writes nothing
/// and sends no asset request, and each later copy writes the one item edited at its source.
pub(crate) fn first_copy_of_the_realistic_plan() -> FirstCopy {
    let plan = Plan::new(LinearMarkdown::Measured);
    let ((project_report, project_sent), (document_report, document_sent)) = plan.copy_plan();
    for report in [&project_report, &document_report] {
        for item in report["items"].as_array().unwrap() {
            assert_eq!(item["action"], "created", "{item}");
        }
    }
    let landed: Vec<(&str, String, String)> = plan
        .items()
        .into_iter()
        .map(|(kind, source)| {
            let report = if kind == "document" {
                &document_report
            } else {
                &project_report
            };
            let destination = Plan::landed(report, &source);
            (kind, source, destination)
        })
        .collect();

    // Every item verifiable from what a read hands back, and carrying the copy record.
    for (kind, source, destination) in &landed {
        let item = plan.show(kind, &format!("dest:{destination}"));
        let content = item["content"].as_str().expect("content");
        assert_eq!(
            item["metadata"]["onetaskgraph.template"]["body_digest"],
            sha256(content),
            "{source} reads back as the content its provenance records:\n{content}"
        );
        for key in [
            "onetaskgraph.copied_version",
            "onetaskgraph.copied_destination_version",
        ] {
            assert!(
                item["metadata"][key]
                    .as_str()
                    .is_some_and(|value| !value.is_empty()),
                "{source} carries {key}: {item}"
            );
        }
        // Linear rewrote it, so what it holds is not what was composed.
        assert!(!content.contains("\n- first item\n"), "{source}: {content}");
    }

    // Each item rewritten takes one content write and exactly one follow-up, and nothing is read
    // after either.
    for (kind, source, destination) in &landed {
        let sent = if *kind == "document" {
            &document_sent
        } else {
            &project_sent
        };
        assert_eq!(
            sent.writes_of(destination),
            2,
            "{source}: one write as composed and one follow-up: {:?}",
            sent.writes()
        );
    }
    for sent in [&project_sent, &document_sent] {
        assert_eq!(sent.reads_after_a_write(), Vec::<String>::new());
    }
    assert_eq!(
        project_sent.before_the_first_write(),
        ["originIssues"],
        "one request answers every origin lookup, the organization and the resolution"
    );
    assert!(
        project_sent
            .requests
            .iter()
            .all(|request| request.mutation() || request.root() == "originIssues"),
        "no issues or projects origin query of its own: {:?}",
        project_sent
            .requests
            .iter()
            .map(Request::root)
            .collect::<Vec<_>>()
    );
    assert_eq!(document_sent.before_the_first_write(), ["documents"]);
    assert!(
        !document_sent
            .requests
            .iter()
            .any(|request| request.query.contains("project(id:")),
        "the document copy reads no destination project"
    );
    assert_eq!(
        (
            document_sent.uploads,
            document_sent.puts,
            document_sent.gets
        ),
        (2, 2, 0),
        "each new image is one fileUpload and one PUT, and nothing reads it back"
    );
    let design = plan.show("document", &format!("dest:{}", landed[6].2));
    let content = design["content"].as_str().unwrap();
    assert_eq!(
        content
            .matches(&format!("]({}/0/asset/", plan.journey.workspace.endpoint))
            .count(),
        2,
        "both images are in the content read back: {content}"
    );
    assert!(!content.contains("](./"), "{content}");

    let requests = project_sent.total() + document_sent.total();
    let follow_ups = landed.len();
    let detail = format!(
        "1 project, 5 tasks and 1 design document with 2 adjacent new images, copied into an \
         empty loopback Linear that rewrites every long form: {} origin lookup before the \
         project copy's first write, 1 documents lookup, {} content writes, {follow_ups} \
         follow-up writes, {} reads after a write, 0 destination-project reads, {} asset requests \
         ({} fileUpload, {} PUT, {} GET)",
        project_sent.before_the_first_write().len(),
        project_sent.writes().len() + document_sent.writes().len() - follow_ups,
        project_sent.reads_after_a_write().len() + document_sent.reads_after_a_write().len(),
        document_sent.uploads + document_sent.puts + document_sent.gets,
        document_sent.uploads,
        document_sent.puts,
        document_sent.gets,
    );

    // An unchanged repeat writes nothing at all, and sends no asset request.
    let ((again, project_again), (document_again_report, document_again)) = plan.copy_plan();
    assert_eq!(project_again.mutations(), Vec::<String>::new(), "{again}");
    assert_eq!(
        document_again.mutations(),
        Vec::<String>::new(),
        "{document_again_report}"
    );
    assert_eq!(
        (
            document_again.uploads,
            document_again.puts,
            document_again.gets
        ),
        (0, 0, 0)
    );
    for item in again["items"]
        .as_array()
        .unwrap()
        .iter()
        .chain(document_again_report["items"].as_array().unwrap())
    {
        assert_eq!(item["action"], "unchanged", "{item}");
    }

    // A task edited at its source: the next copy writes that task alone.
    let edited = &landed[2];
    let file = plan.file("task", &edited.1);
    let held = std::fs::read_to_string(&file).unwrap();
    std::fs::write(
        &file,
        format!("{}\nEdited at the source.\n", held.trim_end()),
    )
    .unwrap();
    let (_, third) = plan.json(&["project", "copy", "notes:plan", "--to", "dest"]);
    assert_eq!(
        third.writes(),
        [edited.2.clone(), edited.2.clone()],
        "the edited task alone, with its follow-up"
    );
    assert_eq!(third.reads_after_a_write(), Vec::<String>::new());

    // The design document edited at its source: the next copy writes it alone.
    let file = plan.file("document", &plan.design);
    let held = std::fs::read_to_string(&file).unwrap();
    std::fs::write(
        &file,
        held.replacen("Goal: Design the plan", "Goal: Design the plan again", 1),
    )
    .unwrap();
    let (_, fourth) = plan.json(&["document", "copy", &plan.design, "--to", "dest"]);
    assert_eq!(
        fourth.writes(),
        [landed[6].2.clone(), landed[6].2.clone()],
        "the edited document alone, with its follow-up"
    );
    assert_eq!(
        (fourth.uploads, fourth.puts, fourth.gets),
        (0, 0, 0),
        "its unchanged images are not sent again"
    );

    FirstCopy { requests, detail }
}

#[test]
fn edits_made_in_linear_are_written_back_to_what_the_source_holds_and_nothing_else_is_written() {
    let plan = Plan::new(LinearMarkdown::Measured);
    let ((project_report, _), (document_report, _)) = plan.copy_plan();
    let project = Plan::landed(&project_report, "notes:plan");
    let edited = Plan::landed(&project_report, &plan.tasks[0]);
    let other = Plan::landed(&project_report, &plan.tasks[1]);
    let design = Plan::landed(&document_report, &plan.design);
    let held = &plan.journey.workspace.held;
    let before = |kind: &str, id: &str| plan.show(kind, &format!("dest:{id}"));
    let (project_before, task_before, design_before) = (
        before("project", &project),
        before("task", &edited),
        before("document", &design),
    );
    let done = held
        .states()
        .into_iter()
        .find(|(_, kind)| kind == "completed")
        .expect("a completed state");
    held.prepend_long_form("projects", &project, "Edited in Linear.\n\n");
    held.prepend_long_form("documents", &design, "Edited in Linear.\n\n");
    held.retitle("tasks", &edited, "Renamed in Linear");
    held.move_to_state(&edited, &done.0, &done.1);
    held.relate(&edited, &other, "blocks");

    let ((_, project_sent), (_, document_sent)) = plan.copy_plan();
    let written: BTreeSet<String> = project_sent
        .writes()
        .into_iter()
        .chain(document_sent.writes())
        .collect();
    assert_eq!(
        written,
        BTreeSet::from([project.clone(), edited.clone(), design.clone()]),
        "each edited item is written back, and no other"
    );
    assert!(
        project_sent
            .mutations()
            .iter()
            .any(|root| root == "issueRelationDelete"),
        "the edge added in Linear is taken off: {:?}",
        project_sent.mutations()
    );
    let task = before("task", &edited);
    assert_eq!(task["title"], task_before["title"], "{task}");
    assert_eq!(task["status"], task_before["status"], "{task}");
    assert_eq!(task["content"], task_before["content"]);
    for (kind, id, was) in [
        ("project", &project, &project_before),
        ("document", &design, &design_before),
    ] {
        let now = before(kind, id);
        assert_eq!(now["content"], was["content"], "{kind} written back");
        assert_eq!(
            now["metadata"]["onetaskgraph.template"]["body_digest"],
            sha256(now["content"].as_str().unwrap())
        );
    }
    // And the next copy, with nothing edited since, writes nothing.
    let ((_, project_again), (_, document_again)) = plan.copy_plan();
    assert_eq!(project_again.mutations(), Vec::<String>::new());
    assert_eq!(document_again.mutations(), Vec::<String>::new());
}

#[test]
fn a_task_linear_stores_as_sent_takes_one_content_write_and_no_read_after_it() {
    let plan = Plan::new(LinearMarkdown::Verbatim);
    let (report, sent) = plan.json(&["task", "copy", &plan.tasks[0], "--to", "dest"]);
    let destination = Plan::landed(&report, &plan.tasks[0]);
    assert_eq!(
        sent.writes(),
        std::slice::from_ref(&destination),
        "{:?}",
        sent.mutations()
    );
    assert_eq!(sent.reads_after_a_write(), Vec::<String>::new());
    let item = plan.show("task", &format!("dest:{destination}"));
    assert_eq!(
        item["metadata"]["onetaskgraph.template"]["body_digest"],
        sha256(item["content"].as_str().unwrap())
    );
}

/// One change a re-render follows, made and rendered by the closure.
type Change<'a> = Box<dyn Fn() -> Sent + 'a>;

/// One create or render on the Linear source: its writes and whether it read after one.
fn rendering(plan: &Plan, arguments: &[&str]) -> Sent {
    let (output, sent) = plan.run(arguments);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        sent.reads_after_a_write(),
        Vec::<String>::new(),
        "`{}` reads nothing after its write: {:?}",
        arguments.join(" "),
        sent.roots()
    );
    sent
}

/// Assert the item `id` of `kind` reads back as content whose digest its provenance records.
fn verifiable(plan: &Plan, kind: &str, id: &str) {
    let item = plan.show(kind, id);
    let content = item["content"].as_str().expect("content");
    assert_eq!(
        item["metadata"]["onetaskgraph.template"]["body_digest"],
        sha256(content),
        "{kind} {id}:\n{content}"
    );
}

#[test]
fn items_created_and_rendered_in_linear_record_what_linear_stores_and_rerender_only_when_changed() {
    let plan = Plan::new(LinearMarkdown::Measured);
    let held = &plan.journey.workspace.held;
    let inputs = &plan.inputs;
    // A task's and a document's template references one image, `shot.png`: two files of that
    // name with different bytes are its asset set before and after it changes.
    let write_loader = |name: &str, heading: &str, image: bool| {
        let path = inputs.join(name);
        let shot = if image { "\n\n![shot](./shot.png)" } else { "" };
        std::fs::write(
            &path,
            loader(
                &format!("host:{name}"),
                &format!(
                    "# {heading}\n\nGoal: {{{{ goal }}}}\n\n{}{shot}\n",
                    samples(0)
                ),
            ),
        )
        .unwrap();
        path.to_string_lossy().into_owned()
    };
    let shot = |image: &str, directory: &str| {
        let directory = inputs.join(directory);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::copy(inputs.join(image), directory.join("shot.png")).unwrap();
        directory.join("shot.png").to_string_lossy().into_owned()
    };
    let first = shot("first.png", "shot-first");
    let second = shot("second.png", "shot-second");
    let project_loader = write_loader("home.json", "Home", false);
    // Each create and render runs without `--json`, whose output for a create is defined as the
    // `show --json` of what was created and so reads it: the command's own writes read nothing.
    let created = plan.run(&[
        "project",
        "create",
        "dest",
        "--id",
        "home",
        "--title",
        "Home",
        "--template-loader",
        &project_loader,
        "--var",
        "goal=one",
        "--no-interactive",
    ]);
    assert!(created.0.status.success(), "{}", stderr(&created.0));
    let home = stdout(&created.0).trim().to_owned();
    let home_native = home.split_once(':').unwrap().1.to_owned();
    assert!(created.1.writes().len() <= 2, "{:?}", created.1.mutations());
    assert_eq!(
        created.1.reads_after_a_write(),
        Vec::<String>::new(),
        "{:?}",
        created.1.roots()
    );
    verifiable(&plan, "project", &home);

    for (kind, collection, carries_assets) in [
        ("task", "tasks", true),
        ("document", "documents", true),
        ("project", "projects", false),
    ] {
        let loader_path = write_loader(&format!("{kind}.json"), kind, carries_assets);
        let assets = |image: &str| -> Vec<String> {
            if carries_assets {
                vec!["--asset".to_owned(), image.to_owned()]
            } else {
                Vec::new()
            }
        };
        // Created in Linear itself — a project's is the home made above.
        let id = if kind == "project" {
            home.clone()
        } else {
            let mut arguments: Vec<String> = [
                kind,
                "create",
                "dest",
                "--project",
                &home_native,
                "--title",
                "Made in Linear",
                "--template-loader",
                &loader_path,
                "--var",
                "goal=one",
                "--no-interactive",
            ]
            .iter()
            .map(|&argument| argument.to_owned())
            .collect();
            arguments.extend(assets(&first));
            let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
            let (created, sent) = plan.run(&arguments);
            assert!(created.status.success(), "{}", stderr(&created));
            assert_eq!(
                sent.reads_after_a_write(),
                Vec::<String>::new(),
                "{:?}",
                sent.roots()
            );
            assert_eq!(sent.writes().len(), 2, "{kind}: a create and its follow-up");
            stdout(&created).trim().to_owned()
        };
        verifiable(&plan, kind, &id);
        let native = id.split_once(':').unwrap().1.to_owned();
        let render = |loader: &str, goal: &str, image: &str| -> Sent {
            let goal = format!("goal={goal}");
            let mut arguments: Vec<String> = [
                kind,
                "render",
                &id,
                "--template-loader",
                loader,
                "--var",
                &goal,
                "--no-interactive",
            ]
            .iter()
            .map(|&argument| argument.to_owned())
            .collect();
            arguments.extend(assets(image));
            let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
            rendering(&plan, &arguments)
        };
        let project_or = |loader: &str| -> String {
            if kind == "project" {
                project_loader.clone()
            } else {
                loader.to_owned()
            }
        };
        let loader_path = project_or(&loader_path);
        // The same template, answers and assets over unedited content: nothing is written.
        let before = plan.show(kind, &id);
        let unchanged = render(&loader_path, "one", &first);
        assert_eq!(
            unchanged.mutations(),
            Vec::<String>::new(),
            "{kind}: {before:#}\n{:#}",
            plan.show(kind, &id)
        );
        // Each change re-renders it — one write and at most its one follow-up — and it reads
        // back verifiable.
        let mut changes: Vec<(&str, Change<'_>)> = vec![
            (
                "the answers",
                Box::new(|| render(&loader_path, "two", &first)),
            ),
            (
                "the template's digest",
                Box::new(|| {
                    let changed = inputs.join(format!("{kind}-changed.json"));
                    let text = std::fs::read_to_string(&loader_path).unwrap();
                    std::fs::write(&changed, text.replacen("Goal:", "Purpose:", 1)).unwrap();
                    render(&changed.to_string_lossy(), "two", &first)
                }),
            ),
            (
                "its content edited in Linear",
                Box::new(|| {
                    held.prepend_long_form(collection, &native, "Edited in Linear.\n\n");
                    let changed = inputs.join(format!("{kind}-changed.json"));
                    render(&changed.to_string_lossy(), "two", &first)
                }),
            ),
        ];
        if carries_assets {
            changes.push((
                "its asset set",
                Box::new(|| {
                    let changed = inputs.join(format!("{kind}-changed.json"));
                    render(&changed.to_string_lossy(), "two", &second)
                }),
            ));
        }
        for (what, change) in changes {
            let sent = change();
            let writes = sent.writes();
            assert!(
                (1..=2).contains(&writes.len()) && writes.iter().all(|written| *written == native),
                "{kind}: {what} re-renders it once: {:?}",
                sent.mutations()
            );
            verifiable(&plan, kind, &id);
        }
    }

    // A document replaced by `document create --id` is a rendering write as well.
    let loader_path = inputs.join("document.json").to_string_lossy().into_owned();
    let document = held
        .document_titled("Made in Linear")
        .expect("the document");
    let sent = rendering(
        &plan,
        &[
            "document",
            "create",
            "dest",
            "--project",
            &home_native,
            "--id",
            &document,
            "--title",
            "Made in Linear",
            "--template-loader",
            &loader_path,
            "--var",
            "goal=three",
            "--asset",
            &first,
            "--no-interactive",
        ],
    );
    assert!(
        (1..=2).contains(&sent.writes().len()),
        "{:?}",
        sent.mutations()
    );
    let qualified = format!("dest:{document}");
    verifiable(&plan, "document", &qualified);
    let unchanged = rendering(
        &plan,
        &[
            "document",
            "render",
            &qualified,
            "--template-loader",
            &loader_path,
            "--var",
            "goal=three",
            "--asset",
            &first,
            "--no-interactive",
        ],
    );
    assert_eq!(unchanged.mutations(), Vec::<String>::new());
}

#[test]
fn a_content_write_and_metadata_writes_on_copied_renderings_answer_what_linear_stores() {
    let plan = Plan::new(LinearMarkdown::Measured);
    let ((project_report, _), (document_report, _)) = plan.copy_plan();
    let task = format!("dest:{}", Plan::landed(&project_report, &plan.tasks[0]));
    for (kind, id) in [
        ("task", task.clone()),
        (
            "project",
            format!("dest:{}", Plan::landed(&project_report, "notes:plan")),
        ),
        (
            "document",
            format!("dest:{}", Plan::landed(&document_report, &plan.design)),
        ),
    ] {
        let before = plan.show(kind, &id);
        let (answer, sent) = plan.json(&[kind, "metadata", "set", &id, "myapp.approved", "true"]);
        assert_eq!(sent.mutations().len(), 1, "{kind}: {answer}");
        assert_eq!(sent.reads_after_a_write(), Vec::<String>::new(), "{kind}");
        assert!(
            sent.requests
                .iter()
                .skip_while(|request| !request.mutation())
                .skip(1)
                .all(Request::mutation),
            "{kind}: nothing follows the write"
        );
        assert_eq!(answer["value"], json!(true), "{kind}: {answer}");
        let after = plan.show(kind, &id);
        assert_eq!(after["content"], before["content"], "{kind}");
        assert_eq!(
            after["metadata"]["onetaskgraph.template"]["body_digest"],
            sha256(after["content"].as_str().unwrap()),
            "{kind}"
        );
    }

    // A content write answers the content Linear stored, read off the write's own answer.
    let body = plan.inputs.join("content.md");
    std::fs::write(&body, "- first item\n- second item\n").unwrap();
    let (answer, sent) = plan.json(&[
        "task",
        "content",
        "set",
        &task,
        "--file",
        &body.to_string_lossy(),
    ]);
    assert_eq!(answer["content"], "* first item\n* second item", "{answer}");
    assert_eq!(sent.mutations(), ["issueUpdate"]);
    assert!(
        sent.requests
            .iter()
            .skip_while(|request| !request.mutation())
            .nth(1)
            .is_none(),
        "nothing is read after the write"
    );
    assert_eq!(plan.show("task", &task)["content"], answer["content"]);
}

#[test]
fn a_document_filed_under_a_stale_link_is_refused_and_recreate_recovers_the_project() {
    let plan = Plan::new(LinearMarkdown::Measured);
    let (project_report, _) = plan.json(&["project", "copy", "notes:plan", "--to", "dest"]);
    let project = Plan::landed(&project_report, "notes:plan");
    // The local project's link names a project the destination no longer holds.
    let file = plan.file("project", "notes:plan");
    let held = std::fs::read_to_string(&file).unwrap();
    assert!(held.contains(&format!("dest:{project}")), "{held}");
    std::fs::write(
        &file,
        held.replace(&format!("dest:{project}"), "dest:P-GONE"),
    )
    .unwrap();
    plan.journey.workspace.held.refuse_unknown_projects();
    let documents = || {
        plan.journey
            .workspace
            .held
            .document_titled("Design")
            .into_iter()
            .count()
    };
    let (refused, _) = plan.run(&["--json", "document", "copy", &plan.design, "--to", "dest"]);
    assert_eq!(refused.status.code(), Some(1), "{}", stderr(&refused));
    let failure: Value = serde_json::from_str(&stdout(&refused)).unwrap_or(Value::Null);
    assert_eq!(failure["failure"]["kind"], "stale-link", "{failure}");
    assert!(
        stderr(&refused).contains("dest:P-GONE"),
        "names the link: {}",
        stderr(&refused)
    );
    assert_eq!(documents(), 0, "no document is left created");

    let (recreated, _) = plan.json(&[
        "document",
        "copy",
        &plan.design,
        "--to",
        "dest",
        "--recreate",
    ]);
    let document = Plan::landed(&recreated, &plan.design);
    assert_eq!(
        plan.journey.workspace.held.document_project(&document),
        Some(project.clone()),
        "filed under the project the walk recovered"
    );
    let source = plan.show("project", "notes:plan");
    assert_eq!(
        source["metadata"]["onetaskgraph.copies"]["dest"],
        format!("dest:{project}"),
        "the local link names the recovered project"
    );
}

/// Every link a copy recorded on the local project, tasks and document — `onetaskgraph.copies`
/// and `onetaskgraph.copy_locations` — taken off their files, each line of the folder's
/// metadata block holding one entry.
fn unlinked(plan: &Plan) {
    for kind in ["projects", "tasks", "documents"] {
        let folder = plan.journey.sandbox.subdirectory("notes").join(kind);
        for entry in std::fs::read_dir(&folder).unwrap() {
            let path = entry.unwrap().path();
            // A record's assets sit beside its file, in a folder of their own.
            if path.extension().is_none_or(|extension| extension != "md") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            let kept: Vec<&str> = text
                .lines()
                .filter(|line| {
                    let entry = line.trim_start();
                    !entry.starts_with("\"onetaskgraph.copies\"")
                        && !entry.starts_with("\"onetaskgraph.copy_locations\"")
                })
                .collect();
            let unlinked = format!("{}\n", kept.join("\n"));
            assert!(
                !unlinked.contains("onetaskgraph.copies")
                    && !unlinked.contains("onetaskgraph.copy_locations"),
                "every link is gone from {}: {unlinked}",
                path.display()
            );
            std::fs::write(&path, unlinked).unwrap();
        }
    }
}

#[test]
fn a_plan_whose_local_links_are_removed_is_found_again_by_origin_and_updated_in_place() {
    let plan = Plan::new(LinearMarkdown::Measured);
    let ((project_report, _), (document_report, _)) = plan.copy_plan();
    let landed: BTreeSet<String> = plan
        .items()
        .iter()
        .map(|(kind, source)| {
            Plan::landed(
                if *kind == "document" {
                    &document_report
                } else {
                    &project_report
                },
                source,
            )
        })
        .collect();
    unlinked(&plan);
    let ((project_again, project_sent), (document_again, document_sent)) = plan.copy_plan();
    // The project and every task by the one batched origin read — each counterpart it finds then
    // read by id before it is written — and the document by its own lookup.
    // An origin lookup names an origin in the plan's folder; the walk for orphans under the
    // project, which a copy of a project it finds makes, names none.
    let lookups = |sent: &Sent| -> Vec<String> {
        sent.requests
            .iter()
            .filter(|request| {
                ["originIssues", "issues", "projects", "documents"]
                    .contains(&request.root().as_str())
                    && request.variables.to_string().contains("notes:")
            })
            .map(Request::root)
            .collect()
    };
    assert_eq!(lookups(&project_sent), ["originIssues"]);
    assert_eq!(
        document_sent
            .roots()
            .iter()
            .filter(|root| *root == "documents")
            .count(),
        1,
        "the document is found by its own lookup: {:?}",
        document_sent.roots()
    );
    let mut updated = BTreeSet::new();
    for item in project_again["items"]
        .as_array()
        .unwrap()
        .iter()
        .chain(document_again["items"].as_array().unwrap())
    {
        // Found where it is, never created: a task whose source moved with its link is
        // written, and a project or a document whose record still matches is left as it is.
        assert!(
            ["updated", "unchanged"].contains(&item["action"].as_str().unwrap_or_default())
                && ["origin", "scan"].contains(&item["via"].as_str().unwrap_or_default()),
            "{item}"
        );
        updated.insert(
            item["destination"]
                .as_str()
                .unwrap()
                .split_once(':')
                .unwrap()
                .1
                .to_owned(),
        );
    }
    assert_eq!(
        updated, landed,
        "each existing Linear item is the one written to"
    );
    let created: Vec<String> = project_sent
        .mutations()
        .into_iter()
        .chain(document_sent.mutations())
        .filter(|root| root.ends_with("Create"))
        .collect();
    assert_eq!(
        created,
        Vec::<String>::new(),
        "the second copy creates nothing"
    );
}
