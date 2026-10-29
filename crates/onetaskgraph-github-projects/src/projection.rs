//! What a producer's projection carries from one process to the next on this board: the
//! board's field and option ids and the repositories' node ids a create reads, and the ids of
//! an item this process wrote — never its title or its content.
//!
//! Every human task projection is its own process, so without these each create reads the
//! board's fields and its repository again and each terminal write reads the issue it wrote.
//! The producer's durable intent keeps them between processes (`docs/task-projection.md`);
//! what a write still has to read — the project issue a task is filed under, which every
//! sub-issue it gains moves — is read as it always is.

use std::collections::BTreeMap;

use onetaskgraph_plugin_api::{NativeId, Task};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    BoardFields, BoardKind, ContentKind, GitHubProjectsSource, ItemKind, RepositoryTarget,
    Resolved, compose_body,
};

/// The version of both contexts, so one an earlier or later build wrote is declined rather
/// than misread.
const PROJECTION_CONTEXT_VERSION: u64 = 1;

/// The ids a create in this board reads before it writes.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Placement {
    version: u64,
    fields: BoardFields,
    /// Each repository's node id, by `owner/name`.
    repositories: BTreeMap<String, String>,
}

/// One issue this source wrote, with its title and both spellings of its body left out.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Written {
    version: u64,
    item: Resolved,
}

impl GitHubProjectsSource {
    /// The board's fields and every repository id this process read, when it read the fields.
    pub(super) fn placement_held(&self) -> Option<Value> {
        let fields = self.fields_cache().ok()?.clone()?;
        let repositories = self
            .repository_cache()
            .ok()?
            .iter()
            .map(|(target, id)| (target.slug(), id.clone()))
            .collect();
        serde_json::to_value(Placement {
            version: PROJECTION_CONTEXT_VERSION,
            fields,
            repositories,
        })
        .ok()
    }

    /// Take a placement an earlier process held, where this process has read none of it.
    pub(super) fn prime_placement_held(&self, context: &Value) -> bool {
        let Ok(placement) = serde_json::from_value::<Placement>(context.clone()) else {
            return false;
        };
        if placement.version != PROJECTION_CONTEXT_VERSION {
            return false;
        }
        let Ok(repositories) = placement
            .repositories
            .iter()
            .map(|(slug, id)| RepositoryTarget::parse(slug).map(|target| (target, id.clone())))
            .collect::<Result<Vec<_>, _>>()
        else {
            return false;
        };
        let (Ok(mut fields), Ok(mut held)) = (self.fields_cache(), self.repository_cache()) else {
            return false;
        };
        if fields.is_none() {
            *fields = Some(placement.fields);
        }
        for (target, id) in repositories {
            held.entry(target).or_insert(id);
        }
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

    /// Take `context` with `task`'s title and content as the record of issue `id`, its body
    /// composed exactly as the write that created it composed it.
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
        true
    }
}
