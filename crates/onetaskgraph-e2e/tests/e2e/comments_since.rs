//! `task comments <SOURCE> --since`: every comment made or edited since an instant, through
//! the binary, on a folder of Markdown, the loopback GitHub board, the loopback Linear workspace
//! and an in-memory source.
//!
//! Each journey seeds comments before and after an instant, edits an older one after it, and
//! reads: the newer and the edited comments come back, oldest first, each with who wrote it — a
//! `bot` for the stand-in's bot, a `person` for a person — and the answer's watermark; a read
//! from that watermark returns any comment it returns again with the same id and version. The
//! board's journey also holds a newly made comment out of its search index for some seconds,
//! inside the plugin's stated window, and shows the read from the earlier read's watermark
//! returning it once it is indexed: no comment is lost across watermark reads.

use std::collections::BTreeSet;
use std::path::PathBuf;

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{
    GitHubBoardFields, LinearWorkspace, dataset, document, github_projects_with_board,
    linear_default_mapping, linear_workspace,
};

/// Every member the contract gives an answer, and every member it gives a comment.
const ANSWER_MEMBERS: [&str; 3] = ["comments", "schema_version", "watermark"];
const COMMENT_MEMBERS: [&str; 9] = [
    "author",
    "author_kind",
    "body",
    "created_at",
    "id",
    "item",
    "updated_at",
    "url",
    "version",
];

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

/// The answer to a read of `source`'s comments since `since`, narrowed by `narrowing`, held to
/// the contract's shape.
fn since(sandbox: &Sandbox, source: &str, since: &str, narrowing: &[&str]) -> Value {
    let mut arguments = vec!["task", "comments", source, "--since", since];
    arguments.extend(narrowing);
    let answer = run(sandbox, &arguments, 0);
    let members: BTreeSet<&str> = answer
        .as_object()
        .expect("an object")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(members, ANSWER_MEMBERS.into_iter().collect(), "{answer:#}");
    assert_eq!(answer["schema_version"], 1);
    assert!(
        chrono::DateTime::parse_from_rfc3339(answer["watermark"].as_str().expect("a watermark"))
            .is_ok()
    );
    for comment in answer["comments"].as_array().expect("a list") {
        let members: BTreeSet<&str> = comment
            .as_object()
            .expect("an object")
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            members,
            COMMENT_MEMBERS.into_iter().collect(),
            "{comment:#}"
        );
        for time in ["created_at", "updated_at"] {
            assert!(
                chrono::DateTime::parse_from_rfc3339(comment[time].as_str().expect("a time"))
                    .is_ok()
            );
        }
    }
    answer
}

/// The ids an answer lists, in its order.
fn ids(answer: &Value) -> Vec<String> {
    answer["comments"]
        .as_array()
        .expect("a list")
        .iter()
        .map(|comment| comment["id"].as_str().expect("an id").to_owned())
        .collect()
}

fn listed<'a>(answer: &'a Value, id: &str) -> &'a Value {
    answer["comments"]
        .as_array()
        .expect("a list")
        .iter()
        .find(|comment| comment["id"] == id)
        .unwrap_or_else(|| panic!("{id} is listed: {answer:#}"))
}

/// The items an answer's comments are on.
fn items(answer: &Value) -> BTreeSet<String> {
    answer["comments"]
        .as_array()
        .expect("a list")
        .iter()
        .map(|comment| comment["item"].as_str().expect("an item").to_owned())
        .collect()
}

/// Every comment a later read from `first`'s watermark returns again carries the id and version
/// `first` gave it, so a caller deduplicating by the two sees it once.
fn rereads_are_the_same(sandbox: &Sandbox, source: &str, first: &Value) {
    let watermark = first["watermark"].as_str().expect("a watermark");
    let again = since(sandbox, source, watermark, &[]);
    for comment in again["comments"].as_array().expect("a list") {
        if let Some(earlier) = first["comments"]
            .as_array()
            .expect("a list")
            .iter()
            .find(|earlier| earlier["id"] == comment["id"])
        {
            assert_eq!(earlier["version"], comment["version"], "{comment:#}");
            assert_eq!(earlier, comment, "{comment:#}");
        }
    }
}

