//! Deterministic drift check for the exact GitHub GraphQL production documents.
//!
//! The pinned subset defines no `updateProjectV2`, no `ProjectV2.shortDescription` and no
//! `ProjectV2.readme`, so a document that reached for one would fail here rather than
//! rename a user's board. It deliberately defines `updateProjectV2Field` for the one
//! guarded status-options document, and validates the exact document production sends.

use graphql_parser::{query, schema};
use std::collections::{HashMap, HashSet};

// Shared with the two GitHub doubles, which call the halves of it this file does not.
#[allow(dead_code)]
mod github_schema;

#[test]
fn status_option_colors_match_the_pinned_graphql_vocabulary() {
    use onetaskgraph_github_projects::StatusOptionColor;

    let rust_schema = schemars::schema_for!(StatusOptionColor);
    let rust_value = rust_schema.as_value();
    let rust_variants = rust_value
        .get("enum")
        .or_else(|| rust_value.get("oneOf"))
        .and_then(serde_json::Value::as_array)
        .expect("StatusOptionColor is a closed enum");
    let rust_colors = rust_variants
        .iter()
        .map(|color| {
            color
                .as_str()
                .or_else(|| color.get("const").and_then(serde_json::Value::as_str))
                .expect("a color token")
                .to_owned()
        })
        .collect::<HashSet<_>>();
    let graphql_schema =
        schema::parse_schema::<String>(include_str!("fixtures/schema.graphql")).unwrap();
    let graphql_colors = graphql_schema
        .definitions
        .iter()
        .find_map(|definition| match definition {
            schema::Definition::TypeDefinition(schema::TypeDefinition::Enum(value))
                if value.name == "ProjectV2SingleSelectFieldOptionColor" =>
            {
                Some(
                    value
                        .values
                        .iter()
                        .map(|color| color.name.clone())
                        .collect::<HashSet<_>>(),
                )
            }
            _ => None,
        })
        .expect("the pinned schema defines GitHub's option colors");
    assert_eq!(rust_colors, graphql_colors);
}

/// Every enum member a value names, inside lists and objects alike.
fn enum_values<'a>(value: &'a query::Value<'a, String>) -> Vec<&'a str> {
    match value {
        query::Value::Enum(member) => vec![member.as_str()],
        query::Value::List(values) => values.iter().flat_map(enum_values).collect(),
        query::Value::Object(fields) => fields.values().flat_map(enum_values).collect(),
        _ => Vec::new(),
    }
}

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
    // Every enum's members, so a variable's default naming one can be held to the pin.
    let enum_members = schema
        .definitions
        .iter()
        .filter_map(|definition| match definition {
            schema::Definition::TypeDefinition(schema::TypeDefinition::Enum(value)) => Some((
                value.name.as_str(),
                value
                    .values
                    .iter()
                    .map(|member| member.name.as_str())
                    .collect::<HashSet<_>>(),
            )),
            _ => None,
        })
        .collect::<HashMap<_, _>>();
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
            graphql::BOARD,
            Some("/data/owner"),
            Some(include_str!("fixtures/project.json")),
        ),
        (
            graphql::SEARCH_ISSUES,
            Some("/data/search"),
            Some(include_str!("fixtures/issues.json")),
        ),
        (graphql::ISSUE, None, None),
        (graphql::DOCUMENT_IN_PROJECT, None, None),
        (graphql::BOARD_FIELDS, None, None),
        (
            graphql::ORIGIN_LOOKUP,
            Some("/data"),
            Some(include_str!("fixtures/origin-lookup.json")),
        ),
        (graphql::DRAFT, None, None),
        (
            graphql::ISSUE_BOARD_ITEMS,
            Some("/data/node"),
            Some(include_str!("fixtures/issue-board-items.json")),
        ),
        (
            graphql::SUB_ISSUES,
            Some("/data/node"),
            Some(include_str!("fixtures/sub-issues.json")),
        ),
        (graphql::REPOSITORY, None, None),
        (graphql::CREATION_CONTEXT, None, None),
        (graphql::ORIGIN_LOOKUPS, None, None),
        (graphql::ORIGIN_LOOKUPS_WITH_MEMBERS, None, None),
        (graphql::CREATED_BOARD_ITEMS, None, None),
        (graphql::CREATE_ISSUES, None, None),
        (graphql::WRITE_FIELD_VALUES, None, None),
        (graphql::CLOSE_ISSUES, None, None),
        (graphql::UPDATE_ISSUES, None, None),
        (graphql::ADD_BLOCKED_BYS, None, None),
        (graphql::WRITE_FIELDS_AND_EDGES, None, None),
        (graphql::DELETE_ISSUES, None, None),
        (
            graphql::ISSUE_DEPENDENCIES,
            Some("/data/node"),
            Some(include_str!("fixtures/dependencies.json")),
        ),
        (
            graphql::ISSUE_DEPENDS_ON,
            Some("/data"),
            Some(include_str!("fixtures/dependencies-depends-on.json")),
        ),
        (
            graphql::ISSUE_DEPENDED_ON_BY,
            Some("/data"),
            Some(include_str!("fixtures/dependencies-depended-on-by.json")),
        ),
        (graphql::CREATE_ISSUE, None, None),
        (graphql::ADD_TO_BOARD, None, None),
        (graphql::UPDATE_ISSUE, None, None),
        (graphql::UPDATE_DRAFT, None, None),
        (graphql::UPDATE_FIELD, None, None),
        (graphql::UPDATE_FIELDS, None, None),
        (graphql::CLEAR_FIELD, None, None),
        (graphql::CREATE_FIELD, None, None),
        (graphql::STATUS_OPTIONS_UPDATE, None, None),
        (graphql::STATUS_OPTIONS_SNAPSHOT, None, None),
        (graphql::ADD_SUB_ISSUE, None, None),
        (graphql::REMOVE_SUB_ISSUE, None, None),
        (graphql::ADD_BLOCKED_BY, None, None),
        (graphql::REMOVE_BLOCKED_BY, None, None),
        (graphql::DELETE_ISSUE, None, None),
        (
            graphql::ISSUE_COMMENTS,
            Some("/data/node"),
            Some(include_str!("fixtures/comments.json")),
        ),
        (graphql::ISSUE_DETAIL, None, None),
        (graphql::ISSUE_DETAILS, None, None),
        (graphql::COMMENT_ISSUE, None, None),
        (graphql::ADD_COMMENT, None, None),
        (graphql::UPDATE_COMMENT, None, None),
        (graphql::DELETE_COMMENT, None, None),
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
            // A default is part of what the document sends, so every enum member it names is
            // one the pinned enum declares — and the credentialed lane holds GitHub to the pin.
            if let Some(default) = &variable.default_value {
                let enum_name = named_type(&variable.var_type);
                for member in enum_values(default) {
                    let members = enum_members.get(enum_name).unwrap_or_else(|| {
                        panic!("${} defaults to enum members of {enum_name}, which the pinned schema declares no enum of", variable.name)
                    });
                    assert!(
                        members.contains(member),
                        "${} defaults to {member}, which the pinned {enum_name} does not declare",
                        variable.name
                    );
                }
            }
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

