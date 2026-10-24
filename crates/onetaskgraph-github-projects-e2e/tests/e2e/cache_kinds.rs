//! The store's local cache of a GitHub board's projects and documents, driven through the binary
//! against the loopback board, doubling only GitHub: what a pinned `project show` and `document
//! show` are answered from, what a listing leaves the cache holding, and which pins miss.
//!
//! The board's `gh3` token covers a project's and a document's own fields and a document's asset
//! manifest, so a pinned read the entry is at or after sends nothing; a newer pin, a pin the
//! board's ordering cannot place beside the entry — another facet, another relationship count —
//! a token from before `gh3`, and an item read too soon after its change to carry a token each
//! cost the one read an uncached show makes. Every journey counts the documents the board served
//! for each command, and reads the values back after the board changed, so nothing older than the
//! token a caller holds is ever served.

use std::path::{Path, PathBuf};

use onetaskgraph_github_projects::{DESIGN_TITLE_PREFIX, graphql};
use serde_json::{Value, json};

use crate::cache::Cached;

/// The one request a project or a document read by id sends: the issue by its own id.
const READ_BY_ID: &str = graphql::ISSUE;

/// `<kind> show <id>`, pinned to `pin` when one is given.
pub(crate) fn show(
    cached: &Cached,
    kind: &str,
    id: &str,
    pin: Option<&str>,
) -> (Value, Vec<String>) {
    let mut arguments = vec![kind, "show", id, "--json"];
    if let Some(pin) = pin {
        arguments.extend(["--pin", pin]);
    }
    cached.ok(&arguments)
}

/// The version one read of a project or a document reported.
pub(crate) fn version(shown: &Value) -> String {
    shown["items"][0]["item"]["version"]
        .as_str()
        .unwrap_or_else(|| panic!("the read gave the item a version: {shown}"))
        .to_owned()
}

/// The version `<kind> list` carries for `id` — the token a caller takes from a listing.
pub(crate) fn listed(cached: &Cached, kind: &str, id: &str) -> String {
    let (listing, _) = cached.ok(&[
        kind, "list", "--source", "board", "--json", "--limit", "100",
    ]);
    listing["items"]
        .as_array()
        .expect("a listing")
        .iter()
        .find(|item| item["id"] == id)
        .unwrap_or_else(|| panic!("{kind} list carries {id}: {listing}"))["item"]["version"]
        .as_str()
        .unwrap_or_else(|| panic!("{kind} list gave {id} a version: {listing}"))
        .to_owned()
}

/// Every entry file the cache holds under its `directory` — `projects-v4`, `documents-v4`,
/// `tasks` — of `dir`.
pub(crate) fn kind_entries(dir: &Path, directory: &str) -> Vec<PathBuf> {
    let Ok(listed) = std::fs::read_dir(dir.join(directory)) else {
        return Vec::new();
    };
    let mut entries: Vec<PathBuf> = listed
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect();
    entries.sort();
    entries
}

/// The one cache directory under the sandbox's cache home, or `None` before anything made one.
fn cache_dir(cached: &Cached) -> Option<PathBuf> {
    let root = cached.cache_home.join("onetaskgraph");
    std::fs::read_dir(root)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .next()
}

