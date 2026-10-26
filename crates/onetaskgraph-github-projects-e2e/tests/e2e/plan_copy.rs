//! The plan copy the copy budgets read: the workload the timing spike measured
//! (`board-copy-speed-2026-10-18-spike-copy-timing-report`), generated from a seed, copied onto
//! a loopback board holding thousands of other items through the real binary, on the simulated
//! clock, with each request held for the spike's measured latency of its kind.
//!
//! What this holds is the shape of what a copy sends — which operations, how many of each —
//! and that the board ends holding exactly the plan. Neither may depend on how many items the
//! board has accumulated, so every scenario is driven on a board of 2,000 items and again on
//! one of 20,000, and the two request tallies are held equal. It asserts no time and no point
//! total against a bound: that is the copy budgets' job. What each scenario on the board of
//! 2,000 cost is recorded here as their telemetry — `github-plan-copy-seconds-*`,
//! `github-plan-copy-points-*` and `github-plan-copy-content-creations` — which
//! `budget_runner::report_plan_copy` hands to onebudgetspec.

use super::asset_board::{Board, Call, Kind, creations};
use onetaskgraph_e2e_support::{
    binary,
    clock::SimulatedClock,
    common::{Sandbox, assert_spawnable_on_windows, stderr, stdout},
    fixtures::{
        GitHubBoardFields, MEASURED_PLACEMENT_MILLIS, MembersPageField, document,
        github_projects_with_items,
    },
    images,
    onevcs::{BoundaryCall, BoundaryCommand, BoundaryRecord, OnevcsHome},
};
use onetaskgraph_github_projects::graphql;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    process::{Output, Stdio},
};

/// The seed every plan below is generated from.
const SEED: u64 = 20_261_018;

/// What a recording of the copy budgets names the plan it measured: the generated plan the
/// budgets' workload states.
pub(super) const WORKLOAD: &str = "generated 10-item plan";

/// The three scenarios the copy budgets measure, by the name a recording gives each.
pub(super) const SCENARIOS: [&str; 3] = ["first", "changed", "unchanged"];

/// The board size the spike's `plans` board stood at, rounded, and ten times it.
pub(super) const SMALL: usize = 2_000;
pub(super) const LARGE: usize = 20_000;

/// The plan's tasks, `T-1` to `T-8`.
pub(super) const TASKS: usize = 8;

/// The plan's ten blocked-by edges, as (task, the task it depends on).
const EDGES: [(usize, usize); 10] = [
    (2, 1),
    (3, 1),
    (3, 2),
    (4, 3),
    (5, 3),
    (6, 4),
    (6, 5),
    (7, 6),
    (8, 7),
    (8, 2),
];

/// The caller metadata keys the plan's records carry, two on the project and on every task and
/// the first alone on the document, each in its record's metadata slot.
///
/// The board projects none of them onto a text field. The workload's 19 field values are what
/// the timing spike's copy wrote, on a board configured with no `metadata_fields`: the `Status`
/// of the project and its eight tasks and the origin field of all ten records. A projection
/// would add a write per value on top of those, which is a cost of configuring one rather than
/// of copying a plan, and `metadata_fields.rs` is where projecting a key onto a field is proven.
const FIELDS: [&str; 2] = ["plan.area", "plan.owner"];

/// The PNG the document references, the spike's 104,072 bytes.
const PNG_BYTES: usize = 104_072;

/// A deterministic stream of numbers: the same seed always writes the same plan.
struct Seeded(u64);

impl Seeded {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }

    /// Seeded prose of at least `bytes` bytes: ASCII sentences in paragraphs, nothing a
    /// Markdown front matter or a reference rewrite could mistake for its own.
    fn prose(&mut self, bytes: usize) -> String {
        const WORDS: [&str; 24] = [
            "board", "copy", "plan", "task", "reads", "writes", "quota", "token", "budget",
            "project", "review", "measure", "latency", "batch", "origin", "member", "link",
            "change", "request", "approve", "document", "edge", "field", "status",
        ];
        let mut text = String::new();
        while text.len() < bytes {
            let sentences = 3 + self.next() % 4;
            for _ in 0..sentences {
                let words = 6 + self.next() % 10;
                for index in 0..words {
                    let word = WORDS[(self.next() % WORDS.len() as u64) as usize];
                    if index == 0 {
                        let mut letters = word.chars();
                        let first = letters.next().unwrap().to_ascii_uppercase();
                        text.push(first);
                        text.push_str(letters.as_str());
                    } else {
                        text.push(' ');
                        text.push_str(word);
                    }
                }
                text.push_str(". ");
            }
            text.push_str("\n\n");
        }
        text
    }
}

/// The plan the copy budgets' workload states: a project of about 50 KB, eight tasks of 45 to
/// 49 KB with ten blocked-by edges among them, and a document of about 35 KB naming every task
/// by the path its folder reports and referencing one 104 KB PNG — the project and every task
/// carrying a value of both [`FIELDS`], the document one of the first.
pub(super) struct Generated {
    pub project: String,
    pub tasks: Vec<String>,
    pub document: String,
    pub png: Vec<u8>,
    /// The values of [`FIELDS`] each record carries, by its id in the folder.
    pub fields: BTreeMap<String, Vec<String>>,
}

impl Generated {
    pub fn new(seed: u64) -> Self {
        let mut stream = Seeded(seed);
        let project = stream.prose(50_100);
        let tasks = (0..TASKS)
            .map(|index| stream.prose(45_200 + index * 500))
            .collect();
        let document = stream.prose(33_000);
        let png = images::png(seed, PNG_BYTES);
        const AREAS: [&str; 4] = ["engine", "board plugin", "live lane", "journeys"];
        let mut value = |prefix: &str| {
            format!(
                "{prefix} {}",
                AREAS[(stream.next() % AREAS.len() as u64) as usize]
            )
        };
        let mut fields = BTreeMap::new();
        fields.insert("P".to_owned(), vec![value("area"), value("owner")]);
        for n in 1..=TASKS {
            fields.insert(format!("T-{n}"), vec![value("area"), value("owner")]);
        }
        fields.insert("D".to_owned(), vec![value("area")]);
        Self {
            project,
            tasks,
            document,
            png,
            fields,
        }
    }
}

/// The synthetic repository the screened plan's board creates its issues in, registered public
/// in the onevcs home the screening runs against — an invented identity, so no answer depends on
/// any host.
pub(super) const SCREENED_REPOSITORY: (&str, &str) = ("synthco", "public-widgets");

/// The folder of Markdown the plan is written to, and the loopback board it is copied onto.
pub(super) struct Plan {
    pub sandbox: Sandbox,
    pub root: PathBuf,
    pub board: Board,
    pub github: GitHubBoardFields,
    pub clock: SimulatedClock,
    pub generated: Generated,
    /// When the store screens its public writes: the onevcs home its `write_policy` runs the
    /// released boundary commands against, and the record of every one it ran.
    pub screening: Option<(OnevcsHome, BoundaryRecord)>,
    /// Which client of the clock this plan's commands attach as — its manager, when several
    /// share one loopback host.
    pub client: usize,
    /// The host configuration axes this plan was laid with (spike-host-copy-parity).
    pub axes: Axes,
}

/// The axes on which this host's plan copy differs from the scenario, each off by default
/// (spike-host-copy-parity): the scenario as the copy budgets measure it.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Axes {
    /// The `notes` source declared `visibility: private`, as the host's `authoring` source is.
    pub private_source: bool,
    /// The plan written as the host writes one: every task and the document rendered through
    /// `task create`/`document create` from a template, carrying the host's own metadata keys,
    /// and the changed task regenerated by `task render` rather than rewritten.
    pub host_shape: bool,
    /// The board answering every request at once, as the host's Python double does, rather
    /// than holding each for the spike's latency of its kind on the simulated clock.
    pub instant_double: bool,
    /// `project copy --status-policy preserve-destination`, as `just copy-plan` copies.
    pub preserve_status: bool,
    /// Every task and the project carrying an `orchestrator.plan-review` record, the revised
    /// task's and the project's written again by `metadata set` after a revision, as
    /// `just review-plan` does.
    pub reviewed: bool,
    /// The board document approved after the first copy: one `document metadata set` of an
    /// `orchestrator.design-approval` record on it, as `just approve-design` writes.
    pub approved: bool,
    /// The board source's `status_mapping` as ai-orchestrator's `plans` source declares it.
    pub status_mapping: bool,
    /// The host plan's one edge — its second task depending on its first — in place of the
    /// workload's ten.
    pub one_edge: bool,
    /// The board's cache emptied before each copy: configured, and holding nothing a copy
    /// wrote before it.
    pub cold_cache: bool,
    /// The board answering a project's members listing as ai-orchestrator's Python double
    /// does: without the `blockedBy` of any member or the project issue's own fields.
    pub host_double_listing: bool,
}

/// One unrelated item of the board's own history: a project every tenth, a design document
/// every fiftieth, a task filed under the latest project otherwise — each a copy of something
/// elsewhere, as every item of a board plans are copied onto is.
pub(super) fn background(index: usize) -> Value {
    let id = format!("BG-{index}");
    let project = index.is_multiple_of(10);
    let parent = (!project).then(|| format!("BG-{}", index - index % 10));
    let origin = format!("elsewhere:{id}");
    let mut slot = json!({"onetaskgraph.origin": origin});
    if project {
        slot["onetaskgraph.item_kind"] = json!("project");
    }
    let title = if index % 50 == 5 {
        format!(
            "{}Earlier design {index}",
            onetaskgraph_github_projects::DESIGN_TITLE_PREFIX
        )
    } else {
        format!("Earlier item {index}")
    };
    json!({"item":format!("ITEM-{id}"),"id":id,"type":"Issue","title":title,
        "body":format!("An item of an earlier plan.\n\n<!-- onetaskgraph.metadata\n{slot}\n-->"),
        "state":"OPEN","reason":null,"parent":parent,
        "repo":"nickderobertis/onetaskgraph","status":"Todo","origin":origin,"labels":[]})
}

impl Plan {
    /// The generated plan in a folder, beside a board holding `items` unrelated items and
    /// `extra` more, every request to it answered after the spike's latency for its kind.
    pub fn new(items: usize, extra: Vec<Value>) -> Self {
        Self::configured(items, extra, None)
    }

    /// [`Plan::new`], with the board source's `cache` block set to `cache` when one is given.
    pub fn configured(items: usize, extra: Vec<Value>, cache: Option<Value>) -> Self {
        Self::laid(items, extra, cache, false)
    }

    /// [`Plan::new`], with the store screening every public write through the released onevcs
    /// boundary commands — its `write_policy` — against an onevcs home holding one synthetic
    /// identity, the public [`SCREENED_REPOSITORY`] the board creates its issues in.
    pub fn screened(items: usize, extra: Vec<Value>, cache: Option<Value>) -> Self {
        Self::laid(items, extra, cache, true)
    }

    fn laid(items: usize, extra: Vec<Value>, cache: Option<Value>, screened: bool) -> Self {
        Self::hosted(
            items,
            extra,
            cache,
            screened,
            &SimulatedClock::start(1),
            0,
            None,
            Axes::default(),
        )
    }

    /// The plan laid on one of the host's axes, or several (spike-host-copy-parity).
    pub fn variant(items: usize, cache: Option<Value>, screened: bool, axes: Axes) -> Self {
        Self::hosted(
            items,
            Vec::new(),
            cache,
            screened,
            &SimulatedClock::start(1),
            0,
            None,
            axes,
        )
    }

