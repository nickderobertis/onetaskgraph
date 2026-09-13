//! Machine-readable output validates against the schema the binary itself emits.
//!
//! Not against a schema checked in beside these tests, and not against one written by
//! hand: the document `onetaskgraph schema` prints is what both SDKs are generated from,
//! so it is the only document worth validating against. If `--json` and the bundle ever
//! disagree, an SDK generated from the bundle is a generator emitting models the binary
//! never sends — which is a failure nothing else here would notice.

use std::process::Output;

use serde_json::{Value, json};

use crate::common::{SOURCE_BOUNDARIES, Sandbox, stderr, stdout};
use crate::fixtures::{
    NATIVE, SCANNED, document, empty_document_store, empty_folder, github_projects_rate_limited,
    github_projects_unreachable, github_projects_with_board, pair_at, qualified,
};

/// The capability pair, plus the two destinations a copy needs.
///
/// A folder of Markdown for tasks and projects, and an in-memory source that has documents
/// for documents: `local-md` declares it holds none, so a document copy naming it is
/// refused before anything is read and there would be no report to validate.
fn with_a_destination(sandbox: &Sandbox, boundary: crate::common::SourceBoundary) -> String {
    let mut document: Value =
        serde_json::from_str(&pair_at(sandbox, boundary)).expect("a fixture document is JSON");
    document["sources"]["notes"] =
        serde_json::json!({"plugin": "local-md", "config": empty_folder(sandbox, "notes")});
    document["sources"]["store"] =
        serde_json::json!({"plugin": "in-memory", "config": empty_document_store()});
    serde_json::to_string(&document).expect("a document renders")
}

/// The bundle this binary emits, as a validator can read it.
fn bundle(sandbox: &Sandbox) -> Value {
    let rendered = stdout(
        sandbox
            .command()
            .arg("schema")
            .assert()
            .success()
            .get_output(),
    );
    serde_json::from_str(&rendered).expect("the bundle is JSON")
}

/// Validate `document` against the bundle's root called `root`.
fn validates(bundle: &Value, root: &str, document: &Value, what: &str) {
    let schema = &bundle["roots"][root];
    assert!(schema.is_object(), "the bundle has no root called {root}");
    let validator = jsonschema::validator_for(schema)
        .unwrap_or_else(|error| panic!("{root} is not a usable schema: {error}"));
    let problems: Vec<String> = validator
        .iter_errors(document)
        .map(|problem| format!("  {} at {}", problem, problem.instance_path()))
        .collect();
    assert!(
        problems.is_empty(),
        "`{what}` does not validate against {root}:\n{}\n{}",
        problems.join("\n"),
        serde_json::to_string_pretty(document).expect("renders")
    );
}

