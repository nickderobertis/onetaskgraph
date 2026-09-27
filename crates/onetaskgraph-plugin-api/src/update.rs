//! A targeted update of one existing task: every field a caller names, and nothing else.
//!
//! The narrow writes each change one field in one call, and a copy rewrites the whole item. A
//! caller whose change touches a status and several metadata keys at once needs neither: this
//! names exactly the fields that changed, and a source applies them in as few writes as it
//! can, sending nothing for a field that already holds the requested value.
//!
//! The source's answer carries everything the engine reports about the call — the task read
//! back, the fields it wrote, and the `delivers` it held before — so the engine reads nothing
//! of its own around it. That is the difference between one read of an item and two on a
//! hosted backend, on every update.

use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    DependencyEdge, Direction, ItemWrite, MetadataKey, NativeId, PageRequest, Priority,
    SourceError, Status, Task, TaskRef, TaskSource,
};

/// A targeted update of one existing task. A member left `None` (or empty) leaves that field
/// exactly as the source holds it.
///
/// Labels, repositories and project membership are deliberately outside it — an existing
/// item is never moved, and no caller writes labels this way — and so is creation, which is a
/// copy's.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TaskUpdate {
    /// The task's title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// `Task::content` exactly as a read reports it; a metadata block the source keeps in the
    /// same backend field is kept byte for byte.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// Name and category. A source that keeps status names stores the name; one that maps by
    /// category (GitHub Projects) writes its mapped option, exactly as `write_task` does, and
    /// refuses a category it has disabled in the same words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<Status>,
    /// The task's priority; `none` clears it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<Priority>,
    // llmlint: ignore-block[invalid_states_unrepresentable] `metadata_set` and `metadata_remove` are two members because Contract 1 of the `graphql-writeback-quota` project record fixes them by name and type, and onepipeline builds against exactly this shape; the one state they can express that is not an update — one key in both — is refused by `consistent` before any source is read or written, in one wording every source and the engine share. A single map of key to set-or-remove would be the contract's owner's change to make, not this crate's.
    /// Keys added or replaced; every other key is kept.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[schemars(!skip_serializing_if)]
    pub metadata_set: BTreeMap<MetadataKey, Value>,
    /// Keys removed; a key the task does not hold is no write. Refused when it names a key
    /// `metadata_set` also names.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    #[schemars(!skip_serializing_if)]
    pub metadata_remove: BTreeSet<MetadataKey>,
    // llmlint: ignore-end[invalid_states_unrepresentable]
    /// Replaces the list; the engine keeps each ticket's `delivered_by` in step.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivers: Option<Vec<TaskRef>>,
    /// Replaces the task's forward dependency edges (`from` is this task); the source sends
    /// only the difference.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depends_on: Option<Vec<DependencyEdge>>,
}

