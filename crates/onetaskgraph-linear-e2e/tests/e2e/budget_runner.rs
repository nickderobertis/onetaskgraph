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

#[test]
fn report() {
    let budget = match std::env::var("ONEBUDGETSPEC_BUDGET_ID") {
        Ok(budget) => budget,
        Err(VarError::NotPresent) => return,
        Err(VarError::NotUnicode(raw)) => {
            panic!("ONEBUDGETSPEC_BUDGET_ID {raw:?} is not a budget id: it is not even Unicode")
        }
    };
    let (value, detail) = telemetry::recorded(&budget).unwrap_or_else(|reason| panic!("{reason}"));
    let reported = onebudgetspec_core::report(value, detail.as_deref())
        .unwrap_or_else(|error| panic!("{budget}'s figure could not be reported: {error}"));
    assert!(
        reported,
        "ONEBUDGETSPEC_RESULT is not set, so {budget}'s figure has nowhere to go; run it through \
         `onebudgetspec check crates/onetaskgraph-linear-e2e/budgets.yaml`"
    );
}

/// The argv that runs [`report`] alone, as each budget's command does.
fn runner() -> Vec<String> {
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
    let ids = [
        "linear-requests-recorded",
        "linear-requests-unrecorded",
        "linear-requests-malformed",
        "linear-requests-valueless",
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
        .output()
        .expect(
            "bun runs the workspace's pinned onebudgetspec; run `just bootstrap` to install it",
        );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(3),
        "four budgets errored: {stderr}"
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
        ("linear-requests-valueless", "records no numeric `value`"),
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
