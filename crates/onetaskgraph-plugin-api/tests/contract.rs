//! The contract, exercised the way a plugin author meets it.
//!
//! These drive the public surface — the traits through `dyn`, the newtypes through
//! their real constructors, the work types through serde — rather than asserting on
//! internals, because that surface is what six other crates are written against.

use chrono::{TimeZone as _, Utc};
use onetaskgraph_plugin_api::{
    Capabilities, Cursor, DependencyEdge, DependencyKind, DependencySupport, Direction, Health,
    Label, LabelFilter, NativeId, Page, PageRequest, Project, ProjectFilter, ProjectQuery,
    SOURCE_NAME_PATTERN, SecretResolver, SourceError, SourceName, SourcePlugin, Status,
    StatusCategory, Support, Task, TaskQuery, TaskSource, TextFields, TextQuery,
};
use schemars::{Schema, schema_for};
use secrecy::{ExposeSecret as _, SecretString};

/// A source that answers everything emptily. Enough to prove the trait is usable
/// through `dyn`, which is the property the engine's `Vec<Box<dyn TaskSource>>`
/// depends on.
struct Silent(&'static str);

#[async_trait::async_trait]
impl TaskSource for Silent {
    fn kind(&self) -> &'static str {
        self.0
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            projects: Support::Native,
            orphan_tasks: Support::Native,
            filter_by_label: Support::Unsupported,
            filter_by_status: Support::Native,
            search_title: Support::Native,
            search_content: Support::Unsupported,
            task_dependencies: DependencySupport::ForwardOnly,
            project_dependencies: DependencySupport::BothDirections,
            max_page_size: 25,
        }
    }
    async fn health(&self) -> Result<Health, SourceError> {
        Ok(Health {
            reachable: true,
            detail: None,
        })
    }
    async fn get_task(&self, _id: &NativeId) -> Result<Option<Task>, SourceError> {
        Ok(None)
    }
    async fn get_project(&self, _id: &NativeId) -> Result<Option<Project>, SourceError> {
        Ok(None)
    }
    async fn query_tasks(
        &self,
        _query: &TaskQuery,
        _page: &PageRequest,
    ) -> Result<Page<Task>, SourceError> {
        Ok(Page::last(Vec::new()))
    }
    async fn query_projects(
        &self,
        _query: &ProjectQuery,
        _page: &PageRequest,
    ) -> Result<Page<Project>, SourceError> {
        Ok(Page::last(Vec::new()))
    }
    async fn labels(&self, _page: &PageRequest) -> Result<Page<Label>, SourceError> {
        Ok(Page::last(Vec::new()))
    }
    async fn task_dependencies(
        &self,
        _id: &NativeId,
        _direction: Direction,
        _page: &PageRequest,
    ) -> Result<Page<DependencyEdge>, SourceError> {
        Ok(Page::last(Vec::new()))
    }
    async fn project_dependencies(
        &self,
        _id: &NativeId,
        _direction: Direction,
        _page: &PageRequest,
    ) -> Result<Page<DependencyEdge>, SourceError> {
        Ok(Page::last(Vec::new()))
    }
}

/// A second, unrelated implementation, so the collection below is genuinely
/// heterogeneous rather than one type twice.
struct Refusing;

