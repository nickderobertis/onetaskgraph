use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
};

use onetaskgraph_plugin_api::{
    Cursor, Direction, NativeId, PageRequest, ProjectQuery, SecretResolver, SourceError,
    SourceName, SourcePlugin, StatusCategory, TaskQuery,
};
use secrecy::SecretString;
use serde_json::{Value, json};

struct Secrets;
impl SecretResolver for Secrets {
    fn get(&self, var: &str) -> Option<SecretString> {
        (var == "GH_PROJECTS_TOKEN").then(|| "test-token".into())
    }
}

fn server(
    status: &str,
    body: Value,
    requests: usize,
    expected_query: &str,
) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let body = body.to_string();
    let status = status.to_owned();
    let expected_query = expected_query.to_owned();
    let handle = thread::spawn(move || {
        for _ in 0..requests {
            let (mut stream, _) = listener.accept().unwrap();
            let mut bytes = Vec::new();
            let mut chunk = [0_u8; 4096];
            loop {
                let count = stream.read(&mut chunk).unwrap();
                bytes.extend_from_slice(&chunk[..count]);
                if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let headers = String::from_utf8_lossy(&bytes);
            assert!(headers.contains("authorization: Bearer test-token"));
            let length = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length: ")
                        .and_then(|v| v.parse::<usize>().ok())
                })
                .unwrap();
            let header_end = bytes
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .unwrap()
                + 4;
            while bytes.len() - header_end < length {
                let count = stream.read(&mut chunk).unwrap();
                bytes.extend_from_slice(&chunk[..count]);
            }
            let request: Value =
                serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap();
            assert!(request["query"].as_str().unwrap().contains(&expected_query));
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
        }
    });
    (format!("http://{address}/graphql"), handle)
}

fn raw_server(status: &str, body: &str) -> (String, thread::JoinHandle<()>) {
    raw_server_with_headers(status, body, "")
}

fn raw_server_with_headers(
    status: &str,
    body: &str,
    headers: &str,
) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let body = body.to_owned();
    let status = status.to_owned();
    let headers = headers.to_owned();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0_u8; 8192];
        let _ = stream.read(&mut request).unwrap();
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).unwrap();
    });
    (format!("http://{address}/graphql"), handle)
}

fn sequence_server(bodies: Vec<Value>) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        for body in bodies {
            let (mut stream, _) = listener.accept().unwrap();
            let mut bytes = Vec::new();
            let mut chunk = [0_u8; 4096];
            loop {
                let count = stream.read(&mut chunk).unwrap();
                bytes.extend_from_slice(&chunk[..count]);
                if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let body = body.to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
        }
    });
    (format!("http://{address}/graphql"), handle)
}

fn project_response(has_next: bool) -> Value {
    let mut fixture: Value = serde_json::from_str(include_str!("fixtures/project.json"))
        .expect("the committed project fixture is valid JSON");
    let page = &mut fixture["data"]["organization"]["projectV2"]["items"]["pageInfo"];
    page["hasNextPage"] = json!(has_next);
    page["endCursor"] = if has_next {
        json!("cursor-2")
    } else {
        Value::Null
    };
    fixture
}

fn build(endpoint: &str) -> Box<dyn onetaskgraph_plugin_api::TaskSource> {
    onetaskgraph_github_projects::Plugin.build(
        &SourceName::new("work").unwrap(),
        &json!({"owner":"octo-org","project_number":7,"endpoint":endpoint,"status_mapping":{"doing":"in-progress"}}),
        &Secrets,
    ).unwrap()
}

fn page(limit: u32) -> PageRequest {
    PageRequest {
        cursor: None,
        limit,
    }
}

