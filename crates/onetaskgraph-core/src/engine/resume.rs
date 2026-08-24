//! Where a walk resumes, per source and per stream.
//!
//! This is the inside of the engine's own [`PageToken`](crate::PageToken), and nothing
//! outside this crate names it. A source's [`Cursor`] passes through untouched — the
//! engine stores one and hands it back, exactly as a plugin stores nothing of the
//! engine's page token.
//!
//! The `skip` beside the cursor is the engine counting **its own** rows, never
//! interpreting the source's. It exists because compensation narrows a source page in
//! memory: a page of 50 that yields 7 surviving rows, of which the caller's page had
//! room for 3, has to resume at "the page beginning at this cursor, four surviving rows
//! in". The alternative would be holding the other four somewhere between calls, which
//! is exactly the caching this product does not do.

use onetaskgraph_plugin_api::{Cursor, SourceName};
use serde::{Deserialize, Serialize};

/// Which of a source's streams a state addresses.
///
/// A verb reads one stream per source, except `search --kind both`, which reads two —
/// so the key is the pair rather than the source alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum StreamKind {
    /// The one stream a single-entity verb reads.
    Items,
    /// The task half of a search over both entities.
    Tasks,
    /// The project half of a search over both entities.
    Projects,
}

impl StreamKind {
    /// What this stream is, for a message a user has to act on.
    pub(crate) fn describe(self) -> &'static str {
        match self {
            Self::Items => "a single-entity listing",
            Self::Tasks => "the task half of a search",
            Self::Projects => "the project half of a search",
        }
    }
}

/// The place one walk of one stream picks up from.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub(crate) struct Resume {
    /// The source cursor whose page the next row is in, or `None` for the beginning.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<Cursor>,
    /// How many surviving rows of that page were already delivered.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub skip: u32,
}

/// One stream's place, as the page token carries it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct StreamState {
    /// The configured source this stream belongs to.
    pub source: SourceName,
    /// Which of that source's streams it is.
    pub stream: StreamKind,
    /// Where to pick up.
    #[serde(flatten)]
    pub resume: Resume,
    /// Whether the next page hands this stream's row back first.
    ///
    /// Rows come back one from each stream in turn, and a page boundary can fall in the
    /// middle of one of those rounds: a page of three rows over two streams ends having
    /// taken two from the first and one from the second, with the second's turn still
    /// owed. Without this flag the next page would begin its rounds at the first stream
    /// again, and the walk as a whole would stop alternating — every row would still come
    /// back exactly once, but not in the order this product documents.
    ///
    /// Carried by stream rather than as a position, because a stream the previous page
    /// exhausted is not in the next one and every position after it would shift.
    ///
    /// At most one stream in a token carries it, and a token that resumes a round that
    /// ended evenly carries it on none — which is why it is omitted when false, leaving
    /// the tokens that do not need it exactly as they were.
    #[serde(default, skip_serializing_if = "is_false")]
    pub first: bool,
}

/// Keeps a token that resumes at the beginning of a stream down to its source and kind.
fn is_zero(value: &u32) -> bool {
    *value == 0
}

/// Keeps a token resuming an evenly ended round down to its source, kind and place.
fn is_false(value: &bool) -> bool {
    !*value
}
