//! The configuration journeys, driven the way a user drives them.
//!
//! Journeys 16 (precedence and the verb that names the layer), 17 (a named source's
//! own field at each of the three layers) and 18 (the credentials file supplying and
//! deferring) of `AGENTS.md`, plus the part of 21 the machine-readable configuration
//! output owes and the refusals 22 owes for a malformed configuration. Every one of
//! them spawns the compiled binary as a subprocess against real files in a temporary
//! directory and a real process environment, and asserts on the exit code and on what
//! the binary wrote. Nothing here stands in for the filesystem or for the
//! environment: those *are* the layer under test.

mod common;

use std::process::Output;

use common::{SOURCE_BOUNDARIES, Sandbox, SourceBoundary, one_source, stderr, stdout};
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
    for boundary in SOURCE_BOUNDARIES {
        let sandbox = Sandbox::new();
        let document = sandbox.project_document(
            &serde_json::to_string(&serde_json::json!({
                "page_size": 25,
                "sources": {
                    "work": boundary.source("in-memory", serde_json::json!({})),
                    "notes": boundary.source("in-memory", serde_json::json!({})),
                }
            }))
            .expect("a document"),
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
            setting(
                &shown,
                match boundary {
                    SourceBoundary::Direct => "sources.work.plugin",
                    SourceBoundary::Subprocess => "sources.work.config.settings.kind",
                },
            )["origin"]["path"],
            document.to_string_lossy().to_string(),
            "a setting no higher layer touched still comes from the document that set it"
        );
    }
}

#[test]
fn the_project_document_layers_over_the_user_level_one() {
    for boundary in SOURCE_BOUNDARIES {
        let sandbox = Sandbox::new();
        let user = sandbox.user_document(
            &serde_json::to_string(&serde_json::json!({
                "page_size": 11,
                "sources": {"work": boundary.source("in-memory", serde_json::json!({}))}
            }))
            .expect("a user document"),
        );
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
            setting(
                &shown,
                match boundary {
                    SourceBoundary::Direct => "sources.work.plugin",
                    SourceBoundary::Subprocess => "sources.work.config.settings.kind",
                },
            )["origin"]["path"],
            user.to_string_lossy().to_string(),
            "a source the project document does not mention still comes from the user's"
        );
    }
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

#[test]
fn the_named_flags_reach_the_settings_they_are_shorthand_for() {
    for boundary in SOURCE_BOUNDARIES {
        let sandbox = Sandbox::new();
        sandbox.project_document(
            &serde_json::to_string(&serde_json::json!({
                "sources": {
                    "work": boundary.source("in-memory", serde_json::json!({})),
                    "notes": boundary.source("in-memory", serde_json::json!({})),
                }
            }))
            .expect("a document"),
        );

        let output = sandbox
            .command()
            .args([
                "config",
                "show",
                "--output",
                "json",
                "--default-sources",
                "notes,work",
            ])
            .assert()
            .success()
            .get_output()
            .clone();

        let shown = shown(&output);
        assert_eq!(
            setting(&shown, "output")["origin"]["flag"],
            "--output",
            "--output json renders JSON and says the flag it came from"
        );
        let selected = setting(&shown, "default_sources").clone();
        assert_eq!(selected["value"], serde_json::json!(["notes", "work"]));
        assert_eq!(selected["origin"]["flag"], "--default-sources");
    }
}

#[test]
fn an_output_format_this_build_does_not_have_is_refused_with_the_ones_it_does() {
    let sandbox = Sandbox::new();

    let output = sandbox
        .command()
        .args(["config", "show", "--output", "yaml"])
        .assert()
        .failure()
        .get_output()
        .clone();

    // Clap's own code for a malformed invocation: `--output` takes one of a closed set,
    // so this never reaches the configuration layer at all.
    assert_eq!(output.status.code(), Some(2));
    let message = stderr(&output);
    assert!(
        message.contains("yaml"),
        "the refusal quotes what was asked for: {message}"
    );
    assert!(
        message.contains("text") && message.contains("json"),
        "the refusal lists the formats this build has: {message}"
    );
}

