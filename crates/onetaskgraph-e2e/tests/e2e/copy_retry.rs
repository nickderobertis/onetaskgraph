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
fn a_create_stopped_after_its_intent_holds_it_and_its_retry_creates_under_that_key() {
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
        assert_eq!(
            key,
            documented_key(NOTE, destination, None),
            "{destination}: the key the README states"
        );

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
        assert_eq!(
            created["metadata"]["onetaskgraph.create_key"],
            json!(key),
            "{destination}: the key is kept verbatim: {created:#}"
        );
        if destination == "linear" {
            // Linear holds the issue under the key's v4-shaped id, the format its schema takes.
            assert_eq!(
                created["id"],
                json!(onetaskgraph_linear::linear_id_of_key(&key).expect("a key")),
                "{created:#}"
            );
        }
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
    for (variable, key) in [
        (
            "ONETASKGRAPH_SOURCES__NOTES__CREATE_LEASE_SECONDS",
            "sources.notes.create_lease_seconds",
        ),
        (
            "ONETASKGRAPH_SOURCES__BOARD__CREATE_QUIESCENCE_SECONDS",
            "sources.board.create_quiescence_seconds",
        ),
    ] {
        for value in ["0", "86401"] {
            let output = stores
                .sandbox
                .command()
                .env(variable, value)
                .args(["config", "show"])
                .assert()
                .get_output()
                .clone();
            assert_ne!(output.status.code(), Some(0), "{key} {value}");
            assert!(
                stderr(&output).contains(key),
                "{key} {value}: {}",
                stderr(&output)
            );
        }
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
    // Not an object at all, and one whose time is past what any copy could have written.
    let key = "8213b96a-ad84-54be-990c-798d8c12c97b";
    let attempt = "5db60da7-f0de-4044-9aec-2fa2f9f4be74";
    for entry in [
        "5".to_owned(),
        format!(
            "{{\"key\": \"{key}\", \"at\": \"1969-12-31T23:59:59Z\", \"attempt\": \
             \"{attempt}\", \"lease_until\": \"2020-01-01T00:00:00Z\"}}"
        ),
        format!(
            "{{\"key\": \"{key}\", \"at\": \"2020-01-01T00:00:00Z\", \"attempt\": \
             \"{attempt}\", \"lease_until\": \"0001-01-01T00:00:00Z\"}}"
        ),
        // A key and an attempt of the wrong version each: neither is what a copy writes.
        format!(
            "{{\"key\": \"{attempt}\", \"at\": \"2020-01-01T00:00:00Z\", \"attempt\": \
             \"{attempt}\", \"lease_until\": \"2020-01-01T00:00:00Z\"}}"
        ),
        format!(
            "{{\"key\": \"{key}\", \"at\": \"2020-01-01T00:00:00Z\", \"attempt\": \
             \"{key}\", \"lease_until\": \"2020-01-01T00:00:00Z\"}}"
        ),
    ] {
        let stores = Stores::new();
        std::fs::write(
            stores.notes.join("tasks/A.md"),
            format!(
                "---\ntitle: Task A\nstatus: todo\nmetadata:\n  onetaskgraph.creating:\n    \
                 board: {entry}\n---\nThe body.\n"
            ),
        )
        .expect("a malformed intent");
        let output = stores.run(&["--json", "task", "copy", NOTE, "--to", "board"], None);
        assert_eq!(
            output.status.code(),
            Some(1),
            "{entry}: {}",
            stderr(&output)
        );
        assert!(
            stderr(&output).contains("onetaskgraph.creating"),
            "{entry}: names the entry: {}",
            stderr(&output)
        );
        assert!(stores.board.items_titled("Task A").is_empty(), "{entry}");
    }
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
    // Linear holds it under the v4-shaped id the key is sent as.
    let key = onetaskgraph_linear::linear_id_of_key(&key).expect("a key");
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

/// Whether a request is a board write the engine sends after a create's own: a comment added.
fn adds_comment(document: &str, _: &Value) -> bool {
    document.contains("addComment(")
}

/// The intent the note's file records for the board, read off the file as it stands.
fn board_intent(notes: &std::path::Path) -> Option<(String, String)> {
    let text = std::fs::read_to_string(notes.join("tasks/A.md")).ok()?;
    let line = text
        .lines()
        .find(|line| line.contains("onetaskgraph.creating"))?;
    let value: Value = serde_json::from_str(line.split_once(':')?.1.trim()).ok()?;
    let entry = &value["board"];
    Some((
        entry["attempt"].as_str()?.to_owned(),
        entry["lease_until"].as_str()?.to_owned(),
    ))
}

/// A note with one comment, copied with `--comments` onto the board from a folder leasing its
/// intents for two seconds, the board holding its next answer for three once it has answered
/// the create: the comment write that follows is sent after the first lease would have run out.
fn slow_create_with_a_comment() -> Stores {
    let stores = Stores::leased(2);
    let body = stores.sandbox.project().join("comment.md");
    std::fs::write(&body, "A word.\n").expect("a comment body");
    stores.json(&[
        "task",
        "comment",
        "add",
        NOTE,
        "--body-file",
        body.to_str().expect("a path"),
    ]);
    stores.board.after_request(creates_issue, 0, |_| {
        std::thread::sleep(std::time::Duration::from_secs(3));
    });
    stores
}

#[test]
fn a_holder_renews_its_lease_before_each_destination_write_after_its_create() {
    let stores = slow_create_with_a_comment();
    let seen = std::sync::Arc::new(std::sync::Mutex::new(None));
    let notes = stores.notes.clone();
    let record = seen.clone();
    // Read the source's intent at the moment the comment write is answered: the holder's, and
    // leased past now.
    stores.board.after_request(adds_comment, 0, move |_| {
        *record.lock().unwrap() = Some((board_intent(&notes), chrono::Utc::now()));
    });
    let copied = stores.json(&["task", "copy", NOTE, "--to", "board", "--comments"]);
    assert_eq!(copied["items"][0]["action"], "created", "{copied:#}");
    let (intent, at) = seen
        .lock()
        .unwrap()
        .clone()
        .expect("the comment was written");
    let (_, until) = intent.expect("the intent stood while the comment was written");
    let until = chrono::DateTime::parse_from_rfc3339(&until).expect("a time");
    assert!(
        until > at,
        "the lease was renewed before the comment write: it ran until {until}, written at {at}"
    );
    assert!(
        stores
            .source_metadata()
            .get("onetaskgraph.creating")
            .is_none()
    );
}

#[test]
fn a_holder_that_finds_another_attempt_holding_its_intent_writes_nothing_more() {
    // The note has a comment, so after its create the copy has one more board write to send.
    let stores = Stores::new();
    let body = stores.sandbox.project().join("comment.md");
    std::fs::write(&body, "A word.\n").expect("a comment body");
    stores.json(&[
        "task",
        "comment",
        "add",
        NOTE,
        "--body-file",
        body.to_str().expect("a path"),
    ]);
    // The first copy creates the issue and is held there until its one-second lease has run
    // out, as a copy stalled after its create would be.
    let first = Paused::copying(&stores, &[NOTE], "first", &["after-create"]);
    first.reached("after-create");
    let (holder, _) = board_intent(&stores.notes).expect("the first copy's intent");
    std::thread::sleep(std::time::Duration::from_millis(1500));
    // A second copy finds the issue by its key and takes the create over, then dies.
    let second = stores.run(
        &["task", "copy", NOTE, "--to", "board", "--comments"],
        Some("after-intent"),
    );
    assert_eq!(second.status.code(), Some(86), "{}", stderr(&second));
    let (taker, _) = board_intent(&stores.notes).expect("the second copy's intent");
    assert_ne!(taker, holder, "the second copy holds the intent now");

    let creates = count(&stores.board, creates_issue);
    first.release("after-create");
    let (code, failure) = first.finish();
    assert_eq!(code, Some(1), "{failure:#}");
    assert_eq!(
        failure["failure"]["kind"], "create-in-progress",
        "{failure:#}"
    );
    assert_eq!(
        count(&stores.board, creates_issue),
        creates,
        "no create after the loss"
    );
    assert_eq!(
        count(&stores.board, adds_comment),
        0,
        "no comment written after the loss"
    );
    // Nothing was taken back: the issue is the new holder's to resolve, and its intent stands.
    assert_eq!(stores.board.items_titled("Task A").len(), 1);
    let (attempt, _) = board_intent(&stores.notes).expect("the new holder's intent");
    assert_eq!(attempt, taker);
}

#[test]
fn a_holder_whose_create_another_copy_has_since_finished_and_linked_writes_nothing_more() {
    let stores = Stores::new();
    let body = stores.sandbox.project().join("comment.md");
    std::fs::write(&body, "A word.\n").expect("a comment body");
    stores.json(&[
        "task",
        "comment",
        "add",
        NOTE,
        "--body-file",
        body.to_str().expect("a path"),
    ]);
    let first = Paused::copying(&stores, &[NOTE], "first", &["after-create"]);
    first.reached("after-create");
    std::thread::sleep(std::time::Duration::from_millis(1500));
    // A second copy finds the first's issue by its key, takes the create over, finishes it —
    // its comment included — links it and removes the intent.
    let finished = stores.json(&["task", "copy", NOTE, "--to", "board", "--comments"]);
    assert_eq!(finished["items"][0]["via"], "intent", "{finished:#}");
    let linked = finished["items"][0]["destination"]
        .as_str()
        .expect("the counterpart")
        .to_owned();
    assert!(
        board_intent(&stores.notes).is_none(),
        "the intent is settled"
    );
    let comments = count(&stores.board, adds_comment);

    first.release("after-create");
    let (code, failure) = first.finish();
    assert_eq!(code, Some(1), "{failure:#}");
    assert_eq!(
        failure["failure"]["kind"], "create-in-progress",
        "{failure:#}"
    );
    assert!(
        failure["failure"]["message"]
            .as_str()
            .is_some_and(|message| message.contains(&linked)),
        "names the counterpart the other copy linked: {failure:#}"
    );
    assert_eq!(
        count(&stores.board, adds_comment),
        comments,
        "no comment written after the loss"
    );
    assert_eq!(stores.board.items_titled("Task A").len(), 1);
    assert_eq!(
        stores.source_metadata()["onetaskgraph.copies"]["board"],
        json!(linked)
    );
}

#[test]
fn a_holder_renews_before_each_comment_write_and_stops_between_them_once_taken_over() {
    let stores = Stores::new();
    for word in ["A first word.\n", "A second word.\n"] {
        let body = stores.sandbox.project().join("comment.md");
        std::fs::write(&body, word).expect("a comment body");
        stores.json(&[
            "task",
            "comment",
            "add",
            NOTE,
            "--body-file",
            body.to_str().expect("a path"),
        ]);
    }
    // The first copy creates the issue, writes its first comment, and is held before its
    // second until its one-second lease has run out.
    let first = Paused::copying(&stores, &[NOTE], "first", &["before-comment-1"]);
    first.reached("before-comment-1");
    assert_eq!(
        count(&stores.board, adds_comment),
        1,
        "the first comment landed"
    );
    std::thread::sleep(std::time::Duration::from_millis(1500));
    // A second copy takes the create over, then dies.
    let second = stores.run(
        &["task", "copy", NOTE, "--to", "board", "--comments"],
        Some("after-intent"),
    );
    assert_eq!(second.status.code(), Some(86), "{}", stderr(&second));

    first.release("before-comment-1");
    let (code, failure) = first.finish();
    assert_eq!(code, Some(1), "{failure:#}");
    assert_eq!(
        failure["failure"]["kind"], "create-in-progress",
        "{failure:#}"
    );
    assert_eq!(
        count(&stores.board, adds_comment),
        1,
        "no comment written after the loss"
    );
}

#[test]
fn a_listing_by_key_the_board_answers_malformed_is_refused_and_nothing_is_created_again() {
    type Bend = fn(&mut Value);
    let cases: [(&str, Bend); 7] = [
        ("issues that are not a list", |issues| {
            issues["nodes"] = json!({});
        }),
        ("a body that is not text", |issues| {
            issues["nodes"][0]["body"] = json!(5);
        }),
        ("a cursor that does not move", |issues| {
            issues["pageInfo"] = json!({"hasNextPage": true, "endCursor": "the same page"});
        }),
        ("no creation time", |issues| {
            issues["nodes"][0]
                .as_object_mut()
                .map(|node| node.remove("createdAt"));
        }),
        ("no id", |issues| {
            issues["nodes"][0]
                .as_object_mut()
                .map(|node| node.remove("id"));
        }),
        ("no word on another page", |issues| {
            issues["pageInfo"]
                .as_object_mut()
                .map(|page| page.remove("hasNextPage"));
        }),
        ("another page with no cursor to it", |issues| {
            issues["pageInfo"] = json!({"hasNextPage": true, "endCursor": null});
        }),
    ];
    for (what, bend) in cases {
        let stores = Stores::new();
        stores.board.drop_response_once("createIssue");
        let failed = stores.run(&["task", "copy", NOTE, "--to", "board"], None);
        assert_ne!(failed.status.code(), Some(0), "{what}: {}", stderr(&failed));
        stores.board.bend_issue_listings(bend);
        // One issue a page, so a page stops short of the issues older than the intent and says
        // whether another follows.
        stores.board.answer_issue_listings_in_pages_of(1);
        let creates = count(&stores.board, creates_issue);
        let output = stores.run(&["--json", "task", "copy", NOTE, "--to", "board"], None);
        assert_eq!(output.status.code(), Some(1), "{what}: {}", stderr(&output));
        let failure: Value = serde_json::from_str(&stdout(&output)).expect("a failure document");
        assert_eq!(
            failure["failure"]["kind"], "malformed",
            "{what}: {failure:#}"
        );
        assert!(
            failure["failure"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("issues")),
            "{what}: {failure:#}"
        );
        assert_eq!(
            count(&stores.board, creates_issue),
            creates,
            "{what}: nothing created"
        );
        assert_eq!(stores.board.items_titled("Task A").len(), 1, "{what}");
    }
}

/// One copy process of a Linear issue onto the board, waiting at each point `pauses` names in
/// its own directory under the sandbox.
struct Paused {
    child: std::process::Child,
    directory: PathBuf,
}

impl Paused {
    fn start(stores: &Stores, issue: &str, name: &str, points: &[&str]) -> Self {
        Self::copying(stores, &[issue], name, points)
    }

    /// A copy of `items` onto the board, with `--comments` when the first is the note.
    fn copying(stores: &Stores, items: &[&str], name: &str, points: &[&str]) -> Self {
        let directory = stores.sandbox.subdirectory(name);
        let spec = points
            .iter()
            .map(|point| format!("{point}={}", directory.display()))
            .collect::<Vec<_>>()
            .join(",");
        let child = stores
            .sandbox
            .subprocess(onetaskgraph_e2e_support::binary())
            .env("OTG_TEST_PAUSE_AT", spec)
            .args(["--json", "task", "copy"])
            .args(items)
            .args(["--to", "board"])
            .args((items.first() == Some(&NOTE)).then_some("--comments"))
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("a copy starts");
        Self { child, directory }
    }

    /// Wait until the process waits at `point`.
    fn reached(&self, point: &str) {
        let started = std::time::Instant::now();
        while !self.directory.join(format!("{point}.paused")).exists() {
            assert!(
                started.elapsed() < std::time::Duration::from_secs(30),
                "the copy reached {point}"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    /// Let the process go on past `point`.
    fn release(&self, point: &str) {
        std::fs::write(self.directory.join(format!("{point}.go")), b"").expect("released");
    }

    /// The process's exit code and the JSON it answered.
    fn finish(self) -> (Option<i32>, Value) {
        let output = self.child.wait_with_output().expect("the copy ends");
        let answer = serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
            panic!(
                "JSON: {}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        });
        (output.status.code(), answer)
    }
}

/// A Linear issue never copied onto the board, as a hosted source for the two copies.
fn a_linear_issue(stores: &Stores) -> String {
    let copied = stores.json(&["task", "copy", NOTE, "--to", "linear"]);
    copied["items"][0]["destination"]
        .as_str()
        .expect("an issue")
        .to_owned()
}

/// The issues on the board carrying `issue`'s origin.
fn board_issues_of(stores: &Stores, issue: &str) -> Vec<String> {
    stores.json(&["task", "list", "--source", "board", "--origin", issue])["items"]
        .as_array()
        .expect("a page")
        .iter()
        .map(|item| item["id"].as_str().expect("an id").to_owned())
        .collect()
}

/// Assert `answer` refuses the copy as `create-duplicated`, naming every one of `issues`.
fn refused_as_duplicated(answer: &Value, issues: &[String]) {
    let error = &answer["items"][0]["error"];
    assert_eq!(error["kind"], "create-duplicated", "{answer:#}");
    let message = error["message"].as_str().expect("a message");
    for issue in issues {
        assert!(message.contains(issue.as_str()), "names {issue}: {message}");
    }
}

#[test]
fn two_hosted_source_copies_whose_creates_both_land_before_either_relist_both_refuse() {
    let stores = Stores::new();
    let issue = a_linear_issue(&stores);
    let points = ["intent-read", "after-create"];
    let first = Paused::start(&stores, &issue, "first", &points);
    let second = Paused::start(&stores, &issue, "second", &points);
    // Both read the issue before either records its intent: the lease is advisory.
    first.reached("intent-read");
    second.reached("intent-read");
    first.release("intent-read");
    second.release("intent-read");
    // Both create before either lists the repository by the key again.
    first.reached("after-create");
    second.reached("after-create");
    first.release("after-create");
    second.release("after-create");
    let (first_code, first_answer) = first.finish();
    let (second_code, second_answer) = second.finish();
    let issues = board_issues_of(&stores, &issue);
    assert_eq!(issues.len(), 2, "both created: {issues:?}");
    assert_eq!((first_code, second_code), (Some(4), Some(4)));
    refused_as_duplicated(&first_answer, &issues);
    refused_as_duplicated(&second_answer, &issues);
    let linked = stores.json(&["task", "show", &issue])["items"][0]["item"]["metadata"]
        ["onetaskgraph.copies"]
        .clone();
    assert!(linked.get("board").is_none(), "no link: {linked:#}");
}

#[test]
fn a_hosted_source_copy_whose_relist_ran_first_links_and_the_later_create_refuses() {
    let stores = Stores::new();
    let issue = a_linear_issue(&stores);
    let first = Paused::start(&stores, &issue, "first", &["intent-read"]);
    let second = Paused::start(&stores, &issue, "second", &["intent-read"]);
    first.reached("intent-read");
    second.reached("intent-read");
    // The first creates, lists the repository by its key and links before the second creates.
    first.release("intent-read");
    let (first_code, first_answer) = first.finish();
    second.release("intent-read");
    let (second_code, second_answer) = second.finish();
    let issues = board_issues_of(&stores, &issue);
    assert_eq!(issues.len(), 2, "both created: {issues:?}");
    assert_eq!(first_code, Some(0), "{first_answer:#}");
    let linked = first_answer["items"][0]["destination"]
        .as_str()
        .expect("the first linked its issue")
        .to_owned();
    assert_eq!(second_code, Some(4), "{second_answer:#}");
    refused_as_duplicated(&second_answer, &issues);
    let held = stores.json(&["task", "show", &issue])["items"][0]["item"]["metadata"]
        ["onetaskgraph.copies"]["board"]
        .clone();
    assert_eq!(held, json!(linked), "exactly the first's link");
}

#[test]
fn two_hosted_source_copies_onto_linear_at_once_make_one_issue_under_one_id() {
    let stores = Stores::new();
    let copied = stores.json(&["task", "copy", NOTE, "--to", "board"]);
    let issue = copied["items"][0]["destination"]
        .as_str()
        .expect("a board issue")
        .to_owned();
    let start = |name: &str| {
        let directory = stores.sandbox.subdirectory(name);
        let child = stores
            .sandbox
            .subprocess(onetaskgraph_e2e_support::binary())
            .env(
                "OTG_TEST_PAUSE_AT",
                format!("intent-read={}", directory.display()),
            )
            .args(["--json", "task", "copy", &issue, "--to", "linear"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("a copy starts");
        Paused { child, directory }
    };
    let (first, second) = (start("first"), start("second"));
    first.reached("intent-read");
    second.reached("intent-read");
    first.release("intent-read");
    second.release("intent-read");
    let outcomes = [first.finish(), second.finish()];
    let landed = stores.json(&["task", "list", "--source", "linear", "--origin", &issue])["items"]
        .as_array()
        .expect("a page")
        .len();
    assert_eq!(landed, 1, "one issue under the one id: {outcomes:#?}");
    for (code, answer) in &outcomes {
        match code {
            Some(0) => assert!(answer["items"][0]["destination"].is_string(), "{answer:#}"),
            _ => assert_eq!(
                answer["failure"]["kind"], "create-in-progress",
                "{answer:#}"
            ),
        }
    }
    let links = stores.json(&["task", "show", &issue])["items"][0]["item"]["metadata"]
        ["onetaskgraph.copies"]
        .clone();
    assert!(links["linear"].is_string(), "one link: {links:#}");
}

#[test]
fn a_linear_create_whose_answer_never_arrived_whole_is_found_by_its_retry_under_its_id() {
    let stores = Stores::new();
    // Linear stores the issue and its answer to `issueCreate` says nothing of it, as an answer
    // lost on the way would: the copy cannot vouch for what it made, and keeps its intent.
    stores.linear.bend_answers(
        |document, _| document == onetaskgraph_linear::graphql::ISSUE_CREATE,
        1,
        |_, data| *data = json!({"issueCreate": null}),
    );
    let failed = stores.run(&["task", "copy", NOTE, "--to", "linear"], None);
    assert_ne!(failed.status.code(), Some(0), "{}", stderr(&failed));
    assert!(stores.source_metadata()["onetaskgraph.creating"]["linear"].is_object());
    let copied = stores.json(&["task", "copy", NOTE, "--to", "linear"]);
    assert_eq!(copied["items"][0]["via"], "intent", "{copied:#}");
    assert_eq!(
        stores.carrying_origin("linear").len(),
        1,
        "exactly one issue"
    );
    let metadata = stores.source_metadata();
    assert!(
        metadata.get("onetaskgraph.creating").is_none(),
        "{metadata:#}"
    );
    assert_eq!(
        metadata["onetaskgraph.copies"]["linear"],
        copied["items"][0]["destination"]
    );
}

/// The key the README states a create is made under: the version-5 UUID, under the documented
/// namespace, of `<source>|<destination>|<replaced link>`, with the kind appended for a project
/// or a document — computed here from that statement, independently of the engine.
fn documented_key(source: &str, destination: &str, kind: Option<&str>) -> String {
    replacing_key(source, destination, "", kind)
}

/// [`documented_key`] of a create that replaces the link naming `replaces`.
fn replacing_key(source: &str, destination: &str, replaces: &str, kind: Option<&str>) -> String {
    let namespace =
        uuid::Uuid::parse_str("7a1c3f0e-5b2d-4c8a-9e61-0f4d2b7c5a39").expect("the namespace");
    let mut name = format!("{source}|{destination}|{replaces}");
    if let Some(kind) = kind {
        name.push_str(&format!("|{kind}"));
    }
    uuid::Uuid::new_v5(&namespace, name.as_bytes())
        .hyphenated()
        .to_string()
}

/// A project and a document of the notes, each copied on its own.
fn with_a_project_and_a_document(stores: &Stores) {
    std::fs::create_dir_all(stores.notes.join("projects")).expect("the project folder");
    std::fs::create_dir_all(stores.notes.join("documents")).expect("the document folder");
    std::fs::create_dir_all(stores.folder.join("projects")).expect("the project folder");
    std::fs::create_dir_all(stores.folder.join("documents")).expect("the document folder");
    std::fs::write(
        stores.notes.join("projects/P.md"),
        "---\ntitle: Project P\nstatus: todo\n---\nThe plan.\n",
    )
    .expect("the project file");
    std::fs::write(
        stores.notes.join("documents/D.md"),
        "---\ntitle: Document D\n---\nThe design.\n",
    )
    .expect("the document file");
}

/// The command copying one `kind` of record, and the id it copies.
fn record_copy(kind: &str, destination: &str) -> (Vec<String>, &'static str) {
    let id = if kind == "project" {
        "notes:P"
    } else {
        "notes:D"
    };
    let mut arguments = vec![kind, "copy", id, "--to", destination];
    if kind == "project" {
        arguments.push("--no-tasks");
    }
    (arguments.into_iter().map(str::to_owned).collect(), id)
}

/// The metadata of the `kind` record `id`, as `<kind> show` reads it.
fn record_metadata(stores: &Stores, kind: &str, id: &str) -> Value {
    stores.json(&[kind, "show", id])["items"][0]["item"]["metadata"].clone()
}

/// Every `kind` record of `destination` titled `title`.
fn records_titled(stores: &Stores, kind: &str, destination: &str, title: &str) -> Vec<Value> {
    stores.json(&[kind, "list", "--source", destination])["items"]
        .as_array()
        .expect("a page")
        .iter()
        .filter(|item| item["item"]["title"] == title)
        .map(|item| item["item"].clone())
        .collect()
}

#[test]
fn a_project_or_document_copy_stopped_at_either_point_of_its_create_is_finished_by_its_retry() {
    for kind in ["project", "document"] {
        let title = if kind == "project" {
            "Project P"
        } else {
            "Document D"
        };
        for destination in ["folder", "board", "linear"] {
            for point in ["after-intent", "before-link"] {
                let stores = Stores::new();
                with_a_project_and_a_document(&stores);
                let (arguments, id) = record_copy(kind, destination);
                let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
                let stopped = stores.run(&arguments, Some(point));
                assert_eq!(
                    stopped.status.code(),
                    Some(86),
                    "{kind} {destination} {point}: stopped there: {}",
                    stderr(&stopped)
                );
                let intent =
                    &record_metadata(&stores, kind, id)["onetaskgraph.creating"][destination];
                assert!(
                    intent["key"].is_string(),
                    "{kind} {destination} {point}: the intent is recorded: {intent:#}"
                );
                let key = intent["key"].as_str().expect("a key").to_owned();
                assert_eq!(
                    key,
                    documented_key(id, destination, Some(kind)),
                    "{kind} {destination}: the key the README states"
                );

                let mut full = vec!["--json"];
                full.extend(&arguments);
                let copied = stores.json(&arguments);
                let outcome = &copied["items"][0];
                if point == "before-link" {
                    assert_eq!(
                        outcome["via"], "intent",
                        "{kind} {destination} {point}: {copied:#}"
                    );
                } else {
                    assert_eq!(
                        outcome["action"], "created",
                        "{kind} {destination} {point}: {copied:#}"
                    );
                }
                let landed = records_titled(&stores, kind, destination, title);
                assert_eq!(
                    landed.len(),
                    1,
                    "{kind} {destination} {point}: exactly one: {landed:#?}"
                );
                assert_eq!(
                    landed[0]["metadata"]["onetaskgraph.create_key"],
                    json!(key),
                    "{kind} {destination} {point}: the key is kept verbatim"
                );
                if destination == "linear" {
                    assert_eq!(
                        landed[0]["id"],
                        json!(onetaskgraph_linear::linear_id_of_key(&key).expect("a key")),
                    );
                }
                let metadata = record_metadata(&stores, kind, id);
                assert!(
                    metadata.get("onetaskgraph.creating").is_none(),
                    "{kind} {destination} {point}: no intent is left: {metadata:#}"
                );
                assert_eq!(
                    metadata["onetaskgraph.copies"][destination],
                    outcome["destination"]
                );
            }
        }
    }
}

#[test]
fn a_project_copy_whose_create_is_taken_back_leaves_its_file_byte_for_byte() {
    let stores = Stores::new();
    with_a_project_and_a_document(&stores);
    let before = std::fs::read(stores.notes.join("projects/P.md")).expect("the file");
    // A refused create — a source that answers it — leaves nothing at the destination, so the
    // copy takes every write back, its intent included.
    stores.linear.bend_answers(
        |document, _| document == onetaskgraph_linear::graphql::PROJECT_CREATE,
        1,
        |_, data| *data = json!({"projectCreate": {"success": false, "project": null}}),
    );
    let (arguments, _) = record_copy("project", "linear");
    let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
    let failed = stores.run(&arguments, None);
    assert_ne!(failed.status.code(), Some(0), "{}", stderr(&failed));
    assert_eq!(
        std::fs::read(stores.notes.join("projects/P.md")).expect("the file"),
        before,
        "the project file is as it was"
    );
}

#[test]
fn a_linear_project_or_document_create_answered_as_an_id_held_is_read_by_that_id_and_written_whole()
{
    for kind in ["project", "document"] {
        let stores = Stores::new();
        with_a_project_and_a_document(&stores);
        let (arguments, id) = record_copy(kind, "linear");
        let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
        let stopped = stores.run(&arguments, Some("after-intent"));
        assert_eq!(
            stopped.status.code(),
            Some(86),
            "{kind}: {}",
            stderr(&stopped)
        );
        let key = record_metadata(&stores, kind, id)["onetaskgraph.creating"]["linear"]["key"]
            .as_str()
            .expect("the intent's key")
            .to_owned();
        // The earlier attempt's create lands only once the retry has looked for it by its id and
        // not found it: the retry's own create, sent with the same id, is refused as taken.
        let held = onetaskgraph_linear::linear_id_of_key(&key).expect("a key");
        let looked_up = if kind == "project" {
            onetaskgraph_linear::graphql::PROJECT
        } else {
            onetaskgraph_linear::graphql::DOCUMENT
        };
        let landed = held.clone();
        stores.linear.after_request(
            move |document, variables| document == looked_up && variables["id"] == json!(landed),
            0,
            move |workspace| {
                if kind == "project" {
                    workspace.add_project(json!({
                        "id": held, "title": "Project P, as first sent", "content": "The plan.",
                        "status": {"category": "todo", "name": "Todo"}, "labels": [],
                    }));
                } else {
                    workspace.add_document(json!({
                        "id": held, "title": "Document D, as first sent",
                        "content": "The design.", "labels": [],
                    }));
                }
            },
        );
        let title = if kind == "project" {
            "Project P"
        } else {
            "Document D"
        };
        let before = stores.linear.served().len();
        let copied = stores.json(&arguments);
        // Its create was sent, refused as an id taken, the item read by that id and written
        // whole after it.
        let (create, rewrite) = if kind == "project" {
            (
                onetaskgraph_linear::graphql::PROJECT_CREATE,
                onetaskgraph_linear::graphql::PROJECT_REWRITE,
            )
        } else {
            (
                onetaskgraph_linear::graphql::DOCUMENT_CREATE,
                onetaskgraph_linear::graphql::DOCUMENT_UPDATE,
            )
        };
        let sent: Vec<String> = stores.linear.served()[before..]
            .iter()
            .map(|(document, _)| document.clone())
            .collect();
        let after_create = sent
            .iter()
            .position(|document| document == create)
            .map(|created| &sent[created + 1..])
            .unwrap_or_default();
        let read = after_create
            .iter()
            .position(|document| document == looked_up);
        assert!(
            read.is_some_and(|read| after_create[read + 1..].contains(&rewrite.to_owned())),
            "{kind}: a create, a read by its id, then a whole write: {sent:#?}"
        );
        let destination = copied["items"][0]["destination"]
            .as_str()
            .expect("an id")
            .to_owned();
        assert_eq!(
            destination,
            format!(
                "linear:{}",
                onetaskgraph_linear::linear_id_of_key(&key).unwrap()
            ),
            "{kind}: the item the earlier attempt made: {copied:#}"
        );
        let all = stores.json(&[kind, "list", "--source", "linear"])["items"]
            .as_array()
            .expect("a page")
            .iter()
            .filter(|item| {
                item["item"]["title"]
                    .as_str()
                    .is_some_and(|shown| shown.starts_with(title))
            })
            .count();
        assert_eq!(all, 1, "{kind}: one item");
        let shown = &stores.json(&[kind, "show", &destination])["items"][0]["item"];
        assert_eq!(shown["title"], title, "{kind}: written whole: {shown:#}");
        assert!(
            record_metadata(&stores, kind, id)
                .get("onetaskgraph.creating")
                .is_none(),
            "{kind}"
        );
    }
}

/// Plant a lock beside the note's file dated an hour ahead of this host's clock: another host's,
/// whose age this one cannot tell.
fn lock_ahead(notes: &std::path::Path) -> PathBuf {
    let lock = notes.join("tasks/A.md.lock");
    let file = std::fs::File::create(&lock).expect("another host's lock");
    file.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(3600))
        .expect("an hour ahead");
    lock
}

#[test]
fn a_copy_whose_source_cannot_give_its_hold_is_refused_before_any_destination_request() {
    let stores = Stores::new();
    let lock = lock_ahead(&stores.notes);
    let before = std::fs::read(stores.notes.join("tasks/A.md")).expect("the note");
    let creates = count(&stores.board, creates_issue);
    let output = stores.run(&["--json", "task", "copy", NOTE, "--to", "board"], None);
    assert_ne!(output.status.code(), Some(0), "{}", stdout(&output));
    assert!(
        stderr(&output).contains("ahead of this host's clock"),
        "names the lock: {}",
        stderr(&output)
    );
    assert_eq!(
        count(&stores.board, creates_issue),
        creates,
        "nothing created"
    );
    assert_eq!(
        std::fs::read(stores.notes.join("tasks/A.md")).expect("the note"),
        before,
        "nothing written to the note"
    );
    // Once the lock is gone, the copy goes through as it always did.
    std::fs::remove_file(lock).expect("removed");
    let copied = stores.json(&["task", "copy", NOTE, "--to", "board"]);
    assert_eq!(copied["items"][0]["action"], "created", "{copied:#}");
}

#[test]
fn a_copy_that_cannot_put_its_source_back_leaves_the_intent_for_its_retry_to_resolve() {
    let stores = Stores::new();
    // Linear refuses the create, and the note's file can no longer be held when the copy comes to
    // put it back: neither the byte-for-byte restore nor taking the intent off can be made.
    let notes = stores.notes.clone();
    stores.linear.bend_answers(
        |document, _| document == onetaskgraph_linear::graphql::ISSUE_CREATE,
        1,
        move |_, data| {
            lock_ahead(&notes);
            *data = json!({"issueCreate": {"success": false, "issue": null}});
        },
    );
    let failed = stores.run(&["--json", "task", "copy", NOTE, "--to", "linear"], None);
    assert_ne!(failed.status.code(), Some(0), "{}", stdout(&failed));
    let failure: Value = serde_json::from_str(&stdout(&failed)).expect("a failure document");
    assert!(
        !failure["failure"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("ahead of this host's clock"),
        "the copy reports why it failed, not why it could not tidy up: {failure:#}"
    );
    let lock = stores.notes.join("tasks/A.md.lock");
    let text = std::fs::read_to_string(stores.notes.join("tasks/A.md")).expect("the note");
    assert!(
        text.contains("onetaskgraph.creating"),
        "the intent stands: {text}"
    );
    // The lock gone and the lease run out, the retry resolves the intent and lands one issue.
    std::fs::remove_file(lock).expect("removed");
    std::thread::sleep(std::time::Duration::from_millis(1500));
    stores.json(&["task", "copy", NOTE, "--to", "linear"]);
    assert_eq!(stores.carrying_origin("linear").len(), 1, "one issue");
    assert!(
        stores
            .source_metadata()
            .get("onetaskgraph.creating")
            .is_none()
    );
}

#[test]
fn a_recreate_after_a_stale_link_takes_a_key_of_its_own_and_its_retry_keeps_it() {
    let stores = Stores::new();
    let first = stores.json(&["task", "copy", NOTE, "--to", "folder"]);
    let stale = first["items"][0]["destination"]
        .as_str()
        .expect("an id")
        .to_owned();
    // The copy is deleted at the destination, so the note's link names nothing there.
    let file = stale.trim_start_matches("folder:");
    std::fs::remove_file(stores.folder.join("tasks").join(format!("{file}.md")))
        .expect("deleted at the destination");
    let recreate = ["task", "copy", NOTE, "--to", "folder", "--recreate"];
    let stopped = stores.run(&recreate, Some("after-intent"));
    assert_eq!(stopped.status.code(), Some(86), "{}", stderr(&stopped));
    let key = stores.source_metadata()["onetaskgraph.creating"]["folder"]["key"]
        .as_str()
        .expect("the intent's key")
        .to_owned();
    assert_eq!(
        key,
        replacing_key(NOTE, "folder", &stale, None),
        "the key names the link it replaces"
    );
    assert_ne!(key, documented_key(NOTE, "folder", None));
    let copied = stores.json(&recreate);
    let landed = copied["items"][0]["destination"].as_str().expect("an id");
    assert_eq!(copied["items"][0]["action"], "created", "{copied:#}");
    let created = &stores.json(&["task", "show", landed])["items"][0]["item"];
    assert_eq!(
        created["metadata"]["onetaskgraph.create_key"],
        json!(key),
        "the retry kept the key its first attempt took"
    );
}

#[test]
fn a_source_that_will_not_keep_the_key_is_copied_as_it_always_was_with_no_intent() {
    // Metadata written on one line, which the folder will not edit one key of without
    // rewriting the rest: the intent cannot be recorded, so the copy goes on without it.
    let stores = Stores::new();
    let text = "---\ntitle: Task A\nstatus: todo\nmetadata: {caller.tag: one}\n---\nThe body.\n";
    std::fs::write(stores.notes.join("tasks/A.md"), text).expect("the note");
    let copied = stores.json(&["task", "copy", NOTE, "--to", "board"]);
    assert_eq!(copied["items"][0]["action"], "created", "{copied:#}");
    assert_eq!(copied["items"][0]["link"], "unrecorded", "{copied:#}");
    assert_eq!(stores.board.items_titled("Task A").len(), 1);
    assert_eq!(
        std::fs::read_to_string(stores.notes.join("tasks/A.md")).expect("the note"),
        text,
        "no intent and no link written"
    );
}

#[test]
fn a_source_record_gone_before_its_intent_is_copied_as_it_always_was_with_no_intent() {
    let stores = Stores::new();
    let issue = a_linear_issue(&stores);
    let native = issue.trim_start_matches("linear:").to_owned();
    // The copy reads the issue to plan it, and Linear trashes it before the intent is taken.
    let reads = stores.linear.served().len();
    let trashed = native.clone();
    stores.linear.after_request(
        move |document, variables| {
            document == onetaskgraph_linear::graphql::ISSUE && variables["id"] == json!(native)
        },
        0,
        move |workspace| workspace.trash_issue(&trashed),
    );
    let output = stores.run(&["--json", "task", "copy", &issue, "--to", "board"], None);
    let sent: Vec<String> = stores.linear.served()[reads..]
        .iter()
        .map(|(document, _)| document.chars().take(60).collect())
        .collect();
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}\n{sent:#?}",
        stderr(&output)
    );
    let copied: Value = serde_json::from_str(&stdout(&output)).expect("a report");
    assert_eq!(copied["items"][0]["action"], "created", "{copied:#}");
    assert_eq!(copied["items"][0]["link"], "unrecorded", "{copied:#}");
    assert_eq!(board_issues_of(&stores, &issue).len(), 1, "{sent:#?}");
}

#[test]
fn an_intent_found_malformed_when_the_copy_comes_to_take_it_refuses_the_copy() {
    let stores = Stores::new();
    let creates = count(&stores.board, creates_issue);
    // Planned on a note with no intent; held just before the intent is read under the hold.
    let copy = Paused::copying(&stores, &[NOTE], "copy", &["before-intent"]);
    copy.reached("before-intent");
    std::fs::write(
        stores.notes.join("tasks/A.md"),
        "---\ntitle: Task A\nstatus: todo\nmetadata:\n  onetaskgraph.creating:\n    board: 5\n\
         ---\nThe body.\n",
    )
    .expect("an entry that is not an intent");
    copy.release("before-intent");
    let (code, failure) = copy.finish();
    assert_eq!(code, Some(1), "{failure:#}");
    assert_eq!(failure["failure"]["kind"], "malformed", "{failure:#}");
    assert!(
        failure["failure"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("onetaskgraph.creating")),
        "{failure:#}"
    );
    assert_eq!(
        count(&stores.board, creates_issue),
        creates,
        "nothing created"
    );
}
