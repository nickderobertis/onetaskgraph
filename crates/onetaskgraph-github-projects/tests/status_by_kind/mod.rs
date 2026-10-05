//! `status_mapping` scoped by item kind, driven through the real `TaskSource` over the
//! loopback board this suite's other tests use.
//!
//! A task and a project are both issues on one board, and both kinds' names are options of
//! its one `Status` field — so every case here holds the two halves of one mapping against
//! each other: what a task is written and read as, what a project is, and what neither may
//! fall back to.

use super::*;

/// Every category, in the contract's order.
const EVERY: [StatusCategory; 8] = [
    StatusCategory::Draft,
    StatusCategory::Backlog,
    StatusCategory::Todo,
    StatusCategory::Queued,
    StatusCategory::InProgress,
    StatusCategory::Done,
    StatusCategory::Cancelled,
    StatusCategory::Unknown,
];

/// A category as `status_mapping` spells it.
fn key_of(category: StatusCategory) -> String {
    serde_json::to_value(category)
        .unwrap()
        .as_str()
        .unwrap()
        .to_owned()
}

/// The option `kind` maps `category` to under [`by_kind`]: different for the two kinds.
fn mapped(kind: ItemKind, category: StatusCategory) -> String {
    let prefix = match kind {
        ItemKind::Task => "Task",
        ItemKind::Project => "Plan",
    };
    format!("{prefix} {}", key_of(category))
}

/// A mapping naming all eight categories for both kinds, each kind its own options.
fn by_kind() -> Value {
    let mut mapping = serde_json::Map::new();
    for category in EVERY {
        mapping.insert(
            key_of(category),
            json!({"task": mapped(ItemKind::Task, category),
                   "project": mapped(ItemKind::Project, category)}),
        );
    }
    Value::Object(mapping)
}

/// A board whose `Status` field offers every option [`by_kind`] names, beside its own.
fn by_kind_board(items: Vec<Item>) -> Fixture {
    let fixture = board(items);
    for kind in [ItemKind::Task, ItemKind::Project] {
        for category in EVERY {
            let name = mapped(kind, category);
            fixture.offer_option(leaked(&format!("OPT_{name}")), leaked(&name));
        }
    }
    fixture
}

fn with_mapping(fixture: &Fixture, mapping: Value) -> Box<dyn TaskSource> {
    configured(&fixture.endpoint, json!({ "status_mapping": mapping }))
}

/// Build a source of `mapping`, answering the refusal or `None` when it loads.
fn loads(mapping: Value) -> Option<String> {
    Plugin
        .build(
            &SourceName::new("work").unwrap(),
            &fixture_config(
                "https://api.github.com/graphql",
                &json!({"status_mapping": mapping}),
            ),
            &Secrets,
        )
        .err()
        .map(|error| error.to_string())
}

#[test]
fn every_form_the_grammar_admits_loads() {
    for mapping in [
        json!({"todo": "Todo"}),
        json!({"todo": null}),
        json!({"done": {"task": "Done"}}),
        json!({"done": {"project": "Done"}}),
        json!({"done": {"task": "Done", "project": "Completed"}}),
        by_kind(),
        // The two configurations this host runs today, which must load unchanged.
        json!({"unknown": "Needs attention", "queued": "Queued", "done": "Done",
               "cancelled": "Cancelled"}),
        json!({"backlog": "Proposal", "draft": "Deferred", "queued": "Queued", "done": "Done",
               "cancelled": "Cancelled"}),
    ] {
        assert_eq!(loads(mapping.clone()), None, "{mapping} should load");
    }
}

#[test]
fn every_form_the_grammar_refuses_is_refused_naming_the_source_and_the_part() {
    for (mapping, part) in [
        (
            json!({"done": {}}),
            "status_mapping.done is an empty object",
        ),
        (
            json!({"done": {"task": "Done", "epic": "Done"}}),
            "status_mapping.done names \"epic\", which is not an item kind",
        ),
        (
            json!({"done": {"task": null}}),
            "status_mapping.done.task is null",
        ),
        (
            json!({"done": {"project": "  "}}),
            "status_mapping.done.project is blank",
        ),
        (json!({"done": ""}), "status_mapping.done is blank"),
        (
            json!({"shipped": "Shipped"}),
            "status_mapping names \"shipped\", which is not a status category",
        ),
    ] {
        let refused = loads(mapping.clone()).unwrap_or_else(|| panic!("{mapping} loaded"));
        assert!(
            refused.contains(&format!("source work: {part}")),
            "{mapping} was refused with {refused:?}, not naming the source and {part:?}"
        );
    }
}

