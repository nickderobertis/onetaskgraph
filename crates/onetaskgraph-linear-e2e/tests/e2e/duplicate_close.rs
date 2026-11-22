//! Closing an issue as a duplicate of another, through the binary, against the loopback Linear:
//! Linear's own duplicate relation from the duplicate to its survivor, then the `cancelled` the
//! close implies at the state the mapping names it, and the typed `duplicate` link the store
//! records at both ends — so a later read reports the issue `cancelled` and `task relations`
//! names its survivor. A survivor in another backend is refused before anything is written.

use serde_json::{Value, json};

use crate::cache::dataset;
use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{LinearWorkspace, document, linear_workspace};

struct Setup {
    sandbox: Sandbox,
    workspace: LinearWorkspace,
}

impl Setup {
    fn new() -> Self {
        let sandbox = Sandbox::new();
        let notes = sandbox.subdirectory("notes");
        std::fs::create_dir_all(notes.join("tasks")).unwrap();
        std::fs::write(
            notes.join("tasks/N-1.md"),
            "---\ntitle: Elsewhere\nstatus: todo\n---\nIn another backend.\n",
        )
        .unwrap();
        let (config, workspace) = linear_workspace(&sandbox, dataset());
        sandbox.project_document(&document(&json!({
            "linear": {"plugin": "linear", "config": config},
            "notes": {"plugin": "local-md", "config": {"root": notes}},
        })));
        Self { sandbox, workspace }
    }

    fn ok(&self, arguments: &[&str]) -> Value {
        let output = self
            .sandbox
            .command()
            .args(arguments)
            .assert()
            .get_output()
            .clone();
        assert_eq!(
            output.status.code(),
            Some(0),
            "`onetaskgraph {}` failed\n{}",
            arguments.join(" "),
            stderr(&output)
        );
        serde_json::from_str(&stdout(&output)).expect("JSON")
    }

    fn mutations(&self) -> usize {
        self.workspace
            .served()
            .iter()
            .filter(|(query, _)| query.trim_start().starts_with("mutation"))
            .count()
    }
}

#[test]
fn a_status_set_closes_an_issue_as_a_duplicate_with_linear_s_relation_and_its_link() {
    let setup = Setup::new();
    let answer = setup.ok(&[
        "--json",
        "task",
        "status",
        "set",
        "linear:L-3",
        "cancelled",
        "--duplicate-of",
        "linear:L-2",
    ]);
    assert_eq!(answer["status"]["category"], "cancelled", "{answer:#}");
    assert_eq!(
        setup.workspace.duplicates(),
        [("L-3".to_owned(), "L-2".to_owned())],
        "Linear's own duplicate relation"
    );
    let shown = setup.ok(&["--json", "task", "show", "linear:L-3"]);
    assert_eq!(
        shown["items"][0]["item"]["status"]["category"], "cancelled",
        "{shown:#}"
    );
    let relations = setup.ok(&["task", "relations", "linear:L-3", "linear:L-2", "--json"]);
    assert_eq!(
        relations["items"][0]["links"],
        json!([{"type": "duplicate", "direction": "out", "other": "linear:L-2",
                "other_end": "present"}]),
        "{relations:#}"
    );
}

#[test]
fn a_duplicate_close_beside_another_status_or_of_another_backend_s_task_writes_nothing() {
    let setup = Setup::new();
    for (arguments, said) in [
        (
            vec![
                "task",
                "status",
                "set",
                "linear:L-3",
                "done",
                "--duplicate-of",
                "linear:L-2",
            ],
            "cancelled",
        ),
        (
            vec![
                "task",
                "update",
                "linear:L-3",
                "--duplicate-of",
                "notes:N-1",
            ],
            "notes:N-1",
        ),
    ] {
        let output = setup
            .sandbox
            .command()
            .args(&arguments)
            .assert()
            .get_output()
            .clone();
        assert_ne!(output.status.code(), Some(0), "{arguments:?}");
        assert!(
            stderr(&output).contains(said),
            "{arguments:?}: {}",
            stderr(&output)
        );
    }
    assert_eq!(setup.mutations(), 0, "nothing was written");
    assert!(setup.workspace.duplicates().is_empty());
}

#[test]
fn an_update_many_entry_closes_an_issue_as_a_duplicate_beside_an_ordinary_update() {
    let setup = Setup::new();
    let batch = json!({"schema_version": 1, "updates": [
        {"item": "linear:L-1", "title": "Groomed"},
        {"item": "linear:L-3", "duplicate_of": "linear:L-2"},
    ]});
    let output = setup
        .sandbox
        .command()
        .args(["task", "update-many", "linear", "--input", "-", "--json"])
        .write_stdin(batch.to_string())
        .assert()
        .get_output()
        .clone();
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let answer: Value = serde_json::from_str(&stdout(&output)).unwrap();
    for result in answer["results"].as_array().unwrap() {
        assert_eq!(result["outcome"], "written", "{answer:#}");
    }
    assert_eq!(
        setup.workspace.duplicates(),
        [("L-3".to_owned(), "L-2".to_owned())]
    );
    let shown = setup.ok(&["--json", "task", "show", "linear:L-3"]);
    assert_eq!(shown["items"][0]["item"]["status"]["category"], "cancelled");
    let relations = setup.ok(&["task", "relations", "linear:L-3", "--json"]);
    assert_eq!(
        relations["items"][0]["links"][0]["type"], "duplicate",
        "{relations:#}"
    );
    let renamed = setup.ok(&["--json", "task", "show", "linear:L-1"]);
    assert_eq!(renamed["items"][0]["item"]["title"], "Groomed");
}

#[test]
fn a_repeated_close_succeeds_whether_linear_answers_the_held_relation_created_or_refused() {
    let setup = Setup::new();
    let arguments = [
        "--json",
        "task",
        "status",
        "set",
        "linear:L-3",
        "cancelled",
        "--duplicate-of",
        "linear:L-2",
    ];
    setup.ok(&arguments);
    // Answered as created, then refused as a relation Linear already holds.
    setup.ok(&arguments);
    setup.workspace.refuse_repeated_duplicate();
    let answer = setup.ok(&arguments);
    assert_eq!(answer["status"]["category"], "cancelled", "{answer:#}");
    assert_eq!(
        setup.workspace.duplicates(),
        [("L-3".to_owned(), "L-2".to_owned())],
        "one relation"
    );
    let relations = setup.ok(&["task", "relations", "linear:L-3", "--json"]);
    assert_eq!(
        relations["items"][0]["links"].as_array().unwrap().len(),
        1,
        "{relations:#}"
    );
}
