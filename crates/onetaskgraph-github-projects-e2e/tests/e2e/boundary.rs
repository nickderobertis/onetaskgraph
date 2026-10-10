//! A GitHub board declared private, held to its own live visibility at every write.
//!
//! The board is private for a write only when its Project and the repository the issue lives
//! in are both private, and each write reads both, with the source's own credential, before
//! its first mutation. Every journey spawns the binary against the loopback board and asserts
//! on what it said and on every request the board was sent — a refused write is held to having
//! sent no mutation at all.

use std::path::PathBuf;
use std::process::Output;

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{
    GitHubBoardFields, RepositoryAnswer, document, github_projects_with_board,
    github_projects_with_items,
};

/// The repository the fixture board creates an issue in when nothing else decides.
const CONFIGURED: &str = "nickderobertis/onetaskgraph";

/// One sandbox: a private folder of Markdown `plan`, and the fixture board declared private.
struct Setup {
    sandbox: Sandbox,
    board: GitHubBoardFields,
    root: PathBuf,
}

impl Setup {
    fn new() -> Self {
        Self::over(github_projects_with_board)
    }

    /// The same, over the board `board` opens.
    fn over(board: impl FnOnce(&Sandbox) -> (Value, GitHubBoardFields)) -> Self {
        let sandbox = Sandbox::new();
        let root = sandbox.subdirectory("plan");
        let (config, board) = board(&sandbox);
        sandbox.project_document(&document(&json!({
            "plan": {"plugin": "local-md", "config": {"root": root}, "visibility": "private"},
            "board": {"plugin": "github-projects", "config": config, "visibility": "private"},
        })));
        Self {
            sandbox,
            board,
            root,
        }
    }

    fn record(&self, kind: &str, id: &str, front: &str) {
        let path = self.root.join(kind).join(format!("{id}.md"));
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("the folder");
        std::fs::write(&path, format!("---\n{front}\n---\nBody of {id}.\n")).expect("a record");
    }

    fn run(&self, arguments: &[&str]) -> (Output, Vec<String>) {
        let before = self.board.served().len();
        let output = self
            .sandbox
            .command()
            .args(arguments)
            .assert()
            .get_output()
            .clone();
        let served = self.board.served()[before..]
            .iter()
            .map(|(document, _)| document.clone())
            .collect();
        (output, served)
    }

    fn ok(&self, arguments: &[&str]) -> Vec<String> {
        let (output, served) = self.run(arguments);
        assert_eq!(
            output.status.code(),
            Some(0),
            "`onetaskgraph {}` failed\n{}",
            arguments.join(" "),
            stderr(&output)
        );
        served
    }

    /// The failure document's kind and message of a run that had to be refused, having sent
    /// the board no mutation.
    fn refused(&self, arguments: &[&str]) -> (String, String, Vec<String>) {
        let mut with_json = arguments.to_vec();
        with_json.push("--json");
        let (output, served) = self.run(&with_json);
        assert_eq!(
            output.status.code(),
            Some(1),
            "`onetaskgraph {}` was expected to be refused\n{}{}",
            arguments.join(" "),
            stdout(&output),
            stderr(&output)
        );
        assert!(
            !served.iter().any(|document| is_mutation(document)),
            "a refused write sent the board a mutation: {served:#?}"
        );
        let failure: Value = serde_json::from_str(&stdout(&output)).expect("a failure document");
        (
            failure["failure"]["kind"]
                .as_str()
                .expect("a kind")
                .to_owned(),
            failure["failure"]["message"]
                .as_str()
                .expect("a message")
                .to_owned(),
            served,
        )
    }
}

fn is_mutation(document: &str) -> bool {
    document.trim_start().starts_with("mutation")
}

/// The two reads a write to a board declared private spends learning who can read it.
fn visibility_reads(served: &[String]) -> Vec<&str> {
    served
        .iter()
        .filter_map(|document| {
            if document == onetaskgraph_github_projects::graphql::PROJECT_VISIBILITY {
                Some("project")
            } else if document.starts_with("GET /repos/") {
                Some("repository")
            } else {
                None
            }
        })
        .collect()
}

