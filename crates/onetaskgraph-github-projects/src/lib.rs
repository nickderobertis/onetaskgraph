//! A stateless onetaskgraph source over one GitHub Projects v2 board.
//!
//! **A board is a container of projects, not a project.** Its own `title`,
//! `shortDescription` and `readme` are never read as an item's fields and are never
//! written: nothing in this source can rename the board a user configured.
//!
//! **A project is an issue and its tasks are that issue's sub-issues.** GitHub's schema
//! decides that: `Issue` exposes `parent`, `subIssues` and `subIssuesSummary`, and
//! `DraftIssue` exposes none of them. Creating an issue needs a `repositoryId`, and a
//! board has none, so [`GitHubProjectsConfig::repository`] names the one repository this
//! source creates its project and task issues in; a write without it is refused naming
//! the field.
//!
//! **Telling a project from a task.** A board issue is a project when *either* it has
//! sub-issues *or* it carries [`ItemKind::METADATA_KEY`]; otherwise it is a task. A
//! sub-issue is always a task, whatever it carries. The marker is sufficient and never
//! necessary: it is what makes an *empty* project — the state a project copy passes
//! through between creating the project and filing its first task — readable as a
//! project, while the sub-issue arm lets a person author a project on the board by hand
//! with no knowledge of this product's metadata at all. Pull requests are neither a
//! project nor a task and are ignored.
//!
//! **Where metadata lives.** Short typed things go to typed fields and native relations:
//! status to the board's `Status` single-select and the issue's own state, the copy
//! origin to a source-owned `onetaskgraph.origin` text field, and dependencies to
//! `blockedBy` and to sub-issue links. Unbounded caller JSON goes in a trailing
//! `<!-- onetaskgraph.metadata ... -->` comment at the end of the issue body — the same
//! encoding `docs/metadata.md` settles for Linear, not a second one. A ProjectV2 text
//! field is length-bounded and `shortDescription` is capped at 300 characters, which is
//! why neither can hold a caller's own prose.
//!
//! **Status.** `status_mapping` is per-instance configuration from a status category to
//! `null`, a board `Status` option name, or a closed state of `completed` or
//! `not-planned`. Nothing here ever calls `updateProjectV2Field`: that mutation's
//! `singleSelectOptions` *overwrites* a field's option set, so no addition is additive
//! and a mistake destroys every item's status. A status this board cannot represent is a
//! refusal naming the status and the instance instead.
//!
//! `done` closes the issue by default because GitHub derives `subIssuesSummary.completed`
//! and the board's own `Sub-issues progress` field from closed sub-issues: a plan whose
//! finished tasks were only moved to a "Done" column would read 0% complete forever.
//!
//! # What this source declares, field by field
//!
//! One verdict per field of [`Capabilities`], and what `Native` means when this source
//! says it. *Proven* means a shared journey drives it against the real
//! binary over this source's own row in `crates/onetaskgraph/tests/e2e/fixtures.rs`, and
//! `every_row_declares_exactly_what_its_plugin_reports` is what keeps this list and
//! [`capabilities`](TaskSource::capabilities) from parting.
//!
//! | Field | Verdict |
//! | --- | --- |
//! | `projects` | **Supported and proven.** A task's project is the issue it is a sub-issue of, and a listing scoped to one keeps the items filed under that issue. This is the field that was declared and then not applied, which silently returned another project's tasks. |
//! | `orphan_tasks` | **Supported and proven.** A task issue with no `parent` is in no project. |
//! | `filter_by_label` | **Supported and proven,** over the issue's own labels. |
//! | `filter_by_status` | **Supported and proven,** over the board's `Status` option and the issue's open or closed state, through this instance's own `status_mapping`. |
//! | `search_title` | **Supported and proven,** over `Issue.title`. |
//! | `search_content` | **Supported and proven,** over the visible body — the trailing metadata comment is not part of it. |
//! | `task_dependencies` | **Supported and proven,** in both directions: `blockedBy` and `blocking`. |
//! | `project_dependencies` | **Supported and proven,** in both directions, over the same two connections, because a project here is an issue. |
//! | `max_page_size` | **Supported and proven.** [`MAX_PAGE_SIZE`], GitHub's own connection maximum. |
//!
//! Nothing here is unsupported, and the three facts behind that are recorded below rather
//! than re-derived — because a reader who takes `Native` to mean *the remote service
//! filters* will read the uniform verdict as a lie.
//!
//! First, the plugin contract defines `Support::Native` as *the source applies this
//! predicate itself*, and says nothing about where it applies it. What the declaration
//! promises the engine is capability rule 1 — a predicate declared `Native` **is** applied
//! — so that the engine may push it down and apply nothing of its own.
//!
//! Second, this source can keep that promise for every predicate at no additional API
//! cost, because this source's own `board` walk already reads every page of the board
//! before it answers anything at all. That walk is what `get_task`, `labels` and every
//! listing already pay for; filtering the items it returns is in-process work over data
//! already in hand.
//!
//! Third, no predicate could be pushed into the API even if that were wanted:
//! `ProjectV2.items` takes `first` and `after` and offers no filter argument of any kind.
//! So there is no server-side filtering to declare, and — the other half of the same fact
//! — there is no predicate here that is genuinely unsupportable. Declaring one
//! `Unsupported` would make the engine compensate for work this source has already done,
//! and declaring `projects` native while ignoring the filter (which this source once did)
//! silently returns another project's tasks, because the engine trusts the declaration and
//! applies nothing locally.
//!
//! Filtering happens before paging, so a page of a filtered result is a page of the
//! survivors rather than the survivors of a page. Label and text matching answer the same
//! question the same way the local Markdown source's do, so one cross-source expectation
//! holds for both.
//!
//! <!-- llmlint: ignore[contracts_have_one_source_or_a_drift_gate] The declaration itself
//! has one source, `capabilities`, and the note above is the reasoning behind it rather
//! than a second copy of it: without the three facts recorded here a reader takes the
//! uniform `Native` for a lie and reverts it. The drift gate on the declaration is this
//! crate's own capabilities test, which pins every field of it against a fully spelled-out
//! `Capabilities` literal — a struct with no `Default`, so a field added to the contract
//! fails to compile there rather than going unasserted. -->
//! Required checks use only the local fixture server; the ignored credentialed lane
//! verifies the current schema, then drives every field of the table above against the
//! real board. It builds its own fixture there — two projects, one task filed under each,
//! one filed under neither, a label on one of the three and a closed status on another —
//! because that shape is what tells an honoured predicate from an ignored one: a board
//! holding a single project answers a project filter the same way whether or not this
//! source applies it, which is exactly how the defect above went unseen.
//!
//! That lane writes only to the board `GH_PROJECTS_OWNER` and `GH_PROJECTS_NUMBER` name,
//! and only into the repository `GH_PROJECTS_REPOSITORY` names, and skips — as it does
//! without `GH_PROJECTS_TOKEN` — when any of them is absent. Requiring both to be
//! nominated is what keeps a credentialed write lane off a board and a repository nobody
//! nominated; it never asks GitHub which project was updated most recently. Before it
//! starts, the lane also clears any item titled — and any repository label named — the way
//! it titles and names its own artifacts, which is self-healing after an interrupted run:
//! a process killed between its writes and its cleanup leaves artifacts the next run
//! removes.
#![deny(missing_docs)]

use std::collections::BTreeMap;
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use onetaskgraph_plugin_api::{
    Capabilities, Cursor, DependencyEdge, DependencyEndpoint, DependencyKind, DependencySupport,
    Direction, Health, ItemKind, ItemWrite, Label, LabelFilter, NativeId, Page, PageRequest,
    Project, ProjectFilter, ProjectQuery, Repository, SecretResolver, SourceError, SourceName,
    SourcePlugin, Status, StatusCategory, Support, Task, TaskQuery, TaskSource, TextFields,
    TextQuery, WriteSupport,
};
use reqwest::{Client, StatusCode, Url};
use schemars::{Schema, schema_for};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};

/// The registry name for this plugin.
pub const KIND: &str = "github-projects";
/// GitHub's maximum connection page size.
pub const MAX_PAGE_SIZE: u32 = 100;
/// Nested connection size which keeps GitHub's worst-case query below its node limit.
const NESTED_PAGE_SIZE: u32 = 50;

/// Exact GraphQL query documents issued by this plugin.
///
/// Keeping the production documents here lets the pinned-schema test validate the same
/// bytes that are sent to GitHub, rather than a test-only copy which could drift
/// independently. No document in this module writes the board itself, and none of them
/// names `updateProjectV2Field`.
pub mod graphql {
    /// Reads the board's fields and one page of its items.
    pub const BOARD: &str = r#"query($owner:String!,$number:Int!,$first:Int!,$after:String,$nestedFirst:Int!,$duplicates:Boolean!){
      owner:repositoryOwner(login:$owner){
        ... on ProjectV2Owner{projectV2(number:$number){...Board}}
      }
    } fragment Board on ProjectV2 { id title
      fields(first:$nestedFirst){nodes{
        ... on ProjectV2SingleSelectField{__typename id name options{id name}}
        ... on ProjectV2Field{__typename id name}
      }pageInfo{hasNextPage}}
      items(first:$first,after:$after){nodes{id fieldValues(first:$nestedFirst){nodes{
        ... on ProjectV2ItemFieldSingleSelectValue{name field{
          ... on ProjectV2SingleSelectField{id name options{id name}}
        }}
        ... on ProjectV2ItemFieldTextValue{text field{... on ProjectV2Field{id name}}}
        ... on ProjectV2ItemFieldLabelValue{labels(first:$nestedFirst){nodes{id name color}pageInfo{hasNextPage}}}
      }pageInfo{hasNextPage}} content{
        ... on Issue{__typename id title body url createdAt updatedAt state stateReason(enableDuplicate:$duplicates) repository{nameWithOwner} parent{id} subIssuesSummary{total} labels(first:$nestedFirst){nodes{id name color}pageInfo{hasNextPage}}}
        ... on PullRequest{__typename id}
        ... on DraftIssue{__typename id title body createdAt updatedAt}
      }} pageInfo{hasNextPage endCursor}}
    }"#;
    /// Resolves the configured repository's node id, which creating an issue requires.
    pub const REPOSITORY: &str = r#"query($owner:String!,$name:String!){repository(owner:$owner,name:$name){id nameWithOwner}}"#;
    /// Reads both dependency directions for one issue, with each far end's own kind.
    pub const ISSUE_DEPENDENCIES: &str = r#"query($id:ID!,$first:Int!,$after:String){node(id:$id){__typename
      ... on Issue{
        blockedBy(first:$first,after:$after){nodes{...Related}pageInfo{hasNextPage endCursor}}
        blocking(first:$first,after:$after){nodes{...Related}pageInfo{hasNextPage endCursor}}
      }}} fragment Related on Issue{id body parent{id} subIssuesSummary{total}}"#;
    /// Creates one issue in the configured repository.
    pub const CREATE_ISSUE: &str =
        r#"mutation($input:CreateIssueInput!){createIssue(input:$input){issue{id}}}"#;
    /// Puts an existing issue on the configured board.
    pub const ADD_TO_BOARD: &str = r#"mutation($input:AddProjectV2ItemByIdInput!){addProjectV2ItemById(input:$input){item{id}}}"#;
    /// Updates an issue's visible fields and its open or closed state in one call.
    pub const UPDATE_ISSUE: &str =
        r#"mutation($input:UpdateIssueInput!){updateIssue(input:$input){issue{id}}}"#;
    /// Updates an existing draft's user-visible fields.
    pub const UPDATE_DRAFT: &str = r#"mutation($input:UpdateProjectV2DraftIssueInput!){updateProjectV2DraftIssue(input:$input){draftIssue{id}}}"#;
    /// Updates a text or single-select value on one project item.
    pub const UPDATE_FIELD: &str = r#"mutation($input:UpdateProjectV2ItemFieldValueInput!){updateProjectV2ItemFieldValue(input:$input){projectV2Item{id}}}"#;
    /// Files one issue under another as a sub-issue, which is what project membership is.
    pub const ADD_SUB_ISSUE: &str =
        r#"mutation($input:AddSubIssueInput!){addSubIssue(input:$input){issue{id} subIssue{id}}}"#;
    /// Takes one issue back out of its parent.
    pub const REMOVE_SUB_ISSUE: &str = r#"mutation($input:RemoveSubIssueInput!){removeSubIssue(input:$input){issue{id} subIssue{id}}}"#;
    /// Adds GitHub's native issue blocked-by relationship.
    pub const ADD_BLOCKED_BY: &str = r#"mutation($input:AddBlockedByInput!){addBlockedBy(input:$input){issue{id} blockingIssue{id}}}"#;
    /// Removes one native issue blocked-by relationship.
    pub const REMOVE_BLOCKED_BY: &str = r#"mutation($input:RemoveBlockedByInput!){removeBlockedBy(input:$input){issue{id} blockingIssue{id}}}"#;
    /// Deletes one issue, which takes its board item with it.
    ///
    /// The engine sends this in one situation only: undoing a copy that could not finish,
    /// over the items that same copy created. Deleting the issue removes the board item
    /// too, so there is no second `deleteProjectV2Item` to keep in step with it.
    pub const DELETE_ISSUE: &str =
        r#"mutation($input:DeleteIssueInput!){deleteIssue(input:$input){repository{id}}}"#;
}

