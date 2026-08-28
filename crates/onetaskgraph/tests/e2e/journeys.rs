//! Shared journeys over complete-dataset rows, plus focused journeys for sources whose
//! native model cannot represent that whole dataset.
//!
//! Every one of them drives the compiled binary as a subprocess. Where a journey asserts
//! on the plan as well as the rows, it asserts what *this row declares* — so the same
//! journey proves that a source applying a predicate natively has it pushed down and that
//! a source applying none of them still returns the correct rows.

use std::process::Output;

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{ROWS, Row, SOURCE, dataset, document, qualified};
use serde_json::json;

/// A sandbox holding this row's configuration document and nothing else.
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

/// Quotes stderr on failure, because a journey that fails on an exit code alone sends
/// its reader back to the shell to find out why.
fn ok(row: &Row, sandbox: &Sandbox, arguments: &[&str]) -> String {
    let output = run(sandbox, arguments);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}: `onetaskgraph {}` exited {:?}\n{}",
        row.name,
        arguments.join(" "),
        output.status.code(),
        stderr(&output)
    );
    stdout(&output)
}

/// The qualified ids in a rendered list, in order.
///
/// The id is the first column of every list this binary prints, which is what makes one
/// reader enough for tasks, projects, labels and dependency edges.
fn listed(rendered: &str) -> Vec<String> {
    rendered
        .lines()
        .take_while(|line| !line.trim().is_empty())
        .filter_map(|line| line.split_whitespace().next())
        .map(str::to_owned)
        .collect()
}

fn edge_starts(rendered: &str) -> Vec<String> {
    rendered
        .lines()
        .take_while(|line| !line.trim().is_empty())
        .filter_map(|line| line.split_whitespace().nth(1))
        .map(str::to_owned)
        .collect()
}

/// The ids this row's source is expected to answer with.
fn ours(natives: &[&str]) -> Vec<String> {
    natives
        .iter()
        .map(|native| qualified(SOURCE, native))
        .collect()
}

/// Assert the rendered plan says `outcome` covers `predicate` for this row's source.
fn plan_says(row: &Row, rendered: &str, outcome: &str, predicate: &str) {
    let line = rendered
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with(&format!("{outcome}:")))
        .unwrap_or_else(|| {
            panic!(
                "{}: the plan has no `{outcome}:` line:\n{rendered}",
                row.name
            )
        });
    assert!(
        line.contains(predicate),
        "{}: the plan's `{outcome}` line does not name {predicate}:\n{rendered}",
        row.name
    );
}

/// Rows that can represent every entity and relationship in the shared dataset.
fn complete_dataset_rows() -> impl Iterator<Item = &'static Row> {
    ROWS.iter().filter(|row| row.fixture.complete_dataset)
}

#[test]
fn github_projects_runs_shared_binary_journeys_against_its_fixture_server() {
    let row = ROWS
        .iter()
        .find(|row| row.plugin == "github-projects")
        .expect("GitHub Projects fixture row");
    let sandbox = host(row);

    let listed_tasks = ok(row, &sandbox, &["task", "list"]);
    assert_eq!(listed(&listed_tasks), ours(&["T-1", "T-2", "T-3", "T-4"]));

    let filtered = ok(
        row,
        &sandbox,
        &[
            "task",
            "list",
            "--label",
            "bug",
            "--status",
            "todo",
            "--explain",
        ],
    );
    assert_eq!(listed(&filtered), ours(&["T-1", "T-3"]));
    plan_says(row, &filtered, "applied locally", "label");
    plan_says(row, &filtered, "applied locally", "status");

    let searched = ok(
        row,
        &sandbox,
        &[
            "task",
            "list",
            "--search",
            "alpha",
            "--in",
            "both",
            "--explain",
        ],
    );
    assert_eq!(listed(&searched), ours(&["T-1", "T-2"]));
    plan_says(row, &searched, "applied locally", "search-title");
    plan_says(row, &searched, "applied locally", "search-content");

    let dependencies = ok(row, &sandbox, &["task", "deps", &qualified(SOURCE, "T-1")]);
    assert!(
        dependencies.contains(&qualified(SOURCE, "T-2")),
        "{dependencies}"
    );

    let mut walked = Vec::new();
    let mut token: Option<String> = None;
    loop {
        let mut arguments = vec!["task", "list", "--limit", "1"];
        if let Some(page) = &token {
            arguments.extend(["--page", page]);
        }
        let rendered = ok(row, &sandbox, &arguments);
        walked.extend(listed(&rendered));
        token = rendered
            .lines()
            .find_map(|line| line.strip_prefix("next page: --page "))
            .map(str::to_owned);
        if token.is_none() {
            break;
        }
    }
    assert_eq!(walked, ours(&["T-1", "T-2", "T-3", "T-4"]));
}