#[test]
fn two_categories_on_one_option_of_one_kind_are_refused_ignoring_case() {
    for (mapping, said) in [
        (
            json!({"todo": "Todo", "queued": "TODO"}),
            "sends both todo and queued to the task status \"TODO\"",
        ),
        (
            json!({"todo": {"project": "Ready"}, "queued": {"project": "ready"}}),
            "sends both todo and queued to the project status \"ready\"",
        ),
        // A shipped default is a name like any other: `backlog` keeps `Backlog` for both
        // kinds, so a task's `queued` cannot be `backlog` too.
        (
            json!({"queued": {"task": "backlog"}}),
            "sends both backlog and queued to the task status \"backlog\"",
        ),
    ] {
        let refused = loads(mapping.clone()).unwrap_or_else(|| panic!("{mapping} loaded"));
        assert!(
            refused.contains(&format!("source work: status_mapping {said}")),
            "{mapping} was refused with {refused:?}"
        );
        assert_eq!(
            refused.matches("source work").count(),
            1,
            "the source is named once: {refused}"
        );
    }
}

#[test]
fn one_option_may_be_different_categories_of_the_two_kinds() {
    // `todo` and `queued` are both mentioned, so neither has a shipped default for the kind
    // it leaves out, and `Todo` is a task's `todo` and a project's `queued` alone.
    assert_eq!(
        loads(json!({"todo": {"task": "Todo"}, "queued": {"project": "Todo"}})),
        None
    );
}

#[tokio::test]
async fn every_category_is_written_and_read_back_for_each_kind_under_its_own_option() {
    let fixture = by_kind_board(vec![]);
    let writer = with_mapping(&fixture, by_kind());
    let mut written = Vec::new();
    for category in EVERY {
        let task_name = mapped(ItemKind::Task, category);
        let task_id = writer
            .write_task(&write(task("T", "one", status(category, &task_name))))
            .await
            .unwrap_or_else(|error| panic!("task {category:?}: {error}"));
        let plan_name = mapped(ItemKind::Project, category);
        let plan_id = writer
            .write_project(&write(project("P", "plan", status(category, &plan_name))))
            .await
            .unwrap_or_else(|error| panic!("project {category:?}: {error}"));
        for (id, name) in [(&task_id, &task_name), (&plan_id, &plan_name)] {
            let held = fixture.item(&id.0);
            assert_eq!(held.status.as_deref(), Some(name.as_str()), "{category:?}");
            let (state, reason) = match category {
                StatusCategory::Done => ("CLOSED", Some("COMPLETED")),
                StatusCategory::Cancelled => ("CLOSED", Some("NOT_PLANNED")),
                _ => ("OPEN", None),
            };
            assert_eq!(
                (held.state, held.state_reason.as_deref()),
                (state, reason),
                "{category:?} closes or leaves open either kind alike"
            );
        }
        written.push((category, task_id, task_name, plan_id, plan_name));
    }
    // A source that wrote nothing reads every one from the board rather than its record.
    let reader = with_mapping(&fixture, by_kind());
    for (category, task_id, task_name, plan_id, plan_name) in written {
        assert_eq!(
            reader.get_task(&task_id).await.unwrap().unwrap().status,
            status(category, &task_name),
        );
        assert_eq!(
            reader.get_project(&plan_id).await.unwrap().unwrap().status,
            status(category, &plan_name),
        );
    }
}

/// What a refused status write must have said, and that the board received no mutation.
fn refused_unwritten(
    fixture: &Fixture,
    error: SourceError,
    kind: &str,
    category: &str,
    also: &[&str],
) {
    let message = error.to_string();
    for part in [
        "work",
        &format!("{kind} status"),
        category,
        &format!("status_mapping.{category}.{kind}"),
    ]
    .into_iter()
    .chain(also.iter().copied())
    {
        assert!(message.contains(part), "{message} does not name {part:?}");
    }
    assert!(
        fixture.seen().is_empty(),
        "a refused status write sends no mutation: {:?}",
        fixture.seen()
    );
}