#[tokio::test]
async fn reads_and_normalizes_a_synthetic_graphql_response_through_http() {
    let (endpoint, handle) = server("200 OK", project_response(true), 1, "projectV2");
    let source = build(&endpoint);
    assert_eq!(source.kind(), "github-projects");
    assert_eq!(source.capabilities().max_page_size, 100);
    let result = source
        .query_tasks(&TaskQuery::default(), &page(1))
        .await
        .unwrap();
    assert_eq!(result.next.unwrap().0, "cursor-2");
    let task = &result.items[0];
    assert_eq!(task.id.0, "I_task");
    assert_eq!(task.project.as_ref().unwrap().0, "PVT_project");
    assert_eq!(task.status.category, StatusCategory::InProgress);
    assert_eq!(task.labels.len(), 2);
    assert_eq!(
        task.created_at.unwrap().to_rfc3339(),
        "2026-01-02T00:00:00+00:00"
    );
    handle.join().unwrap();
}

#[tokio::test]
async fn maps_pull_request_and_draft_issue_content_shapes() {
    let mut pull_request = project_response(false);
    pull_request["data"]["organization"]["projectV2"]["items"]["nodes"][0]["content"]["title"] =
        json!("Review change");
    pull_request["data"]["organization"]["projectV2"]["items"]["nodes"][0]["content"]["state"] =
        json!("MERGED");

    let mut draft = project_response(false);
    let content = &mut draft["data"]["organization"]["projectV2"]["items"]["nodes"][0]["content"];
    content["id"] = json!("DI_draft");
    content["title"] = json!("Unpublished idea");
    content.as_object_mut().unwrap().remove("state");
    content.as_object_mut().unwrap().remove("labels");
    content.as_object_mut().unwrap().remove("url");

    for (response, title) in [(pull_request, "Review change"), (draft, "Unpublished idea")] {
        let (endpoint, handle) = server("200 OK", response, 1, "projectV2");
        let task = build(&endpoint)
            .query_tasks(&TaskQuery::default(), &page(10))
            .await
            .unwrap()
            .items
            .remove(0);
        assert_eq!(task.title, title);
        handle.join().unwrap();
    }
}

#[tokio::test]
async fn resolves_user_owned_projects_and_ignores_unsupported_predicates() {
    let mut response = project_response(false);
    let project = response["data"]["organization"]["projectV2"].clone();
    response["data"]["organization"]["projectV2"] = Value::Null;
    response["data"]["user"]["projectV2"] = project;
    let (endpoint, handle) = server("200 OK", response, 1, "projectV2");
    let source = build(&endpoint);
    let mut query = TaskQuery::default();
    query.labels.any_of.push("absent".into());
    query.statuses.push(StatusCategory::Cancelled);
    query.text = Some(onetaskgraph_plugin_api::TextQuery {
        terms: "absent".into(),
        fields: onetaskgraph_plugin_api::TextFields::TitleOrContent,
    });
    assert_eq!(
        source
            .query_tasks(&query, &page(10))
            .await
            .unwrap()
            .items
            .len(),
        1
    );
    handle.join().unwrap();
}

