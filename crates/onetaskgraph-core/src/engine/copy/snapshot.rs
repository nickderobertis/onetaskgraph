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
//! When they disagree the task is read again from its first page, once. A second
//! disagreement is not chased further: the task is left unwritten and the copy reports it as
//! failed, because a read that keeps losing a race with a person editing the item would
//! otherwise spend an allowance every session on the host shares, without bound.

use onetaskgraph_plugin_api::{
    Comment, Cursor, NativeId, PageRequest, SourceError, Task, TaskDetailRead,
};

use crate::resolve::ResolvedSource;

use super::super::fetch::{fits, unrepeated};

/// One task as a copy reads it: the task, and every one of its comments when they were asked
/// for — `None` when they were not.
pub(super) struct Snapshot {
    /// The task, as the first request of the read reported it.
    pub(super) task: Task,
    /// Its comments, oldest first, every page of them.
    pub(super) comments: Option<Vec<Comment>>,
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
    source
        .source()
        .get_task_details(std::slice::from_ref(id), Some(&comment_page(source)))
        .await
        .pop()
        .unwrap_or(Ok(None))
}

/// Finish reading `id` from the detail read `read` answered with: every further page of its
/// comments, each held to the revision the first request carried.
///
/// # Errors
///
/// Whatever the source answered a page with, and the refusals every pagination loop of this
/// engine owes.
pub(super) async fn take(
    source: &ResolvedSource,
    id: &NativeId,
    read: TaskDetailRead,
) -> Result<Taken, SourceError> {
    let TaskDetailRead { task, comments } = read;
    let page = match comments {
        None => {
            return Ok(Taken::Whole(Box::new(Snapshot {
                task,
                comments: None,
            })));
        }
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
        let answer = source
            .source()
            .get_task_details(std::slice::from_ref(id), Some(&request))
            .await
            .pop()
            .unwrap_or(Ok(None))?;
        let Some(later) = answer else {
            return Ok(Taken::Gone);
        };
        if !one_revision(&task, &later.task) {
            return Ok(Taken::Changed);
        }
        let page = match later.comments {
            Some(Ok(Some(page))) => page,
            Some(Ok(None)) => return Ok(Taken::Gone),
            Some(Err(error)) => return Err(error),
            None => {
                return Err(SourceError::Malformed {
                    message: format!(
                        "the source answered a read of task {id}'s comments without them"
                    ),
                });
            }
        };
        fits(page.items.len(), limit)?;
        held.extend(page.items);
        asked = Some(cursor);
        next = page.next;
    }
    Ok(Taken::Whole(Box::new(Snapshot {
        task,
        comments: Some(held),
    })))
}

/// Whether two reads of one task carried the same revision of it: the same change token when
/// both gave one, and otherwise the same task in every member but the token.
fn one_revision(first: &Task, later: &Task) -> bool {
    if let (Some(first), Some(later)) = (&first.version, &later.version) {
        return first == later;
    }
    let untokened = |task: &Task| Task {
        version: None,
        ..task.clone()
    };
    untokened(first) == untokened(later)
}
