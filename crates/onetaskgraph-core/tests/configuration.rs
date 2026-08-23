//! The configuration layer, the credentials file, and resolution into live sources.
//!
//! The journeys that prove this through the binary live in the `onetaskgraph` crate.
//! What is here is the engine's own surface: the layer algebra a document, the
//! environment and the command line all reduce to, the reads that find them, and the
//! step that turns a validated configuration into sources. Nothing is mocked —
//! discovery runs against real temporary directories, because a fake filesystem
//! would only prove that the fake agrees with itself.

use std::collections::BTreeMap;
use std::path::Path;

use onetaskgraph_core::config::{
    self, ConfigError, Layer, Origin, PROJECT_DOCUMENT_NAME, SECRETS_FILE_VARIABLE,
    SECRETS_RELATIVE_PATH, Setting, SettingPath, USER_DOCUMENT_RELATIVE_PATH, documents, merge,
    read_optional, secrets_path, unflatten, user_document_path, value_from_text, variable_for,
};
use onetaskgraph_core::{Config, Environment, OutputFormat, Secrets, plugin_for, resolve};
use onetaskgraph_plugin_api::{SecretResolver, SourceName};
use serde_json::{Value, json};
use tempfile::TempDir;

/// A path, for tests that read like the settings they name.
fn path(dotted: &str) -> SettingPath {
    SettingPath::parse(dotted).expect("a literal path")
}

/// One setting attributed to a made-up flag.
fn flag(dotted: &str, value: Value) -> Setting {
    Setting {
        key: path(dotted),
        value,
        origin: Origin::Flag {
            flag: format!("--set {dotted}"),
        },
    }
}

/// One document layer over `document`, attributed to a file called `name`.
fn document(name: &str, document: Value) -> Layer {
    Layer::from_document(Path::new(name).to_path_buf(), &document).expect("a mapping")
}

// ---------------------------------------------------------------------------
// The layer algebra: pure, and the same for all three layers.
// ---------------------------------------------------------------------------

#[test]
fn a_later_layer_replaces_a_setting_an_earlier_one_made() {
    let merged = merge(&[
        document("a.yaml", json!({"page_size": 25})),
        Layer::new(vec![flag("page_size", json!(9))]),
    ]);

    let page_size = &merged[&path("page_size")];
    assert_eq!(page_size.value, json!(9));
    assert_eq!(
        page_size.origin,
        Origin::Flag {
            flag: "--set page_size".to_owned()
        }
    );
    assert_eq!(merged.len(), 1, "one setting, not two");
}

#[test]
fn a_later_layer_setting_a_leaf_replaces_an_earlier_layers_whole_subtree() {
    let merged = merge(&[
        Layer::new(vec![flag(
            "sources.work.config",
            json!({"root": "/one", "depth": 2}),
        )]),
        Layer::new(vec![flag("sources.work.config.root", json!("/two"))]),
    ]);

    assert_eq!(
        merged.keys().cloned().collect::<Vec<_>>(),
        vec![path("sources.work.config.root")],
        "a leaf below a replaced subtree does not survive beside it"
    );
}

#[test]
fn a_later_layer_setting_a_subtree_replaces_an_earlier_layers_leaves_below_it() {
    let merged = merge(&[
        Layer::new(vec![flag("sources.work.config.root", json!("/one"))]),
        Layer::new(vec![flag("sources.work.config", json!({"root": "/two"}))]),
    ]);

    assert_eq!(
        merged.keys().cloned().collect::<Vec<_>>(),
        vec![path("sources.work.config")]
    );
}

#[test]
fn merging_and_unflattening_rebuild_the_document_the_layers_describe() {
    let merged = merge(&[
        document("a.yaml", json!({"page_size": 25})),
        Layer::new(vec![
            flag("sources.work.plugin", json!("in-memory")),
            flag("sources.work.config.root", json!("/notes")),
        ]),
    ]);

    assert_eq!(
        unflatten(&merged),
        json!({
            "page_size": 25,
            "sources": { "work": { "plugin": "in-memory", "config": { "root": "/notes" } } }
        })
    );
}