#[async_trait::async_trait]
impl TaskSource for Refusing {
    fn kind(&self) -> &'static str {
        "refusing"
    }
    fn capabilities(&self) -> Capabilities {
        Silent("x").capabilities()
    }
    async fn health(&self) -> Result<Health, SourceError> {
        Err(SourceError::Unavailable {
            message: "no route to host".to_owned(),
        })
    }
    async fn get_task(&self, _id: &NativeId) -> Result<Option<Task>, SourceError> {
        Err(SourceError::RateLimited {
            retry_after_seconds: Some(30),
        })
    }
    async fn get_project(&self, _id: &NativeId) -> Result<Option<Project>, SourceError> {
        Ok(None)
    }
    async fn query_tasks(
        &self,
        _query: &TaskQuery,
        _page: &PageRequest,
    ) -> Result<Page<Task>, SourceError> {
        Ok(Page::last(Vec::new()))
    }
    async fn query_projects(
        &self,
        _query: &ProjectQuery,
        _page: &PageRequest,
    ) -> Result<Page<Project>, SourceError> {
        Ok(Page::last(Vec::new()))
    }
    async fn labels(&self, _page: &PageRequest) -> Result<Page<Label>, SourceError> {
        Ok(Page::last(Vec::new()))
    }
    async fn task_dependencies(
        &self,
        _id: &NativeId,
        _direction: Direction,
        _page: &PageRequest,
    ) -> Result<Page<DependencyEdge>, SourceError> {
        Ok(Page::last(Vec::new()))
    }
    async fn project_dependencies(
        &self,
        _id: &NativeId,
        _direction: Direction,
        _page: &PageRequest,
    ) -> Result<Page<DependencyEdge>, SourceError> {
        Ok(Page::last(Vec::new()))
    }
}

#[tokio::test]
async fn the_engine_can_hold_a_heterogeneous_collection_of_boxed_sources() {
    // This is what dyn-compatibility buys, and the only reason it is a
    // requirement: the engine fans one query out over sources of different types.
    let sources: Vec<Box<dyn TaskSource>> = vec![Box::new(Silent("silent")), Box::new(Refusing)];

    let kinds: Vec<&str> = sources.iter().map(|source| source.kind()).collect();
    assert_eq!(kinds, ["silent", "refusing"]);

    let health: Vec<bool> = {
        let mut out = Vec::new();
        for source in &sources {
            out.push(source.health().await.is_ok());
        }
        out
    };
    assert_eq!(health, [true, false]);

    let page = PageRequest {
        cursor: None,
        limit: 10,
    };
    let first = sources[0]
        .query_tasks(&TaskQuery::default(), &page)
        .await
        .expect("the silent source answers");
    assert!(first.items.is_empty());
    assert!(first.next.is_none());
}

#[tokio::test]
async fn a_source_reports_its_own_capability_declaration() {
    let source: Box<dyn TaskSource> = Box::new(Silent("silent"));
    let declared = source.capabilities();

    assert!(declared.filter_by_status.is_native());
    assert!(!declared.filter_by_label.is_native());
    assert!(declared.project_dependencies.answers_reverse());
    assert!(!declared.task_dependencies.answers_reverse());
    assert_eq!(declared.max_page_size, 25);
}

#[tokio::test]
async fn every_remaining_trait_method_is_reachable_through_dyn() {
    let source: Box<dyn TaskSource> = Box::new(Silent("silent"));
    let id = NativeId::from("t-1");
    let page = PageRequest {
        cursor: Some(Cursor("0".to_owned())),
        limit: 5,
    };

    assert!(source.get_task(&id).await.expect("answers").is_none());
    assert!(source.get_project(&id).await.expect("answers").is_none());
    assert!(
        source
            .query_projects(&ProjectQuery::default(), &page)
            .await
            .expect("answers")
            .items
            .is_empty()
    );
    assert!(
        source
            .labels(&page)
            .await
            .expect("answers")
            .items
            .is_empty()
    );
    assert!(
        source
            .task_dependencies(&id, Direction::DependsOn, &page)
            .await
            .expect("answers")
            .items
            .is_empty()
    );
    assert!(
        source
            .project_dependencies(&id, Direction::DependedOnBy, &page)
            .await
            .expect("answers")
            .items
            .is_empty()
    );

    let refusing: Box<dyn TaskSource> = Box::new(Refusing);
    let error = refusing.get_task(&id).await.expect_err("rate-limited");
    assert_eq!(
        error,
        SourceError::RateLimited {
            retry_after_seconds: Some(30)
        }
    );
    assert!(
        refusing
            .query_tasks(&TaskQuery::default(), &page)
            .await
            .expect("answers")
            .items
            .is_empty()
    );
    assert!(refusing.get_project(&id).await.expect("answers").is_none());
    assert!(
        refusing
            .query_projects(&ProjectQuery::default(), &page)
            .await
            .expect("answers")
            .items
            .is_empty()
    );
    assert!(
        refusing
            .labels(&page)
            .await
            .expect("answers")
            .items
            .is_empty()
    );
    assert!(
        refusing
            .task_dependencies(&id, Direction::DependsOn, &page)
            .await
            .expect("answers")
            .items
            .is_empty()
    );
    assert!(
        refusing
            .project_dependencies(&id, Direction::DependsOn, &page)
            .await
            .expect("answers")
            .items
            .is_empty()
    );
    assert_eq!(refusing.capabilities().max_page_size, 25);
}

