//! Comments on a task, and the one refusal a source with none answers with.
//!
//! A comment is the one part of a task a person adds to in pieces, after the task exists
//! and without rewriting it: evidence appended to an issue somebody else opened. So it is
//! not a field of [`Task`](crate::Task) — a copy writes a task whole, and a copy without
//! `--comments` adds, changes and removes no comment — but a thing of its own, read and
//! written through its own methods of [`TaskSource`](crate::TaskSource).
//!
//! A copy with `--comments` keeps a destination's copies of a source's comments in step,
//! and each copy it writes names the comment it was copied from as its [`CommentOrigin`]. A
//! source that can store that natively overrides
//! [`TaskSource::write_comment_copy`](crate::TaskSource::write_comment_copy); every other one
//! is handed it as a hidden marker at the end of the body, [`CommentOrigin::mark`], which the
//! engine strips back out of every comment it reads, [`Comment::unmarked`].

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{NativeId, Page, SourceError, Task};

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
    // llmlint: ignore[invalid_states_unrepresentable] `CommentBody` is the invariant on what this product *writes*, and this is what a source already *holds*: a Linear comment that is only an attachment, or one a person cleared on the service's own page, has an empty body, and refusing it here would fail the whole of a task's comment list over one comment nobody here wrote. The read reports what the source holds; the write is what refuses nothing.
    pub body: String,
    /// Where a person can open it.
    // llmlint: ignore[invalid_states_unrepresentable, boundary_inputs_validated] the reason recorded at `Task::url` in work.rs, at a new site: every entity of this contract carries its web address as `Option<String>`, parsing one would add a URL dependency to the crate AGENTS.md says to keep still, and a comment's address narrowed here alone would describe one thing in two types.
    pub url: Option<String>,
    /// The comment this one is a copy of, at its origin — present only on a comment a copy
    /// wrote, and absent on every comment a person wrote where it is.
    ///
    /// On the second hop of a chained copy it still names the *first* origin, never the
    /// source the copy read it from. Left out of the wire when absent, so a reader written
    /// before it existed reads exactly what it read before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<CommentOrigin>,
}

/// Where a copied comment came from: the comment as its origin holds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CommentOrigin {
    /// The task the comment is on at its origin, qualified: `<source>:<native task id>`.
    // llmlint: ignore[invalid_states_unrepresentable] A qualified id is `onetaskgraph-core`'s `GlobalId`, which this crate cannot name (a plugin never sees one — AGENTS.md, "The plugin contract"); `TaskRef` admits a bare id, which this must never be. The engine is the one writer, and it writes a qualified id's own rendering.
    pub task: String,
    /// The comment's own id at its origin.
    pub id: NativeId,
    /// Who wrote it there, in that source's spelling of a person.
    pub author: Option<String>,
    /// When the origin says it was written.
    pub created_at: Option<DateTime<Utc>>,
    /// When the origin says it last changed.
    pub updated_at: Option<DateTime<Utc>>,
    /// Where a person can open it at its origin.
    // llmlint: ignore[invalid_states_unrepresentable, boundary_inputs_validated] the reason recorded at `Comment::url` above, at a new site.
    pub url: Option<String>,
}

/// The opening of the marker [`CommentOrigin::mark`] appends to a body.
const MARKER_OPEN: &str = "<!-- onetaskgraph.comment_origin `";
/// The close of that marker.
const MARKER_CLOSE: &str = "` -->";
/// What separates the marker from the body it follows.
const MARKER_SEPARATOR: &str = "\n\n";

impl CommentOrigin {
    /// Whether `task` is a qualified task id — `<source>:<native task id>`, the source a valid
    /// source name and the native id not empty — and so can be an origin's [`task`](Self::task).
    ///
    /// What a reader holds to before it takes a stored origin as one: the origin's task decides
    /// which copies a sync owns, edits and deletes, and which comments it never sends back, so
    /// one naming no task is no origin at all and is read as the text it was stored as.
    #[must_use]
    pub fn is_qualified_task(task: &str) -> bool {
        task.split_once(':').is_some_and(|(source, native)| {
            !native.is_empty() && crate::SourceName::new(source).is_ok()
        })
    }

    /// This origin, refused as [`SourceError::Malformed`] when its task is not a qualified task
    /// id — what every writer of a copied comment holds an origin to before it writes one, since
    /// such an origin would be stored and then read back as none.
    ///
    /// # Errors
    ///
    /// [`SourceError::Malformed`], naming the task it was given.
    pub fn qualified(&self) -> Result<&Self, SourceError> {
        if Self::is_qualified_task(&self.task) {
            Ok(self)
        } else {
            Err(SourceError::Malformed {
                message: format!(
                    "a copied comment's origin names its task as {:?}, which is not a \
                     qualified task id `<source>:<native task id>`",
                    self.task
                ),
            })
        }
    }

