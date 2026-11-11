//! A public copy is put to the write policy as one batch, read off a record of every policy
//! process the store started.
//!
//! The plan is public — a project, six tasks, a design document and two spike reports in the
//! public folder `draft`, its tasks naming two public repositories — and is copied with its
//! documents onto a loopback GitHub board declared public. The policy is a recording script
//! answering in onevcs's pinned boundary schema — `pass`, `refuse` with the surface it found a
//! term in, or `unavailable` at exit 2 — so a journey decides what it answers; the budget's
//! own journey runs the released onevcs behind the recorder instead, as a host does.

use std::collections::BTreeMap;
use std::path::PathBuf;

use onetaskgraph_e2e_support::onevcs::BoundaryCommand;
use serde_json::{Value, json};

use super::{OnevcsHome, Sandbox, Store, registered};
use crate::common::{stderr, stdout};
use crate::fixtures::GitHubBoardFields;

/// The two public repositories the plan's tasks name, by turns: both of the owner the loopback
/// board files its issues under, so each task's issue lands in the repository it names.
const OPEN: &str = "github.com/nickderobertis/onetaskgraph";
const SHOP: &str = "github.com/nickderobertis/widgets";

/// [`registered`], with the plan's two repositories registered public.
fn registered_with_plan(home: &mut OnevcsHome) {
    registered(home);
    home.public("nickderobertis", "onetaskgraph")
        .public("nickderobertis", "widgets");
}

/// A term the planting journey puts in one task's body, which no refusal may print.
const PLANTED: &str = "quietharbor-ledger-7731";

/// What the plan holds: one project, six tasks, one design document and two spike reports.
const ITEMS: usize = 10;

/// The recording policy: every run appends `{command, input}` to the log its first argument
/// names, and answers as onevcs's pinned schema has `boundary inspect` and `boundary check`
/// answer — every repository `public`; a check by the mode its third argument names.
const POLICY: &str = r#"import json, sys
log, command, mode = sys.argv[1], sys.argv[2], sys.argv[3]
document = json.loads(sys.stdin.buffer.read())
with open(log, "a", encoding="utf-8", newline="\n") as record:
    record.write(json.dumps({"command": command, "input": document}) + "\n")
if command == "inspect":
    print(json.dumps({"visibility": "public"}))
    sys.exit(0)
with open(log, encoding="utf-8") as record:
    checks = sum(1 for line in record if json.loads(line)["command"] == "check")
everything = document.get("text", []) + document.get("paths", []) + document.get("metadata", [])
if mode == "unavailable":
    print(json.dumps({"verdict": "unavailable", "reason": "registry"}))
    sys.exit(2)
if mode == "planted" and any("quietharbor-ledger-7731" in value for value in everything):
    print(json.dumps({"verdict": "refuse", "surface": "text"}))
    sys.exit(1)
print(json.dumps({"verdict": "pass"}))
sys.exit(0)
"#;

/// One policy process the store started.
#[derive(Clone)]
struct Run {
    command: String,
    input: Value,
}

/// The recording policy configured in a store's `write_policy`, and its log.
struct Policy {
    log: PathBuf,
}

impl Policy {
    /// The policy in `sandbox`, its checks answering by `mode`.
    fn new(sandbox: &Sandbox, mode: &str) -> (Self, Value) {
        let directory = sandbox.subdirectory(&format!("policy-{mode}"));
        let script = directory.join("policy.py");
        std::fs::write(&script, POLICY).expect("the policy script");
        let log = directory.join("runs.jsonl");
        let python = onetaskgraph_e2e_support::onevcs::python();
        let command = |name: &str| {
            json!([
                python.to_string_lossy(),
                script.to_string_lossy(),
                log.to_string_lossy(),
                name,
                mode
            ])
        };
        let configured =
            json!({"visibility_command": command("inspect"), "check_command": command("check")});
        (Self { log }, configured)
    }

    fn runs(&self) -> Vec<Run> {
        std::fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(|line| {
                let run: Value = serde_json::from_str(line).expect("a recorded run");
                Run {
                    command: run["command"].as_str().expect("a command").to_owned(),
                    input: run["input"].clone(),
                }
            })
            .collect()
    }

