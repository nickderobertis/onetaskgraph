//! The janitor's GraphQL documents against the GitHub Projects plugin's pinned schema, so a
//! field, an argument or a variable type GitHub does not have fails here, offline — with the
//! three the live lane shares with it from `onetaskgraph-github-live`: the field delete both
//! send, and the lane's board pre-flight's measurement and item listing.
use std::collections::HashMap;

use graphql_parser::{query, schema};
use onetaskgraph_github_live::{
    ADD_DRAFT_DOCUMENT, BOARD_WORKLOAD_DOCUMENT, COPY_BOARD_DOCUMENT, DELETE_BOARD_DOCUMENT,
    DELETE_FIELD_DOCUMENT, RESIDUE_ITEMS_DOCUMENT, RUN_BOARDS_DOCUMENT, TEMPLATE_DOCUMENT,
    TEMPLATE_ITEMS_DOCUMENT,
};
use onetaskgraph_live_janitor::{BOARD_DOCUMENT, DELETE_ISSUE_DOCUMENT, DELETE_ITEM_DOCUMENT};

const PINNED: &str =
    include_str!("../../onetaskgraph-github-projects/tests/fixtures/schema.graphql");

struct Schema<'a> {
    fields: HashMap<&'a str, &'a [schema::Field<'a, String>]>,
    members: HashMap<&'a str, Vec<&'a str>>,
}

impl<'a> Schema<'a> {
    fn new(document: &'a schema::Document<'a, String>) -> Self {
        let mut fields = HashMap::new();
        let mut members: HashMap<&str, Vec<&str>> = HashMap::new();
        for definition in &document.definitions {
            let schema::Definition::TypeDefinition(definition) = definition else {
                continue;
            };
            match definition {
                schema::TypeDefinition::Object(object) => {
                    fields.insert(object.name.as_str(), object.fields.as_slice());
                    members
                        .entry(object.name.as_str())
                        .or_default()
                        .push(object.name.as_str());
                    for interface in &object.implements_interfaces {
                        members
                            .entry(interface.as_str())
                            .or_default()
                            .push(object.name.as_str());
                    }
                }
                schema::TypeDefinition::Interface(interface) => {
                    fields.insert(interface.name.as_str(), interface.fields.as_slice());
                }
                schema::TypeDefinition::Union(union) => {
                    members
                        .entry(union.name.as_str())
                        .or_default()
                        .extend(union.types.iter().map(String::as_str));
                }
                _ => {}
            }
        }
        Self { fields, members }
    }

    /// Whether a fragment on `condition` can apply where a value of `parent` is.
    fn overlaps(&self, parent: &str, condition: &str) -> bool {
        let of = |name| self.members.get(name).cloned().unwrap_or_default();
        of(parent)
            .iter()
            .any(|member| of(condition).contains(member))
    }

