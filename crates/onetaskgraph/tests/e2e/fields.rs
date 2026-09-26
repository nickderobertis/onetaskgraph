//! `sources fields`: the guarded setup of every board field a GitHub Projects source names,
//! through the real command-line boundary against the loopback board.

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{GitHubBoardFields, document, github_projects_with_board};

// llmlint: ignore-block[tests_mirror_real_usage] Every test below drives the compiled CLI
// against the real loopback HTTP boundary. What the verb owes is a wire effect — the whole
// option list each mutation sends, with every existing id, and no mutation at all for a plan —
// and the fixture's received-request log is the server-side observation of it, the same
// instrument `status_options.rs` reads, rather than an inspection of application internals.

fn configured() -> (Sandbox, GitHubBoardFields) {
    let sandbox = Sandbox::new();
    let (config, board) = github_projects_with_board(&sandbox);
    sandbox.project_document(&document(&json!({"board": {
        "plugin": "github-projects", "config": config
    }})));
    (sandbox, board)
}

fn report(sandbox: &Sandbox, arguments: &[&str]) -> Value {
    let output = sandbox
        .command()
        .args(arguments)
        .assert()
        .success()
        .get_output()
        .clone();
    serde_json::from_str(&stdout(&output)).expect("a JSON fields report")
}

/// Every mutation the board received, by the field each named.
fn mutations(board: &GitHubBoardFields) -> Vec<(String, Value)> {
    board
        .served()
        .into_iter()
        .filter(|(query, _)| query.trim_start().starts_with("mutation"))
        .collect()
}

/// Each task's status and priority, as the product reads them off the board.
fn values(sandbox: &Sandbox) -> Value {
    report(sandbox, &["--json", "task", "list"])["items"]
        .as_array()
        .expect("a page of tasks")
        .iter()
        .map(|task| json!([task["id"], task["item"]["status"], task["item"]["priority"]]))
        .collect()
}

#[test]
fn a_board_already_set_up_is_a_plan_that_writes_nothing_and_an_unchanged_apply() {
    let (sandbox, board) = configured();
    let status_before = board.status_options();
    let planned = report(&sandbox, &["--json", "sources", "fields", "board"]);
    assert_eq!(planned["source"], "board");
    let fields = planned["fields"].as_array().expect("one entry per field");
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0]["field"], "Status");
    assert_eq!(fields[1]["field"], "Priority");
    for field in fields {
        assert_eq!(field["exists"], true);
        assert_eq!(field["missing"], json!([]));
        assert_eq!(field["outcome"], "planned");
    }
    assert_eq!(fields[0]["existing"], json!(status_before));
    assert_eq!(
        fields[1]["existing"],
        json!(board.priority_options().expect("the field"))
    );

    let applied = report(
        &sandbox,
        &["--json", "sources", "fields", "board", "--apply"],
    );
    for field in applied["fields"].as_array().expect("fields") {
        assert_eq!(field["outcome"], "unchanged");
    }
    assert!(mutations(&board).is_empty(), "{:?}", mutations(&board));

    let text = sandbox
        .command()
        .args(["sources", "fields", "board"])
        .assert()
        .success()
        .get_output()
        .clone();
    assert_eq!(
        stdout(&text),
        "board: missing configured Status options: none\n\
         board: missing configured Priority options: none\n"
    );
}

#[test]
fn a_missing_priority_field_is_planned_then_created_with_the_mapped_options_in_order() {
    let (sandbox, board) = configured();
    board.without_priority_field();
    let status_before = board.status_options();
    let values_before = values(&sandbox);

    let planned = report(&sandbox, &["--json", "sources", "fields", "board"]);
    assert_eq!(
        planned["fields"][1],
        json!({"field": "Priority", "exists": false,
               "missing": ["Urgent", "High", "Medium", "Low"],
               "outcome": "planned", "existing": []})
    );
    assert!(mutations(&board).is_empty(), "a plan writes nothing");
    let text = sandbox
        .command()
        .args(["sources", "fields", "board"])
        .assert()
        .success()
        .get_output()
        .clone();
    assert!(
        stdout(&text)
            .contains("board: no Priority field; would create it with: Urgent, High, Medium, Low"),
        "{}",
        stdout(&text)
    );

    let applied = sandbox
        .command()
        .args(["sources", "fields", "board", "--apply"])
        .assert()
        .success()
        .get_output()
        .clone();
    assert_eq!(
        stdout(&applied),
        "board: missing configured Status options: none\n\
         board: created the Priority field with: Urgent, High, Medium, Low\n"
    );
    let sent = mutations(&board);
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert!(sent[0].0.contains("createProjectV2Field(input:$input)"));
    assert_eq!(
        sent[0].1["input"],
        json!({"projectId": "PVT-board", "dataType": "SINGLE_SELECT", "name": "Priority",
               "singleSelectOptions": [
                   {"name": "Urgent", "color": "GRAY", "description": ""},
                   {"name": "High", "color": "GRAY", "description": ""},
                   {"name": "Medium", "color": "GRAY", "description": ""},
                   {"name": "Low", "color": "GRAY", "description": ""}]})
    );
    let names: Vec<Value> = board
        .priority_options()
        .expect("the field was created")
        .iter()
        .map(|option| option["name"].clone())
        .collect();
    assert_eq!(
        names,
        [
            json!("Urgent"),
            json!("High"),
            json!("Medium"),
            json!("Low")
        ]
    );
    assert_eq!(board.status_options(), status_before, "Status is untouched");
    assert_eq!(values(&sandbox), values_before, "no item's values moved");

    // The board is now set up: a second apply changes nothing, and a priority write lands.
    let again = report(
        &sandbox,
        &["--json", "sources", "fields", "board", "--apply"],
    );
    assert_eq!(again["fields"][1]["outcome"], "unchanged");
    report(
        &sandbox,
        &["--json", "task", "priority", "set", "board:T-2", "medium"],
    );
    assert_eq!(board.priority("T-2").as_deref(), Some("Medium"));
}

