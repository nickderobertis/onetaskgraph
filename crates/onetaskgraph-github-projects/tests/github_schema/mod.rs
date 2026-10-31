//! GitHub's own validation of a GraphQL request, against the complete schema GitHub publishes.
//!
//! A test double that answers whatever it is sent proves nothing about whether GitHub would
//! answer it: the guarded field setup of 0.12.13 sent `projectId` inside
//! `UpdateProjectV2FieldInput`, the loopback board accepted it, and GitHub refused it before
//! anything ran. So every GitHub double here puts each request to [`validate`] first and
//! answers a refusal the way GitHub does — HTTP 200 with an `errors` array and no `data`.
//!
//! The schema is `fixtures/github-schema.docs.graphql`, GitHub's published schema file whole
//! (its provenance is in `fixtures/README.md`), never a reduction of it: a reduction is a second
//! statement of GitHub's contract written by the same hand as the request, which is how the
//! defect above passed the pinned-subset drift check too.
//!
//! What it checks is what GitHub's executor refuses a document for before it runs: that it
//! parses; that every selected field exists on its type and every argument on its field, that
//! every required argument is given, that a leaf has no selection and a composite one; that
//! every fragment's type exists and can apply where it is spread; that every variable used is
//! declared and every declared one used, each at an input type the schema defines; and that
//! each variable's value — and each literal argument — coerces to its declared type, with no
//! input field the type does not define and every required one present. Its messages follow
//! GitHub's own wording, so a failure here reads like the one a live run would print.
//!
//! Included by path from the loopback board in `onetaskgraph-e2e-support` as well as from this
//! crate's own tests, so both doubles hold to one implementation.

use graphql_parser::{query, schema};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

/// GitHub's published GraphQL schema, whole.
pub const SCHEMA: &str = include_str!("../fixtures/github-schema.docs.graphql");

type Definition = schema::TypeDefinition<'static, String>;

struct Index {
    types: HashMap<&'static str, &'static Definition>,
    /// Every interface and union, with the object types it can be.
    possible: HashMap<&'static str, HashSet<&'static str>>,
}

fn index() -> &'static Index {
    static INDEX: OnceLock<Index> = OnceLock::new();
    INDEX.get_or_init(|| {
        static DOCUMENT: OnceLock<schema::Document<'static, String>> = OnceLock::new();
        let document = DOCUMENT.get_or_init(|| {
            schema::parse_schema::<String>(SCHEMA).expect("GitHub's published schema parses")
        });
        let mut types = HashMap::new();
        let mut possible: HashMap<&'static str, HashSet<&'static str>> = HashMap::new();
        for definition in &document.definitions {
            let schema::Definition::TypeDefinition(kind) = definition else {
                continue;
            };
            let name = type_name(kind);
            types.insert(name, kind);
            match kind {
                schema::TypeDefinition::Object(object) => {
                    for interface in &object.implements_interfaces {
                        possible.entry(interface.as_str()).or_default().insert(name);
                    }
                }
                schema::TypeDefinition::Union(union) => {
                    possible
                        .entry(name)
                        .or_default()
                        .extend(union.types.iter().map(String::as_str));
                }
                _ => {}
            }
        }
        Index { types, possible }
    })
}

fn type_name(kind: &'static Definition) -> &'static str {
    match kind {
        schema::TypeDefinition::Scalar(value) => &value.name,
        schema::TypeDefinition::Object(value) => &value.name,
        schema::TypeDefinition::Interface(value) => &value.name,
        schema::TypeDefinition::Union(value) => &value.name,
        schema::TypeDefinition::Enum(value) => &value.name,
        schema::TypeDefinition::InputObject(value) => &value.name,
    }
}

const BUILT_IN_SCALARS: [&str; 5] = ["Int", "Float", "String", "Boolean", "ID"];

fn base<'k>(kind: &'k query::Type<'_, String>) -> &'k str {
    match kind {
        query::Type::NamedType(name) => name,
        query::Type::ListType(inner) | query::Type::NonNullType(inner) => base(inner),
    }
}

/// A type as GraphQL spells it, `[ID!]!` and the like.
fn spelled(kind: &query::Type<'_, String>) -> String {
    match kind {
        query::Type::NamedType(name) => name.clone(),
        query::Type::ListType(inner) => format!("[{}]", spelled(inner)),
        query::Type::NonNullType(inner) => format!("{}!", spelled(inner)),
    }
}

/// The object types a named type can be at run time.
fn possible_types(name: &str) -> HashSet<&'static str> {
    let index = index();
    match index.types.get(name) {
        Some(schema::TypeDefinition::Object(object)) => HashSet::from([object.name.as_str()]),
        Some(_) => index.possible.get(name).cloned().unwrap_or_default(),
        None => HashSet::new(),
    }
}

