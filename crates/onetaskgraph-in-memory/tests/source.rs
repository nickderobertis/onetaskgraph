//! The in-memory source, driven through the real trait.
//!
//! Every test here configures the *same* work with a different capability block,
//! because that difference is the whole reason this plugin exists: the engine's
//! compensation can only be exercised against two sources that genuinely disagree
//! about what they can do.

mod common;

use common::with_capabilities;
use onetaskgraph_in_memory::{InMemoryConfig, Plugin};
use onetaskgraph_plugin_api::{
    Cursor, DependencyEdge, DependencyEndpoint, DependencyKind, DependencySupport, Direction,
    ItemKind, ItemWrite, LabelFilter, NativeId, Page, PageRequest, ProjectFilter, ProjectQuery,
    SecretResolver, SourceError, SourceName, SourcePlugin, Support, Task, TaskQuery, TaskSource,
    TextFields, TextQuery,
};
use secrecy::SecretString;
use serde_json::json;

/// An in-memory source needs no credential, so this resolver defines nothing.
struct NoSecrets;
impl SecretResolver for NoSecrets {
    fn get(&self, _var: &str) -> Option<SecretString> {
        None
    }
}

/// Everything native, both dependency tables answered in both directions.
fn fully_capable() -> Box<dyn TaskSource> {
    Box::new(
        onetaskgraph_in_memory::InMemorySource::new(with_capabilities(json!({})))
            .expect("the fixture graph is coherent"),
    )
}

/// The same work, but the source ignores labels and content search, has no
/// projects, and answers dependencies forward only.
fn poorly_capable() -> Box<dyn TaskSource> {
    Box::new(
        onetaskgraph_in_memory::InMemorySource::new(with_capabilities(json!({
            "projects": "unsupported",
            "orphan_tasks": "unsupported",
            "filter_by_label": "unsupported",
            "filter_by_status": "native",
            "search_title": "native",
            "search_content": "unsupported",
            "task_dependencies": "forward-only",
            "project_dependencies": "forward-only",
            "max_page_size": 2,
        })))
        .expect("the fixture graph is coherent"),
    )
}

/// A page big enough to hold the whole fixture.
fn whole() -> PageRequest {
    PageRequest {
        cursor: None,
        limit: 50,
    }
}

fn ids<T>(page: &Page<T>, id: impl Fn(&T) -> String) -> Vec<String> {
    page.items.iter().map(id).collect()
}

fn task_ids(page: &Page<Task>) -> Vec<String> {
    ids(page, |task| task.id.to_string())
}

#[tokio::test]
async fn a_source_declares_the_capabilities_its_own_config_block_set() {
    // Load-bearing: DependencySupport comes from configuration, so two sources
    // over one graph can differ deliberately.
    let rich = fully_capable().capabilities();
    assert_eq!(rich.task_dependencies, DependencySupport::BothDirections);
    assert_eq!(rich.filter_by_label, Support::Native);
    assert_eq!(rich.max_page_size, 100);

    let poor = poorly_capable().capabilities();
    assert_eq!(poor.task_dependencies, DependencySupport::ForwardOnly);
    assert_eq!(poor.filter_by_label, Support::Unsupported);
    assert_eq!(poor.max_page_size, 2);

    assert_eq!(fully_capable().kind(), "in-memory");
}

#[tokio::test]
async fn a_source_reports_healthy_and_says_what_it_holds() {
    let health = fully_capable()
        .health()
        .await
        .expect("in memory is reachable");
    assert!(health.reachable);
    // The whole rendered detail, not a substring of it: `health` reports both counts, and
    // asserting only the task count left the project count — and the rendering that joins
    // them — as observable output nothing checked. The fixture holds four tasks and two
    // projects; a detail that miscounts either, or stops naming one, fails here.
    assert_eq!(
        health.detail.as_deref(),
        Some("4 task(s), 2 project(s) held in memory"),
        "{health:?}"
    );
}

#[tokio::test]
async fn one_item_is_fetched_by_its_native_id_and_a_miss_is_none() {
    let source = fully_capable();

    let task = source
        .get_task(&NativeId::from("T-1"))
        .await
        .expect("answers")
        .expect("T-1 exists");
    assert_eq!(task.title, "Land the contract");
    assert_eq!(task.status.name, "In Review");

    let project = source
        .get_project(&NativeId::from("P-2"))
        .await
        .expect("answers")
        .expect("P-2 exists");
    assert_eq!(project.title, "Plugins");

    assert!(
        source
            .get_task(&NativeId::from("nope"))
            .await
            .expect("answers")
            .is_none()
    );
    assert!(
        source
            .get_project(&NativeId::from("nope"))
            .await
            .expect("answers")
            .is_none()
    );
}

#[tokio::test]
async fn a_native_label_filter_is_applied_and_an_unsupported_one_is_ignored() {
    let query = TaskQuery {
        labels: LabelFilter {
            any_of: vec!["INFRA".to_owned()], // matched case-insensitively, by name
            ..LabelFilter::default()
        },
        ..TaskQuery::default()
    };

    let applied = fully_capable()
        .query_tasks(&query, &whole())
        .await
        .expect("answers");
    assert_eq!(task_ids(&applied), ["T-1", "T-2"]);

    // Rule 2: a source that declares a predicate unsupported returns the WIDER
    // set — never a narrower one — so the engine can narrow it correctly.
    let ignored = poorly_capable()
        .query_tasks(
            &query,
            &PageRequest {
                cursor: None,
                limit: 2,
            },
        )
        .await
        .expect("answers");
    assert_eq!(task_ids(&ignored), ["T-1", "T-2"]);
    assert!(ignored.next.is_some(), "the wider set spills past one page");
}

