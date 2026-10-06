//! Image assets as a **Rust caller** reaches them: create a task holding assets, copy it with
//! [`Engine::copy`], and read back what landed.
//!
//! The journeys that drive the same verbs as a user does are in
//! `crates/onetaskgraph/tests/e2e/assets.rs`. These are the library half the copy verb owes:
//! a folder of Markdown holding the assets, and an `in-memory` destination declaring them
//! native, which serves each at a URL as a hosted destination does — and which a copy that
//! cannot finish has to leave exactly as it found it.

use std::path::Path;

use onetaskgraph_core::{
    Body, Config, CopyAction, CopyItems, CopyRequest, CopyScope, Engine, EngineError, GlobalId,
    TaskCreate,
};
use onetaskgraph_plugin_api::{
    AssetName, AssetPayload, AssetUploads, NativeId, SecretResolver, SourceName, asset_sha256,
};
use secrecy::SecretString;
use serde_json::{Value, json};

struct NoSecrets;
impl SecretResolver for NoSecrets {
    fn get(&self, _var: &str) -> Option<SecretString> {
        None
    }
}

/// A folder of Markdown at `root` called `notes`, and an `in-memory` source called `into` whose
/// configuration is `into`.
fn engine(root: &Path, into: Value) -> Engine {
    let config = Config::from_document(json!({"sources": {
        "notes": {"plugin": "local-md", "config": {"root": root}},
        "into": {"plugin": "in-memory", "config": into},
    }}))
    .expect("a valid configuration");
    Engine::build(&config, &NoSecrets)
}

fn name(value: &str) -> AssetName {
    AssetName::new(value).expect("an asset name")
}

/// Create a task called `title` in `notes`, filed under `launch`, holding `shot.png`.
async fn create(engine: &Engine, title: &str, bytes: &[u8]) -> GlobalId {
    engine
        .create_task(&TaskCreate {
            source: SourceName::new("notes").expect("a name"),
            project: NativeId::from("launch"),
            title: title.to_owned(),
            body: Body::plain("![shot](./shot.png)\n"),
            status: None,
            labels: Vec::new(),
            repositories: Vec::new(),
            depends_on: Vec::new(),
            delivers: Vec::new(),
            metadata: Default::default(),
            assets: vec![AssetPayload::of(name("shot.png"), bytes.to_vec())],
        })
        .await
        .expect("the task is created")
        .task
        .id
}

fn copy_of(items: &[GlobalId], scope: CopyScope) -> CopyRequest {
    CopyRequest {
        items: CopyItems::new(items.to_vec()).expect("at least one item"),
        scope,
        destination: SourceName::new("into").expect("a name"),
        match_by: None,
        recreate: false,
        create: false,
        dry_run: false,
    }
}

/// The destination's copy of one task: its content and what it records it uploaded.
async fn landed(engine: &Engine, id: &GlobalId) -> (String, AssetUploads) {
    let task = engine
        .task(id)
        .await
        .expect("the task reads")
        .items
        .remove(0)
        .item;
    let uploads = AssetUploads::read(&task.metadata)
        .expect("a well-formed record")
        .expect("a record of what was uploaded");
    (task.content.unwrap_or_default(), uploads)
}

fn destination(action: &CopyAction) -> GlobalId {
    action
        .destination()
        .cloned()
        .unwrap_or_else(|| panic!("an item that landed, not {action:?}"))
}