#[test]
fn a_pinned_project_read_is_answered_from_its_entry_and_a_newer_pin_from_the_board() {
    let cached = Cached::new();
    // Cold: the one read by id, which fills the entry.
    let (first, sent) = show(&cached, "project", "board:P-1", None);
    assert_eq!(sent, [READ_BY_ID], "a cold read is the one read by id");
    let held = version(&first);
    assert!(held.starts_with(onetaskgraph_github_projects::change_token::PREFIX));
    assert_eq!(
        kind_entries(&cache_dir(&cached).expect("a cache"), "projects-v4").len(),
        1
    );

    // Pinned to the version it holds: nothing sent, and the very project the board answered.
    let (pinned, sent) = show(&cached, "project", "board:P-1", Some(&held));
    assert_eq!(sent, Vec::<String>::new(), "a hit sends nothing");
    assert_eq!(pinned["items"], first["items"]);

    // A person renames it; the caller takes the new token from a listing, and the read pinned
    // to it is the one read — answering with the rename, never the entry.
    cached
        .board
        .edit_title("P-1", "Engine, renamed by a person");
    let newer = listed(&cached, "project", "board:P-1");
    assert_ne!(newer, held);
    let (current, sent) = show(&cached, "project", "board:P-1", Some(&newer));
    assert_eq!(sent, [READ_BY_ID], "a newer pin misses");
    assert_eq!(
        current["items"][0]["item"]["title"],
        "Engine, renamed by a person"
    );
    // That read refreshed the entry, so the same pin is now a hit.
    let (again, sent) = show(&cached, "project", "board:P-1", Some(&newer));
    assert_eq!(sent, Vec::<String>::new());
    assert_eq!(again["items"], current["items"]);

    // Unpinned, the read goes to the board as it always did.
    let (_, sent) = show(&cached, "project", "board:P-1", None);
    assert_eq!(sent, [READ_BY_ID]);
}

#[test]
fn a_pinned_document_read_serves_its_fields_and_manifest_from_its_entry() {
    let cached = Cached::new();
    let (first, sent) = show(&cached, "document", "board:D-1", None);
    assert_eq!(sent, [READ_BY_ID], "a cold read is the one read by id");
    let held = version(&first);

    let (pinned, sent) = show(&cached, "document", "board:D-1", Some(&held));
    assert_eq!(
        sent,
        Vec::<String>::new(),
        "fields and manifest, nothing sent"
    );
    assert_eq!(pinned["items"], first["items"]);
    assert_eq!(pinned["assets"], first["assets"]);

    cached
        .board
        .edit_body("D-1", "the engine core, reviewed again");
    let newer = listed(&cached, "document", "board:D-1");
    let (current, sent) = show(&cached, "document", "board:D-1", Some(&newer));
    assert_eq!(sent, [READ_BY_ID], "a newer pin misses");
    assert_eq!(
        current["items"][0]["item"]["content"],
        "the engine core, reviewed again"
    );
}

#[test]
fn a_pin_the_board_cannot_order_beside_the_entry_is_the_one_read() {
    let cached = Cached::new();
    for (kind, id) in [("project", "board:P-1"), ("document", "board:D-1")] {
        let (first, _) = show(&cached, kind, id, None);
        let held = version(&first);
        // Another repository: the board orders no two tokens naming different ones.
        let elsewhere = held.replace("nickderobertis/onetaskgraph", "acme/other");
        assert_ne!(elsewhere, held);
        let (_, sent) = show(&cached, kind, id, Some(&elsewhere));
        assert_eq!(sent, [READ_BY_ID], "{kind}: an unordered pin");
        // A token from before gh3 — what 0.6.0 gave — is never ordered either.
        let rest = held
            .strip_prefix(onetaskgraph_github_projects::change_token::PREFIX)
            .expect("a gh3 token");
        let parts: Vec<&str> = rest.splitn(6, '|').collect();
        let gh2 = format!("gh2|{}|{}", parts[..4].join("|"), parts[5]);
        let (_, sent) = show(&cached, kind, id, Some(&gh2));
        assert_eq!(sent, [READ_BY_ID], "{kind}: a gh2 pin");
        // And a value that is no token at all.
        let (_, sent) = show(&cached, kind, id, Some("not-a-token"));
        assert_eq!(sent, [READ_BY_ID], "{kind}: no token");
        // The entry still answers the pin it holds.
        let (_, sent) = show(&cached, kind, id, Some(&held));
        assert_eq!(sent, Vec::<String>::new(), "{kind}: the held pin");
    }
}