    fn validate(
        &self,
        parent: &str,
        selection: &query::SelectionSet<'_, String>,
        variables: &HashMap<&str, String>,
    ) {
        for item in &selection.items {
            match item {
                query::Selection::Field(selected) if selected.name == "__typename" => {}
                query::Selection::Field(selected) => {
                    let field = self
                        .fields
                        .get(parent)
                        .and_then(|fields| fields.iter().find(|f| f.name == selected.name))
                        .unwrap_or_else(|| {
                            panic!("pinned schema {parent} lacks field {}", selected.name)
                        });
                    for (name, value) in &selected.arguments {
                        let argument = field
                            .arguments
                            .iter()
                            .find(|argument| argument.name == *name)
                            .unwrap_or_else(|| {
                                panic!("pinned {parent}.{} lacks argument {name}", field.name)
                            });
                        match value {
                            query::Value::Int(_) if named(&argument.value_type) == "Int" => {}
                            query::Value::Variable(variable) => {
                                let declared = &variables[variable.as_str()];
                                let wanted = argument.value_type.to_string();
                                assert!(
                                    *declared == wanted
                                        || declared.strip_suffix('!') == Some(&wanted),
                                    "${variable}: {declared} cannot be {parent}.{}({name}: {wanted})",
                                    field.name
                                );
                            }
                            _ => panic!(
                                "{parent}.{}({name}:) takes a variable or a literal of its type",
                                field.name
                            ),
                        }
                    }
                    for required in field.arguments.iter().filter(|argument| {
                        matches!(argument.value_type, schema::Type::NonNullType(_))
                            && argument.default_value.is_none()
                    }) {
                        assert!(
                            selected.arguments.iter().any(|(n, _)| *n == required.name),
                            "{parent}.{} omits required {}",
                            field.name,
                            required.name
                        );
                    }
                    if !selected.selection_set.items.is_empty() {
                        self.validate(named(&field.field_type), &selected.selection_set, variables);
                    }
                }
                query::Selection::InlineFragment(fragment) => {
                    let condition = match &fragment.type_condition {
                        Some(query::TypeCondition::On(name)) => name.as_str(),
                        None => parent,
                    };
                    assert!(
                        self.overlaps(parent, condition),
                        "a fragment on {condition} can never apply to {parent}"
                    );
                    self.validate(condition, &fragment.selection_set, variables);
                }
                query::Selection::FragmentSpread(spread) => {
                    panic!(
                        "no named fragment is defined, yet ...{} is spread",
                        spread.fragment_name
                    )
                }
            }
        }
    }
}

fn named<'a>(kind: &'a schema::Type<'a, String>) -> &'a str {
    match kind {
        schema::Type::NamedType(name) => name,
        schema::Type::ListType(inner) | schema::Type::NonNullType(inner) => named(inner),
    }
}

fn validate(document: &str) {
    validate_against(document, PINNED);
}

fn validate_against(document: &str, pinned: &str) {
    let pinned = schema::parse_schema::<String>(pinned).expect("the pinned schema parses");
    let schema = Schema::new(&pinned);
    let parsed = query::parse_query::<String>(document).expect("the document parses");
    let [query::Definition::Operation(operation)] = parsed.definitions.as_slice() else {
        panic!("one operation per document");
    };
    let (root, declared, selection) = match operation {
        query::OperationDefinition::Query(query) => {
            ("Query", &query.variable_definitions, &query.selection_set)
        }
        query::OperationDefinition::Mutation(mutation) => (
            "Mutation",
            &mutation.variable_definitions,
            &mutation.selection_set,
        ),
        _ => panic!("the janitor sends queries and mutations only"),
    };
    let variables = declared
        .iter()
        .map(|variable| (variable.name.as_str(), variable.var_type.to_string()))
        .collect();
    schema.validate(root, selection, &variables);
}

#[test]
fn every_janitor_document_is_valid_against_the_pinned_schema() {
    for document in [
        BOARD_DOCUMENT,
        DELETE_ISSUE_DOCUMENT,
        DELETE_ITEM_DOCUMENT,
        DELETE_FIELD_DOCUMENT,
        BOARD_WORKLOAD_DOCUMENT,
        RESIDUE_ITEMS_DOCUMENT,
        TEMPLATE_DOCUMENT,
        COPY_BOARD_DOCUMENT,
        DELETE_BOARD_DOCUMENT,
        RUN_BOARDS_DOCUMENT,
        TEMPLATE_ITEMS_DOCUMENT,
        ADD_DRAFT_DOCUMENT,
    ] {
        validate(document);
    }
}

