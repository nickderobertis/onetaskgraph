use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::TcpListener,
    thread,
};

use onetaskgraph_plugin_api::{
    Cursor, DependencyEdge, DependencyEndpoint, DependencyKind, Direction, ItemKind, ItemWrite,
    NativeId, PageRequest, Project, ProjectQuery, Repository, SecretResolver, SourceError,
    SourceName, SourcePlugin, Status, StatusCategory, Task, TaskQuery, WriteSupport,
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
            let query = request["query"].as_str().unwrap();
            graphql_parser::parse_query::<String>(query)
                .expect("the production request must be valid GraphQL");
            assert!(query.contains(&expected_query));
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
    let page = &mut fixture["data"]["owner"]["projectV2"]["items"]["pageInfo"];
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

fn mutation_data(path: &str, id: &str) -> Value {
    let mut value = json!({});
    let mut current = &mut value;
    for part in path.split('/') {
        current = current
            .as_object_mut()
            .unwrap()
            .entry(part)
            .or_insert_with(|| json!({}));
    }
    current["id"] = json!(id);
    json!({"data":value})
}

#[tokio::test]
async fn creates_and_updates_drafts_and_updates_only_the_configured_project() {
    let mut existing = project_response(false);
    let item = &mut existing["data"]["owner"]["projectV2"]["items"]["nodes"][0];
    item["id"] = json!("PVTI-draft");
    item["content"] = json!({"__typename":"DraftIssue","id":"DRAFT-1","title":"Old","body":"Old body","createdAt":null,"updatedAt":null});
    item["fieldValues"]["nodes"][0]["name"] = json!("Doing");
    let created = json!({"data":{"addProjectV2DraftIssue":{"projectItem":{"id":"PVTI-new","content":{"id":"DRAFT-new"}}}}});
    let ok_field = mutation_data("updateProjectV2ItemFieldValue/projectV2Item", "PVTI-new");
    let (endpoint, handle) = sequence_server(vec![
        project_response(false),
        created,
        ok_field.clone(),
        ok_field.clone(),
        existing,
        mutation_data("updateProjectV2DraftIssue/draftIssue", "DRAFT-1"),
        mutation_data("updateProjectV2ItemFieldValue/projectV2Item", "PVTI-draft"),
        mutation_data("updateProjectV2ItemFieldValue/projectV2Item", "PVTI-draft"),
        project_response(false),
        mutation_data("updateProjectV2/projectV2", "PVT_project"),
    ]);
    let source = build(&endpoint);
    assert_eq!(source.writes(), WriteSupport::Supported);
    let task = Task {
        id: NativeId("source-task".into()),
        title: "New plan".into(),
        content: Some("Body".into()),
        status: Status {
            category: StatusCategory::InProgress,
            name: "Doing".into(),
        },
        labels: vec![],
        project: None,
        url: None,
        created_at: None,
        updated_at: None,
        metadata: BTreeMap::from([(
            "caller.typed".into(),
            json!({"count":2,"flags":[true,null]}),
        )]),
        repositories: vec![Repository::try_from("github.com/acme/work".to_owned()).unwrap()],
    };
    let created_id = source
        .write_task(&ItemWrite {
            target: None,
            item: task.clone(),
            depends_on: vec![],
        })
        .await
        .unwrap();
    assert_eq!(created_id.0, "DRAFT-new");
    assert_eq!(
        source
            .write_task(&ItemWrite {
                target: Some(NativeId("DRAFT-1".into())),
                item: task,
                depends_on: vec![]
            })
            .await
            .unwrap()
            .0,
        "DRAFT-1"
    );
    let project = Project {
        id: NativeId("source-project".into()),
        title: "Published".into(),
        content: Some("Plan".into()),
        status: Status {
            category: StatusCategory::Done,
            name: "Closed".into(),
        },
        labels: vec![],
        url: None,
        created_at: None,
        updated_at: None,
        metadata: BTreeMap::from([("caller.ok".into(), json!(true))]),
        repositories: vec![Repository::try_from("github.com/acme/work".to_owned()).unwrap()],
    };
    assert_eq!(
        source
            .write_project(&ItemWrite {
                target: None,
                item: project,
                depends_on: vec![DependencyEdge {
                    from: DependencyEndpoint::from_native(
                        NativeId("source-project".into()),
                        ItemKind::Project
                    ),
                    to: serde_json::from_value(json!({"id":"elsewhere:P-9","kind":"project"}))
                        .unwrap(),
                    kind: DependencyKind::Blocks,
                }]
            })
            .await
            .unwrap()
            .0,
        "PVT_project"
    );
    handle.join().unwrap();
}

#[tokio::test]
async fn updates_issue_backed_items_and_writes_native_and_fallback_dependencies() {
    let project = project_response(false);
    let (endpoint, handle) = sequence_server(vec![
        project.clone(),
        project.clone(),
        mutation_data("updateIssue/issue", "I_task"),
        project,
        json!({"data":{"addBlockedBy":{"issue":{"id":"I_task"},"blockingIssue":{"id":"I_task"}}}}),
        mutation_data("updateProjectV2ItemFieldValue/projectV2Item", "PVTI_1"),
        mutation_data("updateProjectV2ItemFieldValue/projectV2Item", "PVTI_1"),
    ]);
    let source = build(&endpoint);
    let mut task = source
        .query_tasks(&TaskQuery::default(), &page(10))
        .await
        .unwrap()
        .items
        .remove(0);
    task.title = "Revised issue".into();
    let native = DependencyEdge {
        from: DependencyEndpoint::from_native(task.id.clone(), ItemKind::Task),
        to: DependencyEndpoint::from_native(NativeId("I_task".into()), ItemKind::Task),
        kind: DependencyKind::Blocks,
    };
    let external = DependencyEdge {
        from: DependencyEndpoint::from_native(task.id.clone(), ItemKind::Task),
        to: serde_json::from_value(json!({"id":"elsewhere:T-9","kind":"task"})).unwrap(),
        kind: DependencyKind::Blocks,
    };
    assert_eq!(
        source
            .write_task(&ItemWrite {
                target: Some(task.id.clone()),
                item: task,
                depends_on: vec![native, external],
            })
            .await
            .unwrap()
            .0,
        "I_task"
    );
    handle.join().unwrap();
}

#[tokio::test]
async fn write_refusals_name_stale_targets_and_unrepresentable_fields() {
    let (endpoint, handle) = sequence_server(vec![
        project_response(false),
        project_response(false),
        project_response(false),
    ]);
    let source = build(&endpoint);
    let task = Task {
        id: NativeId("source".into()),
        title: "Title".into(),
        content: None,
        status: Status {
            category: StatusCategory::Todo,
            name: "Doing".into(),
        },
        labels: vec![],
        project: None,
        url: None,
        created_at: None,
        updated_at: None,
        metadata: BTreeMap::new(),
        repositories: vec![],
    };
    assert!(
        matches!(source.write_task(&ItemWrite { target: Some(NativeId("missing".into())), item: task, depends_on: vec![] }).await, Err(SourceError::Refused { message }) if message.contains("missing"))
    );
    let mut project = Project {
        id: NativeId("source".into()),
        title: "Title".into(),
        content: None,
        status: Status {
            category: StatusCategory::InProgress,
            name: "Open".into(),
        },
        labels: vec![],
        url: None,
        created_at: None,
        updated_at: None,
        metadata: BTreeMap::new(),
        repositories: vec![],
    };
    assert!(
        matches!(source.write_project(&ItemWrite { target: Some(NativeId("wrong".into())), item: project.clone(), depends_on: vec![] }).await, Err(SourceError::Refused { message }) if message.contains("wrong"))
    );
    project.labels.push(onetaskgraph_plugin_api::Label {
        id: NativeId("L".into()),
        name: "label".into(),
        color: None,
    });
    assert!(
        matches!(source.write_project(&ItemWrite { target: None, item: project, depends_on: vec![] }).await, Err(SourceError::Refused { message }) if message.contains("labels"))
    );
    handle.join().unwrap();
}

#[tokio::test]
async fn a_write_resolves_an_item_id_across_board_pages() {
    let mut first = project_response(true);
    first["data"]["owner"]["projectV2"]["items"]["nodes"][0]["content"]["id"] = json!("OTHER");
    let mut second = project_response(false);
    let item = &mut second["data"]["owner"]["projectV2"]["items"]["nodes"][0];
    item["id"] = json!("PVTI-draft");
    item["content"] = json!({"__typename":"DraftIssue","id":"DRAFT-1","title":"Old","body":null,"createdAt":null,"updatedAt":null});
    let (endpoint, handle) = sequence_server(vec![
        first,
        second,
        mutation_data("updateProjectV2DraftIssue/draftIssue", "DRAFT-1"),
        mutation_data("updateProjectV2ItemFieldValue/projectV2Item", "PVTI-draft"),
        mutation_data("updateProjectV2ItemFieldValue/projectV2Item", "PVTI-draft"),
        project_response(false),
        mutation_data("updateProjectV2/projectV2", "PVT_project"),
    ]);
    let source = build(&endpoint);
    let task = Task {
        id: NativeId("source".into()),
        title: "Updated".into(),
        content: None,
        status: Status {
            category: StatusCategory::InProgress,
            name: "Doing".into(),
        },
        labels: vec![],
        project: None,
        url: None,
        created_at: None,
        updated_at: None,
        metadata: BTreeMap::new(),
        repositories: vec![],
    };
    assert_eq!(
        source
            .write_task(&ItemWrite {
                target: Some(NativeId("DRAFT-1".into())),
                item: task,
                depends_on: vec![]
            })
            .await
            .unwrap()
            .0,
        "DRAFT-1"
    );
    let project = Project {
        id: NativeId("source".into()),
        title: "Empty".into(),
        content: None,
        status: Status {
            category: StatusCategory::InProgress,
            name: "Open".into(),
        },
        labels: vec![],
        url: None,
        created_at: None,
        updated_at: None,
        metadata: BTreeMap::new(),
        repositories: vec![],
    };
    source
        .write_project(&ItemWrite {
            target: None,
            item: project,
            depends_on: vec![],
        })
        .await
        .unwrap();
    handle.join().unwrap();
}

#[tokio::test]
async fn a_write_refuses_a_board_without_its_owned_field_or_status_option() {
    let mut no_metadata = project_response(false);
    no_metadata["data"]["owner"]["projectV2"]["items"]["nodes"][0]["fieldValues"]["nodes"]
        .as_array_mut()
        .unwrap()
        .retain(|value| {
            value.pointer("/field/name").and_then(Value::as_str) != Some("onetaskgraph.metadata")
        });
    let (endpoint, handle) = sequence_server(vec![no_metadata, project_response(false)]);
    let source = build(&endpoint);
    let task = |status: &str| Task {
        id: NativeId("source".into()),
        title: "Title".into(),
        content: None,
        status: Status {
            category: StatusCategory::Unknown,
            name: status.into(),
        },
        labels: vec![],
        project: None,
        url: None,
        created_at: None,
        updated_at: None,
        metadata: BTreeMap::new(),
        repositories: vec![],
    };
    assert!(
        matches!(source.write_task(&ItemWrite { target: None, item: task("Doing"), depends_on: vec![] }).await,
        Err(SourceError::Refused { message }) if message.contains("onetaskgraph.metadata"))
    );
    assert!(
        matches!(source.write_task(&ItemWrite { target: None, item: task("Impossible"), depends_on: vec![] }).await,
        Err(SourceError::Refused { message }) if message.contains("Impossible"))
    );
    handle.join().unwrap();
}

#[tokio::test]
async fn malformed_write_responses_are_refused_before_they_can_be_mistaken_for_success() {
    let mut no_page = project_response(false);
    no_page["data"]["owner"]["projectV2"]["items"]
        .as_object_mut()
        .unwrap()
        .remove("pageInfo");
    let (endpoint, handle) = sequence_server(vec![
        no_page,
        project_response(false),
        json!({"data":{"addProjectV2DraftIssue":{}}}),
    ]);
    let source = build(&endpoint);
    let task = Task {
        id: NativeId("source".into()),
        title: "Title".into(),
        content: None,
        status: Status {
            category: StatusCategory::InProgress,
            name: "Doing".into(),
        },
        labels: vec![],
        project: None,
        url: None,
        created_at: None,
        updated_at: None,
        metadata: BTreeMap::new(),
        repositories: vec![],
    };
    assert!(
        matches!(source.write_task(&ItemWrite { target: Some(NativeId("missing".into())), item: task.clone(), depends_on: vec![] }).await,
        Err(SourceError::Malformed { message }) if message.contains("pageInfo"))
    );
    assert!(
        matches!(source.write_task(&ItemWrite { target: None, item: task, depends_on: vec![] }).await,
        Err(SourceError::Malformed { message }) if message.contains("no project item"))
    );
    handle.join().unwrap();
}

#[tokio::test]
async fn write_refuses_unrepresentable_draft_and_issue_fields() {
    let mut draft = project_response(false);
    draft["data"]["owner"]["projectV2"]["items"]["nodes"][0]["content"]["__typename"] =
        json!("DraftIssue");
    let mut pull = project_response(false);
    pull["data"]["owner"]["projectV2"]["items"]["nodes"][0]["content"]["__typename"] =
        json!("PullRequest");
    let (endpoint, handle) = sequence_server(vec![
        project_response(false),
        draft,
        project_response(false),
        project_response(false),
        pull,
    ]);
    let source = build(&endpoint);
    let mut task = source_task_for_write();
    task.labels.push(onetaskgraph_plugin_api::Label {
        id: NativeId("L".into()),
        name: "label".into(),
        color: None,
    });
    assert!(
        matches!(source.write_task(&ItemWrite { target: None, item: task.clone(), depends_on: vec![] }).await, Err(SourceError::Refused { message }) if message.contains("draft") || message.contains("Draft"))
    );
    assert!(
        matches!(source.write_task(&ItemWrite { target: Some(NativeId("I_task".into())), item: task.clone(), depends_on: vec![] }).await, Err(SourceError::Refused { message }) if message.contains("draft") || message.contains("Draft"))
    );
    task.labels.clear();
    assert!(
        matches!(source.write_task(&ItemWrite { target: Some(NativeId("I_task".into())), item: task.clone(), depends_on: vec![] }).await, Err(SourceError::Refused { message }) if message.contains("labels"))
    );
    task.labels = vec![
        onetaskgraph_plugin_api::Label {
            id: NativeId("L_bug".into()),
            name: "bug".into(),
            color: Some("ff0000".into()),
        },
        onetaskgraph_plugin_api::Label {
            id: NativeId("L_field".into()),
            name: "team".into(),
            color: Some("00ff00".into()),
        },
    ];
    task.repositories = vec![Repository::try_from("github.com/other/repo".to_owned()).unwrap()];
    assert!(
        matches!(source.write_task(&ItemWrite { target: Some(NativeId("I_task".into())), item: task.clone(), depends_on: vec![] }).await, Err(SourceError::Refused { message }) if message.contains("repository"))
    );
    assert!(
        matches!(source.write_task(&ItemWrite { target: Some(NativeId("I_task".into())), item: task, depends_on: vec![] }).await, Err(SourceError::Refused { message }) if message.contains("PullRequest"))
    );
    handle.join().unwrap();
}

fn source_task_for_write() -> Task {
    Task {
        id: NativeId("source".into()),
        title: "Title".into(),
        content: None,
        status: Status {
            category: StatusCategory::InProgress,
            name: "Doing".into(),
        },
        labels: vec![],
        project: None,
        url: None,
        created_at: None,
        updated_at: None,
        metadata: BTreeMap::new(),
        repositories: vec![Repository::try_from("github.com/acme/work".to_owned()).unwrap()],
    }
}

#[tokio::test]
async fn malformed_mutation_payloads_are_rejected_at_each_write_boundary() {
    for pointer in [
        "/data/owner/projectV2/items/nodes",
        "/data/owner/projectV2/items/nodes/0/fieldValues/nodes",
    ] {
        let mut malformed = project_response(false);
        *malformed.pointer_mut(pointer).unwrap() = json!("not-an-array");
        let (endpoint, handle) = sequence_server(vec![malformed]);
        assert!(matches!(
            build(&endpoint)
                .write_task(&ItemWrite {
                    target: None,
                    item: source_task_for_write(),
                    depends_on: vec![]
                })
                .await,
            Err(SourceError::Malformed { .. })
        ));
        handle.join().unwrap();
    }
    let mut draft = project_response(false);
    draft["data"]["owner"]["projectV2"]["items"]["nodes"][0]["content"] = json!({"__typename":"DraftIssue","id":"DRAFT-1","title":"Old","body":null,"createdAt":null,"updatedAt":null});
    let (endpoint, handle) = sequence_server(vec![
        draft,
        json!({"data":{"updateProjectV2DraftIssue":{}}}),
    ]);
    assert!(matches!(
        build(&endpoint)
            .write_task(&ItemWrite {
                target: Some(NativeId("DRAFT-1".into())),
                item: source_task_for_write(),
                depends_on: vec![]
            })
            .await,
        Err(SourceError::Malformed { .. })
    ));
    handle.join().unwrap();

    let created = json!({"data":{"addProjectV2DraftIssue":{"projectItem":{"id":"PVTI-new","content":{"id":"DRAFT-new"}}}}});
    let wrong_field = mutation_data("updateProjectV2ItemFieldValue/projectV2Item", "wrong-item");
    let (endpoint, handle) = sequence_server(vec![project_response(false), created, wrong_field]);
    assert!(
        matches!(build(&endpoint).write_task(&ItemWrite { target: None, item: source_task_for_write(), depends_on: vec![] }).await, Err(SourceError::Malformed { message }) if message.contains("wrong project item"))
    );
    handle.join().unwrap();

    let created = json!({"data":{"addProjectV2DraftIssue":{"projectItem":{"id":"PVTI-new","content":{"id":"DRAFT-new"}}}}});
    let (endpoint, handle) = sequence_server(vec![
        project_response(false),
        created,
        json!({"data":{"updateProjectV2ItemFieldValue":{}}}),
    ]);
    assert!(matches!(
        build(&endpoint)
            .write_task(&ItemWrite {
                target: None,
                item: source_task_for_write(),
                depends_on: vec![]
            })
            .await,
        Err(SourceError::Malformed { .. })
    ));
    handle.join().unwrap();

    let (endpoint, handle) = sequence_server(vec![
        project_response(false),
        json!({"data":{"updateProjectV2":{}}}),
    ]);
    let project = Project {
        id: NativeId("source".into()),
        title: "Title".into(),
        content: None,
        status: Status {
            category: StatusCategory::InProgress,
            name: "Open".into(),
        },
        labels: vec![],
        url: None,
        created_at: None,
        updated_at: None,
        metadata: BTreeMap::new(),
        repositories: vec![],
    };
    assert!(matches!(
        build(&endpoint)
            .write_project(&ItemWrite {
                target: None,
                item: project,
                depends_on: vec![]
            })
            .await,
        Err(SourceError::Malformed { .. })
    ));
    handle.join().unwrap();

    let project = project_response(false);
    let (endpoint, handle) = sequence_server(vec![
        project.clone(),
        project.clone(),
        mutation_data("updateIssue/issue", "I_task"),
        project,
        json!({"data":{"addBlockedBy":{"issue":{"id":"I_task"}}}}),
    ]);
    let source = build(&endpoint);
    let mut task = source
        .query_tasks(&TaskQuery::default(), &page(10))
        .await
        .unwrap()
        .items
        .remove(0);
    task.title = "Changed".into();
    let edge = DependencyEdge {
        from: DependencyEndpoint::from_native(task.id.clone(), ItemKind::Task),
        to: DependencyEndpoint::from_native(task.id.clone(), ItemKind::Task),
        kind: DependencyKind::Blocks,
    };
    assert!(matches!(
        source
            .write_task(&ItemWrite {
                target: Some(task.id.clone()),
                item: task,
                depends_on: vec![edge]
            })
            .await,
        Err(SourceError::Malformed { .. })
    ));
    handle.join().unwrap();
}

#[tokio::test]
async fn reads_and_normalizes_a_synthetic_graphql_response_through_http() {
    let (endpoint, handle) = server(
        "200 OK",
        project_response(true),
        1,
        "... on ProjectV2SingleSelectField{id name options{id name}}",
    );
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
    assert_eq!(task.metadata["caller.number"], serde_json::json!(7));
    assert_eq!(task.repositories[0].as_str(), "github.com/acme/work");
    assert_eq!(
        task.created_at.unwrap().to_rfc3339(),
        "2026-01-02T00:00:00+00:00"
    );
    handle.join().unwrap();
}

#[tokio::test]
async fn maps_pull_request_and_draft_issue_content_shapes() {
    let mut pull_request = project_response(false);
    pull_request["data"]["owner"]["projectV2"]["items"]["nodes"][0]["content"]["title"] =
        json!("Review change");
    pull_request["data"]["owner"]["projectV2"]["items"]["nodes"][0]["content"]["state"] =
        json!("MERGED");

    let mut draft = project_response(false);
    let content = &mut draft["data"]["owner"]["projectV2"]["items"]["nodes"][0]["content"];
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
async fn resolves_a_project_v2_owner_and_ignores_unsupported_predicates() {
    let (endpoint, handle) = server(
        "200 OK",
        project_response(false),
        1,
        "owner:repositoryOwner",
    );
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
    assert_eq!(project.metadata["caller.enabled"], serde_json::json!(true));
    assert_eq!(project.repositories[0].as_str(), "github.com/acme/work");
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
    // `from` depends on `to`: the board asked about is what waits on the blocker's board.
    assert_eq!(edges.items[0].from.id(), "PVT_project");
    assert_eq!(edges.items[0].to.id(), "PVT_blocker");
    handle.join().unwrap();

    let first = json!({"data":{"node":{"__typename":"Issue","blockedBy":{"nodes":[{
        "id":"I_blocker","projectItems":{"nodes":[],"pageInfo":{"hasNextPage":true,"endCursor":"projects-2"}}
    }],"pageInfo":{"hasNextPage":false,"endCursor":null}},"blocking":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}});
    let second = json!({"data":{"node":{"projectItems":{
        "nodes":[{"project":{"id":"PVT_blocker"}}],
        "pageInfo":{"hasNextPage":false,"endCursor":null}
    }}}});
    let (endpoint, handle) = sequence_server(vec![
        project_response(false),
        project_response(false),
        first,
        second,
    ]);
    let edges = build(&endpoint)
        .project_dependencies(
            &NativeId("PVT_project".into()),
            Direction::DependsOn,
            &page(10),
        )
        .await
        .unwrap();
    assert_eq!(edges.items[0].from.id(), "PVT_project");
    assert_eq!(edges.items[0].to.id(), "PVT_blocker");
    handle.join().unwrap();
}

#[tokio::test]
async fn a_project_records_a_far_end_no_issue_relationship_can_reach() {
    // A ProjectV2 board relates to nothing directly — its edges are aggregated from its
    // issues — so a far end in another source lives in the board's own metadata slot.
    let mut project = project_response(false);
    project["data"]["owner"]["projectV2"]["shortDescription"] = json!(
        "Delivery plan\n\n<!-- onetaskgraph.metadata\n{\"onetaskgraph.depends_on\":[{\"id\":\"elsewhere:T-9\",\"kind\":\"task\"}]}\n-->"
    );
    let no_edges = json!({"data":{"node":{"__typename":"Issue",
        "blockedBy":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}},
        "blocking":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}});
    let (endpoint, handle) =
        sequence_server(vec![project.clone(), project.clone(), no_edges.clone()]);
    let edges = build(&endpoint)
        .project_dependencies(
            &NativeId("PVT_project".into()),
            Direction::DependsOn,
            &page(10),
        )
        .await
        .expect("the recorded far end is reported");
    assert_eq!(edges.items.len(), 1);
    assert_eq!(edges.items[0].from.id(), "PVT_project");
    assert_eq!(edges.items[0].from.kind, ItemKind::Project);
    assert_eq!(edges.items[0].to.id(), "elsewhere:T-9");
    assert_eq!(edges.items[0].to.kind, ItemKind::Task);
    handle.join().unwrap();

    // ...and never in reverse, which belongs to the far end.
    let (endpoint, handle) = sequence_server(vec![project.clone(), project, no_edges]);
    let reverse = build(&endpoint)
        .project_dependencies(
            &NativeId("PVT_project".into()),
            Direction::DependedOnBy,
            &page(10),
        )
        .await
        .expect("the reverse page is answered");
    assert!(reverse.items.is_empty());
    handle.join().unwrap();
}

#[tokio::test]
async fn project_dependencies_skip_pull_requests_and_drafts() {
    let mut mixed = project_response(false);
    let issue = mixed["data"]["owner"]["projectV2"]["items"]["nodes"][0].clone();
    let mut pull_request = issue.clone();
    pull_request["content"]["id"] = json!("PR_task");
    pull_request["content"]["state"] = json!("OPEN");
    let mut draft = issue;
    draft["content"]["id"] = json!("DI_task");
    draft["content"].as_object_mut().unwrap().remove("state");
    mixed["data"]["owner"]["projectV2"]["items"]["nodes"] = json!([
        mixed["data"]["owner"]["projectV2"]["items"]["nodes"][0],
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
    let first_dependencies = json!({"data":{"node":{"__typename":"Issue",
        "blockedBy":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}},
        "blocking":{"nodes":[{"id":"I_dependent_1","projectItems":{"nodes":[{"project":{"id":"PVT_dependent_1"}}],"pageInfo":{"hasNextPage":false,"endCursor":null}}}],
            "pageInfo":{"hasNextPage":true,"endCursor":"dependency-page-2"}}
    }}});
    let second_dependencies = json!({"data":{"node":{"__typename":"Issue",
        "blockedBy":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}},
        "blocking":{"nodes":[{"id":"I_dependent_2","projectItems":{"nodes":[{"project":{"id":"PVT_dependent_2"}}],"pageInfo":{"hasNextPage":false,"endCursor":null}}}],
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
    assert_eq!(first.items[0].from.id(), "PVT_dependent_1");
    assert_eq!(first.items[0].to.id(), "PVT_project");
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
    assert_eq!(second.items[0].from.id(), "PVT_dependent_2");
    assert_eq!(second.items[0].to.id(), "PVT_project");
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

    let malformed_dependencies = json!({"data":{"node":{"__typename":"Issue","blockedBy":{
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
async fn walks_issue_dependencies_in_both_directions_through_graphql() {
    let response = json!({"data":{"node":{
        "__typename":"Issue",
        "blockedBy":{"nodes":[{"id":"I_blocker"}],"pageInfo":{"hasNextPage":true,"endCursor":"next"}},
        "blocking":{"nodes":[{"id":"I_dependent"}],"pageInfo":{"hasNextPage":false,"endCursor":null}}
    }}});
    let (endpoint, handle) = server("200 OK", response, 2, "blockedBy");
    let source = build(&endpoint);
    let forward = source
        .task_dependencies(&NativeId("I_task".into()), Direction::DependsOn, &page(1))
        .await
        .unwrap();
    // `blockedBy` says what `I_task` waits on, so `I_task` is what depends.
    assert_eq!(forward.items[0].from.id(), "I_task");
    assert_eq!(forward.items[0].to.id(), "I_blocker");
    assert_eq!(forward.next.unwrap().0, "next");
    let reverse = source
        .task_dependencies(
            &NativeId("I_task".into()),
            Direction::DependedOnBy,
            &page(1),
        )
        .await
        .unwrap();
    // ...and `blocking` says what waits on it, so the dependent is `from` there.
    assert_eq!(reverse.items[0].from.id(), "I_dependent");
    assert_eq!(reverse.items[0].to.id(), "I_task");
    handle.join().unwrap();
}

#[tokio::test]
async fn non_issue_project_tasks_have_no_issue_dependencies() {
    // A pull request and a draft have no `blockedBy`, so the source falls through to the
    // reserved key, which neither of these two records anything under.
    let responses = ["PullRequest", "DraftIssue"]
        .into_iter()
        .flat_map(|kind| {
            [
                json!({"data":{"node":{"__typename":kind}}}),
                project_response(false),
            ]
        })
        .collect();
    let (endpoint, handle) = sequence_server(responses);
    let source = build(&endpoint);

    for id in ["PR_task", "DI_task"] {
        let dependencies = source
            .task_dependencies(&NativeId(id.into()), Direction::DependsOn, &page(10))
            .await
            .unwrap();
        assert!(dependencies.items.is_empty());
        assert!(dependencies.next.is_none());
    }
    handle.join().unwrap();
}

/// The project fixture with `I_task` recording `recorded` under the reserved key.
fn recording(recorded: Value) -> Value {
    let mut fixture = project_response(false);
    fixture["data"]["owner"]["projectV2"]["items"]["nodes"][0]["fieldValues"]["nodes"][1]["text"] =
        json!(serde_json::to_string(&json!({ "onetaskgraph.depends_on": recorded })).unwrap());
    fixture
}

/// Two far ends `blockedBy` cannot name: a board of this source, and another source.
fn recorded_project_response() -> Value {
    recording(json!([
        {"id":"PVT_other","kind":"project"},
        {"id":"elsewhere:P-9","kind":"project"}
    ]))
}

#[tokio::test]
async fn a_far_end_no_issue_relationship_can_name_is_read_from_the_reserved_key() {
    // `blockedBy` holds GitHub issues of this project and nothing else, so an edge to a
    // board, or into another source, has to live on the near item. It is served after the
    // native relationship is spent, and its own page walks under a cursor of its own.
    let native = json!({"data":{"node":{"__typename":"Issue","blockedBy":{
        "nodes":[{"id":"I_blocker"}],"pageInfo":{"hasNextPage":false,"endCursor":null}
    }}}});
    // Each page is one `node` read — which says whether the item is issue-backed, and so
    // which far ends its reserved key may hold — and one board scan for the metadata.
    let (endpoint, handle) = sequence_server(vec![
        native.clone(),
        recorded_project_response(),
        native.clone(),
        recorded_project_response(),
        native,
        recorded_project_response(),
    ]);
    let source = build(&endpoint);

    let first = source
        .task_dependencies(&NativeId("I_task".into()), Direction::DependsOn, &page(10))
        .await
        .expect("the native page is answered");
    assert_eq!(first.items.len(), 1);
    assert_eq!(first.items[0].from.id(), "I_task");
    assert_eq!(first.items[0].to.id(), "I_blocker");
    let next = first.next.expect("a recorded tail follows the native page");

    let tail = source
        .task_dependencies(
            &NativeId("I_task".into()),
            Direction::DependsOn,
            &PageRequest {
                limit: 1,
                cursor: Some(next),
            },
        )
        .await
        .expect("the recorded tail is answered");
    assert_eq!(tail.items.len(), 1);
    assert_eq!(tail.items[0].from.id(), "I_task");
    assert_eq!(tail.items[0].to.id(), "PVT_other");
    assert_eq!(tail.items[0].to.kind, ItemKind::Project);
    assert!(!tail.items[0].to.is_qualified());

    let last = source
        .task_dependencies(
            &NativeId("I_task".into()),
            Direction::DependsOn,
            &PageRequest {
                limit: 1,
                cursor: Some(tail.next.expect("one recorded edge is still owed")),
            },
        )
        .await
        .expect("the last recorded page is answered");
    assert_eq!(last.items[0].to.id(), "elsewhere:P-9");
    assert_eq!(last.items[0].to.kind, ItemKind::Project);
    assert!(last.next.is_none());
    handle.join().unwrap();
}

#[tokio::test]
async fn an_issue_may_not_record_a_far_end_its_own_relationship_can_name() {
    // The rule is the backend's relationship first. An issue's `blockedBy` holds issues of
    // this source, so recording one there is a plan GitHub itself would not draw — refused,
    // naming the entry and what to do with it instead. Qualifying the entry with this
    // source's own configured name changes its spelling and not where the edge belongs, so
    // that entry is refused on the same terms.
    for far in [
        json!(["I_sibling"]),
        json!([{"id":"work:I_sibling","kind":"task"}]),
    ] {
        let native = json!({"data":{"node":{"__typename":"Issue","blockedBy":{
            "nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}
        }}}});
        let (endpoint, handle) = sequence_server(vec![native, recording(far.clone())]);
        let error = build(&endpoint)
            .task_dependencies(&NativeId("I_task".into()), Direction::DependsOn, &page(10))
            .await
            .expect_err("an issue naming an issue of this source is GitHub's own edge");
        let message = format!("{error}");
        assert!(message.contains("I_sibling"), "{far}: {message}");
        assert!(message.contains("relate natively"), "{far}: {message}");
        handle.join().unwrap();
    }
}

#[tokio::test]
async fn a_recorded_cursor_is_refused_before_this_source_asks_github_anything() {
    // Both refusals are decided from the cursor alone, so this source is pointed at a port
    // nothing listens on: an answer at all would mean the request was made first. The
    // recorded tail is forward-only — the reverse of a recorded edge is derived from the
    // far end and is never written down — so a reverse read carrying its cursor is
    // resuming a walk it did not come from, and an offset that is not a number resumes
    // nothing at all.
    let source = build("http://127.0.0.1:1/graphql");
    for (direction, cursor, expected) in [
        (
            Direction::DependedOnBy,
            "onetaskgraph.depends_on:0",
            "reverse dependency read",
        ),
        (
            Direction::DependsOn,
            "onetaskgraph.depends_on:x",
            "is not a recorded-edge cursor",
        ),
    ] {
        let error = source
            .task_dependencies(
                &NativeId("I_task".into()),
                direction,
                &PageRequest {
                    cursor: Some(Cursor(cursor.to_owned())),
                    limit: 10,
                },
            )
            .await
            .expect_err("a cursor no walk of this source reported");
        let message = format!("{error}");
        assert!(message.contains(cursor), "{message}");
        assert!(message.contains(expected), "{message}");
    }
}

#[tokio::test]
async fn a_draft_may_record_the_far_end_an_issue_may_not() {
    // A draft has no `blockedBy` at all, so nothing it depends on can be named natively and
    // the reserved key is the only place any far end of its own can be.
    let draft = json!({"data":{"node":{"__typename":"DraftIssue"}}});
    let mut board = recording(json!(["I_sibling"]));
    board["data"]["owner"]["projectV2"]["items"]["nodes"][0]["content"] = json!({"id":"DI_task","title":"Sketch","body":null,
               "createdAt":"2026-01-02T00:00:00Z","updatedAt":"2026-01-03T00:00:00Z"});
    let (endpoint, handle) = sequence_server(vec![draft, board]);
    let edges = build(&endpoint)
        .task_dependencies(&NativeId("DI_task".into()), Direction::DependsOn, &page(10))
        .await
        .expect("a draft records what it cannot relate");
    assert_eq!(edges.items.len(), 1);
    assert_eq!(edges.items[0].from.id(), "DI_task");
    assert_eq!(edges.items[0].to.id(), "I_sibling");
    handle.join().unwrap();
}

#[tokio::test]
async fn a_board_may_not_record_a_far_end_its_aggregated_edges_can_name() {
    let mut board = project_response(false);
    board["data"]["owner"]["projectV2"]["shortDescription"] = json!(
        "Delivery plan\n\n<!-- onetaskgraph.metadata\n{\"onetaskgraph.depends_on\":[{\"id\":\"PVT_other\",\"kind\":\"project\"}]}\n-->"
    );
    let no_edges = json!({"data":{"node":{"__typename":"Issue",
        "blockedBy":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}},
        "blocking":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}});
    let (endpoint, handle) = sequence_server(vec![board.clone(), board, no_edges]);
    let error = build(&endpoint)
        .project_dependencies(
            &NativeId("PVT_project".into()),
            Direction::DependsOn,
            &page(10),
        )
        .await
        .expect_err("a board relates to another board through its issues");
    assert!(format!("{error}").contains("PVT_other"), "{error}");
    handle.join().unwrap();
}

#[tokio::test]
async fn a_board_may_not_record_another_board_of_this_source_by_qualified_id_either() {
    // `work:PVT_other` names a board of the very source reading it, which its aggregated
    // issue edges relate; only a board of another source belongs under the reserved key.
    let mut board = project_response(false);
    board["data"]["owner"]["projectV2"]["shortDescription"] = json!(
        "Delivery plan\n\n<!-- onetaskgraph.metadata\n{\"onetaskgraph.depends_on\":[{\"id\":\"work:PVT_other\",\"kind\":\"project\"}]}\n-->"
    );
    let no_edges = json!({"data":{"node":{"__typename":"Issue",
        "blockedBy":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}},
        "blocking":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}});
    let (endpoint, handle) = sequence_server(vec![board.clone(), board, no_edges]);
    let error = build(&endpoint)
        .project_dependencies(
            &NativeId("PVT_project".into()),
            Direction::DependsOn,
            &page(10),
        )
        .await
        .expect_err("a board of this source is not a far end this source cannot name");
    let message = format!("{error}");
    assert!(message.contains("work:PVT_other"), "{message}");
    assert!(message.contains("relate natively"), "{message}");
    handle.join().unwrap();
}

#[tokio::test]
async fn a_reserved_dependency_entry_this_interface_cannot_read_is_refused_by_name() {
    for (recorded, expected) in [
        (json!([{"id":"","kind":"task"}]), "cannot be empty"),
        (
            json!([{"id":"bad source:P-9","kind":"project"}]),
            "source name",
        ),
        (json!([{"id":"elsewhere:","kind":"project"}]), "native id"),
        (json!("elsewhere:P-9"), "not a list of dependency endpoints"),
    ] {
        let native = json!({"data":{"node":{"__typename":"Issue","blockedBy":{
            "nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}
        }}}});
        let (endpoint, handle) = sequence_server(vec![native, recording(recorded.clone())]);
        let error = build(&endpoint)
            .task_dependencies(&NativeId("I_task".into()), Direction::DependsOn, &page(10))
            .await
            .expect_err("an entry this interface cannot represent");
        let message = format!("{error}");
        assert!(message.contains(expected), "{recorded}: {message}");
        handle.join().unwrap();
    }
}

#[tokio::test]
async fn a_recorded_far_end_is_never_reported_in_reverse() {
    // The reverse of a recorded edge belongs to the far end, which this source cannot
    // reach — so it is derived from there and never written down here.
    let native = json!({"data":{"node":{"__typename":"Issue","blocking":{
        "nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}
    }}}});
    let (endpoint, handle) = sequence_server(vec![native]);
    let source = build(&endpoint);
    let reverse = source
        .task_dependencies(
            &NativeId("I_task".into()),
            Direction::DependedOnBy,
            &page(10),
        )
        .await
        .expect("the reverse page is answered");
    assert!(reverse.items.is_empty());
    assert!(reverse.next.is_none());
    handle.join().unwrap();
}

#[tokio::test]
async fn a_reserved_dependency_key_holding_the_wrong_shape_is_refused_by_name() {
    let native = json!({"data":{"node":{"__typename":"Issue","blockedBy":{
        "nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}
    }}}});
    let mut malformed = project_response(false);
    malformed["data"]["owner"]["projectV2"]["items"]["nodes"][0]["fieldValues"]["nodes"][1]["text"] =
        json!(r#"{"onetaskgraph.depends_on":{"id":"elsewhere:P-9"}}"#);
    let (endpoint, handle) = sequence_server(vec![native, malformed]);
    let source = build(&endpoint);
    let error = source
        .task_dependencies(&NativeId("I_task".into()), Direction::DependsOn, &page(10))
        .await
        .expect_err("a mapping is not a list of endpoints");
    assert!(
        format!("{error}").contains("onetaskgraph.depends_on"),
        "{error}"
    );
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
    assert!(matches!(
        source
            .query_projects(
                &ProjectQuery::default(),
                &PageRequest {
                    cursor: Some(Cursor("not-issued".into())),
                    limit: 10
                }
            )
            .await,
        Err(SourceError::Config { .. })
    ));
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

    let malformed = json!({"data":{"owner":{"projectV2":{"id":"P","title":"x","items":{"nodes":[]}}},"user":{"projectV2":null}}});
    let (endpoint, handle) = server("200 OK", malformed, 1, "projectV2");
    assert!(
        build(&endpoint)
            .get_task(&NativeId("x".into()))
            .await
            .is_err()
    );
    handle.join().unwrap();

    let mut truncated = project_response(false);
    truncated["data"]["owner"]["projectV2"]["items"]["nodes"][0]["fieldValues"]["pageInfo"]["hasNextPage"] =
        json!(true);
    let (endpoint, handle) = server("200 OK", truncated, 1, "projectV2");
    let error = build(&endpoint)
        .query_tasks(&TaskQuery::default(), &page(10))
        .await
        .unwrap_err();
    assert!(
        matches!(error, SourceError::Malformed { ref message } if message.contains("exceeds the supported nested connection size"))
    );
    handle.join().unwrap();

    for malformed in [
        json!({"data":{"owner":{"projectV2":{"id":"P","title":"x","items":{"pageInfo":{"hasNextPage":false,"endCursor":null}}}},"user":{"projectV2":null}}}),
        json!({"data":{"owner":{"projectV2":{"id":"P","title":"x","items":{"nodes":[{}],"pageInfo":{"hasNextPage":false,"endCursor":null}}}},"user":{"projectV2":null}}}),
        json!({"data":{"owner":{"projectV2":{"id":"P","title":"x","items":{"nodes":[{"content":{"id":"T","title":"missing fields"}}],"pageInfo":{"hasNextPage":false,"endCursor":null}}}},"user":{"projectV2":null}}}),
        json!({"data":{"owner":{"projectV2":{"id":"P","title":"x","items":{"nodes":[{"fieldValues":{"nodes":[],"pageInfo":{"hasNextPage":false}},"content":{"title":"missing id"}}],"pageInfo":{"hasNextPage":false,"endCursor":null}}}},"user":{"projectV2":null}}}),
        json!({"data":{"owner":{"projectV2":{"id":"P","title":"x","items":{"nodes":[{"fieldValues":{"nodes":[],"pageInfo":{"hasNextPage":false}},"content":{"id":"T","title":"bad body","body":7}}],"pageInfo":{"hasNextPage":false,"endCursor":null}}}},"user":{"projectV2":null}}}),
        json!({"data":{"owner":{"projectV2":{"id":"P","title":"x","items":{"nodes":[{"fieldValues":{"nodes":[],"pageInfo":{"hasNextPage":false}},"content":{"id":"T","title":"bad time","createdAt":"yesterday"}}],"pageInfo":{"hasNextPage":false,"endCursor":null}}}},"user":{"projectV2":null}}}),
        json!({"data":{"owner":{"projectV2":{"id":"P","title":"x","items":{"nodes":[{"fieldValues":{"nodes":[],"pageInfo":{"hasNextPage":false}},"content":{"id":"T","title":"bad label","labels":{"nodes":[{"name":"missing id"}],"pageInfo":{"hasNextPage":false}}}}],"pageInfo":{"hasNextPage":false,"endCursor":null}}}},"user":{"projectV2":null}}}),
        json!({"data":{"owner":{"projectV2":{"id":"P","title":"x","items":{"nodes":[{"fieldValues":{"nodes":"bad","pageInfo":{"hasNextPage":false}},"content":{"id":"T","title":"bad status"}}],"pageInfo":{"hasNextPage":false,"endCursor":null}}}},"user":{"projectV2":null}}}),
        json!({"data":{"owner":{"projectV2":{"id":"P","title":"x","items":{"nodes":[{"fieldValues":{"nodes":[],"pageInfo":{"hasNextPage":false}},"content":{"id":"T","title":"bad labels","labels":{}}}],"pageInfo":{"hasNextPage":false,"endCursor":null}}}},"user":{"projectV2":null}}}),
        json!({"data":{"owner":{"projectV2":{"id":"P","title":"x","items":{"nodes":[],"pageInfo":{"endCursor":null}}}},"user":{"projectV2":null}}}),
        json!({"data":{"owner":{"projectV2":{"id":"P","title":"x","items":{"nodes":[],"pageInfo":null}}},"user":{"projectV2":null}}}),
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

    let missing = json!({"data":{"owner":{"projectV2":null},"user":{"projectV2":null}}});
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

    let (endpoint, handle) = sequence_server(vec![project_response(true), project_response(true)]);
    assert!(matches!(
        build(&endpoint).labels(&page(100)).await,
        Err(SourceError::Malformed { .. })
    ));
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

    let (endpoint, handle) = server("200 OK", project_response(true), 1, "projectV2");
    assert!(matches!(
        build(&endpoint)
            .query_tasks(
                &TaskQuery::default(),
                &PageRequest {
                    cursor: Some(Cursor("cursor-2".into())),
                    limit: 10,
                },
            )
            .await,
        Err(SourceError::Malformed { .. })
    ));
    handle.join().unwrap();

    let repeated_dependency_cursor = json!({"data":{"node":{"__typename":"Issue","blockedBy":{"nodes":[],"pageInfo":{"hasNextPage":true,"endCursor":"same"}},"blocking":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}});
    let (endpoint, handle) = server("200 OK", repeated_dependency_cursor, 1, "blockedBy");
    assert!(matches!(
        build(&endpoint)
            .task_dependencies(
                &NativeId("I_task".into()),
                Direction::DependsOn,
                &PageRequest {
                    cursor: Some(Cursor("same".into())),
                    limit: 10,
                },
            )
            .await,
        Err(SourceError::Malformed { .. })
    ));
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
        response["data"]["owner"]["projectV2"][field] = value;
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
async fn metadata_slots_are_validated_and_absence_remains_backward_compatible() {
    for value in [json!(7), json!("{bad json")] {
        let mut response = project_response(false);
        response["data"]["owner"]["projectV2"]["items"]["nodes"][0]["fieldValues"]["nodes"][1]["text"] =
            value;
        let (endpoint, handle) = server("200 OK", response, 1, "projectV2");
        assert!(matches!(
            build(&endpoint)
                .query_tasks(&TaskQuery::default(), &page(10))
                .await,
            Err(SourceError::Malformed { .. })
        ));
        handle.join().unwrap();
    }

    for description in [Value::Null, json!("ordinary description")] {
        let mut response = project_response(false);
        response["data"]["owner"]["projectV2"]["shortDescription"] = description;
        let (endpoint, handle) = server("200 OK", response, 1, "projectV2");
        assert!(
            build(&endpoint)
                .query_projects(&ProjectQuery::default(), &page(10))
                .await
                .is_ok()
        );
        handle.join().unwrap();
    }

    let description = "visible\n<!-- onetaskgraph.metadata\n{}\n-->\ntrailing content";
    let mut response = project_response(false);
    response["data"]["owner"]["projectV2"]["shortDescription"] = json!(description);
    let (endpoint, handle) = server("200 OK", response, 1, "projectV2");
    let projects = build(&endpoint)
        .query_projects(&ProjectQuery::default(), &page(10))
        .await
        .expect("a non-trailing marker is visible content, not a reserved slot");
    assert_eq!(projects.items[0].content.as_deref(), Some(description));
    assert!(projects.items[0].metadata.is_empty());
    handle.join().unwrap();

    for description in [
        json!("visible\n<!-- onetaskgraph.metadata\n{}"),
        json!("visible\n<!-- onetaskgraph.metadata\n{bad}\n-->"),
    ] {
        let mut response = project_response(false);
        response["data"]["owner"]["projectV2"]["shortDescription"] = description;
        let (endpoint, handle) = server("200 OK", response, 1, "projectV2");
        assert!(matches!(
            build(&endpoint)
                .query_projects(&ProjectQuery::default(), &page(10))
                .await,
            Err(SourceError::Malformed { .. })
        ));
        handle.join().unwrap();
    }

    for (repositories, succeeds) in [
        (json!(["github.com/acme/fallback"]), true),
        (json!(7), false),
    ] {
        let mut response = project_response(false);
        response["data"]["owner"]["projectV2"]["items"]["nodes"][0]["content"]
            .as_object_mut()
            .unwrap()
            .remove("repository");
        response["data"]["owner"]["projectV2"]["items"]["nodes"][0]["fieldValues"]["nodes"][1]["text"] = json!(
            serde_json::to_string(&json!({"onetaskgraph.repositories": repositories})).unwrap()
        );
        let (endpoint, handle) = server("200 OK", response, 1, "projectV2");
        assert_eq!(
            build(&endpoint)
                .query_tasks(&TaskQuery::default(), &page(10))
                .await
                .is_ok(),
            succeeds
        );
        handle.join().unwrap();
    }

    let mut response = project_response(false);
    response["data"]["owner"]["projectV2"]["shortDescription"] =
        json!("visible\n<!-- onetaskgraph.metadata\n{\"onetaskgraph.repositories\":7}\n-->");
    let (endpoint, handle) = server("200 OK", response, 1, "projectV2");
    assert!(matches!(
        build(&endpoint)
            .query_projects(&ProjectQuery::default(), &page(10))
            .await,
        Err(SourceError::Malformed { .. })
    ));
    handle.join().unwrap();
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
        response["data"]["owner"]["projectV2"]["items"]["nodes"][0]["fieldValues"]["nodes"][0]["name"] =
            json!(name);
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
    response["data"]["owner"]["projectV2"]["closed"] = json!(true);
    response["data"]["owner"]["projectV2"]["items"]["nodes"]
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
    response["data"]["owner"]["projectV2"]["items"]["nodes"][0]["fieldValues"] =
        json!({"nodes":[],"pageInfo":{"hasNextPage":false}});
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
        json!({"data":{"node":{"__typename":"Issue","blockedBy":{"pageInfo":{"hasNextPage":false,"endCursor":null}}}}}),
        json!({"data":{"node":{"__typename":"Issue","blockedBy":{"nodes":[{}],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}}),
        json!({"data":{"node":{"__typename":"Issue","blockedBy":{"nodes":[],"pageInfo":{"hasNextPage":true,"endCursor":null}}}}}),
        json!({"data":{"node":{"__typename":"Issue","blockedBy":{"nodes":[],"pageInfo":{"hasNextPage":true,"endCursor":""}}}}}),
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
