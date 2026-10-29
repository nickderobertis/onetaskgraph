//! The two human-items reporters, driven as onebudgetspec drives them — `node` in this crate's
//! directory, `ONEBUDGETSPEC_RESULT` naming where the figure goes — over recordings built by the
//! very functions the journeys record with. So a reporter handing over the wrong figure, taking
//! a recording it should refuse, or drifting from what the journeys write fails here.

use std::path::Path;

use onetaskgraph_e2e_support::common::Sandbox;
use serde_json::{Value, json};

/// Run `script` over `recording`, saved as `budget`'s telemetry: its exit, and the result file.
fn report(script: &str, budget: &str, recording: &Value) -> (bool, Option<Value>) {
    let sandbox = Sandbox::new();
    let directory = sandbox.subdirectory("telemetry");
    std::fs::write(
        onetaskgraph_e2e_support::telemetry::file_in(&directory, budget),
        recording.to_string(),
    )
    .unwrap();
    let result = sandbox.project().join("result.json");
    let output = std::process::Command::new("node")
        .arg(Path::new("human-items").join(script))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env("GITHUB_ASSET_TELEMETRY_DIR", &directory)
        .env("ONEBUDGETSPEC_RESULT", &result)
        .output()
        .expect("node runs the reporter");
    let written = std::fs::read_to_string(&result)
        .ok()
        .map(|text| serde_json::from_str(&text).unwrap());
    (output.status.success(), written)
}

/// One phase's row of `requests` reads, priced a point each.
fn row(phase: &str, requests: usize) -> Value {
    let priced: Vec<Value> = (0..requests)
        .map(|_| {
            json!({"operation": "reading one issue", "points": 1, "mutation_fields": 0,
                   "secondary_points": 1})
        })
        .collect();
    json!({"phase": phase, "requests": requests, "priced": priced})
}

#[test]
fn the_projection_reporter_hands_over_points_per_item_and_refuses_an_incomplete_recording() {
    let budget = super::human_cost::BUDGET;
    let rows = vec![
        row("create", 510),
        row("terminal", 200),
        row("create_recovery", 10),
        row("cold_terminal_retry", 10),
        row("warm_noop", 0),
    ];
    let recorded = super::human_cost::recording(&rows);
    let (succeeded, written) = report("report.mjs", budget, &recorded);
    assert!(succeeded);
    let written = written.expect("a figure");
    assert_eq!(written["value"], json!(7.3));
    let detail: Value = serde_json::from_str(written["detail"].as_str().unwrap()).unwrap();
    assert_eq!(detail["totals"]["points"], 730);
    assert_eq!(detail["phases"].as_array().unwrap().len(), 5);

    let with = |pointer: &str, replaced: Value| {
        let mut changed = recorded.clone();
        *changed.pointer_mut(pointer).unwrap() = replaced;
        changed
    };
    for refused in [
        with("/workload", json!("another workload")),
        with("/tasks", json!(99)),
        with("/terminals/completed", json!(33)),
        with("/recoveries/create", json!(0)),
        with("/totals/points", json!(700)),
        super::human_cost::recording(&rows[..3]),
    ] {
        assert_eq!(
            report("report.mjs", budget, &refused),
            (false, None),
            "{refused}"
        );
    }
}

#[test]
fn the_approval_reporter_hands_over_the_larger_sequence_of_the_journeys_calls() {
    let budget = super::approval::BUDGET;
    let calls = |extra: u64| -> Vec<Value> {
        super::approval::CALLS
            .iter()
            .map(|call| {
                let points = if call.starts_with("task project-human") {
                    4
                } else {
                    1
                } + extra;
                json!({"call": call, "requests": points, "points": points})
            })
            .collect()
    };
    let recorded = super::approval::recording(
        super::approval::spent_of(calls(0)),
        super::approval::spent_of(calls(1)),
    );
    let (succeeded, written) = report("approval-report.mjs", budget, &recorded);
    assert!(succeeded);
    let written = written.expect("a figure");
    assert_eq!(
        written["value"],
        json!(18),
        "the screened sequence is the larger"
    );
    let detail: Value = serde_json::from_str(written["detail"].as_str().unwrap()).unwrap();
    assert_eq!(detail["unscreened"]["existing"], 4);
    assert_eq!(detail["unscreened"]["projection"], 8);

    let mut reordered = calls(0);
    reordered.swap(1, 2);
    let mut unpriced = recorded.clone();
    unpriced["screened"]["projection"] = json!(0);
    let mut uncounted = recorded.clone();
    uncounted["unscreened"]["calls"][2]["requests"] = json!(-1);
    for refused in [
        super::approval::recording(
            super::approval::spent_of(reordered),
            recorded["screened"].clone(),
        ),
        unpriced,
        uncounted,
        {
            let mut other = recorded.clone();
            other["workload"] = json!("a plan copy");
            other
        },
    ] {
        assert_eq!(
            report("approval-report.mjs", budget, &refused),
            (false, None),
            "{refused}"
        );
    }
}
