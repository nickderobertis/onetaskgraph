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
    DependencySupport, Direction, ItemWrite, NativeId, PageRequest, ProjectQuery, SecretResolver,
    SourceName, SourcePlugin, Status, StatusCategory, Support, Task, TaskQuery,
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

async fn remove_live_state(
    token: &str,
    project_id: &str,
    title: &str,
    remove_origin_field: bool,
) -> Result<(), String> {
    let item_result =
        remove_live_artifacts(token, project_id, &|candidate| candidate == title).await;
    let field_result = if remove_origin_field {
        remove_live_origin_field(token, project_id).await
    } else {
        Ok(())
    };
    match (item_result, field_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(item), Ok(())) => Err(item),
        (Ok(()), Err(field)) => Err(field),
        (Err(item), Err(field)) => Err(format!("{item}; additionally, {field}")),
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
    // Self-healing after an interrupted run: a process killed between its write and its cleanup
    // never reaches `run_then_cleanup`, so the next run clears what it left. What bounds where
    // this lane may write is `live_lane`, not this sweep.
    remove_live_artifacts(&token, &project_id, &is_artifact_title)
        .await
        .unwrap_or_else(|error| {
            panic!("live residue left by an earlier interrupted run could not be cleared: {error}")
        });

    assert!(source.health().await.unwrap().reachable);
    let capabilities = source.capabilities();
    assert_eq!(capabilities.projects, Support::Native);
    assert_eq!(capabilities.filter_by_label, Support::Unsupported);
    assert_eq!(capabilities.filter_by_status, Support::Unsupported);
    assert_eq!(capabilities.search_title, Support::Unsupported);
    assert_eq!(capabilities.search_content, Support::Unsupported);
    assert_eq!(
        capabilities.task_dependencies,
        DependencySupport::BothDirections
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

    // GitHub cannot push these predicates into ProjectV2.items. The source must return the
    // same wider page so the engine, which is covered by deterministic subprocess journeys,
    // can apply label, status, and search predicates locally.
    let mut unsupported = TaskQuery::default();
    unsupported.labels.any_of.push("unlikely-live-label".into());
    unsupported.statuses.push(StatusCategory::Done);
    unsupported.text = Some(onetaskgraph_plugin_api::TextQuery {
        terms: "unlikely-live-search".into(),
        fields: onetaskgraph_plugin_api::TextFields::TitleOrContent,
    });
    let wider = source.query_tasks(&unsupported, &page(None)).await.unwrap();
    let baseline = source
        .query_tasks(&TaskQuery::default(), &page(None))
        .await
        .unwrap();
    assert_eq!(wider.items, baseline.items);

    // Find an Issue with a real blocked-by edge, then follow the blocker back through its
    // blocking connection. Draft issues and pull requests simply contribute no Issue edges.
    //
    // Every edge is oriented from the item that depends, whichever connection reported it:
    // `blockedBy` and `blocking` are one relationship read from either end, so the forward
    // read names the waiting task as `from` and the reverse read of the blocker returns
    // that same edge rather than its mirror.
    let mut dependency_round_trip = false;
    for task in &tasks {
        let forward = source
            .task_dependencies(&task.id, Direction::DependsOn, &page(None))
            .await
            .unwrap_or_else(|error| panic!("forward dependency read failed: {error}"));
        let Some(edge) = forward.items.first() else {
            continue;
        };
        assert_eq!(
            edge.from, task.id,
            "a forward edge is reported from the item that depends"
        );
        assert!(
            tasks.iter().any(|candidate| edge.to == candidate.id),
            "the forward dependency must resolve to another task on the project"
        );
        let reverse = source
            .task_dependencies(
                &NativeId(edge.to.id().to_owned()),
                Direction::DependedOnBy,
                &page(None),
            )
            .await
            .unwrap_or_else(|error| panic!("reverse dependency read failed: {error}"));
        assert!(
            reverse.items.contains(edge),
            "the blocker must name the blocked task through its reverse edge"
        );
        dependency_round_trip = true;
        break;
    }
    // Whether any board item is blocked at all is the board's business and it changes
    // between runs, so an empty graph is reported rather than failed: this lane says
    // whether the product read the board correctly, not what somebody put on it.
    if !dependency_round_trip {
        eprintln!(
            "live GitHub Projects journey exercised no task dependency: no item on this board \
             has a non-empty Issue.blockedBy connection"
        );
    }

    for project in &projects {
        let forward_projects = source
            .project_dependencies(&project.id, Direction::DependsOn, &page(None))
            .await
            .expect("forward project dependency read failed");
        assert!(
            forward_projects
                .items
                .iter()
                .all(|edge| edge.from == project.id),
            "a forward project edge is reported from the project that depends"
        );
        let reverse_projects = source
            .project_dependencies(&project.id, Direction::DependedOnBy, &page(None))
            .await
            .expect("reverse project dependency read failed");
        assert!(
            reverse_projects
                .items
                .iter()
                .all(|edge| edge.to == project.id),
            "a reverse project edge is reported at the project that is depended on"
        );
    }

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
    // Writes go by status *category*, and this board's own first Status option is the only
    // column this lane knows exists, so `unknown` is pointed at it for this instance alone.
    let writer = onetaskgraph_github_projects::Plugin
        .build(
            &SourceName::new("github-live").unwrap(),
            &json!({"owner":owner,"project_number":project_number,"repository":repository,
                    "status_mapping":{"unknown":status_name}}),
            &LiveSecret(token.clone().into()),
        )
        .unwrap_or_else(|error| panic!("the live write configuration was refused: {error}"));
    let title = artifact_title(std::process::id(), chrono::Utc::now().timestamp_micros());
    let written_title = title.clone();
    let cleanup_title = title.clone();
    run_then_cleanup(
        || async {
            let mut metadata = BTreeMap::new();
            metadata.insert("live.round_trip".into(), json!({"nested":[1,true,null]}));
            let id = writer
                .write_task(&ItemWrite {
                    target: None,
                    item: Task {
                        id: NativeId("live-source-item".into()),
                        title: written_title.clone(),
                        content: Some(
                            "temporary credentialed write; the live lane removes this".into(),
                        ),
                        status: Status {
                            category: StatusCategory::Unknown,
                            name: "unknown".into(),
                        },
                        labels: vec![],
                        project: None,
                        url: None,
                        created_at: None,
                        updated_at: None,
                        metadata,
                        repositories: vec![],
                    },
                    depends_on: vec![],
                })
                .await
                .map_err(|error| format!("live GitHub write failed: {error}"))?;
            let mut written = None;
            for _ in 0..10 {
                written = writer
                    .get_task(&id)
                    .await
                    .map_err(|error| format!("live GitHub write read-back failed: {error}"))?;
                if written.is_some() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
            let written = written
                .ok_or_else(|| "live GitHub write was not readable after creation".to_owned())?;
            if written.title != written_title
                || written.metadata.get("live.round_trip") != Some(&json!({"nested":[1,true,null]}))
            {
                return Err("live GitHub write did not round-trip title and metadata".to_owned());
            }
            Ok(())
        },
        || remove_live_state(&token, &project_id, &cleanup_title, origin_field_created),
    )
    .await
    .unwrap_or_else(|error| panic!("GitHub live write journey failed: {error}"));
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
fn the_lane_takes_its_board_only_from_the_two_names() {
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
