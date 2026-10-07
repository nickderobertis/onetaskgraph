//! Scheduled residue cleanup, separate from the credentialed test lane.
//!
//! Scratch CI artifacts require their owning Actions run to read `completed` immediately
//! before their delete batch. Machine stamps stay the writing machine's lock sweep's.
//! The legacy pass requires fully paginated non-completed `ci.yml` runs all created strictly
//! after the artifact's time plus the skew margin. Both passes wait a day; that margin is
//! never ownership evidence. Legacy cleanup has no evidence about development machines:
//! a machine run that wrote before cutover and lives a day may lose its artifacts.
//! Linear keeps its machine stamps and lock sweep everywhere; this janitor never reads it.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::{Duration, Instant};

use onetaskgraph_github_live::{
    BOARD_NUMBER, BOARD_OWNER, CORE_REPOSITORY, SCRATCH_REPOSITORY, labelled_stamp, titled_stamp,
};
use onetaskgraph_github_projects::DESIGN_TITLE_PREFIX;
use onetaskgraph_live::artifact::{CiStamp, Stamp};
use onetaskgraph_live::{Allowance, Credential, Demand, Metered, affordable};
use serde_json::{Value, json};

/// A generous delay, never the authorisation for a deletion.
pub const WAITING_PERIOD: Duration = Duration::from_secs(24 * 60 * 60);
/// Clock disagreement permitted when comparing legacy stamps with Actions creation times.
pub const CLOCK_SKEW_MARGIN: Duration = Duration::from_secs(10 * 60);
/// The commit introducing the CI nomination has this exact author second.
pub const CUTOVER_MICROS: u64 = 1_791_387_489_000_000;
/// Shared runtime limits, across both passes; these are not performance budgets.
pub const WRITE_LIMIT: usize = 150;
pub const REST_READ_LIMIT: usize = 250;
pub const GRAPHQL_READ_LIMIT: usize = 50;
/// One content request per second at most, including the boundary between passes.
pub const WRITE_INTERVAL: Duration = Duration::from_secs(1);
const BATCH_SIZE: usize = 25;

/// The monotonic clock used by pacing. Ownership age uses the invocation's fixed wall clock.
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

/// Endpoints and credentials validated together before an invocation can reach GitHub.
/// Construction admits the production host or an explicitly selected loopback origin.
pub struct Config {
    rest: reqwest::Url,
    graphql: reqwest::Url,
    write_token: Credential,
    actions_token: Credential,
    now_micros: u64,
}

impl Config {
    pub fn github(write_token: &str, actions_token: &str, now_micros: u64) -> Result<Self, String> {
        Self::at(
            reqwest::Url::parse("https://api.github.com/").map_err(|e| e.to_string())?,
            write_token,
            actions_token,
            now_micros,
        )
    }

    pub fn loopback(
        origin: &str,
        write_token: &str,
        actions_token: &str,
        now_micros: u64,
    ) -> Result<Self, String> {
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
        Self::at(origin, write_token, actions_token, now_micros)
    }

