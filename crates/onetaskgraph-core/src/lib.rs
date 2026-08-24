//! The onetaskgraph engine.
//!
//! It drives the sources a configuration names and reports what it had to do to
//! answer. Everything a *plugin author* needs lives in `onetaskgraph-plugin-api`
//! instead, and **no plugin crate may depend on this one**: `deny.toml` permits
//! this crate exactly one wrapper, the binary, failing the required `deny` job, and
//! `scripts/check-plugin-isolation.sh` reads the real `cargo metadata` graph inside
//! `just check`.
//!
//! # The invariant that shapes this crate
//!
//! No work data may be stored, cached, indexed or mirrored outside a plugin. The
//! engine compensates for a missing capability *transiently*: it holds at most one
//! source page plus the caller's page and writes nothing down. Three mechanisms
//! enforce that rather than asking for it: `deny.toml` refuses every embedded store,
//! index and cache crate, so reaching for one fails the required `deny` job; a sandboxed
//! journey plants sentinels, drives every verb, and fails if one reaches any file written
//! during the run; and a re-ask test fails if one query asked twice reaches the source
//! once. The latter two land with the verbs they drive.
#![deny(missing_docs)]

pub mod config;

mod engine;

mod environment;
mod global_id;
mod plan;
mod registry;
mod resolve;
mod schema;
mod secrets;

pub use config::{Config, ConfigError, Loaded, OutputFormat, SourceConfig};
pub use engine::{
    ConfiguredSource, DependencyRequest, Engine, EngineError, Filters, LabelRequest, Paging,
    ProjectRequest, ProjectSelector, Qualified, QualifiedEdge, SearchHit, SearchKind,
    SearchRequest, SourceListing, SourceState, TaskRequest,
};
pub use environment::Environment;
pub use global_id::GlobalId;
pub use plan::{PageToken, Predicate, QueryPlan, QueryResponse, SourceFailure, SourcePlan};
pub use registry::{PluginKind, plugin_for, plugin_kinds, registry};
pub use resolve::{
    ResolvedSource, UnavailableSource, resolve, resolve_available, validate_sources,
};
pub use schema::{SCHEMA_BUNDLE_VERSION, schema_bundle};
pub use secrets::{CredentialLayer, CredentialName, ResolvedCredential, Secrets, SecretsReport};
