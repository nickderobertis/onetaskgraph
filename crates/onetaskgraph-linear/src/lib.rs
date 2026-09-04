//! A read/write source over Linear's published GraphQL API.
//!
//! Linear `Issue` maps to [`Task`], `Project` to [`Project`], `Document` to [`Document`],
//! `IssueLabel` and `ProjectLabel` to [`Label`], and `WorkflowState.name` is preserved
//! while its `type` (`backlog`, `unstarted`, `started`, `completed`, or `canceled`) maps to
//! the normalized status category. Issue `relations`/`inverseRelations` and
//! project relations provide native dependency traversal in both directions.
//!
//! Label, workflow-state, project, and orphan filters are sent in the
//! `issues(filter:)`/`projects(filter:)` variables. Pagination uses Relay `first` and
//! `after`.
//!
//! Every issue, project and document reports its own Linear web address as its
//! [`Location`], as a link rather than a path — the counterpart of a folder of Markdown
//! reporting the path of the file behind an item. It does not replace the `url` field
//! those types already carry; it is the same address said in the shape a reader can act on.
//!
//! # What this source declares, field by field
//!
//! One verdict per field of [`Capabilities`]. A field is *supported and proven* when this
//! source applies it and a shared journey drives it against the real binary; the shared
//! table is `crates/onetaskgraph/tests/e2e/fixtures.rs`, the journeys are beside it, and
//! `every_row_declares_exactly_what_its_plugin_reports` is what keeps this list and
//! [`capabilities`](TaskSource::capabilities) from parting.
//!
//! | Field | Verdict |
//! | --- | --- |
//! | `projects` | **Supported and proven.** `issues(filter:{project:{id:{eq:…}}})`. |
//! | `documents` | **Supported and proven.** Linear's own first-class `Document`, read through `documents(first:,after:,filter:)` and `document(id:)`, written through `documentCreate`/`documentUpdate` and taken back by `documentDelete`. See the ruling below on what a Linear document cannot hold. |
//! | `orphan_tasks` | **Supported and proven.** `issues(filter:{project:{null:true}})`. |
//! | `filter_by_label` | **Supported and proven.** `labels:{some:{name:{eqIgnoreCase:…}}}` for what an item must carry — one per label, gathered under `or:` where any one of them will do — and `labels:{every:{name:{neqIgnoreCase:…}}}` for what it must not. Linear's `StringComparator` has no case-insensitive list operator; see the note beside `filter`. |
//! | `filter_by_status` | **Supported and proven.** `state:{type:{in:[…]}}`, over the `WorkflowState.type` vocabulary the category maps to. |
//! | `search_title` | **Unsupported, and unimplemented** rather than a limit of the API. See the ruling below. |
//! | `search_content` | **Unsupported, and unimplemented** rather than a limit of the API. See the ruling below. |
//! | `task_dependencies` | **Supported and proven,** in both directions: `relations` and `inverseRelations`. |
//! | `project_dependencies` | **Supported and proven,** in both directions, by the project relations of the same shape. |
//! | `max_page_size` | **Supported and proven.** 250, Linear's own connection maximum; every read pages with Relay `first`/`after`. |
//!
//! ## Ruling: the two searches are unimplemented, not unsupportable
//!
//! Linear's published API *does* offer issue search — `searchIssues` is a documented
//! operation of it — so there is no property of the remote service that makes a title-only
//! or a body-only match impossible here. What is true today is narrower and is recorded as
//! such: no production operation in this crate sends one, so declaring either predicate
//! `Native` would break capability rule 1, and `Unsupported` is the only honest
//! declaration for the code that exists.
//!
//! The engine compensates correctly for both — it over-fetches and narrows, and the shared
//! journeys assert that this row returns the same rows every native row does with the plan
//! naming the engine — so the declaration is sound as well as honest. It is still a gap
//! rather than a limit, and reading it as a limit is what would leave it here forever.
//! Implementing it is tracked in `docs/follow-ups.md`.
//!
//! ## Ruling: a Linear document carries no label, and that is Linear's
//!
//! Unlike the two searches above, this one *is* a property of the remote service. The
//! types of Linear's published schema carrying a `labels` field are `Issue`, `Project`,
//! `Team`, `Initiative` and `Organization`; `Document` is not among them, re-observed
//! 2026-09-01 and pinned in `tests/fixtures/schema.graphql`. So this source reports a
//! document's labels as none and **refuses by name** a document write carrying one, rather
//! than dropping it or standing a slot up beside a first-class type. The shared journey
//! table's row says so, and the shared document journeys drive that claim.
//!
//! Two predicates therefore reach a fetched page rather than the `documents(filter:)`
//! variables, and both are still *applied* — which is what `Native` means here, and why
//! the declaration stays honest. Labels, for the reason above. And orphans, because
//! `DocumentFilter.project` is a `ProjectFilter` where `IssueFilter.project` is a
//! `NullableProjectFilter`: only the nullable one carries `null:`, so Linear cannot be
//! asked for the documents belonging to no project. The page-by-page walk asks for only
//! what is still owed, so neither predicate can make a read return more than the caller
//! asked for, and neither can drop a document the walk already fetched.
//!
//! Caller metadata is canonical JSON in a trailing
//! `<!-- onetaskgraph.metadata ... -->` Markdown comment in the item's description. The
//! visible description is returned unchanged without that slot. Writes put the same
//! canonical encoding back beside the visible description, and use Linear issue/project
//! relations for same-source dependencies. Only cross-source far ends use the reserved
//! `onetaskgraph.depends_on` metadata key.
//!
//! Fixture provenance is recorded in `tests/fixtures/README.md`. The live journey in
//! `tests/live.rs` drives every field of the table above against Linear itself: it builds its own fixture
//! on the scratch team `LINEAR_WRITE_TEAM` names — two projects, one issue filed under
//! each, one filed under neither, two labels and two workflow states — because that shape
//! is what tells an honoured predicate from an ignored one, and a workspace where every
//! issue carries the label answers a filter the same way either way. The two searches are
//! asserted as what they are declared: the wider set, unnarrowed. Everything the lane
//! creates it deletes whether its assertions passed or failed, and it clears residue named
//! the way it names its own before it starts. A failed live cleanup is reported as a test
//! failure and may require manual deletion from that scratch team.
#![deny(missing_docs)]

use chrono::{DateTime, Utc};
use onetaskgraph_plugin_api::{
    Capabilities, Cursor, DependencyEdge, DependencyEndpoint, DependencyKind, DependencySupport,
    Direction, Document, DocumentQuery, Health, ItemKind, ItemWrite, Label, LabelFilter, Location,
    NativeId, Page, PageRequest, Project, ProjectFilter, ProjectQuery, Repository, SecretResolver,
    SourceError, SourceName, SourcePlugin, Status, StatusCategory, Support, Task, TaskQuery,
    TaskSource, WriteSupport,
};
use schemars::{Schema, schema_for};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};

/// The plugin kind a `linear` source's `plugin:` field names.
pub const KIND: &str = "linear";
const DEFAULT_ENDPOINT: &str = "https://api.linear.app/graphql";