/// Every way this source writes a status, against items `T_1` (a task) and `P_1` (a
/// project) of `fixture`, each answered with what it was refused with.
async fn every_status_write(
    source: &dyn TaskSource,
    kind: ItemKind,
    category: StatusCategory,
) -> Vec<(&'static str, Result<(), SourceError>)> {
    let wanted = status(category, "as configured");
    let here = vec![Repository::try_from("github.com/acme/work".to_owned()).unwrap()];
    match kind {
        ItemKind::Task => {
            let mut existing = task("T", "one", wanted.clone());
            existing.repositories.clone_from(&here);
            vec![
                (
                    "create",
                    source
                        .write_task(&write(task("T", "new", wanted.clone())))
                        .await
                        .map(drop),
                ),
                (
                    "copy onto an existing task",
                    source
                        .write_task(&ItemWrite {
                            target: Some(id("T_1")),
                            item: existing,
                            depends_on: vec![],
                        })
                        .await
                        .map(drop),
                ),
                (
                    "status set",
                    source.set_task_status(&id("T_1"), category).await.map(drop),
                ),
                (
                    "targeted update",
                    source
                        .update_task(
                            &id("T_1"),
                            &TaskUpdate {
                                status: Some(wanted),
                                ..TaskUpdate::default()
                            },
                        )
                        .await
                        .map(drop),
                ),
            ]
        }
        ItemKind::Project => {
            let mut existing = project("P", "plan", wanted.clone());
            existing.repositories = here;
            vec![
                (
                    "create",
                    source
                        .write_project(&write(project("P", "new plan", wanted)))
                        .await
                        .map(drop),
                ),
                (
                    "update",
                    source
                        .write_project(&ItemWrite {
                            target: Some(id("P_1")),
                            item: existing,
                            depends_on: vec![],
                        })
                        .await
                        .map(drop),
                ),
            ]
        }
    }
}

fn written_board() -> Fixture {
    by_kind_board(vec![
        Item::issue("T_1", "one").status("Todo"),
        Item::issue("P_1", "plan").sub_issues(1).status("Todo"),
    ])
}

#[tokio::test]
async fn a_status_a_kind_has_no_option_for_is_refused_on_every_write_before_any_mutation() {
    // Each case: the mapping, the kind written, the category, and what the refusal says why.
    let cases = [
        // Unconfigured, and no shipped default.
        (
            json!({}),
            ItemKind::Task,
            StatusCategory::Draft,
            "does not name draft",
        ),
        (
            json!({}),
            ItemKind::Project,
            StatusCategory::Unknown,
            "does not name unknown",
        ),
        // Disabled for every kind.
        (
            json!({"backlog": null}),
            ItemKind::Task,
            StatusCategory::Backlog,
            "sets backlog to null",
        ),
        (
            json!({"backlog": null}),
            ItemKind::Project,
            StatusCategory::Backlog,
            "sets backlog to null",
        ),
        // Missing from a per-kind object: configured for the other kind only, so the
        // shipped default `Done` is not the fallback.
        (
            json!({"done": {"task": "Done"}}),
            ItemKind::Project,
            StatusCategory::Done,
            "names done for the other kind only",
        ),
        (
            json!({"in-progress": {"project": "In Progress"}}),
            ItemKind::Task,
            StatusCategory::InProgress,
            "names in-progress for the other kind only",
        ),
    ];
    for (mapping, kind, category, why) in cases {
        let fixture = written_board();
        let source = with_mapping(&fixture, mapping.clone());
        for (path, outcome) in every_status_write(source.as_ref(), kind, category).await {
            let error = outcome
                .err()
                .unwrap_or_else(|| panic!("{mapping}: a {kind:?} {category:?} {path} was written"));
            refused_unwritten(&fixture, error, kind.marker(), &key_of(category), &[why]);
        }
    }
}

#[tokio::test]
async fn a_mapped_option_the_board_lacks_is_refused_on_every_write_before_any_mutation() {
    for kind in [ItemKind::Task, ItemKind::Project] {
        for category in [StatusCategory::Queued, StatusCategory::Done] {
            let fixture = written_board();
            let lacking = mapped(kind, category);
            fixture
                .state
                .lock()
                .unwrap()
                .options
                .retain(|(_, name)| *name != lacking);
            let source = with_mapping(&fixture, by_kind());
            for (path, outcome) in every_status_write(source.as_ref(), kind, category).await {
                let error = outcome.err().unwrap_or_else(|| {
                    panic!("a {kind:?} {category:?} {path} was written without its option")
                });
                refused_unwritten(
                    &fixture,
                    error,
                    kind.marker(),
                    &key_of(category),
                    &[&format!("{lacking:?}"), "sources fields work --apply"],
                );
            }
        }
    }
}

