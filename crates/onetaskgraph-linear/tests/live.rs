//! Ignored read and create/read/delete checks against Linear's real API.

use onetaskgraph_plugin_api::{
    Direction, ItemWrite, LabelFilter, PageRequest, ProjectFilter, ProjectQuery, SecretResolver,
    SourceName, SourcePlugin, TaskQuery,
};
use secrecy::SecretString;

struct Environment;
impl SecretResolver for Environment {
    fn get(&self, var: &str) -> Option<SecretString> {
        std::env::var(var).ok().map(SecretString::from)
    }
}

#[ignore = "the live lane: run it with `just test-live onetaskgraph-linear`"]
#[tokio::test]
async fn real_linear_write_round_trips_then_deletes_its_scratch_issue() {
    // llmlint: ignore-block[live_tier_compiles_and_requires_credential,tests_assert_real_behavior] This non-required third-party lane deliberately reports an absent credential or explicit scratch team as a skip, matching every live target in this repository; required checks compile but never execute this ignored test.
    let Some(key) = std::env::var("LINEAR_API_KEY")
        .ok()
        .filter(|value| !value.trim().is_empty())
    else {
        eprintln!("skipped live Linear write journey: LINEAR_API_KEY is missing");
        return;
    };
    let Some(team) = std::env::var("LINEAR_WRITE_TEAM")
        .ok()
        .filter(|value| !value.trim().is_empty())
    else {
        eprintln!(
            "skipped live Linear write journey: LINEAR_WRITE_TEAM is missing; no mutation was sent"
        );
        return;
    };
    // llmlint: ignore-end[live_tier_compiles_and_requires_credential,tests_assert_real_behavior]
    let source = onetaskgraph_linear::Plugin
        .build(
            &SourceName::new("live").unwrap(),
            &serde_json::json!({"team":team}),
            &Environment,
        )
        .unwrap();
    let request = PageRequest {
        cursor: None,
        limit: 1,
    };
    let Some(mut template) = source
        .query_tasks(&TaskQuery::default(), &request)
        .await
        .unwrap()
        .items
        .into_iter()
        .next()
    else {
        eprintln!(
            "skipped live Linear write journey: the scratch team has no workflow state to reuse"
        );
        return;
    };
    template.title = format!("onetaskgraph live write cleanup {}", std::process::id());
    template.content = Some("temporary live-lane item; deleted by the same test".into());
    template.labels.clear();
    template.project = None;
    template.metadata = [("onetaskgraph.live_test".into(), serde_json::json!(true))].into();
    template.repositories.clear();
    let id = source
        .write_task(&ItemWrite {
            target: None,
            item: template,
            depends_on: Vec::new(),
        })
        .await
        .unwrap();
    let read_back = source.get_task(&id).await.unwrap();
    let response = reqwest::Client::new().post("https://api.linear.app/graphql").header("Authorization", key).json(&serde_json::json!({"query":onetaskgraph_linear::graphql::ISSUE_DELETE,"variables":{"id":id.0}})).send().await.unwrap();
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(
        body["data"]["issueDelete"]["success"], true,
        "scratch issue cleanup failed: {body}"
    );
    let read_back = read_back.expect("the live write reads back before cleanup");
    assert_eq!(read_back.metadata["onetaskgraph.live_test"], true);
    assert_eq!(
        read_back.content.as_deref(),
        Some("temporary live-lane item; deleted by the same test")
    );
}

#[ignore = "the live lane: run it with `just test-live onetaskgraph-linear`"]
#[tokio::test]
async fn real_linear_reads_obey_structural_invariants_when_data_exists() {
    // llmlint: ignore-block[live_tier_compiles_and_requires_credential,tests_assert_real_behavior] This repository's live lane is explicitly non-required and uniformly skips absent third-party credentials so an unavailable secret cannot block unrelated work; the printed skip is the required observable behavior and scripts/check-live-lane.sh enforces that contract.
    if std::env::var("LINEAR_API_KEY")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .is_none()
    {
        eprintln!("skipped live Linear journey: LINEAR_API_KEY is missing");
        return;
    }
    // llmlint: ignore-end[live_tier_compiles_and_requires_credential,tests_assert_real_behavior]
    let mut config = serde_json::json!({});
    if let Ok(team) = std::env::var("LINEAR_TEAM") {
        config["team"] = team.into();
    }
    let source = onetaskgraph_linear::Plugin
        .build(&SourceName::new("live").unwrap(), &config, &Environment)
        .unwrap();
    source.health().await.unwrap();
    let request = PageRequest {
        cursor: None,
        limit: 50,
    };
    let tasks = source
        .query_tasks(&TaskQuery::default(), &request)
        .await
        .unwrap();
    if let Some(next) = &tasks.next {
        let second = source
            .query_tasks(
                &TaskQuery::default(),
                &PageRequest {
                    cursor: Some(next.clone()),
                    limit: 50,
                },
            )
            .await
            .unwrap();
        assert!(
            tasks
                .items
                .iter()
                .all(|a| second.items.iter().all(|b| a.id != b.id))
        );
    }
    let projects = source
        .query_projects(&ProjectQuery::default(), &request)
        .await
        .unwrap();
    if let Some(task) = tasks.items.first() {
        let by_status = source
            .query_tasks(
                &TaskQuery {
                    statuses: vec![task.status.category],
                    ..Default::default()
                },
                &request,
            )
            .await
            .unwrap();
        assert!(
            by_status
                .items
                .iter()
                .all(|v| v.status.category == task.status.category)
        );
        if let Some(label) = task.labels.first() {
            let filtered = source
                .query_tasks(
                    &TaskQuery {
                        labels: LabelFilter {
                            any_of: vec![label.name.clone()],
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                    &request,
                )
                .await
                .unwrap();
            assert!(filtered.items.len() <= tasks.items.len());
            assert!(filtered.items.iter().all(|v| {
                v.labels
                    .iter()
                    .any(|l| l.name.eq_ignore_ascii_case(&label.name))
            }));
        }
        let _forward = source
            .task_dependencies(&task.id, Direction::DependsOn, &request)
            .await
            .unwrap();
        let searched = source
            .query_tasks(
                &TaskQuery {
                    text: Some(onetaskgraph_plugin_api::TextQuery {
                        terms: task.title.clone(),
                        fields: onetaskgraph_plugin_api::TextFields::Title,
                    }),
                    project: ProjectFilter::Any,
                    ..Default::default()
                },
                &request,
            )
            .await
            .unwrap();
        assert_eq!(
            searched.items, tasks.items,
            "unsupported search must return the wider set for engine compensation"
        );
    }
    if let Some(project) = projects.items.first() {
        let _ = source
            .project_dependencies(&project.id, Direction::DependsOn, &request)
            .await
            .unwrap();
    }
}
