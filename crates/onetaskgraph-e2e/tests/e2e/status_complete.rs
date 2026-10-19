//! `task status complete` across every kind of configured source, through the binary — and its
//! library form, `Engine::complete_issues`, answering the same.
//!
//! A landing's closures reach the store by issue reference, naming no source: the store decides
//! from its own configuration which sources can hold an item for an issue. So each journey here
//! configures one loopback GitHub serving two boards as two sources, a folder of Markdown, a
//! loopback Linear workspace and a stdio plugin declaring no `complete_by_issue` — with the
//! default selection naming the second board and the folder, so the board holding most of the
//! named issues is one no default selects — and reads every one of their request records.

use std::path::{Path, PathBuf};
use std::process::Output;

use onetaskgraph_core::{Config, Engine, EngineError, Environment, Secrets};
use onetaskgraph_plugin_api::{IssueRef, SourceName};
use serde_json::{Value, json};

use crate::common::{Sandbox, SourceBoundary, stderr, stdout};
use crate::document_store::store_at;
use crate::fixtures::{
    GITHUB_SECOND_BOARD_TOKEN, GitHubBoardFields, LinearWorkspace, ReferenceFault, dataset,
    github_number, github_projects_with_items, linear_workspace,
};

/// The repository every fixture issue lives in.
const REPOSITORY: &str = "nickderobertis/onetaskgraph";

/// The credential every loopback here accepts, in one secrets file.
const SECRETS: &str = "GITHUB_PROJECTS_FIXTURE_TOKEN=test-token\n\
                       GITHUB_PROJECTS_FIXTURE_TOKEN_B=test-token-b\n\
                       LINEAR_API_KEY=fixture-key\n";

/// The reference a landing's `closes` names the issue `id` by.
fn reference(id: &str) -> String {
    format!("{REPOSITORY}#{}", github_number(id))
}

/// One task issue filed under no project, its card at `status`.
fn issue(id: &str, status: &str) -> Value {
    json!({"item":format!("ITEM-{id}"),"id":id,"type":"Issue","title":format!("Task {id}"),
           "body":format!("{id}, as filed"),"state":"OPEN","reason":null,"parent":null,
           "repo":REPOSITORY,"status":status,"origin":"","labels":[]})
}

/// One task issue filed as a sub-issue of `parent`, its card at `status`.
fn sub_issue(id: &str, parent: &str, status: &str) -> Value {
    let mut item = issue(id, status);
    item["parent"] = json!(parent);
    item
}

/// A plan's home on board A — `N-9`, its issue open at `In Progress`, holding the plan's two
/// task issues `N-10` and `N-11` as sub-issues, which is what makes it a project item.
const HOME: &str = "N-9";

/// The references every journey below names, in this order: an open issue at `Todo` and one at
/// `In Progress`; one closed as completed at `Todo` and one at `Done`; one closed as not planned
/// and one as duplicate; an issue no board holds a card for; the first again; and an open issue
/// at `Todo` on both boards.
const NAMED: [&str; 9] = [
    "N-1", "N-2", "N-3", "N-4", "N-5", "N-6", "N-7", "N-1", "N-8",
];

/// One issue as the server holds it: its id, its card on each board, and its state and reason.
type Cards = (
    String,
    Option<String>,
    Option<String>,
    (String, Option<String>),
);

/// The configuration, its sandbox and a handle on every store in it.
struct Stores {
    sandbox: Sandbox,
    board: GitHubBoardFields,
    linear: LinearWorkspace,
    notes: PathBuf,
    plugin_log: PathBuf,
    /// The sources block, which the library journey builds its engine from.
    sources: Value,
}

