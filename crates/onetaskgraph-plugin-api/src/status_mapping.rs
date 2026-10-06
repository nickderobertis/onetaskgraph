//! `status_mapping`: the one grammar every source that names its statuses is configured with.
//!
//! A source whose backend has a vocabulary of status names of its own — a board's `Status`
//! options, a Linear team's workflow states and its workspace's project statuses — is told
//! which name each [`StatusCategory`] is, for each [`ItemKind`], by one object:
//!
//! ```yaml
//! status_mapping:
//!   todo: Todo                                 # every kind
//!   draft: null                                # disabled for every kind
//!   done: { task: Done, project: Completed }   # per kind
//! ```
//!
//! The keys are status categories, spelled exactly as [`StatusCategory`] serializes. A value
//! is a non-blank name for every kind, `null` to disable the category for every kind, or an
//! object with the optional keys `task` and `project`, each a non-blank name, where a key left
//! out leaves the category unmapped for that kind. Anything else is refused as the
//! configuration is read, naming the category and the offending part.
//!
//! Defined here rather than in each plugin because it is one concept with one grammar: two
//! spellings of it would let a configuration that loads under one source be refused, or read
//! differently, under another.

use std::{borrow::Cow, fmt};

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{self, MapAccess, Visitor},
    ser::SerializeMap,
};
use serde_json::Value;

use crate::{ItemKind, SourceError, SourceName, StatusCategory};

/// Every status category, in the contract's own order — the order a mapping is held and
/// reported in.
const CATEGORIES: [StatusCategory; 8] = [
    StatusCategory::Draft,
    StatusCategory::Backlog,
    StatusCategory::Todo,
    StatusCategory::Queued,
    StatusCategory::InProgress,
    StatusCategory::Done,
    StatusCategory::Cancelled,
    StatusCategory::Unknown,
];

/// Where `category` sits in [`CATEGORIES`] — an exhaustive match, so a category the contract
/// adds fails to compile here rather than going unmapped.
const fn position(category: StatusCategory) -> usize {
    match category {
        StatusCategory::Draft => 0,
        StatusCategory::Backlog => 1,
        StatusCategory::Todo => 2,
        StatusCategory::Queued => 3,
        StatusCategory::InProgress => 4,
        StatusCategory::Done => 5,
        StatusCategory::Cancelled => 6,
        StatusCategory::Unknown => 7,
    }
}

const _: () = {
    let mut index = 0;
    while index < CATEGORIES.len() {
        assert!(position(CATEGORIES[index]) == index);
        index += 1;
    }
};

/// A category as the configuration spells it — `in-progress`, `queued`.
const fn category_key(category: StatusCategory) -> &'static str {
    match category {
        StatusCategory::Draft => "draft",
        StatusCategory::Backlog => "backlog",
        StatusCategory::Todo => "todo",
        StatusCategory::Queued => "queued",
        StatusCategory::InProgress => "in-progress",
        StatusCategory::Done => "done",
        StatusCategory::Cancelled => "cancelled",
        StatusCategory::Unknown => "unknown",
    }
}

/// An item kind as the configuration spells it.
const fn kind_key(kind: ItemKind) -> &'static str {
    match kind {
        ItemKind::Task => "task",
        ItemKind::Project => "project",
    }
}

/// One status name a source's backend holds: a board option, a workflow state, a project
/// status.
///
/// Validated on the way in, so a blank name — which nothing in any backend can be called — is
/// a name this type cannot hold. Compared case-insensitively wherever a source matches it,
/// because every backend this product reads matches its own names that way.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct StatusName(String);

impl StatusName {
    /// The name, as the configuration spells it.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether `other` is this name, as every backend compares its own: ignoring ASCII case.
    #[must_use]
    pub fn matches(&self, other: &str) -> bool {
        self.0.eq_ignore_ascii_case(other)
    }
}

impl TryFrom<String> for StatusName {
    type Error = String;

    fn try_from(name: String) -> Result<Self, Self::Error> {
        if name.trim().is_empty() {
            Err("a status name cannot be blank".to_owned())
        } else {
            Ok(Self(name))
        }
    }
}

