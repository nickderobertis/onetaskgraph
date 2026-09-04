//! Public factory and real-HTTP fixture journeys.

use onetaskgraph_plugin_api::{
    DependencyEdge, DependencyEndpoint, DependencyKind, Direction, Document, DocumentQuery,
    ItemKind, ItemWrite, Label, LabelFilter, Location, PageRequest, Project, ProjectFilter,
    ProjectQuery, SecretResolver, SourceError, SourceName, SourcePlugin, StatusCategory, Task,
    TaskQuery, TaskSource,
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

fn writable_source(endpoint: &str) -> Box<dyn TaskSource> {
    onetaskgraph_linear::Plugin
        .build(
            &SourceName::new("work").unwrap(),
            &serde_json::json!({"endpoint":endpoint,"team":"ENG"}),
            &Secrets(Some("fixture-key".into())),
        )
        .unwrap()
}

fn response_server(responses: Vec<serde_json::Value>) -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for response in responses {
            let (mut stream, _) = listener.accept().unwrap();
            let mut bytes = vec![0; 65536];
            let n = stream.read(&mut bytes).unwrap();
            bytes.truncate(n);
            let _ = tx.send(String::from_utf8_lossy(&bytes).into_owned());
            let body = serde_json::to_string(&serde_json::json!({"data":response})).unwrap();
            write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
        }
    });
    (format!("http://{addr}/graphql"), rx)
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
    let inputs = schema
        .definitions
        .iter()
        .filter_map(|definition| match definition {
            schema::Definition::TypeDefinition(schema::TypeDefinition::InputObject(input)) => {
                Some((input.name.as_str(), input))
            }
            _ => None,
        })
        .collect::<std::collections::HashMap<_, _>>();
    for (name, expected) in [
        (
            "IssueCreateInput",
            &[
                "teamId",
                "title",
                "description",
                "stateId",
                "labelIds",
                "projectId",
            ][..],
        ),
        (
            "IssueUpdateInput",
            &["title", "description", "stateId", "labelIds", "projectId"][..],
        ),
        (
            "ProjectCreateInput",
            &["teamIds", "name", "description", "statusId", "labelIds"][..],
        ),
        (
            "ProjectUpdateInput",
            &["name", "description", "statusId", "labelIds"][..],
        ),
        (
            "IssueRelationCreateInput",
            &["issueId", "relatedIssueId", "type"][..],
        ),
        (
            "ProjectRelationCreateInput",
            &[
                "projectId",
                "relatedProjectId",
                "type",
                "anchorType",
                "relatedAnchorType",
            ][..],
        ),
        (
            "DocumentCreateInput",
            &["title", "content", "projectId", "teamId"][..],
        ),
        (
            "DocumentUpdateInput",
            &["title", "content", "projectId"][..],
        ),
    ] {
        let actual = inputs[name]
            .fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(actual, expected, "pinned {name} fields drifted");
    }
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
        (graphql::DOCUMENT, None),
        (
            graphql::DOCUMENTS,
            Some(include_str!("fixtures/documents.json")),
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
    let inputs = schema
        .definitions
        .iter()
        .filter_map(|definition| match definition {
            schema::Definition::TypeDefinition(schema::TypeDefinition::InputObject(input)) => {
                Some((input.name.as_str(), input))
            }
            _ => None,
        })
        .collect::<std::collections::HashMap<_, _>>();
    fn named<'a>(kind: &'a schema::Type<'a, String>) -> &'a str {
        match kind {
            schema::Type::NamedType(name) => name,
            schema::Type::ListType(inner) | schema::Type::NonNullType(inner) => named(inner),
        }
    }
    fn same_type(schema: &schema::Type<'_, String>, query: &query::Type<'_, String>) -> bool {
        match (schema, query) {
            (schema::Type::NamedType(left), query::Type::NamedType(right)) => left == right,
            (schema::Type::ListType(left), query::Type::ListType(right))
            | (schema::Type::NonNullType(left), query::Type::NonNullType(right)) => {
                same_type(left, right)
            }
            _ => false,
        }
    }
    /// Whether a variable declared `variable` may stand at a location typed `location`.
    ///
    /// GraphQL admits it when the two are the same type, or when the variable is the
    /// location type's non-null form — never when only the value would coerce. `String!`
    /// at an `ID` is the second case failing, and it is what Linear refused the state
    /// lookup for; `String!` at a `String` is the non-null case passing, which the sibling
    /// `eqIgnoreCase` in the very same filter relies on.
    fn usable_at(location: &schema::Type<'_, String>, variable: &query::Type<'_, String>) -> bool {
        same_type(location, variable)
            || matches!(variable, query::Type::NonNullType(inner) if same_type(location, inner))
    }
    /// Check every variable an inline input-object literal puts inside an argument.
    ///
    /// A filter written out in the document rather than passed whole is where `$team`
    /// hid: it is not a root argument, so the root-argument check above never saw it, and
    /// nothing else here descended into the literal. Linear did, and refused the document.
    fn literal_variables<'a>(
        inputs: &std::collections::HashMap<&str, &'a schema::InputObjectType<'a, String>>,
        variables: &[query::VariableDefinition<'_, String>],
        input_name: &str,
        value: &query::Value<'_, String>,
        path: &str,
    ) {
        let query::Value::Object(fields) = value else {
            return;
        };
        let input = inputs
            .get(input_name)
            .unwrap_or_else(|| panic!("pinned schema lacks input {input_name}"));
        for (key, value) in fields {
            let field = input
                .fields
                .iter()
                .find(|field| field.name == *key)
                .unwrap_or_else(|| panic!("{input_name} lacks field {key}"));
            let path = format!("{path}.{key}");
            match value {
                query::Value::Variable(name) => {
                    let variable = variables
                        .iter()
                        .find(|variable| variable.name == *name)
                        .unwrap_or_else(|| panic!("no ${name} is declared for {path}"));
                    assert!(
                        usable_at(&field.value_type, &variable.var_type),
                        "${name} is declared {:?} and {path} is {:?}: Linear refuses a \
                         variable that is neither the location's type nor its non-null form",
                        variable.var_type,
                        field.value_type,
                    );
                }
                value => {
                    literal_variables(inputs, variables, named(&field.value_type), value, &path)
                }
            }
        }
    }
    fn validate<'a>(
        objects: &std::collections::HashMap<&str, &'a schema::ObjectType<'a, String>>,
        type_name: &str,
        selections: &query::SelectionSet<'_, String>,
    ) {
        let object = objects[type_name];
        for selection in &selections.items {
            let query::Selection::Field(selected) = selection else {
                panic!("no fragments")
            };
            let field = object
                .fields
                .iter()
                .find(|field| field.name == selected.name)
                .unwrap_or_else(|| panic!("{type_name} lacks {}", selected.name));
            for (argument, _) in &selected.arguments {
                assert!(
                    field.arguments.iter().any(|input| input.name == *argument),
                    "{type_name}.{} lacks {argument}",
                    field.name
                );
            }
            if !selected.selection_set.items.is_empty() {
                validate(objects, named(&field.field_type), &selected.selection_set);
            }
        }
    }
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
        (graphql::ISSUE_RELATION_DELETE, true),
        (graphql::PROJECT_RELATION_DELETE, true),
        (graphql::ISSUE_DELETE, true),
        (graphql::PROJECT_DELETE, true),
        (graphql::DOCUMENT_CREATE, true),
        (graphql::DOCUMENT_UPDATE, true),
        (graphql::DOCUMENT_DELETE, true),
    ] {
        let parsed = query::parse_query::<String>(document).unwrap();
        let (selection_set, variables) = match &parsed.definitions[0] {
            query::Definition::Operation(query::OperationDefinition::Query(operation)) => {
                (&operation.selection_set, &operation.variable_definitions)
            }
            query::Definition::Operation(query::OperationDefinition::Mutation(operation)) => {
                (&operation.selection_set, &operation.variable_definitions)
            }
            _ => panic!("production document is an explicit query or mutation"),
        };
        let selection = &selection_set.items[0];
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
        let schema_root = objects[if mutation { "Mutation" } else { "Query" }]
            .fields
            .iter()
            .find(|field| field.name == root.name)
            .unwrap();
        for (argument_name, value) in &root.arguments {
            let query::Value::Variable(variable_name) = value else {
                let argument = schema_root
                    .arguments
                    .iter()
                    .find(|argument| argument.name == *argument_name)
                    .unwrap();
                literal_variables(
                    &inputs,
                    variables,
                    named(&argument.value_type),
                    value,
                    &format!("{}({argument_name}:)", root.name),
                );
                continue;
            };
            let variable = variables
                .iter()
                .find(|variable| variable.name == *variable_name)
                .unwrap();
            let argument = schema_root
                .arguments
                .iter()
                .find(|argument| argument.name == *argument_name)
                .unwrap();
            assert!(
                same_type(&argument.value_type, &variable.var_type),
                "{}.{} variable ${variable_name} type or nullability drifted",
                if mutation { "Mutation" } else { "Query" },
                root.name
            );
        }
        validate(
            &objects,
            if mutation { "Mutation" } else { "Query" },
            selection_set,
        );
    }
}

