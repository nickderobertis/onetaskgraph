//! Content-keyed attestations: a record that someone cleared a task's, a project's or a
//! document's authored content against a bar, which goes void the moment that content or the
//! bar it was cleared against moves.
//!
//! This module is the one declaration of the shapes — [`Attestation`], [`AttestationCheck`],
//! [`ProjectAttestationCheck`] and [`Attested`] — that the command line answers with and both
//! SDKs are generated from, and of the digest a record is keyed on. The verbs that record and
//! check one are the engine's ([`crate::Engine::attest`] and its siblings).
//!
//! # The record
//!
//! An item holds its attestations under the reserved metadata key
//! [`MetadataKey::ATTESTATIONS_KEY`] (`onetaskgraph.attestations`): a JSON object mapping an
//! attestation kind to the one record of that kind, so recording a kind again replaces it:
//!
//! ```json
//! {"design-approval": {"kind": "design-approval", "by": "<attester label>",
//!   "at": "<RFC 3339>", "content_digest": "sha256:<hex>", "bar": "<fingerprint>",
//!   "note": "<optional text>", "excludes": ["<caller key>", "…"]}}
//! ```
//!
//! `note` is left out when none was given, and `excludes` when it is empty.
//!
//! # The digest
//!
//! `content_digest` is computed by the store, never handed to it: `sha256:` and the lowercase
//! hex SHA-256 of the canonical JSON (keys sorted at every depth, no insignificant whitespace)
//! of the item's **authored content**:
//!
//! - `title` and `content` (the body, `null` when it has none);
//! - `answers_digest`: the `answers_digest` of the item's `onetaskgraph.template` entry — the
//!   hash of the answers it was rendered from — or `null` for an item rendered from none;
//! - `repositories`, sorted;
//! - `depends_on`: the item's own forward dependency edges, each `[<edge kind>, <far end's
//!   kind>, <far end>]` sorted, where the far end is named by its title when the store can
//!   read it and by its id when it cannot. A copy files the far end under another id; naming it
//!   by what it is keeps one edge one edge wherever the plan is copied. A document has none;
//! - `metadata`: every caller-owned metadata key and its value, but those the record
//!   `excludes`.
//!
//! Nothing else is in it: not the status, priority, labels, assets, location, id or version,
//! not any key of the reserved `onetaskgraph.` namespace — the attestations included — so an
//! item's attestation is not voided by recording another, and a copy, which carries every
//! member above and re-keys only what is left out, carries a record that stays true of its
//! copy.
//!
//! The attestation is deliberately policy-free: the store records and checks, and says
//! nothing about who may attest or which kinds anything requires.

use std::collections::BTreeMap;

use chrono::{DateTime, SecondsFormat, Utc};
use onetaskgraph_plugin_api::{MetadataKey, Repository};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest as _, Sha256};

use crate::GlobalId;

/// What an attestation kind, an attester label or a bar fingerprint may be: a non-empty string
/// of at most this many characters with no line break, so a record reads back on one line.
pub const MAX_LABEL: usize = 200;

/// Why an attestation could not be composed from what a caller gave.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{field} {problem}")]
pub struct AttestationInputError {
    /// Which input: `--kind`, `--by`, `--bar`, `--exclude` or `--note`.
    pub field: &'static str,
    /// What is wrong with it.
    pub problem: String,
}

/// One kind of attestation, such as `plan-review` or `design-approval`: one to 200 letters,
/// digits, `.`, `_` or `-`.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(try_from = "String", into = "String")]
pub struct AttestationKind(String);

impl AttestationKind {
    /// One kind, once it is established it is one.
    ///
    /// # Errors
    ///
    /// [`AttestationInputError`] naming `--kind` for an empty kind, one longer than
    /// [`MAX_LABEL`], or one holding anything but letters, digits, `.`, `_` and `-`.
    pub fn new(kind: impl Into<String>) -> Result<Self, AttestationInputError> {
        let kind = kind.into();
        let problem = if kind.is_empty() {
            Some("is empty; name the kind, such as `design-approval`".to_owned())
        } else if kind.chars().count() > MAX_LABEL {
            Some(format!("is longer than {MAX_LABEL} characters"))
        } else if !kind
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._-".contains(character))
        {
            Some(format!(
                "{kind:?} holds a character that is not a letter, a digit, `.`, `_` or `-`"
            ))
        } else {
            None
        };
        match problem {
            Some(problem) => Err(AttestationInputError {
                field: "--kind",
                problem,
            }),
            None => Ok(Self(kind)),
        }
    }

    /// The kind as it is spelled.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for AttestationKind {
    type Error = AttestationInputError;

    fn try_from(kind: String) -> Result<Self, Self::Error> {
        Self::new(kind)
    }
}