#[test]
fn github_projects_missing_credential_reaches_the_binary_user() {
    let sandbox = Sandbox::new();
    sandbox.project_document(&document(&json!({
        SOURCE: {"plugin":"github-projects","config":{
            "owner":"fixture-owner","project_number":7,
            "token_env":"DELIBERATELY_MISSING_GITHUB_TOKEN"
        }}
    })));
    let output = run(&sandbox, &["task", "list"]);
    assert_eq!(output.status.code(), Some(4), "{}", stderr(&output));
    let complaint = stderr(&output);
    assert!(
        complaint.contains("DELIBERATELY_MISSING_GITHUB_TOKEN"),
        "{complaint}"
    );
    assert!(complaint.contains("--allow-partial"), "{complaint}");

    let allowed = run(&sandbox, &["task", "list", "--allow-partial"]);
    assert_eq!(
        allowed.status.code(),
        Some(0),
        "allowing the missing source must make the partial run succeed: {}",
        stderr(&allowed)
    );
    assert!(
        stderr(&allowed).contains("DELIBERATELY_MISSING_GITHUB_TOKEN"),
        "the partial run must still explain the missing credential: {}",
        stderr(&allowed)
    );
}

#[test]
fn every_complete_dataset_source_lists_its_tasks_and_shows_one_by_its_qualified_id() {
    for row in complete_dataset_rows() {
        let sandbox = host(row);

        let listing = ok(row, &sandbox, &["task", "list"]);
        assert_eq!(
            listed(&listing),
            ours(&["T-1", "T-2", "T-3", "T-4"]),
            "{}: every task, in the source's own order",
            row.name
        );
        assert!(
            listing.contains("Alpha engine") && listing.contains("in-progress"),
            "{}: a list carries each task's title and normalised status:\n{listing}",
            row.name
        );

        let shown = ok(row, &sandbox, &["task", "show", &qualified(SOURCE, "T-1")]);
        for expected in [
            "Alpha engine",
            "todo (Todo)",
            "bug, core",
            "the engine core",
        ] {
            assert!(
                shown.contains(expected),
                "{}: `task show` omits {expected}:\n{shown}",
                row.name
            );
        }
        assert!(
            shown.contains(&format!("project:  {}", qualified(SOURCE, "P-1"))),
            "{}: a task's project is qualified too:\n{shown}",
            row.name
        );
    }
}

#[test]
fn every_complete_dataset_source_lists_its_projects_and_shows_one_by_its_qualified_id() {
    for row in complete_dataset_rows() {
        let sandbox = host(row);

        let listing = ok(row, &sandbox, &["project", "list"]);
        assert_eq!(listed(&listing), ours(&["P-1", "P-2"]), "{}", row.name);

        let shown = ok(
            row,
            &sandbox,
            &["project", "show", &qualified(SOURCE, "P-1")],
        );
        assert!(shown.contains("Engine"), "{}: {shown}", row.name);
        assert!(
            shown.contains("in-progress (Doing)"),
            "{}: {shown}",
            row.name
        );
    }
}

#[test]
fn every_source_preserves_typed_metadata_and_repository_origins_through_the_binary() {
    for row in ROWS {
        let sandbox = host(row);
        let task: serde_json::Value = serde_json::from_str(&ok(
            row,
            &sandbox,
            &["task", "show", &qualified(SOURCE, "T-1"), "--json"],
        ))
        .expect("task show emits JSON");
        let task = &task["items"][0]["item"];
        assert_eq!(
            task["metadata"]["onepipeline.turn_budget"],
            json!(12),
            "{}",
            row.name
        );
        assert_eq!(
            task["metadata"]["caller.flags"],
            json!([true, null]),
            "{}",
            row.name
        );
        assert_eq!(
            task["repositories"],
            json!(["github.com/nickderobertis/onetaskgraph"]),
            "{}",
            row.name
        );

        let project: serde_json::Value = serde_json::from_str(&ok(
            row,
            &sandbox,
            &["project", "show", &qualified(SOURCE, "P-1"), "--json"],
        ))
        .expect("project show emits JSON");
        let project = &project["items"][0]["item"];
        assert_eq!(
            project["metadata"]["onepipeline.publication"],
            json!({"mode":"review"}),
            "{}",
            row.name
        );
        assert_eq!(
            project["repositories"],
            json!(["github.com/nickderobertis/onetaskgraph"]),
            "{}",
            row.name
        );
    }
}

