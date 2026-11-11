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
//!
//! The same loopback answers a page resumed from a cursor with Linear's `cursor not found`, as
//! the live capability journey met it: a walk in pages of one reads each such page again, on
//! the source's clock, and reaches every row exactly once and in order; a cursor Linear never
//! recognises is refused naming it, after the bounded rereads.

use onetaskgraph_plugin_api::{
    Document, ItemWrite, MetadataKey, MetadataRecord, NativeId, PageRequest, Project,
    SecretResolver, SourceError, SourceName, SourcePlugin, Task, TaskSource, WriteTarget,
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
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
    spelled_not_found(kind, Spelling::Both)
}

/// Which of Linear's two spellings of a not-found an answer carries: both, as Linear sends it,
/// or one alone, as a rewording of the other would leave it.
#[derive(Clone, Copy, Debug)]
enum Spelling {
    Both,
    MessageOnly,
    PresentableOnly,
}

fn spelled_not_found(kind: &str, spelling: Spelling) -> Value {
    let message = match spelling {
        Spelling::PresentableOnly => "Lookup failed for the requested record".to_owned(),
        _ => format!("Entity not found: {kind}"),
    };
    let mut error = json!({"message": message});
    if !matches!(spelling, Spelling::MessageOnly) {
        error["extensions"] = json!({"type":"invalid input","code":"INPUT_ERROR",
            "statusCode":400,"userError":true,
            "userPresentableMessage":format!("Could not find referenced {kind}.")});
    }
    json!({"errors":[error],"data":null})
}

