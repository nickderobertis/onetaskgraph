//! What a `task update` and a bound re-copy of one existing GitHub board item cost, what each
//! applies, and what a write refused part-way leaves behind.
//!
//! Every journey here spawns the compiled binary against the loopback fixture board and
//! asserts on exit code, stdout and stderr, and on the requests the board served. The
//! guarantee being held is the one a caller retrying into a following write relies on: **a
//! write refused part-way leaves the item's body and metadata exactly as they stood.** GitHub
//! runs no two requests as one, and runs a document's mutation fields in order without undoing
//! an earlier field when a later one fails, so the source writes an existing item's board
//! fields and relationships first and its content — the body, and the metadata slot inside
//! it — last. Each write such an update or re-copy performs is refused here in turn, as a whole
//! request and, for the batched field write, as one aliased field failing after the field
//! before it landed.

use std::path::PathBuf;
use std::process::Output;

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{GitHubBoardFields, document, github_projects_with_board};

/// One sandbox configuring the fixture board as `board` and a folder of Markdown as `plans`.
struct Setup {
    sandbox: Sandbox,
    board: GitHubBoardFields,
    root: PathBuf,
}

impl Setup {
    fn new() -> Self {
        let sandbox = Sandbox::new();
        let root = sandbox.subdirectory("plans");
        std::fs::create_dir_all(root.join("projects")).expect("the project folder");
        std::fs::create_dir_all(root.join("tasks")).expect("the task folder");
        let (config, board) = github_projects_with_board(&sandbox);
        sandbox.project_document(&document(&json!({
            "plans": {"plugin":"local-md","config":{
                "root": root,
                "status_mapping": {"Todo":"todo","Doing":"in-progress"}}},
            "board": {"plugin":"github-projects","config":config}
        })));
        Self {
            sandbox,
            board,
            root,
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

    /// The JSON a command printed, having had to succeed, and the documents the board served
    /// for it.
    fn ok(&self, arguments: &[&str]) -> (Value, Vec<String>) {
        let before = self.board.served().len();
        let output = self.run(arguments);
        assert_eq!(
            output.status.code(),
            Some(0),
            "`onetaskgraph {}` failed\n{}",
            arguments.join(" "),
            stderr(&output)
        );
        let served = self.board.served()[before..]
            .iter()
            .map(|(document, _)| document.clone())
            .collect();
        let printed = serde_json::from_str(&stdout(&output)).expect("the command emits JSON");
        (printed, served)
    }

    /// A command that had to be refused, and what it said.
    fn refused(&self, arguments: &[&str]) -> String {
        let output = self.run(arguments);
        let said = stderr(&output);
        assert_ne!(
            output.status.code(),
            Some(0),
            "`onetaskgraph {}` was meant to be refused\n{}",
            arguments.join(" "),
            stdout(&output)
        );
        said
    }

    /// The item as a fresh read of it reports it.
    fn item(&self, id: &str) -> Value {
        let (shown, _) = self.ok(&["task", "show", id, "--no-comments", "--json"]);
        shown["items"][0]["item"].clone()
    }

    /// A file under the sandbox holding `text`, for `--body-file`.
    fn file(&self, name: &str, text: &str) -> String {
        let path = self.root.join(name);
        std::fs::write(&path, text).expect("a body file");
        path.to_str().expect("a UTF-8 path").to_owned()
    }

    fn task(&self, name: &str, front_matter: &str, body: &str) {
        std::fs::write(
            self.root.join(format!("tasks/{name}.md")),
            format!("---\n{front_matter}---\n{body}\n"),
        )
        .expect("a task file");
    }
}

/// The record a write must leave as it stood when it is refused part-way: the body a person
/// reads, and every metadata key the item carries.
fn record(item: &Value) -> (Value, Value) {
    (item["content"].clone(), item["metadata"].clone())
}

/// The one-read-then-writes shape both verbs are held to: the item's read by its own id
/// first, nothing else read, and the content written last when it is written at all.
fn one_read_then_writes(what: &str, served: &[String], most: usize) {
    use onetaskgraph_github_projects::graphql;
    assert!(served.len() <= most, "{what}: {served:#?}");
    assert_eq!(
        served[0],
        graphql::ISSUE,
        "{what}: one read of the item first"
    );
    assert!(
        served[1..]
            .iter()
            .all(|document| document.trim_start().starts_with("mutation")),
        "{what}: nothing read after the item: {served:#?}"
    );
    if let Some(at) = served
        .iter()
        .position(|document| document == graphql::UPDATE_ISSUE)
    {
        assert_eq!(
            at,
            served.len() - 1,
            "{what}: the content is written last: {served:#?}"
        );
    }
}

#[test]
fn a_task_update_applies_each_field_alone_and_all_together_in_one_read_and_two_writes() {
    let setup = Setup::new();
    let id = "board:T-1";
    let body = setup.file("body.md", "A new body.\n");
    for (what, flags, check) in [
        (
            "the title",
            vec!["--title", "Retitled"],
            ("title", json!("Retitled")),
        ),
        (
            "the body",
            vec!["--body-file", body.as_str()],
            ("content", json!("A new body.\n")),
        ),
        (
            "the status",
            vec!["--status", "in-progress"],
            ("status", json!({"category":"in-progress","name":"Doing"})),
        ),
        (
            "the priority",
            vec!["--priority", "low"],
            ("priority", json!("low")),
        ),
    ] {
        let mut arguments = vec!["task", "update", id];
        arguments.extend(flags);
        arguments.push("--json");
        let (_, served) = setup.ok(&arguments);
        one_read_then_writes(what, &served, 2);
        assert_eq!(setup.item(id)[check.0], check.1, "{what}");
    }
    let (_, served) = setup.ok(&[
        "task",
        "update",
        id,
        "--metadata",
        "caller.reviewer=\"lin\"",
        "--json",
    ]);
    one_read_then_writes("a metadata key", &served, 2);
    assert_eq!(setup.item(id)["metadata"]["caller.reviewer"], "lin");

    // All five at once: one read, then the status option and the priority in one request,
    // then the title, the body and the metadata slot in one `updateIssue`.
    let all = setup.file("all.md", "Every field moves.\n");
    let (answer, served) = setup.ok(&[
        "task",
        "update",
        id,
        "--title",
        "Everything",
        "--body-file",
        &all,
        "--status",
        "todo",
        "--priority",
        "urgent",
        "--metadata",
        "caller.reviewer=\"ada\"",
        "--json",
    ]);
    one_read_then_writes("every field", &served, 3);
    assert_eq!(served.len(), 3, "{served:#?}");
    assert_eq!(
        answer["written"],
        json!(["title", "content", "status", "priority", "metadata"])
    );
    let item = setup.item(id);
    assert_eq!(item["title"], "Everything");
    assert_eq!(item["content"], "Every field moves.\n");
    assert_eq!(item["status"]["category"], "todo");
    assert_eq!(item["priority"], "urgent");
    assert_eq!(item["metadata"]["caller.reviewer"], "ada");
}

/// What every refusal journey below names: the update that moves all five fields and the
/// dependency, so every write an update can make is one this update makes.
fn every_field_update<'a>(id: &'a str, body: &'a str, depends_on: &'a str) -> Vec<&'a str> {
    vec![
        "task",
        "update",
        id,
        "--title",
        "Refused",
        "--body-file",
        body,
        "--status",
        "in-progress",
        "--priority",
        "low",
        "--metadata",
        "caller.reviewer=\"refused\"",
        "--depends-on",
        depends_on,
    ]
}