#[test]
fn every_source_orients_a_native_edge_from_the_item_that_depends() {
    // One orientation across every backend: `from` depends on `to`. Each source spells the
    // relationship its own way — a `depends_on` list, a Linear relation, a GitHub
    // `blockedBy` connection — and the whole point of one interface over them is that a
    // caller cannot tell which by reading the answer. So the same edge has to come back
    // identical whether it is asked for from the end that depends or the end that blocks.
    let edge = |from: &str, to: &str, kind: &str| {
        json!({
            "from": {"id": qualified(SOURCE, from), "kind": kind},
            "to": {"id": qualified(SOURCE, to), "kind": kind},
            "kind": "blocks"
        })
    };
    let items = |rendered: &str| -> Vec<serde_json::Value> {
        serde_json::from_str::<serde_json::Value>(rendered).expect("dependency output is JSON")
            ["items"]
            .as_array()
            .expect("an edge page is a list")
            .clone()
    };

    for row in ROWS {
        let sandbox = host(row);

        // T-1 waits on T-2, read from T-1...
        let forward = items(&ok(
            row,
            &sandbox,
            &["task", "deps", &qualified(SOURCE, "T-1"), "--json"],
        ));
        assert!(
            forward.contains(&edge("T-1", "T-2", "task")),
            "{}: T-1 is what depends:\n{forward:#?}",
            row.name
        );

        // ...and the same edge, unchanged, read from T-2.
        let reverse = items(&ok(
            row,
            &sandbox,
            &[
                "task",
                "deps",
                &qualified(SOURCE, "T-2"),
                "--direction",
                "depended-on-by",
                "--json",
            ],
        ));
        assert!(
            reverse.contains(&edge("T-1", "T-2", "task")),
            "{}: the reverse read reports the same edge, not its mirror:\n{reverse:#?}",
            row.name
        );

        let projects = items(&ok(
            row,
            &sandbox,
            &["project", "deps", &qualified(SOURCE, "P-1"), "--json"],
        ));
        assert!(
            projects.contains(&edge("P-1", "P-2", "project")),
            "{}: P-1 is what depends:\n{projects:#?}",
            row.name
        );

        // The reverse project read is asked of P-2, and one GitHub source is exactly one
        // board, so that row cannot be asked about another. Its reverse orientation is
        // covered where it can be: `project_dependencies_map_reverse_edges_and_page_them`
        // in that crate's own suite, over a real socket.
        if !row.fixture.complete_dataset {
            continue;
        }
        let reverse_projects = items(&ok(
            row,
            &sandbox,
            &[
                "project",
                "deps",
                &qualified(SOURCE, "P-2"),
                "--direction",
                "depended-on-by",
                "--json",
            ],
        ));
        assert!(
            reverse_projects.contains(&edge("P-1", "P-2", "project")),
            "{}: the reverse read reports the same edge:\n{reverse_projects:#?}",
            row.name
        );
    }
}

#[test]
fn every_source_reports_a_cross_source_cross_level_edge_without_following_it() {
    // The far ends are in a source called `elsewhere`, which no row configures. A read
    // that resolved one would need the far plugin — the state this product does not hold —
    // so the proof that it does not is that the read succeeds while the far source does
    // not exist, and that asking for the far item is the caller's own next command.
    for row in ROWS {
        let sandbox = host(row);
        for (verb, near, far, far_kind) in [
            ("task", "T-1", "elsewhere:P-9", "project"),
            ("project", "P-1", "elsewhere:T-9", "task"),
        ] {
            let forward: serde_json::Value = serde_json::from_str(&ok(
                row,
                &sandbox,
                &[verb, "deps", &qualified(SOURCE, near), "--json"],
            ))
            .expect("dependency output is JSON");
            let recorded = json!({
                "from": {"id": qualified(SOURCE, near), "kind": if verb == "task" {"task"} else {"project"}},
                "to": {"id": far, "kind": far_kind},
                "kind": "blocks"
            });
            assert!(
                forward["items"]
                    .as_array()
                    .expect("an edge page is a list")
                    .contains(&recorded),
                "{}: {verb} {near} names its far end by qualified id and kind:\n{forward:#}",
                row.name
            );

            // The reverse of a recorded edge belongs to the far end and is never held here.
            let reverse = ok(
                row,
                &sandbox,
                &[
                    verb,
                    "deps",
                    &qualified(SOURCE, near),
                    "--direction",
                    "depended-on-by",
                ],
            );
            assert!(
                !reverse.contains("elsewhere:"),
                "{}: {verb} {near} reversed:\n{reverse}",
                row.name
            );
        }

        // Following the edge is the caller's next command, against a source they configure.
        let unknown = run(&sandbox, &["project", "show", "elsewhere:P-9"]);
        assert_ne!(unknown.status.code(), Some(0), "{}", row.name);
        assert!(
            stderr(&unknown).contains("elsewhere"),
            "{}: {}",
            row.name,
            stderr(&unknown)
        );
    }
}

#[test]
fn a_far_end_a_source_cannot_name_travels_with_its_kind_through_text_output() {
    let row = &ROWS[0];
    let sandbox = host(row);
    let rendered = ok(row, &sandbox, &["task", "deps", &qualified(SOURCE, "T-1")]);
    assert!(
        rendered
            .lines()
            .any(|line| line.split_whitespace().collect::<Vec<_>>()
                == ["task", "work:T-1", "blocks", "project", "elsewhere:P-9"]),
        "{rendered}"
    );
}

