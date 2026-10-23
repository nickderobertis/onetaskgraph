//! A reader of the folder never opens a record that a write has left part-written.
//!
//! Every write this source makes of a task, a project or a document file is driven here in a
//! second process — this very test binary, run again as that writer — whose file size limit
//! (`ulimit -f`) is smaller than the record, so the write is stopped part-way through its
//! bytes. Then this process lists the folder and reads each record through the source, as any
//! other reader of the folder would, and every record it finds must be whole: as it was, or as
//! the write meant it to be, and never a staging file.
//!
//! Two ways of stopping the write: the limit's own signal, which kills the writer in the
//! middle of the `write` call and so holds it there for good; and the signal ignored, which
//! makes that call fail, so the source's own failure handling runs and must leave the record
//! as it was and no staging file behind.
//!
//! The limit is set by `sh`, so these run where there is one.
#![cfg(unix)]

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use onetaskgraph_local_md::STAGING_SUFFIX;
use onetaskgraph_plugin_api::{
    AssetName, AssetPayload, AssetWrite, CommentBody, CommentCopy, CommentOrigin, DocumentQuery,
    ItemWrite, MetadataKey, NativeId, NewComment, PageRequest, Priority, ProjectQuery,
    SecretResolver, SourceName, SourcePlugin, StatusCategory, TaskQuery, TaskRef, TaskSource,
    TaskUpdate,
};
use secrecy::SecretString;
use serde_json::{Value, json};

struct NoSecrets;
impl SecretResolver for NoSecrets {
    fn get(&self, _: &str) -> Option<SecretString> {
        None
    }
}

/// The variable naming the write the writer process makes.
const CASE: &str = "ONETASKGRAPH_LOCAL_MD_TORN_CASE";
/// The variable naming the folder the writer process writes in.
const ROOT: &str = "ONETASKGRAPH_LOCAL_MD_TORN_ROOT";
/// The test that is the writer when [`CASE`] is set, and does nothing otherwise.
const WRITER: &str = "torn_writer";
/// The exit status of a writer whose write landed whole: the limit held nothing, so the case
/// would prove nothing.
const LANDED: i32 = 3;
/// `SIGXFSZ`, the signal a write past the file size limit raises — 25 on Linux and macOS.
const SIGXFSZ: i32 = 25;
/// The writer's file size limit, in `ulimit -f` blocks: 512 bytes under a POSIX `sh`, 1024
/// under bash, so 4 or 8 KiB — far below every record, far above every front matter.
const LIMIT_BLOCKS: u32 = 8;

/// One record's content as it is before every write.
fn previous() -> String {
    filler("previous")
}

/// One record's content as a write that replaces it means it to be.
fn replacement() -> String {
    filler("replacement")
}

/// The content a write carrying assets gives its record: small enough to land under the
/// limit, so it is the asset after it, and then the putting back of the record, that the
/// limit stops.
const SMALL: &str = "SMALL replacement\n";

/// A content of about 24 KiB, opening and closing with lines naming `tag`, so any prefix of
/// it reads as neither it nor any other.
fn filler(tag: &str) -> String {
    let mut text = format!("BEGIN {tag}\n");
    for line in 0..400 {
        text.push_str(&format!(
            "{tag} line {line:04} of a record long enough to outgrow the limit\n"
        ));
    }
    text.push_str(&format!("END {tag}\n"));
    text
}

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

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
}

fn provenance() -> Value {
    json!({
        "template": "/templates/item.md",
        "digest": format!("sha256:{}", "a".repeat(64)),
        "body_digest": format!("sha256:{}", "b".repeat(64)),
        "answers_digest": format!("sha256:{}", "c".repeat(64)),
    })
}

fn answers() -> BTreeMap<String, Value> {
    BTreeMap::from([("goal".to_owned(), json!("ship it"))])
}