#[tokio::test]
async fn all_of_and_none_of_narrow_a_label_filter_further() {
    let source = fully_capable();

    let all_of = source
        .query_tasks(
            &TaskQuery {
                labels: LabelFilter {
                    all_of: vec!["infra".to_owned(), "p1".to_owned()],
                    ..LabelFilter::default()
                },
                ..TaskQuery::default()
            },
            &whole(),
        )
        .await
        .expect("answers");
    assert_eq!(task_ids(&all_of), ["T-1"]);

    let none_of = source
        .query_tasks(
            &TaskQuery {
                labels: LabelFilter {
                    any_of: vec!["infra".to_owned()],
                    none_of: vec!["p1".to_owned()],
                    ..LabelFilter::default()
                },
                ..TaskQuery::default()
            },
            &whole(),
        )
        .await
        .expect("answers");
    assert_eq!(task_ids(&none_of), ["T-2"]);
}

#[tokio::test]
async fn a_status_filter_selects_by_normalised_category_not_by_the_sources_wording() {
    let matched = fully_capable()
        .query_tasks(
            &TaskQuery {
                statuses: vec![
                    onetaskgraph_plugin_api::StatusCategory::InProgress,
                    onetaskgraph_plugin_api::StatusCategory::Done,
                ],
                ..TaskQuery::default()
            },
            &whole(),
        )
        .await
        .expect("answers");
    // T-1 is "In Review" and T-4 is "Shipped"; the categories are what matched.
    assert_eq!(task_ids(&matched), ["T-1", "T-4"]);
}

#[tokio::test]
async fn a_search_is_applied_per_half_according_to_what_the_source_declared() {
    let content_only = TaskQuery {
        text: Some(TextQuery {
            terms: "nx affected".to_owned(),
            fields: TextFields::Content,
        }),
        ..TaskQuery::default()
    };
    let found = fully_capable()
        .query_tasks(&content_only, &whole())
        .await
        .expect("answers");
    assert_eq!(task_ids(&found), ["T-2"]);

    let title_only = TaskQuery {
        text: Some(TextQuery {
            terms: "LINEAR".to_owned(),
            fields: TextFields::Title,
        }),
        ..TaskQuery::default()
    };
    let found = fully_capable()
        .query_tasks(&title_only, &whole())
        .await
        .expect("answers");
    assert_eq!(task_ids(&found), ["T-3"]);

    let either = TaskQuery {
        text: Some(TextQuery {
            terms: "contract".to_owned(),
            fields: TextFields::TitleOrContent,
        }),
        ..TaskQuery::default()
    };
    let found = fully_capable()
        .query_tasks(&either, &whole())
        .await
        .expect("answers");
    assert_eq!(task_ids(&found), ["T-1"]);

    // The poor source searches titles but not content, so a content-only search
    // is a predicate it did not declare: it returns the wider set untouched.
    let wider = poorly_capable()
        .query_tasks(&content_only, &whole())
        .await
        .expect("answers");
    assert_eq!(task_ids(&wider), ["T-1", "T-2"], "clamped to max_page_size");
}

#[tokio::test]
async fn a_source_declaring_neither_search_half_ignores_the_predicate_entirely() {
    let blind = onetaskgraph_in_memory::InMemorySource::new(with_capabilities(json!({
        "search_title": "unsupported",
        "search_content": "unsupported",
    })))
    .expect("the fixture graph is coherent");
    let found = blind
        .query_tasks(
            &TaskQuery {
                text: Some(TextQuery {
                    terms: "nothing matches this".to_owned(),
                    fields: TextFields::TitleOrContent,
                }),
                ..TaskQuery::default()
            },
            &whole(),
        )
        .await
        .expect("answers");
    assert_eq!(task_ids(&found), ["T-1", "T-2", "T-3", "T-4"]);
}

#[tokio::test]
async fn an_orphan_task_is_selectable_on_its_own_and_by_its_project_otherwise() {
    let source = fully_capable();

    let orphans = source
        .query_tasks(
            &TaskQuery {
                project: ProjectFilter::Orphans,
                ..TaskQuery::default()
            },
            &whole(),
        )
        .await
        .expect("answers");
    assert_eq!(task_ids(&orphans), ["T-4"]);

    let in_project = source
        .query_tasks(
            &TaskQuery {
                project: ProjectFilter::Is(NativeId::from("P-1")),
                ..TaskQuery::default()
            },
            &whole(),
        )
        .await
        .expect("answers");
    assert_eq!(task_ids(&in_project), ["T-1", "T-2"]);

    // A source without projects cannot honour either, so it returns the wider set.
    let unfiltered = poorly_capable()
        .query_tasks(
            &TaskQuery {
                project: ProjectFilter::Is(NativeId::from("P-1")),
                ..TaskQuery::default()
            },
            &PageRequest {
                cursor: None,
                limit: 100,
            },
        )
        .await
        .expect("answers");
    assert_eq!(
        task_ids(&unfiltered),
        ["T-1", "T-2"],
        "clamped to 2 per page"
    );
}