    /// `managers` plans copied at once, each its manager's — a client of one simulated clock —
    /// on a board of its own behind one loopback host, whose record of every request and whose
    /// GitHub secondary-limit windows they share, with write screening on when `screened` is.
    pub fn managers(managers: usize, items: usize, screened: bool) -> Vec<Self> {
        let clock = SimulatedClock::start(managers);
        let first = Self::hosted(
            items,
            Vec::new(),
            None,
            screened,
            &clock,
            0,
            None,
            Axes::default(),
        );
        let mut plans = Vec::with_capacity(managers);
        for client in 1..managers {
            plans.push(Self::hosted(
                items,
                Vec::new(),
                None,
                screened,
                &clock,
                client,
                Some(&first.board),
                Axes::default(),
            ));
        }
        plans.insert(0, first);
        plans
    }

    // Each argument is one independent axis of the plan a journey lays — its board, its cache,
    // its screening, its clock, its manager and the host it shares — and every caller names all
    // of them, so a builder would only restate this signature.
    #[allow(clippy::too_many_arguments)]
    fn hosted(
        items: usize,
        extra: Vec<Value>,
        cache: Option<Value>,
        screened: bool,
        clock: &SimulatedClock,
        client: usize,
        host: Option<&Board>,
        axes: Axes,
    ) -> Self {
        let clock = clock.clone();
        let sandbox = Sandbox::new();
        let mut held: Vec<Value> = (0..items).map(background).collect();
        held.extend(extra);
        let (mut config, github) = github_projects_with_items(&sandbox, held);
        // GitHub's default workflows set no `Status` on an item added to a board, so the copy
        // writes every one it carries — the workload's nine.
        github.file_new_items_without_status(true);
        github.answer_members_as_host_double(axes.host_double_listing);
        let board = Board::new(config["endpoint"].as_str().unwrap(), &clock, client, host);
        if axes.instant_double {
            board.hold_answers_for(std::time::Duration::ZERO);
        } else {
            board.measure_latency();
        }
        config["endpoint"] = json!(board.endpoint);
        config["pacing"] = json!({});
        if axes.status_mapping {
            config["status_mapping"] = json!({"unknown": "Needs attention", "queued": "Queued",
                "done": "Done", "cancelled": "Cancelled"});
        }
        let root = sandbox.subdirectory("notes");
        for directory in ["projects", "tasks", "documents/D.assets"] {
            std::fs::create_dir_all(root.join(directory)).unwrap();
        }
        let root = std::fs::canonicalize(root).unwrap();
        let mut board_source = json!({"plugin":"github-projects","config": config});
        if let Some(mut cache) = cache {
            // Inside the sandbox, so what the cache keeps is this journey's and nobody else's.
            cache["dir"] = json!(sandbox.subdirectory("board-cache"));
            board_source["cache"] = cache;
        }
        let screening = screened.then(|| {
            let (owner, name) = SCREENED_REPOSITORY;
            let mut home = OnevcsHome::new(&sandbox);
            home.public(owner, name);
            let record = home.recorded(&sandbox);
            board_source["config"]["repository"] = json!(format!("{owner}/{name}"));
            github.use_repository(&format!("{owner}/{name}"));
            (home, record)
        });
        let mut notes = json!({"plugin":"local-md","config":{"root":root}});
        if axes.private_source {
            notes["visibility"] = json!("private");
        }
        let mut configured: Value = serde_json::from_str(&document(&json!({
            "notes": notes,
            "board": board_source
        })))
        .unwrap();
        if let Some((_, record)) = &screening {
            configured["write_policy"] = record.write_policy().clone();
        }
        sandbox.project_document(&configured.to_string());
        let plan = Self {
            sandbox,
            root,
            board,
            github,
            clock,
            generated: Generated::new(SEED),
            screening,
            client,
            axes,
        };
        plan.author();
        plan
    }

    /// Place every issue a create files on the board as GitHub places one — with no board item
    /// in the create's answer, and on the board only the spike's measured delay after it, on
    /// the simulated clock — rather than at once.
    pub fn place_as_measured(&self) {
        let timed = self.clock.clone();
        self.github.place_filed_issues_after(
            MEASURED_PLACEMENT_MILLIS
                .iter()
                .map(|millis| std::time::Duration::from_millis(*millis))
                .collect(),
            move || timed.now(),
        );
        let placing = self.github.clone();
        self.board.on_answered(move |at| placing.answered_at(at));
    }

    /// The plan's edges: the workload's ten, or the host plan's one.
    fn edges(&self) -> &'static [(usize, usize)] {
        if self.axes.one_edge { &EDGES[..1] } else { &EDGES }
    }

    /// The path the folder reports for task `n`, which the document names it by.
    pub fn task_path(&self, n: usize) -> String {
        self.root
            .join(format!("tasks/T-{n}.md"))
            .to_str()
            .unwrap()
            .to_owned()
    }

    /// The front matter giving record `id` its generated metadata values.
    fn field_values(&self, id: &str) -> String {
        let mut metadata = String::from("metadata:\n");
        for (key, value) in FIELDS.iter().zip(&self.generated.fields[id]) {
            metadata.push_str(&format!("  {key}: {value}\n"));
        }
        metadata
    }

    /// Write the project, its tasks and its document, each with the content and the board field
    /// values generated for it.
    fn author(&self) {
        let generated = &self.generated;
        std::fs::write(
            self.root.join("projects/P.md"),
            format!(
                "---\ntitle: Copy the plan in a fraction of the time\nstatus: todo\n{}---\n{}",
                self.field_values("P"),
                generated.project
            ),
        )
        .unwrap();
        for n in 1..=TASKS {
            if self.axes.host_shape {
                self.render_task(n, &generated.tasks[n - 1]);
            } else {
                self.write_task(n, &generated.tasks[n - 1], true);
            }
        }
        let mut table = String::from("| Task | Where |\n| --- | --- |\n");
        for n in 1..=TASKS {
            table.push_str(&format!("| T-{n} | `{}` |\n", self.task_path(n)));
        }
        self.review(0, None);
        std::fs::write(
            self.root.join("documents/D.assets/diagram.png"),
            &generated.png,
        )
        .unwrap();
        std::fs::write(
            self.root.join("documents/D.md"),
            format!(
                "---\ntitle: Design of the plan copy\nproject: P\n{}---\n{}\n## Planned tasks\n\n{table}\n![Where the copy spends its time](./diagram.png)\n",
                self.field_values("D"),
                generated.document
            ),
        )
        .unwrap();
    }

    /// Write task `n` with `content`, filed under the project or not, keeping whatever metadata
    /// an earlier copy recorded in its front matter — its generated board field values when
    /// nothing was recorded yet.
    pub fn write_task(&self, n: usize, content: &str, filed: bool) {
        let path = self.root.join(format!("tasks/T-{n}.md"));
        let kept = std::fs::read_to_string(&path)
            .ok()
            .and_then(|held| {
                let front = held
                    .strip_prefix("---\n")?
                    .split_once("\n---\n")?
                    .0
                    .to_owned();
                let at = front.find("metadata:")?;
                Some(format!("{}\n", &front[at..]))
            })
            .unwrap_or_else(|| self.field_values(&format!("T-{n}")));
        let depends: Vec<String> = self
            .edges()
            .iter()
            .filter(|(task, _)| *task == n)
            .map(|(_, on)| format!("T-{on}"))
            .collect();
        let depends = if depends.is_empty() {
            String::new()
        } else {
            format!("depends_on: [{}]\n", depends.join(", "))
        };
        std::fs::write(
            path,
            format!(
                "---\ntitle: Step {n} of the plan\nstatus: todo\n{}{depends}{kept}---\n{content}",
                if filed { "project: P\n" } else { "" }
            ),
        )
        .unwrap();
    }

    /// The template a host-shaped plan renders its tasks from: `what` and `why`, as the host's
    /// `plan-task` renders a task's sections from its answers (spike-host-copy-parity).
    fn task_template(&self) -> PathBuf {
        let path = self.sandbox.subdirectory("templates").join("plan-task.md.j2");
        if !path.exists() {
            std::fs::write(
                &path,
                "---\nonetaskgraph_template: 1\nvariables:\n  what:\n    description: What it delivers\n    type: text\n  why:\n    description: Why\n    type: text\n---\n## What\n\n{{ what }}\n\n## Why\n\n{{ why }}\n",
            )
            .unwrap();
        }
        path
    }

    /// One local command through the binary, attached to nothing: what reads and writes the
    /// notes folder alone.
    fn run_local(&self, arguments: &[&str]) -> Output {
        let mut command = self.sandbox.subprocess(binary());
        if let Some((home, _)) = &self.screening {
            command.env("ONEVCS_HOME", home.path());
        }
        command
            .args(arguments)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command.spawn().unwrap().wait_with_output().unwrap()
    }

    /// Task `n` created as the host creates a planned task: `task create` from a template with
    /// the host's own metadata keys beside the plan's, then filed as `T-n` with its edges.
    fn render_task(&self, n: usize, what: &str) {
        let template = self.task_template();
        let answers = self.sandbox.subdirectory("answers").join(format!("T-{n}.json"));
        std::fs::write(
            &answers,
            json!({"what": what, "why": "An operator cannot see past the first screen."})
                .to_string(),
        )
        .unwrap();
        let mut arguments: Vec<String> = [
            "task",
            "create",
            "notes",
            "--project",
            "P",
            "--title",
            &format!("Step {n} of the plan"),
            "--template",
            template.to_str().unwrap(),
            "--answers",
            answers.to_str().unwrap(),
            "--no-interactive",
            &format!("--metadata=onepipeline.id=\"part-{n}\""),
            "--metadata=onepipeline.persona=\"engineer\"",
            "--metadata=orchestrator.unit=\"Listing\"",
        ]
        .iter()
        .map(|argument| (*argument).to_owned())
        .collect();
        for (key, value) in FIELDS.iter().zip(&self.generated.fields[&format!("T-{n}")]) {
            arguments.push(format!("--metadata={key}={}", json!(value)));
        }
        let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
        let output = self.run_local(&arguments);
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        let created = stdout(&output).trim().to_owned();
        let native = created.strip_prefix("notes:").unwrap_or(&created);
        let from = self.root.join(format!("tasks/{native}.md"));
        let held = std::fs::read_to_string(&from).unwrap();
        std::fs::remove_file(&from).unwrap();
        let depends: Vec<String> = self
            .edges()
            .iter()
            .filter(|(task, _)| *task == n)
            .map(|(_, on)| format!("T-{on}"))
            .collect();
        let held = if depends.is_empty() {
            held
        } else {
            held.replacen(
                "\nproject: P\n",
                &format!("\nproject: P\ndepends_on: [{}]\n", depends.join(", ")),
                1,
            )
        };
        assert!(held.contains("project: P"), "{held}");
        std::fs::write(self.root.join(format!("tasks/T-{n}.md")), held).unwrap();
    }

    /// Revise task `n` as a manager does between two copies: rewritten with an edit appended
    /// in the scenario's shape, regenerated by `task render` from changed answers in the
    /// host's. Answers the content it now holds.
    pub fn revise(&self, n: usize) -> String {
        let edited = format!(
            "{}An edit made since the last copy.\n",
            self.generated.tasks[n - 1]
        );
        if !self.axes.host_shape {
            self.write_task(n, &edited, true);
            self.review(1, Some(n));
            return edited;
        }
        let answers = self.sandbox.subdirectory("answers").join(format!("T-{n}-revised.json"));
        std::fs::write(&answers, json!({"what": edited}).to_string()).unwrap();
        let id = format!("notes:T-{n}");
        let template = self.task_template();
        let output = self.run_local(&[
            "task",
            "render",
            &id,
            "--template",
            template.to_str().unwrap(),
            "--answers",
            answers.to_str().unwrap(),
            "--no-interactive",
        ]);
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        self.review(1, Some(n));
        edited
    }

    /// Record a review as `just review-plan` does, the `round`th time, when the plan is reviewed
    /// at all: of every task and the project at first, then of the revised task `revised` and
    /// the project — what the host's re-copy after a revision updated on the board.
    pub fn review(&self, round: usize, revised: Option<usize>) {
        if !self.axes.reviewed {
            return;
        }
        let value = |id: &str| {
            json!({"by": "planning-closeout", "key": format!("{id}-{round}"),
                   "reviewed_at": format!("2026-10-26T13:0{round}:00+00:00")})
            .to_string()
        };
        let mut ids: Vec<(String, &str)> = (1..=TASKS)
            .filter(|n| revised.is_none_or(|revised| revised == *n))
            .map(|n| (format!("notes:T-{n}"), "task"))
            .collect();
        ids.push(("notes:P".to_owned(), "project"));
        for (id, kind) in ids {
            let output = self.run_local(&[
                kind,
                "metadata",
                "set",
                &id,
                "orchestrator.plan-review",
                &value(&id),
            ]);
            assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        }
    }

    /// Approve the board document `native` as `just approve-design` does, outside any
    /// scenario's measurement.
    pub fn approve(&self, native: &str) {
        if !self.axes.approved {
            return;
        }
        let lease = self.clock.lease(self.client);
        let output = self.run(&[
            "document",
            "metadata",
            "set",
            &format!("board:{native}"),
            "orchestrator.design-approval",
            &json!({"key": "approved", "by": "a synthetic approver"}).to_string(),
        ]);
        lease.wait_for_detach();
        drop(lease);
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    }

    /// One command through the binary, attached to the clock.
    pub fn run(&self, arguments: &[&str]) -> Output {
        let mut command = self.sandbox.subprocess(binary());
        if let Some((home, _)) = &self.screening {
            command.env("ONEVCS_HOME", home.path());
        }
        command
            .args(arguments)
            .envs(self.clock.client_env(self.client))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        assert_spawnable_on_windows(&command);
        command.spawn().unwrap().wait_with_output().unwrap()
    }

    /// One scenario as `just copy-plan` sends it — `project copy` then `document copy` — with
    /// both reports and every request the board answered for the two.
    pub fn copy(&self) -> (Value, Value, Vec<Call>) {
        let scenario = self.copy_timed();
        (scenario.project, scenario.document, scenario.calls)
    }

    /// [`Self::copy`], with how long each of its two commands took: in the board's simulated
    /// time, and in the real time its process ran.
    pub fn copy_timed(&self) -> Scenario {
        if self.axes.cold_cache {
            let cache = self.sandbox.subdirectory("board-cache");
            std::fs::remove_dir_all(&cache).unwrap();
            std::fs::create_dir_all(&cache).unwrap();
        }
        let before = self.board.calls().len();
        let lease = self.clock.lease(self.client);
        let mut phases = Vec::new();
        let mut reports = Vec::new();
        let mut boundary = Vec::new();
        for (kind, id) in [("project", "notes:P"), ("document", "notes:D")] {
            let simulated = self.clock.now();
            let started = std::time::Instant::now();
            let screened_before = self.boundary_calls().len();
            let sent_before = self.board.calls().len();
            let mut arguments = vec![kind, "copy", id, "--to", "board", "--json"];
            if kind == "project" && self.axes.preserve_status {
                arguments.extend(["--status-policy", "preserve-destination"]);
            }
            let output = self.run(&arguments);
            let real = started.elapsed();
            lease.wait_for_detach();
            phases.push((kind, self.clock.now() - simulated, real));
            reports.push(success(&output));
            boundary.push(Phase {
                kind,
                screening: self.boundary_calls()[screened_before..].to_vec(),
                calls: self.own_calls(sent_before),
            });
        }
        drop(lease);
        let document = reports.pop().expect("the document copy's report");
        let project = reports.pop().expect("the project copy's report");
        Scenario {
            project,
            document,
            calls: self.own_calls(before),
            phases,
            boundary,
        }
    }

    /// Every request the host answered this plan's commands from the `from`th it recorded on:
    /// another manager's sharing the host are not this plan's.
    fn own_calls(&self, from: usize) -> Vec<Call> {
        self.board.calls()[from..]
            .iter()
            .filter(|call| call.client == self.client)
            .cloned()
            .collect()
    }

    /// Every boundary command the store has run so far, when it screens its writes.
    pub fn boundary_calls(&self) -> Vec<BoundaryCall> {
        self.screening
            .as_ref()
            .map(|(_, record)| record.calls())
            .unwrap_or_default()
    }
}

