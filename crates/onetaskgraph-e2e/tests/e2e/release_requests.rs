//! Every request a copy and a task write verb sends, held to what the v0.12.13 release sent
//! for the same command over the same loopback stores.
//!
//! The golden beside this file, `release_requests.v0.12.13.json`, was recorded by running these
//! very scenarios with the v0.12.13 binary — `OTG_REQUESTS_BINARY` naming it and
//! `OTG_REQUESTS_RECORD` set — so it is a measurement of that release, not a restatement of this
//! one. Each scenario is a sequence of commands; what is held is the documents each loopback was
//! sent for each command, in order. The one difference this release owes is stated where it is
//! allowed: a create onto a board from a hosted source lists the repository by its create key
//! once more, right after `createIssue`.
//!
//! The comparison parses the golden, so its layout is not part of it; a recording writes serde's
//! pretty-print, which `workspace:format-check` refuses, so run
//! `bun run biome format --write` over the golden after recording it.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{
    GitHubBoardFields, LinearWorkspace, dataset, document, github_projects_with_board,
    linear_default_mapping, linear_workspace,
};

fn golden() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/e2e/release_requests.v0.12.13.json")
}

struct Stores {
    sandbox: Sandbox,
    board: GitHubBoardFields,
    linear: LinearWorkspace,
    notes: PathBuf,
}

impl Stores {
    fn new() -> Self {
        let sandbox = Sandbox::new();
        let (board_config, board) = github_projects_with_board(&sandbox);
        let (mut linear_config, linear) = linear_workspace(&sandbox, dataset());
        linear_config["status_mapping"] = linear_default_mapping();
        sandbox
            .secrets_file("GITHUB_PROJECTS_FIXTURE_TOKEN=test-token\nLINEAR_API_KEY=fixture-key\n");
        let notes = sandbox.subdirectory("notes");
        std::fs::create_dir_all(notes.join("tasks")).expect("tasks");
        let mapping = json!({"todo": "todo", "queued": "queued", "in-progress": "in-progress",
                             "done": "done", "backlog": "backlog", "cancelled": "cancelled"});
        sandbox.project_document(&document(&json!({
            "board": {"plugin": "github-projects", "config": board_config},
            "linear": {"plugin": "linear", "config": linear_config},
            "notes": {"plugin": "local-md", "config": {"root": notes, "status_mapping": mapping}},
        })));
        std::fs::write(
            notes.join("tasks/A.md"),
            "---\ntitle: Task A\nstatus: todo\npriority: high\n---\nThe body.\n",
        )
        .expect("a note");
        Self {
            sandbox,
            board,
            linear,
            notes,
        }
    }

    /// Run `arguments` with the binary under measurement, answering its JSON.
    fn run(&self, arguments: &[&str]) -> Value {
        let binary = std::env::var_os("OTG_REQUESTS_BINARY").map_or_else(
            || onetaskgraph_e2e_support::binary().to_owned(),
            PathBuf::from,
        );
        let mut full = vec!["--json"];
        full.extend(arguments);
        let output = self
            .sandbox
            .subprocess(binary)
            .args(&full)
            .output()
            .expect("the binary runs");
        assert!(
            output.status.success(),
            "`onetaskgraph {}`\n{}\n{}",
            full.join(" "),
            stdout(&output),
            stderr(&output)
        );
        serde_json::from_str(&stdout(&output)).expect("JSON")
    }

    /// The documents each store was sent while `step` ran, by store.
    fn measured(&self, step: impl FnOnce(&Self)) -> Value {
        let (board, linear) = (self.board.served().len(), self.linear.served().len());
        step(self);
        let documents = |served: Vec<(String, Value)>, from: usize| -> Vec<String> {
            served[from..]
                .iter()
                .map(|(document, _)| spelled(document))
                .collect()
        };
        json!({"board": documents(self.board.served(), board),
               "linear": documents(self.linear.served(), linear)})
    }

    fn revise(&self) {
        let path = self.notes.join("tasks/A.md");
        let text = std::fs::read_to_string(&path).expect("the note");
        std::fs::write(
            path,
            text.replace("title: Task A", "title: Task A, revised"),
        )
        .expect("an edit");
    }
}