#[tokio::test]
async fn projects_are_queryable_and_a_source_without_them_returns_none() {
    let projects = fully_capable()
        .query_projects(&ProjectQuery::default(), &whole())
        .await
        .expect("answers");
    assert_eq!(ids(&projects, |p| p.id.to_string()), ["P-1", "P-2"]);

    let filtered = fully_capable()
        .query_projects(
            &ProjectQuery {
                text: Some(TextQuery {
                    terms: "found".to_owned(),
                    fields: TextFields::Title,
                }),
                labels: LabelFilter {
                    any_of: vec!["infra".to_owned()],
                    ..LabelFilter::default()
                },
                statuses: vec![onetaskgraph_plugin_api::StatusCategory::Backlog],
            },
            &whole(),
        )
        .await
        .expect("answers");
    assert_eq!(ids(&filtered, |p| p.id.to_string()), ["P-1"]);

    let none = poorly_capable()
        .query_projects(&ProjectQuery::default(), &whole())
        .await
        .expect("answers");
    assert!(none.items.is_empty());
    assert!(none.next.is_none());
}

#[tokio::test]
async fn labels_are_listed_across_the_source() {
    let labels = fully_capable().labels(&whole()).await.expect("answers");
    assert_eq!(ids(&labels, |l| l.name.clone()), ["infra", "p1", "wontfix"]);
    assert_eq!(labels.items[0].color.as_deref(), Some("#336699"));
}

#[tokio::test]
async fn dependencies_are_answered_forward_by_every_source_and_in_reverse_only_by_a_capable_one() {
    let rich = fully_capable();

    let forward = rich
        .task_dependencies(&NativeId::from("T-2"), Direction::DependsOn, &whole())
        .await
        .expect("answers");
    assert_eq!(forward.items.len(), 1);
    assert_eq!(forward.items[0].to, NativeId::from("T-1"));
    assert_eq!(forward.items[0].kind, DependencyKind::Blocks);

    let reverse = rich
        .task_dependencies(&NativeId::from("T-1"), Direction::DependedOnBy, &whole())
        .await
        .expect("answers");
    assert_eq!(
        ids(&reverse, |e| e.from.to_string()),
        ["T-2", "T-3"],
        "both edges point at T-1"
    );

    let project_forward = rich
        .project_dependencies(&NativeId::from("P-2"), Direction::DependsOn, &whole())
        .await
        .expect("answers");
    assert_eq!(project_forward.items.len(), 1);
    let project_reverse = rich
        .project_dependencies(&NativeId::from("P-1"), Direction::DependedOnBy, &whole())
        .await
        .expect("answers");
    assert_eq!(ids(&project_reverse, |e| e.from.to_string()), ["P-2"]);
}

#[tokio::test]
async fn a_forward_only_source_refuses_the_reverse_direction_rather_than_answering_it_emptily() {
    // Rule 3: a dependency read is never silently empty. The engine emulates the
    // reverse direction for this source, so being asked for it is a bug worth
    // naming — an empty page would look like "nothing depends on T-1".
    let poor = poorly_capable();

    let forward = poor
        .task_dependencies(&NativeId::from("T-2"), Direction::DependsOn, &whole())
        .await
        .expect("forward edges are always real");
    assert_eq!(forward.items.len(), 1);

    for (label, result) in [
        (
            "task dependencies",
            poor.task_dependencies(&NativeId::from("T-1"), Direction::DependedOnBy, &whole())
                .await,
        ),
        (
            "project dependencies",
            poor.project_dependencies(&NativeId::from("P-1"), Direction::DependedOnBy, &whole())
                .await,
        ),
    ] {
        let Err(SourceError::Refused { message }) = result else {
            panic!("a forward-only source must refuse the reverse direction for {label}");
        };
        assert!(message.contains(label), "{message}");
        assert!(message.contains("forward-only"), "{message}");
    }
}

#[tokio::test]
async fn a_limit_smaller_than_the_result_set_walks_to_exhaustion_in_a_stable_order() {
    let source = fully_capable();
    let mut seen = Vec::new();
    let mut cursor = None;

    loop {
        let page = source
            .query_tasks(
                &TaskQuery::default(),
                &PageRequest {
                    cursor: cursor.clone(),
                    limit: 3,
                },
            )
            .await
            .expect("answers");
        assert!(page.items.len() <= 3);
        seen.extend(page.items.iter().map(|t| t.id.to_string()));
        cursor = page.next;
        if cursor.is_none() {
            break;
        }
    }

    assert_eq!(seen, ["T-1", "T-2", "T-3", "T-4"]);
}

#[tokio::test]
async fn a_limit_is_clamped_to_the_declared_page_ceiling() {
    // Asking for more than max_page_size gets max_page_size, never more.
    let page = poorly_capable()
        .query_tasks(&TaskQuery::default(), &whole())
        .await
        .expect("answers");
    assert_eq!(page.items.len(), 2);
    assert_eq!(page.next, Some(Cursor("2".to_owned())));

    // A zero limit is refused, not quietly served as one row: coercing it would turn a
    // caller's bug into a walk that never advances.
    let Err(SourceError::Config { message }) = fully_capable()
        .query_tasks(
            &TaskQuery::default(),
            &PageRequest {
                cursor: None,
                limit: 0,
            },
        )
        .await
    else {
        panic!("a page of no rows is not a page");
    };
    assert!(message.contains("ask for at least 1 row"), "{message}");
}

