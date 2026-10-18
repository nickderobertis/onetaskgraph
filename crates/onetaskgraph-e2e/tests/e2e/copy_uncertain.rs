//! A copy whose create the destination answers malformed, driven through the binary.
//!
//! A create is journalled by the id its answer carries, so a plugin that stores an item and
//! then answers in a shape the engine refuses has kept it under an id the copy never learned.
//! Each journey here copies into `asset_store.py` over a real pipe, set to do exactly that for
//! one title, after the same command has already landed writes it can identify: the copy fails
//! naming the destination and the method that answered, warns that the destination may hold an
//! item it could not take back, and every earlier write is gone when it returns.

use std::path::Path;
use std::process::Output;

use serde_json::{Value, json};

use crate::assets::store;
use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::document;
use onetaskgraph_e2e_support::images;

/// The title the destination stores and then answers without its id.
const DOOMED: &str = "Doomed";
/// The home project's title, which a routed copy gives the member project it creates.
const LAUNCH: &str = "Launch";

/// A folder of Markdown — `notes` — holding project `launch`; `careless`, an asset store over a
/// real pipe that answers a create of `Doomed` or `Launch` without its id; and `routing`, a
/// folder of Markdown routing `github.com/widgetco/*` to `careless`.
struct Careless {
    sandbox: Sandbox,
    notes: std::path::PathBuf,
    routing: std::path::PathBuf,
    log: std::path::PathBuf,
}

impl Careless {
    fn new() -> Self {
        let sandbox = Sandbox::new();
        let notes = sandbox.subdirectory("notes");
        let routing = sandbox.subdirectory("routing");
        let stores = sandbox.subdirectory("stores");
        let log = stores.join("careless.log");
        sandbox.project_document(&document(&json!({
            "notes": {"plugin": "local-md", "config": {"root": notes}},
            "routing": {
                "plugin": "local-md",
                "config": {"root": routing},
                "routes": [{"repositories": ["github.com/widgetco/*"], "to": "careless"}],
            },
            "careless": store(
                &stores.join("careless.json"),
                &log,
                json!({"assets": "native", "malformed_creates": [DOOMED, LAUNCH]}),
            ),
        })));
        let careless = Self {
            sandbox,
            notes,
            routing,
            log,
        };
        careless.record(
            "projects",
            "launch",
            &format!("title: {LAUNCH}\nstatus: todo"),
        );
        careless
    }

    /// Write one record of `kind` into `notes`.
    fn record(&self, kind: &str, id: &str, front: &str) {
        let path = self.notes.join(kind).join(format!("{id}.md"));
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("the folder");
        std::fs::write(&path, format!("---\n{front}\n---\nBody of {id}.\n")).expect("a record");
    }

    fn run(&self, arguments: &[&str]) -> Output {
        self.sandbox
            .command()
            .args(arguments)
            .assert()
            .get_output()
            .clone()
    }

    /// Standard error of a run that failed with exit `1`.
    fn refused(&self, arguments: &[&str]) -> String {
        let output = self.run(arguments);
        assert_eq!(
            output.status.code(),
            Some(1),
            "`onetaskgraph {}`\nstdout:\n{}\nstderr:\n{}",
            arguments.join(" "),
            stdout(&output),
            stderr(&output)
        );
        stderr(&output)
    }

    /// The titles `<verb> list --source <source>` reads back, sorted.
    fn titles(&self, verb: &str, source: &str) -> Vec<String> {
        let output = self.run(&[verb, "list", "--source", source, "--json"]);
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        let listed: Value = serde_json::from_str(&stdout(&output)).expect("a listing is JSON");
        let mut titles: Vec<String> = listed["items"]
            .as_array()
            .expect("a listing has items")
            .iter()
            .map(|entry| entry["item"]["title"].as_str().expect("a title").to_owned())
            .collect();
        titles.sort();
        titles
    }

    /// Every write and delete `careless` was sent, in order, as its log recorded it.
    fn sent(&self) -> Vec<Value> {
        std::fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).expect("a log line is JSON"))
            .collect()
    }
}

/// What every one of these failures owes on standard error: the destination, the method whose
/// answer was malformed, the warning, and a next action to inspect it before copying again.
fn warns(said: &str, destination: &str, method: &str) {
    for wanted in [
        format!("{destination} may hold an item it could not take back"),
        format!("{destination}'s answer to {method} was malformed"),
        "every write it could identify was undone".to_owned(),
        format!("next: inspect {destination} for an item this copy may have created"),
        "before copying again".to_owned(),
    ] {
        assert!(said.contains(&wanted), "wanted {wanted:?} in:\n{said}");
    }
    assert!(
        said.trim_end().ends_with("before copying again."),
        "the next action is the inspection, last:\n{said}"
    );
    assert!(
        !said.contains("next: fix what the source named above, then copy again"),
        "a copy that may have left residue does not say to copy again first:\n{said}"
    );
}

/// How many creates `careless` was sent, and how many of them it was asked to take back.
fn creates_and_deletes(sent: &[Value], method: &str) -> (usize, usize) {
    let creates = sent
        .iter()
        .filter(|entry| entry["method"] == json!(method) && entry["target"].is_null())
        .count();
    let deletes = sent
        .iter()
        .filter(|entry| {
            entry["method"]
                .as_str()
                .is_some_and(|sent| sent.starts_with("delete_"))
        })
        .count();
    (creates, deletes)
}

