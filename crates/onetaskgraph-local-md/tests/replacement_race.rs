//! A folder listed while another process replaces its records in place.
//!
//! Within one process a listing and a replacement never overlap: the plugin holds them apart
//! itself. Across processes nothing does, so these tests put the writer in a second process
//! — this very test binary, run again as that writer — and list the folder from this one
//! for as long as it writes. On Windows a rename over a record used to make the listing
//! spell that record, for an instant, with a path that failed the root-containment check
//! (`a.md escapes configured root`, with both paths naming the same root); the listing now
//! resolves only links, so there is no such instant to land in. What a record depends on, and
//! the record read by its id, are found by the same rule, and are read here at the same
//! instants.

use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use onetaskgraph_plugin_api::{
    Direction, MetadataKey, NativeId, PageRequest, SecretResolver, SourceName, SourcePlugin,
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
        "---\ntitle: Busy\nstatus: todo\ndepends_on: [team/b]\n---\nThe body.\n",
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
        let edges = runtime
            .block_on(reader.task_dependencies(&id("a"), Direction::DependsOn, &page()))
            .unwrap_or_else(|error| {
                let _ = writer.kill();
                panic!("a dependency read during a replacement failed: {error:?}")
            });
        let to: Vec<String> = edges.items.iter().map(|e| e.to.id().to_string()).collect();
        assert_eq!(
            to,
            ["team/b"],
            "the record's one edge is read from its file"
        );
        let shown = runtime
            .block_on(reader.get_task(&id("a")))
            .unwrap_or_else(|error| {
                let _ = writer.kill();
                panic!("a read of the record during a replacement failed: {error:?}")
            })
            .expect("the record is held throughout");
        assert_eq!(shown.title, "Busy");
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

    use onetaskgraph_plugin_api::SourceError;

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

#[cfg(unix)]
#[tokio::test]
async fn a_dependency_read_follows_a_link_inside_its_folder_and_refuses_one_that_leaves_it() {
    use std::os::unix::fs::symlink;

    use onetaskgraph_plugin_api::SourceError;

    let root = tempfile::tempdir().expect("temporary notes");
    fs::create_dir_all(root.path().join("tasks/real")).unwrap();
    fs::create_dir_all(root.path().join("shared")).unwrap();
    let record = "---\ntitle: Linked\nstatus: todo\ndepends_on: [b]\n---\n";
    fs::write(root.path().join("tasks/real/c.md"), record).unwrap();
    fs::write(root.path().join("shared/d.md"), record).unwrap();
    symlink(
        root.path().join("tasks/real/c.md"),
        root.path().join("tasks/c.md"),
    )
    .unwrap();
    symlink(
        root.path().join("shared/d.md"),
        root.path().join("tasks/d.md"),
    )
    .unwrap();
    symlink(root.path().join("shared"), root.path().join("tasks/away")).unwrap();
    let source = source(root.path());
    let read = |name: &'static str| {
        let source = &source;
        async move {
            source
                .task_dependencies(&id(name), Direction::DependsOn, &page())
                .await
        }
    };
    // A link inside the folder is followed and its record named where it really is, as the
    // listing names it: the edge is `from` `real/c`, so the link's own name holds none.
    let to = |name: &'static str| async move {
        let edges = read(name)
            .await
            .expect("a link inside the folder is followed");
        edges
            .items
            .iter()
            .map(|e| e.to.id().to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(to("real/c").await, ["b"]);
    assert!(to("c").await.is_empty());
    for name in ["d", "away/d"] {
        let error = read(name).await.unwrap_err();
        assert!(
            matches!(error, SourceError::Config { ref message } if message.contains("escapes configured root")),
            "{name}: {error:?}"
        );
    }
    // A folder that is not there, and a segment that is a file, hold no record.
    for name in ["absent/e", "real/c.md/e", "real/c.md/f/e"] {
        let held_none = read(name).await.unwrap();
        assert!(held_none.items.is_empty(), "{name}");
    }
    // A link that leads nowhere is the author's to mend, as the listing reports it.
    symlink(
        root.path().join("shared/gone.md"),
        root.path().join("tasks/g.md"),
    )
    .unwrap();
    let error = read("g").await.unwrap_err();
    assert!(
        matches!(error, SourceError::Malformed { ref message } if message.contains("g.md")),
        "{error:?}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_dependency_read_in_a_folder_it_may_not_search_reports_the_folder_unavailable() {
    use std::os::unix::fs::PermissionsExt;

    use onetaskgraph_plugin_api::SourceError;

    let root = tempfile::tempdir().expect("temporary notes");
    let locked = root.path().join("tasks/locked");
    fs::create_dir_all(locked.join("inner")).unwrap();
    fs::write(locked.join("a.md"), "---\ntitle: A\nstatus: todo\n---\n").unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    // A process the filesystem does not refuse — run as root — cannot pose the refusal.
    let refused = fs::symlink_metadata(locked.join("a.md")).is_err();
    let source = source(root.path());
    let mut answers = Vec::new();
    for name in ["locked/a", "locked/inner/a"] {
        answers.push(
            source
                .task_dependencies(&id(name), Direction::DependsOn, &page())
                .await,
        );
    }
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    if !refused {
        return;
    }
    for answer in answers {
        let error = answer.unwrap_err();
        assert!(
            matches!(error, SourceError::Unavailable { ref message } if message.contains("cannot read") && message.contains("locked")),
            "{error:?}"
        );
    }
}
