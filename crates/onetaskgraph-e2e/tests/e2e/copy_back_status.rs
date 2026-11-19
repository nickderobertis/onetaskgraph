//! `project copy --update-fields status --metadata KEY=JSON`, and the counterpart a copy back
//! finds through the private side's own link, driven through the binary over two folders of
//! Markdown: `authoring`, declared private, and `board`, declared public — so a copy onto the
//! board withholds every origin naming the authoring plan.
//!
//! The plan is copied onto the board, worked there — statuses moved, a task added that depends
//! on a copied one, a key written on three items — and copied back with the status alone. The
//! authoring plan takes its counterparts' statuses in place, nothing else of it moves, the added
//! task is imported once with its edge on the authoring original, the named key is gone from
//! every item written and the other is set on each, and a second identical copy-back writes
//! nothing.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::attestations::Plan;
use crate::common::{stderr, stdout};
use crate::document_store::interpreter;
use crate::fixtures::document;

/// The key the board writes and the copy back removes, and the one the copy back sets.
const K: &str = "host.k";
const M: &str = "host.m";

struct Stores {
    plan: Plan,
    authoring: PathBuf,
    board: PathBuf,
}

fn stores() -> Stores {
    let plan = Plan::new(false);
    let authoring = plan.sandbox.subdirectory("authoring");
    let board = plan.sandbox.subdirectory("board");
    let mut configured: Value = serde_json::from_str(&document(&json!({
        "authoring": {"plugin": "local-md", "visibility": "private",
                      "config": {"root": authoring}},
        "board": {"plugin": "local-md", "visibility": "public", "config": {"root": board}},
    })))
    .expect("a document");
    configured["write_policy"] = json!({
        "check_command": [
            interpreter(), "-c", "import sys; sys.stdin.read(); print('{\"verdict\": \"pass\"}')"
        ],
        "visibility_command": [
            interpreter(), "-c",
            "import sys; sys.stdin.read(); print('{\"visibility\": \"public\"}')"
        ],
    });
    plan.sandbox
        .project_document(&serde_json::to_string_pretty(&configured).expect("a document"));
    Stores {
        plan,
        authoring,
        board,
    }
}

impl Stores {
    fn created(&self, arguments: &[&str]) -> String {
        stdout(&self.plan.exits(arguments, 0)).trim().to_owned()
    }

    fn task(&self, id: &str) -> Value {
        self.plan.json(&["task", "show", id, "--no-comments"])["items"][0]["item"].clone()
    }

    fn project(&self, id: &str) -> Value {
        self.plan.json(&["project", "show", id])["items"][0]["item"].clone()
    }

    /// Every task of the project `id`, by title.
    fn tasks_of(&self, id: &str) -> BTreeMap<String, Value> {
        let listed = self.plan.json(&["task", "list", "--project", id, "--all"]);
        listed["items"]
            .as_array()
            .expect("items")
            .iter()
            .map(|task| {
                (
                    task["item"]["title"].as_str().expect("a title").to_owned(),
                    task.clone(),
                )
            })
            .collect()
    }

    fn projects(&self, source: &str) -> Vec<Value> {
        self.plan.json(&["project", "list", "--source", source])["items"]
            .as_array()
            .expect("items")
            .clone()
    }
}

/// Every file under `root`, with its bytes.
fn tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory).expect("a directory") {
            let path = entry.expect("an entry").path();
            if path.is_dir() {
                pending.push(path);
            } else {
                files.insert(path.clone(), std::fs::read(&path).expect("a file"));
            }
        }
    }
    files
}

