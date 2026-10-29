//! Linear's complexity score of a query document, which it refuses over [`CAP`] with HTTP 400
//! before reading a row and which no schema shows, so a replay must refuse it too.
//!
//! The model is the one `onetaskgraph_linear::graphql::RESOLUTION` documents — a property 0.1,
//! an object 1, a connection's children times its `first` (or `last`, else 50) — and Linear's
//! own scores run [`LINEAR_ABOVE_MODEL`] above it, which the first test reconciles.
use graphql_parser::query as q;
use serde_json::Value;

/// Linear's per-document complexity cap.
pub const CAP: f64 = 10_000.0;

/// How far Linear's reported score ran above [`score`] on both documents it has scored here.
pub const LINEAR_ABOVE_MODEL: f64 = 225.0;

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
        Ok(points) if points + LINEAR_ABOVE_MODEL <= CAP => None,
        Ok(points) => Some(format!(
            "The query is too complex. (scored {:.1} against {CAP})",
            points + LINEAR_ABOVE_MODEL
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

// llmlint: ignore[contracts_have_one_source_or_a_drift_gate] Linear scores documents only at its own runtime and publishes no scoring artifact; this reconciles the model with the two scores Linear itself reported, recorded in `onetaskgraph_linear`'s `MAX_PAGE_SIZE` and `graphql::RESOLUTION` documentation, and re-taking them needs the credential the live lane holds.
#[test]
fn the_model_reconciles_with_the_scores_linear_reported() {
    use onetaskgraph_linear::graphql;
    let key = serde_json::json!({"key":"T"});
    // `RESOLUTION`'s documentation prices it 1827.2, plus 1.1 for `organization{id}`.
    let resolution = score(graphql::RESOLUTION, &key).unwrap();
    assert!((resolution - 1828.3).abs() < 0.01, "{resolution}");
    // Linear scored `RESOLUTION` 30905 and refused it while it had `teams` at its default page
    // and no `organization{id}`, and `PROJECTS` at `first: 250` 17475.
    let refused = graphql::RESOLUTION
        .replace("teams(first:2,", "teams(")
        .replace(" organization{id}", "");
    let projects = score(graphql::PROJECTS, &serde_json::json!({"first":250})).unwrap();
    for (points, reported) in [
        (score(&refused, &key).unwrap(), 30905.0),
        (projects, 17475.0),
    ] {
        assert!(
            (points + LINEAR_ABOVE_MODEL - reported).abs() < 0.01,
            "{points}"
        );
    }
    assert!(refusal(&refused, &key).is_some());
}

#[test]
fn a_nested_hundred_row_page_inside_a_hundred_row_page_is_refused() {
    let nested = "query($after:String){documents(first:100,after:$after){nodes{id project{teams(first:100){nodes{key} pageInfo{hasNextPage}}}} pageInfo{hasNextPage endCursor}}}";
    let refused = refusal(nested, &serde_json::json!({"after":null})).unwrap();
    assert!(refused.contains("too complex"), "{refused}");
    let bounded = nested.replace("teams(first:100)", "teams(first:2)");
    assert_eq!(refusal(&bounded, &serde_json::json!({"after":null})), None);
}
