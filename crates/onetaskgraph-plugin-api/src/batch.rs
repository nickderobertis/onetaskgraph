//! Batched writes: many items' writes in one call, each held and answered on its own.
//!
//! A caller doing the same thing to many items — a comment on each, an update of each — asks a
//! source once rather than once per item, and a source whose backend can carry several writes
//! in one request sends them together. Every item is still its own write: one item's failure
//! never writes or refuses another's, and each answer says what became of its own item.
//!
//! The defaults on [`TaskSource`] make one call per item through the single-item methods, so a
//! source that predates these behaves exactly as it did, one request per item; one whose
//! backend batches overrides them, and keeps a per-item request for exactly the case its
//! backend would fail as a whole.

use std::cmp::Ordering;
use std::sync::Arc;

use crate::{
    Comment, HeldVersion, NativeId, NewComment, Priority, SourceError, Task, TaskSource,
    TaskUpdate, TaskUpdateOutcome,
};

/// One update of a batch: a targeted update of one existing task, held to the version the
/// caller read when `if_version` names one, and written only while the task holds one of the
/// priorities `when_priority` names when it names any.
#[derive(Debug, Clone, PartialEq)]
pub struct BatchUpdate {
    /// The task.
    pub id: NativeId,
    /// Every field written, together, in one held write.
    pub update: TaskUpdate,
    /// The version the task must be at for anything to be written.
    // llmlint: ignore[invalid_states_unrepresentable] A version is the source's own opaque token, which only the source that spelled it reads; `None` is the ordinary unheld write.
    pub if_version: Option<String>,
    /// The priorities the task must hold for anything to be written — [`Priority::None`]
    /// matching a task with none. Empty sets no guard.
    pub when_priority: Vec<Priority>,
}

/// One comment of a batch: a new comment on one existing task, held to the version the caller
/// read when `if_version` names one.
#[derive(Debug, Clone, PartialEq)]
pub struct BatchComment {
    /// The task commented on.
    pub task: NativeId,
    /// The comment.
    pub comment: NewComment,
    /// The version the task must be at for the comment to be written.
    // llmlint: ignore[invalid_states_unrepresentable] As `BatchUpdate::if_version`.
    pub if_version: Option<String>,
}

/// What became of one item of a batched write.
#[derive(Debug, Clone, PartialEq)]
pub enum BatchAnswer<T> {
    /// It was written, and this is the source's answer for it.
    Written(T),
    /// It was at another version than the one it was held to, so nothing was written; `found`
    /// is the version it is at.
    VersionMoved {
        /// The version the source found the task at.
        found: Option<String>,
    },
    /// Its priority was not one the write's guard names, so nothing was written; this is the
    /// task as the source read it.
    PriorityHeld(Box<Task>),
    /// The source holds no such task.
    Missing,
}

/// The default of [`TaskSource::update_tasks`]: each update on its own, in order.
pub(crate) async fn update_each<S: TaskSource + ?Sized>(
    source: &S,
    updates: &[BatchUpdate],
) -> Vec<Result<BatchAnswer<TaskUpdateOutcome>, SourceError>> {
    let mut answers = Vec::with_capacity(updates.len());
    for each in updates {
        answers.push(update_one(source, each).await);
    }
    answers
}

async fn update_one<S: TaskSource + ?Sized>(
    source: &S,
    each: &BatchUpdate,
) -> Result<BatchAnswer<TaskUpdateOutcome>, SourceError> {
    let held = match decide(
        source,
        &each.id,
        each.if_version.as_deref(),
        &each.when_priority,
    )
    .await?
    {
        Decided::Answer(answer) => return Ok(answer),
        Decided::Write(held) => held,
    };
    let written = match held {
        Some(version) if source.holds_versions() => {
            let hold = Arc::new(HeldVersion::new(each.id.clone(), version));
            let written = HeldVersion::scope(
                Arc::clone(&hold),
                source.update_task(&each.id, &each.update),
            )
            .await;
            if let Some(found) = hold.found() {
                return Ok(BatchAnswer::VersionMoved { found: Some(found) });
            }
            written?
        }
        _ => source.update_task(&each.id, &each.update).await?,
    };
    Ok(written.map_or(BatchAnswer::Missing, BatchAnswer::Written))
}

