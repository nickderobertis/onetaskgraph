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

/// Where the record's figures are explained, one table of which this test holds as well.
const EXPLAINED: &str = include_str!("../../../onetaskgraph-github-projects/session-cost.md");

/// The protocol document, whose GitHub Projects cost table states the rows this change moved
/// or added and is held to the same figures as the plugin's own.
const PROTOCOL: &str = include_str!("../../../../docs/plugin-protocol.md");

/// The rows `docs/plugin-protocol.md` states.
const PROTOCOL_ROWS: [&str; 7] = [
    "new copy",
    "copy --create",
    "bound copy",
    "comment",
    "detail",
    "batched detail",
    "update",
];

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
        graphql::UPDATE_FIELDS,
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

/// A plan of `tasks` tasks the board holds, landed by one whole copy that recorded every
/// item's link, and two re-copies of the task at `changed` with its status changed: first by
/// the link the whole copy recorded, then — every link removed, as every item was before
/// there were links — by asking the board for the task's origin.
fn one_task_recopy(tasks: usize, changed: usize) -> [Measured; 2] {
    let plan = Plan::of(tasks);
    plan.copy(&[]);
    let task = format!("plans:T-{changed}");
    let recopy = ["task", "copy", task.as_str(), "--to", "board", "--json"];
    // `author` writes every file afresh, which drops the links the whole copy recorded, so
    // the status is moved by hand here and the links are kept.
    let path = plan.root.join(format!("tasks/T-{changed}.md"));
    let linked = std::fs::read_to_string(&path).expect("the task");
    assert!(
        linked.contains("onetaskgraph.copies"),
        "the whole copy recorded the task's link: {linked}"
    );
    std::fs::write(&path, linked.replace("status: Todo", "status: Doing")).expect("an edit");
    let by_link = plan.measure(&recopy);
    plan.author(&[], None);
    let by_origin = plan.measure(&recopy);
    [by_link, by_origin]
}

/// A task copy into a board that already holds the task's counterpart, written the way the
/// release before this one writes a copy — its origin in the board field and not in the
/// body, and no link on the task it was copied from — so the copy's second rule can only
/// find it by the board's own field filter.
fn a_task_copy_finding_a_counterpart_written_before_this_release() -> Measured {
    let plan = Plan::of(1);
    let (_, _, first) = plan.copy(&[]);
    // That release recorded no link either, and `author` writes every file afresh without one.
    plan.author(&[], None);
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
        .filter(|(document, _)| document == onetaskgraph_github_projects::graphql::BOARD)
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

    // (g) and (h): one task re-copied on its own. By its link, the copy reads its board item
    // and its project's by id and never walks or searches the board; without one, it asks
    // the board for the task's origin, and never walks it either.
    let [
        (by_link, by_link_served, by_link_report),
        (by_origin, by_origin_served, by_origin_report),
    ] = one_task_recopy(10, 3);
    assert_eq!(
        (
            &by_link_report["items"][0]["via"],
            &by_link_report["items"][0]["action"]
        ),
        (&json!("link"), &json!("updated")),
        "{by_link_report:#}"
    );
    assert_eq!(
        (
            &by_origin_report["items"][0]["via"],
            &by_origin_report["items"][0]["action"]
        ),
        (&json!("scan"), &json!("updated")),
        "{by_origin_report:#}"
    );
    {
        use onetaskgraph_github_projects::graphql;
        let walked: Vec<&(String, Value)> = by_link_served
            .iter()
            .filter(|(document, _)| {
                document == graphql::BOARD || document == graphql::SEARCH_ISSUES
            })
            .collect();
        assert!(
            walked.is_empty(),
            "(g) read the board or searched its issues: {walked:#?}"
        );
    }
    asks_for_the_origin_and_never_walks_the_board("(h)", "T-3", &by_origin_served);
    assert!(
        by_link.total_node_count() < by_origin.total_node_count(),
        "(g) cost {} worst-case nodes and (h) {}",
        by_link.total_node_count(),
        by_origin.total_node_count()
    );

    // The comparison `session-cost.md` draws between the two is the same two figures, so it
    // is held to them here rather than trusted to be kept in step by hand.
    for (row, searching, linked) in [
        (
            "**requests**",
            by_origin.total_requests(),
            by_link.total_requests(),
        ),
        (
            "**node count**",
            usize::try_from(by_origin.total_node_count()).expect("a node count fits"),
            usize::try_from(by_link.total_node_count()).expect("a node count fits"),
        ),
    ] {
        let line = format!("| {row:<18} | {searching:>22} | {linked:>21} |");
        assert!(
            EXPLAINED.contains(&line),
            "session-cost.md's table of (h) against (g) no longer says what they cost; its \
             row should read:\n{line}"
        );
    }

    // (i) and (j): a task copy finds its counterpart by asking the board for its origin, and
    // never by walking the board — one written the way the release before this one wrote it,
    // with its origin in the board field alone, included — and creates when there is none.
    // llmlint: ignore-block[expensive_tests_stay_behind_their_own_edge] Two more rows of a record this test already takes, against the same loopback fixture board every journey in this crate drives: no credential, no socket beyond 127.0.0.1, no third party, and well under a second for the whole test. A copy is the engine's, so this lives in the binary crate for the reason the module documentation gives, and it is held to `copy-cost.txt`, which the board's own crate records beside its session cost.
    let (found, found_served, _) = a_task_copy_finding_a_counterpart_written_before_this_release();
    asks_for_the_origin_and_never_walks_the_board("(i)", "T-0", &found_served);
    let (created, created_served, _) = a_task_copy_finding_no_counterpart();
    asks_for_the_origin_and_never_walks_the_board("(j)", "L", &created_served);
    // llmlint: ignore-end[expensive_tests_stay_behind_their_own_edge]

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
            "(g) a task copy of 1 of those 10 tasks, its status changed, found by the link the whole copy recorded",
            &by_link,
        ),
        rendered(
            "(h) the same task copy with every link removed, found by asking the board for its origin",
            &by_origin,
        ),
        rendered(
            "(i) a task copy into a board holding its counterpart, its origin in the board field alone",
            &found,
        ),
        rendered("(j) a task copy into a board holding no counterpart", &created),
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