#[test]
fn a_task_create_answered_malformed_names_the_destination_and_undoes_the_task_before_it() {
    let careless = Careless::new();
    careless.record(
        "tasks",
        "first",
        "title: First\nstatus: todo\nproject: launch",
    );
    careless.record(
        "tasks",
        "second",
        &format!("title: {DOOMED}\nstatus: todo\nproject: launch"),
    );
    let said = careless.refused(&[
        "task",
        "copy",
        "notes:first",
        "notes:second",
        "--to",
        "careless",
    ]);
    warns(&said, "careless", "write_task");
    // `First` landed and was identified, so it is gone; `Doomed` is the residue the copy warned
    // of, stored under an id it was never told.
    assert_eq!(careless.titles("task", "careless"), vec![DOOMED.to_owned()]);
    assert_eq!(creates_and_deletes(&careless.sent(), "write_task"), (2, 1));
}

#[test]
fn a_document_create_answered_malformed_names_the_destination_and_undoes_the_one_before_it() {
    let careless = Careless::new();
    careless.record("documents", "first", "title: First\nproject: launch");
    careless.record(
        "documents",
        "second",
        &format!("title: {DOOMED}\nproject: launch"),
    );
    let said = careless.refused(&[
        "document",
        "copy",
        "notes:first",
        "notes:second",
        "--to",
        "careless",
    ]);
    warns(&said, "careless", "write_document");
    assert_eq!(
        careless.titles("document", "careless"),
        vec![DOOMED.to_owned()]
    );
    assert_eq!(
        creates_and_deletes(&careless.sent(), "write_document"),
        (2, 1)
    );
}

#[test]
fn an_asset_write_answered_malformed_names_the_destination_and_undoes_the_one_before_it() {
    let careless = Careless::new();
    let inputs = careless.sandbox.subdirectory("inputs");
    let screen = inputs.join("screen.png");
    std::fs::write(&screen, images::png(7, 60_000)).expect("an input image");
    let body = inputs.join("body.md");
    std::fs::write(&body, "The change:\n\n![screen](./screen.png)\n").expect("a body");
    let created = |title: &str| {
        let output = careless.run(&[
            "task",
            "create",
            "notes",
            "--project",
            "launch",
            "--title",
            title,
            "--body-file",
            spelled(&body),
            "--asset",
            spelled(&screen),
        ]);
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        stdout(&output).trim().to_owned()
    };
    let first = created("First");
    let doomed = created(DOOMED);
    let said = careless.refused(&["task", "copy", &first, &doomed, "--to", "careless"]);
    warns(&said, "careless", "write_task");
    assert_eq!(careless.titles("task", "careless"), vec![DOOMED.to_owned()]);
    let sent = careless.sent();
    // Both creates carried the asset, so it is the asset write whose answer was malformed.
    let creates: Vec<&Value> = sent
        .iter()
        .filter(|entry| entry["method"] == json!("write_task") && entry["target"].is_null())
        .collect();
    assert_eq!(creates.len(), 2, "{sent:?}");
    for create in &creates {
        assert!(
            create["members"]
                .as_array()
                .is_some_and(|members| members.contains(&json!("assets"))),
            "{create}"
        );
    }
    assert!(creates[1]["answered"].get("id").is_none(), "{}", creates[1]);
    assert_eq!(creates_and_deletes(&sent, "write_task"), (2, 1));
}

#[test]
fn a_project_create_answered_malformed_names_the_destination_and_undoes_the_home_before_it() {
    // A routed copy lands the home in `routing` and creates the member project the widgetco
    // task is filed under in `careless`; that create is the one answered malformed, after the
    // home and its other task have landed in `routing`.
    let careless = Careless::new();
    careless.record(
        "tasks",
        "a-plain",
        "title: Plain\nstatus: todo\nproject: launch\nrepositories: [github.com/nickderobertis/lib]",
    );
    careless.record(
        "tasks",
        "b-widget",
        "title: Widget\nstatus: todo\nproject: launch\nrepositories: [github.com/widgetco/app]",
    );
    let said = careless.refused(&["project", "copy", "notes:launch", "--to", "routing"]);
    warns(&said, "careless", "write_project");
    // Everything `routing` was written is gone: it holds no project and no task.
    assert_eq!(careless.titles("project", "routing"), Vec::<String>::new());
    assert_eq!(careless.titles("task", "routing"), Vec::<String>::new());
    assert!(
        files(&careless.routing).is_empty(),
        "nothing of the copy is left in routing: {:?}",
        files(&careless.routing)
    );
    // The member project is the residue the copy warned of.
    assert_eq!(
        careless.titles("project", "careless"),
        vec![LAUNCH.to_owned()]
    );
    assert_eq!(careless.titles("task", "careless"), Vec::<String>::new());
}

#[test]
fn a_malformed_answer_to_a_read_keeps_its_own_failure() {
    // A non-mutating call answered malformed is not a write the copy cannot vouch for: the
    // destination's store is not a store at all, so its first read fails as it always has.
    let careless = Careless::new();
    careless.record(
        "tasks",
        "first",
        "title: First\nstatus: todo\nproject: launch",
    );
    let store = careless
        .sandbox
        .subdirectory("stores")
        .join("careless.json");
    std::fs::write(&store, "not json").expect("an unreadable store");
    let said = careless.refused(&["task", "copy", "notes:first", "--to", "careless"]);
    assert!(said.contains("is not JSON"), "{said}");
    assert!(!said.contains("may hold an item"), "{said}");
    assert!(
        said.contains("next: fix what the source named above, then copy again"),
        "{said}"
    );
}

fn spelled(path: &Path) -> &str {
    path.to_str().expect("a UTF-8 path")
}

/// Every file under `root`, relative to it.
fn files(root: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries {
            let path = entry.expect("an entry").path();
            if path.is_dir() {
                pending.push(path);
            } else {
                found.push(path.display().to_string());
            }
        }
    }
    found
}
