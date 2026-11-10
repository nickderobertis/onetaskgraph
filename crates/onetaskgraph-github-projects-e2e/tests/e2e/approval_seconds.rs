//! What recording a design approval costs a person: the plan the copy budgets measure —
//! a project, eight tasks and one design document carrying one image — drafted in a folder of
//! Markdown declared private and copied, as this host copies one, onto a public board holding
//! 2,000 other items, every public write screened by the write policy; then one attestation
//! of kind `design-approval` recorded on the board copy of the design document through the
//! real binary.
//!
//! The board holds each request for the timing spike's measured latency of its kind, on the
//! simulated clock, so the seconds the attestation took are the part of a person's wait the
//! store's requests decide — reproducible on any runner, the binary's own processing left out.
//! What it cost is recorded as the `plan-approval-seconds` budget's telemetry, with each request
//! kind's count and modelled latency as its detail, which `budget_runner::report_plan_approval`
//! hands to onebudgetspec; nothing here holds the figure to a bound.
//!
//! What is held here is what the approval is: one read of the document and one write, screened
//! before it is sent; a record keyed on the document's content that checks `valid` against the
//! bar it was given and `bar-moved` against another; and a check that is a read alone.

use super::asset_board::{Call, Kind};
use super::plan_copy::{Plan, SMALL, WORKLOAD, counterparts};
use onetaskgraph_e2e_support::common::{stderr, stdout};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// The budget this journey records.
pub(super) const BUDGET: &str = "plan-approval-seconds";

/// The kind of attestation the workload records.
pub(super) const KIND: &str = "design-approval";

/// The bar the approval clears the design against: a fingerprint of the caller's own.
const BAR: &str = "sha256:5f1d8a3b1c0e7f2a9d4b6c8e0a1f3b5d7c9e1a2b4c6d8e0f1a3b5c7d9e1f2a3b";

/// Each request kind the approval sent, as `{count, modelled_millis}` and their product.
fn by_kind(calls: &[Call]) -> BTreeMap<String, Value> {
    let mut counts: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    for call in calls {
        let entry = counts
            .entry(format!("{:?}", call.kind))
            .or_insert((0, call.kind.measured_millis()));
        entry.0 += 1;
    }
    counts
        .into_iter()
        .map(|(kind, (count, millis))| {
            (
                kind,
                json!({"count": count, "modelled_millis": millis, "total_millis": count * millis}),
            )
        })
        .collect()
}

/// The recording the budget reads.
pub(super) fn recording(seconds: f64, kinds: &BTreeMap<String, Value>) -> Value {
    let detail = kinds
        .iter()
        .map(|(kind, figures)| {
            format!(
                "{kind}: {} x {} ms",
                figures["count"], figures["modelled_millis"]
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    json!({
        "budget": BUDGET,
        "value": seconds,
        "detail": format!(
            "One {KIND} attestation on the board copy of the {WORKLOAD}'s design document, \
             drafted in a private folder and copied onto a public board of {SMALL} items with \
             every public write screened; simulated seconds by request kind: {detail}"
        ),
        "plan": WORKLOAD,
        "board": SMALL,
        "private": true,
        "screened": true,
        "items": 10,
        "images": 1,
        "kind": KIND,
        "requests": kinds,
    })
}

#[test]
fn a_design_approval_on_a_copied_plan_is_one_read_and_one_screened_write() {
    let plan = Plan::host(SMALL, Vec::new(), true);
    let (project, document, _) = plan.copy();
    let at = counterparts(&project, &document);
    assert_eq!(at.len(), 10, "the plan is ten items: {at:?}");
    let design = format!("board:{}", at["D"]);
    let shown = super::plan_copy::success(&plan.run(&["document", "show", &design, "--json"]));
    assert_eq!(
        shown["assets"].as_array().map(Vec::len),
        Some(1),
        "the design document carries one image: {shown}"
    );

    // Approved at the version the person read, as the approval's own read is held to it.
    let version = shown["items"][0]["item"]["version"]
        .as_str()
        .expect("a board document reports its version")
        .to_owned();
    let screened_before = plan.boundary_calls().len();
    let started = plan.clock.now();
    let (attested, calls) = plan.measured(&[
        "document",
        "attest",
        &design,
        "--kind",
        KIND,
        "--by",
        "ada",
        "--bar",
        BAR,
        "--note",
        "approved as drafted",
        "--if-version",
        &version,
        "--json",
    ]);
    let seconds = (plan.clock.now() - started).as_secs_f64();
    assert_eq!(attested["id"], json!(design));
    let record = &attested["record"];
    assert_eq!(
        (
            &record["kind"],
            &record["by"],
            &record["bar"],
            &record["note"]
        ),
        (
            &json!(KIND),
            &json!("ada"),
            &json!(BAR),
            &json!("approved as drafted")
        )
    );
    // The write went through the write policy, as every public write of the plan did.
    assert!(
        plan.boundary_calls().len() > screened_before,
        "the approval's write was screened before it was sent"
    );
    let kinds: Vec<Kind> = calls.iter().map(|call| call.kind).collect();
    assert_eq!(
        kinds,
        [Kind::ItemRead, Kind::Mutation],
        "one read of the document, then one write: {:?}",
        by_kind(&calls)
    );

    // Held to a version the board has moved past, it is refused writing nothing.
    let (refused, sent) = plan.measured_output(&[
        "document",
        "attest",
        &design,
        "--kind",
        KIND,
        "--by",
        "ada",
        "--bar",
        BAR,
        "--if-version",
        &version,
        "--json",
    ]);
    assert_eq!(refused.status.code(), Some(1), "{}", stderr(&refused));
    assert!(
        stderr(&refused).contains("is at version"),
        "{}",
        stderr(&refused)
    );
    assert!(
        sent.iter()
            .all(|call| !matches!(call.kind, Kind::Mutation | Kind::AliasedMutations)),
        "a refused approval writes nothing"
    );

    // The record holds against the bar it was given, and a check is a read alone.
    let (checked, check_calls) = plan.measured(&[
        "document",
        "attestation",
        "check",
        &design,
        "--kind",
        KIND,
        "--bar",
        BAR,
        "--json",
    ]);
    assert_eq!(checked["state"], "valid", "{checked}");
    assert_eq!(&checked["record"], record);
    assert!(
        check_calls
            .iter()
            .all(|call| !matches!(call.kind, Kind::Mutation | Kind::AliasedMutations)),
        "a check writes nothing"
    );
    let (output, _) = plan.measured_output(&[
        "document",
        "attestation",
        "check",
        &design,
        "--kind",
        KIND,
        "--bar",
        "sha256:other",
        "--json",
    ]);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let moved: Value = serde_json::from_str(&stdout(&output)).expect("JSON");
    assert_eq!(
        (&moved["state"], &moved["reason"]),
        (&json!("void"), &json!("bar-moved"))
    );

    let kinds = by_kind(&calls);
    let modelled: u64 = kinds
        .values()
        .map(|figures| figures["total_millis"].as_u64().unwrap())
        .sum();
    assert!(
        seconds * 1000.0 + 1.0 >= modelled as f64,
        "the simulated wait is at least the modelled latency of what was sent: {seconds} s, \
         {kinds:?}"
    );
    let directory = super::budget_runner::directory();
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        onetaskgraph_e2e_support::telemetry::file_in(&directory, BUDGET),
        serde_json::to_vec(&recording(seconds, &kinds)).unwrap(),
    )
    .unwrap();
}
