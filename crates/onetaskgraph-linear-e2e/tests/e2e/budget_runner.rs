//! The one runner of `budgets.yaml`, which every budget there names as its command.
//!
//! [`report`] measures nothing: it reads the telemetry the budget's journey recorded while this
//! target ran — the budget is the one `ONEBUDGETSPEC_BUDGET_ID` names, so no list of ids lives
//! here — and hands that figure to `onebudgetspec check` through `onebudgetspec_core::report`.
//! The comparison with the threshold is onebudgetspec's alone. In this target's ordinary run no
//! budget is named, so it reports nothing and passes. The journeys below drive it as each
//! budget's command does: as this test executable, through the real `onebudgetspec check`, over
//! telemetry written by the same `record` the measuring journeys use.

use std::env::VarError;
use std::path::Path;
use std::process::{Command, Output};

use serde_json::{Value, json};

use crate::common::Sandbox;
use crate::telemetry;

use crate::linear_assets::document_lifecycle::journal as transport_journal;

#[test]
fn report() {
    let budget = match std::env::var("ONEBUDGETSPEC_BUDGET_ID") {
        Ok(budget) => budget,
        Err(VarError::NotPresent) => return,
        Err(VarError::NotUnicode(raw)) => refuse(&format!(
            "ONEBUDGETSPEC_BUDGET_ID {raw:?} is not a budget id: it is not even Unicode"
        )),
    };
    let (value, detail) = telemetry::recorded(&budget).unwrap_or_else(|reason| refuse(&reason));
    let reported = onebudgetspec_core::report(value, detail.as_deref()).unwrap_or_else(|error| {
        refuse(&format!("{budget}'s figure could not be reported: {error}"))
    });
    if !reported {
        refuse(&format!(
            "ONEBUDGETSPEC_RESULT is not set, so {budget}'s figure has nowhere to go; run it \
             through `onebudgetspec check crates/onetaskgraph-linear-e2e/budgets.yaml`"
        ));
    }
}

/// End the runner with `reason` as the last of its stderr, and nothing after it.
///
/// onebudgetspec keeps only the tail of a failed command's stderr, so a panic would not do: the
/// backtrace `RUST_BACKTRACE` appends — set on every hosted lane — follows the reason and pushed
/// it out of that tail on the Windows runner, whose paths are the longest.
fn refuse(reason: &str) -> ! {
    eprintln!("{reason}");
    std::process::exit(1)
}

/// The argv that runs [`report`] alone, as each budget's command does.
pub(crate) fn runner() -> Vec<String> {
    let executable = std::env::current_exe().expect("this test executable has a path");
    vec![
        executable.to_string_lossy().into_owned(),
        "budget_runner::report".to_owned(),
        "--exact".to_owned(),
        "--nocapture".to_owned(),
    ]
}

/// [`report`], run as its own process the way onebudgetspec runs it, over `telemetry`.
fn run_report(telemetry: &Path, configure: impl FnOnce(&mut Command)) -> Output {
    let argv = runner();
    let mut command = Command::new(&argv[0]);
    command
        .args(&argv[1..])
        .env(telemetry::DIRECTORY, telemetry)
        .env_remove("ONEBUDGETSPEC_RESULT")
        .env_remove("ONEBUDGETSPEC_BUDGET_ID");
    configure(&mut command);
    command.output().expect("the runner starts")
}

