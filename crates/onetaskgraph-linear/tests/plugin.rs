//! Public factory and real-HTTP fixture journeys.

use onetaskgraph_plugin_api::{
    Direction, LabelFilter, PageRequest, ProjectFilter, ProjectQuery, SecretResolver, SourceError,
    SourceName, SourcePlugin, StatusCategory, TaskQuery, TaskSource,
};
use secrecy::SecretString;
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::mpsc,
    thread,
};

struct Secrets(Option<SecretString>);
impl SecretResolver for Secrets {
    fn get(&self, _: &str) -> Option<SecretString> {
        self.0.clone()
    }
}

struct NamedSecrets;
impl SecretResolver for NamedSecrets {
    fn get(&self, name: &str) -> Option<SecretString> {
        (name == "CUSTOM_OTG_TOKEN").then(|| "named-key".into())
    }
}

fn source(endpoint: &str) -> Box<dyn TaskSource> {
    onetaskgraph_linear::Plugin
        .build(
            &SourceName::new("work").unwrap(),
            &serde_json::json!({"endpoint":endpoint}),
            &Secrets(Some(SecretString::from("fixture-key"))),
        )
        .unwrap()
}

fn server(
    status: &'static str,
    headers: &'static str,
    body: impl Into<String>,
) -> (String, mpsc::Receiver<String>) {
    let body = body.into();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut bytes = vec![0; 65536];
        let n = stream.read(&mut bytes).unwrap();
        bytes.truncate(n);
        let _ = tx.send(String::from_utf8_lossy(&bytes).into_owned());
        write!(stream,"HTTP/1.1 {status}\r\nContent-Type: application/json\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
    });
    (format!("http://{addr}/graphql"), rx)
}

fn team_filtering_server(projects: bool) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut bytes = vec![0; 65536];
        let n = stream.read(&mut bytes).unwrap();
        bytes.truncate(n);
        let narrowed = String::from_utf8_lossy(&bytes).contains("eqIgnoreCase");
        let body = if projects {
            let second = if narrowed {
                ""
            } else {
                r#",{"id":"p2","name":"Other","description":null,"url":null,"createdAt":null,"updatedAt":null,"status":{"name":"Started","type":"started"},"labels":{"nodes":[]}}"#
            };
            format!(
                r#"{{"data":{{"projects":{{"nodes":[{{"id":"p1","name":"Team","description":null,"url":null,"createdAt":null,"updatedAt":null,"status":{{"name":"Started","type":"started"}},"labels":{{"nodes":[]}}}}{second}],"pageInfo":{{"hasNextPage":false,"endCursor":null}}}}}}}}"#
            )
        } else {
            let second = if narrowed {
                ""
            } else {
                r#",{"id":"i2","title":"Other","description":null,"url":null,"createdAt":null,"updatedAt":null,"project":null,"state":{"name":"Todo","type":"unstarted"},"labels":{"nodes":[]}}"#
            };
            format!(
                r#"{{"data":{{"issues":{{"nodes":[{{"id":"i1","title":"Team","description":null,"url":null,"createdAt":null,"updatedAt":null,"project":null,"state":{{"name":"Todo","type":"unstarted"}},"labels":{{"nodes":[]}}}}{second}],"pageInfo":{{"hasNextPage":false,"endCursor":null}}}}}}}}"#
            )
        };
        write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
    });
    format!("http://{addr}/graphql")
}

