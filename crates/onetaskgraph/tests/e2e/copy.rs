//! The copy verb, driven the way a user drives it.
//!
//! The round trip is the point of the whole verb, so it is proven rather than asserted:
//! a task is copied out of a destination into a folder of Markdown, the Markdown is
//! edited the way a person edits it, and the copy back updates the item it came from
//! instead of creating a second one — with every field the edit did not touch
//! byte-for-byte what it was.
//!
//! Every test here spawns the compiled binary as a subprocess and asserts on its exit
//! code, stdout and stderr. The same verb driven as a *library* call, which is the
//! consumer a command-line-only copy would strand, is proven in
//! `crates/onetaskgraph-core/tests/copy.rs`.

use std::process::Output;

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{
    LINEAR_REFUSED_WRITE, ROWS, SOURCE, document, empty_folder,
    github_projects_failing_a_field_write_and_its_cleanup,
    github_projects_failing_a_field_write_once, github_projects_failing_to_file_and_its_cleanup,
    github_projects_failing_to_file_once, github_projects_reading_one_item_behind,
    github_projects_with_board, linear_block, linear_failing_a_relation_write_once, qualified,
};

/// The folder every copy journey copies into, configured beside the source under test.
const NOTES: &str = "notes";

fn run(sandbox: &Sandbox, arguments: &[&str]) -> Output {
    sandbox
        .command()
        .args(arguments)
        .assert()
        .get_output()
        .clone()
}

/// Standard output of a run that had to succeed, quoting stderr when it did not.
fn ok(sandbox: &Sandbox, arguments: &[&str]) -> String {
    let output = run(sandbox, arguments);
    assert_eq!(
        output.status.code(),
        Some(0),
        "`onetaskgraph {}` exited {:?}\n{}",
        arguments.join(" "),
        output.status.code(),
        stderr(&output)
    );
    stdout(&output)
}

/// Standard error of a run that had to fail with `code`.
fn refused(sandbox: &Sandbox, arguments: &[&str], code: i32) -> String {
    let output = run(sandbox, arguments);
    assert_eq!(
        output.status.code(),
        Some(code),
        "`onetaskgraph {}` was expected to exit {code}\n{}{}",
        arguments.join(" "),
        stdout(&output),
        stderr(&output)
    );
    stderr(&output)
}

/// One item of a `--json` copy report, as a comparable triple.
fn reported(rendered: &str) -> Vec<(String, Value, String)> {
    let report: Value = serde_json::from_str(rendered).expect("a copy emits JSON");
    report["items"]
        .as_array()
        .expect("a copy report carries items")
        .iter()
        .map(|item| {
            (
                item["source"].as_str().expect("a source id").to_owned(),
                item["destination"].clone(),
                item["action"].as_str().expect("an action").to_owned(),
            )
        })
        .collect()
}

/// One item of a `<verb> show --json` response.
fn shown(sandbox: &Sandbox, verb: &str, id: &str) -> Value {
    let rendered = ok(sandbox, &[verb, "show", id, "--json"]);
    let response: Value = serde_json::from_str(&rendered).expect("show emits JSON");
    response["items"][0]["item"].clone()
}

/// Two Markdown folders, the first holding one task and the second empty, and the root
/// of the first.
///
/// This is the user's own flow in miniature: a folder standing in for the system their
/// team works out of, and a folder they author and edit in.
fn folders(sandbox: &Sandbox) -> std::path::PathBuf {
    let root = sandbox.subdirectory("remote");
    let tasks = root.join("tasks");
    std::fs::create_dir_all(&tasks).expect("the remote task folder");
    std::fs::write(
        tasks.join("ENG-1.md"),
        "---\ntitle: Rate-limit the sync loop\nstatus: doing\n\
         labels: [{id: L-1, name: bug}]\n\
         metadata: {caller.count: 3, caller.shape: {nested: [1, true, null]}}\n\
         repositories: [github.com/nickderobertis/onetaskgraph]\n---\nthe body\n",
    )
    .expect("the remote task");
    sandbox.project_document(&document(&json!({
        "remote": {"plugin": "local-md", "config": {
            "root": root,
            "status_mapping": {"todo": "todo", "doing": "in-progress", "shipped": "done"},
        }},
        NOTES: {"plugin": "local-md", "config": empty_folder(sandbox, NOTES)},
    })));
    root
}

#[test]
fn linear_is_a_permanent_task_destination_with_typed_metadata_and_repository_origins() {
    let sandbox = Sandbox::new();
    let root = sandbox.subdirectory("linear-task-source");
    std::fs::create_dir_all(root.join("tasks")).unwrap();
    std::fs::write(root.join("tasks/A.md"), "---\ntitle: Authored locally\nstatus: Todo\nlabels: [{id: local-bug, name: bug}]\nmetadata: {object: {a: 1}, array: [1, true], string: text, number: 3.5, boolean: true, null: null}\nrepositories: [github.com/acme/work]\n---\nvisible body\n").unwrap();
    sandbox.project_document(&document(&json!({
        "authored": {"plugin":"local-md","config":{"root":root,"status_mapping":{"Todo":"todo"}}},
        "linear": {"plugin":"linear","config":linear_block(&sandbox)},
    })));

    let first = reported(&ok(
        &sandbox,
        &["task", "copy", "authored:A", "--to", "linear", "--json"],
    ));
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].2, "created");
    let destination = first[0].1.as_str().unwrap().to_owned();
    let item = shown(&sandbox, "task", &destination);
    assert_eq!(item["title"], "Authored locally");
    assert_eq!(item["content"], "visible body");
    assert_eq!(item["status"]["name"], "Todo");
    assert_eq!(item["labels"][0]["name"], "bug");
    assert_eq!(item["repositories"], json!(["github.com/acme/work"]));
    for key in ["object", "array", "string", "number", "boolean", "null"] {
        let source = shown(&sandbox, "task", "authored:A");
        assert_eq!(
            item["metadata"][key], source["metadata"][key],
            "metadata key {key}"
        );
    }
    let second = reported(&ok(
        &sandbox,
        &["task", "copy", "authored:A", "--to", "linear", "--json"],
    ));
    assert_eq!(second[0].1, destination);
    assert!(matches!(second[0].2.as_str(), "updated" | "unchanged"));
}

