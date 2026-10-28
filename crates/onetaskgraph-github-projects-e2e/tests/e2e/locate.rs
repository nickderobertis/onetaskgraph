//! `task locate`'s `creates_in` against the loopback board: the repository it names is the one
//! the `task copy` after it creates the issue in.
//!
//! The board files every issue under one owner, so each repository here is one of that owner's.
//! Which repository a create chose is read off the request it resolved the repository with —
//! the `repository(owner, name)` lookup that comes before `createIssue`.

use std::process::Output;

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{GitHubBoardFields, github_projects_with_items};

/// The owner the fixture board files issues under.
const OWNER: &str = "nickderobertis";

/// A folder of Markdown `plan` and the fixture board `board`, holding the project issue `GOAL-1`
/// in the repository `planning`, copied from `plan:goal`.
struct Setup {
    sandbox: Sandbox,
    board: GitHubBoardFields,
}

impl Setup {
    fn new() -> Self {
        let sandbox = Sandbox::new();
        let (config, board) = github_projects_with_items(
            &sandbox,
            vec![json!({
                "item": "ITEM-GOAL-1", "id": "GOAL-1", "type": "Issue", "title": "Goal",
                "body": "The goal.\n\n<!-- onetaskgraph.metadata\n{\"onetaskgraph.item_kind\":\"project\",\"onetaskgraph.origin\":\"plan:goal\"}\n-->",
                "state": "OPEN", "reason": null, "parent": null,
                "repo": format!("{OWNER}/planning"), "status": "Todo",
                "origin": "plan:goal", "labels": [],
            })],
        );
        let root = sandbox.subdirectory("plan");
        sandbox.project_document(
            &json!({"sources": {
                "plan": {"plugin": "local-md", "config": {"root": root}},
                "board": {"plugin": "github-projects", "config": config},
            }})
            .to_string(),
        );
        let setup = Self { sandbox, board };
        setup.record(
            "projects",
            "goal",
            "title: Goal\nstatus: todo\nmetadata: {onetaskgraph.copies: {board: \"board:GOAL-1\"}}",
        );
        setup
    }

    fn record(&self, kind: &str, id: &str, front: &str) {
        let path = self
            .sandbox
            .project()
            .join("plan")
            .join(kind)
            .join(format!("{id}.md"));
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("the folder");
        std::fs::write(&path, format!("---\n{front}\n---\nBody of {id}.\n")).expect("a record");
    }

    /// One command's output and every request the board was sent for it.
    fn run(&self, arguments: &[&str]) -> (Output, Vec<(String, Value)>) {
        let before = self.board.served().len();
        let output = self
            .sandbox
            .command()
            .args(arguments)
            .assert()
            .get_output()
            .clone();
        (output, self.board.served()[before..].to_vec())
    }

    fn ok(&self, arguments: &[&str]) -> (Value, Vec<(String, Value)>) {
        let (output, served) = self.run(arguments);
        assert_eq!(
            output.status.code(),
            Some(0),
            "`onetaskgraph {}`\n{}{}",
            arguments.join(" "),
            stdout(&output),
            stderr(&output)
        );
        (
            serde_json::from_str(&stdout(&output)).expect("a JSON answer"),
            served,
        )
    }
}

/// The repository each issue a command created was created in, as `github.com/owner/name`: the
/// last one it resolved before that `createIssue` — by the lookup of the repository alone, or
/// by a read carrying it beside the board's fields.
fn created_in(served: &[(String, Value)]) -> Vec<String> {
    let mut resolved = None;
    let mut created = Vec::new();
    for (document, variables) in served {
        let named = if document.contains("repository(owner:$owner,name:$name)") {
            Some(("owner", "name"))
        } else if variables.get("repositoryName").is_some() {
            Some(("repositoryOwner", "repositoryName"))
        } else {
            None
        };
        if let Some((owner, name)) = named {
            resolved = Some(format!(
                "github.com/{}/{}",
                variables[owner].as_str().unwrap_or_default(),
                variables[name].as_str().unwrap_or_default()
            ));
        }
        if document.contains("createIssue(input:$input)") {
            created.push(
                resolved
                    .clone()
                    .expect("a create resolves its repository first"),
            );
        }
    }
    created
}

#[test]
fn locate_names_the_repository_the_copy_after_it_creates_each_issue_in() {
    for no_counterpart in [false, true] {
        let setup = Setup::new();
        setup.record(
            "tasks",
            "one",
            &format!("title: One\nstatus: todo\nrepositories: [github.com/{OWNER}/tooling]"),
        );
        setup.record(
            "tasks",
            "several",
            &format!(
                "title: Several\nstatus: todo\n\
                 repositories: [github.com/{OWNER}/tooling, github.com/{OWNER}/docs]"
            ),
        );
        setup.record(
            "tasks",
            "filed",
            "title: Filed\nstatus: todo\nproject: goal",
        );
        for (id, repository) in [
            ("plan:one", format!("github.com/{OWNER}/tooling")),
            // Several, and no project: the source's configured repository.
            ("plan:several", format!("github.com/{OWNER}/onetaskgraph")),
            // Filed under `GOAL-1`, whose issue sits in another repository than the configured one.
            ("plan:filed", format!("github.com/{OWNER}/planning")),
        ] {
            let mut arguments = vec!["task", "locate", id, "--to", "board", "--json"];
            if no_counterpart {
                arguments.push("--no-counterpart");
            }
            let (located, served) = setup.ok(&arguments);
            let item = &located["items"][0];
            assert_eq!(
                item["creates_in"],
                json!({"repository": repository}),
                "{id}: {located:#}"
            );
            assert!(
                !served
                    .iter()
                    .any(|(document, _)| document.trim_start().starts_with("mutation")),
                "{id}: locate wrote nothing: {served:#?}"
            );
            if no_counterpart {
                assert!(item.get("counterpart").is_none(), "{id}: {item:#}");
                // The parent project issue's read is the one request a filed task sends, and an
                // unfiled one sends none.
                let read: Vec<&Value> = served.iter().map(|(_, variables)| variables).collect();
                if id == "plan:filed" {
                    assert!(!served.is_empty(), "{id}: the project issue was read");
                    for variables in &read {
                        assert!(
                            variables.to_string().contains("GOAL-1"),
                            "{id}: only the project issue was read: {served:#?}"
                        );
                    }
                } else {
                    assert!(served.is_empty(), "{id}: {served:#?}");
                }
            } else {
                assert_eq!(item["counterpart"], Value::Null, "{id}: {item:#}");
            }
            let (copied, served) = setup.ok(&["task", "copy", id, "--to", "board", "--json"]);
            assert_eq!(copied["items"][0]["action"], "created", "{id}: {copied:#}");
            assert_eq!(
                created_in(&served),
                std::slice::from_ref(&repository),
                "{id}: the copy created the issue where locate said"
            );
        }
    }
}