impl Stores {
    fn new() -> Self {
        let sandbox = Sandbox::new();
        let items = vec![
            issue("N-1", "Todo"),
            issue("N-2", "In Progress"),
            issue("N-3", "Todo"),
            issue("N-4", "Done"),
            issue("N-5", "Todo"),
            issue("N-6", "Todo"),
            issue("N-7", "Todo"),
            issue("N-8", "Todo"),
            issue("F-1", "Todo"),
            issue("F-2", "In Progress"),
            issue(HOME, "In Progress"),
            sub_issue("N-10", HOME, "Todo"),
            sub_issue("N-11", HOME, "Todo"),
        ];
        let (mut first, board) = github_projects_with_items(&sandbox, items);
        board.with_status_option("OPT-in-progress", "In Progress");
        board.close("N-3", Some("COMPLETED"));
        board.close("N-4", Some("COMPLETED"));
        board.close("N-5", Some("NOT_PLANNED"));
        board.close("N-6", Some("DUPLICATE"));
        board.take_off_the_board("N-7");
        board.put_on_second_board("N-8", Some("Todo"));
        board.put_on_second_board("F-1", Some("Todo"));
        board.put_on_second_board("F-2", Some("In Progress"));
        let mapping = json!({"todo":"Todo","in-progress":"In Progress"});
        first["status_mapping"] = mapping.clone();
        let mut second = board.second_board();
        second["status_mapping"] = mapping;
        let (linear_config, linear) = linear_workspace(&sandbox, dataset());
        sandbox.secrets_file(SECRETS);
        let notes = sandbox.subdirectory("notes");
        std::fs::create_dir_all(notes.join("tasks")).expect("the task folder");
        std::fs::write(
            notes.join("tasks/T-1.md"),
            "---\ntitle: A note\nstatus: todo\n---\nKept.\n",
        )
        .expect("a note");
        let plugin_log = sandbox.subdirectory("plugin").join("requests.log");
        let store = sandbox.subdirectory("plugin").join("store.json");
        std::fs::write(&store, "[]").expect("the plugin's store");
        let sources = json!({
            "board-a": {"plugin": "github-projects", "config": first},
            "board-b": {"plugin": "github-projects", "config": second},
            "docs": store_at(&store, Some(&plugin_log), "native"),
            "linear": {"plugin": "linear", "config": linear_config},
            "notes": {"plugin": "local-md", "config": {"root": notes}},
        });
        sandbox.project_document(
            &serde_json::to_string_pretty(&json!({
                "sources": sources,
                "default_sources": ["board-b", "notes"],
            }))
            .expect("a configuration renders"),
        );
        Self {
            sandbox,
            board,
            linear,
            notes,
            plugin_log,
            sources,
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

    /// `task status complete --json` for `references` and the `extra` arguments, asserting it
    /// exits `code`.
    fn complete(&self, references: &[String], extra: &[&str], code: i32) -> Value {
        let mut arguments = vec!["--json", "task", "status", "complete"];
        for reference in references {
            arguments.push("--issue");
            arguments.push(reference);
        }
        arguments.extend(extra);
        let output = self.run(&arguments);
        assert_eq!(
            output.status.code(),
            Some(code),
            "`onetaskgraph {}`\n{}\n{}",
            arguments.join(" "),
            stdout(&output),
            stderr(&output)
        );
        serde_json::from_str(&stdout(&output)).expect("the command emits JSON")
    }

    /// Every method the stdio plugin was asked for, in order.
    fn plugin_requests(&self) -> Vec<String> {
        std::fs::read_to_string(&self.plugin_log)
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// The folder of Markdown, file by file.
    fn notes(&self) -> Vec<(PathBuf, String)> {
        files(&self.notes)
    }

    /// Every card on either board and every issue's state, by id.
    fn boards(&self) -> Vec<Cards> {
        [
            "N-1", "N-2", "N-3", "N-4", "N-5", "N-6", "N-7", "N-8", "F-1", "F-2",
        ]
        .into_iter()
        .map(|id| {
            (
                id.to_owned(),
                self.board.status(id),
                self.board.second_board_status(id),
                self.board.state(id),
            )
        })
        .collect()
    }

    /// That `token`'s board was sent exactly one read, of the named issues — no read of a
    /// board's items, no search, no project read.
    fn one_read_of_the_named_issues(&self, token: &str) {
        let reads: Vec<String> = self
            .board
            .served_to(token)
            .into_iter()
            .map(|(document, _)| document)
            .filter(|document| !document.trim_start().starts_with("mutation"))
            .collect();
        assert_eq!(
            reads,
            vec![onetaskgraph_github_projects::graphql::ISSUES_BY_REFERENCE.to_owned()],
            "{token}: one read of the named issues and nothing else"
        );
    }
}

/// Every file under `root`, with its bytes.
fn files(root: &Path) -> Vec<(PathBuf, String)> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory).expect("a readable folder") {
            let path = entry.expect("an entry").path();
            if path.is_dir() {
                pending.push(path);
            } else {
                found.push((
                    path.clone(),
                    std::fs::read_to_string(&path).expect("a readable file"),
                ));
            }
        }
    }
    found.sort();
    found
}

/// The references [`NAMED`] spells.
fn named() -> Vec<String> {
    NAMED.iter().map(|id| reference(id)).collect()
}

/// The machine output the full completion owes, with `updated` as `action` for every item it
/// writes.
fn owed(updated: &str) -> Value {
    let item = |id: &str, source: &str, task: Option<&str>, from: Value, to: Value, action| {
        json!({"issue":reference(id),"source":source,
               "task":task.map(|task| format!("{source}:{task}")),"kind":"task",
               "from":from,"to":to,"action":action})
    };
    let done = json!({"category":"done","name":"Done"});
    let mut skipped_n5 = item(
        "N-5",
        "board-a",
        Some("N-5"),
        json!("Todo"),
        Value::Null,
        "skipped",
    );
    skipped_n5["reason"] = json!("closed as not planned");
    let mut skipped_n6 = item(
        "N-6",
        "board-a",
        Some("N-6"),
        json!("Todo"),
        Value::Null,
        "skipped",
    );
    skipped_n6["reason"] = json!("closed as duplicate");
    let absent =
        |id: &str, source: &str| item(id, source, None, Value::Null, Value::Null, "absent");
    json!({
        "items": [
            item("N-1", "board-a", Some("N-1"), json!("Todo"), done.clone(), updated),
            item("N-2", "board-a", Some("N-2"), json!("In Progress"), done.clone(), updated),
            item("N-3", "board-a", Some("N-3"), json!("Todo"), done.clone(), updated),
            item("N-4", "board-a", Some("N-4"), json!("Done"), done.clone(), "unchanged"),
            skipped_n5,
            skipped_n6,
            absent("N-7", "board-a"),
            item("N-8", "board-a", Some("N-8"), json!("Todo"), done.clone(), updated),
            absent("N-1", "board-b"),
            absent("N-2", "board-b"),
            absent("N-3", "board-b"),
            absent("N-4", "board-b"),
            absent("N-5", "board-b"),
            absent("N-6", "board-b"),
            absent("N-7", "board-b"),
            item("N-8", "board-b", Some("N-8"), json!("Todo"), done, updated),
        ],
        "unsupported": ["docs", "linear", "notes"],
    })
}

