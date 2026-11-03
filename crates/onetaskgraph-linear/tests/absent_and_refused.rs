//! A read by an id Linear does not hold, and a create Linear refuses, over a real HTTP loopback
//! that answers both as Linear does.
//!
//! Linear answers `issue(id:)`, `project(id:)` and `document(id:)` naming nothing with an
//! errored response — `Entity not found: <type>` — never a null, and it sends that under HTTP
//! 400 as readily as under a 200 carrying an `errors` array. The loopback here answers both
//! ways. What it proves is the contract the engine's copy rests on: an absent id reads as
//! absent on every read path — plain, cached and for a write — and a create Linear refuses
//! fails with that create's own refusal, never with what the look for an earlier attempt's
//! item answered.

use onetaskgraph_plugin_api::{
    Document, ItemWrite, MetadataKey, MetadataRecord, NativeId, PageRequest, Project,
    SecretResolver, SourceError, SourceName, SourcePlugin, Task, TaskSource, WriteTarget,
};
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{Arc, Mutex},
    thread,
};

use onetaskgraph_linear::graphql;

struct Secrets;
impl SecretResolver for Secrets {
    fn get(&self, _: &str) -> Option<secrecy::SecretString> {
        Some("fixture-key".into())
    }
}

/// How the loopback answers one request: an HTTP status and a body.
type Answer = (u16, Value);

/// A loopback Linear answering each request through `handler`, given its GraphQL document and
/// its variables, and keeping every document it was sent.
fn loopback(
    handler: impl Fn(&str, &Value) -> Answer + Send + 'static,
) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/graphql", listener.local_addr().unwrap());
    let sent = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&sent);
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let Some(request) = read_request(&stream) else {
                continue;
            };
            let query = request["query"].as_str().unwrap_or_default().to_owned();
            let (status, body) = handler(&query, &request["variables"]);
            recorded.lock().unwrap().push(query);
            let text = body.to_string();
            let reason = if status == 200 { "OK" } else { "Bad Request" };
            let mut stream = stream;
            let _ = write!(
                stream,
                "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                text.len()
            );
        }
    });
    (endpoint, sent)
}

fn read_request(mut stream: &TcpStream) -> Option<Value> {
    let mut bytes = Vec::new();
    let mut chunk = [0; 8192];
    loop {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        bytes.extend_from_slice(&chunk[..n]);
        let Some(split) = bytes.windows(4).position(|w| w == b"\r\n\r\n") else {
            continue;
        };
        let head = String::from_utf8_lossy(&bytes[..split]).to_ascii_lowercase();
        let length = head
            .lines()
            .find_map(|line| line.strip_prefix("content-length: "))
            .and_then(|value| value.trim().parse::<usize>().ok())?;
        if bytes.len() >= split + 4 + length {
            return serde_json::from_slice(&bytes[split + 4..split + 4 + length]).ok();
        }
    }
}

/// Linear's errored answer to a read or a mutation addressing a `kind` it does not hold.
fn not_found(kind: &str) -> Value {
    json!({"errors":[{"message":format!("Entity not found: {kind}"),
        "extensions":{"type":"invalid input","code":"INPUT_ERROR","statusCode":400,
            "userError":true,"userPresentableMessage":format!("Could not find referenced {kind}.")}}],
        "data":null})
}

/// The two statuses Linear sends an errored answer under.
const STATUSES: [u16; 2] = [400, 200];

/// The kind of item a document reads by id, if it is one.
fn read_kind(query: &str) -> Option<&'static str> {
    let query = query.trim_start();
    if !query.starts_with("query") {
        return None;
    }
    [
        ("issue(id:", "Issue"),
        ("project(id:", "Project"),
        ("document(id:", "Document"),
    ]
    .into_iter()
    .find(|(field, _)| query.contains(field))
    .map(|(_, kind)| kind)
}

fn source(endpoint: &str) -> Box<dyn TaskSource> {
    onetaskgraph_linear::Plugin
        .build(
            &SourceName::new("work").unwrap(),
            &json!({"endpoint":endpoint,"team":"ENG",
                "status_mapping":{"todo":"Todo","in-progress":"In Progress","done":"Done"}}),
            &Secrets,
        )
        .unwrap()
}

/// What [`graphql::RESOLUTION`] answers for [`source`]'s team.
fn resolution() -> Value {
    let page = |nodes: Value| json!({"nodes":nodes,"pageInfo":{"hasNextPage":false}});
    json!({"data":{"organization":{"id":"ORG"},
        "teams":{"nodes":[{"id":"TEAM","states":page(json!([
            {"id":"STATE","name":"Todo","type":"unstarted"},
            {"id":"STATE-IN-PROGRESS","name":"In Progress","type":"started"},
            {"id":"STATE-DONE","name":"Done","type":"completed"}]))}]},
        "projectStatuses":page(json!([
            {"id":"STATUS","name":"Todo","type":"planned","position":1.0},
            {"id":"STATUS-IN-PROGRESS","name":"In Progress","type":"started","position":2.0},
            {"id":"STATUS-DONE","name":"Done","type":"completed","position":3.0}]))}})
}