// llmlint: ignore[expensive_tests_stay_behind_their_own_edge] This real CLI paging regression uses the existing loopback board and local Markdown only: both copy_cost tests together took 0.70 seconds. It adds no service session or toolchain; the binary crate owns CLI process resumption, which a plugin test cannot drive without the forbidden engine dependency.
#[test]
fn a_narrowed_cli_list_limits_fetches_and_resumes_in_a_new_process() {
    let plan = Plan::of(130);
    for step in 0..130 {
        let path = plan.root.join(format!("tasks/T-{step}.md"));
        let contents = std::fs::read_to_string(&path).unwrap();
        std::fs::write(
            path,
            contents.replace("project: P\n", "project: P\nmetadata: {team.owner: ada}\n"),
        )
        .unwrap();
    }
    plan.copy(&[]);
    for predicate in [["--search", "Step"], ["--metadata", "team.owner=ada"]] {
        let mut ids = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let mut args = vec![
                "task", "list", "--source", "board", "--limit", "20", "--json",
            ];
            args.extend(predicate);
            if let Some(ref token) = token {
                args.extend(["--page", token]);
            }
            let (_, served, report) = plan.measure(&args);
            let searches = served
                .iter()
                .filter(|(query, _)| query == onetaskgraph_github_projects::graphql::SEARCH_ISSUES)
                .collect::<Vec<_>>();
            assert_eq!(
                searches.len(),
                1,
                "each CLI process fetches exactly one needed page"
            );
            assert_eq!(searches[0].1["first"], json!(20));
            ids.extend(
                report["items"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|item| item["id"].clone()),
            );
            token = report["next"].as_str().map(str::to_owned);
            if token.is_none() {
                break;
            }
        }
        assert_eq!(ids.len(), 130);
        let mut whole = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let mut args = vec![
                "task", "list", "--source", "board", "--limit", "100", "--json",
            ];
            args.extend(predicate);
            if let Some(ref token) = token {
                args.extend(["--page", token]);
            }
            let (_, _, report) = plan.measure(&args);
            whole.extend(
                report["items"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|item| item["id"].clone()),
            );
            token = report["next"].as_str().map(str::to_owned);
            if token.is_none() {
                break;
            }
        }
        assert_eq!(
            ids, whole,
            "resuming CLI processes return the whole walk's order"
        );
        let output = plan
            .sandbox
            .command()
            .args([
                "task",
                "list",
                "--source",
                "board",
                predicate[0],
                predicate[1],
                "--page",
                "invalid",
            ])
            .assert()
            .get_output()
            .clone();
        assert_ne!(output.status.code(), Some(0));
    }
}

