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
//!
//! The repository README is included below so its worked Rust example is compiled and
//! executed as a doctest whenever this crate's documentation tests run.
#![doc = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/",
    env!("CARGO_PKG_README")
))]
#![deny(missing_docs)]

mod clock;
pub mod config;

mod engine;

mod environment;
mod failure;
mod global_id;
mod plan;
mod registry;
mod resolve;
mod schema;
mod secrets;
pub mod subprocess;
// llmlint: ignore[code_lands_in_the_domain_that_owns_it] The task that introduced templates fixes their API at this crate's root; the head of `template/mod.rs` states why.
pub mod template;

pub use clock::{
    ClockChoice, SIMULATED_CLOCK_VARIABLE, attach as attach_simulated_clock, clock_choice,
    process_clock,
};
pub use config::{Config, ConfigError, Loaded, OutputFormat, SourceConfig};
pub use engine::{
    Body, BudgetSpent, CommentList, ConfiguredSource, CopyAction, CopyItems, CopyLink, CopyLookup,
    CopyOutcome, CopyReport, CopyRequest, CopyScope, CopyVia, DeletedComment, Delivered,
    DeliveryOutcome, DependencyRequest, DocumentDetail, DocumentFilters, DocumentRequest, Engine,
    EngineError, Filters, GraphDirection, GraphEdge, GraphNode, LabelRequest, LeftBehind, MatchBy,
    MetadataSet, NoCounterpart, PROJECT_GRAPH_SCHEMA_VERSION, Paging, ProjectGraph,
    ProjectGraphRequest, ProjectRequest, ProjectSelector, Qualified, QualifiedEdge,
    QualifiedEndpoint, SearchHit, SearchKind, SearchRequest, SourceListing, SourceState, Spent,
    TaskContentSet, TaskDetail, TaskDetails, TaskPrioritySet, TaskRequest, TaskStatusSet,
    TaskUpdated, graph_label, settled,
};
pub use engine::{
    DocumentCreate, ProjectCreate, Regenerated, Regeneration, RenderRequest, RenderTemplate,
    RenderedRecord, TaskCreate, TaskCreated, TemplateAnswers, UnusedAnswers,
};
pub use environment::Environment;
pub use failure::{Failure, FailureClass, FailureDocument, classify};
pub use global_id::GlobalId;
/// Named by the answer of [`TaskSource::update_task`], so it lives in the plugin api, and
/// re-exported here because [`TaskUpdated`] reports it.
///
/// [`TaskSource::update_task`]: onetaskgraph_plugin_api::TaskSource::update_task
pub use onetaskgraph_plugin_api::UpdatedField;
pub use plan::{PageToken, Predicate, QueryPlan, QueryResponse, SourceFailure, SourcePlan};
pub use registry::{PluginKind, plugin_for, plugin_kinds, registry};
pub use resolve::{
    ResolvedSource, UnavailableSource, resolve, resolve_available, resolve_available_with_clock,
    validate_sources,
};
pub use schema::{SCHEMA_BUNDLE_VERSION, schema_bundle};
pub use secrets::{CredentialLayer, CredentialName, ResolvedCredential, Secrets, SecretsReport};
pub use subprocess::{
    MAX_LINE, Program, RequestDeadline, SubprocessConfig, SubprocessPlugin, SubprocessSource,
    serve, serve_plugin,
};
pub use template::{
    Answers, ChainField, DECLARATION_KEYS, FRONT_MATTER_KEYS, ItemType, LoaderDocument,
    RenderedTemplate, Sha256Digest, Template, TemplateError, TemplateInput, TemplateLoader,
    TemplateProvenance, TemplateVariable, TemplateVariables, VariableType,
};
