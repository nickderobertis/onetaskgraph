//! `sources migrate` on a Linear source: every issue, project and document still holding its
//! metadata in a spelling this source no longer writes, rewritten into the collapsed block.
//!
//! The source's items are listed through the same reads its queries send — the issues and
//! projects its team and project scope reach, and the documents its document listing reaches —
//! each page asked for with the stored form beside what is read of it, so the spelling an item is
//! held in is read off Linear's own bytes. An item in a superseded spelling (see
//! [`StoredForm`](crate::slot::StoredForm)) is sent one update carrying its long form as a read
//! gives it back, which the transport writes as every other write is written: the content as it
//! was, then the `Fields` section its metadata shows and the block. So what a read gives back of it
//! afterwards — its content and its metadata — is what it gave before. An item already current, or
//! holding no metadata, is counted and sent nothing; a second sweep therefore rewrites nothing.

use onetaskgraph_plugin_api::{
    MetadataRecord, MigratedItem, MigrationFailure, NativeId, SourceError, StoredFormMigration,
};
use serde_json::{Value, json};

use super::graphql::{DOCUMENTS, ISSUES, PROJECTS};
use super::{Keep, LinearSource, MAX_PAGE_SIZE, Under400, backend_id, page_next, slot};

/// The form an item whose comment slot never closes is reported as having been left in.
const UNREADABLE: &str = "unreadable-comment-slot";

impl LinearSource {
    /// See [`TaskSource::migrate_stored_forms`](onetaskgraph_plugin_api::TaskSource::migrate_stored_forms).
    pub(crate) async fn migrate(&self, dry_run: bool) -> Result<StoredFormMigration, SourceError> {
        let mut migration = StoredFormMigration::default();
        for record in [
            MetadataRecord::Task,
            MetadataRecord::Project,
            MetadataRecord::Document,
        ] {
            let (query, root, member, first, filter) = match record {
                MetadataRecord::Task => (
                    ISSUES,
                    "issues",
                    "description",
                    MAX_PAGE_SIZE,
                    Self::narrowed(self.issue_scope()),
                ),
                MetadataRecord::Project => (
                    PROJECTS,
                    "projects",
                    "content",
                    self.projects_page(),
                    Self::narrowed(self.project_scope()),
                ),
                MetadataRecord::Document => (
                    DOCUMENTS,
                    "documents",
                    "content",
                    MAX_PAGE_SIZE,
                    // The documents `query_documents` lists: the scope's own, or every one.
                    self.project.as_ref().map_or_else(
                        || json!({}),
                        |scope| json!({"project": {"id": {"eq": scope.0}}}),
                    ),
                ),
            };
            let mut cursor: Option<String> = None;
            loop {
                let (read, stored) = self
                    .answer_keeping(
                        query,
                        json!({"first": first, "after": cursor, "filter": filter}),
                        Under400::Unavailable,
                        Keep::Stored,
                    )
                    .await?
                    .map_err(|refusal| refusal.into_error())?;
                let stored = stored.unwrap_or(Value::Null);
                let connection = read.get(root).ok_or_else(|| SourceError::Malformed {
                    message: format!("missing {root}"),
                })?;
                let nodes = connection
                    .get("nodes")
                    .and_then(Value::as_array)
                    .ok_or_else(|| SourceError::Malformed {
                        message: format!("missing array {root}.nodes"),
                    })?;
                let stored_nodes = stored
                    .get(root)
                    .and_then(|connection| connection.get("nodes"))
                    .and_then(Value::as_array)
                    .filter(|stored| stored.len() == nodes.len())
                    .ok_or_else(|| SourceError::Malformed {
                        message: format!(
                            "the stored form of the {root} listing does not hold one node per \
                             node read"
                        ),
                    })?;
                for (at, node) in nodes.iter().enumerate() {
                    let id = NativeId(backend_id(node, "id")?.to_owned());
                    if record == MetadataRecord::Document
                        && !self.in_scope(
                            node.pointer("/project/id")
                                .and_then(Value::as_str)
                                .map(|project| NativeId(project.to_owned()))
                                .as_ref(),
                        )
                    {
                        continue;
                    }
                    let raw = match stored_nodes[at].get(member) {
                        None | Some(Value::Null) => None,
                        Some(Value::String(raw)) => Some(raw.as_str()),
                        Some(_) => {
                            return Err(SourceError::Malformed {
                                message: format!("{root}.nodes[{at}].{member} is not a string"),
                            });
                        }
                    };
                    let tail = match raw.map(slot::tail).transpose() {
                        Ok(tail) => tail.flatten(),
                        // A comment slot that opens and never closes: no read gives its metadata
                        // back, so no rewrite can carry it, and the item is named for a person.
                        Err(error) => {
                            migration.failed.push(MigrationFailure::new(
                                MigratedItem::new(record, id, UNREADABLE),
                                error.to_string(),
                            ));
                            continue;
                        }
                    };
                    // An item holding no metadata, or holding it in the block, holds the form
                    // this source writes now.
                    let Some(tail) = tail.filter(|tail| !tail.form.is_current()) else {
                        migration.current += 1;
                        continue;
                    };
                    let item = MigratedItem::new(record, id.clone(), tail.form.name());
                    if let Err(error) = tail.metadata(raw.unwrap_or_default()) {
                        migration
                            .failed
                            .push(MigrationFailure::new(item, error.to_string()));
                        continue;
                    }
                    if dry_run {
                        migration.rewritten.push(item);
                        continue;
                    }
                    let long_form = node.get(member).and_then(Value::as_str);
                    let written = match record {
                        MetadataRecord::Task => self.write_description_alone(&id, long_form).await,
                        MetadataRecord::Project => self.write_project_content(&id, long_form).await,
                        MetadataRecord::Document => {
                            self.write_document_content(&id, long_form).await
                        }
                    };
                    match written {
                        Ok(_) => migration.rewritten.push(item),
                        Err(error) => migration
                            .failed
                            .push(MigrationFailure::new(item, error.to_string())),
                    }
                }
                match page_next(connection)? {
                    Some(next) => cursor = Some(next.0),
                    None => break,
                }
            }
        }
        Ok(migration)
    }
}