#[test]
fn an_empty_plugin_block_survives_flattening_as_a_deliberate_setting() {
    let merged = merge(&[document(
        "a.yaml",
        json!({"sources": {"work": {"plugin": "linear", "config": {}}}}),
    )]);
    assert_eq!(merged[&path("sources.work.config")].value, json!({}));
}

#[test]
fn an_empty_document_contributes_nothing() {
    let layer = Layer::from_document(Path::new("a.yaml").to_path_buf(), &Value::Null)
        .expect("an empty document");
    assert!(layer.settings().is_empty());
}

#[test]
fn a_document_that_is_not_a_mapping_is_refused() {
    let error = Layer::from_document(Path::new("a.yaml").to_path_buf(), &json!([1, 2]))
        .expect_err("a list is not a document");
    assert!(error.to_string().contains("a list"), "{error}");
}

#[test]
fn a_setting_path_with_an_empty_segment_addresses_nothing() {
    let error = SettingPath::parse("sources..plugin").expect_err("an empty segment");
    assert_eq!(error.key(), Some("sources..plugin"));
}

// ---------------------------------------------------------------------------
// Reading one textual value, shared by the environment and by `--set`.
// ---------------------------------------------------------------------------

#[test]
fn a_textual_value_is_typed_as_far_as_it_reads() {
    assert_eq!(value_from_text("100"), json!(100));
    assert_eq!(value_from_text("-3"), json!(-3));
    assert_eq!(value_from_text("1.5"), json!(1.5));
    assert_eq!(value_from_text("true"), json!(true));
    assert_eq!(value_from_text("false"), json!(false));
    assert_eq!(value_from_text("/tmp/tasks"), json!("/tmp/tasks"));
    assert_eq!(
        value_from_text("github-projects"),
        json!("github-projects"),
        "a hyphen does not make a value arithmetic"
    );
    assert_eq!(
        value_from_text("inf"),
        json!("inf"),
        "a float that has no JSON number stays the text it was"
    );
}

#[test]
fn a_textual_value_containing_a_comma_is_a_list() {
    assert_eq!(value_from_text("work,notes"), json!(["work", "notes"]));
    assert_eq!(value_from_text("1, 2"), json!([1, 2]));
}

// ---------------------------------------------------------------------------
// The environment naming rule, in the direction the documentation states it.
// ---------------------------------------------------------------------------

#[test]
fn the_variable_for_a_setting_is_the_one_the_contract_documents() {
    assert_eq!(variable_for(&path("page_size")), "ONETASKGRAPH_PAGE_SIZE");
    assert_eq!(
        variable_for(&path("default_sources")),
        "ONETASKGRAPH_DEFAULT_SOURCES"
    );
    assert_eq!(
        variable_for(&path("sources.work.config.root")),
        "ONETASKGRAPH_SOURCES__WORK__CONFIG__ROOT"
    );
    assert_eq!(
        variable_for(&path("sources.gh-main.plugin")),
        "ONETASKGRAPH_SOURCES__GH_MAIN__PLUGIN",
        "a hyphen in a source name is an underscore in the variable"
    );
    assert_eq!(config::ENVIRONMENT_PREFIX, "ONETASKGRAPH_");
}

#[test]
fn a_variable_that_names_no_path_is_refused_rather_than_ignored() {
    let sandbox = TempDir::new().expect("a temporary directory");
    let environment = Environment::from_pairs([
        ("HOME", sandbox.path().to_string_lossy().to_string()),
        ("ONETASKGRAPH_SOURCES__", "in-memory".to_owned()),
    ]);

    let error = config::load(sandbox.path(), &environment, &Layer::default())
        .expect_err("a variable naming no setting");
    assert_eq!(error.key(), Some("ONETASKGRAPH_SOURCES__"));
}

// ---------------------------------------------------------------------------
// Discovery: the only filesystem reads this crate makes.
// ---------------------------------------------------------------------------

