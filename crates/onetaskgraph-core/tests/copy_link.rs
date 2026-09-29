//! The link a copy records on the item it copied, `onetaskgraph.copies`, and the copy that
//! follows it — as a **Rust caller** reaches both, through `Engine::copy`.
//!
//! A copy records where each item landed on the item itself, keyed by destination, so the
//! next copy of that item reads its counterpart by id instead of scanning the destination.
//! What these hold is the order the rules are tried in, what a link that no longer holds
//! does, and that a copy which cannot finish takes its links back with everything else.
//! The destination scans are counted rather than inferred: every page read of the
//! destination goes through [`Watched`], and a copy found by its link reads none.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use onetaskgraph_core::{
    ConfiguredSource, CopyAction, CopyItems, CopyLink, CopyOutcome, CopyReport, CopyRequest,
    CopyScope, CopyVia, Engine, EngineError, Failure, GlobalId, MatchBy, ResolvedSource,
};
use onetaskgraph_plugin_api::{
    Capabilities, DependencyEdge, Direction, Document, DocumentQuery, Health, ItemWrite, Label,
    MetadataKey, NativeId, Page, PageRequest, Project, ProjectQuery, SecretResolver, SourceError,
    SourceName, SourcePlugin, Task, TaskQuery, TaskSource, WriteSupport,
};
use secrecy::SecretString;
use serde_json::{Value, json};

/// No source in this crate's tests needs a credential.
struct NoSecrets;
impl SecretResolver for NoSecrets {
    fn get(&self, _var: &str) -> Option<SecretString> {
        None
    }
}

fn name(value: &str) -> SourceName {
    SourceName::new(value).expect("a valid source name")
}

fn id(value: &str) -> GlobalId {
    value.parse().expect("a qualified id")
}

/// An `in-memory` source that counts every page it serves and can be told to fail one
/// item's link write as a backend would.
///
/// Every page read goes through the three `query_*` methods, so `pages` is every scan of
/// this source — the rule-2 search, a `--match-by` search, the walk for a project to file
/// under and the walk for what a project copy left behind alike. A link write it is told
/// to fail answers `Unavailable`, which is neither of the two refusals a copy reads as the
/// source being unable to hold a link, so the copy fails and has to undo itself.
struct Watched {
    inner: Box<dyn TaskSource>,
    pages: Arc<AtomicU32>,
    unavailable_link: Option<NativeId>,
}