#[test]
fn a_missing_option_of_either_field_is_added_with_every_existing_option_and_value_preserved() {
    let (sandbox, board) = configured();
    board.without_priority_option("Medium");
    board.without_option("Queued");
    let status_before = board.status_options();
    let priority_before = board.priority_options().expect("the field");
    let values_before = values(&sandbox);

    let planned = sandbox
        .command()
        .args(["sources", "fields", "board"])
        .assert()
        .success()
        .get_output()
        .clone();
    assert_eq!(
        stdout(&planned),
        "board: missing configured Status options: Queued\n\
         board: missing configured Priority options: Medium\n"
    );
    let applied = report(
        &sandbox,
        &["--json", "sources", "fields", "board", "--apply"],
    );
    assert_eq!(applied["fields"][0]["missing"], json!(["Queued"]));
    assert_eq!(applied["fields"][0]["outcome"], "applied");
    assert_eq!(applied["fields"][1]["missing"], json!(["Medium"]));
    assert_eq!(applied["fields"][1]["outcome"], "applied");
    assert_eq!(applied["fields"][1]["existing"], json!(priority_before));

    let sent = mutations(&board);
    assert_eq!(sent.len(), 2, "{sent:?}");
    for (query, variables) in &sent {
        assert!(
            query.contains("updateProjectV2Field(input:$input)"),
            "{query}"
        );
        let (before, added) = if variables["input"]["fieldId"] == "FIELD-status" {
            (&status_before, "Queued")
        } else {
            assert_eq!(variables["input"]["fieldId"], "FIELD-priority");
            (&priority_before, "Medium")
        };
        let options = variables["input"]["singleSelectOptions"]
            .as_array()
            .expect("the whole option list");
        // Every existing option goes back with its own id, name, color and description.
        assert_eq!(&options[..before.len()], &before[..]);
        assert_eq!(
            options[before.len()..],
            [json!({"name": added, "color": "GRAY", "description": ""})]
        );
    }
    let after = board.priority_options().expect("the field");
    assert!(priority_before.iter().all(|option| after.contains(option)));
    assert!(
        status_before
            .iter()
            .all(|option| board.status_options().contains(option))
    );
    assert_eq!(values(&sandbox), values_before, "no item's values moved");
}

#[test]
fn the_human_rendering_says_what_an_apply_added_and_verified() {
    let (sandbox, board) = configured();
    board.without_priority_option("Low");
    let applied = sandbox
        .command()
        .args(["sources", "fields", "board", "--apply"])
        .assert()
        .success()
        .get_output()
        .clone();
    assert_eq!(
        stdout(&applied),
        "board: missing configured Status options: none\n\
         board: added and verified Priority options: Low\n"
    );
    assert!(
        board
            .priority_options()
            .expect("the field")
            .iter()
            .any(|option| option["name"] == "Low")
    );
}

#[test]
fn drift_after_the_write_is_refused_with_the_pre_write_assignments() {
    let (sandbox, board) = configured();
    board.without_priority_option("Medium");
    board.drift_after_status_update();
    let output = sandbox
        .command()
        .args(["sources", "fields", "board", "--apply"])
        .assert()
        .failure()
        .get_output()
        .clone();
    let said = stderr(&output);
    assert!(
        said.contains("an item's Priority value")
            && said.contains("the pre-write item assignments are:")
            && said.contains("\"Priority\"")
            && said.contains("OPT-p-high"),
        "{said}"
    );
}

#[test]
fn a_board_without_status_and_a_source_that_is_not_a_board_are_refused_by_name() {
    let (sandbox, board) = configured();
    board.without_status_field();
    let output = sandbox
        .command()
        .args(["sources", "fields", "board"])
        .assert()
        .failure()
        .get_output()
        .clone();
    assert!(
        stderr(&output).contains("source board board has no Status field"),
        "{}",
        stderr(&output)
    );

    let sandbox = Sandbox::new();
    sandbox.project_document(&document(&json!({
        "notes": {"plugin": "in-memory", "config": {}}
    })));
    for (source, said) in [
        (
            "notes",
            "source notes uses plugin in-memory, not github-projects; fields is only available for github-projects sources",
        ),
        ("absent", "no configured source is named absent"),
    ] {
        let output = sandbox
            .command()
            .args(["sources", "fields", source])
            .assert()
            .failure()
            .get_output()
            .clone();
        assert!(stderr(&output).contains(said), "{}", stderr(&output));
    }
}

#[test]
fn a_board_with_no_priority_mapping_sets_up_its_status_field_alone() {
    let sandbox = Sandbox::new();
    let (mut config, board) = github_projects_with_board(&sandbox);
    config
        .as_object_mut()
        .expect("a config block")
        .remove("priority_mapping");
    sandbox.project_document(&document(&json!({"board": {
        "plugin": "github-projects", "config": config
    }})));
    board.without_priority_field();
    let applied = report(
        &sandbox,
        &["--json", "sources", "fields", "board", "--apply"],
    );
    let fields = applied["fields"].as_array().expect("fields");
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0]["field"], "Status");
    assert!(
        board.priority_options().is_none(),
        "no Priority field is created"
    );
    assert!(mutations(&board).is_empty());
}
// llmlint: ignore-end[tests_mirror_real_usage]