fn default_token_env() -> String {
    "GH_PROJECTS_TOKEN".to_owned()
}
fn default_endpoint() -> String {
    "https://api.github.com/graphql".to_owned()
}

/// Where one status category lands on this board.
///
/// `null` — an absent value — disables the category for this instance, and using a
/// disabled status is a refusal naming the status and the instance.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(untagged)]
pub enum StatusTargetConfig {
    /// The name of a `Status` single-select option already on the board.
    Column(ColumnName),
    /// A closed issue state, whose reason is what tells done from cancelled.
    Closed {
        /// The `IssueClosedStateReason` to close with.
        closed: ClosedState,
    },
}

/// The name of a `Status` single-select option on the board.
///
/// Validated on the way in rather than checked later, so a blank option name — which
/// nothing on a board can be — is a state this type cannot hold.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, schemars::JsonSchema)]
#[serde(try_from = "String")]
pub struct ColumnName(String);

impl ColumnName {
    /// The option name, as the board spells it.
    fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ColumnName {
    type Error = String;

    fn try_from(name: String) -> Result<Self, Self::Error> {
        if name.trim().is_empty() {
            return Err("a status_mapping option name cannot be blank".to_owned());
        }
        Ok(Self(name))
    }
}

/// The two closed states this product can mean.
///
/// GitHub's `IssueClosedStateReason` also spells `DUPLICATE`, which is neither finished
/// work nor abandoned work, so nothing here ever writes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ClosedState {
    /// `COMPLETED` — precisely done.
    Completed,
    /// `NOT_PLANNED` — precisely cancelled.
    NotPlanned,
}

impl ClosedState {
    const fn reason(self) -> &'static str {
        match self {
            Self::Completed => "COMPLETED",
            Self::NotPlanned => "NOT_PLANNED",
        }
    }
}

/// Configuration for one GitHub Projects v2 board.
#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct GitHubProjectsConfig {
    /// Login of the user or organization which owns the board.
    pub owner: String, // llmlint: ignore[invalid_states_unrepresentable] Schema DTO; `new` validates GitHub's owner grammar before private construction.
    /// The project number shown in the board's GitHub URL.
    pub project_number: u32, // llmlint: ignore[invalid_states_unrepresentable] Schema DTO; `new` bounds this to a positive GraphQL Int.
    /// `owner/name` of the one repository this source creates its issues in.
    ///
    /// A board has no repository of its own and `createIssue` requires one, so a write
    /// without this is refused naming the field. Reads never need it.
    pub repository: Option<String>, // llmlint: ignore[invalid_states_unrepresentable] Schema DTO; `new` validates the `owner/name` grammar before private construction.
    /// Environment variable containing a fine-grained token with Projects and Issues
    /// read/write plus Pull requests read-only access for every repository represented on
    /// the board.
    #[serde(default = "default_token_env")]
    pub token_env: String, // llmlint: ignore[invalid_states_unrepresentable] Schema DTO; `new` validates the environment-variable grammar.
    /// GraphQL endpoint. GitHub Enterprise installations may override it.
    #[serde(default = "default_endpoint")]
    pub endpoint: String, // llmlint: ignore[invalid_states_unrepresentable] Schema DTO; `new` converts it to the private validated `Url`.
    /// Per-instance mapping from a status category to where it lands on this board.
    ///
    /// A category this does not mention keeps its shipped default: `backlog` to
    /// "Backlog", `todo` to "Todo", `in-progress` to "In Progress", `done` to closed as
    /// completed, `cancelled` to closed as not planned, and `draft` and `unknown`
    /// disabled.
    #[serde(default)]
    pub status_mapping: BTreeMap<String, Option<StatusTargetConfig>>, // llmlint: ignore[invalid_states_unrepresentable] Schema DTO; `new` parses each key into a `StatusCategory` and reports an unknown one against this instance.
}

/// Factory for [`GitHubProjectsSource`].
#[derive(Debug, Clone, Copy, Default)]
pub struct Plugin;

impl SourcePlugin for Plugin {
    fn kind(&self) -> &'static str {
        KIND
    }
    fn config_schema(&self) -> Schema {
        schema_for!(GitHubProjectsConfig)
    }
    fn build(
        &self,
        name: &SourceName,
        config: &Value,
        secrets: &dyn SecretResolver,
    ) -> Result<Box<dyn TaskSource>, SourceError> {
        let config: GitHubProjectsConfig =
            serde_json::from_value(config.clone()).map_err(|e| SourceError::Config {
                message: format!("source {name}: {e}"),
            })?;
        let source =
            GitHubProjectsSource::new(name, config, secrets).map_err(|error| match error {
                SourceError::Config { message } => SourceError::Config {
                    message: format!("source {name}: {message}"),
                },
                SourceError::Auth { message } => SourceError::Auth {
                    message: format!("source {name}: {message}"),
                },
                other => other,
            })?;
        Ok(Box::new(source))
    }
}

/// Where a status category lands on this board, once configuration is resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
enum StatusTarget {
    /// Not usable against this instance.
    Disabled,
    /// The board's `Status` option of this name.
    Column(ColumnName),
    /// A closed issue, with the reason that says which closed it means.
    Closed(ClosedState),
}

/// Every status category, in the order the vocabulary declares them.
///
/// This list mirrors `StatusCategory`, so it carries its own drift gate rather than a
/// reviewer's attention: [`category_position`] is a wildcard-free match, so a variant
/// added to the shared vocabulary fails to compile until it is named there, and this
/// crate's suite reconciles this list against that enum's own derived schema, which is
/// generated from the variants rather than written beside them. The schema is what
/// catches a list left one short — a list checking only the positions it already holds
/// would pass while every mapping indexed by the new position panicked.
pub const CATEGORIES: [StatusCategory; 7] = [
    StatusCategory::Draft,
    StatusCategory::Backlog,
    StatusCategory::Todo,
    StatusCategory::InProgress,
    StatusCategory::Done,
    StatusCategory::Cancelled,
    StatusCategory::Unknown,
];

/// Where one category sits in [`CATEGORIES`]; see that list for what this pins.
#[must_use]
pub const fn category_position(category: StatusCategory) -> usize {
    match category {
        StatusCategory::Draft => 0,
        StatusCategory::Backlog => 1,
        StatusCategory::Todo => 2,
        StatusCategory::InProgress => 3,
        StatusCategory::Done => 4,
        StatusCategory::Cancelled => 5,
        StatusCategory::Unknown => 6,
    }
}

/// The spelling a status category is configured and reported under.
fn category_name(category: StatusCategory) -> &'static str {
    match category {
        StatusCategory::Draft => "draft",
        StatusCategory::Backlog => "backlog",
        StatusCategory::Todo => "todo",
        StatusCategory::InProgress => "in-progress",
        StatusCategory::Done => "done",
        StatusCategory::Cancelled => "cancelled",
        StatusCategory::Unknown => "unknown",
    }
}

/// A shipped default's option name.
///
/// The literals below are this file's own and non-blank, and they are validated by the
/// one constructor a configured name goes through rather than beside it.
fn shipped_column(name: &'static str) -> ColumnName {
    ColumnName::try_from(name.to_owned()).expect("a shipped default names a board option")
}

/// The shipped default for one category, before this instance's configuration.
fn shipped_default(category: StatusCategory) -> StatusTarget {
    match category {
        StatusCategory::Backlog => StatusTarget::Column(shipped_column("Backlog")),
        StatusCategory::Todo => StatusTarget::Column(shipped_column("Todo")),
        StatusCategory::InProgress => StatusTarget::Column(shipped_column("In Progress")),
        StatusCategory::Done => StatusTarget::Closed(ClosedState::Completed),
        StatusCategory::Cancelled => StatusTarget::Closed(ClosedState::NotPlanned),
        StatusCategory::Draft | StatusCategory::Unknown => StatusTarget::Disabled,
    }
}

/// This instance's complete category-to-target mapping, read in both directions.
///
/// One target per category, held at that category's own [`category_position`], so a
/// category missing from the mapping, named twice in it, or filed out of order is a
/// state this type cannot hold rather than one [`Self::target`] has to defend against.
#[derive(Debug, Clone)]
struct StatusMapping {
    targets: [StatusTarget; CATEGORIES.len()],
}

impl StatusMapping {
    fn resolve(
        configured: BTreeMap<String, Option<StatusTargetConfig>>,
        instance: &SourceName,
    ) -> Result<Self, SourceError> {
        let mut overrides: BTreeMap<&'static str, Option<StatusTargetConfig>> = BTreeMap::new();
        for (key, value) in configured {
            let category = CATEGORIES
                .iter()
                .find(|category| category_name(**category) == key)
                .ok_or_else(|| SourceError::Config {
                    message: format!(
                        "status_mapping names {key:?}, which is not a status category of source \
                         {instance}; the categories are {}",
                        CATEGORIES
                            .iter()
                            .map(|category| category_name(*category))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                })?;
            overrides.insert(category_name(*category), value);
        }
        // `CATEGORIES[position] == category` for every category — the crate's suite
        // asserts it — so mapping the list in order fills each category's own slot.
        let targets = CATEGORIES.map(|category| match overrides.remove(category_name(category)) {
            None => shipped_default(category),
            Some(None) => StatusTarget::Disabled,
            Some(Some(StatusTargetConfig::Column(option))) => StatusTarget::Column(option),
            Some(Some(StatusTargetConfig::Closed { closed })) => StatusTarget::Closed(closed),
        });
        let mapping = Self { targets };
        for (index, category) in CATEGORIES.into_iter().enumerate() {
            let StatusTarget::Column(option) = mapping.target(category) else {
                continue;
            };
            if let Some(other) = CATEGORIES[..index].iter().find(|earlier| {
                matches!(mapping.target(**earlier), StatusTarget::Column(name)
                    if name.as_str().eq_ignore_ascii_case(option.as_str()))
            }) {
                return Err(SourceError::Config {
                    message: format!(
                        "status_mapping of source {instance} sends both {} and {} to the board \
                         option {:?}; one option cannot read back as two categories",
                        category_name(*other),
                        category_name(category),
                        option.as_str()
                    ),
                });
            }
        }
        Ok(mapping)
    }

    fn target(&self, category: StatusCategory) -> &StatusTarget {
        &self.targets[category_position(category)]
    }

    /// The category a board option name reports, or `None` when nothing maps to it.
    fn category_of(&self, option: &str) -> Option<StatusCategory> {
        CATEGORIES.into_iter().find(|category| {
            matches!(self.target(*category), StatusTarget::Column(name)
                if name.as_str().eq_ignore_ascii_case(option))
        })
    }
}

/// The one repository this source creates issues in.
#[derive(Debug, Clone)]
struct RepositoryTarget {
    owner: String, // llmlint: ignore[invalid_states_unrepresentable] Private, constructed only after `owner/name` validation in `new`.
    name: String, // llmlint: ignore[invalid_states_unrepresentable] Private, constructed only after `owner/name` validation in `new`.
}

impl RepositoryTarget {
    fn parse(value: &str) -> Result<Self, SourceError> {
        let (owner, name) = value.split_once('/').ok_or_else(|| SourceError::Config {
            message: format!(
                "repository must be spelled owner/name; {value:?} names no repository"
            ),
        })?;
        if !valid_github_owner(owner) || !valid_github_repository_name(name) {
            return Err(SourceError::Config {
                message: format!(
                    "repository must be spelled owner/name with a GitHub login and one \
                     repository name; {value:?} is not"
                ),
            });
        }
        Ok(Self {
            owner: owner.to_owned(),
            name: name.to_owned(),
        })
    }