/// One command of a scenario: the boundary commands it ran and the requests the board answered.
pub(super) struct Phase {
    pub kind: &'static str,
    pub screening: Vec<BoundaryCall>,
    pub calls: Vec<Call>,
}

/// One scenario of the copy budgets, as [`Plan::copy_timed`] measured it.
pub(super) struct Scenario {
    pub project: Value,
    pub document: Value,
    /// Every request the board answered for it.
    pub calls: Vec<Call>,
    /// Each command, its simulated time and its process's real time.
    pub phases: Vec<(&'static str, std::time::Duration, std::time::Duration)>,
    /// Each command's boundary commands beside its requests.
    pub boundary: Vec<Phase>,
}

/// The points one request of each kind costs against the primary GraphQL allowance, as the
/// timing spike's `pricing.json` priced it, each beside how the points budgets' descriptions
/// in `budgets.yaml` name it — the one spelling of that price list here, which
/// [`the_points_budgets_state_the_prices_they_are_measured_at`] holds those descriptions to.
const SPIKE_PRICES: [(&str, u64, &[Kind]); 5] = [
    ("a board page", 2, &[Kind::BoardPage]),
    ("an issue-search page", 5, &[Kind::SearchPage]),
    ("any other read", 1, &[Kind::ItemRead, Kind::BatchedRead]),
    (
        "each mutation request",
        1,
        &[Kind::Mutation, Kind::AliasedMutations],
    ),
    (
        "REST",
        0,
        &[Kind::Upload, Kind::Repository, Kind::Verification],
    ),
];

/// What one request costs against the primary GraphQL allowance, by [`SPIKE_PRICES`].
fn spike_points(call: &Call) -> u64 {
    SPIKE_PRICES
        .iter()
        .find(|(_, _, kinds)| kinds.contains(&call.kind))
        .map(|(_, points, _)| *points)
        .expect("every kind of request is priced")
}

#[test]
fn the_points_budgets_state_the_prices_they_are_measured_at() {
    // Each points budget's description says what a request costs; the figure it holds is
    // measured by `SPIKE_PRICES`, so the two are one statement, read here from the file.
    let stated = SPIKE_PRICES
        .iter()
        .map(|(named, points, _)| format!("{named} {points}"))
        .collect::<Vec<_>>()
        .join(", ");
    let budgets = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("budgets.yaml"),
    )
    .unwrap();
    let mut described = 0;
    // Each entry of the file's `budgets` list, from its `- id:` line to the next one's.
    for entry in budgets.split("\n  - id: ").skip(1) {
        if entry.starts_with("github-plan-copy-points-") {
            described += 1;
            assert!(
                entry.contains(&format!("pricing.json — {stated}.")),
                "a points budget does not state the prices it is measured at ({stated}): {entry}"
            );
        }
    }
    assert_eq!(described, SCENARIOS.len(), "a points budget per scenario");
}

/// What one scenario cost, in each of the figures the copy budgets read.
struct Cost {
    simulated: f64,
    real: f64,
    points: u64,
    writes: u64,
    /// The seconds' detail: each command's simulated seconds, its requests and its polls.
    seconds: Value,
    /// The real seconds' detail.
    real_detail: Value,
    /// The points' detail.
    points_detail: Value,
    /// The writes' detail.
    writes_detail: Value,
}

/// The writes of a request of a copy's aliased field values, by the field each set.
fn field_values(call: &Call, by_field: &mut BTreeMap<String, u64>) {
    let Ok(request) = serde_json::from_slice::<Value>(&call.bytes) else {
        return;
    };
    let send = match request["query"].as_str() {
        Some(query) if query == graphql::WRITE_FIELD_VALUES => "send",
        Some(query) if query == graphql::WRITE_FIELDS_AND_EDGES => "sendField",
        _ => return,
    };
    let variables = &request["variables"];
    for slot in 0..onetaskgraph_github_projects::WRITE_BATCH {
        if variables[format!("{send}{slot}")] == true {
            let field = variables[format!("field{slot}")]["fieldId"]
                .as_str()
                .unwrap_or("a field this write did not name")
                .to_owned();
            *by_field.entry(field).or_default() += 1;
        }
    }
}

/// What each command of `scenario` asked the boundary, in the order it asked: each repository
/// it inspected, and each check with how many values of each surface it carried.
fn screening_calls(scenario: &Scenario) -> Value {
    scenario
        .boundary
        .iter()
        .map(|phase| {
            let calls: Vec<Value> = phase
                .screening
                .iter()
                .map(|call| match call.command {
                    BoundaryCommand::Inspect => json!({"inspect": call.input["repository"]}),
                    _ => json!({"check": {
                        "text": call.input["text"].as_array().map_or(0, Vec::len),
                        "paths": call.input["paths"].as_array().map_or(0, Vec::len),
                        "metadata": call.input["metadata"].as_array().map_or(0, Vec::len)},
                        "exit": call.exit}),
                })
                .collect();
            (phase.kind.to_owned(), Value::Array(calls))
        })
        .collect::<serde_json::Map<String, Value>>()
        .into()
}

/// Every figure the copy budgets read of `scenario`, each with the detail it is reported with,
/// priced by [`SPIKE_PRICES`] and counted as [`creations`] counts a write.
fn cost(scenario: &Scenario) -> Cost {
    let simulated: f64 = scenario
        .phases
        .iter()
        .map(|(_, simulated, _)| simulated.as_secs_f64())
        .sum();
    let real: f64 = scenario
        .phases
        .iter()
        .map(|(_, _, real)| real.as_secs_f64())
        .sum();
    let requests = tally(&scenario.calls);
    let mut points_by: BTreeMap<String, u64> = BTreeMap::new();
    let mut creations_by: BTreeMap<String, u64> = BTreeMap::new();
    let mut values_by_field: BTreeMap<String, u64> = BTreeMap::new();
    for call in &scenario.calls {
        *points_by.entry(operation(call)).or_default() += spike_points(call);
        let created = creations(&call.path, &call.bytes);
        if created > 0 {
            *creations_by.entry(operation(call)).or_default() += created;
        }
        field_values(call, &mut values_by_field);
    }
    let latencies: BTreeMap<String, u64> = [
        Kind::ItemRead,
        Kind::BoardPage,
        Kind::SearchPage,
        Kind::BatchedRead,
        Kind::Mutation,
        Kind::AliasedMutations,
        Kind::Upload,
        Kind::Repository,
        Kind::Verification,
    ]
    .iter()
    .map(|kind| (format!("{kind:?}"), kind.measured_millis()))
    .collect();
    let phases: BTreeMap<&str, f64> = scenario
        .phases
        .iter()
        .map(|(kind, simulated, _)| (*kind, simulated.as_secs_f64()))
        .collect();
    // What each command spent reading where GitHub placed what it created, and how many
    // aliases each of its mutation requests carried.
    let shapes: BTreeMap<&str, Value> = scenario
        .boundary
        .iter()
        .map(|phase| {
            let polls = phase
                .calls
                .iter()
                .filter(|call| {
                    operation(call) == "reading the board items of the issues a copy created"
                })
                .count();
            let aliases: Vec<u64> = phase
                .calls
                .iter()
                .filter(|call| call.creating && call.kind != Kind::Upload)
                .map(|call| creations(&call.path, &call.bytes))
                .collect();
            let lookups = phase
                .calls
                .iter()
                .filter(|call| operation(call).starts_with("looking up the items copied"))
                .count();
            (
                phase.kind,
                json!({"placement polls": polls, "aliases per mutation request": aliases,
                       "origin lookup requests": lookups}),
            )
        })
        .collect();
    let points: u64 = points_by.values().sum();
    let writes: u64 = creations_by.values().sum();
    Cost {
        simulated,
        real,
        points,
        writes,
        seconds: json!({"simulated seconds by phase": phases, "requests by operation": requests,
                        "milliseconds charged by kind": latencies, "shape by phase": shapes,
                        "screening": screening_calls(scenario)}),
        real_detail: json!({"real seconds by phase": scenario
            .phases
            .iter()
            .map(|(kind, _, real)| (*kind, real.as_secs_f64()))
            .collect::<BTreeMap<&str, f64>>()}),
        points_detail: json!({"points": points, "points by operation": points_by,
                              "requests by operation": requests, "shape by phase": shapes,
                              "screening": screening_calls(scenario)}),
        writes_detail: json!({"writes": writes, "writes by mutation": creations_by,
                              "field values by field": values_by_field,
                              "requests": scenario.calls.iter().filter(|call| creations(&call.path, &call.bytes) > 0).count(),
                              "screening": screening_calls(scenario)}),
    }
}

