//! The record a copy leaves on each task, project and document it lands, and the test a later
//! copy reads it with to write nothing to an item nothing has changed at either end.
//!
//! Three reserved keys, each on the **destination** item and each the store's own:
//!
//! - [`MetadataKey::COPIED_VERSION_KEY`] — what the source item was when the copy composed it:
//!   its change token where its source gives one, otherwise [`composed`], a digest of the item
//!   the copy composed from it. It says whether the **source** changed since that copy;
//! - [`MetadataKey::COPIED_WITH_COMMENTS_KEY`] — `true` when that copy carried a task's
//!   comments, and absent after one that did not;
//! - [`MetadataKey::COPIED_DESTINATION_VERSION_KEY`] — a digest of the destination item as the
//!   copy left it, over every member a copy writes and, after a `--comments` copy, over its
//!   comments, the three keys themselves left out. It says whether anybody changed the
//!   **destination** item since, its title, status and edges included. It is taken over the
//!   content the destination reported storing, so it is what a read of the item digests to
//!   even where the destination rewrote the Markdown it was sent.
//!
//! A later copy writes nothing to an item only when the source still matches the first **and**
//! the destination digests now to the second — so an edit made at either end since, a copied
//! comment edited or deleted at the destination included, is never served from the record. The
//! destination's digest is taken over the reads a copy already makes of the destination to
//! find its counterpart, so the test costs no request at either end. Each fingerprint has one
//! job: a rendering's `body_digest` lets any reader verify the item is its template's rendering
//! unedited, the first key says whether the source changed, the third whether the destination
//! did.

use std::collections::BTreeMap;

use onetaskgraph_plugin_api::{
    DependencyEdge, Document, MetadataKey, Project, Repository, SourceName, Status, Task,
};
use serde_json::Value;

use crate::template::{Sha256Digest, answers_digest};

use super::super::delivery::targets;
use super::Item;

/// The three keys, which a copy writes and nothing else does.
pub(super) const KEYS: [&str; 3] = [
    MetadataKey::COPIED_VERSION_KEY,
    MetadataKey::COPIED_WITH_COMMENTS_KEY,
    MetadataKey::COPIED_DESTINATION_VERSION_KEY,
];

/// What a copy records its source item as having been, at [`MetadataKey::COPIED_VERSION_KEY`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum SourceVersion {
    /// The change token the item's source gave it.
    Token(String),
    /// The digest of what the copy composed of an item whose source gives no token — see
    /// [`composed`].
    Composed(Sha256Digest),
}

impl std::fmt::Display for SourceVersion {
    /// The string the key holds: the token as its source spells it, or the digest.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Token(token) => f.write_str(token),
            Self::Composed(digest) => write!(f, "{digest}"),
        }
    }
}

/// What one copy records on one destination item.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Record {
    /// What the source item was: its token, or the digest of what the copy composed of it.
    pub(super) version: SourceVersion,
    /// Whether the copy carried the task's comments.
    pub(super) with_comments: bool,
    /// The destination item's digest as the copy left it.
    pub(super) digest: Sha256Digest,
}

impl Record {
    /// `metadata` with this record in it, in place of any it held.
    pub(super) fn stamp(&self, metadata: &mut BTreeMap<String, Value>) {
        for key in KEYS {
            metadata.remove(key);
        }
        metadata.insert(
            MetadataKey::COPIED_VERSION_KEY.to_owned(),
            Value::String(self.version.to_string()),
        );
        if self.with_comments {
            metadata.insert(
                MetadataKey::COPIED_WITH_COMMENTS_KEY.to_owned(),
                Value::Bool(true),
            );
        }
        metadata.insert(
            MetadataKey::COPIED_DESTINATION_VERSION_KEY.to_owned(),
            Value::String(self.digest.to_string()),
        );
    }

    /// Whether `metadata` already records exactly this.
    pub(super) fn held_by(&self, metadata: &BTreeMap<String, Value>) -> bool {
        let mut stamped = BTreeMap::new();
        self.stamp(&mut stamped);
        KEYS.iter()
            .all(|key| metadata.get(*key) == stamped.get(*key))
    }
}

/// `metadata` without the three keys.
pub(super) fn without_record(metadata: &BTreeMap<String, Value>) -> BTreeMap<String, Value> {
    let mut kept = metadata.clone();
    for key in KEYS {
        kept.remove(key);
    }
    kept
}

/// Caller metadata as a copy writes it: repository names are already digested from the
/// item's `repositories` field. Hosted sources also expose that field's transport slot in
/// metadata, while a folder keeps it in front matter; that duplication is not an edit.
fn digest_metadata(metadata: &BTreeMap<String, Value>) -> BTreeMap<String, Value> {
    let mut kept = without_record(metadata);
    kept.remove(Repository::METADATA_KEY);
    kept
}

/// Whether `metadata` records that the copy it describes carried the task's comments.
pub(super) fn carried_comments(metadata: &BTreeMap<String, Value>) -> bool {
    metadata.get(MetadataKey::COPIED_WITH_COMMENTS_KEY) == Some(&Value::Bool(true))
}

