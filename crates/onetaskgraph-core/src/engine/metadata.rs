//! One key of one record's metadata, set on its own.
//!
//! The three verbs — `task`, `project` and `document metadata set` — share one answer and one
//! path through the engine, which differ only in which of the source's three narrow writes is
//! called and what a missing record is called. Nothing here reads the record first: the
//! source's own write says whether it holds the record, and the record it answers with is
//! what the answer is read off, so what is reported is what the source now holds rather than
//! what it was handed.
//!
//! Metadata is not status, so no delivered task is re-evaluated after one of these writes.

use onetaskgraph_plugin_api::{Location, MetadataKey, MetadataRecord, SourceError};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::delivery::source_failed;
use super::{Engine, EngineError};
use crate::GlobalId;
use crate::resolve::ResolvedSource;

/// What `task`, `project` and `document metadata set` answer with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MetadataSet {
    /// The record whose metadata key was set.
    pub id: GlobalId,
    /// The key that was set.
    pub key: MetadataKey,
    /// The value the source holds under `key` as it reads the record back after the write —
    /// which is not always the value it was handed.
    pub value: Value,
    /// Where the source reports the record to be, left out when it does not say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<Location>,
}

impl Engine {
    /// Set one key of one task's metadata, and nothing else about it.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::UnknownSource`] for a source nothing configures,
    /// [`EngineError::MetadataNotWritable`] for one with no write side,
    /// [`EngineError::NoSuchTask`] when the task is not there, and
    /// [`EngineError::SourceFailed`] when the source refuses — a plugin that cannot write one
    /// key on its own included.
    pub async fn set_task_metadata(
        &self,
        id: &GlobalId,
        key: &MetadataKey,
        value: &Value,
    ) -> Result<MetadataSet, EngineError> {
        let source = self.metadata_writable(id, MetadataRecord::Task)?;
        let task = source
            .source()
            .set_task_metadata(&id.native, key, value)
            .await
            .map_err(|error| source_failed(source, error))?
            .ok_or_else(|| EngineError::NoSuchTask { id: id.to_string() })?;
        answer(source, id, key, &task.metadata, task.location)
    }

    /// Set one key of one project's metadata, and nothing else about it.
    ///
    /// # Errors
    ///
    /// As [`set_task_metadata`](Self::set_task_metadata), with
    /// [`EngineError::NoSuchProject`] when the project is not there.
    pub async fn set_project_metadata(
        &self,
        id: &GlobalId,
        key: &MetadataKey,
        value: &Value,
    ) -> Result<MetadataSet, EngineError> {
        let source = self.metadata_writable(id, MetadataRecord::Project)?;
        let project = source
            .source()
            .set_project_metadata(&id.native, key, value)
            .await
            .map_err(|error| source_failed(source, error))?
            .ok_or_else(|| EngineError::NoSuchProject { id: id.to_string() })?;
        answer(source, id, key, &project.metadata, project.location)
    }

    /// Set one key of one document's metadata, and nothing else about it.
    ///
    /// # Errors
    ///
    /// As [`set_task_metadata`](Self::set_task_metadata), with [`EngineError::NoDocuments`]
    /// for a source declaring it has none — which is never asked — and
    /// [`EngineError::NoSuchDocument`] when the document is not there.
    pub async fn set_document_metadata(
        &self,
        id: &GlobalId,
        key: &MetadataKey,
        value: &Value,
    ) -> Result<MetadataSet, EngineError> {
        let source = self.metadata_writable(id, MetadataRecord::Document)?;
        let document = source
            .source()
            .set_document_metadata(&id.native, key, value)
            .await
            .map_err(|error| source_failed(source, error))?
            .ok_or_else(|| EngineError::NoSuchDocument { id: id.to_string() })?;
        answer(source, id, key, &document.metadata, document.location)
    }

    /// The built source `id` names, when a metadata write of `record` may be sent to it.
    fn metadata_writable(
        &self,
        id: &GlobalId,
        record: MetadataRecord,
    ) -> Result<&ResolvedSource, EngineError> {
        let source = self.built(&id.source)?;
        if !source.source().writes().is_supported() {
            return Err(EngineError::MetadataNotWritable {
                name: source.name().to_string(),
                kind: source.kind().to_owned(),
                record,
            });
        }
        if record == MetadataRecord::Document
            && !source.source().capabilities().documents.is_native()
        {
            return Err(EngineError::NoDocuments {
                name: source.name().to_string(),
                kind: source.kind().to_owned(),
            });
        }
        Ok(source)
    }
}

/// The answer, read off the record the source answered the write with.
///
/// A record that does not hold the key it was just written under is not an answer this can
/// report a value from, so it is refused as malformed rather than reported as `null`.
fn answer(
    source: &ResolvedSource,
    id: &GlobalId,
    key: &MetadataKey,
    metadata: &std::collections::BTreeMap<String, Value>,
    location: Option<Location>,
) -> Result<MetadataSet, EngineError> {
    let value = metadata.get(key.as_str()).cloned().ok_or_else(|| {
        source_failed(
            source,
            SourceError::Malformed {
                message: format!(
                    "the record {id} it answered the write with does not hold the key {key} it \
                     was just written under"
                ),
            },
        )
    })?;
    Ok(MetadataSet {
        id: id.clone(),
        key: key.clone(),
        value,
        location,
    })
}
