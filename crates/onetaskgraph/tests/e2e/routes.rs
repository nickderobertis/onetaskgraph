//! Routing, member projects, and reading a home with its members, driven the way a user
//! drives them.
//!
//! A source's `routes` send an item written to it somewhere else by the item's
//! repositories. Two shapes are proven: two folders of Markdown, one routing to the other,
//! for the general rule; and a GitHub board routing `github.com/petsinc/*` to a Linear
//! workspace, over the shared loopback servers, for the plan this exists for — a goal that
//! spans both orgs and lands as one home with a member project, not as two plans.
//!
//! Every test spawns the compiled binary and asserts on its exit code, stdout and stderr.

use std::path::Path;
use std::process::Output;

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{document, github_projects_with_board, linear_empty_workspace};

/// The folder a plan is authored in.
const PLAN: &str = "plan";
/// The source a plan is copied to, which routes.
const BOARD: &str = "plans";
/// Where it routes `github.com/petsinc/*`.
const LINEAR: &str = "hellopatient";

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
        "`onetaskgraph {}` exited {:?}\n{}{}",
        arguments.join(" "),
        output.status.code(),
        stdout(&output),
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

/// A JSON answer of a run that had to succeed.
fn answer(sandbox: &Sandbox, arguments: &[&str]) -> Value {
    let mut with_json = arguments.to_vec();
    with_json.push("--json");
    serde_json::from_str(&ok(sandbox, &with_json)).expect("the command emits JSON")
}

/// Write one Markdown record under `root`.
fn record(root: &Path, kind: &str, id: &str, front: &str) {
    let path = root.join(kind).join(format!("{id}.md"));
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("the folder");
    std::fs::write(&path, format!("---\n{front}\n---\nBody of {id}.\n")).expect("a record");
}

/// A plan spanning both orgs: two library tasks and an app task between them, each edge
/// crossing from one org to the other, plus the plan's design document.
fn mixed_plan(sandbox: &Sandbox) -> std::path::PathBuf {
    let root = sandbox.subdirectory(PLAN);
    record(&root, "projects", "goal", "title: One goal\nstatus: todo");
    record(
        &root,
        "tasks",
        "lib",
        "title: Library change\nstatus: todo\nproject: goal\n\
         repositories: [github.com/nickderobertis/lib]",
    );
    record(
        &root,
        "tasks",
        "app",
        "title: App consumes it\nstatus: todo\nproject: goal\n\
         repositories: [github.com/petsinc/app]\ndepends_on: [lib]",
    );
    record(
        &root,
        "tasks",
        "docs",
        "title: Library documents it\nstatus: todo\nproject: goal\n\
         repositories: [github.com/nickderobertis/lib]\ndepends_on: [app]",
    );
    record(
        &root,
        "documents",
        "design",
        "title: The design\nproject: goal",
    );
    root
}

/// An all-petsinc plan with its design document.
fn petsinc_plan(root: &Path) {
    record(root, "projects", "pets", "title: Pets only\nstatus: todo");
    record(
        root,
        "tasks",
        "pets-a",
        "title: Pets A\nstatus: todo\nproject: pets\nrepositories: [github.com/petsinc/api]",
    );
    record(
        root,
        "tasks",
        "pets-b",
        "title: Pets B\nstatus: todo\nproject: pets\n\
         repositories: [github.com/petsinc/web]\ndepends_on: [pets-a]",
    );
    record(
        root,
        "documents",
        "pets-design",
        "title: Pets design\nproject: pets",
    );
}

/// The routing board, the Linear workspace it routes to, and the folder a plan is in.
fn board_and_linear(sandbox: &Sandbox) {
    let (board, _) = github_projects_with_board(sandbox);
    let linear = linear_empty_workspace(sandbox);
    // Each fixture writes its own credential into the one secrets file, the second over the
    // first, so the two are written together here.
    sandbox.secrets_file("GITHUB_PROJECTS_FIXTURE_TOKEN=test-token\nLINEAR_API_KEY=fixture-key\n");
    sandbox.project_document(&document(&json!({
        PLAN: {"plugin": "local-md", "config": {"root": sandbox.project().join(PLAN)}},
        BOARD: {
            "plugin": "github-projects",
            "config": board,
            "routes": [{"repositories": ["github.com/petsinc/*"], "to": LINEAR}],
        },
        LINEAR: {"plugin": "linear", "config": linear},
    })));
}