#[test]
fn detail_and_record_only_reads_reuse_one_issue_resolution_through_cli_and_sdks() {
    use onetaskgraph_github_projects::graphql;
    let plan = Plan::of(1);
    let (_, _, copied) = plan.copy(&[]);
    let id = landed(&copied)
        .into_iter()
        .find(|(source, _)| source == "plans:T-0")
        .unwrap()
        .1
        .as_str()
        .unwrap()
        .to_owned();
    for comments in [true, false] {
        let mut arguments = vec!["task", "show", &id, "--json"];
        if !comments {
            arguments.push("--no-comments");
        }
        let (_, sent, shown) = plan.measure(&arguments);
        // One request either way: the item with its first page of comments, or the item.
        let read = if comments {
            graphql::ISSUE_DETAIL
        } else {
            graphql::ISSUE
        };
        assert_eq!(sent.len(), 1, "{sent:#?}");
        assert_eq!(sent[0].0, read, "{sent:#?}");
        assert_eq!(shown.get("comments").is_some(), comments);
    }
    plan.sandbox
        .command()
        .args(["task", "comment", "add", &id])
        .write_stdin("Human-visible evidence")
        .assert()
        .success();
    let full = plan
        .sandbox
        .command()
        .args(["task", "show", &id])
        .assert()
        .success()
        .get_output()
        .clone();
    assert!(stdout(&full).contains("Human-visible evidence"));
    let before = plan.board.served().len();
    let record = plan
        .sandbox
        .command()
        .args(["task", "show", &id, "--no-comments"])
        .assert()
        .success()
        .get_output()
        .clone();
    assert!(stdout(&record).contains("Step 0"));
    assert!(!stdout(&record).contains("Human-visible evidence"));
    assert_eq!(plan.board.served().len() - before, 1);
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    for typescript in [false, true] {
        for comments in [true, false] {
            let before = plan.board.served().len();
            let mut command = if typescript {
                let mut command = std::process::Command::new("bun");
                command.args(["-e", &format!(
                    "import {{ OnetaskgraphClient }} from {}; const client = new OnetaskgraphClient({{binaryPath: process.env.DISPATCH_BINARY,cwd:process.cwd()}}); const id = process.env.DISPATCH_ITEM; if (id === undefined) throw new Error('missing fixture item'); const result = await client.taskShow(id, {{noComments:{}}}); console.log(JSON.stringify(result)); try {{ await client.taskShow('board:missing', {{noComments:true}}); throw new Error('missing task passed'); }} catch (error) {{ if (!(error instanceof Error) || !error.message.includes('no task')) throw error; }}",
                    serde_json::to_string(&workspace.join("sdks/typescript/src/client.ts")).unwrap(), !comments,
                )]);
                command
            } else {
                let mut command = std::process::Command::new("uv");
                command.args(["run", "--frozen", "--project"])
                    .arg(workspace.join("sdks/python"))
                    .args(["python", "-c", &format!(
                        "import asyncio,os,json\nfrom onetaskgraph_sdk import Client,OnetaskgraphError\nasync def run():\n c=Client(os.environ['DISPATCH_BINARY'],cwd=os.getcwd())\n result=await c.task_show(id=os.environ['DISPATCH_ITEM'],no_comments={})\n print(result.model_dump_json(exclude_none=True))\n try:\n  await c.task_show(id='board:missing',no_comments=True)\n except OnetaskgraphError:\n  pass\n else:\n  raise AssertionError('missing task passed')\nasyncio.run(run())", if comments { "False" } else { "True" },
                    )]);
                command
            };
            for (key, _) in std::env::vars() {
                if key.starts_with("ONETASKGRAPH_") {
                    command.env_remove(key);
                }
            }
            let output = command
                .current_dir(plan.sandbox.project())
                .env("XDG_CONFIG_HOME", plan.sandbox.config_home())
                .env_remove("HOME")
                .env("DISPATCH_BINARY", env!("CARGO_BIN_EXE_onetaskgraph"))
                .env("DISPATCH_ITEM", &id)
                .output()
                .unwrap();
            assert!(output.status.success(), "{}", stderr(&output));
            let shown: Value = serde_json::from_str(&stdout(&output)).unwrap();
            assert_eq!(shown.get("comments").is_some(), comments);
            let served = plan.board.served();
            let sent = &served[before..];
            // One successful detail/record read and one missing-item failure, each one request.
            let read = if comments {
                graphql::ISSUE_DETAIL
            } else {
                graphql::ISSUE
            };
            assert_eq!(
                sent.iter()
                    .map(|(query, _)| query.as_str())
                    .collect::<Vec<_>>(),
                [read, graphql::ISSUE],
                "{sent:#?}"
            );
        }
    }
}

