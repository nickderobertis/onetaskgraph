use std::fs;

use onetaskgraph_plugin_api::{
    Cursor, DependencyKind, Direction, LabelFilter, NativeId, PageRequest, ProjectFilter,
    ProjectQuery, SecretResolver, SourceError, SourceName, SourcePlugin, StatusCategory, TaskQuery,
    TaskSource, TextFields, TextQuery,
};
use secrecy::SecretString;

struct NoSecrets;
impl SecretResolver for NoSecrets {
    fn get(&self, _: &str) -> Option<SecretString> {
        None
    }
}

fn source() -> (tempfile::TempDir, Box<dyn TaskSource>) {
    let root = tempfile::tempdir().expect("temporary notes");
    fs::create_dir_all(root.path().join("tasks/nested")).expect("task folders");
    fs::create_dir_all(root.path().join("projects")).expect("project folder");
    fs::write(root.path().join("tasks/nested/a.md"), "---\nstatus: doing\nlabels: [Bug, {id: urgent-id, name: Urgent, color: red}]\nproject: p\ndepends_on:\n  - b\n  - id: c\n  - id: related\n    kind: related\n---\n# Alpha\nbody needle\n").expect("task");
    fs::write(
        root.path().join("tasks/b.md"),
        "---\ntitle: Beta\nstatus: done\n---\nbody\n",
    )
    .expect("task");
    fs::write(
        root.path().join("projects/p.md"),
        "---\ntitle: Project\nstatus: todo\ndepends_on: [q]\n---\nproject body\n",
    )
    .expect("project");
    let source = onetaskgraph_local_md::Plugin
        .build(
            &SourceName::new("notes").unwrap(),
            &serde_json::json!({"root":root.path()}),
            &NoSecrets,
        )
        .expect("source builds");
    (root, source)
}

fn page(limit: u32) -> PageRequest {
    PageRequest {
        cursor: None,
        limit,
    }
}

