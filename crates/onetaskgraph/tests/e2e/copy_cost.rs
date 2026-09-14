//! What a `project copy` into a GitHub board costs, measured against the loopback fixture
//! board the copy journeys already drive.
//!
//! The record this holds the copies to is
//! `crates/onetaskgraph-github-projects/tests/fixtures/copy-cost.txt`, beside the live
//! journey's own `session-cost.txt`, and it measures the same two quantities that file does
//! — requests, and each GraphQL request's worst-case node count under the variables it
//! really sent — in the same per-call shape. `session-cost.md` is where the before and
//! after are written down.
//!
//! It lives in this crate rather than beside its record because a copy is the engine's,
//! and no plugin crate may depend on the engine at any depth. What it measures is still the
//! plugin's own inventory: every request the board served is named and counted through
//! `onetaskgraph_github_projects::accounting`, the same calculation that crate's session
//! record is taken with, so the two records cannot count one document two ways.

use std::path::PathBuf;

use onetaskgraph_github_projects::accounting::{Accounting, Call, RateLimit, Request, Session};
use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{GitHubBoardFields, document, github_projects_with_board};

/// The checked-in record the copies below are held to.
const RECORD: &str =
    include_str!("../../../onetaskgraph-github-projects/tests/fixtures/copy-cost.txt");

/// One Markdown project and its tasks, beside the fixture board it is copied into.
struct Plan {
    sandbox: Sandbox,
    board: GitHubBoardFields,
    root: PathBuf,
    tasks: usize,
}

impl Plan {
    /// A project of `tasks` tasks, none of which the board holds yet.
    fn of(tasks: usize) -> Self {
        let sandbox = Sandbox::new();
        let root = sandbox.subdirectory("plans");
        std::fs::create_dir_all(root.join("projects")).expect("the project folder");
        std::fs::create_dir_all(root.join("tasks")).expect("the task folder");
        let (config, board) = github_projects_with_board(&sandbox);
        // Pacing is already off on this board's configuration: it spaces this source's own
        // mutations in time and changes nothing about how many it sends.
        sandbox.project_document(&document(&json!({
            "plans": {"plugin":"local-md","config":{
                "root": root,
                "status_mapping": {"Todo":"todo","Doing":"in-progress"}}},
            "board": {"plugin":"github-projects","config":config}
        })));
        let plan = Self {
            sandbox,
            board,
            root,
            tasks,
        };
        plan.author(&[], None);
        plan
    }

    /// Write the project and every task, each recording the origin `origins` gives it.
    ///
    /// That is the shape `onepipeline`'s write-back authors: a shadow project whose every
    /// task the destination already holds names that destination task at
    /// `onetaskgraph.origin`, and whose project names the destination project.
    fn author(&self, origins: &[(String, Value)], doing: Option<usize>) {
        let origin_of = |id: &str| {
            origins
                .iter()
                .find(|(source, _)| source == &format!("plans:{id}"))
                .and_then(|(_, destination)| destination.as_str())
                .map_or_else(String::new, |destination| {
                    format!("metadata: {{onetaskgraph.origin: \"{destination}\"}}\n")
                })
        };
        std::fs::write(
            self.root.join("projects/P.md"),
            format!(
                "---\ntitle: Measured plan\nstatus: Doing\n{}---\nthe plan\n",
                origin_of("P")
            ),
        )
        .expect("the project");
        for step in 0..self.tasks {
            let id = format!("T-{step}");
            let status = if doing == Some(step) { "Doing" } else { "Todo" };
            std::fs::write(
                self.root.join(format!("tasks/{id}.md")),
                format!(
                    "---\ntitle: Step {step}\nstatus: {status}\nproject: P\n{}---\nstep {step}\n",
                    origin_of(&id)
                ),
            )
            .expect("a task");
        }
    }