#[test]
fn onebudgetspec_check_reports_what_was_recorded_and_errors_on_what_it_cannot_read() {
    let sandbox = Sandbox::new();
    let recorded = sandbox.subdirectory("telemetry");
    telemetry::record_in(
        &recorded,
        "linear-requests-recorded",
        2,
        "two requests, as the journey counted them",
    );
    std::fs::write(
        telemetry::file_in(&recorded, "linear-requests-malformed"),
        "not json",
    )
    .expect("writable");
    std::fs::write(
        telemetry::file_in(&recorded, "linear-requests-undetailed"),
        json!({"value": 1, "detail": null}).to_string(),
    )
    .expect("writable");
    std::fs::write(
        telemetry::file_in(&recorded, "linear-requests-wordless"),
        json!({"value": 1, "detail": 7}).to_string(),
    )
    .expect("writable");
    std::fs::write(
        telemetry::file_in(&recorded, "linear-requests-valueless"),
        json!({"detail": "no figure"}).to_string(),
    )
    .expect("writable");
    for (id, value) in [
        ("linear-requests-negative", json!(-1)),
        ("linear-requests-fractional", json!(0.5)),
    ] {
        std::fs::write(
            telemetry::file_in(&recorded, id),
            json!({"value": value, "detail": "not a count"}).to_string(),
        )
        .expect("writable");
    }
    let ids = [
        "linear-requests-recorded",
        "linear-requests-unrecorded",
        "linear-requests-malformed",
        "linear-requests-valueless",
        "linear-requests-negative",
        "linear-requests-fractional",
        "linear-requests-wordless",
        "linear-requests-undetailed",
    ];
    let budgets = ids
        .iter()
        .map(|id| {
            json!({"id": id, "measure": "reported", "unit": "requests", "direction": "max",
                   "threshold": 2, "command": runner()})
        })
        .collect::<Vec<_>>();
    let file = sandbox.subdirectory("budgets").join("budgets.yaml");
    // JSON is YAML, so the file is written as the JSON it is.
    std::fs::write(
        &file,
        json!({"schema_version": 1, "budgets": budgets}).to_string(),
    )
    .expect("the budgets file is writable");

    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new("bun")
        .args(["run", "onebudgetspec", "check", "--json"])
        .arg(&file)
        .current_dir(&workspace)
        .env(telemetry::DIRECTORY, &recorded)
        // The longest stderr a refusal could carry, so the reason is proven to survive it.
        .env("RUST_BACKTRACE", "full")
        .output()
        .expect(
            "bun runs the workspace's pinned onebudgetspec; run `just bootstrap` to install it",
        );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(3),
        "six budgets errored: {stderr}"
    );
    let report: Value = serde_json::from_slice(&output.stdout).expect("a JSON report");
    let results = report["results"].as_array().expect("results");
    let result = |id: &str| {
        results
            .iter()
            .find(|result| result["id"] == id)
            .unwrap_or_else(|| panic!("no result for {id}: {report:#}"))
    };

    let within = result("linear-requests-recorded");
    assert_eq!(within["verdict"], "within", "{within:#}");
    assert_eq!(within["actual"], 2.0);
    assert_eq!(
        within["detail"],
        "two requests, as the journey counted them"
    );
    let undetailed = result("linear-requests-undetailed");
    assert_eq!(undetailed["verdict"], "within", "{undetailed:#}");
    assert_eq!(undetailed["actual"], 1.0);
    assert_eq!(
        undetailed["detail"],
        Value::Null,
        "no detail is reported as none"
    );

    for (id, reason) in [
        ("linear-requests-unrecorded", "no telemetry at "),
        ("linear-requests-malformed", "is not JSON"),
        ("linear-requests-valueless", "a whole number of requests"),
        ("linear-requests-negative", "a whole number of requests"),
        ("linear-requests-fractional", "a whole number of requests"),
        (
            "linear-requests-wordless",
            "records a `detail` that is not a string",
        ),
    ] {
        let errored = result(id);
        assert_eq!(errored["verdict"], "error", "{errored:#}");
        let error = errored["error"].as_str().expect("why it errored");
        assert!(error.contains(&format!("{id}: ")), "{error}");
        assert!(error.contains(reason), "{id}: {error}");
    }
    let unrecorded = result("linear-requests-unrecorded")["error"].to_string();
    assert!(
        unrecorded.contains("onetaskgraph-linear-e2e:test"),
        "it names the target that records it: {unrecorded}"
    );
}