/// The ids, oldest first by when each last changed — what an answer lists.
fn oldest_first(answer: &Value) {
    let times: Vec<&str> = answer["comments"]
        .as_array()
        .expect("a list")
        .iter()
        .map(|comment| comment["updated_at"].as_str().expect("a time"))
        .collect();
    let mut sorted = times.clone();
    sorted.sort_unstable();
    assert_eq!(times, sorted, "oldest first: {answer:#}");
}

fn folder_mapping() -> Value {
    json!({"backlog": "backlog", "todo": "todo", "queued": "queued",
           "in-progress": "in-progress", "done": "done", "cancelled": "cancelled"})
}

/// A folder of Markdown holding three tasks: `A` and `B` in the `followup` team at `todo`,
/// `C` in another team at `done`.
fn folder(sandbox: &Sandbox) -> PathBuf {
    let root = sandbox.subdirectory("folder");
    std::fs::create_dir_all(root.join("tasks")).expect("the task folder");
    for (id, status, team) in [
        ("A", "todo", "followup"),
        ("B", "todo", "followup"),
        ("C", "done", "other"),
    ] {
        std::fs::write(
            root.join("tasks").join(format!("{id}.md")),
            format!(
                "---\ntitle: Task {id}\nstatus: {status}\nmetadata:\n  orchestrator.team: {team}\n---\nBody.\n"
            ),
        )
        .expect("a task");
    }
    root
}

/// Rewrite every time a task file's comments section holds to `at`: comments a person wrote
/// before the instant a journey reads from.
fn backdate(file: &std::path::Path, at: &str) {
    let text = std::fs::read_to_string(file).expect("the task file");
    let stamp = regex_lite_times(&text);
    let mut backdated = text.clone();
    for time in stamp {
        backdated = backdated.replace(&time, at);
    }
    std::fs::write(file, backdated).expect("the task file");
}

/// Every `YYYY-MM-DDTHH:MM:SSZ` the text holds.
fn regex_lite_times(text: &str) -> BTreeSet<String> {
    let bytes = text.as_bytes();
    let mut found = BTreeSet::new();
    for start in 0..bytes.len().saturating_sub(19) {
        let candidate = &bytes[start..start + 20];
        let shape = candidate.iter().enumerate().all(|(index, c)| match index {
            4 | 7 => *c == b'-',
            10 => *c == b'T',
            13 | 16 => *c == b':',
            19 => *c == b'Z',
            _ => c.is_ascii_digit(),
        });
        if shape {
            found.insert(String::from_utf8_lossy(candidate).into_owned());
        }
    }
    found
}

fn body_file(sandbox: &Sandbox, text: &str) -> String {
    let path = sandbox.project().join(format!("body-{}.md", text.len()));
    std::fs::write(&path, text).expect("a body");
    path.to_str().expect("a path").to_owned()
}

fn added(sandbox: &Sandbox, task: &str, text: &str, author: Option<&str>) -> String {
    let file = body_file(sandbox, text);
    let mut arguments = vec!["task", "comment", "add", task, "--body-file", &file];
    if let Some(author) = author {
        arguments.extend(["--author", author]);
    }
    run(sandbox, &arguments, 0)["id"]
        .as_str()
        .expect("an id")
        .to_owned()
}

