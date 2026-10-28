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

use onetaskgraph_plugin_api::{MetadataRecord, Priority};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::Spent;
use super::boundary::{Exposure, Held};
use super::delivery::source_failed;
use super::{Engine, EngineError};
use crate::GlobalId;
use crate::cache::{WriteFallback, WriteResult};
use crate::resolve::ResolvedSource;

/// What `task priority set` answers with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TaskPrioritySet {
    /// The task whose priority was set.
    pub id: GlobalId,
    /// Its priority as its source reads it back after the write.
    pub priority: Priority,
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
}

/// What `task content set` answers with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TaskContentSet {
    /// The task whose content was replaced.
    pub id: GlobalId,
    /// The content the task holds after the write: what its source's backend stores, as the
    /// write's own answer reported it, where the source reports one — a backend may normalize
    /// the spelling of the Markdown it is written while keeping its rendered meaning — and
    /// otherwise exactly the bytes sent. An answer, never stored.
    pub content: String,
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
        self.admit_existing(
            source,
            &id.to_string(),
            &id.native,
            Held::Task,
            &Exposure::metadata([priority.as_str().to_owned()]),
        )
        .await?;
        let read = source
            .source()
            .set_task_priority(&id.native, priority)
            .await
            .map_err(|error| source_failed(source, error))?
            .ok_or_else(|| EngineError::NoSuchTask { id: id.to_string() })?;
        let (result, fallback, spent) = self.expected_parts(id);
        Ok(TaskPrioritySet {
            id: id.clone(),
            priority: read,
            result,
            fallback,
            spent,
        })
    }

    /// Replace one task's content with `content`, byte for byte, and nothing else about it.
    ///
    /// Answers with the content the task then holds, taken from the write's own answer where
    /// the source reports what its backend stores, with no read of the task after the write.
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
        self.admit_existing(
            source,
            &id.to_string(),
            &id.native,
            Held::Task,
            &Exposure::text(content),
        )
        .await?;
        super::stored::forget(source, MetadataRecord::Task, &id.native);
        source
            .source()
            .set_task_content(&id.native, content)
            .await
            .map_err(|error| source_failed(source, error))?
            .ok_or_else(|| EngineError::NoSuchTask { id: id.to_string() })?;
        let held = source
            .source()
            .stored_content(MetadataRecord::Task, &id.native)
            .unwrap_or_else(|| content.to_owned());
        let (result, fallback, spent) = self.expected_parts(id);
        Ok(TaskContentSet {
            id: id.clone(),
            content: held,
            result,
            fallback,
            spent,
        })
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
