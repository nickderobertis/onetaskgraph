//! The shape of what the store's local cache writes, held to the schema number every entry
//! carries.
//!
//! The cache directory is shared by every release on a host, and a reader trusts an entry of
//! its own schema number as one it can read. So a change to what an entry holds that keeps the
//! number would have one release read another's entries as its own — the one failure a schema
//! number exists to prevent. Each number is recorded beside a digest of the shapes it publishes;
//! the table is append-only, and a shape changed without a new row fails here.

use onetaskgraph_core::cache::{ENTRY_SCHEMA, entry_shapes};

include!("support/bundle_digest.rs");

/// Every entry schema number with the digest of the task and board entry shapes it publishes.
const PUBLISHED_ENTRIES: &[(u32, u64)] = &[(1, 0x9da1_0712_ef55_81f8)];

#[test]
fn the_entry_schema_number_moves_whenever_the_entry_shape_does() {
    for (index, (version, _)) in PUBLISHED_ENTRIES.iter().enumerate() {
        assert_eq!(
            *version as usize,
            index + 1,
            "rows are the numbers in order"
        );
        assert!(
            !PUBLISHED_ENTRIES[..index]
                .iter()
                .any(|(_, earlier)| *earlier == PUBLISHED_ENTRIES[index].1),
            "number {version} republishes an earlier shape"
        );
    }
    let (version, recorded) = PUBLISHED_ENTRIES.last().expect("one row at least");
    assert_eq!(
        *version, ENTRY_SCHEMA,
        "ENTRY_SCHEMA is {ENTRY_SCHEMA} and the last row records {version}: append a row"
    );
    let emitted = digest(&entry_shapes());
    assert_eq!(
        emitted,
        *recorded,
        "the entry shape changed: bump ENTRY_SCHEMA and append ({}, {emitted:#018x}), or put \
         the shape back",
        ENTRY_SCHEMA + 1
    );
}