#[test]
fn a_colon_in_a_source_native_dependency_id_is_not_reinterpreted_as_a_source() {
    let sandbox = Sandbox::new();
    sandbox.project_document(&document(&json!({
        SOURCE: {
            "plugin": "in-memory",
            "config": {
                "tasks": [
                    {"id":"urn:task:7", "title":"Near", "status":{"category":"todo","name":"Todo"}, "labels":[]},
                    {"id":"T-2", "title":"Far", "status":{"category":"todo","name":"Todo"}, "labels":[]}
                ],
                "task_dependencies": [{"from":"urn:task:7", "to":"T-2", "kind":"blocks"}]
            }
        }
    })));
    let row = ROWS
        .iter()
        .find(|row| row.plugin == "in-memory")
        .expect("in-memory row");
    let response: serde_json::Value = serde_json::from_str(&ok(
        row,
        &sandbox,
        &["task", "deps", &qualified(SOURCE, "urn:task:7"), "--json"],
    ))
    .expect("dependency output is JSON");
    assert_eq!(response["items"][0]["from"]["id"], "work:urn:task:7");
    assert_eq!(response["items"][0]["to"]["id"], "work:T-2");
}

#[test]
fn malformed_local_markdown_names_its_path_without_hiding_valid_rows() {
    let sandbox = Sandbox::new();
    let root = sandbox.subdirectory("malformed-local-md");
    let tasks = root.join("tasks");
    std::fs::create_dir_all(&tasks).expect("the task directory");
    std::fs::write(
        tasks.join("valid.md"),
        "---\ntitle: Still available\nstatus: todo\n---\nA valid task.\n",
    )
    .expect("the valid Markdown task");
    let malformed = tasks.join("broken.md");
    std::fs::write(&malformed, "title: missing front-matter delimiters\n")
        .expect("the malformed Markdown task");
    sandbox.project_document(&document(&json!({
        SOURCE: {"plugin": "local-md", "config": {"root": root}},
    })));

    let shown = run(&sandbox, &["task", "show", &qualified(SOURCE, "broken")]);
    assert_eq!(shown.status.code(), Some(4), "{}", stderr(&shown));
    let complaint = stderr(&shown);
    let malformed = malformed
        .canonicalize()
        .expect("the malformed Markdown path is canonicalizable");
    assert!(
        complaint.contains(&malformed.display().to_string()),
        "the malformed file's exact path must reach the user:\n{complaint}"
    );

    let listing = run(&sandbox, &["task", "list"]);
    assert_eq!(listing.status.code(), Some(0), "{}", stderr(&listing));
    assert_eq!(listed(&stdout(&listing)), ours(&["valid"]));
    assert!(
        stderr(&listing).is_empty(),
        "a usable listing stays quiet:\n{}",
        stderr(&listing)
    );
}

#[test]
fn a_task_in_no_project_is_listed_by_default_and_can_be_selected_on_its_own() {
    for row in complete_dataset_rows() {
        let sandbox = host(row);
        let declared = row.declared();

        let all = ok(row, &sandbox, &["task", "list"]);
        assert!(
            listed(&all).contains(&qualified(SOURCE, "T-3")),
            "{}: an orphan is not an edge case; it is listed by default",
            row.name
        );

        let orphans = ok(
            row,
            &sandbox,
            &["task", "list", "--no-project", "--explain"],
        );
        assert_eq!(listed(&orphans), ours(&["T-3"]), "{}", row.name);
        plan_says(
            row,
            &orphans,
            if declared.orphan_tasks {
                "pushed down"
            } else {
                "applied locally"
            },
            "project",
        );

        // The other way round: tasks of one project, qualified.
        let of_project = ok(
            row,
            &sandbox,
            &["task", "list", "--project", &qualified(SOURCE, "P-1")],
        );
        assert_eq!(listed(&of_project), ours(&["T-1", "T-2"]), "{}", row.name);
    }
}

#[test]
fn every_complete_dataset_source_lists_the_labels_it_knows() {
    for row in complete_dataset_rows() {
        let listing = ok(row, &host(row), &["label", "list"]);
        assert_eq!(
            listed(&listing),
            ours(&["L-1", "L-2", "L-3"]),
            "{}",
            row.name
        );
        assert!(listing.contains("chore"), "{}: {listing}", row.name);
    }
}

#[test]
fn complete_dataset_sources_agree_on_label_filtering_wherever_it_is_applied() {
    for row in complete_dataset_rows() {
        let sandbox = host(row);
        let declared = row.declared();
        let outcome = if declared.filter_by_label {
            "pushed down"
        } else {
            "applied locally"
        };

        let one = ok(
            row,
            &sandbox,
            &["task", "list", "--label", "bug", "--explain"],
        );
        assert_eq!(listed(&one), ours(&["T-1", "T-3"]), "{}", row.name);
        plan_says(row, &one, outcome, "label");

        // Several at once narrows rather than widens: a second `--label` is a second
        // requirement.
        let several = ok(
            row,
            &sandbox,
            &["task", "list", "--label", "bug", "--label", "core"],
        );
        assert_eq!(listed(&several), ours(&["T-1"]), "{}", row.name);

        let excluded = ok(row, &sandbox, &["task", "list", "--not-label", "bug"]);
        assert_eq!(listed(&excluded), ours(&["T-2", "T-4"]), "{}", row.name);
    }
}

