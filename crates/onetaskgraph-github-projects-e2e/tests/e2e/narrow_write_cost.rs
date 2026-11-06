//! What a narrow write costs a GitHub board under a policy that can rewrite: exactly what the same
//! write costs under one that only checks — every request compared, document and variables —
//! with its own text stored replaced. A term already stored is left for `sources sanitize`,
//! unless the store opted in to `write_policy.sanitize_stored_on_write`.
//!
//! The policy is the released onevcs, against a home this run owns, holding the invented private
//! identity `hiddenco/quietharbor`. Each measured write runs on a board of its own.

use onetaskgraph_e2e_support::{
    common::{Sandbox, stderr, stdout},
    fixtures::{GitHubBoardFields, document, github_projects_with_items},
    onevcs::{OnevcsHome, onevcs},
};
use serde_json::{Value, json};

/// The invented private term, and what the store replaces it with.
const TERM: &str = "quietharbor";
const WITH: &str = "harbor-x";
const TASK: &str = "board:X-901";
const NOTE: &str = "myapp.note";

#[derive(Clone, Copy)]
enum Policy {
    /// `visibility_command` and `check_command`, with no `sanitize_command`.
    Checks,
    /// A `sanitize_command` replacing [`TERM`] by [`WITH`].
    Sanitizes,
    /// The same, opted in to the read-first guard.
    Guards,
}

/// One board holding the one issue every write here is made to, on a public repository.
struct Board {
    sandbox: Sandbox,
    board: GitHubBoardFields,
    home: OnevcsHome,
    /// The configuration document, without its `write_policy`.
    configured: Value,
    /// The comment every board holds on the issue from the start.
    comment: String,
}

impl Board {
    fn holding(said: &str) -> Self {
        let sandbox = Sandbox::new();
        let (config, board) = github_projects_with_items(
            &sandbox,
            vec![json!({
                "item": "ITEM-X-901", "id": "X-901", "type": "Issue", "title": "Plain",
                "body": "Nothing private here.", "state": "OPEN", "reason": null, "parent": null,
                "repo": "nickderobertis/onetaskgraph", "status": "Todo", "origin": "",
                "labels": [],
            })],
        );
        let comment = board.add_comment("X-901", said);
        let mut home = OnevcsHome::new(&sandbox);
        home.private("hiddenco", TERM)
            .public("nickderobertis", "onetaskgraph");
        let configured = serde_json::from_str(&document(&json!({
            "board": {"plugin": "github-projects", "config": config, "visibility": "public"},
        })))
        .expect("a configuration document");
        sandbox.project_document(&Value::to_string(&configured));
        Self {
            sandbox,
            board,
            home,
            configured,
            comment,
        }
    }

    fn under(policy: Policy, said: &str) -> Self {
        let board = Self::holding(said);
        let mut configured = board.configured.clone();
        configured["write_policy"] = match policy {
            Policy::Checks => board.home.write_policy(),
            Policy::Sanitizes | Policy::Guards => {
                let mut sanitizing = board
                    .home
                    .sanitizing_write_policy(json!([{ "term": TERM, "with": WITH }]));
                if matches!(policy, Policy::Guards) {
                    sanitizing["sanitize_stored_on_write"] = json!(true);
                }
                sanitizing
            }
        };
        board.sandbox.project_document(&configured.to_string());
        board
    }

    fn clean(policy: Policy) -> Self {
        Self::under(policy, "A plain comment.")
    }

    /// `arguments` with `said` on standard input, succeeding: every request it sent the board.
    fn ran(&self, arguments: &[String], said: &str) -> Vec<(String, Value)> {
        let mark = self.board.served_count();
        let output = self
            .sandbox
            .command()
            .args(arguments)
            .env("ONEVCS_HOME", self.home.path())
            .write_stdin(said)
            .assert()
            .get_output()
            .clone();
        assert_eq!(
            output.status.code(),
            Some(0),
            "`onetaskgraph {}`\n{}{}",
            arguments.join(" "),
            stdout(&output),
            stderr(&output)
        );
        self.board.served_since(mark)
    }

