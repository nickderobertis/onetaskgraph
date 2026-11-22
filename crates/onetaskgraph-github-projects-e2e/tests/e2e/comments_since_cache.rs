//! `task comments --since` on a board whose source keeps the store's cache: a comment read once,
//! then edited by another process, is answered at its new version and with its new body by the
//! next read from before the edit — never as the cache last saw it.

use serde_json::Value;

use crate::cache::Cached;

/// Every comment `task comments board --since <since>` answers, through the binary.
fn since(cached: &Cached, since: &str) -> Vec<Value> {
    let (answer, _) = cached.ok(&["task", "comments", "board", "--since", since, "--json"]);
    answer["comments"].as_array().expect("comments").clone()
}

/// The one comment `id` among `comments`.
fn find<'a>(comments: &'a [Value], id: &str) -> &'a Value {
    comments
        .iter()
        .find(|comment| comment["id"] == id)
        .unwrap_or_else(|| panic!("{id} is answered: {comments:#?}"))
}

#[test]
fn a_comment_edited_in_another_process_is_answered_at_its_new_version() {
    let cached = Cached::new();
    let id = cached.board.add_comment("T-1", "first words");
    let from = "2000-01-01T00:00:00Z";
    // Read once — and its task's comments listed — so the cache holds what it saw.
    let read = since(&cached, from);
    let before = find(&read, &id).clone();
    assert_eq!(before["body"], "first words");
    cached.ok(&["task", "comment", "list", "board:T-1", "--json"]);

    // Another process edits it, through the binary.
    let body = cached.sandbox.project().join("edited.md");
    std::fs::write(&body, "second words").unwrap();
    cached.ok(&[
        "task",
        "comment",
        "edit",
        "board:T-1",
        &id,
        "--body-file",
        body.to_str().unwrap(),
        "--json",
    ]);

    let after = since(&cached, from);
    let edited = find(&after, &id);
    assert_eq!(edited["body"], "second words", "{edited:#}");
    assert_ne!(edited["version"], before["version"], "{edited:#}");
    assert_ne!(edited["updated_at"], before["updated_at"], "{edited:#}");
}