#[tokio::test]
async fn exposes_project_health_and_lookup_over_the_public_trait() {
    let (endpoint, handle) = server("200 OK", project_response(false), 3, "projectV2");
    let source = build(&endpoint);
    assert!(source.health().await.unwrap().reachable);
    let project = source
        .get_project(&NativeId("PVT_project".into()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(project.title, "Roadmap");
    assert_eq!(project.content.as_deref(), Some("Delivery plan"));
    assert!(
        source
            .get_project(&NativeId("missing".into()))
            .await
            .unwrap()
            .is_none()
    );
    handle.join().unwrap();
}

#[test]
fn config_schema_is_strict_and_build_validates_inputs_and_secret() {
    let plugin = onetaskgraph_github_projects::Plugin;
    assert_eq!(plugin.kind(), onetaskgraph_github_projects::KIND);
    let schema = serde_json::to_value(plugin.config_schema()).unwrap();
    assert_eq!(schema["additionalProperties"], false);
    assert!(schema.get("required").is_none());
    for config in [
        json!({}),
        json!({"owner":"","project_number":1}),
        json!({"owner":"-invalid","project_number":1}),
        json!({"owner":"invalid--owner","project_number":1}),
        json!({"owner":"org","project_number":0}),
        json!({"owner":"org","project_number":u32::MAX}),
        json!({"owner":"org","project_number":1,"token_env":""}),
        json!({"owner":"org","project_number":1,"token_env":"BAD-NAME"}),
        json!({"owner":"org","project_number":1,"endpoint":"not a url"}),
        json!({"owner":"org","project_number":1,"endpoint":"http://example.com"}),
        json!({"owner":"org","project_number":1,"status_mapping":{"Doing":"todo","doing":"done"}}),
        json!({"owner":"org","project_number":1,"status_mapping":{" ":"todo"}}),
        json!({"owner":"org","project_number":1,"typo":true}),
    ] {
        let result = plugin.build(&SourceName::new("work").unwrap(), &config, &Secrets);
        assert!(
            matches!(result, Err(SourceError::Config { ref message }) if message.contains("work")),
            "invalid configuration should name its source"
        );
    }
    struct Empty;
    impl SecretResolver for Empty {
        fn get(&self, _: &str) -> Option<SecretString> {
            None
        }
    }
    let result = plugin.build(
        &SourceName::new("work").unwrap(),
        &json!({"owner":"org","project_number":1}),
        &Empty,
    );
    assert!(
        matches!(result, Err(SourceError::Auth { ref message }) if message.contains("GH_PROJECTS_TOKEN") && message.contains("work"))
    );
    struct EmptyValue;
    impl SecretResolver for EmptyValue {
        fn get(&self, _: &str) -> Option<SecretString> {
            Some("".into())
        }
    }
    assert!(matches!(
        plugin.build(
            &SourceName::new("work").unwrap(),
            &json!({"owner":"org","project_number":1}),
            &EmptyValue
        ),
        Err(SourceError::Auth { .. })
    ));
    struct WhitespaceValue;
    impl SecretResolver for WhitespaceValue {
        fn get(&self, _: &str) -> Option<SecretString> {
            Some(" \t ".into())
        }
    }
    assert!(matches!(
        plugin.build(
            &SourceName::new("work").unwrap(),
            &json!({"owner":"org","project_number":1}),
            &WhitespaceValue
        ),
        Err(SourceError::Auth { .. })
    ));
}

#[tokio::test]
async fn maps_authentication_failure_without_disclosing_the_token() {
    let (endpoint, handle) = server("401 Unauthorized", json!({}), 1, "projectV2");
    let error = build(&endpoint).health().await.unwrap_err();
    let message = error.to_string();
    assert!(matches!(error, SourceError::Auth { .. }));
    assert!(!message.contains("test-token"));
    handle.join().unwrap();
}

#[tokio::test]
async fn project_dependencies_aggregate_underlying_issue_edges() {
    // llmlint: ignore[contracts_have_one_source_or_a_drift_gate] Synthetic edge data;
    // malformed variants below and the authenticated live query are its drift guards.
    let dependencies: Value = serde_json::from_str(include_str!("fixtures/dependencies.json"))
        .expect("the committed dependency fixture is valid JSON");
    let responses = vec![
        project_response(false),
        project_response(false),
        dependencies,
    ];
    let (endpoint, handle) = sequence_server(responses.clone());
    let source = build(&endpoint);
    let edges = source
        .project_dependencies(
            &NativeId("PVT_project".into()),
            Direction::DependsOn,
            &page(10),
        )
        .await
        .unwrap();
    assert_eq!(edges.items.len(), 1);
    assert_eq!(edges.items[0].from.0, "PVT_blocker");
    assert_eq!(edges.items[0].to.0, "PVT_project");
    handle.join().unwrap();
}

#[tokio::test]
async fn project_dependencies_skip_pull_requests_and_drafts() {
    let mut mixed = project_response(false);
    let issue = mixed["data"]["organization"]["projectV2"]["items"]["nodes"][0].clone();
    let mut pull_request = issue.clone();
    pull_request["content"]["id"] = json!("PR_task");
    pull_request["content"]["state"] = json!("OPEN");
    let mut draft = issue;
    draft["content"]["id"] = json!("DI_task");
    draft["content"].as_object_mut().unwrap().remove("state");
    mixed["data"]["organization"]["projectV2"]["items"]["nodes"] = json!([
        mixed["data"]["organization"]["projectV2"]["items"]["nodes"][0],
        pull_request,
        draft
    ]);
    let dependencies: Value = serde_json::from_str(include_str!("fixtures/dependencies.json"))
        .expect("the committed dependency fixture is valid JSON");
    let (endpoint, handle) = sequence_server(vec![
        project_response(false),
        mixed,
        dependencies,
        json!({"data":{"node":{}}}),
        json!({"data":{"node":{}}}),
    ]);
    let edges = build(&endpoint)
        .project_dependencies(
            &NativeId("PVT_project".into()),
            Direction::DependsOn,
            &page(10),
        )
        .await
        .unwrap();
    assert_eq!(edges.items.len(), 1);
    handle.join().unwrap();
}

#[tokio::test]
async fn project_dependencies_map_reverse_edges_and_page_them() {
    let first_dependencies = json!({"data":{"node":{
        "blockedBy":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}},
        "blocking":{"nodes":[{"id":"I_dependent_1","projectItems":{"nodes":[{"project":{"id":"PVT_dependent_1"}}]}}],
            "pageInfo":{"hasNextPage":true,"endCursor":"dependency-page-2"}}
    }}});
    let second_dependencies = json!({"data":{"node":{
        "blockedBy":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}},
        "blocking":{"nodes":[{"id":"I_dependent_2","projectItems":{"nodes":[{"project":{"id":"PVT_dependent_2"}}]}}],
            "pageInfo":{"hasNextPage":false,"endCursor":null}}
    }}});
    let responses = vec![
        project_response(false),
        project_response(false),
        first_dependencies,
        second_dependencies,
    ];
    let (endpoint, handle) = sequence_server(responses.clone());
    let first = build(&endpoint)
        .project_dependencies(
            &NativeId("PVT_project".into()),
            Direction::DependedOnBy,
            &page(1),
        )
        .await
        .unwrap();
    assert_eq!(first.items[0].from.0, "PVT_project");
    assert_eq!(first.items[0].to.0, "PVT_dependent_1");
    assert_eq!(first.next.unwrap().0, "1");
    handle.join().unwrap();

    let (endpoint, handle) = sequence_server(responses);
    let second = build(&endpoint)
        .project_dependencies(
            &NativeId("PVT_project".into()),
            Direction::DependedOnBy,
            &PageRequest {
                cursor: Some(Cursor("1".into())),
                limit: 1,
            },
        )
        .await
        .unwrap();
    assert_eq!(second.items[0].to.0, "PVT_dependent_2");
    assert!(second.next.is_none());
    handle.join().unwrap();
}

