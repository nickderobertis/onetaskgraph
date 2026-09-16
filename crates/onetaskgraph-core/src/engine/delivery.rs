//! A task's status set on its own, and the `delivers` relation the store keeps in step.
//!
//! A task names the tasks it **delivers**: finishing it finishes them. Whenever this engine
//! writes a task whose `delivers` is not empty, or was not before the write, it keeps two
//! things true of every task that list names now and every task it dropped:
//!
//! 1. **The back-reference.** A delivered task's `delivered_by` holds the deliverer's
//!    qualified id while the deliverer names it, and does not once the deliverer drops it.
//! 2. **The status.** The delivered task is re-evaluated over every deliverer it names, by
//!    [`settled`], and its status is written through the status-only write when it is at
//!    `todo`, `queued` or `in-progress` and the result differs from what it holds.
//!
//! Every write of such a task re-evaluates, a write that changes nothing about the deliverer
//! included, so a retried write re-fires the rule. Nothing here is written down outside the
//! plugins: `delivered_by` lives on the delivered task, in its own source, and every
//! evaluation reads the deliverers afresh.

use onetaskgraph_plugin_api::{SourceError, SourceName, Status, StatusCategory, Task, TaskRef};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{ConfiguredSource, Engine, EngineError, Qualified};
use crate::resolve::ResolvedSource;
use crate::{Failure, GlobalId};

/// What `task status set` answers with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TaskStatusSet {
    /// The task whose status was set.
    pub id: GlobalId,
    /// Its status as its source reads it back.
    pub status: Status,
    /// One entry per task it delivers, or dropped, that this write re-evaluated. Empty when
    /// it delivers nothing.
    pub delivered: Vec<Delivered>,
}

/// What keeping one delivered task in step with one deliverer came to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Delivered {
    /// The delivered task.
    pub ticket: GlobalId,
    /// The deliverer whose write re-evaluated it — the one that just dropped it, when it was
    /// re-evaluated for being dropped.
    pub deliverer: GlobalId,
    /// What happened to it.
    #[serde(flatten)]
    pub outcome: DeliveryOutcome,
    /// Deliverers its source read as not found, removed from its `delivered_by` on this
    /// write. Left out when there were none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(!skip_serializing_if)]
    pub pruned: Vec<GlobalId>,
}

impl Delivered {
    /// Whether the rule could not keep this task in step, which is what exits `4`.
    #[must_use]
    pub fn failed(&self) -> bool {
        matches!(self.outcome, DeliveryOutcome::Failed { .. })
    }
}

/// The four things re-evaluating a delivered task can come to, and the categories each is
/// about.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "outcome", rename_all = "kebab-case")]
pub enum DeliveryOutcome {
    /// Its status was written.
    Written {
        /// The category it read.
        from: StatusCategory,
        /// The category written.
        to: StatusCategory,
    },
    /// The rule asked for what it already holds, or for nothing at all.
    Unchanged {
        /// The category it read.
        from: StatusCategory,
    },
    /// It is at `draft`, `backlog`, `unknown`, `done` or `cancelled`, which a claim never
    /// accepts, reopens or un-defers on a person's behalf.
    Left {
        /// The category it read.
        from: StatusCategory,
    },
    /// It, or a deliverer it names, could not be read or written; the rule did not guess.
    Failed {
        /// The category it read, when it could be read.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<StatusCategory>,
        /// Why, as the failure object itself: the same `class`, `kind`, `source`, `message`
        /// and `retry_after_seconds` a failure document carries under its own `failure`
        /// member — not that whole document nested again.
        failure: Failure,
    },
}

impl Engine {
    /// Set one task's status, and nothing else about it, then keep every task it delivers in
    /// step with it.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::UnknownSource`] for a source nothing configures,
    /// [`EngineError::StatusNotWritable`] for one with no write side,
    /// [`EngineError::NoSuchTask`] when the task is not there, and
    /// [`EngineError::SourceFailed`] when the source refuses — a category it has disabled
    /// included. A delivered task that cannot be kept in step is not an error: it is reported
    /// `failed` beside the status that was set.
    pub async fn set_task_status(
        &self,
        id: &GlobalId,
        category: StatusCategory,
    ) -> Result<TaskStatusSet, EngineError> {
        let source = self.status_writable(&id.source)?;
        let no_such_task = || EngineError::NoSuchTask { id: id.to_string() };
        let task = source
            .source()
            .get_task(&id.native)
            .await
            .map_err(|error| source_failed(source, error))?
            .ok_or_else(no_such_task)?;
        let status = source
            .source()
            .set_task_status(&id.native, category)
            .await
            .map_err(|error| source_failed(source, error))?
            .ok_or_else(no_such_task)?;
        let delivers = targets(&task.delivers, &id.source);
        let delivered = self
            .deliver(id, status.category, &delivers, &delivers)
            .await;
        Ok(TaskStatusSet {
            id: id.clone(),
            status,
            delivered,
        })
    }

