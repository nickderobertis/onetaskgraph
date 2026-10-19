//! An issue a landing closed, named the way the version-control side names it, and what one
//! source holds for it — and the items whose card contradicts their closure.
//!
//! Both halves serve a source whose status is a card on a board beside an issue that closes on
//! its own: [`Capabilities::complete_by_issue`](crate::Capabilities::complete_by_issue) and
//! [`Capabilities::reconcile_status`](crate::Capabilities::reconcile_status) say whether a
//! source answers them, and a source whose status *is* its closure declares neither.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::str::FromStr;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};

use crate::{
    Classification, ItemKind, NativeId, Project, Repository, StatusCategory, Task, TaskRef,
};

/// The pattern every [`IssueRef`] is spelled in: `owner/name#N`, the number at most nineteen
/// digits — every such number fits the `u64` it is read into.
///
/// What the JSON Schema publishes; [`IssueRef`]'s `FromStr` is the same rule written out, and
/// `tests/issue_ref.rs` derives a matcher from this constant and holds the two to one language.
pub const ISSUE_REF_PATTERN: &str = "^[A-Za-z0-9-]+/[A-Za-z0-9._-]+#[1-9][0-9]{0,18}$";

/// The most digits an issue number is spelled with: nineteen, so every one fits a `u64`.
const MAX_NUMBER_DIGITS: usize = 19;

/// One issue of one repository, as `owner/name#N` — exactly the spelling `onevcs` reports in a
/// landing event's `closes`.
///
/// The owner and the name are compared ignoring case, as GitHub resolves them; the number is
/// the issue's own number on that repository.
#[derive(Debug, Clone, Eq)]
pub struct IssueRef {
    /// The login of the repository's owner.
    pub owner: String, // llmlint: ignore[invalid_states_unrepresentable] The fields are the contract's stated shape, `IssueRef { owner, name, number }`, which the engine and the read API build against; `FromStr` is the one validating boundary every spelling crosses, and serde reads through it.
    /// The repository's name.
    pub name: String, // llmlint: ignore[invalid_states_unrepresentable] As `owner`: the contract's stated shape, validated by `FromStr`.
    /// The issue's number on that repository.
    pub number: u64, // llmlint: ignore[invalid_states_unrepresentable] As `owner`: the contract's stated shape, validated by `FromStr`.
}

impl IssueRef {
    /// Whether `owner` is a GitHub login: letters, digits and hyphens.
    fn valid_owner(owner: &str) -> bool {
        !owner.is_empty()
            && owner
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '-')
    }

    /// Whether `name` is a repository name: letters, digits, `.`, `_` and `-`.
    fn valid_name(name: &str) -> bool {
        !name.is_empty()
            && name.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
            })
    }
}

impl PartialEq for IssueRef {
    fn eq(&self, other: &Self) -> bool {
        self.number == other.number
            && self.owner.eq_ignore_ascii_case(&other.owner)
            && self.name.eq_ignore_ascii_case(&other.name)
    }
}

impl Hash for IssueRef {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.owner.to_ascii_lowercase().hash(state);
        self.name.to_ascii_lowercase().hash(state);
        self.number.hash(state);
    }
}

impl FromStr for IssueRef {
    type Err = String;

    /// Read `owner/name#N`, refusing anything else and naming the reference refused.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let refused = || {
            format!(
                "{value:?} is not an issue reference; spell one owner/name#N, as a landing's \
                 `closes` does"
            )
        };
        let (repository, number) = value.rsplit_once('#').ok_or_else(refused)?;
        let (owner, name) = repository.split_once('/').ok_or_else(refused)?;
        if !Self::valid_owner(owner)
            || !Self::valid_name(name)
            || number.is_empty()
            || number.len() > MAX_NUMBER_DIGITS
            || number.starts_with('0')
            || !number.chars().all(|digit| digit.is_ascii_digit())
        {
            return Err(refused());
        }
        Ok(Self {
            owner: owner.to_owned(),
            name: name.to_owned(),
            number: number.parse().map_err(|_| refused())?,
        })
    }
}

impl fmt::Display for IssueRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}#{}", self.owner, self.name, self.number)
    }
}

impl Serialize for IssueRef {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for IssueRef {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for IssueRef {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "IssueRef".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "pattern": ISSUE_REF_PATTERN,
            "description": "One issue of one repository, as `owner/name#N`.",
        })
    }
}

/// What one source holds for one issue a landing closed, and what moving it to `done` would
/// write — the answer to [`TaskSource::issue_holdings`](crate::TaskSource::issue_holdings).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct IssueHolding {
    /// The issue asked about.
    pub issue: IssueRef,
    /// What completing the issue comes to here, its tag and members beside the issue.
    #[serde(flatten)]
    pub completion: Completion,
}