impl From<AttestationKind> for String {
    fn from(kind: AttestationKind) -> Self {
        kind.0
    }
}

impl std::fmt::Display for AttestationKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

/// One attestation: who cleared the item's authored content, when, which content and against
/// which bar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Attestation {
    /// The kind of attestation this is; the key it is held under.
    pub kind: AttestationKind,
    /// Who attested, as the caller labels them.
    pub by: String,
    /// When it was recorded, in RFC 3339.
    pub at: String,
    /// The digest of the item's authored content when it was recorded: `sha256:` and 64
    /// lowercase hex digits, computed by the store (see the module's documentation).
    pub content_digest: String,
    /// The fingerprint of the bar the content was cleared against, as the caller gave it.
    pub bar: String,
    /// What the attester added, when they added anything.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// The caller-owned metadata keys left out of `content_digest`, sorted; empty when none
    /// were.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(!skip_serializing_if)]
    pub excludes: Vec<String>,
}

/// What a check of one kind of attestation found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum AttestationState {
    /// A record of the kind is there, the item's authored content is the content it cleared,
    /// and the bar asked about, when one was, is the bar it cleared it against.
    Valid,
    /// A record of the kind is there and no longer holds: `reason` says why.
    Void,
    /// The item holds no record of the kind.
    Absent,
}

/// Why a record of an attestation no longer holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum VoidReason {
    /// The item's authored content is not the content the record cleared.
    ContentMoved,
    /// The bar asked about is not the bar the record cleared the content against.
    BarMoved,
}

/// What `attestation check` answers: one kind on one item.
///
/// Every member is always written, `reason` and `record` as `null` when they have nothing to
/// say.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AttestationCheck {
    /// The kind asked about.
    pub kind: AttestationKind,
    /// What the check found.
    pub state: AttestationState,
    /// Why the record is void: `null` unless `state` is `void`.
    pub reason: Option<VoidReason>,
    /// The record of the kind the item holds, or `null` when it holds none.
    pub record: Option<Attestation>,
}

impl Attestation {
    /// The record as the item's metadata holds it.
    #[must_use]
    pub fn to_value(&self) -> Value {
        let mut record = Map::new();
        record.insert(
            "kind".to_owned(),
            Value::String(self.kind.as_str().to_owned()),
        );
        record.insert("by".to_owned(), Value::String(self.by.clone()));
        record.insert("at".to_owned(), Value::String(self.at.clone()));
        record.insert(
            "content_digest".to_owned(),
            Value::String(self.content_digest.clone()),
        );
        record.insert("bar".to_owned(), Value::String(self.bar.clone()));
        if let Some(note) = &self.note {
            record.insert("note".to_owned(), Value::String(note.clone()));
        }
        if !self.excludes.is_empty() {
            record.insert(
                "excludes".to_owned(),
                Value::Array(self.excludes.iter().cloned().map(Value::String).collect()),
            );
        }
        Value::Object(record)
    }
}

impl AttestationCheck {
    /// Whether the record holds.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.state == AttestationState::Valid
    }
}

/// One task's entry in a project's check: the task's qualified id beside what its check found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TaskAttestationCheck {
    /// The task, by qualified id.
    pub id: GlobalId,
    /// What its check found.
    #[serde(flatten)]
    pub check: AttestationCheck,
}

/// What `project attestation check --tasks` answers: one entry per task filed under the
/// project, in the order the source lists them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProjectAttestationCheck {
    /// The project, by qualified id.
    pub project: GlobalId,
    /// The kind asked about.
    pub kind: AttestationKind,
    /// One entry per task.
    pub tasks: Vec<TaskAttestationCheck>,
}

impl ProjectAttestationCheck {
    /// Whether every task's record holds — a project with no task included.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.tasks.iter().all(|task| task.check.is_valid())
    }
}

/// What `attest` answers: the record it wrote, on the item it wrote it to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Attested {
    /// The item, by qualified id.
    pub id: GlobalId,
    /// The record as the source holds it after the write.
    pub record: Attestation,
}

/// The item's authored content, as the digest reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct Authored<'a> {
    /// Its title.
    pub title: &'a str,
    /// Its body.
    pub content: Option<&'a str>,
    /// Its repositories.
    pub repositories: &'a [Repository],
    /// Its whole metadata, as a read answers it: the caller's keys are taken from it, and the
    /// `onetaskgraph.template` entry's `answers_digest`.
    pub metadata: &'a BTreeMap<String, Value>,
    /// Its own forward dependency edges, each `[edge kind, far end's kind, far end]`.
    pub depends_on: Vec<[String; 3]>,
}

