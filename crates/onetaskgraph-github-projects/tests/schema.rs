//! Deterministic drift check for the exact GitHub GraphQL production documents.
//!
//! The pinned subset defines no `updateProjectV2`, no `ProjectV2.shortDescription` and no
//! `ProjectV2.readme`, so a document that reached for one would fail here rather than
//! rename a user's board; it defines no `updateProjectV2Field` either, so nothing can
//! overwrite a Status field's option set.

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
        (
            graphql::SUB_ISSUES,
            Some("/data/node"),
            Some(include_str!("fixtures/sub-issues.json")),
        ),
        (graphql::REPOSITORY, None, None),
        (
            graphql::ISSUE_DEPENDENCIES,
            Some("/data/node"),
            Some(include_str!("fixtures/dependencies.json")),
        ),
        (graphql::CREATE_ISSUE, None, None),
        (graphql::ADD_TO_BOARD, None, None),
        (graphql::UPDATE_ISSUE, None, None),
        (graphql::UPDATE_DRAFT, None, None),
        (graphql::UPDATE_FIELD, None, None),
        (graphql::ADD_SUB_ISSUE, None, None),
        (graphql::REMOVE_SUB_ISSUE, None, None),
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

/// Nothing this crate can do renames a board or rewrites a Status field's options.
///
/// The pinned schema already refuses a *document* naming either, and this reads the whole
/// crate rather than the documents alone: the criterion is that no code path invokes
/// `updateProjectV2Field` with `singleSelectOptions`, and a source file is where a path
/// would have to be spelled. GitHub documents that input as overwriting a field's existing
/// options, so no addition is additive and a mistake destroys every item's status; a
/// status this board cannot represent is a refusal instead.
#[test]
fn no_source_path_writes_the_board_itself_or_a_status_fields_option_set() {
    /// Everything the file says that is not a comment about what it does not do.
    fn code(text: &str, comment: &str) -> String {
        text.lines()
            .filter(|line| !line.trim_start().starts_with(comment))
            .collect::<Vec<_>>()
            .join("\n")
    }
    const FORBIDDEN: [&str; 5] = [
        "updateProjectV2Field",
        "singleSelectOptions",
        "updateProjectV2(",
        "shortDescription",
        "readme",
    ];
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
    // GitHub's refusal wordings and its published ceiling on content creation are
    // GitHub's contract, not this source's, and this source restates both: the wordings
    // are what `Limiter::classify` matches a refusal on, and the per-minute ceiling is
    // what `MIN_MUTATION_INTERVAL_MS` is derived from. Restating an external contract
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
    // And the derivation itself, because the pin is only worth having if the value this
    // source actually paces at moves with it.
    assert_eq!(
        onetaskgraph_github_projects::MIN_MUTATION_INTERVAL_MS,
        60_000 / onetaskgraph_github_projects::CONTENT_CREATION_PER_MINUTE,
        "the shipped interval stopped being the fastest rate the pinned per-minute \
         ceiling allows"
    );
}