    fn origin(&self) -> String {
        format!("github.com/{}/{}", self.owner, self.name)
    }
}

/// A source which reads GitHub afresh for every operation.
pub struct GitHubProjectsSource {
    /// This source's configured name, used both to tell a far end naming this source
    /// from one naming a system it knows nothing about, and to name the instance a
    /// status refusal is about.
    name: SourceName,
    owner: String, // llmlint: ignore[invalid_states_unrepresentable] Private, constructed only by `new` after full GitHub-owner validation.
    project_number: u32, // llmlint: ignore[invalid_states_unrepresentable] Private, constructed only by `new` after GraphQL-Int validation.
    repository: Option<RepositoryTarget>,
    endpoint: Url,
    token: SecretString,
    credential_name: String, // llmlint: ignore[invalid_states_unrepresentable] Private diagnostic value constructed only after environment-name validation.
    statuses: StatusMapping,
    client: Client,
    /// Every item this source has created since it was built, in the order it created
    /// them.
    ///
    /// GitHub's `projectV2.items` is eventually consistent: an issue added to a board with
    /// `addProjectV2ItemById` is routinely absent from the very next read of that board.
    /// So a copy that created a task and then wrote the task that depends on it looked up
    /// the far end, did not find it, and refused with *"GitHub dependency item … was not
    /// found"* — naming an item that same run had just created. Three runs of one project
    /// copy failed that way, each leaving a different half of the project behind.
    ///
    /// This is what a board read is completed from: an item remembered here and absent
    /// from the read is added back, because the board really does hold it and only the
    /// read is behind. It is not a cache of a user's work — nothing is remembered that
    /// this process did not itself just write, it lives and dies with the process, and it
    /// is never consulted for an item this source did not create.
    created: Mutex<Vec<Resolved>>,
}

impl GitHubProjectsSource {
    /// Validate configuration and capture the named credential without exposing it.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::Config`] for a configuration this instance cannot use and
    /// [`SourceError::Auth`] when the named credential is missing or empty.
    pub fn new(
        name: &SourceName,
        config: GitHubProjectsConfig,
        secrets: &dyn SecretResolver,
    ) -> Result<Self, SourceError> {
        if !valid_github_owner(&config.owner) {
            return Err(SourceError::Config {
                message: "owner must be 1-39 ASCII letters, digits, or single hyphens, and cannot start or end with a hyphen".into(),
            });
        }
        if config.project_number == 0 || config.project_number > i32::MAX as u32 {
            return Err(SourceError::Config {
                message: format!("project_number must be between 1 and {}", i32::MAX),
            });
        }
        if !valid_environment_name(&config.token_env) {
            return Err(SourceError::Config {
                message: "token_env must be a valid environment-variable name".into(),
            });
        }
        let repository = config
            .repository
            .as_deref()
            .map(RepositoryTarget::parse)
            .transpose()?;
        let endpoint = Url::parse(&config.endpoint).map_err(|e| SourceError::Config {
            message: format!("endpoint is not a valid URL: {e}"),
        })?;
        if endpoint.scheme() != "https"
            && !(endpoint.scheme() == "http"
                && endpoint
                    .host_str()
                    .is_some_and(|h| h == "127.0.0.1" || h == "localhost" || h == "::1"))
        {
            return Err(SourceError::Config {
                message:
                    "endpoint must use HTTPS (HTTP is accepted only for a loopback test server)"
                        .into(),
            });
        }
        let token = secrets.get(&config.token_env).filter(|token| !token.expose_secret().trim().is_empty()).ok_or_else(|| SourceError::Auth {
            message: format!("environment variable {} is missing or empty; set it to a fine-grained GitHub token granting Projects and Issues read/write plus Pull requests read-only access for every repository represented on the board", config.token_env),
        })?;
        Ok(Self {
            name: name.clone(),
            owner: config.owner,
            project_number: config.project_number,
            repository,
            endpoint,
            token,
            credential_name: config.token_env,
            statuses: StatusMapping::resolve(config.status_mapping, name)?,
            client: Client::builder()
                .user_agent("onetaskgraph")
                .build()
                .map_err(|e| SourceError::Config {
                    message: format!("cannot build HTTP client: {e}"),
                })?,
            created: Mutex::new(Vec::new()),
        })
    }

    async fn graphql(&self, query: &str, variables: Value) -> Result<Value, SourceError> {
        let response = self
            .client
            .post(self.endpoint.clone())
            .bearer_auth(self.token.expose_secret())
            .json(&json!({"query": query, "variables": variables}))
            .send()
            .await
            .map_err(|e| SourceError::Unavailable {
                message: format!("GitHub GraphQL request failed: {e}"),
            })?;
        let status = response.status();
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok());
        let exhausted = response
            .headers()
            .get("x-ratelimit-remaining")
            .and_then(|v| v.to_str().ok())
            == Some("0");
        if status == StatusCode::TOO_MANY_REQUESTS || exhausted {
            return Err(SourceError::RateLimited {
                retry_after_seconds: retry_after,
            });
        }
        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            return Err(SourceError::Auth {
                message: format!(
                    "GitHub rejected the configured credential with HTTP {status}; grant it Projects and Issues read/write plus Pull requests read-only access for every repository represented on the board"
                ),
            });
        }
        if !status.is_success() {
            return Err(SourceError::Unavailable {
                message: format!("GitHub GraphQL returned HTTP {status}"),
            });
        }
        let body: Value = response.json().await.map_err(|e| SourceError::Malformed {
            message: format!("GitHub returned invalid JSON: {e}"),
        })?;
        let errors = body
            .get("errors")
            .map(|value| {
                value.as_array().ok_or_else(|| SourceError::Malformed {
                    message: "GitHub response errors is not an array".into(),
                })
            })
            .transpose()?;
        if let Some(errors) = errors.filter(|errors| !errors.is_empty()) {
            let messages = errors
                .iter()
                .filter_map(|e| e.get("message").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("; ");
            let message = if messages.is_empty() {
                "GitHub returned GraphQL errors".into()
            } else {
                messages
            };
            let normalized = message.to_ascii_lowercase();
            if normalized.contains("resource not accessible") || normalized.contains("scope") {
                return Err(SourceError::Auth {
                    message: format!(
                        "{message}; grant {} Projects and Issues read/write plus Pull requests read-only access for every repository represented on the board",
                        self.credential_name
                    ),
                });
            }
            return Err(SourceError::Refused { message });
        }
        body.get("data")
            .filter(|data| data.is_object())
            .cloned()
            .ok_or_else(|| SourceError::Malformed {
                message: "GitHub response has no data object".into(),
            })
    }

    // llmlint: ignore[boundary_inputs_validated] GitHub caps nested connections at 100 and
    // GraphQL cannot independently page them inside the outer item page. This source page is
    // deliberately bounded at that published maximum; the live drift journey exercises it.
    async fn board_page(
        &self,
        items_after: Option<&str>,
        items_first: u32,
    ) -> Result<Value, SourceError> {
        let data = self
            .graphql(
                graphql::BOARD,
                json!({"owner":self.owner,"number":self.project_number,
                       "first":items_first.min(MAX_PAGE_SIZE),"after":items_after,
                       "nestedFirst":NESTED_PAGE_SIZE,"duplicates":true}),
            )
            .await?;
        data.pointer("/owner/projectV2")
            .filter(|v| !v.is_null())
            .cloned()
            .ok_or_else(|| SourceError::Refused {
                message: format!(
                    "GitHub project {}/{} was not found or is not visible to the token",
                    self.owner, self.project_number
                ),
            })
    }

    /// Every item on the board, with the one board identity they all share.
    async fn board(&self) -> Result<Board, SourceError> {
        let mut after: Option<String> = None;
        let mut items = Vec::new();
        let mut board;
        loop {
            let page = self.board_page(after.as_deref(), MAX_PAGE_SIZE).await?;
            for item in page
                .pointer("/items/nodes")
                .and_then(Value::as_array)
                .ok_or_else(|| SourceError::Malformed {
                    message: "GitHub project items.nodes is not an array".into(),
                })?
            {
                if let Some(resolved) = self.resolve(item)? {
                    items.push(resolved);
                }
            }
            let info = page
                .pointer("/items/pageInfo")
                .ok_or_else(|| SourceError::Malformed {
                    message: "GitHub project items have no pageInfo".into(),
                })?;
            let has_next = required_bool(info, "hasNextPage")?;
            let next = has_next
                .then(|| required_str(info, "endCursor"))
                .transpose()?;
            board = page.clone();
            match next {
                Some(next) => {
                    validate_cursor_progress(after.as_deref(), next)?;
                    after = Some(next.to_owned());
                }
                None => break,
            }
        }
        for own in self.created()?.iter() {
            if !items.iter().any(|item| item.id == own.id) {
                items.push(own.clone());
            }
        }
        Ok(Board {
            id: required_str(&board, "id")?.to_owned(),
            fields: board.get("fields").cloned().unwrap_or(Value::Null),
            items,
        })
    }

