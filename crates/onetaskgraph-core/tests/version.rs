//! The store's ordering of two change tokens, driven over tokens the plugins themselves
//! spell — a GitHub board's from its three parts, a folder of Markdown's from a real task
//! file read through the plugin — so a change to either spelling is a change this sees.

use chrono::{TimeZone as _, Utc};
use onetaskgraph_core::GlobalId;
use onetaskgraph_core::version::{VersionOrder, Versioned, compare};
use onetaskgraph_github_projects::change_token as github;

fn id(text: &str) -> GlobalId {
    text.parse().expect("a qualified id")
}

fn of<'a>(item: &'a GlobalId, version: &'a str) -> Versioned<'a> {
    Versioned { item, version }
}

#[test]
fn two_equal_versions_are_equal_and_a_change_orders_them() {
    let item = id("board:I_1");
    let at = |second| Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, second).unwrap();
    let before = github::spell(at(1), at(1), 2);
    assert_eq!(
        compare(of(&item, &before), of(&item, &before)),
        VersionOrder::Equal
    );
    for after in [
        github::spell(at(2), at(1), 3),
        github::spell(at(2), at(1), 2),
        github::spell(at(1), at(1), 1),
        github::spell(at(1), at(2), 2),
    ] {
        assert_eq!(
            compare(of(&item, &after), of(&item, &before)),
            VersionOrder::Later
        );
        assert_eq!(
            compare(of(&item, &before), of(&item, &after)),
            VersionOrder::Earlier
        );
    }
}

#[test]
fn versions_of_other_items_other_sources_and_unparseable_values_are_incomparable() {
    let at = Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap();
    let token = github::spell(at, at, 0);
    let (one, other, elsewhere) = (id("board:I_1"), id("board:I_2"), id("other:I_1"));
    assert_eq!(
        compare(of(&one, &token), of(&other, &token)),
        VersionOrder::Incomparable
    );
    assert_eq!(
        compare(of(&one, &token), of(&elsewhere, &token)),
        VersionOrder::Incomparable
    );
    for unreadable in ["", "opaque", "gh1|x|y|z", "md1|not-hex"] {
        assert_eq!(
            compare(of(&one, unreadable), of(&one, unreadable)),
            VersionOrder::Incomparable,
            "{unreadable:?}"
        );
        assert_eq!(
            compare(of(&one, unreadable), of(&one, &token)),
            VersionOrder::Incomparable
        );
    }
}

#[tokio::test]
async fn two_digests_of_one_task_file_are_equal_until_it_changes_and_then_incomparable() {
    use onetaskgraph_plugin_api::{NativeId, SecretResolver, SourceName, SourcePlugin as _};
    let root = tempfile::tempdir().unwrap();
    let tasks = root.path().join("tasks");
    std::fs::create_dir_all(&tasks).unwrap();
    let file = tasks.join("a.md");
    std::fs::write(&file, "---\ntitle: A\nstatus: todo\n---\nBody.\n").unwrap();
    struct NoSecrets;
    impl SecretResolver for NoSecrets {
        fn get(&self, _var: &str) -> Option<secrecy::SecretString> {
            None
        }
    }
    let source = onetaskgraph_local_md::Plugin
        .build(
            &SourceName::new("folder".to_owned()).unwrap(),
            &serde_json::json!({ "root": root.path() }),
            &NoSecrets,
        )
        .unwrap();
    let read = || async {
        source
            .get_task(&NativeId("a".to_owned()))
            .await
            .unwrap()
            .unwrap()
            .version
            .expect("a folder of Markdown gives every task a version")
    };
    let item = id("folder:a");
    let first = read().await;
    let again = read().await;
    assert_eq!(
        compare(of(&item, &first), of(&item, &again)),
        VersionOrder::Equal
    );
    std::fs::write(&file, "---\ntitle: A\nstatus: done\n---\nBody.\n").unwrap();
    let changed = read().await;
    assert_ne!(first, changed);
    assert_eq!(
        compare(of(&item, &changed), of(&item, &first)),
        VersionOrder::Incomparable
    );
}
