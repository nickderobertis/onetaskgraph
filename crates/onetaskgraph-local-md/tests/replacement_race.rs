//! A folder listed while another process replaces its records in place.
//!
//! Within one process a listing and a replacement never overlap: the plugin holds them apart
//! itself. Across processes nothing does, so these tests put the writer in a second process
//! — this very test binary, run again as that writer — and list the folder from this one
//! for as long as it writes. On Windows a rename over a record used to make the listing
//! spell that record, for an instant, with a path that failed the root-containment check
//! (`a.md escapes configured root`, with both paths naming the same root); the listing now
//! resolves only links, so there is no such instant to land in.

use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use onetaskgraph_plugin_api::{
    MetadataKey, NativeId, PageRequest, SecretResolver, SourceError, SourceName, SourcePlugin,
    StatusCategory, TaskQuery, TaskSource,
};
use secrecy::SecretString;
use serde_json::json;

struct NoSecrets;
impl SecretResolver for NoSecrets {
    fn get(&self, _: &str) -> Option<SecretString> {
        None
    }
}

/// The variable naming the folder the writer process replaces records in.
const WRITER: &str = "ONETASKGRAPH_LOCAL_MD_REPLACING_IN";

/// How many replacements the writer makes. More on Windows, where the race was observed.
const REPLACEMENTS: u32 = if cfg!(windows) { 1000 } else { 400 };

fn source(root: &Path) -> Box<dyn TaskSource> {
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

fn page() -> PageRequest {
    PageRequest {
        limit: 200,
        cursor: None,
    }
}

/// Replace `tasks/a.md` under `root` in place, over and over: a metadata write and a status
/// write in turn, each a staging file renamed over the record.
fn replace_repeatedly(root: &Path) {
    let writer = source(root);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let key = MetadataKey::new("myapp.counter").unwrap();
    for round in 1..=REPLACEMENTS {
        runtime
            .block_on(writer.set_task_metadata(&id("a"), &key, &json!(round)))
            .expect("the metadata write lands")
            .expect("held");
        let category = if round % 2 == 0 {
            StatusCategory::Todo
        } else {
            StatusCategory::Done
        };
        runtime
            .block_on(writer.set_task_status(&id("a"), category))
            .expect("the status write lands")
            .expect("held");
    }
}

#[test]
fn a_folder_lists_while_another_process_replaces_a_record_in_place() {
    if let Some(root) = std::env::var_os(WRITER) {
        replace_repeatedly(Path::new(&root));
        return;
    }
    let root = tempfile::tempdir().expect("temporary notes");
    fs::create_dir_all(root.path().join("tasks/team")).unwrap();
    fs::write(
        root.path().join("tasks/a.md"),
        "---\ntitle: Busy\nstatus: todo\n---\nThe body.\n",
    )
    .unwrap();
    fs::write(
        root.path().join("tasks/team/b.md"),
        "---\ntitle: Still\nstatus: todo\n---\n",
    )
    .unwrap();
    let mut writer = Command::new(std::env::current_exe().expect("this test binary"))
        .args([
            "--exact",
            "a_folder_lists_while_another_process_replaces_a_record_in_place",
            "--test-threads=1",
        ])
        .env(WRITER, root.path())
        .stdout(Stdio::null())
        .spawn()
        .expect("the writer process starts");
    let reader = source(root.path());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let mut listings = 0_u32;
    let status = loop {
        let exited = writer.try_wait().expect("the writer can be waited on");
        let listed = runtime
            .block_on(reader.query_tasks(&TaskQuery::default(), &page()))
            .unwrap_or_else(|error| {
                let _ = writer.kill();
                panic!("a listing during a replacement failed: {error:?}")
            });
        let ids: Vec<&str> = listed.items.iter().map(|t| t.id.0.as_str()).collect();
        assert_eq!(ids, ["a", "team/b"], "every record is listed, once");
        assert_eq!(listed.items[0].title, "Busy");
        listings += 1;
        if let Some(status) = exited {
            break status;
        }
    };
    assert!(status.success(), "the writer process failed: {status}");
    assert!(listings > 1, "the folder was listed while it was written");
    let task = runtime
        .block_on(reader.get_task(&id("a")))
        .unwrap()
        .unwrap();
    assert_eq!(task.metadata["myapp.counter"], json!(REPLACEMENTS));
}

#[cfg(unix)]
#[tokio::test]
async fn a_linked_folder_is_named_where_it_really_is_and_one_outside_the_root_is_refused() {
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir().expect("temporary notes");
    fs::create_dir_all(root.path().join("tasks")).unwrap();
    fs::create_dir_all(root.path().join("shared")).unwrap();
    fs::write(
        root.path().join("shared/c.md"),
        "---\ntitle: Linked\nstatus: todo\n---\n",
    )
    .unwrap();
    // A link to a folder inside the root is followed, and what it holds is named by where
    // it really is — which is outside `tasks/`, so it is refused as not a task.
    symlink(root.path().join("shared"), root.path().join("tasks/shared")).unwrap();
    let error = source(root.path())
        .query_tasks(&TaskQuery::default(), &page())
        .await
        .unwrap_err();
    assert!(
        matches!(error, SourceError::Malformed { ref message } if message.contains("is outside")),
        "{error:?}"
    );
    fs::remove_file(root.path().join("tasks/shared")).unwrap();

    let outside = tempfile::tempdir().expect("a folder outside the root");
    fs::write(
        outside.path().join("secret.md"),
        "---\ntitle: Secret\nstatus: todo\n---\n",
    )
    .unwrap();
    symlink(outside.path(), root.path().join("tasks/away")).unwrap();
    let error = source(root.path())
        .query_tasks(&TaskQuery::default(), &page())
        .await
        .unwrap_err();
    assert!(
        matches!(error, SourceError::Config { ref message } if message.contains("away") && message.contains("escapes configured root")),
        "{error:?}"
    );
}