/// The item one source holds for an issue — a task, or a project such as a plan's home card —
/// as far as moving it to `done` needs it.
///
/// It carries of the item exactly what the write and the rules around it need — which item and
/// of which kind, the card it sits at, the tasks it delivers, and what decides where its id may
/// be written — rather than the whole item, because the read that answers it is priced per issue
/// and a whole item is not what a landing's closure needs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct HeldTask {
    /// The item, by this source's own id for it.
    pub id: NativeId,
    /// Which kind of item it is. Defaulted to `task` and left out of the wire while it is one,
    /// so a plugin written before a project could be held answers exactly what it answered.
    #[serde(default = "task_kind", skip_serializing_if = "is_task")]
    #[schemars(!skip_serializing_if)]
    pub kind: ItemKind,
    /// The name of the option the task's card sits at, as the source spells it, when it sits
    /// at one.
    #[serde(default)]
    pub option: Option<String>,
    /// The task's own classification.
    #[serde(default)]
    pub classification: Classification,
    /// The repositories the task names.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub repositories: Vec<Repository>,
    /// The tasks it delivers, which a write of it to `done` keeps in step — always empty for a
    /// project, which delivers none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub delivers: Vec<TaskRef>,
}

/// The kind a [`HeldTask`] that does not say is: a task, as every holding was before a project
/// could be held.
const fn task_kind() -> ItemKind {
    ItemKind::Task
}

/// Whether `kind` is the one a [`HeldTask`] leaves unsaid.
// The signature is the one `skip_serializing_if` calls with.
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_task(kind: &ItemKind) -> bool {
    *kind == ItemKind::Task
}

/// What moving one issue's item to `done` comes to at one source — the task it holds riding on
/// every outcome that has one, so none can be told without its task, or with one, that has not.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "completion", rename_all = "kebab-case")]
pub enum Completion {
    /// The source holds no item for the issue, or cannot see the issue at all.
    Absent,
    /// The issue's closure already says something else, which a landing does not overrule.
    Skipped {
        /// The task held for the issue.
        task: HeldTask,
        /// Why, as `closed as <reason>`.
        reason: String,
    },
    /// The issue is closed as completed and its card already sits at the `done` option.
    Unchanged {
        /// The task held for the issue.
        task: HeldTask,
    },
    /// The item is to be written `done`: its card moved to the `done` option, and its issue
    /// closed as completed if it is still open.
    Due {
        /// The task held for the issue.
        task: HeldTask,
        /// The name of the option `done` writes it at.
        to: String,
    },
}

/// The two closures a card is brought in line with: the categories a closed issue's reason can
/// read as that name an option to move its card to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ClosedAs {
    /// Closed as completed, or with no reason: `done`.
    Done,
    /// Closed as not planned or as a duplicate: `cancelled`.
    Cancelled,
}

impl ClosedAs {
    /// The status category this closure reads as.
    #[must_use]
    pub const fn category(self) -> StatusCategory {
        match self {
            Self::Done => StatusCategory::Done,
            Self::Cancelled => StatusCategory::Cancelled,
        }
    }
}

/// One item whose card contradicts its issue's closure, and the option that brings it in line
/// — one entry of [`TaskSource::status_contradictions`](crate::TaskSource::status_contradictions).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StatusContradiction {
    /// The item as it reads now: `task` or `project`, named by its kind.
    #[serde(flatten)]
    pub item: ContradictedItem,
    /// The option its card sits at, or `None` for a card with no option.
    #[serde(default)]
    pub option: Option<String>,
    /// What its closure reads as.
    pub closed_as: ClosedAs,
    /// The name of the option the source's mapping gives that category for the item's kind,
    /// which its card is moved to.
    pub to: String,
}

/// The item a [`StatusContradiction`] is about, under the member naming its kind — `task`, as
/// every contradiction was before a project could be one, or `project`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ContradictedItem {
    /// A task, as `get_task` reads it.
    Task(Task),
    /// A project, as `get_project` reads it.
    Project(Project),
}

impl ContradictedItem {
    /// Which kind of item it is.
    #[must_use]
    pub const fn kind(&self) -> ItemKind {
        match self {
            Self::Task(_) => ItemKind::Task,
            Self::Project(_) => ItemKind::Project,
        }
    }

    /// The item's own id at its source.
    #[must_use]
    pub const fn id(&self) -> &NativeId {
        match self {
            Self::Task(task) => &task.id,
            Self::Project(project) => &project.id,
        }
    }

    /// The item's declared classification.
    #[must_use]
    pub const fn classification(&self) -> Classification {
        match self {
            Self::Task(task) => task.classification,
            Self::Project(project) => project.classification,
        }
    }

    /// The repositories the item names.
    #[must_use]
    pub fn repositories(&self) -> &[Repository] {
        match self {
            Self::Task(task) => &task.repositories,
            Self::Project(project) => &project.repositories,
        }
    }
}
