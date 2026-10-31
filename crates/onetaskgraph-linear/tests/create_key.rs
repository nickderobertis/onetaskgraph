//! The id a create safe to retry is sent to Linear under, pinned to a fixed vector so it can
//! never drift between releases: an attempt of one release and its retry under the next have to
//! send, and look up, the same id.

use onetaskgraph_linear::linear_id_of_key;

#[test]
fn a_create_key_is_sent_as_its_v4_shaped_id_and_a_malformed_one_is_refused() {
    // A version-5 key, the version nibble set to 4 and the RFC 4122 variant kept: the format
    // Linear's published schema documents `IssueCreateInput.id` in.
    assert_eq!(
        linear_id_of_key("017656ef-4f62-53d5-ba4b-ae0bc4c07fa5").expect("a key"),
        "017656ef-4f62-43d5-ba4b-ae0bc4c07fa5"
    );
    assert_eq!(
        linear_id_of_key("C681C775-20FB-5D73-9F51-A6924CB0C252").expect("a key"),
        "c681c775-20fb-4d73-9f51-a6924cb0c252"
    );
    for malformed in [
        "",
        "not-a-uuid",
        "017656ef4f6253d5ba4bae0bc4c07fa5",
        "017656ef-4f62-53d5-ba4b-ae0bc4c07fz5",
    ] {
        assert!(linear_id_of_key(malformed).is_err(), "{malformed:?}");
    }
}
