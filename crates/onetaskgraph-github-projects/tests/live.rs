//! Structural and residue-free write verification against GitHub's real Projects v2 API.
//!
//! The board comes from `GH_PROJECTS_OWNER` and `GH_PROJECTS_NUMBER`, and the repository this
//! source creates its issues in comes from `GH_PROJECTS_REPOSITORY`, or the lane skips:
//! requiring both to be named is what keeps a credentialed write lane off a board and a
//! repository nobody nominated. Clearing residue before each run is a separate thing —
//! self-healing after an interrupted run.
//!
//! The artifact this lane writes is a real issue, because a project is an issue and a task is
//! its sub-issue, so leaving no residue means deleting the board item **and** the issue. The
//! credential therefore needs to be able to delete an issue in `GH_PROJECTS_REPOSITORY`.

use std::{collections::BTreeMap, env, future::Future};

use onetaskgraph_plugin_api::{
    Capabilities, DependencyEdge, DependencyEndpoint, DependencyKind, DependencySupport, Direction,
    ItemKind, ItemWrite, LabelFilter, NativeId, PageRequest, Project, ProjectFilter, ProjectQuery,
    SecretResolver, SourceName, SourcePlugin, Status, StatusCategory, Support, Task, TaskQuery,
    TaskSource, TextFields, TextQuery,
};
use secrecy::SecretString;
use serde_json::{Value, json};

struct LiveSecret(SecretString);

impl SecretResolver for LiveSecret {
    fn get(&self, variable: &str) -> Option<SecretString> {
        (variable == "GH_PROJECTS_TOKEN").then(|| self.0.clone())
    }
}

async fn graphql(token: &str, query: &str, query_name: &str) -> Result<Value, String> {
    graphql_variables(token, query, query_name, json!({})).await
}

async fn graphql_variables(
    token: &str,
    query: &str,
    query_name: &str,
    variables: Value,
) -> Result<Value, String> {
    let response: Value = reqwest::Client::new()
        .post("https://api.github.com/graphql")
        .header("user-agent", "onetaskgraph-live-test")
        .bearer_auth(token)
        .json(&json!({"query":query,"variables":variables}))
        .send()
        .await
        .map_err(|error| format!("{query_name} query could not reach GitHub: {error}"))?
        .error_for_status()
        .map_err(|error| format!("{query_name} query failed: {error}"))?
        .json()
        .await
        .map_err(|error| format!("{query_name} query returned invalid JSON: {error}"))?;
    if let Some(errors) = response.get("errors") {
        return Err(format!(
            "{query_name} query was rejected by GitHub: {errors}"
        ));
    }
    Ok(response)
}

/// The one board text field this source keeps a copy's origin in.
const ORIGIN_FIELD: &str = "onetaskgraph.origin";

async fn writable_fields(token: &str, project_id: &str) -> Result<Vec<Value>, String> {
    let mut after = Value::Null;
    let mut fields = Vec::new();
    loop {
        let response = graphql_variables(
            token,
            "query($id:ID!,$after:String){node(id:$id){... on ProjectV2{fields(first:100,after:$after){nodes{... on ProjectV2SingleSelectField{id name options{name}} ... on ProjectV2Field{id name}}pageInfo{hasNextPage endCursor}}}}}",
            "writable field discovery",
            json!({"id":project_id,"after":after}),
        )
        .await?;
        let connection = response
            .pointer("/data/node/fields")
            .ok_or_else(|| "writable field discovery returned no fields connection".to_owned())?;
        fields.extend(
            connection
                .get("nodes")
                .and_then(Value::as_array)
                .ok_or_else(|| "writable field discovery nodes is not an array".to_owned())?
                .iter()
                .cloned(),
        );
        if connection
            .pointer("/pageInfo/hasNextPage")
            .and_then(Value::as_bool)
            != Some(true)
        {
            return Ok(fields);
        }
        let next = connection
            .pointer("/pageInfo/endCursor")
            .and_then(Value::as_str)
            .ok_or_else(|| "writable field discovery has no advancing cursor".to_owned())?;
        if after.as_str() == Some(next) {
            return Err("writable field discovery cursor did not advance".to_owned());
        }
        after = Value::String(next.to_owned());
    }
}

async fn ensure_origin_field(token: &str, project_id: &str) -> Result<bool, String> {
    if writable_fields(token, project_id)
        .await?
        .iter()
        .any(|field| field.get("name").and_then(Value::as_str) == Some(ORIGIN_FIELD))
    {
        return Ok(false);
    }
    let response = graphql_variables(
        token,
        "mutation($input:CreateProjectV2FieldInput!){createProjectV2Field(input:$input){projectV2Field{... on ProjectV2Field{id name}}}}",
        "live origin field creation",
        json!({"input":{"projectId":project_id,"dataType":"TEXT","name":ORIGIN_FIELD}}),
    )
    .await?;
    if response
        .pointer("/data/createProjectV2Field/projectV2Field/name")
        .and_then(Value::as_str)
        != Some(ORIGIN_FIELD)
    {
        return Err("GitHub did not confirm creation of the live origin field".to_owned());
    }
    Ok(true)
}