fn origin() -> CommentOrigin {
    CommentOrigin {
        task: "board:I_kw1".to_owned(),
        id: id("IC_7"),
        author: Some("ada".to_owned()),
        created_at: Some("2026-09-01T00:00:01Z".parse().unwrap()),
        updated_at: Some("2026-09-01T00:00:02Z".parse().unwrap()),
        url: Some("https://example.invalid/I_kw1#IC_7".to_owned()),
    }
}

/// Every write of a record's file this source makes, by the name a case is run under: each
/// path the audit of `src/lib.rs` found, for every kind it writes.
const CASES: &[&str] = &[
    // `write_entry` with no answers: a plain create and update, of each kind.
    "write_task",
    "create_task",
    "write_project",
    "create_project",
    "write_document",
    // `write_entry` with answers.
    "write_task_rendered",
    "write_project_rendered",
    "write_document_rendered",
    // `set_rendering`.
    "set_task_rendering",
    "set_project_rendering",
    "set_document_rendering",
    // `update_task`.
    "update_task",
    // `rewrite_front_entry`.
    "set_task_status",
    "set_project_status",
    "set_task_priority",
    "set_delivered_by",
    // `replace_content`.
    "set_task_content",
    // `set_metadata`.
    "set_task_metadata",
    "set_project_metadata",
    "set_document_metadata",
    // `TaskFile::rewrite`: the comments section.
    "add_comment",
    "edit_comment",
    "delete_comment",
    "write_comment_copy",
    // `install`: a record put back after its assets would not land.
    "write_task_with_assets",
];