#[test]
fn a_field_github_does_not_have_is_refused() {
    for (drifted, reason) in [
        (
            BOARD_DOCUMENT.replace("repositoryOwner(", "user("),
            "Query lacks field user",
        ),
        (
            DELETE_ISSUE_DOCUMENT.replace("repository{nameWithOwner}", "clientMutationId"),
            "DeleteIssuePayload lacks field clientMutationId",
        ),
        (
            DELETE_ITEM_DOCUMENT.replace("DeleteProjectV2ItemInput!", "String!"),
            "String! cannot be",
        ),
        (
            BOARD_DOCUMENT.replace("$number:Int!", "$number:String!"),
            "String! cannot be",
        ),
        (
            DELETE_FIELD_DOCUMENT.replace("DeleteProjectV2FieldInput!", "ID!"),
            "ID! cannot be",
        ),
        (
            RESIDUE_ITEMS_DOCUMENT.replace("repository{nameWithOwner}", "repo{nameWithOwner}"),
            "Issue lacks field repo",
        ),
        (
            BOARD_WORKLOAD_DOCUMENT.replace("{id name}", "{id label}"),
            "ProjectV2Field lacks field label",
        ),
    ] {
        let refusal = std::panic::catch_unwind(|| validate(&drifted))
            .expect_err("a document GitHub would refuse fails offline");
        let message = refusal
            .downcast_ref::<String>()
            .cloned()
            .unwrap_or_default();
        assert!(message.contains(reason), "{reason}: {message}");
    }
}

// The one statement of Linear's complexity model, shared with the Linear lanes' replays; the
// janitor depends on that crate, so a change there selects these tests too.
#[path = "../../onetaskgraph-linear/tests/document_lifecycle/complexity.rs"]
mod complexity;

/// Schema validation cannot see Linear's complexity cap, which refuses a document over it
/// with HTTP 400.
#[test]
fn linear_documents_stay_under_linears_complexity_cap() {
    use onetaskgraph_live_janitor::linear::{DELETE_DOCUMENT, DELETE_PROJECT, DOCUMENTS, PROJECTS};
    let variables = serde_json::json!({"after":null,"filter":{},"id":"d"});
    for document in [DOCUMENTS, DELETE_DOCUMENT, PROJECTS, DELETE_PROJECT] {
        assert_eq!(
            complexity::refusal(document, &variables),
            None,
            "{document}"
        );
    }
}

#[test]
fn linear_documents_match_the_pinned_upstream_schema() {
    use onetaskgraph_live_janitor::linear::{
        DELETE_DOCUMENT, DELETE_PROJECT, DOCUMENTS, LEASE_READ, LEASE_RELEASE, PROJECTS,
    };
    let upstream = std::process::Command::new("python3")
        .args([
            "-c",
            "import gzip,sys; sys.stdout.buffer.write(gzip.open(sys.argv[1], 'rb').read())",
        ])
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/linear-upstream.graphql.gz"
        ))
        .output()
        .expect("read the pinned upstream schema");
    assert!(upstream.status.success());
    let pinned = String::from_utf8(upstream.stdout).expect("upstream schema is UTF-8");
    for document in [
        DOCUMENTS,
        DELETE_DOCUMENT,
        PROJECTS,
        DELETE_PROJECT,
        LEASE_READ,
        LEASE_RELEASE,
    ] {
        validate_against(document, &pinned);
    }
    let input_schema = schema::parse_schema::<String>(&pinned).expect("upstream parses");
    let inputs: HashMap<_, _> = input_schema
        .definitions
        .iter()
        .filter_map(|definition| {
            if let schema::Definition::TypeDefinition(schema::TypeDefinition::InputObject(input)) =
                definition
            {
                Some((input.name.as_str(), input.fields.as_slice()))
            } else {
                None
            }
        })
        .collect();
    validate_input(
        &inputs,
        "DocumentFilter",
        &serde_json::to_value(onetaskgraph_live_janitor::linear::document_filter()).unwrap(),
    );
    validate_input(
        &inputs,
        "ProjectFilter",
        &serde_json::to_value(onetaskgraph_live_janitor::linear::project_filter()).unwrap(),
    );
    let parsed = schema::parse_schema::<String>(&pinned).expect("upstream parses");
    let schema = Schema::new(&parsed);
    let query = query::parse_query::<String>(DOCUMENTS).unwrap();
    let query::Definition::Operation(query::OperationDefinition::Query(operation)) =
        &query.definitions[0]
    else {
        panic!("document listing is a query")
    };
    let response: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/linear-documents.json")).unwrap();
    validate_response(
        &schema,
        "Query",
        &operation.selection_set,
        &response["data"],
    );
    let mutation = query::parse_query::<String>(DELETE_DOCUMENT).unwrap();
    let query::Definition::Operation(query::OperationDefinition::Mutation(operation)) =
        &mutation.definitions[0]
    else {
        panic!("document deletion is a mutation")
    };
    let response: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/linear-document-delete.json")).unwrap();
    validate_response(
        &schema,
        "Mutation",
        &operation.selection_set,
        &response["data"],
    );
    for (document, fixture) in [
        (PROJECTS, include_str!("fixtures/linear-projects.json")),
        (
            DELETE_PROJECT,
            include_str!("fixtures/linear-project-delete.json"),
        ),
    ] {
        let parsed = query::parse_query::<String>(document).unwrap();
        let (parent, selection) = match &parsed.definitions[0] {
            query::Definition::Operation(query::OperationDefinition::Query(operation)) => {
                ("Query", &operation.selection_set)
            }
            query::Definition::Operation(query::OperationDefinition::Mutation(operation)) => {
                ("Mutation", &operation.selection_set)
            }
            _ => panic!("a project operation is a query or a mutation"),
        };
        let response: serde_json::Value = serde_json::from_str(fixture).unwrap();
        validate_response(&schema, parent, selection, &response["data"]);
    }
}

