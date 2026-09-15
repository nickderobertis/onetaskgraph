//! The live journey's exact-set reads, against a loopback workspace whose index lags.
//!
//! The reads and the wait are the real ones — `settle::settled_tasks`,
//! `settle::settled_documents` and `settle::settled_walk`, the same code `tests/live.rs`
//! compares Linear's listings through — and they reach that workspace through the real plugin
//! over real HTTP. What stands in for Linear is a local server that answers a filtered listing
//! from an index that has not caught up with a write for a number of reads, or never does.
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

use settle::{Bound, settled_documents, settled_tasks, settled_walk};

/// Room for the late listings below, which agree on their third read, with reads to spare, so
/// that a pass is the listing catching up rather than the bound running out on the last read.
const BOUND: Bound = Bound {
    reads: 5,
    interval: Duration::from_millis(50),
};

/// How late past the bound a failing wait may still report, for a loaded machine.
const SLACK: Duration = Duration::from_secs(10);

struct Key;
impl SecretResolver for Key {
    fn get(&self, _: &str) -> Option<SecretString> {
        Some("test-key".into())
    }
}

/// A workspace answering the `n`th request, counted from one, with `answer(n, request)`, and
/// how many requests it has answered.
fn workspace(
    answer: impl Fn(u32, &str) -> Value + Send + 'static,
) -> (Box<dyn TaskSource>, Arc<AtomicU32>) {
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
    let source = onetaskgraph_linear::Plugin
        .build(
            &SourceName::new("live").unwrap(),
            &json!({"endpoint": url}),
            &Key,
        )
        .unwrap();
    (source, answered)
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

/// Whether `request` asks for the page after the cursor `c1`.
fn after_c1(request: &str) -> bool {
    request.contains(r#""after":"c1""#)
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
        source.as_ref(),
        &DocumentQuery::default(),
        &|_| true,
        "the two documents this run created",
        &["loose".to_owned(), "filed".to_owned()],
    )
    .await
    .unwrap();
    settled_documents(
        BOUND,
        source.as_ref(),
        &filed_under("p1"),
        &|_| true,
        "a document listing narrowed to this run's project",
        &expected("filed"),
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
        source.as_ref(),
        &filed_under("p1"),
        &|title| title != "another run's",
        "a document listing narrowed to this run's project",
        &expected("filed"),
    )
    .await
    .unwrap_err();
    let waited = started.elapsed();
    assert!(
        refusal.starts_with(
            r#"a document listing narrowed to this run's project came back as [] rather than ["filed"], still after 5 reads"#
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
async fn a_document_listing_reads_past_a_first_page_full_of_other_documents() {
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
        source.as_ref(),
        &DocumentQuery::default(),
        &|title| !title.starts_with("another run's"),
        "the documents this run created",
        &expected("filed"),
    )
    .await
    .unwrap();
    assert_eq!(
        answered.load(Ordering::SeqCst),
        2,
        "one read walks both pages, and agrees on them"
    );
}

#[tokio::test]
async fn a_walk_in_pages_of_one_the_index_catches_up_with_late_passes_without_walking_again() {
    // The first walk finds the index empty; every walk after it reaches both issues, one a page.
    let walks = Arc::new(AtomicU32::new(0));
    let seen = Arc::clone(&walks);
    let (source, answered) = workspace(move |_, request| {
        assert!(
            request.contains(r#""first":1"#),
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