#[tokio::test]
async fn a_cursor_this_source_did_not_issue_is_rejected_rather_than_silently_restarting() {
    let Err(SourceError::Malformed { message }) = fully_capable()
        .query_tasks(
            &TaskQuery::default(),
            &PageRequest {
                cursor: Some(Cursor("not-a-cursor".to_owned())),
                limit: 10,
            },
        )
        .await
    else {
        panic!("a foreign cursor must be refused, not treated as the beginning");
    };
    assert!(message.contains("not-a-cursor"), "{message}");
}

#[tokio::test]
async fn a_cursor_past_the_end_is_refused_rather_than_answered_as_an_exhausted_walk() {
    // It parses, but this source never issued it. An empty page would be indistinguishable
    // from a walk that legitimately ended, which is how a paging bug goes unnoticed.
    let Err(SourceError::Malformed { message }) = fully_capable()
        .query_tasks(
            &TaskQuery::default(),
            &PageRequest {
                cursor: Some(Cursor("99".to_owned())),
                limit: 10,
            },
        )
        .await
    else {
        panic!("a cursor past the end was never issued by this source");
    };
    assert!(message.contains("past the 4 result(s)"), "{message}");

    // The offset exactly one past the last row is never issued either: `next` is `Some`
    // only while the window stops short of the end, so a walk that reaches the last row
    // ends with `next: None` rather than with a cursor of 4. Accepting 4 would answer a
    // cursor from some other result set as though this walk had legitimately finished.
    let Err(SourceError::Malformed { message }) = fully_capable()
        .query_tasks(
            &TaskQuery::default(),
            &PageRequest {
                cursor: Some(Cursor("4".to_owned())),
                limit: 10,
            },
        )
        .await
    else {
        panic!("an offset equal to the result count is a cursor this source never issued");
    };
    assert!(message.contains("past the 4 result(s)"), "{message}");
}

#[tokio::test]
async fn a_walk_to_the_last_row_ends_with_no_cursor_rather_than_one_past_the_end() {
    // The other half of the rule above: this is the only way a walk over these 4 results
    // terminates, which is what makes an offset of 4 provably foreign.
    let page = fully_capable()
        .query_tasks(
            &TaskQuery::default(),
            &PageRequest {
                cursor: Some(Cursor("2".to_owned())),
                limit: 2,
            },
        )
        .await
        .expect("a cursor this source issued");
    assert_eq!(page.items.len(), 2);
    assert!(
        page.next.is_none(),
        "the final page must end the walk rather than hand out an offset of 4"
    );
}

#[test]
fn the_factory_builds_a_source_from_a_config_block_and_names_its_kind() {
    assert_eq!(Plugin.kind(), "in-memory");
    assert!(Plugin.config_schema().as_value().is_object());

    let name = SourceName::new("notes").expect("a valid name");
    let built = Plugin
        .build(&name, &common::work(), &NoSecrets)
        .expect("the fixture is a valid config block");
    assert_eq!(built.kind(), "in-memory");
    assert_eq!(built.capabilities().max_page_size, 100);
}

#[test]
fn the_factory_refuses_a_config_block_of_the_wrong_shape_and_names_the_source() {
    let name = SourceName::new("notes").expect("a valid name");
    let Err(SourceError::Config { message }) = Plugin.build(
        &name,
        &json!({ "tasks": "not a list", "unknown_field": 1 }),
        &NoSecrets,
    ) else {
        panic!("a malformed config block must be refused");
    };
    assert!(message.starts_with("source notes:"), "{message}");
}

#[test]
fn an_empty_config_block_is_a_valid_empty_source() {
    let config: InMemoryConfig = serde_json::from_value(json!({})).expect("defaults apply");
    let source =
        onetaskgraph_in_memory::InMemorySource::new(config).expect("an empty config is coherent");
    assert_eq!(source.capabilities().max_page_size, 100);
    assert_eq!(
        source.capabilities().project_dependencies,
        DependencySupport::BothDirections
    );
}

#[tokio::test]
async fn a_source_that_applies_only_one_search_half_ignores_a_both_halves_search() {
    // The narrowing failure rule 2 exists to prevent: the poor source searches
    // titles but not bodies, and "nx affected" appears only in T-2's body. Half-
    // applying a `title-or-content` search would return an empty page — narrower
    // than the truth, and the engine has no way to tell that from "no matches".
    let poor = poorly_capable();
    let page = poor
        .query_tasks(
            &TaskQuery {
                text: Some(TextQuery {
                    terms: "nx affected".to_owned(),
                    fields: TextFields::TitleOrContent,
                }),
                ..TaskQuery::default()
            },
            &whole(),
        )
        .await
        .expect("answers");
    assert_eq!(
        task_ids(&page),
        ["T-1", "T-2"],
        "the wider set, clamped to 2"
    );

    // A title-only search is fully within what it declared, so it does apply it.
    let applied = poor
        .query_tasks(
            &TaskQuery {
                text: Some(TextQuery {
                    terms: "linear".to_owned(),
                    fields: TextFields::Title,
                }),
                ..TaskQuery::default()
            },
            &whole(),
        )
        .await
        .expect("answers");
    assert_eq!(task_ids(&applied), ["T-3"]);
}

