//! The wait every exact-set read of the live journey makes, and the bound it keeps.
//!
//! `tests/live.rs` reads through it against Linear; `tests/settle_gate.rs` drives these very
//! reads, through the real plugin, against a loopback workspace whose index lags — so that a
//! listing which catches up passes and one which never does fails, naming what it returned,
//! is proven with no credential and no third party. Both are ordinary tests in this crate's
//! ordinary `test` target.
//!
//! Linear indexes a write filter by filter rather than all at once. One run had its label read
//! return all three issues while `project: {null: true}` still returned none; the next had a
//! document listing narrowed to a project come back without the document filed under it,
//! while the unfiltered listing just before it already held both. Each failed on code that was
//! correct, so no listing the journey compares to an exact set is read only once.
//!
//! A write reaches the same index before any listing does. The plugin resolves a label a write
//! names through Linear's label filter, and a run had its first task write refused as unable to
//! resolve uniquely a label it had created a moment before — so a label the journey creates is
//! waited for too, through that very lookup.

use std::future::Future;
use std::time::{Duration, Instant};

use onetaskgraph_plugin_api::{DocumentQuery, NativeId, PageRequest, TaskQuery, TaskSource};
use serde_json::{Value, json};

/// How many times a listing is read, and how far apart, before its disagreement is the answer.
#[derive(Clone, Copy, Debug)]
pub struct Bound {
    pub reads: u32,
    pub interval: Duration,
}

/// What the live journey gives Linear's index: the first read, then the same twenty
/// one-second polls the fixture always settled with.
pub const LINEAR_INDEX: Bound = Bound {
    reads: 21,
    interval: Duration::from_secs(1),
};

/// `Ok` once `read` answers `expected`, compared as sorted sets; otherwise an `Err` naming the
/// last listing it answered, once `bound` is spent.
///
/// A read that fails is returned at once rather than retried: a refusal is not an index that
/// has not caught up, and waiting would only report it twenty seconds later.
pub async fn settled<F, Fut>(
    bound: Bound,
    what: &str,
    expected: &[String],
    mut read: F,
) -> Result<(), String>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<Vec<String>, String>>,
{
    let mut expected = expected.to_vec();
    expected.sort();
    let started = Instant::now();
    let mut listed = read().await?;
    let mut reads = 1;
    while listed != expected && reads < bound.reads {
        tokio::time::sleep(bound.interval).await;
        listed = read().await?;
        reads += 1;
    }
    if listed == expected {
        return Ok(());
    }
    Err(format!(
        "{what} came back as {listed:?} rather than {expected:?}, still after {reads} reads \
         over {:?}",
        started.elapsed()
    ))
}

/// The sorted titles of one page of `limit` tasks `query` answers.
pub async fn task_titles(
    source: &dyn TaskSource,
    query: &TaskQuery,
    limit: u32,
    what: &str,
) -> Result<Vec<String>, String> {
    let mut titles = source
        .query_tasks(
            query,
            &PageRequest {
                cursor: None,
                limit,
            },
        )
        .await
        .map_err(|error| format!("{what} could not be read: {error}"))?
        .items
        .into_iter()
        .map(|task| task.title)
        .collect::<Vec<_>>();
    titles.sort();
    Ok(titles)
}

/// The titles of every task `query` answers, walked in pages of one, in the order the walk
/// reached them.
///
/// A page holding more than one row, or a walk that has not ended after `most` pages, is an
/// `Err` naming what it reached: neither is an index that has not caught up.
pub async fn walked_task_titles(
    source: &dyn TaskSource,
    query: &TaskQuery,
    most: usize,
    what: &str,
) -> Result<Vec<String>, String> {
    let mut walked = Vec::new();
    let mut cursor = None;
    for _ in 0..most {
        let step = source
            .query_tasks(query, &PageRequest { cursor, limit: 1 })
            .await
            .map_err(|error| format!("{what} could not be read: {error}"))?;
        if step.items.len() > 1 {
            return Err(format!(
                "{what} returned {} rows on a page of one",
                step.items.len()
            ));
        }
        walked.extend(step.items.into_iter().map(|task| task.title));
        cursor = step.next;
        if cursor.is_none() {
            return Ok(walked);
        }
    }
    Err(format!(
        "{what} had not ended after {most} pages, having reached {walked:?}"
    ))
}

