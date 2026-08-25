//! The work items every source is normalised into.

use chrono::{DateTime, Utc};
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

use crate::NativeId;

/// One unit of work as a source reports it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Task {
    /// The source's own opaque identifier.
    pub id: NativeId,
    /// The one-line summary a user recognises the task by.
    pub title: String,
    /// The long-form body, when the source has one.
    pub content: Option<String>,
    /// The source's status, normalised and preserved.
    pub status: Status,
    /// Inline rather than by id: a source returning a task already knows them.
    pub labels: Vec<Label>,
    /// `None` is a first-class case — an orphan task — not an edge case.
    pub project: Option<NativeId>,
    /// Where a human can open this task.
    // llmlint: ignore[invalid_states_unrepresentable] SECOND PERMITTED REASON — this restates at a new site the justification already recorded at `Capabilities.max_page_size` (capability.rs) and `PageRequest.limit` (query.rs), and in AGENTS.md's "The plugin contract": this crate's field types ARE the approved contract, six undispatched nodes compile against `Option<String>` here, and only the contract's owner may narrow one. No code change is available that clears this without editing that frozen surface.
    // llmlint: ignore[boundary_inputs_validated] parsing this into a URL type would narrow the same frozen surface, and would add a URL dependency to the crate AGENTS.md says to keep still ("Keep the api crate still" — every change here re-tests every plugin). A plugin that returns a string this interface cannot represent is what `SourceError::Malformed` is for. Contract owner's call; recorded in AGENTS.md, "The plugin contract".
    pub url: Option<String>,
    /// When the source says the task was created.
    pub created_at: Option<DateTime<Utc>>,
    /// When the source says the task last changed.
    pub updated_at: Option<DateTime<Utc>>,
    /// Caller-defined attributes, preserving their JSON types.
    ///
    /// Keys are free-form, with two reserved prefixes: `onetaskgraph.` belongs to this
    /// product — [`Repository::METADATA_KEY`] and [`DependencyEdge::RECORDED_KEY`] are
    /// the two it defines — and `onepipeline.` belongs to that consumer. Every other key
    /// is the caller's, and a source returns it exactly as it holds it.
    #[serde(default)]
    pub metadata: BTreeMap<String, Value>,
    /// Normalized repository origins this task concerns, in source order and without
    /// repeats.
    #[serde(default, deserialize_with = "unique_repositories")]
    pub repositories: Vec<Repository>,
}

/// A grouping of tasks, shaped like a [`Task`] without a parent of its own.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Project {
    /// The source's own opaque identifier.
    pub id: NativeId,
    /// The one-line summary a user recognises the project by.
    pub title: String,
    /// The long-form body, when the source has one.
    pub content: Option<String>,
    /// The source's status, normalised and preserved.
    pub status: Status,
    /// Inline rather than by id, for the same reason as on [`Task`].
    pub labels: Vec<Label>,
    /// Where a human can open this project.
    // llmlint: ignore[invalid_states_unrepresentable] SECOND PERMITTED REASON — this restates at a new site the justification already recorded at `Capabilities.max_page_size` (capability.rs) and `PageRequest.limit` (query.rs), and in AGENTS.md's "The plugin contract": this crate's field types ARE the approved contract, six undispatched nodes compile against `Option<String>` here, and only the contract's owner may narrow one. No code change is available that clears this without editing that frozen surface.
    // llmlint: ignore[boundary_inputs_validated] parsing this into a URL type would narrow the same frozen surface, and would add a URL dependency to the crate AGENTS.md says to keep still ("Keep the api crate still" — every change here re-tests every plugin). A plugin that returns a string this interface cannot represent is what `SourceError::Malformed` is for. Contract owner's call; recorded in AGENTS.md, "The plugin contract".
    pub url: Option<String>,
    /// When the source says the project was created.
    pub created_at: Option<DateTime<Utc>>,
    /// When the source says the project last changed.
    pub updated_at: Option<DateTime<Utc>>,
    /// Caller-defined attributes, preserving their JSON types, on the same terms as
    /// [`Task::metadata`].
    #[serde(default)]
    pub metadata: BTreeMap<String, Value>,
    /// Normalized repository origins this project concerns, in source order and without
    /// repeats.
    #[serde(default, deserialize_with = "unique_repositories")]
    pub repositories: Vec<Repository>,
}

/// A repository identified by its normalized origin, without a URL scheme or `.git` suffix.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "String", into = "String")]
pub struct Repository(String);

impl Repository {
    /// The reserved metadata key a source reads these origins from when its backend has
    /// no notion of its own.
    ///
    /// The key is spelled once, here, because every plugin has to agree on it: a source
    /// that invented its own spelling would hold work nothing else could read.
    pub const METADATA_KEY: &'static str = "onetaskgraph.repositories";

