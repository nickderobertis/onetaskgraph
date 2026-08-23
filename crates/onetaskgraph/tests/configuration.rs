//! The configuration journeys, driven the way a user drives them.
//!
//! Journeys 16, 17 and 18 of `AGENTS.md`, plus the refusals journey 22 owes for a
//! malformed configuration. Every one of them spawns the compiled binary as a
//! subprocess against real files in a temporary directory and a real process
//! environment, and asserts on the exit code and on what the binary wrote. Nothing
//! here stands in for the filesystem or for the environment: those *are* the layer
//! under test.

mod common;

use std::process::Output;

use common::{ONE_SOURCE, Sandbox, stderr, stdout};
use serde_json::Value;

/// The whole `config show --json` document.
fn shown(output: &Output) -> Value {
    serde_json::from_str(&stdout(output)).expect("`config show --json` emits one JSON document")
}

/// One setting from a `config show --json` document.
fn setting<'a>(shown: &'a Value, key: &str) -> &'a Value {
    shown["settings"]
        .as_array()
        .expect("settings is a list")
        .iter()
        .find(|setting| setting["key"] == key)
        .unwrap_or_else(|| panic!("{key} is reported; the document was {shown:#}"))
}

// ---------------------------------------------------------------------------
// Journey 16: configuration precedence, and the verb that names the layer.
// ---------------------------------------------------------------------------

#[test]
fn the_file_layer_alone_sets_a_setting_and_the_verb_names_the_file() {
    let sandbox = Sandbox::new();
    let document = sandbox.project_document("page_size: 25\n");

    let output = sandbox
        .command()
        .args(["config", "show", "--json"])
        .assert()
        .success()
        .get_output()
        .clone();

    let page_size = setting(&shown(&output), "page_size").clone();
    assert_eq!(page_size["value"], 25);
    assert_eq!(page_size["origin"]["layer"], "file");
    assert_eq!(
        page_size["origin"]["path"],
        document.to_string_lossy().to_string()
    );
}

#[test]
fn the_environment_layer_alone_sets_a_setting_and_the_verb_names_the_variable() {
    let sandbox = Sandbox::new();

    let output = sandbox
        .command()
        .env("ONETASKGRAPH_PAGE_SIZE", "70")
        .args(["config", "show", "--json"])
        .assert()
        .success()
        .get_output()
        .clone();

    let page_size = setting(&shown(&output), "page_size").clone();
    assert_eq!(page_size["value"], 70);
    assert_eq!(page_size["origin"]["layer"], "environment");
    assert_eq!(page_size["origin"]["variable"], "ONETASKGRAPH_PAGE_SIZE");
}

#[test]
fn the_flag_layer_alone_sets_a_setting_and_the_verb_names_the_flag() {
    let sandbox = Sandbox::new();

    let output = sandbox
        .command()
        .args(["config", "show", "--json", "--page-size", "9"])
        .assert()
        .success()
        .get_output()
        .clone();

    let page_size = setting(&shown(&output), "page_size").clone();
    assert_eq!(page_size["value"], 9);
    assert_eq!(page_size["origin"]["layer"], "flag");
    assert_eq!(page_size["origin"]["flag"], "--page-size");
}

#[test]
fn the_environment_beats_the_file() {
    let sandbox = Sandbox::new();
    sandbox.project_document("page_size: 25\n");

    let output = sandbox
        .command()
        .env("ONETASKGRAPH_PAGE_SIZE", "70")
        .args(["config", "show", "--json"])
        .assert()
        .success()
        .get_output()
        .clone();

    let page_size = setting(&shown(&output), "page_size").clone();
    assert_eq!(page_size["value"], 70);
    assert_eq!(page_size["origin"]["variable"], "ONETASKGRAPH_PAGE_SIZE");
}