/// Exact GraphQL query documents issued by this plugin.
///
/// Fixture servers consume these constants so their recognized contract cannot drift
/// from the production requests.
pub mod graphql {
    /// Check the authenticated viewer.
    pub const VIEWER: &str = "query { viewer { id } }";
    /// Fetch one issue.
    pub const ISSUE: &str = "query($id:String!){ issue(id:$id){ id title description url createdAt updatedAt state{name type} labels{nodes{id name color}} project{id} } }";
    /// Fetch one project.
    pub const PROJECT: &str = "query($id:String!){ project(id:$id){ id name description url createdAt updatedAt status{name type} labels{nodes{id name color}} } }";
    /// List issues.
    pub const ISSUES: &str = "query($first:Int!,$after:String,$filter:IssueFilter){ issues(first:$first,after:$after,filter:$filter){ nodes{id title description url createdAt updatedAt state{name type} labels{nodes{id name color}} project{id}} pageInfo{hasNextPage endCursor} } }";
    /// List projects.
    pub const PROJECTS: &str = "query($first:Int!,$after:String,$filter:ProjectFilter){ projects(first:$first,after:$after,filter:$filter){ nodes{id name description url createdAt updatedAt status{name type} labels{nodes{id name color}}} pageInfo{hasNextPage endCursor} } }";
    /// List issue labels.
    pub const LABELS: &str = "query($first:Int,$after:String){ issueLabels(first:$first,after:$after){ nodes{id name color} pageInfo{hasNextPage endCursor} } }";
    /// Fetch issue dependency relations.
    pub const ISSUE_RELATIONS: &str = "query($id:String!,$first:Int!,$after:String){ issue(id:$id){ description relations(first:$first,after:$after){nodes{id type relatedIssue{id}} pageInfo{hasNextPage endCursor}} inverseRelations(first:$first,after:$after){nodes{id type issue{id}} pageInfo{hasNextPage endCursor}} } }";
    /// Fetch project dependency relations.
    pub const PROJECT_RELATIONS: &str = "query($id:String!,$first:Int!,$after:String){ project(id:$id){ description relations(first:$first,after:$after){nodes{id type relatedProject{id}} pageInfo{hasNextPage endCursor}} inverseRelations(first:$first,after:$after){nodes{id type project{id}} pageInfo{hasNextPage endCursor}} } }";
    /// Resolve the configured team key to Linear's backend id.
    pub const TEAM: &str =
        "query($key:String!){ teams(filter:{key:{eqIgnoreCase:$key}}){nodes{id}} }";
    /// Resolve an issue workflow-state display name.
    ///
    /// `$team` is an `ID!` and `$name` a `String!` because that is what each one's
    /// *location* declares, not because of what this source passes: both carry a Linear
    /// identifier string. `WorkflowStateFilter.team` is a `NullableTeamFilter`, whose `id`
    /// is an `IDComparator`, whose `eq` is an `ID`; the sibling `name` reaches a
    /// `StringComparator.eqIgnoreCase`, which is a `String`.
    ///
    /// That distinction is what the live lane was refused for on 2026-09-04, with HTTP 400
    /// and `Variable "$team" of type "String!" used in position expecting type "ID".`
    /// GraphQL admits a variable at a location only when the variable's type is the
    /// location's type or that type's non-null form, and `String` is not `ID` however the
    /// value is spelled — so `String!` there fails validation before any field is read,
    /// while `ID!` is the non-null form of the location's own type and is accepted.
    ///
    /// It reached Linear because a variable inside an inline filter literal is not a root
    /// argument, and the pinned-schema checks only compared root arguments. They now walk
    /// into these literals too, so this class of drift fails here rather than in the live
    /// lane.
    pub const ISSUE_STATE: &str = "query($name:String!,$team:ID!){ workflowStates(filter:{name:{eqIgnoreCase:$name},team:{id:{eq:$team}}}){nodes{id}} }";
    /// List the workspace's project statuses, so one can be resolved by display name.
    ///
    /// Unlike `teams`, `workflowStates` and the two label connections, Linear's
    /// `projectStatuses` accepts no `filter` argument: asking for one is refused outright
    /// with `Unknown argument "filter" on field "Query.projectStatuses"`. The display name
    /// is therefore matched locally over the whole connection, which a workspace holds few
    /// enough of to answer in one page.
    // llmlint: ignore[changed_behavior_has_e2e] The uncovered case the rule names — a status
    // on a later page — is not a test that is missing but a document this repository has no
    // evidence Linear would accept: `tests/fixtures/schema.graphql` pins `after` alone,
    // because Linear's own refusal is where that correction came from, and its
    // `ProjectStatusConnection` declares `nodes` and no `pageInfo`. Selecting a cursor field
    // to page on would fail `pinned_schema_checks_selected_fields_arguments_and_fixture_keys`
    // here and risk, against Linear, the same `GRAPHQL_VALIDATION_FAILED` this document was
    // changed to stop sending. Reading one page is not what changed either: `teams`,
    // `workflowStates` and `projectLabels` resolve a display name through the same `one_id`
    // over the same unpaged connections, and did before this change. What did change is
    // driven end to end — the CLI journey
    // `linear_project_and_task_copies_write_native_relations_and_record_only_cross_source_edges`
    // copies a project whose status is resolved this way, and
    // `a_project_status_is_matched_locally_because_linear_narrows_that_connection_for_nobody`
    // holds the match, the ambiguity and the absence against a real HTTP server.
    pub const PROJECT_STATUS: &str = "query{ projectStatuses{nodes{id name}} }";
    /// Resolve an issue-label display name.
    pub const ISSUE_LABEL: &str =
        "query($name:String!){ issueLabels(filter:{name:{eqIgnoreCase:$name}}){nodes{id}} }";
    /// Resolve a project-label display name.
    pub const PROJECT_LABEL: &str =
        "query($name:String!){ projectLabels(filter:{name:{eqIgnoreCase:$name}}){nodes{id}} }";
    /// Create an issue.
    pub const ISSUE_CREATE: &str =
        "mutation($input:IssueCreateInput!){ issueCreate(input:$input){success issue{id}} }";
    /// Update an issue.
    pub const ISSUE_UPDATE: &str = "mutation($id:String!,$input:IssueUpdateInput!){ issueUpdate(id:$id,input:$input){success issue{id}} }";
    /// Create a project.
    pub const PROJECT_CREATE: &str =
        "mutation($input:ProjectCreateInput!){ projectCreate(input:$input){success project{id}} }";
    /// Update a project.
    pub const PROJECT_UPDATE: &str = "mutation($id:String!,$input:ProjectUpdateInput!){ projectUpdate(id:$id,input:$input){success project{id}} }";
    /// Create a native issue dependency.
    pub const ISSUE_RELATION_CREATE: &str = "mutation($input:IssueRelationCreateInput!){ issueRelationCreate(input:$input){success issueRelation{id}} }";
    /// Create a native project dependency.
    pub const PROJECT_RELATION_CREATE: &str = "mutation($input:ProjectRelationCreateInput!){ projectRelationCreate(input:$input){success projectRelation{id}} }";
    /// Delete a native issue dependency before replacing its full edge set.
    pub const ISSUE_RELATION_DELETE: &str =
        "mutation($id:String!){ issueRelationDelete(id:$id){success} }";
    /// Delete a native project dependency before replacing its full edge set.
    pub const PROJECT_RELATION_DELETE: &str =
        "mutation($id:String!){ projectRelationDelete(id:$id){success} }";
    /// Delete an issue, so a copy that could not finish can take back what it created.
    pub const ISSUE_DELETE: &str = "mutation($id:String!){ issueDelete(id:$id){success} }";
    /// Delete a project, for the same reason and on the same terms.
    pub const PROJECT_DELETE: &str = "mutation($id:String!){ projectDelete(id:$id){success} }";
    /// Fetch one document.
    pub const DOCUMENT: &str = "query($id:String!){ document(id:$id){ id title content url createdAt updatedAt project{id} } }";
    /// List documents.
    ///
    /// `first` is an `Int` rather than an `Int!` because that is what Linear's `documents`
    /// connection declares, unlike its `issues` one.
    pub const DOCUMENTS: &str = "query($first:Int,$after:String,$filter:DocumentFilter){ documents(first:$first,after:$after,filter:$filter){ nodes{id title content url createdAt updatedAt project{id}} pageInfo{hasNextPage endCursor} } }";
    /// Create a document.
    pub const DOCUMENT_CREATE: &str = "mutation($input:DocumentCreateInput!){ documentCreate(input:$input){success document{id}} }";
    /// Update a document.
    pub const DOCUMENT_UPDATE: &str = "mutation($id:String!,$input:DocumentUpdateInput!){ documentUpdate(id:$id,input:$input){success document{id}} }";
    /// Delete a document, so a copy that could not finish can take back what it created.
    pub const DOCUMENT_DELETE: &str = "mutation($id:String!){ documentDelete(id:$id){success} }";
}

use graphql::{
    DOCUMENT, DOCUMENTS, ISSUE, ISSUE_RELATIONS, ISSUES, LABELS, PROJECT, PROJECT_RELATIONS,
    PROJECTS, VIEWER,
};

/// Configuration contains only the credential variable's name, never its value.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct LinearConfig {
    /// Environment variable resolved by the host.
    #[schemars(with = "String")]
    api_key_env: EnvName,
    /// Linear team key/id used to narrow reads and required for item writes.
    #[schemars(with = "Option<String>")]
    team: Option<Team>,
    /// GraphQL endpoint override, primarily for fixture servers.
    #[schemars(with = "String")]
    endpoint: Endpoint,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(try_from = "String")]
struct EnvName(String);
impl TryFrom<String> for EnvName {
    type Error = String;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        let mut bytes = value.bytes();
        if bytes
            .next()
            .is_some_and(|byte| byte == b'_' || byte.is_ascii_uppercase())
            && bytes.all(|byte| byte == b'_' || byte.is_ascii_uppercase() || byte.is_ascii_digit())
        {
            Ok(Self(value))
        } else {
            Err("must be an uppercase environment-variable name".into())
        }
    }
}
#[derive(Debug, Clone, Deserialize)]
#[serde(try_from = "String")]
struct Team(String);
impl TryFrom<String> for Team {
    type Error = String;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.trim().is_empty() {
            Err("must not be empty".into())
        } else {
            Ok(Self(value))
        }
    }
}
#[derive(Debug, Clone, Deserialize)]
#[serde(try_from = "String")]
struct Endpoint(String);
impl TryFrom<String> for Endpoint {
    type Error = String;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        let url = reqwest::Url::parse(&value).map_err(|e| e.to_string())?;
        if matches!(url.scheme(), "http" | "https") {
            Ok(Self(value))
        } else {
            Err("must use http or https".into())
        }
    }
}

impl Default for LinearConfig {
    fn default() -> Self {
        Self {
            api_key_env: EnvName("LINEAR_API_KEY".into()),
            team: None,
            endpoint: Endpoint(DEFAULT_ENDPOINT.into()),
        }
    }
}

/// The Linear plugin factory.
#[derive(Debug, Clone, Copy, Default)]
pub struct Plugin;

impl SourcePlugin for Plugin {
    fn kind(&self) -> &'static str {
        KIND
    }
    fn config_schema(&self) -> Schema {
        schema_for!(LinearConfig)
    }
    fn build(
        &self,
        name: &SourceName,
        config: &Value,
        secrets: &dyn SecretResolver,
    ) -> Result<Box<dyn TaskSource>, SourceError> {
        let config: LinearConfig =
            serde_json::from_value(config.clone()).map_err(|e| SourceError::Config {
                message: format!("source {name}: {e}"),
            })?;
        let key = secrets
            .get(&config.api_key_env.0)
            .filter(|v| !v.expose_secret().trim().is_empty())
            .ok_or_else(|| SourceError::Auth {
                message: format!("set environment variable {}", config.api_key_env.0),
            })?;
        Ok(Box::new(LinearSource {
            client: reqwest::Client::new(),
            endpoint: config.endpoint,
            key,
            team: config.team,
            name: name.clone(),
        }))
    }
}

