//! What every document this source sends costs GitHub's hourly rate-limit allowance.
//!
//! No network, no credential and no schema: the price is arithmetic over the document's
//! own text, so this runs in the ordinary check on a pull request from a fork.
//!
//! The number is `cost`, the rate-limit **points** one call spends against an allowance one
//! credential shares across everything it does in an hour. It is not `nodeCount`, which is
//! what `tests/node_count.rs` holds every document below GitHub's per-query limit — two
//! numbers against two limits, and a document well under the node limit says nothing about
//! its price.
//!
//! **There is no ceiling to sweep against, so the pin is the check.** GitHub publishes no
//! per-call point limit a document can be over: points are an allowance that runs out over
//! an hour rather than a bound a query is refused for crossing. So what is asserted here is
//! that every document has a recorded price and that every recorded price is what that
//! document computes — which is what makes a price that quadruples a number moving rather
//! than a test still passing.
//!
//! **GitHub is the authority on its own pricing and this workspace is not.** The figures
//! below are what stop a regression merging, because they need no credential; the
//! credentialed lane reconciles each of them against GitHub's own `cost`, from the
//! `rateLimit(dryRun: true)` probe `tests/journey/mod.rs` already sends per read document.

use onetaskgraph_github_projects::{
    COPY_DETAIL_BATCH, DETAIL_BATCH, MEMBER_PAGE_SIZE, graphql, worst_case_point_cost,
};

