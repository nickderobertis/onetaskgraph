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
//!    `todo`, `queued`, `in-progress` or `in-review` and the result differs from what it
//!    holds. A result of `in-review` is written as `in-progress` to a delivered task whose
//!    own source has no name for `in-review` on a task, by [`Engine::maps_status`], decided
//!    before any request — so delivery never sends a write refused for want of a name.
//!
//! Every write of such a task re-evaluates, a write that changes nothing about the deliverer
//! included, so a retried write re-fires the rule. Nothing here is written down outside the
//! plugins: `delivered_by` lives on the delivered task, in its own source, and every
//! evaluation reads the deliverers afresh.

use onetaskgraph_plugin_api::{
    Classification, ItemKind, SourceError, SourceName, Status, StatusCategory, Task, TaskRef,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::Spent;
use super::boundary::{Exposure, Held};
use super::{ConfiguredSource, Engine, EngineError, Qualified};
use crate::cache::{WriteFallback, WriteResult};
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
    /// What each mutation an expected write sent came to — present when the write was made
    /// with `--expect` on a source that reports its writes' parts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<WriteResult>,
    /// That an expected write read its target first, and why — absent when it sent only its
    /// mutations, and for a write made without `--expect`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<WriteFallback>,
    /// What an expected write spent at the sources that meter their own requests, its
    /// fallback read included — absent for a write made without `--expect`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spent: Option<Spent>,
    /// The public projection the new status category was mirrored to — absent for a task
    /// with none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projection: Option<GlobalId>,
}