#[test]
// llmlint: ignore-block[contracts_have_one_source_or_a_drift_gate] This deterministic check covers the locally pin-able field/argument and fixture-key contract; full scalar, nullability, variable-type, operator, and enum freshness exists only in Linear's authenticated unversioned explorer and cannot be gated without violating the mandated absent-credential skip.
fn pinned_schema_checks_selected_fields_arguments_and_fixture_keys() {
    use graphql_parser::{query, schema};
    use onetaskgraph_linear::graphql;
    let schema = schema::parse_schema::<String>(include_str!("fixtures/schema.graphql")).unwrap();
    let objects = schema
        .definitions
        .iter()
        .filter_map(|definition| match definition {
            schema::Definition::TypeDefinition(schema::TypeDefinition::Object(object)) => {
                Some((object.name.as_str(), object))
            }
            _ => None,
        })
        .collect::<std::collections::HashMap<_, _>>();
    fn named_type<'a>(kind: &'a schema::Type<'a, String>) -> &'a str {
        match kind {
            schema::Type::NamedType(name) => name,
            schema::Type::ListType(inner) | schema::Type::NonNullType(inner) => named_type(inner),
        }
    }
    fn validate<'a>(
        objects: &std::collections::HashMap<&str, &'a schema::ObjectType<'a, String>>,
        type_name: &str,
        selection: &query::SelectionSet<'a, String>,
        value: Option<&serde_json::Value>,
    ) {
        let object = objects
            .get(type_name)
            .unwrap_or_else(|| panic!("schema lacks {type_name}"));
        for selected in &selection.items {
            let query::Selection::Field(selected) = selected else {
                panic!("fixtures use no fragments")
            };
            let field = object
                .fields
                .iter()
                .find(|field| field.name == selected.name)
                .unwrap_or_else(|| panic!("{type_name} lacks field {}", selected.name));
            for (argument, _) in &selected.arguments {
                assert!(
                    field.arguments.iter().any(|input| input.name == *argument),
                    "{}.{} lacks argument {argument}",
                    type_name,
                    field.name
                );
            }
            let response = value.and_then(|value| value.get(&selected.name));
            if value.is_some() {
                assert!(
                    response.is_some() || selected.name == "endCursor",
                    "fixture lacks {type_name}.{}",
                    selected.name
                );
            }
            if !selected.selection_set.items.is_empty() {
                let response = response.and_then(|value| match value {
                    serde_json::Value::Array(values) => values.first(),
                    serde_json::Value::Null => None,
                    value => Some(value),
                });
                validate(
                    objects,
                    named_type(&field.field_type),
                    &selected.selection_set,
                    response,
                );
            }
        }
    }
    for (operation, fixture) in [
        (graphql::VIEWER, None),
        (graphql::ISSUE, None),
        (graphql::PROJECT, None),
        (graphql::ISSUES, Some(include_str!("fixtures/issues.json"))),
        (
            graphql::PROJECTS,
            Some(include_str!("fixtures/projects.json")),
        ),
        (graphql::LABELS, Some(include_str!("fixtures/labels.json"))),
        (
            graphql::ISSUE_RELATIONS,
            Some(include_str!("fixtures/issue-relations.json")),
        ),
        (
            graphql::PROJECT_RELATIONS,
            Some(include_str!("fixtures/project-relations.json")),
        ),
    ] {
        let document = query::parse_query::<String>(operation).unwrap();
        let fixture =
            fixture.map(|fixture| serde_json::from_str::<serde_json::Value>(fixture).unwrap());
        let query::Definition::Operation(query::OperationDefinition::Query(operation)) =
            &document.definitions[0]
        else {
            panic!("expected query")
        };
        let query::Selection::Field(root) = &operation.selection_set.items[0] else {
            panic!("expected root field")
        };
        let schema_root = objects["Query"]
            .fields
            .iter()
            .find(|field| field.name == root.name)
            .unwrap();
        for variable in &operation.variable_definitions {
            let argument = schema_root
                .arguments
                .iter()
                .find(|argument| argument.name == variable.name)
                .or_else(|| {
                    objects.values().find_map(|object| {
                        object.fields.iter().find_map(|field| {
                            field
                                .arguments
                                .iter()
                                .find(|argument| argument.name == variable.name)
                        })
                    })
                })
                .unwrap_or_else(|| panic!("schema lacks variable {}", variable.name));
            assert_eq!(
                format!("{:?}", variable.var_type),
                format!("{:?}", argument.value_type),
                "variable {} type drifted",
                variable.name
            );
        }
        validate(
            &objects,
            "Query",
            &operation.selection_set,
            fixture.as_ref().map(|fixture| &fixture["data"]),
        );
    }
}
// llmlint: ignore-end[contracts_have_one_source_or_a_drift_gate]

