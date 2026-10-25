//! Scheduled residue cleanup, separate from the credentialed test lane.
//!
//! Scratch CI artifacts require their owning Actions run to read `completed` immediately
//! before their delete batch, and that read is the whole of the rule: there is no waiting
//! period, so a finished run's artifacts go on the first pass after it completes, whatever
//! their age. A board refused for its item count by the live lane must not wait a day for
//! residue whose run is already over. Machine stamps stay the writing machine's lock sweep's.
//!
//! Every kind of artifact the GitHub Projects lane writes under a CI stamp is listed: its
//! issues, their board items, its repository label, and its scratch board field — which
//! matters because the lane refuses a board whose field count differs from its registered
//! workload.
//!
//! [`Ownership::CancelledRun`] is the same cleanup asked by a run about itself: the step
//! `ci.yml` runs when a live run is cancelled removes exactly that run attempt's stamps,
//! while its run still reads in progress, and reads no ownership at all.
//! Linear keeps its machine stamps and lock sweep everywhere; this janitor never reads it.
//!
//! Every REST route, query parameter and response field this crate uses is held to GitHub's
//! published description: `tests/fixtures/rest-operations.json` is that description reduced
//! to these operations, pinned with its source commit, and the offline journeys refuse any
//! request the janitor sends, or body the stand-in serves, that the pin does not admit.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::{Duration, Instant};

use onetaskgraph_github_live::{
    BOARD_NUMBER, BOARD_OWNER, DELETE_FIELD_DOCUMENT, RunStatus, SCRATCH_REPOSITORY, fielded_stamp,
    labelled_stamp, run_path, titled_stamp,
};
use onetaskgraph_github_projects::DESIGN_TITLE_PREFIX;
use onetaskgraph_live::artifact::{CiRun, CiStamp};
use onetaskgraph_live::{Allowance, Credential, Demand, Metered, affordable};
use serde_json::{Value, json};

/// Runtime limits; these are not performance budgets.
pub const WRITE_LIMIT: usize = 150;
pub const REST_READ_LIMIT: usize = 250;
pub const GRAPHQL_READ_LIMIT: usize = 50;
/// One content request per second at most, across delete batches.
pub const WRITE_INTERVAL: Duration = Duration::from_secs(1);
const BATCH_SIZE: usize = 25;

/// The monotonic clock used by pacing. Ownership reads no clock at all.
pub trait Clock: Send + Sync {
    fn monotonic(&self) -> Duration;
    fn wait(&self, duration: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;
}

struct RealClock(Instant);
impl Clock for RealClock {
    fn monotonic(&self) -> Duration {
        self.0.elapsed()
    }
    fn wait(&self, duration: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(tokio::time::sleep(duration))
    }
}

/// A controllable clock for offline loopback journeys. It advances by exactly the wait
/// requested by the production pacer; it changes no ownership decision or request.
#[derive(Default)]
pub struct VirtualClock(AtomicU64);
impl Clock for VirtualClock {
    fn monotonic(&self) -> Duration {
        Duration::from_micros(self.0.load(Ordering::SeqCst))
    }
    fn wait(&self, duration: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            self.0
                .fetch_add(duration.as_micros() as u64, Ordering::SeqCst);
        })
    }
}

/// What authorises a listed artifact's removal.
pub enum Ownership {
    /// The scheduled pass: an artifact goes once its run reads `completed` — succeeded,
    /// failed or cancelled alike — in a read made with this Actions token immediately before
    /// its delete batch.
    CompletedRuns { actions_token: Credential },
    /// A cancelled run cleaning up after itself: exactly the artifacts stamped with this run
    /// and attempt go, and no other run's, whatever its status. No ownership read is made,
    /// because the run asking is the one that wrote them and still reads in progress.
    CancelledRun(CiRun),
}

impl Ownership {
    /// The scheduled pass's rule, reading runs with `actions_token`.
    pub fn completed_runs(actions_token: &str) -> Result<Self, String> {
        Ok(Self::CompletedRuns {
            actions_token: Credential::new(actions_token).ok_or("GITHUB_TOKEN is required")?,
        })
    }
}