    /// Keep every task `deliverer` delivers `now`, and every one it delivered `before` and
    /// dropped, in step with it — one entry each, in that order.
    pub(crate) async fn deliver(
        &self,
        deliverer: &GlobalId,
        category: StatusCategory,
        now: &[GlobalId],
        before: &[GlobalId],
    ) -> Vec<Delivered> {
        let mut tickets: Vec<(&GlobalId, bool)> = Vec::new();
        for ticket in now {
            if !tickets.iter().any(|(held, _)| *held == ticket) {
                tickets.push((ticket, true));
            }
        }
        for ticket in before {
            if !now.contains(ticket) && !tickets.iter().any(|(held, _)| *held == ticket) {
                tickets.push((ticket, false));
            }
        }
        let mut delivered = Vec::with_capacity(tickets.len());
        for (ticket, kept) in tickets {
            delivered.push(self.evaluate(deliverer, category, ticket, kept).await);
        }
        delivered
    }

    /// Re-evaluate one delivered task for one deliverer, which names it when `kept` and has
    /// just dropped it otherwise.
    async fn evaluate(
        &self,
        deliverer: &GlobalId,
        category: StatusCategory,
        ticket: &GlobalId,
        kept: bool,
    ) -> Delivered {
        let entry = |outcome, pruned| Delivered {
            ticket: ticket.clone(),
            deliverer: deliverer.clone(),
            outcome,
            pruned,
        };
        let failed = |from, error: &EngineError| DeliveryOutcome::Failed {
            from,
            failure: Failure::from(error),
        };
        let no_such_task = || EngineError::NoSuchTask {
            id: ticket.to_string(),
        };
        let source = match self.built(&ticket.source) {
            Ok(source) => source,
            Err(error) => return entry(failed(None, &error), Vec::new()),
        };
        let task = match source.source().get_task(&ticket.native).await {
            Ok(Some(task)) => task,
            Ok(None) => return entry(failed(None, &no_such_task()), Vec::new()),
            Err(error) => {
                return entry(failed(None, &source_failed(source, error)), Vec::new());
            }
        };
        let from = task.status.category;
        let held = targets(&task.delivered_by, &ticket.source);
        let mut named = held.clone();
        if kept && !named.contains(deliverer) {
            named.push(deliverer.clone());
        } else if !kept {
            named.retain(|other| other != deliverer);
        }

        // Only a task the rule could write is worth reading its deliverers for: one it leaves
        // alone is left alone whatever they say.
        let active = matches!(
            from,
            StatusCategory::Todo | StatusCategory::Queued | StatusCategory::InProgress
        );
        let mut categories = Vec::new();
        let mut pruned = Vec::new();
        let mut unreadable = None;
        if active {
            for other in &named {
                if other == deliverer {
                    categories.push(category);
                    continue;
                }
                match self.category_of(other).await {
                    Ok(Some(found)) => categories.push(found),
                    Ok(None) => pruned.push(other.clone()),
                    Err(error) => {
                        unreadable = Some(error);
                        break;
                    }
                }
            }
        }
        // A rule that never finished reading prunes nothing: what it read before the failure
        // is half an answer, and the deliverer it could not read stays named.
        if unreadable.is_some() {
            pruned.clear();
        }
        let kept_by: Vec<GlobalId> = named
            .into_iter()
            .filter(|other| !pruned.contains(other))
            .collect();
        if kept_by != held {
            let list: Vec<TaskRef> = kept_by
                .iter()
                .map(|other| TaskRef::qualified(&other.source, &other.native))
                .collect();
            match source
                .source()
                .set_delivered_by(&ticket.native, &list)
                .await
            {
                Ok(Some(())) => {}
                Ok(None) => return entry(failed(Some(from), &no_such_task()), Vec::new()),
                Err(error) => {
                    return entry(
                        failed(Some(from), &source_failed(source, error)),
                        Vec::new(),
                    );
                }
            }
        }
        if let Some(error) = unreadable {
            return entry(failed(Some(from), &error), Vec::new());
        }
        if !active {
            return entry(DeliveryOutcome::Left { from }, pruned);
        }
        let Some(to) = settled(&categories).filter(|to| *to != from) else {
            return entry(DeliveryOutcome::Unchanged { from }, pruned);
        };
        match source.source().set_task_status(&ticket.native, to).await {
            Ok(Some(status)) => entry(
                DeliveryOutcome::Written {
                    from,
                    to: status.category,
                },
                pruned,
            ),
            Ok(None) => entry(failed(Some(from), &no_such_task()), pruned),
            Err(error) => entry(failed(Some(from), &source_failed(source, error)), pruned),
        }
    }

