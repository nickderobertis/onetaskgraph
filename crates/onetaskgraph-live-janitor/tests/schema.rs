//! The janitor's GraphQL documents against the GitHub Projects plugin's pinned schema, so a
//! field, an argument or a variable type GitHub does not have fails here, offline.
use std::collections::HashMap;

use graphql_parser::{query, schema};
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
    let pinned = schema::parse_schema::<String>(PINNED).expect("the pinned schema parses");
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
    for document in [BOARD_DOCUMENT, DELETE_ISSUE_DOCUMENT, DELETE_ITEM_DOCUMENT] {
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
