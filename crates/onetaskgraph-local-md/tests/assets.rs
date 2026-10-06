//! A record's image assets, kept in a directory beside the record's own file.
//!
//! Every test drives the real plugin over a real folder and asserts on what a later read
//! answers and on the files the folder holds: where the bytes are is the promise
//! `docs/local-md.md` makes, not only what a read reports.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use onetaskgraph_plugin_api::{
    AssetName, AssetPayload, AssetWrite, Document, ItemWrite, NativeId, Priority, SecretResolver,
    SourceError, SourceName, SourcePlugin, Status, StatusCategory, Task, TaskSource, asset_sha256,
};
use secrecy::SecretString;
use serde_json::json;

struct NoSecrets;
impl SecretResolver for NoSecrets {
    fn get(&self, _: &str) -> Option<SecretString> {
        None
    }
}

/// An empty folder named `work`.
fn folder() -> (tempfile::TempDir, Box<dyn TaskSource>) {
    let root = tempfile::tempdir().expect("temporary notes");
    let source = onetaskgraph_local_md::Plugin
        .build(
            &SourceName::new("work").unwrap(),
            &json!({ "root": root.path() }),
            &NoSecrets,
        )
        .expect("the folder builds");
    (root, source)
}

fn id(value: &str) -> NativeId {
    NativeId(value.to_owned())
}

fn name(value: &str) -> AssetName {
    AssetName::new(value).expect("an asset name")
}

fn task(native: &str, content: &str) -> Task {
    Task {
        id: id(native),
        key: None,
        title: native.to_owned(),
        content: Some(content.to_owned()),
        status: Status {
            category: StatusCategory::Todo,
            name: "todo".to_owned(),
        },
        priority: Priority::None,
        labels: Vec::new(),
        project: None,
        url: None,
        location: None,
        created_at: None,
        updated_at: None,
        metadata: BTreeMap::new(),
        repositories: Vec::new(),
        delivers: Vec::new(),
        delivered_by: Vec::new(),
    }
}

fn document(native: &str, content: &str) -> Document {
    Document {
        id: id(native),
        title: native.to_owned(),
        content: Some(content.to_owned()),
        project: None,
        labels: Vec::new(),
        url: None,
        location: None,
        created_at: None,
        updated_at: None,
        metadata: BTreeMap::new(),
        repositories: Vec::new(),
    }
}

fn carrying(assets: &[(&str, &[u8])]) -> AssetWrite {
    AssetWrite {
        assets: assets
            .iter()
            .map(|(asset, bytes)| AssetPayload::of(name(asset), bytes.to_vec()))
            .collect(),
        recorded_assets: None,
    }
}

