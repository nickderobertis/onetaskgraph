//! The record a copy leaves on each task it lands, and the test a later copy reads it with to
//! skip a task nothing has changed at either end.
//!
//! Three reserved keys, each on the **destination** task and each the store's own:
//!
//! - [`MetadataKey::COPIED_VERSION_KEY`] — the source task's change token as the copy read it,
//!   absent when the source gave none;
//! - [`MetadataKey::COPIED_WITH_COMMENTS_KEY`] — `true` when that copy carried the task's
//!   comments, and absent after one that did not;
//! - [`MetadataKey::COPIED_DESTINATION_VERSION_KEY`] — a digest of the destination task as the
//!   copy left it, over every member a copy writes and, after a `--comments` copy, over its
//!   comments, the three keys themselves left out.
//!
//! A later copy skips a task only when the source's token now is the one recorded **and** the
//! destination digests now to the one recorded — so an edit made at either end since, a
//! copied comment edited or deleted at the destination included, is never served from a skip.
//! The destination's digest is taken over the reads a copy already makes of the destination to
//! find its counterpart, so the test costs no request at either end.

use std::collections::BTreeMap;

use onetaskgraph_plugin_api::{DependencyEdge, MetadataKey, SourceName, Task};
use serde_json::Value;

use crate::template::{Sha256Digest, answers_digest};

use super::super::delivery::targets;

/// The three keys, which a copy writes and nothing else does.
pub(super) const KEYS: [&str; 3] = [
    MetadataKey::COPIED_VERSION_KEY,
    MetadataKey::COPIED_WITH_COMMENTS_KEY,
    MetadataKey::COPIED_DESTINATION_VERSION_KEY,
];

/// What one copy records on one destination task.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Record {
    /// The source task's token as the copy read it.
    pub(super) version: Option<String>,
    /// Whether the copy carried the task's comments.
    pub(super) with_comments: bool,
    /// The destination task's digest as the copy left it.
    pub(super) digest: Sha256Digest,
}

impl Record {
    /// `metadata` with this record in it, in place of any it held.
    pub(super) fn stamp(&self, metadata: &mut BTreeMap<String, Value>) {
        for key in KEYS {
            metadata.remove(key);
        }
        if let Some(version) = &self.version {
            metadata.insert(
                MetadataKey::COPIED_VERSION_KEY.to_owned(),
                Value::String(version.clone()),
            );
        }
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
                Value::String(edge.to.id().to_owned()),
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
                without_record(&task.metadata)
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
