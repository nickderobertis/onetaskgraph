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
/// the host copies one there, each task depending on the one before it.
fn copied(tasks: usize) -> Copied {
    copied_with(tasks, &|n| if n > 1 { vec![n - 1] } else { Vec::new() })
}

/// [`copied`], with task `n` depending on each task `blockers(n)` names.
fn copied_with(tasks: usize, blockers: &dyn Fn(usize) -> Vec<usize>) -> Copied {
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
        let named: Vec<String> = blockers(n).iter().map(|k| format!("T-{k}")).collect();
        let depends = if named.is_empty() {
            String::new()
        } else {
            format!("depends_on: [{}]\n", named.join(", "))
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

/// The edges `project graph` draws of the folder's plan, sorted.
fn graph_edges(sandbox: &Sandbox) -> Vec<(String, String)> {
    let graph = run(
        sandbox,
        &["project", "graph", "authoring:P", "--format", "json"],
    );
    let mut edges: Vec<(String, String)> = graph["edges"]
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
    edges.sort();
    edges
}

/// Every file of the plan's folder, by path, as its bytes stand.
fn folder(sandbox: &Sandbox) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    let mut pending = vec![sandbox.subdirectory("authoring")];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory).expect("the folder") {
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

/// More blockers than a members listing carries of one task (`$nestedFirst`, 50), so the copy
/// back's read ahead of the members has that one task to read in a batch rather than from the
/// listing — the one request a read ahead sends in a copy back, which these journeys refuse.
const BLOCKERS: usize = 51;
/// The task blocked by every other.
const HUB: usize = BLOCKERS + 1;

/// A plan whose last task depends on each of the [`BLOCKERS`] before it, copied onto the board
/// with its cache then emptied, and two cards — a blocker and the hub — moved to `Done`.
fn hub_plan() -> Copied {
    let copied = copied_with(HUB, &|n| {
        if n == HUB {
            (1..=BLOCKERS).collect()
        } else {
            Vec::new()
        }
    });
    for n in [1, HUB] {
        copied
            .board
            .move_card(&copied.tasks[&format!("T-{n}")], "Done");
    }
    // Cold, so what is read ahead is decided by the listing alone.
    std::fs::remove_dir_all(&copied.cache).expect("the cache emptied");
    copied
}

/// The substring of the batched read a copy reads ahead with, and of no other document.
const READ_AHEAD: &str = "$children:Boolean!";
/// The substring of a read of one issue by id, and of no other document.
const ONE_ISSUE: &str =
    "query($id:ID!,$first:Int!,$nestedFirst:Int!,$boardItems:Int!,$duplicates:Boolean!";

fn copy_back_arguments(copied: &Copied) -> Vec<String> {
    [
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
    ]
    .map(str::to_owned)
    .to_vec()
}

/// The ids each request of `sent` carrying `fragment` asked about, in order.
fn asked(sent: &[(String, Value)], fragment: &str) -> Vec<Value> {
    sent.iter()
        .filter(|(document, _)| document.contains(fragment))
        .map(|(_, variables)| variables["id"].clone())
        .collect()
}

#[test]
fn the_markers_each_name_one_document() {
    for fragment in [READ_AHEAD, ONE_ISSUE] {
        let naming: Vec<&str> = graphql::DOCUMENTS
            .iter()
            .filter(|(document, _)| document.contains(fragment))
            .map(|(_, name)| *name)
            .collect();
        assert_eq!(naming.len(), 1, "{fragment}: {naming:?}");
    }
}

#[test]
fn a_refused_read_ahead_of_a_projects_members_falls_back_to_reading_each_and_lands_the_same() {
    let hub_of = |copied: &Copied| json!(copied.tasks[&format!("T-{HUB}")]);
    // Unrefused, the hub — the one member its listing did not carry whole — is read ahead in a
    // batch and never read on its own.
    let plain = hub_plan();
    let mark = plain.board.served_count();
    let arguments = copy_back_arguments(&plain);
    let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
    run(&plain.sandbox, &arguments);
    let sent = plain.board.served_since(mark);
    assert_eq!(
        sent.iter()
            .filter(|(document, _)| document.contains(READ_AHEAD))
            .count(),
        1,
        "the members not carried whole are read ahead in one batch"
    );
    assert!(
        !asked(&sent, ONE_ISSUE).contains(&hub_of(&plain)),
        "a member read ahead is not read again on its own"
    );
    let expected = folder(&plain.sandbox);

    // Refused, the read ahead holds nothing, and the hub is read on its own instead: the copy
    // still lands exactly what it lands unrefused.
    let refused = hub_plan();
    refused.board.refuse_once(READ_AHEAD);
    let mark = refused.board.served_count();
    let arguments = copy_back_arguments(&refused);
    let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
    let back = run(&refused.sandbox, &arguments);
    let sent = refused.board.served_since(mark);
    assert_eq!(
        sent.iter()
            .filter(|(document, _)| document.contains(READ_AHEAD))
            .count(),
        1,
        "the read ahead was sent once, and refused"
    );
    assert_eq!(
        asked(&sent, ONE_ISSUE)
            .iter()
            .filter(|id| **id == hub_of(&refused))
            .count(),
        1,
        "the member the refused read ahead would have answered is read on its own, once"
    );
    assert!(sent.iter().all(|(document, _)| !is_mutation(document)));
    for item in back["items"].as_array().expect("the copy's items") {
        assert_ne!(item["action"], "created", "{item}");
        assert_ne!(item["action"], "failed", "{item}");
    }
    // The same bytes in every file as the unrefused copy wrote, but for the board ids each
    // link names, which differ between the two boards.
    let landed = folder(&refused.sandbox);
    assert_eq!(
        landed.len(),
        expected.len(),
        "the same files as the unrefused copy"
    );
    let relink = |copied: &Copied, text: String| {
        let mut text = text.replace(&copied.project, "board:PROJECT");
        for (task, id) in &copied.tasks {
            text = text.replace(id.as_str(), &format!("BOARD-{task}"));
        }
        text
    };
    let normalised = |copied: &Copied, files: &BTreeMap<PathBuf, Vec<u8>>| {
        let root = copied.sandbox.subdirectory("authoring");
        files
            .iter()
            .map(|(path, bytes)| {
                (
                    path.strip_prefix(&root)
                        .expect("under the folder")
                        .to_owned(),
                    relink(copied, String::from_utf8(bytes.clone()).expect("text")),
                )
            })
            .collect::<BTreeMap<_, _>>()
    };
    assert_eq!(
        normalised(&refused, &landed),
        normalised(&plain, &expected),
        "a refused read ahead changes nothing the copy writes"
    );
    // And what it wrote is what a copy back owes: the moved statuses, the key removed, the
    // links kept, and every edge as authored.
    let listed = run(
        &refused.sandbox,
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
    assert_eq!(listed.len(), HUB, "no task was imported twice");
    for listed in listed {
        let task = &listed["item"];
        let id = task["id"].as_str().expect("an id");
        let status = if id == "T-1" || id == format!("T-{HUB}") {
            "done"
        } else {
            "todo"
        };
        assert_eq!(task["status"]["category"], status, "{id}: {task}");
        assert!(task["metadata"].get(KEY).is_none(), "{id}: {task}");
        assert_eq!(
            task["metadata"]["onetaskgraph.copies"]["board"],
            json!(format!("board:{}", refused.tasks[id])),
            "{id}: {task}"
        );
    }
    let mut authored: Vec<(String, String)> = (1..=BLOCKERS)
        .map(|k| (format!("authoring:T-{k}"), format!("authoring:T-{HUB}")))
        .collect();
    authored.sort();
    assert_eq!(graph_edges(&refused.sandbox), authored);
}

#[test]
fn a_member_whose_read_ahead_and_own_read_are_both_refused_fails_the_copy_writing_nothing() {
    // Where the hub's own read falls among the copy's reads of one issue once its read ahead is
    // refused, as a copy of the same plan with only that refused sends them.
    let probe = hub_plan();
    probe.board.refuse_once(READ_AHEAD);
    let mark = probe.board.served_count();
    let arguments = copy_back_arguments(&probe);
    let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
    run(&probe.sandbox, &arguments);
    let at = asked(&probe.board.served_since(mark), ONE_ISSUE)
        .iter()
        .position(|id| *id == json!(probe.tasks[&format!("T-{HUB}")]))
        .expect("the hub is read on its own");
    let copied = hub_plan();
    let before = folder(&copied.sandbox);
    copied.board.refuse_once(READ_AHEAD);
    copied.board.refuse_after(ONE_ISSUE, at);
    let mark = copied.board.served_count();
    let arguments = copy_back_arguments(&copied);
    let output = copied
        .sandbox
        .command()
        .args(&arguments)
        .write_stdin("")
        .assert()
        .get_output()
        .clone();
    let sent = copied.board.served_since(mark);
    assert_eq!(
        asked(&sent, ONE_ISSUE).get(at),
        Some(&json!(copied.tasks[&format!("T-{HUB}")])),
        "the refused read of one issue was the hub's"
    );
    assert_ne!(output.status.code(), Some(0), "{}", stdout(&output));
    let error = stderr(&output);
    assert!(
        error.contains("board") && error.contains(ONE_ISSUE),
        "the failure names the source and GitHub's refusal of the hub's own read, not the \
         refused read ahead: {error}"
    );
    assert!(sent.iter().all(|(document, _)| !is_mutation(document)));
    assert_eq!(
        folder(&copied.sandbox),
        before,
        "a copy that could not read its members writes nothing"
    );
}
