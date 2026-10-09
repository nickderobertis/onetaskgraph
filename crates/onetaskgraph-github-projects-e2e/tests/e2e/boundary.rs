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
use crate::fixtures::{GitHubBoardFields, document, github_projects_with_board};

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
        let sandbox = Sandbox::new();
        let root = sandbox.subdirectory("plan");
        let (config, board) = github_projects_with_board(&sandbox);
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
    for (project_public, repository) in [(false, "public"), (true, "private")] {
        let setup = Setup::new();
        setup.board.set_project_public(project_public);
        setup
            .board
            .set_repository_visibility(CONFIGURED, Some(repository));
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
        .set_repository_visibility("nickderobertis/elsewhere", Some("public"));
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
    setup.board.set_repository_visibility(CONFIGURED, None);
    setup.record(
        "tasks",
        "secret",
        "title: Secret\nstatus: todo\nclassification: private",
    );
    let (kind, message, _) = setup.refused(&["task", "copy", "plan:secret", "--to", "board"]);
    assert_eq!(kind, "visibility-unreadable", "{message}");
    assert!(message.contains("never guessed"), "{message}");
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
