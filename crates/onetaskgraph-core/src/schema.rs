//! The JSON Schema bundle both SDKs are generated from.
//!
//! It is emitted by `onetaskgraph schema` rather than committed, so the schema a
//! consumer generates against can never drift from the types this binary actually
//! serialises: they are the same types.

use std::collections::BTreeMap;

use onetaskgraph_plugin_api::{
    Capabilities, DependencyEdge, DependencyKind, Direction, Health, Label, Page, PageRequest,
    Project, ProjectQuery, SourceError, Status, StatusCategory, Task, TaskQuery,
};
use schemars::{Schema, schema_for};
use serde_json::{Value, json};

use crate::config::{EffectiveConfig, Origin, OutputFormat, Setting};
use crate::registry::registry;
use crate::secrets::{CredentialLayer, ResolvedCredential, SecretsReport};
use crate::{GlobalId, Predicate, QueryPlan, QueryResponse, SourceFailure, SourcePlan};

/// The bundle's own version, bumped whenever a root is added, removed or renamed.
///
/// Consumers generate code from this document, so the version is part of the
/// contract rather than a convenience: an SDK can refuse a bundle it was not
/// generated against instead of silently emitting the wrong models.
///
/// `2` added the roots `config show --json` emits — `EffectiveConfig`, `Setting`,
/// `Origin`, `OutputFormat` and `SecretsReport` — because a machine-readable output
/// with no root in this bundle is one no SDK can be generated against.
pub const SCHEMA_BUNDLE_VERSION: u32 = 2;

/// Every contract root, keyed by name, plus each registered plugin's config schema.
#[must_use]
pub fn schema_bundle() -> Value {
    let mut roots: BTreeMap<&'static str, Schema> = BTreeMap::new();

    roots.insert("Task", schema_for!(Task));
    roots.insert("Project", schema_for!(Project));
    roots.insert("Label", schema_for!(Label));
    roots.insert("Status", schema_for!(Status));
    roots.insert("StatusCategory", schema_for!(StatusCategory));
    roots.insert("DependencyEdge", schema_for!(DependencyEdge));
    roots.insert("DependencyKind", schema_for!(DependencyKind));
    roots.insert("Direction", schema_for!(Direction));

    roots.insert("TaskQuery", schema_for!(TaskQuery));
    roots.insert("ProjectQuery", schema_for!(ProjectQuery));
    roots.insert("PageRequest", schema_for!(PageRequest));
    roots.insert("PageOfTask", schema_for!(Page<Task>));
    roots.insert("PageOfProject", schema_for!(Page<Project>));
    roots.insert("PageOfLabel", schema_for!(Page<Label>));
    roots.insert("PageOfDependencyEdge", schema_for!(Page<DependencyEdge>));

    roots.insert("Capabilities", schema_for!(Capabilities));
    roots.insert("Health", schema_for!(Health));
    roots.insert("SourceError", schema_for!(SourceError));

    roots.insert("GlobalId", schema_for!(GlobalId));
    roots.insert("QueryPlan", schema_for!(QueryPlan));
    roots.insert("SourcePlan", schema_for!(SourcePlan));
    roots.insert("Predicate", schema_for!(Predicate));
    roots.insert("SourceFailure", schema_for!(SourceFailure));
    roots.insert("QueryResponseOfTask", schema_for!(QueryResponse<Task>));
    roots.insert(
        "QueryResponseOfProject",
        schema_for!(QueryResponse<Project>),
    );
    roots.insert("QueryResponseOfLabel", schema_for!(QueryResponse<Label>));

    roots.insert("EffectiveConfig", schema_for!(EffectiveConfig));
    roots.insert("Setting", schema_for!(Setting));
    roots.insert("Origin", schema_for!(Origin));
    roots.insert("OutputFormat", schema_for!(OutputFormat));
    roots.insert("SecretsReport", schema_for!(SecretsReport));
    roots.insert("ResolvedCredential", schema_for!(ResolvedCredential));
    roots.insert("CredentialLayer", schema_for!(CredentialLayer));

    let plugins: BTreeMap<String, Schema> = registry()
        .iter()
        .map(|plugin| (plugin.kind().to_owned(), plugin.config_schema()))
        .collect();

    json!({
        "version": SCHEMA_BUNDLE_VERSION,
        "roots": roots,
        "plugin_config": plugins,
    })
}
