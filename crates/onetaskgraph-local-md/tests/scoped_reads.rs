//! What a query scoped to one project reads, and what a file that vanishes mid-walk does.
//!
//! A task's project is a front-matter key rather than a folder, so a scoped query still lists
//! the whole folder — but a record whose front matter files it under another project is
//! passed over before it is parsed, and a file another process removes while the folder is
//! walked is skipped rather than reported as malformed. Every test drives the real plugin
//! over a real folder, the vanishing ones with a real second thread deleting files.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use onetaskgraph_plugin_api::{
    DocumentQuery, NativeId, PageRequest, ProjectFilter, SecretResolver, SourceError, SourceName,
    SourcePlugin, TaskQuery, TaskSource,
};
use secrecy::SecretString;
use serde_json::json;

struct NoSecrets;
impl SecretResolver for NoSecrets {
    fn get(&self, _: &str) -> Option<SecretString> {
        None
    }
}

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

fn page() -> PageRequest {
    PageRequest {
        limit: 200,
        cursor: None,
    }
}

fn in_project(id: &str) -> ProjectFilter {
    ProjectFilter::Is(NativeId(id.to_owned()))
}

async fn task_ids(source: &dyn TaskSource, project: ProjectFilter) -> Vec<String> {
    source
        .query_tasks(
            &TaskQuery {
                project,
                ..TaskQuery::default()
            },
            &page(),
        )
        .await
        .expect("the scoped query answers")
        .items
        .into_iter()
        .map(|task| task.id.0)
        .collect()
}

fn malformed(error: SourceError) -> String {
    match error {
        SourceError::Malformed { message } => message,
        other => panic!("expected the record to be reported malformed, got {other:?}"),
    }
}

/// Two projects' tasks, one orphan, and — filed under the other project, or under none at
/// all — the three kinds of file that do not parse as a task.
const MIXED: &[(&str, &str)] = &[
    ("tasks/mine/a.md", "---\ntitle: A\nproject: mine\n---\n"),
    ("tasks/mine/b.md", "---\ntitle: B\nproject: mine\n---\n"),
    ("tasks/theirs/c.md", "---\ntitle: C\nproject: theirs\n---\n"),
    ("tasks/loose.md", "---\ntitle: Loose\n---\n"),
    // No front matter at all: a note that is not a task, filed under no project.
    (
        "tasks/theirs/replies/note.md",
        "A reply, not a task, and no front matter.\n",
    ),
    // Front matter naming the other project and a key a task does not have.
    (
        "tasks/theirs/odd.md",
        "---\ntitle: Odd\nproject: theirs\nnot_a_task_key: 1\n---\n",
    ),
    // Front matter naming the other project and a status of the wrong shape.
    (
        "tasks/theirs/shaped.md",
        "---\nproject: theirs\nstatus: [todo, done]\n---\n",
    ),
];

#[tokio::test]
async fn a_query_scoped_to_a_project_answers_despite_records_elsewhere_that_do_not_parse() {
    let (_root, source) = folder(MIXED);
    assert_eq!(
        task_ids(source.as_ref(), in_project("mine")).await,
        ["mine/a", "mine/b"]
    );
    // The unscoped query is not narrowed: every record is its business, and one that does
    // not parse fails it as before.
    let error = source
        .query_tasks(&TaskQuery::default(), &page())
        .await
        .unwrap_err();
    let message = malformed(error);
    assert!(message.contains("theirs"), "{message}");
}

#[tokio::test]
async fn a_record_in_the_scoped_project_that_does_not_parse_still_fails_the_query_naming_it() {
    let mut files = MIXED.to_vec();
    files.push((
        "tasks/mine/broken.md",
        "---\ntitle: Broken\nproject: mine\nnot_a_task_key: 1\n---\n",
    ));
    let (_root, source) = folder(&files);
    let error = source
        .query_tasks(
            &TaskQuery {
                project: in_project("mine"),
                ..TaskQuery::default()
            },
            &page(),
        )
        .await
        .unwrap_err();
    let message = malformed(error);
    assert!(message.contains("broken.md"), "{message}");
    assert!(message.contains("not_a_task_key"), "{message}");
}

#[tokio::test]
async fn a_record_whose_project_cannot_be_read_fails_every_scoped_query() {
    // Front matter that is not YAML cannot be shown to be in another project, so it is not
    // passed over: it might be the scoped project's.
    let (_root, source) = folder(&[
        ("tasks/mine/a.md", "---\ntitle: A\nproject: mine\n---\n"),
        (
            "tasks/theirs/torn.md",
            "---\ntitle: [unclosed\nproject: theirs\n---\n",
        ),
        (
            "tasks/theirs/numbered.md",
            "---\nproject: 7\nstatus: [x]\n---\n",
        ),
    ]);
    for project in [in_project("mine"), ProjectFilter::Orphans] {
        let error = source
            .query_tasks(
                &TaskQuery {
                    project,
                    ..TaskQuery::default()
                },
                &page(),
            )
            .await
            .unwrap_err();
        let message = malformed(error);
        assert!(
            message.contains("torn.md") || message.contains("numbered.md"),
            "{message}"
        );
    }
}