/// Make the write `case` names in `root`, as the writer process.
fn write(case: &str, root: &Path) -> Result<(), String> {
    let source = source(root);
    let runtime = runtime();
    let key = MetadataKey::new("myapp.counter").unwrap();
    let new = replacement();
    let rendered = |metadata: &mut BTreeMap<String, Value>| {
        metadata.insert(MetadataKey::TEMPLATE_KEY.to_owned(), provenance());
    };
    let outcome = runtime.block_on(async {
        let task = || async { source.get_task(&id("alpha")).await.map(Option::unwrap) };
        let project = || async { source.get_project(&id("launch")).await.map(Option::unwrap) };
        let document = || async { source.get_document(&id("brief")).await.map(Option::unwrap) };
        let comment = || async {
            let listed = source.task_comments(&id("alpha"), &page()).await?.unwrap();
            Ok(listed.items[0].id.clone())
        };
        match case {
            "write_task" | "create_task" | "write_task_rendered" | "write_task_with_assets" => {
                let mut item = task().await?;
                item.content = Some(new.clone());
                let mut target = Some(id("alpha"));
                if case == "create_task" {
                    item.id = id("fresh");
                    item.title = "Fresh".to_owned();
                    target = None;
                }
                let write = |item| ItemWrite {
                    target: target.clone(),
                    item,
                    depends_on: Vec::new(),
                };
                if case == "write_task_rendered" {
                    rendered(&mut item.metadata);
                    source.write_task_rendered(&write(item), &answers()).await?;
                } else if case == "write_task_with_assets" {
                    item.content = Some(SMALL.to_owned());
                    let assets = AssetWrite {
                        assets: vec![AssetPayload::of(
                            AssetName::new("big.png").unwrap(),
                            vec![7; 64 * 1024],
                        )],
                        recorded_assets: None,
                    };
                    source
                        .write_task_with_assets(&write(item), None, &assets)
                        .await?;
                } else {
                    source.write_task(&write(item)).await?;
                }
            }
            "write_project" | "create_project" | "write_project_rendered" => {
                let mut item = project().await?;
                item.content = Some(new.clone());
                let mut target = Some(id("launch"));
                if case == "create_project" {
                    item.id = id("fresh");
                    item.title = "Fresh".to_owned();
                    target = None;
                }
                let mut write = ItemWrite {
                    target,
                    item,
                    depends_on: Vec::new(),
                };
                if case == "write_project_rendered" {
                    rendered(&mut write.item.metadata);
                    source.write_project_rendered(&write, &answers()).await?;
                } else {
                    source.write_project(&write).await?;
                }
            }
            "write_document" | "write_document_rendered" => {
                let mut item = document().await?;
                item.content = Some(new.clone());
                let mut write = ItemWrite {
                    target: Some(id("brief")),
                    item,
                    depends_on: Vec::new(),
                };
                if case == "write_document_rendered" {
                    rendered(&mut write.item.metadata);
                    source.write_document_rendered(&write, &answers()).await?;
                } else {
                    source.write_document(&write).await?;
                }
            }
            "set_task_rendering" => {
                source
                    .set_task_rendering(&id("alpha"), &new, &provenance(), &answers())
                    .await?;
            }
            "set_project_rendering" => {
                source
                    .set_project_rendering(&id("launch"), &new, &provenance(), &answers())
                    .await?;
            }
            "set_document_rendering" => {
                source
                    .set_document_rendering(&id("brief"), &new, &provenance(), &answers())
                    .await?;
            }
            "update_task" => {
                let update = TaskUpdate {
                    content: Some(new.clone()),
                    ..TaskUpdate::default()
                };
                source.update_task(&id("alpha"), &update).await?;
            }
            "set_task_status" => {
                source
                    .set_task_status(&id("alpha"), StatusCategory::Done)
                    .await?;
            }
            "set_project_status" => {
                source
                    .set_project_status(&id("launch"), StatusCategory::Done)
                    .await?;
            }
            "set_task_priority" => {
                source
                    .set_task_priority(&id("alpha"), Priority::High)
                    .await?;
            }
            "set_delivered_by" => {
                source
                    .set_delivered_by(&id("alpha"), &[TaskRef::new("plan:P-9").unwrap()])
                    .await?;
            }
            "set_task_content" => {
                source.set_task_content(&id("alpha"), &new).await?;
            }
            "set_task_metadata" => {
                source
                    .set_task_metadata(&id("alpha"), &key, &json!(2))
                    .await?;
            }
            "set_project_metadata" => {
                source
                    .set_project_metadata(&id("launch"), &key, &json!(2))
                    .await?;
            }
            "set_document_metadata" => {
                source
                    .set_document_metadata(&id("brief"), &key, &json!(2))
                    .await?;
            }
            "add_comment" => {
                let comment = NewComment {
                    body: CommentBody::new("A second word.")?,
                    author: None,
                };
                source.add_comment(&id("alpha"), &comment).await?;
            }
            "edit_comment" => {
                let held = comment().await?;
                source
                    .edit_comment(&id("alpha"), &held, &CommentBody::new("Edited.")?)
                    .await?;
            }
            "delete_comment" => {
                let held = comment().await?;
                source.delete_comment(&id("alpha"), &held).await?;
            }
            "write_comment_copy" => {
                let copy = CommentCopy {
                    body: CommentBody::new("Copied.")?,
                    origin: origin(),
                };
                source.write_comment_copy(&id("alpha"), None, &copy).await?;
            }
            other => panic!("no write is named {other:?}"),
        }
        Ok::<_, onetaskgraph_plugin_api::SourceError>(())
    });
    outcome.map_err(|error| format!("{error:?}"))
}

/// The writer process: make the write [`CASE`] names, exiting [`LANDED`] when it landed whole.
#[test]
fn torn_writer() {
    let (Some(case), Some(root)) = (std::env::var(CASE).ok(), std::env::var_os(ROOT)) else {
        return;
    };
    match write(&case, Path::new(&root)) {
        Ok(()) => std::process::exit(LANDED),
        Err(error) => eprintln!("{case}: the write failed: {error}"),
    }
}