#[test]
fn every_verbs_machine_readable_output_validates_against_the_emitted_schema() {
    for boundary in SOURCE_BOUNDARIES {
        let sandbox = Sandbox::new();
        sandbox.project_document(&with_a_destination(&sandbox, boundary));
        let bundle = bundle(&sandbox);

        for (arguments, root) in [
            (
                vec!["task", "list", "--json"],
                "QueryResponseOfQualifiedTask",
            ),
            (
                vec!["task", "show", &qualified(NATIVE, "T-1"), "--json"],
                "QueryResponseOfQualifiedTask",
            ),
            (
                vec!["project", "list", "--json"],
                "QueryResponseOfQualifiedProject",
            ),
            (
                vec!["project", "show", &qualified(NATIVE, "P-1"), "--json"],
                "QueryResponseOfQualifiedProject",
            ),
            (
                vec!["document", "list", "--json"],
                "QueryResponseOfQualifiedDocument",
            ),
            (
                vec!["document", "show", &qualified(NATIVE, "D-1"), "--json"],
                "QueryResponseOfQualifiedDocument",
            ),
            (
                vec!["label", "list", "--json"],
                "QueryResponseOfQualifiedLabel",
            ),
            (
                vec!["task", "deps", &qualified(NATIVE, "T-1"), "--json"],
                "QueryResponseOfQualifiedEdge",
            ),
            (
                vec![
                    "task",
                    "deps",
                    &qualified(SCANNED, "T-2"),
                    "--direction",
                    "depended-on-by",
                    "--json",
                ],
                "QueryResponseOfQualifiedEdge",
            ),
            (
                vec!["project", "deps", &qualified(NATIVE, "P-1"), "--json"],
                "QueryResponseOfQualifiedEdge",
            ),
            (
                vec!["search", "alpha", "--json"],
                "QueryResponseOfSearchHit",
            ),
        ] {
            let rendered = stdout(
                sandbox
                    .command()
                    .args(&arguments)
                    .assert()
                    .success()
                    .get_output(),
            );
            let document: Value = serde_json::from_str(&rendered).unwrap_or_else(|error| {
                panic!("{arguments:?} did not emit JSON: {error}\n{rendered}")
            });
            validates(&bundle, root, &document, &arguments.join(" "));

            // Not vacuously: the response carries rows and the plan that produced them.
            assert!(
                document["items"]
                    .as_array()
                    .is_some_and(|items| !items.is_empty()),
                "{arguments:?} returned no rows to validate:\n{rendered}"
            );
            assert!(
                document["plan"]["per_source"]
                    .as_array()
                    .is_some_and(|plans| !plans.is_empty()),
                "every machine-readable response carries the plan:\n{rendered}"
            );
        }

        // A copy answers with what it did to each item rather than with rows and a plan,
        // so it validates against its own root — and the same document is what an SDK is
        // generated against, so a copy a script reads cannot be a shape nothing declared.
        let copied = stdout(
            sandbox
                .command()
                .args([
                    "task",
                    "copy",
                    &qualified(NATIVE, "T-1"),
                    "--to",
                    "notes",
                    "--json",
                ])
                .assert()
                .success()
                .get_output(),
        );
        let copied: Value = serde_json::from_str(&copied).expect("a copy emits JSON");
        validates(&bundle, "CopyReport", &copied, "task copy --json");
        assert_eq!(
            copied["items"][0]["action"], "created",
            "the copy has to have done something: {copied}"
        );
        validates(
            &bundle,
            "CopyOutcome",
            &copied["items"][0],
            "one entry of a copy report",
        );

        let copied_project = stdout(
            sandbox
                .command()
                .args([
                    "project",
                    "copy",
                    &qualified(NATIVE, "P-1"),
                    "--to",
                    "notes",
                    "--no-tasks",
                    "--json",
                ])
                .assert()
                .success()
                .get_output(),
        );
        let copied_project: Value =
            serde_json::from_str(&copied_project).expect("a project copy emits JSON");
        validates(
            &bundle,
            "CopyReport",
            &copied_project,
            "project copy --json",
        );
        assert_eq!(
            copied_project["items"][0]["action"], "created",
            "the project copy has to have done something: {copied_project}"
        );

        let copied_document = stdout(
            sandbox
                .command()
                .args([
                    "document",
                    "copy",
                    &qualified(NATIVE, "D-1"),
                    "--to",
                    "store",
                    "--json",
                ])
                .assert()
                .success()
                .get_output(),
        );
        let copied_document: Value =
            serde_json::from_str(&copied_document).expect("a document copy emits JSON");
        validates(
            &bundle,
            "CopyReport",
            &copied_document,
            "document copy --json",
        );
        assert_eq!(
            copied_document["items"][0]["action"], "created",
            "the document copy has to have done something: {copied_document}"
        );

        // A figure of zero is absent from the machine output rather than written as nought,
        // so a copy that recognised no reference emits exactly the document a consumer
        // written before these figures existed already handles — and the schema, which
        // declares each of them optional with a default of `0`, still validates it above.
        for figure in [
            "references_rewritten",
            "references_unresolved",
            "references_ambiguous",
        ] {
            for (verb, report) in [
                ("task copy", &copied),
                ("project copy", &copied_project),
                ("document copy", &copied_document),
            ] {
                assert!(
                    report.get(figure).is_none(),
                    "{boundary:?}: `{verb} --json` recognised no reference, so it carries no \
                     {figure}: {report}"
                );
            }
        }

        // `config show --json` answers about the configuration rather than about work, so it
        // carries no items and no plan — but it is a verb with a machine-readable form, and a
        // root in the bundle, so an SDK is generated against it like any other.
        let effective = stdout(
            sandbox
                .command()
                .args(["config", "show", "--json"])
                .assert()
                .success()
                .get_output(),
        );
        let effective: Value = serde_json::from_str(&effective).expect("JSON");
        validates(&bundle, "EffectiveConfig", &effective, "config show --json");

        // `sources list --json` is an array of listings rather than a query response.
        let listings = stdout(
            sandbox
                .command()
                .args(["sources", "list", "--json"])
                .assert()
                .success()
                .get_output(),
        );
        let listings: Value = serde_json::from_str(&listings).expect("JSON");
        for listing in listings.as_array().expect("an array of listings") {
            validates(&bundle, "SourceListing", listing, "sources list --json");
        }
    }
}

