//! `project attestation check --tasks` narrowed by `--status`, `--not-status` and `--state`,
//! and `task list --not-status`, driven through the binary over a folder of Markdown.
//!
//! The plan's six tasks span `todo`, `in-progress`, `done` and `cancelled`, and hold valid,
//! void and absent records. Each filter combination lists exactly the tasks it keeps, in the
//! order the unfiltered check lists them, in the unfiltered answer's JSON shape, and exits `0`
//! exactly when every task it lists is `valid`; no filter at all answers what it always did.

use serde_json::{Value, json};

use crate::attestations::Plan;
use crate::common::{stderr, stdout};

const KIND: &str = "plan-review";

/// The plan, and each task's id under the name the journeys call it by.
struct Seeded {
    plan: Plan,
    project: String,
    ids: Vec<(&'static str, String)>,
}

impl Seeded {
    fn id(&self, name: &str) -> &str {
        &self
            .ids
            .iter()
            .find(|(held, _)| *held == name)
            .expect("a seeded task")
            .1
    }

    /// `project attestation check <project> --kind plan-review --tasks <filters> --json`, held
    /// to `code`, answering the document.
    fn check(&self, filters: &[&str], code: i32) -> Value {
        let mut arguments = vec![
            "project",
            "attestation",
            "check",
            &self.project,
            "--kind",
            KIND,
            "--tasks",
        ];
        arguments.extend_from_slice(filters);
        self.plan.json_exiting(&arguments, code)
    }

    /// The ids the check answers, in its order.
    fn listed(&self, filters: &[&str], code: i32) -> Vec<String> {
        let checked = self.check(filters, code);
        assert_eq!(checked["project"], json!(self.project), "{checked}");
        assert_eq!(checked["kind"], KIND, "{checked}");
        let tasks = checked["tasks"].as_array().expect("tasks");
        for task in tasks {
            for member in ["id", "kind", "state", "reason", "record"] {
                assert!(task.get(member).is_some(), "{member} in {task}");
            }
        }
        tasks
            .iter()
            .map(|task| task["id"].as_str().expect("an id").to_owned())
            .collect()
    }

    /// The ids of `names`, in the order the unfiltered check lists them.
    fn in_order(&self, everything: &[String], names: &[&str]) -> Vec<String> {
        let wanted: Vec<&str> = names.iter().map(|name| self.id(name)).collect();
        everything
            .iter()
            .filter(|id| wanted.contains(&id.as_str()))
            .cloned()
            .collect()
    }

    /// The ids `task list --project <project> <filters>` answers, sorted.
    fn task_list(&self, filters: &[&str]) -> Vec<String> {
        let mut arguments = vec!["task", "list", "--project", &self.project, "--all"];
        arguments.extend_from_slice(filters);
        let listed = self.plan.json(&arguments);
        let mut ids: Vec<String> = listed["items"]
            .as_array()
            .expect("items")
            .iter()
            .map(|task| task["id"].as_str().expect("an id").to_owned())
            .collect();
        ids.sort();
        ids
    }