    /// One copy command, the requests the board served for it, and what it reported.
    fn copy(&self, extra: &[&str]) -> (Session, Vec<(String, Value)>, Value) {
        let before = self.board.served().len();
        let mut arguments = vec!["project", "copy", "plans:P", "--to", "board", "--json"];
        arguments.extend_from_slice(extra);
        let output = self
            .sandbox
            .command()
            .args(&arguments)
            .assert()
            .get_output()
            .clone();
        assert_eq!(
            output.status.code(),
            Some(0),
            "`onetaskgraph {}` failed\n{}",
            arguments.join(" "),
            stderr(&output)
        );
        let served = self.board.served()[before..].to_vec();
        let ledger = Accounting::new();
        for (query, variables) in &served {
            ledger.record(
                Request::graphql(query, variables, None, None).answered(RateLimit::default()),
            );
        }
        let report = serde_json::from_str(&stdout(&output)).expect("a copy emits JSON");
        (ledger.snapshot(), served, report)
    }
}

/// Every item of a copy report, as the source id and the destination id it landed on.
fn landed(report: &Value) -> Vec<(String, Value)> {
    report["items"]
        .as_array()
        .expect("a copy report carries items")
        .iter()
        .map(|item| {
            (
                item["source"].as_str().expect("a source id").to_owned(),
                item["destination"].clone(),
            )
        })
        .collect()
}

/// One copy's cost, in the record's own per-call shape.
///
/// The same rendering the live journey's `session-cost.txt` is written in, so one reader
/// reads both: the name is the document's own from `graphql::DOCUMENTS`, never anything a
/// board held.
fn rendered(heading: &str, session: &Session) -> String {
    let mut calls: std::collections::BTreeMap<&str, (usize, u64)> =
        std::collections::BTreeMap::new();
    for request in session.requests() {
        let entry = calls.entry(request.name()).or_default();
        entry.0 += 1;
        entry.1 += match request.call() {
            Call::Document { node_count, .. } => node_count.unwrap_or_default(),
            Call::Endpoint { .. } => 0,
        };
    }
    let mut out = format!(
        "{heading}\nrequests {}\nnode count {}\n\nrequests per call\n",
        session.total_requests(),
        session.total_node_count()
    );
    for (name, (requests, nodes)) in calls {
        out.push_str(&format!("  {requests:>3}  {nodes:>6}  {name}\n"));
    }
    out
}

/// What the same two whole copies sent before a command resolved each item's target once,
/// measured on this harness against the engine as it stood at onetaskgraph 0.2.28.
const BEFORE: (usize, usize) = (69, 47);

/// The documents a member copy may send: reads and writes of one issue and its board item,
/// and nothing that walks the board, searches its issues or reads a project's sub-issues.
///
/// Named by the document itself from `graphql::DOCUMENTS` rather than by the label a
/// record prints, so renaming a row cannot let a board walk through.
fn per_node(document: &str) -> bool {
    use onetaskgraph_github_projects::graphql;
    [
        graphql::ISSUE,
        graphql::ISSUE_BOARD_ITEMS,
        graphql::ISSUE_DEPENDENCIES,
        graphql::UPDATE_ISSUE,
        graphql::UPDATE_DRAFT,
        graphql::UPDATE_FIELD,
        graphql::ADD_SUB_ISSUE,
        graphql::REMOVE_SUB_ISSUE,
        graphql::ADD_BLOCKED_BY,
        graphql::REMOVE_BLOCKED_BY,
    ]
    .contains(&document)
}

/// Whether a document reads one issue by the node id its `id` variable carries.
fn reads_one_issue(document: &str) -> bool {
    use onetaskgraph_github_projects::graphql;
    [
        graphql::ISSUE,
        graphql::ISSUE_BOARD_ITEMS,
        graphql::ISSUE_DEPENDENCIES,
    ]
    .contains(&document)
}

/// The native id a qualified destination id in a copy report names.
fn native(destination: &Value) -> String {
    destination
        .as_str()
        .and_then(|id| id.strip_prefix("board:"))
        .unwrap_or_else(|| panic!("a board id: {destination}"))
        .to_owned()
}