#[test]
fn factory_validates_config_and_missing_credentials() {
    let name = SourceName::new("work").unwrap();
    let error =
        match onetaskgraph_linear::Plugin.build(&name, &serde_json::json!({}), &Secrets(None)) {
            Err(e) => e,
            Ok(_) => panic!("missing secret accepted"),
        };
    assert!(matches!(error,SourceError::Auth{ref message} if message.contains("LINEAR_API_KEY")));
    let empty = Secrets(Some(SecretString::from("")));
    assert!(matches!(
        onetaskgraph_linear::Plugin.build(&name, &serde_json::json!({}), &empty),
        Err(SourceError::Auth { .. })
    ));
    let rendered = format!("{:?}", onetaskgraph_linear::LinearConfig::default());
    assert!(!rendered.contains("fixture-key"));
    assert_eq!(onetaskgraph_linear::Plugin.kind(), "linear");
    assert!(
        onetaskgraph_linear::Plugin
            .build(
                &name,
                &serde_json::json!({"api_key_env":"CUSTOM_OTG_TOKEN"}),
                &NamedSecrets,
            )
            .is_ok()
    );
}

#[tokio::test]
async fn tasks_use_real_http_parse_mapping_filters_and_paging() {
    let body = include_str!("fixtures/issues.json");
    let (endpoint, request) = server("200 OK", "", body);
    let source = source(&endpoint);
    let query = TaskQuery {
        labels: LabelFilter {
            any_of: vec!["Bug".into()],
            ..Default::default()
        },
        statuses: vec![StatusCategory::InProgress],
        project: ProjectFilter::Is("p1".into()),
        ..Default::default()
    };
    let page = source
        .query_tasks(
            &query,
            &PageRequest {
                cursor: None,
                limit: 1,
            },
        )
        .await
        .unwrap();
    assert_eq!(page.items[0].title, "Fixture issue");
    assert_eq!(page.items[0].status.name, "In Progress");
    assert_eq!(page.items[0].content.as_deref(), Some("Recorded body"));
    assert_eq!(
        page.items[0].metadata["caller.number"],
        serde_json::json!(7)
    );
    assert_eq!(
        page.items[0].repositories[0].as_str(),
        "github.com/acme/work"
    );
    assert_eq!(page.items[0].project.as_ref().unwrap().0, "p1");
    assert_eq!(page.items[0].labels[0].color.as_deref(), Some("#ff0000"));
    assert_eq!(
        page.items[0].url.as_deref(),
        Some("https://linear.app/acme/issue/ENG-1")
    );
    assert_eq!(
        page.items[0].created_at.unwrap().to_rfc3339(),
        "2026-08-01T12:00:00+00:00"
    );
    assert_eq!(
        page.items[0].updated_at.unwrap().to_rfc3339(),
        "2026-08-02T12:00:00+00:00"
    );
    assert_eq!(page.next.unwrap().0, "next-1");
    let wire = request.recv().unwrap();
    assert!(wire.contains("issues(first:$first"));
    assert!(wire.contains("inIgnoreCase"), "{wire}");
    assert!(wire.contains("started"));
    assert!(wire.contains("fixture-key"));
}

