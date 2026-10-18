//! A source one layer refines and no layer declares, driven the way a user drives it.
//!
//! A layer may carry only refining fields for a source — its `routes`, one `config` field —
//! whose `plugin` another layer declares. Run where no layer read declares it, that entry is
//! a refinement with nothing to refine: the source is not configured there, everything else
//! loads, `config show` reports the entry's settings and marks it incomplete, and only a
//! command that names it — by `--source`, a qualified id, `default_sources` or a route it
//! would follow — is refused, naming it.
//!
//! Every test spawns the compiled binary in a sandbox whose user layer, project document and
//! environment it writes itself, and asserts on the exit code and on what the binary wrote.

use std::path::{Path, PathBuf};
use std::process::Output;

use onetaskgraph_e2e_support::common::{Sandbox, stderr, stdout};
use serde_json::{Value, json};

/// The complete source every case reads and writes.
const NOTES: &str = "notes";
/// The source a layer refines without declaring.
const PARTNER: &str = "partner";
const TASK: &str = "notes:ship";

/// A folder of Markdown holding one project and one task, under the project tree.
fn notes(sandbox: &Sandbox) -> PathBuf {
    let root = sandbox.subdirectory(NOTES);
    for (kind, id, front) in [
        ("projects", "goal", "title: Goal\nstatus: todo"),
        ("tasks", "ship", "title: Ship\nstatus: todo\nproject: goal"),
    ] {
        let path = root.join(kind).join(format!("{id}.md"));
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("the folder");
        std::fs::write(&path, format!("---\n{front}\n---\nBody of {id}.\n")).expect("a record");
    }
    root
}

/// A document declaring `notes` at `root`, plus `extra` lines under its entry.
fn declaring_notes(root: &Path, extra: &str) -> String {
    format!(
        "sources:\n  {NOTES}:\n    plugin: local-md\n    config:\n      root: {}\n{extra}",
        json!(root.display().to_string())
    )
}

/// A document refining `partner` with a routing list and nothing else.
const REFINING_PARTNER: &str = "sources:\n  partner:\n    routes:\n      - repositories: \
                                [\"github.com/example-org/*\"]\n        to: notes\n";

/// The environment overlay refining one `config` field of `partner`.
const PARTNER_VARIABLE: (&str, &str) = ("ONETASKGRAPH_SOURCES__PARTNER__CONFIG__ROOT", "elsewhere");

fn run(sandbox: &Sandbox, environment: &[(&str, &str)], arguments: &[&str]) -> Output {
    let mut command = sandbox.command();
    for (name, value) in environment {
        command.env(name, value);
    }
    command.args(arguments).assert().get_output().clone()
}

/// Standard output of a run that had to succeed, quoting both streams when it did not.
fn ok(sandbox: &Sandbox, environment: &[(&str, &str)], arguments: &[&str]) -> String {
    let output = run(sandbox, environment, arguments);
    assert_eq!(
        output.status.code(),
        Some(0),
        "`onetaskgraph {}` exited {:?}\n{}{}",
        arguments.join(" "),
        output.status.code(),
        stdout(&output),
        stderr(&output)
    );
    stdout(&output)
}

fn answer(sandbox: &Sandbox, environment: &[(&str, &str)], arguments: &[&str]) -> Value {
    let mut with_json = arguments.to_vec();
    with_json.push("--json");
    serde_json::from_str(&ok(sandbox, environment, &with_json)).expect("the command emits JSON")
}

/// Standard error of a run that had to fail, asserted to name `partner` as incomplete.
fn refused_as_incomplete(
    sandbox: &Sandbox,
    environment: &[(&str, &str)],
    arguments: &[&str],
) -> String {
    let output = run(sandbox, environment, arguments);
    let message = stderr(&output);
    assert_ne!(
        output.status.code(),
        Some(0),
        "`onetaskgraph {}` was expected to be refused\n{}{message}",
        arguments.join(" "),
        stdout(&output)
    );
    assert!(
        message.contains("source \"partner\" is not configured here")
            && message.contains("no configuration layer declares its plugin")
            && message.contains("next:"),
        "`onetaskgraph {}` names the source and why it is not configured:\n{message}",
        arguments.join(" ")
    );
    message
}

fn status_of_ship(sandbox: &Sandbox, environment: &[(&str, &str)]) -> Value {
    answer(sandbox, environment, &["task", "show", TASK])["items"][0]["item"]["status"]["category"]
        .clone()
}

