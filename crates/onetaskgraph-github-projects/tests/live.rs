//! Structural and residue-free write verification against GitHub's real Projects v2 API.

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

async fn ensure_metadata_field(token: &str, project_id: &str) -> Result<bool, String> {
    if writable_fields(token, project_id)
        .await?
        .iter()
        .any(|field| field.get("name").and_then(Value::as_str) == Some("onetaskgraph.metadata"))
    {
        return Ok(false);
    }
    let response = graphql_variables(
        token,
        "mutation($input:CreateProjectV2FieldInput!){createProjectV2Field(input:$input){projectV2Field{... on ProjectV2Field{id name}}}}",
        "live metadata field creation",
        json!({"input":{"projectId":project_id,"dataType":"TEXT","name":"onetaskgraph.metadata"}}),
    )
    .await?;
    if response
        .pointer("/data/createProjectV2Field/projectV2Field/name")
        .and_then(Value::as_str)
        != Some("onetaskgraph.metadata")
    {
        return Err("GitHub did not confirm creation of the live metadata field".to_owned());
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

async fn remove_live_metadata_field(token: &str, project_id: &str) -> Result<(), String> {
    for _ in 0..10 {
        let field_ids = writable_fields(token, project_id)
            .await?
            .into_iter()
            .filter(|field| {
                field.get("name").and_then(Value::as_str) == Some("onetaskgraph.metadata")
            })
            .map(|field| {
                field
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .ok_or_else(|| "live metadata field has no id".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()?;
        if field_ids.is_empty() {
            return Ok(());
        }
        for field_id in field_ids {
            graphql_variables(
                token,
                "mutation($input:DeleteProjectV2FieldInput!){deleteProjectV2Field(input:$input){projectV2Field{... on ProjectV2Field{id}}}}",
                "live metadata field cleanup",
                json!({"input":{"fieldId":field_id}}),
            )
            .await?;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    Err("live metadata field cleanup left the temporary field behind".to_owned())
}

async fn artifact_item_ids(
    token: &str,
    project_id: &str,
    title: &str,
) -> Result<Vec<String>, String> {
    let mut after = Value::Null;
    let mut matches = Vec::new();
    loop {
        let response = graphql_variables(
            token,
            "query($id:ID!,$after:String){node(id:$id){... on ProjectV2{items(first:100,after:$after){nodes{id content{... on DraftIssue{title}}}pageInfo{hasNextPage endCursor}}}}}",
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
            if node.pointer("/content/title").and_then(Value::as_str) == Some(title) {
                matches.push(
                    node.get("id")
                        .and_then(Value::as_str)
                        .ok_or_else(|| "live artifact has no project item id".to_owned())?
                        .to_owned(),
                );
            }
        }
        if connection
            .pointer("/pageInfo/hasNextPage")
            .and_then(Value::as_bool)
            != Some(true)
        {
            return Ok(matches);
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

async fn remove_live_artifacts(token: &str, project_id: &str, title: &str) -> Result<(), String> {
    for _ in 0..10 {
        let item_ids = artifact_item_ids(token, project_id, title).await?;
        if item_ids.is_empty() {
            return Ok(());
        }
        for item_id in item_ids {
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
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    Err(format!(
        "live artifact cleanup left project items: {}",
        artifact_item_ids(token, project_id, title)
            .await?
            .join(", ")
    ))
}

async fn remove_live_state(
    token: &str,
    project_id: &str,
    title: &str,
    remove_metadata_field: bool,
) -> Result<(), String> {
    let item_result = remove_live_artifacts(token, project_id, title).await;
    let field_result = if remove_metadata_field {
        remove_live_metadata_field(token, project_id).await
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

async fn discover_project(token: &str) -> Result<Option<(String, u32)>, String> {
    let configured_owner = env::var("GH_PROJECTS_OWNER").ok();
    let configured_number = env::var("GH_PROJECTS_NUMBER").ok();
    if configured_owner.is_some() || configured_number.is_some() {
        let owner = configured_owner.expect("GH_PROJECTS_OWNER must accompany GH_PROJECTS_NUMBER");
        let number = configured_number
            .expect("GH_PROJECTS_NUMBER must accompany GH_PROJECTS_OWNER")
            .parse::<u32>()
            .expect("GH_PROJECTS_NUMBER must be an unsigned integer");
        if !owner.trim().is_empty() && number > 0 && number <= i32::MAX as u32 {
            return Ok(Some((owner, number)));
        }
        panic!(
            "GH_PROJECTS_OWNER must be non-blank and GH_PROJECTS_NUMBER must be a positive GraphQL Int"
        );
    }
    let response = graphql(
        token,
        "query { viewer { login projectsV2(first:1, orderBy:{field:UPDATED_AT,direction:DESC}) { nodes { number } } } }",
        "viewer project discovery",
    )
    .await?;
    if let (Some(owner), Some(number)) = (
        response
            .pointer("/data/viewer/login")
            .and_then(Value::as_str),
        response
            .pointer("/data/viewer/projectsV2/nodes/0/number")
            .and_then(Value::as_u64),
    ) {
        return Ok(Some((
            owner.to_owned(),
            u32::try_from(number)
                .map_err(|_| "viewer project number does not fit in a u32".to_owned())?,
        )));
    }
    let response = graphql(
        token,
        "query { viewer { organizations(first:100) { nodes { login projectsV2(first:1, orderBy:{field:UPDATED_AT,direction:DESC}) { nodes { number } } } } } }",
        "organization project discovery",
    )
    .await
    .map_err(|error| {
        format!(
            "the viewer owns no visible project, and {error}; set GH_PROJECTS_OWNER and \
             GH_PROJECTS_NUMBER when organization enumeration is unavailable"
        )
    })?;
    let organizations = response
        .pointer("/data/viewer/organizations/nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            "organization project discovery returned no organizations connection".to_owned()
        })?;
    Ok(organizations.iter().find_map(|organization| {
        Some((
            organization.get("login")?.as_str()?.to_owned(),
            u32::try_from(
                organization
                    .pointer("/projectsV2/nodes/0/number")?
                    .as_u64()?,
            )
            .ok()?,
        ))
    }))
}

fn named_type(value: &Value) -> Option<&str> {
    value
        .get("name")
        .and_then(Value::as_str)
        .or_else(|| value.get("ofType").and_then(named_type))
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
        (
            "addProjectV2DraftIssue",
            "AddProjectV2DraftIssueInput",
            "AddProjectV2DraftIssuePayload",
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
            "updateProjectV2",
            "UpdateProjectV2Input",
            "UpdateProjectV2Payload",
        ),
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
            "AddProjectV2DraftIssueInput",
            true,
            &["projectId", "title", "body"][..],
        ),
        (
            "UpdateProjectV2DraftIssueInput",
            true,
            &["draftIssueId", "title", "body"][..],
        ),
        ("UpdateIssueInput", true, &["id", "title", "body"][..]),
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
            "UpdateProjectV2Input",
            true,
            &["projectId", "title", "shortDescription", "closed"][..],
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
        (
            "CreateProjectV2FieldInput",
            true,
            &["projectId", "dataType", "name"][..],
        ),
        ("DeleteProjectV2FieldInput", true, &["fieldId"][..]),
        ("AddProjectV2DraftIssuePayload", false, &["projectItem"][..]),
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
        ("UpdateProjectV2Payload", false, &["projectV2"][..]),
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
            "query TypeContract {{ __type(name:\"{type_name}\") {{ {selection} {{ name }} }} }}"
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
    }
    Ok(())
}

fn page(cursor: Option<onetaskgraph_plugin_api::Cursor>) -> PageRequest {
    PageRequest { cursor, limit: 25 }
}

#[ignore = "the live lane: run it with `just test-live onetaskgraph-github-projects`"]
#[tokio::test]
async fn real_projects_v2_contract_writes_and_leaves_no_residue() {
    // llmlint: ignore-block[live_tier_compiles_and_requires_credential] This lane is non-required by decision (AGENTS.md), so an absent credential skips; `ONETASKGRAPH_LIVE_REQUIRED=1` demands one.
    let Ok(token) = env::var("GH_PROJECTS_TOKEN") else {
        assert_ne!(
            env::var("ONETASKGRAPH_LIVE_REQUIRED").as_deref(),
            Ok("1"),
            "GH_PROJECTS_TOKEN is required by the GitHub Projects live lane"
        );
        eprintln!("skipped live GitHub Projects journey: GH_PROJECTS_TOKEN is not set");
        return;
    };
    if token.trim().is_empty() {
        assert_ne!(
            env::var("ONETASKGRAPH_LIVE_REQUIRED").as_deref(),
            Ok("1"),
            "GH_PROJECTS_TOKEN is empty in the GitHub Projects live lane"
        );
        eprintln!("skipped live GitHub Projects journey: GH_PROJECTS_TOKEN is empty");
        return;
    }
    // llmlint: ignore-end[live_tier_compiles_and_requires_credential]
    let (owner, project_number) = discover_project(&token)
        .await
        .unwrap_or_else(|error| panic!("GitHub Projects discovery failed: {error}"))
        .expect(
            "GH_PROJECTS_TOKEN can enumerate projects, but none are visible; set \
             GH_PROJECTS_OWNER and GH_PROJECTS_NUMBER to a visible project containing at least \
             one Issue",
        );
    verify_mutation_schema(&token)
        .await
        .unwrap_or_else(|error| panic!("GitHub mutation schema drifted: {error}"));
    let source = onetaskgraph_github_projects::Plugin
        .build(
            &SourceName::new("github-live").unwrap(),
            &json!({"owner":owner,"project_number":project_number}),
            &LiveSecret(token.clone().into()),
        )
        .unwrap();

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
    let projects = source
        .query_projects(&ProjectQuery::default(), &page(None))
        .await
        .unwrap();
    assert_eq!(projects.items.len(), 1);
    assert!(projects.next.is_none());
    let project = &projects.items[0];
    assert_eq!(
        source.get_project(&project.id).await.unwrap().as_ref(),
        Some(project)
    );
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

    let forward_projects = source
        .project_dependencies(&project.id, Direction::DependsOn, &page(None))
        .await
        .expect("forward project dependency read failed");
    assert!(
        forward_projects
            .items
            .iter()
            .all(|edge| edge.from == project.id
                && projects.items.iter().any(|item| edge.to == item.id)),
        "every forward issue dependency must resolve through projectItems to a visible project"
    );
    let reverse_projects = source
        .project_dependencies(&project.id, Direction::DependedOnBy, &page(None))
        .await
        .expect("reverse project dependency read failed");
    assert!(
        reverse_projects
            .items
            .iter()
            .all(|edge| edge.to == project.id
                && projects.items.iter().any(|item| edge.from == item.id)),
        "every reverse issue dependency must resolve through projectItems to a visible project"
    );

    let project_id = project.id.0.clone();
    let metadata_field_created = match ensure_metadata_field(&token, &project_id).await {
        Ok(created) => created,
        Err(error) => {
            let cleanup = remove_live_metadata_field(&token, &project_id).await;
            panic!("GitHub live metadata setup failed: {error}; cleanup result: {cleanup:?}");
        }
    };
    let status_name = match live_write_status(&token, &project_id).await {
        Ok(status) => status,
        Err(error) => {
            let cleanup = if metadata_field_created {
                remove_live_metadata_field(&token, &project_id).await
            } else {
                Ok(())
            };
            panic!(
                "GitHub live project cannot exercise writes: {error}; cleanup result: {cleanup:?}"
            );
        }
    };
    let title = format!(
        "onetaskgraph live cleanup {}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_micros()
    );
    let written_title = title.clone();
    let cleanup_title = title.clone();
    run_then_cleanup(
        || async {
            let mut metadata = BTreeMap::new();
            metadata.insert("live.round_trip".into(), json!({"nested":[1,true,null]}));
            let id = source
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
                            name: status_name,
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
                written = source
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
        || remove_live_state(&token, &project_id, &cleanup_title, metadata_field_created),
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
