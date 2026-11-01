//! Recovery of test documents after a CI machine has disappeared.
//!
//! Unlike the local lock sweep, this scheduled sweep deliberately expires documents after
//! 24 hours. Only the exact test title grammar and the nominated team's ownership authorize
//! deletion. Live journeys normally delete their own documents long before this deadline.
//!
//! It also retires the one standing set the current scheme superseded: the pre-pool CI set
//! (`retained-v1 ci <TEAM> <slot>`, with no `pool-N`), four documents and their parent
//! project, which no lane creates, reads or owns since the pool replaced it. Every other
//! retained or lease title — a pool member's, a local invocation's, or one no scheme parses —
//! is never touched here.
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use onetaskgraph_linear::live_artifact::{Slot, classify_retained, lease_created_micros};
use onetaskgraph_live::Credential;
use onetaskgraph_live::artifact::{CiStamp, Stamp, now_micros};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

use crate::{Clock, GRAPHQL_READ_LIMIT, RealClock, Report, WRITE_INTERVAL, WRITE_LIMIT};

pub use onetaskgraph_linear::LIVE_ARTIFACT_TITLE_PREFIX as DOCUMENT_PREFIX;
pub const DOCUMENT_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);
pub const DOCUMENTS: &str = "query($after:String,$filter:DocumentFilter!){documents(first:100,after:$after,filter:$filter){nodes{id title updatedAt team{key} project{teams(first:2){nodes{key} pageInfo{hasNextPage}}}} pageInfo{hasNextPage endCursor}}}";
pub const DELETE_DOCUMENT: &str = "mutation($id:String!){documentDelete(id:$id){success}}";
/// Lists the projects whose name begins with the test prefix, for the superseded parent.
pub const PROJECTS: &str = "query($after:String,$filter:ProjectFilter!){projects(first:100,after:$after,filter:$filter){nodes{id name updatedAt teams(first:2){nodes{key} pageInfo{hasNextPage}}} pageInfo{hasNextPage endCursor}}}";
pub const DELETE_PROJECT: &str = "mutation($id:String!){projectDelete(id:$id){success}}";

fn valid_text(value: String) -> Result<String, String> {
    if value.trim().is_empty() || value.chars().any(char::is_control) {
        Err("Linear identifier or nomination is empty or contains control characters".into())
    } else {
        Ok(value)
    }
}

#[derive(Deserialize)]
#[serde(try_from = "String")]
struct TeamKey(String);
impl TryFrom<String> for TeamKey {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        valid_text(value).map(Self)
    }
}
impl TeamKey {
    fn matches(&self, other: &Self) -> bool {
        self.0.eq_ignore_ascii_case(&other.0)
    }
}

