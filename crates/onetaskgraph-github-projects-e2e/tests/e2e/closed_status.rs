//! A closed issue reads as its closure says, through the binary, against the loopback board.
//!
//! Whichever way an issue closes on GitHub — a person closing it, a change request merged with
//! a closing line, a landing's issue-closing line — every reader of it sees only its state and
//! the reason it was closed with, and its card stays where it was. So each journey here builds
//! a board of open issues, closes each one on the fixture the way GitHub records a closure, and
//! then reads it with nothing written in between: `task show`, `task list --status` and a
//! `task copy` into a folder of Markdown configured with the board's own status names all have
//! to agree that an issue closed as completed at `Todo` is done.

use std::path::PathBuf;
use std::process::Output;

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{GitHubBoardFields, document, github_projects_with_items};

/// A board of open issues, each at the `Status` option named, and a folder of Markdown called
/// `snapshot` whose mapping reads the board's own option names.
struct Setup {
    sandbox: Sandbox,
    board: GitHubBoardFields,
    snapshot: PathBuf,
}

/// One open task issue on the board, filed under no project, at `status`.
fn open_issue(id: &str, title: &str, status: &str) -> Value {
    json!({"item":format!("ITEM-{id}"),"id":id,"type":"Issue","title":title,
           "body":format!("{title}, as filed"),"state":"OPEN","reason":null,"parent":null,
           "repo":"nickderobertis/onetaskgraph","status":status,"origin":"","labels":[]})
}

impl Setup {
    fn new(items: Vec<Value>) -> Self {
        Self::mapped(items, json!({"todo":"Todo","in-progress":"In Progress"}))
    }

    /// The same setup with the board's `status_mapping` set to `mapping`.
    fn mapped(items: Vec<Value>, mapping: Value) -> Self {
        let sandbox = Sandbox::new();
        let snapshot = sandbox.subdirectory("snapshot");
        std::fs::create_dir_all(snapshot.join("tasks")).expect("the snapshot's task folder");
        let (mut config, board) = github_projects_with_items(&sandbox, items);
        board.with_status_option("OPT-in-progress", "In Progress");
        config["status_mapping"] = mapping;
        sandbox.project_document(&document(&json!({
            "board": {"plugin":"github-projects","config":config},
            "snapshot": {"plugin":"local-md","config":{"root":snapshot,"status_mapping":{
                "Todo":"todo","In Progress":"in-progress","Done":"done",
                "Cancelled":"cancelled","Shipped":"done"}}}
        })));
        Self {
            sandbox,
            board,
            snapshot,
        }
    }

    fn run(&self, arguments: &[&str]) -> Output {
        self.sandbox
            .command()
            .args(arguments)
            .assert()
            .get_output()
            .clone()
    }

    fn json(&self, arguments: &[&str]) -> Value {
        let output = self.run(arguments);
        assert_eq!(
            output.status.code(),
            Some(0),
            "`onetaskgraph {}` failed\n{}",
            arguments.join(" "),
            stderr(&output)
        );
        serde_json::from_str(&stdout(&output)).expect("the command emits JSON")
    }

    /// The status `task show` reads for one board task.
    fn shown(&self, id: &str) -> Value {
        self.json(&["--json", "task", "show", &format!("board:{id}")])["items"][0]["item"]["status"]
            .clone()
    }

    /// The board tasks `task list --status <category>` keeps, by native id.
    fn listed(&self, category: &str) -> Vec<String> {
        self.json(&[
            "--json", "task", "list", "--source", "board", "--status", category,
        ])["items"]
            .as_array()
            .expect("a page")
            .iter()
            .map(|item| {
                item["id"]
                    .as_str()
                    .and_then(|id| id.strip_prefix("board:"))
                    .expect("a board id")
                    .to_owned()
            })
            .collect()
    }

