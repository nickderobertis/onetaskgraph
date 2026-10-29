//! What a producer's projection carries from one process to the next on this board: the
//! placement a create reads before it writes, held behind a setup-context token, and the ids of
//! an item this process wrote — never its title or its content.
//!
//! Every human task projection is its own process, so without these each create reads the
//! board's fields and its repository again and each terminal write reads the issue it wrote.
//! The producer's durable intent keeps them between processes (`docs/task-projection.md`).
//!
//! **A kept placement is reused only while its setup-context token still holds.** The token is
//! the digest of everything the placement depends on — the board, every field definition and
//! option the board defines, the repository the issue is created in, the project issue it is
//! filed under, and whether this source is declared private — and it is computed afresh from
//! the read of that project issue every create already makes, which carries the board's whole
//! field list. A kept placement whose token differs, or one this process cannot compute, is not
//! taken: the create reads its context as it always did and keeps the new one. So the steady
//! state reads nothing the create did not already read, and a changed option, field, repository,
//! project or visibility is read afresh rather than reused.

use std::sync::{Mutex, MutexGuard};

use onetaskgraph_plugin_api::{NativeId, Task, asset_sha256};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{
    BoardFields, BoardKind, ContentKind, GitHubProjectsSource, ItemKind, RepositoryTarget,
    Resolved, compose_body,
};

/// The version of both contexts, so one an earlier or later build wrote is declined rather
/// than misread.
const PROJECTION_CONTEXT_VERSION: u64 = 2;

/// The placement a create in this board reads before it writes, and the token it holds under.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Placement {
    version: u64,
    /// The setup-context token the placement was kept under.
    token: String,
    /// The repository the issue is created in, `owner/name`.
    repository: String,
    /// That repository's node id.
    repository_id: String,
}

/// One issue this source wrote, with its title and both spellings of its body left out.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Written {
    version: u64,
    item: Resolved,
}

/// What this source holds for projections, for the length of one process.
#[derive(Default)]
pub(super) struct Held {
    /// A placement an earlier process kept, not yet held to this process's token.
    kept: Mutex<Option<Placement>>,
    /// The repository and project issue the last create of this process was placed by.
    placed: Mutex<Option<(RepositoryTarget, Option<NativeId>)>>,
    /// Issues primed from a producer's written context whose body nothing in this process has
    /// read: an update changing one reads the body first.
    unread: Mutex<std::collections::BTreeSet<NativeId>>,
}

/// A lock an earlier panic poisoned is read as it stands: what these hold is an optimisation,
/// and a create that cannot use it reads instead.
fn held<T>(lock: &Mutex<T>) -> MutexGuard<'_, T> {
    lock.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The setup-context token of `board`'s fields, for an issue created in `repository` under
/// `parent` by a source `private` or not: the digest of each field's id, name, kind and options,
/// in id order, beside the rest.
fn setup_token(
    board: &BoardFields,
    repository: &RepositoryTarget,
    parent: Option<&NativeId>,
    private: bool,
) -> Option<String> {
    let mut fields: Vec<Value> = board
        .fields
        .get("nodes")?
        .as_array()?
        .iter()
        .map(|field| {
            let options = field
                .get("options")
                .and_then(Value::as_array)
                .map(|options| {
                    options
                        .iter()
                        .map(|option| json!([option.get("id"), option.get("name")]))
                        .collect::<Vec<_>>()
                });
            json!({"id": field.get("id"), "name": field.get("name"),
                   "kind": field.get("__typename"), "type": field.get("dataType"),
                   "options": options})
        })
        .collect();
    fields.sort_by_key(|field| field["id"].to_string());
    let parts = json!({"version": PROJECTION_CONTEXT_VERSION, "board": board.id.as_str(),
        "fields": fields, "repository": repository.slug(), "parent": parent.map(|id| &id.0),
        "private": private});
    Some(asset_sha256(parts.to_string().as_bytes()))
}

impl GitHubProjectsSource {
    /// The placement this process's last create read, under the token it holds now.
    pub(super) fn placement_held(&self) -> Option<Value> {
        let (repository, parent) = held(&self.projection.placed).clone()?;
        let fields = self.fields_cache().ok()?.clone()?;
        let repository_id = self.repository_cache().ok()?.get(&repository)?.clone();
        let token = setup_token(
            &fields,
            &repository,
            parent.as_ref(),
            self.is_declared_private(),
        )?;
        serde_json::to_value(Placement {
            version: PROJECTION_CONTEXT_VERSION,
            token,
            repository: repository.slug(),
            repository_id,
        })
        .ok()
    }

