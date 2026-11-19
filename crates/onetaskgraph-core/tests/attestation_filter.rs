//! `Engine::check_project_attestations_filtered`, as a Rust caller links it: a task its status
//! filter leaves out is never read for its record, so a narrow check costs only the tasks it
//! answers; a task its check-state filter leaves out is read, checked and dropped.
//!
//! The journeys that drive `project attestation check --tasks` through the binary are in
//! `crates/onetaskgraph-e2e/tests/e2e/attestations.rs`; what they cannot see is which reads
//! reached the source, which this counts.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use onetaskgraph_core::attestation::{AttestationKind, AttestationState, TaskCheckFilter};
use onetaskgraph_core::{ConfiguredSource, Engine, GlobalId, ResolvedSource};
use onetaskgraph_plugin_api::{
    Capabilities, DependencyEdge, Direction, Health, Label, NativeId, Page, PageRequest, Project,
    ProjectQuery, SecretResolver, SourceError, SourceName, SourcePlugin, StatusCategory, Task,
    TaskQuery, TaskSource,
};
use secrecy::SecretString;
use serde_json::json;

struct NoSecrets;

impl SecretResolver for NoSecrets {
    fn get(&self, _var: &str) -> Option<SecretString> {
        None
    }
}

/// An `in-memory` source counting the dependency reads each task receives.
struct Counting {
    inner: Box<dyn TaskSource>,
    reads: Arc<Mutex<BTreeMap<String, u32>>>,
}

#[async_trait::async_trait]
impl TaskSource for Counting {
    fn kind(&self) -> &'static str {
        self.inner.kind()
    }

    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }

    async fn health(&self) -> Result<Health, SourceError> {
        self.inner.health().await
    }

    async fn get_task(&self, id: &NativeId) -> Result<Option<Task>, SourceError> {
        self.inner.get_task(id).await
    }

    async fn get_project(&self, id: &NativeId) -> Result<Option<Project>, SourceError> {
        self.inner.get_project(id).await
    }

    async fn query_tasks(
        &self,
        query: &TaskQuery,
        page: &PageRequest,
    ) -> Result<Page<Task>, SourceError> {
        self.inner.query_tasks(query, page).await
    }

    async fn query_projects(
        &self,
        query: &ProjectQuery,
        page: &PageRequest,
    ) -> Result<Page<Project>, SourceError> {
        self.inner.query_projects(query, page).await
    }

    async fn labels(&self, page: &PageRequest) -> Result<Page<Label>, SourceError> {
        self.inner.labels(page).await
    }

    async fn task_dependencies(
        &self,
        id: &NativeId,
        direction: Direction,
        page: &PageRequest,
    ) -> Result<Page<DependencyEdge>, SourceError> {
        *self
            .reads
            .lock()
            .expect("an unpoisoned counter")
            .entry(id.0.clone())
            .or_default() += 1;
        self.inner.task_dependencies(id, direction, page).await
    }

    async fn project_dependencies(
        &self,
        id: &NativeId,
        direction: Direction,
        page: &PageRequest,
    ) -> Result<Page<DependencyEdge>, SourceError> {
        self.inner.project_dependencies(id, direction, page).await
    }
}

#[tokio::test]
async fn a_status_filter_reads_only_the_tasks_it_keeps_and_a_state_filter_drops_after_checking() {
    let at = |category: &str| json!({"category": category, "name": category});
    let config = json!({
        "projects": [{"id": "P-1", "title": "Plan", "status": at("todo"), "labels": []}],
        "tasks": [
            {"id": "T-1", "title": "Open", "status": at("todo"), "labels": [], "project": "P-1"},
            {"id": "T-2", "title": "Shipped", "status": at("done"), "labels": [],
             "project": "P-1"},
            {"id": "T-3", "title": "Dropped", "status": at("cancelled"), "labels": [],
             "project": "P-1"},
        ],
    });
    let name = SourceName::new("work").expect("a valid source name");
    let reads = Arc::new(Mutex::new(BTreeMap::new()));
    let engine = Engine::new(
        vec![ConfiguredSource::Ready(ResolvedSource::adopt(
            name.clone(),
            Box::new(Counting {
                inner: onetaskgraph_in_memory::Plugin
                    .build(&name, &config, &NoSecrets)
                    .expect("the in-memory plugin builds"),
                reads: Arc::clone(&reads),
            }),
        ))],
        vec![name],
    );
    let project: GlobalId = "work:P-1".parse().expect("a qualified id");
    let kind = AttestationKind::new("review").expect("a kind");

    let open = TaskCheckFilter {
        not_statuses: vec![StatusCategory::Done, StatusCategory::Cancelled],
        ..TaskCheckFilter::default()
    };
    let checked = engine
        .check_project_attestations_filtered(&project, &kind, None, &open)
        .await
        .expect("the check answers");
    let ids: Vec<String> = checked
        .tasks
        .iter()
        .map(|task| task.id.to_string())
        .collect();
    assert_eq!(ids, ["work:T-1"]);
    assert_eq!(
        reads.lock().expect("unpoisoned").clone(),
        BTreeMap::from([("T-1".to_owned(), 1)]),
        "a task its status leaves out is read"
    );

    // A state filter keeping only `valid` answers no task here, every task being unattested,
    // yet each was read to learn that.
    reads.lock().expect("unpoisoned").clear();
    let valid = TaskCheckFilter {
        states: vec![AttestationState::Valid],
        ..TaskCheckFilter::default()
    };
    let checked = engine
        .check_project_attestations_filtered(&project, &kind, None, &valid)
        .await
        .expect("the check answers");
    assert!(checked.tasks.is_empty(), "{:?}", checked.tasks);
    assert_eq!(
        reads
            .lock()
            .expect("unpoisoned")
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        ["T-1", "T-2", "T-3"]
    );
}