struct LinearSource {
    client: reqwest::Client,
    endpoint: Endpoint,
    key: SecretString,
    team: Option<Team>,
    /// This source's configured name, kept for one comparison: a far end recorded as
    /// `<this name>:<native>` is a Linear item Linear itself relates, so the reserved key
    /// is refused for it exactly as a bare id of the same kind is.
    name: SourceName,
}
#[derive(Clone, Copy)]
enum WriteKind {
    Task,
    Project,
}
enum Lookup<'a> {
    Team(&'a str),
    IssueState { name: &'a str, team: &'a NativeId },
    ProjectStatus(&'a str),
    IssueLabel(&'a str),
    ProjectLabel(&'a str),
}
impl Lookup<'_> {
    fn query(&self) -> &'static str {
        match self {
            Self::Team(_) => graphql::TEAM,
            Self::IssueState { .. } => graphql::ISSUE_STATE,
            Self::ProjectStatus(_) => graphql::PROJECT_STATUS,
            Self::IssueLabel(_) => graphql::ISSUE_LABEL,
            Self::ProjectLabel(_) => graphql::PROJECT_LABEL,
        }
    }
    fn connection(&self) -> &'static str {
        match self {
            Self::Team(_) => "teams",
            Self::IssueState { .. } => "workflowStates",
            Self::ProjectStatus(_) => "projectStatuses",
            Self::IssueLabel(_) => "issueLabels",
            Self::ProjectLabel(_) => "projectLabels",
        }
    }
    fn diagnostic(&self) -> String {
        match self {
            Self::Team(_) => "configured team".into(),
            Self::IssueState { name, .. } => format!("workflow state {name:?}"),
            Self::ProjectStatus(name) => format!("project status {name:?}"),
            Self::IssueLabel(name) | Self::ProjectLabel(name) => format!("label {name:?}"),
        }
    }
    fn variables(&self) -> Value {
        match self {
            Self::Team(key) => json!({"key":key}),
            Self::IssueState { name, team } => json!({"name":name,"team":team.0}),
            Self::IssueLabel(name) | Self::ProjectLabel(name) => json!({"name":name}),
            // `PROJECT_STATUS` names nothing, for the reason recorded on that document.
            Self::ProjectStatus(_) => json!({}),
        }
    }
    /// The display name `one_id` matches locally, for the one lookup whose connection
    /// Linear will not narrow server-side.
    fn local_name(&self) -> Option<&str> {
        match self {
            Self::ProjectStatus(name) => Some(name),
            _ => None,
        }
    }
}
#[derive(Clone, Copy)]
enum MutationRoot {
    IssueCreate,
    IssueUpdate,
    ProjectCreate,
    ProjectUpdate,
    IssueRelationCreate,
    ProjectRelationCreate,
    IssueRelationDelete,
    ProjectRelationDelete,
    IssueDelete,
    ProjectDelete,
    DocumentCreate,
    DocumentUpdate,
    DocumentDelete,
}
impl MutationRoot {
    fn as_str(self) -> &'static str {
        match self {
            Self::IssueCreate => "issueCreate",
            Self::IssueUpdate => "issueUpdate",
            Self::ProjectCreate => "projectCreate",
            Self::ProjectUpdate => "projectUpdate",
            Self::IssueRelationCreate => "issueRelationCreate",
            Self::ProjectRelationCreate => "projectRelationCreate",
            Self::IssueRelationDelete => "issueRelationDelete",
            Self::ProjectRelationDelete => "projectRelationDelete",
            Self::IssueDelete => "issueDelete",
            Self::ProjectDelete => "projectDelete",
            Self::DocumentCreate => "documentCreate",
            Self::DocumentUpdate => "documentUpdate",
            Self::DocumentDelete => "documentDelete",
        }
    }
}

#[derive(Deserialize)]
struct Envelope {
    // llmlint: ignore[invalid_states_unrepresentable] One transport envelope carries eight distinct GraphQL data shapes; each operation immediately validates its own complete mapper into typed plugin-api values, so malformed external data cannot cross the plugin boundary and a union here would duplicate every query response solely inside transport code.
    data: Option<Value>,
    #[serde(default)]
    errors: Vec<GqlError>,
}
#[derive(Deserialize)]
struct GqlError {
    message: String,
    // Held raw rather than typed, for two reasons. Linear puts the whole of *why* it
    // refused in here — `message` is a category name like `Argument Validation Error`,
    // which named neither the field nor the value when the live project-relation write
    // was refused by it — so a refusal carries this verbatim and a reader diagnoses from
    // it. And a typed shape with a required `code` fails the whole envelope's
    // deserialization when Linear sends extensions without one, turning a refusal this
    // source could explain into an unexplained malformed response.
    extensions: Option<Value>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GqlExtensions {
    code: GqlErrorCode,
    retry_after: Option<u64>,
}
impl GqlError {
    /// The rate-limit shape of [`Self::extensions`], when it has one.
    fn coded(&self) -> Option<GqlExtensions> {
        self.extensions
            .as_ref()
            .and_then(|value| serde_json::from_value(value.clone()).ok())
    }
    /// Everything Linear said about this refusal, on one line and cut to [`SAID_LIMIT`].
    fn said(&self) -> String {
        match &self.extensions {
            Some(extensions) => elided(&format!("{}: {extensions}", self.message)),
            None => elided(&self.message),
        }
    }
}
#[derive(Deserialize)]
enum GqlErrorCode {
    #[serde(rename = "RATELIMITED", alias = "RATE_LIMITED")]
    RateLimited,
    #[serde(other)]
    Other,
}

/// How much of a failed response's body a refusal carries.
///
/// Enough for Linear's own error envelope, which is one or two sentences naming the field
/// or argument it would not accept, and short enough that a proxy's HTML error page does
/// not become the whole message.
const SAID_LIMIT: usize = 400;

/// `said` made safe to put in a message: one line of printable text, cut to [`SAID_LIMIT`].
///
/// A failed response's body is whatever answered — Linear's error envelope, or an HTML
/// page from a proxy in front of it — and this message is written to a terminal. So every
/// control character goes, escape sequences with them, and each run of whitespace becomes
/// one space: a body cannot move the cursor, repaint the line or hide the rest of the
/// diagnostic behind itself. Cut by characters rather than bytes, because slicing UTF-8
/// mid-codepoint would panic inside the path that exists to explain a failure.
fn elided(said: &str) -> String {
    let mut printable = String::new();
    let mut spaced = true;
    for character in said.chars() {
        if character.is_control() || character.is_whitespace() {
            if !spaced {
                printable.push(' ');
                spaced = true;
            }
            continue;
        }
        printable.push(character);
        spaced = false;
    }
    let printable = printable.trim_end();
    if printable.chars().count() <= SAID_LIMIT {
        return printable.to_owned();
    }
    let kept: String = printable.chars().take(SAID_LIMIT).collect();
    format!("{kept}…")
}

impl LinearSource {
    // llmlint: ignore[invalid_states_unrepresentable] This private generic transport accepts only variables constructed immediately at typed TaskSource call sites, never untrusted input; per-operation response mappers validate every external field before returning public values.
    async fn send(&self, query: &str, variables: Value) -> Result<Value, SourceError> {
        let response = self
            .client
            .post(&self.endpoint.0)
            .header("Authorization", self.key.expose_secret())
            .json(&json!({"query": query, "variables": variables}))
            .send()
            .await
            .map_err(|e| SourceError::Unavailable {
                message: e.to_string(),
            })?;
        let status = response.status();
        let retry = response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok());
        if status.as_u16() == 429 {
            return Err(SourceError::RateLimited {
                retry_after_seconds: retry,
                // Linear has one rate limiter and the status is the whole of what it said,
                // so there is nothing to add beyond the kind — which is what an absent
                // message means.
                message: None,
            });
        }
        if status.as_u16() == 401 || status.as_u16() == 403 {
            return Err(SourceError::Auth {
                message: "Linear rejected the configured credential".into(),
            });
        }
        if !status.is_success() {
            // Linear puts its GraphQL error envelope in the *body* of a 400, so the status
            // alone names the whole call and nothing about what Linear objected to. The
            // body is Linear's answer to this request and holds no credential; it is cut
            // because a proxy in front of Linear can answer with a page.
            let said = elided(&response.text().await.unwrap_or_default());
            return Err(SourceError::Unavailable {
                message: if said.is_empty() {
                    format!("Linear returned HTTP {status}")
                } else {
                    format!("Linear returned HTTP {status}: {said}")
                },
            });
        }
        let body: Envelope = response.json().await.map_err(|e| SourceError::Malformed {
            message: e.to_string(),
        })?;
        if let Some(error) = body.errors.first() {
            if let Some(extensions) = error
                .coded()
                .filter(|extensions| matches!(extensions.code, GqlErrorCode::RateLimited))
            {
                return Err(SourceError::RateLimited {
                    retry_after_seconds: extensions.retry_after.or(retry),
                    message: None,
                });
            }
            return Err(SourceError::Refused {
                message: error.said(),
            });
        }
        body.data.ok_or_else(|| SourceError::Malformed {
            message: "GraphQL response has no data".into(),
        })
    }

