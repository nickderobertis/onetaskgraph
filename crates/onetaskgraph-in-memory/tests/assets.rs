//! An in-memory source declaring assets native serves each at a URL, as a hosted destination
//! does; one declaring nothing refuses them.
//!
//! Each test drives the real source through the trait and reads back what it holds.

use onetaskgraph_in_memory::{InMemoryConfig, InMemorySource};
use onetaskgraph_plugin_api::{
    AssetName, AssetPayload, AssetUploads, AssetWrite, Document, ItemWrite, MetadataKey, NativeId,
    SourceError, Task, TaskSource, body_digest,
};
use serde_json::json;

fn source(assets: &str) -> InMemorySource {
    let config: InMemoryConfig = serde_json::from_value(json!({
        "capabilities": {"documents": "native", "assets": assets},
    }))
    .expect("a configuration");
    InMemorySource::new(config).expect("a coherent source")
}

fn name(value: &str) -> AssetName {
    AssetName::new(value).expect("an asset name")
}

fn task(content: &str) -> Task {
    serde_json::from_value(json!({
        "id": "T-1", "title": "Alpha", "content": content,
        "status": {"category": "todo", "name": "Todo"}, "labels": [],
        "metadata": {MetadataKey::TEMPLATE_KEY: {
            "template": "t.md", "digest": "sha256:aa", "body_digest": body_digest(content),
            "answers_digest": "sha256:bb"
        }},
    }))
    .expect("a task")
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

#[tokio::test]
async fn a_native_source_serves_each_asset_at_a_url_and_rewrites_the_references_to_it() {
    let source = source("native");
    let content = "![a](./a.png) and ![b](./b.gif)";
    let written = source
        .write_task_with_assets(
            &ItemWrite {
                target: None,
                item: task(content),
                depends_on: Vec::new(),
            },
            None,
            &carrying(&[("a.png", b"aaa"), ("b.gif", b"bbb")]),
        )
        .await
        .expect("lands");
    let held = source.get_task(&written.id).await.unwrap().expect("held");
    assert_eq!(held.content, written.content);
    let stored = held.content.clone().unwrap_or_default();
    assert!(
        !stored.contains("./a.png") && stored.contains("in-memory://"),
        "{stored}"
    );

    let uploads = AssetUploads::read(&held.metadata)
        .expect("a well-formed record")
        .expect("a record of what was served");
    assert_eq!(uploads.0.len(), 2);
    for (asset, upload) in &uploads.0 {
        assert!(stored.contains(&upload.url), "{asset} at {}", upload.url);
    }
    // The rendering still vouches for what landed.
    assert_eq!(
        held.metadata[MetadataKey::TEMPLATE_KEY]["body_digest"],
        body_digest(&stored)
    );
    let listed = source.task_assets(&written.id).await.unwrap();
    assert_eq!(listed.len(), 2);
    assert!(listed.iter().all(|asset| asset.path.is_none()));
    assert_eq!(
        source
            .task_asset(&written.id, &name("a.png"))
            .await
            .unwrap(),
        Some(b"aaa".to_vec())
    );

    // An update reusing one upload by digest and sending new bytes for the other keeps the
    // first URL and moves the second.
    let mut again = carrying(&[("a.png", b"aaa"), ("b.gif", b"changed")]);
    again.assets[0].bytes = None;
    again.recorded_assets = Some(uploads.clone());
    let rewritten = source
        .write_task_with_assets(
            &ItemWrite {
                target: Some(written.id.clone()),
                item: task(content),
                depends_on: Vec::new(),
            },
            None,
            &again,
        )
        .await
        .expect("updates");
    let held = source.get_task(&rewritten.id).await.unwrap().expect("held");
    let after = AssetUploads::read(&held.metadata).unwrap().unwrap();
    assert_eq!(after.0[&name("a.png")], uploads.0[&name("a.png")]);
    assert_ne!(after.0[&name("b.gif")].url, uploads.0[&name("b.gif")].url);
    assert_eq!(
        source
            .task_asset(&written.id, &name("a.png"))
            .await
            .unwrap(),
        Some(b"aaa".to_vec())
    );

    source.delete_task(&written.id).await.expect("removed");
    assert!(source.task_assets(&written.id).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_payload_without_bytes_that_nothing_records_is_refused() {
    let source = source("native");
    let mut write = carrying(&[("a.png", b"aaa")]);
    write.assets[0].bytes = None;
    let refused = source
        .write_document_with_assets(
            &ItemWrite {
                target: None,
                item: serde_json::from_value::<Document>(
                    json!({"id": "D-1", "title": "Design", "content": "![a](./a.png)", "labels": []}),
                )
                .unwrap(),
                depends_on: Vec::new(),
            },
            None,
            &write,
        )
        .await
        .expect_err("refused");
    assert!(
        matches!(&refused, SourceError::Refused { message } if message.contains("a.png")),
        "{refused:?}"
    );
}

#[tokio::test]
async fn a_source_declaring_nothing_refuses_an_asset_write_and_writes_nothing() {
    let source = source("unsupported");
    let refused = source
        .write_task_with_assets(
            &ItemWrite {
                target: None,
                item: task("![a](./a.png)"),
                depends_on: Vec::new(),
            },
            None,
            &carrying(&[("a.png", b"aaa")]),
        )
        .await
        .expect_err("refused");
    assert_eq!(refused, onetaskgraph_plugin_api::assetless("in-memory"));
    assert!(
        source
            .get_task(&NativeId::from("T-1"))
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn a_payload_whose_bytes_do_not_hash_to_its_digest_is_refused_and_nothing_is_written() {
    let source = source("native");
    let mut write = carrying(&[("a.png", b"aaa")]);
    write.assets[0].sha256 = onetaskgraph_plugin_api::asset_sha256(b"other");
    let refused = source
        .write_task_with_assets(
            &ItemWrite {
                target: None,
                item: task("![a](./a.png)"),
                depends_on: Vec::new(),
            },
            None,
            &write,
        )
        .await
        .expect_err("refused");
    assert!(
        matches!(&refused, SourceError::Refused { message } if message.contains("do not hash")),
        "{refused:?}"
    );
    assert!(
        source
            .get_task(&NativeId::from("T-1"))
            .await
            .unwrap()
            .is_none()
    );
}
