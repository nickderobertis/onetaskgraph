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

use std::future::Future;
use std::time::{Duration, Instant};

use onetaskgraph_plugin_api::{DocumentQuery, PageRequest, TaskQuery, TaskSource};

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

/// The sorted titles `keep` accepts of one whole page of documents `query` answers.
///
/// `keep` is there because Linear's `documents` connection is the whole workspace: the live
/// journey compares only its own run's documents, never another run's in flight beside it.
pub async fn document_titles(
    source: &dyn TaskSource,
    query: &DocumentQuery,
    keep: &dyn Fn(&str) -> bool,
    what: &str,
) -> Result<Vec<String>, String> {
    let mut titles = source
        .query_documents(
            query,
            &PageRequest {
                cursor: None,
                limit: onetaskgraph_linear::MAX_PAGE_SIZE,
            },
        )
        .await
        .map_err(|error| format!("{what} could not be read: {error}"))?
        .items
        .into_iter()
        .map(|document| document.title)
        .filter(|title| keep(title))
        .collect::<Vec<_>>();
    titles.sort();
    Ok(titles)
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

/// [`settled`] over the documents `keep` accepts.
pub async fn settled_documents(
    bound: Bound,
    source: &dyn TaskSource,
    query: &DocumentQuery,
    keep: &dyn Fn(&str) -> bool,
    what: &str,
    expected: &[String],
) -> Result<(), String> {
    settled(bound, what, expected, || {
        document_titles(source, query, keep, what)
    })
    .await
}
