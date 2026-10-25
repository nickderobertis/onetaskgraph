//! What approving a plan on a board costs: the store calls an approval makes, each through the
//! real binary against the loopback board of `plan_copy` holding 2,000 other items — `project
//! show`, `task list --project`, `document list --project`, then `document metadata set
//! --expect` carrying the version that listing reported for the design document. The listing
//! carries the document whole, exactly as `document show` would answer it, so approval reads it
//! once.
//!
//! Each call's requests are priced by the plugin's own point-cost table, the one
//! `tests/point_cost.rs` pins and the credentialed lane reconciles against GitHub, and what the
//! sequence spent is recorded as the `github-approval-calls-points` budget's telemetry, which
//! `budget_runner::report_approval_calls` hands to onebudgetspec. Nothing here holds the total
//! to a bound: that is the budget's. What is held here is what each call sends, that the
//! records come back whole and in order, that the binary waits for nothing on the way, and that
//! a write expecting a version the board has moved past — whoever moved it, and whether or not
//! anything here saw it — writes nothing.

use super::asset_board::{Call, Kind};
use super::plan_copy::{Plan, SMALL, TASKS, counterparts, never_walks, operation, success};
use onetaskgraph_e2e_support::common::{stderr, stdout};
use onetaskgraph_e2e_support::onevcs::BoundaryCall;
use onetaskgraph_github_projects::worst_case_point_cost;
use serde_json::{Value, json};

pub(super) const BUDGET: &str = "github-approval-calls-points";

/// Named in the recording, so the runner refuses telemetry another journey wrote.
pub(super) const WORKLOAD: &str = "plan approval store calls";

/// The calls an approval makes, in the order it makes them, by what each is.
pub(super) const CALLS: [&str; 4] = [
    "project show",
    "task list --project",
    "document list --project",
    "document metadata set --expect",
];

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

fn approve(plan: &Plan, project: &str, document: &str) -> Vec<Step> {
    let project = format!("board:{project}");
    let document = format!("board:{document}");
    let mut steps = Vec::new();
    for what in &CALLS[..3] {
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
        what: CALLS[3],
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

/// Each call of `steps`: its requests, its points and its operations.
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

/// Record what the sequence spent as the budget's telemetry: the larger of the run with no
/// write policy and the run with write screening on, each call's requests and points beside
/// it, and both runs in the detail.
fn record(unscreened: &[Step], screened: &[Step]) {
    let total =
        |steps: &[Step]| -> u64 { steps.iter().flat_map(|step| &step.calls).map(points).sum() };
    let larger = if total(screened) > total(unscreened) {
        screened
    } else {
        unscreened
    };
    let directory = super::budget_runner::directory();
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        onetaskgraph_e2e_support::telemetry::file_in(&directory, BUDGET),
        serde_json::to_vec(&json!({"budget": BUDGET, "value": total(larger),
            "detail": json!({"unscreened": {"points": total(unscreened), "calls": breakdown(unscreened)},
                             "screened": {"points": total(screened), "calls": breakdown(screened)}})
                .to_string(),
            "workload": WORKLOAD, "board": SMALL, "tasks": TASKS, "documents": 1,
            "calls": breakdown(larger)}))
        .unwrap(),
    )
    .unwrap();
}

#[test]
fn approving_a_plan_lists_each_kind_in_one_point_and_writes_its_approval_behind_one_read() {
    let (plan, project, document) = copied(false);
    let waits_before = plan.clock.slept().len();
    let steps = approve(&plan, &project, &document);
    let waits: Vec<_> = plan.clock.slept()[waits_before..].to_vec();
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

    // Each listing is one request, priced at one point, carrying the project's whole page.
    // llmlint: ignore-block[tests_hold_no_nonfunctional_thresholds] The node's acceptance criteria require this journey to show each listing is one request priced at one point by the plugin's own table: the point is the functional outcome of MEMBER_PAGE_SIZE, which `tests/point_cost.rs` pins, rather than a budget figure. The sequence's total is recorded below for onebudgetspec, which alone judges it against `github-approval-calls-points`.
    for (what, records) in [
        ("task list --project", TASKS),
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

    // The tasks come back in the order they were filed under the project, each with what a read
    // of it by its own id reports.
    let listed = rows(&step("task list --project").report);
    let titles: Vec<String> = listed.iter().map(|(_, title)| title.clone()).collect();
    let filed: Vec<String> = (1..=TASKS)
        .map(|n| format!("Step {n} of the plan"))
        .collect();
    assert_eq!(titles, filed);
    for row in step("task list --project").report["items"]
        .as_array()
        .unwrap()
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
    let write = &screened[3];
    assert_eq!(
        sent(&write.calls),
        GUARDED_WRITE,
        "{:?}",
        sent(&write.calls)
    );
    let checks: Vec<&BoundaryCall> = write
        .screening
        .iter()
        .filter(|call| call.command == "check")
        .collect();
    assert_eq!(checks.len(), 1, "{:?}", write.screening);
    let updated = write
        .calls
        .iter()
        .find(|call| call.creating)
        .expect("the write");
    assert!(checks[0].finished < updated.wall, "checked before it wrote");
    for step in &screened[..3] {
        assert!(step.screening.is_empty(), "{} screened nothing", step.what);
    }
    assert_eq!(
        slot(&screened_plan, &screened_document)[KEY],
        json!("approved")
    );

    record(&steps, &screened);
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
        first[3]
            .screening
            .iter()
            .all(|call| call.command != "inspect"
                || call.input["repository"] != "github.com/synthco/secret-gadgets"),
        "{:?}",
        first[3].screening
    );
    plan.screening
        .as_mut()
        .expect("a screened plan")
        .0
        .private("synthco", "secret-gadgets");
    let members = rows(&first[1].report);
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
        asked.iter().any(|call| call.command == "inspect"
            && call.input["repository"] == "github.com/synthco/secret-gadgets"),
        "the new command asked about the repository the member moved to: {asked:?}"
    );
    let failure: Value = serde_json::from_str(&stdout(&refused)).expect("a failure document");
    assert_eq!(
        failure["failure"]["kind"], "not-private-destination",
        "{failure}"
    );
}
