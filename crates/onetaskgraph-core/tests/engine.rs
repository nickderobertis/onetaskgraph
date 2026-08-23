//! The engine's public surface: qualification, resume tokens, the registry, and
//! the schema bundle both SDKs are generated from.

use std::collections::BTreeMap;

use onetaskgraph_core::{
    GlobalId, PageToken, Predicate, QueryPlan, QueryResponse, SCHEMA_BUNDLE_VERSION, SourceFailure,
    SourcePlan, plugin_for, plugin_kinds, registry, schema_bundle,
};
use onetaskgraph_plugin_api::{
    Cursor, NativeId, SecretResolver, SourceError, SourceName, Status, StatusCategory, Task,
};
use secrecy::SecretString;

/// No source in this crate's tests needs a credential.
struct NoSecrets;
impl SecretResolver for NoSecrets {
    fn get(&self, _var: &str) -> Option<SecretString> {
        None
    }
}

fn source(name: &str) -> SourceName {
    SourceName::new(name).expect("a valid source name")
}

#[test]
fn a_global_id_renders_as_source_colon_native() {
    let id = GlobalId::new(source("work"), NativeId::from("ENG-1"));
    assert_eq!(id.to_string(), "work:ENG-1");
    assert_eq!(String::from(id.clone()), "work:ENG-1");
    assert_eq!(id.source.as_str(), "work");
    assert_eq!(id.native.as_str(), "ENG-1");
}

#[test]
fn a_global_id_parses_by_splitting_on_the_first_colon_so_a_native_id_may_contain_them() {
    let id: GlobalId = "notes:urn:task:7".parse().expect("parses");
    assert_eq!(id.source.as_str(), "notes");
    assert_eq!(id.native.as_str(), "urn:task:7");
    assert_eq!(id.to_string(), "notes:urn:task:7");
}

#[test]
fn an_unqualified_or_malformed_id_is_refused_with_a_suggested_form() {
    for (input, expected) in [
        ("ENG-1", "write it as <source>:<id>"),
        ("work:", "names a source but no id"),
        ("WORK:ENG-1", "source name"),
    ] {
        let Err(SourceError::Config { message }) = input.parse::<GlobalId>() else {
            panic!("{input:?} is not a usable qualified id");
        };
        assert!(message.contains(expected), "{input:?}: {message}");
    }
}

#[test]
fn a_global_id_round_trips_through_json_as_a_single_string() {
    let id = GlobalId::new(source("gh-main"), NativeId::from("PVTI_1"));
    let encoded = serde_json::to_string(&id).expect("encodes");
    assert_eq!(encoded, "\"gh-main:PVTI_1\"");
    assert_eq!(
        serde_json::from_str::<GlobalId>(&encoded).expect("decodes"),
        id
    );
    assert!(serde_json::from_str::<GlobalId>("\"unqualified\"").is_err());
}

#[test]
fn a_page_token_carries_one_cursor_per_source_and_round_trips() {
    let mut cursors = BTreeMap::new();
    cursors.insert(source("work"), Cursor("50".to_owned()));
    cursors.insert(source("notes"), Cursor("opaque-plugin-cursor".to_owned()));

    let token = PageToken::encode(&cursors).expect("encodes");
    assert_eq!(token.decode().expect("decodes"), cursors);

    // The token is opaque to a caller exactly as a plugin's cursor is to the engine.
    let encoded = serde_json::to_string(&token).expect("encodes");
    assert_eq!(
        serde_json::from_str::<PageToken>(&encoded).expect("decodes"),
        token
    );
}