#[test]
fn a_flag_beats_the_environment() {
    let sandbox = Sandbox::new();

    let output = sandbox
        .command()
        .env("ONETASKGRAPH_PAGE_SIZE", "70")
        .args(["config", "show", "--json", "--page-size", "9"])
        .assert()
        .success()
        .get_output()
        .clone();

    let page_size = setting(&shown(&output), "page_size").clone();
    assert_eq!(page_size["value"], 9);
    assert_eq!(page_size["origin"]["flag"], "--page-size");
}

#[test]
fn a_flag_beats_the_file() {
    let sandbox = Sandbox::new();
    sandbox.project_document("page_size: 25\n");

    let output = sandbox
        .command()
        .args(["config", "show", "--json", "--page-size", "9"])
        .assert()
        .success()
        .get_output()
        .clone();

    let page_size = setting(&shown(&output), "page_size").clone();
    assert_eq!(page_size["value"], 9);
    assert_eq!(page_size["origin"]["flag"], "--page-size");
}

#[test]
fn all_three_layers_at_once_leave_the_flag_on_top_and_each_other_setting_with_its_own_layer() {
    let sandbox = Sandbox::new();
    let document = sandbox.project_document(
        "page_size: 25\nsources:\n  work:\n    plugin: in-memory\n  notes:\n    plugin: in-memory\n",
    );

    let output = sandbox
        .command()
        .env("ONETASKGRAPH_PAGE_SIZE", "70")
        .env("ONETASKGRAPH_DEFAULT_SOURCES", "work,notes")
        .args(["config", "show", "--json", "--page-size", "9"])
        .assert()
        .success()
        .get_output()
        .clone();

    let shown = shown(&output);
    assert_eq!(setting(&shown, "page_size")["value"], 9);
    assert_eq!(
        setting(&shown, "page_size")["origin"]["flag"],
        "--page-size"
    );
    assert_eq!(
        setting(&shown, "default_sources")["origin"]["variable"],
        "ONETASKGRAPH_DEFAULT_SOURCES",
        "the layer that set a setting is the layer reported for it, per setting"
    );
    assert_eq!(
        setting(&shown, "sources.work.plugin")["origin"]["path"],
        document.to_string_lossy().to_string(),
        "a setting no higher layer touched still comes from the document that set it"
    );
}

#[test]
fn the_project_document_layers_over_the_user_level_one() {
    let sandbox = Sandbox::new();
    let user = sandbox.user_document("page_size: 11\nsources:\n  work:\n    plugin: in-memory\n");
    let project = sandbox.project_document("page_size: 25\n");

    let output = sandbox
        .command()
        .args(["config", "show", "--json"])
        .assert()
        .success()
        .get_output()
        .clone();

    let shown = shown(&output);
    assert_eq!(setting(&shown, "page_size")["value"], 25);
    assert_eq!(
        setting(&shown, "page_size")["origin"]["path"],
        project.to_string_lossy().to_string()
    );
    assert_eq!(
        setting(&shown, "sources.work.plugin")["origin"]["path"],
        user.to_string_lossy().to_string(),
        "a source the project document does not mention still comes from the user's"
    );
}

#[test]
fn the_document_is_discovered_upward_from_the_working_directory() {
    let sandbox = Sandbox::new();
    let document = sandbox.project_document("page_size: 25\n");
    let deep = sandbox.subdirectory("crates/onetaskgraph/src");

    let output = sandbox
        .command_in(&deep)
        .args(["config", "show", "--json"])
        .assert()
        .success()
        .get_output()
        .clone();

    assert_eq!(
        setting(&shown(&output), "page_size")["origin"]["path"],
        document.to_string_lossy().to_string()
    );
}

#[test]
fn a_setting_nothing_sets_is_still_reported_with_the_value_the_run_will_use() {
    let sandbox = Sandbox::new();

    let output = sandbox
        .command()
        .args(["config", "show", "--json"])
        .assert()
        .success()
        .get_output()
        .clone();

    let page_size = setting(&shown(&output), "page_size").clone();
    assert_eq!(page_size["origin"]["layer"], "default");
    assert_eq!(page_size["value"], 50);
}

