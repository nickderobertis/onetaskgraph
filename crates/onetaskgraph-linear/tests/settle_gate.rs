//! The live journey's exact-set reads, against a loopback workspace whose index lags.
//!
//! The reads and the wait are the real ones — `settle::settled_tasks`,
//! `settle::settled_documents` and `settle::settled_walk`, the same code `tests/live.rs`
//! compares Linear's listings through — and they reach that workspace through the real plugin
//! over real HTTP. What stands in for Linear is a local server that answers a filtered listing
//! from an index that has not caught up with a write for a number of reads, or never does.
//! `settle::settled_label` is driven the same way, sending the plugin's own label lookup to a
//! workspace whose label filter holds a just-created label late, never, or twice. A missing
//! label is retried as index lag; duplicate data is refused immediately with its ids.
//!
//! No credential and no third-party API: every issue and document below is one this file
//! answered with. A short bound stands in for [`settle::LINEAR_INDEX`], because what is proven
//! is what happens when a bound is spent, and nothing about that depends on its length.

use std::io::{Read, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use onetaskgraph_plugin_api::{
    DocumentQuery, NativeId, ProjectFilter, SecretResolver, SourceName, SourcePlugin, TaskQuery,
    TaskSource,
};
use secrecy::SecretString;
use serde_json::{Value, json};

// `LINEAR_INDEX` is the live journey's bound, which these drives replace with a short one.
#[allow(dead_code)]
mod settle;

use settle::{
    Bound, DocumentListingBudget, LABEL_CONNECTION, LABEL_VARIABLE, settled_document_absent,
    settled_documents, settled_label, settled_tasks, settled_walk,
};

/// Room for the late listings below, which agree on their third read, with reads to spare, so
/// that a pass is the listing catching up rather than the bound running out on the last read.
const BOUND: Bound = Bound {
    reads: 5,
    interval: Duration::from_millis(50),
};

const DOCUMENT_LISTING: DocumentListingBudget = DocumentListingBudget {
    pages: 10,
    elapsed: Duration::from_secs(5),
};

/// How late past the bound a failing wait may still report, for a loaded machine.
const SLACK: Duration = Duration::from_secs(10);

struct Key;
impl SecretResolver for Key {
    fn get(&self, _: &str) -> Option<SecretString> {
        Some("test-key".into())
    }
}

/// The plugin reaching a workspace answering the `n`th request, counted from one, with
/// `answer(n, request)`, and how many requests it has answered.
fn workspace(
    answer: impl Fn(u32, &str) -> Value + Send + 'static,
) -> (Box<dyn TaskSource>, Arc<AtomicU32>) {
    let (url, answered) = serve(answer);
    let source = onetaskgraph_linear::Plugin
        .build(
            &SourceName::new("live").unwrap(),
            &json!({"endpoint": url}),
            &Key,
        )
        .unwrap();
    (source, answered)
}

/// [`workspace`] without the plugin in front of it, for the label wait: that wait is handed a
/// way to send the plugin's lookup rather than a source, so it is pointed at the URL itself.
fn serve(answer: impl Fn(u32, &str) -> Value + Send + 'static) -> (String, Arc<AtomicU32>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/graphql", listener.local_addr().unwrap());
    let answered = Arc::new(AtomicU32::new(0));
    let counter = Arc::clone(&answered);
    thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let request = request(&mut stream);
            let n = counter.fetch_add(1, Ordering::SeqCst) + 1;
            let body = json!({"data": answer(n, &request)}).to_string();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
                 Connection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        }
    });
    (url, answered)
}

/// One GraphQL request to `url`, sent as the live journey's `linear` sends one: the answer's
/// `data`, or an `Err` naming what came back instead.
async fn post(url: &str, query: &str, variables: Value) -> Result<Value, String> {
    let body = reqwest::Client::new()
        .post(url)
        .json(&json!({"query": query, "variables": variables}))
        .send()
        .await
        .map_err(|error| format!("the workspace could not be reached: {error}"))?
        .json::<Value>()
        .await
        .map_err(|error| format!("the workspace answered invalid JSON: {error}"))?;
    body.get("data")
        .cloned()
        .ok_or_else(|| format!("the workspace answered no data: {body}"))
}

