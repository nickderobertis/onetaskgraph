//! Deterministic drift check for the exact GitHub GraphQL production documents.

use graphql_parser::{query, schema};
use std::collections::{HashMap, HashSet};

fn named_type<'a>(kind: &'a schema::Type<'a, String>) -> &'a str {
    match kind {
        schema::Type::NamedType(name) => name,
        schema::Type::ListType(inner) | schema::Type::NonNullType(inner) => named_type(inner),
    }
}

fn selected_keys(selection: &query::SelectionSet<'_, String>, keys: &mut HashSet<String>) {
    for selected in &selection.items {
        match selected {
            query::Selection::Field(value) => {
                keys.insert(value.alias.as_ref().unwrap_or(&value.name).clone());
                selected_keys(&value.selection_set, keys);
            }
            query::Selection::InlineFragment(value) => selected_keys(&value.selection_set, keys),
            query::Selection::FragmentSpread(_) => {}
        }
    }
}

fn assert_fixture_keys(value: &serde_json::Value, selected: &HashSet<String>) {
    match value {
        serde_json::Value::Object(object) => {
            for (key, value) in object {
                assert!(
                    key == "__typename" || selected.contains(key),
                    "fixture key {key} is absent from its production operation"
                );
                assert_fixture_keys(value, selected);
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                assert_fixture_keys(value, selected);
            }
        }
        _ => {}
    }
}

