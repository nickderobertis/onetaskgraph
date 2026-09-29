//! The in-memory source's metadata and origin filters — declared, applied over the metadata
//! each task holds, and ignored when a configuration declares them unsupported — driven
//! through the real trait and the plugin's own configuration reader.

use onetaskgraph_in_memory::Plugin;
use onetaskgraph_plugin_api::{
    MetadataMatch, PageRequest, SecretResolver, SourceName, SourcePlugin, Support, TaskQuery,
    TaskSource,
};
use secrecy::SecretString;
use serde_json::{Value, json};

struct NoSecrets;
impl SecretResolver for NoSecrets {
    fn get(&self, _var: &str) -> Option<SecretString> {
        None
    }
}

/// Four tasks: one tagged with a nested root cause, one with that value as a number, one
/// copied from `work:ENG-1`, and one copied from an id that only begins with it.
fn source(capabilities: Value) -> Box<dyn TaskSource> {
    let task = |id: &str, metadata: Value| {
        json!({"id": id, "title": id, "content": null,
               "status": {"category": "todo", "name": "todo"}, "labels": [],
               "metadata": metadata})
    };
    Plugin
        .build(
            &SourceName::new("work").expect("a name"),
            &json!({
                "capabilities": capabilities,
                "tasks": [
                    task("T-tagged", json!({"orchestrator.follow-up": {"root_cause": "stale-cache"}})),
                    task("T-number", json!({"orchestrator.follow-up": {"root_cause": 3}})),
                    task("T-copied", json!({"onetaskgraph.origin": "work:ENG-1"})),
                    task("T-near", json!({"onetaskgraph.origin": "work:ENG-10"})),
                ]
            }),
            &NoSecrets,
        )
        .expect("builds")
}

async fn listed(source: &dyn TaskSource, query: &TaskQuery) -> Vec<String> {
    source
        .query_tasks(
            query,
            &PageRequest {
                cursor: None,
                limit: 50,
            },
        )
        .await
        .expect("answers")
        .items
        .into_iter()
        .map(|task| task.id.0)
        .collect()
}

fn narrowed() -> [(TaskQuery, Vec<&'static str>); 3] {
    [
        (
            TaskQuery {
                metadata: vec![
                    MetadataMatch::new(
                        "orchestrator.follow-up".to_owned(),
                        vec!["root_cause".to_owned()],
                        "stale-cache".to_owned(),
                    )
                    .expect("a metadata location"),
                ],
                ..TaskQuery::default()
            },
            vec!["T-tagged"],
        ),
        (
            TaskQuery {
                metadata: vec![
                    MetadataMatch::new(
                        "orchestrator.follow-up".to_owned(),
                        vec!["root_cause".to_owned()],
                        "3".to_owned(),
                    )
                    .expect("a metadata location"),
                ],
                ..TaskQuery::default()
            },
            vec![],
        ),
        (
            TaskQuery {
                origin: Some("work:ENG-1".to_owned()),
                ..TaskQuery::default()
            },
            vec!["T-copied"],
        ),
    ]
}

#[tokio::test]
async fn both_predicates_are_native_by_default_and_applied_over_each_tasks_metadata() {
    let source = source(json!({}));
    let declared = source.capabilities();
    assert_eq!(declared.filter_by_metadata, Support::Native);
    assert_eq!(declared.filter_by_origin, Support::Native);
    for (query, expected) in narrowed() {
        assert_eq!(listed(source.as_ref(), &query).await, expected, "{query:?}");
    }
}

#[tokio::test]
async fn a_source_declaring_them_unsupported_ignores_them_and_returns_the_wider_set() {
    let source = source(json!({"filter_by_metadata": "unsupported",
                                "filter_by_origin": "unsupported"}));
    for (query, _) in narrowed() {
        assert_eq!(
            listed(source.as_ref(), &query).await,
            ["T-tagged", "T-number", "T-copied", "T-near"],
            "rule 2: a predicate declared unsupported is ignored, never half-applied: {query:?}"
        );
    }
}
