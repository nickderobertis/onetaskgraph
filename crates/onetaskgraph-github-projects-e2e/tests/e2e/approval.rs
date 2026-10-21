//! What approving a plan on a board costs: the store calls an approval makes, each through the
//! real binary against the loopback board of `plan_copy` holding 2,000 other items, with the
//! board source's cache on — `project show`, `task list --project`, `document list --project`,
//! `document show`, then `document metadata set` carrying the version that read reported.
//!
//! Each call's requests are priced by the plugin's own point-cost table, the one
//! `tests/point_cost.rs` pins and the credentialed lane reconciles against GitHub, and what the
//! sequence spent is recorded as the `github-approval-calls-points` budget's telemetry, which
//! `budget_runner::report_approval_calls` hands to onebudgetspec. Nothing here holds the total
//! to a bound: that is the budget's. What is held here is what each call sends, that the
//! records come back whole and in order, that the binary waits for nothing on the way, and that
//! a write expecting a version the store has seen superseded writes nothing.

use super::asset_board::{Call, Kind};
use super::plan_copy::{Plan, SMALL, TASKS, counterparts, never_walks, operation, success};
use onetaskgraph_e2e_support::common::{stderr, stdout};
use onetaskgraph_github_projects::worst_case_point_cost;
use serde_json::{Value, json};

pub(super) const BUDGET: &str = "github-approval-calls-points";

/// Named in the recording, so the runner refuses telemetry another journey wrote.
pub(super) const WORKLOAD: &str = "plan approval store calls";

/// The calls an approval makes, in the order it makes them, by what each is.
pub(super) const CALLS: [&str; 5] = [
    "project show",
    "task list --project",
    "document list --project",
    "document show",
    "document metadata set --expect",
];

/// The metadata key approval writes.
const KEY: &str = "plan.approval";

/// One call of the sequence: what it printed, and the requests the board answered for it.
struct Step {
    what: &'static str,
    report: Value,
    calls: Vec<Call>,
}

/// The plan copied onto a board of [`SMALL`] other items with the cache on, and the board ids
/// of its project and its document.
fn copied() -> (Plan, String, String) {
    let plan = Plan::configured(SMALL, Vec::new(), Some(json!({"enabled": true})));
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

/// The version `document show --json` reported.
fn version_of(shown: &Value) -> String {
    shown["items"][0]["item"]["version"]
        .as_str()
        .expect("a document read through the cache reports its version")
        .to_owned()
}

/// The approval sequence, as approval sends it.
fn approve(plan: &Plan, project: &str, document: &str) -> Vec<Step> {
    let project = format!("board:{project}");
    let document = format!("board:{document}");
    let mut steps = Vec::new();
    for what in &CALLS[..4] {
        let arguments: Vec<&str> = match *what {
            "project show" => vec!["project", "show", &project, "--json"],
            "task list --project" => vec![
                "task",
                "list",
                "--source",
                "board",
                "--project",
                &project,
                "--json",
            ],
            "document list --project" => vec![
                "document",
                "list",
                "--source",
                "board",
                "--project",
                &project,
                "--json",
            ],
            _ => vec!["document", "show", &document, "--json"],
        };
        let (report, calls) = plan.measured(&arguments);
        steps.push(Step {
            what,
            report,
            calls,
        });
    }
    let version = version_of(&steps[3].report);
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
    });
    steps
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

/// Record what the sequence spent as the budget's telemetry: the points, and each call's
/// requests and points beside it.
fn record(steps: &[Step]) {
    let breakdown: Vec<Value> = steps
        .iter()
        .map(|step| {
            json!({"call": step.what,
                   "requests": step.calls.len(),
                   "points": step.calls.iter().map(points).sum::<u64>(),
                   "operations": step.calls.iter().map(operation).collect::<Vec<_>>()})
        })
        .collect();
    let total: u64 = steps.iter().flat_map(|step| &step.calls).map(points).sum();
    let directory = super::budget_runner::directory();
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        onetaskgraph_e2e_support::telemetry::file_in(&directory, BUDGET),
        serde_json::to_vec(&json!({"budget": BUDGET, "value": total,
            "detail": json!({"calls": breakdown}).to_string(),
            "workload": WORKLOAD, "board": SMALL, "tasks": TASKS, "documents": 1,
            "calls": breakdown}))
        .unwrap(),
    )
    .unwrap();
}

