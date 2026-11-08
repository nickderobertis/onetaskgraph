//! A copy into Linear finds what an earlier copy wrote there however soon after it runs, through
//! the binary, against a loopback Linear whose filtered and searched reads lag its writes.
//!
//! The plan is the shape that missed on the real `TES2` team: a folder of Markdown holding one
//! project, two tasks each under a budget heading, a design document whose budget table links
//! each heading by a reference-style definition and links the project's other document — copied
//! into a Linear source declared private. The folder is not, so the copy records no link and no
//! intent in it, and every later copy finds its counterparts only by asking Linear. The loopback
//! is put in a lag window ([`LinearWorkspace::lag_listings`]): each record created inside it is
//! missing from every `issues`, `projects` and `documents` listing, a copy's batched lookup by
//! origin among them, while a read by its id answers it.
//!
//! What finds the counterpart is the read by id of the ids the store's own create key derives
//! for that origin, which every create of a copy carrying an origin is sent under; see the
//! README's "How a copy to Linear finds what it wrote".

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::{Value, json};

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{LinearWorkspace, document, linear_workspace_with};

const TEAM_STATES: &[(&str, &str, &str)] = &[
    ("S-ready", "Ready", "unstarted"),
    ("S-working", "Working", "started"),
    ("S-finished", "Finished", "completed"),
];

const PROJECT_STATUSES: &[(&str, &str)] = &[("Scheduled", "planned"), ("Shipped", "completed")];

/// The project copy every journey here makes.
const PROJECT_COPY: [&str; 6] = ["project", "copy", "notes:P", "--to", "dest", "--json"];

/// The document copy, of both of the project's documents.
const DOCUMENT_COPY: [&str; 7] = [
    "document", "copy", "notes:D", "notes:E", "--to", "dest", "--json",
];

/// One sandbox: the plan in a folder of Markdown called `notes`, and a loopback Linear called
/// `dest` declared private.
struct Plan {
    sandbox: Sandbox,
    workspace: LinearWorkspace,
    notes: PathBuf,
}

impl Plan {
    fn new() -> Self {
        let sandbox = Sandbox::new();
        let held = json!({"tasks": [], "projects": [], "documents": [], "labels": [],
                          "task_dependencies": [], "project_dependencies": []});
        let (mut config, workspace) =
            linear_workspace_with(&sandbox, held, TEAM_STATES, PROJECT_STATUSES);
        config["status_mapping"] = json!({
            "todo": {"task": "Ready", "project": "Scheduled"},
            "done": {"task": "Finished", "project": "Shipped"},
        });
        let notes = sandbox.subdirectory("notes");
        for (path, text) in [
            (
                "projects/P.md",
                "---\ntitle: Plan\nstatus: todo\n---\nThe plan.\n",
            ),
            (
                "tasks/first.md",
                "---\ntitle: First\nstatus: todo\nproject: P\n---\n## Budget requests\n\nAt most \
                 twenty.\n",
            ),
            (
                "tasks/second.md",
                "---\ntitle: Second\nstatus: todo\nproject: P\n---\n## Budget seconds\n\nAt most \
                 thirty.\n",
            ),
            (
                "documents/D.md",
                "---\ntitle: Design\nproject: P\n---\n## Budgets\n\n| Budget | Limit |\n| --- | --- \
                 |\n| [requests][1] | 20 |\n| [seconds][2] | 30 |\n\n[1]: \
                 ../tasks/first.md#budget-requests\n[2]: ../tasks/second.md#budget-seconds\n\nThe \
                 rest is in [the second document](E.md).\n",
            ),
            (
                "documents/E.md",
                "---\ntitle: Second document\nproject: P\n---\nBack to [the design](D.md).\n",
            ),
        ] {
            let path = notes.join(path);
            std::fs::create_dir_all(path.parent().expect("a folder")).expect("a folder");
            std::fs::write(path, text).expect("a record");
        }
        sandbox.project_document(&document(&json!({
            "notes": {"plugin": "local-md", "config": {"root": notes}},
            "dest": {"plugin": "linear", "config": config, "visibility": "private"},
        })));
        Self {
            sandbox,
            workspace,
            notes,
        }
    }

