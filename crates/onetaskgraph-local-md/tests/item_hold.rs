//! A folder's hold on one record's file, and the byte-for-byte snapshot it restores — what a
//! copy decides its intent to create under, and what it puts back when it takes everything back.
//!
//! Driven through the real plugin over a real folder, two instances of it standing in for two
//! processes: a hold is a lock file beside the record, created exclusively, so the second
//! instance waits for the first to let go and finds the file as the first left it.

use std::fs;
use std::time::Duration;

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
    // The second instance says when it starts asking for the record and the moment it holds it;
    // the first lets go only once the second is asking, and notes the moment it did. Exclusion is
    // that the second's hold came after the first's release — an order, whatever the timing.
    let (asking, asked) = std::sync::mpsc::channel();
    let waiter = std::thread::spawn({
        let root = root.path().to_owned();
        move || {
            let second = instance(&root);
            asking.send(()).expect("the test is listening");
            let taken = second
                .hold_item(MetadataRecord::Task, &NativeId("T-1".to_owned()))
                .expect("held once released");
            (taken, std::time::Instant::now())
        }
    });
    asked.recv().expect("the second instance is asking");
    // Room for a hold that excluded nothing to be taken before the release below.
    std::thread::sleep(Duration::from_millis(200));
    let released = std::time::Instant::now();
    first.release_item(MetadataRecord::Task, &id);
    let (taken, held_at) = waiter.join().expect("the second hold");
    assert!(taken, "the second instance holds it");
    assert!(
        held_at >= released,
        "the second instance held it only once the first let go"
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

#[tokio::test]
async fn one_metadata_key_comes_off_a_project_and_a_document_and_nothing_else_moves() {
    let root = tempfile::tempdir().expect("a folder");
    for (directory, text) in [
        (
            "projects/P.md",
            "---\ntitle: P\nstatus: todo\nmetadata:\n  \"onetaskgraph.creating\": {\"x\": 1}\n  \
             keep.me: true\n---\nThe plan.\n",
        ),
        (
            "documents/D.md",
            "---\ntitle: D\nmetadata:\n  \"onetaskgraph.creating\": {\"x\": 1}\n  keep.me: true\n\
             ---\nThe design.\n",
        ),
    ] {
        let path = root.path().join(directory);
        fs::create_dir_all(path.parent().expect("a parent")).expect("a folder");
        fs::write(path, text).expect("a record");
    }
    let source = instance(root.path());
    let key = onetaskgraph_plugin_api::MetadataKey::creating();
    for (record, id, file) in [
        (MetadataRecord::Project, "P", "projects/P.md"),
        (MetadataRecord::Document, "D", "documents/D.md"),
    ] {
        let before = fs::read_to_string(root.path().join(file)).expect("the file");
        let removed = source
            .remove_record_metadata(record, &NativeId(id.to_owned()), &key)
            .await
            .expect("removed");
        assert!(removed.is_some(), "{file}: the record is held");
        let after = fs::read_to_string(root.path().join(file)).expect("the file");
        assert_eq!(
            after,
            before.replace("  \"onetaskgraph.creating\": {\"x\": 1}\n", ""),
            "{file}: only that one entry left the file"
        );
        // Taking off a key the record does not hold writes nothing.
        source
            .remove_record_metadata(record, &NativeId(id.to_owned()), &key)
            .await
            .expect("nothing to remove");
        assert_eq!(fs::read_to_string(root.path().join(file)).unwrap(), after);
    }
    assert!(
        source
            .remove_record_metadata(MetadataRecord::Project, &NativeId("none".to_owned()), &key)
            .await
            .expect("an absent record")
            .is_none()
    );
}

#[test]
fn a_lock_written_ahead_of_this_clock_is_refused_at_once_and_one_that_cannot_be_made_is_refused() {
    let root = folder();
    let id = NativeId("T-1".to_owned());
    let lock = root.path().join("tasks/T-1.md.lock");
    let file = fs::File::create(&lock).expect("another host's lock");
    file.set_modified(std::time::SystemTime::now() + Duration::from_secs(3600))
        .expect("an hour ahead");
    drop(file);
    let source = instance(root.path());
    let refused = source
        .hold_item(MetadataRecord::Task, &id)
        .expect_err("its age cannot be told");
    assert!(
        matches!(&refused, onetaskgraph_plugin_api::SourceError::Unavailable { message }
            if message.contains("ahead of this host's clock")),
        "{refused:?}"
    );
    assert!(lock.exists(), "another host's lock is left where it is");
    fs::remove_file(&lock).expect("removed");

    // A folder this process may not write in: no lock can be made beside the record.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let tasks = root.path().join("tasks");
        fs::set_permissions(&tasks, fs::Permissions::from_mode(0o555)).expect("read-only");
        let refused = source.hold_item(MetadataRecord::Task, &id);
        fs::set_permissions(&tasks, fs::Permissions::from_mode(0o755)).expect("writable");
        assert!(
            matches!(&refused, Err(onetaskgraph_plugin_api::SourceError::Unavailable { message })
                if message.contains("could not lock")),
            "{refused:?}"
        );
        assert!(!lock.exists());
    }
}