#[test]
fn follow_up_writes_resolve_each_item_once_and_batch_the_copy_fields() {
    use onetaskgraph_github_projects::graphql;
    let plan = Plan::of(1);
    let path = plan.root.join("tasks/T-0.md");
    let body = std::fs::read_to_string(&path)
        .unwrap()
        .replace("project: P\n", "priority: urgent\n");
    std::fs::write(&path, body).unwrap();
    let (_, new_calls, report) =
        plan.measure(&["task", "copy", "plans:T-0", "--to", "board", "--json"]);
    std::fs::write(
        plan.root.join("tasks/T-1.md"),
        "---\ntitle: Asserted new\nstatus: Todo\npriority: high\n---\nnew\n",
    )
    .unwrap();
    let (_, create_calls, _) = plan.measure(&[
        "task",
        "copy",
        "plans:T-1",
        "--to",
        "board",
        "--create",
        "--json",
    ]);
    let id = report["items"][0]["destination"].as_str().unwrap();
    std::fs::write(&path, format!("---\ntitle: Revised ticket\nstatus: Doing\npriority: high\nmetadata: {{onetaskgraph.origin: {id}, myapp.owner: ada}}\n---\nRevised body.\n")).unwrap();
    let (_, bound_calls, _) =
        plan.measure(&["task", "copy", "plans:T-0", "--to", "board", "--json"]);
    let evidence_file = plan.root.join("evidence.txt");
    std::fs::write(&evidence_file, "Evidence").unwrap();
    let evidence_path = evidence_file.to_str().unwrap();
    let content_file = plan.root.join("content.txt");
    std::fs::write(&content_file, "Final body").unwrap();
    let content_path = content_file.to_str().unwrap();
    let (_, comment_calls, _) = plan.measure(&[
        "task",
        "comment",
        "add",
        id,
        "--body-file",
        evidence_path,
        "--json",
    ]);
    let (_, recount_calls, _) = plan.measure(&["task", "show", id, "--json"]);
    let (_, detail_calls, _) = plan.measure(&["task", "comment", "list", id, "--json"]);
    let (_, status_calls, _) = plan.measure(&["task", "status", "set", id, "todo", "--json"]);
    let (_, priority_calls, _) = plan.measure(&["task", "priority", "set", id, "medium", "--json"]);
    let (_, content_calls, _) = plan.measure(&[
        "task",
        "content",
        "set",
        id,
        "--file",
        content_path,
        "--json",
    ]);
    let update_body = plan.root.join("update.txt");
    std::fs::write(&update_body, "Updated body").unwrap();
    let (_, update_calls, updated) = plan.measure(&[
        "task",
        "update",
        id,
        "--title",
        "Updated ticket",
        "--body-file",
        update_body.to_str().unwrap(),
        "--status",
        "in-progress",
        "--priority",
        "low",
        "--metadata",
        "myapp.reviewer=\"lin\"",
        "--json",
    ]);
    assert_eq!(
        updated["written"],
        json!(["title", "content", "status", "priority", "metadata"]),
        "{updated}"
    );
    let (_, metadata_calls, _) = plan.measure(&[
        "task",
        "metadata",
        "set",
        id,
        "myapp.owner",
        "\"grace\"",
        "--json",
    ]);
    for (verb, sent, expected) in [
        ("new copy", &new_calls, 5),
        ("copy --create", &create_calls, 4),
        ("bound copy", &bound_calls, 3),
        ("comment", &comment_calls, 2),
        ("recount", &recount_calls, 1),
        ("detail", &detail_calls, 1),
        ("status", &status_calls, 2),
        ("priority", &priority_calls, 2),
        ("content", &content_calls, 2),
        ("metadata", &metadata_calls, 2),
        ("update", &update_calls, 3),
    ] {
        let mut reads = std::collections::BTreeMap::<String, usize>::new();
        for (_, variables) in sent
            .iter()
            .filter(|(query, _)| query == graphql::ISSUE || query == graphql::ISSUE_DETAIL)
        {
            *reads
                .entry(variables["id"].as_str().unwrap().to_owned())
                .or_default() += 1;
        }
        assert!(reads.values().all(|count| *count <= 1), "{verb}: {sent:#?}");
        let points: u64 = sent
            .iter()
            .map(|(document, _)| {
                onetaskgraph_github_projects::worst_case_point_cost(document).unwrap()
            })
            .sum();
        assert_eq!(sent.len(), expected, "{verb}: {sent:#?}");
        assert_eq!(points, expected as u64, "{verb}");
        assert!(
            include_str!("../../../onetaskgraph-github-projects/src/lib.rs")
                .contains(&format!("//! | {verb} | {expected} |")),
            "cost table: {verb}"
        );
        if PROTOCOL_ROWS.contains(&verb) {
            assert!(
                PROTOCOL.contains(&format!("\n| {verb} | {expected} |")),
                "docs/plugin-protocol.md cost table: {verb}"
            );
        }
        println!("{verb}: {} requests, {points} declared points", sent.len());
    }
    // The board's fields and the repository's id in one read — so no BOARD_FIELDS or
    // REPOSITORY — then the issue created on no board and filed with ADD_TO_BOARD, because
    // GitHub answers a create naming the board in `projectV2Ids` with no item and refuses the
    // filing that then follows; `--create` is the same without the origin lookup.
    fn documents(sent: &[(String, Value)]) -> Vec<&str> {
        sent.iter().map(|(query, _)| query.as_str()).collect()
    }
    assert_eq!(
        documents(&new_calls),
        [
            graphql::ORIGIN_LOOKUP,
            graphql::CREATION_CONTEXT,
            graphql::CREATE_ISSUE,
            graphql::ADD_TO_BOARD,
            graphql::UPDATE_FIELDS
        ]
    );
    assert_eq!(
        documents(&create_calls),
        [
            graphql::CREATION_CONTEXT,
            graphql::CREATE_ISSUE,
            graphql::ADD_TO_BOARD,
            graphql::UPDATE_FIELDS
        ]
    );
    for sent in [&new_calls, &create_calls] {
        let (_, created) = sent
            .iter()
            .find(|(query, _)| query == graphql::CREATE_ISSUE)
            .unwrap();
        assert_eq!(created["input"].get("projectV2Ids"), None);
    }
    assert_eq!(
        bound_calls
            .iter()
            .filter(|(query, _)| query == graphql::UPDATE_FIELDS)
            .count(),
        1
    );
    assert!(
        !new_calls
            .iter()
            .chain(&bound_calls)
            .any(|(query, _)| query == graphql::UPDATE_FIELD)
    );
    let (_, _, shown) = plan.measure(&["task", "show", id, "--no-comments", "--json"]);
    assert_eq!(shown["items"][0]["item"]["content"], "Updated body");
    assert_eq!(shown["items"][0]["item"]["title"], "Updated ticket");
    assert_eq!(shown["items"][0]["item"]["priority"], "low");
    assert_eq!(
        shown["items"][0]["item"]["status"]["category"],
        "in-progress"
    );
    assert_eq!(
        shown["items"][0]["item"]["metadata"]["myapp.reviewer"],
        "lin"
    );
    assert_eq!(
        shown["items"][0]["item"]["metadata"]["myapp.owner"],
        "grace"
    );
}

