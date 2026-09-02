//! Every document this source sends, held below GitHub's per-query node limit.
//!
//! No network, no credential and no schema: the count is arithmetic over the document's
//! own text, so this runs in the ordinary check on a pull request from a fork.
//!
//! The number is `nodeCount`, the most nodes **one query may return**, which GitHub checks
//! per query and refuses a document above before executing it. It is not `cost`, the
//! rate-limit points a call spends against an hourly allowance; nothing here is about that.

use onetaskgraph_github_projects::{NODE_COUNT_LIMIT, graphql, worst_case_node_count};

/// What this source's own text says about `document`, or the failure it is.
///
/// One place the verdict is spelled, so the inventory sweep below and the discrimination
/// case beneath it are the same check over different documents rather than two checks
/// that could come to differ. The message names the document and the count it computed,
/// because a reader who has just added a connection needs to know which document grew and
/// by how much — the limit alone says neither.
fn over_limit(doing: &str, document: &str) -> Option<String> {
    let count = worst_case_node_count(document)
        .unwrap_or_else(|error| panic!("the document for {doing} could not be counted: {error}"));
    (count >= NODE_COUNT_LIMIT).then(|| {
        format!(
            "the document for {doing} may return {count} nodes, at or above GitHub's limit \
             of {NODE_COUNT_LIMIT}; next: give one of its connections a smaller page size, \
             or stop selecting it"
        )
    })
}

/// Every document, from the inventory rather than from a list written out here.
///
/// Reading [`graphql::DOCUMENTS`] is what makes this cover a document nobody thought
/// about: `documents_are_all_inventoried` already fails when a `pub const` there is left
/// out of that list, so a connection added later to a shared fragment is counted here
/// without this test being edited to know about it.
#[test]
fn every_document_this_source_sends_stays_under_githubs_node_limit() {
    let refused = graphql::DOCUMENTS
        .iter()
        .filter_map(|(document, doing)| over_limit(doing, document))
        .collect::<Vec<_>>();
    assert!(refused.is_empty(), "{}", refused.join("\n"));
}

/// The three documents that share the board-issue fragment, with the headroom each has.
///
/// The sweep above says only that nothing crossed. This says where each of them sits, so a
/// change that quadruples a count while staying under the limit is visible as a number
/// moving rather than as a test still passing. The figures are recomputed here from the
/// production text, never restated from the documentation.
#[test]
fn the_documents_that_reach_an_issue_under_a_page_are_the_ones_with_least_headroom() {
    let count = |document: &str| worst_case_node_count(document).expect("a countable document");
    assert_eq!(count(graphql::SEARCH_ISSUES), 56_100);
    assert_eq!(count(graphql::SUB_ISSUES), 56_100);
    assert_eq!(count(graphql::BOARD), 260_150);
    assert_eq!(count(graphql::ISSUE), 560);
    assert_eq!(count(graphql::ISSUE_DEPENDENCIES), 200);
    assert_eq!(count(graphql::REPOSITORY), 0);
}

/// The board-issue fragment as it stood before the innermost label connection came out.
///
/// Kept verbatim rather than described: this is the document GitHub refused with *"by the
/// time this query traverses to the labels connection, it is requesting up to 2,500,000
/// possible nodes which exceeds the maximum limit of 500,000"*, and it is what makes the
/// check above evidence rather than an assertion nobody has watched fail.
const SEARCH_ISSUES_BEFORE_THE_FIX: &str = r#"query($search:String!,$type:SearchType!,$first:Int!,$after:String,$nestedFirst:Int!,$boardItems:Int!,$duplicates:Boolean!){
      search(query:$search,type:$type,first:$first,after:$after){
        pageInfo{hasNextPage endCursor}
        nodes{__typename ...BoardIssue}
      }
    } fragment BoardIssue on Issue{__typename id title body url createdAt updatedAt state stateReason(enableDuplicate:$duplicates) repository{nameWithOwner} parent{id} subIssuesSummary{total}
      labels(first:$nestedFirst){nodes{id name color}pageInfo{hasNextPage}}
      projectItems(first:$boardItems){nodes{id project{number}
        fieldValues(first:$nestedFirst){nodes{
          ... on ProjectV2ItemFieldSingleSelectValue{name field{
            ... on ProjectV2SingleSelectField{id name options{id name}}
          }}
          ... on ProjectV2ItemFieldTextValue{text field{... on ProjectV2Field{id name}}}
          ... on ProjectV2ItemFieldLabelValue{labels(first:$nestedFirst){nodes{id name color}pageInfo{hasNextPage}}}
        }pageInfo{hasNextPage}}}pageInfo{hasNextPage}}}"#;

/// The check refuses an over-limit document, naming it and the count it computed.
///
/// Without this the sweep above could pass over any tree at all — including one where the
/// calculation had been wired up to return zero.
#[test]
fn the_check_reports_a_failure_naming_an_over_limit_document_and_its_count() {
    let refusal = over_limit(
        "searching this board's issues before the fix",
        SEARCH_ISSUES_BEFORE_THE_FIX,
    )
    .expect("the pre-fix document is over the limit");
    assert!(
        refusal.contains("searching this board's issues before the fix"),
        "{refusal}"
    );
    assert!(refusal.contains("2556100"), "{refusal}");
    assert!(refusal.contains("500000"), "{refusal}");
}

/// The sum across sibling paths, not the deepest path alone.
///
/// GitHub's message quotes 2,500,000 — the innermost path on its own — and a check that
/// computed only that would be a different check from the one GitHub runs. This pins the
/// pre-fix document at the sum of every path, which is the figure the published rules
/// produce and the one the limit is applied to.
#[test]
fn the_count_sums_sibling_paths_rather_than_taking_the_deepest_one() {
    assert_eq!(
        worst_case_node_count(SEARCH_ISSUES_BEFORE_THE_FIX).expect("a countable document"),
        2_556_100
    );
}