#[test]
fn pinned_schema_checks_selected_fields_arguments_types_fragments_and_fixture_keys() {
    use onetaskgraph_github_projects::graphql;

    let schema = schema::parse_schema::<String>(include_str!("fixtures/schema.graphql")).unwrap();
    let fields = schema
        .definitions
        .iter()
        .filter_map(|definition| match definition {
            schema::Definition::TypeDefinition(schema::TypeDefinition::Object(value)) => {
                Some((value.name.as_str(), value.fields.as_slice()))
            }
            schema::Definition::TypeDefinition(schema::TypeDefinition::Interface(value)) => {
                Some((value.name.as_str(), value.fields.as_slice()))
            }
            _ => None,
        })
        .collect::<HashMap<_, _>>();
    let known_types = schema
        .definitions
        .iter()
        .filter_map(|definition| match definition {
            schema::Definition::TypeDefinition(value) => Some(match value {
                schema::TypeDefinition::Scalar(value) => value.name.as_str(),
                schema::TypeDefinition::Object(value) => value.name.as_str(),
                schema::TypeDefinition::Interface(value) => value.name.as_str(),
                schema::TypeDefinition::Union(value) => value.name.as_str(),
                schema::TypeDefinition::Enum(value) => value.name.as_str(),
                schema::TypeDefinition::InputObject(value) => value.name.as_str(),
            }),
            _ => None,
        })
        .collect::<HashSet<_>>();
    let possible_types = |name: &str| {
        let mut possible = HashSet::new();
        for definition in &schema.definitions {
            match definition {
                schema::Definition::TypeDefinition(schema::TypeDefinition::Object(value)) => {
                    if value.name == name
                        || value
                            .implements_interfaces
                            .iter()
                            .any(|interface| interface == name)
                    {
                        possible.insert(value.name.as_str());
                    }
                }
                schema::Definition::TypeDefinition(schema::TypeDefinition::Union(value))
                    if value.name == name =>
                {
                    possible.extend(value.types.iter().map(String::as_str));
                }
                _ => {}
            }
        }
        possible
    };

    fn validate<'query, 'schema>(
        type_name: &str,
        selection: &'query query::SelectionSet<'query, String>,
        fragments: &HashMap<&str, &'query query::FragmentDefinition<'query, String>>,
        fields: &HashMap<&str, &'schema [schema::Field<'schema, String>]>,
        known_types: &HashSet<&str>,
        possible_types: &impl Fn(&str) -> HashSet<&'schema str>,
        variables: &HashMap<&str, &query::Type<'query, String>>,
    ) {
        for selected in &selection.items {
            match selected {
                query::Selection::Field(selected) => {
                    if selected.name == "__typename" {
                        continue;
                    }
                    let field = fields
                        .get(type_name)
                        .and_then(|fields| fields.iter().find(|field| field.name == selected.name))
                        .unwrap_or_else(|| {
                            panic!(
                                "pinned schema {type_name} lacks selected field {}",
                                selected.name
                            )
                        });
                    for (argument, _) in &selected.arguments {
                        let schema_argument = field
                            .arguments
                            .iter()
                            .find(|value| value.name == *argument)
                            .unwrap_or_else(|| {
                                panic!(
                                    "pinned schema {type_name}.{} lacks argument {argument}",
                                    selected.name
                                )
                            });
                        let query::Value::Variable(variable) = &selected
                            .arguments
                            .iter()
                            .find(|(name, _)| name == argument)
                            .unwrap()
                            .1
                        else {
                            panic!("production arguments must use declared variables")
                        };
                        let variable_type = format!("{:?}", variables[variable.as_str()]);
                        let argument_type = format!("{:?}", schema_argument.value_type);
                        let compatible = variable_type == argument_type
                            || variable_type
                                .strip_prefix("NonNullType(")
                                .and_then(|value| value.strip_suffix(')'))
                                == Some(argument_type.as_str());
                        assert!(
                            compatible,
                            "variable ${variable} no longer matches {type_name}.{}({argument}:): {variable_type:?} versus {:?}",
                            selected.name, schema_argument.value_type
                        );
                    }
                    for required in field.arguments.iter().filter(|argument| {
                        matches!(argument.value_type, schema::Type::NonNullType(_))
                            && argument.default_value.is_none()
                    }) {
                        assert!(
                            selected
                                .arguments
                                .iter()
                                .any(|(name, _)| name == &required.name),
                            "production operation omits required {type_name}.{}({}:)",
                            selected.name,
                            required.name
                        );
                    }
                    if !selected.selection_set.items.is_empty() {
                        validate(
                            named_type(&field.field_type),
                            &selected.selection_set,
                            fragments,
                            fields,
                            known_types,
                            possible_types,
                            variables,
                        );
                    }
                }
                query::Selection::InlineFragment(fragment) => {
                    let condition = fragment.type_condition.as_ref().map_or(type_name, |value| {
                        let query::TypeCondition::On(name) = value;
                        name.as_str()
                    });
                    assert!(
                        known_types.contains(condition),
                        "pinned schema lacks {condition}"
                    );
                    assert!(
                        !possible_types(type_name).is_disjoint(&possible_types(condition)),
                        "fragment on {condition} cannot apply to {type_name}"
                    );
                    validate(
                        condition,
                        &fragment.selection_set,
                        fragments,
                        fields,
                        known_types,
                        possible_types,
                        variables,
                    );
                }
                query::Selection::FragmentSpread(spread) => {
                    let fragment = fragments[spread.fragment_name.as_str()];
                    let query::TypeCondition::On(condition) = &fragment.type_condition;
                    assert!(
                        known_types.contains(condition.as_str()),
                        "pinned schema lacks {condition}"
                    );
                    assert!(
                        !possible_types(type_name).is_disjoint(&possible_types(condition)),
                        "fragment {condition} cannot apply to {type_name}"
                    );
                    validate(
                        condition,
                        &fragment.selection_set,
                        fragments,
                        fields,
                        known_types,
                        possible_types,
                        variables,
                    );
                }
            }
        }
    }

    for (operation, fixture_pointer, fixture) in [
        (
            graphql::PROJECT,
            Some("/data/owner"),
            Some(include_str!("fixtures/project.json")),
        ),
        (graphql::TASK_DEPENDENCIES, None, None),
        (graphql::RELATED_PROJECTS, None, None),
        (
            graphql::PROJECT_DEPENDENCIES,
            Some("/data/node"),
            Some(include_str!("fixtures/dependencies.json")),
        ),
        (graphql::CREATE_DRAFT, None, None),
        (graphql::UPDATE_DRAFT, None, None),
        (graphql::UPDATE_ISSUE, None, None),
        (graphql::UPDATE_FIELD, None, None),
        (graphql::UPDATE_PROJECT, None, None),
        (graphql::ADD_BLOCKED_BY, None, None),
        (graphql::REMOVE_BLOCKED_BY, None, None),
    ] {
        let document = query::parse_query::<String>(operation).unwrap();
        let fragments = document
            .definitions
            .iter()
            .filter_map(|definition| match definition {
                query::Definition::Fragment(fragment) => Some((fragment.name.as_str(), fragment)),
                _ => None,
            })
            .collect::<HashMap<_, _>>();
        let (root, variable_definitions, selection_set) = match &document.definitions[0] {
            query::Definition::Operation(query::OperationDefinition::Query(operation)) => (
                "Query",
                &operation.variable_definitions,
                &operation.selection_set,
            ),
            query::Definition::Operation(query::OperationDefinition::Mutation(operation)) => (
                "Mutation",
                &operation.variable_definitions,
                &operation.selection_set,
            ),
            _ => panic!("production document must begin with a named query or mutation"),
        };
        for variable in variable_definitions {
            assert!(
                known_types.contains(named_type(&variable.var_type)),
                "pinned schema lacks variable type {}",
                named_type(&variable.var_type)
            );
        }
        let variables = variable_definitions
            .iter()
            .map(|variable| (variable.name.as_str(), &variable.var_type))
            .collect::<HashMap<_, _>>();
        validate(
            root,
            selection_set,
            &fragments,
            &fields,
            &known_types,
            &possible_types,
            &variables,
        );

        if let (Some(pointer), Some(fixture)) = (fixture_pointer, fixture) {
            let fixture: serde_json::Value = serde_json::from_str(fixture).unwrap();
            let mut keys = HashSet::new();
            selected_keys(selection_set, &mut keys);
            for fragment in fragments.values() {
                selected_keys(&fragment.selection_set, &mut keys);
            }
            assert_fixture_keys(fixture.pointer(pointer).unwrap(), &keys);
        }
    }
}
