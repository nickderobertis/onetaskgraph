//! `task status set` and the `delivers` relation as a **Rust caller** reaches them.
//!
//! Every test drives the engine's own public methods over real `in-memory` sources, which keep
//! what they are written for as long as the engine holding them lives — so a rule that writes
//! one task and reads another is observable here within one engine. The journeys that drive
//! the same verbs through the binary, over sources that outlive a process, are in
//! `crates/onetaskgraph/tests/e2e/delivery.rs`.

use std::num::NonZeroU32;

use onetaskgraph_core::{
    Config, CopyItems, CopyRequest, CopyScope, Delivered, DeliveryOutcome, Engine, EngineError,
    Filters, GlobalId, Paging, ProjectSelector, SearchKind, SearchRequest, TaskRequest, settled,
};
use onetaskgraph_plugin_api::{
    SecretResolver, SourceName, StatusCategory, TaskRef, TextFields, TextQuery,
};
use secrecy::SecretString;
use serde_json::{Value, json};

use StatusCategory::{Backlog, Cancelled, Done, Draft, InProgress, Queued, Todo, Unknown};

struct NoSecrets;
impl SecretResolver for NoSecrets {
    fn get(&self, _var: &str) -> Option<SecretString> {
        None
    }
}

fn engine_over(sources: Value) -> Engine {
    let config =
        Config::from_document(json!({ "sources": sources })).expect("a valid configuration");
    Engine::build(&config, &NoSecrets)
}

fn id(value: &str) -> GlobalId {
    value.parse().expect("a qualified id")
}

fn spelled(category: StatusCategory) -> Value {
    serde_json::to_value(category).expect("a category serialises")
}

/// One task an `in-memory` source holds.
fn task(native: &str, category: StatusCategory, delivers: &[&str], delivered_by: &[&str]) -> Value {
    json!({
        "id": native,
        "title": native,
        "content": null,
        "status": {"category": spelled(category), "name": spelled(category)},
        "labels": [],
        "delivers": delivers,
        "delivered_by": delivered_by,
    })
}

fn source(tasks: Vec<Value>) -> Value {
    json!({"plugin": "in-memory", "config": {"tasks": tasks}})
}

async fn read(engine: &Engine, qualified: &str) -> (StatusCategory, Vec<String>, Vec<String>) {
    let response = engine
        .task(&id(qualified))
        .await
        .expect("the task verb answers");
    let task = &response.items.first().expect("held").item;
    (
        task.status.category,
        task.delivers.iter().map(ToString::to_string).collect(),
        task.delivered_by.iter().map(ToString::to_string).collect(),
    )
}

/// What one entry came to, as a comparable value.
fn outcome(entry: &Delivered) -> (String, String, String, Vec<String>) {
    let said = match &entry.outcome {
        DeliveryOutcome::Written { from, to } => format!("written {from:?}->{to:?}"),
        DeliveryOutcome::Unchanged { from } => format!("unchanged {from:?}"),
        DeliveryOutcome::Left { from } => format!("left {from:?}"),
        DeliveryOutcome::Failed { from, failure } => {
            format!(
                "failed {from:?}: {}",
                failure.message().lines().next().unwrap_or_default()
            )
        }
    };
    (
        entry.ticket.to_string(),
        entry.deliverer.to_string(),
        said,
        entry.pruned.iter().map(ToString::to_string).collect(),
    )
}

#[test]
fn the_result_rule_is_decided_by_its_first_matching_branch() {
    for (categories, expected) in [
        (vec![Done], Some(Done)),
        (vec![Done, Cancelled], Some(Done)),
        (vec![Cancelled, Cancelled], Some(Todo)),
        (vec![InProgress], Some(InProgress)),
        (vec![InProgress, Cancelled], Some(InProgress)),
        (vec![InProgress, Queued], Some(InProgress)),
        (vec![Queued], Some(Queued)),
        (vec![Queued, Todo], Some(Queued)),
        (vec![Done, Todo], Some(Todo)),
        (vec![Done, Queued], Some(Queued)),
        (vec![Done, Backlog], Some(Todo)),
        (vec![Done, Unknown], Some(Todo)),
        (vec![Todo], Some(Todo)),
        (vec![Unknown], Some(Todo)),
        (vec![Cancelled], Some(Todo)),
        (vec![], Some(Todo)),
        (vec![Draft], None),
        (vec![Backlog], None),
        (vec![Draft, Backlog], None),
    ] {
        assert_eq!(settled(&categories), expected, "{categories:?}");
    }
}

