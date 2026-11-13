//! A `github-projects` source's `metadata_fields`: a caller's metadata value projected onto a
//! board text field, through the real command-line boundary against the loopback board — the
//! copy that lands it, the field setup that creates the field, and the write allowance the
//! projection spends, recorded as telemetry for `github-metadata-field-content-creation-share`.

use std::path::PathBuf;

use onetaskgraph_github_projects::{FIELD_CLEAR_SLOTS, FIELD_WRITE_SLOTS, graphql};
use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{
    GITHUB_TEXT_FIELD_LIMIT, GitHubBoardFields, document, github_projects_with_board,
};

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
            } else if batched(query) {
                sent_slots(variables).len()
            } else {
                1
            }
        })
        .sum()
}

/// Whether `query` is one of a copy's aliased writes, whose `$send<n>` says which of its slots
/// it sent.
fn batched(query: &str) -> bool {
    [
        graphql::CREATE_ISSUES,
        graphql::WRITE_FIELD_VALUES,
        graphql::CLOSE_ISSUES,
        graphql::UPDATE_ISSUES,
        graphql::ADD_BLOCKED_BYS,
    ]
    .contains(&query)
}

/// The slots of one aliased write it sent, by number.
fn sent_slots(variables: &Value) -> Vec<usize> {
    (0..onetaskgraph_github_projects::MUTATION_BATCH)
        .filter(|slot| variables[format!("send{slot}")] == true)
        .collect()
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
    text_writes(board, served, "Host")
}