    // llmlint: ignore-block[contracts_have_one_source_or_a_drift_gate] These operators follow the accepted 2026-08-24 Linear contract, but Linear exposes their authoritative definitions only through an authenticated unversioned explorer; the real-HTTP tests assert every serialized operator and the shared CLI journeys assert resulting rows without making credentials required.
    fn filter(
        &self,
        labels: &onetaskgraph_plugin_api::LabelFilter,
        statuses: &[StatusCategory],
        project: Option<&ProjectFilter>,
    ) -> Value {
        let mut parts = Vec::new();
        if let Some(team) = &self.team {
            parts.push(json!({"team": {"key": {"eqIgnoreCase": team.0}}}));
        }
        // "At least one of these" is a disjunction of `eqIgnoreCase` rather than one
        // case-insensitive list operator, because Linear has no such operator. This source
        // sent `labels:{some:{name:{inIgnoreCase:[…]}}}` until Linear refused it outright,
        // HTTP 400, on the first read of the live lane that ever reached a label filter:
        //
        //     Variable "$filter" got invalid value { inIgnoreCase: […] } at
        //     "filter.and[1].labels.some.name"; Field "inIgnoreCase" is not defined by
        //     type "StringComparator". Did you mean "eqIgnoreCase" or "neqIgnoreCase"?
        //
        // That refusal is also the evidence for the replacement: Linear named the two
        // members of `StringComparator` closest to what it was sent, and `eqIgnoreCase` is
        // one of them — the same operator `all_of` below has always sent and the live lane
        // has always exercised. `in` exists there too and would need no `or`, but it is
        // case-sensitive, so `any_of` would stop agreeing with `all_of` and `none_of` and
        // with what the table at the top of this file says this source does.
        //
        // Nothing offline could have caught this: the pinned schema carries `StringComparator`
        // as the one member the operations it pins use, and the e2e fixture server accepted
        // `inIgnoreCase` because this source was what it was written against. Both now say
        // what Linear says instead.
        if !labels.any_of.is_empty() {
            parts.push(json!({"or": labels
                .any_of
                .iter()
                .map(|name| json!({"labels": {"some": {"name": {"eqIgnoreCase": name}}}}))
                .collect::<Vec<_>>()}));
        }
        for name in &labels.all_of {
            parts.push(json!({"labels": {"some": {"name": {"eqIgnoreCase": name}}}}));
        }
        for name in &labels.none_of {
            parts.push(json!({"labels": {"every": {"name": {"neqIgnoreCase": name}}}}));
        }
        if !statuses.is_empty() {
            parts.push(json!({"state": {"type": {"in": statuses.iter().flat_map(linear_statuses).collect::<Vec<_>>()}}}));
        }
        match project {
            Some(ProjectFilter::Orphans) => parts.push(json!({"project": {"null": true}})),
            Some(ProjectFilter::Is(id)) => parts.push(json!({"project": {"id": {"eq": id.0}}})),
            _ => {}
        }
        if parts.len() == 1 {
            parts.pop().unwrap()
        } else {
            json!({"and": parts})
        }
    }
    // llmlint: ignore-end[contracts_have_one_source_or_a_drift_gate]

    async fn one_id(&self, lookup: Lookup<'_>) -> Result<NativeId, SourceError> {
        let data = self.send(lookup.query(), lookup.variables()).await?;
        let connection = lookup.connection();
        let nodes = data
            .get(connection)
            .and_then(|v| v.get("nodes"))
            .and_then(Value::as_array)
            .ok_or_else(|| SourceError::Malformed {
                message: format!("missing {connection}.nodes"),
            })?;
        // A node this comparison cannot read is malformed rather than a nonmatch: dropping
        // it would turn Linear having answered nonsense into this source reporting no such
        // status, which is a different thing and reads as the caller's mistake.
        let matched = match lookup.local_name() {
            Some(name) => {
                let mut matched = Vec::new();
                for node in nodes {
                    if str_at(node, "name")?.eq_ignore_ascii_case(name) {
                        matched.push(node);
                    }
                }
                matched
            }
            None => nodes.iter().collect::<Vec<_>>(),
        };
        if matched.len() != 1 {
            return Err(SourceError::Refused {
                message: format!(
                    "source {} cannot resolve {} uniquely",
                    self.name,
                    lookup.diagnostic()
                ),
            });
        }
        Ok(NativeId(backend_id(matched[0], "id")?.to_owned()))
    }
    async fn team_id(&self) -> Result<NativeId, SourceError> {
        let team = self.team.as_ref().ok_or_else(|| SourceError::Refused {
            message: format!(
                "source {} needs config.team before it can create Linear items",
                self.name
            ),
        })?;
        self.one_id(Lookup::Team(&team.0)).await
    }
    async fn label_ids(
        &self,
        labels: &[Label],
        kind: WriteKind,
    ) -> Result<Vec<NativeId>, SourceError> {
        let mut ids = Vec::with_capacity(labels.len());
        for label in labels {
            ids.push(
                self.one_id(if matches!(kind, WriteKind::Project) {
                    Lookup::ProjectLabel(&label.name)
                } else {
                    Lookup::IssueLabel(&label.name)
                })
                .await?,
            );
        }
        Ok(ids)
    }
    fn write_description(
        &self,
        content: Option<&str>,
        metadata: &std::collections::BTreeMap<String, Value>,
        repositories: &[Repository],
        edges: &[DependencyEdge],
        kind: WriteKind,
    ) -> Result<Option<String>, SourceError> {
        let recorded = edges
            .iter()
            .filter(|edge| {
                edge.to.kind
                    != match kind {
                        WriteKind::Task => ItemKind::Task,
                        WriteKind::Project => ItemKind::Project,
                    }
                    || edge
                        .to
                        .id()
                        .split_once(':')
                        .is_some_and(|(source, _)| source != self.name.as_str())
            })
            .map(|edge| json!({"id":edge.to.id(),"kind":edge.to.kind}))
            .collect::<Vec<_>>();
        Self::long_form(content, metadata, repositories, recorded)
    }

