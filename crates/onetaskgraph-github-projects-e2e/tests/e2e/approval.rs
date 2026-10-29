//! What approving a plan on a board costs: the store calls an approval makes, each through the
//! real binary against the loopback board of `plan_copy` holding 2,000 other items — the review
//! task `task project-human` creates under the plan's project, `project show`, `task list
//! --project`, `document list --project`, then `document metadata set --expect` carrying the
//! version that listing reported for the design document, and last the review task resolved
//! through `task project-human`. The listing carries the document whole, exactly as `document
//! show` would answer it, so approval reads it once.
//!
//! Each call's requests are priced by the plugin's own point-cost table, the one
//! `tests/point_cost.rs` pins and the credentialed lane reconciles against GitHub, and what the
//! sequence spent — split into the existing calls and the review's projection — is recorded as
//! the `github-approval-calls-points` budget's telemetry, which `human-items/approval-report.mjs`
//! hands to onebudgetspec. Nothing here holds the total to a bound: that is the budget's. What
//! is held here is what each call sends, that the records come back whole and in order, that the
//! existing calls wait for nothing on the way, that the review task is created once and resolved
//! once, and that a write expecting a version the board has moved past — whoever moved it, and
//! whether or not anything here saw it — writes nothing and leaves the review open.

use super::asset_board::{Call, Kind};
use super::plan_copy::{Plan, SMALL, TASKS, counterparts, never_walks, operation, success};
use onetaskgraph_e2e_support::common::{stderr, stdout};
use onetaskgraph_e2e_support::onevcs::{BoundaryCall, BoundaryCommand};
use onetaskgraph_github_projects::worst_case_point_cost;
use serde_json::{Value, json};

pub(super) const BUDGET: &str = "github-approval-calls-points";

/// Named in the recording, so the runner refuses telemetry another journey wrote.
pub(super) const WORKLOAD: &str = "plan approval store calls";

/// The calls an approval makes, in the order it makes them, by what each is: the review task's
/// creation, the four existing store calls, and the review task's resolution.
pub(super) const CALLS: [&str; 6] = [
    REVIEW_OPEN,
    "project show",
    "task list --project",
    "document list --project",
    "document metadata set --expect",
    REVIEW_RESOLVED,
];

/// The review task's creation through `task project-human`.
const REVIEW_OPEN: &str = "task project-human (review open)";

/// The review task's resolution through `task project-human`.
const REVIEW_RESOLVED: &str = "task project-human (review completed)";

/// The host every approval's configuration names as authoritative over its projections.
const HOST: &str = "approval-host";

const KEY: &str = "plan.approval";

/// The read that holds a guarded write to the version: the document, with its project and the
/// project's members beside it.
const GUARDED_READ: &str = "reading one document with its project and the project's members";

/// What a guarded write sends: the read that holds it to the version, then the write.
const GUARDED_WRITE: [&str; 2] = [GUARDED_READ, "updating an issue"];

/// One call of the sequence: what it printed, and the requests the board answered for it.
struct Step {
    what: &'static str,
    report: Value,
    calls: Vec<Call>,
    /// The boundary commands the call ran, when the store screens its writes.
    screening: Vec<BoundaryCall>,
}

/// The plan copied onto a board of [`SMALL`] other items — with the board source's cache on when
/// `cache` is — and the board ids of its project and its document.
fn copied(cache: bool) -> (Plan, String, String) {
    landed(Plan::configured(
        SMALL,
        Vec::new(),
        cache.then(|| json!({"enabled": true})),
    ))
}

/// [`copied`], with the store screening every public write through the released onevcs
/// boundary commands against a home of synthetic identities.
fn copied_screened() -> (Plan, String, String) {
    landed(Plan::screened(SMALL, Vec::new(), None))
}

fn landed(plan: Plan) -> (Plan, String, String) {
    let (project, document, _) = plan.copy();
    let at = counterparts(&project, &document);
    (plan, at["P"].clone(), at["D"].clone())
}

/// What `call` costs against the GraphQL allowance by the plugin's own table: a GraphQL
/// document's worst-case price, and nothing for a REST request.
fn points(call: &Call) -> u64 {
    if matches!(
        call.kind,
        Kind::Upload | Kind::Repository | Kind::Verification
    ) {
        return 0;
    }
    let request: Value = serde_json::from_slice(&call.bytes).unwrap();
    worst_case_point_cost(request["query"].as_str().unwrap()).expect("a document the plugin prices")
}

