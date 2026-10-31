//! GitHub's static validation of a GraphQL request, against the complete schema GitHub
//! publishes.
//!
//! Every GitHub double here puts each request to [`validate`] first and answers a refusal the
//! way GitHub does — HTTP 200 with an `errors` array and no `data` — so a request GitHub would
//! refuse cannot pass a test. The schema is `fixtures/github-schema.docs.graphql`, GitHub's
//! published file whole; `fixtures/README.md` holds its provenance and why it is not a
//! reduction.
//!
//! The validation is, all but two rules of it, `apollo-compiler`'s: a maintained implementation
//! of the GraphQL specification's Validation section (§5) — documents, operations, fields,
//! field-selection merging, arguments, fragments and their cycles, values, directives and
//! variables — and of `CoerceVariableValues` (§6.1.2), which holds each variable's JSON value
//! to its declared type with no input field the type does not define and every required one
//! present. The two it does not apply as the specification states are applied here:
//! Input Object Field Uniqueness (§5.6.3), which 1.33 does not check at all, and the `null`
//! default that licenses nothing in All Variable Usages Are Allowed (§5.8.5). A hand-written
//! validator over `graphql-parser` was here before it, and kept missing rules: nested
//! selection merging, a fragment cycle that overflowed the stack, duplicate input fields a
//! parser collapsed. The parser bounds a document's nesting and token count, the validation
//! bounds fragment recursion, and the variables' nesting is bounded here before they are
//! coerced, so a malformed or adversarial request is refused rather than exhausting the stack.
//!
//! What it does not hold a request to is what only GitHub's executor knows — a node id's
//! existence, a permission, a rate or a node-count limit, a custom scalar's spelling. Its
//! messages are `apollo-compiler`'s, not GitHub's wording; the shape of a refusal, an
//! `errors` array of `{message, locations}` objects and no `data`, is GraphQL's own (§7.1.2).
//!
//! Included by path from the loopback board in `onetaskgraph-e2e-support` as well as from this
//! crate's own tests, so both doubles hold to one implementation.

use apollo_compiler::executable::{ExecutableDocument, Operation};
use apollo_compiler::parser::{Parser, SourceMap};
use apollo_compiler::response::{GraphQLError, JsonMap};
use apollo_compiler::validation::{Valid, WithErrors};
use apollo_compiler::{Node, Schema, ast, request};
use serde_json::Value;
use std::collections::HashSet;
use std::sync::OnceLock;

// llmlint: ignore[contracts_have_one_source_or_a_drift_gate] A dated snapshot of an external contract by design: it is pinned to the provenance in fixtures/README.md, which states its gates and why upstream freshness is not one.
/// GitHub's published GraphQL schema, whole.
pub const SCHEMA: &str = include_str!("../fixtures/github-schema.docs.graphql");

/// The deepest a request's selections, values and lists may nest before it is refused rather
/// than parsed further. GitHub's own documents nest a few levels; this is far beyond any of
/// them and far short of the stack a test thread has.
const NESTING_LIMIT: usize = 200;

/// The most tokens a request may hold before it is refused unparsed: well beyond this
/// source's largest batched document.
const TOKEN_LIMIT: usize = 200_000;

/// The schema, parsed and validated once per process. GitHub's file is a valid schema, so a
/// failure here is the vendored copy being wrong, and every test says so.
fn schema() -> &'static Valid<Schema> {
    static PARSED: OnceLock<Valid<Schema>> = OnceLock::new();
    PARSED.get_or_init(|| {
        Schema::parse_and_validate(SCHEMA, "github-schema.docs.graphql").unwrap_or_else(|invalid| {
            panic!("GitHub's published schema is invalid: {}", invalid.errors)
        })
    })
}

