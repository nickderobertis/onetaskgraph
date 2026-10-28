//! `Engine::locate` as a Rust caller reaches it: the request type this crate exports, the
//! method on the engine, and the report it returns. The journeys that drive `task locate` as a
//! user does, against the copy after it, are in `crates/onetaskgraph-e2e/tests/e2e/locate.rs`.

use onetaskgraph_core::{
    Config, ConfiguredSource, CopyItems, CopyRequest, CopyScope, CopyVia, Engine, EngineError,
    GlobalId, LocateLookup, LocateRequest, LocatedCounterpart, NotATask, ResolvedSource,
    StatusPolicy,
};
use onetaskgraph_plugin_api::{
    Capabilities, DependencyEdge, Direction, Health, Label, NativeId, Page, PageRequest, Project,
    ProjectQuery, SecretResolver, SourceError, SourceName, SourcePlugin, Task, TaskQuery,
    TaskSource, WriteSupport,
};
use secrecy::SecretString;
use serde_json::{Value, json};

/// No source here needs a credential.
struct NoSecrets;
impl SecretResolver for NoSecrets {
    fn get(&self, _var: &str) -> Option<SecretString> {
        None
    }
}

fn id(value: &str) -> GlobalId {
    value.parse().expect("a qualified id")
}

fn task(native: &str, metadata: Value) -> Value {
    json!({"id": native, "title": native, "status": {"category": "todo", "name": "Todo"}, "labels": [],
           "metadata": metadata})
}

/// `from` holds `T-1`, `T-2` and the project `P-1`; `into` holds `D-1`, copied from `from:T-2`.
fn engine() -> Engine {
    let config = Config::from_document(json!({"sources": {
        "from": {"plugin": "in-memory", "config": {
            "tasks": [task("T-1", json!({})), task("T-2", json!({}))],
            "projects": [{"id": "P-1", "title": "Plan", "labels": [],
                          "status": {"category": "todo", "name": "Todo"}}],
        }},
        "into": {"plugin": "in-memory", "config": {
            "tasks": [task("D-1", json!({"onetaskgraph.origin": "from:T-2"}))],
        }},
    }}))
    .expect("a valid configuration");
    Engine::build(&config, &NoSecrets)
}

fn locating(items: &[&str], lookup: LocateLookup) -> LocateRequest {
    LocateRequest {
        items: items.iter().map(|item| id(item)).collect(),
        destination: SourceName::new("into").unwrap(),
        lookup,
    }
}

#[tokio::test]
async fn several_tasks_are_answered_in_the_order_named_and_nothing_meters_spends_nothing() {
    let engine = engine();
    let report = engine
        .locate(&locating(
            &["from:T-2", "from:T-1"],
            LocateLookup::Counterpart,
        ))
        .await
        .expect("both are located");
    let sources: Vec<String> = report
        .items
        .iter()
        .map(|item| item.source.to_string())
        .collect();
    assert_eq!(sources, ["from:T-2", "from:T-1"]);
    assert_eq!(
        report.items[0].counterpart,
        Some(Some(LocatedCounterpart {
            id: id("into:D-1"),
            via: CopyVia::Scan,
        }))
    );
    assert_eq!(report.items[0].carriers, Some(vec![id("into:D-1")]));
    assert_eq!(report.items[1].counterpart, Some(None));
    assert_eq!(report.spent, None, "no source here meters its requests");

    // The same answer a copy of them then acts on: `T-2` updates `D-1`, `T-1` is created.
    let copied = engine
        .copy(&CopyRequest {
            items: CopyItems::new(vec![id("from:T-2"), id("from:T-1")]).unwrap(),
            scope: CopyScope::Tasks,
            status_policy: StatusPolicy::Source,
            destination: SourceName::new("into").unwrap(),
            match_by: None,
            target_task: None,
            recreate: false,
            create: false,
            dry_run: true,
            comments: false,
            no_link: true,
            expect: None,
            statuses: Vec::new(),
        })
        .await
        .expect("the copy");
    assert_eq!(
        copied.items[0].destination().map(ToString::to_string),
        Some("into:D-1".to_owned())
    );
    assert_eq!(
        copied.items[1].destination(),
        None,
        "a dry run that would create"
    );
}

