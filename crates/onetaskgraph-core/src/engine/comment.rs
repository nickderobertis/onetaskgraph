//! Comments on a task: the four verbs that read and write them, and the task detail
//! `task show` renders them in.
//!
//! Every verb here addresses exactly one task of exactly one source, so none of them fans
//! out, pages for a caller or compensates for anything. What they owe instead is the
//! refusals: a source declaring no comments is refused before anything is read, a source
//! whose comments cannot be written is refused before a write is attempted, and a task or a
//! comment that is not there is named rather than answered with an empty result.
//!
//! Nothing here writes anything down. A list walks the source's pages to the end and hands
//! the caller exactly what it read; a copy never reaches this module at all, which is what
//! keeps a copy from reading or writing a comment at either end.

use onetaskgraph_plugin_api::{
    Comment, CommentBody, Cursor, NativeId, NewComment, PageRequest, SourceError, SourceName, Task,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::fetch::{fits, unrepeated};
use super::{ConfiguredSource, Engine, EngineError, Qualified};
use crate::GlobalId;
use crate::plan::{QueryResponse, SourceFailure};
use crate::resolve::ResolvedSource;

/// Every comment on one task, oldest first: what `task comment list` answers with.
///
/// An object rather than a bare list, so a later member — a total, say — is an addition a
/// reader already written against this shape can ignore.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CommentList {
    /// The task's comments in the order they were written. Empty when it has none.
    pub comments: Vec<Comment>,
}

/// What `task comment delete` answers with: the id of the comment it removed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DeletedComment {
    /// The id the comment was removed under, exactly as `list` reported it.
    pub deleted: NativeId,
}

/// One task as `task show` reports it: the response every show verb answers with, and the
/// task's comments beside it.
///
/// The response is flattened rather than nested, so a reader of `task show --json` written
/// before comments existed reads exactly the members it read before.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TaskDetail {
    /// The task, its plan and any failure, exactly as [`Engine::task`] answers.
    #[serde(flatten)]
    pub response: QueryResponse<Qualified<Task>>,
    /// The task's comments, oldest first, for a source whose tasks have comments.
    ///
    /// **Absent** rather than empty for a source declaring none, and for a task that was not
    /// found or whose comments could not be read — the last of those with the failure in the
    /// response's `errors`. An empty list says the source has comments and this task holds
    /// none, which is a different thing to tell a reader.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comments: Option<Vec<Comment>>,
}

impl Engine {
    /// One task by its qualified id, with its comments when its source has them.
    ///
    /// The task is read exactly as [`task`](Self::task) reads it, and the comments are read
    /// only once the task was found — a source declaring no comments is never asked.
    ///
    /// # Errors
    ///
    /// As [`task`](Self::task). A comment read that fails is not an error: it lands in the
    /// response's `errors` beside the task that was read.
    pub async fn task_detail(&self, id: &GlobalId) -> Result<TaskDetail, EngineError> {
        let mut response = self.task(id).await?;
        let source = match self.configured(&id.source) {
            Some(ConfiguredSource::Ready(source))
                if !response.items.is_empty()
                    && source.source().capabilities().comments.is_native() =>
            {
                source
            }
            _ => {
                return Ok(TaskDetail {
                    response,
                    comments: None,
                });
            }
        };
        let comments = match walk(source, &id.native).await {
            Ok(comments) => comments,
            Err(error) => {
                response.errors.push(SourceFailure {
                    source: source.name().clone(),
                    error,
                });
                None
            }
        };
        Ok(TaskDetail { response, comments })
    }

    /// Every comment on one task, oldest first.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::NoComments`] for a source whose tasks have none, before
    /// anything is read; [`EngineError::NoSuchTask`] when the source holds no such task; and
    /// [`EngineError::SourceFailed`] when the source could not answer.
    pub async fn comments(&self, task: &GlobalId) -> Result<CommentList, EngineError> {
        let source = self.commented(&task.source)?;
        match walk(source, &task.native).await {
            Ok(Some(comments)) => Ok(CommentList { comments }),
            Ok(None) => Err(no_such_task(task)),
            Err(error) => Err(failed(source, error)),
        }
    }

    /// Add one comment to a task, answering with the comment as its source now holds it.
    ///
    /// # Errors
    ///
    /// As [`comments`](Self::comments), plus [`EngineError::CommentsNotWritable`] for a
    /// source whose comments cannot be written, before anything is written.
    pub async fn add_comment(
        &self,
        task: &GlobalId,
        comment: &NewComment,
    ) -> Result<Comment, EngineError> {
        let source = self.writable_comments(&task.source)?;
        match source.source().add_comment(&task.native, comment).await {
            Ok(Some(added)) => Ok(added),
            Ok(None) => Err(no_such_task(task)),
            Err(error) => Err(failed(source, error)),
        }
    }

