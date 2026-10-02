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
use crate::fixtures::{
    document, github_projects_with_board, linear_empty_workspace,
    linear_failing_a_relation_write_once,
};

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
    let linear = linear_empty_workspace(sandbox);
    board_and(sandbox, linear);
}

/// The routing board, the Linear workspace `linear` configures, and the plan's folder.
fn board_and(sandbox: &Sandbox, linear: Value) {
    let (board, _) = github_projects_with_board(sandbox);
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
        // A folder whose root does not exist: `sources route` never builds or asks a source,
        // so it answers anyway. The third entry overlaps both before it.
        BOARD: {
            "plugin": "local-md",
            "config": {"root": sandbox.project().join("never-created")},
            "routes": [
                {"repositories": ["github.com/petsinc/*"], "to": LINEAR},
                {"repositories": ["github.com/nickderobertis/*"], "to": LINEAR},
                {"repositories": ["github.com/*/*"], "to": LINEAR},
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
                "github.com/someone/x"
            ]
        ),
        json!({"source": BOARD, "destination": LINEAR, "route": 2}),
        "the overlapping entry answers only what the earlier ones do not"
    );
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

/// The folder a routed write is sent to by name.
const NOTES: &str = "notes";
/// The folder `notes` routes `github.com/petsinc/*` to.
const TEAM: &str = "team";

/// Three folders of Markdown: a plan, `notes` routing petsinc work to `team`, and `team`.
fn folders(sandbox: &Sandbox) -> std::path::PathBuf {
    let plan = sandbox.subdirectory(PLAN);
    sandbox.project_document(&document(&json!({
        PLAN: {"plugin": "local-md", "config": {"root": plan}},
        NOTES: {
            "plugin": "local-md",
            "config": {"root": sandbox.subdirectory(NOTES)},
            "routes": [{"repositories": ["github.com/petsinc/*"], "to": TEAM}],
        },
        TEAM: {"plugin": "local-md", "config": {"root": sandbox.subdirectory(TEAM)}},
    })));
    plan
}

/// Every file under `root`, by path, with its bytes: what "unchanged" is compared against.
fn tree(root: &Path) -> std::collections::BTreeMap<String, Vec<u8>> {
    let mut held = std::collections::BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries {
            let path = entry.expect("a directory entry").path();
            if path.is_dir() {
                pending.push(path);
            } else {
                held.insert(
                    path.display().to_string(),
                    std::fs::read(&path).expect("a readable file"),
                );
            }
        }
    }
    held
}