#[tokio::test]
async fn a_delivered_task_follows_its_one_deliverer_and_records_it() {
    let engine = engine_over(json!({
        "plan": source(vec![task("P-1", Todo, &["tickets:T-1"], &[])]),
        "tickets": source(vec![task("T-1", Todo, &[], &[])]),
    }));
    for (category, from, to) in [
        (Queued, Todo, Queued),
        (InProgress, Queued, InProgress),
        (Done, InProgress, Done),
    ] {
        let set = engine
            .set_task_status(&id("plan:P-1"), category)
            .await
            .expect("set");
        assert_eq!(set.id, id("plan:P-1"));
        assert_eq!(set.status.category, category);
        assert_eq!(
            set.delivered.iter().map(outcome).collect::<Vec<_>>(),
            [(
                "tickets:T-1".to_owned(),
                "plan:P-1".to_owned(),
                format!("written {from:?}->{to:?}"),
                Vec::new()
            )]
        );
        let (held, _, by) = read(&engine, "tickets:T-1").await;
        assert_eq!(held, to);
        assert_eq!(by, ["plan:P-1"]);
    }
    // A ticket the rule has finished is left alone however its deliverer moves after.
    let set = engine
        .set_task_status(&id("plan:P-1"), Todo)
        .await
        .expect("set");
    assert_eq!(
        set.delivered
            .iter()
            .map(outcome)
            .map(|o| o.2)
            .collect::<Vec<_>>(),
        ["left Done"]
    );
}

#[tokio::test]
async fn a_release_returns_a_ticket_to_todo_and_draft_or_backlog_writes_nothing() {
    for (deliverer, ticket_after, said) in [
        (Todo, Todo, "written InProgress->Todo"),
        (Cancelled, Todo, "written InProgress->Todo"),
        (Unknown, Todo, "written InProgress->Todo"),
        (Draft, InProgress, "unchanged InProgress"),
        (Backlog, InProgress, "unchanged InProgress"),
    ] {
        let engine = engine_over(json!({
            "plan": source(vec![task("P-1", Todo, &["tickets:T-1"], &[])]),
            "tickets": source(vec![task("T-1", InProgress, &[], &["plan:P-1"])]),
        }));
        let set = engine
            .set_task_status(&id("plan:P-1"), deliverer)
            .await
            .expect("set");
        assert_eq!(
            set.delivered
                .iter()
                .map(outcome)
                .map(|o| o.2)
                .collect::<Vec<_>>(),
            [said],
            "{deliverer:?}"
        );
        assert_eq!(
            read(&engine, "tickets:T-1").await.0,
            ticket_after,
            "{deliverer:?}"
        );
    }
}

#[tokio::test]
async fn a_ticket_the_rule_may_not_touch_is_left_and_keeps_its_status() {
    for held in [Draft, Backlog, Unknown, Done, Cancelled] {
        let engine_with_ticket = engine_over(json!({
            "plan": source(vec![task("P-1", Todo, &["tickets:T-1"], &[])]),
            "tickets": source(vec![task("T-1", held, &[], &[])]),
        }));
        let set = engine_with_ticket
            .set_task_status(&id("plan:P-1"), InProgress)
            .await
            .expect("set");
        assert_eq!(
            set.delivered
                .iter()
                .map(outcome)
                .map(|o| o.2)
                .collect::<Vec<_>>(),
            [format!("left {held:?}")]
        );
        let (after, _, by) = read(&engine_with_ticket, "tickets:T-1").await;
        assert_eq!(after, held);
        assert_eq!(by, ["plan:P-1"], "the back-reference is kept even so");
    }
}

