//! The projection domain is the one wire contract used by callers and schema consumers.
use onetaskgraph_core::task_projection::{HumanTaskProjected, HumanTaskProjection};
use serde_json::{Value, json};

fn request() -> Value {
    json!({
        "schema_version": 1,
        "item_id": "host/session/item",
        "owner": {"session": "session", "run": null, "plan": null},
        "title": "Review release",
        "body": "Decide whether to release.",
        "priority": "high",
        "repositories": ["github.com/operator/product"],
        "project": null,
        "source": "plans",
        "state": "open",
        "reason": null
    })
}

#[test]
fn projection_request_round_trips_existing_priority_and_nullable_reason() {
    let value = request();
    let parsed: HumanTaskProjection = serde_json::from_value(value.clone()).expect("v1");
    assert_eq!(serde_json::to_value(parsed).expect("serialize"), value);
    for reason in [None, Some(Value::Null), Some(json!("Not needed"))] {
        let mut value = request();
        value["state"] = json!("dismissed");
        match reason {
            Some(reason) => value["reason"] = reason,
            None => {
                value.as_object_mut().expect("mapping").remove("reason");
            }
        }
        let parsed: HumanTaskProjection = serde_json::from_value(value.clone()).expect("dismissal");
        assert_eq!(parsed.reason.as_deref(), value["reason"].as_str());
    }
}

#[test]
fn projection_refuses_questions_unknown_fields_wrong_versions_and_malformed_origins() {
    for (key, invalid) in [
        ("schema_version", json!(2)),
        ("schema_version", json!("1")),
        ("state", json!("question")),
        ("priority", json!("critical")),
        (
            "repositories",
            json!(["https://github.com/operator/product.git"]),
        ),
        ("question", json!("Proceed?")),
        ("item_id", json!(12)),
    ] {
        let mut value = request();
        value[key] = invalid;
        assert!(
            serde_json::from_value::<HumanTaskProjection>(value).is_err(),
            "{key}"
        );
    }
    for key in ["schema_version", "item_id", "owner", "project", "state"] {
        let mut value = request();
        value.as_object_mut().expect("mapping").remove(key);
        assert!(
            serde_json::from_value::<HumanTaskProjection>(value).is_err(),
            "{key}"
        );
    }
    let mut value = request();
    value["owner"]["question"] = json!("Proceed?");
    assert!(serde_json::from_value::<HumanTaskProjection>(value).is_err());
}

#[test]
fn projection_response_has_only_the_declared_card_binding() {
    let value = json!({"task":"human:ISSUE-1","url":"https://github.com/operator/product/issues/1",
        "project":"P-1","source":"human"});
    let parsed: HumanTaskProjected = serde_json::from_value(value.clone()).expect("binding");
    assert_eq!(serde_json::to_value(parsed).expect("serialize"), value);
    let mut invalid = value;
    invalid["question"] = json!("Proceed?");
    assert!(serde_json::from_value::<HumanTaskProjected>(invalid).is_err());
}

#[test]
fn exported_projection_schemas_come_from_the_domain_declaration() {
    let bundle = onetaskgraph_core::schema_bundle();
    for (name, derived) in [
        (
            "HumanTaskProjection",
            schemars::schema_for!(HumanTaskProjection),
        ),
        (
            "HumanTaskProjected",
            schemars::schema_for!(HumanTaskProjected),
        ),
    ] {
        assert_eq!(
            bundle["roots"][name],
            serde_json::to_value(derived).expect("schema")
        );
    }
    let schema = &bundle["roots"]["HumanTaskProjection"];
    let validator = jsonschema::validator_for(schema).expect("valid request schema");
    assert!(validator.is_valid(&request()));
    let mut question = request();
    question["state"] = json!("question");
    assert!(!validator.is_valid(&question));
    let mut version = request();
    version["schema_version"] = json!(2);
    assert!(!validator.is_valid(&version));
    let mut absent_reason = request();
    absent_reason
        .as_object_mut()
        .expect("mapping")
        .remove("reason");
    assert!(validator.is_valid(&absent_reason));
}
