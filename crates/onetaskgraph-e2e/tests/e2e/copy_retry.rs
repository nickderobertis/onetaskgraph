//! A create that is safe to retry: a copy that dies part way through a create, run again,
//! finishes the one item its first attempt made rather than making a second.
//!
//! The realistic caller syncs a folder of Markdown onto a tracker from a job that can be killed
//! or time out at any moment. Each journey stops the binary at a point of a create — by the
//! test-only `OTG_TEST_STOP_AT`, which ends the process there as a kill would, or by a
//! loopback that applies a request and never answers it — then runs the same copy again and
//! asserts on the destination's items and on the source's `onetaskgraph.creating` and
//! `onetaskgraph.copies`.

use std::path::PathBuf;
use std::process::Output;

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{
    GitHubBoardFields, LinearWorkspace, dataset, document, github_projects_with_board,
    linear_default_mapping, linear_workspace,
};

const NOTE: &str = "notes:A";

struct Stores {
    sandbox: Sandbox,
    board: GitHubBoardFields,
    linear: LinearWorkspace,
    notes: PathBuf,
    folder: PathBuf,
}

impl Stores {
    fn new() -> Self {
        Self::leased(1)
    }

    /// The stores, the folder of notes leasing an intent for `lease` seconds.
    fn leased(lease: u64) -> Self {
        let sandbox = Sandbox::new();
        let (board_config, board) = github_projects_with_board(&sandbox);
        let (mut linear_config, linear) = linear_workspace(&sandbox, dataset());
        linear_config["status_mapping"] = linear_default_mapping();
        sandbox
            .secrets_file("GITHUB_PROJECTS_FIXTURE_TOKEN=test-token\nLINEAR_API_KEY=fixture-key\n");
        let notes = sandbox.subdirectory("notes");
        std::fs::create_dir_all(notes.join("tasks")).expect("the task folder");
        let folder = sandbox.subdirectory("folder");
        std::fs::create_dir_all(folder.join("tasks")).expect("the task folder");
        sandbox.project_document(&document(&json!({
            "board": {"plugin": "github-projects", "config": board_config,
                      "create_quiescence_seconds": 2},
            "linear": {"plugin": "linear", "config": linear_config},
            // A lease of a second, so a retry of a killed copy takes over after a short wait.
            "notes": {"plugin": "local-md", "config": {"root": notes},
                      "create_lease_seconds": lease},
            "folder": {"plugin": "local-md", "config": {"root": folder}},
        })));
        std::fs::write(
            notes.join("tasks/A.md"),
            "---\ntitle: Task A\nstatus: todo\n---\nThe body.\n",
        )
        .expect("the task file");
        Self {
            sandbox,
            board,
            linear,
            notes,
            folder,
        }
    }

    fn run(&self, arguments: &[&str], stop_at: Option<&str>) -> Output {
        let mut command = self.sandbox.command();
        if let Some(point) = stop_at {
            command.env("OTG_TEST_STOP_AT", point);
        }
        let output = command.args(arguments).assert().get_output().clone();
        if stop_at.is_some() {
            // A killed copy's lease outlives it; the retry waits it out.
            std::thread::sleep(std::time::Duration::from_millis(1500));
        }
        output
    }

    fn json(&self, arguments: &[&str]) -> Value {
        let mut full = vec!["--json"];
        full.extend(arguments);
        let output = self.run(&full, None);
        assert_eq!(
            output.status.code(),
            Some(0),
            "`onetaskgraph {}`\nstdout:\n{}\nstderr:\n{}",
            full.join(" "),
            stdout(&output),
            stderr(&output)
        );
        serde_json::from_str(&stdout(&output)).expect("JSON")
    }

    /// The source task's metadata, as `task show` reads it.
    fn source_metadata(&self) -> Value {
        self.json(&["task", "show", NOTE])["items"][0]["item"]["metadata"].clone()
    }

    /// Every item of `destination` carrying the source's origin.
    fn carrying_origin(&self, destination: &str) -> Vec<Value> {
        self.json(&["task", "list", "--source", destination, "--origin", NOTE])["items"]
            .as_array()
            .expect("a page")
            .iter()
            .map(|item| item["item"].clone())
            .collect()
    }
}