/// The digest of `authored`, leaving out the caller-owned metadata keys `excludes` names.
#[must_use]
pub fn content_digest(authored: &Authored<'_>, excludes: &[String]) -> String {
    let metadata: Map<String, Value> = authored
        .metadata
        .iter()
        .filter(|(key, _)| MetadataKey::new(key.as_str()).is_ok())
        .filter(|(key, _)| !excludes.iter().any(|excluded| excluded == *key))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    let answers = authored
        .metadata
        .get(MetadataKey::TEMPLATE_KEY)
        .and_then(|entry| entry.get("answers_digest"))
        .cloned()
        .unwrap_or(Value::Null);
    let mut repositories: Vec<&str> = authored
        .repositories
        .iter()
        .map(Repository::as_str)
        .collect();
    repositories.sort_unstable();
    let mut edges = authored.depends_on.clone();
    edges.sort();
    edges.dedup();
    let document = json!({
        "title": authored.title,
        "content": authored.content,
        "answers_digest": answers,
        "repositories": repositories,
        "depends_on": edges,
        "metadata": metadata,
    });
    let hash = Sha256::digest(canonical(&document).as_bytes());
    let mut digest = String::with_capacity(7 + hash.len() * 2);
    digest.push_str("sha256:");
    for byte in hash {
        digest.push_str(&format!("{byte:02x}"));
    }
    digest
}

/// `value` as canonical JSON: keys sorted at every depth, no insignificant whitespace.
fn canonical(value: &Value) -> String {
    match value {
        Value::Object(entries) => {
            let sorted: BTreeMap<&String, &Value> = entries.iter().collect();
            let inner: Vec<String> = sorted
                .into_iter()
                .map(|(key, entry)| format!("{}:{}", Value::String(key.clone()), canonical(entry)))
                .collect();
            format!("{{{}}}", inner.join(","))
        }
        Value::Array(entries) => {
            let inner: Vec<String> = entries.iter().map(canonical).collect();
            format!("[{}]", inner.join(","))
        }
        scalar => scalar.to_string(),
    }
}

/// One label a record carries — `--by`, `--bar` or `--note` — once it is established it is one.
///
/// # Errors
///
/// [`AttestationInputError`] naming `field` for an empty label, one longer than [`MAX_LABEL`]
/// (`--note` may be any length), or one holding a line break (`--note` may).
pub fn label(field: &'static str, value: &str) -> Result<String, AttestationInputError> {
    let note = field == "--note";
    let problem = if value.trim().is_empty() {
        Some("is empty".to_owned())
    } else if !note && value.chars().count() > MAX_LABEL {
        Some(format!("is longer than {MAX_LABEL} characters"))
    } else if !note && value.contains(['\n', '\r']) {
        Some("holds a line break".to_owned())
    } else {
        None
    };
    match problem {
        Some(problem) => Err(AttestationInputError { field, problem }),
        None => Ok(value.to_owned()),
    }
}

/// The records `metadata` holds, by kind: none when it holds no attestations entry, and only
/// the entries that read as records — one this product did not write is passed over rather
/// than refused, so a hand-edited entry cannot stop a check of another kind.
#[must_use]
pub fn records(metadata: &BTreeMap<String, Value>) -> BTreeMap<String, Value> {
    match metadata.get(MetadataKey::ATTESTATIONS_KEY) {
        Some(Value::Object(entries)) => entries
            .iter()
            .map(|(kind, record)| (kind.clone(), record.clone()))
            .collect(),
        _ => BTreeMap::new(),
    }
}

/// The record of `kind` that `metadata` holds, when it holds one this product can read.
#[must_use]
pub fn record_of(
    metadata: &BTreeMap<String, Value>,
    kind: &AttestationKind,
) -> Option<Attestation> {
    records(metadata)
        .get(kind.as_str())
        .and_then(|record| serde_json::from_value::<Attestation>(record.clone()).ok())
        .filter(|record| record.kind == *kind)
}

/// Hold `record` to the item's current `digest` and, when one is asked about, to `bar`.
#[must_use]
pub fn check(
    kind: &AttestationKind,
    record: Option<Attestation>,
    digest: impl FnOnce(&[String]) -> String,
    bar: Option<&str>,
) -> AttestationCheck {
    let Some(record) = record else {
        return AttestationCheck {
            kind: kind.clone(),
            state: AttestationState::Absent,
            reason: None,
            record: None,
        };
    };
    let reason = if digest(&record.excludes) != record.content_digest {
        Some(VoidReason::ContentMoved)
    } else if bar.is_some_and(|bar| bar != record.bar) {
        Some(VoidReason::BarMoved)
    } else {
        None
    };
    AttestationCheck {
        kind: kind.clone(),
        state: if reason.is_some() {
            AttestationState::Void
        } else {
            AttestationState::Valid
        },
        reason,
        record: Some(record),
    }
}