/// The outcome a copy report gives one source item: where it landed, what happened, and
/// the placement it names.
fn outcome<'a>(report: &'a Value, source: &str) -> &'a Value {
    report["items"]
        .as_array()
        .expect("a copy report carries items")
        .iter()
        .find(|item| item["source"] == source)
        .unwrap_or_else(|| panic!("the report names {source}: {report:#}"))
}

/// The qualified id an outcome landed on.
fn landed(report: &Value, source: &str) -> String {
    outcome(report, source)["destination"]
        .as_str()
        .unwrap_or_else(|| panic!("{source} landed somewhere: {report:#}"))
        .to_owned()
}

/// The source half of a qualified id.
fn source_of(id: &str) -> &str {
    id.split_once(':').expect("a qualified id").0
}

/// The forward edges of a task, as qualified far ends.
fn depends_on(sandbox: &Sandbox, task: &str) -> Vec<String> {
    let edges = answer(sandbox, &["task", "deps", task]);
    edges["items"]
        .as_array()
        .expect("edges")
        .iter()
        .map(|edge| edge["to"]["id"].as_str().expect("a far end").to_owned())
        .collect()
}

#[test]
fn a_mixed_plan_lands_its_home_on_the_board_and_its_petsinc_tasks_in_a_linear_member() {
    let sandbox = Sandbox::new();
    mixed_plan(&sandbox);
    board_and_linear(&sandbox);

    let report = answer(
        &sandbox,
        &["project", "copy", &format!("{PLAN}:goal"), "--to", BOARD],
    );
    let home = landed(&report, "plan:goal");
    assert_eq!(source_of(&home), BOARD, "the home stays on the board");
    assert_eq!(
        outcome(&report, "plan:goal")["placed"],
        json!({"destination": BOARD, "route": null}),
        "and the report says no route placed it"
    );
    for (task, to, route) in [
        ("plan:lib", BOARD, Value::Null),
        ("plan:docs", BOARD, Value::Null),
        ("plan:app", LINEAR, json!(0)),
    ] {
        let id = landed(&report, task);
        assert_eq!(source_of(&id), to, "{task} lands in {to}: {report:#}");
        assert_eq!(
            outcome(&report, task)["placed"],
            json!({"destination": to, "route": route}),
            "{task}'s outcome names its placement"
        );
    }

    // The home and its member name each other.
    let held = answer(&sandbox, &["project", "show", &home]);
    let members = held["items"][0]["item"]["metadata"]["onetaskgraph.members"].clone();
    let members = members.as_array().expect("the home names its members");
    assert_eq!(members.len(), 1, "one member, in Linear: {held:#}");
    let member = members[0].as_str().expect("a qualified id").to_owned();
    assert_eq!(source_of(&member), LINEAR);
    let shown = answer(&sandbox, &["project", "show", &member]);
    assert_eq!(
        shown["items"][0]["item"]["metadata"]["onetaskgraph.member_of"],
        json!(home),
        "the member names its home"
    );
    assert_eq!(shown["items"][0]["item"]["title"], "One goal");
    let human = ok(&sandbox, &["project", "show", &home]);
    assert!(
        human.contains(&member),
        "`project show` of a home prints its members:\n{human}"
    );

    // The petsinc task is filed under the member, and the edges cross both ways.
    let app = landed(&report, "plan:app");
    let lib = landed(&report, "plan:lib");
    let docs = landed(&report, "plan:docs");
    let app_held = answer(&sandbox, &["task", "show", &app]);
    assert_eq!(
        format!(
            "{LINEAR}:{}",
            app_held["items"][0]["item"]["project"].as_str().unwrap()
        ),
        member,
        "the routed task is filed under the member project"
    );
    assert_eq!(
        depends_on(&sandbox, &app),
        vec![lib.clone()],
        "Linear records its edge to the board"
    );
    assert_eq!(
        depends_on(&sandbox, &docs),
        vec![app.clone()],
        "the board records its edge to Linear"
    );

    // The plan reads as one: the home's tasks and its member's.
    let listed = answer(&sandbox, &["task", "list", "--project", &home, "--members"]);
    let mut ids: Vec<String> = listed["items"]
        .as_array()
        .expect("tasks")
        .iter()
        .map(|task| task["id"].as_str().expect("an id").to_owned())
        .collect();
    ids.sort();
    let mut wanted = vec![app.clone(), lib.clone(), docs.clone()];
    wanted.sort();
    assert_eq!(ids, wanted, "the home and its member read together");

    // The document goes with the home.
    let copied = answer(
        &sandbox,
        &["document", "copy", &format!("{PLAN}:design"), "--to", BOARD],
    );
    let design = landed(&copied, "plan:design");
    assert_eq!(source_of(&design), BOARD, "{copied:#}");

    // And a re-copy finds every counterpart where it routes, creating nothing.
    let again = answer(
        &sandbox,
        &["project", "copy", &format!("{PLAN}:goal"), "--to", BOARD],
    );
    for item in again["items"].as_array().expect("items") {
        assert_ne!(
            item["action"], "created",
            "nothing is created again: {again:#}"
        );
    }
    assert_eq!(landed(&again, "plan:app"), app);
    assert_eq!(landed(&again, "plan:goal"), home);
}

