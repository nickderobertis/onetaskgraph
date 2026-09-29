//! What a caller asks a source for, and how a source hands back more than fits
//! in one answer.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, de::Error as _};
use serde_json::Value;

use crate::{Comment, NativeId, Priority, StatusCategory};

/// A filter over a source's tasks.
///
/// Every field narrows; an empty or `None` field means unfiltered.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct TaskQuery {
    /// Free-text search, when the caller asked for one.
    pub text: Option<TextQuery>,
    /// Label membership.
    pub labels: LabelFilter,
    /// Status categories to keep. Empty means unfiltered.
    pub statuses: Vec<StatusCategory>,
    /// Which project the task belongs to.
    pub project: ProjectFilter,
    /// Priorities to keep: a task matches when its priority is any one of these. Empty means
    /// unfiltered.
    ///
    /// Defaulted when absent and left out of the wire when empty, so a plugin written before
    /// there were priorities reads exactly the query it read before — and, declaring no
    /// [`Capabilities::filter_by_priority`](crate::Capabilities::filter_by_priority), is never
    /// handed one it would have to ignore.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(!skip_serializing_if)]
    pub priorities: Vec<Priority>,
    /// Comment activity to keep: a task matches when **at least one of its comments** was
    /// created, or last edited, at or after this instant — its
    /// [`Comment::created_at`](crate::Comment::created_at) or
    /// [`Comment::updated_at`](crate::Comment::updated_at) is at or after it. A task with no
    /// comments never matches, and a comment deleted before the query is not a match. `None`
    /// means unfiltered. An RFC 3339 string on the wire.
    ///
    /// Defaulted when absent and left out of the wire when `None`, so a plugin written before
    /// there was comment activity reads exactly the query it read before — and, declaring no
    /// [`Capabilities::filter_by_comment_activity`](crate::Capabilities::filter_by_comment_activity),
    /// is never handed one it would have to ignore.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commented_since: Option<DateTime<Utc>>,
    /// Caller-defined metadata values to keep: a task matches when **every** one of these
    /// holds of its [`Task::metadata`](crate::Task::metadata). Empty means unfiltered.
    ///
    /// Defaulted when absent and left out of the wire when empty, so a plugin written before
    /// there were metadata matches reads exactly the query it read before — and, declaring no
    /// [`Capabilities::filter_by_metadata`](crate::Capabilities::filter_by_metadata), is never
    /// handed one it would have to ignore.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(!skip_serializing_if)]
    pub metadata: Vec<MetadataMatch>,
    /// The copy origin to keep: a task matches when its
    /// [`ORIGIN_KEY`](Self::ORIGIN_KEY) metadata entry is a string equal to this, exactly.
    /// `None` means unfiltered.
    ///
    /// A **qualified id** — `<source>:<native>` — spelled exactly as a copy stores it. A
    /// plugin never constructs or interprets one: it compares this string with the one it
    /// holds, byte for byte, and nothing else, which is why it is a string here rather than
    /// the engine's own qualified-id type.
    ///
    /// Defaulted when absent and left out of the wire when `None`, on the terms
    /// [`metadata`](Self::metadata) gives, with
    /// [`Capabilities::filter_by_origin`](crate::Capabilities::filter_by_origin).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    // llmlint: ignore[boundary_inputs_validated, invalid_states_unrepresentable] The contract's owner ruled this an opaque string (planner reply c-9fa2fba58e652a06127c709c738311f9): a plugin never constructs or interprets a qualified id, which is why `GlobalId` is absent from this crate (AGENTS.md, "The plugin contract"), so this crate cannot check the syntax without interpreting it. Every source compares it byte for byte with the string it holds, so a value naming no qualified id matches nothing rather than something wrong, and the one boundary a person types it at — `task list --origin` — parses it as a `GlobalId` and refuses a malformed one before any source is asked.
    pub origin: Option<String>,
}

impl TaskQuery {
    /// The reserved metadata key a copied item records the qualified id it was copied from
    /// under, which [`origin`](Self::origin) is compared with.
    ///
    /// The engine owns this key; it is restated here so a source applying the predicate
    /// natively and the engine narrowing for one that does not read the same entry.
    /// `scripts/check-origin-key-spelling.sh` holds this spelling to the engine's own.
    pub const ORIGIN_KEY: &'static str = "onetaskgraph.origin";