/// The instant a record is recorded at, as it spells it.
#[must_use]
pub fn now() -> String {
    stamp(Utc::now())
}

fn stamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Secs, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn authored(metadata: &BTreeMap<String, Value>) -> Authored<'_> {
        Authored {
            title: "T",
            content: Some("body"),
            repositories: &[],
            metadata,
            depends_on: vec![["blocks".into(), "task".into(), "Other".into()]],
        }
    }

    #[test]
    fn the_digest_reads_caller_keys_and_answers_and_never_reserved_ones() {
        let mut metadata = BTreeMap::new();
        metadata.insert("myapp.size".to_owned(), json!(3));
        let first = content_digest(&authored(&metadata), &[]);
        assert!(first.starts_with("sha256:") && first.len() == 71, "{first}");

        // A reserved key — the attestations themselves among them — never moves it.
        metadata.insert(MetadataKey::ATTESTATIONS_KEY.to_owned(), json!({"a": 1}));
        metadata.insert("onetaskgraph.origin".to_owned(), json!("x:1"));
        assert_eq!(content_digest(&authored(&metadata), &[]), first);

        // The answers digest of the provenance entry does, and nothing else of the entry.
        metadata.insert(
            MetadataKey::TEMPLATE_KEY.to_owned(),
            json!({"answers_digest": "sha256:a", "body_digest": "sha256:b"}),
        );
        let answered = content_digest(&authored(&metadata), &[]);
        assert_ne!(answered, first);
        metadata.insert(
            MetadataKey::TEMPLATE_KEY.to_owned(),
            json!({"answers_digest": "sha256:a", "body_digest": "sha256:c"}),
        );
        assert_eq!(content_digest(&authored(&metadata), &[]), answered);

        // A caller key moves it, unless it is excluded.
        metadata.insert("myapp.size".to_owned(), json!(4));
        assert_ne!(content_digest(&authored(&metadata), &[]), answered);
        assert_eq!(
            content_digest(&authored(&metadata), &["myapp.size".to_owned()]),
            {
                let mut without = metadata.clone();
                without.remove("myapp.size");
                content_digest(&authored(&without), &[])
            }
        );
    }

    #[test]
    fn a_check_says_content_before_bar() {
        let kind = AttestationKind::new("design-approval").expect("a kind");
        let record = Attestation {
            kind: kind.clone(),
            by: "ada".into(),
            at: "2026-11-09T00:00:00Z".into(),
            content_digest: "sha256:x".into(),
            bar: "bar-1".into(),
            note: None,
            excludes: Vec::new(),
        };
        let valid = check(
            &kind,
            Some(record.clone()),
            |_| "sha256:x".into(),
            Some("bar-1"),
        );
        assert_eq!((valid.state, valid.reason), (AttestationState::Valid, None));
        let bar = check(
            &kind,
            Some(record.clone()),
            |_| "sha256:x".into(),
            Some("bar-2"),
        );
        assert_eq!(bar.reason, Some(VoidReason::BarMoved));
        let moved = check(&kind, Some(record), |_| "sha256:y".into(), Some("bar-2"));
        assert_eq!(moved.reason, Some(VoidReason::ContentMoved));
        let absent = check(&kind, None, |_| unreachable!(), None);
        assert_eq!(
            (absent.state, absent.record),
            (AttestationState::Absent, None)
        );
    }

    #[test]
    fn a_record_reads_back_as_it_serialises() {
        let record = Attestation {
            kind: AttestationKind::new("plan-review").expect("a kind"),
            by: "ada".into(),
            at: "2026-11-09T00:00:00Z".into(),
            content_digest: "sha256:x".into(),
            bar: "b".into(),
            note: Some("n".into()),
            excludes: vec!["myapp.k".into()],
        };
        assert_eq!(
            record.to_value(),
            serde_json::to_value(&record).expect("JSON")
        );
        let bare = Attestation {
            note: None,
            excludes: Vec::new(),
            ..record
        };
        assert_eq!(bare.to_value(), serde_json::to_value(&bare).expect("JSON"));
        assert!(bare.to_value().get("note").is_none());
    }

    #[test]
    fn a_kind_and_a_label_are_refused_by_their_flag() {
        assert!(AttestationKind::new("plan-review").is_ok());
        assert_eq!(AttestationKind::new("").unwrap_err().field, "--kind");
        assert!(AttestationKind::new("two words").is_err());
        assert_eq!(label("--by", "a\nb").unwrap_err().field, "--by");
        assert!(label("--note", "a\nb").is_ok());
        assert_eq!(stamp(DateTime::UNIX_EPOCH), "1970-01-01T00:00:00Z");
    }
}