#[test]
fn a_folder_lists_every_comment_made_or_edited_since_an_instant() {
    let sandbox = Sandbox::new();
    let root = folder(&sandbox);
    sandbox.project_document(&document(&json!({
        "folder": {"plugin": "local-md",
                   "config": {"root": root, "status_mapping": folder_mapping()}},
    })));
    // Before the instant: one comment on each task, one of which is edited after it.
    let old_a = added(&sandbox, "folder:A", "An old note on A.\n", Some("ada"));
    added(&sandbox, "folder:B", "An old note on B.\n", Some("ada"));
    added(&sandbox, "folder:C", "An old note on C.\n", Some("ada"));
    for id in ["A", "B", "C"] {
        backdate(
            &root.join("tasks").join(format!("{id}.md")),
            "2026-09-13T15:11:07Z",
        );
    }
    let instant = "2026-10-01T00:00:00Z";

    // After it: a person's and a bot's new comments, and the older comment on A edited.
    let person = added(
        &sandbox,
        "folder:A",
        "A person follows up.\n",
        Some("grace"),
    );
    let bot = added(
        &sandbox,
        "folder:B",
        "A bot follows up.\n",
        Some("ci-helper[bot]"),
    );
    let on_c = added(&sandbox, "folder:C", "Late note on C.\n", Some("ada"));
    let edit = body_file(&sandbox, "The old note on A, edited.\n");
    run(
        &sandbox,
        &[
            "task",
            "comment",
            "edit",
            "folder:A",
            &old_a,
            "--body-file",
            &edit,
        ],
        0,
    );

    let answer = since(&sandbox, "folder", instant, &[]);
    let got: BTreeSet<String> = ids(&answer).into_iter().collect();
    let wanted: BTreeSet<String> = [&person, &bot, &on_c, &old_a]
        .into_iter()
        .cloned()
        .collect();
    assert_eq!(got, wanted, "{answer:#}");
    oldest_first(&answer);
    assert_eq!(listed(&answer, &person)["author_kind"], "person");
    assert_eq!(listed(&answer, &bot)["author_kind"], "bot");
    assert_eq!(listed(&answer, &bot)["author"], "ci-helper[bot]");
    assert_eq!(
        listed(&answer, &old_a)["body"],
        "The old note on A, edited.\n"
    );
    assert_eq!(listed(&answer, &person)["item"], "folder:A");
    rereads_are_the_same(&sandbox, "folder", &answer);

    // Each narrowing keeps exactly the matching items.
    let team = since(
        &sandbox,
        "folder",
        instant,
        &["--metadata", "orchestrator.team=followup"],
    );
    assert_eq!(
        items(&team),
        ["folder:A", "folder:B"].map(str::to_owned).into()
    );
    let done = since(&sandbox, "folder", instant, &["--status", "done"]);
    assert_eq!(items(&done), ["folder:C"].map(str::to_owned).into());
    let one = since(&sandbox, "folder", instant, &["--item", "folder:B"]);
    assert_eq!(ids(&one), vec![bot.clone()]);

    // An item of another source is refused by name.
    let refused = run(
        &sandbox,
        &[
            "task", "comments", "folder", "--since", instant, "--item", "other:X",
        ],
        1,
    );
    assert_eq!(
        refused["failure"]["kind"], "item-outside-source",
        "{refused:#}"
    );
}