    /// The items this source has created, for completing a board read that is behind.
    fn created(&self) -> Result<std::sync::MutexGuard<'_, Vec<Resolved>>, SourceError> {
        self.created.lock().map_err(|_| SourceError::Unavailable {
            message: "this source's record of what it created in this run was left \
                      inconsistent by an earlier failure; next: run the command again"
                .into(),
        })
    }

    /// One board item as this source reports it, or `None` for content it ignores.
    ///
    /// A pull request is neither a project nor a task — it is somebody's change, not a
    /// unit of plan — and an item whose content the token cannot see has nothing to
    /// report at all.
    fn resolve(&self, item: &Value) -> Result<Option<Resolved>, SourceError> {
        let content = item.get("content").ok_or_else(|| SourceError::Malformed {
            message: "GitHub project item is missing content".into(),
        })?;
        if content.is_null() {
            return Ok(None);
        }
        let content_kind = match required_str(content, "__typename")? {
            "Issue" => ContentKind::Issue,
            "DraftIssue" => ContentKind::DraftIssue,
            _ => return Ok(None),
        };
        let field_values = item
            .get("fieldValues")
            .ok_or_else(|| SourceError::Malformed {
                message: "GitHub project item is missing fieldValues".into(),
            })?;
        complete_connection(field_values, "project item field values")?;
        let nodes = field_values
            .get("nodes")
            .and_then(Value::as_array)
            .ok_or_else(|| SourceError::Malformed {
                message: "GitHub project item fieldValues.nodes is not an array".into(),
            })?;
        if let Some(labels) = content.get("labels") {
            complete_connection(labels, "content labels")?;
        }
        for field_value in nodes {
            if let Some(labels) = field_value.get("labels") {
                complete_connection(labels, "project item field labels")?;
            }
        }
        let (body, slot) = metadata_body(optional_str(content, "body")?.map(str::to_owned))?;
        let parent = optional_str(content.get("parent").unwrap_or(&Value::Null), "id")?
            .map(|id| NativeId(id.to_owned()));
        // A draft has no sub-issues to summarise, and GitHub's schema gives it no field
        // to read one from; it is a task, and never a project.
        let sub_issues = match content_kind {
            ContentKind::Issue => sub_issue_total(content)?,
            ContentKind::DraftIssue => 0,
        };
        let content_id = required_str(content, "id")?;
        let marked = ItemKind::from_metadata(&slot).map_err(|message| SourceError::Malformed {
            message: format!("GitHub issue {content_id}: {message}"),
        })?;
        // Being a sub-issue wins outright, and no marker overrides it: an issue filed
        // under a project is that project's task even when it has sub-issues of its own.
        let kind = if parent.is_some() {
            ItemKind::Task
        } else if sub_issues > 0 || marked == Some(ItemKind::Project) {
            ItemKind::Project
        } else {
            ItemKind::Task
        };
        let own_repository = content
            .pointer("/repository/nameWithOwner")
            .and_then(Value::as_str)
            .map(|origin| Repository::try_from(format!("github.com/{origin}")))
            .transpose()
            .map_err(|message| SourceError::Malformed { message })?;
        let repositories = if slot.contains_key(Repository::METADATA_KEY) {
            Repository::from_metadata(&slot)
                .map_err(|message| SourceError::Malformed { message })?
        } else {
            own_repository.clone().into_iter().collect()
        };
        Ok(Some(Resolved {
            item_id: required_str(item, "id")?.to_owned(),
            id: NativeId(content_id.to_owned()),
            content_kind,
            kind,
            title: required_str(content, "title")?.to_owned(),
            body: body.filter(|value| !value.is_empty()),
            status: self.status(item, content)?,
            labels: labels(content, nodes)?,
            parent,
            origin: text_field(nodes, ORIGIN_FIELD)?.filter(|value| !value.is_empty()),
            url: optional_str(content, "url")?.map(str::to_owned),
            created_at: optional_time(content, "createdAt")?,
            updated_at: optional_time(content, "updatedAt")?,
            own_repository,
            repositories,
            slot,
        }))
    }

    /// The status one board item reports.
    ///
    /// The closed state decides the category and the `Status` option decides the name, so
    /// a closed issue sitting in a "Shipped" column reports `done` named `Shipped`. A
    /// closed issue whose reason is `DUPLICATE` or `REOPENED` reports `Unknown`: a
    /// duplicate is not finished work, and calling it done is a lie the next copy would
    /// write back. `REOPENED`-while-closed is a state this source can never produce, so
    /// it is read permissively rather than refused — reads are faithful, and refusals
    /// belong on writes.
    fn status(&self, item: &Value, content: &Value) -> Result<Status, SourceError> {
        let nodes = item
            .pointer("/fieldValues/nodes")
            .and_then(Value::as_array)
            .expect("resolve validates fieldValues.nodes before mapping status");
        let option = nodes
            .iter()
            .find(|value| value.pointer("/field/name").and_then(Value::as_str) == Some("Status"))
            .map(|value| required_str(value, "name"))
            .transpose()?;
        let state = optional_str(content, "state")?;
        if state == Some("CLOSED") {
            let category = match optional_str(content, "stateReason")? {
                None | Some("COMPLETED") => StatusCategory::Done,
                Some("NOT_PLANNED") => StatusCategory::Cancelled,
                Some(_) => StatusCategory::Unknown,
            };
            let fallback = match category {
                StatusCategory::Done => "Done",
                StatusCategory::Cancelled => "Cancelled",
                _ => "Closed",
            };
            return Ok(Status {
                category,
                name: option.unwrap_or(fallback).to_owned(),
            });
        }
        let name = option.unwrap_or("Open").to_owned();
        Ok(Status {
            category: self
                .statuses
                .category_of(&name)
                .unwrap_or(StatusCategory::Unknown),
            name,
        })
    }

    /// The board Status option this write selects, or the refusal that says why not.
    ///
    /// For a column target the option is what the status *is*, so a board that has no such
    /// option is a refusal naming the status and the instance. For a closed target the
    /// issue's own state carries the category, and the option carries only the name a
    /// reader reports — so an option spelled the way this status is spelled is selected
    /// when the board has one, and nothing is refused when it does not.
    fn column_for(
        &self,
        board: &Board,
        status: &Status,
        target: &StatusTarget,
    ) -> Result<Option<(String, String)>, SourceError> {
        let (wanted, required) = match target {
            StatusTarget::Column(wanted) => (wanted.as_str(), true),
            StatusTarget::Closed(_) => (status.name.as_str(), false),
            StatusTarget::Disabled => return Ok(None),
        };
        let missing = |detail: &str| SourceError::Refused {
            message: format!(
                "status {} of source {} needs the board Status option {wanted:?}, and {detail};                  add that option to the board, or point status_mapping.{} of this source at one                  it has",
                category_name(status.category),
                self.name,
                category_name(status.category)
            ),
        };
        let Some(field) = Board::field(&board.fields, "Status")? else {
            return if required {
                Err(missing("this board has no Status field"))
            } else {
                Ok(None)
            };
        };
        if required_str(field, "__typename")? != "ProjectV2SingleSelectField" {
            return if required {
                Err(missing(
                    "this board's Status field is not a single-select field",
                ))
            } else {
                Ok(None)
            };
        }
        let option = field
            .get("options")
            .and_then(Value::as_array)
            .and_then(|options| {
                options.iter().find(|option| {
                    option
                        .get("name")
                        .and_then(Value::as_str)
                        .is_some_and(|name| name.eq_ignore_ascii_case(wanted))
                })
            });
        match option {
            None if required => Err(missing("this board does not have it")),
            None => Ok(None),
            Some(option) => Ok(Some((
                required_str(field, "id")?.to_owned(),
                required_str(option, "id")?.to_owned(),
            ))),
        }
    }

    /// This instance's target for a category, refusing one it has disabled.
    ///
    /// Nothing here mutates the board's option set to make room for a status. GitHub
    /// documents `UpdateProjectV2FieldInput.singleSelectOptions` as *"provided values
    /// overwrite existing options"*, so no addition is additive and a mistake destroys the
    /// field and every item's status.
    fn resolved_target(&self, category: StatusCategory) -> Result<StatusTarget, SourceError> {
        let target = self.statuses.target(category).clone();
        if target != StatusTarget::Disabled {
            return Ok(target);
        }
        Err(SourceError::Refused {
            message: if category == StatusCategory::Draft {
                format!(
                    "status draft is disabled for source {}: draft is incompatible with this \
                     integration because GitHub draft issues cannot have sub-issues, and this \
                     source stores a project's tasks as its issue's sub-issues",
                    self.name
                )
            } else {
                format!(
                    "status {} is disabled for source {}; set status_mapping.{} of this source \
                     to a board Status option name or to a closed state",
                    category_name(category),
                    self.name,
                    category_name(category)
                )
            },
        })
    }

    async fn set_item_field(
        &self,
        board_id: &str,
        item_id: &str,
        field_id: &str,
        value: Value,
    ) -> Result<(), SourceError> {
        let data = self
            .graphql(
                graphql::UPDATE_FIELD,
                json!({"input":{
                    "projectId":board_id,"itemId":item_id,"fieldId":field_id,"value":value
                }}),
            )
            .await?;
        let returned = data
            .pointer("/updateProjectV2ItemFieldValue/projectV2Item")
            .ok_or_else(|| SourceError::Malformed {
                message: "GitHub field update returned no project item".into(),
            })?;
        if required_str(returned, "id")? != item_id {
            return Err(SourceError::Malformed {
                message: "GitHub field update returned the wrong project item".into(),
            });
        }
        Ok(())
    }

    async fn native_dependency_ids(&self, id: &NativeId) -> Result<Vec<String>, SourceError> {
        let mut after: Option<String> = None;
        let mut ids = Vec::new();
        loop {
            let data = self
                .graphql(
                    graphql::ISSUE_DEPENDENCIES,
                    json!({"id":id.0,"first":MAX_PAGE_SIZE,"after":after}),
                )
                .await?;
            let connection =
                data.pointer("/node/blockedBy")
                    .ok_or_else(|| SourceError::Malformed {
                        message: "GitHub dependency response has no blockedBy connection".into(),
                    })?;
            ids.extend(
                connection
                    .get("nodes")
                    .and_then(Value::as_array)
                    .ok_or_else(|| SourceError::Malformed {
                        message: "GitHub dependency response nodes is not an array".into(),
                    })?
                    .iter()
                    .map(|value| required_str(value, "id").map(str::to_owned))
                    .collect::<Result<Vec<_>, _>>()?,
            );
            let next = next_cursor(connection)?;
            if let Some(next) = &next {
                validate_cursor_progress(after.as_deref(), &next.0)?;
            }
            after = next.map(|cursor| cursor.0);
            if after.is_none() {
                return Ok(ids);
            }
        }
    }

    async fn dependencies(
        &self,
        id: &NativeId,
        near_kind: ItemKind,
        direction: Direction,
        page: &PageRequest,
    ) -> Result<Page<DependencyEdge>, SourceError> {
        validate_page(page)?;
        let limit = page.limit.min(MAX_PAGE_SIZE) as usize;
        let cursor = page.cursor.as_ref().map(|c| c.0.as_str());
        let recorded = recorded_offset(cursor, direction)?;
        // Asked for even in the recorded phase, whose page reads nothing from the
        // connection: `__typename` is what says whether this item has a native
        // relationship at all, and that is what decides which far ends the reserved key is
        // allowed to hold.
        let data = self
            .graphql(
                graphql::ISSUE_DEPENDENCIES,
                json!({"id":id.0,"first":page.limit.min(MAX_PAGE_SIZE),
                       "after":if recorded.is_some() {None} else {cursor}}),
            )
            .await?;
        let node =
            data.get("node")
                .filter(|v| !v.is_null())
                .ok_or_else(|| SourceError::Refused {
                    message: format!(
                        "GitHub item {} was not found or does not support dependencies",
                        id.0
                    ),
                })?;
        let connection_name = match direction {
            Direction::DependsOn => "blockedBy",
            Direction::DependedOnBy => "blocking",
        };
        // A draft has neither `blockedBy` nor `blocking`, so nothing it depends on can be
        // named natively and the reserved key may hold any far end. An issue's connections
        // hold issues, and this source reads them at the near item's own level.
        let natively_names = (required_str(node, "__typename")? == "Issue").then_some(near_kind);
        if let Some(offset) = recorded {
            return Ok(recorded_page(
                self.recorded_edges(id, near_kind, direction, natively_names)
                    .await?,
                offset,
                limit,
            ));
        }
        if natively_names.is_none() {
            return Ok(recorded_page(
                self.recorded_edges(id, near_kind, direction, natively_names)
                    .await?,
                0,
                limit,
            ));
        }
        let connection = node
            .get(connection_name)
            .ok_or_else(|| SourceError::Malformed {
                message: "GitHub dependency response is missing its connection".into(),
            })?;
        let nodes = connection
            .get("nodes")
            .and_then(Value::as_array)
            .ok_or_else(|| SourceError::Malformed {
                message: "GitHub dependency response nodes is not an array".into(),
            })?;
        // `from` depends on `to`, always. GitHub spells the same relationship from either
        // end — `blockedBy` lists what this item waits on, `blocking` lists what waits on
        // it — so the near item is `from` in one direction and `to` in the other.
        let items = nodes
            .iter()
            .map(|value| {
                let related = NativeId(required_str(value, "id")?.into());
                let related_kind = related_kind(value)?;
                let (from, to) = match direction {
                    Direction::DependsOn => (
                        DependencyEndpoint::from_native(id.clone(), near_kind),
                        DependencyEndpoint::from_native(related, related_kind),
                    ),
                    Direction::DependedOnBy => (
                        DependencyEndpoint::from_native(related, related_kind),
                        DependencyEndpoint::from_native(id.clone(), near_kind),
                    ),
                };
                Ok(DependencyEdge {
                    from,
                    to,
                    kind: DependencyKind::Blocks,
                })
            })
            .collect::<Result<Vec<_>, SourceError>>()?;
        let mut next = next_cursor(connection)?;
        if let Some(next) = &next {
            validate_cursor_progress(cursor, &next.0)?;
        }
        if next.is_none()
            && !self
                .recorded_edges(id, near_kind, direction, natively_names)
                .await?
                .is_empty()
        {
            next = Some(Cursor(format!("{RECORDED_CURSOR}0")));
        }
        Ok(Page { items, next })
    }

    /// The edges this item records under [`DependencyEdge::RECORDED_KEY`], which is where
    /// a far end in another source has to live: no GitHub issue relationship can name one.
    ///
    /// Only forwards. The reverse of a recorded edge is derived from the far end, and this
    /// source never writes one down.
    ///
    /// The metadata lives in the item's own body slot, so reading it costs one board scan.
    /// That is why it happens once the native connection is spent rather than on every
    /// page.
    async fn recorded_edges(
        &self,
        id: &NativeId,
        near_kind: ItemKind,
        direction: Direction,
        natively_names: Option<ItemKind>,
    ) -> Result<Vec<DependencyEdge>, SourceError> {
        if direction != Direction::DependsOn {
            return Ok(Vec::new());
        }
        let Some(item) = self
            .board()
            .await?
            .items
            .into_iter()
            .find(|item| item.id == *id)
        else {
            return Ok(Vec::new());
        };
        DependencyEdge::recorded(&item.slot, id, near_kind, &self.name, natively_names)
            .map_err(|message| SourceError::Malformed { message })
    }

    /// The configured repository's node id, or the refusal naming the field it needs.
    async fn repository_id(&self) -> Result<String, SourceError> {
        let repository = self
            .repository
            .as_ref()
            .ok_or_else(|| SourceError::Refused {
                message: format!(
                    "source {} has no repository configured, and a GitHub Projects board has no \
                 repository of its own to create an issue in; set repository: owner/name on \
                 this source",
                    self.name
                ),
            })?;
        let data = self
            .graphql(
                graphql::REPOSITORY,
                json!({"owner":repository.owner,"name":repository.name}),
            )
            .await?;
        let node = data
            .get("repository")
            .filter(|value| !value.is_null())
            .ok_or_else(|| SourceError::Refused {
                message: format!(
                    "GitHub repository {}/{} was not found or is not visible to the token",
                    repository.owner, repository.name
                ),
            })?;
        Ok(required_str(node, "id")?.to_owned())
    }

    /// Create or update one board item, whichever kind it is.
    async fn write_item(
        &self,
        incoming: &Incoming<'_>,
        target: Option<&NativeId>,
        depends_on: &[DependencyEdge],
    ) -> Result<NativeId, SourceError> {
        let board = self.board().await?;
        let status_target = self.resolved_target(incoming.status.category)?;
        let column = self.column_for(&board, incoming.status, &status_target)?;
        let existing = target
            .map(|target| {
                board
                    .items
                    .iter()
                    .find(|item| item.id == *target)
                    .ok_or_else(|| SourceError::Refused {
                        message: format!("GitHub destination item {} was not found", target.0),
                    })
            })
            .transpose()?;
        let content_kind = existing.map_or(ContentKind::Issue, |item| item.content_kind);
        if content_kind == ContentKind::DraftIssue {
            if let StatusTarget::Closed(_) = status_target {
                return Err(SourceError::Refused {
                    message: format!(
                        "status {} of source {} closes the item's issue, and GitHub draft items \
                         have no open or closed state",
                        category_name(incoming.status.category),
                        self.name
                    ),
                });
            }
            if incoming.parent.is_some() {
                return Err(SourceError::Refused {
                    message: "GitHub draft items cannot be a project's sub-issue".into(),
                });
            }
        }
        match existing {
            Some(item) if content_kind == ContentKind::Issue => {
                if item.labels != incoming.labels {
                    return Err(SourceError::Refused {
                        message: "GitHub issue labels differ from the labels being written".into(),
                    });
                }
            }
            _ => {
                if !incoming.labels.is_empty() {
                    return Err(SourceError::Refused {
                        message: "GitHub items created by this destination carry no labels".into(),
                    });
                }
            }
        }

        let own_repository = match existing {
            Some(item) => item.own_repository.clone(),
            None => self
                .repository
                .as_ref()
                .map(|repository| Repository::try_from(repository.origin()))
                .transpose()
                .map_err(|message| SourceError::Config { message })?,
        };
        let (native, fallback) = self
            .partition_edges(&board, incoming.kind, content_kind, depends_on)
            .await?;
        let slot = slot_metadata(incoming, own_repository.as_ref(), &fallback);
        let body = compose_body(incoming.content, &slot)?;
        // Read before anything is created, for the reason the field below is: a value
        // this destination cannot store has to refuse, and refusing after `createIssue`
        // would leave an issue behind that nothing asked for. The engine writes a
        // qualified id here; a caller handing this key anything else is told so rather
        // than having it silently stored as no origin at all.
        // llmlint: ignore[boundary_inputs_validated, changed_behavior_has_e2e] The qualified id's syntax is the engine's and not this plugin's to police: `GlobalId` is deliberately absent from the contract crate because a plugin never sees a qualified id (AGENTS.md), no plugin crate may depend on the engine to parse one, and `docs/metadata.md` says the contents of this key are what no plugin constructs or interprets. What this boundary owns is whether the value is a string its text field can hold, and that is what it checks.
        let origin = match incoming.metadata.get(ORIGIN_KEY) {
            None => "",
            Some(Value::String(origin)) => origin.as_str(),
            Some(other) => {
                return Err(SourceError::Refused {
                    message: format!(
                        "{ORIGIN_KEY} holds a qualified id spelled as a string, and this item's \
                         is {other}"
                    ),
                });
            }
        };
        // Resolved before anything is created: a board that cannot carry the copy origin
        // has to refuse the write, and refusing it after `createIssue` would leave an
        // issue behind that nothing asked for.
        let origin_field = match Board::field(&board.fields, ORIGIN_FIELD)? {
            Some(field) => {
                if required_str(field, "__typename")? != "ProjectV2Field" {
                    return Err(SourceError::Refused {
                        message: format!(
                            "GitHub board source-owned {ORIGIN_FIELD} field is not a text field"
                        ),
                    });
                }
                Some(required_str(field, "id")?.to_owned())
            }
            None if incoming.metadata.contains_key(ORIGIN_KEY) => {
                return Err(SourceError::Refused {
                    message: format!(
                        "GitHub board has no source-owned {ORIGIN_FIELD} text field, and the \
                         item carries {ORIGIN_KEY}; add a text field named {ORIGIN_FIELD} to \
                         the board"
                    ),
                });
            }
            None => None,
        };

        let (content_id, item_id) = match existing {
            Some(item) => {
                self.update_existing(item, incoming, &body, &status_target)
                    .await?;
                (item.id.clone(), item.item_id.clone())
            }
            None => {
                self.create_and_file_issue(&board, incoming, &body, &status_target)
                    .await?
            }
        };

        // Creating an item here is several calls — `createIssue`, `addProjectV2ItemById`,
        // then each board field, the parent and the dependencies — and GitHub can fail at
        // any of them. Everything this source can refuse *before* the first of those is
        // already checked above, so what is left is GitHub itself failing part way. When it
        // does over an item this call created, the issue is taken back: a write that
        // refused must not leave an item behind that nobody asked for, and one that does
        // makes the retry create a second.
        let landed = self
            .finish_write(
                &board,
                incoming,
                &content_id,
                &item_id,
                content_kind,
                existing,
                origin_field.as_deref(),
                origin,
                column,
                &native,
            )
            .await;
        if let Err(error) = landed {
            if existing.is_none() {
                // Best effort, and the write's own failure is what the caller is told: a
                // refusal naming the tidy-up would hide why the write failed at all.
                let _ = self.delete_issue(&content_id).await;
            }
            return Err(error);
        }

        if existing.is_none() {
            // Remember it, so the next board read in this run holds it whether or not
            // GitHub's own has caught up. See the field's own documentation.
            let remembered = Resolved {
                item_id,
                id: content_id.clone(),
                content_kind,
                kind: incoming.kind,
                title: incoming.title.to_owned(),
                body: body.clone(),
                status: incoming.status.clone(),
                labels: incoming.labels.to_vec(),
                parent: incoming.parent.cloned(),
                origin: (!origin.is_empty()).then(|| origin.to_owned()),
                url: None,
                created_at: None,
                updated_at: None,
                own_repository,
                repositories: incoming.repositories.to_vec(),
                slot,
            };
            self.created()?.push(remembered);
        }
        Ok(content_id)
    }

    /// Everything a write does after the item exists: its board fields, its parent, and
    /// its dependencies.
    ///
    /// Split out of `write_item` so there is one place a failure past the point of no
    /// return is caught, rather than a tidy-up repeated at each `?` above.
    // llmlint: ignore[suppressions_justified] This is the tail of `write_item` lifted out
    // so there is one place a failure past the point of no return is caught, and its
    // arguments are exactly the values that tail already had in scope. Bundling them into a
    // struct would describe no concept — it would be "the arguments of this function" — and
    // would put the whole of `write_item`'s locals behind one more indirection.
    #[allow(clippy::too_many_arguments)]
    async fn finish_write(
        &self,
        board: &Board,
        incoming: &Incoming<'_>,
        content_id: &NativeId,
        item_id: &str,
        content_kind: ContentKind,
        existing: Option<&Resolved>,
        origin_field: Option<&str>,
        origin: &str,
        column: Option<(String, String)>,
        native: &[String],
    ) -> Result<(), SourceError> {
        if let Some(field_id) = origin_field {
            self.set_item_field(&board.id, item_id, field_id, json!({"text":origin}))
                .await?;
        }

        if let Some((field_id, option_id)) = column {
            self.set_item_field(
                &board.id,
                item_id,
                &field_id,
                json!({"singleSelectOptionId":option_id}),
            )
            .await?;
        }

        if content_kind == ContentKind::Issue {
            self.reparent(
                existing.and_then(|item| item.parent.clone()),
                content_id,
                incoming.parent,
            )
            .await?;
            self.reconcile_blocked_by(content_id, native).await?;
        }
        Ok(())
    }

    /// Delete one issue, which takes its board item with it.
    async fn delete_issue(&self, id: &NativeId) -> Result<(), SourceError> {
        let data = self
            .graphql(graphql::DELETE_ISSUE, json!({"input":{"issueId":id.0}}))
            .await?;
        data.pointer("/deleteIssue/repository")
            .filter(|value| !value.is_null())
            .ok_or_else(|| SourceError::Malformed {
                message: "GitHub issue deletion returned no repository".into(),
            })?;
        self.created()?.retain(|own| own.id != *id);
        Ok(())
    }

    /// Remove one item this copy created, so a copy that could not finish leaves the board
    /// as it found it.
    ///
    /// Deleting the issue takes its board item with it, so there is no second mutation to
    /// keep in step. An id the board does not hold is not an error: the item is already
    /// gone, which is the state this asks for.
    async fn delete_item(&self, id: &NativeId) -> Result<(), SourceError> {
        let board = self.board().await?;
        let Some(item) = board.items.iter().find(|item| item.id == *id) else {
            return Ok(());
        };
        if item.content_kind == ContentKind::DraftIssue {
            return Err(SourceError::Refused {
                message: format!(
                    "GitHub item {} is a draft, and this source removes an item by deleting \
                     its issue; next: remove it from the board by hand",
                    id.0
                ),
            });
        }
        let data = self
            .graphql(graphql::DELETE_ISSUE, json!({"input":{"issueId":id.0}}))
            .await?;
        data.pointer("/deleteIssue/repository")
            .filter(|value| !value.is_null())
            .ok_or_else(|| SourceError::Malformed {
                message: "GitHub issue deletion returned no repository".into(),
            })?;
        self.created()?.retain(|own| own.id != *id);
        Ok(())
    }

    /// Which far ends this item's own `blockedBy` relationship holds, and which it cannot.
    async fn partition_edges(
        &self,
        board: &Board,
        near_kind: ItemKind,
        near_content: ContentKind,
        depends_on: &[DependencyEdge],
    ) -> Result<(Vec<String>, Vec<DependencyEdge>), SourceError> {
        let mut native = Vec::new();
        let mut fallback = Vec::new();
        for edge in depends_on {
            let same_source = edge
                .to
                .source()
                .is_none_or(|source| source == self.name.as_str());
            // A qualified id's source segment runs to its *first* colon — `GlobalId` and
            // `DependencyEndpoint::source` both read it that way — and a native id may hold
            // colons of its own, so the far end is everything after that one separator.
            // Splitting at the last would truncate `work:urn:task:7` to `7`.
            let far_id = if edge.to.is_qualified() {
                edge.to
                    .id()
                    .split_once(':')
                    .map_or(edge.to.id(), |(_, native)| native)
            } else {
                edge.to.id()
            };
            let far = if same_source {
                Some(
                    board
                        .items
                        .iter()
                        .find(|item| item.id.0 == far_id)
                        .ok_or_else(|| SourceError::Refused {
                            message: format!("GitHub dependency item {far_id} was not found"),
                        })?,
                )
            } else {
                None
            };
            // The caller says which kind the far end is, and this board holds the far end
            // itself, so a disagreement is settled here rather than stored: recorded, the
            // wrong kind would read back as a cross-level edge that never existed; written
            // natively, it would name a relationship of a different level than the caller
            // asked for.
            if let Some(disagreeing) = far.filter(|far| far.kind != edge.to.kind) {
                return Err(SourceError::Refused {
                    message: format!(
                        "GitHub dependency item {far_id} is a {} of this board, and this item \
                         names it as a {}; record the kind it is",
                        disagreeing.kind.marker(),
                        edge.to.kind.marker()
                    ),
                });
            }
            // A draft has neither `blockedBy` nor `blocking`, so no edge of one is native
            // however the far end is spelled — and one classified native here would be
            // written nowhere at all, because a draft's native reconciliation never runs.
            let native_here = near_content == ContentKind::Issue
                && far.is_some_and(|far| {
                    far.content_kind == ContentKind::Issue && edge.to.kind == near_kind
                });
            if native_here {
                native.push(far_id.to_owned());
            } else {
                fallback.push(edge.clone());
            }
        }
        Ok((native, fallback))
    }

    async fn update_existing(
        &self,
        item: &Resolved,
        incoming: &Incoming<'_>,
        body: &Option<String>,
        status_target: &StatusTarget,
    ) -> Result<(), SourceError> {
        let (operation, input, pointer) = match item.content_kind {
            ContentKind::DraftIssue => (
                graphql::UPDATE_DRAFT,
                json!({"draftIssueId":item.id.0,"title":incoming.title,"body":body}),
                "/updateProjectV2DraftIssue/draftIssue",
            ),
            ContentKind::Issue => (
                graphql::UPDATE_ISSUE,
                json!({"id":item.id.0,"title":incoming.title,"body":body,
                       "stateInput":state_input(status_target)}),
                "/updateIssue/issue",
            ),
        };
        let data = self.graphql(operation, json!({"input":input})).await?;
        let returned = data
            .pointer(pointer)
            .ok_or_else(|| SourceError::Malformed {
                message: "GitHub item update returned no item".into(),
            })?;
        if required_str(returned, "id")? != item.id.0 {
            return Err(SourceError::Malformed {
                message: "GitHub item update returned the wrong item".into(),
            });
        }
        Ok(())
    }

    /// Creates one issue, files it on the board, and closes it when the status says so.
    ///
    /// Three calls rather than one: `createIssue` needs a repository and answers with an
    /// issue that is on no board, `addProjectV2ItemById` is what puts it there, and a
    /// closed status is a state of the issue rather than a field of the board item.
    async fn create_and_file_issue(
        &self,
        board: &Board,
        incoming: &Incoming<'_>,
        body: &Option<String>,
        status_target: &StatusTarget,
    ) -> Result<(NativeId, String), SourceError> {
        let repository_id = self.repository_id().await?;
        let data = self
            .graphql(
                graphql::CREATE_ISSUE,
                json!({"input":{
                    "repositoryId":repository_id,"title":incoming.title,"body":body
                }}),
            )
            .await?;
        let created = data
            .pointer("/createIssue/issue")
            .filter(|value| !value.is_null())
            .ok_or_else(|| SourceError::Malformed {
                message: "GitHub issue creation returned no issue".into(),
            })?;
        let content_id = NativeId(required_str(created, "id")?.to_owned());
        // The issue exists from here on, so a failure filing it on the board takes it
        // back: an issue in the repository that is on no board is an item nobody asked for
        // and nothing here would find again.
        let added = match self
            .graphql(
                graphql::ADD_TO_BOARD,
                json!({"input":{"projectId":board.id,"contentId":content_id.0}}),
            )
            .await
        {
            Ok(added) => added,
            Err(error) => {
                let _ = self.delete_issue(&content_id).await;
                return Err(error);
            }
        };
        let item = added
            .pointer("/addProjectV2ItemById/item")
            .filter(|value| !value.is_null())
            .ok_or_else(|| SourceError::Malformed {
                message: "GitHub board addition returned no project item".into(),
            })?;
        if let StatusTarget::Closed(_) = status_target {
            let closed = self
                .graphql(
                    graphql::UPDATE_ISSUE,
                    json!({"input":{"id":content_id.0,"stateInput":state_input(status_target)}}),
                )
                .await?;
            let returned =
                closed
                    .pointer("/updateIssue/issue")
                    .ok_or_else(|| SourceError::Malformed {
                        message: "GitHub item update returned no item".into(),
                    })?;
            if required_str(returned, "id")? != content_id.0 {
                return Err(SourceError::Malformed {
                    message: "GitHub item update returned the wrong item".into(),
                });
            }
        }
        Ok((content_id, required_str(item, "id")?.to_owned()))
    }

    /// Move one issue under the project it now belongs to, or out of the one it left.
    async fn reparent(
        &self,
        held: Option<NativeId>,
        child: &NativeId,
        wanted: Option<&NativeId>,
    ) -> Result<(), SourceError> {
        if held.as_ref() == wanted {
            return Ok(());
        }
        if let Some(held) = &held {
            self.sub_issue(graphql::REMOVE_SUB_ISSUE, held, child, "removeSubIssue")
                .await?;
        }
        if let Some(wanted) = wanted {
            self.sub_issue(graphql::ADD_SUB_ISSUE, wanted, child, "addSubIssue")
                .await?;
        }
        Ok(())
    }

    async fn sub_issue(
        &self,
        operation: &str,
        parent: &NativeId,
        child: &NativeId,
        root: &str,
    ) -> Result<(), SourceError> {
        let data = self
            .graphql(
                operation,
                json!({"input":{"issueId":parent.0,"subIssueId":child.0}}),
            )
            .await?;
        let issue =
            data.pointer(&format!("/{root}/issue"))
                .ok_or_else(|| SourceError::Malformed {
                    message: "GitHub sub-issue update returned no issue".into(),
                })?;
        let sub =
            data.pointer(&format!("/{root}/subIssue"))
                .ok_or_else(|| SourceError::Malformed {
                    message: "GitHub sub-issue update returned no sub-issue".into(),
                })?;
        if required_str(issue, "id")? != parent.0 || required_str(sub, "id")? != child.0 {
            return Err(SourceError::Malformed {
                message: "GitHub sub-issue update returned the wrong issues".into(),
            });
        }
        Ok(())
    }

    async fn reconcile_blocked_by(
        &self,
        content_id: &NativeId,
        native: &[String],
    ) -> Result<(), SourceError> {
        let current = self.native_dependency_ids(content_id).await?;
        for (operation, far_id) in current
            .iter()
            .filter(|id| !native.contains(id))
            .map(|id| (graphql::REMOVE_BLOCKED_BY, id))
            .chain(
                native
                    .iter()
                    .filter(|id| !current.contains(id))
                    .map(|id| (graphql::ADD_BLOCKED_BY, id)),
            )
        {
            let data = self
                .graphql(
                    operation,
                    json!({"input":{"issueId":content_id.0,"blockingIssueId":far_id}}),
                )
                .await?;
            let root = if operation == graphql::ADD_BLOCKED_BY {
                "addBlockedBy"
            } else {
                "removeBlockedBy"
            };
            let issue =
                data.pointer(&format!("/{root}/issue"))
                    .ok_or_else(|| SourceError::Malformed {
                        message: "GitHub dependency update returned no issue".into(),
                    })?;
            let blocker = data
                .pointer(&format!("/{root}/blockingIssue"))
                .ok_or_else(|| SourceError::Malformed {
                    message: "GitHub dependency update returned no blocking issue".into(),
                })?;
            if required_str(issue, "id")? != content_id.0 || required_str(blocker, "id")? != far_id
            {
                return Err(SourceError::Malformed {
                    message: "GitHub dependency update returned the wrong issues".into(),
                });
            }
        }
        Ok(())
    }
}

