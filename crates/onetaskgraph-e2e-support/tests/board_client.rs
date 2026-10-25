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
