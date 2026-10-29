//! Every selected response field is checked against the pinned Linear schema. Synthetic
//! values replace fixture bodies, never their selected wire types.
use graphql_parser::{query as q, schema as s};
use serde_json::Value;
use std::{collections::BTreeMap, sync::LazyLock};
#[path = "complexity.rs"]
pub mod complexity;
static SCHEMA: LazyLock<s::Document<'static, String>> =
    LazyLock::new(|| s::parse_schema(include_str!("../fixtures/schema.graphql")).unwrap());

pub fn validate(query: &str, body: &Value) -> Result<(), String> {
    let objects: BTreeMap<_, _> = SCHEMA
        .definitions
        .iter()
        .filter_map(|d| match d {
            s::Definition::TypeDefinition(s::TypeDefinition::Object(o)) => {
                Some((o.name.as_str(), o))
            }
            _ => None,
        })
        .collect();
    let query = q::parse_query::<String>(query).map_err(|e| e.to_string())?;
    for definition in &query.definitions {
        let q::Definition::Operation(operation) = definition else {
            return Err("replay has no fragments".into());
        };
        let (root, selection) = match operation {
            q::OperationDefinition::Query(v) => ("Query", &v.selection_set),
            q::OperationDefinition::Mutation(v) => ("Mutation", &v.selection_set),
            q::OperationDefinition::SelectionSet(v) => ("Query", v),
            _ => return Err("replay has no subscriptions".into()),
        };
        selected(&objects, root, selection, &body["data"])?;
    }
    Ok(())
}
fn selected(
    objects: &BTreeMap<&str, &s::ObjectType<'_, String>>,
    name: &str,
    selection: &q::SelectionSet<'_, String>,
    value: &Value,
) -> Result<(), String> {
    // A deleted document is returned null by the recorded service behavior even though
    // introspection declares Query.document non-null; live absence assertions require it.
    if value.is_null() {
        return Ok(());
    }
    let object = objects
        .get(name)
        .ok_or_else(|| format!("schema lacks {name}"))?;
    for field in &selection.items {
        let q::Selection::Field(field) = field else {
            return Err("replay has no fragments".into());
        };
        let definition = object
            .fields
            .iter()
            .find(|f| f.name == field.name)
            .ok_or_else(|| format!("schema lacks {name}.{}", field.name))?;
        for (argument, _) in &field.arguments {
            if !definition.arguments.iter().any(|a| a.name == *argument) {
                return Err(format!("schema lacks {name}.{}({argument})", field.name));
            }
        }
        let key = field.alias.as_ref().unwrap_or(&field.name);
        let value = value
            .get(key)
            .ok_or_else(|| format!("response lacks {name}.{key}"))?;
        typed(
            objects,
            &definition.field_type,
            &field.selection_set,
            value,
            &format!("{name}.{key}"),
        )?;
    }
    Ok(())
}
fn typed(
    objects: &BTreeMap<&str, &s::ObjectType<'_, String>>,
    kind: &s::Type<'_, String>,
    selection: &q::SelectionSet<'_, String>,
    value: &Value,
    path: &str,
) -> Result<(), String> {
    match kind {
        s::Type::NonNullType(inner) => {
            if value.is_null() && path != "Query.document" {
                return Err(format!("response null at required {path}"));
            }
            typed(objects, inner, selection, value, path)
        }
        _ if value.is_null() => Ok(()),
        s::Type::ListType(inner) => {
            let array = value
                .as_array()
                .ok_or_else(|| format!("response non-list at {path}"))?;
            for item in array {
                typed(objects, inner, selection, item, path)?;
            }
            Ok(())
        }
        s::Type::NamedType(name) if objects.contains_key(name.as_str()) => {
            selected(objects, name, selection, value)
        }
        s::Type::NamedType(name) => {
            let valid = match name.as_str() {
                "Boolean" => value.is_boolean(),
                "Float" => value.is_number(),
                "Int" => value.as_i64().is_some(),
                _ => value.is_string(),
            };
            if valid {
                Ok(())
            } else {
                Err(format!("response wrong scalar {name} at {path}"))
            }
        }
    }
}

#[test]
fn retained_ownership_fields_match_the_recorded_full_linear_schema() {
    let output = std::process::Command::new("python3")
        .args([
            "-c",
            "import gzip,sys; print(gzip.open(sys.argv[1], 'rt').read())",
        ])
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../onetaskgraph-live-janitor/tests/fixtures/linear-upstream.graphql.gz"
        ))
        .output()
        .expect("read existing pinned upstream schema");
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    let upstream = s::parse_schema::<String>(&text).unwrap();
    for (name, field_name) in [
        ("Document", "team"),
        ("Project", "teams"),
        ("Team", "key"),
        ("TeamConnection", "pageInfo"),
        ("DocumentConnection", "pageInfo"),
        ("DocumentConnection", "nodes"),
        ("PageInfo", "hasNextPage"),
        ("PageInfo", "endCursor"),
    ] {
        let field = |document: &s::Document<'_, String>| {
            let object = document
                .definitions
                .iter()
                .find_map(|d| match d {
                    s::Definition::TypeDefinition(s::TypeDefinition::Object(o))
                        if o.name == name =>
                    {
                        Some(o)
                    }
                    _ => None,
                })
                .unwrap();
            let field = object.fields.iter().find(|f| f.name == field_name).unwrap();
            (
                format!("{:?}", field.field_type),
                field
                    .arguments
                    .iter()
                    .map(|a| (a.name.clone(), format!("{:?}", a.value_type)))
                    .collect::<BTreeMap<_, _>>(),
            )
        };
        let (expected_type, expected_arguments) = field(&upstream);
        let (actual_type, actual_arguments) = field(&SCHEMA);
        assert_eq!(actual_type, expected_type, "{name}.{field_name}");
        for (argument, kind) in actual_arguments {
            assert_eq!(
                Some(&kind),
                expected_arguments.get(&argument),
                "{name}.{field_name}({argument})"
            );
        }
    }
}

#[test]
fn response_guard_rejects_missing_required_fields_and_wrong_scalar_types() {
    let query = "query($id:String!){document(id:$id){id title url}}";
    assert!(
        validate(
            query,
            &serde_json::json!({"data":{"document":{"id":"d","title":"t"}}})
        )
        .unwrap_err()
        .contains("url")
    );
    assert!(validate(query, &serde_json::json!({"data":{"document":{"id":"d","title":3,"url":"https://example.test"}}})).unwrap_err().contains("title"));
}

#[test]
fn response_guard_validates_discovery_pagination_before_wire_fault_injection() {
    let query =
        "query { documents(first:100) { nodes { id } pageInfo { hasNextPage endCursor } } }";
    let valid = serde_json::json!({"data":{"documents":{"nodes":[],"pageInfo":{"hasNextPage":true,"endCursor":"1"}}}});
    validate(query, &valid).expect("advancing cursor has the recorded wire types");
    let mut missing = valid.clone();
    missing["data"]["documents"]
        .as_object_mut()
        .unwrap()
        .remove("pageInfo");
    assert!(validate(query, &missing).unwrap_err().contains("pageInfo"));
    let mut malformed = valid;
    malformed["data"]["documents"]["pageInfo"]["hasNextPage"] = serde_json::json!("yes");
    assert!(validate(query, &malformed).unwrap_err().contains("Boolean"));
}