#[test]
fn linear_project_and_task_copies_write_native_relations_and_record_only_cross_source_edges() {
    let sandbox = Sandbox::new();
    let root = sandbox.subdirectory("linear-graph-source");
    std::fs::create_dir_all(root.join("tasks")).unwrap();
    std::fs::create_dir_all(root.join("projects")).unwrap();
    for (path, title, dependencies) in [
        ("tasks/FAR.md", "Far task", ""),
        (
            "tasks/NEAR.md",
            "Near task",
            "depends_on: [FAR, {id: \"elsewhere:P-9\", item: project}]\n",
        ),
        ("tasks/CHILD.md", "Project child", "project: NEAR\n"),
        ("projects/FAR.md", "Far project", ""),
        (
            "projects/NEAR.md",
            "Near project",
            "labels: [{id: local-roadmap, name: roadmap}]\nmetadata: {caller.project: {enabled: true}}\nrepositories: [github.com/acme/project]\ndepends_on: [FAR, {id: \"elsewhere:T-9\", item: task}]\n",
        ),
    ] {
        std::fs::write(
            root.join(path),
            format!("---\ntitle: {title}\nstatus: Todo\n{dependencies}---\nbody\n"),
        )
        .unwrap();
    }
    sandbox.project_document(&document(&json!({
        "authored": {"plugin":"local-md","config":{"root":root,"status_mapping":{"Todo":"todo"}}},
        "linear": {"plugin":"linear","config":linear_block(&sandbox)},
    })));
    let task_far = reported(&ok(
        &sandbox,
        &["task", "copy", "authored:FAR", "--to", "linear", "--json"],
    ))[0]
        .1
        .as_str()
        .unwrap()
        .to_owned();
    let task_near = reported(&ok(
        &sandbox,
        &["task", "copy", "authored:NEAR", "--to", "linear", "--json"],
    ))[0]
        .1
        .as_str()
        .unwrap()
        .to_owned();
    let project_far = reported(&ok(
        &sandbox,
        &[
            "project",
            "copy",
            "authored:FAR",
            "--to",
            "linear",
            "--no-tasks",
            "--json",
        ],
    ))[0]
        .1
        .as_str()
        .unwrap()
        .to_owned();
    let project_report = reported(&ok(
        &sandbox,
        &[
            "project",
            "copy",
            "authored:NEAR",
            "--to",
            "linear",
            "--json",
        ],
    ));
    assert_eq!(
        project_report.len(),
        2,
        "the project copy includes its task"
    );
    let project_near = project_report[0].1.as_str().unwrap().to_owned();
    let written_project = shown(&sandbox, "project", &project_near);
    assert_eq!(written_project["content"], "body");
    assert_eq!(written_project["status"]["name"], "Todo");
    assert_eq!(written_project["labels"][0]["name"], "roadmap");
    assert_eq!(
        written_project["metadata"]["caller.project"]["enabled"],
        true
    );
    assert_eq!(
        written_project["repositories"],
        json!(["github.com/acme/project"])
    );
    let child = shown(&sandbox, "task", project_report[1].1.as_str().unwrap());
    assert_eq!(child["project"], project_near.split_once(':').unwrap().1);
    let repeated = reported(&ok(
        &sandbox,
        &[
            "project",
            "copy",
            "authored:NEAR",
            "--to",
            "linear",
            "--json",
        ],
    ));
    assert_eq!(
        repeated.iter().map(|item| &item.1).collect::<Vec<_>>(),
        project_report
            .iter()
            .map(|item| &item.1)
            .collect::<Vec<_>>()
    );

    let edges = |verb: &str, id: &str, reverse: bool| {
        let mut args = vec![verb, "deps", id, "--json"];
        if reverse {
            args.splice(3..3, ["--direction", "depended-on-by"]);
        }
        serde_json::from_str::<Value>(&ok(&sandbox, &args)).unwrap()["items"]
            .as_array()
            .unwrap()
            .clone()
    };
    let task_forward = edges("task", &task_near, false);
    assert_eq!(
        task_forward.len(),
        2,
        "the repeated write does not duplicate relations"
    );
    assert!(
        task_forward.iter().any(|edge| edge["to"]["id"] == task_far),
        "{task_forward:#?}"
    );
    assert!(
        task_forward
            .iter()
            .any(|edge| edge["to"]["id"] == "elsewhere:P-9")
    );
    assert!(
        edges("task", &task_far, true)
            .iter()
            .any(|edge| edge["from"]["id"] == task_near)
    );
    let project_forward = edges("project", &project_near, false);
    assert_eq!(
        project_forward.len(),
        2,
        "the repeated project write replaces its relation set"
    );
    assert!(
        project_forward
            .iter()
            .any(|edge| edge["to"]["id"] == project_far)
    );
    assert!(
        project_forward
            .iter()
            .any(|edge| edge["to"]["id"] == "elsewhere:T-9")
    );
    assert!(
        edges("project", &project_far, true)
            .iter()
            .any(|edge| edge["from"]["id"] == project_near)
    );

    for (verb, id) in [("task", task_near.clone()), ("project", project_near)] {
        let item = shown(&sandbox, verb, &id);
        let recorded = item["metadata"]["onetaskgraph.depends_on"]
            .as_array()
            .unwrap();
        assert_eq!(recorded.len(), 1, "only the cross-source edge is recorded");
        assert!(
            recorded[0]["id"]
                .as_str()
                .unwrap()
                .starts_with("elsewhere:")
        );
    }
    let near_file = root.join("tasks/NEAR.md");
    let edited = std::fs::read_to_string(&near_file).unwrap().replace(
        "[FAR, {id: \"elsewhere:P-9\", item: project}]",
        &format!("[{{id: \"{project_far}\", item: project}}]"),
    );
    std::fs::write(&near_file, edited).unwrap();
    ok(
        &sandbox,
        &["task", "copy", "authored:NEAR", "--to", "linear"],
    );
    let replaced = edges("task", &task_near, false);
    assert_eq!(replaced.len(), 1);
    assert_eq!(replaced[0]["to"]["id"], project_far);
    let recorded = shown(&sandbox, "task", &task_near)["metadata"]["onetaskgraph.depends_on"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(
        recorded[0]["kind"], "project",
        "a same-source cross-kind far end uses the fallback because an issue relation cannot name a project"
    );
}

#[test]
fn a_round_trip_edit_updates_the_item_it_came_from_rather_than_duplicating_it() {
    let sandbox = Sandbox::new();
    folders(&sandbox);

    // Out of the destination and into Markdown.
    let copied = ok(&sandbox, &["task", "copy", "remote:ENG-1", "--to", NOTES]);
    assert_eq!(
        copied.split_whitespace().collect::<Vec<_>>(),
        ["remote:ENG-1", "notes:ENG-1", "created"]
    );
    let before = shown(&sandbox, "task", "remote:ENG-1");

    // Edited the way a person edits it: one field, in the file.
    let file = sandbox.project().join(NOTES).join("tasks/ENG-1.md");
    let text = std::fs::read_to_string(&file).expect("the copied Markdown is there");
    assert!(
        text.contains("onetaskgraph.origin: remote:ENG-1"),
        "the copy records where it came from:\n{text}"
    );
    std::fs::write(
        &file,
        text.replace(
            "title: Rate-limit the sync loop",
            "title: Rate-limit the sync loop, carefully",
        ),
    )
    .expect("the edit lands");

    let back = ok(
        &sandbox,
        &["task", "copy", "notes:ENG-1", "--to", "remote", "--json"],
    );
    assert_eq!(
        reported(&back),
        vec![(
            "notes:ENG-1".to_owned(),
            json!("remote:ENG-1"),
            "updated".to_owned()
        )]
    );

    // Exactly one item where there was one before: the copy back updated, and the
    // folder holds no second document.
    assert_eq!(
        ok(&sandbox, &["task", "list", "--source", "remote"])
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count(),
        1
    );

    let after = shown(&sandbox, "task", "remote:ENG-1");
    assert_eq!(
        after["title"],
        json!("Rate-limit the sync loop, carefully"),
        "the edited field changed"
    );
    // And every field the edit did not touch is byte-for-byte what it was.
    for field in [
        "id",
        "content",
        "status",
        "labels",
        "project",
        "repositories",
        "url",
        "created_at",
        "updated_at",
    ] {
        assert_eq!(
            after[field], before[field],
            "{field} survived the round trip"
        );
    }
    for key in ["caller.count", "caller.shape"] {
        assert_eq!(
            after["metadata"][key], before["metadata"][key],
            "{key} survived the round trip"
        );
    }
    // The one thing that did change is the correspondence itself, which is the mechanism
    // rather than a field of the user's: the remote item now records where its last copy
    // came from, so the next edit finds it directly.
    assert_eq!(
        after["metadata"]["onetaskgraph.origin"],
        json!("notes:ENG-1")
    );
}

#[test]
fn every_source_kind_can_be_copied_into_a_folder_of_markdown_with_its_fields_intact() {
    // A journey written once and run against every configured source kind, so no plugin
    // is proven by a suite of its own writing. The destination is a Markdown folder
    // because that is the one every source can be copied *into* today, and it is what the
    // user's own flow authors and edits in.
    for row in ROWS {
        let sandbox = Sandbox::new();
        sandbox.project_document(&row.document_with_folder(&sandbox, NOTES));
        let from = qualified(SOURCE, "T-1");

        let planned = ok(&sandbox, &["task", "copy", &from, "--to", NOTES, "--json"]);
        assert_eq!(
            reported(&planned),
            vec![(from.clone(), json!("notes:T-1"), "created".to_owned())],
            "{}",
            row.name
        );

        let source = shown(&sandbox, "task", &from);
        let copied = shown(&sandbox, "task", "notes:T-1");
        for field in ["title", "content", "status", "labels", "repositories"] {
            assert_eq!(copied[field], source[field], "{}: {field}", row.name);
        }
        // Value and JSON type alike, for every key the caller owns.
        for key in ["onepipeline.turn_budget", "caller.flags"] {
            assert_eq!(
                copied["metadata"][key], source["metadata"][key],
                "{}: {key}",
                row.name
            );
        }
        assert_eq!(
            copied["metadata"]["onetaskgraph.origin"],
            json!(from),
            "{}",
            row.name
        );
        // `url` is the destination's own and is never written.
        assert_eq!(copied["url"], Value::Null, "{}", row.name);

        // A second copy of the same item updates that one and creates nothing.
        let again = ok(&sandbox, &["task", "copy", &from, "--to", NOTES, "--json"]);
        assert_eq!(
            reported(&again),
            vec![(from.clone(), json!("notes:T-1"), "unchanged".to_owned())],
            "{}",
            row.name
        );
        assert_eq!(
            ok(&sandbox, &["task", "list", "--source", NOTES])
                .lines()
                .filter(|line| !line.trim().is_empty())
                .count(),
            1,
            "{}",
            row.name
        );
    }
}

/// The GitHub board every journey below copies into, beside the folder it copies from.
fn board_with_plans(sandbox: &Sandbox, folder: &str) -> crate::fixtures::GitHubBoardFields {
    let (config, board) = github_projects_with_board(sandbox);
    sandbox.project_document(&document(&json!({
        folder: {"plugin":"local-md","config":{
            "root": sandbox.subdirectory(folder),
            "status_mapping": {"Todo":"todo","Doing":"in-progress","Shipped":"done",
                               "Idea":"draft"}}},
        "board": {"plugin":"github-projects","config":config}
    })));
    board
}

#[test]
fn a_project_and_its_tasks_copy_into_a_board_without_touching_the_board_itself() {
    // The defect this replaces: the source resolved to one board id and treated it as the
    // project, so copying a project into it renamed a real user's board. A board is a
    // container of projects now — a project lands as an issue and its tasks land as that
    // issue's sub-issues — and the board's own fields are never written by anything here.
    let sandbox = Sandbox::new();
    let root = sandbox.subdirectory("plans");
    std::fs::create_dir_all(root.join("projects")).unwrap();
    std::fs::create_dir_all(root.join("tasks")).unwrap();
    std::fs::write(root.join("projects/P-1.md"), "---\ntitle: Published roadmap\nstatus: Doing\nmetadata: {caller.approved: true, caller.shape: {nested: [1, true, null]}}\n---\nThe permanent plan\n").unwrap();
    std::fs::write(
        root.join("tasks/A.md"),
        "---\ntitle: First step\nstatus: Todo\nproject: P-1\n---\ndo this first\n",
    )
    .unwrap();
    std::fs::write(
        root.join("tasks/B.md"),
        "---\ntitle: Second step\nstatus: Todo\nproject: P-1\n---\nthen this\n",
    )
    .unwrap();
    let board = board_with_plans(&sandbox, "plans");
    let before = board.own();
    assert_eq!(
        before,
        json!({"title":"Fixture board",
               "shortDescription":"the board a person set up",
               "readme":"# Fixture board\n\nA person wrote this."}),
        "the board this copy lands on is a person's, with a title and a readme of its own"
    );

    let copied = ok(
        &sandbox,
        &["project", "copy", "plans:P-1", "--to", "board", "--json"],
    );
    let reported = reported(&copied);
    assert_eq!(
        reported
            .iter()
            .map(|(source, _, action)| (source.as_str(), action.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("plans:P-1", "created"),
            ("plans:A", "created"),
            ("plans:B", "created"),
        ]
    );
    let project = reported[0].1.as_str().expect("a created project id");

    let written = shown(&sandbox, "project", project);
    assert_eq!(written["title"], "Published roadmap");
    assert_eq!(written["content"], "The permanent plan");
    assert_eq!(written["status"]["category"], "in-progress");
    assert_eq!(written["metadata"]["caller.approved"], true);
    assert_eq!(
        written["metadata"]["caller.shape"],
        json!({"nested":[1,true,null]}),
        "unbounded caller JSON survives with its types intact"
    );

    let listed = ok(&sandbox, &["task", "list", "--source", "board"]);
    for title in ["First step", "Second step"] {
        assert!(listed.contains(title), "{listed}");
    }
    let filed = shown(&sandbox, "task", reported[1].1.as_str().unwrap());
    assert_eq!(
        filed["project"].as_str(),
        project.strip_prefix("board:"),
        "a project's tasks are that issue's sub-issues"
    );
    assert_eq!(filed["status"]["name"], "Todo");

    assert_eq!(
        board.own(),
        before,
        "the board's own title, shortDescription and readme are never written"
    );
}

#[test]
fn a_project_whose_goal_outgrows_a_board_description_still_copies() {
    // Why the metadata slot moved out of `shortDescription`: that field is capped at 300
    // characters, and the metadata comment spent about 110 of them before any content, so
    // a project carrying an ordinary goal statement could not be copied at all.
    let sandbox = Sandbox::new();
    let root = sandbox.subdirectory("plans");
    std::fs::create_dir_all(root.join("projects")).unwrap();
    let goal =
        "This plan exists so that the whole harness can author its work on a board. ".repeat(6);
    assert!(goal.len() > 300, "the goal must outgrow the old slot");
    std::fs::write(
        root.join("projects/P-1.md"),
        format!("---\ntitle: Long goal\nstatus: Todo\nmetadata: {{onepipeline.steps: [{goal:?}]}}\n---\n{goal}\n"),
    )
    .unwrap();
    board_with_plans(&sandbox, "plans");

    let copied = ok(
        &sandbox,
        &[
            "project",
            "copy",
            "plans:P-1",
            "--to",
            "board",
            "--no-tasks",
            "--json",
        ],
    );
    let id = reported(&copied)[0].1.as_str().expect("an id").to_owned();
    let written = shown(&sandbox, "project", &id);
    assert_eq!(written["content"], goal.trim_end());
    assert_eq!(written["metadata"]["onepipeline.steps"], json!([goal]));
}

#[test]
fn a_board_that_fails_between_creating_an_issue_and_filing_it_says_so_and_recovers() {
    // Landing an item on a board is two calls — `createIssue`, then
    // `addProjectV2ItemById` — so GitHub can fail between them, and what happens then is
    // a journey rather than a reading of the code: the copy exits non-zero saying what
    // GitHub said, the board holds nothing it was not already holding, and the retry
    // lands exactly one item rather than two.
    let sandbox = Sandbox::new();
    let root = sandbox.subdirectory("plans");
    std::fs::create_dir_all(root.join("tasks")).unwrap();
    std::fs::write(
        root.join("tasks/A.md"),
        "---\ntitle: First step\nstatus: Todo\n---\ndo this first\n",
    )
    .unwrap();
    sandbox.project_document(&document(&json!({
        "plans": {"plugin":"local-md","config":{
            "root": root, "status_mapping": {"Todo":"todo"}}},
        "board": {"plugin":"github-projects",
                  "config": github_projects_failing_to_file_once(&sandbox)}
    })));

    let said = refused(&sandbox, &["task", "copy", "plans:A", "--to", "board"], 1);
    assert!(
        said.contains("Something went wrong while executing your query"),
        "the failure GitHub reported is what the caller is told: {said}"
    );
    let listed = ok(&sandbox, &["task", "list", "--source", "board"]);
    assert!(
        !listed.contains("First step"),
        "an issue that was never filed is on no board: {listed}"
    );

    let copied = ok(
        &sandbox,
        &["task", "copy", "plans:A", "--to", "board", "--json"],
    );
    let landed = reported(&copied);
    assert_eq!(landed.len(), 1, "{copied}");
    assert_eq!(
        landed[0].1,
        json!("board:ISSUE-2"),
        "the issue the failed attempt created is on no board, so nothing can match it and \
         the retry creates its own — the orphan stays in the repository: {copied}"
    );
    let listed = ok(&sandbox, &["task", "list", "--source", "board"]);
    assert_eq!(
        listed.matches("First step").count(),
        1,
        "the retry lands one item where the failed attempt landed none: {listed}"
    );
}

#[test]
fn a_dependency_naming_a_board_item_as_the_wrong_kind_is_refused_before_anything_lands() {
    // The board holds the far end itself, so it is the board that says which kind `P-2`
    // is — a project. An edge naming it a task would be stored as a relationship at a
    // level it is not at, and the refusal comes before the issue is created.
    let sandbox = Sandbox::new();
    let root = sandbox.subdirectory("plans");
    std::fs::create_dir_all(root.join("tasks")).unwrap();
    std::fs::write(
        root.join("tasks/A.md"),
        "---\ntitle: First step\nstatus: Todo\n\
         depends_on: [{id: \"board:P-2\", item: task}]\n---\ndo this first\n",
    )
    .unwrap();
    board_with_plans(&sandbox, "plans");

    let said = refused(&sandbox, &["task", "copy", "plans:A", "--to", "board"], 1);
    for expected in ["P-2", "project", "task"] {
        assert!(said.contains(expected), "{expected} is named: {said}");
    }
    let listed = ok(&sandbox, &["task", "list", "--source", "board"]);
    assert!(
        !listed.contains("First step"),
        "nothing was created before the refusal: {listed}"
    );
}

#[test]
fn a_field_write_that_fails_after_an_issue_is_filed_takes_the_issue_back() {
    // The later half of the same sequence: the issue exists and is on the board, and the
    // board field that would let the next copy find it is what did not land.
    //
    // That state used to be left behind, and `--match-by title` was the escape a person
    // had to know to reach for. It is not left behind any more: creating an item here is
    // several calls, GitHub can fail at any of them, and a write that refused must not
    // leave an item nobody asked for — so the source takes back the issue it created and
    // the plain retry is a clean one. The escape itself is unaffected and is proven where
    // it belongs, over a correspondence a *person* removed.
    let sandbox = Sandbox::new();
    let root = sandbox.subdirectory("plans");
    std::fs::create_dir_all(root.join("tasks")).unwrap();
    std::fs::write(
        root.join("tasks/A.md"),
        "---\ntitle: First step\nstatus: Doing\n---\ndo this first\n",
    )
    .unwrap();
    sandbox.project_document(&document(&json!({
        "plans": {"plugin":"local-md","config":{
            "root": root, "status_mapping": {"Doing":"in-progress"}}},
        "board": {"plugin":"github-projects",
                  "config": github_projects_failing_a_field_write_once(&sandbox)}
    })));

    let said = refused(&sandbox, &["task", "copy", "plans:A", "--to", "board"], 1);
    assert!(
        said.contains("Something went wrong while executing your query"),
        "the failure GitHub reported is what the caller is told: {said}"
    );
    let listed = ok(&sandbox, &["task", "list", "--source", "board"]);
    assert_eq!(
        listed.matches("First step").count(),
        0,
        "the issue the failed write created is not left on the board: {listed}"
    );

    // The failure is spent, so the plain retry — no escape, no flag — creates the one
    // issue this plan owes, with the status the failed attempt could not write.
    let copied = ok(
        &sandbox,
        &["task", "copy", "plans:A", "--to", "board", "--json"],
    );
    let landed = reported(&copied);
    assert_eq!(landed.len(), 1, "{copied}");
    assert_eq!(landed[0].2, "created", "{copied}");
    let finished = shown(&sandbox, "task", landed[0].1.as_str().expect("an id"));
    assert_eq!(
        finished["status"]["category"], "in-progress",
        "the status the failed attempt could not write is what the retry lands"
    );
    let listed = ok(&sandbox, &["task", "list", "--source", "board"]);
    assert_eq!(
        listed.matches("First step").count(),
        1,
        "and one plan is one issue rather than two: {listed}"
    );
}

#[test]
fn a_status_this_integration_cannot_hold_is_refused_naming_it_and_the_source() {
    // `draft` is an ordinary status everywhere else. This source refuses it, and says why:
    // a GitHub draft issue cannot have sub-issues, and a project's tasks are sub-issues.
    let sandbox = Sandbox::new();
    let root = sandbox.subdirectory("plans");
    std::fs::create_dir_all(root.join("tasks")).unwrap();
    std::fs::write(
        root.join("tasks/A.md"),
        "---\ntitle: Not yet committed\nstatus: Idea\n---\nan idea\n",
    )
    .unwrap();
    board_with_plans(&sandbox, "plans");

    assert_eq!(
        shown(&sandbox, "task", "plans:A")["status"]["category"],
        "draft",
        "draft is an ordinary accepted status in the source it came from"
    );
    let complaint = refused(&sandbox, &["task", "copy", "plans:A", "--to", "board"], 1);
    assert!(complaint.contains("draft"), "{complaint}");
    assert!(complaint.contains("board"), "{complaint}");
    assert!(complaint.contains("sub-issue"), "{complaint}");
}

#[test]
fn a_copy_into_a_board_settles_instead_of_reporting_a_change_on_every_run() {
    // Writing `done` closes the issue, and writing a non-terminal status over a closed one
    // has to reopen it. Without that the item reads back `Unknown` and this loop never
    // reaches `unchanged` — a copy would report a change forever.
    let sandbox = Sandbox::new();
    let root = sandbox.subdirectory("plans");
    std::fs::create_dir_all(root.join("tasks")).unwrap();
    let write_status = |status: &str| {
        std::fs::write(
            root.join("tasks/A.md"),
            format!("---\ntitle: One step\nstatus: {status}\n---\ndo this\n"),
        )
        .unwrap();
    };
    write_status("Todo");
    board_with_plans(&sandbox, "plans");

    let copy = |sandbox: &Sandbox| {
        reported(&ok(
            sandbox,
            &["task", "copy", "plans:A", "--to", "board", "--json"],
        ))
    };
    let created = copy(&sandbox);
    assert_eq!(created[0].2, "created");
    let id = created[0].1.as_str().expect("an id").to_owned();
    assert_eq!(copy(&sandbox)[0].2, "unchanged");

    write_status("Shipped");
    assert_eq!(copy(&sandbox)[0].2, "updated");
    assert_eq!(shown(&sandbox, "task", &id)["status"]["category"], "done");
    assert_eq!(
        copy(&sandbox)[0].2,
        "unchanged",
        "a closed issue reads back as the status that closed it"
    );

    write_status("Todo");
    assert_eq!(copy(&sandbox)[0].2, "updated");
    let reopened = shown(&sandbox, "task", &id);
    assert_eq!(
        reopened["status"],
        json!({"category":"todo","name":"Todo"}),
        "a non-terminal status reopens the issue rather than leaving it Unknown"
    );
    assert_eq!(copy(&sandbox)[0].2, "unchanged");
}

#[test]
fn github_projects_is_a_permanent_destination_whose_created_items_are_issues() {
    let sandbox = Sandbox::new();
    let root = sandbox.subdirectory("authored");
    std::fs::create_dir_all(root.join("tasks")).unwrap();
    std::fs::write(root.join("tasks/PLAN-1.md"), "---\ntitle: Publish the plan\nstatus: Todo\nmetadata: {caller.count: 3, caller.shape: {nested: [1, true, null]}}\nrepositories: [github.com/nickderobertis/onetaskgraph]\ndepends_on: [PLAN-2, {id: 'elsewhere:T-9', item: task}]\n---\nshare this plan\n").unwrap();
    std::fs::write(
        root.join("tasks/PLAN-2.md"),
        "---\ntitle: Supporting plan\nstatus: Todo\n---\nsupport it\n",
    )
    .unwrap();
    board_with_plans(&sandbox, "authored");

    let first = ok(
        &sandbox,
        &[
            "task",
            "copy",
            "authored:PLAN-1",
            "authored:PLAN-2",
            "--to",
            "board",
            "--json",
        ],
    );
    let first = reported(&first);
    assert_eq!(
        first
            .iter()
            .map(|(source, _, action)| (source.as_str(), action.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("authored:PLAN-1", "created"),
            ("authored:PLAN-2", "created"),
        ]
    );
    let one = first[0].1.as_str().expect("an id").to_owned();
    let two = first[1].1.as_str().expect("an id").to_owned();

    let copied = shown(&sandbox, "task", &one);
    assert_eq!(copied["title"], "Publish the plan");
    assert_eq!(copied["content"], "share this plan");
    assert_eq!(copied["status"]["name"], "Todo");
    assert_eq!(copied["metadata"]["caller.count"], 3);
    assert_eq!(
        copied["metadata"]["caller.shape"],
        json!({"nested":[1,true,null]})
    );
    assert_eq!(
        copied["repositories"],
        json!(["github.com/nickderobertis/onetaskgraph"]),
        "a list that is exactly the issue's own repository is derived rather than recorded"
    );

    let dependencies: Value =
        serde_json::from_str(&ok(&sandbox, &["task", "deps", &one, "--json"])).unwrap();
    assert_eq!(
        dependencies["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|edge| edge["to"].clone())
            .collect::<Vec<_>>(),
        vec![
            json!({"id":two,"kind":"task"}),
            json!({"id":"elsewhere:T-9","kind":"task"})
        ],
        "the far end inside the copied set is native and the one elsewhere is recorded"
    );

    let second = ok(
        &sandbox,
        &["task", "copy", "authored:PLAN-1", "--to", "board", "--json"],
    );
    // Copying PLAN-1 alone is not the same copy: PLAN-2 is no longer in the copied set, so
    // its edge is rewritten to name the source it is still in. That is a real change, and
    // the point here is that it lands on the item already there rather than beside it.
    assert_eq!(
        reported(&second),
        vec![("authored:PLAN-1".into(), json!(one), "updated".into())]
    );
    assert_eq!(
        ok(&sandbox, &["task", "list", "--source", "board"])
            .lines()
            .count(),
        6,
        "a second copy of the same item updates it rather than duplicating it"
    );
}

#[test]
fn copying_an_issue_back_updates_fields_and_replaces_a_native_dependency() {
    let sandbox = Sandbox::new();
    let github = ROWS
        .iter()
        .find(|row| row.plugin == "github-projects")
        .unwrap();
    sandbox.project_document(&github.document_with_folder(&sandbox, NOTES));
    ok(&sandbox, &["task", "copy", "work:T-1", "--to", NOTES]);
    let file = sandbox.project().join(NOTES).join("tasks/T-1.md");
    std::fs::write(&file, "---\ntitle: Alpha engine revised\nstatus: Todo\nlabels: [{id: L-1, name: bug}, {id: L-3, name: core}]\nmetadata: {onetaskgraph.origin: 'work:T-1'}\nrepositories: [github.com/nickderobertis/onetaskgraph]\ndepends_on: [{id: 'work:T-3', item: task}]\n---\nthe engine core\n").unwrap();
    let copied = ok(
        &sandbox,
        &["task", "copy", "notes:T-1", "--to", "work", "--json"],
    );
    assert_eq!(
        reported(&copied),
        vec![("notes:T-1".into(), json!("work:T-1"), "updated".into())]
    );
}

#[test]
fn several_ids_in_one_command_are_one_copied_set_whose_edges_are_recreated() {
    // The ids named together *are* the set: an edge between two of them is recreated at
    // the destination, which one command per id could not do — the far end's own
    // destination id is not known until the copy that creates it has run.
    let row = &ROWS[0];
    let sandbox = Sandbox::new();
    sandbox.project_document(&row.document_with_folder(&sandbox, NOTES));
    let first = qualified(SOURCE, "T-1");
    let second = qualified(SOURCE, "T-2");

    let copied = ok(
        &sandbox,
        &["task", "copy", &first, &second, "--to", NOTES, "--json"],
    );
    assert_eq!(
        reported(&copied),
        vec![
            (first.clone(), json!("notes:T-1"), "created".to_owned()),
            (second.clone(), json!("notes:T-2"), "created".to_owned()),
        ]
    );

    let edges = ok(&sandbox, &["task", "deps", "notes:T-1", "--json"]);
    let edges: Value = serde_json::from_str(&edges).expect("deps emits JSON");
    let ends: Vec<&str> = edges["items"]
        .as_array()
        .expect("an array of edges")
        .iter()
        .map(|edge| edge["to"]["id"].as_str().expect("a qualified id"))
        .collect();
    assert!(
        ends.contains(&"notes:T-2"),
        "the far end inside the copied set is the destination's own id: {ends:?}"
    );
    assert!(
        ends.contains(&"elsewhere:P-9"),
        "a far end already naming another source is left exactly as it is: {ends:?}"
    );
}

#[test]
fn a_dry_run_reads_everything_writes_nothing_and_says_what_it_would_have_done() {
    let sandbox = Sandbox::new();
    folders(&sandbox);

    let planned = ok(
        &sandbox,
        &[
            "task",
            "copy",
            "remote:ENG-1",
            "--to",
            NOTES,
            "--dry-run",
            "--json",
        ],
    );
    assert_eq!(
        reported(&planned),
        vec![("remote:ENG-1".to_owned(), Value::Null, "created".to_owned())],
        "a dry run that would create has no destination id, because nothing was created"
    );
    assert!(
        !sandbox.project().join(NOTES).join("tasks").exists(),
        "a dry run writes nothing"
    );

    // And once something is there, a dry run over it names the id it would update.
    ok(&sandbox, &["task", "copy", "remote:ENG-1", "--to", NOTES]);
    let file = sandbox.project().join(NOTES).join("tasks/ENG-1.md");
    let before = std::fs::read(&file).expect("the copied Markdown is there");
    std::fs::write(
        &file,
        String::from_utf8(before.clone())
            .expect("UTF-8")
            .replace("title: Rate-limit", "title: Edited rate-limit"),
    )
    .expect("the edit lands");
    let edited = std::fs::read(&file).expect("the edited Markdown is there");

    let planned = ok(
        &sandbox,
        &[
            "task",
            "copy",
            "remote:ENG-1",
            "--to",
            NOTES,
            "--dry-run",
            "--json",
        ],
    );
    assert_eq!(
        reported(&planned),
        vec![(
            "remote:ENG-1".to_owned(),
            json!("notes:ENG-1"),
            "updated".to_owned()
        )]
    );
    assert_eq!(
        std::fs::read(&file).expect("the Markdown is still there"),
        edited,
        "a dry run over an item it would update still writes nothing"
    );
}

#[test]
fn a_destination_configured_with_no_write_side_exits_non_zero_naming_it_and_its_plugin() {
    let sandbox = Sandbox::new();
    sandbox.project_document(&document(&json!({
        SOURCE: {"plugin": "in-memory", "config": {"tasks": [
            {"id": "T-1", "title": "Alpha", "status": {"category": "todo", "name": "Todo"},
             "labels": []}
        ]}},
        "sealed": {"plugin": "in-memory", "config": {
            "capabilities": {"writes": "unsupported"}
        }},
    })));

    let said = refused(
        &sandbox,
        &["task", "copy", &qualified(SOURCE, "T-1"), "--to", "sealed"],
        1,
    );
    assert!(said.contains("source sealed cannot be written"), "{said}");
    assert!(said.contains("its plugin is in-memory"), "{said}");
    assert!(said.contains("sources list"), "{said}");
}

#[test]
fn a_destination_that_cannot_carry_a_key_refuses_the_write_naming_the_source_and_the_keys() {
    let sandbox = Sandbox::new();
    sandbox.project_document(&document(&json!({
        SOURCE: {"plugin": "in-memory", "config": {"tasks": [
            {"id": "T-1", "title": "Alpha", "status": {"category": "todo", "name": "Todo"},
             "labels": [], "metadata": {"caller.flags": [true, null], "caller.count": 3}}
        ]}},
        "picky": {"plugin": "in-memory", "config": {
            "capabilities": {"unwritable_metadata_keys": ["caller.flags", "caller.count"]}
        }},
    })));

    let said = refused(
        &sandbox,
        &["task", "copy", &qualified(SOURCE, "T-1"), "--to", "picky"],
        1,
    );
    assert!(said.contains("source picky could not do it"), "{said}");
    assert!(said.contains("caller.count, caller.flags"), "{said}");
    assert!(
        !said.contains("dropped"),
        "the keys are named rather than dropped: {said}"
    );
}

#[test]
fn a_stale_origin_refuses_and_recreate_falls_through_to_matching_by_origin_instead() {
    let sandbox = Sandbox::new();
    let remote = folders(&sandbox);
    ok(&sandbox, &["task", "copy", "remote:ENG-1", "--to", NOTES]);

    // Somebody deletes the counterpart at the destination on purpose.
    std::fs::remove_file(remote.join("tasks/ENG-1.md")).expect("the remote document goes away");

    let said = refused(
        &sandbox,
        &["task", "copy", "notes:ENG-1", "--to", "remote"],
        1,
    );
    assert!(
        said.contains("notes:ENG-1 was copied from remote:ENG-1"),
        "{said}"
    );
    assert!(said.contains("--recreate"), "{said}");

    let created = ok(
        &sandbox,
        &[
            "task",
            "copy",
            "notes:ENG-1",
            "--to",
            "remote",
            "--recreate",
            "--json",
        ],
    );
    assert_eq!(
        reported(&created),
        vec![(
            "notes:ENG-1".to_owned(),
            json!("remote:ENG-1"),
            "created".to_owned()
        )]
    );
}

#[test]
fn an_origin_a_person_removed_creates_until_match_by_re_establishes_the_correspondence() {
    let sandbox = Sandbox::new();
    folders(&sandbox);
    ok(&sandbox, &["task", "copy", "remote:ENG-1", "--to", NOTES]);

    // A person edits the Markdown and deletes the key: neither rule can find the
    // counterpart any more, so the next copy creates a second document.
    let file = sandbox.project().join(NOTES).join("tasks/ENG-1.md");
    let text = std::fs::read_to_string(&file).expect("the copied Markdown is there");
    std::fs::write(
        &file,
        text.replace("  onetaskgraph.origin: remote:ENG-1\n", ""),
    )
    .expect("the edit lands");

    let duplicated = ok(
        &sandbox,
        &["task", "copy", "remote:ENG-1", "--to", NOTES, "--json"],
    );
    assert_eq!(
        reported(&duplicated),
        vec![(
            "remote:ENG-1".to_owned(),
            json!("notes:ENG-1-2"),
            "created".to_owned()
        )]
    );

    // The caller-named escape re-establishes it without hand-editing ids.
    std::fs::remove_file(sandbox.project().join(NOTES).join("tasks/ENG-1-2.md"))
        .expect("the duplicate goes away");
    let matched = ok(
        &sandbox,
        &[
            "task",
            "copy",
            "remote:ENG-1",
            "--to",
            NOTES,
            "--match-by",
            "title",
            "--json",
        ],
    );
    assert_eq!(
        reported(&matched),
        vec![(
            "remote:ENG-1".to_owned(),
            json!("notes:ENG-1"),
            "updated".to_owned()
        )]
    );
}

#[test]
fn copying_a_project_carries_its_tasks_and_reports_one_the_source_no_longer_holds() {
    let sandbox = Sandbox::new();
    let root = sandbox.subdirectory("remote");
    for (kind, id, front) in [
        (
            "projects",
            "P-1",
            "title: Engine\nstatus: doing\ndepends_on: [{id: T-1, item: task}]",
        ),
        ("tasks", "T-1", "title: Alpha\nstatus: todo\nproject: P-1"),
        ("tasks", "T-2", "title: Beta\nstatus: todo\nproject: P-1"),
    ] {
        let path = root.join(kind).join(format!("{id}.md"));
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("the folder");
        std::fs::write(path, format!("---\n{front}\n---\nbody\n")).expect("the document");
    }
    sandbox.project_document(&document(&json!({
        "remote": {"plugin": "local-md", "config": {
            "root": root,
            "status_mapping": {"todo": "todo", "doing": "in-progress"},
        }},
        NOTES: {"plugin": "local-md", "config": empty_folder(&sandbox, NOTES)},
    })));

    // A dry run of a project the destination does not hold yet still reads every task in
    // it and reports what each would have got — there is simply no destination id for any
    // of them, because nothing was written and the project they would be filed under does
    // not exist.
    let planned = ok(
        &sandbox,
        &[
            "project",
            "copy",
            "remote:P-1",
            "--to",
            NOTES,
            "--dry-run",
            "--json",
        ],
    );
    assert_eq!(
        reported(&planned),
        vec![
            ("remote:P-1".to_owned(), Value::Null, "created".to_owned()),
            ("remote:T-1".to_owned(), Value::Null, "created".to_owned()),
            ("remote:T-2".to_owned(), Value::Null, "created".to_owned()),
        ]
    );
    assert!(
        !sandbox.project().join(NOTES).join("tasks").exists()
            && !sandbox.project().join(NOTES).join("projects").exists(),
        "a dry run writes nothing"
    );

    let copied = ok(
        &sandbox,
        &["project", "copy", "remote:P-1", "--to", NOTES, "--json"],
    );
    assert_eq!(
        reported(&copied),
        vec![
            (
                "remote:P-1".to_owned(),
                json!("notes:P-1"),
                "created".to_owned()
            ),
            (
                "remote:T-1".to_owned(),
                json!("notes:T-1"),
                "created".to_owned()
            ),
            (
                "remote:T-2".to_owned(),
                json!("notes:T-2"),
                "created".to_owned()
            ),
        ]
    );
    // Each copied task is filed under the destination project rather than the source's.
    assert_eq!(
        shown(&sandbox, "task", "notes:T-1")["project"],
        json!("P-1")
    );
    let dependencies: Value =
        serde_json::from_str(&ok(&sandbox, &["project", "deps", "notes:P-1", "--json"]))
            .expect("project dependencies emit JSON");
    assert!(
        dependencies["items"]
            .as_array()
            .expect("dependency items")
            .contains(&json!({
                "from": {"id": "notes:P-1", "kind": "project"},
                "to": {"id": "notes:T-1", "kind": "task"},
                "kind": "blocks"
            })),
        "the copied project edge is recreated between destination items: {dependencies:#}"
    );

    // A second copy matches each task independently and duplicates nothing.
    let again = ok(
        &sandbox,
        &["project", "copy", "remote:P-1", "--to", NOTES, "--json"],
    );
    assert!(
        reported(&again)
            .iter()
            .all(|(_, _, action)| action == "unchanged"),
        "{again}"
    );

    // A destination item the source no longer holds is left alone and reported.
    std::fs::remove_file(root.join("tasks/T-2.md")).expect("the source drops a task");
    let orphaned = ok(
        &sandbox,
        &["project", "copy", "remote:P-1", "--to", NOTES, "--json"],
    );
    assert!(
        reported(&orphaned).contains(&(
            "remote:T-2".to_owned(),
            json!("notes:T-2"),
            "orphaned".to_owned()
        )),
        "{orphaned}"
    );
    assert_eq!(shown(&sandbox, "task", "notes:T-2")["title"], json!("Beta"));

    let alone = ok(
        &sandbox,
        &[
            "project",
            "copy",
            "remote:P-1",
            "--to",
            NOTES,
            "--no-tasks",
            "--json",
        ],
    );
    assert_eq!(reported(&alone).len(), 1);
}

#[test]
fn a_copy_that_cannot_run_at_all_exits_non_zero_with_a_suggested_next_action() {
    let sandbox = Sandbox::new();
    folders(&sandbox);

    for (arguments, expected) in [
        (
            vec!["task", "copy", "remote:absent", "--to", NOTES],
            "no item with the id remote:absent",
        ),
        (
            vec!["task", "copy", "remote:ENG-1", "--to", "nowhere"],
            "no source named \"nowhere\" is configured",
        ),
        (
            vec!["task", "copy", "ENG-1", "--to", NOTES],
            "is not a qualified id",
        ),
        (
            vec!["task", "copy", "remote:ENG-1", "--to", "NOT A NAME"],
            "--to NOT A NAME",
        ),
    ] {
        let said = refused(&sandbox, &arguments, 1);
        assert!(said.contains(expected), "{arguments:?}: {said}");
        assert!(said.contains("next:"), "{arguments:?}: {said}");
    }
}

/// A folder holding one project and two tasks in it, the second blocking the first.
///
/// The shape a plan is authored in: `A` cannot start until `B` is done, and both are part
/// of one project. Copying it is what needs the far end of that edge — an item the same
/// run is creating — to be findable at the destination.
fn plans_with_a_dependency(sandbox: &Sandbox) -> std::path::PathBuf {
    let root = sandbox.subdirectory("plans");
    std::fs::create_dir_all(root.join("projects")).unwrap();
    std::fs::create_dir_all(root.join("tasks")).unwrap();
    std::fs::write(
        root.join("projects/P-1.md"),
        "---\ntitle: Published roadmap\nstatus: Doing\n---\nThe permanent plan\n",
    )
    .unwrap();
    std::fs::write(
        root.join("tasks/A.md"),
        "---\ntitle: First step\nstatus: Todo\nproject: P-1\n\
         depends_on: [B]\n---\ndo this after B\n",
    )
    .unwrap();
    std::fs::write(
        root.join("tasks/B.md"),
        "---\ntitle: Second step\nstatus: Todo\nproject: P-1\n---\nthen this\n",
    )
    .unwrap();
    root
}

#[test]
fn a_copy_resolves_a_dependency_on_an_item_it_created_in_the_same_run() {
    // The defect: three runs of one project copy each created some items and then refused,
    // naming "GitHub dependency item <node-id> was not found" — an item that same run had
    // just created. GitHub's board read is eventually consistent, so the far end of an edge
    // written moments after its creation was routinely absent from the read that looked it
    // up. The board this runs against never shows the item most recently filed on it, which
    // is that hazard with the timing taken out of it.
    let sandbox = Sandbox::new();
    plans_with_a_dependency(&sandbox);
    sandbox.project_document(&document(&json!({
        "plans": {"plugin":"local-md","config":{
            "root": sandbox.subdirectory("plans"),
            "status_mapping": {"Todo":"todo","Doing":"in-progress","Shipped":"done"}}},
        "board": {"plugin":"github-projects",
                  "config": github_projects_reading_one_item_behind(&sandbox)}
    })));

    let output = run(
        &sandbox,
        &["project", "copy", "plans:P-1", "--to", "board", "--json"],
    );
    let said = stderr(&output);
    assert!(
        !said.contains("was not found"),
        "no item this run created is reported as missing: {said}"
    );
    assert_eq!(output.status.code(), Some(0), "{said}");

    let landed = reported(&stdout(&output));
    assert_eq!(
        landed
            .iter()
            .map(|(source, _, action)| (source.as_str(), action.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("plans:P-1", "created"),
            ("plans:A", "created"),
            ("plans:B", "created"),
        ]
    );

    // The edge is really there, read back through the binary's own dependency verb, and it
    // names the destination's item rather than the id it had at its source.
    let first = landed[1].1.as_str().expect("a created task id");
    let second = landed[2].1.as_str().expect("a created task id");
    let edges = ok(&sandbox, &["task", "deps", first]);
    assert!(
        edges.contains(second),
        "{first} depends on {second} at the destination:\n{edges}"
    );
    assert!(
        !edges.contains("plans:B"),
        "the far end is the destination's own item, not the id it had at its source:\n{edges}"
    );
}

#[test]
fn a_copy_that_cannot_finish_leaves_the_board_as_it_found_it() {
    // A copy is either complete or it never happened. Half of one has to be run again, and
    // the re-run is the mutation burst that trips GitHub's secondary rate limiter — which
    // then refuses even reads for the next fifty minutes. So the copy undoes the items it
    // created and the retry starts from the board it started from.
    //
    // The board here fails the first field write onto an item it has already filed: the
    // issue exists and is on the board when the refusal arrives, which is exactly the state
    // that used to be left behind.
    let sandbox = Sandbox::new();
    plans_with_a_dependency(&sandbox);
    sandbox.project_document(&document(&json!({
        "plans": {"plugin":"local-md","config":{
            "root": sandbox.subdirectory("plans"),
            "status_mapping": {"Todo":"todo","Doing":"in-progress","Shipped":"done"}}},
        "board": {"plugin":"github-projects",
                  "config": github_projects_failing_a_field_write_once(&sandbox)}
    })));

    let said = refused(
        &sandbox,
        &["project", "copy", "plans:P-1", "--to", "board"],
        1,
    );
    assert!(
        said.contains("Something went wrong while executing your query"),
        "the failure GitHub reported is what the caller is told: {said}"
    );
    assert!(
        !said.contains("could not be undone"),
        "this board takes its items back, so the copy must not report otherwise: {said}"
    );

    // Nothing of that copy is on the board: not the project written first, and not the
    // task whose creation landed before the refusal.
    let projects = ok(&sandbox, &["project", "list", "--source", "board"]);
    assert!(
        !projects.contains("Published roadmap"),
        "the destination holds none of that copy's items:\n{projects}"
    );
    let tasks = ok(&sandbox, &["task", "list", "--source", "board"]);
    for title in ["First step", "Second step"] {
        assert!(
            !tasks.contains(title),
            "the destination holds none of that copy's items:\n{tasks}"
        );
    }

    // And the retry is a clean one: the failure is spent, so the same copy now completes.
    let again = ok(
        &sandbox,
        &["project", "copy", "plans:P-1", "--to", "board", "--json"],
    );
    assert_eq!(
        reported(&again)
            .iter()
            .map(|(source, _, action)| (source.as_str(), action.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("plans:P-1", "created"),
            ("plans:A", "created"),
            ("plans:B", "created"),
        ]
    );
}

#[test]
fn a_copy_into_linear_that_cannot_finish_leaves_the_workspace_as_it_found_it() {
    // The same rule as the board above, against the other hosted destination this
    // repository ships: a copy is either complete or it never happened. Linear could not
    // take a project back until `projectDelete` was pinned, so a project copy that failed
    // part way left the project it had created behind — which is the state this asserts
    // is gone.
    //
    // The workspace here fails the relation between two issues it has already created, so
    // the refusal arrives with the project and both of its tasks landed: every kind of
    // item this copy can create is there to be taken back.
    let sandbox = Sandbox::new();
    plans_with_a_dependency(&sandbox);
    sandbox.project_document(&document(&json!({
        "plans": {"plugin":"local-md","config":{
            "root": sandbox.subdirectory("plans"),
            "status_mapping": {"Todo":"todo","Doing":"in-progress","Shipped":"done"}}},
        "work": {"plugin":"linear","config":linear_failing_a_relation_write_once(&sandbox)}
    })));

    let said = refused(
        &sandbox,
        &["project", "copy", "plans:P-1", "--to", "work"],
        1,
    );
    assert!(
        said.contains(LINEAR_REFUSED_WRITE),
        "the failure Linear reported is what the caller is told: {said}"
    );
    assert!(
        !said.contains("could not be undone"),
        "this workspace takes its items back, so the copy must not report otherwise: {said}"
    );

    // Nothing of that copy is in the workspace: not the project written first, and not
    // either task whose creation landed before the refusal.
    let projects = ok(&sandbox, &["project", "list", "--source", "work"]);
    assert!(
        !projects.contains("Published roadmap"),
        "the destination holds none of that copy's items:\n{projects}"
    );
    let tasks = ok(&sandbox, &["task", "list", "--source", "work"]);
    for title in ["First step", "Second step"] {
        assert!(
            !tasks.contains(title),
            "the destination holds none of that copy's items:\n{tasks}"
        );
    }

    // And the retry is a clean one: the failure is spent, so the same copy now completes.
    let again = ok(
        &sandbox,
        &["project", "copy", "plans:P-1", "--to", "work", "--json"],
    );
    assert_eq!(
        reported(&again)
            .iter()
            .map(|(source, _, action)| (source.as_str(), action.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("plans:P-1", "created"),
            ("plans:A", "created"),
            ("plans:B", "created"),
        ]
    );
}

#[test]
fn a_cleanup_that_also_fails_reports_the_write_that_failed_rather_than_the_tidy_up() {
    // Filing an item on a board is several calls, so a failure after the first leaves an
    // issue this run created and nobody asked for — which the source takes back. GitHub can
    // refuse that removal too, and what the caller is owed then is *why the copy stopped*.
    // The tidy-up's own failure is about an item they never asked to exist; reporting it
    // instead would leave them reading a deletion error for a copy they made.
    //
    // This board refuses the field write and then refuses the removal, so both failures are
    // real and the copy has to choose which one to say.
    let sandbox = Sandbox::new();
    let root = sandbox.subdirectory("plans");
    std::fs::create_dir_all(root.join("tasks")).unwrap();
    std::fs::write(
        root.join("tasks/A.md"),
        "---\ntitle: First step\nstatus: Doing\n---\ndo this first\n",
    )
    .unwrap();
    sandbox.project_document(&document(&json!({
        "plans": {"plugin":"local-md","config":{
            "root": root, "status_mapping": {"Doing":"in-progress"}}},
        "board": {"plugin":"github-projects",
                  "config": github_projects_failing_a_field_write_and_its_cleanup(&sandbox)}
    })));

    let said = refused(&sandbox, &["task", "copy", "plans:A", "--to", "board"], 1);
    assert!(
        said.contains("updateProjectV2ItemFieldValue"),
        "the failure that stopped the copy is what the caller is told: {said}"
    );
    assert!(
        !said.contains("deleteIssue"),
        "and not the failure of the tidy-up that followed it: {said}"
    );

    // The item the removal could not take back really is still there — the refusal above is
    // the whole of what the caller gets, so this is the state they are left in.
    let listed = ok(&sandbox, &["task", "list", "--source", "board"]);
    assert_eq!(
        listed.matches("First step").count(),
        1,
        "the removal failed, so the item it could not take back is on the board: {listed}"
    );

    // Both failures are spent, so the copy the caller runs next matches that item by title
    // rather than filing a second one for the same plan.
    let again = ok(
        &sandbox,
        &[
            "task",
            "copy",
            "plans:A",
            "--to",
            "board",
            "--match-by",
            "title",
            "--json",
        ],
    );
    let landed = reported(&again);
    assert_eq!(landed.len(), 1, "{again}");
    assert_eq!(landed[0].2, "updated", "{again}");
    let listed = ok(&sandbox, &["task", "list", "--source", "board"]);
    assert_eq!(
        listed.matches("First step").count(),
        1,
        "and one plan is one issue rather than two: {listed}"
    );
}

#[test]
fn a_creation_whose_filing_and_cleanup_both_fail_still_reports_the_filing() {
    // The earlier half of the same sequence: `createIssue` lands, filing it on the board
    // does not, and the removal that would take the created issue back is refused as well.
    // The caller is told why the copy stopped — an issue they never asked to exist failing
    // to be removed is not something they can act on, and reporting it would replace the
    // reason with a consequence.
    let sandbox = Sandbox::new();
    let root = sandbox.subdirectory("plans");
    std::fs::create_dir_all(root.join("tasks")).unwrap();
    std::fs::write(
        root.join("tasks/A.md"),
        "---\ntitle: First step\nstatus: Doing\n---\ndo this first\n",
    )
    .unwrap();
    sandbox.project_document(&document(&json!({
        "plans": {"plugin":"local-md","config":{
            "root": root, "status_mapping": {"Doing":"in-progress"}}},
        "board": {"plugin":"github-projects",
                  "config": github_projects_failing_to_file_and_its_cleanup(&sandbox)}
    })));

    let said = refused(&sandbox, &["task", "copy", "plans:A", "--to", "board"], 1);
    assert!(
        said.contains("addProjectV2ItemById"),
        "the failure that stopped the copy is what the caller is told: {said}"
    );
    assert!(
        !said.contains("deleteIssue"),
        "and not the failure of the tidy-up that followed it: {said}"
    );

    // The issue never reached the board, so nothing there answers for this plan — and the
    // retry, with both failures spent, files exactly one.
    let listed = ok(&sandbox, &["task", "list", "--source", "board"]);
    assert_eq!(listed.matches("First step").count(), 0, "{listed}");
    let again = ok(
        &sandbox,
        &["task", "copy", "plans:A", "--to", "board", "--json"],
    );
    assert_eq!(reported(&again)[0].2, "created", "{again}");
    let listed = ok(&sandbox, &["task", "list", "--source", "board"]);
    assert_eq!(listed.matches("First step").count(), 1, "{listed}");
}

/// The document-bearing destination the document copy journeys copy into.
const STORE: &str = "store";

/// Every row whose source holds documents; the rest assert the refusal below.
///
/// The copy *round trip* out of these rows — created, read back, and matched rather than
/// duplicated on a second copy — needs a destination that outlives one invocation, so it
/// lives in `document_store.rs` beside the peer that provides one. What stays here is the
/// pair of refusals, which write nothing and so need no destination to read back.
fn documentary_rows() -> impl Iterator<Item = &'static crate::fixtures::Row> {
    ROWS.iter()
        .filter(|row| row.declared().documents.is_native())
}

#[test]
fn a_document_copy_into_a_source_that_has_none_is_refused_naming_it_and_its_plugin() {
    // Refused from the declaration rather than from a failed write, so nothing is read
    // first — the same shape the write-support refusal already has.
    for row in documentary_rows() {
        let sandbox = Sandbox::new();
        sandbox.project_document(&row.document_with_folder(&sandbox, NOTES));

        let complaint = refused(
            &sandbox,
            &["document", "copy", &qualified(SOURCE, "D-1"), "--to", NOTES],
            1,
        );
        assert!(
            complaint.contains(NOTES) && complaint.contains("local-md"),
            "{}: the refusal names the source and its plugin:\n{complaint}",
            row.name
        );
        assert!(
            complaint.contains("has no documents"),
            "{}: and says what is wrong with it:\n{complaint}",
            row.name
        );
    }
}

#[test]
fn a_document_copy_out_of_a_source_that_has_none_is_refused_naming_it_and_its_plugin() {
    for row in ROWS
        .iter()
        .filter(|row| !row.declared().documents.is_native())
    {
        let sandbox = Sandbox::new();
        sandbox.project_document(&row.document_with_store(&sandbox, STORE));

        let complaint = refused(
            &sandbox,
            &["document", "copy", &qualified(SOURCE, "D-1"), "--to", STORE],
            1,
        );
        assert!(
            complaint.contains(SOURCE) && complaint.contains(row.plugin),
            "{}: the refusal names the source and its plugin:\n{complaint}",
            row.name
        );
        assert!(
            complaint.contains("has no documents"),
            "{}: and says what is wrong with it:\n{complaint}",
            row.name
        );
    }
}