/// A host with a configuration home and a project tree.
struct Host {
    root: TempDir,
}

impl Host {
    fn new() -> Self {
        let root = TempDir::new().expect("a temporary directory");
        std::fs::create_dir_all(root.path().join("home/onetaskgraph")).expect("a config home");
        std::fs::create_dir_all(root.path().join("project/deep/deeper")).expect("a project tree");
        Self { root }
    }

    fn environment(&self) -> Environment {
        Environment::from_pairs([(
            "XDG_CONFIG_HOME",
            self.root.path().join("home").to_string_lossy().to_string(),
        )])
    }

    fn write(&self, relative: &str, text: &str) -> std::path::PathBuf {
        let path = self.root.path().join(relative);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("the directory");
        std::fs::write(&path, text).expect("written");
        path
    }
}

#[test]
fn documents_are_the_user_level_one_then_the_nearest_one_above_the_working_directory() {
    let host = Host::new();
    let user = host.write(
        &format!("home/{USER_DOCUMENT_RELATIVE_PATH}"),
        "page_size: 11\n",
    );
    let project = host.write(
        &format!("project/{PROJECT_DOCUMENT_NAME}"),
        "page_size: 25\n",
    );

    let found = documents(
        &host.root.path().join("project/deep/deeper"),
        &host.environment(),
    )
    .expect("both documents read");

    assert_eq!(
        found.iter().map(|d| d.path.clone()).collect::<Vec<_>>(),
        vec![user, project],
        "lowest precedence first"
    );
}

#[test]
fn a_nearer_document_hides_one_further_up_rather_than_stacking_with_it() {
    let host = Host::new();
    host.write(
        &format!("project/{PROJECT_DOCUMENT_NAME}"),
        "page_size: 25\n",
    );
    let nearer = host.write(
        &format!("project/deep/{PROJECT_DOCUMENT_NAME}"),
        "page_size: 7\n",
    );

    let found = documents(
        &host.root.path().join("project/deep/deeper"),
        &host.environment(),
    )
    .expect("one document read");

    assert_eq!(
        found.iter().map(|d| d.path.clone()).collect::<Vec<_>>(),
        vec![nearer]
    );
}

#[test]
fn no_documents_at_all_is_the_ordinary_case_rather_than_an_error() {
    let host = Host::new();
    let found =
        documents(&host.root.path().join("project"), &host.environment()).expect("nothing to read");
    assert!(found.is_empty());
}

#[test]
fn a_document_that_cannot_be_read_stops_the_run_rather_than_being_skipped() {
    let host = Host::new();
    // A directory where the user-level document belongs: it exists as far as a read is
    // concerned and cannot be read, without needing a permission this test user might
    // not be able to set.
    std::fs::create_dir_all(
        host.root
            .path()
            .join(format!("home/{USER_DOCUMENT_RELATIVE_PATH}")),
    )
    .expect("a directory in the document's place");

    let error = documents(&host.root.path().join("project"), &host.environment())
        .expect_err("a document that cannot be read");
    assert!(matches!(error, ConfigError::Read { .. }), "{error}");
    assert!(error.to_string().contains("config.yaml"), "{error}");
    assert_eq!(error.key(), None);
}

#[test]
fn the_configuration_home_falls_back_to_home_when_xdg_is_unset_or_empty() {
    let from_home = Environment::from_pairs([("HOME", "/home/someone")]);
    assert_eq!(
        user_document_path(&from_home),
        Some(Path::new("/home/someone/.config").join(USER_DOCUMENT_RELATIVE_PATH))
    );

    let empty_xdg = Environment::from_pairs([("XDG_CONFIG_HOME", ""), ("HOME", "/home/someone")]);
    assert_eq!(
        user_document_path(&empty_xdg),
        user_document_path(&from_home)
    );

    assert_eq!(
        user_document_path(&Environment::default()),
        None,
        "a host that says where nothing lives has no user-level document"
    );
}