    fn shown(&self) -> Value {
        let output = self
            .sandbox
            .command()
            .args(["task", "show", TASK, "--json"])
            .env("ONEVCS_HOME", self.home.path())
            .assert()
            .success()
            .get_output()
            .clone();
        let shown: Value = serde_json::from_str(&stdout(&output)).expect("a task");
        shown["items"][0]["item"].clone()
    }

    fn comments(&self) -> Vec<String> {
        self.board
            .comments_on("X-901")
            .iter()
            .map(|comment| comment["body"].as_str().expect("a body").to_owned())
            .collect()
    }
}

/// One narrow write of each kind, carrying `word` wherever it carries text, on `board`.
fn each_write(board: &Board, word: &str) -> Vec<(&'static str, Vec<String>, String)> {
    let said = format!("Retry the way {word} does.");
    let file = board.sandbox.subdirectory("written").join("content.md");
    std::fs::write(&file, &said).expect("the content");
    let file = file.to_string_lossy().into_owned();
    let words =
        |words: &[&str]| -> Vec<String> { words.iter().map(|word| (*word).to_owned()).collect() };
    let note = json!(said).to_string();
    vec![
        (
            "priority set",
            words(&["task", "priority", "set", TASK, "high", "--json"]),
            String::new(),
        ),
        (
            "status set",
            words(&["task", "status", "set", TASK, "in-progress", "--json"]),
            String::new(),
        ),
        (
            "content set",
            words(&["task", "content", "set", TASK, "--file", &file, "--json"]),
            String::new(),
        ),
        (
            "metadata set",
            words(&["task", "metadata", "set", TASK, NOTE, &note, "--json"]),
            String::new(),
        ),
        (
            "update",
            words(&[
                "task",
                "update",
                TASK,
                "--title",
                &format!("Port {word}"),
                "--body-file",
                &file,
                "--metadata",
                &format!("{NOTE}={note}"),
                "--json",
            ]),
            String::new(),
        ),
        (
            "comment add",
            words(&["task", "comment", "add", TASK, "--json"]),
            said.clone(),
        ),
        (
            "comment edit",
            words(&[
                "task",
                "comment",
                "edit",
                TASK,
                &board.comment,
                "--body-file",
                &file,
                "--json",
            ]),
            String::new(),
        ),
    ]
}

/// What `kind` wrote, as `board` stores it, held to carrying [`WITH`] and never [`TERM`].
fn holds_replaced(board: &Board, kind: &str) {
    let shown = board.shown();
    let replaced = format!("Retry the way {WITH} does.");
    let comments = board.comments();
    match kind {
        "content set" => assert_eq!(shown["content"], json!(replaced), "{shown}"),
        "metadata set" => assert_eq!(shown["metadata"][NOTE], json!(replaced), "{shown}"),
        "update" => {
            assert_eq!(shown["title"], json!(format!("Port {WITH}")), "{shown}");
            assert_eq!(shown["content"], json!(replaced), "{shown}");
            assert_eq!(shown["metadata"][NOTE], json!(replaced), "{shown}");
        }
        "comment add" => assert_eq!(comments.last(), Some(&replaced), "{comments:?}"),
        "comment edit" => assert_eq!(comments, [replaced], "{comments:?}"),
        _ => {}
    }
    let held = format!("{shown} {comments:?} {}", board.board.body("X-901"));
    assert!(!held.contains(TERM), "{kind}: {held}");
}