/// The cost table's `batched detail` row: `task show-many` of `n` tasks, each with its first
/// page of comments, is `ceil(n / DETAIL_BATCH)` requests and as many declared points.
#[test]
fn a_batched_detail_read_costs_one_request_and_one_point_per_detail_batch() {
    use onetaskgraph_github_projects::{DETAIL_BATCH, graphql};
    let plan = Plan::of(DETAIL_BATCH + 1);
    let (_, _, copied) = plan.copy(&[]);
    let tasks: Vec<String> = landed(&copied)
        .into_iter()
        .filter(|(source, _)| source.starts_with("plans:T-"))
        .map(|(_, destination)| destination.as_str().unwrap().to_owned())
        .collect();
    assert_eq!(tasks.len(), DETAIL_BATCH + 1);
    for n in [1, 3, DETAIL_BATCH, DETAIL_BATCH + 1] {
        let mut arguments = vec!["task", "show-many"];
        arguments.extend(tasks[..n].iter().map(String::as_str));
        arguments.push("--json");
        let (_, sent, report) = plan.measure(&arguments);
        let expected = n.div_ceil(DETAIL_BATCH);
        let points: u64 = sent
            .iter()
            .map(|(document, _)| {
                onetaskgraph_github_projects::worst_case_point_cost(document).unwrap()
            })
            .sum();
        assert_eq!(sent.len(), expected, "{n} items: {sent:#?}");
        assert_eq!(points, expected as u64, "{n} items");
        // One item is the one-item detail read; more are the batch.
        let read = if n == 1 {
            graphql::ISSUE_DETAIL
        } else {
            graphql::ISSUE_DETAILS
        };
        assert!(
            sent.iter().all(|(document, _)| document == read),
            "{sent:#?}"
        );
        assert_eq!(report["details"].as_array().map(Vec::len), Some(n));
    }
    assert!(
        include_str!("../../../onetaskgraph-github-projects/src/lib.rs")
            .contains("//! | batched detail | ceil(n / DETAIL_BATCH) |"),
        "cost table: batched detail"
    );
    assert!(
        PROTOCOL.contains("\n| batched detail | ceil(n / DETAIL_BATCH) |")
            && PROTOCOL.contains(&format!("`DETAIL_BATCH` is {DETAIL_BATCH}")),
        "docs/plugin-protocol.md cost table: batched detail"
    );
    assert!(
        include_str!("../../../onetaskgraph-github-projects/src/lib.rs")
            .contains(&format!("pub const DETAIL_BATCH: usize = {DETAIL_BATCH};")),
        "cost table: DETAIL_BATCH"
    );
}

