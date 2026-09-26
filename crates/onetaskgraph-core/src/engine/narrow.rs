//! A task's priority and a task's content, each set on its own.
//!
//! Both verbs are one call to one source, with no compare-and-set: what lands is what was
//! asked for, whatever the task held a moment before. Neither moves the task's status, so no
//! delivered task is re-evaluated after one of these writes.
//!
//! A priority is refused here, before the source is asked, when the source declares it holds
//! none: a source that cannot hold a priority has nowhere to put one, and asking it anyway
//! would leave the refusal to each plugin's own wording — or, for a plugin written before
//! priorities existed, to a write that drops it in silence.

use onetaskgraph_plugin_api::Priority;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::delivery::source_failed;
use super::{Engine, EngineError};
use crate::GlobalId;
use crate::resolve::ResolvedSource;

/// What `task priority set` answers with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TaskPrioritySet {
    /// The task whose priority was set.
    pub id: GlobalId,
    /// Its priority as its source reads it back after the write.
    pub priority: Priority,
}

/// What `task content set` answers with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TaskContentSet {
    /// The task whose content was replaced.
    pub id: GlobalId,
}

impl Engine {
    /// Set one task's priority, and nothing else about it.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::UnknownSource`] for a source nothing configures,
    /// [`EngineError::PriorityNotWritable`] for one with no write side,
    /// [`EngineError::NoPriority`] for one declaring it holds no priority — neither of which
    /// is asked — [`EngineError::NoSuchTask`] when the task is not there, and
    /// [`EngineError::SourceFailed`] when the source refuses, a board with no option for the
    /// priority included.
    pub async fn set_task_priority(
        &self,
        id: &GlobalId,
        priority: Priority,
    ) -> Result<TaskPrioritySet, EngineError> {
        let source = self.built(&id.source)?;
        if !source.source().writes().is_supported() {
            return Err(EngineError::PriorityNotWritable {
                name: source.name().to_string(),
                kind: source.kind().to_owned(),
            });
        }
        // Every value, `none` included: a source that holds no priority has none to clear,
        // and the verb asks it for a write it has no field for.
        if !source.source().capabilities().priority.is_native() {
            return Err(no_priority(source, &id.to_string(), priority));
        }
        let read = source
            .source()
            .set_task_priority(&id.native, priority)
            .await
            .map_err(|error| source_failed(source, error))?
            .ok_or_else(|| EngineError::NoSuchTask { id: id.to_string() })?;
        Ok(TaskPrioritySet {
            id: id.clone(),
            priority: read,
        })
    }

    /// Replace one task's content with `content`, byte for byte, and nothing else about it.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::UnknownSource`] for a source nothing configures,
    /// [`EngineError::ContentNotWritable`] for one with no write side, which is not asked,
    /// [`EngineError::NoSuchTask`] when the task is not there, and
    /// [`EngineError::SourceFailed`] when the source refuses.
    pub async fn set_task_content(
        &self,
        id: &GlobalId,
        content: &str,
    ) -> Result<TaskContentSet, EngineError> {
        let source = self.built(&id.source)?;
        if !source.source().writes().is_supported() {
            return Err(EngineError::ContentNotWritable {
                name: source.name().to_string(),
                kind: source.kind().to_owned(),
            });
        }
        source
            .source()
            .set_task_content(&id.native, content)
            .await
            .map_err(|error| source_failed(source, error))?
            .ok_or_else(|| EngineError::NoSuchTask { id: id.to_string() })?;
        Ok(TaskContentSet { id: id.clone() })
    }
}

/// Refuse `priority` for `task` when `source` declares it holds no priority at all.
///
/// `none` always passes: it is what such a source already reports for every task, so a write
/// carrying it asks the source for nothing it cannot hold. That is what lets a copy carrying
/// `none` into such a source write exactly as it did before priorities existed.
pub(super) fn holds_priority(
    source: &ResolvedSource,
    task: &str,
    priority: Priority,
) -> Result<(), EngineError> {
    if priority == Priority::None || source.source().capabilities().priority.is_native() {
        return Ok(());
    }
    Err(no_priority(source, task, priority))
}

/// The refusal of `priority` for `task` at a source that holds none.
fn no_priority(source: &ResolvedSource, task: &str, priority: Priority) -> EngineError {
    EngineError::NoPriority {
        name: source.name().to_string(),
        kind: source.kind().to_owned(),
        task: task.to_owned(),
        priority,
    }
}
