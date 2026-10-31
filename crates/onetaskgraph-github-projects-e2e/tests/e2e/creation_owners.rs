//! A GitHub Projects source's `creation_owners`, driven through the binary against the loopback
//! board: which owners' repositories a create may file its issue in.
//!
//! The realistic caller is a board shared by several owners' work, synced from a folder of
//! Markdown, where an issue must only ever be created in one organisation's repositories. Every
//! refusal is asserted on the board's own request log — nothing written, no intent either — and
//! on the folder, which records no link to a copy that never happened.

use std::path::PathBuf;
use std::process::Output;

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{GitHubBoardFields, github_projects_with_board};

/// The board's configured repository, owned by a login `creation_owners` names in another case.
const CONFIGURED: &str = "Example-Owner/onetaskgraph";

/// A repository of an owner `creation_owners` does not name.
const OTHER: &str = "other-owner/tooling";

struct Setup {
    sandbox: Sandbox,
    board: GitHubBoardFields,
    plan: PathBuf,
    sources: Value,
}

impl Setup {
    /// A folder `plan` and the fixture board `board`, its source restricted to `owners`, with
    /// `extra` added to the configuration's sources.
    fn new(owners: Option<Value>, extra: impl FnOnce(&Sandbox, &mut Value)) -> Self {
        let sandbox = Sandbox::new();
        let (mut config, board) = github_projects_with_board(&sandbox);
        config["repository"] = json!(CONFIGURED);
        board.use_repository(CONFIGURED);
        if let Some(owners) = owners {
            config["creation_owners"] = owners;
        }
        let plan = sandbox.subdirectory("plan");
        std::fs::create_dir_all(plan.join("tasks")).expect("the folder");
        let mut sources = json!({
            "plan": {"plugin": "local-md", "config": {"root": plan}},
            "board": {"plugin": "github-projects", "config": config},
        });
        extra(&sandbox, &mut sources);
        sandbox.secrets_file(
            "GITHUB_PROJECTS_FIXTURE_TOKEN=test-token\nGITHUB_PROJECTS_FIXTURE_TOKEN_B=test-token-b\n",
        );
        sandbox.project_document(&json!({"sources": sources}).to_string());
        Self {
            sandbox,
            board,
            plan,
            sources,
        }
    }

    /// Rewrite the configuration with `change` applied to its sources.
    fn reconfigure(&mut self, change: impl FnOnce(&mut Value)) {
        change(&mut self.sources);
        self.sandbox
            .project_document(&json!({"sources": self.sources}).to_string());
    }

    fn record(&self, kind: &str, id: &str, front: &str) {
        let path = self.plan.join(kind).join(format!("{id}.md"));
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("the folder");
        std::fs::write(&path, format!("---\n{front}\n---\nBody of {id}.\n")).expect("a record");
    }

    fn file(&self, kind: &str, id: &str) -> String {
        std::fs::read_to_string(self.plan.join(kind).join(format!("{id}.md"))).expect("a record")
    }

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