    /// Run the binary with this host's own state under `state`; what it printed and every
    /// request the workspace answered while it ran.
    fn run_on(&self, state: &Path, arguments: &[&str]) -> (Output, Vec<(String, Value)>) {
        let from = self.workspace.served().len();
        let output = self
            .sandbox
            .command()
            .env("XDG_CACHE_HOME", state.join("cache"))
            .env("XDG_STATE_HOME", state.join("state"))
            .env("XDG_DATA_HOME", state.join("data"))
            .args(arguments)
            .assert()
            .get_output()
            .clone();
        (output, self.workspace.served()[from..].to_vec())
    }

    /// [`Self::run_on`] this sandbox's one host state, which has to succeed, answering its JSON.
    fn ok(&self, arguments: &[&str]) -> (Value, Vec<(String, Value)>) {
        self.ok_on(&self.sandbox.subdirectory("host"), arguments)
    }

    fn ok_on(&self, state: &Path, arguments: &[&str]) -> (Value, Vec<(String, Value)>) {
        let (output, served) = self.run_on(state, arguments);
        assert!(
            output.status.success(),
            "`onetaskgraph {}`\nstdout:\n{}\nstderr:\n{}",
            arguments.join(" "),
            stdout(&output),
            stderr(&output)
        );
        (
            serde_json::from_str(&stdout(&output)).expect("JSON"),
            served,
        )
    }

    /// Every file of the folder, by its path, byte for byte.
    fn notes_files(&self) -> BTreeMap<PathBuf, Vec<u8>> {
        fn walk(at: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
            for entry in std::fs::read_dir(at).expect("a folder") {
                let path = entry.expect("an entry").path();
                if path.is_dir() {
                    walk(&path, files);
                } else {
                    files.insert(path.clone(), std::fs::read(&path).expect("a file"));
                }
            }
        }
        let mut files = BTreeMap::new();
        walk(&self.notes, &mut files);
        files
    }

    /// The item `onetaskgraph <kind> show <id> --json` prints.
    fn show(&self, kind: &str, id: &str) -> Value {
        let shown = self.ok(&[kind, "show", id, "--json"]).0;
        match shown.pointer("/items/0/item") {
            Some(item) => item.clone(),
            None => shown,
        }
    }
}

/// Each copied item's action, by the qualified id it was copied from.
fn actions(report: &Value) -> BTreeMap<String, String> {
    report["items"]
        .as_array()
        .expect("the copy's items")
        .iter()
        .map(|item| {
            (
                item["source"].as_str().expect("a source").to_owned(),
                item["action"].as_str().expect("an action").to_owned(),
            )
        })
        .collect()
}

/// The destination id a copy gave the item copied from `source`.
fn landed(report: &Value, source: &str) -> String {
    report["items"]
        .as_array()
        .expect("the copy's items")
        .iter()
        .find(|item| item["source"] == source)
        .and_then(|item| item["destination"].as_str())
        .unwrap_or_else(|| panic!("{source} landed: {report}"))
        .to_owned()
}

/// The root field of every mutation among `served`.
fn mutations(served: &[(String, Value)]) -> Vec<String> {
    served
        .iter()
        .filter(|(query, _)| query.trim_start().starts_with("mutation"))
        .map(|(query, _)| {
            query
                .split_once('{')
                .and_then(|(_, rest)| rest.trim_start().split(['(', ' ', '{']).next())
                .unwrap_or_default()
                .to_owned()
        })
        .collect()
}

/// Every create among `served`.
fn creates(served: &[(String, Value)]) -> Vec<String> {
    mutations(served)
        .into_iter()
        .filter(|root| root.ends_with("Create") && !root.contains("Relation"))
        .collect()
}

fn every(report: &Value, action: &str) -> bool {
    actions(report).values().all(|held| held == action)
}