    /// Whether `metadata` — one task's — satisfies every [`metadata`](Self::metadata) match:
    /// always when the query carries none.
    ///
    /// The one statement of the predicate's meaning, so a source applying it natively and the
    /// engine narrowing for a source that does not cannot answer the same store differently.
    #[must_use]
    pub fn metadata_matches(&self, metadata: &BTreeMap<String, Value>) -> bool {
        self.metadata.iter().all(|wanted| wanted.holds(metadata))
    }

    /// Whether `metadata` — one task's — satisfies [`origin`](Self::origin): always when the
    /// query carries none, and otherwise exactly when its [`ORIGIN_KEY`](Self::ORIGIN_KEY)
    /// entry is a string equal to it.
    #[must_use]
    pub fn origin_matches(&self, metadata: &BTreeMap<String, Value>) -> bool {
        self.origin.as_ref().is_none_or(|origin| {
            metadata
                .get(Self::ORIGIN_KEY)
                .and_then(Value::as_str)
                .is_some_and(|held| held == origin)
        })
    }

    /// Whether `comments` — one task's — satisfy [`commented_since`](Self::commented_since):
    /// always when the query carries no instant, and otherwise exactly when one of them was
    /// created or last edited at or after it.
    ///
    /// The one statement of the predicate's meaning, so a source applying it natively and the
    /// engine narrowing for a source that does not cannot answer the same store differently.
    #[must_use]
    pub fn comments_match<'a>(&self, comments: impl IntoIterator<Item = &'a Comment>) -> bool {
        let Some(since) = self.commented_since else {
            return true;
        };
        comments
            .into_iter()
            .any(|comment| commented_at_or_after(comment, since))
    }
}

/// Whether one comment was created, or last edited, at or after `since`.
///
/// A comment whose source gave neither time is not evidence of activity, so it never matches.
fn commented_at_or_after(comment: &Comment, since: DateTime<Utc>) -> bool {
    comment.created_at.is_some_and(|at| at >= since)
        || comment.updated_at.is_some_and(|at| at >= since)
}

/// One caller-defined metadata value a task must hold.
///
/// The location is a top-level metadata key — which may itself contain dots, such as
/// `orchestrator.follow-up` — and zero or more nested object keys below it. The match holds
/// when the value there is a JSON **string** equal to [`value`](Self::value), case-sensitively;
/// a number, a boolean, an array, an object, a missing key and a path through a non-object
/// all fail it.
///
/// Neither the key nor a nested segment may be empty, which is checked wherever one is built,
/// deserialized included, so a source handed one never has to ask what an empty location
/// means.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct MetadataMatch {
    /// The top-level metadata key.
    key: String,
    /// Nested object keys under [`key`](Self::key), outermost first. Empty names the
    /// top-level value itself.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(!skip_serializing_if)]
    path: Vec<String>,
    /// The string the value there must equal.
    value: String,
}

impl MetadataMatch {
    /// A match for `value` at `key` and the nested `path` under it.
    ///
    /// # Errors
    ///
    /// Returns a message naming the location and what to write instead when the key or one
    /// of the segments is empty.
    pub fn new(
        key: impl Into<String>,
        path: Vec<String>,
        value: impl Into<String>,
    ) -> Result<Self, String> {
        let key = key.into();
        if key.is_empty() || path.iter().any(String::is_empty) {
            return Err(format!(
                "the metadata location {:?} has an empty key or segment; next: name a key, and \
                 a non-empty object key for each nested segment",
                std::iter::once(key.as_str())
                    .chain(path.iter().map(String::as_str))
                    .collect::<Vec<_>>()
                    .join("/")
            ));
        }
        Ok(Self {
            key,
            path,
            value: value.into(),
        })
    }

    /// The top-level metadata key.
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    /// The nested object keys under [`key`](Self::key), outermost first.
    #[must_use]
    pub fn path(&self) -> &[String] {
        &self.path
    }