/// The version of the one document a `document show` or `document list --json` printed.
fn version_of(shown: &Value) -> String {
    shown["items"][0]["item"]["version"]
        .as_str()
        .expect("a document read from the board reports its version")
        .to_owned()
}

/// The version `document show` reports for `id` now.
fn shown_version(plan: &Plan, id: &str) -> String {
    version_of(&success(&plan.run(&["document", "show", id, "--json"])))
}

/// Give the board source the projection authority a review task is written under: this host,
/// an intent directory in the plan's sandbox, and the plan's project.
fn authorize_reviews(plan: &Plan, project: &str) {
    let path = plan.sandbox.project().join("onetaskgraph.yaml");
    let mut configured: Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    configured["sources"]["board"]["task_projection"] = json!({"host": HOST,
        "directory": plan.sandbox.subdirectory("intents"), "project": project});
    plan.sandbox.project_document(&configured.to_string());
}

/// The review task of the plan whose project is `project`, at `state`.
fn review(plan: &Plan, project: &str, state: &str) -> Value {
    let repository = plan
        .github
        .repository_of(project)
        .expect("the plan's project issue");
    let request = json!({"schema_version": 1, "item_id": format!("review-{project}"),
        "owner": {"session": "manager-session", "run": "run-1", "plan": "plan-1"},
        "title": "Review the plan's design", "body": "Approve the design or say what changes.",
        "priority": "high", "repositories": [format!("github.com/{repository}")],
        "project": format!("board:{project}"), "source": "board", "state": state,
        "reason": null});
    let path = plan.sandbox.project().join(format!("review-{state}.json"));
    std::fs::write(&path, request.to_string()).unwrap();
    json!(path)
}

/// Project the review task at `state`, with what the board answered.
fn project_review(plan: &Plan, project: &str, state: &str, what: &'static str) -> Step {
    let path = review(plan, project, state);
    let screened = plan.boundary_calls().len();
    let (output, calls) = plan.measured_with(
        &[
            "task",
            "project-human",
            "--input",
            path.as_str().unwrap(),
            "--json",
        ],
        &[("ONETASKGRAPH_HOST", HOST)],
    );
    Step {
        what,
        report: success(&output),
        calls,
        screening: plan.boundary_calls()[screened..].to_vec(),
    }
}

fn approve(plan: &Plan, project: &str, document: &str) -> Vec<Step> {
    authorize_reviews(plan, project);
    let mut steps = vec![project_review(plan, project, "open", REVIEW_OPEN)];
    steps.extend(approve_existing(plan, project, document));
    steps.push(project_review(plan, project, "completed", REVIEW_RESOLVED));
    steps
}

/// The four existing store calls of an approval, as they were before a review task joined them.
fn approve_existing(plan: &Plan, project: &str, document: &str) -> Vec<Step> {
    let project = format!("board:{project}");
    let document = format!("board:{document}");
    let mut steps = Vec::new();
    for what in &CALLS[1..4] {
        let verb = match *what {
            "project show" => vec!["project", "show", &project, "--json"],
            "task list --project" => vec!["task", "list"],
            _ => vec!["document", "list"],
        };
        let mut arguments = verb;
        if arguments.len() == 2 {
            arguments.extend(["--source", "board", "--project", &project, "--json"]);
        }
        let screened = plan.boundary_calls().len();
        let (report, calls) = plan.measured(&arguments);
        steps.push(Step {
            what,
            report,
            calls,
            screening: plan.boundary_calls()[screened..].to_vec(),
        });
    }
    let version = version_of(&steps[2].report);
    let screened = plan.boundary_calls().len();
    let (report, calls) = plan.measured(&[
        "document",
        "metadata",
        "set",
        &document,
        KEY,
        "\"approved\"",
        "--expect",
        &version,
        "--json",
    ]);
    steps.push(Step {
        what: CALLS[4],
        report,
        calls,
        screening: plan.boundary_calls()[screened..].to_vec(),
    });
    steps
}

/// The operation each request of `calls` is.
fn sent(calls: &[Call]) -> Vec<String> {
    calls.iter().map(operation).collect()
}

