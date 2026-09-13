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

#[test]
fn a_project_copy_into_a_board_costs_what_the_record_beside_the_session_record_says() {
    let ten = Plan::of(10);
    let (whole, _, first) = ten.copy(&[]);
    // Every destination id the first copy landed on, recorded back into the shadow the
    // way the consumer's write-back records them.
    let origins = landed(&first);
    ten.author(&origins, None);
    let (repeat, _, _) = ten.copy(&[]);

    let measured = [
        rendered("(a) a whole copy of a project of 10 tasks", &whole),
        rendered(
            "(b) a repeat whole copy of the same project, unchanged, each item recording its origin",
            &repeat,
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
