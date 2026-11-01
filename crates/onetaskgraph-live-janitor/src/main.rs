//! The entry point `.github/workflows/live-janitor.yml` runs, and — given `--cancelled-run`
//! first — the one `.github/workflows/ci.yml` runs when a live run is cancelled.
//!
//! `--linear` instead expires stamped test documents in `LINEAR_WRITE_TEAM` after 24
//! hours, using `LINEAR_API_KEY` alone. It recovers residue from discarded CI hosts, and
//! retires the pre-pool standing CI set the pool superseded once it is a day untouched.
//! `--linear --release-leases DIR`, which `ci.yml` runs when a live run is cancelled, releases
//! the pool leases that run recorded in `DIR` and still holds, and nothing else.
//!
//! `--cancelled-run` removes exactly the artifacts stamped with the run and attempt GitHub
//! Actions names in `GITHUB_RUN_ID` and `GITHUB_RUN_ATTEMPT` (with `GITHUB_ACTIONS=true`),
//! every kind of them, and nothing of another run. It reads no run's status and so needs no
//! `GITHUB_TOKEN`; every other bound — the nominations, the allowance read, the read and
//! write limits and the pacing — is the scheduled pass's.
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

use onetaskgraph_github_live::{
    BOARD_NUMBER, BOARD_OWNER, GITHUB_ACTIONS_VARIABLE, RUN_ATTEMPT_VARIABLE, RUN_ID_VARIABLE,
    SCRATCH_REPOSITORY, ci_run,
};
use onetaskgraph_live_janitor::{
    Config, Outcome, Ownership, Report, VirtualClock, run, run_with_clock,
};

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
    ] {
        if variable(name)? != expected {
            return Err(Failure::Configuration(format!(
                "{name} must nominate {expected}"
            )));
        }
    }
    let write_token = variable("GH_PROJECTS_TOKEN")?;
    let mut arguments: Vec<_> = std::env::args().skip(1).collect();
    let ownership = if arguments
        .first()
        .is_some_and(|first| first == "--cancelled-run")
    {
        arguments.remove(0);
        let read = |name| std::env::var(name).ok();
        let run = ci_run(
            read(GITHUB_ACTIONS_VARIABLE).as_deref(),
            read(RUN_ID_VARIABLE).as_deref(),
            read(RUN_ATTEMPT_VARIABLE).as_deref(),
        )
        .map_err(Failure::Configuration)?
        .ok_or_else(|| {
            Failure::Configuration(format!(
                "--cancelled-run cleans up after a GitHub Actions run, and \
                 {GITHUB_ACTIONS_VARIABLE} is not true"
            ))
        })?;
        Ownership::CancelledRun(run)
    } else {
        Ownership::completed_runs(&variable("GITHUB_TOKEN")?).map_err(Failure::Configuration)?
    };
    // Explicit loopback-only test boundary: no offline journey can send a token to a host.
    if arguments.is_empty() {
        return Config::github(&write_token, ownership)
            .map(|config| (config, ClockMode::Real))
            .map_err(Failure::Configuration);
    }
    if !(arguments.len() == 2 || (arguments.len() == 3 && arguments[2] == "--virtual-clock"))
        || arguments[0] != "--loopback"
    {
        return Err(Failure::Configuration(
            "expected [--cancelled-run] [--loopback URL [--virtual-clock]]".into(),
        ));
    }
    Config::loopback(&arguments[1], &write_token, ownership)
        .map(|config| {
            let mode = if arguments.len() == 3 {
                ClockMode::Virtual
            } else {
                ClockMode::Real
            };
            (config, mode)
        })
        .map_err(Failure::Configuration)
}

async fn enter() -> Result<Report, Failure> {
    if std::env::args().nth(1).as_deref() == Some("--linear") {
        use onetaskgraph_live_janitor::linear::{LinearConfig, run_linear, run_linear_with_clock};
        let key = variable("LINEAR_API_KEY")?;
        let team = variable("LINEAR_WRITE_TEAM")?;
        let args: Vec<_> = std::env::args().skip(2).collect();
        // `--release-leases DIR [--loopback URL]`: a cancelled job hands back its pool leases.
        if args.first().map(String::as_str) == Some("--release-leases") {
            use onetaskgraph_live_janitor::linear::release_recorded_leases;
            let config = match args.as_slice() {
                [_, _] => LinearConfig::production(&key, &team),
                [_, _, flag, origin] if flag == "--loopback" => {
                    LinearConfig::loopback(origin, &key, &team)
                }
                _ => {
                    return Err(Failure::Configuration(
                        "expected --linear --release-leases DIR [--loopback URL]".into(),
                    ));
                }
            }
            .map_err(Failure::Configuration)?;
            return release_recorded_leases(config, std::path::Path::new(&args[1]))
                .await
                .map_err(Failure::Cleanup);
        }
        let result = if args.is_empty() {
            run_linear(LinearConfig::production(&key, &team).map_err(Failure::Configuration)?).await
        } else if (args.len() == 2 || (args.len() == 3 && args[2] == "--virtual-clock"))
            && args[0] == "--loopback"
        {
            let config =
                LinearConfig::loopback(&args[1], &key, &team).map_err(Failure::Configuration)?;
            if args.len() == 3 {
                run_linear_with_clock(config, Arc::new(VirtualClock::default())).await
            } else {
                run_linear(config).await
            }
        } else {
            return Err(Failure::Configuration(
                "expected --linear [--loopback URL [--virtual-clock]]".into(),
            ));
        };
        return result.map_err(Failure::Cleanup);
    }
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