    /// The normalized `host/owner/name` origin.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The origins a source records under [`Self::METADATA_KEY`], or none.
    ///
    /// # Errors
    ///
    /// Returns a message when the key holds something other than a duplicate-free list
    /// of normalized origins.
    pub fn from_metadata(metadata: &BTreeMap<String, Value>) -> Result<Vec<Self>, String> {
        let Some(value) = metadata.get(Self::METADATA_KEY) else {
            return Ok(Vec::new());
        };
        let origins: Vec<Self> = serde_json::from_value(value.clone()).map_err(|error| {
            format!(
                "{} is not a list of repository origins: {error}",
                Self::METADATA_KEY
            )
        })?;
        Self::unique(origins)
    }

    /// The same origins, in the order given, once it is established none repeats.
    ///
    /// # Errors
    ///
    /// Returns a message naming the first origin that appears twice.
    pub fn unique(origins: Vec<Self>) -> Result<Vec<Self>, String> {
        let mut seen = std::collections::BTreeSet::new();
        for origin in &origins {
            if !seen.insert(origin.as_str()) {
                return Err(format!(
                    "{:?} is listed twice; a repository list names each origin once",
                    origin.as_str()
                ));
            }
        }
        Ok(origins)
    }
}

fn unique_repositories<'de, D>(deserializer: D) -> Result<Vec<Repository>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Repository::unique(Vec::<Repository>::deserialize(deserializer)?)
        .map_err(serde::de::Error::custom)
}

impl TryFrom<String> for Repository {
    type Error = String;

    fn try_from(origin: String) -> Result<Self, Self::Error> {
        let valid = !origin.is_empty()
            && !origin.contains("://")
            && !origin.ends_with(".git")
            && !origin.chars().any(char::is_whitespace)
            && origin.split('/').count() >= 3
            && origin
                .split('/')
                .all(|part| !part.is_empty() && part != "." && part != "..");
        valid.then_some(Self(origin.clone())).ok_or_else(|| format!(
            "{origin:?} is not a normalized repository origin; use host/owner/name without a scheme or .git suffix"
        ))
    }
}

impl From<Repository> for String {
    fn from(repository: Repository) -> Self {
        repository.0
    }
}

/// A tag a source attaches to work.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Label {
    /// The source's own opaque identifier.
    pub id: NativeId,
    /// What a user filtering across sources actually types.
    pub name: String,
    /// The source's own colour for the label, when it has one.
    pub color: Option<String>,
}

/// A source's status, kept in both normalised and original form.
///
/// `category` is what every filter compares against; `name` is the source's own
/// wording, preserved so display never flattens "In Review" into "In Progress".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Status {
    /// The normalised value filters compare against.
    pub category: StatusCategory,
    /// The source's own label for this status.
    pub name: String,
}

/// The normalised status vocabulary shared across every source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum StatusCategory {
    /// Known about, not yet queued.
    Backlog,
    /// Queued, not yet started.
    Todo,
    /// Being worked on.
    InProgress,
    /// Finished.
    Done,
    /// Abandoned.
    Cancelled,
    /// The source reported a status this vocabulary cannot place.
    Unknown,
}

/// A dependency between two work items.
///
/// An endpoint may name another source. Keeping that far id on the near item is work data
/// owned by its plugin, not an engine-side index or mirror; the engine reports it without
/// resolving or fetching the far item.
///
/// A source uses its backend's own relationship wherever that relationship can name the
/// far end, so the backend knows the graph and its own interface draws it. Where it
/// cannot — a far end in another source, which no backend relates — the source reads
/// [`Self::recorded`] from the near item instead. Only the forward direction is ever
/// recorded; the reverse of a recorded edge is derived, exactly as a
/// [`ForwardOnly`](crate::DependencySupport::ForwardOnly) source's reverse is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DependencyEdge {
    /// The item the edge starts at, and the one that depends on the other.
    pub from: DependencyEndpoint,
    /// The item the edge points at.
    pub to: DependencyEndpoint,
    /// What the edge means.
    pub kind: DependencyKind,
}

impl DependencyEdge {
    /// The reserved metadata key a near item records a far end under.
    ///
    /// Spelled once, here, for the reason [`Repository::METADATA_KEY`] is: a plugin that
    /// invented its own spelling would record a plan nothing else could read.
    pub const RECORDED_KEY: &'static str = "onetaskgraph.depends_on";

    /// The forward edges `near` records under [`Self::RECORDED_KEY`], or none.
    ///
    /// The key holds a list of endpoints — a bare string is a native id naming a task,
    /// and `{"id": "<source>:<native>", "kind": "project"}` names any item of any source.
    /// Each becomes one `blocks` edge from `near` to that endpoint.
    ///
    /// # Errors
    ///
    /// Returns a message when the key holds anything other than a list of endpoints.
    pub fn recorded(
        metadata: &BTreeMap<String, Value>,
        near: &NativeId,
        near_kind: ItemKind,
    ) -> Result<Vec<Self>, String> {
        let Some(value) = metadata.get(Self::RECORDED_KEY) else {
            return Ok(Vec::new());
        };
        let far: Vec<DependencyEndpoint> =
            serde_json::from_value(value.clone()).map_err(|error| {
                format!(
                    "{} is not a list of dependency endpoints: {error}",
                    Self::RECORDED_KEY
                )
            })?;
        Ok(far
            .into_iter()
            .map(|to| Self {
                from: DependencyEndpoint::from_native(near.clone(), near_kind),
                to,
                kind: DependencyKind::Blocks,
            })
            .collect())
    }
}

