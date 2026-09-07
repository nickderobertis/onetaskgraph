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
use serde_json::Value;

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

/// What one published version of the bundle is recorded as.
///
/// Two variants because this repository can attest less about its own past than about its
/// present. Versions 1 to 9 were recorded as a set of root *names* and nothing more, and
/// the schemas those versions really emitted are on the registries rather than here — so a
/// row for one of them can only ever say which roots it had. From version 10 the row
/// records the shape: every root, with a digest of the schema that root emitted. A row may
/// not go back from [`Self::Shapes`] to [`Self::Names`], which the test below refuses.
enum Published {
    /// Root names alone — all this repository recorded before version 10.
    Names(&'static [&'static str]),
    /// Every root's name beside a digest of the schema it emitted.
    Shapes(&'static [(&'static str, u64)]),
}

impl Published {
    /// The root names this row publishes, however it records them.
    fn names(&self) -> Vec<&'static str> {
        match self {
            Self::Names(names) => names.to_vec(),
            Self::Shapes(shapes) => shapes.iter().map(|(name, _)| *name).collect(),
        }
    }
}

/// The first version whose row records the shape of every root rather than its name alone.
const SHAPES_FROM: u32 = 10;

/// Every version of the bundle and the exact shape it published. **Append-only.**
///
/// Both SDKs are generated from this bundle, so adding, removing or renaming a root
/// changes the surface they emit — and so does changing what one root *contains*, which
/// this table records from version [`SHAPES_FROM`] on. A property added to `CopyReport` is
/// a new field in both SDKs' generated models exactly as a new root is a new model, and
/// [`SCHEMA_BUNDLE_VERSION`] is what lets an SDK refuse a bundle it was not generated
/// against. Nothing else ties the two together.
///
/// To change any root: **append a row** with the next version and bump
/// `SCHEMA_BUNDLE_VERSION` to match. The test below checks that workflow is
/// internally consistent — the version equals the row count, no version is listed
/// twice, no two rows publish the same shape, and the current row matches what the
/// binary actually emits, root for root and digest for digest — so the sanctioned path is
/// the mechanically checked one. When it disagrees it prints the row to paste.
///
/// What it deliberately does **not** claim: editing a row in place cannot be
/// detected from inside the repository, because the edited table is
/// indistinguishable from one that always read that way. The gate makes a schema
/// change impossible to land *accidentally* — it will not compile past this test
/// without a conscious edit to a table that says not to do that — rather than
/// impossible to land at all. Catching an in-place edit needs the previously
/// published bundle, which lives on the registries, not here.
///
/// The cost of recording digests is stated rather than discovered: a schemars upgrade that
/// reworded one generated keyword moves a digest and demands a version bump. That is the
/// correct answer — the emitted document really did change, and both SDKs really are
/// regenerated from it — but it is a bump nobody wrote the code for, and this is where a
/// reader meets that.
const PUBLISHED_BUNDLES: &[(u32, Published)] = &[
    (1, Published::Names(&FIRST_BUNDLE_ROOTS)),
    (2, Published::Names(&SECOND_BUNDLE_ROOTS)),
    (3, Published::Names(&THIRD_BUNDLE_ROOTS)),
    (4, Published::Names(&FOURTH_BUNDLE_ROOTS)),
    (5, Published::Names(&FIFTH_BUNDLE_ROOTS)),
    (6, Published::Names(&SIXTH_BUNDLE_ROOTS)),
    (7, Published::Names(&SEVENTH_BUNDLE_ROOTS)),
    (8, Published::Names(&EIGHTH_BUNDLE_ROOTS)),
    (9, Published::Names(&NINTH_BUNDLE_ROOTS)),
    (10, Published::Shapes(&TENTH_BUNDLE_SHAPE)),
];

/// One root's schema rendered so that two equal documents render equally.
///
/// Object keys sorted at every depth, so nothing about the order `schemars` happened to
/// build a map in reaches the digest. Arrays keep their order, because in JSON Schema an
/// array's order is part of what it says.
fn canonical(value: &Value) -> String {
    match value {
        Value::Object(fields) => {
            let mut keys: Vec<&String> = fields.keys().collect();
            keys.sort_unstable();
            let rendered: Vec<String> = keys
                .iter()
                .map(|key| {
                    format!(
                        "{}:{}",
                        serde_json::to_string(key).expect("a key renders"),
                        canonical(&fields[*key])
                    )
                })
                .collect();
            format!("{{{}}}", rendered.join(","))
        }
        Value::Array(items) => {
            let rendered: Vec<String> = items.iter().map(canonical).collect();
            format!("[{}]", rendered.join(","))
        }
        other => serde_json::to_string(other).expect("a scalar renders"),
    }
}