    fn exited(&self, arguments: &[&str], code: i32) -> (Value, Vec<(String, Value)>) {
        let (output, served) = self.run(arguments);
        assert_eq!(
            output.status.code(),
            Some(code),
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

/// A second loopback board of its own, whose issues live in `widgetco/app`, so a route can
/// place an item naming that repository there.
fn second_board(sandbox: &Sandbox) -> Value {
    let (mut config, board) = github_projects_with_board(sandbox);
    config["repository"] = json!("widgetco/app");
    board.use_repository("widgetco/app");
    config
}

/// Whether any request was a mutation.
fn wrote(served: &[(String, Value)]) -> bool {
    served
        .iter()
        .any(|(document, _)| document.trim_start().starts_with("mutation"))
}

/// Assert `answer` is the `creation-owner` refusal naming `repository` and the allowed owner.
fn refused_for_owner(answer: &Value, repository: &str) {
    let failure = &answer["failure"];
    assert_eq!(failure["kind"], "creation-owner", "{answer:#}");
    assert_eq!(failure["source"], "board", "{answer:#}");
    let message = failure["message"].as_str().expect("a message");
    assert!(
        message.contains(repository) && message.contains("example-owner"),
        "names the repository and the allowed owners: {message}"
    );
    assert!(message.contains("next:"), "{message}");
}

#[test]
fn a_create_into_another_owners_repository_is_refused_by_every_road_writing_nothing() {
    let setup = Setup::new(Some(json!(["example-owner"])), |_, _| {});
    setup.record(
        "tasks",
        "lone",
        &format!("title: Lone\nstatus: todo\nrepositories: [github.com/{OTHER}]"),
    );
    let before = setup.file("tasks", "lone");
    for extra in [vec![], vec!["--create"], vec!["--dry-run"]] {
        let mut arguments = vec!["--json", "task", "copy", "plan:lone", "--to", "board"];
        arguments.extend(extra.iter().copied());
        let (answer, served) = setup.exited(&arguments, 1);
        refused_for_owner(&answer, OTHER);
        assert!(!wrote(&served), "{extra:?}: {served:#?}");
        assert_eq!(
            setup.file("tasks", "lone"),
            before,
            "{extra:?}: no link, no intent"
        );
    }
    // A member create of a project copy: the project names an allowed owner's repository,
    // spelled in another case, and its member names another owner's.
    setup.record("projects", "goal", "title: Goal\nstatus: todo");
    setup.record(
        "tasks",
        "member",
        "title: Member\nstatus: todo\nproject: goal\nrepositories: [github.com/other-owner/tooling]",
    );
    for extra in [vec![], vec!["--dry-run"]] {
        let mut arguments = vec!["--json", "project", "copy", "plan:goal", "--to", "board"];
        arguments.extend(extra.iter().copied());
        let (answer, served) = setup.exited(&arguments, 1);
        refused_for_owner(&answer, OTHER);
        assert!(!wrote(&served), "{extra:?}: {served:#?}");
    }
    assert!(!setup.file("projects", "goal").contains("onetaskgraph"));
}

#[test]
fn a_create_in_an_allowed_owners_repository_lands_whatever_the_case_of_its_login() {
    let setup = Setup::new(Some(json!(["example-owner"])), |_, _| {});
    // Naming no repository, it lands in the configured one, owned by `Example-Owner`.
    setup.record("tasks", "allowed", "title: Allowed\nstatus: todo");
    let (copied, served) = setup.exited(
        &["--json", "task", "copy", "plan:allowed", "--to", "board"],
        0,
    );
    assert_eq!(copied["items"][0]["action"], "created", "{copied:#}");
    assert!(
        served
            .iter()
            .any(|(document, _)| document.contains("createIssue(input:$input)")),
        "{served:#?}"
    );
    let (located, _) = setup.exited(
        &["--json", "task", "locate", "plan:allowed", "--to", "board"],
        0,
    );
    assert!(
        located["items"][0].get("creates_in").is_none()
            || located["items"][0]["creates_in"].get("refused").is_none(),
        "{located:#}"
    );
}

#[test]
fn an_update_of_an_issue_in_another_owners_repository_is_never_refused() {
    // Created before the restriction, in the configured repository.
    let mut open = Setup::new(None, |_, _| {});
    open.record("tasks", "lone", "title: Lone\nstatus: todo");
    open.exited(&["--json", "task", "copy", "plan:lone", "--to", "board"], 0);
    // The same board, now restricted: the counterpart is updated, not refused.
    open.reconfigure(|sources| {
        sources["board"]["config"]["creation_owners"] = json!(["somebody-else"]);
    });
    let held = open
        .file("tasks", "lone")
        .replace("title: Lone", "title: Lone, revised");
    std::fs::write(open.plan.join("tasks/lone.md"), held).expect("an edit");
    let (copied, _) = open.exited(&["--json", "task", "copy", "plan:lone", "--to", "board"], 0);
    assert_eq!(copied["items"][0]["action"], "updated", "{copied:#}");
}

#[test]
fn locate_reports_a_refused_create_without_failing_and_an_allowed_one_without_the_member() {
    let setup = Setup::new(Some(json!(["example-owner"])), |_, _| {});
    setup.record(
        "tasks",
        "lone",
        &format!("title: Lone\nstatus: todo\nrepositories: [github.com/{OTHER}]"),
    );
    setup.record("tasks", "allowed", "title: Allowed\nstatus: todo");
    let (located, served) = setup.exited(
        &[
            "--json",
            "task",
            "locate",
            "plan:lone",
            "plan:allowed",
            "--to",
            "board",
        ],
        0,
    );
    assert!(!wrote(&served));
    let refused = &located["items"][0]["creates_in"];
    assert_eq!(
        refused["repository"],
        format!("github.com/{OTHER}"),
        "{located:#}"
    );
    let reason = refused["refused"].as_str().expect("a refusal");
    assert!(
        reason.contains(OTHER) && reason.contains("example-owner"),
        "{reason}"
    );
    assert_eq!(
        located["items"][1]["creates_in"],
        json!({"repository": format!("github.com/{CONFIGURED}")}),
        "{located:#}"
    );
}

#[test]
fn a_routed_copy_is_judged_by_the_setting_of_the_destination_it_was_placed_at() {
    // Routed from the restricted board to a second board without the setting: created there.
    let setup = Setup::new(Some(json!(["example-owner"])), |sandbox, sources| {
        sources["second"] = json!({"plugin": "github-projects", "config": second_board(sandbox)});
        sources["board"]["routes"] =
            json!([{"repositories": ["github.com/widgetco/*"], "to": "second"}]);
    });
    setup.record(
        "tasks",
        "widget",
        "title: Widget\nstatus: todo\nrepositories: [github.com/widgetco/app]",
    );
    let (copied, _) = setup.exited(
        &["--json", "task", "copy", "plan:widget", "--to", "board"],
        0,
    );
    assert_eq!(copied["items"][0]["action"], "created", "{copied:#}");
    assert_eq!(copied["items"][0]["placed"]["destination"], "second");

    // Routed from an unrestricted board to a second board carrying the setting: refused.
    let setup = Setup::new(None, |sandbox, sources| {
        let mut second = second_board(sandbox);
        second["creation_owners"] = json!(["example-owner"]);
        sources["second"] = json!({"plugin": "github-projects", "config": second});
        sources["board"]["routes"] =
            json!([{"repositories": ["github.com/widgetco/*"], "to": "second"}]);
    });
    setup.record(
        "tasks",
        "widget",
        "title: Widget\nstatus: todo\nrepositories: [github.com/widgetco/app]",
    );
    let (answer, served) = setup.exited(
        &["--json", "task", "copy", "plan:widget", "--to", "board"],
        1,
    );
    assert_eq!(answer["failure"]["kind"], "creation-owner", "{answer:#}");
    assert_eq!(answer["failure"]["source"], "second", "{answer:#}");
    assert!(!wrote(&served), "{served:#?}");
}

#[test]
fn a_configuration_naming_a_login_outside_githubs_grammar_is_refused_naming_it() {
    let setup = Setup::new(Some(json!(["example-owner", "not a login"])), |_, _| {});
    let (output, _) = setup.run(&["task", "list", "--source", "board"]);
    assert_ne!(output.status.code(), Some(0));
    assert!(
        stderr(&output).contains("\"not a login\"") && stderr(&output).contains("creation_owners"),
        "{}",
        stderr(&output)
    );
    // Without the setting the configuration loads and prints no such key.
    let open = Setup::new(None, |_, _| {});
    let (output, _) = open.run(&["--json", "config", "show"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(
        !stdout(&output).contains("creation_owners"),
        "{}",
        stdout(&output)
    );
}
