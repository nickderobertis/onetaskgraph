//! Producer-owned human task projection: the public wire declaration lives here.
//!
//! Requests identify an action item rather than a question. The same declaration is used
//! by the CLI and exported schemas so consumers need not maintain a parallel contract.

use onetaskgraph_plugin_api::{Priority, Repository, SourceName};
use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The sole supported projection request version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ProjectionVersion;

impl Serialize for ProjectionVersion {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u32(1)
    }
}

impl<'de> Deserialize<'de> for ProjectionVersion {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match u32::deserialize(deserializer)? {
            1 => Ok(Self),
            other => Err(serde::de::Error::custom(format!(
                "unsupported task projection schema_version {other}; expected 1"
            ))),
        }
    }
}

impl JsonSchema for ProjectionVersion {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "ProjectionVersion".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        serde_json::json!({"type": "integer", "const": 1})
            .try_into()
            .expect("a literal schema")
    }
}

/// The producer that owns the intent; nullable run and plan are explicit on the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectionOwner {
    /// The owning session.
    pub session: String,
    /// The owning run, when applicable.
    #[serde(deserialize_with = "nullable")]
    #[schemars(required, schema_with = "nullable_string_schema")]
    pub run: Option<String>,
    /// The owning plan, when applicable.
    #[serde(deserialize_with = "nullable")]
    #[schemars(required, schema_with = "nullable_string_schema")]
    pub plan: Option<String>,
}

/// The requested tracker lifecycle. Questions have no representation here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ProjectionState {
    /// The operator has work to do.
    Open,
    /// The operator completed the work.
    Completed,
    /// The operator dismissed the work, optionally recording a reason.
    Dismissed,
}

/// Version 1 of the general human task projection request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HumanTaskProjection {
    /// The wire contract version, always 1.
    pub schema_version: ProjectionVersion,
    /// The external idempotency identity, stable across producer retries.
    pub item_id: String,
    /// The producer that owns the intent.
    pub owner: ProjectionOwner,
    /// The visible task title.
    pub title: String,
    /// The visible task content.
    pub body: String,
    /// Its priority, using the existing plugin API serialization.
    pub priority: Priority,
    /// Normalized repository origins whose work this item concerns.
    pub repositories: Vec<Repository>,
    /// The plan's resolved destination project, or null for a configured human project.
    #[serde(deserialize_with = "nullable")]
    #[schemars(required, schema_with = "nullable_string_schema")]
    pub project: Option<String>,
    /// The configured source through which placement is resolved.
    pub source: SourceName,
    /// The desired lifecycle.
    pub state: ProjectionState,
    /// An optional dismissal reason. Omission and null both preserve absence.
    #[serde(default)]
    pub reason: Option<String>,
}

/// A durable projection's bound tracker identity, suitable for a directly clickable card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HumanTaskProjected {
    /// The qualified task identity.
    pub task: String,
    /// The canonical tracker issue URL.
    pub url: String,
    /// The resolved destination project.
    pub project: String,
    /// The resolved configured source.
    pub source: SourceName,
}

/// Require a nullable key to be present, distinguishing it from an optional reason.
fn nullable<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    Option::<String>::deserialize(deserializer)
}

/// The purpose selected by human task projection writers on existing route declarations.
#[must_use]
pub fn route_purpose() -> crate::config::RoutePurpose {
    crate::config::RoutePurpose::new("human-task").expect("a literal routing identity")
}

fn nullable_string_schema(_generator: &mut SchemaGenerator) -> Schema {
    serde_json::json!({"type": ["string", "null"]})
        .try_into()
        .expect("a literal nullable string schema")
}

/// The schema bundle registers this domain once; new roots stay beside their declaration.
pub(crate) fn schemas() -> [(&'static str, Schema); 2] {
    [
        (
            "HumanTaskProjection",
            schemars::schema_for!(HumanTaskProjection),
        ),
        (
            "HumanTaskProjected",
            schemars::schema_for!(HumanTaskProjected),
        ),
    ]
}