/// Linear's refusal of a create under an id a record already holds — a deleted one's included —
/// as the live project-create journey observed it on 2026-11-03, under HTTP 200.
fn id_conflict(kind: &str, id: &str) -> Value {
    let record = match kind {
        "Project" => MetadataRecord::Project,
        "Issue" => MetadataRecord::Task,
        _ => MetadataRecord::Document,
    };
    json!({"data":null,"errors":[{"message":graphql::id_conflict_message(record),
        "path":[format!("{}Create", kind.to_ascii_lowercase())],
        "extensions":{"code":"INPUT_ERROR","statusCode":400,"type":"invalid input","userError":true,
            "userPresentableMessage":graphql::id_conflict_presentable(record, id)}}]})
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
async fn either_spelling_of_linears_not_found_alone_reads_as_absent() {
    for spelling in [Spelling::MessageOnly, Spelling::PresentableOnly] {
        for status in STATUSES {
            let (endpoint, _) = loopback(move |query, _| match read_kind(query) {
                Some(kind) => (status, spelled_not_found(kind, spelling)),
                None => (200, resolution()),
            });
            let source = source(&endpoint);
            let absent = NativeId::from(ABSENT);
            let at = format!("{spelling:?}, HTTP {status}");
            assert!(source.get_task(&absent).await.unwrap().is_none(), "{at}");
            assert!(source.get_project(&absent).await.unwrap().is_none(), "{at}");
            assert!(
                source.get_document(&absent).await.unwrap().is_none(),
                "{at}"
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
            // Linear refused each one, so each was tried under every id its key derives, and
            // under none twice — and the refusal reported is still the create's own.
            for create in creates {
                assert_eq!(
                    sent.iter().filter(|query| *query == create).count(),
                    usize::from(onetaskgraph_linear::CREATE_ID_GENERATIONS),
                    "{at}: each create is tried under each generation once: {sent:#?}"
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

/// The ids a loopback's project creates were sent under, in order.
type Sent = Arc<Mutex<Vec<String>>>;

#[tokio::test]
async fn a_keyed_create_whose_id_another_record_holds_lands_under_the_next() {
    // The first id the key derives is held by a record this source cannot read — a project a
    // copy's undo deleted, or one of another workspace — so Linear refuses a create under it.
    let ids = onetaskgraph_linear::linear_ids_of_key(KEY).unwrap();
    assert_eq!(ids[0], onetaskgraph_linear::linear_id_of_key(KEY).unwrap());
    for status in STATUSES {
        let sent: Sent = Arc::default();
        let created: Arc<Mutex<Option<String>>> = Arc::default();
        let (record, held) = (Arc::clone(&sent), Arc::clone(&created));
        let burnt = ids[0].clone();
        let (endpoint, _) = loopback(move |query, variables| {
            if query == graphql::PROJECT_CREATE {
                let id = variables["input"]["id"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
                record.lock().unwrap().push(id.clone());
                if id == burnt {
                    return (status, id_conflict("Project", &id));
                }
                *held.lock().unwrap() = Some(id.clone());
                let project = json!({"id":id,"name":"Harbor rollout","content":variables["input"]["content"],
                    "url":null,"createdAt":null,"updatedAt":null,"archivedAt":null,
                    "status":{"name":"Todo","type":"planned"},"labels":{"nodes":[]},
                    "relations":{"nodes":[],"pageInfo":{"hasNextPage":false}},
                    "inverseRelations":{"nodes":[],"pageInfo":{"hasNextPage":false}}});
                return (
                    200,
                    json!({"data":{"projectCreate":{"success":true,"project":project}}}),
                );
            }
            if query == graphql::RESOLUTION {
                return (200, resolution());
            }
            if query == graphql::PROJECT {
                let asked = variables["id"].as_str().unwrap_or_default();
                if held.lock().unwrap().as_deref() == Some(asked) {
                    let project = json!({"id":asked,"name":"Harbor rollout","content":null,"url":null,
                        "createdAt":null,"updatedAt":null,"archivedAt":null,
                        "status":{"name":"Todo","type":"planned"},"labels":{"nodes":[]}});
                    return (200, json!({"data":{"project":project}}));
                }
                return (400, not_found("Project"));
            }
            (400, json!({"errors":[{"message":"unexpected request"}]}))
        });
        let source = source(&endpoint);
        let project: Project = serde_json::from_value(keyed(json!({"id":"plan:P",
            "title":"Harbor rollout","content":"body","status":{"category":"todo","name":"Todo"},
            "labels":[],"repositories":[]})))
        .unwrap();
        let landed = source
            .write_project(&ItemWrite {
                target: None,
                item: project,
                depends_on: vec![],
            })
            .await
            .unwrap_or_else(|error| panic!("HTTP {status}: the create lands: {error}"));
        assert_eq!(landed.0, ids[1], "HTTP {status}: under the next generation");
        assert_eq!(*sent.lock().unwrap(), ids[..2].to_vec(), "HTTP {status}");
        // A retry's look for it finds the one project, under the generation it landed under.
        let target = WriteTarget::New {
            repositories: &[],
            project: None,
        };
        assert_eq!(
            source
                .created_with_key(MetadataRecord::Project, KEY, None, &target)
                .await
                .unwrap(),
            vec![NativeId(ids[1].clone())],
            "HTTP {status}"
        );
    }
}

#[tokio::test]
async fn a_keyed_create_whose_answer_never_arrived_is_never_sent_again() {
    // Linear may have applied it, so another create under another id could make a second item.
    let sent: Sent = Arc::default();
    let record = Arc::clone(&sent);
    let (endpoint, _) = loopback(move |query, variables| {
        if query == graphql::PROJECT_CREATE {
            record.lock().unwrap().push(
                variables["input"]["id"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
            );
            return (502, json!({"errors":[{"message":"bad gateway"}]}));
        }
        if query == graphql::RESOLUTION {
            return (200, resolution());
        }
        (400, not_found("Project"))
    });
    let project: Project = serde_json::from_value(keyed(json!({"id":"plan:P",
        "title":"Harbor rollout","content":"body","status":{"category":"todo","name":"Todo"},
        "labels":[],"repositories":[]})))
    .unwrap();
    let failed = source(&endpoint)
        .write_project(&ItemWrite {
            target: None,
            item: project,
            depends_on: vec![],
        })
        .await
        .expect_err("an unanswered create is not reported landed");
    assert!(failed.to_string().contains("502"), "{failed}");
    assert_eq!(sent.lock().unwrap().len(), 1, "{:?}", sent.lock().unwrap());
}

/// A project as Linear answers one, under `id`, with empty pages of relations.
fn held_project(id: &str) -> Value {
    let page = json!({"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}});
    json!({"id":id,"name":"Harbor rollout","content":"body","url":null,"createdAt":null,
        "updatedAt":null,"archivedAt":null,"status":{"name":"Todo","type":"planned"},
        "labels":{"nodes":[]},"relations":page,"inverseRelations":page})
}

fn keyed_project() -> Project {
    serde_json::from_value(keyed(json!({"id":"plan:P","title":"Harbor rollout",
        "content":"body","status":{"category":"todo","name":"Todo"},"labels":[],
        "repositories":[]})))
    .unwrap()
}

#[tokio::test]
async fn a_keyed_create_refused_under_an_id_its_earlier_attempt_holds_finishes_that_item() {
    // An earlier attempt's create landed under the key's first id; this one is refused it, and
    // the read of that id finds the project, which this create is: written whole, not again.
    let ids = onetaskgraph_linear::linear_ids_of_key(KEY).unwrap();
    let sent: Arc<Mutex<Vec<(String, Value)>>> = Arc::default();
    let record = Arc::clone(&sent);
    let held = ids[0].clone();
    let (endpoint, _) = loopback(move |query, variables| {
        record
            .lock()
            .unwrap()
            .push((query.to_owned(), variables.clone()));
        match query {
            graphql::PROJECT_CREATE => (
                200,
                id_conflict(
                    "Project",
                    variables["input"]["id"].as_str().unwrap_or_default(),
                ),
            ),
            graphql::RESOLUTION => (200, resolution()),
            graphql::PROJECT if variables["id"] == held.as_str() => {
                (200, json!({"data":{"project":held_project(&held)}}))
            }
            graphql::PROJECT_REWRITE if variables["id"] == held.as_str() => (
                200,
                json!({"data":{"projectUpdate":{"success":true,"project":held_project(&held)}}}),
            ),
            _ => (400, not_found("Project")),
        }
    });
    let landed = source(&endpoint)
        .write_project(&ItemWrite {
            target: None,
            item: keyed_project(),
            depends_on: vec![],
        })
        .await
        .expect("the earlier attempt's project is this create's");
    assert_eq!(landed.0, ids[0]);
    let sent = sent.lock().unwrap().clone();
    let count = |document: &str| sent.iter().filter(|(query, _)| query == document).count();
    assert_eq!(count(graphql::PROJECT_CREATE), 1, "{sent:#?}");
    assert_eq!(count(graphql::PROJECT_REWRITE), 1, "{sent:#?}");
}

/// A write guard refusing every write it is asked about, as one does once another attempt holds
/// the create.
struct Revoked;

#[async_trait::async_trait]
impl onetaskgraph_plugin_api::WriteGuard for Revoked {
    async fn before_write(&self) -> Result<(), SourceError> {
        Err(SourceError::Refused {
            message: "create-in-progress: another attempt holds this create".to_owned(),
        })
    }
}

#[tokio::test]
async fn a_create_whose_hold_passed_to_another_attempt_is_not_sent_under_another_id() {
    let creates: Sent = Arc::default();
    let record = Arc::clone(&creates);
    let (endpoint, _) = loopback(move |query, variables| match query {
        graphql::PROJECT_CREATE => {
            record.lock().unwrap().push(
                variables["input"]["id"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
            );
            (
                200,
                id_conflict(
                    "Project",
                    variables["input"]["id"].as_str().unwrap_or_default(),
                ),
            )
        }
        graphql::RESOLUTION => (200, resolution()),
        _ => (400, not_found("Project")),
    });
    let source = source(&endpoint);
    let failed = <dyn onetaskgraph_plugin_api::WriteGuard>::scope(
        Arc::new(Revoked),
        source.write_project(&ItemWrite {
            target: None,
            item: keyed_project(),
            depends_on: vec![],
        }),
    )
    .await
    .expect_err("a create whose hold has passed sends nothing more");
    assert!(
        failed.to_string().contains("create-in-progress"),
        "the guard's refusal is the answer: {failed}"
    );
    assert_eq!(
        creates.lock().unwrap().len(),
        1,
        "the first id alone was tried: {:?}",
        creates.lock().unwrap()
    );
}

/// What one record's keyed create meets: a deleted or foreign record holding the key's first id,
/// or its own earlier attempt's item there.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Held {
    ByAnother,
    ByEarlierAttempt,
}

#[tokio::test]
async fn an_issue_and_a_document_land_under_the_next_id_or_finish_their_earlier_attempt() {
    let ids = onetaskgraph_linear::linear_ids_of_key(KEY).unwrap();
    for record in [MetadataRecord::Task, MetadataRecord::Document] {
        for case in [Held::ByAnother, Held::ByEarlierAttempt] {
            let at = format!("{record:?}, held {case:?}");
            let (create, read, rewrite, created, updated, field, kind) = match record {
                MetadataRecord::Task => (
                    graphql::ISSUE_CREATE,
                    graphql::ISSUE,
                    graphql::ISSUE_REWRITE,
                    "issueCreate",
                    "issueUpdate",
                    "issue",
                    "Issue",
                ),
                _ => (
                    graphql::DOCUMENT_CREATE,
                    graphql::DOCUMENT,
                    graphql::DOCUMENT_UPDATE,
                    "documentCreate",
                    "documentUpdate",
                    "document",
                    "Document",
                ),
            };
            let item = move |id: &str| match record {
                MetadataRecord::Task => {
                    let page =
                        json!({"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}});
                    json!({"id":id,"identifier":"ENG-1","title":"Draft the rollout",
                        "description":"body","url":null,"createdAt":null,"updatedAt":null,
                        "archivedAt":null,"state":{"name":"Todo","type":"unstarted"},"priority":0,
                        "labels":{"nodes":[]},"project":null,"relations":page})
                }
                _ => json!({"id":id,"title":"Rollout notes","content":"body","url":null,
                    "createdAt":null,"updatedAt":null,"archivedAt":null,"project":null}),
            };
            let held: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(
                (case == Held::ByEarlierAttempt).then(|| ids[0].clone()),
            ));
            let burnt = ids[0].clone();
            let creates: Sent = Arc::default();
            let rewrites: Sent = Arc::default();
            let (record_creates, record_rewrites, holding) = (
                Arc::clone(&creates),
                Arc::clone(&rewrites),
                Arc::clone(&held),
            );
            let (endpoint, _) = loopback(move |query, variables| {
                let mut holding = holding.lock().unwrap();
                if query == create {
                    let id = variables["input"]["id"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned();
                    record_creates.lock().unwrap().push(id.clone());
                    if id == burnt || holding.as_deref() == Some(id.as_str()) {
                        return (200, id_conflict(kind, &id));
                    }
                    *holding = Some(id.clone());
                    return (
                        200,
                        json!({"data":{created:{"success":true,field:item(&id)}}}),
                    );
                }
                let asked = variables["id"].as_str().unwrap_or_default();
                let holds = holding.as_deref() == Some(asked);
                if query == read && holds {
                    return (200, json!({"data":{field:item(asked)}}));
                }
                // A document's earlier attempt is finished by a whole write; an issue's by a
                // compared update of what differs, and its relations read to reconcile them.
                let finishing = query == rewrite
                    || (record == MetadataRecord::Task && query == graphql::ISSUE_UPDATE_READ);
                if finishing && holds {
                    record_rewrites.lock().unwrap().push(asked.to_owned());
                    return (
                        200,
                        json!({"data":{updated:{"success":true,field:item(asked)}}}),
                    );
                }
                if query == graphql::ISSUE_RELATIONS && holds {
                    let page =
                        json!({"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}});
                    return (200, json!({"data":{"issue":{"relations":page}}}));
                }
                if query == graphql::RESOLUTION {
                    return (200, resolution());
                }
                (400, not_found(kind))
            });
            let source = source(&endpoint);
            let landed = match record {
                MetadataRecord::Task => {
                    let task: Task = serde_json::from_value(keyed(json!({"id":"plan:T",
                        "title":"Draft the rollout","content":"body",
                        "status":{"category":"todo","name":"Todo"},"labels":[],
                        "repositories":[]})))
                    .unwrap();
                    source
                        .write_task(&ItemWrite {
                            target: None,
                            item: task,
                            depends_on: vec![],
                        })
                        .await
                }
                _ => {
                    let document: Document = serde_json::from_value(keyed(json!({
                        "id":"plan:D","title":"Rollout notes","content":"body","labels":[],
                        "repositories":[]})))
                    .unwrap();
                    source
                        .write_document(&ItemWrite {
                            target: None,
                            item: document,
                            depends_on: vec![],
                        })
                        .await
                }
            }
            .unwrap_or_else(|error| panic!("{at}: the create lands: {error}"));
            let target = WriteTarget::New {
                repositories: &[],
                project: None,
            };
            let found = source
                .created_with_key(record, KEY, None, &target)
                .await
                .unwrap();
            match case {
                Held::ByAnother => {
                    assert_eq!(landed.0, ids[1], "{at}: under the next id");
                    assert_eq!(*creates.lock().unwrap(), ids[..2].to_vec(), "{at}");
                    assert!(rewrites.lock().unwrap().is_empty(), "{at}");
                }
                Held::ByEarlierAttempt => {
                    assert_eq!(landed.0, ids[0], "{at}: the earlier attempt's item");
                    assert_eq!(*creates.lock().unwrap(), ids[..1].to_vec(), "{at}");
                    assert_eq!(*rewrites.lock().unwrap(), ids[..1].to_vec(), "{at}");
                }
            }
            assert_eq!(found, vec![landed], "{at}: a retry finds that one item");
        }
    }
}

/// One `record` as Linear answers it under `id`: the create document, its payload's root and
/// field, the read document, and the item.
fn answered(
    record: MetadataRecord,
    id: &str,
) -> (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    Value,
) {
    let page = json!({"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}});
    match record {
        MetadataRecord::Task => (
            graphql::ISSUE_CREATE,
            "issueCreate",
            "issue",
            graphql::ISSUE,
            json!({"id":id,"identifier":"ENG-1","title":"Draft the rollout","description":"body",
                "url":null,"createdAt":null,"updatedAt":null,"archivedAt":null,
                "state":{"name":"Todo","type":"unstarted"},"priority":0,"labels":{"nodes":[]},
                "project":null,"relations":page}),
        ),
        MetadataRecord::Project => (
            graphql::PROJECT_CREATE,
            "projectCreate",
            "project",
            graphql::PROJECT,
            held_project(id),
        ),
        MetadataRecord::Document => (
            graphql::DOCUMENT_CREATE,
            "documentCreate",
            "document",
            graphql::DOCUMENT,
            json!({"id":id,"title":"Rollout notes","content":"body","url":null,"createdAt":null,
                "updatedAt":null,"archivedAt":null,"project":null}),
        ),
    }
}

fn kind_of(record: MetadataRecord) -> &'static str {
    match record {
        MetadataRecord::Task => "Issue",
        MetadataRecord::Project => "Project",
        MetadataRecord::Document => "Document",
    }
}

const RECORDS: [MetadataRecord; 3] = [
    MetadataRecord::Task,
    MetadataRecord::Project,
    MetadataRecord::Document,
];

#[tokio::test]
async fn a_retry_finds_every_item_held_under_any_of_its_keys_ids() {
    // Two readable items under one key's ids is a duplicate the engine refuses by name, so a
    // look across the generations answers both rather than the first it meets.
    let ids = onetaskgraph_linear::linear_ids_of_key(KEY).unwrap();
    for record in RECORDS {
        let held = [ids[0].clone(), ids[2].clone()];
        let readable = held.clone();
        let (endpoint, _) = loopback(move |query, variables| {
            let (_, _, field, read, _) = answered(record, "");
            let asked = variables["id"].as_str().unwrap_or_default();
            if query == read && readable.iter().any(|id| id == asked) {
                return (200, json!({"data":{field:answered(record, asked).4}}));
            }
            (400, not_found(kind_of(record)))
        });
        let target = WriteTarget::New {
            repositories: &[],
            project: None,
        };
        let found = source(&endpoint)
            .created_with_key(record, KEY, None, &target)
            .await
            .unwrap();
        assert_eq!(
            found,
            held.iter().cloned().map(NativeId).collect::<Vec<_>>(),
            "{record:?}"
        );
    }
}

#[tokio::test]
async fn a_create_under_a_later_id_answered_as_another_is_refused_for_every_record() {
    let ids = onetaskgraph_linear::linear_ids_of_key(KEY).unwrap();
    for record in RECORDS {
        let burnt = ids[0].clone();
        let (endpoint, _) = loopback(move |query, variables| {
            let (create, root, field, _, _) = answered(record, "");
            if query == create {
                let id = variables["input"]["id"].as_str().unwrap_or_default();
                if id == burnt {
                    return (200, id_conflict(kind_of(record), id));
                }
                // Landed, but under an id the create was not sent with.
                let elsewhere = answered(record, ABSENT).4;
                return (200, json!({"data":{root:{"success":true,field:elsewhere}}}));
            }
            if query == graphql::RESOLUTION {
                return (200, resolution());
            }
            (400, not_found(kind_of(record)))
        });
        let source = source(&endpoint);
        let failed = match record {
            MetadataRecord::Task => {
                let task: Task = serde_json::from_value(keyed(json!({"id":"plan:T",
                    "title":"Draft the rollout","content":"body",
                    "status":{"category":"todo","name":"Todo"},"labels":[],"repositories":[]})))
                .unwrap();
                source
                    .write_task(&ItemWrite {
                        target: None,
                        item: task,
                        depends_on: vec![],
                    })
                    .await
            }
            MetadataRecord::Project => {
                source
                    .write_project(&ItemWrite {
                        target: None,
                        item: keyed_project(),
                        depends_on: vec![],
                    })
                    .await
            }
            MetadataRecord::Document => {
                let document: Document = serde_json::from_value(keyed(json!({"id":"plan:D",
                    "title":"Rollout notes","content":"body","labels":[],"repositories":[]})))
                .unwrap();
                source
                    .write_document(&ItemWrite {
                        target: None,
                        item: document,
                        depends_on: vec![],
                    })
                    .await
            }
        }
        .expect_err("an item Linear filed under another id is not this create's");
        let message = failed.to_string();
        assert!(
            message.contains(&ids[1]) && message.contains(ABSENT) && message.contains("not the id"),
            "{record:?}: names the id it was sent and the one it was answered: {message}"
        );
    }
}

#[tokio::test]
async fn an_update_of_a_status_and_a_title_reads_an_absent_issue_as_absent() {
    // With nothing of the team's resolution held, the update reads the issue beside it in one
    // request, [`graphql::ISSUE_RESOLVED`]; Linear's not-found for that issue is no such task.
    for status in STATUSES {
        let (endpoint, sent) = loopback(move |query, _| match read_kind(query) {
            Some(kind) => (status, not_found(kind)),
            None => (400, json!({"errors":[{"message":"unexpected request"}]})),
        });
        let update = onetaskgraph_plugin_api::TaskUpdate {
            title: Some("Draft the rollout again".to_owned()),
            status: Some(serde_json::from_value(json!({"category":"todo","name":"Todo"})).unwrap()),
            ..onetaskgraph_plugin_api::TaskUpdate::default()
        };
        let updated = source(&endpoint)
            .update_task(&NativeId::from(ABSENT), &update)
            .await
            .unwrap_or_else(|error| panic!("HTTP {status}: {error}"));
        assert!(updated.is_none(), "HTTP {status}: {updated:?}");
        let sent = sent.lock().unwrap().clone();
        assert_eq!(
            sent,
            vec![graphql::ISSUE_RESOLVED.to_owned()],
            "HTTP {status}"
        );
    }
}

/// Linear's refusal of a page asked for after (or before) `cursor`, as the live capability
/// journey met it, in `spelling`.
fn cursor_not_found(cursor: &str, spelling: Spelling) -> Value {
    let message = match spelling {
        Spelling::PresentableOnly => "Lookup failed for the requested page".to_owned(),
        _ => graphql::CURSOR_NOT_FOUND_MESSAGE.to_owned(),
    };
    let mut error = json!({"message": message});
    if !matches!(spelling, Spelling::MessageOnly) {
        error["extensions"] = json!({"code":"INPUT_ERROR","statusCode":400,
            "type":"invalid input","userError":true,
            "userPresentableMessage":
                format!("{} '{cursor}'.", graphql::CURSOR_NOT_FOUND_PRESENTABLE)});
    }
    json!({"data":null,"errors":[error]})
}

/// A clock whose every wait completes at once, keeping each one and its cause.
#[derive(Default)]
struct Waits(Mutex<Vec<(std::time::Duration, String)>>);

impl Waits {
    fn taken(&self) -> Vec<(std::time::Duration, String)> {
        self.0.lock().unwrap().clone()
    }
}

impl onetaskgraph_plugin_api::Clock for Waits {
    fn now(&self) -> std::time::Duration {
        self.0.lock().unwrap().iter().map(|(wait, _)| *wait).sum()
    }
    fn sleep(
        &self,
        duration: std::time::Duration,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>> {
        self.0.lock().unwrap().push((duration, String::new()));
        Box::pin(std::future::ready(()))
    }
    fn sleep_because(
        &self,
        duration: std::time::Duration,
        cause: onetaskgraph_plugin_api::WaitCause,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>> {
        self.0
            .lock()
            .unwrap()
            .push((duration, format!("{cause:?}")));
        Box::pin(std::future::ready(()))
    }
}

/// The waits a page refused `times` times for its cursor takes before it is answered: each on
/// the source's clock, under its cause, doubling from half a second.
fn rereads(times: u32) -> Vec<(std::time::Duration, String)> {
    (0..times)
        .map(|n| {
            (
                std::time::Duration::from_millis(500) * 2_u32.pow(n),
                "WaitCause(\"linear-cursor-lag\")".to_owned(),
            )
        })
        .collect()
}

/// What the loopback answers the `nth` request (from 0) for a page named by `cursor` (`None` for
/// a first page) before it answers the page itself, if anything.
type Interference = dyn Fn(Option<&str>, usize) -> Option<Answer> + Send + Sync;

/// A page request as the loopback counts it: its document and the cursor it names, if any.
type PageAsked = (String, Option<String>);

/// A loopback holding three issues, each with three comments, listing one issue per page after
/// a cursor and one comment per page before one, as Linear does — each cursor the id of the row
/// the page before ended on — and asking `interfere` first about every page request.
fn workspace(interfere: Box<Interference>) -> (String, Arc<Mutex<Vec<String>>>) {
    let asked: Arc<Mutex<HashMap<PageAsked, usize>>> = Arc::default();
    loopback(move |query, variables| {
        if query == graphql::RESOLUTION {
            return (200, resolution());
        }
        let issues = query == graphql::ISSUES;
        if !issues && query != graphql::ISSUE_COMMENTS {
            return (400, json!({"errors":[{"message":"unexpected request"}]}));
        }
        let cursor = variables[if issues { "after" } else { "before" }].as_str();
        let nth = {
            let mut asked = asked.lock().unwrap();
            let count = asked
                .entry((query.to_owned(), cursor.map(str::to_owned)))
                .or_default();
            *count += 1;
            *count - 1
        };
        if let Some(answer) = interfere(cursor, nth) {
            return answer;
        }
        if issues {
            let ids = ["I-1", "I-2", "I-3"];
            let from = cursor.map_or(0, |cursor| {
                ids.iter()
                    .position(|id| *id == cursor)
                    .expect("an issued cursor")
                    + 1
            });
            let nodes: Vec<Value> = ids[from..]
                .iter()
                .take(variables["first"].as_u64().unwrap_or(50) as usize)
                .map(|id| {
                    json!({"id":id,"identifier":format!("ENG-{id}"),"title":format!("Issue {id}"),
                        "description":null,"url":null,"createdAt":null,"updatedAt":null,
                        "project":null,"state":{"name":"Todo","type":"unstarted"},"priority":0,
                        "labels":{"nodes":[]}})
                })
                .collect();
            let more = from + nodes.len() < ids.len();
            let last = nodes.last().map(|node| node["id"].clone());
            return (
                200,
                json!({"data":{"issues":{"nodes":nodes,
                    "pageInfo":{"hasNextPage":more,"endCursor":last}}}}),
            );
        }
        // Oldest first as Linear holds them; a page is the `last` rows before the cursor.
        let ids = ["C-1", "C-2", "C-3"];
        let to = cursor.map_or(ids.len(), |cursor| {
            ids.iter()
                .position(|id| *id == cursor)
                .expect("an issued cursor")
        });
        let last = variables["last"].as_u64().unwrap_or(50) as usize;
        let from = to.saturating_sub(last);
        let nodes: Vec<Value> = ids[from..to]
            .iter()
            .enumerate()
            .map(|(at, id)| {
                let time = format!("2026-11-11T00:00:0{}Z", from + at);
                json!({"id":id,"body":format!("comment {id}"),"url":null,"createdAt":time,
                    "updatedAt":time,"user":{"displayName":"Ada"}})
            })
            .collect();
        let first = nodes.first().map(|node| node["id"].clone());
        (
            200,
            json!({"data":{"issue":{"archivedAt":null,"comments":{"nodes":nodes,
                "pageInfo":{"hasPreviousPage":from > 0,"startCursor":first}}}}}),
        )
    })
}

fn source_on(endpoint: &str, clock: Arc<Waits>) -> Box<dyn TaskSource> {
    onetaskgraph_linear::Plugin
        .build_with_clock(
            &SourceName::new("work").unwrap(),
            &json!({"endpoint":endpoint,"team":"ENG",
                "status_mapping":{"todo":"Todo","in-progress":"In Progress","done":"Done"}}),
            &Secrets,
            clock,
        )
        .unwrap()
}

/// Every issue's id, walking the source's task listing in pages of one from its first page.
async fn walk_issues(source: &dyn TaskSource) -> Result<Vec<String>, SourceError> {
    let (mut walked, mut cursor) = (Vec::new(), None);
    loop {
        let page = source
            .query_tasks(
                &onetaskgraph_plugin_api::TaskQuery::default(),
                &PageRequest { cursor, limit: 1 },
            )
            .await?;
        assert!(page.items.len() <= 1, "a page of one");
        walked.extend(page.items.into_iter().map(|task| task.id.0));
        match page.next {
            Some(next) => cursor = Some(next),
            None => return Ok(walked),
        }
    }
}

/// Every comment's id of issue I-1, walking its comments in pages of one from the newest page.
async fn walk_comments(source: &dyn TaskSource) -> Result<Vec<String>, SourceError> {
    let (mut walked, mut cursor) = (Vec::new(), None);
    loop {
        let page = source
            .task_comments(&NativeId::from("I-1"), &PageRequest { cursor, limit: 1 })
            .await?
            .expect("the issue is held");
        assert!(page.items.len() <= 1, "a page of one");
        walked.extend(page.items.into_iter().map(|comment| comment.id.0));
        match page.next {
            Some(next) => cursor = Some(next),
            None => return Ok(walked),
        }
    }
}

/// How many requests of `document` the loopback was sent.
fn sent_of(sent: &Mutex<Vec<String>>, document: &str) -> usize {
    sent.lock()
        .unwrap()
        .iter()
        .filter(|query| *query == document)
        .count()
}

/// A resumed page refused twice for its cursor, in any of its spellings and under either
/// status, and then answered.
fn lags_twice(status: u16, spelling: Spelling) -> Box<Interference> {
    Box::new(move |cursor, nth| {
        cursor
            .filter(|_| nth < 2)
            .map(|cursor| (status, cursor_not_found(cursor, spelling)))
    })
}

#[tokio::test]
async fn a_walk_whose_cursor_linear_cannot_place_yet_reads_that_page_again_and_reaches_every_row_once()
 {
    for status in STATUSES {
        for spelling in [
            Spelling::Both,
            Spelling::MessageOnly,
            Spelling::PresentableOnly,
        ] {
            let at = format!("HTTP {status}, spelled {spelling:?}");
            let (endpoint, sent) = workspace(lags_twice(status, spelling));
            let clock = Arc::new(Waits::default());
            let source = source_on(&endpoint, Arc::clone(&clock));
            let walked = walk_issues(source.as_ref())
                .await
                .unwrap_or_else(|error| panic!("{at}: the walk failed: {error}"));
            assert_eq!(walked, ["I-1", "I-2", "I-3"], "{at}");
            // The first page, and two resumed pages each refused twice and then answered.
            assert_eq!(sent_of(&sent, graphql::ISSUES), 1 + 2 * 3, "{at}");
            assert_eq!(clock.taken(), [rereads(2), rereads(2)].concat(), "{at}");
        }
    }
}

#[tokio::test]
async fn a_comment_walk_before_a_cursor_linear_cannot_place_yet_reads_that_page_again() {
    for status in STATUSES {
        let (endpoint, _) = workspace(Box::new(|_, _| None));
        let unhindered = walk_comments(source_on(&endpoint, Arc::default()).as_ref())
            .await
            .expect("the walk with no refusal");
        assert_eq!(unhindered.len(), 3, "every comment, once");

        let (endpoint, sent) = workspace(lags_twice(status, Spelling::Both));
        let clock = Arc::new(Waits::default());
        let walked = walk_comments(source_on(&endpoint, Arc::clone(&clock)).as_ref())
            .await
            .unwrap_or_else(|error| panic!("HTTP {status}: the walk failed: {error}"));
        assert_eq!(walked, unhindered, "HTTP {status}");
        assert_eq!(
            sent_of(&sent, graphql::ISSUE_COMMENTS),
            1 + 2 * 3,
            "HTTP {status}"
        );
        assert_eq!(
            clock.taken(),
            [rereads(2), rereads(2)].concat(),
            "HTTP {status}"
        );
    }
}

#[tokio::test]
async fn a_cursor_linear_never_places_is_refused_naming_it_after_the_bounded_rereads() {
    for status in STATUSES {
        let (endpoint, sent) = workspace(Box::new(move |cursor, _| {
            cursor.map(|cursor| (status, cursor_not_found(cursor, Spelling::Both)))
        }));
        let clock = Arc::new(Waits::default());
        let refused = walk_issues(source_on(&endpoint, Arc::clone(&clock)).as_ref())
            .await
            .expect_err("a cursor Linear never places is refused");
        let SourceError::Refused { message } = &refused else {
            panic!("HTTP {status}: refused as {refused:?}");
        };
        assert!(
            message.contains("on 5 reads of it")
                && message.contains("Could not find cursor 'I-1'")
                && message.ends_with("start the walk again from its first page"),
            "HTTP {status}: {message}"
        );
        assert_eq!(sent_of(&sent, graphql::ISSUES), 1 + 5, "HTTP {status}");
        assert_eq!(clock.taken(), rereads(4), "HTTP {status}");
    }
}

#[tokio::test]
async fn only_a_resumed_page_is_read_again_and_any_other_failure_ends_the_rereads_as_itself() {
    // A first page names no cursor, so a `cursor not found` there is Linear's refusal as it
    // stands, read once.
    let (endpoint, sent) = workspace(Box::new(|cursor, _| {
        cursor
            .is_none()
            .then(|| (200, cursor_not_found("none", Spelling::Both)))
    }));
    let clock = Arc::new(Waits::default());
    let refused = walk_issues(source_on(&endpoint, Arc::clone(&clock)).as_ref())
        .await
        .expect_err("the first page is refused");
    assert!(
        matches!(&refused, SourceError::Refused { message } if message.starts_with("cursor not found")),
        "{refused:?}"
    );
    assert_eq!(sent_of(&sent, graphql::ISSUES), 1);
    assert!(clock.taken().is_empty(), "a first page is never waited for");

    // A resumed page refused once for its cursor and then for something else, or failing to
    // arrive at all, ends with that failure: nothing after it is read again.
    let validation = json!({"data":null,"errors":[{"message":"Argument Validation Error",
        "extensions":{"code":"INPUT_ERROR","userPresentableMessage":"first must be positive"}}]});
    for (then, expected) in [
        ((200, validation), "a refusal of its own"),
        (
            (500, json!({"errors":[{"message":"internal"}]})),
            "an unavailable Linear",
        ),
    ] {
        let then = Arc::new(then);
        let (endpoint, sent) = workspace(Box::new(move |cursor, nth| {
            let cursor = cursor?;
            Some(if nth == 0 {
                (200, cursor_not_found(cursor, Spelling::Both))
            } else {
                (*then).clone()
            })
        }));
        let clock = Arc::new(Waits::default());
        let failed = walk_issues(source_on(&endpoint, Arc::clone(&clock)).as_ref())
            .await
            .expect_err(expected);
        match expected {
            "a refusal of its own" => assert!(
                matches!(&failed, SourceError::Refused { message }
                    if message.starts_with("Argument Validation Error: first must be positive")),
                "{failed:?}"
            ),
            _ => assert!(
                matches!(&failed, SourceError::Unavailable { message } if message.contains("HTTP 500")),
                "{failed:?}"
            ),
        }
        assert_eq!(sent_of(&sent, graphql::ISSUES), 1 + 2, "{expected}");
        assert_eq!(clock.taken(), rereads(1), "{expected}");
    }
}