/// The digest of a destination task, its forward edges there and — after a `--comments` copy —
/// its comments as [`super::comments::digest_entries`] gives them.
///
/// Over exactly what a copy writes and compares: every member [`super::same`] holds a task to
/// but `delivered_by` and the dependents a task of a private source records, which the store
/// keeps in step after the copy rather than the copy writing them, and none of the
/// destination's own — its id, url, location and times.
pub(super) fn digest(
    task: &Task,
    edges: &[DependencyEdge],
    comments: Option<Vec<Value>>,
    destination: &SourceName,
) -> Sha256Digest {
    let as_json =
        |value: serde_json::Result<Value>| value.expect("a contract type renders as JSON");
    let mut ends: Vec<Value> = edges
        .iter()
        .map(|edge| {
            Value::Array(vec![
                Value::String(edge.to.to_string()),
                as_json(serde_json::to_value(edge.to.kind)),
                as_json(serde_json::to_value(edge.kind)),
            ])
        })
        .collect();
    ends.sort_by_key(ToString::to_string);
    let mut fields: BTreeMap<String, Value> = BTreeMap::from([
        ("title".to_owned(), Value::String(task.title.clone())),
        (
            "content".to_owned(),
            as_json(serde_json::to_value(&task.content)),
        ),
        (
            "status".to_owned(),
            as_json(serde_json::to_value(&task.status)),
        ),
        (
            "priority".to_owned(),
            as_json(serde_json::to_value(task.priority)),
        ),
        (
            "labels".to_owned(),
            as_json(serde_json::to_value(&task.labels)),
        ),
        (
            "project".to_owned(),
            as_json(serde_json::to_value(&task.project)),
        ),
        (
            "metadata".to_owned(),
            Value::Object(
                digest_metadata(&task.metadata)
                    .into_iter()
                    .filter(|(key, _)| key != super::super::dependents::DEPENDED_ON_BY_KEY)
                    .collect(),
            ),
        ),
        (
            "repositories".to_owned(),
            as_json(serde_json::to_value(&task.repositories)),
        ),
        (
            "classification".to_owned(),
            as_json(serde_json::to_value(task.classification)),
        ),
        (
            "delivers".to_owned(),
            Value::Array(
                targets(&task.delivers, destination)
                    .iter()
                    .map(|entry| Value::String(entry.to_string()))
                    .collect(),
            ),
        ),
        ("edges".to_owned(), Value::Array(ends)),
    ]);
    if let Some(comments) = comments {
        fields.insert(
            "comments".to_owned(),
            Value::Array(super::comments::ordered(comments)),
        );
    }
    Sha256Digest::parse(answers_digest(&fields)).expect("a canonical digest spells sha256")
}

/// Whether a copy may skip a task whose source gives `version` and whose destination
/// counterpart holds `metadata` and digests, as read now, to `now` — the comments in it when
/// the copy carries them.
pub(super) fn skips(
    version: Option<&str>,
    with_comments: bool,
    metadata: &BTreeMap<String, Value>,
    now: &Sha256Digest,
) -> bool {
    let Some(version) = version else {
        return false;
    };
    metadata
        .get(MetadataKey::COPIED_VERSION_KEY)
        .and_then(Value::as_str)
        == Some(version)
        && (!with_comments || carried_comments(metadata))
        && metadata
            .get(MetadataKey::COPIED_DESTINATION_VERSION_KEY)
            .and_then(Value::as_str)
            == Some(now.as_str())
}

/// The digest of a destination item as a read returns it, on the terms of [`digest`] for a task:
/// a project over its fields and forward edges, a document over its fields — every member a
/// copy writes, none of the destination's own, and the three keys left out.
pub(super) fn digest_of(
    item: &Item,
    edges: &[DependencyEdge],
    comments: Option<Vec<Value>>,
    destination: &SourceName,
) -> Sha256Digest {
    match item {
        Item::Task(task) => digest(task, edges, comments, destination),
        Item::Project(project) => project_digest(project, edges),
        Item::Document(document) => document_digest(document),
    }
}

/// `value` as JSON: a contract type's serialization has no failing case, so a failure here is a
/// defect in the type rather than in the item.
fn as_json(value: serde_json::Result<Value>) -> Value {
    value.expect("a contract type renders as JSON")
}