    fn sorted(&self, names: &[&str]) -> Vec<String> {
        let mut ids: Vec<String> = names.iter().map(|name| self.id(name).to_owned()).collect();
        ids.sort();
        ids
    }
}

/// Six tasks under one project: `todo` valid and absent, `in-progress` void, `done` valid and
/// absent, `cancelled` void.
fn seed() -> Seeded {
    let plan = Plan::new(false);
    let project = stdout(&plan.exits(
        &[
            "project", "create", "notes", "--id", "plan-1", "--title", "The plan",
        ],
        0,
    ))
    .trim()
    .to_owned();
    let mut ids = Vec::new();
    for (name, status) in [
        ("todo-valid", "todo"),
        ("todo-absent", "todo"),
        ("progress-void", "in-progress"),
        ("done-valid", "done"),
        ("done-absent", "done"),
        ("cancelled-void", "cancelled"),
    ] {
        let id = stdout(&plan.exits(
            &[
                "task",
                "create",
                "notes",
                "--project",
                "plan-1",
                "--title",
                name,
                "--status",
                status,
            ],
            0,
        ))
        .trim()
        .to_owned();
        ids.push((name, id));
    }
    let seeded = Seeded { plan, project, ids };
    for name in [
        "todo-valid",
        "progress-void",
        "done-valid",
        "cancelled-void",
    ] {
        seeded.plan.attest("task", seeded.id(name));
    }
    for name in ["progress-void", "cancelled-void"] {
        let id = seeded.id(name).to_owned();
        seeded.plan.exits(
            &[
                "task",
                "update",
                &id,
                "--title",
                &format!("{name} retitled"),
            ],
            0,
        );
    }
    seeded
}

#[test]
fn each_filter_lists_exactly_the_tasks_it_keeps_in_listing_order() {
    let seeded = seed();
    // No filter answers what the check always did: every task, and exit 3 for any not valid.
    let everything = seeded.listed(&[], 3);
    assert_eq!(everything.len(), 6, "{everything:?}");
    let states: Vec<(String, Value)> = seeded.check(&[], 3)["tasks"]
        .as_array()
        .expect("tasks")
        .iter()
        .map(|task| {
            (
                task["id"].as_str().unwrap().to_owned(),
                task["state"].clone(),
            )
        })
        .collect();
    for (name, state) in [
        ("todo-valid", "valid"),
        ("todo-absent", "absent"),
        ("progress-void", "void"),
        ("done-valid", "valid"),
        ("done-absent", "absent"),
        ("cancelled-void", "void"),
    ] {
        assert!(
            states.contains(&(seeded.id(name).to_owned(), json!(state))),
            "{name} is {state}: {states:?}"
        );
    }

    let cases: [(&[&str], &[&str], i32); 8] = [
        // What a review still owes: open tasks holding no valid record.
        (
            &[
                "--not-status",
                "done",
                "--not-status",
                "cancelled",
                "--state",
                "void",
                "--state",
                "absent",
            ],
            &["todo-absent", "progress-void"],
            3,
        ),
        (
            &["--status", "done", "--state", "valid"],
            &["done-valid"],
            0,
        ),
        (
            &["--status", "todo", "--status", "in-progress"],
            &["todo-valid", "todo-absent", "progress-void"],
            3,
        ),
        (&["--state", "absent"], &["todo-absent", "done-absent"], 3),
        (&["--state", "valid"], &["todo-valid", "done-valid"], 0),
        (&["--status", "done", "--not-status", "done"], &[], 0),
        (&["--status", "draft"], &[], 0),
        (
            &["--not-status", "todo", "--state", "void"],
            &["progress-void", "cancelled-void"],
            3,
        ),
    ];
    for (filters, names, code) in cases {
        assert_eq!(
            seeded.listed(filters, code),
            seeded.in_order(&everything, names),
            "filters {filters:?}"
        );
    }
}

#[test]
fn a_filter_without_tasks_is_refused_as_an_invocation() {
    let seeded = seed();
    for filter in [
        ["--state", "void"],
        ["--status", "todo"],
        ["--not-status", "done"],
    ] {
        let output = seeded.plan.run(&[
            "project",
            "attestation",
            "check",
            &seeded.project,
            "--kind",
            KIND,
            filter[0],
            filter[1],
        ]);
        assert_eq!(
            output.status.code(),
            Some(2),
            "{filter:?}: {}",
            stderr(&output)
        );
        assert!(
            stderr(&output).contains("--tasks"),
            "{filter:?}: {}",
            stderr(&output)
        );
    }
}

#[test]
fn task_list_not_status_drops_the_categories_it_names() {
    let seeded = seed();
    assert_eq!(
        seeded.task_list(&["--not-status", "done", "--not-status", "cancelled"]),
        seeded.sorted(&["todo-valid", "todo-absent", "progress-void"])
    );
    assert_eq!(
        seeded.task_list(&[
            "--status",
            "todo",
            "--status",
            "done",
            "--not-status",
            "todo"
        ]),
        seeded.sorted(&["done-valid", "done-absent"])
    );
    assert_eq!(
        seeded.task_list(&["--not-status", "draft"]),
        seeded.sorted(&[
            "todo-valid",
            "todo-absent",
            "progress-void",
            "done-valid",
            "done-absent",
            "cancelled-void"
        ])
    );
    // A pair leaving no category can match nothing, and says so rather than listing all.
    let output = seeded.plan.run(&[
        "task",
        "list",
        "--project",
        &seeded.project,
        "--status",
        "done",
        "--not-status",
        "done",
    ]);
    assert_ne!(output.status.code(), Some(0), "{}", stdout(&output));
    assert!(
        stderr(&output).contains("--not-status"),
        "{}",
        stderr(&output)
    );
}
