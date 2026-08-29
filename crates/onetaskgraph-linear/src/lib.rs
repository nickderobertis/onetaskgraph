//! A read/write source over Linear's published GraphQL API.
//!
//! Linear `Issue` maps to [`Task`], `Project` to [`Project`], `IssueLabel` and
//! `ProjectLabel` to [`Label`], and `WorkflowState.name` is preserved while its
//! `type` (`backlog`, `unstarted`, `started`, `completed`, or `canceled`) maps to
//! the normalized status category. Issue `relations`/`inverseRelations` and
//! project relations provide native dependency traversal in both directions.
//!
//! Label, workflow-state, project, and orphan filters are sent in the
//! `issues(filter:)`/`projects(filter:)` variables. Pagination uses Relay `first` and
//! `after`.
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
//! | `orphan_tasks` | **Supported and proven.** `issues(filter:{project:{null:true}})`. |
//! | `filter_by_label` | **Supported and proven.** `labels:{some:{name:{inIgnoreCase:…}}}` and `{eqIgnoreCase:…}` for what an item must carry, `labels:{every:{name:{neqIgnoreCase:…}}}` for what it must not. |
//! | `filter_by_status` | **Supported and proven.** `state:{type:{in:[…]}}`, over the `WorkflowState.type` vocabulary the category maps to. |
//! | `search_title` | **Unsupported — and unimplemented rather than unsupportable.** See the ruling below. |
//! | `search_content` | **Unsupported — and unimplemented rather than unsupportable.** See the ruling below. |
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
//! Caller metadata is canonical JSON in a trailing
//! `<!-- onetaskgraph.metadata ... -->` Markdown comment in the item's description. The
//! visible description is returned unchanged without that slot. Writes put the same
//! canonical encoding back beside the visible description, and use Linear issue/project
//! relations for same-source dependencies. Only cross-source far ends use the reserved
//! `onetaskgraph.depends_on` metadata key.
//!
//! Fixture provenance is recorded in `tests/fixtures/README.md`. The ignored live lane
//! exercises reads by default; its mutation journey requires an explicitly named scratch
//! team and immediately requests deletion of its scratch issue. A failed live cleanup is
//! reported as a test failure and may require manual deletion from that scratch team.
#![deny(missing_docs)]

use chrono::{DateTime, Utc};
use onetaskgraph_plugin_api::{
    Capabilities, Cursor, DependencyEdge, DependencyEndpoint, DependencyKind, DependencySupport,
    Direction, Health, ItemKind, ItemWrite, Label, NativeId, Page, PageRequest, Project,
    ProjectFilter, ProjectQuery, Repository, SecretResolver, SourceError, SourceName, SourcePlugin,
    Status, StatusCategory, Support, Task, TaskQuery, TaskSource, WriteSupport,
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
    pub const ISSUE_STATE: &str = "query($name:String!,$team:String!){ workflowStates(filter:{name:{eqIgnoreCase:$name},team:{id:{eq:$team}}}){nodes{id}} }";
    /// Resolve a project status display name.
    pub const PROJECT_STATUS: &str =
        "query($name:String!){ projectStatuses(filter:{name:{eqIgnoreCase:$name}}){nodes{id}} }";
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
    /// Delete an issue created only by the ignored live write journey.
    pub const ISSUE_DELETE: &str = "mutation($id:String!){ issueDelete(id:$id){success} }";
}

use graphql::{
    ISSUE, ISSUE_RELATIONS, ISSUES, LABELS, PROJECT, PROJECT_RELATIONS, PROJECTS, VIEWER,
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
            Self::ProjectStatus(name) | Self::IssueLabel(name) | Self::ProjectLabel(name) => {
                json!({"name":name})
            }
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
    extensions: Option<GqlExtensions>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GqlExtensions {
    code: GqlErrorCode,
    retry_after: Option<u64>,
}
#[derive(Deserialize)]
enum GqlErrorCode {
    #[serde(rename = "RATELIMITED", alias = "RATE_LIMITED")]
    RateLimited,
    #[serde(other)]
    Other,
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
            });
        }
        if status.as_u16() == 401 || status.as_u16() == 403 {
            return Err(SourceError::Auth {
                message: "Linear rejected the configured credential".into(),
            });
        }
        if !status.is_success() {
            return Err(SourceError::Unavailable {
                message: format!("Linear returned HTTP {status}"),
            });
        }
        let body: Envelope = response.json().await.map_err(|e| SourceError::Malformed {
            message: e.to_string(),
        })?;
        if let Some(error) = body.errors.first() {
            if error
                .extensions
                .as_ref()
                .is_some_and(|extensions| matches!(extensions.code, GqlErrorCode::RateLimited))
            {
                let hint = error
                    .extensions
                    .as_ref()
                    .and_then(|value| value.retry_after);
                return Err(SourceError::RateLimited {
                    retry_after_seconds: hint.or(retry),
                });
            }
            return Err(SourceError::Refused {
                message: error.message.clone(),
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
        if !labels.any_of.is_empty() {
            parts.push(json!({"labels": {"some": {"name": {"inIgnoreCase": labels.any_of}}}}));
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
        if nodes.len() != 1 {
            return Err(SourceError::Refused {
                message: format!(
                    "source {} cannot resolve {} uniquely",
                    self.name,
                    lookup.diagnostic()
                ),
            });
        }
        Ok(NativeId(backend_id(&nodes[0], "id")?.to_owned()))
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
        let mut metadata = metadata.clone();
        if repositories.is_empty() {
            metadata.remove(Repository::METADATA_KEY);
        } else {
            metadata.insert(Repository::METADATA_KEY.into(), json!(repositories));
        }
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
            let relation_type = match edge.kind {
                DependencyKind::Blocks => "blocks",
                DependencyKind::Related => "related",
            };
            let (query, input) = if matches!(kind, WriteKind::Project) {
                (
                    graphql::PROJECT_RELATION_CREATE,
                    json!({"projectId":near.0,"relatedProjectId":far,"type":relation_type}),
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
        // llmlint: ignore[changed_behavior_has_e2e] The shared CLI journey `every_source_filters_its_projects_by_label_by_status_and_by_text` asserts that Linear status filtering returns only P-2 and reports native pushdown; this lower-level HTTP test separately asserts the serialized `started` predicate.
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
        project: match v.get("project") {
            None => {
                return Err(SourceError::Malformed {
                    message: "missing project field".into(),
                });
            }
            Some(Value::Null) => None,
            Some(p) => Some(NativeId(str_at(p, "id")?.into())),
        },
        url: optional_string(v, "url")?,
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
        url: optional_string(v, "url")?,
        created_at: time(v, "createdAt")?,
        updated_at: time(v, "updatedAt")?,
        metadata,
        repositories,
    })
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
        // llmlint: ignore-block[contracts_have_one_source_or_a_drift_gate] Linear publishes relation type as a string in the accepted 2026-08-24 schema; this boundary enum deliberately rejects every undocumented value, and real-HTTP tests prove both accepted values and rejection.
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        enum RelationKind {
            Blocks,
            Related,
        }
        let kind = match serde_json::from_value::<RelationKind>(n.get("type").cloned().ok_or_else(
            || SourceError::Malformed {
                message: "missing relation type".into(),
            },
        )?)
        .map_err(|e| SourceError::Malformed {
            message: format!("invalid relation type: {e}"),
        })? {
            RelationKind::Blocks => DependencyKind::Blocks,
            RelationKind::Related => DependencyKind::Related,
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