/// The most a request to this workspace may be; the plugin's listings are a few hundred bytes.
const MOST_A_REQUEST_IS: usize = 64 * 1024;

/// One whole HTTP request: its headers, and exactly the body their `Content-Length` declares.
///
/// Anything else — a request past [`MOST_A_REQUEST_IS`], whether read or declared, one with no
/// `Content-Length` or an unreadable one, one that is not UTF-8, or a connection closed short of
/// what it declared — is not a request the plugin sends, so it fails the drive naming what
/// arrived rather than being answered.
fn request(stream: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut chunk = [0; 8192];
    loop {
        let n = stream.read(&mut chunk).unwrap();
        bytes.extend_from_slice(&chunk[..n]);
        assert!(
            bytes.len() <= MOST_A_REQUEST_IS,
            "the plugin sent a request of more than {MOST_A_REQUEST_IS} bytes"
        );
        if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            let headers = std::str::from_utf8(&bytes[..end]).unwrap_or_else(|error| {
                panic!("the plugin sent headers that are not UTF-8: {error}")
            });
            let declared = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>())
                })
                .unwrap_or_else(|| {
                    panic!("the plugin sent a request with no Content-Length: {headers}")
                })
                .unwrap_or_else(|error| {
                    panic!("the plugin sent an unreadable Content-Length ({error}): {headers}")
                });
            // `end + 4` is at most what was read, which the ceiling already holds, so this
            // subtraction cannot underflow and the sum below cannot overflow.
            assert!(
                declared <= MOST_A_REQUEST_IS - (end + 4),
                "the plugin declared a body of {declared} bytes, past {MOST_A_REQUEST_IS}: {headers}"
            );
            let whole = end + 4 + declared;
            if bytes.len() >= whole {
                bytes.truncate(whole);
                return String::from_utf8(bytes).unwrap_or_else(|error| {
                    panic!("the plugin sent a request that is not UTF-8: {error}")
                });
            }
        }
        assert!(
            n > 0,
            "the connection closed partway through a request, after {} bytes",
            bytes.len()
        );
    }
}

fn issue(id: &str, title: &str, project: &str) -> Value {
    json!({
        "id": id, "title": title, "description": null, "url": null,
        "createdAt": "2026-09-15T12:00:00Z", "updatedAt": "2026-09-15T12:00:00Z",
        "state": {"name": "Todo", "type": "unstarted"}, "labels": {"nodes": []},
        "project": {"id": project}
    })
}

fn document(id: &str, title: &str, project: Option<&str>) -> Value {
    json!({
        "id": id, "title": title, "content": null, "url": null,
        "createdAt": "2026-09-15T12:00:00Z", "updatedAt": "2026-09-15T12:00:00Z",
        "project": project.map(|id| json!({"id": id}))
    })
}

fn issues(nodes: Vec<Value>) -> Value {
    json!({"issues": {"nodes": nodes, "pageInfo": {"hasNextPage": false, "endCursor": null}}})
}

/// A page of issues with another after it, at `cursor`.
fn issues_then(nodes: Vec<Value>, cursor: &str) -> Value {
    json!({"issues": {"nodes": nodes, "pageInfo": {"hasNextPage": true, "endCursor": cursor}}})
}

fn documents(nodes: Vec<Value>) -> Value {
    json!({"documents": {"nodes": nodes, "pageInfo": {"hasNextPage": false, "endCursor": null}}})
}

/// A page of documents with another after it, at `cursor`.
fn documents_then(nodes: Vec<Value>, cursor: &str) -> Value {
    json!({"documents": {"nodes": nodes, "pageInfo": {"hasNextPage": true, "endCursor": cursor}}})
}