#[test]
fn an_item_read_too_soon_after_its_change_gives_no_token_and_fills_no_entry() {
    let cached = Cached::new();
    // Read in the very second of a change: the board withholds the token, so the read has
    // nothing to key an entry on.
    cached.board.read_as_it_changes(true);
    for (kind, id) in [("project", "board:P-2"), ("document", "board:D-2")] {
        let (shown, sent) = show(&cached, kind, id, None);
        assert_eq!(sent, [READ_BY_ID]);
        assert!(
            shown["items"][0]["item"].get("version").is_none(),
            "{kind}: no token while it settles: {shown}"
        );
    }
    let dir = cache_dir(&cached);
    for directory in ["projects-v4", "documents-v4"] {
        assert!(
            dir.as_deref()
                .is_none_or(|dir| kind_entries(dir, directory).is_empty()),
            "no {directory} entry without a token"
        );
    }
    cached.board.read_as_it_changes(false);
    // Settled, a read gives the token and fills the entry, and a read pinned to it is a hit.
    let (shown, _) = show(&cached, "project", "board:P-2", None);
    let (_, sent) = show(&cached, "project", "board:P-2", Some(&version(&shown)));
    assert_eq!(sent, Vec::<String>::new());
}

#[test]
fn a_listing_of_any_kind_fills_no_entry() {
    let cached = Cached::new();
    for kind in ["task", "project", "document"] {
        cached.ok(&[
            kind, "list", "--source", "board", "--json", "--limit", "100",
        ]);
    }
    let dir = cache_dir(&cached);
    for directory in ["tasks", "projects-v4", "documents-v4", "edges-v4"] {
        assert!(
            dir.as_deref()
                .is_none_or(|dir| kind_entries(dir, directory).is_empty()),
            "a listing fills no {directory} entry"
        );
    }
}

#[test]
fn an_entry_a_release_before_gh3_left_is_read_again_rather_than_served() {
    let cached = Cached::new();
    let (first, _) = show(&cached, "project", "board:P-1", None);
    let held = version(&first);
    let dir = cache_dir(&cached).expect("a cache");
    let entry = kind_entries(&dir, "projects-v4").remove(0);
    // What 0.6.0 would have keyed it on: the gh2 spelling of the same parts.
    let rest = held
        .strip_prefix(onetaskgraph_github_projects::change_token::PREFIX)
        .expect("a gh3 token");
    let parts: Vec<&str> = rest.splitn(6, '|').collect();
    let gh2 = format!("gh2|{}|{}", parts[..4].join("|"), parts[5]);
    let mut written: Value =
        serde_json::from_slice(&std::fs::read(&entry).expect("the entry")).expect("JSON");
    written["project"]["version"] = json!(gh2);
    written["project"]["title"] = json!("a title only the old entry holds");
    std::fs::write(&entry, serde_json::to_vec(&written).unwrap()).expect("the entry");

    let (current, sent) = show(&cached, "project", "board:P-1", Some(&held));
    assert_eq!(sent, [READ_BY_ID], "a gh2 entry is never served");
    assert_eq!(current["items"][0]["item"]["title"], "Engine");
}

#[test]
fn an_item_whose_labels_go_on_past_one_page_is_refused_and_leaves_the_entry_at_the_version_held() {
    let cached = Cached::new();
    let (first, _) = show(&cached, "document", "board:D-1", None);
    let held = version(&first);
    // More labels than one page holds: no read spells a token from the first page alone — the
    // plugin refuses the incomplete read — so no listing hands the caller one to pin, and the
    // entry at the version the caller held keeps answering only that version.
    cached.board.report_more_labels("D-1", true);
    let output = cached.run_with(&["document", "show", "board:D-1", "--json"], None);
    assert_ne!(
        output.status.code(),
        Some(0),
        "an incomplete page is refused"
    );
    let listing = cached.run_with(
        &[
            "document", "list", "--source", "board", "--json", "--limit", "100",
        ],
        None,
    );
    assert_ne!(
        listing.status.code(),
        Some(0),
        "a listing reaching it is refused too, so it hands the caller no token"
    );
    assert!(
        crate::common::stderr(&listing).contains("labels"),
        "{}",
        crate::common::stderr(&listing)
    );
    let dir = cache_dir(&cached).expect("a cache");
    let entry: Value = serde_json::from_slice(
        &std::fs::read(kind_entries(&dir, "documents-v4").remove(0)).expect("the entry"),
    )
    .expect("JSON");
    assert_eq!(
        entry["document"]["version"], held,
        "nothing was kept from the incomplete page"
    );
}