#[derive(Clone, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(try_from = "String")]
struct DocumentId(String);
impl TryFrom<String> for DocumentId {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        valid_text(value).map(Self)
    }
}
#[derive(Clone, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(try_from = "String")]
struct Cursor(String);
impl TryFrom<String> for Cursor {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        valid_text(value).map(Self)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WirePageInfo {
    has_next_page: bool,
    end_cursor: Option<Cursor>,
}
#[derive(Deserialize)]
#[serde(try_from = "WirePageInfo")]
enum PageInfo {
    Complete,
    More(Cursor),
}
impl TryFrom<WirePageInfo> for PageInfo {
    type Error = String;
    fn try_from(value: WirePageInfo) -> Result<Self, String> {
        if value.has_next_page {
            value
                .end_cursor
                .map(Self::More)
                .ok_or("Linear omitted advancing cursor".into())
        } else {
            Ok(Self::Complete)
        }
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TeamPageInfo {
    has_next_page: bool,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Teams {
    nodes: Vec<Team>,
    page_info: TeamPageInfo,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Connection<T> {
    nodes: Vec<T>,
    page_info: PageInfo,
}
#[derive(Deserialize)]
struct Team {
    key: TeamKey,
}
#[derive(Deserialize)]
struct Project {
    teams: Teams,
}
impl Teams {
    /// Every team is the nomination, and the page holds them all.
    fn only(&self, nomination: &TeamKey) -> bool {
        !self.page_info.has_next_page
            && !self.nodes.is_empty()
            && self.nodes.iter().all(|team| team.key.matches(nomination))
    }
}
#[derive(Deserialize)]
#[serde(transparent)]
struct Nullable<T>(Option<T>);

/// When Linear last changed a record, in microseconds since the epoch, or `None` for a time
/// that does not read — which is never taken for an old one.
#[derive(Deserialize)]
#[serde(from = "String")]
struct UpdatedAt(Option<u64>);
impl From<String> for UpdatedAt {
    fn from(value: String) -> Self {
        Self(lease_created_micros(&value))
    }
}
impl UpdatedAt {
    /// Whether Linear last changed it longer than [`DOCUMENT_MAX_AGE`] ago. A run still on a
    /// branch from before the pool writes its set every pass, so this keeps that set in place
    /// while such a run may be using it.
    fn untouched(&self, now: u64) -> bool {
        self.0
            .is_some_and(|micros| now.saturating_sub(micros) > DOCUMENT_MAX_AGE.as_micros() as u64)
    }
}

#[derive(Deserialize)]
struct Document {
    id: DocumentId,
    title: String,
    #[serde(rename = "updatedAt")]
    updated_at: UpdatedAt,
    #[serde(deserialize_with = "Nullable::deserialize")]
    team: Nullable<Team>,
    #[serde(deserialize_with = "Nullable::deserialize")]
    project: Nullable<Project>,
}
impl Document {
    fn in_team(&self, nomination: &TeamKey) -> bool {
        if let Some(team) = &self.team.0 {
            return team.key.matches(nomination);
        }
        self.project
            .0
            .as_ref()
            .is_some_and(|project| project.teams.only(nomination))
    }
}
#[derive(Deserialize)]
struct Documents {
    documents: Connection<Document>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListedProject {
    id: DocumentId,
    name: String,
    updated_at: UpdatedAt,
    teams: Teams,
}
#[derive(Deserialize)]
struct Projects {
    projects: Connection<ListedProject>,
}
#[derive(Deserialize)]
struct Confirmation {
    success: bool,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Deleted {
    document_delete: Confirmation,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProjectDeleted {
    project_delete: Confirmation,
}
#[derive(Deserialize)]
struct GraphqlError {
    #[serde(rename = "message")]
    _message: String,
}
#[derive(Deserialize)]
struct Response<T> {
    data: Option<T>,
    errors: Option<Vec<GraphqlError>>,
}
#[derive(Serialize)]
struct Request<V> {
    query: &'static str,
    variables: V,
}
#[derive(Serialize)]
struct DocumentVariables {
    after: Option<Cursor>,
    filter: DocumentFilter,
}
#[derive(Serialize)]
struct ProjectVariables {
    after: Option<Cursor>,
    filter: ProjectFilter,
}
/// The typed predicate sent when listing stamped Linear projects.
#[derive(Serialize)]
pub struct ProjectFilter {
    name: TitleFilter,
}
/// The listing predicate serialized on every Linear project request.
pub fn project_filter() -> ProjectFilter {
    ProjectFilter {
        name: TitleFilter {
            starts_with: DOCUMENT_PREFIX,
        },
    }
}
/// The typed predicate sent when listing stamped Linear documents.
#[derive(Serialize)]
pub struct DocumentFilter {
    title: TitleFilter,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TitleFilter {
    starts_with: &'static str,
}
/// The listing predicate serialized on every Linear document request.
pub fn document_filter() -> DocumentFilter {
    DocumentFilter {
        title: TitleFilter {
            starts_with: DOCUMENT_PREFIX,
        },
    }
}

#[derive(Serialize)]
struct DeleteVariables {
    id: DocumentId,
}

/// The explicitly nominated test team and a credential that never appears in diagnostics.
pub struct LinearConfig {
    endpoint: reqwest::Url,
    key: Credential,
    team: TeamKey,
}
impl LinearConfig {
    pub fn production(key: &str, team: &str) -> Result<Self, String> {
        Self::at("https://api.linear.app/graphql", key, team)
    }
    pub fn loopback(origin: &str, key: &str, team: &str) -> Result<Self, String> {
        let validated =
            crate::Config::loopback(origin, key, crate::Ownership::completed_runs("offline")?)?;
        Self::at(validated.graphql.as_str(), key, team)
    }
    fn at(endpoint: &str, key: &str, team: &str) -> Result<Self, String> {
        Ok(Self {
            endpoint: reqwest::Url::parse(endpoint).map_err(|e| e.to_string())?,
            key: Credential::new(key).ok_or("LINEAR_API_KEY is required")?,
            team: TeamKey::try_from(team.to_owned())?,
        })
    }
}

async fn request<V: Serialize, T: DeserializeOwned>(
    client: &reqwest::Client,
    config: &LinearConfig,
    query: &'static str,
    variables: V,
) -> Result<T, String> {
    let body: Response<T> = client
        .post(config.endpoint.clone())
        .header("Authorization", config.key.expose())
        .json(&Request { query, variables })
        .send()
        .await
        .map_err(|_| "Linear request failed")?
        .error_for_status()
        .map_err(|_| "Linear HTTP request refused")?
        .json()
        .await
        .map_err(|_| "Linear returned invalid or malformed JSON")?;
    if body.errors.is_some_and(|errors| !errors.is_empty()) {
        return Err("Linear GraphQL request refused".into());
    }
    body.data.ok_or("Linear returned no data".into())
}

fn old_test_title(title: &str, now: u64) -> bool {
    // Retained sets never expire by a stamp's age: a local set retires through its invocation
    // registration and the superseded pre-pool set by `superseded_slot`. Pool members, leases
    // and any malformed or future version of either, which confer nothing, are never removed.
    if onetaskgraph_linear::live_artifact::is_reserved_title(title) {
        return false;
    }
    let Some(suffix) = title.strip_prefix(DOCUMENT_PREFIX) else {
        return false;
    };
    let micros = Stamp::read(suffix)
        .filter(|stamp| stamp.run().host().is_some())
        .map(Stamp::micros)
        .or_else(|| CiStamp::read(suffix).map(CiStamp::micros));
    micros.is_some_and(|micros| {
        micros > 0 && now.saturating_sub(micros) > DOCUMENT_MAX_AGE.as_micros() as u64
    })
}

/// The slot `title` names when it is the nominated team's pre-pool standing CI set, which the
/// pool superseded and nothing owns; `None` for every other title, pool members, local sets
/// and unparsed retained titles included.
fn superseded_slot(title: &str, nomination: &TeamKey) -> Option<Slot> {
    let retained = classify_retained(title)?;
    let scope = retained.scope;
    (scope.is_ci()
        && scope.pool_index().is_none()
        && scope.team().eq_ignore_ascii_case(&nomination.0))
    .then_some(retained.slot)
}

enum Retire {
    Document(DocumentId),
    Project(DocumentId),
}

pub async fn run_linear(config: LinearConfig) -> Result<Report, String> {
    run_linear_with_clock(config, Arc::new(RealClock(Instant::now()))).await
}
/// Same interface and requests with only the monotonic pacing clock controlled offline.
pub async fn run_linear_with_clock(
    config: LinearConfig,
    clock: Arc<dyn Clock>,
) -> Result<Report, String> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    let now = now_micros();
    let mut report = Report::default();
    let mut after = None;
    let mut cursors = BTreeSet::new();
    let mut ids = BTreeSet::new();
    let mut listed = Vec::new();
    // Whether a superseded document stays, which keeps its parent project too.
    let mut superseded_kept = false;
    // Finish enumeration before deleting: a failed or cyclic listing authorizes no writes.
    loop {
        if report.graphql_queries == GRAPHQL_READ_LIMIT {
            return Err("Linear document listing exceeded its read limit".into());
        }
        report.graphql_queries += 1;
        let data: Documents = request(
            &client,
            &config,
            DOCUMENTS,
            DocumentVariables {
                after,
                filter: document_filter(),
            },
        )
        .await?;
        for document in data.documents.nodes {
            if !ids.insert(document.id.clone()) {
                return Err("Linear document listing repeated an id".into());
            }
            // A document titled as the parent is nothing a lane made, and is left alone.
            let superseded = superseded_slot(&document.title, &config.team)
                .is_some_and(|slot| slot != Slot::Parent);
            if superseded {
                if document.in_team(&config.team) && document.updated_at.untouched(now) {
                    listed.push(Retire::Document(document.id))
                } else {
                    superseded_kept = true;
                }
            } else if old_test_title(&document.title, now) && document.in_team(&config.team) {
                listed.push(Retire::Document(document.id))
            }
        }
        let next = match data.documents.page_info {
            PageInfo::Complete => break,
            PageInfo::More(next) => next,
        };
        if !cursors.insert(next.clone()) {
            return Err("Linear document cursor did not advance".into());
        }
        after = Some(next);
    }
    // The superseded parent project, listed whole before any write too.
    let mut after = None;
    let mut cursors = BTreeSet::new();
    let mut projects = Vec::new();
    loop {
        if report.graphql_queries == GRAPHQL_READ_LIMIT {
            return Err("Linear project listing exceeded its read limit".into());
        }
        report.graphql_queries += 1;
        let data: Projects = request(
            &client,
            &config,
            PROJECTS,
            ProjectVariables {
                after,
                filter: project_filter(),
            },
        )
        .await?;
        for project in data.projects.nodes {
            if !ids.insert(project.id.clone()) {
                return Err("Linear project listing repeated an id".into());
            }
            if superseded_slot(&project.name, &config.team) == Some(Slot::Parent)
                && project.teams.only(&config.team)
                && project.updated_at.untouched(now)
            {
                projects.push(Retire::Project(project.id))
            }
        }
        let next = match data.projects.page_info {
            PageInfo::Complete => break,
            PageInfo::More(next) => next,
        };
        if !cursors.insert(next.clone()) {
            return Err("Linear project cursor did not advance".into());
        }
        after = Some(next);
    }
    // Documents first, so a project goes only after the documents filed under it; and none
    // while a superseded document stays.
    if !superseded_kept {
        listed.extend(projects);
    }
    for target in listed.into_iter().take(WRITE_LIMIT) {
        if report.writes > 0 {
            clock.wait(WRITE_INTERVAL).await
        }
        report
            .write_times
            .push(clock.monotonic().as_micros() as u64);
        report.writes += 1;
        match target {
            Retire::Document(id) => {
                let deleted: Deleted =
                    request(&client, &config, DELETE_DOCUMENT, DeleteVariables { id }).await?;
                if !deleted.document_delete.success {
                    return Err("Linear did not confirm document deletion".into());
                }
            }
            Retire::Project(id) => {
                let deleted: ProjectDeleted =
                    request(&client, &config, DELETE_PROJECT, DeleteVariables { id }).await?;
                if !deleted.project_delete.success {
                    return Err("Linear did not confirm project deletion".into());
                }
            }
        }
    }
    Ok(report)
}

/// Reads one lease issue back, for `--release-leases`.
pub const LEASE_READ: &str = "query($id:String!){issue(id:$id){id title}}";
/// Retitles one lease issue released, for `--release-leases`.
pub const LEASE_RELEASE: &str =
    "mutation($id:String!,$input:IssueUpdateInput!){issueUpdate(id:$id,input:$input){success}}";

/// Releases every pool lease a cancelled job recorded in `directory`
/// (`ONETASKGRAPH_LINEAR_LEASE_DIR`), and nothing else: a recorded lease is released only when
/// it still reads back exactly as recorded — this run's holder, this generation — which is the
/// positive evidence that it is still the cancelled run's. One Linear answers absent, or that
/// reads otherwise, was superseded or released already, and its record is settled without a
/// write; an answer that is neither stops the pass with the record kept. An absent directory is
/// a run that held no lease.
pub async fn release_recorded_leases(
    config: LinearConfig,
    directory: &std::path::Path,
) -> Result<Report, String> {
    use onetaskgraph_linear::live_artifact::{Lease, LeaseHolder, LeaseRecord, lease_title};
    let mut report = Report::default();
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(report),
        Err(error) => return Err(format!("the lease records could not be read: {error}")),
    };
    let client = reqwest::Client::new();
    let team = config.team.0.clone();
    for entry in entries {
        let path = entry.map_err(|error| error.to_string())?.path();
        let record = LeaseRecord::read(
            &std::fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?,
        )
        .map_err(|error| format!("{}: {error}", path.display()))?;
        let lease = record.lease()?;
        if !lease.team().eq_ignore_ascii_case(&team) {
            return Err(format!(
                "{} records no held lease of {team}",
                path.display()
            ));
        }
        let id = record.id();
        let read: Value = request(&client, &config, LEASE_READ, json!({"id":id})).await?;
        report.graphql_queries += 1;
        let still = match read.get("issue") {
            Some(Value::Null) => false,
            Some(issue) => match (
                issue.get("id").and_then(Value::as_str),
                issue.get("title").and_then(Value::as_str),
            ) {
                (Some(answered), Some(title)) if answered == id => title == record.title(),
                _ => {
                    return Err(format!(
                        "Linear answered lease {id} with an issue it does not identify: {issue}; \
                         the record is kept"
                    ));
                }
            },
            None => {
                return Err(format!(
                    "Linear answered lease {id} with no issue at all; the record is kept"
                ));
            }
        };
        if still {
            let released = lease_title(Lease::new(
                lease.scope(),
                lease.lane(),
                std::num::NonZeroU32::new(lease.generation()).ok_or("generation 0")?,
                LeaseHolder::Released,
            )?);
            let answer: Value = request(
                &client,
                &config,
                LEASE_RELEASE,
                json!({"id":id,"input":{"title":released}}),
            )
            .await?;
            if answer.pointer("/issueUpdate/success") != Some(&json!(true)) {
                return Err(format!(
                    "Linear did not confirm releasing lease {id}; the record is kept"
                ));
            }
            report.writes += 1;
        }
        std::fs::remove_file(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    }
    Ok(report)
}