fn id_of(task: &Value) -> String {
    task["id"].as_str().expect("an id").to_owned()
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one journey: the copy out, the edits on the board, the copy back and the repeat \
              each read the state the step before left, so splitting it would only pass that \
              state between helpers"
)]
fn a_copy_back_from_the_board_writes_status_alone_imports_board_tasks_and_overrides_keys() {
    let stores = stores();
    let template = stores.plan.template("task.md");
    // The authoring plan: a project, `Build` from a template, and `Ship` depending on it and
    // already holding K, so the destination of the copy back holds the key it removes.
    let project = stores.created(&[
        "project",
        "create",
        "authoring",
        "--id",
        "plan-1",
        "--title",
        "The plan",
        "--body-file",
        &{
            let body = stores.plan.sandbox.subdirectory("inputs").join("plan.md");
            std::fs::write(&body, "The plan's description.\n").expect("a body");
            body.to_str().expect("UTF-8").to_owned()
        },
    ]);
    let build = stores.created(&[
        "task",
        "create",
        "authoring",
        "--project",
        "plan-1",
        "--title",
        "Build",
        "--template",
        &template,
        "--var",
        "goal=Build it",
        "--no-interactive",
    ]);
    let ship = stores.created(&[
        "task",
        "create",
        "authoring",
        "--project",
        "plan-1",
        "--title",
        "Ship",
        "--template",
        &template,
        "--var",
        "goal=Ship it",
        "--depends-on",
        &build,
        "--metadata",
        &format!("{K}=\"authored\""),
        "--no-interactive",
    ]);

    let onto = stores
        .plan
        .json(&["project", "copy", &project, "--to", "board"]);
    let board_project = onto["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|item| item["source"] == json!(project))
        .expect("the project landed")["destination"]
        .as_str()
        .expect("an id")
        .to_owned();
    let on_board = stores.tasks_of(&board_project);
    let (board_build, board_ship) = (id_of(&on_board["Build"]), id_of(&on_board["Ship"]));
    // The board withholds every origin naming the private plan.
    for task in on_board.values() {
        assert!(
            task["item"]["metadata"]
                .get("onetaskgraph.origin")
                .is_none(),
            "{task}"
        );
    }

    // Worked on the board.
    stores
        .plan
        .exits(&["task", "status", "set", &board_build, "done"], 0);
    stores.plan.exits(
        &["project", "status", "set", &board_project, "in-progress"],
        0,
    );
    let board_native = board_project.split_once(':').expect("qualified").1;
    let added = stores.created(&[
        "task",
        "create",
        "board",
        "--project",
        board_native,
        "--title",
        "Verify",
        "--depends-on",
        &board_build,
    ]);
    for (noun, id) in [
        ("project", board_project.as_str()),
        ("task", board_ship.as_str()),
        ("task", added.as_str()),
    ] {
        stores
            .plan
            .exits(&[noun, "metadata", "set", id, K, "\"board\""], 0);
    }

    let before_build = stores.task(&build);
    let before_ship = stores.task(&ship);
    let before_project = stores.project(&project);
    let back = [
        "project",
        "copy",
        board_project.as_str(),
        "--to",
        "authoring",
        "--update-fields",
        "status",
        "--metadata",
        &format!("{K}=null"),
        "--metadata",
        &format!("{M}={{\"a\":1}}"),
    ];
    let copied = stores.plan.json(&back);

    // The project and both tasks updated in place; nothing duplicated.
    assert_eq!(stores.projects("authoring").len(), 1, "{copied}");
    let authored = stores.tasks_of(&project);
    assert_eq!(
        authored.keys().cloned().collect::<Vec<_>>(),
        ["Build", "Ship", "Verify"],
        "{copied}"
    );
    assert_eq!(id_of(&authored["Build"]), build);
    assert_eq!(id_of(&authored["Ship"]), ship);
    for item in copied["items"].as_array().expect("items") {
        let expected = if item["source"] == json!(added) {
            "created"
        } else {
            "updated"
        };
        assert_eq!(item["action"], expected, "{item}");
    }

    // Statuses taken; titles, bodies, the description and the template answers untouched.
    let (after_build, after_ship) = (stores.task(&build), stores.task(&ship));
    let after_project = stores.project(&project);
    assert_eq!(after_build["status"]["category"], "done");
    assert_eq!(after_ship["status"]["category"], "todo");
    assert_eq!(after_project["status"]["category"], "in-progress");
    for (before, after) in [
        (&before_build, &after_build),
        (&before_ship, &after_ship),
        (&before_project, &after_project),
    ] {
        for field in ["title", "content", "repositories"] {
            assert_eq!(before[field], after[field], "{field}: {after}");
        }
        assert_eq!(
            before["metadata"]["onetaskgraph.template"],
            after["metadata"]["onetaskgraph.template"]
        );
    }
    let answers = |id: &str| stdout(&stores.plan.exits(&["task", "answers", id], 0));
    assert!(answers(&build).contains("Build it"));
    assert!(answers(&ship).contains("Ship it"));
    assert_eq!(after_ship["depends_on"], before_ship["depends_on"]);

    // The board's task imported once, its edge on the authoring original.
    let verify = id_of(&authored["Verify"]);
    let deps = stores.plan.json(&["task", "deps", &verify]);
    let ends: Vec<&str> = deps["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|edge| edge["to"]["id"].as_str().expect("a far end"))
        .collect();
    assert_eq!(ends, [build.as_str()], "{deps}");

    // K gone from every item written, the authoring task that held it before included; M on
    // each.
    for metadata in [
        &after_project["metadata"],
        &after_build["metadata"],
        &after_ship["metadata"],
        &stores.task(&verify)["metadata"],
    ] {
        assert!(metadata.get(K).is_none(), "{metadata}");
        assert_eq!(metadata[M], json!({"a": 1}), "{metadata}");
    }

    // A second identical copy back writes nothing.
    let before = tree(&stores.authoring);
    let again = stores.plan.json(&back);
    for item in again["items"].as_array().expect("items") {
        assert_eq!(item["action"], "unchanged", "{item}");
    }
    assert_eq!(
        tree(&stores.authoring),
        before,
        "a repeat wrote to the plan"
    );

    // A value that is not JSON is refused with nothing written at either end.
    let (authoring, board) = (tree(&stores.authoring), tree(&stores.board));
    let refused = stores.plan.run(&[
        "project",
        "copy",
        &board_project,
        "--to",
        "authoring",
        "--update-fields",
        "status",
        "--metadata",
        &format!("{M}=not json"),
    ]);
    assert_ne!(refused.status.code(), Some(0), "{}", stdout(&refused));
    assert!(
        stderr(&refused).contains("--metadata"),
        "{}",
        stderr(&refused)
    );
    assert_eq!(tree(&stores.authoring), authoring);
    assert_eq!(tree(&stores.board), board);
}