/// Every error GitHub's static validation answers `document`, sent with `operation_name` and
/// `variables`, with; empty when GitHub would run it. `variables` of `None` skips their
/// coercion, for a document whose values a caller supplies later.
fn errors(
    document: &str,
    operation_name: Option<&str>,
    variables: Option<&Value>,
) -> Vec<GraphQLError> {
    let schema = schema();
    let ast = match Parser::new()
        .recursion_limit(NESTING_LIMIT)
        .token_limit(TOKEN_LIMIT)
        .parse_ast(document, "request.graphql")
    {
        Ok(ast) => ast,
        Err(invalid) => return invalid.errors.iter().map(|error| error.to_json()).collect(),
    };
    let mut refused = Vec::new();
    for definition in &ast.definitions {
        match definition {
            ast::Definition::OperationDefinition(operation) => {
                for variable in &operation.variables {
                    if let Some(default) = &variable.default_value {
                        unique_input_fields(default, &ast.sources, &mut refused);
                    }
                    directives(&variable.directives, &ast.sources, &mut refused);
                }
                directives(&operation.directives, &ast.sources, &mut refused);
                selections(&operation.selection_set, &ast.sources, &mut refused);
            }
            ast::Definition::FragmentDefinition(fragment) => {
                directives(&fragment.directives, &ast.sources, &mut refused);
                selections(&fragment.selection_set, &ast.sources, &mut refused);
            }
            // Any other definition is refused by the executable document's own rules.
            _ => {}
        }
    }
    // A document that does not build — a field, a type or a fragment the schema does not have —
    // is refused for that alone, since the rules after it would only report its consequences.
    let built = ast
        .to_executable(schema)
        .map_err(|invalid| diagnostics(&invalid));
    let validated = built.and_then(|document| {
        document
            .validate(schema)
            .map_err(|invalid| diagnostics(&invalid))
    });
    let document =
        match validated.and_then(|document| null_defaults_are_no_defaults(document, schema)) {
            Ok(document) if refused.is_empty() => document,
            Ok(_) => return refused,
            Err(invalid) => {
                refused.extend(invalid);
                return refused;
            }
        };
    let Some(variables) = variables else {
        return Vec::new();
    };
    let Ok(operation) = document.operations.get(operation_name) else {
        return vec![message(match operation_name {
            Some(name) => format!("Unknown operation named \"{name}\"."),
            None => "An operation name is required when the document holds more than one \
                     operation."
                .to_owned(),
        })];
    };
    if !matches!(variables, Value::Object(_)) {
        return vec![message(format!(
            "Variables must be a JSON object, not {variables}"
        ))];
    }
    if nesting(variables) > NESTING_LIMIT {
        return vec![message(format!(
            "Variables nest deeper than {NESTING_LIMIT} levels"
        ))];
    }
    let values =
        serde_json::from_value::<JsonMap>(variables.clone()).expect("a JSON object is a JSON map");
    match request::coerce_variable_values(schema, operation, &values) {
        Ok(_) => Vec::new(),
        Err(error) => vec![error.to_graphql_error(&document.sources)],
    }
}

/// How deeply `value` nests, measured without recursion so that no depth of it can exhaust
/// the stack before it is refused.
fn nesting(value: &Value) -> usize {
    let mut deepest = 0;
    let mut pending = vec![(value, 1)];
    while let Some((value, depth)) = pending.pop() {
        deepest = deepest.max(depth);
        match value {
            Value::Array(items) => pending.extend(items.iter().map(|item| (item, depth + 1))),
            Value::Object(fields) => pending.extend(fields.values().map(|item| (item, depth + 1))),
            _ => {}
        }
    }
    deepest
}

/// The specification's Input Object Field Uniqueness (§5.6.3), which `apollo-compiler` 1.33
/// does not apply: an input object literal names each of its fields once, in every argument
/// and default the document holds. The parser has already bounded how deeply any of them
/// nests, so the walk below cannot exhaust the stack.
fn selections(selections: &[ast::Selection], sources: &SourceMap, refused: &mut Vec<GraphQLError>) {
    for selection in selections {
        match selection {
            ast::Selection::Field(field) => {
                for argument in &field.arguments {
                    unique_input_fields(&argument.value, sources, refused);
                }
                directives(&field.directives, sources, refused);
                self::selections(&field.selection_set, sources, refused);
            }
            ast::Selection::FragmentSpread(spread) => {
                directives(&spread.directives, sources, refused)
            }
            ast::Selection::InlineFragment(inline) => {
                directives(&inline.directives, sources, refused);
                self::selections(&inline.selection_set, sources, refused);
            }
        }
    }
}

fn directives(
    directives: &ast::DirectiveList,
    sources: &SourceMap,
    refused: &mut Vec<GraphQLError>,
) {
    for directive in directives.iter() {
        for argument in &directive.arguments {
            unique_input_fields(&argument.value, sources, refused);
        }
    }
}