    /// The one long-form field a Linear item has, with this source's own slot at the end.
    ///
    /// Shared by every kind this source writes rather than reimplemented per kind: a
    /// document keeps caller metadata in exactly the slot an issue and a project do, which
    /// is what lets the same read side take it back out.
    fn long_form(
        content: Option<&str>,
        metadata: &std::collections::BTreeMap<String, Value>,
        repositories: &[Repository],
        recorded: Vec<Value>,
    ) -> Result<Option<String>, SourceError> {
        let mut metadata = metadata.clone();
        if repositories.is_empty() {
            metadata.remove(Repository::METADATA_KEY);
        } else {
            metadata.insert(Repository::METADATA_KEY.into(), json!(repositories));
        }
        if recorded.is_empty() {
            metadata.remove(DependencyEdge::RECORDED_KEY);
        } else {
            metadata.insert(DependencyEdge::RECORDED_KEY.into(), Value::Array(recorded));
        }
        let visible = content.unwrap_or_default();
        if metadata.is_empty() {
            return Ok((!visible.is_empty()).then(|| visible.to_owned()));
        }
        let encoded = serde_json::to_string(&metadata).map_err(|error| SourceError::Malformed {
            message: error.to_string(),
        })?;
        Ok(Some(if visible.is_empty() {
            format!("{METADATA_OPEN}{encoded}{METADATA_CLOSE}")
        } else {
            format!("{visible}\n\n{METADATA_OPEN}{encoded}{METADATA_CLOSE}")
        }))
    }
    async fn write_relations(
        &self,
        near: &NativeId,
        edges: &[DependencyEdge],
        kind: WriteKind,
    ) -> Result<(), SourceError> {
        let mut cursor: Option<Cursor> = None;
        loop {
            let data = self
                .send(
                    if matches!(kind, WriteKind::Project) {
                        PROJECT_RELATIONS
                    } else {
                        ISSUE_RELATIONS
                    },
                    json!({"id":near.0,"first":250,"after":cursor.as_ref().map(|cursor|&cursor.0)}),
                )
                .await?;
            let root = data
                .get(if matches!(kind, WriteKind::Project) {
                    "project"
                } else {
                    "issue"
                })
                .ok_or_else(|| SourceError::Malformed {
                    message: "missing relation item".into(),
                })?;
            let relations = root
                .get("relations")
                .ok_or_else(|| SourceError::Malformed {
                    message: "missing relations".into(),
                })?;
            for relation in relations
                .get("nodes")
                .and_then(Value::as_array)
                .ok_or_else(|| SourceError::Malformed {
                    message: "missing relations.nodes".into(),
                })?
            {
                let id = backend_id(relation, "id")?;
                let (query, mutation) = if matches!(kind, WriteKind::Project) {
                    (
                        graphql::PROJECT_RELATION_DELETE,
                        MutationRoot::ProjectRelationDelete,
                    )
                } else {
                    (
                        graphql::ISSUE_RELATION_DELETE,
                        MutationRoot::IssueRelationDelete,
                    )
                };
                let deleted = self.send(query, json!({"id":id})).await?;
                mutation_payload(&deleted, mutation)?;
            }
            let Some(next) = page_next(relations)? else {
                break;
            };
            cursor = Some(next);
        }
        // Linear requires an anchor at each end of a project relation and validates both
        // against an enum GraphQL introspection cannot see: `ProjectRelationCreateInput`
        // declares them `String!` and enumerates nothing. The values are `start`, `end`
        // and `milestone`. `start` and `end` are the anchors for a relation on the project
        // as a whole — they name which of its own two ends the dependency line touches,
        // and Linear's project-dependency documentation says it "only support[s] a end ->
        // start dependency" — while `milestone` is what pairs with the two milestone ids
        // this source never sends. The terse field description reads as though the choice
        // were the project versus a milestone, but there is no value naming the project
        // alone: `project` is refused.
        //
        // Finish-to-start is the blocker's `end` onto the blocked project's `start`, and
        // `near` is the item that depends, so the near end takes `start` and the far end
        // it waits on takes `end`. That pair *is* the end -> start line Linear documents,
        // read the only way an anchor pair can be read — each anchor names a point on the
        // project it sits beside, and the line runs from the far end's `end` to the near
        // end's `start`. Anchoring by role rather than by position is what carries the
        // direction here, and it is deliberate: this source puts the depending item in
        // `projectId`, where Linear's own callers put the blocker, so a pair copied across
        // by position rather than by role would state the dependency backwards — which
        // Linear accepts as readily as the right way round, so it would be wrong in the
        // workspace rather than refused here. A `Related` edge carries no ordering and
        // Linear offers no anchor that says so, so it sends the same pair.
        const NEAR_ANCHOR: &str = "start";
        const FAR_ANCHOR: &str = "end";
        for edge in edges {
            if edge.to.kind
                != match kind {
                    WriteKind::Task => ItemKind::Task,
                    WriteKind::Project => ItemKind::Project,
                }
            {
                continue;
            }
            let far = match edge.to.id().split_once(':') {
                Some((source, native)) if source == self.name.as_str() => native,
                Some(_) => continue,
                None => edge.to.id(),
            };
            // A project relation is not spelled the way an issue relation is, and this is
            // the whole of what a project's `type` may say.
            //
            // `blocks` there is what the live journey's project write was refused for
            // once the two anchors above stopped being missing: Linear answered
            // `Argument Validation Error`, the message class its input validator raises
            // for a value outside an accepted set, having already accepted every field of
            // the same input by name. `tests/fixtures/README.md` records the observation
            // that decides which field: a live read-back found `blocks`, `dependsOn` and
            // `related` refused on a project relation in favour of `dependency`, the one
            // type Linear's project dependencies have. The anchors above are not that
            // failure — each of `start` and `end` is in the documented anchor vocabulary,
            // and Linear had already coerced both as `String!` before validation ran.
            //
            // What is *not* settled is whether Linear also constrains the anchor pair by
            // position, requiring `anchorType` itself to be `end`. Nothing offline can
            // decide it: the pair this source sends is the documented end -> start line
            // written with the depending project in `projectId`, and Linear enumerates
            // neither anchor anywhere introspection reaches. If the live lane is refused
            // again, the refusal now carries Linear's own `extensions` — see `GqlError` —
            // which names the field and the value, and settles it in one run. Swapping the
            // ids to satisfy such a constraint would also have to swap `relation_page`'s
            // reading of `relations`/`inverseRelations`, or the source would stop reading
            // back what it wrote.
            //
            // `Related` keeps `related` rather than being refused here. The read-back
            // above is second-hand and covers a relation Linear was asked to *create*; a
            // project edge carrying no ordering is a capability this source has, and
            // removing it on that evidence would be a wider claim than the evidence
            // supports. If Linear refuses it, it now says so in the message.
            let relation_type = match (kind, edge.kind) {
                (WriteKind::Project, DependencyKind::Blocks) => "dependency",
                (WriteKind::Task, DependencyKind::Blocks) => "blocks",
                (_, DependencyKind::Related) => "related",
            };
            let (query, input) = if matches!(kind, WriteKind::Project) {
                (
                    graphql::PROJECT_RELATION_CREATE,
                    json!({"projectId":near.0,"relatedProjectId":far,"type":relation_type,"anchorType":NEAR_ANCHOR,"relatedAnchorType":FAR_ANCHOR}),
                )
            } else {
                (
                    graphql::ISSUE_RELATION_CREATE,
                    json!({"issueId":near.0,"relatedIssueId":far,"type":relation_type}),
                )
            };
            let data = self.send(query, json!({"input":input})).await?;
            let mutation = if matches!(kind, WriteKind::Project) {
                MutationRoot::ProjectRelationCreate
            } else {
                MutationRoot::IssueRelationCreate
            };
            let payload = mutation_payload(&data, mutation)?;
            let relation = payload
                .get(if matches!(kind, WriteKind::Project) {
                    "projectRelation"
                } else {
                    "issueRelation"
                })
                .ok_or_else(|| SourceError::Malformed {
                    message: format!("missing {} relation", mutation.as_str()),
                })?;
            backend_id(relation, "id")?;
        }
        Ok(())
    }

    async fn prepare_edges(
        &self,
        edges: &[DependencyEdge],
        kind: WriteKind,
    ) -> Result<Vec<DependencyEdge>, SourceError> {
        let mut prepared = Vec::with_capacity(edges.len());
        for edge in edges {
            let mut edge = edge.clone();
            if edge.to.kind
                == match kind {
                    WriteKind::Task => ItemKind::Task,
                    WriteKind::Project => ItemKind::Project,
                }
                && edge
                    .to
                    .id()
                    .split_once(':')
                    .is_some_and(|(source, _)| source != self.name.as_str())
            {
                let mut cursor: Option<Cursor> = None;
                loop {
                    let data = self.send(if matches!(kind, WriteKind::Project) { PROJECTS } else { ISSUES }, json!({"first":250,"after":cursor.as_ref().map(|cursor|&cursor.0),"filter":{}})).await?;
                    let (items, next) = if matches!(kind, WriteKind::Project) {
                        let page = connection(&data, "projects", map_project)?;
                        (
                            page.items
                                .into_iter()
                                .map(|item| (item.id, item.metadata))
                                .collect::<Vec<_>>(),
                            page.next,
                        )
                    } else {
                        let page = connection(&data, "issues", map_task)?;
                        (
                            page.items
                                .into_iter()
                                .map(|item| (item.id, item.metadata))
                                .collect::<Vec<_>>(),
                            page.next,
                        )
                    };
                    if let Some((id, _)) = items.into_iter().find(|(_, metadata)| {
                        metadata.get("onetaskgraph.origin").and_then(Value::as_str)
                            == Some(edge.to.id())
                    }) {
                        edge.to = DependencyEndpoint::from_native(id, edge.to.kind);
                        break;
                    }
                    let Some(next) = next else { break };
                    cursor = Some(next);
                }
            }
            prepared.push(edge);
        }
        Ok(prepared)
    }
}

