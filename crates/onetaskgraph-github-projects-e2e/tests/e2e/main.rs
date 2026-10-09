//! The GitHub Projects plugin's journeys, driven through the compiled binary the way a user
//! drives them.
//!
//! Every journey spawns the binary `onetaskgraph:build` made against the loopback board of
//! `onetaskgraph_e2e_support::fixtures` — beside a folder of Markdown where one is the other end
//! of a copy — with no credential and no network, and asserts on its exit code, stdout and
//! stderr and on every request the board was sent. They live in a test-only member of their
//! own, whose project names the GitHub Projects plugin as what it exercises, so a change to that
//! plugin selects them and a change to a sibling plugin does not. The plugin's own half is
//! proven behind the same edge in its own `tests/`.

use onetaskgraph_e2e_support::{common, fixtures};

mod boundary;
mod copy_cost;
mod fields;
mod github_status_by_kind;
mod write_order;

mod asset_board;
mod assets;
mod budget_runner;
mod visibility_cost;