#[test]
fn the_text_rendering_names_the_layer_beside_every_setting() {
    let sandbox = Sandbox::new();
    sandbox.project_document("page_size: 25\n");

    let output = sandbox
        .command()
        .env("ONETASKGRAPH_OUTPUT", "text")
        .args(["config", "show"])
        .assert()
        .success()
        .get_output()
        .clone();

    let rendered = stdout(&output);
    assert!(
        rendered.contains("page_size") && rendered.contains("onetaskgraph.yaml"),
        "the file that set page_size is named: {rendered}"
    );
    assert!(
        rendered.contains("environment ONETASKGRAPH_OUTPUT"),
        "the variable that set output is named: {rendered}"
    );
    assert!(
        rendered.contains("default"),
        "a setting nothing set is named as a default: {rendered}"
    );
}

// ---------------------------------------------------------------------------
// Journey 17: a field of one named source, set at each of the three layers.
// ---------------------------------------------------------------------------

/// The setting a named source's own field lives at.
const SOURCE_FIELD: &str = "sources.work.config.capabilities.max_page_size";

#[test]
fn a_named_sources_own_field_is_set_by_the_file() {
    let sandbox = Sandbox::new();
    let document = sandbox.project_document(ONE_SOURCE);

    let output = sandbox
        .command()
        .args(["config", "show", "--json"])
        .assert()
        .success()
        .get_output()
        .clone();

    let field = setting(&shown(&output), SOURCE_FIELD).clone();
    assert_eq!(field["value"], 20);
    assert_eq!(
        field["origin"]["path"],
        document.to_string_lossy().to_string()
    );
}

#[test]
fn a_named_sources_own_field_is_set_by_the_environment_over_the_file() {
    let sandbox = Sandbox::new();
    sandbox.project_document(ONE_SOURCE);

    let output = sandbox
        .command()
        .env(
            "ONETASKGRAPH_SOURCES__WORK__CONFIG__CAPABILITIES__MAX_PAGE_SIZE",
            "7",
        )
        .args(["config", "show", "--json"])
        .assert()
        .success()
        .get_output()
        .clone();

    let field = setting(&shown(&output), SOURCE_FIELD).clone();
    assert_eq!(field["value"], 7);
    assert_eq!(
        field["origin"]["variable"],
        "ONETASKGRAPH_SOURCES__WORK__CONFIG__CAPABILITIES__MAX_PAGE_SIZE"
    );
}

#[test]
fn a_named_sources_own_field_is_set_by_a_flag_over_the_environment_and_the_file() {
    let sandbox = Sandbox::new();
    sandbox.project_document(ONE_SOURCE);

    let output = sandbox
        .command()
        .env(
            "ONETASKGRAPH_SOURCES__WORK__CONFIG__CAPABILITIES__MAX_PAGE_SIZE",
            "7",
        )
        .args([
            "config",
            "show",
            "--json",
            "--set",
            "sources.work.config.capabilities.max_page_size=3",
        ])
        .assert()
        .success()
        .get_output()
        .clone();

    let field = setting(&shown(&output), SOURCE_FIELD).clone();
    assert_eq!(field["value"], 3);
    assert_eq!(field["origin"]["flag"], format!("--set {SOURCE_FIELD}"));
}

#[test]
fn a_whole_named_source_is_configured_from_the_environment_alone() {
    let sandbox = Sandbox::new();

    let output = sandbox
        .command()
        .env("ONETASKGRAPH_SOURCES__GH_MAIN__PLUGIN", "github-projects")
        .args(["config", "show", "--json"])
        .assert()
        .success()
        .get_output()
        .clone();

    let plugin = setting(&shown(&output), "sources.gh-main.plugin").clone();
    assert_eq!(
        plugin["value"], "github-projects",
        "an underscore in the variable is a hyphen in the source name"
    );
}

