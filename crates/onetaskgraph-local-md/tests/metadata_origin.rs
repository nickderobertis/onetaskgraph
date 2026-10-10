//! A task list narrowed by caller metadata and by copy origin over a folder of Markdown, both
//! read from the `metadata:` map each task file's front matter already holds.
//!
//! Every test drives the real plugin over a real folder.

use std::fs;

use onetaskgraph_plugin_api::{
    ItemWrite, MetadataMatch, NativeId, PageRequest, SecretResolver, SourceName, SourcePlugin,
    Status, StatusCategory, Support, Task, TaskQuery, TaskSource,
};
use secrecy::SecretString;
use serde_json::json;

struct NoSecrets;
impl SecretResolver for NoSecrets {
    fn get(&self, _: &str) -> Option<SecretString> {
        None
    }
}

/// Five tasks: one tagged with a nested root cause, one with the same value one level up,
/// one with a longer value, one copied from `work:ENG-1`, and one copied from an id that only
/// begins with it.
fn folder() -> (tempfile::TempDir, Box<dyn TaskSource>) {
    let root = tempfile::tempdir().expect("temporary notes");
    let tasks = root.path().join("tasks");
    fs::create_dir_all(&tasks).expect("the tasks folder");
    for (name, metadata) in [
        (
            "tagged",
            "{orchestrator.follow-up: {root_cause: stale-cache}, team.owner: ada}",
        ),
        ("shallow", "{orchestrator.follow-up: stale-cache}"),
        (
            "longer",
            "{orchestrator.follow-up: {root_cause: stale-cache-2}}",
        ),
        ("copied", "{onetaskgraph.origin: \"work:ENG-1\"}"),
        ("near", "{onetaskgraph.origin: \"work:ENG-10\"}"),
    ] {
        fs::write(
            tasks.join(format!("{name}.md")),
            format!("---\ntitle: {name}\nstatus: todo\nmetadata: {metadata}\n---\nThe body.\n"),
        )
        .expect("a task file");
    }
    let source = onetaskgraph_local_md::Plugin
        .build(
            &SourceName::new("work").unwrap(),
            &json!({ "root": root.path() }),
            &NoSecrets,
        )
        .expect("the folder builds");
    (root, source)
}

async fn listed(source: &dyn TaskSource, query: &TaskQuery) -> Vec<String> {
    let mut ids: Vec<String> = source
        .query_tasks(
            query,
            &PageRequest {
                limit: 200,
                cursor: None,
            },
        )
        .await
        .expect("answers")
        .items
        .iter()
        .map(|task| task.id.to_string())
        .collect();
    ids.sort();
    ids
}

fn at(key: &str, path: &[&str], value: &str) -> MetadataMatch {
    MetadataMatch::new(
        key.to_owned(),
        path.iter().map(|segment| (*segment).to_owned()).collect(),
        value.to_owned(),
    )
    .expect("a metadata location")
}

#[test]
fn both_predicates_are_declared_native() {
    let (_root, source) = folder();
    let declared = source.capabilities();
    assert_eq!(declared.filter_by_metadata, Support::Native);
    assert_eq!(declared.filter_by_origin, Support::Native);
}

#[tokio::test]
async fn a_metadata_match_keeps_exactly_the_tasks_holding_that_string_there() {
    let (_root, source) = folder();
    let query = |matches: Vec<MetadataMatch>| TaskQuery {
        metadata: matches,
        ..TaskQuery::default()
    };
    assert_eq!(
        listed(
            source.as_ref(),
            &query(vec![at(
                "orchestrator.follow-up",
                &["root_cause"],
                "stale-cache"
            )])
        )
        .await,
        ["tagged"]
    );
    assert_eq!(
        listed(
            source.as_ref(),
            &query(vec![at("orchestrator.follow-up", &[], "stale-cache")])
        )
        .await,
        ["shallow"]
    );
    assert_eq!(
        listed(
            source.as_ref(),
            &query(vec![
                at("orchestrator.follow-up", &["root_cause"], "stale-cache"),
                at("team.owner", &[], "ada"),
            ])
        )
        .await,
        ["tagged"],
        "every match holds"
    );
    assert_eq!(
        listed(
            source.as_ref(),
            &query(vec![
                at("orchestrator.follow-up", &["root_cause"], "stale-cache"),
                at("team.owner", &[], "bob"),
            ])
        )
        .await,
        Vec::<String>::new(),
        "the matches are ANDed"
    );
}

#[tokio::test]
async fn an_origin_keeps_exactly_the_tasks_copied_from_that_id() {
    let (_root, source) = folder();
    let query = |origin: &str| TaskQuery {
        origin: Some(origin.to_owned()),
        ..TaskQuery::default()
    };
    assert_eq!(
        listed(source.as_ref(), &query("work:ENG-1")).await,
        ["copied"]
    );
    assert_eq!(
        listed(source.as_ref(), &query("work:ENG-10")).await,
        ["near"]
    );
    assert_eq!(
        listed(source.as_ref(), &query("work:ENG")).await,
        Vec::<String>::new(),
        "a prefix of an origin is not that origin"
    );
}

#[tokio::test]
async fn a_task_copied_in_is_found_by_its_origin_straight_after_the_write() {
    let (_root, source) = folder();
    let id = source
        .write_task(&ItemWrite {
            target: None,
            item: Task {
                id: NativeId::from("ignored"),
                key: None,
                title: "copied in".to_owned(),
                content: None,
                status: Status {
                    category: StatusCategory::Todo,
                    name: "todo".to_owned(),
                },
                priority: onetaskgraph_plugin_api::Priority::None,
                labels: Vec::new(),
                project: None,
                url: None,
                location: None,
                created_at: None,
                updated_at: None,
                metadata: [("onetaskgraph.origin".to_owned(), json!("notes:N-1"))]
                    .into_iter()
                    .collect(),
                repositories: Vec::new(),
                delivers: Vec::new(),
                delivered_by: Vec::new(),
                classification: Default::default(),
            },
            depends_on: Vec::new(),
        })
        .await
        .expect("the copy lands");
    assert_eq!(
        listed(
            source.as_ref(),
            &TaskQuery {
                origin: Some("notes:N-1".to_owned()),
                ..TaskQuery::default()
            }
        )
        .await,
        [id.to_string()]
    );
}
