//! One targeted update of one existing task: every field the caller names, and nothing else.
//!
//! One call to one source. The engine reads nothing of its own around it: the source's answer
//! carries the task as it reads back, the fields it wrote and the `delivers` it held before,
//! and every figure this reports is read off that answer. On a hosted backend that is the
//! difference between one read of the item and two, on every update — which is the whole of
//! why this exists beside a copy and the narrow verbs.
//!
//! `delivers` is kept exactly as `task status set` keeps it: whenever `status` or `delivers` is
//! named, every task the list names now, and every task it dropped, is re-evaluated.

use std::collections::BTreeSet;

use onetaskgraph_plugin_api::{
    DependencyEdge, DependencyEndpoint, ItemKind, NativeId, SourceName, Task, TaskUpdate,
    UpdatedField,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::boundary::{Exposure, Held};
use super::copy::{Spent, readings, spent_between};
use super::delivery::{Delivered, qualified_task, source_failed, targets};
use super::narrow::holds_priority;
use super::{Engine, EngineError};
use crate::GlobalId;

/// What `task update` answers with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TaskUpdated {
    /// The task updated.
    pub id: GlobalId,
    /// The task as its source reads it back, with every entry of its `delivers` and
    /// `delivered_by` qualified.
    pub task: Task,
    /// The fields the source actually wrote — empty when nothing differed.
    pub written: BTreeSet<UpdatedField>,
    /// As `set_task_status` reports it; re-evaluated whenever `status` or `delivers` was
    /// named, empty otherwise.
    pub delivered: Vec<Delivered>,
    /// What the source's meter says this call spent, as `CopyReport::spent` — and absent,
    /// never zero, when the source does not meter its own requests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spent: Option<Spent>,
}

impl Engine {
    /// Apply a targeted update to one task — every field `update` names, and nothing else —
    /// then keep every task it delivers in step with it when `status` or `delivers` was named.
    ///
    /// The source is asked once, and nothing else of it is read: a field already holding the
    /// requested value is sent no write, and an update in which nothing differs writes nothing
    /// at all — an update naming no field included, which answers the task as its source reads
    /// it and re-evaluates nothing; the CLI refuses that one as a usage error before it gets
    /// here, a library caller is answered. `update.depends_on` may name its far ends qualified; one in this task's own
    /// source reaches the source as its native id, and each edge's `from` is this task.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::UnknownSource`] for a source nothing configures,
    /// [`EngineError::UpdateNotWritable`] for one with no write side,
    /// [`EngineError::NoPriority`] for a priority other than `none` to a source that holds
    /// none — neither of which is asked — [`EngineError::NoSuchTask`] when the task is not
    /// there, and [`EngineError::SourceFailed`] carrying the source's own [`SourceError`] when
    /// it refuses or fails, so its class and its `retry_after_seconds` are what a caller
    /// reads. An update both setting and removing one metadata key is refused that way too,
    /// before anything is sent, in the words the source itself refuses it with. A delivered
    /// task that cannot be kept in step is not an error: it is reported `failed` beside the
    /// update that landed.
    ///
    /// [`SourceError`]: onetaskgraph_plugin_api::SourceError
    pub async fn update_task(
        &self,
        id: &GlobalId,
        update: &TaskUpdate,
    ) -> Result<TaskUpdated, EngineError> {
        let source = self.built(&id.source)?;
        if !source.source().writes().is_supported() {
            return Err(EngineError::UpdateNotWritable {
                name: source.name().to_string(),
                kind: source.kind().to_owned(),
            });
        }
        update
            .consistent()
            .map_err(|error| source_failed(source, error))?;
        if let Some(priority) = update.priority {
            holds_priority(source, &id.to_string(), priority)?;
        }
        let update = TaskUpdate {
            depends_on: update
                .depends_on
                .as_ref()
                .map(|edges| near_edges(edges, &id.native, &id.source)),
            ..update.clone()
        };
        let near = id.source.as_str();
        let references: Vec<(&str, &str)> = update
            .depends_on
            .iter()
            .flatten()
            .map(|edge| {
                let far = edge.to.id();
                match edge.to.source() {
                    Some(source) => (source, &far[source.len() + 1..]),
                    None => (near, far),
                }
            })
            .chain(
                update
                    .delivers
                    .iter()
                    .flatten()
                    .map(|entry| entry.parts(near)),
            )
            .collect();
        self.withhold_private_references(&id.source, &id.to_string(), references)?;
        self.admit_existing(
            source,
            &id.to_string(),
            &id.native,
            Held::Task,
            &Exposure::of_update(&update),
        )
        .await?;
        let before = readings(&[source]).await;
        let outcome = source
            .source()
            .update_task(&id.native, &update)
            .await
            .map_err(|error| source_failed(source, error))?
            .ok_or_else(|| EngineError::NoSuchTask { id: id.to_string() })?;
        let delivered = if update.status.is_some() || update.delivers.is_some() {
            let now = targets(&outcome.task.delivers, &id.source);
            let dropped = targets(&outcome.delivers_before, &id.source);
            self.deliver(id, outcome.task.status.category, &now, &dropped)
                .await
        } else {
            Vec::new()
        };
        let spent = spent_between(&before, &readings(&[source]).await);
        Ok(TaskUpdated {
            id: id.clone(),
            task: qualified_task(id.clone(), outcome.task).item,
            written: outcome.written,
            delivered,
            spent,
        })
    }
}

/// `edges` as the task `near` of `source` holds them: each starting at `near`, and each far
/// end in `source` itself named by its native id, which is how a source names one of its own.
fn near_edges(
    edges: &[DependencyEdge],
    near: &NativeId,
    source: &SourceName,
) -> Vec<DependencyEdge> {
    edges
        .iter()
        .map(|edge| {
            let to = match edge.to.id().parse::<GlobalId>() {
                Ok(far) if &far.source == source => {
                    DependencyEndpoint::from_native(far.native, edge.to.kind)
                }
                _ => edge.to.clone(),
            };
            DependencyEdge {
                from: DependencyEndpoint::from_native(near.clone(), ItemKind::Task),
                to,
                kind: edge.kind,
            }
        })
        .collect()
}
