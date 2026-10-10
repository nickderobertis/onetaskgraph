//! A `github-projects` source's `metadata_fields`: a caller's metadata value projected onto a
//! board text field, through the real command-line boundary against the loopback board — the
//! copy that lands it, the field setup that creates the field, and the write allowance the
//! projection spends, recorded as telemetry for `github-metadata-field-content-creation-share`.

use std::path::PathBuf;

use onetaskgraph_github_projects::{FIELD_CLEAR_SLOTS, FIELD_WRITE_SLOTS, graphql};
use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{GitHubBoardFields, document, github_projects_with_board};

/// The budget the copy journey below records telemetry for.
pub(super) const BUDGET: &str = "github-metadata-field-content-creation-share";

/// The projection every case configures: the host a follow-up ticket was filed from.
fn host_projection() -> Value {
    json!([{"field": "Host", "key": "orchestrator.follow-up", "path": ["host"]}])
}

/// A folder of Markdown beside the loopback board, the board's source projecting
/// `orchestrator.follow-up` at `host` onto `Host` when `projecting` says so.
struct Setup {
    sandbox: Sandbox,
    board: GitHubBoardFields,
    root: PathBuf,
}

impl Setup {
    fn new(projecting: bool, host_field: bool) -> Self {
        Self::projecting(projecting.then(host_projection), host_field)
    }

    /// The same, configuring `entries` as the source's `metadata_fields` when given.
    fn projecting(entries: Option<Value>, host_field: bool) -> Self {
        let sandbox = Sandbox::new();
        let root = sandbox.subdirectory("notes");
        std::fs::create_dir_all(root.join("tasks")).expect("the task folder");
        let (mut config, board) = github_projects_with_board(&sandbox);
        if let Some(entries) = entries {
            config["metadata_fields"] = entries;
        }
        if host_field {
            board.with_text_field("Host");
        }
        sandbox.project_document(&document(&json!({
            "notes": {"plugin": "local-md", "config": {"root": root}},
            "board": {"plugin": "github-projects", "config": config},
        })));
        Self {
            sandbox,
            board,
            root,
        }
    }

    /// Write the follow-up task `T-1`, carrying `host` — and nothing at that path for `None`.
    fn follow_up(&self, host: Option<&str>) {
        let host = host.map_or_else(String::new, |host| format!(", host: {host}"));
        std::fs::write(
            self.root.join("tasks/T-1.md"),
            format!(
                "---\ntitle: Disk filling on the build host\nstatus: todo\nmetadata: \
                 {{orchestrator.follow-up: {{run: r-1{host}}}}}\n---\nSeen twice tonight.\n"
            ),
        )
        .expect("the follow-up task");
    }

    /// Run one command to success, answering what it printed as JSON and every request the
    /// board was sent for it.
    fn run(&self, arguments: &[&str]) -> (Value, Vec<(String, Value)>) {
        let before = self.board.served().len();
        let output = self
            .sandbox
            .command()
            .args(arguments)
            .assert()
            .get_output()
            .clone();
        assert_eq!(
            output.status.code(),
            Some(0),
            "`onetaskgraph {}` failed\n{}",
            arguments.join(" "),
            stderr(&output)
        );
        let served = self.board.served()[before..].to_vec();
        (
            serde_json::from_str(&stdout(&output)).expect("a JSON answer"),
            served,
        )
    }

    /// Copy `T-1` onto the board, answering the board's id for it and the requests sent.
    fn copy(&self) -> (String, Vec<(String, Value)>) {
        let (report, served) = self.run(&["task", "copy", "notes:T-1", "--to", "board", "--json"]);
        let destination = report["items"][0]["destination"]
            .as_str()
            .and_then(|id| id.strip_prefix("board:"))
            .unwrap_or_else(|| panic!("a board destination: {report}"))
            .to_owned();
        (destination, served)
    }
}

