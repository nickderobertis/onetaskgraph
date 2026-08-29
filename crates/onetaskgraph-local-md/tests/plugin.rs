use std::fs;

use onetaskgraph_plugin_api::{
    Cursor, DependencyEdge, DependencyEndpoint, DependencyKind, Direction, ItemKind, ItemWrite,
    Label, LabelFilter, NativeId, PageRequest, Project, ProjectFilter, ProjectQuery,
    SecretResolver, SourceError, SourceName, SourcePlugin, Status, StatusCategory, Task, TaskQuery,
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
    assert_eq!(forward.items[0].to.id(), "b");
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
    assert_eq!(reverse.items[0].from.id(), "nested/a");
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
async fn front_matter_carries_typed_metadata_repositories_and_a_far_end_of_its_own() {
    let root = tempfile::tempdir().expect("temporary notes");
    fs::create_dir_all(root.path().join("tasks")).expect("task folder");
    fs::create_dir_all(root.path().join("projects")).expect("project folder");
    fs::write(
        root.path().join("tasks/near.md"),
        "---\nstatus: todo\nmetadata:\n  onepipeline.turn_budget: 12\n  caller.flags: [true, null]\n  caller.shape: {nested: \"value\"}\nrepositories: [github.com/acme/work, github.com/acme/docs]\ndepends_on:\n  - far\n  - id: \"elsewhere:P-9\"\n    item: project\n    kind: related\n---\nnear body\n",
    )
    .expect("task");
    let source = onetaskgraph_local_md::Plugin
        .build(
            &SourceName::new("notes").unwrap(),
            &serde_json::json!({"root":root.path()}),
            &NoSecrets,
        )
        .expect("source builds");

    let task = source
        .get_task(&NativeId("near".into()))
        .await
        .unwrap()
        .expect("the near task is read");
    assert_eq!(
        task.metadata["onepipeline.turn_budget"],
        serde_json::json!(12)
    );
    assert_eq!(
        task.metadata["caller.flags"],
        serde_json::json!([true, null])
    );
    assert_eq!(
        task.metadata["caller.shape"],
        serde_json::json!({"nested":"value"})
    );
    assert_eq!(
        task.repositories
            .iter()
            .map(onetaskgraph_plugin_api::Repository::as_str)
            .collect::<Vec<_>>(),
        ["github.com/acme/work", "github.com/acme/docs"]
    );

    let edges = source
        .task_dependencies(&NativeId("near".into()), Direction::DependsOn, &page(10))
        .await
        .unwrap();
    assert_eq!(edges.items[0].to.id(), "far");
    assert!(!edges.items[0].to.is_qualified());
    assert_eq!(edges.items[1].to.id(), "elsewhere:P-9");
    assert!(edges.items[1].to.is_qualified());
    assert_eq!(edges.items[1].to.kind, ItemKind::Project);
    assert_eq!(edges.items[1].from.kind, ItemKind::Task);
    assert_eq!(edges.items[1].kind, DependencyKind::Related);
}

#[tokio::test]
async fn front_matter_that_repeats_a_repository_or_misnames_a_far_end_is_refused_by_path() {
    for (name, front, expected) in [
        (
            "repeated",
            "repositories: [github.com/acme/work, github.com/acme/work]",
            "listed twice",
        ),
        (
            "unnormalized",
            "repositories: [https://github.com/acme/work]",
            "normalized repository origin",
        ),
        (
            "far",
            "depends_on: [{id: \"bad source:P-9\", item: project}]",
            "source name",
        ),
        ("unknown", "invented: true", "unknown field"),
    ] {
        let root = tempfile::tempdir().expect("temporary notes");
        fs::create_dir_all(root.path().join("tasks")).expect("task folder");
        fs::write(
            root.path().join("tasks/near.md"),
            format!("---\nstatus: todo\n{front}\n---\nbody\n"),
        )
        .expect("task");
        let source = onetaskgraph_local_md::Plugin
            .build(
                &SourceName::new("notes").unwrap(),
                &serde_json::json!({"root":root.path()}),
                &NoSecrets,
            )
            .expect("source builds");
        let error = source
            .get_task(&NativeId("near".into()))
            .await
            .expect_err(name);
        let message = format!("{error}");
        assert!(message.contains(expected), "{name}: {message}");
        assert!(message.contains("near.md"), "{name}: {message}");
    }
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
    assert_eq!(projects.items[0].from.id(), "p");
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
    assert_eq!(task.status.category, StatusCategory::Backlog);

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

/// A task on its way into a folder, with a caller-defined key of every JSON type.
fn outgoing(id: &str, title: &str, status: &str, category: StatusCategory) -> Task {
    Task {
        id: NativeId(id.into()),
        title: title.into(),
        content: Some("the engine core".into()),
        status: Status {
            category,
            name: status.into(),
        },
        labels: vec![Label {
            id: NativeId("L-1".into()),
            name: "bug".into(),
            color: Some("red".into()),
        }],
        project: Some(NativeId("p".into())),
        // The destination's own, and never written.
        url: Some("https://example.invalid/ignored".into()),
        created_at: None,
        updated_at: None,
        metadata: [
            ("caller.count".to_owned(), serde_json::json!(3)),
            ("caller.flag".to_owned(), serde_json::json!(true)),
            ("caller.absent".to_owned(), serde_json::Value::Null),
            ("caller.text".to_owned(), serde_json::json!("3")),
            (
                "caller.shape".to_owned(),
                serde_json::json!({"nested": [1, "two", false]}),
            ),
        ]
        .into_iter()
        .collect(),
        repositories: vec![
            serde_json::from_value(serde_json::json!("github.com/nickderobertis/onetaskgraph"))
                .expect("a normalized origin"),
        ],
    }
}

#[tokio::test]
async fn a_written_task_reads_back_with_every_value_and_json_type_intact() {
    let (_root, source) = source();
    assert!(source.writes().is_supported());

    let written = source
        .write_task(&ItemWrite {
            target: None,
            item: outgoing("T-1", "Alpha engine", "doing", StatusCategory::InProgress),
            depends_on: vec![DependencyEdge {
                from: DependencyEndpoint::from_native(NativeId("T-1".into()), ItemKind::Task),
                to: DependencyEndpoint::new("elsewhere:P-9".into(), ItemKind::Project)
                    .expect("a qualified endpoint"),
                kind: DependencyKind::Related,
            }],
        })
        .await
        .expect("the folder takes the write");
    assert_eq!(written, NativeId("T-1".into()));

    let read = source
        .get_task(&written)
        .await
        .expect("the folder answers")
        .expect("the task is there");
    assert_eq!(read.title, "Alpha engine");
    assert_eq!(read.content.as_deref(), Some("the engine core"));
    assert_eq!(read.status.name, "doing");
    assert_eq!(read.status.category, StatusCategory::InProgress);
    assert_eq!(read.labels[0].name, "bug");
    assert_eq!(read.labels[0].color.as_deref(), Some("red"));
    assert_eq!(read.project, Some(NativeId("p".into())));
    assert_eq!(
        read.repositories[0].as_str(),
        "github.com/nickderobertis/onetaskgraph"
    );
    // Value and JSON type alike: a 3 does not come back as "3", and a null is not absent.
    assert_eq!(read.metadata["caller.count"], serde_json::json!(3));
    assert_eq!(read.metadata["caller.text"], serde_json::json!("3"));
    assert_eq!(read.metadata["caller.flag"], serde_json::json!(true));
    assert_eq!(read.metadata["caller.absent"], serde_json::Value::Null);
    assert_eq!(
        read.metadata["caller.shape"],
        serde_json::json!({"nested": [1, "two", false]})
    );
    // `url` is the destination's own and is never written.
    assert_eq!(read.url, None);

    let edges = source
        .task_dependencies(&written, Direction::DependsOn, &page(10))
        .await
        .expect("the folder answers")
        .items;
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].to.id(), "elsewhere:P-9");
    assert_eq!(edges[0].to.kind, ItemKind::Project);
    assert_eq!(edges[0].kind, DependencyKind::Related);
}

