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

use std::{collections::BTreeMap, env};

use onetaskgraph_plugin_api::{
    Capabilities, DependencyEdge, DependencyEndpoint, DependencyKind, DependencySupport, Direction,
    Document, DocumentQuery, ItemKind, ItemWrite, LabelFilter, NativeId, PageRequest, Project,
    ProjectFilter, ProjectQuery, SourceName, SourcePlugin, Status, StatusCategory, Support, Task,
    TaskQuery, TaskSource, TextFields, TextQuery,
};
use serde_json::{Value, json};

mod lane;

use lane::{
    ARTIFACT_PREFIX, LABEL_PREFIX, LiveLane, LiveSecret, artifact_label, artifact_title,
    is_artifact_label, is_artifact_title, is_run_artifact_title, live_lane, live_write_config,
    run_then_cleanup,
};

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
        location: None,
        created_at: None,
        updated_at: None,
        metadata: BTreeMap::new(),
        repositories: vec![],
    }
}

/// The one document this run writes.
///
/// The title given here is the title a person wrote; the source puts its own design prefix
/// in front of it on the way to the board, and takes it off again on the way back — which
/// is the round trip this leg of the lane is for.
fn artifact_document(title: &str, project: Option<NativeId>) -> Document {
    Document {
        id: NativeId("live-source-item".into()),
        title: title.to_owned(),
        content: Some("temporary credentialed write; the live lane removes this".into()),
        project,
        labels: vec![],
        url: None,
        location: None,
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
        location: None,
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

async fn document_titles(
    source: &dyn TaskSource,
    query: &DocumentQuery,
    what: &str,
) -> Result<Vec<String>, String> {
    Ok(sorted(
        source
            .query_documents(query, &page(None))
            .await
            .map_err(|error| format!("live {what} failed: {error}"))?
            .items
            .into_iter()
            .map(|document| document.title)
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
    // A source built the way `writer` was, for the legs that follow the one mutation this
    // journey makes without going through a source at all. See where it is called.
    rebuilt: &dyn Fn() -> Box<dyn TaskSource>,
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
    // That label went onto the issue through GitHub's own REST API rather than through
    // this source, and this source reads the board once for the command it is serving:
    // one invocation of the binary is one process, one source and one read of the board,
    // which is what stops a copy of a project re-reading the whole board per item it
    // writes. This journey is many commands' worth of work driven through one object, so
    // the legs below take a source built the way the next command would build one — which
    // is what makes a change nothing here wrote visible to them.
    let rebuilt = rebuilt();
    let writer = rebuilt.as_ref();

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

    // `documents`: a board has no document type, so one is an issue this source titles with
    // its own design prefix. Written here rather than beside the five above because that
    // is the discrimination worth having — this run's title search has already reported
    // exactly three tasks and two projects, so a design issue that turned up in either of
    // those listings afterwards would be the failure this leg exists to catch.
    let design = run.title(5);
    let design_id = writer
        .write_document(&ItemWrite {
            target: None,
            item: artifact_document(&design, Some(alpha_id.clone())),
            depends_on: vec![],
        })
        .await
        .map_err(|error| format!("live document write of {design:?} failed: {error}"))?;
    let by_prefix_document = || DocumentQuery {
        text: Some(TextQuery {
            terms: prefix.clone(),
            fields: TextFields::Title,
        }),
        ..Default::default()
    };
    let mut settled = false;
    for _ in 0..20 {
        if document_titles(writer, &by_prefix_document(), "document settling read").await?
            == vec![design.clone()]
        {
            settled = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    ensure!(
        settled,
        "the board never reported the document this run created ({design:?})"
    );
    let read_design = writer
        .get_document(&design_id)
        .await
        .map_err(|error| format!("live document read-back failed: {error}"))?;
    ensure!(
        read_design.as_ref().map(|held| held.title.as_str()) == Some(design.as_str()),
        "a document read back under a title other than the one written: {read_design:?}"
    );
    ensure!(
        read_design
            .as_ref()
            .and_then(|held| held.project.clone())
            .as_ref()
            == Some(&alpha_id),
        "the document this run filed under one of its projects came back in {:?}",
        read_design.as_ref().map(|held| held.project.clone())
    );
    ensure!(
        read_design
            .as_ref()
            .and_then(|held| held.location.clone())
            .is_some(),
        "every entity of a hosted board is somewhere a reader can open: {read_design:?}"
    );
    ensure!(
        read_design.as_ref().and_then(|held| held.content.clone())
            == artifact_document(&design, None).content,
        "a document read back carrying something other than the content written: \
         {read_design:?}"
    );
    // And it is a document and nothing else: the same two searches that reported three
    // tasks and two projects above report exactly the same items now.
    let tasks_after = task_titles(writer, &by_prefix(), "task read after the document").await?;
    ensure!(
        tasks_after == run_tasks,
        "a design issue turned up among this run's tasks: {tasks_after:?}"
    );
    let projects_after = project_titles(
        writer,
        &ProjectQuery {
            text: Some(TextQuery {
                terms: prefix.clone(),
                fields: TextFields::Title,
            }),
            ..Default::default()
        },
        "project read after the document",
    )
    .await?;
    ensure!(
        projects_after == run_projects,
        "a design issue turned up among this run's projects: {projects_after:?}"
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
            documents: Support::Native,
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
    let rebuild = || {
        onetaskgraph_github_projects::Plugin
            .build(
                &SourceName::new("github-live").unwrap(),
                &live_write_config(&owner, project_number, &repository, &status_name),
                &LiveSecret(token.clone().into()),
            )
            .unwrap_or_else(|error| panic!("the live write configuration was refused: {error}"))
    };
    let writer = rebuild();
    run_then_cleanup(
        || drive_every_declared_capability(&run, writer.as_ref(), &rebuild),
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