#[test]
fn a_private_task_lands_on_a_board_whose_project_and_repository_are_both_private() {
    let setup = Setup::new();
    setup.record(
        "tasks",
        "secret",
        "title: Secret\nstatus: todo\nclassification: private",
    );
    let served = setup.ok(&["task", "copy", "plan:secret", "--to", "board"]);
    let mut reads = visibility_reads(&served);
    reads.sort_unstable();
    assert_eq!(reads, ["project", "repository"], "{served:#?}");
    let first_mutation = served
        .iter()
        .position(|document| is_mutation(document))
        .expect("the copy wrote");
    assert!(
        served[..first_mutation]
            .iter()
            .filter(|document| visibility_reads(std::slice::from_ref(document)).len() == 1)
            .count()
            == 2,
        "both reads come before the first mutation: {served:#?}"
    );
    assert!(served.contains(&format!("GET /repos/{CONFIGURED}")));
}

#[test]
fn a_board_whose_project_or_issue_repository_is_public_is_refused_before_any_mutation() {
    for (project_public, repository) in [
        (false, RepositoryAnswer::Public),
        (true, RepositoryAnswer::Private),
    ] {
        let setup = Setup::new();
        setup.board.set_project_public(project_public);
        setup
            .board
            .set_repository_visibility(CONFIGURED, repository);
        setup.record(
            "tasks",
            "secret",
            "title: Secret\nstatus: todo\nclassification: private",
        );
        let (kind, message, _) = setup.refused(&["task", "copy", "plan:secret", "--to", "board"]);
        assert_eq!(kind, "destination-not-private", "{message}");
        assert!(message.contains("reads public"), "{message}");
        // A public item is held to the declaration too: a board declared private that is not.
        setup.record("tasks", "open", "title: Open\nstatus: todo");
        let (kind, _, _) = setup.refused(&["task", "copy", "plan:open", "--to", "board"]);
        assert_eq!(kind, "destination-not-private");
    }
}

#[test]
fn a_repository_named_by_the_task_is_the_one_whose_visibility_is_read() {
    let setup = Setup::new();
    setup
        .board
        .set_repository_visibility("nickderobertis/elsewhere", RepositoryAnswer::Public);
    setup.record(
        "tasks",
        "named",
        "title: Named\nstatus: todo\nclassification: private\n\
         repositories: [github.com/nickderobertis/elsewhere]",
    );
    let (kind, _, served) = setup.refused(&["task", "copy", "plan:named", "--to", "board"]);
    assert_eq!(kind, "destination-not-private");
    assert!(
        served.contains(&"GET /repos/nickderobertis/elsewhere".to_owned()),
        "{served:#?}"
    );
}

#[test]
fn visibility_is_read_again_at_every_write_and_a_change_between_writes_never_passes() {
    let setup = Setup::new();
    setup.record(
        "tasks",
        "first",
        "title: First\nstatus: todo\nclassification: private",
    );
    setup.record(
        "tasks",
        "second",
        "title: Second\nstatus: todo\nclassification: private",
    );
    let served = setup.ok(&["task", "copy", "plan:first", "--to", "board"]);
    assert_eq!(visibility_reads(&served).len(), 2);
    // Somebody makes the board public between two writes.
    setup.board.set_project_public(true);
    let (kind, _, served) = setup.refused(&["task", "copy", "plan:second", "--to", "board"]);
    assert_eq!(kind, "destination-not-private");
    assert_eq!(
        visibility_reads(&served).len(),
        2,
        "read again, not remembered"
    );
    // And a narrow write to the item already there is refused as well, before it is sent.
    let landed: Value = serde_json::from_str(&stdout(
        &setup.run(&["task", "show", "plan:first", "--json"]).0,
    ))
    .expect("the task");
    let link = landed["items"][0]["item"]["metadata"]["onetaskgraph.copies"]["board"]
        .as_str()
        .unwrap_or_else(|| panic!("the copy recorded where it landed: {landed:#}"))
        .to_owned();
    let (kind, _, served) = setup.refused(&["task", "status", "set", &link, "done"]);
    assert_eq!(kind, "destination-not-private");
    assert_eq!(visibility_reads(&served).len(), 2);
}