#[tokio::test]
async fn naming_no_task_answers_none_and_a_project_a_missing_id_and_two_targets_are_refused() {
    let engine = engine();
    let report = engine
        .locate(&locating(&[], LocateLookup::Counterpart))
        .await
        .expect("no task, no answer");
    assert!(report.items.is_empty() && report.spent.is_none());

    let refused = engine
        .locate(&locating(&["from:P-1"], LocateLookup::NoCounterpart))
        .await
        .expect_err("a project is no task");
    assert_eq!(
        refused,
        EngineError::NotATask {
            id: id("from:P-1"),
            kind: NotATask::Project,
        }
    );
    let missing = engine
        .locate(&locating(&["from:T-404"], LocateLookup::NoCounterpart))
        .await
        .expect_err("nothing there");
    assert!(
        matches!(missing, EngineError::NoSuchItem { .. }),
        "{missing:?}"
    );
    let several = engine
        .locate(&locating(
            &["from:T-1", "from:T-2"],
            LocateLookup::TargetNative("D-1".into()),
        ))
        .await
        .expect_err("--target-native names one task's counterpart");
    assert_eq!(several, EngineError::TargetNativeNeedsOne { count: 2 });
}

/// An `in-memory` source whose every project read fails as unreachable.
struct ProjectsUnreachable(Box<dyn TaskSource>);

#[async_trait::async_trait]
impl TaskSource for ProjectsUnreachable {
    fn kind(&self) -> &'static str {
        self.0.kind()
    }
    fn capabilities(&self) -> Capabilities {
        self.0.capabilities()
    }
    fn writes(&self) -> WriteSupport {
        self.0.writes()
    }
    async fn health(&self) -> Result<Health, SourceError> {
        self.0.health().await
    }
    async fn get_task(&self, id: &NativeId) -> Result<Option<Task>, SourceError> {
        self.0.get_task(id).await
    }
    async fn get_project(&self, _id: &NativeId) -> Result<Option<Project>, SourceError> {
        Err(SourceError::Unavailable {
            message: "the projects could not be reached".to_owned(),
        })
    }
    async fn query_tasks(
        &self,
        query: &TaskQuery,
        page: &PageRequest,
    ) -> Result<Page<Task>, SourceError> {
        self.0.query_tasks(query, page).await
    }
    async fn query_projects(
        &self,
        query: &ProjectQuery,
        page: &PageRequest,
    ) -> Result<Page<Project>, SourceError> {
        self.0.query_projects(query, page).await
    }
    async fn labels(&self, page: &PageRequest) -> Result<Page<Label>, SourceError> {
        self.0.labels(page).await
    }
    async fn task_dependencies(
        &self,
        id: &NativeId,
        direction: Direction,
        page: &PageRequest,
    ) -> Result<Page<DependencyEdge>, SourceError> {
        self.0.task_dependencies(id, direction, page).await
    }
    async fn project_dependencies(
        &self,
        id: &NativeId,
        direction: Direction,
        page: &PageRequest,
    ) -> Result<Page<DependencyEdge>, SourceError> {
        self.0.project_dependencies(id, direction, page).await
    }
}

#[tokio::test]
async fn an_id_naming_no_task_whose_project_read_fails_is_refused_by_that_failure() {
    let built = |name: &str| {
        onetaskgraph_in_memory::Plugin
            .build(
                &SourceName::new(name).unwrap(),
                &json!({"tasks": [task("T-1", json!({}))]}),
                &NoSecrets,
            )
            .expect("the in-memory plugin builds")
    };
    let unreachable = ConfiguredSource::Ready(ResolvedSource::adopt(
        SourceName::new("from").unwrap(),
        Box::new(ProjectsUnreachable(built("from"))),
    ));
    let into = ConfiguredSource::Ready(ResolvedSource::adopt(
        SourceName::new("into").unwrap(),
        built("into"),
    ));
    let engine = Engine::new(
        vec![unreachable, into],
        vec![
            SourceName::new("from").unwrap(),
            SourceName::new("into").unwrap(),
        ],
    );
    let refused = engine
        .locate(&locating(&["from:P-1"], LocateLookup::NoCounterpart))
        .await
        .expect_err("whether it names a project cannot be read");
    let message = refused.to_string();
    assert!(
        message.contains("the projects could not be reached"),
        "{message}"
    );
    assert!(
        !matches!(refused, EngineError::NoSuchItem { .. }),
        "never answered as naming nothing: {refused:?}"
    );
    // A task it holds is still located.
    let located = engine
        .locate(&locating(&["from:T-1"], LocateLookup::NoCounterpart))
        .await
        .expect("a task is read without the probe");
    assert_eq!(located.items.len(), 1);
}
