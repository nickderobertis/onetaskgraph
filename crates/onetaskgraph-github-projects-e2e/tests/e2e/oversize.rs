//! An item too large for its board is found before anything of its copy is written, through
//! the binary.
//!
//! GitHub refuses an issue body over 65,536 characters, and a copy used to learn that from the
//! refusal of the write that carried it — after every item planned before it had landed. Every
//! journey here copies the generated plan out of a folder of Markdown onto the loopback board
//! with one item made too large, and holds the copy to sizing every item first: its dry run
//! reports each item's composed body — the content, the board's metadata slot and every asset
//! reference rewritten to where the board serves it — beside the board's limit, and the copy
//! itself refuses as `oversize`, naming the item, with no write of it reaching the board and
//! nothing of it at its source.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{Value, json};

use super::asset_board::{Call, Kind};
use super::plan_copy::{Plan, TASKS, success};
use crate::common::{stderr, stdout};

/// The most characters GitHub holds in one issue body, as the plugin states it.
const LIMIT: u64 = onetaskgraph_github_projects::BODY_LIMIT;

/// Every request of `calls` that writes anything at the board: a mutation, aliased or not, and
/// an upload.
fn writes(calls: &[Call]) -> Vec<&Call> {
    calls
        .iter()
        .filter(|call| {
            matches!(
                call.kind,
                Kind::Mutation | Kind::AliasedMutations | Kind::Upload
            )
        })
        .collect()
}

/// Every file under `root`, with its bytes.
pub(super) fn tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut held = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else {
                held.insert(
                    path.strip_prefix(root).unwrap().display().to_string(),
                    std::fs::read(&path).unwrap(),
                );
            }
        }
    }
    held
}

/// Prose of exactly `characters` characters, none of which a front matter or a reference rewrite
/// reads as its own.
fn prose(characters: usize) -> String {
    "Too much to say about one step of the plan. "
        .chars()
        .cycle()
        .take(characters)
        .collect()
}

/// The report's outcome for `source`.
fn outcome<'a>(report: &'a Value, source: &str) -> &'a Value {
    report["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["source"] == json!(source))
        .unwrap_or_else(|| panic!("{source} is not reported: {report}"))
}

/// Run `arguments` as a dry run, which has to report an oversize item and so exit `4`, and
/// answer its report — holding every item of it to a body size and the board's writes to none.
fn dry_run(plan: &Plan, arguments: &[&str]) -> Value {
    let mark = plan.board.calls_count();
    let mut with = arguments.to_vec();
    with.extend(["--dry-run", "--json"]);
    let output = plan.run(&with);
    assert_eq!(
        output.status.code(),
        Some(4),
        "a dry run naming an oversize item exits 4\n{}\n{}",
        stdout(&output),
        stderr(&output)
    );
    assert!(
        writes(&plan.board.calls_since(mark)).is_empty(),
        "a dry run writes nothing"
    );
    let report: Value = serde_json::from_str(&stdout(&output)).unwrap();
    for item in report["items"].as_array().unwrap() {
        assert!(
            item["body_size"].as_u64().is_some(),
            "every item a dry run sizes reports its body size: {item}"
        );
    }
    report
}

/// The dry run's outcome for `source`, held to being `oversize` with the board's limit, its
/// size over it: that size.
fn reported_oversize(report: &Value, source: &str) -> u64 {
    let item = outcome(report, source);
    assert_eq!(item["action"], json!("oversize"), "{item}");
    assert_eq!(item["limit"], json!(LIMIT), "{item}");
    let size = item["body_size"].as_u64().unwrap();
    assert_eq!(
        item["size"],
        json!(size),
        "the action names the size it is over by: {item}"
    );
    assert!(size > LIMIT, "{item}");
    size
}

/// Run `arguments`, which has to refuse as `oversize` naming exactly `oversize`, writing nothing
/// at the board and nothing at the folder the plan is written in.
fn refused(plan: &Plan, arguments: &[&str], oversize: &[(&str, u64)]) {
    let mark = plan.board.calls_count();
    let issues = plan.github.issues();
    let notes = tree(&plan.root);
    let mut with = arguments.to_vec();
    with.push("--json");
    let output = plan.run(&with);
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}\n{}",
        stdout(&output),
        stderr(&output)
    );
    let failure: Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(failure["failure"]["kind"], json!("oversize"), "{failure}");
    let named: Vec<(String, u64, u64)> = failure["oversize"]
        .as_array()
        .unwrap_or_else(|| panic!("the failure lists every oversize item: {failure}"))
        .iter()
        .map(|item| {
            assert_eq!(item["destination"], json!("board"), "{item}");
            (
                item["source"].as_str().unwrap().to_owned(),
                item["body_size"].as_u64().unwrap(),
                item["limit"].as_u64().unwrap(),
            )
        })
        .collect();
    let expected: Vec<(String, u64, u64)> = oversize
        .iter()
        .map(|(source, size)| ((*source).to_owned(), *size, LIMIT))
        .collect();
    assert_eq!(named, expected, "the refusal sizes as its dry run did");
    for (source, _) in oversize {
        assert!(
            stderr(&output).contains(&format!("{source} would write a body of")),
            "{}",
            stderr(&output)
        );
    }
    let sent = plan.board.calls_since(mark);
    assert_eq!(
        writes(&sent)
            .iter()
            .map(|call| super::plan_copy::operation(call))
            .collect::<Vec<_>>(),
        Vec::<String>::new(),
        "no write of the refused copy reached the board"
    );
    assert_eq!(plan.github.issues(), issues, "the board holds what it held");
    assert_eq!(tree(&plan.root), notes, "nothing was written at the source");
}