/// A folder holding one task with one comment, one project and one document, each with
/// [`previous`] as its content.
fn folder() -> tempfile::TempDir {
    let root = tempfile::tempdir().expect("temporary notes");
    let content = previous();
    for (path, front) in [
        (
            "tasks/alpha.md",
            "title: Alpha\nstatus: todo\nproject: launch\nmetadata:\n  myapp.counter: 1\n",
        ),
        (
            "projects/launch.md",
            "title: Launch\nstatus: todo\nmetadata:\n  myapp.counter: 1\n",
        ),
        (
            "documents/brief.md",
            "title: Brief\nproject: launch\nmetadata:\n  myapp.counter: 1\n",
        ),
    ] {
        let path = root.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, format!("---\n{front}---\n{content}")).unwrap();
    }
    let comment = NewComment {
        body: CommentBody::new("A first word.").unwrap(),
        author: Some("ada".to_owned()),
    };
    runtime()
        .block_on(source(root.path()).add_comment(&id("alpha"), &comment))
        .expect("the comment is added")
        .expect("the task exists");
    root
}

/// How the writer is stopped part-way.
#[derive(Clone, Copy, PartialEq)]
enum Stop {
    /// Killed by the limit's signal in the middle of its `write`, so held there for good.
    Killed,
    /// The signal ignored, so its `write` fails and the source handles the failure.
    Failed,
}

/// Run the writer of `case` over `root` under the limit, stopped as `stop` says, and answer
/// with a refusal when it was not stopped that way.
fn stopped(case: &str, root: &Path, stop: Stop) -> Result<(), String> {
    let trap = match stop {
        Stop::Killed => "",
        Stop::Failed => "trap '' XFSZ; ",
    };
    let output = Command::new("sh")
        .arg("-c")
        .arg(format!(
            "ulimit -c 0; ulimit -f {LIMIT_BLOCKS}; {trap}exec \"$0\" \"$@\""
        ))
        .arg(std::env::current_exe().expect("this test binary"))
        .args(["--exact", WRITER, "--test-threads=1", "--nocapture"])
        .env(CASE, case)
        .env(ROOT, root)
        .stdin(Stdio::null())
        .output()
        .expect("the writer process starts");
    let said = String::from_utf8_lossy(&output.stderr);
    let held = match stop {
        Stop::Killed => output.status.signal() == Some(SIGXFSZ),
        Stop::Failed => output.status.success() && said.contains("the write failed"),
    };
    if held {
        Ok(())
    } else {
        Err(format!(
            "{case}: the limit did not stop the writer as intended ({}): {said}",
            output.status
        ))
    }
}

/// Every record the folder lists, read through the source: whole, or the reason it is not.
fn whole(root: &Path, case: &str) -> Result<(), String> {
    let reader = source(root);
    let wholes = [previous(), replacement(), SMALL.to_owned()];
    let check = |kind: &str, id: &NativeId, content: Option<&str>| {
        if id.0.contains(STAGING_SUFFIX) {
            return Err(format!("{case}: a staging file is listed as {kind} {id:?}"));
        }
        match content {
            // Compared without the line endings that close it: a record's last newline frames
            // what follows the content rather than being part of it, and no part of a filler
            // short of its whole reads the same however its newlines are trimmed.
            Some(content)
                if wholes.iter().any(|whole| {
                    whole.trim_end_matches('\n') == content.trim_end_matches('\n')
                }) =>
            {
                Ok(())
            }
            other => Err(format!(
                "{case}: {kind} {id:?} reads as an incomplete document: {} bytes of content, \
                 ending {:?}",
                other.map_or(0, str::len),
                other.map(|text| &text[text.len().saturating_sub(60)..]),
            )),
        }
    };
    runtime().block_on(async {
        let failed = |what: &str| {
            let what = what.to_owned();
            move |error| format!("{case}: the {what} failed: {error:?}")
        };
        let tasks = reader
            .query_tasks(&TaskQuery::default(), &page())
            .await
            .map_err(failed("task listing"))?;
        let projects = reader
            .query_projects(&ProjectQuery::default(), &page())
            .await
            .map_err(failed("project listing"))?;
        let documents = reader
            .query_documents(&DocumentQuery::default(), &page())
            .await
            .map_err(failed("document listing"))?;
        let ids = |listed: Vec<&NativeId>| -> Vec<String> {
            listed.into_iter().map(|id| id.0.clone()).collect()
        };
        for (held, ids) in [
            ("alpha", ids(tasks.items.iter().map(|t| &t.id).collect())),
            (
                "launch",
                ids(projects.items.iter().map(|p| &p.id).collect()),
            ),
            (
                "brief",
                ids(documents.items.iter().map(|d| &d.id).collect()),
            ),
        ] {
            if !ids.iter().any(|listed| listed == held) {
                return Err(format!(
                    "{case}: {held:?}, which the folder held, is not listed"
                ));
            }
        }
        for task in &tasks.items {
            check("task", &task.id, task.content.as_deref())?;
            let read = reader
                .get_task(&task.id)
                .await
                .map_err(failed("task read"))?;
            check("task", &task.id, read.and_then(|t| t.content).as_deref())?;
        }
        for project in &projects.items {
            check("project", &project.id, project.content.as_deref())?;
            let read = reader
                .get_project(&project.id)
                .await
                .map_err(failed("project read"))?;
            check(
                "project",
                &project.id,
                read.and_then(|p| p.content).as_deref(),
            )?;
        }
        for document in &documents.items {
            check("document", &document.id, document.content.as_deref())?;
            let read = reader
                .get_document(&document.id)
                .await
                .map_err(failed("document read"))?;
            check(
                "document",
                &document.id,
                read.and_then(|d| d.content).as_deref(),
            )?;
        }
        Ok(())
    })
}