#[tokio::test]
async fn two_deliverers_decide_a_ticket_together() {
    for (other, mine, expected) in [
        (InProgress, Cancelled, InProgress),
        (Todo, Queued, Queued),
        (Todo, Done, Todo),
        (Queued, Done, Queued),
        (Backlog, Done, Todo),
        (Cancelled, Cancelled, Todo),
        (Done, Done, Done),
        (Cancelled, Done, Done),
    ] {
        let engine = engine_over(json!({
            "plan": source(vec![
                task("P-1", Todo, &["tickets:T-1"], &[]),
                task("P-2", other, &["tickets:T-1"], &[]),
            ]),
            "tickets": source(vec![task("T-1", Queued, &[], &["plan:P-2"])]),
        }));
        engine
            .set_task_status(&id("plan:P-1"), mine)
            .await
            .expect("set");
        let (after, _, by) = read(&engine, "tickets:T-1").await;
        assert_eq!(after, expected, "{other:?} beside {mine:?}");
        assert_eq!(by, ["plan:P-2", "plan:P-1"]);
    }
}

#[tokio::test]
async fn a_deliverer_read_as_not_found_is_pruned_and_one_nothing_configures_fails_and_stays() {
    let engine = engine_over(json!({
        "plan": source(vec![task("P-1", Todo, &["tickets:T-1", "tickets:T-2"], &[])]),
        "tickets": source(vec![
            task("T-1", Todo, &[], &["plan:P-GONE"]),
            task("T-2", Todo, &[], &["elsewhere:X-1"]),
        ]),
    }));
    let set = engine
        .set_task_status(&id("plan:P-1"), InProgress)
        .await
        .expect("set");
    let entries: Vec<_> = set.delivered.iter().map(outcome).collect();
    assert_eq!(
        entries[0],
        (
            "tickets:T-1".to_owned(),
            "plan:P-1".to_owned(),
            "written Todo->InProgress".to_owned(),
            vec!["plan:P-GONE".to_owned()]
        )
    );
    assert_eq!(read(&engine, "tickets:T-1").await.2, ["plan:P-1"]);
    assert!(
        entries[1]
            .2
            .starts_with("failed Some(Todo): no source named \"elsewhere\" is configured"),
        "{entries:?}"
    );
    assert!(set.delivered[1].failed() && !set.delivered[0].failed());
    let (after, _, by) = read(&engine, "tickets:T-2").await;
    assert_eq!(
        after, Todo,
        "the rule does not guess past a deliverer it could not read"
    );
    assert_eq!(by, ["elsewhere:X-1", "plan:P-1"], "and prunes nothing");
}

#[tokio::test]
async fn a_ticket_that_cannot_be_read_or_written_is_failed_and_the_deliverer_still_lands() {
    let engine = engine_over(json!({
        "plan": source(vec![task(
            "P-1",
            Todo,
            &["nowhere:T-1", "tickets:T-GONE", "frozen:T-1", "frozen:T-2", "broken:T-1"],
            &[],
        )]),
        "tickets": source(vec![]),
        "frozen": {"plugin": "in-memory", "config": {
            "capabilities": {"writes": "unsupported"},
            "tasks": [task("T-1", Todo, &[], &[]), task("T-2", Todo, &[], &["plan:P-1"])],
        }},
        "broken": {"plugin": "local-md", "config": {"root": "/nonexistent/onetaskgraph/root"}},
    }));
    let set = engine
        .set_task_status(&id("plan:P-1"), Queued)
        .await
        .expect("set");
    assert_eq!(
        set.status.category, Queued,
        "the deliverer's own write landed"
    );
    let said: Vec<String> = set.delivered.iter().map(outcome).map(|o| o.2).collect();
    assert!(
        said[0].starts_with("failed None: no source named \"nowhere\""),
        "{said:?}"
    );
    assert!(
        said[1].starts_with("failed None: no task with the id tickets:T-GONE"),
        "{said:?}"
    );
    assert!(
        said[2].starts_with("failed Some(Todo): source frozen could not do it"),
        "{said:?}"
    );
    assert!(
        said[3].starts_with("failed Some(Todo): source frozen could not do it"),
        "{said:?}"
    );
    assert!(
        said[4].starts_with("failed None: source broken could not be built"),
        "{said:?}"
    );
    assert!(set.delivered.iter().all(Delivered::failed));
    assert_eq!(read(&engine, "plan:P-1").await.0, Queued);
}