/// `sources list`, a task read and a task write on `notes` all succeed, and `partner` is not
/// among the configured sources.
fn the_complete_source_answers(sandbox: &Sandbox, environment: &[(&str, &str)]) {
    let listed = answer(sandbox, environment, &["sources", "list"]);
    let names: Vec<&str> = listed
        .as_array()
        .expect("sources list answers a list")
        .iter()
        .map(|listing| listing["source"].as_str().expect("a source name"))
        .collect();
    assert_eq!(
        names,
        [NOTES],
        "only the complete source is configured: {listed:#}"
    );

    let tasks = answer(sandbox, environment, &["task", "list", "--source", NOTES]);
    let ids: Vec<&str> = tasks["items"]
        .as_array()
        .expect("items is a list")
        .iter()
        .map(|entry| entry["id"].as_str().expect("a qualified id"))
        .collect();
    assert_eq!(ids, [TASK], "{tasks:#}");

    assert_eq!(status_of_ship(sandbox, environment), "todo");
    ok(
        sandbox,
        environment,
        &["task", "status", "set", TASK, "done"],
    );
    assert_eq!(
        status_of_ship(sandbox, environment),
        "done",
        "the write landed and reads back"
    );
}

/// A command naming `partner` by `--source`, by a qualified id, or as the source a
/// configuration verb reads is refused, naming it.
fn naming_the_incomplete_source_is_refused(sandbox: &Sandbox, environment: &[(&str, &str)]) {
    for arguments in [
        &["task", "list", "--source", PARTNER][..],
        &["task", "show", "partner:anything"],
        &["sources", "route", PARTNER],
        &["sources", "fields", PARTNER],
        &["cache", "clear", PARTNER],
    ] {
        refused_as_incomplete(sandbox, environment, arguments);
    }
    let output = run(
        sandbox,
        environment,
        &["task", "list", "--source", PARTNER, "--json"],
    );
    let failure: Value =
        serde_json::from_str(&stdout(&output)).expect("a refusal under --json is a document");
    assert_eq!(
        (&failure["failure"]["class"], &failure["failure"]["kind"]),
        (&json!("refused"), &json!("incomplete-source")),
        "{failure:#}"
    );
}

#[test]
fn a_source_the_user_layer_alone_refines_leaves_every_other_source_working() {
    let sandbox = Sandbox::new();
    let root = notes(&sandbox);
    sandbox.project_document(&declaring_notes(&root, ""));
    sandbox.user_document(REFINING_PARTNER);

    the_complete_source_answers(&sandbox, &[]);
    naming_the_incomplete_source_is_refused(&sandbox, &[]);
}

#[test]
fn a_source_the_project_document_alone_refines_leaves_every_other_source_working() {
    let sandbox = Sandbox::new();
    let root = notes(&sandbox);
    sandbox.user_document(&declaring_notes(&root, ""));
    sandbox.project_document(REFINING_PARTNER);

    the_complete_source_answers(&sandbox, &[]);
    naming_the_incomplete_source_is_refused(&sandbox, &[]);
}

#[test]
fn a_source_an_environment_overlay_alone_refines_leaves_every_other_source_working() {
    let sandbox = Sandbox::new();
    let root = notes(&sandbox);
    sandbox.project_document(&declaring_notes(&root, ""));

    the_complete_source_answers(&sandbox, &[PARTNER_VARIABLE]);
    naming_the_incomplete_source_is_refused(&sandbox, &[PARTNER_VARIABLE]);
}

#[test]
fn a_source_the_user_layer_and_the_environment_refine_together_leaves_every_other_source_working() {
    let sandbox = Sandbox::new();
    let root = notes(&sandbox);
    sandbox.project_document(&declaring_notes(&root, ""));
    sandbox.user_document(REFINING_PARTNER);

    the_complete_source_answers(&sandbox, &[PARTNER_VARIABLE]);
    naming_the_incomplete_source_is_refused(&sandbox, &[PARTNER_VARIABLE]);
}

