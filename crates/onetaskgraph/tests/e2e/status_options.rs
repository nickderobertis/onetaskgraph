//! Guarded Status-option reconciliation through the real command-line boundary.

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{document, github_projects_with_board};

fn configured() -> (Sandbox, crate::fixtures::GitHubBoardFields) {
    let sandbox = Sandbox::new();
    let (config, board) = github_projects_with_board(&sandbox);
    sandbox.project_document(&document(&json!({"board": {
        "plugin": "github-projects", "config": config
    }})));
    (sandbox, board)
}

#[test]
fn nothing_missing_is_a_read_only_plan_and_an_unchanged_apply() {
    let (sandbox, board) = configured();
    let output = sandbox
        .command()
        .args(["sources", "status-options", "board"])
        .assert()
        .success()
        .get_output()
        .clone();
    assert!(stdout(&output).contains("missing configured Status options: none"));
    assert!(
        !board
            .documents()
            .iter()
            .any(|document| document.contains("updateProjectV2Field"))
    );

    let json_output = sandbox
        .command()
        .args(["--json", "sources", "status-options", "board"])
        .assert()
        .success()
        .get_output()
        .clone();
    let report: Value = serde_json::from_str(&stdout(&json_output)).expect("a JSON report");
    assert_eq!(report["source"], "board");
    assert_eq!(report["missing"], json!([]));
    assert_eq!(report["outcome"], "planned");

    let applied_noop = sandbox
        .command()
        .args(["sources", "status-options", "board", "--apply"])
        .assert()
        .success()
        .get_output()
        .clone();
    assert_eq!(
        stdout(&applied_noop),
        "board: missing configured Status options: none\n"
    );
    let json_noop = sandbox
        .command()
        .args(["--json", "sources", "status-options", "board", "--apply"])
        .assert()
        .success()
        .get_output()
        .clone();
    let report: Value = serde_json::from_str(&stdout(&json_noop)).expect("a no-op JSON report");
    assert_eq!(report["outcome"], "unchanged");
    assert_eq!(report["missing"], json!([]));
    assert!(
        !board
            .documents()
            .iter()
            .any(|document| document.contains("updateProjectV2Field"))
    );
}

#[test]
fn configured_option_matching_is_case_insensitive() {
    let (sandbox, board) = configured();
    board.rename_option("Queued", "queued");
    let output = sandbox
        .command()
        .args(["sources", "status-options", "board"])
        .assert()
        .success()
        .get_output()
        .clone();
    assert_eq!(
        stdout(&output),
        "board: missing configured Status options: none\n"
    );
}

#[test]
fn apply_adds_only_the_missing_option_with_every_existing_id_and_assignment_preserved() {
    let (sandbox, board) = configured();
    board.without_option("Queued");
    let plan = sandbox
        .command()
        .args(["sources", "status-options", "board"])
        .assert()
        .success()
        .get_output()
        .clone();
    assert_eq!(
        stdout(&plan),
        "board: missing configured Status options: Queued\n"
    );
    let output = sandbox
        .command()
        .args(["--json", "sources", "status-options", "board", "--apply"])
        .assert()
        .success()
        .get_output()
        .clone();
    let report: Value = serde_json::from_str(&stdout(&output)).expect("an applied JSON report");
    assert_eq!(report["outcome"], "applied");
    assert_eq!(report["missing"], json!(["Queued"]));
    let existing = report["existing"]
        .as_array()
        .expect("the existing option list is machine-readable");
    assert!(existing.iter().any(|option| option["id"] == "OPT-todo"
        && option["name"] == "Todo"
        && option["color"] == "BLUE"
        && option["description"] == "ready"));
    assert!(existing.iter().any(|option| option["id"] == "OPT-shipped"
        && option["name"] == "Shipped"
        && option["color"] == "PURPLE"
        && option["description"] == "custom"));
    let (_, variables) = board
        .served()
        .into_iter()
        .find(|(document, _)| document.contains("updateProjectV2Field"))
        .expect("the guarded mutation was sent");
    let options = variables
        .pointer("/input/singleSelectOptions")
        .and_then(Value::as_array)
        .unwrap();
    assert!(
        options
            .iter()
            .filter(|option| option["name"] != "Queued")
            .all(|option| option["id"].is_string())
    );
    assert_eq!(
        options
            .iter()
            .filter(|option| option["name"] == "Queued")
            .count(),
        1
    );
    assert!(
        options
            .iter()
            .any(|option| option["name"] == "Shipped" && option["id"] == "OPT-shipped")
    );

    let (human_sandbox, human_board) = configured();
    human_board.without_option("Queued");
    let applied = human_sandbox
        .command()
        .args(["sources", "status-options", "board", "--apply"])
        .assert()
        .success()
        .get_output()
        .clone();
    assert_eq!(stdout(&applied), "board: added and verified: Queued\n");
}