#[test]
fn an_uninterrupted_create_records_its_intent_first_and_leaves_none_behind() {
    for destination in ["folder", "board", "linear"] {
        let stores = Stores::new();
        let before = std::fs::read_to_string(stores.notes.join("tasks/A.md")).unwrap();
        stores.json(&["task", "copy", NOTE, "--to", destination, "--dry-run"]);
        assert_eq!(
            std::fs::read_to_string(stores.notes.join("tasks/A.md")).unwrap(),
            before,
            "{destination}: a dry run writes no intent"
        );
        // Stopped right after the intent is recorded: it is there before any create is sent.
        let stopped = stores.run(
            &["task", "copy", NOTE, "--to", destination],
            Some("after-intent"),
        );
        assert_ne!(stopped.status.code(), Some(0));
        let intent = &stores.source_metadata()["onetaskgraph.creating"][destination];
        assert!(intent["key"].is_string(), "{destination}: {intent:#}");
        assert!(intent["lease_until"].is_string() && intent["at"].is_string());
        let key = intent["key"].as_str().unwrap().to_owned();

        let copied = stores.json(&["task", "copy", NOTE, "--to", destination]);
        let outcome = &copied["items"][0];
        assert_eq!(outcome["action"], "created", "{destination}: {copied:#}");
        let metadata = stores.source_metadata();
        assert!(
            metadata.get("onetaskgraph.creating").is_none(),
            "{destination}: no intent is left: {metadata:#}"
        );
        assert_eq!(
            metadata["onetaskgraph.copies"][destination],
            outcome["destination"]
        );
        let landed = stores.carrying_origin(destination);
        assert_eq!(landed.len(), 1, "{destination}: {landed:#?}");
        let created = &stores.json(&["task", "show", outcome["destination"].as_str().unwrap()])["items"]
            [0]["item"];
        let carried = created["metadata"]["onetaskgraph.create_key"]
            .as_str()
            .map(str::to_owned)
            .or_else(|| created["id"].as_str().map(str::to_owned));
        assert_eq!(
            carried.as_deref(),
            Some(key.as_str()),
            "{destination}: {created:#}"
        );
        // A second copy is an update of that item, and writes no intent.
        let again = stores.json(&["task", "copy", NOTE, "--to", destination]);
        assert_ne!(again["items"][0]["action"], "created", "{again:#}");
        assert!(
            stores
                .source_metadata()
                .get("onetaskgraph.creating")
                .is_none()
        );
    }
}

#[test]
fn a_copy_killed_after_its_writes_and_before_its_link_is_finished_by_its_retry() {
    for destination in ["folder", "board", "linear"] {
        let stores = Stores::new();
        let stopped = stores.run(
            &["task", "copy", NOTE, "--to", destination],
            Some("before-link"),
        );
        assert_ne!(stopped.status.code(), Some(0), "{destination}");
        assert!(
            stores
                .source_metadata()
                .get("onetaskgraph.copies")
                .is_none()
        );
        // The located counterpart is the one the key names, and locating writes nothing.
        let before = std::fs::read_to_string(stores.notes.join("tasks/A.md")).unwrap();
        let located = stores.json(&["task", "locate", NOTE, "--to", destination]);
        assert_eq!(
            located["items"][0]["counterpart"]["via"], "intent",
            "{destination}: {located:#}"
        );
        assert_eq!(
            std::fs::read_to_string(stores.notes.join("tasks/A.md")).unwrap(),
            before
        );

        let copied = stores.json(&["task", "copy", NOTE, "--to", destination]);
        let outcome = &copied["items"][0];
        assert_eq!(outcome["via"], "intent", "{destination}: {copied:#}");
        let landed = stores.carrying_origin(destination);
        assert_eq!(landed.len(), 1, "{destination}: exactly one: {landed:#?}");
        let metadata = stores.source_metadata();
        assert!(
            metadata.get("onetaskgraph.creating").is_none(),
            "{metadata:#}"
        );
        assert_eq!(
            metadata["onetaskgraph.copies"][destination],
            outcome["destination"]
        );
    }
}

