//! `sources migrate`: rewrite every item of one source still held in a stored form its plugin no
//! longer writes.
//!
//! Which forms are superseded, and how an item is rewritten, is the plugin's
//! ([`TaskSource::migrate_stored_forms`](onetaskgraph_plugin_api::TaskSource::migrate_stored_forms));
//! the engine owns the command and its report, and qualifies every id the plugin names. A source
//! whose plugin has only ever written one form — every plugin served over the stdio protocol
//! among them — answers with nothing to migrate, and is sent nothing.

use onetaskgraph_plugin_api::{MetadataRecord, MigratedItem, SourceName};
use schemars::JsonSchema;
use serde::Serialize;

use super::{Engine, EngineError};
use crate::GlobalId;

/// Which kind of record a migrated item is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum MigratedKind {
    /// A task.
    Task,
    /// A project.
    Project,
    /// A document.
    Document,
}

impl From<MetadataRecord> for MigratedKind {
    fn from(record: MetadataRecord) -> Self {
        match record {
            MetadataRecord::Task => Self::Task,
            MetadataRecord::Project => Self::Project,
            MetadataRecord::Document => Self::Document,
        }
    }
}

/// One item a migration rewrote, or under `--dry-run` would rewrite.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct MigratedRecord {
    /// What kind of record it is.
    pub kind: MigratedKind,
    /// Its qualified id.
    pub id: GlobalId,
    /// The superseded stored form it was held in, as its plugin names it.
    pub from: String,
}

/// One item a migration found in a superseded form and could not rewrite.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct MigrationRefused {
    /// What kind of record it is.
    pub kind: MigratedKind,
    /// Its qualified id.
    pub id: GlobalId,
    /// The superseded stored form it is still held in.
    pub from: String,
    /// Why it was not rewritten, as the source said it.
    pub reason: String,
}

/// What one `sources migrate` did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct StoredFormsMigrated {
    /// The source migrated.
    pub source: SourceName,
    /// Whether nothing was written: every entry of `rewritten` is one that would be.
    pub dry_run: bool,
    /// Each item rewritten into the form its source writes now, in the order the source listed
    /// it.
    pub rewritten: Vec<MigratedRecord>,
    /// How many of the items examined already held the current form, and were sent nothing.
    pub current: u64,
    /// Each item left in a superseded form, with why.
    pub failed: Vec<MigrationRefused>,
}

impl StoredFormsMigrated {
    /// The exit status the command line answers with: `1` when any item was left unrewritten,
    /// and `0` otherwise.
    #[must_use]
    pub fn exit_code(&self) -> u8 {
        u8::from(!self.failed.is_empty())
    }
}

impl Engine {
    /// Rewrite every item of `source` held in a stored form its plugin no longer writes, unless
    /// `dry_run`, which sends reads only and reports what would be rewritten.
    ///
    /// # Errors
    ///
    /// [`EngineError::UnknownSource`] for a source nothing configures,
    /// [`EngineError::SourceUnavailable`] for one that could not be built, and
    /// [`EngineError::SourceRefused`] when the source could not list its items.
    pub async fn migrate_stored_forms(
        &self,
        source: &SourceName,
        dry_run: bool,
    ) -> Result<StoredFormsMigrated, EngineError> {
        let resolved = self.built(source)?;
        let migrated = resolved
            .source()
            .migrate_stored_forms(dry_run)
            .await
            .map_err(|error| EngineError::SourceRefused {
                name: source.to_string(),
                error,
            })?;
        let qualified = |item: &MigratedItem| GlobalId::new(source.clone(), item.id.clone());
        Ok(StoredFormsMigrated {
            source: source.clone(),
            dry_run,
            rewritten: migrated
                .rewritten
                .iter()
                .map(|item| MigratedRecord {
                    kind: item.kind.into(),
                    id: qualified(item),
                    from: item.from.clone(),
                })
                .collect(),
            current: migrated.current,
            failed: migrated
                .failed
                .iter()
                .map(|failure| MigrationRefused {
                    kind: failure.item.kind.into(),
                    id: qualified(&failure.item),
                    from: failure.item.from.clone(),
                    reason: failure.reason.clone(),
                })
                .collect(),
        })
    }
}

/// The migration domain's published response roots.
pub(crate) fn schemas() -> [(&'static str, schemars::Schema); 1] {
    [(
        "StoredFormsMigrated",
        schemars::schema_for!(StoredFormsMigrated),
    )]
}
