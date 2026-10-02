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
    /// `onetaskgraph.delivered_by`, `onetaskgraph.item_kind`, `onetaskgraph.template`
    /// ([`Self::TEMPLATE_KEY`]), `onetaskgraph.copies` ([`Self::COPIES_KEY`]),
    /// `onetaskgraph.members` ([`Self::MEMBERS_KEY`]) and `onetaskgraph.member_of`
    /// ([`Self::MEMBER_OF_KEY`]) — and so does any key it reserves later, which is why the
    /// whole namespace is refused rather than a list.
    pub const RESERVED_NAMESPACE: &'static str = "onetaskgraph";

    /// The reserved key a task or a document rendered from a template records where it came
    /// from under: an object holding the template's reference, the chain digest it was
    /// rendered with, and the SHA-256 of its content and of its resolved answers.
    ///
    /// A key of this product's own, so no [`MetadataKey`] can name it: only a rendering write
    /// — [`TaskSource::write_task_rendered`](crate::TaskSource::write_task_rendered) and
    /// [`TaskSource::set_task_rendering`](crate::TaskSource::set_task_rendering) and their
    /// document siblings — ever sets it, and a copy carries it like any other entry. The
    /// engine builds and reads the value; a plugin only puts it where its metadata lives.
    pub const TEMPLATE_KEY: &'static str = "onetaskgraph.template";

    /// The reserved key a copied item records where it landed under: a JSON object mapping a
    /// destination source name to the qualified id of that item's counterpart there, at
    /// most one entry per destination.
    ///
    /// A key of this product's own, so [`Self::new`] refuses it like every other key in the
    /// namespace: only the engine's copy writes it, through the narrow metadata write, and
    /// [`Self::copies`] is how it names the key there. A plugin only puts it where its
    /// metadata lives.
    pub const COPIES_KEY: &'static str = "onetaskgraph.copies";

    /// The reserved key a **home** project records its member projects under: a JSON list of
    /// qualified project ids, each in a different source from the home and from the others,
    /// so a home has at most one member per source.
    ///
    /// Store-owned, like every key in the namespace: only the engine's routed writes keep it —
    /// a copy, or a `task create` routed away from its project's source — through the home's
    /// own project write, and nothing a caller types can name it. A plugin
    /// only puts it where its metadata lives.
    pub const MEMBERS_KEY: &'static str = "onetaskgraph.members";

    /// The reserved key a **member** project records its home under: the home's qualified id.
    ///
    /// Written once, when a routed write — a copy or a `task create` — creates the member, and
    /// never by a caller.
    pub const MEMBER_OF_KEY: &'static str = "onetaskgraph.member_of";

    /// [`Self::COPIES_KEY`], as the one reserved key a narrow metadata write may carry.
    ///
    /// Not a way round [`Self::new`] for a caller: nothing a person types reaches this, and
    /// the one write that sends it is the copy recording where an item landed.
    #[must_use]
    // llmlint: ignore[invalid_states_unrepresentable] A key type of its own would change the
    // signature of the three narrow metadata writes on `TaskSource`, which every plugin
    // implements and this crate keeps still (AGENTS.md); the reserved key reaches that seam
    // only through this one named constructor, while `new`, deserialization, the command line
    // and both SDKs refuse it, and the stdio boundary admits it only with a value that is links.
    pub fn copies() -> Self {
        Self(Self::COPIES_KEY.to_owned())
    }

    /// Whether this is [`Self::COPIES_KEY`].
    #[must_use]
    pub fn is_copies(&self) -> bool {
        self.0 == Self::COPIES_KEY
    }

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