fn fields_of(name: &str) -> Option<&'static [schema::Field<'static, String>]> {
    match index().types.get(name)? {
        schema::TypeDefinition::Object(object) => Some(&object.fields),
        schema::TypeDefinition::Interface(interface) => Some(&interface.fields),
        _ => None,
    }
}

fn is_composite(name: &str) -> bool {
    matches!(
        index().types.get(name),
        Some(
            schema::TypeDefinition::Object(_)
                | schema::TypeDefinition::Interface(_)
                | schema::TypeDefinition::Union(_)
        )
    )
}

fn is_input(name: &str) -> bool {
    BUILT_IN_SCALARS.contains(&name)
        || matches!(
            index().types.get(name),
            Some(
                schema::TypeDefinition::Scalar(_)
                    | schema::TypeDefinition::Enum(_)
                    | schema::TypeDefinition::InputObject(_)
            )
        )
}

struct Walk<'q> {
    fragments: HashMap<&'q str, &'q query::FragmentDefinition<'q, String>>,
    used: HashSet<String>,
    errors: Vec<String>,
    /// Fragments already walked from the current operation, so a cycle cannot recurse forever.
    entered: HashSet<String>,
}

/// Whether GitHub would accept `document` sent with `variables`; `Err` carries each message
/// GitHub would answer it with, in order.
pub fn validate(document: &str, variables: &Value) -> Result<(), Vec<String>> {
    check(document, Some(variables))
}

/// [`validate`] of the document alone: every rule but the coercion of the variables' values,
/// for a document whose values a caller supplies later.
pub fn validate_document(document: &str) -> Result<(), Vec<String>> {
    check(document, None)
}

