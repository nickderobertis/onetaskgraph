//! Reading one task and its comments as one revision of it.
//!
//! A `--comments` copy writes a task and its comments together, so what it writes has to be
//! one state the source really held: never a body from before an edit beside comments from
//! after it. A task whose comments fit the first page is read whole in one request — the
//! detail read that names it — and is one revision by construction. One whose comments run
//! longer is read a page at a time, each further page through the same detail read, so every
//! request carries the task beside the page it answers. The pages are one revision when every
//! request carried the same task: the same change token where the source gives one, and
//! otherwise the same task in every member it reports.
//!
//! **One bound, stated rather than discovered.** Where the source gives no token — Linear, and
//! a GitHub task changed within the minute before the read — the task's own members are all
//! two reads can be compared by, and a comment edited alone between two pages moves none of
//! them. Such a task's comments are then one revision only as far as each page shows.
//!
//! When they disagree the task is read again from its first page, once. A second
//! disagreement is not chased further: the task is left unwritten and the copy reports it as
//! failed, because a read that keeps losing a race with a person editing the item would
//! otherwise spend an allowance every session on the host shares, without bound.

use onetaskgraph_plugin_api::{
    Comment, Cursor, NativeId, PageRequest, SourceError, Task, TaskDetailRead,
};

use crate::resolve::ResolvedSource;

use super::super::fetch::{fits, unrepeated};

/// One task as a copy reads it: the task, and every one of its comments.
pub(super) struct Snapshot {
    /// The task, as the first request of the read reported it.
    pub(super) task: Task,
    /// Its comments, oldest first, every page of them.
    pub(super) comments: Vec<Comment>,
}

/// What one attempt at reading a task whole came to.
pub(super) enum Taken {
    /// Every request agreed: one revision of the task.
    Whole(Box<Snapshot>),
    /// The source no longer holds the task.
    Gone,
    /// A later request carried another revision of the task than the first.
    Changed,
}

/// The page size a copy reads comments in: the most the source declares it answers.
pub(super) fn comment_page(source: &ResolvedSource) -> PageRequest {
    PageRequest {
        cursor: None,
        limit: source.source().capabilities().max_page_size.max(1),
    }
}

/// Read the first page of `id`'s comments beside the task, in one detail read.
///
/// # Errors
///
/// Whatever the source answered the read with.
pub(super) async fn first(
    source: &ResolvedSource,
    id: &NativeId,
) -> Result<Option<TaskDetailRead>, SourceError> {
    let answers = source
        .source()
        .get_task_details_for_copy(std::slice::from_ref(id), &comment_page(source))
        .await;
    single(id, answers)
}

/// The one answer a detail read of `id` alone owes: refused as malformed when the source gave
/// any other number of them, or answered about another task — either would put some other
/// read's content under `id`.
fn single(
    id: &NativeId,
    mut answers: Vec<Result<Option<TaskDetailRead>, SourceError>>,
) -> Result<Option<TaskDetailRead>, SourceError> {
    if answers.len() != 1 {
        return Err(SourceError::Malformed {
            message: format!(
                "the source answered {} details for one read of task {id}",
                answers.len()
            ),
        });
    }
    let answer = answers.pop().expect("exactly one answer")?;
    about(id, answer.as_ref())?;
    Ok(answer)
}

/// Refuse a detail answered about another task than `id`.
fn about(id: &NativeId, read: Option<&TaskDetailRead>) -> Result<(), SourceError> {
    match read {
        Some(read) if read.task.id != *id => Err(SourceError::Malformed {
            message: format!(
                "the source answered a read of task {id} with task {}",
                read.task.id
            ),
        }),
        _ => Ok(()),
    }
}

/// Finish reading `id` from the detail read `read` answered with: every further page of its
/// comments, each held to the revision the first request carried.
///
/// # Errors
///
/// Whatever the source answered a page with, the refusals every pagination loop of this engine
/// owes, and [`SourceError::Malformed`] for an answer without the comments it was asked for —
/// on the first page as on any later one. Read as none, it would have a copy delete every
/// copy the destination holds of them.
pub(super) async fn take(
    source: &ResolvedSource,
    id: &NativeId,
    read: TaskDetailRead,
) -> Result<Taken, SourceError> {
    // A batched read answered ahead of this one is held to its id here, as `first` holds its own.
    about(id, Some(&read))?;
    let TaskDetailRead { task, comments } = read;
    let page = match comments {
        None => return Err(commentless(id)),
        Some(Err(error)) => return Err(error),
        Some(Ok(None)) => return Ok(Taken::Gone),
        Some(Ok(Some(page))) => page,
    };
    let limit = comment_page(source).limit;
    fits(page.items.len(), limit)?;
    let mut held = page.items;
    let mut next = page.next;
    let mut asked: Option<Cursor> = None;
    while let Some(cursor) = next {
        unrepeated(Some(&cursor), asked.as_ref(), "reading a task's comments")?;
        let request = PageRequest {
            cursor: Some(cursor.clone()),
            limit,
        };
        let answer = single(
            id,
            source
                .source()
                .get_task_details(std::slice::from_ref(id), Some(&request))
                .await,
        )?;
        let Some(later) = answer else {
            return Ok(Taken::Gone);
        };
        if !reads_alike(&task, &later.task) {
            return Ok(Taken::Changed);
        }
        let page = match later.comments {
            Some(Ok(Some(page))) => page,
            Some(Ok(None)) => return Ok(Taken::Gone),
            Some(Err(error)) => return Err(error),
            None => return Err(commentless(id)),
        };
        fits(page.items.len(), limit)?;
        held.extend(page.items);
        asked = Some(cursor);
        next = page.next;
    }
    // Oldest first, whichever way the source walks its pages: Linear's run from the newest
    // back, each page oldest first within itself. A stable sort leaves a source that walks
    // forward, and comments sharing an instant, in the order the source gave them.
    held.sort_by_key(|comment| comment.created_at);
    Ok(Taken::Whole(Box::new(Snapshot {
        task,
        comments: held,
    })))
}

/// What a detail read answered without the comments it was asked for is refused as.
fn commentless(id: &NativeId) -> SourceError {
    SourceError::Malformed {
        message: format!("the source answered a read of task {id}'s comments without them"),
    }
}

/// Whether two reads of one task agree on it: the same change token when both gave one — which
/// covers its comments — and otherwise the same task in every member but the token, which says
/// nothing of a change to a comment alone.
fn reads_alike(first: &Task, later: &Task) -> bool {
    if let (Some(first), Some(later)) = (&first.version, &later.version) {
        return first == later;
    }
    let untokened = |task: &Task| Task {
        version: None,
        ..task.clone()
    };
    untokened(first) == untokened(later)
}