    /// The string the value at this location must equal.
    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }

    /// Whether `metadata` holds [`value`](Self::value) as a string at this location.
    #[must_use]
    pub fn holds(&self, metadata: &BTreeMap<String, Value>) -> bool {
        let mut held = metadata.get(&self.key);
        for segment in &self.path {
            held = held
                .and_then(Value::as_object)
                .and_then(|object| object.get(segment));
        }
        held.and_then(Value::as_str) == Some(self.value.as_str())
    }
}

impl<'de> Deserialize<'de> for MetadataMatch {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        /// The wire shape, before its location is checked.
        #[derive(Deserialize)]
        struct Wire {
            key: String,
            #[serde(default)]
            path: Vec<String>,
            value: String,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.key, wire.path, wire.value).map_err(D::Error::custom)
    }
}

/// A filter over a source's projects.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct ProjectQuery {
    /// Free-text search, when the caller asked for one.
    pub text: Option<TextQuery>,
    /// Label membership.
    pub labels: LabelFilter,
    /// Status categories to keep. Empty means unfiltered.
    pub statuses: Vec<StatusCategory>,
}

/// A filter over a source's documents.
///
/// No statuses, deliberately: a [`Document`](crate::Document) is not work and carries no
/// status, so there is nothing here for a status filter to compare against.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct DocumentQuery {
    /// Free-text search, when the caller asked for one.
    pub text: Option<TextQuery>,
    /// Label membership.
    pub labels: LabelFilter,
    /// Which project the document lives in.
    pub project: ProjectFilter,
}

/// A free-text search and the fields it searches.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TextQuery {
    /// What the user typed.
    pub terms: String,
    /// Where to look for it.
    pub fields: TextFields,
}

/// Which fields a [`TextQuery`] searches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum TextFields {
    /// Titles only.
    Title,
    /// Bodies only.
    Content,
    /// Either one matching is a match.
    TitleOrContent,
}

/// Label membership, by **name** rather than by id.
///
/// A label id is per-source; a user filtering across sources types a word. Names
/// are matched case-insensitively for the same reason.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct LabelFilter {
    /// Keep an item carrying at least one of these.
    pub any_of: Vec<String>,
    /// Keep an item carrying all of these.
    pub all_of: Vec<String>,
    /// Drop an item carrying any of these.
    pub none_of: Vec<String>,
}

impl LabelFilter {
    /// Whether this filter constrains anything at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.any_of.is_empty() && self.all_of.is_empty() && self.none_of.is_empty()
    }
}

/// Which project a task must belong to.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ProjectFilter {
    /// No constraint.
    #[default]
    Any,
    /// Only tasks belonging to no project.
    Orphans,
    /// Only tasks belonging to this project.
    Is(NativeId),
}

/// One step of a walk through a result set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PageRequest {
    /// Where to resume, or `None` to start at the beginning.
    pub cursor: Option<Cursor>,
    /// The most items to return, at least 1. A source may return fewer, never more.
    #[serde(deserialize_with = "non_zero_limit")]
    // llmlint: ignore[invalid_states_unrepresentable] this field's wire shape is frozen by the plugin contract every source is written against; only the contract's owner may change it, and tightening it is post-build follow-up.
    pub limit: u32,
}

/// Reject a zero page size where a request is read, so an ask for no rows never reaches a
/// source as if it were an ask for one.
fn non_zero_limit<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u32, D::Error> {
    let value = u32::deserialize(deserializer)?;
    if value == 0 {
        return Err(D::Error::custom(
            "limit must be at least 1; a page of no rows is not a page",
        ));
    }
    Ok(value)
}

/// One page of results, and where to pick up.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Page<T> {
    /// This page's items, in the source's stable order.
    pub items: Vec<T>,
    /// The cursor for the next page, or `None` when the walk is exhausted.
    pub next: Option<Cursor>,
}

impl<T> Page<T> {
    /// The last page of a walk: these items and nothing after them.
    #[must_use]
    pub fn last(items: Vec<T>) -> Self {
        Self { items, next: None }
    }
}

/// A plugin-defined resume token. The engine stores and returns one; it never
/// interprets one.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct Cursor(pub String);