    /// `body` with this origin appended as a hidden marker, for a source that cannot store a
    /// comment's origin, author or times natively.
    ///
    /// One line, an HTML comment whose payload is compact JSON inside a code span —
    /// ``<!-- onetaskgraph.comment_origin `{…}` -->`` — because that is the spelling both
    /// hosted services this product drives keep byte for byte (the Linear plugin's module
    /// documentation records what Linear does to every other one). `<`, `>` and `` ` `` are
    /// escaped inside the JSON, where the escape means the same character, so no value can
    /// close the marker early.
    #[must_use]
    pub fn mark(&self, body: &str) -> String {
        let encoded = serde_json::to_string(self)
            .expect("a comment origin serialises")
            .replace('<', "\\u003c")
            .replace('>', "\\u003e")
            .replace('`', "\\u0060");
        format!("{body}{MARKER_SEPARATOR}{MARKER_OPEN}{encoded}{MARKER_CLOSE}")
    }
}

impl CommentOrigin {
    /// `body`, carrying over the hidden marker the comment it replaces ends in — `held`, that
    /// comment's body exactly as its source holds it — when `body` ends in none of its own.
    ///
    /// What a source that keeps a copied comment's origin as the marker answers an edit with:
    /// an edit changes what a comment says and nothing else, so a comment a copy wrote is still
    /// that copy afterwards, and the next copy keeps it in step rather than writing another
    /// beside it. A `body` that ends in a marker of its own — a copy rewriting its own comment —
    /// is left as it is.
    ///
    /// # Errors
    ///
    /// None in practice, because the body only grows: [`SourceError::Refused`] is what
    /// [`CommentBody::new`] answers an empty text with.
    pub fn kept(held: &str, body: &CommentBody) -> Result<CommentBody, SourceError> {
        let origin_of = |text: &str| {
            Comment {
                id: NativeId(String::new()),
                author: None,
                created_at: None,
                updated_at: None,
                body: text.to_owned(),
                url: None,
                origin: None,
            }
            .unmarked()
            .origin
        };
        match (origin_of(held), origin_of(body.as_str())) {
            (Some(origin), None) => CommentBody::new(origin.mark(body.as_str())),
            _ => Ok(body.clone()),
        }
    }
}

impl Comment {
    /// This comment with a marker [`CommentOrigin::mark`] wrote taken out of its body and
    /// reported as its [`origin`](Self::origin); unchanged when it carries an origin already
    /// or its body ends in no such marker.
    ///
    /// A marker that does not parse is left in the body as text: it is not one this product
    /// wrote, and reading it as an origin would put words in a person's comment. An origin a
    /// source reports natively — a stdio peer's, say — is held to the same test: one naming no
    /// qualified task is dropped, so the comment reads as one written where it is, which a copy
    /// never edits or deletes.
    #[must_use]
    pub fn unmarked(mut self) -> Self {
        if let Some(origin) = &self.origin {
            if !CommentOrigin::is_qualified_task(&origin.task) {
                self.origin = None;
            }
            return self;
        }
        let trimmed = self.body.trim_end();
        let Some(open) = trimmed.rfind(MARKER_OPEN) else {
            return self;
        };
        let Some(encoded) = trimmed[open + MARKER_OPEN.len()..].strip_suffix(MARKER_CLOSE) else {
            return self;
        };
        let Ok(origin) = serde_json::from_str::<CommentOrigin>(encoded) else {
            return self;
        };
        if !CommentOrigin::is_qualified_task(&origin.task) {
            return self;
        }
        let before = &self.body[..open];
        let body = before.strip_suffix(MARKER_SEPARATOR).unwrap_or(before);
        self.body = body.to_owned();
        self.origin = Some(origin);
        self
    }
}

/// One comment a copy writes: what it says, and the comment it is a copy of.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CommentCopy {
    /// What it says, as its origin says it.
    pub body: CommentBody,
    /// The comment it is a copy of.
    pub origin: CommentOrigin,
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
                message: "a comment body cannot be empty; write what the comment says".to_owned(),
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

/// One task as [`TaskSource::get_task_details`](crate::TaskSource::get_task_details) reads it:
/// the task, and the first page of its comments when they were asked for.
///
/// The comments are a result of their own rather than part of the task's, because the two
/// can fail apart: a task that was read and whose comments could not be — a board draft,
/// which has none — is still a task a caller is owed, with the comment failure beside it.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskDetailRead {
    /// The task, exactly as [`TaskSource::get_task`](crate::TaskSource::get_task) answers it.
    pub task: Task,
    /// The first page of its comments, oldest first: `None` when the read did not ask for
    /// them, and otherwise exactly what
    /// [`TaskSource::task_comments`](crate::TaskSource::task_comments) answers for that page
    /// — `Ok(None)` included, for a task gone between the two halves of a read that made two.
    pub comments: Option<Result<Option<Page<Comment>>, SourceError>>,
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