impl From<StatusName> for String {
    fn from(name: StatusName) -> Self {
        name.0
    }
}

impl fmt::Display for StatusName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl JsonSchema for StatusName {
    fn schema_name() -> Cow<'static, str> {
        "StatusName".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "One status name a source's backend holds, matched ignoring case. Never blank.",
            "type": "string",
            "minLength": 1,
            "pattern": "\\S"
        })
    }
}

/// What one category of a `status_mapping` names: one name for every kind, or a name for
/// each kind it maps.
///
/// Four variants rather than a pair of optional names, so a per-kind object naming no kind —
/// which the grammar refuses — is a value this type cannot hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusNames {
    /// The same name for a task and for a project: the bare-string form.
    Every(StatusName),
    /// `{ task: … }`: a name for a task, and the category unmapped for a project.
    Task(StatusName),
    /// `{ project: … }`: a name for a project, and the category unmapped for a task.
    Project(StatusName),
    /// `{ task: …, project: … }`: a name for each.
    PerKind {
        /// The name for a task.
        task: StatusName,
        /// The name for a project.
        project: StatusName,
    },
}

impl StatusNames {
    /// The name this gives `kind`, or `None` where it leaves that kind unmapped.
    #[must_use]
    pub fn for_kind(&self, kind: ItemKind) -> Option<&StatusName> {
        match (self, kind) {
            (Self::Every(name), _)
            | (Self::Task(name), ItemKind::Task)
            | (Self::Project(name), ItemKind::Project)
            | (Self::PerKind { task: name, .. }, ItemKind::Task)
            | (Self::PerKind { project: name, .. }, ItemKind::Project) => Some(name),
            (Self::Task(_), ItemKind::Project) | (Self::Project(_), ItemKind::Task) => None,
        }
    }

    /// One configured value, read with the category it is under so a refusal can name it.
    fn parse(category: StatusCategory, value: Value) -> Result<Self, String> {
        let at = category_key(category);
        let name = |value: Value, path: &str| -> Result<StatusName, String> {
            match value {
                Value::String(name) => StatusName::try_from(name).map_err(|_| {
                    format!("status_mapping.{path} is blank; a status name cannot be blank")
                }),
                Value::Null => Err(format!(
                    "status_mapping.{path} is null; leave that key out to leave {at} unmapped for \
                     that kind, or set status_mapping.{at} to null to disable {at} for every kind"
                )),
                other => Err(format!(
                    "status_mapping.{path} is {other}, which is not a status name; write the \
                     name as a string"
                )),
            }
        };
        match value {
            Value::String(_) => name(value, at).map(Self::Every),
            Value::Object(object) => {
                if object.is_empty() {
                    return Err(format!(
                        "status_mapping.{at} is an empty object, which maps no kind; to disable \
                         {at} for every kind, write null"
                    ));
                }
                let mut task = None;
                let mut project = None;
                for (key, value) in object {
                    match key.as_str() {
                        "task" => task = Some(name(value, &format!("{at}.task"))?),
                        "project" => project = Some(name(value, &format!("{at}.project"))?),
                        other => {
                            return Err(format!(
                                "status_mapping.{at} names {other:?}, which is not an item kind; \
                                 the kinds are task and project"
                            ));
                        }
                    }
                }
                Ok(match (task, project) {
                    (Some(task), Some(project)) => Self::PerKind { task, project },
                    (Some(task), None) => Self::Task(task),
                    (None, Some(project)) => Self::Project(project),
                    (None, None) => unreachable!("a non-empty object of known keys names a kind"),
                })
            }
            other => Err(format!(
                "status_mapping.{at} is {other}, which is none of a status name, null, or an \
                 object with the keys task and project"
            )),
        }
    }
}

impl Serialize for StatusNames {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Every(name) => name.serialize(serializer),
            Self::Task(task) => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("task", task)?;
                map.end()
            }
            Self::Project(project) => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("project", project)?;
                map.end()
            }
            Self::PerKind { task, project } => {
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("task", task)?;
                map.serialize_entry("project", project)?;
                map.end()
            }
        }
    }
}