#[tokio::test]
async fn project_dependency_failures_are_explicit() {
    let source = build("http://127.0.0.1:1/graphql");
    assert!(matches!(
        source
            .project_dependencies(
                &NativeId("PVT_project".into()),
                Direction::DependedOnBy,
                &page(10)
            )
            .await,
        Err(SourceError::Unavailable { .. })
    ));
    let (endpoint, handle) = server("200 OK", project_response(false), 1, "projectV2");
    assert!(matches!(
        build(&endpoint)
            .project_dependencies(&NativeId("missing".into()), Direction::DependsOn, &page(10))
            .await,
        Err(SourceError::Refused { .. })
    ));
    handle.join().unwrap();

    let malformed_dependencies = json!({"data":{"node":{"blockedBy":{
        "nodes":[{"id":"I_blocker","projectItems":{}}],
        "pageInfo":{"hasNextPage":false,"endCursor":null}
    }}}});
    let (endpoint, handle) = sequence_server(vec![
        project_response(false),
        project_response(false),
        malformed_dependencies,
    ]);
    assert!(matches!(
        build(&endpoint)
            .project_dependencies(
                &NativeId("PVT_project".into()),
                Direction::DependsOn,
                &page(10)
            )
            .await,
        Err(SourceError::Malformed { .. })
    ));
    handle.join().unwrap();
}

