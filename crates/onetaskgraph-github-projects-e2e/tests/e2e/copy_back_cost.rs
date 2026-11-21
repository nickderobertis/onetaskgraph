//! What copying a plan back off a GitHub board costs: `project copy <board project> --to
//! <folder> --update-fields status --metadata KEY=null`, the plain copy a host refreshes its
//! plan with, driven through the binary against the loopback board and recorded as the
//! telemetry of the `github-project-copy-back-requests` budget, which `budget_runner` hands to
//! onebudgetspec, the one judge of it.
//!
//! The workload is the host's: a plan authored in a folder declared private — a project and its
//! tasks, each depending on the one before it and holding a caller key — copied onto a public
//! board whose cache is on, worked there (three cards moved to `Done`), and copied back with the
//! status alone and the key removed. The copy back is measured twice: with the board's cache
//! warmed by the copy that put the plan there, and with it cold. Before, it read every task by
//! its own `issue` request after listing them, so its requests grew with the plan; the members
//! listing it already makes now answers those reads. So every request but that listing's pages
//! is the same for 3 tasks and for 27, and the listing is one page per [`MEMBER_PAGE_SIZE`]
//! members, which is what the budget's reading of "the same number" is.

use std::collections::BTreeMap;
use std::path::PathBuf;

use onetaskgraph_github_projects::{MEMBER_PAGE_SIZE, graphql};
use serde_json::{Value, json};

use super::plan_copy::operation_name;
use crate::budget_runner::directory;
use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{GitHubBoardFields, github_projects_with_items};

/// The budget this journey records.
pub(crate) const BUDGET: &str = "github-project-copy-back-requests";
/// The workload the budget states, which its report refuses any other recording of.
pub(crate) const WORKLOAD: &str = "a 27-task plan copied from a folder declared private onto a \
    public board with its cache on, three cards moved to Done, then copied back with \
    --update-fields status --metadata host.k=null, the cache warmed by the copy onto the board \
    and cold";
/// The two scenarios, by the name a recording gives each.
pub(crate) const SCENARIOS: [&str; 2] = ["warm", "cold"];
/// The tasks of the measured plan, and of the plan it is held equal to.
pub(crate) const TASKS: usize = 27;
const FEW: usize = 3;
/// The caller key the copy back removes.
const KEY: &str = "host.k";
/// The tasks a person moves to `Done` on the board before the copy back.
const MOVED: [usize; 3] = [1, 2, 3];

/// The plan's folder, the board it was copied onto, and the board project that copy made.
struct Copied {
    sandbox: Sandbox,
    board: GitHubBoardFields,
    cache: PathBuf,
    project: String,
    /// Each task's board id, by its folder id.
    tasks: BTreeMap<String, String>,
}

fn run(sandbox: &Sandbox, arguments: &[&str]) -> Value {
    let output = sandbox
        .command()
        .args(arguments)
        .write_stdin("")
        .assert()
        .get_output()
        .clone();
    assert_eq!(
        output.status.code(),
        Some(0),
        "`onetaskgraph {}`:\n{}\n{}",
        arguments.join(" "),
        stdout(&output),
        stderr(&output)
    );
    serde_json::from_str(&stdout(&output)).expect("machine output is JSON")
}