    fn checks(&self) -> Vec<Value> {
        self.runs()
            .into_iter()
            .filter(|run| run.command == "check")
            .map(|run| run.input)
            .collect()
    }
}

/// A store holding the public folder `draft` and the public loopback board `board`, its writes
/// screened by `policy`: the store, and the board.
fn store(
    onevcs: Option<fn(&mut OnevcsHome)>,
    policy: impl FnOnce(&Sandbox) -> Value,
) -> (Store, GitHubBoardFields) {
    let sandbox = Sandbox::new();
    let onevcs = onevcs.map(|register| {
        let mut home = OnevcsHome::new(&sandbox);
        register(&mut home);
        home
    });
    let policy = policy(&sandbox);
    let (config, board) = crate::fixtures::github_projects_with_items(&sandbox, Vec::new());
    let extra = json!({
        "draft": super::folder(&sandbox, "draft", "public"),
        "board": {"plugin": "github-projects", "config": config, "visibility": "public"},
    });
    (
        Store::configured(sandbox, onevcs, Some(policy), extra),
        board,
    )
}

/// The plan in `draft`: `goal`, six tasks naming [`OPEN`] and [`SHOP`] by turns — two of them
/// saying the same thing, so the batch holds that text once — the design document and two spike
/// reports, every one filed under `goal`.
fn plan(store: &Store) {
    store.record(
        "draft",
        "projects",
        "goal",
        "title: Faster retries\nstatus: todo",
        "Make the retry loop recover in a second.",
    );
    for n in 1..=6 {
        let repository = if n % 2 == 1 { OPEN } else { SHOP };
        let depends = if n > 1 {
            format!("\ndepends_on: [t{}]", n - 1)
        } else {
            String::new()
        };
        // The last two say the same thing.
        let body = if n >= 5 {
            "Measure the retry loop again.".to_owned()
        } else {
            format!("Step {n}: change the retry loop's backoff.")
        };
        store.record(
            "draft",
            "tasks",
            &format!("t{n}"),
            &format!(
                "title: Retry step {n}\nstatus: todo\nproject: goal\nrepositories: \
                 [{repository}]{depends}"
            ),
            &body,
        );
    }
    for (id, title) in [
        ("design", "Design of the faster retries"),
        ("spike-a", "Spike: backoff curves"),
        ("spike-b", "Spike: jitter"),
    ] {
        store.record(
            "draft",
            "documents",
            id,
            &format!("title: {title:?}\nproject: goal"),
            &format!("{title}, as measured."),
        );
    }
}

/// The copy of the whole plan with its documents onto the board.
const PLAN_COPY: [&str; 6] = [
    "project",
    "copy",
    "draft:goal",
    "--documents",
    "--to",
    "board",
];

/// Every mutation the board has been sent.
fn mutations(board: &GitHubBoardFields) -> Vec<String> {
    board
        .documents()
        .into_iter()
        .filter(|document| document.trim_start().starts_with("mutation"))
        .collect()
}

/// `values` counted: each value, as many times as it occurs.
fn counted(values: &Value) -> BTreeMap<String, usize> {
    let mut counted = BTreeMap::new();
    for value in values.as_array().into_iter().flatten() {
        *counted
            .entry(value.as_str().expect("a string entry").to_owned())
            .or_default() += 1;
    }
    counted
}

/// The multiset union of every input's `list`: each value as many times as the input holding it
/// most often holds it.
fn union(inputs: &[Value], list: &str) -> BTreeMap<String, usize> {
    let mut union: BTreeMap<String, usize> = BTreeMap::new();
    for input in inputs {
        for (value, times) in counted(&input[list]) {
            let held = union.entry(value).or_default();
            *held = (*held).max(times);
        }
    }
    union
}

/// Each item of the plan copied on its own, as a dry run: one command per item, each a batch of
/// one, so each check its record holds is that item's own composition, screened alone.
const ALONE: [&[&str]; ITEMS] = [
    &["project", "copy", "draft:goal", "--no-tasks"],
    &["task", "copy", "draft:t1"],
    &["task", "copy", "draft:t2"],
    &["task", "copy", "draft:t3"],
    &["task", "copy", "draft:t4"],
    &["task", "copy", "draft:t5"],
    &["task", "copy", "draft:t6"],
    &["document", "copy", "draft:design"],
    &["document", "copy", "draft:spike-a"],
    &["document", "copy", "draft:spike-b"],
];

