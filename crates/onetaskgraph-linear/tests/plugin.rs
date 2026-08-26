//! Public factory and real-HTTP fixture journeys.

use onetaskgraph_plugin_api::{
    Direction, ItemKind, LabelFilter, PageRequest, ProjectFilter, ProjectQuery, SecretResolver,
    SourceError, SourceName, SourcePlugin, StatusCategory, TaskQuery, TaskSource,
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

#[test]
fn pinned_schema_names_every_write_operation_the_plugin_sends() {
    use graphql_parser::{query, schema};
    use onetaskgraph_linear::graphql;
    let schema = schema::parse_schema::<String>(include_str!("fixtures/schema.graphql")).unwrap();
    let fields = |root: &str| {
        schema
            .definitions
            .iter()
            .find_map(|definition| match definition {
                schema::Definition::TypeDefinition(schema::TypeDefinition::Object(object))
                    if object.name == root =>
                {
                    Some(
                        object
                            .fields
                            .iter()
                            .map(|field| field.name.as_str())
                            .collect::<Vec<_>>(),
                    )
                }
                _ => None,
            })
            .unwrap()
    };
    let query_fields = fields("Query");
    let mutation_fields = fields("Mutation");
    for (document, mutation) in [
        (graphql::TEAM, false),
        (graphql::ISSUE_STATE, false),
        (graphql::PROJECT_STATUS, false),
        (graphql::ISSUE_LABEL, false),
        (graphql::PROJECT_LABEL, false),
        (graphql::ISSUE_CREATE, true),
        (graphql::ISSUE_UPDATE, true),
        (graphql::PROJECT_CREATE, true),
        (graphql::PROJECT_UPDATE, true),
        (graphql::ISSUE_RELATION_CREATE, true),
        (graphql::PROJECT_RELATION_CREATE, true),
        (graphql::ISSUE_DELETE, true),
    ] {
        let parsed = query::parse_query::<String>(document).unwrap();
        let selection = match &parsed.definitions[0] {
            query::Definition::Operation(query::OperationDefinition::Query(operation)) => {
                &operation.selection_set.items[0]
            }
            query::Definition::Operation(query::OperationDefinition::Mutation(operation)) => {
                &operation.selection_set.items[0]
            }
            _ => panic!("production document is an explicit query or mutation"),
        };
        let query::Selection::Field(root) = selection else {
            panic!("operation has a root field")
        };
        assert!(
            (if mutation {
                &mutation_fields
            } else {
                &query_fields
            })
            .contains(&root.name.as_str()),
            "pinned schema lacks {}",
            root.name
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
async fn the_metadata_slot_changes_nothing_else_the_item_carries() {
    // The slot lives inside the description, so the field it could plausibly disturb is
    // the content — and the ones a reader would never think to check are the rest. This
    // reads the same issue twice, once with the slot and once without, and asserts that
    // the only difference between the two is the metadata and the origins read out of it.
    async fn read(description: &str) -> onetaskgraph_plugin_api::Task {
        let mut body: serde_json::Value =
            serde_json::from_str(include_str!("fixtures/issues.json")).unwrap();
        body["data"]["issues"]["nodes"][0]["description"] = serde_json::json!(description);
        let (endpoint, _) = server("200 OK", "", body.to_string());
        source(&endpoint)
            .query_tasks(
                &TaskQuery::default(),
                &PageRequest {
                    cursor: None,
                    limit: 1,
                },
            )
            .await
            .expect("the fixture issue reads")
            .items
            .remove(0)
    }

    let bare = read("Recorded body").await;
    let with_slot = read(
        "Recorded body\n\n<!-- onetaskgraph.metadata\n{\"caller.number\":7,\"onetaskgraph.repositories\":[\"github.com/acme/work\"]}\n-->",
    )
    .await;

    assert!(bare.metadata.is_empty());
    assert!(bare.repositories.is_empty());
    assert_eq!(with_slot.metadata["caller.number"], serde_json::json!(7));
    assert_eq!(with_slot.repositories[0].as_str(), "github.com/acme/work");
    assert_eq!(
        onetaskgraph_plugin_api::Task {
            metadata: Default::default(),
            repositories: Vec::new(),
            ..with_slot
        },
        bare,
        "the slot must leave the title, the content, the labels, the state and the rest alone"
    );
}

#[tokio::test]
async fn linear_metadata_slot_rejects_malformed_values_and_preserves_non_trailing_markers() {
    for description in [
        "visible\n<!-- onetaskgraph.metadata\n{}",
        "visible\n<!-- onetaskgraph.metadata\n{bad}\n-->",
        "visible\n<!-- onetaskgraph.metadata\n{\"onetaskgraph.repositories\":7}\n-->",
    ] {
        let mut body: serde_json::Value =
            serde_json::from_str(include_str!("fixtures/issues.json")).unwrap();
        body["data"]["issues"]["nodes"][0]["description"] = serde_json::json!(description);
        let (endpoint, _) = server("200 OK", "", body.to_string());
        assert!(matches!(
            source(&endpoint)
                .query_tasks(
                    &TaskQuery::default(),
                    &PageRequest {
                        cursor: None,
                        limit: 1
                    }
                )
                .await,
            Err(SourceError::Malformed { .. })
        ));
    }

    let description = "visible\n<!-- onetaskgraph.metadata\n{}\n-->\ntrailing content";
    let mut body: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/issues.json")).unwrap();
    body["data"]["issues"]["nodes"][0]["description"] = serde_json::json!(description);
    let (endpoint, _) = server("200 OK", "", body.to_string());
    let page = source(&endpoint)
        .query_tasks(
            &TaskQuery::default(),
            &PageRequest {
                cursor: None,
                limit: 1,
            },
        )
        .await
        .expect("a non-trailing marker is visible content, not a reserved slot");
    assert_eq!(page.items[0].content.as_deref(), Some(description));
    assert!(page.items[0].metadata.is_empty());
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
    assert_eq!(edges.items[0].to.id(), "i2");
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
    assert_eq!(edges.items[0].from.id(), "i3");
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
            .id(),
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
async fn a_far_end_in_another_source_is_read_from_the_reserved_key_at_both_levels() {
    // `relatedIssue` and `relatedProject` hold a Linear id and nothing else, so an edge
    // into another source is the one edge no Linear relation can name. It is read from the
    // near item's own reserved key and served after the native relations are spent.
    for projects in [false, true] {
        let (root, fixture) = if projects {
            ("project", include_str!("fixtures/project-relations.json"))
        } else {
            ("issue", include_str!("fixtures/issue-relations.json"))
        };
        let request = PageRequest {
            cursor: None,
            limit: 50,
        };
        let (endpoint, _) = server("200 OK", "", fixture);
        let native = if projects {
            source(&endpoint)
                .project_dependencies(&"p1".into(), Direction::DependsOn, &request)
                .await
                .unwrap()
        } else {
            source(&endpoint)
                .task_dependencies(&"i1".into(), Direction::DependsOn, &request)
                .await
                .unwrap()
        };
        assert_eq!(
            native.items.len(),
            1,
            "{root}: the native relation is first"
        );
        let tail = native
            .next
            .expect("a recorded far end still owes the walk a page");

        let (endpoint, _) = server("200 OK", "", fixture);
        let recorded = if projects {
            source(&endpoint)
                .project_dependencies(
                    &"p1".into(),
                    Direction::DependsOn,
                    &PageRequest {
                        cursor: Some(tail),
                        limit: 50,
                    },
                )
                .await
                .unwrap()
        } else {
            source(&endpoint)
                .task_dependencies(
                    &"i1".into(),
                    Direction::DependsOn,
                    &PageRequest {
                        cursor: Some(tail),
                        limit: 50,
                    },
                )
                .await
                .unwrap()
        };
        assert_eq!(recorded.items.len(), 1, "{root}");
        assert_eq!(recorded.items[0].to.id(), "elsewhere:P-9", "{root}");
        assert_eq!(recorded.items[0].to.kind, ItemKind::Project, "{root}");
        assert_eq!(
            recorded.items[0].from.id(),
            if projects { "p1" } else { "i1" },
            "{root}"
        );
        assert!(recorded.next.is_none(), "{root}");

        // The reverse of a recorded edge is derived from the far end, never recorded here.
        let (endpoint, _) = server("200 OK", "", fixture);
        let reverse = if projects {
            source(&endpoint)
                .project_dependencies(&"p1".into(), Direction::DependedOnBy, &request)
                .await
                .unwrap()
        } else {
            source(&endpoint)
                .task_dependencies(&"i1".into(), Direction::DependedOnBy, &request)
                .await
                .unwrap()
        };
        assert!(reverse.next.is_none(), "{root}");
        assert!(
            reverse.items.iter().all(|edge| !edge.from.is_qualified()),
            "{root}"
        );
    }
}

/// One Linear relations response for `root`, whose description records `recorded`.
fn relations_recording(root: &str, recorded: &serde_json::Value) -> String {
    let slot = format!(
        "body\n\n<!-- onetaskgraph.metadata\n{}\n-->",
        serde_json::json!({ "onetaskgraph.depends_on": recorded })
    );
    serde_json::json!({"data":{(root):{
        "description": slot,
        "relations":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}},
        "inverseRelations":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}
    }}})
    .to_string()
}

#[tokio::test]
async fn linear_may_not_record_a_far_end_its_own_relations_can_name() {
    // `relations` on an issue holds issues and on a project holds projects, all of this
    // workspace. Recording one of those is a plan Linear itself would have drawn, so it is
    // refused rather than read — that is the native-first rule, enforced at the boundary.
    // Writing this source's own name out is the same entry spelled differently, so it is
    // refused on the same terms: `work` is what this source is configured as.
    for (projects, root, misplaced) in [
        (false, "issue", serde_json::json!(["ENG-2"])),
        (
            false,
            "issue",
            serde_json::json!([{"id":"ENG-2","kind":"task"}]),
        ),
        (
            false,
            "issue",
            serde_json::json!([{"id":"work:ENG-2","kind":"task"}]),
        ),
        (
            true,
            "project",
            serde_json::json!([{"id":"PRJ-2","kind":"project"}]),
        ),
        (
            true,
            "project",
            serde_json::json!([{"id":"work:PRJ-2","kind":"project"}]),
        ),
    ] {
        let (endpoint, _) = server("200 OK", "", relations_recording(root, &misplaced));
        let request = PageRequest {
            cursor: None,
            limit: 50,
        };
        let source = source(&endpoint);
        let error = if projects {
            source
                .project_dependencies(&"p1".into(), Direction::DependsOn, &request)
                .await
        } else {
            source
                .task_dependencies(&"i1".into(), Direction::DependsOn, &request)
                .await
        }
        .expect_err("a same-source far end of the kind Linear relates");
        let message = format!("{error}");
        assert!(message.contains("relate natively"), "{message}");
        assert!(message.contains("onetaskgraph.depends_on"), "{message}");
    }
}

#[tokio::test]
async fn linear_records_the_far_end_no_relation_of_its_own_can_hold() {
    // The two cases no Linear relation can express: an item of another source, and one at
    // the other level of this one — which this source's own name may qualify, because
    // naming the source says nothing about a level `relations` cannot cross.
    for (recorded, expected) in [
        (
            serde_json::json!([{"id":"elsewhere:P-9","kind":"project"}]),
            "elsewhere:P-9",
        ),
        (
            serde_json::json!([{"id":"PRJ-9","kind":"project"}]),
            "PRJ-9",
        ),
        (
            serde_json::json!([{"id":"work:PRJ-9","kind":"project"}]),
            "work:PRJ-9",
        ),
    ] {
        let (endpoint, _) = server("200 OK", "", relations_recording("issue", &recorded));
        let first = source(&endpoint)
            .task_dependencies(
                &"i1".into(),
                Direction::DependsOn,
                &PageRequest {
                    cursor: None,
                    limit: 50,
                },
            )
            .await
            .expect("the native page is answered");
        let tail = first.next.expect("a recorded far end still owes a page");

        let (endpoint, _) = server("200 OK", "", relations_recording("issue", &recorded));
        let recorded_page = source(&endpoint)
            .task_dependencies(
                &"i1".into(),
                Direction::DependsOn,
                &PageRequest {
                    cursor: Some(tail),
                    limit: 50,
                },
            )
            .await
            .expect("the recorded tail is answered");
        assert_eq!(recorded_page.items.len(), 1, "{recorded}");
        assert_eq!(recorded_page.items[0].from.id(), "i1", "{recorded}");
        assert_eq!(recorded_page.items[0].to.id(), expected, "{recorded}");
        assert_eq!(
            recorded_page.items[0].to.kind,
            ItemKind::Project,
            "{recorded}"
        );
    }
}

#[tokio::test]
async fn a_recorded_cursor_is_refused_in_the_direction_that_never_issued_it() {
    // The recorded tail is forward-only: the reverse of a recorded edge is derived from
    // the far end and is never written down here. So a reverse read handed the forward
    // tail's cursor is resuming a walk it did not come from, and answering it would return
    // forward edges to a caller who asked which items depend on this one.
    // An offset that is not a number resumes nothing at all, and is the other way to
    // present a cursor this source never reported.
    //
    // Both refusals are decided from the cursor alone, so this source is pointed at a port
    // nothing listens on: an answer at all would mean the request was made first.
    let source = source("http://127.0.0.1:1/graphql");
    for (projects, direction, cursor, expected) in [
        (
            false,
            Direction::DependedOnBy,
            "onetaskgraph.depends_on:0",
            "reverse dependency read",
        ),
        (
            true,
            Direction::DependedOnBy,
            "onetaskgraph.depends_on:0",
            "reverse dependency read",
        ),
        (
            false,
            Direction::DependsOn,
            "onetaskgraph.depends_on:x",
            "is not a recorded-edge cursor",
        ),
    ] {
        let request = PageRequest {
            cursor: Some(onetaskgraph_plugin_api::Cursor(cursor.to_owned())),
            limit: 50,
        };
        let error = if projects {
            source
                .project_dependencies(&"p1".into(), direction, &request)
                .await
        } else {
            source
                .task_dependencies(&"i1".into(), direction, &request)
                .await
        }
        .expect_err("a cursor no walk of this source reported");
        let message = format!("{error}");
        assert!(message.contains(cursor), "{message}");
        assert!(message.contains(expected), "{message}");
    }
}

#[tokio::test]
async fn a_reserved_dependency_entry_this_interface_cannot_read_is_refused_by_name() {
    for (recorded, expected) in [
        (
            serde_json::json!([{"id":"","kind":"project"}]),
            "cannot be empty",
        ),
        (
            serde_json::json!([{"id":"bad source:P-9","kind":"project"}]),
            "source name",
        ),
        (
            serde_json::json!([{"id":"elsewhere:","kind":"project"}]),
            "native id",
        ),
        (
            serde_json::json!("elsewhere:P-9"),
            "not a list of dependency endpoints",
        ),
    ] {
        let (endpoint, _) = server("200 OK", "", relations_recording("issue", &recorded));
        let error = source(&endpoint)
            .task_dependencies(
                &"i1".into(),
                Direction::DependsOn,
                &PageRequest {
                    cursor: None,
                    limit: 50,
                },
            )
            .await
            .expect_err("an entry this interface cannot represent");
        let message = format!("{error}");
        assert!(message.contains(expected), "{recorded}: {message}");
    }
}

#[tokio::test]
async fn a_reserved_dependency_key_holding_the_wrong_shape_is_refused_by_name() {
    let body = r#"{"data":{"issue":{"description":"body\n\n<!-- onetaskgraph.metadata\n{\"onetaskgraph.depends_on\":7}\n-->","relations":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}},"inverseRelations":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}}"#;
    let (endpoint, _) = server("200 OK", "", body);
    let error = source(&endpoint)
        .task_dependencies(
            &"i1".into(),
            Direction::DependsOn,
            &PageRequest {
                cursor: None,
                limit: 50,
            },
        )
        .await
        .expect_err("a number is not a list of endpoints");
    assert!(
        format!("{error}").contains("onetaskgraph.depends_on"),
        "{error}"
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
            r#"{{"data":{{"{root}":{{"description":null,"relations":{{"nodes":[{{"type":"blocks","{related}":{{"id":"other"}}}}],"pageInfo":{{"hasNextPage":true,"endCursor":"next-edge"}}}},"inverseRelations":{{"nodes":[],"pageInfo":{{"hasNextPage":false,"endCursor":null}}}}}}}}}}"#
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
            r#"{{"data":{{"{root}":{{"description":null,"relations":{{"nodes":[],"pageInfo":{{"hasNextPage":false,"endCursor":null}}}},"inverseRelations":{{"nodes":[],"pageInfo":{{"hasNextPage":false,"endCursor":null}}}}}}}}}}"#
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
    assert_eq!(edge.from.id(), "p3");
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