#[test]
fn a_hand_edited_page_token_fails_loudly_rather_than_silently_restarting_the_walk() {
    let Err(SourceError::Malformed { message }) = PageToken("{not json".to_owned()).decode() else {
        panic!("a token this engine did not issue must be refused");
    };
    assert!(message.contains("not issued by this engine"), "{message}");

    // A well-formed token naming an unusable source is refused at the same boundary.
    let Err(SourceError::Malformed { .. }) = PageToken(r#"{"BAD_NAME":"0"}"#.to_owned()).decode()
    else {
        panic!("an invalid source name inside a token must be refused");
    };
}

#[test]
fn the_registry_names_every_plugin_kind_this_build_knows() {
    // All four are nameable from this commit on, whether or not their source is
    // implemented — so a config naming `linear` gets the plugin's own message
    // rather than "unknown plugin".
    assert_eq!(
        plugin_kinds(),
        ["github-projects", "in-memory", "linear", "local-md"]
    );
    assert_eq!(registry().len(), 4);
}

#[test]
fn the_registry_resolves_a_kind_to_its_plugin_and_nothing_to_an_unknown_one() {
    let plugin = plugin_for("in-memory").expect("the in-memory plugin is registered");
    assert_eq!(plugin.kind(), "in-memory");

    let built = plugin
        .build(&source("notes"), &serde_json::json!({}), &NoSecrets)
        .expect("an empty in-memory source is valid");
    assert_eq!(built.kind(), "in-memory");

    assert!(plugin_for("jira").is_none());
}

#[test]
fn a_registered_but_unimplemented_plugin_refuses_with_its_own_message() {
    for kind in ["linear", "github-projects", "local-md"] {
        let plugin = plugin_for(kind).expect("registered");
        let Err(SourceError::Config { message }) =
            plugin.build(&source("work"), &serde_json::json!({}), &NoSecrets)
        else {
            panic!("the `{kind}` plugin is not implemented yet, so build must refuse");
        };
        assert!(message.contains(kind), "{message}");
        assert!(message.contains("not implemented yet"), "{message}");
    }
}

#[test]
fn the_schema_bundle_describes_every_contract_root_and_every_plugin_config() {
    let bundle = schema_bundle();
    assert_eq!(bundle["version"], SCHEMA_BUNDLE_VERSION);

    let roots = bundle["roots"].as_object().expect("roots is an object");
    for root in [
        "Task",
        "Project",
        "Label",
        "Status",
        "StatusCategory",
        "DependencyEdge",
        "DependencyKind",
        "Direction",
        "TaskQuery",
        "ProjectQuery",
        "PageRequest",
        "PageOfTask",
        "PageOfProject",
        "PageOfLabel",
        "PageOfDependencyEdge",
        "Capabilities",
        "Health",
        "SourceError",
        "GlobalId",
        "QueryPlan",
        "SourcePlan",
        "Predicate",
        "SourceFailure",
        "QueryResponseOfTask",
        "QueryResponseOfProject",
        "QueryResponseOfLabel",
    ] {
        assert!(roots[root].is_object(), "the bundle is missing {root}");
    }

    let plugins = bundle["plugin_config"]
        .as_object()
        .expect("plugin_config is an object");
    let mut kinds: Vec<&String> = plugins.keys().collect();
    kinds.sort();
    assert_eq!(
        kinds,
        ["github-projects", "in-memory", "linear", "local-md"]
    );
}

#[test]
fn a_response_carries_the_plan_that_produced_it_and_round_trips() {
    // `--explain` renders this; `--json` carries it. The point is that two sources
    // of differing capability answer one query correctly by different routes, and
    // a user can see which.
    let response = QueryResponse {
        items: vec![Task {
            id: NativeId::from("ENG-1"),
            title: "Land the contract".to_owned(),
            content: None,
            status: Status {
                category: StatusCategory::InProgress,
                name: "In Review".to_owned(),
            },
            labels: Vec::new(),
            project: None,
            url: None,
            created_at: None,
            updated_at: None,
        }],
        next: Some(PageToken(r#"{"work":"50"}"#.to_owned())),
        plan: QueryPlan {
            per_source: vec![
                SourcePlan {
                    source: source("work"),
                    kind: "in-memory".to_owned(),
                    pushed_down: vec![Predicate::Label, Predicate::Status],
                    applied_locally: Vec::new(),
                    emulated: Vec::new(),
                    unavailable: Vec::new(),
                    pages_fetched: 1,
                },
                SourcePlan {
                    source: source("notes"),
                    kind: "local-md".to_owned(),
                    pushed_down: Vec::new(),
                    applied_locally: vec![Predicate::Label, Predicate::SearchContent],
                    emulated: vec![Predicate::ReverseDependencies],
                    unavailable: vec![Predicate::Project],
                    pages_fetched: 4,
                },
            ],
        },
        errors: vec![SourceFailure {
            source: source("gh-main"),
            error: SourceError::Unavailable {
                message: "no route to host".to_owned(),
            },
        }],
    };

    let encoded = serde_json::to_string(&response).expect("encodes");
    let decoded: QueryResponse<Task> = serde_json::from_str(&encoded).expect("decodes");
    assert_eq!(decoded, response);

    // One source failing never fails the whole query: the other results stand.
    assert_eq!(decoded.items.len(), 1);
    assert_eq!(decoded.errors.len(), 1);
    assert_eq!(decoded.plan.per_source.len(), 2);
    assert_eq!(QueryPlan::default().per_source, Vec::new());
}

#[test]
fn every_predicate_serialises_as_kebab_case_so_explain_output_is_stable() {
    for (predicate, wire) in [
        (Predicate::Label, "label"),
        (Predicate::Status, "status"),
        (Predicate::SearchTitle, "search-title"),
        (Predicate::SearchContent, "search-content"),
        (Predicate::Project, "project"),
        (Predicate::ReverseDependencies, "reverse-dependencies"),
    ] {
        assert_eq!(
            serde_json::to_value(predicate).expect("encodes"),
            serde_json::json!(wire)
        );
        assert_eq!(
            serde_json::from_value::<Predicate>(serde_json::json!(wire)).expect("decodes"),
            predicate
        );
    }
}
