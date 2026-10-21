//! `project copy --member <ID>... --members-only` into a GitHub board, against the loopback
//! fixture board: the named tasks land under the project the board already holds, and that
//! project is neither read for comparison nor written — counted on every request the board was
//! sent, and read back through the binary afterwards.

use std::path::PathBuf;

use onetaskgraph_github_projects::graphql;
use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{GitHubBoardFields, document, github_projects_with_board};

/// A folder of Markdown holding project `P`, beside the fixture board it is copied into.
struct Plan {
    sandbox: Sandbox,
    board: GitHubBoardFields,
    root: PathBuf,
}

impl Plan {
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

    fn write(&self, relative: &str, text: &str) {
        std::fs::write(self.root.join(relative), text).expect("a Markdown file");
    }

    /// One command, which had to exit `code`: what it printed, and every request the board
    /// served for it.
    fn run(&self, arguments: &[&str], code: i32) -> (String, String, Vec<(String, Value)>) {
        let before = self.board.served().len();
        let output = self
            .sandbox
            .command()
            .args(arguments)
            .assert()
            .get_output()
            .clone();
        assert_eq!(
            output.status.code(),
            Some(code),
            "`onetaskgraph {}`\n{}{}",
            arguments.join(" "),
            stdout(&output),
            stderr(&output)
        );
        let served = self.board.served()[before..].to_vec();
        (stdout(&output), stderr(&output), served)
    }

    fn json(&self, arguments: &[&str]) -> Value {
        let (out, _, _) = self.run(arguments, 0);
        serde_json::from_str(&out).expect("a command emits JSON")
    }
}

/// The source-to-destination pairs of a copy report.
fn landed(report: &Value) -> Vec<(String, String)> {
    report["items"]
        .as_array()
        .expect("a copy report carries items")
        .iter()
        .map(|item| {
            (
                item["source"].as_str().expect("a source id").to_owned(),
                item["destination"]
                    .as_str()
                    .expect("a destination id")
                    .to_owned(),
            )
        })
        .collect()
}

/// The mutations that rewrite an existing item: its issue, its draft, its board fields or its
/// state.
fn rewrites(document: &str) -> bool {
    [
        graphql::UPDATE_ISSUE,
        graphql::UPDATE_DRAFT,
        graphql::UPDATE_FIELD,
        graphql::UPDATE_FIELDS,
        graphql::WRITE_FIELD_VALUES,
        graphql::CLEAR_FIELD,
        graphql::CLOSE_ISSUES,
        graphql::DELETE_ISSUE,
    ]
    .contains(&document)
}

/// The reads of issues by their node ids.
fn reads_by_node_id(document: &str) -> bool {
    [
        graphql::ISSUE,
        graphql::ISSUE_COPY_DETAILS,
        graphql::ISSUE_DETAIL,
        graphql::ISSUE_BOARD_ITEMS,
        graphql::ISSUE_DEPENDENCIES,
    ]
    .contains(&document)
}