async fn live_write_status(token: &str, project_id: &str) -> Result<String, String> {
    let fields = writable_fields(token, project_id).await?;
    fields
        .iter()
        .find(|field| field.get("name").and_then(Value::as_str) == Some("Status"))
        .and_then(|field| field.get("options"))
        .and_then(Value::as_array)
        .and_then(|options| options.first())
        .and_then(|option| option.get("name"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| "live project has no selectable Status option".to_owned())
}

/// The write configuration this lane builds for the board it was pointed at.
///
/// Writes go by status *category*, and the board's own first Status option is the only
/// column this lane knows exists — so `todo` is pointed at it and every other
/// column-bearing category is disabled. Exactly one category writes a column, so however
/// this board spells that option, no two categories can send it the same one and the
/// source's own validation has nothing to refuse. Pointing `unknown` at that column
/// instead is the collision itself: `unknown` and `draft` map to no column by design,
/// precisely so neither can collide with a category that has one.
fn live_write_config(
    owner: &str,
    project_number: u32,
    repository: &str,
    status_option: &str,
) -> Value {
    json!({"owner":owner,"project_number":project_number,"repository":repository,
           "status_mapping":{"todo":status_option,"backlog":null,"in-progress":null}})
}

async fn remove_live_origin_field(token: &str, project_id: &str) -> Result<(), String> {
    for _ in 0..10 {
        let field_ids = writable_fields(token, project_id)
            .await?
            .into_iter()
            .filter(|field| field.get("name").and_then(Value::as_str) == Some(ORIGIN_FIELD))
            .map(|field| {
                field
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .ok_or_else(|| "live origin field has no id".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()?;
        if field_ids.is_empty() {
            return Ok(());
        }
        for field_id in field_ids {
            graphql_variables(
                token,
                "mutation($input:DeleteProjectV2FieldInput!){deleteProjectV2Field(input:$input){projectV2Field{... on ProjectV2Field{id}}}}",
                "live origin field cleanup",
                json!({"input":{"fieldId":field_id}}),
            )
            .await?;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    Err("live origin field cleanup left the temporary field behind".to_owned())
}

/// The prefix of every board item this lane writes.
///
/// The rest of a title is `<process id>-<microsecond timestamp>`, which makes one run's artifact
/// unique and makes any run's artifact recognisable to the next run.
const ARTIFACT_PREFIX: &str = "onetaskgraph live cleanup ";

fn artifact_title(process_id: u32, stamp_micros: i64) -> String {
    format!("{ARTIFACT_PREFIX}{process_id}-{stamp_micros}")
}

/// Whether a board item is one this lane wrote, in this run or in an earlier one.
fn is_artifact_title(title: &str) -> bool {
    let Some(suffix) = title.strip_prefix(ARTIFACT_PREFIX) else {
        return false;
    };
    let Some((process_id, stamp_micros)) = suffix.split_once('-') else {
        return false;
    };
    [process_id, stamp_micros]
        .iter()
        .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

/// One artifact this lane wrote: its board item, and the issue behind it when there is one.
type Artifact = (String, Option<String>);

async fn artifact_item_ids(
    token: &str,
    project_id: &str,
    matches: &dyn Fn(&str) -> bool,
) -> Result<Vec<Artifact>, String> {
    let mut after = Value::Null;
    let mut found = Vec::new();
    loop {
        let response = graphql_variables(
            token,
            "query($id:ID!,$after:String){node(id:$id){... on ProjectV2{items(first:100,after:$after){nodes{id content{... on DraftIssue{title} ... on Issue{__typename id title}}}pageInfo{hasNextPage endCursor}}}}}",
            "live artifact lookup",
            json!({"id":project_id,"after":after}),
        )
        .await?;
        let connection = response
            .pointer("/data/node/items")
            .ok_or_else(|| "live artifact lookup returned no items connection".to_owned())?;
        let nodes = connection
            .get("nodes")
            .and_then(Value::as_array)
            .ok_or_else(|| "live artifact lookup nodes is not an array".to_owned())?;
        for node in nodes {
            let Some(title) = node.pointer("/content/title").and_then(Value::as_str) else {
                continue;
            };
            if matches(title) {
                let issue = (node.pointer("/content/__typename").and_then(Value::as_str)
                    == Some("Issue"))
                .then(|| node.pointer("/content/id").and_then(Value::as_str))
                .flatten()
                .map(str::to_owned);
                found.push((
                    node.get("id")
                        .and_then(Value::as_str)
                        .ok_or_else(|| "live artifact has no project item id".to_owned())?
                        .to_owned(),
                    issue,
                ));
            }
        }
        if connection
            .pointer("/pageInfo/hasNextPage")
            .and_then(Value::as_bool)
            != Some(true)
        {
            return Ok(found);
        }
        let next = connection
            .pointer("/pageInfo/endCursor")
            .and_then(Value::as_str)
            .ok_or_else(|| "live artifact lookup has no advancing cursor".to_owned())?;
        if after.as_str() == Some(next) {
            return Err("live artifact lookup cursor did not advance".to_owned());
        }
        after = Value::String(next.to_owned());
    }
}

async fn remove_live_artifacts(
    token: &str,
    project_id: &str,
    matches: &dyn Fn(&str) -> bool,
) -> Result<(), String> {
    for _ in 0..10 {
        let item_ids = artifact_item_ids(token, project_id, matches).await?;
        if item_ids.is_empty() {
            return Ok(());
        }
        for (item_id, issue_id) in item_ids {
            let response = graphql_variables(
                token,
                "mutation($input:DeleteProjectV2ItemInput!){deleteProjectV2Item(input:$input){deletedItemId}}",
                "live artifact cleanup",
                json!({"input":{"projectId":project_id,"itemId":item_id}}),
            )
            .await?;
            if response
                .pointer("/data/deleteProjectV2Item/deletedItemId")
                .and_then(Value::as_str)
                != Some(item_id.as_str())
            {
                return Err(format!(
                    "GitHub did not confirm deletion of project item {item_id}"
                ));
            }
            // Taking the item off the board leaves the issue in the repository, and this
            // lane's whole claim is that it leaves no residue anywhere.
            if let Some(issue_id) = issue_id {
                graphql_variables(
                    token,
                    "mutation($input:DeleteIssueInput!){deleteIssue(input:$input){repository{id}}}",
                    "live artifact issue cleanup",
                    json!({"input":{"issueId":issue_id}}),
                )
                .await?;
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    Err(format!(
        "live artifact cleanup left project items: {}",
        artifact_item_ids(token, project_id, matches)
            .await?
            .into_iter()
            .map(|(item, _)| item)
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

/// A live assertion that returns rather than panics.
///
/// Every check inside the journey below has to reach `run_then_cleanup` as an `Err`: a
/// panic would unwind past the cleanup and leave this run's projects, tasks, issues and
/// label on the board for the next run to find.
macro_rules! ensure {
    ($condition:expr, $($message:tt)+) => {
        if !$condition {
            return Err(format!($($message)+));
        }
    };
}

/// Whether a board item is one *this* run wrote.
///
/// Every artifact of one run carries this process's id, so a run names its own for
/// cleanup without touching one an interrupted earlier run left for [`is_artifact_title`]
/// to sweep.
fn is_run_artifact_title(process_id: u32, title: &str) -> bool {
    is_artifact_title(title) && title.starts_with(&format!("{ARTIFACT_PREFIX}{process_id}-"))
}

/// The prefix of the one repository label this lane creates.
///
/// A label this lane created is residue exactly as an issue is, so it is named the way
/// board items are — this process's id and a timestamp — and swept the same way before a
/// run starts. The grammar [`is_artifact_label`] accepts is letters, digits and hyphens
/// only, which is what lets the cleanup below name one in a URL path unescaped.
const LABEL_PREFIX: &str = "onetaskgraph-live-";

fn artifact_label(process_id: u32, stamp_micros: i64) -> String {
    format!("{LABEL_PREFIX}{process_id}-{stamp_micros}")
}

/// Whether a repository label is one this lane created, in this run or in an earlier one.
fn is_artifact_label(name: &str) -> bool {
    let Some(suffix) = name.strip_prefix(LABEL_PREFIX) else {
        return false;
    };
    let Some((process_id, stamp_micros)) = suffix.split_once('-') else {
        return false;
    };
    [process_id, stamp_micros]
        .iter()
        .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

/// One REST call to GitHub, for the label lifecycle GraphQL puts behind a schema preview.
async fn rest(
    token: &str,
    method: reqwest::Method,
    url: &str,
    body: Option<Value>,
    what: &str,
) -> Result<Value, String> {
    let mut request = reqwest::Client::new()
        .request(method, url)
        .header("user-agent", "onetaskgraph-live-test")
        .header("accept", "application/vnd.github+json")
        .bearer_auth(token);
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request
        .send()
        .await
        .map_err(|error| format!("{what} could not reach GitHub: {error}"))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|error| format!("{what} returned no readable body: {error}"))?;
    if !status.is_success() {
        return Err(format!("{what} failed with HTTP {status}: {text}"));
    }
    if text.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(&text).map_err(|error| format!("{what} returned invalid JSON: {error}"))
}

/// Creates the one repository label this run filters by, and reports its node id.
async fn create_artifact_label(
    token: &str,
    repository: &str,
    name: &str,
) -> Result<String, String> {
    let created = rest(
        token,
        reqwest::Method::POST,
        &format!("https://api.github.com/repos/{repository}/labels"),
        Some(json!({"name":name,"color":"ededed",
                    "description":"temporary onetaskgraph live-lane label"})),
        "live label creation",
    )
    .await?;
    created
        .get("node_id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("live label creation returned no node id: {created}"))
}

async fn attach_artifact_label(token: &str, issue_id: &str, label_id: &str) -> Result<(), String> {
    let response = graphql_variables(
        token,
        "mutation($input:AddLabelsToLabelableInput!){addLabelsToLabelable(input:$input){labelable{... on Issue{id}}}}",
        "live label attachment",
        json!({"input":{"labelableId":issue_id,"labelIds":[label_id]}}),
    )
    .await?;
    if response
        .pointer("/data/addLabelsToLabelable/labelable/id")
        .and_then(Value::as_str)
        != Some(issue_id)
    {
        return Err(format!(
            "GitHub did not confirm attaching the live label to issue {issue_id}"
        ));
    }
    Ok(())
}

async fn remove_artifact_labels(
    token: &str,
    repository: &str,
    matches: &dyn Fn(&str) -> bool,
) -> Result<(), String> {
    let mut names = Vec::new();
    for number in 1..=50 {
        let listed = rest(
            token,
            reqwest::Method::GET,
            &format!("https://api.github.com/repos/{repository}/labels?per_page=100&page={number}"),
            None,
            "live label lookup",
        )
        .await?;
        let nodes = listed
            .as_array()
            .ok_or_else(|| "live label lookup did not return a list of labels".to_owned())?;
        names.extend(
            nodes
                .iter()
                .filter_map(|node| node.get("name").and_then(Value::as_str))
                .filter(|name| matches(name))
                .map(str::to_owned),
        );
        if nodes.len() < 100 {
            break;
        }
    }
    for name in &names {
        // Safe unescaped: `matches` accepts only the grammar `LABEL_PREFIX` documents.
        rest(
            token,
            reqwest::Method::DELETE,
            &format!("https://api.github.com/repos/{repository}/labels/{name}"),
            None,
            "live label cleanup",
        )
        .await?;
    }
    Ok(())
}

/// Everything one run of this lane created, removed whether its journey passed or failed.
///
/// Three stores, because a run writes to three: the board holds its items, the repository
/// holds the issues behind them and the label they filter by, and the board's own field
/// set holds the origin field the write path needs. Every one of them is swept, and every
/// failure is reported rather than the first, because residue left in one is residue the
/// next run has to heal.
async fn remove_live_state(
    token: &str,
    project_id: &str,
    repository: &str,
    process_id: u32,
    remove_origin_field: bool,
) -> Result<(), String> {
    let item_result = remove_live_artifacts(token, project_id, &|title| {
        is_run_artifact_title(process_id, title)
    })
    .await;
    let label_result = remove_artifact_labels(token, repository, &|name| {
        is_artifact_label(name) && name.starts_with(&format!("{LABEL_PREFIX}{process_id}-"))
    })
    .await;
    let field_result = if remove_origin_field {
        remove_live_origin_field(token, project_id).await
    } else {
        Ok(())
    };
    let problems = [item_result, label_result, field_result]
        .into_iter()
        .filter_map(Result::err)
        .collect::<Vec<_>>();
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("; additionally, "))
    }
}

async fn run_then_cleanup<J, JF, C, CF>(journey: J, cleanup: C) -> Result<(), String>
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

#[derive(Debug, PartialEq, Eq)]
enum LiveLane {
    Run {
        token: String,
        owner: String,
        project_number: u32,
        repository: String,
    },
    Skip(String),
}

/// Decides whether this lane may run, and against which board.
///
/// The board comes from `GH_PROJECTS_OWNER` and `GH_PROJECTS_NUMBER`, and the repository this
/// source creates its issues in comes from `GH_PROJECTS_REPOSITORY`, or the lane does not run.
/// Nothing here asks GitHub which project was updated most recently, for the viewer or for any
/// organization: a credentialed lane that writes and deletes must reach only a board somebody
/// nominated by name, and that requirement — rather than any cleanup — is what keeps it off a
/// board nobody nominated. `ONETASKGRAPH_LIVE_REQUIRED=1` turns a skip into
/// a failure, the same pairing an absent credential already has. `Err` is a misconfiguration,
/// which fails whether or not the lane is required.
fn live_lane(
    token: Option<&str>,
    owner: Option<&str>,
    project_number: Option<&str>,
    repository: Option<&str>,
    required: Option<&str>,
) -> Result<LiveLane, String> {
    let required = match required.map(str::trim) {
        None | Some("") | Some("0") => false,
        Some("1") => true,
        Some(other) => {
            return Err(format!(
                "ONETASKGRAPH_LIVE_REQUIRED must be 1, 0 or unset, not {other:?}"
            ));
        }
    };
    let skip = |reason: String| -> Result<LiveLane, String> {
        if required {
            return Err(format!(
                "{reason}, and ONETASKGRAPH_LIVE_REQUIRED=1 requires the GitHub Projects live \
                 lane to run"
            ));
        }
        Ok(LiveLane::Skip(reason))
    };
    // llmlint: ignore[live_tier_compiles_and_requires_credential] An absent credential
    // skipping is the recorded decision, not an oversight: AGENTS.md keeps this lane off
    // the required checks because a required check a third party can turn red stops being
    // trusted, and a Linear or GitHub outage must not block an unrelated merge. What makes
    // the decision true rather than stated is that the lane is `#[ignore]`d and only
    // `test-live` runs it — with `ONETASKGRAPH_LIVE_REQUIRED=1`, which turns every skip
    // below into the failure this rule asks for.
    let Some(token) = token else {
        return skip("GH_PROJECTS_TOKEN is not set".to_owned());
    };
    if token.trim().is_empty() {
        return skip("GH_PROJECTS_TOKEN is empty".to_owned());
    }
    let owner = owner.map(str::trim).filter(|owner| !owner.is_empty());
    let project_number = project_number
        .map(str::trim)
        .filter(|number| !number.is_empty());
    let (owner, project_number) = match (owner, project_number) {
        (Some(owner), Some(project_number)) => (owner, project_number),
        (None, None) => {
            return skip(
                "GH_PROJECTS_OWNER and GH_PROJECTS_NUMBER are not set, and this lane writes only \
                 to the board those two name rather than discovering one"
                    .to_owned(),
            );
        }
        (owner, _) => {
            return Err(format!(
                "GH_PROJECTS_OWNER and GH_PROJECTS_NUMBER name one board together: {} is missing",
                if owner.is_some() {
                    "GH_PROJECTS_NUMBER"
                } else {
                    "GH_PROJECTS_OWNER"
                }
            ));
        }
    };
    let number = project_number
        .parse::<u32>()
        .ok()
        .filter(|number| *number > 0 && *number <= i32::MAX as u32)
        .ok_or_else(|| {
            format!("GH_PROJECTS_NUMBER must be a positive GraphQL Int, not {project_number:?}")
        })?;
    let Some(repository) = repository
        .map(str::trim)
        .filter(|repository| !repository.is_empty())
    else {
        return skip(
            "GH_PROJECTS_REPOSITORY is not set, and this lane creates its artifact as an issue \
             in the repository that name gives rather than discovering one"
                .to_owned(),
        );
    };
    if repository
        .split_once('/')
        .is_none_or(|(owner, name)| owner.is_empty() || name.is_empty() || name.contains('/'))
    {
        return Err(format!(
            "GH_PROJECTS_REPOSITORY must be spelled owner/name, not {repository:?}"
        ));
    }
    Ok(LiveLane::Run {
        token: token.to_owned(),
        owner: owner.to_owned(),
        project_number: number,
        repository: repository.to_owned(),
    })
}

/// Reads the node id of the one nominated board, so residue can be cleared before the journey
/// builds the source it later reads the same board through.
async fn nominated_project_id(
    token: &str,
    owner: &str,
    project_number: u32,
) -> Result<String, String> {
    let response = graphql_variables(
        token,
        "query($owner:String!,$number:Int!){repositoryOwner(login:$owner){... on ProjectV2Owner{projectV2(number:$number){id}}}}",
        "nominated board lookup",
        json!({"owner":owner,"number":project_number}),
    )
    .await?;
    response
        .pointer("/data/repositoryOwner/projectV2/id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| {
            format!(
                "GH_PROJECTS_OWNER={owner} with GH_PROJECTS_NUMBER={project_number} names no \
                 project this credential can see"
            )
        })
}

fn named_type(value: &Value) -> Option<&str> {
    value
        .get("name")
        .and_then(Value::as_str)
        .or_else(|| value.get("ofType").and_then(named_type))
}

fn type_signature(value: &Value) -> Option<String> {
    match value.get("kind").and_then(Value::as_str)? {
        "NON_NULL" => Some(format!("{}!", type_signature(value.get("ofType")?)?)),
        "LIST" => Some(format!("[{}]", type_signature(value.get("ofType")?)?)),
        _ => value.get("name").and_then(Value::as_str).map(str::to_owned),
    }
}

async fn verify_mutation_schema(token: &str) -> Result<(), String> {
    let response = graphql(
        token,
        "query MutationContract { __type(name:\"Mutation\") { fields { name type { name ofType { name } } args { name type { name ofType { name } } } } } }",
        "mutation contract introspection",
    )
    .await?;
    let fields = response
        .pointer("/data/__type/fields")
        .and_then(Value::as_array)
        .ok_or_else(|| "mutation contract introspection returned no fields".to_owned())?;
    for (field_name, input_name, payload_name) in [
        ("createIssue", "CreateIssueInput", "CreateIssuePayload"),
        (
            "addProjectV2ItemById",
            "AddProjectV2ItemByIdInput",
            "AddProjectV2ItemByIdPayload",
        ),
        ("addSubIssue", "AddSubIssueInput", "AddSubIssuePayload"),
        (
            "removeSubIssue",
            "RemoveSubIssueInput",
            "RemoveSubIssuePayload",
        ),
        ("addBlockedBy", "AddBlockedByInput", "AddBlockedByPayload"),
        (
            "removeBlockedBy",
            "RemoveBlockedByInput",
            "RemoveBlockedByPayload",
        ),
        (
            "deleteProjectV2Item",
            "DeleteProjectV2ItemInput",
            "DeleteProjectV2ItemPayload",
        ),
        ("deleteIssue", "DeleteIssueInput", "DeleteIssuePayload"),
        (
            "createProjectV2Field",
            "CreateProjectV2FieldInput",
            "CreateProjectV2FieldPayload",
        ),
        (
            "deleteProjectV2Field",
            "DeleteProjectV2FieldInput",
            "DeleteProjectV2FieldPayload",
        ),
        ("updateIssue", "UpdateIssueInput", "UpdateIssuePayload"),
        (
            "updateProjectV2DraftIssue",
            "UpdateProjectV2DraftIssueInput",
            "UpdateProjectV2DraftIssuePayload",
        ),
        (
            "updateProjectV2ItemFieldValue",
            "UpdateProjectV2ItemFieldValueInput",
            "UpdateProjectV2ItemFieldValuePayload",
        ),
    ] {
        let field = fields
            .iter()
            .find(|field| field.get("name").and_then(Value::as_str) == Some(field_name))
            .ok_or_else(|| format!("GitHub mutation schema has no {field_name} field"))?;
        let input = field
            .get("args")
            .and_then(Value::as_array)
            .and_then(|args| {
                args.iter()
                    .find(|argument| argument.get("name").and_then(Value::as_str) == Some("input"))
            })
            .and_then(|argument| argument.get("type"))
            .and_then(named_type);
        if input != Some(input_name) {
            return Err(format!(
                "GitHub mutation {field_name} input changed: expected {input_name}, got {input:?}"
            ));
        }
        let payload = field.get("type").and_then(named_type);
        if payload != Some(payload_name) {
            return Err(format!(
                "GitHub mutation {field_name} payload changed: expected {payload_name}, got {payload:?}"
            ));
        }
    }
    for (type_name, input, expected_fields) in [
        (
            "CreateIssueInput",
            true,
            &["repositoryId", "title", "body"][..],
        ),
        (
            "AddProjectV2ItemByIdInput",
            true,
            &["projectId", "contentId"][..],
        ),
        ("AddSubIssueInput", true, &["issueId", "subIssueId"][..]),
        ("RemoveSubIssueInput", true, &["issueId", "subIssueId"][..]),
        (
            "UpdateProjectV2DraftIssueInput",
            true,
            &["draftIssueId", "title", "body"][..],
        ),
        (
            "UpdateIssueInput",
            true,
            &["id", "title", "body", "stateInput"][..],
        ),
        ("IssueStateUpdateInput", true, &["value", "stateReason"][..]),
        (
            "UpdateProjectV2ItemFieldValueInput",
            true,
            &["projectId", "itemId", "fieldId", "value"][..],
        ),
        (
            "ProjectV2FieldValue",
            true,
            &["text", "singleSelectOptionId"][..],
        ),
        (
            "AddBlockedByInput",
            true,
            &["issueId", "blockingIssueId"][..],
        ),
        (
            "RemoveBlockedByInput",
            true,
            &["issueId", "blockingIssueId"][..],
        ),
        (
            "DeleteProjectV2ItemInput",
            true,
            &["projectId", "itemId"][..],
        ),
        ("DeleteIssueInput", true, &["issueId"][..]),
        (
            "CreateProjectV2FieldInput",
            true,
            &["projectId", "dataType", "name"][..],
        ),
        ("DeleteProjectV2FieldInput", true, &["fieldId"][..]),
        ("CreateIssuePayload", false, &["issue"][..]),
        ("AddProjectV2ItemByIdPayload", false, &["item"][..]),
        ("AddSubIssuePayload", false, &["issue", "subIssue"][..]),
        ("RemoveSubIssuePayload", false, &["issue", "subIssue"][..]),
        (
            "UpdateProjectV2DraftIssuePayload",
            false,
            &["draftIssue"][..],
        ),
        ("UpdateIssuePayload", false, &["issue"][..]),
        (
            "UpdateProjectV2ItemFieldValuePayload",
            false,
            &["projectV2Item"][..],
        ),
        (
            "AddBlockedByPayload",
            false,
            &["issue", "blockingIssue"][..],
        ),
        (
            "RemoveBlockedByPayload",
            false,
            &["issue", "blockingIssue"][..],
        ),
        ("DeleteProjectV2ItemPayload", false, &["deletedItemId"][..]),
        ("DeleteIssuePayload", false, &["repository"][..]),
        (
            "CreateProjectV2FieldPayload",
            false,
            &["projectV2Field"][..],
        ),
        (
            "DeleteProjectV2FieldPayload",
            false,
            &["projectV2Field"][..],
        ),
    ] {
        let selection = if input { "inputFields" } else { "fields" };
        let document = format!(
            "query TypeContract {{ __type(name:\"{type_name}\") {{ {selection} {{ name type {{ kind name ofType {{ kind name ofType {{ kind name }} }} }} }} }} }}"
        );
        let response = graphql(token, &document, "mutation type introspection").await?;
        let fields = response
            .pointer(&format!("/data/__type/{selection}"))
            .and_then(Value::as_array)
            .ok_or_else(|| format!("GitHub mutation schema has no {type_name} {selection}"))?;
        for expected in expected_fields {
            if !fields
                .iter()
                .any(|field| field.get("name").and_then(Value::as_str) == Some(expected))
            {
                return Err(format!(
                    "GitHub mutation type {type_name} has no {expected} field"
                ));
            }
        }
        for (field_name, expected_type) in mutation_field_types(type_name) {
            let field = fields
                .iter()
                .find(|field| field.get("name").and_then(Value::as_str) == Some(field_name))
                .ok_or_else(|| {
                    format!("GitHub mutation type {type_name} has no {field_name} field")
                })?;
            let actual = field.get("type").and_then(type_signature);
            if actual.as_deref() != Some(expected_type) {
                return Err(format!(
                    "GitHub mutation type {type_name}.{field_name} changed: expected {expected_type}, got {actual:?}"
                ));
            }
        }
    }
    Ok(())
}

fn mutation_field_types(type_name: &str) -> &'static [(&'static str, &'static str)] {
    match type_name {
        "CreateIssueInput" => &[
            ("repositoryId", "ID!"),
            ("title", "String!"),
            ("body", "String"),
        ],
        "AddProjectV2ItemByIdInput" => &[("projectId", "ID!"), ("contentId", "ID!")],
        "AddSubIssueInput" => &[("issueId", "ID!"), ("subIssueId", "ID")],
        "RemoveSubIssueInput" => &[("issueId", "ID!"), ("subIssueId", "ID!")],
        "UpdateProjectV2DraftIssueInput" => &[
            ("draftIssueId", "ID!"),
            ("title", "String"),
            ("body", "String"),
        ],
        "UpdateIssueInput" => &[
            ("id", "ID!"),
            ("title", "String"),
            ("body", "String"),
            ("stateInput", "IssueStateUpdateInput"),
        ],
        // The two facts this redesign rests on: an issue's state moves with its title and
        // body in one mutation, and the reason is what tells done from cancelled.
        "IssueStateUpdateInput" => &[
            ("value", "IssueState!"),
            ("stateReason", "IssueClosedStateReason"),
        ],
        "UpdateProjectV2ItemFieldValueInput" => &[
            ("projectId", "ID!"),
            ("itemId", "ID!"),
            ("fieldId", "ID!"),
            ("value", "ProjectV2FieldValue!"),
        ],
        "ProjectV2FieldValue" => &[("text", "String"), ("singleSelectOptionId", "String")],
        "AddBlockedByInput" | "RemoveBlockedByInput" => {
            &[("issueId", "ID!"), ("blockingIssueId", "ID!")]
        }
        "DeleteProjectV2ItemInput" => &[("projectId", "ID!"), ("itemId", "ID!")],
        "DeleteIssueInput" => &[("issueId", "ID!")],
        "CreateProjectV2FieldInput" => &[
            ("projectId", "ID!"),
            ("dataType", "ProjectV2CustomFieldType!"),
            ("name", "String!"),
        ],
        "DeleteProjectV2FieldInput" => &[("fieldId", "ID!")],
        "CreateIssuePayload" | "UpdateIssuePayload" => &[("issue", "Issue")],
        "AddProjectV2ItemByIdPayload" => &[("item", "ProjectV2Item")],
        "AddSubIssuePayload" | "RemoveSubIssuePayload" => {
            &[("issue", "Issue"), ("subIssue", "Issue")]
        }
        "UpdateProjectV2DraftIssuePayload" => &[("draftIssue", "DraftIssue")],
        "UpdateProjectV2ItemFieldValuePayload" => &[("projectV2Item", "ProjectV2Item")],
        "AddBlockedByPayload" | "RemoveBlockedByPayload" => {
            &[("issue", "Issue"), ("blockingIssue", "Issue")]
        }
        "DeleteProjectV2ItemPayload" => &[("deletedItemId", "ID")],
        "DeleteIssuePayload" => &[("repository", "Repository")],
        "CreateProjectV2FieldPayload" | "DeleteProjectV2FieldPayload" => {
            &[("projectV2Field", "ProjectV2FieldConfiguration")]
        }
        _ => &[],
    }
}

fn page(cursor: Option<onetaskgraph_plugin_api::Cursor>) -> PageRequest {
    PageRequest { cursor, limit: 50 }
}

/// The one board, repository and naming this run may write under.
struct LiveRun {
    token: String,
    repository: String,
    project_id: String,
    process_id: u32,
    stamp_micros: i64,
    status_option: String,
}

impl LiveRun {
    /// The title of this run's `offset`-th artifact.
    ///
    /// One stamp per artifact, so every title this run writes is unique and every one of
    /// them still reads as this run's to [`is_run_artifact_title`] and as the lane's own
    /// to the sweep the next run does.
    fn title(&self, offset: i64) -> String {
        artifact_title(self.process_id, self.stamp_micros + offset)
    }

    /// The prefix no other item on the board carries, which is what lets the listings
    /// below assert an exact set rather than a containment.
    fn prefix(&self) -> String {
        format!("{ARTIFACT_PREFIX}{}-", self.process_id)
    }
}

fn artifact_project(title: &str, status: &Status) -> Project {
    Project {
        id: NativeId("live-source-item".into()),
        title: title.to_owned(),
        content: Some("temporary credentialed write; the live lane removes this".into()),
        status: status.clone(),
        labels: vec![],
        url: None,
        created_at: None,
        updated_at: None,
        metadata: BTreeMap::new(),
        repositories: vec![],
    }
}

fn artifact_task(
    title: &str,
    content: String,
    status: &Status,
    project: Option<NativeId>,
    metadata: BTreeMap<String, Value>,
) -> Task {
    Task {
        id: NativeId("live-source-item".into()),
        title: title.to_owned(),
        content: Some(content),
        status: status.clone(),
        labels: vec![],
        project,
        url: None,
        created_at: None,
        updated_at: None,
        metadata,
        repositories: vec![],
    }
}

/// The edge a written item records as one it depends on.
///
/// Only `to` decides where the relationship goes: the source names the near end from the
/// item it is writing, which has no id of its own until GitHub creates it.
fn blocks(far: &NativeId, kind: ItemKind) -> DependencyEdge {
    DependencyEdge {
        from: DependencyEndpoint::from_native(NativeId("live-source-item".into()), kind),
        to: DependencyEndpoint::from_native(far.clone(), kind),
        kind: DependencyKind::Blocks,
    }
}

fn sorted(mut titles: Vec<String>) -> Vec<String> {
    titles.sort();
    titles
}

async fn task_titles(
    source: &dyn TaskSource,
    query: &TaskQuery,
    what: &str,
) -> Result<Vec<String>, String> {
    Ok(sorted(
        source
            .query_tasks(query, &page(None))
            .await
            .map_err(|error| format!("live {what} failed: {error}"))?
            .items
            .into_iter()
            .map(|task| task.title)
            .collect(),
    ))
}

async fn project_titles(
    source: &dyn TaskSource,
    query: &ProjectQuery,
    what: &str,
) -> Result<Vec<String>, String> {
    Ok(sorted(
        source
            .query_projects(query, &page(None))
            .await
            .map_err(|error| format!("live {what} failed: {error}"))?
            .items
            .into_iter()
            .map(|project| project.title)
            .collect(),
    ))
}

/// One page of the nominated board's items, asked for at exactly `first`.
///
/// Reads nothing this lane needs; what it establishes is whether GitHub's own connection
/// accepts that page size, which is what `max_page_size` claims to describe.
async fn board_items_page(token: &str, project_id: &str, first: u32) -> Result<Value, String> {
    graphql_variables(
        token,
        "query($id:ID!,$first:Int!){node(id:$id){... on ProjectV2{items(first:$first){nodes{id}}}}}",
        "board page size probe",
        json!({"id":project_id,"first":first}),
    )
    .await
}

/// Waits until the board itself reports an item this run just created.
///
/// `addProjectV2ItemById` returns before GitHub's own `ProjectV2.items` connection lists
/// the new item, and a write naming that item as a dependency resolves the far end
/// through exactly that connection — so a fixture that referenced it the moment it was
/// created would be refused for an item which by then certainly exists.
async fn await_on_board(
    writer: &dyn TaskSource,
    id: &NativeId,
    kind: ItemKind,
) -> Result<(), String> {
    for _ in 0..30 {
        let seen = match kind {
            ItemKind::Task => writer.get_task(id).await.map(|task| task.is_some()),
            ItemKind::Project => writer
                .get_project(id)
                .await
                .map(|project| project.is_some()),
        }
        .map_err(|error| {
            format!(
                "waiting for a created {} to reach the board failed: {error}",
                kind.marker()
            )
        })?;
        if seen {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    Err(format!(
        "the board never reported the {} this run created ({})",
        kind.marker(),
        id.0
    ))
}

/// Drives every field of the source's declared `Capabilities` against the real board.
///
/// The fixture is five items this run creates: two projects, one task filed under each,
/// and one task filed under neither. That shape is what makes an honoured predicate and
/// an ignored one *different answers* rather than the same one — a project filter over a
/// board holding a single project, or a label filter over a board where every item
/// carries the label, passes whether or not the source applies it, and a predicate
/// declared and then not applied is exactly the defect this lane exists to catch.
///
/// Nothing here panics: every failure returns, so the caller's cleanup runs over a board
/// this run is still holding artifacts on.
async fn drive_every_declared_capability(
    run: &LiveRun,
    writer: &dyn TaskSource,
) -> Result<(), String> {
    let (alpha, beta) = (run.title(0), run.title(1));
    let (first, second, orphan) = (run.title(2), run.title(3), run.title(4));
    let prefix = run.prefix();
    let body_marker = format!("livebodymarker{}x{}", run.process_id, run.stamp_micros);
    let label_name = artifact_label(run.process_id, run.stamp_micros);
    let open = Status {
        category: StatusCategory::Todo,
        name: run.status_option.clone(),
    };
    // The one category this board reaches without an option of its own: a closed issue
    // carries `done` in its own state, so the fixture separates by status without needing
    // a second column that however this board is set up may not exist.
    let closed = Status {
        category: StatusCategory::Done,
        name: "Done".into(),
    };
    let by_prefix = || TaskQuery {
        text: Some(TextQuery {
            terms: prefix.clone(),
            fields: TextFields::Title,
        }),
        ..Default::default()
    };

    let alpha_id = writer
        .write_project(&ItemWrite {
            target: None,
            item: artifact_project(&alpha, &open),
            depends_on: vec![],
        })
        .await
        .map_err(|error| format!("live project write of {alpha:?} failed: {error}"))?;
    await_on_board(writer, &alpha_id, ItemKind::Project).await?;
    let beta_id = writer
        .write_project(&ItemWrite {
            target: None,
            item: artifact_project(&beta, &open),
            depends_on: vec![blocks(&alpha_id, ItemKind::Project)],
        })
        .await
        .map_err(|error| format!("live project write of {beta:?} failed: {error}"))?;
    let mut round_trip = BTreeMap::new();
    round_trip.insert(
        "live.round_trip".to_owned(),
        json!({"nested":[1,true,null]}),
    );
    let first_id = writer
        .write_task(&ItemWrite {
            target: None,
            item: artifact_task(
                &first,
                format!("temporary credentialed write; {body_marker}"),
                &open,
                Some(alpha_id.clone()),
                round_trip,
            ),
            depends_on: vec![],
        })
        .await
        .map_err(|error| format!("live task write of {first:?} failed: {error}"))?;
    await_on_board(writer, &first_id, ItemKind::Task).await?;
    let second_id = writer
        .write_task(&ItemWrite {
            target: None,
            item: artifact_task(
                &second,
                "temporary credentialed write; the live lane removes this".into(),
                &open,
                Some(beta_id.clone()),
                BTreeMap::new(),
            ),
            depends_on: vec![blocks(&first_id, ItemKind::Task)],
        })
        .await
        .map_err(|error| format!("live task write of {second:?} failed: {error}"))?;
    let orphan_id = writer
        .write_task(&ItemWrite {
            target: None,
            item: artifact_task(
                &orphan,
                "temporary credentialed write; the live lane removes this".into(),
                &closed,
                None,
                BTreeMap::new(),
            ),
            depends_on: vec![],
        })
        .await
        .map_err(|error| format!("live task write of {orphan:?} failed: {error}"))?;
    let label_id = create_artifact_label(&run.token, &run.repository, &label_name).await?;
    attach_artifact_label(&run.token, &first_id.0, &label_id).await?;

    // GitHub decides when a created issue appears on the board, so the reads below wait
    // for the fixture rather than racing it.
    let mut readable = None;
    for _ in 0..20 {
        let tasks = task_titles(writer, &by_prefix(), "fixture settling task read").await?;
        let projects = project_titles(
            writer,
            &ProjectQuery {
                text: Some(TextQuery {
                    terms: prefix.clone(),
                    fields: TextFields::Title,
                }),
                ..Default::default()
            },
            "fixture settling project read",
        )
        .await?;
        if tasks.len() == 3 && projects.len() == 2 {
            readable = Some((tasks, projects));
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    let Some((run_tasks, run_projects)) = readable else {
        return Err(format!(
            "the live fixture never became readable: the board never reported all three \
             tasks and both projects titled {prefix}*"
        ));
    };

    // `search_title` and `projects`: one title search over the whole board selects exactly
    // this run's five items, three of which are tasks and two of which are projects.
    ensure!(
        run_tasks == sorted(vec![first.clone(), second.clone(), orphan.clone()]),
        "a title search for this run's own prefix returned {run_tasks:?}"
    );
    ensure!(
        run_projects == sorted(vec![alpha.clone(), beta.clone()]),
        "a title search for this run's own prefix returned the projects {run_projects:?}"
    );
    let read_alpha = writer
        .get_project(&alpha_id)
        .await
        .map_err(|error| format!("live project read-back failed: {error}"))?;
    ensure!(
        read_alpha.as_ref().map(|project| project.title.as_str()) == Some(alpha.as_str()),
        "the written project did not read back by its own id: {read_alpha:?}"
    );

    // `search_content`: the marker is in one body and in no title at all, so a content
    // search finds that one task, a title search finds none, and an either-field search
    // finds it again.
    let searching = |fields| TaskQuery {
        text: Some(TextQuery {
            terms: body_marker.clone(),
            fields,
        }),
        ..Default::default()
    };
    let in_content = task_titles(writer, &searching(TextFields::Content), "content search").await?;
    ensure!(
        in_content == vec![first.clone()],
        "a content search for a marker only one body carries returned {in_content:?}"
    );
    let in_title = task_titles(writer, &searching(TextFields::Title), "title-only search").await?;
    ensure!(
        in_title.is_empty(),
        "a title search read a marker that is in no title at all and returned {in_title:?}"
    );
    let in_either = task_titles(
        writer,
        &searching(TextFields::TitleOrContent),
        "either-field search",
    )
    .await?;
    ensure!(
        in_either == vec![first.clone()],
        "an either-field search for that same marker returned {in_either:?}"
    );

    // `projects`: a listing scoped to one project keeps the tasks filed under it and no
    // other. Unscoped on purpose — the board holds tasks of its own, so a filter declared
    // and then ignored returns them too.
    let under = |project: &NativeId| TaskQuery {
        project: ProjectFilter::Is(project.clone()),
        ..Default::default()
    };
    let under_alpha = task_titles(writer, &under(&alpha_id), "project filter").await?;
    ensure!(
        under_alpha == vec![first.clone()],
        "the tasks of one of this run's two projects came back as {under_alpha:?}"
    );
    let under_beta = task_titles(writer, &under(&beta_id), "project filter").await?;
    ensure!(
        under_beta == vec![second.clone()],
        "the tasks of the other of this run's two projects came back as {under_beta:?}"
    );

    // `orphan_tasks`: the one task filed under neither project, and neither of the two
    // filed under one.
    let orphans = task_titles(
        writer,
        &TaskQuery {
            project: ProjectFilter::Orphans,
            ..by_prefix()
        },
        "orphan selection",
    )
    .await?;
    ensure!(
        orphans == vec![orphan.clone()],
        "this run's tasks belonging to no project came back as {orphans:?}"
    );

    // `filter_by_label`: one of the three carries the label this run created, and the
    // exclusion keeps exactly the other two.
    let carrying = task_titles(
        writer,
        &TaskQuery {
            labels: LabelFilter {
                any_of: vec![label_name.clone()],
                ..Default::default()
            },
            ..by_prefix()
        },
        "label filter",
    )
    .await?;
    ensure!(
        carrying == vec![first.clone()],
        "this run's tasks carrying its own label came back as {carrying:?}"
    );
    let without = task_titles(
        writer,
        &TaskQuery {
            labels: LabelFilter {
                none_of: vec![label_name.clone()],
                ..Default::default()
            },
            ..by_prefix()
        },
        "label exclusion",
    )
    .await?;
    ensure!(
        without == sorted(vec![second.clone(), orphan.clone()]),
        "this run's tasks not carrying its own label came back as {without:?}"
    );
    let mut listed = Vec::new();
    let mut cursor = None;
    loop {
        let step = writer
            .labels(&page(cursor))
            .await
            .map_err(|error| format!("live label listing failed: {error}"))?;
        listed.extend(step.items.into_iter().map(|label| label.name));
        cursor = step.next;
        if cursor.is_none() {
            break;
        }
        ensure!(listed.len() < 10_000, "the label walk must terminate");
    }
    ensure!(
        listed.contains(&label_name),
        "the label this run attached is not in the source's own label listing"
    );

    // `filter_by_status`: two of the three sit in the board's own first column and one is
    // closed, so the normalised categories separate them.
    let todo = task_titles(
        writer,
        &TaskQuery {
            statuses: vec![StatusCategory::Todo],
            ..by_prefix()
        },
        "status filter",
    )
    .await?;
    ensure!(
        todo == sorted(vec![first.clone(), second.clone()]),
        "this run's tasks in the board's first column came back as {todo:?}"
    );
    let done = task_titles(
        writer,
        &TaskQuery {
            statuses: vec![StatusCategory::Done],
            ..by_prefix()
        },
        "status filter",
    )
    .await?;
    ensure!(
        done == vec![orphan.clone()],
        "this run's closed task came back as {done:?}"
    );

    // `task_dependencies` and `project_dependencies`, both directions each. One
    // relationship reads the same from either end: the waiting item is `from` whichever
    // connection GitHub answered from.
    let task_edge = DependencyEdge {
        from: DependencyEndpoint::from_native(second_id.clone(), ItemKind::Task),
        to: DependencyEndpoint::from_native(first_id.clone(), ItemKind::Task),
        kind: DependencyKind::Blocks,
    };
    let project_edge = DependencyEdge {
        from: DependencyEndpoint::from_native(beta_id.clone(), ItemKind::Project),
        to: DependencyEndpoint::from_native(alpha_id.clone(), ItemKind::Project),
        kind: DependencyKind::Blocks,
    };
    for (near, direction, expected, level) in [
        (&second_id, Direction::DependsOn, &task_edge, "task"),
        (&first_id, Direction::DependedOnBy, &task_edge, "task"),
        (&beta_id, Direction::DependsOn, &project_edge, "project"),
        (&alpha_id, Direction::DependedOnBy, &project_edge, "project"),
    ] {
        let read = if level == "task" {
            writer.task_dependencies(near, direction, &page(None)).await
        } else {
            writer
                .project_dependencies(near, direction, &page(None))
                .await
        }
        .map_err(|error| format!("live {level} {direction:?} dependency read failed: {error}"))?;
        ensure!(
            read.items == vec![expected.clone()],
            "the {level} {direction:?} read of {} returned {:?}",
            near.0,
            read.items
        );
    }

    // Paging: a limit smaller than the result set walks to exhaustion, reaching every row
    // exactly once and in the order one whole page reports them.
    let whole = writer
        .query_tasks(&by_prefix(), &page(None))
        .await
        .map_err(|error| format!("live whole-page read failed: {error}"))?
        .items
        .into_iter()
        .map(|task| task.title)
        .collect::<Vec<_>>();
    let mut walked = Vec::new();
    let mut cursor = None;
    loop {
        let step = writer
            .query_tasks(&by_prefix(), &PageRequest { cursor, limit: 1 })
            .await
            .map_err(|error| format!("live paged read failed: {error}"))?;
        ensure!(
            step.items.len() <= 1,
            "a page of one returned {} rows",
            step.items.len()
        );
        walked.extend(step.items.into_iter().map(|task| task.title));
        cursor = step.next;
        if cursor.is_none() {
            break;
        }
        ensure!(
            walked.len() <= 10,
            "the paged walk over this run's own three tasks must terminate"
        );
    }
    ensure!(
        walked == whole,
        "a walk in pages of one reached {walked:?} where one whole page reports {whole:?}"
    );

    // `max_page_size`: a limit above the declared ceiling is clamped rather than sent to
    // GitHub, and the ceiling is GitHub's own connection maximum rather than a guess at
    // one — the board serves a page of exactly that size and refuses one row more.
    let ceiling = onetaskgraph_github_projects::MAX_PAGE_SIZE;
    let clamped = writer
        .query_tasks(
            &by_prefix(),
            &PageRequest {
                cursor: None,
                limit: ceiling + 1,
            },
        )
        .await
        .map_err(|error| {
            format!("a limit above the declared ceiling was refused rather than clamped: {error}")
        })?;
    ensure!(
        sorted(
            clamped
                .items
                .into_iter()
                .map(|task| task.title)
                .collect::<Vec<_>>()
        ) == run_tasks,
        "a limit above the declared ceiling did not return this run's own three tasks"
    );
    board_items_page(&run.token, &run.project_id, ceiling)
        .await
        .map_err(|error| {
            format!("GitHub refused a page of the declared maximum {ceiling}: {error}")
        })?;
    if board_items_page(&run.token, &run.project_id, ceiling + 1)
        .await
        .is_ok()
    {
        return Err(format!(
            "GitHub served a page of {} board items, so {ceiling} is not its connection \
             maximum and max_page_size no longer describes it",
            ceiling + 1
        ));
    }

    // The values a copy carries: caller metadata keeps its JSON types, and the column the
    // write chose is the one the read reports.
    let written = writer
        .get_task(&first_id)
        .await
        .map_err(|error| format!("live task read-back failed: {error}"))?
        .ok_or_else(|| "the written task was not readable by its own id".to_owned())?;
    ensure!(
        written.title == first
            && written.metadata.get("live.round_trip") == Some(&json!({"nested":[1,true,null]})),
        "the live write did not round-trip its title and metadata: {written:?}"
    );
    ensure!(
        written.status.category == StatusCategory::Todo,
        "the live write filed under todo read back as {:?}",
        written.status.category
    );
    let closed_back = writer
        .get_task(&orphan_id)
        .await
        .map_err(|error| format!("live closed-task read-back failed: {error}"))?
        .ok_or_else(|| "the closed task was not readable by its own id".to_owned())?;
    ensure!(
        closed_back.status.category == StatusCategory::Done,
        "the live write filed under done read back as {:?}",
        closed_back.status.category
    );
    Ok(())
}

#[tokio::test]
#[ignore = "the live lane: run it with `just test-live onetaskgraph-github-projects`"]
async fn real_projects_v2_contract_writes_and_leaves_no_residue() {
    // llmlint: ignore-block[live_tier_compiles_and_requires_credential] This lane is non-required by decision (AGENTS.md), so an absent credential or an unnamed board skips; `ONETASKGRAPH_LIVE_REQUIRED=1` demands both.
    let lane = live_lane(
        env::var("GH_PROJECTS_TOKEN").ok().as_deref(),
        // llmlint: ignore[contracts_have_one_source_or_a_drift_gate] The live workflow spells these three names too, and the drift gate is the lane's own refusal: project.json runs test-live with ONETASKGRAPH_LIVE_REQUIRED=1, so a name spelled differently on either side fails that job naming the variable rather than skipping green.
        env::var("GH_PROJECTS_OWNER").ok().as_deref(),
        env::var("GH_PROJECTS_NUMBER").ok().as_deref(),
        env::var("GH_PROJECTS_REPOSITORY").ok().as_deref(),
        env::var("ONETASKGRAPH_LIVE_REQUIRED").ok().as_deref(),
    )
    .unwrap_or_else(|error| panic!("the GitHub Projects live lane cannot run: {error}"));
    let (token, owner, project_number, repository) = match lane {
        LiveLane::Run {
            token,
            owner,
            project_number,
            repository,
        } => (token, owner, project_number, repository),
        LiveLane::Skip(reason) => {
            eprintln!("skipped live GitHub Projects journey: {reason}");
            return;
        }
    };
    // llmlint: ignore-end[live_tier_compiles_and_requires_credential]
    verify_mutation_schema(&token)
        .await
        .unwrap_or_else(|error| panic!("GitHub mutation schema drifted: {error}"));
    // The production boundary validates the board this lane was pointed at — GitHub's owner
    // grammar and the project number's range — before either reaches GitHub.
    let source = onetaskgraph_github_projects::Plugin
        .build(
            &SourceName::new("github-live").unwrap(),
            &json!({"owner":owner,"project_number":project_number,"repository":repository}),
            &LiveSecret(token.clone().into()),
        )
        .unwrap_or_else(|error| {
            panic!("the GitHub Projects live lane cannot use this board: {error}")
        });
    let project_id = nominated_project_id(&token, &owner, project_number)
        .await
        .unwrap_or_else(|error| panic!("GitHub Projects live board lookup failed: {error}"));
    // Self-healing after an interrupted run: a process killed between its writes and its
    // cleanup never reaches `run_then_cleanup`, so the next run clears the items and the
    // label it left. What bounds where this lane may write is `live_lane`, not this sweep.
    remove_live_artifacts(&token, &project_id, &is_artifact_title)
        .await
        .unwrap_or_else(|error| {
            panic!("live residue left by an earlier interrupted run could not be cleared: {error}")
        });
    remove_artifact_labels(&token, &repository, &is_artifact_label)
        .await
        .unwrap_or_else(|error| {
            panic!(
                "live label residue from an earlier interrupted run could not be cleared: {error}"
            )
        });

    assert!(source.health().await.unwrap().reachable);
    // Every field of the contract's `Capabilities`, spelled out: the struct has no
    // `Default`, so a field added to the contract fails to compile here rather than going
    // unasserted, and the journey below drives each of these against the real board.
    assert_eq!(
        source.capabilities(),
        Capabilities {
            projects: Support::Native,
            orphan_tasks: Support::Native,
            filter_by_label: Support::Native,
            filter_by_status: Support::Native,
            search_title: Support::Native,
            search_content: Support::Native,
            task_dependencies: DependencySupport::BothDirections,
            project_dependencies: DependencySupport::BothDirections,
            max_page_size: onetaskgraph_github_projects::MAX_PAGE_SIZE,
        }
    );
    // A board is a container of projects now, so how many it holds is the board's business.
    let mut projects = Vec::new();
    let mut cursor = None;
    loop {
        let read = source
            .query_projects(&ProjectQuery::default(), &page(cursor))
            .await
            .unwrap();
        projects.extend(read.items);
        cursor = read.next;
        if cursor.is_none() {
            break;
        }
        assert!(projects.len() < 10_000, "the project walk must terminate");
    }
    if let Some(project) = projects.first() {
        assert_eq!(
            source.get_project(&project.id).await.unwrap().as_ref(),
            Some(project)
        );
    }
    assert!(
        source
            .get_project(&NativeId("not-a-real-project".into()))
            .await
            .unwrap()
            .is_none()
    );

    let mut tasks = Vec::new();
    let mut cursor = None;
    loop {
        let result = source
            .query_tasks(&TaskQuery::default(), &page(cursor))
            .await
            .unwrap();
        tasks.extend(result.items);
        cursor = result.next;
        if cursor.is_none() {
            break;
        }
        assert!(tasks.len() < 10_000, "cursor walk must terminate");
    }
    let mut ids = tasks.iter().map(|task| &task.id.0).collect::<Vec<_>>();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), tasks.len(), "cursor walk must not repeat tasks");
    assert!(tasks.iter().all(|task| matches!(
        task.status.category,
        StatusCategory::Backlog
            | StatusCategory::Todo
            | StatusCategory::InProgress
            | StatusCategory::Done
            | StatusCategory::Cancelled
            | StatusCategory::Unknown
    )));
    if let Some(task) = tasks.first() {
        assert_eq!(
            source.get_task(&task.id).await.unwrap().as_ref(),
            Some(task)
        );
    }

    let labels = source.labels(&page(None)).await.unwrap();
    let mut label_ids = labels
        .items
        .iter()
        .map(|label| &label.id.0)
        .collect::<Vec<_>>();
    label_ids.sort_unstable();
    label_ids.dedup();
    assert_eq!(label_ids.len(), labels.items.len());

    let origin_field_created = match ensure_origin_field(&token, &project_id).await {
        Ok(created) => created,
        Err(error) => {
            let cleanup = remove_live_origin_field(&token, &project_id).await;
            panic!("GitHub live origin field setup failed: {error}; cleanup result: {cleanup:?}");
        }
    };
    let status_name = match live_write_status(&token, &project_id).await {
        Ok(status) => status,
        Err(error) => {
            let cleanup = if origin_field_created {
                remove_live_origin_field(&token, &project_id).await
            } else {
                Ok(())
            };
            panic!(
                "GitHub live project cannot exercise writes: {error}; cleanup result: {cleanup:?}"
            );
        }
    };
    let run = LiveRun {
        token: token.clone(),
        repository: repository.clone(),
        project_id: project_id.clone(),
        process_id: std::process::id(),
        stamp_micros: chrono::Utc::now().timestamp_micros(),
        status_option: status_name.clone(),
    };
    let writer = onetaskgraph_github_projects::Plugin
        .build(
            &SourceName::new("github-live").unwrap(),
            &live_write_config(&owner, project_number, &repository, &status_name),
            &LiveSecret(token.clone().into()),
        )
        .unwrap_or_else(|error| panic!("the live write configuration was refused: {error}"));
    run_then_cleanup(
        || drive_every_declared_capability(&run, writer.as_ref()),
        || {
            remove_live_state(
                &token,
                &project_id,
                &repository,
                run.process_id,
                origin_field_created,
            )
        },
    )
    .await
    .unwrap_or_else(|error| panic!("GitHub live capability journey failed: {error}"));
}

#[tokio::test]
async fn cleanup_runs_after_a_successful_live_journey() {
    let cleaned = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let observed = cleaned.clone();
    assert_eq!(
        run_then_cleanup(
            || async { Ok(()) },
            || async move {
                observed.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            }
        )
        .await,
        Ok(())
    );
    assert!(cleaned.load(std::sync::atomic::Ordering::SeqCst));
}

#[tokio::test]
async fn cleanup_runs_after_a_failed_live_journey() {
    let cleaned = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let observed = cleaned.clone();
    assert_eq!(
        run_then_cleanup(
            || async { Err("injected mutation failure".to_owned()) },
            || async move {
                observed.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            }
        )
        .await,
        Err("injected mutation failure".to_owned())
    );
    assert!(cleaned.load(std::sync::atomic::Ordering::SeqCst));
}

#[test]
fn the_lane_takes_its_board_and_repository_only_from_the_names_it_is_given() {
    assert_eq!(
        live_lane(
            Some("live-token"),
            Some("nickderobertis"),
            Some("1"),
            Some("acme/work"),
            None
        ),
        Ok(LiveLane::Run {
            token: "live-token".to_owned(),
            owner: "nickderobertis".to_owned(),
            project_number: 1,
            repository: "acme/work".to_owned(),
        })
    );
}

#[test]
fn the_lanes_write_configuration_is_accepted_whatever_the_board_calls_its_first_column() {
    // The lane discovers this option name from the board at run time, so its configuration
    // has to be accepted whichever name comes back — including each name a shipped default
    // already claims, which is where pointing `unknown` at the column collided.
    for option in ["Todo", "Backlog", "In Progress", "Ready for review"] {
        onetaskgraph_github_projects::Plugin
            .build(
                &SourceName::new("github-live").unwrap(),
                &live_write_config("nickderobertis", 1, "acme/work", option),
                &LiveSecret("live-token".into()),
            )
            .unwrap_or_else(|error| {
                panic!(
                    "the live write configuration was refused for a board whose first Status \
                     option is {option:?}: {error}"
                )
            });
    }
}

#[test]
fn an_unnamed_board_skips_the_lane_and_says_which_two_names_it_needs() {
    let Ok(LiveLane::Skip(reason)) =
        live_lane(Some("live-token"), None, None, Some("acme/work"), None)
    else {
        panic!("a credential without a named board must skip rather than discover one");
    };
    assert!(
        reason.contains("GH_PROJECTS_OWNER") && reason.contains("GH_PROJECTS_NUMBER"),
        "the skip must name both variables, not just report a skip: {reason}"
    );
}

#[test]
fn an_unnamed_board_fails_the_lane_when_the_live_tier_is_required() {
    let error = live_lane(Some("live-token"), None, None, Some("acme/work"), Some("1"))
        .expect_err("ONETASKGRAPH_LIVE_REQUIRED=1 must turn the unnamed-board skip into a failure");
    assert!(
        error.contains("GH_PROJECTS_OWNER")
            && error.contains("GH_PROJECTS_NUMBER")
            && error.contains("ONETASKGRAPH_LIVE_REQUIRED"),
        "the failure must name both variables and what demanded them: {error}"
    );
}

#[test]
fn an_absent_credential_keeps_its_own_skip_or_fail_pairing() {
    let Ok(LiveLane::Skip(reason)) = live_lane(
        None,
        Some("nickderobertis"),
        Some("1"),
        Some("acme/work"),
        None,
    ) else {
        panic!("an absent credential must skip");
    };
    assert!(reason.contains("GH_PROJECTS_TOKEN"), "{reason}");
    let error = live_lane(
        None,
        Some("nickderobertis"),
        Some("1"),
        Some("acme/work"),
        Some("1"),
    )
    .expect_err("ONETASKGRAPH_LIVE_REQUIRED=1 must turn the absent-credential skip into a failure");
    assert!(error.contains("GH_PROJECTS_TOKEN"), "{error}");
    let Ok(LiveLane::Skip(empty)) = live_lane(
        Some("  "),
        Some("nickderobertis"),
        Some("1"),
        Some("acme/work"),
        None,
    ) else {
        panic!("an empty credential must skip");
    };
    assert!(empty.contains("GH_PROJECTS_TOKEN"), "{empty}");
}

#[test]
fn half_a_board_and_an_unusable_number_are_misconfigurations_rather_than_skips() {
    for (owner, number) in [
        (Some("nickderobertis"), None),
        (None, Some("1")),
        (Some("nickderobertis"), Some("0")),
        (Some("nickderobertis"), Some("not-a-number")),
        (Some("nickderobertis"), Some("4294967295")),
    ] {
        live_lane(Some("live-token"), owner, number, Some("acme/work"), None).expect_err(&format!(
            "GH_PROJECTS_OWNER={owner:?} with GH_PROJECTS_NUMBER={number:?} must fail rather than \
             skip or select a board"
        ));
    }
}

#[test]
fn an_unnamed_repository_skips_the_lane_and_a_malformed_one_is_a_misconfiguration() {
    // A lane that creates issues names the repository it creates them in, for the reason it
    // names the board: a credentialed write must not land somewhere nobody nominated.
    let Ok(LiveLane::Skip(reason)) = live_lane(
        Some("live-token"),
        Some("nickderobertis"),
        Some("1"),
        None,
        None,
    ) else {
        panic!("an unnamed repository must skip rather than discover one");
    };
    assert!(reason.contains("GH_PROJECTS_REPOSITORY"), "{reason}");
    let required = live_lane(
        Some("live-token"),
        Some("nickderobertis"),
        Some("1"),
        None,
        Some("1"),
    )
    .expect_err("ONETASKGRAPH_LIVE_REQUIRED=1 turns that skip into a failure");
    assert!(required.contains("GH_PROJECTS_REPOSITORY"), "{required}");
    for malformed in ["nameless", "acme/", "/work", "acme/work/extra"] {
        live_lane(
            Some("live-token"),
            Some("nickderobertis"),
            Some("1"),
            Some(malformed),
            None,
        )
        .expect_err(&format!("GH_PROJECTS_REPOSITORY={malformed:?} must fail"));
    }
}

#[test]
fn residue_recovery_matches_this_lanes_own_artifacts_and_nothing_else() {
    // The stale item this lane's recovery exists for: another run's process id and timestamp.
    assert!(is_artifact_title(
        "onetaskgraph live cleanup 2533-1787816134627361"
    ));
    assert!(is_artifact_title(&artifact_title(std::process::id(), 1)));
    for foreign in [
        "AI Orchestrator plan",
        "onetaskgraph live cleanup",
        "onetaskgraph live cleanup 2533",
        "onetaskgraph live cleanup abc-1787816134627361",
        "onetaskgraph live cleanup 2533-",
        "copy of onetaskgraph live cleanup 2533-1787816134627361",
    ] {
        assert!(
            !is_artifact_title(foreign),
            "residue recovery must not match {foreign:?}"
        );
    }
    // A run names its own by the process id every one of its artifacts shares, so it
    // cleans up after itself without touching what an interrupted earlier run left for
    // the sweep above.
    assert!(is_run_artifact_title(
        2533,
        "onetaskgraph live cleanup 2533-17"
    ));
    for other in [
        "onetaskgraph live cleanup 25330-17",
        "onetaskgraph live cleanup 253-17",
        "onetaskgraph live cleanup 12533-17",
    ] {
        assert!(
            !is_run_artifact_title(2533, other),
            "one run's cleanup must not match another run's {other:?}"
        );
    }
    // The label is residue exactly as an item is, and is recognised the same way.
    assert!(is_artifact_label("onetaskgraph-live-2533-1787816134627361"));
    assert!(is_artifact_label(&artifact_label(std::process::id(), 1)));
    for foreign in [
        "bug",
        "onetaskgraph-live-",
        "onetaskgraph-live-2533",
        "onetaskgraph-live-abc-1787816134627361",
        "onetaskgraph-live-2533-",
        "not-onetaskgraph-live-2533-1787816134627361",
    ] {
        assert!(
            !is_artifact_label(foreign),
            "label recovery must not match {foreign:?}"
        );
    }
}

#[test]
fn an_unreadable_live_tier_demand_is_a_misconfiguration() {
    for unusable in ["yes", "true", "2", "on"] {
        live_lane(
            Some("live-token"),
            Some("nickderobertis"),
            Some("1"),
            Some("acme/work"),
            Some(unusable),
        )
        .expect_err(&format!(
            "ONETASKGRAPH_LIVE_REQUIRED={unusable:?} must fail rather than quietly mean not-required"
        ));
    }
    assert_eq!(
        live_lane(Some("live-token"), None, None, Some("acme/work"), Some("0")),
        live_lane(Some("live-token"), None, None, Some("acme/work"), None),
        "0 and unset both mean the lane may skip"
    );
}