    /// Replace one comment's body, answering with the comment as its source now holds it.
    ///
    /// # Errors
    ///
    /// As [`add_comment`](Self::add_comment), plus [`EngineError::NoSuchComment`] when the
    /// task has no comment under `comment`.
    pub async fn edit_comment(
        &self,
        task: &GlobalId,
        comment: &NativeId,
        body: &CommentBody,
    ) -> Result<Comment, EngineError> {
        let source = self.writable_comments(&task.source)?;
        match source
            .source()
            .edit_comment(&task.native, comment, body)
            .await
        {
            Ok(Some(edited)) => Ok(edited),
            Ok(None) => Err(missing(source, task, comment).await),
            Err(error) => Err(failed(source, error)),
        }
    }

    /// Remove one comment from a task, answering with the id it removed.
    ///
    /// # Errors
    ///
    /// As [`edit_comment`](Self::edit_comment).
    pub async fn delete_comment(
        &self,
        task: &GlobalId,
        comment: &NativeId,
    ) -> Result<DeletedComment, EngineError> {
        let source = self.writable_comments(&task.source)?;
        match source.source().delete_comment(&task.native, comment).await {
            Ok(Some(deleted)) => Ok(DeletedComment { deleted }),
            Ok(None) => Err(missing(source, task, comment).await),
            Err(error) => Err(failed(source, error)),
        }
    }

    /// The configured source called `name`, in whichever state it is in.
    fn configured(&self, name: &SourceName) -> Option<&ConfiguredSource> {
        self.sources.iter().find(|source| source.name() == name)
    }

    /// The built source called `name`, when its tasks have comments.
    fn commented(&self, name: &SourceName) -> Result<&ResolvedSource, EngineError> {
        let name = self.known(name)?;
        match self.configured(&name) {
            Some(ConfiguredSource::Ready(source)) => {
                if source.source().capabilities().comments.is_native() {
                    Ok(source)
                } else {
                    Err(EngineError::NoComments {
                        name: name.to_string(),
                        kind: source.kind().to_owned(),
                    })
                }
            }
            Some(ConfiguredSource::Unavailable(source)) => Err(EngineError::SourceUnavailable {
                name: name.to_string(),
                error: source.error().clone(),
            }),
            // `known` has just said a source by this name is configured, and `configured`
            // reads the same list it read.
            None => Err(EngineError::NoSources),
        }
    }

    /// The built source called `name`, when its tasks have comments it can write.
    fn writable_comments(&self, name: &SourceName) -> Result<&ResolvedSource, EngineError> {
        let source = self.commented(name)?;
        if source.source().writes().is_supported() {
            return Ok(source);
        }
        Err(EngineError::CommentsNotWritable {
            name: source.name().to_string(),
            kind: source.kind().to_owned(),
        })
    }
}

/// Every comment on `task`, walked to the end of the source's pages, or `None` when the
/// source holds no such task.
///
/// Each page is asked at the source's own ceiling, and each is held to the two refusals every
/// pagination loop of this engine owes: a page longer than the one asked for, and a cursor
/// handed back unchanged. What the walk accumulates is the caller's answer and nothing else.
async fn walk(source: &ResolvedSource, task: &NativeId) -> Result<Option<Vec<Comment>>, SourceError> {
    let limit = source.source().capabilities().max_page_size.max(1);
    let mut comments = Vec::new();
    let mut cursor: Option<Cursor> = None;
    loop {
        let request = PageRequest {
            cursor: cursor.clone(),
            limit,
        };
        // A task that is gone part way through a walk is gone: reporting the comments read
        // before it went would describe a task nobody can address any more.
        let Some(page) = source.source().task_comments(task, &request).await? else {
            return Ok(None);
        };
        fits(page.items.len(), limit)?;
        unrepeated(
            page.next.as_ref(),
            cursor.as_ref(),
            "walking a task's comments",
        )?;
        comments.extend(page.items);
        match page.next {
            Some(next) => cursor = Some(next),
            None => return Ok(Some(comments)),
        }
    }
}

/// Which of the two things an edit or a delete named was not there.
///
/// Asked only once the source has already said one of them is missing, so a comment verb
/// that succeeds costs its source exactly one call.
async fn missing(source: &ResolvedSource, task: &GlobalId, comment: &NativeId) -> EngineError {
    match source.source().get_task(&task.native).await {
        Ok(Some(_)) => EngineError::NoSuchComment {
            task: task.to_string(),
            comment: comment.to_string(),
        },
        Ok(None) => no_such_task(task),
        Err(error) => failed(source, error),
    }
}

fn no_such_task(task: &GlobalId) -> EngineError {
    EngineError::NoSuchTask {
        id: task.to_string(),
    }
}

fn failed(source: &ResolvedSource, error: SourceError) -> EngineError {
    EngineError::SourceFailed {
        name: source.name().to_string(),
        error,
    }
}