/// What `project status set` and [`Engine::set_project_status`] answer with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ProjectStatusSet {
    /// The project whose status was set.
    pub id: GlobalId,
    /// Its status as its source reads it back.
    pub status: Status,
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
    /// Whether `source` has a name to write `category` as on an item of `kind`, so a status
    /// write of it would not be refused for want of one.
    ///
    /// Answered from the source's configuration alone, sending no request to any backend: its
    /// `status_mapping` over that plugin's shipped defaults, every category a folder of
    /// Markdown's mapping has a word for or the in-memory source writes, and for a stdio plugin
    /// what its handshake lists (`docs/plugin-protocol.md` §3.5). `false` for a category the
    /// mapping does not name, sets to `null`, or names for the other kind only, and for a
    /// category a stdio plugin's handshake does not list. A mapped name the backend's own
    /// vocabulary lacks still answers `true`: only a write discovers that.
    ///
    /// A caller deciding which category to write — `in-review` where a board has a review
    /// column, `done` otherwise — asks this before its one write, rather than writing and
    /// retrying on a refusal.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::UnknownSource`] for a source nothing configures, and
    /// [`EngineError::SourceUnavailable`] for one configured but not built.
    pub fn maps_status(
        &self,
        source: &SourceName,
        kind: ItemKind,
        category: StatusCategory,
    ) -> Result<bool, EngineError> {
        Ok(self.built(source)?.source().maps_status(kind, category))
    }

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
        self.admit_existing(
            source,
            &id.to_string(),
            &id.native,
            Held::Task,
            &Exposure::metadata([category_name(category)]),
        )
        .await?;
        let no_such_task = || EngineError::NoSuchTask { id: id.to_string() };
        // One call answering the task as it now reads, rather than a read and then a write: a
        // source whose status write answers the whole task saves the read, and every other
        // source makes exactly those two calls itself.
        let task = source
            .source()
            .set_task_status_reading(&id.native, category)
            .await
            .map_err(|error| source_failed(source, error))?
            .ok_or_else(no_such_task)?;
        let status = task.status.clone();
        // Taken before the delivered tasks are kept in step, which reads and writes other
        // tasks and none of them this write's own.
        let (result, fallback, spent) = self.expected_parts(id);
        let result = self.verb_result(source, id, Some(&task), result).await;
        let delivers = targets(&task.delivers, &id.source);
        let delivered = self
            .deliver(
                id,
                task.classification,
                status.category,
                &delivers,
                &delivers,
            )
            .await;
        let projection = self.mirror_status(id, &task, status.category).await?;
        Ok(TaskStatusSet {
            id: id.clone(),
            status,
            delivered,
            result,
            fallback,
            spent,
            projection,
        })
    }

    /// Close one task as a duplicate of `survivor`, a task of its own source: its status
    /// `cancelled` — natively a duplicate where its backend can say so, GitHub's duplicate
    /// closure and Linear's duplicate relation — and the typed `duplicate` link recorded at both
    /// ends, answered as [`set_task_status`](Self::set_task_status) answers a status write.
    ///
    /// It is the targeted update closing nothing but that, so a task already closed as that
    /// survivor's duplicate is written nothing again.
    ///
    /// # Errors
    ///
    /// As [`update_task`](Self::update_task): refused before anything is sent for a survivor
    /// in another source or the task itself, and for a stdio plugin whose handshake does not
    /// declare duplicate closes.
    pub async fn close_as_duplicate(
        &self,
        id: &GlobalId,
        survivor: &onetaskgraph_plugin_api::TaskRef,
    ) -> Result<TaskStatusSet, EngineError> {
        let updated = self
            .update_task(
                id,
                &onetaskgraph_plugin_api::TaskUpdate {
                    duplicate_of: Some(survivor.clone()),
                    ..onetaskgraph_plugin_api::TaskUpdate::default()
                },
            )
            .await?;
        Ok(TaskStatusSet {
            id: updated.id,
            status: updated.task.status.clone(),
            delivered: updated.delivered,
            result: updated.result,
            fallback: updated.fallback,
            spent: updated.spent,
            projection: updated.projection,
        })
    }

    /// Set one project's status, and nothing else about it.
    ///
    /// A project delivers no task, so nothing is kept in step after it.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::UnknownSource`] for a source nothing configures,
    /// [`EngineError::StatusNotWritable`] for one with no write side,
    /// [`EngineError::NoSuchProject`] when the project is not there, and
    /// [`EngineError::SourceFailed`] when the source refuses — a category it has disabled
    /// included.
    pub async fn set_project_status(
        &self,
        id: &GlobalId,
        category: StatusCategory,
    ) -> Result<ProjectStatusSet, EngineError> {
        let source = self.status_writable(&id.source)?;
        self.admit_existing(
            source,
            &id.to_string(),
            &id.native,
            Held::Project,
            &Exposure::metadata([category_name(category)]),
        )
        .await?;
        let status = source
            .source()
            .set_project_status(&id.native, category)
            .await
            .map_err(|error| source_failed(source, error))?
            .ok_or_else(|| EngineError::NoSuchProject { id: id.to_string() })?;
        // What a plugin answers is input: a write that reads back as another category did not
        // land as asked, and is refused as malformed rather than reported as having written it.
        if status.category != category {
            return Err(source_failed(
                source,
                SourceError::Malformed {
                    message: format!(
                        "a write of {id} to {} read back as {}",
                        category_name(category),
                        category_name(status.category)
                    ),
                },
            ));
        }
        Ok(ProjectStatusSet {
            id: id.clone(),
            status,
        })
    }

    /// Keep every task `deliverer` delivers `now`, and every one it delivered `before` and
    /// dropped, in step with it — one entry each, in that order.
    ///
    /// `classification` is the deliverer's own: a delivered task anywhere not declared private
    /// is never handed the id of one classified private, nor of one in a source declared
    /// private, in its `delivered_by` (see [`Engine::withholds`]).
    pub(crate) async fn deliver(
        &self,
        deliverer: &GlobalId,
        classification: Classification,
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
            delivered.push(
                self.evaluate(deliverer, classification, category, ticket, kept)
                    .await,
            );
        }
        delivered
    }

    /// Re-evaluate one delivered task for one deliverer, which names it when `kept` and has
    /// just dropped it otherwise.
    async fn evaluate(
        &self,
        deliverer: &GlobalId,
        classification: Classification,
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
            StatusCategory::Todo
                | StatusCategory::Queued
                | StatusCategory::InProgress
                | StatusCategory::InReview
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
        // What the ticket's own record holds of them: a deliverer it withholds still counts
        // toward the rule, and is kept only in the deliverer's own `delivers`.
        let private = if classification == Classification::Private {
            std::slice::from_ref(deliverer)
        } else {
            &[]
        };
        let written: Vec<&GlobalId> = kept_by
            .iter()
            .filter(|other| !self.withholds(&ticket.source, other, private))
            .collect();
        // The ticket is held to its own record alone: a task inherits nothing from the project
        // it is filed under.
        let ticket_class = task.classification;
        if !written.iter().copied().eq(&held) {
            let list: Vec<TaskRef> = written
                .iter()
                .map(|other| TaskRef::qualified(&other.source, &other.native))
                .collect();
            if let Err(error) = self
                .admit_known(
                    source,
                    &ticket.to_string(),
                    &ticket.native,
                    ticket_class,
                    &task.repositories,
                    &Exposure::metadata(list.iter().map(ToString::to_string)),
                )
                .await
            {
                return entry(failed(Some(from), &error), Vec::new());
            }
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
        // A ticket whose own source has no name for `in-review` takes what the rules after it
        // give a deliverer that is still working, `in-progress`: decided from configuration,
        // before any request, so no write is sent that the source refuses for want of a name.
        let Some(to) = settled(&categories)
            .map(|to| {
                if to == StatusCategory::InReview
                    && !source
                        .source()
                        .maps_status(ItemKind::Task, StatusCategory::InReview)
                {
                    StatusCategory::InProgress
                } else {
                    to
                }
            })
            .filter(|to| *to != from)
        else {
            return entry(DeliveryOutcome::Unchanged { from }, pruned);
        };
        if let Err(error) = self
            .admit_known(
                source,
                &ticket.to_string(),
                &ticket.native,
                ticket_class,
                &task.repositories,
                &Exposure::metadata([category_name(to)]),
            )
            .await
        {
            return entry(failed(Some(from), &error), pruned);
        }
        match source.source().set_task_status(&ticket.native, to).await {
            // A ticket with a public projection hands it the category too; a mirror that could
            // not be made is this ticket's failure, its own status written.
            Ok(Some(status)) => {
                match Box::pin(self.mirror_status(ticket, &task, status.category)).await {
                    Ok(_) => entry(
                        DeliveryOutcome::Written {
                            from,
                            to: status.category,
                        },
                        pruned,
                    ),
                    Err(error) => entry(failed(Some(from), &error), pruned),
                }
            }
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
/// 2. `in-review` when any deliverer is `in-review`.
/// 3. `in-progress` when any deliverer is `in-progress`.
/// 4. `queued` when any deliverer is `queued`.
/// 5. `todo` when any deliverer releases — `todo`, `cancelled`, `unknown` — or is `done`.
/// 6. `todo` when no deliverer remains: the claim is released.
/// 7. `None` when every remaining deliverer is `draft` or `backlog`, which count as nothing.
///
/// So the ranking runs `done`, `in-review`, `in-progress`, `queued`, `todo`. This answers the
/// category alone; a delivered task whose own source has no name for `in-review` is written
/// `in-progress` in its place.
#[must_use]
pub fn settled(categories: &[StatusCategory]) -> Option<StatusCategory> {
    use StatusCategory::{
        Backlog, Cancelled, Done, Draft, InProgress, InReview, Queued, Todo, Unknown,
    };
    if categories.contains(&Done)
        && categories
            .iter()
            .all(|category| matches!(category, Done | Cancelled))
    {
        return Some(Done);
    }
    if categories.contains(&InReview) {
        return Some(InReview);
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

/// A category as the wire spells it, for a check reading what a status write carries.
fn category_name(category: StatusCategory) -> String {
    serde_json::to_value(category)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}
