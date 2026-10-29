//! Human task projection into Linear through the binary: `task project-human --input FILE
//! --json` against the loopback Linear workspace — an organization's item kept in its plan's
//! existing project, its canonical issue URL answered, dismissed with and without a reason, a
//! lost create answer bound to its one verified origin match, and a person's edit surviving the
//! terminal projection.

use std::path::PathBuf;
use std::process::{Output, Stdio};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{LinearWorkspace, document, linear_workspace};
use crate::linear::{eng_mapping, held, issue, linear};

const HOST: &str = "linear-projection-host";

/// The plan's existing project the organization's items land in.
const PLAN_PROJECT: &str = "P-1";

struct Store {
    sandbox: Sandbox,
    workspace: LinearWorkspace,
}

impl Store {
    fn new() -> Self {
        let sandbox = Sandbox::new();
        let (config, workspace) = linear_workspace(
            &sandbox,
            held(
                vec![issue("I-1", "Todo", json!({"project": PLAN_PROJECT}))],
                vec![
                    json!({"id": PLAN_PROJECT, "title": "The organization's plan",
                    "content": "The plan.", "labels": [],
                    "status": {"category": "in-progress", "name": "Doing"}}),
                ],
                vec![],
            ),
        );
        let intents: PathBuf = sandbox.subdirectory("intents");
        let mut org = linear(&config, json!({"status_mapping": eng_mapping()}));
        org["task_projection"] = json!({"host": HOST, "directory": intents});
        org["cache"] = json!({"dir": sandbox.subdirectory("cache-org")});
        sandbox.project_document(&document(&json!({"org": org})));
        Self { sandbox, workspace }
    }

    fn file(&self, request: &Value) -> PathBuf {
        let path = self.sandbox.project().join(format!(
            "request-{}-{}.json",
            request["item_id"].as_str().unwrap(),
            request["state"].as_str().unwrap()
        ));
        std::fs::write(&path, request.to_string()).unwrap();
        path
    }

    fn project(&self, request: &Value) -> Output {
        self.sandbox
            .command()
            .args(["task", "project-human", "--input"])
            .arg(self.file(request))
            .arg("--json")
            .env("ONETASKGRAPH_HOST", HOST)
            .output()
            .unwrap()
    }

    /// Project `request`, the binary killed once the workspace applied the first request
    /// `matches` holds of, its answer never reaching it.
    fn crashed(&self, request: &Value, operation: &'static str) {
        let mut command = self.sandbox.subprocess(onetaskgraph_e2e_support::binary());
        command
            .args(["task", "project-human", "--input"])
            .arg(self.file(request))
            .arg("--json")
            .env("ONETASKGRAPH_HOST", HOST)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let child = Arc::new(Mutex::new(command.spawn().unwrap()));
        let killed = Arc::clone(&child);
        self.workspace.after_request(
            move |query: &str, _: &Value| query.contains(operation),
            0,
            move |_| {
                let _ = killed.lock().unwrap().kill();
            },
        );
        let status = loop {
            if let Some(status) = child.lock().unwrap().try_wait().unwrap() {
                break status;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        assert!(!status.success(), "the binary finished before {operation}");
    }

    /// One task as `task show --json` reads it back.
    fn shown(&self, id: &str) -> Value {
        let output = self
            .sandbox
            .command()
            .args(["--json", "task", "show", id])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        serde_json::from_str::<Value>(&stdout(&output)).unwrap()["items"][0]["item"].clone()
    }

    /// Every issue the workspace holds whose metadata names `item_id` as its human item.
    fn tickets(&self, item_id: &str) -> usize {
        let output = self
            .sandbox
            .command()
            .args(["--json", "task", "list", "--source", "org", "--metadata"])
            .arg(format!("human.item_id={item_id}"))
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        serde_json::from_str::<Value>(&stdout(&output)).unwrap()["items"]
            .as_array()
            .unwrap()
            .len()
    }

    fn creates_since(&self, from: usize) -> usize {
        self.workspace.served()[from..]
            .iter()
            .filter(|(query, _)| query.contains("issueCreate"))
            .count()
    }
}

/// An organization's open item for its plan's project.
fn request(item_id: &str) -> Value {
    json!({"schema_version": 1, "item_id": item_id,
        "owner": {"session": "session-1", "run": "run-1", "plan": "plan-1"},
        "title": format!("Decide on {item_id}"), "body": "Choose the release date.",
        "priority": "urgent", "repositories": ["github.com/orgco/service"],
        "project": format!("org:{PLAN_PROJECT}"), "source": "org", "state": "open",
        "reason": null})
}

fn projected(output: &Output) -> Value {
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}\n{}",
        stdout(output),
        stderr(output)
    );
    serde_json::from_str(&stdout(output)).unwrap()
}

