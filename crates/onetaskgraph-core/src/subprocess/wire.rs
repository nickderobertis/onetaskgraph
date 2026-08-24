//! The envelopes and parameter shapes of `docs/plugin-protocol.md`.
//!
//! Every type here is a restatement of that document, and every *work* type inside one
//! comes from `onetaskgraph-plugin-api` unchanged — the protocol says the JSON shape of a
//! contract type "is what `onetaskgraph schema` emits for the type of the same name", so
//! re-spelling `Task` or `Capabilities` here would create a second place the contract
//! lives and a way for the two to disagree. What this module adds is only what the
//! contract has no type for: the envelope, the handshake, and the per-method wrappers.
//!
//! Nothing here uses `deny_unknown_fields`, and that is §2.1 rather than an oversight: a
//! reader on either side ignores members it does not know, which is what lets a later
//! version add an optional field without a version bump.

use std::collections::BTreeMap;

use onetaskgraph_plugin_api::{
    Capabilities, Direction, NativeId, PageRequest, Project, ProjectQuery, SourceError, Task,
    TaskQuery,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The protocol version this build speaks. `docs/plugin-protocol.md` specifies 1.
pub(crate) const PROTOCOL_VERSION: u32 = 1;

/// One request line: `{ "id": …, "method": …, "params": … }` (§2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Request {
    /// Unique within the connection, echoed in the response.
    pub(crate) id: String,
    /// One of the names in §4, or `initialize`.
    pub(crate) method: String,
    /// Present even when empty, which is why it is not `Option`.
    pub(crate) params: Value,
}

/// One response line: an `id` and exactly one of `result` and `error` (§2).
///
/// Both members are optional here so that "both present" and "neither present" are
/// *representable* — they are the protocol violations §6.3 names, and a shape that could
/// not hold them would turn a violation into a parse error that says something else.
/// [`Response::outcome`] is where the pair is reduced to the one thing it may be.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Response {
    /// The request's `id`, echoed.
    pub(crate) id: String,
    /// Present on success.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) result: Option<Value>,
    /// Present on failure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<SourceError>,
}

impl Response {
    /// A response carrying a result.
    pub(crate) fn ok(id: String, result: Value) -> Self {
        Self {
            id,
            result: Some(result),
            error: None,
        }
    }

    /// A response carrying a failure.
    pub(crate) fn failed(id: String, error: SourceError) -> Self {
        Self {
            id,
            result: None,
            error: Some(error),
        }
    }

    /// The one thing this envelope may say, or `None` when it says both or neither.
    ///
    /// Returning `None` rather than a guess is §6.3: an envelope with both members is a
    /// protocol violation, and picking one of them would run the caller against a shape
    /// the peer did not mean.
    pub(crate) fn outcome(self) -> Option<Result<Value, SourceError>> {
        match (self.result, self.error) {
            (Some(result), None) => Some(Ok(result)),
            (None, Some(error)) => Some(Err(error)),
            _ => None,
        }
    }
}

/// `initialize` parameters (§3).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct InitializeParams {
    /// The version the engine is speaking.
    pub(crate) protocol_version: u32,
    /// For the plugin's diagnostics only.
    pub(crate) engine: EngineIdentity,
    /// The configured name — for error messages only (§3.2).
    pub(crate) source_name: String,
    /// This source's settings, verbatim.
    pub(crate) config: Value,
    /// Only the variables this plugin's configuration names (§3.1).
    pub(crate) secrets: BTreeMap<String, String>,
}

/// Who is asking, for the plugin's diagnostics (§3).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct EngineIdentity {
    /// The engine's name.
    pub(crate) name: String,
    /// The engine's own version. Advisory.
    pub(crate) version: String,
}

/// The `initialize` result (§3).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct InitializeResult {
    /// The version the plugin will speak.
    ///
    /// Optional so that an omitted one is a refusal this engine can *name* — §6.2 makes
    /// omitting it and answering in another version the same failure, and a required
    /// field would surface it as a parse error naming neither version.
    #[serde(default)]
    pub(crate) protocol_version: Option<u32>,
    /// The plugin kind, as the plugin reports it.
    pub(crate) kind: HandshakePluginKind,
    /// Read once; the engine does not ask again.
    pub(crate) capabilities: Capabilities,
}

/// A plugin's non-empty, open-vocabulary kind from the handshake.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub(crate) struct HandshakePluginKind(String);

impl HandshakePluginKind {
    /// Validate a kind at the process boundary.
    pub(crate) fn new(kind: impl Into<String>) -> Result<Self, &'static str> {
        let kind = kind.into();
        if kind.trim().is_empty() {
            Err("plugin kind must contain a non-whitespace character")
        } else {
            Ok(Self(kind))
        }
    }

    /// Recover the peer's spelling after it has been validated.
    pub(crate) fn into_string(self) -> String {
        self.0
    }
}

impl TryFrom<String> for HandshakePluginKind {
    type Error = &'static str;

    fn try_from(kind: String) -> Result<Self, Self::Error> {
        Self::new(kind)
    }
}

impl From<HandshakePluginKind> for String {
    fn from(kind: HandshakePluginKind) -> Self {
        kind.0
    }
}

/// `get_task` and `get_project` parameters (§4.4).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct IdParams {
    /// The source's own opaque id.
    pub(crate) id: NativeId,
}

/// The `get_task` result (§4.4): a task, or `null` when there is no such task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct TaskResult {
    /// The task, or `null`.
    #[serde(default)]
    pub(crate) task: Option<Task>,
}

/// The `get_project` result (§4.4).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ProjectResult {
    /// The project, or `null`.
    #[serde(default)]
    pub(crate) project: Option<Project>,
}

/// `query_tasks` parameters (§4.5).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct TaskQueryParams {
    /// The predicates the plugin is being asked to apply.
    pub(crate) query: TaskQuery,
    /// Where to resume and how much to return.
    pub(crate) page: PageRequest,
}

/// `query_projects` parameters (§4.6).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ProjectQueryParams {
    /// The predicates the plugin is being asked to apply.
    pub(crate) query: ProjectQuery,
    /// Where to resume and how much to return.
    pub(crate) page: PageRequest,
}

/// `labels` parameters (§4.7).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct LabelParams {
    /// Where to resume and how much to return.
    pub(crate) page: PageRequest,
}

/// `task_dependencies` and `project_dependencies` parameters (§4.8).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct DependencyParams {
    /// The item whose edges are wanted.
    pub(crate) id: NativeId,
    /// Which way to walk.
    pub(crate) direction: Direction,
    /// Where to resume and how much to return.
    pub(crate) page: PageRequest,
}