/// The board, and every item on it this source reports.
struct Board {
    id: String,
    fields: Value,
    items: Vec<Resolved>,
}

impl Board {
    fn field<'a>(fields: &'a Value, name: &str) -> Result<Option<&'a Value>, SourceError> {
        complete_connection(fields, "project fields")?;
        let nodes = fields
            .get("nodes")
            .and_then(Value::as_array)
            .ok_or_else(|| SourceError::Malformed {
                message: "GitHub project fields.nodes is not an array".into(),
            })?;
        Ok(nodes
            .iter()
            .find(|field| field.get("name").and_then(Value::as_str) == Some(name)))
    }
}

/// One board item, resolved into everything this source reports about it.
#[derive(Clone)]
struct Resolved {
    item_id: String,
    id: NativeId,
    content_kind: ContentKind,
    kind: ItemKind,
    title: String,
    body: Option<String>,
    status: Status,
    labels: Vec<Label>,
    parent: Option<NativeId>,
    // llmlint: ignore[invalid_states_unrepresentable] The write side's reason, read back: this is the engine's qualified id, taken out of a board text field and handed on untouched. A newtype here would have this plugin define the syntax of an id `docs/metadata.md` says no plugin ever constructs or interprets.
    origin: Option<String>,
    url: Option<String>,
    created_at: Option<DateTime<Utc>>,
    updated_at: Option<DateTime<Utc>>,
    own_repository: Option<Repository>,
    repositories: Vec<Repository>,
    slot: BTreeMap<String, Value>,
}