#[test]
fn the_credentials_path_is_the_override_when_one_is_set_and_the_default_otherwise() {
    let overridden = Environment::from_pairs([
        (SECRETS_FILE_VARIABLE, "/tmp/elsewhere.env"),
        ("HOME", "/home/someone"),
    ]);
    assert_eq!(
        secrets_path(&overridden),
        Some(Path::new("/tmp/elsewhere.env").to_path_buf())
    );

    let default = Environment::from_pairs([("XDG_CONFIG_HOME", "/cfg")]);
    assert_eq!(
        secrets_path(&default),
        Some(Path::new("/cfg").join(SECRETS_RELATIVE_PATH))
    );

    assert_eq!(secrets_path(&Environment::default()), None);
}

#[test]
fn reading_a_file_that_is_not_there_is_nothing_rather_than_an_error() {
    let host = Host::new();
    assert_eq!(
        read_optional(&host.root.path().join("nothing-here")).expect("not an error"),
        None
    );
}

// ---------------------------------------------------------------------------
// The environment snapshot, which holds credentials and must not print them.
// ---------------------------------------------------------------------------

#[test]
fn the_environment_snapshot_never_debug_prints_a_value() {
    let environment = Environment::from_pairs([("LINEAR_API_KEY", "lin_api_CANARY")]);

    let rendered = format!("{environment:?}");
    assert!(!rendered.contains("lin_api_CANARY"), "{rendered}");
    assert!(rendered.contains("LINEAR_API_KEY"), "{rendered}");
    assert!(rendered.contains("<redacted>"), "{rendered}");
}

#[test]
fn the_environment_snapshot_reads_this_process() {
    // Nothing is asserted about a particular variable: what matters is that the
    // capture succeeds and yields the process's own names, which is what the binary
    // hands every layer below it.
    let captured = Environment::from_process();
    assert_eq!(
        captured.names().count(),
        std::env::vars().count(),
        "every variable this process has is captured"
    );
    assert!(captured.non_empty("").is_none());
}

// ---------------------------------------------------------------------------
// The credentials file.
// ---------------------------------------------------------------------------

/// A resolver over a credentials file holding `text`, plus `exported`.
fn secrets(host: &Host, text: &str, exported: &[(&str, &str)]) -> Result<Secrets, ConfigError> {
    host.write(&format!("home/{SECRETS_RELATIVE_PATH}"), text);
    let mut pairs: Vec<(String, String)> = vec![(
        "XDG_CONFIG_HOME".to_owned(),
        host.root.path().join("home").to_string_lossy().to_string(),
    )];
    for (name, value) in exported {
        pairs.push(((*name).to_owned(), (*value).to_owned()));
    }
    Secrets::load(Environment::from_pairs(pairs))
}

#[test]
fn the_credentials_file_answers_the_names_it_defines() {
    let host = Host::new();
    let resolved = secrets(
        &host,
        "# a comment\n\nLINEAR_API_KEY=lin_api_one\nexport GH_PROJECTS_TOKEN=\"ghp_two\"\n",
        &[],
    )
    .expect("the file parses");

    assert!(resolved.get("LINEAR_API_KEY").is_some());
    assert!(
        resolved.get("GH_PROJECTS_TOKEN").is_some(),
        "an `export ` prefix and surrounding quotes are both read"
    );
    assert!(resolved.get("NOTHING_DEFINES_THIS").is_none());
}

#[test]
fn the_process_environment_beats_the_credentials_file_for_a_name_it_defines() {
    let host = Host::new();
    let resolved = secrets(
        &host,
        "LINEAR_API_KEY=from-the-file\n",
        &[("LINEAR_API_KEY", "from-the-shell")],
    )
    .expect("the file parses");

    let report = resolved.report();
    assert_eq!(report.variables.len(), 1);
    assert_eq!(
        report.variables[0].resolved_from,
        onetaskgraph_core::CredentialLayer::Environment
    );
}