#[tokio::test]
async fn walks_issue_dependencies_forward_through_graphql() {
    let response = json!({"data":{"node":{
        "blockedBy":{"nodes":[{"id":"I_blocker"}],"pageInfo":{"hasNextPage":true,"endCursor":"next"}},
        "blocking":{"nodes":[{"id":"I_dependent"}],"pageInfo":{"hasNextPage":false,"endCursor":null}}
    }}});
    let (endpoint, handle) = server("200 OK", response, 2, "blockedBy");
    let source = build(&endpoint);
    let forward = source
        .task_dependencies(&NativeId("I_task".into()), Direction::DependsOn, &page(1))
        .await
        .unwrap();
    assert_eq!(forward.items[0].from.0, "I_blocker");
    assert_eq!(forward.items[0].to.0, "I_task");
    assert_eq!(forward.next.unwrap().0, "next");
    let reverse = source
        .task_dependencies(
            &NativeId("I_task".into()),
            Direction::DependedOnBy,
            &page(1),
        )
        .await
        .unwrap();
    assert_eq!(reverse.items[0].from.0, "I_task");
    assert_eq!(reverse.items[0].to.0, "I_dependent");
    handle.join().unwrap();
}

#[tokio::test]
async fn lists_labels_tasks_and_projects_with_cursor_validation() {
    let (endpoint, handle) = server("200 OK", project_response(false), 5, "projectV2");
    let source = build(&endpoint);
    assert_eq!(
        source
            .get_task(&NativeId("I_task".into()))
            .await
            .unwrap()
            .unwrap()
            .title,
        "Ship it"
    );
    assert!(
        source
            .get_task(&NativeId("missing".into()))
            .await
            .unwrap()
            .is_none()
    );
    let labels = source.labels(&page(1)).await.unwrap();
    assert_eq!(labels.items.len(), 1);
    assert!(labels.next.is_some());
    let labels = source
        .labels(&PageRequest {
            cursor: labels.next,
            limit: 10,
        })
        .await
        .unwrap();
    assert_eq!(labels.items.len(), 1);
    let projects = source
        .query_projects(&ProjectQuery::default(), &page(10))
        .await
        .unwrap();
    assert_eq!(projects.items[0].id.0, "PVT_project");
    assert!(
        source
            .query_projects(
                &ProjectQuery::default(),
                &PageRequest {
                    cursor: Some(Cursor("done".into())),
                    limit: 10
                }
            )
            .await
            .unwrap()
            .items
            .is_empty()
    );
    handle.join().unwrap();
}