/// Nothing except the named guarded document can rewrite a Status field's options.
///
/// The pinned schema already refuses a *document* naming either, and this reads the whole
/// crate rather than the documents alone: the criterion is that no code path invokes
/// `updateProjectV2Field` with `singleSelectOptions`, and a source file is where a path
/// would have to be spelled. GitHub documents that input as overwriting a field's existing
/// options, so no addition is additive and a mistake destroys every item's status; a
/// status this board cannot represent is a refusal instead.
#[test]
fn only_the_guarded_source_path_writes_a_status_fields_option_set() {
    /// Everything the file says that is not a comment about what it does not do.
    fn code(text: &str, comment: &str) -> String {
        text.lines()
            .filter(|line| !line.trim_start().starts_with(comment))
            .collect::<Vec<_>>()
            .join("\n")
    }
    const FORBIDDEN: [&str; 3] = ["updateProjectV2(", "shortDescription", "readme"];
    for forbidden in FORBIDDEN {
        assert!(
            !code(include_str!("../src/lib.rs"), "//").contains(forbidden),
            "the source names {forbidden}, which this board is never written through"
        );
        assert!(
            !code(include_str!("fixtures/schema.graphql"), "#").contains(forbidden),
            "the pinned schema defines {forbidden}, which would let a document reach it"
        );
    }
    assert_eq!(
        code(include_str!("../src/lib.rs"), "//")
            .matches("updateProjectV2Field(input:$input)")
            .count(),
        1,
        "exactly the guarded mutation document may call updateProjectV2Field"
    );
}

/// The category list this source maps cannot silently lose a variant of the vocabulary.
///
/// `CATEGORIES` mirrors `StatusCategory`, and `category_position` is a wildcard-free
/// match over it — so a variant added to the shared vocabulary fails to compile there
/// until it is named, and this reconciliation fails until the list holds it too.
#[test]
fn every_status_category_this_source_can_be_handed_has_a_place_in_its_mapping() {
    use onetaskgraph_github_projects::{CATEGORIES, category_position};

    let mut filled = [false; CATEGORIES.len()];
    for category in CATEGORIES {
        let at = category_position(category);
        assert!(at < CATEGORIES.len(), "{category:?} sits past the list");
        assert!(
            !filled[at],
            "{category:?} shares a place with another category"
        );
        assert_eq!(CATEGORIES[at], category, "{category:?} is filed elsewhere");
        filled[at] = true;
    }
    assert!(
        filled.iter().all(|filled| *filled),
        "a place in the list is unfilled, so a category the vocabulary declares is missing"
    );
}

