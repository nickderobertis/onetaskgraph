//! GitHub's static validation of a GraphQL request, against the complete schema GitHub
//! publishes.
//!
//! Every GitHub double here puts each request to [`validate`] first and answers a refusal the
//! way GitHub does — HTTP 200 with an `errors` array and no `data` — so a request GitHub would
//! refuse cannot pass a test. The schema is `fixtures/github-schema.docs.graphql`, GitHub's
//! published file whole; `fixtures/README.md` holds its provenance and why it is not a
//! reduction.
//!
//! The rules are the GraphQL specification's validation section as they reach a document and
//! its variables: the document parses, its operation names are unique and an anonymous one
//! stands alone; every selected field exists on its type — introspection included, against the
//! specification's own introspection types — with each argument known and given once, every
//! required one given and every literal one of its type; two selections answering under one
//! name on one type select the same field with the same arguments; a leaf has no selection and
//! a composite one; every fragment is used, its name unique, its type existing and able to
//! apply where it is spread, and none spreads itself; every directive is defined, allowed where
//! it stands, not repeated unless repeatable, and given its required arguments; every variable
//! used is declared, every declared one used and declared once, each at an input type, its
//! default a constant of that type, and each use at a position its type and nullability fit;
//! and the variables are an object whose every value coerces to its declared type, with no
//! input field the type does not define and every required one present. What it does not hold
//! a request to is what only GitHub's executor knows — a node id's existence, a permission, a
//! rate or a node-count limit, a custom scalar's spelling — nor whether two same-named
//! selections on mutually exclusive object types return the same shape. Its messages follow
//! GitHub's wording where GitHub's is known.
//!
//! Included by path from the loopback board in `onetaskgraph-e2e-support` as well as from this
//! crate's own tests, so both doubles hold to one implementation.

use graphql_parser::{query, schema};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

// llmlint: ignore-block[contracts_have_one_source_or_a_drift_gate] Dated snapshots of external contracts by design: SCHEMA is pinned to the provenance in fixtures/README.md, which states its gates and why upstream freshness is not one; INTROSPECTION is the fixed October 2021 edition of the GraphQL specification.
/// GitHub's published GraphQL schema, whole.
pub const SCHEMA: &str = include_str!("../fixtures/github-schema.docs.graphql");

/// The introspection types every GraphQL schema has, as the specification (October 2021,
/// §4.5) defines them. A published schema file never spells them, and a request may select
/// them through `__schema` and `__type`; `@skip` and `@include` are the specification's
/// executable directives, which a published schema file does not spell either.
const INTROSPECTION: &str = r#"
type __Schema { description: String types: [__Type!]! queryType: __Type! mutationType: __Type
  subscriptionType: __Type directives: [__Directive!]! }
type __Type { kind: __TypeKind! name: String description: String specifiedByURL: String
  fields(includeDeprecated: Boolean = false): [__Field!] interfaces: [__Type!]
  possibleTypes: [__Type!] enumValues(includeDeprecated: Boolean = false): [__EnumValue!]
  inputFields(includeDeprecated: Boolean = false): [__InputValue!] ofType: __Type }
enum __TypeKind { SCALAR OBJECT INTERFACE UNION ENUM INPUT_OBJECT LIST NON_NULL }
type __Field { name: String! description: String
  args(includeDeprecated: Boolean = false): [__InputValue!]! type: __Type!
  isDeprecated: Boolean! deprecationReason: String }
type __InputValue { name: String! description: String type: __Type! defaultValue: String
  isDeprecated: Boolean! deprecationReason: String }
type __EnumValue { name: String! description: String isDeprecated: Boolean!
  deprecationReason: String }
type __Directive { name: String! description: String locations: [__DirectiveLocation!]!
  args(includeDeprecated: Boolean = false): [__InputValue!]! isRepeatable: Boolean! }
enum __DirectiveLocation { QUERY MUTATION SUBSCRIPTION FIELD FRAGMENT_DEFINITION
  FRAGMENT_SPREAD INLINE_FRAGMENT VARIABLE_DEFINITION SCHEMA SCALAR OBJECT FIELD_DEFINITION
  ARGUMENT_DEFINITION INTERFACE UNION ENUM ENUM_VALUE INPUT_OBJECT INPUT_FIELD_DEFINITION }