    /// Hold a placement an earlier process kept, to be taken only once this process's own read
    /// of the project issue shows its token unchanged.
    pub(super) fn prime_placement_held(&self, context: &Value) -> bool {
        let Ok(placement) = serde_json::from_value::<Placement>(context.clone()) else {
            return false;
        };
        if placement.version != PROJECTION_CONTEXT_VERSION {
            return false;
        }
        *held(&self.projection.kept) = Some(placement);
        true
    }

    /// Record where a create is placed, and take the kept placement when the project issue this
    /// create already read shows its token unchanged — answering whether it did, so the create
    /// reads its context only when it did not.
    pub(super) fn placed_by_kept(
        &self,
        repository: &RepositoryTarget,
        parent: Option<&NativeId>,
    ) -> bool {
        *held(&self.projection.placed) = Some((repository.clone(), parent.cloned()));
        let Some(kept) = held(&self.projection.kept).clone() else {
            return false;
        };
        let Some(board) = parent.and_then(|parent| {
            self.resolved_cache()
                .ok()?
                .get(parent)
                .and_then(Resolved::carried_board)
        }) else {
            return false;
        };
        let current = setup_token(&board, repository, parent, self.is_declared_private());
        if kept.repository != repository.slug() || current.as_deref() != Some(kept.token.as_str()) {
            return false;
        }
        let (Ok(mut fields), Ok(mut repositories)) = (self.fields_cache(), self.repository_cache())
        else {
            return false;
        };
        *fields = Some(board);
        repositories.insert(repository.clone(), kept.repository_id);
        true
    }

    /// The record this process wrote of issue `id`, without its title or body.
    pub(super) fn written_held(&self, id: &NativeId) -> Option<Value> {
        let mut item = self.resolved_cache().ok()?.get(id).cloned()?;
        if item.kind != BoardKind::Work(ItemKind::Task)
            || item.content_kind != ContentKind::Issue
            || item.item_id.is_empty()
        {
            return None;
        }
        item.title = String::new();
        item.body = None;
        item.raw_body = None;
        serde_json::to_value(Written {
            version: PROJECTION_CONTEXT_VERSION,
            item,
        })
        .ok()
    }

    /// Take `context` with `task`'s title and content as the record of issue `id`.
    ///
    /// What it holds of the body is the caller's word, not a read, so it is marked unread: an
    /// update that changes the body reads the body first (`targeted_update`), and one that does
    /// not sends no body at all.
    pub(super) fn prime_written_held(&self, id: &NativeId, context: &Value, task: &Task) -> bool {
        let Ok(written) = serde_json::from_value::<Written>(context.clone()) else {
            return false;
        };
        let mut item = written.item;
        if written.version != PROJECTION_CONTEXT_VERSION || item.id != *id || task.id != *id {
            return false;
        }
        let Ok(raw_body) = compose_body(task.content.as_deref(), &item.slot) else {
            return false;
        };
        item.title.clone_from(&task.title);
        item.body.clone_from(&task.content);
        item.raw_body = raw_body;
        let Ok(mut cache) = self.resolved_cache() else {
            return false;
        };
        cache.insert(id.clone(), item);
        held(&self.projection.unread).insert(id.clone());
        true
    }

    /// Whether issue `id`'s record was primed from a written context and its body not read
    /// since — answered once, so the read that follows makes it read.
    pub(super) fn take_unread(&self, id: &NativeId) -> bool {
        held(&self.projection.unread).remove(id)
    }

    /// Select `option` in the board field `field` of the item `item` by the mutation that answers
    /// with the issue whole, and answer with the issue's body as GitHub holds it now: a write of
    /// the body composed against that answer is composed against the body as it stands.
    pub(super) async fn select_reading_body(
        &self,
        board: &str,
        item: &str,
        field: &str,
        option: &str,
    ) -> Result<String, onetaskgraph_plugin_api::SourceError> {
        let data = self
            .graphql_sent(
                super::graphql::UPDATE_FIELD_WHOLE,
                super::expected::whole_variables(json!({"input":{
                    "projectId":board,"itemId":item,"fieldId":field,
                    "value":{"singleSelectOptionId":option}
                },"readPriority":false,"priorityName":super::PRIORITY_FIELD})),
            )
            .await?;
        let returned = data
            .pointer("/updateProjectV2ItemFieldValue/projectV2Item")
            .ok_or_else(|| onetaskgraph_plugin_api::SourceError::Malformed {
                message: "GitHub field update returned no project item".into(),
            })?;
        if returned.get("id").and_then(Value::as_str) != Some(item) {
            return Err(onetaskgraph_plugin_api::SourceError::Malformed {
                message: "GitHub field update returned the wrong project item".into(),
            });
        }
        Ok(returned
            .pointer("/content/body")
            .and_then(Value::as_str)
            .ok_or_else(|| onetaskgraph_plugin_api::SourceError::Malformed {
                message: "GitHub field update returned no issue body".into(),
            })?
            .to_owned())
    }
}