/// A resolver over a fixed table, standing in for the process environment.
struct Table(Vec<(&'static str, &'static str)>);

impl SecretResolver for Table {
    fn get(&self, var: &str) -> Option<SecretString> {
        self.0
            .iter()
            .find(|(name, _)| *name == var)
            .map(|(_, value)| SecretString::from((*value).to_owned()))
    }
}

/// A plugin that builds a source only once its named credential resolves — the
/// shape every real plugin's `build` takes.
struct Gated;

impl SourcePlugin for Gated {
    fn kind(&self) -> &'static str {
        "gated"
    }
    fn config_schema(&self) -> Schema {
        schema_for!(TaskQuery)
    }
    fn build(
        &self,
        name: &SourceName,
        config: &serde_json::Value,
        secrets: &dyn SecretResolver,
    ) -> Result<Box<dyn TaskSource>, SourceError> {
        let var = config["api_key_env"]
            .as_str()
            .ok_or_else(|| SourceError::Config {
                message: format!("source {name}: config.api_key_env must be a string"),
            })?;
        let key = secrets.get(var).ok_or_else(|| SourceError::Auth {
            message: format!("source {name}: nothing defines {var}"),
        })?;
        assert!(!key.expose_secret().is_empty());
        Ok(Box::new(Silent("gated")))
    }
}

#[test]
fn a_plugin_builds_a_source_from_a_config_block_and_a_named_credential() {
    let name = SourceName::new("work").expect("a valid name");
    let secrets = Table(vec![("LINEAR_API_KEY", "lin_api_live")]);

    let built = Gated
        .build(
            &name,
            &serde_json::json!({ "api_key_env": "LINEAR_API_KEY" }),
            &secrets,
        )
        .expect("the credential resolves");
    assert_eq!(built.kind(), "gated");
    assert_eq!(Gated.kind(), "gated");
    assert!(Gated.config_schema().as_value().is_object());
}

#[test]
fn a_plugin_refuses_when_the_named_credential_is_absent() {
    let name = SourceName::new("work").expect("a valid name");
    let secrets = Table(Vec::new());

    let Err(error) = Gated.build(
        &name,
        &serde_json::json!({ "api_key_env": "LINEAR_API_KEY" }),
        &secrets,
    ) else {
        panic!("nothing defines the variable, so build must refuse");
    };
    assert_eq!(
        error,
        SourceError::Auth {
            message: "source work: nothing defines LINEAR_API_KEY".to_owned()
        }
    );
    assert!(secrets.get("ABSENT").is_none());
}

#[test]
fn a_plugin_refuses_a_config_block_of_the_wrong_shape() {
    let name = SourceName::new("work").expect("a valid name");
    let Err(error) = Gated.build(&name, &serde_json::json!({}), &Table(Vec::new())) else {
        panic!("the config block is the wrong shape, so build must refuse");
    };
    assert!(
        matches!(&error, SourceError::Config { message } if message.contains("api_key_env")),
        "{error:?}"
    );
}

