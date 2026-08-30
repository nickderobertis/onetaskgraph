//! The copy verb as a **Rust caller** reaches it: build the request type this crate
//! exports, call the method on the engine, read the outcomes it returns.
//!
//! Nothing here goes through the command line, and that is the point. This product is
//! exposed three ways from one engine, so a copy a script makes, a copy an application
//! makes and a copy typed at a shell have to be the same call — and the consumer a
//! command-line-only copy would strand is this one, the Rust caller that links the crate.
//! The journeys that drive the same verb as a user does are in
//! `crates/onetaskgraph/tests/e2e/`.

use std::num::NonZeroU32;

use onetaskgraph_core::{
    Config, CopyAction, CopyItems, CopyOutcome, CopyRequest, CopyScope, DependencyRequest, Engine,
    EngineError, GlobalId, MatchBy, Paging, TaskRequest,
};
use onetaskgraph_plugin_api::{Direction, SecretResolver, SourceName};
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

/// An engine over a configuration document's `sources:` block.
fn engine_over(sources: Value) -> Engine {
    let config =
        Config::from_document(json!({ "sources": sources })).expect("a valid configuration");
    Engine::build(&config, &NoSecrets)
}

/// One task, held by an `in-memory` source.
fn task(id: &str, title: &str) -> Value {
    json!({
        "id": id,
        "title": title,
        "content": "the engine core",
        "status": {"category": "todo", "name": "Todo"},
        "labels": [{"id": "L-1", "name": "bug"}],
        "metadata": {"caller.shape": {"nested": [1, true, null]}},
        "repositories": ["github.com/nickderobertis/onetaskgraph"]
    })
}

/// Two in-memory sources: one holding `T-1`, one empty and writable.
fn pair() -> Engine {
    engine_over(json!({
        "from": {"plugin": "in-memory", "config": {"tasks": [task("T-1", "Alpha engine")]}},
        "into": {"plugin": "in-memory", "config": {}},
    }))
}

/// A copy of one task into `into`, with every escape switched off.
fn one(item: &str) -> CopyRequest {
    many(&[item], CopyScope::Tasks)
}