#[tokio::test]
async fn rejects_invalid_pages_cursors_and_malformed_source_shapes() {
    let source = build("http://127.0.0.1:1/graphql");
    assert!(
        source
            .labels(&PageRequest {
                cursor: Some(Cursor("not-a-number".into())),
                limit: 1
            })
            .await
            .is_err()
    );

    let malformed = json!({"data":{"organization":{"projectV2":{"id":"P","title":"x","items":{"nodes":[]}}},"user":{"projectV2":null}}});
    let (endpoint, handle) = server("200 OK", malformed, 1, "projectV2");
    assert!(
        build(&endpoint)
            .get_task(&NativeId("x".into()))
            .await
            .is_err()
    );
    handle.join().unwrap();

    for malformed in [
        json!({"data":{"organization":{"projectV2":{"id":"P","title":"x","items":{"pageInfo":{"hasNextPage":false,"endCursor":null}}}},"user":{"projectV2":null}}}),
        json!({"data":{"organization":{"projectV2":{"id":"P","title":"x","items":{"nodes":[{}],"pageInfo":{"hasNextPage":false,"endCursor":null}}}},"user":{"projectV2":null}}}),
        json!({"data":{"organization":{"projectV2":{"id":"P","title":"x","items":{"nodes":[{"content":{"id":"T","title":"missing fields"}}],"pageInfo":{"hasNextPage":false,"endCursor":null}}}},"user":{"projectV2":null}}}),
        json!({"data":{"organization":{"projectV2":{"id":"P","title":"x","items":{"nodes":[{"fieldValues":{"nodes":[]},"content":{"title":"missing id"}}],"pageInfo":{"hasNextPage":false,"endCursor":null}}}},"user":{"projectV2":null}}}),
        json!({"data":{"organization":{"projectV2":{"id":"P","title":"x","items":{"nodes":[{"fieldValues":{"nodes":[]},"content":{"id":"T","title":"bad body","body":7}}],"pageInfo":{"hasNextPage":false,"endCursor":null}}}},"user":{"projectV2":null}}}),
        json!({"data":{"organization":{"projectV2":{"id":"P","title":"x","items":{"nodes":[{"fieldValues":{"nodes":[]},"content":{"id":"T","title":"bad time","createdAt":"yesterday"}}],"pageInfo":{"hasNextPage":false,"endCursor":null}}}},"user":{"projectV2":null}}}),
        json!({"data":{"organization":{"projectV2":{"id":"P","title":"x","items":{"nodes":[{"fieldValues":{"nodes":[]},"content":{"id":"T","title":"bad label","labels":{"nodes":[{"name":"missing id"}]}}}],"pageInfo":{"hasNextPage":false,"endCursor":null}}}},"user":{"projectV2":null}}}),
        json!({"data":{"organization":{"projectV2":{"id":"P","title":"x","items":{"nodes":[{"fieldValues":{"nodes":"bad"},"content":{"id":"T","title":"bad status"}}],"pageInfo":{"hasNextPage":false,"endCursor":null}}}},"user":{"projectV2":null}}}),
        json!({"data":{"organization":{"projectV2":{"id":"P","title":"x","items":{"nodes":[{"fieldValues":{"nodes":[]},"content":{"id":"T","title":"bad labels","labels":{}}}],"pageInfo":{"hasNextPage":false,"endCursor":null}}}},"user":{"projectV2":null}}}),
        json!({"data":{"organization":{"projectV2":{"id":"P","title":"x","items":{"nodes":[],"pageInfo":{"endCursor":null}}}},"user":{"projectV2":null}}}),
        json!({"data":{"organization":{"projectV2":{"id":"P","title":"x","items":{"nodes":[],"pageInfo":null}}},"user":{"projectV2":null}}}),
    ] {
        let (endpoint, handle) = server("200 OK", malformed, 1, "projectV2");
        assert!(matches!(
            build(&endpoint)
                .query_tasks(&TaskQuery::default(), &page(10))
                .await,
            Err(SourceError::Malformed { .. })
        ));
        handle.join().unwrap();
    }

    let missing = json!({"data":{"organization":{"projectV2":null},"user":{"projectV2":null}}});
    let (endpoint, handle) = server("200 OK", missing, 1, "projectV2");
    assert!(matches!(
        build(&endpoint).health().await,
        Err(SourceError::Refused { .. })
    ));
    handle.join().unwrap();
}

