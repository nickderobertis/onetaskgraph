//! Read-only structural verification against GitHub's real Projects v2 API.

use std::env;

use onetaskgraph_plugin_api::{
    DependencySupport, Direction, NativeId, PageRequest, ProjectQuery, SecretResolver, SourceName,
    SourcePlugin, StatusCategory, Support, TaskQuery,
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
    let configured_owner = env::var("GH_PROJECTS_OWNER").ok();
    let configured_number = env::var("GH_PROJECTS_NUMBER").ok();
    if configured_owner.is_some() || configured_number.is_some() {
        let owner = configured_owner.expect("GH_PROJECTS_OWNER must accompany GH_PROJECTS_NUMBER");
        let number = configured_number
            .expect("GH_PROJECTS_NUMBER must accompany GH_PROJECTS_OWNER")
            .parse::<u32>()
            .expect("GH_PROJECTS_NUMBER must be an unsigned integer");
        if !owner.trim().is_empty() && number > 0 && number <= i32::MAX as u32 {
            return Some((owner, number));
        }
        panic!(
            "GH_PROJECTS_OWNER must be non-blank and GH_PROJECTS_NUMBER must be a positive GraphQL Int"
        );
    }
    let response: Value = reqwest::Client::new()
        .post("https://api.github.com/graphql")
        .header("user-agent", "onetaskgraph-live-test")
        .bearer_auth(token)
        .json(&json!({"query":"query { viewer { login projectsV2(first:1, orderBy:{field:UPDATED_AT,direction:DESC}) { nodes { number } } organizations(first:100) { nodes { login projectsV2(first:1, orderBy:{field:UPDATED_AT,direction:DESC}) { nodes { number } } } } } }"}))
        .send()
        .await
        .ok()?
        .error_for_status()
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
    let (owner, project_number) = discover_project(&token).await.expect(
        "GH_PROJECTS_TOKEN has no discoverable project; set GH_PROJECTS_OWNER and \
         GH_PROJECTS_NUMBER to a visible project containing at least one Issue",
    );
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
    let mut dependency_round_trip = false;
    for task in &tasks {
        let forward = source
            .task_dependencies(&task.id, Direction::DependsOn, &page(None))
            .await
            .unwrap_or_else(|error| panic!("forward dependency read failed: {error}"));
        let Some(edge) = forward.items.first() else {
            continue;
        };
        assert_eq!(edge.to, task.id);
        assert!(
            tasks.iter().any(|candidate| candidate.id == edge.from),
            "the forward dependency must resolve to another task on the project"
        );
        let reverse = source
            .task_dependencies(&edge.from, Direction::DependedOnBy, &page(None))
            .await
            .unwrap_or_else(|error| panic!("reverse dependency read failed: {error}"));
        assert!(
            reverse.items.contains(edge),
            "the blocker must name the blocked task through its reverse edge"
        );
        dependency_round_trip = true;
        break;
    }
    assert!(
        dependency_round_trip,
        "live project has no non-empty Issue.blockedBy connection whose blocker names the task \
         through Issue.blocking"
    );

    let forward_projects = source
        .project_dependencies(&project.id, Direction::DependsOn, &page(None))
        .await
        .expect("forward project dependency read failed");
    assert!(
        forward_projects
            .items
            .iter()
            .all(|edge| edge.to == project.id
                && projects.items.iter().any(|item| item.id == edge.from)),
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
            .all(|edge| edge.from == project.id
                && projects.items.iter().any(|item| item.id == edge.to)),
        "every reverse issue dependency must resolve through projectItems to a visible project"
    );
}