/// Every run of one scenario the copy budgets read: on the board of [`SMALL`] and of [`LARGE`],
/// each with no write policy and with write screening on.
pub(super) struct Runs<'a> {
    pub small: &'a Scenario,
    pub small_screened: &'a Scenario,
    pub large: &'a Scenario,
    pub large_screened: &'a Scenario,
    /// Each of [`MANAGERS`] managers' run of it, copying at once against one loopback host,
    /// with no write policy and with write screening on.
    pub managers: Vec<&'a Scenario>,
    pub managers_screened: Vec<&'a Scenario>,
}

/// Record what scenario `name` cost as the copy budgets' telemetry, for
/// `budget_runner::report_plan_copy`, on the board their workload states: its seconds — the
/// simulated seconds of both commands — and, apart, the real seconds their processes ran,
/// each from the run with no write policy, as the seconds budgets state them; and its points
/// and, for a first copy, the writes GitHub's secondary limit may count, each the larger of
/// the run with no write policy and the run with write screening on. Every detail carries
/// both runs, their screening calls, and the same figures on the board of [`LARGE`].
fn record(name: &str, runs: &Runs<'_>) {
    let costs = [
        ("unscreened", cost(runs.small)),
        ("screened", cost(runs.small_screened)),
        ("unscreened, on a board of 20,000 items", cost(runs.large)),
        (
            "screened, on a board of 20,000 items",
            cost(runs.large_screened),
        ),
    ];
    let at_once: Vec<(String, Cost)> = runs
        .managers
        .iter()
        .enumerate()
        .map(|(manager, run)| {
            (
                format!("manager {manager} of {MANAGERS} at once"),
                cost(run),
            )
        })
        .chain(
            runs.managers_screened
                .iter()
                .enumerate()
                .map(|(manager, run)| {
                    (
                        format!("manager {manager} of {MANAGERS} at once, screened"),
                        cost(run),
                    )
                }),
        )
        .collect();
    let of = |figure: fn(&Cost) -> &Value| -> Value {
        costs
            .iter()
            .map(|(label, cost)| ((*label).to_owned(), figure(cost).clone()))
            .chain(
                at_once
                    .iter()
                    .map(|(label, cost)| (label.clone(), figure(cost).clone())),
            )
            .collect::<serde_json::Map<String, Value>>()
            .into()
    };
    let small = &costs[..2];
    let larger =
        |figure: fn(&Cost) -> u64| small.iter().map(|(_, cost)| figure(cost)).max().unwrap();
    let directory = super::budget_runner::directory();
    std::fs::create_dir_all(&directory).unwrap();
    let mut recordings = vec![
        (
            format!("github-plan-copy-seconds-{name}"),
            costs[0].1.simulated,
            of(|cost| &cost.seconds),
        ),
        // The processes' own real time, which the host's load moves: reported beside the
        // simulated figure, never added to it.
        (
            format!("github-plan-copy-real-seconds-{name}"),
            costs[0].1.real,
            of(|cost| &cost.real_detail),
        ),
        (
            format!("github-plan-copy-points-{name}"),
            larger(|cost| cost.points) as f64,
            of(|cost| &cost.points_detail),
        ),
    ];
    if name == SCENARIOS[0] {
        recordings.push((
            "github-plan-copy-content-creations".to_owned(),
            larger(|cost| cost.writes) as f64,
            of(|cost| &cost.writes_detail),
        ));
    }
    for (budget, value, detail) in recordings {
        std::fs::write(
            onetaskgraph_e2e_support::telemetry::file_in(&directory, &budget),
            serde_json::to_vec(&json!({"budget": budget, "value": value,
                "detail": detail.to_string(), "plan": WORKLOAD, "board": SMALL,
                "scenario": name}))
            .unwrap(),
        )
        .unwrap();
    }
}

/// What a command printed, once it exited 0.
pub(super) fn success(output: &Output) -> Value {
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}\n{}",
        stdout(output),
        stderr(output)
    );
    serde_json::from_str(&stdout(output)).unwrap()
}

/// What one request was, by the plugin's own name for the document it sent, or by its REST
/// path for the three that are not GraphQL.
pub(super) fn operation(call: &Call) -> String {
    match call.kind {
        Kind::Upload => return "POST /user-attachments/assets".into(),
        Kind::Repository => return "GET /repos/{owner}/{repo}".into(),
        Kind::Verification => return "GET /user-attachments/assets/{id}".into(),
        _ => {}
    }
    let request: Value = serde_json::from_slice(&call.bytes).unwrap();
    let query = request["query"].as_str().unwrap();
    graphql::DOCUMENTS
        .iter()
        .find(|(document, _)| *document == query)
        .map_or_else(
            || panic!("a request no document of the plugin names: {query}"),
            |(_, name)| (*name).to_owned(),
        )
}

pub(super) fn tally(calls: &[Call]) -> BTreeMap<String, usize> {
    let mut counted = BTreeMap::new();
    for call in calls {
        *counted.entry(operation(call)).or_default() += 1;
    }
    counted
}

/// Hold `calls` to never reading a page of the board's items or of its issue search.
pub(super) fn never_walks(what: &str, calls: &[Call]) {
    let walked: Vec<String> = calls
        .iter()
        .filter(|call| matches!(call.kind, Kind::BoardPage | Kind::SearchPage))
        .map(operation)
        .collect();
    assert!(
        walked.is_empty(),
        "{what} walked the board: {walked:?} of {:?}",
        tally(calls)
    );
}