#[test]
fn a_document_copied_at_once_after_its_project_resolves_every_link_while_listings_lag() {
    let plan = Plan::new();
    plan.workspace.lag_listings();
    let (project, _) = plan.ok(&PROJECT_COPY);
    assert!(every(&project, "created"), "{project}");
    let (documents, _) = plan.ok(&DOCUMENT_COPY);
    assert!(every(&documents, "created"), "{documents}");
    assert_eq!(
        documents.get("references_unresolved"),
        None,
        "every link the design carries resolved: {documents}"
    );

    let address = |kind: &str, id: &str| {
        plan.show(kind, id)["location"]["url"]
            .as_str()
            .unwrap_or_else(|| panic!("{id} reports where it is"))
            .to_owned()
    };
    let first = address("task", &landed(&project, "notes:first"));
    let second = address("task", &landed(&project, "notes:second"));
    let other = address("document", &landed(&documents, "notes:E"));
    let design = plan.show("document", &landed(&documents, "notes:D"))["content"]
        .as_str()
        .expect("the design's content")
        .to_owned();
    for (what, wanted) in [
        (
            "the requests budget",
            format!("({first}#heading-budget-requests-"),
        ),
        (
            "the seconds budget",
            format!("({second}#heading-budget-seconds-"),
        ),
        ("the second document", format!("({other})")),
    ] {
        assert!(
            design.contains(&wanted),
            "{what} links its counterpart: {design}"
        );
    }
    assert!(
        !design.contains("../tasks/") && !design.contains("E.md"),
        "no link is left as authored: {design}"
    );
}

#[test]
fn a_repeat_project_copy_straight_after_the_first_writes_nothing_while_listings_lag() {
    let plan = Plan::new();
    plan.workspace.lag_listings();
    let (first, _) = plan.ok(&PROJECT_COPY);
    assert!(every(&first, "created"), "{first}");
    let (repeat, served) = plan.ok(&PROJECT_COPY);
    assert_eq!(mutations(&served), Vec::<String>::new(), "{repeat}");
    assert!(every(&repeat, "unchanged"), "{repeat}");
    for source in ["notes:P", "notes:first", "notes:second"] {
        assert_eq!(landed(&repeat, source), landed(&first, source), "{source}");
    }

    // The documents too: copied once, and again at once, inside the same window.
    let (documents, _) = plan.ok(&DOCUMENT_COPY);
    assert!(every(&documents, "created"), "{documents}");
    let (again, served) = plan.ok(&DOCUMENT_COPY);
    assert_eq!(creates(&served), Vec::<String>::new(), "{again}");
    for source in ["notes:D", "notes:E"] {
        assert_eq!(
            landed(&again, source),
            landed(&documents, source),
            "{source}"
        );
    }
}

#[test]
fn a_repeat_after_the_lag_window_has_passed_writes_nothing() {
    let plan = Plan::new();
    plan.workspace.lag_listings();
    let (first, _) = plan.ok(&PROJECT_COPY);
    let (documents, _) = plan.ok(&DOCUMENT_COPY);
    // Repeated at once, inside the window, before it passes.
    plan.ok(&PROJECT_COPY);
    plan.ok(&DOCUMENT_COPY);
    plan.workspace.settle_listings();
    let (repeat, served) = plan.ok(&PROJECT_COPY);
    assert_eq!(mutations(&served), Vec::<String>::new(), "{repeat}");
    assert!(every(&repeat, "unchanged"), "{repeat}");
    for source in ["notes:P", "notes:first", "notes:second"] {
        assert_eq!(landed(&repeat, source), landed(&first, source), "{source}");
    }
    let (again, served) = plan.ok(&DOCUMENT_COPY);
    assert_eq!(mutations(&served), Vec::<String>::new(), "{again}");
    for source in ["notes:D", "notes:E"] {
        assert_eq!(
            landed(&again, source),
            landed(&documents, source),
            "{source}"
        );
    }
    // Listed now the window has passed, the workspace holds one record of each item and no more.
    for (kind, held) in [("project", 1), ("task", 2), ("document", 2)] {
        let listed = plan.ok(&[kind, "list", "--source", "dest", "--json"]).0;
        assert_eq!(
            listed["items"].as_array().map(Vec::len),
            Some(held),
            "{kind}s: {listed}"
        );
    }
}