#[test]
fn a_members_only_copy_lands_its_members_and_neither_reads_nor_writes_the_boards_project() {
    let plan = Plan::new();
    plan.write(
        "projects/P.md",
        "---\ntitle: The plan\nstatus: Doing\n---\nwhat the board was given\n",
    );
    for (id, title) in [("T-1", "First"), ("T-2", "Second")] {
        plan.write(
            &format!("tasks/{id}.md"),
            &format!("---\ntitle: {title}\nstatus: Todo\nproject: P\n---\n{title} step\n"),
        );
    }
    let first = plan.json(&["project", "copy", "plans:P", "--to", "board", "--json"]);
    let ids = landed(&first);
    let id_of = |source: &str| {
        ids.iter()
            .find(|(from, _)| from == source)
            .map(|(_, to)| to.clone())
            .unwrap_or_else(|| panic!("{source} landed: {first:#}"))
    };
    let project = id_of("plans:P");
    let native = project
        .strip_prefix("board:")
        .expect("a board id")
        .to_owned();

    // The write-back's shadow: every item records where it landed, and the shadow project's
    // title, body and labels are deliberately not the board's — a copy that compared or wrote
    // the project would carry them across.
    plan.write(
        "projects/P.md",
        &format!(
            "---\ntitle: The shadow's own title\nstatus: Todo\nlabels: [shadow]\n\
             metadata: {{onetaskgraph.origin: \"{project}\"}}\n---\nthe shadow's own body\n"
        ),
    );
    plan.write(
        "tasks/T-1.md",
        &format!(
            "---\ntitle: First, moving\nstatus: Doing\nproject: P\n\
             metadata: {{onetaskgraph.origin: \"{}\"}}\n---\nFirst step\n",
            id_of("plans:T-1")
        ),
    );
    plan.write(
        "tasks/T-3.md",
        "---\ntitle: Third\nstatus: Todo\nproject: P\n---\nadded mid-run\n",
    );
    let shown = |plan: &Plan| plan.json(&["project", "show", &project, "--json"])["items"].clone();
    let before = shown(&plan);
    let members_only = |members: &[&str]| {
        let mut arguments = vec!["project", "copy", "plans:P", "--to", "board"];
        for member in members {
            arguments.extend(["--member", member]);
        }
        arguments.extend(["--members-only", "--json"]);
        let (out, _, served) = plan.run(&arguments, 0);
        let report: Value = serde_json::from_str(&out).expect("a copy emits JSON");
        let naming: Vec<(String, Value)> = served
            .iter()
            .filter(|(_, variables)| names(variables, &native))
            .cloned()
            .collect();
        (report, served, naming)
    };

    // A change to a member the board holds: nothing the board is sent names its project.
    let (report, served, naming) = members_only(&["plans:T-1"]);
    assert_eq!(
        actions(&report),
        vec![("plans:T-1".to_owned(), "updated".to_owned())],
        "the named member, and not the project: {report:#}"
    );
    assert_eq!(naming, Vec::new(), "the project was read or written");
    assert!(
        served
            .iter()
            .any(|(document, _)| document == graphql::UPDATE_ISSUE),
        "the named member's own change was written"
    );
    let moved = plan.json(&["task", "show", &id_of("plans:T-1"), "--json"]);
    assert_eq!(
        moved["items"][0]["item"]["status"]["category"],
        json!("in-progress"),
        "{moved:#}"
    );
    assert_eq!(moved["items"][0]["item"]["title"], json!("First, moving"));

    // A member the board does not hold yet: it is created under the project, and the one read
    // naming the project is the plugin's own read of a parent issue before `createIssue` —
    // where the sub-issue is placed, and that the parent is not a draft — which every task
    // created under a project costs. Nothing compares the project or rewrites it.
    let (report, served, naming) = members_only(&["plans:T-3"]);
    assert_eq!(
        actions(&report),
        vec![("plans:T-3".to_owned(), "created".to_owned())],
        "{report:#}"
    );
    let reads: Vec<&(String, Value)> = naming
        .iter()
        .filter(|(document, _)| reads_by_node_id(document))
        .collect();
    assert_eq!(reads.len(), 1, "one creation-context read: {naming:#?}");
    let at = served
        .iter()
        .position(|request| request == reads[0])
        .expect("the read was served");
    assert_eq!(
        (reads[0].0.as_str(), served[at + 1].0.as_str()),
        (graphql::ISSUE, graphql::CREATE_ISSUE),
        "the read naming the project is the one before the member's createIssue"
    );
    assert!(
        naming.iter().all(|(document, _)| !rewrites(document)),
        "the project was written: {naming:#?}"
    );
    let created = landed(&report)
        .into_iter()
        .map(|(_, to)| to)
        .next()
        .expect("T-3 landed");
    let task = plan.json(&["task", "show", &created, "--json"]);
    assert_eq!(
        task["items"][0]["item"]["project"],
        json!(native),
        "{task:#}"
    );

    assert_eq!(
        shown(&plan),
        before,
        "the board's project — title, body, labels and fields — is as it was"
    );
}

/// Whether any string in `variables` names `native`: the issue itself, or a board item whose
/// id is spelled from it.
fn names(variables: &Value, native: &str) -> bool {
    match variables {
        Value::String(text) => text == native || text.ends_with(&format!("-{native}")),
        Value::Array(values) => values.iter().any(|value| names(value, native)),
        Value::Object(fields) => fields.values().any(|value| names(value, native)),
        _ => false,
    }
}

/// Each item of a copy report as its source id and action.
fn actions(report: &Value) -> Vec<(String, String)> {
    report["items"]
        .as_array()
        .expect("a copy report carries items")
        .iter()
        .map(|item| {
            (
                item["source"].as_str().expect("a source").to_owned(),
                item["action"].as_str().expect("an action").to_owned(),
            )
        })
        .collect()
}

#[test]
fn a_members_only_copy_of_a_project_the_board_lacks_writes_nothing() {
    let plan = Plan::new();
    plan.write(
        "projects/Q.md",
        "---\ntitle: Never copied\nstatus: Todo\n---\nnot on the board\n",
    );
    plan.write(
        "tasks/T-9.md",
        "---\ntitle: Orphaned step\nstatus: Todo\nproject: Q\n---\nnowhere to go\n",
    );
    let held = |plan: &Plan| {
        (
            plan.json(&["task", "list", "--source", "board", "--json"])["items"].clone(),
            plan.json(&["project", "list", "--source", "board", "--json"])["items"].clone(),
        )
    };
    let before = held(&plan);

    let (_, said, served) = plan.run(
        &[
            "project",
            "copy",
            "plans:Q",
            "--to",
            "board",
            "--member",
            "plans:T-9",
            "--members-only",
        ],
        1,
    );
    assert!(
        said.contains("plans:Q has no counterpart in board")
            && said.contains("`onetaskgraph project copy plans:Q --to board`")
            && said.contains("next:"),
        "{said}"
    );
    assert!(
        served
            .iter()
            .all(|(document, _)| !document.trim_start().starts_with("mutation")),
        "a refused members-only copy sent a mutation: {served:#?}"
    );
    assert_eq!(held(&plan), before, "the board holds no new item");
}
