//! A write held to the task's version, made as one step with checking it under the lock beside
//! the task's file — and a write this folder makes inside one it already holds, a rendering made
//! with its assets, checked under the lock it holds rather than waiting on itself.

use std::sync::Arc;

use onetaskgraph_plugin_api::{
    AssetWrite, HeldVersion, NativeId, Priority, SecretResolver, SourceError, SourceName,
    SourcePlugin, TaskSource,
};
use secrecy::SecretString;
use serde_json::json;

struct NoSecrets;
impl SecretResolver for NoSecrets {
    fn get(&self, _: &str) -> Option<SecretString> {
        None
    }
}

/// A folder holding one task, `T`.
fn folder() -> (tempfile::TempDir, Box<dyn TaskSource>) {
    let root = tempfile::tempdir().expect("a folder");
    std::fs::create_dir_all(root.path().join("tasks")).expect("the task folder");
    std::fs::write(
        root.path().join("tasks").join("T.md"),
        "---\ntitle: T\nstatus: todo\n---\nBody.\n",
    )
    .expect("a task");
    let source = onetaskgraph_local_md::Plugin
        .build(
            &SourceName::new("work").expect("a name"),
            &json!({ "root": root.path() }),
            &NoSecrets,
        )
        .expect("the folder builds");
    (root, source)
}

async fn version(source: &dyn TaskSource) -> String {
    source
        .get_task(&NativeId::from("T"))
        .await
        .expect("read")
        .and_then(|task| task.version)
        .expect("a version")
}

#[tokio::test]
async fn a_held_rendering_with_its_assets_is_checked_once_under_the_lock_it_holds() {
    let (root, source) = folder();
    assert!(source.holds_versions());
    // The lock beside the task's file, which a held write holds and lets go of.
    let lock = root.path().join("tasks").join("T.md.lock");
    let id = NativeId::from("T");
    let provenance = json!({"template": "t", "body_digest": "d"});
    let answers = std::collections::BTreeMap::new();
    let assets = AssetWrite::default();
    let render = |held: &str| {
        let held = Arc::new(HeldVersion::new(id.clone(), held.to_owned()));
        let call = source.set_task_rendering_with_assets(
            &id,
            "Rendered.\n",
            &provenance,
            &answers,
            &assets,
        );
        (Arc::clone(&held), HeldVersion::scope(held, call))
    };

    // At its version: it lands — a rendering write waiting on the lock its own held write holds
    // would end refused as held by another — and the lock is let go of.
    let read = version(source.as_ref()).await;
    let (held, call) = render(&read);
    let written = call.await.expect("rendered").expect("the task is there");
    assert!(!lock.exists(), "the lock is released");
    assert_eq!(written.content.as_deref(), Some("Rendered.\n"));
    assert_eq!(held.found(), None);
    let task = source.get_task(&id).await.expect("read").expect("held");
    assert_eq!(task.content.as_deref(), Some("Rendered.\n"));

    // At a version it has moved past: refused, writing nothing, naming the version it is at.
    let stale = read;
    let at = version(source.as_ref()).await;
    let (held, call) = render(&stale);
    let refused = call.await.expect_err("a stale version");
    assert!(
        matches!(refused, SourceError::Refused { .. }),
        "{refused:?}"
    );
    assert_eq!(held.found().as_deref(), Some(at.as_str()));
    assert_eq!(version(source.as_ref()).await, at, "nothing was written");
    assert!(!lock.exists(), "a refusal lets go of the lock too");

    // The next held write takes the lock again and lands.
    let next = Arc::new(HeldVersion::new(id.clone(), at));
    HeldVersion::scope(next, source.set_task_priority(&id, Priority::Low))
        .await
        .expect("written")
        .expect("the task is there");
    assert!(!lock.exists());
    let task = source.get_task(&id).await.expect("read").expect("held");
    assert_eq!(task.priority, Priority::Low);
}
