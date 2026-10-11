//! The engine's ordering of two change tokens, asked of the source that spelled them, driven
//! over tokens the plugins themselves spell — a GitHub board's from its three parts, a folder of
//! Markdown's from a real task file read through the engine — so a change to either spelling is
//! a change this sees.

use std::path::Path;

use chrono::{TimeZone as _, Utc};
use onetaskgraph_core::version::{VersionOrder, Versioned};
use onetaskgraph_core::{Config, Engine, GlobalId};
use onetaskgraph_github_projects::change_token as github;
use onetaskgraph_plugin_api::SecretResolver;
use secrecy::SecretString;
use serde_json::json;

/// Answers every credential a source names, so a board builds without reaching GitHub: an
/// ordering asks the source nothing.
struct AnyCredential;

impl SecretResolver for AnyCredential {
    fn get(&self, _var: &str) -> Option<SecretString> {
        Some(SecretString::from("unused".to_owned()))
    }
}

/// An engine over a board, a folder of Markdown at `root` and an in-memory source.
fn engine(root: &Path) -> Engine {
    let config = Config::from_document(json!({"sources": {
        "board": {"plugin": "github-projects",
                  "config": {"owner": "someone", "project_number": 1}},
        "folder": {"plugin": "local-md", "config": {"root": root}},
        "memory": {"plugin": "in-memory", "config": {}},
    }}))
    .expect("a valid configuration");
    Engine::build(&config, &AnyCredential)
}

fn id(text: &str) -> GlobalId {
    text.parse().expect("a qualified id")
}

fn of<'a>(item: &'a GlobalId, version: &'a str) -> Versioned<'a> {
    Versioned { item, version }
}

#[test]
fn two_equal_versions_are_equal_and_a_change_orders_them() {
    let root = tempfile::tempdir().unwrap();
    let engine = engine(root.path());
    let item = id("board:I_1");
    let at = |second| Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, second).unwrap();
    let before = github::spell(at(1), at(1), 2);
    assert_eq!(
        engine.compare_versions(of(&item, &before), of(&item, &before)),
        VersionOrder::Equal
    );
    for after in [
        github::spell(at(2), at(1), 3),
        github::spell(at(2), at(1), 2),
        github::spell(at(1), at(1), 1),
        github::spell(at(1), at(2), 2),
    ] {
        assert_eq!(
            engine.compare_versions(of(&item, &after), of(&item, &before)),
            VersionOrder::Later
        );
        assert_eq!(
            engine.compare_versions(of(&item, &before), of(&item, &after)),
            VersionOrder::Earlier
        );
    }
}

#[test]
fn versions_of_other_items_unbuilt_sources_and_unparseable_values_are_incomparable() {
    let root = tempfile::tempdir().unwrap();
    let engine = engine(root.path());
    let at = Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap();
    let token = github::spell(at, at, 0);
    let (one, other, elsewhere) = (id("board:I_1"), id("board:I_2"), id("folder:I_1"));
    assert_eq!(
        engine.compare_versions(of(&one, &token), of(&other, &token)),
        VersionOrder::Incomparable
    );
    assert_eq!(
        engine.compare_versions(of(&one, &token), of(&elsewhere, &token)),
        VersionOrder::Incomparable
    );
    let nowhere = id("nowhere:I_1");
    assert_eq!(
        engine.compare_versions(of(&nowhere, &token), of(&nowhere, &token)),
        VersionOrder::Incomparable,
        "a source this engine does not hold orders nothing"
    );
    for unreadable in ["", "opaque", "gh1|x|y|z", "md1|not-hex"] {
        assert_eq!(
            engine.compare_versions(of(&one, unreadable), of(&one, unreadable)),
            VersionOrder::Incomparable,
            "{unreadable:?}"
        );
        assert_eq!(
            engine.compare_versions(of(&one, unreadable), of(&one, &token)),
            VersionOrder::Incomparable
        );
    }
    // A board's token, handed to the folder for one of its own items, is no token of the folder.
    assert_eq!(
        engine.compare_versions(of(&elsewhere, &token), of(&elsewhere, &token)),
        VersionOrder::Incomparable
    );
}

#[test]
fn a_source_keeping_the_default_orders_by_equality_alone() {
    let root = tempfile::tempdir().unwrap();
    let engine = engine(root.path());
    let item = id("memory:T-1");
    assert_eq!(
        engine.compare_versions(of(&item, "v2"), of(&item, "v2")),
        VersionOrder::Equal
    );
    assert_eq!(
        engine.compare_versions(of(&item, "v2"), of(&item, "v1")),
        VersionOrder::Incomparable
    );
}

#[tokio::test]
async fn two_digests_of_one_task_file_are_equal_until_it_changes_and_then_incomparable() {
    let root = tempfile::tempdir().unwrap();
    let tasks = root.path().join("tasks");
    std::fs::create_dir_all(&tasks).unwrap();
    let file = tasks.join("a.md");
    std::fs::write(&file, "---\ntitle: A\nstatus: todo\n---\nBody.\n").unwrap();
    let engine = engine(root.path());
    let item = id("folder:a");
    let read = || async {
        engine
            .task(&item)
            .await
            .expect("the folder answers")
            .items
            .remove(0)
            .item
            .version
            .expect("a folder of Markdown gives every task a version")
    };
    let first = read().await;
    let again = read().await;
    assert_eq!(
        engine.compare_versions(of(&item, &first), of(&item, &again)),
        VersionOrder::Equal
    );
    std::fs::write(&file, "---\ntitle: A\nstatus: done\n---\nBody.\n").unwrap();
    let changed = read().await;
    assert_ne!(first, changed);
    assert_eq!(
        engine.compare_versions(of(&item, &changed), of(&item, &first)),
        VersionOrder::Incomparable
    );
}
