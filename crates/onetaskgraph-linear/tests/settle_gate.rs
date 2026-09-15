//! The live journey's exact-set reads, against a loopback workspace whose index lags.
//!
//! The reads and the wait are the real ones — `settle::settled_tasks` and
//! `settle::settled_documents`, the same code `tests/live.rs` compares Linear's listings
//! through — and they reach that workspace through the real plugin over real HTTP. What
//! stands in for Linear is a local server that answers a filtered listing from an index that
//! has not caught up with a write for a number of reads, or never does.
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

use settle::{Bound, settled_documents, settled_tasks};

/// Five reads, fifty milliseconds apart.
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

/// One whole HTTP request: its headers, and as much body as they declare.
fn request(stream: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut chunk = [0; 8192];
    loop {
        let n = stream.read(&mut chunk).unwrap();
        bytes.extend_from_slice(&chunk[..n]);
        let text = String::from_utf8_lossy(&bytes);
        if let Some(end) = text.find("\r\n\r\n") {
            let declared = text[..end]
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())?
                })
                .unwrap_or(0);
            if bytes.len() >= end + 4 + declared {
                return text.into_owned();
            }
        }
        if n == 0 {
            return String::from_utf8_lossy(&bytes).into_owned();
        }
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

fn documents(nodes: Vec<Value>) -> Value {
    json!({"documents": {"nodes": nodes, "pageInfo": {"hasNextPage": false, "endCursor": null}}})
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