/// What one command answered, as a reader compares two answers: its exit code, its JSON
/// without the `cache` member — which says only where the answer came from — and what it said
/// on stderr.
fn answer(cached: &Cached, arguments: &[&str]) -> (Option<i32>, Value, String) {
    let output = cached.run_with(arguments, None);
    let mut printed: Value =
        serde_json::from_str(&crate::common::stdout(&output)).unwrap_or(Value::Null);
    if let Some(object) = printed.as_object_mut() {
        object.remove("cache");
    }
    (
        output.status.code(),
        printed,
        crate::common::stderr(&output),
    )
}

/// `arguments` answered with the cache on, then again with it disabled for that one command:
/// the two answers, which a journey holds equal.
fn on_and_off(cached: &Cached, arguments: &[&str]) -> [(Option<i32>, Value, String); 2] {
    let on = answer(cached, arguments);
    let mut off_arguments = vec!["--set", "sources.board.cache.enabled=false"];
    off_arguments.extend_from_slice(arguments);
    let off = answer(cached, &off_arguments);
    [on, off]
}

/// One change a person makes on the board, which moves no time the token carries.
type Change = (&'static str, Box<dyn Fn(&Cached)>);

/// A change to one item, named, with the two kinds or the item it reaches.
type Case = (
    &'static str,
    &'static str,
    &'static str,
    Box<dyn Fn(&Cached)>,
);

/// The changes `spike-coverage` measured a `gh2` token missing, each made to the board as a
/// person makes it: a label renamed and recoloured, a board field option renamed.
fn facet_changes(label: &'static str, option: &'static str) -> Vec<Change> {
    vec![
        (
            "a label renamed",
            Box::new(move |cached: &Cached| cached.board.rename_label(label, "renamed")),
        ),
        (
            "a label recoloured",
            Box::new(move |cached: &Cached| cached.board.recolour_label(label, "ff0000")),
        ),
        (
            "a board field option renamed",
            Box::new(move |cached: &Cached| {
                cached
                    .board
                    .rename_status_option(option, &format!("{option}, renamed"));
            }),
        ),
    ]
}

#[test]
fn a_label_or_option_renamed_or_recoloured_is_read_again_for_a_task_a_project_and_a_document() {
    for (kind, id, label, option) in [
        ("task", "board:T-3", "L-1", "Todo"),
        ("project", "board:P-1", "L-3", "Doing"),
        ("document", "board:D-1", "L-1", "Todo"),
    ] {
        for (change, make) in facet_changes(label, option) {
            let cached = Cached::new();
            // Filled by a pinned read, as a caller holding the listing's token makes it.
            let held = listed(&cached, kind, id);
            show(&cached, kind, id, Some(&held));
            let (_, sent) = show(&cached, kind, id, Some(&held));
            assert_eq!(sent, Vec::<String>::new(), "{kind} {change}: filled");

            make(&cached);
            let current = listed(&cached, kind, id);
            assert_ne!(current, held, "{kind} {change}: the token moves");
            let (shown, sent) = show(&cached, kind, id, Some(&current));
            assert!(
                !sent.is_empty(),
                "{kind} {change}: a pin to the new token is never the entry"
            );
            let [on, off] = on_and_off(&cached, &[kind, "show", id, "--pin", &current, "--json"]);
            assert_eq!(on, off, "{kind} {change}: what an uncached read answers");
            let item = &shown["items"][0]["item"];
            match change {
                "a label renamed" => assert_eq!(item["labels"][0]["name"], "renamed"),
                "a label recoloured" => assert_eq!(item["labels"][0]["color"], "ff0000"),
                _ if kind != "document" => assert_eq!(
                    item["status"]["name"],
                    format!("{option}, renamed"),
                    "{kind}: the renamed option"
                ),
                // A document has no status to report the option by; what holds it is that the
                // read went to the board.
                _ => {}
            }
        }
    }
}

#[test]
fn a_far_end_deleted_lowers_the_count_and_the_pinned_read_is_never_the_entry() {
    // Each item, related to a scratch far end through a relationship GitHub records an event
    // for, then that far end deleted — which takes the event off the item's timeline.
    for (kind, id, far, relate) in [
        ("task", "board:T-3", "T-4", "depends"),
        ("project", "board:P-1", "T-3", "depends"),
        ("document", "board:D-3", "P-2", "filed"),
    ] {
        let cached = Cached::new();
        let native = id.strip_prefix("board:").unwrap();
        match relate {
            "depends" => cached.board.block(native, &[far.to_owned()]),
            _ => cached.board.file_under(native, Some(far)),
        }
        let held = listed(&cached, kind, id);
        show(&cached, kind, id, Some(&held));
        let (_, sent) = show(&cached, kind, id, Some(&held));
        assert_eq!(sent, Vec::<String>::new(), "{kind}: filled");

        cached.board.delete_issue(far);
        let current = listed(&cached, kind, id);
        assert_ne!(
            current, held,
            "{kind}: the far end's deletion moves the token"
        );
        assert_eq!(
            onetaskgraph_github_projects::change_token::order(&held, &current),
            None,
            "{kind}: and the held token is unordered against it, never later"
        );
        let (shown, sent) = show(&cached, kind, id, Some(&current));
        assert!(
            !sent.is_empty(),
            "{kind}: the pinned read goes to the board"
        );
        let [on, off] = on_and_off(&cached, &[kind, "show", id, "--pin", &current, "--json"]);
        assert_eq!(on, off, "{kind}: what an uncached read answers");
        if kind == "document" {
            assert!(shown["items"][0]["item"]["project"].is_null(), "{shown}");
        }
        if kind == "task" {
            let [on, off] = on_and_off(&cached, &["task", "deps", id, "--json"]);
            assert_eq!(on, off, "the task's edges are what the board holds");
            assert!(
                !on.1.to_string().contains(&format!("board:{far}")),
                "the deleted far end is no edge: {}",
                on.1
            );
        }
    }
}

#[test]
fn an_items_own_kind_change_is_never_answered_with_the_entry_of_its_old_kind() {
    // (what changes its kind, the kind it read as, the kind it becomes, the change)
    let cached = Cached::new();
    let flips: Vec<Case> = vec![
        (
            "a task gaining a sub-issue",
            "task",
            "project",
            Box::new(|cached: &Cached| cached.board.file_under("T-1", Some("T-3"))),
        ),
        (
            "a project losing its last sub-issue",
            "project",
            "task",
            Box::new(|cached: &Cached| cached.board.file_under("T-1", Some("P-1"))),
        ),
        (
            "a task retitled into a document",
            "task",
            "document",
            Box::new(|cached: &Cached| {
                cached
                    .board
                    .edit_title("T-3", &format!("{DESIGN_TITLE_PREFIX}Gamma"))
            }),
        ),
    ];
    for (change, was, becomes, make) in flips {
        // Filled by a pinned read of the kind it is.
        let held = listed(&cached, was, "board:T-3");
        show(&cached, was, "board:T-3", Some(&held));
        let (_, sent) = show(&cached, was, "board:T-3", Some(&held));
        assert_eq!(sent, Vec::<String>::new(), "{change}: filled as a {was}");

        make(&cached);
        // The caller's current token, from a listing of the kind it became.
        let current = listed(&cached, becomes, "board:T-3");
        for kind in [was, becomes] {
            let arguments = [kind, "show", "board:T-3", "--pin", &current, "--json"];
            let [on, off] = on_and_off(&cached, &arguments);
            assert_eq!(
                on, off,
                "{change}: `{kind} show` pinned to the new token answers what an uncached one does"
            );
            if kind == was {
                assert_ne!(
                    on.0,
                    Some(0),
                    "{change}: it is no {was} now, so the read is refused: {on:?}"
                );
            } else {
                assert_eq!(on.0, Some(0), "{change}: it reads as a {becomes}: {on:?}");
            }
        }
    }
}

#[test]
fn a_far_ends_kind_flip_or_retitle_into_a_document_is_read_from_the_board_in_every_edge() {
    // (the near item, what changes its far end, the change)
    let cases: Vec<Case> = vec![
        (
            "task",
            "board:T-4",
            "its far end T-3 gaining a sub-issue, so becoming a project",
            Box::new(|cached: &Cached| cached.board.file_under("T-1", Some("T-3"))),
        ),
        (
            "task",
            "board:T-4",
            "its far end T-2 retitled into a document",
            Box::new(|cached: &Cached| {
                cached
                    .board
                    .edit_title("T-2", &format!("{DESIGN_TITLE_PREFIX}Beta"))
            }),
        ),
        (
            "project",
            "board:P-1",
            "its far end P-2 retitled into a document",
            Box::new(|cached: &Cached| {
                cached
                    .board
                    .edit_title("P-2", &format!("{DESIGN_TITLE_PREFIX}Docs"))
            }),
        ),
    ];
    for (kind, id, change, make) in cases {
        let cached = Cached::new();
        // T-4 depends on T-2 and on the top-level T-3; P-1 depends on P-2.
        cached.board.block("T-4", &["T-3".to_owned()]);
        let held = listed(&cached, kind, id);
        show(&cached, kind, id, Some(&held));
        let arguments = [kind, "deps", id, "--json", "--limit", "100"];
        let (before, _) = cached.ok(&arguments);

        // Nothing here moves the near item's own token: what changes is the far end.
        make(&cached);
        let current = listed(&cached, kind, id);
        assert_eq!(current, held, "{id}, {change}: its own token stays");
        let (_, sent) = show(&cached, kind, id, Some(&current));
        assert_eq!(
            sent,
            Vec::<String>::new(),
            "{id}, {change}: its own fields are still its entry's"
        );
        let served = cached.board.served().len();
        let [on, off] = on_and_off(&cached, &arguments);
        assert!(
            cached.board.served().len() > served + 1,
            "{id}, {change}: its forward edges are read from the board, cache on and off alike"
        );
        assert_eq!(
            on, off,
            "{id}, {change}: the edges answered are what an uncached read answers"
        );
        assert_ne!(on.1, before, "{id}, {change}: and they show the change");
        if change.contains("project") {
            assert!(
                on.1.to_string().contains("\"project\""),
                "{id}: the far end reads as the project it became: {}",
                on.1
            );
        } else {
            assert_ne!(on.0, Some(0), "{id}, {change}: refused, as uncached");
            assert!(on.2.contains("document"), "{}", on.2);
        }
    }
}

#[cfg(unix)]
#[test]
fn an_entry_the_cache_directory_will_not_replace_is_served_to_no_pin_after_its_item_changed() {
    use std::os::unix::fs::PermissionsExt;
    // (the kind, its id, its native id, its entries' directory, the title a person types on
    // the board, the title a read reports)
    for (kind, id, native, directory, typed, renamed) in [
        (
            "project",
            "board:P-1",
            "P-1",
            "projects-v4",
            "Engine, renamed by a person".to_owned(),
            "Engine, renamed by a person",
        ),
        (
            "document",
            "board:D-1",
            "D-1",
            "documents-v4",
            format!("{DESIGN_TITLE_PREFIX}Design, renamed by a person"),
            "Design, renamed by a person",
        ),
    ] {
        let cached = Cached::new();
        let (first, _) = show(&cached, kind, id, None);
        let held = version(&first);
        cached.board.edit_title(native, &typed);
        let newer = listed(&cached, kind, id);
        // The directory takes no new file, so the read pinned to the newer version cannot keep
        // what it read — and the entry it would have replaced is emptied rather than left.
        let entries = cache_dir(&cached).expect("a cache").join(directory);
        std::fs::set_permissions(&entries, std::fs::Permissions::from_mode(0o555)).unwrap();
        let (current, sent) = show(&cached, kind, id, Some(&newer));
        let (older, sent_older) = show(&cached, kind, id, Some(&held));
        std::fs::set_permissions(&entries, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(sent, [READ_BY_ID], "{kind}: a newer pin misses");
        assert_eq!(current["items"][0]["item"]["title"], renamed);
        assert_eq!(
            sent_older,
            [READ_BY_ID],
            "{kind}: the entry at the older version is no longer served, even to its own pin"
        );
        assert_eq!(
            older["items"][0]["item"]["title"], renamed,
            "{kind}: the read is the board's, never the entry it could not replace"
        );
    }
}
