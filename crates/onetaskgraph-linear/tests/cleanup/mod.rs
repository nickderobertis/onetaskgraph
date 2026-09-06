//! The cleanup both of this lane's targets share, and the one contract they derive it from.
//!
//! `tests/live.rs` is the journey against Linear itself; `tests/sweep_gate.rs` drives this
//! very code against a loopback stand-in, with no credential and no third party, so that
//! what a run removes and what it must leave is proven whether or not a key is present.
//! Both are ordinary tests in this crate's ordinary `test` target.
//!
//! The seam between them is [`against`]: everything below sends to whatever endpoint that
//! names, and it names Linear unless a drive says otherwise.

use std::future::Future;
use std::sync::OnceLock;
use std::time::Duration;

use onetaskgraph_live::artifact::{Run, Stamp, Sweep};
use serde_json::{Value, json};

/// Where every request below goes when nothing has said otherwise.
const LINEAR: &str = "https://api.linear.app/graphql";

static ENDPOINT: OnceLock<String> = OnceLock::new();

/// Point everything below at `url` instead of Linear, for a drive that has no credential.
///
/// Once per process, because a journey is pointed at one API per test binary — the same
/// reason `tests/live.rs` never calls it at all.
pub fn against(url: &str) {
    ENDPOINT
        .set(url.to_owned())
        .expect("one endpoint per process");
}

fn endpoint() -> &'static str {
    ENDPOINT.get().map_or(LINEAR, String::as_str)
}

pub async fn linear(key: &str, query: &str, variables: Value, what: &str) -> Result<Value, String> {
    let body: Value = reqwest::Client::new()
        .post(endpoint())
        .header("Authorization", key)
        .json(&json!({"query":query,"variables":variables}))
        .send()
        .await
        .map_err(|error| format!("{what} could not reach Linear: {error}"))?
        .error_for_status()
        .map_err(|error| format!("{what} failed: {error}"))?
        .json()
        .await
        .map_err(|error| format!("{what} returned invalid JSON: {error}"))?;
    if let Some(errors) = body
        .get("errors")
        .filter(|errors| !errors.as_array().is_some_and(Vec::is_empty))
    {
        return Err(format!("{what} was rejected by Linear: {errors}"));
    }
    body.get("data")
        .cloned()
        .ok_or_else(|| format!("{what} returned no data: {body}"))
}

/// The session this lane opens against Linear, by the name its seat and its refusals use.
pub const SESSION_NAME: &str = "Linear";

/// The prefix of every issue, project and document this lane writes.
///
/// The rest of a name is an [`onetaskgraph_live::artifact::Stamp`] — the machine and process
/// that wrote it, and a microsecond timestamp — which is what makes one run's artifacts
/// unique, what makes them recognisable to that run's own cleanup, and what a later run
/// looks the writing run up by before deciding whether anything still owns them. The grammar
/// of that half is not spelled here: it is one contract shared with the GitHub Projects
/// lane, because both have to answer the same question about an artifact with the same
/// answer.
pub const ARTIFACT_PREFIX: &str = "onetaskgraph live cleanup ";

/// The same for a label, whose name Linear shows in its own filter menus.
pub const LABEL_PREFIX: &str = "otg-live-";

pub fn artifact_title(run: Run, stamp_micros: u64) -> String {
    format!("{ARTIFACT_PREFIX}{}", Stamp::new(run, stamp_micros))
}

pub fn artifact_label(run: Run, stamp_micros: u64) -> String {
    format!("{LABEL_PREFIX}{}", Stamp::new(run, stamp_micros))
}

/// Whether a name under `prefix` is one *this* run wrote.
pub fn is_this_runs(run: Run, prefix: &str, name: &str) -> bool {
    Stamp::read(name.strip_prefix(prefix).unwrap_or("")).is_some_and(|stamp| stamp.run() == run)
}

