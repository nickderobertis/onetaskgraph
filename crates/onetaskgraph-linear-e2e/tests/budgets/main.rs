//! The one runner of `budgets.yaml`, which every budget there names as its command.
//!
//! It measures nothing: it reads the telemetry the budget's journey recorded while the `e2e`
//! target ran — the budget is the one `ONEBUDGETSPEC_BUDGET_ID` names, so no list of ids lives
//! here — and hands that figure to `onebudgetspec check` through `onebudgetspec_core::report`.
//! The comparison with the threshold is onebudgetspec's alone. `test = false` in the manifest
//! keeps it out of the `e2e` run, so it is built and run only when a budget names it.

#[path = "../telemetry/mod.rs"]
mod telemetry;

use std::process::ExitCode;

fn main() -> ExitCode {
    let Ok(budget) = std::env::var("ONEBUDGETSPEC_BUDGET_ID") else {
        eprintln!(
            "budgets: ONEBUDGETSPEC_BUDGET_ID is not set, so there is no budget to report; run \
             `onebudgetspec check crates/onetaskgraph-linear-e2e/budgets.yaml`, which sets it"
        );
        return ExitCode::from(2);
    };
    let (value, detail) = match telemetry::recorded(&budget) {
        Ok(recorded) => recorded,
        Err(reason) => {
            eprintln!("budgets: {reason}");
            return ExitCode::FAILURE;
        }
    };
    match onebudgetspec_core::report(value, detail.as_deref()) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => {
            eprintln!(
                "budgets: ONEBUDGETSPEC_RESULT is not set, so {budget}'s figure has nowhere to go; \
                 run it through `onebudgetspec check`"
            );
            ExitCode::from(2)
        }
        Err(error) => {
            eprintln!("budgets: {budget}'s figure could not be reported: {error}");
            ExitCode::FAILURE
        }
    }
}