#[test]
fn a_completion_naming_no_source_asks_every_board_the_configuration_holds_and_no_other_source() {
    let stores = Stores::new();
    let notes = stores.notes();
    let before = stores.boards();
    let answer = stores.complete(&named(), &[], 0);
    assert_eq!(answer, owed("updated"), "{answer:#}");

    for (id, card, second, state) in before {
        let (card, second, state) = match id.as_str() {
            "N-1" | "N-2" => (
                Some("Done".to_owned()),
                second,
                ("CLOSED".to_owned(), Some("COMPLETED".to_owned())),
            ),
            "N-3" => (Some("Done".to_owned()), second, state),
            "N-8" => (
                Some("Done".to_owned()),
                Some("Done".to_owned()),
                ("CLOSED".to_owned(), Some("COMPLETED".to_owned())),
            ),
            _ => (card, second, state),
        };
        assert_eq!(stores.board.status(&id), card, "{id}'s card on board A");
        assert_eq!(
            stores.board.second_board_status(&id),
            second,
            "{id}'s card on board B"
        );
        assert_eq!(stores.board.state(&id), state, "{id}'s issue");
    }
    // One call closes an issue at most once: board A's write closed N-8, and board B read it
    // closed as completed and moved only its card.
    let closes: Vec<Value> = stores
        .board
        .served()
        .into_iter()
        .filter(|(document, _)| document.contains("updateIssue(input:"))
        .map(|(_, variables)| variables["input"]["id"].clone())
        .collect();
    assert_eq!(closes, vec![json!("N-1"), json!("N-2"), json!("N-8")]);
    stores.one_read_of_the_named_issues("test-token");
    stores.one_read_of_the_named_issues(GITHUB_SECOND_BOARD_TOKEN);
    assert_eq!(stores.linear.served(), Vec::new(), "Linear is sent nothing");
    assert_eq!(
        stores.plugin_requests(),
        vec!["initialize"],
        "the plugin is sent nothing after its handshake"
    );
    assert_eq!(stores.notes(), notes, "the folder of Markdown is unchanged");

    let human = stores.run(&["task", "status", "complete", "--issue", &reference("N-1")]);
    let said = stdout(&human);
    assert!(
        said.contains(&format!("{} in board-a: unchanged", reference("N-1")))
            && said.contains(
                "unsupported (status is its closure, or no complete_by_issue): docs, linear, notes"
            )
            && said
                .contains("0 updated, 0 would update, 1 unchanged, 0 skipped, 1 absent, 0 failed"),
        "{said}"
    );
}

#[test]
fn a_completion_narrowed_to_one_board_asks_that_board_alone() {
    let stores = Stores::new();
    let answer = stores.complete(&named(), &["--source", "board-a"], 0);
    let owed = owed("updated");
    let board_a: Vec<Value> = owed["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["source"] == "board-a")
        .cloned()
        .collect();
    assert_eq!(answer, json!({"items": board_a, "unsupported": []}));
    assert!(stores.board.served_to(GITHUB_SECOND_BOARD_TOKEN).is_empty());
    assert_eq!(stores.linear.served(), Vec::new());
    assert_eq!(stores.plugin_requests(), vec!["initialize"]);
    assert_eq!(
        stores.board.second_board_status("N-8").as_deref(),
        Some("Todo"),
        "board B was not asked"
    );
}

#[test]
fn a_dry_run_answers_would_update_for_the_same_items_and_changes_neither_board() {
    let stores = Stores::new();
    let before = stores.boards();
    let answer = stores.complete(&named(), &["--dry-run"], 0);
    assert_eq!(answer, owed("would-update"), "{answer:#}");
    assert_eq!(stores.boards(), before);
    assert!(
        !stores
            .board
            .served()
            .iter()
            .any(|(document, _)| document.trim_start().starts_with("mutation")),
        "a dry run writes nothing"
    );
}

#[test]
fn a_reference_whose_write_is_refused_is_answered_failed_and_the_others_are_written() {
    let stores = Stores::new();
    stores
        .board
        .refuse_once("updateProjectV2ItemFieldValue(input:$input)");
    let answer = stores.complete(&named(), &[], 4);
    let items = answer["items"].as_array().expect("items");
    assert_eq!(items[0]["action"], "failed", "{answer:#}");
    assert_eq!(items[0]["error"]["class"], "refused", "{answer:#}");
    assert_eq!(stores.board.status("N-1").as_deref(), Some("Todo"));
    assert_eq!(
        stores.board.state("N-1").0,
        "OPEN",
        "a refused option leaves the issue open"
    );
    for (at, id) in [(1, "N-2"), (2, "N-3"), (7, "N-8")] {
        assert_eq!(items[at]["action"], "updated", "{answer:#}");
        assert_eq!(stores.board.status(id).as_deref(), Some("Done"), "{id}");
    }
}

#[test]
fn an_unknown_source_and_a_malformed_reference_are_refused_before_any_request() {
    let stores = Stores::new();
    for (arguments, says) in [
        (
            vec![
                "task",
                "status",
                "complete",
                "--issue",
                "nickderobertis/onetaskgraph#100001",
                "--source",
                "nowhere",
            ],
            "--source nowhere: no source named \"nowhere\" is configured",
        ),
        (
            vec![
                "task",
                "status",
                "complete",
                "--issue",
                "nickderobertis/onetaskgraph-1",
            ],
            "\"nickderobertis/onetaskgraph-1\" is not an issue reference; spell one owner/name#N",
        ),
        (
            vec!["task", "status", "complete", "--issue", "onetaskgraph#3"],
            "\"onetaskgraph#3\" is not an issue reference",
        ),
        (
            vec!["task", "status", "complete", "--source", "board-a"],
            "the following required arguments were not provided",
        ),
    ] {
        let output = stores.run(&arguments);
        assert_eq!(
            output.status.code(),
            Some(2),
            "{arguments:?}: a usage error"
        );
        assert!(
            stderr(&output).contains(says),
            "{arguments:?}: {}",
            stderr(&output)
        );
    }
    assert!(stores.board.served().is_empty(), "no board was asked");
    assert_eq!(stores.linear.served(), Vec::new());
    assert!(
        !stores
            .plugin_requests()
            .iter()
            .any(|method| method != "initialize"),
        "{:?}",
        stores.plugin_requests()
    );
}

#[test]
fn a_source_whose_status_is_its_closure_answers_nothing_and_is_sent_nothing() {
    let stores = Stores::new();
    let notes = stores.notes();
    for source in ["linear", "notes", "docs"] {
        let answer = stores.complete(&named(), &["--source", source], 0);
        assert_eq!(
            answer,
            json!({"items": [], "unsupported": [source]}),
            "{source}"
        );
    }
    assert_eq!(stores.linear.served(), Vec::new(), "Linear is sent nothing");
    assert!(
        stores
            .plugin_requests()
            .iter()
            .all(|method| method == "initialize"),
        "the plugin is sent nothing but its handshakes: {:?}",
        stores.plugin_requests()
    );
    assert_eq!(stores.notes(), notes);
    assert!(stores.board.served().is_empty(), "no board was asked");
}

/// An engine built from `stores`' own configuration, as a Rust caller links one.
fn library_engine(stores: &Stores) -> Engine {
    let config = Config::from_document(json!({
        "sources": stores.sources,
        "default_sources": ["board-b", "notes"],
    }))
    .expect("a valid configuration");
    let secrets = Secrets::load(Environment::from_pairs([
        ("GITHUB_PROJECTS_FIXTURE_TOKEN", "test-token"),
        ("GITHUB_PROJECTS_FIXTURE_TOKEN_B", "test-token-b"),
        ("LINEAR_API_KEY", "fixture-key"),
    ]))
    .expect("the loopbacks' credentials");
    Engine::build(&config, &secrets)
}

/// `engine.complete_issues(issues)`, naming no source, run to its answer.
fn completed_by(engine: &Engine, issues: &[IssueRef]) -> onetaskgraph_core::IssuesCompleted {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime")
        .block_on(engine.complete_issues(issues, &[] as &[SourceName], false))
        .expect("the completion runs")
}

#[test]
fn the_library_form_answers_what_the_command_line_answers() {
    let cli = Stores::new();
    let answer = cli.complete(&named(), &[], 0);

    let library = Stores::new();
    let engine = library_engine(&library);
    let issues: Vec<IssueRef> = NAMED
        .iter()
        .map(|id| reference(id).parse().expect("a reference"))
        .collect();
    let completed = completed_by(&engine, &issues);
    assert_eq!(
        serde_json::to_value(&completed).expect("it serialises"),
        answer
    );
    assert_eq!(library.board.status("N-1").as_deref(), Some("Done"));
    assert_eq!(
        library.board.second_board_status("N-8").as_deref(),
        Some("Done")
    );
}

/// `future` on a runtime of its own, as a Rust caller drives one engine call to its answer.
fn run_to_answer<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime")
        .block_on(future)
}