#[test]
fn every_write_of_an_update_refused_leaves_the_body_and_metadata_as_they_stood() {
    // T-1 is blocked by T-2; naming T-3 instead is one removal and one addition.
    for (refusal, what) in [
        (
            Refusal::Request("updateProjectV2ItemFieldValue(input:$input)"),
            "the batched field write refused whole",
        ),
        (
            Refusal::Alias("second"),
            "the priority failing after the status option landed in the same request",
        ),
        (
            Refusal::Request("removeBlockedBy(input:$input)"),
            "the dependency removal refused",
        ),
        (
            Refusal::Request("addBlockedBy(input:$input)"),
            "the dependency addition refused",
        ),
        (
            Refusal::Request("updateIssue(input:$input)"),
            "the content update refused",
        ),
    ] {
        let setup = Setup::new();
        let id = "board:T-1";
        let before = setup.item(id);
        let body = setup.file("refused.md", "A body that must not land.\n");
        refusal.arm(&setup.board);
        let from = setup.board.served().len();
        let said = setup.refused(&every_field_update(id, &body, "board:T-3"));
        assert!(said.contains("next:"), "{what}: {said}");
        if !matches!(refusal, Refusal::Request("updateIssue(input:$input)")) {
            never_wrote(&setup, from, "A body that must not land.", what);
        }
        let after = setup.item(id);
        assert_eq!(record(&after), record(&before), "{what}");
        assert_eq!(after["title"], before["title"], "{what}");
        if let Refusal::Alias(_) = refusal {
            // The option before the failing field did land: the guarantee is the record's.
            assert_eq!(after["status"]["category"], "in-progress", "{what}");
            assert_eq!(after["priority"], before["priority"], "{what}");
        }
    }
}