impl JsonSchema for StatusNames {
    fn schema_name() -> Cow<'static, str> {
        "StatusNames".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let name = generator.subschema_for::<StatusName>();
        let only = |kind: &str| {
            json_schema!({
                "type": "object",
                "properties": { (kind): name },
                "required": [kind],
                "additionalProperties": false
            })
        };
        json_schema!({
            "description": "What one category of a status_mapping names: one name for every item kind, or an object naming it for a task, for a project, or for each. A kind the object leaves out leaves the category unmapped for that kind.",
            "anyOf": [
                name,
                only("task"),
                only("project"),
                {
                    "type": "object",
                    "properties": { "task": name, "project": name },
                    "required": ["task", "project"],
                    "additionalProperties": false
                }
            ]
        })
    }
}

/// Why a mapping gives no name for one category and one kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnmappedStatus {
    /// The mapping does not mention the category at all.
    Unconfigured,
    /// The mapping sets the category to `null`, disabling it for every kind.
    Disabled,
    /// The mapping names the category for the other kind only.
    OtherKindOnly,
}

impl UnmappedStatus {
    /// The refusal of a status write `source` has no name for: naming the source, the kind,
    /// the category, why, and the key to set.
    #[must_use]
    pub fn refusal(
        self,
        source: &SourceName,
        category: StatusCategory,
        kind: ItemKind,
    ) -> SourceError {
        let at = category_key(category);
        let of = kind_key(kind);
        let why = match self {
            Self::Unconfigured => format!("its status_mapping does not name {at}"),
            Self::Disabled => {
                format!("its status_mapping sets {at} to null, disabling it for every kind")
            }
            Self::OtherKindOnly => format!("its status_mapping names {at} for the other kind only"),
        };
        SourceError::Refused {
            message: format!(
                "source {source} has no {of} status name for {at}: {why}; next: set \
                 status_mapping.{at}.{of} of this source to the {of} status {at} should be \
                 written as"
            ),
        }
    }
}

/// One source's `status_mapping`: for each status category it mentions, the names it gives
/// each item kind, or `null`.
///
/// Held in the contract's category order with each category at most once, so iterating it
/// reports in that order and a category named twice is a state it cannot hold.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StatusMapping {
    entries: Vec<(StatusCategory, Option<StatusNames>)>,
}

impl StatusMapping {
    /// Whether the mapping mentions no category at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Whether the mapping mentions `category`, as a name or as `null`.
    #[must_use]
    pub fn mentions(&self, category: StatusCategory) -> bool {
        self.entries.iter().any(|(held, _)| *held == category)
    }

    /// What the mapping says of `category`: `None` where it does not mention it, `Some(None)`
    /// where it sets it to `null`, and the names otherwise.
    #[must_use]
    pub fn entry(&self, category: StatusCategory) -> Option<Option<&StatusNames>> {
        self.entries
            .iter()
            .find(|(held, _)| *held == category)
            .map(|(_, names)| names.as_ref())
    }

    /// The name `category` is written as for `kind`, or why the mapping gives none.
    ///
    /// # Errors
    ///
    /// The [`UnmappedStatus`] saying which of the three ways the mapping gives no name.
    pub fn name_for(
        &self,
        category: StatusCategory,
        kind: ItemKind,
    ) -> Result<&StatusName, UnmappedStatus> {
        match self.entry(category) {
            None => Err(UnmappedStatus::Unconfigured),
            Some(None) => Err(UnmappedStatus::Disabled),
            Some(Some(names)) => names.for_kind(kind).ok_or(UnmappedStatus::OtherKindOnly),
        }
    }

    /// Every name the mapping gives `kind`, with its category, in category order.
    pub fn names(&self, kind: ItemKind) -> impl Iterator<Item = (StatusCategory, &StatusName)> {
        self.entries.iter().filter_map(move |(category, names)| {
            names
                .as_ref()
                .and_then(|names| names.for_kind(kind))
                .map(|name| (*category, name))
        })
    }

    /// The category a `kind` name reads as, when the mapping names it for that kind.
    #[must_use]
    pub fn category_of(&self, kind: ItemKind, name: &str) -> Option<StatusCategory> {
        self.names(kind)
            .find_map(|(category, mapped)| mapped.matches(name).then_some(category))
    }