#[test]
fn every_narrow_write_under_a_sanitize_command_costs_what_it_costs_with_none() {
    let kinds = each_write(&Board::clean(Policy::Checks), WITH).len();
    for at in 0..kinds {
        let checks = Board::clean(Policy::Checks);
        let (kind, arguments, said) = each_write(&checks, WITH).swap_remove(at);
        let expected = checks.ran(&arguments, &said);
        assert!(!expected.is_empty(), "{kind}");

        let sanitizes = Board::clean(Policy::Sanitizes);
        let (_, arguments, said) = each_write(&sanitizes, TERM).swap_remove(at);
        let sent = sanitizes.ran(&arguments, &said);
        // Every document and every variable: the text the policy rewrote went out as the text
        // the check-only store was handed directly.
        assert_eq!(sent, expected, "{kind}");
        holds_replaced(&sanitizes, kind);
    }
}

#[test]
fn a_narrow_write_leaves_a_stored_comment_as_it_stands_and_a_sweep_rewrites_it() {
    let stored = format!("Asked {TERM} first.");
    let board = Board::under(Policy::Sanitizes, &stored);
    let checks = Board::under(Policy::Checks, &stored);
    let priority = |board: &Board| {
        let (_, arguments, said) = each_write(board, WITH).swap_remove(0);
        board.ran(&arguments, &said)
    };
    let spent = priority(&board);
    assert_eq!(spent, priority(&checks));
    assert_eq!(board.shown()["priority"], json!("high"));
    assert_eq!(board.comments(), std::slice::from_ref(&stored));

    let swept = board.ran(
        &["sources", "sanitize", "board", "--complete", "--json"].map(str::to_owned),
        "",
    );
    assert!(!swept.is_empty());
    assert_eq!(board.comments(), [format!("Asked {WITH} first.")]);
}

#[test]
fn opted_in_a_narrow_write_rewrites_the_stored_comment_first_at_the_cost_of_its_reads() {
    let stored = format!("Asked {TERM} first.");
    let opted = Board::under(Policy::Guards, &stored);
    let (_, arguments, said) = each_write(&opted, WITH).swap_remove(0);
    let sent = opted.ran(&arguments, &said);
    assert_eq!(opted.comments(), [format!("Asked {WITH} first.")]);
    assert_eq!(opted.shown()["priority"], json!("high"));

    let default = Board::under(Policy::Sanitizes, &stored);
    let (_, arguments, said) = each_write(&default, WITH).swap_remove(0);
    let spent = default.ran(&arguments, &said);
    assert!(
        sent.len() > spent.len(),
        "the guard reads what the default does not: {} against {}",
        sent.len(),
        spent.len()
    );
}

/// A linking caller's own policy, rewriting through the released onevcs.
struct Linked {
    home: std::path::PathBuf,
}

impl Linked {
    fn answered(&self, words: &[&str], input: &Value) -> (Option<i32>, Vec<u8>) {
        let mut child = std::process::Command::new(onevcs())
            .args(words)
            .env("ONEVCS_HOME", &self.home)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("onevcs runs");
        std::io::Write::write_all(
            &mut child.stdin.take().expect("stdin"),
            input.to_string().as_bytes(),
        )
        .expect("the input is written");
        let output = child.wait_with_output().expect("onevcs answers");
        (output.status.code(), output.stdout)
    }
}

#[async_trait::async_trait]
impl onetaskgraph_core::boundary::WritePolicy for Linked {
    async fn repository_visibility(
        &self,
        repository: &onetaskgraph_plugin_api::Repository,
    ) -> Result<
        onetaskgraph_core::boundary::RepositoryVisibility,
        onetaskgraph_core::boundary::PolicyError,
    > {
        let (code, answer) = self.answered(
            &["boundary", "inspect", "--input", "-"],
            &json!({"repository": repository.as_str()}),
        );
        let answer: Value = serde_json::from_slice(&answer).unwrap_or(Value::Null);
        match (code, answer["visibility"].as_str()) {
            (Some(0), Some("public")) => {
                Ok(onetaskgraph_core::boundary::RepositoryVisibility::Public)
            }
            (Some(0), Some("private")) => {
                Ok(onetaskgraph_core::boundary::RepositoryVisibility::Private)
            }
            _ => Err(onetaskgraph_core::boundary::PolicyError::new(format!(
                "onevcs answered {code:?} with {answer}"
            ))),
        }
    }