    /// Copy one board task into the snapshot, answering the `status:` line of the file it
    /// landed in.
    fn copied_status(&self, id: &str) -> String {
        let copied = self.json(&[
            "--json",
            "task",
            "copy",
            &format!("board:{id}"),
            "--to",
            "snapshot",
        ]);
        let landed = copied["items"][0]["destination"]
            .as_str()
            .and_then(|id| id.strip_prefix("snapshot:"))
            .unwrap_or_else(|| panic!("{id} landed in the snapshot: {copied:#}"))
            .to_owned();
        let text = std::fs::read_to_string(self.snapshot.join(format!("tasks/{landed}.md")))
            .expect("the copied task's file");
        text.lines()
            .find_map(|line| line.strip_prefix("status: "))
            .unwrap_or_else(|| panic!("{id}'s copy has a status:\n{text}"))
            .trim_matches('"')
            .to_owned()
    }
}

#[test]
fn an_issue_closed_as_completed_at_todo_reads_and_copies_as_done_named_by_the_done_option() {
    let setup = Setup::new(vec![
        open_issue("C-1", "Landed with a reason", "Todo"),
        open_issue("C-2", "Landed before reasons", "Todo"),
        open_issue("C-6", "Still open", "Todo"),
    ]);
    setup.board.close("C-1", Some("COMPLETED"));
    setup.board.close("C-2", None);

    for id in ["C-1", "C-2"] {
        assert_eq!(
            setup.shown(id),
            json!({"category":"done","name":"Done"}),
            "{id}: closed as completed, so done, named by the board's `done` option"
        );
        assert_eq!(
            setup.board.status(id).as_deref(),
            Some("Todo"),
            "{id}: unwritten"
        );
    }
    assert_eq!(
        setup.shown("C-6"),
        json!({"category":"todo","name":"Todo"}),
        "an open issue reads exactly as before"
    );
    let done = setup.listed("done");
    let todo = setup.listed("todo");
    for id in ["C-1", "C-2"] {
        assert!(done.contains(&id.to_owned()), "{id} is done: {done:?}");
        assert!(!todo.contains(&id.to_owned()), "{id} is not todo: {todo:?}");
    }
    assert!(todo.contains(&"C-6".to_owned()), "{todo:?}");
    assert!(!done.contains(&"C-6".to_owned()), "{done:?}");

    assert_eq!(setup.copied_status("C-1"), "Done");
    assert_eq!(setup.copied_status("C-2"), "Done");
    assert_eq!(setup.copied_status("C-6"), "Todo");
    assert_eq!(
        setup.board.state("C-1"),
        ("CLOSED".to_owned(), Some("COMPLETED".to_owned())),
        "nothing wrote the issue"
    );
}

#[test]
fn an_issue_closed_as_not_planned_or_duplicate_in_progress_reads_and_copies_as_cancelled() {
    let setup = Setup::new(vec![
        open_issue("C-3", "Dropped", "In Progress"),
        open_issue("C-4", "Duplicated", "In Progress"),
    ]);
    setup.board.close("C-3", Some("NOT_PLANNED"));
    setup.board.close("C-4", Some("DUPLICATE"));

    let cancelled = setup.listed("cancelled");
    let in_progress = setup.listed("in-progress");
    for id in ["C-3", "C-4"] {
        assert_eq!(
            setup.shown(id),
            json!({"category":"cancelled","name":"Cancelled"}),
            "{id}: named by the board's `cancelled` option"
        );
        assert!(cancelled.contains(&id.to_owned()), "{cancelled:?}");
        assert!(!in_progress.contains(&id.to_owned()), "{in_progress:?}");
        assert_eq!(setup.copied_status(id), "Cancelled");
        assert_eq!(
            setup.board.status(id).as_deref(),
            Some("In Progress"),
            "{id}: its card is not written"
        );
    }
}

#[test]
fn an_issue_closed_as_completed_in_an_option_the_mapping_does_not_name_keeps_that_name() {
    let setup = Setup::new(vec![open_issue("C-5", "Shipped already", "Shipped")]);
    setup.board.close("C-5", Some("COMPLETED"));
    assert_eq!(
        setup.shown("C-5"),
        json!({"category":"done","name":"Shipped"})
    );
    assert!(setup.listed("done").contains(&"C-5".to_owned()));
    assert_eq!(setup.copied_status("C-5"), "Shipped");
}