/// `task copy --create`: each task is created without the correspondence lookup — no
/// `ORIGIN_LOOKUP` and no other read of the board's index — a later read in the same
/// process finds what it created, and every refusal it owes names what to do instead.
#[test]
fn a_create_copy_sends_no_origin_lookup_and_refuses_what_it_cannot_assert() {
    use onetaskgraph_github_projects::graphql;
    let plan = Plan::of(0);
    std::fs::write(
        plan.root.join("tasks/A.md"),
        "---\ntitle: First step\nstatus: Todo\n---\nfirst\n",
    )
    .unwrap();
    std::fs::write(
        plan.root.join("tasks/B.md"),
        "---\ntitle: Second step\nstatus: Doing\ndepends_on: [A]\n---\nsecond\n",
    )
    .unwrap();

    let (_, sent, report) = plan.measure(&[
        "task", "copy", "plans:A", "plans:B", "--to", "board", "--create", "--json",
    ]);
    let landed = landed(&report);
    assert_eq!(landed.len(), 2, "{report}");
    for item in report["items"].as_array().unwrap() {
        assert_eq!(item["action"], "created", "{report}");
    }
    assert!(
        !sent.iter().any(|(document, _)| [
            graphql::ORIGIN_LOOKUP,
            graphql::SEARCH_ISSUES,
            graphql::BOARD
        ]
        .contains(&document.as_str())),
        "--create looks for nothing: {sent:#?}"
    );
    // The second task's dependency names the first by the id this same command created it
    // under, read from this process's own record of what it wrote.
    let first = native(&landed[0].1);
    assert!(
        sent.iter()
            .any(|(document, variables)| document == graphql::ADD_BLOCKED_BY
                && variables["input"]["blockingIssueId"] == first.as_str()),
        "{sent:#?}"
    );
    assert!(
        !sent
            .iter()
            .any(|(document, variables)| reads_one_issue(document)
                && variables["id"] == first.as_str()),
        "the item it created is answered from its own record: {sent:#?}"
    );

    // Beside a way of looking, refused before the board is asked anything.
    for flag in [&["--match-by", "title"][..], &["--recreate"][..]] {
        let before = plan.board.served().len();
        let mut arguments = vec!["task", "copy", "plans:A", "--to", "board", "--create"];
        arguments.extend_from_slice(flag);
        let output = plan
            .sandbox
            .command()
            .args(&arguments)
            .assert()
            .get_output()
            .clone();
        let said = stderr(&output);
        assert_eq!(output.status.code(), Some(1), "{said}");
        assert!(said.contains(flag[0]) && said.contains("next:"), "{said}");
        assert_eq!(plan.board.served().len(), before, "nothing was sent");
    }

    // The copy recorded where each landed, so a second `--create` of the first is refused,
    // naming the item its link already names, and writes nothing.
    let before = plan.board.served().len();
    let output = plan
        .sandbox
        .command()
        .args(["task", "copy", "plans:A", "--to", "board", "--create"])
        .assert()
        .get_output()
        .clone();
    let said = stderr(&output);
    assert_eq!(output.status.code(), Some(1), "{said}");
    assert!(
        said.contains(landed[0].1.as_str().unwrap()) && said.contains("next:"),
        "{said}"
    );
    assert_eq!(mutations(&plan.board.served()[before..]), 0);

    // Without `--create` the same copy follows its link and updates that item, as it always
    // did.
    let (_, _, again) = plan.measure(&["task", "copy", "plans:A", "--to", "board", "--json"]);
    assert_eq!(again["items"][0]["destination"], landed[0].1, "{again}");
    assert_ne!(again["items"][0]["action"], "created", "{again}");
}

