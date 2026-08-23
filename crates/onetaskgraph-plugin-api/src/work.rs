//! The work items every source is normalised into.

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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
    // llmlint: ignore[invalid_states_unrepresentable] `url: Option<String>` is fixed by the frozen cross-node contract — it appears
    // byte-identically in the task text of every node of this plan, six of which have not
    // dispatched yet and will be written against it, so tightening it to a validated URL
    // newtype here would silently desynchronise this repository from every plugin that
    // follows. Only the contract's owner may amend it; tightening is tracked as post-build
    // follow-up.
    /// Where a human can open this task.
    pub url: Option<String>,
    /// When the source says the task was created.
    pub created_at: Option<DateTime<Utc>>,
    /// When the source says the task last changed.
    pub updated_at: Option<DateTime<Utc>>,
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
    // llmlint: ignore[invalid_states_unrepresentable] `url: Option<String>` is fixed by the frozen cross-node contract — it appears
    // byte-identically in the task text of every node of this plan, six of which have not
    // dispatched yet and will be written against it, so tightening it to a validated URL
    // newtype here would silently desynchronise this repository from every plugin that
    // follows. Only the contract's owner may amend it; tightening is tracked as post-build
    // follow-up.
    /// Where a human can open this project.
    pub url: Option<String>,
    /// When the source says the project was created.
    pub created_at: Option<DateTime<Utc>>,
    /// When the source says the project last changed.
    pub updated_at: Option<DateTime<Utc>>,
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

/// A dependency between two items **of the same source**.
///
/// Cross-source edges are deliberately absent: relating an id in one system to an
/// id in another needs state, and the engine is forbidden to hold any.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DependencyEdge {
    /// The item the edge starts at.
    pub from: NativeId,
    /// The item the edge points at.
    pub to: NativeId,
    /// What the edge means.
    pub kind: DependencyKind,
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