#[test]
fn complete_dataset_sources_agree_on_status_filtering_wherever_it_is_applied() {
    for row in complete_dataset_rows() {
        let sandbox = host(row);
        let declared = row.declared();

        let todo = ok(
            row,
            &sandbox,
            &["task", "list", "--status", "todo", "--explain"],
        );
        assert_eq!(listed(&todo), ours(&["T-1", "T-3"]), "{}", row.name);
        plan_says(
            row,
            &todo,
            if declared.filter_by_status {
                "pushed down"
            } else {
                "applied locally"
            },
            "status",
        );

        let several = ok(
            row,
            &sandbox,
            &["task", "list", "--status", "todo", "--status", "done"],
        );
        assert_eq!(
            listed(&several),
            ours(&["T-1", "T-2", "T-3"]),
            "{}",
            row.name
        );
    }
}

#[test]
fn searching_covers_titles_bodies_or_either_over_tasks_and_projects() {
    for row in complete_dataset_rows() {
        let sandbox = host(row);
        let declared = row.declared();

        for (fields, predicate, native, expected) in [
            ("title", "search-title", declared.search_title, vec!["T-1"]),
            (
                "content",
                "search-content",
                declared.search_content,
                vec!["T-2"],
            ),
        ] {
            let found = ok(
                row,
                &sandbox,
                &[
                    "task",
                    "list",
                    "--search",
                    "alpha",
                    "--in",
                    fields,
                    "--explain",
                ],
            );
            assert_eq!(listed(&found), ours(&expected), "{} in {fields}", row.name);
            plan_says(
                row,
                &found,
                if native {
                    "pushed down"
                } else {
                    "applied locally"
                },
                predicate,
            );
        }

        // Either: a source that cannot search *both* halves must not be asked at all, or
        // the body-only match would be dropped — a narrower answer than the truth.
        let either = ok(
            row,
            &sandbox,
            &["task", "list", "--search", "alpha", "--in", "both"],
        );
        assert_eq!(listed(&either), ours(&["T-1", "T-2"]), "{}", row.name);

        // And over projects, through the verb that searches both entities.
        let projects = ok(row, &sandbox, &["search", "alpha", "--kind", "project"]);
        assert_eq!(
            listed(&projects),
            ["project"],
            "{}: a search hit says which entity matched\n{projects}",
            row.name
        );
        assert!(projects.contains(&qualified(SOURCE, "P-2")), "{}", row.name);

        let both = ok(row, &sandbox, &["search", "alpha", "--kind", "both"]);
        assert!(
            both.contains(&qualified(SOURCE, "T-1")) && both.contains(&qualified(SOURCE, "P-2")),
            "{}: searching both entities returns both:\n{both}",
            row.name
        );
    }
}

#[test]
fn complete_dataset_sources_walk_task_dependencies_forwards_and_backwards() {
    for row in complete_dataset_rows() {
        let sandbox = host(row);
        let declared = row.declared();

        let forward = ok(
            row,
            &sandbox,
            &["task", "deps", &qualified(SOURCE, "T-1"), "--explain"],
        );
        assert_eq!(
            edge_starts(&forward),
            ours(&["T-1", "T-1"]),
            "{}: T-1 depends on a task of this source and on a project of another",
            row.name
        );
        assert!(
            forward.contains("blocks") && forward.contains(&qualified(SOURCE, "T-2")),
            "{}: an edge names both ends and what it means:\n{forward}",
            row.name
        );
        assert!(
            forward
                .lines()
                .any(|line| line.split_whitespace().collect::<Vec<_>>()
                    == ["task", "work:T-1", "blocks", "task", "work:T-2"]),
            "{}: text edges render both endpoint kinds:\n{forward}",
            row.name
        );

        let reverse = ok(
            row,
            &sandbox,
            &[
                "task",
                "deps",
                &qualified(SOURCE, "T-2"),
                "--direction",
                "depended-on-by",
                "--explain",
            ],
        );
        assert_eq!(
            edge_starts(&reverse),
            ours(&["T-1", "T-3", "T-4"]),
            "{}: three tasks depend on T-2",
            row.name
        );
        plan_says(
            row,
            &reverse,
            if declared.reverse_task_dependencies {
                "pushed down"
            } else {
                "emulated"
            },
            "reverse-dependencies",
        );
    }
}

#[test]
fn complete_dataset_sources_walk_project_dependencies_forwards_and_backwards() {
    for row in complete_dataset_rows() {
        let sandbox = host(row);
        let declared = row.declared();

        let forward = ok(
            row,
            &sandbox,
            &["project", "deps", &qualified(SOURCE, "P-1")],
        );
        assert!(
            forward.contains(&qualified(SOURCE, "P-2")),
            "{}: {forward}",
            row.name
        );

        let reverse = ok(
            row,
            &sandbox,
            &[
                "project",
                "deps",
                &qualified(SOURCE, "P-2"),
                "--direction",
                "depended-on-by",
                "--explain",
            ],
        );
        assert_eq!(edge_starts(&reverse), ours(&["P-1"]), "{}", row.name);
        plan_says(
            row,
            &reverse,
            if declared.reverse_project_dependencies {
                "pushed down"
            } else {
                "emulated"
            },
            "reverse-dependencies",
        );
    }
}