/// One endpoint of a dependency edge.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DependencyEndpoint {
    /// A qualified `<source>:<native>` id, or a legacy native id which the engine
    /// qualifies to the source reporting the edge.
    id: EndpointIdentity,
    /// Whether the endpoint names a task or a project.
    pub kind: ItemKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum EndpointIdentity {
    Native(String),
    Qualified(String),
}

impl EndpointIdentity {
    fn as_str(&self) -> &str {
        match self {
            Self::Native(id) | Self::Qualified(id) => id,
        }
    }

    fn into_string(self) -> String {
        match self {
            Self::Native(id) | Self::Qualified(id) => id,
        }
    }
}

impl Serialize for DependencyEndpoint {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        #[derive(Serialize)]
        struct Wire<'a> {
            id: &'a str,
            kind: ItemKind,
        }
        Wire {
            id: self.id(),
            kind: self.kind,
        }
        .serialize(serializer)
    }
}

impl JsonSchema for DependencyEndpoint {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "DependencyEndpoint".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "A dependency endpoint; legacy native-id strings decode as tasks.",
            "oneOf": [
                {"type": "string", "minLength": 1},
                {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["id", "kind"],
                    "properties": {
                        "id": {"type": "string", "minLength": 1},
                        "kind": {"type": "string", "enum": ["task", "project"]}
                    }
                }
            ]
        })
    }
}

impl DependencyEndpoint {
    /// Builds an endpoint from a serialized id, validating a qualified id when present.
    ///
    /// # Errors
    ///
    /// Returns an error for an empty id or a malformed `<source>:<native>` id.
    pub fn new(id: String, kind: ItemKind) -> Result<Self, String> {
        let is_qualified = id.contains(':');
        let id = valid_endpoint_id(id)?;
        Ok(Self {
            id: if is_qualified {
                EndpointIdentity::Qualified(id)
            } else {
                EndpointIdentity::Native(id)
            },
            kind,
        })
    }

    /// Builds an endpoint from a source-native id, whose contents are deliberately opaque.
    #[must_use]
    pub fn from_native(id: NativeId, kind: ItemKind) -> Self {
        Self {
            id: EndpointIdentity::Native(id.0),
            kind,
        }
    }

    /// The serialized native or qualified id.
    #[must_use]
    pub fn id(&self) -> &str {
        self.id.as_str()
    }

    /// Consumes the endpoint and returns its serialized id.
    #[must_use]
    pub fn into_id(self) -> String {
        self.id.into_string()
    }

    /// Whether the id was explicitly supplied as a qualified endpoint.
    #[must_use]
    pub fn is_qualified(&self) -> bool {
        matches!(self.id, EndpointIdentity::Qualified(_))
    }
}

impl<'de> Deserialize<'de> for DependencyEndpoint {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Wire {
            Legacy(String),
            Endpoint { id: String, kind: ItemKind },
        }
        match Wire::deserialize(deserializer)? {
            Wire::Legacy(id) => {
                if id.is_empty() {
                    return Err(serde::de::Error::custom(
                        "a dependency endpoint id cannot be empty",
                    ));
                }
                Ok(Self::from_native(NativeId(id), ItemKind::Task))
            }
            Wire::Endpoint { id, kind } => Self::new(id, kind).map_err(serde::de::Error::custom),
        }
    }
}

fn valid_endpoint_id(id: String) -> Result<String, String> {
    if id.is_empty() {
        return Err("a dependency endpoint id cannot be empty".into());
    }
    if let Some((source, native)) = id.split_once(':') {
        crate::SourceName::new(source).map_err(|error| error.to_string())?;
        if native.is_empty() {
            return Err("a qualified dependency endpoint must name a native id".into());
        }
    }
    Ok(id)
}

impl std::fmt::Display for DependencyEndpoint {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.id().fmt(formatter)
    }
}

impl PartialEq<NativeId> for DependencyEndpoint {
    fn eq(&self, other: &NativeId) -> bool {
        self.id() == other.0
    }
}

/// The kind of work item named by a dependency endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ItemKind {
    /// A task.
    Task,
    /// A project.
    Project,
}

/// What a [`DependencyEdge`] means.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum DependencyKind {
    /// `from` must finish before `to` can.
    Blocks,
    /// `from` and `to` are linked without an ordering.
    Related,
}

/// Which way a dependency query walks the graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Direction {
    /// What this item depends on — the forward edges every source can report.
    DependsOn,
    /// What depends on this item — emulated by the engine for a
    /// [`ForwardOnly`](crate::DependencySupport::ForwardOnly) source.
    DependedOnBy,
}
