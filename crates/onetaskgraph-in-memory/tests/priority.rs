//! The in-memory source's priority — held, filtered, declared, and written — and its narrow
//! content write, driven through the real trait and the plugin's own configuration reader.

use onetaskgraph_in_memory::Plugin;
use onetaskgraph_plugin_api::{
    ItemWrite, NativeId, PageRequest, Priority, SecretResolver, SourceError, SourceName,
    SourcePlugin, Support, Task, TaskQuery, TaskSource,
};
use secrecy::SecretString;
use serde_json::{Value, json};

struct NoSecrets;
impl SecretResolver for NoSecrets {
    fn get(&self, _var: &str) -> Option<SecretString> {
        None
    }
}

/// Four tasks, one at each of four priorities and one with none written at all.
fn source(capabilities: Value) -> Result<Box<dyn TaskSource>, SourceError> {
    Plugin.build(
        &SourceName::new("work").expect("a name"),
        &json!({
            "capabilities": capabilities,
            "tasks": [
                {"id": "T-1", "title": "Fire", "content": "now",
                 "status": {"category": "todo", "name": "Todo"}, "labels": [],
                 "priority": "urgent", "metadata": {"myapp.kept": [1]}},
                {"id": "T-2", "title": "Soon", "content": null,
                 "status": {"category": "todo", "name": "Todo"}, "labels": [],
                 "priority": "high"},
                {"id": "T-3", "title": "Later", "content": null,
                 "status": {"category": "backlog", "name": "Backlog"}, "labels": [],
                 "priority": "low"},
                {"id": "T-4", "title": "Unranked", "content": null,
                 "status": {"category": "todo", "name": "Todo"}, "labels": []}
            ]
        }),
        &NoSecrets,
    )
}

fn id(value: &str) -> NativeId {
    NativeId(value.to_owned())
}

fn whole() -> PageRequest {
    PageRequest {
        cursor: None,
        limit: 50,
    }
}

async fn listed(source: &dyn TaskSource, priorities: Vec<Priority>) -> Vec<String> {
    source
        .query_tasks(
            &TaskQuery {
                priorities,
                ..TaskQuery::default()
            },
            &whole(),
        )
        .await
        .expect("answers")
        .items
        .iter()
        .map(|task| task.id.to_string())
        .collect()
}

#[tokio::test]
async fn a_configured_priority_is_held_and_an_absent_one_reads_as_none() {
    let source = source(json!({})).expect("builds");
    assert_eq!(source.capabilities().priority, Support::Native);
    assert_eq!(source.capabilities().filter_by_priority, Support::Native);
    let read = |task: Option<Task>| task.expect("held").priority;
    assert_eq!(
        read(source.get_task(&id("T-1")).await.unwrap()),
        Priority::Urgent
    );
    assert_eq!(
        read(source.get_task(&id("T-4")).await.unwrap()),
        Priority::None
    );
}

#[tokio::test]
async fn a_source_filtering_natively_keeps_any_listed_priority_and_one_that_does_not_ignores_it() {
    let native = source(json!({})).expect("builds");
    assert_eq!(
        listed(native.as_ref(), vec![Priority::Urgent, Priority::Low]).await,
        ["T-1", "T-3"]
    );
    assert_eq!(listed(native.as_ref(), vec![Priority::None]).await, ["T-4"]);
    assert_eq!(
        listed(native.as_ref(), Vec::new()).await,
        ["T-1", "T-2", "T-3", "T-4"]
    );

    // Rule 2: a source declaring the predicate unsupported returns the wider set.
    let ignoring = source(json!({"filter_by_priority": "unsupported"})).expect("builds");
    assert_eq!(
        ignoring.capabilities().filter_by_priority,
        Support::Unsupported
    );
    assert_eq!(
        listed(ignoring.as_ref(), vec![Priority::Urgent]).await,
        ["T-1", "T-2", "T-3", "T-4"]
    );
}

#[tokio::test]
async fn a_source_declaring_no_priority_refuses_one_configured_written_or_set() {
    let Err(SourceError::Config { message }) = source(json!({"priority": "unsupported"})) else {
        panic!("a configured priority under a source holding none is refused");
    };
    assert!(
        message.contains("task T-1 is configured with the priority urgent")
            && message.contains("capabilities.priority: native"),
        "{message}"
    );

    let config = json!({
        "capabilities": {"priority": "unsupported"},
        "tasks": [{"id": "T-1", "title": "Fire", "content": null,
                   "status": {"category": "todo", "name": "Todo"}, "labels": []}]
    });
    let holding_none = Plugin
        .build(&SourceName::new("bare").unwrap(), &config, &NoSecrets)
        .expect("a source without priorities builds");
    let mut task = holding_none.get_task(&id("T-1")).await.unwrap().unwrap();
    assert_eq!(task.priority, Priority::None);
    // `none` is what it already holds, so a write carrying it lands.
    holding_none
        .write_task(&ItemWrite {
            target: Some(id("T-1")),
            item: task.clone(),
            depends_on: Vec::new(),
        })
        .await
        .expect("a write carrying none lands");
    task.priority = Priority::High;
    let Err(SourceError::Refused { message }) = holding_none
        .write_task(&ItemWrite {
            target: Some(id("T-1")),
            item: task,
            depends_on: Vec::new(),
        })
        .await
    else {
        panic!("a write carrying a priority is refused");
    };
    assert_eq!(
        message,
        "the in-memory plugin cannot write a task's priority on its own"
    );
    assert!(
        holding_none
            .set_task_priority(&id("T-1"), Priority::Low)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn a_priority_set_on_its_own_moves_nothing_else_and_none_clears_it() {
    let source = source(json!({})).expect("builds");
    let before = source.get_task(&id("T-1")).await.unwrap().unwrap();
    assert_eq!(
        source
            .set_task_priority(&id("T-1"), Priority::Medium)
            .await
            .unwrap(),
        Some(Priority::Medium)
    );
    let after = source.get_task(&id("T-1")).await.unwrap().unwrap();
    assert_eq!(
        after,
        Task {
            priority: Priority::Medium,
            ..before.clone()
        }
    );
    assert_eq!(
        source
            .set_task_priority(&id("T-1"), Priority::None)
            .await
            .unwrap(),
        Some(Priority::None)
    );
    assert_eq!(
        source
            .set_task_priority(&id("T-9"), Priority::High)
            .await
            .unwrap(),
        None,
        "a task this source does not hold is not found rather than created"
    );
}

#[tokio::test]
async fn a_content_set_on_its_own_replaces_the_body_byte_for_byte_and_moves_nothing_else() {
    let source = source(json!({})).expect("builds");
    let before = source.get_task(&id("T-1")).await.unwrap().unwrap();
    let written = "line one\r\n\n  indented — and a trailing newline\n";
    assert_eq!(
        source.set_task_content(&id("T-1"), written).await.unwrap(),
        Some(())
    );
    let after = source.get_task(&id("T-1")).await.unwrap().unwrap();
    assert_eq!(
        after,
        Task {
            content: Some(written.to_owned()),
            ..before
        }
    );
    assert_eq!(
        source.set_task_content(&id("T-9"), "x").await.unwrap(),
        None
    );

    let read_only = self::source(json!({"writes": "unsupported"})).expect("builds");
    let Err(SourceError::Refused { message }) = read_only.set_task_content(&id("T-1"), "x").await
    else {
        panic!("a source with no write side refuses");
    };
    assert_eq!(message, "the in-memory plugin cannot be written");
}