#[tokio::test]
async fn a_status_set_is_refused_by_name_before_anything_is_written() {
    let engine = engine_over(json!({
        "plan": source(vec![task("P-1", Todo, &[], &[])]),
        "frozen": {"plugin": "in-memory", "config": {"capabilities": {"writes": "unsupported"},
                   "tasks": [task("T-1", Todo, &[], &[])]}},
        "broken": {"plugin": "local-md", "config": {"root": "/nonexistent/onetaskgraph/root"}},
    }));
    let refused = |error: EngineError| error.to_string();
    let unknown = refused(
        engine
            .set_task_status(&id("gone:T-1"), Done)
            .await
            .expect_err("refused"),
    );
    assert!(
        unknown.starts_with("no source named \"gone\" is configured"),
        "{unknown}"
    );
    let frozen = refused(
        engine
            .set_task_status(&id("frozen:T-1"), Done)
            .await
            .expect_err("refused"),
    );
    assert!(
        frozen.starts_with("source frozen cannot write a status: its plugin is in-memory"),
        "{frozen}"
    );
    let missing = refused(
        engine
            .set_task_status(&id("plan:P-9"), Done)
            .await
            .expect_err("refused"),
    );
    assert!(
        missing.starts_with("no task with the id plan:P-9"),
        "{missing}"
    );
    let broken = refused(
        engine
            .set_task_status(&id("broken:T-1"), Done)
            .await
            .expect_err("refused"),
    );
    assert!(
        broken.starts_with("source broken could not be built"),
        "{broken}"
    );
    let nothing = engine
        .set_task_status(&id("plan:P-1"), Done)
        .await
        .expect("set");
    assert!(
        nothing.delivered.is_empty(),
        "a task delivering nothing reports nothing"
    );
}

#[tokio::test]
async fn every_verb_reports_both_lists_qualified() {
    let engine = engine_over(json!({
        "work": source(vec![task("T-1", Todo, &["T-2", "other:X-1"], &["plan:P-1"]), task("T-2", Todo, &[], &[])]),
    }));
    let (_, delivers, by) = read(&engine, "work:T-1").await;
    assert_eq!(delivers, ["work:T-2", "other:X-1"]);
    assert_eq!(by, ["plan:P-1"]);
    let listed = engine
        .tasks(&TaskRequest {
            sources: Vec::new(),
            filters: Filters::default(),
            project: ProjectSelector::Any,
            priorities: Vec::new(),
            paging: Paging {
                limit: NonZeroU32::new(10).unwrap(),
                token: None,
            },
        })
        .await
        .expect("listed");
    assert_eq!(
        listed.items[0].item.delivers,
        [
            TaskRef::new("work:T-2").unwrap(),
            TaskRef::new("other:X-1").unwrap()
        ]
    );
    let hits = engine
        .search(&SearchRequest {
            sources: Vec::new(),
            text: TextQuery {
                terms: "T-1".to_owned(),
                fields: TextFields::Title,
            },
            kind: SearchKind::Tasks,
            paging: Paging {
                limit: NonZeroU32::new(10).unwrap(),
                token: None,
            },
        })
        .await
        .expect("searched");
    let rendered = serde_json::to_value(&hits.items).expect("renders");
    assert_eq!(
        rendered[0]["item"]["delivers"],
        json!(["work:T-2", "other:X-1"])
    );
}

fn copy(items: &[&str], scope: CopyScope, into: &str) -> CopyRequest {
    CopyRequest {
        items: CopyItems::new(items.iter().map(|item| id(item)).collect()).expect("an item"),
        scope,
        destination: SourceName::new(into).expect("a name"),
        match_by: None,
        recreate: false,
        dry_run: false,
    }
}