#[async_trait::async_trait]
impl TaskSource for LinearSource {
    fn kind(&self) -> &'static str {
        KIND
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            projects: Support::Native,
            documents: Support::Native,
            orphan_tasks: Support::Native,
            filter_by_label: Support::Native,
            filter_by_status: Support::Native,
            search_title: Support::Unsupported,
            search_content: Support::Unsupported,
            task_dependencies: DependencySupport::BothDirections,
            project_dependencies: DependencySupport::BothDirections,
            max_page_size: 250,
        }
    }
    fn writes(&self) -> WriteSupport {
        WriteSupport::Supported
    }
    async fn health(&self) -> Result<Health, SourceError> {
        let data = self.send(VIEWER, json!({})).await?;
        str_at(
            data.get("viewer").ok_or_else(|| SourceError::Malformed {
                message: "missing viewer".into(),
            })?,
            "id",
        )?;
        Ok(Health {
            reachable: true,
            detail: None,
        })
    }
    async fn get_task(&self, id: &NativeId) -> Result<Option<Task>, SourceError> {
        let d = self.send(ISSUE, json!({"id":id.0})).await?;
        optional(&d, "issue", map_task)
    }
    async fn get_project(&self, id: &NativeId) -> Result<Option<Project>, SourceError> {
        let d = self.send(PROJECT, json!({"id":id.0})).await?;
        optional(&d, "project", map_project)
    }
    async fn query_tasks(
        &self,
        query: &TaskQuery,
        page: &PageRequest,
    ) -> Result<Page<Task>, SourceError> {
        let d=self.send(ISSUES,json!({"first":page.limit.min(250),"after":page.cursor.as_ref().map(|c|&c.0),"filter":self.filter(&query.labels,&query.statuses,Some(&query.project))})).await?;
        connection(&d, "issues", map_task)
    }
    async fn query_projects(
        &self,
        query: &ProjectQuery,
        page: &PageRequest,
    ) -> Result<Page<Project>, SourceError> {
        // llmlint: ignore[changed_behavior_has_e2e] The shared CLI journey `every_complete_dataset_source_filters_projects_by_label_status_and_text` asserts that Linear status filtering returns only P-2 and reports native pushdown; this lower-level HTTP test separately asserts the serialized `started` predicate.
        let d=self.send(PROJECTS,json!({"first":page.limit.min(250),"after":page.cursor.as_ref().map(|c|&c.0),"filter":self.filter(&query.labels,&query.statuses,None)})).await?;
        connection(&d, "projects", map_project)
    }
    async fn labels(&self, page: &PageRequest) -> Result<Page<Label>, SourceError> {
        let d = self
            .send(
                LABELS,
                json!({"first":page.limit.min(250),"after":page.cursor.as_ref().map(|c|&c.0)}),
            )
            .await?;
        connection(&d, "issueLabels", map_label)
    }
    async fn task_dependencies(
        &self,
        id: &NativeId,
        direction: Direction,
        page: &PageRequest,
    ) -> Result<Page<DependencyEdge>, SourceError> {
        self.dependencies(ISSUE_RELATIONS, DependencyRoot::Issue, id, direction, page)
            .await
    }
    async fn project_dependencies(
        &self,
        id: &NativeId,
        direction: Direction,
        page: &PageRequest,
    ) -> Result<Page<DependencyEdge>, SourceError> {
        self.dependencies(
            PROJECT_RELATIONS,
            DependencyRoot::Project,
            id,
            direction,
            page,
        )
        .await
    }
    async fn write_task(&self, write: &ItemWrite<Task>) -> Result<NativeId, SourceError> {
        let edges = self
            .prepare_edges(&write.depends_on, WriteKind::Task)
            .await?;
        let team = self.team_id().await?;
        let state = self
            .one_id(Lookup::IssueState {
                name: &write.item.status.name,
                team: &team,
            })
            .await?;
        let labels = self.label_ids(&write.item.labels, WriteKind::Task).await?;
        let description = self.write_description(
            write.item.content.as_deref(),
            &write.item.metadata,
            &write.item.repositories,
            &edges,
            WriteKind::Task,
        )?;
        let input = json!({"title":write.item.title,"description":description,"stateId":state,"labelIds":labels,"projectId":write.item.project.as_ref().map(|id| id.0.clone())});
        let (query, variables, root) = match &write.target {
            Some(id) => (
                graphql::ISSUE_UPDATE,
                json!({"id":id.0,"input":input}),
                MutationRoot::IssueUpdate,
            ),
            None => (
                graphql::ISSUE_CREATE,
                {
                    let mut input = input;
                    input["teamId"] = Value::String(team.0);
                    json!({"input":input})
                },
                MutationRoot::IssueCreate,
            ),
        };
        let data = self.send(query, variables).await?;
        let issue =
            mutation_payload(&data, root)?
                .get("issue")
                .ok_or_else(|| SourceError::Malformed {
                    message: format!("missing {}.issue", root.as_str()),
                })?;
        let id = NativeId(backend_id(issue, "id")?.into());
        self.write_relations(&id, &edges, WriteKind::Task).await?;
        Ok(id)
    }
    async fn write_project(&self, write: &ItemWrite<Project>) -> Result<NativeId, SourceError> {
        let edges = self
            .prepare_edges(&write.depends_on, WriteKind::Project)
            .await?;
        let team = self.team_id().await?;
        let status = self
            .one_id(Lookup::ProjectStatus(&write.item.status.name))
            .await?;
        let labels = self
            .label_ids(&write.item.labels, WriteKind::Project)
            .await?;
        let description = self.write_description(
            write.item.content.as_deref(),
            &write.item.metadata,
            &write.item.repositories,
            &edges,
            WriteKind::Project,
        )?;
        let input = json!({"name":write.item.title,"description":description,"statusId":status,"labelIds":labels});
        let (query, variables, root) = match &write.target {
            Some(id) => (
                graphql::PROJECT_UPDATE,
                json!({"id":id.0,"input":input}),
                MutationRoot::ProjectUpdate,
            ),
            None => (
                graphql::PROJECT_CREATE,
                {
                    let mut input = input;
                    input["teamIds"] = json!([team]);
                    json!({"input":input})
                },
                MutationRoot::ProjectCreate,
            ),
        };
        let data = self.send(query, variables).await?;
        let project = mutation_payload(&data, root)?
            .get("project")
            .ok_or_else(|| SourceError::Malformed {
                message: format!("missing {}.project", root.as_str()),
            })?;
        let id = NativeId(backend_id(project, "id")?.into());
        self.write_relations(&id, &edges, WriteKind::Project)
            .await?;
        Ok(id)
    }
    async fn delete_task(&self, id: &NativeId) -> Result<(), SourceError> {
        // An id naming nothing is the state this asks for, not an error — Linear reports
        // an unknown issue as an errored response rather than an unsuccessful payload, and
        // `get_task` answering `None` is what says the item is already gone.
        if self.get_task(id).await?.is_none() {
            return Ok(());
        }
        let data = self.send(graphql::ISSUE_DELETE, json!({"id":id.0})).await?;
        mutation_payload(&data, MutationRoot::IssueDelete)?;
        Ok(())
    }
    async fn delete_project(&self, id: &NativeId) -> Result<(), SourceError> {
        // An id naming nothing is the state this asks for, on exactly the terms
        // `delete_task` reads it on.
        if self.get_project(id).await?.is_none() {
            return Ok(());
        }
        let data = self
            .send(graphql::PROJECT_DELETE, json!({"id":id.0}))
            .await?;
        mutation_payload(&data, MutationRoot::ProjectDelete)?;
        Ok(())
    }
    async fn get_document(&self, id: &NativeId) -> Result<Option<Document>, SourceError> {
        // Read as an optional although the pinned `document(id:)` returns `Document!`, for
        // the reason `delete_task` records: Linear answers an id naming nothing with an
        // errored response rather than a null, and reading the null defensively is what
        // keeps a responder that does answer one from being a malformed-response failure.
        let d = self.send(DOCUMENT, json!({"id":id.0})).await?;
        optional(&d, "document", map_document)
    }
    async fn query_documents(
        &self,
        query: &DocumentQuery,
        page: &PageRequest,
    ) -> Result<Page<Document>, SourceError> {
        // `query.text` is read by nothing here on purpose. Both searches are declared
        // `Unsupported`, and capability rule 2 says an ignored predicate returns the
        // *wider* set for the engine to narrow — half-applying one is what would drop rows.
        let want = page.limit.min(250) as usize;
        let mut filter = serde_json::Map::new();
        if let ProjectFilter::Is(id) = &query.project {
            filter.insert("project".into(), json!({"id": {"eq": id.0}}));
        }
        let filter = Value::Object(filter);
        let mut items = Vec::new();
        let mut cursor = page.cursor.clone();
        loop {
            // Only what is still owed, so the predicates applied here can never make this
            // return more than the caller asked for, and never drop what it fetched.
            let first = want.saturating_sub(items.len()).max(1);
            let d = self
                .send(
                    DOCUMENTS,
                    json!({"first":first,"after":cursor.as_ref().map(|cursor|&cursor.0),"filter":filter}),
                )
                .await?;
            let fetched = connection(&d, "documents", map_document)?;
            items.extend(
                fetched
                    .items
                    .into_iter()
                    .filter(|document| document_matches(document, &query.project, &query.labels)),
            );
            cursor = fetched.next;
            if cursor.is_none() || items.len() >= want {
                return Ok(Page {
                    items,
                    next: cursor,
                });
            }
        }
    }
    async fn write_document(&self, write: &ItemWrite<Document>) -> Result<NativeId, SourceError> {
        // Two refusals by name rather than two silent drops. Linear's own document type
        // has no labels and a document is not work, so neither a label nor a dependency
        // has anywhere here to land — and a copy that dropped one would report success for
        // an item the destination does not hold.
        if !write.item.labels.is_empty() {
            let named = write
                .item
                .labels
                .iter()
                .map(|label| label.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(SourceError::Refused {
                message: format!(
                    "source {} cannot carry a document's labels, because Linear's own \
                     document type has none: {named}",
                    self.name
                ),
            });
        }
        if !write.depends_on.is_empty()
            || write
                .item
                .metadata
                .contains_key(DependencyEdge::RECORDED_KEY)
        {
            return Err(SourceError::Refused {
                message: format!(
                    "source {} cannot carry {} on a document, because a document is not \
                     work and nothing may depend on one",
                    self.name,
                    DependencyEdge::RECORDED_KEY
                ),
            });
        }
        let content = Self::long_form(
            write.item.content.as_deref(),
            &write.item.metadata,
            &write.item.repositories,
            Vec::new(),
        )?;
        let project = write.item.project.as_ref().map(|id| id.0.clone());
        let (query, variables, root) = match &write.target {
            Some(id) => {
                // A target this workspace does not hold is refused rather than created:
                // the engine established that id before asking, so an absent one is a race
                // this destination must not paper over by writing a second document.
                if self.get_document(id).await?.is_none() {
                    return Err(SourceError::Refused {
                        message: format!("source {} holds no document {}", self.name, id.0),
                    });
                }
                (
                    graphql::DOCUMENT_UPDATE,
                    json!({"id":id.0,"input":{"title":write.item.title,"content":content,"projectId":project}}),
                    MutationRoot::DocumentUpdate,
                )
            }
            None => {
                let mut input =
                    json!({"title":write.item.title,"content":content,"projectId":project});
                // A Linear document lives in a project, an initiative, an issue or a team.
                // One filed under no project needs the configured team to be its home, and
                // one filed under a project already has one — so the team is asked for
                // only where it is the answer, rather than made a condition of every write.
                if project.is_none() {
                    input["teamId"] = Value::String(self.team_id().await?.0);
                }
                (
                    graphql::DOCUMENT_CREATE,
                    json!({ "input": input }),
                    MutationRoot::DocumentCreate,
                )
            }
        };
        let data = self.send(query, variables).await?;
        let document = mutation_payload(&data, root)?
            .get("document")
            .ok_or_else(|| SourceError::Malformed {
                message: format!("missing {}.document", root.as_str()),
            })?;
        Ok(NativeId(backend_id(document, "id")?.into()))
    }
    async fn delete_document(&self, id: &NativeId) -> Result<(), SourceError> {
        // An id naming nothing is the state this asks for, on exactly the terms
        // `delete_task` reads it on.
        if self.get_document(id).await?.is_none() {
            return Ok(());
        }
        let data = self
            .send(graphql::DOCUMENT_DELETE, json!({"id":id.0}))
            .await?;
        mutation_payload(&data, MutationRoot::DocumentDelete)?;
        Ok(())
    }
}

