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
    Capabilities, Comment, CommentBody, Direction, Document, DocumentQuery, ItemWrite, MetadataKey,
    Metering, NativeId, NewComment, Page, PageRequest, Project, ProjectQuery, SourceError, Status,
    StatusCategory, Task, TaskQuery, TaskRef, WriteSupport,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The protocol version this build speaks. `docs/plugin-protocol.md` specifies 2.
///
/// Version 2 added `delete_task` and `delete_project` (§4.10). The handshake matches
/// versions exactly, so a plugin speaking 1 is refused by name rather than being asked
/// for a method it has never heard of halfway through undoing a copy.
pub(crate) const PROTOCOL_VERSION: u32 = 2;

/// Every status category this build knows, in the order the vocabulary lists them (§3.5).
pub(crate) const STATUS_VOCABULARY: [StatusCategory; 8] = [
    StatusCategory::Draft,
    StatusCategory::Backlog,
    StatusCategory::Todo,
    StatusCategory::Queued,
    StatusCategory::InProgress,
    StatusCategory::Done,
    StatusCategory::Cancelled,
    StatusCategory::Unknown,
];

/// Whether `category` is one a peer that lists no statuses was not written against (§3.5).
///
/// A wildcard-free match, so a category added to the vocabulary does not compile until
/// somebody decides here whether a peer that predates it can be handed it.
pub(crate) const fn after_the_first_vocabulary(category: StatusCategory) -> bool {
    match category {
        StatusCategory::Queued => true,
        StatusCategory::Draft
        | StatusCategory::Backlog
        | StatusCategory::Todo
        | StatusCategory::InProgress
        | StatusCategory::Done
        | StatusCategory::Cancelled
        | StatusCategory::Unknown => false,
    }
}

/// One category as the wire spells it.
pub(crate) fn spelled(category: StatusCategory) -> String {
    serde_json::to_value(category)
        .ok()
        .and_then(|word| word.as_str().map(str::to_owned))
        .expect("a status category serialises as its own word")
}

/// Every category this build knows, as a handshake lists them.
pub(crate) fn vocabulary() -> Vec<String> {
    STATUS_VOCABULARY.into_iter().map(spelled).collect()
}

/// Whether a handshake's `statuses` member lists every category this build knows.
///
/// An absent member is the first vocabulary, so it lists none of the categories added
/// after it.
pub(crate) fn knows_every_category(statuses: Option<&[String]>) -> bool {
    let Some(listed) = statuses else {
        return false;
    };
    STATUS_VOCABULARY
        .into_iter()
        .filter(|category| after_the_first_vocabulary(*category))
        .all(|category| listed.contains(&spelled(category)))
}

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
    /// The configured name — for error messages, and for recognising itself in a
    /// recorded qualified id (§3.2).
    pub(crate) source_name: String,
    /// This source's settings, verbatim.
    pub(crate) config: Value,
    /// Only the variables this plugin's configuration names (§3.1).
    pub(crate) secrets: BTreeMap<String, String>,
    /// The status categories this engine knows (§3.5).
    ///
    /// Optional, and absent means the first vocabulary: an engine written before `queued`
    /// sends none, and a plugin answering it reports a category outside that vocabulary as
    /// `unknown`, keeping its name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) statuses: Option<Vec<String>>,
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
    /// Whether this plugin can be written through (§3.3).
    ///
    /// Optional, and absent means [`WriteSupport::Unsupported`]: §2.1 lets a later
    /// version add a member without a version bump, and a plugin written against version
    /// 1 before there was a write side says nothing here and is read as the read-only
    /// source it is.
    #[serde(default)]
    pub(crate) writes: Option<WriteSupport>,
    /// Whether this plugin answers `metering` (§3.4).
    ///
    /// Optional, and absent means it does not: a plugin written before there was metering
    /// says nothing here, is never sent the method, and is reported as not metering.
    #[serde(default)]
    pub(crate) meters: bool,
    /// The status categories this plugin knows (§3.5).
    ///
    /// Optional, and absent means the first vocabulary — the seven categories before
    /// `queued` — so a plugin written before there was a `queued` is never handed one.
    /// Strings rather than categories, so a plugin listing a category this engine has never
    /// heard of is read rather than refused.
    #[serde(default)]
    pub(crate) statuses: Option<Vec<String>>,
    /// Whether this plugin answers `set_task_status` and `set_delivered_by`, and holds a
    /// task's `delivers` and `delivered_by` (§3.6).
    ///
    /// Optional, and absent means it does not: such a plugin is never sent either method, and
    /// never handed a task carrying either list.
    #[serde(default)]
    pub(crate) task_updates: bool,
    /// Whether this plugin answers `set_task_metadata`, `set_project_metadata` and
    /// `set_document_metadata` (§3.7).
    ///
    /// Optional, and absent means it does not: such a plugin is never sent any of the three,
    /// and each is refused by name before anything is sent.
    #[serde(default)]
    pub(crate) metadata_updates: bool,
}