/// The destination id a copy report gave `source`.
pub(super) fn landed(report: &Value, source: &str) -> String {
    report["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["source"] == source)
        .unwrap_or_else(|| panic!("{source} is not in {report:#}"))["destination"]
        .as_str()
        .unwrap()
        .strip_prefix("board:")
        .unwrap()
        .to_owned()
}

/// A board body without the metadata slot this plugin appends to it, and the slot itself.
fn split_slot(body: &str) -> (&str, Value) {
    let (content, slot) = body
        .rsplit_once("\n\n<!-- onetaskgraph.metadata\n")
        .unwrap_or_else(|| panic!("no metadata slot in {body}"));
    let slot = slot.strip_suffix("\n-->").unwrap();
    (content, serde_json::from_str(slot).unwrap())
}

/// Every id the board holds a counterpart of the plan under, by the plan's own ids.
pub(super) type Counterparts = BTreeMap<String, String>;

/// Hold the board item `native` to carrying the metadata values generated for `source` in its
/// metadata slot, and no key the source gives it no value of.
fn carries_fields(plan: &Plan, native: &str, source: &str) {
    let values = &plan.generated.fields[source];
    let (_, slot) = split_slot(plan.github.body(native).as_str().unwrap());
    for (index, key) in FIELDS.iter().enumerate() {
        assert_eq!(
            slot.get(*key).and_then(Value::as_str).map(str::to_owned),
            values.get(index).cloned(),
            "{source}'s {key} in its metadata slot"
        );
    }
}

/// Hold the board to holding exactly the generated plan, `changed` naming a task whose
/// content was replaced since: the project, eight tasks and the document each with the
/// source's title, content, state, status and generated board field values, every task a sub-issue of the project,
/// the ten blocked-by edges and no others, and the document referencing each task's
/// counterpart and its PNG as an uploaded asset whose bytes are the source's.
pub(super) fn holds_the_plan(plan: &Plan, at: &Counterparts, changed: Option<(usize, &str)>) {
    let github = &plan.github;
    let project = &at["P"];
    let item = |id: &str| github.item(id).unwrap_or_else(|| panic!("no item {id}"));
    let held = item(project);
    assert_eq!(held["title"], "Copy the plan in a fraction of the time");
    assert_eq!(held["state"], "OPEN");
    assert_eq!(held["status"], "Todo");
    assert_eq!(held["origin"], "notes:P");
    assert_eq!(held["parent"], Value::Null);
    let (content, slot) = split_slot(held["body"].as_str().unwrap());
    assert_eq!(content.trim_end(), plan.generated.project.trim_end());
    assert_eq!(slot["onetaskgraph.item_kind"], "project");
    carries_fields(plan, project, "P");
    for n in 1..=TASKS {
        let native = &at[&format!("T-{n}")];
        let held = item(native);
        let expected = match changed {
            Some((task, content)) if task == n => content,
            _ => plan.generated.tasks[n - 1].as_str(),
        };
        assert_eq!(held["title"], format!("Step {n} of the plan"));
        assert_eq!(held["state"], "OPEN");
        assert_eq!(held["status"], "Todo");
        assert_eq!(held["origin"], format!("notes:T-{n}"));
        assert_eq!(
            held["parent"],
            json!(project),
            "T-{n} is a sub-issue of the project"
        );
        let (content, _) = split_slot(held["body"].as_str().unwrap());
        assert_eq!(content.trim_end(), expected.trim_end(), "T-{n}'s body");
        carries_fields(plan, native, &format!("T-{n}"));
        let mut blockers = github.blockers(native);
        blockers.sort();
        let mut expected: Vec<String> = EDGES
            .iter()
            .filter(|(task, _)| *task == n)
            .map(|(_, on)| at[&format!("T-{on}")].clone())
            .collect();
        expected.sort();
        assert_eq!(blockers, expected, "T-{n}'s blocked-by edges");
    }
    let edges: usize = (1..=TASKS)
        .map(|n| github.blockers(&at[&format!("T-{n}")]).len())
        .sum::<usize>()
        + github.blockers(project).len()
        + github.blockers(&at["D"]).len();
    assert_eq!(edges, EDGES.len(), "the ten edges and no others");

    let held = item(&at["D"]);
    assert_eq!(
        held["title"],
        format!(
            "{}Design of the plan copy",
            onetaskgraph_github_projects::DESIGN_TITLE_PREFIX
        )
    );
    assert_eq!(held["state"], "OPEN");
    assert_eq!(held["origin"], "notes:D");
    assert_eq!(held["parent"], json!(project));
    carries_fields(plan, &at["D"], "D");
    let (content, slot) = split_slot(held["body"].as_str().unwrap());
    let asset = &slot["onetaskgraph.assets"]["diagram.png"];
    let url = asset["url"].as_str().unwrap();
    let path = url
        .strip_prefix(plan.board.endpoint.trim_end_matches("/graphql"))
        .unwrap();
    assert_eq!(
        plan.board.attachment(path).as_deref(),
        Some(plan.generated.png.as_slice()),
        "the uploaded asset's bytes are the source's"
    );
    let mut expected = std::fs::read_to_string(plan.root.join("documents/D.md")).unwrap();
    expected = expected.split_once("\n---\n").unwrap().1.to_owned();
    for n in 1..=TASKS {
        expected = expected.replace(
            &plan.task_path(n),
            &format!("https://example.invalid/{}", at[&format!("T-{n}")]),
        );
    }
    expected = expected.replace("](./diagram.png)", &format!("]({url})"));
    assert_eq!(
        content,
        expected.trim_end(),
        "the document's rewritten body"
    );
}

/// The board ids of the plan's counterparts, from a scenario's two reports.
pub(super) fn counterparts(project: &Value, document: &Value) -> Counterparts {
    let mut at = Counterparts::new();
    at.insert("P".into(), landed(project, "notes:P"));
    for n in 1..=TASKS {
        at.insert(format!("T-{n}"), landed(project, &format!("notes:T-{n}")));
    }
    at.insert("D".into(), landed(document, "notes:D"));
    at
}

/// Hold every command of a screened `scenario` to what its boundary record shows: each
/// repository it inspected inspected once, and at most one check — of every write it planned
/// together — finished before its first mutation reached the board, carrying the title of every
/// record of the plan it writes.
pub(super) fn screened_once_before_writing(what: &str, scenario: &Scenario, titles: &[&str]) {
    for phase in &scenario.boundary {
        let mut inspected = std::collections::BTreeSet::new();
        for call in phase
            .screening
            .iter()
            .filter(|call| call.command == BoundaryCommand::Inspect)
        {
            let repository = call.input["repository"].to_string();
            assert!(
                inspected.insert(repository.clone()),
                "{what}'s {} copy inspected {repository} twice: {:?}",
                phase.kind,
                phase.screening
            );
        }
        let checks: Vec<&BoundaryCall> = phase
            .screening
            .iter()
            .filter(|call| call.command == BoundaryCommand::Check)
            .collect();
        assert!(
            checks.len() <= 1,
            "{what}'s {} copy checked more than once though nothing was refused: {checks:?}",
            phase.kind
        );
        let first_mutation = phase.calls.iter().find(|call| call.creating);
        if let Some(mutation) = first_mutation {
            let check = checks
                .first()
                .unwrap_or_else(|| panic!("{what}'s {} copy wrote without a check", phase.kind));
            assert!(
                check.finished < mutation.wall,
                "{what}'s {} copy wrote {} before its check finished",
                phase.kind,
                operation(mutation)
            );
            assert_eq!(check.exit, Some(0), "the check passed");
            if phase.kind == "project" {
                let carried = check.input["metadata"].as_array().unwrap();
                for title in titles {
                    assert!(
                        carried.iter().any(|value| value == title),
                        "{what}'s one check carried {title:?} too: {carried:?}"
                    );
                }
            }
        }
    }
}

/// The three scenarios on a board of `items` other items, with write screening on when
/// `screened` is: what each sent, by operation, and each as measured.
fn three_scenarios(items: usize, screened: bool) -> Measured {
    let plan = if screened {
        Plan::screened(items, Vec::new(), None)
    } else {
        Plan::new(items, Vec::new())
    };
    scenarios_of(&plan, screened)
}

/// What each of a plan's three scenarios sent, by operation, and each as measured.
type Measured = ([BTreeMap<String, usize>; 3], [Scenario; 3]);

/// The three scenarios of each of three managers copying their plans at once against one
/// loopback host, each on a board of `items` other items: what each manager's commands sent and
/// cost, attributed to that manager alone.
fn managers_at_once(items: usize, screened: bool) -> Vec<Measured> {
    let plans = Plan::managers(MANAGERS, items, screened);
    std::thread::scope(|scope| {
        let running: Vec<_> = plans
            .iter()
            .map(|plan| scope.spawn(move || scenarios_of(plan, screened)))
            .collect();
        running
            .into_iter()
            .map(|manager| manager.join().expect("a manager's copies"))
            .collect()
    })
}

/// The manager sessions the copy budgets' workload states, each running one command at a time:
/// no more than four workers between them.
pub(super) const MANAGERS: usize = 3;

/// The three scenarios of `plan`, in order, as [`three_scenarios`] drives them.
fn scenarios_of(plan: &Plan, screened: bool) -> Measured {
    plan.place_as_measured();
    let titles: Vec<String> = std::iter::once("Copy the plan in a fraction of the time".to_owned())
        .chain((1..=TASKS).map(|n| format!("Step {n} of the plan")))
        .collect();
    let titles: Vec<&str> = titles.iter().map(String::as_str).collect();
    let screening = |what: &str, scenario: &Scenario| {
        if screened {
            screened_once_before_writing(what, scenario, &titles);
        }
    };

    let first = plan.copy_timed();
    never_walks("the first copy", &first.calls);
    let at = counterparts(&first.project, &first.document);
    holds_the_plan(plan, &at, None);
    screening("the first copy", &first);

    let edited = format!(
        "{}An edit made since the last copy.\n",
        plan.generated.tasks[3]
    );
    plan.write_task(4, &edited, true);
    let changed = plan.copy_timed();
    never_walks("the re-copy after one task changed", &changed.calls);
    assert_eq!(
        counterparts(&changed.project, &changed.document),
        at,
        "the re-copy found every item"
    );
    holds_the_plan(plan, &at, Some((4, &edited)));
    screening("the re-copy after one task changed", &changed);

    let unchanged = plan.copy_timed();
    never_walks("the unchanged re-copy", &unchanged.calls);
    assert_eq!(counterparts(&unchanged.project, &unchanged.document), at);
    holds_the_plan(plan, &at, Some((4, &edited)));
    screening("the unchanged re-copy", &unchanged);

    (
        [
            tally(&first.calls),
            tally(&changed.calls),
            tally(&unchanged.calls),
        ],
        [first, changed, unchanged],
    )
}

/// What a copy that `sent` these requests sent, but for its reads of whether its creates are
/// on the board yet, once those reads are held to the plugin's own bound: at least one when it
/// created anything — the project's issue and its tasks' are read together — and at most the
/// six reads and one after a fallback add it allows each create request.
/// How many of them a copy needs is when GitHub placed its creates, which a clock other copies
/// advance too moves, so a test comparing two copies compares everything else exactly.
fn placement_read_bounded(copy: &str, sent: &BTreeMap<String, usize>) -> BTreeMap<String, usize> {
    const PLACEMENT: &str = "reading the board items of the issues a copy created";
    let mut shape = sent.clone();
    let reads = shape.remove(PLACEMENT).unwrap_or(0);
    let creates: usize = ["creating an issue", "creating several issues together"]
        .iter()
        .filter_map(|create| sent.get(*create))
        .sum();
    assert!(
        (creates.min(1)..=7 * creates).contains(&reads),
        "{copy} read its creates' placement {reads} times after {creates} create requests: {sent:?}"
    );
    shape
}

/// A first copy's tally, as the plan's first copy sends it, with `reads` placement reads.
fn first_copy_reading_placement(reads: usize) -> BTreeMap<String, usize> {
    [
        ("creating an issue", 2),
        ("creating several issues together", 1),
        ("writing board fields of the items a copy created", 1),
        (
            "reading the board items of the issues a copy created",
            reads,
        ),
    ]
    .into_iter()
    .map(|(operation, sent)| (operation.to_owned(), sent))
    .collect()
}

#[test]
fn copies_whose_placement_reads_differ_within_the_bound_send_the_same_shape() {
    // A loaded host placing a copy's creates later reads them once or twice more; every other
    // request kind still has to match.
    for reads in [1, 3, 4, 21] {
        assert_eq!(
            placement_read_bounded("a copy", &first_copy_reading_placement(reads)),
            placement_read_bounded("a lone copy", &first_copy_reading_placement(4)),
        );
    }
    let mut written_twice = first_copy_reading_placement(3);
    written_twice.insert("writing board fields of the items a copy created".into(), 2);
    assert_ne!(
        placement_read_bounded("a copy", &written_twice),
        placement_read_bounded("a lone copy", &first_copy_reading_placement(4)),
        "the bound reaches placement reads alone"
    );
}

#[test]
#[should_panic(expected = "read its creates' placement 22 times after 3 create requests")]
fn a_copy_reading_placement_past_the_plugins_bound_fails() {
    placement_read_bounded("a copy", &first_copy_reading_placement(22));
}

#[test]
#[should_panic(expected = "read its creates' placement 0 times after 3 create requests")]
fn a_copy_that_created_and_never_read_placement_fails() {
    placement_read_bounded("a copy", &first_copy_reading_placement(0));
}

#[test]
fn a_plan_copy_reads_what_the_plan_holds_whatever_the_board_has_accumulated() {
    // Each scenario on a board of 2,000 items and of 20,000, with no write policy and with
    // write screening on: what it sends never depends on the board's size, and screening adds
    // no request to the board but at most one listing of the destination project's members. What
    // each cost is recorded as the copy budgets' telemetry.
    let (small, small_runs) = three_scenarios(SMALL, false);
    let (small_screened, small_screened_runs) = three_scenarios(SMALL, true);
    let (large, large_runs) = three_scenarios(LARGE, false);
    let (large_screened, large_screened_runs) = three_scenarios(LARGE, true);
    let managers = managers_at_once(SMALL, false);
    let managers_screened = managers_at_once(SMALL, true);
    for (index, scenario) in [
        "first copy",
        "re-copy after one task changed",
        "unchanged re-copy",
    ]
    .iter()
    .enumerate()
    {
        // How many times a copy reads whether its creates are on the board yet is when GitHub
        // placed them, which a loaded host's scheduling moves; every comparison below holds
        // that count to the plugin's own bound and every other request kind exactly.
        let bounded = |what: &str, sent: &BTreeMap<String, usize>| {
            placement_read_bounded(&format!("{what} {scenario}"), sent)
        };
        let small_shape = bounded("the", &small[index]);
        let small_screened_shape = bounded("the screened", &small_screened[index]);
        assert_eq!(
            small_shape,
            bounded(&format!("the {LARGE}-item"), &large[index]),
            "the {scenario} sent different requests on a board of {SMALL} and of {LARGE}"
        );
        assert_eq!(
            small_screened_shape,
            bounded(
                &format!("the screened {LARGE}-item"),
                &large_screened[index]
            ),
            "the screened {scenario} sent different requests on a board of {SMALL} and of {LARGE}"
        );
        // Screening sends the board nothing more but what a document's classification reads of
        // the destination project's members: a listing of them, or the page of them its
        // batched origin lookup carries in place of that lookup alone. How many is the points
        // budgets' to judge, from the telemetry recorded below.
        let lookup =
            "looking up the items copied from several origins, and what creating one reads";
        let unscreened_shape = |sent: &BTreeMap<String, usize>| {
            let mut shape = sent.clone();
            shape.remove("reading a project's tasks");
            if let Some(coalesced) = shape.remove(
                "looking up the items copied from several origins, with a project's members",
            ) {
                *shape.entry(lookup.to_owned()).or_default() += coalesced;
            }
            shape
        };
        let unlisted = unscreened_shape(&small_screened_shape);
        let expected = unscreened_shape(&small_shape);
        assert_eq!(
            expected, unlisted,
            "screening the {scenario}'s writes changed what it sent the board"
        );
        // Each of three managers copying at once sends, for its own commands, exactly what one
        // alone does, on a clock the three advance together.
        for (manager, (sent, _)) in managers.iter().enumerate() {
            assert_eq!(
                bounded(&format!("manager {manager}'s"), &sent[index]),
                small_shape,
                "manager {manager}'s {scenario}, beside two others, sent what one alone does"
            );
        }
        for (manager, (sent, _)) in managers_screened.iter().enumerate() {
            assert_eq!(
                bounded(&format!("manager {manager}'s screened"), &sent[index]),
                small_screened_shape,
                "manager {manager}'s screened {scenario}, beside two others, sent what one \
                 alone does"
            );
        }
        record(
            SCENARIOS[index],
            &Runs {
                small: &small_runs[index],
                small_screened: &small_screened_runs[index],
                large: &large_runs[index],
                large_screened: &large_screened_runs[index],
                managers: managers.iter().map(|(_, runs)| &runs[index]).collect(),
                managers_screened: managers_screened
                    .iter()
                    .map(|(_, runs)| &runs[index])
                    .collect(),
            },
        );
    }
}

#[test]
fn a_task_taken_out_of_the_plan_is_reported_orphaned_and_left_as_it_was() {
    let plan = Plan::new(SMALL, Vec::new());
    let (project, document, _) = plan.copy();
    let at = counterparts(&project, &document);
    let left = &at["T-8"];
    let before = plan.github.item(left).unwrap();

    // T-8 leaves the project: its file stays, filed under no project.
    plan.write_task(8, &plan.generated.tasks[7], false);
    let (project, document, calls) = plan.copy();
    never_walks("a copy finding an orphan", &calls);
    let orphaned: Vec<&Value> = project["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["action"] == "orphaned")
        .collect();
    assert_eq!(orphaned.len(), 1, "{project:#}");
    assert_eq!(orphaned[0]["source"], "notes:T-8");
    assert_eq!(orphaned[0]["destination"], format!("board:{left}"));
    assert_eq!(
        plan.github.item(left).unwrap(),
        before,
        "an orphan is left on the destination exactly as it was"
    );
    // The document's references to the plan's records still name the same destination
    // records: every task the project still holds, and the project itself.
    assert_eq!(landed(&document, "notes:D"), at["D"]);
    let body = plan.github.body(&at["D"]);
    let (content, _) = split_slot(body.as_str().unwrap());
    for n in 1..TASKS {
        assert!(
            content.contains(&format!(
                "`https://example.invalid/{}`",
                at[&format!("T-{n}")]
            )),
            "the reference to T-{n} names its counterpart"
        );
    }
}

/// The three ways a page of a project's members can answer one member short of what a full read
/// of it says, by what each journey below calls it: with no body, with a metadata slot that does
/// not parse, and with no repository.
fn members_page_misanswers() -> [(&'static str, MembersPageField, Value); 3] {
    [
        ("with no body", MembersPageField::Body, Value::Null),
        (
            "with a metadata slot that does not parse",
            MembersPageField::Body,
            json!("Moved by hand.\n\n<!-- onetaskgraph.metadata\n{\"onetaskgraph.origin\":\n-->"),
        ),
        (
            "with no repository",
            MembersPageField::Repository,
            Value::Null,
        ),
    ]
}

/// The issue of the board's own history a person files under the copied project in the journeys
/// below: a task copied from elsewhere, whose record names no repository list, so its own
/// repository is what says who may read it.
const FILED_BY_HAND: &str = "BG-3";

/// The screened plan copied as a project onto a small board, with [`FILED_BY_HAND`] then filed
/// under that project by a person, and the board's id for the project.
fn screened_project_with_a_member_filed_by_hand() -> (Plan, String) {
    let plan = Plan::screened(20, Vec::new(), None);
    let copied = success(&plan.run(&["project", "copy", "notes:P", "--to", "board", "--json"]));
    let project = landed(&copied, "notes:P");
    plan.github.file_under(FILED_BY_HAND, Some(&project));
    (plan, project)
}

/// Hold the requests a document copy sent the board, `sent`, to having read the members page
/// beside its origin lookup and then read `project`'s members in full, as the listing of a
/// project's tasks reads them.
fn fell_back_to_the_full_members_read(how: &str, sent: &[Call], project: &str) {
    let query = |call: &Call| -> Value { serde_json::from_slice(&call.bytes).unwrap() };
    let page = sent
        .iter()
        .position(|call| {
            call.path == "/graphql"
                && query(call)["query"] == graphql::ORIGIN_LOOKUPS_WITH_MEMBERS
                && query(call)["variables"]["project"] == project
        })
        .unwrap_or_else(|| {
            panic!(
                "a member {how}: the copy asked for no members page of {project}: {:?}",
                tally(sent)
            )
        });
    assert!(
        sent[page + 1..].iter().any(|call| call.path == "/graphql"
            && query(call)["query"] == graphql::SUB_ISSUES
            && query(call)["variables"]["id"] == project),
        "a member {how}: the copy did not read {project}'s members in full after the page: {:?}",
        tally(sent)
    );
}

/// A document copy screened onto a public board, under a project a person filed a member under
/// from a private repository, never lands public on the account of a members page that
/// answers that member short of what reading it says: with the page carrying no body for it, a
/// slot that does not parse or no repository, the copy reads the members in full, finds the
/// private one, and is refused with nothing written.
#[test]
fn a_document_under_a_member_the_members_page_misanswers_is_classified_from_the_full_read() {
    let (owner, _) = SCREENED_REPOSITORY;
    let private = format!("{owner}/secret-gadgets");
    for (how, field, value) in members_page_misanswers() {
        let (mut plan, project) = screened_project_with_a_member_filed_by_hand();
        plan.screening
            .as_mut()
            .expect("the plan screens its writes")
            .0
            .private(owner, "secret-gadgets");
        plan.github.transfer(FILED_BY_HAND, &private);
        plan.github
            .misanswer_in_members_page(FILED_BY_HAND, field, value);
        let before = plan.board.calls().len();
        let output = plan.run(&["document", "copy", "notes:D", "--to", "board", "--json"]);
        let sent = plan.board.calls()[before..].to_vec();
        assert_ne!(
            output.status.code(),
            Some(0),
            "a member {how}: the document landed public beside a private member\n{}\n{}",
            stdout(&output),
            stderr(&output)
        );
        let written: Vec<String> = sent
            .iter()
            .filter(|call| call.creating)
            .map(operation)
            .collect();
        assert!(
            written.is_empty(),
            "a member {how}: the refused copy wrote {written:?}\n{}",
            stderr(&output)
        );
        fell_back_to_the_full_members_read(how, &sent, &project);
        assert!(
            stderr(&output).contains("private"),
            "a member {how}: the refusal says the document is private: {}",
            stderr(&output)
        );
    }
}

/// The same misanswered members page under a project whose members are all public: the copy
/// reads them in full, classifies the document from what that read says, and lands it.
#[test]
fn a_document_under_public_members_the_members_page_misanswers_lands_after_the_full_read() {
    for (how, field, value) in members_page_misanswers() {
        let (plan, project) = screened_project_with_a_member_filed_by_hand();
        plan.github
            .misanswer_in_members_page(FILED_BY_HAND, field, value);
        let before = plan.board.calls().len();
        let document =
            success(&plan.run(&["document", "copy", "notes:D", "--to", "board", "--json"]));
        let sent = plan.board.calls()[before..].to_vec();
        fell_back_to_the_full_members_read(how, &sent, &project);
        let landed = landed(&document, "notes:D");
        assert_eq!(
            plan.github.item(&landed).unwrap()["parent"],
            json!(project),
            "a member {how}: the document lands under its project"
        );
    }
}

#[test]
fn the_board_holds_each_request_for_the_spikes_mean_latency_of_its_kind() {
    // A first copy of the plan sends item reads, batched reads, single and aliased mutations,
    // an upload and its verification; a re-copy of the document after its image changed sends
    // the repository read an update's upload needs; a board listing sends board and search
    // pages; and a detail read of several tasks batches them. Every request of them is held for
    // exactly its kind's mean, on the virtual clock.
    let plan = Plan::new(30, Vec::new());
    let before = plan.board.calls().len();
    let (project, _, _) = plan.copy();
    std::fs::write(
        plan.root.join("documents/D.assets/diagram.png"),
        images::png(SEED + 1, PNG_BYTES),
    )
    .unwrap();
    plan.measured(&["document", "copy", "notes:D", "--to", "board", "--json"]);
    let tasks: Vec<String> = (1..=3)
        .map(|n| format!("board:{}", landed(&project, &format!("notes:T-{n}"))))
        .collect();
    let lease = plan.clock.lease(0);
    success(&plan.run(&["task", "list", "--source", "board", "--json"]));
    lease.wait_for_detach();
    let mut show = vec!["task", "show-many"];
    show.extend(tasks.iter().map(String::as_str));
    show.push("--json");
    success(&plan.run(&show));
    lease.wait_for_detach();
    drop(lease);
    let calls = &plan.board.calls()[before..];
    let mut seen = std::collections::BTreeSet::new();
    for call in calls {
        assert_eq!(
            (call.answered - call.at).as_millis(),
            u128::from(call.kind.measured_millis()),
            "{} ({:?}) was held for its kind's measured mean",
            operation(call),
            call.kind
        );
        seen.insert(format!("{:?}", call.kind));
    }
    let every: std::collections::BTreeSet<String> = [
        Kind::ItemRead,
        Kind::BoardPage,
        Kind::SearchPage,
        Kind::BatchedRead,
        Kind::Mutation,
        Kind::AliasedMutations,
        Kind::Upload,
        Kind::Repository,
        Kind::Verification,
    ]
    .iter()
    .map(|kind| format!("{kind:?}"))
    .collect();
    assert_eq!(seen, every, "one request of every kind was driven");
    for (kind, millis) in [
        (Kind::ItemRead, 554),
        (Kind::BoardPage, 6_948),
        (Kind::SearchPage, 6_791),
        (Kind::Mutation, 1_082),
        (Kind::AliasedMutations, 3_700),
        (Kind::Upload, 927),
        (Kind::Repository, 526),
        (Kind::Verification, 1_140),
        (Kind::BatchedRead, 4_551),
    ] {
        assert_eq!(
            kind.measured_millis(),
            millis,
            "{kind:?} is the spike's mean"
        );
    }
}

/// The documents that read items of the board by their node ids — the reads a change token
/// stands in for — rather than listing them.
const ITEM_READS: [&str; 5] = [
    graphql::ISSUE,
    graphql::DOCUMENT_IN_PROJECT,
    graphql::ISSUE_DETAIL,
    graphql::ISSUE_DETAILS,
    graphql::ISSUE_COPY_DETAILS,
];

/// Every item read among `calls`, as its document and the ids its variables name.
pub(super) fn item_reads(calls: &[Call]) -> Vec<(String, std::collections::BTreeSet<String>)> {
    calls
        .iter()
        .filter_map(|call| {
            let request: Value = serde_json::from_slice(&call.bytes).ok()?;
            let query = request["query"].as_str()?;
            ITEM_READS.contains(&query).then(|| {
                let ids = request["variables"]
                    .as_object()
                    .into_iter()
                    .flatten()
                    .filter(|(name, _)| {
                        name.strip_prefix("id")
                            .is_some_and(|slot| slot.bytes().all(|byte| byte.is_ascii_digit()))
                    })
                    .filter_map(|(_, id)| id.as_str().map(str::to_owned))
                    .collect();
                (query.to_owned(), ids)
            })
        })
        .collect()
}

/// Every id an item read among `calls` names.
fn named_by_item_reads(calls: &[Call]) -> std::collections::BTreeSet<String> {
    item_reads(calls)
        .into_iter()
        .flat_map(|(_, ids)| ids)
        .collect()
}

impl Plan {
    /// Fill the board's cache with what the reads a person makes leave there: every task of
    /// the plan, the project and the document, each at its current change token.
    fn read_back(&self, at: &Counterparts) {
        let lease = self.clock.lease(self.client);
        let mut tasks = vec!["task".to_owned(), "show-many".to_owned()];
        tasks.extend((1..=TASKS).map(|n| format!("board:{}", at[&format!("T-{n}")])));
        tasks.push("--json".to_owned());
        for arguments in [
            tasks,
            vec![
                "project".into(),
                "show".into(),
                format!("board:{}", at["P"]),
                "--json".into(),
            ],
            vec![
                "document".into(),
                "show".into(),
                format!("board:{}", at["D"]),
                "--json".into(),
            ],
        ] {
            success(&self.run(&arguments.iter().map(String::as_str).collect::<Vec<_>>()));
            lease.wait_for_detach();
        }
    }

    /// One command through the binary, and every request the board answered for it.
    pub(super) fn measured(&self, arguments: &[&str]) -> (Value, Vec<Call>) {
        let (output, calls) = self.measured_output(arguments);
        (success(&output), calls)
    }

    /// [`Self::measured`], for a command that may fail: what it exited with and printed.
    pub(super) fn measured_output(&self, arguments: &[&str]) -> (Output, Vec<Call>) {
        let before = self.board.calls().len();
        let lease = self.clock.lease(self.client);
        let output = self.run(arguments);
        lease.wait_for_detach();
        drop(lease);
        (output, self.own_calls(before))
    }
}

pub(super) fn plan_ids(at: &Counterparts) -> std::collections::BTreeSet<String> {
    at.values().cloned().collect()
}

#[test]
fn a_recopy_with_no_token_reads_every_counterpart_in_batches_and_none_on_its_own() {
    // (b): no cache, so no item has a token the copy could vouch for: the project and its ten
    // tasks are read in aliased batches of at most COPY_DETAIL_BATCH, none by its own read. The
    // document copy that follows has one counterpart to read, its own, and reads it alone.
    let plan = Plan::new(SMALL, Vec::new());
    let (project, document, _) = plan.copy();
    let at = counterparts(&project, &document);
    let (_, _, calls) = plan.copy();
    let reads = item_reads(&calls);
    let alone: Vec<&(String, std::collections::BTreeSet<String>)> = reads
        .iter()
        .filter(|(document, _)| document != graphql::ISSUE_COPY_DETAILS)
        .collect();
    assert_eq!(
        alone,
        vec![&(
            graphql::DOCUMENT_IN_PROJECT.to_owned(),
            std::collections::BTreeSet::from([at["D"].clone()])
        )],
        "an item read that is not a batch, but the document's own: {reads:?}"
    );
    assert!(
        reads
            .iter()
            .all(|(_, ids)| ids.len() <= onetaskgraph_github_projects::COPY_DETAIL_BATCH),
        "a batch over COPY_DETAIL_BATCH: {reads:?}"
    );
    assert_eq!(
        named_by_item_reads(&calls),
        plan_ids(&at),
        "every counterpart was read"
    );
    holds_the_plan(&plan, &at, None);
}

/// Every plan item the board holds, as it holds it, apart from the time GitHub stamps on any
/// write: a write that changes nothing of an item still moves its `updatedAt`, and leaving an
/// unchanged item unwritten is the write path's to do, not a read's.
fn held_content(plan: &Plan, at: &Counterparts) -> BTreeMap<String, Option<Value>> {
    plan_ids(at)
        .into_iter()
        .map(|id| {
            let mut item = plan.github.item(&id);
            if let Some(Value::Object(fields)) = &mut item {
                fields.remove("updatedAt");
            }
            (id, item)
        })
        .collect()
}

#[test]
fn a_recopy_reads_nothing_its_cache_holds_at_the_token_the_listing_carries() {
    // (a): every one of the ten destination items is answered without a read naming it — the
    // project and its tasks by the members listing the copy makes anyway, at the state and
    // token GitHub answered in this command, the project's own edges with them, and the
    // document by its cache entry at the token its listing carries — so neither the project
    // copy nor the document copy after it sends one, and the board holds the plan as it did.
    //
    // The plan is copied twice before the cache is filled: the first copy records each source
    // file's token from before it wrote that file's link, so the second is a real source change
    // and rewrites the record. From the third on, nothing at the source has changed.
    let plan = Plan::configured(SMALL, Vec::new(), Some(json!({"enabled": true})));
    let (project, document, _) = plan.copy();
    let at = counterparts(&project, &document);
    plan.copy();
    plan.read_back(&at);
    let before = held_content(&plan, &at);

    for (what, arguments) in [
        (
            "the project copy",
            ["project", "copy", "notes:P", "--to", "board", "--json"],
        ),
        (
            "the document copy",
            ["document", "copy", "notes:D", "--to", "board", "--json"],
        ),
    ] {
        let (_, calls) = plan.measured(&arguments);
        never_walks(what, &calls);
        let named: std::collections::BTreeSet<String> = named_by_item_reads(&calls)
            .intersection(&plan_ids(&at))
            .cloned()
            .collect();
        assert!(
            named.is_empty(),
            "{what} read {named:?}, though each is answered without a read: {:?}",
            item_reads(&calls)
        );
    }
    holds_the_plan(&plan, &at, None);
    assert_eq!(
        held_content(&plan, &at),
        before,
        "the destination items are unchanged"
    );
}

#[test]
fn a_recopy_reads_only_what_neither_token_nor_cache_answers_and_keeps_a_persons_edit() {
    // (c): the cache holds the plan at its tokens; then a person fills in a board field of
    // their own on T-3's card, which moves its token past its entry, and the plan gains a ninth
    // task never copied. The members listing the copy sends first carries every member at the
    // state GitHub holds now — T-3 with the person's edit and the token it moved to — so no
    // plan item is left unanswered and none is read on its own: T-3 is read, by that listing,
    // before it is written. The ninth has no counterpart to read and is created, and T-3 keeps
    // the field the copy never writes.
    let plan = Plan::configured(SMALL, Vec::new(), Some(json!({"enabled": true})));
    let (project, document, _) = plan.copy();
    let at = counterparts(&project, &document);
    plan.read_back(&at);
    let edited = &at["T-3"];
    plan.github.set_card_field(edited, "Reviewer", "a person");
    std::fs::write(
        plan.root.join("tasks/T-9.md"),
        "---\ntitle: Step 9 of the plan\nstatus: todo\nproject: P\n---\nA step added since.\n",
    )
    .unwrap();

    let (report, calls) = plan.measured(&["project", "copy", "notes:P", "--to", "board", "--json"]);
    never_walks("a partly cached project copy", &calls);
    let reads = item_reads(&calls);
    assert!(
        reads
            .iter()
            .all(|(document, ids)| document == graphql::ISSUE_COPY_DETAILS
                && ids.len() <= onetaskgraph_github_projects::COPY_DETAIL_BATCH),
        "{reads:?}"
    );
    let named = named_by_item_reads(&calls);
    assert!(
        named.intersection(&plan_ids(&at)).next().is_none(),
        "no plan item is read on its own: the listing answers every one — T-3 at the token \
         its edit moved it to, and the rest at theirs: {reads:?}"
    );
    // The members listing is what read T-3 as the person left it: the request naming the
    // project's members, carrying each at the state GitHub answered.
    let read_at = calls
        .iter()
        .position(|call| operation(call) == "reading a project's tasks")
        .unwrap();
    let written_at = calls.iter().position(|call| {
        call.kind == Kind::Mutation
            && String::from_utf8_lossy(&call.bytes).contains(edited.as_str())
    });
    assert!(
        written_at.is_none_or(|written| read_at < written),
        "read before written"
    );
    assert_eq!(
        plan.github.item(edited).unwrap()["own_fields"]["Reviewer"],
        json!("a person"),
        "the person's edit is kept"
    );
    let ninth = landed(&report, "notes:T-9");
    assert_eq!(plan.github.item(&ninth).unwrap()["parent"], json!(at["P"]));
}

#[test]
fn a_recopy_whose_pinning_listings_are_refused_reads_what_they_would_have_carried_and_still_lands()
{
    // Both listings that would pin every member's token and carry the plan — the project's
    // tasks, then its documents, which are sub-issues of it too — are refused once each:
    // nothing is pinned or carried, so every counterpart is read as if there were no cache —
    // in one batch — and the copy lands the plan exactly as it would have. That batch names the
    // project's own sub-issues too, so the orphan check is answered by it and lists nothing.
    let plan = Plan::configured(SMALL, Vec::new(), Some(json!({"enabled": true})));
    let (project, document, _) = plan.copy();
    let at = counterparts(&project, &document);
    plan.read_back(&at);
    plan.github.refuse_once("subIssues(first:$memberFirst");
    plan.github.refuse_once("subIssues(first:$memberFirst");
    let (report, calls) = plan.measured(&["project", "copy", "notes:P", "--to", "board", "--json"]);
    assert!(
        report["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["action"] != "failed"),
        "{report:#}"
    );
    let mut expected: std::collections::BTreeSet<String> =
        (1..=TASKS).map(|n| at[&format!("T-{n}")].clone()).collect();
    expected.insert(at["P"].clone());
    assert_eq!(
        named_by_item_reads(&calls),
        expected,
        "every counterpart is read instead"
    );
    assert_eq!(
        tally(&calls)["reading a project's tasks"],
        2,
        "the two refused pinning listings — the orphan check is answered by the batched read, \
         which named the project's sub-issues: {:?}",
        tally(&calls)
    );
    holds_the_plan(&plan, &at, None);
}

#[test]
fn an_origin_its_batched_lookup_cannot_answer_in_one_page_is_looked_up_on_its_own() {
    // Four unrelated issues mention `notes:T-1` in their bodies, more than one page of the
    // batched lookup's issue search holds: that origin is left to its own lookup, which walks
    // every page, finds none of them copied from it, and the copy creates T-1 once — and finds
    // that one item, and no decoy, on the next copy.
    let decoys: Vec<Value> = (0..4)
        .map(|n| {
            let mut decoy = background(9_001 + n);
            decoy["parent"] = Value::Null;
            decoy["body"] = json!(format!(
                "Mentions notes:T-1 in passing.\n\n<!-- onetaskgraph.metadata\n{}\n-->",
                json!({"onetaskgraph.origin": format!("elsewhere:decoy-{n}")})
            ));
            decoy
        })
        .collect();
    let plan = Plan::new(SMALL, decoys);
    let (project, document, calls) = plan.copy();
    let at = counterparts(&project, &document);
    holds_the_plan(&plan, &at, None);
    assert!(
        tally(&calls)
            .keys()
            .any(|operation| operation == "looking up the items copied from one origin"),
        "{:?}",
        tally(&calls)
    );
    let (project, document, _) = plan.copy();
    assert_eq!(
        counterparts(&project, &document),
        at,
        "no duplicate, no decoy"
    );
    for n in 0..4 {
        let decoy = plan
            .github
            .item(&format!("BG-{}", 9_001 + n))
            .expect("the decoy");
        assert!(
            decoy["body"]
                .as_str()
                .unwrap()
                .starts_with("Mentions notes:T-1")
        );
    }
}

#[test]
fn a_project_whose_sub_issues_come_back_unlisted_still_finds_the_task_taken_out_of_it() {
    // The project's sub-issues ride along with the copy's batched read, and that answer leaves
    // their list out: it is no answer, not an empty project, so the orphan check reads the
    // project's own sub-issues and still finds the task taken out of it.
    let plan = Plan::new(SMALL, Vec::new());
    let (project, document, _) = plan.copy();
    let at = counterparts(&project, &document);
    plan.write_task(8, &plan.generated.tasks[7], false);
    plan.github.omit_sub_issue_list_once();
    let (project, _, calls) = plan.copy();
    let orphaned: Vec<&Value> = project["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["action"] == "orphaned")
        .collect();
    assert_eq!(orphaned.len(), 1, "{project:#}");
    assert_eq!(orphaned[0]["destination"], format!("board:{}", at["T-8"]));
    assert!(
        tally(&calls).contains_key("reading a project's tasks"),
        "{:?}",
        tally(&calls)
    );
}

/// Record, in the metadata slot of board item `id`, that its work changes `repository`: what a
/// person editing the list the slot keeps on GitHub leaves behind.
pub(super) fn record_repository(plan: &Plan, id: &str, repository: &str) {
    let body = plan.github.body(id);
    let (content, mut slot) = split_slot(body.as_str().unwrap());
    slot["onetaskgraph.repositories"] = json!([repository]);
    let content = content.to_owned();
    plan.github.edit_body(
        id,
        &format!("{content}\n\n<!-- onetaskgraph.metadata\n{slot}\n-->"),
    );
}

#[test]
fn a_screened_document_copy_under_a_project_a_person_made_private_is_refused_writing_nothing() {
    // The plan lands on the board screened. Then a person records one of its tasks there as
    // changing a private repository, which makes the project private by what it holds. The
    // document copied again — its content changed, so it would be written — is held to the
    // project it is filed under on the destination, read again in this command: it is refused
    // before anything is sent, and the board keeps the document as it was.
    let mut plan = Plan::screened(SMALL, Vec::new(), None);
    let (project, document, _) = plan.copy();
    let at = counterparts(&project, &document);
    plan.screening
        .as_mut()
        .expect("a screened plan")
        .0
        .private("synthco", "secret-gadgets");
    record_repository(&plan, &at["T-3"], "github.com/synthco/secret-gadgets");
    let held = plan.github.item(&at["D"]).unwrap();
    let path = plan.root.join("documents/D.md");
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, format!("{text}\nA paragraph added since.\n")).unwrap();
    let (output, calls) =
        plan.measured_output(&["document", "copy", "notes:D", "--to", "board", "--json"]);
    assert_ne!(output.status.code(), Some(0), "{}", stdout(&output));
    assert!(
        calls.iter().all(|call| !call.creating),
        "a refused copy sent {:?}",
        tally(&calls)
    );
    assert_eq!(plan.github.item(&at["D"]).unwrap(), held);
    let said = stderr(&output);
    assert!(said.contains("notes:D"), "{said}");
}

/// One request of a recorded trace: its operation, and when it started and ended, in seconds.
struct Traced {
    op: String,
    start: f64,
    end: f64,
}

/// The requests of the 0.11.0 re-measurement `run` on the real board, in the order they were
/// sent, from the sanitized trace committed beside the measurement.
fn traced(run: &str) -> Vec<Traced> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/measurements/board-copy-speed-2026-10-18/after")
        .join(run)
        .join("requests.jsonl");
    let mut traced: Vec<Traced> = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
        .lines()
        .map(|line| {
            let request: Value = serde_json::from_str(line).unwrap();
            Traced {
                op: request["op"].as_str().unwrap().to_owned(),
                start: request["start"].as_f64().unwrap(),
                end: request["end"].as_f64().unwrap(),
            }
        })
        .filter(|request| !request.op.starts_with("CONNECT "))
        .collect();
    traced.sort_by(|a, b| a.start.total_cmp(&b.start));
    traced
}

#[test]
fn the_loopback_board_answers_0_11s_measured_copies_as_github_answered_them() {
    // Fixture evidence, not a bound: 0.11.0's own copies of the plan on the real board, replayed
    // at their measured times against the loopback board placing issues as the spike measured.
    let first = traced("tb-first-span-20261024T095836");
    let unchanged = traced("tb-unchanged-span-20261024T095930");
    let count = |traced: &[Traced], op: &str| traced.iter().filter(|r| r.op == op).count();
    // The trace this replays is the one the copy budgets' baselines name — the six-point
    // unchanged re-copy and the twenty-point first copy by the spike's prices — held here by what
    // each sent: the re-copy two project listings and four single origin lookups and nothing
    // else, and the first copy four placement polls and three single origin lookups among its
    // eighteen GraphQL requests.
    let graphql = |traced: &[Traced]| {
        traced
            .iter()
            .filter(|request| !request.op.starts_with("POST ") && !request.op.starts_with("GET "))
            .count()
    };
    assert_eq!(graphql(&unchanged), 6);
    assert_eq!(count(&unchanged, "SUB_ISSUES"), 2);
    assert_eq!(count(&unchanged, "ORIGIN_LOOKUP"), 4);
    assert_eq!(graphql(&first), 18);
    assert_eq!(count(&first, "CREATED_BOARD_ITEMS"), 4);
    assert_eq!(count(&first, "ORIGIN_LOOKUP"), 3);

    // Each create, then each placement poll after it at the time it was sent after the create's
    // answer, on a clock the replay keeps.
    let sandbox = Sandbox::new();
    let (_, github) = github_projects_with_items(&sandbox, Vec::new());
    let now = std::sync::Arc::new(std::sync::Mutex::new(std::time::Duration::ZERO));
    let read = now.clone();
    github.place_filed_issues_after(
        MEASURED_PLACEMENT_MILLIS
            .iter()
            .map(|millis| std::time::Duration::from_millis(*millis))
            .collect(),
        move || *read.lock().unwrap(),
    );
    let at = |seconds: f64| std::time::Duration::from_secs_f64(seconds);
    let origin = first[0].start;
    let create = |title: &str, parent: Option<&str>| {
        let mut input = json!({"repositoryId":"REPO-1","title":title,"body":"replayed",
                               "projectV2Ids":["PVT-board"]});
        if let Some(parent) = parent {
            input["parentIssueId"] = json!(parent);
        }
        input
    };
    let poll = |ids: &[String]| -> Vec<bool> {
        let mut variables = serde_json::Map::new();
        for slot in 0..onetaskgraph_github_projects::COPY_DETAIL_BATCH {
            variables.insert(format!("id{slot}"), json!(ids.get(slot).unwrap_or(&ids[0])));
            variables.insert(format!("read{slot}"), json!(slot < ids.len()));
        }
        variables.insert("boardItems".into(), json!(3));
        variables.insert("statusName".into(), json!("Status"));
        let answered = github.graphql(graphql::CREATED_BOARD_ITEMS, &Value::Object(variables));
        (0..ids.len())
            .map(|slot| {
                answered[format!("i{slot}")]["projectItems"]["nodes"]
                    .as_array()
                    .is_some_and(|nodes| !nodes.is_empty())
            })
            .collect()
    };
    let mut created: Vec<String> = Vec::new();
    let mut every: Vec<String> = Vec::new();
    let mut placed_by_poll = Vec::new();
    // A create after a poll is the next command's, which polls for its own creates alone.
    let mut polled = false;
    for request in &first {
        match request.op.as_str() {
            "CREATE_ISSUE" => {
                if std::mem::take(&mut polled) {
                    created.clear();
                }
                *now.lock().unwrap() = at(request.start - origin);
                let parent = created.first().cloned();
                let answered = github.graphql(
                    graphql::CREATE_ISSUE,
                    &json!({"input": create("replayed", parent.as_deref())}),
                );
                let id = answered["createIssue"]["issue"]["id"]
                    .as_str()
                    .unwrap()
                    .to_owned();
                every.push(id.clone());
                created.push(id);
                github.answered_at(at(request.end - origin));
            }
            "CREATE_ISSUES" => {
                *now.lock().unwrap() = at(request.start - origin);
                let mut variables = serde_json::Map::new();
                for slot in 0..onetaskgraph_github_projects::MUTATION_BATCH {
                    variables.insert(format!("create{slot}"), create("child", Some(&created[0])));
                    variables.insert(format!("send{slot}"), json!(slot < 8));
                }
                let answered = github.graphql(graphql::CREATE_ISSUES, &Value::Object(variables));
                for slot in 0..8 {
                    let id = answered[format!("create{slot}")]["issue"]["id"]
                        .as_str()
                        .unwrap()
                        .to_owned();
                    every.push(id.clone());
                    created.push(id);
                }
                github.answered_at(at(request.end - origin));
            }
            "CREATED_BOARD_ITEMS" => {
                polled = true;
                *now.lock().unwrap() = at(request.start - origin);
                let unplaced: Vec<String> = created.clone();
                let placed = poll(&unplaced);
                placed_by_poll.push(placed.iter().filter(|placed| **placed).count());
                created = unplaced
                    .into_iter()
                    .zip(placed)
                    .filter(|(_, placed)| !placed)
                    .map(|(id, _)| id)
                    .collect();
            }
            _ => {}
        }
    }
    // As on GitHub: no create's answer carried a board item, so the first poll after each
    // copy's creates — sent within a tenth of a second of the answer — found them not all placed,
    // which is why 0.11 polled again; and every issue is placed once the longest delay measured
    // has passed since its create was answered.
    assert_eq!(placed_by_poll.len(), 4, "{placed_by_poll:?}");
    assert!(placed_by_poll[0] < 9, "{placed_by_poll:?}");
    assert_eq!(placed_by_poll[2], 0, "{placed_by_poll:?}");
    let last = first.iter().map(|request| request.end).fold(0.0, f64::max);
    *now.lock().unwrap() = at(last - origin) + std::time::Duration::from_millis(1_837);
    assert!(
        poll(&every).into_iter().all(|placed| placed),
        "every issue placed"
    );
    assert_eq!(every.len(), 10);
}

/// The plan's project copied onto the board on its own, attached to the clock, as the first
/// command of a plan copy sends it: the board ids of the plan's records.
fn project_landed(plan: &Plan) -> Counterparts {
    let (report, _) = plan.measured(&["project", "copy", "notes:P", "--to", "board", "--json"]);
    let mut at = Counterparts::new();
    at.insert("P".into(), landed(&report, "notes:P"));
    for n in 1..=TASKS {
        at.insert(format!("T-{n}"), landed(&report, &format!("notes:T-{n}")));
    }
    at
}

/// The coalesced lookup's own name for what it sends.
const COALESCED: &str =
    "looking up the items copied from several origins, with a project's members";

#[test]
fn a_documents_first_screened_copy_classifies_its_project_by_the_members_its_lookup_carries() {
    // The project lands, and then a person records one of its tasks on the board as changing a
    // private repository. The document's first copy reads its project's members in the same
    // request as its origins, finds the project private by what it holds, and is refused
    // before anything is sent — the members read nowhere else.
    let mut plan = Plan::screened(SMALL, Vec::new(), None);
    let at = project_landed(&plan);
    plan.screening
        .as_mut()
        .expect("a screened plan")
        .0
        .private("synthco", "secret-gadgets");
    record_repository(&plan, &at["T-3"], "github.com/synthco/secret-gadgets");
    let (output, calls) =
        plan.measured_output(&["document", "copy", "notes:D", "--to", "board", "--json"]);
    assert_ne!(output.status.code(), Some(0), "{}", stdout(&output));
    let sent = tally(&calls);
    assert_eq!(sent.get(COALESCED), Some(&1), "{sent:?}");
    assert!(
        !sent.contains_key("reading a project's tasks"),
        "the members came with the lookup: {sent:?}"
    );
    assert!(
        calls.iter().all(|call| !call.creating),
        "a refused copy sent {sent:?}"
    );
    let failure: Value = serde_json::from_str(&stdout(&output)).expect("a failure document");
    assert_eq!(
        failure["failure"]["kind"], "not-private-destination",
        "{failure}"
    );
}

#[test]
fn a_documents_first_screened_copy_lists_its_projects_members_whole_when_the_page_has_more() {
    // The project holds more members than the page the lookup carries: that page is no answer
    // about all of them, so the copy lists the members whole before it is held to them, and
    // lands.
    let plan = Plan::screened(SMALL, Vec::new(), None);
    let at = project_landed(&plan);
    for n in 1..=20 {
        plan.github
            .file_under(&format!("BG-{n}"), Some(at["P"].as_str()));
    }
    let (_, calls) = plan.measured(&["document", "copy", "notes:D", "--to", "board", "--json"]);
    let sent = tally(&calls);
    assert_eq!(sent.get(COALESCED), Some(&1), "{sent:?}");
    assert!(
        sent.contains_key("reading a project's tasks"),
        "the members were listed whole: {sent:?}"
    );
}

/// spike-host-copy-parity: one measurement of one named variant of the host's plan copy.
mod parity;