#[test]
fn a_library_completion_narrowed_to_a_source_nothing_configures_is_refused_before_any_request() {
    let stores = Stores::new();
    let notes = stores.notes();
    let engine = library_engine(&stores);
    let issues: Vec<IssueRef> = NAMED
        .iter()
        .map(|id| reference(id).parse().expect("a reference"))
        .collect();
    let only = [
        SourceName::new("board-a").expect("a name"),
        SourceName::new("nowhere").expect("a name"),
    ];
    let refused = run_to_answer(engine.complete_issues(&issues, &only, false))
        .expect_err("a source nothing configures is refused");
    let EngineError::UnknownSource { name, .. } = &refused else {
        panic!("refused as an unknown source, not {refused:?}");
    };
    assert_eq!(name, "nowhere");
    assert!(
        refused.to_string().contains("board-a"),
        "the refusal names what is configured: {refused}"
    );
    assert!(stores.board.served().is_empty(), "no board was asked");
    assert_eq!(stores.linear.served(), Vec::new(), "Linear is sent nothing");
    assert!(
        stores
            .plugin_requests()
            .iter()
            .all(|method| method == "initialize"),
        "{:?}",
        stores.plugin_requests()
    );
    assert_eq!(stores.notes(), notes);
    assert_eq!(stores.board.status("N-1").as_deref(), Some("Todo"));
}