#[test]
fn a_repeat_on_a_host_holding_nothing_of_the_earlier_copy_creates_nothing_while_listings_lag() {
    let plan = Plan::new();
    let before = plan.notes_files();
    plan.workspace.lag_listings();
    let (first, _) = plan.ok_on(&plan.sandbox.subdirectory("first-host"), &PROJECT_COPY);
    assert!(every(&first, "created"), "{first}");
    assert_eq!(
        plan.notes_files(),
        before,
        "a copy into a source declared private records nothing in the folder it copied"
    );
    // Another host: no cache, state or data directory of the first, and nothing in the folder.
    let (repeat, served) = plan.ok_on(&plan.sandbox.subdirectory("second-host"), &PROJECT_COPY);
    assert_eq!(creates(&served), Vec::<String>::new(), "{repeat}");
    assert_eq!(mutations(&served), Vec::<String>::new(), "{repeat}");
    for source in ["notes:P", "notes:first", "notes:second"] {
        assert_eq!(landed(&repeat, source), landed(&first, source), "{source}");
    }
}

/// The issue a copy creates of `origin` into the source `dest` is sent under the id the store's
/// create key for that origin derives — the README's rule — whether or not the copy recorded an
/// intent, so a later copy reads it by that id.
#[test]
fn a_copied_task_lands_under_the_id_its_origins_create_key_derives() {
    let plan = Plan::new();
    let (first, _) = plan.ok(&PROJECT_COPY);
    let key = uuid::Uuid::new_v5(
        &uuid::Uuid::from_u128(0x7a1c_3f0e_5b2d_4c8a_9e61_0f4d_2b7c_5a39),
        b"notes:first|dest|",
    )
    .hyphenated()
    .to_string();
    assert_eq!(
        landed(&first, "notes:first"),
        format!(
            "dest:{}",
            onetaskgraph_linear::linear_id_of_key(&key).expect("a key")
        )
    );
}

/// A copy whose lookup cannot show its counterpart absent creates nothing: here the issue under
/// the origin key's first id was trashed, its successor under the second id is inside the lag
/// window, and the read by id stops at the trashed one — so the create is sent, refused for the
/// id each generation's record holds, and the copy is refused naming the counterpart it found.
#[test]
fn a_create_whose_id_holds_its_counterpart_is_refused_naming_it_and_creates_nothing() {
    let plan = Plan::new();
    plan.workspace.lag_listings();
    let (first, _) = plan.ok(&PROJECT_COPY);
    let trashed = landed(&first, "notes:first");
    plan.workspace
        .trash_issue(trashed.trim_start_matches("dest:"));
    let (second, _) = plan.ok(&PROJECT_COPY);
    assert_eq!(actions(&second)["notes:first"], "created", "{second}");
    let successor = landed(&second, "notes:first");
    assert_ne!(successor, trashed);

    let (output, served) = plan.run_on(&plan.sandbox.subdirectory("host"), &PROJECT_COPY);
    assert!(!output.status.success(), "{}", stdout(&output));
    let said = format!("{}{}", stdout(&output), stderr(&output));
    assert!(
        said.contains(successor.trim_start_matches("dest:"))
            && said.contains("notes:first")
            && said.contains("nothing was created"),
        "the refusal names the counterpart it found: {said}"
    );
    let landed_creates: Vec<String> = served
        .iter()
        .zip(plan.workspace.answered()[plan.workspace.answered().len() - served.len()..].iter())
        .filter(|((query, _), answer)| {
            query.trim_start().starts_with("mutation")
                && query.contains("Create(")
                && answer.get("errors").is_none()
                && !query.contains("Relation")
        })
        .map(|((query, _), _)| query.clone())
        .collect();
    assert_eq!(landed_creates, Vec::<String>::new(), "nothing was created");
}