#[test]
fn an_in_memory_source_lists_the_comments_its_configuration_made_since_an_instant() {
    let sandbox = Sandbox::new();
    let comment = |id: &str, task: &str, author: &str, created: &str, updated: &str| {
        json!({"task": task, "comment": {"id": id, "author": author, "created_at": created,
               "updated_at": updated, "body": format!("{id}\n"), "url": null}})
    };
    let task = |id: &str, status: &str, team: &str| {
        json!({"id": id, "title": id, "status": {"category": status, "name": status},
               "labels": [], "metadata": {"orchestrator.team": team}})
    };
    sandbox.project_document(&document(&json!({
        "memory": {"plugin": "in-memory", "config": {
            "capabilities": {"comments": "native"},
            "tasks": [task("A", "todo", "followup"), task("B", "todo", "followup"),
                      task("C", "done", "other")],
            "comments": [
                comment("old", "A", "ada", "2026-09-13T15:11:07Z", "2026-09-13T15:11:07Z"),
                comment("edited", "A", "ada", "2026-09-13T15:12:00Z", "2026-10-02T09:00:00Z"),
                comment("person", "A", "grace", "2026-10-03T10:00:00Z", "2026-10-03T10:00:00Z"),
                comment("bot", "B", "ci-helper[bot]", "2026-10-04T11:00:00Z",
                        "2026-10-04T11:00:00Z"),
                comment("late", "C", "ada", "2026-10-05T12:00:00Z", "2026-10-05T12:00:00Z"),
            ],
        }},
    })));
    let instant = "2026-10-01T00:00:00Z";
    let answer = since(&sandbox, "memory", instant, &[]);
    assert_eq!(
        ids(&answer),
        vec!["edited", "person", "bot", "late"],
        "{answer:#}"
    );
    assert_eq!(listed(&answer, "person")["author_kind"], "person");
    assert_eq!(listed(&answer, "bot")["author_kind"], "bot");
    assert_eq!(
        listed(&answer, "edited")["created_at"],
        "2026-09-13T15:12:00Z"
    );
    rereads_are_the_same(&sandbox, "memory", &answer);
    let team = since(
        &sandbox,
        "memory",
        instant,
        &["--metadata", "orchestrator.team=followup"],
    );
    assert_eq!(ids(&team), vec!["edited", "person", "bot"]);
    let done = since(&sandbox, "memory", instant, &["--status", "done"]);
    assert_eq!(ids(&done), vec!["late"]);
    let one = since(&sandbox, "memory", instant, &["--item", "memory:B"]);
    assert_eq!(ids(&one), vec!["bot"]);
}

/// A board and its handle, with `page_size` items to a page of every engine listing.
fn board(page_size: u64) -> (Sandbox, GitHubBoardFields) {
    let sandbox = Sandbox::new();
    let (config, board) = github_projects_with_board(&sandbox);
    sandbox.secrets_file("GITHUB_PROJECTS_FIXTURE_TOKEN=test-token\n");
    sandbox.project_document(
        &serde_json::to_string_pretty(&json!({
            "page_size": page_size,
            "sources": {"board": {"plugin": "github-projects", "config": config}},
        }))
        .expect("a document"),
    );
    (sandbox, board)
}

/// The board's own ids of the tasks it holds, and their qualified ids.
fn board_tasks(sandbox: &Sandbox) -> Vec<String> {
    run(sandbox, &["task", "list", "--source", "board", "--all"], 0)["items"]
        .as_array()
        .expect("a list")
        .iter()
        .map(|task| task["id"].as_str().expect("an id").to_owned())
        .collect()
}