#[tokio::test]
async fn scans_real_markdown_filters_pages_and_walks_both_directions() {
    let (_root, source) = source();
    let task = source
        .get_task(&NativeId("nested/a".into()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(task.title, "Alpha");
    assert_eq!(task.project, Some(NativeId("p".into())));
    let query = TaskQuery {
        project: ProjectFilter::Orphans,
        ..TaskQuery::default()
    };
    assert_eq!(
        source
            .query_tasks(
                &query,
                &PageRequest {
                    cursor: None,
                    limit: 1
                }
            )
            .await
            .unwrap()
            .items[0]
            .id
            .0,
        "b"
    );
    let forward = source
        .task_dependencies(
            &NativeId("nested/a".into()),
            Direction::DependsOn,
            &PageRequest {
                cursor: None,
                limit: 10,
            },
        )
        .await
        .unwrap();
    assert_eq!(forward.items[0].to.id, "b");
    let reverse = source
        .task_dependencies(
            &NativeId("b".into()),
            Direction::DependedOnBy,
            &PageRequest {
                cursor: None,
                limit: 10,
            },
        )
        .await
        .unwrap();
    assert_eq!(reverse.items[0].from.id, "nested/a");
}

#[tokio::test]
async fn reads_windows_line_endings_from_a_real_markdown_file() {
    let root = tempfile::tempdir().expect("tempdir");
    fs::create_dir(root.path().join("tasks")).expect("tasks directory");
    fs::write(
        root.path().join("tasks/windows.md"),
        "---\r\ntitle: Windows task\r\nstatus: todo\r\n---\r\nBody from disk\r\n",
    )
    .expect("task");
    let source = onetaskgraph_local_md::Plugin
        .build(
            &SourceName::new("windows").unwrap(),
            &serde_json::json!({"root": root.path()}),
            &NoSecrets,
        )
        .expect("source builds");

    let tasks = source
        .query_tasks(&TaskQuery::default(), &page(10))
        .await
        .expect("query tasks");

    assert_eq!(tasks.items.len(), 1);
    assert_eq!(tasks.items[0].title, "Windows task");
    assert_eq!(tasks.items[0].content.as_deref(), Some("Body from disk"));
}

#[tokio::test]
async fn public_queries_cover_fields_labels_statuses_projects_and_paging() {
    let (_root, source) = source();
    assert_eq!(source.kind(), "local-md");
    let capabilities = source.capabilities();
    assert_eq!(capabilities.max_page_size, 200);
    assert!(source.health().await.unwrap().reachable);
    assert!(
        source
            .get_task(&NativeId("missing".into()))
            .await
            .unwrap()
            .is_none()
    );
    let project = source
        .get_project(&NativeId("p".into()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(project.title, "Project");

    for (fields, terms, expected) in [
        (TextFields::Title, "alpha", vec!["nested/a"]),
        (TextFields::Content, "needle", vec!["nested/a"]),
        (TextFields::TitleOrContent, "beta", vec!["b"]),
        (TextFields::Content, "absent", vec![]),
    ] {
        let result = source
            .query_tasks(
                &TaskQuery {
                    text: Some(TextQuery {
                        terms: terms.into(),
                        fields,
                    }),
                    ..TaskQuery::default()
                },
                &page(10),
            )
            .await
            .unwrap();
        assert_eq!(
            result
                .items
                .iter()
                .map(|item| item.id.0.as_str())
                .collect::<Vec<_>>(),
            expected
        );
    }

    let filtered = source
        .query_tasks(
            &TaskQuery {
                labels: LabelFilter {
                    any_of: vec!["bug".into(), "else".into()],
                    all_of: vec!["URGENT".into()],
                    none_of: vec!["ignored".into()],
                },
                statuses: vec![StatusCategory::InProgress],
                project: ProjectFilter::Is(NativeId("p".into())),
                ..TaskQuery::default()
            },
            &page(10),
        )
        .await
        .unwrap();
    assert_eq!(filtered.items[0].id.0, "nested/a");
    assert_eq!(filtered.items[0].labels[1].color.as_deref(), Some("red"));

    let excluded = source
        .query_tasks(
            &TaskQuery {
                labels: LabelFilter {
                    none_of: vec!["bug".into()],
                    ..LabelFilter::default()
                },
                ..TaskQuery::default()
            },
            &page(10),
        )
        .await
        .unwrap();
    assert_eq!(excluded.items.len(), 1);

    let projects = source
        .query_projects(
            &ProjectQuery {
                text: Some(TextQuery {
                    terms: "body".into(),
                    fields: TextFields::Content,
                }),
                statuses: vec![StatusCategory::Todo],
                ..ProjectQuery::default()
            },
            &page(10),
        )
        .await
        .unwrap();
    assert_eq!(projects.items.len(), 1);

    let labels = source.labels(&page(1)).await.unwrap();
    assert_eq!(labels.items.len(), 1);
    let labels = source
        .labels(&PageRequest {
            cursor: labels.next,
            limit: 2000,
        })
        .await
        .unwrap();
    assert!(!labels.items.is_empty());

    let dependencies = source
        .task_dependencies(
            &NativeId("nested/a".into()),
            Direction::DependsOn,
            &page(10),
        )
        .await
        .unwrap();
    assert_eq!(dependencies.items.len(), 3);
    assert_eq!(dependencies.items[2].kind, DependencyKind::Related);
    let projects = source
        .project_dependencies(&NativeId("q".into()), Direction::DependedOnBy, &page(10))
        .await
        .unwrap();
    assert_eq!(projects.items[0].from.id, "p");
}

#[tokio::test]
async fn public_results_expose_fallback_titles_unknown_statuses_deduplicated_labels_and_health() {
    let (root, source) = source();
    fs::write(
        root.path().join("tasks/fallback.md"),
        "---\nstatus: waiting\nlabels: [Bug, BUG]\n---\nbody without a heading\n",
    )
    .unwrap();

    let task = source
        .get_task(&NativeId("fallback".into()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(task.title, "fallback");
    assert_eq!(task.status.category, StatusCategory::Unknown);

    let labels = source.labels(&page(200)).await.unwrap();
    assert_eq!(
        labels
            .items
            .iter()
            .filter(|label| label.name.eq_ignore_ascii_case("bug"))
            .count(),
        1
    );

    let health = source.health().await.unwrap();
    let canonical_root = root.path().canonicalize().unwrap();
    assert_eq!(
        health.detail.as_deref(),
        Some(format!("reading Markdown under {}", canonical_root.display()).as_str())
    );
}

#[tokio::test]
async fn public_scan_rejects_unreadable_entries_and_clamps_pages_to_advertised_maximum() {
    let (root, source) = source();
    for index in 0..=onetaskgraph_local_md::MAX_PAGE_SIZE {
        fs::write(
            root.path().join(format!("tasks/page-{index}.md")),
            format!("---\nlabels: [label-{index}]\n---\n# Page {index}\n"),
        )
        .unwrap();
    }
    let labels = source.labels(&page(u32::MAX)).await.unwrap();
    assert_eq!(
        labels.items.len(),
        onetaskgraph_local_md::MAX_PAGE_SIZE as usize
    );
    assert!(labels.next.is_some());

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        symlink(
            root.path().join("missing-target"),
            root.path().join("tasks/dangling.md"),
        )
        .unwrap();
        let error = source
            .query_tasks(&TaskQuery::default(), &page(10))
            .await
            .unwrap_err();
        assert!(
            matches!(error, SourceError::Malformed { ref message } if message.contains("dangling.md"))
        );
    }
}

#[tokio::test]
async fn invalid_roots_documents_and_pages_are_refused_at_the_public_boundary() {
    let missing = tempfile::tempdir().unwrap().path().join("missing");
    let result = onetaskgraph_local_md::Plugin.build(
        &SourceName::new("notes").unwrap(),
        &serde_json::json!({"root": missing}),
        &NoSecrets,
    );
    assert!(
        matches!(result, Err(SourceError::Config { ref message }) if message.contains("notes") && message.contains("canonicalize"))
    );

    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("file");
    fs::write(&file, "not a directory").unwrap();
    let result = onetaskgraph_local_md::Plugin.build(
        &SourceName::new("notes").unwrap(),
        &serde_json::json!({"root": file}),
        &NoSecrets,
    );
    assert!(
        matches!(result, Err(SourceError::Config { ref message }) if message.contains("not a directory"))
    );

    let empty = tempfile::tempdir().unwrap();
    let empty_source = onetaskgraph_local_md::Plugin
        .build(
            &SourceName::new("empty").unwrap(),
            &serde_json::json!({"root": empty.path()}),
            &NoSecrets,
        )
        .unwrap();
    assert!(empty_source.health().await.unwrap().reachable);
    assert!(
        empty_source
            .query_tasks(&TaskQuery::default(), &page(10))
            .await
            .unwrap()
            .items
            .is_empty()
    );

    let (root, source) = source();
    fs::write(
        root.path().join("tasks/yaml.md"),
        "---\nlabels: [\n---\nbody\n",
    )
    .unwrap();
    let error = source.get_task(&NativeId("yaml".into())).await.unwrap_err();
    assert!(matches!(error, SourceError::Malformed { ref message } if message.contains("yaml.md")));

    for request in [
        PageRequest {
            cursor: None,
            limit: 0,
        },
        PageRequest {
            cursor: Some(Cursor("wrong".into())),
            limit: 1,
        },
        PageRequest {
            cursor: Some(Cursor("99".into())),
            limit: 1,
        },
    ] {
        assert!(
            source
                .query_tasks(&TaskQuery::default(), &request)
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn malformed_file_is_named_on_direct_read_while_other_rows_still_list() {
    let (root, source) = source();
    fs::write(root.path().join("tasks/bad.md"), "not front matter").unwrap();
    let error = source.get_task(&NativeId("bad".into())).await.unwrap_err();
    assert!(matches!(error, SourceError::Malformed { ref message } if message.contains("bad.md")));
    assert_eq!(
        source
            .query_tasks(
                &TaskQuery::default(),
                &PageRequest {
                    cursor: None,
                    limit: 10
                }
            )
            .await
            .unwrap()
            .items
            .len(),
        2
    );
}

#[cfg(unix)]
#[test]
fn a_symlink_escaping_the_root_is_a_configuration_error() {
    use std::os::unix::fs::symlink;
    let (root, source) = source();
    let outside = tempfile::tempdir().unwrap();
    fs::write(
        outside.path().join("secret.md"),
        "---\ntitle: secret\n---\n",
    )
    .unwrap();
    symlink(
        outside.path().join("secret.md"),
        root.path().join("tasks/escape.md"),
    )
    .unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let error = runtime
        .block_on(source.query_tasks(
            &TaskQuery::default(),
            &PageRequest {
                cursor: None,
                limit: 10,
            },
        ))
        .unwrap_err();
    assert!(matches!(error, SourceError::Config { ref message } if message.contains("escapes")));
    let error = runtime
        .block_on(source.get_task(&NativeId("escape".into())))
        .unwrap_err();
    assert!(matches!(error, SourceError::Config { ref message } if message.contains("escapes")));
}

#[cfg(unix)]
#[tokio::test]
async fn a_directory_symlink_cycle_is_a_configuration_error() {
    use std::os::unix::fs::symlink;
    let (root, source) = source();
    fs::create_dir(root.path().join("tasks/cycle-parent")).unwrap();
    symlink(
        root.path().join("tasks"),
        root.path().join("tasks/cycle-parent/cycle"),
    )
    .unwrap();

    let error = source
        .query_tasks(&TaskQuery::default(), &page(10))
        .await
        .unwrap_err();
    assert!(
        matches!(error, SourceError::Config { ref message } if message.contains("directory cycle"))
    );
}

#[cfg(unix)]
#[tokio::test]
async fn escaped_directory_and_non_utf8_document_are_refused() {
    use std::os::unix::fs::{PermissionsExt, symlink};

    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    symlink(outside.path(), root.path().join("tasks")).unwrap();
    let source = onetaskgraph_local_md::Plugin
        .build(
            &SourceName::new("notes").unwrap(),
            &serde_json::json!({"root": root.path()}),
            &NoSecrets,
        )
        .unwrap();
    let error = source.health().await.unwrap_err();
    assert!(matches!(error, SourceError::Config { ref message } if message.contains("escapes")));

    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("tasks")).unwrap();
    fs::write(root.path().join("tasks/non-utf8.md"), [0xff, 0xfe]).unwrap();
    fs::write(
        root.path().join("tasks/default.md"),
        "---\n{}\n---\n# Default status\n",
    )
    .unwrap();
    let source = onetaskgraph_local_md::Plugin
        .build(
            &SourceName::new("notes").unwrap(),
            &serde_json::json!({"root": root.path()}),
            &NoSecrets,
        )
        .unwrap();
    let error = source
        .get_task(&NativeId("non-utf8".into()))
        .await
        .unwrap_err();
    assert!(
        matches!(error, SourceError::Malformed { ref message } if message.contains("non-utf8.md"))
    );
    let task = source
        .get_task(&NativeId("default".into()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(task.status.category, StatusCategory::Todo);

    // macOS filesystems reject this byte sequence before the plugin can observe it.
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::ffi::OsStringExt;
        let non_utf8_name = std::ffi::OsString::from_vec(b"invalid-\xff.md".to_vec());
        fs::write(
            root.path().join("tasks").join(non_utf8_name),
            "---\n{}\n---\n",
        )
        .unwrap();
        let error = source
            .query_tasks(&TaskQuery::default(), &page(10))
            .await
            .unwrap_err();
        assert!(
            matches!(error, SourceError::Malformed { ref message } if message.contains("UTF-8 path"))
        );
    }

    let permissions = fs::metadata(root.path().join("tasks"))
        .unwrap()
        .permissions();
    fs::set_permissions(root.path().join("tasks"), fs::Permissions::from_mode(0o000)).unwrap();
    let error = source.health().await.unwrap_err();
    fs::set_permissions(root.path().join("tasks"), permissions).unwrap();
    assert!(
        matches!(error, SourceError::Unavailable { ref message } if message.contains("cannot read"))
    );
}

#[test]
fn schema_requires_root_and_refuses_unknown_fields() {
    assert_eq!(onetaskgraph_local_md::Plugin.kind(), "local-md");
    let schema = serde_json::to_value(onetaskgraph_local_md::Plugin.config_schema()).unwrap();
    assert!(
        schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x == "root")
    );
    let result = onetaskgraph_local_md::Plugin.build(
        &SourceName::new("work").unwrap(),
        &serde_json::json!({"roott":"notes"}),
        &NoSecrets,
    );
    let Err(error) = result else {
        panic!("unknown field must fail")
    };
    assert!(
        matches!(error, SourceError::Config { ref message } if message.contains("roott") && message.contains("work"))
    );
}