/// A request as the golden records it: whether it reads or writes, and every root field it
/// selects, aliases kept — so a request the same operation sends with a wider selection, as this
/// release's priority write selects the whole issue, is the same request; a request added,
/// removed or sent to another field is not.
fn spelled(document: &str) -> String {
    let document = document.trim_start();
    let kind = if document.starts_with("mutation") {
        "mutation"
    } else if document.starts_with('{') || document.starts_with("query") {
        "query"
    } else {
        // A REST request, recorded by the loopback as `<METHOD> <path>`.
        return document.to_owned();
    };
    let Some(open) = document.find('{') else {
        return kind.to_owned();
    };
    let mut roots = Vec::new();
    let (mut depth, mut parens) = (0usize, 0usize);
    let mut token = String::new();
    for character in document[open..].chars() {
        let part_of_a_name =
            character.is_alphanumeric() || matches!(character, '_' | ':' | '.' | '$');
        if part_of_a_name && depth == 1 && parens == 0 {
            token.push(character);
            continue;
        }
        if !token.is_empty() {
            let name = std::mem::take(&mut token);
            if !name.starts_with("...") && !name.ends_with(':') {
                roots.push(name);
            }
        }
        match character {
            '(' => parens += 1,
            ')' => parens = parens.saturating_sub(1),
            '{' if parens == 0 => depth += 1,
            '}' if parens == 0 => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    format!("{kind} {}", roots.join(" "))
}

/// What a copy of the note into `destination` and back out of it sends, step by step: the
/// create, the update after an edit, the `unchanged` copy after nothing changed, and each of
/// the task write verbs on what it created.
fn copies_and_writes_into(destination: &str) -> Value {
    let stores = Stores::new();
    let mut steps = serde_json::Map::new();
    let mut landed = String::new();
    steps.insert(
        "create".to_owned(),
        stores.measured(|stores| {
            let copied = stores.run(&["task", "copy", "notes:A", "--to", destination]);
            landed = copied["items"][0]["destination"]
                .as_str()
                .expect("an id")
                .to_owned();
        }),
    );
    stores.revise();
    steps.insert(
        "update".to_owned(),
        stores.measured(|stores| {
            stores.run(&["task", "copy", "notes:A", "--to", destination]);
        }),
    );
    steps.insert(
        "unchanged".to_owned(),
        stores.measured(|stores| {
            stores.run(&["task", "copy", "notes:A", "--to", destination]);
        }),
    );
    let content = stores.sandbox.project().join("content.md");
    std::fs::write(&content, "Rewritten.\n").expect("content");
    let content = content.to_str().expect("a path").to_owned();
    for (name, arguments) in [
        (
            "status set",
            vec!["task", "status", "set", &landed, "queued"],
        ),
        (
            "priority set",
            vec!["task", "priority", "set", &landed, "low"],
        ),
        (
            "content set",
            vec!["task", "content", "set", &landed, "--file", &content],
        ),
        (
            "metadata set",
            vec![
                "task",
                "metadata",
                "set",
                &landed,
                "example.sync",
                "\"done\"",
            ],
        ),
        (
            "task update",
            vec!["task", "update", &landed, "--title", "Updated"],
        ),
    ] {
        steps.insert(
            name.to_owned(),
            stores.measured(|stores| {
                stores.run(&arguments);
            }),
        );
    }
    Value::Object(steps)
}

/// What a create of a Linear issue onto the board sends: a copy out of a hosted source.
fn create_from_a_hosted_source() -> Value {
    let stores = Stores::new();
    let copied = stores.run(&["task", "copy", "notes:A", "--to", "linear"]);
    let issue = copied["items"][0]["destination"]
        .as_str()
        .expect("an id")
        .to_owned();
    json!({"create": stores.measured(|stores| {
        stores.run(&["task", "copy", &issue, "--to", "board"]);
    })})
}

fn measured_now() -> BTreeMap<String, Value> {
    BTreeMap::from([
        ("into board".to_owned(), copies_and_writes_into("board")),
        ("into linear".to_owned(), copies_and_writes_into("linear")),
        (
            "linear into board".to_owned(),
            create_from_a_hosted_source(),
        ),
    ])
}

#[test]
fn every_task_copy_and_task_write_sends_what_v0_12_13_sent_but_the_one_relist_owed() {
    let now = measured_now();
    if std::env::var_os("OTG_REQUESTS_RECORD").is_some() {
        std::fs::write(
            golden(),
            serde_json::to_string_pretty(&now).expect("renders") + "\n",
        )
        .expect("the golden");
        return;
    }
    let recorded: BTreeMap<String, Value> =
        serde_json::from_str(&std::fs::read_to_string(golden()).expect("the golden"))
            .expect("the golden parses");
    for (scenario, steps) in &recorded {
        for (step, stores) in steps.as_object().expect("steps") {
            for (store, documents) in stores.as_object().expect("stores") {
                // A hosted source's own side of a create is held to a bound, not a total — its
                // intent write, and the link write that removes it — so only the destination of
                // that scenario is compared.
                if scenario == "linear into board" && store == "linear" {
                    continue;
                }
                let mut expected: Vec<Value> = documents.as_array().expect("documents").clone();
                // The one request this release adds: the listing by key after a board create
                // from a hosted source.
                if scenario == "linear into board" && step == "create" && store == "board" {
                    let after = expected
                        .iter()
                        .position(|document| document == "mutation createIssue")
                        .expect("v0.12.13 created the issue");
                    expected.insert(
                        after + 1,
                        json!(spelled(
                            onetaskgraph_github_projects::graphql::ISSUES_BY_CREATION
                        )),
                    );
                }
                // llmlint: ignore[tests_hold_no_nonfunctional_thresholds] The task's acceptance criteria ask for exactly this: the requests each copy and task write sends held to what the v0.12.13 release sent, as README's "What it costs" states them. What is compared is which operations are sent and the fields each selects — the wire behaviour a backend sees — recorded from that release's binary, not a count or a time judged against a limit.
                assert_eq!(
                    now[scenario][step][store],
                    Value::Array(expected),
                    "{scenario} / {step} / {store}: the documents this release sent against \
                     v0.12.13's"
                );
            }
        }
    }
}