pub const TEAM_STATES: &str = "query($key:String!){teams(filter:{key:{eqIgnoreCase:$key}}){nodes{id key states(first:100){nodes{id name type}}}}}";
pub const PROJECT_STATUSES: &str = "query{projectStatuses{nodes{id name type}}}";
pub const LABEL_CREATE: &str = "mutation($input:IssueLabelCreateInput!){issueLabelCreate(input:$input){success issueLabel{id name}}}";
pub const LABEL_DELETE: &str = "mutation($id:String!){issueLabelDelete(id:$id){success}}";
pub const LABELS_PAGE: &str = "query($first:Int!,$after:String){issueLabels(first:$first,after:$after){nodes{id name} pageInfo{hasNextPage endCursor}}}";
pub const ISSUES_BY_TITLE: &str = "query($first:Int!,$after:String,$prefix:String!){issues(first:$first,after:$after,filter:{title:{startsWith:$prefix}}){nodes{id title} pageInfo{hasNextPage endCursor}}}";
pub const PROJECTS_BY_NAME: &str = "query($first:Int!,$after:String,$prefix:String!){projects(first:$first,after:$after,filter:{name:{startsWith:$prefix}}){nodes{id name} pageInfo{hasNextPage endCursor}}}";
pub const ISSUE_PAGE_PROBE: &str = "query($first:Int!){issues(first:$first){nodes{id}}}";
pub const DOCUMENTS_BY_TITLE: &str = "query($first:Int!,$after:String,$prefix:String!){documents(first:$first,after:$after,filter:{title:{startsWith:$prefix}}){nodes{id title} pageInfo{hasNextPage endCursor}}}";

/// Every `(id, name)` a paged Linear connection reports.
async fn walk(
    key: &str,
    query: &str,
    connection: &str,
    name_field: &str,
    prefix: Option<&str>,
    what: &str,
) -> Result<Vec<(String, String)>, String> {
    let mut after = Value::Null;
    let mut found = Vec::new();
    for _ in 0..50 {
        let mut variables = json!({"first":onetaskgraph_linear::MAX_PAGE_SIZE,"after":after});
        if let Some(prefix) = prefix {
            variables["prefix"] = Value::String(prefix.to_owned());
        }
        let data = linear(key, query, variables, what).await?;
        let page = data
            .get(connection)
            .ok_or_else(|| format!("{what} returned no {connection} connection"))?;
        for node in page
            .get("nodes")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("{what} returned {connection}.nodes that is not an array"))?
        {
            let (Some(id), Some(name)) = (
                node.get("id").and_then(Value::as_str),
                node.get(name_field).and_then(Value::as_str),
            ) else {
                return Err(format!(
                    "{what} returned a {connection} node with no id or name"
                ));
            };
            found.push((id.to_owned(), name.to_owned()));
        }
        if page
            .pointer("/pageInfo/hasNextPage")
            .and_then(Value::as_bool)
            != Some(true)
        {
            return Ok(found);
        }
        let next = page
            .pointer("/pageInfo/endCursor")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{what} has no advancing cursor"))?;
        if after.as_str() == Some(next) {
            return Err(format!("{what} cursor did not advance"));
        }
        after = Value::String(next.to_owned());
    }
    Err(format!("{what} did not terminate"))
}

/// One kind of thing this lane writes: how it is listed, and how one of them is deleted.
///
/// A table rather than four near-identical blocks, because what has to be true of all four
/// is the same — find what matches, delete it, and let Linear's own listing decide whether
/// the cleanup worked.
pub struct Residue {
    pub listing: &'static str,
    pub connection: &'static str,
    pub name_field: &'static str,
    /// The prefix every name of this kind carries.
    pub prefix: &'static str,
    /// Whether `listing` narrows by that prefix itself, which the label connection does not.
    pub narrowed: bool,
    pub delete: &'static str,
    /// Where in the mutation's answer Linear confirms it.
    pub confirm: &'static str,
    pub kind: &'static str,
}

/// The four of them, as one place rather than four blocks that have to agree.
pub static RESIDUE: [Residue; 4] = [
    Residue {
        listing: ISSUES_BY_TITLE,
        connection: "issues",
        name_field: "title",
        prefix: ARTIFACT_PREFIX,
        narrowed: true,
        delete: onetaskgraph_linear::graphql::ISSUE_DELETE,
        confirm: "/issueDelete/success",
        kind: "issue",
    },
    Residue {
        listing: PROJECTS_BY_NAME,
        connection: "projects",
        name_field: "name",
        prefix: ARTIFACT_PREFIX,
        narrowed: true,
        delete: onetaskgraph_linear::graphql::PROJECT_DELETE,
        confirm: "/projectDelete/success",
        kind: "project",
    },
    Residue {
        listing: DOCUMENTS_BY_TITLE,
        connection: "documents",
        name_field: "title",
        prefix: ARTIFACT_PREFIX,
        narrowed: true,
        delete: onetaskgraph_linear::graphql::DOCUMENT_DELETE,
        confirm: "/documentDelete/success",
        kind: "document",
    },
    Residue {
        listing: LABELS_PAGE,
        connection: "issueLabels",
        name_field: "name",
        prefix: LABEL_PREFIX,
        narrowed: false,
        delete: LABEL_DELETE,
        confirm: "/issueLabelDelete/success",
        kind: "label",
    },
];

