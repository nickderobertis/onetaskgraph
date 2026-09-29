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

/// What one measured command came to: the requests the board served for it, as a session and
/// as the documents and variables themselves, and what the command reported.
type Measured = (Session, Vec<(String, Value)>, Value);

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
    fn copy(&self, extra: &[&str]) -> Measured {
        let mut arguments = vec!["project", "copy", "plans:P", "--to", "board", "--json"];
        arguments.extend_from_slice(extra);
        self.measure(&arguments)
    }

    /// One command, the requests the board served for it, and what it reported.
    fn measure(&self, arguments: &[&str]) -> Measured {
        let before = self.board.served().len();
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
        let report = serde_json::from_str(&stdout(&output)).expect("a command emits JSON");
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
fn one_member_copy(tasks: usize, changed: usize) -> Measured {
    let plan = Plan::of(tasks);
    let (_, _, first) = plan.copy(&[]);
    plan.author(&landed(&first), Some(changed));
    let member = format!("plans:T-{changed}");
    plan.copy(&["--member", &member])
}

/// How many mutations one measured command sent.
fn mutations(served: &[(String, Value)]) -> usize {
    served
        .iter()
        .filter(|(document, _)| document.trim_start().starts_with("mutation"))
        .count()
}

/// A plan of `tasks` tasks the board already holds, the task at `changed` holding a claim key
/// of the kind a settlement removes — and the two targeted updates of that one task: a
/// settlement naming its status and three keys set and one removed, then the same update
/// again, when the task already holds every value it names.
fn one_targeted_update(tasks: usize, changed: usize) -> [Measured; 2] {
    let plan = Plan::of(tasks);
    let (_, _, first) = plan.copy(&[]);
    let target = landed(&first)
        .into_iter()
        .find(|(source, _)| source == &format!("plans:T-{changed}"))
        .map(|(_, destination)| destination)
        .expect("the copy landed the task");
    let target = target
        .as_str()
        .expect("a qualified destination id")
        .to_owned();
    // Setup, not measured: the key a settlement removes, held as a claim would hold it.
    plan.measure(&[
        "task",
        "metadata",
        "set",
        &target,
        "onepipeline.claim",
        r#"{"run":"r-1"}"#,
        "--json",
    ]);
    let settlement = [
        "task",
        "update",
        &target,
        "--status",
        "in-progress",
        "--metadata",
        r#"onepipeline.settlement={"outcome":"landed","turns":3}"#,
        "--metadata",
        r#"onepipeline.landing="merged""#,
        "--metadata",
        r#"onepipeline.change_url="https://example.invalid/pull/7""#,
        "--remove-metadata",
        "onepipeline.claim",
        "--json",
    ];
    [plan.measure(&settlement), plan.measure(&settlement)]
}

/// Whether a document is the board's own whole read of its items.
fn reads_the_board(document: &str) -> bool {
    document == onetaskgraph_github_projects::graphql::BOARD
}

/// A task copy into a board that already holds the task's counterpart, written the way the
/// release before this one writes a copy — its origin in the board field and not in the
/// body — so the copy's second rule can only find it by the board's own field filter.
fn a_task_copy_finding_a_counterpart_written_before_this_release() -> Measured {
    let plan = Plan::of(1);
    let (_, _, first) = plan.copy(&[]);
    let (_, counterpart) = landed(&first)
        .into_iter()
        .find(|(source, _)| source == "plans:T-0")
        .expect("the first copy landed the task");
    let counterpart = native(&counterpart);
    plan.board.written_before_the_origin_mirror(&counterpart);
    assert_eq!(plan.board.origin(&counterpart), json!("plans:T-0"));
    let measured = plan.measure(&["task", "copy", "plans:T-0", "--to", "board", "--json"]);
    assert_eq!(
        measured.2["items"]
            .as_array()
            .expect("a copy report carries items")
            .iter()
            .map(|item| (item["action"].clone(), native(&item["destination"])))
            .collect::<Vec<_>>(),
        [(json!("unchanged"), counterpart)],
        "the copy found the counterpart rather than creating a second one: {:#}",
        measured.2
    );
    measured
}

/// A task copy into a board that holds no counterpart of it: a task filed in no project, so
/// the copy has no project to find either.
fn a_task_copy_finding_no_counterpart() -> Measured {
    let plan = Plan::of(0);
    std::fs::write(
        plan.root.join("tasks/L.md"),
        "---\ntitle: A loose step\nstatus: Todo\n---\na step in no project\n",
    )
    .expect("a task");
    let measured = plan.measure(&["task", "copy", "plans:L", "--to", "board", "--json"]);
    assert_eq!(
        measured.2["items"]
            .as_array()
            .expect("a copy report carries items")
            .iter()
            .map(|item| item["action"].clone())
            .collect::<Vec<_>>(),
        [json!("created")],
        "{:#}",
        measured.2
    );
    measured
}

/// Hold a task copy to its second rule's one question: the board is asked for the origin of
/// `task` once, and its items are never walked.
fn asks_for_the_origin_and_never_walks_the_board(
    what: &str,
    task: &str,
    served: &[(String, Value)],
) {
    let lookups = served
        .iter()
        .filter(|(document, _)| document == onetaskgraph_github_projects::graphql::ORIGIN_LOOKUP)
        .map(|(_, variables)| variables["filter"].clone())
        .collect::<Vec<_>>();
    assert_eq!(
        lookups,
        [json!(format!("onetaskgraph.origin:\"plans:{task}\""))],
        "{what} asks the board for that origin once: {served:#?}"
    );
    let whole: Vec<&(String, Value)> = served
        .iter()
        .filter(|(document, _)| reads_the_board(document))
        .collect();
    assert!(whole.is_empty(), "{what} read the whole board: {whole:#?}");
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

    // (e) and (f): the targeted update of one existing task of the same ten, as a settlement
    // writes it — its status, three keys set and one removed — and the same update when the
    // task already holds every value it names. (e) sends fewer requests than the member copy
    // (c) of one changed task and no more mutations; (f) sends no mutation at all. Neither
    // reads anything but the one issue, and neither writes its origin.
    let [
        (settled, settled_served, settled_report),
        (repeat_update, repeat_update_served, again),
    ] = one_targeted_update(10, 3);
    assert!(
        settled.total_requests() < of_ten.total_requests()
            && mutations(&settled_served) <= mutations(&of_ten_served),
        "the targeted update sent {} requests and {} mutations; the one-member copy (c) sent {} \
         and {}",
        settled.total_requests(),
        mutations(&settled_served),
        of_ten.total_requests(),
        mutations(&of_ten_served)
    );
    assert_eq!(
        settled_report["written"],
        json!(["status", "metadata"]),
        "{settled_report:#}"
    );
    assert_eq!(
        mutations(&repeat_update_served),
        0,
        "{repeat_update_served:#?}"
    );
    assert_eq!(again["written"], json!([]), "{again:#}");
    for (what, served) in [("(e)", &settled_served), ("(f)", &repeat_update_served)] {
        let issue_reads = served
            .iter()
            .filter(|(document, _)| reads_one_issue(document))
            .count();
        assert_eq!(
            issue_reads, 1,
            "{what} read the issue other than once: {served:#?}"
        );
        assert!(
            served.iter().all(|(document, variables)| per_node(document)
                && variables.pointer("/input/value/text").is_none()),
            "{what} sent a request that is not a read or a write of the one issue, or wrote \
             its origin: {served:#?}"
        );
    }

    // (g) and (h): a task copy finds its counterpart by asking the board for its origin, and
    // never by walking the board — one written the way the release before this one wrote it,
    // with its origin in the board field alone, included — and creates when there is none.
    let (found, found_served, _) = a_task_copy_finding_a_counterpart_written_before_this_release();
    asks_for_the_origin_and_never_walks_the_board("(g)", "T-0", &found_served);
    let (created, created_served, _) = a_task_copy_finding_no_counterpart();
    asks_for_the_origin_and_never_walks_the_board("(h)", "L", &created_served);

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
        rendered(
            "(e) a targeted update of 1 of those 10 tasks, naming its status, three metadata keys set and one removed",
            &settled,
        ),
        rendered(
            "(f) the same targeted update again, every value it names already held",
            &repeat_update,
        ),
        rendered(
            "(g) a task copy into a board holding its counterpart, its origin in the board field alone",
            &found,
        ),
        rendered("(h) a task copy into a board holding no counterpart", &created),
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
