//! Comments on a task, and the one refusal a source with none answers with.
//!
//! A comment is the one part of a task a person adds to in pieces, after the task exists
//! and without rewriting it: evidence appended to an issue somebody else opened. So it is
//! not a field of [`Task`](crate::Task) — a copy writes a task whole, and nothing a copy
//! does may add, change or remove a comment — but a thing of its own, read and written
//! through its own four methods of [`TaskSource`](crate::TaskSource).

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{NativeId, SourceError};

/// One comment on a task, as its source holds it.
///
/// `id` and `body` are always there; every other member is `None` when the source did not
/// give it, which is not the same as the source saying it is empty.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Comment {
    /// The comment's own id, exactly as its source issued it.
    ///
    /// Opaque, as every [`NativeId`] is, and never qualified: a comment is addressed through
    /// the qualified id of the task it is on, so its own id needs no source of its own.
    pub id: NativeId,
    /// Who wrote it, in the source's own spelling of a person.
    pub author: Option<String>,
    /// When the source says it was written.
    pub created_at: Option<DateTime<Utc>>,
    /// When the source says it last changed.
    pub updated_at: Option<DateTime<Utc>>,
    /// What it says, byte for byte as it was written — a trailing newline included.
    pub body: String,
    /// Where a person can open it.
    // llmlint: ignore[invalid_states_unrepresentable, boundary_inputs_validated] the reason recorded at `Task::url` in work.rs, at a new site: every entity of this contract carries its web address as `Option<String>`, parsing one would add a URL dependency to the crate AGENTS.md says to keep still, and a comment's address narrowed here alone would describe one thing in two types.
    pub url: Option<String>,
}

/// What a comment says: anything at all, except nothing.
///
/// A newtype rather than a `String` checked by whoever happens to read it, because an empty
/// comment is refused by every source this product drives and by the command line before
/// any of them: a value of this type is one a source can be handed without asking again.
/// Everything else about the text is kept exactly — no trimming, no newline normalisation —
/// because a body quoting a command or a stack trace is only useful byte for byte.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "String", into = "String")]
pub struct CommentBody(String);

impl CommentBody {
    /// Wrap `text` as a comment body.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::Refused`] when `text` is empty, saying so.
    pub fn new(text: impl Into<String>) -> Result<Self, SourceError> {
        let text = text.into();
        if text.is_empty() {
            return Err(SourceError::Refused {
                message: "a comment body cannot be empty; write what the comment says"
                    .to_owned(),
            });
        }
        Ok(Self(text))
    }

    /// The text, exactly as it was given.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for CommentBody {
    type Error = String;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        Self::new(text).map_err(|error| error.to_string())
    }
}

impl From<CommentBody> for String {
    fn from(body: CommentBody) -> Self {
        body.0
    }
}

/// One comment to add to a task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NewComment {
    /// What it says.
    pub body: CommentBody,
    /// Who wrote it, when the caller says.
    ///
    /// A source that records the author itself — a hosted service that knows which account
    /// is signed in — refuses a comment carrying one rather than dropping it, naming why:
    /// quietly posting under another name than the one asked for is the one wrong answer
    /// here.
    // llmlint: ignore[invalid_states_unrepresentable] an author is the source's own spelling of a person — a login, a display name, a free-form name in a Markdown file — so there is no narrower type every source could agree on; each source refuses what it cannot record, naming it, which is the rule every write of this contract follows.
    #[serde(default)]
    pub author: Option<String>,
}

/// The refusal a source with no comments answers a comment call with.
///
/// Spelled once beside [`unwritable`](crate::unwritable) and
/// [`documentless`](crate::documentless), for the reason they are: a source saying it does not
/// have that side of the contract at all refuses in the same words whichever plugin it is,
/// and cannot describe something different from the engine's own message about it.
#[must_use]
pub fn commentless(kind: &str) -> SourceError {
    SourceError::Refused {
        message: format!("the {kind} plugin has no comments"),
    }
}
