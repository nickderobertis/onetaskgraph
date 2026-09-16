//! A narrow metadata write: one key of one record's front-matter `metadata:` block, and no
//! other byte of its file.
//!
//! Every test drives the real plugin over a real folder through its own `TaskSource`
//! interface and asserts on what a read answers and on the exact bytes the file holds
//! afterwards: a narrow write is a promise about the file, not only about what a read reports.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use onetaskgraph_plugin_api::{
    DocumentQuery, MetadataKey, NativeId, PageRequest, ProjectQuery, SecretResolver, SourceError,
    SourceName, SourcePlugin, TaskQuery, TaskSource,
};
use secrecy::SecretString;
use serde_json::{Value, json};

struct NoSecrets;
impl SecretResolver for NoSecrets {
    fn get(&self, _: &str) -> Option<SecretString> {
        None
    }
}

/// A folder named `work`, holding `files` under `tasks/`, `projects/` and `documents/`.
fn folder(files: &[(&str, &str)]) -> (tempfile::TempDir, Box<dyn TaskSource>) {
    let root = tempfile::tempdir().expect("temporary notes");
    for (relative, text) in files {
        let path = root.path().join(relative);
        fs::create_dir_all(path.parent().expect("a file has a folder")).expect("folders");
        fs::write(&path, text).expect("a file");
    }
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

fn key(value: &str) -> MetadataKey {
    MetadataKey::new(value).expect("a caller-owned key")
}

fn read(root: &tempfile::TempDir, relative: &str) -> String {
    fs::read_to_string(root.path().join(relative)).expect("the file reads")
}

fn page() -> PageRequest {
    PageRequest {
        limit: 200,
        cursor: None,
    }
}

/// Every path under `root`, relative to it, directories included.
fn listing(root: &Path) -> BTreeSet<PathBuf> {
    fn visit(root: &Path, dir: &Path, out: &mut BTreeSet<PathBuf>) {
        for entry in fs::read_dir(dir).expect("a folder lists") {
            let path = entry.expect("an entry").path();
            out.insert(path.strip_prefix(root).unwrap().to_path_buf());
            if path.is_dir() {
                visit(root, &path, out);
            }
        }
    }
    let mut out = BTreeSet::new();
    visit(root, root, &mut out);
    out
}

fn refused(error: SourceError) -> String {
    match error {
        SourceError::Refused { message } => message,
        other => panic!("expected a refusal naming the problem, got {other:?}"),
    }
}

/// Set `key` on the task `tasks/a.md` holds, starting from `before`, and assert both that the
/// file afterwards is exactly `after` and that the task reads back as it was with that one key
/// set — the answer being that read.
async fn task_edit(before: &str, name: &str, value: Value, after: &str) {
    let (root, source) = folder(&[("tasks/a.md", before)]);
    let mut expected = source.get_task(&id("a")).await.unwrap().expect("held");
    let answered = source
        .set_task_metadata(&id("a"), &key(name), &value)
        .await
        .unwrap()
        .expect("the task is held");
    assert_eq!(
        read(&root, "tasks/a.md"),
        after,
        "the file after setting {name}"
    );
    let reread = source.get_task(&id("a")).await.unwrap().expect("held");
    assert_eq!(answered, reread, "the answer is the record as read back");
    expected.metadata.insert(name.to_owned(), value);
    assert_eq!(
        reread, expected,
        "only {name} moved, and every other field and key is as it was"
    );
    assert_eq!(
        listing(root.path()),
        [PathBuf::from("tasks"), PathBuf::from("tasks/a.md")].into(),
        "no staging file is left behind"
    );
}

#[tokio::test]
async fn flow_json_entries_are_replaced_and_added_beside() {
    let before = "---\ntitle: Alpha\nstatus: todo\nmetadata:\n  \"onepipeline.review\": {\"a\":1}\n  \"myapp.list\": [1, 2]\nlabels: [Bug]\n---\n# Alpha\n\nBody.\n";
    task_edit(
        before,
        "onepipeline.review",
        json!({"a": 2, "b": "x"}),
        "---\ntitle: Alpha\nstatus: todo\nmetadata:\n  \"onepipeline.review\": {\"a\":2,\"b\":\"x\"}\n  \"myapp.list\": [1, 2]\nlabels: [Bug]\n---\n# Alpha\n\nBody.\n",
    )
    .await;
    task_edit(
        before,
        "myapp.added",
        json!(true),
        "---\ntitle: Alpha\nstatus: todo\nmetadata:\n  \"onepipeline.review\": {\"a\":1}\n  \"myapp.list\": [1, 2]\n  \"myapp.added\": true\nlabels: [Bug]\n---\n# Alpha\n\nBody.\n",
    )
    .await;
}

#[tokio::test]
async fn an_entry_on_the_last_line_of_the_front_matter_is_replaced_and_added_after() {
    let before = "---\nstatus: todo\nmetadata:\n    myapp.last: 1\n---\nBody.\n";
    task_edit(
        before,
        "myapp.last",
        json!("two"),
        "---\nstatus: todo\nmetadata:\n    \"myapp.last\": \"two\"\n---\nBody.\n",
    )
    .await;
    task_edit(
        before,
        "myapp.next",
        json!(null),
        "---\nstatus: todo\nmetadata:\n    myapp.last: 1\n    \"myapp.next\": null\n---\nBody.\n",
    )
    .await;
}

#[tokio::test]
async fn plain_and_single_quoted_entries_are_matched_by_their_decoded_key() {
    let before = "---\nstatus: todo\nmetadata:\n  myapp.note: hello\n  'myapp.quoted': 'it''s'\n  \"myapp.escaped\\u002ekey\": 3\ntitle: T\n---\n";
    task_edit(
        before,
        "myapp.note",
        json!("bye"),
        "---\nstatus: todo\nmetadata:\n  \"myapp.note\": \"bye\"\n  'myapp.quoted': 'it''s'\n  \"myapp.escaped\\u002ekey\": 3\ntitle: T\n---\n",
    )
    .await;
    task_edit(
        before,
        "myapp.quoted",
        json!(["it's"]),
        "---\nstatus: todo\nmetadata:\n  myapp.note: hello\n  \"myapp.quoted\": [\"it's\"]\n  \"myapp.escaped\\u002ekey\": 3\ntitle: T\n---\n",
    )
    .await;
    task_edit(
        before,
        "myapp.escaped.key",
        json!(4),
        "---\nstatus: todo\nmetadata:\n  myapp.note: hello\n  'myapp.quoted': 'it''s'\n  \"myapp.escaped.key\": 4\ntitle: T\n---\n",
    )
    .await;
    task_edit(
        before,
        "myapp.new",
        json!(1.5),
        "---\nstatus: todo\nmetadata:\n  myapp.note: hello\n  'myapp.quoted': 'it''s'\n  \"myapp.escaped\\u002ekey\": 3\n  \"myapp.new\": 1.5\ntitle: T\n---\n",
    )
    .await;
    task_edit(
        "---\nstatus: todo\nmetadata:\n  'myapp.it''s': 1\n  myapp.its: 2\n---\n",
        "myapp.it's",
        json!(3),
        "---\nstatus: todo\nmetadata:\n  \"myapp.it's\": 3\n  myapp.its: 2\n---\n",
    )
    .await;
}

#[tokio::test]
async fn a_nested_block_mapping_is_one_entry() {
    let before = "---\nstatus: todo\nmetadata:\n  myapp.first: 1\n  myapp.nested:\n    inner: 1\n    deeper:\n      x: y\ntitle: T\n---\nBody.\n";
    task_edit(
        before,
        "myapp.nested",
        json!({"inner": 2}),
        "---\nstatus: todo\nmetadata:\n  myapp.first: 1\n  \"myapp.nested\": {\"inner\":2}\ntitle: T\n---\nBody.\n",
    )
    .await;
    task_edit(
        before,
        "myapp.first",
        json!(0),
        "---\nstatus: todo\nmetadata:\n  \"myapp.first\": 0\n  myapp.nested:\n    inner: 1\n    deeper:\n      x: y\ntitle: T\n---\nBody.\n",
    )
    .await;
    task_edit(
        before,
        "myapp.beside",
        json!("z"),
        "---\nstatus: todo\nmetadata:\n  myapp.first: 1\n  myapp.nested:\n    inner: 1\n    deeper:\n      x: y\n  \"myapp.beside\": \"z\"\ntitle: T\n---\nBody.\n",
    )
    .await;
}

#[tokio::test]
async fn blank_lines_inside_a_block_scalar_belong_to_its_entry() {
    let before = "---\nstatus: todo\nmetadata:\n  myapp.story: |\n    line one\n\n    line three\n  myapp.tail: >\n    folded\n\n    paragraph\n\nlabels: [a]\n---\nBody.\n";
    task_edit(
        before,
        "myapp.story",
        json!("short"),
        "---\nstatus: todo\nmetadata:\n  \"myapp.story\": \"short\"\n  myapp.tail: >\n    folded\n\n    paragraph\n\nlabels: [a]\n---\nBody.\n",
    )
    .await;
    task_edit(
        before,
        "myapp.tail",
        json!("flat"),
        "---\nstatus: todo\nmetadata:\n  myapp.story: |\n    line one\n\n    line three\n  \"myapp.tail\": \"flat\"\n\nlabels: [a]\n---\nBody.\n",
    )
    .await;
    task_edit(
        before,
        "myapp.after",
        json!(1),
        "---\nstatus: todo\nmetadata:\n  myapp.story: |\n    line one\n\n    line three\n  myapp.tail: >\n    folded\n\n    paragraph\n  \"myapp.after\": 1\n\nlabels: [a]\n---\nBody.\n",
    )
    .await;
}

#[tokio::test]
async fn an_indentless_sequence_belongs_to_the_key_above_it() {
    let before = "---\nstatus: todo\nmetadata:\n  myapp.list:\n  - a\n  - b\n  myapp.other:\n  - c\ntitle: T\n---\n";
    task_edit(
        before,
        "myapp.list",
        json!(["z"]),
        "---\nstatus: todo\nmetadata:\n  \"myapp.list\": [\"z\"]\n  myapp.other:\n  - c\ntitle: T\n---\n",
    )
    .await;
    task_edit(
        before,
        "myapp.more",
        json!({}),
        "---\nstatus: todo\nmetadata:\n  myapp.list:\n  - a\n  - b\n  myapp.other:\n  - c\n  \"myapp.more\": {}\ntitle: T\n---\n",
    )
    .await;
}

#[tokio::test]
async fn a_windows_file_keeps_its_line_endings() {
    let before = "---\r\ntitle: Alpha\r\nstatus: todo\r\nmetadata:\r\n  myapp.a: 1\r\n  myapp.b:\r\n    deep: true\r\n---\r\nBody.\r\n";
    task_edit(
        before,
        "myapp.b",
        json!(2),
        "---\r\ntitle: Alpha\r\nstatus: todo\r\nmetadata:\r\n  myapp.a: 1\r\n  \"myapp.b\": 2\r\n---\r\nBody.\r\n",
    )
    .await;
    task_edit(
        before,
        "myapp.c",
        json!(3),
        "---\r\ntitle: Alpha\r\nstatus: todo\r\nmetadata:\r\n  myapp.a: 1\r\n  myapp.b:\r\n    deep: true\r\n  \"myapp.c\": 3\r\n---\r\nBody.\r\n",
    )
    .await;
    task_edit(
        "---\r\ntitle: Alpha\r\nstatus: todo\r\n---\r\nBody.\r\n",
        "myapp.c",
        json!(3),
        "---\r\ntitle: Alpha\r\nstatus: todo\r\nmetadata:\r\n  \"myapp.c\": 3\r\n---\r\nBody.\r\n",
    )
    .await;
}

#[tokio::test]
async fn a_file_with_no_metadata_block_gains_one_at_the_end_of_its_front_matter() {
    task_edit(
        "---\ntitle: Alpha\nstatus: todo\n---\n# Alpha\n",
        "myapp.first",
        json!({"n": [1, 2]}),
        "---\ntitle: Alpha\nstatus: todo\nmetadata:\n  \"myapp.first\": {\"n\":[1,2]}\n---\n# Alpha\n",
    )
    .await;
    task_edit(
        "---\n\n---\n# Alpha\n",
        "myapp.first",
        json!("x"),
        "---\nmetadata:\n  \"myapp.first\": \"x\"\n---\n# Alpha\n",
    )
    .await;
    task_edit(
        "---\nstatus: todo\nmetadata:\n  # nothing yet\ntitle: T\n---\n",
        "myapp.first",
        json!("x"),
        "---\nstatus: todo\nmetadata:\n  \"myapp.first\": \"x\"\n  # nothing yet\ntitle: T\n---\n",
    )
    .await;
}

#[tokio::test]
async fn a_project_and_a_document_are_edited_and_read_back_as_their_own_kind() {
    let project = "---\ntitle: Platform\nstatus: in progress\ndepends_on: [other]\nmetadata:\n  myapp.owner: ada\n---\n# Platform\n";
    let document = "---\ntitle: Design\nproject: platform\nmetadata:\n  myapp.reviewers:\n  - ada\n---\n# Design\n\nProse.\n";
    let (root, source) = folder(&[
        ("projects/platform.md", project),
        ("documents/design.md", document),
    ]);

    let mut expected = source
        .get_project(&id("platform"))
        .await
        .unwrap()
        .expect("held");
    let answered = source
        .set_project_metadata(&id("platform"), &key("myapp.phase"), &json!(2))
        .await
        .unwrap()
        .expect("the project is held");
    assert_eq!(
        read(&root, "projects/platform.md"),
        "---\ntitle: Platform\nstatus: in progress\ndepends_on: [other]\nmetadata:\n  myapp.owner: ada\n  \"myapp.phase\": 2\n---\n# Platform\n"
    );
    let reread = source
        .get_project(&id("platform"))
        .await
        .unwrap()
        .expect("held");
    assert_eq!(answered, reread);
    expected.metadata.insert("myapp.phase".to_owned(), json!(2));
    assert_eq!(reread, expected);

    let mut expected = source
        .get_document(&id("design"))
        .await
        .unwrap()
        .expect("held");
    let answered = source
        .set_document_metadata(&id("design"), &key("myapp.reviewers"), &json!(["grace"]))
        .await
        .unwrap()
        .expect("the document is held");
    assert_eq!(
        read(&root, "documents/design.md"),
        "---\ntitle: Design\nproject: platform\nmetadata:\n  \"myapp.reviewers\": [\"grace\"]\n---\n# Design\n\nProse.\n"
    );
    let reread = source
        .get_document(&id("design"))
        .await
        .unwrap()
        .expect("held");
    assert_eq!(answered, reread);
    assert!(matches!(
        reread.location,
        Some(onetaskgraph_plugin_api::Location::Path(_))
    ));
    expected
        .metadata
        .insert("myapp.reviewers".to_owned(), json!(["grace"]));
    assert_eq!(reread, expected);
    assert_eq!(
        listing(root.path()),
        [
            "documents",
            "documents/design.md",
            "projects",
            "projects/platform.md"
        ]
        .map(PathBuf::from)
        .into()
    );
}

/// What a no-write promise is checked against: the bytes, the inode and the modification time.
fn fingerprint(path: &Path) -> (Vec<u8>, u64, std::time::SystemTime) {
    let metadata = fs::metadata(path).expect("the file is there");
    #[cfg(unix)]
    let inode = std::os::unix::fs::MetadataExt::ino(&metadata);
    #[cfg(not(unix))]
    let inode = 0;
    (
        fs::read(path).expect("the file reads"),
        inode,
        metadata.modified().expect("a modification time"),
    )
}

#[tokio::test]
async fn setting_the_value_a_key_already_holds_writes_nothing() {
    // Each entry is already in exactly the shape a write would give it, so a rewrite would
    // leave the same bytes and only the inode and the time could tell.
    let (root, source) = folder(&[
        (
            "tasks/a.md",
            "---\nstatus: todo\nmetadata:\n  \"myapp.held\": {\"a\":[1,2]}\n---\n",
        ),
        (
            "projects/p.md",
            "---\nstatus: todo\nmetadata:\n  \"myapp.held\": \"x\"\n---\n",
        ),
        (
            "documents/d.md",
            "---\ntitle: D\nmetadata:\n  \"myapp.held\": 1\n---\n",
        ),
    ]);
    let files = ["tasks/a.md", "projects/p.md", "documents/d.md"].map(|f| root.path().join(f));
    let before: Vec<_> = files.iter().map(|f| fingerprint(f)).collect();
    let listed = listing(root.path());
    // A coarse filesystem clock would hide a rewrite inside the same tick.
    std::thread::sleep(std::time::Duration::from_millis(20));

    let task = source
        .set_task_metadata(&id("a"), &key("myapp.held"), &json!({"a": [1, 2]}))
        .await
        .unwrap()
        .expect("held");
    assert_eq!(task, source.get_task(&id("a")).await.unwrap().unwrap());
    let project = source
        .set_project_metadata(&id("p"), &key("myapp.held"), &json!("x"))
        .await
        .unwrap()
        .expect("held");
    assert_eq!(
        project,
        source.get_project(&id("p")).await.unwrap().unwrap()
    );
    let document = source
        .set_document_metadata(&id("d"), &key("myapp.held"), &json!(1))
        .await
        .unwrap()
        .expect("held");
    assert_eq!(
        document,
        source.get_document(&id("d")).await.unwrap().unwrap()
    );

    let after: Vec<_> = files.iter().map(|f| fingerprint(f)).collect();
    assert_eq!(before, after, "bytes, inode and modification time");
    assert_eq!(listing(root.path()), listed, "no file appeared anywhere");
}

/// Set `name` on a task whose file is `before`, expecting a refusal naming the file and saying
/// `reason`, with the file left byte for byte.
async fn refused_edit(before: &str, name: &str, value: Value, reason: &str) {
    let (root, source) = folder(&[("tasks/a.md", before)]);
    let path = root.path().join("tasks/a.md");
    let fingerprint_before = fingerprint(&path);
    let message = refused(
        source
            .set_task_metadata(&id("a"), &key(name), &value)
            .await
            .expect_err("the write is refused"),
    );
    let canonical = fs::canonicalize(&path).unwrap();
    assert!(
        message.starts_with(&format!(
            "{}: cannot set the metadata key `{name}` without changing anything else: ",
            canonical.display()
        )),
        "{message}"
    );
    assert!(message.contains(reason), "{message}");
    assert!(message.contains("; next: "), "{message}");
    assert_eq!(
        fingerprint(&path),
        fingerprint_before,
        "the file is untouched"
    );
    assert_eq!(
        listing(root.path()),
        [PathBuf::from("tasks"), PathBuf::from("tasks/a.md")].into()
    );
}

#[tokio::test]
async fn metadata_that_cannot_be_edited_narrowly_is_refused_and_left_alone() {
    refused_edit(
        "---\nstatus: todo\nmetadata: {myapp.a: 1}\n---\n",
        "myapp.b",
        json!(2),
        "its `metadata` is written on one line as `{myapp.a: 1}`",
    )
    .await;
    refused_edit(
        "---\nstatus: todo\nmetadata:\n  myapp.a: [1,\n\t2]\n---\n",
        "myapp.a",
        json!(2),
        "line 5 of the file is indented with a tab",
    )
    .await;
    refused_edit(
        "---\nstatus: todo\nmetadata:\n  myapp.a: 1\n  myapp.a: 2\n---\n",
        "myapp.a",
        json!(3),
        "its `metadata:` block holds `myapp.a` more than once",
    )
    .await;
    refused_edit(
        "---\nstatus: todo\nmetadata:\n  ? myapp.a\n  : 1\n---\n",
        "myapp.b",
        json!(3),
        "line 4 of the file is not an entry this source can read the key of",
    )
    .await;
    refused_edit(
        "---\nstatus: todo\nmetadata:\n    myapp.a: [1,\n  2]\n---\n",
        "myapp.c",
        json!(3),
        "line 5 of the file is not indented as the other entries of its `metadata:` block",
    )
    .await;
    // A quoted scalar continued in column 0 reads, but the block this source sees ends there.
    refused_edit(
        "---\nstatus: todo\nmetadata:\n  myapp.a: \"one\ntwo\"\n---\n",
        "myapp.b",
        json!(3),
        "the edited front matter would not read back: ",
    )
    .await;
    // YAML reads a NEL inside a double-quoted string as a space.
    refused_edit(
        "---\nstatus: todo\n---\n",
        "myapp.a",
        json!("x\u{85}y"),
        "does not read back through this source's YAML as the same value",
    )
    .await;
    // A comment in column 0 ends the block this source can see, while YAML reads on past it.
    refused_edit(
        "---\nstatus: todo\nmetadata:\n  myapp.a: 1\n# a note\n  myapp.b: 2\n---\n",
        "myapp.b",
        json!(3),
        "the edited file would read back as more than that one key changed",
    )
    .await;
    // Kept trailing blank lines of a `|+` scalar would move below the added entry.
    refused_edit(
        "---\nstatus: todo\nmetadata:\n  myapp.story: |+\n    text\n\ntitle: T\n---\n",
        "myapp.b",
        json!(3),
        "the edited file would read back as more than that one key changed",
    )
    .await;
}

#[tokio::test]
async fn a_missing_record_answers_none_for_every_kind() {
    let (root, source) = folder(&[("tasks/a.md", "---\nstatus: todo\n---\n")]);
    let (name, value) = (key("myapp.a"), json!(1));
    assert_eq!(
        source
            .set_task_metadata(&id("missing"), &name, &value)
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        source
            .set_project_metadata(&id("missing"), &name, &value)
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        source
            .set_document_metadata(&id("missing"), &name, &value)
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        listing(root.path()),
        [PathBuf::from("tasks"), PathBuf::from("tasks/a.md")].into()
    );
}

#[tokio::test]
async fn a_staging_file_is_never_listed_as_a_record() {
    use onetaskgraph_local_md::STAGING_SUFFIX;

    assert_eq!(STAGING_SUFFIX, ".onetaskgraph-staging");
    let record = "---\ntitle: Staged\nstatus: todo\nlabels: [x]\n---\n";
    // Exactly the names a write in process 4242 would stage these three records through.
    let staged = [
        format!("tasks/.a.md.4242-0{STAGING_SUFFIX}"),
        format!("projects/.p.md.4242-1{STAGING_SUFFIX}"),
        format!("documents/.d.md.4242-2{STAGING_SUFFIX}"),
    ];
    let (root, source) = folder(&[
        ("tasks/a.md", "---\ntitle: A\nstatus: todo\n---\n"),
        (&staged[0], record),
        (&staged[1], record),
        (&staged[2], "---\ntitle: D\n---\n"),
    ]);
    // A staging name that resolves to nothing is what a listing meets when a write renames its
    // staging file away between reading the folder and resolving the entry: it is skipped
    // rather than failing the whole listing.
    plant_dangling_staging_name(root.path());
    let tasks = source
        .query_tasks(&TaskQuery::default(), &page())
        .await
        .unwrap();
    assert_eq!(
        tasks
            .items
            .iter()
            .map(|t| t.id.as_str())
            .collect::<Vec<_>>(),
        ["a"]
    );
    assert!(
        source
            .query_projects(&ProjectQuery::default(), &page())
            .await
            .unwrap()
            .items
            .is_empty()
    );
    assert!(
        source
            .query_documents(&DocumentQuery::default(), &page())
            .await
            .unwrap()
            .items
            .is_empty()
    );
    let labels = source.labels(&page()).await.unwrap();
    assert!(labels.items.is_empty(), "{labels:?}");
}

/// A staging name that is a dangling symlink. Creating one on Windows needs a privilege the
/// runner does not grant, so there the listing is proven without it.
fn plant_dangling_staging_name(root: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        root.join("tasks/renamed-away"),
        root.join(format!(
            "tasks/.a.md.4242-3{}",
            onetaskgraph_local_md::STAGING_SUFFIX
        )),
    )
    .unwrap();
    #[cfg(not(unix))]
    let _ = root;
}