#[tokio::test]
async fn an_open_item_reads_through_its_own_kinds_mapping_and_a_closed_one_by_its_state() {
    let fixture = by_kind_board(vec![
        // The task's `todo` option, on a task and on a project.
        Item::issue("T_todo", "a").status("Task todo"),
        Item::issue("P_task_todo", "b")
            .sub_issues(1)
            .status("Task todo"),
        // The project's `todo` option, on a project and on a task.
        Item::issue("P_todo", "c").sub_issues(1).status("Plan todo"),
        Item::issue("T_plan_todo", "d").status("Plan todo"),
        // Closed, at an option neither kind maps to anything.
        Item::issue("T_done", "e")
            .closed(Some("COMPLETED"))
            .status("Shipped"),
        Item::issue("P_done", "f")
            .sub_issues(1)
            .closed(Some("COMPLETED"))
            .status("Shipped"),
        Item::issue("T_cancelled", "g")
            .closed(Some("NOT_PLANNED"))
            .status("Plan todo"),
        Item::issue("P_cancelled", "h")
            .sub_issues(1)
            .closed(Some("NOT_PLANNED"))
            .status("Task todo"),
    ]);
    let source = with_mapping(&fixture, by_kind());
    for (task_id, expected) in [
        ("T_todo", status(StatusCategory::Todo, "Task todo")),
        ("T_plan_todo", status(StatusCategory::Unknown, "Plan todo")),
        ("T_done", status(StatusCategory::Done, "Shipped")),
        (
            "T_cancelled",
            status(StatusCategory::Cancelled, "Plan todo"),
        ),
    ] {
        assert_eq!(
            source.get_task(&id(task_id)).await.unwrap().unwrap().status,
            expected,
            "{task_id}"
        );
    }
    for (project_id, expected) in [
        ("P_todo", status(StatusCategory::Todo, "Plan todo")),
        ("P_task_todo", status(StatusCategory::Unknown, "Task todo")),
        ("P_done", status(StatusCategory::Done, "Shipped")),
        (
            "P_cancelled",
            status(StatusCategory::Cancelled, "Task todo"),
        ),
    ] {
        assert_eq!(
            source
                .get_project(&id(project_id))
                .await
                .unwrap()
                .unwrap()
                .status,
            expected,
            "{project_id}"
        );
    }

    // `--status` narrows each kind by that kind's own reading, `unknown` included.
    for (statuses, tasks, projects) in [
        (vec![StatusCategory::Todo], vec!["T_todo"], vec!["P_todo"]),
        (
            vec![StatusCategory::Unknown],
            vec!["T_plan_todo"],
            vec!["P_task_todo"],
        ),
        (
            vec![StatusCategory::Done, StatusCategory::Cancelled],
            vec!["T_cancelled", "T_done"],
            vec!["P_cancelled", "P_done"],
        ),
    ] {
        let mut got = selected_tasks(
            source.as_ref(),
            &TaskQuery {
                statuses: statuses.clone(),
                ..TaskQuery::default()
            },
        )
        .await;
        got.sort();
        assert_eq!(got, tasks, "tasks at {statuses:?}");
        let mut got = selected_projects(
            source.as_ref(),
            &ProjectQuery {
                statuses: statuses.clone(),
                ..ProjectQuery::default()
            },
        )
        .await;
        got.sort();
        assert_eq!(got, projects, "projects at {statuses:?}");
    }
}

#[tokio::test]
async fn a_bare_name_mapping_reads_and_writes_both_kinds_exactly_as_before() {
    // `plans`, as this host configures it: every name bare, so both kinds share it.
    let mapping = json!({"unknown": "Needs attention", "queued": "Queued", "done": "Done",
                         "cancelled": "Cancelled"});
    let fixture = queued_board(vec![
        Item::issue("T_1", "one").status("Needs attention"),
        Item::issue("P_1", "plan").sub_issues(1).status("Queued"),
    ]);
    fixture.offer_option("OPT_attention", "Needs attention");
    let source = with_mapping(&fixture, mapping.clone());
    assert_eq!(
        source.get_task(&id("T_1")).await.unwrap().unwrap().status,
        status(StatusCategory::Unknown, "Needs attention")
    );
    assert_eq!(
        source
            .get_project(&id("P_1"))
            .await
            .unwrap()
            .unwrap()
            .status,
        status(StatusCategory::Queued, "Queued")
    );
    // Unmentioned categories keep their shipped option for both kinds.
    let task_id = source
        .write_task(&write(task(
            "T",
            "new",
            status(StatusCategory::Todo, "Todo"),
        )))
        .await
        .unwrap();
    let plan_id = source
        .write_project(&write(project(
            "P",
            "new",
            status(StatusCategory::InProgress, "In Progress"),
        )))
        .await
        .unwrap();
    assert_eq!(fixture.item(&task_id.0).status.as_deref(), Some("Todo"));
    assert_eq!(
        fixture.item(&plan_id.0).status.as_deref(),
        Some("In Progress")
    );
    let reader = with_mapping(&fixture, mapping);
    assert_eq!(
        reader.get_project(&plan_id).await.unwrap().unwrap().status,
        status(StatusCategory::InProgress, "In Progress")
    );
}