/// The `metering` result (§4.14).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct MeteringResult {
    /// The plugin's running totals, or `null` when it has none to give.
    #[serde(default)]
    pub(crate) metering: Option<Metering>,
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

/// The `get_document` result (§4.11): a document, or `null` when there is no such one.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct DocumentResult {
    /// The document, or `null`.
    #[serde(default)]
    pub(crate) document: Option<Document>,
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

/// `query_documents` parameters (§4.11).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct DocumentQueryParams {
    /// The predicates the plugin is being asked to apply.
    pub(crate) query: DocumentQuery,
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

/// `write_task` parameters (§4.9).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct TaskWriteParams {
    /// The item to create or update, and what to write into it.
    pub(crate) write: ItemWrite<Task>,
}

/// `write_project` parameters (§4.9).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ProjectWriteParams {
    /// The item to create or update, and what to write into it.
    pub(crate) write: ItemWrite<Project>,
}

/// `write_document` parameters (§4.12).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct DocumentWriteParams {
    /// The document to create or update, and what to write into it.
    pub(crate) write: ItemWrite<Document>,
}

/// `delete_task`, `delete_project` and `delete_document` parameters (§4.10, §4.12).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct DeleteParams {
    /// The destination item to remove.
    pub(crate) id: NativeId,
}

/// `task_comments` parameters (§4.15).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CommentsParams {
    /// The task whose comments are wanted.
    pub(crate) task: NativeId,
    /// Where to resume and how much to return.
    pub(crate) page: PageRequest,
}

/// The `task_comments` result (§4.15): a page, or `null` when there is no such task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CommentsResult {
    /// The page, or `null`.
    #[serde(default)]
    pub(crate) page: Option<Page<Comment>>,
}

/// `add_comment` parameters (§4.16).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AddCommentParams {
    /// The task to comment on.
    pub(crate) task: NativeId,
    /// What to add.
    pub(crate) comment: NewComment,
}

/// `edit_comment` parameters (§4.16).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct EditCommentParams {
    /// The task the comment is on.
    pub(crate) task: NativeId,
    /// The comment to edit.
    pub(crate) comment: NativeId,
    /// Its new body.
    pub(crate) body: CommentBody,
}

/// `delete_comment` parameters (§4.16).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct DeleteCommentParams {
    /// The task the comment is on.
    pub(crate) task: NativeId,
    /// The comment to remove.
    pub(crate) comment: NativeId,
}

/// The `add_comment` and `edit_comment` result (§4.16): the comment as the source now holds
/// it, or `null` when there is no such task or no such comment on it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CommentResult {
    /// The comment, or `null`.
    #[serde(default)]
    pub(crate) comment: Option<Comment>,
}

/// The `delete_comment` result (§4.16): the id removed, or `null` when there was nothing
/// under it to remove.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct DeletedCommentResult {
    /// The removed comment's id, or `null`.
    #[serde(default)]
    pub(crate) deleted: Option<NativeId>,
}

/// `set_task_status` parameters (§4.17).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct StatusParams {
    /// The task whose status is set.
    pub(crate) id: NativeId,
    /// The category to set it to.
    pub(crate) category: StatusCategory,
}

/// The `set_task_status` result (§4.17): the status as the plugin now reads it, or `null`
/// when there is no such task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct StatusResult {
    /// The status, or `null`.
    #[serde(default)]
    pub(crate) status: Option<Status>,
}

/// `set_delivered_by` parameters (§4.17).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct DeliveredByParams {
    /// The task whose list is replaced.
    pub(crate) id: NativeId,
    /// The whole list it holds afterwards.
    pub(crate) delivered_by: Vec<TaskRef>,
}

/// The `set_delivered_by` result (§4.17): the list the task now holds, or `null` when there
/// is no such task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct DeliveredByResult {
    /// The list, or `null`.
    #[serde(default)]
    pub(crate) delivered_by: Option<Vec<TaskRef>>,
}

/// `set_task_metadata`, `set_project_metadata` and `set_document_metadata` parameters
/// (§4.18).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct MetadataParams {
    /// The record whose metadata key is set.
    pub(crate) id: NativeId,
    /// The key, a caller's own dotted key outside the `onetaskgraph.` namespace.
    pub(crate) key: MetadataKey,
    /// The value to hold under it: any JSON, `null` included.
    pub(crate) value: Value,
}

/// The result of any write method (§4.9, §4.12): the id the destination holds the item
/// under.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct WriteResult {
    /// The destination's own id for the item that was written.
    pub(crate) id: NativeId,
}