#[test]
fn default_sources_naming_an_incomplete_source_refuses_only_the_commands_that_read_them() {
    let sandbox = Sandbox::new();
    let root = notes(&sandbox);
    sandbox.project_document(&format!(
        "default_sources: [notes, partner]\n{}",
        declaring_notes(&root, "")
    ));
    sandbox.user_document(REFINING_PARTNER);

    refused_as_incomplete(&sandbox, &[], &["task", "list"]);
    let tasks = answer(&sandbox, &[], &["task", "list", "--source", NOTES]);
    assert_eq!(tasks["items"][0]["id"], TASK, "{tasks:#}");
    ok(&sandbox, &[], &["task", "status", "set", TASK, "done"]);
    assert_eq!(status_of_ship(&sandbox, &[]), "done");
}

#[test]
fn config_show_reports_the_incomplete_entry_with_its_layers_and_marks_it() {
    let sandbox = Sandbox::new();
    let root = notes(&sandbox);
    sandbox.project_document(&declaring_notes(&root, ""));
    let user = sandbox.user_document(REFINING_PARTNER);

    let shown = answer(&sandbox, &[PARTNER_VARIABLE], &["config", "show"]);
    assert_eq!(shown["incomplete_sources"], json!([PARTNER]), "{shown:#}");
    let settings = shown["settings"].as_array().expect("settings is a list");
    let origin_of = |key: &str| {
        settings
            .iter()
            .find(|setting| setting["key"] == key)
            .unwrap_or_else(|| panic!("{key} is reported: {shown:#}"))["origin"]
            .clone()
    };
    assert_eq!(
        origin_of("sources.partner.routes"),
        json!({"layer": "file", "path": user.display().to_string()})
    );
    assert_eq!(
        origin_of("sources.partner.config.root"),
        json!({"layer": "environment", "variable": PARTNER_VARIABLE.0})
    );

    let human = ok(&sandbox, &[PARTNER_VARIABLE], &["config", "show"]);
    assert!(
        human.contains("incomplete sources") && human.contains("  partner  incomplete"),
        "the human rendering marks the source incomplete:\n{human}"
    );
    assert!(
        human
            .lines()
            .any(|line| line.starts_with("sources.partner.config.root")
                && line.ends_with(&format!("environment {}", PARTNER_VARIABLE.0))),
        "the human rendering names the layer of each of its settings:\n{human}"
    );

    // A configuration with no incomplete source leaves the field out of its document.
    let complete = Sandbox::new();
    complete.project_document(&declaring_notes(&notes(&complete), ""));
    let shown = answer(&complete, &[], &["config", "show"]);
    assert!(shown.get("incomplete_sources").is_none(), "{shown:#}");
    assert!(!ok(&complete, &[], &["config", "show"]).contains("incomplete"));
}