#[test]
fn a_zero_page_ceiling_is_refused_where_the_configuration_is_read() {
    // A source that will serve no rows cannot be paged. Coercing zero to one would hide
    // a typo behind behaviour that looks deliberate, so it is refused at the boundary.
    // The field is a `NonZeroU32`, so a zero is unrepresentable past this point rather
    // than merely rejected here — and the message still names the setting to correct.
    let name = SourceName::new("notes").expect("a valid name");
    let Err(SourceError::Config { message }) = Plugin.build(
        &name,
        &json!({ "capabilities": { "max_page_size": 0 } }),
        &NoSecrets,
    ) else {
        panic!("a zero page ceiling must be refused");
    };
    assert!(
        message.contains("max_page_size must be at least 1"),
        "{message}"
    );

    // One is the smallest usable ceiling, and it still pages.
    let source = onetaskgraph_in_memory::InMemorySource::new(with_capabilities(
        json!({ "max_page_size": 1 }),
    ))
    .expect("the fixture graph is coherent");
    assert_eq!(source.capabilities().max_page_size, 1);
}

/// The fixture, broken one way, as a `build` refusal message.
///
/// Every case below goes through the real factory rather than calling `validate`
/// directly: a user's incoherent configuration arrives as a `serde_json::Value` at
/// `SourcePlugin::build`, and that is the boundary that has to refuse it.
fn refusal(mutate: impl FnOnce(&mut serde_json::Value)) -> String {
    let mut config = common::work();
    mutate(&mut config);
    let name = SourceName::new("notes").expect("a valid name");
    let Err(SourceError::Config { message }) = Plugin.build(&name, &config, &NoSecrets) else {
        panic!("an incoherent configuration must be refused");
    };
    assert!(message.starts_with("source notes:"), "{message}");
    message
}

#[test]
fn the_constructor_refuses_an_incoherent_graph_so_no_caller_can_bypass_the_factory() {
    // The factory is not the only way in: a caller holding an InMemoryConfig — the SDKs
    // and this crate's own tests do — would otherwise build a source that answers
    // wrongly. The refusal belongs to the constructor, so there is no way to hold an
    // incoherent source at all.
    let mut config = common::work();
    let first = config["tasks"][0].clone();
    config["tasks"].as_array_mut().expect("a list").push(first);
    let config: InMemoryConfig = serde_json::from_value(config).expect("the shape is valid");

    let Err(SourceError::Config { message }) = onetaskgraph_in_memory::InMemorySource::new(config)
    else {
        panic!("an incoherent graph must be refused by the constructor");
    };
    assert!(
        message.contains("two or more tasks share the id T-1"),
        "{message}"
    );
}

#[test]
fn the_factory_refuses_a_duplicate_id_because_it_makes_a_lookup_arbitrary() {
    // Two tasks under one id: `get_task` would return whichever came first, which is
    // not an error anywhere the caller can see it.
    let message = refusal(|config| {
        let first = config["tasks"][0].clone();
        config["tasks"].as_array_mut().expect("a list").push(first);
    });
    assert!(
        message.contains("two or more tasks share the id T-1"),
        "{message}"
    );

    let message = refusal(|config| {
        let first = config["projects"][0].clone();
        config["projects"]
            .as_array_mut()
            .expect("a list")
            .push(first);
    });
    assert!(
        message.contains("two or more projects share the id P-1"),
        "{message}"
    );

    let message = refusal(|config| {
        let first = config["labels"][0].clone();
        config["labels"].as_array_mut().expect("a list").push(first);
    });
    assert!(
        message.contains("two or more labels share the id l-1"),
        "{message}"
    );
}

#[test]
fn the_factory_refuses_a_task_filed_under_a_project_it_does_not_hold() {
    let message = refusal(|config| config["tasks"][0]["project"] = json!("P-404"));
    assert!(
        message.contains("task T-1 is filed under project P-404, which this source does not hold"),
        "{message}"
    );
}

#[test]
fn the_factory_refuses_a_dependency_edge_pointing_at_nothing() {
    // A dangling edge does not fail a dependency walk — it makes one come back short,
    // which reads exactly like an item that genuinely has no dependencies.
    let message = refusal(|config| config["task_dependencies"][0]["to"] = json!("T-404"));
    assert!(
        message.contains("a task dependency edge's `to` names T-404"),
        "{message}"
    );

    let message = refusal(|config| config["project_dependencies"][0]["from"] = json!("P-404"));
    assert!(
        message.contains("a project dependency edge's `from` names P-404"),
        "{message}"
    );
}

#[test]
fn the_factory_reports_every_problem_at_once_rather_than_one_per_round_trip() {
    let message = refusal(|config| {
        config["tasks"][0]["project"] = json!("P-404");
        config["task_dependencies"][0]["to"] = json!("T-404");
    });
    assert!(message.contains("P-404"), "{message}");
    assert!(message.contains("T-404"), "{message}");
}

#[test]
fn the_shared_fixture_is_a_coherent_graph() {
    // The control: every case above breaks this fixture one way, so a fixture that was
    // already incoherent would make all of them pass for the wrong reason.
    let config: InMemoryConfig =
        serde_json::from_value(common::work()).expect("the fixture parses");
    assert_eq!(config.validate(), Ok(()));
}

