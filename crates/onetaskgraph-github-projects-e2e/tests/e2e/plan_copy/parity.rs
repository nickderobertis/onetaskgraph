//! spike-host-copy-parity's harness: one measurement of one named variant of the plan copy,
//! on the axes this host's copy differs from the scenario on. Throwaway measurement code.
//!
//! `spike/host-copy-parity/measure.sh <variant> [items]` runs [`one_variant`] with
//! `SPIKE_PARITY_VARIANT` and `SPIKE_PARITY_ITEMS` set; with neither set it measures nothing,
//! so the crate's ordinary run is unchanged. A variant is `scenario` or `+`-joined axes:
//! `private` (the notes source declared `visibility: private`), `cache` (the board's
//! `cache: {enabled: true}`, as ai-orchestrator's onetaskgraph.yaml declares it), `screened`
//! (write screening on), `host-shape` (the plan written and revised as the host writes it) and
//! `instant` (the board answering at once, as the host's Python double does); `host` is all
//! five.

use super::{Axes, Plan, SMALL, TASKS, Scenario, cost, counterparts, operation, tally};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// What a variant name lays: the board's cache block, screening, and the other axes.
fn parse(variant: &str) -> (Option<Value>, bool, Axes) {
    let mut cache = None;
    let mut screened = false;
    let mut axes = Axes::default();
    for axis in variant.split('+') {
        match axis {
            "scenario" => {}
            "private" => axes.private_source = true,
            "cache" => cache = Some(json!({"enabled": true})),
            "screened" => screened = true,
            "host-shape" => axes.host_shape = true,
            "instant" => axes.instant_double = true,
            "preserve" => axes.preserve_status = true,
            "reviewed" => axes.reviewed = true,
            "approved" => axes.approved = true,
            "mapped" => axes.status_mapping = true,
            "one-edge" => axes.one_edge = true,
            "cold-cache" => axes.cold_cache = true,
            "host-listing" => axes.host_double_listing = true,
            "host" => {
                axes = Axes {
                    private_source: true,
                    host_shape: true,
                    instant_double: true,
                    preserve_status: true,
                    reviewed: true,
                    approved: true,
                    status_mapping: true,
                    one_edge: true,
                    ..Axes::default()
                };
                cache = Some(json!({"enabled": true}));
                screened = true;
            }
            other => panic!(
                "unknown axis {other:?}: name scenario, host, or +-joined private, cache, \
                 screened, host-shape, instant, preserve, reviewed, approved, mapped, one-edge, cold-cache, host-listing"
            ),
        }
    }
    (cache, screened, axes)
}

/// One scenario's figures, as printed.
fn figures(name: &str, scenario: &Scenario) -> Value {
    let cost = cost(scenario);
    let mut points: BTreeMap<String, u64> = BTreeMap::new();
    for call in &scenario.calls {
        *points.entry(operation(call)).or_default() += super::spike_points(call);
    }
    json!({
        "scenario": name,
        "points": cost.points,
        "writes": cost.writes,
        "simulated_seconds": (cost.simulated * 1000.0).round() / 1000.0,
        "requests": scenario.calls.len(),
        "requests_by_operation": tally(&scenario.calls),
        "points_by_operation": points,
        "by_phase": scenario.boundary.iter().map(|phase| json!({
            "command": phase.kind,
            "operations": phase.calls.iter().map(operation).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })
}

/// The first copy, the re-copy after task 4 changed and the unchanged re-copy of one variant.
pub(super) fn measure(variant: &str, items: usize) -> Value {
    let (cache, screened, axes) = parse(variant);
    let plan = Plan::variant(items, cache, screened, axes);
    plan.place_as_measured();
    let first = plan.copy_timed();
    let at = counterparts(&first.project, &first.document);
    plan.approve(&at["D"]);
    plan.revise(4);
    let changed = plan.copy_timed();
    assert_eq!(counterparts(&changed.project, &changed.document), at);
    let unchanged = plan.copy_timed();
    assert_eq!(counterparts(&unchanged.project, &unchanged.document), at);
    json!({
        "variant": variant,
        "items": items,
        "tasks": TASKS,
        "scenarios": [
            figures("first", &first),
            figures("changed", &changed),
            figures("unchanged", &unchanged),
        ],
    })
}

#[test]
fn one_variant() {
    let Ok(variant) = std::env::var("SPIKE_PARITY_VARIANT") else {
        return;
    };
    let items = std::env::var("SPIKE_PARITY_ITEMS")
        .ok()
        .map_or(SMALL, |items| items.parse().expect("SPIKE_PARITY_ITEMS is a count"));
    let measured = measure(&variant, items);
    println!("SPIKE-PARITY {}", serde_json::to_string(&measured).unwrap());
}