/// The request and elapsed-time budget for one complete document listing.
#[derive(Clone, Copy, Debug)]
pub struct DocumentListingBudget {
    pub pages: usize,
    pub elapsed: Duration,
}

/// Ten thousand documents at Linear's page size and thirty seconds for their responses, both
/// far past what the scratch workspace ordinarily needs. A listing past either bound is refused
/// explicitly as budget exhaustion rather than being mistaken for a missing document.
pub const LINEAR_DOCUMENT_LISTING: DocumentListingBudget = DocumentListingBudget {
    pages: 100,
    elapsed: Duration::from_secs(30),
};

/// The sorted `(id, title)` pairs belonging to this run that `query` answers, to its last page.
///
/// `run_ids` is there because Linear's `documents` connection is the whole workspace: the live
/// journey retains only ids it created, never another run's in-flight documents. Every page is
/// read because a workspace holding a page of older documents can put this run's ids later.
pub async fn documents_from_run(
    source: &dyn TaskSource,
    query: &DocumentQuery,
    run_ids: &[NativeId],
    budget: DocumentListingBudget,
    what: &str,
) -> Result<Vec<(NativeId, String)>, String> {
    let started = Instant::now();
    let mut found = Vec::new();
    let mut cursor = None;
    for page in 1..=budget.pages {
        let request = PageRequest {
            cursor,
            limit: onetaskgraph_linear::MAX_PAGE_SIZE,
        };
        let read = source.query_documents(query, &request);
        let Some(remaining) = budget.elapsed.checked_sub(started.elapsed()) else {
            return Err(format!(
                "{what} exhausted its document listing budget after {} pages over {:?}, before the listing ended",
                page - 1,
                started.elapsed()
            ));
        };
        let step = tokio::time::timeout(remaining, read)
            .await
            .map_err(|_| {
                format!(
                    "{what} exhausted its document listing budget after {} pages over {:?}, before the listing ended",
                    page - 1,
                    started.elapsed()
                )
            })?
            .map_err(|error| format!("{what} could not be read: {error}"))?;
        found.extend(step.items.into_iter().filter_map(|document| {
            run_ids
                .contains(&document.id)
                .then_some((document.id, document.title))
        }));
        cursor = step.next;
        if cursor.is_none() {
            found.sort_by(|left, right| left.0.cmp(&right.0));
            return Ok(found);
        }
        if page == budget.pages || started.elapsed() >= budget.elapsed {
            return Err(format!(
                "{what} exhausted its document listing budget after {page} pages over {:?}, before the listing ended",
                started.elapsed()
            ));
        }
    }
    unreachable!("a positive document page budget returns inside the loop")
}

/// [`settled`] over a page of fifty tasks.
pub async fn settled_tasks(
    bound: Bound,
    source: &dyn TaskSource,
    query: &TaskQuery,
    what: &str,
    expected: &[String],
) -> Result<(), String> {
    settled(bound, what, expected, || {
        task_titles(source, query, 50, what)
    })
    .await
}

/// [`settled`] over the set a walk in pages of one reaches.
pub async fn settled_walk(
    bound: Bound,
    source: &dyn TaskSource,
    query: &TaskQuery,
    most: usize,
    what: &str,
    expected: &[String],
) -> Result<(), String> {
    settled(bound, what, expected, || async move {
        let mut walked = walked_task_titles(source, query, most, what).await?;
        walked.sort();
        Ok(walked)
    })
    .await
}

/// The root field `graphql::ISSUE_LABEL` answers under, whose `nodes` are the labels it found.
///
/// Named here once for the wait and the loopback workspace that stands in for it;
/// `tests/settle_gate.rs` reconciles it, and [`LABEL_VARIABLE`], with that document.
pub const LABEL_CONNECTION: &str = "issueLabels";

/// The one variable `graphql::ISSUE_LABEL` takes: the label's name.
pub const LABEL_VARIABLE: &str = "name";