#[test]
fn a_create_whose_answer_never_arrived_is_found_by_its_retry_on_the_board() {
    let stores = Stores::new();
    stores.board.drop_response_once("createIssue");
    let failed = stores.run(&["task", "copy", NOTE, "--to", "board"], None);
    assert_ne!(failed.status.code(), Some(0), "{}", stderr(&failed));
    assert!(stores.source_metadata()["onetaskgraph.creating"]["board"].is_object());
    let copied = stores.json(&["task", "copy", NOTE, "--to", "board"]);
    assert_eq!(copied["items"][0]["via"], "intent", "{copied:#}");
    let titled = stores.board.items_titled("Task A");
    assert_eq!(titled.len(), 1, "one issue: {titled:?}");
    assert!(
        stores
            .source_metadata()
            .get("onetaskgraph.creating")
            .is_none()
    );
}

#[test]
fn a_live_lease_another_attempt_holds_is_refused_before_any_destination_write() {
    // A copy killed right after recording its intent holds its lease for ten minutes.
    let stores = Stores::leased(600);
    let stopped = stores.run(
        &["task", "copy", NOTE, "--to", "board"],
        Some("after-intent"),
    );
    assert_ne!(stopped.status.code(), Some(0));
    let until = stores.source_metadata()["onetaskgraph.creating"]["board"]["lease_until"]
        .as_str()
        .expect("the killed copy's lease")
        .to_owned();
    let before = stores.board.served().len();
    let output = stores.run(&["--json", "task", "copy", NOTE, "--to", "board"], None);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    let failure: Value = serde_json::from_str(&stdout(&output)).expect("a failure document");
    assert_eq!(
        failure["failure"]["kind"], "create-in-progress",
        "{failure:#}"
    );
    assert!(
        failure["failure"]["message"]
            .as_str()
            .unwrap()
            .contains(&until),
        "names the lease's end: {failure:#}"
    );
    let writes = stores.board.served()[before..]
        .iter()
        .filter(|(document, _)| document.trim_start().starts_with("mutation"))
        .count();
    assert_eq!(writes, 0, "no destination write");
    assert!(stores.board.items_titled("Task A").is_empty());
}

#[test]
fn the_intent_key_is_refused_to_a_metadata_set() {
    let stores = Stores::new();
    let output = stores.run(
        &[
            "task",
            "metadata",
            "set",
            NOTE,
            "onetaskgraph.creating",
            "{}",
        ],
        None,
    );
    assert_ne!(output.status.code(), Some(0));
    let _ = &stores.folder;
    let _ = &stores.linear;
}

/// Whether a request is the board's create of an issue.
fn creates_issue(document: &str, _: &Value) -> bool {
    document.contains("createIssue(input:$input)")
}

/// Whether a request lists a repository's issues by creation, as a lookup by create key does.
fn lists_by_key(document: &str, _: &Value) -> bool {
    document == onetaskgraph_github_projects::graphql::ISSUES_BY_CREATION
}

/// How many requests of the board's log `matches` holds of.
fn count(board: &GitHubBoardFields, matches: fn(&str, &Value) -> bool) -> usize {
    board
        .served()
        .iter()
        .filter(|(document, variables)| matches(document, variables))
        .count()
}

/// The board's first copy of the note, its `createIssue` answer dropped and the issue it made
/// left out of `Repository.issues`: an earlier attempt whose create ran and is not listed yet.
fn unlisted_first_attempt(stores: &Stores) -> String {
    stores.board.unlist_new_issues(true);
    stores.board.drop_response_once("createIssue");
    let failed = stores.run(&["task", "copy", NOTE, "--to", "board"], None);
    assert_ne!(failed.status.code(), Some(0), "{}", stderr(&failed));
    stores.board.unlist_new_issues(false);
    let made = stores.board.items_titled("Task A");
    assert_eq!(made.len(), 1, "the first attempt's issue: {made:?}");
    made[0].clone()
}

