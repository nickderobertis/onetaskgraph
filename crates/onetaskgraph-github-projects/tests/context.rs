//! The shape of the write context this source hands the store's cache, held to the version
//! every context carries.
//!
//! A context is kept on a host and read back by whichever release runs next, which takes one of
//! its own version as one it can read. A change to what a context holds that keeps the version
//! would have a release misread another's — so each version is recorded beside a digest of the
//! shapes it publishes, append-only, and a shape changed without a new row fails here.

use onetaskgraph_github_projects::{CONTEXT_VERSION, context_shapes};

/// Every context version with the digest of the task and board context shapes it publishes.
const PUBLISHED_CONTEXTS: &[(u64, u64)] = &[(1, 0x4e7d_0ea5_fdec_1358)];

/// FNV-1a over `value`'s JSON, whose object keys serde_json keeps sorted.
fn digest(value: &serde_json::Value) -> u64 {
    serde_json::to_string(value)
        .expect("a schema renders")
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
        })
}

#[test]
fn the_context_version_moves_whenever_the_context_shape_does() {
    for (index, (version, recorded)) in PUBLISHED_CONTEXTS.iter().enumerate() {
        assert_eq!(*version, index as u64 + 1, "rows are the versions in order");
        assert!(
            !PUBLISHED_CONTEXTS[..index]
                .iter()
                .any(|(_, earlier)| earlier == recorded),
            "version {version} republishes an earlier shape"
        );
    }
    let (version, recorded) = PUBLISHED_CONTEXTS.last().expect("one row at least");
    assert_eq!(
        *version, CONTEXT_VERSION,
        "append a row for CONTEXT_VERSION"
    );
    let emitted = digest(&context_shapes());
    assert_eq!(
        emitted,
        *recorded,
        "the context shape changed: bump CONTEXT_VERSION and append ({}, {emitted:#018x}), or \
         put the shape back",
        CONTEXT_VERSION + 1
    );
}