    /// Refuse two categories sent to one name of one kind, ignoring case: that name could read
    /// back as only one of them.
    ///
    /// Takes the names to compare rather than reading this mapping's own, so a source whose
    /// unmentioned categories keep shipped defaults compares what it really resolves to.
    ///
    /// # Errors
    ///
    /// [`SourceError::Config`] naming the source, the kind, both categories and the name.
    pub fn distinct<'a>(
        source: &SourceName,
        kind: ItemKind,
        names: impl IntoIterator<Item = (StatusCategory, &'a str)>,
    ) -> Result<(), SourceError> {
        let mut seen: Vec<(StatusCategory, &str)> = Vec::new();
        for (category, name) in names {
            if let Some((other, _)) = seen
                .iter()
                .find(|(_, held)| held.eq_ignore_ascii_case(name))
            {
                return Err(SourceError::Config {
                    message: format!(
                        "source {source}: status_mapping sends both {} and {} to the {} status \
                         {name:?}; one name cannot read back as two categories, so map one of \
                         them to another name",
                        category_key(*other),
                        category_key(category),
                        kind_key(kind),
                    ),
                });
            }
            seen.push((category, name));
        }
        Ok(())
    }
}

impl Serialize for StatusMapping {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.entries.len()))?;
        for (category, names) in &self.entries {
            map.serialize_entry(category_key(*category), names)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for StatusMapping {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Entries;

        impl<'de> Visitor<'de> for Entries {
            type Value = StatusMapping;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a status_mapping object keyed by status category")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<StatusMapping, A::Error> {
                let mut entries: Vec<(StatusCategory, Option<StatusNames>)> = Vec::new();
                while let Some(key) = map.next_key::<String>()? {
                    let category = CATEGORIES
                        .into_iter()
                        .find(|category| category_key(*category) == key)
                        .ok_or_else(|| {
                            de::Error::custom(format!(
                                "status_mapping names {key:?}, which is not a status category; \
                                 the categories are {}",
                                CATEGORIES.map(category_key).join(", ")
                            ))
                        })?;
                    if entries.iter().any(|(held, _)| *held == category) {
                        return Err(de::Error::custom(format!(
                            "status_mapping names {key} twice"
                        )));
                    }
                    let value: Value = map.next_value()?;
                    let names = match value {
                        Value::Null => None,
                        value => {
                            Some(StatusNames::parse(category, value).map_err(de::Error::custom)?)
                        }
                    };
                    entries.push((category, names));
                }
                entries.sort_by_key(|(category, _)| position(*category));
                Ok(StatusMapping { entries })
            }
        }

        deserializer.deserialize_map(Entries)
    }
}