/// Everything of every kind whose name `matches`, as `(kind, id, name)`.
async fn find_artifacts(
    key: &str,
    matches: &dyn Fn(&str, &str) -> bool,
) -> Result<Vec<(&'static Residue, String, String)>, String> {
    let mut found = Vec::new();
    for residue in &RESIDUE {
        let listed = walk(
            key,
            residue.listing,
            residue.connection,
            residue.name_field,
            residue.narrowed.then_some(residue.prefix),
            &format!("live {} residue lookup", residue.kind),
        )
        .await?;
        found.extend(
            listed
                .into_iter()
                .filter(|(_, name)| matches(residue.prefix, name))
                .map(|(id, name)| (residue, id, name)),
        );
    }
    Ok(found)
}

/// Deletes every issue, project, document and label whose name `matches`.
///
/// Called twice, over two different `matches`: once after the journey over this run's own
/// naming, which is what makes the lane leave nothing behind whether it passed or failed;
/// and once more over the orphans of runs that never got that far. See [`sweep_orphans`].
///
/// **A delete that fails does not fail the cleanup on its own, and the listing is why it
/// does not have to.** An entity this listing named can be gone by the time the delete
/// lands — another run swept it, somebody removed it by hand — and Linear refuses a delete
/// of what is no longer there. That refusal is the outcome the delete was asking for, and
/// treating it as a failure once killed a whole journey over an issue that had already
/// gone. So a refusal is remembered and the next round asks Linear what is actually left:
/// nothing matching means done, however many deletes were refused getting there. Nothing
/// here has to know how Linear spells "already gone", which is what would otherwise have to
/// be guessed at and would then go stale the day that spelling changed.
pub async fn remove_artifacts(
    key: &str,
    matches: &dyn Fn(&str, &str) -> bool,
) -> Result<(), String> {
    let mut refused = Vec::new();
    for _ in 0..3 {
        let found = find_artifacts(key, matches).await?;
        if found.is_empty() {
            return Ok(());
        }
        refused.clear();
        for (residue, id, name) in found {
            match linear(
                key,
                residue.delete,
                json!({ "id": id }),
                &format!("live {} cleanup", residue.kind),
            )
            .await
            {
                Ok(data) if data.pointer(residue.confirm) == Some(&Value::Bool(true)) => {}
                Ok(_) => refused.push(format!(
                    "Linear did not confirm deleting {} {name:?}",
                    residue.kind
                )),
                Err(problem) => refused.push(problem),
            }
        }
        if refused.is_empty() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    Err(format!(
        "live cleanup left {}; Linear refused: {}",
        find_artifacts(key, matches)
            .await?
            .into_iter()
            .map(|(residue, _, name)| format!("{} {name:?}", residue.kind))
            .collect::<Vec<_>>()
            .join(", "),
        refused.join("; ")
    ))
}

/// What an *interrupted* earlier run left behind, and nothing a live run owns.
///
/// A process killed between its writes and its cleanup never reaches [`run_then_cleanup`],
/// so its issues, projects, documents and label stay in the scratch team. Recovering them is
/// what this is for, and the whole difficulty is telling them from the artifacts of a run
/// that is still going.
///
/// `sweep` is that decision and it is not this lane's: `onetaskgraph_live::artifact` holds
/// it, both hosted lanes derive from it, and what authorises a removal is positive evidence
/// that no live run owns the artifact — the registration lock of the run that wrote it,
/// released by the kernel when that process ended. An artifact of a run that is still going
/// is never taken, whatever its age, and neither is one whose machine this sweep cannot ask.
///
/// **It runs after the journey rather than before it, and that ordering is the point.** A
/// sweep at startup was what deleted a concurrent session's in-flight issues, and there is
/// nothing a start can do that an end cannot: the cleanup already runs whether the journey
/// passed or failed, and an orphan will still be an orphan then.
pub async fn sweep_orphans(key: &str, sweep: &Sweep) -> Result<(), String> {
    remove_artifacts(key, &|prefix, name| sweep.names_an_orphan(prefix, name)).await
}

pub async fn run_then_cleanup<J, JF, C, CF>(journey: J, cleanup: C) -> Result<(), String>
where
    J: FnOnce() -> JF,
    JF: Future<Output = Result<(), String>>,
    C: FnOnce() -> CF,
    CF: Future<Output = Result<(), String>>,
{
    let journey_result = journey().await;
    let cleanup_result = cleanup().await;
    match (journey_result, cleanup_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(journey), Ok(())) => Err(journey),
        (Ok(()), Err(cleanup)) => Err(format!("live cleanup failed: {cleanup}")),
        (Err(journey), Err(cleanup)) => Err(format!(
            "{journey}; additionally, live cleanup failed: {cleanup}"
        )),
    }
}