/// `document metadata set <id> <key> true --expect <version>`, with what the board answered.
fn set_expecting(
    plan: &Plan,
    id: &str,
    key: &str,
    version: &str,
) -> (std::process::Output, Vec<Call>) {
    plan.measured_output(&[
        "document", "metadata", "set", id, key, "true", "--expect", version, "--json",
    ])
}

/// The `(id, title)` of every row a listing printed, in the order it printed them.
fn rows(answer: &Value) -> Vec<(String, String)> {
    answer["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            (
                row["item"]["id"].as_str().unwrap().to_owned(),
                row["item"]["title"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

/// The caller keys of the metadata slot the board holds in `id`'s body.
fn slot(plan: &Plan, id: &str) -> Value {
    let body = plan.github.body(id);
    let body = body.as_str().expect("the board holds the item");
    let start = body
        .find("<!-- onetaskgraph.metadata")
        .expect("the body carries its metadata slot");
    let inner = &body[start + "<!-- onetaskgraph.metadata".len()..];
    let end = inner.find("-->").expect("the slot is closed");
    serde_json::from_str(inner[..end].trim()).expect("the slot is JSON")
}

/// `steps` as the budget's detail reads them, the shape `budget_runner` holds a recording to.
fn breakdown(steps: &[Step]) -> Vec<Value> {
    steps
        .iter()
        .map(|step| {
            json!({"call": step.what,
                   "requests": step.calls.len(),
                   "points": step.calls.iter().map(points).sum::<u64>(),
                   "operations": step.calls.iter().map(operation).collect::<Vec<_>>(),
                   "screening": step.screening.iter().map(|call| json!({
                       "command": call.command, "exit": call.exit})).collect::<Vec<_>>()})
        })
        .collect()
}

/// Whether `step` is the review task's projection rather than one of the existing calls.
fn projects_the_review(step: &Step) -> bool {
    step.what == REVIEW_OPEN || step.what == REVIEW_RESOLVED
}

/// What `steps` spent, in points: all of them, the existing calls', and the review's.
fn spent(steps: &[Step]) -> Value {
    let total = |keep: &dyn Fn(&Step) -> bool| -> u64 {
        steps
            .iter()
            .filter(|step| keep(step))
            .flat_map(|step| &step.calls)
            .map(points)
            .sum()
    };
    json!({"points": total(&|_| true),
           "existing": total(&|step| !projects_the_review(step)),
           "projection": total(&projects_the_review),
           "calls": breakdown(steps)})
}

/// Record what the sequence spent as the budget's telemetry: the larger of the run with no
/// write policy and the run with write screening on, each split into the existing store calls
/// and the review task's projection, with both runs' calls in the detail.
fn record(unscreened: &[Step], screened: &[Step]) {
    let (unscreened, screened) = (spent(unscreened), spent(screened));
    let larger = if screened["points"].as_u64() > unscreened["points"].as_u64() {
        &screened
    } else {
        &unscreened
    };
    let directory = super::budget_runner::directory();
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        onetaskgraph_e2e_support::telemetry::file_in(&directory, BUDGET),
        serde_json::to_vec_pretty(&json!({"budget": BUDGET, "value": larger["points"],
            "workload": WORKLOAD, "board": SMALL, "tasks": TASKS, "documents": 1,
            "call_order": CALLS, "unscreened": unscreened, "screened": screened}))
        .unwrap(),
    )
    .unwrap();
}

#[test]
fn approving_a_plan_lists_each_kind_in_one_point_and_writes_its_approval_behind_one_read() {
    let (plan, project, document) = copied(false);
    authorize_reviews(&plan, &project);
    let opened = project_review(&plan, &project, "open", REVIEW_OPEN);
    let waits_before = plan.clock.slept().len();
    let existing = approve_existing(&plan, &project, &document);
    let waits: Vec<_> = plan.clock.slept()[waits_before..].to_vec();
    let resolved = project_review(&plan, &project, "completed", REVIEW_RESOLVED);
    let mut steps = vec![opened];
    steps.extend(existing);
    steps.push(resolved);
    // Every wait the binary schedules — pacing, a secondary limit, a backoff — is recorded on the
    // clock with its cause, and the board answers nothing to a binary not attached to it, so a
    // count of none is a record of none rather than an absence of one.
    assert_eq!(
        waits.len(),
        0,
        "the approval sequence waited on the clock — pacing, a secondary limit or a backoff: \
         {waits:?}"
    );

    for step in &steps {
        never_walks(step.what, &step.calls);
    }
    let step = |what: &str| steps.iter().find(|step| step.what == what).unwrap();

    // Each listing is one request, priced at one point, carrying the project's whole page — the
    // plan's tasks and the review task filed under the same project.
    // llmlint: ignore-block[tests_hold_no_nonfunctional_thresholds] The node's acceptance criteria require this journey to show each listing is one request priced at one point by the plugin's own table: the point is the functional outcome of MEMBER_PAGE_SIZE, which `tests/point_cost.rs` pins, rather than a budget figure. The sequence's total is recorded below for onebudgetspec, which alone judges it against `github-approval-calls-points`.
    for (what, records) in [
        ("task list --project", TASKS + 1),
        ("document list --project", 1),
    ] {
        let listing = step(what);
        assert_eq!(
            sent(&listing.calls),
            vec!["reading a project's tasks".to_owned()],
            "{what}"
        );
        assert_eq!(
            listing.calls.iter().map(points).sum::<u64>(),
            1,
            "{what} is not priced at one point"
        );
        assert_eq!(rows(&listing.report).len(), records, "{what}");
    }
    // llmlint: ignore-end[tests_hold_no_nonfunctional_thresholds]

    // The tasks come back in the order they were filed under the project, the review task last,
    // each with what a read of it by its own id reports.
    let listed = rows(&step("task list --project").report);
    let titles: Vec<String> = listed.iter().map(|(_, title)| title.clone()).collect();
    let mut filed: Vec<String> = (1..=TASKS)
        .map(|n| format!("Step {n} of the plan"))
        .collect();
    filed.push("Review the plan's design".to_owned());
    assert_eq!(titles, filed);
    // The review task is resolved since it was listed, so the plan's own tasks are compared.
    for row in step("task list --project").report["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["item"]["metadata"].get("human.item_id").is_none())
    {
        let id = format!("board:{}", row["item"]["id"].as_str().unwrap());
        let shown = success(&plan.run(&["task", "show", &id, "--no-comments", "--json"]));
        assert_eq!(row["item"], shown["items"][0]["item"], "{id}");
    }

    // The write is held to the version the listing reported by one read, then sent once.
    let write = step("document metadata set --expect");
    assert_eq!(sent(&write.calls), GUARDED_WRITE);
    assert_eq!(slot(&plan, &document)[KEY], json!("approved"));

    // The listing carried the document exactly as reading it by its id does, so approval reading
    // it once loses nothing.
    let listed = &step("document list --project").report["items"][0]["item"];
    assert_eq!(listed["id"], json!(document));
    let mut shown =
        success(&plan.run(&["document", "show", &format!("board:{document}"), "--json"]));
    let mut before = listed.clone();
    // Only what the write moved differs: its key, the version and the time it was updated.
    for item in [&mut before, &mut shown["items"][0]["item"]] {
        item["metadata"].as_object_mut().unwrap().remove(KEY);
        let fields = item.as_object_mut().unwrap();
        fields.remove("version");
        fields.remove("updated_at");
    }
    assert_eq!(before, shown["items"][0]["item"]);

    // The same approval with every public write screened: the metadata write is still the
    // read that holds its version — which carries the project and its members, so who may read
    // the document is decided from it and nothing of the project is read again — and the
    // write, and its one check passed before that write.
    let (screened_plan, screened_project, screened_document) = copied_screened();
    let screened = approve(&screened_plan, &screened_project, &screened_document);
    let write = &screened[4];
    assert_eq!(
        sent(&write.calls),
        GUARDED_WRITE,
        "{:?}",
        sent(&write.calls)
    );
    let checks: Vec<&BoundaryCall> = write
        .screening
        .iter()
        .filter(|call| call.command == BoundaryCommand::Check)
        .collect();
    assert_eq!(checks.len(), 1, "{:?}", write.screening);
    let updated = write
        .calls
        .iter()
        .find(|call| call.creating)
        .expect("the write");
    assert!(checks[0].finished < updated.wall, "checked before it wrote");
    for step in &screened[1..4] {
        assert!(step.screening.is_empty(), "{} screened nothing", step.what);
    }
    // The review task's own writes are screened like every other public write.
    for step in [&screened[0], &screened[5]] {
        assert!(
            step.screening
                .iter()
                .any(|call| call.command == BoundaryCommand::Check),
            "{} was screened",
            step.what
        );
    }
    assert_eq!(
        slot(&screened_plan, &screened_document)[KEY],
        json!("approved")
    );

    // The review task was created once, under the plan's project, before the approval, and
    // resolved once after it — on the board both times, as one issue.
    for (plan, project, steps) in [
        (&plan, &project, &steps),
        (&screened_plan, &screened_project, &screened),
    ] {
        let opened = &steps[0].report;
        let resolved = &steps[5].report;
        assert_eq!(opened, resolved);
        assert_eq!(opened["project"], json!(project));
        assert_eq!(opened["source"], "board");
        let review = opened["task"]
            .as_str()
            .unwrap()
            .strip_prefix("board:")
            .unwrap();
        assert_eq!(
            plan.github.state(review),
            ("CLOSED".to_owned(), Some("COMPLETED".to_owned()))
        );
        assert_eq!(plan.github.status(review).as_deref(), Some("Done"));
        assert_eq!(
            plan.github
                .issues()
                .iter()
                .find(|issue| issue["id"] == json!(review))
                .unwrap()["parent"],
            json!(project)
        );
    }

    record(&steps, &screened);
}

#[test]
fn a_stale_version_refuses_the_approval_and_leaves_its_review_task_open() {
    let (plan, project, document) = copied(false);
    authorize_reviews(&plan, &project);
    let opened = project_review(&plan, &project, "open", REVIEW_OPEN).report;
    let review = opened["task"]
        .as_str()
        .unwrap()
        .strip_prefix("board:")
        .unwrap()
        .to_owned();
    let id = format!("board:{document}");
    let stale = shown_version(&plan, &id);
    plan.github
        .edit_body(&document, "Edited on the board after the version was read.");
    let (refused, calls) = set_expecting(&plan, &id, KEY, &stale);
    assert_ne!(refused.status.code(), Some(0), "{}", stdout(&refused));
    assert_eq!(sent(&calls), [GUARDED_READ], "nothing was written");
    let failure: Value = serde_json::from_str(&stdout(&refused)).expect("a failure document");
    assert_eq!(failure["failure"]["kind"], "version-moved", "{failure}");
    assert!(
        !plan.github.body(&document).as_str().unwrap().contains(KEY),
        "the refused approval wrote nothing"
    );
    // The refused approval resolved nothing: the review task is open on the board.
    assert_eq!(plan.github.state(&review).0, "OPEN");
    assert_eq!(plan.github.status(&review).as_deref(), Some("Todo"));

    // With the version the board holds now, the approval lands and the review is resolved.
    let current = shown_version(&plan, &id);
    let (landed, _) = set_expecting(&plan, &id, KEY, &current);
    assert_eq!(landed.status.code(), Some(0), "{}", stderr(&landed));
    assert_eq!(slot(&plan, &document)[KEY], json!(true));
    let resolved = project_review(&plan, &project, "completed", REVIEW_RESOLVED).report;
    assert_eq!(resolved, opened);
    assert_eq!(
        plan.github.state(&review),
        ("CLOSED".to_owned(), Some("COMPLETED".to_owned()))
    );
}

#[test]
fn a_document_write_expecting_a_version_the_board_has_moved_past_unseen_is_refused() {
    for cache in [false, true] {
        let (plan, _, document) = copied(cache);
        let id = format!("board:{document}");
        // Read through the cache when it is on, so it holds the very version the write expects.
        let version = shown_version(&plan, &id);
        // Changed on the board directly: nothing here reads it.
        plan.github
            .edit_body(&document, "Edited on the board after the token was read.");
        let edited = plan.github.body(&document);
        let (refused, calls) = set_expecting(&plan, &id, KEY, &version);
        assert_ne!(
            refused.status.code(),
            Some(0),
            "cache {cache}: a write expecting a version the board has moved past landed: {:?}",
            sent(&calls)
        );
        assert_eq!(sent(&calls), [GUARDED_READ], "cache {cache}");
        let said = stderr(&refused);
        assert!(
            said.contains(&version) && said.contains("nothing was written"),
            "cache {cache}: {said}"
        );
        let now = shown_version(&plan, &id);
        assert!(
            said.contains(&now),
            "cache {cache}: names the version it found: {said}"
        );
        let failure: Value = serde_json::from_str(&stdout(&refused)).expect("a failure document");
        assert_eq!(
            failure["failure"]["kind"], "version-moved",
            "cache {cache}: {failure}"
        );
        assert_eq!(plan.github.body(&document), edited, "cache {cache}");

        // Read again, the version it reports is written, over the person's edit's own body.
        let (landed, calls) = set_expecting(&plan, &id, KEY, &now);
        assert_eq!(
            landed.status.code(),
            Some(0),
            "cache {cache}: {}",
            stderr(&landed)
        );
        assert_eq!(sent(&calls), GUARDED_WRITE, "cache {cache}");
        assert!(
            plan.github
                .body(&document)
                .as_str()
                .unwrap()
                .starts_with("Edited on the board after the token was read."),
            "cache {cache}: the write kept the person's edit"
        );
        assert_eq!(slot(&plan, &document)[KEY], json!(true), "cache {cache}");
    }
}

#[test]
fn a_document_write_expecting_its_own_superseded_version_is_refused() {
    let (plan, _, document) = copied(true);
    let id = format!("board:{document}");
    let first = shown_version(&plan, &id);
    let (landed, _) = set_expecting(&plan, &id, "plan.first", &first);
    assert_eq!(landed.status.code(), Some(0), "{}", stderr(&landed));
    let after = plan.github.body(&document);
    let (refused, calls) = set_expecting(&plan, &id, "plan.second", &first);
    assert_ne!(refused.status.code(), Some(0), "{}", stdout(&refused));
    assert_eq!(sent(&calls), [GUARDED_READ]);
    assert_eq!(plan.github.body(&document), after);
    assert!(slot(&plan, &document).get("plan.second").is_none());
}

#[test]
fn a_document_write_refuses_a_version_it_cannot_order_and_a_document_the_board_lacks() {
    let (plan, _, document) = copied(false);
    let id = format!("board:{document}");
    let before = plan.github.body(&document);
    let (refused, calls) = set_expecting(&plan, &id, KEY, "not-a-version");
    assert_ne!(refused.status.code(), Some(0), "{}", stdout(&refused));
    assert_eq!(sent(&calls), [GUARDED_READ]);
    assert!(
        stderr(&refused).contains("not-a-version"),
        "{}",
        stderr(&refused)
    );
    assert_eq!(plan.github.body(&document), before);

    let version = shown_version(&plan, &id);
    let (missing, calls) = set_expecting(&plan, "board:ISSUE-NOT-HELD", KEY, &version);
    assert_ne!(missing.status.code(), Some(0), "{}", stdout(&missing));
    assert!(
        sent(&calls)
            .iter()
            .all(|operation| !operation.starts_with("updating an issue")),
        "{:?}",
        sent(&calls)
    );
    assert!(
        stderr(&missing).contains("ISSUE-NOT-HELD"),
        "{}",
        stderr(&missing)
    );

    // A project's metadata still takes no version.
    let refused = plan.run(&[
        "project", "metadata", "set", &id, KEY, "true", "--expect", &version,
    ]);
    assert_ne!(refused.status.code(), Some(0), "{}", stdout(&refused));
    assert!(stderr(&refused).contains("leave --expect out"));
}

#[test]
fn a_document_write_refused_after_its_version_held_writes_nothing_and_the_next_one_lands() {
    let (plan, _, document) = copied(false);
    let id = format!("board:{document}");
    let version = shown_version(&plan, &id);
    let before = plan.github.body(&document);
    plan.github.refuse_once("updateIssue(input:$input)");
    let (refused, calls) = set_expecting(&plan, &id, KEY, &version);
    assert_ne!(refused.status.code(), Some(0), "{}", stdout(&refused));
    assert_eq!(
        sent(&calls),
        GUARDED_WRITE,
        "the write was sent once, after the read"
    );
    let failure: Value = serde_json::from_str(&stdout(&refused)).expect("a failure document");
    assert_ne!(failure["failure"]["kind"], "version-moved", "{failure}");
    assert_eq!(plan.github.body(&document), before);

    // Nothing was written, so the version still holds and the next write lands.
    let (landed, calls) = set_expecting(&plan, &id, KEY, &version);
    assert_eq!(landed.status.code(), Some(0), "{}", stderr(&landed));
    assert_eq!(sent(&calls), GUARDED_WRITE);
    assert_eq!(slot(&plan, &document)[KEY], json!(true));
}

#[test]
fn a_screened_approval_reclassifies_a_project_a_person_changed_since_and_refuses_writing_nothing() {
    // The plan's board copy is approved once, its members public. Then a person records one task
    // of it as changing a private repository, in the list its metadata slot keeps on GitHub —
    // which is what says where a board task's work lives. The next approval is a new command: it reads the
    // document with its project and members afresh, asks about the repository it has not seen,
    // finds the project private by what it holds, and refuses the document's write before it is
    // sent, leaving the board as it was.
    let (mut plan, project, document) = copied_screened();
    let first = approve(&plan, &project, &document);
    assert_eq!(slot(&plan, &document)[KEY], json!("approved"));
    assert!(
        first[4]
            .screening
            .iter()
            .all(|call| call.command != BoundaryCommand::Inspect
                || call.input["repository"] != "github.com/synthco/secret-gadgets"),
        "{:?}",
        first[4].screening
    );
    plan.screening
        .as_mut()
        .expect("a screened plan")
        .0
        .private("synthco", "secret-gadgets");
    let members = rows(&first[2].report);
    let moved = members[2].0.clone();
    let body = plan.github.body(&moved);
    let (content, held) = body
        .as_str()
        .unwrap()
        .rsplit_once("\n\n<!-- onetaskgraph.metadata\n")
        .expect("the member's metadata slot");
    let mut held: Value = serde_json::from_str(held.strip_suffix("\n-->").unwrap()).unwrap();
    held["onetaskgraph.repositories"] = json!(["github.com/synthco/secret-gadgets"]);
    plan.github.edit_body(
        &moved,
        &format!("{content}\n\n<!-- onetaskgraph.metadata\n{held}\n-->"),
    );
    let before = plan.github.body(&document);
    let id = format!("board:{document}");
    let version = shown_version(&plan, &id);
    let screened = plan.boundary_calls().len();
    let (refused, calls) = set_expecting(&plan, &id, "plan.second", &version);
    assert_ne!(refused.status.code(), Some(0), "{}", stdout(&refused));
    assert_eq!(sent(&calls), [GUARDED_READ], "nothing was written");
    assert_eq!(plan.github.body(&document), before);
    let asked = plan.boundary_calls()[screened..].to_vec();
    assert!(
        asked
            .iter()
            .any(|call| call.command == BoundaryCommand::Inspect
                && call.input["repository"] == "github.com/synthco/secret-gadgets"),
        "the new command asked about the repository the member moved to: {asked:?}"
    );
    let failure: Value = serde_json::from_str(&stdout(&refused)).expect("a failure document");
    assert_eq!(
        failure["failure"]["kind"], "not-private-destination",
        "{failure}"
    );
}

#[test]
fn a_screened_approval_of_a_project_holding_more_members_than_one_page_lists_them_whole() {
    // The project holds more members than the page its document's read carries: that page is
    // no answer about all of them, so the write lists the project's members whole before it is
    // held to them, and still lands.
    let (plan, project, document) = copied_screened();
    for n in 1..=20 {
        plan.github
            .file_under(&format!("BG-{n}"), Some(project.as_str()));
    }
    let id = format!("board:{document}");
    let version = shown_version(&plan, &id);
    let (landed, calls) = set_expecting(&plan, &id, KEY, &version);
    assert_eq!(landed.status.code(), Some(0), "{}", stderr(&landed));
    let operations = sent(&calls);
    assert_eq!(operations.first().map(String::as_str), Some(GUARDED_READ));
    assert!(
        operations
            .iter()
            .any(|operation| operation == "reading a project's tasks"),
        "the members were listed whole: {operations:?}"
    );
    assert_eq!(slot(&plan, &document)[KEY], json!(true));
}