#[tokio::test]
async fn a_write_updates_the_document_it_targets_and_refuses_one_that_is_not_there() {
    let (_root, source) = source();
    let updated = source
        .write_task(&ItemWrite {
            target: Some(NativeId("b".into())),
            item: outgoing("T-1", "Renamed", "done", StatusCategory::Done),
            depends_on: Vec::new(),
        })
        .await
        .expect("the folder takes the write");
    assert_eq!(updated, NativeId("b".into()));
    assert_eq!(
        source
            .get_task(&NativeId("b".into()))
            .await
            .unwrap()
            .unwrap()
            .title,
        "Renamed"
    );

    let Err(SourceError::Refused { message }) = source
        .write_task(&ItemWrite {
            target: Some(NativeId("absent".into())),
            item: outgoing("T-1", "Alpha", "done", StatusCategory::Done),
            depends_on: Vec::new(),
        })
        .await
    else {
        panic!("a target this folder does not hold must be refused rather than created");
    };
    assert!(
        message.contains("names no tasks document here"),
        "{message}"
    );
    assert!(message.contains("--recreate"), "{message}");
}

#[tokio::test]
async fn a_create_never_takes_a_name_something_already_answers_to() {
    let (_root, source) = source();
    let first = source
        .write_task(&ItemWrite {
            target: None,
            item: outgoing("b", "A second Beta", "done", StatusCategory::Done),
            depends_on: Vec::new(),
        })
        .await
        .expect("the folder takes the write");
    assert_eq!(first, NativeId("b-2".into()));
    assert_eq!(
        source
            .get_task(&NativeId("b".into()))
            .await
            .unwrap()
            .unwrap()
            .title,
        "Beta",
        "the document already there is left exactly as it is"
    );
}

