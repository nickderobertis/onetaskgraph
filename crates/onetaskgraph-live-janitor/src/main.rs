//! The entry point `.github/workflows/live-janitor.yml` runs.
//!
//! Exit status, one per way an invocation can end:
//!
//! - `0` — cleanup completed: every list was read, and every write the evidence allowed was
//!   sent, up to the write cap. What is left over is the next hourly run's.
//! - `1` — cleanup failed: a read or a write failed, or a list was incomplete. Nothing was
//!   written after the failure, and the message on stderr names it.
//! - `2` — configuration refused: a nomination, a credential or an argument was missing or
//!   wrong. No request was sent.
//! - `3` — declined: the account's allowance cannot afford a run, so nothing was listed or
//!   written. Not a pass — the allowance is shared, and a person should see it was short.

use std::process::ExitCode;
use std::sync::Arc;

use onetaskgraph_github_live::{BOARD_NUMBER, BOARD_OWNER, CORE_REPOSITORY, SCRATCH_REPOSITORY};
use onetaskgraph_live::artifact::now_micros;
use onetaskgraph_live_janitor::{Config, Outcome, Report, VirtualClock, run, run_with_clock};

/// Why an invocation did not complete.
enum Failure {
    /// Refused before any request was sent.
    Configuration(String),
    /// A request failed or answered with something the evidence cannot rest on.
    Cleanup(String),
}

/// Which clock paces the writes: the real one, or the offline journeys' virtual one.
enum ClockMode {
    Real,
    Virtual,
}

fn variable(name: &str) -> Result<String, Failure> {
    std::env::var(name)
        .ok()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| Failure::Configuration(format!("{name} is required")))
}

/// What to run against, and on which clock, read from the environment and the arguments.
fn configure() -> Result<(Config, ClockMode), Failure> {
    let number = BOARD_NUMBER.to_string();
    for (name, expected) in [
        ("GH_PROJECTS_OWNER", BOARD_OWNER),
        ("GH_PROJECTS_NUMBER", number.as_str()),
        ("GH_PROJECTS_REPOSITORY", SCRATCH_REPOSITORY),
        ("GH_PROJECTS_LEGACY_REPOSITORY", CORE_REPOSITORY),
    ] {
        if variable(name)? != expected {
            return Err(Failure::Configuration(format!(
                "{name} must nominate {expected}"
            )));
        }
    }
    let write_token = variable("GH_PROJECTS_TOKEN")?;
    let actions_token = variable("GITHUB_TOKEN")?;
    // Explicit loopback-only test boundary: no offline journey can send a token to a host.
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    if arguments.is_empty() {
        return Config::github(&write_token, &actions_token, now_micros())
            .map(|config| (config, ClockMode::Real))
            .map_err(Failure::Configuration);
    }
    if !(arguments.len() == 3 || (arguments.len() == 4 && arguments[3] == "--virtual-clock"))
        || arguments[0] != "--loopback"
    {
        return Err(Failure::Configuration(
            "expected --loopback URL NOW_MICROS".into(),
        ));
    }
    let now = arguments[2]
        .parse()
        .map_err(|_| Failure::Configuration("NOW_MICROS must be unsigned decimal".into()))?;
    Config::loopback(&arguments[1], &write_token, &actions_token, now)
        .map(|config| {
            let mode = if arguments.len() == 4 {
                ClockMode::Virtual
            } else {
                ClockMode::Real
            };
            (config, mode)
        })
        .map_err(Failure::Configuration)
}

async fn enter() -> Result<Report, Failure> {
    let (config, mode) = configure()?;
    match mode {
        ClockMode::Virtual => run_with_clock(config, Arc::new(VirtualClock::default())).await,
        ClockMode::Real => run(config).await,
    }
    .map_err(Failure::Cleanup)
}

#[tokio::main]
async fn main() -> ExitCode {
    match enter().await {
        Ok(report) => {
            println!("write times (monotonic micros): {:?}", report.write_times);
            println!(
                "janitor: {} REST reads, {} GraphQL queries, {} content writes",
                report.rest_reads, report.graphql_queries, report.writes
            );
            match report.outcome {
                Outcome::Completed => ExitCode::SUCCESS,
                Outcome::Declined => {
                    eprintln!(
                        "janitor declined: the allowance cannot afford a run; nothing was cleaned"
                    );
                    ExitCode::from(3)
                }
            }
        }
        Err(Failure::Configuration(error)) => {
            eprintln!("janitor refused its configuration, sending nothing: {error}");
            ExitCode::from(2)
        }
        Err(Failure::Cleanup(error)) => {
            eprintln!("janitor failed: {error}");
            ExitCode::FAILURE
        }
    }
}