#[tokio::test]
async fn walks_source_pages_for_aggregate_reads_and_accepts_a_native_cursor() {
    let (endpoint, handle) = sequence_server(vec![project_response(true), project_response(false)]);
    let labels = build(&endpoint).labels(&page(100)).await.unwrap();
    assert_eq!(labels.items.len(), 2);
    handle.join().unwrap();

    let (endpoint, handle) = server("200 OK", project_response(false), 1, "projectV2");
    let tasks = build(&endpoint)
        .query_tasks(
            &TaskQuery::default(),
            &PageRequest {
                cursor: Some(Cursor("cursor-2".into())),
                limit: 10,
            },
        )
        .await
        .unwrap();
    assert_eq!(tasks.items.len(), 1);
    handle.join().unwrap();
}

#[tokio::test]
async fn rejects_malformed_optional_project_fields() {
    for (field, value) in [
        ("shortDescription", json!(7)),
        ("url", json!([])),
        ("createdAt", json!("not-a-time")),
        ("closed", Value::Null),
    ] {
        let mut response = project_response(false);
        response["data"]["organization"]["projectV2"][field] = value;
        let (endpoint, handle) = server("200 OK", response, 1, "projectV2");
        assert!(matches!(
            build(&endpoint)
                .query_projects(&ProjectQuery::default(), &page(10))
                .await,
            Err(SourceError::Malformed { .. })
        ));
        handle.join().unwrap();
    }
}

