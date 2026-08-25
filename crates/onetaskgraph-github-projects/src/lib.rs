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
//! `Issue.blockedBy` supplies `DependsOn` edges and `Issue.blocking` supplies `DependedOnBy`
//! edges; pull requests and draft issues have neither field and therefore return an empty edge
//! page. Both dependency capabilities are `BothDirections`; project dependency reads aggregate
//! the configured project's issue edges. Projects v2 has no native project-to-project relationship,
//! so those aggregate edges use the related issues' `projectItems.project.id`.
//!
//! Live verification is non-destructive by construction: [`TaskSource`] has read operations only
//! and this crate sends GraphQL `query` operations only, with no mutation for setup or teardown.
#![deny(missing_docs)]

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use onetaskgraph_plugin_api::{
    Capabilities, Cursor, DependencyEdge, DependencyEndpoint, DependencyKind, DependencySupport,
    Direction, Health, ItemKind, Label, NativeId, Page, PageRequest, Project, ProjectQuery,
    Repository, SecretResolver, SourceError, SourceName, SourcePlugin, Status, StatusCategory,
    Support, Task, TaskQuery, TaskSource,
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
/// Keeping the production documents here lets the pinned-schema test validate the same bytes
/// that are sent to GitHub, rather than a test-only copy which could drift independently.
pub mod graphql {
    /// Reads the configured project and its task page.
    pub const PROJECT: &str = r#"query($owner:String!,$number:Int!,$first:Int!,$after:String,$nestedFirst:Int!){
      owner:repositoryOwner(login:$owner){
        ... on ProjectV2Owner{projectV2(number:$number){...Project}}
      }
    } fragment Project on ProjectV2 { id title shortDescription url createdAt updatedAt closed
      items(first:$first,after:$after){nodes{id fieldValues(first:$nestedFirst){nodes{
        ... on ProjectV2ItemFieldSingleSelectValue{name field{
          ... on ProjectV2SingleSelectField{name}
        }}
        ... on ProjectV2ItemFieldTextValue{text field{... on ProjectV2Field{name}}}
        ... on ProjectV2ItemFieldLabelValue{labels(first:$nestedFirst){nodes{id name color}pageInfo{hasNextPage}}}
      }pageInfo{hasNextPage}} content{
        ... on Issue{id title body url createdAt updatedAt state repository{nameWithOwner} labels(first:$nestedFirst){nodes{id name color}pageInfo{hasNextPage}}}
        ... on PullRequest{id title body url createdAt updatedAt state repository{nameWithOwner} labels(first:$nestedFirst){nodes{id name color}pageInfo{hasNextPage}}}
        ... on DraftIssue{id title body createdAt updatedAt}
      }} pageInfo{hasNextPage endCursor}}
    }"#;
    /// Reads both dependency directions for one issue.
    pub const TASK_DEPENDENCIES: &str = r#"query($id:ID!,$first:Int!,$after:String){node(id:$id){__typename ... on Issue{blockedBy(first:$first,after:$after){nodes{id}pageInfo{hasNextPage endCursor}}blocking(first:$first,after:$after){nodes{id}pageInfo{hasNextPage endCursor}}}}}"#;
    /// Continues the projects connection for an issue related to a dependency.
    pub const RELATED_PROJECTS: &str = r#"query($id:ID!,$first:Int!,$after:String!){node(id:$id){... on Issue{projectItems(first:$first,after:$after){nodes{project{id}}pageInfo{hasNextPage endCursor}}}}}"#;
    /// Reads issue dependencies and the projects containing each related issue.
    pub const PROJECT_DEPENDENCIES: &str = r#"query($id:ID!,$first:Int!,$after:String,$nestedFirst:Int!){node(id:$id){... on Issue{blockedBy(first:$first,after:$after){nodes{id projectItems(first:$nestedFirst){nodes{project{id}}pageInfo{hasNextPage endCursor}}}pageInfo{hasNextPage endCursor}}blocking(first:$first,after:$after){nodes{id projectItems(first:$nestedFirst){nodes{project{id}}pageInfo{hasNextPage endCursor}}}pageInfo{hasNextPage endCursor}}}}}"#;
}

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
    pub owner: String, // llmlint: ignore[invalid_states_unrepresentable] Schema DTO; `new` validates GitHub's owner grammar before private construction.
    /// The project number shown in its GitHub URL.
    pub project_number: u32, // llmlint: ignore[invalid_states_unrepresentable] Schema DTO; `new` bounds this to a positive GraphQL Int.
    /// Environment variable containing a GitHub token with project read access.
    #[serde(default = "default_token_env")]
    pub token_env: String, // llmlint: ignore[invalid_states_unrepresentable] Schema DTO; `new` validates the environment-variable grammar.
    /// GraphQL endpoint. GitHub Enterprise installations may override it.
    #[serde(default = "default_endpoint")]
    pub endpoint: String, // llmlint: ignore[invalid_states_unrepresentable] Schema DTO; `new` converts it to the private validated `Url`.
    /// Case-insensitive project status name to normalized category mapping.
    #[serde(default)]
    pub status_mapping: BTreeMap<String, StatusCategory>, // llmlint: ignore[invalid_states_unrepresentable] Schema DTO; normalization validates keys and converts them to `StatusName`.
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
    owner: String, // llmlint: ignore[invalid_states_unrepresentable] Private, constructed only by `new` after full GitHub-owner validation.
    project_number: u32, // llmlint: ignore[invalid_states_unrepresentable] Private, constructed only by `new` after GraphQL-Int validation.
    endpoint: Url,
    token: SecretString,
    credential_name: String, // llmlint: ignore[invalid_states_unrepresentable] Private diagnostic value constructed only after environment-name validation.
    statuses: BTreeMap<StatusName, StatusCategory>,
    client: Client,
}

