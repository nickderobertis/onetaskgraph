//! `task delete`: removing one task from its source, and refusing to leave an edge dangling.
//!
//! The removal is the source's own [`TaskSource::delete_task`] — the call a copy that cannot
//! finish already takes its own creates back with — so what a source stores for the task alone
//! (a folder's file, its answers block and its assets; a board's issue) goes with it. Nothing
//! here touches a copy of the task in another source: a copy that finds its source gone reports
//! the counterpart as an orphan, and `project copy --close-orphans` is what closes it.
//!
//! [`TaskSource::delete_task`]: onetaskgraph_plugin_api::TaskSource::delete_task

use std::num::NonZeroU32;

use onetaskgraph_plugin_api::Direction;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{Completeness, DependencyRequest, Engine, EngineError, Paging};
use crate::GlobalId;

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
        if source
            .source()
            .get_task(&id.native)
            .await
            .map_err(failed)?
            .is_none()
        {
            return Err(EngineError::NoSuchTask { id: id.to_string() });
        }
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
        let mut dependents: Vec<String> = Vec::new();
        for edge in edges.items {
            let dependent = edge.from.id.to_string();
            if edge.to.id == *id
                && edge.from.id.source == id.source
                && !dependents.contains(&dependent)
            {
                dependents.push(dependent);
            }
        }
        if !dependents.is_empty() {
            return Err(EngineError::TaskHasDependents {
                id: id.to_string(),
                dependents,
            });
        }
        source
            .source()
            .delete_task(&id.native)
            .await
            .map_err(failed)?;
        Ok(TaskDeleted {
            deleted: id.clone(),
        })
    }
}