#[test]
fn an_unreadable_visibility_is_never_guessed() {
    let setup = Setup::new();
    setup.record(
        "tasks",
        "secret",
        "title: Secret\nstatus: todo\nclassification: private",
    );
    // A repository the credential cannot see, an answer that is not JSON, and one whose
    // `visibility` is there but is not a visibility: none is read as either answer.
    for answer in [
        RepositoryAnswer::Unseen,
        RepositoryAnswer::NotJson,
        RepositoryAnswer::MalformedVisibility,
    ] {
        setup.board.set_repository_visibility(CONFIGURED, answer);
        let (kind, message, _) = setup.refused(&["task", "copy", "plan:secret", "--to", "board"]);
        assert_eq!(kind, "visibility-unreadable", "{answer:?}: {message}");
        assert!(message.contains("never guessed"), "{answer:?}: {message}");
    }
}

#[test]
fn an_internal_repository_is_private_and_an_answer_without_a_visibility_reads_its_flag() {
    for (answer, lands) in [
        (RepositoryAnswer::Internal, true),
        (RepositoryAnswer::FlagOnly { private: true }, true),
        (RepositoryAnswer::FlagOnly { private: false }, false),
    ] {
        let setup = Setup::new();
        setup.board.set_repository_visibility(CONFIGURED, answer);
        setup.record(
            "tasks",
            "secret",
            "title: Secret\nstatus: todo\nclassification: private",
        );
        let arguments = ["task", "copy", "plan:secret", "--to", "board"];
        if lands {
            let served = setup.ok(&arguments);
            assert!(
                served.iter().any(|document| is_mutation(document)),
                "{answer:?}"
            );
        } else {
            let (kind, message, _) = setup.refused(&arguments);
            assert_eq!(kind, "destination-not-private", "{answer:?}: {message}");
        }
    }
}

#[test]
fn a_draft_belongs_to_no_repository_so_the_board_alone_decides() {
    let setup = Setup::over(|sandbox| {
        github_projects_with_items(
            sandbox,
            vec![
                json!({"item": "ITEM-SKETCH-1", "id": "SKETCH-1", "type": "DraftIssue",
                "title": "Sketch", "body": "a draft", "state": "OPEN", "reason": null,
                "parent": null, "repo": null, "status": "Todo", "origin": "", "labels": []}),
            ],
        )
    });
    let served = setup.ok(&["task", "status", "set", "board:SKETCH-1", "in-progress"]);
    assert_eq!(visibility_reads(&served), ["project"], "{served:#?}");
    setup.board.set_project_public(true);
    let (kind, _, served) = setup.refused(&["task", "status", "set", "board:SKETCH-1", "todo"]);
    assert_eq!(kind, "destination-not-private");
    assert_eq!(visibility_reads(&served), ["project"], "{served:#?}");
}

#[test]
fn a_new_task_filed_under_a_project_is_held_to_that_projects_repository() {
    let setup = Setup::over(|sandbox| {
        github_projects_with_items(
            sandbox,
            vec![
                json!({"item": "ITEM-HOME-1", "id": "HOME-1", "type": "Issue",
                "title": "Home", "body": "Home.", "state": "OPEN", "reason": null,
                "parent": null, "repo": "nickderobertis/elsewhere", "status": "Todo",
                "origin": "", "labels": []}),
            ],
        )
    });
    // The configured repository is private, so only the project's own repository can refuse.
    setup
        .board
        .set_repository_visibility("nickderobertis/elsewhere", RepositoryAnswer::Public);
    let body = setup.sandbox.config_home().join("body.md");
    std::fs::write(&body, "A body.").expect("a body file");
    let (kind, _, served) = setup.refused(&[
        "task",
        "create",
        "board",
        "--project",
        "HOME-1",
        "--title",
        "Filed",
        "--body-file",
        body.to_str().expect("a UTF-8 path"),
    ]);
    assert_eq!(kind, "destination-not-private");
    assert!(
        served.contains(&"GET /repos/nickderobertis/elsewhere".to_owned()),
        "{served:#?}"
    );
    assert!(
        !served.contains(&format!("GET /repos/{CONFIGURED}")),
        "{served:#?}"
    );
}