/// A copy of several items into `into`, with every escape switched off.
fn many(items: &[&str], scope: CopyScope) -> CopyRequest {
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

/// The destination id and the word an outcome reports, as a comparable pair.
fn landed(outcome: &CopyOutcome) -> (Option<String>, String) {
    (
        outcome.destination().map(ToString::to_string),
        outcome.action.name(),
    )
}

/// Every task one source holds, by qualified id, through the engine's own list verb.
async fn listed(engine: &Engine, source: &str) -> Vec<String> {
    let response = engine
        .tasks(&TaskRequest {
            sources: vec![name(source)],
            filters: onetaskgraph_core::Filters::default(),
            project: onetaskgraph_core::ProjectSelector::Any,
            paging: Paging {
                limit: NonZeroU32::new(50).expect("a non-zero limit"),
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

#[tokio::test]
async fn a_rust_caller_creates_then_updates_the_same_destination_item() {
    let engine = pair();

    let created = engine.copy(&one("from:T-1")).await.expect("the copy runs");
    assert_eq!(created.items.len(), 1);
    assert_eq!(created.items[0].source, id("from:T-1"));
    assert_eq!(
        landed(&created.items[0]),
        (Some("into:T-1".to_owned()), "created".to_owned())
    );

    // The destination really holds it, with the value and the JSON type of every
    // caller-defined key intact — read back through the engine, not through the write.
    let copied = engine
        .task(&id("into:T-1"))
        .await
        .expect("the show verb answers");
    let copied = &copied.items[0].item;
    assert_eq!(copied.title, "Alpha engine");
    assert_eq!(
        copied.metadata["caller.shape"],
        json!({"nested": [1, true, null]})
    );
    assert_eq!(
        copied.metadata[GlobalId::ORIGIN_KEY],
        Value::String("from:T-1".to_owned())
    );
    assert_eq!(
        copied.repositories[0].as_str(),
        "github.com/nickderobertis/onetaskgraph"
    );

    // A second copy of the same item updates that one and creates nothing.
    let again = engine.copy(&one("from:T-1")).await.expect("the copy runs");
    assert_eq!(
        landed(&again.items[0]),
        (Some("into:T-1".to_owned()), "unchanged".to_owned())
    );
    assert_eq!(listed(&engine, "into").await, vec!["into:T-1".to_owned()]);

    // And a copy back the other way follows the origin the copied item carries, so the
    // item it came from is updated rather than duplicated.
    let back = engine
        .copy(&CopyRequest {
            destination: name("from"),
            ..one("into:T-1")
        })
        .await
        .expect("the copy runs");
    assert_eq!(
        landed(&back.items[0]),
        (Some("from:T-1".to_owned()), "updated".to_owned())
    );
    assert_eq!(listed(&engine, "from").await, vec!["from:T-1".to_owned()]);
}

#[tokio::test]
async fn a_rust_caller_is_refused_by_a_destination_configured_with_no_write_side() {
    let engine = engine_over(json!({
        "from": {"plugin": "in-memory", "config": {"tasks": [task("T-1", "Alpha engine")]}},
        "into": {
            "plugin": "in-memory",
            "config": {"capabilities": {"writes": "unsupported"}},
        },
    }));

    let Err(refusal) = engine.copy(&one("from:T-1")).await else {
        panic!("a destination with no write side must refuse");
    };
    assert!(
        matches!(&refusal, EngineError::NotWritable { name, kind }
            if name == "into" && kind == "in-memory"),
        "{refusal:?}"
    );
    let rendered = refusal.to_string();
    assert!(
        rendered.contains("source into cannot be written"),
        "{rendered}"
    );
    assert!(rendered.contains("its plugin is in-memory"), "{rendered}");
}

#[tokio::test]
async fn a_dry_run_reads_everything_and_writes_nothing() {
    let engine = pair();
    let planned = engine
        .copy(&CopyRequest {
            dry_run: true,
            ..one("from:T-1")
        })
        .await
        .expect("the copy runs");
    // Null only for a dry run that would create: there is no id, because nothing was.
    assert_eq!(
        planned.items[0].action,
        CopyAction::Created { destination: None }
    );
    assert!(listed(&engine, "into").await.is_empty());
}

#[tokio::test]
async fn an_id_that_names_nothing_and_a_destination_nothing_configures_are_both_refused() {
    let engine = pair();

    let Err(missing) = engine.copy(&one("from:absent")).await else {
        panic!("an id naming nothing must refuse");
    };
    assert!(
        matches!(&missing, EngineError::NoSuchItem { id } if id == "from:absent"),
        "{missing:?}"
    );

    let Err(unknown) = engine
        .copy(&CopyRequest {
            destination: name("nowhere"),
            ..one("from:T-1")
        })
        .await
    else {
        panic!("a destination nothing configures must refuse");
    };
    assert!(
        matches!(&unknown, EngineError::UnknownSource { name, .. } if name == "nowhere"),
        "{unknown:?}"
    );
}

#[tokio::test]
async fn a_stale_origin_refuses_until_recreate_says_to_create_instead() {
    // The item names an origin at `into` that `into` does not hold: the counterpart was
    // deleted or moved on purpose, and creating there would duplicate it.
    let engine = engine_over(json!({
        "from": {"plugin": "in-memory", "config": {"tasks": [{
            "id": "T-1", "title": "Alpha engine",
            "status": {"category": "todo", "name": "Todo"}, "labels": [],
            "metadata": {GlobalId::ORIGIN_KEY: "into:GONE"},
        }]}},
        "into": {"plugin": "in-memory", "config": {}},
    }));

    let Err(stale) = engine.copy(&one("from:T-1")).await else {
        panic!("an origin naming nothing at the destination must refuse");
    };
    assert!(
        matches!(&stale, EngineError::StaleOrigin { item, origin }
            if item == "from:T-1" && origin == "into:GONE"),
        "{stale:?}"
    );
    assert!(stale.to_string().contains("--recreate"), "{stale}");
    assert!(listed(&engine, "into").await.is_empty());

    let created = engine
        .copy(&CopyRequest {
            recreate: true,
            ..one("from:T-1")
        })
        .await
        .expect("--recreate falls through to the search rule");
    assert_eq!(
        landed(&created.items[0]),
        (Some("into:T-1".to_owned()), "created".to_owned())
    );
}

#[tokio::test]
async fn a_lost_origin_creates_a_second_item_until_match_by_re_establishes_it() {
    let engine = pair();
    engine.copy(&one("from:T-1")).await.expect("the copy runs");

    // A person edits the destination and removes the origin key: neither rule can find
    // the counterpart any more, so the next copy creates a second item.
    let engine = engine_over(json!({
        "from": {"plugin": "in-memory", "config": {"tasks": [task("T-1", "Alpha engine")]}},
        "into": {"plugin": "in-memory", "config": {"tasks": [task("T-1", "Alpha engine")]}},
    }));
    let duplicated = engine.copy(&one("from:T-1")).await.expect("the copy runs");
    assert_eq!(
        landed(&duplicated.items[0]),
        (Some("into:T-1-2".to_owned()), "created".to_owned())
    );

    // The caller-named escape re-establishes it without hand-editing ids.
    let engine = engine_over(json!({
        "from": {"plugin": "in-memory", "config": {"tasks": [task("T-1", "Alpha engine")]}},
        "into": {"plugin": "in-memory", "config": {"tasks": [task("OTHER", "Alpha engine")]}},
    }));
    let matched = engine
        .copy(&CopyRequest {
            match_by: Some(MatchBy::parse("title")),
            ..one("from:T-1")
        })
        .await
        .expect("the copy runs");
    assert_eq!(
        landed(&matched.items[0]),
        (Some("into:OTHER".to_owned()), "updated".to_owned())
    );

    // And on a metadata key of the caller's own choosing, for a title that moved.
    let engine = engine_over(json!({
        "from": {"plugin": "in-memory", "config": {"tasks": [task("T-1", "Alpha engine")]}},
        "into": {"plugin": "in-memory", "config": {"tasks": [task("OTHER", "Renamed")]}},
    }));
    let matched = engine
        .copy(&CopyRequest {
            match_by: Some(MatchBy::parse("caller.shape")),
            ..one("from:T-1")
        })
        .await
        .expect("the copy runs");
    assert_eq!(
        landed(&matched.items[0]),
        (Some("into:OTHER".to_owned()), "updated".to_owned())
    );
}

#[tokio::test]
async fn a_destination_that_cannot_carry_a_key_refuses_the_write_naming_it() {
    let engine = engine_over(json!({
        "from": {"plugin": "in-memory", "config": {"tasks": [task("T-1", "Alpha engine")]}},
        "into": {
            "plugin": "in-memory",
            "config": {"capabilities": {"unwritable_metadata_keys": ["caller.shape"]}},
        },
    }));

    let Err(refused) = engine.copy(&one("from:T-1")).await else {
        panic!("a destination that cannot carry a key must refuse the write");
    };
    let rendered = refused.to_string();
    assert!(
        rendered.contains("source into could not do it"),
        "{rendered}"
    );
    assert!(rendered.contains("caller.shape"), "{rendered}");
    assert!(listed(&engine, "into").await.is_empty());
}

#[tokio::test]
async fn copying_a_project_carries_its_tasks_and_reports_one_the_source_no_longer_holds() {
    let held = |tasks: Value| {
        json!({"plugin": "in-memory", "config": {
            "projects": [{"id": "P-1", "title": "Engine",
                          "status": {"category": "todo", "name": "Todo"}, "labels": []}],
            "tasks": tasks,
        }})
    };
    let member = |id: &str| {
        json!({"id": id, "title": id, "status": {"category": "todo", "name": "Todo"},
               "labels": [], "project": "P-1"})
    };
    let engine = engine_over(json!({
        "from": held(json!([member("T-1"), member("T-2")])),
        "into": {"plugin": "in-memory", "config": {}},
    }));

    let project = many(&["from:P-1"], CopyScope::Projects { tasks: true });
    let copied = engine.copy(&project).await.expect("the copy runs");
    assert_eq!(
        copied
            .items
            .iter()
            .map(|outcome| (outcome.source.to_string(), outcome.action.name()))
            .collect::<Vec<_>>(),
        vec![
            ("from:P-1".to_owned(), "created".to_owned()),
            ("from:T-1".to_owned(), "created".to_owned()),
            ("from:T-2".to_owned(), "created".to_owned()),
        ]
    );

    // A second copy matches each task independently and duplicates nothing.
    let again = engine.copy(&project).await.expect("the copy runs");
    assert!(
        again
            .items
            .iter()
            .all(|outcome| outcome.action.name() == "unchanged"),
        "{again:?}"
    );
    assert_eq!(
        listed(&engine, "into").await,
        vec!["into:T-1".to_owned(), "into:T-2".to_owned()]
    );

    // `--no-tasks` copies the project alone.
    let alone = engine
        .copy(&many(&["from:P-1"], CopyScope::Projects { tasks: false }))
        .await
        .expect("the copy runs");
    assert_eq!(alone.items.len(), 1);
    assert_eq!(alone.items[0].source, id("from:P-1"));
}

#[tokio::test]
async fn a_destination_item_the_source_no_longer_holds_is_left_alone_and_reported() {
    // The destination holds the counterpart of a task the source has since dropped. A
    // copy never deletes, so it stays exactly as it is and is reported as orphaned.
    let copied = |native: &str, origin: &str| {
        json!({"id": native, "title": native,
               "status": {"category": "todo", "name": "Todo"}, "labels": [],
               "project": "P-1", "metadata": {GlobalId::ORIGIN_KEY: origin}})
    };
    let project = |native: &str, origin: Value| {
        json!({"id": native, "title": "Engine",
               "status": {"category": "todo", "name": "Todo"}, "labels": [],
               "metadata": {GlobalId::ORIGIN_KEY: origin}})
    };
    let engine = engine_over(json!({
        "from": {"plugin": "in-memory", "config": {
            "projects": [{"id": "P-1", "title": "Engine",
                          "status": {"category": "todo", "name": "Todo"}, "labels": []}],
            "tasks": [{"id": "T-1", "title": "T-1",
                       "status": {"category": "todo", "name": "Todo"}, "labels": [],
                       "project": "P-1"}],
        }},
        "into": {"plugin": "in-memory", "config": {
            "projects": [project("P-1", json!("from:P-1"))],
            "tasks": [copied("T-1", "from:T-1"), copied("T-2", "from:T-2")],
        }},
    }));

    let report = engine
        .copy(&many(&["from:P-1"], CopyScope::Projects { tasks: true }))
        .await
        .expect("the copy runs");
    let orphan = report
        .items
        .iter()
        .find(|outcome| outcome.action.name() == "orphaned")
        .unwrap_or_else(|| panic!("no orphan was reported: {report:?}"));
    assert_eq!(orphan.source, id("from:T-2"));
    assert_eq!(orphan.destination(), Some(&id("into:T-2")));
    // Left exactly as it is: still there, and still saying what it said.
    let held = engine
        .task(&id("into:T-2"))
        .await
        .expect("the show verb answers");
    assert_eq!(held.items[0].item.title, "T-2");
}

#[tokio::test]
async fn the_edges_a_copy_read_are_written_and_a_far_end_that_leaves_the_set_is_qualified() {
    // Three kinds of far end, in one copy: one inside the copied set, which becomes the
    // destination's own id; one the source holds but the copy did not take, which is
    // qualified to the source it stays in; and one already naming another source, which
    // is left exactly as it is.
    let member = |id: &str| {
        json!({"id": id, "title": id, "status": {"category": "todo", "name": "Todo"},
               "labels": []})
    };
    let engine = engine_over(json!({
        "from": {"plugin": "in-memory", "config": {
            "tasks": [member("T-1"), member("T-2"), member("T-3")],
            "task_dependencies": [
                {"from": "T-1", "to": "T-2", "kind": "blocks"},
                {"from": "T-1", "to": "T-3", "kind": "related"},
                {"from": {"id": "T-1", "kind": "task"},
                 "to": {"id": "elsewhere:P-9", "kind": "project"}, "kind": "blocks"},
            ],
        }},
        "into": {"plugin": "in-memory", "config": {}},
    }));

    let copied = engine
        .copy(&many(&["from:T-1", "from:T-2"], CopyScope::Tasks))
        .await
        .expect("the copy runs");
    assert_eq!(copied.items.len(), 2);

    let edges = engine
        .task_dependencies(&onetaskgraph_core::DependencyRequest {
            id: id("into:T-1"),
            direction: onetaskgraph_plugin_api::Direction::DependsOn,
            paging: Paging {
                limit: NonZeroU32::new(50).expect("a non-zero limit"),
                token: None,
            },
        })
        .await
        .expect("the dependency verb answers");
    let mut ends: Vec<String> = edges
        .items
        .iter()
        .map(|edge| edge.to.id.to_string())
        .collect();
    ends.sort();
    assert_eq!(
        ends,
        vec![
            // The member of the copied set, remapped to the id the destination gave it —
            // and it was written *after* this item, so the second pass is what repaired
            // this edge.
            "into:T-2".to_owned(),
            // The far end that leaves the copied set, and the one that already had.
            "elsewhere:P-9".to_owned(),
            "from:T-3".to_owned(),
        ]
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
    );

    // Copying back the other way unqualifies the far end that names the destination's
    // own source, because that is how a source names its own items.
    let back = engine
        .copy(&CopyRequest {
            destination: name("from"),
            ..one("into:T-1")
        })
        .await
        .expect("the copy runs");
    assert_eq!(back.items[0].destination(), Some(&id("from:T-1")));
    let edges = engine
        .task_dependencies(&onetaskgraph_core::DependencyRequest {
            id: id("from:T-1"),
            direction: onetaskgraph_plugin_api::Direction::DependsOn,
            paging: Paging {
                limit: NonZeroU32::new(50).expect("a non-zero limit"),
                token: None,
            },
        })
        .await
        .expect("the dependency verb answers");
    assert!(
        edges.items.iter().any(|edge| edge.to.id == id("from:T-3")),
        "{edges:?}"
    );
}

#[tokio::test]
async fn a_task_copied_on_its_own_is_filed_under_the_destinations_own_counterpart() {
    let filed = json!({"id": "T-1", "title": "Alpha",
                       "status": {"category": "todo", "name": "Todo"},
                       "labels": [], "project": "P-1"});
    let project = |id: &str, origin: Option<&str>| {
        let mut project = json!({"id": id, "title": "Engine",
                                 "status": {"category": "todo", "name": "Todo"}, "labels": []});
        if let Some(origin) = origin {
            project["metadata"] = json!({GlobalId::ORIGIN_KEY: origin});
        }
        project
    };

    // The destination holds the counterpart of the task's own project, so the copied task
    // is filed under *that* rather than under an id of the source's the destination never
    // issued.
    let engine = engine_over(json!({
        "from": {"plugin": "in-memory", "config": {
            "projects": [project("P-1", None)], "tasks": [filed],
        }},
        "into": {"plugin": "in-memory", "config": {
            "projects": [project("LOCAL-7", Some("from:P-1"))],
        }},
    }));
    engine.copy(&one("from:T-1")).await.expect("the copy runs");
    let copied = engine
        .task(&id("into:T-1"))
        .await
        .expect("the show verb answers");
    assert_eq!(
        copied.items[0].item.project,
        Some(onetaskgraph_plugin_api::NativeId::from("LOCAL-7"))
    );

    // With no counterpart there, the source's own opaque id is carried rather than
    // dropped: this engine does not interpret it, and losing it would lose what the
    // source said.
    let engine = engine_over(json!({
        "from": {"plugin": "in-memory", "config": {
            "projects": [project("P-1", None)], "tasks": [filed],
        }},
        "into": {"plugin": "in-memory", "config": {}},
    }));
    engine.copy(&one("from:T-1")).await.expect("the copy runs");
    let copied = engine
        .task(&id("into:T-1"))
        .await
        .expect("the show verb answers");
    assert_eq!(
        copied.items[0].item.project,
        Some(onetaskgraph_plugin_api::NativeId::from("P-1"))
    );
}

#[tokio::test]
async fn a_project_origin_that_still_names_something_updates_that_project_directly() {
    let engine = engine_over(json!({
        "from": {"plugin": "in-memory", "config": {"projects": [{
            "id": "P-1", "title": "Renamed", "status": {"category": "todo", "name": "Todo"},
            "labels": [], "metadata": {GlobalId::ORIGIN_KEY: "into:BOARD"},
        }]}},
        "into": {"plugin": "in-memory", "config": {"projects": [{
            "id": "BOARD", "title": "Engine", "status": {"category": "todo", "name": "Todo"},
            "labels": [],
        }]}},
    }));

    let copied = engine
        .copy(&many(&["from:P-1"], CopyScope::Projects { tasks: false }))
        .await
        .expect("the copy runs");
    assert_eq!(
        landed(&copied.items[0]),
        (Some("into:BOARD".to_owned()), "updated".to_owned())
    );
    assert_eq!(
        engine
            .project(&id("into:BOARD"))
            .await
            .expect("the show verb answers")
            .items[0]
            .item
            .title,
        "Renamed"
    );
}

#[tokio::test]
async fn a_destination_that_could_not_be_built_and_a_source_that_could_not_be_read_both_refuse() {
    // A source that is configured and did not build is not fatal to a *query* — it lands
    // in that response's errors and the others still answer. A copy is one write into one
    // destination, and half of one is not an answer, so both ends refuse by name.
    let broken = json!({"plugin": "local-md", "config": {"root": "/onetaskgraph/not/a/folder"}});
    let engine = engine_over(json!({
        "from": {"plugin": "in-memory", "config": {"tasks": [task("T-1", "Alpha engine")]}},
        "into": broken,
    }));
    let Err(unavailable) = engine.copy(&one("from:T-1")).await else {
        panic!("a destination that could not be built must refuse");
    };
    assert!(
        matches!(&unavailable, EngineError::DestinationUnavailable { name, .. } if name == "into"),
        "{unavailable:?}"
    );
    assert!(
        unavailable.to_string().contains("could not be built"),
        "{unavailable}"
    );

    let engine = engine_over(json!({
        "from": broken,
        "into": {"plugin": "in-memory", "config": {}},
    }));
    let Err(unreadable) = engine.copy(&one("from:T-1")).await else {
        panic!("a source that could not be built must refuse");
    };
    assert!(
        matches!(&unreadable, EngineError::SourceRefused { name, .. } if name == "from"),
        "{unreadable:?}"
    );
}

#[tokio::test]
async fn the_scan_that_finds_a_counterpart_walks_the_destination_a_page_at_a_time() {
    // One page at a time and nothing written down, which is the same bound every other
    // compensation in this engine works under — so a destination that serves one row per
    // page still finds the counterpart sitting at the end of it.
    let held = |id: &str, origin: &str| {
        json!({"id": id, "title": id, "status": {"category": "todo", "name": "Todo"},
               "labels": [], "metadata": {GlobalId::ORIGIN_KEY: origin}})
    };
    let engine = engine_over(json!({
        "from": {"plugin": "in-memory", "config": {"tasks": [task("T-1", "Alpha engine")]}},
        "into": {"plugin": "in-memory", "config": {
            "capabilities": {"max_page_size": 1},
            "tasks": [
                held("A", "somewhere:1"),
                held("B", "somewhere:2"),
                held("C", "from:T-1"),
            ],
        }},
    }));

    let copied = engine.copy(&one("from:T-1")).await.expect("the copy runs");
    assert_eq!(
        landed(&copied.items[0]),
        (Some("into:C".to_owned()), "updated".to_owned())
    );
}

/// Two projects, with the dependencies that only one copied set can resolve: a task on its
/// sibling, a task on a task in the *other* named project, and a project on that project.
fn interlinked() -> Value {
    json!({"plugin": "in-memory", "config": {
        "projects": [
            {"id": "P-1", "title": "Engine",
             "status": {"category": "todo", "name": "Todo"}, "labels": []},
            {"id": "P-2", "title": "Docs",
             "status": {"category": "todo", "name": "Todo"}, "labels": []},
        ],
        "tasks": [
            {"id": "T-1", "title": "Alpha engine",
             "status": {"category": "todo", "name": "Todo"}, "labels": [], "project": "P-1"},
            {"id": "T-2", "title": "Beta",
             "status": {"category": "todo", "name": "Todo"}, "labels": [], "project": "P-1"},
            {"id": "T-3", "title": "Gamma",
             "status": {"category": "todo", "name": "Todo"}, "labels": [], "project": "P-2"},
        ],
        "task_dependencies": [
            {"from": "T-1", "to": "T-2", "kind": "blocks"},
            {"from": "T-1", "to": "T-3", "kind": "blocks"},
        ],
        "project_dependencies": [
            {"from": "P-1", "to": "P-2", "kind": "blocks"},
        ],
    }})
}

/// Every forward edge at one item, as `<far id> <kind>` pairs.
async fn depends_on(engine: &Engine, near: &str) -> Vec<String> {
    let response = engine
        .task_dependencies(&DependencyRequest {
            id: id(near),
            direction: Direction::DependsOn,
            paging: Paging {
                limit: NonZeroU32::new(50).expect("a non-zero limit"),
                token: None,
            },
        })
        .await
        .expect("the dependency verb answers");
    assert!(
        response.errors.is_empty(),
        "a dependency read must not fail: {:?}",
        response.errors
    );
    response
        .items
        .into_iter()
        .map(|edge| format!("{} {:?}", edge.to.id, edge.to.kind))
        .collect()
}

#[tokio::test]
async fn a_copy_resolves_a_dependency_on_an_item_it_created_in_the_same_run() {
    // The defect: a copy could not see the items it had itself created. Every project was
    // copied on its own, so a task's edge to a sibling in *another* named project, and a
    // task's edge to the project it belongs to, were both written as the id the far end
    // had at its **source** — a reference the destination has never heard of — or refused
    // outright by a destination that checks its far ends, naming an item that same run had
    // just created.
    let engine = engine_over(json!({
        "from": interlinked(),
        "into": {"plugin": "in-memory", "config": {}},
    }));

    let report = engine
        .copy(&many(
            &["from:P-1", "from:P-2"],
            CopyScope::Projects { tasks: true },
        ))
        .await
        .expect("the copy runs");
    assert!(
        report
            .items
            .iter()
            .all(|outcome| outcome.action.name() == "created"),
        "{report:?}"
    );

    // Every edge points at the destination's own item, by the destination's own id.
    assert_eq!(
        depends_on(&engine, "into:T-1").await,
        vec!["into:T-2 Task".to_owned(), "into:T-3 Task".to_owned()]
    );
    // Including the project's own edge to the other project of the same copy.
    let projects = engine
        .project_dependencies(&DependencyRequest {
            id: id("into:P-1"),
            direction: Direction::DependsOn,
            paging: Paging {
                limit: NonZeroU32::new(50).expect("a non-zero limit"),
                token: None,
            },
        })
        .await
        .expect("the dependency verb answers");
    assert_eq!(
        projects
            .items
            .iter()
            .map(|edge| edge.to.id.to_string())
            .collect::<Vec<_>>(),
        vec!["into:P-2".to_owned()]
    );
}

#[tokio::test]
async fn a_copy_that_cannot_finish_leaves_the_destination_as_it_found_it() {
    // A copy is either complete or it never happened. A half-written project has to be run
    // again, and the re-run is the mutation burst that trips a hosted destination's
    // secondary rate limiter — so undoing this run's own writes is what removes the retry
    // at source. `Beta` is the item this destination will not create, and it is the second
    // task of the project, so the project and the first task have already landed when it
    // refuses.
    let engine = engine_over(json!({
        "from": interlinked(),
        "into": {"plugin": "in-memory", "config": {
            "capabilities": {"uncreatable_titles": ["Beta"]},
        }},
    }));

    let Err(refused) = engine
        .copy(&many(&["from:P-1"], CopyScope::Projects { tasks: true }))
        .await
    else {
        panic!("a destination that will not create an item must refuse the copy");
    };
    let rendered = refused.to_string();
    assert!(rendered.contains("Beta"), "{rendered}");
    assert!(
        !rendered.contains("could not be undone"),
        "the destination can be put back, so the copy must not report otherwise: {rendered}"
    );

    // The destination holds none of that copy's items — not the project written first, and
    // not the task that landed before the refusal.
    assert!(listed(&engine, "into").await.is_empty());
    let projects = engine
        .projects(&onetaskgraph_core::ProjectRequest {
            sources: vec![name("into")],
            filters: onetaskgraph_core::Filters::default(),
            paging: Paging {
                limit: NonZeroU32::new(50).expect("a non-zero limit"),
                token: None,
            },
        })
        .await
        .expect("the project list answers");
    assert!(projects.items.is_empty(), "{:?}", projects.items);
}

#[tokio::test]
async fn a_copy_that_cannot_be_undone_names_what_it_left_behind() {
    // Undoing is best effort, and a destination that will not take one of its own items
    // back leaves work the copy owes the user the name of. Told only that the copy failed,
    // they would copy again over a destination nobody has described to them — which is the
    // retry this whole mechanism exists to remove. So the refusal carries both halves: why
    // the copy failed, why it could not be undone, and what is still there.
    let engine = engine_over(json!({
        "from": interlinked(),
        "into": {"plugin": "in-memory", "config": {
            "capabilities": {
                "uncreatable_titles": ["Beta"],
                "undeletable_ids": ["P-1"],
            },
        }},
    }));

    let Err(refused) = engine
        .copy(&many(&["from:P-1"], CopyScope::Projects { tasks: true }))
        .await
    else {
        panic!("the copy must refuse");
    };
    let rendered = refused.to_string();
    assert!(rendered.contains("could not be undone"), "{rendered}");
    // Why it failed, why the undo failed, and the qualified id still sitting there.
    assert!(rendered.contains("Beta"), "{rendered}");
    assert!(rendered.contains("will not remove P-1"), "{rendered}");
    assert!(rendered.contains("into:P-1"), "{rendered}");

    // And it is telling the truth: the project it names is there, and the task it managed
    // to take back is not.
    let projects = engine
        .projects(&onetaskgraph_core::ProjectRequest {
            sources: vec![name("into")],
            filters: onetaskgraph_core::Filters::default(),
            paging: Paging {
                limit: NonZeroU32::new(50).expect("a non-zero limit"),
                token: None,
            },
        })
        .await
        .expect("the project list answers");
    assert_eq!(
        projects
            .items
            .iter()
            .map(|project| project.id.to_string())
            .collect::<Vec<_>>(),
        vec!["into:P-1".to_owned()]
    );
    assert!(listed(&engine, "into").await.is_empty());
}

/// One destination item recorded as the counterpart of `origin`, reading differently from
/// the source so a copy of it is a real update rather than an `unchanged`.
fn counterpart(id: &str, origin: &str, project: Option<&str>) -> Value {
    let mut item = json!({
        "id": id,
        "title": format!("{id} as it was"),
        "content": "as it was",
        "status": {"category": "todo", "name": "Todo"},
        "labels": [],
        "metadata": {GlobalId::ORIGIN_KEY: origin},
    });
    if let Some(project) = project {
        item["project"] = json!(project);
    }
    item
}

/// Every task and project one source holds, as `<id> <title>` pairs.
async fn held(engine: &Engine, source: &str) -> Vec<String> {
    let paging = || Paging {
        limit: NonZeroU32::new(50).expect("a non-zero limit"),
        token: None,
    };
    let projects = engine
        .projects(&onetaskgraph_core::ProjectRequest {
            sources: vec![name(source)],
            filters: onetaskgraph_core::Filters::default(),
            paging: paging(),
        })
        .await
        .expect("the project list answers");
    let tasks = engine
        .tasks(&TaskRequest {
            sources: vec![name(source)],
            filters: onetaskgraph_core::Filters::default(),
            project: onetaskgraph_core::ProjectSelector::Any,
            paging: paging(),
        })
        .await
        .expect("the task list answers");
    projects
        .items
        .into_iter()
        .map(|project| format!("{} {}", project.id, project.item.title))
        .chain(
            tasks
                .items
                .into_iter()
                .map(|task| format!("{} {}", task.id, task.item.title)),
        )
        .collect()
}

/// A destination already holding a counterpart of every item of [`interlinked`] but `T-3`.
fn already_holding() -> Value {
    json!({
        "projects": [
            counterpart("D-P1", "from:P-1", None),
            counterpart("D-P2", "from:P-2", None),
        ],
        "tasks": [
            counterpart("D-T1", "from:T-1", Some("D-P1")),
            counterpart("D-T2", "from:T-2", Some("D-P1")),
        ],
    })
}

#[tokio::test]
async fn a_second_copy_updates_every_counterpart_and_repairs_the_edges_among_them() {
    // The destination already holds a counterpart of everything but `T-3`, recorded by
    // origin the way an earlier copy left it and reading differently from the source. So
    // every one of them is a real update, and `P-1` and `T-1` are written twice — once as
    // they land, once when the edges whose far ends did not exist yet are repaired.
    let engine = engine_over(json!({
        "from": interlinked(),
        "into": {"plugin": "in-memory", "config": already_holding()},
    }));

    let report = engine
        .copy(&many(
            &["from:P-1", "from:P-2"],
            CopyScope::Projects { tasks: true },
        ))
        .await
        .expect("the copy runs");
    assert_eq!(
        report
            .items
            .iter()
            .map(|outcome| (outcome.source.to_string(), outcome.action.name()))
            .collect::<Vec<_>>(),
        vec![
            ("from:P-1".to_owned(), "updated".to_owned()),
            ("from:T-1".to_owned(), "updated".to_owned()),
            ("from:T-2".to_owned(), "updated".to_owned()),
            ("from:P-2".to_owned(), "updated".to_owned()),
            ("from:T-3".to_owned(), "created".to_owned()),
        ]
    );

    // Every edge names the destination's own item, including the one whose far end was
    // created in a project this copy reached after the item that points at it.
    assert_eq!(
        depends_on(&engine, "into:D-T1").await,
        vec!["into:D-T2 Task".to_owned(), "into:T-3 Task".to_owned()]
    );
}

#[tokio::test]
async fn a_copy_that_cannot_finish_puts_back_the_items_it_overwrote() {
    // Undoing is not only about the items a copy created. The four counterparts here were
    // at the destination before this copy started and are overwritten by it, and `Gamma`
    // is the item this destination will not create — so the copy refuses after four
    // successful writes, and every one of those four has to read as it did before rather
    // than as this copy's first pass left it.
    let mut into = already_holding();
    into["capabilities"] = json!({"uncreatable_titles": ["Gamma"]});
    let engine = engine_over(json!({
        "from": interlinked(),
        "into": {"plugin": "in-memory", "config": into},
    }));
    let before = held(&engine, "into").await;
    assert_eq!(
        before,
        vec![
            "into:D-P1 D-P1 as it was".to_owned(),
            "into:D-P2 D-P2 as it was".to_owned(),
            "into:D-T1 D-T1 as it was".to_owned(),
            "into:D-T2 D-T2 as it was".to_owned(),
        ]
    );

    let Err(refused) = engine
        .copy(&many(
            &["from:P-1", "from:P-2"],
            CopyScope::Projects { tasks: true },
        ))
        .await
    else {
        panic!("a destination that will not create an item must refuse the copy");
    };
    assert!(refused.to_string().contains("Gamma"), "{refused}");
    assert!(
        !refused.to_string().contains("could not be undone"),
        "this destination takes its items back: {refused}"
    );

    assert_eq!(
        held(&engine, "into").await,
        before,
        "every item this copy overwrote reads as it did before it started"
    );
}

/// One source project of two tasks, the second of which no destination here will create.
fn one_project_of_two_tasks() -> Value {
    json!({"plugin": "in-memory", "config": {
        "projects": [
            {"id": "P-1", "title": "Engine",
             "status": {"category": "todo", "name": "Todo"}, "labels": []},
        ],
        "tasks": [
            {"id": "T-1", "title": "Alpha engine",
             "status": {"category": "todo", "name": "Todo"}, "labels": [], "project": "P-1"},
            {"id": "T-9", "title": "Gamma",
             "status": {"category": "todo", "name": "Todo"}, "labels": [], "project": "P-1"},
        ],
    }})
}

/// A destination whose task table and project table each hold something called `SHARED`.
///
/// Nothing makes a destination number its two kinds in one namespace, and nothing stops it
/// either: a Markdown store filing `shared.md` as a task beside `shared.md` as a project is
/// the ordinary case rather than the contrived one. So a destination id says which item is
/// meant only once the kind is beside it.
fn sharing_one_id() -> Value {
    let mut project = counterpart("SHARED", "from:P-1", None);
    project["title"] = json!("the project as it was");
    let mut task = counterpart("SHARED", "from:T-1", Some("SHARED"));
    task["title"] = json!("the task as it was");
    json!({
        "projects": [project],
        "tasks": [task],
        "capabilities": {"uncreatable_titles": ["Gamma"]},
    })
}

#[tokio::test]
async fn an_undo_tells_a_task_from_a_project_sharing_one_destination_id() {
    // Both counterparts are updated by this copy and both are journalled under `SHARED`,
    // so a journal that identifies an entry by id alone reads the second as a repeat of
    // the first and drops it. Then `Gamma` cannot be created, the copy undoes itself, and
    // the entry it dropped is the one item nothing puts back — a destination left holding
    // half of a copy that reported it had left nothing behind.
    let engine = engine_over(json!({
        "from": one_project_of_two_tasks(),
        "into": {"plugin": "in-memory", "config": sharing_one_id()},
    }));
    let before = held(&engine, "into").await;
    assert_eq!(
        before,
        vec![
            "into:SHARED the project as it was".to_owned(),
            "into:SHARED the task as it was".to_owned(),
        ]
    );

    let Err(refused) = engine
        .copy(&many(&["from:P-1"], CopyScope::Projects { tasks: true }))
        .await
    else {
        panic!("a destination that will not create an item must refuse the copy");
    };
    assert!(refused.to_string().contains("Gamma"), "{refused}");
    assert!(
        !refused.to_string().contains("could not be undone"),
        "this destination takes its items back: {refused}"
    );

    assert_eq!(
        held(&engine, "into").await,
        before,
        "both items sharing that id are put back, not whichever of them was journalled first"
    );
}

/// A source whose task carries the id the destination already files a project under.
fn a_task_named_like_the_destinations_project() -> Value {
    json!({"plugin": "in-memory", "config": {
        "projects": [
            {"id": "P-1", "title": "Engine",
             "status": {"category": "todo", "name": "Todo"}, "labels": []},
        ],
        "tasks": [
            {"id": "SHARED", "title": "Alpha engine",
             "status": {"category": "todo", "name": "Todo"}, "labels": [], "project": "P-1"},
            {"id": "T-9", "title": "Gamma",
             "status": {"category": "todo", "name": "Todo"}, "labels": [], "project": "P-1"},
        ],
    }})
}

/// A destination holding only the project `SHARED`, and refusing to create `Gamma`.
fn holding_only_the_project() -> Value {
    let mut project = counterpart("SHARED", "from:P-1", None);
    project["title"] = json!("the project as it was");
    json!({
        "projects": [project],
        "capabilities": {"uncreatable_titles": ["Gamma"]},
    })
}

#[tokio::test]
async fn an_item_created_under_one_kind_does_not_hold_back_the_others_restore() {
    // The far side of the same confusion. An undo removes what this copy created rather
    // than restoring it, so every created id is one the restores must skip — and this copy
    // creates a *task* called `SHARED` while updating a *project* that was called `SHARED`
    // before it started. Skipped by id alone, the project is left reading as this copy
    // wrote it: the one item a "nothing was left behind" refusal did leave behind.
    let engine = engine_over(json!({
        "from": a_task_named_like_the_destinations_project(),
        "into": {"plugin": "in-memory", "config": holding_only_the_project()},
    }));
    let before = held(&engine, "into").await;
    assert_eq!(before, vec!["into:SHARED the project as it was".to_owned()]);

    let Err(refused) = engine
        .copy(&many(&["from:P-1"], CopyScope::Projects { tasks: true }))
        .await
    else {
        panic!("a destination that will not create an item must refuse the copy");
    };
    assert!(refused.to_string().contains("Gamma"), "{refused}");
    assert!(
        !refused.to_string().contains("could not be undone"),
        "this destination takes its items back: {refused}"
    );

    assert_eq!(
        held(&engine, "into").await,
        before,
        "the project is restored, and the task this copy created under its id is gone"
    );
}

#[tokio::test]
async fn a_copy_that_stops_part_way_through_an_update_puts_that_item_back_too() {
    // The other half of undoing an overwrite. A destination's own write is several calls —
    // `docs/plugin-protocol.md` §4.9, and the GitHub source's own suite drives one failing
    // after an earlier one landed — so an update can end with the item already changed. No
    // source can put that back: only this journal holds what was there. Recorded after a
    // successful write, this was the one way a copy could stop and leave a destination
    // altered, which is exactly what "either complete or it never happened" forbids.
    //
    // `Beta` is the title this destination applies and then refuses, and it is the third
    // of three updates — so the two before it have landed and the third is half written.
    let mut into = already_holding();
    into["capabilities"] = json!({"half_written_titles": ["Beta"]});
    let engine = engine_over(json!({
        "from": interlinked(),
        "into": {"plugin": "in-memory", "config": into},
    }));
    let before = held(&engine, "into").await;
    assert_eq!(
        before,
        vec![
            "into:D-P1 D-P1 as it was".to_owned(),
            "into:D-P2 D-P2 as it was".to_owned(),
            "into:D-T1 D-T1 as it was".to_owned(),
            "into:D-T2 D-T2 as it was".to_owned(),
        ]
    );

    let Err(refused) = engine
        .copy(&many(&["from:P-1"], CopyScope::Projects { tasks: true }))
        .await
    else {
        panic!("a destination that stops part way through a write must refuse the copy");
    };
    let rendered = refused.to_string();
    assert!(rendered.contains("Beta"), "{rendered}");
    assert!(
        !rendered.contains("could not be undone"),
        "this destination takes its items back: {rendered}"
    );

    assert_eq!(
        held(&engine, "into").await,
        before,
        "the item the write had already changed reads as it did before the copy started"
    );
}

#[tokio::test]
async fn a_restore_the_destination_refuses_names_the_item_left_holding_this_copys_writing() {
    // The item a destination will not take back need not be one this copy created. `D-T2`
    // was here before it started, carrying a key set at the destination that this source
    // will not accept in a write — so the copy overwrites it happily, using metadata of
    // its own, and cannot write the original back. `Gamma` then fails, and the undo that
    // follows puts three of the four items back and is refused the fourth.
    //
    // Both halves have to reach the user: told only that the copy failed, they would copy
    // again over a destination holding one item's content from a run nobody described.
    let mut into = already_holding();
    into["tasks"][1]["metadata"]["reviewed-by"] = json!("a person at the destination");
    into["capabilities"] = json!({
        "uncreatable_titles": ["Gamma"],
        "unwritable_metadata_keys": ["reviewed-by"],
    });
    let engine = engine_over(json!({
        "from": interlinked(),
        "into": {"plugin": "in-memory", "config": into},
    }));

    let Err(refused) = engine
        .copy(&many(
            &["from:P-1", "from:P-2"],
            CopyScope::Projects { tasks: true },
        ))
        .await
    else {
        panic!("a destination that will not create an item must refuse the copy");
    };

    let EngineError::CopyNotUndone { left_behind, .. } = &refused else {
        panic!("a refused restore must report the copy as not undone: {refused}");
    };
    assert_eq!(
        left_behind
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        vec!["into:D-T2".to_owned()],
        "only the item the destination refused is still this copy's"
    );

    let rendered = refused.to_string();
    // Why the copy failed, why the undo failed, and the one item still holding its writing.
    assert!(rendered.contains("Gamma"), "{rendered}");
    assert!(rendered.contains("reviewed-by"), "{rendered}");
    assert!(rendered.contains("into:D-T2"), "{rendered}");

    // And it is telling the truth about which item that is: everything else reads as it
    // did before the copy, and `D-T2` reads as this copy left it.
    assert_eq!(
        held(&engine, "into").await,
        vec![
            "into:D-P1 D-P1 as it was".to_owned(),
            "into:D-P2 D-P2 as it was".to_owned(),
            "into:D-T1 D-T1 as it was".to_owned(),
            "into:D-T2 Beta".to_owned(),
        ]
    );
}
