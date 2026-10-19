//! The plan copy the copy budgets read: the workload the timing spike measured
//! (`board-copy-speed-2026-10-18-spike-copy-timing-report`), generated from a seed, copied onto
//! a loopback board holding thousands of other items through the real binary, on the simulated
//! clock, with each request held for the spike's measured latency of its kind.
//!
//! What this holds is the shape of what a copy sends — which operations, how many of each —
//! and that the board ends holding exactly the plan. Neither may depend on how many items the
//! board has accumulated, so every scenario is driven on a board of 2,000 items and again on
//! one of 20,000, and the two request tallies are held equal. It asserts no time and no point
//! total against a bound: that is the copy budgets' job, recorded on top of this journey.

use super::asset_board::{Board, Call, Kind};
use onetaskgraph_e2e_support::{
    binary,
    clock::SimulatedClock,
    common::{Sandbox, stderr, stdout},
    fixtures::{GitHubBoardFields, document, github_projects_with_items},
    images,
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
/// by the path its folder reports and referencing one 104 KB PNG.
pub(super) struct Generated {
    pub project: String,
    pub tasks: Vec<String>,
    pub document: String,
    pub png: Vec<u8>,
}

impl Generated {
    pub fn new(seed: u64) -> Self {
        let mut stream = Seeded(seed);
        let project = stream.prose(50_100);
        let tasks = (0..TASKS)
            .map(|index| stream.prose(45_200 + index * 500))
            .collect();
        let document = stream.prose(33_000);
        Self {
            project,
            tasks,
            document,
            png: images::png(seed, PNG_BYTES),
        }
    }
}

/// The folder of Markdown the plan is written to, and the loopback board it is copied onto.
pub(super) struct Plan {
    pub sandbox: Sandbox,
    pub root: PathBuf,
    pub board: Board,
    pub github: GitHubBoardFields,
    pub clock: SimulatedClock,
    pub generated: Generated,
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
        let clock = SimulatedClock::start(1);
        let sandbox = Sandbox::new();
        let mut held: Vec<Value> = (0..items).map(background).collect();
        held.extend(extra);
        let (mut config, github) = github_projects_with_items(&sandbox, held);
        let board = Board::new(config["endpoint"].as_str().unwrap(), &clock, 0, None);
        board.measure_latency();
        config["endpoint"] = json!(board.endpoint);
        config["pacing"] = json!({});
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
        sandbox.project_document(&document(&json!({
            "notes": {"plugin":"local-md","config":{"root":root}},
            "board": board_source
        })));
        let plan = Self {
            sandbox,
            root,
            board,
            github,
            clock,
            generated: Generated::new(SEED),
        };
        plan.author();
        plan
    }

    /// The path the folder reports for task `n`, which the document names it by.
    pub fn task_path(&self, n: usize) -> String {
        self.root
            .join(format!("tasks/T-{n}.md"))
            .to_str()
            .unwrap()
            .to_owned()
    }

    /// Write the project, its tasks and its document, each with the content generated for it.
    fn author(&self) {
        let generated = &self.generated;
        std::fs::write(
            self.root.join("projects/P.md"),
            format!(
                "---\ntitle: Copy the plan in a fraction of the time\nstatus: todo\n---\n{}",
                generated.project
            ),
        )
        .unwrap();
        for n in 1..=TASKS {
            self.write_task(n, &generated.tasks[n - 1], true);
        }
        let mut table = String::from("| Task | Where |\n| --- | --- |\n");
        for n in 1..=TASKS {
            table.push_str(&format!("| T-{n} | `{}` |\n", self.task_path(n)));
        }
        std::fs::write(
            self.root.join("documents/D.assets/diagram.png"),
            &generated.png,
        )
        .unwrap();
        std::fs::write(
            self.root.join("documents/D.md"),
            format!(
                "---\ntitle: Design of the plan copy\nproject: P\n---\n{}\n## Planned tasks\n\n{table}\n![Where the copy spends its time](./diagram.png)\n",
                generated.document
            ),
        )
        .unwrap();
    }

    /// Write task `n` with `content`, filed under the project or not, keeping whatever metadata
    /// an earlier copy recorded in its front matter.
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
            .unwrap_or_default();
        let depends: Vec<String> = EDGES
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

    /// One command through the binary, attached to the clock.
    pub fn run(&self, arguments: &[&str]) -> Output {
        self.sandbox
            .subprocess(binary())
            .args(arguments)
            .envs(self.clock.client_env(0))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
            .wait_with_output()
            .unwrap()
    }

    /// One scenario as `just copy-plan` sends it — `project copy` then `document copy` — with
    /// both reports and every request the board answered for the two.
    pub fn copy(&self) -> (Value, Value, Vec<Call>) {
        let before = self.board.calls().len();
        let lease = self.clock.lease(0);
        let project =
            success(&self.run(&["project", "copy", "notes:P", "--to", "board", "--json"]));
        lease.wait_for_detach();
        let document =
            success(&self.run(&["document", "copy", "notes:D", "--to", "board", "--json"]));
        lease.wait_for_detach();
        drop(lease);
        (project, document, self.board.calls()[before..].to_vec())
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

/// Every request of `calls`, counted by operation.
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

/// Hold the board to holding exactly the generated plan, `changed` naming a task whose
/// content was replaced since: the project, eight tasks and the document each with the
/// source's title, content, state and board fields, every task a sub-issue of the project,
/// the ten blocked-by edges and no others, and the document referencing each task's
/// counterpart and its PNG as an uploaded asset whose bytes are the source's.
fn holds_the_plan(plan: &Plan, at: &Counterparts, changed: Option<(usize, &str)>) {
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

/// The three scenarios on a board of `items` other items: what each sent, by operation.
fn three_scenarios(items: usize) -> [BTreeMap<String, usize>; 3] {
    let plan = Plan::new(items, Vec::new());

    let (project, document, first) = plan.copy();
    never_walks("the first copy", &first);
    let at = counterparts(&project, &document);
    holds_the_plan(&plan, &at, None);

    let edited = format!(
        "{}An edit made since the last copy.\n",
        plan.generated.tasks[3]
    );
    plan.write_task(4, &edited, true);
    let (project, document, changed) = plan.copy();
    never_walks("the re-copy after one task changed", &changed);
    assert_eq!(
        counterparts(&project, &document),
        at,
        "the re-copy found every item"
    );
    holds_the_plan(&plan, &at, Some((4, &edited)));

    let (project, document, unchanged) = plan.copy();
    never_walks("the unchanged re-copy", &unchanged);
    assert_eq!(counterparts(&project, &document), at);
    holds_the_plan(&plan, &at, Some((4, &edited)));

    [tally(&first), tally(&changed), tally(&unchanged)]
}

#[test]
fn a_plan_copy_reads_what_the_plan_holds_whatever_the_board_has_accumulated() {
    let small = three_scenarios(SMALL);
    let large = three_scenarios(LARGE);
    for (index, scenario) in [
        "first copy",
        "re-copy after one task changed",
        "unchanged re-copy",
    ]
    .iter()
    .enumerate()
    {
        assert_eq!(
            small[index], large[index],
            "the {scenario} sent different requests on a board of {SMALL} and of {LARGE}"
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

#[test]
fn the_board_holds_each_request_for_the_spikes_mean_latency_of_its_kind() {
    // A first copy of the plan sends item reads, single and aliased mutations, an upload, its
    // repository read and its verification; a board listing sends board and search pages; and
    // a detail read of several tasks batches them. Every request of the three is held for
    // exactly its kind's mean, on the virtual clock.
    let plan = Plan::new(30, Vec::new());
    let before = plan.board.calls().len();
    let (project, _, _) = plan.copy();
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
const ITEM_READS: [&str; 4] = [
    graphql::ISSUE,
    graphql::ISSUE_DETAIL,
    graphql::ISSUE_DETAILS,
    graphql::ISSUE_COPY_DETAILS,
];

/// Every item read among `calls`, as its document and the ids its variables name.
fn item_reads(calls: &[Call]) -> Vec<(String, std::collections::BTreeSet<String>)> {
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
        let lease = self.clock.lease(0);
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
        let before = self.board.calls().len();
        let lease = self.clock.lease(0);
        let report = success(&self.run(arguments));
        lease.wait_for_detach();
        drop(lease);
        (report, self.board.calls()[before..].to_vec())
    }
}

/// The ids of every plan item at the destination.
fn plan_ids(at: &Counterparts) -> std::collections::BTreeSet<String> {
    at.values().cloned().collect()
}

#[test]
fn a_recopy_with_no_token_reads_every_counterpart_in_batches_and_none_on_its_own() {
    // (b): no cache, so no item has a token the copy could vouch for: the ten are read, every
    // one of them in an aliased batch of at most COPY_DETAIL_BATCH, and none by its own read.
    let plan = Plan::new(SMALL, Vec::new());
    let (project, document, _) = plan.copy();
    let at = counterparts(&project, &document);
    let (_, _, calls) = plan.copy();
    let reads = item_reads(&calls);
    assert!(
        reads
            .iter()
            .all(|(document, ids)| document == graphql::ISSUE_COPY_DETAILS
                && ids.len() <= onetaskgraph_github_projects::COPY_DETAIL_BATCH),
        "an item read that is not a batch: {reads:?}"
    );
    assert_eq!(
        named_by_item_reads(&calls),
        plan_ids(&at),
        "every counterpart was read"
    );
    holds_the_plan(&plan, &at, None);
}

#[test]
fn a_recopy_reads_nothing_its_cache_holds_at_the_token_the_listing_carries() {
    // (a): the cache holds every item at the token the copy's own listings carry, so the
    // project copy reads none of its tasks — the project alone is read, once and in a batch,
    // for the forward edges a GitHub token does not cover — and the document copy does not
    // read the document. What the project copy then rewrites, unchanged, the document copy
    // reads again; leaving unchanged items unwritten is the write path's to do.
    let plan = Plan::configured(SMALL, Vec::new(), Some(json!({"enabled": true})));
    let (project, document, _) = plan.copy();
    let at = counterparts(&project, &document);
    plan.read_back(&at);

    let (_, calls) = plan.measured(&["project", "copy", "notes:P", "--to", "board", "--json"]);
    never_walks("a cached project copy", &calls);
    let named = named_by_item_reads(&calls);
    for n in 1..=TASKS {
        assert!(
            !named.contains(&at[&format!("T-{n}")]),
            "T-{n} was read though its cache entry is at its token: {:?}",
            item_reads(&calls)
        );
    }
    assert_eq!(
        item_reads(&calls),
        [(
            graphql::ISSUE_COPY_DETAILS.to_owned(),
            std::collections::BTreeSet::from([at["P"].clone()])
        )],
        "the project, alone, in one batch"
    );
    let (_, calls) = plan.measured(&["document", "copy", "notes:D", "--to", "board", "--json"]);
    assert!(
        !named_by_item_reads(&calls).contains(&at["D"]),
        "the document was read though its cache entry is at its token: {:?}",
        item_reads(&calls)
    );
    holds_the_plan(&plan, &at, None);
}

#[test]
fn a_recopy_reads_only_what_neither_token_nor_cache_answers_and_keeps_a_persons_edit() {
    // (c): the cache holds the plan at its tokens; then a person fills in a board field of
    // their own on T-3's card, which moves its token past its entry, and the plan gains a ninth
    // task never copied. The copy reads T-3 — before it writes it — and of the plan's items
    // only it and the project, whose edges no token covers; creates the ninth; and leaves T-3
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
    let unanswered = std::collections::BTreeSet::from([at["P"].clone(), edited.clone()]);
    assert_eq!(
        named, unanswered,
        "only the project and the task edited since were read"
    );
    let read_at = calls
        .iter()
        .position(|call| named_by_item_reads(std::slice::from_ref(call)).contains(edited))
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
fn a_recopy_whose_pinning_listing_is_refused_reads_what_it_would_have_pinned_and_still_lands() {
    // The listing that would pin every member's token is refused once: nothing is pinned, so
    // every counterpart is read as if there were no cache — in one batch — and the copy lands
    // the plan exactly as it would have. The orphan check's own read of the members is made
    // after it, and answered.
    let plan = Plan::configured(SMALL, Vec::new(), Some(json!({"enabled": true})));
    let (project, document, _) = plan.copy();
    let at = counterparts(&project, &document);
    plan.read_back(&at);
    plan.github.refuse_once("subIssues(first:$first");
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
        "the refused pinning listing, then the orphan check's own: {:?}",
        tally(&calls)
    );
    holds_the_plan(&plan, &at, None);
}