/// The GraphQL `variables` object of one whole request [`request`] read.
///
/// Parsed rather than searched, so a body that is not the JSON document the plugin sends, or
/// one without a `variables` object, fails the drive naming what arrived.
fn variables(request: &str) -> serde_json::Map<String, Value> {
    let (_, body) = request
        .split_once("\r\n\r\n")
        .unwrap_or_else(|| panic!("the plugin sent a request with no body: {request}"));
    let mut document = serde_json::from_str::<Value>(body).unwrap_or_else(|error| {
        panic!("the plugin sent a body that is not JSON ({error}): {body}")
    });
    match document.get_mut("variables").map(Value::take) {
        Some(Value::Object(variables)) => variables,
        _ => panic!("the plugin sent a body without a variables object: {body}"),
    }
}

/// Whether `request` asks for the page after the cursor `c1`.
fn after_c1(request: &str) -> bool {
    variables(request).get("after") == Some(&json!("c1"))
}

fn under(project: &str) -> TaskQuery {
    TaskQuery {
        project: ProjectFilter::Is(NativeId(project.into())),
        ..TaskQuery::default()
    }
}

fn filed_under(project: &str) -> DocumentQuery {
    DocumentQuery {
        project: ProjectFilter::Is(NativeId(project.into())),
        ..DocumentQuery::default()
    }
}

fn expected(title: &str) -> Vec<String> {
    vec![title.to_owned()]
}

fn expected_document(id: &str, title: &str) -> Vec<(NativeId, String)> {
    vec![(NativeId(id.into()), title.to_owned())]
}

/// The label a run has just created, by the name a write resolves it through.
const LABEL: &str = "otg-live-label";

fn labels(ids: Vec<&str>) -> Value {
    json!({LABEL_CONNECTION: {"nodes": ids.into_iter().map(|id| json!({"id": id})).collect::<Vec<_>>()}})
}

/// Refuses `request` unless it is the plugin's own label lookup, asking for [`LABEL`].
fn assert_looks_up_label(request: &str) {
    let (_, body) = request
        .split_once("\r\n\r\n")
        .unwrap_or_else(|| panic!("the wait sent a request with no body: {request}"));
    let document = serde_json::from_str::<Value>(body)
        .unwrap_or_else(|error| panic!("the wait sent a body that is not JSON ({error}): {body}"));
    assert_eq!(
        document.get("query"),
        Some(&json!(onetaskgraph_linear::graphql::ISSUE_LABEL)),
        "the wait asks the lookup a write resolves a label through: {body}"
    );
    assert_eq!(
        variables(request),
        serde_json::Map::from_iter([(LABEL_VARIABLE.to_owned(), json!(LABEL))]),
        "the wait asks for the label by its name and nothing else: {body}"
    );
}

#[tokio::test]
async fn a_task_listing_the_index_catches_up_with_late_passes_without_reading_again() {
    // Filed under `p1` by a write the project filter does not answer for its first two reads.
    let (source, answered) = workspace(|n, request| {
        assert!(
            request.contains("issues("),
            "only issues are asked for: {request}"
        );
        issues(if n > 2 {
            vec![issue("i1", "filed", "p1")]
        } else {
            vec![]
        })
    });
    settled_tasks(
        BOUND,
        source.as_ref(),
        &under("p1"),
        "the issues of this run's project",
        &expected("filed"),
    )
    .await
    .unwrap();
    assert_eq!(
        answered.load(Ordering::SeqCst),
        3,
        "the wait reads until the listing agrees, and not once more"
    );
}

