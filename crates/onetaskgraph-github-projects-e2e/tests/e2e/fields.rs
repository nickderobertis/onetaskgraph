//! `sources fields`: the guarded setup of every board field a GitHub Projects source names,
//! through the real command-line boundary against the loopback board.

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{
    GitHubBoardFields, document, github_projects_with_board, github_projects_with_board_failing,
};
use crate::github_schema;
use onetaskgraph_github_projects::graphql::STATUS_OPTIONS_UPDATE;

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
        "board: missing configured Status options: task: Queued; project: Queued\n\
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
fn a_second_field_failing_after_the_first_changed_is_refused_with_the_recovery_data() {
    let sandbox = Sandbox::new();
    let (config, board) =
        github_projects_with_board_failing(&sandbox, &["createProjectV2Field(input:$input)"]);
    sandbox.project_document(&document(&json!({"board": {
        "plugin": "github-projects", "config": config
    }})));
    board.without_option("Queued");
    board.without_priority_field();
    let output = sandbox
        .command()
        .args(["sources", "fields", "board", "--apply"])
        .assert()
        .failure()
        .get_output()
        .clone();
    let said = stderr(&output);
    assert!(
        said.contains(
            "changed the Status field and then failed on the Priority field, which it may have \
             changed part way"
        ) && said.contains("createProjectV2Field")
            && said.contains("the pre-write item assignments are:")
            && said.contains("\"Status\"")
            && said.contains("OPT-todo"),
        "{said}"
    );
    // The Status half landed, which is exactly what the message says.
    assert!(
        board
            .status_options()
            .iter()
            .any(|option| option["name"] == "Queued")
    );
    assert!(board.priority_options().is_none());
}

#[test]
fn a_persons_own_field_is_read_past_whatever_it_holds() {
    let (sandbox, board) = configured();
    // A colour this product's vocabulary has no member for, on a field it never set up.
    board.with_persons_field(json!({"__typename": "ProjectV2SingleSelectField",
        "id": "FIELD-size", "name": "Size",
        "options": [{"id": "OPT-size-s", "name": "S", "color": "TEAL", "description": ""}]}));
    board.without_priority_option("Low");
    let planned = report(&sandbox, &["--json", "sources", "fields", "board"]);
    let names: Vec<&Value> = planned["fields"]
        .as_array()
        .expect("fields")
        .iter()
        .map(|field| &field["field"])
        .collect();
    assert_eq!(names, [&json!("Status"), &json!("Priority")]);
    assert_eq!(planned["fields"][1]["missing"], json!(["Low"]));
    let status = report(&sandbox, &["--json", "sources", "status-options", "board"]);
    assert_eq!(status["missing"], json!([]));
    let applied = report(
        &sandbox,
        &["--json", "sources", "fields", "board", "--apply"],
    );
    assert_eq!(applied["fields"][1]["outcome"], "applied");
}

/// Plan and apply `sources fields` on a board whose answer is ambiguous, expecting each to be
/// refused naming `refusal` and neither to write anything.
fn refused_without_a_write(sandbox: &Sandbox, board: &GitHubBoardFields, refusal: &str) {
    for arguments in [
        &["sources", "fields", "board"][..],
        &["sources", "fields", "board", "--apply"][..],
    ] {
        let output = sandbox
            .command()
            .args(arguments)
            .assert()
            .failure()
            .get_output()
            .clone();
        assert!(
            stderr(&output).contains(refusal),
            "`onetaskgraph {}` did not refuse naming {refusal:?}:\n{}",
            arguments.join(" "),
            stderr(&output)
        );
    }
    assert!(mutations(board).is_empty(), "{:?}", mutations(board));
}

