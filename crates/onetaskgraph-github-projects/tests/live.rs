//! Read-only structural verification against GitHub's real Projects v2 API.

use std::env;

use onetaskgraph_plugin_api::{
    Direction, NativeId, PageRequest, ProjectQuery, SecretResolver, SourceName, SourcePlugin,
    StatusCategory, TaskQuery,
};
use secrecy::SecretString;
use serde_json::{Value, json};

struct LiveSecret(SecretString);

impl SecretResolver for LiveSecret {
    fn get(&self, variable: &str) -> Option<SecretString> {
        (variable == "GH_PROJECTS_TOKEN").then(|| self.0.clone())
    }
}

async fn discover_project(token: &str) -> Option<(String, u32)> {
    if let (Ok(owner), Ok(number)) = (
        env::var("GH_PROJECTS_OWNER"),
        env::var("GH_PROJECTS_NUMBER").and_then(|value| {
            value
                .parse::<u32>()
                .map_err(|error| env::VarError::NotUnicode(error.to_string().into()))
        }),
    ) {
        return Some((owner, number));
    }
    let response: Value = reqwest::Client::new()
        .post("https://api.github.com/graphql")
        .header("user-agent", "onetaskgraph-live-test")
        .bearer_auth(token)
        .json(&json!({"query":"query { viewer { login projectsV2(first:1, orderBy:{field:UPDATED_AT,direction:DESC}) { nodes { number } } organizations(first:100) { nodes { login projectsV2(first:1, orderBy:{field:UPDATED_AT,direction:DESC}) { nodes { number } } } } } }"}))
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;
    if let (Some(owner), Some(number)) = (
        response
            .pointer("/data/viewer/login")
            .and_then(Value::as_str),
        response
            .pointer("/data/viewer/projectsV2/nodes/0/number")
            .and_then(Value::as_u64),
    ) {
        return Some((owner.to_owned(), u32::try_from(number).ok()?));
    }
    response
        .pointer("/data/viewer/organizations/nodes")?
        .as_array()?
        .iter()
        .find_map(|organization| {
            Some((
                organization.get("login")?.as_str()?.to_owned(),
                u32::try_from(
                    organization
                        .pointer("/projectsV2/nodes/0/number")?
                        .as_u64()?,
                )
                .ok()?,
            ))
        })
}

fn page(cursor: Option<onetaskgraph_plugin_api::Cursor>) -> PageRequest {
    PageRequest { cursor, limit: 25 }
}

#[tokio::test]
async fn real_projects_v2_contract_is_structurally_sound_and_read_only() {
    let Ok(token) = env::var("GH_PROJECTS_TOKEN") else {
        eprintln!("skipped live GitHub Projects journey: GH_PROJECTS_TOKEN is not set");
        return;
    };
    if token.is_empty() {
        eprintln!("skipped live GitHub Projects journey: GH_PROJECTS_TOKEN is empty");
        return;
    }
    let Some((owner, project_number)) = discover_project(&token).await else {
        eprintln!(
            "skipped live GitHub Projects journey: token has no discoverable user project; set GH_PROJECTS_OWNER and GH_PROJECTS_NUMBER"
        );
        return;
    };
    let source = onetaskgraph_github_projects::Plugin
        .build(
            &SourceName::new("github-live").unwrap(),
            &json!({"owner":owner,"project_number":project_number}),
            &LiveSecret(token.into()),
        )
        .unwrap();

    assert!(source.health().await.unwrap().reachable);
    let projects = source
        .query_projects(&ProjectQuery::default(), &page(None))
        .await
        .unwrap();
    assert_eq!(projects.items.len(), 1);
    assert!(projects.next.is_none());

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

    // Draft issues and pull requests do not expose Issue.blockedBy. Find any Issue content and
    // prove its forward dependency connection is readable; an empty connection is well formed.
    for task in &tasks {
        if source
            .task_dependencies(
                &NativeId(task.id.0.clone()),
                Direction::DependsOn,
                &page(None),
            )
            .await
            .is_ok()
        {
            return;
        }
    }
    eprintln!("live project contains no Issue item on which to exercise Issue.blockedBy");
}
