//! Every journey this binary answers, driven the way a user drives it.
//!
//! Each test spawns the compiled binary as a subprocess and asserts on its exit code,
//! stdout and stderr — never an in-process `run()` call, and nothing about the process is
//! mocked. `AGENTS.md` carries the list of journeys this repository owes; the modules
//! below are where they live.
//!
//! [`fixtures`] is the table that makes a journey written once run against every source
//! kind, and `scripts/check-journey-matrix.sh` fails when a plugin the registry knows has
//! no row in it.

// The sandbox `tests/configuration.rs` uses, reached by path because a module of this
// test target would otherwise be looked for under `tests/e2e/`.
//
// `allow` rather than `expect`, and here rather than in the module itself: one support
// module serves two test targets and each uses the part it needs, so what is unused is a
// property of *this* target rather than of the helper. Marking it in the module would
// switch the check off for `tests/configuration.rs` too, where a helper nothing calls
// really is dead.
#[allow(
    dead_code,
    reason = "one shared sandbox, two test targets, a subset used by each"
)]
#[path = "../common/mod.rs"]
mod common;

mod failures;
mod fixtures;
mod journeys;
mod machine;
mod multi_source;
mod no_persistence;
mod source_host;
mod surface;
