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
    Config, CopyAction, CopyRequest, Engine, EngineError, GlobalId, MatchBy, Paging, TaskRequest,
};
use onetaskgraph_plugin_api::{ItemKind, SecretResolver, SourceName};
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
    CopyRequest {
        items: vec![id(item)],
        kind: ItemKind::Task,
        destination: name("into"),
        include_tasks: true,
        match_by: None,
        recreate: false,
        dry_run: false,
    }
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
    assert_eq!(created.items[0].destination, Some(id("into:T-1")));
    assert_eq!(created.items[0].action, CopyAction::Created);

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
    assert_eq!(again.items[0].action, CopyAction::Unchanged);
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
    assert_eq!(back.items[0].destination, Some(id("from:T-1")));
    assert_eq!(back.items[0].action, CopyAction::Updated);
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
    assert_eq!(planned.items[0].action, CopyAction::Created);
    // Null only for a dry run that would create: there is no id, because nothing was.
    assert_eq!(planned.items[0].destination, None);
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
    assert_eq!(created.items[0].action, CopyAction::Created);
    assert_eq!(created.items[0].destination, Some(id("into:T-1")));
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
    assert_eq!(duplicated.items[0].action, CopyAction::Created);
    assert_eq!(duplicated.items[0].destination, Some(id("into:T-1-2")));

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
    assert_eq!(matched.items[0].action, CopyAction::Updated);
    assert_eq!(matched.items[0].destination, Some(id("into:OTHER")));

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
    assert_eq!(matched.items[0].action, CopyAction::Updated);
    assert_eq!(matched.items[0].destination, Some(id("into:OTHER")));
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

    let project = CopyRequest {
        items: vec![id("from:P-1")],
        kind: ItemKind::Project,
        ..one("from:P-1")
    };
    let copied = engine.copy(&project).await.expect("the copy runs");
    assert_eq!(
        copied
            .items
            .iter()
            .map(|outcome| (outcome.source.to_string(), outcome.action))
            .collect::<Vec<_>>(),
        vec![
            ("from:P-1".to_owned(), CopyAction::Created),
            ("from:T-1".to_owned(), CopyAction::Created),
            ("from:T-2".to_owned(), CopyAction::Created),
        ]
    );

    // A second copy matches each task independently and duplicates nothing.
    let again = engine.copy(&project).await.expect("the copy runs");
    assert!(
        again
            .items
            .iter()
            .all(|outcome| outcome.action == CopyAction::Unchanged),
        "{again:?}"
    );
    assert_eq!(
        listed(&engine, "into").await,
        vec!["into:T-1".to_owned(), "into:T-2".to_owned()]
    );

    // `--no-tasks` copies the project alone.
    let alone = engine
        .copy(&CopyRequest {
            include_tasks: false,
            ..project
        })
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
        .copy(&CopyRequest {
            items: vec![id("from:P-1")],
            kind: ItemKind::Project,
            ..one("from:P-1")
        })
        .await
        .expect("the copy runs");
    let orphan = report
        .items
        .iter()
        .find(|outcome| outcome.action == CopyAction::Orphaned)
        .unwrap_or_else(|| panic!("no orphan was reported: {report:?}"));
    assert_eq!(orphan.source, id("from:T-2"));
    assert_eq!(orphan.destination, Some(id("into:T-2")));
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
        .copy(&CopyRequest {
            items: vec![id("from:T-1"), id("from:T-2")],
            ..one("from:T-1")
        })
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
            items: vec![id("into:T-1")],
            destination: name("from"),
            ..one("into:T-1")
        })
        .await
        .expect("the copy runs");
    assert_eq!(back.items[0].destination, Some(id("from:T-1")));
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
        .copy(&CopyRequest {
            items: vec![id("from:P-1")],
            kind: ItemKind::Project,
            include_tasks: false,
            ..one("from:P-1")
        })
        .await
        .expect("the copy runs");
    assert_eq!(copied.items[0].action, CopyAction::Updated);
    assert_eq!(copied.items[0].destination, Some(id("into:BOARD")));
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
    assert_eq!(copied.items[0].action, CopyAction::Updated);
    assert_eq!(copied.items[0].destination, Some(id("into:C")));
}
