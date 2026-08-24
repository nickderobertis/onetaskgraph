//! The engine's public surface: qualification, resume tokens, the registry, and
//! the schema bundle both SDKs are generated from.

use onetaskgraph_core::{
    GlobalId, PageToken, Predicate, QueryPlan, QueryResponse, SCHEMA_BUNDLE_VERSION, SourceFailure,
    SourcePlan, plugin_for, plugin_kinds, registry, schema_bundle,
};
use onetaskgraph_plugin_api::{
    NativeId, SecretResolver, SourceError, SourceName, Status, StatusCategory, Task,
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
fn a_string_that_is_not_this_engines_resume_document_is_refused_where_it_enters() {
    // Structural refusal, at the boundary the caller's string crosses: `parse` is the
    // only way to build one, so a string that is not this engine's document has no
    // window in which it exists as a `PageToken` at all. Whether a token that *does*
    // decode can be honoured by the query being resumed is the engine's to say — see
    // `a_page_token_this_configuration_cannot_honour_is_refused_for_what_it_says`.
    let Err(SourceError::Malformed { message }) = PageToken::parse("{not json") else {
        panic!("a string that is not this engine's document must be refused");
    };
    assert!(
        message.contains("not a page token this engine writes"),
        "{message}"
    );

    // A well-formed document of the wrong shape is refused at the same boundary, and so
    // is one naming a source no configuration could have: the token's inside is the
    // engine's, and only what the engine writes decodes.
    // Hex of `{"work":"0"}`, of a state naming an unusable source, and of `[]` — a
    // document this engine never writes, because a walk with nothing left to resume
    // reports no token at all.
    for unissued in [
        "7b22776f726b223a2230227d",
        "5b7b22736f75726365223a224241445f4e414d45222c2273747265616d223a226974656d73227d5d",
        "5b5d",
    ] {
        let Err(SourceError::Malformed { .. }) = PageToken::parse(unissued) else {
            panic!("a token this engine did not issue must be refused: {unissued}");
        };
    }

    // And deserialising goes through the same gate, so a response cannot carry one in.
    let Err(error) = serde_json::from_str::<PageToken>(r#""{not json""#) else {
        panic!("deserialising a string that is not this engine's document must be refused");
    };
    assert!(
        error
            .to_string()
            .contains("not a page token this engine writes"),
        "{error}"
    );
}

#[test]
fn the_registry_names_every_plugin_kind_this_build_knows() {
    // All five are nameable from this commit on, whether or not their source is
    // implemented — so a config naming `linear` gets the plugin's own message
    // rather than "unknown plugin". `subprocess` is a kind like any other: it is how a
    // source that is a program of its own is configured.
    assert_eq!(
        plugin_kinds(),
        [
            "github-projects",
            "in-memory",
            "linear",
            "local-md",
            "subprocess"
        ]
    );
    assert_eq!(registry().len(), 5);
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
    {
        let kind = "github-projects";
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

/// Every version of the bundle and the exact roots it published. **Append-only.**
///
/// Both SDKs are generated from this bundle, so adding, removing or renaming a root
/// changes the surface they emit, and [`SCHEMA_BUNDLE_VERSION`] is what lets an SDK
/// refuse a bundle it was not generated against. Nothing else ties the two together.
///
/// To change the roots: **append a row** with the next version and bump
/// `SCHEMA_BUNDLE_VERSION` to match. The test below checks that workflow is
/// internally consistent — the version equals the row count, no version is listed
/// twice, no two rows publish the same set, and the current row matches what the
/// binary actually emits — so the sanctioned path is the mechanically checked one.
///
/// What it deliberately does **not** claim: editing a row in place cannot be
/// detected from inside the repository, because the edited table is
/// indistinguishable from one that always read that way. The gate makes a root
/// change impossible to land *accidentally* — it will not compile past this test
/// without a conscious edit to a table that says not to do that — rather than
/// impossible to land at all. Catching an in-place edit needs the previously
/// published bundle, which lives on the registries, not here.
const PUBLISHED_BUNDLES: &[(u32, &[&str])] = &[
    (1, &FIRST_BUNDLE_ROOTS),
    (2, &SECOND_BUNDLE_ROOTS),
    (3, &THIRD_BUNDLE_ROOTS),
    (4, &FOURTH_BUNDLE_ROOTS),
];

/// The roots version 1 of the bundle publishes.
const FIRST_BUNDLE_ROOTS: [&str; 26] = [
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
];

/// The roots version 2 publishes: version 1's, plus the ones `config show --json`
/// emits. A machine-readable output with no root here is one no SDK is generated
/// against, which is the whole reason the configuration types joined the bundle.
const SECOND_BUNDLE_ROOTS: [&str; 33] = [
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
    "EffectiveConfig",
    "Setting",
    "Origin",
    "OutputFormat",
    "SecretsReport",
    "ResolvedCredential",
    "CredentialLayer",
];

/// The roots version 3 publishes: version 2's, minus the three unqualified response
/// roots, plus the ones the query verbs emit.
///
/// Every item the engine returns is **qualified** — a plugin deals in its own
/// `NativeId` and only the engine knows which source an item came from — so
/// `QueryResponseOfTask` became `QueryResponseOfQualifiedTask`. The old three are gone
/// rather than kept beside the new: no verb emits one, and a root nothing emits is a
/// model an SDK would generate and never receive.
const THIRD_BUNDLE_ROOTS: [&str; 42] = [
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
    "EffectiveConfig",
    "Setting",
    "Origin",
    "OutputFormat",
    "SecretsReport",
    "ResolvedCredential",
    "CredentialLayer",
    "PageToken",
    "QualifiedTask",
    "QualifiedProject",
    "QualifiedLabel",
    "QualifiedEdge",
    "SearchHit",
    "SourceListing",
    "QueryResponseOfQualifiedTask",
    "QueryResponseOfQualifiedProject",
    "QueryResponseOfQualifiedLabel",
    "QueryResponseOfQualifiedEdge",
    "QueryResponseOfSearchHit",
];

/// The roots version 4 publishes: version 3's, plus the two vocabularies the command line
/// accepts under `--in` and `--kind`.
///
/// Both were already reachable inside `TaskQuery`'s definitions. Roots of their own are
/// what give `the_command_line_accepts_exactly_the_vocabularies_the_contract_declares`
/// — in the binary's journeys — one document to reconcile the command line against.
const FOURTH_BUNDLE_ROOTS: [&str; 44] = [
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
    "EffectiveConfig",
    "Setting",
    "Origin",
    "OutputFormat",
    "SecretsReport",
    "ResolvedCredential",
    "CredentialLayer",
    "PageToken",
    "QualifiedTask",
    "QualifiedProject",
    "QualifiedLabel",
    "QualifiedEdge",
    "SearchHit",
    "SourceListing",
    "QueryResponseOfQualifiedTask",
    "QueryResponseOfQualifiedProject",
    "QueryResponseOfQualifiedLabel",
    "QueryResponseOfQualifiedEdge",
    "QueryResponseOfSearchHit",
    "TextFields",
    "SearchKind",
];

#[test]
fn the_schema_bundle_describes_every_contract_root_and_every_plugin_config() {
    let bundle = schema_bundle();
    assert_eq!(bundle["version"], SCHEMA_BUNDLE_VERSION);

    let roots = bundle["roots"].as_object().expect("roots is an object");

    let sorted = |names: &[&str]| {
        let mut owned: Vec<String> = names.iter().map(|name| (*name).to_owned()).collect();
        owned.sort_unstable();
        owned
    };

    // The version counts the rows, so appending one without bumping it — or bumping it
    // without appending one — fails here rather than shipping a version that describes
    // no shape.
    assert_eq!(
        SCHEMA_BUNDLE_VERSION as usize,
        PUBLISHED_BUNDLES.len(),
        "SCHEMA_BUNDLE_VERSION is {SCHEMA_BUNDLE_VERSION} but PUBLISHED_BUNDLES has {} row(s). \
         Append a row for the new shape and bump the version to match.",
        PUBLISHED_BUNDLES.len()
    );

    for (index, (version, published)) in PUBLISHED_BUNDLES.iter().enumerate() {
        assert_eq!(
            *version as usize,
            index + 1,
            "PUBLISHED_BUNDLES row {index} claims version {version}; rows are the versions \
             in order, so row {index} is version {}.",
            index + 1
        );
        // A shape may not be republished under a second version, and a version may not
        // describe two shapes — either would make the version useless for the SDK that
        // reads it.
        assert!(
            !PUBLISHED_BUNDLES[..index]
                .iter()
                .any(|(_, earlier)| sorted(earlier) == sorted(published)),
            "version {version} republishes an earlier version's exact root set; if the \
             shape did not change, the version must not either."
        );
    }

    let (_, expected) = PUBLISHED_BUNDLES
        .iter()
        .find(|(version, _)| *version == SCHEMA_BUNDLE_VERSION)
        .expect("PUBLISHED_BUNDLES lists the current SCHEMA_BUNDLE_VERSION");

    assert_eq!(
        sorted(&roots.keys().map(String::as_str).collect::<Vec<_>>()),
        sorted(expected),
        "the bundle's roots are not what version {SCHEMA_BUNDLE_VERSION} publishes. Append a \
         row to PUBLISHED_BUNDLES with the new set and bump SCHEMA_BUNDLE_VERSION to match — \
         an SDK generated against the old version would otherwise silently emit the wrong \
         models."
    );

    for root in expected.iter() {
        assert!(roots[*root].is_object(), "the bundle is missing {root}");
    }

    let plugins = bundle["plugin_config"]
        .as_object()
        .expect("plugin_config is an object");
    let mut kinds: Vec<&String> = plugins.keys().collect();
    kinds.sort();
    assert_eq!(
        kinds,
        [
            "github-projects",
            "in-memory",
            "linear",
            "local-md",
            "subprocess"
        ]
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
        // The hex of
        // `{"query":"0123456789abcdef","streams":[{"source":"work","stream":"items","cursor":"50"}]}`
        // — the fingerprint of the query that minted it, and one stream resuming at the
        // source's own opaque cursor. Spelled out rather than built, so a change to the
        // document a token carries has to be made here too and cannot pass unnoticed.
        next: Some(
            PageToken::parse("7b227175657279223a2230313233343536373839616263646566222c2273747265616d73223a5b7b22736f75726365223a22776f726b222c2273747265616d223a226974656d73222c22637572736f72223a223530227d5d7d")
                .expect("a token this engine issued"),
        ),
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