impl TaskUpdate {
    /// Whether this update names no field at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.content.is_none()
            && self.status.is_none()
            && self.priority.is_none()
            && self.metadata_set.is_empty()
            && self.metadata_remove.is_empty()
            && self.delivers.is_none()
            && self.depends_on.is_none()
    }

    /// Whether this update names `field`, which is the only way a source may report it written.
    #[must_use]
    pub fn names(&self, field: UpdatedField) -> bool {
        match field {
            UpdatedField::Title => self.title.is_some(),
            UpdatedField::Content => self.content.is_some(),
            UpdatedField::Status => self.status.is_some(),
            UpdatedField::Priority => self.priority.is_some(),
            UpdatedField::Metadata => {
                !self.metadata_set.is_empty() || !self.metadata_remove.is_empty()
            }
            UpdatedField::Delivers => self.delivers.is_some(),
            UpdatedField::DependsOn => self.depends_on.is_some(),
        }
    }

    /// Every metadata key this update both sets and removes, in order.
    ///
    /// An update naming one is refused before anything is written: which of the two it meant
    /// is not something a source may guess.
    #[must_use]
    pub fn contradictions(&self) -> Vec<&MetadataKey> {
        self.metadata_remove
            .iter()
            .filter(|key| self.metadata_set.contains_key(*key))
            .collect()
    }

    /// The refusal of an update that both sets and removes a key, or `Ok` when it does not.
    ///
    /// Spelled once here, so every source and the engine refuse it in the same words.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::Refused`] naming every such key.
    pub fn consistent(&self) -> Result<(), SourceError> {
        let both = self.contradictions();
        if both.is_empty() {
            return Ok(());
        }
        let named: Vec<&str> = both.iter().map(|key| key.as_str()).collect();
        Err(SourceError::Refused {
            message: format!(
                "this update both sets and removes the metadata {} {}; next: name each key \
                 either to set or to remove, not both",
                if named.len() == 1 { "key" } else { "keys" },
                named.join(", ")
            ),
        })
    }

    /// `task` as it reads once this update is applied to it: every named field replaced, every
    /// named metadata key set or removed, and everything else exactly as it was.
    ///
    /// [`depends_on`](Self::depends_on) is not a member of a [`Task`], so it is not applied
    /// here.
    #[must_use]
    pub fn applied_to(&self, task: &Task) -> Task {
        let mut updated = task.clone();
        if let Some(title) = &self.title {
            updated.title.clone_from(title);
        }
        if let Some(content) = &self.content {
            updated.content = Some(content.clone());
        }
        if let Some(status) = &self.status {
            updated.status = status.clone();
        }
        if let Some(priority) = self.priority {
            updated.priority = priority;
        }
        for (key, value) in &self.metadata_set {
            updated
                .metadata
                .insert(key.as_str().to_owned(), value.clone());
        }
        for key in &self.metadata_remove {
            updated.metadata.remove(key.as_str());
        }
        if let Some(delivers) = &self.delivers {
            updated.delivers.clone_from(delivers);
        }
        updated
    }

    /// The fields this update names whose value differs between `before` and `after` — which,
    /// for a source that compares before it writes, is exactly what it wrote.
    ///
    /// [`UpdatedField::DependsOn`] is never among them: a [`Task`] carries no edges, so a
    /// source says whether it wrote those itself.
    #[must_use]
    pub fn changed(&self, before: &Task, after: &Task) -> BTreeSet<UpdatedField> {
        let mut written = BTreeSet::new();
        if self.title.is_some() && before.title != after.title {
            written.insert(UpdatedField::Title);
        }
        if self.content.is_some() && before.content != after.content {
            written.insert(UpdatedField::Content);
        }
        if self.status.is_some() && before.status != after.status {
            written.insert(UpdatedField::Status);
        }
        if self.priority.is_some() && before.priority != after.priority {
            written.insert(UpdatedField::Priority);
        }
        let named = self
            .metadata_set
            .keys()
            .chain(&self.metadata_remove)
            .map(MetadataKey::as_str);
        if named
            .into_iter()
            .any(|key| before.metadata.get(key) != after.metadata.get(key))
        {
            written.insert(UpdatedField::Metadata);
        }
        if self.delivers.is_some() && before.delivers != after.delivers {
            written.insert(UpdatedField::Delivers);
        }
        written
    }

    /// Apply this update to the task `id` of `source` by rewriting it: read it, apply the
    /// update, and — only when anything differs — [`write_task`](TaskSource::write_task) it
    /// with its `target` set, then read it back. `None` when `source` holds no such task.
    ///
    /// This is what [`TaskSource::update_task`] does for a source that does not override it,
    /// and it is public so a host forwarding the call to a peer that does not declare the
    /// targeted update can fall back to exactly it. It is correct and not minimal: a write it
    /// makes rewrites the whole item, and a source that can send less owes an override.
    ///
    /// The forward edges are read whenever they are needed — to compare against a named
    /// `depends_on`, or to carry unchanged through a rewrite a named `depends_on` does not
    /// replace — because a [`write_task`](TaskSource::write_task) replaces them.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::Refused`] for an update that both sets and removes a key,
    /// [`SourceError::Malformed`] when the task could not be read back after the write, and
    /// whatever the source's own read or write fails with.
    pub async fn rewrite<S: TaskSource + ?Sized>(
        &self,
        source: &S,
        id: &NativeId,
    ) -> Result<Option<TaskUpdateOutcome>, SourceError> {
        self.consistent()?;
        let Some(held) = source.get_task(id).await? else {
            return Ok(None);
        };
        let updated = self.applied_to(&held);
        let (depends_on, edges_differ) = match &self.depends_on {
            Some(wanted) => {
                let current = forward_edges(source, id).await?;
                (wanted.clone(), !same_edges(&current, wanted))
            }
            None if updated != held => (forward_edges(source, id).await?, false),
            None => (Vec::new(), false),
        };
        if updated == held && !edges_differ {
            return Ok(Some(TaskUpdateOutcome {
                delivers_before: held.delivers.clone(),
                task: held,
                written: BTreeSet::new(),
            }));
        }
        source
            .write_task(&ItemWrite {
                target: Some(id.clone()),
                item: updated,
                depends_on,
            })
            .await?;
        let task = source
            .get_task(id)
            .await?
            .ok_or_else(|| SourceError::Malformed {
                message: format!("task {id} was updated and then could not be read back"),
            })?;
        let mut written = self.changed(&held, &task);
        if edges_differ {
            written.insert(UpdatedField::DependsOn);
        }
        Ok(Some(TaskUpdateOutcome {
            task,
            written,
            delivers_before: held.delivers,
        }))
    }
}