#[test]
fn the_runner_refuses_a_budget_with_nowhere_to_report_and_a_result_it_cannot_write() {
    let sandbox = Sandbox::new();
    let recorded = sandbox.subdirectory("telemetry");
    telemetry::record_in(&recorded, "linear-requests-recorded", 1, "one");

    let nowhere = run_report(&recorded, |command| {
        command.env("ONEBUDGETSPEC_BUDGET_ID", "linear-requests-recorded");
    });
    let stderr = String::from_utf8_lossy(&nowhere.stderr);
    assert!(!nowhere.status.success(), "{stderr}");
    assert!(
        stderr.contains("ONEBUDGETSPEC_RESULT is not set"),
        "{stderr}"
    );

    // A directory is not a file the reporter can replace.
    let unwritable = sandbox.subdirectory("result");
    let refused = run_report(&recorded, |command| {
        command
            .env("ONEBUDGETSPEC_BUDGET_ID", "linear-requests-recorded")
            .env("ONEBUDGETSPEC_RESULT", &unwritable);
    });
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(!refused.status.success(), "{stderr}");
    assert!(stderr.contains("figure could not be reported"), "{stderr}");

    let written = sandbox.subdirectory("written").join("result.json");
    std::fs::write(&written, "").expect("an empty result file, as onebudgetspec makes one");
    let reported = run_report(&recorded, |command| {
        command
            .env("ONEBUDGETSPEC_BUDGET_ID", "linear-requests-recorded")
            .env("ONEBUDGETSPEC_RESULT", &written);
    });
    assert!(
        reported.status.success(),
        "{}",
        String::from_utf8_lossy(&reported.stderr)
    );
    let result: Value =
        serde_json::from_str(&std::fs::read_to_string(&written).expect("the result")).unwrap();
    assert_eq!(result, json!({"value": 1.0, "detail": "one"}));
}

#[test]
fn the_cache_budgets_are_reported_at_their_workload_and_refused_at_another_or_without_a_figure() {
    use crate::cache_budget::{BUDGET, GUARD, WORKLOAD};
    let sandbox = Sandbox::new();
    // Each recording in a telemetry directory of its own, since they share the budget's id.
    let report = |budget: &str, name: &str, recording: &Value| {
        let recorded = sandbox.subdirectory(&format!("telemetry-{budget}-{name}"));
        std::fs::write(telemetry::file_in(&recorded, budget), recording.to_string())
            .expect("writable");
        let result = sandbox
            .subdirectory(&format!("result-{budget}-{name}"))
            .join("result.json");
        std::fs::write(&result, "").expect("an empty result file, as onebudgetspec makes one");
        let output = run_report(&recorded, |command| {
            command
                .env("ONEBUDGETSPEC_BUDGET_ID", budget)
                .env("ONEBUDGETSPEC_RESULT", &result);
        });
        (output, std::fs::read_to_string(&result).unwrap_or_default())
    };

    let scale = |on: u64, off: u64, dearest: u64| {
        json!({"hour": {"on": {"requests": on}, "off": {"requests": off}},
               "largest_single_query": {"on": dearest}})
    };
    let scales = json!({"1x": scale(180, 300, 5_908), "10x": scale(1_811, 3_000, 8_550)});
    // (budget, the figure a valid recording carries — the smaller scale's requests saved, and
    // the larger scale's dearest query — the unit a refusal names, the member it is read from,
    // and which of the scales the figure is)
    for (budget, figure, unit, member, which) in [
        (BUDGET, 120, "a whole number of requests", "hour", "smaller"),
        (
            GUARD,
            8_550,
            "a whole number of complexity points",
            "largest_single_query",
            "larger",
        ),
    ] {
        let valid = json!({"value": figure, "detail": "the cache saves", "workload": WORKLOAD,
                           "scales": scales});
        let with = |pointer: &str, value: Value| {
            let mut changed = valid.clone();
            *changed.pointer_mut(pointer).expect("the member") = value;
            changed
        };
        let (reported, result) = report(budget, "valid", &valid);
        assert!(
            reported.status.success(),
            "{budget}: {}",
            String::from_utf8_lossy(&reported.stderr)
        );
        let result: Value = serde_json::from_str(&result).expect("the result");
        assert_eq!(
            result,
            json!({"value": f64::from(figure), "detail": "the cache saves"}),
            "{budget}: the figure is reported as the number it is"
        );
        let measured = if budget == BUDGET {
            "/scales/1x/hour/off/requests"
        } else {
            "/scales/1x/largest_single_query/on"
        };
        for (name, recording, reason) in [
            (
                "another-workload",
                with("/workload", json!("one host, one pass")),
                "not of the workload the budget states".to_owned(),
            ),
            ("no-figure", with("/value", json!(null)), unit.to_owned()),
            ("fractional", with("/value", json!(-0.5)), unit.to_owned()),
            (
                "no-ten-times-hour",
                with("/scales/10x", json!({})),
                format!("the 10x {member} is not measured in both modes"),
            ),
            (
                "fractional-measure",
                with(measured, json!(1.5)),
                format!("the 1x {member} is not measured in both modes"),
            ),
            (
                "another-figure",
                with("/value", json!(0)),
                format!("is not the {which} scale's {member}, {figure}"),
            ),
        ] {
            let (refused, _) = report(budget, name, &recording);
            let stderr = String::from_utf8_lossy(&refused.stderr);
            assert!(!refused.status.success(), "{budget} {name}: {stderr}");
            assert!(
                stderr.contains(&format!("{budget}: ")),
                "{budget} {name}: {stderr}"
            );
            assert!(stderr.contains(&reason), "{budget} {name}: {stderr}");
        }
    }
}