#[test]
fn a_board_lists_every_comment_made_or_edited_since_an_instant_across_every_page() {
    let (sandbox, board) = board(2);
    let tasks = board_tasks(&sandbox);
    assert!(tasks.len() >= 3, "{tasks:?}");
    let native = |qualified: &str| qualified.trim_start_matches("board:").to_owned();
    let (a, b, c) = (native(&tasks[0]), native(&tasks[1]), native(&tasks[2]));
    let old = board.add_comment(&a, "An old note.");
    let instant = run(&sandbox, &["task", "comment", "list", &tasks[0]], 0)["comments"]
        .as_array()
        .expect("a list")
        .iter()
        .find(|comment| comment["id"] == old)
        .and_then(|comment| comment["created_at"].as_str())
        .map(|made| {
            (chrono::DateTime::parse_from_rfc3339(made).expect("a time")
                + chrono::TimeDelta::seconds(1))
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        })
        .expect("the old comment's time");
    board.pass_time(5);
    let person = board.add_comment(&a, "A person follows up.");
    let bot = board.add_comment_by(&b, "A bot follows up.", "renovate[bot]");
    board.edit_comment(&old, "The old note, edited.");
    // Many comments on one task, more than a page of them, and comments on more tasks than one
    // page of the engine's listing holds.
    board.seed_comments(&c, (0..105).map(|n| format!("Run {n} of the job.")));
    for task in tasks.iter().skip(3) {
        board.add_comment(&native(task), "And one more.");
    }

    let answer = since(&sandbox, "board", &instant, &[]);
    let got = ids(&answer);
    for id in [&person, &bot, &old] {
        assert!(got.contains(id), "{id}: {answer:#}");
    }
    let on_c = answer["comments"]
        .as_array()
        .expect("a list")
        .iter()
        .filter(|comment| comment["item"] == tasks[2])
        .count();
    assert_eq!(on_c, 105, "every page of one task's comments");
    assert_eq!(
        items(&answer).len(),
        tasks.len(),
        "every commented task, across every page: {:?}",
        items(&answer)
    );
    oldest_first(&answer);
    assert_eq!(listed(&answer, &person)["author_kind"], "person");
    assert_eq!(listed(&answer, &bot)["author_kind"], "bot");
    assert_eq!(listed(&answer, &bot)["author"], "renovate[bot]");
    assert_eq!(listed(&answer, &old)["body"], "The old note, edited.");
    rereads_are_the_same(&sandbox, "board", &answer);

    let one = since(&sandbox, "board", &instant, &["--item", &tasks[1]]);
    assert_eq!(ids(&one), vec![bot.clone()]);
}

#[test]
fn a_board_comment_its_search_had_not_yet_indexed_is_read_from_the_earlier_watermark() {
    let (sandbox, board) = board(50);
    let tasks = board_tasks(&sandbox);
    let native = |qualified: &str| qualified.trim_start_matches("board:").to_owned();
    // The board answers with its own clock — the `Date` of each answer — and keeps a change out
    // of its search for ten seconds of it: inside the plugin's two-minute window.
    board.read_as_it_changes(true);
    board.lag_search_index(10);
    let old = board.add_comment(&native(&tasks[0]), "Before.");
    board.pass_time(30);
    let instant =
        run(&sandbox, &["task", "comment", "list", &tasks[0]], 0)["comments"][0]["created_at"]
            .as_str()
            .map(str::to_owned)
            .expect("a time");
    let first = since(&sandbox, "board", &instant, &[]);
    assert_eq!(ids(&first), vec![old.clone()], "{first:#}");

    // Made now, and not yet in the search: the read made before it is indexed misses it.
    let late = board.add_comment_by(&native(&tasks[1]), "Indexed late.", "renovate[bot]");
    let before_indexed = since(
        &sandbox,
        "board",
        first["watermark"].as_str().expect("w"),
        &[],
    );
    assert!(!ids(&before_indexed).contains(&late), "{before_indexed:#}");

    // Once it is indexed, the read from the earlier read's watermark returns it.
    board.pass_time(20);
    let after_indexed = since(
        &sandbox,
        "board",
        before_indexed["watermark"].as_str().expect("a watermark"),
        &[],
    );
    assert!(ids(&after_indexed).contains(&late), "{after_indexed:#}");
    assert_eq!(listed(&after_indexed, &late)["author_kind"], "bot");
}

/// A Linear workspace and its handle, the source named `linear`.
fn linear() -> (Sandbox, LinearWorkspace) {
    let sandbox = Sandbox::new();
    let (mut config, workspace) = linear_workspace(&sandbox, dataset());
    config["status_mapping"] = linear_default_mapping();
    sandbox.secrets_file("LINEAR_API_KEY=fixture-key\n");
    sandbox.project_document(&document(
        &json!({"linear": {"plugin": "linear", "config": config}}),
    ));
    (sandbox, workspace)
}