/// Endpoints and credentials validated together before an invocation can reach GitHub.
/// Construction admits the production host or an explicitly selected loopback origin.
pub struct Config {
    rest: reqwest::Url,
    graphql: reqwest::Url,
    write_token: Credential,
    ownership: Ownership,
}

impl Config {
    pub fn github(write_token: &str, ownership: Ownership) -> Result<Self, String> {
        Self::at(
            reqwest::Url::parse("https://api.github.com/").map_err(|e| e.to_string())?,
            write_token,
            ownership,
        )
    }

    pub fn loopback(origin: &str, write_token: &str, ownership: Ownership) -> Result<Self, String> {
        let origin = reqwest::Url::parse(origin).map_err(|e| e.to_string())?;
        if origin.scheme() != "http"
            || origin.host_str() != Some("127.0.0.1")
            || !origin.username().is_empty()
            || origin.password().is_some()
            || origin.query().is_some()
            || origin.fragment().is_some()
            || origin.path() != "/"
        {
            return Err("offline endpoint must be an HTTP IPv4 loopback origin".into());
        }
        Self::at(origin, write_token, ownership)
    }

    fn at(rest: reqwest::Url, write_token: &str, ownership: Ownership) -> Result<Self, String> {
        Ok(Self {
            graphql: rest.join("graphql").map_err(|e| e.to_string())?,
            rest,
            write_token: Credential::new(write_token).ok_or("GH_PROJECTS_TOKEN is required")?,
            ownership,
        })
    }
}

/// Why a REST read did not answer. Exhausting the read limit is the run's own bound and ends
/// it; any other failure is about the one thing read.
enum ReadError {
    LimitExhausted(String),
    Failed(String),
}
impl From<ReadError> for String {
    fn from(error: ReadError) -> Self {
        match error {
            ReadError::LimitExhausted(message) | ReadError::Failed(message) => message,
        }
    }
}

#[derive(Clone, Copy)]
enum CredentialRole {
    Write,
    Actions,
}

fn identifier(value: &Value, field: &str) -> Result<String, String> {
    let value = text(value, field)?;
    if value.is_empty() || value.chars().any(char::is_control) {
        return Err(format!("invalid {field}"));
    }
    Ok(value.to_owned())
}
#[derive(Clone)]
struct IssueId(String);
#[derive(Clone)]
struct ItemId(String);
#[derive(Clone)]
struct FieldId(String);
#[derive(Clone)]
struct BoardId(String);
#[derive(Clone)]
struct LabelName(String);
/// A board page's `endCursor`: present and non-empty, or there is no next page to ask for.
struct Cursor(String);

/// One page of the nominated board's items, with what recognising residue needs of each, and
/// the board's fields, which are read off the first page alone.
///
/// The field connection does not page with the items, so a later page answers the same
/// fields again and they are not listed twice. GitHub caps a board below a hundred fields;
/// a connection reporting another page fails closed.
///
/// This and the mutations below are validated against the GitHub Projects plugin's
/// pinned schema by `tests/schema.rs`, so a field GitHub does not have fails offline.
pub const BOARD_DOCUMENT: &str = "query($owner:String!,$number:Int!,$after:String){board:repositoryOwner(login:$owner){... on ProjectV2Owner{projectV2(number:$number){id fields(first:100){nodes{... on ProjectV2Field{id name} ... on ProjectV2SingleSelectField{id name}} pageInfo{hasNextPage}} items(first:100,after:$after){nodes{id content{__typename ... on Issue{title repository{nameWithOwner}}}} pageInfo{hasNextPage endCursor}}}}}}";
/// Deletes one issue, answering with the repository it was deleted from.
pub const DELETE_ISSUE_DOCUMENT: &str =
    "mutation($input:DeleteIssueInput!){deleteIssue(input:$input){repository{nameWithOwner}}}";
/// Deletes one board item, answering with the id of the item deleted.
pub const DELETE_ITEM_DOCUMENT: &str =
    "mutation($input:DeleteProjectV2ItemInput!){deleteProjectV2Item(input:$input){deletedItemId}}";