#[test]
fn the_plan_a_machine_reads_and_the_plan_a_person_reads_say_the_same_thing() {
    for boundary in SOURCE_BOUNDARIES {
        let sandbox = Sandbox::new();
        sandbox.project_document(&pair_at(&sandbox, boundary));

        let explained = stdout(
            sandbox
                .command()
                .args(["task", "list", "--label", "bug", "--explain"])
                .assert()
                .success()
                .get_output(),
        );
        let machine: Value = serde_json::from_str(&stdout(
            sandbox
                .command()
                .args(["task", "list", "--label", "bug", "--json"])
                .assert()
                .success()
                .get_output(),
        ))
        .expect("JSON");

        for entry in machine["plan"]["per_source"]
            .as_array()
            .expect("one entry per source")
        {
            let source = entry["source"].as_str().expect("a source name");
            assert!(
                explained.contains(source),
                "the rendered plan omits {source}:\n{explained}"
            );
            for (field, label) in [
                ("pushed_down", "pushed down"),
                ("applied_locally", "applied locally"),
            ] {
                for predicate in entry[field].as_array().expect("a list of predicates") {
                    let predicate = predicate.as_str().expect("a predicate name");
                    assert!(
                        explained.contains(&format!("{label}: {predicate}")),
                        "the rendered plan does not say `{label}: {predicate}`:\n{explained}"
                    );
                }
            }
        }

        // And the machine-readable page token is the one the rendered output tells a user to
        // paste, so the two ways of paging cannot diverge.
        let paged = stdout(
            sandbox
                .command()
                .args(["task", "list", "--limit", "2"])
                .assert()
                .success()
                .get_output(),
        );
        let token = paged
            .lines()
            .find_map(|line| line.strip_prefix("next page: --page "))
            .expect("a walk longer than the limit reports where to resume");
        let machine: Value = serde_json::from_str(&stdout(
            sandbox
                .command()
                .args(["task", "list", "--limit", "2", "--json"])
                .assert()
                .success()
                .get_output(),
        ))
        .expect("JSON");
        assert_eq!(machine["next"].as_str(), Some(token));
    }
}

/// The one failure document a run that exited `1` wrote, held to everything the contract
/// says of it, and the failure inside it.
///
/// Exactly one document and nothing else on stdout, because that is what a caller parses;
/// valid against the root the binary itself emits; one member holding exactly the five
/// the contract names; and a `message` that is the stderr line without its prefix, so the
/// document and the line a person reads cannot say different things.
fn failure_document(bundle: &Value, output: &Output, what: &str) -> Value {
    assert_eq!(
        output.status.code(),
        Some(1),
        "`{what}` must exit 1:\n{}{}",
        stdout(output),
        stderr(output)
    );
    let rendered = stdout(output);
    let mut documents = serde_json::Deserializer::from_str(&rendered).into_iter::<Value>();
    let document = documents
        .next()
        .unwrap_or_else(|| panic!("`{what}` wrote no document:\n{}", stderr(output)))
        .unwrap_or_else(|error| {
            panic!("`{what}` wrote something that is not JSON ({error}):\n{rendered}")
        });
    assert!(
        documents.next().is_none(),
        "`{what}` wrote more than one document to stdout:\n{rendered}"
    );
    validates(bundle, "FailureDocument", &document, what);

    let members = |value: &Value| {
        let mut names: Vec<String> = value
            .as_object()
            .unwrap_or_else(|| panic!("`{what}` wrote a non-object: {value}"))
            .keys()
            .cloned()
            .collect();
        names.sort_unstable();
        names
    };
    assert_eq!(members(&document), ["failure"], "{document}");
    let failure = document["failure"].clone();
    assert_eq!(
        members(&failure),
        ["class", "kind", "message", "retry_after_seconds", "source"],
        "{failure}"
    );
    let message = failure["message"].as_str().expect("a message is a string");
    assert_eq!(
        stderr(output),
        format!("onetaskgraph: {message}\n"),
        "`{what}`: the document's message is the stderr line without its prefix"
    );
    failure
}