/// What each document this source sends costs, in points, under the largest page sizes it
/// can be driven with.
///
/// One line per document, with the price written out as a number, because that is the whole
/// instrument: a shared fragment that grows a connection moves one of these figures, and a
/// figure that moves is a line of this file somebody had to change. The sweep below reads
/// this table against [`graphql::DOCUMENTS`] both ways, so neither a document that landed
/// without a price nor a price for a document this source no longer sends can sit here
/// unnoticed.
///
/// Every mutation costs 1 — GitHub's published minimum for any call — because none of them
/// selects a connection. That is a figure rather than an exemption: a mutation that grew a
/// connection would move it.
const PRICES: &[(&str, u64)] = &[
    (graphql::ISSUES_BY_CREATION, 1),
    // llmlint: ignore[tests_hold_no_nonfunctional_thresholds] Measured price golden, not a budget ceiling; onebudgetspec judges budgets. See session-cost.md.
    (graphql::SEARCH_ISSUES, 6),
    (graphql::ISSUE, 1),
    (graphql::ISSUE_BOARD_ITEMS, 1),
    // llmlint: ignore[tests_hold_no_nonfunctional_thresholds] The price is the instrument, not a threshold: the module documentation records that GitHub publishes no per-call point limit to sweep against, so the pin is the check that a document's price moved. This change moved it deliberately, and `session-cost.md` says why.
    (graphql::SUB_ISSUES, 2),
    // llmlint: ignore[tests_hold_no_nonfunctional_thresholds] Measured price golden, not a budget ceiling; onebudgetspec judges budgets. See session-cost.md.
    (graphql::BOARD, 3),
    (graphql::ORIGIN_LOOKUP, 1),
    // llmlint: ignore-block[tests_hold_no_nonfunctional_thresholds] As at `SUB_ISSUES` above: the price is the instrument, not a threshold. `task status reconcile` reads the board's closed items and `task status complete` reads issues by reference, and the sweep below refuses any document in `graphql::DOCUMENTS` without a recorded price; the second's one point per fifty references is what the `github-landing-closure-points` budget's figure rests on, and onebudgetspec judges that budget.
    (graphql::CLOSED_ITEMS, 3),
    (graphql::ISSUES_BY_REFERENCE, 1),
    // llmlint: ignore-end[tests_hold_no_nonfunctional_thresholds]
    (graphql::BOARD_FIELDS, 1),
    (graphql::DRAFT, 1),
    (graphql::REPOSITORY, 1),
    (graphql::PROJECT_VISIBILITY, 1),
    (graphql::CREATION_CONTEXT, 1),
    (graphql::ISSUE_DEPENDENCIES, 1),
    // llmlint: ignore-block[tests_hold_no_nonfunctional_thresholds] As at `SUB_ISSUES` above: the price is the instrument, not a threshold, and the sweep below refuses any document in `graphql::DOCUMENTS` without a recorded price; these are the two one-direction dependency reads, and what a dependency read spends is the dependency-read budgets', judged by onebudgetspec.
    (graphql::ISSUE_DEPENDS_ON, 1),
    (graphql::ISSUE_DEPENDED_ON_BY, 1),
    // llmlint: ignore-end[tests_hold_no_nonfunctional_thresholds]
    (graphql::CREATE_ISSUE, 1),
    // llmlint: ignore-block[tests_hold_no_nonfunctional_thresholds] As at `SUB_ISSUES` above: the price is the instrument, not a threshold. These are the documents a copy now reads its origins and writes its created items with, and the sweep below refuses any document in `graphql::DOCUMENTS` without a recorded price; what a plan copy spends is the `github-plan-copy-points-*` budgets', judged by onebudgetspec.
    (graphql::ORIGIN_LOOKUPS, 1),
    // GitHub's own dry run, beside the model's: `docs/measurements/board-copy-gaps-2026-10-24/coalesced-members-dry-run`.
    (graphql::ORIGIN_LOOKUPS_WITH_MEMBERS, 1),
    (graphql::CREATE_ISSUES, 1),
    (graphql::CREATED_BOARD_ITEMS, 1),
    (graphql::WRITE_FIELD_VALUES, 1),
    (graphql::CLOSE_ISSUES, 1),
    (graphql::UPDATE_ISSUES, 1),
    (graphql::ADD_BLOCKED_BYS, 1),
    (graphql::WRITE_FIELDS_AND_EDGES, 1),
    (graphql::DELETE_ISSUES, 1),
    (graphql::DOCUMENT_IN_PROJECT, 2),
    // llmlint: ignore-end[tests_hold_no_nonfunctional_thresholds]
    (graphql::ADD_TO_BOARD, 1),
    (graphql::UPDATE_ISSUE, 1),
    (graphql::UPDATE_DRAFT, 1),
    (graphql::UPDATE_FIELD, 1),
    (graphql::UPDATE_FIELDS, 1),
    (graphql::CLEAR_FIELD, 1),
    (graphql::CREATE_FIELD, 1),
    (graphql::ADD_SUB_ISSUE, 1),
    (graphql::REMOVE_SUB_ISSUE, 1),
    (graphql::ADD_BLOCKED_BY, 1),
    (graphql::REMOVE_BLOCKED_BY, 1),
    (graphql::DELETE_ISSUE, 1),
    (graphql::ISSUE_COMMENTS, 1),
    (graphql::ISSUE_DETAIL, 1),
    // llmlint: ignore[tests_hold_no_nonfunctional_thresholds] Measured price golden, not a budget ceiling; onebudgetspec judges budgets. See session-cost.md.
    (graphql::ISSUE_DETAILS, 2),
    // llmlint: ignore[tests_hold_no_nonfunctional_thresholds] As at `SUB_ISSUES` above: the price is the instrument, not a threshold. A copy's batched read is a document of its own, and the sweep below refuses any document in `graphql::DOCUMENTS` without a recorded price, so it needs one; `session-cost.md` records why it was added.
    (graphql::ISSUE_COPY_DETAILS, 2),
    (graphql::COMMENT_ISSUE, 1),
    (graphql::ADD_COMMENT, 1),
    (graphql::UPDATE_COMMENT, 1),
    (graphql::DELETE_COMMENT, 1),
    // The whole-issue twins an expected write sends in place of the document beside each, so
    // the store's cache can take what their responses show: each selects what `ISSUE_DETAIL`
    // does, and costs the one point it does.
    // llmlint: ignore-block[tests_hold_no_nonfunctional_thresholds] As at `SUB_ISSUES` above: the price is the instrument, not a threshold. These five are new documents, and the sweep below refuses any document in `graphql::DOCUMENTS` without a recorded price; the ceiling on what the cache spends is the `github-cache-points-not-above-uncached` budget's, judged by onebudgetspec.
    (graphql::ADD_COMMENT_WHOLE, 1),
    (graphql::UPDATE_COMMENT_WHOLE, 1),
    (graphql::UPDATE_ISSUE_WHOLE, 1),
    (graphql::UPDATE_FIELD_WHOLE, 1),
    (graphql::CLEAR_FIELD_WHOLE, 1),
    // llmlint: ignore-end[tests_hold_no_nonfunctional_thresholds]
    (graphql::STATUS_OPTIONS_SNAPSHOT, 1),
    (graphql::STATUS_OPTIONS_UPDATE, 1),
];

/// What this source's own text says `document` costs against what [`PRICES`] records, or
/// the failure it is.
///
/// One place the verdict is spelled, so the inventory sweep below and the discrimination
/// case beneath it are the same check over different figures rather than two checks that
/// could come to differ. `recorded` is passed in rather than looked up, which is what lets
/// that case drive a price nobody would write down.
fn mispriced(doing: &str, document: &str, recorded: Option<u64>) -> Option<String> {
    let price = worst_case_point_cost(document)
        .unwrap_or_else(|error| panic!("the document for {doing} could not be priced: {error}"));
    match recorded {
        None => Some(format!(
            "the document for {doing} costs {price} points and this file records no price \
             for it; next: add it to PRICES at {price}"
        )),
        Some(recorded) if recorded != price => Some(format!(
            "the document for {doing} now costs {price} points and this file records \
             {recorded}; next: if the change is deliberate, put {price} in PRICES and say \
             in session-cost.md what moved it"
        )),
        Some(_) => None,
    }
}