// Operation variants bind the document, variables, accounting and response contract.
enum GraphqlOperation {
    Board { after: Option<Cursor> },
    DeleteIssue { id: IssueId },
    DeleteItem { board: BoardId, id: ItemId },
    DeleteField { id: FieldId },
}
impl GraphqlOperation {
    fn payload(&self) -> (&'static str, Value) {
        match self {
            Self::Board { after } => (
                BOARD_DOCUMENT,
                json!({"owner":BOARD_OWNER,"number":BOARD_NUMBER,"after":after.as_ref().map(|cursor| &cursor.0)}),
            ),
            Self::DeleteIssue { id, .. } => {
                (DELETE_ISSUE_DOCUMENT, json!({"input":{"issueId":id.0}}))
            }
            Self::DeleteItem { board, id } => (
                DELETE_ITEM_DOCUMENT,
                json!({"input":{"projectId":board.0,"itemId":id.0}}),
            ),
            Self::DeleteField { id } => (DELETE_FIELD_DOCUMENT, json!({"input":{"fieldId":id.0}})),
        }
    }
    fn confirm(&self, value: &Value) -> Result<(), String> {
        let confirmed = match self {
            Self::Board { .. } => true,
            Self::DeleteIssue { .. } => {
                value
                    .pointer("/data/deleteIssue/repository/nameWithOwner")
                    .and_then(Value::as_str)
                    == Some(SCRATCH_REPOSITORY)
            }
            Self::DeleteItem { id, .. } => {
                value
                    .pointer("/data/deleteProjectV2Item/deletedItemId")
                    .and_then(Value::as_str)
                    == Some(id.0.as_str())
            }
            Self::DeleteField { id } => {
                value
                    .pointer("/data/deleteProjectV2Field/projectV2Field/id")
                    .and_then(Value::as_str)
                    == Some(id.0.as_str())
            }
        };
        if confirmed {
            Ok(())
        } else {
            Err("GraphQL mutation did not confirm the requested deletion".into())
        }
    }
}

/// How an invocation ended when it did not fail.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Every list was read and every write the evidence allowed was sent, up to the cap.
    #[default]
    Completed,
    /// `affordable` refused the reading of `GET /rate_limit`, so nothing was listed or written.
    Declined,
}

#[derive(Default, Debug)]
pub struct Report {
    pub rest_reads: usize,
    pub graphql_queries: usize,
    pub writes: usize,
    pub outcome: Outcome,
    pub write_times: Vec<u64>,
}

#[derive(Clone)]
enum Delete {
    Issue(IssueId),
    Item(ItemId),
    Label(LabelName),
    Field(FieldId),
}

/// One listed artifact owned by the scratch CI run that wrote it.
#[derive(Clone)]
struct Artifact {
    owner: CiStamp,
    delete: Delete,
}

type Listed = Vec<Artifact>;

struct Janitor {
    config: Config,
    client: reqwest::Client,
    report: Report,
    board: Option<BoardId>,
    last_write: Option<Duration>,
    clock: Arc<dyn Clock>,
}

impl Janitor {
    async fn rest(&mut self, path: &str, role: CredentialRole) -> Result<Value, ReadError> {
        self.rest_page(path, role).await.map(|(body, _)| body)
    }

    async fn rest_page(
        &mut self,
        path: &str,
        role: CredentialRole,
    ) -> Result<(Value, Option<String>), ReadError> {
        if self.report.rest_reads == REST_READ_LIMIT {
            return Err(ReadError::LimitExhausted(format!(
                "REST read limit {REST_READ_LIMIT} exhausted before enumeration completed at {path}"
            )));
        }
        self.report.rest_reads += 1;
        self.fetch(path, role).await.map_err(ReadError::Failed)
    }