/// One project issue on the board — open, at `status` — whose issue holds `sub` as its one
/// sub-issue, which is what makes it a project item; and that sub-issue, an open task at `Todo`.
fn home_with_sub_issue(id: &str, sub: &str, status: &str) -> [Value; 2] {
    let mut task = open_issue(sub, &format!("Task of {id}"), "Todo");
    task["parent"] = json!(id);
    [open_issue(id, &format!("Plan {id}"), status), task]
}

/// A board for the reconcile journeys: a project closed as completed at `In Progress` (`H-1`), a
/// project closed as not planned at `Todo` (`H-2`), a task closed as completed at `Todo` (`C-7`),
/// an open project at `In Progress` (`H-3`) and a project closed as completed already at `Done`
/// (`H-4`) — the project kind's `cancelled` mapped to its own `Abandoned` option, so each card's
/// destination is its own kind's.
fn reconcile_setup() -> Setup {
    let mut items = Vec::new();
    items.extend(home_with_sub_issue("H-1", "S-1", "In Progress"));
    items.extend(home_with_sub_issue("H-2", "S-2", "Todo"));
    items.push(open_issue("C-7", "Closed against its card", "Todo"));
    items.extend(home_with_sub_issue("H-3", "S-3", "In Progress"));
    items.extend(home_with_sub_issue("H-4", "S-4", "Done"));
    let setup = Setup::mapped(
        items,
        json!({"todo":"Todo","in-progress":"In Progress",
               "cancelled":{"task":"Cancelled","project":"Abandoned"}}),
    );
    setup.board.with_status_option("OPT-abandoned", "Abandoned");
    // The shared dataset's one closed task sits at an option the mapping does not name;
    // reopened, so every closed item on this board is one this journey put there.
    setup.board.reopen("T-2");
    setup.board.close("H-1", Some("COMPLETED"));
    setup.board.close("H-2", Some("NOT_PLANNED"));
    setup.board.close("C-7", Some("COMPLETED"));
    setup.board.close("H-4", Some("COMPLETED"));
    setup
}

/// Every item [`reconcile_setup`] holds, by id.
const RECONCILED: [&str; 9] = [
    "H-1", "S-1", "H-2", "S-2", "C-7", "H-3", "S-3", "H-4", "S-4",
];

/// What a reconcile of [`reconcile_setup`] owes, each item at `action`.
fn reconcile_owed(action: &str) -> Value {
    json!({"items": [
        {"source":"board:H-1","kind":"project","from":"In Progress",
         "to":{"category":"done","name":"Done"},"action":action},
        {"source":"board:H-2","kind":"project","from":"Todo",
         "to":{"category":"cancelled","name":"Abandoned"},"action":action},
        {"source":"board:C-7","kind":"task","from":"Todo",
         "to":{"category":"done","name":"Done"},"action":action},
    ]})
}

impl Setup {
    /// Each item's card and its issue's state and reason.
    fn held(&self, ids: &[&str]) -> Vec<(String, Option<String>, (String, Option<String>))> {
        ids.iter()
            .map(|id| {
                (
                    (*id).to_owned(),
                    self.board.status(id),
                    self.board.state(id),
                )
            })
            .collect()
    }

    /// Every request the board was sent: each read's document, and each mutation as its root
    /// field and the board item it writes.
    fn requests(&self) -> (Vec<String>, Vec<(String, Value)>) {
        let (mutations, reads): (Vec<_>, Vec<_>) = self
            .board
            .served()
            .into_iter()
            .partition(|(document, _)| document.trim_start().starts_with("mutation"));
        (
            reads.into_iter().map(|(document, _)| document).collect(),
            mutations
                .into_iter()
                .map(|(document, variables)| {
                    if document.contains("updateProjectV2ItemFieldValue(input:") {
                        ("option".to_owned(), variables["input"]["itemId"].clone())
                    } else {
                        (document, variables)
                    }
                })
                .collect(),
        )
    }
}

