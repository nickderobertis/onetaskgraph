//! A repeat copy out of Linear skips a task only when its `lin1` token covers every part the
//! copy carries.
//!
//! A Linear task's token never covers its comments — no time moves for one added, edited or
//! deleted — and covers its forward edges only with the store's cache attached. So a copy
//! carrying comments, and any copy from a source with the cache off, reads such a task again
//! rather than skip it on an unchanged token, and carries what changed.

use serde_json::{Value, json};

use crate::cache::{Cached, dataset, walk};

/// Every task file the `notes` folder holds, joined.
fn notes(cached: &Cached) -> String {
    walk(&cached.sandbox.project().join("notes/tasks"))
        .iter()
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The outcome a copy reported for `linear:<native>`.
fn outcome<'a>(copied: &'a Value, native: &str) -> &'a Value {
    copied["items"]
        .as_array()
        .expect("a copy report")
        .iter()
        .find(|item| item["source"] == format!("linear:{native}"))
        .unwrap_or_else(|| panic!("the copy reports linear:{native}: {copied}"))
}

#[test]
fn a_comment_added_in_linear_is_carried_by_the_next_comments_copy_rather_than_skipped() {
    let cached = Cached::with(dataset(), &json!({"enabled": false}));
    let copy = [
        "task",
        "copy",
        "linear:L-1",
        "--to",
        "notes",
        "--comments",
        "--no-link",
        "--json",
    ];
    cached.ok(&copy);
    let before = cached
        .listed("task", "linear:L-1")
        .expect("L-1 carries a token");
    cached
        .workspace
        .add_comment("L-1", "a comment added in Linear after the copy");
    assert_eq!(
        cached.listed("task", "linear:L-1").as_deref(),
        Some(before.as_str()),
        "a comment moves no part of Linear's token"
    );
    let (again, _) = cached.ok(&copy);
    assert_ne!(
        outcome(&again, "L-1")["skipped"],
        true,
        "a task whose comments its token does not cover is never skipped: {again}"
    );
    assert!(
        notes(&cached).contains("a comment added in Linear after the copy"),
        "the new comment is carried: {}",
        notes(&cached)
    );
}

#[test]
fn a_relation_added_in_linear_is_carried_by_the_next_copy_from_a_source_with_no_cache() {
    let cached = Cached::with(dataset(), &json!({"enabled": false}));
    let copy = [
        "task",
        "copy",
        "linear:L-1",
        "--to",
        "notes",
        "--no-link",
        "--json",
    ];
    cached.ok(&copy);
    assert!(!notes(&cached).contains("linear:L-3"), "{}", notes(&cached));
    let before = cached
        .listed("task", "linear:L-1")
        .expect("L-1 carries a token");
    cached.workspace.relate("L-1", "L-3", "blocks");
    assert_eq!(
        cached.listed("task", "linear:L-1").as_deref(),
        Some(before.as_str()),
        "with no cache attached, a relation moves no part of Linear's token"
    );
    let (again, _) = cached.ok(&copy);
    assert_ne!(
        outcome(&again, "L-1")["skipped"],
        true,
        "a task whose edges its token does not cover is never skipped: {again}"
    );
    assert!(
        notes(&cached).contains("linear:L-3"),
        "the new edge is carried: {}",
        notes(&cached)
    );
}
