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
                    .and_then(Value::as_array);
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
                    let raw = stored_nodes
                        .and_then(|nodes| nodes.get(at))
                        .and_then(|node| node.get(member))
                        .and_then(Value::as_str);
                    let form = match raw.map(slot::tail) {
                        Some(Ok(Some(tail))) if !tail.form.is_current() => tail.form,
                        // Current, holding no metadata, or a slot no read can parse — which a
                        // rewrite cannot carry either, and every read of the item names.
                        _ => {
                            migration.current += 1;
                            continue;
                        }
                    };
                    let item = MigratedItem::new(record, id.clone(), form.name());
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
