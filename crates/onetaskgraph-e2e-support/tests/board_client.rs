//! How the loopback GitHub board's own client — what a journey replaying a recorded trace, or
//! reading the board as a person would see it, sends its requests through — answers a
//! connection that ends before its answer arrives, over real loopback sockets.

use onetaskgraph_e2e_support::common::Sandbox;
use onetaskgraph_e2e_support::fixtures::github_projects_with_items;
use serde_json::json;

#[test]
fn requests_share_one_kept_connection() {
    let sandbox = Sandbox::new();
    let (_, board) = github_projects_with_items(&sandbox, Vec::new());
    let first = board.own();
    assert_eq!(board.own(), first);
    assert_eq!(
        board.connections(),
        1,
        "the board's own client opened a connection per request rather than keeping one"
    );
}

#[test]
fn a_read_whose_connection_ends_unanswered_is_sent_again_and_answered() {
    let sandbox = Sandbox::new();
    let (_, board) = github_projects_with_items(&sandbox, Vec::new());
    let answered = board.own();
    board.drop_response_once("shortDescription");
    assert_eq!(board.own(), answered, "the read was answered after all");
    assert_eq!(
        board
            .documents()
            .iter()
            .filter(|query| query.contains("shortDescription"))
            .count(),
        3,
        "the read the board closed unanswered was sent once more, on a fresh connection"
    );
    assert_eq!(board.connections(), 2);
}

#[test]
#[should_panic(expected = "sending it again would apply it twice")]
fn a_mutation_the_board_applied_unanswered_is_never_sent_again() {
    let sandbox = Sandbox::new();
    let (_, board) = github_projects_with_items(&sandbox, Vec::new());
    board.drop_response_once("createIssue");
    board.graphql(
        "mutation($input:CreateIssueInput!){createIssue(input:$input){issue{id}}}",
        &json!({"input":{"repositoryId":"REPO-1","title":"once","body":"once"}}),
    );
}

#[test]
fn comment_pages_keep_issue_order_after_a_removal_and_an_append() {
    let sandbox = Sandbox::new();
    let item = |id: &str| {
        json!({"item":format!("ITEM-{id}"),"id":id,"type":"Issue",
        "title":"Plain","body":"Plain","state":"OPEN","reason":null,"parent":null,
        "repo":"nickderobertis/onetaskgraph","status":"Todo","origin":"","labels":[]})
    };
    let (_, board) = github_projects_with_items(&sandbox, vec![item("A"), item("B")]);
    let first = board.add_comment("A", "First");
    board.add_comment("B", "Other issue");
    let second = board.add_comment("A", "Second");
    let page = |after| {
        board.graphql(
            onetaskgraph_github_projects::graphql::ISSUE_COMMENTS,
            &json!({"id":"A","first":1,"after":after}),
        )["node"]["comments"]
            .clone()
    };
    let initial = page(serde_json::Value::Null);
    assert_eq!(initial["nodes"][0]["id"], first);
    assert_eq!(initial["pageInfo"]["hasNextPage"], true);
    let next = page(initial["pageInfo"]["endCursor"].clone());
    assert_eq!(next["nodes"][0]["id"], second);
    assert_eq!(next["pageInfo"]["hasNextPage"], false);

    // The number of comments stays the same, but positions change. A new walk must
    // see the current store rather than the index built for the previous walk.
    board.delete_comment(&first);
    let third = board.add_comment("A", "Third");
    board.edit_comment(&second, "Edited second");
    let initial = page(serde_json::Value::Null);
    assert_eq!(initial["nodes"][0]["id"], second);
    assert_eq!(initial["nodes"][0]["body"], "Edited second");
    let next = page(initial["pageInfo"]["endCursor"].clone());
    assert_eq!(next["nodes"][0]["id"], third);
    assert_eq!(next["pageInfo"]["hasNextPage"], false);
}