    async fn fetch(
        &mut self,
        path: &str,
        role: CredentialRole,
    ) -> Result<(Value, Option<String>), String> {
        let token = match (role, &self.config.ownership) {
            (CredentialRole::Actions, Ownership::CompletedRuns { actions_token }) => actions_token,
            (CredentialRole::Actions, Ownership::CancelledRun(_)) => {
                return Err(format!(
                    "read {path}: a cancelled run's cleanup reads no run"
                ));
            }
            (CredentialRole::Write, _) => &self.config.write_token,
        };
        let response = self
            .client
            .get(self.config.rest.join(path).map_err(|e| e.to_string())?)
            .bearer_auth(token.expose())
            .send()
            .await
            .map_err(|e| format!("read {path}: {e}"))?;
        if !response.status().is_success() {
            return Err(format!(
                "read {path}: unexpected HTTP {}",
                response.status()
            ));
        }
        let next = response
            .headers()
            .get(reqwest::header::LINK)
            .map(|header| {
                header
                    .to_str()
                    .map_err(|e| format!("{path} invalid Link: {e}"))
            })
            .transpose()?
            .map(|header| next_page(header, self.config.rest.as_str(), path))
            .transpose()?
            .flatten();
        let body = response
            .json()
            .await
            .map_err(|e| format!("decode {path}: {e}"))?;
        Ok((body, next))
    }

    async fn graphql(&mut self, operation: GraphqlOperation) -> Result<Value, String> {
        match &operation {
            GraphqlOperation::Board { .. } => {
                if self.report.graphql_queries == GRAPHQL_READ_LIMIT {
                    return Err(format!(
                        "GraphQL read limit {GRAPHQL_READ_LIMIT} exhausted before board enumeration completed"
                    ));
                }
                self.report.graphql_queries += 1;
            }
            GraphqlOperation::DeleteIssue { .. }
            | GraphqlOperation::DeleteItem { .. }
            | GraphqlOperation::DeleteField { .. } => self.pace().await?,
        }
        let (query, variables) = operation.payload();
        let response = self
            .client
            .post(self.config.graphql.clone())
            .bearer_auth(self.config.write_token.expose())
            .json(&json!({"query":query,"variables":variables}))
            .send()
            .await
            .map_err(|e| format!("GraphQL request: {e}"))?;
        if !response.status().is_success() {
            return Err(format!(
                "GraphQL response: unexpected HTTP {}",
                response.status()
            ));
        }
        let value: Value = response
            .json()
            .await
            .map_err(|e| format!("GraphQL decode: {e}"))?;
        if value.get("errors").is_some() || !value.get("data").is_some_and(Value::is_object) {
            return Err(format!("GraphQL did not answer: {value}"));
        }
        operation.confirm(&value)?;
        Ok(value)
    }

    async fn pace(&mut self) -> Result<(), String> {
        if self.report.writes == WRITE_LIMIT {
            return Err("content write limit reached".into());
        }
        if let Some(last) = self.last_write {
            self.clock
                .wait(WRITE_INTERVAL.saturating_sub(self.clock.monotonic().saturating_sub(last)))
                .await;
        }
        let now = self.clock.monotonic();
        self.last_write = Some(now);
        self.report.write_times.push(now.as_micros() as u64);
        self.report.writes += 1;
        Ok(())
    }

    // Numeric pagination avoids GitHub search's 1,000-result ceiling. A repeated page
    // fails closed rather than claiming that an incomplete listing authorised cleanup.
    async fn pages(&mut self, path: &str, role: CredentialRole) -> Result<Vec<Value>, String> {
        let mut result = Vec::new();
        let mut seen = BTreeSet::new();
        let mut paths = BTreeSet::new();
        let mut next = format!("{path}&per_page=100&page=1");
        let mut page = 1;
        loop {
            if !paths.insert(next.clone()) {
                return Err(format!("{path} page {page} did not advance"));
            }
            let (value, linked) = self.rest_page(&next, role).await?;
            let nodes = Some(&value)
                .and_then(Value::as_array)
                .ok_or_else(|| format!("{path} page {page}: missing array"))?;
            if !nodes.is_empty() && !seen.insert(Value::Array(nodes.to_vec()).to_string()) {
                return Err(format!("{path} page {page} did not advance"));
            }
            result.extend(nodes.iter().cloned());
            if let Some(linked) = linked {
                next = linked;
            } else if nodes.len() == 100 {
                // The endpoint's requested size is also a completion check when no Link
                // header is supplied. Empty terminal pages are valid.
                next = format!("{path}&per_page=100&page={}", page + 1);
            } else {
                return Ok(result);
            }
            page += 1;
        }
    }

    /// Lists every scratch CI artifact, whatever its age: whether it may go is decided by its
    /// run's status, read immediately before its delete batch, and by nothing else.
    fn list(&self, stamp: &str, delete: Delete, listed: &mut Listed) {
        if let Some(owner) = CiStamp::read(stamp) {
            listed.push(Artifact { owner, delete });
        }
    }