#[test]
fn a_public_plan_copy_is_screened_as_one_batch_holding_exactly_what_its_items_screen() {
    let mut recorded = None;
    let (store, board) = store(None, |sandbox| {
        let (policy, configured) = Policy::new(sandbox, "pass");
        recorded = Some(policy);
        configured
    });
    let policy = recorded.unwrap();
    plan(&store);
    // The oracle: what the store composes for each item screened alone, each in a command of
    // its own and none in the plan's copy.
    let mut items = Vec::new();
    for alone in ALONE {
        let before = policy.checks().len();
        let mut with = alone.to_vec();
        with.extend(["--to", "board", "--dry-run"]);
        store.ok(&with);
        let checks = policy.checks();
        assert_eq!(checks.len(), before + 1, "{alone:?} is a batch of one");
        items.push(checks[before].clone());
    }
    assert!(mutations(&board).is_empty(), "a dry run writes nothing");

    // The plan's copy: one check, of exactly the union of those, and one visibility run per
    // repository the plan names — then everything lands.
    let runs_before = policy.runs().len();
    let output = store.ok(&{
        let mut with = PLAN_COPY.to_vec();
        with.push("--json");
        with
    });
    let report: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(report["items"].as_array().unwrap().len(), ITEMS, "{report}");
    let runs = policy.runs()[runs_before..].to_vec();
    let inspected: Vec<&Value> = runs
        .iter()
        .filter(|run| run.command == "inspect")
        .map(|run| &run.input["repository"])
        .collect();
    assert_eq!(inspected.len(), 2, "once per repository: {inspected:?}");
    for repository in [OPEN, SHOP] {
        assert!(inspected.contains(&&json!(repository)), "{repository}");
    }
    let checks: Vec<&Value> = runs
        .iter()
        .filter(|run| run.command == "check")
        .map(|run| &run.input)
        .collect();
    assert_eq!(checks.len(), 1, "one check for the whole copy");
    let batch = checks[0];
    for list in ["text", "paths", "metadata"] {
        assert_eq!(
            counted(&batch[list]),
            union(&items, list),
            "the batch's {list} is the union of its items'"
        );
    }
    for item in &items {
        assert_eq!(item.get("scope"), batch.get("scope"), "one scope");
    }
    assert_eq!(
        counted(&batch["text"])["Measure the retry loop again."],
        1,
        "two items saying the same thing put it in the batch once"
    );
    assert!(!mutations(&board).is_empty(), "the copy landed");
}

#[test]
fn a_planted_term_in_one_task_refuses_the_whole_copy_on_one_check_without_printing_it() {
    let mut recorded = None;
    let (store, board) = store(None, |sandbox| {
        let (policy, configured) = Policy::new(sandbox, "planted");
        recorded = Some(policy);
        configured
    });
    let policy = recorded.unwrap();
    plan(&store);
    store.record(
        "draft",
        "tasks",
        "t3",
        &format!("title: Retry step 3\nstatus: todo\nproject: goal\nrepositories: [{OPEN}]"),
        &format!("Mirror what {PLANTED} does."),
    );
    let held = board.issues();
    let mut with = PLAN_COPY.to_vec();
    with.push("--json");
    let output = store.run(&with);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    let failure: Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(failure["failure"]["kind"], json!("boundary-refused"));
    assert!(
        stderr(&output).contains(&format!("this copy's {ITEMS} writes")),
        "it refuses the copy whole: {}",
        stderr(&output)
    );
    for printed in [stdout(&output), stderr(&output)] {
        assert!(!printed.contains(PLANTED), "neutrally: {printed}");
        assert!(!printed.contains("draft:t3"), "naming no item: {printed}");
    }
    assert_eq!(policy.checks().len(), 1, "one check, and no other");
    assert!(
        mutations(&board).is_empty(),
        "nothing of the plan was written"
    );
    assert_eq!(board.issues(), held, "the board holds nothing of it");
}

