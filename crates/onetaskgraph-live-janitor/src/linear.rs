//! Recovery of test documents after a CI machine has disappeared.
//!
//! Unlike the local lock sweep, this scheduled sweep deliberately expires documents after
//! 24 hours. Only the exact test title grammar and the nominated team's ownership authorize
//! deletion. Live journeys normally delete their own documents long before this deadline.
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use onetaskgraph_live::Credential;
use onetaskgraph_live::artifact::{CiStamp, Stamp, now_micros};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{Clock, GRAPHQL_READ_LIMIT, RealClock, Report, WRITE_INTERVAL, WRITE_LIMIT};

pub use onetaskgraph_linear::LIVE_ARTIFACT_TITLE_PREFIX as DOCUMENT_PREFIX;
pub const DOCUMENT_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);
pub const DOCUMENTS: &str = "query($after:String,$filter:DocumentFilter!){documents(first:100,after:$after,filter:$filter){nodes{id title team{key} project{teams(first:2){nodes{key} pageInfo{hasNextPage}}}} pageInfo{hasNextPage endCursor}}}";
pub const DELETE_DOCUMENT: &str = "mutation($id:String!){documentDelete(id:$id){success}}";

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
#[derive(Deserialize)]
#[serde(transparent)]
struct Nullable<T>(Option<T>);

#[derive(Deserialize)]
struct Document {
    id: DocumentId,
    title: String,
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
        self.project.0.as_ref().is_some_and(|project| {
            !project.teams.page_info.has_next_page
                && !project.teams.nodes.is_empty()
                && project
                    .teams
                    .nodes
                    .iter()
                    .all(|team| team.key.matches(nomination))
        })
    }
}
#[derive(Deserialize)]
struct Documents {
    documents: Connection<Document>,
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
    // Retained sets retire only through their invocation registration, never by age.
    if onetaskgraph_linear::live_artifact::classify_retained(title).is_some() {
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
            if old_test_title(&document.title, now) && document.in_team(&config.team) {
                listed.push(document.id)
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
    for id in listed.into_iter().take(WRITE_LIMIT) {
        if report.writes > 0 {
            clock.wait(WRITE_INTERVAL).await
        }
        report
            .write_times
            .push(clock.monotonic().as_micros() as u64);
        report.writes += 1;
        let deleted: Deleted =
            request(&client, &config, DELETE_DOCUMENT, DeleteVariables { id }).await?;
        if !deleted.document_delete.success {
            return Err("Linear did not confirm document deletion".into());
        }
    }
    Ok(report)
}