/// A change-detecting digest of one root's emitted schema.
///
/// FNV-1a over the canonical rendering above, written out here rather than taken from a
/// crate: what this has to catch is a schema that changed without the version moving, and
/// any digest that changes when its input does catches that. It defends against nothing
/// adversarial and does not pretend to — whoever can edit a row of the table above can edit
/// the digest beside it, which the table already says of itself.
fn digest(schema: &Value) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in canonical(schema).as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// The shape version 10 of the bundle publishes: every root, and a digest of its schema.
///
/// Version 10 is the first row recorded this way, and the change it records is `CopyReport`
/// gaining the three figures a copy reports about the references its documents hold. Under
/// the name-only rows above that change was invisible: no root was added, removed or
/// renamed, and both SDKs' generated models moved all the same.
const TENTH_BUNDLE_SHAPE: [(&str, u64); 58] = [
    ("Capabilities", 0xa10633f78133cd5f),
    ("CopyAction", 0x92821be0daa46894),
    ("CopyOutcome", 0xefcf23cfbd5dde3b),
    ("CopyReport", 0xd45b707b51401c93),
    ("CredentialLayer", 0x54cdffe467a3b7f1),
    ("DependencyEdge", 0x965fcb2880071dcc),
    ("DependencyEndpoint", 0x52371a0138569604),
    ("DependencyKind", 0x62a3106e8479701a),
    ("Direction", 0x497cbce272a8a5bb),
    ("Document", 0x43f55b02791ea15b),
    ("DocumentQuery", 0xab8ca467012e1a12),
    ("EffectiveConfig", 0xa41a3361af17368f),
    ("GlobalId", 0xe692661021d9c53e),
    ("Health", 0x4a65ae032f76c6ca),
    ("ItemKind", 0x75db1aa08ed04b2f),
    ("Label", 0x07555503d77a90c7),
    ("Location", 0x0690620b049c989a),
    ("Origin", 0x653235a0d0c3576e),
    ("OutputFormat", 0xa8a85cfd04d98684),
    ("PageOfDependencyEdge", 0x529f3ea40c6b71c9),
    ("PageOfDocument", 0x985c78b3f3c93eeb),
    ("PageOfLabel", 0xd6110ddeeaaa78e8),
    ("PageOfProject", 0x41abc46f0e84e0d7),
    ("PageOfTask", 0x0921303f42a6cb4e),
    ("PageRequest", 0x6c7be3975028b78c),
    ("PageToken", 0xc685683af2c78c39),
    ("Predicate", 0x2c7629f85d039fc6),
    ("Project", 0x27060ceb590bd2d1),
    ("ProjectQuery", 0x4ab0fa6b012cc9bd),
    ("QualifiedDocument", 0x4e539a8ce7b72c47),
    ("QualifiedEdge", 0xe24f9b34b618df17),
    ("QualifiedEndpoint", 0x7bc0f5163c4c8594),
    ("QualifiedLabel", 0x7ac91fcfd8f41e30),
    ("QualifiedProject", 0x78c2101cd2a90b0d),
    ("QualifiedTask", 0xb6c4cf33ff76e1b2),
    ("QueryPlan", 0x5cd046eed149f89b),
    ("QueryResponseOfQualifiedDocument", 0xea3687e0592dde61),
    ("QueryResponseOfQualifiedEdge", 0x747c236f676fa46a),
    ("QueryResponseOfQualifiedLabel", 0x634a582d4af1fad0),
    ("QueryResponseOfQualifiedProject", 0xd6017091147e11c7),
    ("QueryResponseOfQualifiedTask", 0x2640335859efe43e),
    ("QueryResponseOfSearchHit", 0x6d7372360674cc65),
    ("Repository", 0x98147ade92ced0f0),
    ("ResolvedCredential", 0x14a23b081a4e8d10),
    ("SearchHit", 0xb3b5470d71a6d866),
    ("SearchKind", 0xc4d2cd105ad4b849),
    ("SecretsReport", 0x245d50b08721b73d),
    ("Setting", 0xf593f9ae902cba68),
    ("SourceError", 0x33872c91770f86da),
    ("SourceFailure", 0x452bd4b53ef4d74c),
    ("SourceListing", 0x006592f26f65b8b6),
    ("SourceListings", 0x67cd161375100dcb),
    ("SourcePlan", 0xd0c5548abc7d7223),
    ("Status", 0xd14c325a52e464f6),
    ("StatusCategory", 0xc866ba4d0d422da0),
    ("Task", 0xe39a3442bae8ceda),
    ("TaskQuery", 0x963c214c94159671),
    ("TextFields", 0x7240bd05f9beff93),
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

/// Version 5 adds the array shape emitted by `sources list`; version 4 described one
/// listing even though that command serialises the complete list.
const FIFTH_BUNDLE_ROOTS: [&str; 45] = [
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
    "SourceListings",
    "QueryResponseOfQualifiedTask",
    "QueryResponseOfQualifiedProject",
    "QueryResponseOfQualifiedLabel",
    "QueryResponseOfQualifiedEdge",
    "QueryResponseOfSearchHit",
    "TextFields",
    "SearchKind",
];

/// Version 6 adds repository identity and typed dependency endpoints.
const SIXTH_BUNDLE_ROOTS: [&str; 49] = [
    "Task",
    "Project",
    "Label",
    "Status",
    "StatusCategory",
    "DependencyEdge",
    "DependencyEndpoint",
    "QualifiedEndpoint",
    "ItemKind",
    "Repository",
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
    "SourceListings",
    "QueryResponseOfQualifiedTask",
    "QueryResponseOfQualifiedProject",
    "QueryResponseOfQualifiedLabel",
    "QueryResponseOfQualifiedEdge",
    "QueryResponseOfSearchHit",
    "TextFields",
    "SearchKind",
];

/// Version 7 adds what a copy answers with: one outcome per item it considered.
const SEVENTH_BUNDLE_ROOTS: [&str; 52] = [
    "Task",
    "Project",
    "Label",
    "Status",
    "StatusCategory",
    "DependencyEdge",
    "DependencyEndpoint",
    "QualifiedEndpoint",
    "ItemKind",
    "Repository",
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
    "SourceListings",
    "QueryResponseOfQualifiedTask",
    "QueryResponseOfQualifiedProject",
    "QueryResponseOfQualifiedLabel",
    "QueryResponseOfQualifiedEdge",
    "QueryResponseOfSearchHit",
    "TextFields",
    "SearchKind",
    "CopyReport",
    "CopyOutcome",
    "CopyAction",
];

/// Version 8 adds the documents contract: a `Document`, the `Location` a consumer acts on
/// without knowing the backend, and the query and page a document read answers with.
const EIGHTH_BUNDLE_ROOTS: [&str; 56] = [
    "Task",
    "Project",
    "Document",
    "Location",
    "Label",
    "Status",
    "StatusCategory",
    "DependencyEdge",
    "DependencyEndpoint",
    "QualifiedEndpoint",
    "ItemKind",
    "Repository",
    "DependencyKind",
    "Direction",
    "TaskQuery",
    "ProjectQuery",
    "DocumentQuery",
    "PageRequest",
    "PageOfTask",
    "PageOfProject",
    "PageOfDocument",
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
    "SourceListings",
    "QueryResponseOfQualifiedTask",
    "QueryResponseOfQualifiedProject",
    "QueryResponseOfQualifiedLabel",
    "QueryResponseOfQualifiedEdge",
    "QueryResponseOfSearchHit",
    "TextFields",
    "SearchKind",
    "CopyReport",
    "CopyOutcome",
    "CopyAction",
];
/// Version 9 makes documents reachable: a document read answers with a qualified document
/// and the response that carries it, which is what the `document` verb group returns and
/// what an SDK generates a model for.
const NINTH_BUNDLE_ROOTS: [&str; 58] = [
    "Task",
    "Project",
    "Document",
    "Location",
    "Label",
    "Status",
    "StatusCategory",
    "DependencyEdge",
    "DependencyEndpoint",
    "QualifiedEndpoint",
    "ItemKind",
    "Repository",
    "DependencyKind",
    "Direction",
    "TaskQuery",
    "ProjectQuery",
    "DocumentQuery",
    "PageRequest",
    "PageOfTask",
    "PageOfProject",
    "PageOfDocument",
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
    "SourceListings",
    "QueryResponseOfQualifiedTask",
    "QueryResponseOfQualifiedProject",
    "QueryResponseOfQualifiedLabel",
    "QueryResponseOfQualifiedEdge",
    "QueryResponseOfSearchHit",
    "TextFields",
    "SearchKind",
    "CopyReport",
    "CopyOutcome",
    "CopyAction",
    "QualifiedDocument",
    "QueryResponseOfQualifiedDocument",
];

/// The emitted shape rendered as the Rust literal a reader pastes into the table.
///
/// A diagnostic that hands over the answer, because the alternative is a reader deriving
/// fifty-eight digests by hand from a message that only says they were wrong.
fn literal(emitted: &[(&str, u64)]) -> String {
    let rows: Vec<String> = emitted
        .iter()
        .map(|(name, digest)| format!("    (\"{name}\", {digest:#018x}),"))
        .collect();
    format!(
        "const NEXT_BUNDLE_SHAPE: [(&str, u64); {}] = [\n{}\n];",
        emitted.len(),
        rows.join("\n")
    )
}

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
        // Once a version records the shape, no later version may record less: a row that
        // fell back to names would silently stop noticing a changed root.
        assert!(
            *version < SHAPES_FROM || matches!(published, Published::Shapes(_)),
            "version {version} is at or past {SHAPES_FROM} and must record every root's \
             schema, not its name alone."
        );
        // A shape may not be republished under a second version, and a version may not
        // describe two shapes — either would make the version useless for the SDK that
        // reads it.
        assert!(
            !PUBLISHED_BUNDLES[..index].iter().any(|(_, earlier)| {
                match (earlier, published) {
                    (Published::Names(earlier), Published::Names(now)) => {
                        sorted(earlier) == sorted(now)
                    }
                    (Published::Shapes(earlier), Published::Shapes(now)) => {
                        let mut earlier = earlier.to_vec();
                        let mut now = now.to_vec();
                        earlier.sort_unstable();
                        now.sort_unstable();
                        earlier == now
                    }
                    _ => false,
                }
            }),
            "version {version} republishes an earlier version's exact shape; if the \
             shape did not change, the version must not either."
        );
    }

    let (_, expected) = PUBLISHED_BUNDLES
        .iter()
        .find(|(version, _)| *version == SCHEMA_BUNDLE_VERSION)
        .expect("PUBLISHED_BUNDLES lists the current SCHEMA_BUNDLE_VERSION");

    assert_eq!(
        sorted(&roots.keys().map(String::as_str).collect::<Vec<_>>()),
        sorted(&expected.names()),
        "the bundle's roots are not what version {SCHEMA_BUNDLE_VERSION} publishes. Append a \
         row to PUBLISHED_BUNDLES with the new shape and bump SCHEMA_BUNDLE_VERSION to match \
         — an SDK generated against the old version would otherwise silently emit the wrong \
         models."
    );

    for root in expected.names() {
        assert!(roots[root].is_object(), "the bundle is missing {root}");
    }

    // And what each root *contains*, which the root names alone cannot see. A property
    // added to `CopyReport` is a new field in both SDKs' generated models exactly as a new
    // root is a new model, so it moves the version or it fails here.
    if let Published::Shapes(published) = expected {
        let mut emitted: Vec<(&str, u64)> = roots
            .iter()
            .map(|(name, schema)| (name.as_str(), digest(schema)))
            .collect();
        emitted.sort_unstable();
        let mut recorded = published.to_vec();
        recorded.sort_unstable();
        assert_eq!(
            emitted,
            recorded,
            "the schemas version {SCHEMA_BUNDLE_VERSION} publishes are not the ones the \
             binary emits. If this change is deliberate, append a row to PUBLISHED_BUNDLES \
             and bump SCHEMA_BUNDLE_VERSION to match; the row to paste is:\n{}",
            literal(&emitted)
        );
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
            location: None,
            created_at: None,
            updated_at: None,
            metadata: Default::default(),
            repositories: Vec::new(),
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