impl JsonSchema for StatusMapping {
    fn schema_name() -> Cow<'static, str> {
        "StatusMapping".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let category = generator.subschema_for::<StatusCategory>();
        let names = generator.subschema_for::<StatusNames>();
        json_schema!({
            "description": "Which name each status category is, for each item kind. Keyed by status category; each value is one name for every kind, null to disable the category for every kind, or an object naming it for a task, a project or each.",
            "type": "object",
            "propertyNames": category,
            "additionalProperties": { "anyOf": [names, { "type": "null" }] }
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn read(value: Value) -> Result<StatusMapping, String> {
        serde_json::from_value(value).map_err(|error| error.to_string())
    }

    #[test]
    fn every_form_the_grammar_admits_loads_and_answers_by_kind() {
        let mapping = read(json!({
            "todo": "Todo",
            "draft": null,
            "done": {"task": "Done", "project": "Completed"},
            "queued": {"task": "Queued"},
            "backlog": {"project": "Idea"},
        }))
        .unwrap();
        let name = |category, kind| mapping.name_for(category, kind).map(StatusName::as_str);
        assert_eq!(name(StatusCategory::Todo, ItemKind::Task), Ok("Todo"));
        assert_eq!(name(StatusCategory::Todo, ItemKind::Project), Ok("Todo"));
        assert_eq!(
            name(StatusCategory::Draft, ItemKind::Task),
            Err(UnmappedStatus::Disabled)
        );
        assert_eq!(
            name(StatusCategory::Done, ItemKind::Project),
            Ok("Completed")
        );
        assert_eq!(
            name(StatusCategory::Queued, ItemKind::Project),
            Err(UnmappedStatus::OtherKindOnly)
        );
        assert_eq!(
            name(StatusCategory::Backlog, ItemKind::Task),
            Err(UnmappedStatus::OtherKindOnly)
        );
        assert_eq!(
            name(StatusCategory::Unknown, ItemKind::Task),
            Err(UnmappedStatus::Unconfigured)
        );
        assert_eq!(
            mapping.category_of(ItemKind::Project, "completed"),
            Some(StatusCategory::Done)
        );
        assert_eq!(mapping.category_of(ItemKind::Task, "Completed"), None);
        // Held and written back in the contract's order, every form as it was given.
        assert_eq!(
            serde_json::to_value(&mapping).unwrap(),
            json!({"draft": null, "backlog": {"project": "Idea"}, "todo": "Todo",
                   "queued": {"task": "Queued"}, "done": {"task": "Done", "project": "Completed"}})
        );
        assert_eq!(
            read(serde_json::to_value(&mapping).unwrap()).unwrap(),
            mapping
        );
    }

    #[test]
    fn every_form_the_grammar_refuses_is_refused_naming_the_part() {
        for (value, said) in [
            (
                json!({"doing": "Doing"}),
                "\"doing\", which is not a status category",
            ),
            (
                json!({"done": {}}),
                "status_mapping.done is an empty object, which maps no kind; to disable done for every kind, write null",
            ),
            (
                json!({"done": {"task": "Done", "epic": "Done"}}),
                "status_mapping.done names \"epic\", which is not an item kind",
            ),
            (
                json!({"done": {"task": null}}),
                "status_mapping.done.task is null",
            ),
            (
                json!({"done": {"project": " "}}),
                "status_mapping.done.project is blank",
            ),
            (json!({"done": ""}), "status_mapping.done is blank"),
            (json!({"done": 3}), "status_mapping.done is 3"),
            (
                json!({"done": {"task": 3}}),
                "status_mapping.done.task is 3",
            ),
        ] {
            let error = read(value.clone()).unwrap_err();
            assert!(
                error.contains(said),
                "{value} was refused with {error:?}, not naming {said:?}"
            );
        }
    }

    /// The keys this grammar reads and writes, and the kinds its refusals name, are the
    /// contract's own serialization of `StatusCategory` and `ItemKind`: a variant renamed on the
    /// wire there and not here fails this rather than a configuration.
    #[test]
    fn every_key_is_the_contracts_own_spelling_of_its_category_and_kind() {
        for category in CATEGORIES {
            assert_eq!(
                serde_json::to_value(category).unwrap(),
                json!(category_key(category))
            );
        }
        for kind in [ItemKind::Task, ItemKind::Project] {
            assert_eq!(serde_json::to_value(kind).unwrap(), json!(kind_key(kind)));
        }
    }

    #[test]
    fn two_categories_on_one_name_of_one_kind_are_refused_ignoring_case() {
        let source = SourceName::new("work").unwrap();
        let error = StatusMapping::distinct(
            &source,
            ItemKind::Task,
            [
                (StatusCategory::Todo, "Todo"),
                (StatusCategory::Queued, "TODO"),
            ],
        )
        .unwrap_err();
        assert!(
            error.to_string().contains(
                "source work: status_mapping sends both todo and queued to the task status \"TODO\""
            ),
            "{error}"
        );
        StatusMapping::distinct(
            &source,
            ItemKind::Task,
            [
                (StatusCategory::Todo, "Todo"),
                (StatusCategory::Queued, "Queued"),
            ],
        )
        .unwrap();
    }

    #[test]
    fn an_unmapped_status_is_refused_naming_the_source_kind_category_and_key() {
        let source = SourceName::new("work").unwrap();
        let message = UnmappedStatus::OtherKindOnly
            .refusal(&source, StatusCategory::InProgress, ItemKind::Project)
            .to_string();
        for part in [
            "source work",
            "project status name for in-progress",
            "status_mapping.in-progress.project",
        ] {
            assert!(message.contains(part), "{message} does not name {part}");
        }
    }
}