#[test]
fn approving_a_plan_lists_each_kind_in_one_point_and_writes_its_approval_in_one_request() {
    let (plan, project, document) = copied();
    let waits_before = plan.clock.slept().len();
    let steps = approve(&plan, &project, &document);
    let waits: Vec<_> = plan.clock.slept()[waits_before..].to_vec();
    assert!(
        waits.is_empty(),
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
        let operations: Vec<String> = listing.calls.iter().map(operation).collect();
        assert_eq!(
            operations,
            vec!["reading a project's tasks".to_owned()],
            "{what} sent {operations:?}"
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
    let documents = &step("document list --project").report["items"];
    assert_eq!(documents[0]["item"]["id"], json!(document));
    assert_eq!(
        documents[0]["item"],
        step("document show").report["items"][0]["item"]
    );

    // The write, given the version the read reported, is the one body update and nothing else.
    let write = step("document metadata set --expect");
    let operations: Vec<String> = write.calls.iter().map(operation).collect();
    assert_eq!(operations.len(), 1, "the write sent {operations:?}");
    assert!(
        operations[0].starts_with("updating an issue"),
        "the write sent {operations:?}"
    );
    assert_eq!(write.report["fallback"], Value::Null, "{}", write.report);
    assert_eq!(
        write.report["result"]["parts"][0]["outcome"],
        json!("landed"),
        "{}",
        write.report
    );
    assert_eq!(slot(&plan, &document)[KEY], json!("approved"));

    record(&steps);
}

#[test]
fn a_document_write_expecting_a_superseded_version_is_refused_and_writes_nothing() {
    let (plan, _, document) = copied();
    let id = format!("board:{document}");
    let first = version_of(&success(&plan.run(&["document", "show", &id, "--json"])));

    // A write at the version read lands, and leaves the document at a later one.
    let set = |key: &str, version: &str| {
        plan.measured_output(&[
            "document", "metadata", "set", &id, key, "true", "--expect", version, "--json",
        ])
    };
    let (landed, calls) = set("plan.first", &first);
    assert_eq!(landed.status.code(), Some(0), "{}", stderr(&landed));
    assert_eq!(
        calls.len(),
        1,
        "{:?}",
        calls.iter().map(operation).collect::<Vec<_>>()
    );
    let after_write = plan.github.body(&document);

    // That write took the store's entry out of service, so the stale version is read once and
    // refused on what the read answered.
    let (refused, calls) = set("plan.second", &first);
    assert_ne!(refused.status.code(), Some(0), "{}", stdout(&refused));
    let operations: Vec<String> = calls.iter().map(operation).collect();
    assert_eq!(operations, vec!["reading one issue".to_owned()]);
    let message = stderr(&refused);
    assert!(message.contains(&first), "{message}");
    assert!(message.contains("nothing was written"), "{message}");
    assert!(message.contains("document show"), "{message}");
    assert_eq!(plan.github.body(&document), after_write);
    assert!(slot(&plan, &document).get("plan.second").is_none());

    // Read again, the store holds the later version, and the stale one is refused without a
    // request at all.
    let current = version_of(&success(&plan.run(&["document", "show", &id, "--json"])));
    assert_ne!(current, first);
    let (refused, calls) = set("plan.second", &first);
    assert_ne!(refused.status.code(), Some(0), "{}", stdout(&refused));
    assert!(
        calls.is_empty(),
        "{:?}",
        calls.iter().map(operation).collect::<Vec<_>>()
    );
    let message = stderr(&refused);
    assert!(
        message.contains(&current) && message.contains(&first),
        "{message}"
    );
    assert_eq!(plan.github.body(&document), after_write);

    // A person's edit, once a read has shown it to the store, supersedes the version too.
    plan.github.edit_body(&document, "Rewritten by a person.");
    let edited = plan.github.body(&document);
    let now = version_of(&success(&plan.run(&["document", "show", &id, "--json"])));
    assert_ne!(now, current);
    let (refused, calls) = set("plan.second", &current);
    assert_ne!(refused.status.code(), Some(0), "{}", stdout(&refused));
    assert!(
        calls.is_empty(),
        "{:?}",
        calls.iter().map(operation).collect::<Vec<_>>()
    );
    assert_eq!(plan.github.body(&document), edited);

    // And the version the store holds now is written in one request, over the person's edit.
    let (landed, calls) = set("plan.second", &now);
    assert_eq!(landed.status.code(), Some(0), "{}", stderr(&landed));
    assert_eq!(calls.len(), 1);
    assert_eq!(slot(&plan, &document)["plan.second"], json!(true));
    assert!(
        plan.github
            .body(&document)
            .as_str()
            .unwrap()
            .starts_with("Rewritten by a person."),
        "the write kept the person's edit"
    );
}

#[test]
fn a_document_write_whose_version_the_cache_does_not_hold_reads_once_and_says_so() {
    let (plan, _, document) = copied();
    let id = format!("board:{document}");
    let version = version_of(&success(&plan.run(&["document", "show", &id, "--json"])));
    assert_eq!(
        plan.run(&["cache", "clear", "board"]).status.code(),
        Some(0)
    );
    let (output, calls) = plan.measured_output(&[
        "document",
        "metadata",
        "set",
        &id,
        KEY,
        "\"approved\"",
        "--expect",
        &version,
        "--json",
    ]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let operations: Vec<String> = calls.iter().map(operation).collect();
    assert_eq!(operations.len(), 2, "{operations:?}");
    assert_eq!(operations[0], "reading one issue");
    let answer: Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(answer["fallback"]["read"], json!(true), "{answer}");
    assert_eq!(slot(&plan, &document)[KEY], json!("approved"));

    // A project's metadata still takes no version.
    let refused = plan.run(&[
        "project", "metadata", "set", &id, KEY, "true", "--expect", &version,
    ]);
    assert_ne!(refused.status.code(), Some(0), "{}", stdout(&refused));
    assert!(stderr(&refused).contains("leave --expect out"));
}

/// The one document entry the board source's cache holds, read whole.
fn document_entry(plan: &Plan) -> std::path::PathBuf {
    fn files(directory: &std::path::Path, found: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                files(&path, found);
            } else if path
                .extension()
                .is_some_and(|extension| extension == "json")
            {
                found.push(path);
            }
        }
    }
    let mut found = Vec::new();
    files(
        &plan.sandbox.subdirectory("board-cache").join(format!(
            "documents-v{}",
            onetaskgraph_core::cache::KIND_ENTRY_SCHEMA
        )),
        &mut found,
    );
    assert_eq!(found.len(), 1, "{found:?}");
    found.remove(0)
}

#[test]
fn a_document_entry_without_a_context_the_source_takes_is_read_once_before_the_write() {
    for (case, context) in [
        ("an entry holding no context", None),
        (
            "a context the source cannot take",
            Some(json!({"version": 0})),
        ),
    ] {
        let (plan, _, document) = copied();
        let id = format!("board:{document}");
        let version = version_of(&success(&plan.run(&["document", "show", &id, "--json"])));
        let path = document_entry(&plan);
        let mut entry: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert!(
            entry.get("context").is_some(),
            "{case}: the read kept a context"
        );
        match &context {
            Some(context) => entry["context"] = context.clone(),
            None => {
                entry.as_object_mut().unwrap().remove("context");
            }
        }
        std::fs::write(&path, serde_json::to_vec(&entry).unwrap()).unwrap();
        let (output, calls) = plan.measured_output(&[
            "document", "metadata", "set", &id, KEY, "true", "--expect", &version, "--json",
        ]);
        assert_eq!(output.status.code(), Some(0), "{case}: {}", stderr(&output));
        let operations: Vec<String> = calls.iter().map(operation).collect();
        assert_eq!(operations.len(), 2, "{case}: {operations:?}");
        assert_eq!(operations[0], "reading one issue", "{case}");
        let answer: Value = serde_json::from_str(&stdout(&output)).unwrap();
        assert_eq!(answer["fallback"]["read"], json!(true), "{case}: {answer}");
        assert_eq!(slot(&plan, &document)[KEY], json!(true), "{case}");
    }
}

#[test]
fn a_document_write_whose_kept_id_went_stale_reads_and_is_sent_once_more() {
    let (plan, _, document) = copied();
    let id = format!("board:{document}");
    let version = version_of(&success(&plan.run(&["document", "show", &id, "--json"])));
    plan.github
        .refuse_as_stale_once("updateIssue(input:$input)");
    let (output, calls) = plan.measured_output(&[
        "document", "metadata", "set", &id, KEY, "true", "--expect", &version, "--json",
    ]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let operations: Vec<String> = calls.iter().map(operation).collect();
    assert_eq!(operations.len(), 3, "{operations:?}");
    assert_eq!(operations[1], "reading one issue", "{operations:?}");
    let answer: Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert!(
        answer["fallback"]["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("stale")),
        "{answer}"
    );
    let parts = answer["result"]["parts"].as_array().unwrap();
    assert_eq!(parts.len(), 2, "{answer}");
    assert_eq!(parts[1]["outcome"], json!("landed"), "{answer}");
    assert_eq!(slot(&plan, &document)[KEY], json!(true));
}

#[cfg(unix)]
#[test]
fn a_document_write_whose_entry_cannot_be_retired_is_refused_unsent() {
    use std::os::unix::fs::PermissionsExt;
    let (plan, _, document) = copied();
    let id = format!("board:{document}");
    let version = version_of(&success(&plan.run(&["document", "show", &id, "--json"])));
    let before = plan.github.body(&document);
    let entries = document_entry(&plan).parent().unwrap().to_path_buf();
    std::fs::set_permissions(&entries, std::fs::Permissions::from_mode(0o555)).unwrap();
    let path = document_entry(&plan);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
    let (output, calls) = plan.measured_output(&[
        "document", "metadata", "set", &id, KEY, "true", "--expect", &version, "--json",
    ]);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    std::fs::set_permissions(&entries, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_ne!(output.status.code(), Some(0), "{}", stdout(&output));
    assert!(
        stderr(&output).contains("was not sent"),
        "{}",
        stderr(&output)
    );
    assert!(
        calls
            .iter()
            .all(|call| !operation(call).starts_with("updating an issue")),
        "{:?}",
        calls.iter().map(operation).collect::<Vec<_>>()
    );
    assert_eq!(plan.github.body(&document), before);
}
