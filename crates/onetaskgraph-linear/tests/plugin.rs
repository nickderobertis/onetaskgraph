//! Public factory and real-HTTP fixture journeys.

use onetaskgraph_plugin_api::{
    Comment, CommentBody, Cursor, DependencyEdge, DependencyEndpoint, DependencyKind, Direction,
    Document, DocumentQuery, ItemKind, ItemWrite, Label, LabelFilter, Location, MetadataKey,
    NativeId, NewComment, PageRequest, Priority, Project, ProjectFilter, ProjectQuery,
    SecretResolver, SourceError, SourceName, SourcePlugin, Status, StatusCategory, Task, TaskQuery,
    TaskRef, TaskSource, TaskUpdate,
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

/// The `status_mapping` every writable source here carries: Linear has no built-in names, so
/// a source without one refuses every status write.
fn writable_mapping() -> serde_json::Value {
    serde_json::json!({"todo":"Todo","in-progress":"In Progress","done":"Done"})
}

fn writable_source(endpoint: &str) -> Box<dyn TaskSource> {
    onetaskgraph_linear::Plugin
        .build(
            &SourceName::new("work").unwrap(),
            &serde_json::json!({"endpoint":endpoint,"team":"ENG","status_mapping":writable_mapping()}),
            &Secrets(Some("fixture-key".into())),
        )
        .unwrap()
}

/// What [`onetaskgraph_linear::graphql::RESOLUTION`] answers: the team `TEAM` holding
/// `states`, and the workspace's project `statuses`.
fn resolution(states: serde_json::Value, statuses: serde_json::Value) -> serde_json::Value {
    let page = |nodes| serde_json::json!({"nodes":nodes,"pageInfo":{"hasNextPage":false}});
    serde_json::json!({"teams":{"nodes":[{"id":"TEAM","states":page(states)}]},
        "projectStatuses":page(statuses)})
}

/// The resolution a [`writable_source`] writes through: a workflow state and a project status
/// for every name [`writable_mapping`] gives, under ids naming the kind and the name.
fn writable_resolution() -> serde_json::Value {
    resolution(
        serde_json::json!([
            {"id":"STATE","name":"Todo","type":"unstarted"},
            {"id":"STATE-IN-PROGRESS","name":"In Progress","type":"started"},
            {"id":"STATE-DONE","name":"Done","type":"completed"},
        ]),
        serde_json::json!([
            {"id":"STATUS","name":"Todo","type":"planned","position":1.0},
            {"id":"STATUS-IN-PROGRESS","name":"In Progress","type":"started","position":2.0},
            {"id":"STATUS-DONE","name":"Done","type":"completed","position":3.0},
        ]),
    )
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
                r#",{"id":"i2","identifier":"ENG-2","title":"Other","description":null,"url":null,"createdAt":null,"updatedAt":null,"project":null,"state":{"name":"Todo","type":"unstarted"},"priority":0,"labels":{"nodes":[]}}"#
            };
            format!(
                r#"{{"data":{{"issues":{{"nodes":[{{"id":"i1","identifier":"ENG-1","title":"Team","description":null,"url":null,"createdAt":null,"updatedAt":null,"project":null,"state":{{"name":"Todo","type":"unstarted"}},"priority":0,"labels":{{"nodes":[]}}}}{second}],"pageInfo":{{"hasNextPage":false,"endCursor":null}}}}}}}}"#
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
                "priority",
                "labelIds",
                "projectId",
            ][..],
        ),
        (
            "IssueUpdateInput",
            &[
                "title",
                "description",
                "stateId",
                "priority",
                "labelIds",
                "projectId",
            ][..],
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
        ("CommentCreateInput", &["body", "issueId"][..]),
        ("CommentUpdateInput", &["body"][..]),
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
        variables: &[query::VariableDefinition<'a, String>],
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
            for (argument, supplied) in &selected.arguments {
                assert!(
                    field.arguments.iter().any(|input| input.name == *argument),
                    "{}.{} lacks argument {argument}",
                    type_name,
                    field.name
                );
                if let query::Value::Variable(name) = supplied {
                    let variable = variables
                        .iter()
                        .find(|variable| variable.name == *name)
                        .expect("used variable is declared");
                    let input = field
                        .arguments
                        .iter()
                        .find(|input| input.name == *argument)
                        .expect("argument exists");
                    assert_eq!(
                        format!("{:?}", variable.var_type),
                        format!("{:?}", input.value_type),
                        "variable {name} type drifted at {type_name}.{}.{argument}",
                        field.name,
                    );
                }
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
                    variables,
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
        (
            graphql::ISSUE_COMMENTS,
            Some(include_str!("fixtures/comments.json")),
        ),
        (graphql::COMMENT, None),
    ] {
        let document = query::parse_query::<String>(operation).unwrap();
        let fixture =
            fixture.map(|fixture| serde_json::from_str::<serde_json::Value>(fixture).unwrap());
        let query::Definition::Operation(query::OperationDefinition::Query(operation)) =
            &document.definitions[0]
        else {
            panic!("expected query")
        };
        validate(
            &objects,
            "Query",
            &operation.selection_set,
            fixture.as_ref().map(|fixture| &fixture["data"]),
            &operation.variable_definitions,
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
        (graphql::RESOLUTION, false),
        (graphql::WORKFLOW_STATE_CREATE, true),
        (graphql::PROJECT_STATUS_CREATE, true),
        (graphql::ISSUE_UPDATE_READ, true),
        (graphql::ISSUE_REWRITE, true),
        (graphql::PROJECT_REWRITE, true),
        (graphql::ISSUE_LABEL, false),
        (graphql::PROJECT_LABEL, false),
        (graphql::ISSUE_CREATE, true),
        (graphql::ISSUE_UPDATE, true),
        (graphql::ISSUE_PRIORITY_UPDATE, true),
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
        (graphql::ISSUE_COMMENTS, false),
        (graphql::COMMENT, false),
        (graphql::COMMENT_CREATE, true),
        (graphql::COMMENT_UPDATE, true),
        (graphql::COMMENT_DELETE, true),
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

/// Answer every request with the same superset, and record what was asked.
///
/// One response object carrying every root this source can select, because the source
/// reads the one key it asked for and ignores the rest — so a single constant stands in
/// for the whole API, and driving the surface needs no scripted queue that would have to
/// be kept in step with the order of the calls.
fn superset_server() -> (String, mpsc::Receiver<String>) {
    let page = |nodes: serde_json::Value| serde_json::json!({"nodes":nodes,"pageInfo":{"hasNextPage":false,"endCursor":null}});
    // One relation at each end of each root, so the write path's "delete what is there
    // before writing what was asked for" reaches the two relation deletes: an operation
    // this never provokes is an operation this never checks. Each root carries its own
    // vocabulary, because a project relation is typed `dependency` where an issue's is
    // `blocks` and each read accepts only its own.
    let relations = |kind: &str, far: &str, near: &str, id: &str| {
        serde_json::json!({"nodes":[{"id":id,"type":kind,(far):{"id":"OTHER"},(near):{"id":"OTHER"}}],
                           "pageInfo":{"hasNextPage":false,"endCursor":null}})
    };
    let body = serde_json::json!({"data":{
        "viewer": {"id":"U"},
        "issue": {"id":"I","identifier":"ENG-1","title":"issue","description":null,"url":null,"createdAt":null,
                  "updatedAt":null,"state":{"name":"Todo","type":"unstarted"},"priority":0,
                  "labels":{"nodes":[]},"project":null,
                  "relations":relations("blocks","relatedIssue","issue","IR"),
                  "inverseRelations":relations("blocks","relatedIssue","issue","IR2"),
                  "comments":{"nodes":[{"id":"C","body":"comment","url":"u","createdAt":null,
                                        "updatedAt":null,"user":null}],
                              "pageInfo":{"hasPreviousPage":false,"startCursor":null}}},
        // On the issue above, so an edit and a removal get past placing the comment and reach
        // the two mutations: an operation this never provokes is an operation this never checks.
        "comment": {"id":"C","archivedAt":null,"issue":{"id":"I"}},
        "commentCreate": {"success":true,"comment":{"id":"C","body":"comment","url":"u",
                          "createdAt":null,"updatedAt":null,"user":{"displayName":"ada"}}},
        "commentUpdate": {"success":true,"comment":{"id":"C","body":"comment","url":"u",
                          "createdAt":null,"updatedAt":null,"user":{"displayName":"ada"}}},
        "commentDelete": {"success":true},
        "project": {"id":"P","name":"project","description":null,"url":null,"createdAt":null,
                    "updatedAt":null,"status":{"name":"Todo","type":"planned"},
                    "labels":{"nodes":[]},
                    "relations":relations("dependency","relatedProject","project","PR"),
                    "inverseRelations":relations("dependency","relatedProject","project","PR2")},
        "document": {"id":"D","title":"document","content":null,"url":null,"createdAt":null,
                     "updatedAt":null,"project":null},
        "issues": page(serde_json::json!([])),
        "projects": page(serde_json::json!([])),
        "documents": page(serde_json::json!([])),
        "issueLabels": page(serde_json::json!([{"id":"L","name":"bug","color":null}])),
        "projectLabels": {"nodes":[{"id":"PL","name":"roadmap","color":null}]},
        "teams": {"nodes":[{"id":"TEAM","states":{"nodes":[
            {"id":"STATE","name":"In Progress","type":"started"},
            {"id":"STATE-TODO","name":"Todo","type":"unstarted"}],"pageInfo":{"hasNextPage":false}}}]},
        "projectStatuses": {"nodes":[{"id":"STATUS","name":"Todo","type":"planned","position":1.0}],
                            "pageInfo":{"hasNextPage":false}},
        "workflowStateCreate": {"success":true,"workflowState":{"id":"NEW-STATE","name":"Queued","type":"unstarted"}},
        "projectStatusCreate": {"success":true,"status":{"id":"NEW-STATUS","name":"Queued","type":"planned","position":2.0}},
        "issueCreate": {"success":true,"issue":{"id":"I"}},
        // The whole issue, for a status write and a targeted update, which answer the task
        // this payload reports; `priority` for the narrow priority write. Every other issue
        // write reads the id alone and passes over the rest.
        "issueUpdate": {"success":true,"issue":{"id":"I","identifier":"ENG-1","title":"issue",
                        "description":null,"url":null,"createdAt":null,"updatedAt":null,
                        "archivedAt":null,"state":{"name":"In Progress","type":"started"},
                        "priority":2,"labels":{"nodes":[]},"project":null,
                        "relations":relations("blocks","relatedIssue","issue","IR")}},
        "projectCreate": {"success":true,"project":{"id":"P"}},
        "projectUpdate": {"success":true,"project":{"id":"P",
                          "relations":relations("dependency","relatedProject","project","PR")}},
        "documentCreate": {"success":true,"document":{"id":"D"}},
        "documentUpdate": {"success":true,"document":{"id":"D"}},
        "issueRelationCreate": {"success":true,"issueRelation":{"id":"R"}},
        "projectRelationCreate": {"success":true,"projectRelation":{"id":"R"}},
        "issueRelationDelete": {"success":true},
        "projectRelationDelete": {"success":true},
        "issueDelete": {"success":true},
        "projectDelete": {"success":true},
        "documentDelete": {"success":true},
    }})
    .to_string();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let mut bytes = vec![0; 65536];
            let Ok(n) = stream.read(&mut bytes) else {
                break;
            };
            bytes.truncate(n);
            if tx
                .send(String::from_utf8_lossy(&bytes).into_owned())
                .is_err()
            {
                break;
            }
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    (format!("http://{addr}/graphql"), rx)
}

/// Every variables object this source builds at runtime, against the pinned schema.
///
/// **This is the check the two above could not be.** They parse the production documents,
/// and a filter is not in a document: it is built field by field at runtime and handed over
/// whole as `$filter`. So `IssueFilter` and `ProjectFilter` sat in the pinned schema
/// carrying nothing but `and`/`or` while this source sent a `team` and an issue-shaped
/// `state` into `projects(filter:)`, and Linear was the only thing that ever read them —
/// one refusal per round trip, one round trip per release. Every write input is built the
/// same way and had the same hole.
///
/// So this drives the whole surface against a server that answers everything, records what
/// really went out, and walks each variables object against the pinned type of the argument
/// it stands at: every key against that input type's members, every list against its
/// element type, every enum value against its members, every scalar against its kind. A
/// member Linear does not have fails here, offline, in the pass that introduces it.
#[tokio::test]
async fn every_variables_object_this_source_sends_conforms_to_the_pinned_schema() {
    use graphql_parser::{query, schema};
    let (endpoint, wire) = superset_server();
    let source = writable_source(&endpoint);
    let request = PageRequest {
        cursor: Some(Cursor("cursor".into())),
        limit: 5,
    };
    let labels = LabelFilter {
        any_of: vec!["bug".into(), "chore".into()],
        all_of: vec!["core".into()],
        none_of: vec!["wontfix".into()],
    };
    let statuses = vec![
        StatusCategory::Backlog,
        StatusCategory::Todo,
        StatusCategory::InProgress,
        StatusCategory::Done,
        StatusCategory::Cancelled,
    ];
    source.health().await.unwrap();
    source.get_task(&"I".into()).await.unwrap();
    source.get_project(&"P".into()).await.unwrap();
    source.get_document(&"D".into()).await.unwrap();
    source.labels(&request).await.unwrap();
    for project in [
        ProjectFilter::Any,
        ProjectFilter::Orphans,
        ProjectFilter::Is("P-1".into()),
    ] {
        source
            .query_tasks(
                &TaskQuery {
                    labels: labels.clone(),
                    statuses: statuses.clone(),
                    project: project.clone(),
                    ..TaskQuery::default()
                },
                &request,
            )
            .await
            .unwrap();
        source
            .query_documents(
                &DocumentQuery {
                    labels: labels.clone(),
                    project,
                    ..DocumentQuery::default()
                },
                &request,
            )
            .await
            .unwrap();
    }
    source
        .query_projects(
            &ProjectQuery {
                labels: labels.clone(),
                statuses: statuses.clone(),
                ..ProjectQuery::default()
            },
            &request,
        )
        .await
        .unwrap();
    for direction in [Direction::DependsOn, Direction::DependedOnBy] {
        source
            .task_dependencies(&"I".into(), direction, &request)
            .await
            .unwrap();
        source
            .project_dependencies(&"P".into(), direction, &request)
            .await
            .unwrap();
    }
    let task: Task = serde_json::from_value(serde_json::json!({"id":"authored:T","title":"task","content":"body","status":{"category":"todo","name":"Todo"},"labels":[{"id":"old","name":"bug","color":null}],"project":"P","repositories":["github.com/acme/work"],"metadata":{"n":1}})).unwrap();
    let project: Project = serde_json::from_value(serde_json::json!({"id":"authored:P","title":"project","content":"body","status":{"category":"todo","name":"Todo"},"labels":[{"id":"old","name":"roadmap","color":null}],"repositories":[],"metadata":{}})).unwrap();
    let document: Document = serde_json::from_value(serde_json::json!({"id":"authored:D","title":"document","content":"body","project":"P","labels":[],"repositories":[],"metadata":{}})).unwrap();
    let task_edge = DependencyEdge {
        from: DependencyEndpoint::new("authored:T".into(), ItemKind::Task).unwrap(),
        to: DependencyEndpoint::new("work:I-FAR".into(), ItemKind::Task).unwrap(),
        kind: DependencyKind::Blocks,
    };
    let project_edge = DependencyEdge {
        from: DependencyEndpoint::new("authored:P".into(), ItemKind::Project).unwrap(),
        to: DependencyEndpoint::new("work:P-FAR".into(), ItemKind::Project).unwrap(),
        kind: DependencyKind::Blocks,
    };
    for target in [None, Some(NativeId::from("I"))] {
        source
            .write_task(&ItemWrite {
                target,
                item: task.clone(),
                depends_on: vec![task_edge.clone()],
            })
            .await
            .unwrap();
    }
    for target in [None, Some(NativeId::from("P"))] {
        source
            .write_project(&ItemWrite {
                target,
                item: project.clone(),
                depends_on: vec![project_edge.clone()],
            })
            .await
            .unwrap();
    }
    for target in [None, Some(NativeId::from("D"))] {
        source
            .write_document(&ItemWrite {
                target,
                item: document.clone(),
                depends_on: vec![],
            })
            .await
            .unwrap();
    }
    source
        .task_comments(&"I".into(), &request)
        .await
        .unwrap()
        .expect("the superset holds the issue");
    source
        .add_comment(
            &"I".into(),
            &NewComment {
                body: CommentBody::new("comment\n").unwrap(),
                author: None,
            },
        )
        .await
        .unwrap()
        .expect("the superset holds the issue");
    source
        .edit_comment(
            &"I".into(),
            &"C".into(),
            &CommentBody::new("edited").unwrap(),
        )
        .await
        .unwrap()
        .expect("the superset places the comment on the issue");
    source
        .delete_comment(&"I".into(), &"C".into())
        .await
        .unwrap()
        .expect("the superset places the comment on the issue");
    // `in-progress` rather than the `todo` the superset's issue already reads as, because a
    // task already in the category asked for is answered with no write at all.
    assert_eq!(
        source
            .set_task_status(&"I".into(), StatusCategory::InProgress)
            .await
            .unwrap()
            .expect("the superset holds the issue")
            .name,
        "In Progress"
    );
    assert_eq!(
        source
            .set_task_priority(&"I".into(), Priority::High)
            .await
            .unwrap(),
        Some(Priority::High)
    );
    source
        .set_task_content(&"I".into(), "replaced")
        .await
        .unwrap()
        .expect("the superset holds the issue");
    // Every member a targeted update can send, and a relation replaced, in one call.
    source
        .update_task(
            &"I".into(),
            &TaskUpdate {
                title: Some("renamed".into()),
                content: Some("replaced".into()),
                status: Some(Status {
                    category: StatusCategory::InProgress,
                    name: "In Progress".into(),
                }),
                priority: Some(Priority::High),
                metadata_set: std::collections::BTreeMap::from([(
                    MetadataKey::new("team.a").unwrap(),
                    serde_json::json!(1),
                )]),
                depends_on: Some(vec![DependencyEdge {
                    from: DependencyEndpoint::new("I".into(), ItemKind::Task).unwrap(),
                    to: DependencyEndpoint::new("I-2".into(), ItemKind::Task).unwrap(),
                    kind: DependencyKind::Blocks,
                }]),
                ..TaskUpdate::default()
            },
        )
        .await
        .unwrap()
        .expect("the superset holds the issue");
    // The follow-up searches, each narrowing on its own and every one together.
    let follow_up = TaskQuery {
        priorities: vec![Priority::High, Priority::None],
        commented_since: chrono::DateTime::parse_from_rfc3339("2026-10-01T00:00:00Z")
            .ok()
            .map(|at| at.with_timezone(&chrono::Utc)),
        metadata: vec![
            onetaskgraph_plugin_api::MetadataMatch::new("caller.key", Vec::new(), "v").unwrap(),
            onetaskgraph_plugin_api::MetadataMatch::new(
                "orchestrator.follow-up",
                vec!["root_cause".into()],
                "stale-cache",
            )
            .unwrap(),
        ],
        origin: Some("elsewhere:ORIG-1".into()),
        ..TaskQuery::default()
    };
    for fields in [
        onetaskgraph_plugin_api::TextFields::Title,
        onetaskgraph_plugin_api::TextFields::Content,
        onetaskgraph_plugin_api::TextFields::TitleOrContent,
    ] {
        source
            .query_tasks(
                &TaskQuery {
                    text: Some(onetaskgraph_plugin_api::TextQuery {
                        terms: "alpha".into(),
                        fields,
                    }),
                    ..follow_up.clone()
                },
                &request,
            )
            .await
            .unwrap();
    }
    // Every narrow write of the slot, and the store's own `delivered_by`.
    let key = MetadataKey::new("myapp.review").unwrap();
    source
        .set_task_metadata(&"I".into(), &key, &serde_json::json!(true))
        .await
        .unwrap();
    source
        .set_project_metadata(&"P".into(), &key, &serde_json::json!(true))
        .await
        .unwrap();
    source
        .set_document_metadata(&"D".into(), &key, &serde_json::json!(true))
        .await
        .unwrap();
    let provenance = serde_json::json!({"digest":"sha256:0"});
    source
        .set_task_rendering(
            &"I".into(),
            "rendered",
            &provenance,
            &std::collections::BTreeMap::new(),
        )
        .await
        .unwrap();
    source
        .set_document_rendering(
            &"D".into(),
            "rendered",
            &provenance,
            &std::collections::BTreeMap::new(),
        )
        .await
        .unwrap();
    source
        .set_delivered_by(&"I".into(), &[TaskRef::new("plan:T-1").unwrap()])
        .await
        .unwrap();
    // A source whose `status_mapping` names states and that is scoped to one project: its
    // status narrowing by name, by type-but-not-name and by a type none of the five, its
    // project and document reads narrowed to the scope, and a status written by name.
    let mapped = onetaskgraph_linear::Plugin
        .build(
            &SourceName::new("work").unwrap(),
            &serde_json::json!({"endpoint":endpoint,"team":"ENG","project":"P",
                "status_mapping":{"in-progress":"In Progress","queued":"Queued","draft":null}}),
            &Secrets(Some("fixture-key".into())),
        )
        .unwrap();
    mapped
        .query_tasks(
            &TaskQuery {
                statuses: vec![
                    StatusCategory::Queued,
                    StatusCategory::Todo,
                    StatusCategory::Draft,
                    StatusCategory::Unknown,
                ],
                ..TaskQuery::default()
            },
            &request,
        )
        .await
        .unwrap();
    mapped
        .query_projects(&ProjectQuery::default(), &request)
        .await
        .unwrap();
    mapped
        .query_documents(&DocumentQuery::default(), &request)
        .await
        .unwrap();
    mapped
        .set_task_status(&"I".into(), StatusCategory::InProgress)
        .await
        .unwrap();
    // `sources fields --apply`, creating a name of each kind the superset lacks.
    let report = onetaskgraph_linear::status_names(
        &SourceName::new("work").unwrap(),
        serde_json::from_value(serde_json::json!({"endpoint":endpoint,"team":"ENG",
            "status_mapping":{"queued":"Queued"}}))
        .unwrap(),
        &Secrets(Some("fixture-key".into())),
        true,
    )
    .await
    .unwrap();
    assert!(report.refused.is_none(), "{report:?}");
    source.delete_task(&"I".into()).await.unwrap();
    source.delete_project(&"P".into()).await.unwrap();
    source.delete_document(&"D".into()).await.unwrap();

    let pinned = schema::parse_schema::<String>(include_str!("fixtures/schema.graphql")).unwrap();
    let mut inputs = std::collections::HashMap::new();
    let mut enums = std::collections::HashMap::new();
    let mut roots = std::collections::HashMap::new();
    for definition in &pinned.definitions {
        match definition {
            schema::Definition::TypeDefinition(schema::TypeDefinition::InputObject(input)) => {
                inputs.insert(input.name.as_str(), input);
            }
            schema::Definition::TypeDefinition(schema::TypeDefinition::Enum(kind)) => {
                enums.insert(
                    kind.name.as_str(),
                    kind.values
                        .iter()
                        .map(|value| value.name.as_str())
                        .collect::<Vec<_>>(),
                );
            }
            schema::Definition::TypeDefinition(schema::TypeDefinition::Object(object))
                if object.name == "Query" || object.name == "Mutation" =>
            {
                roots.insert(object.name.as_str(), object);
            }
            _ => {}
        }
    }
    /// Whether `value` is admissible where the pinned schema declares `expected`.
    fn conforms(
        inputs: &std::collections::HashMap<&str, &schema::InputObjectType<'_, String>>,
        enums: &std::collections::HashMap<&str, Vec<&str>>,
        expected: &schema::Type<'_, String>,
        value: &serde_json::Value,
        path: &str,
    ) {
        match expected {
            schema::Type::NonNullType(inner) => {
                assert!(
                    !value.is_null(),
                    "{path} is null where the schema requires a value"
                );
                conforms(inputs, enums, inner, value, path);
            }
            _ if value.is_null() => {}
            schema::Type::ListType(inner) => {
                let items = value
                    .as_array()
                    .unwrap_or_else(|| panic!("{path} is {value} where the schema wants a list"));
                for (index, item) in items.iter().enumerate() {
                    conforms(inputs, enums, inner, item, &format!("{path}[{index}]"));
                }
            }
            schema::Type::NamedType(name) => {
                if let Some(input) = inputs.get(name.as_str()) {
                    let fields = value.as_object().unwrap_or_else(|| {
                        panic!("{path} is {value} where the schema wants a {name}")
                    });
                    for (key, value) in fields {
                        let field = input
                            .fields
                            .iter()
                            .find(|field| field.name == *key)
                            .unwrap_or_else(|| {
                                panic!("{path}.{key}: the pinned {name} has no member {key}")
                            });
                        conforms(
                            inputs,
                            enums,
                            &field.value_type,
                            value,
                            &format!("{path}.{key}"),
                        );
                    }
                    return;
                }
                if let Some(members) = enums.get(name.as_str()) {
                    let member = value
                        .as_str()
                        .unwrap_or_else(|| panic!("{path} is {value} where {name} wants a member"));
                    assert!(
                        members.contains(&member),
                        "{path} is {member:?}, and {name} has only {members:?}"
                    );
                    return;
                }
                let ok = match name.as_str() {
                    "String" | "ID" | "DateTime" | "DateTimeOrDuration" => value.is_string(),
                    "Boolean" => value.is_boolean(),
                    "Int" | "Float" => value.is_number(),
                    // A type this pin does not carry is one nothing can be checked against,
                    // which is the state this whole check exists to end.
                    _ => panic!("{path}: the pinned schema has no type {name}"),
                };
                assert!(ok, "{path} is {value} where the schema wants a {name}");
            }
        }
    }
    let mut seen = std::collections::BTreeSet::new();
    // Drained rather than iterated: the server answers forever, so its sender never hangs
    // up and `iter` would block on a request that is never coming. Every response was read
    // before the last call above returned, so everything sent is already in the channel.
    let requests = wire.try_iter().collect::<Vec<_>>();
    assert!(!requests.is_empty(), "the surface was driven");
    for request in &requests {
        let body = request
            .split_once("\r\n\r\n")
            .expect("the recorded request carries a body")
            .1;
        let body = serde_json::from_str::<serde_json::Value>(body).expect("the body is JSON");
        let document = query::parse_query::<String>(body["query"].as_str().unwrap()).unwrap();
        let (root_name, selection, declared) = match &document.definitions[0] {
            query::Definition::Operation(query::OperationDefinition::Query(operation)) => (
                "Query",
                &operation.selection_set,
                &operation.variable_definitions,
            ),
            query::Definition::Operation(query::OperationDefinition::Mutation(operation)) => (
                "Mutation",
                &operation.selection_set,
                &operation.variable_definitions,
            ),
            _ => panic!("a production document is an explicit query or mutation"),
        };
        let query::Selection::Field(root) = &selection.items[0] else {
            panic!("an operation has a root field")
        };
        seen.insert(root.name.clone());
        let root_field = roots[root_name]
            .fields
            .iter()
            .find(|field| field.name == root.name)
            .unwrap_or_else(|| panic!("the pinned schema lacks {root_name}.{}", root.name));
        for (argument_name, value) in &root.arguments {
            let query::Value::Variable(variable) = value else {
                // A literal argument is in the document, so the two checks above read it.
                continue;
            };
            let location = format!("{}({argument_name}:)", root.name);
            assert!(
                declared.iter().any(|candidate| candidate.name == *variable),
                "{location} stands on ${variable}, which nothing declares"
            );
            let argument = root_field
                .arguments
                .iter()
                .find(|argument| argument.name == *argument_name)
                .unwrap_or_else(|| panic!("{root_name}.{} takes no {argument_name}", root.name));
            let sent = body["variables"]
                .get(variable.as_str())
                .unwrap_or_else(|| panic!("{location} was declared and never sent"));
            conforms(&inputs, &enums, &argument.value_type, sent, &location);
        }
    }
    // The surface, not a sample of it: every operation this source can send has to have
    // been driven, because a variables object nothing sent is one nothing checked.
    let mut expected = std::collections::BTreeSet::new();
    for document in [
        onetaskgraph_linear::graphql::VIEWER,
        onetaskgraph_linear::graphql::ISSUE,
        onetaskgraph_linear::graphql::PROJECT,
        onetaskgraph_linear::graphql::DOCUMENT,
        onetaskgraph_linear::graphql::ISSUES,
        onetaskgraph_linear::graphql::PROJECTS,
        onetaskgraph_linear::graphql::DOCUMENTS,
        onetaskgraph_linear::graphql::LABELS,
        onetaskgraph_linear::graphql::ISSUE_RELATIONS,
        onetaskgraph_linear::graphql::PROJECT_RELATIONS,
        onetaskgraph_linear::graphql::RESOLUTION,
        onetaskgraph_linear::graphql::WORKFLOW_STATE_CREATE,
        onetaskgraph_linear::graphql::PROJECT_STATUS_CREATE,
        onetaskgraph_linear::graphql::ISSUE_UPDATE_READ,
        onetaskgraph_linear::graphql::ISSUE_REWRITE,
        onetaskgraph_linear::graphql::PROJECT_REWRITE,
        onetaskgraph_linear::graphql::ISSUE_LABEL,
        onetaskgraph_linear::graphql::PROJECT_LABEL,
        onetaskgraph_linear::graphql::ISSUE_CREATE,
        onetaskgraph_linear::graphql::ISSUE_UPDATE,
        onetaskgraph_linear::graphql::PROJECT_CREATE,
        onetaskgraph_linear::graphql::PROJECT_UPDATE,
        onetaskgraph_linear::graphql::ISSUE_RELATION_CREATE,
        onetaskgraph_linear::graphql::PROJECT_RELATION_CREATE,
        onetaskgraph_linear::graphql::ISSUE_RELATION_DELETE,
        onetaskgraph_linear::graphql::PROJECT_RELATION_DELETE,
        onetaskgraph_linear::graphql::ISSUE_DELETE,
        onetaskgraph_linear::graphql::PROJECT_DELETE,
        onetaskgraph_linear::graphql::DOCUMENT_CREATE,
        onetaskgraph_linear::graphql::DOCUMENT_UPDATE,
        onetaskgraph_linear::graphql::DOCUMENT_DELETE,
        onetaskgraph_linear::graphql::ISSUE_COMMENTS,
        onetaskgraph_linear::graphql::COMMENT,
        onetaskgraph_linear::graphql::COMMENT_CREATE,
        onetaskgraph_linear::graphql::COMMENT_UPDATE,
        onetaskgraph_linear::graphql::COMMENT_DELETE,
    ] {
        let parsed = query::parse_query::<String>(document).unwrap();
        let selection = match &parsed.definitions[0] {
            query::Definition::Operation(query::OperationDefinition::Query(operation)) => {
                &operation.selection_set
            }
            query::Definition::Operation(query::OperationDefinition::Mutation(operation)) => {
                &operation.selection_set
            }
            _ => panic!("a production document is an explicit query or mutation"),
        };
        let query::Selection::Field(root) = &selection.items[0] else {
            panic!("an operation has a root field")
        };
        expected.insert(root.name.clone());
    }
    assert_eq!(
        seen, expected,
        "every operation this source can send has to reach this check"
    );
}

/// An item Linear has trashed or archived is one this source does not hold.
///
/// None of Linear's three `delete` verbs removes anything: observed on 2026-09-04,
/// `issueDelete`, `projectDelete` and `documentDelete` each answered `success: true` and
/// the item still read back by id, carrying `archivedAt` and `trashed: true`. Its separate
/// archive verb is a third state — `archivedAt` set, `trashed` null — and Linear excludes
/// both from every connection, so a listing had already stopped returning them while a read
/// by id still did. `archivedAt` is the marker they share, which is why it is the one read.
///
/// That is what the live journey failed on with `a document this run removed is still
/// readable`, and it is what a copy's undo depends on: an item this run created has to be
/// gone once it is taken back.
#[tokio::test]
async fn an_archived_or_trashed_item_is_not_held_by_this_source_over_real_http() {
    for (root, body) in [
        (
            "issue",
            serde_json::json!({"issue":{"id":"I","identifier":"ENG-1","title":"gone","description":null,"url":null,
                "createdAt":null,"updatedAt":null,"archivedAt":"2026-09-04T18:55:13.746Z",
                "state":{"name":"Todo","type":"unstarted"},"priority":0,"labels":{"nodes":[]},"project":null}}),
        ),
        (
            "project",
            serde_json::json!({"project":{"id":"P","name":"gone","description":null,"url":null,
                "createdAt":null,"updatedAt":null,"archivedAt":"2026-09-04T18:55:13.746Z",
                "status":{"name":"Todo","type":"planned"},"labels":{"nodes":[]}}}),
        ),
        (
            "document",
            serde_json::json!({"document":{"id":"D","title":"gone","content":null,"url":null,
                "createdAt":null,"updatedAt":null,"archivedAt":"2026-09-04T18:55:13.746Z",
                "project":null}}),
        ),
    ] {
        let (endpoint, _) = response_server(vec![body.clone()]);
        let source = source(&endpoint);
        let held = match root {
            "issue" => source.get_task(&"I".into()).await.unwrap().is_some(),
            "project" => source.get_project(&"P".into()).await.unwrap().is_some(),
            _ => source.get_document(&"D".into()).await.unwrap().is_some(),
        };
        assert!(!held, "an archived {root} is not held: {body}");
    }
    // And a delete over one is the state the caller asked for, with no mutation sent: the
    // read is what says there is nothing left to remove.
    let (endpoint, wire) = response_server(vec![serde_json::json!({"document":{"id":"D",
        "title":"gone","content":null,"url":null,"createdAt":null,"updatedAt":null,
        "archivedAt":"2026-09-04T18:55:13.746Z","project":null}})]);
    source(&endpoint)
        .delete_document(&"D".into())
        .await
        .expect("an item already in the trash is the state this asks for");
    assert!(wire.recv().unwrap().contains("document(id:$id)"));
    assert!(
        wire.recv_timeout(std::time::Duration::from_millis(200))
            .is_err(),
        "nothing is deleted when Linear no longer holds it"
    );
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
    let id_page = |root: &str, id: &str| serde_json::json!({(root):{"nodes":[{"id":id}]}});
    // One source writes all four, so the resolution is read once, by the first write, and
    // every write after it resolves its status from what that read answered. A create reads
    // no relations first: an item that did not exist holds none to replace.
    let (endpoint, wire) = response_server(vec![
        writable_resolution(),
        id_page("issueLabels", "LABEL"),
        serde_json::json!({"issueCreate":{"success":true,"issue":{"id":"I-NEW"}}}),
        serde_json::json!({"issueRelationCreate":{"success":true,"issueRelation":{"id":"R-I"}}}),
        serde_json::json!({"issues":{"nodes":[],"pageInfo":{"hasNextPage":true,"endCursor":"next"}}}),
        serde_json::json!({"issues":{"nodes":[{"id":"I-FAR","identifier":"ENG-9","title":"far","description":"\n\n<!-- onetaskgraph.metadata\n{\"onetaskgraph.origin\":\"authored:FAR\"}\n-->","url":null,"createdAt":null,"updatedAt":null,"state":{"name":"Todo","type":"unstarted"},"priority":0,"labels":{"nodes":[]},"project":null}],"pageInfo":{"hasNextPage":false,"endCursor":null}}}),
        id_page("issueLabels", "LABEL"),
        // A rewrite reads back the relations it replaces in its own answer.
        serde_json::json!({"issueUpdate":{"success":true,"issue":{"id":"I-NEW","relations":{"nodes":[{"id":"OLD","type":"blocks","relatedIssue":{"id":"OLD-FAR"}}],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}}),
        serde_json::json!({"issueRelationDelete":{"success":true}}),
        serde_json::json!({"projects":{"nodes":[{"id":"P-FAR","name":"far","description":"<!-- onetaskgraph.metadata\n{\"onetaskgraph.origin\":\"authored:PFAR\"}\n-->","url":null,"createdAt":null,"updatedAt":null,"status":{"name":"Todo","type":"unstarted"},"labels":{"nodes":[]}}],"pageInfo":{"hasNextPage":false,"endCursor":null}}}),
        id_page("projectLabels", "PLABEL"),
        serde_json::json!({"projectCreate":{"success":true,"project":{"id":"P-NEW"}}}),
        serde_json::json!({"projectRelationCreate":{"success":true,"projectRelation":{"id":"R-P"}}}),
        serde_json::json!({"projects":{"nodes":[{"id":"P-FAR","name":"far","description":"<!-- onetaskgraph.metadata\n{\"onetaskgraph.origin\":\"authored:PFAR\"}\n-->","url":null,"createdAt":null,"updatedAt":null,"status":{"name":"Todo","type":"unstarted"},"labels":{"nodes":[]}}],"pageInfo":{"hasNextPage":false,"endCursor":null}}}),
        id_page("projectLabels", "PLABEL"),
        serde_json::json!({"projectUpdate":{"success":true,"project":{"id":"P-NEW","relations":{"nodes":[{"id":"OLD-P","type":"dependency","relatedProject":{"id":"P-FAR"}}],"pageInfo":{"hasNextPage":true,"endCursor":"next"}}}}}),
        serde_json::json!({"projectRelationDelete":{"success":true}}),
        serde_json::json!({"project":{"description":null,"relations":{"nodes":[{"id":"OLD-P2","type":"related","relatedProject":{"id":"P-OTHER"}}],"pageInfo":{"hasNextPage":false,"endCursor":null}},"inverseRelations":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}),
        serde_json::json!({"projectRelationDelete":{"success":true}}),
        serde_json::json!({"projectRelationCreate":{"success":true,"projectRelation":{"id":"R-P2"}}}),
    ]);
    let writable = writable_source(&endpoint);
    let task: Task = serde_json::from_value(serde_json::json!({"id":"authored:NEAR","title":"visible task","content":"body","status":{"category":"todo","name":"Todo"},"labels":[{"id":"old","name":"bug","color":null}],"project":null,"repositories":["github.com/acme/work"],"metadata":{"object":{"n":1},"null":null}})).unwrap();

    for (labels, expected) in [
        (
            serde_json::json!({"issueLabels":{"nodes":[]}}),
            r#"source work cannot resolve label "bug": found 0 matches"#,
        ),
        (
            serde_json::json!({"issueLabels":{"nodes":[{"id":"L-ONE"},{"id":"L-TWO"}]}}),
            r#"source work cannot resolve label "bug": found 2 matches with ids ["L-ONE", "L-TWO"]"#,
        ),
    ] {
        let (endpoint, _) = response_server(vec![writable_resolution(), labels]);
        let refusal = writable_source(&endpoint)
            .write_task(&ItemWrite {
                target: None,
                item: task.clone(),
                depends_on: Vec::new(),
            })
            .await
            .expect_err("a non-unique label is refused before the write");
        assert!(
            matches!(&refusal, SourceError::Refused { message } if message == expected),
            "the refusal distinguishes this match count: {refusal:?}"
        );
    }

    let (endpoint, _) = response_server(vec![
        writable_resolution(),
        serde_json::json!({"issueLabels":{"nodes":[{"id":"L-ONE"},{}]}}),
    ]);
    let malformed_duplicate = writable_source(&endpoint)
        .write_task(&ItemWrite {
            target: None,
            item: task.clone(),
            depends_on: Vec::new(),
        })
        .await
        .expect_err("a duplicate label with no usable id is malformed");
    assert!(
        matches!(&malformed_duplicate, SourceError::Malformed { message }
            if message.contains("missing string field id")),
        "a response whose duplicate ids cannot be reported is malformed: {malformed_duplicate:?}"
    );

    let missing_team = onetaskgraph_linear::Plugin
        .build(
            &SourceName::new("work").unwrap(),
            &serde_json::json!({"endpoint":"http://127.0.0.1:1","status_mapping":writable_mapping()}),
            &Secrets(Some("fixture-key".into())),
        )
        .unwrap()
        .write_task(&ItemWrite {
            target: None,
            item: task.clone(),
            depends_on: Vec::new(),
        })
        .await
        .unwrap_err();
    assert!(format!("{missing_team}").contains("config.team"));
    let (unresolved_endpoint, unresolved_wire) = response_server(vec![serde_json::json!({
        "teams":{"nodes":[]},"projectStatuses":{"nodes":[],"pageInfo":{"hasNextPage":false}}})]);
    let unresolved_team = writable_source(&unresolved_endpoint)
        .write_task(&ItemWrite {
            target: None,
            item: task.clone(),
            depends_on: Vec::new(),
        })
        .await
        .unwrap_err();
    assert!(
        format!("{unresolved_team}")
            .contains("cannot resolve the configured team \"ENG\": found 0 matches")
    );
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
        kind: DependencyKind::Blocks,
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
    assert_eq!(
        writable
            .write_project(&ItemWrite {
                target: Some("P-NEW".into()),
                item: project,
                depends_on: vec![project_edge]
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
            .any(|request| request.contains(onetaskgraph_linear::graphql::ISSUE_REWRITE))
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
    //
    // The anchor *pair* is asserted with the ids because that pair is what carries the
    // direction: measured against the real API on 2026-09-04, the project whose anchor is
    // `start` is the one Linear reports as blocked, whichever id slot it sits in. This
    // source puts the item that depends in `projectId`, so `start` belongs there and
    // exchanging the two anchors would state every dependency backwards in the workspace
    // without Linear refusing a thing. `src/lib.rs` records the measurement.
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
                "type": "dependency",
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
            request.contains(onetaskgraph_linear::graphql::ISSUE_REWRITE)
                && request.contains("missing:FAR")
        })
        .expect("an unresolved same-source origin remains in recorded dependency metadata");
    assert!(unresolved_update.contains("onetaskgraph.depends_on"));
}

/// Linear types every project relation `dependency`, and that is an ordering.
///
/// Asked for one typed `related` on 2026-09-04 the real API answered
/// `Argument Validation Error` with
/// `constraints: {"isEnum": "type must be one of the following values: dependency"}`, so
/// this source says so itself rather than sending a value that cannot land. It says so
/// *before* the write, which is what this asserts alongside the message: the server here
/// answers nothing at all, so a refusal that had let the project be created first would
/// hang on the create instead of returning.
#[tokio::test]
async fn an_unordered_project_edge_is_refused_before_anything_is_written_over_real_http() {
    let (endpoint, wire) = response_server(Vec::new());
    let project: Project = serde_json::from_value(serde_json::json!({"id":"authored:P","title":"visible project","content":null,"status":{"category":"todo","name":"Todo"},"labels":[],"repositories":[],"metadata":{}})).unwrap();
    let refusal = writable_source(&endpoint)
        .write_project(&ItemWrite {
            target: None,
            item: project,
            depends_on: vec![DependencyEdge {
                from: DependencyEndpoint::new("authored:P".into(), ItemKind::Project).unwrap(),
                to: DependencyEndpoint::new("work:P-FAR".into(), ItemKind::Project).unwrap(),
                kind: DependencyKind::Related,
            }],
        })
        .await
        .expect_err("Linear has no unordered project relation to write this into");
    assert!(
        matches!(&refusal, SourceError::Refused { message }
            if message.contains("unordered dependency between projects")
                && message.contains("`dependency`")
                && message.contains("authored:P")
                && message.contains("work:P-FAR")),
        "the refusal names both ends and what Linear does accept: {refusal:?}"
    );
    assert!(
        wire.try_iter().next().is_none(),
        "the refusal reaches Linear with no request at all"
    );
}

/// An issue relation keeps `related`, because that vocabulary really does have it.
#[tokio::test]
async fn an_unordered_task_edge_is_still_written_as_related_over_real_http() {
    let (endpoint, wire) = response_server(vec![
        writable_resolution(),
        serde_json::json!({"issueCreate":{"success":true,"issue":{"id":"I-NEW"}}}),
        serde_json::json!({"issueRelationCreate":{"success":true,"issueRelation":{"id":"R-I"}}}),
    ]);
    let task: Task = serde_json::from_value(serde_json::json!({"id":"authored:T","title":"task","content":null,"status":{"category":"todo","name":"Todo"},"labels":[],"project":null,"repositories":[],"metadata":{}})).unwrap();
    writable_source(&endpoint)
        .write_task(&ItemWrite {
            target: None,
            item: task,
            depends_on: vec![DependencyEdge {
                from: DependencyEndpoint::new("authored:T".into(), ItemKind::Task).unwrap(),
                to: DependencyEndpoint::new("work:I-FAR".into(), ItemKind::Task).unwrap(),
                kind: DependencyKind::Related,
            }],
        })
        .await
        .expect("an issue relation may be unordered");
    let relation = wire
        .iter()
        .find(|request| request.contains(onetaskgraph_linear::graphql::ISSUE_RELATION_CREATE))
        .expect("the issue relation was written");
    let body = relation
        .split_once("\r\n\r\n")
        .expect("the recorded request carries a body")
        .1;
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(body).expect("the body is JSON")["variables"]["input"],
        serde_json::json!({"issueId":"I-NEW","relatedIssueId":"I-FAR","type":"related"})
    );
}

#[tokio::test]
async fn a_write_with_no_visible_description_or_metadata_sends_null_over_real_http() {
    let (endpoint, wire) = response_server(vec![
        writable_resolution(),
        serde_json::json!({"issueCreate":{"success":true,"issue":{"id":"NEW"}}}),
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
        writable_resolution(),
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
        writable_resolution(),
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
            vec![resolution(serde_json::json!([]), serde_json::json!([]))],
            task(),
            "workflow state",
        ),
        (
            vec![
                writable_resolution(),
                serde_json::json!({"issueCreate":{"success":false,"issue":null}}),
            ],
            task(),
            "unsuccessful",
        ),
        (
            vec![
                writable_resolution(),
                serde_json::json!({"issueCreate":{"success":true}}),
            ],
            task(),
            "missing issueCreate.issue",
        ),
        (
            vec![
                writable_resolution(),
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
        writable_resolution(),
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
    let (endpoint, wire) = response_server(vec![resolution(
        serde_json::json!([]),
        serde_json::json!([]),
    )]);
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
        writable_resolution(),
        serde_json::json!({"issueCreate":{"success":true,"issue":{"id":"NEW"}}}),
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
        writable_resolution(),
        serde_json::json!({"projectCreate":{"success":true,"project":{"id":"NEW"}}}),
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
        writable_resolution(),
        serde_json::json!({"projectUpdate":{"success":true,"project":{"id":"P","relations":{"nodes":[{"id":"R"}],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}}),
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
    // An update, because it is the write that replaces the relations its item holds: a create
    // holds none. Its own answer carries the first page, and a page saying there are more is
    // followed by a read of the next.
    let more = serde_json::json!({"nodes":[],"pageInfo":{"hasNextPage":true,"endCursor":"next"}});
    for (answered, followed, expected) in [
        (
            serde_json::json!({"id":"NEW"}),
            None,
            "a rewrite answered without the relations",
        ),
        (
            serde_json::json!({"id":"NEW","relations":{"nodes":7,"pageInfo":{"hasNextPage":false,"endCursor":null}}}),
            None,
            "missing relations.nodes",
        ),
        (
            serde_json::json!({"id":"NEW","relations":{"nodes":[{"id":""}],"pageInfo":{"hasNextPage":false,"endCursor":null}}}),
            None,
            "empty backend id",
        ),
        (
            serde_json::json!({"id":"NEW","relations":more.clone()}),
            Some(serde_json::json!({})),
            "missing relation item",
        ),
        (
            serde_json::json!({"id":"NEW","relations":more.clone()}),
            Some(serde_json::json!({"issue":{}})),
            "missing relations",
        ),
    ] {
        let mut responses = vec![
            writable_resolution(),
            serde_json::json!({"issueUpdate":{"success":true,"issue":answered}}),
        ];
        responses.extend(followed);
        let (endpoint, wire) = response_server(responses);
        let error = writable_source(&endpoint)
            .write_task(&ItemWrite {
                target: Some("NEW".into()),
                item: task(),
                depends_on: Vec::new(),
            })
            .await
            .unwrap_err();
        assert!(format!("{error}").contains(expected), "{error}");
        drop(wire);
    }
    let (endpoint, wire) = response_server(vec![
        writable_resolution(),
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
        let (endpoint, wire) = response_server(vec![writable_resolution(), response]);
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
        let (endpoint, wire) = response_server(vec![writable_resolution(), response]);
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
        writable_resolution(),
        serde_json::json!({"issueUpdate":{"success":true,"issue":{"id":"I","relations":{"nodes":[{"id":"R"}],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}}),
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
/// connection — inside the one resolution document — and matches the display name itself.
///
/// It is here because sending the filter Linear does not take is a
/// `GRAPHQL_VALIDATION_FAILED` refusal of the entire document, which no amount of correct
/// handling further down recovers from.
#[tokio::test]
async fn a_project_status_is_matched_locally_because_linear_narrows_that_connection_for_nobody() {
    let project = || {
        serde_json::from_value::<Project>(serde_json::json!({"id":"authored:P","title":"a project","content":null,"status":{"category":"todo","name":"Todo"},"labels":[],"repositories":[],"metadata":{}})).unwrap()
    };
    let statuses = |nodes: serde_json::Value| resolution(serde_json::json!([]), nodes);

    let (endpoint, wire) = response_server(vec![
        statuses(serde_json::json!([
            {"id":"S-BACKLOG","name":"Backlog","type":"backlog","position":1.0},
            {"id":"S-TODO","name":"todo","type":"planned","position":2.0},
            {"id":"S-DONE","name":"Done","type":"completed","position":3.0}
        ])),
        serde_json::json!({"projectCreate":{"success":true,"project":{"id":"P-NEW"}}}),
    ]);
    writable_source(&endpoint)
        .write_project(&ItemWrite {
            target: None,
            item: project(),
            depends_on: Vec::new(),
        })
        .await
        .expect("the one status answering to the name resolves it");
    let asked = wire.recv().unwrap();
    assert!(
        asked.contains("projectStatuses(first:250){") && !asked.contains("filter:{name"),
        "the request Linear refuses outright is one passing projectStatuses a filter: {asked}"
    );
    let created = wire.recv().unwrap();
    assert!(
        created.contains("S-TODO"),
        "the write carries the matched status's own id, not its name: {created}"
    );
    drop(wire);

    // Refused after one read: two statuses match the name case-insensitively, so choosing
    // either id would guess which status the operator intended.
    let (endpoint, wire) = response_server(vec![statuses(serde_json::json!([
        {"id":"S-ONE","name":"Todo","type":"planned","position":1.0},
        {"id":"S-TWO","name":"TODO","type":"planned","position":2.0}
    ]))]);
    let ambiguous = writable_source(&endpoint)
        .write_project(&ItemWrite {
            target: None,
            item: project(),
            depends_on: Vec::new(),
        })
        .await
        .unwrap_err();
    assert!(
        format!("{ambiguous}").contains("project status \"Todo\": found 2 matches"),
        "a name two statuses answer to under the same case-insensitive comparison is refused \
         by name: {ambiguous}"
    );
    drop(wire);

    let (endpoint, wire) = response_server(vec![statuses(serde_json::json!([
        {"id":"S-DONE","name":"Done","type":"completed","position":1.0}
    ]))]);
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
        serde_json::json!([{"id":"S-TODO","name":"Todo","type":"planned","position":1.0},{"id":"S-ODD","type":"planned","position":2.0}]),
        serde_json::json!([{"id":"S-TODO","name":"Todo","type":"planned","position":1.0},{"id":"S-ODD","name":7,"type":"planned","position":2.0}]),
    ] {
        // The create is here so that dropping the unreadable node would carry the write all
        // the way through: the refusal below is then the source rejecting the data, not the
        // fixture running out of answers.
        let (endpoint, wire) = response_server(vec![
            statuses(unreadable.clone()),
            serde_json::json!({"projectCreate":{"success":true,"project":{"id":"P-NEW"}}}),
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
    let relations = |nodes: Vec<serde_json::Value>, more: bool| serde_json::json!({"nodes":nodes,"pageInfo":{"hasNextPage":more,"endCursor":if more {Some("next")} else {None}}});
    let page = |nodes: Vec<serde_json::Value>, more: bool| serde_json::json!({"issue":{"relations":relations(nodes, more)}});
    // The first page is the rewrite's own answer; the second is read after it.
    let mut responses = vec![
        writable_resolution(),
        serde_json::json!({"issueUpdate":{"success":true,"issue":{"id":"I","relations":relations(
            (0..250)
                .map(|index| serde_json::json!({"id":format!("R{index}")}))
                .collect(),
            true,
        )}}}),
    ];
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
    // Mapped, because a status reads as a category only through the mapping: both recorded
    // issues are at a name it gives, so both come back from a filter asking for the two.
    let source = writable_source(&endpoint);
    let query = TaskQuery {
        labels: LabelFilter {
            any_of: vec!["Bug".into()],
            ..Default::default()
        },
        statuses: vec![StatusCategory::InProgress, StatusCategory::Todo],
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
    // The handle a person says out loud, beside the id and never instead of it: this is
    // the issue's own `identifier`, which the recorded fixture carries because Linear
    // declares the field non-null on every issue.
    assert_eq!(page.items[0].key.as_deref(), Some("ENG-1"));
    assert_eq!(page.items[0].id.0, "i1");
    assert_eq!(page.items[1].key.as_deref(), Some("ENG-2"));
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
    // And the read asks for it, so the value above is Linear's own rather than something
    // this source made up from the id it already had.
    assert!(wire.contains("identifier"), "{wire}");
    assert!(
        wire.contains(r#"{"or":[{"labels":{"some":{"name":{"eqIgnoreCase":"Bug"}}}}]}"#),
        "{wire}"
    );
    assert!(
        wire.contains(r#"{"or":[{"state":{"name":{"eqIgnoreCase":"In Progress"}}},{"state":{"name":{"eqIgnoreCase":"Todo"}}}]}"#),
        "{wire}"
    );
    assert_eq!(page.items[0].status.category, StatusCategory::InProgress);
    assert_eq!(page.items[1].status.category, StatusCategory::Todo);
    assert!(wire.contains("fixture-key"));
}

/// The label predicate goes on the wire in the shape Linear's `StringComparator` has.
///
/// It is asserted over the whole `filter` variable rather than by substring, because the
/// defect this covers was a member of that comparator that does not exist: this source sent
/// `inIgnoreCase` and Linear refused the read with HTTP 400 the first time a credentialed
/// run reached a label filter. A substring assertion for `eqIgnoreCase` would have passed
/// on the broken spelling too, since `all_of` sends that operator anyway.
#[tokio::test]
async fn a_label_predicate_uses_only_operators_linears_string_comparator_has() {
    let (endpoint, request) = server("200 OK", "", include_str!("fixtures/issues.json"));
    source(&endpoint)
        .query_tasks(
            &TaskQuery {
                labels: LabelFilter {
                    any_of: vec!["Bug".into(), "Chore".into()],
                    all_of: vec!["Core".into()],
                    none_of: vec!["Spike".into()],
                },
                ..Default::default()
            },
            &PageRequest {
                cursor: None,
                limit: 1,
            },
        )
        .await
        .expect("a read narrowed by every kind of label predicate");
    let wire = request.recv().unwrap();
    let body = wire
        .split_once("\r\n\r\n")
        .expect("the request has a body")
        .1;
    let sent: serde_json::Value = serde_json::from_str(body).expect("the body is JSON");
    assert_eq!(
        sent["variables"]["filter"],
        serde_json::json!({"and": [
            // "at least one of these" is a disjunction, because no member of
            // `StringComparator` takes a list case-insensitively.
            {"or": [
                {"labels": {"some": {"name": {"eqIgnoreCase": "Bug"}}}},
                {"labels": {"some": {"name": {"eqIgnoreCase": "Chore"}}}},
            ]},
            {"labels": {"some": {"name": {"eqIgnoreCase": "Core"}}}},
            {"labels": {"every": {"name": {"neqIgnoreCase": "Spike"}}}},
        ]}),
        "{wire}"
    );
    assert!(
        !wire.contains("inIgnoreCase"),
        "Linear defines no such member of StringComparator: {wire}"
    );
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

/// A validation refusal leads with Linear's own sentence, because that is what gets cut.
///
/// The envelope replayed here is the shape the real API answered a `projectRelationCreate`
/// with on 2026-09-04, identifiers replaced by invented ones of the same length: HTTP 200,
/// a `message` that is only the category name, and — after a `validationErrors` echoing
/// the whole rejected input back — the sentence naming the field and the values it would
/// have taken. That envelope is longer than the cut, which is asserted here too, so the
/// question is which part of it survives. The assertion is therefore not that the message
/// carries the sentence but that it carries it *first*: rendered raw the sentence lands
/// ahead of the echo only because this build sorts object keys and
/// `userPresentableMessage` sorts before `validationErrors`, which is a fact about
/// spelling rather than a decision anyone made.
#[tokio::test]
async fn a_validation_refusal_leads_with_the_sentence_that_names_the_field() {
    let (endpoint, _) = server(
        "200 OK",
        "",
        r#"{"errors":[{"message":"Argument Validation Error","path":["projectRelationCreate"],"extensions":{"code":"INVALID_INPUT","validationErrors":[{"target":{"type":"blocks","projectId":"11111111-2222-4333-8444-555555555555","anchorType":"start","relatedProjectId":"66666666-7777-4888-8999-aaaaaaaaaaaa","relatedAnchorType":"end"},"value":"blocks","property":"type","children":[],"constraints":{"isEnum":"type must be one of the following values: dependency"}}],"type":"invalid input","userError":true,"userPresentableMessage":"type must be one of the following values: dependency."}}]}"#,
    );
    let refused = source(&endpoint).health().await.unwrap_err();
    let SourceError::Refused { message } = refused else {
        panic!("a GraphQL error envelope is a refusal: {refused:?}");
    };
    let sentence = "type must be one of the following values: dependency.";
    let at = message
        .find(sentence)
        .unwrap_or_else(|| panic!("Linear's own sentence reaches the caller: {message}"));
    assert!(
        at < "Argument Validation Error: ".len() + 1,
        "and reaches it first, ahead of the echoed input: {message}"
    );
    assert!(
        message.starts_with("Argument Validation Error"),
        "the category name Linear led with is still named: {message}"
    );
    assert!(
        message.ends_with('\u{2026}'),
        "the envelope is long enough to be cut, which is why leading with the sentence \
         is what carries it: {message}"
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
    let issue = r#"{"data":{"issue":{"id":"i1","identifier":"ENG-1","title":"One","description":null,"url":null,"createdAt":null,"updatedAt":null,"state":{"name":"Backlog","type":"backlog"},"priority":0,"labels":{"nodes":[]},"project":null}}}"#;
    let (endpoint, wire) = server("200 OK", "", issue);
    let one = source(&endpoint)
        .get_task(&"i1".into())
        .await
        .unwrap()
        .unwrap();
    // A source without a mapping has no name for any category, so `Backlog` — whatever its
    // type — reads as unknown under its own name.
    assert_eq!(
        one.status,
        Status {
            category: StatusCategory::Unknown,
            name: "Backlog".into()
        }
    );
    // A single-issue read carries the handle too, and the native id is untouched beside
    // it: `ISSUE` and `ISSUES` select `identifier` alike, so one verb cannot report it
    // while the other does not.
    assert_eq!(one.key.as_deref(), Some("ENG-1"));
    assert_eq!(one.id.0, "i1");
    assert!(wire.recv().unwrap().contains("identifier"));
    let project = r#"{"data":{"project":{"id":"p1","name":"One","description":null,"url":null,"createdAt":null,"updatedAt":null,"status":{"name":"Done","type":"completed"},"labels":{"nodes":[]}}}}"#;
    let (endpoint, _) = server("200 OK", "", project);
    // Through the project side of a mapping naming `Done`: what a project reads as is its
    // kind's mapping, never its `ProjectStatusType`.
    assert_eq!(
        writable_source(&endpoint)
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
    let body = r#"{"data":{"issues":{"nodes":[{"id":"a","identifier":"ENG-1","title":"A","description":null,"url":null,"createdAt":null,"updatedAt":null,"project":null,"state":{"name":"Todo","type":"unstarted"},"priority":0,"labels":{"nodes":[]}}, {"id":"b","identifier":"ENG-2","title":"B","description":null,"url":null,"createdAt":null,"updatedAt":null,"project":null,"state":{"name":"Doing","type":"started"},"priority":0,"labels":{"nodes":[]}}, {"id":"c","identifier":"ENG-3","title":"C","description":null,"url":null,"createdAt":null,"updatedAt":null,"project":null,"state":{"name":"Canceled","type":"canceled"},"priority":0,"labels":{"nodes":[]}}, {"id":"d","identifier":"ENG-4","title":"D","description":null,"url":null,"createdAt":null,"updatedAt":null,"project":null,"state":{"name":"Odd","type":"new-value"},"priority":0,"labels":{"nodes":[]}}],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}"#;
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
    assert!(
        wire.contains("every")
            && wire.contains("null")
            && wire.contains(&onetaskgraph_linear::MAX_PAGE_SIZE.to_string())
    );
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
    writable_source(&endpoint)
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
    assert!(
        wire.recv()
            .unwrap()
            .contains(r#"{"status":{"name":{"eqIgnoreCase":"In Progress"}}}"#)
    );
    let caps = source("http://127.0.0.1:1").capabilities();
    assert_eq!(caps.max_page_size, onetaskgraph_linear::MAX_PAGE_SIZE);
    assert!(matches!(
        caps.search_title,
        onetaskgraph_plugin_api::Support::Native
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
    let valid_task = serde_json::json!({"id":"i","title":"t","description":null,"url":null,"createdAt":null,"updatedAt":null,"project":null,"state":{"name":"x","type":"started"},"priority":0,"labels":{"nodes":[]}});
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
        r#"{"data":{"issues":{"nodes":[{"id":"i","title":"t","state":{"name":"x","type":"started"},"priority":0,"labels":{}}],"pageInfo":{}}}}"#,
        // An issue with no `identifier`. Linear declares it `String!` and both read
        // operations select it, so a response without one is a response this source cannot
        // read — not an issue with no handle, which Linear has no way to be.
        r#"{"data":{"issues":{"nodes":[{"id":"i","title":"t","description":null,"url":null,"createdAt":null,"updatedAt":null,"state":{"name":"x","type":"started"},"priority":0,"labels":{"nodes":[]},"project":null}],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}"#,
        // And one whose identifier is not a string, which is the other half: present is
        // not the same as readable.
        r#"{"data":{"issues":{"nodes":[{"id":"i","identifier":7,"title":"t","description":null,"url":null,"createdAt":null,"updatedAt":null,"state":{"name":"x","type":"started"},"priority":0,"labels":{"nodes":[]},"project":null}],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}"#,
    ] {
        let (endpoint, _) = server("200 OK", "", body);
        // Named per body, so a shape this source stopped refusing says which one it was.
        let read = source(&endpoint)
            .query_tasks(&TaskQuery::default(), &request)
            .await;
        assert!(
            matches!(read, Err(SourceError::Malformed { .. })),
            "{body} was read as {read:?}"
        );
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
        r#"{"data":{"issue":{"id":"i","identifier":"ENG-1","title":"t","createdAt":"yesterday","state":{"name":"x","type":"started"},"priority":0,"labels":{"nodes":[]}}}}"#,
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
    // `related` is the issue vocabulary and only the issue vocabulary: Linear's validator
    // enumerates a project relation's type as `dependency` alone, so a project relation
    // typed `related` is a shape this workspace cannot hold and is refused rather than
    // read as an edge this source could never have written.
    let (endpoint, _) = server(
        "200 OK",
        "",
        r#"{"data":{"project":{"relations":{"nodes":[{"type":"related","relatedProject":{"id":"other"}}],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}}"#,
    );
    assert!(matches!(
        source(&endpoint)
            .project_dependencies(&"p".into(), Direction::DependsOn, &request)
            .await
            .unwrap_err(),
        SourceError::Malformed { ref message }
            if message.contains("related") && message.contains("project")
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
async fn a_document_reads_back_the_slot_linear_escaped_when_it_stored_the_content() {
    // Live-captured on 2026-09-14: Linear keeps a document's content as Markdown and hands
    // the slot this source wrote back with its close escaped, which the live lane's own
    // read-back was refused on as an unterminated slot.
    let (endpoint, _) = response_server(vec![serde_json::json!({"document":{"id":"d1",
        "title":"Fixture design note",
        "content":"Body\n\n<!-- onetaskgraph.metadata\n{\"caller.count\":3}\n\\-->",
        "url":"https://linear.app/acme/document/d1","createdAt":null,"updatedAt":null,
        "project":{"id":"p1"}}})]);
    let shown = source(&endpoint)
        .get_document(&"d1".into())
        .await
        .expect("a slot whose close Linear escaped still reads")
        .expect("and is there");
    assert_eq!(shown.content.as_deref(), Some("Body"));
    assert_eq!(shown.metadata["caller.count"], serde_json::json!(3));
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
        writable_resolution(),
        serde_json::json!({"documentCreate":{"success":true,"document":{"id":"D-LOOSE"}}}),
    ]);
    writable_source(&endpoint)
        .write_document(&written("Loose", None, None))
        .await
        .expect("the orphan document is created");
    assert!(wire.recv().unwrap().contains("teams(first:2,filter:"));
    let request = wire.recv().unwrap();
    assert!(request.contains(r#""teamId":"TEAM""#), "{request}");
    // And no `projectId` at all, not a null one. `documentCreate` refuses an input naming
    // more than one home — `Exactly one of initiativeId, teamId, issueId, releaseId,
    // cycleId or projectId must be defined.` — and it counts a key that is *present*:
    // observed on 2026-09-04, `{projectId: null, teamId: …}` is refused where `{teamId: …}`
    // is accepted. A null cannot be spelled out of that, so the key has to go.
    assert!(
        !request.contains("projectId"),
        "a null home is a home as far as Linear's validator is concerned: {request}"
    );

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

    // An update keeps its explicit null, and that is the opposite rule for the opposite
    // reason: there the null is the instruction. It is how a document is moved out of a
    // project, and omitting the key would leave it where it was — Linear answered
    // `project: null` to exactly that update on 2026-09-04.
    let (endpoint, wire) = response_server(vec![
        document("D-NEW"),
        serde_json::json!({"documentUpdate":{"success":true,"document":{"id":"D-NEW"}}}),
    ]);
    writable_source(&endpoint)
        .write_document(&ItemWrite {
            target: Some("D-NEW".into()),
            ..written("Design", Some("Revised"), None)
        })
        .await
        .expect("the document is moved out of its project");
    assert!(wire.recv().unwrap().contains("document(id:$id)"));
    let request = wire.recv().unwrap();
    assert!(request.contains(r#""projectId":null"#), "{request}");

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

/// The JSON body of one request the loopback server recorded.
fn sent(request: &str) -> serde_json::Value {
    let body = request
        .split_once("\r\n\r\n")
        .expect("the recorded request carries a body")
        .1;
    serde_json::from_str(body).expect("the body is JSON")
}

/// The `identifier` Linear gives one issue, derived from its id so a fixture and what a
/// test asserts about it cannot drift.
///
/// Linear declares `Issue.identifier` as `String!` and this source never parses one, so
/// any distinct string is a faithful stand-in; what would not be faithful is two issues in
/// one answer sharing it.
fn identifier(id: &str) -> String {
    format!("ENG-{}", id.replace('-', ""))
}

/// What `issue(id:)` answers for an issue this workspace holds under the backend id `id`.
///
/// Every comment write resolves its task through `get_task` first, so every one of them
/// begins with this answer.
fn held_issue(id: &str) -> serde_json::Value {
    serde_json::json!({"issue":{"id":id,"identifier":identifier(id),"title":"Fixture issue",
        "description":null,"url":null,
        "createdAt":null,"updatedAt":null,"archivedAt":null,
        "state":{"name":"Todo","type":"unstarted"},"priority":0,"labels":{"nodes":[]},"project":null}})
}

/// One comment as Linear's `Comment` selection answers it, written by the user `ada`.
fn comment_node(id: &str, body: &str, updated_at: &str) -> serde_json::Value {
    serde_json::json!({"id":id,"body":body,
        "url":format!("https://linear.app/acme/issue/ENG-1/fixture-issue#comment-{id}"),
        "createdAt":"2026-08-01T12:00:00Z","updatedAt":updated_at,
        "user":{"displayName":"ada"}})
}

fn at(timestamp: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    Some(timestamp.parse().expect("a fixture timestamp"))
}

/// Oldest first across pages, walked from the far end of a connection Linear lists newest
/// first — which is the only way the order holds past the first page: reversing each page of
/// a forward walk would put the newest page first.
#[tokio::test]
async fn comments_are_read_oldest_first_across_pages_over_real_http() {
    let first_page =
        serde_json::from_str::<serde_json::Value>(include_str!("fixtures/comments.json")).unwrap()
            ["data"]
            .clone();
    let (endpoint, wire) = response_server(vec![
        first_page,
        serde_json::json!({"issue":{"archivedAt":null,"comments":{
            "nodes":[comment_node("c3","Third, and newest.","2026-08-04T12:00:00Z")],
            "pageInfo":{"hasPreviousPage":false,"startCursor":"cursor-c3"}}}}),
    ]);
    let source = source(&endpoint);
    let first = source
        .task_comments(
            &"i1".into(),
            &PageRequest {
                cursor: None,
                limit: 2,
            },
        )
        .await
        .expect("the comments read")
        .expect("the task is there");
    assert_eq!(
        first.items,
        vec![
            Comment {
                id: "c1".into(),
                // An integration wrote it, and Linear names no user for one.
                author: None,
                created_at: at("2026-08-01T12:00:00Z"),
                updated_at: at("2026-08-01T12:00:00Z"),
                body: "First, written by an integration.\n".into(),
                url: Some("https://linear.app/acme/issue/ENG-1/fixture-issue#comment-c1".into()),
            },
            Comment {
                id: "c2".into(),
                author: Some("ada".into()),
                created_at: at("2026-08-02T12:00:00Z"),
                updated_at: at("2026-08-03T09:30:00Z"),
                body: "Second, and the newer of this page.".into(),
                url: Some("https://linear.app/acme/issue/ENG-1/fixture-issue#comment-c2".into()),
            },
        ],
        "Linear answered the page newest first, and it is reported oldest first"
    );
    assert_eq!(
        first.next,
        Some(Cursor("cursor-c2".into())),
        "the next page is the one behind this one"
    );
    let request = sent(&wire.recv().unwrap());
    assert_eq!(
        request["query"],
        onetaskgraph_linear::graphql::ISSUE_COMMENTS
    );
    assert_eq!(
        request["variables"],
        serde_json::json!({"id":"i1","last":2,"before":null})
    );

    let second = source
        .task_comments(
            &"i1".into(),
            &PageRequest {
                cursor: first.next.clone(),
                limit: 2,
            },
        )
        .await
        .expect("the older page reads")
        .expect("the task is still there");
    assert_eq!(
        sent(&wire.recv().unwrap())["variables"],
        serde_json::json!({"id":"i1","last":2,"before":"cursor-c2"})
    );
    assert_eq!(second.next, None, "nothing is older than the last page");
    let walked = first
        .items
        .iter()
        .chain(&second.items)
        .map(|comment| comment.id.0.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        walked,
        ["c1", "c2", "c3"],
        "oldest first across the whole walk"
    );
}

#[tokio::test]
async fn a_comment_read_answers_for_the_issue_as_linear_holds_it() {
    let empty = serde_json::json!({"issue":{"archivedAt":null,"comments":{"nodes":[],
        "pageInfo":{"hasPreviousPage":false,"startCursor":null}}}});
    let (endpoint, wire) = response_server(vec![
        empty,
        serde_json::json!({ "issue": serde_json::Value::Null }),
        serde_json::json!({"issue":{"archivedAt":"2026-09-04T00:00:00Z","comments":{"nodes":[],
            "pageInfo":{"hasPreviousPage":false,"startCursor":null}}}}),
    ]);
    let source = source(&endpoint);
    let page = PageRequest {
        cursor: None,
        limit: 500,
    };

    let none_yet = source
        .task_comments(&"i1".into(), &page)
        .await
        .expect("an issue with no comments reads")
        .expect("and is an issue this source holds");
    assert!(none_yet.items.is_empty() && none_yet.next.is_none());
    assert_eq!(
        sent(&wire.recv().unwrap())["variables"]["last"],
        onetaskgraph_linear::MAX_PAGE_SIZE,
        "a page is never asked for past the declared maximum"
    );

    assert!(
        source
            .task_comments(&"never-there".into(), &page)
            .await
            .expect("an id naming nothing is an answer")
            .is_none(),
        "no such task is no page at all, not an empty one"
    );
    assert!(
        source
            .task_comments(&"trashed".into(), &page)
            .await
            .expect("a trashed issue is an answer")
            .is_none(),
        "a trashed issue is not one this source holds, as it is not for `get_task`"
    );

    // The two reads above each sent their one request; taken off the wire here so that what
    // is left on it afterwards is only what the refusal below would have sent.
    for _ in 0..2 {
        wire.recv().expect("each read above sent its request");
    }

    // A page of no rows is refused where it arrives, and Linear is never asked for one: an
    // empty page back would read as an issue with no comments.
    let refused = source
        .task_comments(
            &"i1".into(),
            &PageRequest {
                cursor: None,
                limit: 0,
            },
        )
        .await
        .expect_err("a page limit of zero is not a page");
    assert!(
        matches!(&refused, SourceError::Config { message } if message.contains("limit of 0")),
        "{refused:?}"
    );
    assert!(
        wire.try_recv().is_err(),
        "nothing was sent for a page of no rows"
    );
}

#[tokio::test]
async fn a_comment_is_added_edited_and_removed_over_real_http() {
    // A body quoting a stack trace, ending in a newline: only useful byte for byte.
    let body = "Seen again on main.\n\n    at engine::copy (copy.rs:42)\n";
    let (endpoint, wire) = response_server(vec![
        held_issue("i1-backend"),
        serde_json::json!({"commentCreate":{"success":true,
            "comment":comment_node("c7", body, "2026-08-01T12:00:00Z")}}),
    ]);
    let added = writable_source(&endpoint)
        .add_comment(
            &"ENG-1".into(),
            &NewComment {
                body: CommentBody::new(body).unwrap(),
                author: None,
            },
        )
        .await
        .expect("the comment is added")
        .expect("on a task this source holds");
    assert_eq!(added.id, NativeId::from("c7"));
    assert_eq!(added.body, body);
    assert_eq!(added.author.as_deref(), Some("ada"));
    let lookup = sent(&wire.recv().unwrap());
    assert_eq!(lookup["query"], onetaskgraph_linear::graphql::ISSUE);
    assert_eq!(lookup["variables"], serde_json::json!({"id":"ENG-1"}));
    let create = sent(&wire.recv().unwrap());
    assert_eq!(
        create["query"],
        onetaskgraph_linear::graphql::COMMENT_CREATE
    );
    assert_eq!(
        create["variables"],
        serde_json::json!({"input":{"issueId":"i1-backend","body":body}}),
        "created on the backend id Linear resolved, with the body exactly as given and nothing \
         else — no author member at all"
    );

    let revised = "Seen again on main, twice.\n";
    let (endpoint, wire) = response_server(vec![
        held_issue("i1-backend"),
        serde_json::json!({"comment":{"id":"c7","archivedAt":null,"issue":{"id":"i1-backend"}}}),
        serde_json::json!({"commentUpdate":{"success":true,
            "comment":comment_node("c7", revised, "2026-08-05T08:00:00Z")}}),
    ]);
    let edited = writable_source(&endpoint)
        .edit_comment(
            &"ENG-1".into(),
            &"c7".into(),
            &CommentBody::new(revised).unwrap(),
        )
        .await
        .expect("the comment is edited")
        .expect("it is on this task");
    assert_eq!(
        (&edited.id, &edited.author, edited.created_at),
        (&added.id, &added.author, added.created_at),
        "the id, the author and the time it was written are the comment's own"
    );
    assert_eq!(edited.body, revised);
    assert_ne!(edited.updated_at, added.updated_at);
    assert_eq!(
        sent(&wire.recv().unwrap())["query"],
        onetaskgraph_linear::graphql::ISSUE
    );
    let placed = sent(&wire.recv().unwrap());
    assert_eq!(placed["query"], onetaskgraph_linear::graphql::COMMENT);
    assert_eq!(placed["variables"], serde_json::json!({"id":"c7"}));
    let update = sent(&wire.recv().unwrap());
    assert_eq!(
        update["query"],
        onetaskgraph_linear::graphql::COMMENT_UPDATE
    );
    assert_eq!(
        update["variables"],
        serde_json::json!({"id":"c7","input":{"body":revised}}),
        "only the body is sent, so nothing else is Linear's to move"
    );

    let (endpoint, wire) = response_server(vec![
        held_issue("i1-backend"),
        serde_json::json!({"comment":{"id":"c7","archivedAt":null,"issue":{"id":"i1-backend"}}}),
        serde_json::json!({"commentDelete":{"success":true}}),
    ]);
    let removed = writable_source(&endpoint)
        .delete_comment(&"ENG-1".into(), &"c7".into())
        .await
        .expect("the comment is removed");
    assert_eq!(removed, Some(NativeId::from("c7")));
    wire.recv().unwrap();
    wire.recv().unwrap();
    let delete = sent(&wire.recv().unwrap());
    assert_eq!(
        delete["query"],
        onetaskgraph_linear::graphql::COMMENT_DELETE
    );
    assert_eq!(delete["variables"], serde_json::json!({"id":"c7"}));
}

/// Nothing Linear does not hold on this task is edited or removed — and no mutation is sent
/// to find that out, because `commentUpdate` and `commentDelete` address a comment by its id
/// alone and would change a comment on any issue.
#[tokio::test]
async fn a_comment_this_task_does_not_have_answers_none_and_nothing_is_written() {
    let quiet = |wire: &mpsc::Receiver<String>, what: &str| {
        assert!(
            wire.recv_timeout(std::time::Duration::from_millis(200))
                .is_err(),
            "{what}: nothing was sent past the lookups"
        );
    };
    let body = || CommentBody::new("text").unwrap();

    // No such task: each call stops at the issue lookup.
    for call in ["add", "edit", "delete"] {
        let (endpoint, wire) = response_server(vec![
            serde_json::json!({ "issue": serde_json::Value::Null }),
        ]);
        let source = writable_source(&endpoint);
        let answered = match call {
            "add" => source
                .add_comment(
                    &"never-there".into(),
                    &NewComment {
                        body: body(),
                        author: None,
                    },
                )
                .await
                .map(|comment| comment.is_none()),
            "edit" => source
                .edit_comment(&"never-there".into(), &"c1".into(), &body())
                .await
                .map(|comment| comment.is_none()),
            _ => source
                .delete_comment(&"never-there".into(), &"c1".into())
                .await
                .map(|comment| comment.is_none()),
        };
        assert!(
            answered.expect("an id naming nothing is an answer"),
            "{call}"
        );
        assert_eq!(
            sent(&wire.recv().unwrap())["query"],
            onetaskgraph_linear::graphql::ISSUE
        );
        quiet(&wire, call);
    }

    // A task that is there, and a comment id that is not one of its comments.
    for (case, placed) in [
        ("no such comment", serde_json::Value::Null),
        (
            "a comment on another issue",
            serde_json::json!({"id":"c9","archivedAt":null,"issue":{"id":"i2"}}),
        ),
        (
            "a comment on no issue at all",
            serde_json::json!({"id":"c9","archivedAt":null,"issue":null}),
        ),
        (
            "a trashed comment",
            serde_json::json!({"id":"c9","archivedAt":"2026-09-04T00:00:00Z","issue":{"id":"i1"}}),
        ),
    ] {
        for delete in [false, true] {
            let (endpoint, wire) = response_server(vec![
                held_issue("i1"),
                serde_json::json!({ "comment": placed }),
            ]);
            let source = writable_source(&endpoint);
            let none = if delete {
                source
                    .delete_comment(&"i1".into(), &"c9".into())
                    .await
                    .expect(case)
                    .is_none()
            } else {
                source
                    .edit_comment(&"i1".into(), &"c9".into(), &body())
                    .await
                    .expect(case)
                    .is_none()
            };
            assert!(none, "{case}: this task has no such comment");
            wire.recv().unwrap();
            assert_eq!(
                sent(&wire.recv().unwrap())["query"],
                onetaskgraph_linear::graphql::COMMENT
            );
            quiet(&wire, case);
        }
    }
}

#[tokio::test]
async fn an_author_is_refused_before_any_request_is_sent() {
    let (endpoint, wire) = response_server(Vec::new());
    let refusal = writable_source(&endpoint)
        .add_comment(
            &"i1".into(),
            &NewComment {
                body: CommentBody::new("text").unwrap(),
                author: Some("grace".into()),
            },
        )
        .await
        .expect_err("an author Linear cannot record is refused rather than dropped");
    assert!(
        matches!(&refusal, SourceError::Refused { message }
            if message.contains("work")
                && message.contains("grace")
                && message.contains("API key")
                && message.contains("--author")),
        "the refusal names the source, the author, why, and what to do instead: {refusal:?}"
    );
    assert!(
        wire.recv_timeout(std::time::Duration::from_millis(200))
            .is_err(),
        "nothing reached Linear"
    );
}

#[tokio::test]
async fn comment_failures_cross_the_http_boundary() {
    // Linear's own refusal comes back as this source's usual refusal, carrying what it said.
    let (endpoint, _) = server(
        "200 OK",
        "",
        r#"{"errors":[{"message":"Linear could not answer that"}]}"#,
    );
    let failure = source(&endpoint)
        .task_comments(
            &"i1".into(),
            &PageRequest {
                cursor: None,
                limit: 5,
            },
        )
        .await
        .expect_err("an errored response is not a page");
    assert!(
        matches!(&failure, SourceError::Refused { message }
            if message.contains("Linear could not answer that")),
        "{failure:?}"
    );
    let (endpoint, wire) = server(
        "200 OK",
        "",
        r#"{"errors":[{"message":"Linear could not answer that"}]}"#,
    );
    let failure = writable_source(&endpoint)
        .delete_comment(&"i1".into(), &"c1".into())
        .await
        .expect_err("an errored lookup is not a removal");
    assert!(
        matches!(failure, SourceError::Refused { .. }),
        "{failure:?}"
    );
    wire.recv().unwrap();

    // A mutation Linear reports unsuccessful is refused, at both ends it can happen.
    let (endpoint, _) = response_server(vec![
        held_issue("i1"),
        serde_json::json!({"commentCreate":{"success":false,"comment":null}}),
    ]);
    let failure = writable_source(&endpoint)
        .add_comment(
            &"i1".into(),
            &NewComment {
                body: CommentBody::new("text").unwrap(),
                author: None,
            },
        )
        .await
        .expect_err("an unsuccessful create is not a comment");
    assert!(
        matches!(&failure, SourceError::Refused { message } if message.contains("commentCreate")),
        "{failure:?}"
    );
    let (endpoint, _) = response_server(vec![
        held_issue("i1"),
        serde_json::json!({"comment":{"id":"c1","archivedAt":null,"issue":{"id":"i1"}}}),
        serde_json::json!({"commentDelete":{"success":false}}),
    ]);
    let failure = writable_source(&endpoint)
        .delete_comment(&"i1".into(), &"c1".into())
        .await
        .expect_err("an unsuccessful removal is not a removal");
    assert!(
        matches!(&failure, SourceError::Refused { message } if message.contains("commentDelete")),
        "{failure:?}"
    );
}

#[tokio::test]
async fn malformed_comment_shapes_are_rejected_rather_than_read_past() {
    let page = |nodes: serde_json::Value, info: serde_json::Value| serde_json::json!({"issue":{"archivedAt":null,"comments":{"nodes":nodes,"pageInfo":info}}});
    let last = serde_json::json!({"hasPreviousPage":false,"startCursor":null});
    let node = comment_node("c1", "text", "2026-08-01T12:00:00Z");
    let without = |key: &str| {
        let mut node = node.clone();
        node.as_object_mut().unwrap().remove(key);
        node
    };
    for (description, body) in [
        (
            "a node with no body",
            page(serde_json::json!([without("body")]), last.clone()),
        ),
        (
            "a node with no user field",
            page(serde_json::json!([without("user")]), last.clone()),
        ),
        (
            "a user with no display name",
            page(
                serde_json::json!([{"user":{},"id":"c1","body":"text","url":null,
                "createdAt":null,"updatedAt":null}]),
                last.clone(),
            ),
        ),
        (
            "an empty comment id",
            page(
                serde_json::json!([{"id":"","body":"text","url":null,"createdAt":null,
                "updatedAt":null,"user":null}]),
                last.clone(),
            ),
        ),
        (
            "a timestamp that is not one",
            page(
                serde_json::json!([{"id":"c1","body":"text","url":null,"createdAt":"yesterday",
                "updatedAt":null,"user":null}]),
                last.clone(),
            ),
        ),
        (
            "no comments connection",
            serde_json::json!({"issue":{"archivedAt":null}}),
        ),
        (
            "no comment nodes",
            serde_json::json!({"issue":{"archivedAt":null,"comments":{"pageInfo":last}}}),
        ),
        (
            "no pageInfo",
            serde_json::json!({"issue":{"archivedAt":null,"comments":{"nodes":[]}}}),
        ),
        (
            "no hasPreviousPage",
            page(
                serde_json::json!([]),
                serde_json::json!({"startCursor":null}),
            ),
        ),
        (
            "an older page with no cursor to reach it by",
            page(
                serde_json::json!([]),
                serde_json::json!({"hasPreviousPage":true,"startCursor":null}),
            ),
        ),
    ] {
        let (endpoint, _) = response_server(vec![body]);
        let failure = source(&endpoint)
            .task_comments(
                &"i1".into(),
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

    // The two write-side shapes: a payload with no comment, and a comment lookup that does
    // not say which issue it is on.
    let (endpoint, _) = response_server(vec![
        held_issue("i1"),
        serde_json::json!({"commentCreate":{"success":true}}),
    ]);
    let failure = writable_source(&endpoint)
        .add_comment(
            &"i1".into(),
            &NewComment {
                body: CommentBody::new("text").unwrap(),
                author: None,
            },
        )
        .await
        .expect_err("a create answering no comment");
    assert!(
        matches!(failure, SourceError::Malformed { .. }),
        "{failure:?}"
    );
    for (description, placed) in [
        (
            "no issue field",
            serde_json::json!({"id":"c1","archivedAt":null}),
        ),
        (
            "an issue with an empty id",
            serde_json::json!({"id":"c1","archivedAt":null,"issue":{"id":""}}),
        ),
    ] {
        let (endpoint, _) = response_server(vec![
            held_issue("i1"),
            serde_json::json!({ "comment": placed }),
        ]);
        let failure = writable_source(&endpoint)
            .edit_comment(
                &"i1".into(),
                &"c1".into(),
                &CommentBody::new("text").unwrap(),
            )
            .await
            .expect_err(description);
        assert!(
            matches!(failure, SourceError::Malformed { .. }),
            "{description}: {failure:?}"
        );
    }
}

/// What `project(id:)` answers for one project carrying `description`.
fn held_project(id: &str, description: &str) -> serde_json::Value {
    serde_json::json!({"project":{"id":id,"name":"Fixture project","description":description,
        "url":"https://linear.app/acme/project/p1","createdAt":null,"updatedAt":null,
        "archivedAt":null,"status":{"name":"Started","type":"started"},
        "labels":{"nodes":[{"id":"pl","name":"roadmap","color":null}]}}})
}

/// What `document(id:)` answers for one document carrying `content`.
fn held_document(id: &str, content: &str) -> serde_json::Value {
    serde_json::json!({"document":{"id":id,"title":"Fixture document","content":content,
        "url":"https://linear.app/acme/document/d1","createdAt":null,"updatedAt":null,
        "archivedAt":null,"project":{"id":"p1"}}})
}

/// One metadata key of a task, a project and a document is set by one read and one update
/// carrying the long-form field alone — `description`, `description` and `content` — which
/// differs from what Linear holds only inside the trailing slot: every byte above it, the
/// person's own spacing included, goes back as it was, and so does every other key. The
/// answer is the record read back, and a key already holding the value is the read alone.
#[tokio::test]
async fn a_metadata_key_is_set_by_rewriting_the_slot_alone_for_every_record() {
    let key = MetadataKey::new("myapp.review").expect("a caller key");
    let value = serde_json::json!({"approved": true});
    // Linear escapes a line opening `-->` when it stores a field, so the held slot closes the
    // way Linear hands it back; the rewritten one is sent in this source's one encoding.
    let held = "Prose a person wrote,\n\n  with its own spacing.\n\n<!-- onetaskgraph.metadata\n{\"caller.kept\":[1]}\n\\-->";
    let sent_back = "Prose a person wrote,\n\n  with its own spacing.\n\n<!-- onetaskgraph.metadata `{\"caller.kept\":[1],\"myapp.review\":{\"approved\":true}}` -->";

    let (endpoint, wire) = response_server(vec![
        prioritised_issue("i1", Some(held), serde_json::json!(2)),
        serde_json::json!({"issueUpdate":{"success":true,"issue":{"id":"i1"}}}),
        prioritised_issue("i1", Some(sent_back), serde_json::json!(2)),
    ]);
    let task = writable_source(&endpoint)
        .set_task_metadata(&"i1".into(), &key, &value)
        .await
        .unwrap()
        .expect("the issue is held");
    assert_eq!(task.metadata.get(key.as_str()), Some(&value));
    assert_eq!(
        task.metadata.get("caller.kept"),
        Some(&serde_json::json!([1]))
    );
    let requests = wire
        .try_iter()
        .map(|request| sent(&request))
        .collect::<Vec<_>>();
    assert_eq!(requests.len(), 3, "{requests:?}");
    assert_eq!(
        requests[1]["query"],
        onetaskgraph_linear::graphql::ISSUE_UPDATE
    );
    assert_eq!(
        requests[1]["variables"],
        serde_json::json!({"id":"i1","input":{"description":sent_back}})
    );

    let (endpoint, wire) = response_server(vec![
        held_project("p1", held),
        serde_json::json!({"projectUpdate":{"success":true,"project":{"id":"p1"}}}),
        held_project("p1", sent_back),
    ]);
    let project = writable_source(&endpoint)
        .set_project_metadata(&"p1".into(), &key, &value)
        .await
        .unwrap()
        .expect("the project is held");
    assert_eq!(project.metadata.get(key.as_str()), Some(&value));
    let requests = wire
        .try_iter()
        .map(|request| sent(&request))
        .collect::<Vec<_>>();
    assert_eq!(
        requests[1]["query"],
        onetaskgraph_linear::graphql::PROJECT_UPDATE
    );
    assert_eq!(
        requests[1]["variables"],
        serde_json::json!({"id":"p1","input":{"description":sent_back}})
    );

    let (endpoint, wire) = response_server(vec![
        held_document("d1", held),
        serde_json::json!({"documentUpdate":{"success":true,"document":{"id":"d1"}}}),
        held_document("d1", sent_back),
    ]);
    let document = writable_source(&endpoint)
        .set_document_metadata(&"d1".into(), &key, &value)
        .await
        .unwrap()
        .expect("the document is held");
    assert_eq!(document.metadata.get(key.as_str()), Some(&value));
    let requests = wire
        .try_iter()
        .map(|request| sent(&request))
        .collect::<Vec<_>>();
    assert_eq!(
        requests[1]["query"],
        onetaskgraph_linear::graphql::DOCUMENT_UPDATE
    );
    assert_eq!(
        requests[1]["variables"],
        serde_json::json!({"id":"d1","input":{"content":sent_back}})
    );

    // A description holding no slot gains one after a blank line, and nothing above it moves.
    let (endpoint, wire) = response_server(vec![
        prioritised_issue("i1", Some("Bare prose\n"), serde_json::json!(0)),
        serde_json::json!({"issueUpdate":{"success":true,"issue":{"id":"i1"}}}),
        prioritised_issue("i1", Some("Bare prose\n"), serde_json::json!(0)),
    ]);
    writable_source(&endpoint)
        .set_task_metadata(&"i1".into(), &key, &value)
        .await
        .unwrap();
    let requests = wire
        .try_iter()
        .map(|request| sent(&request))
        .collect::<Vec<_>>();
    assert_eq!(
        requests[1]["variables"]["input"]["description"],
        serde_json::json!(
            "Bare prose\n\n\n<!-- onetaskgraph.metadata `{\"myapp.review\":{\"approved\":true}}` -->"
        )
    );

    // Already holding the value: the read alone, and the record as it was read.
    let (endpoint, wire) = response_server(vec![prioritised_issue(
        "i1",
        Some(sent_back),
        serde_json::json!(2),
    )]);
    writable_source(&endpoint)
        .set_task_metadata(&"i1".into(), &key, &value)
        .await
        .unwrap()
        .expect("the issue is held");
    assert_eq!(wire.try_iter().count(), 1, "the read alone");

    // A record this source does not hold is none, with nothing written.
    for record in ["issue", "project", "document"] {
        let (endpoint, wire) = response_server(vec![serde_json::json!({ (record): null })]);
        let source = writable_source(&endpoint);
        let held = match record {
            "issue" => source
                .set_task_metadata(&"x".into(), &key, &value)
                .await
                .unwrap()
                .is_some(),
            "project" => source
                .set_project_metadata(&"x".into(), &key, &value)
                .await
                .unwrap()
                .is_some(),
            _ => source
                .set_document_metadata(&"x".into(), &key, &value)
                .await
                .unwrap()
                .is_some(),
        };
        assert!(!held, "{record}");
        assert_eq!(wire.try_iter().count(), 1, "{record}: the read alone");
    }
}

/// A rendering replaces the content and the slot's `onetaskgraph.template` entry together in
/// one update of the long-form field alone, every other slot entry kept.
#[tokio::test]
async fn a_rendering_replaces_the_content_and_its_provenance_in_one_write() {
    let held = "Old rendering\n\n<!-- onetaskgraph.metadata\n{\"caller.kept\":[1],\"onetaskgraph.template\":{\"digest\":\"old\"}}\n-->";
    let provenance = serde_json::json!({"digest":"new"});
    let expected = "New rendering\n\n<!-- onetaskgraph.metadata `{\"caller.kept\":[1],\"onetaskgraph.template\":{\"digest\":\"new\"}}` -->";
    let (endpoint, wire) = response_server(vec![
        prioritised_issue("i1", Some(held), serde_json::json!(2)),
        serde_json::json!({"issueUpdate":{"success":true,"issue":{"id":"i1"}}}),
    ]);
    writable_source(&endpoint)
        .set_task_rendering(
            &"i1".into(),
            "New rendering",
            &provenance,
            &std::collections::BTreeMap::new(),
        )
        .await
        .unwrap()
        .expect("the issue is held");
    let requests = wire
        .try_iter()
        .map(|request| sent(&request))
        .collect::<Vec<_>>();
    assert_eq!(
        requests[1]["variables"],
        serde_json::json!({"id":"i1","input":{"description":expected}})
    );

    let (endpoint, wire) = response_server(vec![
        held_document("d1", held),
        serde_json::json!({"documentUpdate":{"success":true,"document":{"id":"d1"}}}),
    ]);
    writable_source(&endpoint)
        .set_document_rendering(
            &"d1".into(),
            "New rendering",
            &provenance,
            &std::collections::BTreeMap::new(),
        )
        .await
        .unwrap()
        .expect("the document is held");
    let requests = wire
        .try_iter()
        .map(|request| sent(&request))
        .collect::<Vec<_>>();
    assert_eq!(
        requests[1]["variables"],
        serde_json::json!({"id":"d1","input":{"content":expected}})
    );

    // Content that itself ends in something shaped like the slot is still content: the
    // source's own slot is written after it, and a read takes only the last one off.
    let quoted = "Quotes a slot\n\n<!-- onetaskgraph.metadata\n{}\n-->";
    let (endpoint, wire) = response_server(vec![
        prioritised_issue("i1", Some(held), serde_json::json!(2)),
        serde_json::json!({"issueUpdate":{"success":true,"issue":{"id":"i1"}}}),
    ]);
    writable_source(&endpoint)
        .set_task_rendering(
            &"i1".into(),
            quoted,
            &provenance,
            &std::collections::BTreeMap::new(),
        )
        .await
        .unwrap()
        .expect("the issue is held");
    let requests = wire
        .try_iter()
        .map(|request| sent(&request))
        .collect::<Vec<_>>();
    let description = requests[1]["variables"]["input"]["description"]
        .as_str()
        .expect("a description");
    assert!(description.starts_with(&format!("{quoted}\n\n<!-- onetaskgraph.metadata `")));
}

/// `body` with its first node's long-form field replaced by `visible` and a metadata slot
/// holding `slot`.
fn with_slot(
    fixture: &str,
    root: &str,
    field: &str,
    visible: &str,
    slot: &serde_json::Value,
) -> String {
    let mut body: serde_json::Value = serde_json::from_str(fixture).unwrap();
    body["data"][root]["nodes"][0][field] = serde_json::json!(format!(
        "{visible}\n\n<!-- onetaskgraph.metadata\n{slot}\n-->"
    ));
    body.to_string()
}

/// The two delivery lists come out of an issue's metadata slot into their typed fields, and
/// out of the caller's metadata with them; a project or a document never shows either key as
/// metadata. And what a read answers can be written back without either key reappearing —
/// while a write still carrying a list is refused rather than dropped.
#[tokio::test]
async fn delivery_lists_are_read_out_of_the_slot_and_never_left_in_free_metadata() {
    use onetaskgraph_plugin_api::TaskRef;
    let slot = serde_json::json!({
        "caller.number": 7,
        (TaskRef::DELIVERS_KEY): ["i2", "elsewhere:T-9"],
        (TaskRef::DELIVERED_BY_KEY): ["plan:T-1"],
    });
    let page = PageRequest {
        cursor: None,
        limit: 1,
    };
    let (endpoint, _) = server(
        "200 OK",
        "",
        with_slot(
            include_str!("fixtures/issues.json"),
            "issues",
            "description",
            "Recorded body",
            &slot,
        ),
    );
    // Read through the mapping it is written back through, so the status it carries is one
    // that source has a name for.
    let task = writable_source(&endpoint)
        .query_tasks(&TaskQuery::default(), &page)
        .await
        .expect("the issue reads")
        .items
        .remove(0);
    assert_eq!(
        task.delivers,
        vec![
            TaskRef::new("i2").unwrap(),
            TaskRef::new("elsewhere:T-9").unwrap()
        ]
    );
    assert_eq!(task.delivered_by, vec![TaskRef::new("plan:T-1").unwrap()]);
    assert_eq!(
        task.metadata,
        [("caller.number".to_owned(), serde_json::json!(7))]
            .into_iter()
            .collect(),
        "the reserved keys are not the caller's metadata"
    );

    // One issue read by id reads the same, since a status write begins with that read.
    let mut node: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/issues.json")).unwrap();
    node = node["data"]["issues"]["nodes"][0].clone();
    node["description"] = serde_json::json!(format!(
        "Recorded body\n\n<!-- onetaskgraph.metadata\n{slot}\n-->"
    ));
    let (endpoint, _) = response_server(vec![serde_json::json!({ "issue": node })]);
    let by_id = source(&endpoint)
        .get_task(&"i1".into())
        .await
        .unwrap()
        .expect("the issue is held");
    assert_eq!(
        (by_id.delivers, by_id.delivered_by, by_id.metadata),
        (
            task.delivers.clone(),
            task.delivered_by.clone(),
            task.metadata.clone()
        )
    );

    let (endpoint, _) = server(
        "200 OK",
        "",
        with_slot(
            include_str!("fixtures/projects.json"),
            "projects",
            "description",
            "Project body",
            &slot,
        ),
    );
    let project = source(&endpoint)
        .query_projects(&ProjectQuery::default(), &page)
        .await
        .unwrap()
        .items
        .remove(0);
    assert_eq!(
        project.metadata.keys().collect::<Vec<_>>(),
        ["caller.number"],
        "a project never shows a delivery key as metadata"
    );
    let (endpoint, _) = server(
        "200 OK",
        "",
        with_slot(
            include_str!("fixtures/documents.json"),
            "documents",
            "content",
            "Recorded body",
            &slot,
        ),
    );
    let document = source(&endpoint)
        .query_documents(&DocumentQuery::default(), &page)
        .await
        .unwrap()
        .items
        .remove(0);
    assert_eq!(
        document.metadata.keys().collect::<Vec<_>>(),
        ["caller.number"],
        "a document never shows a delivery key as metadata"
    );

    // Written back as it was read, the lists land in the slot under their reserved keys,
    // beside the caller's metadata, rather than being dropped.
    let (endpoint, wire) = response_server(vec![
        writable_resolution(),
        serde_json::json!({"issueUpdate":{"success":true,"issue":{"id":"i1",
            "relations":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}}),
    ]);
    writable_source(&endpoint)
        .write_task(&ItemWrite {
            target: Some("i1".into()),
            item: Task {
                labels: Vec::new(),
                ..task.clone()
            },
            depends_on: Vec::new(),
        })
        .await
        .expect("a task carrying its delivery lists writes");
    let update = wire
        .iter()
        .map(|request| sent(&request))
        .find(|request| request["query"] == onetaskgraph_linear::graphql::ISSUE_REWRITE)
        .expect("the issue was updated");
    assert_eq!(
        update["variables"]["input"]["description"],
        serde_json::json!(
            "Recorded body\n\n<!-- onetaskgraph.metadata `{\"caller.number\":7,\"onetaskgraph.delivered_by\":[\"plan:T-1\"],\"onetaskgraph.delivers\":[\"i2\",\"elsewhere:T-9\"]}` -->"
        )
    );

    // Written back without them, the caller's metadata goes back exactly and neither key does.
    let (endpoint, wire) = response_server(vec![
        writable_resolution(),
        serde_json::json!({"issueCreate":{"success":true,"issue":{"id":"I-NEW"}}}),
    ]);
    writable_source(&endpoint)
        .write_task(&ItemWrite {
            target: None,
            item: Task {
                delivers: Vec::new(),
                delivered_by: Vec::new(),
                labels: Vec::new(),
                ..task
            },
            depends_on: Vec::new(),
        })
        .await
        .expect("a task without delivery lists writes");
    let create = wire
        .iter()
        .map(|request| sent(&request))
        .find(|request| request["query"] == onetaskgraph_linear::graphql::ISSUE_CREATE)
        .expect("the issue was created");
    assert_eq!(
        create["variables"]["input"]["description"],
        serde_json::json!(
            "Recorded body\n\n<!-- onetaskgraph.metadata `{\"caller.number\":7}` -->"
        )
    );
}

/// A delivery list Linear hands back that this contract cannot hold is a malformed response
/// naming the task and the entry — including a qualified entry naming the issue itself, which
/// only the source's own name can recognise.
#[tokio::test]
async fn a_delivery_entry_that_is_no_task_the_task_itself_or_a_repeat_is_refused_on_read() {
    use onetaskgraph_plugin_api::TaskRef;
    for (key, held, expected) in [
        (TaskRef::DELIVERS_KEY, serde_json::json!("i2"), "not a list"),
        (
            TaskRef::DELIVERS_KEY,
            serde_json::json!([7]),
            "not a task id",
        ),
        (
            TaskRef::DELIVERS_KEY,
            serde_json::json!([""]),
            "not a task id",
        ),
        (
            TaskRef::DELIVERS_KEY,
            serde_json::json!(["i1"]),
            "that task itself",
        ),
        (
            TaskRef::DELIVERS_KEY,
            serde_json::json!(["work:i1"]),
            "that task itself",
        ),
        (
            TaskRef::DELIVERS_KEY,
            serde_json::json!(["i2", "work:i2"]),
            "more than once",
        ),
        (
            TaskRef::DELIVERED_BY_KEY,
            serde_json::json!(["plan:T-1", "plan:T-1"]),
            "more than once",
        ),
    ] {
        let (endpoint, _) = server(
            "200 OK",
            "",
            with_slot(
                include_str!("fixtures/issues.json"),
                "issues",
                "description",
                "Recorded body",
                &serde_json::json!({ (key): held }),
            ),
        );
        let failure = source(&endpoint)
            .query_tasks(
                &TaskQuery::default(),
                &PageRequest {
                    cursor: None,
                    limit: 1,
                },
            )
            .await
            .expect_err(expected);
        assert!(
            matches!(&failure, SourceError::Malformed { message }
                if message.contains(key) && message.contains("task i1") && message.contains(expected)),
            "{key} holding {held}: {failure:?}"
        );
    }
}

/// A delivery list a task cannot hold — one naming the task itself, or naming one task twice —
/// is refused by name before anything is sent, on a write and on the store's own
/// `set_delivered_by` alike; and a project or a document carrying either reserved key is
/// refused, because only a task delivers or is delivered.
#[tokio::test]
async fn a_delivery_list_a_task_cannot_hold_is_refused_by_name_before_any_request() {
    use onetaskgraph_plugin_api::TaskRef;
    let task = |extra: serde_json::Value| {
        let mut task = serde_json::json!({"id":"T","title":"task","content":null,
            "status":{"category":"todo","name":"Todo"},"labels":[],"project":null,
            "repositories":[],"metadata":{}});
        for (key, value) in extra.as_object().unwrap() {
            task[key] = value.clone();
        }
        serde_json::from_value::<Task>(task).unwrap()
    };
    for (item, named) in [
        (task(serde_json::json!({"delivers":["work:T"]})), "delivers"),
        (
            task(serde_json::json!({"delivered_by":["plan:T-1","plan:T-1"]})),
            "delivered_by",
        ),
    ] {
        let (endpoint, wire) = response_server(Vec::new());
        let refusal = writable_source(&endpoint)
            .write_task(&ItemWrite {
                target: None,
                item,
                depends_on: Vec::new(),
            })
            .await
            .expect_err(named);
        assert!(
            matches!(&refusal, SourceError::Refused { message } if message.contains(named)),
            "{refusal:?}"
        );
        assert!(wire.try_iter().next().is_none(), "nothing was sent");
    }

    let metadata: std::collections::BTreeMap<String, serde_json::Value> =
        [(TaskRef::DELIVERS_KEY.to_owned(), serde_json::json!(["i2"]))]
            .into_iter()
            .collect();
    let project: Project = serde_json::from_value(serde_json::json!({"id":"authored:P",
        "title":"project","content":null,"status":{"category":"todo","name":"Todo"},
        "labels":[],"repositories":[],"metadata":metadata}))
    .unwrap();
    let document: Document = serde_json::from_value(serde_json::json!({"id":"authored:D",
        "title":"document","content":null,"project":null,"labels":[],"repositories":[],
        "metadata":metadata}))
    .unwrap();
    let (endpoint, wire) = response_server(Vec::new());
    let source = writable_source(&endpoint);
    let refusals = [
        source
            .write_project(&ItemWrite {
                target: None,
                item: project,
                depends_on: Vec::new(),
            })
            .await
            .expect_err("a project"),
        source
            .write_document(&ItemWrite {
                target: None,
                item: document,
                depends_on: Vec::new(),
            })
            .await
            .expect_err("a document"),
        source
            .set_delivered_by(&"i1".into(), &[TaskRef::new("work:i1").unwrap()])
            .await
            .expect_err("the task itself"),
    ];
    for (refusal, expected) in refusals.iter().zip([
        "cannot carry onetaskgraph.delivers on a project: only a task delivers",
        "cannot carry onetaskgraph.delivers on a document: only a task delivers",
        "delivered_by",
    ]) {
        assert!(
            matches!(refusal, SourceError::Refused { message } if message.contains(expected)),
            "{refusal:?}"
        );
    }
    assert!(wire.try_iter().next().is_none(), "nothing was sent");
}

/// The store's `set_delivered_by` is one read of the issue and one `issueUpdate` carrying the
/// description alone, which differs from what Linear holds only inside the metadata slot —
/// and an issue already holding the list is the read alone.
#[tokio::test]
async fn delivered_by_is_written_into_the_slot_and_moves_nothing_else() {
    use onetaskgraph_plugin_api::TaskRef;
    let held = "Prose a person wrote,\nwith its own spacing.  \n\n<!-- onetaskgraph.metadata\n{\"caller.kept\":[1]}\n\\-->";
    let (endpoint, wire) = response_server(vec![
        prioritised_issue("i1", Some(held), serde_json::json!(2)),
        serde_json::json!({"issueUpdate":{"success":true,"issue":{"id":"i1"}}}),
    ]);
    writable_source(&endpoint)
        .set_delivered_by(&"i1".into(), &[TaskRef::new("plan:T-1").unwrap()])
        .await
        .unwrap()
        .expect("the issue is held");
    let requests = wire
        .try_iter()
        .map(|request| sent(&request))
        .collect::<Vec<_>>();
    assert_eq!(requests.len(), 2, "{requests:?}");
    assert_eq!(
        requests[1]["variables"],
        serde_json::json!({"id":"i1","input":{"description":
            "Prose a person wrote,\nwith its own spacing.  \n\n<!-- onetaskgraph.metadata `{\"caller.kept\":[1],\"onetaskgraph.delivered_by\":[\"plan:T-1\"]}` -->"}})
    );

    let already =
        "Prose.\n\n<!-- onetaskgraph.metadata\n{\"onetaskgraph.delivered_by\":[\"plan:T-1\"]}\n-->";
    let (endpoint, wire) = response_server(vec![prioritised_issue(
        "i1",
        Some(already),
        serde_json::json!(0),
    )]);
    writable_source(&endpoint)
        .set_delivered_by(&"i1".into(), &[TaskRef::new("plan:T-1").unwrap()])
        .await
        .unwrap()
        .expect("the issue is held");
    assert_eq!(wire.try_iter().count(), 1, "the read alone");

    let (endpoint, wire) = response_server(vec![serde_json::json!({"issue":null})]);
    assert_eq!(
        writable_source(&endpoint)
            .set_delivered_by(&"i9".into(), &[])
            .await
            .unwrap(),
        None
    );
    assert_eq!(wire.try_iter().count(), 1);
}

/// What `issue(id:)` answers for one issue carrying `description` and Linear's raw
/// `priority`, with a label and a project so a write that moved either would show.
fn prioritised_issue(
    id: &str,
    description: Option<&str>,
    priority: serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({"issue":{"id":id,"identifier":identifier(id),"title":"Fixture issue",
        "description":description,"url":"https://linear.app/acme/issue/ENG-1",
        "createdAt":"2026-09-01T12:00:00Z","updatedAt":"2026-09-02T12:00:00Z","archivedAt":null,
        "state":{"name":"In Progress","type":"started"},"priority":priority,
        "labels":{"nodes":[{"id":"l1","name":"Bug","color":"#ff0000"}]},"project":{"id":"p1"}}})
}

/// Linear's scale, both ways: `0` none, `1` urgent, `2` high, `3` normal, `4` low.
const LINEAR_SCALE: [(Priority, u8); 5] = [
    (Priority::None, 0),
    (Priority::Urgent, 1),
    (Priority::High, 2),
    (Priority::Medium, 3),
    (Priority::Low, 4),
];

/// Every level of Linear's scale reads as its own priority, whether Linear spells it as the
/// `Float!` its schema declares or as a whole number, on a read by id and on a listing.
#[tokio::test]
async fn each_of_linears_priorities_reads_as_its_own_level_over_real_http() {
    for (priority, level) in LINEAR_SCALE {
        for raw in [
            serde_json::json!(level),
            serde_json::json!(f64::from(level)),
        ] {
            let (endpoint, _) = response_server(vec![prioritised_issue("I-1", None, raw.clone())]);
            let task = source(&endpoint)
                .get_task(&"I-1".into())
                .await
                .unwrap()
                .expect("the issue is held");
            assert_eq!(task.priority, priority, "a read of {raw}");
        }
    }
    let nodes = LINEAR_SCALE
        .iter()
        .map(|(_, level)| {
            let mut issue = prioritised_issue(&format!("I-{level}"), None, (*level).into());
            issue["issue"].take()
        })
        .collect::<Vec<_>>();
    let (endpoint, wire) = response_server(vec![serde_json::json!({"issues":{"nodes":nodes,
        "pageInfo":{"hasNextPage":false,"endCursor":null}}})]);
    let page = source(&endpoint)
        .query_tasks(
            &TaskQuery::default(),
            &PageRequest {
                cursor: None,
                limit: 10,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        page.items
            .iter()
            .map(|task| task.priority)
            .collect::<Vec<_>>(),
        LINEAR_SCALE
            .iter()
            .map(|(priority, _)| *priority)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        sent(&wire.recv().unwrap())["query"],
        onetaskgraph_linear::graphql::ISSUES
    );
}

/// A priority that is none of the five is a response this source cannot read — never the
/// nearest level, and never no priority.
#[tokio::test]
async fn a_priority_off_linears_scale_is_refused_naming_the_field() {
    for raw in [
        serde_json::json!(5),
        serde_json::json!(-1),
        serde_json::json!(2.5),
        serde_json::json!("2"),
        serde_json::Value::Null,
    ] {
        let (endpoint, _) = response_server(vec![prioritised_issue("I-1", None, raw.clone())]);
        let refusal = source(&endpoint)
            .get_task(&"I-1".into())
            .await
            .expect_err("no priority of Linear's is this");
        assert!(
            matches!(&refusal, SourceError::Malformed { message }
                if message.contains("field priority is") && message.contains("4 (low)")),
            "{raw}: {refusal:?}"
        );
    }
    let mut absent = prioritised_issue("I-1", None, 0.into());
    absent["issue"].as_object_mut().unwrap().remove("priority");
    let (endpoint, _) = response_server(vec![absent]);
    let refusal = source(&endpoint)
        .get_task(&"I-1".into())
        .await
        .expect_err("the field is selected and `Float!`");
    assert!(
        matches!(&refusal, SourceError::Malformed { message } if message.contains("priority")),
        "{refusal:?}"
    );
}

/// A copy carries the priority as Linear's own number, on a create and on an update alike —
/// `0` included, so moving a task back to no priority is written rather than left out.
#[tokio::test]
async fn a_copy_sends_linears_priority_on_create_and_on_update_including_none() {
    let no_relations = serde_json::json!({"issue":{"description":null,
        "relations":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}},
        "inverseRelations":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}});
    for (priority, level) in LINEAR_SCALE {
        for target in [None, Some(NativeId::from("I-OLD"))] {
            let (mutation, root) = if target.is_some() {
                (onetaskgraph_linear::graphql::ISSUE_REWRITE, "issueUpdate")
            } else {
                (onetaskgraph_linear::graphql::ISSUE_CREATE, "issueCreate")
            };
            // An update replaces the relations the issue holds, so its answer reads them back;
            // a create holds none.
            let mut written = serde_json::json!({"id":"I-OLD"});
            if target.is_some() {
                written["relations"] = no_relations["issue"]["relations"].clone();
            }
            let (endpoint, wire) = response_server(vec![
                writable_resolution(),
                serde_json::json!({(root):{"success":true,"issue":written}}),
            ]);
            let task: Task = serde_json::from_value(serde_json::json!({"id":"authored:T",
                "title":"task","content":"body","status":{"category":"todo","name":"Todo"},
                "priority":priority,"labels":[],"repositories":[],"metadata":{}}))
            .unwrap();
            writable_source(&endpoint)
                .write_task(&ItemWrite {
                    target: target.clone(),
                    item: task,
                    depends_on: Vec::new(),
                })
                .await
                .unwrap();
            let written = wire
                .iter()
                .map(|request| sent(&request))
                .find(|request| request["query"] == mutation)
                .expect("the issue was written");
            assert_eq!(
                written["variables"]["input"]["priority"],
                serde_json::json!(level),
                "{priority:?} into {target:?}"
            );
        }
    }
}

/// A priority set on its own reads the issue, sends `issueUpdate` carrying `priority` and
/// nothing else, and answers with the priority Linear's own payload reports — not an echo.
#[tokio::test]
async fn a_priority_is_set_by_sending_only_priority_and_answers_what_linear_read_back() {
    use onetaskgraph_linear::graphql;
    for (priority, level) in LINEAR_SCALE {
        let (endpoint, wire) = response_server(vec![
            prioritised_issue("I-1", Some("body"), 3.into()),
            serde_json::json!({"issueUpdate":{"success":true,
                "issue":{"id":"I-1","priority":f64::from(level)}}}),
        ]);
        let answered = writable_source(&endpoint)
            .set_task_priority(&"ENG-1".into(), priority)
            .await
            .unwrap();
        assert_eq!(answered, Some(priority));
        let shapes = wire
            .iter()
            .map(|request| {
                let request = sent(&request);
                (request["query"].clone(), request["variables"].clone())
            })
            .collect::<Vec<_>>();
        assert_eq!(
            shapes,
            vec![
                (
                    serde_json::json!(graphql::ISSUE),
                    serde_json::json!({"id":"ENG-1"})
                ),
                // The backend id the read answered with, and `priority` alone.
                (
                    serde_json::json!(graphql::ISSUE_PRIORITY_UPDATE),
                    serde_json::json!({"id":"I-1","input":{"priority":level}})
                ),
            ],
            "{priority:?}"
        );
    }

    // What Linear holds is the answer, even where it is not what was sent.
    let (endpoint, _) = response_server(vec![
        prioritised_issue("I-1", None, 0.into()),
        serde_json::json!({"issueUpdate":{"success":true,"issue":{"id":"I-1","priority":4}}}),
    ]);
    assert_eq!(
        writable_source(&endpoint)
            .set_task_priority(&"I-1".into(), Priority::Urgent)
            .await
            .unwrap(),
        Some(Priority::Low)
    );
}

/// No such issue, and an issue Linear has trashed, are no such task for both narrow writes:
/// `None`, after the read alone.
#[tokio::test]
async fn a_narrow_priority_or_content_write_answers_none_for_no_issue_and_writes_nothing() {
    let answers = || {
        [serde_json::json!({"issue":null}), {
            let mut trashed = prioritised_issue("I-1", Some("body"), 2.into());
            trashed["issue"]["archivedAt"] = serde_json::json!("2026-09-04T12:00:00Z");
            trashed
        }]
    };
    for answer in answers() {
        let (endpoint, wire) = response_server(vec![answer]);
        assert_eq!(
            writable_source(&endpoint)
                .set_task_priority(&"I-1".into(), Priority::High)
                .await
                .unwrap(),
            None
        );
        assert_eq!(wire.iter().count(), 1, "the read alone");
    }
    for answer in answers() {
        let (endpoint, wire) = response_server(vec![answer]);
        assert_eq!(
            writable_source(&endpoint)
                .set_task_content(&"I-1".into(), "new body")
                .await
                .unwrap(),
            None
        );
        assert_eq!(wire.iter().count(), 1, "the read alone");
    }
}

/// Linear refusing either narrow write, or answering one with a payload this source cannot
/// read, is a failure — never a priority or a content reported as written.
#[tokio::test]
async fn a_narrow_write_linear_did_not_confirm_is_not_reported_written() {
    for (payload, refused) in [
        (
            serde_json::json!({"issueUpdate":{"success":false,"issue":null}}),
            true,
        ),
        (
            serde_json::json!({"issueUpdate":{"success":true,"issue":null}}),
            false,
        ),
        (
            serde_json::json!({"issueUpdate":{"success":true,"issue":{"id":"I-1"}}}),
            false,
        ),
        (
            serde_json::json!({"issueUpdate":{"success":true,"issue":{"id":"I-1","priority":9}}}),
            false,
        ),
        // An answer naming another issue is not this write landing.
        (
            serde_json::json!({"issueUpdate":{"success":true,"issue":{"id":"I-2","priority":2}}}),
            false,
        ),
    ] {
        let (endpoint, _) = response_server(vec![
            prioritised_issue("I-1", None, 0.into()),
            payload.clone(),
        ]);
        let failure = writable_source(&endpoint)
            .set_task_priority(&"I-1".into(), Priority::High)
            .await
            .expect_err("not confirmed");
        assert!(
            matches!(
                (&failure, refused),
                (SourceError::Refused { .. }, true) | (SourceError::Malformed { .. }, false)
            ),
            "{payload}: {failure:?}"
        );
    }
    for (payload, refused) in [
        (
            serde_json::json!({"issueUpdate":{"success":false,"issue":null}}),
            true,
        ),
        (
            serde_json::json!({"issueUpdate":{"success":true,"issue":null}}),
            false,
        ),
        (
            serde_json::json!({"issueUpdate":{"success":true,"issue":{"id":"I-2"}}}),
            false,
        ),
    ] {
        let (endpoint, _) = response_server(vec![
            prioritised_issue("I-1", None, 0.into()),
            payload.clone(),
        ]);
        let failure = writable_source(&endpoint)
            .set_task_content(&"I-1".into(), "body")
            .await
            .expect_err("not confirmed");
        assert!(
            matches!(
                (&failure, refused),
                (SourceError::Refused { .. }, true) | (SourceError::Malformed { .. }, false)
            ),
            "{payload}: {failure:?}"
        );
    }
    // An issue this source could not read is refused before anything is written over it.
    let (endpoint, wire) = response_server(vec![prioritised_issue("I-1", None, 7.into())]);
    assert!(matches!(
        writable_source(&endpoint)
            .set_task_content(&"I-1".into(), "body")
            .await,
        Err(SourceError::Malformed { .. })
    ));
    assert_eq!(wire.iter().count(), 1, "no issueUpdate was sent");
}

/// A content write sends `issueUpdate` with `description` alone, and that description keeps
/// the issue's metadata slot byte for byte — so a later read reports the given content and
/// every other field, the metadata in the slot included, exactly as it was.
#[tokio::test]
async fn a_content_write_sends_only_the_description_and_moves_nothing_else() {
    use onetaskgraph_linear::graphql;
    let slot = "<!-- onetaskgraph.metadata\n{\"caller.number\":7,\"onetaskgraph.repositories\":[\"github.com/acme/work\"]}\n-->";
    for (before, content, sent_description, read_content) in [
        // A slot is kept as stored, after the separator a copy writes.
        (
            Some(format!("old body\n\n{slot}")),
            "new body\nsecond line",
            format!("new body\nsecond line\n\n{slot}"),
            Some("new body\nsecond line"),
        ),
        // Whitespace at the content's end is content: it goes before the separator, and a
        // read takes off the separator alone.
        (
            Some(format!("old body\n\n{slot}")),
            "new body  \n\n\n",
            format!("new body  \n\n\n\n\n{slot}"),
            Some("new body  \n\n\n"),
        ),
        // Content emptied beside a slot: the slot alone.
        (
            Some(format!("old body\n\n{slot}")),
            "",
            slot.to_owned(),
            None,
        ),
        // No slot: the content verbatim, its trailing newline included.
        (
            Some("old body".to_owned()),
            "new body\n",
            "new body\n".to_owned(),
            Some("new body\n"),
        ),
        (None, "new body", "new body".to_owned(), Some("new body")),
    ] {
        let (endpoint, wire) = response_server(vec![
            prioritised_issue("I-1", before.as_deref(), 2.into()),
            serde_json::json!({"issueUpdate":{"success":true,"issue":{"id":"I-1"}}}),
        ]);
        assert_eq!(
            writable_source(&endpoint)
                .set_task_content(&"ENG-1".into(), content)
                .await
                .unwrap(),
            Some(())
        );
        let requests = wire
            .iter()
            .map(|request| sent(&request))
            .collect::<Vec<_>>();
        assert_eq!(requests.len(), 2, "one read and one write: {requests:?}");
        assert_eq!(requests[0]["query"], graphql::ISSUE);
        assert_eq!(requests[1]["query"], graphql::ISSUE_UPDATE);
        // The backend id the read answered with, and `description` as the input's only member.
        assert_eq!(
            requests[1]["variables"],
            serde_json::json!({"id":"I-1","input":{"description":sent_description}})
        );

        // Read back what was written, beside what was there before.
        let (endpoint, _) = response_server(vec![
            prioritised_issue("I-1", before.as_deref(), 2.into()),
            prioritised_issue("I-1", Some(&sent_description), 2.into()),
        ]);
        let reader = source(&endpoint);
        let was = reader.get_task(&"I-1".into()).await.unwrap().unwrap();
        let now = reader.get_task(&"I-1".into()).await.unwrap().unwrap();
        assert_eq!(now.content.as_deref(), read_content, "{content:?}");
        assert_eq!(
            Task {
                content: was.content.clone(),
                ..now
            },
            was,
            "only the content moved"
        );
    }
}

/// Content ending in what this source reads as its own metadata slot is refused before
/// anything is sent — written, it would read back as metadata rather than as the content it
/// was — and one naming such a block anywhere but the end, beside the issue's real slot, reads
/// back as the content it is.
#[tokio::test]
async fn content_that_would_read_back_as_metadata_is_refused_before_it_is_sent() {
    let lookalike = "Notes.\n\n<!-- onetaskgraph.metadata\n{\"caller.number\":1}\n-->";
    let (endpoint, wire) = response_server(vec![prioritised_issue("I-1", Some("Old."), 0.into())]);
    let refused = writable_source(&endpoint)
        .set_task_content(&"I-1".into(), lookalike)
        .await
        .expect_err("refused");
    assert!(
        matches!(&refused, SourceError::Refused { message }
            if message.contains("reads as its own metadata slot") && message.contains("next:")),
        "{refused:?}"
    );
    assert_eq!(wire.iter().count(), 1, "no issueUpdate was sent");

    let slot = "<!-- onetaskgraph.metadata\n{\"caller.number\":7}\n-->";
    let (endpoint, wire) = response_server(vec![
        prioritised_issue("I-1", Some(&format!("Old.\n\n{slot}")), 0.into()),
        serde_json::json!({"issueUpdate":{"success":true,"issue":{"id":"I-1"}}}),
    ]);
    writable_source(&endpoint)
        .set_task_content(&"I-1".into(), lookalike)
        .await
        .expect("the issue's own slot stays the slot");
    let requests: Vec<serde_json::Value> = wire.iter().map(|request| sent(&request)).collect();
    assert_eq!(
        requests[1]["variables"]["input"]["description"],
        format!("{lookalike}\n\n{slot}")
    );
}

/// A source over `endpoint` on the team `ENG`, configured with `extra` beside it.
fn configured_source(endpoint: &str, extra: serde_json::Value) -> Box<dyn TaskSource> {
    let mut config =
        serde_json::json!({"endpoint":endpoint,"team":"ENG","status_mapping":writable_mapping()});
    for (key, value) in extra.as_object().expect("an object") {
        config[key] = value.clone();
    }
    onetaskgraph_linear::Plugin
        .build(
            &SourceName::new("work").unwrap(),
            &config,
            &Secrets(Some("fixture-key".into())),
        )
        .unwrap_or_else(|error| panic!("{error}"))
}

/// `project` narrows every read to that project and files every write there: an issue of
/// another project is no task of this source, a task written with no project is placed in
/// the scope, and one naming another is refused naming both.
#[tokio::test]
async fn a_project_scoped_source_reads_and_writes_that_project_alone() {
    let scope = serde_json::json!({"project":"P-SCOPE"});
    let (endpoint, wire) = response_server(vec![serde_json::json!({"issues":{"nodes":[],
        "pageInfo":{"hasNextPage":false,"endCursor":null}}})]);
    configured_source(&endpoint, scope.clone())
        .query_tasks(
            &TaskQuery::default(),
            &PageRequest {
                cursor: None,
                limit: 10,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        sent(&wire.recv().unwrap())["variables"]["filter"],
        serde_json::json!({"and":[{"team":{"key":{"eqIgnoreCase":"ENG"}}},
            {"project":{"id":{"eq":"P-SCOPE"}}}]})
    );

    let (endpoint, wire) = response_server(vec![serde_json::json!({"projects":{"nodes":[],
        "pageInfo":{"hasNextPage":false,"endCursor":null}}})]);
    configured_source(&endpoint, scope.clone())
        .query_projects(
            &ProjectQuery::default(),
            &PageRequest {
                cursor: None,
                limit: 10,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        sent(&wire.recv().unwrap())["variables"]["filter"],
        serde_json::json!({"and":[{"accessibleTeams":{"some":{"key":{"eqIgnoreCase":"ENG"}}}},
            {"id":{"eq":"P-SCOPE"}}]})
    );

    // An issue filed elsewhere is no task of this source.
    let (endpoint, _) = response_server(vec![prioritised_issue("i1", None, serde_json::json!(0))]);
    assert_eq!(
        configured_source(&endpoint, scope.clone())
            .get_task(&"i1".into())
            .await
            .unwrap(),
        None
    );

    let task: Task = serde_json::from_value(serde_json::json!({"id":"T","title":"task",
        "content":null,"status":{"category":"todo","name":"Todo"},"labels":[],"project":null,
        "repositories":[],"metadata":{}}))
    .unwrap();
    let (endpoint, wire) = response_server(vec![
        writable_resolution(),
        serde_json::json!({"issueCreate":{"success":true,"issue":{"id":"I-NEW"}}}),
    ]);
    configured_source(&endpoint, scope.clone())
        .write_task(&ItemWrite {
            target: None,
            item: task.clone(),
            depends_on: Vec::new(),
        })
        .await
        .unwrap();
    let create = wire
        .try_iter()
        .map(|request| sent(&request))
        .find(|request| request["query"] == onetaskgraph_linear::graphql::ISSUE_CREATE)
        .expect("the issue was created");
    assert_eq!(create["variables"]["input"]["projectId"], "P-SCOPE");

    let (endpoint, wire) = response_server(Vec::new());
    let refused = configured_source(&endpoint, scope)
        .write_task(&ItemWrite {
            target: None,
            item: Task {
                project: Some("P-OTHER".into()),
                ..task
            },
            depends_on: Vec::new(),
        })
        .await
        .expect_err("another project");
    let said = refused.to_string();
    assert!(
        said.contains("P-SCOPE") && said.contains("P-OTHER"),
        "{said}"
    );
    assert!(wire.try_iter().next().is_none(), "nothing was sent");
}

/// The follow-up searches go to Linear as narrowings of `issues(filter:)` — and each row is
/// confirmed by the contract's own rule before it is returned, so a decoy whose prose
/// carries the searched phrase and one whose slot holds another value are both kept out.
#[tokio::test]
async fn the_follow_up_searches_are_narrowed_by_linear_and_confirmed_in_process() {
    let issue = |id: &str, title: &str, description: &str, priority: u8| {
        serde_json::json!({"id":id,"identifier":identifier(id),"title":title,
            "description":description,"url":null,"createdAt":null,"updatedAt":null,
            "project":null,"state":{"name":"Todo","type":"unstarted"},"priority":priority,
            "labels":{"nodes":[]}})
    };
    let slot = |json: &str| format!("Prose.\n\n<!-- onetaskgraph.metadata\n{json}\n-->");
    let nodes = vec![
        issue(
            "match",
            "Alpha follow-up",
            &slot(r#"{"caller.key":"v","onetaskgraph.origin":"elsewhere:O-1"}"#),
            2,
        ),
        // The phrase in prose, outside the slot.
        issue(
            "prose",
            "Alpha follow-up",
            "Mentions \"caller.key\":\"v\" and \"onetaskgraph.origin\":\"elsewhere:O-1\".",
            2,
        ),
        // The slot holding another value.
        issue(
            "other",
            "Alpha follow-up",
            &slot(r#"{"caller.key":"w","onetaskgraph.origin":"elsewhere:O-2"}"#),
            2,
        ),
    ];
    let (endpoint, wire) = response_server(vec![serde_json::json!({"issues":{"nodes":nodes,
        "pageInfo":{"hasNextPage":false,"endCursor":null}}})]);
    let query = TaskQuery {
        priorities: vec![Priority::High],
        commented_since: chrono::DateTime::parse_from_rfc3339("2026-10-01T00:00:00Z")
            .ok()
            .map(|at| at.with_timezone(&chrono::Utc)),
        metadata: vec![
            onetaskgraph_plugin_api::MetadataMatch::new("caller.key", Vec::new(), "v").unwrap(),
        ],
        origin: Some("elsewhere:O-1".into()),
        text: Some(onetaskgraph_plugin_api::TextQuery {
            terms: "ALPHA".into(),
            fields: onetaskgraph_plugin_api::TextFields::Title,
        }),
        ..TaskQuery::default()
    };
    let page = configured_source(&endpoint, serde_json::json!({}))
        .query_tasks(
            &query,
            &PageRequest {
                cursor: None,
                limit: 10,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        page.items
            .iter()
            .map(|task| task.id.0.as_str())
            .collect::<Vec<_>>(),
        ["match"]
    );
    assert_eq!(
        sent(&wire.recv().unwrap())["variables"]["filter"],
        serde_json::json!({"and":[
            {"team":{"key":{"eqIgnoreCase":"ENG"}}},
            {"priority":{"in":[2]}},
            {"comments":{"some":{"or":[{"createdAt":{"gte":"2026-10-01T00:00:00Z"}},
                                       {"updatedAt":{"gte":"2026-10-01T00:00:00Z"}}]}}},
            {"description":{"contains":"\"v\""}},
            {"description":{"contains":"\"elsewhere:O-1\""}},
            {"title":{"containsIgnoreCase":"ALPHA"}}]})
    );
}

/// A metadata value and an origin holding a character an encoder may escape are not sent to
/// Linear, so every issue of the team is a candidate; the confirmation over the parsed slot
/// still returns exactly the issue whose slot holds both — spelled escaped, as an encoder that
/// escapes `/` writes it — and keeps out a decoy carrying them as prose and one whose slot holds
/// other values.
#[tokio::test]
async fn an_escapable_metadata_value_and_origin_are_decided_by_the_confirmation_alone() {
    let issue = |id: &str, description: &str| {
        serde_json::json!({"id":id,"identifier":identifier(id),"title":"Follow-up",
            "description":description,"url":null,"createdAt":null,"updatedAt":null,
            "project":null,"state":{"name":"Todo","type":"unstarted"},"priority":0,
            "labels":{"nodes":[]}})
    };
    let slot = |json: &str| format!("Prose.\n\n<!-- onetaskgraph.metadata\n{json}\n-->");
    let nodes = vec![
        issue(
            "match",
            &slot(r#"{"caller.site":"github.com\/a\/b","onetaskgraph.origin":"work:notes\/a.md"}"#),
        ),
        issue(
            "prose",
            "Mentions \"caller.site\":\"github.com/a/b\" and \"onetaskgraph.origin\":\"work:notes/a.md\".",
        ),
        issue(
            "other",
            &slot(r#"{"caller.site":"github.com/a/c","onetaskgraph.origin":"work:notes/b.md"}"#),
        ),
    ];
    let (endpoint, wire) = response_server(vec![serde_json::json!({"issues":{"nodes":nodes,
        "pageInfo":{"hasNextPage":false,"endCursor":null}}})]);
    let page = configured_source(&endpoint, serde_json::json!({}))
        .query_tasks(
            &TaskQuery {
                metadata: vec![
                    onetaskgraph_plugin_api::MetadataMatch::new(
                        "caller.site",
                        Vec::new(),
                        "github.com/a/b",
                    )
                    .unwrap(),
                ],
                origin: Some("work:notes/a.md".into()),
                ..TaskQuery::default()
            },
            &PageRequest {
                cursor: None,
                limit: 10,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        page.items
            .iter()
            .map(|task| task.id.0.as_str())
            .collect::<Vec<_>>(),
        ["match"]
    );
    assert_eq!(
        sent(&wire.recv().unwrap())["variables"]["filter"],
        serde_json::json!({"team":{"key":{"eqIgnoreCase":"ENG"}}})
    );
}

/// The scan for a copy of a cross-source far end asks for the issues of this source's team —
/// and of its project, when it is scoped — whose description carries that far end as its
/// origin, rather than walking the whole workspace; and an issue answering the scan whose
/// slot does not hold that origin is not taken for it.
#[tokio::test]
async fn the_cross_source_edge_scan_is_narrowed_to_this_source_and_the_far_ends_origin() {
    let task: Task = serde_json::from_value(serde_json::json!({"id":"T","title":"task",
        "content":null,"status":{"category":"todo","name":"Todo"},"labels":[],"project":null,
        "repositories":[],"metadata":{}}))
    .unwrap();
    let edge = DependencyEdge {
        from: DependencyEndpoint::new("T".into(), ItemKind::Task).unwrap(),
        to: DependencyEndpoint::new("plan:F-1".into(), ItemKind::Task).unwrap(),
        kind: DependencyKind::Blocks,
    };
    for (extra, filter) in [
        (
            serde_json::json!({}),
            serde_json::json!({"and":[{"team":{"key":{"eqIgnoreCase":"ENG"}}},
                {"description":{"contains":"\"plan:F-1\""}}]}),
        ),
        (
            serde_json::json!({"project":"P-SCOPE"}),
            serde_json::json!({"and":[{"team":{"key":{"eqIgnoreCase":"ENG"}}},
                {"project":{"id":{"eq":"P-SCOPE"}}},
                {"description":{"contains":"\"plan:F-1\""}}]}),
        ),
    ] {
        // The one issue the scan answers holds the phrase in prose, not as its origin.
        let decoy = serde_json::json!({"id":"decoy","identifier":"ENG-9","title":"decoy",
            "description":"\"onetaskgraph.origin\":\"plan:F-1\"","url":null,"createdAt":null,
            "updatedAt":null,"project":{"id":"P-SCOPE"},"state":{"name":"Todo","type":"unstarted"},
            "priority":0,"labels":{"nodes":[]}});
        // The status first, so a write the mapping cannot name is refused before the scan.
        let (endpoint, wire) = response_server(vec![
            writable_resolution(),
            serde_json::json!({"issues":{"nodes":[decoy],
                "pageInfo":{"hasNextPage":false,"endCursor":null}}}),
            serde_json::json!({"issueCreate":{"success":true,"issue":{"id":"I-NEW"}}}),
        ]);
        configured_source(&endpoint, extra)
            .write_task(&ItemWrite {
                target: None,
                item: task.clone(),
                depends_on: vec![edge.clone()],
            })
            .await
            .unwrap();
        let requests = wire
            .try_iter()
            .map(|request| sent(&request))
            .collect::<Vec<_>>();
        assert_eq!(requests[1]["query"], onetaskgraph_linear::graphql::ISSUES);
        assert_eq!(requests[1]["variables"]["filter"], filter);
        let description = requests[2]["variables"]["input"]["description"]
            .as_str()
            .expect("the far end is recorded in the slot")
            .to_owned();
        assert!(
            description.contains("\"onetaskgraph.depends_on\":[{\"id\":\"plan:F-1\""),
            "the decoy was not taken for the far end: {description}"
        );
        assert!(
            requests
                .iter()
                .all(|request| request["query"]
                    != onetaskgraph_linear::graphql::ISSUE_RELATION_CREATE),
            "no native relation to the decoy"
        );
    }
}

/// The slot is written on one line, its JSON inside a code span, with `<`, `>` and backticks
/// escaped — the spelling Linear keeps byte for byte — and a value holding what would close
/// the span or the comment reads back exactly.
#[tokio::test]
async fn the_slot_is_written_in_the_one_spelling_linear_keeps_byte_for_byte() {
    let key = MetadataKey::new("caller.live").unwrap();
    let hostile = serde_json::json!(["a --> b <!-- c", "`tick`", "github.com/a/b", "_y_ [z]"]);
    let (endpoint, wire) = response_server(vec![
        prioritised_issue("i1", Some("Prose."), serde_json::json!(0)),
        serde_json::json!({"issueUpdate":{"success":true,"issue":{"id":"i1"}}}),
        prioritised_issue("i1", Some("Prose."), serde_json::json!(0)),
    ]);
    writable_source(&endpoint)
        .set_task_metadata(&"i1".into(), &key, &hostile)
        .await
        .unwrap();
    let requests = wire
        .try_iter()
        .map(|request| sent(&request))
        .collect::<Vec<_>>();
    let written = requests[1]["variables"]["input"]["description"]
        .as_str()
        .expect("a description")
        .to_owned();
    assert_eq!(
        written,
        "Prose.\n\n<!-- onetaskgraph.metadata `{\"caller.live\":[\"a --\\u003e b \\u003c!-- c\",\"\\u0060tick\\u0060\",\"github.com/a/b\",\"_y_ [z]\"]}` -->"
    );
    // What was written reads back as the value it was.
    let (endpoint, _) = response_server(vec![prioritised_issue(
        "i1",
        Some(&written),
        serde_json::json!(0),
    )]);
    let task = source(&endpoint)
        .get_task(&"i1".into())
        .await
        .unwrap()
        .expect("held");
    assert_eq!(task.metadata.get("caller.live"), Some(&hostile));
    assert_eq!(task.content.as_deref(), Some("Prose."));
}

/// An issue written before the slot was a code span still parses: the multi-line spelling, in
/// the close this source wrote and in the escaped close Linear hands it back with, reads as the
/// metadata, the delivery lists and the content it held — and the next narrow write of that
/// issue writes the code span, every byte above the slot as it was.
#[tokio::test]
async fn a_slot_in_the_multi_line_spelling_written_before_still_parses() {
    for old in [
        "Prose.\n\n<!-- onetaskgraph.metadata\n{\"caller.kept\":1,\"onetaskgraph.delivered_by\":[\"plan:T-1\"]}\n-->",
        "Prose.\n\n<!-- onetaskgraph.metadata\n{\"caller.kept\":1,\"onetaskgraph.delivered_by\":[\"plan:T-1\"]}\n\\-->",
    ] {
        let (endpoint, _) = response_server(vec![prioritised_issue(
            "i1",
            Some(old),
            serde_json::json!(0),
        )]);
        let task = source(&endpoint)
            .get_task(&"i1".into())
            .await
            .unwrap()
            .expect("held");
        assert_eq!(
            task.metadata.get("caller.kept"),
            Some(&serde_json::json!(1)),
            "{old:?}"
        );
        assert_eq!(
            task.delivered_by,
            vec![TaskRef::new("plan:T-1").unwrap()],
            "{old:?}"
        );
        assert_eq!(task.content.as_deref(), Some("Prose."), "{old:?}");

        let (endpoint, wire) = response_server(vec![
            prioritised_issue("i1", Some(old), serde_json::json!(0)),
            serde_json::json!({"issueUpdate":{"success":true,"issue":{"id":"i1"}}}),
            prioritised_issue("i1", Some(old), serde_json::json!(0)),
        ]);
        writable_source(&endpoint)
            .set_task_metadata(
                &"i1".into(),
                &MetadataKey::new("caller.added").unwrap(),
                &serde_json::json!(true),
            )
            .await
            .unwrap();
        let requests = wire
            .try_iter()
            .map(|request| sent(&request))
            .collect::<Vec<_>>();
        assert_eq!(
            requests[1]["variables"]["input"]["description"],
            "Prose.\n\n<!-- onetaskgraph.metadata `{\"caller.added\":true,\"caller.kept\":1,\"onetaskgraph.delivered_by\":[\"plan:T-1\"]}` -->",
            "{old:?}"
        );
    }
}

/// A narrow write Linear acknowledges for another item, or after which the record cannot be
/// read back, is not reported written: each is the malformed answer it is, naming what went
/// wrong.
#[tokio::test]
async fn a_narrow_slot_write_that_lands_elsewhere_or_vanishes_is_not_reported_written() {
    let key = MetadataKey::new("myapp.review").unwrap();
    let value = serde_json::json!(true);
    let (endpoint, _) = response_server(vec![
        held_project("p1", "Prose."),
        serde_json::json!({"projectUpdate":{"success":true,"project":{"id":"p-other"}}}),
    ]);
    let refused = writable_source(&endpoint)
        .set_project_metadata(&"p1".into(), &key, &value)
        .await
        .expect_err("another project acknowledged");
    assert!(
        matches!(&refused, SourceError::Malformed { message } if message.contains("projectUpdate for p1 answered with the item p-other")),
        "{refused:?}"
    );
    let (endpoint, _) = response_server(vec![
        held_document("d1", "Prose."),
        serde_json::json!({"documentUpdate":{"success":true,"document":{"id":"d-other"}}}),
    ]);
    let refused = writable_source(&endpoint)
        .set_document_metadata(&"d1".into(), &key, &value)
        .await
        .expect_err("another document acknowledged");
    assert!(
        matches!(&refused, SourceError::Malformed { message } if message.contains("documentUpdate for d1 answered with the item d-other")),
        "{refused:?}"
    );

    // Written, and then gone: the record a metadata write answers with is a read, and a read
    // that finds nothing is not the record the write left.
    for (record, held, acknowledged, gone) in [
        (
            "task",
            prioritised_issue("i1", Some("Prose."), serde_json::json!(0)),
            serde_json::json!({"issueUpdate":{"success":true,"issue":{"id":"i1"}}}),
            serde_json::json!({"issue":null}),
        ),
        (
            "project",
            held_project("p1", "Prose."),
            serde_json::json!({"projectUpdate":{"success":true,"project":{"id":"p1"}}}),
            serde_json::json!({"project":null}),
        ),
        (
            "document",
            held_document("d1", "Prose."),
            serde_json::json!({"documentUpdate":{"success":true,"document":{"id":"d1"}}}),
            serde_json::json!({"document":null}),
        ),
    ] {
        let (endpoint, _) = response_server(vec![held, acknowledged, gone]);
        let source = writable_source(&endpoint);
        let refused = match record {
            "task" => source
                .set_task_metadata(&"i1".into(), &key, &value)
                .await
                .map(|_| ()),
            "project" => source
                .set_project_metadata(&"p1".into(), &key, &value)
                .await
                .map(|_| ()),
            _ => source
                .set_document_metadata(&"d1".into(), &key, &value)
                .await
                .map(|_| ()),
        }
        .expect_err("the record vanished after its write");
        assert!(
            matches!(&refused, SourceError::Malformed { message }
                if message.contains(&format!("{record} ")) && message.contains("could not be read back")),
            "{record}: {refused:?}"
        );
    }
}

/// A code-span slot that never closes, or holds what is not JSON, is a malformed field naming
/// the slot rather than metadata read past.
#[tokio::test]
async fn a_malformed_code_span_slot_is_refused_rather_than_read_past() {
    for (description, said) in [
        (
            "Prose.\n\n<!-- onetaskgraph.metadata `{\"caller.kept\":1}",
            "unterminated onetaskgraph metadata slot",
        ),
        (
            "Prose.\n\n<!-- onetaskgraph.metadata `{\"caller.kept\":` -->",
            "invalid canonical JSON in Linear onetaskgraph metadata slot",
        ),
    ] {
        let (endpoint, _) = response_server(vec![prioritised_issue(
            "i1",
            Some(description),
            serde_json::json!(0),
        )]);
        let refused = source(&endpoint)
            .get_task(&"i1".into())
            .await
            .expect_err("a malformed slot");
        assert!(
            matches!(&refused, SourceError::Malformed { message } if message.contains(said)),
            "{description:?}: {refused:?}"
        );
    }
}

/// A status write's answer is the task its mutation's own selection reports, so that answer is
/// held to the issue asked for: an unsuccessful payload is a refusal, and a missing, partial or
/// foreign issue is a response this source cannot read — never a status reported as written.
/// An issue addressed by its identifier is the issue whose `identifier` that is.
#[tokio::test]
async fn a_status_write_is_answered_only_by_the_issue_it_addressed() {
    let issue = |id: &str| held_issue(id)["issue"].clone();
    let mut partial = issue("I-1");
    partial.as_object_mut().unwrap().remove("state");
    for (payload, said) in [
        (
            serde_json::json!({"issueUpdate":{"success":false,"issue":null}}),
            "issueUpdate",
        ),
        (
            serde_json::json!({"issueUpdate":{"success":true,"issue":null}}),
            "missing issueUpdate.issue",
        ),
        (
            serde_json::json!({"issueUpdate":{"success":true,"issue":issue("I-2")}}),
            "answered with the issue I-2",
        ),
        (
            serde_json::json!({"issueUpdate":{"success":true,"issue":partial}}),
            "missing state",
        ),
    ] {
        let (endpoint, _) = response_server(vec![writable_resolution(), payload.clone()]);
        let refused = writable_source(&endpoint)
            .set_task_status(&"I-1".into(), StatusCategory::Todo)
            .await
            .expect_err("not this write landing");
        assert!(refused.to_string().contains(said), "{payload}: {refused}");
    }
    // Addressed by its identifier, answered by the issue that identifier names.
    let (endpoint, wire) = response_server(vec![
        writable_resolution(),
        serde_json::json!({"issueUpdate":{"success":true,"issue":issue("I-1")}}),
    ]);
    assert_eq!(
        writable_source(&endpoint)
            .set_task_status(&identifier("I-1").as_str().into(), StatusCategory::Todo)
            .await
            .unwrap(),
        Some(Status {
            category: StatusCategory::Todo,
            name: "Todo".into()
        })
    );
    let sent = wire
        .iter()
        .map(|request| sent(&request))
        .collect::<Vec<_>>();
    assert_eq!(
        sent[1]["variables"],
        serde_json::json!({"id": identifier("I-1"), "input": {"stateId": "STATE"}})
    );
}