#[test]
fn assignment_drift_is_refused_with_the_pre_write_recovery_snapshot() {
    let (sandbox, board) = configured();
    board.without_option("Queued");
    board.drift_after_status_update();
    let output = sandbox
        .command()
        .args(["sources", "status-options", "board", "--apply"])
        .assert()
        .failure()
        .get_output()
        .clone();
    let complaint = stderr(&output);
    assert!(
        complaint.contains("pre-write item assignment snapshot"),
        "{complaint}"
    );
    assert!(complaint.contains("OPT-todo"), "{complaint}");
    assert!(complaint.contains("item_id"), "{complaint}");
}

#[test]
fn option_id_drift_and_an_omitted_addition_are_each_refused() {
    for disturb in [
        crate::fixtures::GitHubBoardFields::remint_after_status_update,
        crate::fixtures::GitHubBoardFields::omit_added_status_option,
    ] {
        let (sandbox, board) = configured();
        board.without_option("Queued");
        disturb(&board);
        let output = sandbox
            .command()
            .args(["sources", "status-options", "board", "--apply"])
            .assert()
            .failure()
            .get_output()
            .clone();
        assert!(stderr(&output).contains("pre-write item assignment snapshot"));
    }
}

#[test]
fn unknown_and_non_github_sources_are_refused_by_name() {
    let (sandbox, _) = configured();
    let unknown = sandbox
        .command()
        .args(["sources", "status-options", "missing"])
        .assert()
        .failure()
        .get_output()
        .clone();
    assert!(stderr(&unknown).contains("missing"));

    sandbox.project_document(&document(&json!({"notes": {
        "plugin": "in-memory", "config": {}
    }})));
    let wrong = sandbox
        .command()
        .args(["sources", "status-options", "notes"])
        .assert()
        .failure()
        .get_output()
        .clone();
    let complaint = stderr(&wrong);
    assert!(
        complaint.contains("notes") && complaint.contains("not github-projects"),
        "{complaint}"
    );
}

#[test]
fn a_board_without_a_status_field_is_refused_by_source_name() {
    let (sandbox, board) = configured();
    board.without_status_field();
    let output = sandbox
        .command()
        .args(["sources", "status-options", "board"])
        .assert()
        .failure()
        .get_output()
        .clone();
    let complaint = stderr(&output);
    assert!(
        complaint.contains("board") && complaint.contains("no Status field"),
        "{complaint}"
    );
}

#[test]
fn an_inaccessible_board_is_refused_by_source_name() {
    let (sandbox, board) = configured();
    board.without_accessible_status_board();
    let output = sandbox
        .command()
        .args(["sources", "status-options", "board"])
        .assert()
        .failure()
        .get_output()
        .clone();
    let complaint = stderr(&output);
    assert!(
        complaint.contains("board") && complaint.contains("no accessible GitHub Projects board"),
        "{complaint}"
    );
}

#[test]
fn the_assignment_snapshot_walks_every_page() {
    let (sandbox, board) = configured();
    board.paginate_status_snapshots();
    sandbox
        .command()
        .args(["sources", "status-options", "board"])
        .assert()
        .success();
    assert!(
        board
            .documents()
            .iter()
            .filter(|document| document.contains("optionId field{"))
            .count()
            > 1
    );
}
