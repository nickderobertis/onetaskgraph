//! The Linear plugin's journeys, driven through the compiled binary the way a user drives them.
//!
//! Every journey spawns the binary `onetaskgraph:build` made against the loopback Linear
//! workspace of `onetaskgraph_e2e_support::fixtures`, with no credential and no network, and
//! asserts on its exit code, stdout and stderr and on what the workspace was sent and holds.
//! They live in a test-only member of their own, whose project names the Linear plugin as what
//! it exercises, so a change to that plugin selects them and a change to a sibling plugin does
//! not. The plugin's own half is proven behind the same edge in its `tests/plugin.rs`.
//!
//! [`linear_budget`] is where `crates/onetaskgraph-linear/budgets.yaml` measures each Linear
//! request budget: `scripts/linear-budget.sh` runs one of its `measure_*` journeys by name.

use onetaskgraph_e2e_support::{common, fixtures, linear_vocabulary};

mod linear;
mod linear_budget;
mod linear_status;