    async fn repositories(&mut self, listed: &mut Listed) -> Result<(), String> {
        let repository = SCRATCH_REPOSITORY;
        for issue in self
            .pages(
                &format!("/repos/{repository}/issues?state=all&sort=created&direction=asc"),
                CredentialRole::Write,
            )
            .await?
        {
            let title = text(&issue, "title")?;
            let id = IssueId(identifier(&issue, "node_id")?);
            if issue.get("pull_request").is_some() {
                continue;
            }
            if let Some(stamp) = titled_stamp(title, DESIGN_TITLE_PREFIX) {
                self.list(stamp, Delete::Issue(id), listed);
            }
        }
        for label in self
            .pages(
                &format!("/repos/{repository}/labels?"),
                CredentialRole::Write,
            )
            .await?
        {
            let name = text(&label, "name")?;
            if let Some(stamp) = labelled_stamp(name) {
                self.list(stamp, Delete::Label(LabelName(name.to_owned())), listed);
            }
        }
        Ok(())
    }

    async fn board(&mut self, listed: &mut Listed) -> Result<(), String> {
        let mut after = None;
        let mut cursors = BTreeSet::new();
        loop {
            let value = self
                .graphql(GraphqlOperation::Board {
                    after: after.take(),
                })
                .await?;
            let board = value
                .pointer("/data/board/projectV2")
                .ok_or("board #1 missing")?;
            self.board = Some(BoardId(identifier(board, "id")?));
            if cursors.is_empty() {
                self.fields(board, listed)?;
            }
            let items = board.get("items").ok_or("board items missing")?;
            for node in items
                .get("nodes")
                .and_then(Value::as_array)
                .ok_or("board nodes missing")?
            {
                let item_id = ItemId(identifier(node, "id")?);
                // `null` is content this token cannot see, which is nobody's residue to read;
                // an absent field or discriminator is a page that did not answer the query.
                let content = node.get("content").ok_or("board item content missing")?;
                if content.is_null() || text(content, "__typename")? != "Issue" {
                    continue;
                }
                let Some(repository) = content
                    .pointer("/repository/nameWithOwner")
                    .and_then(Value::as_str)
                else {
                    return Err("board issue repository missing".into());
                };
                if repository != SCRATCH_REPOSITORY {
                    continue;
                }
                if let Some(stamp) = titled_stamp(text(content, "title")?, DESIGN_TITLE_PREFIX) {
                    self.list(stamp, Delete::Item(item_id), listed);
                }
            }
            match items
                .pointer("/pageInfo/hasNextPage")
                .and_then(Value::as_bool)
            {
                Some(false) => return Ok(()),
                Some(true) => {
                    let cursor = items
                        .pointer("/pageInfo/endCursor")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                        .ok_or("board next cursor missing")?;
                    if !cursors.insert(cursor.to_owned()) {
                        return Err("board page did not advance".into());
                    }
                    after = Some(Cursor(cursor.to_owned()));
                }
                None => return Err("board pageInfo missing".into()),
            }
        }
    }

    /// Lists the board's stamped fields. A field of a kind the document selects nothing of
    /// answers `{}`, and the lane creates none of those kinds, so it is nobody's residue.
    fn fields(&self, board: &Value, listed: &mut Listed) -> Result<(), String> {
        let fields = board.get("fields").ok_or("board fields missing")?;
        match fields
            .pointer("/pageInfo/hasNextPage")
            .and_then(Value::as_bool)
        {
            Some(false) => {}
            Some(true) => return Err("board fields span more than one page".into()),
            None => return Err("board fields pageInfo missing".into()),
        }
        for node in fields
            .get("nodes")
            .and_then(Value::as_array)
            .ok_or("board field nodes missing")?
        {
            if node.as_object().is_some_and(serde_json::Map::is_empty) {
                continue;
            }
            let id = FieldId(identifier(node, "id")?);
            if let Some(stamp) = fielded_stamp(text(node, "name")?) {
                self.list(stamp, Delete::Field(id), listed);
            }
        }
        Ok(())
    }