/// Every mutation operation the requests carried, each aliased field of one request counted
/// on its own, because GitHub's content-creation limit may count each.
fn operations(served: &[(String, Value)]) -> usize {
    served
        .iter()
        .filter(|(query, _)| query.trim_start().starts_with("mutation"))
        .map(|(query, variables)| {
            if query == graphql::UPDATE_FIELDS {
                FIELD_WRITE_SLOTS
                    .iter()
                    .chain(&FIELD_CLEAR_SLOTS)
                    .filter(|slot| variables[slot.include] == true)
                    .count()
            } else {
                1
            }
        })
        .sum()
}

/// The modelled primary GraphQL points the requests spend.
fn points(served: &[(String, Value)]) -> u64 {
    served
        .iter()
        .map(|(query, _)| onetaskgraph_github_projects::worst_case_point_cost(query).unwrap())
        .sum()
}

/// The `Host` writes the requests carried: the text, or `None` for a clear.
fn host_writes(board: &GitHubBoardFields, served: &[(String, Value)]) -> Vec<Option<String>> {
    let host = board
        .field_list()
        .into_iter()
        .find(|field| field["name"] == "Host")
        .map(|field| field["id"].clone())
        .expect("the board has a Host field");
    let mut writes = Vec::new();
    for (query, variables) in served {
        if query == graphql::UPDATE_FIELDS {
            for slot in &FIELD_WRITE_SLOTS {
                if variables[slot.include] == true && variables[slot.variable]["fieldId"] == host {
                    writes.push(
                        variables[slot.variable]["value"]["text"]
                            .as_str()
                            .map(str::to_owned),
                    );
                }
            }
            for slot in &FIELD_CLEAR_SLOTS {
                if variables[slot.include] == true && variables[slot.variable]["fieldId"] == host {
                    writes.push(None);
                }
            }
        } else if variables["input"]["fieldId"] == host {
            writes.push(
                variables["input"]["value"]["text"]
                    .as_str()
                    .map(str::to_owned),
            );
        }
    }
    writes
}

#[test]
fn a_copied_follow_up_lands_its_host_on_the_board_and_keeps_it_in_step() {
    let setup = Setup::new(true, true);
    setup.follow_up(Some("build-7"));
    let (id, first) = setup.copy();
    assert_eq!(setup.board.text_of(&id, "Host").as_deref(), Some("build-7"));
    assert_eq!(
        host_writes(&setup.board, &first),
        [Some("build-7".to_owned())]
    );
    // It rides the one ordered field write a copy already sends.
    assert_eq!(
        first
            .iter()
            .filter(|(query, _)| query.trim_start().starts_with("mutation"))
            .map(|(query, _)| query.as_str())
            .collect::<Vec<_>>(),
        [
            graphql::CREATE_ISSUE,
            graphql::ADD_TO_BOARD,
            graphql::UPDATE_FIELDS
        ]
    );

    // An unchanged re-copy sends the field nothing; a moved host is written; a removed one
    // is cleared.
    let (again, recopied) = setup.copy();
    assert_eq!(again, id);
    assert_eq!(
        host_writes(&setup.board, &recopied),
        Vec::<Option<String>>::new()
    );
    setup.follow_up(Some("build-9"));
    let (_, moved) = setup.copy();
    assert_eq!(
        host_writes(&setup.board, &moved),
        [Some("build-9".to_owned())]
    );
    assert_eq!(setup.board.text_of(&id, "Host").as_deref(), Some("build-9"));
    setup.follow_up(None);
    let (_, cleared) = setup.copy();
    assert_eq!(host_writes(&setup.board, &cleared), [None]);
    assert_eq!(setup.board.text_of(&id, "Host"), None);

    // The metadata comment stays the value's home: a read reports it from there.
    setup.follow_up(Some("build-9"));
    setup.copy();
    let (shown, _) = setup.run(&[
        "task",
        "show",
        &format!("board:{id}"),
        "--no-comments",
        "--json",
    ]);
    assert_eq!(
        shown["items"][0]["item"]["metadata"]["orchestrator.follow-up"],
        json!({"run": "r-1", "host": "build-9"})
    );
}