/// A task on its way in, carrying a caller-defined key of every JSON type.
fn outgoing(id: &str, title: &str) -> Task {
    serde_json::from_value(json!({
        "id": id,
        "title": title,
        "content": "written",
        "status": {"category": "todo", "name": "Todo"},
        "labels": [{"id": "l-9", "name": "written", "color": null}],
        "metadata": {
            "caller.count": 3,
            "caller.text": "3",
            "caller.flag": true,
            "caller.absent": null,
            "caller.shape": {"nested": [1, "two", false]}
        },
        "repositories": ["github.com/nickderobertis/onetaskgraph"]
    }))
    .expect("a task")
}

#[tokio::test]
async fn a_written_task_reads_back_with_every_value_and_json_type_intact() {
    let source = fully_capable();
    assert!(source.writes().is_supported());

    let written = source
        .write_task(&ItemWrite {
            target: None,
            item: outgoing("T-9", "Written"),
            depends_on: vec![DependencyEdge {
                from: DependencyEndpoint::from_native(NativeId("T-9".into()), ItemKind::Task),
                to: DependencyEndpoint::from_native(NativeId("T-1".into()), ItemKind::Task),
                kind: DependencyKind::Blocks,
            }],
        })
        .await
        .expect("this source takes the write");
    assert_eq!(written, NativeId("T-9".into()));

    let read = source
        .get_task(&written)
        .await
        .expect("this source answers")
        .expect("the task is there");
    assert_eq!(read.title, "Written");
    assert_eq!(read.metadata["caller.count"], json!(3));
    assert_eq!(read.metadata["caller.text"], json!("3"));
    assert_eq!(read.metadata["caller.flag"], json!(true));
    assert_eq!(read.metadata["caller.absent"], serde_json::Value::Null);
    assert_eq!(
        read.metadata["caller.shape"],
        json!({"nested": [1, "two", false]})
    );
    assert_eq!(
        read.repositories[0].as_str(),
        "github.com/nickderobertis/onetaskgraph"
    );

    // The edge landed as this source's own forward edge, at the id it was written under.
    let edges = source
        .task_dependencies(&written, Direction::DependsOn, &whole())
        .await
        .expect("this source answers")
        .items;
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].from.id(), "T-9");
    assert_eq!(edges[0].to.id(), "T-1");

    // And a label the write brought with it is one this source now knows.
    let labels = source
        .labels(&whole())
        .await
        .expect("this source answers")
        .items;
    assert!(
        labels.iter().any(|label| label.name == "written"),
        "{labels:?}"
    );

    // A second write at the same target updates that item and adds nothing.
    let updated = source
        .write_task(&ItemWrite {
            target: Some(written.clone()),
            item: outgoing("T-9", "Renamed"),
            depends_on: Vec::new(),
        })
        .await
        .expect("this source takes the write");
    assert_eq!(updated, written);
    assert_eq!(
        source.get_task(&written).await.unwrap().unwrap().title,
        "Renamed"
    );
    // The write said what it depends on now, so the edge it no longer carries is gone.
    assert!(
        source
            .task_dependencies(&written, Direction::DependsOn, &whole())
            .await
            .unwrap()
            .items
            .is_empty()
    );
}

#[tokio::test]
async fn a_written_project_reads_back_and_a_create_never_reuses_a_held_id() {
    let source = fully_capable();
    let written = source
        .write_project(&ItemWrite {
            target: None,
            // `P-1` is already held, so the destination chooses its own id rather than
            // making which item a lookup returns arbitrary.
            item: serde_json::from_value(json!({
                "id": "P-1", "title": "Second foundation",
                "status": {"category": "todo", "name": "Todo"}, "labels": [],
                "metadata": {"caller.key": [1, null]}
            }))
            .expect("a project"),
            depends_on: Vec::new(),
        })
        .await
        .expect("this source takes the write");
    assert_eq!(written, NativeId("P-1-2".into()));
    let read = source.get_project(&written).await.unwrap().unwrap();
    assert_eq!(read.title, "Second foundation");
    assert_eq!(read.metadata["caller.key"], json!([1, null]));
    assert_eq!(
        source
            .get_project(&NativeId("P-1".into()))
            .await
            .unwrap()
            .unwrap()
            .title,
        "Foundation",
        "the project already held is left exactly as it is"
    );

    let Err(SourceError::Refused { message }) = source
        .write_project(&ItemWrite {
            target: Some(NativeId("absent".into())),
            item: read,
            depends_on: Vec::new(),
        })
        .await
    else {
        panic!("a target this source does not hold must be refused rather than created");
    };
    assert!(
        message.contains("names no project this source holds"),
        "{message}"
    );
}

#[tokio::test]
async fn a_source_configured_with_no_write_side_refuses_both_writes() {
    let source: Box<dyn TaskSource> = Box::new(
        onetaskgraph_in_memory::InMemorySource::new(with_capabilities(json!({
            "writes": "unsupported"
        })))
        .expect("the fixture graph is coherent"),
    );
    assert!(!source.writes().is_supported());
    let Err(SourceError::Refused { message }) = source
        .write_task(&ItemWrite {
            target: None,
            item: outgoing("T-9", "Written"),
            depends_on: Vec::new(),
        })
        .await
    else {
        panic!("a source with no write side must refuse");
    };
    assert_eq!(message, "the in-memory plugin cannot be written");
}