#[test]
fn a_credentials_file_line_that_is_not_an_assignment_names_its_line_and_not_its_value() {
    let host = Host::new();
    let error = secrets(
        &host,
        "LINEAR_API_KEY=canary-value\nnot an assignment\n",
        &[],
    )
    .expect_err("a malformed line");

    let message = error.to_string();
    assert!(message.contains(":2"), "{message}");
    assert!(!message.contains("canary-value"), "{message}");
}

#[test]
fn a_credentials_file_key_that_could_not_be_exported_is_refused() {
    let host = Host::new();
    let error = secrets(&host, "not a name=value\n", &[]).expect_err("an unusable name");
    assert!(error.to_string().contains("not a name"), "{error}");
}

#[test]
fn the_resolver_never_debug_prints_a_value() {
    let host = Host::new();
    let resolved = secrets(&host, "LINEAR_API_KEY=lin_api_CANARY\n", &[]).expect("parses");

    let rendered = format!("{resolved:?}");
    assert!(!rendered.contains("lin_api_CANARY"), "{rendered}");
    assert!(rendered.contains("LINEAR_API_KEY"), "{rendered}");
    assert!(rendered.contains("<redacted>"), "{rendered}");
}

#[test]
fn a_host_that_says_where_nothing_lives_resolves_nothing_and_does_not_fail() {
    let resolved = Secrets::load(Environment::default()).expect("nothing to read");
    assert_eq!(resolved.report().path, None);
    assert!(resolved.report().variables.is_empty());
    assert!(resolved.get("LINEAR_API_KEY").is_none());
}

// ---------------------------------------------------------------------------
// The validated configuration.
// ---------------------------------------------------------------------------

#[test]
fn a_document_with_nothing_in_it_carries_the_built_in_values() {
    let config = Config::from_document(json!({})).expect("an empty document is a configuration");
    assert_eq!(config.page_size.get(), 50);
    assert_eq!(config.output, OutputFormat::Text);
    assert!(config.sources.is_empty());
    assert_eq!(config.default_sources, None);
    assert!(config.selected_sources().is_empty());
}

#[test]
fn omitting_default_sources_selects_every_configured_source_in_name_order() {
    let config = Config::from_document(json!({
        "sources": {
            "work": {"plugin": "in-memory"},
            "notes": {"plugin": "in-memory"},
        }
    }))
    .expect("a configuration");

    assert_eq!(
        config
            .selected_sources()
            .iter()
            .map(SourceName::as_str)
            .collect::<Vec<_>>(),
        vec!["notes", "work"]
    );
}

#[test]
fn one_default_source_named_alone_is_read_as_a_list_of_one() {
    let config = Config::from_document(json!({
        "default_sources": "work",
        "sources": {"work": {"plugin": "in-memory"}},
    }))
    .expect("a configuration");

    assert_eq!(
        config.selected_sources(),
        vec![SourceName::new("work").expect("a name")]
    );
}

#[test]
fn an_unknown_field_names_the_key_it_was_written_at() {
    let error = Config::from_document(json!({"sources": {"work": {"plugn": "in-memory"}}}))
        .expect_err("a mistyped field");
    assert_eq!(error.key(), Some("sources.work.plugn"));
}

#[test]
fn a_default_source_naming_something_unconfigured_is_refused() {
    let error = Config::from_document(json!({"default_sources": ["nope"]}))
        .expect_err("an unconfigured source");
    assert_eq!(error.key(), Some("default_sources"));
    assert!(error.to_string().contains("none are"), "{error}");
}

#[test]
fn a_default_source_that_is_not_a_usable_name_is_refused() {
    let error = Config::from_document(json!({"default_sources": ["Work_1"]}))
        .expect_err("an unusable name");
    assert_eq!(error.key(), Some("default_sources"));
}

// ---------------------------------------------------------------------------
// Resolution: the registry, and the check that runs before a source is built.
// ---------------------------------------------------------------------------

/// A configuration naming one source of `kind` over `block`.
fn one_source(kind: &str, block: Value) -> Config {
    Config::from_document(json!({"sources": {"work": {"plugin": kind, "config": block}}}))
        .expect("a configuration")
}