#[test]
fn a_copy_onto_a_board_without_the_field_is_refused_naming_the_setup_that_creates_it() {
    let setup = Setup::new(true, false);
    setup.follow_up(Some("build-7"));
    let output = setup
        .sandbox
        .command()
        .args(["task", "copy", "notes:T-1", "--to", "board"])
        .assert()
        .failure()
        .get_output()
        .clone();
    let said = stderr(&output);
    for named in ["board", "\"Host\"", "sources fields board --apply"] {
        assert!(said.contains(named), "{named}: {said}");
    }
    assert!(
        !setup
            .board
            .served()
            .iter()
            .any(|(query, _)| query.trim_start().starts_with("mutation")),
        "refused before any mutation"
    );
}

#[test]
fn sources_fields_plans_creates_and_then_leaves_a_projected_text_field() {
    let setup = Setup::new(true, false);
    let status_before = setup.board.status_options();
    let priority_before = setup.board.priority_options();
    let mutations = |served: &[(String, Value)]| {
        served
            .iter()
            .filter(|(query, _)| query.trim_start().starts_with("mutation"))
            .cloned()
            .collect::<Vec<_>>()
    };

    let (planned, served) = setup.run(&["--json", "sources", "fields", "board"]);
    assert_eq!(
        planned["metadata_fields"],
        json!([{"field": "Host", "key": "orchestrator.follow-up", "path": ["host"],
                "exists": false, "outcome": "planned"}])
    );
    assert!(mutations(&served).is_empty(), "a plan writes nothing");

    let (applied, served) = setup.run(&["--json", "sources", "fields", "board", "--apply"]);
    assert_eq!(applied["metadata_fields"][0]["outcome"], "created");
    let sent = mutations(&served);
    assert_eq!(sent.len(), 1, "{sent:#?}");
    assert_eq!(sent[0].0, graphql::CREATE_FIELD);
    assert_eq!(
        sent[0].1["input"],
        json!({"projectId": "PVT-board", "dataType": "TEXT", "name": "Host"})
    );
    assert!(
        setup
            .board
            .field_list()
            .iter()
            .any(|field| field["name"] == "Host" && field["dataType"] == "TEXT")
    );
    // Every field that was there, and its options, as it was.
    assert_eq!(setup.board.status_options(), status_before);
    assert_eq!(setup.board.priority_options(), priority_before);

    let (again, served) = setup.run(&["--json", "sources", "fields", "board", "--apply"]);
    assert_eq!(again["metadata_fields"][0]["exists"], true);
    assert_eq!(again["metadata_fields"][0]["outcome"], "unchanged");
    assert!(mutations(&served).is_empty());
    let output = setup
        .sandbox
        .command()
        .args(["sources", "fields", "board"])
        .assert()
        .success()
        .get_output()
        .clone();
    assert!(
        stdout(&output)
            .contains("board: the Host text field for orchestrator.follow-up at host is there\n"),
        "{}",
        stdout(&output)
    );

    // And the copy the field was set up for now lands.
    setup.follow_up(Some("build-7"));
    let (id, _) = setup.copy();
    assert_eq!(setup.board.text_of(&id, "Host").as_deref(), Some("build-7"));
}

#[test]
fn sources_fields_reports_a_same_named_field_of_another_type_and_changes_nothing() {
    let setup = Setup::new(true, false);
    setup.board.with_persons_field(
        json!({"__typename": "ProjectV2Field", "id": "FIELD-host-number", "name": "Host",
               "dataType": "NUMBER"}),
    );
    let (applied, served) = setup.run(&["--json", "sources", "fields", "board", "--apply"]);
    assert_eq!(
        applied["metadata_fields"],
        json!([{"field": "Host", "key": "orchestrator.follow-up", "path": ["host"],
                "exists": false, "conflict": "NUMBER", "outcome": "unchanged"}])
    );
    assert!(
        !served
            .iter()
            .any(|(query, _)| query.trim_start().starts_with("mutation")),
        "a conflict is reported, never changed"
    );
    assert!(
        setup
            .board
            .field_list()
            .iter()
            .any(|field| field["id"] == "FIELD-host-number" && field["dataType"] == "NUMBER")
    );
    let output = setup
        .sandbox
        .command()
        .args(["sources", "fields", "board", "--apply"])
        .assert()
        .success()
        .get_output()
        .clone();
    assert!(
        stdout(&output).contains("board: the Host field is a NUMBER field, not a text field"),
        "{}",
        stdout(&output)
    );
}