impl Resolved {
    /// The metadata a caller sees: their own keys, plus the copy origin this source keeps
    /// in a field of its own, and none of the three keys that are only an encoding.
    fn metadata(&self) -> BTreeMap<String, Value> {
        let mut metadata = self.slot.clone();
        metadata.remove(Repository::METADATA_KEY);
        metadata.remove(DependencyEdge::RECORDED_KEY);
        metadata.remove(ItemKind::METADATA_KEY);
        if let Some(origin) = &self.origin {
            metadata.insert(ORIGIN_KEY.to_owned(), Value::String(origin.clone()));
        }
        metadata
    }

    fn task(&self) -> Task {
        Task {
            id: self.id.clone(),
            title: self.title.clone(),
            content: self.body.clone(),
            status: self.status.clone(),
            labels: self.labels.clone(),
            project: self.parent.clone(),
            url: self.url.clone(),
            created_at: self.created_at,
            updated_at: self.updated_at,
            metadata: self.metadata(),
            repositories: self.repositories.clone(),
        }
    }

    fn project(&self) -> Project {
        Project {
            id: self.id.clone(),
            title: self.title.clone(),
            content: self.body.clone(),
            status: self.status.clone(),
            labels: self.labels.clone(),
            url: self.url.clone(),
            created_at: self.created_at,
            updated_at: self.updated_at,
            metadata: self.metadata(),
            repositories: self.repositories.clone(),
        }
    }
}