    async fn delete(&mut self, delete: &Delete) -> Result<(), String> {
        match delete {
            Delete::Issue(id) => {
                self.graphql(GraphqlOperation::DeleteIssue { id: id.clone() })
                    .await?;
            }
            Delete::Item(id) => {
                self.graphql(GraphqlOperation::DeleteItem {
                    board: self.board.clone().ok_or("board ID missing")?,
                    id: id.clone(),
                })
                .await?;
            }
            Delete::Field(id) => {
                self.graphql(GraphqlOperation::DeleteField { id: id.clone() })
                    .await?;
            }
            Delete::Label(name) => {
                self.pace().await?;
                let path = format!("/repos/{}/labels/{}", SCRATCH_REPOSITORY, name.0);
                let response = self
                    .client
                    .delete(self.config.rest.join(&path).map_err(|e| e.to_string())?)
                    .bearer_auth(self.config.write_token.expose())
                    .send()
                    .await
                    .map_err(|e| format!("delete label: {e}"))?
                    .error_for_status()
                    .map_err(|e| format!("delete label: {e}"))?;
                if response.status() != reqwest::StatusCode::NO_CONTENT {
                    return Err(format!(
                        "delete label: expected HTTP 204, received {}",
                        response.status()
                    ));
                }
            }
        }
        Ok(())
    }

    /// Whether `run` reads `completed`, in a read made now. Anything undecodable or failed
    /// ends the invocation: no further writes.
    async fn owner_completed(&mut self, run: u64) -> Result<bool, String> {
        // No write intervenes between this fresh read and the batch it permits.
        let status = self.rest(&run_path(run), CredentialRole::Actions).await;
        match status.as_ref().map(RunStatus::read) {
            Ok(Some(RunStatus::Completed)) => Ok(true),
            Ok(Some(_)) => Ok(false),
            Err(ReadError::LimitExhausted(error)) => Err(error.clone()),
            Err(ReadError::Failed(error)) => Err(format!(
                "ownership read for run {run} failed: {error}; no further writes"
            )),
            Ok(None) => {
                let sent = status
                    .as_ref()
                    .ok()
                    .and_then(|value| value.get("status"))
                    .map_or_else(|| "no status".to_owned(), Value::to_string);
                Err(format!(
                    "ownership read for run {run} returned an undecodable status ({sent}); no further writes"
                ))
            }
        }
    }

    async fn scratch_pass(&mut self, artifacts: Vec<Artifact>) -> Result<(), String> {
        let mut groups: BTreeMap<u64, Vec<Artifact>> = BTreeMap::new();
        for artifact in artifacts {
            groups
                .entry(artifact.owner.run().run_id())
                .or_default()
                .push(artifact);
        }
        for (run, artifacts) in groups {
            for batch in artifacts.chunks(BATCH_SIZE) {
                if self.report.writes == WRITE_LIMIT {
                    return Ok(());
                }
                // A cancelled run's own list holds its own stamps alone (see `run_with_clock`),
                // and the run that wrote them is the one asking, so there is nothing to read.
                if let Ownership::CompletedRuns { .. } = self.config.ownership
                    && !self.owner_completed(run).await?
                {
                    // A run still going, a re-run of it included, owns every attempt's work.
                    break;
                }
                for artifact in batch {
                    if self.report.writes == WRITE_LIMIT {
                        return Ok(());
                    }
                    self.delete(&artifact.delete).await?;
                }
            }
        }
        Ok(())
    }
}