#[tokio::test]
async fn a_key_this_source_cannot_carry_refuses_the_write_naming_every_one_of_them() {
    let source: Box<dyn TaskSource> = Box::new(
        onetaskgraph_in_memory::InMemorySource::new(with_capabilities(json!({
            "unwritable_metadata_keys": ["caller.shape", "caller.count", "caller.unused"]
        })))
        .expect("the fixture graph is coherent"),
    );
    let Err(SourceError::Refused { message }) = source
        .write_task(&ItemWrite {
            target: None,
            item: outgoing("T-9", "Written"),
            depends_on: Vec::new(),
        })
        .await
    else {
        panic!("a key this source cannot carry must refuse the write rather than drop it");
    };
    // Every key it could not carry, not the first: someone correcting a document wants
    // the whole list rather than one round trip per key.
    assert!(message.contains("caller.count, caller.shape"), "{message}");
    assert!(!message.contains("caller.unused"), "{message}");
    // And nothing landed.
    assert!(
        source
            .get_task(&NativeId("T-9".into()))
            .await
            .unwrap()
            .is_none()
    );
}

/// `draft` is an ordinary category here: it is configurable, it lists, and it filters.
///
/// Built as its own configuration block rather than by widening the shared graph, because
/// this asserts what the vocabulary admits and every other test asserts on that graph's
/// exact rows. The source parses the block through the real `Plugin::build`, so the
/// assertion covers `StatusCategory`'s own wire spelling as well as the filter.
#[tokio::test]
async fn a_draft_status_is_configurable_listed_and_filtered_like_any_other() {
    let source = Plugin
        .build(
            &SourceName::new("work").unwrap(),
            &json!({
                "tasks": [
                    {
                        "id": "T-1", "title": "Sketch", "content": null,
                        "status": {"category": "draft", "name": "Sketch"},
                        "labels": [], "project": null, "url": null,
                        "created_at": null, "updated_at": null,
                    },
                    {
                        "id": "T-2", "title": "Queued", "content": null,
                        "status": {"category": "todo", "name": "Todo"},
                        "labels": [], "project": null, "url": null,
                        "created_at": null, "updated_at": null,
                    },
                ],
                "capabilities": {"filter_by_status": "native"},
            }),
            &NoSecrets,
        )
        .expect("a draft is a status this configuration may state");

    let all = source
        .query_tasks(&TaskQuery::default(), &whole())
        .await
        .expect("answers");
    assert_eq!(task_ids(&all), ["T-1", "T-2"]);
    assert_eq!(
        all.items[0].status.category,
        onetaskgraph_plugin_api::StatusCategory::Draft
    );
    // The source's own wording survives normalisation, as it does for every category.
    assert_eq!(all.items[0].status.name, "Sketch");

    let drafts = source
        .query_tasks(
            &TaskQuery {
                statuses: vec![onetaskgraph_plugin_api::StatusCategory::Draft],
                ..TaskQuery::default()
            },
            &whole(),
        )
        .await
        .expect("answers");
    assert_eq!(task_ids(&drafts), ["T-1"]);

    // And `todo` did not move: it still selects only what it selected before.
    let todo = source
        .query_tasks(
            &TaskQuery {
                statuses: vec![onetaskgraph_plugin_api::StatusCategory::Todo],
                ..TaskQuery::default()
            },
            &whole(),
        )
        .await
        .expect("answers");
    assert_eq!(task_ids(&todo), ["T-2"]);
}