#[test]
fn each_worked_environment_example_sets_the_setting_it_claims_to_set() {
    let sandbox = Sandbox::new();
    sandbox.project_document(ONE_SOURCE);

    let output = sandbox
        .command()
        .env("ONETASKGRAPH_PAGE_SIZE", "100")
        .env("ONETASKGRAPH_DEFAULT_SOURCES", "work,gh-main")
        .env(
            "ONETASKGRAPH_SOURCES__WORK__CONFIG__CAPABILITIES__MAX_PAGE_SIZE",
            "42",
        )
        .env("ONETASKGRAPH_SOURCES__GH_MAIN__PLUGIN", "github-projects")
        .args(["config", "show", "--json"])
        .assert()
        .success()
        .get_output()
        .clone();

    let shown = shown(&output);
    assert_eq!(setting(&shown, "page_size")["value"], 100);
    assert_eq!(
        setting(&shown, "default_sources")["value"],
        serde_json::json!(["work", "gh-main"]),
        "a list is comma-separated"
    );
    assert_eq!(setting(&shown, SOURCE_FIELD)["value"], 42);
    assert_eq!(
        setting(&shown, "sources.gh-main.plugin")["value"],
        "github-projects"
    );
}

// ---------------------------------------------------------------------------
// Journey 18: the credentials file supplies, and defers.
// ---------------------------------------------------------------------------

/// A value distinctive enough that finding it anywhere is unambiguous.
const PLANTED: &str = "lin_api_PLANTED-CANARY-8f21c0";

#[test]
fn the_credentials_file_supplies_a_variable_the_process_environment_does_not_define() {
    let sandbox = Sandbox::new();
    sandbox.secrets_file(&format!("# credentials\nLINEAR_API_KEY={PLANTED}\n"));

    let output = sandbox
        .command()
        .env_remove("LINEAR_API_KEY")
        .args(["config", "show", "--json"])
        .assert()
        .success()
        .get_output()
        .clone();

    let shown = shown(&output);
    assert_eq!(
        shown["secrets"]["variables"],
        serde_json::json!([
            { "variable": "LINEAR_API_KEY", "resolved_from": "secrets-file" }
        ])
    );
}

#[test]
fn the_credentials_file_defers_to_a_variable_the_process_environment_already_defines() {
    let sandbox = Sandbox::new();
    sandbox.secrets_file(&format!("LINEAR_API_KEY={PLANTED}\n"));

    let output = sandbox
        .command()
        .env("LINEAR_API_KEY", "exported-by-the-shell")
        .args(["config", "show", "--json"])
        .assert()
        .success()
        .get_output()
        .clone();

    let shown = shown(&output);
    assert_eq!(
        shown["secrets"]["variables"][0]["resolved_from"], "environment",
        "an explicitly exported variable wins: {shown:#}"
    );
}

#[test]
fn a_missing_credentials_file_is_not_an_error() {
    let sandbox = Sandbox::new();

    let output = sandbox
        .command()
        .args(["config", "show", "--json"])
        .assert()
        .success()
        .get_output()
        .clone();

    assert_eq!(
        shown(&output)["secrets"]["variables"],
        serde_json::json!([])
    );
}

#[test]
fn the_override_variable_moves_the_credentials_file() {
    let sandbox = Sandbox::new();
    sandbox.secrets_file("LINEAR_API_KEY=from-the-default-path\n");
    let elsewhere = sandbox.project().join("elsewhere.env");
    std::fs::write(&elsewhere, format!("GH_PROJECTS_TOKEN={PLANTED}\n")).expect("written");

    let output = sandbox
        .command()
        .env("ONETASKGRAPH_SECRETS_FILE", &elsewhere)
        .env_remove("GH_PROJECTS_TOKEN")
        .args(["config", "show", "--json"])
        .assert()
        .success()
        .get_output()
        .clone();

    let shown = shown(&output);
    assert_eq!(
        shown["secrets"]["path"],
        elsewhere.to_string_lossy().to_string()
    );
    assert_eq!(
        shown["secrets"]["variables"],
        serde_json::json!([
            { "variable": "GH_PROJECTS_TOKEN", "resolved_from": "secrets-file" }
        ]),
        "the override replaces the default path rather than adding to it"
    );
}

