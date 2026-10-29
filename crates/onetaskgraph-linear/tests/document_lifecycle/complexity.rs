//! Linear's documented complexity model, applied to a query document before it is answered.
//!
//! Linear scores every document it is sent and refuses one over [`CAP`] outright, HTTP 400,
//! `The query is too complex.` — before it reads a row. Nothing in a schema says so, so a
//! replay that only checks field names and wire types answers a document real Linear
//! refuses: the standing-set discovery once nested `teams(first:100)` inside a `first:100`
//! page, about 22,000 points by this model, and both live lanes failed on it at once while
//! every offline journey passed. The model is the one `onetaskgraph_linear::graphql::RESOLUTION`
//! records: a property 0.1, an object 1, and a connection's children multiplied by its `first`
//! (or `last`), or 50 without one. Linear prices some connections by the rows it returns
//! rather than the rows asked for, so this is an upper bound — the direction a refusal here
//! has to err in.
use graphql_parser::query as q;
use serde_json::Value;

/// Linear's per-document complexity cap.
pub const CAP: f64 = 10_000.0;

/// What Linear's model scores `query` at under `variables`, or why it cannot be scored.
pub fn score(query: &str, variables: &Value) -> Result<f64, String> {
    let document = q::parse_query::<String>(query).map_err(|e| e.to_string())?;
    let mut total = 0.0;
    for definition in &document.definitions {
        let q::Definition::Operation(operation) = definition else {
            return Err("complexity is scored without fragments".into());
        };
        let selection = match operation {
            q::OperationDefinition::Query(v) => &v.selection_set,
            q::OperationDefinition::Mutation(v) => &v.selection_set,
            q::OperationDefinition::SelectionSet(v) => v,
            q::OperationDefinition::Subscription(v) => &v.selection_set,
        };
        total += selected(selection, variables)?;
    }
    Ok(total)
}

/// The refusal Linear answers a document over [`CAP`] with, or `None` when it is under it.
pub fn refusal(query: &str, variables: &Value) -> Option<String> {
    match score(query, variables) {
        Ok(points) if points <= CAP => None,
        Ok(points) => Some(format!(
            "The query is too complex. (model scores {points:.1} against {CAP})"
        )),
        Err(error) => Some(format!("complexity could not be scored: {error}")),
    }
}

fn selected(selection: &q::SelectionSet<'_, String>, variables: &Value) -> Result<f64, String> {
    let mut total = 0.0;
    for item in &selection.items {
        let q::Selection::Field(field) = item else {
            return Err("complexity is scored without fragments".into());
        };
        total += if field.selection_set.items.is_empty() {
            0.1
        } else if let Some(rows) = page(field, variables)? {
            rows * selected(&field.selection_set, variables)?
        } else {
            1.0 + selected(&field.selection_set, variables)?
        };
    }
    Ok(total)
}

/// The rows a connection field asks for, or `None` when the field is a plain object.
fn page(field: &q::Field<'_, String>, variables: &Value) -> Result<Option<f64>, String> {
    let named = field
        .arguments
        .iter()
        .find(|(name, _)| name == "first" || name == "last");
    if let Some((name, value)) = named {
        let rows = match value {
            q::Value::Int(rows) => rows.as_i64(),
            q::Value::Variable(variable) => variables.get(variable).and_then(Value::as_i64),
            _ => None,
        };
        return match rows {
            Some(rows) => Ok(Some(rows as f64)),
            // A variable left unset is Linear's default page.
            None if matches!(value, q::Value::Variable(_)) => Ok(Some(50.0)),
            None => Err(format!("{}({name}) is not an integer", field.name)),
        };
    }
    let connection = field.selection_set.items.iter().any(|item| {
        matches!(item, q::Selection::Field(child) if child.name == "nodes" || child.name == "edges")
    });
    Ok(connection.then_some(50.0))
}

#[test]
fn the_model_reproduces_the_scores_linear_documented_for_the_plugin() {
    // `graphql::RESOLUTION`'s own documentation prices it 1827.2 plus 1.1 for `organization{id}`.
    let points = score(
        onetaskgraph_linear::graphql::RESOLUTION,
        &serde_json::json!({"key":"T"}),
    )
    .unwrap();
    assert!((points - 1828.3).abs() < 0.01, "{points}");
    // The document Linear refused `Query too complex`, with `teams` at its default page.
    let refused = "query($key:String!){ organization{id} teams(filter:{key:{eqIgnoreCase:$key}}){nodes{id states(first:250){nodes{id name type} pageInfo{hasNextPage}}}} projectStatuses(first:250){nodes{id name type position} pageInfo{hasNextPage}} }";
    assert!(refusal(refused, &serde_json::json!({"key":"T"})).is_some());
}

#[test]
fn a_nested_hundred_row_page_inside_a_hundred_row_page_is_refused() {
    let nested = "query($after:String){documents(first:100,after:$after){nodes{id project{teams(first:100){nodes{key} pageInfo{hasNextPage}}}} pageInfo{hasNextPage endCursor}}}";
    let refused = refusal(nested, &serde_json::json!({"after":null})).unwrap();
    assert!(refused.contains("too complex"), "{refused}");
    let bounded = nested.replace("teams(first:100)", "teams(first:2)");
    assert_eq!(refusal(&bounded, &serde_json::json!({"after":null})), None);
}