#[tokio::test]
async fn a_copy_hands_a_serving_destination_the_assets_and_lands_its_rewritten_references() {
    let root = tempfile::tempdir().expect("a folder");
    let engine = engine(root.path(), json!({"capabilities": {"assets": "native"}}));
    let shot = vec![7_u8; 2048];
    let task = create(&engine, "Pictured", &shot).await;

    let report = engine
        .copy(&copy_of(std::slice::from_ref(&task), CopyScope::Tasks))
        .await
        .expect("the copy lands");
    let copied = destination(&report.items[0].action);
    let (content, uploads) = landed(&engine, &copied).await;
    let upload = &uploads.0[&name("shot.png")];
    assert_eq!(upload.sha256, asset_sha256(&shot));
    assert_eq!(content, format!("![shot]({})\n", upload.url));
    let held = engine
        .task_without_comments(&copied)
        .await
        .expect("the task reads")
        .assets
        .expect("the assets it holds");
    assert_eq!(held.len(), 1);
    assert!(
        held[0].path.is_none(),
        "a serving destination keeps no path"
    );

    // The same bytes again: nothing differs, so nothing is written.
    let again = engine
        .copy(&copy_of(std::slice::from_ref(&task), CopyScope::Tasks))
        .await
        .expect("the copy runs");
    assert!(matches!(
        again.items[0].action,
        CopyAction::Unchanged { .. }
    ));
}

#[tokio::test]
async fn a_copy_into_a_destination_that_stores_no_assets_is_refused_before_anything_is_written() {
    let root = tempfile::tempdir().expect("a folder");
    let engine = engine(root.path(), json!({}));
    let task = create(&engine, "Pictured", &[1, 2, 3]).await;
    let refused = engine
        .copy(&copy_of(std::slice::from_ref(&task), CopyScope::Tasks))
        .await
        .expect_err("refused");
    assert_eq!(
        refused,
        EngineError::AssetsUnsupported {
            name: "into".to_owned(),
            kind: "in-memory".to_owned(),
            record: task.to_string(),
            asset: "shot.png".to_owned(),
        }
    );
}

#[tokio::test]
async fn a_copy_that_cannot_finish_takes_back_what_it_created_and_puts_back_what_it_changed() {
    let root = tempfile::tempdir().expect("a folder");
    std::fs::create_dir_all(root.path().join("projects")).expect("a projects folder");
    std::fs::write(
        root.path().join("projects/launch.md"),
        "---\ntitle: Launch\nstatus: todo\n---\n",
    )
    .expect("a project");
    // The second task's update is applied and then refused, so a re-copy that changed the first
    // fails after the first has landed.
    let engine = engine(
        root.path(),
        json!({"capabilities": {"assets": "native", "half_written_titles": ["Second"]}}),
    );
    let first = create(&engine, "First", &[1; 64]).await;
    let second = create(&engine, "Second", &[2; 64]).await;
    let project = GlobalId::new(
        SourceName::new("notes").expect("a name"),
        NativeId::from("launch"),
    );
    let projects = CopyScope::Projects { tasks: true };
    let report = engine
        .copy(&copy_of(std::slice::from_ref(&project), projects.clone()))
        .await
        .expect("the first copy lands");
    let copied = report
        .items
        .iter()
        .find(|outcome| outcome.source == first)
        .map(|outcome| destination(&outcome.action))
        .expect("the first task landed");
    let before = landed(&engine, &copied).await;

    // Both tasks' pictures change at their source; the re-copy writes the first, then fails on
    // the second.
    for (task, bytes) in [(&first, [3; 64]), (&second, [4; 64])] {
        std::fs::write(
            root.path()
                .join(format!("tasks/{}.assets/shot.png", task.native)),
            bytes,
        )
        .expect("new bytes");
    }
    engine
        .copy(&copy_of(std::slice::from_ref(&project), projects))
        .await
        .expect_err("the second task's update is refused");

    // The first task holds exactly what it held before: its content, its record of what was
    // uploaded, and the bytes served at that record's URL.
    assert_eq!(landed(&engine, &copied).await, before);
    let held = engine
        .task_without_comments(&copied)
        .await
        .expect("the task reads")
        .assets
        .expect("the assets it holds");
    assert_eq!(held[0].sha256, asset_sha256(&[1; 64]));
    assert!(
        before.1.0[&name("shot.png")]
            .url
            .contains(&asset_sha256(&[1; 64])),
        "{:?}",
        before.1
    );
}