/// The item being written, in the one shape both write methods reach.
struct Incoming<'a> {
    kind: ItemKind,
    title: &'a str,
    content: Option<&'a str>,
    status: &'a Status,
    labels: &'a [Label],
    metadata: &'a BTreeMap<String, Value>,
    repositories: &'a [Repository],
    parent: Option<&'a NativeId>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ContentKind {
    DraftIssue,
    Issue,
}

/// Whether `labels` satisfies `filter`, matching by name, case-insensitively.
///
/// This is the local Markdown source's `labels_match`, spelled the same way on purpose:
/// the shared cross-source journeys assert one answer to one question, so two sources
/// that disagree about what "carries the label bug" means fail them.
fn labels_match(labels: &[Label], filter: &LabelFilter) -> bool {
    let holds = |name: &String| {
        labels
            .iter()
            .any(|label| label.name.eq_ignore_ascii_case(name))
    };
    (filter.any_of.is_empty() || filter.any_of.iter().any(holds))
        && filter.all_of.iter().all(holds)
        && !filter.none_of.iter().any(holds)
}

/// Whether `category` is one of `statuses`. An empty list is unfiltered rather than
/// "keeps nothing", which is what lets a `Vec<StatusCategory>` spell no filter at all.
fn status_matches(category: StatusCategory, statuses: &[StatusCategory]) -> bool {
    statuses.is_empty() || statuses.contains(&category)
}

/// Whether `title`/`content` satisfies `query`, matching case-insensitively.
///
/// `content` is the item's own prose — the body with this source's trailing metadata
/// comment already taken off — so a search never matches an encoding the author of the
/// issue never wrote.
fn text_matches(title: &str, content: Option<&str>, query: &TextQuery) -> bool {
    let terms = query.terms.to_lowercase();
    let in_title = title.to_lowercase().contains(&terms);
    let in_content = content.is_some_and(|body| body.to_lowercase().contains(&terms));
    match query.fields {
        TextFields::Title => in_title,
        TextFields::Content => in_content,
        TextFields::TitleOrContent => in_title || in_content,
    }
}

fn task_matches(task: &Task, query: &TaskQuery) -> bool {
    labels_match(&task.labels, &query.labels)
        && status_matches(task.status.category, &query.statuses)
        && match &query.project {
            ProjectFilter::Any => true,
            ProjectFilter::Orphans => task.project.is_none(),
            ProjectFilter::Is(id) => task.project.as_ref() == Some(id),
        }
        && query
            .text
            .as_ref()
            .is_none_or(|text| text_matches(&task.title, task.content.as_deref(), text))
}

fn project_matches(project: &Project, query: &ProjectQuery) -> bool {
    labels_match(&project.labels, &query.labels)
        && status_matches(project.status.category, &query.statuses)
        && query
            .text
            .as_ref()
            .is_none_or(|text| text_matches(&project.title, project.content.as_deref(), text))
}

#[async_trait::async_trait]
impl TaskSource for GitHubProjectsSource {
    fn kind(&self) -> &'static str {
        KIND
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            projects: Support::Native,
            orphan_tasks: Support::Native,
            filter_by_label: Support::Native,
            filter_by_status: Support::Native,
            search_title: Support::Native,
            search_content: Support::Native,
            task_dependencies: DependencySupport::BothDirections,
            project_dependencies: DependencySupport::BothDirections,
            max_page_size: MAX_PAGE_SIZE,
        }
    }
    async fn health(&self) -> Result<Health, SourceError> {
        let board = self.board_page(None, 1).await?;
        Ok(Health {
            reachable: true,
            detail: Some(format!(
                "reading GitHub project {}/{} ({})",
                self.owner,
                self.project_number,
                required_str(&board, "title")?
            )),
        })
    }
    async fn get_task(&self, id: &NativeId) -> Result<Option<Task>, SourceError> {
        Ok(self
            .board()
            .await?
            .items
            .iter()
            .find(|item| item.id == *id && item.kind == ItemKind::Task)
            .map(Resolved::task))
    }
    async fn get_project(&self, id: &NativeId) -> Result<Option<Project>, SourceError> {
        Ok(self
            .board()
            .await?
            .items
            .iter()
            .find(|item| item.id == *id && item.kind == ItemKind::Project)
            .map(Resolved::project))
    }
    async fn query_tasks(
        &self,
        query: &TaskQuery,
        page: &PageRequest,
    ) -> Result<Page<Task>, SourceError> {
        validate_page(page)?;
        // Filtered before paged: a page of a filtered result is a page of the survivors,
        // never the survivors of a page.
        let tasks = self
            .board()
            .await?
            .items
            .iter()
            .filter(|item| item.kind == ItemKind::Task)
            .map(Resolved::task)
            .filter(|task| task_matches(task, query))
            .collect();
        Ok(offset_page(
            tasks,
            numeric_cursor(page.cursor.as_ref())?,
            page.limit.min(MAX_PAGE_SIZE) as usize,
        ))
    }
    async fn query_projects(
        &self,
        query: &ProjectQuery,
        page: &PageRequest,
    ) -> Result<Page<Project>, SourceError> {
        validate_page(page)?;
        let projects = self
            .board()
            .await?
            .items
            .iter()
            .filter(|item| item.kind == ItemKind::Project)
            .map(Resolved::project)
            .filter(|project| project_matches(project, query))
            .collect();
        Ok(offset_page(
            projects,
            numeric_cursor(page.cursor.as_ref())?,
            page.limit.min(MAX_PAGE_SIZE) as usize,
        ))
    }
    async fn labels(&self, page: &PageRequest) -> Result<Page<Label>, SourceError> {
        validate_page(page)?;
        let offset = numeric_cursor(page.cursor.as_ref())?;
        let mut labels = self
            .board()
            .await?
            .items
            .into_iter()
            .flat_map(|item| item.labels)
            .fold(Vec::new(), |mut all, label| {
                if !all.iter().any(|x: &Label| x.id == label.id) {
                    all.push(label);
                }
                all
            });
        labels.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.0.cmp(&b.id.0)));
        Ok(offset_page(
            labels,
            offset,
            page.limit.min(MAX_PAGE_SIZE) as usize,
        ))
    }
    async fn task_dependencies(
        &self,
        id: &NativeId,
        direction: Direction,
        page: &PageRequest,
    ) -> Result<Page<DependencyEdge>, SourceError> {
        self.dependencies(id, ItemKind::Task, direction, page).await
    }
    async fn project_dependencies(
        &self,
        id: &NativeId,
        direction: Direction,
        page: &PageRequest,
    ) -> Result<Page<DependencyEdge>, SourceError> {
        self.dependencies(id, ItemKind::Project, direction, page)
            .await
    }

    fn writes(&self) -> WriteSupport {
        WriteSupport::Supported
    }

    async fn write_task(&self, write: &ItemWrite<Task>) -> Result<NativeId, SourceError> {
        self.write_item(
            &Incoming {
                kind: ItemKind::Task,
                title: &write.item.title,
                content: write.item.content.as_deref(),
                status: &write.item.status,
                labels: &write.item.labels,
                metadata: &write.item.metadata,
                repositories: &write.item.repositories,
                parent: write.item.project.as_ref(),
            },
            write.target.as_ref(),
            &write.depends_on,
        )
        .await
    }

    async fn write_project(&self, write: &ItemWrite<Project>) -> Result<NativeId, SourceError> {
        self.write_item(
            &Incoming {
                kind: ItemKind::Project,
                title: &write.item.title,
                content: write.item.content.as_deref(),
                status: &write.item.status,
                labels: &write.item.labels,
                metadata: &write.item.metadata,
                repositories: &write.item.repositories,
                parent: None,
            },
            write.target.as_ref(),
            &write.depends_on,
        )
        .await
    }

    async fn delete_task(&self, id: &NativeId) -> Result<(), SourceError> {
        self.delete_item(id).await
    }

    async fn delete_project(&self, id: &NativeId) -> Result<(), SourceError> {
        self.delete_item(id).await
    }
}

/// Where the recorded tail of a dependency walk resumes; see
/// [`GitHubProjectsSource::recorded_edges`].
const RECORDED_CURSOR: &str = "onetaskgraph.depends_on:";

/// The board text field this source keeps a copy's origin in.
///
/// Named after the key it holds, and held to that name by the guard below rather than by
/// a reader noticing.
const ORIGIN_FIELD: &str = "onetaskgraph.origin";

/// The metadata key that field holds.
///
/// The engine owns this key and spells it once as `GlobalId::ORIGIN_KEY`; a plugin never
/// constructs or interprets the qualified id it carries. This source names it only to
/// route it — a short, typed value belongs in a typed field rather than in the body slot
/// a caller's own prose shares.
///
/// Restated rather than imported, because no plugin crate may depend on the engine. What
/// keeps the two spellings one contract is `scripts/check-origin-key-spelling.sh`, a
/// target in `check`: it reads the engine's own literal and fails naming the file and the
/// line when a plugin's parts from it either way. Drift here has one symptom — a copy
/// that creates a second item every run instead of finding the one it wrote — and that is
/// too late to learn it.
const ORIGIN_KEY: &str = "onetaskgraph.origin";

/// Where a recorded tail resumes, refusing a cursor no walk in `direction` reported.
///
/// The reserved key holds forward edges and nothing else — the reverse of a recorded edge
/// is derived from the far end, never written down on the near item — so only a forward
/// walk ever reports one of these cursors. A reverse read carrying one is resuming a walk
/// it did not come from, and it is told so rather than answered with an empty page that
/// reads as a walk which ended.
fn recorded_offset(
    cursor: Option<&str>,
    direction: Direction,
) -> Result<Option<usize>, SourceError> {
    cursor
        .and_then(|cursor| cursor.strip_prefix(RECORDED_CURSOR))
        .map(|offset| {
            if direction != Direction::DependsOn {
                return Err(SourceError::Config {
                    message: format!(
                        "{RECORDED_CURSOR}{offset} resumes recorded forward edges, which a \
                         reverse dependency read never issues; resume it in the direction \
                         that reported it"
                    ),
                });
            }
            offset.parse().map_err(|_| SourceError::Config {
                message: format!("{RECORDED_CURSOR}{offset} is not a recorded-edge cursor"),
            })
        })
        .transpose()
}

fn recorded_page(edges: Vec<DependencyEdge>, offset: usize, limit: usize) -> Page<DependencyEdge> {
    let mut page = offset_page(edges, offset, limit.max(1));
    page.next = page
        .next
        .map(|cursor| Cursor(format!("{RECORDED_CURSOR}{}", cursor.0)));
    page
}