#[test]
fn a_spelled_out_set_beats_the_json_shorthand_within_the_flag_layer() {
    let sandbox = Sandbox::new();

    let output = sandbox
        .command()
        .args(["config", "show", "--json", "--set", "output=text"])
        .assert()
        .success()
        .get_output()
        .clone();

    let rendered = stdout(&output);
    assert!(
        rendered.contains("output") && !rendered.starts_with('{'),
        "the spelled-out setting wins and the table is rendered: {rendered}"
    );
    assert!(
        rendered.contains("flag --set output"),
        "the layer named is the flag that actually set it: {rendered}"
    );
}

#[test]
fn asking_for_a_format_and_the_json_shorthand_at_once_is_refused_as_a_bad_invocation() {
    let sandbox = Sandbox::new();

    let output = sandbox
        .command()
        .args(["config", "show", "--output", "text", "--json"])
        .assert()
        .failure()
        .get_output()
        .clone();

    // Clap's own code for a malformed invocation, distinct from the `1` a command
    // that ran and failed exits with — a caller can tell the two apart.
    assert_eq!(output.status.code(), Some(2));
    let message = stderr(&output);
    assert!(
        message.contains("--output") && message.contains("--json"),
        "the refusal names both flags: {message}"
    );
    assert!(
        stdout(&output).is_empty(),
        "a refused invocation writes nothing to stdout"
    );
}

fn source_document(boundary: SourceBoundary, max_page_size: u32) -> String {
    serde_json::to_string(&serde_json::json!({
        "sources": {"work": boundary.source("in-memory", serde_json::json!({
            "capabilities": {"max_page_size": max_page_size}
        }))}
    }))
    .expect("a source document")
}

#[test]
fn a_malformed_configuration_flag_is_refused_on_a_verb_that_does_not_use_it() {
    let sandbox = Sandbox::new();

    // `schema` renders a static bundle, but the flags are global and parse on it, so a
    // flag written there has to be refused rather than accepted and dropped.
    let message = mistyped(&sandbox, &["schema", "--set", "page_size"]);
    assert!(
        message.contains("--set page_size: that is not an assignment"),
        "the refusal names the flag: {message}"
    );
}

#[test]
fn a_page_size_of_zero_is_refused_wherever_it_is_written() {
    for boundary in SOURCE_BOUNDARIES {
        let sandbox = Sandbox::new();
        sandbox.project_document(&one_source(boundary));

        for verb in [["schema"], ["config"]] {
            let mut command = sandbox.command();
            command.args(verb);
            if verb == ["config"] {
                command.arg("show");
            }
            let output = command
                .args(["--page-size", "0"])
                .assert()
                .failure()
                .get_output()
                .clone();

            assert!(
                stderr(&output).contains("--page-size"),
                "`onetaskgraph {} --page-size 0` names the flag it refused: {}",
                verb[0],
                stderr(&output)
            );
        }
    }
}

#[test]
fn a_well_formed_configuration_flag_leaves_the_bundle_this_verb_emits_alone() {
    let sandbox = Sandbox::new();

    let output = sandbox
        .command()
        .args(["schema", "--set", "page_size=10", "--page-size", "7"])
        .assert()
        .success()
        .get_output()
        .clone();

    let bundle: Value =
        serde_json::from_str(&stdout(&output)).expect("the schema bundle is still JSON");
    assert!(
        bundle["roots"]["EffectiveConfig"].is_object(),
        "a flag this verb does not read does not change what it emits"
    );
}