/// The two additions the cost table states to a bound re-copy's 3 requests: a task filed
/// under a project adds the one read that confirms the project's link, and the dependencies a
/// re-copy newly names are read together, `ceil(n / DETAIL_BATCH)` requests for `n` of them,
/// beside the one `addBlockedBy` each new edge is.
#[test]
fn a_bound_recopy_adds_one_project_read_and_batches_the_dependencies_it_newly_names() {
    use onetaskgraph_github_projects::{DETAIL_BATCH, graphql};
    let plan = Plan::of(DETAIL_BATCH + 2);
    let (_, _, first) = plan.copy(&[]);
    let landed = landed(&first);
    let on_board = |task: usize| {
        landed
            .iter()
            .find(|(source, _)| source == &format!("plans:T-{task}"))
            .map(|(_, destination)| destination.as_str().unwrap().to_owned())
            .unwrap()
    };
    let path = plan.root.join("tasks/T-0.md");
    let linked = std::fs::read_to_string(&path).unwrap();
    let recopy = ["task", "copy", "plans:T-0", "--to", "board", "--json"];
    let edit = |status: &str, depends_on: &[usize]| {
        let names = depends_on
            .iter()
            .map(|task| format!("{{id: \"{}\", item: task}}", on_board(*task)))
            .collect::<Vec<_>>()
            .join(", ");
        let dependencies = if depends_on.is_empty() {
            String::new()
        } else {
            format!("\ndepends_on: [{names}]")
        };
        std::fs::write(
            &path,
            linked.replace("status: Todo", &format!("status: {status}{dependencies}")),
        )
        .unwrap();
    };
    edit("Doing", &[]);
    let (_, under_project, _) = plan.measure(&recopy);
    // Three newly named dependencies are one batch, and one more than a batch holds is two.
    let few: Vec<usize> = (1..=3).collect();
    edit("Todo", &few);
    let (_, newly_few, _) = plan.measure(&recopy);
    // Setup, not measured: the three released again, so every one below is newly named.
    edit("Doing", &[]);
    plan.measure(&recopy);
    // A far end the board does not hold, read in a batch beside two it does, refuses the copy
    // by name before the item's own writes begin, and the item reads back as it stood.
    let show = ["task", "show", &on_board(0), "--no-comments", "--json"];
    let (_, _, before) = plan.measure(&show);
    std::fs::write(
        &path,
        linked.replace(
            "status: Todo",
            &format!(
                "status: Todo\ndepends_on: [{{id: \"{}\", item: task}}, {{id: \"board:ISSUE-404\", \
                 item: task}}, {{id: \"{}\", item: task}}]",
                on_board(1),
                on_board(2)
            ),
        ),
    )
    .unwrap();
    let served = plan.board.served().len();
    let output = plan
        .sandbox
        .command()
        .args(recopy)
        .assert()
        .get_output()
        .clone();
    assert_ne!(output.status.code(), Some(0), "{}", stdout(&output));
    assert!(
        stderr(&output).contains("GitHub dependency item ISSUE-404 was not found"),
        "{}",
        stderr(&output)
    );
    let refused = plan.board.served()[served..].to_vec();
    // Refused before the item's own writes began: no field and no edge was sent, and what the
    // copy's undo puts back is what was there.
    assert!(
        !refused
            .iter()
            .any(|(query, _)| query == graphql::UPDATE_FIELDS || query == graphql::ADD_BLOCKED_BY),
        "{refused:#?}"
    );
    assert_eq!(
        refused
            .iter()
            .filter(|(query, _)| query == graphql::ISSUE_DETAILS)
            .count(),
        1,
        "{refused:#?}"
    );
    let (_, _, after) = plan.measure(&show);
    assert_eq!(after, before);
    let many: Vec<usize> = (1..=DETAIL_BATCH + 1).collect();
    edit("Todo", &many);
    let (_, newly_many, _) = plan.measure(&recopy);
    // Named again, every one already blocks the item: its own read answers them, so nothing
    // is read for them and nothing is written for them.
    edit("Doing", &many);
    let (_, carried, _) = plan.measure(&recopy);
    let count = |sent: &[(String, Value)], document: &str| {
        sent.iter().filter(|(query, _)| query == document).count()
    };
    for (case, sent, newly) in [
        ("under a project", &under_project, 0),
        ("three newly named", &newly_few, 3),
        (
            "a batch and one more newly named",
            &newly_many,
            DETAIL_BATCH + 1,
        ),
        ("every one already carried", &carried, 0),
    ] {
        let batches = newly.div_ceil(DETAIL_BATCH);
        // The item, then the project its link names, each read once by its own id.
        assert_eq!(count(sent, graphql::ISSUE), 2, "{case}: {sent:#?}");
        assert_eq!(
            count(sent, graphql::ISSUE_DETAILS),
            batches,
            "{case}: {sent:#?}"
        );
        assert_eq!(
            count(sent, graphql::ADD_BLOCKED_BY),
            newly,
            "{case}: {sent:#?}"
        );
        assert_eq!(sent.len(), 4 + batches + newly, "{case}: {sent:#?}");
        let points: u64 = sent
            .iter()
            .map(|(document, _)| {
                onetaskgraph_github_projects::worst_case_point_cost(document).unwrap()
            })
            .sum();
        assert_eq!(points, sent.len() as u64, "{case}");
        // Every far end read, each exactly once, and nothing else in the batch's slots.
        let asked: std::collections::BTreeSet<String> = sent
            .iter()
            .filter(|(query, _)| query == graphql::ISSUE_DETAILS)
            .flat_map(|(_, variables)| {
                (0..DETAIL_BATCH)
                    .map(move |slot| variables[format!("id{slot}")].as_str().unwrap().to_owned())
            })
            .collect();
        let wanted: std::collections::BTreeSet<String> = (1..=newly)
            .map(|task| native(&json!(on_board(task))))
            .collect();
        assert_eq!(asked, wanted, "{case}");
    }
    for row in [
        "//! | bound copy, filed under a project | 4 |",
        "//! | bound copy, newly naming n dependencies | + ceil(n / DETAIL_BATCH) + n |",
    ] {
        assert!(
            include_str!("../../../onetaskgraph-github-projects/src/lib.rs").contains(row),
            "cost table: {row}"
        );
        assert!(
            PROTOCOL.contains(&row.replacen("//! ", "\n", 1)),
            "docs/plugin-protocol.md cost table: {row}"
        );
    }
    let (_, _, shown) = plan.measure(&["task", "deps", &on_board(0), "--json"]);
    let blockers: std::collections::BTreeSet<String> = shown["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|edge| edge["to"]["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        blockers,
        many.iter().map(|task| on_board(*task)).collect(),
        "{shown}"
    );
}