/// The same failing command with text output: the same exit code and the same stderr line
/// as the machine-readable run, and nothing on stdout.
fn unchanged_as_text(text: &Output, machine: &Output, what: &str) {
    assert_eq!(text.status.code(), Some(1), "`{what}`:\n{}", stderr(text));
    assert!(
        text.stdout.is_empty(),
        "`{what}` with text output writes no document:\n{}",
        stdout(text)
    );
    assert_eq!(
        stderr(text),
        stderr(machine),
        "`{what}`: machine output changes nothing on stderr"
    );
}

/// A folder of plans holding one project in progress and one task to do in it.
fn plans(sandbox: &Sandbox) -> std::path::PathBuf {
    let root = sandbox.subdirectory("plans");
    std::fs::create_dir_all(root.join("projects")).expect("the projects folder");
    std::fs::create_dir_all(root.join("tasks")).expect("the tasks folder");
    std::fs::write(
        root.join("projects/P-1.md"),
        "---\ntitle: Published roadmap\nstatus: Doing\n---\nThe permanent plan\n",
    )
    .expect("the project");
    std::fs::write(
        root.join("tasks/A.md"),
        "---\ntitle: First step\nstatus: Todo\nproject: P-1\n---\ndo this first\n",
    )
    .expect("the task");
    root
}

/// Those plans, configured beside a GitHub board configured as `board`.
fn plans_beside(sandbox: &Sandbox, board: &Value) {
    let root = plans(sandbox);
    sandbox.project_document(&document(&json!({
        "plans": {"plugin": "local-md", "config": {
            "root": root, "status_mapping": {"Todo": "todo", "Doing": "in-progress"}}},
        "board": {"plugin": "github-projects", "config": board}
    })));
}

fn run(sandbox: &Sandbox, arguments: &[&str]) -> Output {
    sandbox
        .command()
        .args(arguments)
        .assert()
        .get_output()
        .clone()
}

#[test]
fn a_refusal_from_a_real_source_is_a_failure_document_classed_refused() {
    // The refusal a settlement write-back was retrying forever: a board whose status
    // mapping disables a status an item carries. Asking again changes nothing, and the
    // document says so in the one member a caller branches on.
    let sandbox = Sandbox::new();
    let (mut board, _) = github_projects_with_board(&sandbox);
    board["status_mapping"]["todo"] = Value::Null;
    plans_beside(&sandbox, &board);
    let bundle = bundle(&sandbox);

    let copy = ["project", "copy", "plans:P-1", "--to", "board"];
    let machine = run(&sandbox, &[&copy[..], &["--json"]].concat());
    let failure = failure_document(&bundle, &machine, "project copy --json");
    assert_eq!(failure["class"], "refused", "{failure}");
    assert_eq!(
        failure["kind"], "refused",
        "the source error's own kind: {failure}"
    );
    assert_eq!(failure["source"], "board", "{failure}");
    assert_eq!(failure["retry_after_seconds"], Value::Null, "{failure}");
    assert!(
        failure["message"]
            .as_str()
            .is_some_and(|message| message.contains("status todo is disabled for source board")),
        "the board's own reason is what the caller is told: {failure}"
    );

    unchanged_as_text(&run(&sandbox, &copy), &machine, "project copy");
}

#[test]
fn a_rate_limit_is_transient_and_carries_the_wait_it_named() {
    // Selected by `--output json` rather than `--json`, so a selection mode that still
    // answered prose on failure would fail here.
    for (retry_after, named) in [(Some(120), json!(120)), (None, Value::Null)] {
        let sandbox = Sandbox::new();
        plans_beside(
            &sandbox,
            &github_projects_rate_limited(&sandbox, retry_after),
        );
        let bundle = bundle(&sandbox);

        let copy = ["project", "copy", "plans:P-1", "--to", "board"];
        let machine = run(&sandbox, &[&copy[..], &["--output", "json"]].concat());
        let failure = failure_document(&bundle, &machine, "project copy --output json");
        assert_eq!(failure["class"], "transient", "{failure}");
        assert_eq!(failure["kind"], "rate-limited", "{failure}");
        assert_eq!(failure["source"], "board", "{failure}");
        assert_eq!(failure["retry_after_seconds"], named, "{failure}");

        unchanged_as_text(&run(&sandbox, &copy), &machine, "project copy");
    }
}