/// Every file under `root`, by its path there.
fn files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut found = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else {
                let bytes = fs::read(&path).unwrap();
                found.insert(path.strip_prefix(root).unwrap().to_path_buf(), bytes);
            }
        }
    }
    found
}

/// Run every case stopped as `stop` says, `then` judging each folder afterwards, and fail
/// naming every case that did not hold.
fn every_case(
    stop: Stop,
    then: impl Fn(&str, &Path, &BTreeMap<PathBuf, Vec<u8>>) -> Result<(), String>,
) {
    let refused: Vec<String> = CASES
        .iter()
        .filter_map(|case| {
            let root = folder();
            let before = files(root.path());
            stopped(case, root.path(), stop)
                .and_then(|()| then(case, root.path(), &before))
                .err()
        })
        .collect();
    assert!(refused.is_empty(), "{}", refused.join("\n"));
}

#[test]
fn a_writer_stopped_part_way_shows_a_reader_only_whole_records() {
    every_case(Stop::Killed, |case, root, _| whole(root, case));
}

#[test]
fn a_write_failing_part_way_leaves_the_previous_record_and_no_staging_file() {
    every_case(Stop::Failed, |case, root, before| {
        whole(root, case)?;
        let after = files(root);
        if let Some(staging) = after
            .keys()
            .find(|path| path.to_string_lossy().ends_with(STAGING_SUFFIX))
        {
            return Err(format!(
                "{case}: a staging file is left behind: {staging:?}"
            ));
        }
        // A write carrying assets lands its record, small, before its asset fails; the record
        // put back is then the write the limit stops, so the record it leaves is the one that
        // write found — the small one — and the asset directory it made is taken back.
        let leaves_previous = case != "write_task_with_assets";
        if leaves_previous && &after != before {
            let changed: Vec<_> = after
                .keys()
                .chain(before.keys())
                .filter(|path| after.get(*path) != before.get(*path))
                .collect();
            return Err(format!(
                "{case}: the failed write did not leave the folder as it was: {changed:?} changed"
            ));
        }
        if !leaves_previous {
            let task = runtime()
                .block_on(source(root).get_task(&id("alpha")))
                .map_err(|error| format!("{case}: {error:?}"))?
                .ok_or_else(|| format!("{case}: the task is gone"))?;
            if task.content.as_deref() != Some(SMALL) {
                return Err(format!("{case}: the record is not the one the write found"));
            }
        }
        Ok(())
    });
}
