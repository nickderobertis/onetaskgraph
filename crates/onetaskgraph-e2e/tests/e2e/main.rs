//! Every journey of the engine and the command line, driven the way a user drives them.
//!
//! Each test spawns the compiled binary as a subprocess and asserts on its exit code,
//! stdout and stderr — never an in-process `run()` call, and nothing about the process is
//! mocked. `AGENTS.md` carries the list of journeys this repository owes; the modules
//! below are where the engine's and the CLI's own live, and each hosted plugin's own are in
//! the e2e suite named after it.
//!
//! This is a test-only workspace member of its own rather than the binary crate's `tests/`, so
//! a change the engine and the binary cannot see does not run it. It drives the binary
//! `onetaskgraph:build` made, found by `onetaskgraph_e2e_support::binary`, and
//! `onetaskgraph_e2e_support::fixtures` is the table that makes a journey written once run
//! against every source kind — `scripts/check-journey-matrix.sh` fails when a plugin the
//! registry knows has no row in it.

use onetaskgraph_e2e_support::{common, fixtures, linear_vocabulary};

mod commented_since;
mod comments;
mod copy;
mod copy_link;
mod delivery;
mod document_store;
mod end_command;
mod failures;
mod journeys;
mod machine;
mod metadata;
mod multi_source;
mod no_persistence;
mod priority;
mod rendered;
mod routes;
mod show_many;
mod source_host;
mod surface;
mod templates;
mod update;
