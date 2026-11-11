//! The in-memory source's parents and the `## Links` section of a linked task, driven through
//! the real trait: its work dies with its process, so what one write left is read back by the
//! same source rather than by a later command.

use std::collections::BTreeMap;

use onetaskgraph_in_memory::{InMemoryConfig, InMemorySource};
use onetaskgraph_plugin_api::{
    ItemWrite, NativeId, ParentAnswer, ParentWrite, TaskSource, TaskUpdate, with_links_section,
};
use serde_json::json;

const LINE: &str = "**caused-by** → [Root](https://example.test/root) `memory:root`";

/// A writable source holding `linked`, which holds one link and the section a link write
/// rendered, and `root`, the task it links to.
fn source() -> InMemorySource {
    let status = json!({"category": "todo", "name": "todo"});
    let config: InMemoryConfig = serde_json::from_value(json!({
        "capabilities": {"writes": "supported", "assets": "native"},
        "tasks": [
            {"id": "root", "title": "Root", "labels": [], "status": status},
            {"id": "linked", "title": "Linked", "labels": [], "status": status,
             "content": with_links_section("First.", &[LINE.to_owned()]),
             "metadata": {"onetaskgraph.links": [
                 {"type": "caused-by", "direction": "out", "other": "memory:root"}]}},
        ],
        "parents": {"linked": "root"},
    }))
    .expect("a configuration");
    InMemorySource::new(config).expect("a coherent source")
}

/// The task `id` as a read answers it.
async fn read(source: &InMemorySource, id: &NativeId) -> onetaskgraph_plugin_api::Task {
    source.get_task(id).await.unwrap().unwrap()
}

#[tokio::test]
async fn every_content_write_of_a_linked_task_keeps_its_section_and_a_read_strips_it() {
    let source = source();
    let id = NativeId("linked".into());
    assert_eq!(read(&source, &id).await.content.as_deref(), Some("First."));

    source.set_task_content(&id, "Second.").await.unwrap();
    assert_eq!(read(&source, &id).await.content.as_deref(), Some("Second."));

    source
        .set_task_rendering(&id, "Third.", &json!({"reference": "t"}), &BTreeMap::new())
        .await
        .unwrap();
    assert_eq!(read(&source, &id).await.content.as_deref(), Some("Third."));

    let update = TaskUpdate {
        content: Some("Fourth.".to_owned()),
        ..TaskUpdate::default()
    };
    let outcome = source.update_task(&id, &update).await.unwrap().unwrap();
    assert_eq!(outcome.task.content.as_deref(), Some("Fourth."));

    let mut task = read(&source, &id).await;
    task.content = Some("Fifth.".to_owned());
    source
        .write_task(&ItemWrite {
            target: Some(id.clone()),
            item: task,
            depends_on: Vec::new(),
        })
        .await
        .unwrap();
    assert_eq!(read(&source, &id).await.content.as_deref(), Some("Fifth."));

    // The section the link write rendered survived every write: removing the link's metadata
    // shows the stored content whole, the line it kept beside it.
    let unlinked = TaskUpdate {
        metadata_remove: std::iter::once(onetaskgraph_plugin_api::MetadataKey::links()).collect(),
        ..TaskUpdate::default()
    };
    let bare = source.update_task(&id, &unlinked).await.unwrap().unwrap();
    assert_eq!(
        bare.task.content.as_deref(),
        Some(with_links_section("Fifth.", &[LINE.to_owned()]).as_str())
    );
}

#[tokio::test]
async fn a_parent_write_answers_each_edge_on_its_own() {
    let source = source();
    let answers = source
        .set_parents(&[
            ParentWrite {
                child: NativeId("root".into()),
                parent: Some(NativeId("linked".into())),
                if_version: None,
            },
            ParentWrite {
                child: NativeId("linked".into()),
                parent: None,
                if_version: None,
            },
        ])
        .await;
    assert!(
        matches!(answers[0], Ok(ParentAnswer::Refused { .. })),
        "{answers:?}"
    );
    assert!(
        matches!(answers[1], Ok(ParentAnswer::Written { .. })),
        "{answers:?}"
    );
    let read = source
        .read_relations(&[NativeId("root".into()), NativeId("linked".into())])
        .await
        .unwrap();
    let (root, linked) = (read[0].as_ref().unwrap(), read[1].as_ref().unwrap());
    assert!(root.children.is_empty() && linked.parent.is_none());
}

#[tokio::test]
async fn a_parent_write_names_a_missing_task_a_moved_version_and_an_unchanged_parent() {
    let source = source();
    let version = read(&source, &NativeId("linked".into())).await.version;
    let write = |child: &str, parent: Option<&str>, if_version: Option<String>| ParentWrite {
        child: NativeId(child.into()),
        parent: parent.map(|parent| NativeId(parent.into())),
        if_version,
    };
    let answers = source
        .set_parents(&[
            write("ghost", Some("root"), None),
            write("linked", Some("ghost"), None),
            write("linked", Some("root"), Some("mem1|stale".to_owned())),
            write("linked", Some("root"), version.clone()),
        ])
        .await;
    assert_eq!(
        answers[0],
        Ok(ParentAnswer::Missing {
            what: NativeId("ghost".into())
        })
    );
    assert_eq!(
        answers[1],
        Ok(ParentAnswer::Missing {
            what: NativeId("ghost".into())
        })
    );
    assert_eq!(
        answers[2],
        Ok(ParentAnswer::VersionMoved {
            found: version.clone()
        })
    );
    assert_eq!(answers[3], Ok(ParentAnswer::Unchanged { version }));
}

#[tokio::test]
async fn a_render_with_image_assets_keeps_a_linked_task_s_section_and_answers_without_it() {
    let source = source();
    let id = NativeId("linked".into());
    let assets = onetaskgraph_plugin_api::AssetWrite {
        assets: vec![onetaskgraph_plugin_api::AssetPayload::of(
            onetaskgraph_plugin_api::AssetName::new("a.png").expect("an asset name"),
            vec![0x89, b'P', b'N', b'G'],
        )],
        recorded_assets: None,
    };
    let written = source
        .set_task_rendering_with_assets(
            &id,
            "![a](./a.png) rendered.",
            &json!({"reference": "t"}),
            &BTreeMap::new(),
            &assets,
        )
        .await
        .unwrap()
        .expect("the task is held");
    let content = read(&source, &id).await.content.expect("content");
    assert!(content.ends_with(" rendered."), "{content}");
    assert!(!content.contains("## Links"), "{content}");
    assert!(
        !written
            .content
            .as_deref()
            .unwrap_or_default()
            .contains("## Links"),
        "{written:?}"
    );
    // The section is still stored: without the link's metadata the content reads whole.
    let unlinked = TaskUpdate {
        metadata_remove: std::iter::once(onetaskgraph_plugin_api::MetadataKey::links()).collect(),
        ..TaskUpdate::default()
    };
    let bare = source.update_task(&id, &unlinked).await.unwrap().unwrap();
    assert!(
        bare.task
            .content
            .as_deref()
            .unwrap_or_default()
            .ends_with(&format!("## Links\n\n- {LINE}\n")),
        "{:?}",
        bare.task.content
    );
}
