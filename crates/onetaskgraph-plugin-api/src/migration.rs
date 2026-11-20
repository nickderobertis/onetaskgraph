//! What one sweep of a source's stored forms rewrote: the answer of
//! [`TaskSource::migrate_stored_forms`](crate::TaskSource::migrate_stored_forms).
//!
//! A source that once wrote an item's long form in a spelling it no longer writes — a metadata
//! slot its tracker shows as text, say — keeps reading that spelling, so nothing is lost by
//! leaving it; the sweep is what rewrites the backlog once rather than item by item as each is
//! next written. Which forms are superseded, and how an item is rewritten, is the plugin's alone;
//! these types only say what it did.

use crate::{MetadataRecord, NativeId};

/// One item a sweep rewrote, or under a dry run would rewrite.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct MigratedItem {
    /// Whether the item is a task, a project or a document.
    pub kind: MetadataRecord,
    /// The item's id in its source.
    pub id: NativeId,
    /// The superseded form the item was held in, as its plugin names it — `html-comment-slot`,
    /// say.
    pub from: String,
}

impl MigratedItem {
    /// The item of `kind` with `id`, held in the superseded form named `from`.
    #[must_use]
    pub fn new(kind: MetadataRecord, id: NativeId, from: impl Into<String>) -> Self {
        Self {
            kind,
            id,
            from: from.into(),
        }
    }
}

/// One item a sweep found in a superseded form and could not rewrite.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct MigrationFailure {
    /// The item, and the form it is still held in.
    pub item: MigratedItem,
    /// Why it was refused or not written, as the source said it.
    pub reason: String,
}

impl MigrationFailure {
    /// `item`, left in its superseded form for `reason`.
    #[must_use]
    pub fn new(item: MigratedItem, reason: impl Into<String>) -> Self {
        Self {
            item,
            reason: reason.into(),
        }
    }
}

/// What one sweep of a source's stored forms did.
///
/// The default — nothing rewritten, nothing current, nothing failed — is the answer of a
/// source that writes every item in the one form it has always written.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct StoredFormMigration {
    /// Each item rewritten into the current form, or under a dry run each item that would be.
    pub rewritten: Vec<MigratedItem>,
    /// How many of the items examined already held the current form — one holding nothing a
    /// stored form could spell included — and were sent nothing.
    pub current: u64,
    /// Each item held in a superseded form that was refused or not written.
    pub failed: Vec<MigrationFailure>,
}
