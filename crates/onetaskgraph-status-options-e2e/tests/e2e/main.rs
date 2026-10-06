//! The status-options setup's journeys, driven through the compiled binary the way a user drives
//! them.
//!
//! Every journey spawns the binary `onetaskgraph:build` made against the loopback board of
//! `onetaskgraph_e2e_support::fixtures`, with no credential and no network, and asserts on its
//! exit code, stdout and stderr and on every request the board was sent. They live in a
//! test-only member of their own, whose project names `onetaskgraph-status-options` as what it
//! exercises, so a change to it — or to the GitHub Projects plugin it plans against — selects
//! them, and a change to the Linear plugin does not.

use onetaskgraph_e2e_support::{common, fixtures};

mod status_options;