fn unique_input_fields(
    value: &Node<ast::Value>,
    sources: &SourceMap,
    refused: &mut Vec<GraphQLError>,
) {
    match value.as_ref() {
        ast::Value::Object(fields) => {
            let mut named = HashSet::new();
            for (name, field) in fields {
                if !named.insert(name.as_str()) {
                    refused.push(GraphQLError::new(
                        format!("there can be only one input field named `{name}`"),
                        field.location(),
                        sources,
                    ));
                }
                unique_input_fields(field, sources, refused);
            }
        }
        ast::Value::List(items) => {
            for item in items {
                unique_input_fields(item, sources, refused);
            }
        }
        _ => {}
    }
}

/// Each diagnostic of a document refused, in GraphQL's error shape.
fn diagnostics(invalid: &WithErrors<ExecutableDocument>) -> Vec<GraphQLError> {
    invalid.errors.iter().map(|error| error.to_json()).collect()
}

/// A rule `apollo-compiler` 1.33 applies otherwise than the specification does. Its
/// `IsVariableUsageAllowed` reads any default as licensing a nullable variable in a non-null
/// position, where the specification's `hasNonNullVariableDefaultValue` (§5.8.5) is true only
/// of a default that is not `null`. A `null` default is no licence, so a document holding one
/// is validated again with each such default taken away — which changes no other rule's
/// answer, since `null` is a value of every nullable type and nothing else may hold it.
fn null_defaults_are_no_defaults(
    document: Valid<ExecutableDocument>,
    schema: &Valid<Schema>,
) -> Result<Valid<ExecutableDocument>, Vec<GraphQLError>> {
    let null_default = |operation: &Operation| {
        operation.variables.iter().any(|variable| {
            variable
                .default_value
                .as_ref()
                .is_some_and(|value| value.is_null())
        })
    };
    if !document
        .operations
        .iter()
        .any(|operation| null_default(operation))
    {
        return Ok(document);
    }
    let mut stripped = document.clone().into_inner();
    let operations = stripped
        .operations
        .anonymous
        .iter_mut()
        .chain(stripped.operations.named.values_mut());
    for operation in operations {
        for variable in &mut operation.make_mut().variables {
            if variable
                .default_value
                .as_ref()
                .is_some_and(|value| value.is_null())
            {
                variable.make_mut().default_value = None;
            }
        }
    }
    match stripped.validate(schema) {
        Ok(_) => Ok(document),
        Err(invalid) => Err(diagnostics(&invalid)),
    }
}

/// A request error that stands at no location in the document.
fn message(text: String) -> GraphQLError {
    GraphQLError {
        message: text,
        locations: Vec::new(),
        path: Vec::new(),
        extensions: Default::default(),
    }
}

fn messages(errors: Vec<GraphQLError>) -> Result<(), Vec<String>> {
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.into_iter().map(|error| error.message).collect())
    }
}

/// Whether GitHub's static validation accepts `document` sent with `variables`; `Err` carries
/// each message it would answer, in order.
pub fn validate(document: &str, variables: &Value) -> Result<(), Vec<String>> {
    messages(errors(document, None, Some(variables)))
}

/// [`validate`] of the document alone: every rule but the coercion of the variables' values,
/// for a document whose values a caller supplies later.
pub fn validate_document(document: &str) -> Result<(), Vec<String>> {
    messages(errors(document, None, None))
}

/// `body`, a GraphQL request's JSON — `query`, and `operationName` and `variables` when given —
/// put to GitHub's static validation: `Some` refusal body when GitHub would refuse it, in
/// GraphQL's error shape with no `data`, and `None` when it would run it or when `body` is not
/// a GraphQL request.
#[must_use]
pub fn refuse(body: &Value) -> Option<Value> {
    let document = body.get("query")?.as_str()?;
    let operation_name = match body.get("operationName") {
        None | Some(Value::Null) => None,
        Some(Value::String(name)) => Some(name.as_str()),
        Some(other) => {
            return Some(serde_json::json!({"errors": [{
                "message": format!("operationName must be a string, not {other}")
            }]}));
        }
    };
    let empty = Value::Object(serde_json::Map::new());
    let variables = body
        .get("variables")
        .filter(|value| !value.is_null())
        .unwrap_or(&empty);
    let errors = errors(document, operation_name, Some(variables));
    (!errors.is_empty()).then(|| {
        serde_json::json!({
            "errors": serde_json::to_value(errors).expect("a GraphQL error serializes")
        })
    })
}