#[test]
fn a_source_name_accepts_the_documented_pattern_and_rejects_everything_else() {
    for good in ["work", "notes", "gh-main", "s3", "0"] {
        let name = SourceName::new(good).expect("a valid name");
        assert_eq!(name.as_str(), good);
        assert_eq!(name.to_string(), good);
        assert_eq!(String::from(name.clone()), good);
        assert_eq!(SourceName::try_from(good.to_owned()).expect("valid"), name);
    }

    // Underscores are the load-bearing exclusion: `ONETASKGRAPH_SOURCES__<NAME>__…`
    // joins segments with a double underscore, so a name holding one is ambiguous.
    for bad in ["gh_main", "Work", "-lead", "", "notes!", "a b"] {
        let Err(error) = SourceName::new(bad) else {
            panic!("{bad:?} is not a usable source name");
        };
        let SourceError::Config { message } = error else {
            panic!("a bad name is a configuration error");
        };
        assert!(message.contains(SOURCE_NAME_PATTERN), "{message}");
    }
}

#[test]
fn a_source_name_round_trips_through_json_and_rejects_a_bad_one_at_the_boundary() {
    let name = SourceName::new("gh-main").expect("valid");
    let encoded = serde_json::to_string(&name).expect("encodes");
    assert_eq!(encoded, "\"gh-main\"");
    assert_eq!(
        serde_json::from_str::<SourceName>(&encoded).expect("decodes"),
        name
    );
    assert!(serde_json::from_str::<SourceName>("\"gh_main\"").is_err());

    let schema = serde_json::to_value(schema_for!(SourceName)).expect("renders");
    assert_eq!(schema["pattern"], SOURCE_NAME_PATTERN);
}

#[test]
fn a_native_id_carries_whatever_the_source_says_including_colons() {
    let id = NativeId::from("urn:task:7");
    assert_eq!(id.as_str(), "urn:task:7");
    assert_eq!(id.to_string(), "urn:task:7");
    assert_eq!(NativeId::from("urn:task:7".to_owned()), id);
    assert_eq!(
        serde_json::to_string(&id).expect("encodes"),
        "\"urn:task:7\""
    );
}

#[test]
fn a_task_round_trips_through_json_with_every_field_populated() {
    let task = Task {
        id: NativeId::from("ENG-1"),
        title: "Ship the contract".to_owned(),
        content: Some("Two crates, one direction.".to_owned()),
        status: Status {
            category: StatusCategory::InProgress,
            name: "In Review".to_owned(),
        },
        labels: vec![Label {
            id: NativeId::from("l-1"),
            name: "infra".to_owned(),
            color: Some("#336699".to_owned()),
        }],
        project: Some(NativeId::from("P-1")),
        url: Some("https://example.invalid/ENG-1".to_owned()),
        created_at: Some(Utc.with_ymd_and_hms(2026, 8, 22, 9, 0, 0).unwrap()),
        updated_at: None,
    };

    let encoded = serde_json::to_string(&task).expect("encodes");
    assert_eq!(
        serde_json::from_str::<Task>(&encoded).expect("decodes"),
        task
    );
    // The source's own wording survives normalisation.
    assert_eq!(task.status.name, "In Review");
    assert_eq!(task.status.category, StatusCategory::InProgress);
}

#[test]
fn a_project_and_an_orphan_task_round_trip_through_json() {
    let project = Project {
        id: NativeId::from("P-1"),
        title: "Foundation".to_owned(),
        content: None,
        status: Status {
            category: StatusCategory::Backlog,
            name: "Planned".to_owned(),
        },
        labels: Vec::new(),
        url: None,
        created_at: None,
        updated_at: Some(Utc.with_ymd_and_hms(2026, 8, 22, 9, 0, 0).unwrap()),
    };
    let encoded = serde_json::to_string(&project).expect("encodes");
    assert_eq!(
        serde_json::from_str::<Project>(&encoded).expect("decodes"),
        project
    );

    // A task belonging to no project is a first-class case, not an edge case.
    let orphan: Task = serde_json::from_value(serde_json::json!({
        "id": "T-9",
        "title": "Loose end",
        "content": null,
        "status": { "category": "todo", "name": "Todo" },
        "labels": [],
        "project": null,
        "url": null,
        "created_at": null,
        "updated_at": null,
    }))
    .expect("decodes");
    assert!(orphan.project.is_none());
    assert_eq!(orphan.status.category, StatusCategory::Todo);
}

