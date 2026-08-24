//! Machine-readable output validates against the schema the binary itself emits.
//!
//! Not against a schema checked in beside these tests, and not against one written by
//! hand: the document `onetaskgraph schema` prints is what both SDKs are generated from,
//! so it is the only document worth validating against. If `--json` and the bundle ever
//! disagree, an SDK generated from the bundle is a generator emitting models the binary
//! never sends — which is a failure nothing else here would notice.

use serde_json::Value;

use crate::common::{SOURCE_BOUNDARIES, Sandbox, stdout};
use crate::fixtures::{NATIVE, SCANNED, pair_at, qualified};

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
        sandbox.project_document(&pair_at(&sandbox, boundary));
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