#[test]
fn a_retry_waits_for_an_earlier_issue_listed_within_the_window_and_creates_nothing() {
    let stores = Stores::new();
    let first = unlisted_first_attempt(&stores);
    // Listed once the retry's first lookup has answered without it, within the window.
    let board = stores.board.clone();
    let listed = first.clone();
    stores
        .board
        .after_request(lists_by_key, 0, move |_| board.list_issue(&listed));
    let creates = count(&stores.board, creates_issue);
    let lookups = count(&stores.board, lists_by_key);
    let copied = stores.json(&["task", "copy", NOTE, "--to", "board"]);
    assert_eq!(copied["items"][0]["via"], "intent", "{copied:#}");
    assert_eq!(
        count(&stores.board, creates_issue),
        creates,
        "nothing created"
    );
    assert_eq!(
        count(&stores.board, lists_by_key) - lookups,
        2,
        "one lookup, then one after the wait"
    );
    assert_eq!(stores.board.items_titled("Task A"), vec![first]);
    let metadata = stores.source_metadata();
    assert!(
        metadata.get("onetaskgraph.creating").is_none(),
        "{metadata:#}"
    );
    assert!(metadata["onetaskgraph.copies"]["board"].is_string());
}

#[test]
fn an_earlier_issue_listed_after_the_window_and_before_the_relist_is_refused_as_a_duplicate() {
    let stores = Stores::new();
    let first = unlisted_first_attempt(&stores);
    // Listed only once the retry has created again: its relist by key sees both.
    let board = stores.board.clone();
    let listed = first.clone();
    stores
        .board
        .after_request(creates_issue, 0, move |_| board.list_issue(&listed));
    let output = stores.run(&["--json", "task", "copy", NOTE, "--to", "board"], None);
    assert_eq!(output.status.code(), Some(4), "{}", stderr(&output));
    let report: Value = serde_json::from_str(&stdout(&output)).expect("a report");
    let outcome = &report["items"][0];
    assert_eq!(outcome["error"]["kind"], "create-duplicated", "{report:#}");
    let message = outcome["error"]["message"].as_str().unwrap();
    let issues = stores.board.items_titled("Task A");
    assert_eq!(issues.len(), 2, "{issues:?}");
    for issue in &issues {
        assert!(message.contains(issue.as_str()), "names {issue}: {message}");
    }
    let metadata = stores.source_metadata();
    assert!(
        metadata.get("onetaskgraph.copies").is_none(),
        "no link: {metadata:#}"
    );
    assert!(
        metadata["onetaskgraph.creating"]["board"].is_object(),
        "the intent is left"
    );
}

#[test]
fn an_earlier_issue_listed_only_after_the_relist_is_the_documented_unreported_residual() {
    let stores = Stores::new();
    let first = unlisted_first_attempt(&stores);
    let copied = stores.json(&["task", "copy", NOTE, "--to", "board"]);
    let outcome = &copied["items"][0];
    assert_eq!(outcome["action"], "created", "{copied:#}");
    // The first attempt's issue is listed only now, after every lookup the retry made: two
    // issues carry the key, the first unannounced. This is the residual the README states;
    // a change in the boundary turns this red.
    stores.board.list_issue(&first);
    assert_eq!(stores.board.items_titled("Task A").len(), 2);
    let metadata = stores.source_metadata();
    assert_eq!(
        metadata["onetaskgraph.copies"]["board"],
        outcome["destination"]
    );
    assert!(metadata.get("onetaskgraph.creating").is_none());
}