fn files_in(directory: &Path) -> Vec<String> {
    let mut names: Vec<String> = match fs::read_dir(directory) {
        Ok(entries) => entries
            .map(|entry| {
                entry
                    .expect("an entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect(),
        Err(_) => Vec::new(),
    };
    names.sort();
    names
}

#[tokio::test]
async fn a_tasks_assets_are_files_beside_it_listed_with_their_digest_and_path() {
    let (root, source) = folder();
    let write = ItemWrite {
        target: None,
        item: task("shot", "![a](./a.png) ![b](./B.JpEg)"),
        depends_on: Vec::new(),
    };
    let written = source
        .write_task_with_assets(
            &write,
            None,
            &carrying(&[("a.png", b"png"), ("B.JpEg", b"jpg")]),
        )
        .await
        .expect("the task lands with its assets");
    assert_eq!(written.id, id("shot"));
    assert_eq!(
        written.content.as_deref(),
        Some("![a](./a.png) ![b](./B.JpEg)")
    );

    let directory = root.path().join("tasks/shot.assets");
    assert_eq!(files_in(&directory), vec!["B.JpEg", "a.png"]);
    assert_eq!(fs::read(directory.join("a.png")).unwrap(), b"png");

    let listed = source.task_assets(&id("shot")).await.unwrap();
    assert_eq!(listed.len(), 2);
    for asset in &listed {
        let path = asset.path.as_deref().expect("a path on this machine");
        assert!(Path::new(path).is_absolute(), "{path}");
        assert_eq!(asset_sha256(&fs::read(path).unwrap()), asset.sha256);
        assert_eq!(asset.content_type, asset.name.content_type());
    }
    assert_eq!(
        source
            .task_asset(&id("shot"), &name("a.png"))
            .await
            .unwrap(),
        Some(b"png".to_vec())
    );
    assert_eq!(
        source
            .task_asset(&id("shot"), &name("c.png"))
            .await
            .unwrap(),
        None
    );
    assert!(source.task_assets(&id("absent")).await.unwrap().is_empty());
}

#[tokio::test]
async fn an_update_replaces_the_asset_set_whole_and_leaves_another_records_alone() {
    let (root, source) = folder();
    for (native, bytes) in [("one", b"first" as &[u8]), ("two", b"second")] {
        source
            .write_document_with_assets(
                &ItemWrite {
                    target: None,
                    item: document(native, "![x](./x.png) ![y](./y.gif)"),
                    depends_on: Vec::new(),
                },
                None,
                &carrying(&[("x.png", bytes), ("y.gif", b"gif")]),
            )
            .await
            .expect("lands");
    }
    // `one` now holds a new `x.png` and no `y.gif`.
    source
        .write_document_with_assets(
            &ItemWrite {
                target: Some(id("one")),
                item: document("one", "![x](./x.png)"),
                depends_on: Vec::new(),
            },
            None,
            &carrying(&[("x.png", b"replaced")]),
        )
        .await
        .expect("updates");
    assert_eq!(
        files_in(&root.path().join("documents/one.assets")),
        vec!["x.png"]
    );
    assert_eq!(
        fs::read(root.path().join("documents/one.assets/x.png")).unwrap(),
        b"replaced"
    );
    assert_eq!(
        fs::read(root.path().join("documents/two.assets/x.png")).unwrap(),
        b"second"
    );
    assert_eq!(
        files_in(&root.path().join("documents/two.assets")),
        vec!["x.png", "y.gif"]
    );
    // A write naming none leaves no directory at all.
    source
        .write_document_with_assets(
            &ItemWrite {
                target: Some(id("one")),
                item: document("one", "no pictures"),
                depends_on: Vec::new(),
            },
            None,
            &AssetWrite::default(),
        )
        .await
        .expect("updates");
    assert!(!root.path().join("documents/one.assets").exists());
}

#[tokio::test]
async fn removing_a_record_removes_its_assets_and_leaves_no_file_of_them() {
    let (root, source) = folder();
    source
        .write_task_with_assets(
            &ItemWrite {
                target: None,
                item: task("gone", "![a](./a.png)"),
                depends_on: Vec::new(),
            },
            None,
            &carrying(&[("a.png", b"png")]),
        )
        .await
        .expect("lands");
    source
        .write_document_with_assets(
            &ItemWrite {
                target: None,
                item: document("gone", "![w](./w.webp)"),
                depends_on: Vec::new(),
            },
            None,
            &carrying(&[("w.webp", b"webp")]),
        )
        .await
        .expect("lands");
    assert!(root.path().join("tasks/gone.assets/a.png").is_file());
    assert!(root.path().join("documents/gone.assets/w.webp").is_file());

    source.delete_task(&id("gone")).await.expect("removed");
    source.delete_document(&id("gone")).await.expect("removed");

    for folder in ["tasks", "documents"] {
        assert!(files_in(&root.path().join(folder)).is_empty(), "{folder}");
    }
}

#[tokio::test]
async fn a_rendering_write_replaces_the_asset_set_with_the_content() {
    let (root, source) = folder();
    source
        .write_task_with_assets(
            &ItemWrite {
                target: None,
                item: task("drawn", "![a](./a.png) ![b](./b.png)"),
                depends_on: Vec::new(),
            },
            Some(&BTreeMap::new()),
            &carrying(&[("a.png", b"a"), ("b.png", b"b")]),
        )
        .await
        .expect("lands");
    let written = source
        .set_task_rendering_with_assets(
            &id("drawn"),
            "![a](./a.png)",
            &json!({"template": "t", "digest": "sha256:00", "body_digest": "sha256:00",
                    "answers_digest": "sha256:00"}),
            &BTreeMap::new(),
            &carrying(&[("a.png", b"new a")]),
        )
        .await
        .expect("renders")
        .expect("the task is there");
    assert_eq!(written.content.as_deref(), Some("![a](./a.png)"));
    assert_eq!(
        files_in(&root.path().join("tasks/drawn.assets")),
        vec!["a.png"]
    );
    assert_eq!(
        source
            .get_task(&id("drawn"))
            .await
            .unwrap()
            .expect("read")
            .content
            .as_deref(),
        Some("![a](./a.png)")
    );
    assert_eq!(
        source
            .set_task_rendering_with_assets(
                &id("absent"),
                "x",
                &json!({}),
                &BTreeMap::new(),
                &AssetWrite::default(),
            )
            .await
            .expect("answers"),
        None
    );
}

#[tokio::test]
async fn a_payload_whose_bytes_are_missing_or_misdigested_is_refused_before_anything_is_written() {
    let (root, source) = folder();
    let mut wrong = carrying(&[("a.png", b"png")]);
    wrong.assets[0].sha256 = asset_sha256(b"other");
    let mut absent = carrying(&[("a.png", b"png")]);
    absent.assets[0].bytes = None;
    for (write, says) in [(wrong, "do not hash"), (absent, "carries no bytes")] {
        let refused = source
            .write_task_with_assets(
                &ItemWrite {
                    target: None,
                    item: task("refused", "![a](./a.png)"),
                    depends_on: Vec::new(),
                },
                None,
                &write,
            )
            .await
            .expect_err("refused");
        let SourceError::Refused { message } = refused else {
            panic!("a refusal, got {refused:?}")
        };
        assert!(
            message.contains("a.png") && message.contains(says),
            "{message}"
        );
    }
    assert!(!root.path().join("tasks").exists());
}

#[tokio::test]
async fn a_payload_without_bytes_reuses_the_asset_the_record_already_holds_by_digest() {
    let (root, source) = folder();
    source
        .write_task_with_assets(
            &ItemWrite {
                target: None,
                item: task("kept", "![a](./a.png)"),
                depends_on: Vec::new(),
            },
            None,
            &carrying(&[("a.png", b"png")]),
        )
        .await
        .expect("lands");
    let mut reused = carrying(&[("a.png", b"png")]);
    reused.assets[0].bytes = None;
    source
        .write_task_with_assets(
            &ItemWrite {
                target: Some(id("kept")),
                item: task("kept", "![a](./a.png) again"),
                depends_on: Vec::new(),
            },
            None,
            &reused,
        )
        .await
        .expect("updates");
    assert_eq!(
        fs::read(root.path().join("tasks/kept.assets/a.png")).unwrap(),
        b"png"
    );
}

#[tokio::test]
async fn replacing_and_removing_assets_leaves_a_file_a_person_put_beside_them() {
    let (root, source) = folder();
    let write = |assets: AssetWrite| {
        let source = &source;
        async move {
            source
                .write_task_with_assets(
                    &ItemWrite {
                        target: None,
                        item: task("kept", "![a](./a.png)"),
                        depends_on: Vec::new(),
                    },
                    None,
                    &assets,
                )
                .await
        }
    };
    write(carrying(&[("a.png", b"png")])).await.expect("lands");
    let directory = root.path().join("tasks/kept.assets");
    std::fs::write(directory.join("notes.txt"), "a person's own note").expect("a note");
    source
        .write_task_with_assets(
            &ItemWrite {
                target: Some(id("kept")),
                item: task("kept", "no pictures"),
                depends_on: Vec::new(),
            },
            None,
            &AssetWrite::default(),
        )
        .await
        .expect("updates");
    assert_eq!(files_in(&directory), vec!["notes.txt"]);
    source.delete_task(&id("kept")).await.expect("removed");
    assert_eq!(
        files_in(&directory),
        vec!["notes.txt"],
        "only assets are ever removed"
    );
}

#[tokio::test]
async fn a_payload_sent_as_a_content_type_its_name_does_not_give_is_refused() {
    let (root, source) = folder();
    let mut write = carrying(&[("a.png", b"png")]);
    write.assets[0].content_type = onetaskgraph_plugin_api::AssetContentType::Gif;
    let refused = source
        .write_task_with_assets(
            &ItemWrite {
                target: None,
                item: task("typed", "![a](./a.png)"),
                depends_on: Vec::new(),
            },
            None,
            &write,
        )
        .await
        .expect_err("refused");
    assert!(
        refused.to_string().contains("a.png") && refused.to_string().contains("image/gif"),
        "{refused}"
    );
    assert!(!root.path().join("tasks").exists());
}

#[tokio::test]
async fn a_malformed_digest_is_refused_and_a_failed_asset_write_leaves_no_staging_file() {
    let (root, source) = folder();
    let mut malformed = carrying(&[("a.png", b"png")]);
    malformed.assets[0].sha256 = "NOT-HEX".to_owned();
    malformed.assets[0].bytes = None;
    let refused = source
        .write_task_with_assets(
            &ItemWrite {
                target: None,
                item: task("digest", "![a](./a.png)"),
                depends_on: Vec::new(),
            },
            None,
            &malformed,
        )
        .await
        .expect_err("refused");
    assert!(refused.to_string().contains("NOT-HEX"), "{refused}");
    assert!(!root.path().join("tasks").exists());

    // A directory standing where the asset's file belongs: the rename over it fails.
    let blocked = root.path().join("tasks/blocked.assets/a.png");
    std::fs::create_dir_all(&blocked).expect("a directory in the asset's place");
    std::fs::write(blocked.join("inside"), "keeps the directory non-empty").expect("a file");
    source
        .write_task_with_assets(
            &ItemWrite {
                target: None,
                item: task("blocked", "![a](./a.png)"),
                depends_on: Vec::new(),
            },
            None,
            &carrying(&[("a.png", b"png")]),
        )
        .await
        .expect_err("the asset cannot be written");
    assert_eq!(
        files_in(&root.path().join("tasks/blocked.assets")),
        vec!["a.png"],
        "no staging file is left beside it"
    );
}