/// A plan of `tasks` tasks the board already holds, each recording its origin, with the
/// task at `changed` moved to `Doing` — and the one-member copy naming it.
fn one_member_copy(tasks: usize, changed: usize) -> (Session, Vec<(String, Value)>, Value) {
    let plan = Plan::of(tasks);
    let (_, _, first) = plan.copy(&[]);
    plan.author(&landed(&first), Some(changed));
    let member = format!("plans:T-{changed}");
    plan.copy(&["--member", &member])
}

#[test]
fn a_project_copy_into_a_board_costs_what_the_record_beside_the_session_record_says() {
    let ten = Plan::of(10);
    let (whole, _, first) = ten.copy(&[]);
    // Every destination id the first copy landed on, recorded back into the shadow the
    // way the consumer's write-back records them.
    let origins = landed(&first);
    ten.author(&origins, None);
    let (repeat, repeat_served, _) = ten.copy(&[]);
    let (of_ten, of_ten_served, of_ten_report) = one_member_copy(10, 3);
    let (of_three, of_three_served, _) = one_member_copy(3, 1);

    // (a) and (b) cost less than they did, and (b) — where every item already has a
    // counterpart — reads no issue by its node id twice.
    assert!(
        whole.total_requests() < BEFORE.0 && repeat.total_requests() < BEFORE.1,
        "the whole copies sent {} and {} requests, and before this change they sent {} and {}",
        whole.total_requests(),
        repeat.total_requests(),
        BEFORE.0,
        BEFORE.1
    );
    let mut read: Vec<&Value> = repeat_served
        .iter()
        .filter(|(document, _)| document == onetaskgraph_github_projects::graphql::ISSUE)
        .map(|(_, variables)| &variables["id"])
        .collect();
    let reads = read.len();
    read.sort_by_key(|id| id.to_string());
    read.dedup();
    assert_eq!(
        reads,
        read.len(),
        "(b) read one issue twice: {repeat_served:#?}"
    );

    // (c) and (d): the same number of requests, whatever the project holds besides the one
    // member named, and every one of them about one issue and its board item.
    assert_eq!(
        of_ten.total_requests(),
        of_three.total_requests(),
        "a one-member copy grew with the members it was not told about"
    );
    for (what, served) in [("(c)", &of_ten_served), ("(d)", &of_three_served)] {
        let walked: Vec<&(String, Value)> = served
            .iter()
            .filter(|(document, _)| !per_node(document))
            .collect();
        assert!(
            walked.is_empty(),
            "{what} sent a request that is not a read or a write of one issue: {walked:#?}"
        );
    }
    let reached: Vec<String> = of_ten_report["items"]
        .as_array()
        .expect("a copy report carries items")
        .iter()
        .map(|item| native(&item["destination"]))
        .collect();
    assert_eq!(
        of_ten_report["items"].as_array().map(|items| items
            .iter()
            .map(|item| item["action"].clone())
            .collect::<Vec<_>>()),
        Some(vec![json!("unchanged"), json!("updated")]),
        "{of_ten_report:#}"
    );
    for (document, variables) in &of_ten_served {
        if reads_one_issue(document) {
            let id = variables["id"].as_str().expect("a node id").to_owned();
            assert!(
                reached.contains(&id),
                "(c) read issue {id}, which is neither the project's nor the named member's \
                 ({reached:?})"
            );
        }
    }

    let measured = [
        rendered("(a) a whole copy of a project of 10 tasks", &whole),
        rendered(
            "(b) a repeat whole copy of the same project, unchanged, each item recording its origin",
            &repeat,
        ),
        rendered(
            "(c) a member copy naming 1 of those 10 tasks, its status changed, every task recording its origin",
            &of_ten,
        ),
        rendered(
            "(d) the same one-member copy of a project of 3 tasks",
            &of_three,
        ),
    ]
    .join("\n");
    assert_eq!(
        measured.trim(),
        RECORD.trim(),
        "a project copy no longer costs what \
         crates/onetaskgraph-github-projects/tests/fixtures/copy-cost.txt records; if the \
         change is deliberate, put the measurement above into that file and say in \
         session-cost.md what moved it"
    );
}