#[cfg(unix)]
#[tokio::test]
async fn a_write_that_cannot_land_leaves_the_file_and_no_staging_file() {
    use std::os::unix::fs::PermissionsExt;

    let before = "---\nstatus: todo\nmetadata:\n  myapp.a: 1\n---\n";
    let (root, source) = folder(&[("tasks/a.md", before)]);
    let tasks = root.path().join("tasks");
    fs::set_permissions(&tasks, fs::Permissions::from_mode(0o555)).unwrap();
    let outcome = source
        .set_task_metadata(&id("a"), &key("myapp.a"), &json!(2))
        .await;
    fs::set_permissions(&tasks, fs::Permissions::from_mode(0o755)).unwrap();
    match outcome {
        Err(SourceError::Unavailable { message }) => {
            assert!(message.starts_with("cannot write "), "{message}");
            assert!(message.contains("a.md"), "{message}");
        }
        other => panic!("expected the write to be unavailable, got {other:?}"),
    }
    assert_eq!(read(&root, "tasks/a.md"), before);
    assert_eq!(
        listing(root.path()),
        [PathBuf::from("tasks"), PathBuf::from("tasks/a.md")].into()
    );
}

#[cfg(unix)]
#[tokio::test]
async fn the_replacement_keeps_the_file_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let (root, source) = folder(&[("tasks/a.md", "---\nstatus: todo\n---\n")]);
    let path = root.path().join("tasks/a.md");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    source
        .set_task_metadata(&id("a"), &key("myapp.a"), &json!(1))
        .await
        .unwrap()
        .expect("held");
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o640
    );
}