#[test]
fn a_plan_whose_later_tasks_are_too_large_is_refused_before_its_first_task_is_written() {
    let plan = Plan::new(0, Vec::new());
    // Task 1 fits and is planned before tasks 5 and 7, which do not.
    plan.write_task(5, &prose(70_000), true);
    plan.write_task(7, &prose(68_000), true);
    let report = dry_run(&plan, &["project", "copy", "notes:P", "--to", "board"]);
    let five = reported_oversize(&report, "notes:T-5");
    let seven = reported_oversize(&report, "notes:T-7");
    assert!(
        five > 70_000,
        "the composed body is larger than its content"
    );
    for source in std::iter::once("notes:P".to_owned()).chain(
        (1..=TASKS)
            .filter(|n| *n != 5 && *n != 7)
            .map(|n| format!("notes:T-{n}")),
    ) {
        let item = outcome(&report, &source);
        assert_eq!(item["action"], json!("created"), "{item}");
        assert!(item["body_size"].as_u64().unwrap() <= LIMIT, "{item}");
    }
    // The human rendering says the same, row by row, and names each item it would not copy.
    let output = plan.run(&["project", "copy", "notes:P", "--to", "board", "--dry-run"]);
    assert_eq!(output.status.code(), Some(4), "{}", stderr(&output));
    let rendered = stdout(&output);
    let row = |source: &str| {
        rendered
            .lines()
            .find(|line| line.starts_with(&format!("{source} ")))
            .unwrap_or_else(|| panic!("no row for {source}: {rendered}"))
            .to_owned()
    };
    let over = row("notes:T-5");
    assert!(over.contains("oversize"), "{over}");
    assert!(
        over.contains(&format!(
            "body: {five} characters, over the limit of {LIMIT}"
        )),
        "{over}"
    );
    let fits = row("notes:T-1");
    let size = outcome(&report, "notes:T-1")["body_size"].as_u64().unwrap();
    assert!(fits.contains(&format!("body: {size} characters")), "{fits}");
    assert!(!fits.contains("over the limit"), "{fits}");
    assert!(
        stderr(&output).contains(&format!(
            "notes:T-7 would not be copied: its body of {seven} characters is larger than the \
             {LIMIT} its destination holds"
        )),
        "{}",
        stderr(&output)
    );
    refused(
        &plan,
        &["project", "copy", "notes:P", "--to", "board"],
        &[("notes:T-5", five), ("notes:T-7", seven)],
    );
}

#[test]
fn a_plan_whose_tasks_fit_and_whose_document_does_not_is_refused_with_its_documents() {
    let plan = Plan::new(0, Vec::new());
    let path = plan.root.join("documents/D.md");
    let held = std::fs::read_to_string(&path).unwrap();
    let (front, body) = held.split_once("\n---\n").unwrap();
    std::fs::write(&path, format!("{front}\n---\n{}{body}", prose(66_000))).unwrap();
    let arguments = ["project", "copy", "notes:P", "--documents", "--to", "board"];
    let report = dry_run(&plan, &arguments);
    let size = reported_oversize(&report, "notes:D");
    for n in 1..=TASKS {
        let item = outcome(&report, &format!("notes:T-{n}"));
        assert!(item["body_size"].as_u64().unwrap() <= LIMIT, "{item}");
        assert_ne!(item["action"], json!("oversize"), "{item}");
    }
    refused(&plan, &arguments, &[("notes:D", size)]);
    // Without the document the same plan lands: only the document was over.
    let landed = success(&plan.run(&["project", "copy", "notes:P", "--to", "board", "--json"]));
    assert_eq!(landed["items"].as_array().unwrap().len(), 1 + TASKS);
}

#[test]
fn a_task_and_a_document_too_large_alone_are_each_refused_and_sized_by_their_dry_run() {
    let plan = Plan::new(0, Vec::new());
    plan.write_task(2, &prose(80_000), true);
    let path = plan.root.join("documents/D.md");
    let held = std::fs::read_to_string(&path).unwrap();
    let (front, _) = held.split_once("\n---\n").unwrap();
    std::fs::write(&path, format!("{front}\n---\n{}", prose(90_000))).unwrap();
    for (kind, source) in [("task", "notes:T-2"), ("document", "notes:D")] {
        let report = dry_run(&plan, &[kind, "copy", source, "--to", "board"]);
        let size = reported_oversize(&report, source);
        refused(
            &plan,
            &[kind, "copy", source, "--to", "board"],
            &[(source, size)],
        );
    }
}

