//! The follow-up features adopted by configuration alone, by an orchestrator repository that
//! carries none of this repository's own orchestration files.
//!
//! Its configuration is its own: three Linear team sources on Linear stand-ins — none a default,
//! no repository route declared, so the destination is named by the source a command names —
//! drafts in a folder of Markdown named `drafts` and plans in one named `authoring`, and its own
//! personas, templates and status vocabulary in a directory of its own, which the configuration
//! reaches by relative paths. The binary is driven there through every feature a follow-up loop
//! needs, each against a named team source.

use std::path::Path;

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{LinearWorkspace, dataset, linear_default_mapping, linear_workspace};

/// The status vocabulary the adopter keeps in its own directory: each of its words, and the
/// category it reads as.
const VOCABULARY: &str = r#"{"Parked": "backlog", "Proposed": "todo", "Ready": "queued",
"Doing": "in-progress", "Shipped": "done", "Dropped": "cancelled"}"#;

fn run(sandbox: &Sandbox, arguments: &[&str], code: i32) -> Value {
    let mut full = vec!["--json"];
    full.extend(arguments);
    let output = sandbox.command().args(&full).assert().get_output().clone();
    assert_eq!(
        output.status.code(),
        Some(code),
        "`onetaskgraph {}`\nstdout:\n{}\nstderr:\n{}",
        full.join(" "),
        stdout(&output),
        stderr(&output)
    );
    serde_json::from_str(&stdout(&output)).expect("the run answers JSON")
}

/// Write `text` at `relative` under `root`, making its folders.
fn file(root: &Path, relative: &str, text: &str) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("a folder");
    std::fs::write(path, text).expect("a file");
}

/// The adopter's repository: its own directory of personas, templates and vocabulary, its two
/// folders, and the configuration naming three Linear team sources.
fn adopter() -> (Sandbox, Vec<LinearWorkspace>) {
    let sandbox = Sandbox::new();
    let root = sandbox.project();
    file(
        &root,
        "followups/personas/reviewer.md",
        "Reviewed by the release persona.\n",
    );
    file(
        &root,
        "followups/templates/followup.md",
        "---\nonetaskgraph_template: 1\nvariables:\n  goal: {description: what the follow-up \
         does}\n---\n{{ goal }}\n\n{% include \"personas/reviewer.md\" %}",
    );
    file(&root, "followups/statuses.json", VOCABULARY);
    file(
        &root,
        "drafts/projects/inbox.md",
        "---\ntitle: Inbox\nstatus: Proposed\n---\nDrafts.\n",
    );
    std::fs::create_dir_all(root.join("authoring/tasks")).expect("a folder");
    let vocabulary: Value = serde_json::from_str(VOCABULARY).expect("the vocabulary");

    let mut workspaces = Vec::new();
    let mut sources = serde_json::Map::new();
    for team in ["team-a", "team-b", "team-c"] {
        let (mut config, workspace) = linear_workspace(&sandbox, dataset());
        config["status_mapping"] = linear_default_mapping();
        sources.insert(
            team.to_owned(),
            json!({"plugin": "linear", "config": config}),
        );
        workspaces.push(workspace);
    }
    for folder in ["drafts", "authoring"] {
        sources.insert(
            folder.to_owned(),
            json!({"plugin": "local-md",
                   "config": {"root": format!("./{folder}"), "status_mapping": vocabulary}}),
        );
    }
    sandbox.secrets_file("LINEAR_API_KEY=fixture-key\n");
    sandbox.project_document(
        &serde_json::to_string_pretty(&json!({
            // The default selection is the adopter's own folders: no Linear team is one.
            "default_sources": ["drafts", "authoring"],
            "sources": sources,
        }))
        .expect("a document"),
    );
    (sandbox, workspaces)
}