#[test]
fn a_budget_id_that_is_not_one_names_no_file() {
    for id in ["../escape", "/abs", "Upper", "", "a/b"] {
        let refused = telemetry::recorded(id).expect_err("not an id");
        assert!(refused.contains("is not a budget id"), "{id:?}: {refused}");
    }
}

#[cfg(unix)]
#[test]
fn a_budget_id_that_is_not_unicode_is_refused_rather_than_read_as_absent() {
    use std::os::unix::ffi::OsStringExt as _;
    let sandbox = Sandbox::new();
    let output = run_report(&sandbox.subdirectory("telemetry"), |command| {
        command.env(
            "ONEBUDGETSPEC_BUDGET_ID",
            std::ffi::OsString::from_vec(vec![0x66, 0xff]),
        );
    });
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "{stderr}");
    assert!(stderr.contains("is not even Unicode"), "{stderr}");
}

#[test]
fn asset_reports_require_the_full_recorded_workload_before_reporting() {
    let sandbox = Sandbox::new();
    let directory = sandbox.subdirectory("asset-telemetry");
    let budgets: Value = serde_saphyr::from_str(include_str!("../../budgets.yaml")).unwrap();
    for budget in budgets["budgets"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|budget| {
            let id = budget["id"].as_str().unwrap();
            id.starts_with("linear-asset-") || id.starts_with("linear-concurrent-")
        })
    {
        let id = budget["id"].as_str().unwrap();
        let (count, copies) = match budget["unit"].as_str().unwrap() {
            "seconds" => (24, 1),
            "copies" => (72, 3),
            _ => (8, if id.contains("unchanged") { 3 } else { 1 }),
        };
        let result = directory.join("result.json");
        for broken in [
            "missing",
            "count",
            "size",
            "oversize",
            "noninteger",
            "no-large",
            "copies",
            "no-workload",
            "malformed-workload",
            "no-sizes",
            "malformed-sizes",
            "no-value",
            "nonnumeric-value",
            "negative-value",
            "fractional-copies",
            "too-many-copies",
            "valid",
        ] {
            if matches!(broken, "fractional-copies" | "too-many-copies")
                && budget["unit"] != "copies"
            {
                continue;
            }
            let path = telemetry::file_in(&directory, id);
            let _ = std::fs::remove_file(&path);
            let _ = std::fs::remove_file(&result);
            if broken != "missing" {
                let mut sizes = vec![475_000; count];
                if broken == "count" {
                    sizes.pop();
                }
                if broken == "size" {
                    sizes[0] = 49_999;
                }
                let mut recorded = json!({"value":1,"detail":"observed", "workload":{"sizes":sizes,"copies":if broken == "copies" {0} else {copies}}});
                match broken {
                    "oversize" => recorded["workload"]["sizes"][0] = json!(500_001),
                    "noninteger" => recorded["workload"]["sizes"][0] = json!(475_000.5),
                    "no-large" => recorded["workload"]["sizes"] = json!(vec![100_000; count]),
                    "no-workload" => {
                        recorded.as_object_mut().unwrap().remove("workload");
                    }
                    "malformed-workload" => recorded["workload"] = json!("not an object"),
                    "no-sizes" => {
                        recorded["workload"]
                            .as_object_mut()
                            .unwrap()
                            .remove("sizes");
                    }
                    "malformed-sizes" => recorded["workload"]["sizes"] = json!("not an array"),
                    "no-value" => {
                        recorded.as_object_mut().unwrap().remove("value");
                    }
                    "nonnumeric-value" => recorded["value"] = json!("not a number"),
                    "negative-value" => recorded["value"] = json!(-1),
                    "fractional-copies" => recorded["value"] = json!(0.5),
                    "too-many-copies" => recorded["value"] = json!(4),
                    _ => {}
                }
                std::fs::write(&path, recorded.to_string()).unwrap();
            }
            let output = run_report(&directory, |command| {
                command
                    .env("ONEBUDGETSPEC_BUDGET_ID", id)
                    .env("ONEBUDGETSPEC_RESULT", &result);
            });
            assert_eq!(
                output.status.success(),
                broken == "valid",
                "{id} {broken}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                result.exists(),
                broken == "valid",
                "{id} {broken}: refused workloads report no figure"
            );
        }
    }
}

