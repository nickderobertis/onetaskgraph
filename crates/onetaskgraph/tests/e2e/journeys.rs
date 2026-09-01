//! Shared journeys over complete-dataset rows, plus focused journeys for sources whose
//! native model cannot represent that whole dataset.
//!
//! Every one of them drives the compiled binary as a subprocess. Where a journey asserts
//! on the plan as well as the rows, it asserts what *this row declares* — so the same
//! journey proves that a source applying a predicate natively has it pushed down and that
//! a source applying none of them still returns the correct rows.

use std::process::Output;

use crate::common::{Sandbox, stderr, stdout};
use crate::fixtures::{Declared, ROWS, Row, SOURCE, dataset, document, qualified};
use onetaskgraph_plugin_api::Capabilities;
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

/// The value of one field of a detail rendering, whatever the label column is padded to.
///
/// Read by label rather than by column width because the width is the widest label
/// present, so a row that carries a `location` line pads every other label one further —
/// and an assertion written against a literal run of spaces would be asserting on which
/// *other* fields the source happened to give.
fn field(rendered: &str, name: &str) -> Option<String> {
    rendered
        .lines()
        .find_map(|line| line.trim_end().strip_prefix(&format!("{name}:")))
        .map(|value| value.trim().to_owned())
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
    // This source applies every predicate a query carries, so the engine pushes them all
    // down and applies nothing of its own.
    plan_says(row, &filtered, "pushed down", "label");
    plan_says(row, &filtered, "pushed down", "status");

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
    plan_says(row, &searched, "pushed down", "search-title");
    plan_says(row, &searched, "pushed down", "search-content");

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

/// One capability field, the invocation that drives it, and what a correct answer is.
///
/// The rows a query keeps are the same whoever applies the predicate — that is the whole
/// claim of this product — so `expected` does not vary by row. What varies is the *plan*,
/// and that is read off the row's own declaration: `pushed down` where it declares the
/// field native, and the field's own compensation wording where it does not.
struct Probe {
    /// The [`Capabilities`] field this drives.
    field: &'static str,
    /// The `--explain` invocation that exercises it.
    arguments: Vec<String>,
    /// The ids the answer must hold, in order.
    expected: Vec<String>,
    /// The name the plan reports this predicate under.
    predicate: &'static str,
    /// Whether the row declares this field native.
    native: bool,
    /// What the plan says when the engine did the work instead.
    compensated: &'static str,
    /// How ids are read out of this answer.
    read: fn(&str) -> Vec<String>,
}

/// Every `Support`- and `DependencySupport`-typed field of the contract's capability
/// value, one probe each, as this row declares them.
fn probes(declared: &Declared) -> Vec<Probe> {
    let filter = |field, arguments: &[&str], expected: &[&str], predicate, native| Probe {
        field,
        arguments: arguments
            .iter()
            .map(|argument| (*argument).to_owned())
            .collect(),
        expected: ours(expected),
        predicate,
        native,
        compensated: "applied locally",
        read: listed,
    };
    let reverse = |field, verb: &str, of: &str, expected: &[&str], native| Probe {
        field,
        arguments: vec![
            verb.to_owned(),
            "deps".to_owned(),
            qualified(SOURCE, of),
            "--direction".to_owned(),
            "depended-on-by".to_owned(),
            "--explain".to_owned(),
        ],
        expected: ours(expected),
        predicate: "reverse-dependencies",
        native,
        compensated: "emulated",
        read: edge_starts,
    };
    vec![
        filter(
            "projects",
            &[
                "task",
                "list",
                "--project",
                &qualified(SOURCE, "P-1"),
                "--explain",
            ],
            &["T-1", "T-2"],
            "project",
            declared.projects.is_native(),
        ),
        filter(
            "orphan_tasks",
            &["task", "list", "--no-project", "--explain"],
            &["T-3"],
            "project",
            declared.orphan_tasks.is_native(),
        ),
        filter(
            "filter_by_label",
            &["task", "list", "--label", "bug", "--explain"],
            &["T-1", "T-3"],
            "label",
            declared.filter_by_label.is_native(),
        ),
        filter(
            "filter_by_status",
            &["task", "list", "--status", "todo", "--explain"],
            &["T-1", "T-3"],
            "status",
            declared.filter_by_status.is_native(),
        ),
        filter(
            "search_title",
            &[
                "task",
                "list",
                "--search",
                "alpha",
                "--in",
                "title",
                "--explain",
            ],
            &["T-1"],
            "search-title",
            declared.search_title.is_native(),
        ),
        filter(
            "search_content",
            &[
                "task",
                "list",
                "--search",
                "alpha",
                "--in",
                "content",
                "--explain",
            ],
            &["T-2"],
            "search-content",
            declared.search_content.is_native(),
        ),
        reverse(
            "task_dependencies",
            "task",
            "T-2",
            &["T-1", "T-3", "T-4"],
            declared.task_dependencies.answers_reverse(),
        ),
        reverse(
            "project_dependencies",
            "project",
            "P-2",
            &["P-1"],
            declared.project_dependencies.answers_reverse(),
        ),
    ]
}

#[test]
fn every_row_declares_exactly_what_its_plugin_reports() {
    // The table is what every shared journey's plan assertion is written against, so a row
    // claiming a capability its plugin does not report makes those assertions prove the
    // table rather than the product. `sources list` is where the binary reports what a
    // configured source declares, so this reconciles the two at the same boundary a user
    // reads them from — and names the row and the field when they part.
    for row in ROWS {
        let sandbox = host(row);
        let listing: serde_json::Value =
            serde_json::from_str(&ok(row, &sandbox, &["sources", "list", "--json"]))
                .expect("sources list emits JSON");
        let entry = listing
            .as_array()
            .expect("a list of configured sources")
            .iter()
            .find(|entry| entry["source"] == json!(SOURCE))
            .unwrap_or_else(|| panic!("{}: no source called {SOURCE}:\n{listing:#}", row.name));
        assert_eq!(
            entry["state"],
            json!("available"),
            "{}: the row's own source must build:\n{entry:#}",
            row.name
        );
        let reported: Capabilities = serde_json::from_value(entry["capabilities"].clone())
            .expect("a source reports the contract's capability value");
        let disagreements = row.declared().disagreements(&reported);
        assert!(
            disagreements.is_empty(),
            "{}: the journey table and the plugin disagree — {}",
            row.name,
            disagreements.join("; ")
        );
    }
}

#[test]
fn every_row_drives_every_capability_field_and_the_plan_says_who_applied_it() {
    // One journey per row per *predicate* field of the plugin contract's capability value.
    // A field a row declares native is driven and the plan must say the source applied it;
    // a field a row declares unsupported is driven to the *same answer* and the plan must
    // say the engine did — which is the property that makes an unsupported declaration
    // sound rather than merely honest.
    //
    // `documents` has no probe here and must not grow one while it stays unsupported
    // everywhere: it is not a predicate, no verb reaches it, and there is nothing for the
    // engine to compensate. What holds it honest is the reconciliation above — every row
    // declares it and every plugin is made to report the same — and the plugin-host journey
    // in `source_host.rs`, which drives the refusal itself over a real pipe.
    for row in complete_dataset_rows() {
        let sandbox = host(row);
        for probe in probes(row.declared()) {
            let arguments: Vec<&str> = probe.arguments.iter().map(String::as_str).collect();
            let rendered = ok(row, &sandbox, &arguments);
            assert_eq!(
                (probe.read)(&rendered),
                probe.expected,
                "{}: {} returns the same answer however it is applied:\n{rendered}",
                row.name,
                probe.field
            );
            plan_says(
                row,
                &rendered,
                if probe.native {
                    "pushed down"
                } else {
                    probe.compensated
                },
                probe.predicate,
            );
        }

        // `max_page_size` is the last field, and it is not a predicate: it is the ceiling
        // the engine asks a source for rows in. So it is driven twice — the declaration as
        // the binary reports it to a user, and the behaviour behind it: a caller asking for
        // four rows gets four, assembled from as many source pages as the ceiling forces.
        // The compensated row's ceiling is two, so that walk really is more than one page.
        let ceiling = row.declared().max_page_size;
        let sources = ok(row, &sandbox, &["sources", "list"]);
        assert!(
            sources.contains(&format!("page <= {ceiling}")),
            "{}: the page ceiling a source declares reaches the user:\n{sources}",
            row.name
        );
        let whole = ok(row, &sandbox, &["task", "list", "--limit", "4"]);
        assert_eq!(
            listed(&whole),
            ours(&["T-1", "T-2", "T-3", "T-4"]),
            "{}: a page larger than the source's ceiling is filled from several of them",
            row.name
        );
    }
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
        assert_eq!(
            field(&shown, "project").as_deref(),
            Some(qualified(SOURCE, "P-1").as_str()),
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
            if declared.orphan_tasks.is_native() {
                "pushed down"
            } else {
                "applied locally"
            },
            "project",
        );

        // The other way round: tasks of one project, qualified. The plan is asserted too,
        // because `projects` is the field that says whether the source scoped the listing
        // itself — a source declaring it native and then ignoring it returns another
        // project's tasks, and the engine, trusting the declaration, applies nothing.
        let of_project = ok(
            row,
            &sandbox,
            &[
                "task",
                "list",
                "--project",
                &qualified(SOURCE, "P-1"),
                "--explain",
            ],
        );
        assert_eq!(listed(&of_project), ours(&["T-1", "T-2"]), "{}", row.name);
        plan_says(
            row,
            &of_project,
            if declared.projects.is_native() {
                "pushed down"
            } else {
                "applied locally"
            },
            "project",
        );
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
        let outcome = if declared.filter_by_label.is_native() {
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
            if declared.filter_by_status.is_native() {
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
            (
                "title",
                "search-title",
                declared.search_title.is_native(),
                vec!["T-1"],
            ),
            (
                "content",
                "search-content",
                declared.search_content.is_native(),
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
            if declared.task_dependencies.answers_reverse() {
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
            if declared.project_dependencies.answers_reverse() {
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
            if declared.filter_by_label.is_native() {
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
            if declared.filter_by_status.is_native() {
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
            if declared.filter_by_status.is_native() {
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
        assert_eq!(
            field(&orphan, "project").as_deref(),
            Some("none"),
            "{}: a task in no project says so:\n{orphan}",
            row.name
        );

        // And a field the source did not give is left out entirely. The field asserted
        // absent is `labels`, not `url`: every GitHub issue has a url by construction, so
        // a board fixture claiming an item without one would be modelling something GitHub
        // cannot return — and the property under test is the renderer's, which any
        // genuinely absent field proves. P-2 carries no label in any row's fixture.
        let unlabelled = ok(
            row,
            &sandbox,
            &["project", "show", &qualified(SOURCE, "P-2")],
        );
        assert!(
            !unlabelled.contains("labels:"),
            "{}: a field the source did not give is left out entirely:\n{unlabelled}",
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
    // One task and nothing else: this fixture is about one metadata key, and a document
    // table beside it would be work no assertion here reads.
    memory["documents"] = json!([]);
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

/// Rows whose source holds documents, and rows that declare it holds none.
///
/// Both halves are driven by the journeys below: a row that declares no documents asserts
/// the honest answer — reported as holding none, never as having failed — rather than
/// skipping, which would leave the honest-capability rule proven by nothing.
fn documentary_rows() -> impl Iterator<Item = &'static Row> {
    ROWS.iter()
        .filter(|row| row.declared().documents.is_native())
}

fn documentless_rows() -> impl Iterator<Item = &'static Row> {
    ROWS.iter()
        .filter(|row| !row.declared().documents.is_native())
}

#[test]
fn every_row_lists_the_documents_it_holds_and_shows_one_by_its_qualified_id() {
    for row in documentary_rows() {
        let sandbox = host(row);

        let listing = ok(row, &sandbox, &["document", "list"]);
        assert_eq!(
            listed(&listing),
            ours(&["D-1", "D-2", "D-3"]),
            "{}: every document, in the source's own order",
            row.name
        );
        assert!(
            listing.contains("Alpha design") && listing.contains("Loose note"),
            "{}: a list carries each document's title:\n{listing}",
            row.name
        );

        let shown = ok(
            row,
            &sandbox,
            &["document", "show", &qualified(SOURCE, "D-1")],
        );
        for expected in ["Alpha design", "bug", "the engine core, reviewed"] {
            assert!(
                shown.contains(expected),
                "{}: `document show` omits {expected}:\n{shown}",
                row.name
            );
        }
        assert_eq!(
            field(&shown, "project").as_deref(),
            Some(qualified(SOURCE, "P-1").as_str()),
            "{}: a document's project is qualified too:\n{shown}",
            row.name
        );
        // A document is not work, so it has no status to show and no graph to walk.
        assert!(
            field(&shown, "status").is_none(),
            "{}: a document carries no status:\n{shown}",
            row.name
        );
        let orphan = ok(
            row,
            &sandbox,
            &["document", "show", &qualified(SOURCE, "D-3")],
        );
        assert_eq!(
            field(&orphan, "project").as_deref(),
            Some("none"),
            "{}: a document in no project says so:\n{orphan}",
            row.name
        );
    }
}

#[test]
fn a_source_declaring_it_has_no_documents_is_reported_as_holding_none_rather_than_as_failing() {
    // The honest-capability rule, end to end: the engine read the declaration once and did
    // not ask, so the run succeeds, the answer is empty, and the plan says why — rather
    // than the source appearing as one that could not answer.
    for row in documentless_rows() {
        let sandbox = host(row);

        let output = run(&sandbox, &["document", "list", "--explain"]);
        assert_eq!(
            output.status.code(),
            Some(0),
            "{}: a source that holds no documents has not failed\n{}",
            row.name,
            stderr(&output)
        );
        let rendered = stdout(&output);
        assert!(
            listed(&rendered).is_empty(),
            "{}: it holds none:\n{rendered}",
            row.name
        );
        plan_says(row, &rendered, "unavailable", "document");
        assert!(
            !stderr(&output).contains("could not answer"),
            "{}: holding none is not failing:\n{}",
            row.name,
            stderr(&output)
        );

        // And one document of it by id: nothing there either, said as plainly.
        let shown = run(&sandbox, &["document", "show", &qualified(SOURCE, "D-1")]);
        assert_eq!(shown.status.code(), Some(1), "{}", row.name);
        assert!(
            stderr(&shown).contains("no document with that id"),
            "{}: {}",
            row.name,
            stderr(&shown)
        );
    }
}

#[test]
fn a_document_list_narrows_by_project_by_label_and_by_text_and_has_no_status_filter() {
    for row in documentary_rows() {
        let sandbox = host(row);

        let in_project = ok(
            row,
            &sandbox,
            &[
                "document",
                "list",
                "--project",
                &qualified(SOURCE, "P-1"),
                "--explain",
            ],
        );
        assert_eq!(listed(&in_project), ours(&["D-1"]), "{}", row.name);

        let orphans = ok(
            row,
            &sandbox,
            &["document", "list", "--no-project", "--explain"],
        );
        assert_eq!(listed(&orphans), ours(&["D-3"]), "{}", row.name);

        // The same rows whoever applied the predicate, and a plan that says which it was.
        let labelled = ok(
            row,
            &sandbox,
            &["document", "list", "--label", "bug", "--explain"],
        );
        assert_eq!(listed(&labelled), ours(&["D-1"]), "{}", row.name);
        plan_says(
            row,
            &labelled,
            if row.declared().filter_by_label.is_native() {
                "pushed down"
            } else {
                "applied locally"
            },
            "label",
        );

        let excluded = ok(row, &sandbox, &["document", "list", "--not-label", "bug"]);
        assert_eq!(listed(&excluded), ours(&["D-2", "D-3"]), "{}", row.name);

        // `D-1` matches in its title and `D-2` in its body, so a search over either has to
        // return both — which is what a source that can only search one half must not
        // half-apply.
        let searched = ok(
            row,
            &sandbox,
            &[
                "document",
                "list",
                "--search",
                "alpha design",
                "--in",
                "both",
                "--explain",
            ],
        );
        assert_eq!(listed(&searched), ours(&["D-1", "D-2"]), "{}", row.name);
        plan_says(
            row,
            &searched,
            if row.declared().search_title.is_native() && row.declared().search_content.is_native()
            {
                "pushed down"
            } else {
                "applied locally"
            },
            "search-title",
        );

        let by_title = ok(
            row,
            &sandbox,
            &[
                "document",
                "list",
                "--search",
                "alpha design",
                "--in",
                "title",
            ],
        );
        assert_eq!(listed(&by_title), ours(&["D-1"]), "{}", row.name);

        // No status filter, because a document has no status: the flag is not a flag this
        // verb has, and clap refuses the invocation rather than accepting and ignoring it.
        let with_status = run(&sandbox, &["document", "list", "--status", "todo"]);
        assert_eq!(
            with_status.status.code(),
            Some(2),
            "{}: `document list` has no --status:\n{}",
            row.name,
            stderr(&with_status)
        );

        // And no dependency verb, for the same reason: nothing may point at a document.
        let deps = run(&sandbox, &["document", "deps", &qualified(SOURCE, "D-1")]);
        assert_eq!(
            deps.status.code(),
            Some(2),
            "{}: `document deps` is not a verb:\n{}",
            row.name,
            stderr(&deps)
        );
    }
}

#[test]
fn both_renderings_report_where_an_entity_is_for_documents_tasks_and_projects_alike() {
    for row in documentary_rows() {
        let sandbox = host(row);

        // The human rendering says which kind of place it is, so a reader knows whether to
        // open a link or read a file out.
        for (verb, id, expected) in [
            ("document", "D-1", "url https://example.invalid/D-1"),
            ("document", "D-2", "path /srv/notes/D-2.md"),
            ("task", "T-1", "url https://example.invalid/T-1"),
            ("project", "P-1", "path /srv/engine"),
        ] {
            let shown = ok(row, &sandbox, &[verb, "show", &qualified(SOURCE, id)]);
            assert_eq!(
                field(&shown, "location").as_deref(),
                Some(expected),
                "{}: `{verb} show {id}` must say where it is and which kind of place:\n{shown}",
                row.name
            );
        }

        // A source that did not say where an entity is leaves the line out entirely,
        // which is not the same as saying it is nowhere.
        let unplaced = ok(
            row,
            &sandbox,
            &["document", "show", &qualified(SOURCE, "D-3")],
        );
        assert!(
            field(&unplaced, "location").is_none(),
            "{}: a location the source did not give is left out:\n{unplaced}",
            row.name
        );

        // The machine rendering carries the contract type's own JSON, so a consumer
        // branches on which key is present rather than parsing a sentence.
        for (verb, id, key, place) in [
            ("document", "D-1", "url", "https://example.invalid/D-1"),
            ("document", "D-2", "path", "/srv/notes/D-2.md"),
            ("task", "T-1", "url", "https://example.invalid/T-1"),
            ("project", "P-1", "path", "/srv/engine"),
        ] {
            let response: serde_json::Value = serde_json::from_str(&ok(
                row,
                &sandbox,
                &[verb, "show", &qualified(SOURCE, id), "--json"],
            ))
            .expect("a show emits JSON");
            let location = &response["items"][0]["item"]["location"];
            assert_eq!(
                location,
                &json!({key: place}),
                "{}: `{verb} show {id} --json` carries the location's own JSON:\n{location}",
                row.name
            );
        }
        let none: serde_json::Value = serde_json::from_str(&ok(
            row,
            &sandbox,
            &["document", "show", &qualified(SOURCE, "D-3"), "--json"],
        ))
        .expect("a show emits JSON");
        assert_eq!(
            none["items"][0]["item"]["location"],
            json!(null),
            "{}: a source that did not say reports null, not a third variant",
            row.name
        );

        // And the list rendering says it too, because a document list is where a reader
        // finds the thing they were asked to open.
        let listing = ok(row, &sandbox, &["document", "list"]);
        assert!(
            listing.contains("url https://example.invalid/D-1")
                && listing.contains("path /srv/notes/D-2.md"),
            "{}: a document list says where each one is:\n{listing}",
            row.name
        );
    }
}

#[test]
fn a_document_walk_pages_to_exhaustion_in_a_stable_order() {
    for row in documentary_rows() {
        let sandbox = host(row);
        let mut walked = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let mut arguments = vec!["document", "list", "--limit", "1"];
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
        assert_eq!(walked, ours(&["D-1", "D-2", "D-3"]), "{}", row.name);
    }
}

#[test]
fn writing_an_items_identifier_where_its_title_belongs_is_caught_by_every_row() {
    // A shipped defect wrote a project's *identifier* into the field its *title* belongs
    // in, and no test could have caught it: every fixture spelled the two the same, so the
    // wrong value and the right value were the same bytes.
    //
    // This is the property that makes the assertions everywhere else in this suite able to
    // fail. For every source kind, over every task and project it serves, it takes the
    // value the defect would have written — the item's own identifier — and asserts that
    // the assertion a journey makes about the title *rejects* it. A fixture that stops
    // discriminating fails here rather than quietly making its whole row untestable.
    //
    // `scripts/check-store-fixtures.sh` holds the same rule over the JSON fixtures the
    // plugin suites serve; this holds it over the datasets these journeys are written
    // against, which are Rust and out of that check's reach.
    for row in ROWS {
        let sandbox = host(row);
        // The document verbs only where the row's source has documents; a row that holds
        // none has no document rows for this to discriminate between, and the honest
        // refusal it answers with is what that row's own journey asserts.
        let entities: &[(&str, &str)] = if row.declared().documents.is_native() {
            &[
                ("task", "tasks"),
                ("project", "projects"),
                ("document", "documents"),
            ]
        } else {
            &[("task", "tasks"), ("project", "projects")]
        };
        for &(verb, noun) in entities {
            let rendered = ok(row, &sandbox, &[verb, "list", "--json"]);
            let response: serde_json::Value =
                serde_json::from_str(&rendered).expect("a list emits JSON");
            let items = response["items"]
                .as_array()
                .expect("a list response carries items");
            assert!(
                !items.is_empty(),
                "{}: this row serves no {noun}, so nothing below is being checked",
                row.name
            );
            for item in items {
                let qualified = item["id"].as_str().expect("a qualified id");
                let native = qualified
                    .split_once(':')
                    .expect("a qualified id is <source>:<native>")
                    .1;
                let title = item["item"]["title"].as_str().expect("a title");
                // What a journey asserts is that the title is what the source said. The
                // substitution under test writes the identifier there instead — so this
                // fixture discriminates exactly when that assertion would fail.
                assert_ne!(
                    native, title,
                    "{}: {qualified} is titled with its own identifier, so writing the \
                     identifier where the title belongs is a change no assertion over this \
                     row can see",
                    row.name
                );
            }
        }
    }
}
