//! The opaque dependency endpoint: a designator naming a tracker, and an item's native id there.

use std::collections::BTreeMap;

use onetaskgraph_plugin_api::{
    DependencyEdge, DependencyEndpoint, Designator, EndpointRefusal, ItemKind, NativeId, SourceName,
};
use serde_json::json;

/// A backend's designator is the contract's digest: `<plugin>/` and the SHA-256 of
/// `<plugin>\n<identifier>`, which never contains the identifier.
#[test]
fn a_backend_designator_is_the_digest_of_plugin_and_identifier() {
    let designator = Designator::of_backend("linear", "0b6c1d6e-org");
    assert_eq!(
        designator.as_str(),
        format!("linear/{}", sha256_hex(b"linear\n0b6c1d6e-org"))
    );
    assert_eq!(designator.prefix(), "linear");
    assert!(!designator.as_str().contains("0b6c1d6e-org"));
    assert_eq!(
        Designator::of_backend("linear", "0b6c1d6e-org"),
        designator,
        "the same backend designates the same way every time"
    );
    assert_ne!(
        Designator::of_backend("github-projects", "0b6c1d6e-org"),
        designator
    );
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// An alias is accepted at 16 to 64 of `[A-Za-z0-9_-]` and refused otherwise.
#[test]
fn an_alias_designator_holds_the_pattern() {
    let alias = Designator::of_alias("planning_notes-01").expect("a valid alias");
    assert_eq!(alias.as_str(), "alias/planning_notes-01");
    assert!(alias.is_alias());
    for refused in [
        "short",
        "has a space in it ok",
        "dots.are.not.allowed.here",
        &"x".repeat(65),
    ] {
        let message = Designator::of_alias(refused).expect_err(refused);
        assert!(!message.contains(refused), "{message} repeats the value");
    }
    assert!(Designator::of_alias(&"x".repeat(64)).is_ok());
    assert!(Designator::of_alias(&"x".repeat(16)).is_ok());
}

/// The opaque form round-trips as exactly its three keys, and a malformed designator is refused.
#[test]
fn the_opaque_form_round_trips_as_exactly_three_keys() {
    let designator = Designator::of_backend("linear", "org");
    let endpoint =
        DependencyEndpoint::opaque(designator.clone(), "uuid-1".to_owned(), ItemKind::Task)
            .expect("an opaque endpoint");
    let written = serde_json::to_value(&endpoint).expect("serializes");
    assert_eq!(
        written,
        json!({"designator": designator.as_str(), "id": "uuid-1", "kind": "task"})
    );
    let read: DependencyEndpoint = serde_json::from_value(written).expect("reads back");
    assert_eq!(read, endpoint);
    assert_eq!(read.designator(), Some(&designator));
    assert!(!read.is_native() && !read.is_qualified());
    assert_eq!(read.source(), None);

    for malformed in [
        json!({"designator": "linear/abc", "id": "x", "kind": "task"}),
        json!({"designator": "Linear/".to_owned() + &"a".repeat(64), "id": "x", "kind": "task"}),
        json!({"designator": "alias/short", "id": "x", "kind": "task"}),
        json!({"designator": designator.as_str(), "id": "", "kind": "task"}),
    ] {
        assert!(
            serde_json::from_value::<DependencyEndpoint>(malformed.clone()).is_err(),
            "{malformed} is refused"
        );
    }
}

/// What a caller asks to be written is refused naming a key beyond the three, never its value.
#[test]
fn a_caller_endpoint_with_another_key_is_refused_naming_the_key() {
    let designator = Designator::of_backend("linear", "org");
    for key in [
        "title", "team", "project", "labels", "status", "url", "anything",
    ] {
        let given = json!({
            "designator": designator.as_str(),
            "id": "uuid-1",
            "kind": "task",
            key: "Secret launch plan",
        });
        let refused = DependencyEndpoint::from_value(&given).expect_err(key);
        assert_eq!(refused, EndpointRefusal::Key(key.to_owned()));
        assert!(!refused.to_string().contains("Secret launch plan"));
    }
    let accepted = DependencyEndpoint::from_value(&json!({
        "designator": designator.as_str(), "id": "uuid-1", "kind": "project"
    }))
    .expect("the three keys alone");
    assert_eq!(accepted.kind, ItemKind::Project);
}

/// A recorded opaque far end is read as a cross-tracker edge, never refused as one the near
/// backend could relate natively.
#[test]
fn a_recorded_opaque_far_end_is_never_a_native_relation() {
    let designator = Designator::of_backend("linear", "org");
    let metadata = BTreeMap::from([(
        DependencyEdge::RECORDED_KEY.to_owned(),
        json!([{"designator": designator.as_str(), "id": "uuid-1", "kind": "task"}]),
    )]);
    let edges = DependencyEdge::recorded(
        &metadata,
        &NativeId("near".to_owned()),
        ItemKind::Task,
        &SourceName::new("board").expect("a name"),
        Some(ItemKind::Task),
    )
    .expect("an opaque far end is recorded");
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].to.designator(), Some(&designator));
}
