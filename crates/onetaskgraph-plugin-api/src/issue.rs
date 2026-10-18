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

use crate::{Classification, NativeId, Repository, Status, Task, TaskRef};

/// The pattern every [`IssueRef`] is spelled in: `owner/name#N`.
pub const ISSUE_REF_PATTERN: &str = "^[A-Za-z0-9-]+/[A-Za-z0-9._-]+#[1-9][0-9]*$";

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
///
/// It carries of the task exactly what the write and the rules around it need — which task, the
/// card it sits at, the tasks it delivers, and what decides where its id may be written — rather
/// than the whole task, because the read that answers it is priced per issue and a whole task
/// is not what a landing's closure needs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct IssueHolding {
    /// The issue asked about.
    pub issue: IssueRef,
    /// The task this source holds for it — `None` when it holds none.
    #[serde(default)]
    pub task: Option<NativeId>,
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
    /// The tasks it delivers, which a write of it to `done` keeps in step.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub delivers: Vec<TaskRef>,
    /// What completing the issue comes to here, its tag and members beside the others.
    #[serde(flatten)]
    pub completion: Completion,
}

/// What moving one issue's item to `done` comes to at one source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "completion", rename_all = "kebab-case")]
pub enum Completion {
    /// The source holds no item for the issue, or cannot see the issue at all.
    Absent,
    /// The issue's closure already says something else, which a landing does not overrule.
    Skipped {
        /// Why, as `closed as <reason>`.
        reason: String,
    },
    /// The issue is closed as completed and its card already sits at the `done` option.
    Unchanged,
    /// The item is to be written `done`: its card moved to this status, and its issue closed
    /// as completed if it is still open.
    Due {
        /// The status the item will read once written.
        to: Status,
    },
}

/// One item whose card contradicts its issue's closure, and the status that brings it in line
/// — one entry of [`TaskSource::status_contradictions`](crate::TaskSource::status_contradictions).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StatusContradiction {
    /// The task, as it reads now.
    pub task: Task,
    /// The option its card sits at, or `None` for a card with no option.
    #[serde(default)]
    pub option: Option<String>,
    /// The status its closure implies: `done` or `cancelled`, named by the option the source's
    /// mapping gives that category.
    pub to: Status,
}