#[test]
fn a_layer_declaring_the_plugin_completes_the_refined_source_as_before() {
    let sandbox = Sandbox::new();
    let root = notes(&sandbox);
    let partner = sandbox.subdirectory("partner-store");
    sandbox.project_document(&format!(
        "{}  partner:\n    plugin: local-md\n    config:\n      root: {}\n",
        declaring_notes(&root, ""),
        json!(partner.display().to_string())
    ));
    sandbox.user_document(REFINING_PARTNER);

    let listed = answer(&sandbox, &[], &["sources", "list"]);
    let names: Vec<&str> = listed
        .as_array()
        .expect("a list")
        .iter()
        .map(|listing| listing["source"].as_str().expect("a name"))
        .collect();
    assert_eq!(names, [NOTES, PARTNER], "{listed:#}");
    let shown = answer(&sandbox, &[], &["config", "show"]);
    assert!(shown.get("incomplete_sources").is_none(), "{shown:#}");
    // The user layer's route merged onto the declared source and is followed.
    assert_eq!(
        answer(
            &sandbox,
            &[],
            &[
                "sources",
                "route",
                PARTNER,
                "--repository",
                "github.com/example-org/app"
            ]
        ),
        json!({"source": PARTNER, "destination": NOTES, "route": 0})
    );

    // A source that has a plugin but is otherwise invalid is still refused, everywhere.
    let output = run(
        &sandbox,
        &[("ONETASKGRAPH_SOURCES__PARTNER__CONFIG__NOPE", "1")],
        &["sources", "list"],
    );
    assert_ne!(output.status.code(), Some(0));
    assert!(
        stderr(&output).contains("sources.partner"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn a_route_to_an_incomplete_source_is_refused_by_the_command_that_would_follow_it() {
    let sandbox = Sandbox::new();
    let root = notes(&sandbox);
    let archive = sandbox.subdirectory("archive");
    sandbox.project_document(&format!(
        "{}  archive:\n    plugin: local-md\n    config:\n      root: {}\n",
        declaring_notes(
            &root,
            "    routes:\n      - repositories: [\"github.com/example-org/*\"]\n        to: \
             partner\n      - repositories: [\"github.com/other-org/*\"]\n        to: archive\n",
        ),
        json!(archive.display().to_string())
    ));
    sandbox.user_document("sources:\n  partner:\n    visibility: private\n");
    let body = sandbox.subdirectory("bodies").join("body.md");
    std::fs::write(&body, "What to do.\n").expect("a body");
    let body = body.display().to_string();
    let create = |title: &'static str, repository: &'static str| {
        vec![
            "task".to_owned(),
            "create".to_owned(),
            NOTES.to_owned(),
            "--project".to_owned(),
            "goal".to_owned(),
            "--title".to_owned(),
            title.to_owned(),
            "--repository".to_owned(),
            repository.to_owned(),
            "--body-file".to_owned(),
            body.clone(),
        ]
    };

    // The route to a complete source still names its destination, and is followed.
    assert_eq!(
        answer(
            &sandbox,
            &[],
            &[
                "sources",
                "route",
                NOTES,
                "--repository",
                "github.com/other-org/app"
            ]
        ),
        json!({"source": NOTES, "destination": "archive", "route": 1})
    );
    let arguments = create("Archived work", "github.com/other-org/app");
    let created = answer(
        &sandbox,
        &[],
        &arguments.iter().map(String::as_str).collect::<Vec<_>>(),
    );
    let id = created["items"][0]["id"].as_str().expect("an id");
    assert!(id.starts_with("archive:"), "{created:#}");

    // The route to the incomplete one is refused where it would be followed, writing nothing.
    let before = files_under(sandbox.project().as_path());
    let arguments = create("Partner work", "github.com/example-org/app");
    refused_as_incomplete(
        &sandbox,
        &[],
        &arguments.iter().map(String::as_str).collect::<Vec<_>>(),
    );
    assert_eq!(
        files_under(sandbox.project().as_path()),
        before,
        "nothing was written"
    );
}

/// Every file under `root`, with its bytes, in path order.
fn files_under(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory).expect("a readable directory") {
            let path = entry.expect("an entry").path();
            if path.is_dir() {
                pending.push(path);
            } else {
                let bytes = std::fs::read(&path).expect("a readable file");
                found.push((path, bytes));
            }
        }
    }
    found.sort();
    found
}

/// A sandbox whose `notes` carries `routes` and whose user layer refines `partner` alone,
/// and the arguments creating a private task in `notes` from `repository`.
fn private_create(routes: &str, repository: &str) -> (Sandbox, Vec<String>) {
    let sandbox = Sandbox::new();
    let root = notes(&sandbox);
    sandbox.project_document(&declaring_notes(&root, routes));
    sandbox.user_document("sources:\n  partner:\n    visibility: private\n");
    std::fs::write(
        root.join("projects/vault.md"),
        "---\ntitle: Vault\nstatus: todo\nclassification: private\n---\nPrivate work.\n",
    )
    .expect("a private project");
    let body = sandbox.subdirectory("bodies").join("body.md");
    std::fs::write(&body, "What to do.\n").expect("a body");
    let arguments = [
        "task",
        "create",
        NOTES,
        "--project",
        "vault",
        "--title",
        "Private work",
        "--classification",
        "private",
        "--repository",
        repository,
        "--body-file",
    ]
    .into_iter()
    .map(str::to_owned)
    .chain([body.display().to_string()])
    .collect();
    (sandbox, arguments)
}

#[test]
fn a_private_item_routed_to_an_incomplete_source_is_refused_rather_than_kept_where_it_was_written()
{
    for routes in [
        // A classification entry, which loads although no layer read declares `partner`
        // private.
        "    routes:\n      - classification: private\n        to: partner\n",
        // A repository entry, which a private item takes only toward a private destination.
        "    routes:\n      - repositories: [\"github.com/example-org/*\"]\n        to: partner\n",
    ] {
        let (sandbox, arguments) = private_create(routes, "github.com/example-org/app");
        let before = files_under(sandbox.project().as_path());
        refused_as_incomplete(
            &sandbox,
            &[],
            &arguments.iter().map(String::as_str).collect::<Vec<_>>(),
        );
        assert_eq!(
            files_under(sandbox.project().as_path()),
            before,
            "nothing was written under\n{routes}"
        );
    }
}