impl Watched {
    fn unavailable(&self, id: &NativeId) -> Result<(), SourceError> {
        if self.unavailable_link.as_ref() == Some(id) {
            return Err(SourceError::Unavailable {
                message: format!("the backend dropped the metadata write of {id}"),
            });
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl TaskSource for Watched {
    fn kind(&self) -> &'static str {
        self.inner.kind()
    }

    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }

    fn writes(&self) -> WriteSupport {
        self.inner.writes()
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

    async fn get_document(&self, id: &NativeId) -> Result<Option<Document>, SourceError> {
        self.inner.get_document(id).await
    }

    async fn query_tasks(
        &self,
        query: &TaskQuery,
        page: &PageRequest,
    ) -> Result<Page<Task>, SourceError> {
        self.pages.fetch_add(1, Ordering::Relaxed);
        self.inner.query_tasks(query, page).await
    }

    async fn query_projects(
        &self,
        query: &ProjectQuery,
        page: &PageRequest,
    ) -> Result<Page<Project>, SourceError> {
        self.pages.fetch_add(1, Ordering::Relaxed);
        self.inner.query_projects(query, page).await
    }

    async fn query_documents(
        &self,
        query: &DocumentQuery,
        page: &PageRequest,
    ) -> Result<Page<Document>, SourceError> {
        self.pages.fetch_add(1, Ordering::Relaxed);
        self.inner.query_documents(query, page).await
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

    async fn write_task(&self, write: &ItemWrite<Task>) -> Result<NativeId, SourceError> {
        self.inner.write_task(write).await
    }

    async fn write_project(&self, write: &ItemWrite<Project>) -> Result<NativeId, SourceError> {
        self.inner.write_project(write).await
    }

    async fn write_document(&self, write: &ItemWrite<Document>) -> Result<NativeId, SourceError> {
        self.inner.write_document(write).await
    }

    async fn set_task_metadata(
        &self,
        id: &NativeId,
        key: &MetadataKey,
        value: &Value,
    ) -> Result<Option<Task>, SourceError> {
        self.unavailable(id)?;
        self.inner.set_task_metadata(id, key, value).await
    }

    async fn set_project_metadata(
        &self,
        id: &NativeId,
        key: &MetadataKey,
        value: &Value,
    ) -> Result<Option<Project>, SourceError> {
        self.unavailable(id)?;
        self.inner.set_project_metadata(id, key, value).await
    }

    async fn set_document_metadata(
        &self,
        id: &NativeId,
        key: &MetadataKey,
        value: &Value,
    ) -> Result<Option<Document>, SourceError> {
        self.unavailable(id)?;
        self.inner.set_document_metadata(id, key, value).await
    }

    async fn delete_task(&self, id: &NativeId) -> Result<(), SourceError> {
        self.inner.delete_task(id).await
    }

    async fn delete_project(&self, id: &NativeId) -> Result<(), SourceError> {
        self.inner.delete_project(id).await
    }

    async fn delete_document(&self, id: &NativeId) -> Result<(), SourceError> {
        self.inner.delete_document(id).await
    }
}

/// One `in-memory` source named `source`, watched, and the count of the pages it serves.
fn watched(
    source: &str,
    config: &Value,
    unavailable_link: Option<&str>,
) -> (ConfiguredSource, Arc<AtomicU32>) {
    let pages = Arc::new(AtomicU32::new(0));
    let inner = onetaskgraph_in_memory::Plugin
        .build(&name(source), config, &NoSecrets)
        .expect("the in-memory plugin builds");
    let built = Watched {
        inner,
        pages: Arc::clone(&pages),
        unavailable_link: unavailable_link.map(|id| NativeId(id.to_owned())),
    };
    (
        ConfiguredSource::Ready(ResolvedSource::adopt(name(source), Box::new(built))),
        pages,
    )
}

/// An engine copying out of `from` into `into`, both `in-memory` and both watched, and the
/// count of the pages `into` serves.
fn engine(from: &Value, into: &Value) -> (Engine, Arc<AtomicU32>) {
    engine_failing(from, into, None)
}

/// The same, with `from` failing the link write of `unavailable_link`.
fn engine_failing(
    from: &Value,
    into: &Value,
    unavailable_link: Option<&str>,
) -> (Engine, Arc<AtomicU32>) {
    let (from, _) = watched("from", from, unavailable_link);
    let (into, pages) = watched("into", into, None);
    (
        Engine::new(vec![from, into], vec![name("from"), name("into")]),
        pages,
    )
}

/// One task, as an `in-memory` source holds it, recording `metadata`.
fn task(id: &str, title: &str, metadata: &Value) -> Value {
    json!({
        "id": id,
        "title": title,
        "content": "the engine core",
        "status": {"category": "todo", "name": "Todo"},
        "labels": [],
        "metadata": metadata,
    })
}

/// A copy of `items` into `into`, with every escape switched off.
fn copy_of(items: &[&str], scope: CopyScope) -> CopyRequest {
    CopyRequest {
        items: CopyItems::new(items.iter().map(|item| id(item)).collect())
            .expect("a copy names at least one item"),
        scope,
        destination: name("into"),
        match_by: None,
        recreate: false,
        dry_run: false,
    }
}

/// A copy of one task into `into`.
fn one(item: &str) -> CopyRequest {
    copy_of(&[item], CopyScope::Tasks)
}

/// What a report says about one item: where it landed, what the copy did, which rule found
/// it and what became of its link.
fn said(outcome: &CopyOutcome) -> (Option<String>, String, Option<CopyVia>, Option<CopyLink>) {
    (
        outcome.destination().map(ToString::to_string),
        outcome.action.name(),
        outcome.via,
        outcome.link,
    )
}

fn landed(
    destination: &str,
    action: &str,
    via: CopyVia,
    link: CopyLink,
) -> (Option<String>, String, Option<CopyVia>, Option<CopyLink>) {
    (
        Some(destination.to_owned()),
        action.to_owned(),
        Some(via),
        Some(link),
    )
}

/// The metadata one task holds, read back through the engine's own show verb.
async fn task_metadata(engine: &Engine, item: &str) -> serde_json::Map<String, Value> {
    engine
        .task(&id(item))
        .await
        .expect("the show verb answers")
        .items[0]
        .item
        .metadata
        .clone()
        .into_iter()
        .collect()
}

/// What one task records at `onetaskgraph.copies`, or `Value::Null` when it records none.
async fn links(engine: &Engine, item: &str) -> Value {
    task_metadata(engine, item)
        .await
        .get(MetadataKey::COPIES_KEY)
        .cloned()
        .unwrap_or(Value::Null)
}

/// Every task one in-memory source holds, by id.
async fn held(engine: &Engine, source: &str) -> Vec<String> {
    let response = engine
        .tasks(&onetaskgraph_core::TaskRequest {
            sources: vec![name(source)],
            filters: onetaskgraph_core::Filters::default(),
            project: onetaskgraph_core::ProjectSelector::Any,
            priorities: Vec::new(),
            commented_since: None,
            paging: onetaskgraph_core::Paging {
                limit: std::num::NonZeroU32::new(50).expect("a non-zero limit"),
                token: None,
            },
        })
        .await
        .expect("the list verb answers");
    response
        .items
        .into_iter()
        .map(|task| task.id.to_string())
        .collect()
}

async fn copied(engine: &Engine, request: &CopyRequest) -> CopyReport {
    engine.copy(request).await.expect("the copy runs")
}

#[tokio::test]
async fn a_copied_task_records_its_link_and_the_next_copy_follows_it_without_a_scan() {
    let (engine, pages) = engine(
        &json!({"tasks": [task("T-1", "Alpha", &json!({}))]}),
        &json!({}),
    );

    let first = copied(&engine, &one("from:T-1")).await;
    assert_eq!(
        said(&first.items[0]),
        landed("into:T-1", "created", CopyVia::Created, CopyLink::Recorded)
    );
    assert_eq!(
        links(&engine, "from:T-1").await,
        json!({"into": "into:T-1"})
    );
    // The link is the copied item's own and never travels: the destination item records
    // its origin, and nothing at the other key.
    let arrived = task_metadata(&engine, "into:T-1").await;
    assert_eq!(arrived[GlobalId::ORIGIN_KEY], json!("from:T-1"));
    assert!(
        !arrived.contains_key(MetadataKey::COPIES_KEY),
        "the link is not carried onto the destination: {arrived:?}"
    );

    // The next copy reads its counterpart by id and scans nothing — unchanged, and then
    // after an edit at the source, updated.
    pages.store(0, Ordering::Relaxed);
    let again = copied(&engine, &one("from:T-1")).await;
    assert_eq!(
        said(&again.items[0]),
        landed("into:T-1", "unchanged", CopyVia::Link, CopyLink::Unchanged)
    );
    engine
        .set_task_metadata(
            &id("from:T-1"),
            &MetadataKey::new("caller.edited").expect("a caller key"),
            &json!(true),
        )
        .await
        .expect("the source takes an edit");
    let edited = copied(&engine, &one("from:T-1")).await;
    assert_eq!(
        said(&edited.items[0]),
        landed("into:T-1", "updated", CopyVia::Link, CopyLink::Unchanged)
    );
    assert_eq!(
        pages.load(Ordering::Relaxed),
        0,
        "a copy found by its link reads no page of the destination"
    );
    assert_eq!(
        task_metadata(&engine, "into:T-1").await["caller.edited"],
        json!(true)
    );
    assert_eq!(held(&engine, "into").await, vec!["into:T-1".to_owned()]);
}

#[tokio::test]
async fn a_project_and_a_document_record_their_links_and_are_found_by_them() {
    let from = json!({
        "capabilities": {"documents": "native"},
        "projects": [{"id": "P-1", "title": "Engine",
                      "status": {"category": "todo", "name": "Todo"}, "labels": []}],
        "documents": [{"id": "D-1", "title": "Design", "content": "why",
                       "project": null, "labels": []}],
    });
    let (engine, pages) = engine(&from, &json!({"capabilities": {"documents": "native"}}));
    let project = copy_of(&["from:P-1"], CopyScope::Projects { tasks: false });
    let document = copy_of(&["from:D-1"], CopyScope::Documents);

    for (request, landed_on) in [(&project, "into:P-1"), (&document, "into:D-1")] {
        let first = copied(&engine, request).await;
        assert_eq!(
            said(&first.items[0]),
            landed(landed_on, "created", CopyVia::Created, CopyLink::Recorded)
        );
    }
    let project_links = engine
        .project(&id("from:P-1"))
        .await
        .expect("the show verb answers")
        .items[0]
        .item
        .metadata[MetadataKey::COPIES_KEY]
        .clone();
    assert_eq!(project_links, json!({"into": "into:P-1"}));
    let document_links = engine
        .document(&id("from:D-1"))
        .await
        .expect("the show verb answers")
        .items[0]
        .item
        .metadata[MetadataKey::COPIES_KEY]
        .clone();
    assert_eq!(document_links, json!({"into": "into:D-1"}));

    pages.store(0, Ordering::Relaxed);
    for (request, landed_on) in [(&project, "into:P-1"), (&document, "into:D-1")] {
        let again = copied(&engine, request).await;
        assert_eq!(
            said(&again.items[0]),
            landed(landed_on, "unchanged", CopyVia::Link, CopyLink::Unchanged)
        );
    }
    assert_eq!(
        pages.load(Ordering::Relaxed),
        0,
        "a project and a document found by their links read no page of the destination"
    );
}

#[tokio::test]
async fn a_link_naming_nothing_there_refuses_until_recreate_creates_and_records_it_again() {
    let from = json!({"tasks": [task("T-1", "Alpha",
        &json!({MetadataKey::COPIES_KEY: {"into": "into:GONE", "elsewhere": "elsewhere:E-1"}}))]});
    let (engine, _) = engine(&from, &json!({}));

    let refused = engine
        .copy(&one("from:T-1"))
        .await
        .expect_err("a link naming nothing is refused");
    assert!(
        matches!(&refused, EngineError::StaleLink { item, link }
            if item == "from:T-1" && link == "into:GONE"),
        "{refused:?}"
    );
    let message = refused.to_string();
    assert!(
        message.contains("from:T-1") && message.contains("into:GONE"),
        "the refusal names both ids: {message}"
    );
    assert!(message.contains("--recreate"), "and the escape: {message}");
    assert_eq!(Failure::from(&refused).kind(), "stale-link");
    assert!(
        held(&engine, "into").await.is_empty(),
        "nothing was written"
    );

    let recreated = copied(
        &engine,
        &CopyRequest {
            recreate: true,
            ..one("from:T-1")
        },
    )
    .await;
    assert_eq!(
        said(&recreated.items[0]),
        landed("into:T-1", "created", CopyVia::Created, CopyLink::Recorded)
    );
    // The one entry is rewritten and every other destination's is left as it was.
    assert_eq!(
        links(&engine, "from:T-1").await,
        json!({"into": "into:T-1", "elsewhere": "elsewhere:E-1"})
    );
}

#[tokio::test]
async fn a_link_to_an_item_somebody_repointed_is_ignored_and_rewritten_to_what_the_rules_find() {
    let from = json!({"tasks": [task("T-1", "Alpha",
        &json!({MetadataKey::COPIES_KEY: {"into": "into:X"}}))]});
    let into = json!({"tasks": [
        // The item the link names, re-pointed by a person at something else.
        task("X", "Somebody else's", &json!({GlobalId::ORIGIN_KEY: "elsewhere:Z"})),
        // The item that really records this one as its origin.
        task("Y", "Alpha", &json!({GlobalId::ORIGIN_KEY: "from:T-1"})),
    ]});
    let (engine, pages) = engine(&from, &into);

    let report = copied(&engine, &one("from:T-1")).await;
    assert_eq!(
        said(&report.items[0]),
        landed("into:Y", "unchanged", CopyVia::Scan, CopyLink::Recorded)
    );
    assert_eq!(links(&engine, "from:T-1").await, json!({"into": "into:Y"}));
    assert_eq!(
        task_metadata(&engine, "into:X").await[GlobalId::ORIGIN_KEY],
        json!("elsewhere:Z"),
        "the re-pointed item is not touched"
    );

    // And from then on it is the link that finds it.
    pages.store(0, Ordering::Relaxed);
    let again = copied(&engine, &one("from:T-1")).await;
    assert_eq!(
        said(&again.items[0]),
        landed("into:Y", "unchanged", CopyVia::Link, CopyLink::Unchanged)
    );
    assert_eq!(pages.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn a_repointed_link_with_nothing_else_to_find_creates_and_records_the_new_item() {
    let from = json!({"tasks": [task("T-1", "Alpha",
        &json!({MetadataKey::COPIES_KEY: {"into": "into:X"}}))]});
    let into = json!({"tasks": [
        task("X", "Somebody else's", &json!({GlobalId::ORIGIN_KEY: "elsewhere:Z"})),
    ]});
    let (engine, _) = engine(&from, &into);

    let report = copied(&engine, &one("from:T-1")).await;
    assert_eq!(
        said(&report.items[0]),
        landed("into:T-1", "created", CopyVia::Created, CopyLink::Recorded)
    );
    assert_eq!(
        links(&engine, "from:T-1").await,
        json!({"into": "into:T-1"})
    );
}

#[tokio::test]
async fn an_item_whose_own_origin_names_the_destination_is_found_by_it_and_records_nothing() {
    // The two hand-written shapes a caller used before there was a link — a follow-up
    // ticket and a write-back's shadow task each naming their board item at their own
    // origin — keep being found by rule 1 and gain no write.
    let from = json!({"tasks": [task("T-1", "Alpha, settled",
        &json!({GlobalId::ORIGIN_KEY: "into:A"}))]});
    let into = json!({"tasks": [task("A", "Alpha", &json!({}))]});
    let (engine, pages) = engine(&from, &into);

    let report = copied(&engine, &one("from:T-1")).await;
    assert_eq!(
        said(&report.items[0]),
        landed("into:A", "updated", CopyVia::Origin, CopyLink::Unchanged)
    );
    assert_eq!(links(&engine, "from:T-1").await, Value::Null);
    assert_eq!(pages.load(Ordering::Relaxed), 0);
    assert!(
        !task_metadata(&engine, "into:A")
            .await
            .contains_key(GlobalId::ORIGIN_KEY),
        "a copy-back leaves the original's own provenance as it was"
    );
}

#[tokio::test]
async fn a_match_by_escape_is_reported_as_a_match_and_records_the_link() {
    let from = json!({"tasks": [task("T-1", "Alpha", &json!({}))]});
    let into = json!({"tasks": [task("M", "Alpha", &json!({}))]});
    let (engine, _) = engine(&from, &into);

    let report = copied(
        &engine,
        &CopyRequest {
            match_by: Some(MatchBy::Title),
            ..one("from:T-1")
        },
    )
    .await;
    assert_eq!(
        said(&report.items[0]),
        landed("into:M", "updated", CopyVia::Match, CopyLink::Recorded)
    );
    assert_eq!(links(&engine, "from:T-1").await, json!({"into": "into:M"}));
}

#[tokio::test]
async fn a_dry_run_says_which_rule_would_answer_and_records_no_link() {
    let (engine, _) = engine(
        &json!({"tasks": [task("T-1", "Alpha", &json!({}))]}),
        &json!({}),
    );

    let report = copied(
        &engine,
        &CopyRequest {
            dry_run: true,
            ..one("from:T-1")
        },
    )
    .await;
    assert_eq!(
        (report.items[0].via, report.items[0].link),
        (Some(CopyVia::Created), None)
    );
    let wire = serde_json::to_value(&report).expect("a report serialises");
    assert_eq!(wire["items"][0]["via"], json!("created"));
    assert!(
        wire["items"][0].get("link").is_none(),
        "a dry run leaves `link` out: {wire}"
    );
    assert_eq!(links(&engine, "from:T-1").await, Value::Null);
}

#[tokio::test]
async fn a_source_that_cannot_hold_a_link_is_copied_from_as_before_and_says_so() {
    for (why, capabilities) in [
        ("has no write side", json!({"writes": "unsupported"})),
        (
            "refuses the key",
            json!({"unwritable_metadata_keys": [MetadataKey::COPIES_KEY]}),
        ),
    ] {
        let from = json!({"capabilities": capabilities,
                          "tasks": [task("T-1", "Alpha", &json!({}))]});
        let (engine, pages) = engine(&from, &json!({}));

        let first = copied(&engine, &one("from:T-1")).await;
        assert_eq!(
            said(&first.items[0]),
            landed(
                "into:T-1",
                "created",
                CopyVia::Created,
                CopyLink::Unrecorded
            ),
            "a source that {why}"
        );
        assert_eq!(links(&engine, "from:T-1").await, Value::Null, "{why}");
        // With no link, the next copy is found exactly as it was before there were any.
        pages.store(0, Ordering::Relaxed);
        let again = copied(&engine, &one("from:T-1")).await;
        assert_eq!(
            said(&again.items[0]),
            landed("into:T-1", "unchanged", CopyVia::Scan, CopyLink::Unrecorded),
            "a source that {why}"
        );
        assert!(pages.load(Ordering::Relaxed) > 0, "{why}");
    }
}

#[tokio::test]
async fn a_copy_that_fails_after_creating_leaves_the_source_without_the_link() {
    let from = json!({"tasks": [
        task("T-1", "Alpha", &json!({})),
        task("T-2", "Beta", &json!({})),
    ]});
    let (engine, _) = engine(
        &from,
        &json!({"capabilities": {"uncreatable_titles": ["Beta"]}}),
    );

    engine
        .copy(&copy_of(&["from:T-1", "from:T-2"], CopyScope::Tasks))
        .await
        .expect_err("the second create is refused");
    assert!(
        held(&engine, "into").await.is_empty(),
        "the destination holds none of the copy's items"
    );
    for item in ["from:T-1", "from:T-2"] {
        assert_eq!(links(&engine, item).await, Value::Null, "{item}");
    }
}

#[tokio::test]
async fn a_link_write_that_fails_undoes_every_link_and_every_item_the_copy_wrote() {
    // Three items, and the third one's link write fails after the first two have landed:
    // one of them held a link to another destination before, which is written back as it
    // was, and one held none, which is left holding none.
    let from = json!({"tasks": [
        task("T-1", "Alpha", &json!({MetadataKey::COPIES_KEY: {"elsewhere": "elsewhere:E-1"}})),
        task("T-3", "Gamma", &json!({"caller.kept": [1, "two"]})),
        task("T-2", "Beta", &json!({})),
    ]});
    let (engine, _) = engine_failing(&from, &json!({}), Some("T-2"));

    let refused = engine
        .copy(&copy_of(
            &["from:T-1", "from:T-3", "from:T-2"],
            CopyScope::Tasks,
        ))
        .await
        .expect_err("the third link write fails");
    assert!(
        refused
            .to_string()
            .contains("dropped the metadata write of T-2"),
        "the copy names what failed: {refused}"
    );
    assert!(held(&engine, "into").await.is_empty(), "{refused}");
    assert_eq!(
        links(&engine, "from:T-1").await,
        json!({"elsewhere": "elsewhere:E-1"})
    );
    let untouched = task_metadata(&engine, "from:T-3").await;
    assert!(
        !untouched.contains_key(MetadataKey::COPIES_KEY),
        "{untouched:?}"
    );
    assert_eq!(untouched["caller.kept"], json!([1, "two"]));
    assert_eq!(links(&engine, "from:T-2").await, Value::Null);
}

#[tokio::test]
async fn a_repeated_copy_of_a_linked_item_keeps_the_destinations_own_link() {
    // The copy-back of an item whose destination counterpart has itself been copied on
    // somewhere: that counterpart's own link is its own, and the copy keeps it.
    let (engine, _) = engine(
        &json!({"tasks": [task("T-1", "Alpha", &json!({}))]}),
        &json!({}),
    );
    copied(&engine, &one("from:T-1")).await;

    let back = copied(
        &engine,
        &CopyRequest {
            destination: name("from"),
            ..one("into:T-1")
        },
    )
    .await;
    assert_eq!(
        said(&back.items[0]),
        landed(
            "from:T-1",
            "unchanged",
            CopyVia::Origin,
            CopyLink::Unchanged
        ),
        "the copy back finds the original by the copy's own origin, and the original's \
         link is not a difference"
    );
    assert_eq!(
        links(&engine, "from:T-1").await,
        json!({"into": "into:T-1"})
    );
    assert!(matches!(back.items[0].action, CopyAction::Unchanged { .. }));
}