#[test]
fn a_copy_killed_after_its_create_answered_and_before_its_board_writes_is_finished_by_its_retry() {
    let stores = Stores::new();
    // The board holds every request after the create answered until the process is gone.
    let (release, held) = std::sync::mpsc::channel::<()>();
    let (blocked, waiting) = std::sync::mpsc::channel::<()>();
    stores.board.after_request(creates_issue, 0, move |_| {
        let _ = blocked.send(());
        let _ = held.recv_timeout(std::time::Duration::from_secs(20));
    });
    let mut child = stores
        .sandbox
        .subprocess(onetaskgraph_e2e_support::binary())
        .args(["task", "copy", NOTE, "--to", "board"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("the copy starts");
    waiting
        .recv_timeout(std::time::Duration::from_secs(20))
        .expect("the create was answered");
    child.kill().expect("the copy is killed");
    let _ = child.wait();
    drop(release);
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert_eq!(stores.board.items_titled("Task A").len(), 1);

    let copied = stores.json(&["task", "copy", NOTE, "--to", "board"]);
    let outcome = &copied["items"][0];
    assert_eq!(outcome["via"], "intent", "{copied:#}");
    assert_eq!(stores.board.items_titled("Task A").len(), 1, "exactly one");
    let shown = stores.json(&["task", "show", outcome["destination"].as_str().unwrap()]);
    assert_eq!(
        shown["items"][0]["item"]["status"]["category"], "todo",
        "{shown:#}"
    );
    let metadata = stores.source_metadata();
    assert!(metadata.get("onetaskgraph.creating").is_none());
    assert_eq!(
        metadata["onetaskgraph.copies"]["board"],
        outcome["destination"]
    );
}

#[test]
fn the_create_settings_are_shown_by_their_layer_and_refused_out_of_range() {
    let stores = Stores::new();
    let shown = stores.json(&["config", "show"]);
    let text = shown.to_string();
    assert!(
        text.contains("create_lease_seconds") && text.contains("create_quiescence_seconds"),
        "{shown:#}"
    );
    for value in ["0", "86401"] {
        let output = stores
            .sandbox
            .command()
            .env("ONETASKGRAPH_SOURCES__NOTES__CREATE_LEASE_SECONDS", value)
            .args(["config", "show"])
            .assert()
            .get_output()
            .clone();
        assert_ne!(output.status.code(), Some(0), "{value}");
        assert!(
            stderr(&output).contains("sources.notes.create_lease_seconds"),
            "{value}: {}",
            stderr(&output)
        );
    }
}

#[test]
fn a_lock_a_dead_copy_left_beside_the_file_is_taken_over_and_released() {
    let stores = Stores::new();
    let lock = stores.notes.join("tasks/A.md.lock");
    let file = std::fs::File::create(&lock).expect("a lock a dead copy left");
    file.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(120))
        .expect("an old lock");
    drop(file);
    let copied = stores.json(&["task", "copy", NOTE, "--to", "folder"]);
    assert_eq!(copied["items"][0]["action"], "created", "{copied:#}");
    assert!(!lock.exists(), "the lock is released");
    assert!(
        stores
            .source_metadata()
            .get("onetaskgraph.creating")
            .is_none()
    );
}

#[test]
fn a_retry_pages_back_through_the_repository_to_the_issue_its_key_names_and_no_further() {
    let stores = Stores::new();
    stores.board.drop_response_once("createIssue");
    let failed = stores.run(&["task", "copy", NOTE, "--to", "board"], None);
    assert_ne!(failed.status.code(), Some(0), "{}", stderr(&failed));
    // A newer issue of the same repository, so the earlier attempt's is on a later page.
    std::fs::write(
        stores.notes.join("tasks/B.md"),
        "---\ntitle: Task B\nstatus: todo\n---\nAnother.\n",
    )
    .expect("another task");
    stores.json(&["task", "copy", "notes:B", "--to", "board"]);
    stores.board.answer_issue_listings_in_pages_of(1);
    let before = stores.board.served().len();
    let copied = stores.json(&["task", "copy", NOTE, "--to", "board"]);
    assert_eq!(copied["items"][0]["via"], "intent", "{copied:#}");
    assert_eq!(stores.board.items_titled("Task A").len(), 1);
    let cursors: Vec<Value> = stores.board.served()[before..]
        .iter()
        .filter(|(document, variables)| lists_by_key(document, variables))
        .map(|(_, variables)| variables["after"].clone())
        .collect();
    // Task B's page, Task A's, and the first issue created before the intent's window, where
    // the listing stops: each page from the cursor the one before it answered.
    assert_eq!(
        cursors,
        vec![Value::Null, json!("1"), json!("2")],
        "{cursors:?}"
    );
}

#[test]
fn an_intent_entry_that_is_not_one_is_refused_and_nothing_is_created() {
    let stores = Stores::new();
    std::fs::write(
        stores.notes.join("tasks/A.md"),
        "---\ntitle: Task A\nstatus: todo\nmetadata:\n  onetaskgraph.creating:\n    board: 5\n\
         ---\nThe body.\n",
    )
    .expect("a malformed intent");
    let output = stores.run(&["--json", "task", "copy", NOTE, "--to", "board"], None);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("onetaskgraph.creating"),
        "names the entry: {}",
        stderr(&output)
    );
    assert!(stores.board.items_titled("Task A").is_empty());
}