#[test]
fn a_task_whose_content_fits_and_whose_composed_body_does_not_is_refused_at_its_composed_size() {
    let plan = Plan::new(0, Vec::new());
    // Content under the limit, referencing one image five times: the board's metadata slot, the
    // record the copy leaves and each reference rewritten to where the board serves the image
    // take it over.
    let reference = "\n\n![Where the copy spends its time](./diagram.png)\n";
    let content = format!("{}{}", prose(65_200), reference.repeat(5));
    let authored = content.chars().count() as u64;
    assert!(authored < LIMIT, "the content alone fits: {authored}");
    let assets = plan.root.join("tasks/T-3.assets");
    std::fs::create_dir_all(&assets).unwrap();
    std::fs::write(assets.join("diagram.png"), &plan.generated.png).unwrap();
    plan.write_task(3, &content, true);
    let report = dry_run(&plan, &["task", "copy", "notes:T-3", "--to", "board"]);
    let size = reported_oversize(&report, "notes:T-3");
    // Each of the five references grows from its relative path to a user-attachment URL.
    let grown = 5 * (attachment_url_length() - "./diagram.png".len());
    assert!(
        size >= authored + grown as u64,
        "the composed size counts the rewritten links: {size} against {authored} authored"
    );
    refused(
        &plan,
        &["task", "copy", "notes:T-3", "--to", "board"],
        &[("notes:T-3", size)],
    );
}

/// How long the URL GitHub serves a user attachment at is: what a body is sized with before an
/// image of it is uploaded.
fn attachment_url_length() -> usize {
    "https://github.com/user-attachments/assets/00000000-0000-0000-0000-000000000000".len()
}

#[test]
fn a_dry_run_sizes_a_task_as_the_board_then_stores_it() {
    let plan = Plan::new(0, Vec::new());
    // Filed under no project, so it is copied on its own.
    plan.write_task(1, &plan.generated.tasks[0], false);
    let output = plan.run(&[
        "task",
        "copy",
        "notes:T-1",
        "--to",
        "board",
        "--dry-run",
        "--json",
    ]);
    let report = success(&output);
    let item = outcome(&report, "notes:T-1");
    assert_eq!(item["action"], json!("created"), "{item}");
    let sized = item["body_size"].as_u64().unwrap();
    let landed = success(&plan.run(&["task", "copy", "notes:T-1", "--to", "board", "--json"]));
    let landed = outcome(&landed, "notes:T-1");
    assert_eq!(
        landed.get("body_size"),
        None,
        "a copy that writes reports no size"
    );
    let id = landed["destination"]
        .as_str()
        .unwrap()
        .strip_prefix("board:")
        .unwrap();
    let body = plan.github.body(id);
    let stored = body.as_str().unwrap().chars().count() as u64;
    assert_eq!(
        sized, stored,
        "the dry run sized the body the board then stored"
    );
}

#[test]
fn an_updates_dry_run_sizes_the_body_the_board_then_stores_its_recorded_image_reused() {
    let plan = Plan::new(0, Vec::new());
    let assets = plan.root.join("tasks/T-1.assets");
    std::fs::create_dir_all(&assets).unwrap();
    std::fs::write(assets.join("diagram.png"), &plan.generated.png).unwrap();
    let content = format!(
        "{}\n\n![Where the copy spends its time](./diagram.png)\n",
        plan.generated.tasks[0]
    );
    plan.write_task(1, &content, false);
    let first = success(&plan.run(&["task", "copy", "notes:T-1", "--to", "board", "--json"]));
    let id = outcome(&first, "notes:T-1")["destination"]
        .as_str()
        .unwrap()
        .strip_prefix("board:")
        .unwrap()
        .to_owned();
    // Edited, so the next copy updates the issue, keeping what the board records of it — the
    // image's uploaded URL among it, which the update reuses rather than uploading again.
    plan.write_task(1, &format!("{content}One more line.\n"), false);
    let dry = success(&plan.run(&[
        "task",
        "copy",
        "notes:T-1",
        "--to",
        "board",
        "--dry-run",
        "--json",
    ]));
    let item = outcome(&dry, "notes:T-1");
    assert_eq!(item["action"], json!("updated"), "{item}");
    let sized = item["body_size"].as_u64().unwrap();
    let mark = plan.board.calls_count();
    success(&plan.run(&["task", "copy", "notes:T-1", "--to", "board", "--json"]));
    assert!(
        !plan
            .board
            .calls_since(mark)
            .iter()
            .any(|call| call.kind == Kind::Upload),
        "the recorded image was reused, not uploaded again"
    );
    let stored = plan.github.body(&id).as_str().unwrap().chars().count() as u64;
    assert_eq!(
        sized, stored,
        "the update's dry run sized what the board stores"
    );
}