#[test]
fn a_linear_source_lists_every_comment_made_or_edited_since_an_instant() {
    let (sandbox, workspace) = linear();
    let tasks: Vec<String> = run(
        &sandbox,
        &["task", "list", "--source", "linear", "--all"],
        0,
    )["items"]
        .as_array()
        .expect("a list")
        .iter()
        .map(|task| task["id"].as_str().expect("an id").to_owned())
        .collect();
    let native = |qualified: &str| qualified.trim_start_matches("linear:").to_owned();
    let old = workspace.add_comment(&native(&tasks[0]), "An old note.");
    let instant = run(&sandbox, &["task", "comment", "list", &tasks[0]], 0)["comments"]
        .as_array()
        .expect("a list")
        .iter()
        .find(|comment| comment["id"] == old)
        .and_then(|comment| comment["created_at"].as_str())
        .map(|made| {
            (chrono::DateTime::parse_from_rfc3339(made).expect("a time")
                + chrono::TimeDelta::milliseconds(1))
            .to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true)
        })
        .expect("the old comment's time");
    let person = workspace.add_comment(&native(&tasks[0]), "A person follows up.");
    let bot = workspace.add_bot_comment(&native(&tasks[1]), "A bot follows up.");
    workspace.edit_comment(&old, "The old note, edited.");

    let answer = since(&sandbox, "linear", &instant, &[]);
    let got: BTreeSet<String> = ids(&answer).into_iter().collect();
    assert_eq!(
        got,
        [&person, &bot, &old].into_iter().cloned().collect(),
        "{answer:#}"
    );
    oldest_first(&answer);
    assert_eq!(listed(&answer, &person)["author_kind"], "person");
    assert_eq!(listed(&answer, &bot)["author_kind"], "bot");
    assert_eq!(listed(&answer, &old)["body"], "The old note, edited.");
    rereads_are_the_same(&sandbox, "linear", &answer);
    let one = since(&sandbox, "linear", &instant, &["--item", &tasks[1]]);
    assert_eq!(ids(&one), vec![bot.clone()]);
}

#[test]
fn a_person_reads_the_comments_listing_and_a_task_with_its_edges() {
    let sandbox = Sandbox::new();
    let root = folder(&sandbox);
    std::fs::write(
        root.join("tasks").join("D.md"),
        "---\ntitle: Task D\nstatus: todo\ndepends_on: [A]\n---\nBody.\n",
    )
    .expect("a task");
    sandbox.project_document(&document(&json!({
        "folder": {"plugin": "local-md",
                   "config": {"root": root, "status_mapping": folder_mapping()}},
    })));
    let text = |arguments: &[&str]| -> String {
        let output = sandbox
            .command()
            .args(arguments)
            .assert()
            .get_output()
            .clone();
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        stdout(&output)
    };
    // Nothing since then: said so, with the watermark to read from next.
    let empty = text(&[
        "task",
        "comments",
        "folder",
        "--since",
        "2999-01-01T00:00:00Z",
    ]);
    assert!(
        empty.contains("no comment was made or edited since then"),
        "{empty}"
    );
    assert!(empty.contains("watermark: "), "{empty}");
    // A comment with no author, and one by a bot.
    let file = body_file(&sandbox, "Unsigned.\n");
    text(&["task", "comment", "add", "folder:A", "--body-file", &file]);
    added(&sandbox, "folder:B", "Signed.\n", Some("ci-helper[bot]"));
    let listed = text(&[
        "task",
        "comments",
        "folder",
        "--since",
        "2020-01-01T00:00:00Z",
    ]);
    assert!(
        listed.contains("folder:A") && listed.contains("(no author) (unknown)"),
        "{listed}"
    );
    assert!(
        listed.contains("ci-helper[bot] (bot)") && listed.contains("Signed."),
        "{listed}"
    );
    assert!(listed.contains("watermark: "), "{listed}");

    let shown = text(&["task", "show", "folder:D", "--with-deps"]);
    let (_, deps) = shown.split_once("depends on:").expect("the edges section");
    assert!(deps.contains("folder:A"), "{shown}");
    let plain = text(&["task", "show", "folder:D"]);
    assert!(!plain.contains("depends on:"), "{plain}");
}