type __IntrospectionRoot { __schema: __Schema! __type(name: String!): __Type }
directive @skip(if: Boolean!) on FIELD | FRAGMENT_SPREAD | INLINE_FRAGMENT
directive @include(if: Boolean!) on FIELD | FRAGMENT_SPREAD | INLINE_FRAGMENT
"#;
// llmlint: ignore-end[contracts_have_one_source_or_a_drift_gate]

type Definition = schema::TypeDefinition<'static, String>;
type Field = schema::Field<'static, String>;
type Directive = schema::DirectiveDefinition<'static, String>;

struct Index {
    types: HashMap<&'static str, &'static Definition>,
    /// Every interface and union, with the object types it can be.
    possible: HashMap<&'static str, HashSet<&'static str>>,
    directives: HashMap<&'static str, &'static Directive>,
    /// `__schema` and `__type`, which every query root answers beside its own fields.
    introspection_roots: &'static [Field],
}

fn index() -> &'static Index {
    static INDEX: OnceLock<Index> = OnceLock::new();
    static DOCUMENTS: OnceLock<[schema::Document<'static, String>; 2]> = OnceLock::new();
    INDEX.get_or_init(|| {
        let documents = DOCUMENTS.get_or_init(|| {
            [
                schema::parse_schema::<String>(SCHEMA).expect("GitHub's published schema parses"),
                schema::parse_schema::<String>(INTROSPECTION)
                    .expect("the specification's introspection types parse"),
            ]
        });
        let mut index = Index {
            types: HashMap::new(),
            possible: HashMap::new(),
            directives: HashMap::new(),
            introspection_roots: &[],
        };
        for definition in documents.iter().flat_map(|document| &document.definitions) {
            match definition {
                schema::Definition::DirectiveDefinition(directive) => {
                    index.directives.insert(&directive.name, directive);
                }
                schema::Definition::TypeDefinition(schema::TypeDefinition::Object(object))
                    if object.name == "__IntrospectionRoot" =>
                {
                    index.introspection_roots = &object.fields;
                }
                schema::Definition::TypeDefinition(kind) => {
                    let name = type_name(kind);
                    index.types.insert(name, kind);
                    match kind {
                        schema::TypeDefinition::Object(object) => {
                            for interface in &object.implements_interfaces {
                                index
                                    .possible
                                    .entry(interface.as_str())
                                    .or_default()
                                    .insert(name);
                            }
                        }
                        schema::TypeDefinition::Union(union) => {
                            index
                                .possible
                                .entry(name)
                                .or_default()
                                .extend(union.types.iter().map(String::as_str));
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        index
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

/// A GraphQL type reference, owned, so a variable's declared type and a schema position's
/// type compare as one shape whatever document each was parsed from.
#[derive(Clone, PartialEq, Eq)]
enum Ty {
    Named(String),
    List(Box<Ty>),
    NonNull(Box<Ty>),
}

impl Ty {
    fn of(kind: &query::Type<'_, String>) -> Self {
        match kind {
            query::Type::NamedType(name) => Self::Named(name.clone()),
            query::Type::ListType(inner) => Self::List(Box::new(Self::of(inner))),
            query::Type::NonNullType(inner) => Self::NonNull(Box::new(Self::of(inner))),
        }
    }

    fn base(&self) -> &str {
        match self {
            Self::Named(name) => name,
            Self::List(inner) | Self::NonNull(inner) => inner.base(),
        }
    }

    /// This type with every non-null wrapper taken off: two types equal here differ only in
    /// nullability.
    fn nullable(&self) -> Self {
        match self {
            Self::Named(name) => Self::Named(name.clone()),
            Self::List(inner) => Self::List(Box::new(inner.nullable())),
            Self::NonNull(inner) => inner.nullable(),
        }
    }

    const fn is_non_null(&self) -> bool {
        matches!(self, Self::NonNull(_))
    }
}

impl std::fmt::Display for Ty {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Named(name) => write!(formatter, "{name}"),
            Self::List(inner) => write!(formatter, "[{inner}]"),
            Self::NonNull(inner) => write!(formatter, "{inner}!"),
        }
    }
}

/// Whether a variable of type `variable` may be used where `location` is expected — the
/// specification's `IsVariableUsageAllowed`, with its one allowance: a nullable variable in a
/// non-null position when the variable or the position has a default.
fn usable(variable: &Ty, variable_default: bool, location: &Ty, location_default: bool) -> bool {
    if let (Ty::NonNull(inner), false) = (location, variable.is_non_null())
        && (variable_default || location_default)
    {
        return compatible(variable, inner);
    }
    compatible(variable, location)
}

/// The specification's `AreTypesCompatible`.
fn compatible(variable: &Ty, location: &Ty) -> bool {
    match (variable, location) {
        (Ty::NonNull(variable), Ty::NonNull(location)) => compatible(variable, location),
        (_, Ty::NonNull(_)) => false,
        (Ty::NonNull(variable), location) => compatible(variable, location),
        (Ty::List(variable), Ty::List(location)) => compatible(variable, location),
        (Ty::List(_), _) | (_, Ty::List(_)) => false,
        (Ty::Named(variable), Ty::Named(location)) => variable == location,
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

fn field_of(parent: &str, name: &str) -> Option<&'static Field> {
    let index = index();
    let fields: &'static [Field] = match index.types.get(parent)? {
        schema::TypeDefinition::Object(object) => &object.fields,
        schema::TypeDefinition::Interface(interface) => &interface.fields,
        _ => return None,
    };
    fields.iter().find(|field| field.name == name).or_else(|| {
        (parent == "Query")
            .then(|| index.introspection_roots.iter().find(|f| f.name == name))
            .flatten()
    })
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

/// Each variable an operation declares: its type and whether it has a default.
type Declared = HashMap<String, (Ty, bool)>;

/// Where a value stands: the type expected there, whether the position has a default, and the
/// argument it is under, for the messages that name it.
struct Position<'a> {
    kind: Ty,
    has_default: bool,
    described: &'a str,
}

impl Position<'_> {
    /// The position inside this one's wrapper, which carries no default of its own.
    fn within(&self, inner: &Ty) -> Self {
        Position {
            kind: inner.clone(),
            has_default: false,
            described: self.described,
        }
    }
}

struct Walk<'q> {
    fragments: HashMap<&'q str, &'q query::FragmentDefinition<'q, String>>,
    declared: Declared,
    used: HashSet<String>,
    errors: Vec<String>,
    /// The fragments being walked, outermost first: one spread inside itself is a cycle.
    entered: Vec<String>,
    /// Every fragment some operation spreads.
    spread: HashSet<String>,
}

/// Whether GitHub's static validation accepts `document` sent with `variables`; `Err` carries
/// each message it would answer, in order.
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
        declared: HashMap::new(),
        used: HashSet::new(),
        errors: Vec::new(),
        entered: Vec::new(),
        spread: HashSet::new(),
    };
    let variables = match variables {
        Some(Value::Object(_)) | None => variables,
        Some(other) => {
            return Err(vec![format!(
                "Variables must be a JSON object, not {other}"
            )]);
        }
    };
    for definition in &parsed.definitions {
        if let query::Definition::Fragment(fragment) = definition
            && walk
                .fragments
                .insert(fragment.name.as_str(), fragment)
                .is_some()
        {
            walk.errors.push(format!(
                "Fragment name \"{}\" must be unique",
                fragment.name
            ));
        }
    }
    let operations = parsed
        .definitions
        .iter()
        .filter_map(|definition| match definition {
            query::Definition::Operation(operation) => Some(operation),
            query::Definition::Fragment(_) => None,
        })
        .collect::<Vec<_>>();
    let mut named = HashSet::new();
    for operation in &operations {
        match operation_name(operation) {
            None if operations.len() > 1 => walk
                .errors
                .push("This anonymous operation must be the only defined operation.".to_owned()),
            Some(name) if !named.insert(name) => walk
                .errors
                .push(format!("Operation name \"{name}\" must be unique")),
            _ => {}
        }
        walk.operation(operation, variables);
    }
    for definition in &parsed.definitions {
        if let query::Definition::Fragment(fragment) = definition
            && !walk.spread.contains(fragment.name.as_str())
        {
            walk.errors.push(format!(
                "Fragment {} was defined, but not used",
                fragment.name
            ));
        }
    }
    if walk.errors.is_empty() {
        Ok(())
    } else {
        Err(walk.errors)
    }
}

fn operation_name<'q>(operation: &'q query::OperationDefinition<'q, String>) -> Option<&'q str> {
    match operation {
        query::OperationDefinition::SelectionSet(_) => None,
        query::OperationDefinition::Query(value) => value.name.as_deref(),
        query::OperationDefinition::Mutation(value) => value.name.as_deref(),
        query::OperationDefinition::Subscription(value) => value.name.as_deref(),
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
    fn operation(
        &mut self,
        operation: &'q query::OperationDefinition<'q, String>,
        variables: Option<&Value>,
    ) {
        let (root, location, name, declared, directives, selection) = match operation {
            query::OperationDefinition::SelectionSet(selection) => {
                ("Query", "QUERY", None, &[][..], &[][..], selection)
            }
            query::OperationDefinition::Query(value) => (
                "Query",
                "QUERY",
                value.name.as_deref(),
                &value.variable_definitions[..],
                &value.directives[..],
                &value.selection_set,
            ),
            query::OperationDefinition::Mutation(value) => (
                "Mutation",
                "MUTATION",
                value.name.as_deref(),
                &value.variable_definitions[..],
                &value.directives[..],
                &value.selection_set,
            ),
            query::OperationDefinition::Subscription(value) => (
                "Subscription",
                "SUBSCRIPTION",
                value.name.as_deref(),
                &value.variable_definitions[..],
                &value.directives[..],
                &value.selection_set,
            ),
        };
        let operation_name = name.map_or_else(
            || format!("anonymous {}", root.to_lowercase()),
            |name| format!("{} {name}", root.to_lowercase()),
        );
        self.declared.clear();
        for definition in declared {
            self.variable(definition, variables);
        }
        self.used.clear();
        self.entered.clear();
        self.directives(directives, location);
        self.mergeable(root, selection);
        self.selection(root, selection);
        for definition in declared {
            if !self.used.contains(definition.name.as_str()) {
                self.errors.push(format!(
                    "Variable ${} is declared by {operation_name} but not used",
                    definition.name
                ));
            }
        }
        let mut undeclared = self
            .used
            .iter()
            .filter(|name| !self.declared.contains_key(name.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        undeclared.sort();
        for name in undeclared {
            self.errors.push(format!(
                "Variable ${name} is used by {operation_name} but not declared"
            ));
        }
    }

    /// One declared variable: unique, of an input type, its default of that type, and — when
    /// the request's values are given — its value coercing to it.
    fn variable(
        &mut self,
        definition: &'q query::VariableDefinition<'q, String>,
        variables: Option<&Value>,
    ) {
        let kind = Ty::of(&definition.var_type);
        if self
            .declared
            .insert(
                definition.name.clone(),
                (
                    kind.clone(),
                    non_null_default(definition.default_value.as_ref()),
                ),
            )
            .is_some()
        {
            self.errors.push(format!(
                "There can only be one variable named \"{}\"",
                definition.name
            ));
        }
        if !is_input(kind.base()) {
            self.errors.push(format!(
                "{} isn't a defined input type (on ${})",
                kind.base(),
                definition.name
            ));
            return;
        }
        if let Some(default) = &definition.default_value {
            let position = Position {
                kind: kind.clone(),
                has_default: false,
                described: &definition.name,
            };
            // A default is a constant: it may name no variable, so it is checked against an
            // empty declaration, and a variable inside it is reported as not one of its type.
            if let Err(problem) =
                literal(&position, default, &HashMap::new()).and_then(|()| constant(default))
            {
                self.errors.push(format!(
                    "Default value for ${} doesn't match type {kind}{problem}",
                    definition.name
                ));
            }
        }
        let Some(variables) = variables else {
            return;
        };
        match variables.get(definition.name.as_str()) {
            None if definition.default_value.is_some() => {}
            given => {
                if let Err(problem) = coerce(&kind, given.unwrap_or(&Value::Null)) {
                    self.errors.push(format!(
                        "Variable ${} of type {kind} was provided invalid value{problem}",
                        definition.name
                    ));
                }
            }
        }
    }

    fn selection(&mut self, parent: &str, selection: &'q query::SelectionSet<'q, String>) {
        for item in &selection.items {
            match item {
                query::Selection::Field(field) => {
                    self.directives(&field.directives, "FIELD");
                    self.field(parent, field);
                }
                query::Selection::InlineFragment(fragment) => {
                    self.directives(&fragment.directives, "INLINE_FRAGMENT");
                    let condition = match &fragment.type_condition {
                        Some(query::TypeCondition::On(name)) => name.as_str(),
                        None => parent,
                    };
                    if self.applies(parent, condition) {
                        self.selection(condition, &fragment.selection_set);
                    }
                }
                query::Selection::FragmentSpread(spread) => {
                    self.directives(&spread.directives, "FRAGMENT_SPREAD");
                    self.spread(parent, spread.fragment_name.as_str());
                }
            }
        }
    }

    /// Two selections answering under one response name in one selection set — through any
    /// fragment — must be the same field with the same arguments, unless they sit on two
    /// different object types, which can never both apply.
    fn mergeable(&mut self, parent: &str, selection: &'q query::SelectionSet<'q, String>) {
        let mut fields = Vec::new();
        self.collect_fields(parent, selection, &mut fields, &mut Vec::new());
        let mut reported = HashSet::new();
        for (at, (answer, on, name, arguments)) in fields.iter().enumerate() {
            for (other_answer, other_on, other_name, other_arguments) in &fields[at + 1..] {
                let exclusive = on != other_on
                    && matches!(
                        index().types.get(on.as_str()),
                        Some(schema::TypeDefinition::Object(_))
                    )
                    && matches!(
                        index().types.get(other_on.as_str()),
                        Some(schema::TypeDefinition::Object(_))
                    );
                if answer != other_answer || exclusive || !reported.insert(answer.clone()) {
                    continue;
                }
                if name != other_name {
                    self.errors.push(format!(
                        "Field '{answer}' has a field conflict: {name} or {other_name}?"
                    ));
                } else if arguments != other_arguments {
                    self.errors.push(format!(
                        "Field '{answer}' has an argument conflict: {{{arguments}}} or \
                         {{{other_arguments}}}?"
                    ));
                } else {
                    reported.remove(answer);
                }
            }
        }
    }

    /// Every field a selection set selects, flattened through its fragments: the name it
    /// answers under, the type it is selected on, its name and its arguments as written.
    fn collect_fields(
        &self,
        parent: &str,
        selection: &'q query::SelectionSet<'q, String>,
        fields: &mut Vec<(String, String, String, String)>,
        entered: &mut Vec<&'q str>,
    ) {
        for item in &selection.items {
            match item {
                query::Selection::Field(field) => {
                    let mut arguments = field
                        .arguments
                        .iter()
                        .map(|(name, value)| format!("{name}: {value}"))
                        .collect::<Vec<_>>();
                    arguments.sort();
                    fields.push((
                        field.alias.clone().unwrap_or_else(|| field.name.clone()),
                        parent.to_owned(),
                        field.name.clone(),
                        arguments.join(", "),
                    ));
                }
                query::Selection::InlineFragment(fragment) => {
                    let on = match &fragment.type_condition {
                        Some(query::TypeCondition::On(name)) => name.as_str(),
                        None => parent,
                    };
                    self.collect_fields(on, &fragment.selection_set, fields, entered);
                }
                query::Selection::FragmentSpread(spread) => {
                    let name = spread.fragment_name.as_str();
                    if let Some(fragment) = self.fragments.get(name)
                        && !entered.contains(&name)
                    {
                        let query::TypeCondition::On(on) = &fragment.type_condition;
                        entered.push(name);
                        self.collect_fields(on, &fragment.selection_set, fields, entered);
                        entered.pop();
                    }
                }
            }
        }
    }

    fn spread(&mut self, parent: &str, name: &str) {
        self.spread.insert(name.to_owned());
        let Some(fragment) = self.fragments.get(name).copied() else {
            self.errors
                .push(format!("Fragment {name} was used, but not defined"));
            return;
        };
        if self.entered.iter().any(|entered| entered == name) {
            self.errors
                .push(format!("Fragment {name} contains an infinite loop"));
            return;
        }
        let query::TypeCondition::On(condition) = &fragment.type_condition;
        if self.applies(parent, condition) {
            self.directives(&fragment.directives, "FRAGMENT_DEFINITION");
            self.entered.push(name.to_owned());
            self.selection(condition, &fragment.selection_set);
            self.entered.pop();
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

    /// Every directive at one place in the document: defined, allowed at `location`, and given
    /// exactly the arguments its definition takes.
    fn directives(&mut self, given: &'q [query::Directive<'q, String>], location: &str) {
        let mut seen = HashSet::new();
        for directive in given {
            let name = directive.name.as_str();
            if !seen.insert(name)
                && index()
                    .directives
                    .get(name)
                    .is_some_and(|definition| !definition.repeatable)
            {
                self.errors.push(format!(
                    "The directive \"{name}\" can only be used once at this location."
                ));
            }
            let Some(definition) = index().directives.get(name) else {
                self.errors
                    .push(format!("Directive @{name} is not defined"));
                for (_, value) in &directive.arguments {
                    collect_variables(value, &mut self.used);
                }
                continue;
            };
            if !definition
                .locations
                .iter()
                .any(|allowed| allowed.as_str() == location)
            {
                self.errors.push(format!(
                    "'@{name}' can't be applied to {} (allowed: {})",
                    spelled_location(location),
                    definition
                        .locations
                        .iter()
                        .map(|allowed| spelled_location(allowed.as_str()))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            self.arguments(
                &format!("Directive '@{name}'"),
                &definition.arguments,
                &directive.arguments,
            );
        }
    }

    /// The arguments given at a field or a directive against those it defines.
    fn arguments(
        &mut self,
        owner: &str,
        defined: &'static [schema::InputValue<'static, String>],
        given: &'q [(String, query::Value<'q, String>)],
    ) {
        let mut seen = HashSet::new();
        for (argument, value) in given {
            if !seen.insert(argument.as_str()) {
                self.errors.push(format!(
                    "There can be only one argument named \"{argument}\""
                ));
            }
            collect_variables(value, &mut self.used);
            match defined.iter().find(|a| &a.name == argument) {
                None => self
                    .errors
                    .push(format!("{owner} doesn't accept argument '{argument}'")),
                Some(expected) => {
                    let position = Position {
                        kind: Ty::of(&expected.value_type),
                        has_default: non_null_default(expected.default_value.as_ref()),
                        described: argument,
                    };
                    if let Err(problem) = literal(&position, value, &self.declared) {
                        self.errors.push(format!(
                            "Argument '{argument}' on {owner} has an invalid value{problem}"
                        ));
                    }
                }
            }
        }
        let missing = defined
            .iter()
            .filter(|argument| {
                matches!(argument.value_type, schema::Type::NonNullType(_))
                    && argument.default_value.is_none()
                    && !given.iter().any(|(name, _)| name == &argument.name)
            })
            .map(|argument| argument.name.as_str())
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            self.errors.push(format!(
                "{owner} is missing required arguments: {}",
                missing.join(", ")
            ));
        }
    }

    fn field(&mut self, parent: &str, field: &'q query::Field<'q, String>) {
        let name = field.name.as_str();
        if name == "__typename" {
            self.arguments(&format!("Field '{name}'"), &[], &field.arguments);
            if !field.selection_set.items.is_empty() {
                self.errors.push(format!(
                    "Selections can't be made on scalars (field '__typename' returns String but \
                     has selections) on {parent}"
                ));
            }
            return;
        }
        let Some(definition) = field_of(parent, name) else {
            for (_, value) in &field.arguments {
                collect_variables(value, &mut self.used);
            }
            self.errors
                .push(format!("Field '{name}' doesn't exist on type '{parent}'"));
            return;
        };
        self.arguments(
            &format!("Field '{name}'"),
            &definition.arguments,
            &field.arguments,
        );
        let returns = Ty::of(&definition.field_type);
        let returns = returns.base();
        if is_composite(returns) {
            if field.selection_set.items.is_empty() {
                self.errors.push(format!(
                    "Field must have selections (field '{name}' returns {returns} but has no \
                     selections. Did you mean '{name} {{ ... }}'?)"
                ));
            } else {
                self.mergeable(returns, &field.selection_set);
                self.selection(returns, &field.selection_set);
            }
        } else if !field.selection_set.items.is_empty() {
            self.errors.push(format!(
                "Selections can't be made on scalars (field '{name}' returns {returns} but has \
                 selections)"
            ));
        }
    }
}

/// Whether a default lets a nullable variable stand where a non-null one is expected: only a
/// default that is itself not `null` does.
fn non_null_default(default: Option<&query::Value<'_, String>>) -> bool {
    default.is_some_and(|value| !matches!(value, query::Value::Null))
}

/// A directive location as GitHub's messages spell it: `inline fragments` for
/// `INLINE_FRAGMENT`.
fn spelled_location(location: &str) -> String {
    format!("{}s", location.to_lowercase().replace('_', " "))
}

/// Whether a default value is a constant — no variable anywhere inside it.
fn constant(value: &query::Value<'_, String>) -> Result<(), String> {
    let mut named = HashSet::new();
    collect_variables(value, &mut named);
    match named.into_iter().next() {
        None => Ok(()),
        Some(variable) => Err(format!(" (a default cannot name the variable ${variable})")),
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
fn coerce(kind: &Ty, value: &Value) -> Result<(), String> {
    match kind {
        Ty::NonNull(inner) => {
            if value.is_null() {
                Err(" (Expected value to not be null)".to_owned())
            } else {
                coerce(inner, value)
            }
        }
        _ if value.is_null() => Ok(()),
        Ty::List(inner) => match value {
            Value::Array(items) => items.iter().try_for_each(|item| coerce(inner, item)),
            single => coerce(inner, single),
        },
        Ty::Named(name) => coerce_named(name, value),
    }
}

fn coerce_named(name: &str, value: &Value) -> Result<(), String> {
    let fits = match name {
        "Int" => value.as_i64().is_some_and(|n| i32::try_from(n).is_ok()),
        "Float" => value.is_number(),
        "String" => value.is_string(),
        "Boolean" => value.is_boolean(),
        "ID" => value.is_string() || value.is_i64() || value.is_u64(),
        _ => match index().types.get(name) {
            Some(schema::TypeDefinition::Enum(members)) => {
                if value
                    .as_str()
                    .is_some_and(|given| members.values.iter().any(|m| m.name == given))
                {
                    return Ok(());
                }
                return Err(format!(
                    " (Expected {value} to be one of: {})",
                    members
                        .values
                        .iter()
                        .map(|m| m.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            Some(schema::TypeDefinition::InputObject(input)) => match value {
                Value::Object(given) => return coerce_input(input, given),
                _ => false,
            },
            // GitHub's custom scalars (`DateTime`, `URI`, `Base64String`…) take a serialized
            // spelling only GitHub's executor parses; a structured value is never one.
            Some(schema::TypeDefinition::Scalar(_)) => !(value.is_object() || value.is_array()),
            _ => return Err(format!(" ({name} isn't a defined input type)")),
        },
    };
    if fits {
        Ok(())
    } else {
        Err(format!(" (Could not coerce value {value} to {name})"))
    }
}

fn coerce_input(
    input: &schema::InputObjectType<'static, String>,
    given: &serde_json::Map<String, Value>,
) -> Result<(), String> {
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
            Some(inner) => coerce(&Ty::of(&field.value_type), inner).map_err(|problem| {
                problem.strip_prefix(" for ").map_or_else(
                    || format!(" for {}{problem}", field.name),
                    |nested| format!(" for {}.{nested}", field.name),
                )
            })?,
        }
    }
    Ok(())
}

/// Whether a literal value — or the variable it names — fits the position it stands at.
fn literal(
    position: &Position<'_>,
    value: &query::Value<'_, String>,
    declared: &Declared,
) -> Result<(), String> {
    let kind = &position.kind;
    match value {
        query::Value::Variable(name) => match declared.get(name.as_str()) {
            // An undeclared variable is reported once, by the operation.
            None => Ok(()),
            Some((given, has_default)) => {
                if usable(given, *has_default, kind, position.has_default) {
                    Ok(())
                } else {
                    let mismatch = if given.nullable() == kind.nullable() {
                        "Nullability mismatch"
                    } else {
                        "Type mismatch"
                    };
                    Err(format!(
                        " ({mismatch} on variable ${name} and argument {} ({given} / {kind}))",
                        position.described
                    ))
                }
            }
        },
        query::Value::Null => {
            if kind.is_non_null() {
                Err(" (Expected value to not be null)".to_owned())
            } else {
                Ok(())
            }
        }
        _ => match kind {
            Ty::NonNull(inner) => literal(&position.within(inner), value, declared),
            Ty::List(inner) => match value {
                query::Value::List(items) => items
                    .iter()
                    .try_for_each(|item| literal(&position.within(inner), item, declared)),
                single => literal(&position.within(inner), single, declared),
            },
            Ty::Named(name) => literal_named(name, position.described, value, declared),
        },
    }
}

fn literal_named(
    name: &str,
    described: &str,
    value: &query::Value<'_, String>,
    declared: &Declared,
) -> Result<(), String> {
    let fits = match (name, value) {
        ("Int", query::Value::Int(number)) => number
            .as_i64()
            .is_some_and(|number| i32::try_from(number).is_ok()),
        ("Float", query::Value::Int(_) | query::Value::Float(_))
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
                    let position = Position {
                        kind: Ty::of(&field.value_type),
                        has_default: non_null_default(field.default_value.as_ref()),
                        described,
                    };
                    literal(&position, inner, declared)?;
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