#[test]
fn a_linear_create_answered_as_an_id_already_held_is_read_by_that_id_and_written_whole() {
    let stores = Stores::new();
    let stopped = stores.run(
        &["task", "copy", NOTE, "--to", "linear"],
        Some("after-intent"),
    );
    assert_ne!(stopped.status.code(), Some(0));
    let key = stores.source_metadata()["onetaskgraph.creating"]["linear"]["key"]
        .as_str()
        .expect("the intent's key")
        .to_owned();
    // The earlier attempt's `issueCreate` lands only once the retry has looked for it and not
    // found it: the retry's own create, sent with the same id, is answered as an id taken.
    let landed = key.clone();
    stores.linear.after_request(
        move |document, variables| {
            document == onetaskgraph_linear::graphql::ISSUE && variables["id"] == json!(landed)
        },
        0,
        move |workspace| {
            workspace.add_issue(json!({
                "id": key, "title": "Task A, as first sent", "content": "The body.",
                "status": {"name": "Todo", "category": "todo"},
                "_linear_state": {"name": "Todo", "type": "unstarted"},
                "labels": [], "priority": "none",
                "metadata": {"onetaskgraph.origin": NOTE},
            }));
        },
    );
    let copied = stores.json(&["task", "copy", NOTE, "--to", "linear"]);
    let destination = copied["items"][0]["destination"].as_str().expect("an id");
    let landed = stores.carrying_origin("linear");
    assert_eq!(landed.len(), 1, "one issue: {landed:#?}");
    let shown = stores.json(&["task", "show", destination]);
    assert_eq!(
        shown["items"][0]["item"]["title"], "Task A",
        "written whole: {shown:#}"
    );
    assert!(
        stores
            .source_metadata()
            .get("onetaskgraph.creating")
            .is_none()
    );
}

#[test]
fn two_copies_of_one_never_copied_task_started_at_once_make_one_issue_and_one_link() {
    // A lease long enough that the loser cannot outlast it within the test.
    let stores = Stores::leased(600);
    let spawn = || {
        stores
            .sandbox
            .subprocess(onetaskgraph_e2e_support::binary())
            .args(["--json", "task", "copy", NOTE, "--to", "board"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("a copy starts")
    };
    let (first, second) = (spawn(), spawn());
    let outputs = [
        first.wait_with_output().expect("the first copy ends"),
        second.wait_with_output().expect("the second copy ends"),
    ];
    let mut linked = 0;
    for output in &outputs {
        let answer: Value = serde_json::from_slice(&output.stdout).expect("JSON");
        match output.status.code() {
            // It linked the one issue — the one it created, or the one the other created.
            Some(0) => {
                linked += 1;
                assert!(answer["items"][0]["destination"].is_string(), "{answer:#}");
            }
            // Or it found the other's live lease and wrote nothing, naming the lease's end.
            Some(1) => {
                assert_eq!(
                    answer["failure"]["kind"], "create-in-progress",
                    "{answer:#}"
                );
            }
            code => panic!("{code:?}: {answer:#}"),
        }
    }
    assert!(linked >= 1, "one of them links the issue");
    assert_eq!(
        stores.board.items_titled("Task A").len(),
        1,
        "exactly one issue: {}\n---\n{}\n---\n{}\n---\n{}",
        String::from_utf8_lossy(&outputs[0].stdout),
        String::from_utf8_lossy(&outputs[0].stderr),
        String::from_utf8_lossy(&outputs[1].stdout),
        String::from_utf8_lossy(&outputs[1].stderr)
    );
    let metadata = stores.source_metadata();
    assert!(
        metadata["onetaskgraph.copies"]["board"].is_string(),
        "{metadata:#}"
    );
    assert!(
        metadata.get("onetaskgraph.creating").is_none(),
        "{metadata:#}"
    );
}