/// The lookup a write resolves the label `name` through, until exactly one label answers it.
///
/// The lookup is the plugin's own `graphql::ISSUE_LABEL`, sent by `send`, which answers with the
/// request's `data`. No match is the index-lag answer this wait can settle. Multiple matches are
/// duplicate data, so they are refused immediately with the ids the plugin would report rather
/// than pointlessly waiting out the bound.
pub async fn settled_label<F, Fut>(bound: Bound, name: &str, send: F) -> Result<(), String>
where
    F: Fn(&'static str, Value) -> Fut,
    Fut: Future<Output = Result<Value, String>>,
{
    let what = format!("the label named {name:?} by the lookup a write resolves it through");
    let (what, send) = (what.as_str(), &send);
    let started = Instant::now();
    let mut reads = 0;
    loop {
        let data = send(
            onetaskgraph_linear::graphql::ISSUE_LABEL,
            json!({ LABEL_VARIABLE: name }),
        )
        .await
        .map_err(|error| format!("{what} could not be read: {error}"))?;
        let nodes = data
            .get(LABEL_CONNECTION)
            .and_then(|connection| connection.get("nodes"))
            .and_then(Value::as_array)
            .ok_or_else(|| {
                format!("{what} could not be read: no {LABEL_CONNECTION}.nodes in {data}")
            })?;
        reads += 1;
        let ids = nodes
            .iter()
            .map(|node| {
                node.get("id").and_then(Value::as_str).ok_or_else(|| {
                    format!("{what} could not be read: label node has no string id: {node}")
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        match nodes.len() {
            1 => return Ok(()),
            0 if reads < bound.reads => tokio::time::sleep(bound.interval).await,
            0 => {
                return Err(format!(
                    "{what} found 0 matches, still after {reads} reads over {:?}",
                    started.elapsed()
                ));
            }
            found => {
                return Err(format!("{what} found {} matches with ids {ids:?}", found));
            }
        }
    }
}

/// Wait until a complete listing has exactly this run's expected documents.
pub async fn settled_documents(
    bound: Bound,
    listing_budget: DocumentListingBudget,
    source: &dyn TaskSource,
    query: &DocumentQuery,
    run_ids: &[NativeId],
    what: &str,
    expected: &[(NativeId, String)],
) -> Result<(), String> {
    assert!(
        listing_budget.pages > 0,
        "a document listing budget needs a page"
    );
    let started = Instant::now();
    let mut reads = 0;
    loop {
        let mut listed = documents_from_run(source, query, run_ids, listing_budget, what).await?;
        listed.sort_by(|left, right| left.0.cmp(&right.0));
        let mut expected = expected.to_vec();
        expected.sort_by(|left, right| left.0.cmp(&right.0));
        reads += 1;
        if listed == expected {
            return Ok(());
        }
        if reads == bound.reads {
            let missing = expected
                .iter()
                .filter(|(id, _)| !listed.iter().any(|(listed_id, _)| listed_id == id))
                .map(|(id, _)| id.as_str())
                .collect::<Vec<_>>();
            if !missing.is_empty() {
                return Err(format!(
                    "{what} completed its listing but the document ids {missing:?} were missing, still after {reads} reads over {:?}",
                    started.elapsed()
                ));
            }
            return Err(format!(
                "{what} completed its listing with {listed:?} rather than {expected:?}, still after {reads} reads over {:?}",
                started.elapsed()
            ));
        }
        tokio::time::sleep(bound.interval).await;
    }
}

/// [`settled`] over the direct read of a deleted document, until it is absent.
pub async fn settled_document_absent(
    bound: Bound,
    source: &dyn TaskSource,
    id: &NativeId,
    name: &str,
) -> Result<(), String> {
    let what = format!("the deleted document {name:?}");
    settled(bound, &what, &[], || async {
        source
            .get_document(id)
            .await
            .map(|document| {
                document
                    .into_iter()
                    .map(|document| document.title)
                    .collect()
            })
            .map_err(|error| format!("{what} could not be read: {error}"))
    })
    .await
}