#[test]
fn the_category_list_is_reconciled_against_the_vocabulary_it_mirrors() {
    use onetaskgraph_github_projects::CATEGORIES;
    use onetaskgraph_plugin_api::StatusCategory;

    // The list checking itself proves nothing about the enum it restates: a variant added
    // to the shared vocabulary and given a place by `category_position` compiles with the
    // list one short, and every mapping indexed by that place then panics. `StatusCategory`'s
    // own derived schema is the second source — generated from the variants rather than
    // written beside them — so it grows the moment the vocabulary does, in its order.
    let schema = serde_json::to_value(schemars::schema_for!(StatusCategory))
        .expect("the derived schema serializes");
    let declared = schema["oneOf"]
        .as_array()
        .expect("a unit-variant enum schema lists its variants")
        .iter()
        .map(|variant| {
            variant["const"]
                .as_str()
                .expect("each variant is a string constant")
                .to_owned()
        })
        .collect::<Vec<_>>();
    let mirrored = CATEGORIES
        .iter()
        .map(|category| {
            serde_json::to_value(category)
                .expect("a category serializes")
                .as_str()
                .expect("a category serializes as a string")
                .to_owned()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        declared, mirrored,
        "CATEGORIES must name every category the vocabulary declares, in the order it \
         declares them"
    );
}

#[test]
fn documents_are_all_inventoried_with_what_the_source_is_doing_when_it_sends_one() {
    // `graphql::DOCUMENTS` is what a rate-limit diagnostic reads to name the call that was
    // refused, and a document missing from it would be reported as "talking to GitHub"
    // with nothing saying so. Nothing about a set of `&str` constants makes that a
    // compile error, so this reads the module back and is the gate instead.
    let source = include_str!("../src/lib.rs");
    let module = source
        .split_once("pub mod graphql {")
        .expect("the production documents live in one module")
        .1
        .split_once("\n}\n")
        .expect("that module ends")
        .0;
    let declared: Vec<&str> = module
        .lines()
        .filter_map(|line| line.trim().strip_prefix("pub const "))
        .filter_map(|rest| rest.split_once(':'))
        .map(|(name, _)| name.trim())
        .filter(|name| *name != "DOCUMENTS")
        .collect();
    let inventory = onetaskgraph_github_projects::graphql::DOCUMENTS;
    // Counting alone would let a duplicated entry stand in for an omitted document, so
    // distinctness is asserted too. Every entry is a compiler-checked reference to one of
    // the constants declared above it, so `as many entries as constants` and `no two
    // entries alike` together mean each constant is inventoried exactly once.
    let documents: HashSet<&str> = inventory.iter().map(|(document, _)| *document).collect();
    assert_eq!(
        documents.len(),
        inventory.len(),
        "graphql::DOCUMENTS holds the same document twice, which hides one it omits"
    );
    assert_eq!(
        declared.len(),
        inventory.len(),
        "graphql::DOCUMENTS names {} documents and the module declares {declared:?}; add the \
         missing one to that list with what this source is doing when it sends it",
        inventory.len()
    );
    let described: HashSet<&str> = inventory.iter().map(|(_, doing)| *doing).collect();
    assert_eq!(
        described.len(),
        inventory.len(),
        "two documents are inventoried as the same activity, so a diagnostic naming it \
         cannot say which call was refused"
    );
    for (document, doing) in inventory {
        assert!(
            !doing.trim().is_empty(),
            "every document is inventoried with what sending it is doing"
        );
        query::parse_query::<String>(document).expect("every inventoried document parses");
    }
}

#[test]
fn the_rate_limit_vocabulary_and_published_limits_match_their_pinned_artifact() {
    // GitHub's refusal wordings and its published secondary limits are GitHub's contract,
    // not this source's, and this source restates both: the wordings are what
    // `Limiter::classify` matches a refusal on, and the limits are the windows this source
    // paces under, the per-minute one also what `MIN_MUTATION_INTERVAL_MS` is derived from. Restating an external contract
    // without a gate is how the restatement quietly stops being true, so
    // `fixtures/rate-limits.json` is the pin — recorded with its provenance in the README
    // beside it — and this reconciles the two **both ways**: a wording the source matches
    // on that nothing pinned it, and a pinned wording the source no longer matches on,
    // each fail here naming the entry.
    let pinned: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/rate-limits.json")).expect("the pin parses");

    for (side, shipped) in [
        (
            "secondary",
            onetaskgraph_github_projects::SECONDARY_WORDINGS.as_slice(),
        ),
        (
            "primary",
            onetaskgraph_github_projects::PRIMARY_WORDINGS.as_slice(),
        ),
    ] {
        let pinned_wordings: HashSet<&str> = pinned[side]["wordings"]
            .as_array()
            .unwrap_or_else(|| panic!("the pin records the {side} wordings"))
            .iter()
            .map(|wording| wording.as_str().expect("each pinned wording is a string"))
            .collect();
        let shipped_wordings: HashSet<&str> = shipped.iter().copied().collect();
        assert_eq!(
            shipped_wordings.len(),
            shipped.len(),
            "the {side} wordings list the same phrase twice, which hides one it omits"
        );
        for wording in &shipped_wordings {
            assert!(
                pinned_wordings.contains(wording),
                "this source matches a {side} refusal on {wording:?}, which is not in \
                 fixtures/rate-limits.json; pin it there with where it was read, or stop \
                 matching on it"
            );
        }
        for wording in &pinned_wordings {
            assert!(
                shipped_wordings.contains(wording),
                "fixtures/rate-limits.json pins the {side} wording {wording:?} and this \
                 source no longer matches on it; a refusal GitHub still sends that way \
                 would be reported as something it is not"
            );
        }
        for wording in &shipped_wordings {
            assert_eq!(
                *wording,
                wording.to_ascii_lowercase(),
                "`classify` lower-cases the response before matching, so an upper-case \
                 character in {wording:?} could never match"
            );
        }
    }

    assert_eq!(
        pinned["content_creation"]["per_minute"].as_u64(),
        Some(onetaskgraph_github_projects::CONTENT_CREATION_PER_MINUTE),
        "the shipped per-minute ceiling is not the one pinned from GitHub's documentation"
    );
    assert_eq!(
        pinned["content_creation"]["per_hour"].as_u64(),
        Some(onetaskgraph_github_projects::CONTENT_CREATION_PER_HOUR),
        "the shipped per-hour ceiling is not the one pinned from GitHub's documentation"
    );
    for (key, shipped) in [
        (
            "per_minute",
            onetaskgraph_github_projects::SECONDARY_POINTS_PER_MINUTE,
        ),
        (
            "per_mutation_request",
            onetaskgraph_github_projects::SECONDARY_POINTS_PER_MUTATION,
        ),
        (
            "per_query_request",
            onetaskgraph_github_projects::SECONDARY_POINTS_PER_QUERY,
        ),
    ] {
        assert_eq!(
            pinned["secondary_points"][key].as_u64(),
            Some(shipped),
            "the shipped secondary points {key} is not the one pinned from GitHub's \
             documentation"
        );
    }
    let pinned_points: HashSet<&str> = pinned["secondary_points"]
        .as_object()
        .expect("the pin records the secondary points")
        .keys()
        .map(String::as_str)
        .filter(|key| *key != "read")
        .collect();
    assert_eq!(
        pinned_points,
        HashSet::from(["per_minute", "per_mutation_request", "per_query_request"]),
        "fixtures/rate-limits.json pins a secondary points figure this source does not pace by"
    );
    // And the derivation itself, because the pin is only worth having if the value this
    // source actually paces at moves with it.
    assert_eq!(
        onetaskgraph_github_projects::MIN_MUTATION_INTERVAL_MS,
        60_000 / onetaskgraph_github_projects::CONTENT_CREATION_PER_MINUTE,
        "the shipped interval stopped being the fastest rate the pinned per-minute \
         ceiling allows"
    );
}

/// The priority levels this source maps, and the configuration naming them, cannot silently
/// lose a priority the vocabulary gains.
///
/// `PRIORITY_LEVELS` and `PriorityMappingConfig`'s members both restate `Priority` less its
/// `none`, which is no option of a board field; `level_position` is a wildcard-free match, so a
/// new priority fails to compile there, and this reconciles both lists against the enum's own
/// derived schema — generated from the variants rather than written beside them.
#[test]
fn the_priority_levels_and_their_configuration_are_reconciled_against_the_vocabulary() {
    use onetaskgraph_github_projects::{
        GitHubProjectsConfig, PRIORITY_LEVELS, PriorityMappingConfig, level_position,
    };
    use onetaskgraph_plugin_api::Priority;

    let spelled = |value: serde_json::Value| {
        value
            .as_str()
            .expect("a priority serializes as a string")
            .to_owned()
    };
    let schema = serde_json::to_value(schemars::schema_for!(Priority))
        .expect("the derived schema serializes");
    let declared = schema["oneOf"]
        .as_array()
        .expect("a unit-variant enum schema lists its variants")
        .iter()
        .map(|variant| spelled(variant["const"].clone()))
        .filter(|name| name != "none")
        .collect::<Vec<_>>();
    let mirrored = PRIORITY_LEVELS
        .iter()
        .map(|level| spelled(serde_json::to_value(level).expect("serializes")))
        .collect::<Vec<_>>();
    assert_eq!(
        declared, mirrored,
        "PRIORITY_LEVELS must name every priority but none, in the order the vocabulary does"
    );
    for (index, level) in PRIORITY_LEVELS.iter().enumerate() {
        assert_eq!(level_position(*level), Some(index), "{level}");
    }
    assert_eq!(level_position(Priority::None), None);

    let config = serde_json::to_value(schemars::schema_for!(PriorityMappingConfig))
        .expect("the configuration schema serializes");
    let mut members = config["properties"]
        .as_object()
        .expect("the mapping is an object of levels")
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    members.sort();
    let mut levels = mirrored.clone();
    levels.sort();
    assert_eq!(
        members, levels,
        "priority_mapping must take exactly one member per level"
    );
    // And the whole configuration schema reaches it under the name a document writes.
    let whole = serde_json::to_value(schemars::schema_for!(GitHubProjectsConfig))
        .expect("the configuration schema serializes");
    assert!(whole["properties"]["priority_mapping"].is_object());
}

/// Each aliased field of `UPDATE_FIELDS`, in the order GitHub runs them: the key its answer
/// comes back under, the mutation it is, the variable carrying its input and the boolean that
/// includes it.
fn update_fields_slots(document: &str) -> Vec<(String, String, String, String)> {
    let document = query::parse_query::<String>(document).expect("a valid document");
    let query::Definition::Operation(query::OperationDefinition::Mutation(mutation)) =
        &document.definitions[0]
    else {
        panic!("UPDATE_FIELDS is a mutation");
    };
    let variable = |value: &query::Value<String>| match value {
        query::Value::Variable(name) => name.clone(),
        other => panic!("a slot is bound to a variable, not {other:?}"),
    };
    mutation
        .selection_set
        .items
        .iter()
        .map(|selection| {
            let query::Selection::Field(field) = selection else {
                panic!("UPDATE_FIELDS selects fields alone");
            };
            let input = field
                .arguments
                .iter()
                .find(|(name, _)| name == "input")
                .map(|(_, value)| variable(value))
                .expect("an input");
            let include = field
                .directives
                .iter()
                .find(|directive| directive.name == "include")
                .and_then(|directive| directive.arguments.first())
                .map(|(_, value)| variable(value))
                .expect("every slot is included by its own boolean");
            (
                field.alias.clone().unwrap_or_else(|| field.name.clone()),
                field.name.clone(),
                input,
                include,
            )
        })
        .collect()
}

/// `FIELD_WRITE_SLOTS` and `FIELD_CLEAR_SLOTS` are what the source and every loopback board
/// read `UPDATE_FIELDS`' aliases off, so they are held to the document itself, both ways and in
/// order: a slot added to one and not the other fails here rather than as a write whose answer
/// is read under the wrong key.
#[test]
fn the_field_slots_are_exactly_the_aliased_fields_of_update_fields() {
    use onetaskgraph_github_projects::{FIELD_CLEAR_SLOTS, FIELD_WRITE_SLOTS, graphql};
    let declared = FIELD_WRITE_SLOTS
        .iter()
        .map(|slot| (slot, "updateProjectV2ItemFieldValue"))
        .chain(
            FIELD_CLEAR_SLOTS
                .iter()
                .map(|slot| (slot, "clearProjectV2ItemFieldValue")),
        )
        .map(|(slot, mutation)| {
            (
                slot.alias.to_owned(),
                mutation.to_owned(),
                slot.variable.to_owned(),
                slot.include.to_owned(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(update_fields_slots(graphql::UPDATE_FIELDS), declared);
    // And the comparison is one that can fail: the same document with two slots' includes
    // swapped is refused.
    let swapped = graphql::UPDATE_FIELDS
        .replacen("@include(if:$writeSecond)", "@include(if:$PLACEHOLDER)", 1)
        .replacen("@include(if:$writeThird)", "@include(if:$writeSecond)", 1)
        .replacen("@include(if:$PLACEHOLDER)", "@include(if:$writeThird)", 1);
    assert_ne!(update_fields_slots(&swapped), declared);
}

/// A `github-projects` configuration serializes to a document it reads back as itself, with an
/// empty `metadata_fields`, and an entry's empty `path`, left out — so a configuration written
/// before either existed and one written with them empty are the same document.
#[test]
fn a_configuration_round_trips_and_leaves_its_empty_lists_out() {
    use onetaskgraph_github_projects::GitHubProjectsConfig;
    use serde_json::{Value, json};
    let base = json!({"owner": "octo-org", "project_number": 7, "repository": "acme/work"});
    let read = |document: &Value| -> GitHubProjectsConfig {
        serde_json::from_value(document.clone())
            .unwrap_or_else(|error| panic!("{document} does not read: {error}"))
    };
    let written = |config: &GitHubProjectsConfig| serde_json::to_value(config).unwrap();
    let with = |entries: Value| {
        let mut document = base.clone();
        document["metadata_fields"] = entries;
        document
    };

    // Absent and empty read alike, and both write no key at all.
    for document in [base.clone(), with(json!([]))] {
        let config = read(&document);
        assert!(config.metadata_fields.is_empty(), "{document}");
        let serialized = written(&config);
        assert_eq!(serialized.get("metadata_fields"), None, "{serialized}");
        assert_eq!(read(&serialized), config, "{serialized}");
    }
    assert_eq!(read(&base), read(&with(json!([]))));

    // Populated: every entry kept in order; a path is written only when it has a step.
    let populated = read(&with(json!([
        {"field": "Host", "key": "orchestrator.follow-up", "path": ["host"]},
        {"field": "Team", "key": "team.name", "path": []},
        {"field": "Owner", "key": "team.owner"},
    ])));
    let serialized = written(&populated);
    assert_eq!(
        serialized["metadata_fields"],
        json!([
            {"field": "Host", "key": "orchestrator.follow-up", "path": ["host"]},
            {"field": "Team", "key": "team.name"},
            {"field": "Owner", "key": "team.owner"},
        ])
    );
    let again = read(&serialized);
    assert_eq!(again, populated);
    assert_eq!(
        written(&again),
        serialized,
        "a second round trip writes the same bytes"
    );
    assert_eq!(populated.metadata_fields[1].path, Vec::<String>::new());

    // Every other member survives the trip too, defaults included.
    let full = read(&json!({
        "owner": "octo-org", "project_number": 7, "repository": "acme/work",
        "token_env": "BOARD_TOKEN", "endpoint": "https://github.example/api/graphql",
        "status_mapping": {"todo": "Ready", "draft": null},
        "priority_mapping": {"urgent": "P0"},
        "pacing": {"min_mutation_interval_ms": 0, "retry_budget_ms": 0},
        "metadata_fields": [{"field": "Host", "key": "orchestrator.follow-up", "path": ["host"]}],
    }));
    assert_eq!(read(&written(&full)), full);
}

#[test]
fn every_batch_document_declares_and_sends_exactly_the_slots_its_constant_counts() {
    // The batch documents spell their slots out one by one, and the source fills and reads them
    // by the constants: each prefix's input variable, its include variable and its alias are
    // declared exactly that many times, numbered from nought, and no more.
    use onetaskgraph_github_projects::{DELETE_BATCH, MUTATION_BATCH, WRITE_BATCH, graphql};
    // Each document by name, the document, each kind's slot prefix and include prefix, and how
    // many slots its constant counts.
    type Batch<'a> = (&'a str, &'a str, &'a [(&'a str, &'a str)], usize);
    let batches: [Batch<'_>; 7] = [
        (
            "CREATE_ISSUES",
            graphql::CREATE_ISSUES,
            &[("create", "send")],
            MUTATION_BATCH,
        ),
        (
            "CLOSE_ISSUES",
            graphql::CLOSE_ISSUES,
            &[("close", "send")],
            MUTATION_BATCH,
        ),
        (
            "UPDATE_ISSUES",
            graphql::UPDATE_ISSUES,
            &[("update", "send")],
            MUTATION_BATCH,
        ),
        (
            "WRITE_FIELD_VALUES",
            graphql::WRITE_FIELD_VALUES,
            &[("field", "send")],
            WRITE_BATCH,
        ),
        (
            "ADD_BLOCKED_BYS",
            graphql::ADD_BLOCKED_BYS,
            &[("block", "send")],
            WRITE_BATCH,
        ),
        (
            "WRITE_FIELDS_AND_EDGES",
            graphql::WRITE_FIELDS_AND_EDGES,
            &[("field", "sendField"), ("block", "sendBlock")],
            WRITE_BATCH,
        ),
        (
            "DELETE_ISSUES",
            graphql::DELETE_ISSUES,
            &[("delete", "send")],
            DELETE_BATCH,
        ),
    ];
    for (name, document, kinds, slots) in batches {
        for (prefix, send) in kinds {
            let numbered = |pattern: &str| -> Vec<usize> {
                let mut found = Vec::new();
                let mut rest = document;
                while let Some(at) = rest.find(pattern) {
                    rest = &rest[at + pattern.len()..];
                    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
                    if !digits.is_empty() && rest[digits.len()..].starts_with([':', ')']) {
                        found.push(digits.parse().unwrap());
                    }
                }
                found.sort_unstable();
                found
            };
            let expected: Vec<usize> = (0..slots).collect();
            assert_eq!(
                numbered(&format!("${prefix}")),
                expected
                    .iter()
                    .flat_map(|slot| [*slot, *slot])
                    .collect::<Vec<_>>(),
                "{name} declares and passes `${prefix}<n>` other than once each for {slots} slots"
            );
            assert_eq!(
                numbered(&format!("${send}")),
                expected
                    .iter()
                    .flat_map(|slot| [*slot, *slot])
                    .collect::<Vec<_>>(),
                "{name} declares and reads `${send}<n>` other than once each for {slots} slots"
            );
            assert_eq!(
                numbered(&format!(" {prefix}")),
                expected,
                "{name} aliases `{prefix}<n>` other than once each for {slots} slots"
            );
        }
    }
}

/// Every document this source sends holds to GitHub's complete published schema, not only to
/// the reduced subset above: the subset is written by the same hand as the documents, which is
/// how a field-setup input GitHub does not define passed the check above in 0.12.13.
#[test]
fn every_production_document_validates_against_githubs_complete_published_schema() {
    let refused = onetaskgraph_github_projects::graphql::DOCUMENTS
        .iter()
        .filter_map(|(document, doing)| {
            github_schema::validate_document(document)
                .err()
                .map(|errors| format!("{doing}: {errors:?}"))
        })
        .collect::<Vec<_>>();
    assert!(
        refused.is_empty(),
        "GitHub would refuse:\n{}",
        refused.join("\n")
    );
}

/// The field setup's two writes, with the variables it sends, are what GitHub accepts — and
/// the input 0.12.13 sent is refused with the message GitHub answered it with.
#[test]
fn the_field_setup_requests_validate_and_the_input_0_12_13_sent_is_refused() {
    use onetaskgraph_github_projects::graphql::{CREATE_FIELD, STATUS_OPTIONS_UPDATE};
    use serde_json::json;

    let options = json!([
        {"id": "opt-todo", "name": "Todo", "color": "GRAY", "description": ""},
        {"name": "Cancelled", "color": "GRAY", "description": ""},
    ]);
    github_schema::validate(
        STATUS_OPTIONS_UPDATE,
        &json!({"input": {"fieldId": "PVTSSF-status", "singleSelectOptions": options}}),
    )
    .expect("adding a Status option is a request GitHub runs");
    github_schema::validate(
        CREATE_FIELD,
        &json!({"input": {"projectId": "PVT-board", "dataType": "SINGLE_SELECT",
            "name": "Priority", "singleSelectOptions": [
                {"name": "Critical", "color": "GRAY", "description": ""},
                {"name": "Low", "color": "GRAY", "description": ""}]}}),
    )
    .expect("creating the Priority field is a request GitHub runs");
    github_schema::validate(
        CREATE_FIELD,
        &json!({"input": {"projectId": "PVT-board", "dataType": "TEXT", "name": "Origin"}}),
    )
    .expect("creating a metadata field is a request GitHub runs");

    assert_eq!(
        github_schema::validate(
            STATUS_OPTIONS_UPDATE,
            &json!({"input": {"projectId": "PVT-board", "fieldId": "PVTSSF-status",
                "singleSelectOptions": options}}),
        ),
        Err(vec![
            "Variable \"$input\" has invalid value: Expected value of type \
             \"UpdateProjectV2FieldInput\" not to include unknown field \"projectId\", found: { \
             fieldId: \"PVTSSF-status\", projectId: \"PVT-board\", singleSelectOptions: \
             [[Object], [Object]] }."
                .to_owned()
        ])
    );
}

/// Every validation rule of the GraphQL specification's §5, and the coercion of variables in
/// §6.1.2, each broken by one request that the doubles refuse, as GitHub refuses it: a body of
/// `errors` alone — no `data` — each error a `message` and, where the document has one, the
/// `locations` it stands at.
#[test]
fn the_schema_check_refuses_a_request_breaking_each_validation_rule() {
    use serde_json::{Value, json};

    // (the rule, the specification's section, the document, its variables, what is answered)
    let cases: &[(&str, &str, &str, Value, &[&str])] = &[
        (
            "document parses",
            "2",
            "query{viewer{login}",
            json!({}),
            &["Syntax Error: Expected Name, found <EOF>."],
        ),
        (
            "executable definitions",
            "5.1.1",
            "query{viewer{login}} type Extra{id:ID}",
            json!({}),
            &["The \"Extra\" definition is not executable."],
        ),
        (
            "operation name uniqueness",
            "5.2.1.1",
            "query a{viewer{login}} query a{viewer{name}}",
            json!({}),
            &["There can be only one operation named \"a\"."],
        ),
        (
            "lone anonymous operation",
            "5.2.2.1",
            "{viewer{login}} query b{viewer{name}}",
            json!({}),
            &["This anonymous operation must be the only defined operation."],
        ),
        // GitHub's schema defines no subscription root, so §5.2.3.1's single root field is
        // reached only as the operation type it cannot answer.
        (
            "operation type is defined",
            "5.2.3.1",
            "subscription{a:__typename b:__typename}",
            json!({}),
            &["The subscription operation is not supported by the schema."],
        ),
        (
            "field selections",
            "5.3.1",
            "query{viewer{loginName}}",
            json!({}),
            &["Cannot query field \"loginName\" on type \"User\". Did you mean \"login\"?"],
        ),
        (
            "field selections, introspection",
            "5.3.1",
            "query{__schema{queryType{nom}}}",
            json!({}),
            &["Cannot query field \"nom\" on type \"__Type\". Did you mean \"name\"?"],
        ),
        (
            "field selection merging, aliases",
            "5.3.2",
            "query{viewer{x:login x:name}}",
            json!({}),
            &[
                "Fields \"x\" conflict because \"login\" and \"name\" are different fields. Use different aliases on the fields to fetch both if this was intentional.",
            ],
        ),
        (
            "field selection merging, arguments",
            "5.3.2",
            "query{viewer{repositories(first:1){totalCount} repositories(first:2){totalCount}}}",
            json!({}),
            &[
                "Fields \"repositories\" conflict because they have differing arguments. Use different aliases on the fields to fetch both if this was intentional.",
            ],
        ),
        (
            "field selection merging, nested",
            "5.3.2",
            "query{viewer{r:repositories(first:1){nodes{a:name}} r:repositories(first:1){nodes{a:id}}}}",
            json!({}),
            &[
                "Fields \"r\" conflict because subfields \"nodes\" conflict because subfields \"a\" conflict because \"name\" and \"id\" are different fields. Use different aliases on the fields to fetch both if this was intentional.",
            ],
        ),
        (
            "field selection merging, exclusive fragments",
            "5.3.2",
            "query($id:ID!){node(id:$id){... on Issue{x:title} ... on PullRequest{x:number}}}",
            json!({"id": "I_1"}),
            &[
                "Fields \"x\" conflict because they return conflicting types \"String!\" and \"Int!\". Use different aliases on the fields to fetch both if this was intentional.",
            ],
        ),
        (
            "leaf field selections, scalar",
            "5.3.3",
            "query{viewer{login{name}}}",
            json!({}),
            &["Field \"login\" must not have a selection since type \"String!\" has no subfields."],
        ),
        (
            "leaf field selections, composite",
            "5.3.3",
            "query{viewer}",
            json!({}),
            &[
                "Field \"viewer\" of type \"User!\" must have a selection of subfields. Did you mean \"viewer { ... }\"?",
            ],
        ),
        (
            "argument names",
            "5.4.1",
            "query{viewer{login(first:1)}}",
            json!({}),
            &["Unknown argument \"first\" on field \"User.login\"."],
        ),
        (
            "argument uniqueness",
            "5.4.2",
            "query{node(id:\"A\",id:\"B\"){id}}",
            json!({}),
            &["There can be only one argument named \"id\"."],
        ),
        (
            "required arguments",
            "5.4.2.1",
            "query{node{id}}",
            json!({}),
            &["Argument \"Query.node(id:)\" of type \"ID!\" is required, but it was not provided."],
        ),
        (
            "fragment name uniqueness",
            "5.5.1.1",
            "query{viewer{...f}} fragment f on User{name} fragment f on User{login}",
            json!({}),
            &["There can be only one fragment named \"f\"."],
        ),
        (
            "fragment spread type existence",
            "5.5.1.2",
            "query($id:ID!){node(id:$id){... on Labelled{id}}}",
            json!({"id": "I_1"}),
            &[
                "Unknown type \"Labelled\". Did you mean \"Label\", \"LabelEdge\", \"Labelable\", or \"LabelOrder\"?",
            ],
        ),
        (
            "fragments on composite types",
            "5.5.1.3",
            "query{viewer{...f}} fragment f on String{__typename}",
            json!({}),
            &["Fragment \"f\" cannot condition on non composite type \"String\"."],
        ),
        (
            "fragments must be used",
            "5.5.1.4",
            "query{viewer{login}} fragment f on User{name}",
            json!({}),
            &["Fragment \"f\" is never used."],
        ),
        (
            "fragment spread target defined",
            "5.5.2.1",
            "query{viewer{...g}}",
            json!({}),
            &["Unknown fragment \"g\"."],
        ),
        (
            "fragment spreads must not form cycles",
            "5.5.2.2",
            "query{viewer{...a}} fragment a on User{login ...b} fragment b on User{...a}",
            json!({}),
            &["Cannot spread fragment \"a\" within itself via \"b\"."],
        ),
        (
            "fragment spread is possible",
            "5.5.2.3",
            "query{viewer{... on Issue{title}}}",
            json!({}),
            &[
                "Fragment cannot be spread here as objects of type \"User\" can never be of type \"Issue\".",
            ],
        ),
        (
            "values of correct type",
            "5.6.1",
            "query{viewer{repositories(first:3000000000){totalCount}}}",
            json!({}),
            &["Int cannot represent non 32-bit signed integer value: 3000000000"],
        ),
        (
            "input object field names",
            "5.6.2",
            "mutation{updateProjectV2Field(input:{fieldId:\"F\",projectId:\"P\"}){clientMutationId}}",
            json!({}),
            &[
                "Expected value of type \"UpdateProjectV2FieldInput\" not to include unknown field \"projectId\", found: { fieldId: \"F\", projectId: \"P\" }.",
            ],
        ),
        (
            "input object field uniqueness",
            "5.6.3",
            "mutation{updateProjectV2Field(input:{fieldId:\"F\",fieldId:\"G\"}){clientMutationId}}",
            json!({}),
            &["There can be only one input field named \"fieldId\"."],
        ),
        (
            "input object required fields",
            "5.6.4",
            "mutation{updateProjectV2Field(input:{}){clientMutationId}}",
            json!({}),
            &[
                "Expected value of type \"UpdateProjectV2FieldInput\" to include required field \"fieldId\", found: {  }.",
            ],
        ),
        (
            "directives are defined",
            "5.7.1",
            "query{viewer{login @nowhere}}",
            json!({}),
            &["Unknown directive \"@nowhere\"."],
        ),
        (
            "directives are in valid locations",
            "5.7.2",
            "query @include(if:true){viewer{login}}",
            json!({}),
            &["Directive \"@include\" may not be used on QUERY."],
        ),
        (
            "directives are unique per location",
            "5.7.3",
            "query{viewer{login @skip(if:false) @skip(if:false)}}",
            json!({}),
            &["The directive \"@skip\" can only be used once at this location."],
        ),
        (
            "directive required arguments",
            "5.4.2.1",
            "query{viewer{login @skip}}",
            json!({}),
            &["Argument \"@skip(if:)\" of type \"Boolean!\" is required, but it was not provided."],
        ),
        (
            "variable uniqueness",
            "5.8.1",
            "query($id:ID!,$id:ID!){node(id:$id){id}}",
            json!({"id": "I_1"}),
            &["There can be only one variable named \"$id\"."],
        ),
        (
            "variables are input types",
            "5.8.2",
            "query($u:User){viewer{login}}",
            json!({}),
            &[
                "Variable \"$u\" is never used.",
                "Variable \"$u\" cannot be non-input type \"User\".",
            ],
        ),
        (
            "all variable uses defined",
            "5.8.3",
            "query{node(id:$id){id}}",
            json!({}),
            &["Variable \"$id\" is not defined."],
        ),
        (
            "all variable uses defined, through a fragment",
            "5.8.3",
            "query{viewer{...f}} fragment f on User{repositories(first:$n){totalCount}}",
            json!({}),
            &["Variable \"$n\" is not defined."],
        ),
        (
            "all variables used",
            "5.8.4",
            "query($id:ID!,$unused:Int){node(id:$id){id}}",
            json!({"id": "I_1"}),
            &["Variable \"$unused\" is never used."],
        ),
        (
            "all variable usages are allowed, nullability",
            "5.8.5",
            "query($id:ID){node(id:$id){id}}",
            json!({"id": "I_1"}),
            &["Variable \"$id\" of type \"ID\" used in position expecting type \"ID!\"."],
        ),
        (
            "all variable usages are allowed, a null default is no licence",
            "5.8.5",
            "query($id:ID = null){node(id:$id){id}}",
            json!({}),
            &["Variable \"$id\" of type \"ID\" used in position expecting type \"ID!\"."],
        ),
        (
            "all variable usages are allowed, list",
            "5.8.5",
            "query($ids:[ID!]!){node(id:$ids){id}}",
            json!({"ids": ["I_1"]}),
            &["Variable \"$ids\" of type \"[ID!]!\" used in position expecting type \"ID!\"."],
        ),
        (
            "a default is a value of its type",
            "5.6.1",
            "query($first:Int = \"ten\"){viewer{repositories(first:$first){totalCount}}}",
            json!({}),
            &["Int cannot represent non-integer value: \"ten\""],
        ),
        (
            "a default is a constant",
            "2.10",
            "query($a:Int,$b:Int=$a){viewer{repositories(first:$b){totalCount}}}",
            json!({}),
            &["Syntax Error: Unexpected variable \"$a\" in constant value."],
        ),
        // §6.1.2, CoerceVariableValues, over each variable's JSON value.
        (
            "variables are an object",
            "7.1.1",
            "query{viewer{login}}",
            json!(["not", "an", "object"]),
            &["Variables must be provided as an Object where each property is a variable value."],
        ),
        (
            "a required variable is given",
            "6.1.2",
            "mutation($input:UpdateProjectV2FieldInput!){updateProjectV2Field(input:$input){clientMutationId}}",
            json!({}),
            &[
                "Variable \"$input\" has invalid value: Expected a value of non-null type \"UpdateProjectV2FieldInput!\" to be provided.",
            ],
        ),
        (
            "a variable coerces to its scalar",
            "6.1.2",
            "query($first:Int){viewer{repositories(first:$first){totalCount}}}",
            json!({"first": "ten"}),
            &[
                "Variable \"$first\" has invalid value: Int cannot represent non-integer value: \"ten\"",
            ],
        ),
        (
            "a variable coerces to its enum",
            "6.1.2",
            "mutation($input:CreateProjectV2FieldInput!){createProjectV2Field(input:$input){clientMutationId}}",
            json!({"input": {"projectId": "PVT_1", "dataType": "PRIORITY", "name": "Priority"}}),
            &[
                "Variable \"$input\" has invalid value at .dataType: Value \"PRIORITY\" does not exist in \"ProjectV2CustomFieldType\" enum.",
            ],
        ),
        (
            "a variable names no input field its type lacks",
            "6.1.2",
            "mutation($input:DeleteIssueInput!){deleteIssue(input:$input){clientMutationId}}",
            json!({"input": {"issueId": "I_1", "projectId": "PVT_1"}}),
            &[
                "Variable \"$input\" has invalid value: Expected value of type \"DeleteIssueInput\" not to include unknown field \"projectId\", found: { issueId: \"I_1\", projectId: \"PVT_1\" }.",
            ],
        ),
        (
            "a variable gives every required input field",
            "6.1.2",
            "mutation($input:CreateProjectV2FieldInput!){createProjectV2Field(input:$input){clientMutationId}}",
            json!({"input": {"projectId": "PVT_1", "dataType": "SINGLE_SELECT",
                "name": "Priority", "singleSelectOptions": [{"name": "High", "color": "GRAY"}]}}),
            &[
                "Variable \"$input\" has invalid value at .singleSelectOptions[0]: Expected value of type \"ProjectV2SingleSelectFieldOptionInput\" to include required field \"description\", found: { color: \"GRAY\", name: \"High\" }.",
            ],
        ),
    ];
    let mut wrong = Vec::new();
    for (rule, section, document, variables, expected) in cases {
        let answered = github_schema::refuse(&json!({"query": document, "variables": variables}));
        let Some(answered) = answered else {
            wrong.push(format!("§{section} {rule}: {document} was not refused"));
            continue;
        };
        let errors = answered["errors"].as_array().cloned().unwrap_or_default();
        let messages = errors
            .iter()
            .map(|error| error["message"].as_str().unwrap_or_default())
            .collect::<Vec<_>>();
        if messages != *expected {
            wrong.push(format!(
                "§{section} {rule}: {document} answered {messages:?}"
            ));
        }
        let shaped = answered.as_object().is_some_and(|body| body.len() == 1)
            && errors.iter().all(|error| {
                error["message"].is_string()
                    && error.as_object().is_some_and(|fields| {
                        fields
                            .keys()
                            .all(|key| key == "message" || key == "locations")
                    })
                    && error.get("locations").is_none_or(|locations| {
                        locations.as_array().is_some_and(|all| {
                            all.iter()
                                .all(|at| at["line"].is_u64() && at["column"].is_u64())
                        })
                    })
            });
        if !shaped {
            wrong.push(format!(
                "§{section} {rule}: {answered} is not GraphQL's error shape"
            ));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));

    // A refusal of a rule broken at a place in the document says where.
    assert_eq!(
        github_schema::refuse(&json!({"query": "query{\n  viewer{loginName}}"})),
        Some(
            json!({"errors": [{"message": "Cannot query field \"loginName\" on type \"User\". \
                Did you mean \"login\"?",
            "locations": [{"line": 2, "column": 10}]}]})
        )
    );
    // And what breaks none of them runs: a nullable variable with a non-null default is allowed
    // where a non-null one is expected, a list position takes a single value, and a named
    // operation is chosen by `operationName`.
    for (document, variables, operation) in [
        ("query($id:ID = \"I_1\"){node(id:$id){id}}", json!({}), None),
        (
            "query($ids:[ID!]!){nodes(ids:$ids){id}}",
            json!({"ids": "I_1"}),
            None,
        ),
        (
            "query a{viewer{login}} query b{viewer{name}}",
            json!({}),
            Some("b"),
        ),
    ] {
        assert_eq!(
            github_schema::refuse(
                &json!({"query": document, "variables": variables, "operationName": operation})
            ),
            None,
            "{document}"
        );
    }
    assert_eq!(
        github_schema::refuse(&json!({"query": "query a{viewer{login}}", "operationName": "c"})),
        Some(json!({"errors": [{"message": "Unknown operation named \"c\"."}]}))
    );
    assert_eq!(
        github_schema::refuse(&json!({"query": "query a{viewer{login}}", "operationName": 1})),
        Some(json!({"errors": [{"message": "operationName must be a string, not 1"}]}))
    );
}

/// A malformed or adversarial request is refused, never answered by a crash or an exhausted
/// stack: bytes that are not GraphQL, nesting far past any real document's, a fragment cycle
/// two thousand spreads long, a wide selection and deeply nested variables.
#[test]
fn a_malformed_or_adversarial_request_is_refused_rather_than_crashing() {
    use serde_json::json;

    let refused = |document: &str, variables: serde_json::Value| {
        github_schema::refuse(&json!({"query": document, "variables": variables}))
            .unwrap_or_else(|| panic!("{} was not refused", &document[..document.len().min(80)]))
    };
    refused("", json!({}));
    refused("\u{0}\u{1}garbage{{{", json!({}));
    refused("query{viewer{login}} }}}}", json!({}));
    let deep = |open: &str, close: &str, depth: usize| (open.repeat(depth), close.repeat(depth));
    let (open, close) = deep("viewer{", "}", 50_000);
    assert_eq!(
        refused(&format!("query{{{open}login{close}}}"), json!({})),
        json!({"errors": [{"message": "The request nests too deeply to validate."}]})
    );
    // A deep document that breaks no rule is validated, not refused for its depth.
    let (open, close) = deep("ofType{", "}", 190);
    assert_eq!(
        github_schema::refuse(
            &json!({"query": format!("query{{__schema{{types{{{open}name{close}}}}}}}")})
        ),
        None
    );
    let (open, close) = deep("[", "]", 50_000);
    assert_eq!(
        refused(
            &format!("query{{node(id:{open}1{close}){{id}}}}"),
            json!({}),
        ),
        json!({"errors": [{"message": "The request nests too deeply to validate."}]})
    );
    let (open, close) = deep("{a:", "}", 50_000);
    assert_eq!(
        refused(
            &format!("mutation{{updateProjectV2Field(input:{open}1{close}){{clientMutationId}}}}"),
            json!({}),
        ),
        json!({"errors": [{"message": "The request nests too deeply to validate."}]})
    );
    let chain = (0..2_000)
        .map(|at| format!("fragment f{at} on User{{...f{}}} ", at + 1))
        .collect::<String>();
    let cycle = refused(
        &format!("query{{viewer{{...f0}}}} {chain} fragment f2000 on User{{...f0}}"),
        json!({}),
    );
    assert!(
        cycle["errors"][0]["message"].as_str().is_some_and(
            |message| message.starts_with("Cannot spread fragment \"f0\" within itself via")
        ),
        "{cycle}"
    );
    let mut nested = json!("I_1");
    for _ in 0..300 {
        nested = json!([nested]);
    }
    let answered = refused("query($id:ID!){node(id:$id){id}}", json!({"id": nested}));
    assert_eq!(
        answered,
        json!({"errors": [{"message": "Variables nest deeper than 200 levels"}]})
    );
    // A wide document that breaks nothing is not refused for its width.
    let wide = (0..3_000)
        .map(|at| format!("a{at}:login "))
        .collect::<String>();
    assert_eq!(
        github_schema::refuse(&json!({"query": format!("query{{viewer{{{wide}}}}}")})),
        None
    );
}

/// The schema check stops, rather than answering requests, against a schema breaking any
/// type-system rule but the one GitHub's published file breaks — so a copy that is not the
/// schema GitHub serves cannot pass for it — and starts against one breaking that rule alone.
#[test]
fn the_schema_check_refuses_a_schema_breaking_any_rule_but_the_tolerated_one() {
    use serde_json::{Value, json};

    let first_answer = |sdl: &str| -> Value {
        let file = tempfile::NamedTempFile::new().expect("a scratch schema file");
        std::fs::write(file.path(), sdl).expect("the scratch schema is written");
        let output = std::process::Command::new("bun")
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/github_schema/validate.mjs"
            ))
            .arg(file.path())
            .env("BUN_RUNTIME_TRANSPILER_CACHE_PATH", "0")
            .stdin(std::process::Stdio::null())
            .output()
            .expect("bun runs the schema check");
        let stdout = String::from_utf8_lossy(&output.stdout);
        serde_json::from_str(stdout.lines().next().unwrap_or_default())
            .unwrap_or_else(|_| panic!("the schema check answered {stdout:?}"))
    };
    let tolerated = "interface Node { id: ID! } \
                     type Card implements Node { id: ID! @deprecated(reason: \"gone\") } \
                     type Query { node: Node card: Card }";
    assert_eq!(first_answer(tolerated)["ready"], json!(true));

    let invalid = first_answer("type Empty type Query { empty: Empty }");
    assert!(
        invalid["internal"]
            .as_str()
            .is_some_and(|message| message.contains("is invalid")
                && message.contains("Type Empty must define one or more fields.")),
        "{invalid}"
    );
}

/// graphql-js is npm's own published tarball, and a copy of it that does not hash to the
/// integrity npm records is refused before anything is unpacked or imported from it — so the
/// validation every GitHub double runs cannot quietly become some other code. The check is run
/// from a scratch directory holding `validate.mjs` beside a tarball one byte off npm's.
#[test]
fn the_schema_check_refuses_a_graphql_tarball_npm_did_not_publish() {
    let tests = concat!(env!("CARGO_MANIFEST_DIR"), "/tests");
    let scratch = tempfile::tempdir().expect("a scratch directory");
    std::fs::copy(
        format!("{tests}/github_schema/validate.mjs"),
        scratch.path().join("validate.mjs"),
    )
    .expect("the schema check is copied");
    let mut tarball = std::fs::read(format!("{tests}/github_schema/graphql-17.0.2.tgz"))
        .expect("the pinned graphql-js tarball is readable");
    let last = tarball.len() - 1;
    tarball[last] ^= 0xff;
    std::fs::write(scratch.path().join("graphql-17.0.2.tgz"), tarball)
        .expect("the altered tarball is written");

    let output = std::process::Command::new("bun")
        .arg(scratch.path().join("validate.mjs"))
        .arg(format!("{tests}/fixtures/github-schema.docs.graphql"))
        .env("BUN_RUNTIME_TRANSPILER_CACHE_PATH", "0")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("bun runs the schema check");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let answer: serde_json::Value = serde_json::from_str(stdout.lines().next().unwrap_or_default())
        .unwrap_or_else(|_| panic!("the schema check answered {stdout:?}"));
    assert!(!output.status.success(), "{answer}");
    assert!(
        answer["internal"].as_str().is_some_and(|message| {
            message.contains("graphql-17.0.2.tgz has integrity sha512-")
                && message.contains("not the sha512-FRWbddMx")
        }),
        "{answer}"
    );
    assert!(
        !scratch.path().join("node_modules").exists(),
        "nothing is unpacked from a tarball npm did not publish"
    );
}