#[tokio::test]
async fn a_task_listing_whose_filter_is_wrong_fails_within_the_bound_naming_what_it_returned() {
    // A project filter ignored rather than lagging: the listing never narrows to `p1`.
    let (source, answered) = workspace(|_, _| {
        issues(vec![
            issue("i1", "filed", "p1"),
            issue("i2", "elsewhere", "p2"),
        ])
    });
    let started = Instant::now();
    let refusal = settled_tasks(
        BOUND,
        source.as_ref(),
        &under("p1"),
        "the issues of this run's project",
        &expected("filed"),
    )
    .await
    .unwrap_err();
    let waited = started.elapsed();
    assert!(
        refusal.starts_with(
            r#"the issues of this run's project came back as ["elsewhere", "filed"] rather than ["filed"], still after 5 reads"#
        ),
        "the failure names what the listing returned: {refusal}"
    );
    assert_eq!(answered.load(Ordering::SeqCst), BOUND.reads);
    assert!(
        waited >= BOUND.interval * (BOUND.reads - 1)
            && waited < BOUND.interval * BOUND.reads + SLACK,
        "a wrong listing fails once the bound is spent, and waited {waited:?}"
    );
}

#[tokio::test]
async fn a_document_listing_the_index_catches_up_with_late_passes_without_reading_again() {
    // The failure this wait was extended for: the unfiltered listing holds both documents
    // while the one narrowed to their project does not yet hold the document filed there.
    let (source, answered) = workspace(|n, request| {
        assert!(
            request.contains("documents("),
            "only documents are asked for: {request}"
        );
        let narrowed = request.contains(r#""project":{"id":{"eq":"p1"}}"#);
        documents(if narrowed && n <= 3 {
            vec![]
        } else if narrowed {
            vec![document("d1", "filed", Some("p1"))]
        } else {
            vec![
                document("d1", "filed", Some("p1")),
                document("d2", "loose", None),
            ]
        })
    });
    settled_documents(
        BOUND,
        DOCUMENT_LISTING,
        source.as_ref(),
        &DocumentQuery::default(),
        &[NativeId("d1".into()), NativeId("d2".into())],
        "the two documents this run created",
        &[
            (NativeId("d2".into()), "loose".to_owned()),
            (NativeId("d1".into()), "filed".to_owned()),
        ],
    )
    .await
    .unwrap();
    settled_documents(
        BOUND,
        DOCUMENT_LISTING,
        source.as_ref(),
        &filed_under("p1"),
        &[NativeId("d1".into()), NativeId("d2".into())],
        "a document listing narrowed to this run's project",
        &expected_document("d1", "filed"),
    )
    .await
    .unwrap();
    assert_eq!(
        answered.load(Ordering::SeqCst),
        4,
        "one read of the whole listing, then the narrowed one until it agrees"
    );
}

#[tokio::test]
async fn a_document_listing_that_never_catches_up_fails_within_the_bound_naming_what_it_returned() {
    // Everything but this run's document, for ever, and one document the caller does not keep.
    let (source, answered) = workspace(|_, _| {
        documents(vec![
            document("d2", "loose", None),
            document("d3", "another run's", Some("p1")),
        ])
    });
    let started = Instant::now();
    let refusal = settled_documents(
        BOUND,
        DOCUMENT_LISTING,
        source.as_ref(),
        &filed_under("p1"),
        &[NativeId("d1".into())],
        "a document listing narrowed to this run's project",
        &expected_document("d1", "filed"),
    )
    .await
    .unwrap_err();
    let waited = started.elapsed();
    assert!(
        refusal.starts_with(
            r#"a document listing narrowed to this run's project completed its listing but the document ids ["d1"] were missing, still after 5 reads"#
        ),
        "the failure names what the listing returned: {refusal}"
    );
    assert_eq!(answered.load(Ordering::SeqCst), BOUND.reads);
    assert!(
        waited >= BOUND.interval * (BOUND.reads - 1)
            && waited < BOUND.interval * BOUND.reads + SLACK,
        "a listing that never catches up fails once the bound is spent, and waited {waited:?}"
    );
}

#[tokio::test]
async fn this_runs_document_does_not_pass_an_empty_expectation() {
    let (source, answered) = workspace(|_, _| documents(vec![document("d1", "filed", None)]));
    let refusal = settled_documents(
        Bound {
            reads: 1,
            interval: Duration::ZERO,
        },
        DOCUMENT_LISTING,
        source.as_ref(),
        &DocumentQuery::default(),
        &[NativeId("d1".into())],
        "a document listing demanding a label",
        &[],
    )
    .await
    .unwrap_err();
    assert!(
        refusal
            .contains(r#"completed its listing with [(NativeId("d1"), "filed")] rather than []"#),
        "the unexpected document is reported: {refusal}"
    );
    assert_eq!(answered.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_deleted_document_that_stays_readable_for_some_reads_then_disappears_passes() {
    let (source, answered) = workspace(|n, request| {
        assert!(
            request.contains("document(id:"),
            "only one document is asked for: {request}"
        );
        json!({"document": (n <= 2).then(|| document("d1", "deleted", None))})
    });
    settled_document_absent(BOUND, source.as_ref(), &NativeId("d1".into()), "deleted")
        .await
        .unwrap();
    assert_eq!(
        answered.load(Ordering::SeqCst),
        3,
        "the wait reads until the deleted document disappears, and not once more"
    );
}

#[tokio::test]
async fn a_deleted_document_that_never_disappears_fails_at_the_bound_naming_it() {
    let (source, answered) = workspace(|_, _| json!({"document": document("d1", "deleted", None)}));
    let started = Instant::now();
    let refusal =
        settled_document_absent(BOUND, source.as_ref(), &NativeId("d1".into()), "deleted")
            .await
            .unwrap_err();
    let waited = started.elapsed();
    assert!(
        refusal.starts_with(
            r#"the deleted document "deleted" came back as ["deleted"] rather than [], still after 5 reads"#
        ),
        "the failure names the document that remained readable: {refusal}"
    );
    assert_eq!(answered.load(Ordering::SeqCst), BOUND.reads);
    assert!(
        waited >= BOUND.interval * (BOUND.reads - 1)
            && waited < BOUND.interval * BOUND.reads + SLACK,
        "a document that never disappears fails once the bound is spent, and waited {waited:?}"
    );
}

#[tokio::test]
async fn unfiltered_and_orphan_document_listings_read_past_other_documents() {
    // A workspace holding a whole page of documents that are not this run's before its own:
    // a listing that stopped at the first page would never see `filed`, however long it waited.
    let (source, answered) = workspace(|_, request| {
        if after_c1(request) {
            documents(vec![document("d1", "filed", None)])
        } else {
            documents_then(
                (0..onetaskgraph_linear::MAX_PAGE_SIZE)
                    .map(|n| document(&format!("o{n}"), &format!("another run's {n}"), None))
                    .collect(),
                "c1",
            )
        }
    });
    settled_documents(
        BOUND,
        DOCUMENT_LISTING,
        source.as_ref(),
        &DocumentQuery::default(),
        &[NativeId("d1".into())],
        "the documents this run created",
        &expected_document("d1", "filed"),
    )
    .await
    .unwrap();
    settled_documents(
        BOUND,
        DOCUMENT_LISTING,
        source.as_ref(),
        &DocumentQuery {
            project: ProjectFilter::Orphans,
            ..DocumentQuery::default()
        },
        &[NativeId("d1".into())],
        "the orphan documents this run created",
        &expected_document("d1", "filed"),
    )
    .await
    .unwrap();
    assert_eq!(
        answered.load(Ordering::SeqCst),
        4,
        "each of the unfiltered and orphan reads walks both pages, and agrees on them"
    );
}

#[tokio::test]
async fn a_walk_in_pages_of_one_the_index_catches_up_with_late_passes_without_walking_again() {
    // The first walk finds the index empty; every walk after it reaches both issues, one a page.
    let walks = Arc::new(AtomicU32::new(0));
    let seen = Arc::clone(&walks);
    let (source, answered) = workspace(move |_, request| {
        assert_eq!(
            variables(request).get("first"),
            Some(&json!(1)),
            "a walk asks for pages of one: {request}"
        );
        if after_c1(request) {
            return issues(vec![issue("i2", "second", "p1")]);
        }
        if seen.fetch_add(1, Ordering::SeqCst) == 0 {
            issues(vec![])
        } else {
            issues_then(vec![issue("i1", "first", "p1")], "c1")
        }
    });
    settled_walk(
        BOUND,
        source.as_ref(),
        &under("p1"),
        10,
        "a walk in pages of one",
        &["second".to_owned(), "first".to_owned()],
    )
    .await
    .unwrap();
    assert_eq!(
        walks.load(Ordering::SeqCst),
        2,
        "one walk more than the lag"
    );
    assert_eq!(
        answered.load(Ordering::SeqCst),
        3,
        "the empty walk's one page, then both pages of the walk that agrees"
    );
}

#[tokio::test]
async fn a_walk_that_never_reaches_the_expected_set_fails_within_the_bound_naming_what_it_reached()
{
    let (source, answered) = workspace(|_, request| {
        if after_c1(request) {
            issues(vec![issue("i3", "stranger", "p2")])
        } else {
            issues_then(vec![issue("i1", "first", "p1")], "c1")
        }
    });
    let started = Instant::now();
    let refusal = settled_walk(
        BOUND,
        source.as_ref(),
        &under("p1"),
        10,
        "a walk in pages of one",
        &["first".to_owned(), "second".to_owned()],
    )
    .await
    .unwrap_err();
    let waited = started.elapsed();
    assert!(
        refusal.starts_with(
            r#"a walk in pages of one came back as ["first", "stranger"] rather than ["first", "second"], still after 5 reads"#
        ),
        "the failure names what the walk reached: {refusal}"
    );
    assert_eq!(answered.load(Ordering::SeqCst), 2 * BOUND.reads);
    assert!(
        waited >= BOUND.interval * (BOUND.reads - 1)
            && waited < BOUND.interval * BOUND.reads + SLACK,
        "a wrong walk fails once the bound is spent, and waited {waited:?}"
    );
}

#[test]
fn the_label_wait_names_the_root_field_and_the_variable_of_the_plugins_own_lookup() {
    use graphql_parser::query;
    let lookup = onetaskgraph_linear::graphql::ISSUE_LABEL;
    let document = query::parse_query::<String>(lookup).unwrap();
    let [query::Definition::Operation(query::OperationDefinition::Query(operation))] =
        &document.definitions[..]
    else {
        panic!("the plugin's label lookup is not one query: {lookup}");
    };
    assert_eq!(
        operation
            .variable_definitions
            .iter()
            .map(|variable| variable.name.as_str())
            .collect::<Vec<_>>(),
        [LABEL_VARIABLE],
        "the wait sends the one variable the lookup takes: {lookup}"
    );
    let [query::Selection::Field(root)] = &operation.selection_set.items[..] else {
        panic!("the plugin's label lookup does not select one root field: {lookup}");
    };
    assert_eq!(
        root.name, LABEL_CONNECTION,
        "the wait reads the root field the lookup answers under: {lookup}"
    );
    let nodes = root
        .selection_set
        .items
        .iter()
        .find_map(|selected| match selected {
            query::Selection::Field(field) if field.name == "nodes" => Some(field),
            _ => None,
        })
        .unwrap_or_else(|| panic!("the lookup selects no nodes for the wait to count: {lookup}"));
    assert!(
        nodes.selection_set.items.iter().any(
            |selected| matches!(selected, query::Selection::Field(field) if field.name == "id")
        ),
        "the lookup selects the ids a duplicate refusal reports: {lookup}"
    );
}

#[tokio::test]
async fn a_label_the_lookup_holds_late_passes_without_looking_again() {
    // Created a moment before, and not held by the label filter for the first two lookups.
    let (url, answered) = serve(|n, request| {
        assert_looks_up_label(request);
        labels(if n > 2 { vec!["l1"] } else { vec![] })
    });
    settled_label(BOUND, LABEL, |query, variables| {
        post(&url, query, variables)
    })
    .await
    .unwrap();
    assert_eq!(
        answered.load(Ordering::SeqCst),
        3,
        "the wait looks the label up until one answers, and not once more"
    );
}

#[tokio::test]
async fn a_label_the_lookup_never_holds_fails_within_the_bound_naming_what_it_found() {
    let (url, answered) = serve(|_, request| {
        assert_looks_up_label(request);
        labels(vec![])
    });
    let started = Instant::now();
    let refusal = settled_label(BOUND, LABEL, |query, variables| {
        post(&url, query, variables)
    })
    .await
    .unwrap_err();
    let waited = started.elapsed();
    assert!(
        refusal.starts_with(
            r#"the label named "otg-live-label" by the lookup a write resolves it through found 0 matches, still after 5 reads"#
        ),
        "the failure names what the lookup found: {refusal}"
    );
    assert_eq!(answered.load(Ordering::SeqCst), BOUND.reads);
    assert!(
        waited >= BOUND.interval * (BOUND.reads - 1)
            && waited < BOUND.interval * BOUND.reads + SLACK,
        "a label never held fails once the bound is spent, and waited {waited:?}"
    );
}

#[tokio::test]
async fn a_label_two_labels_answer_to_is_refused_at_once_with_their_ids() {
    // Duplicate data cannot settle, so waiting would only delay the actionable refusal.
    let (url, answered) = serve(|_, request| {
        assert_looks_up_label(request);
        labels(vec!["l1", "l2"])
    });
    let started = Instant::now();
    let refusal = settled_label(BOUND, LABEL, |query, variables| {
        post(&url, query, variables)
    })
    .await
    .unwrap_err();
    assert!(
        refusal.starts_with(
            r#"the label named "otg-live-label" by the lookup a write resolves it through found 2 matches with ids ["l1", "l2"]"#
        ),
        "the failure names what the lookup found: {refusal}"
    );
    assert_eq!(answered.load(Ordering::SeqCst), 1);
    assert!(started.elapsed() < BOUND.interval + SLACK);
}

#[tokio::test]
async fn a_label_lookup_that_cannot_be_read_fails_at_once_rather_than_waiting() {
    let (url, answered) = serve(|_, _| json!({"issueLabels": null}));
    let started = Instant::now();
    let refusal = settled_label(BOUND, LABEL, |query, variables| {
        post(&url, query, variables)
    })
    .await
    .unwrap_err();
    assert!(
        refusal.starts_with(
            r#"the label named "otg-live-label" by the lookup a write resolves it through could not be read: "#
        ),
        "{refusal}"
    );
    assert_eq!(answered.load(Ordering::SeqCst), 1);
    assert!(started.elapsed() < BOUND.interval + SLACK);
}

#[tokio::test]
async fn a_label_lookup_with_an_unusable_id_fails_at_once_rather_than_settling() {
    let (url, answered) = serve(|_, request| {
        assert_looks_up_label(request);
        json!({LABEL_CONNECTION: {"nodes": [{}]}})
    });
    let started = Instant::now();
    let refusal = settled_label(BOUND, LABEL, |query, variables| {
        post(&url, query, variables)
    })
    .await
    .unwrap_err();
    assert!(refusal.contains("label node has no string id"), "{refusal}");
    assert_eq!(answered.load(Ordering::SeqCst), 1);
    assert!(started.elapsed() < BOUND.interval + SLACK);
}

#[tokio::test]
async fn a_listing_that_cannot_be_read_fails_at_once_rather_than_waiting() {
    let (source, answered) = workspace(|_, _| json!({"issues": null}));
    let started = Instant::now();
    let refusal = settled_tasks(
        BOUND,
        source.as_ref(),
        &under("p1"),
        "the issues of this run's project",
        &expected("filed"),
    )
    .await
    .unwrap_err();
    assert!(
        refusal.starts_with("the issues of this run's project could not be read: "),
        "{refusal}"
    );
    assert_eq!(answered.load(Ordering::SeqCst), 1);
    assert!(started.elapsed() < BOUND.interval + SLACK);
}

#[tokio::test]
async fn a_walk_given_more_than_one_row_on_a_page_of_one_fails_at_once() {
    // Exactly the expected set, so only the page limit can refuse it.
    let (source, answered) = workspace(|_, _| {
        issues(vec![
            issue("i1", "first", "p1"),
            issue("i2", "second", "p1"),
        ])
    });
    let started = Instant::now();
    let refusal = settled_walk(
        BOUND,
        source.as_ref(),
        &under("p1"),
        10,
        "a walk in pages of one",
        &["first".to_owned(), "second".to_owned()],
    )
    .await
    .unwrap_err();
    assert_eq!(
        refusal,
        "a walk in pages of one returned 2 rows on a page of one"
    );
    assert_eq!(
        answered.load(Ordering::SeqCst),
        1,
        "a page past its limit is not an index catching up, so it is not read again"
    );
    assert!(started.elapsed() < BOUND.interval + SLACK);
}

#[tokio::test]
async fn a_walk_that_does_not_end_fails_at_its_page_bound_naming_what_it_reached() {
    let (source, answered) = workspace(|_, _| issues_then(vec![issue("i1", "first", "p1")], "c1"));
    let refusal = settled_walk(
        BOUND,
        source.as_ref(),
        &under("p1"),
        3,
        "a walk in pages of one",
        &expected("first"),
    )
    .await
    .unwrap_err();
    assert_eq!(
        refusal,
        r#"a walk in pages of one had not ended after 3 pages, having reached ["first", "first", "first"]"#
    );
    assert_eq!(answered.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn a_document_listing_that_does_not_end_fails_at_its_page_bound() {
    // Every page full, so the plugin answers each of its pages with one request, and every page
    // names another after it.
    let (source, answered) = workspace(|_, _| {
        documents_then(
            (0..onetaskgraph_linear::MAX_PAGE_SIZE)
                .map(|n| document(&format!("d{n}"), "filed", None))
                .collect(),
            "c1",
        )
    });
    let refusal = settled_documents(
        BOUND,
        DocumentListingBudget {
            pages: 3,
            elapsed: Duration::from_secs(5),
        },
        source.as_ref(),
        &DocumentQuery::default(),
        &[NativeId("d0".into())],
        "the documents this run created",
        &expected_document("d0", "filed"),
    )
    .await
    .unwrap_err();
    assert!(
        refusal.starts_with(
            "the documents this run created exhausted its document listing budget after 3 pages"
        ),
        "budget exhaustion is named distinctly: {refusal}"
    );
    assert_eq!(answered.load(Ordering::SeqCst) as usize, 3);
}

#[tokio::test]
async fn a_document_listing_that_outlasts_its_time_budget_is_budget_exhaustion() {
    let (source, answered) = workspace(|_, _| {
        thread::sleep(Duration::from_millis(50));
        documents(vec![document("d1", "filed", None)])
    });
    let refusal = settled_documents(
        BOUND,
        DocumentListingBudget {
            pages: 10,
            elapsed: Duration::from_millis(10),
        },
        source.as_ref(),
        &DocumentQuery::default(),
        &[NativeId("d1".into())],
        "the documents this run created",
        &expected_document("d1", "filed"),
    )
    .await
    .unwrap_err();
    assert!(
        refusal.starts_with(
            "the documents this run created exhausted its document listing budget after 0 pages"
        ),
        "time exhaustion is named as budget exhaustion with the pages read: {refusal}"
    );
    assert_eq!(answered.load(Ordering::SeqCst), 1);
}