#[test]
fn a_source_that_cannot_be_reached_is_transient() {
    // Selected by the configuration's own `output` setting, through `--set`.
    let sandbox = Sandbox::new();
    plans_beside(&sandbox, &github_projects_unreachable(&sandbox));
    let bundle = bundle(&sandbox);

    let copy = ["project", "copy", "plans:P-1", "--to", "board"];
    let machine = run(&sandbox, &[&copy[..], &["--set", "output=json"]].concat());
    let failure = failure_document(&bundle, &machine, "project copy --set output=json");
    assert_eq!(failure["class"], "transient", "{failure}");
    assert_eq!(failure["kind"], "unavailable", "{failure}");
    assert_eq!(failure["source"], "board", "{failure}");
    assert_eq!(failure["retry_after_seconds"], Value::Null, "{failure}");

    unchanged_as_text(&run(&sandbox, &copy), &machine, "project copy");
}

#[test]
fn a_failure_no_source_caused_is_refused_under_every_way_of_asking_for_json() {
    let sandbox = Sandbox::new();
    sandbox.project_document(&document(
        &json!({"work": {"plugin": "in-memory", "config": {}}}),
    ));
    let bundle = bundle(&sandbox);
    let missing = qualified("work", "NOPE");
    let show = ["task", "show", missing.as_str()];

    // The configuration's own `output` setting, through its `ONETASKGRAPH_` variable.
    let machine = sandbox
        .command()
        .args(show)
        .env("ONETASKGRAPH_OUTPUT", "json")
        .assert()
        .get_output()
        .clone();
    let failure = failure_document(&bundle, &machine, "task show with ONETASKGRAPH_OUTPUT=json");
    assert_eq!(
        failure,
        json!({"class": "refused", "kind": "no-such-item", "source": null,
               "message": failure["message"], "retry_after_seconds": null})
    );
    unchanged_as_text(&run(&sandbox, &show), &machine, "task show");

    // The other three selections, over failures the engine and the command decide.
    for (arguments, kind) in [
        (
            vec!["task", "show", missing.as_str(), "--json"],
            "no-such-item",
        ),
        (
            vec!["task", "list", "--source", "elsewhere", "--output", "json"],
            "unknown-source",
        ),
        (
            vec!["task", "show", "T-1", "--set", "output=json"],
            "invalid-id",
        ),
        (
            vec!["task", "list", "--page", "not-a-token", "--json"],
            "page-token",
        ),
    ] {
        let what = arguments.join(" ");
        let failure = failure_document(&bundle, &run(&sandbox, &arguments), &what);
        assert_eq!(failure["class"], "refused", "{what}: {failure}");
        assert_eq!(failure["kind"], kind, "{what}: {failure}");
        assert_eq!(failure["source"], Value::Null, "{what}: {failure}");
    }

    // A usage failure is the invocation that was wrong, and writes no document.
    let usage = run(
        &sandbox,
        &[
            "task",
            "show",
            missing.as_str(),
            "--json",
            "--set",
            "nonsense",
        ],
    );
    assert_eq!(usage.status.code(), Some(2), "{}", stderr(&usage));
    assert!(
        usage.stdout.is_empty(),
        "a usage failure writes no document:\n{}",
        stdout(&usage)
    );
}

#[test]
fn a_configuration_that_does_not_load_still_answers_in_the_format_asked_for() {
    // The configuration is what failed, so the format cannot come from a configuration
    // that loaded: `--json` and the environment still say what the caller reads.
    let sandbox = Sandbox::new();
    let bundle = bundle(&sandbox);
    sandbox.project_document("sources:\n  work:\n   plugin: [this is not a plugin name\n");

    let flagged = run(&sandbox, &["task", "list", "--json"]);
    let failure = failure_document(&bundle, &flagged, "task list --json");
    assert_eq!(failure["class"], "refused", "{failure}");
    assert_eq!(failure["kind"], "config-syntax", "{failure}");
    assert_eq!(failure["source"], Value::Null, "{failure}");

    let exported = sandbox
        .command()
        .args(["task", "list"])
        .env("ONETASKGRAPH_OUTPUT", "json")
        .assert()
        .get_output()
        .clone();
    assert_eq!(
        failure_document(
            &bundle,
            &exported,
            "task list with ONETASKGRAPH_OUTPUT=json"
        ),
        failure
    );

    unchanged_as_text(&run(&sandbox, &["task", "list"]), &flagged, "task list");
}

