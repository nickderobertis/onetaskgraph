//! The one key a narrow metadata write names, and the refusal a source that cannot make one
//! answers with.
//!
//! A narrow metadata write sets one key of one record's metadata and changes nothing else
//! about the record — see [`TaskSource::set_task_metadata`](crate::TaskSource::set_task_metadata).
//! What a caller may name there is narrower than what a record may hold: a record read from a
//! source carries this product's own `onetaskgraph.` keys, and a caller writing one of them
//! through this seam would be editing the store's bookkeeping by hand.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::SourceError;

/// One caller-owned metadata key: `<namespace>.<name>`, at least two non-empty segments
/// separated by dots, whose first segment is not [`MetadataKey::RESERVED_NAMESPACE`].
///
/// Validated wherever one is built, deserialized included, so a plugin handed one never has
/// to ask whether it names a key this product owns.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(try_from = "String", into = "String")]
pub struct MetadataKey(String);

impl MetadataKey {
    /// The namespace this product owns, and a narrow write therefore never names.
    ///
    /// Every key the contract reserves lives under it — `onetaskgraph.origin`,
    /// `onetaskgraph.repositories`, `onetaskgraph.depends_on`, `onetaskgraph.delivers`,
    /// `onetaskgraph.delivered_by` and `onetaskgraph.item_kind` — and so does any key it
    /// reserves later, which is why the whole namespace is refused rather than a list.
    pub const RESERVED_NAMESPACE: &'static str = "onetaskgraph";

    /// One key, once it is established it is a caller's own dotted key.
    ///
    /// # Errors
    ///
    /// Returns a message saying why, and what to write instead, when the key has no dot, has
    /// an empty segment, or is in [`Self::RESERVED_NAMESPACE`].
    pub fn new(key: impl Into<String>) -> Result<Self, String> {
        let key = key.into();
        let segments: Vec<&str> = key.split('.').collect();
        if segments.len() < 2 {
            return Err(format!(
                "the metadata key {key:?} has no namespace: a key is `<namespace>.<name>`, two \
                 or more non-empty segments separated by dots; next: name it under a namespace \
                 of your own, such as `myapp.{key}`"
            ));
        }
        if segments.iter().any(|segment| segment.is_empty()) {
            return Err(format!(
                "the metadata key {key:?} has an empty segment: a key is `<namespace>.<name>`, \
                 two or more non-empty segments separated by dots; next: remove the extra dot"
            ));
        }
        if segments[0] == Self::RESERVED_NAMESPACE {
            return Err(format!(
                "the metadata key {key:?} is in the `{}.` namespace, which this product owns \
                 and keeps in step itself; next: name the key under a namespace of your own",
                Self::RESERVED_NAMESPACE
            ));
        }
        Ok(Self(key))
    }

    /// The key as a record's metadata map spells it.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for MetadataKey {
    type Error = String;

    fn try_from(key: String) -> Result<Self, Self::Error> {
        Self::new(key)
    }
}

impl From<MetadataKey> for String {
    fn from(key: MetadataKey) -> Self {
        key.0
    }
}

impl std::fmt::Display for MetadataKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Which kind of record a narrow metadata write names.
///
/// The three a record can be, and no fourth: a refusal, a not-found error or a protocol guard
/// that names the record takes one of these rather than a noun, so it cannot name something
/// that is not a record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MetadataRecord {
    /// A task, written through [`TaskSource::set_task_metadata`](crate::TaskSource::set_task_metadata).
    Task,
    /// A project, written through
    /// [`TaskSource::set_project_metadata`](crate::TaskSource::set_project_metadata).
    Project,
    /// A document, written through
    /// [`TaskSource::set_document_metadata`](crate::TaskSource::set_document_metadata).
    Document,
}

impl MetadataRecord {
    /// The record's noun as a message spells it: `task`, `project` or `document`.
    #[must_use]
    pub const fn noun(self) -> &'static str {
        match self {
            Self::Task => "task",
            Self::Project => "project",
            Self::Document => "document",
        }
    }
}

impl std::fmt::Display for MetadataRecord {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.noun())
    }
}

/// The refusal a source answers a narrow metadata write with when it cannot make one on its
/// own.
///
/// It reads `the linear plugin cannot write a document's metadata on its own`, naming the
/// record. Spelled once beside [`unwritable_field`](crate::unwritable_field) for that
/// function's reason: the trait's defaults and the engine's refusal of a plugin whose
/// handshake does not declare the write say the same thing in the same words.
#[must_use]
pub fn unwritable_metadata(kind: &str, record: MetadataRecord) -> SourceError {
    SourceError::Refused {
        message: format!("the {kind} plugin cannot write a {record}'s metadata on its own"),
    }
}