fn validate_response(
    schema: &Schema<'_>,
    parent: &str,
    selection: &query::SelectionSet<'_, String>,
    value: &serde_json::Value,
) {
    let object = value.as_object().expect("selected object is an object");
    assert_eq!(
        object.len(),
        selection.items.len(),
        "response carries exactly the selected fields of {parent}"
    );
    for item in &selection.items {
        let query::Selection::Field(selected) = item else {
            panic!("Linear selections are fields")
        };
        let field = schema.fields[parent]
            .iter()
            .find(|field| field.name == selected.name)
            .expect("selected field exists upstream");
        let value = object
            .get(selected.alias.as_ref().unwrap_or(&selected.name))
            .expect("selected field is present");
        validate_value(schema, &field.field_type, &selected.selection_set, value);
    }
}

fn validate_value(
    schema: &Schema<'_>,
    kind: &schema::Type<'_, String>,
    selection: &query::SelectionSet<'_, String>,
    value: &serde_json::Value,
) {
    if let schema::Type::NonNullType(inner) = kind {
        assert!(!value.is_null(), "non-null field is present");
        return validate_value(schema, inner, selection, value);
    }
    if value.is_null() {
        return;
    }
    match kind {
        schema::Type::ListType(inner) => {
            for item in value.as_array().expect("connection field is an array") {
                validate_value(schema, inner, selection, item);
            }
        }
        schema::Type::NamedType(name) if !selection.items.is_empty() => {
            validate_response(schema, name, selection, value)
        }
        schema::Type::NamedType(name) => match name.as_str() {
            "String" | "ID" | "DateTime" => assert!(value.is_string()),
            "Boolean" => assert!(value.is_boolean()),
            other => panic!("unexpected scalar {other}"),
        },
        schema::Type::NonNullType(_) => unreachable!(),
    }
}

fn validate_input<'a>(
    inputs: &HashMap<&str, &'a [schema::InputValue<'a, String>]>,
    name: &str,
    value: &serde_json::Value,
) {
    for (key, value) in value.as_object().expect("input object").iter() {
        let field = inputs[name]
            .iter()
            .find(|field| field.name == *key)
            .expect("input field exists upstream");
        let kind = named(&field.value_type);
        if inputs.contains_key(kind) {
            validate_input(inputs, kind, value);
        } else {
            assert_eq!(kind, "String");
            assert!(value.is_string());
        }
    }
}