#[test]
fn an_orchestrator_with_none_of_this_repositorys_files_adopts_every_follow_up_feature() {
    let (sandbox, workspaces) = adopter();
    let team_b = &workspaces[1];

    // Its drafts are written from its own template, persona included, and moved in its own
    // vocabulary.
    let drafted = run(
        &sandbox,
        &[
            "task",
            "create",
            "drafts",
            "--project",
            "inbox",
            "--title",
            "Follow up",
            "--template",
            "followups/templates/followup.md",
            "--search-path",
            "followups",
            "--var",
            "goal=Close the loop",
            "--no-interactive",
        ],
        0,
    );
    let draft = &drafted["items"][0]["item"];
    assert!(
        draft["content"]
            .as_str()
            .is_some_and(|content| content.contains("Reviewed by the release persona.")),
        "{drafted:#}"
    );
    let draft_id = drafted["items"][0]["id"]
        .as_str()
        .expect("an id")
        .to_owned();
    let moved = run(&sandbox, &["task", "status", "set", &draft_id, "queued"], 0);
    // Read back in the adopter's own word — which this folder keeps lower-cased — and category.
    assert_eq!(moved["status"]["category"], "queued", "{moved:#}");
    assert!(
        moved["status"]["name"]
            .as_str()
            .is_some_and(|name| name.eq_ignore_ascii_case("Ready")),
        "{moved:#}"
    );

    // The default selection is its folders; a team is reached only by naming it.
    let listed = run(&sandbox, &["task", "list", "--all"], 0);
    assert!(
        listed["items"]
            .as_array()
            .expect("a list")
            .iter()
            .all(|task| !task["id"].as_str().unwrap_or_default().starts_with("team-")),
        "{listed:#}"
    );
    let every = run(
        &sandbox,
        &[
            "task", "list", "--source", "team-b", "--all", "--limit", "1",
        ],
        0,
    );
    let tasks: Vec<String> = every["items"]
        .as_array()
        .expect("a list")
        .iter()
        .map(|task| task["id"].as_str().expect("an id").to_owned())
        .collect();
    assert_eq!(tasks.len(), 4, "every page of team-b: {every:#}");
    assert!(every["next"].is_null());
    let task = tasks[0].clone();
    let native = |id: &str| id.trim_start_matches("team-b:").to_owned();

    // A held update lands at the version read, and one at a version since moved is refused.
    let read = run(&sandbox, &["task", "show", &task], 0)["items"][0]["item"]["version"]
        .as_str()
        .expect("a version")
        .to_owned();
    run(
        &sandbox,
        &[
            "task",
            "update",
            &task,
            "--title",
            "Held",
            "--if-version",
            &read,
        ],
        0,
    );
    let refused = run(
        &sandbox,
        &[
            "task",
            "update",
            &task,
            "--title",
            "Stale",
            "--if-version",
            &read,
        ],
        1,
    );
    assert_eq!(refused["failure"]["kind"], "version-moved", "{refused:#}");
    let shown = run(&sandbox, &["task", "show", &task], 0)["items"][0]["item"].clone();
    assert_eq!(shown["title"], "Held");
    assert_eq!(refused["held"]["version"], shown["version"]);

    // A guarded priority set answers `held` when the task is not at a priority it names.
    let held = run(
        &sandbox,
        &[
            "task",
            "priority",
            "set",
            &task,
            "low",
            "--when-priority",
            "none",
        ],
        0,
    );
    assert_eq!(held["outcome"], "held", "{held:#}");

    // Its repositories alone.
    let updated = run(
        &sandbox,
        &[
            "task",
            "update",
            &task,
            "--repository",
            "github.com/adopter/service",
        ],
        0,
    );
    assert_eq!(updated["written"], json!(["repositories"]), "{updated:#}");

    // A person's comment since an instant, and the next watermark.
    let existing = team_b.add_comment(&native(&task), "Before.");
    let instant = run(&sandbox, &["task", "comment", "list", &task], 0)["comments"]
        .as_array()
        .expect("a list")
        .iter()
        .find(|comment| comment["id"] == existing)
        .and_then(|comment| comment["created_at"].as_str())
        .map(|made| {
            (chrono::DateTime::parse_from_rfc3339(made).expect("a time")
                + chrono::TimeDelta::milliseconds(1))
            .to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true)
        })
        .expect("the earlier comment's time");
    let person = team_b.add_comment(&native(&task), "A person follows up.");
    let since = run(
        &sandbox,
        &["task", "comments", "team-b", "--since", &instant],
        0,
    );
    assert_eq!(
        since["comments"].as_array().map(Vec::len),
        Some(1),
        "{since:#}"
    );
    assert_eq!(since["comments"][0]["id"], person);
    assert_eq!(since["comments"][0]["author_kind"], "person");
    assert!(since["watermark"].is_string());

    // A task with its edges, in one read.
    team_b.relate(&native(&tasks[1]), &native(&tasks[2]), "blocks");
    let with = run(&sandbox, &["task", "show", &tasks[1], "--with-deps"], 0);
    let deps = run(&sandbox, &["task", "deps", &tasks[1]], 0);
    assert_eq!(with["depends_on"], deps["items"], "{with:#}");
    assert_eq!(
        with["depends_on"].as_array().map(Vec::len),
        Some(1),
        "{with:#}"
    );

    // The other teams are untouched by everything written to team-b.
    for other in ["team-a", "team-c"] {
        let titles: Vec<Value> =
            run(&sandbox, &["task", "list", "--source", other, "--all"], 0)["items"]
                .as_array()
                .expect("a list")
                .iter()
                .map(|task| task["item"]["title"].clone())
                .collect();
        assert!(!titles.contains(&json!("Held")), "{other}: {titles:?}");
    }
}