#[test]
fn a_reader_never_sees_a_record_part_written() {
    // A long body makes a non-atomic write's window wide enough to land in.
    let body = "Prose that makes the file long enough to be caught half written.\n".repeat(400);
    let text = format!(
        "---\ntitle: Busy\nstatus: todo\nmetadata:\n  myapp.stable: kept\n  myapp.counter: 0\n---\n{body}"
    );
    let (root, writer) = folder(&[("tasks/a.md", &text)]);
    let reader = onetaskgraph_local_md::Plugin
        .build(
            &SourceName::new("work").unwrap(),
            &json!({ "root": root.path() }),
            &NoSecrets,
        )
        .expect("a second source over the same folder");
    let done = Arc::new(AtomicBool::new(false));
    let reading = {
        let done = Arc::clone(&done);
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap();
            let mut reads = 0_u32;
            while !done.load(Ordering::SeqCst) || reads == 0 {
                let task = runtime
                    .block_on(reader.get_task(&id("a")))
                    .expect("the record always reads")
                    .expect("the record is always there");
                assert_eq!(task.metadata["myapp.stable"], json!("kept"));
                let listed = runtime
                    .block_on(reader.query_tasks(&TaskQuery::default(), &page()))
                    .expect("the folder always lists");
                assert_eq!(listed.items.len(), 1, "the record is always listed, once");
                assert_eq!(listed.items[0].metadata["myapp.stable"], json!("kept"));
                reads += 1;
            }
            reads
        })
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    for counter in 1..=300 {
        let task = runtime
            .block_on(writer.set_task_metadata(&id("a"), &key("myapp.counter"), &json!(counter)))
            .unwrap()
            .expect("held");
        assert_eq!(task.metadata["myapp.counter"], json!(counter));
    }
    done.store(true, Ordering::SeqCst);
    let reads = reading
        .join()
        .expect("the reader never saw a broken record");
    assert!(reads > 0);
    assert_eq!(
        listing(root.path()),
        [PathBuf::from("tasks"), PathBuf::from("tasks/a.md")].into()
    );
}
