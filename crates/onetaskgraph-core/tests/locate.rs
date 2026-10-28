//! `Engine::locate` as a Rust caller reaches it: the request type this crate exports, the
//! method on the engine, and the report it returns. The journeys that drive `task locate` as a
//! user does, against the copy after it, are in `crates/onetaskgraph-e2e/tests/e2e/locate.rs`.

use onetaskgraph_core::{
    Config, CopyItems, CopyRequest, CopyScope, CopyVia, Engine, EngineError, GlobalId,
    LocateLookup, LocateRequest, LocatedCounterpart, NotATask, StatusPolicy,
};
use onetaskgraph_plugin_api::{SecretResolver, SourceName};
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
async fn naming_no_task_answers_none_and_a_project_or_a_document_is_refused_by_kind() {
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