impl GitHubProjectsSource {
    /// Validate configuration and capture the named credential without exposing it.
    pub fn new(
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
            message: format!("environment variable {} is missing or empty; set it to a GitHub token with read:project and repository Issues read access", config.token_env),
        })?;
        Ok(Self {
            owner: config.owner,
            project_number: config.project_number,
            endpoint,
            token,
            credential_name: config.token_env,
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
                        "{message}; grant {} read:project and repository Issues read access",
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
    async fn project_value(
        &self,
        items_after: Option<&str>,
        items_first: u32,
    ) -> Result<Value, SourceError> {
        let data = self.graphql(graphql::PROJECT, json!({"owner":self.owner,"number":self.project_number,"first":items_first.min(MAX_PAGE_SIZE),"after":items_after,"nestedFirst":NESTED_PAGE_SIZE})).await?;
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

    fn status(&self, item: &Value) -> Result<Status, SourceError> {
        let fields = item
            .pointer("/fieldValues/nodes")
            .and_then(Value::as_array)
            .expect("task validates fieldValues.nodes before mapping status");
        let name = fields
            .iter()
            .find(|v| v.pointer("/field/name").and_then(Value::as_str) == Some("Status"))
            .map(|value| required_str(value, "name"))
            .transpose()?
            .or(optional_str(
                item.get("content").unwrap_or(&Value::Null),
                "state",
            )?)
            .unwrap_or("Unknown")
            .to_owned();
        let category = self
            .statuses
            .get(&StatusName::new(&name))
            .copied()
            .unwrap_or_else(|| match name.to_ascii_lowercase().as_str() {
                "backlog" => StatusCategory::Backlog,
                "todo" | "open" => StatusCategory::Todo,
                "in progress" | "in review" => StatusCategory::InProgress,
                "done" | "closed" | "merged" => StatusCategory::Done,
                "cancelled" | "canceled" => StatusCategory::Cancelled,
                _ => StatusCategory::Unknown,
            });
        Ok(Status { category, name })
    }

    fn labels(item: &Value) -> Result<Vec<Label>, SourceError> {
        let direct = optional_nodes(item.pointer("/content/labels"), "content labels")?;
        let field_values = item
            .pointer("/fieldValues/nodes")
            .and_then(Value::as_array)
            .expect("task validates fieldValues.nodes before mapping labels");
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

    fn task(&self, project_id: &str, item: &Value) -> Result<Option<Task>, SourceError> {
        let content = item.get("content").ok_or_else(|| SourceError::Malformed {
            message: "GitHub project item is missing content".into(),
        })?;
        if content.is_null() {
            return Ok(None);
        }
        let field_values = item
            .get("fieldValues")
            .ok_or_else(|| SourceError::Malformed {
                message: "GitHub project item is missing fieldValues".into(),
            })?;
        complete_connection(field_values, "project item field values")?;
        field_values
            .get("nodes")
            .and_then(Value::as_array)
            .ok_or_else(|| SourceError::Malformed {
                message: "GitHub project item fieldValues.nodes is not an array".into(),
            })?;
        if let Some(labels) = content.get("labels") {
            complete_connection(labels, "content labels")?;
        }
        for field_value in field_values["nodes"].as_array().expect("validated above") {
            if let Some(labels) = field_value.get("labels") {
                complete_connection(labels, "project item field labels")?;
            }
        }
        Ok(Some(Task {
            id: NativeId(required_str(content, "id")?.to_owned()),
            title: required_str(content, "title")?.to_owned(),
            content: optional_str(content, "body")?
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
            status: self.status(item)?,
            labels: Self::labels(item)?,
            project: Some(NativeId(project_id.to_owned())),
            url: optional_str(content, "url")?.map(str::to_owned),
            created_at: optional_time(content, "createdAt")?,
            updated_at: optional_time(content, "updatedAt")?,
            metadata: metadata_field(field_values)?,
            repositories: repositories(content, field_values)?,
        }))
    }

    fn project(&self, value: &Value) -> Result<Project, SourceError> {
        let id = required_str(value, "id")?;
        let (content, metadata) =
            metadata_description(optional_str(value, "shortDescription")?.map(str::to_owned))?;
        let repositories = repositories_from_metadata(&metadata)?;
        Ok(Project {
            id: NativeId(id.into()),
            title: required_str(value, "title")?.into(),
            content,
            status: Status {
                category: if required_bool(value, "closed")? {
                    StatusCategory::Done
                } else {
                    StatusCategory::InProgress
                },
                name: if required_bool(value, "closed")? {
                    "Closed"
                } else {
                    "Open"
                }
                .into(),
            },
            labels: vec![],
            url: optional_str(value, "url")?.map(str::to_owned),
            created_at: optional_time(value, "createdAt")?,
            updated_at: optional_time(value, "updatedAt")?,
            metadata,
            repositories,
        })
    }

    async fn all_tasks(&self) -> Result<Vec<Task>, SourceError> {
        let mut after = None;
        let mut tasks = Vec::new();
        loop {
            let project = self.project_value(after.as_deref(), MAX_PAGE_SIZE).await?;
            let project_id = required_str(&project, "id")?;
            let items = project
                .pointer("/items/nodes")
                .and_then(Value::as_array)
                .ok_or_else(|| SourceError::Malformed {
                    message: "GitHub project items.nodes is not an array".into(),
                })?;
            for item in items {
                if let Some(task) = self.task(project_id, item)? {
                    tasks.push(task);
                }
            }
            let page =
                project
                    .pointer("/items/pageInfo")
                    .ok_or_else(|| SourceError::Malformed {
                        message: "GitHub project items have no pageInfo".into(),
                    })?;
            if !required_bool(page, "hasNextPage")? {
                break;
            }
            let next = required_str(page, "endCursor")?;
            validate_cursor_progress(after.as_deref(), next)?;
            after = Some(next.to_owned());
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
        let data = self.graphql(graphql::TASK_DEPENDENCIES, json!({"id":id.0,"first":page.limit.min(MAX_PAGE_SIZE),"after":page.cursor.as_ref().map(|c| c.0.as_str())})).await?;
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
        if required_str(node, "__typename")? != "Issue" {
            return Ok(Page {
                items: Vec::new(),
                next: None,
            });
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
        let items = nodes
            .iter()
            .map(|value| {
                let related = NativeId(required_str(value, "id")?.into());
                Ok(match direction {
                    Direction::DependsOn => DependencyEdge {
                        from: DependencyEndpoint {
                            id: related.0,
                            kind: ItemKind::Task,
                        },
                        to: DependencyEndpoint {
                            id: id.0.clone(),
                            kind: ItemKind::Task,
                        },
                        kind: DependencyKind::Blocks,
                    },
                    Direction::DependedOnBy => DependencyEdge {
                        from: DependencyEndpoint {
                            id: id.0.clone(),
                            kind: ItemKind::Task,
                        },
                        to: DependencyEndpoint {
                            id: related.0,
                            kind: ItemKind::Task,
                        },
                        kind: DependencyKind::Blocks,
                    },
                })
            })
            .collect::<Result<Vec<_>, SourceError>>()?;
        let next = next_cursor(connection)?;
        if let Some(next) = &next {
            validate_cursor_progress(
                page.cursor.as_ref().map(|cursor| cursor.0.as_str()),
                &next.0,
            )?;
        }
        Ok(Page { items, next })
    }

    async fn related_issue_projects(&self, issue: &Value) -> Result<Vec<NativeId>, SourceError> {
        let issue_id = required_str(issue, "id")?;
        let mut connection =
            issue
                .get("projectItems")
                .cloned()
                .ok_or_else(|| SourceError::Malformed {
                    message: "GitHub related issue is missing projectItems".into(),
                })?;
        let mut projects = Vec::new();
        let mut previous = None;
        loop {
            let nodes = connection
                .get("nodes")
                .and_then(Value::as_array)
                .ok_or_else(|| SourceError::Malformed {
                    message: "GitHub related issue projectItems.nodes is not an array".into(),
                })?;
            for item in nodes {
                projects.push(NativeId(
                    required_str(
                        item.get("project").ok_or_else(|| SourceError::Malformed {
                            message: "GitHub dependency project item has no project".into(),
                        })?,
                        "id",
                    )?
                    .into(),
                ));
            }
            let Some(cursor) = next_cursor(&connection)? else {
                break;
            };
            validate_cursor_progress(previous.as_deref(), &cursor.0)?;
            previous = Some(cursor.0.clone());
            let data = self
                .graphql(
                    graphql::RELATED_PROJECTS,
                    json!({"id":issue_id,"first":MAX_PAGE_SIZE,"after":cursor.0}),
                )
                .await?;
            connection = data.pointer("/node/projectItems").cloned().ok_or_else(|| {
                SourceError::Malformed {
                    message: "GitHub related issue response is missing projectItems".into(),
                }
            })?;
        }
        Ok(projects)
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
            task_dependencies: DependencySupport::BothDirections,
            project_dependencies: DependencySupport::BothDirections,
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
        let items_connection = value.get("items").ok_or_else(|| SourceError::Malformed {
            message: "GitHub project response is missing items".into(),
        })?;
        let nodes = items_connection
            .get("nodes")
            .and_then(Value::as_array)
            .ok_or_else(|| SourceError::Malformed {
                message: "GitHub project items.nodes is not an array".into(),
            })?;
        let items = nodes
            .iter()
            .map(|item| self.task(id, item))
            .collect::<Result<Vec<_>, SourceError>>()?
            .into_iter()
            .flatten()
            .collect();
        let next = next_cursor(items_connection)?;
        if let Some(next) = &next {
            validate_cursor_progress(
                page.cursor.as_ref().map(|cursor| cursor.0.as_str()),
                &next.0,
            )?;
        }
        Ok(Page { items, next })
    }
    async fn query_projects(
        &self,
        _query: &ProjectQuery,
        page: &PageRequest,
    ) -> Result<Page<Project>, SourceError> {
        validate_page(page)?;
        if page.cursor.is_some() {
            return Err(SourceError::Config {
                message: "GitHub project listing does not issue page cursors".into(),
            });
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
                let data = self.graphql(graphql::PROJECT_DEPENDENCIES, json!({"id":task.id.0,"first":MAX_PAGE_SIZE,"after":cursor.as_ref().map(|cursor: &Cursor| cursor.0.as_str()),"nestedFirst":MAX_PAGE_SIZE})).await?;
                let connection_name = match direction {
                    Direction::DependsOn => "blockedBy",
                    Direction::DependedOnBy => "blocking",
                };
                let Some(connection) = data.pointer(&format!("/node/{connection_name}")) else {
                    // Pull requests and draft issues are valid project tasks, but the inline
                    // `... on Issue` selection intentionally yields no dependency connection.
                    break;
                };
                let related_issues = connection
                    .get("nodes")
                    .and_then(Value::as_array)
                    .ok_or_else(|| SourceError::Malformed {
                        message: "GitHub project dependency nodes is not an array".into(),
                    })?;
                for related_issue in related_issues {
                    for related in self.related_issue_projects(related_issue).await? {
                        if related != *id {
                            edges.push(match direction {
                                Direction::DependsOn => DependencyEdge {
                                    from: DependencyEndpoint {
                                        id: related.0,
                                        kind: ItemKind::Project,
                                    },
                                    to: DependencyEndpoint {
                                        id: id.0.clone(),
                                        kind: ItemKind::Project,
                                    },
                                    kind: DependencyKind::Blocks,
                                },
                                Direction::DependedOnBy => DependencyEdge {
                                    from: DependencyEndpoint {
                                        id: id.0.clone(),
                                        kind: ItemKind::Project,
                                    },
                                    to: DependencyEndpoint {
                                        id: related.0,
                                        kind: ItemKind::Project,
                                    },
                                    kind: DependencyKind::Blocks,
                                },
                            });
                        }
                    }
                }
                let next = next_cursor(connection)?;
                if let Some(next) = &next {
                    validate_cursor_progress(
                        cursor.as_ref().map(|value: &Cursor| value.0.as_str()),
                        &next.0,
                    )?;
                }
                cursor = next;
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
) -> Result<BTreeMap<StatusName, StatusCategory>, SourceError> {
    let mut normalized = BTreeMap::new();
    for (name, category) in mapping {
        if name.trim().is_empty() {
            return Err(SourceError::Config {
                message: "status_mapping contains a blank status name".into(),
            });
        }
        let key = StatusName::new(&name);
        if normalized.insert(key, category).is_some() {
            return Err(SourceError::Config {
                message: format!("status_mapping contains case-insensitive duplicate {name}"),
            });
        }
    }
    Ok(normalized)
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

fn valid_environment_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct StatusName(String);

impl StatusName {
    fn new(name: &str) -> Self {
        Self(name.to_lowercase())
    }
}

fn required_str<'a>(value: &'a Value, field: &str) -> Result<&'a str, SourceError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| SourceError::Malformed {
            message: format!("GitHub response is missing string field {field}"),
        })
}

const METADATA_FIELD: &str = "onetaskgraph.metadata";

fn metadata_field(field_values: &Value) -> Result<BTreeMap<String, Value>, SourceError> {
    let nodes = field_values
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| SourceError::Malformed {
            message: "GitHub project item fieldValues.nodes is not an array".into(),
        })?;
    let Some(text) = nodes
        .iter()
        .find(|node| node.pointer("/field/name").and_then(Value::as_str) == Some(METADATA_FIELD))
        .and_then(|node| node.get("text"))
    else {
        return Ok(BTreeMap::new());
    };
    serde_json::from_value(text.clone())
        .or_else(|_| {
            text.as_str()
                .ok_or_else(|| {
                    serde_json::Error::io(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "metadata text is not a string",
                    ))
                })
                .and_then(serde_json::from_str)
        })
        .map_err(|error| SourceError::Malformed {
            message: format!(
                "GitHub {METADATA_FIELD} field is not canonical JSON metadata: {error}"
            ),
        })
}

fn repositories(content: &Value, field_values: &Value) -> Result<Vec<Repository>, SourceError> {
    if let Some(origin) = content
        .pointer("/repository/nameWithOwner")
        .and_then(Value::as_str)
    {
        return Ok(vec![Repository(format!("github.com/{origin}"))]);
    }
    let metadata = metadata_field(field_values)?;
    metadata
        .get("onetaskgraph.repositories")
        .map_or(Ok(Vec::new()), |value| {
            serde_json::from_value(value.clone()).map_err(|error| SourceError::Malformed {
                message: format!(
                    "onetaskgraph.repositories is not a list of repository origins: {error}"
                ),
            })
        })
}

const PROJECT_METADATA_OPEN: &str = "<!-- onetaskgraph.metadata\n";
const PROJECT_METADATA_CLOSE: &str = "\n-->";

fn metadata_description(
    description: Option<String>,
) -> Result<(Option<String>, BTreeMap<String, Value>), SourceError> {
    let Some(description) = description else {
        return Ok((None, BTreeMap::new()));
    };
    let Some(start) = description.rfind(PROJECT_METADATA_OPEN) else {
        return Ok((Some(description), BTreeMap::new()));
    };
    let value_start = start + PROJECT_METADATA_OPEN.len();
    let Some(relative_end) = description[value_start..].find(PROJECT_METADATA_CLOSE) else {
        return Err(SourceError::Malformed {
            message: "unterminated onetaskgraph metadata slot in GitHub project description".into(),
        });
    };
    let value_end = value_start + relative_end;
    let metadata = serde_json::from_str(&description[value_start..value_end]).map_err(|error| {
        SourceError::Malformed {
            message: format!("invalid canonical JSON in GitHub project metadata slot: {error}"),
        }
    })?;
    let visible = description[..start].trim_end();
    Ok(((!visible.is_empty()).then(|| visible.into()), metadata))
}

fn repositories_from_metadata(
    metadata: &BTreeMap<String, Value>,
) -> Result<Vec<Repository>, SourceError> {
    metadata
        .get("onetaskgraph.repositories")
        .map_or(Ok(Vec::new()), |value| {
            serde_json::from_value(value.clone()).map_err(|error| SourceError::Malformed {
                message: format!(
                    "onetaskgraph.repositories is not a list of repository origins: {error}"
                ),
            })
        })
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