    async fn check_public_write(
        &self,
        _input: &onetaskgraph_core::boundary::PublicWriteInput,
    ) -> Result<onetaskgraph_core::boundary::WriteVerdict, onetaskgraph_core::boundary::PolicyError>
    {
        panic!("a rewriting policy is asked to sanitise, never to check");
    }

    async fn sanitize_public_write(
        &self,
        input: &onetaskgraph_core::boundary::PublicSanitizeInput,
    ) -> Result<
        onetaskgraph_core::boundary::SanitizeVerdict,
        onetaskgraph_core::boundary::PolicyError,
    > {
        let (code, answer) = self.answered(
            &["boundary", "sanitize", "--input", "-"],
            &serde_json::to_value(input).expect("an input serializes"),
        );
        Ok(onetaskgraph_core::boundary::sanitize_verdict(
            input, code, &answer,
        ))
    }
}

/// The engine `board`'s configuration document, with `write_policy` set to `policy`, builds.
fn linked_engine(board: &Board, policy: Option<Value>) -> onetaskgraph_core::Engine {
    let mut configured = board.configured.clone();
    if let Some(policy) = policy {
        configured["write_policy"] = policy;
    }
    board.sandbox.project_document(&configured.to_string());
    let environment = onetaskgraph_core::Environment::from_pairs([(
        "XDG_CONFIG_HOME",
        board.sandbox.config_home().to_string_lossy().into_owned(),
    )]);
    let loaded = onetaskgraph_core::config::load(
        &board.sandbox.project(),
        &environment,
        &onetaskgraph_core::config::Layer::default(),
    )
    .expect("the configuration loads");
    onetaskgraph_core::Engine::build(&loaded.config, &loaded.secrets)
}

#[test]
fn a_linking_caller_s_rewriting_policy_costs_a_narrow_write_what_no_rewriting_policy_costs() {
    let runtime = tokio::runtime::Runtime::new().expect("a runtime");
    let id: onetaskgraph_core::GlobalId = TASK.parse().expect("a qualified id");

    let checks = Board::clean(Policy::Checks);
    let policy = checks.home.write_policy_naming_home(&checks.sandbox);
    let engine = linked_engine(&checks, Some(policy));
    let said = format!("Retry the way {WITH} does.");
    runtime
        .block_on(engine.set_task_content(&id, &said))
        .expect("the check passes");
    let expected = checks.board.served();
    assert!(!expected.is_empty());

    let linked = Board::clean(Policy::Checks);
    let engine = linked_engine(&linked, None)
        .with_write_policy(std::sync::Arc::new(Linked {
            home: linked.home.path().to_path_buf(),
        }))
        .with_replacements(vec![onetaskgraph_core::boundary::Replacement::Replace {
            term: TERM.to_owned(),
            with: WITH.to_owned(),
        }]);
    let carrying = format!("Retry the way {TERM} does.");
    let written = runtime
        .block_on(engine.set_task_content(&id, &carrying))
        .expect("written replaced");
    assert_eq!(written.content, said);
    assert_eq!(linked.board.served(), expected);
    assert_eq!(linked.shown()["content"], json!(said));
    assert_eq!(engine.take_sanitization().replaced, 1);

    // The same caller, opting in, is the guard: it reads the comments the default does not.
    let opted = Board::clean(Policy::Checks);
    let engine = linked_engine(&opted, None)
        .with_write_policy(std::sync::Arc::new(Linked {
            home: opted.home.path().to_path_buf(),
        }))
        .with_sanitize_stored_on_write(true);
    runtime
        .block_on(engine.set_task_content(&id, &said))
        .expect("written");
    assert!(opted.board.served().len() > expected.len());
}