#[tokio::test]
async fn a_status_this_folder_would_read_as_something_else_refuses_naming_the_field() {
    let (_root, source) = source();
    let Err(SourceError::Refused { message }) = source
        .write_task(&ItemWrite {
            target: None,
            // This source's mapping has no entry for "Shipped", so writing it would have
            // the folder read the task back as `unknown` — a narrowing nothing above the
            // plugin could see.
            item: outgoing("T-9", "Alpha", "Shipped", StatusCategory::Done),
            depends_on: Vec::new(),
        })
        .await
    else {
        panic!("a status this folder cannot represent must refuse rather than drop it");
    };
    assert!(message.contains("the field `status`"), "{message}");
    assert!(message.contains("status_mapping"), "{message}");
    assert!(message.contains("\"Shipped\""), "{message}");
}

#[tokio::test]
async fn a_project_write_carries_its_own_fields_and_an_id_no_path_can_be_made_of_refuses() {
    let (_root, source) = source();
    let written = source
        .write_project(&ItemWrite {
            target: None,
            item: Project {
                id: NativeId("P/../../escape".into()),
                title: "Engine".into(),
                content: None,
                status: Status {
                    category: StatusCategory::Todo,
                    name: "todo".into(),
                },
                labels: Vec::new(),
                url: None,
                created_at: None,
                updated_at: None,
                metadata: [("caller.key".to_owned(), serde_json::json!(1))]
                    .into_iter()
                    .collect(),
                repositories: Vec::new(),
            },
            depends_on: Vec::new(),
        })
        .await
        .expect("the folder takes the write");
    // Every path-significant character is replaced rather than obeyed, so nothing a
    // native id spells can reach outside the configured root.
    assert_eq!(written, NativeId("P/escape".into()));
    let read = source.get_project(&written).await.unwrap().unwrap();
    assert_eq!(read.title, "Engine");
    assert_eq!(read.metadata["caller.key"], serde_json::json!(1));

    let Err(SourceError::Refused { message }) = source
        .write_project(&ItemWrite {
            target: None,
            item: Project {
                id: NativeId("///".into()),
                ..read
            },
            depends_on: Vec::new(),
        })
        .await
    else {
        panic!("an id with no usable character must be refused");
    };
    assert!(message.contains("no character a file name"), "{message}");
}

#[tokio::test]
async fn a_status_this_folder_reads_as_another_category_names_both_in_its_refusal() {
    // One refusal per category the folder would have read it as, because the message is
    // what a user acts on and "not what you meant" is no help without saying what.
    let (root, _source) = source();
    let source = onetaskgraph_local_md::Plugin
        .build(
            &SourceName::new("notes").unwrap(),
            &serde_json::json!({
                "root": root.path(),
                "status_mapping": {
                    "later": "backlog",
                    "queued": "todo",
                    "doing": "in-progress",
                    "dropped": "cancelled",
                },
            }),
            &NoSecrets,
        )
        .expect("source builds");

    for (name, reads_as, wanted, category) in [
        ("later", "backlog", "todo", StatusCategory::Todo),
        ("queued", "todo", "in-progress", StatusCategory::InProgress),
        (
            "doing",
            "in-progress",
            "cancelled",
            StatusCategory::Cancelled,
        ),
        ("dropped", "cancelled", "backlog", StatusCategory::Backlog),
    ] {
        let Err(SourceError::Refused { message }) = source
            .write_task(&ItemWrite {
                target: None,
                item: outgoing("T-9", "Alpha", name, category),
                depends_on: Vec::new(),
            })
            .await
        else {
            panic!("{name} would be read as {reads_as} rather than {wanted}");
        };
        assert!(
            message.contains(&format!("as {reads_as}, not {wanted}")),
            "{message}"
        );
    }
}