#[tokio::test]
async fn projects_labels_both_issue_directions_and_forward_project_edges_map() {
    let (endpoint, _) = server("200 OK", "", include_str!("fixtures/projects.json"));
    let projects = source(&endpoint)
        .query_projects(
            &ProjectQuery::default(),
            &PageRequest {
                cursor: None,
                limit: 50,
            },
        )
        .await
        .unwrap();
    assert_eq!(projects.items[0].title, "Fixture project");
    assert_eq!(projects.items[0].content.as_deref(), Some("Project body"));
    assert_eq!(
        projects.items[0].metadata["caller.enabled"],
        serde_json::json!(true)
    );
    assert_eq!(
        projects.items[0].repositories[0].as_str(),
        "github.com/acme/work"
    );
    assert_eq!(
        projects.items[0].labels[0].color.as_deref(),
        Some("#00ff00")
    );
    assert_eq!(
        projects.items[0].url.as_deref(),
        Some("https://linear.app/acme/project/p1")
    );
    assert!(projects.items[0].created_at.is_some() && projects.items[0].updated_at.is_some());
    let (endpoint, _) = server("200 OK", "", include_str!("fixtures/labels.json"));
    assert_eq!(
        source(&endpoint)
            .labels(&PageRequest {
                cursor: None,
                limit: 50
            })
            .await
            .unwrap()
            .items[0]
            .name,
        "Bug"
    );
    let (endpoint, _) = server("200 OK", "", include_str!("fixtures/issue-relations.json"));
    let edges = source(&endpoint)
        .task_dependencies(
            &"i1".into(),
            Direction::DependsOn,
            &PageRequest {
                cursor: None,
                limit: 50,
            },
        )
        .await
        .unwrap();
    assert_eq!(edges.items[0].to.id, "i2");
    let (endpoint, _) = server("200 OK", "", include_str!("fixtures/issue-relations.json"));
    let edges = source(&endpoint)
        .task_dependencies(
            &"i1".into(),
            Direction::DependedOnBy,
            &PageRequest {
                cursor: None,
                limit: 50,
            },
        )
        .await
        .unwrap();
    assert_eq!(edges.items[0].from.id, "i3");
    let (endpoint, _) = server(
        "200 OK",
        "",
        include_str!("fixtures/project-relations.json"),
    );
    assert_eq!(
        source(&endpoint)
            .project_dependencies(
                &"p1".into(),
                Direction::DependsOn,
                &PageRequest {
                    cursor: None,
                    limit: 50
                }
            )
            .await
            .unwrap()
            .items[0]
            .to
            .id,
        "p2"
    );
}