/// Linear relates one Linear item to another and nothing else, so an edge whose far end
/// is in a different source is the one edge no `relations` entry can hold. Those edges
/// are read from the near item's own [`DependencyEdge::RECORDED_KEY`] metadata, and they
/// are served *after* the native relations are spent: a page under this cursor is the
/// recorded tail of the same walk, which keeps the native pages exactly what they were.
const RECORDED_CURSOR: &str = "onetaskgraph.depends_on:";

impl LinearSource {
    async fn dependencies(
        &self,
        query: &str,
        root: DependencyRoot,
        id: &NativeId,
        direction: Direction,
        page: &PageRequest,
    ) -> Result<Page<DependencyEdge>, SourceError> {
        let limit = page.limit.min(250);
        let cursor = page.cursor.as_ref().map(|c| c.0.as_str());
        if let Some(offset) = cursor.and_then(|c| c.strip_prefix(RECORDED_CURSOR)) {
            // This cursor resumes the *forward* tail and only a forward walk ever issues
            // one, so a reverse read carrying it is resuming a walk it did not come from.
            // Serving it would answer a reverse read with forward edges, which is the one
            // thing a recorded edge must never do — its reverse is derived from the far
            // end and is never written down here.
            if direction != Direction::DependsOn {
                return Err(SourceError::Malformed {
                    message: format!(
                        "{RECORDED_CURSOR}{offset} resumes recorded forward edges, which a                          reverse dependency read never issues; resume it in the direction                          that reported it"
                    ),
                });
            }
            let offset: usize = offset.parse().map_err(|_| SourceError::Malformed {
                message: format!("{RECORDED_CURSOR}{offset} is not a recorded-edge cursor"),
            })?;
            let d = self
                .send(query, json!({"id":id.0,"first":1,"after":null}))
                .await?;
            return Ok(recorded_page(
                recorded(&d, root, id, &self.name)?,
                offset,
                limit as usize,
            ));
        }
        let d = self
            .send(query, json!({"id":id.0,"first":limit,"after":cursor}))
            .await?;
        let mut answered = relation_page(&d, root, id, direction)?;
        // Only forwards: the reverse of a recorded edge is derived from the far end, never
        // written down on the near item.
        if answered.next.is_none()
            && direction == Direction::DependsOn
            && !recorded(&d, root, id, &self.name)?.is_empty()
        {
            answered.next = Some(Cursor(format!("{RECORDED_CURSOR}0")));
        }
        Ok(answered)
    }
}

fn recorded(
    d: &Value,
    root: DependencyRoot,
    id: &NativeId,
    name: &SourceName,
) -> Result<Vec<DependencyEdge>, SourceError> {
    let item = d.get(root.as_str()).ok_or_else(|| SourceError::Malformed {
        message: format!("missing {}", root.as_str()),
    })?;
    let (_, metadata) = metadata_description(optional_string(item, "description")?)?;
    // `relations` on an issue holds issues and on a project holds projects, both of this
    // workspace — so a same-kind far end in this same source is one Linear itself was
    // supposed to hold, and the key is refused rather than quietly read, whether the entry
    // left the source out or spelled this one.
    DependencyEdge::recorded(
        &metadata,
        id,
        root.item_kind(),
        name,
        Some(root.item_kind()),
    )
    .map_err(|message| SourceError::Malformed { message })
}

fn recorded_page(edges: Vec<DependencyEdge>, offset: usize, limit: usize) -> Page<DependencyEdge> {
    let total = edges.len();
    let items: Vec<DependencyEdge> = edges.into_iter().skip(offset).take(limit.max(1)).collect();
    let end = offset.saturating_add(items.len());
    Page {
        items,
        next: (end < total).then(|| Cursor(format!("{RECORDED_CURSOR}{end}"))),
    }
}

// llmlint: ignore-block[contracts_have_one_source_or_a_drift_gate] Linear's workflow-state strings follow the accepted 2026-08-24 contract; its authoritative enum is exposed only through an authenticated unversioned explorer, while real-HTTP tests cover every serialized and parsed value.
fn linear_statuses(s: &StatusCategory) -> Vec<&'static str> {
    match s {
        // Linear's workflow states are triage, backlog, unstarted, started, completed and
        // canceled; none of them is a draft, so this narrows to nothing exactly as
        // `Unknown` does rather than filtering on a state Linear does not have.
        StatusCategory::Draft => vec![],
        StatusCategory::Backlog => vec!["backlog"],
        StatusCategory::Todo => vec!["unstarted"],
        StatusCategory::InProgress => vec!["started"],
        StatusCategory::Done => vec!["completed"],
        StatusCategory::Cancelled => vec!["canceled"],
        StatusCategory::Unknown => vec![],
    }
}
fn status(v: &Value) -> Result<Status, SourceError> {
    let name = str_at(v, "name")?.into();
    let category = match str_at(v, "type")? {
        "backlog" => StatusCategory::Backlog,
        "unstarted" => StatusCategory::Todo,
        "started" => StatusCategory::InProgress,
        "completed" => StatusCategory::Done,
        "canceled" => StatusCategory::Cancelled,
        _ => StatusCategory::Unknown,
    };
    Ok(Status { category, name })
}
// llmlint: ignore-end[contracts_have_one_source_or_a_drift_gate]
fn str_at<'a>(v: &'a Value, k: &str) -> Result<&'a str, SourceError> {
    v.get(k)
        .and_then(Value::as_str)
        .ok_or_else(|| SourceError::Malformed {
            message: format!("missing string field {k}"),
        })
}
fn map_label(v: &Value) -> Result<Label, SourceError> {
    Ok(Label {
        id: NativeId(str_at(v, "id")?.into()),
        name: str_at(v, "name")?.into(),
        color: optional_string(v, "color")?,
    })
}
fn labels_of(v: &Value) -> Result<Vec<Label>, SourceError> {
    v.get("nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| SourceError::Malformed {
            message: "missing label nodes".into(),
        })?
        .iter()
        .map(map_label)
        .collect()
}
fn time(v: &Value, k: &str) -> Result<Option<DateTime<Utc>>, SourceError> {
    optional_str(v, k)?
        .map(|s| {
            s.parse().map_err(|e| SourceError::Malformed {
                message: format!("invalid {k}: {e}"),
            })
        })
        .transpose()
}
fn map_task(v: &Value) -> Result<Task, SourceError> {
    let (content, metadata) = metadata_description(optional_string(v, "description")?)?;
    let repositories = Repository::from_metadata(&metadata)
        .map_err(|message| SourceError::Malformed { message })?;
    let url = optional_string(v, "url")?;
    Ok(Task {
        id: NativeId(str_at(v, "id")?.into()),
        title: str_at(v, "title")?.into(),
        content,
        status: status(v.get("state").ok_or_else(|| SourceError::Malformed {
            message: "missing state".into(),
        })?)?,
        labels: labels_of(v.get("labels").ok_or_else(|| SourceError::Malformed {
            message: "missing labels".into(),
        })?)?,
        project: filed_under(v)?,
        location: web_address(url.as_deref()),
        url,
        created_at: time(v, "createdAt")?,
        updated_at: time(v, "updatedAt")?,
        metadata,
        repositories,
    })
}
fn map_project(v: &Value) -> Result<Project, SourceError> {
    let (content, metadata) = metadata_description(optional_string(v, "description")?)?;
    let repositories = Repository::from_metadata(&metadata)
        .map_err(|message| SourceError::Malformed { message })?;
    let url = optional_string(v, "url")?;
    Ok(Project {
        id: NativeId(str_at(v, "id")?.into()),
        title: str_at(v, "name")?.into(),
        content,
        status: status(v.get("status").ok_or_else(|| SourceError::Malformed {
            message: "missing status".into(),
        })?)?,
        labels: labels_of(v.get("labels").ok_or_else(|| SourceError::Malformed {
            message: "missing project labels".into(),
        })?)?,
        location: web_address(url.as_deref()),
        url,
        created_at: time(v, "createdAt")?,
        updated_at: time(v, "updatedAt")?,
        metadata,
        repositories,
    })
}

/// Where a Linear entity is: the web address Linear itself reports for it, as a link.
///
/// Every issue, project and document of a Linear workspace has a page a person can open,
/// so this source says so for all three — the counterpart of a folder of Markdown
/// reporting the path of the file behind an item. A source that reported nothing here is
/// what leaves a reader holding an opaque id, and `None` is reserved for the case Linear
/// really did not say, which is not the same as saying the entity is nowhere.
fn web_address(url: Option<&str>) -> Option<Location> {
    url.map(|url| Location::Url(url.to_owned()))
}

/// The project a Linear item is filed under, or `None` for one filed under nothing.
///
/// One reader for issues and documents alike, because the field is the same field: an
/// absent `project` key is a malformed response, a null one is an orphan.
fn filed_under(v: &Value) -> Result<Option<NativeId>, SourceError> {
    match v.get("project") {
        None => Err(SourceError::Malformed {
            message: "missing project field".into(),
        }),
        Some(Value::Null) => Ok(None),
        Some(project) => Ok(Some(NativeId(str_at(project, "id")?.into()))),
    }
}