// Follow only this same origin and list endpoint. A malformed or redirected page
// fails closed rather than sending either credential to an unrelated host or repository.
fn next_page(header: &str, origin: &str, path: &str) -> Result<Option<String>, String> {
    let mut next = None;
    for link in header.split(',') {
        let (target, parameters) = link
            .trim()
            .split_once(';')
            .ok_or("malformed pagination Link")?;
        if !parameters.split(';').any(|p| p.trim() == "rel=\"next\"") {
            continue;
        }
        let url = reqwest::Url::parse(
            target
                .trim()
                .strip_prefix('<')
                .and_then(|s| s.strip_suffix('>'))
                .ok_or("malformed pagination target")?,
        )
        .map_err(|e| e.to_string())?;
        let base = reqwest::Url::parse(origin).map_err(|e| e.to_string())?;
        if url.origin() != base.origin()
            || url.path() != path.split('?').next().unwrap_or(path)
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return Err("pagination Link leaves nominated endpoint".into());
        }
        let current = base.join(path).map_err(|e| e.to_string())?;
        let parameters = |url: &reqwest::Url| -> Result<BTreeMap<String, String>, String> {
            let mut result = BTreeMap::new();
            for (key, value) in url.query_pairs() {
                if result.insert(key.to_string(), value.to_string()).is_some() {
                    return Err("duplicate pagination query parameter".into());
                }
            }
            Ok(result)
        };
        let mut before = parameters(&current)?;
        let mut after = parameters(&url)?;
        let page = |value: Option<String>| -> Result<u64, String> {
            let value = value.ok_or("pagination page missing")?;
            if value.starts_with('0') || !value.bytes().all(|b| b.is_ascii_digit()) {
                return Err("invalid pagination page".into());
            }
            value.parse().map_err(|_| "invalid pagination page".into())
        };
        let previous = page(before.remove("page"))?;
        let following = page(after.remove("page"))?;
        if previous.checked_add(1) != Some(following) || before != after {
            return Err("pagination Link changed enumeration filters or did not advance".into());
        }
        if next.is_some() {
            return Err("multiple next pagination Links".into());
        }
        next = Some(format!(
            "{}{}",
            url.path(),
            url.query().map(|q| format!("?{q}")).unwrap_or_default()
        ));
    }
    Ok(next)
}

fn text<'a>(value: &'a Value, field: &str) -> Result<&'a str, String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing or undecodable {field}"))
}

/// Enumerate every nominated list before any deletion; an incomplete list fails closed.
/// The workflow and offline subprocess journeys both enter here.
pub async fn run(config: Config) -> Result<Report, String> {
    run_with_clock(config, Arc::new(RealClock(Instant::now()))).await
}

/// Run the same cleanup with a controlled monotonic clock for offline journeys.
pub async fn run_with_clock(config: Config, clock: Arc<dyn Clock>) -> Result<Report, String> {
    let mut janitor = Janitor {
        config,
        client: reqwest::Client::builder()
            .user_agent("onetaskgraph-live-janitor")
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| e.to_string())?,
        report: Report::default(),
        board: None,
        last_write: None,
        clock,
    };
    let rate = janitor.rest("/rate_limit", CredentialRole::Write).await?;
    let mut demands = Vec::new();
    for (resource, cost) in [
        ("core", REST_READ_LIMIT as u64 + WRITE_LIMIT as u64),
        ("graphql", GRAPHQL_READ_LIMIT as u64 + WRITE_LIMIT as u64),
    ] {
        let resource = rate
            .pointer(&format!("/resources/{resource}"))
            .ok_or("rate_limit resource missing")?;
        let number = |field| {
            resource
                .get(field)
                .and_then(Value::as_u64)
                .ok_or_else(|| format!("rate_limit {field} missing"))
        };
        let allowance = Allowance::read(number("limit")?, number("remaining")?, number("reset")?)
            .ok_or("rate_limit allowance invalid")?;
        demands.push(Demand::read(
            Metered::new("live-janitor-requests-per-run", "requests"),
            cost,
            allowance,
        ));
    }
    if let Err(refusal) = affordable(&demands) {
        eprintln!("janitor declined: {refusal:?}; no writes sent");
        janitor.report.outcome = Outcome::Declined;
        return Ok(janitor.report);
    }
    let mut listed = Listed::default();
    janitor.board(&mut listed).await?;
    janitor.repositories(&mut listed).await?;
    // A cancelled run removes exactly its own attempt's stamps: another run's, and another
    // attempt of its own, are the scheduled pass's once their run reads `completed`.
    if let Ownership::CancelledRun(own) = janitor.config.ownership {
        listed.retain(|artifact| artifact.owner.run() == own);
    }
    // Remove board items before their issues; issue deletion also removes its board item.
    listed.sort_by_key(|a| (!matches!(a.delete, Delete::Item(_)), a.owner.to_string()));
    janitor.scratch_pass(listed).await?;
    Ok(janitor.report)
}
