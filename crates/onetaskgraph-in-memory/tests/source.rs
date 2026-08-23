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
    Cursor, DependencyKind, DependencySupport, Direction, LabelFilter, NativeId, Page, PageRequest,
    ProjectFilter, ProjectQuery, SecretResolver, SourceError, SourceName, SourcePlugin, Support,
    Task, TaskQuery, TaskSource, TextFields, TextQuery,
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
    Box::new(onetaskgraph_in_memory::InMemorySource::new(
        with_capabilities(json!({})),
    ))
}

/// The same work, but the source ignores labels and content search, has no
/// projects, and answers dependencies forward only.
fn poorly_capable() -> Box<dyn TaskSource> {
    Box::new(onetaskgraph_in_memory::InMemorySource::new(
        with_capabilities(json!({
            "projects": "unsupported",
            "orphan_tasks": "unsupported",
            "filter_by_label": "unsupported",
            "filter_by_status": "native",
            "search_title": "native",
            "search_content": "unsupported",
            "task_dependencies": "forward-only",
            "project_dependencies": "forward-only",
            "max_page_size": 2,
        })),
    ))
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
    assert!(
        health
            .detail
            .as_deref()
            .is_some_and(|d| d.contains("4 task")),
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
    })));
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
    let source = onetaskgraph_in_memory::InMemorySource::new(config);
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
    ));
    assert_eq!(source.capabilities().max_page_size, 1);
}