#[test]
fn every_complete_dataset_source_filters_projects_by_label_status_and_text() {
    // `project list` carries the same filters `task list` does, over a source's other
    // entity and through a different query type. A source that applied them to tasks and
    // dropped them for projects would pass every task journey above.
    for row in complete_dataset_rows() {
        let sandbox = host(row);
        let declared = row.declared();

        let by_label = ok(
            row,
            &sandbox,
            &["project", "list", "--label", "core", "--explain"],
        );
        assert_eq!(listed(&by_label), ours(&["P-1"]), "{}", row.name);
        plan_says(
            row,
            &by_label,
            if declared.filter_by_label {
                "pushed down"
            } else {
                "applied locally"
            },
            "label",
        );

        let excluded = ok(row, &sandbox, &["project", "list", "--not-label", "core"]);
        assert_eq!(listed(&excluded), ours(&["P-2"]), "{}", row.name);

        let by_status = ok(
            row,
            &sandbox,
            &["project", "list", "--status", "todo", "--explain"],
        );
        assert_eq!(listed(&by_status), ours(&["P-2"]), "{}", row.name);
        plan_says(
            row,
            &by_status,
            if declared.filter_by_status {
                "pushed down"
            } else {
                "applied locally"
            },
            "status",
        );

        // `--status` is one vocabulary across both verbs, so every category it spells
        // reaches this one too. These two are categories no project here carries: they
        // are accepted, they select nothing rather than everything, and the plan reports
        // the predicate exactly as it does for a category that matches.
        let unheld = ok(
            row,
            &sandbox,
            &[
                "project",
                "list",
                "--status",
                "draft",
                "--status",
                "cancelled",
                "--explain",
            ],
        );
        assert!(listed(&unheld).is_empty(), "{}: {unheld}", row.name);
        plan_says(
            row,
            &unheld,
            if declared.filter_by_status {
                "pushed down"
            } else {
                "applied locally"
            },
            "status",
        );

        // `alpha` is in P-2's body and in neither title, so each field selector keeps a
        // different set and a source searching the wrong one is caught.
        let in_title = ok(
            row,
            &sandbox,
            &["project", "list", "--search", "engine", "--in", "title"],
        );
        assert_eq!(listed(&in_title), ours(&["P-1"]), "{}", row.name);

        let in_content = ok(
            row,
            &sandbox,
            &["project", "list", "--search", "alpha", "--in", "content"],
        );
        assert_eq!(listed(&in_content), ours(&["P-2"]), "{}", row.name);

        let in_both = ok(
            row,
            &sandbox,
            &["project", "list", "--search", "engine", "--in", "both"],
        );
        assert_eq!(listed(&in_both), ours(&["P-1"]), "{}", row.name);
    }
}

#[test]
fn showing_one_item_from_a_source_that_cannot_answer_exits_four_unless_partial_is_allowed() {
    // `show` reads one item from one source, so a source that cannot answer is the whole
    // of its result rather than one contribution among several — and it must still cost
    // exit 4 rather than reading as "no such id", which is a different problem with a
    // different fix. `--allow-partial` is the caller saying an answer without it is fine.
    for verb in ["task", "project"] {
        let sandbox = Sandbox::new();
        sandbox.project_document(&document(&json!({
            "broken": {"plugin": "linear", "config": {}},
        })));

        let id = qualified("broken", "X-1");
        let refused = run(&sandbox, &[verb, "show", &id]);
        assert_eq!(
            refused.status.code(),
            Some(4),
            "`{verb} show` against a source that cannot answer must exit 4:\n{}",
            stderr(&refused)
        );
        let complaint = stderr(&refused);
        assert!(
            complaint.contains("broken"),
            "the failure must name the source:\n{complaint}"
        );
        assert!(
            complaint.contains("--allow-partial"),
            "and say how to accept it:\n{complaint}"
        );

        let allowed = run(&sandbox, &[verb, "show", &id, "--allow-partial"]);
        assert_eq!(
            allowed.status.code(),
            Some(0),
            "the same run with --allow-partial must exit 0:\n{}",
            stderr(&allowed)
        );
        assert!(
            stderr(&allowed).contains("broken"),
            "and still say which source was lost:\n{}",
            stderr(&allowed)
        );
    }
}

