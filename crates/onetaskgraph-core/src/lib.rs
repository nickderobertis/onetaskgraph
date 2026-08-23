//! The onetaskgraph engine.
//!
//! It drives the sources a configuration names and reports what it had to do to
//! answer. Everything a *plugin author* needs lives in `onetaskgraph-plugin-api`
//! instead, and **no plugin crate may depend on this one** — see `AGENTS.md` for
//! the two mechanisms that enforce that rather than stating it.
//!
//! # The invariant that shapes this crate
//!
//! No work data may be stored, cached, indexed or mirrored outside a plugin. The
//! engine compensates for a missing capability *transiently*: it holds at most one
//! source page plus the caller's page and writes nothing down. Three mechanisms
//! enforce that rather than asking for it; `AGENTS.md` names all three.
#![deny(missing_docs)]

mod global_id;
mod plan;
mod registry;
mod schema;

pub use global_id::GlobalId;
pub use plan::{PageToken, Predicate, QueryPlan, QueryResponse, SourceFailure, SourcePlan};
pub use registry::{plugin_for, plugin_kinds, registry};
pub use schema::{SCHEMA_BUNDLE_VERSION, schema_bundle};