fn check(document: &str, variables: Option<&Value>) -> Result<(), Vec<String>> {
    let parsed = query::parse_query::<String>(document)
        .map_err(|error| vec![format!("Parse error on document: {error}")])?;
    let mut walk = Walk {
        fragments: HashMap::new(),
        used: HashSet::new(),
        errors: Vec::new(),
        entered: HashSet::new(),
    };
    for definition in &parsed.definitions {
        if let query::Definition::Fragment(fragment) = definition {
            walk.fragments.insert(fragment.name.as_str(), fragment);
        }
    }
    for definition in &parsed.definitions {
        let query::Definition::Operation(operation) = definition else {
            continue;
        };
        let (root, name, declared, selection) = match operation {
            query::OperationDefinition::SelectionSet(selection) => {
                ("Query", None, &[][..], selection)
            }
            query::OperationDefinition::Query(value) => (
                "Query",
                value.name.as_deref(),
                &value.variable_definitions[..],
                &value.selection_set,
            ),
            query::OperationDefinition::Mutation(value) => (
                "Mutation",
                value.name.as_deref(),
                &value.variable_definitions[..],
                &value.selection_set,
            ),
            query::OperationDefinition::Subscription(value) => (
                "Subscription",
                value.name.as_deref(),
                &value.variable_definitions[..],
                &value.selection_set,
            ),
        };
        let operation_name = name.map_or_else(
            || format!("anonymous {}", root.to_lowercase()),
            |name| format!("{} {name}", root.to_lowercase()),
        );
        let declared_types: HashMap<&str, &query::Type<'_, String>> = declared
            .iter()
            .map(|definition| (definition.name.as_str(), &definition.var_type))
            .collect();
        for definition in declared {
            let named = base(&definition.var_type);
            if !is_input(named) {
                walk.errors.push(format!(
                    "{named} isn't a defined input type (on ${})",
                    definition.name
                ));
                continue;
            }
            let Some(variables) = variables else {
                continue;
            };
            let given = variables.get(definition.name.as_str());
            match given {
                None if definition.default_value.is_some() => {}
                given => {
                    let given = given.unwrap_or(&Value::Null);
                    if let Err(problem) = coerce(&definition.var_type, given) {
                        walk.errors.push(format!(
                            "Variable ${} of type {} was provided invalid value{problem}",
                            definition.name,
                            spelled(&definition.var_type)
                        ));
                    }
                }
            }
        }
        walk.used.clear();
        walk.entered.clear();
        walk.selection(root, selection, &declared_types);
        for definition in declared {
            if !walk.used.contains(definition.name.as_str()) {
                walk.errors.push(format!(
                    "Variable ${} is declared by {operation_name} but not used",
                    definition.name
                ));
            }
        }
        let mut undeclared = walk
            .used
            .iter()
            .filter(|name| !declared_types.contains_key(name.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        undeclared.sort();
        for name in undeclared {
            walk.errors.push(format!(
                "Variable ${name} is used by {operation_name} but not declared"
            ));
        }
    }
    if walk.errors.is_empty() {
        Ok(())
    } else {
        Err(walk.errors)
    }
}

/// The body GitHub answers a refused document with: HTTP 200, `errors`, no `data`.
#[must_use]
pub fn refusal(errors: &[String]) -> Value {
    Value::Object(
        [(
            "errors".to_owned(),
            Value::Array(
                errors
                    .iter()
                    .map(|message| serde_json::json!({"message": message}))
                    .collect(),
            ),
        )]
        .into_iter()
        .collect(),
    )
}

/// `body`, a GraphQL request's JSON, put to [`validate`]: `Some` refusal body when GitHub
/// would refuse it, `None` when it would run it or when `body` is not a GraphQL request.
#[must_use]
pub fn refuse(body: &Value) -> Option<Value> {
    let document = body.get("query")?.as_str()?;
    let empty = Value::Object(serde_json::Map::new());
    let variables = body
        .get("variables")
        .filter(|value| !value.is_null())
        .unwrap_or(&empty);
    validate(document, variables)
        .err()
        .map(|errors| refusal(&errors))
}

impl<'q> Walk<'q> {
    fn selection(
        &mut self,
        parent: &str,
        selection: &'q query::SelectionSet<'q, String>,
        declared: &HashMap<&str, &query::Type<'_, String>>,
    ) {
        for item in &selection.items {
            let directives = match item {
                query::Selection::Field(field) => &field.directives,
                query::Selection::InlineFragment(fragment) => &fragment.directives,
                query::Selection::FragmentSpread(spread) => &spread.directives,
            };
            for directive in directives {
                for (_, value) in &directive.arguments {
                    collect_variables(value, &mut self.used);
                }
            }
            match item {
                query::Selection::Field(field) => self.field(parent, field, declared),
                query::Selection::InlineFragment(fragment) => {
                    let condition = match &fragment.type_condition {
                        Some(query::TypeCondition::On(name)) => name.as_str(),
                        None => parent,
                    };
                    if self.applies(parent, condition) {
                        self.selection(condition, &fragment.selection_set, declared);
                    }
                }
                query::Selection::FragmentSpread(spread) => {
                    let Some(fragment) = self.fragments.get(spread.fragment_name.as_str()).copied()
                    else {
                        self.errors.push(format!(
                            "Fragment {} was used, but not defined",
                            spread.fragment_name
                        ));
                        continue;
                    };
                    let query::TypeCondition::On(condition) = &fragment.type_condition;
                    if self.applies(parent, condition) && self.entered.insert(fragment.name.clone())
                    {
                        self.selection(condition, &fragment.selection_set, declared);
                    }
                }
            }
        }
    }

    /// Whether a fragment on `condition` can apply inside a selection on `parent`.
    fn applies(&mut self, parent: &str, condition: &str) -> bool {
        if !is_composite(condition) {
            self.errors.push(format!(
                "No such type {condition}, so it can't be a fragment condition"
            ));
            return false;
        }
        if possible_types(parent).is_disjoint(&possible_types(condition)) {
            self.errors.push(format!(
                "Fragment on {condition} can't be spread inside {parent}"
            ));
            return false;
        }
        true
    }

    fn field(
        &mut self,
        parent: &str,
        field: &'q query::Field<'q, String>,
        declared: &HashMap<&str, &query::Type<'_, String>>,
    ) {
        let name = field.name.as_str();
        for (_, value) in &field.arguments {
            collect_variables(value, &mut self.used);
        }
        if name == "__typename" {
            if !field.selection_set.items.is_empty() {
                self.errors.push(format!(
                    "Selections can't be made on scalars (field '__typename' returns String but \
                     has selections) on {parent}"
                ));
            }
            return;
        }
        if parent == "Query" && (name == "__schema" || name == "__type") {
            return;
        }
        let Some(definition) =
            fields_of(parent).and_then(|fields| fields.iter().find(|f| f.name == name))
        else {
            self.errors
                .push(format!("Field '{name}' doesn't exist on type '{parent}'"));
            return;
        };
        for (argument, value) in &field.arguments {
            match definition.arguments.iter().find(|a| &a.name == argument) {
                None => self.errors.push(format!(
                    "Field '{name}' doesn't accept argument '{argument}'"
                )),
                Some(expected) => {
                    if let Err(problem) = literal(&expected.value_type, value, declared) {
                        self.errors.push(format!(
                            "Argument '{argument}' on Field '{name}' has an invalid value{problem}"
                        ));
                    }
                }
            }
        }
        let missing = definition
            .arguments
            .iter()
            .filter(|argument| {
                matches!(argument.value_type, schema::Type::NonNullType(_))
                    && argument.default_value.is_none()
                    && !field
                        .arguments
                        .iter()
                        .any(|(given, _)| given == &argument.name)
            })
            .map(|argument| argument.name.as_str())
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            self.errors.push(format!(
                "Field '{name}' is missing required arguments: {}",
                missing.join(", ")
            ));
        }
        let returns = base(&definition.field_type);
        if is_composite(returns) {
            if field.selection_set.items.is_empty() {
                self.errors.push(format!(
                    "Field must have selections (field '{name}' returns {returns} but has no \
                     selections. Did you mean '{name} {{ ... }}'?)"
                ));
            } else {
                self.selection(returns, &field.selection_set, declared);
            }
        } else if !field.selection_set.items.is_empty() {
            self.errors.push(format!(
                "Selections can't be made on scalars (field '{name}' returns {returns} but has \
                 selections)"
            ));
        }
    }
}

fn collect_variables(value: &query::Value<'_, String>, used: &mut HashSet<String>) {
    match value {
        query::Value::Variable(name) => {
            used.insert(name.clone());
        }
        query::Value::List(values) => {
            for value in values {
                collect_variables(value, used);
            }
        }
        query::Value::Object(fields) => {
            for value in fields.values() {
                collect_variables(value, used);
            }
        }
        _ => {}
    }
}

/// Whether a JSON variable value coerces to `kind`; `Err` names where it does not, as
/// GitHub's ` for <path> (<reason>)` suffix.
fn coerce(kind: &query::Type<'_, String>, value: &Value) -> Result<(), String> {
    match kind {
        query::Type::NonNullType(inner) => {
            if value.is_null() {
                Err(" (Expected value to not be null)".to_owned())
            } else {
                coerce(inner, value)
            }
        }
        _ if value.is_null() => Ok(()),
        query::Type::ListType(inner) => match value {
            Value::Array(items) => items.iter().try_for_each(|item| coerce(inner, item)),
            single => coerce(inner, single),
        },
        query::Type::NamedType(name) => coerce_named(name, value),
    }
}

fn coerce_named(name: &str, value: &Value) -> Result<(), String> {
    let wrong = || Err(format!(" (Could not coerce value {value} to {name})"));
    match name {
        "Int" => match value.as_i64() {
            Some(number) if i32::try_from(number).is_ok() => Ok(()),
            _ => wrong(),
        },
        "Float" => {
            if value.is_number() {
                Ok(())
            } else {
                wrong()
            }
        }
        "String" => {
            if value.is_string() {
                Ok(())
            } else {
                wrong()
            }
        }
        "Boolean" => {
            if value.is_boolean() {
                Ok(())
            } else {
                wrong()
            }
        }
        "ID" => {
            if value.is_string() || value.is_i64() || value.is_u64() {
                Ok(())
            } else {
                wrong()
            }
        }
        _ => match index().types.get(name) {
            Some(schema::TypeDefinition::Enum(members)) => {
                if value
                    .as_str()
                    .is_some_and(|given| members.values.iter().any(|m| m.name == given))
                {
                    Ok(())
                } else {
                    Err(format!(" (Expected {value} to be one of: {})", {
                        members
                            .values
                            .iter()
                            .map(|m| m.name.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    }))
                }
            }
            Some(schema::TypeDefinition::InputObject(input)) => {
                let Value::Object(given) = value else {
                    return wrong();
                };
                for key in given.keys() {
                    if !input.fields.iter().any(|field| &field.name == key) {
                        return Err(format!(
                            " for {key} (Field is not defined on {})",
                            input.name
                        ));
                    }
                }
                for field in &input.fields {
                    match given.get(field.name.as_str()) {
                        None if field.default_value.is_some() => {}
                        None if matches!(field.value_type, schema::Type::NonNullType(_)) => {
                            return Err(format!(
                                " for {} (Expected value to not be null)",
                                field.name
                            ));
                        }
                        None => {}
                        Some(inner) => coerce(&field.value_type, inner).map_err(|problem| {
                            if let Some(inner) = problem.strip_prefix(" for ") {
                                format!(" for {}.{inner}", field.name)
                            } else {
                                format!(" for {}{problem}", field.name)
                            }
                        })?,
                    }
                }
                Ok(())
            }
            // GitHub's custom scalars (`DateTime`, `URI`, `Base64String`…) take their
            // serialized spelling; a structured value is never one.
            Some(schema::TypeDefinition::Scalar(_)) => {
                if value.is_object() || value.is_array() {
                    wrong()
                } else {
                    Ok(())
                }
            }
            _ => Err(format!(" ({name} isn't a defined input type)")),
        },
    }
}

/// Whether a literal argument value — or the variable it names — fits `kind`.
fn literal(
    kind: &schema::Type<'_, String>,
    value: &query::Value<'_, String>,
    declared: &HashMap<&str, &query::Type<'_, String>>,
) -> Result<(), String> {
    match value {
        query::Value::Variable(name) => match declared.get(name.as_str()) {
            // An undeclared variable is reported once, by the operation.
            None => Ok(()),
            Some(given) if base(given) == base(kind) => Ok(()),
            Some(given) => Err(format!(
                " (Type mismatch on variable ${name}: declared as {}, expected {})",
                spelled(given),
                spelled(kind)
            )),
        },
        query::Value::Null => match kind {
            schema::Type::NonNullType(_) => Err(" (Expected value to not be null)".to_owned()),
            _ => Ok(()),
        },
        _ => match kind {
            schema::Type::NonNullType(inner) => literal(inner, value, declared),
            schema::Type::ListType(inner) => match value {
                query::Value::List(items) => items
                    .iter()
                    .try_for_each(|item| literal(inner, item, declared)),
                single => literal(inner, single, declared),
            },
            schema::Type::NamedType(name) => literal_named(name, value, declared),
        },
    }
}

fn literal_named(
    name: &str,
    value: &query::Value<'_, String>,
    declared: &HashMap<&str, &query::Type<'_, String>>,
) -> Result<(), String> {
    let fits = match (name, value) {
        ("Int", query::Value::Int(_))
        | ("Float", query::Value::Int(_) | query::Value::Float(_))
        | ("String", query::Value::String(_))
        | ("Boolean", query::Value::Boolean(_))
        | ("ID", query::Value::String(_) | query::Value::Int(_)) => true,
        ("Int" | "Float" | "String" | "Boolean" | "ID", _) => false,
        _ => match (index().types.get(name), value) {
            (Some(schema::TypeDefinition::Enum(members)), query::Value::Enum(given)) => {
                members.values.iter().any(|m| &m.name == given)
            }
            (Some(schema::TypeDefinition::InputObject(input)), query::Value::Object(given)) => {
                for (key, inner) in given {
                    let Some(field) = input.fields.iter().find(|field| &field.name == key) else {
                        return Err(format!(" for {key} (Field is not defined on {name})"));
                    };
                    literal(&field.value_type, inner, declared)?;
                }
                if let Some(missing) = input.fields.iter().find(|field| {
                    matches!(field.value_type, schema::Type::NonNullType(_))
                        && field.default_value.is_none()
                        && !given.contains_key(field.name.as_str())
                }) {
                    return Err(format!(
                        " for {} (Expected value to not be null)",
                        missing.name
                    ));
                }
                true
            }
            (Some(schema::TypeDefinition::Scalar(_)), value) => {
                !matches!(value, query::Value::Object(_) | query::Value::List(_))
            }
            _ => false,
        },
    };
    if fits {
        Ok(())
    } else {
        Err(format!(" (Expected {value} to be a {name})"))
    }
}