/// The writes to the text field called `name` the requests carried: the text, or `None` for a
/// clear.
fn text_writes(
    board: &GitHubBoardFields,
    served: &[(String, Value)],
    name: &str,
) -> Vec<Option<String>> {
    let host = board
        .field_list()
        .into_iter()
        .find(|field| field["name"] == name)
        .map(|field| field["id"].clone())
        .unwrap_or_else(|| panic!("the board has a {name} field"));
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
        } else if query == graphql::WRITE_FIELD_VALUES {
            for slot in sent_slots(variables) {
                let value = &variables[format!("field{slot}")];
                if value["fieldId"] == host {
                    writes.push(value["value"]["text"].as_str().map(str::to_owned));
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
    // It rides the one ordered field write a copy already sends, once the issue its create
    // filed on the board is there to write it on.
    assert_eq!(
        first
            .iter()
            .filter(|(query, _)| query.trim_start().starts_with("mutation"))
            .map(|(query, _)| query.as_str())
            .collect::<Vec<_>>(),
        [graphql::CREATE_ISSUE, graphql::WRITE_FIELD_VALUES]
    );

    // An unchanged re-copy sends the field nothing; a moved host is written; a removed one
    // is cleared.
    let (again, recopied) = setup.copy();
    assert_eq!(again, id);
    assert_eq!(
        host_writes(&setup.board, &recopied),
        Vec::<Option<String>>::new()
    );
    let (_, recopied_again) = setup.copy();
    assert_eq!(
        host_writes(&setup.board, &recopied_again),
        Vec::<Option<String>>::new()
    );
    // What this journey's first copy and two unchanged re-copies spent is the budget's
    // telemetry, recorded beside the same copies made without the projection.
    record_share(
        (
            Spent::of(&first),
            [Spent::of(&recopied), Spent::of(&recopied_again)],
        ),
        unprojected_follow_up(),
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

#[test]
fn sources_fields_names_a_conflict_without_a_data_type_by_its_graphql_type() {
    // A field GitHub answers with no `dataType` is named by its GraphQL type, and one with
    // neither is still a conflict rather than a text field.
    for (field, named) in [
        (
            json!({"__typename": "ProjectV2SingleSelectField", "id": "FIELD-host-select",
                   "name": "Host", "options": []}),
            json!("ProjectV2SingleSelectField"),
        ),
        (
            json!({"id": "FIELD-host-untyped", "name": "Host"}),
            json!("field of another type"),
        ),
    ] {
        let setup = Setup::new(true, false);
        setup.board.with_persons_field(field.clone());
        let (applied, served) = setup.run(&["--json", "sources", "fields", "board", "--apply"]);
        assert_eq!(
            applied["metadata_fields"],
            json!([{"field": "Host", "key": "orchestrator.follow-up", "path": ["host"],
                    "exists": false, "conflict": named, "outcome": "unchanged"}]),
            "{field}"
        );
        assert!(
            !served
                .iter()
                .any(|(query, _)| query.trim_start().starts_with("mutation")),
            "{field}: a conflict is reported, never changed"
        );
    }
}

#[test]
fn sources_fields_refuses_a_field_type_it_cannot_read_before_any_mutation() {
    // A blank `dataType` names no type at all: the setup fails as a malformed answer rather
    // than reporting a conflict it cannot name, and creates nothing.
    let setup = Setup::new(true, false);
    setup.board.with_persons_field(
        json!({"__typename": "ProjectV2Field", "id": "FIELD-host-blank", "name": "Host",
               "dataType": " "}),
    );
    let before = setup.board.served().len();
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
        said.contains("board's Host field with a type this setup cannot read"),
        "{said}"
    );
    assert!(said.contains("cannot be blank"), "{said}");
    assert!(
        !setup.board.served()[before..]
            .iter()
            .any(|(query, _)| query.trim_start().starts_with("mutation")),
        "a type it cannot read creates nothing"
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

/// The baseline the budget compares against: the same follow-up copied once and re-copied
/// twice unchanged onto a board with the `Host` field and no projection configured.
fn unprojected_follow_up() -> (Spent, [Spent; 2]) {
    let setup = Setup::new(false, true);
    setup.follow_up(Some("build-7"));
    let (_, first) = setup.copy();
    let (_, second) = setup.copy();
    let (_, third) = setup.copy();
    (Spent::of(&first), [Spent::of(&second), Spent::of(&third)])
}

/// Hold the one property the budget asserts — the projection rides a request the copy already
/// sends, for each copy kind — and record what each kind spent with and without it.
fn record_share(with: (Spent, [Spent; 2]), without: (Spent, [Spent; 2])) {
    let ((first_with, unchanged_with), (first_without, unchanged_without)) = (with, without);
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

/// The two projections a follow-up record carries as a list and a map: its owning runs and the
/// commit each repository was checked at.
fn record_projections() -> Value {
    json!([
        {"field": "Runs", "key": "orchestrator.follow-up", "path": ["owning_runs"]},
        {"field": "Basis", "key": "orchestrator.follow-up", "path": ["basis"]},
    ])
}

/// A folder of Markdown beside the loopback board, the board projecting
/// [`record_projections`] onto its `Runs` and `Basis` text fields.
fn record_setup() -> Setup {
    let setup = Setup::projecting(Some(record_projections()), false);
    setup.board.with_text_field("Runs");
    setup.board.with_text_field("Basis");
    setup
}

/// The kinds of item a value is projected from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Task,
    Project,
    Document,
}

impl Kind {
    const ALL: [Self; 3] = [Self::Task, Self::Project, Self::Document];

    fn noun(self) -> &'static str {
        match self {
            Self::Task => "task",
            Self::Project => "project",
            Self::Document => "document",
        }
    }

    /// The item's qualified id in the folder of Markdown.
    fn local(self) -> &'static str {
        match self {
            Self::Task => "notes:T-1",
            Self::Project => "notes:P",
            Self::Document => "notes:D",
        }
    }
}

impl Setup {
    /// Write the folder's item of `kind`, its `orchestrator.follow-up` record `record`.
    fn record(&self, kind: Kind, record: &Value) {
        let metadata = format!("metadata: {}\n", json!({"orchestrator.follow-up": record}));
        let (path, front) = match kind {
            Kind::Task => ("tasks/T-1.md", "title: Disk filling\nstatus: todo\n"),
            Kind::Project => ("projects/P.md", "title: Follow-ups\nstatus: todo\n"),
            Kind::Document => ("documents/D.md", "title: Design\nproject: P\n"),
        };
        std::fs::create_dir_all(self.root.join(path).parent().unwrap()).unwrap();
        std::fs::write(
            self.root.join(path),
            format!("---\n{front}{metadata}---\nSeen twice tonight.\n"),
        )
        .expect("the item");
    }

    /// Copy the folder's item of `kind` onto the board, answering the board's id for it and the
    /// requests sent. A document's project is copied first, once, outside what is answered.
    fn copy_kind(&self, kind: Kind) -> (String, Vec<(String, Value)>) {
        let mut arguments = vec![kind.noun(), "copy", kind.local(), "--to", "board", "--json"];
        if kind == Kind::Project {
            arguments.push("--no-tasks");
        }
        if kind == Kind::Document && !self.root.join("projects/P.md").exists() {
            std::fs::create_dir_all(self.root.join("projects")).unwrap();
            std::fs::write(
                self.root.join("projects/P.md"),
                "---\ntitle: Follow-ups\nstatus: todo\n---\nThe plan.\n",
            )
            .unwrap();
            self.run(&[
                "project",
                "copy",
                "notes:P",
                "--to",
                "board",
                "--no-tasks",
                "--json",
            ]);
        }
        let (report, served) = self.run(&arguments);
        let destination = report["items"]
            .as_array()
            .and_then(|items| items.iter().find(|item| item["source"] == kind.local()))
            .and_then(|item| item["destination"].as_str())
            .and_then(|id| id.strip_prefix("board:"))
            .unwrap_or_else(|| panic!("a board destination: {report}"))
            .to_owned();
        (destination, served)
    }

    /// What the board item `id` of `kind` reads back as its `orchestrator.follow-up` record.
    fn read_record(&self, kind: Kind, id: &str) -> Value {
        let id = format!("board:{id}");
        let mut arguments = vec![kind.noun(), "show", id.as_str(), "--json"];
        if kind == Kind::Task {
            arguments.push("--no-comments");
        }
        let (shown, _) = self.run(&arguments);
        let item = &shown["items"][0];
        let item = if item["item"].is_object() {
            &item["item"]
        } else {
            item
        };
        item["metadata"]["orchestrator.follow-up"].clone()
    }
}

/// One write a journey makes, and what the two projected fields then hold.
struct Step {
    /// What it is, as a failure names it.
    named: &'static str,
    /// The record the item holds after it.
    record: Value,
    /// What `Runs` and `Basis` hold after it, `None` for nothing.
    runs: Option<&'static str>,
    basis: Option<&'static str>,
}

/// The record as a list and a map, or as the strings that hold the same text.
fn shaped(lists: bool, runs: &[&str], basis: &[(&str, &str)]) -> Value {
    if lists {
        json!({
            "owning_runs": runs,
            "basis": basis.iter().map(|(k, v)| ((*k).to_owned(), json!(v))).collect::<serde_json::Map<_, _>>(),
        })
    } else {
        let mut sorted = basis.to_vec();
        sorted.sort_unstable();
        json!({
            "owning_runs": runs.join("\n"),
            "basis": sorted.iter().map(|(k, v)| format!("{k}: {v}")).collect::<Vec<_>>().join("\n"),
        })
    }
}

/// Every write that projects a value, of one kind, in order — each answering the requests it
/// sent and the writes to `Runs` and `Basis` among them — with the record as lists and maps or
/// as the strings that render the same.
fn every_write(setup: &Setup, kind: Kind, lists: bool) -> Vec<(Spent, [Vec<Option<String>>; 2])> {
    // The map is written out of byte order, upper case first in byte order, to show it sorted.
    let first = shaped(
        lists,
        &["run-b", "run-a"],
        &[("zeta.example/b", "2"), ("alpha", "3"), ("Alpha", "1")],
    );
    let moved = shaped(lists, &["run-c"], &[("github.com/acme/widget", "0123abc")]);
    let set = shaped(lists, &["run-d", "run-e"], &[("b", "2"), ("a", "1")]);
    let updated = shaped(lists, &["run-f"], &[("c", "3")]);
    let empty = shaped(lists, &[], &[]);
    let mut steps = vec![
        Step {
            named: "a create",
            record: first,
            runs: Some("run-b\nrun-a"),
            basis: Some("Alpha: 1\nalpha: 3\nzeta.example/b: 2"),
        },
        Step {
            named: "an update by a copy",
            record: moved.clone(),
            runs: Some("run-c"),
            basis: Some("github.com/acme/widget: 0123abc"),
        },
        Step {
            named: "an unchanged copy",
            record: moved,
            runs: Some("run-c"),
            basis: Some("github.com/acme/widget: 0123abc"),
        },
        Step {
            named: "metadata set",
            record: set,
            runs: Some("run-d\nrun-e"),
            basis: Some("a: 1\nb: 2"),
        },
    ];
    if kind == Kind::Task {
        steps.push(Step {
            named: "task update",
            record: updated,
            runs: Some("run-f"),
            basis: Some("c: 3"),
        });
    }
    steps.push(Step {
        named: "an empty list and map",
        record: empty,
        runs: Some(""),
        basis: Some(""),
    });
    let mut id = String::new();
    let mut spent = Vec::new();
    for step in &steps {
        let served = match step.named {
            "a create" | "an update by a copy" | "an unchanged copy" => {
                setup.record(kind, &step.record);
                let (copied, served) = setup.copy_kind(kind);
                id = copied;
                served
            }
            "task update" => {
                let value = format!("orchestrator.follow-up={}", step.record);
                let target = format!("board:{id}");
                setup
                    .run(&["task", "update", &target, "--metadata", &value, "--json"])
                    .1
            }
            _ => {
                let target = format!("board:{id}");
                let value = step.record.to_string();
                setup
                    .run(&[
                        kind.noun(),
                        "metadata",
                        "set",
                        &target,
                        "orchestrator.follow-up",
                        &value,
                        "--json",
                    ])
                    .1
            }
        };
        let (named, kind_named) = (step.named, kind.noun());
        assert_eq!(
            setup.board.text_of(&id, "Runs").as_deref(),
            step.runs,
            "{kind_named}: {named}"
        );
        assert_eq!(
            setup.board.text_of(&id, "Basis").as_deref(),
            step.basis,
            "{kind_named}: {named}"
        );
        // The item's metadata keeps the value whole, as it was written.
        assert_eq!(
            setup.read_record(kind, &id),
            step.record,
            "{kind_named}: {named}"
        );
        spent.push((
            Spent::of(&served),
            [
                text_writes(&setup.board, &served, "Runs"),
                text_writes(&setup.board, &served, "Basis"),
            ],
        ));
    }
    spent
}

#[test]
fn a_list_and_a_map_land_on_their_text_fields_on_every_write_of_every_kind() {
    for kind in Kind::ALL {
        let with_lists = every_write(&record_setup(), kind, true);
        let with_strings = every_write(&record_setup(), kind, false);
        let kind = kind.noun();
        // Each write sends the field what it changed and nothing else: an unchanged copy sends
        // neither field anything.
        let writes: Vec<_> = with_lists
            .iter()
            .map(|(_, writes)| writes.clone())
            .collect();
        let some = |text: &str| vec![Some(text.to_owned())];
        let mut expected = vec![
            [
                some("run-b\nrun-a"),
                some("Alpha: 1\nalpha: 3\nzeta.example/b: 2"),
            ],
            [some("run-c"), some("github.com/acme/widget: 0123abc")],
            [Vec::new(), Vec::new()],
            [some("run-d\nrun-e"), some("a: 1\nb: 2")],
        ];
        if kind == "task" {
            expected.push([some("run-f"), some("c: 3")]);
        }
        expected.push([some(""), some("")]);
        assert_eq!(writes, expected, "{kind}");
        // And each sends exactly the requests and operations the same write sends with the
        // strings that render the same text.
        for (at, ((lists, list_writes), (strings, string_writes))) in
            with_lists.iter().zip(&with_strings).enumerate()
        {
            assert_eq!(list_writes, string_writes, "{kind}: write {at}");
            assert_eq!(
                (lists.requests, lists.operations),
                (strings.requests, strings.operations),
                "{kind}: write {at}"
            );
        }
    }
}

/// `count` lines of exactly `width` characters, each a draft id as a follow-up record names one.
fn draft_lines(count: usize, width: usize) -> Vec<String> {
    (0..count)
        .map(|at| {
            let head = format!("drafts:run-{at:02}/drafts/20261112T1955{at:02}Z-");
            format!("{head}{}", "x".repeat(width - head.len()))
        })
        .collect()
}

#[test]
fn a_value_longer_than_the_text_field_is_written_as_its_leading_lines_and_a_count_of_the_rest() {
    let setup = Setup::projecting(
        Some(json!([{"field": "Drafts", "key": "orchestrator.follow-up", "path": ["drafts"]}])),
        false,
    );
    setup.board.with_text_field("Drafts");
    let drafts_writes = |served: &[(String, Value)]| text_writes(&setup.board, served, "Drafts");
    // The realistic size: the longest rendered list the live followups board held was 1,671
    // characters, about twelve draft ids of 140 characters.
    let realistic = draft_lines(12, 140);
    assert!(
        realistic.join("\n").len() > GITHUB_TEXT_FIELD_LIMIT,
        "realistic, and over"
    );
    // Six lines, a line feed and the line counting the six left out fit; a seventh would not.
    let elided = |left: usize| format!("\u{2026} {left} more lines in this item's metadata");
    let kept = format!("{}\n{}", realistic[..6].join("\n"), elided(6));
    assert!(kept.len() <= GITHUB_TEXT_FIELD_LIMIT);
    assert!(realistic[..7].join("\n").len() + 1 + elided(5).len() > GITHUB_TEXT_FIELD_LIMIT);
    // Exactly the limit, written whole: four lines of 255, 255, 255 and 256 and three line
    // feeds.
    let mut exact = draft_lines(4, 255);
    exact[3].push('y');
    assert_eq!(exact.join("\n").len(), GITHUB_TEXT_FIELD_LIMIT);
    // A single line over the limit: the counting line alone.
    let single = draft_lines(1, GITHUB_TEXT_FIELD_LIMIT + 1);
    // A map whose first line alone is over: the counting line alone, counting every line.
    let over_first = json!({"a": "x".repeat(GITHUB_TEXT_FIELD_LIMIT), "b": "short"});
    // The same lines as one string holding line breaks: shortened alike.
    let as_string = json!(realistic.join("\n"));
    // Two lines of 400 two-byte characters: 801 characters, which GitHub might take, but 1,601
    // UTF-8 bytes, which this source counts, so the second is left out.
    let wide = vec!["\u{e9}".repeat(400), "\u{e9}".repeat(400)];
    let wide_kept = format!(
        "{}\n\u{2026} 1 more line in this item's metadata",
        "\u{e9}".repeat(400)
    );
    for (value, field) in [
        (as_string, kept.clone()),
        (json!(wide), wide_kept),
        (json!(realistic), kept),
        (json!(exact), exact.join("\n")),
        (
            json!(single),
            "\u{2026} 1 more line in this item's metadata".to_owned(),
        ),
        (over_first, elided(2)),
    ] {
        let record = json!({"run": "r-1", "drafts": value});
        setup.record(Kind::Task, &record);
        let (id, served) = setup.copy_kind(Kind::Task);
        assert_eq!(drafts_writes(&served), [Some(field.clone())], "{value}");
        assert_eq!(
            setup.board.text_of(&id, "Drafts").as_deref(),
            Some(field.as_str())
        );
        assert_eq!(setup.read_record(Kind::Task, &id), record, "kept whole");
        // Unchanged, over-long included, it is not sent again.
        let (_, again) = setup.copy_kind(Kind::Task);
        assert_eq!(
            drafts_writes(&again),
            Vec::<Option<String>>::new(),
            "{value}"
        );
    }
}

#[test]
fn every_shape_no_text_field_can_hold_is_refused_before_any_mutation_naming_what_it_found() {
    for (runs, found) in [
        (json!(true), "a boolean".to_owned()),
        (json!(7), "a number".to_owned()),
        (
            json!(["run-a", 2]),
            "an array whose element 1 is a number".to_owned(),
        ),
        (
            json!(["run-a", ["run-b"]]),
            "an array whose element 1 is an array".to_owned(),
        ),
        (
            json!({"b": "x", "a": {"c": "d"}}),
            "an object whose value at key \"a\" is an object".to_owned(),
        ),
        (
            json!({"a": "x", "b": false}),
            "an object whose value at key \"b\" is a boolean".to_owned(),
        ),
    ] {
        let setup = record_setup();
        setup.record(Kind::Task, &json!({"owning_runs": runs, "basis": {}}));
        let output = setup
            .sandbox
            .command()
            .args(["task", "copy", "notes:T-1", "--to", "board"])
            .assert()
            .failure()
            .get_output()
            .clone();
        let said = stderr(&output);
        for named in [
            "source board",
            "\"orchestrator.follow-up\"",
            "[\"owning_runs\"]",
            "\"Runs\"",
            found.as_str(),
        ] {
            assert!(said.contains(named), "{runs}: {named}: {said}");
        }
        assert!(
            !setup
                .board
                .served()
                .iter()
                .any(|(query, _)| query.trim_start().starts_with("mutation")),
            "{runs}: refused before any mutation"
        );
    }
}