#[tokio::test]
async fn a_written_edge_carries_its_kind_and_the_level_of_its_far_end() {
    let (_root, source) = source();
    let written = source
        .write_task(&ItemWrite {
            target: None,
            item: outgoing("T-8", "Alpha", "doing", StatusCategory::InProgress),
            depends_on: vec![
                DependencyEdge {
                    from: DependencyEndpoint::from_native(NativeId("T-8".into()), ItemKind::Task),
                    to: DependencyEndpoint::from_native(NativeId("b".into()), ItemKind::Task),
                    kind: DependencyKind::Blocks,
                },
                DependencyEdge {
                    from: DependencyEndpoint::from_native(NativeId("T-8".into()), ItemKind::Task),
                    to: DependencyEndpoint::from_native(NativeId("p".into()), ItemKind::Project),
                    kind: DependencyKind::Related,
                },
            ],
        })
        .await
        .expect("the folder takes the write");

    let edges = source
        .task_dependencies(&written, Direction::DependsOn, &page(10))
        .await
        .expect("the folder answers")
        .items;
    assert_eq!(
        edges
            .iter()
            .map(|edge| (edge.to.id().to_owned(), edge.to.kind, edge.kind))
            .collect::<Vec<_>>(),
        vec![
            ("b".to_owned(), ItemKind::Task, DependencyKind::Blocks),
            ("p".to_owned(), ItemKind::Project, DependencyKind::Related),
        ]
    );
}

#[tokio::test]
async fn a_path_the_filesystem_will_not_take_is_reported_rather_than_silently_lost() {
    let (root, source) = source();
    // A file where the folder for a nested id has to go, so creating that folder fails.
    fs::write(root.path().join("tasks/blocked"), "not a directory").expect("the blocker");
    let Err(SourceError::Unavailable { message }) = source
        .write_task(&ItemWrite {
            target: None,
            item: outgoing(
                "blocked/child",
                "Alpha",
                "doing",
                StatusCategory::InProgress,
            ),
            depends_on: Vec::new(),
        })
        .await
    else {
        panic!("a folder the filesystem will not create must be reported");
    };
    assert!(message.contains("cannot create"), "{message}");

    // A directory where the document has to go, so writing the document fails. It exists,
    // so the update path reaches the write rather than refusing the target.
    fs::create_dir(root.path().join("tasks/occupied.md")).expect("the occupying directory");
    let Err(SourceError::Unavailable { message }) = source
        .write_task(&ItemWrite {
            target: Some(NativeId("occupied".into())),
            item: outgoing("occupied", "Alpha", "doing", StatusCategory::InProgress),
            depends_on: Vec::new(),
        })
        .await
    else {
        panic!("a document the filesystem will not write must be reported");
    };
    assert!(message.contains("cannot write"), "{message}");
}

#[cfg(unix)]
#[tokio::test]
async fn a_target_whose_document_is_a_link_out_of_the_root_is_refused() {
    use std::os::unix::fs::symlink;
    let (root, source) = source();
    let outside = tempfile::tempdir().expect("a folder outside the root");
    fs::write(
        outside.path().join("elsewhere.md"),
        "---\ntitle: Elsewhere\nstatus: doing\n---\nbody\n",
    )
    .expect("the document outside");
    symlink(
        outside.path().join("elsewhere.md"),
        root.path().join("tasks/escape.md"),
    )
    .expect("the escaping link");

    let Err(SourceError::Config { message }) = source
        .write_task(&ItemWrite {
            target: Some(NativeId("escape".into())),
            item: outgoing("escape", "Alpha", "doing", StatusCategory::InProgress),
            depends_on: Vec::new(),
        })
        .await
    else {
        panic!("a write that would land outside the configured root must be refused");
    };
    assert!(message.contains("escapes configured root"), "{message}");
}