#[tokio::test]
async fn a_copy_rewrites_members_carries_the_rest_qualified_and_keeps_the_destinations_delivered_by()
 {
    let engine = engine_over(json!({
        "from": source(vec![
            task("A", Queued, &["B", "tickets:T-1", "C"], &["plan:P-9"]),
            // Already where the rule will put it, so a second copy of B has nothing to change.
            task("B", Queued, &[], &[]),
            task("C", Todo, &[], &[]),
        ]),
        "into": source(vec![]),
        "tickets": source(vec![task("T-1", Todo, &[], &[])]),
    }));
    let report = engine
        .copy(&copy(&["from:A", "from:B"], CopyScope::Tasks, "into"))
        .await
        .expect("copied");
    assert_eq!(report.delivers_rewritten, 1, "B is the one member A names");
    let (_, delivers, by) = read(&engine, "into:A").await;
    assert_eq!(delivers, ["into:B", "tickets:T-1", "from:C"]);
    assert!(
        by.is_empty(),
        "a copy never takes delivered_by from its source"
    );
    assert_eq!(
        report.delivered.iter().map(outcome).collect::<Vec<_>>(),
        [
            (
                "into:B".to_owned(),
                "into:A".to_owned(),
                "unchanged Queued".to_owned(),
                Vec::new()
            ),
            (
                "tickets:T-1".to_owned(),
                "into:A".to_owned(),
                "written Todo->Queued".to_owned(),
                Vec::new()
            ),
            (
                "from:C".to_owned(),
                "into:A".to_owned(),
                "written Todo->Queued".to_owned(),
                Vec::new()
            ),
        ]
    );
    assert_eq!(read(&engine, "into:B").await.2, ["into:A"]);
    assert_eq!(
        read(&engine, "tickets:T-1").await,
        (Queued, Vec::new(), vec!["into:A".to_owned()])
    );

    // A second copy changes nothing about A, keeps B's back-reference the store wrote, and
    // re-evaluates every ticket all the same.
    let again = engine
        .copy(&copy(&["from:A", "from:B"], CopyScope::Tasks, "into"))
        .await
        .expect("copied again");
    assert!(
        again
            .items
            .iter()
            .all(|item| item.action.name() == "unchanged"),
        "{again:?}"
    );
    assert_eq!(
        again
            .delivered
            .iter()
            .map(outcome)
            .map(|o| o.2)
            .collect::<Vec<_>>(),
        ["unchanged Queued", "unchanged Queued", "unchanged Queued"]
    );
    assert_eq!(read(&engine, "into:B").await.2, ["into:A"]);

    // A dry run writes nothing, so it keeps nothing in step either.
    let dry = engine
        .copy(&CopyRequest {
            dry_run: true,
            ..copy(&["from:A"], CopyScope::Tasks, "into")
        })
        .await
        .expect("dry");
    assert!(dry.delivered.is_empty());
}

#[tokio::test]
async fn a_member_created_later_in_the_same_copy_is_named_by_its_destination_id() {
    let engine = engine_over(json!({
        "from": source(vec![task("A", InProgress, &["B"], &[]), task("B", Todo, &[], &[])]),
        "into": {"plugin": "in-memory", "config": {"tasks": [task("B", Draft, &[], &[])]}},
    }));
    // `into` already holds an unrelated `B`, so the copied B lands as `B-2` after A was
    // written: A is completed by the repair pass.
    let report = engine
        .copy(&copy(&["from:A", "from:B"], CopyScope::Tasks, "into"))
        .await
        .expect("copied");
    let (_, delivers, _) = read(&engine, "into:A").await;
    assert_eq!(delivers, ["into:B-2"]);
    assert_eq!(
        read(&engine, "into:B-2").await,
        (InProgress, Vec::new(), vec!["into:A".to_owned()])
    );
    assert_eq!(
        read(&engine, "into:B").await.0,
        Draft,
        "the unrelated B is untouched"
    );
    assert_eq!(report.delivered.len(), 1);
}