#[test]
fn the_normalised_vocabularies_serialise_as_kebab_case() {
    let categories = [
        (StatusCategory::Backlog, "backlog"),
        (StatusCategory::Todo, "todo"),
        (StatusCategory::InProgress, "in-progress"),
        (StatusCategory::Done, "done"),
        (StatusCategory::Cancelled, "cancelled"),
        (StatusCategory::Unknown, "unknown"),
    ];
    for (value, wire) in categories {
        assert_eq!(
            serde_json::to_value(value).expect("encodes"),
            serde_json::json!(wire)
        );
        assert_eq!(
            serde_json::from_value::<StatusCategory>(serde_json::json!(wire)).expect("decodes"),
            value
        );
    }

    assert_eq!(
        serde_json::to_value(DependencyKind::Blocks).expect("encodes"),
        serde_json::json!("blocks")
    );
    assert_eq!(
        serde_json::to_value(DependencyKind::Related).expect("encodes"),
        serde_json::json!("related")
    );
    assert_eq!(
        serde_json::to_value(Direction::DependedOnBy).expect("encodes"),
        serde_json::json!("depended-on-by")
    );
    assert_eq!(
        serde_json::to_value(Direction::DependsOn).expect("encodes"),
        serde_json::json!("depends-on")
    );
    assert_eq!(
        serde_json::to_value(TextFields::TitleOrContent).expect("encodes"),
        serde_json::json!("title-or-content")
    );
    assert_eq!(
        serde_json::to_value(TextFields::Title).expect("encodes"),
        serde_json::json!("title")
    );
    assert_eq!(
        serde_json::to_value(TextFields::Content).expect("encodes"),
        serde_json::json!("content")
    );
    assert_eq!(
        serde_json::to_value(Support::Unsupported).expect("encodes"),
        serde_json::json!("unsupported")
    );
    assert_eq!(
        serde_json::to_value(DependencySupport::ForwardOnly).expect("encodes"),
        serde_json::json!("forward-only")
    );
}

#[test]
fn a_dependency_edge_round_trips_through_json() {
    let edge = DependencyEdge {
        from: NativeId::from("A"),
        to: NativeId::from("B"),
        kind: DependencyKind::Blocks,
    };
    let encoded = serde_json::to_string(&edge).expect("encodes");
    assert_eq!(
        serde_json::from_str::<DependencyEdge>(&encoded).expect("decodes"),
        edge
    );
}

#[test]
fn an_empty_label_filter_constrains_nothing_and_a_populated_one_does() {
    assert!(LabelFilter::default().is_empty());
    assert!(
        !LabelFilter {
            none_of: vec!["wontfix".to_owned()],
            ..LabelFilter::default()
        }
        .is_empty()
    );
    assert!(
        !LabelFilter {
            any_of: vec!["infra".to_owned()],
            ..LabelFilter::default()
        }
        .is_empty()
    );
    assert!(
        !LabelFilter {
            all_of: vec!["infra".to_owned()],
            ..LabelFilter::default()
        }
        .is_empty()
    );
}

