//! The work items every source is normalised into.

use chrono::{DateTime, Utc};
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

use crate::{NativeId, SourceName};

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
    /// the two every source honours, and [`ItemKind::METADATA_KEY`] is one plugin's —
    /// and `onepipeline.` belongs to that consumer. Every other key is the caller's, and
    /// a source returns it exactly as it holds it.
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
    /// Written down but not yet committed to as work.
    Draft,
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
    /// The item the edge starts at, and the one that **depends on** the other.
    ///
    /// This is the orientation every source reports in, whichever way its own backend
    /// spells the relationship: a GitHub `blockedBy` connection read for `ENG-1` yields
    /// `from: ENG-1`, because `ENG-1` is what depends.
    pub from: DependencyEndpoint,
    /// The item the edge points at, and the one that must finish first.
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
    /// `natively_names` is the kind of item the near item's **own backend** can relate it
    /// to — `Some(ItemKind::Task)` for a GitHub issue, whose `blockedBy` connection holds
    /// issues; `None` for a GitHub draft, which has no such connection at all. An endpoint
    /// of that kind naming an item of `near_source` is refused, because it names an item
    /// the backend itself could hold, and the rule this key exists to serve is the
    /// backend's own relationship first. Naming one's own source is what an unqualified id
    /// does implicitly and what `<near_source>:<native>` does in writing, so both are
    /// refused: which of the two spellings a plan happened to use says nothing about where
    /// the edge belongs.
    ///
    /// An endpoint qualified to a *different* source is never refused. That is the whole
    /// case this key is for: no backend relates an id in a system it knows nothing about.
    ///
    /// # Errors
    ///
    /// Returns a message when the key holds anything other than a list of endpoints, or
    /// holds one the near item's own backend was supposed to name.
    pub fn recorded(
        metadata: &BTreeMap<String, Value>,
        near: &NativeId,
        near_kind: ItemKind,
        near_source: &SourceName,
        natively_names: Option<ItemKind>,
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
        far.into_iter()
            .map(|to| {
                let names_this_source = to
                    .source()
                    .is_none_or(|source| source == near_source.as_str());
                if names_this_source && natively_names == Some(to.kind) {
                    return Err(format!(
                        "{key} on {near} records {to}, which this source can relate \
                         natively; record it as this backend's own dependency and keep \
                         {key} for a far end no relationship here can name",
                        key = Self::RECORDED_KEY
                    ));
                }
                Ok(Self {
                    from: DependencyEndpoint::from_native(near.clone(), near_kind),
                    to,
                    kind: DependencyKind::Blocks,
                })
            })
            .collect()
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
            "description": "A dependency endpoint. A bare string is a native id of the source reporting it, and this decoding reads one as a task; a reader that knows the level it was written at — a source's own configuration, say — may read it at that level instead.",
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

    /// The source segment of a qualified id, or `None` for a native one.
    ///
    /// A native id belongs to whichever source reports it, so `None` reads as "this
    /// source" rather than "no source".
    #[must_use]
    pub fn source(&self) -> Option<&str> {
        match &self.id {
            EndpointIdentity::Qualified(id) => id.split_once(':').map(|(source, _)| source),
            EndpointIdentity::Native(_) => None,
        }
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

impl ItemKind {
    /// The reserved metadata key an item is marked with when its backend cannot say
    /// which kind it is.
    ///
    /// Spelled once, here, for the reason [`Repository::METADATA_KEY`] is: a key under
    /// this product's prefix belongs to the product, and a plugin inventing its own
    /// spelling would collide with the next one to want it.
    ///
    /// Unlike the other two reserved keys, this one obliges **no** source. A backend that
    /// knows its own kinds — folders, native projects — never reads or writes it, and
    /// passes it through as ordinary caller metadata with its JSON type intact, exactly
    /// as it passes through every other key it does not own. `github-projects` is the one
    /// source that needs it, because a GitHub Projects board holds only issues and an
    /// empty project is indistinguishable from a task without it.
    pub const METADATA_KEY: &'static str = "onetaskgraph.item_kind";

    /// The value this kind is marked with under [`Self::METADATA_KEY`].
    #[must_use]
    pub const fn marker(self) -> &'static str {
        match self {
            Self::Task => "task",
            Self::Project => "project",
        }
    }

    /// The kind `metadata` marks, or `None` when it carries no marker at all.
    ///
    /// # Errors
    ///
    /// Returns a message when [`Self::METADATA_KEY`] holds anything other than the two
    /// markers [`Self::marker`] spells.
    pub fn from_metadata(metadata: &BTreeMap<String, Value>) -> Result<Option<Self>, String> {
        let Some(value) = metadata.get(Self::METADATA_KEY) else {
            return Ok(None);
        };
        match value.as_str() {
            Some(marker) if marker == Self::Task.marker() => Ok(Some(Self::Task)),
            Some(marker) if marker == Self::Project.marker() => Ok(Some(Self::Project)),
            _ => Err(format!(
                "{} is {value}; it accepts only {:?} or {:?}",
                Self::METADATA_KEY,
                Self::Project.marker(),
                Self::Task.marker()
            )),
        }
    }
}

/// What a [`DependencyEdge`] means.
///
/// Both variants are read in the one direction [`DependencyEdge::from`] fixes: `from`
/// depends on `to`. This enum said the opposite of that until the orientation was settled,
/// which is why it is spelled out twice rather than once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum DependencyKind {
    /// `from` depends on `to`, and `to` must finish before `from` can.
    // llmlint: ignore[names_match_behavior] `"blocks"` is the approved serialized value, spelled in docs/plugin-protocol.md §4.8 and both generated SDKs; the variant names the kind of dependency, and `from`/`to` carry the direction. Renaming it is a wire change and the contract owner's call.
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