/// One field of a task a [`TaskUpdate`] names, as the answer to it reports what was written.
///
/// kebab-case on the wire. [`Metadata`](Self::Metadata) stands for every metadata key the
/// update set or removed together; [`DependsOn`](Self::DependsOn) for the task's forward edges.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum UpdatedField {
    /// [`TaskUpdate::title`].
    Title,
    /// [`TaskUpdate::content`].
    Content,
    /// [`TaskUpdate::status`].
    Status,
    /// [`TaskUpdate::priority`].
    Priority,
    /// [`TaskUpdate::metadata_set`] and [`TaskUpdate::metadata_remove`].
    Metadata,
    /// [`TaskUpdate::delivers`].
    Delivers,
    /// [`TaskUpdate::depends_on`].
    DependsOn,
}

/// What a source answers a [`TaskUpdate`] of a task it holds with.
///
/// Everything the engine reports about the call is read off this, so the engine sends no read
/// of its own before or after it: `task` is what it answers with, `written` is what it says
/// was written, and `delivers_before` is what tells it which delivered tasks the update
/// dropped.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TaskUpdateOutcome {
    /// The task as this source reads it once the update landed.
    pub task: Task,
    /// The fields this source actually wrote — empty when nothing differed, and never a field
    /// the update did not name. Required on the wire: an answer that leaves it out has not said
    /// what it wrote, which is not the same as having written nothing.
    pub written: BTreeSet<UpdatedField>,
    /// The task's [`Task::delivers`] as this source held it before the update, which is the
    /// list a named `delivers` replaced. Required on the wire, for the reason `written` is: an
    /// answer leaving it out would hide every delivered task the update dropped.
    pub delivers_before: Vec<TaskRef>,
}

/// Every forward dependency edge `source` reports at `id`, walked to exhaustion.
async fn forward_edges<S: TaskSource + ?Sized>(
    source: &S,
    id: &NativeId,
) -> Result<Vec<DependencyEdge>, SourceError> {
    let limit = source.capabilities().max_page_size.max(1);
    let mut edges = Vec::new();
    let mut cursor = None;
    loop {
        let page = source
            .task_dependencies(id, Direction::DependsOn, &PageRequest { cursor, limit })
            .await?;
        edges.extend(page.items);
        match page.next {
            Some(next) => cursor = Some(next),
            None => return Ok(edges),
        }
    }
}

/// Whether two edge lists name the same far ends by the same kinds, whatever their order and
/// whatever each says its near end is.
fn same_edges(held: &[DependencyEdge], wanted: &[DependencyEdge]) -> bool {
    let far = |edges: &[DependencyEdge]| -> BTreeSet<(String, String, String)> {
        edges
            .iter()
            .map(|edge| {
                (
                    edge.to.id().to_owned(),
                    format!("{:?}", edge.to.kind),
                    format!("{:?}", edge.kind),
                )
            })
            .collect()
    };
    held.len() == wanted.len() && far(held) == far(wanted)
}