#[test]
fn each_absolute_linear_budget_is_reported_as_the_recorded_hour_holds_it_and_refused_otherwise() {
    use crate::cache_budget::{ABSOLUTE, WORKLOAD};
    let sandbox = Sandbox::new();
    let hour = |requests: u64, complexity: u64, warm: u64| {
        json!({"hour": {"on": {"requests": requests, "complexity": complexity}},
               "all_warm_hour": {"on": {"requests": warm}}})
    };
    let scales = json!({"1x": hour(240, 93_087, 120), "10x": hour(2_250, 509_142, 1_050)});
    for (budget, scale, member) in ABSOLUTE {
        let figure = scales[scale].pointer(member).cloned().expect("a figure");
        let report = |name: &str, recording: &Value| {
            let recorded = sandbox.subdirectory(&format!("telemetry-{budget}-{name}"));
            std::fs::write(telemetry::file_in(&recorded, budget), recording.to_string())
                .expect("writable");
            let result = sandbox
                .subdirectory(&format!("result-{budget}-{name}"))
                .join("result.json");
            std::fs::write(&result, "").expect("an empty result file");
            let output = run_report(&recorded, |command| {
                command
                    .env("ONEBUDGETSPEC_BUDGET_ID", budget)
                    .env("ONEBUDGETSPEC_RESULT", &result);
            });
            (output, std::fs::read_to_string(&result).unwrap_or_default())
        };
        let valid = json!({"value": figure, "detail": "measured", "workload": WORKLOAD,
                           "scales": scales});
        let (reported, result) = report("valid", &valid);
        assert!(
            reported.status.success(),
            "{budget}: {}",
            String::from_utf8_lossy(&reported.stderr)
        );
        let result: Value = serde_json::from_str(&result).expect("the result");
        assert_eq!(
            result["value"].as_f64(),
            figure.as_u64().map(|figure| figure as f64),
            "{budget}"
        );
        let with = |pointer: &str, value: Value| {
            let mut changed = valid.clone();
            *changed.pointer_mut(pointer).expect("the member") = value;
            changed
        };
        let without = |pointer: &str, key: &str| {
            let mut changed = valid.clone();
            changed
                .pointer_mut(pointer)
                .and_then(Value::as_object_mut)
                .expect("the parent")
                .remove(key);
            changed
        };
        let whole = "a whole, non-negative number";
        let (parent, member_key) = member.rsplit_once('/').expect("a nested member");
        for (name, recording, reason) in [
            (
                "another-workload",
                with("/workload", json!("one host, one pass")),
                "not of the workload the budget states",
            ),
            ("another-figure", with("/value", json!(1)), "is not the"),
            ("no-figure", without("", "value"), whole),
            ("negative", with("/value", json!(-1)), whole),
            ("fractional", with("/value", json!(1.5)), whole),
            ("text", with("/value", json!("207")), whole),
            (
                "malformed-measure",
                with(&format!("/scales/{scale}{member}"), json!("207")),
                "is not the",
            ),
            (
                "no-measure",
                without(&format!("/scales/{scale}{parent}"), member_key),
                "is not the",
            ),
        ] {
            let (refused, _) = report(name, &recording);
            let stderr = String::from_utf8_lossy(&refused.stderr);
            assert!(!refused.status.success(), "{budget} {name}: {stderr}");
            assert!(stderr.contains(reason), "{budget} {name}: {stderr}");
        }
    }
}