/// How a journey makes one write of a command fail.
#[derive(Clone, Copy)]
enum Refusal {
    /// The whole request carrying this operation is refused.
    Request(&'static str),
    /// The batched field write's field under this alias fails, after the ones before it land.
    Alias(&'static str),
}

impl Refusal {
    fn arm(self, board: &GitHubBoardFields) {
        match self {
            Self::Request(operation) => board.refuse_once(operation),
            Self::Alias(alias) => board.fail_field_alias(alias),
        }
    }
}

/// A board holding tasks `B` and `C`, copied there out of the folder, and `A`, which `B`
/// blocks — named by its board id, as a re-copy bound to the board names it — with the ids
/// `A` and `B` and `C` landed on.
fn copied_tasks(setup: &Setup) -> (String, String, String) {
    setup.task("B", "title: Second\nstatus: Todo\n", "second");
    setup.task("C", "title: Third\nstatus: Todo\n", "third");
    let (copied, _) = setup.ok(&[
        "task", "copy", "plans:B", "plans:C", "--to", "board", "--json",
    ]);
    let b = copied["items"][0]["destination"]
        .as_str()
        .unwrap()
        .to_owned();
    let c = copied["items"][1]["destination"]
        .as_str()
        .unwrap()
        .to_owned();
    setup.task(
        "A",
        &format!("{}metadata:\n  caller.note: one\n", as_copied(&b)),
        "first body",
    );
    let (first, _) = setup.ok(&["task", "copy", "plans:A", "--to", "board", "--json"]);
    let a = first["items"][0]["destination"]
        .as_str()
        .unwrap()
        .to_owned();
    (a, b, c)
}

/// `A` as it stands on the board, spelled as the folder writes it — blocked by `b` — which
/// every re-copy below changes one thing of.
fn as_copied(b: &str) -> String {
    format!("title: First\nstatus: Todo\npriority: high\ndepends_on: [{{id: '{b}', item: task}}]\n")
}

/// `A` rewritten as a copy of the board item `a` — bound to it by its origin — with these
/// fields, its own metadata key, and `reviewer` as one more key when it is given.
fn rebound(setup: &Setup, a: &str, front: &str, reviewer: Option<&str>, body: &str) {
    let reviewer = reviewer.map_or_else(String::new, |who| format!("  caller.reviewer: {who}\n"));
    setup.task(
        "A",
        &format!("{front}metadata:\n  caller.note: one\n{reviewer}  onetaskgraph.origin: {a}\n"),
        body,
    );
}

#[test]
fn a_bound_recopy_applies_each_change_alone_and_all_together_in_three_requests() {
    let setup = Setup::new();
    let (a, b, _) = copied_tasks(&setup);
    let as_copied = as_copied(&b);
    for (what, front, reviewer, body, check) in [
        (
            "the title",
            as_copied.replace("First", "Renamed"),
            None,
            "first body",
            ("title", json!("Renamed")),
        ),
        (
            "the body",
            as_copied.clone(),
            None,
            "a new body",
            ("content", json!("a new body")),
        ),
        (
            "the status",
            as_copied.replace("status: Todo", "status: Doing"),
            None,
            "first body",
            ("status", json!({"category":"in-progress","name":"Doing"})),
        ),
        (
            "the priority",
            as_copied.replace("priority: high", "priority: urgent"),
            None,
            "first body",
            ("priority", json!("urgent")),
        ),
        (
            "a metadata key",
            as_copied.clone(),
            Some("lin"),
            "first body",
            (
                "metadata",
                json!({"caller.note":"one","caller.reviewer":"lin",
                                "onetaskgraph.origin":"plans:A"}),
            ),
        ),
    ] {
        rebound(&setup, &a, &front, reviewer, body);
        let (_, served) = setup.ok(&["task", "copy", "plans:A", "--to", "board", "--json"]);
        one_read_then_writes(what, &served, 3);
        let item = setup.item(&a);
        assert_eq!(item[check.0], check.1, "{what}");
        // Back as it was, so the next row changes one thing.
        rebound(&setup, &a, &as_copied, None, "first body");
        setup.ok(&["task", "copy", "plans:A", "--to", "board", "--json"]);
    }

    // Every change at once: still one read and two writes.
    rebound(
        &setup,
        &a,
        &as_copied
            .replace("First", "Everything")
            .replace("status: Todo", "status: Doing")
            .replace("priority: high", "priority: low"),
        Some("ada"),
        "every field moves",
    );
    let (_, served) = setup.ok(&["task", "copy", "plans:A", "--to", "board", "--json"]);
    one_read_then_writes("every change", &served, 3);
    assert_eq!(served.len(), 3, "{served:#?}");
    let item = setup.item(&a);
    assert_eq!(item["title"], "Everything");
    assert_eq!(item["content"], "every field moves");
    assert_eq!(item["status"]["category"], "in-progress");
    assert_eq!(item["priority"], "low");
    assert_eq!(item["metadata"]["caller.reviewer"], "ada");
}

/// No `updateIssue` this command sent carried `body` — the one place a refusal could have
/// left the new content behind had it been sent first.
fn never_wrote(setup: &Setup, from: usize, body: &str, what: &str) {
    let served = setup.board.served()[from..].to_vec();
    assert!(
        !served.iter().any(|(document, variables)| {
            document == onetaskgraph_github_projects::graphql::UPDATE_ISSUE
                && variables["input"]["body"]
                    .as_str()
                    .is_some_and(|sent| sent.contains(body))
        }),
        "{what}: the new content was sent before the refused write: {served:#?}"
    );
}

#[test]
fn every_write_of_a_bound_recopy_refused_leaves_the_body_and_metadata_as_they_stood() {
    // Every change a re-copy of A can write: its title, body, status and priority, a metadata
    // key, and the issues that block it — each refusal taken with all of them at once.
    for (refusal, what, blockers) in [
        (
            Refusal::Request("updateProjectV2ItemFieldValue(input:$input)"),
            "the batched field write refused whole",
            "[B]",
        ),
        (
            Refusal::Alias("second"),
            "the priority failing after the status option landed in the same request",
            "[B]",
        ),
        (
            Refusal::Request("removeBlockedBy(input:$input)"),
            "the dependency removal refused",
            "[]",
        ),
        (
            Refusal::Request("addBlockedBy(input:$input)"),
            "the dependency addition refused",
            "[B, C]",
        ),
        (
            Refusal::Request("updateIssue(input:$input)"),
            "the content update refused",
            "[B]",
        ),
    ] {
        let setup = Setup::new();
        let (a, b, c) = copied_tasks(&setup);
        let named = blockers
            .replace('B', &format!("{{id: '{b}', item: task}}"))
            .replace('C', &format!("{{id: '{c}', item: task}}"));
        let before = setup.item(&a);
        rebound(
            &setup,
            &a,
            &format!("title: Refused\nstatus: Doing\npriority: low\ndepends_on: {named}\n"),
            Some("refused"),
            "a body that must not land",
        );
        refusal.arm(&setup.board);
        let from = setup.board.served().len();
        setup.refused(&["task", "copy", "plans:A", "--to", "board", "--json"]);
        if !matches!(refusal, Refusal::Request("updateIssue(input:$input)")) {
            never_wrote(&setup, from, "a body that must not land", what);
        }
        let after = setup.item(&a);
        assert_eq!(record(&after), record(&before), "{what}");
        assert_eq!(after["title"], before["title"], "{what}");
    }
}

#[test]
fn a_bound_recopy_whose_project_move_is_refused_leaves_the_body_and_metadata_as_they_stood() {
    // A task filed under a project, re-copied out of it, with taking it out refused.
    let setup = Setup::new();
    std::fs::write(
        setup.root.join("projects/P.md"),
        "---\ntitle: The plan\nstatus: Doing\n---\nthe plan\n",
    )
    .expect("the project");
    setup.task(
        "A",
        "title: First\nstatus: Todo\nproject: P\n",
        "first body",
    );
    let (copied, _) = setup.ok(&["project", "copy", "plans:P", "--to", "board", "--json"]);
    let a = copied["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["source"] == "plans:A")
        .and_then(|item| item["destination"].as_str())
        .unwrap()
        .to_owned();
    let before = setup.item(&a);
    assert!(before["project"].is_string(), "{before}");
    rebound(
        &setup,
        &a,
        "title: Refused\nstatus: Doing\n",
        Some("refused"),
        "a body that must not land",
    );
    setup.board.refuse_once("removeSubIssue(input:$input)");
    let from = setup.board.served().len();
    setup.refused(&["task", "copy", "plans:A", "--to", "board", "--json"]);
    never_wrote(
        &setup,
        from,
        "a body that must not land",
        "the project move refused",
    );
    let after = setup.item(&a);
    assert_eq!(record(&after), record(&before));
    assert_eq!(after["title"], before["title"]);
    assert_eq!(after["project"], before["project"]);
}