/// What one copy kind came to, with and without the projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Spent {
    operations: usize,
    requests: usize,
    points: u64,
}

impl Spent {
    fn of(served: &[(String, Value)]) -> Self {
        Self {
            operations: operations(served),
            requests: served.len(),
            points: points(served),
        }
    }

    fn as_json(self) -> Value {
        json!({"operations": self.operations, "requests": self.requests, "points": self.points})
    }
}

/// One follow-up copied once and re-copied twice unchanged, with the projection configured or
/// not: what the first copy and each re-copy spent.
fn one_follow_up(projecting: bool) -> (Spent, [Spent; 2]) {
    let setup = Setup::new(projecting, true);
    setup.follow_up(Some("build-7"));
    let (id, first) = setup.copy();
    if projecting {
        assert_eq!(setup.board.text_of(&id, "Host").as_deref(), Some("build-7"));
    }
    let (_, second) = setup.copy();
    let (_, third) = setup.copy();
    (Spent::of(&first), [Spent::of(&second), Spent::of(&third)])
}

#[test]
fn projecting_a_field_adds_no_request_to_a_copy_and_records_what_it_spends() {
    let (first_with, unchanged_with) = one_follow_up(true);
    let (first_without, unchanged_without) = one_follow_up(false);
    // The one property this asserts: the write rides a request the copy already sends.
    assert_eq!(
        first_with.requests, first_without.requests,
        "a first copy: {first_with:?} against {first_without:?}"
    );
    for (with, without) in unchanged_with.iter().zip(&unchanged_without) {
        assert_eq!(
            with.requests, without.requests,
            "an unchanged re-copy: {with:?} against {without:?}"
        );
    }
    let directory = super::budget_runner::directory();
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        onetaskgraph_e2e_support::telemetry::file_in(&directory, BUDGET),
        serde_json::to_vec(&json!({
            "budget": BUDGET,
            "detail": "Mutation operations (each aliased field counted), HTTP requests and \
                       modelled primary points the real binary sent the loopback board for one \
                       follow-up copied once and re-copied twice unchanged",
            "workload": super::budget_runner::METADATA_FIELD_WORKLOAD,
            "first": {"with": first_with.as_json(), "without": first_without.as_json()},
            "unchanged": {
                "with": unchanged_with.map(Spent::as_json),
                "without": unchanged_without.map(Spent::as_json),
            },
        }))
        .unwrap(),
    )
    .unwrap();
}

/// `Host`, projected from a path, and `Team`, projected from a key's own value.
fn two_projections() -> Value {
    json!([
        {"field": "Host", "key": "orchestrator.follow-up", "path": ["host"]},
        {"field": "Team", "key": "team.name"},
    ])
}

#[test]
fn sources_fields_says_in_words_what_it_would_create_and_what_it_created() {
    let setup = Setup::projecting(Some(two_projections()), false);
    let said = |arguments: &[&str]| {
        let output = setup
            .sandbox
            .command()
            .args(arguments)
            .assert()
            .success()
            .get_output()
            .clone();
        stdout(&output)
    };
    let planned = said(&["sources", "fields", "board"]);
    for line in [
        "board: no Host text field; would create it for orchestrator.follow-up at host\n",
        "board: no Team text field; would create it for team.name\n",
    ] {
        assert!(planned.contains(line), "{line}{planned}");
    }
    let applied = said(&["sources", "fields", "board", "--apply"]);
    for line in [
        "board: created the Host text field for orchestrator.follow-up at host\n",
        "board: created the Team text field for team.name\n",
    ] {
        assert!(applied.contains(line), "{line}{applied}");
    }
}