#[test]
fn document_reports_require_complete_transport_journals_from_both_lanes() {
    let sandbox = Sandbox::new();
    let assets = sandbox.subdirectory("assets");
    let plugin = sandbox.subdirectory("plugin");
    let result = sandbox.subdirectory("result").join("report.json");
    let budget = "linear-live-documents-created-per-daily-pass";
    let run = |id: &str| {
        run_report(&assets, |command| {
            command
                .env("LINEAR_PLUGIN_TELEMETRY_DIR", &plugin)
                .env("ONEBUDGETSPEC_BUDGET_ID", id)
                .env("ONEBUDGETSPEC_RESULT", &result);
        })
    };
    for (lane, directory) in [("plugin", &plugin), ("assets", &assets)] {
        for mode in ["ordinary", "fresh", "initialize", "failure"] {
            let failed = run(budget);
            assert!(
                !failed.status.success(),
                "missing {lane}/{mode} was accepted"
            );
            assert!(!result.exists(), "missing journals emitted a report");
            let journal = transport_journal::Journal::new(lane, mode)
                .durable(directory.to_path_buf())
                .expect("durable transport journal");
            journal.begin(mode);
            // The ordinary pass's one create is a cold rebuild of a missing standing slot,
            // which is filed under that journey and must stay out of the ordinary total.
            if mode != "ordinary" {
                journal.journey("probe");
            }
            // Exercise the same acknowledgement recorder and serializer the HTTP replay
            // uses, rather than a separately maintained consumer-shaped record.
            journal.observe(
                &json!({"query":"mutation { documentCreate(input: {}) { success } }"}),
                &json!({"data":{"documentCreate":{"success":true}}}),
            );
            assert_eq!(journal.count(), 1);
            let interrupted = run(budget);
            assert!(
                !interrupted.status.success(),
                "unfinished durable {lane}/{mode} was accepted"
            );
            assert!(!result.exists(), "unfinished journal emitted a report");
            journal.finish_in(directory).expect("writable journal");
            let path = directory.join(format!("linear-live-documents-{mode}.json"));
            let mut recorded: Value =
                serde_json::from_slice(&std::fs::read(&path).expect("journal")).expect("JSON");
            recorded["residue"] = json!(1);
            std::fs::write(path, recorded.to_string()).expect("writable diagnostic");
        }
    }
    let success = run(budget);
    assert!(
        success.status.success(),
        "{}",
        String::from_utf8_lossy(&success.stderr)
    );
    let report: Value =
        serde_json::from_slice(&std::fs::read(&result).expect("report")).expect("JSON");
    assert_eq!(report["value"], 2.0, "{report:#}");
    let detail = report["detail"].as_str().expect("per-lane detail");
    for lane in ["plugin", "assets"] {
        for mode in ["ordinary", "fresh", "initialize", "failure"] {
            assert!(detail.contains(&format!("{lane}/{mode}:")), "{detail}");
        }
    }
    assert!(
        detail.contains("plugin/failure residue: 1 documents"),
        "{detail}"
    );
    for lane in ["plugin", "assets"] {
        assert!(
            detail.contains(&format!(
                "{lane}/ordinary cold rebuild: 1 documents, outside the pass total"
            )),
            "{detail}"
        );
    }
    let ordinary = run("linear-live-documents-created-per-ordinary-pass");
    assert!(
        ordinary.status.success(),
        "{}",
        String::from_utf8_lossy(&ordinary.stderr)
    );
    let report: Value =
        serde_json::from_slice(&std::fs::read(&result).expect("ordinary report")).expect("JSON");
    assert_eq!(
        report["value"], 0.0,
        "a cold rebuild is never counted in the ordinary pass: {report:#}"
    );
    std::fs::remove_file(&result).expect("remove previous report");
    let path = plugin.join("linear-live-documents-fresh.json");
    std::fs::write(&path, "not JSON").expect("writable");
    let malformed = run(budget);
    assert!(!malformed.status.success());
    assert!(String::from_utf8_lossy(&malformed.stderr).contains("invalid plugin/fresh journal"));
    assert!(!result.exists(), "malformed journal emitted a report");
    for invalid in [
        json!({"schema_version":2,"lane":"plugin","mode":"fresh","completed":true,"operations":[]}),
        json!({"schema_version":1,"lane":"assets","mode":"fresh","completed":true,"operations":[]}),
        json!({"schema_version":1,"lane":"plugin","mode":"ordinary","completed":true,"operations":[]}),
        json!({"schema_version":1,"lane":"plugin","mode":"fresh","completed":true,"operations":[{"operation":"documentCreate","success":true}]}),
        json!({"schema_version":1,"lane":"plugin","mode":"fresh","completed":true,"operations":[{"journey":"","operation":"documentCreate","success":true}]}),
        json!({"schema_version":1,"lane":"plugin","mode":"fresh","completed":true,"operations":[{"journey":"probe","operation":"documentUpdate","success":true}]}),
        json!({"schema_version":1,"lane":"plugin","mode":"fresh","completed":true,"operations":[],"residue":"private body"}),
        json!({"schema_version":1,"lane":"plugin","mode":"fresh","completed":false,"operations":[]}),
        json!({"schema_version":1,"lane":"plugin","mode":"fresh","completed":true}),
        json!({"schema_version":1,"lane":"plugin","mode":"fresh","completed":true,"operations":[{"journey":"probe","operation":"documentCreate","success":false}]}),
    ] {
        std::fs::write(&path, invalid.to_string()).expect("writable");
        let refused = run(budget);
        assert!(
            !refused.status.success(),
            "invalid journal was accepted: {invalid}"
        );
        assert!(!result.exists(), "invalid journal emitted a report");
    }
}

#[test]
fn durable_transport_journal_counts_aliased_and_spaced_creates_before_completion() {
    let sandbox = Sandbox::new();
    let directory = sandbox.subdirectory("durable-journal");
    let journal = transport_journal::Journal::new("assets", "fresh")
        .durable(directory.clone())
        .unwrap();
    journal.journey("transport-acknowledgements");
    journal.observe(&json!({"query":"mutation { first: documentCreate (input:{title:\"one\"}) {success} second: documentCreate(input:{title:\"two\"}) {success} }"}),&json!({"data":{"first":{"success":true,"document":null},"second":{"success":true}}}));
    journal.observe(
        &json!({"query":"mutation { documentCreate (input:{title:\"refused\"}) {success} }"}),
        &json!({"data":{"documentCreate":{"success":false}}}),
    );
    assert_eq!(
        journal.count(),
        2,
        "each actual successful operation counts, even without a returned id"
    );
    let path = directory.join("linear-live-documents-fresh.json");
    let incomplete: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(incomplete["completed"], false);
    assert_eq!(incomplete["operations"].as_array().unwrap().len(), 2);
    journal.finish_in(&directory).unwrap();
    let complete: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(complete["completed"], true);
    assert_eq!(complete["operations"], incomplete["operations"]);
}