#[tokio::test]
async fn an_orphan_query_passes_over_every_record_filed_under_a_project() {
    let (root, source) = folder(&[
        ("tasks/loose.md", "---\ntitle: Loose\n---\n"),
        ("tasks/also.md", "---\ntitle: Also\nproject: null\n---\n"),
        ("tasks/empty.md", "---\n\n---\nAn empty front matter.\n"),
        (
            "tasks/theirs/odd.md",
            "---\nproject: theirs\nnot_a_task_key: 1\n---\n",
        ),
    ]);
    assert_eq!(
        task_ids(source.as_ref(), ProjectFilter::Orphans).await,
        ["also", "empty", "loose"]
    );
    // A file with no front matter is filed under no project, so it is an orphan query's
    // business, and it fails that query.
    fs::write(root.path().join("tasks/note.md"), "No front matter.\n").unwrap();
    let error = source
        .query_tasks(
            &TaskQuery {
                project: ProjectFilter::Orphans,
                ..TaskQuery::default()
            },
            &page(),
        )
        .await
        .unwrap_err();
    assert!(malformed(error).contains("note.md"));
}

#[tokio::test]
async fn a_document_query_scoped_to_a_project_is_read_on_the_same_terms() {
    let (_root, source) = folder(&[
        (
            "documents/mine.md",
            "---\ntitle: Mine\nproject: mine\n---\n",
        ),
        (
            "documents/theirs.md",
            "---\ntitle: Theirs\nproject: theirs\nstatus: todo\n---\n",
        ),
    ]);
    let query = |project| DocumentQuery {
        project,
        ..DocumentQuery::default()
    };
    let listed = source
        .query_documents(&query(in_project("mine")), &page())
        .await
        .expect("the scoped query answers");
    let ids: Vec<&str> = listed.items.iter().map(|d| d.id.0.as_str()).collect();
    assert_eq!(ids, ["mine"]);
    let error = source
        .query_documents(&query(ProjectFilter::Any), &page())
        .await
        .unwrap_err();
    assert!(malformed(error).contains("theirs.md"));
}

/// While `churn` runs, create and delete tasks under `tasks/theirs/` — and whole folders of
/// them — on another thread as fast as it will go, so the walk keeps meeting entries that are
/// gone by the time it resolves or reads them. Every file is renamed into place, so each one
/// is only ever there whole: what the walk meets is a vanish, never a file half written.
fn while_files_vanish(root: &Path, churn: impl FnOnce()) {
    let done = Arc::new(AtomicBool::new(false));
    let deleting = {
        let done = Arc::clone(&done);
        let theirs = root.join("tasks/theirs");
        std::thread::spawn(move || {
            let place = |path: std::path::PathBuf| {
                let staged = path.with_extension("staged");
                fs::write(&staged, "---\ntitle: Brief\nproject: theirs\n---\n").unwrap();
                fs::rename(&staged, path).unwrap();
            };
            let mut cycles = 0_u32;
            while !done.load(Ordering::SeqCst) || cycles == 0 {
                let deep = theirs.join(format!("deep-{}", cycles % 4));
                fs::create_dir_all(&deep).unwrap();
                for n in 0..40 {
                    place(theirs.join(format!("brief-{n}.md")));
                    place(deep.join(format!("{n}.md")));
                }
                for n in 0..40 {
                    fs::remove_file(theirs.join(format!("brief-{n}.md"))).unwrap();
                }
                fs::remove_dir_all(&deep).unwrap();
                cycles += 1;
            }
            cycles
        })
    };
    churn();
    done.store(true, Ordering::SeqCst);
    assert!(deleting.join().expect("the deleting thread finished") > 0);
}

#[test]
fn a_file_that_vanishes_during_the_walk_is_skipped_rather_than_reported() {
    let (root, source) = folder(&[
        ("tasks/mine/a.md", "---\ntitle: A\nproject: mine\n---\n"),
        ("tasks/loose.md", "---\ntitle: Loose\n---\n"),
    ]);
    fs::create_dir_all(root.path().join("tasks/theirs")).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    while_files_vanish(root.path(), || {
        for _ in 0..300 {
            // Unscoped: every file the walk meets is either a whole task or gone, so every
            // listing answers, holding the two tasks that stay and whatever of the churn it
            // caught.
            let listed = runtime
                .block_on(source.query_tasks(&TaskQuery::default(), &page()))
                .expect("a file that vanished is skipped, not reported");
            let ids: Vec<&str> = listed.items.iter().map(|t| t.id.0.as_str()).collect();
            assert!(ids.contains(&"loose") && ids.contains(&"mine/a"), "{ids:?}");
            assert!(
                ids.iter()
                    .all(|id| ["loose", "mine/a"].contains(id) || id.starts_with("theirs/")),
                "{ids:?}"
            );
            // Scoped to another project: that project's churn is passed over.
            let ids = runtime.block_on(task_ids(source.as_ref(), in_project("mine")));
            assert_eq!(ids, ["mine/a"]);
        }
    });
}