/// A resolver with nothing in it, for sources that need no credential.
fn no_secrets() -> Secrets {
    Secrets::load(Environment::default()).expect("nothing to read")
}

#[test]
fn every_registered_plugin_declares_a_schema_that_compiles_and_accepts_a_valid_block() {
    for kind in onetaskgraph_core::plugin_kinds() {
        let plugin = plugin_for(kind).expect("the registry names it");
        let block = json!({});
        // `validate_sources` compiles the schema and runs it; a plugin whose schema
        // would not compile panics here rather than in a user's run.
        let config = one_source(plugin.kind(), block);
        let outcome = onetaskgraph_core::validate_sources(&config);
        assert!(
            outcome.is_ok(),
            "{kind} refuses an empty block against its own schema: {outcome:?}"
        );
    }
}

#[test]
fn resolving_builds_the_sources_a_configuration_names_in_name_order() {
    let config = Config::from_document(json!({
        "sources": {
            "work": {"plugin": "in-memory", "config": {}},
            "archive": {"plugin": "in-memory", "config": {}},
        }
    }))
    .expect("a configuration");

    let resolved = resolve(&config, &no_secrets()).expect("both sources build");
    assert_eq!(
        resolved
            .iter()
            .map(|source| source.name.as_str())
            .collect::<Vec<_>>(),
        vec!["archive", "work"]
    );
    assert_eq!(resolved[0].kind, "in-memory");
    assert_eq!(resolved[0].source.kind(), "in-memory");
}

#[test]
fn a_plugin_this_build_does_not_have_is_refused_by_the_key_that_names_it() {
    let error = onetaskgraph_core::validate_sources(&one_source("jira", json!({})))
        .expect_err("no such plugin");
    assert_eq!(error.key(), Some("sources.work.plugin"));
    assert!(error.to_string().contains("in-memory"), "{error}");
}

#[test]
fn a_blocks_own_field_is_checked_against_the_plugins_schema_before_the_source_is_built() {
    // `linear`'s factory is registered but its source is not written yet, so *every*
    // call to its `build` fails with the plugin's own message. A mistyped field must
    // therefore be reported as a mistyped field — which it can only be if the check
    // ran first.
    let reached_build = resolve(&one_source("linear", json!({})), &no_secrets())
        .expect_err("the linear source is not written yet");
    assert!(
        reached_build.to_string().contains("not implemented yet"),
        "a valid block reaches the factory: {reached_build}"
    );

    let refused = resolve(
        &one_source("linear", json!({"api_key_env": 7})),
        &no_secrets(),
    )
    .expect_err("a field this plugin does not declare");
    assert_eq!(refused.key(), Some("sources.work.config.api_key_env"));
    assert!(
        !refused.to_string().contains("not implemented yet"),
        "the block is checked before the factory is called: {refused}"
    );
}

#[test]
fn a_block_a_plugin_refuses_names_the_source_it_belongs_to() {
    let config = one_source(
        "in-memory",
        json!({"tasks": [{"id": "T-1", "title": "one",
        "status": {"category": "todo", "name": "Todo"}, "labels": [], "project": "P-9"}]}),
    );

    let error = resolve(&config, &no_secrets()).expect_err("a task under a project that is absent");
    assert_eq!(error.key(), Some("sources.work"));
    assert!(error.to_string().contains("P-9"), "{error}");
}

// ---------------------------------------------------------------------------
// Loading: the whole stack, over a real host.
// ---------------------------------------------------------------------------