    fn at(
        rest: reqwest::Url,
        write_token: &str,
        actions_token: &str,
        now_micros: u64,
    ) -> Result<Self, String> {
        Ok(Self {
            graphql: rest.join("graphql").map_err(|e| e.to_string())?,
            rest,
            write_token: Credential::new(write_token).ok_or("GH_PROJECTS_TOKEN is required")?,
            actions_token: Credential::new(actions_token).ok_or("GITHUB_TOKEN is required")?,
            now_micros,
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

#[derive(Clone, Copy)]
enum Target {
    Scratch,
    Legacy,
}
impl Target {
    fn repository(self) -> &'static str {
        match self {
            Self::Scratch => SCRATCH_REPOSITORY,
            Self::Legacy => CORE_REPOSITORY,
        }
    }
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
struct BoardId(String);
#[derive(Clone)]
struct LabelName(String);
/// A board page's `endCursor`: present and non-empty, or there is no next page to ask for.
struct Cursor(String);

// llmlint: ignore-block[contracts_have_one_source_or_a_drift_gate] GitHub publishes no machine-readable vocabulary of a run's `status` (the OpenAPI schema types it as a bare string), so there is no artifact to derive this from. The six words are pinned with their provenance below, the offline journeys derive theirs from this one declaration, and the drift gate is the janitor itself: a word GitHub adds is undecodable, fails the scheduled run red naming that word, and deletes nothing.
/// A workflow run's `status`, as GitHub documents it. Anything else is undecodable, and the
/// janitor's failure names it, which is how a word GitHub adds is noticed.
///
/// The six statuses of the `status` values GitHub lists for "List workflow runs for a
/// workflow" (<https://docs.github.com/en/rest/actions/workflow-runs?apiVersion=2022-11-28>,
/// read 2026-10-07); the rest of that list — `action_required`, `cancelled`, `failure`,
/// `neutral`, `skipped`, `stale`, `success`, `timed_out` — are conclusions, which a run's
/// `status` never holds. This is the one declaration: the offline journeys derive theirs
/// from it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RunStatus {
    Completed,
    Queued,
    InProgress,
    Waiting,
    Requested,
    Pending,
}
impl RunStatus {
    /// Every status a run that has not finished can have: the legacy pass lists each.
    const ACTIVE: [Self; 5] = [
        Self::Queued,
        Self::InProgress,
        Self::Waiting,
        Self::Requested,
        Self::Pending,
    ];

    /// GitHub's spelling, in a run's `status` and in a listing's `status` filter alike.
    fn spelling(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Queued => "queued",
            Self::InProgress => "in_progress",
            Self::Waiting => "waiting",
            Self::Requested => "requested",
            Self::Pending => "pending",
        }
    }

    fn read(value: &Value) -> Option<Self> {
        let spelled = value.get("status")?.as_str()?;
        std::iter::once(Self::Completed)
            .chain(Self::ACTIVE)
            .find(|status| status.spelling() == spelled)
    }
}
// llmlint: ignore-end[contracts_have_one_source_or_a_drift_gate]

/// One page of the nominated board's items, with what recognising residue needs of each.
///
/// This and the two mutations below are validated against the GitHub Projects plugin's
/// pinned schema by `tests/schema.rs`, so a field GitHub does not have fails offline.
pub const BOARD_DOCUMENT: &str = "query($owner:String!,$number:Int!,$after:String){board:repositoryOwner(login:$owner){... on ProjectV2Owner{projectV2(number:$number){id items(first:100,after:$after){nodes{id content{__typename ... on Issue{title repository{nameWithOwner}}}} pageInfo{hasNextPage endCursor}}}}}}";
/// Deletes one issue, answering with the repository it was deleted from.
pub const DELETE_ISSUE_DOCUMENT: &str =
    "mutation($input:DeleteIssueInput!){deleteIssue(input:$input){repository{nameWithOwner}}}";
/// Deletes one board item, answering with the id of the item deleted.
pub const DELETE_ITEM_DOCUMENT: &str =
    "mutation($input:DeleteProjectV2ItemInput!){deleteProjectV2Item(input:$input){deletedItemId}}";

// Operation variants bind the document, variables, accounting and response contract.
enum GraphqlOperation {
    Board { after: Option<Cursor> },
    DeleteIssue { id: IssueId, target: Target },
    DeleteItem { board: BoardId, id: ItemId },
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
        }
    }
    fn confirm(&self, value: &Value) -> Result<(), String> {
        let confirmed = match self {
            Self::Board { .. } => true,
            Self::DeleteIssue { target, .. } => {
                value
                    .pointer("/data/deleteIssue/repository/nameWithOwner")
                    .and_then(Value::as_str)
                    == Some(target.repository())
            }
            Self::DeleteItem { id, .. } => {
                value
                    .pointer("/data/deleteProjectV2Item/deletedItemId")
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
}

/// One listed artifact, owned the way its pass requires: a scratch artifact by the CI run
/// that wrote it, a legacy one by a machine stamp.
#[derive(Clone)]
struct Artifact<O> {
    owner: O,
    delete: Delete,
}

/// Everything enumeration found eligible, already split by the pass that may delete it.
#[derive(Default)]
struct Listed {
    scratch: Vec<Artifact<CiStamp>>,
    legacy: Vec<Artifact<Stamp>>,
}

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
        let token = match role {
            CredentialRole::Actions => &self.config.actions_token,
            CredentialRole::Write => &self.config.write_token,
        };
        let response = self
            .client
            .get(self.config.rest.join(path).map_err(|e| e.to_string())?)
            .bearer_auth(token.expose())
            .send()
            .await
            .map_err(|e| format!("read {path}: {e}"))?
            .error_for_status()
            .map_err(|e| format!("read {path}: {e}"))?;
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
            GraphqlOperation::DeleteIssue { .. } | GraphqlOperation::DeleteItem { .. } => {
                self.pace().await?
            }
        }
        let (query, variables) = operation.payload();
        let value: Value = self
            .client
            .post(self.config.graphql.clone())
            .bearer_auth(self.config.write_token.expose())
            .json(&json!({"query":query,"variables":variables}))
            .send()
            .await
            .map_err(|e| format!("GraphQL request: {e}"))?
            .error_for_status()
            .map_err(|e| format!("GraphQL response: {e}"))?
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
    async fn pages(
        &mut self,
        path: &str,
        field: Option<&str>,
        role: CredentialRole,
    ) -> Result<Vec<Value>, String> {
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
            let nodes = field
                .map_or(Some(&value), |field| value.get(field))
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
                // A wrapped listing (the Actions runs) states its own size, and must.
                if field.is_some()
                    && value.get("total_count").and_then(Value::as_u64) != Some(result.len() as u64)
                {
                    return Err(format!("{path} incomplete enumeration at page {page}"));
                }
                return Ok(result);
            }
            page += 1;
        }
    }

    /// Lists `stamp`'s artifact for its target's pass when it is that pass's form and age.
    fn list(&self, stamp: &str, delete: Delete, target: Target, listed: &mut Listed) {
        let waited = |micros: u64| {
            self.config
                .now_micros
                .checked_sub(micros)
                .is_some_and(|age| age >= WAITING_PERIOD.as_micros() as u64)
        };
        match target {
            Target::Scratch => {
                if let Some(owner) = CiStamp::read(stamp).filter(|o| waited(o.micros())) {
                    listed.scratch.push(Artifact { owner, delete });
                }
            }
            Target::Legacy => {
                if let Some(owner) =
                    Stamp::read(stamp).filter(|o| waited(o.micros()) && o.micros() < CUTOVER_MICROS)
                {
                    listed.legacy.push(Artifact { owner, delete });
                }
            }
        }
    }

    async fn repositories(&mut self, target: Target, listed: &mut Listed) -> Result<(), String> {
        let repository = target.repository();
        for issue in self
            .pages(
                &format!("/repos/{repository}/issues?state=all&sort=created&direction=asc"),
                None,
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
                self.list(stamp, Delete::Issue(id), target, listed);
            }
        }
        for label in self
            .pages(
                &format!("/repos/{repository}/labels?"),
                None,
                CredentialRole::Write,
            )
            .await?
        {
            let name = text(&label, "name")?;
            if let Some(stamp) = labelled_stamp(name) {
                self.list(
                    stamp,
                    Delete::Label(LabelName(name.to_owned())),
                    target,
                    listed,
                );
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
                let target = match repository {
                    CORE_REPOSITORY => Target::Legacy,
                    SCRATCH_REPOSITORY => Target::Scratch,
                    _ => continue,
                };
                if let Some(stamp) = titled_stamp(text(content, "title")?, DESIGN_TITLE_PREFIX) {
                    self.list(stamp, Delete::Item(item_id), target, listed);
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

    async fn legacy_owners(&mut self) -> Result<Vec<u64>, String> {
        let mut times = Vec::new();
        for status in RunStatus::ACTIVE.map(RunStatus::spelling) {
            for run in self
                .pages(
                    &format!(
                        "/repos/{CORE_REPOSITORY}/actions/workflows/ci.yml/runs?status={status}"
                    ),
                    Some("workflow_runs"),
                    CredentialRole::Actions,
                )
                .await?
            {
                let created = chrono::DateTime::parse_from_rfc3339(text(&run, "created_at")?)
                    .map_err(|e| format!("Actions created_at: {e}"))?;
                times.push(
                    u64::try_from(created.timestamp_micros())
                        .map_err(|_| "Actions time before epoch")?,
                );
            }
        }
        Ok(times)
    }

    async fn delete(&mut self, delete: &Delete, target: Target) -> Result<(), String> {
        match delete {
            Delete::Issue(id) => {
                self.graphql(GraphqlOperation::DeleteIssue {
                    id: id.clone(),
                    target,
                })
                .await?;
            }
            Delete::Item(id) => {
                self.graphql(GraphqlOperation::DeleteItem {
                    board: self.board.clone().ok_or("board ID missing")?,
                    id: id.clone(),
                })
                .await?;
            }
            Delete::Label(name) => {
                self.pace().await?;
                let path = format!("/repos/{}/labels/{}", target.repository(), name.0);
                self.client
                    .delete(self.config.rest.join(&path).map_err(|e| e.to_string())?)
                    .bearer_auth(self.config.write_token.expose())
                    .send()
                    .await
                    .map_err(|e| format!("delete label: {e}"))?
                    .error_for_status()
                    .map_err(|e| format!("delete label: {e}"))?;
            }
        }
        Ok(())
    }

    async fn scratch_pass(&mut self, artifacts: Vec<Artifact<CiStamp>>) -> Result<(), String> {
        let mut groups: BTreeMap<u64, Vec<Artifact<CiStamp>>> = BTreeMap::new();
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
                // No write intervenes between this fresh read and the batch it permits.
                let status = self
                    .rest(
                        &format!("/repos/{CORE_REPOSITORY}/actions/runs/{run}"),
                        CredentialRole::Actions,
                    )
                    .await;
                match status.as_ref().map(RunStatus::read) {
                    Ok(Some(RunStatus::Completed)) => {}
                    // A run still going, a re-run of it included, owns every attempt's work.
                    Ok(Some(_)) => break,
                    Err(ReadError::LimitExhausted(error)) => return Err(error.clone()),
                    Err(ReadError::Failed(error)) => {
                        return Err(format!(
                            "ownership read for run {run} failed: {error}; no further writes"
                        ));
                    }
                    Ok(None) => {
                        let sent = status
                            .as_ref()
                            .ok()
                            .and_then(|value| value.get("status"))
                            .map_or_else(|| "no status".to_owned(), Value::to_string);
                        return Err(format!(
                            "ownership read for run {run} returned an undecodable status ({sent}); no further writes"
                        ));
                    }
                }
                for artifact in batch {
                    if self.report.writes == WRITE_LIMIT {
                        return Ok(());
                    }
                    self.delete(&artifact.delete, Target::Scratch).await?;
                }
            }
        }
        Ok(())
    }

    async fn legacy_pass(&mut self, artifacts: Vec<Artifact<Stamp>>) -> Result<(), String> {
        if self.config.now_micros < CUTOVER_MICROS.saturating_add(WAITING_PERIOD.as_micros() as u64)
        {
            return Ok(());
        }
        // Enumeration obtained the first evidence after listing. Every batch refreshes
        // that complete listing immediately before its own writes.
        for batch in artifacts.chunks(BATCH_SIZE) {
            if self.report.writes == WRITE_LIMIT {
                return Ok(());
            }
            let owners = self.legacy_owners().await?;
            for artifact in batch {
                if self.report.writes == WRITE_LIMIT {
                    return Ok(());
                }
                if owners.iter().all(|created| {
                    *created
                        > artifact
                            .owner
                            .micros()
                            .saturating_add(CLOCK_SKEW_MARGIN.as_micros() as u64)
                }) {
                    self.delete(&artifact.delete, Target::Legacy).await?;
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
    janitor.repositories(Target::Scratch, &mut listed).await?;
    janitor.repositories(Target::Legacy, &mut listed).await?;
    let Listed {
        mut scratch,
        mut legacy,
    } = listed;
    // Remove board items before their issues; issue deletion also removes its board item.
    scratch.sort_by_key(|a| (!matches!(a.delete, Delete::Item(_)), a.owner.to_string()));
    legacy.sort_by_key(|a| (!matches!(a.delete, Delete::Item(_)), a.owner.to_string()));
    if janitor.config.now_micros >= CUTOVER_MICROS.saturating_add(WAITING_PERIOD.as_micros() as u64)
    {
        janitor.legacy_owners().await?;
    }
    janitor.scratch_pass(scratch).await?;
    janitor.legacy_pass(legacy).await?;
    Ok(janitor.report)
}
