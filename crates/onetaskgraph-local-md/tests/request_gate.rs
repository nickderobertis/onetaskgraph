//! A caller's request gate over a folder of Markdown: a rendering written with its assets is
//! one request, asked once, and told ended only once every file of it is on disk — the
//! rendered record and its asset directory both.
//!
//! The gate is the real plugin's (`<dyn RequestGate>::scope`) over a real folder, and what it
//! asserts is the folder as it stood at each signal, not a count alone: an end delivered before
//! the write finished would release a one-request-at-a-time governor while the folder is still
//! being written.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use onetaskgraph_plugin_api::{
    AssetName, AssetPayload, AssetWrite, Document, GatedRequest, ItemWrite, NativeId, Priority,
    QuotaReading, RequestGate, SecretResolver, SourceError, SourceName, SourcePlugin, Status,
    StatusCategory, Task, TaskSource,
};
use secrecy::SecretString;
use serde_json::json;

struct NoSecrets;
impl SecretResolver for NoSecrets {
    fn get(&self, _: &str) -> Option<SecretString> {
        None
    }
}

/// What the gate saw: an ask, or an end with the folder's state at that moment.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Event {
    Asked,
    /// Whether the rendered content and the new asset bytes were both on disk.
    Ended {
        rendered: bool,
        asset: bool,
    },
}

/// A gate recording each ask, and at each end whether `record` holds `rendered` and `asset`
/// holds `bytes`.
struct Recording {
    record: PathBuf,
    rendered: String,
    asset: PathBuf,
    bytes: Vec<u8>,
    events: Mutex<Vec<Event>>,
}

/// What [`RequestGate::before_request`] answers, as `#[async_trait]` spells it.
type GateFuture<'future> =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), SourceError>> + Send + 'future>>;

// Spelled as `#[async_trait]` expands it, so this crate needs no dependency of its own for the
// macro: a new `Cargo.lock` entry is a global selection input that runs every project's tests.
impl RequestGate for Recording {
    fn before_request<'gate, 'request, 'future>(
        &'gate self,
        _request: &'request GatedRequest,
    ) -> GateFuture<'future>
    where
        'gate: 'future,
        'request: 'future,
        Self: 'future,
    {
        self.events.lock().unwrap().push(Event::Asked);
        Box::pin(std::future::ready(Ok(())))
    }

    fn after_response(&self, _readings: &[QuotaReading]) {}

    fn request_ended(&self) {
        let rendered =
            std::fs::read_to_string(&self.record).is_ok_and(|text| text.contains(&self.rendered));
        let asset = std::fs::read(&self.asset).is_ok_and(|bytes| bytes == self.bytes);
        self.events
            .lock()
            .unwrap()
            .push(Event::Ended { rendered, asset });
    }
}

fn folder(root: &Path) -> Box<dyn TaskSource> {
    onetaskgraph_local_md::Plugin
        .build(
            &SourceName::new("work").unwrap(),
            &json!({ "root": root }),
            &NoSecrets,
        )
        .expect("the folder builds")
}

fn id(value: &str) -> NativeId {
    NativeId(value.to_owned())
}

fn carrying(asset: &str, bytes: &[u8]) -> AssetWrite {
    AssetWrite {
        assets: vec![AssetPayload::of(
            AssetName::new(asset).expect("an asset name"),
            bytes.to_vec(),
        )],
        recorded_assets: None,
    }
}

fn task(native: &str, content: &str) -> Task {
    Task {
        version: None,
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
        classification: Default::default(),
        assignees: None,
    }
}

fn document(native: &str, content: &str) -> Document {
    Document {
        version: None,
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
        classification: Default::default(),
    }
}

fn provenance() -> serde_json::Value {
    json!({"template": "t", "digest": "sha256:00", "body_digest": "sha256:00",
           "answers_digest": "sha256:00"})
}

const RENDERED: &str = "Rendered afresh ![a](./a.png)";

/// One ask, and one end that finds the rendering and its asset already written.
fn one_request_ended_after_both_files(gate: &Recording) {
    assert_eq!(
        *gate.events.lock().unwrap(),
        [
            Event::Asked,
            Event::Ended {
                rendered: true,
                asset: true
            }
        ],
        "the rendering with its assets is one request, ended once both are on disk"
    );
}

#[tokio::test]
async fn a_task_rendering_with_its_assets_ends_once_after_both_are_written() {
    let root = tempfile::tempdir().expect("a folder");
    let source = folder(root.path());
    source
        .write_task_with_assets(
            &ItemWrite {
                target: None,
                item: task("drawn", "![a](./a.png)"),
                depends_on: Vec::new(),
            },
            Some(&BTreeMap::new()),
            &carrying("a.png", b"old a"),
        )
        .await
        .expect("lands");
    let gate = Arc::new(Recording {
        record: root.path().join("tasks/drawn.md"),
        rendered: RENDERED.to_owned(),
        asset: root.path().join("tasks/drawn.assets/a.png"),
        bytes: b"new a".to_vec(),
        events: Mutex::new(Vec::new()),
    });
    let installed: Arc<dyn RequestGate> = gate.clone();
    <dyn RequestGate>::scope(
        installed,
        source.set_task_rendering_with_assets(
            &id("drawn"),
            RENDERED,
            &provenance(),
            &BTreeMap::new(),
            &carrying("a.png", b"new a"),
        ),
    )
    .await
    .expect("renders")
    .expect("the task is there");
    one_request_ended_after_both_files(&gate);
}

#[tokio::test]
async fn a_document_rendering_with_its_assets_ends_once_after_both_are_written() {
    let root = tempfile::tempdir().expect("a folder");
    let source = folder(root.path());
    source
        .write_document_with_assets(
            &ItemWrite {
                target: None,
                item: document("drawn", "![a](./a.png)"),
                depends_on: Vec::new(),
            },
            None,
            &carrying("a.png", b"old a"),
        )
        .await
        .expect("lands");
    let gate = Arc::new(Recording {
        record: root.path().join("documents/drawn.md"),
        rendered: RENDERED.to_owned(),
        asset: root.path().join("documents/drawn.assets/a.png"),
        bytes: b"new a".to_vec(),
        events: Mutex::new(Vec::new()),
    });
    let installed: Arc<dyn RequestGate> = gate.clone();
    <dyn RequestGate>::scope(
        installed,
        source.set_document_rendering_with_assets(
            &id("drawn"),
            RENDERED,
            &provenance(),
            &BTreeMap::new(),
            &carrying("a.png", b"new a"),
        ),
    )
    .await
    .expect("renders")
    .expect("the document is there");
    one_request_ended_after_both_files(&gate);
}
