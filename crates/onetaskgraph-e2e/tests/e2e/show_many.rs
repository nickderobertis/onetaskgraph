//! `task show-many`: several tasks by their qualified ids, each as `task show` reports it.
//!
//! Every journey here spawns the compiled binary and asserts on its exit code, stdout and
//! stderr. What `show-many` owes is stated against `task show` itself rather than against a
//! shape written out here: each detail is compared, whole, with the document `task show`
//! prints for that id in a separate invocation, so the two verbs cannot come to disagree.

use std::process::Output;

use serde_json::{Value, json};

use crate::common::{SOURCE_BOUNDARIES, Sandbox, stderr, stdout};
use crate::fixtures::{
    GITHUB_DRAFT_TASK, ROWS, Row, SOURCE, document, github_extra_task, github_projects_with_draft,
    github_projects_with_tasks, qualified,
};

fn host(row: &Row) -> Sandbox {
    let sandbox = Sandbox::new();
    sandbox.project_document(&row.document(&sandbox));
    sandbox
}

fn run(sandbox: &Sandbox, arguments: &[&str]) -> Output {
    sandbox
        .command()
        .args(arguments)
        .assert()
        .get_output()
        .clone()
}

/// The JSON a run printed, having exited `code`, quoting stderr when it did not.
fn answered(who: &str, sandbox: &Sandbox, arguments: &[&str], code: i32) -> (Value, String) {
    let output = run(sandbox, arguments);
    let said = stderr(&output);
    assert_eq!(
        output.status.code(),
        Some(code),
        "{who}: `onetaskgraph {}` exited {:?}\n{said}",
        arguments.join(" "),
        output.status.code(),
    );
    let printed = serde_json::from_str(&stdout(&output))
        .unwrap_or_else(|error| panic!("{who}: not JSON ({error}):\n{}", stdout(&output)));
    (printed, said)
}

/// What `task show <id> --json` prints for `id`, with `extra` flags.
fn shown(who: &str, sandbox: &Sandbox, id: &str, extra: &[&str]) -> Value {
    let mut arguments = vec!["task", "show", id, "--json"];
    arguments.extend_from_slice(extra);
    answered(who, sandbox, &arguments, 0).0
}

/// The details of a `show-many` answer, held to one per id.
fn details(who: &str, answer: &Value, count: usize) -> Vec<Value> {
    let details = answer["details"]
        .as_array()
        .unwrap_or_else(|| panic!("{who}: an answer carries details: {answer}"))
        .clone();
    assert_eq!(details.len(), count, "{who}: one detail per id: {answer}");
    assert_eq!(
        answer.as_object().map(|members| members.len()),
        Some(1),
        "{who}: `details` is the whole answer: {answer}"
    );
    details
}

/// The one failure a detail carries, held to name the source it was asked of.
fn failure_of<'a>(who: &str, detail: &'a Value, source: &str) -> &'a str {
    assert_eq!(detail["items"], json!([]), "{who}: no task: {detail}");
    assert!(
        detail.get("comments").is_none(),
        "{who}: no comments: {detail}"
    );
    let errors = detail["errors"].as_array().expect("errors is a list");
    assert_eq!(errors.len(), 1, "{who}: one failure: {detail}");
    assert_eq!(errors[0]["source"], json!(source), "{who}: {detail}");
    errors[0]["error"]["message"]
        .as_str()
        .unwrap_or_else(|| panic!("{who}: a failure says why: {detail}"))
}

#[test]
fn every_id_is_answered_in_request_order_as_task_show_answers_it_on_every_row() {
    for row in ROWS.iter().filter(|row| row.fixture.complete_dataset) {
        let who = row.name;
        let sandbox = host(row);
        let readable = ["T-3", "T-1", "T-2"].map(|id| qualified(SOURCE, id));
        let missing = qualified(SOURCE, "T-missing");
        let elsewhere = qualified("nowhere", "T-1");
        let ids = [
            readable[0].as_str(),
            readable[1].as_str(),
            elsewhere.as_str(),
            missing.as_str(),
            readable[2].as_str(),
        ];

        // A mix of readable and unreadable ids: every id answered, in the order given, and
        // the run exits non-zero because two details carry an error.
        let mut arguments = vec!["task", "show-many"];
        arguments.extend_from_slice(&ids);
        arguments.push("--json");
        let (answer, said) = answered(who, &sandbox, &arguments, 4);
        let held = details(who, &answer, ids.len());
        for (at, id) in [(0, &readable[0]), (1, &readable[1]), (4, &readable[2])] {
            assert_eq!(
                held[at],
                shown(who, &sandbox, id, &[]),
                "{who}: detail {at} is what `task show {id}` prints"
            );
            assert_eq!(held[at]["items"][0]["id"], json!(id), "{who}");
        }
        let unknown = failure_of(who, &held[2], "nowhere");
        assert!(
            unknown.contains("no source named") && unknown.contains("next:"),
            "{who}: an id naming no configured source says so: {unknown}"
        );
        let absent = failure_of(who, &held[3], SOURCE);
        assert!(
            absent.contains(&format!("no task with the id {missing}")) && absent.contains("next:"),
            "{who}: an id naming no task says so: {absent}"
        );
        for id in [&elsewhere, &missing] {
            assert!(
                said.contains(id.as_str()),
                "{who}: stderr names {id}:\n{said}"
            );
        }
        assert!(said.contains("next:"), "{who}:\n{said}");

        // Every id readable: the run succeeds, and with comments omitted each detail is what
        // `task show --no-comments` prints.
        let mut arguments = vec!["task", "show-many"];
        arguments.extend(readable.iter().map(String::as_str));
        arguments.extend(["--no-comments", "--json"]);
        let (answer, said) = answered(who, &sandbox, &arguments, 0);
        assert!(said.is_empty(), "{who}: a clean read says nothing:\n{said}");
        for (detail, id) in details(who, &answer, 3).iter().zip(&readable) {
            assert_eq!(
                *detail,
                shown(who, &sandbox, id, &["--no-comments"]),
                "{who}: {id} without its comments"
            );
            assert!(detail.get("comments").is_none(), "{who}: {detail}");
        }

        // The human rendering shows each task in the order asked, and names the failure.
        let output = run(
            &sandbox,
            &["task", "show-many", &readable[1], &missing, &readable[0]],
        );
        assert_eq!(output.status.code(), Some(4), "{who}: {}", stderr(&output));
        let rendered = stdout(&output);
        let alpha = rendered.find("Alpha engine");
        let gamma = rendered.find("Gamma");
        assert!(
            alpha.is_some() && gamma.is_some() && alpha < gamma,
            "{who}: T-1 then T-3:\n{rendered}"
        );
        assert!(
            rendered.contains(&missing) && rendered.contains("no task with the id"),
            "{who}:\n{rendered}"
        );
    }
}