/// The member a home names, read back through the binary.
fn members_of(sandbox: &Sandbox, home: &str) -> Vec<String> {
    let held = answer(sandbox, &["project", "show", home]);
    held["items"][0]["item"]["metadata"]["onetaskgraph.members"]
        .as_array()
        .map(|members| {
            members
                .iter()
                .map(|member| member.as_str().expect("a qualified id").to_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// The project a task is filed under, qualified by the task's own source.
fn filed_under(sandbox: &Sandbox, task: &str) -> String {
    let held = answer(sandbox, &["task", "show", task]);
    format!(
        "{}:{}",
        source_of(task),
        held["items"][0]["item"]["project"]
            .as_str()
            .unwrap_or_else(|| panic!("{task} is filed: {held:#}"))
    )
}

#[test]
fn task_create_and_task_copy_land_a_petsinc_task_in_the_routed_source_and_any_other_in_the_named_one()
 {
    let sandbox = Sandbox::new();
    let plan = folders(&sandbox);
    record(&plan, "projects", "goal", "title: Goal\nstatus: todo");
    record(
        &plan,
        "tasks",
        "lib",
        "title: Lib\nstatus: todo\nproject: goal\nrepositories: [github.com/nickderobertis/lib]",
    );
    record(
        &plan,
        "tasks",
        "app",
        "title: App\nstatus: todo\nproject: goal\nrepositories: [github.com/petsinc/app]",
    );
    record(
        &plan,
        "tasks",
        "loose",
        "title: Loose\nstatus: todo\nrepositories: [github.com/nickderobertis/x]",
    );

    // A task copied on its own goes where its repositories route it.
    let copied = answer(
        &sandbox,
        &["task", "copy", "plan:app", "plan:loose", "--to", NOTES],
    );
    assert_eq!(source_of(&landed(&copied, "plan:app")), TEAM, "{copied:#}");
    assert_eq!(
        outcome(&copied, "plan:app")["placed"],
        json!({"destination": TEAM, "route": 0})
    );
    assert_eq!(source_of(&landed(&copied, "plan:loose")), NOTES);
    assert_eq!(
        outcome(&copied, "plan:loose")["placed"],
        json!({"destination": NOTES, "route": null})
    );
    let human = ok(&sandbox, &["task", "copy", "plan:app", "--to", NOTES]);
    assert!(
        human.contains("in team by route 0"),
        "the human report names the placement too:\n{human}"
    );

    // `task create` names the project in the source it names; routed away, the task is
    // filed under that project's member in the source it lands in, created on first need.
    let home = landed(
        &answer(
            &sandbox,
            &["project", "copy", "plan:goal", "--no-tasks", "--to", NOTES],
        ),
        "plan:goal",
    );
    assert_eq!(home, "notes:goal");
    let body = sandbox.subdirectory("bodies").join("body.md");
    std::fs::write(&body, "What to do.\n").expect("a body");
    let body = body.display().to_string();
    let created = answer(
        &sandbox,
        &[
            "task",
            "create",
            NOTES,
            "--project",
            "goal",
            "--title",
            "Pets work",
            "--repository",
            "github.com/petsinc/api",
            "--body-file",
            &body,
        ],
    );
    let pets = created["items"][0]["id"]
        .as_str()
        .expect("an id")
        .to_owned();
    assert_eq!(source_of(&pets), TEAM, "{created:#}");
    let members = members_of(&sandbox, &home);
    assert_eq!(members.len(), 1, "the home gained its member");
    assert_eq!(filed_under(&sandbox, &pets), members[0]);
    let created = answer(
        &sandbox,
        &[
            "task",
            "create",
            NOTES,
            "--project",
            "goal",
            "--title",
            "Own work",
            "--repository",
            "github.com/nickderobertis/lib",
            "--body-file",
            &body,
        ],
    );
    let own = created["items"][0]["id"]
        .as_str()
        .expect("an id")
        .to_owned();
    assert_eq!(source_of(&own), NOTES, "{created:#}");
    assert_eq!(filed_under(&sandbox, &own), home);
    // A second routed create reuses the member rather than adding one.
    answer(
        &sandbox,
        &[
            "task",
            "create",
            NOTES,
            "--project",
            "goal",
            "--title",
            "More pets work",
            "--repository",
            "github.com/petsinc/web",
            "--body-file",
            &body,
        ],
    );
    assert_eq!(members_of(&sandbox, &home), members);
}

#[test]
fn a_project_copy_places_its_home_by_its_tasks_or_by_its_own_repositories_without_them() {
    let sandbox = Sandbox::new();
    let plan = folders(&sandbox);
    // Its tasks are every one of them nickderobertis work, but the project itself names
    // petsinc: without its tasks, the project's own repositories alone decide.
    record(
        &plan,
        "projects",
        "pets-owned",
        "title: Pets owned\nstatus: todo\nrepositories: [github.com/petsinc/app]",
    );
    record(
        &plan,
        "tasks",
        "own",
        "title: Own\nstatus: todo\nproject: pets-owned\n\
         repositories: [github.com/nickderobertis/lib]",
    );
    record(&plan, "projects", "plain", "title: Plain\nstatus: todo");

    let alone = answer(
        &sandbox,
        &[
            "project",
            "copy",
            "plan:pets-owned",
            "--no-tasks",
            "--to",
            NOTES,
        ],
    );
    assert_eq!(landed(&alone, "plan:pets-owned"), "team:pets-owned");
    assert_eq!(
        outcome(&alone, "plan:pets-owned")["placed"],
        json!({"destination": TEAM, "route": 0})
    );
    assert_eq!(
        alone["items"].as_array().expect("items").len(),
        1,
        "no task travels"
    );
    let plain = answer(
        &sandbox,
        &["project", "copy", "plan:plain", "--no-tasks", "--to", NOTES],
    );
    assert_eq!(landed(&plain, "plan:plain"), "notes:plain");
    assert_eq!(
        outcome(&plain, "plan:plain")["placed"],
        json!({"destination": NOTES, "route": null})
    );

    // A copy into a source with no routes places nothing, and its report says nothing about
    // placement — exactly the document it was before routes existed.
    let whole = answer(
        &sandbox,
        &[
            "project",
            "copy",
            "plan:pets-owned",
            "--to",
            TEAM,
            "--dry-run",
        ],
    );
    assert!(
        outcome(&whole, "plan:pets-owned").get("placed").is_none(),
        "a copy into a source with no routes reports no placement, as before: {whole:#}"
    );
}

#[test]
fn a_member_copy_places_the_named_tasks_and_a_home_wholly_routed_keeps_its_place() {
    let sandbox = Sandbox::new();
    let plan = folders(&sandbox);
    record(&plan, "projects", "goal", "title: Goal\nstatus: todo");
    record(
        &plan,
        "tasks",
        "pets",
        "title: Pets\nstatus: todo\nproject: goal\nrepositories: [github.com/petsinc/api]",
    );
    record(
        &plan,
        "tasks",
        "own",
        "title: Own\nstatus: todo\nproject: goal\nrepositories: [github.com/nickderobertis/lib]",
    );

    // Only the petsinc task is named, so every task copied routes to `team`: the home
    // follows it there, wholly.
    let first = answer(
        &sandbox,
        &[
            "project",
            "copy",
            "plan:goal",
            "--member",
            "plan:pets",
            "--to",
            NOTES,
        ],
    );
    assert_eq!(landed(&first, "plan:goal"), "team:goal", "{first:#}");
    assert_eq!(
        outcome(&first, "plan:goal")["placed"],
        json!({"destination": TEAM, "route": 0})
    );
    assert_eq!(source_of(&landed(&first, "plan:pets")), TEAM);
    assert!(members_of(&sandbox, "team:goal").is_empty());

    // A later task that routes back to the named destination goes into a member project
    // there, and the home stays where it landed.
    let second = answer(
        &sandbox,
        &[
            "project",
            "copy",
            "plan:goal",
            "--member",
            "plan:own",
            "--to",
            NOTES,
        ],
    );
    assert_eq!(landed(&second, "plan:goal"), "team:goal", "{second:#}");
    let own = landed(&second, "plan:own");
    assert_eq!(source_of(&own), NOTES);
    let members = members_of(&sandbox, "team:goal");
    assert_eq!(members.len(), 1, "the home records its new member");
    assert_eq!(source_of(&members[0]), NOTES);
    assert_eq!(filed_under(&sandbox, &own), members[0]);
    let member = answer(&sandbox, &["project", "show", &members[0]]);
    assert_eq!(
        member["items"][0]["item"]["metadata"]["onetaskgraph.member_of"],
        json!("team:goal")
    );

    // A re-copy of the whole project finds every counterpart and creates nothing.
    let again = answer(&sandbox, &["project", "copy", "plan:goal", "--to", NOTES]);
    for item in again["items"].as_array().expect("items") {
        assert_ne!(item["action"], "created", "{again:#}");
    }
    assert_eq!(members_of(&sandbox, "team:goal"), members);
}

#[test]
fn a_project_whose_last_unrouted_task_is_gone_lands_wholly_in_the_routed_source() {
    let sandbox = Sandbox::new();
    let plan = folders(&sandbox);
    record(&plan, "projects", "goal", "title: Goal\nstatus: todo");
    record(
        &plan,
        "tasks",
        "pets",
        "title: Pets\nstatus: todo\nproject: goal\nrepositories: [github.com/petsinc/api]",
    );
    record(
        &plan,
        "tasks",
        "own",
        "title: Own\nstatus: todo\nproject: goal\nrepositories: [github.com/nickderobertis/lib]",
    );
    std::fs::remove_file(plan.join("tasks/own.md")).expect("the unrouted task is removed");
    let report = answer(&sandbox, &["project", "copy", "plan:goal", "--to", NOTES]);
    assert_eq!(landed(&report, "plan:goal"), "team:goal", "{report:#}");
    assert_eq!(source_of(&landed(&report, "plan:pets")), TEAM);
    assert!(
        tree(&sandbox.project().join(NOTES)).is_empty(),
        "nothing lands in the named destination"
    );
}

#[test]
fn a_dry_run_reports_the_placement_the_copy_then_makes_and_writes_nothing() {
    let sandbox = Sandbox::new();
    let plan = folders(&sandbox);
    record(&plan, "projects", "goal", "title: Goal\nstatus: todo");
    record(
        &plan,
        "tasks",
        "pets",
        "title: Pets\nstatus: todo\nproject: goal\nrepositories: [github.com/petsinc/api]",
    );
    record(
        &plan,
        "tasks",
        "own",
        "title: Own\nstatus: todo\nproject: goal\n\
         repositories: [github.com/nickderobertis/lib]\ndepends_on: [pets]",
    );
    let before: Vec<_> = [PLAN, NOTES, TEAM]
        .iter()
        .map(|folder| tree(&sandbox.project().join(folder)))
        .collect();
    let dry = answer(
        &sandbox,
        &["project", "copy", "plan:goal", "--to", NOTES, "--dry-run"],
    );
    let after: Vec<_> = [PLAN, NOTES, TEAM]
        .iter()
        .map(|folder| tree(&sandbox.project().join(folder)))
        .collect();
    assert_eq!(before, after, "a dry run leaves every source as it was");

    let real = answer(&sandbox, &["project", "copy", "plan:goal", "--to", NOTES]);
    for item in ["plan:goal", "plan:pets", "plan:own"] {
        assert_eq!(
            outcome(&dry, item)["placed"],
            outcome(&real, item)["placed"],
            "{item}: the dry run named the placement the copy made"
        );
        assert_eq!(
            outcome(&real, item)["placed"]["destination"].as_str(),
            Some(source_of(&landed(&real, item))),
        );
    }
    assert_eq!(
        outcome(&real, "plan:pets")["placed"],
        json!({"destination": TEAM, "route": 0})
    );
}

#[test]
fn a_routed_copy_refuses_an_item_whose_counterpart_sits_where_it_no_longer_routes() {
    let sandbox = Sandbox::new();
    let plan = folders(&sandbox);
    record(
        &plan,
        "tasks",
        "moving",
        "title: Moving\nstatus: todo\nrepositories: [github.com/petsinc/api]",
    );
    let first = answer(&sandbox, &["task", "copy", "plan:moving", "--to", NOTES]);
    let counterpart = landed(&first, "plan:moving");
    assert_eq!(source_of(&counterpart), TEAM);

    // Somebody moves the work to another repository after it was copied.
    let path = plan.join("tasks/moving.md");
    let text = std::fs::read_to_string(&path).expect("the task");
    std::fs::write(
        &path,
        text.replace("github.com/petsinc/api", "github.com/nickderobertis/api"),
    )
    .expect("the task is edited");
    let before = (
        tree(&sandbox.project().join(NOTES)),
        tree(&sandbox.project().join(TEAM)),
    );
    let refusal = refused(&sandbox, &["task", "copy", "plan:moving", "--to", NOTES], 1);
    assert!(
        refusal.contains("plan:moving")
            && refusal.contains(&counterpart)
            && refusal.contains(NOTES)
            && refusal.contains("next:"),
        "the refusal names the item, its counterpart and where it routes now:\n{refusal}"
    );
    assert_eq!(
        before,
        (
            tree(&sandbox.project().join(NOTES)),
            tree(&sandbox.project().join(TEAM)),
        ),
        "nothing is written before the refusal"
    );

    // And the member keys are the store's: a caller cannot set either.
    for key in ["onetaskgraph.members", "onetaskgraph.member_of"] {
        let refusal = refused(
            &sandbox,
            &["task", "metadata", "set", &counterpart, key, "[]"],
            1,
        );
        assert!(
            refusal.contains(key),
            "{key} is refused by name:\n{refusal}"
        );
    }
}

#[test]
fn a_home_reads_with_its_members_across_pages_and_reports_a_member_it_cannot_read() {
    let sandbox = Sandbox::new();
    let plan = folders(&sandbox);
    record(&plan, "projects", "goal", "title: Goal\nstatus: todo");
    for (id, repository) in [
        ("a", "github.com/nickderobertis/a"),
        ("b", "github.com/petsinc/b"),
        ("c", "github.com/nickderobertis/c"),
        ("d", "github.com/petsinc/d"),
        ("e", "github.com/petsinc/e"),
    ] {
        record(
            &plan,
            "tasks",
            id,
            &format!("title: Task {id}\nstatus: todo\nproject: goal\nrepositories: [{repository}]"),
        );
    }
    let report = answer(&sandbox, &["project", "copy", "plan:goal", "--to", NOTES]);
    let home = landed(&report, "plan:goal");
    let mut wanted: Vec<String> = ["a", "b", "c", "d", "e"]
        .iter()
        .map(|id| landed(&report, &format!("plan:{id}")))
        .collect();
    wanted.sort();

    // Two at a time, to exhaustion: every task once, each under its own source.
    let mut seen: Vec<String> = Vec::new();
    let mut token: Option<String> = None;
    loop {
        let mut arguments = vec![
            "task",
            "list",
            "--project",
            home.as_str(),
            "--members",
            "--limit",
            "2",
        ];
        if let Some(token) = &token {
            arguments.extend(["--page", token.as_str()]);
        }
        let page = answer(&sandbox, &arguments);
        let items = page["items"].as_array().expect("items");
        assert!(items.len() <= 2, "a page holds at most the limit");
        seen.extend(
            items
                .iter()
                .map(|task| task["id"].as_str().expect("an id").to_owned()),
        );
        match page["next"].as_str() {
            Some(next) => token = Some(next.to_owned()),
            None => break,
        }
    }
    seen.sort();
    assert_eq!(seen, wanted, "no task dropped or repeated across pages");

    // Without --members only the home's own.
    let own = answer(&sandbox, &["task", "list", "--project", &home]);
    assert!(
        own["items"]
            .as_array()
            .expect("items")
            .iter()
            .all(|task| source_of(task["id"].as_str().expect("an id")) == NOTES),
        "{own:#}"
    );

    // A member that cannot be read is an error, never a silent omission.
    let member = members_of(&sandbox, &home)[0].clone();
    let file = sandbox.project().join(TEAM).join("projects").join(format!(
        "{}.md",
        member.split_once(':').expect("qualified").1
    ));
    std::fs::remove_file(&file).expect("the member project is removed");
    let output = run(
        &sandbox,
        &["task", "list", "--project", &home, "--members", "--json"],
    );
    assert_eq!(output.status.code(), Some(4), "{}", stderr(&output));
    let partial: Value = serde_json::from_str(&stdout(&output)).expect("JSON");
    let errors = partial["errors"].as_array().expect("errors");
    assert!(
        errors
            .iter()
            .any(|error| error["source"] == TEAM && error.to_string().contains(&member)),
        "the unreadable member is reported: {partial:#}"
    );
    let refusal = refused(
        &sandbox,
        &["task", "list", "--project", "goal", "--members"],
        1,
    );
    assert!(refusal.contains("--members"), "{refusal}");
}

#[test]
fn a_member_copy_adding_a_petsinc_task_to_a_board_home_creates_its_linear_member_once() {
    let sandbox = Sandbox::new();
    let plan = sandbox.subdirectory(PLAN);
    record(&plan, "projects", "goal", "title: One goal\nstatus: todo");
    record(
        &plan,
        "tasks",
        "lib",
        "title: Library change\nstatus: todo\nproject: goal\n\
         repositories: [github.com/nickderobertis/lib]",
    );
    board_and_linear(&sandbox);
    let first = answer(&sandbox, &["project", "copy", "plan:goal", "--to", BOARD]);
    let home = landed(&first, "plan:goal");
    assert_eq!(source_of(&home), BOARD);
    assert!(members_of(&sandbox, &home).is_empty(), "no member yet");
    let lib = landed(&first, "plan:lib");

    // A running engine adds a Hello Patient task to the plan and writes it back.
    record(
        &plan,
        "tasks",
        "app",
        "title: App consumes it\nstatus: todo\nproject: goal\n\
         repositories: [github.com/petsinc/app]\ndepends_on: [lib]",
    );
    let added = answer(
        &sandbox,
        &[
            "project",
            "copy",
            "plan:goal",
            "--member",
            "plan:app",
            "--to",
            BOARD,
        ],
    );
    assert_eq!(
        landed(&added, "plan:goal"),
        home,
        "the home stays: {added:#}"
    );
    let app = landed(&added, "plan:app");
    assert_eq!(source_of(&app), LINEAR);
    assert_eq!(
        outcome(&added, "plan:app")["placed"],
        json!({"destination": LINEAR, "route": 0})
    );
    let members = members_of(&sandbox, &home);
    assert_eq!(members.len(), 1, "the member was created and recorded");
    assert_eq!(source_of(&members[0]), LINEAR);
    assert_eq!(filed_under(&sandbox, &app), members[0]);
    assert_eq!(depends_on(&sandbox, &app), vec![lib]);

    // A second one lands in the same member.
    record(
        &plan,
        "tasks",
        "app2",
        "title: App follows up\nstatus: todo\nproject: goal\n\
         repositories: [github.com/petsinc/app]",
    );
    let again = answer(
        &sandbox,
        &[
            "project",
            "copy",
            "plan:goal",
            "--member",
            "plan:app2",
            "--to",
            BOARD,
        ],
    );
    let app2 = landed(&again, "plan:app2");
    assert_eq!(filed_under(&sandbox, &app2), members[0]);
    assert_eq!(members_of(&sandbox, &home), members, "and no second member");
}

/// What a routed copy's undo has to put back, read through the binary: each record's
/// fields that a copy writes, and its forward edges.
fn state(sandbox: &Sandbox, projects: &[&str], tasks: &[&str]) -> Vec<Value> {
    let fields = |held: &Value| {
        let item = &held["items"][0]["item"];
        json!({
            "title": item["title"],
            "content": item["content"],
            "status": item["status"],
            "project": item["project"],
            "metadata": item["metadata"],
            "repositories": item["repositories"],
        })
    };
    let mut read = Vec::new();
    for project in projects {
        read.push(fields(&answer(sandbox, &["project", "show", project])));
    }
    for task in tasks {
        read.push(fields(&answer(sandbox, &["task", "show", task])));
        read.push(json!(depends_on(sandbox, task)));
    }
    read
}

#[test]
fn a_routed_copy_that_fails_in_its_second_source_leaves_both_as_it_found_them() {
    let sandbox = Sandbox::new();
    let plan = mixed_plan(&sandbox);
    // Linear refuses the first native relation it is asked for — an edge between two of
    // its own issues, which the first copy below never makes.
    let linear = linear_failing_a_relation_write_once(&sandbox);
    board_and(&sandbox, linear);
    let first = answer(&sandbox, &["project", "copy", "plan:goal", "--to", BOARD]);
    let home = landed(&first, "plan:goal");
    let member = members_of(&sandbox, &home)[0].clone();
    let (app, lib, docs) = (
        landed(&first, "plan:app"),
        landed(&first, "plan:lib"),
        landed(&first, "plan:docs"),
    );
    let tasks = [app.as_str(), lib.as_str(), docs.as_str()];
    let before = state(&sandbox, &[&home, &member], &tasks);
    let listed_before = answer(&sandbox, &["task", "list", "--project", &home, "--members"]);
    let plan_before = tree(&plan);

    // The next copy writes the board first — the home's title — then Linear. Two new petsinc
    // tasks land there, the first depending on the second; that edge is written once the
    // second has landed, as an update of the first, and Linear refuses it — after a new
    // board task has landed too.
    record(
        &plan,
        "projects",
        "goal",
        "title: One goal, renamed\nstatus: todo",
    );
    let path = plan.join("tasks/app.md");
    let text = std::fs::read_to_string(&path).expect("the task");
    std::fs::write(
        &path,
        text.replace("App consumes it", "App consumes it, renamed"),
    )
    .expect("an edit");
    record(
        &plan,
        "tasks",
        "app2",
        "title: App follows up\nstatus: todo\nproject: goal\n\
         repositories: [github.com/petsinc/app]\ndepends_on: [app3]",
    );
    record(
        &plan,
        "tasks",
        "app3",
        "title: App finishes\nstatus: todo\nproject: goal\n\
         repositories: [github.com/petsinc/app]",
    );
    record(
        &plan,
        "tasks",
        "lib2",
        "title: Library again\nstatus: todo\nproject: goal\n\
         repositories: [github.com/nickderobertis/lib]",
    );
    let edited_plan = tree(&plan);
    let refusal = refused(
        &sandbox,
        &["project", "copy", "plan:goal", "--to", BOARD],
        1,
    );
    assert!(
        refusal.contains(LINEAR) && !refusal.contains("could not be undone"),
        "the copy failed in Linear and was undone:\n{refusal}"
    );

    assert_eq!(
        state(&sandbox, &[&home, &member], &tasks),
        before,
        "every item the copy touched, in both sources, reads as it did — the home's members, \
         each counterpart's metadata and every edge included"
    );
    assert_eq!(
        answer(&sandbox, &["task", "list", "--project", &home, "--members"])["items"],
        listed_before["items"],
        "no item the copy created remains in either source"
    );
    // The plan's own files carry only the edits made to them: no copy link was written.
    let mut expected = edited_plan;
    for (path, bytes) in &plan_before {
        if !path.ends_with("goal.md") && !path.ends_with("app.md") {
            assert_eq!(
                expected.get(path),
                Some(bytes),
                "{path} keeps its link as it was"
            );
        }
    }
    expected.retain(|path, _| {
        path.ends_with("app2.md") || path.ends_with("app3.md") || path.ends_with("lib2.md")
    });
    for (path, bytes) in expected {
        assert!(
            !String::from_utf8_lossy(&bytes).contains("onetaskgraph.copies"),
            "{path} records no link to an item that was taken back"
        );
    }
}

#[test]
fn a_configuration_whose_routes_cannot_hold_is_refused_at_load_naming_the_source_and_entry() {
    for (routes, wanted) in [
        (
            json!([{"repositories": ["github.com/petsinc/*"], "to": "nowhere"}]),
            "nowhere",
        ),
        (
            json!([{"repositories": ["github.com/petsinc/*"], "to": NOTES}]),
            "itself",
        ),
        (
            json!([{"repositories": ["github.com/petsinc/*"], "to": "chained"}]),
            "routes of its own",
        ),
        (json!([{"repositories": [], "to": TEAM}]), "empty"),
        (
            json!([{"repositories": ["github.com/pets*/x"], "to": TEAM}]),
            "not a repository pattern",
        ),
    ] {
        let sandbox = Sandbox::new();
        sandbox.project_document(&document(&json!({
            NOTES: {
                "plugin": "local-md",
                "config": {"root": sandbox.subdirectory(NOTES)},
                "routes": routes,
            },
            TEAM: {"plugin": "local-md", "config": {"root": sandbox.subdirectory(TEAM)}},
            "chained": {
                "plugin": "local-md",
                "config": {"root": sandbox.subdirectory("chained")},
                "routes": [{"repositories": ["a.com/b/*"], "to": TEAM}],
            },
        })));
        // Any verb, because the configuration is refused before any verb runs.
        let refusal = refused(&sandbox, &["sources", "list"], 1);
        assert!(
            refusal.contains("sources.notes.routes.0") && refusal.contains(wanted),
            "{routes}: refused naming the source and the entry:\n{refusal}"
        );
        assert!(refusal.contains("next:"), "{refusal}");
    }
}

#[test]
fn routes_set_by_flags_alone_route_a_lookup_and_a_copy_and_config_show_names_their_layer() {
    let sandbox = Sandbox::new();
    let plan = sandbox.subdirectory(PLAN);
    // No `routes` in any document.
    sandbox.project_document(&document(&json!({
        PLAN: {"plugin": "local-md", "config": {"root": plan}},
        NOTES: {"plugin": "local-md", "config": {"root": sandbox.subdirectory(NOTES)}},
        TEAM: {"plugin": "local-md", "config": {"root": sandbox.subdirectory(TEAM)}},
    })));
    record(
        &plan,
        "tasks",
        "pets",
        "title: Pets\nstatus: todo\nrepositories: [github.com/petsinc/api]",
    );
    let flags = [
        "--set",
        "sources.notes.routes.0.repositories=github.com/petsinc/*",
        "--set",
        "sources.notes.routes.0.to=team",
    ];
    let with = |verb: &[&str]| {
        let mut arguments: Vec<&str> = flags.to_vec();
        arguments.extend(verb);
        arguments.push("--json");
        let output = ok(&sandbox, &arguments);
        serde_json::from_str::<Value>(&output).expect("JSON")
    };
    assert_eq!(
        with(&[
            "sources",
            "route",
            NOTES,
            "--repository",
            "github.com/petsinc/api"
        ]),
        json!({"source": NOTES, "destination": TEAM, "route": 0})
    );
    let copied = with(&["task", "copy", "plan:pets", "--to", NOTES]);
    assert_eq!(source_of(&landed(&copied, "plan:pets")), TEAM, "{copied:#}");
    let shown = with(&["config", "show"]);
    let settings = shown["settings"].as_array().expect("settings");
    for (key, value) in [
        (
            "sources.notes.routes.0.repositories",
            json!("github.com/petsinc/*"),
        ),
        ("sources.notes.routes.0.to", json!("team")),
    ] {
        let setting = settings
            .iter()
            .find(|setting| setting["key"] == key)
            .unwrap_or_else(|| panic!("config show names {key}: {shown:#}"));
        assert_eq!(setting["value"], value);
        assert_eq!(setting["origin"]["layer"], "flag", "{setting}");
    }

    // The environment layer spells the same entry the same way.
    let output = sandbox
        .command()
        .env(
            "ONETASKGRAPH_SOURCES__NOTES__ROUTES__0__REPOSITORIES",
            "github.com/petsinc/*",
        )
        .env("ONETASKGRAPH_SOURCES__NOTES__ROUTES__0__TO", TEAM)
        .args([
            "sources",
            "route",
            NOTES,
            "--repository",
            "github.com/petsinc/api",
            "--json",
        ])
        .assert()
        .get_output()
        .clone();
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(
        serde_json::from_str::<Value>(&stdout(&output)).expect("JSON"),
        json!({"source": NOTES, "destination": TEAM, "route": 0})
    );

    // And a document's routes are shown whole, attributed to the document.
    let sandbox = Sandbox::new();
    folders(&sandbox);
    let human = ok(&sandbox, &["config", "show"]);
    let line = human
        .lines()
        .find(|line| line.starts_with("sources.notes.routes"))
        .unwrap_or_else(|| panic!("config show prints routes:\n{human}"));
    assert!(
        line.contains("github.com/petsinc/*") && line.contains("file "),
        "with its layer: {line}"
    );
}

#[test]
fn every_malformed_routes_value_is_refused_at_load_naming_its_key() {
    let to = json!(TEAM);
    let pets = json!(["github.com/petsinc/*"]);
    for (routes, key, wanted) in [
        (
            json!("github.com/petsinc/*"),
            "sources.notes.routes",
            "list of entries",
        ),
        (json!(["team"]), "sources.notes.routes.0", "mapping"),
        (
            json!([{"repositories": pets, "to": to, "via": "x"}]),
            "sources.notes.routes.0.via",
            "unknown field",
        ),
        (
            json!([{"to": to}]),
            "sources.notes.routes.0.repositories",
            "names none",
        ),
        (
            json!([{"repositories": [7], "to": to}]),
            "sources.notes.routes.0.repositories",
            "not a pattern",
        ),
        (
            json!([{"repositories": {"a": "b"}, "to": to}]),
            "sources.notes.routes.0.repositories",
            "not a list of patterns",
        ),
        (
            json!([{"repositories": pets}]),
            "sources.notes.routes.0.to",
            "names",
        ),
        (
            json!([{"repositories": pets, "to": 5}]),
            "sources.notes.routes.0.to",
            "names",
        ),
        (
            json!([{"repositories": pets, "to": "Not_A_Name"}]),
            "sources.notes.routes.0.to",
            "next:",
        ),
    ] {
        let sandbox = Sandbox::new();
        sandbox.project_document(&document(&json!({
            NOTES: {
                "plugin": "local-md",
                "config": {"root": sandbox.subdirectory(NOTES)},
                "routes": routes,
            },
            TEAM: {"plugin": "local-md", "config": {"root": sandbox.subdirectory(TEAM)}},
        })));
        let refusal = refused(&sandbox, &["sources", "list"], 1);
        assert!(
            refusal.contains(&format!("{key}:")) && refusal.contains(wanted),
            "{routes}: refused under {key}:\n{refusal}"
        );
    }

    // The forms only a flag or a variable can write: an entry addressed by something that is
    // not an index, and indices with a gap.
    let sandbox = Sandbox::new();
    folders(&sandbox);
    for (flags, key) in [
        (
            vec!["--set", "sources.notes.routes.first.to=team"],
            "sources.notes.routes.first",
        ),
        (
            vec![
                "--set",
                "sources.notes.routes.1.to=team",
                "--set",
                "sources.notes.routes.1.repositories=github.com/a/*",
            ],
            "sources.notes.routes.1",
        ),
    ] {
        let mut arguments = flags.clone();
        arguments.extend(["sources", "list"]);
        let refusal = refused(&sandbox, &arguments, 1);
        assert!(
            refusal.contains(&format!("{key}:")) && refusal.contains("next:"),
            "{flags:?}: refused under {key}:\n{refusal}"
        );
    }
}

#[test]
fn routes_a_flag_sets_replace_the_documents_rather_than_merging_into_them() {
    let sandbox = Sandbox::new();
    folders(&sandbox);
    let flags = [
        "--set",
        "sources.notes.routes.0.repositories=github.com/nickderobertis/*",
        "--set",
        "sources.notes.routes.0.to=team",
    ];
    let with = |repository: &str| {
        let mut arguments: Vec<&str> = flags.to_vec();
        arguments.extend(["sources", "route", NOTES, "--repository", repository]);
        answer(&sandbox, &arguments)
    };
    assert_eq!(
        with("github.com/nickderobertis/lib"),
        json!({"source": NOTES, "destination": TEAM, "route": 0})
    );
    assert_eq!(
        with("github.com/petsinc/api"),
        json!({"source": NOTES, "destination": NOTES, "route": null}),
        "the document's petsinc entry is gone, not kept beside the flag's"
    );
}

#[test]
fn a_home_found_by_its_origin_stays_where_it_is_and_a_task_whose_origin_sits_elsewhere_is_refused()
{
    let sandbox = Sandbox::new();
    let plan = folders(&sandbox);
    let team = sandbox.project().join(TEAM);
    // The plan came out of `team`: the project records its counterpart there as its origin.
    record(&team, "projects", "kept", "title: Kept\nstatus: todo");
    record(
        &plan,
        "projects",
        "kept",
        "title: Kept\nstatus: todo\nmetadata: {onetaskgraph.origin: \"team:kept\"}",
    );
    record(
        &plan,
        "tasks",
        "own",
        "title: Own\nstatus: todo\nproject: kept\nrepositories: [github.com/nickderobertis/lib]",
    );
    let report = answer(&sandbox, &["project", "copy", "plan:kept", "--to", NOTES]);
    assert_eq!(landed(&report, "plan:kept"), "team:kept", "{report:#}");
    assert_eq!(
        outcome(&report, "plan:kept")["placed"],
        json!({"destination": TEAM, "route": 0})
    );
    let own = landed(&report, "plan:own");
    assert_eq!(source_of(&own), NOTES);
    assert_eq!(
        members_of(&sandbox, "team:kept"),
        vec![filed_under(&sandbox, &own)]
    );

    // A task recording a counterpart in `team` by its origin alone, which now routes to
    // `notes`, is refused before anything is written.
    record(&team, "tasks", "moved", "title: Moved\nstatus: todo");
    record(
        &plan,
        "tasks",
        "moved",
        "title: Moved\nstatus: todo\nrepositories: [github.com/nickderobertis/lib]\n\
         metadata: {onetaskgraph.origin: \"team:moved\"}",
    );
    let before = tree(&sandbox.project().join(NOTES));
    let refusal = refused(&sandbox, &["task", "copy", "plan:moved", "--to", NOTES], 1);
    assert!(
        refusal.contains("plan:moved") && refusal.contains("team:moved"),
        "{refusal}"
    );
    assert_eq!(tree(&sandbox.project().join(NOTES)), before);
}

#[test]
fn a_project_whose_tasks_and_own_repositories_disagree_keeps_its_home_in_the_named_source() {
    let sandbox = Sandbox::new();
    let plan = folders(&sandbox);
    record(
        &plan,
        "projects",
        "split",
        "title: Split\nstatus: todo\nrepositories: [github.com/petsinc/app]",
    );
    record(
        &plan,
        "tasks",
        "own",
        "title: Own\nstatus: todo\nproject: split\nrepositories: [github.com/nickderobertis/lib]",
    );
    let report = answer(&sandbox, &["project", "copy", "plan:split", "--to", NOTES]);
    assert_eq!(landed(&report, "plan:split"), "notes:split", "{report:#}");
    assert_eq!(
        outcome(&report, "plan:split")["placed"],
        json!({"destination": NOTES, "route": null})
    );
}

#[test]
fn a_document_with_no_home_to_follow_goes_where_its_own_repositories_route_it() {
    let sandbox = Sandbox::new();
    let plan = folders(&sandbox);
    record(
        &plan,
        "projects",
        "uncopied",
        "title: Never copied\nstatus: todo",
    );
    record(
        &plan,
        "documents",
        "loose",
        "title: Loose\nrepositories: [github.com/petsinc/app]",
    );
    record(
        &plan,
        "documents",
        "early",
        "title: Early\nproject: uncopied\nrepositories: [github.com/petsinc/app]",
    );
    record(
        &plan,
        "documents",
        "plain",
        "title: Plain\nproject: uncopied",
    );
    let report = answer(
        &sandbox,
        &[
            "document",
            "copy",
            "plan:loose",
            "plan:early",
            "plan:plain",
            "--to",
            NOTES,
        ],
    );
    for (document, to, route) in [
        ("plan:loose", TEAM, json!(0)),
        ("plan:early", TEAM, json!(0)),
        ("plan:plain", NOTES, Value::Null),
    ] {
        assert_eq!(source_of(&landed(&report, document)), to, "{report:#}");
        assert_eq!(
            outcome(&report, document)["placed"],
            json!({"destination": to, "route": route})
        );
    }
}

#[test]
fn a_member_the_routed_source_no_longer_holds_is_replaced_and_recorded() {
    let sandbox = Sandbox::new();
    let plan = folders(&sandbox);
    record(&plan, "projects", "goal", "title: Goal\nstatus: todo");
    record(
        &plan,
        "tasks",
        "own",
        "title: Own\nstatus: todo\nproject: goal\nrepositories: [github.com/nickderobertis/lib]",
    );
    record(
        &plan,
        "tasks",
        "pets",
        "title: Pets\nstatus: todo\nproject: goal\nrepositories: [github.com/petsinc/api]",
    );
    answer(&sandbox, &["project", "copy", "plan:goal", "--to", NOTES]);
    let member = members_of(&sandbox, "notes:goal")[0].clone();
    let file = sandbox.project().join(TEAM).join("projects").join(format!(
        "{}.md",
        member.split_once(':').expect("qualified").1
    ));
    std::fs::remove_file(&file).expect("somebody removes the member");

    record(
        &plan,
        "tasks",
        "pets2",
        "title: Pets two\nstatus: todo\nproject: goal\nrepositories: [github.com/petsinc/web]",
    );
    let added = answer(
        &sandbox,
        &[
            "project",
            "copy",
            "plan:goal",
            "--member",
            "plan:pets2",
            "--to",
            NOTES,
        ],
    );
    let members = members_of(&sandbox, "notes:goal");
    assert_eq!(members.len(), 1, "one member, replaced rather than added");
    assert!(file.exists(), "the member is written again");
    assert_eq!(
        filed_under(&sandbox, &landed(&added, "plan:pets2")),
        members[0]
    );
}

#[test]
fn a_routed_delivers_entry_names_the_task_in_the_source_it_landed_in() {
    let sandbox = Sandbox::new();
    let plan = folders(&sandbox);
    record(&plan, "projects", "goal", "title: Goal\nstatus: todo");
    record(
        &plan,
        "tasks",
        "pets",
        "title: Pets\nstatus: todo\nproject: goal\nrepositories: [github.com/petsinc/api]",
    );
    record(
        &plan,
        "tasks",
        "own",
        "title: Own\nstatus: todo\nproject: goal\n\
         repositories: [github.com/nickderobertis/lib]\ndelivers: [pets]",
    );
    let report = answer(&sandbox, &["project", "copy", "plan:goal", "--to", NOTES]);
    let own = landed(&report, "plan:own");
    let pets = landed(&report, "plan:pets");
    assert_eq!((source_of(&own), source_of(&pets)), (NOTES, TEAM));
    assert_eq!(report["delivers_rewritten"], 1, "{report:#}");
    let held = answer(&sandbox, &["task", "show", &own]);
    assert_eq!(
        held["items"][0]["item"]["delivers"],
        json!([pets]),
        "the entry names the routed task, qualified by the source it landed in"
    );
}

#[test]
fn a_routed_task_create_refuses_a_missing_home_and_takes_back_a_member_its_failed_write_made() {
    let sandbox = Sandbox::new();
    let plan = folders(&sandbox);
    record(&plan, "projects", "goal", "title: Goal\nstatus: todo");
    answer(
        &sandbox,
        &["project", "copy", "plan:goal", "--no-tasks", "--to", NOTES],
    );
    let body = sandbox.subdirectory("bodies").join("body.md");
    std::fs::write(&body, "What to do.\n").expect("a body");
    let body = body.display().to_string();
    let create = |project: &str, title: &str| {
        vec![
            "task".to_owned(),
            "create".to_owned(),
            NOTES.to_owned(),
            "--project".to_owned(),
            project.to_owned(),
            "--title".to_owned(),
            title.to_owned(),
            "--repository".to_owned(),
            "github.com/petsinc/api".to_owned(),
            "--body-file".to_owned(),
            body.clone(),
        ]
    };
    let arguments = create("nowhere", "Pets work");
    let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
    let refusal = refused(&sandbox, &arguments, 1);
    assert!(refusal.contains("notes:nowhere"), "{refusal}");
    assert!(
        tree(&sandbox.project().join(TEAM)).is_empty(),
        "nothing written"
    );

    // A file where `team`'s task folder goes: the task's write fails there after the member
    // project was made beside it.
    std::fs::write(sandbox.project().join(TEAM).join("tasks"), "not a folder")
        .expect("an obstacle");
    let home = sandbox.project().join(NOTES).join("projects/goal.md");
    let home_before = std::fs::read(&home).expect("the home");
    let arguments = create("goal", "Blocked work");
    let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
    refused(&sandbox, &arguments, 1);
    assert!(
        !sandbox.project().join(TEAM).join("projects").exists()
            || tree(&sandbox.project().join(TEAM).join("projects")).is_empty(),
        "the member the create made is taken back"
    );
    assert_eq!(
        std::fs::read(&home).expect("the home"),
        home_before,
        "and the home's member list is as it was"
    );
}

#[test]
fn a_members_read_refuses_a_stale_token_and_reports_every_member_it_cannot_read() {
    let sandbox = Sandbox::new();
    let plan = sandbox.subdirectory(PLAN);
    sandbox.project_document(&document(&json!({
        PLAN: {"plugin": "local-md", "config": {"root": plan}},
        NOTES: {
            "plugin": "local-md",
            "config": {"root": sandbox.subdirectory(NOTES)},
            "routes": [{"repositories": ["github.com/petsinc/*"], "to": TEAM}],
        },
        TEAM: {"plugin": "local-md", "config": {"root": sandbox.subdirectory(TEAM)}},
        // A source that cannot be built: its credential is nowhere.
        "broken": {"plugin": "github-projects", "config": {
            "owner": "nobody", "project_number": 1, "token_env": "ROUTES_ABSENT_TOKEN",
        }},
    })));
    record(&plan, "projects", "goal", "title: Goal\nstatus: todo");
    for (id, repository) in [
        ("a", "github.com/nickderobertis/a"),
        ("b", "github.com/petsinc/b"),
        ("c", "github.com/petsinc/c"),
    ] {
        record(
            &plan,
            "tasks",
            id,
            &format!("title: {id}\nstatus: todo\nproject: goal\nrepositories: [{repository}]"),
        );
    }
    answer(&sandbox, &["project", "copy", "plan:goal", "--to", NOTES]);
    let home = sandbox.project().join(NOTES).join("projects/goal.md");
    let text = std::fs::read_to_string(&home).expect("the home");
    let first = answer(
        &sandbox,
        &[
            "task",
            "list",
            "--project",
            "notes:goal",
            "--members",
            "--limit",
            "1",
        ],
    );
    let token = first["next"].as_str().expect("a second page").to_owned();

    // Another member joins the plan between the two pages: the token belongs to a different
    // query now.
    let rewrite = |members: &str| {
        std::fs::write(&home, text.replace("- team:goal", members)).expect("the home is edited");
    };
    rewrite("- team:goal\n  - plan:goal");
    let stale = refused(
        &sandbox,
        &[
            "task",
            "list",
            "--project",
            "notes:goal",
            "--members",
            "--limit",
            "1",
            "--page",
            &token,
        ],
        1,
    );
    assert!(stale.contains("different query"), "{stale}");

    // A member in a source nothing configures, and one in a source that cannot be built.
    rewrite("- team:goal\n  - ghost:goal\n  - broken:1");
    let output = run(
        &sandbox,
        &[
            "task",
            "list",
            "--project",
            "notes:goal",
            "--members",
            "--json",
        ],
    );
    assert_eq!(output.status.code(), Some(4), "{}", stderr(&output));
    let partial: Value = serde_json::from_str(&stdout(&output)).expect("JSON");
    for source in ["ghost", "broken"] {
        assert!(
            partial["errors"]
                .as_array()
                .expect("errors")
                .iter()
                .any(|error| error["source"] == source),
            "{source} is reported: {partial:#}"
        );
    }
    assert!(
        partial["items"]
            .as_array()
            .expect("items")
            .iter()
            .any(|task| source_of(task["id"].as_str().expect("an id")) == TEAM),
        "the readable member is still read"
    );

    // And a member list nobody can read is the home's source failing, not a plan without
    // members.
    let notes_text = std::fs::read_to_string(&home).expect("the home");
    let start = notes_text.find("onetaskgraph.members").expect("the key");
    let end = notes_text[start..]
        .find("\n  onetaskgraph.origin")
        .expect("the next key")
        + start;
    std::fs::write(
        &home,
        format!(
            "{}onetaskgraph.members: \"team:goal\"{}",
            &notes_text[..start],
            &notes_text[end..]
        ),
    )
    .expect("the home is edited");
    let output = run(
        &sandbox,
        &[
            "task",
            "list",
            "--project",
            "notes:goal",
            "--members",
            "--json",
        ],
    );
    assert_eq!(output.status.code(), Some(4), "{}", stderr(&output));
    let partial: Value = serde_json::from_str(&stdout(&output)).expect("JSON");
    assert!(
        partial["errors"]
            .as_array()
            .expect("errors")
            .iter()
            .any(|error| {
                error["source"] == NOTES && error.to_string().contains("onetaskgraph.members")
            }),
        "{partial:#}"
    );
}

#[test]
fn a_routed_document_copy_points_its_references_at_the_source_its_project_landed_in() {
    let sandbox = Sandbox::new();
    let plan = folders(&sandbox);
    petsinc_plan(&plan);
    let task_path = plan.join("tasks/pets-a.md");
    record(
        &plan,
        "documents",
        "pets-design",
        &format!(
            "title: Pets design\nproject: pets\nrepositories: [github.com/petsinc/api]\n---\n\
             Start at `{}`.\n\n<!-- -->",
            task_path.display()
        ),
    );
    let copied = answer(&sandbox, &["project", "copy", "plan:pets", "--to", NOTES]);
    let landed_task = landed(&copied, "plan:pets-a");
    assert_eq!(source_of(&landed_task), TEAM);
    let report = answer(
        &sandbox,
        &["document", "copy", "plan:pets-design", "--to", NOTES],
    );
    let document = landed(&report, "plan:pets-design");
    assert_eq!(source_of(&document), TEAM);
    assert_eq!(report["references_rewritten"], 1, "{report:#}");
    let held = answer(&sandbox, &["document", "show", &document]);
    let content = held["items"][0]["item"]["content"]
        .as_str()
        .expect("content");
    let there = sandbox.project().join(TEAM).join("tasks/pets-a.md");
    assert!(
        content.contains(&there.display().to_string()),
        "the reference names the task in the routed source:\n{content}"
    );
}

#[test]
fn a_routed_copy_whose_home_member_list_write_fails_takes_back_the_member_and_its_tasks() {
    let sandbox = Sandbox::new();
    let plan = sandbox.subdirectory(PLAN);
    // The repository and the status name the board reports for it, so a re-copy of the home
    // has nothing to write and the board's one write is the home's new member list.
    record(
        &plan,
        "projects",
        "goal",
        "title: One goal\nstatus: Todo\nrepositories: [github.com/nickderobertis/onetaskgraph]",
    );
    record(
        &plan,
        "tasks",
        "lib",
        "title: Library change\nstatus: todo\nproject: goal\n\
         repositories: [github.com/nickderobertis/lib]",
    );
    let linear = linear_empty_workspace(&sandbox);
    let (board, _) = crate::fixtures::github_projects_with_board_failing(
        &sandbox,
        &["updateIssue(input:$input)"],
    );
    sandbox.secrets_file("GITHUB_PROJECTS_FIXTURE_TOKEN=test-token\nLINEAR_API_KEY=fixture-key\n");
    sandbox.project_document(&document(&json!({
        PLAN: {"plugin": "local-md", "config": {"root": plan}},
        BOARD: {
            "plugin": "github-projects",
            "config": board,
            "routes": [{"repositories": ["github.com/petsinc/*"], "to": LINEAR}],
        },
        LINEAR: {"plugin": "linear", "config": linear},
    })));
    let first = answer(&sandbox, &["project", "copy", "plan:goal", "--to", BOARD]);
    let home = landed(&first, "plan:goal");
    let home_before = answer(&sandbox, &["project", "show", &home])["items"][0]["item"].clone();
    let again = answer(
        &sandbox,
        &["project", "copy", "plan:goal", "--no-tasks", "--to", BOARD],
    );
    assert_eq!(
        outcome(&again, "plan:goal")["action"],
        "unchanged",
        "the home needs no write of its own: {again:#}"
    );

    record(
        &plan,
        "tasks",
        "app",
        "title: App consumes it\nstatus: todo\nproject: goal\n\
         repositories: [github.com/petsinc/app]",
    );
    // The member and the task land in Linear, and then the board refuses the home's new list.
    let refusal = refused(
        &sandbox,
        &[
            "project",
            "copy",
            "plan:goal",
            "--member",
            "plan:app",
            "--to",
            BOARD,
        ],
        1,
    );
    assert!(
        refusal.contains("updateIssue") && !refusal.contains("could not be undone"),
        "the home's write failed and the copy was undone:\n{refusal}"
    );
    for verb in ["task", "project"] {
        let left = answer(&sandbox, &[verb, "list", "--source", LINEAR]);
        assert_eq!(
            left["items"],
            json!([]),
            "no {verb} the copy made remains in Linear"
        );
    }
    let home_after = answer(&sandbox, &["project", "show", &home])["items"][0]["item"].clone();
    assert_eq!(home_after["metadata"], home_before["metadata"]);
    assert_eq!(home_after["title"], home_before["title"]);
}
