//! A stateless onetaskgraph source over one GitHub Projects v2 project.
//!
//! A project maps to the configured GitHub `ProjectV2`, not a repository: one Projects v2
//! board can contain work from several repositories and draft work from none. Project fields
//! read `ProjectV2.id`, `title`, `shortDescription`, `url`, `createdAt`, and `updatedAt`. Tasks
//! map from `ProjectV2Item.content` (`Issue`, `PullRequest`, or `DraftIssue`); labels read the
//! content's `labels` connection and `ProjectV2ItemFieldLabelValue`.
//!
//! Status reads the item value whose `ProjectV2ItemFieldSingleSelectValue.field.name` is
//! `Status`. Its option name is retained. The default maps Backlog, Todo/Open, In Progress/In
//! Review, Done/Closed/Merged, and Cancelled/Canceled; `status_mapping` overrides option names
//! case-insensitively, and all other user-defined names remain `Unknown`.
//!
//! `ProjectV2.items` pages but has no label, status, orphan, or content-search arguments.
//! Project listing alone is native; the plugin ignores every unsupported query predicate so the
//! engine can compensate from the wider result. Dependencies traverse underlying `Issue` nodes.
//! `Issue.blockedBy` supplies forward `Blocks` edges. GitHub documents `Issue.blocking` as being
//! removed and always empty, so both dependency capabilities are `ForwardOnly`; project
//! dependency reads aggregate the configured project's issue edges. Projects v2 has no native
//! project-to-project relationship, so those aggregate edges retain the underlying issue IDs.
//!
//! Live verification is non-destructive by construction: [`TaskSource`] has read operations only
//! and this crate sends GraphQL `query` operations only, with no mutation for setup or teardown.
#![deny(missing_docs)]

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use onetaskgraph_plugin_api::{
    Capabilities, Cursor, DependencyEdge, DependencyKind, DependencySupport, Direction, Health,
    Label, NativeId, Page, PageRequest, Project, ProjectQuery, SecretResolver, SourceError,
    SourceName, SourcePlugin, Status, StatusCategory, Support, Task, TaskQuery, TaskSource,
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

fn default_token_env() -> String {
    "GH_PROJECTS_TOKEN".to_owned()
}
fn default_endpoint() -> String {
    "https://api.github.com/graphql".to_owned()
}

/// Configuration for one GitHub Projects v2 project.
#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct GitHubProjectsConfig {
    /// Login of the user or organization which owns the project.
    pub owner: String,
    /// The project number shown in its GitHub URL.
    pub project_number: u32,
    /// Environment variable containing a GitHub token with project read access.
    #[serde(default = "default_token_env")]
    pub token_env: String,
    /// GraphQL endpoint. GitHub Enterprise installations may override it.
    #[serde(default = "default_endpoint")]
    pub endpoint: String,
    /// Case-insensitive project status name to normalized category mapping.
    #[serde(default)]
    pub status_mapping: BTreeMap<String, StatusCategory>,
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
        let source = GitHubProjectsSource::new(config, secrets).map_err(|error| match error {
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

/// A source which reads GitHub afresh for every operation.
pub struct GitHubProjectsSource {
    owner: String,
    project_number: u32,
    endpoint: Url,
    token: SecretString,
    statuses: BTreeMap<String, StatusCategory>,
    client: Client,
}

impl GitHubProjectsSource {
    /// Validate configuration and capture the named credential without exposing it.
    pub fn new(
        config: GitHubProjectsConfig,
        secrets: &dyn SecretResolver,
    ) -> Result<Self, SourceError> {
        if config.owner.trim().is_empty() {
            return Err(SourceError::Config {
                message: "owner must not be empty".into(),
            });
        }
        if config.project_number == 0 {
            return Err(SourceError::Config {
                message: "project_number must be at least 1".into(),
            });
        }
        if config.token_env.trim().is_empty() {
            return Err(SourceError::Config {
                message: "token_env must not be empty".into(),
            });
        }
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
        let token = secrets.get(&config.token_env).filter(|token| !token.expose_secret().is_empty()).ok_or_else(|| SourceError::Auth {
            message: format!("environment variable {} is missing or empty; set it to a GitHub token with read:project and repository Issues read access", config.token_env),
        })?;
        Ok(Self {
            owner: config.owner,
            project_number: config.project_number,
            endpoint,
            token,
            statuses: normalize_status_mapping(config.status_mapping)?,
            client: Client::builder()
                .user_agent("onetaskgraph")
                .build()
                .map_err(|e| SourceError::Config {
                    message: format!("cannot build HTTP client: {e}"),
                })?,
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
                    "GitHub rejected the configured credential with HTTP {status}; grant it read:project and repository Issues read access"
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
        if let Some(errors) = body
            .get("errors")
            .and_then(Value::as_array)
            .filter(|e| !e.is_empty())
        {
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
                        "{message}; grant GH_PROJECTS_TOKEN read:project and repository Issues read access"
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

    async fn project_value(
        &self,
        items_after: Option<&str>,
        items_first: u32,
    ) -> Result<Value, SourceError> {
        const QUERY: &str = r#"query($owner:String!,$number:Int!,$first:Int!,$after:String){
          organization(login:$owner){projectV2(number:$number){...Project}}
          user(login:$owner){projectV2(number:$number){...Project}}
        } fragment Project on ProjectV2 { id title shortDescription url createdAt updatedAt closed
          items(first:$first,after:$after){nodes{id fieldValues(first:100){nodes{
            ... on ProjectV2ItemFieldSingleSelectValue{name field{name}}
            ... on ProjectV2ItemFieldLabelValue{labels(first:100){nodes{id name color}}}
          }} content{
            ... on Issue{id title body url createdAt updatedAt state labels(first:100){nodes{id name color}}}
            ... on PullRequest{id title body url createdAt updatedAt state labels(first:100){nodes{id name color}}}
            ... on DraftIssue{id title body createdAt updatedAt}
          }} pageInfo{hasNextPage endCursor}}
        }"#;
        let data = self.graphql(QUERY, json!({"owner":self.owner,"number":self.project_number,"first":items_first.min(MAX_PAGE_SIZE),"after":items_after})).await?;
        data.pointer("/organization/projectV2")
            .filter(|v| !v.is_null())
            .or_else(|| {
                data.pointer("/user/projectV2")
                    .filter(|value| !value.is_null())
            })
            .cloned()
            .ok_or_else(|| SourceError::Refused {
                message: format!(
                    "GitHub project {}/{} was not found or is not visible to the token",
                    self.owner, self.project_number
                ),
            })
    }

    fn status(&self, item: &Value) -> Status {
        let name = item
            .pointer("/fieldValues/nodes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|v| v.pointer("/field/name").and_then(Value::as_str) == Some("Status"))
            .and_then(|v| v.get("name"))
            .and_then(Value::as_str)
            .or_else(|| item.pointer("/content/state").and_then(Value::as_str))
            .unwrap_or("Unknown")
            .to_owned();
        let category = self
            .statuses
            .get(&name.to_lowercase())
            .copied()
            .unwrap_or_else(|| match name.to_ascii_lowercase().as_str() {
                "backlog" => StatusCategory::Backlog,
                "todo" | "open" => StatusCategory::Todo,
                "in progress" | "in review" => StatusCategory::InProgress,
                "done" | "closed" | "merged" => StatusCategory::Done,
                "cancelled" | "canceled" => StatusCategory::Cancelled,
                _ => StatusCategory::Unknown,
            });
        Status { category, name }
    }

    fn labels(item: &Value) -> Vec<Label> {
        let direct = item
            .pointer("/content/labels/nodes")
            .and_then(Value::as_array);
        let field = item
            .pointer("/fieldValues/nodes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find_map(|v| v.pointer("/labels/nodes").and_then(Value::as_array));
        direct
            .into_iter()
            .flatten()
            .chain(field.into_iter().flatten())
            .filter_map(|v| {
                Some(Label {
                    id: NativeId(v.get("id")?.as_str()?.to_owned()),
                    name: v.get("name")?.as_str()?.to_owned(),
                    color: v.get("color").and_then(Value::as_str).map(str::to_owned),
                })
            })
            .fold(Vec::new(), |mut labels, label| {
                if !labels.iter().any(|x: &Label| x.id == label.id) {
                    labels.push(label);
                }
                labels
            })
    }

    fn task(&self, project_id: &str, item: &Value) -> Option<Task> {
        let content = item.get("content")?;
        if content.is_null() {
            return None;
        }
        Some(Task {
            id: NativeId(content.get("id")?.as_str()?.to_owned()),
            title: content.get("title")?.as_str()?.to_owned(),
            content: content
                .get("body")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
            status: self.status(item),
            labels: Self::labels(item),
            project: Some(NativeId(project_id.to_owned())),
            url: content
                .get("url")
                .and_then(Value::as_str)
                .map(str::to_owned),
            created_at: parse_time(content.get("createdAt")),
            updated_at: parse_time(content.get("updatedAt")),
        })
    }

    fn project(&self, value: &Value) -> Result<Project, SourceError> {
        let id = required_str(value, "id")?;
        Ok(Project {
            id: NativeId(id.into()),
            title: required_str(value, "title")?.into(),
            content: value
                .get("shortDescription")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
            status: Status {
                category: if value.get("closed").and_then(Value::as_bool) == Some(true) {
                    StatusCategory::Done
                } else {
                    StatusCategory::InProgress
                },
                name: if value.get("closed").and_then(Value::as_bool) == Some(true) {
                    "Closed"
                } else {
                    "Open"
                }
                .into(),
            },
            labels: vec![],
            url: value.get("url").and_then(Value::as_str).map(str::to_owned),
            created_at: parse_time(value.get("createdAt")),
            updated_at: parse_time(value.get("updatedAt")),
        })
    }

    async fn all_tasks(&self) -> Result<Vec<Task>, SourceError> {
        let mut after = None;
        let mut tasks = Vec::new();
        loop {
            let project = self.project_value(after.as_deref(), MAX_PAGE_SIZE).await?;
            let project_id = required_str(&project, "id")?;
            for item in project
                .pointer("/items/nodes")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if let Some(task) = self.task(project_id, item) {
                    tasks.push(task);
                }
            }
            let page =
                project
                    .pointer("/items/pageInfo")
                    .ok_or_else(|| SourceError::Malformed {
                        message: "GitHub project items have no pageInfo".into(),
                    })?;
            if page.get("hasNextPage").and_then(Value::as_bool) != Some(true) {
                break;
            }
            after = Some(required_str(page, "endCursor")?.to_owned());
        }
        Ok(tasks)
    }

    async fn dependencies(
        &self,
        id: &NativeId,
        direction: Direction,
        page: &PageRequest,
    ) -> Result<Page<DependencyEdge>, SourceError> {
        validate_page(page)?;
        if direction == Direction::DependedOnBy {
            return Err(SourceError::Refused {
                message: "GitHub's Issue.blocking field is being removed and always empty; reverse traversal must be emulated by the engine".into(),
            });
        }
        const QUERY: &str = r#"query($id:ID!,$first:Int!,$after:String){node(id:$id){... on Issue{blockedBy(first:$first,after:$after){nodes{id}pageInfo{hasNextPage endCursor}}}}}"#;
        let data = self.graphql(QUERY, json!({"id":id.0,"first":page.limit.min(MAX_PAGE_SIZE),"after":page.cursor.as_ref().map(|c| c.0.as_str())})).await?;
        let node =
            data.get("node")
                .filter(|v| !v.is_null())
                .ok_or_else(|| SourceError::Refused {
                    message: format!(
                        "GitHub item {} was not found or does not support dependencies",
                        id.0
                    ),
                })?;
        let connection = node
            .get("blockedBy")
            .ok_or_else(|| SourceError::Malformed {
                message: "GitHub dependency response is missing its connection".into(),
            })?;
        let items = connection
            .get("nodes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|v| v.get("id").and_then(Value::as_str))
            .map(|other| DependencyEdge {
                from: NativeId(other.into()),
                to: id.clone(),
                kind: DependencyKind::Blocks,
            })
            .collect();
        Ok(Page {
            items,
            next: next_cursor(connection),
        })
    }
}

#[async_trait::async_trait]
impl TaskSource for GitHubProjectsSource {
    fn kind(&self) -> &'static str {
        KIND
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            projects: Support::Native,
            orphan_tasks: Support::Unsupported,
            filter_by_label: Support::Unsupported,
            filter_by_status: Support::Unsupported,
            search_title: Support::Unsupported,
            search_content: Support::Unsupported,
            task_dependencies: DependencySupport::ForwardOnly,
            project_dependencies: DependencySupport::ForwardOnly,
            max_page_size: MAX_PAGE_SIZE,
        }
    }
    async fn health(&self) -> Result<Health, SourceError> {
        let project = self.project_value(None, 1).await?;
        Ok(Health {
            reachable: true,
            detail: Some(format!(
                "reading GitHub project {}/{} ({})",
                self.owner,
                self.project_number,
                required_str(&project, "title")?
            )),
        })
    }
    async fn get_task(&self, id: &NativeId) -> Result<Option<Task>, SourceError> {
        Ok(self
            .all_tasks()
            .await?
            .into_iter()
            .find(|task| task.id == *id))
    }
    async fn get_project(&self, id: &NativeId) -> Result<Option<Project>, SourceError> {
        let value = self.project_value(None, 1).await?;
        let project = self.project(&value)?;
        Ok((project.id == *id).then_some(project))
    }
    async fn query_tasks(
        &self,
        _query: &TaskQuery,
        page: &PageRequest,
    ) -> Result<Page<Task>, SourceError> {
        validate_page(page)?;
        let value = self
            .project_value(page.cursor.as_ref().map(|c| c.0.as_str()), page.limit)
            .await?;
        let id = required_str(&value, "id")?;
        let items = value
            .pointer("/items/nodes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|item| self.task(id, item))
            .collect();
        Ok(Page {
            items,
            next: value.get("items").and_then(next_cursor),
        })
    }
    async fn query_projects(
        &self,
        _query: &ProjectQuery,
        page: &PageRequest,
    ) -> Result<Page<Project>, SourceError> {
        validate_page(page)?;
        if page.cursor.is_some() {
            return Ok(Page::last(vec![]));
        }
        Ok(Page::last(vec![
            self.project(&self.project_value(None, 1).await?)?,
        ]))
    }
    async fn labels(&self, page: &PageRequest) -> Result<Page<Label>, SourceError> {
        validate_page(page)?;
        let offset = numeric_cursor(page.cursor.as_ref())?;
        let mut labels = self
            .all_tasks()
            .await?
            .into_iter()
            .flat_map(|t| t.labels)
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
        self.dependencies(id, direction, page).await
    }
    async fn project_dependencies(
        &self,
        id: &NativeId,
        direction: Direction,
        page: &PageRequest,
    ) -> Result<Page<DependencyEdge>, SourceError> {
        validate_page(page)?;
        if direction == Direction::DependedOnBy {
            return Err(SourceError::Refused {
                message: "GitHub exposes forward project dependencies through Issue.blockedBy; reverse traversal must be emulated by the engine".into(),
            });
        }
        let project = self.project_value(None, 1).await?;
        if required_str(&project, "id")? != id.0 {
            return Err(SourceError::Refused {
                message: format!("GitHub project {} was not found", id.0),
            });
        }
        let mut edges = Vec::new();
        for task in self.all_tasks().await? {
            let mut cursor = None;
            loop {
                const QUERY: &str = r#"query($id:ID!,$first:Int!,$after:String){node(id:$id){... on Issue{blockedBy(first:$first,after:$after){nodes{id projectItems(first:100){nodes{project{id}}}}pageInfo{hasNextPage endCursor}}}}}"#;
                let data = self.graphql(QUERY, json!({"id":task.id.0,"first":MAX_PAGE_SIZE,"after":cursor.as_ref().map(|cursor: &Cursor| cursor.0.as_str())})).await?;
                let connection =
                    data.pointer("/node/blockedBy")
                        .ok_or_else(|| SourceError::Malformed {
                            message:
                                "GitHub project dependency response is missing Issue.blockedBy"
                                    .into(),
                        })?;
                for blocker in connection
                    .get("nodes")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    for project_item in blocker
                        .pointer("/projectItems/nodes")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                    {
                        let blocker_project = required_str(
                            project_item
                                .get("project")
                                .ok_or_else(|| SourceError::Malformed {
                                    message: "GitHub dependency project item has no project".into(),
                                })?,
                            "id",
                        )?;
                        if blocker_project != id.0 {
                            edges.push(DependencyEdge {
                                from: NativeId(blocker_project.into()),
                                to: id.clone(),
                                kind: DependencyKind::Blocks,
                            });
                        }
                    }
                }
                cursor = next_cursor(connection);
                if cursor.is_none() {
                    break;
                }
            }
        }
        let offset = numeric_cursor(page.cursor.as_ref())?;
        Ok(offset_page(edges, offset, page.limit as usize))
    }
}

fn normalize_status_mapping(
    mapping: BTreeMap<String, StatusCategory>,
) -> Result<BTreeMap<String, StatusCategory>, SourceError> {
    let mut normalized = BTreeMap::new();
    for (name, category) in mapping {
        let key = name.to_lowercase();
        if normalized.insert(key, category).is_some() {
            return Err(SourceError::Config {
                message: format!("status_mapping contains case-insensitive duplicate {name}"),
            });
        }
    }
    Ok(normalized)
}

fn required_str<'a>(value: &'a Value, field: &str) -> Result<&'a str, SourceError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| SourceError::Malformed {
            message: format!("GitHub response is missing string field {field}"),
        })
}
fn parse_time(value: Option<&Value>) -> Option<DateTime<Utc>> {
    value.and_then(Value::as_str).and_then(|v| v.parse().ok())
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
fn next_cursor(connection: &Value) -> Option<Cursor> {
    connection
        .get("pageInfo")
        .filter(|v| v.get("hasNextPage").and_then(Value::as_bool) == Some(true))
        .and_then(|v| v.get("endCursor"))
        .and_then(Value::as_str)
        .map(|v| Cursor(v.into()))
}
fn numeric_cursor(cursor: Option<&Cursor>) -> Result<usize, SourceError> {
    cursor.map_or(Ok(0), |c| {
        c.0.parse().map_err(|_| SourceError::Config {
            message: "label cursor is invalid".into(),
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