#[test]
fn an_unqualified_id_refuses_the_invocation_before_any_source_is_asked() {
    let row = ROWS
        .iter()
        .find(|row| row.fixture.complete_dataset)
        .expect("a complete row");
    let sandbox = host(row);
    let output = run(
        &sandbox,
        &["task", "show-many", &qualified(SOURCE, "T-1"), "T-2"],
    );
    let said = stderr(&output);
    assert_ne!(output.status.code(), Some(0), "{said}");
    assert!(said.contains("next:"), "{said}");
    assert!(stdout(&output).is_empty(), "{}", stdout(&output));
}

#[test]
fn a_github_draft_carries_its_comment_refusal_without_refusing_the_tasks_beside_it() {
    for boundary in SOURCE_BOUNDARIES {
        let who = format!("github-projects draft across the {boundary:?} boundary");
        let sandbox = Sandbox::new();
        let config = github_projects_with_draft(&sandbox);
        sandbox.project_document(&document(&json!({
            SOURCE: boundary.source_with_secrets(
                "github-projects",
                config,
                &["GITHUB_PROJECTS_FIXTURE_TOKEN"],
            )
        })));
        let draft = qualified(SOURCE, GITHUB_DRAFT_TASK);
        let task = qualified(SOURCE, "T-1");
        let (answer, said) = answered(
            &who,
            &sandbox,
            &["task", "show-many", &draft, &task, "--json"],
            4,
        );
        let held = details(&who, &answer, 2);
        assert_eq!(held[0]["items"][0]["id"], json!(draft), "{who}");
        assert!(held[0].get("comments").is_none(), "{who}: {}", held[0]);
        assert!(
            held[0]["errors"][0]["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("draft")),
            "{who}: {}",
            held[0]
        );
        assert_eq!(held[1], shown(&who, &sandbox, &task, &[]), "{who}");
        assert!(
            said.contains("draft") && said.contains(&draft),
            "{who}:\n{said}"
        );
    }
}

#[test]
fn a_github_board_reads_its_items_with_their_comments_a_whole_batch_per_request() {
    use onetaskgraph_github_projects::{DETAIL_BATCH, graphql};

    let sandbox = Sandbox::new();
    let count = DETAIL_BATCH + 2;
    let (config, board) = github_projects_with_tasks(&sandbox, count);
    sandbox.project_document(&document(&json!({
        SOURCE: {"plugin":"github-projects","config":config}
    })));
    let ids: Vec<String> = (0..count)
        .map(|n| qualified(SOURCE, &github_extra_task(n)))
        .collect();
    // A comment on the first and the last of them, so the batch has comments to carry.
    for id in [&ids[0], &ids[count - 1]] {
        let output = sandbox
            .command()
            .args(["task", "comment", "add", id])
            .write_stdin("evidence\n")
            .assert()
            .get_output()
            .clone();
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    }
    let batches = count.div_ceil(DETAIL_BATCH);

    for (flags, commented) in [(&[][..], true), (&["--no-comments"][..], false)] {
        let before = board.served().len();
        let mut arguments = vec!["task", "show-many"];
        arguments.extend(ids.iter().map(String::as_str));
        arguments.extend_from_slice(flags);
        arguments.push("--json");
        let (answer, _) = answered("github-projects", &sandbox, &arguments, 0);
        let served = board.served()[before..].to_vec();
        assert_eq!(
            served.len(),
            batches,
            "{count} items cost {batches} requests: {served:#?}"
        );
        assert!(
            served
                .iter()
                .all(|(document, variables)| document == graphql::ISSUE_DETAILS
                    && variables["comments"] == json!(commented)),
            "every request is one batch read: {served:#?}"
        );
        let held = details("github-projects", &answer, count);
        for (detail, id) in held.iter().zip(&ids) {
            assert_eq!(detail["items"][0]["id"], json!(id));
            assert_eq!(detail["errors"], json!([]));
        }
        if commented {
            assert_eq!(held[0]["comments"][0]["body"], "evidence\n");
            assert_eq!(held[count - 1]["comments"][0]["body"], "evidence\n");
            assert_eq!(held[1]["comments"], json!([]));
            assert_eq!(held[0], shown("github-projects", &sandbox, &ids[0], &[]));
        } else {
            assert!(held.iter().all(|detail| detail.get("comments").is_none()));
        }
    }
}