/// The price [`PRICES`] records for `document`, if it records one.
fn recorded(document: &str) -> Option<u64> {
    PRICES
        .iter()
        .find(|(pinned, _)| *pinned == document)
        .map(|(_, price)| *price)
}

/// Every document, from the inventory rather than from a list written out here.
///
/// Reading [`graphql::DOCUMENTS`] is what makes this cover a document nobody thought
/// about: `documents_are_all_inventoried` already fails when a `pub const` there is left
/// out of that list, so a document added later is priced here without this test being
/// edited to know about it — and, having no price recorded, fails until somebody writes
/// one down.
#[test]
fn every_document_this_source_sends_costs_what_this_file_records() {
    let moved = graphql::DOCUMENTS
        .iter()
        .filter_map(|(document, doing)| mispriced(doing, document, recorded(document)))
        .collect::<Vec<_>>();
    assert!(moved.is_empty(), "{}", moved.join("\n"));
}

/// And the other way: nothing is priced here that this source does not send.
///
/// A price left behind by a document that was deleted or rewritten would go on passing the
/// sweep above for ever, because that sweep only ever reads the pins the inventory reaches.
#[test]
fn every_price_recorded_here_belongs_to_a_document_this_source_sends() {
    let orphaned = PRICES
        .iter()
        .filter(|(pinned, _)| {
            !graphql::DOCUMENTS
                .iter()
                .any(|(document, _)| document == pinned)
        })
        .map(|(pinned, price)| {
            format!("this file records {price} points for a document the inventory does not hold; next: delete the entry, or put the document back in graphql::DOCUMENTS:\n{pinned}")
        })
        .collect::<Vec<_>>();
    assert!(orphaned.is_empty(), "{}", orphaned.join("\n"));
}

/// The check refuses a document whose price has moved, naming both figures.
///
/// Without this the sweep above could pass over any tree at all — including one where the
/// calculation had been wired up to answer whatever was recorded. It drives the real
/// verdict over a real document against a price nobody would write down, which is the same
/// shape and the same reason as `tests/node_count.rs`'s pre-fix document.
#[test]
fn the_check_reports_a_failure_naming_a_document_whose_price_moved() {
    let refusal = mispriced("reading the board", graphql::BOARD, Some(8))
        .expect("a price this document does not cost is refused");
    assert!(refusal.contains("reading the board"), "{refusal}");
    assert!(refusal.contains('8'), "{refusal}");
    assert!(refusal.contains('3'), "{refusal}");
}

/// And it refuses a document with no price at all.
///
/// This is the half that catches a document landing without anybody pricing it: the sweep
/// reads the inventory, so a new document reaches this verdict with `None` recorded.
#[test]
fn the_check_reports_a_failure_naming_a_document_nobody_priced() {
    let refusal = mispriced("reading the board", graphql::BOARD, None)
        .expect("a document with no recorded price is refused");
    assert!(refusal.contains("reading the board"), "{refusal}");
    assert!(refusal.contains("records no price"), "{refusal}");
}

/// gh4 adds a comment-clock connection per issue. Keep the established batch sizes so
/// correctness costs no additional request; their measured price rises from one point to two.
// llmlint: ignore-block[tests_hold_no_nonfunctional_thresholds] Measured prices and preserved batch shapes, not budget ceilings; onebudgetspec judges budgets. See session-cost.md.
#[test]
fn the_batch_read_keeps_its_batch_at_the_recorded_gh4_price() {
    assert_eq!(
        DETAIL_BATCH, 24,
        "gh4 must not shrink batches and add requests"
    );
    batch_price(graphql::ISSUE_DETAILS, DETAIL_BATCH);
}

#[test]
fn the_copy_batch_read_keeps_its_batch_at_the_recorded_gh4_price() {
    assert_eq!(
        COPY_DETAIL_BATCH, 21,
        "gh4 must not shrink batches and add requests"
    );
    batch_price(graphql::ISSUE_COPY_DETAILS, COPY_DETAIL_BATCH);
}

#[test]
fn the_member_page_keeps_its_size_at_the_recorded_gh4_price() {
    assert_eq!(
        MEMBER_PAGE_SIZE, 23,
        "gh4 must not shrink pages and add requests"
    );
    assert_eq!(worst_case_point_cost(graphql::SUB_ISSUES).unwrap(), 2);
}

fn batch_price(document: &str, batch: usize) {
    assert!(!document.contains("nodes(ids:"), "{document}");
    assert_eq!(document.matches(":node(id:$id").count(), batch);
    assert_eq!(worst_case_point_cost(document).unwrap(), 2);
}
// llmlint: ignore-end[tests_hold_no_nonfunctional_thresholds]