/// Every field of the contract's `Capabilities`, under one configuration, driven against
/// the store the source really answers from.
///
/// The fixture is what makes an honoured predicate and an ignored one *different
/// answers*: two projects with tasks under each, a task under neither, a label two tasks
/// carry and two do not, four statuses, and edges in both dependency tables. A source
/// over a single project, or over work where every task carries the label, answers a
/// filter the same way whether or not it applies it.
#[tokio::test]
async fn every_declared_capability_is_applied_to_the_held_work() {
    let source = fully_capable();
    // The struct has no `Default`, so a field added to the contract fails to compile here
    // rather than going unasserted.
    assert_eq!(
        source.capabilities(),
        onetaskgraph_plugin_api::Capabilities {
            projects: Support::Native,
            orphan_tasks: Support::Native,
            filter_by_label: Support::Native,
            filter_by_status: Support::Native,
            search_title: Support::Native,
            search_content: Support::Native,
            task_dependencies: DependencySupport::BothDirections,
            project_dependencies: DependencySupport::BothDirections,
            max_page_size: 100,
        }
    );

    let sorted = |mut ids: Vec<String>| {
        ids.sort();
        ids
    };
    let selected = async |query: TaskQuery| -> Vec<String> {
        sorted(task_ids(
            &source.query_tasks(&query, &whole()).await.unwrap(),
        ))
    };

    // `projects`: two are held, and a listing scoped to one keeps the tasks filed under
    // it and no other.
    assert_eq!(
        sorted(ids(
            &source
                .query_projects(&ProjectQuery::default(), &whole())
                .await
                .unwrap(),
            |project| project.id.to_string()
        )),
        ["P-1", "P-2"]
    );
    let under = |id: &str| TaskQuery {
        project: ProjectFilter::Is(NativeId(id.into())),
        ..TaskQuery::default()
    };
    assert_eq!(selected(under("P-1")).await, ["T-1", "T-2"]);
    assert_eq!(selected(under("P-2")).await, ["T-3"]);

    // `orphan_tasks`: the one task belonging to no project.
    assert_eq!(
        selected(TaskQuery {
            project: ProjectFilter::Orphans,
            ..TaskQuery::default()
        })
        .await,
        ["T-4"]
    );

    // `filter_by_label`: two tasks carry `infra` and two do not, and `all_of` narrows to
    // the one carrying both.
    let labelled = |filter: LabelFilter| TaskQuery {
        labels: filter,
        ..TaskQuery::default()
    };
    assert_eq!(
        selected(labelled(LabelFilter {
            any_of: vec!["infra".into()],
            ..LabelFilter::default()
        }))
        .await,
        ["T-1", "T-2"]
    );
    assert_eq!(
        selected(labelled(LabelFilter {
            none_of: vec!["infra".into()],
            ..LabelFilter::default()
        }))
        .await,
        ["T-3", "T-4"]
    );
    assert_eq!(
        selected(labelled(LabelFilter {
            all_of: vec!["infra".into(), "p1".into()],
            ..LabelFilter::default()
        }))
        .await,
        ["T-1"]
    );

    // `filter_by_status`: the four tasks sit in four categories, so each selects one.
    let filed_under = |category| TaskQuery {
        statuses: vec![category],
        ..TaskQuery::default()
    };
    assert_eq!(
        selected(filed_under(
            onetaskgraph_plugin_api::StatusCategory::InProgress
        ))
        .await,
        ["T-1"]
    );
    assert_eq!(
        selected(filed_under(onetaskgraph_plugin_api::StatusCategory::Todo)).await,
        ["T-2"]
    );
    assert_eq!(
        selected(filed_under(onetaskgraph_plugin_api::StatusCategory::Done)).await,
        ["T-4"]
    );

    // `search_title` and `search_content`: "loose" is in one title and no body, "belongs"
    // is in one body and no title, so each half finds its own and neither the other's.
    let searching = |terms: &str, fields| TaskQuery {
        text: Some(TextQuery {
            terms: terms.into(),
            fields,
        }),
        ..TaskQuery::default()
    };
    assert_eq!(
        selected(searching("loose", TextFields::Title)).await,
        ["T-4"]
    );
    assert!(
        selected(searching("loose", TextFields::Content))
            .await
            .is_empty()
    );
    assert_eq!(
        selected(searching("belongs", TextFields::Content)).await,
        ["T-4"]
    );
    assert!(
        selected(searching("belongs", TextFields::Title))
            .await
            .is_empty()
    );
    assert_eq!(
        selected(searching("belongs", TextFields::TitleOrContent)).await,
        ["T-4"]
    );

    // `task_dependencies` and `project_dependencies`, both directions each: one
    // relationship reads the same from either end, `from` being the item that waits.
    let task_edge = DependencyEdge {
        from: DependencyEndpoint::from_native(NativeId("T-2".into()), ItemKind::Task),
        to: DependencyEndpoint::from_native(NativeId("T-1".into()), ItemKind::Task),
        kind: DependencyKind::Blocks,
    };
    assert_eq!(
        source
            .task_dependencies(&NativeId("T-2".into()), Direction::DependsOn, &whole())
            .await
            .unwrap()
            .items,
        std::slice::from_ref(&task_edge)
    );
    assert!(
        source
            .task_dependencies(&NativeId("T-1".into()), Direction::DependedOnBy, &whole())
            .await
            .unwrap()
            .items
            .contains(&task_edge)
    );
    let project_edge = DependencyEdge {
        from: DependencyEndpoint::from_native(NativeId("P-2".into()), ItemKind::Project),
        to: DependencyEndpoint::from_native(NativeId("P-1".into()), ItemKind::Project),
        kind: DependencyKind::Blocks,
    };
    assert_eq!(
        source
            .project_dependencies(&NativeId("P-2".into()), Direction::DependsOn, &whole())
            .await
            .unwrap()
            .items,
        std::slice::from_ref(&project_edge)
    );
    assert_eq!(
        source
            .project_dependencies(&NativeId("P-1".into()), Direction::DependedOnBy, &whole())
            .await
            .unwrap()
            .items,
        std::slice::from_ref(&project_edge)
    );

    // `max_page_size`: a limit above the declared ceiling is clamped rather than refused,
    // and a limit below the result set walks to exhaustion in the order one whole page
    // reports.
    let whole_page = task_ids(
        &source
            .query_tasks(&TaskQuery::default(), &whole())
            .await
            .unwrap(),
    );
    let clamped = source
        .query_tasks(
            &TaskQuery::default(),
            &PageRequest {
                cursor: None,
                limit: source.capabilities().max_page_size + 1,
            },
        )
        .await
        .unwrap();
    assert_eq!(task_ids(&clamped), whole_page);
    let mut walked = Vec::new();
    let mut cursor = None;
    loop {
        let step = source
            .query_tasks(&TaskQuery::default(), &PageRequest { cursor, limit: 1 })
            .await
            .unwrap();
        assert!(step.items.len() <= 1);
        walked.extend(task_ids(&step));
        cursor = step.next;
        if cursor.is_none() {
            break;
        }
        assert!(walked.len() <= 10, "the paged walk must terminate");
    }
    assert_eq!(walked, whole_page);
}