#[test]
fn a_reconcile_brings_each_closed_project_and_task_card_in_line_with_its_own_kinds_option() {
    let setup = reconcile_setup();
    let before = setup.held(&RECONCILED);
    let answer = setup.json(&["--json", "task", "status", "reconcile", "--source", "board"]);
    assert_eq!(answer, reconcile_owed("updated"), "{answer:#}");

    let after = setup.held(&RECONCILED);
    for ((id, card, state), (_, was, was_state)) in after.iter().zip(&before) {
        let owed = match id.as_str() {
            "H-1" | "C-7" => Some("Done".to_owned()),
            "H-2" => Some("Abandoned".to_owned()),
            _ => was.clone(),
        };
        assert_eq!(card, &owed, "{id}'s card");
        assert_eq!(state, was_state, "{id}'s issue keeps its state and reason");
    }
    let (reads, mutations) = setup.requests();
    assert!(!reads.is_empty(), "the reconcile read the board");
    assert!(
        reads
            .iter()
            .all(|read| read == onetaskgraph_github_projects::graphql::CLOSED_ITEMS),
        "only the board's is:closed filtered read, never its unfiltered items: {reads:#?}"
    );
    assert_eq!(
        mutations,
        vec![
            ("option".to_owned(), json!("ITEM-H-1")),
            ("option".to_owned(), json!("ITEM-H-2")),
            ("option".to_owned(), json!("ITEM-C-7")),
        ],
        "one option write per listed item, and no write of any issue"
    );

    let human = reconcile_setup();
    let output = human.run(&["task", "status", "reconcile", "--source", "board"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let said = stdout(&output);
    assert!(
        said.contains("project board:H-1: In Progress -> Done (done): updated")
            && said.contains("project board:H-2: Todo -> Abandoned (cancelled): updated")
            && said.contains("\nboard:C-7: Todo -> Done (done): updated")
            && said.contains("3 updated, 0 would update, 0 failed"),
        "{said}"
    );
}

#[test]
fn a_dry_run_reconcile_answers_would_update_for_the_same_items_and_writes_nothing() {
    let setup = reconcile_setup();
    let before = setup.held(&RECONCILED);
    let answer = setup.json(&[
        "--json",
        "task",
        "status",
        "reconcile",
        "--source",
        "board",
        "--dry-run",
    ]);
    assert_eq!(answer, reconcile_owed("would-update"), "{answer:#}");
    assert_eq!(setup.held(&RECONCILED), before);
    let (reads, mutations) = setup.requests();
    assert!(
        mutations.is_empty(),
        "a dry run writes nothing: {mutations:?}"
    );
    assert!(
        reads
            .iter()
            .all(|read| read == onetaskgraph_github_projects::graphql::CLOSED_ITEMS),
        "{reads:#?}"
    );
}

#[test]
fn the_library_reconcile_answers_what_the_command_line_answers() {
    let cli = reconcile_setup();
    let answer = cli.json(&["--json", "task", "status", "reconcile", "--source", "board"]);

    let library = reconcile_setup();
    let environment = onetaskgraph_core::Environment::from_pairs([(
        "XDG_CONFIG_HOME",
        library.sandbox.config_home().to_string_lossy().into_owned(),
    )]);
    let loaded = onetaskgraph_core::config::load(
        &library.sandbox.project(),
        &environment,
        &onetaskgraph_core::config::Layer::default(),
    )
    .expect("the configuration loads");
    let engine = onetaskgraph_core::Engine::build(&loaded.config, &loaded.secrets);
    let board = onetaskgraph_plugin_api::SourceName::new("board").expect("a name");
    let reconciled = tokio::runtime::Runtime::new()
        .expect("a runtime")
        .block_on(engine.reconcile_statuses(&board, false))
        .expect("the reconcile runs")
        .expect("a board declares reconcile_status");
    assert_eq!(
        reconciled
            .items
            .iter()
            .map(|item| item.kind)
            .collect::<Vec<_>>(),
        [
            onetaskgraph_plugin_api::ItemKind::Project,
            onetaskgraph_plugin_api::ItemKind::Project,
            onetaskgraph_plugin_api::ItemKind::Task,
        ]
    );
    assert_eq!(
        serde_json::to_value(&reconciled).expect("it serialises"),
        answer
    );
    assert_eq!(library.board.status("H-2").as_deref(), Some("Abandoned"));
}