/// A plan of `tasks` tasks authored in a folder declared private and copied onto the board, as
/// the host copies one there.
fn copied(tasks: usize) -> Copied {
    let sandbox = Sandbox::new();
    let (config, board) = github_projects_with_items(&sandbox, Vec::new());
    let root = sandbox.subdirectory("authoring");
    for directory in ["projects", "tasks"] {
        std::fs::create_dir_all(root.join(directory)).expect("the folder");
    }
    std::fs::write(
        root.join("projects/P.md"),
        format!(
            "---\ntitle: The plan\nstatus: todo\nmetadata:\n  {KEY}: \"authored\"\n---\nThe plan.\n"
        ),
    )
    .expect("the project");
    for n in 1..=tasks {
        let depends = if n > 1 {
            format!("depends_on: [T-{}]\n", n - 1)
        } else {
            String::new()
        };
        std::fs::write(
            root.join(format!("tasks/T-{n}.md")),
            format!(
                "---\ntitle: Step {n}\nstatus: todo\nproject: P\n{depends}metadata:\n  {KEY}: \
                 \"authored\"\n---\nStep {n} of the plan.\n"
            ),
        )
        .expect("a task");
    }
    let cache = sandbox.subdirectory("board-cache");
    sandbox.project_document(
        &serde_json::to_string_pretty(&json!({
            "sources": {
                "authoring": {"plugin": "local-md", "visibility": "private",
                              "config": {"root": root}},
                "board": {"plugin": "github-projects", "visibility": "public", "config": config,
                          "cache": {"enabled": true, "dir": cache}},
            },
            // A store declaring a visibility is held to the public boundary, which nothing here
            // is about: every write passes.
            "write_policy": {
                "check_command": ["python3", "-c",
                    "import sys; sys.stdin.read(); print('{\"verdict\": \"pass\"}')"],
                "visibility_command": ["python3", "-c",
                    "import sys; sys.stdin.read(); print('{\"visibility\": \"public\"}')"],
            },
        }))
        .expect("a document"),
    );
    let onto = run(
        &sandbox,
        &["project", "copy", "authoring:P", "--to", "board", "--json"],
    );
    let mut project = None;
    let mut landed = BTreeMap::new();
    for item in onto["items"].as_array().expect("the copy's items") {
        let source = item["source"].as_str().expect("a source id");
        let destination = item["destination"]
            .as_str()
            .and_then(|id| id.strip_prefix("board:"))
            .expect("a board id")
            .to_owned();
        match source.strip_prefix("authoring:") {
            Some("P") => project = Some(format!("board:{destination}")),
            Some(task) => {
                landed.insert(task.to_owned(), destination);
            }
            None => panic!("an item of another source: {item}"),
        }
    }
    assert_eq!(landed.len(), tasks, "every task landed: {onto}");
    Copied {
        sandbox,
        board,
        cache,
        project: project.expect("the project landed"),
        tasks: landed,
    }
}

/// One copy back of `tasks` tasks, the cache warmed by the copy onto the board or emptied
/// first: every request it sent, held to writing what a copy back writes.
fn copy_back(tasks: usize, warm: bool) -> Vec<(String, Value)> {
    let copied = copied(tasks);
    for n in MOVED {
        copied
            .board
            .move_card(&copied.tasks[&format!("T-{n}")], "Done");
    }
    if !warm {
        std::fs::remove_dir_all(&copied.cache).expect("the cache emptied");
    }
    let mark = copied.board.served_count();
    let back = run(
        &copied.sandbox,
        &[
            "project",
            "copy",
            &copied.project,
            "--to",
            "authoring",
            "--update-fields",
            "status",
            "--metadata",
            &format!("{KEY}=null"),
            "--json",
        ],
    );
    let sent = copied.board.served_since(mark);
    assert!(
        sent.iter().all(|(document, _)| !is_mutation(document)),
        "a copy back writes nothing to the board it reads"
    );
    // Every record is written where it was authored, with no new one made.
    for item in back["items"].as_array().expect("the copy's items") {
        assert!(
            item["destination"]
                .as_str()
                .is_some_and(|id| id.starts_with("authoring:")),
            "{item}"
        );
        assert_ne!(item["action"], "created", "{item}");
    }
    // What it writes, read back through the binary: the moved statuses, every other status as
    // it was, the key removed from every item, the edges as authored, and each item's link to
    // its board counterpart — with no task imported twice.
    let project = run(
        &copied.sandbox,
        &["project", "show", "authoring:P", "--json"],
    );
    let project = &project["items"][0]["item"];
    assert!(project["metadata"].get(KEY).is_none(), "{project}");
    assert_eq!(
        project["metadata"]["onetaskgraph.copies"]["board"],
        json!(copied.project),
        "{project}"
    );
    let listed = run(
        &copied.sandbox,
        &[
            "task",
            "list",
            "--project",
            "authoring:P",
            "--all",
            "--json",
        ],
    );
    let listed = listed["items"].as_array().expect("the tasks");
    assert_eq!(listed.len(), tasks, "no task was imported twice");
    for listed in listed {
        let task = &listed["item"];
        let id = task["id"].as_str().expect("an id");
        let n: usize = id
            .strip_prefix("T-")
            .and_then(|n| n.parse().ok())
            .expect("an authored task");
        let status = if MOVED.contains(&n) { "done" } else { "todo" };
        assert_eq!(task["status"]["category"], status, "{id}: {task}");
        assert!(task["metadata"].get(KEY).is_none(), "{id}: {task}");
        assert_eq!(
            task["metadata"]["onetaskgraph.copies"]["board"],
            json!(format!("board:{}", copied.tasks[id])),
            "{id} keeps its link to its board counterpart: {task}"
        );
    }
    let graph = run(
        &copied.sandbox,
        &["project", "graph", "authoring:P", "--format", "json"],
    );
    let edges: Vec<(String, String)> = graph["edges"]
        .as_array()
        .expect("the edges")
        .iter()
        .map(|edge| {
            (
                edge["from"].as_str().expect("an id").to_owned(),
                edge["to"].as_str().expect("an id").to_owned(),
            )
        })
        .collect();
    let mut authored: Vec<(String, String)> = (2..=tasks)
        .map(|n| (format!("authoring:T-{}", n - 1), format!("authoring:T-{n}")))
        .collect();
    let mut edges = edges;
    edges.sort();
    authored.sort();
    assert_eq!(edges, authored, "every edge as authored, and no other");
    sent
}