fn project_digest(project: &Project, edges: &[DependencyEdge]) -> Sha256Digest {
    let mut ends: Vec<Value> = edges
        .iter()
        .map(|edge| {
            Value::Array(vec![
                Value::String(edge.to.to_string()),
                as_json(serde_json::to_value(edge.to.kind)),
                as_json(serde_json::to_value(edge.kind)),
            ])
        })
        .collect();
    ends.sort_by_key(ToString::to_string);
    let fields: BTreeMap<String, Value> = BTreeMap::from([
        ("title".to_owned(), Value::String(project.title.clone())),
        (
            "content".to_owned(),
            as_json(serde_json::to_value(&project.content)),
        ),
        (
            "status".to_owned(),
            as_json(serde_json::to_value(&project.status)),
        ),
        (
            "labels".to_owned(),
            as_json(serde_json::to_value(&project.labels)),
        ),
        (
            "metadata".to_owned(),
            Value::Object(digest_metadata(&project.metadata).into_iter().collect()),
        ),
        (
            "repositories".to_owned(),
            as_json(serde_json::to_value(&project.repositories)),
        ),
        (
            "classification".to_owned(),
            as_json(serde_json::to_value(project.classification)),
        ),
        ("edges".to_owned(), Value::Array(ends)),
    ]);
    Sha256Digest::parse(answers_digest(&fields)).expect("a canonical digest spells sha256")
}

fn document_digest(document: &Document) -> Sha256Digest {
    let fields: BTreeMap<String, Value> = BTreeMap::from([
        ("title".to_owned(), Value::String(document.title.clone())),
        (
            "content".to_owned(),
            as_json(serde_json::to_value(&document.content)),
        ),
        (
            "labels".to_owned(),
            as_json(serde_json::to_value(&document.labels)),
        ),
        (
            "project".to_owned(),
            as_json(serde_json::to_value(&document.project)),
        ),
        (
            "metadata".to_owned(),
            Value::Object(digest_metadata(&document.metadata).into_iter().collect()),
        ),
        (
            "repositories".to_owned(),
            as_json(serde_json::to_value(&document.repositories)),
        ),
        (
            "classification".to_owned(),
            as_json(serde_json::to_value(document.classification)),
        ),
    ]);
    Sha256Digest::parse(answers_digest(&fields)).expect("a canonical digest spells sha256")
}

/// What [`MetadataKey::COPIED_VERSION_KEY`] records of a source item that gives no change token:
/// the digest of the item a copy composed from it, its content as `content` gives it — after
/// reference and image rewriting — and its title, status, priority, labels, repositories,
/// classification and metadata as the source holds them, without the three keys and without
/// the copy's own links there, `onetaskgraph.copies` and `onetaskgraph.copy_locations`, which a
/// copy writes onto its source once it has landed.
pub(super) fn composed(
    item: &Item,
    content: Option<&str>,
    source_status: Option<&Status>,
) -> Sha256Digest {
    let (title, metadata, mut fields) = match item {
        Item::Task(task) => (
            &task.title,
            &task.metadata,
            BTreeMap::from([
                (
                    "status".to_owned(),
                    as_json(serde_json::to_value(source_status.unwrap_or(&task.status))),
                ),
                (
                    "priority".to_owned(),
                    as_json(serde_json::to_value(task.priority)),
                ),
                (
                    "labels".to_owned(),
                    as_json(serde_json::to_value(&task.labels)),
                ),
                (
                    "repositories".to_owned(),
                    as_json(serde_json::to_value(&task.repositories)),
                ),
                (
                    "classification".to_owned(),
                    as_json(serde_json::to_value(task.classification)),
                ),
            ]),
        ),
        Item::Project(project) => (
            &project.title,
            &project.metadata,
            BTreeMap::from([
                (
                    "status".to_owned(),
                    as_json(serde_json::to_value(
                        source_status.unwrap_or(&project.status),
                    )),
                ),
                (
                    "labels".to_owned(),
                    as_json(serde_json::to_value(&project.labels)),
                ),
                (
                    "repositories".to_owned(),
                    as_json(serde_json::to_value(&project.repositories)),
                ),
                (
                    "classification".to_owned(),
                    as_json(serde_json::to_value(project.classification)),
                ),
            ]),
        ),
        Item::Document(document) => (
            &document.title,
            &document.metadata,
            BTreeMap::from([
                (
                    "labels".to_owned(),
                    as_json(serde_json::to_value(&document.labels)),
                ),
                (
                    "repositories".to_owned(),
                    as_json(serde_json::to_value(&document.repositories)),
                ),
                (
                    "classification".to_owned(),
                    as_json(serde_json::to_value(document.classification)),
                ),
            ]),
        ),
    };
    let mut kept = without_record(metadata);
    kept.remove(MetadataKey::COPIES_KEY);
    kept.remove(MetadataKey::COPY_LOCATIONS_KEY);
    fields.insert("title".to_owned(), Value::String(title.clone()));
    fields.insert("content".to_owned(), as_json(serde_json::to_value(content)));
    fields.insert(
        "metadata".to_owned(),
        Value::Object(kept.into_iter().collect()),
    );
    Sha256Digest::parse(answers_digest(&fields)).expect("a canonical digest spells sha256")
}

/// Whether `metadata`, a destination item's, records `version` as the source it was copied from
/// and `now` as its own digest — the source unchanged and the destination untouched since.
pub(super) fn holds(
    metadata: &BTreeMap<String, Value>,
    version: &str,
    with_comments: bool,
    now: &Sha256Digest,
) -> bool {
    skips(Some(version), with_comments, metadata, now)
}