#[test]
fn a_reconcile_of_a_board_that_cannot_start_is_refused_whole_naming_why() {
    let stores = Stores::new();
    stores
        .sandbox
        .secrets_file("GITHUB_PROJECTS_FIXTURE_TOKEN=test-token\nLINEAR_API_KEY=fixture-key\n");
    let output = stores.run(&[
        "--json",
        "task",
        "status",
        "reconcile",
        "--source",
        "board-b",
    ]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    let failure: Value = serde_json::from_str(&stdout(&output)).expect("a failure document");
    assert_eq!(failure["failure"]["source"], "board-b", "{failure:#}");
    assert!(
        failure["failure"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("GITHUB_PROJECTS_FIXTURE_TOKEN_B")),
        "the failure names the missing credential: {failure:#}"
    );
    assert!(
        failure.get("items").is_none(),
        "no item is listed: {failure:#}"
    );
    assert!(
        stderr(&output).contains("GITHUB_PROJECTS_FIXTURE_TOKEN_B"),
        "{}",
        stderr(&output)
    );

    let config =
        Config::from_document(json!({"sources": stores.sources})).expect("a valid configuration");
    let secrets = Secrets::load(Environment::from_pairs([
        ("GITHUB_PROJECTS_FIXTURE_TOKEN", "test-token"),
        ("LINEAR_API_KEY", "fixture-key"),
    ]))
    .expect("every credential but board B's");
    let engine = Engine::build(&config, &secrets);
    let board_b = SourceName::new("board-b").expect("a name");
    let refused = run_to_answer(engine.reconcile_statuses(&board_b, false))
        .expect_err("a source that did not build is refused whole");
    assert!(
        matches!(&refused, EngineError::SourceUnavailable { name, .. } if name == "board-b"),
        "{refused:?}"
    );
    assert!(stores.board.served().is_empty(), "no board was asked");
    assert_eq!(
        stores.board.second_board_status("N-8").as_deref(),
        Some("Todo")
    );
}

#[test]
fn a_reconcile_of_a_source_whose_status_is_its_closure_answers_nothing_and_sends_nothing() {
    let stores = Stores::new();
    let notes = stores.notes();
    for source in ["linear", "notes", "docs"] {
        let output = stores.run(&["--json", "task", "status", "reconcile", "--source", source]);
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        let answer: Value = serde_json::from_str(&stdout(&output)).expect("JSON");
        assert_eq!(answer, json!({"items": []}), "{source}");
        let human = stores.run(&["task", "status", "reconcile", "--source", source]);
        assert_eq!(
            stdout(&human).trim_end(),
            format!("source {source}'s status is its closure, so nothing needs reconciling"),
            "{source}"
        );
    }
    assert_eq!(stores.linear.served(), Vec::new(), "Linear is sent nothing");
    assert!(
        stores
            .plugin_requests()
            .iter()
            .all(|method| method == "initialize"),
        "{:?}",
        stores.plugin_requests()
    );
    assert_eq!(stores.notes(), notes);
    assert!(stores.board.served().is_empty());
}

/// The operation every read by reference carries.
const READ_BY_REFERENCE: &str = "r0:repository(owner:$owner0";

/// The items of `answer` one source answered, each as its issue and its action.
fn answered_by(answer: &Value, source: &str) -> Vec<(String, String)> {
    answer["items"]
        .as_array()
        .expect("items")
        .iter()
        .filter(|item| item["source"] == source)
        .map(|item| {
            (
                item["issue"].as_str().expect("an issue").to_owned(),
                item["action"].as_str().expect("an action").to_owned(),
            )
        })
        .collect()
}

/// The distinct references of [`NAMED`], in the order given.
fn distinct() -> Vec<String> {
    let mut seen = Vec::new();
    for reference in named() {
        if !seen.contains(&reference) {
            seen.push(reference);
        }
    }
    seen
}

/// That every item `source` answered in `answer` is `failed`, carrying a failure, one per
/// distinct reference.
fn every_reference_failed(answer: &Value, source: &str) {
    let items: Vec<&Value> = answer["items"]
        .as_array()
        .expect("items")
        .iter()
        .filter(|item| item["source"] == source)
        .collect();
    let issues: Vec<String> = items
        .iter()
        .map(|item| item["issue"].as_str().expect("an issue").to_owned())
        .collect();
    assert_eq!(
        issues,
        distinct(),
        "{source}: one per reference: {answer:#}"
    );
    for item in items {
        assert_eq!(item["action"], "failed", "{source}: {item:#}");
        assert!(
            item["error"]["message"].is_string(),
            "{source}: the failure is carried: {item:#}"
        );
        assert_eq!(item["task"], Value::Null, "{item:#}");
    }
}

#[test]
fn a_board_that_cannot_start_answers_each_reference_failed_and_the_other_board_is_still_written() {
    let stores = Stores::new();
    stores
        .sandbox
        .secrets_file("GITHUB_PROJECTS_FIXTURE_TOKEN=test-token\nLINEAR_API_KEY=fixture-key\n");
    let answer = stores.complete(&named(), &[], 4);
    every_reference_failed(&answer, "board-b");
    let owed = owed("updated");
    let board_a: Vec<Value> = owed["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["source"] == "board-a")
        .cloned()
        .collect();
    let answered_a: Vec<Value> = answer["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["source"] == "board-a")
        .cloned()
        .collect();
    assert_eq!(answered_a, board_a, "board A is answered as ever");
    assert!(
        answer["items"].as_array().unwrap().iter().any(|item| {
            item["source"] == "board-b"
                && item["error"]["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("GITHUB_PROJECTS_FIXTURE_TOKEN_B"))
        }),
        "the failure names the missing credential: {answer:#}"
    );
    assert!(
        stores.board.served_to(GITHUB_SECOND_BOARD_TOKEN).is_empty(),
        "board B is never reached"
    );
    assert_eq!(stores.board.status("N-1").as_deref(), Some("Done"));
}

#[test]
fn a_board_whose_read_by_reference_is_refused_answers_each_reference_failed_and_writes_nothing() {
    let stores = Stores::new();
    stores.board.refuse_once(READ_BY_REFERENCE);
    let answer = stores.complete(&named(), &[], 4);
    // Sources are asked in name order, so the refused read is board A's.
    every_reference_failed(&answer, "board-a");
    assert_eq!(stores.board.status("N-1").as_deref(), Some("Todo"));
    assert_eq!(stores.board.state("N-1").0, "OPEN", "board A wrote nothing");
    let board_b = answered_by(&answer, "board-b");
    assert!(
        board_b.contains(&(reference("N-8"), "updated".to_owned())),
        "board B is still written: {answer:#}"
    );
    assert_eq!(
        stores.board.second_board_status("N-8").as_deref(),
        Some("Done")
    );
}

#[test]
fn a_reference_no_repository_holds_is_absent_and_an_error_beside_it_fails_that_board() {
    let stores = Stores::new();
    let nothing = format!("{REPOSITORY}#999999");
    let answer = stores.complete(&[reference("N-1"), nothing.clone()], &[], 0);
    assert_eq!(
        answered_by(&answer, "board-a"),
        vec![
            (reference("N-1"), "updated".to_owned()),
            (nothing.clone(), "absent".to_owned())
        ],
        "a NOT_FOUND is the issue's absence: {answer:#}"
    );
    assert_eq!(
        answered_by(&answer, "board-b"),
        vec![
            (reference("N-1"), "absent".to_owned()),
            (nothing.clone(), "absent".to_owned())
        ]
    );

    let stores = Stores::new();
    stores.board.error_beside_once(
        READ_BY_REFERENCE,
        "Something went wrong reading a repository",
    );
    let answer = stores.complete(&[reference("N-1"), nothing.clone()], &[], 4);
    let failed = answered_by(&answer, "board-a");
    assert_eq!(
        failed,
        vec![
            (reference("N-1"), "failed".to_owned()),
            (nothing, "failed".to_owned())
        ],
        "an error that is not a NOT_FOUND is not read as absence: {answer:#}"
    );
    assert_eq!(stores.board.status("N-1").as_deref(), Some("Todo"));
}

#[test]
fn a_board_hosted_over_the_stdio_protocol_reconciles_and_completes_as_one_in_process_does() {
    let sandbox = Sandbox::new();
    let items = vec![
        issue("N-1", "Todo"),
        issue("N-2", "In Progress"),
        issue("N-3", "Todo"),
    ];
    let (mut config, board) = github_projects_with_items(&sandbox, items);
    board.with_status_option("OPT-in-progress", "In Progress");
    board.reopen("T-2");
    board.close("N-2", Some("NOT_PLANNED"));
    board.close("N-3", Some("COMPLETED"));
    config["status_mapping"] = json!({"todo":"Todo","in-progress":"In Progress"});
    sandbox.secrets_file(SECRETS);
    sandbox.project_document(
        &serde_json::to_string_pretty(&json!({"sources": {
            "hosted": SourceBoundary::Subprocess.source_with_secrets(
                "github-projects",
                config,
                &["GITHUB_PROJECTS_FIXTURE_TOKEN"],
            ),
        }}))
        .expect("a configuration renders"),
    );
    let run = |arguments: &[&str]| -> Value {
        let output = sandbox
            .command()
            .args(arguments)
            .assert()
            .get_output()
            .clone();
        assert_eq!(
            output.status.code(),
            Some(0),
            "`onetaskgraph {}`\n{}\n{}",
            arguments.join(" "),
            stdout(&output),
            stderr(&output)
        );
        serde_json::from_str(&stdout(&output)).expect("JSON")
    };

    let reconciled = run(&[
        "--json",
        "task",
        "status",
        "reconcile",
        "--source",
        "hosted",
    ]);
    assert_eq!(
        reconciled,
        json!({"items": [
            {"source":"hosted:N-2","kind":"task","from":"In Progress",
             "to":{"category":"cancelled","name":"Cancelled"},"action":"updated"},
            {"source":"hosted:N-3","kind":"task","from":"Todo",
             "to":{"category":"done","name":"Done"},"action":"updated"},
        ]}),
        "{reconciled:#}"
    );
    assert_eq!(board.status("N-2").as_deref(), Some("Cancelled"));
    assert_eq!(board.status("N-3").as_deref(), Some("Done"));

    let completed = run(&[
        "--json",
        "task",
        "status",
        "complete",
        "--issue",
        &reference("N-1"),
        "--issue",
        &reference("N-3"),
    ]);
    let done = json!({"category":"done","name":"Done"});
    assert_eq!(
        completed,
        json!({"items": [
            {"issue":reference("N-1"),"source":"hosted","task":"hosted:N-1","kind":"task",
             "from":"Todo",
             "to":done,"action":"updated"},
            {"issue":reference("N-3"),"source":"hosted","task":"hosted:N-3","kind":"task",
             "from":"Done",
             "to":done,"action":"unchanged"},
        ], "unsupported": []}),
        "{completed:#}"
    );
    assert_eq!(board.status("N-1").as_deref(), Some("Done"));
    assert_eq!(
        board.state("N-1"),
        ("CLOSED".to_owned(), Some("COMPLETED".to_owned()))
    );
}

#[test]
fn a_library_completion_of_no_issues_answers_nothing_and_sends_nothing() {
    let stores = Stores::new();
    let engine = library_engine(&stores);
    let completed = completed_by(&engine, &[]);
    assert_eq!(
        serde_json::to_value(&completed).expect("it serialises"),
        json!({"items": [], "unsupported": ["docs", "linear", "notes"]})
    );
    assert!(stores.board.served().is_empty(), "no board was asked");
    assert_eq!(stores.linear.served(), Vec::new(), "Linear is sent nothing");
}

#[test]
fn a_read_by_reference_answered_malformed_fails_that_boards_references_and_writes_nothing() {
    for (fault, says) in [
        (
            ReferenceFault::NoAlias,
            "answered a read by reference with no r0",
        ),
        (
            ReferenceFault::NoIssue,
            "answered r0 of a read by reference with no issue",
        ),
        (
            ReferenceFault::NotAnIssue,
            "answered r0 of a read by reference with an issue whose __typename is not Issue",
        ),
        (
            ReferenceFault::MembershipNotAnObject,
            "a board membership that is not an object",
        ),
        (
            ReferenceFault::StatusNamingNoOption,
            "a membership status that names no option",
        ),
    ] {
        let stores = Stores::new();
        stores.board.bend_the_next_reference_read(fault);
        let answer = stores.complete(&named(), &[], 4);
        // Sources are asked in name order, so the bent read is board A's.
        every_reference_failed(&answer, "board-a");
        assert!(
            answer["items"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|item| item["source"] == "board-a")
                .all(|item| item["error"]["message"]
                    .as_str()
                    .is_some_and(|message| message.contains(says))),
            "{fault:?}: {answer:#}"
        );
        assert_eq!(
            stores.board.status("N-1").as_deref(),
            Some("Todo"),
            "{fault:?}"
        );
        assert_eq!(stores.board.state("N-1").0, "OPEN", "{fault:?}");
        assert!(
            answered_by(&answer, "board-b").contains(&(reference("N-8"), "updated".to_owned())),
            "{fault:?}: board B is still written: {answer:#}"
        );
    }
}

#[test]
fn a_card_past_the_first_page_of_an_issues_memberships_is_found_and_written() {
    let stores = Stores::new();
    // N-8 sits on board A, then two other boards, then board B: board B's card is its fourth
    // membership, past the first page a read by reference carries.
    stores.board.put_on_other_boards("N-8", &[20, 21]);
    let answer = stores.complete(&[reference("N-8")], &["--source", "board-b"], 0);
    assert_eq!(
        answered_by(&answer, "board-b"),
        vec![(reference("N-8"), "updated".to_owned())],
        "{answer:#}"
    );
    assert_eq!(answer["items"][0]["from"], "Todo", "{answer:#}");
    assert_eq!(
        stores.board.second_board_status("N-8").as_deref(),
        Some("Done")
    );
    let reads: Vec<String> = stores
        .board
        .served_to(GITHUB_SECOND_BOARD_TOKEN)
        .into_iter()
        .map(|(document, _)| document)
        .filter(|document| !document.trim_start().starts_with("mutation"))
        .collect();
    assert_eq!(
        reads,
        vec![
            onetaskgraph_github_projects::graphql::ISSUES_BY_REFERENCE.to_owned(),
            onetaskgraph_github_projects::graphql::ISSUE_BOARD_ITEMS.to_owned(),
        ],
        "the read by reference, then the issue's later memberships, and no board read"
    );
}

#[test]
fn a_completion_reads_to_a_person_item_by_item_with_each_failure_on_stderr() {
    let stores = Stores::new();
    let dry = stores.run(&[
        "task",
        "status",
        "complete",
        "--issue",
        &reference("N-2"),
        "--issue",
        &reference("N-5"),
        "--source",
        "board-a",
        "--dry-run",
    ]);
    assert_eq!(dry.status.code(), Some(0), "{}", stderr(&dry));
    let said = stdout(&dry);
    assert!(
        said.contains(&format!(
            "{} in board-a: would-update board-a:N-2 (In Progress -> Done)",
            reference("N-2")
        )) && said.contains(&format!(
            "{} in board-a: skipped board-a:N-5 — closed as not planned",
            reference("N-5")
        )) && said.contains("unsupported (status is its closure, or no complete_by_issue): none")
            && said
                .contains("0 updated, 1 would update, 0 unchanged, 1 skipped, 0 absent, 0 failed"),
        "{said}"
    );
    assert_eq!(
        stderr(&dry),
        "",
        "nothing failed, so nothing is said on stderr"
    );

    stores
        .board
        .refuse_once("updateProjectV2ItemFieldValue(input:$input)");
    let output = stores.run(&[
        "task",
        "status",
        "complete",
        "--issue",
        &reference("N-1"),
        "--issue",
        &reference("N-2"),
        "--source",
        "board-a",
    ]);
    assert_eq!(output.status.code(), Some(4), "{}", stderr(&output));
    let said = stdout(&output);
    assert!(
        said.contains(&format!(
            "{} in board-a: failed board-a:N-1",
            reference("N-1")
        )) && said.contains(&format!(
            "{} in board-a: updated board-a:N-2 (In Progress -> Done)",
            reference("N-2")
        )) && said
            .contains("1 updated, 0 would update, 0 unchanged, 0 skipped, 0 absent, 1 failed"),
        "{said}"
    );
    assert!(
        stderr(&output).contains(&format!(
            "onetaskgraph: {} was not completed in board-a: ",
            reference("N-1")
        )),
        "{}",
        stderr(&output)
    );
}

#[test]
fn a_reference_past_githubs_issue_number_range_is_absent_and_never_sent() {
    let stores = Stores::new();
    let beyond = format!("{REPOSITORY}#9999999999");
    let answer = stores.complete(
        &[reference("N-1"), beyond.clone()],
        &["--source", "board-a"],
        0,
    );
    assert_eq!(
        answered_by(&answer, "board-a"),
        vec![
            (reference("N-1"), "updated".to_owned()),
            (beyond, "absent".to_owned())
        ],
        "{answer:#}"
    );
    for (document, variables) in stores.board.served() {
        if document == onetaskgraph_github_projects::graphql::ISSUES_BY_REFERENCE {
            assert!(
                variables
                    .as_object()
                    .expect("variables")
                    .iter()
                    .filter(|(name, _)| name.starts_with("number"))
                    .all(|(_, number)| number.as_u64().is_some_and(|n| n <= i32::MAX as u64)),
                "no number past GitHub's Int was bound: {variables}"
            );
        }
    }
    assert_eq!(stores.board.status("N-1").as_deref(), Some("Done"));
}

/// The mutations `token`'s board was sent, each as its root field and the item or issue it
/// names.
fn mutations_to(stores: &Stores, token: &str) -> Vec<(String, Value)> {
    stores
        .board
        .served_to(token)
        .into_iter()
        .filter(|(document, _)| document.trim_start().starts_with("mutation"))
        .map(|(document, variables)| {
            if document.contains("updateIssue(input:") {
                ("close".to_owned(), variables["input"]["id"].clone())
            } else if document.contains("updateProjectV2ItemFieldValue(input:") {
                ("option".to_owned(), variables["input"]["itemId"].clone())
            } else {
                (document, variables)
            }
        })
        .collect()
}

/// What completing the home and the task `N-1` owes, naming no source: both written on board A,
/// the home named a project; neither held on board B.
fn home_and_task_owed(action: &str) -> Value {
    let done = json!({"category":"done","name":"Done"});
    json!({"items": [
        {"issue":reference(HOME),"source":"board-a","task":format!("board-a:{HOME}"),
         "kind":"project","from":"In Progress","to":done,"action":action},
        {"issue":reference("N-1"),"source":"board-a","task":"board-a:N-1","kind":"task",
         "from":"Todo","to":done,"action":action},
        {"issue":reference(HOME),"source":"board-b","task":null,"kind":"task","from":null,
         "to":null,"action":"absent"},
        {"issue":reference("N-1"),"source":"board-b","task":null,"kind":"task","from":null,
         "to":null,"action":"absent"},
    ], "unsupported": ["docs", "linear", "notes"]})
}

#[test]
fn a_plan_home_closes_beside_its_task_as_a_project_item_from_the_reads_a_task_takes() {
    let stores = Stores::new();
    let notes = stores.notes();
    let second_before = stores.board.second_board_status("N-8");
    let answer = stores.complete(&[reference(HOME), reference("N-1")], &[], 0);
    assert_eq!(answer, home_and_task_owed("updated"), "{answer:#}");

    assert_eq!(
        stores.board.status(HOME).as_deref(),
        Some("Done"),
        "the home's card sits at the project kind's done option"
    );
    assert_eq!(
        stores.board.state(HOME),
        ("CLOSED".to_owned(), Some("COMPLETED".to_owned())),
        "the home's issue is closed as completed"
    );
    assert_eq!(stores.board.status("N-1").as_deref(), Some("Done"));
    for sub in ["N-10", "N-11"] {
        assert_eq!(stores.board.status(sub).as_deref(), Some("Todo"), "{sub}");
        assert_eq!(stores.board.state(sub).0, "OPEN", "{sub}");
    }
    assert_eq!(
        mutations_to(&stores, "test-token"),
        vec![
            ("option".to_owned(), json!(format!("ITEM-{HOME}"))),
            ("close".to_owned(), json!(HOME)),
            ("option".to_owned(), json!("ITEM-N-1")),
            ("close".to_owned(), json!("N-1")),
        ],
        "one option write and one close per moved item"
    );
    assert_eq!(
        mutations_to(&stores, GITHUB_SECOND_BOARD_TOKEN),
        Vec::new(),
        "board B is written nothing"
    );
    assert_eq!(stores.board.second_board_status("N-8"), second_before);
    // One batched read by reference per board, and no read of either board's items.
    stores.one_read_of_the_named_issues("test-token");
    stores.one_read_of_the_named_issues(GITHUB_SECOND_BOARD_TOKEN);
    assert_eq!(stores.linear.served(), Vec::new(), "Linear is sent nothing");
    assert_eq!(
        stores.plugin_requests(),
        vec!["initialize"],
        "the plugin is sent nothing after its handshake"
    );
    assert_eq!(stores.notes(), notes, "the folder of Markdown is unchanged");

    let fresh = Stores::new();
    let human = fresh.run(&[
        "task",
        "status",
        "complete",
        "--issue",
        &reference(HOME),
        "--issue",
        &reference("N-1"),
        "--source",
        "board-a",
    ]);
    assert_eq!(human.status.code(), Some(0), "{}", stderr(&human));
    let said = stdout(&human);
    assert!(
        said.contains(&format!(
            "{} in board-a: updated project board-a:{HOME} (In Progress -> Done)",
            reference(HOME)
        )) && said.contains(&format!(
            "{} in board-a: updated board-a:N-1 (Todo -> Done)",
            reference("N-1")
        )),
        "{said}"
    );
}

#[test]
fn a_plan_home_closed_as_not_planned_is_skipped_and_one_already_done_is_unchanged() {
    let stores = Stores::new();
    stores.board.close(HOME, Some("NOT_PLANNED"));
    let answer = stores.complete(&[reference(HOME)], &["--source", "board-a"], 0);
    assert_eq!(
        answer,
        json!({"items": [{"issue":reference(HOME),"source":"board-a",
                          "task":format!("board-a:{HOME}"),"kind":"project",
                          "from":"In Progress","to":null,"action":"skipped",
                          "reason":"closed as not planned"}],
               "unsupported": []}),
        "{answer:#}"
    );
    assert_eq!(stores.board.status(HOME).as_deref(), Some("In Progress"));

    stores.board.close(HOME, Some("COMPLETED"));
    stores.board.move_card(HOME, "Done");
    let answer = stores.complete(&[reference(HOME)], &["--source", "board-a"], 0);
    assert_eq!(
        answer,
        json!({"items": [{"issue":reference(HOME),"source":"board-a",
                          "task":format!("board-a:{HOME}"),"kind":"project","from":"Done",
                          "to":{"category":"done","name":"Done"},"action":"unchanged"}],
               "unsupported": []}),
        "{answer:#}"
    );
    assert_eq!(
        mutations_to(&stores, "test-token"),
        Vec::new(),
        "neither is written"
    );
    assert_eq!(
        stores.board.state(HOME),
        ("CLOSED".to_owned(), Some("COMPLETED".to_owned()))
    );
}

#[test]
fn the_library_form_completes_a_plan_home_as_the_command_line_does() {
    let cli = Stores::new();
    let answer = cli.complete(&[reference(HOME), reference("N-1")], &[], 0);

    let library = Stores::new();
    let engine = library_engine(&library);
    let issues: Vec<IssueRef> = [HOME, "N-1"]
        .iter()
        .map(|id| reference(id).parse().expect("a reference"))
        .collect();
    let completed = completed_by(&engine, &issues);
    assert_eq!(
        completed.items[0].kind,
        onetaskgraph_plugin_api::ItemKind::Project
    );
    assert!(
        completed.items[0].delivered.is_empty(),
        "a project delivers none"
    );
    assert_eq!(
        serde_json::to_value(&completed).expect("it serialises"),
        answer
    );
    assert_eq!(answer, home_and_task_owed("updated"), "{answer:#}");
    assert_eq!(library.board.status(HOME).as_deref(), Some("Done"));
    assert_eq!(
        library.board.state(HOME),
        ("CLOSED".to_owned(), Some("COMPLETED".to_owned()))
    );
}

#[test]
fn a_dry_run_names_a_plan_home_would_update_and_a_refused_home_write_fails_beside_the_task() {
    let stores = Stores::new();
    let answer = stores.complete(&[reference(HOME), reference("N-1")], &["--dry-run"], 0);
    assert_eq!(answer, home_and_task_owed("would-update"), "{answer:#}");
    assert_eq!(
        mutations_to(&stores, "test-token"),
        Vec::new(),
        "a dry run writes nothing"
    );
    assert_eq!(stores.board.status(HOME).as_deref(), Some("In Progress"));
    assert_eq!(stores.board.state(HOME).0, "OPEN");

    // The home is written first, so the one refused option write is its own.
    stores
        .board
        .refuse_once("updateProjectV2ItemFieldValue(input:$input)");
    let output = stores.run(&[
        "--json",
        "task",
        "status",
        "complete",
        "--issue",
        &reference(HOME),
        "--issue",
        &reference("N-1"),
        "--source",
        "board-a",
    ]);
    assert_eq!(output.status.code(), Some(4), "{}", stderr(&output));
    let answer: Value = serde_json::from_str(&stdout(&output)).expect("JSON");
    let home = &answer["items"][0];
    assert_eq!(
        (&home["kind"], &home["action"], &home["error"]["class"]),
        (&json!("project"), &json!("failed"), &json!("refused")),
        "{answer:#}"
    );
    assert_eq!(answer["items"][1]["action"], "updated", "{answer:#}");
    assert_eq!(stores.board.status(HOME).as_deref(), Some("In Progress"));
    assert_eq!(
        stores.board.state(HOME).0,
        "OPEN",
        "a refused option leaves the home's issue open"
    );
    assert_eq!(stores.board.status("N-1").as_deref(), Some("Done"));
    assert!(
        stderr(&output).contains(&format!(
            "onetaskgraph: {} was not completed in board-a: ",
            reference(HOME)
        )),
        "{}",
        stderr(&output)
    );
}