#[tokio::test]
async fn a_folder_that_already_answers_to_every_name_a_create_could_take_refuses() {
    let (root, source) = source();
    // The create tries the id it was given and then a thousand suffixes; a folder holding
    // all of them has no name left to give, and saying so beats writing over one.
    fs::write(
        root.path().join("tasks/taken.md"),
        "---\ntitle: Taken\nstatus: doing\n---\nbody\n",
    )
    .expect("the first name");
    for attempt in 2..=1_000 {
        fs::write(
            root.path().join(format!("tasks/taken-{attempt}.md")),
            "---\ntitle: Taken\nstatus: doing\n---\nbody\n",
        )
        .expect("a taken name");
    }

    let Err(SourceError::Refused { message }) = source
        .write_task(&ItemWrite {
            target: None,
            item: outgoing("taken", "Alpha", "doing", StatusCategory::InProgress),
            depends_on: Vec::new(),
        })
        .await
    else {
        panic!("a folder with no name left must refuse rather than write over one");
    };
    assert!(
        message.contains("every name from taken to taken-1000"),
        "{message}"
    );
}

/// A document that says nothing about its status lands in the backlog, and `draft` is a
/// status a document may simply state.
///
/// The default is what a user gets for writing a file and nothing else, so it is asserted
/// on the front matter a user would actually write rather than on the mapping table: an
/// omitted `status:` reads `backlog`, and `todo` continues to read `todo` — this source's
/// spelling of `todo` did not move when the default did.
#[tokio::test]
async fn an_unstated_status_reads_as_backlog_and_draft_is_read_as_an_ordinary_status() {
    let root = tempfile::tempdir().expect("temporary notes");
    fs::create_dir_all(root.path().join("tasks")).expect("task folder");
    fs::write(
        root.path().join("tasks/unstated.md"),
        "---\n{}\n---\n# Unstated\n",
    )
    .expect("task");
    fs::write(
        root.path().join("tasks/stated.md"),
        "---\nstatus: todo\n---\n# Stated\n",
    )
    .expect("task");
    fs::write(
        root.path().join("tasks/sketch.md"),
        "---\nstatus: Draft\n---\n# Sketch\n",
    )
    .expect("task");
    let source = onetaskgraph_local_md::Plugin
        .build(
            &SourceName::new("notes").unwrap(),
            &serde_json::json!({"root": root.path()}),
            &NoSecrets,
        )
        .expect("source builds");

    let unstated = source
        .get_task(&NativeId("unstated".into()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(unstated.status.category, StatusCategory::Backlog);
    assert_eq!(unstated.status.name, "backlog");

    let stated = source
        .get_task(&NativeId("stated".into()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stated.status.category, StatusCategory::Todo);
    assert_eq!(stated.status.name, "todo");

    // Read, not refused, and case-insensitively like every other status name.
    let sketch = source
        .get_task(&NativeId("sketch".into()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(sketch.status.category, StatusCategory::Draft);
    assert_eq!(sketch.status.name, "Draft");

    // And the category is a filter like any other, so a draft is selectable on its own.
    let drafts = source
        .query_tasks(
            &TaskQuery {
                statuses: vec![StatusCategory::Draft],
                ..Default::default()
            },
            &page(10),
        )
        .await
        .unwrap();
    assert_eq!(
        drafts
            .items
            .iter()
            .map(|task| task.id.as_str())
            .collect::<Vec<_>>(),
        vec!["sketch"]
    );

    // A draft is also a status this source can write back, which is what keeps a copy
    // into a Markdown folder from refusing the field it just read.
    let written = source
        .write_task(&ItemWrite {
            target: Some(NativeId("sketch".into())),
            item: Task {
                id: NativeId("sketch".into()),
                title: "Sketch".into(),
                content: None,
                status: Status {
                    category: StatusCategory::Draft,
                    name: "draft".into(),
                },
                labels: Vec::new(),
                project: None,
                url: None,
                created_at: None,
                updated_at: None,
                metadata: Default::default(),
                repositories: Vec::new(),
            },
            depends_on: Vec::new(),
        })
        .await
        .expect("the folder takes a draft back");
    assert_eq!(written, NativeId("sketch".into()));
    assert_eq!(
        source
            .get_task(&written)
            .await
            .unwrap()
            .unwrap()
            .status
            .category,
        StatusCategory::Draft
    );
}

/// The folder every capability assertion below reads.
///
/// Two projects with one task filed under each, one task filed under neither, one label
/// on one of the three and a closed status on another: that shape is what makes an
/// honoured predicate and an ignored one *different answers*. A folder holding one
/// project, or one where every task carries the label, answers a filter the same way
/// whether or not the source applies it.
fn capability_folder() -> (tempfile::TempDir, Box<dyn TaskSource>) {
    let root = tempfile::tempdir().expect("temporary notes");
    fs::create_dir_all(root.path().join("tasks")).expect("task folder");
    fs::create_dir_all(root.path().join("projects")).expect("project folder");
    fs::write(
        root.path().join("projects/alpha.md"),
        "---\ntitle: Alpha project\nstatus: todo\n---\nthe first plan\n",
    )
    .expect("project");
    fs::write(
        root.path().join("projects/beta.md"),
        "---\ntitle: Beta project\nstatus: todo\ndepends_on: [alpha]\n---\nthe second plan\n",
    )
    .expect("project");
    fs::write(
        root.path().join("tasks/first.md"),
        "---\ntitle: Alpha task\nstatus: todo\nlabels: [Backend]\nproject: alpha\n---\nordinary body\n",
    )
    .expect("task");
    fs::write(
        root.path().join("tasks/second.md"),
        "---\ntitle: Beta task\nstatus: todo\nlabels: [Frontend]\nproject: beta\ndepends_on: [first]\n---\nordinary body\n",
    )
    .expect("task");
    fs::write(
        root.path().join("tasks/orphan.md"),
        "---\ntitle: Loose task\nstatus: done\n---\na needle in this body alone\n",
    )
    .expect("task");
    let source = onetaskgraph_local_md::Plugin
        .build(
            &SourceName::new("notes").unwrap(),
            &serde_json::json!({"root":root.path()}),
            &NoSecrets,
        )
        .expect("source builds");
    (root, source)
}

async fn task_ids(source: &dyn TaskSource, query: &TaskQuery) -> Vec<String> {
    let mut ids = source
        .query_tasks(query, &page(50))
        .await
        .expect("the folder answers this query")
        .items
        .into_iter()
        .map(|task| task.id.0)
        .collect::<Vec<_>>();
    ids.sort();
    ids
}

#[tokio::test]
async fn every_declared_capability_is_applied_to_the_real_folder() {
    let (root, source) = capability_folder();
    // Every field of the contract's `Capabilities`, spelled out: the struct has no
    // `Default`, so a field added to the contract fails to compile here rather than going
    // unasserted, and each one is driven against the folder below.
    assert_eq!(
        source.capabilities(),
        onetaskgraph_plugin_api::Capabilities {
            projects: onetaskgraph_plugin_api::Support::Native,
            orphan_tasks: onetaskgraph_plugin_api::Support::Native,
            filter_by_label: onetaskgraph_plugin_api::Support::Native,
            filter_by_status: onetaskgraph_plugin_api::Support::Native,
            search_title: onetaskgraph_plugin_api::Support::Native,
            search_content: onetaskgraph_plugin_api::Support::Native,
            task_dependencies: onetaskgraph_plugin_api::DependencySupport::BothDirections,
            project_dependencies: onetaskgraph_plugin_api::DependencySupport::BothDirections,
            max_page_size: onetaskgraph_local_md::MAX_PAGE_SIZE,
        }
    );

    // `projects`: the folder holds two, and a listing scoped to one keeps the task filed
    // under it and no other.
    let mut projects = source
        .query_projects(&ProjectQuery::default(), &page(50))
        .await
        .unwrap()
        .items
        .into_iter()
        .map(|project| project.id.0)
        .collect::<Vec<_>>();
    projects.sort();
    assert_eq!(projects, ["alpha", "beta"]);
    assert_eq!(
        source
            .get_project(&NativeId("alpha".into()))
            .await
            .unwrap()
            .map(|project| project.title),
        Some("Alpha project".to_owned())
    );
    let under = |id: &str| TaskQuery {
        project: ProjectFilter::Is(NativeId(id.into())),
        ..TaskQuery::default()
    };
    assert_eq!(task_ids(source.as_ref(), &under("alpha")).await, ["first"]);
    assert_eq!(task_ids(source.as_ref(), &under("beta")).await, ["second"]);

    // `orphan_tasks`: the one document with no `project:` key, and neither of the two
    // that have one.
    assert_eq!(
        task_ids(
            source.as_ref(),
            &TaskQuery {
                project: ProjectFilter::Orphans,
                ..TaskQuery::default()
            }
        )
        .await,
        ["orphan"]
    );

    // `filter_by_label`: one of the three carries it, matched however the file spells it.
    let labelled = |filter: LabelFilter| TaskQuery {
        labels: filter,
        ..TaskQuery::default()
    };
    assert_eq!(
        task_ids(
            source.as_ref(),
            &labelled(LabelFilter {
                any_of: vec!["backend".into()],
                ..LabelFilter::default()
            })
        )
        .await,
        ["first"]
    );
    assert_eq!(
        task_ids(
            source.as_ref(),
            &labelled(LabelFilter {
                none_of: vec!["backend".into()],
                ..LabelFilter::default()
            })
        )
        .await,
        ["orphan", "second"]
    );

    // `filter_by_status`: two documents say `todo` and one says `done`.
    let filed_under = |category| TaskQuery {
        statuses: vec![category],
        ..TaskQuery::default()
    };
    assert_eq!(
        task_ids(source.as_ref(), &filed_under(StatusCategory::Todo)).await,
        ["first", "second"]
    );
    assert_eq!(
        task_ids(source.as_ref(), &filed_under(StatusCategory::Done)).await,
        ["orphan"]
    );

    // `search_title` and `search_content`: "Loose" is in one title and no body, "needle"
    // is in one body and no title, so each half finds its own and neither finds the
    // other's.
    let searching = |terms: &str, fields| TaskQuery {
        text: Some(TextQuery {
            terms: terms.into(),
            fields,
        }),
        ..TaskQuery::default()
    };
    assert_eq!(
        task_ids(source.as_ref(), &searching("loose", TextFields::Title)).await,
        ["orphan"]
    );
    assert!(
        task_ids(source.as_ref(), &searching("needle", TextFields::Title))
            .await
            .is_empty()
    );
    assert_eq!(
        task_ids(source.as_ref(), &searching("needle", TextFields::Content)).await,
        ["orphan"]
    );
    assert!(
        task_ids(source.as_ref(), &searching("loose", TextFields::Content))
            .await
            .is_empty()
    );
    assert_eq!(
        task_ids(
            source.as_ref(),
            &searching("needle", TextFields::TitleOrContent)
        )
        .await,
        ["orphan"]
    );

    // `task_dependencies` and `project_dependencies`, both directions each: one
    // relationship reads the same from either end, `from` being the document that waits.
    let task_edge = DependencyEdge {
        from: DependencyEndpoint::from_native(NativeId("second".into()), ItemKind::Task),
        to: DependencyEndpoint::from_native(NativeId("first".into()), ItemKind::Task),
        kind: DependencyKind::Blocks,
    };
    assert_eq!(
        source
            .task_dependencies(&NativeId("second".into()), Direction::DependsOn, &page(50))
            .await
            .unwrap()
            .items,
        std::slice::from_ref(&task_edge)
    );
    assert_eq!(
        source
            .task_dependencies(
                &NativeId("first".into()),
                Direction::DependedOnBy,
                &page(50)
            )
            .await
            .unwrap()
            .items,
        [task_edge]
    );
    let project_edge = DependencyEdge {
        from: DependencyEndpoint::from_native(NativeId("beta".into()), ItemKind::Project),
        to: DependencyEndpoint::from_native(NativeId("alpha".into()), ItemKind::Project),
        kind: DependencyKind::Blocks,
    };
    assert_eq!(
        source
            .project_dependencies(&NativeId("beta".into()), Direction::DependsOn, &page(50))
            .await
            .unwrap()
            .items,
        std::slice::from_ref(&project_edge)
    );
    assert_eq!(
        source
            .project_dependencies(
                &NativeId("alpha".into()),
                Direction::DependedOnBy,
                &page(50)
            )
            .await
            .unwrap()
            .items,
        [project_edge]
    );

    // `max_page_size`: a folder holding one document more than the ceiling serves the
    // ceiling and says the walk continues, rather than serving the folder.
    for index in 0..=onetaskgraph_local_md::MAX_PAGE_SIZE {
        fs::write(
            root.path().join(format!("tasks/bulk-{index}.md")),
            format!("---\ntitle: Bulk {index}\n---\nbulk\n"),
        )
        .unwrap();
    }
    let ceiling = source
        .query_tasks(&TaskQuery::default(), &page(u32::MAX))
        .await
        .unwrap();
    assert_eq!(
        ceiling.items.len(),
        onetaskgraph_local_md::MAX_PAGE_SIZE as usize
    );
    assert!(ceiling.next.is_some());
}
