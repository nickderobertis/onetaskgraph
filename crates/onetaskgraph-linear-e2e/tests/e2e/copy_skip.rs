//! A repeat copy out of Linear skips a task only when its `lin1` token covers every part the
//! copy carries.
//!
//! A Linear task's token covers its comments and its forward edges only with the store's cache
//! attached — no time moves for a comment added, edited or deleted, nor for a relation added —
//! when it carries the digests of their first pages. So any copy from a source with the cache off
//! reads such a task again rather than skip it on an unchanged token, and carries what changed;
//! with it on, an unchanged task is skipped and a comment added moves the token, so the next
//! `--comments` copy carries it.

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
fn a_repeat_copy_by_query_skips_a_linear_task_only_where_its_token_covers_the_edges() {
    let query = [
        "task",
        "copy",
        "--source",
        "linear",
        "--to",
        "notes",
        "--no-link",
        "--json",
    ];
    let skipped = |copied: &Value| {
        copied["items"]
            .as_array()
            .expect("a copy report")
            .iter()
            .filter(|item| item["skipped"] == true)
            .count()
    };
    // With the cache on, a task's token carries its relations, so an unchanged repeat skips.
    let cached = Cached::new(dataset());
    cached.ok(&query);
    let (again, _) = cached.ok(&query);
    assert!(
        skipped(&again) > 0,
        "a covered, unchanged task is skipped: {again}"
    );

    // With it off, the token does not cover forward edges, so nothing is skipped — and a
    // relation added in Linear, which moves no part of that token, is carried.
    let uncached = Cached::with(dataset(), &json!({"enabled": false}));
    uncached.ok(&query);
    let (again, _) = uncached.ok(&query);
    assert_eq!(
        skipped(&again),
        0,
        "no task is skipped on a token blind to its edges: {again}"
    );
    let before = uncached
        .listed("task", "linear:L-1")
        .expect("L-1 carries a token");
    uncached.workspace.relate("L-1", "L-3", "blocks");
    assert_eq!(
        uncached.listed("task", "linear:L-1").as_deref(),
        Some(before.as_str()),
        "with no cache attached, a relation moves no part of Linear's token"
    );
    let (related, _) = uncached.ok(&query);
    assert_ne!(outcome(&related, "L-1")["skipped"], true, "{related}");
    assert!(
        notes(&uncached).contains("linear:L-3") || notes(&uncached).contains("notes:L-3"),
        "the new edge is carried: {}",
        notes(&uncached)
    );
}

#[test]
fn with_the_cache_on_an_unchanged_comments_copy_skips_and_a_comment_added_is_carried() {
    let cached = Cached::new(dataset());
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
    let (again, _) = cached.ok(&copy);
    assert_eq!(
        outcome(&again, "L-1")["skipped"],
        true,
        "an unchanged task whose token covers its comments is skipped: {again}"
    );
    let before = cached
        .listed("task", "linear:L-1")
        .expect("L-1 carries a token");
    cached
        .workspace
        .add_comment("L-1", "a comment added in Linear after the copy");
    assert_ne!(
        cached.listed("task", "linear:L-1").as_deref(),
        Some(before.as_str()),
        "with the cache on, a comment moves the token's comments part"
    );
    let (carried, _) = cached.ok(&copy);
    assert_ne!(outcome(&carried, "L-1")["skipped"], true, "{carried}");
    assert!(
        notes(&cached).contains("a comment added in Linear after the copy"),
        "the new comment is carried: {}",
        notes(&cached)
    );
}