#[test]
fn an_organization_item_lands_in_its_plans_project_and_is_dismissed_with_and_without_a_reason() {
    let store = Store::new();
    for (item, reason) in [
        ("org-plain", None),
        ("org-reasoned", Some("The customer withdrew the request.")),
    ] {
        let open = request(item);
        let answer = projected(&store.project(&open));
        assert_eq!(answer["source"], "org");
        assert_eq!(answer["project"], PLAN_PROJECT);
        let url = answer["url"].as_str().unwrap();
        assert!(
            url.starts_with("https://linear.app/") && url.contains("/issue/"),
            "{answer}"
        );
        let task = answer["task"].as_str().unwrap();
        let shown = store.shown(task);
        assert_eq!(shown["project"], PLAN_PROJECT, "kept in the plan's project");
        assert_eq!(shown["priority"], "urgent");
        assert_eq!(shown["url"], url, "the card's URL is the issue's own");
        assert_eq!(shown["metadata"]["human.item_id"], item);

        // A retry of the bound item is the same answer and sends nothing.
        let before = store.workspace.served().len();
        assert_eq!(projected(&store.project(&open)), answer);
        assert_eq!(store.workspace.served().len(), before);

        let mut dismissed = open.clone();
        dismissed["state"] = json!("dismissed");
        dismissed["reason"] = json!(reason);
        assert_eq!(projected(&store.project(&dismissed)), answer);
        let shown = store.shown(task);
        assert_eq!(shown["status"]["category"], "cancelled");
        assert_eq!(
            shown["metadata"]
                .get("human.reason")
                .and_then(Value::as_str),
            reason,
            "{item}"
        );
        // A different terminal state is refused, writing nothing.
        let mut completed = open;
        completed["state"] = json!("completed");
        let before = store.workspace.served().len();
        let refused = store.project(&completed);
        assert_ne!(refused.status.code(), Some(0));
        assert!(
            stderr(&refused).contains("conflicting terminal"),
            "{}",
            stderr(&refused)
        );
        assert_eq!(store.workspace.served().len(), before);
        assert_eq!(store.tickets(item), 1);
    }
}

#[test]
fn a_lost_create_answer_is_bound_to_its_one_verified_origin_match_in_linear() {
    let store = Store::new();
    let open = request("org-lost");
    store.crashed(&open, "issueCreate");
    assert_eq!(store.tickets("org-lost"), 1);
    let before = store.workspace.served().len();
    let answer = projected(&store.project(&open));
    assert_eq!(store.creates_since(before), 0, "no second create");
    assert_eq!(store.tickets("org-lost"), 1);
    assert_eq!(answer["project"], PLAN_PROJECT);
    let mut completed = open;
    completed["state"] = json!("completed");
    assert_eq!(projected(&store.project(&completed)), answer);
    assert_eq!(
        store.shown(answer["task"].as_str().unwrap())["status"]["category"],
        "done"
    );
}

#[test]
fn an_edit_a_person_makes_in_linear_survives_the_terminal_projection() {
    let store = Store::new();
    let open = request("org-edited");
    let answer = projected(&store.project(&open));
    let task = answer["task"].as_str().unwrap();
    let native = task.split_once(':').unwrap().1;
    store
        .workspace
        .edit_description(native, "A person rewrote this in Linear.");
    let mut dismissed = open;
    dismissed["state"] = json!("dismissed");
    dismissed["reason"] = json!("No longer needed.");
    projected(&store.project(&dismissed));
    let shown = store.shown(task);
    assert_eq!(
        shown["content"], "A person rewrote this in Linear.",
        "{shown}"
    );
    assert_eq!(shown["metadata"]["human.reason"], "No longer needed.");
}

#[test]
fn one_origin_match_with_another_page_behind_it_is_refused_rather_than_bound() {
    let store = Store::new();
    let open = request("org-paged");
    store.crashed(&open, "issueCreate");
    assert_eq!(store.tickets("org-paged"), 1);
    // The origin lookup answers its one match and says another page follows: that page is no
    // whole answer, so the match is not bound and no second create is sent.
    store.workspace.bend_answers(
        |query: &str, variables: &Value| {
            query.contains("issues(")
                && variables
                    .pointer("/filter/and/1/description/contains")
                    .and_then(Value::as_str)
                    .is_some_and(|phrase| phrase.contains("human-task:"))
        },
        1,
        |_, data: &mut Value| {
            data["issues"]["pageInfo"] = json!({"hasNextPage": true, "endCursor": "more"});
        },
    );
    let before = store.workspace.served().len();
    let refused = store.project(&open);
    assert_ne!(refused.status.code(), Some(0), "{}", stdout(&refused));
    assert!(
        stderr(&refused).contains("zero or multiple origin matches"),
        "{}",
        stderr(&refused)
    );
    assert_eq!(store.creates_since(before), 0, "no second create");
    assert_eq!(store.tickets("org-paged"), 1);

    // Answered whole, the same match binds.
    let answer = projected(&store.project(&open));
    assert_eq!(answer["project"], PLAN_PROJECT);
    assert_eq!(store.tickets("org-paged"), 1);
}
