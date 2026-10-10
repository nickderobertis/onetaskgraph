//! A task copy that names its items by a query rather than by id: `task copy --to` with the
//! narrowing `task list` takes.
//!
//! The query's own answer is what makes a repeat copy cheap. Every task it lists comes with
//! the change token its source gave, so a task whose token is the one its counterpart records
//! — and whose counterpart reads as the copy that left it — is reported `unchanged` with
//! `skipped: true` from the listing and the destination read alone, and nothing more of it is
//! read from its source. Every other task is read whole, the way a copy by id reads it.
//!
//! The query's comment activity is never handed to a source. A source that applies it reads
//! each candidate's comments to confirm it, and the copy would then read them again to carry
//! them; instead the copy confirms it from what it reads anyway — the comments it reads to
//! carry, or, for a task it skips, the copies of them the destination already holds. So no
//! task's comments are read from its source more than once in one invocation.

use chrono::{DateTime, Utc};
use onetaskgraph_plugin_api::{MetadataKey, Task};

use crate::{Failure, GlobalId};

use super::super::{Engine, EngineError, Qualified, TaskRequest};
use super::{
    CopyAction, CopyOutcome, CopyRequest, Item, Level, Linking, Running, Target, skipped_counts,
    skips,
};

/// One item of a copy, in the order the copy reports it: one already decided, or one still to
/// be read and landed.
pub(super) enum Slot {
    /// Decided without reading it from its source.
    Done(CopyOutcome),
    /// To be read and landed.
    Read(GlobalId),
}

/// The outcome of an item the copy left unwritten because of `error`.
pub(super) fn failed(source: GlobalId, error: &EngineError) -> CopyOutcome {
    CopyOutcome {
        source,
        action: CopyAction::Failed {
            error: Failure::from(error),
        },
        placed: None,
        comments: None,
    }
}

impl Engine {
    /// Every task `query` matches, each either skipped from its listing or left to be read.
    pub(super) async fn matching(
        &self,
        request: &CopyRequest,
        query: &TaskRequest,
        running: &mut Running,
    ) -> Result<Vec<Slot>, EngineError> {
        let mut slots = Vec::new();
        for listed in self.listed(query).await? {
            if let Some(outcome) = self
                .in_hand(request, &listed, query.commented_since, running)
                .await?
            {
                slots.push(Slot::Done(outcome));
            } else {
                slots.push(Slot::Read(listed.id));
            }
        }
        Ok(slots)
    }

    /// Every task the query lists, through every page, each once — its comment activity left
    /// for the copy to confirm.
    async fn listed(&self, query: &TaskRequest) -> Result<Vec<Qualified<Task>>, EngineError> {
        let mut asked = query.clone();
        asked.commented_since = None;
        asked.paging.token = None;
        let mut listed: Vec<Qualified<Task>> = Vec::new();
        loop {
            let page = self.tasks(&asked).await?;
            // A copy is one write: a query one of whose sources failed is not an answer to copy.
            if let Some(failure) = page.errors.into_iter().next() {
                return Err(EngineError::SourceFailed {
                    name: failure.source.to_string(),
                    error: failure.error,
                });
            }
            for task in page.items {
                if !listed.iter().any(|held| held.id == task.id) {
                    listed.push(task);
                }
            }
            match page.next {
                Some(next) => asked.paging.token = Some(next),
                None => return Ok(listed),
            }
        }
    }

    /// The outcome of a listed task the copy skips without reading it from its source, or
    /// `None` when it has to be read.
    ///
    /// Only a task whose source gave a token, copied into a destination without routes, is
    /// asked about: where a routed copy lands a task is decided from the whole task. The
    /// destination is read here exactly as the copy reads it to find the counterpart, and a
    /// task that has to be read after all takes what was found with it, so the destination is
    /// not read for it twice.
    async fn in_hand(
        &self,
        request: &CopyRequest,
        listed: &Qualified<Task>,
        since: Option<DateTime<Utc>>,
        running: &mut Running,
    ) -> Result<Option<CopyOutcome>, EngineError> {
        if listed.item.version.is_none() || self.routes.routes(&request.destination) {
            return Ok(None);
        }
        let destination = self.writable(&request.destination)?;
        let withheld = self.declared(&listed.id.source) == crate::config::SourceVisibility::Private
            && self.declared(destination.name()) != crate::config::SourceVisibility::Private;
        let item = Item::Task(Box::new(listed.item.clone()));
        let (target, held) = self
            .target(destination, request, &listed.id, &item, withheld)
            .await?;
        let skip = match (&target, &held) {
            (Target::Update { .. }, Some(held)) => {
                skips(&listed.item, held, request.comments, destination.name())
                    && since.is_none_or(|since| {
                        // Confirmed from the destination's copies of the comments written at the
                        // source, which carry their origin's own times; a task whose activity
                        // they cannot confirm is read and confirmed from its source instead.
                        held.comments.iter().flatten().any(|comment| {
                            comment.origin.as_ref().is_some_and(|origin| {
                                origin.task == listed.id.to_string()
                                    && (origin.created_at.is_some_and(|at| at >= since)
                                        || origin.updated_at.is_some_and(|at| at >= since))
                            })
                        })
                    })
            }
            _ => false,
        };
        let Target::Update { id, found } = &target else {
            running.aimed.insert(listed.id.to_string(), (target, held));
            return Ok(None);
        };
        if !skip {
            running
                .aimed
                .insert(listed.id.to_string(), (target.clone(), held));
            return Ok(None);
        }
        let landed = GlobalId::new(destination.name().clone(), id.clone());
        running
            .counterparts
            .insert(listed.id.to_string(), landed.clone());
        if !request.dry_run {
            running.linking.push(Linking {
                item: listed.id.clone(),
                level: Level::Task,
                found: Some(*found),
                destination: landed.clone(),
                held: listed.item.metadata.get(MetadataKey::COPIES_KEY).cloned(),
            });
        }
        let counts = request.comments.then(|| skipped_counts(held.as_ref()));
        Ok(Some(CopyOutcome {
            source: listed.id.clone(),
            action: CopyAction::Unchanged {
                destination: landed,
                via: found.via(),
                link: None,
                skipped: true,
            },
            placed: None,
            comments: counts,
        }))
    }
}