#[test]
fn a_board_answering_two_status_or_two_priority_fields_is_refused_rather_than_one_chosen() {
    for name in ["Status", "Priority"] {
        let (sandbox, board) = configured();
        // A person's own single-select field beside the two is read past, as always.
        board.with_persons_field(json!({"__typename": "ProjectV2SingleSelectField",
            "id": "FIELD-size", "name": "Size",
            "options": [{"id": "OPT-size-s", "name": "S", "color": "TEAL", "description": ""}]}));
        let planned = report(&sandbox, &["--json", "sources", "fields", "board"]);
        assert_eq!(planned["fields"].as_array().expect("fields").len(), 2);

        // A second field of one name leaves no way to tell which the setup would act on.
        board.with_persons_field(json!({"__typename": "ProjectV2SingleSelectField",
            "id": format!("FIELD-{name}-again"), "name": name,
            "options": [{"id": "OPT-again", "name": "Again", "color": "GRAY",
                         "description": ""}]}));
        refused_without_a_write(
            &sandbox,
            &board,
            &format!("GitHub answered two {name} fields for this board"),
        );
    }
}

#[test]
fn an_item_answered_with_two_values_of_one_field_is_refused_rather_than_one_kept() {
    for name in ["Status", "Priority"] {
        let (sandbox, board) = configured();
        let applied = report(
            &sandbox,
            &["--json", "sources", "fields", "board", "--apply"],
        );
        assert_eq!(applied["fields"][0]["outcome"], "unchanged");

        // Keeping either value would verify an apply against an assignment the item may not
        // hold, and recovery data built on it would restore the wrong one.
        board.repeat_snapshot_value(name);
        refused_without_a_write(
            &sandbox,
            &board,
            &format!("GitHub answered two {name} values for board item"),
        );
    }
}

#[test]
fn a_verification_read_that_fails_after_the_write_is_refused_with_the_recovery_data() {
    let (sandbox, board) = configured();
    board.without_priority_option("Low");
    board.hide_after_update();
    let output = sandbox
        .command()
        .args(["sources", "fields", "board", "--apply"])
        .assert()
        .failure()
        .get_output()
        .clone();
    let said = stderr(&output);
    assert!(
        said.contains(
            "changed the Priority field and then could not read the board back to verify it"
        ) && said.contains("the pre-write item assignments are:")
            && said.contains("OPT-p-high"),
        "{said}"
    );
}