#[test]
fn a_query_round_trips_with_every_filter_populated() {
    let query = TaskQuery {
        text: Some(TextQuery {
            terms: "contract".to_owned(),
            fields: TextFields::TitleOrContent,
        }),
        labels: LabelFilter {
            any_of: vec!["infra".to_owned()],
            all_of: vec!["p1".to_owned()],
            none_of: vec!["wontfix".to_owned()],
        },
        statuses: vec![StatusCategory::Todo, StatusCategory::InProgress],
        project: ProjectFilter::Is(NativeId::from("P-1")),
    };
    let encoded = serde_json::to_string(&query).expect("encodes");
    assert_eq!(
        serde_json::from_str::<TaskQuery>(&encoded).expect("decodes"),
        query
    );

    let projects = ProjectQuery {
        text: None,
        labels: LabelFilter::default(),
        statuses: vec![StatusCategory::Done],
    };
    let encoded = serde_json::to_string(&projects).expect("encodes");
    assert_eq!(
        serde_json::from_str::<ProjectQuery>(&encoded).expect("decodes"),
        projects
    );

    assert_eq!(ProjectFilter::default(), ProjectFilter::Any);
    for filter in [
        ProjectFilter::Any,
        ProjectFilter::Orphans,
        ProjectFilter::Is(NativeId::from("P-2")),
    ] {
        let encoded = serde_json::to_string(&filter).expect("encodes");
        assert_eq!(
            serde_json::from_str::<ProjectFilter>(&encoded).expect("decodes"),
            filter
        );
    }
}

#[test]
fn a_page_carries_a_cursor_only_while_the_walk_continues() {
    let exhausted = Page::last(vec![1_u8, 2, 3]);
    assert!(exhausted.next.is_none());

    let more = Page {
        items: vec![1_u8],
        next: Some(Cursor("3".to_owned())),
    };
    let encoded = serde_json::to_string(&more).expect("encodes");
    assert_eq!(
        serde_json::from_str::<Page<u8>>(&encoded).expect("decodes"),
        more
    );

    let request = PageRequest {
        cursor: Some(Cursor("3".to_owned())),
        limit: 50,
    };
    let encoded = serde_json::to_string(&request).expect("encodes");
    assert_eq!(
        serde_json::from_str::<PageRequest>(&encoded).expect("decodes"),
        request
    );
}