/// The default of [`TaskSource::add_comments`]: each comment on its own, in order.
pub(crate) async fn comment_each<S: TaskSource + ?Sized>(
    source: &S,
    comments: &[BatchComment],
) -> Vec<Result<BatchAnswer<Comment>, SourceError>> {
    let mut answers = Vec::with_capacity(comments.len());
    for each in comments {
        answers.push(comment_one(source, each).await);
    }
    answers
}

async fn comment_one<S: TaskSource + ?Sized>(
    source: &S,
    each: &BatchComment,
) -> Result<BatchAnswer<Comment>, SourceError> {
    let held = match decide(source, &each.task, each.if_version.as_deref(), &[]).await? {
        Decided::Answer(answer) => return Ok(answer),
        Decided::Write(held) => held,
    };
    let written = match held {
        Some(version) if source.holds_versions() => {
            let hold = Arc::new(HeldVersion::new(each.task.clone(), version));
            let written = HeldVersion::scope(
                Arc::clone(&hold),
                source.add_comment(&each.task, &each.comment),
            )
            .await;
            if let Some(found) = hold.found() {
                return Ok(BatchAnswer::VersionMoved { found: Some(found) });
            }
            written?
        }
        _ => source.add_comment(&each.task, &each.comment).await?,
    };
    Ok(written.map_or(BatchAnswer::Missing, BatchAnswer::Written))
}

/// What deciding one held item came to before its write: an answer that writes nothing, or the
/// write, held to the version it is to be checked at where the source checks it itself.
enum Decided<T> {
    Answer(BatchAnswer<T>),
    Write(Option<String>),
}

/// Decide one item against its hold and its guard.
///
/// A source that checks a held version inside its own write is not read for a hold alone; any
/// other is read once, immediately before the write, and so is every source for a priority
/// guard — whose write is then held to the version that read found, so a change landing
/// between the read and the write is refused rather than written over.
async fn decide<S: TaskSource + ?Sized, T>(
    source: &S,
    id: &NativeId,
    if_version: Option<&str>,
    when_priority: &[Priority],
) -> Result<Decided<T>, SourceError> {
    if if_version.is_none() && when_priority.is_empty() {
        return Ok(Decided::Write(None));
    }
    if when_priority.is_empty() && source.holds_versions() {
        return Ok(Decided::Write(if_version.map(str::to_owned)));
    }
    let Some(task) = source.get_task(id).await? else {
        return Ok(Decided::Answer(BatchAnswer::Missing));
    };
    if let Some(version) = if_version
        && !version_at(source, task.version.as_deref(), version)
    {
        return Ok(Decided::Answer(BatchAnswer::VersionMoved {
            found: task.version,
        }));
    }
    if !when_priority.is_empty() && !when_priority.contains(&task.priority) {
        return Ok(Decided::Answer(BatchAnswer::PriorityHeld(Box::new(task))));
    }
    Ok(Decided::Write(task.version))
}

/// Whether `found` is the version `version` names, in `source`'s own order of its tokens.
#[must_use]
pub fn version_at<S: TaskSource + ?Sized>(source: &S, found: Option<&str>, version: &str) -> bool {
    found.is_some_and(|found| source.order_versions(found, version) == Some(Ordering::Equal))
}

/// Decide one batched item against its hold and its guard on `task`, the source's own read of
/// it — the decision [`TaskSource::update_tasks`]' default makes, for an override that read
/// many items together: `Some` answer when nothing is to be written.
#[must_use]
pub fn decided_on<S: TaskSource + ?Sized, T>(
    source: &S,
    task: Option<&Task>,
    if_version: Option<&str>,
    when_priority: &[Priority],
) -> Option<BatchAnswer<T>> {
    let Some(task) = task else {
        return Some(BatchAnswer::Missing);
    };
    if let Some(version) = if_version
        && !version_at(source, task.version.as_deref(), version)
    {
        return Some(BatchAnswer::VersionMoved {
            found: task.version.clone(),
        });
    }
    if !when_priority.is_empty() && !when_priority.contains(&task.priority) {
        return Some(BatchAnswer::PriorityHeld(Box::new(task.clone())));
    }
    None
}