    /// A deliverer's category, or `None` when its source reads it as not found.
    async fn category_of(
        &self,
        deliverer: &GlobalId,
    ) -> Result<Option<StatusCategory>, EngineError> {
        let source = self.built(&deliverer.source)?;
        source
            .source()
            .get_task(&deliverer.native)
            .await
            .map(|task| task.map(|task| task.status.category))
            .map_err(|error| source_failed(source, error))
    }

    /// The built source called `name`.
    pub(super) fn built(&self, name: &SourceName) -> Result<&ResolvedSource, EngineError> {
        let name = self.known(name)?;
        match self.sources.iter().find(|source| source.name() == &name) {
            Some(ConfiguredSource::Ready(source)) => Ok(source),
            Some(ConfiguredSource::Unavailable(source)) => Err(EngineError::SourceUnavailable {
                name: name.to_string(),
                error: source.error().clone(),
            }),
            // `known` has just said a source by this name is configured.
            None => Err(EngineError::NoSources),
        }
    }

    /// The built source called `name`, when it can be written through.
    fn status_writable(&self, name: &SourceName) -> Result<&ResolvedSource, EngineError> {
        let source = self.built(name)?;
        if source.source().writes().is_supported() {
            return Ok(source);
        }
        Err(EngineError::StatusNotWritable {
            name: source.name().to_string(),
            kind: source.kind().to_owned(),
        })
    }
}

/// What a delivered task's status should be, over the categories of every deliverer that
/// remains — or `None` when it should not be written at all.
///
/// The first branch that matches decides:
///
/// 1. `done` when every deliverer is `done` or `cancelled` and at least one is `done`. A
///    `cancelled` deliverer releases its claim and never counts as completion; it only stops
///    blocking a sibling's.
/// 2. `in-progress` when any deliverer is `in-progress`.
/// 3. `queued` when any deliverer is `queued`.
/// 4. `todo` when any deliverer releases — `todo`, `cancelled`, `unknown` — or is `done`.
/// 5. `todo` when no deliverer remains: the claim is released.
/// 6. `None` when every remaining deliverer is `draft` or `backlog`, which count as nothing.
#[must_use]
pub fn settled(categories: &[StatusCategory]) -> Option<StatusCategory> {
    use StatusCategory::{Backlog, Cancelled, Done, Draft, InProgress, Queued, Todo, Unknown};
    if categories.contains(&Done)
        && categories
            .iter()
            .all(|category| matches!(category, Done | Cancelled))
    {
        return Some(Done);
    }
    if categories.contains(&InProgress) {
        return Some(InProgress);
    }
    if categories.contains(&Queued) {
        return Some(Queued);
    }
    if categories
        .iter()
        .any(|category| matches!(category, Todo | Cancelled | Unknown | Done))
    {
        return Some(Todo);
    }
    if categories.is_empty() {
        return Some(Todo);
    }
    debug_assert!(
        categories
            .iter()
            .all(|category| matches!(category, Draft | Backlog))
    );
    None
}

/// Every entry of one task's list as a qualified id, reading a bare one as naming a task of
/// `near`, the source holding the list.
pub(crate) fn targets(list: &[TaskRef], near: &SourceName) -> Vec<GlobalId> {
    list.iter()
        .filter_map(|entry| entry.in_source(near).as_str().parse().ok())
        .collect()
}

/// A task as a verb reports it: under its qualified id, with every entry of its two lists
/// qualified too, so a reader never has to know which source a bare entry meant.
pub(crate) fn qualified_task(id: GlobalId, task: Task) -> Qualified<Task> {
    let delivers = task
        .delivers
        .iter()
        .map(|entry| entry.in_source(&id.source))
        .collect();
    let delivered_by = task
        .delivered_by
        .iter()
        .map(|entry| entry.in_source(&id.source))
        .collect();
    Qualified {
        id,
        item: Task {
            delivers,
            delivered_by,
            ..task
        },
    }
}

pub(super) fn source_failed(source: &ResolvedSource, error: SourceError) -> EngineError {
    EngineError::SourceFailed {
        name: source.name().to_string(),
        error,
    }
}