#[test]
fn loading_layers_the_documents_the_environment_and_the_flags_in_that_order() {
    let host = Host::new();
    host.write(
        &format!("home/{USER_DOCUMENT_RELATIVE_PATH}"),
        "page_size: 11\noutput: json\n",
    );
    host.write(
        &format!("project/{PROJECT_DOCUMENT_NAME}"),
        "page_size: 25\nsources:\n  work:\n    plugin: in-memory\n",
    );

    let mut pairs: Vec<(String, String)> = vec![
        (
            "XDG_CONFIG_HOME".to_owned(),
            host.root.path().join("home").to_string_lossy().to_string(),
        ),
        ("ONETASKGRAPH_PAGE_SIZE".to_owned(), "70".to_owned()),
    ];
    pairs.push((
        "ONETASKGRAPH_SOURCES__WORK__CONFIG__CAPABILITIES__MAX_PAGE_SIZE".to_owned(),
        "5".to_owned(),
    ));

    let loaded = config::load(
        &host.root.path().join("project/deep"),
        &Environment::from_pairs(pairs),
        &Layer::new(vec![flag("page_size", json!(9))]),
    )
    .expect("the stack loads");

    assert_eq!(loaded.config.page_size.get(), 9, "the flag is on top");
    assert_eq!(
        loaded.config.output,
        OutputFormat::Json,
        "a setting only the user-level document mentions still lands"
    );

    let origins: BTreeMap<String, String> = loaded
        .effective
        .settings
        .iter()
        .map(|setting| (setting.key.to_string(), setting.origin.to_string()))
        .collect();
    assert_eq!(origins["page_size"], "flag --set page_size");
    assert_eq!(
        origins["sources.work.config.capabilities.max_page_size"],
        "environment ONETASKGRAPH_SOURCES__WORK__CONFIG__CAPABILITIES__MAX_PAGE_SIZE"
    );
    assert!(origins["sources.work.plugin"].starts_with("file "));
    assert_eq!(origins["default_sources"], "default");
}

#[test]
fn a_document_that_will_not_parse_names_the_file_and_the_position() {
    let host = Host::new();
    host.write(
        &format!("project/{PROJECT_DOCUMENT_NAME}"),
        "page_size: [\n",
    );

    let error = config::load(
        &host.root.path().join("project"),
        &host.environment(),
        &Layer::default(),
    )
    .expect_err("a malformed document");

    assert!(matches!(error, ConfigError::Syntax { .. }), "{error}");
    assert!(error.to_string().contains(PROJECT_DOCUMENT_NAME), "{error}");
}

#[test]
fn the_effective_configuration_renders_a_line_per_setting_naming_its_layer() {
    let host = Host::new();
    host.write(
        &format!("project/{PROJECT_DOCUMENT_NAME}"),
        "page_size: 25\n",
    );
    host.write(
        &format!("home/{SECRETS_RELATIVE_PATH}"),
        "LINEAR_API_KEY=x\n",
    );

    let loaded = config::load(
        &host.root.path().join("project"),
        &host.environment(),
        &Layer::default(),
    )
    .expect("the stack loads");

    let rendered = loaded.effective.render_text();
    assert!(rendered.contains("page_size"), "{rendered}");
    assert!(rendered.contains(PROJECT_DOCUMENT_NAME), "{rendered}");
    assert!(rendered.contains("output"), "{rendered}");
    assert!(rendered.contains("default"), "{rendered}");
    assert!(
        rendered.contains("LINEAR_API_KEY  resolved from the secrets file"),
        "{rendered}"
    );
    assert!(!rendered.contains("=x"), "no value reaches the rendering");
}

#[test]
fn the_effective_configuration_says_so_when_there_is_no_credentials_file_to_look_in() {
    let host = Host::new();
    let loaded = config::load(
        &host.root.path().join("project"),
        &host.environment(),
        &Layer::default(),
    )
    .expect("the stack loads");

    assert!(
        loaded
            .effective
            .render_text()
            .contains("(it defines no variables, or is not there)")
    );
}

#[test]
fn a_host_with_no_configuration_home_still_loads() {
    let host = Host::new();
    let loaded = config::load(
        &host.root.path().join("project"),
        &Environment::default(),
        &Layer::default(),
    )
    .expect("nothing configured is a configuration");

    assert_eq!(loaded.config.page_size.get(), 50);
    assert!(
        loaded
            .effective
            .render_text()
            .contains("neither XDG_CONFIG_HOME nor HOME is set")
    );
}