#[test]
fn a_check_that_cannot_decide_starts_one_check_and_leaves_the_board_without_a_write() {
    let mut recorded = None;
    let (store, board) = store(None, |sandbox| {
        let (policy, configured) = Policy::new(sandbox, "unavailable");
        recorded = Some(policy);
        configured
    });
    let policy = recorded.unwrap();
    plan(&store);
    let held = board.issues();
    let (kind, said) = store.refused(&PLAN_COPY);
    assert_eq!(kind, "boundary-unavailable", "{said}");
    assert_eq!(policy.checks().len(), 1, "one check, and no other");
    assert!(mutations(&board).is_empty(), "nothing was written");
    assert_eq!(board.issues(), held, "the board holds what it held");
}

#[test]
fn a_task_copied_on_its_own_is_screened_as_a_batch_of_one() {
    let mut policy = None;
    let (store, board) = store(None, |sandbox| {
        let (recording, configured) = Policy::new(sandbox, "pass");
        policy = Some(recording);
        configured
    });
    let policy = policy.unwrap();
    store.record(
        "draft",
        "tasks",
        "solo",
        &format!("title: A task of its own\nstatus: todo\nrepositories: [{OPEN}]"),
        "Nothing else is copied with it.",
    );
    store.ok(&["task", "copy", "draft:solo", "--to", "board"]);
    let checks = policy.checks();
    assert_eq!(checks.len(), 1, "one check");
    assert!(counted(&checks[0]["metadata"]).contains_key("A task of its own"));
    assert_eq!(
        counted(&checks[0]["text"]),
        BTreeMap::from([("Nothing else is copied with it.".to_owned(), 1)])
    );
    assert_eq!(
        policy
            .runs()
            .iter()
            .filter(|run| run.command == "inspect")
            .count(),
        1
    );
    assert!(!mutations(&board).is_empty(), "it landed");
}

#[test]
fn a_public_plan_copy_starts_one_check_and_one_inspect_per_repository_of_the_released_onevcs() {
    // As a host runs it: the released onevcs boundary commands behind the recorder.
    let sandbox = Sandbox::new();
    let mut home = OnevcsHome::new(&sandbox);
    registered_with_plan(&mut home);
    let record = home.recorded(&sandbox);
    let (config, board) = crate::fixtures::github_projects_with_items(&sandbox, Vec::new());
    let extra = json!({
        "draft": super::folder(&sandbox, "draft", "public"),
        "board": {"plugin": "github-projects", "config": config, "visibility": "public"},
    });
    let policy = record.write_policy().clone();
    let store = Store::configured(sandbox, Some(home), Some(policy), extra);
    plan(&store);
    let mut with = PLAN_COPY.to_vec();
    with.push("--json");
    let report: Value = serde_json::from_str(&store.ok(&with)).unwrap();
    let items = report["items"].as_array().unwrap().len();
    assert_eq!(items, ITEMS, "{report}");
    assert!(!mutations(&board).is_empty(), "the plan landed");
    let calls = record.calls();
    let inspects = calls
        .iter()
        .filter(|call| call.command == BoundaryCommand::Inspect)
        .count();
    let checks: Vec<_> = calls
        .iter()
        .filter(|call| call.command == BoundaryCommand::Check)
        .collect();
    let screened = ["Faster retries"]
        .into_iter()
        .map(str::to_owned)
        .chain((1..=6).map(|n| format!("Retry step {n}")))
        .chain(
            [
                "Design of the faster retries",
                "Spike: backoff curves",
                "Spike: jitter",
            ]
            .map(str::to_owned),
        )
        .filter(|title| {
            checks
                .iter()
                .any(|check| counted(&check.input["metadata"]).contains_key(title))
        })
        .count();
    assert_eq!(screened, ITEMS, "every item was screened");
    crate::budget_runner::record(
        BOUNDARY_BUDGET,
        (inspects + checks.len()) as f64,
        &format!(
            "{inspects} visibility runs (boundary inspect), {} check runs (boundary check), \
             {screened} items screened",
            checks.len()
        ),
        BOUNDARY_WORKLOAD,
    );
}