#[test]
fn the_override_variable_is_not_mistaken_for_a_setting() {
    let sandbox = Sandbox::new();
    let elsewhere = sandbox.project().join("elsewhere.env");
    std::fs::write(&elsewhere, "").expect("written");

    let output = sandbox
        .command()
        .env("ONETASKGRAPH_SECRETS_FILE", &elsewhere)
        .args(["config", "show", "--json"])
        .assert()
        .success()
        .get_output()
        .clone();

    assert!(
        !stdout(&output).contains("secrets_file\""),
        "ONETASKGRAPH_SECRETS_FILE is the credentials path, not a setting called \
         `secrets_file`"
    );
}

#[test]
fn a_planted_credential_reaches_no_output_of_any_verb() {
    let sandbox = Sandbox::new();
    sandbox.project_document(ONE_SOURCE);
    sandbox.secrets_file(&format!(
        "LINEAR_API_KEY={PLANTED}\nGH_PROJECTS_TOKEN={PLANTED}\n"
    ));

    // Every verb this binary answers, and a failing invocation, because an error path
    // that renders more state than a success path is exactly how one of these leaks.
    let invocations: Vec<Vec<&str>> = vec![
        vec!["--help"],
        vec!["--version"],
        vec!["schema"],
        vec!["config", "show"],
        vec!["config", "show", "--json"],
        vec!["config", "show", "--set", "sources.work.plugin=nope"],
        vec!["config", "show", "--set", "sources.work.config.taks=1"],
    ];

    for arguments in invocations {
        let output = sandbox
            .command()
            .args(&arguments)
            .output()
            .expect("the binary runs");
        for (stream, text) in [("stdout", stdout(&output)), ("stderr", stderr(&output))] {
            assert!(
                !text.contains(PLANTED),
                "`onetaskgraph {}` put a credential on {stream}",
                arguments.join(" ")
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Journey 22: a configuration this product will not run on says why, and what next.
// ---------------------------------------------------------------------------

/// Run `config show` over `sandbox`, expect a refusal, and return what it said.
fn refusal(sandbox: &Sandbox, arguments: &[&str]) -> String {
    let output = sandbox
        .command()
        .args(["config", "show"])
        .args(arguments)
        .assert()
        .failure()
        .get_output()
        .clone();
    assert!(
        stdout(&output).is_empty(),
        "a refusal writes nothing to stdout"
    );
    let message = stderr(&output);
    assert!(
        message.contains("next:"),
        "a refusal suggests a next action: {message}"
    );
    message
}

#[test]
fn an_unknown_field_is_refused_by_name() {
    let sandbox = Sandbox::new();
    sandbox.project_document("page_sise: 25\n");

    let message = refusal(&sandbox, &[]);
    assert!(message.contains("page_sise"), "{message}");
}

#[test]
fn a_bad_value_is_refused_by_name() {
    let sandbox = Sandbox::new();
    sandbox.project_document("page_size: nowhere-near-a-number\n");

    let message = refusal(&sandbox, &[]);
    assert!(message.contains("page_size"), "{message}");
}

#[test]
fn an_unknown_plugin_name_is_refused_by_name_and_the_known_ones_are_listed() {
    let sandbox = Sandbox::new();
    sandbox.project_document("sources:\n  work:\n    plugin: jira\n");

    let message = refusal(&sandbox, &[]);
    assert!(message.contains("sources.work.plugin"), "{message}");
    assert!(message.contains("in-memory"), "{message}");
    assert!(
        message.contains("linear") && message.contains("github-projects"),
        "a plugin whose source is not written yet is still a name this build knows: \
         {message}"
    );
}

#[test]
fn a_source_name_that_breaks_the_pattern_is_refused_by_name() {
    let sandbox = Sandbox::new();
    sandbox.project_document("sources:\n  Work_1:\n    plugin: in-memory\n");

    let message = refusal(&sandbox, &[]);
    assert!(message.contains("sources.Work_1"), "{message}");
    assert!(message.contains("^[a-z0-9][a-z0-9-]*$"), "{message}");
}

#[test]
fn a_mistyped_field_of_a_named_source_is_refused_at_the_boundary_rather_than_ignored() {
    let sandbox = Sandbox::new();
    sandbox.project_document(
        "sources:\n  work:\n    plugin: in-memory\n    config:\n      taks: []\n",
    );

    let message = refusal(&sandbox, &[]);
    assert!(
        message.contains("sources.work.config.taks"),
        "the offending field is named, not just the block it sits in: {message}"
    );
    assert!(message.contains("in-memory"), "{message}");
}

#[test]
fn a_default_source_naming_nothing_configured_is_refused() {
    let sandbox = Sandbox::new();
    sandbox.project_document("default_sources: [nope]\n");

    let message = refusal(&sandbox, &[]);
    assert!(message.contains("default_sources"), "{message}");
    assert!(message.contains("nope"), "{message}");
}

#[test]
fn a_malformed_document_is_refused_with_the_position_of_the_problem() {
    let sandbox = Sandbox::new();
    let document = sandbox.project_document("page_size: [\n");

    let message = refusal(&sandbox, &[]);
    assert!(
        message.contains(&document.to_string_lossy().to_string()),
        "{message}"
    );
    assert!(message.contains("line"), "{message}");
}

#[test]
fn a_document_that_is_not_a_mapping_is_refused_rather_than_read_as_unset() {
    let sandbox = Sandbox::new();
    sandbox.project_document("- page_size: 25\n");

    let message = refusal(&sandbox, &[]);
    assert!(message.contains("mapping"), "{message}");
}

#[test]
fn a_set_argument_that_is_not_an_assignment_is_refused() {
    let sandbox = Sandbox::new();

    let message = refusal(&sandbox, &["--set", "page_size"]);
    assert!(message.contains("--set page_size"), "{message}");
}

#[test]
fn a_set_argument_addressing_no_setting_is_refused() {
    let sandbox = Sandbox::new();

    let message = refusal(&sandbox, &["--set", "sources..plugin=in-memory"]);
    assert!(message.contains("--set"), "{message}");
}

#[test]
fn a_credentials_file_line_that_is_not_an_assignment_is_refused_without_quoting_it() {
    let sandbox = Sandbox::new();
    let path = sandbox.secrets_file(&format!("LINEAR_API_KEY={PLANTED}\nthis is not a line\n"));

    let message = refusal(&sandbox, &[]);
    assert!(
        message.contains(&format!("{}:2", path.display())),
        "{message}"
    );
    assert!(
        !message.contains(PLANTED),
        "a refusal about one line does not print another line's value"
    );
}

// ---------------------------------------------------------------------------
// Journey 21: the machine-readable output has a root in the schema the binary emits.
// ---------------------------------------------------------------------------

#[test]
fn the_machine_readable_output_matches_a_root_of_the_schema_this_binary_emits() {
    let sandbox = Sandbox::new();
    sandbox.project_document(ONE_SOURCE);

    let bundle: Value = serde_json::from_str(&stdout(
        &sandbox
            .command()
            .arg("schema")
            .assert()
            .success()
            .get_output()
            .clone(),
    ))
    .expect("the schema bundle is JSON");

    let root = &bundle["roots"]["EffectiveConfig"];
    assert!(
        root.is_object(),
        "`config show --json` emits an EffectiveConfig, so the bundle carries that root"
    );

    let output = sandbox
        .command()
        .args(["config", "show", "--json"])
        .assert()
        .success()
        .get_output()
        .clone();
    let shown = shown(&output);

    let required: Vec<&str> = root["required"]
        .as_array()
        .expect("the root names its required properties")
        .iter()
        .map(|name| name.as_str().expect("a property name"))
        .collect();
    for property in required {
        assert!(
            shown.get(property).is_some(),
            "the emitted document carries the schema's required property {property}: \
             {shown:#}"
        );
    }
}
