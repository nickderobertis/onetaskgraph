//! A folder's hold on one record's file, and the byte-for-byte snapshot it restores — what a
//! copy decides its intent to create under, and what it puts back when it takes everything back.
//!
//! Driven through the real plugin over a real folder, two instances of it standing in for two
//! processes: a hold is a lock file beside the record, created exclusively, so the second
//! instance waits for the first to let go and finds the file as the first left it.

use std::fs;
use std::time::{Duration, Instant};

use onetaskgraph_plugin_api::{
    MetadataRecord, NativeId, SecretResolver, SourceName, SourcePlugin, TaskSource,
};
use secrecy::SecretString;
use serde_json::json;

struct NoSecrets;
impl SecretResolver for NoSecrets {
    fn get(&self, _: &str) -> Option<SecretString> {
        None
    }
}

fn instance(root: &std::path::Path) -> Box<dyn TaskSource> {
    onetaskgraph_local_md::Plugin
        .build(
            &SourceName::new("work").unwrap(),
            &json!({ "root": root }),
            &NoSecrets,
        )
        .expect("the folder builds")
}

const TASK: &str = "---\ntitle: One\nstatus: todo\n---\nBody.\n";

fn folder() -> tempfile::TempDir {
    let root = tempfile::tempdir().expect("a folder");
    fs::create_dir_all(root.path().join("tasks")).expect("tasks");
    fs::write(root.path().join("tasks/T-1.md"), TASK).expect("a task");
    root
}

#[test]
fn a_hold_excludes_another_instance_until_it_is_released() {
    let root = folder();
    let id = NativeId("T-1".to_owned());
    let first = instance(root.path());
    assert!(first.hold_item(MetadataRecord::Task, &id).expect("held"));
    let lock = root.path().join("tasks/T-1.md.lock");
    assert!(lock.exists(), "the hold is a lock file beside the record");
    let started = Instant::now();
    let waiter = std::thread::spawn({
        let root = root.path().to_owned();
        move || {
            let second = instance(&root);
            second
                .hold_item(MetadataRecord::Task, &NativeId("T-1".to_owned()))
                .expect("held once released")
        }
    });
    std::thread::sleep(Duration::from_millis(300));
    first.release_item(MetadataRecord::Task, &id);
    assert!(
        waiter.join().expect("the second hold"),
        "the second instance holds it next"
    );
    assert!(
        started.elapsed() >= Duration::from_millis(250),
        "it waited for the first to let go"
    );
    assert!(lock.exists(), "the second instance holds it now");
}

#[test]
fn a_snapshot_is_restored_byte_for_byte_and_a_missing_record_offers_none() {
    let root = folder();
    let source = instance(root.path());
    let id = NativeId("T-1".to_owned());
    let snapshot = source
        .item_snapshot(MetadataRecord::Task, &id)
        .expect("the file's bytes");
    assert_eq!(snapshot, TASK.as_bytes());
    fs::write(
        root.path().join("tasks/T-1.md"),
        "---\ntitle: Edited\n---\n",
    )
    .expect("an edit");
    source
        .restore_item(MetadataRecord::Task, &id, &snapshot)
        .expect("restored");
    assert_eq!(
        fs::read(root.path().join("tasks/T-1.md")).unwrap(),
        TASK.as_bytes()
    );
    assert!(
        !root.path().join("tasks/T-1.md.lock").exists(),
        "no lock is left"
    );
    assert!(
        source
            .item_snapshot(MetadataRecord::Task, &NativeId("T-9".to_owned()))
            .is_none()
    );
    assert!(
        !source
            .hold_item(MetadataRecord::Task, &NativeId("T-9".to_owned()))
            .expect("nothing to hold")
    );
}