#[test]
fn a_credential_without_the_project_read_scope_refuses_naming_the_scope() {
    let setup = Setup::new();
    setup.board.withhold_project_scope();
    setup.record(
        "tasks",
        "secret",
        "title: Secret\nstatus: todo\nclassification: private",
    );
    let (kind, message, served) = setup.refused(&["task", "copy", "plan:secret", "--to", "board"]);
    assert_eq!(kind, "visibility-unreadable", "{message}");
    assert!(
        message.contains("`read:project`") && message.contains("GITHUB_PROJECTS_FIXTURE_TOKEN"),
        "names the missing scope and the credential it is missing from: {message}"
    );
    assert!(
        !message.contains("test-token"),
        "the credential's value is never said: {message}"
    );
    // Every request this run sent carried the source's own credential — the loopback board
    // refuses any other — and none of them was a write.
    assert!(served.iter().all(|document| !is_mutation(document)));
}

#[test]
fn a_board_with_no_declared_visibility_spends_nothing_learning_it() {
    // The inactive store: a board nothing declares is never asked who can read it.
    let sandbox = Sandbox::new();
    let root = sandbox.subdirectory("plan");
    let (config, board) = github_projects_with_board(&sandbox);
    sandbox.project_document(&document(&json!({
        "plan": {"plugin": "local-md", "config": {"root": root}},
        "board": {"plugin": "github-projects", "config": config},
    })));
    std::fs::create_dir_all(root.join("tasks")).expect("the folder");
    std::fs::write(
        root.join("tasks/open.md"),
        "---\ntitle: Open\nstatus: todo\n---\nBody.\n",
    )
    .expect("a record");
    let output = sandbox
        .command()
        .args(["task", "copy", "plan:open", "--to", "board"])
        .assert()
        .get_output()
        .clone();
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let served: Vec<String> = board
        .served()
        .into_iter()
        .map(|(document, _)| document)
        .collect();
    assert!(visibility_reads(&served).is_empty(), "{served:#?}");
}

#[test]
fn a_dry_run_reads_the_board_it_would_write_to_and_reports_the_refusal_its_copy_would_meet() {
    let setup = Setup::new();
    setup.record(
        "tasks",
        "secret",
        "title: Secret\nstatus: todo\nclassification: private",
    );
    let arguments = ["task", "copy", "plan:secret", "--to", "board", "--dry-run"];
    // A board that reads private: the dry run reads it once, as the copy's write would, and
    // writes nothing.
    let served = setup.ok(&arguments);
    let mut reads = visibility_reads(&served);
    reads.sort_unstable();
    assert_eq!(reads, ["project", "repository"], "{served:#?}");
    assert!(
        !served.iter().any(|document| is_mutation(document)),
        "{served:#?}"
    );
    // One a person has made public: the dry run is refused exactly as the copy would be.
    setup.board.set_project_public(true);
    let (kind, _, served) = setup.refused(&arguments);
    assert_eq!(kind, "destination-not-private");
    assert!(!visibility_reads(&served).is_empty(), "{served:#?}");
}