#[test]
fn a_priority_field_status_options_does_not_own_cannot_refuse_it() {
    let (sandbox, board) = configured();
    board.recolor_priority_option("High", "TEAL");
    // The Status-only form reads Status alone, exactly as it always did.
    let status = report(&sandbox, &["--json", "sources", "status-options", "board"]);
    assert_eq!(status["missing"], json!([]));
    assert_eq!(status["outcome"], "planned");
    // The whole setup owns the Priority field, and refuses an answer it cannot read.
    let output = sandbox
        .command()
        .args(["sources", "fields", "board"])
        .assert()
        .failure()
        .get_output()
        .clone();
    assert!(
        stderr(&output).contains("GitHub single-select option color is invalid"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn a_priority_field_of_another_type_is_refused_rather_than_created_beside() {
    let (sandbox, board) = configured();
    board.without_priority_field();
    board.with_persons_field(json!({"__typename": "ProjectV2Field",
        "id": "FIELD-priority-text", "name": "Priority"}));
    for arguments in [
        &["sources", "fields", "board"][..],
        &["sources", "fields", "board", "--apply"][..],
    ] {
        let output = sandbox
            .command()
            .args(arguments)
            .assert()
            .failure()
            .get_output()
            .clone();
        assert!(
            stderr(&output).contains(
                "board's board has a Priority field that is not a single-select field (it is a \
                 ProjectV2Field)"
            ) && stderr(&output).contains("next:"),
            "{}",
            stderr(&output)
        );
    }
    assert!(mutations(&board).is_empty(), "nothing was created");
}

#[test]
fn a_reminted_or_dropped_priority_option_is_refused_with_the_recovery_data() {
    for (drift, said) in [
        (
            GitHubBoardFields::remint_after_priority_update as fn(&GitHubBoardFields),
            "a pre-existing Priority option id, name, color or description",
        ),
        (
            GitHubBoardFields::omit_added_priority_option,
            "an added Priority option",
        ),
    ] {
        let (sandbox, board) = configured();
        board.without_priority_option("Medium");
        drift(&board);
        let output = sandbox
            .command()
            .args(["sources", "fields", "board", "--apply"])
            .assert()
            .failure()
            .get_output()
            .clone();
        let stderr = stderr(&output);
        assert!(
            stderr.contains(&format!("GitHub changed {said}"))
                && stderr.contains("the pre-write item assignments are:")
                && stderr.contains("OPT-p-high"),
            "{stderr}"
        );
    }
}

#[test]
fn drift_after_the_write_is_refused_with_the_pre_write_assignments() {
    let (sandbox, board) = configured();
    board.without_priority_option("Medium");
    board.drift_after_priority_update();
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
            "source notes uses plugin in-memory, not github-projects; fields is only available for github-projects and linear sources",
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

/// The board the live field setup failed on: a configured Status option missing and no
/// Priority field. Every request the setup sends is one GitHub's complete published schema
/// accepts — the board refuses any other, as GitHub does — and both fields end up configured
/// with no item's status or priority moved.
#[test]
fn a_board_missing_a_status_option_and_the_priority_field_is_set_up_by_requests_github_accepts() {
    let (sandbox, board) = configured();
    board.without_option("Cancelled");
    board.without_priority_field();
    let status_before = board.status_options();
    let values_before = values(&sandbox);

    let planned = report(&sandbox, &["--json", "sources", "fields", "board"]);
    assert_eq!(planned["fields"][0]["missing"], json!(["Cancelled"]));
    assert_eq!(planned["fields"][1]["exists"], false);
    assert!(mutations(&board).is_empty(), "a plan writes nothing");

    let applied = sandbox
        .command()
        .args(["sources", "fields", "board", "--apply"])
        .assert()
        .success()
        .get_output()
        .clone();
    assert_eq!(
        stdout(&applied),
        "board: added and verified Status options: task: Cancelled; project: Cancelled\n\
         board: created the Priority field with: Urgent, High, Medium, Low\n"
    );

    for (query, variables) in board.served() {
        if let Err(errors) = github_schema::validate(&query, &variables) {
            panic!("GitHub would refuse {query} with {variables}: {errors:?}");
        }
    }
    let sent = mutations(&board);
    assert_eq!(sent.len(), 2, "{sent:?}");
    assert!(sent[0].0.contains("updateProjectV2Field(input:$input)"));
    let options = sent[0].1["input"]["singleSelectOptions"].clone();
    assert_eq!(
        sent[0].1["input"],
        json!({"fieldId": "FIELD-status", "singleSelectOptions": options})
    );
    assert!(sent[1].0.contains("createProjectV2Field(input:$input)"));

    let status_after = board.status_options();
    assert!(
        status_before
            .iter()
            .all(|option| status_after.contains(option))
    );
    assert!(
        status_after
            .iter()
            .any(|option| option["name"] == "Cancelled")
    );
    let priorities: Vec<Value> = board
        .priority_options()
        .expect("the field was created")
        .iter()
        .map(|option| option["name"].clone())
        .collect();
    assert_eq!(
        priorities,
        [
            json!("Urgent"),
            json!("High"),
            json!("Medium"),
            json!("Low")
        ]
    );
    assert_eq!(
        values(&sandbox),
        values_before,
        "no item's status or priority moved"
    );

    let again = report(
        &sandbox,
        &["--json", "sources", "fields", "board", "--apply"],
    );
    for field in again["fields"].as_array().expect("fields") {
        assert_eq!(field["outcome"], "unchanged", "{again}");
    }
}

/// The request 0.12.13's field setup sent — `projectId` inside `UpdateProjectV2FieldInput`,
/// which GitHub does not define — is refused by the board exactly as GitHub refused it, and
/// nothing on the board changes.
#[test]
fn the_board_refuses_the_status_option_update_0_12_13_sent_as_github_does() {
    let (_sandbox, board) = configured();
    board.without_option("Cancelled");
    let before = board.status_options();
    let mut options = before.clone();
    options.push(json!({"name": "Cancelled", "color": "GRAY", "description": ""}));

    let answered = board.graphql_response(
        STATUS_OPTIONS_UPDATE,
        &json!({"input": {"projectId": "PVT-board", "fieldId": "FIELD-status",
                          "singleSelectOptions": options}}),
    );
    // GitHub answers it HTTP 200 with `errors` naming the field it does not define, at the
    // variable it was sent in, and no `data`.
    assert_eq!(
        answered.as_object().map(|body| body.len()),
        Some(1),
        "{answered}"
    );
    assert_eq!(
        answered["errors"].as_array().map(Vec::len),
        Some(1),
        "{answered}"
    );
    assert_eq!(
        answered["errors"][0]["locations"],
        json!([{"line": 1, "column": 10}])
    );
    let message = answered["errors"][0]["message"]
        .as_str()
        .unwrap_or_default();
    assert!(
        message.starts_with(
            "Variable \"$input\" has invalid value: Expected value of type \
             \"UpdateProjectV2FieldInput\" not to include unknown field \"projectId\""
        ),
        "{answered}"
    );
    assert_eq!(
        board.status_options(),
        before,
        "a refused request changes nothing"
    );

    // The same update without it is the one GitHub runs.
    let answered = board.graphql_response(
        STATUS_OPTIONS_UPDATE,
        &json!({"input": {"fieldId": "FIELD-status", "singleSelectOptions": options}}),
    );
    assert!(answered.get("errors").is_none(), "{answered}");
    assert!(
        board
            .status_options()
            .iter()
            .any(|option| option["name"] == "Cancelled")
    );
}

/// The board refuses two selections whose merged response would answer one name with two
/// different fields — even when each selection is valid on its own, the conflict sitting a
/// level down — as GitHub refuses them, and nothing of the request is answered.
#[test]
fn the_board_refuses_selections_whose_merged_response_conflicts() {
    let (_sandbox, board) = configured();
    let refused = |query: &str, variables: Value| {
        let answered = board.graphql_response(query, &variables);
        assert!(answered.get("data").is_none(), "{answered}");
        answered["errors"]
            .as_array()
            .unwrap_or_else(|| panic!("the board ran {query}: {answered}"))
            .iter()
            .map(|error| error["message"].as_str().expect("a message").to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        refused("query { viewer { a: login } viewer { a: id } }", json!({})),
        [
            "Fields \"viewer\" conflict because subfields \"a\" conflict because \"login\" and \"id\" \
          are different fields. Use different aliases on the fields to fetch both if this was \
          intentional."
        ]
    );
    // Two fragments that can never both apply may select different fields under one name,
    // but never ones whose answers have different shapes.
    assert_eq!(
        refused(
            "query($id:ID!){node(id:$id){... on Issue{x:title} ... on PullRequest{x:number}}}",
            json!({"id": "I_1"})
        ),
        [
            "Fields \"x\" conflict because they return conflicting types \"String!\" and \
          \"Int!\". Use different aliases on the fields to fetch both if this was intentional."
        ]
    );
    assert_eq!(
        refused(
            "query($id:ID!){node(id:$id){... on Issue{x:author{login}} \
             ... on PullRequest{x:author{y:login}} ... on PullRequest{x:author{y:url}}}}",
            json!({"id": "I_1"})
        ),
        [
            "Fields \"x\" conflict because subfields \"y\" conflict because \"login\" and \"url\" \
          are different fields. Use different aliases on the fields to fetch both if this was \
          intentional."
        ]
    );
}

/// Repeated selections whose merged response is consistent are what GitHub runs, and the board
/// answers them.
#[test]
fn the_board_answers_repeated_selections_that_merge() {
    let (_sandbox, board) = configured();
    let answered = board.graphql_response(
        "query($id:ID!){node(id:$id){... on ProjectV2{title shortDescription readme}} \
         node(id:$id){... on ProjectV2{title}}}",
        &json!({"id": "PVT-board"}),
    );
    assert!(answered.get("errors").is_none(), "{answered}");
    assert_eq!(answered["data"]["node"]["title"], "Fixture board");
    for compatible in [
        "query { viewer { a: login } viewer { a: login name } }",
        "query($id:ID!){node(id:$id){... on Issue{x:title} ... on PullRequest{x:title}}}",
        "query($id:ID!){node(id:$id){... on Issue{x:number} ... on PullRequest{x:number}}}",
    ] {
        let variables = if compatible.contains("$id") {
            json!({"id": "I_1"})
        } else {
            json!({})
        };
        if let Err(errors) = github_schema::validate(compatible, &variables) {
            panic!("GitHub runs {compatible}: {errors:?}");
        }
    }
}

/// A request the board refuses changes nothing on it, as one GitHub refuses runs none of: a
/// removal the board holds for the next request naming an item is still held after a refused
/// request named it, and the item is still listed.
#[test]
fn a_request_the_board_refuses_changes_nothing_on_it() {
    let (sandbox, board) = configured();
    let listed = |sandbox: &Sandbox| {
        report(sandbox, &["--json", "task", "list"])["items"]
            .as_array()
            .expect("a page of tasks")
            .iter()
            .any(|task| task["id"] == "board:T-2")
    };
    assert!(listed(&sandbox));
    board.replace_after("T-2", 0, None);
    let answered = board.graphql_response(
        "query($id:ID!){node(id:$id){... on Issue{x:title x:body}}}",
        &json!({"id": "T-2"}),
    );
    // graphql-js reports the conflict once for each order of the pair.
    let conflict = json!({
        "message": "Fields \"x\" conflict because \"title\" and \"body\" are different fields. \
            Use different aliases on the fields to fetch both if this was intentional.",
        "locations": [{"line": 1, "column": 42}, {"line": 1, "column": 50}]
    });
    assert_eq!(answered, json!({"errors": [conflict.clone(), conflict]}));
    assert!(
        listed(&sandbox),
        "a refused request removed the item it named"
    );
}

/// Set in the child process [`a_fragment_cycle_inside_repeated_selections_is_refused`] runs
/// itself in.
const CYCLE_CHILD: &str = "ONETASKGRAPH_FRAGMENT_CYCLE_CHILD";

/// A fragment that spreads itself, inside two selections the board must merge, is refused as
/// GitHub refuses it — an HTTP answer with `errors`, no `data`, nothing on the board changed —
/// rather than recursing until the stack overflows.
///
/// The request is sent from a child process running this same test, because a validator that
/// recurses without end aborts its whole process on the overflow: run in this one, it would
/// take the test runner and every other journey with it instead of failing this test.
#[test]
fn a_fragment_cycle_inside_repeated_selections_is_refused() {
    if std::env::var_os(CYCLE_CHILD).is_some() {
        let (sandbox, board) = configured();
        let status = board.status_options();
        let priority = board.priority_options();
        let values_before = values(&sandbox);
        let answered = board.graphql_response(
            "query { viewer { ...F } viewer { ...F } } \
             fragment F on User { followers(first: 1) { nodes { ...F } } }",
            &json!({}),
        );
        assert_eq!(
            answered,
            json!({"errors": [{"message": "Cannot spread fragment \"F\" within itself.",
                "locations": [{"line": 1, "column": 94}]}]})
        );
        assert_eq!(board.status_options(), status);
        assert_eq!(board.priority_options(), priority);
        assert_eq!(
            values(&sandbox),
            values_before,
            "a refused request changed the board"
        );
        return;
    }
    let child = std::process::Command::new(std::env::current_exe().expect("this test binary"))
        .args([
            "--exact",
            "fields::a_fragment_cycle_inside_repeated_selections_is_refused",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CYCLE_CHILD, "1")
        .output()
        .expect("the child test runs");
    assert!(
        child.status.success(),
        "the child exited {}:\n{}\n{}",
        child.status,
        String::from_utf8_lossy(&child.stdout),
        String::from_utf8_lossy(&child.stderr)
    );
    assert!(
        String::from_utf8_lossy(&child.stdout).contains("1 passed"),
        "the child ran no test: {}",
        String::from_utf8_lossy(&child.stdout)
    );
}
// llmlint: ignore-end[tests_mirror_real_usage]