/// A create key, as the engine mints one, and an id Linear holds nothing under.
const KEY: &str = "4f1e9a52-7c3d-5b8e-9a21-6d0c3e7f8b14";
const ABSENT: &str = "0b7d2f6e-3a91-4c58-8e2d-5f4a1c9b7e30";

#[tokio::test]
async fn a_read_by_an_id_linear_does_not_hold_answers_absent_on_every_read_path() {
    for status in STATUSES {
        for cached in [false, true] {
            let (endpoint, sent) = loopback(move |query, _| match read_kind(query) {
                Some(kind) => (status, not_found(kind)),
                None if query == graphql::RESOLUTION => (200, resolution()),
                None => (400, json!({"errors":[{"message":"unexpected request"}]})),
            });
            let source = source(&endpoint);
            if cached {
                source.attach_cache();
            }
            let at = format!("HTTP {status}, cache attached: {cached}");
            let absent = NativeId::from(ABSENT);
            // The plain read, or the cached one while the cache is attached.
            assert!(source.get_task(&absent).await.unwrap().is_none(), "{at}");
            assert!(source.get_project(&absent).await.unwrap().is_none(), "{at}");
            assert!(
                source.get_document(&absent).await.unwrap().is_none(),
                "{at}"
            );
            let details = source
                .get_task_details(
                    std::slice::from_ref(&absent),
                    Some(&PageRequest {
                        cursor: None,
                        limit: 10,
                    }),
                )
                .await;
            assert!(
                matches!(details.as_slice(), [Ok(None)]),
                "{at}: {details:?}"
            );
            // The look for an earlier attempt's item, by every record's key.
            let target = WriteTarget::New {
                repositories: &[],
                project: None,
            };
            for record in [
                MetadataRecord::Task,
                MetadataRecord::Project,
                MetadataRecord::Document,
            ] {
                assert_eq!(
                    source
                        .created_with_key(record, KEY, None, &target)
                        .await
                        .unwrap(),
                    Vec::<NativeId>::new(),
                    "{at}: {record:?}"
                );
            }
            // The read each narrow write makes of its target first.
            assert_eq!(
                source.set_task_content(&absent, "body").await.unwrap(),
                None,
                "{at}"
            );
            let key = MetadataKey::new("acme.reviewed").unwrap();
            assert!(
                source
                    .set_project_metadata(&absent, &key, &json!(true))
                    .await
                    .unwrap()
                    .is_none(),
                "{at}"
            );
            assert!(
                source
                    .set_document_metadata(&absent, &key, &json!(true))
                    .await
                    .unwrap()
                    .is_none(),
                "{at}"
            );
            let sent = sent.lock().unwrap().clone();
            assert!(
                !sent.iter().any(|query| read_kind(query).is_none()
                    && query != graphql::RESOLUTION
                    && !query.trim_start().starts_with("mutation")),
                "{at}: only reads by id were sent: {sent:#?}"
            );
            assert!(
                !sent
                    .iter()
                    .any(|query| query.trim_start().starts_with("mutation")),
                "{at}: nothing is written to an item that is not there: {sent:#?}"
            );
            // Which documents the reads went out as: the cached spelling exactly while attached.
            for (plain, widened) in [
                (graphql::ISSUE, graphql::ISSUE_CACHED),
                (graphql::PROJECT, graphql::PROJECT_CACHED),
                (graphql::ISSUE_DETAIL, graphql::ISSUE_DETAIL_CACHED),
            ] {
                let expected = if cached { widened } else { plain };
                assert!(
                    sent.iter().any(|query| query == expected),
                    "{at}: the read went out as {expected}: {sent:#?}"
                );
            }
            // A narrow write reads its target unwidened, whether or not the cache is attached.
            assert!(
                sent.iter()
                    .any(|query| query == graphql::ISSUE || query == graphql::ISSUE_RESOLVED),
                "{at}: {sent:#?}"
            );
            assert!(
                sent.iter().any(|query| query == graphql::PROJECT),
                "{at}: {sent:#?}"
            );
            assert!(
                sent.iter().any(|query| query == graphql::DOCUMENT),
                "{at}: {sent:#?}"
            );
        }
    }
}