#[test]
fn a_page_request_for_no_rows_is_refused_where_the_request_is_read() {
    // A page of no rows is not a page. Coercing zero to one would turn a caller's bug
    // into a walk that never advances, so it is refused at the boundary instead.
    let error = serde_json::from_str::<PageRequest>(r#"{"cursor":null,"limit":0}"#)
        .expect_err("a zero limit is not a page size");
    assert!(
        error.to_string().contains("limit must be at least 1"),
        "{error}"
    );

    let smallest: PageRequest =
        serde_json::from_str(r#"{"cursor":null,"limit":1}"#).expect("one row is a page");
    assert_eq!(smallest.limit, 1);
}

#[test]
fn health_round_trips_in_both_shapes() {
    for health in [
        Health {
            reachable: true,
            detail: Some("200 OK".to_owned()),
        },
        Health {
            reachable: false,
            detail: None,
        },
    ] {
        let encoded = serde_json::to_string(&health).expect("encodes");
        assert_eq!(
            serde_json::from_str::<Health>(&encoded).expect("decodes"),
            health
        );
    }
}

#[test]
fn every_error_variant_renders_a_message_and_survives_the_stdio_boundary() {
    // Owned data only, so an error crosses JSON-over-stdio to a subprocess-hosted
    // plugin and back without losing anything.
    let cases = [
        (
            SourceError::Config {
                message: "team is required".to_owned(),
            },
            "configuration for this source is invalid: team is required",
            "config",
        ),
        (
            SourceError::Auth {
                message: "token rejected".to_owned(),
            },
            "authentication for this source failed: token rejected",
            "auth",
        ),
        (
            SourceError::Refused {
                message: "forbidden".to_owned(),
            },
            "the source refused the request: forbidden",
            "refused",
        ),
        (
            SourceError::RateLimited {
                retry_after_seconds: Some(30),
            },
            "the source rate-limited the request",
            "rate-limited",
        ),
        (
            SourceError::Unavailable {
                message: "no route".to_owned(),
            },
            "the source could not be reached: no route",
            "unavailable",
        ),
        (
            SourceError::Malformed {
                message: "not a date".to_owned(),
            },
            "the source returned data this interface cannot represent: not a date",
            "malformed",
        ),
    ];

    for (error, rendered, tag) in cases {
        assert_eq!(error.to_string(), rendered);
        let encoded = serde_json::to_value(&error).expect("encodes");
        assert_eq!(encoded["kind"], tag);
        assert_eq!(
            serde_json::from_value::<SourceError>(encoded).expect("decodes"),
            error
        );
    }
}

#[test]
fn every_contract_root_generates_a_json_schema() {
    // Both SDKs are generated from these, so a type that cannot describe itself
    // is a broken contract even when it compiles.
    for schema in [
        schema_for!(Task),
        schema_for!(Project),
        schema_for!(Label),
        schema_for!(Capabilities),
        schema_for!(TaskQuery),
        schema_for!(ProjectQuery),
        schema_for!(Page<Task>),
        schema_for!(PageRequest),
        schema_for!(Health),
        schema_for!(SourceError),
        schema_for!(DependencyEdge),
    ] {
        assert!(schema.as_value().is_object());
    }
}

/// Expand a regex character-class body such as `a-z0-9-` into the characters it
/// denotes. A trailing `-` is a literal, which is exactly why the pattern spells
/// the hyphen last.
fn expand_class(body: &str) -> Vec<char> {
    let chars: Vec<char> = body.chars().collect();
    let mut out = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        // `x-y` is a range only when a `y` follows it.
        if index + 2 < chars.len() && chars[index + 1] == '-' {
            out.extend(chars[index]..=chars[index + 2]);
            index += 3;
        } else {
            out.push(chars[index]);
            index += 1;
        }
    }
    out
}

/// Match `value` against the two character classes of `^[first][rest]*$`.
///
/// Derived from the published constant rather than restating it, which is the
/// whole point: this cannot agree with a pattern it did not read.
fn matches_published_pattern(value: &str) -> bool {
    let body = SOURCE_NAME_PATTERN
        .strip_prefix('^')
        .and_then(|rest| rest.strip_suffix('$'))
        .and_then(|rest| rest.strip_suffix('*'))
        .expect("the pattern is anchored and its tail repeats");
    let (first, rest) = body.split_once("][").expect("the pattern has two classes");
    let first = expand_class(first.strip_prefix('[').expect("a class opens the pattern"));
    let rest = expand_class(rest.strip_suffix(']').expect("a class closes the pattern"));

    let mut chars = value.chars();
    let Some(head) = chars.next() else {
        return false;
    };
    first.contains(&head) && chars.all(|c| rest.contains(&c))
}

#[test]
fn source_name_validation_agrees_with_the_pattern_it_publishes() {
    // `SourceName::new` hand-rolls its check for speed while `SOURCE_NAME_PATTERN`
    // is what the JSON Schema publishes to both SDKs, so nothing but this stops the
    // two describing different languages — a name the schema accepts and the
    // constructor rejects, or the reverse. The matcher above is built FROM the
    // constant, so changing either side alone fails here.
    let mut corpus: Vec<String> = vec![
        String::new(),
        "work".to_owned(),
        "gh-main".to_owned(),
        "a1-b2-c3".to_owned(),
        "0".to_owned(),
        "-leading".to_owned(),
        "trailing-".to_owned(),
        "Work".to_owned(),
        "work_name".to_owned(),
        "wörk".to_owned(),
        "a".repeat(200),
    ];
    for byte in 0u8..=127 {
        let c = char::from(byte);
        corpus.push(c.to_string());
        corpus.push(format!("a{c}"));
    }

    for name in corpus {
        assert_eq!(
            SourceName::new(name.clone()).is_ok(),
            matches_published_pattern(&name),
            "SourceName::new and SOURCE_NAME_PATTERN ({SOURCE_NAME_PATTERN}) disagree \
             about {name:?}. They are one rule in two places — change both together, or \
             a configuration the published schema accepts is refused at load."
        );
    }
}