#[tokio::test]
async fn a_project_this_source_created_is_removed_again_over_real_http() {
    // What makes a copy into Linear atomic: the engine undoes a failed copy's own writes,
    // and a project is one of the two things it can have written. An id naming nothing is
    // the state the caller asked for, so it is answered without a mutation — the same
    // reading `delete_task` gives it.
    let project = |id: &str| serde_json::json!({"project":{"id":id,"name":"One","description":null,"url":null,"createdAt":null,"updatedAt":null,"status":{"name":"Done","type":"completed"},"labels":{"nodes":[]}}});
    let (endpoint, wire) = response_server(vec![
        project("P-GONE"),
        serde_json::json!({"projectDelete":{"success":true}}),
    ]);
    source(&endpoint)
        .delete_project(&"P-GONE".into())
        .await
        .expect("the project this copy created is taken back");
    let read = wire.recv().unwrap();
    assert!(read.contains("project(id:$id)"), "it is read first: {read}");
    let removal = wire.recv().unwrap();
    assert!(
        removal.contains("projectDelete(id:$id)") && removal.contains("P-GONE"),
        "the pinned project delete is what removes it: {removal}"
    );

    let (endpoint, wire) = response_server(vec![serde_json::json!({"project":null})]);
    source(&endpoint)
        .delete_project(&"never-there".into())
        .await
        .expect("an id naming nothing is the state this asks for");
    assert!(wire.recv().unwrap().contains("project(id:$id)"));
    assert!(
        wire.recv_timeout(std::time::Duration::from_millis(200))
            .is_err(),
        "nothing is deleted when there was nothing there"
    );

    let (endpoint, _) = response_server(vec![
        project("P-KEPT"),
        serde_json::json!({"projectDelete":{"success":false}}),
    ]);
    let refusal = source(&endpoint)
        .delete_project(&"P-KEPT".into())
        .await
        .expect_err("a removal Linear did not confirm is not a removal");
    assert!(
        matches!(&refusal, SourceError::Refused { message } if message.contains("projectDelete")),
        "the refusal names the operation that did not succeed: {refusal:?}"
    );
}

