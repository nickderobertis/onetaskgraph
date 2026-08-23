//! What a caller asks a source for, and how a source hands back more than fits
//! in one answer.

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, de::Error as _};

use crate::{NativeId, StatusCategory};

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