#[test]
fn under_a_policy_that_rewrites_a_value_rewritable_in_one_write_and_fixed_in_another_keeps_both() {
    // `t1` is the id task t1's create offers the board, which a rewriting policy may rewrite, and
    // the far end of t2's dependency, which nothing rewrites: the batch hands the released
    // `boundary sanitize` both, so the fixed one is never screened only behind the other.
    let sandbox = Sandbox::new();
    let mut home = OnevcsHome::new(&sandbox);
    registered_with_plan(&mut home);
    let record = home.recorded_sanitizing(&sandbox, json!([]));
    let (config, board) = crate::fixtures::github_projects_with_items(&sandbox, Vec::new());
    let extra = json!({
        "draft": super::folder(&sandbox, "draft", "public"),
        "board": {"plugin": "github-projects", "config": config, "visibility": "public"},
    });
    let policy = record.write_policy().clone();
    let store = Store::configured(sandbox, Some(home), Some(policy), extra);
    plan(&store);
    let mut with = PLAN_COPY.to_vec();
    with.push("--json");
    store.ok(&with);
    assert!(!mutations(&board).is_empty(), "the plan landed");
    let batches: Vec<Value> = record
        .calls()
        .into_iter()
        .filter(|call| call.command == BoundaryCommand::Sanitize)
        .map(|call| call.input)
        .filter(|input| counted(&input["metadata"]).contains_key("Retry step 2"))
        .collect();
    assert_eq!(batches.len(), 1, "one sanitisation of the whole plan");
    let batch = &batches[0];
    assert!(
        counted(&batch["metadata"]).contains_key("t1"),
        "rewritable: {batch}"
    );
    assert!(
        counted(&batch["fixed"]["metadata"]).contains_key("t1"),
        "fixed: {batch}"
    );
}

#[test]
fn a_dry_run_sizes_a_body_as_the_policy_rewrote_it() {
    // The released `boundary sanitize` replaces a declared term with a shorter word: the dry run
    // sizes the body as it would be written, replacement made, and the board stores exactly that.
    let sandbox = Sandbox::new();
    let mut home = OnevcsHome::new(&sandbox);
    registered_with_plan(&mut home);
    let record = home.recorded_sanitizing(
        &sandbox,
        json!([{"term": "quietharbor", "with": "harbor-x"}]),
    );
    let (config, board) = crate::fixtures::github_projects_with_items(&sandbox, Vec::new());
    let extra = json!({
        "draft": super::folder(&sandbox, "draft", "public"),
        "board": {"plugin": "github-projects", "config": config, "visibility": "public"},
    });
    let policy = record.write_policy().clone();
    let store = Store::configured(sandbox, Some(home), Some(policy), extra);
    store.record(
        "draft",
        "tasks",
        "solo",
        &format!("title: A task of its own\nstatus: todo\nrepositories: [{OPEN}]"),
        "Mirror the retry loop quietharbor uses, as quietharbor does.",
    );
    let copy = ["task", "copy", "draft:solo", "--to", "board", "--json"];
    let mut dry = copy.to_vec();
    dry.push("--dry-run");
    let report: Value = serde_json::from_str(&store.ok(&dry)).expect("a report");
    let sized = report["items"][0]["body_size"]
        .as_u64()
        .unwrap_or_else(|| panic!("a size: {report}"));
    let landed: Value = serde_json::from_str(&store.ok(&copy)).expect("a report");
    let id = landed["items"][0]["destination"]
        .as_str()
        .expect("a destination")
        .strip_prefix("board:")
        .expect("on the board")
        .to_owned();
    let body = board.body(&id);
    let body = body.as_str().expect("a body");
    assert!(
        body.contains("harbor-x") && !body.contains("quietharbor"),
        "{body}"
    );
    assert_eq!(
        sized,
        body.chars().count() as u64,
        "the dry run sized the body as the policy rewrote it"
    );
}

/// The budget the journey above records, and the workload it records it for.
pub(crate) const BOUNDARY_BUDGET: &str = "boundary-invocations-per-public-copy";
pub(crate) const BOUNDARY_WORKLOAD: &str = "public plan of a project, six tasks, a design document and two spike reports naming two \
     public repositories, copied with its documents onto a public board";