#[tokio::test]
async fn writes_create_update_and_route_task_and_project_edges_over_real_http() {
    let empty_page = |root: &str| serde_json::json!({(root):{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}});
    let id_page = |root: &str, id: &str| serde_json::json!({(root):{"nodes":[{"id":id}]}});
    let (endpoint, wire) = response_server(vec![
        id_page("teams", "TEAM"),
        id_page("workflowStates", "STATE"),
        id_page("issueLabels", "LABEL"),
        serde_json::json!({"issueCreate":{"success":true,"issue":{"id":"I-NEW"}}}),
        serde_json::json!({"issue":{"description":null,"relations":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}},"inverseRelations":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}),
        serde_json::json!({"issueRelationCreate":{"success":true,"issueRelation":{"id":"R-I"}}}),
        serde_json::json!({"issues":{"nodes":[],"pageInfo":{"hasNextPage":true,"endCursor":"next"}}}),
        serde_json::json!({"issues":{"nodes":[{"id":"I-FAR","title":"far","description":"\n\n<!-- onetaskgraph.metadata\n{\"onetaskgraph.origin\":\"authored:FAR\"}\n-->","url":null,"createdAt":null,"updatedAt":null,"state":{"name":"Todo","type":"unstarted"},"labels":{"nodes":[]},"project":null}],"pageInfo":{"hasNextPage":false,"endCursor":null}}}),
        id_page("teams", "TEAM"),
        id_page("workflowStates", "STATE"),
        id_page("issueLabels", "LABEL"),
        serde_json::json!({"issueUpdate":{"success":true,"issue":{"id":"I-NEW"}}}),
        serde_json::json!({"issue":{"description":null,"relations":{"nodes":[{"id":"OLD","type":"blocks","relatedIssue":{"id":"OLD-FAR"}}],"pageInfo":{"hasNextPage":false,"endCursor":null}},"inverseRelations":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}),
        serde_json::json!({"issueRelationDelete":{"success":true}}),
        serde_json::json!({"projects":{"nodes":[{"id":"P-FAR","name":"far","description":"<!-- onetaskgraph.metadata\n{\"onetaskgraph.origin\":\"authored:PFAR\"}\n-->","url":null,"createdAt":null,"updatedAt":null,"status":{"name":"Todo","type":"unstarted"},"labels":{"nodes":[]}}],"pageInfo":{"hasNextPage":false,"endCursor":null}}}),
        id_page("teams", "TEAM"),
        serde_json::json!({"projectStatuses":{"nodes":[{"id":"STATUS","name":"Todo"}]}}),
        id_page("projectLabels", "PLABEL"),
        serde_json::json!({"projectCreate":{"success":true,"project":{"id":"P-NEW"}}}),
        serde_json::json!({"project":{"description":null,"relations":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}},"inverseRelations":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}),
        serde_json::json!({"projectRelationCreate":{"success":true,"projectRelation":{"id":"R-P"}}}),
        serde_json::json!({"projects":{"nodes":[{"id":"P-FAR","name":"far","description":"<!-- onetaskgraph.metadata\n{\"onetaskgraph.origin\":\"authored:PFAR\"}\n-->","url":null,"createdAt":null,"updatedAt":null,"status":{"name":"Todo","type":"unstarted"},"labels":{"nodes":[]}}],"pageInfo":{"hasNextPage":false,"endCursor":null}}}),
        id_page("teams", "TEAM"),
        serde_json::json!({"projectStatuses":{"nodes":[{"id":"STATUS","name":"Todo"}]}}),
        id_page("projectLabels", "PLABEL"),
        serde_json::json!({"projectUpdate":{"success":true,"project":{"id":"P-NEW"}}}),
        serde_json::json!({"project":{"description":null,"relations":{"nodes":[{"id":"OLD-P","type":"dependency","relatedProject":{"id":"P-FAR"}}],"pageInfo":{"hasNextPage":true,"endCursor":"next"}},"inverseRelations":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}),
        serde_json::json!({"projectRelationDelete":{"success":true}}),
        serde_json::json!({"project":{"description":null,"relations":{"nodes":[{"id":"OLD-P2","type":"related","relatedProject":{"id":"P-OTHER"}}],"pageInfo":{"hasNextPage":false,"endCursor":null}},"inverseRelations":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}),
        serde_json::json!({"projectRelationDelete":{"success":true}}),
        serde_json::json!({"projectRelationCreate":{"success":true,"projectRelation":{"id":"R-P2"}}}),
    ]);
    let writable = writable_source(&endpoint);
    let task: Task = serde_json::from_value(serde_json::json!({"id":"authored:NEAR","title":"visible task","content":"body","status":{"category":"todo","name":"Todo"},"labels":[{"id":"old","name":"bug","color":null}],"project":null,"repositories":["github.com/acme/work"],"metadata":{"object":{"n":1},"null":null}})).unwrap();
    let missing_team = source("http://127.0.0.1:1")
        .write_task(&ItemWrite {
            target: None,
            item: task.clone(),
            depends_on: Vec::new(),
        })
        .await
        .unwrap_err();
    assert!(format!("{missing_team}").contains("config.team"));
    let (unresolved_endpoint, unresolved_wire) = response_server(vec![empty_page("teams")]);
    let unresolved_team = writable_source(&unresolved_endpoint)
        .write_task(&ItemWrite {
            target: None,
            item: task.clone(),
            depends_on: Vec::new(),
        })
        .await
        .unwrap_err();
    assert!(format!("{unresolved_team}").contains("cannot resolve configured team uniquely"));
    drop(unresolved_wire);
    let native_task = DependencyEdge {
        from: DependencyEndpoint::new("authored:NEAR".into(), ItemKind::Task).unwrap(),
        to: DependencyEndpoint::from_native("I-FAR".into(), ItemKind::Task),
        kind: DependencyKind::Blocks,
    };
    let cross_task = DependencyEdge {
        from: native_task.from.clone(),
        to: DependencyEndpoint::new("elsewhere:P-9".into(), ItemKind::Project).unwrap(),
        kind: DependencyKind::Related,
    };
    assert_eq!(
        writable
            .write_task(&ItemWrite {
                target: None,
                item: task.clone(),
                depends_on: vec![native_task.clone(), cross_task]
            })
            .await
            .unwrap()
            .0,
        "I-NEW"
    );
    let unresolved = DependencyEdge {
        to: DependencyEndpoint::new("missing:FAR".into(), ItemKind::Task).unwrap(),
        ..native_task
    };
    assert_eq!(
        writable
            .write_task(&ItemWrite {
                target: Some("I-NEW".into()),
                item: task,
                depends_on: vec![unresolved]
            })
            .await
            .unwrap()
            .0,
        "I-NEW"
    );

    let project: Project = serde_json::from_value(serde_json::json!({"id":"authored:P","title":"visible project","content":"project body","status":{"category":"todo","name":"Todo"},"labels":[{"id":"old","name":"roadmap","color":null}],"repositories":["github.com/acme/work"],"metadata":{"array":[true,null]}})).unwrap();
    let project_edge = DependencyEdge {
        from: DependencyEndpoint::new("authored:P".into(), ItemKind::Project).unwrap(),
        to: DependencyEndpoint::new("authored:PFAR".into(), ItemKind::Project).unwrap(),
        kind: DependencyKind::Related,
    };
    assert_eq!(
        writable
            .write_project(&ItemWrite {
                target: None,
                item: project.clone(),
                depends_on: vec![project_edge.clone()]
            })
            .await
            .unwrap()
            .0,
        "P-NEW"
    );
    // The second write carries the *ordering* kind, which Linear spells differently at
    // the project level than at the issue level, so both project relation types are
    // written here and both are asserted below.
    let ordering_edge = DependencyEdge {
        kind: DependencyKind::Blocks,
        ..project_edge
    };
    assert_eq!(
        writable
            .write_project(&ItemWrite {
                target: Some("P-NEW".into()),
                item: project,
                depends_on: vec![ordering_edge]
            })
            .await
            .unwrap()
            .0,
        "P-NEW"
    );
    let requests = wire.iter().collect::<Vec<_>>();
    assert!(
        requests
            .iter()
            .any(|request| request.contains(onetaskgraph_linear::graphql::ISSUE_CREATE))
    );
    assert!(
        requests
            .iter()
            .any(|request| request.contains(onetaskgraph_linear::graphql::ISSUE_UPDATE))
    );
    assert!(
        requests
            .iter()
            .any(|request| request.contains("relatedIssueId") && request.contains("I-FAR"))
    );
    assert!(
        requests
            .iter()
            .any(|request| request.contains(onetaskgraph_linear::graphql::PROJECT_CREATE))
    );
    // Linear declares both anchors required on `ProjectRelationCreateInput`, so the whole
    // input object is asserted rather than the far id alone: dropping either anchor is
    // what the live lane failed on, and a substring assertion would not have seen it. The
    // `type` is asserted for the same reason and by the same evidence — `blocks` there is
    // what the live lane was refused for next, with `Argument Validation Error`, and a
    // project dependency is typed `dependency`.
    let relation_inputs = requests
        .iter()
        .filter(|request| request.contains(onetaskgraph_linear::graphql::PROJECT_RELATION_CREATE))
        .map(|request| {
            let body = request
                .split_once("\r\n\r\n")
                .expect("the recorded request carries a body")
                .1;
            serde_json::from_str::<serde_json::Value>(body).expect("the body is JSON")["variables"]
                ["input"]
                .clone()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        relation_inputs,
        vec![
            serde_json::json!({
                "projectId": "P-NEW",
                "relatedProjectId": "P-FAR",
                "type": "related",
                "anchorType": "start",
                "relatedAnchorType": "end",
            }),
            serde_json::json!({
                "projectId": "P-NEW",
                "relatedProjectId": "P-FAR",
                "type": "dependency",
                "anchorType": "start",
                "relatedAnchorType": "end",
            }),
        ],
        "the project relation input drifted from what Linear requires"
    );
    let create = requests
        .iter()
        .find(|request| request.contains(onetaskgraph_linear::graphql::ISSUE_CREATE))
        .unwrap();
    assert!(
        create.contains("visible task")
            && create.contains("onetaskgraph.repositories")
            && create.contains("elsewhere:P-9")
    );
    let unresolved_update = requests
        .iter()
        .find(|request| {
            request.contains(onetaskgraph_linear::graphql::ISSUE_UPDATE)
                && request.contains("missing:FAR")
        })
        .expect("an unresolved same-source origin remains in recorded dependency metadata");
    assert!(unresolved_update.contains("onetaskgraph.depends_on"));
}

#[tokio::test]
async fn a_write_with_no_visible_description_or_metadata_sends_null_over_real_http() {
    let page = |root: &str, nodes: serde_json::Value| serde_json::json!({(root):{"nodes":nodes,"pageInfo":{"hasNextPage":false,"endCursor":null}}});
    let (endpoint, wire) = response_server(vec![
        page("teams", serde_json::json!([{"id":"TEAM"}])),
        page("workflowStates", serde_json::json!([{"id":"STATE"}])),
        serde_json::json!({"issueCreate":{"success":true,"issue":{"id":"NEW"}}}),
        serde_json::json!({"issue":{"description":null,"relations":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}},"inverseRelations":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}),
    ]);
    let item = serde_json::from_value::<Task>(serde_json::json!({
        "id":"from:T", "title":"task", "content":null,
        "status":{"category":"todo","name":"Todo"}, "labels":[], "project":null,
        "repositories":[], "metadata":{}
    }))
    .unwrap();
    writable_source(&endpoint)
        .write_task(&ItemWrite {
            target: None,
            item,
            depends_on: Vec::new(),
        })
        .await
        .unwrap();
    let requests = wire.iter().collect::<Vec<_>>();
    let create = requests
        .iter()
        .find(|request| request.contains(onetaskgraph_linear::graphql::ISSUE_CREATE))
        .unwrap();
    assert!(create.contains("\"description\":null"), "{create}");
}

#[tokio::test]
async fn write_failures_from_lookups_and_mutation_payloads_cross_the_http_boundary() {
    let task = || {
        serde_json::from_value::<Task>(serde_json::json!({"id":"from:T","title":"task","content":null,"status":{"category":"todo","name":"Todo"},"labels":[],"project":null,"repositories":[],"metadata":{}})).unwrap()
    };
    let project = || {
        serde_json::from_value::<Project>(serde_json::json!({"id":"from:P","title":"project","content":null,"status":{"category":"todo","name":"Todo"},"labels":[],"repositories":[],"metadata":{}})).unwrap()
    };
    let page = |root: &str, nodes: serde_json::Value| serde_json::json!({(root):{"nodes":nodes,"pageInfo":{"hasNextPage":false,"endCursor":null}}});
    let (endpoint, wire) = response_server(vec![serde_json::json!({"teams":{}})]);
    let error = writable_source(&endpoint)
        .write_task(&ItemWrite {
            target: None,
            item: task(),
            depends_on: Vec::new(),
        })
        .await
        .unwrap_err();
    assert!(format!("{error}").contains("missing teams.nodes"));
    drop(wire);
    let (endpoint, wire) = response_server(vec![page("teams", serde_json::json!([{"id":""}]))]);
    assert!(
        format!(
            "{}",
            writable_source(&endpoint)
                .write_task(&ItemWrite {
                    target: None,
                    item: task(),
                    depends_on: Vec::new()
                })
                .await
                .unwrap_err()
        )
        .contains("empty backend id")
    );
    drop(wire);
    let (endpoint, wire) = response_server(vec![
        page("teams", serde_json::json!([{"id":"TEAM"}])),
        page("workflowStates", serde_json::json!([{"id":"STATE"}])),
        serde_json::json!({"issueCreate":{"success":true,"issue":{"id":""}}}),
    ]);
    assert!(
        format!(
            "{}",
            writable_source(&endpoint)
                .write_task(&ItemWrite {
                    target: None,
                    item: task(),
                    depends_on: Vec::new()
                })
                .await
                .unwrap_err()
        )
        .contains("empty backend id")
    );
    drop(wire);
    let (endpoint, wire) = response_server(vec![
        page("teams", serde_json::json!([{"id":"TEAM"}])),
        page(
            "projectStatuses",
            serde_json::json!([{"id":"STATUS","name":"Todo"}]),
        ),
        serde_json::json!({"projectCreate":{"success":true,"project":{"id":""}}}),
    ]);
    assert!(
        format!(
            "{}",
            writable_source(&endpoint)
                .write_project(&ItemWrite {
                    target: None,
                    item: project(),
                    depends_on: Vec::new()
                })
                .await
                .unwrap_err()
        )
        .contains("empty backend id")
    );
    drop(wire);
    for (responses, item, expected) in [
        (
            vec![
                page("teams", serde_json::json!([{"id":"TEAM"}])),
                page("workflowStates", serde_json::json!([])),
            ],
            task(),
            "workflow state",
        ),
        (
            vec![
                page("teams", serde_json::json!([{"id":"TEAM"}])),
                page("workflowStates", serde_json::json!([{"id":"STATE"}])),
                serde_json::json!({"issueCreate":{"success":false,"issue":null}}),
            ],
            task(),
            "unsuccessful",
        ),
        (
            vec![
                page("teams", serde_json::json!([{"id":"TEAM"}])),
                page("workflowStates", serde_json::json!([{"id":"STATE"}])),
                serde_json::json!({"issueCreate":{"success":true}}),
            ],
            task(),
            "missing issueCreate.issue",
        ),
        (
            vec![
                page("teams", serde_json::json!([{"id":"TEAM"}])),
                page("workflowStates", serde_json::json!([{"id":"STATE"}])),
                serde_json::json!({"issueCreate":{"issue":{"id":"NEW"}}}),
            ],
            task(),
            "missing boolean issueCreate.success",
        ),
    ] {
        let (endpoint, wire) = response_server(responses);
        let error = writable_source(&endpoint)
            .write_task(&ItemWrite {
                target: None,
                item,
                depends_on: Vec::new(),
            })
            .await
            .unwrap_err();
        assert!(format!("{error}").contains(expected), "{error}");
        drop(wire);
    }
    let mut labeled = task();
    labeled.labels.push(onetaskgraph_plugin_api::Label {
        id: "old".into(),
        name: "missing".into(),
        color: None,
    });
    let (endpoint, wire) = response_server(vec![
        page("teams", serde_json::json!([{"id":"TEAM"}])),
        page("workflowStates", serde_json::json!([{"id":"STATE"}])),
        page("issueLabels", serde_json::json!([])),
    ]);
    let error = writable_source(&endpoint)
        .write_task(&ItemWrite {
            target: None,
            item: labeled,
            depends_on: Vec::new(),
        })
        .await
        .unwrap_err();
    assert!(format!("{error}").contains("label \"missing\""));
    drop(wire);
    let (endpoint, wire) = response_server(vec![
        page("teams", serde_json::json!([{"id":"TEAM"}])),
        page("projectStatuses", serde_json::json!([])),
    ]);
    let error = writable_source(&endpoint)
        .write_project(&ItemWrite {
            target: None,
            item: project(),
            depends_on: Vec::new(),
        })
        .await
        .unwrap_err();
    assert!(format!("{error}").contains("project status"));
    drop(wire);
    let relation = DependencyEdge {
        from: DependencyEndpoint::from_native("T".into(), ItemKind::Task),
        to: DependencyEndpoint::from_native("FAR".into(), ItemKind::Task),
        kind: DependencyKind::Blocks,
    };
    let (endpoint, wire) = response_server(vec![
        page("teams", serde_json::json!([{"id":"TEAM"}])),
        page("workflowStates", serde_json::json!([{"id":"STATE"}])),
        serde_json::json!({"issueCreate":{"success":true,"issue":{"id":"NEW"}}}),
        serde_json::json!({"issue":{"description":null,"relations":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}},"inverseRelations":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}),
        serde_json::json!({"issueRelationCreate":{"success":false,"issueRelation":null}}),
    ]);
    let error = writable_source(&endpoint)
        .write_task(&ItemWrite {
            target: None,
            item: task(),
            depends_on: vec![relation],
        })
        .await
        .unwrap_err();
    assert!(format!("{error}").contains("issueRelationCreate"));
    drop(wire);
    let project_relation = DependencyEdge {
        from: DependencyEndpoint::from_native("P".into(), ItemKind::Project),
        to: DependencyEndpoint::from_native("FAR".into(), ItemKind::Project),
        kind: DependencyKind::Blocks,
    };
    let (endpoint, wire) = response_server(vec![
        page("teams", serde_json::json!([{"id":"TEAM"}])),
        page(
            "projectStatuses",
            serde_json::json!([{"id":"STATUS","name":"Todo"}]),
        ),
        serde_json::json!({"projectCreate":{"success":true,"project":{"id":"NEW"}}}),
        serde_json::json!({"project":{"relations":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}),
        serde_json::json!({"projectRelationCreate":{"success":true,"projectRelation":{"id":""}}}),
    ]);
    let error = writable_source(&endpoint)
        .write_project(&ItemWrite {
            target: None,
            item: project(),
            depends_on: vec![project_relation],
        })
        .await
        .unwrap_err();
    assert!(format!("{error}").contains("empty backend id"), "{error}");
    drop(wire);
    let (endpoint, wire) = response_server(vec![
        page("teams", serde_json::json!([{"id":"TEAM"}])),
        page(
            "projectStatuses",
            serde_json::json!([{"id":"STATUS","name":"Todo"}]),
        ),
        serde_json::json!({"projectUpdate":{"success":true,"project":{"id":"P"}}}),
        serde_json::json!({"project":{"relations":{"nodes":[{"id":"R"}],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}),
        serde_json::json!({"projectRelationDelete":{"success":false}}),
    ]);
    let error = writable_source(&endpoint)
        .write_project(&ItemWrite {
            target: Some("P".into()),
            item: project(),
            depends_on: Vec::new(),
        })
        .await
        .unwrap_err();
    assert!(
        format!("{error}").contains("projectRelationDelete"),
        "{error}"
    );
    drop(wire);
    for (relation_response, expected) in [
        (serde_json::json!({}), "missing relation item"),
        (serde_json::json!({"issue":{}}), "missing relations"),
        (
            serde_json::json!({"issue":{"relations":{"nodes":7,"pageInfo":{"hasNextPage":false,"endCursor":null}}}}),
            "missing relations.nodes",
        ),
        (
            serde_json::json!({"issue":{"relations":{"nodes":[{"id":""}],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}),
            "empty backend id",
        ),
    ] {
        let (endpoint, wire) = response_server(vec![
            page("teams", serde_json::json!([{"id":"TEAM"}])),
            page("workflowStates", serde_json::json!([{"id":"STATE"}])),
            serde_json::json!({"issueCreate":{"success":true,"issue":{"id":"NEW"}}}),
            relation_response,
        ]);
        let error = writable_source(&endpoint)
            .write_task(&ItemWrite {
                target: None,
                item: task(),
                depends_on: Vec::new(),
            })
            .await
            .unwrap_err();
        assert!(format!("{error}").contains(expected), "{error}");
        drop(wire);
    }
    let (endpoint, wire) = response_server(vec![
        page("teams", serde_json::json!([{"id":"TEAM"}])),
        page(
            "projectStatuses",
            serde_json::json!([{"id":"STATUS","name":"Todo"}]),
        ),
        serde_json::json!({"projectCreate":{"success":true}}),
    ]);
    let error = writable_source(&endpoint)
        .write_project(&ItemWrite {
            target: None,
            item: project(),
            depends_on: Vec::new(),
        })
        .await
        .unwrap_err();
    assert!(format!("{error}").contains("missing projectCreate.project"));
    drop(wire);
    for response in [
        serde_json::json!({"issueUpdate":{"success":false,"issue":null}}),
        serde_json::json!({"issueUpdate":{"success":true}}),
    ] {
        let (endpoint, wire) = response_server(vec![
            page("teams", serde_json::json!([{"id":"TEAM"}])),
            page("workflowStates", serde_json::json!([{"id":"STATE"}])),
            response,
        ]);
        assert!(
            writable_source(&endpoint)
                .write_task(&ItemWrite {
                    target: Some("I".into()),
                    item: task(),
                    depends_on: Vec::new()
                })
                .await
                .is_err()
        );
        drop(wire);
    }
    for response in [
        serde_json::json!({"projectUpdate":{"success":false,"project":null}}),
        serde_json::json!({"projectUpdate":{"success":true}}),
    ] {
        let (endpoint, wire) = response_server(vec![
            page("teams", serde_json::json!([{"id":"TEAM"}])),
            page(
                "projectStatuses",
                serde_json::json!([{"id":"STATUS","name":"Todo"}]),
            ),
            response,
        ]);
        assert!(
            writable_source(&endpoint)
                .write_project(&ItemWrite {
                    target: Some("P".into()),
                    item: project(),
                    depends_on: Vec::new()
                })
                .await
                .is_err()
        );
        drop(wire);
    }
    let (endpoint, wire) = response_server(vec![
        page("teams", serde_json::json!([{"id":"TEAM"}])),
        page("workflowStates", serde_json::json!([{"id":"STATE"}])),
        serde_json::json!({"issueUpdate":{"success":true,"issue":{"id":"I"}}}),
        serde_json::json!({"issue":{"relations":{"nodes":[{"id":"R"}],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}),
        serde_json::json!({"issueRelationDelete":{"success":false}}),
    ]);
    assert!(
        writable_source(&endpoint)
            .write_task(&ItemWrite {
                target: Some("I".into()),
                item: task(),
                depends_on: Vec::new()
            })
            .await
            .is_err()
    );
    drop(wire);
}

/// Linear's `projectStatuses` accepts no `filter`, so the plugin asks for the whole
/// connection and matches the display name itself.
///
/// This is the one lookup of the five that works that way. It is here because sending the
/// filter Linear does not take is a `GRAPHQL_VALIDATION_FAILED` refusal of the entire
/// document, which no amount of correct handling further down recovers from.
#[tokio::test]
async fn a_project_status_is_matched_locally_because_linear_narrows_that_connection_for_nobody() {
    let project = || {
        serde_json::from_value::<Project>(serde_json::json!({"id":"authored:P","title":"a project","content":null,"status":{"category":"todo","name":"Todo"},"labels":[],"repositories":[],"metadata":{}})).unwrap()
    };
    let teams = serde_json::json!({"teams":{"nodes":[{"id":"TEAM"}]}});
    let statuses =
        |nodes: serde_json::Value| serde_json::json!({"projectStatuses":{"nodes":nodes}});

    let (endpoint, wire) = response_server(vec![
        teams.clone(),
        statuses(serde_json::json!([
            {"id":"S-BACKLOG","name":"Backlog"},
            {"id":"S-TODO","name":"todo"},
            {"id":"S-DONE","name":"Done"}
        ])),
        serde_json::json!({"projectCreate":{"success":true,"project":{"id":"P-NEW"}}}),
        serde_json::json!({"project":{"description":null,"relations":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}},"inverseRelations":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}),
    ]);
    writable_source(&endpoint)
        .write_project(&ItemWrite {
            target: None,
            item: project(),
            depends_on: Vec::new(),
        })
        .await
        .expect("the one status answering to the name resolves it");
    let _team = wire.recv().unwrap();
    let asked = wire.recv().unwrap();
    assert!(
        !asked.contains("filter"),
        "the request Linear refuses outright is one naming a filter: {asked}"
    );
    let created = wire.recv().unwrap();
    assert!(
        created.contains("S-TODO"),
        "the write carries the matched status's own id, not its name: {created}"
    );
    drop(wire);

    let (endpoint, wire) = response_server(vec![
        teams.clone(),
        statuses(serde_json::json!([
            {"id":"S-ONE","name":"Todo"},
            {"id":"S-TWO","name":"TODO"}
        ])),
    ]);
    let ambiguous = writable_source(&endpoint)
        .write_project(&ItemWrite {
            target: None,
            item: project(),
            depends_on: Vec::new(),
        })
        .await
        .unwrap_err();
    assert!(
        format!("{ambiguous}").contains("project status \"Todo\""),
        "a name two statuses answer to under the same case-insensitive comparison Linear \
         applied server-side is refused by name: {ambiguous}"
    );
    drop(wire);

    let (endpoint, wire) = response_server(vec![
        teams,
        statuses(serde_json::json!([{"id":"S-DONE","name":"Done"}])),
    ]);
    let absent = writable_source(&endpoint)
        .write_project(&ItemWrite {
            target: None,
            item: project(),
            depends_on: Vec::new(),
        })
        .await
        .unwrap_err();
    assert!(
        format!("{absent}").contains("project status \"Todo\""),
        "a connection holding no such name is refused by name too: {absent}"
    );
    drop(wire);

    // A status this source cannot read the name of is malformed data from Linear, not a
    // status that failed to match: reporting it as the latter would blame the caller for
    // naming a status that is in fact right there.
    for unreadable in [
        serde_json::json!([{"id":"S-TODO","name":"Todo"},{"id":"S-ODD"}]),
        serde_json::json!([{"id":"S-TODO","name":"Todo"},{"id":"S-ODD","name":7}]),
    ] {
        // The create and its relation read are here so that dropping the unreadable node
        // would carry the write all the way through: the refusal below is then the source
        // rejecting the data, not the fixture running out of answers.
        let (endpoint, wire) = response_server(vec![
            serde_json::json!({"teams":{"nodes":[{"id":"TEAM"}]}}),
            statuses(unreadable.clone()),
            serde_json::json!({"projectCreate":{"success":true,"project":{"id":"P-NEW"}}}),
            serde_json::json!({"project":{"description":null,"relations":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}},"inverseRelations":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}),
        ]);
        let malformed = writable_source(&endpoint)
            .write_project(&ItemWrite {
                target: None,
                item: project(),
                depends_on: Vec::new(),
            })
            .await
            .unwrap_err();
        assert!(
            matches!(&malformed, SourceError::Malformed { message } if message.contains("name")),
            "an unreadable status name is malformed rather than a nonmatch, for {unreadable}: {malformed:?}"
        );
        drop(wire);
    }
}

#[tokio::test]
async fn replacing_more_than_one_full_relation_page_deletes_every_existing_edge() {
    let task: Task = serde_json::from_value(serde_json::json!({"id":"from:T","title":"task","content":null,"status":{"category":"todo","name":"Todo"},"labels":[],"project":null,"repositories":[],"metadata":{}})).unwrap();
    let page = |nodes: Vec<serde_json::Value>, more: bool| serde_json::json!({"issue":{"relations":{"nodes":nodes,"pageInfo":{"hasNextPage":more,"endCursor":if more {Some("next")} else {None}}}}});
    let mut responses = vec![
        serde_json::json!({"teams":{"nodes":[{"id":"TEAM"}]}}),
        serde_json::json!({"workflowStates":{"nodes":[{"id":"STATE"}]}}),
        serde_json::json!({"issueUpdate":{"success":true,"issue":{"id":"I"}}}),
    ];
    responses.push(page(
        (0..250)
            .map(|index| serde_json::json!({"id":format!("R{index}")}))
            .collect(),
        true,
    ));
    responses.extend((0..250).map(|_| serde_json::json!({"issueRelationDelete":{"success":true}})));
    responses.push(page(vec![serde_json::json!({"id":"R250"})], false));
    responses.push(serde_json::json!({"issueRelationDelete":{"success":true}}));
    let (endpoint, wire) = response_server(responses);
    writable_source(&endpoint)
        .write_task(&ItemWrite {
            target: Some("I".into()),
            item: task,
            depends_on: Vec::new(),
        })
        .await
        .unwrap();
    assert_eq!(
        wire.iter()
            .filter(|request| request.contains(onetaskgraph_linear::graphql::ISSUE_RELATION_DELETE))
            .count(),
        251
    );
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
    // Where it is: the issue's own Linear page, as a link. It sits beside the `url` field
    // rather than replacing it, which is what lets a reader branch on the shape.
    assert_eq!(
        page.items[0].location,
        Some(Location::Url("https://linear.app/acme/issue/ENG-1".into()))
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
    assert_eq!(
        projects.items[0].location,
        Some(Location::Url("https://linear.app/acme/project/p1".into())),
        "a project says where it is, on the terms an issue and a document do"
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

/// A status Linear will not run a document under says which part of it Linear refused.
///
/// Linear answers a rejected document with 400 and its error envelope in the body, so a
/// refusal reported as the status alone leaves nothing to act on. A long body is cut
/// rather than carried whole.
#[tokio::test]
async fn a_status_linear_refuses_under_carries_what_linear_said() {
    let (endpoint, _) = server(
        "400 Bad Request",
        "",
        r#"{"errors":[{"message":"Argument 'statusId' on InputObject 'ProjectCreateInput' has an invalid value"}]}"#,
    );
    let refused = source(&endpoint).health().await.unwrap_err();
    let SourceError::Unavailable { message } = refused else {
        panic!("a refused status is an unavailable source: {refused:?}");
    };
    assert!(
        message.contains("HTTP 400 Bad Request"),
        "the status is still named: {message}"
    );
    assert!(
        message.contains("statusId") && message.contains("ProjectCreateInput"),
        "and Linear's own words come with it: {message}"
    );

    // An answering proxy chooses the body, and this message is written to a terminal, so a
    // body cannot carry an escape sequence or a newline into it.
    let (endpoint, _) = server(
        "502 Bad Gateway",
        "",
        "<html>\r\n\u{1b}[2J\u{1b}[HTaken over\n\tby a proxy</html>",
    );
    let refused = source(&endpoint).health().await.unwrap_err();
    let SourceError::Unavailable { message } = refused else {
        panic!("a refused status is an unavailable source: {refused:?}");
    };
    assert!(
        !message.chars().any(char::is_control),
        "no control character reaches the message: {message:?}"
    );
    assert!(
        message.ends_with("<html> [2J [HTaken over by a proxy</html>"),
        "what the proxy said stays readable, with the escapes' introducer gone rather \
         than their text guessed at: {message}"
    );

    // And whatever it answers with, the message stays a message.
    let page = "x".repeat(5000);
    let (endpoint, _) = server("502 Bad Gateway", "", page);
    let refused = source(&endpoint).health().await.unwrap_err();
    let SourceError::Unavailable { message } = refused else {
        panic!("a refused status is an unavailable source: {refused:?}");
    };
    assert!(
        message.len() < 600 && message.ends_with('\u{2026}'),
        "a body that is a page is cut and marked: {} characters",
        message.chars().count()
    );
}

#[tokio::test]
async fn rate_limit_carries_retry_hint() {
    let (endpoint, _) = server("429 Too Many Requests", "Retry-After: 17\r\n", r#"{}"#);
    assert_eq!(
        source(&endpoint).health().await.unwrap_err(),
        SourceError::RateLimited {
            retry_after_seconds: Some(17),
            message: None,
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
            retry_after_seconds: Some(23),
            message: None,
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
    // A refusal carries Linear's `extensions` as well as its `message`, because the
    // message alone is a category name: the live project-relation write was refused with
    // the bare phrase `Argument Validation Error`, which names neither the field nor the
    // value, while `extensions` says which. So the whole message is asserted rather than
    // its first clause.
    let (endpoint, _) = server(
        "200 OK",
        "",
        r#"{"errors":[{"message":"ordinary refusal","extensions":{"code":"BAD"}}]}"#,
    );
    assert!(
        matches!(source(&endpoint).health().await.unwrap_err(), SourceError::Refused { ref message } if message == r#"ordinary refusal: {"code":"BAD"}"#)
    );
    // Extensions Linear sends without a `code` are still carried, and are still a refusal:
    // typing them would fail the whole envelope's deserialization and report a refusal
    // this source could have explained as an unexplained malformed response instead.
    let (endpoint, _) = server(
        "200 OK",
        "",
        r#"{"errors":[{"message":"Argument Validation Error","extensions":{"exception":{"validationErrors":[{"property":"type"}]}}}]}"#,
    );
    assert!(
        matches!(source(&endpoint).health().await.unwrap_err(), SourceError::Refused { ref message } if message.contains("Argument Validation Error") && message.contains("\"property\":\"type\""))
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
        // Linear's two roots do not share a relation vocabulary: a project dependency is
        // typed `dependency` and an issue's is `blocks`.
        let ordering = if projects { "dependency" } else { "blocks" };
        let body = format!(
            r#"{{"data":{{"{root}":{{"description":null,"relations":{{"nodes":[{{"type":"{ordering}","{related}":{{"id":"other"}}}}],"pageInfo":{{"hasNextPage":true,"endCursor":"next-edge"}}}},"inverseRelations":{{"nodes":[],"pageInfo":{{"hasNextPage":false,"endCursor":null}}}}}}}}}}"#
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
                    retry_after_seconds: Some(9),
                    message: None,
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

/// `Draft` narrows to no Linear workflow-state type, exactly as `Unknown` does.
///
/// Linear's workflow states are `triage`, `backlog`, `unstarted`, `started`, `completed`
/// and `canceled`; none of them is a draft. Asserted on the GraphQL this source really
/// sends rather than on the mapping table, because a state name invented here would be
/// a filter Linear rejects — and a state name borrowed from a *neighbouring* category
/// would silently answer a draft query with that category's issues.
#[tokio::test]
async fn a_draft_filter_names_no_linear_workflow_state_the_way_an_unknown_one_does() {
    let empty =
        r#"{"data":{"issues":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}"#;
    let mut sent = Vec::new();
    for category in [StatusCategory::Draft, StatusCategory::Unknown] {
        let (endpoint, wire) = server("200 OK", "", empty);
        let page = source(&endpoint)
            .query_tasks(
                &TaskQuery {
                    statuses: vec![category],
                    ..Default::default()
                },
                &PageRequest {
                    cursor: None,
                    limit: 10,
                },
            )
            .await
            .expect("Linear answers");
        assert!(page.items.is_empty());
        sent.push(wire.recv().expect("the request reached the server"));
    }

    let draft = &sent[0];
    assert!(
        draft.contains(r#"{"state":{"type":{"in":[]}}}"#),
        "a draft filter names no workflow-state type: {draft}"
    );
    for state in [
        "triage",
        "backlog",
        "unstarted",
        "started",
        "completed",
        "canceled",
    ] {
        assert!(
            !draft.contains(&format!("\"{state}\"")),
            "a draft filter must not borrow the `{state}` workflow state: {draft}"
        );
    }

    // The two requests differ only in nothing: `Unknown` already narrows this way, and
    // `Draft` joining it is what makes the pair the same query.
    assert_eq!(
        draft.split("\r\n\r\n").nth(1),
        sent[1].split("\r\n\r\n").nth(1),
        "draft and unknown send the same filter"
    );
}

#[tokio::test]
async fn documents_use_real_http_parse_mapping_paging_and_report_their_linear_address() {
    let body = include_str!("fixtures/documents.json");
    let (endpoint, request) = server("200 OK", "", body);
    let page = source(&endpoint)
        .query_documents(
            &DocumentQuery::default(),
            &PageRequest {
                cursor: None,
                limit: 3,
            },
        )
        .await
        .expect("the fixture documents read");

    assert_eq!(page.items[0].title, "Fixture design note");
    assert_eq!(page.items[0].content.as_deref(), Some("Recorded body"));
    assert_eq!(page.items[0].project.as_ref().unwrap().0, "p1");
    assert_eq!(
        page.items[0].metadata["caller.number"],
        serde_json::json!(7),
        "a caller's own key keeps its JSON type through the slot"
    );
    assert_eq!(
        page.items[0].repositories[0].as_str(),
        "github.com/acme/work"
    );
    assert_eq!(
        page.items[0].created_at.unwrap().to_rfc3339(),
        "2026-08-01T12:00:00+00:00"
    );
    assert_eq!(
        page.items[0].updated_at.unwrap().to_rfc3339(),
        "2026-08-02T12:00:00+00:00"
    );
    // Where it is: the document's own Linear page, as a link rather than a path, beside
    // the `url` field it does not replace.
    assert_eq!(
        page.items[0].url.as_deref(),
        Some("https://linear.app/acme/document/fixture-design-note-aaaaaaaaaaaa")
    );
    assert_eq!(
        page.items[0].location,
        Some(Location::Url(
            "https://linear.app/acme/document/fixture-design-note-aaaaaaaaaaaa".into()
        ))
    );
    // Linear's own document type has no labels, so this source reports none.
    assert!(page.items[0].labels.is_empty());
    assert_eq!(
        page.items[1].project.as_ref().expect("a second project").0,
        "p2",
        "two projects, so a predicate applied and one dropped are different answers"
    );
    assert_eq!(page.items[2].project, None, "a document in no project");
    assert_eq!(page.next.unwrap().0, "next-1");

    let wire = request.recv().unwrap();
    assert!(wire.contains("documents(first:$first"), "{wire}");
    assert!(wire.contains("fixture-key"), "{wire}");
}

#[tokio::test]
async fn a_document_read_pushes_down_a_project_and_applies_orphans_and_labels_itself() {
    // The two predicates Linear cannot be asked for are still applied, over a page this
    // source fetched: `DocumentFilter.project` carries no `null:` member, and a Linear
    // document carries no label at all.
    // One page, holding exactly what Linear would have returned for the filter under test:
    // the project predicate is the one this source pushes down, so its page is narrowed,
    // and the orphan and label predicates are the ones it applies to a page of everything.
    let page = |kept: &[&str]| {
        let mut body: serde_json::Value =
            serde_json::from_str(include_str!("fixtures/documents.json")).unwrap();
        let nodes = body["data"]["documents"]["nodes"]
            .as_array()
            .expect("the fixture documents")
            .iter()
            .filter(|node| kept.contains(&node["id"].as_str().unwrap_or_default()))
            .cloned()
            .collect::<Vec<_>>();
        body["data"]["documents"]["nodes"] = serde_json::Value::Array(nodes);
        body["data"]["documents"]["pageInfo"] =
            serde_json::json!({"hasNextPage": false, "endCursor": null});
        body.to_string()
    };

    let (endpoint, request) = server("200 OK", "", page(&["d1"]));
    let narrowed = source(&endpoint)
        .query_documents(
            &DocumentQuery {
                project: ProjectFilter::Is("p1".into()),
                ..Default::default()
            },
            &PageRequest {
                cursor: None,
                limit: 5,
            },
        )
        .await
        .expect("a document read narrowed to one project");
    assert_eq!(narrowed.items.len(), 1);
    assert_eq!(narrowed.items[0].id.0, "d1");
    let wire = request.recv().unwrap();
    assert!(
        wire.contains(r#""project":{"id":{"eq":"p1"}}"#),
        "the project predicate is pushed into the documents filter: {wire}"
    );

    let (endpoint, request) = server("200 OK", "", page(&["d1", "d2", "d3"]));
    let orphans = source(&endpoint)
        .query_documents(
            &DocumentQuery {
                project: ProjectFilter::Orphans,
                ..Default::default()
            },
            &PageRequest {
                cursor: None,
                limit: 5,
            },
        )
        .await
        .expect("a document read narrowed to the orphans");
    assert_eq!(
        orphans
            .items
            .iter()
            .map(|document| document.id.0.as_str())
            .collect::<Vec<_>>(),
        vec!["d3"],
        "the document in no project, kept by this source rather than by Linear"
    );
    let wire = request.recv().unwrap();
    assert!(
        !wire.contains(r#""null""#),
        "Linear is never asked for a predicate its DocumentFilter has no member for: {wire}"
    );

    let (endpoint, _) = server("200 OK", "", page(&["d1", "d2", "d3"]));
    let demanded = source(&endpoint)
        .query_documents(
            &DocumentQuery {
                labels: LabelFilter {
                    any_of: vec!["bug".into()],
                    ..Default::default()
                },
                ..Default::default()
            },
            &PageRequest {
                cursor: None,
                limit: 5,
            },
        )
        .await
        .expect("a document read demanding a label");
    assert!(
        demanded.items.is_empty(),
        "no Linear document carries a label, so a query demanding one keeps nothing"
    );

    let (endpoint, _) = server("200 OK", "", page(&["d1", "d2", "d3"]));
    let excluded = source(&endpoint)
        .query_documents(
            &DocumentQuery {
                labels: LabelFilter {
                    none_of: vec!["bug".into()],
                    ..Default::default()
                },
                ..Default::default()
            },
            &PageRequest {
                cursor: None,
                limit: 5,
            },
        )
        .await
        .expect("a document read excluding a label");
    assert_eq!(
        excluded.items.len(),
        3,
        "and a query excluding one keeps every document, rather than narrowing"
    );
}

#[tokio::test]
async fn a_document_walk_asks_only_for_what_is_still_owed_and_never_returns_more() {
    // A page this source narrowed itself is short, so the walk goes back for the rest —
    // and asks for exactly the remainder, which is what keeps a caller's limit a limit.
    let node = |id: &str, project: serde_json::Value| {
        serde_json::json!({"id":id,"title":format!("Note {id}"),"content":null,
            "url":format!("https://linear.app/acme/document/{id}"),
            "createdAt":null,"updatedAt":null,"project":project})
    };
    let (endpoint, request) = response_server(vec![
        serde_json::json!({"documents":{
            "nodes":[node("d1", serde_json::json!({"id":"p1"})), node("d2", serde_json::Value::Null)],
            "pageInfo":{"hasNextPage":true,"endCursor":"c1"}}}),
        serde_json::json!({"documents":{
            "nodes":[node("d3", serde_json::Value::Null)],
            "pageInfo":{"hasNextPage":true,"endCursor":"c2"}}}),
    ]);
    let page = source(&endpoint)
        .query_documents(
            &DocumentQuery {
                project: ProjectFilter::Orphans,
                ..Default::default()
            },
            &PageRequest {
                cursor: None,
                limit: 2,
            },
        )
        .await
        .expect("the walk fills the page it was asked for");

    assert_eq!(
        page.items
            .iter()
            .map(|d| d.id.0.as_str())
            .collect::<Vec<_>>(),
        vec!["d2", "d3"]
    );
    assert_eq!(
        page.next.expect("more to walk").0,
        "c2",
        "and resumes where Linear left off rather than where this source did"
    );
    assert!(request.recv().unwrap().contains("\"first\":2"));
    assert!(
        request.recv().unwrap().contains("\"first\":1"),
        "the second request asks only for the one document still owed"
    );
}

#[tokio::test]
async fn one_document_is_shown_by_its_id_and_an_unknown_one_is_no_document() {
    let (endpoint, request) = response_server(vec![
        serde_json::json!({"document":{"id":"d1","title":"Fixture design note","content":"Body",
            "url":"https://linear.app/acme/document/d1","createdAt":null,"updatedAt":null,
            "project":{"id":"p1"}}}),
        serde_json::json!({ "document": serde_json::Value::Null }),
    ]);
    let source = source(&endpoint);
    let shown = source
        .get_document(&"d1".into())
        .await
        .expect("the document reads")
        .expect("and is there");
    assert_eq!(shown.title, "Fixture design note");
    assert_eq!(
        shown.location,
        Some(Location::Url("https://linear.app/acme/document/d1".into()))
    );
    assert!(request.recv().unwrap().contains("document(id:$id)"));

    assert!(
        source
            .get_document(&"never-there".into())
            .await
            .expect("an id naming nothing is an answer")
            .is_none()
    );
}

#[tokio::test]
async fn a_document_is_created_updated_and_removed_again_over_real_http() {
    let document = |id: &str| {
        serde_json::json!({"document":{"id":id,"title":"Design","content":"Body",
            "url":format!("https://linear.app/acme/document/{id}"),
            "createdAt":null,"updatedAt":null,"project":{"id":"p1"}}})
    };
    let written = |title: &str, content: Option<&str>, project: Option<&str>| ItemWrite {
        target: None,
        item: Document {
            id: "ignored".into(),
            title: title.into(),
            content: content.map(str::to_owned),
            project: project.map(Into::into),
            labels: Vec::new(),
            url: None,
            location: None,
            created_at: None,
            updated_at: None,
            metadata: [("caller.count".to_owned(), serde_json::json!(3))]
                .into_iter()
                .collect(),
            repositories: vec![
                onetaskgraph_plugin_api::Repository::try_from("github.com/acme/work".to_owned())
                    .expect("an origin"),
            ],
        },
        depends_on: Vec::new(),
    };

    // Created into a project: the team is not asked for, because the project is the home.
    let (endpoint, wire) = response_server(vec![
        serde_json::json!({"documentCreate":{"success":true,"document":{"id":"D-NEW"}}}),
    ]);
    let created = writable_source(&endpoint)
        .write_document(&written("Design", Some("Body"), Some("p1")))
        .await
        .expect("the document is created");
    assert_eq!(created.0, "D-NEW");
    let request = wire.recv().unwrap();
    assert!(
        request.contains("documentCreate(input:$input)"),
        "{request}"
    );
    assert!(
        request.contains("onetaskgraph.metadata"),
        "the caller's metadata goes back into this source's own slot: {request}"
    );
    assert!(
        request.contains("caller.count") && request.contains("onetaskgraph.repositories"),
        "{request}"
    );
    assert!(
        !request.contains("teamId"),
        "a document filed under a project has a home already: {request}"
    );

    // Created under no project: the configured team is what gives it one.
    let (endpoint, wire) = response_server(vec![
        serde_json::json!({"teams":{"nodes":[{"id":"TEAM"}]}}),
        serde_json::json!({"documentCreate":{"success":true,"document":{"id":"D-LOOSE"}}}),
    ]);
    writable_source(&endpoint)
        .write_document(&written("Loose", None, None))
        .await
        .expect("the orphan document is created");
    assert!(wire.recv().unwrap().contains("teams(filter:"));
    let request = wire.recv().unwrap();
    assert!(request.contains(r#""teamId":"TEAM""#), "{request}");

    // Updated: the target is read first, so a second copy addresses the one already there.
    let (endpoint, wire) = response_server(vec![
        document("D-NEW"),
        serde_json::json!({"documentUpdate":{"success":true,"document":{"id":"D-NEW"}}}),
    ]);
    let updated = writable_source(&endpoint)
        .write_document(&ItemWrite {
            target: Some("D-NEW".into()),
            ..written("Design", Some("Revised"), Some("p1"))
        })
        .await
        .expect("the document is updated");
    assert_eq!(updated.0, "D-NEW");
    assert!(wire.recv().unwrap().contains("document(id:$id)"));
    let request = wire.recv().unwrap();
    assert!(request.contains("documentUpdate(id:$id"), "{request}");
    assert!(request.contains("Revised"), "{request}");

    // And removed again, which is what lets a copy that could not finish take it back.
    let (endpoint, wire) = response_server(vec![
        document("D-NEW"),
        serde_json::json!({"documentDelete":{"success":true}}),
    ]);
    writable_source(&endpoint)
        .delete_document(&"D-NEW".into())
        .await
        .expect("the document this copy created is taken back");
    assert!(wire.recv().unwrap().contains("document(id:$id)"));
    assert!(
        wire.recv().unwrap().contains("documentDelete(id:$id)"),
        "the pinned document delete is what removes it"
    );

    let (endpoint, _) = response_server(vec![serde_json::json!({
        "document": serde_json::Value::Null
    })]);
    writable_source(&endpoint)
        .delete_document(&"never-there".into())
        .await
        .expect("an id naming nothing is the state this asks for");
}

#[tokio::test]
async fn a_document_write_refuses_by_name_what_this_source_cannot_carry() {
    let document = Document {
        id: "ignored".into(),
        title: "Design".into(),
        content: None,
        project: None,
        labels: Vec::new(),
        url: None,
        location: None,
        created_at: None,
        updated_at: None,
        metadata: Default::default(),
        repositories: Vec::new(),
    };

    // A label, which Linear's own document type has no field for.
    let (endpoint, wire) = response_server(Vec::new());
    let refusal = writable_source(&endpoint)
        .write_document(&ItemWrite {
            target: None,
            item: Document {
                labels: vec![Label {
                    id: "L-1".into(),
                    name: "bug".into(),
                    color: None,
                }],
                ..document.clone()
            },
            depends_on: Vec::new(),
        })
        .await
        .expect_err("a label is refused rather than dropped");
    assert!(
        matches!(&refusal, SourceError::Refused { message }
            if message.contains("bug") && message.contains("labels")),
        "the refusal names the label: {refusal:?}"
    );
    assert!(
        wire.recv_timeout(std::time::Duration::from_millis(200))
            .is_err(),
        "and nothing was written before it"
    );

    // A dependency, which a document cannot have at all — as an edge, and as the reserved
    // key an edge would be recorded under.
    for write in [
        ItemWrite {
            target: None,
            item: document.clone(),
            depends_on: vec![DependencyEdge {
                from: DependencyEndpoint::from_native("d1".into(), ItemKind::Task),
                to: DependencyEndpoint::from_native("t1".into(), ItemKind::Task),
                kind: DependencyKind::Blocks,
            }],
        },
        ItemWrite {
            target: None,
            item: Document {
                metadata: [(
                    onetaskgraph_plugin_api::DependencyEdge::RECORDED_KEY.to_owned(),
                    serde_json::json!([{"id": "elsewhere:T-9", "kind": "task"}]),
                )]
                .into_iter()
                .collect(),
                ..document.clone()
            },
            depends_on: Vec::new(),
        },
    ] {
        let (endpoint, _) = response_server(Vec::new());
        let refusal = writable_source(&endpoint)
            .write_document(&write)
            .await
            .expect_err("a document depends on nothing");
        assert!(
            matches!(&refusal, SourceError::Refused { message }
                if message.contains("onetaskgraph.depends_on")),
            "the refusal names the key: {refusal:?}"
        );
    }

    // And a target this workspace does not hold is refused rather than created.
    let (endpoint, wire) = response_server(vec![serde_json::json!({
        "document": serde_json::Value::Null
    })]);
    let refusal = writable_source(&endpoint)
        .write_document(&ItemWrite {
            target: Some("D-GONE".into()),
            item: document,
            depends_on: Vec::new(),
        })
        .await
        .expect_err("a target that is not there is not a create");
    assert!(
        matches!(&refusal, SourceError::Refused { message }
            if message.contains("D-GONE") && message.contains("work")),
        "the refusal names the source and the document: {refusal:?}"
    );
    assert!(wire.recv().unwrap().contains("document(id:$id)"));
    assert!(
        wire.recv_timeout(std::time::Duration::from_millis(200))
            .is_err(),
        "nothing was created in its place"
    );
}

#[tokio::test]
async fn malformed_document_shapes_are_rejected_rather_than_read_past() {
    for (description, body) in [
        (
            "a node with no title",
            serde_json::json!({"documents":{"nodes":[{"id":"d1","content":null,"url":"u",
                "createdAt":null,"updatedAt":null,"project":null}],
                "pageInfo":{"hasNextPage":false,"endCursor":null}}}),
        ),
        (
            "a node with no project field at all",
            serde_json::json!({"documents":{"nodes":[{"id":"d1","title":"T","content":null,
                "url":"u","createdAt":null,"updatedAt":null}],
                "pageInfo":{"hasNextPage":false,"endCursor":null}}}),
        ),
        (
            "an unterminated metadata slot",
            serde_json::json!({"documents":{"nodes":[{"id":"d1","title":"T",
                "content":"body\n<!-- onetaskgraph.metadata\n{}","url":"u",
                "createdAt":null,"updatedAt":null,"project":null}],
                "pageInfo":{"hasNextPage":false,"endCursor":null}}}),
        ),
        (
            "no documents connection",
            serde_json::json!({ "viewer": {"id": "u"} }),
        ),
    ] {
        let (endpoint, _) = response_server(vec![body]);
        let failure = source(&endpoint)
            .query_documents(
                &DocumentQuery::default(),
                &PageRequest {
                    cursor: None,
                    limit: 5,
                },
            )
            .await
            .expect_err(description);
        assert!(
            matches!(failure, SourceError::Malformed { .. }),
            "{description}: {failure:?}"
        );
    }
}