#[tokio::test]
async fn a_not_found_of_another_kind_and_any_other_error_stay_refusals() {
    // Each read answered as missing something it did not address, or as failing outright.
    for status in STATUSES {
        for (answered, said) in [
            ("other-kind", ""),
            (
                "other-error",
                "Argument Validation Error: the fixture refuses this read",
            ),
        ] {
            let (endpoint, _) = loopback(move |query, _| {
                let Some(kind) = read_kind(query) else {
                    return (200, resolution());
                };
                if answered == "other-error" {
                    return (status, json!({"errors":[{"message":said}],"data":null}));
                }
                let other = if kind == "Issue" { "Project" } else { "Issue" };
                (status, not_found(other))
            });
            let source = source(&endpoint);
            let absent = NativeId::from(ABSENT);
            let at = format!("HTTP {status}, {answered}");
            let refusals = [
                source.get_task(&absent).await.map(|held| held.is_some()),
                source.get_project(&absent).await.map(|held| held.is_some()),
                source
                    .get_document(&absent)
                    .await
                    .map(|held| held.is_some()),
            ];
            for refused in refusals {
                let error = refused.expect_err(&at);
                let message = error.to_string();
                if answered == "other-error" {
                    assert!(message.contains(said), "{at}: {message}");
                } else {
                    assert!(message.contains("Entity not found"), "{at}: {message}");
                }
            }
        }
    }
}

/// The refusal the loopback answers every create with.
const CREATE_REFUSED: &str = "Argument Validation Error: the fixture refuses this create";
/// The unrelated failure the look for an earlier attempt's item meets in its second case.
const LOOKUP_FAILED: &str = "Internal server error: the fixture fails this lookup";

/// `item` carrying the create key a create safe to retry carries.
fn keyed(mut item: Value) -> Value {
    item["metadata"] = json!({ MetadataKey::CREATE_KEY: KEY });
    item
}

#[tokio::test]
async fn a_create_linear_refuses_fails_with_its_own_refusal_whatever_the_lookup_answers() {
    let creates = [
        graphql::ISSUE_CREATE,
        graphql::PROJECT_CREATE,
        graphql::DOCUMENT_CREATE,
    ];
    for status in STATUSES {
        for lookup_fails in [false, true] {
            let (endpoint, sent) = loopback(move |query, _| {
                if creates.contains(&query) {
                    return (
                        status,
                        json!({"errors":[{"message":CREATE_REFUSED}],"data":null}),
                    );
                }
                if query == graphql::RESOLUTION {
                    return (200, resolution());
                }
                if lookup_fails {
                    return (500, json!({"errors":[{"message":LOOKUP_FAILED}]}));
                }
                match read_kind(query) {
                    Some(kind) => (400, not_found(kind)),
                    None => (400, json!({"errors":[{"message":"unexpected request"}]})),
                }
            });
            let source = source(&endpoint);
            let at = format!("create answered HTTP {status}, lookup fails: {lookup_fails}");
            let task: Task =
                serde_json::from_value(keyed(json!({"id":"plan:T","title":"Draft the rollout",
                "content":"body","status":{"category":"todo","name":"Todo"},"labels":[],
                "repositories":[]})))
                .unwrap();
            let project: Project =
                serde_json::from_value(keyed(json!({"id":"plan:P","title":"Harbor rollout",
                "content":"body","status":{"category":"todo","name":"Todo"},"labels":[],
                "repositories":[]})))
                .unwrap();
            let document: Document =
                serde_json::from_value(keyed(json!({"id":"plan:D","title":"Rollout notes",
                "content":"body","labels":[],"repositories":[]})))
                .unwrap();
            let failures: [Result<NativeId, SourceError>; 3] = [
                source
                    .write_task(&ItemWrite {
                        target: None,
                        item: task,
                        depends_on: vec![],
                    })
                    .await,
                source
                    .write_project(&ItemWrite {
                        target: None,
                        item: project,
                        depends_on: vec![],
                    })
                    .await,
                source
                    .write_document(&ItemWrite {
                        target: None,
                        item: document,
                        depends_on: vec![],
                    })
                    .await,
            ];
            for (record, failed) in ["issue", "project", "document"].iter().zip(failures) {
                let message = failed.expect_err(&at).to_string();
                assert!(
                    message.contains(CREATE_REFUSED),
                    "{at}: the {record} create's own refusal is the answer: {message}"
                );
                assert!(
                    !message.contains("Entity not found") && !message.contains(LOOKUP_FAILED),
                    "{at}: nothing the lookup answered stands in for it: {message}"
                );
            }
            let sent = sent.lock().unwrap().clone();
            for create in creates {
                assert_eq!(
                    sent.iter().filter(|query| *query == create).count(),
                    1,
                    "{at}: each create is sent once: {sent:#?}"
                );
            }
            // The lookup did run after each refusal, so each case is what its name says.
            assert!(
                sent.iter()
                    .filter(|query| read_kind(query).is_some())
                    .count()
                    >= 3,
                "{at}: {sent:#?}"
            );
        }
    }
}