#[test]
fn a_board_made_public_part_way_through_a_copy_refuses_the_next_write_and_undoes_the_ones_before() {
    let setup = Setup::new();
    setup.record(
        "projects",
        "goal",
        "title: Goal\nstatus: todo\nclassification: private",
    );
    for task in ["first", "second"] {
        setup.record(
            "tasks",
            task,
            &format!("title: {task}\nstatus: todo\nproject: goal\nclassification: private"),
        );
    }
    // Everything the board holds, tasks and projects alike.
    let listed = || {
        ["task", "project"]
            .iter()
            .map(|kind| {
                let (output, _) = setup.run(&[kind, "list", "--source", "board", "--json"]);
                let listed: Value = serde_json::from_str(&stdout(&output)).expect("a listing");
                listed["items"].as_array().expect("items").len()
            })
            .sum::<usize>()
    };
    let before = listed();
    // The project's write reads private; a person makes the board public before the next.
    setup.board.make_public_after_visibility_reads(1);
    let (output, served) = setup.run(&["project", "copy", "plan:goal", "--to", "board", "--json"]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    let failure: Value = serde_json::from_str(&stdout(&output)).expect("a failure document");
    assert_eq!(
        failure["failure"]["kind"], "destination-not-private",
        "{failure:#}"
    );
    assert!(
        served
            .iter()
            .any(|document| document.contains("createIssue(input:$input)")),
        "the first write landed before the change: {served:#?}"
    );
    assert!(
        served
            .iter()
            .any(|document| document.contains("deleteIssue(input:$input)")),
        "and was taken back: {served:#?}"
    );
    assert_eq!(
        listed(),
        before,
        "the board holds nothing of the refused copy"
    );
}

#[test]
fn a_private_task_project_and_document_read_back_private_from_the_board() {
    let setup = Setup::new();
    setup.record(
        "projects",
        "goal",
        "title: Goal\nstatus: todo\nclassification: private",
    );
    setup.record(
        "tasks",
        "step",
        "title: Step\nstatus: todo\nproject: goal\nclassification: private",
    );
    setup.record(
        "documents",
        "design",
        "title: Design\nproject: goal\nclassification: private",
    );
    let (output, _) = setup.run(&["project", "copy", "plan:goal", "--to", "board", "--json"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let (output, _) = setup.run(&["document", "copy", "plan:design", "--to", "board", "--json"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let document: Value = serde_json::from_str(&stdout(&output)).expect("a copy report");
    let landed = |kind: &str, id: &str| {
        let (output, _) = setup.run(&[kind, "show", id, "--json"]);
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        let shown: Value = serde_json::from_str(&stdout(&output)).expect("a show");
        shown["items"][0]["item"].clone()
    };
    let project = landed("project", "plan:goal");
    let task = landed("task", "plan:step");
    let copies = |item: &Value| {
        item["metadata"]["onetaskgraph.copies"]["board"]
            .as_str()
            .unwrap_or_else(|| panic!("where it landed: {item:#}"))
            .to_owned()
    };
    let document_at = document["items"][0]["destination"]
        .as_str()
        .unwrap_or_else(|| panic!("where the document landed: {document:#}"))
        .to_owned();
    for (kind, id) in [
        ("project", copies(&project)),
        ("task", copies(&task)),
        ("document", document_at),
    ] {
        let read = landed(kind, &id);
        assert_eq!(read["classification"], "private", "{kind}: {read:#}");
        // The reserved key is how the board keeps it, never the caller's own metadata.
        assert!(
            read["metadata"]
                .get("onetaskgraph.classification")
                .is_none(),
            "{kind}: {read:#}"
        );
    }
}

#[test]
fn a_board_item_whose_stored_classification_is_malformed_is_refused_rather_than_read_public() {
    let setup = Setup::over(|sandbox| {
        github_projects_with_items(
            sandbox,
            vec![json!({"item": "ITEM-ODD-1", "id": "ODD-1", "type": "Issue",
                "title": "Odd", "state": "OPEN", "reason": null, "parent": null,
                "repo": "nickderobertis/onetaskgraph", "status": "Todo", "origin": "",
                "labels": [],
                "body": "Odd.\n\n<!-- onetaskgraph.metadata\n{\"onetaskgraph.classification\":\"secret\"}\n-->"})],
        )
    });
    let (output, _) = setup.run(&["task", "show", "board:ODD-1"]);
    assert_ne!(output.status.code(), Some(0), "{}", stdout(&output));
    assert!(
        stderr(&output).contains("onetaskgraph.classification"),
        "{}",
        stderr(&output)
    );
}