#[tokio::test]
async fn maps_transport_http_json_and_graphql_failures() {
    for (status, body, check) in [
        ("429 Too Many Requests", "{}", "rate"),
        ("500 Internal Server Error", "{}", "unavailable"),
        ("200 OK", "not-json", "malformed"),
        ("200 OK", r#"{"errors":[{"message":"denied"}]}"#, "refused"),
        (
            "200 OK",
            r#"{"errors":[{"message":"Resource not accessible by integration scope"}]}"#,
            "auth",
        ),
        ("200 OK", r#"{"errors":[{}]}"#, "refused"),
        ("200 OK", "{}", "malformed"),
        ("200 OK", r#"{"errors":{}}"#, "malformed"),
    ] {
        let (endpoint, handle) = raw_server(status, body);
        let error = build(&endpoint).health().await.unwrap_err();
        assert!(match check {
            "rate" => matches!(error, SourceError::RateLimited { .. }),
            "unavailable" => matches!(error, SourceError::Unavailable { .. }),
            "refused" => matches!(error, SourceError::Refused { .. }),
            "auth" => matches!(error, SourceError::Auth { .. }),
            _ => matches!(error, SourceError::Malformed { .. }),
        });
        handle.join().unwrap();
    }

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/graphql", listener.local_addr().unwrap());
    drop(listener);
    assert!(matches!(
        build(&endpoint).health().await,
        Err(SourceError::Unavailable { .. })
    ));

    let (endpoint, handle) = raw_server_with_headers(
        "200 OK",
        "{}",
        "x-ratelimit-remaining: 0\r\nretry-after: 17\r\n",
    );
    assert!(matches!(
        build(&endpoint).health().await,
        Err(SourceError::RateLimited {
            retry_after_seconds: Some(17)
        })
    ));
    handle.join().unwrap();

    struct CustomSecret;
    impl SecretResolver for CustomSecret {
        fn get(&self, variable: &str) -> Option<SecretString> {
            (variable == "CUSTOM_GITHUB_TOKEN").then(|| "test-token".into())
        }
    }
    let (endpoint, handle) = raw_server(
        "200 OK",
        r#"{"errors":[{"message":"Resource not accessible by integration scope"}]}"#,
    );
    let source = onetaskgraph_github_projects::Plugin
        .build(
            &SourceName::new("work").unwrap(),
            &json!({"owner":"org","project_number":7,"endpoint":endpoint,"token_env":"CUSTOM_GITHUB_TOKEN"}),
            &CustomSecret,
        )
        .unwrap();
    let error = source.health().await.unwrap_err().to_string();
    assert!(error.contains("CUSTOM_GITHUB_TOKEN"), "{error}");
    assert!(!error.contains("GH_PROJECTS_TOKEN"), "{error}");
    handle.join().unwrap();
}

#[tokio::test]
async fn normalizes_builtin_statuses_closed_projects_and_empty_items() {
    for (name, expected) in [
        ("Backlog", StatusCategory::Backlog),
        ("Todo", StatusCategory::Todo),
        ("In Review", StatusCategory::InProgress),
        ("Done", StatusCategory::Done),
        ("Canceled", StatusCategory::Cancelled),
        ("Something Else", StatusCategory::Unknown),
    ] {
        let mut response = project_response(false);
        response["data"]["organization"]["projectV2"]["items"]["nodes"][0]["fieldValues"]["nodes"]
            [0]["name"] = json!(name);
        let (endpoint, handle) = server("200 OK", response, 1, "projectV2");
        let task = build(&endpoint)
            .query_tasks(&TaskQuery::default(), &page(10))
            .await
            .unwrap()
            .items
            .remove(0);
        assert_eq!(task.status.category, expected);
        handle.join().unwrap();
    }

    let mut response = project_response(false);
    response["data"]["organization"]["projectV2"]["closed"] = json!(true);
    response["data"]["organization"]["projectV2"]["items"]["nodes"]
        .as_array_mut()
        .unwrap()
        .push(json!({"content":null}));
    let (endpoint, handle) = server("200 OK", response, 1, "projectV2");
    let project = build(&endpoint)
        .query_projects(&ProjectQuery::default(), &page(10))
        .await
        .unwrap()
        .items
        .remove(0);
    assert_eq!(project.status.category, StatusCategory::Done);
    handle.join().unwrap();

    let mut response = project_response(false);
    response["data"]["organization"]["projectV2"]["items"]["nodes"][0]["fieldValues"] =
        json!({"nodes":[]});
    let (endpoint, handle) = server("200 OK", response, 1, "projectV2");
    let task = build(&endpoint)
        .query_tasks(&TaskQuery::default(), &page(10))
        .await
        .unwrap()
        .items
        .remove(0);
    assert_eq!(task.status.category, StatusCategory::Todo);
    assert_eq!(task.status.name, "OPEN");
    handle.join().unwrap();

    let (endpoint, handle) = server("200 OK", project_response(false), 1, "projectV2");
    let mut unsupported = ProjectQuery::default();
    unsupported.statuses.push(StatusCategory::Cancelled);
    unsupported.labels.any_of.push("absent".into());
    unsupported.text = Some(onetaskgraph_plugin_api::TextQuery {
        terms: "absent".into(),
        fields: onetaskgraph_plugin_api::TextFields::TitleOrContent,
    });
    assert_eq!(
        build(&endpoint)
            .query_projects(&unsupported, &page(10))
            .await
            .unwrap()
            .items
            .len(),
        1
    );
    handle.join().unwrap();
}

#[tokio::test]
async fn rejects_zero_pages_and_malformed_dependency_shapes() {
    let source = build("http://127.0.0.1:1/graphql");
    assert!(
        source
            .query_tasks(&TaskQuery::default(), &page(0))
            .await
            .is_err()
    );

    for response in [
        json!({"data":{"node":null}}),
        json!({"data":{"node":{}}}),
        json!({"data":{"node":{"blockedBy":{"pageInfo":{"hasNextPage":false,"endCursor":null}}}}}),
        json!({"data":{"node":{"blockedBy":{"nodes":[{}],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}}),
        json!({"data":{"node":{"blockedBy":{"nodes":[],"pageInfo":{"hasNextPage":true,"endCursor":null}}}}}),
    ] {
        let (endpoint, handle) = server("200 OK", response, 1, "blockedBy");
        assert!(
            build(&endpoint)
                .task_dependencies(&NativeId("x".into()), Direction::DependsOn, &page(1))
                .await
                .is_err()
        );
        handle.join().unwrap();
    }
}