/// The kind of one issue reached through a dependency connection.
///
/// The same three questions the board scan asks, over the fields the dependency document
/// selects: a sub-issue is a task, and anything else with sub-issues or the marker is a
/// project.
fn related_kind(value: &Value) -> Result<ItemKind, SourceError> {
    let parent = optional_str(value.get("parent").unwrap_or(&Value::Null), "id")?;
    if parent.is_some() {
        return Ok(ItemKind::Task);
    }
    let (_, slot) = metadata_body(optional_str(value, "body")?.map(str::to_owned))?;
    let id = required_str(value, "id")?;
    let marked = ItemKind::from_metadata(&slot).map_err(|message| SourceError::Malformed {
        message: format!("GitHub issue {id}: {message}"),
    })?;
    let sub_issues = sub_issue_total(value)?;
    Ok(if sub_issues > 0 || marked == Some(ItemKind::Project) {
        ItemKind::Project
    } else {
        ItemKind::Task
    })
}

/// The `IssueStateUpdateInput` one status target asks for.
///
/// `stateInput` and `state` are mutually exclusive on `UpdateIssueInput`, and only this
/// one is ever sent. A non-terminal status always asks for `OPEN`, which is what reopens
/// a currently-closed issue: without that the item would read back `Unknown` and a copy
/// would report a change forever.
fn state_input(target: &StatusTarget) -> Value {
    match target {
        StatusTarget::Closed(reason) => json!({"value":"CLOSED","stateReason":reason.reason()}),
        StatusTarget::Column(_) | StatusTarget::Disabled => json!({"value":"OPEN"}),
    }
}

/// The metadata one write stores in the item's body slot.
///
/// The typed fields travel as themselves, so the three reserved keys are rebuilt here
/// rather than carried: the kind marker so an empty project stays readable, the
/// repository list only when it is not exactly the issue's own repository, and the far
/// ends no relationship here can name.
fn slot_metadata(
    incoming: &Incoming<'_>,
    own_repository: Option<&Repository>,
    fallback: &[DependencyEdge],
) -> BTreeMap<String, Value> {
    let mut metadata = incoming.metadata.clone();
    metadata.remove(ORIGIN_KEY);
    metadata.insert(
        ItemKind::METADATA_KEY.to_owned(),
        Value::String(incoming.kind.marker().to_owned()),
    );
    let derivable = own_repository
        .map(|own| incoming.repositories == [own.clone()])
        .unwrap_or(incoming.repositories.is_empty());
    if derivable {
        metadata.remove(Repository::METADATA_KEY);
    } else {
        metadata.insert(
            Repository::METADATA_KEY.to_owned(),
            Value::Array(
                incoming
                    .repositories
                    .iter()
                    .map(|repository| Value::String(repository.as_str().to_owned()))
                    .collect(),
            ),
        );
    }
    if fallback.is_empty() {
        metadata.remove(DependencyEdge::RECORDED_KEY);
    } else {
        metadata.insert(
            DependencyEdge::RECORDED_KEY.to_owned(),
            Value::Array(
                fallback
                    .iter()
                    .map(|edge| json!({"id":edge.to.id(),"kind":edge.to.kind}))
                    .collect(),
            ),
        );
    }
    metadata
}

fn labels(content: &Value, field_values: &[Value]) -> Result<Vec<Label>, SourceError> {
    let direct = optional_nodes(content.get("labels"), "content labels")?;
    let field = field_values
        .iter()
        .find_map(|value| value.get("labels"))
        .map(|labels| optional_nodes(Some(labels), "field labels"))
        .transpose()?
        .flatten();
    let labels = direct
        .into_iter()
        .flatten()
        .chain(field.into_iter().flatten())
        .map(|v| {
            Ok(Label {
                id: NativeId(required_str(v, "id")?.to_owned()),
                name: required_str(v, "name")?.to_owned(),
                color: optional_str(v, "color")?.map(str::to_owned),
            })
        })
        .collect::<Result<Vec<_>, SourceError>>()?
        .into_iter()
        .fold(Vec::new(), |mut labels, label| {
            if !labels.iter().any(|x: &Label| x.id == label.id) {
                labels.push(label);
            }
            labels
        });
    Ok(labels)
}

fn text_field(field_values: &[Value], name: &str) -> Result<Option<String>, SourceError> {
    let Some(node) = field_values
        .iter()
        .find(|node| node.pointer("/field/name").and_then(Value::as_str) == Some(name))
    else {
        return Ok(None);
    };
    Ok(optional_str(node, "text")?.map(str::to_owned))
}

fn valid_github_owner(owner: &str) -> bool {
    !owner.is_empty()
        && owner.len() <= 39
        && !owner.starts_with('-')
        && !owner.ends_with('-')
        && !owner.contains("--")
        && owner
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

/// GitHub's repository-name grammar: 1-100 ASCII letters, digits, `-`, `_` or `.`, and
/// neither of the two names a path segment already means.
fn valid_github_repository_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 100
        && name != "."
        && name != ".."
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn valid_environment_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

/// How many sub-issues one issue has.
///
/// `Issue.subIssuesSummary` is `SubIssuesSummary!` and its `total` is `Int!`, so an
/// absent or non-integer one is a response this source cannot read — and reading it as
/// zero would classify a project as a task, which is exactly the mistake the marker
/// exists to keep from happening quietly.
fn sub_issue_total(issue: &Value) -> Result<u64, SourceError> {
    let summary = issue
        .get("subIssuesSummary")
        .ok_or_else(|| SourceError::Malformed {
            message: "GitHub issue is missing subIssuesSummary".into(),
        })?;
    summary
        .get("total")
        .and_then(Value::as_u64)
        .ok_or_else(|| SourceError::Malformed {
            message: "GitHub issue subIssuesSummary.total is not an unsigned integer".into(),
        })
}

fn required_str<'a>(value: &'a Value, field: &str) -> Result<&'a str, SourceError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| SourceError::Malformed {
            message: format!("GitHub response is missing string field {field}"),
        })
}

/// The slot's delimiters, which `docs/metadata.md` settles once for every source that
/// needs one — Linear spells them too, in its own description field.
///
/// Restated rather than shared, because a plugin crate depends on the contract crate and
/// nothing else of this workspace. `scripts/check-metadata-slot-encoding.sh`, a target in
/// `check`, is what keeps the two one encoding: drift is otherwise quiet, since each
/// source round-trips its own writes perfectly well under its own spelling.
const METADATA_OPEN: &str = "<!-- onetaskgraph.metadata\n";
const METADATA_CLOSE: &str = "\n-->";

/// The visible body and the metadata slot at the end of it.
///
/// The encoding is the one `docs/metadata.md` settles for Linear, which is where its
/// reasons are. Only a comment at the very end is a slot; one in the middle is a person's
/// own content and is left alone.
fn metadata_body(
    body: Option<String>,
) -> Result<(Option<String>, BTreeMap<String, Value>), SourceError> {
    let Some(body) = body else {
        return Ok((None, BTreeMap::new()));
    };
    let Some(start) = body.rfind(METADATA_OPEN) else {
        return Ok((Some(body), BTreeMap::new()));
    };
    let encoded_start = start + METADATA_OPEN.len();
    let Some(relative_end) = body[encoded_start..].find(METADATA_CLOSE) else {
        return Err(SourceError::Malformed {
            message: "unterminated onetaskgraph metadata slot in GitHub issue body".into(),
        });
    };
    let encoded_end = encoded_start + relative_end;
    if !body[encoded_end + METADATA_CLOSE.len()..].trim().is_empty() {
        return Ok((Some(body), BTreeMap::new()));
    }
    let metadata = serde_json::from_str(&body[encoded_start..encoded_end]).map_err(|error| {
        SourceError::Malformed {
            message: format!(
                "invalid canonical JSON in GitHub issue onetaskgraph metadata slot: {error}"
            ),
        }
    })?;
    let visible = body[..start].trim_end();
    Ok(((!visible.is_empty()).then(|| visible.to_owned()), metadata))
}

fn compose_body(
    content: Option<&str>,
    metadata: &BTreeMap<String, Value>,
) -> Result<Option<String>, SourceError> {
    let visible = content.unwrap_or_default();
    if metadata.is_empty() {
        return Ok((!visible.is_empty()).then(|| visible.to_owned()));
    }
    let encoded = serde_json::to_string(metadata).map_err(|error| SourceError::Malformed {
        message: error.to_string(),
    })?;
    Ok(Some(if visible.is_empty() {
        format!("{METADATA_OPEN}{encoded}{METADATA_CLOSE}")
    } else {
        format!("{visible}\n\n{METADATA_OPEN}{encoded}{METADATA_CLOSE}")
    }))
}

fn required_bool(value: &Value, field: &str) -> Result<bool, SourceError> {
    value
        .get(field)
        .and_then(Value::as_bool)
        .ok_or_else(|| SourceError::Malformed {
            message: format!("GitHub response is missing boolean field {field}"),
        })
}
fn optional_str<'a>(value: &'a Value, field: &str) -> Result<Option<&'a str>, SourceError> {
    match value.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_str()
            .map(Some)
            .ok_or_else(|| SourceError::Malformed {
                message: format!("GitHub response field {field} is not a string or null"),
            }),
    }
}
fn optional_nodes<'a>(
    connection: Option<&'a Value>,
    name: &str,
) -> Result<Option<&'a Vec<Value>>, SourceError> {
    match connection {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .get("nodes")
            .and_then(Value::as_array)
            .map(Some)
            .ok_or_else(|| SourceError::Malformed {
                message: format!("GitHub {name}.nodes is not an array"),
            }),
    }
}
fn complete_connection(connection: &Value, name: &str) -> Result<(), SourceError> {
    let page_info = connection
        .get("pageInfo")
        .ok_or_else(|| SourceError::Malformed {
            message: format!("GitHub {name} has no pageInfo"),
        })?;
    if required_bool(page_info, "hasNextPage")? {
        return Err(SourceError::Malformed {
            message: format!(
                "GitHub {name} exceeds the supported nested connection size of {NESTED_PAGE_SIZE}"
            ),
        });
    }
    Ok(())
}
fn optional_time(value: &Value, field: &str) -> Result<Option<DateTime<Utc>>, SourceError> {
    optional_str(value, field)?
        .map(|timestamp| {
            timestamp.parse().map_err(|error| SourceError::Malformed {
                message: format!("GitHub response field {field} is not a timestamp: {error}"),
            })
        })
        .transpose()
}
fn validate_page(page: &PageRequest) -> Result<(), SourceError> {
    if page.limit == 0 {
        Err(SourceError::Config {
            message: "page limit must be at least 1".into(),
        })
    } else {
        Ok(())
    }
}
fn next_cursor(connection: &Value) -> Result<Option<Cursor>, SourceError> {
    let page = connection
        .get("pageInfo")
        .filter(|value| value.is_object())
        .ok_or_else(|| SourceError::Malformed {
            message: "GitHub connection is missing pageInfo".into(),
        })?;
    if required_bool(page, "hasNextPage")? {
        let cursor = required_str(page, "endCursor")?;
        validate_cursor_progress(None, cursor)?;
        Ok(Some(Cursor(cursor.into())))
    } else {
        Ok(None)
    }
}
fn validate_cursor_progress(previous: Option<&str>, next: &str) -> Result<(), SourceError> {
    if next.is_empty() || previous == Some(next) {
        Err(SourceError::Malformed {
            message: "GitHub pagination cursor is empty or did not advance".into(),
        })
    } else {
        Ok(())
    }
}
fn numeric_cursor(cursor: Option<&Cursor>) -> Result<usize, SourceError> {
    cursor.map_or(Ok(0), |c| {
        c.0.parse().map_err(|_| SourceError::Config {
            message: "page cursor is invalid".into(),
        })
    })
}
fn offset_page<T>(mut items: Vec<T>, offset: usize, limit: usize) -> Page<T> {
    if offset > items.len() {
        return Page::last(vec![]);
    }
    let tail = items.split_off(offset);
    let mut selected = tail;
    let next = (selected.len() > limit).then(|| Cursor((offset + limit).to_string()));
    selected.truncate(limit);
    Page {
        items: selected,
        next,
    }
}
