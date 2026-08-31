//! The write seam a destination is reached through, and the two refusals a source with
//! nothing on one side of the contract answers with.
//!
//! [`TaskSource`](crate::TaskSource) is a read interface, and it stays one for every
//! source that has nothing to write into: both write methods are defaulted, so a source
//! that cannot be written needs no edit and keeps refusing by saying so. What a source
//! opts into is [`WriteSupport::Supported`], and what that opt-in owes is real
//! implementations of [`TaskSource::write_task`](crate::TaskSource::write_task) and
//! [`TaskSource::write_project`](crate::TaskSource::write_project).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{DependencyEdge, NativeId, SourceError};

/// Whether a source can be written through at all.
///
/// Deliberately its own enum rather than a reuse of [`Support`](crate::Support): that one
/// says whether a source applies a *predicate* itself, and the engine compensates for an
/// `Unsupported` there by narrowing a wider result. There is no compensating for a
/// destination that cannot be written — the copy is refused, naming the source and the
/// plugin behind it — so conflating the two would invite an engine-side workaround for a
/// case that has none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum WriteSupport {
    /// The source creates and updates items through its own write interface.
    Supported,
    /// The source has no write side; a copy naming it as a destination is refused.
    Unsupported,
}

impl WriteSupport {
    /// Whether this source can be written through.
    #[must_use]
    pub fn is_supported(self) -> bool {
        matches!(self, Self::Supported)
    }
}

/// One create-or-update of a work item at a destination.
///
/// The two cases are one type because a destination decides between them by exactly one
/// question — is there an item here to update — and the engine has already answered it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ItemWrite<T> {
    /// The destination item to update, or `None` to create one.
    ///
    /// A source handed a `target` it does not hold returns [`SourceError::Refused`]
    /// rather than creating: the engine established that id before asking, so an absent
    /// one is a race the destination must not paper over.
    pub target: Option<NativeId>,
    /// The item as the destination should hold it once the write lands.
    ///
    /// Its `id` is the id the item was read under at the **source**. A destination
    /// updating an item addresses [`target`](Self::target) and ignores it; a destination
    /// creating one may derive a name from it and is free not to. Its `url`,
    /// `created_at` and `updated_at` are the destination's own and are never written.
    pub item: T,
    /// The forward dependency edges the copy read, with their far ends already resolved.
    ///
    /// Each edge's `from` is the item being written as the *source* named it, so a
    /// destination reads the `to` and the `kind` of each and supplies its own near end.
    /// A `to` naming another source is qualified and stays that way; a `to` inside the
    /// copied set arrives as the destination's own native id.
    #[serde(default)]
    pub depends_on: Vec<DependencyEdge>,
}

/// The refusal a source with no write side answers a write with.
///
/// Spelled once so every unwritten source refuses in the same words, and so a plugin's
/// refusal and the engine's own message about it cannot describe different things.
#[must_use]
pub fn unwritable(kind: &str) -> SourceError {
    SourceError::Refused {
        message: format!("the {kind} plugin cannot be written"),
    }
}

/// The refusal a source with no documents answers a document read with.
///
/// Spelled once beside [`unwritable`], and here rather than beside the reads it answers,
/// because the two are the same kind of thing: a source saying it does not have that side
/// of the contract at all. Every document-free source therefore refuses in the same words,
/// and a plugin's refusal cannot describe something different from the engine's own
/// message about it.
///
/// The two document *writes* reuse [`unwritable`]: a source with no write side refuses a
/// document write for the reason it refuses every other write, and saying it twice in two
/// wordings would make one refusal read as two.
#[must_use]
pub fn documentless(kind: &str) -> SourceError {
    SourceError::Refused {
        message: format!("the {kind} plugin has no documents"),
    }
}