#[test]
fn a_both_kind_search_whose_projects_ran_out_still_resumes_under_the_narrower_kind() {
    // `search --kind both` reads two streams per source and drops each as it is spent, so
    // a token from the middle of such a walk can carry the task half alone. Handed to
    // `--kind task` it resumes, and that is deliberate rather than an oversight: which
    // streams a search covers is what the token's stream check reads, name by name, so
    // the scope is left out of the query fingerprint on purpose. A token still carrying
    // the project half is refused by that check, and this is the other side of it.
    for row in complete_dataset_rows() {
        let sandbox = host(row);

        let mut token = String::new();
        let mut narrowed = None;
        for _ in 0..6 {
            let mut arguments = vec!["search", "alpha", "--kind", "both", "--limit", "1"];
            if !token.is_empty() {
                arguments.push("--page");
                arguments.push(&token);
            }
            let rendered = ok(row, &sandbox, &arguments);
            let Some(next) = rendered
                .lines()
                .find_map(|line| line.strip_prefix("next page: --page "))
                .map(str::to_owned)
            else {
                break;
            };
            // The first token that no longer mentions a project stream is the one worth
            // handing to `--kind task`.
            if !ok(
                row,
                &sandbox,
                &[
                    "search", "alpha", "--kind", "both", "--limit", "1", "--page", &next,
                ],
            )
            .lines()
            .any(|line| line.starts_with("project"))
            {
                narrowed = Some(next.clone());
            }
            token = next;
            if narrowed.is_some() {
                break;
            }
        }

        let narrowed = narrowed
            .unwrap_or_else(|| panic!("{}: the walk never reached a task-only token", row.name));
        let resumed = ok(
            row,
            &sandbox,
            &[
                "search", "alpha", "--kind", "task", "--limit", "1", "--page", &narrowed,
            ],
        );
        // A search hit leads with its entity, so the id is the second column here rather
        // than the first that `listed` reads.
        assert!(
            resumed.contains(&qualified(SOURCE, "T-2")),
            "{}: the task half must carry on:\n{resumed}",
            row.name
        );
        assert!(
            !resumed.lines().any(|line| line.starts_with("project")),
            "{}: and the exhausted project half must stay gone:\n{resumed}",
            row.name
        );
    }
}

#[test]
fn a_limit_smaller_than_the_result_set_walks_to_exhaustion_in_a_stable_order() {
    for row in complete_dataset_rows() {
        let sandbox = host(row);
        let whole = listed(&ok(row, &sandbox, &["task", "list"]));

        for limit in ["1", "3"] {
            let mut walked = Vec::new();
            let mut token: Option<String> = None;
            for step in 0..10 {
                let mut arguments = vec!["task", "list", "--limit", limit];
                if let Some(page) = &token {
                    arguments.push("--page");
                    arguments.push(page);
                }
                let rendered = ok(row, &sandbox, &arguments);
                walked.extend(listed(&rendered));
                token = rendered
                    .lines()
                    .find_map(|line| line.strip_prefix("next page: --page "))
                    .map(str::to_owned);
                if token.is_none() {
                    break;
                }
                assert!(step < 9, "{}: a four-row walk must terminate", row.name);
            }
            assert_eq!(
                walked, whole,
                "{}: walking at --limit {limit} must return every row exactly once, in \
                 the order one page returns them",
                row.name
            );
        }
    }
}

#[test]
fn a_shown_item_carries_the_fields_the_source_gave_it_and_says_when_it_has_none() {
    for row in complete_dataset_rows() {
        let sandbox = host(row);

        let with_url = ok(row, &sandbox, &["task", "show", &qualified(SOURCE, "T-1")]);
        assert!(
            with_url.contains("url:") && with_url.contains("https://example.invalid/T-1"),
            "{}: a task's url is worth showing when the source has one:\n{with_url}",
            row.name
        );

        // An orphan says so rather than leaving the row out, which would read as a task
        // whose project the renderer forgot.
        let orphan = ok(row, &sandbox, &["task", "show", &qualified(SOURCE, "T-3")]);
        assert!(
            orphan.contains("project:  none"),
            "{}: a task in no project says so:\n{orphan}",
            row.name
        );
        assert!(
            !orphan.contains("url:"),
            "{}: and a field the source did not give is left out entirely:\n{orphan}",
            row.name
        );

        let project = ok(
            row,
            &sandbox,
            &["project", "show", &qualified(SOURCE, "P-1")],
        );
        assert!(
            project.contains("https://example.invalid/P-1"),
            "{}",
            row.name
        );

        // `--explain` reaches the single-item verbs too.
        let explained = ok(
            row,
            &sandbox,
            &["task", "show", &qualified(SOURCE, "T-1"), "--explain"],
        );
        assert!(
            explained.contains("plan:") && explained.contains(SOURCE),
            "{}: {explained}",
            row.name
        );
    }
}

#[test]
fn every_status_category_and_search_scope_the_command_line_spells_is_accepted() {
    for row in complete_dataset_rows() {
        let sandbox = host(row);

        // The three categories this fixture holds, one flag each.
        for (category, expected) in [
            ("todo", vec!["T-1", "T-3"]),
            ("done", vec!["T-2"]),
            ("in-progress", vec!["T-4"]),
        ] {
            let found = ok(row, &sandbox, &["task", "list", "--status", category]);
            assert_eq!(listed(&found), ours(&expected), "{} {category}", row.name);
        }

        // And the four it does not: accepted, and correctly matching nothing. `draft` is
        // one of them at every row, which is what says the vocabulary reaches every
        // source kind rather than only the one that spells a draft in its own store.
        let none = ok(
            row,
            &sandbox,
            &[
                "task",
                "list",
                "--status",
                "draft",
                "--status",
                "backlog",
                "--status",
                "cancelled",
                "--status",
                "unknown",
            ],
        );
        assert!(none.trim().is_empty(), "{}: {none}", row.name);

        let tasks_only = ok(row, &sandbox, &["search", "alpha", "--kind", "task"]);
        assert!(
            tasks_only.lines().all(|line| line.starts_with("task")),
            "{}: --kind task returns tasks and nothing else:\n{tasks_only}",
            row.name
        );

        // `--output text` is the other spelling of the default, and has to reach the
        // same renderer as leaving it out.
        let explicit = ok(row, &sandbox, &["task", "list", "--output", "text"]);
        assert_eq!(
            explicit,
            ok(row, &sandbox, &["task", "list"]),
            "{}",
            row.name
        );

        // A project named by its native id alone is asked of every selected source.
        let bare = ok(row, &sandbox, &["task", "list", "--project", "P-2"]);
        assert_eq!(listed(&bare), ours(&["T-4"]), "{}", row.name);
    }
}

