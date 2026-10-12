//! What a source tells the store's local cache of remote sources.
//!
//! The cache itself is the engine's: it keys an item on the [`Task::version`] its source
//! gives, serves it only at that version or a later one, and holds nothing a source did not
//! hand it. A source says three things here, and nothing else about the cache reaches it:
//!
//! - which parts of a task its change token vouches for — [`VersionCoverage`] — so a part it
//!   does not cover is never served from the cache;
//! - what a write that expects one version of a task may take instead of reading it — an
//!   opaque [`TaskSource::write_context`](crate::TaskSource::write_context) and
//!   [`TaskSource::board_context`](crate::TaskSource::board_context), handed back through
//!   [`TaskSource::prime_write`](crate::TaskSource::prime_write);
//! - what each request of such a write did — a [`WriteJournal`] — so the engine can report
//!   every part's outcome from the mutation's own response, and advance its entry only to
//!   content that response vouches for.
//!
//! [`Task::version`]: crate::Task::version

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Asset, Comment, DependencyEdge, Task};

/// Whether a source's change token vouches for one part of a task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "coverage", rename_all = "kebab-case")]
pub enum Coverage {
    /// Two equal tokens say this part did not change.
    Covered,
    /// The token says nothing about this part, so a cached copy of it is never served.
    Uncovered {
        /// Why, as a reader of a read that went to the source is told.
        reason: String,
    },
}

impl Coverage {
    /// The reason this part is not covered, or `None` when it is.
    #[must_use]
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Covered => None,
            Self::Uncovered { reason } => Some(reason),
        }
    }
}

/// Which parts of a task a source's change token vouches for.
///
/// Two parts, because they are read apart: a task's own fields — title, content, status,
/// priority, labels, metadata, edges and the image assets its content names — and its
/// comments.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct VersionCoverage {
    /// The task's own fields.
    pub fields: Coverage,
    /// Its comments.
    pub comments: Coverage,
}

impl VersionCoverage {
    /// A token covering every part of a task.
    #[must_use]
    pub fn whole() -> Self {
        Self {
            fields: Coverage::Covered,
            comments: Coverage::Covered,
        }
    }

    /// A token covering no part of a task, for `reason`.
    #[must_use]
    pub fn none(reason: impl Into<String>) -> Self {
        let reason = reason.into();
        Self {
            fields: Coverage::Uncovered {
                reason: reason.clone(),
            },
            comments: Coverage::Uncovered { reason },
        }
    }
}

/// What one request a write sent came to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "outcome", rename_all = "kebab-case")]
pub enum PartOutcome {
    /// The backend answered that it was applied.
    Landed,
    /// The backend answered that it was refused.
    Rejected {
        /// What the backend said.
        error: String,
        /// Whether it was refused because an id the write took from a primed context — an
        /// item's node id, a field's or an option's — no longer resolves. Such a part is
        /// sent again once, after one read.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        #[schemars(!skip_serializing_if)]
        stale: bool,
    },
    /// The request left and no answer came back, so whether it was applied cannot be known.
    /// Never sent again: doing so could apply it twice.
    Unknown {
        /// What went wrong.
        error: String,
    },
}

/// What one request of a write was.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RequestKind {
    /// A read the write needed: the write fell back to reading its target.
    Read,
    /// A mutation: a part of the write itself.
    Mutation,
}

/// One task as a write's own response showed it whole: everything an entry of the cache
/// holds, at the version that response determines.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WholeTask {
    /// The task, its [`Task::version`] the version the response determines.
    pub task: Task,
    /// Every comment it holds, oldest first.
    pub comments: Vec<Comment>,
    /// The image assets it holds.
    pub assets: Vec<Asset>,
    /// Its forward dependency edges, when the response carried them whole.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edges: Option<Vec<DependencyEdge>>,
}

/// One request a source sent while a journal was open.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct JournalEntry {
    /// Which part of the write it was, in the source's own words — `fields`, `issue`,
    /// `comment` — or, for a read, what was read.
    pub part: String,
    /// Whether it was a read or a mutation.
    pub kind: RequestKind,
    /// What it came to.
    #[serde(flatten)]
    pub outcome: PartOutcome,
    /// Each field the part set, with the value the response shows it holding.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub fields: BTreeMap<String, Value>,
    /// The task's version after this request, when its response determines it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// The whole task, when the response carried every part an entry holds at `version`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub whole: Option<Box<WholeTask>>,
}

/// Every request a source sent between [`TaskSource::begin_journal`] and
/// [`TaskSource::end_journal`], in the order sent.
///
/// [`TaskSource::begin_journal`]: crate::TaskSource::begin_journal
/// [`TaskSource::end_journal`]: crate::TaskSource::end_journal
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WriteJournal {
    /// The requests, oldest first.
    pub requests: Vec<JournalEntry>,
}

impl WriteJournal {
    /// Whether any request was a read.
    #[must_use]
    pub fn read(&self) -> bool {
        self.requests
            .iter()
            .any(|entry| entry.kind == RequestKind::Read)
    }

    /// The mutations, oldest first.
    pub fn mutations(&self) -> impl Iterator<Item = &JournalEntry> {
        self.requests
            .iter()
            .filter(|entry| entry.kind == RequestKind::Mutation)
    }

    /// Whether a mutation was refused for a stale primed id.
    #[must_use]
    pub fn stale(&self) -> bool {
        self.mutations()
            .any(|entry| matches!(entry.outcome, PartOutcome::Rejected { stale: true, .. }))
    }

    /// Whether a mutation's outcome cannot be known.
    #[must_use]
    pub fn unknown(&self) -> bool {
        self.mutations()
            .any(|entry| matches!(entry.outcome, PartOutcome::Unknown { .. }))
    }
}