#[test]
fn an_all_petsinc_plan_lands_wholly_in_linear_with_its_document() {
    let sandbox = Sandbox::new();
    let root = sandbox.subdirectory(PLAN);
    petsinc_plan(&root);
    board_and_linear(&sandbox);

    let report = answer(
        &sandbox,
        &["project", "copy", &format!("{PLAN}:pets"), "--to", BOARD],
    );
    for item in ["plan:pets", "plan:pets-a", "plan:pets-b"] {
        assert_eq!(source_of(&landed(&report, item)), LINEAR, "{report:#}");
        assert_eq!(
            outcome(&report, item)["placed"],
            json!({"destination": LINEAR, "route": 0})
        );
    }
    let home = landed(&report, "plan:pets");
    let held = answer(&sandbox, &["project", "show", &home]);
    assert!(
        held["items"][0]["item"]["metadata"]
            .get("onetaskgraph.members")
            .is_none(),
        "a home with nothing routed elsewhere has no member: {held:#}"
    );
    let copied = answer(
        &sandbox,
        &[
            "document",
            "copy",
            &format!("{PLAN}:pets-design"),
            "--to",
            BOARD,
        ],
    );
    assert_eq!(
        source_of(&landed(&copied, "plan:pets-design")),
        LINEAR,
        "the document goes with its home: {copied:#}"
    );
    assert_eq!(
        outcome(&copied, "plan:pets-design")["placed"],
        json!({"destination": LINEAR, "route": 0})
    );
}

#[test]
fn sources_route_answers_from_configuration_alone_and_refuses_by_name() {
    let sandbox = Sandbox::new();
    sandbox.project_document(&document(&json!({
        // A board nothing listens on: `sources route` never asks a source, so it answers.
        BOARD: {
            "plugin": "local-md",
            "config": {"root": sandbox.subdirectory("board")},
            "routes": [
                {"repositories": ["github.com/petsinc/*"], "to": LINEAR},
                {"repositories": ["github.com/nickderobertis/*"], "to": LINEAR},
            ],
        },
        LINEAR: {"plugin": "local-md", "config": {"root": sandbox.subdirectory("linear")}},
    })));
    assert_eq!(
        answer(
            &sandbox,
            &[
                "sources",
                "route",
                BOARD,
                "--repository",
                "github.com/petsinc/api"
            ]
        ),
        json!({"source": BOARD, "destination": LINEAR, "route": 0})
    );
    assert_eq!(
        answer(
            &sandbox,
            &[
                "sources",
                "route",
                BOARD,
                "--repository",
                "github.com/nickderobertis/lib"
            ]
        ),
        json!({"source": BOARD, "destination": LINEAR, "route": 1}),
        "the first entry that matches wins"
    );
    assert_eq!(
        answer(
            &sandbox,
            &[
                "sources",
                "route",
                BOARD,
                "--repository",
                "github.com/petsinc/api",
                "--repository",
                "gitlab.com/other/x"
            ]
        ),
        json!({"source": BOARD, "destination": BOARD, "route": null}),
        "an entry matches only when every repository does"
    );
    assert_eq!(
        answer(&sandbox, &["sources", "route", BOARD]),
        json!({"source": BOARD, "destination": BOARD, "route": null}),
        "an item with no repositories stays"
    );
    let human = ok(&sandbox, &["sources", "route", BOARD]);
    assert!(
        human.contains("route:") && human.contains("none"),
        "{human}"
    );

    let unknown = refused(&sandbox, &["sources", "route", "nowhere"], 1);
    assert!(
        unknown.contains("\"nowhere\"") && unknown.contains("next:"),
        "an unknown source is refused by name:\n{unknown}"
    );
    let malformed = refused(
        &sandbox,
        &["sources", "route", BOARD, "--repository", "petsinc/api"],
        2,
    );
    assert!(
        malformed.contains("petsinc/api"),
        "a malformed origin is refused by name:\n{malformed}"
    );
}