#[test]
fn a_native_id_may_contain_colons_because_a_qualified_id_splits_on_the_first_one() {
    // Its own fixture rather than the shared table's: this is about *addressing*, and a
    // colon-bearing id in the shared dataset would make every other journey's expected
    // list carry a detail none of them are about.
    for boundary in crate::common::SOURCE_BOUNDARIES {
        let sandbox = Sandbox::new();
        sandbox.project_document(&crate::fixtures::document(&serde_json::json!({
            SOURCE: boundary.source("in-memory", serde_json::json!({
                "projects": [{"id": "urn:project:1", "title": "Urn", "content": null,
                              "status": {"category": "todo", "name": "Todo"}, "labels": []}],
                "tasks": [{"id": "urn:task:7", "title": "Colonised", "content": null,
                           "status": {"category": "todo", "name": "Todo"}, "labels": [],
                           "project": "urn:project:1"}]
            }))
        })));
        let row = &ROWS[0];

        let shown = ok(row, &sandbox, &["task", "show", "work:urn:task:7"]);
        assert!(
            shown.contains("id:       work:urn:task:7") && shown.contains("Colonised"),
            "a qualified id splits on the FIRST colon, so the rest is the native id:\n{shown}"
        );
        assert!(
            shown.contains("project:  work:urn:project:1"),
            "and a qualified project id is rendered the same way:\n{shown}"
        );

        // A bare project id full of colons is a native id, because its prefix — `urn` — is
        // not a configured source. That is the whole disambiguation rule, exercised.
        let of_project = ok(
            row,
            &sandbox,
            &["task", "list", "--project", "urn:project:1"],
        );
        assert_eq!(listed(&of_project), ["work:urn:task:7"], "{of_project}");

        // And the same id qualified narrows to that source instead, to the same answer.
        let qualified_form = ok(
            row,
            &sandbox,
            &["task", "list", "--project", "work:urn:project:1"],
        );
        assert_eq!(listed(&qualified_form), ["work:urn:task:7"]);
    }
}

#[test]
fn the_kind_marker_one_plugin_owns_is_ordinary_caller_metadata_everywhere_else() {
    // `onetaskgraph.item_kind` is registered in the contract crate so no plugin can invent
    // a colliding spelling, and it obliges nobody: `github-projects` needs it because a
    // board holds only issues and an empty project is otherwise indistinguishable from a
    // task, while local-md has folders and Linear has native projects. Every other source
    // hands it back exactly as it holds it — including a value `github-projects` itself
    // would refuse, which is what says the key is not being interpreted.
    let sandbox = Sandbox::new();
    let root = sandbox.subdirectory("folder");
    std::fs::create_dir_all(root.join("tasks")).unwrap();
    std::fs::write(
        root.join("tasks/T-1.md"),
        "---\ntitle: Marked\nstatus: Todo\nmetadata: {onetaskgraph.item_kind: {shape: [1, true, null]}}\n---\nbody\n",
    )
    .unwrap();
    let mut memory = dataset();
    memory["tasks"] = json!([{"id":"T-1","title":"Marked",
        "status":{"category":"todo","name":"Todo"},"labels":[],
        "metadata":{"onetaskgraph.item_kind":{"shape":[1, true, null]}}}]);
    memory["task_dependencies"] = json!([]);
    sandbox.project_document(&document(&json!({
        "folder": {"plugin":"local-md","config":{"root":root}},
        "memory": {"plugin":"in-memory","config":memory},
    })));

    for source in ["folder", "memory"] {
        let shown: serde_json::Value = serde_json::from_str(&ok_at(
            &sandbox,
            &["task", "show", &qualified(source, "T-1"), "--json"],
        ))
        .expect("task show emits JSON");
        assert_eq!(
            shown["items"][0]["item"]["metadata"]["onetaskgraph.item_kind"],
            json!({"shape":[1, true, null]}),
            "{source} must return the key with its JSON type intact"
        );
    }
}

/// Standard output of a run that had to succeed, for a sandbox with no row behind it.
fn ok_at(sandbox: &Sandbox, arguments: &[&str]) -> String {
    let output = run(sandbox, arguments);
    assert_eq!(
        output.status.code(),
        Some(0),
        "`onetaskgraph {}` exited {:?}\n{}",
        arguments.join(" "),
        output.status.code(),
        stderr(&output)
    );
    stdout(&output)
}