fn is_mutation(document: &str) -> bool {
    document.trim_start().starts_with("mutation")
}

/// How many of `sent` were pages of the project's members listing.
fn listing_pages(sent: &[(String, Value)]) -> usize {
    sent.iter()
        .filter(|(document, _)| document == graphql::SUB_ISSUES)
        .count()
}

/// Every request but the members listing's pages, by operation, in order.
fn beside_the_listing(sent: &[(String, Value)]) -> Vec<&'static str> {
    sent.iter()
        .filter(|(document, _)| document != graphql::SUB_ISSUES)
        .map(|(document, _)| operation_name(document))
        .collect()
}

#[test]
fn measure_github_project_copy_back_requests() {
    let mut recorded = Vec::new();
    for scenario in SCENARIOS {
        let warm = scenario == "warm";
        let many = copy_back(TASKS, warm);
        let few = copy_back(FEW, warm);
        // No read is made per task: what is sent beside the listing is the same for 3 tasks as
        // for 27, and the listing takes one page per `MEMBER_PAGE_SIZE` members.
        assert_eq!(
            beside_the_listing(&many),
            beside_the_listing(&few),
            "{scenario}: a copy back of {TASKS} tasks sent what one of {FEW} did beside the \
             listing"
        );
        for (tasks, sent) in [(TASKS, &many), (FEW, &few)] {
            assert_eq!(
                listing_pages(sent),
                tasks.div_ceil(MEMBER_PAGE_SIZE as usize),
                "{scenario}: {tasks} tasks"
            );
        }
        let mut by_operation = BTreeMap::new();
        for (document, _) in &many {
            *by_operation
                .entry(operation_name(document))
                .or_insert(0_usize) += 1;
        }
        recorded.push(
            json!({"scenario": scenario, "tasks": TASKS, "requests": many.len(),
            "listing_pages": listing_pages(&many), "by_operation": by_operation,
            "few": {"tasks": FEW, "requests": few.len()}}),
        );
    }
    let value = recorded
        .iter()
        .map(|scenario| scenario["requests"].as_u64().expect("a count"))
        .max()
        .expect("two scenarios");
    let detail = recorded
        .iter()
        .map(|scenario| {
            format!(
                "{}: {} requests for {TASKS} tasks ({} listing pages), {} for {FEW}",
                scenario["scenario"].as_str().expect("a name"),
                scenario["requests"],
                scenario["listing_pages"],
                scenario["few"]["requests"]
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    let directory = directory();
    std::fs::create_dir_all(&directory).expect("the telemetry directory");
    std::fs::write(
        onetaskgraph_e2e_support::telemetry::file_in(&directory, BUDGET),
        json!({"budget": BUDGET, "value": value, "workload": WORKLOAD,
               "scenarios": recorded, "detail": detail})
        .to_string(),
    )
    .expect("the telemetry file");
    eprintln!("{BUDGET}: {value} requests; {detail}");
}