#[test]
fn a_partial_answer_classes_each_source_that_did_not_answer() {
    // One source answers, one is rate-limited and one has no credential: the answer
    // stands at exit 4 with an `errors` entry per failed source, and each entry carries the
    // class the failure document would have given it.
    let boundary = SOURCE_BOUNDARIES[0];
    let sandbox = Sandbox::new();
    let mut configured: Value =
        serde_json::from_str(&pair_at(&sandbox, boundary)).expect("a fixture document is JSON");
    configured["sources"]["board"] = json!({"plugin": "github-projects",
                                            "config": github_projects_rate_limited(&sandbox, Some(30))});
    configured["sources"]["broken"] = json!({"plugin": "linear", "config": {}});
    sandbox.project_document(&serde_json::to_string(&configured).expect("a document renders"));
    let bundle = bundle(&sandbox);

    let output = run(&sandbox, &["task", "list", "--json"]);
    assert_eq!(output.status.code(), Some(4), "{}", stderr(&output));
    let response: Value = serde_json::from_str(&stdout(&output)).expect("a partial answer is JSON");
    validates(
        &bundle,
        "QueryResponseOfQualifiedTask",
        &response,
        "task list --json",
    );
    assert!(
        response["items"]
            .as_array()
            .is_some_and(|items| !items.is_empty()),
        "the sources that answered still stand: {response}"
    );

    let mut classed: Vec<(String, String, String)> = response["errors"]
        .as_array()
        .expect("errors is an array")
        .iter()
        .map(|entry| {
            validates(&bundle, "SourceFailure", entry, "one errors entry");
            (
                entry["source"].as_str().expect("a source").to_owned(),
                entry["error"]["kind"].as_str().expect("a kind").to_owned(),
                entry["class"]
                    .as_str()
                    .expect("every entry carries a class")
                    .to_owned(),
            )
        })
        .collect();
    classed.sort();
    assert_eq!(
        classed,
        [
            (
                "board".to_owned(),
                "rate-limited".to_owned(),
                "transient".to_owned()
            ),
            ("broken".to_owned(), "auth".to_owned(), "refused".to_owned()),
        ]
    );
    let board = response["errors"]
        .as_array()
        .and_then(|errors| errors.iter().find(|entry| entry["source"] == "board"))
        .expect("the board's entry");
    assert_eq!(board["error"]["retry_after_seconds"], 30, "{board}");
}

#[test]
fn a_document_asking_for_json_gets_a_failure_document_whether_or_not_it_loads() {
    // The configuration's own `output` setting, written in a document that loads: nothing
    // on the command line asks for JSON, and the failure still answers in it.
    let sandbox = Sandbox::new();
    let bundle = bundle(&sandbox);
    sandbox.project_document(
        &serde_json::to_string(&json!({
            "output": "json",
            "sources": {"work": {"plugin": "in-memory", "config": {}}}
        }))
        .expect("a document renders"),
    );
    let missing = qualified("work", "NOPE");
    let machine = run(&sandbox, &["task", "show", &missing]);
    let failure = failure_document(&bundle, &machine, "task show under `output: json`");
    assert_eq!(failure["class"], "refused", "{failure}");
    assert_eq!(failure["kind"], "no-such-item", "{failure}");
    // And a flag over that document still wins, in the other direction.
    unchanged_as_text(
        &run(&sandbox, &["task", "show", &missing, "--output", "text"]),
        &machine,
        "task show --output text",
    );

    // A user-level document asking for JSON beneath a project document that will not
    // parse: the configuration does not load, and the layer that still reads still asks.
    let broken = Sandbox::new();
    broken.user_document("output: json\n");
    broken.project_document("sources:\n  work:\n   plugin: [this is not a plugin name\n");
    let unloaded = run(&broken, &["task", "list"]);
    let failure = failure_document(&bundle, &unloaded, "task list under a user document");
    assert_eq!(failure["class"], "refused", "{failure}");
    assert_eq!(failure["kind"], "config-syntax", "{failure}");
    assert_eq!(failure["source"], Value::Null, "{failure}");

    // And beneath a project document that cannot be read at all — a directory where the
    // file should be. The read that fails is the project's alone, so the user's document
    // beside it still says what the caller reads.
    let obstructed = Sandbox::new();
    obstructed.user_document("output: json\n");
    obstructed.subdirectory("onetaskgraph.yaml");
    let unread = run(&obstructed, &["task", "list"]);
    let failure = failure_document(&bundle, &unread, "task list beside an unreadable document");
    assert_eq!(failure["class"], "refused", "{failure}");
    assert_eq!(failure["kind"], "config-read", "{failure}");
    assert_eq!(failure["source"], Value::Null, "{failure}");
}