fn map_document(v: &Value) -> Result<Document, SourceError> {
    let (content, metadata) = metadata_description(optional_string(v, "content")?)?;
    let repositories = Repository::from_metadata(&metadata)
        .map_err(|message| SourceError::Malformed { message })?;
    let url = optional_string(v, "url")?;
    Ok(Document {
        id: NativeId(str_at(v, "id")?.into()),
        title: str_at(v, "title")?.into(),
        content,
        project: filed_under(v)?,
        // Linear's `Document` carries no labels, and that is the published schema rather
        // than a gap here: the types of it that carry `labels` are `Issue`, `Project`,
        // `Team`, `Initiative` and `Organization`. Reporting none is what a source with no
        // native slot owes; standing one up beside a first-class type is what this source
        // exists not to do, and `write_document` refuses a label by name for the same
        // reason rather than dropping it.
        labels: Vec::new(),
        location: web_address(url.as_deref()),
        url,
        created_at: time(v, "createdAt")?,
        updated_at: time(v, "updatedAt")?,
        metadata,
        repositories,
    })
}

/// Whether this document satisfies the predicates this source applies to a fetched page.
///
/// Two of them reach a page rather than the `documents(filter:)` variables, and each for a
/// reason of Linear's own. `DocumentFilter.project` is a `ProjectFilter` where
/// `IssueFilter.project` is a `NullableProjectFilter`, so only the issue side can be asked
/// for the items belonging to no project. And a Linear document carries no label at all,
/// so a query demanding one keeps nothing and a query excluding one keeps everything —
/// which is this source *applying* the predicate it declares native, over the labels the
/// document really has, rather than ignoring it.
fn document_matches(document: &Document, project: &ProjectFilter, labels: &LabelFilter) -> bool {
    let carries = |name: &String| {
        document
            .labels
            .iter()
            .any(|label| label.name.eq_ignore_ascii_case(name))
    };
    let filed = match project {
        ProjectFilter::Any => true,
        ProjectFilter::Orphans => document.project.is_none(),
        ProjectFilter::Is(id) => document.project.as_ref() == Some(id),
    };
    filed
        && (labels.any_of.is_empty() || labels.any_of.iter().any(&carries))
        && labels.all_of.iter().all(&carries)
        && !labels.none_of.iter().any(&carries)
}

fn optional<T>(
    d: &Value,
    k: &str,
    f: fn(&Value) -> Result<T, SourceError>,
) -> Result<Option<T>, SourceError> {
    match d.get(k) {
        None => Err(SourceError::Malformed {
            message: format!("missing {k}"),
        }),
        Some(Value::Null) => Ok(None),
        Some(value) => f(value).map(Some),
    }
}
fn connection<T>(
    d: &Value,
    k: &str,
    f: fn(&Value) -> Result<T, SourceError>,
) -> Result<Page<T>, SourceError> {
    let c = d.get(k).ok_or_else(|| SourceError::Malformed {
        message: format!("missing {k} connection"),
    })?;
    let items = c
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| SourceError::Malformed {
            message: "missing nodes".into(),
        })?
        .iter()
        .map(f)
        .collect::<Result<_, _>>()?;
    let next = page_next(c)?;
    Ok(Page { items, next })
}
#[derive(Clone, Copy)]
enum DependencyRoot {
    Issue,
    Project,
}
impl DependencyRoot {
    const fn item_kind(self) -> ItemKind {
        match self {
            Self::Issue => ItemKind::Task,
            Self::Project => ItemKind::Project,
        }
    }
    const fn as_str(self) -> &'static str {
        match self {
            Self::Issue => "issue",
            Self::Project => "project",
        }
    }
}
fn relation_page(
    d: &Value,
    root: DependencyRoot,
    id: &NativeId,
    direction: Direction,
) -> Result<Page<DependencyEdge>, SourceError> {
    let key = if direction == Direction::DependsOn {
        "relations"
    } else {
        "inverseRelations"
    };
    let c = d
        .get(root.as_str())
        .and_then(|v| v.get(key))
        .ok_or_else(|| SourceError::Malformed {
            message: format!("missing {key}"),
        })?;
    let nodes = c
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| SourceError::Malformed {
            message: "missing relation nodes".into(),
        })?;
    let mut items = Vec::new();
    for n in nodes {
        let other = n
            .get(if direction == Direction::DependsOn {
                "relatedIssue"
            } else {
                "issue"
            })
            .or_else(|| {
                n.get(if direction == Direction::DependsOn {
                    "relatedProject"
                } else {
                    "project"
                })
            })
            .and_then(|v| v.get("id"))
            .and_then(Value::as_str)
            .ok_or_else(|| SourceError::Malformed {
                message: "missing related id".into(),
            })?;
        let (from, to) = if direction == Direction::DependsOn {
            (id.clone(), NativeId(other.into()))
        } else {
            (NativeId(other.into()), id.clone())
        };
        // llmlint: ignore-block[contracts_have_one_source_or_a_drift_gate] Linear publishes relation type as a string in the accepted 2026-08-24 schema; this boundary deliberately rejects every undocumented value, and real-HTTP tests prove both accepted values and rejection.
        let relation_type =
            n.get("type")
                .and_then(Value::as_str)
                .ok_or_else(|| SourceError::Malformed {
                    message: "missing relation type".into(),
                })?;
        // An issue relation and a project relation do not share a vocabulary. Linear
        // spells a project dependency `dependency`, where an issue's is `blocks`; the
        // write side sends exactly that pair and says why. So each root reads only its
        // own, and a value the other root would have accepted is refused here rather than
        // read as an edge this source could not have written.
        let kind = match (root, relation_type) {
            (DependencyRoot::Issue, "blocks") | (DependencyRoot::Project, "dependency") => {
                DependencyKind::Blocks
            }
            (_, "related") => DependencyKind::Related,
            _ => {
                return Err(SourceError::Malformed {
                    message: format!(
                        "invalid relation type: {relation_type} on a {} relation",
                        root.as_str()
                    ),
                });
            }
        };
        // llmlint: ignore-end[contracts_have_one_source_or_a_drift_gate]
        let item_kind = root.item_kind();
        items.push(DependencyEdge {
            from: DependencyEndpoint::from_native(from, item_kind),
            to: DependencyEndpoint::from_native(to, item_kind),
            kind,
        });
    }
    let next = page_next(c)?;
    Ok(Page { items, next })
}

fn optional_str<'a>(v: &'a Value, k: &str) -> Result<Option<&'a str>, SourceError> {
    match v.get(k) {
        None => Err(SourceError::Malformed {
            message: format!("missing field {k}"),
        }),
        Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_str()
            .map(Some)
            .ok_or_else(|| SourceError::Malformed {
                message: format!("field {k} is not a string"),
            }),
    }
}

/// Linear has no caller-defined fields. The source owns an unobtrusive Markdown comment
/// at the end of `description`; its later write side must use this exact encoding.
const METADATA_OPEN: &str = "<!-- onetaskgraph.metadata\n";
const METADATA_CLOSE: &str = "\n-->";

fn metadata_description(
    description: Option<String>,
) -> Result<(Option<String>, std::collections::BTreeMap<String, Value>), SourceError> {
    let Some(description) = description else {
        return Ok((None, Default::default()));
    };
    let Some(start) = description.rfind(METADATA_OPEN) else {
        return Ok((Some(description), Default::default()));
    };
    let encoded_start = start + METADATA_OPEN.len();
    let Some(relative_end) = description[encoded_start..].find(METADATA_CLOSE) else {
        return Err(SourceError::Malformed {
            message: "unterminated onetaskgraph metadata slot in Linear description".into(),
        });
    };
    let encoded_end = encoded_start + relative_end;
    if !description[encoded_end + METADATA_CLOSE.len()..]
        .trim()
        .is_empty()
    {
        return Ok((Some(description), Default::default()));
    }
    let metadata =
        serde_json::from_str(&description[encoded_start..encoded_end]).map_err(|error| {
            SourceError::Malformed {
                message: format!(
                    "invalid canonical JSON in Linear onetaskgraph metadata slot: {error}"
                ),
            }
        })?;
    let visible = description[..start].trim_end();
    Ok(((!visible.is_empty()).then(|| visible.to_owned()), metadata))
}

fn optional_string(v: &Value, k: &str) -> Result<Option<String>, SourceError> {
    Ok(optional_str(v, k)?.map(Into::into))
}
fn backend_id<'a>(value: &'a Value, field: &str) -> Result<&'a str, SourceError> {
    let id = str_at(value, field)?;
    (!id.is_empty())
        .then_some(id)
        .ok_or_else(|| SourceError::Malformed {
            message: format!("field {field} is an empty backend id"),
        })
}
fn mutation_payload(data: &Value, root: MutationRoot) -> Result<&Value, SourceError> {
    let root = root.as_str();
    let payload = data.get(root).ok_or_else(|| SourceError::Malformed {
        message: format!("missing {root}"),
    })?;
    match payload.get("success").and_then(Value::as_bool) {
        Some(true) => Ok(payload),
        Some(false) => Err(SourceError::Refused {
            message: format!("Linear reported {root} was unsuccessful"),
        }),
        None => Err(SourceError::Malformed {
            message: format!("missing boolean {root}.success"),
        }),
    }
}
fn page_next(c: &Value) -> Result<Option<Cursor>, SourceError> {
    let info = c.get("pageInfo").ok_or_else(|| SourceError::Malformed {
        message: "missing pageInfo".into(),
    })?;
    let more = info
        .get("hasNextPage")
        .and_then(Value::as_bool)
        .ok_or_else(|| SourceError::Malformed {
            message: "missing boolean pageInfo.hasNextPage".into(),
        })?;
    if !more {
        return Ok(None);
    }
    let cursor = str_at(info, "endCursor")?;
    Ok(Some(Cursor(cursor.into())))
}