#[test]
fn a_refused_text_field_create_names_what_landed_and_a_rerun_creates_only_what_is_missing() {
    let setup = Setup::projecting(Some(two_projections()), false);
    // Host is created; Team's create is refused.
    setup
        .board
        .refuse_after("createProjectV2Field(input:$input)", 1);
    let output = setup
        .sandbox
        .command()
        .args(["sources", "fields", "board", "--apply"])
        .assert()
        .failure()
        .get_output()
        .clone();
    let said = stderr(&output);
    for named in [
        "changed the Host field and then failed creating the Team text field",
        "run it again",
    ] {
        assert!(said.contains(named), "{named}: {said}");
    }
    let (rerun, served) = setup.run(&["--json", "sources", "fields", "board", "--apply"]);
    assert_eq!(rerun["metadata_fields"][0]["outcome"], "unchanged");
    assert_eq!(rerun["metadata_fields"][1]["outcome"], "created");
    let creates = served
        .iter()
        .filter(|(query, _)| query == graphql::CREATE_FIELD)
        .map(|(_, variables)| variables["input"]["name"].clone())
        .collect::<Vec<_>>();
    assert_eq!(creates, [json!("Team")]);
}

#[test]
fn a_created_text_field_the_board_does_not_hold_afterwards_is_refused_as_drift() {
    let setup = Setup::new(true, false);
    setup.board.omit_created_text_field();
    let output = setup
        .sandbox
        .command()
        .args(["sources", "fields", "board", "--apply"])
        .assert()
        .failure()
        .get_output()
        .clone();
    let said = stderr(&output);
    assert!(
        said.contains("GitHub changed the created Host text field after the guarded field setup"),
        "{said}"
    );
}

#[test]
fn a_verification_read_refused_after_a_create_says_what_changed_and_a_rerun_verifies_it() {
    let setup = Setup::new(true, false);
    // The setup's own read of the board's fields lands; the read verifying the create is
    // refused.
    setup.board.refuse_after("boardFields:repositoryOwner", 1);
    let output = setup
        .sandbox
        .command()
        .args(["sources", "fields", "board", "--apply"])
        .assert()
        .failure()
        .get_output()
        .clone();
    let said = stderr(&output);
    assert!(
        said.contains("changed the Host field and then could not read the board back to verify it"),
        "{said}"
    );
    let (rerun, served) = setup.run(&["--json", "sources", "fields", "board", "--apply"]);
    assert_eq!(rerun["metadata_fields"][0]["outcome"], "unchanged");
    assert!(
        !served
            .iter()
            .any(|(query, _)| query == graphql::CREATE_FIELD),
        "the field it created is not created again"
    );
}

#[test]
fn a_first_text_field_create_refused_changes_nothing_and_a_rerun_creates_them_all() {
    let setup = Setup::projecting(Some(two_projections()), false);
    let fields_before = setup.board.field_list();
    setup
        .board
        .refuse_once("createProjectV2Field(input:$input)");
    let output = setup
        .sandbox
        .command()
        .args(["sources", "fields", "board", "--apply"])
        .assert()
        .failure()
        .get_output()
        .clone();
    let said = stderr(&output);
    assert!(
        said.contains("the guarded field setup failed creating the Host text field"),
        "{said}"
    );
    assert!(!said.contains("changed the"), "nothing had landed: {said}");
    assert_eq!(
        setup.board.field_list(),
        fields_before,
        "the board is as it was"
    );
    let (rerun, served) = setup.run(&["--json", "sources", "fields", "board", "--apply"]);
    assert_eq!(rerun["metadata_fields"][0]["outcome"], "created");
    assert_eq!(rerun["metadata_fields"][1]["outcome"], "created");
    let creates = served
        .iter()
        .filter(|(query, _)| query == graphql::CREATE_FIELD)
        .map(|(_, variables)| variables["input"]["name"].clone())
        .collect::<Vec<_>>();
    assert_eq!(creates, [json!("Host"), json!("Team")]);
}