/// Every layer is validated on every verb, including the verbs that do not use it.
///
/// `schema` renders a static bundle, so nothing it emits depends on the configuration —
/// which is exactly why a setting written for it could be accepted and dropped without
/// anything looking wrong. A setting that addresses nothing this build has is a mistake
/// wherever it was written, so it is refused here as it is on `config show`: at the
/// flag layer a user typed on this invocation, and at the environment layer around it.
#[test]
fn a_setting_that_addresses_nothing_is_refused_on_a_verb_that_does_not_use_it() {
    let sandbox = Sandbox::new();

    let from_flag = sandbox
        .command()
        .args(["schema", "--set", "unknown=value"])
        .assert()
        .failure()
        .get_output()
        .clone();
    assert!(
        stdout(&from_flag).is_empty(),
        "a refusal emits no bundle for a generator to consume"
    );
    assert!(
        stderr(&from_flag).contains("unknown"),
        "the refusal names the setting: {}",
        stderr(&from_flag)
    );

    let from_environment = sandbox
        .command()
        .env("ONETASKGRAPH_UNKNOWN", "value")
        .args(["schema"])
        .assert()
        .failure()
        .get_output()
        .clone();
    assert!(
        stderr(&from_environment).contains("unknown"),
        "the refusal names the setting: {}",
        stderr(&from_environment)
    );
}

/// The other three refusals of the same clause, on that same verb.
///
/// An unknown field is one of four ways a setting can be wrong; a source name that
/// breaks the pattern and a plugin this build does not have are two more, and each is
/// found by a different check. Proving only the first would leave the other two free to
/// pass a verb that does not use the configuration.
#[test]
fn a_source_this_build_cannot_build_is_refused_on_a_verb_that_does_not_use_it() {
    let sandbox = Sandbox::new();

    for (assignment, named) in [
        ("sources.WORK.plugin=in-memory", "sources.WORK"),
        ("sources.work.plugin=nope", "sources.work.plugin"),
    ] {
        let output = sandbox
            .command()
            .args(["schema", "--set", assignment])
            .assert()
            .failure()
            .get_output()
            .clone();
        assert!(
            stderr(&output).contains(named),
            "`--set {assignment}` is refused naming {named}: {}",
            stderr(&output)
        );
    }
}

#[test]
fn a_named_sources_own_field_is_set_by_the_file() {
    for boundary in SOURCE_BOUNDARIES {
        let sandbox = Sandbox::new();
        let document = sandbox.project_document(&source_document(boundary, 20));

        let output = sandbox
            .command()
            .args(["config", "show", "--json"])
            .assert()
            .success()
            .get_output()
            .clone();

        let field = setting(
            &shown(&output),
            &boundary.config_path("capabilities.max_page_size"),
        )
        .clone();
        assert_eq!(field["value"], 20);
        assert_eq!(
            field["origin"]["path"],
            document.to_string_lossy().to_string()
        );
    }
}

#[test]
fn a_named_sources_own_field_is_set_by_the_environment_over_the_file() {
    for boundary in SOURCE_BOUNDARIES {
        let sandbox = Sandbox::new();
        sandbox.project_document(&source_document(boundary, 20));

        let variable = boundary.config_variable("capabilities.max_page_size");

        let output = sandbox
            .command()
            .env(&variable, "7")
            .args(["config", "show", "--json"])
            .assert()
            .success()
            .get_output()
            .clone();

        let field = setting(
            &shown(&output),
            &boundary.config_path("capabilities.max_page_size"),
        )
        .clone();
        assert_eq!(field["value"], 7);
        assert_eq!(field["origin"]["variable"], variable);
    }
}

#[test]
fn a_named_sources_own_field_is_set_by_a_flag_over_the_environment_and_the_file() {
    for boundary in SOURCE_BOUNDARIES {
        let sandbox = Sandbox::new();
        sandbox.project_document(&source_document(boundary, 20));

        let variable = boundary.config_variable("capabilities.max_page_size");
        let field_path = boundary.config_path("capabilities.max_page_size");
        let assignment = format!("{field_path}=3");

        let output = sandbox
            .command()
            .env(&variable, "7")
            .args(["config", "show", "--json", "--set", &assignment])
            .assert()
            .success()
            .get_output()
            .clone();

        let field = setting(&shown(&output), &field_path).clone();
        assert_eq!(field["value"], 3);
        assert_eq!(field["origin"]["flag"], format!("--set {field_path}"));
    }
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
    for boundary in SOURCE_BOUNDARIES {
        let sandbox = Sandbox::new();
        sandbox.project_document(&one_source(boundary));
        let variable = boundary.config_variable("capabilities.max_page_size");

        let output = sandbox
            .command()
            .env("ONETASKGRAPH_PAGE_SIZE", "100")
            .env("ONETASKGRAPH_DEFAULT_SOURCES", "work,gh-main")
            .env(&variable, "42")
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
        assert_eq!(
            setting(&shown, &boundary.config_path("capabilities.max_page_size"))["value"],
            42
        );
        assert_eq!(
            setting(&shown, "sources.gh-main.plugin")["value"],
            "github-projects"
        );
    }
}