#[tokio::test]
async fn rate_limit_carries_retry_hint() {
    let (endpoint, _) = server("429 Too Many Requests", "Retry-After: 17\r\n", r#"{}"#);
    assert_eq!(
        source(&endpoint).health().await.unwrap_err(),
        SourceError::RateLimited {
            retry_after_seconds: Some(17)
        }
    );
}

#[tokio::test]
async fn graphql_rate_limit_uses_http_hint_and_viewer_id_is_validated() {
    let request = PageRequest {
        cursor: None,
        limit: 2,
    };
    let (endpoint, _) = server(
        "200 OK",
        "Retry-After: 23\r\n",
        r#"{"errors":[{"message":"slow","extensions":{"code":"RATELIMITED"}}]}"#,
    );
    assert_eq!(
        source(&endpoint).health().await.unwrap_err(),
        SourceError::RateLimited {
            retry_after_seconds: Some(23)
        }
    );
    for body in [
        r#"{"data":{"viewer":{}}}"#,
        r#"{"data":{"viewer":{"id":7}}}"#,
    ] {
        let (endpoint, _) = server("200 OK", "", body);
        assert!(matches!(
            source(&endpoint).health().await.unwrap_err(),
            SourceError::Malformed { .. }
        ));
    }
    let valid_project = serde_json::json!({"id":"p","name":"p","description":null,"url":null,"createdAt":null,"updatedAt":null,"status":{"name":"x","type":"started"},"labels":{"nodes":[]}});
    for field in ["status", "labels"] {
        let mut project = valid_project.clone();
        project.as_object_mut().unwrap().remove(field);
        let body = serde_json::json!({"data":{"projects":{"nodes":[project],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}).to_string();
        let (endpoint, _) = server("200 OK", "", body);
        assert!(matches!(
            source(&endpoint)
                .query_projects(&ProjectQuery::default(), &request)
                .await
                .unwrap_err(),
            SourceError::Malformed { .. }
        ));
    }
    for page_info in [
        serde_json::Value::Null,
        serde_json::json!({}),
        serde_json::json!({"hasNextPage":"no"}),
    ] {
        let body =
            serde_json::json!({"data":{"issues":{"nodes":[],"pageInfo":page_info}}}).to_string();
        let (endpoint, _) = server("200 OK", "", body);
        assert!(matches!(
            source(&endpoint)
                .query_tasks(&TaskQuery::default(), &request)
                .await
                .unwrap_err(),
            SourceError::Malformed { .. }
        ));
    }
    let (endpoint, _) = server(
        "200 OK",
        "",
        r#"{"errors":[{"message":"ordinary refusal","extensions":{"code":"BAD"}}]}"#,
    );
    assert!(
        matches!(source(&endpoint).health().await.unwrap_err(), SourceError::Refused { ref message } if message == "ordinary refusal")
    );
}

#[tokio::test]
async fn dependency_cursors_are_sent_on_second_task_and_project_requests() {
    let request = PageRequest {
        cursor: None,
        limit: 2,
    };
    for projects in [false, true] {
        let root = if projects { "project" } else { "issue" };
        let related = if projects {
            "relatedProject"
        } else {
            "relatedIssue"
        };
        let body = format!(
            r#"{{"data":{{"{root}":{{"relations":{{"nodes":[{{"type":"blocks","{related}":{{"id":"other"}}}}],"pageInfo":{{"hasNextPage":true,"endCursor":"next-edge"}}}},"inverseRelations":{{"nodes":[],"pageInfo":{{"hasNextPage":false,"endCursor":null}}}}}}}}}}"#
        );
        let (endpoint, _) = server("200 OK", "", body);
        let first = if projects {
            source(&endpoint)
                .project_dependencies(&"id".into(), Direction::DependsOn, &request)
                .await
                .unwrap()
        } else {
            source(&endpoint)
                .task_dependencies(&"id".into(), Direction::DependsOn, &request)
                .await
                .unwrap()
        };
        let cursor = first.next.unwrap();
        let body = format!(
            r#"{{"data":{{"{root}":{{"relations":{{"nodes":[],"pageInfo":{{"hasNextPage":false,"endCursor":null}}}},"inverseRelations":{{"nodes":[],"pageInfo":{{"hasNextPage":false,"endCursor":null}}}}}}}}}}"#
        );
        let (endpoint, wire) = server("200 OK", "", body);
        let second = PageRequest {
            cursor: Some(cursor),
            limit: 2,
        };
        if projects {
            source(&endpoint)
                .project_dependencies(&"id".into(), Direction::DependsOn, &second)
                .await
                .unwrap();
        } else {
            source(&endpoint)
                .task_dependencies(&"id".into(), Direction::DependsOn, &second)
                .await
                .unwrap();
        }
        assert!(wire.recv().unwrap().contains("next-edge"));
    }
}

#[tokio::test]
async fn team_configuration_narrows_task_and_project_results() {
    let name = SourceName::new("team").unwrap();
    for projects in [false, true] {
        let source = onetaskgraph_linear::Plugin
            .build(
                &name,
                &serde_json::json!({"endpoint":team_filtering_server(projects),"team":"ENG"}),
                &Secrets(Some("x".into())),
            )
            .unwrap();
        let request = PageRequest {
            cursor: None,
            limit: 10,
        };
        let count = if projects {
            source
                .query_projects(&ProjectQuery::default(), &request)
                .await
                .unwrap()
                .items
                .len()
        } else {
            source
                .query_tasks(&TaskQuery::default(), &request)
                .await
                .unwrap()
                .items
                .len()
        };
        assert_eq!(count, 1);
    }
}

#[tokio::test]
async fn item_reads_and_transport_error_boundaries_are_exercised() {
    let name = SourceName::new("work").unwrap();
    for config in [
        serde_json::json!({"api_key_env":""}),
        serde_json::json!({"api_key_env":"lowercase"}),
        serde_json::json!({"api_key_env":"123_INVALID"}),
        serde_json::json!({"endpoint":""}),
        serde_json::json!({"endpoint":"not a url"}),
        serde_json::json!({"endpoint":"file:///tmp/x"}),
        serde_json::json!({"team":" "}),
        serde_json::json!({"unknown":true}),
    ] {
        assert!(matches!(
            onetaskgraph_linear::Plugin.build(&name, &config, &Secrets(Some("x".into()))),
            Err(SourceError::Config { .. })
        ));
    }
    let (endpoint, _) = server("200 OK", "", r#"{"data":{"viewer":{"id":"u"}}}"#);
    assert!(source(&endpoint).health().await.unwrap().reachable);
    let issue = r#"{"data":{"issue":{"id":"i1","title":"One","description":null,"url":null,"createdAt":null,"updatedAt":null,"state":{"name":"Backlog","type":"backlog"},"labels":{"nodes":[]},"project":null}}}"#;
    let (endpoint, _) = server("200 OK", "", issue);
    assert_eq!(
        source(&endpoint)
            .get_task(&"i1".into())
            .await
            .unwrap()
            .unwrap()
            .status
            .category,
        StatusCategory::Backlog
    );
    let project = r#"{"data":{"project":{"id":"p1","name":"One","description":null,"url":null,"createdAt":null,"updatedAt":null,"status":{"name":"Done","type":"completed"},"labels":{"nodes":[]}}}}"#;
    let (endpoint, _) = server("200 OK", "", project);
    assert_eq!(
        source(&endpoint)
            .get_project(&"p1".into())
            .await
            .unwrap()
            .unwrap()
            .status
            .category,
        StatusCategory::Done
    );
    let (endpoint, _) = server("200 OK", "", r#"{"data":{"issue":null}}"#);
    assert!(
        source(&endpoint)
            .get_task(&"none".into())
            .await
            .unwrap()
            .is_none()
    );
    for (status, expected) in [
        ("401 Unauthorized", "auth"),
        ("403 Forbidden", "auth"),
        ("500 Server Error", "unavailable"),
    ] {
        let (endpoint, _) = server(status, "", r#"{}"#);
        let error = source(&endpoint).health().await.unwrap_err();
        assert!(match expected {
            "auth" => matches!(error, SourceError::Auth { .. }),
            _ => matches!(error, SourceError::Unavailable { .. }),
        });
    }
    for (body, rate) in [
        (
            r#"{"errors":[{"message":"slow","extensions":{"code":"RATELIMITED","retryAfter":9}}]}"#,
            true,
        ),
        (
            r#"{"errors":[{"message":"slow","extensions":{"code":"RATE_LIMITED","retryAfter":9}}]}"#,
            true,
        ),
        (
            r#"{"errors":[{"message":"bad","extensions":{"code":"BAD"}}]}"#,
            false,
        ),
    ] {
        let (endpoint, _) = server("200 OK", "", body);
        let error = source(&endpoint).health().await.unwrap_err();
        assert_eq!(
            matches!(
                error,
                SourceError::RateLimited {
                    retry_after_seconds: Some(9)
                }
            ),
            rate
        );
    }
    for body in [r#"{"data":null}"#, r#"not json"#] {
        let (endpoint, _) = server("200 OK", "", body);
        assert!(matches!(
            source(&endpoint).health().await.unwrap_err(),
            SourceError::Malformed { .. }
        ));
    }
}

#[tokio::test]
async fn query_shapes_reverse_project_edges_and_public_metadata_are_covered() {
    let body = r#"{"data":{"issues":{"nodes":[{"id":"a","title":"A","description":null,"url":null,"createdAt":null,"updatedAt":null,"project":null,"state":{"name":"Todo","type":"unstarted"},"labels":{"nodes":[]}}, {"id":"b","title":"B","description":null,"url":null,"createdAt":null,"updatedAt":null,"project":null,"state":{"name":"Doing","type":"started"},"labels":{"nodes":[]}}, {"id":"c","title":"C","description":null,"url":null,"createdAt":null,"updatedAt":null,"project":null,"state":{"name":"Canceled","type":"canceled"},"labels":{"nodes":[]}}, {"id":"d","title":"D","description":null,"url":null,"createdAt":null,"updatedAt":null,"project":null,"state":{"name":"Odd","type":"new-value"},"labels":{"nodes":[]}}],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}"#;
    let (endpoint, wire) = server("200 OK", "", body);
    let query = TaskQuery {
        labels: LabelFilter {
            all_of: vec!["One".into()],
            none_of: vec!["Two".into()],
            ..Default::default()
        },
        statuses: vec![
            StatusCategory::Todo,
            StatusCategory::InProgress,
            StatusCategory::Cancelled,
            StatusCategory::Unknown,
            StatusCategory::Backlog,
            StatusCategory::Done,
        ],
        project: ProjectFilter::Orphans,
        ..Default::default()
    };
    let page = source(&endpoint)
        .query_tasks(
            &query,
            &PageRequest {
                cursor: Some(onetaskgraph_plugin_api::Cursor("2".into())),
                limit: 999,
            },
        )
        .await
        .unwrap();
    assert_eq!(page.items.len(), 4);
    let wire = wire.recv().unwrap();
    assert!(wire.contains("every") && wire.contains("null") && wire.contains("250"));
    let (endpoint, _) = server(
        "200 OK",
        "",
        include_str!("fixtures/project-relations.json"),
    );
    let edge = source(&endpoint)
        .project_dependencies(
            &"p1".into(),
            Direction::DependedOnBy,
            &PageRequest {
                cursor: None,
                limit: 2,
            },
        )
        .await
        .unwrap()
        .items
        .remove(0);
    assert_eq!(edge.from.id, "p3");
    let (endpoint, wire) = server("200 OK", "", include_str!("fixtures/projects.json"));
    source(&endpoint)
        .query_projects(
            &ProjectQuery {
                statuses: vec![StatusCategory::InProgress],
                ..Default::default()
            },
            &PageRequest {
                cursor: None,
                limit: 10,
            },
        )
        .await
        .unwrap();
    assert!(wire.recv().unwrap().contains("started"));
    let caps = source("http://127.0.0.1:1").capabilities();
    assert_eq!(caps.max_page_size, 250);
    assert!(matches!(
        caps.search_title,
        onetaskgraph_plugin_api::Support::Unsupported
    ));
    assert_eq!(source("http://127.0.0.1:1").kind(), "linear");
    let schema = serde_json::to_value(onetaskgraph_linear::Plugin.config_schema()).unwrap();
    assert!(schema.to_string().contains("api_key_env"));
}

#[tokio::test]
async fn selected_malformed_task_project_and_relation_shapes_are_rejected() {
    let request = PageRequest {
        cursor: None,
        limit: 2,
    };
    let valid_task = serde_json::json!({"id":"i","title":"t","description":null,"url":null,"createdAt":null,"updatedAt":null,"project":null,"state":{"name":"x","type":"started"},"labels":{"nodes":[]}});
    for field in [
        "description",
        "state",
        "labels",
        "project",
        "url",
        "createdAt",
        "updatedAt",
    ] {
        let mut task = valid_task.clone();
        task.as_object_mut().unwrap().remove(field);
        let body = serde_json::json!({"data":{"issue":task}}).to_string();
        let (endpoint, _) = server("200 OK", "", body);
        assert!(matches!(
            source(&endpoint).get_task(&"i".into()).await.unwrap_err(),
            SourceError::Malformed { .. }
        ));
    }
    for (field, value) in [
        ("description", serde_json::json!(7)),
        ("createdAt", serde_json::json!("yesterday")),
    ] {
        let mut task = valid_task.clone();
        task[field] = value;
        let body = serde_json::json!({"data":{"issue":task}}).to_string();
        let (endpoint, _) = server("200 OK", "", body);
        assert!(matches!(
            source(&endpoint).get_task(&"i".into()).await.unwrap_err(),
            SourceError::Malformed { .. }
        ));
    }
    let (endpoint, _) = server("200 OK", "", r#"{"data":{}}"#);
    assert!(matches!(
        source(&endpoint).get_task(&"i".into()).await.unwrap_err(),
        SourceError::Malformed { .. }
    ));
    let (endpoint, _) = server("200 OK", "", r#"{"data":{}}"#);
    assert!(matches!(
        source(&endpoint).health().await.unwrap_err(),
        SourceError::Malformed { .. }
    ));
    for body in [
        r#"{"data":{}}"#,
        r#"{"data":{"issues":{}}}"#,
        r#"{"data":{"issues":{"nodes":[{}],"pageInfo":{}}}}"#,
        r#"{"data":{"issues":{"nodes":[{"id":"i","title":"t","state":{"name":"x","type":"started"}}],"pageInfo":{}}}}"#,
        r#"{"data":{"issues":{"nodes":[{"id":"i","title":"t","state":{"name":"x","type":"started"},"labels":{}}],"pageInfo":{}}}}"#,
    ] {
        let (endpoint, _) = server("200 OK", "", body);
        assert!(matches!(
            source(&endpoint)
                .query_tasks(&TaskQuery::default(), &request)
                .await
                .unwrap_err(),
            SourceError::Malformed { .. }
        ));
    }
    for body in [
        r#"{"data":{"projects":{"nodes":[{"id":"p","name":"p","labels":{"nodes":[]}}],"pageInfo":{}}}}"#,
        r#"{"data":{"projects":{"nodes":[{"id":"p","name":"p","status":{"name":"x","type":"started"}}],"pageInfo":{}}}}"#,
    ] {
        let (endpoint, _) = server("200 OK", "", body);
        assert!(matches!(
            source(&endpoint)
                .query_projects(&ProjectQuery::default(), &request)
                .await
                .unwrap_err(),
            SourceError::Malformed { .. }
        ));
    }
    let (endpoint, _) = server(
        "200 OK",
        "",
        r#"{"data":{"issue":{"id":"i","title":"t","createdAt":"yesterday","state":{"name":"x","type":"started"},"labels":{"nodes":[]}}}}"#,
    );
    assert!(matches!(
        source(&endpoint).get_task(&"i".into()).await.unwrap_err(),
        SourceError::Malformed { .. }
    ));
    for body in [
        r#"{"data":{"issue":{}}}"#,
        r#"{"data":{"issue":{"relations":{}}}}"#,
        r#"{"data":{"issue":{"relations":{"nodes":[{}],"pageInfo":{}}}}}"#,
        r#"{"data":{"issue":{"relations":{"nodes":[{"relatedIssue":{"id":"other"}}],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}}"#,
    ] {
        let (endpoint, _) = server("200 OK", "", body);
        assert!(matches!(
            source(&endpoint)
                .task_dependencies(&"i".into(), Direction::DependsOn, &request)
                .await
                .unwrap_err(),
            SourceError::Malformed { .. }
        ));
    }
    let (endpoint, _) = server(
        "200 OK",
        "",
        r#"{"data":{"issue":{"relations":{"nodes":[{"type":"related","relatedIssue":{"id":"other"}}],"pageInfo":{"hasNextPage":true,"endCursor":"r2"}},"inverseRelations":{"nodes":[],"pageInfo":{}}}}}"#,
    );
    let page = source(&endpoint)
        .task_dependencies(&"i".into(), Direction::DependsOn, &request)
        .await
        .unwrap();
    assert!(matches!(
        page.items[0].kind,
        onetaskgraph_plugin_api::DependencyKind::Related
    ));
    assert_eq!(page.next.unwrap().0, "r2");
    let (endpoint, _) = server(
        "200 OK",
        "",
        r#"{"data":{"issue":{"relations":{"nodes":[{"type":"invented","relatedIssue":{"id":"other"}}],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}}"#,
    );
    assert!(matches!(
        source(&endpoint)
            .task_dependencies(&"i".into(), Direction::DependsOn, &request)
            .await
            .unwrap_err(),
        SourceError::Malformed { ref message } if message.contains("relation type")
    ));
    assert!(matches!(
        source("http://127.0.0.1:1").health().await.unwrap_err(),
        SourceError::Unavailable { .. }
    ));
    let (endpoint, wire) = server("200 OK", "", include_str!("fixtures/issues.json"));
    let configured = onetaskgraph_linear::Plugin
        .build(
            &SourceName::new("team").unwrap(),
            &serde_json::json!({"endpoint":endpoint,"team":"ENG"}),
            &Secrets(Some("x".into())),
        )
        .unwrap();
    configured
        .query_tasks(&TaskQuery::default(), &request)
        .await
        .unwrap();
    assert!(wire.recv().unwrap().contains("eqIgnoreCase"));
    let (endpoint, wire) = server("200 OK", "", include_str!("fixtures/projects.json"));
    let configured = onetaskgraph_linear::Plugin
        .build(
            &SourceName::new("team-projects").unwrap(),
            &serde_json::json!({"endpoint":endpoint,"team":"ENG"}),
            &Secrets(Some("x".into())),
        )
        .unwrap();
    configured
        .query_projects(&ProjectQuery::default(), &request)
        .await
        .unwrap();
    assert!(wire.recv().unwrap().contains("eqIgnoreCase"));
}
