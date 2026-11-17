//! `task delete`: removing one task from its source, and refusing to leave an edge dangling.
//!
//! The removal is the source's own [`TaskSource::delete_task`] — the call a copy that cannot
//! finish already takes its own creates back with — so what a source stores for the task alone
//! (a folder's file, its answers block and its assets; a board's issue) goes with it. Nothing
//! here touches a copy of the task in another source: a copy that finds its source gone reports
//! the counterpart as an orphan, and `project copy --close-orphans` is what closes it.
//!
//! A copy that withholds provenance writes no origin on its counterpart, so the deleted task's
//! own `onetaskgraph.copies` link is the only record of where it landed. Before removing a task
//! filed under a project, the delete therefore writes those links onto that project at
//! [`MetadataKey::DELETED_COPIES_KEY`] — ids only — for the next `project copy
//! --close-orphans` to close and take out. A source that cannot hold the record refuses the
//! delete, and a delete the source then refuses puts the record back as it was.
//!
//! [`TaskSource::delete_task`]: onetaskgraph_plugin_api::TaskSource::delete_task

use std::num::NonZeroU32;

use std::collections::BTreeMap;

use onetaskgraph_plugin_api::{Direction, MetadataKey, NativeId, Task};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Completeness, DependencyRequest, Engine, EngineError, Paging};
use crate::GlobalId;
use crate::resolve::ResolvedSource;

/// What a project records at [`MetadataKey::DELETED_COPIES_KEY`]: for each destination source
/// name, each counterpart's native id there mapped to the qualified id of the deleted task it
/// was a copy of. An entry that is not that shape is left out of what is read.
pub(super) type DeletedCopies = BTreeMap<String, BTreeMap<String, GlobalId>>;

/// The record `value` holds, its ill-formed entries left out.
pub(super) fn deleted_copies(value: Option<&Value>) -> DeletedCopies {
    let Some(Value::Object(held)) = value else {
        return DeletedCopies::new();
    };
    held.iter()
        .filter_map(|(destination, entries)| {
            let Value::Object(entries) = entries else {
                return None;
            };
            let entries: BTreeMap<String, GlobalId> = entries
                .iter()
                .filter_map(|(native, task)| Some((native.clone(), task.as_str()?.parse().ok()?)))
                .collect();
            (!entries.is_empty()).then(|| (destination.clone(), entries))
        })
        .collect()
}

/// `record` as the value written at [`MetadataKey::DELETED_COPIES_KEY`].
pub(super) fn deleted_copies_value(record: &DeletedCopies) -> Value {
    Value::Object(
        record
            .iter()
            .filter(|(_, entries)| !entries.is_empty())
            .map(|(destination, entries)| {
                let entries = entries
                    .iter()
                    .map(|(native, task)| (native.clone(), Value::String(task.to_string())))
                    .collect();
                (destination.clone(), Value::Object(entries))
            })
            .collect(),
    )
}

/// What `task delete` answers with: the qualified id of the task it removed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TaskDeleted {
    /// The task that was removed, as `<source>:<native-id>`.
    pub deleted: GlobalId,
}

impl Engine {
    /// Remove one task from its source, with everything its source stores for that task alone.
    ///
    /// Refused, removing nothing, when the task is not there, when its source has no write
    /// side, and when any item of the same source depends on it — a removal would leave that
    /// item's edge naming nothing. Its edges to the tasks it depends on go with it, and a copy
    /// of it in another source is not touched.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::UnknownSource`] for a source nothing configures,
    /// [`EngineError::DeleteNotWritable`] for one with no write side,
    /// [`EngineError::NoSuchTask`] when the task is not there,
    /// [`EngineError::TaskHasDependents`] naming each item that depends on it, and
    /// [`EngineError::SourceFailed`] when the source cannot be read or refuses the removal.
    pub async fn delete_task(&self, id: &GlobalId) -> Result<TaskDeleted, EngineError> {
        let source = self.built(&id.source)?;
        if !source.source().writes().is_supported() {
            return Err(EngineError::DeleteNotWritable {
                name: source.name().to_string(),
                kind: source.kind().to_owned(),
            });
        }
        let failed = |error| EngineError::SourceFailed {
            name: source.name().to_string(),
            error,
        };
        let Some(task) = source.source().get_task(&id.native).await.map_err(failed)? else {
            return Err(EngineError::NoSuchTask { id: id.to_string() });
        };
        let edges = self
            .task_dependencies_all(&DependencyRequest {
                id: id.clone(),
                direction: Direction::DependedOnBy,
                completeness: Completeness::NamingUnasked,
                paging: Paging {
                    limit: NonZeroU32::new(100).expect("a non-zero page"),
                    token: None,
                },
            })
            .await?;
        // A dependent this read could not see would be left naming nothing, so a read that did
        // not answer whole refuses rather than deleting past it.
        if let Some(failure) = edges.errors.into_iter().next() {
            return Err(EngineError::SourceFailed {
                name: failure.source.to_string(),
                error: failure.error,
            });
        }
        let mut dependents: Vec<GlobalId> = Vec::new();
        for edge in edges.items {
            let dependent = edge.from.id;
            if edge.to.id == *id
                && dependent.source == id.source
                && !dependents.contains(&dependent)
            {
                dependents.push(dependent);
            }
        }
        if !dependents.is_empty() {
            return Err(EngineError::TaskHasDependents {
                id: id.clone(),
                dependents,
            });
        }
        let recorded = record_copies(source, id, &task).await?;
        if let Err(error) = source.source().delete_task(&id.native).await {
            // The record would name the copies of a task that is still there, for a later copy
            // to close: it goes back to what it was.
            if let Some((project, before)) = recorded {
                let _ = source
                    .source()
                    .set_project_metadata(&project, &MetadataKey::deleted_copies(), &before)
                    .await;
            }
            return Err(failed(error));
        }
        Ok(TaskDeleted {
            deleted: id.clone(),
        })
    }
}

/// Write where `task`'s copies landed onto the project it is filed under, at
/// [`MetadataKey::DELETED_COPIES_KEY`], answering that project and what it held there before;
/// nothing for a task filed under no project, linking no copy, or whose project is gone.
///
/// # Errors
///
/// [`EngineError::SourceFailed`] when the project cannot be read or the record not written,
/// so the delete is refused rather than losing the only record of those copies.
async fn record_copies(
    source: &ResolvedSource,
    id: &GlobalId,
    task: &Task,
) -> Result<Option<(NativeId, Value)>, EngineError> {
    let Some(project) = &task.project else {
        return Ok(None);
    };
    let links = super::copy::counterparts_linked(task.metadata.get(MetadataKey::COPIES_KEY));
    let links: Vec<_> = links
        .into_iter()
        .filter(|(destination, _)| *destination != id.source)
        .collect();
    if links.is_empty() {
        return Ok(None);
    }
    let failed = |error| EngineError::SourceFailed {
        name: source.name().to_string(),
        error,
    };
    let Some(held) = source.source().get_project(project).await.map_err(failed)? else {
        return Ok(None);
    };
    let before = held
        .metadata
        .get(MetadataKey::DELETED_COPIES_KEY)
        .cloned()
        .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
    let mut record = deleted_copies(Some(&before));
    for (destination, native) in links {
        record
            .entry(destination.to_string())
            .or_default()
            .insert(native.to_string(), id.clone());
    }
    source
        .source()
        .set_project_metadata(
            project,
            &MetadataKey::deleted_copies(),
            &deleted_copies_value(&record),
        )
        .await
        .map_err(failed)?;
    Ok(Some((project.clone(), before)))
}