/// The worked example above is written against a `root`, which belongs to `local-md` —
/// a plugin whose configuration block is still empty, so nothing here can accept that
/// field yet. What the example claims is not that `root` is valid but *where the variable
/// lands*, so that is what this asserts: the value reaches `sources.work.config.root` and
/// is refused under that name, rather than being dropped or landing on another setting.
#[test]
fn the_worked_example_for_a_named_sources_own_field_reaches_that_field() {
    // Subprocess settings are intentionally opaque to `config show`, so the protocol cannot repeat an inner-schema refusal.
    let boundary = SourceBoundary::Direct;
    let sandbox = Sandbox::new();
    sandbox.project_document(&one_source(boundary));
    let variable = boundary.config_variable("root");

    let output = sandbox
        .command()
        .env(&variable, "/tmp/tasks")
        .args(["config", "show"])
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
        message.contains(&boundary.config_path("root")),
        "the variable reaches the field it names: {message}"
    );
}

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

/// A one-source document whose source is a program of its own, naming one credential.
///
/// The hosted plugin is `in-memory` over a single task, because what this journey is about
/// is the credential reaching a spawned child rather than the work behind it.
fn subprocess_document(secrets: &[&str]) -> String {
    serde_json::to_string(&serde_json::json!({
        "sources": {
            "work": {
                "plugin": "subprocess",
                "config": {
                    "command": env!("CARGO_BIN_EXE_onetaskgraph-source"),
                    "secrets": secrets,
                    "settings": {
                        "kind": "in-memory",
                        "config": {
                            "tasks": [{
                                "id": "T-1",
                                "title": "Alpha",
                                "status": {"category": "todo", "name": "Todo"},
                                "labels": []
                            }]
                        }
                    }
                }
            }
        }
    }))
    .expect("a document")
}

#[test]
fn the_credentials_file_supplies_a_variable_a_source_in_another_process_names() {
    let sandbox = Sandbox::new();
    sandbox.secrets_file(&format!("LINEAR_API_KEY={PLANTED}\n"));
    sandbox.project_document(&subprocess_document(&["LINEAR_API_KEY"]));

    let output = sandbox
        .command()
        .env_remove("LINEAR_API_KEY")
        .args(["task", "list"])
        .assert()
        .success()
        .get_output()
        .clone();

    // The run reached the spawned plugin and got its rows, which it could only do with the
    // credential resolved and forwarded: the source refuses to build without it, as the
    // next journey asserts.
    let listed = String::from_utf8(output.stdout).expect("UTF-8");
    assert!(listed.contains("work:T-1"), "{listed}");
    assert!(
        !listed.contains(PLANTED),
        "a credential is never printed: {listed}"
    );
}

#[test]
fn a_credential_a_source_in_another_process_names_and_nothing_defines_is_refused_by_name() {
    let sandbox = Sandbox::new();
    sandbox.project_document(&subprocess_document(&["LINEAR_API_KEY"]));

    let output = sandbox
        .command()
        .env_remove("LINEAR_API_KEY")
        .args(["task", "list"])
        .assert()
        .failure()
        .get_output()
        .clone();

    let complaint = String::from_utf8(output.stderr).expect("UTF-8");
    assert!(complaint.contains("LINEAR_API_KEY"), "{complaint}");
}

