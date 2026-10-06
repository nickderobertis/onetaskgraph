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

/// Whether `source` refuses a write reusing, without sending them, `bytes` under a record
/// claiming an upload of them — which it accepts only for bytes it really holds.
async fn refuses_a_reuse_of(source: &InMemorySource, bytes: &[u8]) -> bool {
    let mut claimed = carrying(&[("a.png", bytes)]);
    claimed.recorded_assets = Some(AssetUploads(std::collections::BTreeMap::from([(
        name("a.png"),
        onetaskgraph_plugin_api::AssetUpload {
            sha256: claimed.assets[0].sha256.clone(),
            url: "in-memory://assets/claimed".to_owned(),
        },
    )])));
    claimed.assets[0].bytes = None;
    source
        .write_task_with_assets(
            &ItemWrite {
                target: None,
                item: task("![a](./a.png)"),
                depends_on: Vec::new(),
            },
            None,
            &claimed,
        )
        .await
        .is_err_and(|refused| refused.to_string().contains("carries no bytes"))
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

#[tokio::test]
async fn a_name_given_twice_and_a_reuse_of_bytes_this_source_never_held_are_refused() {
    let source = source("native");
    let write = |assets: AssetWrite| {
        let source = &source;
        async move {
            source
                .write_task_with_assets(
                    &ItemWrite {
                        target: None,
                        item: task("![a](./a.png)"),
                        depends_on: Vec::new(),
                    },
                    None,
                    &assets,
                )
                .await
                .expect_err("refused")
        }
    };
    let twice = write(carrying(&[("a.png", b"one"), ("a.png", b"two")])).await;
    assert!(twice.to_string().contains("given twice"), "{twice}");
    // The refused write kept none of its bytes, not even the payload before the duplicate.
    assert!(refuses_a_reuse_of(&source, b"one").await);

    // A record a caller handed over claims an upload this source never received.
    let mut claimed = carrying(&[("a.png", b"never sent")]);
    let upload = AssetUploads(std::collections::BTreeMap::from([(
        name("a.png"),
        onetaskgraph_plugin_api::AssetUpload {
            sha256: claimed.assets[0].sha256.clone(),
            url: "in-memory://assets/elsewhere".to_owned(),
        },
    )]));
    claimed.assets[0].bytes = None;
    claimed.recorded_assets = Some(upload);
    let unbacked = write(claimed).await;
    assert!(
        unbacked.to_string().contains("carries no bytes"),
        "{unbacked}"
    );
    assert!(
        source
            .get_task(&NativeId::from("T-1"))
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn deleting_a_document_takes_its_assets_with_it() {
    let source = source("native");
    let written = source
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
            &carrying(&[("a.png", b"aaa")]),
        )
        .await
        .expect("lands");
    assert_eq!(source.document_assets(&written.id).await.unwrap().len(), 1);
    source.delete_document(&written.id).await.expect("removed");
    assert!(
        source
            .document_assets(&written.id)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        source
            .document_asset(&written.id, &name("a.png"))
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn a_record_with_no_content_keeps_none_through_an_asset_write() {
    let source = source("native");
    let mut bare = task("x");
    bare.content = None;
    let written = source
        .write_task_with_assets(
            &ItemWrite {
                target: None,
                item: bare,
                depends_on: Vec::new(),
            },
            None,
            &AssetWrite::default(),
        )
        .await
        .expect("lands");
    assert_eq!(written.content, None);
    assert_eq!(
        source
            .get_task(&written.id)
            .await
            .unwrap()
            .expect("held")
            .content,
        None
    );
    let mut document = serde_json::from_value::<Document>(
        json!({"id": "D-1", "title": "Design", "content": null, "labels": []}),
    )
    .unwrap();
    document.content = None;
    let written = source
        .write_document_with_assets(
            &ItemWrite {
                target: None,
                item: document,
                depends_on: Vec::new(),
            },
            None,
            &AssetWrite::default(),
        )
        .await
        .expect("lands");
    assert_eq!(written.content, None);
    assert_eq!(
        source
            .get_document(&written.id)
            .await
            .unwrap()
            .expect("held")
            .content,
        None
    );
}

/// A source seeded with one task and one document, each with an asset, whose writes are
/// `writes`.
fn seeded(writes: &str) -> InMemorySource {
    let config: InMemoryConfig = serde_json::from_value(json!({
        "capabilities": {"documents": "native", "assets": "native", "writes": writes},
        "tasks": [task("![a](./a.png)")],
        "documents": [{"id": "D-1", "title": "Design", "content": "![a](./a.png)", "labels": []}],
    }))
    .expect("a configuration");
    InMemorySource::new(config).expect("a coherent source")
}

#[tokio::test]
async fn a_rendering_with_assets_is_refused_by_a_read_only_source_and_changes_nothing() {
    let source = seeded("unsupported");
    let provenance = json!({"rendered": "afresh"});
    let answers = std::collections::BTreeMap::new();
    let assets = carrying(&[("a.png", b"aaa")]);
    let task_id = NativeId::from("T-1");
    let document_id = NativeId::from("D-1");
    let refused = source
        .set_task_rendering_with_assets(
            &task_id,
            "![a](./a.png) new",
            &provenance,
            &answers,
            &assets,
        )
        .await
        .expect_err("refused");
    assert_eq!(refused, onetaskgraph_plugin_api::unwritable("in-memory"));
    let refused = source
        .set_document_rendering_with_assets(
            &document_id,
            "![a](./a.png) new",
            &provenance,
            &answers,
            &assets,
        )
        .await
        .expect_err("refused");
    assert_eq!(refused, onetaskgraph_plugin_api::unwritable("in-memory"));

    let held = source.get_task(&task_id).await.unwrap().expect("held");
    assert_eq!(held.content.as_deref(), Some("![a](./a.png)"));
    assert_ne!(held.metadata[MetadataKey::TEMPLATE_KEY], provenance);
    let held = source
        .get_document(&document_id)
        .await
        .unwrap()
        .expect("held");
    assert_eq!(held.content.as_deref(), Some("![a](./a.png)"));
    assert!(source.task_assets(&task_id).await.unwrap().is_empty());
    assert!(
        source
            .document_assets(&document_id)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn a_rendering_with_assets_of_a_record_this_source_does_not_hold_answers_none_and_stores_nothing()
 {
    let source = seeded("supported");
    let provenance = json!({"rendered": "afresh"});
    let answers = std::collections::BTreeMap::new();
    let assets = carrying(&[("a.png", b"aaa")]);
    let missing = NativeId::from("missing");
    assert_eq!(
        source
            .set_task_rendering_with_assets(
                &missing,
                "![a](./a.png)",
                &provenance,
                &answers,
                &assets
            )
            .await
            .expect("answered"),
        None
    );
    assert_eq!(
        source
            .set_document_rendering_with_assets(
                &missing,
                "![a](./a.png)",
                &provenance,
                &answers,
                &assets,
            )
            .await
            .expect("answered"),
        None
    );
    assert!(source.get_task(&missing).await.unwrap().is_none());
    assert!(source.get_document(&missing).await.unwrap().is_none());
    assert!(source.task_assets(&missing).await.unwrap().is_empty());
    assert!(source.document_assets(&missing).await.unwrap().is_empty());
    // Nor did it keep the bytes it was sent for a record it does not hold.
    assert!(refuses_a_reuse_of(&source, b"aaa").await);
    // The records it does hold are as they were, with no asset of the refused rendering.
    for id in ["T-1", "D-1"] {
        let id = NativeId::from(id);
        assert!(source.task_assets(&id).await.unwrap().is_empty());
        assert!(source.document_assets(&id).await.unwrap().is_empty());
    }
    let held = source
        .get_task(&NativeId::from("T-1"))
        .await
        .unwrap()
        .expect("held");
    assert_eq!(held.content.as_deref(), Some("![a](./a.png)"));
    let held = source
        .get_document(&NativeId::from("D-1"))
        .await
        .unwrap()
        .expect("held");
    assert_eq!(held.content.as_deref(), Some("![a](./a.png)"));
}

#[tokio::test]
async fn a_reuse_backed_by_another_asset_of_the_same_write_is_accepted_and_holds_those_bytes() {
    let source = source("native");
    // `b.gif` is sent without bytes under a record of an upload of exactly the bytes `a.png`
    // carries in this very write, to a source that never held them before.
    let mut write = carrying(&[("a.png", b"same"), ("b.gif", b"same")]);
    let recorded = "in-memory://assets/recorded-b";
    write.recorded_assets = Some(AssetUploads(std::collections::BTreeMap::from([(
        name("b.gif"),
        onetaskgraph_plugin_api::AssetUpload {
            sha256: write.assets[1].sha256.clone(),
            url: recorded.to_owned(),
        },
    )])));
    write.assets[1].bytes = None;
    let written = source
        .write_task_with_assets(
            &ItemWrite {
                target: None,
                item: task("![a](./a.png) ![b](./b.gif)"),
                depends_on: Vec::new(),
            },
            None,
            &write,
        )
        .await
        .expect("the reuse is backed by the bytes the write carries");
    let held = source.get_task(&written.id).await.unwrap().expect("held");
    let uploads = AssetUploads::read(&held.metadata).unwrap().unwrap();
    assert_eq!(
        uploads.0[&name("b.gif")].url,
        recorded,
        "the recorded URL is kept"
    );
    assert!(
        held.content
            .as_deref()
            .unwrap_or_default()
            .contains(recorded),
        "{:?}",
        held.content
    );
    for asset in ["a.png", "b.gif"] {
        assert_eq!(
            source.task_asset(&written.id, &name(asset)).await.unwrap(),
            Some(b"same".to_vec()),
            "{asset}"
        );
    }
}