#[test]
fn a_variable_name_a_source_in_another_process_could_never_resolve_is_refused_at_the_field() {
    let sandbox = Sandbox::new();
    sandbox.project_document(&subprocess_document(&["not a variable name"]));

    let output = sandbox
        .command()
        .args(["task", "list"])
        .assert()
        .failure()
        .get_output()
        .clone();

    let complaint = String::from_utf8(output.stderr).expect("UTF-8");
    assert!(
        complaint.contains("secrets") || complaint.contains("not a variable name"),
        "the field that holds the mistake is named: {complaint}"
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
    for boundary in SOURCE_BOUNDARIES {
        let sandbox = Sandbox::new();
        sandbox.project_document(&one_source(boundary));
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
}

/// Expect a refusal over `sandbox` from **every** verb, and return what it said.
///
/// Both verbs, because a configuration that cannot be loaded is a refusal wherever it
/// was written: `config show` reads the configuration and `schema` renders a static
/// bundle without it, so `schema` is the one that would answer anyway if a layer were
/// only validated where it is used. Every refusal below is configuration-shaped rather
/// than verb-shaped, so checking both here holds that for all of them at once — the
/// malformed document, the configuration and credentials files that cannot be read,
/// and each setting a layer got wrong.
fn refusal(sandbox: &Sandbox, arguments: &[&str]) -> String {
    let mut messages = Vec::new();
    for verb in [&["config", "show"][..], &["schema"][..]] {
        let output = sandbox
            .command()
            .args(verb)
            .args(arguments)
            .assert()
            .failure()
            .get_output()
            .clone();
        assert!(
            stdout(&output).is_empty(),
            "`onetaskgraph {}` wrote to stdout while refusing",
            verb.join(" ")
        );
        let message = stderr(&output);
        assert!(
            message.contains("next:"),
            "`onetaskgraph {}` suggests a next action: {message}",
            verb.join(" ")
        );
        messages.push(message);
    }
    let [from_config_show, from_schema] = messages
        .try_into()
        .expect("one message from each of the two verbs");
    assert_eq!(
        from_config_show, from_schema,
        "both verbs refuse the same configuration with the same problem"
    );
    from_config_show
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
    // Invalid source names are refused before source construction, so no protocol capability can repeat this case.
    let sandbox = Sandbox::new();
    sandbox.project_document("sources:\n  Work_1:\n    plugin: in-memory\n");

    let message = refusal(&sandbox, &[]);
    assert!(message.contains("sources.Work_1"), "{message}");
    assert!(message.contains("^[a-z0-9][a-z0-9-]*$"), "{message}");
}

#[test]
fn a_mistyped_field_of_a_named_source_is_refused_at_the_boundary_rather_than_ignored() {
    // Subprocess settings are intentionally opaque until host startup, so no config-show protocol capability can repeat this refusal.
    let boundary = SourceBoundary::Direct;
    let sandbox = Sandbox::new();
    sandbox.project_document(
        &serde_json::to_string(&serde_json::json!({
            "sources": {"work": boundary.source("in-memory", serde_json::json!({"taks": []}))}
        }))
        .expect("a document"),
    );

    let message = refusal(&sandbox, &[]);
    assert!(
        message.contains(&boundary.config_path("taks")),
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
fn a_set_argument_that_is_not_an_assignment_is_refused_as_a_bad_invocation() {
    let sandbox = Sandbox::new();

    let message = mistyped(&sandbox, &["config", "show", "--set", "page_size"]);
    assert!(message.contains("--set page_size"), "{message}");
}

#[test]
fn a_set_argument_addressing_no_setting_is_refused_as_a_bad_invocation() {
    let sandbox = Sandbox::new();

    let message = mistyped(
        &sandbox,
        &["config", "show", "--set", "sources..plugin=in-memory"],
    );
    assert!(message.contains("--set"), "{message}");
}

/// Run `arguments` over `sandbox`, expect the exit code for a mistyped invocation, and
/// return what it said.
///
/// `2` is what the command line documents for an invocation that was typed wrongly, and
/// what clap itself exits with for an unknown flag. A caller branching on the code cannot
/// be asked to know which of the two spotted the mistake, so a `--set` that is not
/// `PATH=VALUE` exits the same way an unknown flag does rather than as a run that broke.
fn mistyped(sandbox: &Sandbox, arguments: &[&str]) -> String {
    let output = sandbox
        .command()
        .args(arguments)
        .assert()
        .code(2)
        .get_output()
        .clone();
    assert!(
        stdout(&output).is_empty(),
        "a refusal writes nothing to stdout"
    );
    stderr(&output)
}

/// Unix only: this drives a shell into removing its own working directory before it
/// execs the binary, and Windows will not unlink a directory a process is sitting in.
/// The functional lanes still gate that platform through every other journey here.
#[cfg(unix)]
#[test]
fn a_working_directory_that_no_longer_exists_is_reported_rather_than_crashing() {
    let sandbox = Sandbox::new();
    let doomed = sandbox.subdirectory("doomed");
    let binary = env!("CARGO_BIN_EXE_onetaskgraph");

    // The shell removes the directory it is standing in and then execs the binary into
    // it, so the binary really starts life somewhere the kernel can no longer name —
    // which is the one way `current_dir` fails that a user can actually reach.
    let output = std::process::Command::new("sh")
        .current_dir(&doomed)
        .env("XDG_CONFIG_HOME", sandbox.config_home())
        .env_remove("HOME")
        .args(["-c", r#"rm -rf "$PWD" && exec "$0" config show"#, binary])
        .output()
        .expect("the shell runs");

    assert_eq!(output.status.code(), Some(1));
    let message = stderr(&output);
    assert!(
        message.contains("could not read the working directory"),
        "the failure names what could not be read: {message}"
    );
    assert!(message.contains("next:"), "{message}");
    assert!(
        stdout(&output).is_empty(),
        "a refusal writes nothing to stdout"
    );
}

/// Unix only: an environment block holding bytes that are not UTF-8 is something only
/// `OsString` can express, and Windows environment blocks are UTF-16 rather than bytes.
#[cfg(unix)]
#[test]
fn a_setting_variable_this_build_cannot_read_is_refused_by_name_rather_than_ignored() {
    use std::os::unix::ffi::OsStringExt;

    let sandbox = Sandbox::new();
    let not_utf8 = std::ffi::OsString::from_vec(vec![0x66, 0xff, 0x6f]);

    let output = sandbox
        .command()
        .env("ONETASKGRAPH_OUTPUT", &not_utf8)
        .args(["config", "show"])
        .assert()
        .failure()
        .get_output()
        .clone();

    let message = stderr(&output);
    assert!(
        message.contains("ONETASKGRAPH_OUTPUT"),
        "the refusal names the variable: {message}"
    );
    assert!(message.contains("next:"), "{message}");
    assert!(
        stdout(&output).is_empty(),
        "a refusal writes nothing to stdout"
    );
}

/// Unix only, for the same reason as the refusal above.
#[cfg(unix)]
#[test]
fn a_variable_this_build_cannot_read_and_never_asked_for_leaves_the_run_alone() {
    use std::os::unix::ffi::OsStringExt;

    let sandbox = Sandbox::new();
    let not_utf8 = std::ffi::OsString::from_vec(vec![0x66, 0xff, 0x6f]);

    let output = sandbox
        .command()
        .env("SOMETHING_ELSE", &not_utf8)
        .args(["config", "show", "--json"])
        .assert()
        .success()
        .get_output()
        .clone();

    let page_size = setting(&shown(&output), "page_size").clone();
    assert_eq!(page_size["value"], 50);
    assert_eq!(page_size["origin"]["layer"], "default");
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

/// Check `document` against the bundle's `root`, saying which root refused it.
fn validates(bundle: &Value, root: &str, document: &Value) {
    let schema = &bundle["roots"][root];
    assert!(
        schema.is_object(),
        "the bundle this binary emits carries the {root} root: {bundle:#}"
    );
    let validator = jsonschema::validator_for(schema)
        .unwrap_or_else(|error| panic!("the {root} root compiles: {error}"));
    let problems: Vec<String> = validator
        .iter_errors(document)
        .map(|error| format!("{} at {}", error, error.instance_path()))
        .collect();
    assert!(
        problems.is_empty(),
        "what the binary emitted does not match the {root} root it also emits: {} \
         (the value was {document:#})",
        problems.join("; ")
    );
}

#[test]
fn the_machine_readable_output_matches_a_root_of_the_schema_this_binary_emits() {
    for boundary in SOURCE_BOUNDARIES {
        let sandbox = Sandbox::new();
        sandbox.project_document(&one_source(boundary));
        // Both credential layers at once, so the report carries one entry of each kind and
        // the enum of layers is exercised rather than only its first variant.
        sandbox.secrets_file(&format!(
            "LINEAR_API_KEY={PLANTED}\nGH_PROJECTS_TOKEN={PLANTED}\n"
        ));

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

        let output = sandbox
            .command()
            .env_remove("LINEAR_API_KEY")
            .env("GH_PROJECTS_TOKEN", "exported-by-the-shell")
            .args(["config", "show", "--json"])
            .assert()
            .success()
            .get_output()
            .clone();
        let shown = shown(&output);

        // The whole document, against the root the verb claims to emit.
        validates(&bundle, "EffectiveConfig", &shown);

        // `Setting.value` is an unconstrained JSON value — a setting holds whatever its
        // own type is — so the roots for the settings that *do* have a shape are checked
        // against the values this run actually reported, not left implied by the parent.
        validates(&bundle, "OutputFormat", &setting(&shown, "output")["value"]);

        let credentials = shown["secrets"]["variables"]
            .as_array()
            .expect("the report lists the credentials the file supplied");
        assert_eq!(
            credentials.len(),
            2,
            "both planted names are reported: {shown:#}"
        );
        for credential in credentials {
            validates(&bundle, "ResolvedCredential", credential);
            validates(&bundle, "CredentialLayer", &credential["resolved_from"]);
        }
        let layers: Vec<&Value> = credentials
            .iter()
            .map(|credential| &credential["resolved_from"])
            .collect();
        assert_eq!(
            layers,
            vec!["environment", "secrets-file"],
            "the exported name resolves from the environment and the other from the file: \
         {shown:#}"
        );
    }
}

#[test]
fn a_configuration_document_that_cannot_be_read_stops_the_run_rather_than_being_skipped() {
    let sandbox = Sandbox::new();
    // A project document that would set `page_size` if the layer beneath it were the
    // one in trouble, so a run that skipped the unreadable file would look successful.
    sandbox.project_document("page_size: 25\n");
    let path = sandbox.unreadable("onetaskgraph/config.yaml");

    let message = refusal(&sandbox, &[]);
    assert!(
        message.contains(&path.display().to_string()),
        "the refusal names the file it could not read: {message}"
    );
    assert!(
        message.contains("could not read"),
        "the refusal says what went wrong: {message}"
    );
}

#[test]
fn an_obstructed_project_document_stops_the_run_rather_than_being_walked_past() {
    let sandbox = Sandbox::new();
    // The user-level document would set `page_size` if the walk simply skipped what it
    // could not read, so a run that walked past this one would look successful and read
    // a configuration the user does not believe is in force.
    sandbox.user_document("page_size: 11\n");
    let obstruction = sandbox.project().join("onetaskgraph.yaml");
    std::fs::create_dir_all(&obstruction).expect("a directory in the document's place");

    let message = refusal(&sandbox, &[]);
    assert!(
        message.contains(&obstruction.display().to_string()),
        "the refusal names the document it could not read: {message}"
    );
    assert!(
        message.contains("could not read"),
        "the refusal says what went wrong: {message}"
    );
}

#[test]
fn a_credentials_file_that_cannot_be_read_stops_the_run_rather_than_being_skipped() {
    for boundary in SOURCE_BOUNDARIES {
        let sandbox = Sandbox::new();
        sandbox.project_document(&one_source(boundary));
        let path = sandbox.unreadable("onetaskgraph/secrets.env");

        let message = refusal(&sandbox, &[]);
        assert!(
            message.contains(&path.display().to_string()),
            "the refusal names the credentials file it could not read: {message}"
        );
        assert!(
            message.contains("could not read"),
            "the refusal says what went wrong: {message}"
        );
    }
}
