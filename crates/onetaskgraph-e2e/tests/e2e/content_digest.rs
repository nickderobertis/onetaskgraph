//! `task digest`, driven through the binary over a folder of Markdown: the digest of a task's
//! authored content with caller-named metadata keys left out, equal to the `content_digest` an
//! attestation records for the same content and the same exclusions.

use serde_json::Value;

use crate::attestations::Plan;
use crate::common::stderr;

/// A task holding the caller keys `myapp.owner` and `myapp.size`, under one project.
fn seed(plan: &Plan) -> String {
    plan.exits(
        &[
            "project", "create", "notes", "--id", "plan-1", "--title", "The plan",
        ],
        0,
    );
    let created = plan.exits(
        &[
            "task",
            "create",
            "notes",
            "--project",
            "plan-1",
            "--title",
            "Build",
            "--metadata",
            "myapp.owner=\"ops\"",
            "--metadata",
            "myapp.size=1",
        ],
        0,
    );
    String::from_utf8(created.stdout)
        .expect("UTF-8")
        .trim()
        .to_owned()
}

fn digest(plan: &Plan, id: &str, excludes: &[&str]) -> Value {
    let mut arguments = vec!["task", "digest", id];
    for key in excludes {
        arguments.extend(["--exclude", key]);
    }
    let answered = plan.json(&arguments);
    assert_eq!(answered["id"], id, "{answered}");
    answered
}

#[test]
fn the_digest_is_the_one_an_attestation_records_for_the_same_exclusions() {
    let plan = Plan::new(false);
    let id = seed(&plan);
    // `Plan::attest` leaves `myapp.owner` out of a task's digest.
    let record = plan.attest("task", &id);
    let excluded = digest(&plan, &id, &["myapp.owner"]);
    assert_eq!(excluded["content_digest"], record["content_digest"]);
    assert_eq!(excluded["excludes"], serde_json::json!(["myapp.owner"]));
    // A repeated key is the same exclusion, and the digest is stable across reads.
    assert_eq!(
        digest(&plan, &id, &["myapp.owner", "myapp.owner"])["content_digest"],
        record["content_digest"]
    );
    // Without the exclusion the key is part of the content, so the digest differs.
    let whole = digest(&plan, &id, &[]);
    assert_ne!(whole["content_digest"], record["content_digest"]);
    // An excluded key moving leaves it; the content moving moves it.
    plan.exits(
        &["task", "metadata", "set", &id, "myapp.owner", "\"dev\""],
        0,
    );
    assert_eq!(
        digest(&plan, &id, &["myapp.owner"])["content_digest"],
        record["content_digest"]
    );
    plan.exits(&["task", "update", &id, "--title", "Build it"], 0);
    assert_ne!(
        digest(&plan, &id, &["myapp.owner"])["content_digest"],
        record["content_digest"]
    );
}

#[test]
fn an_unknown_task_and_a_reserved_key_are_refused() {
    let plan = Plan::new(false);
    let id = seed(&plan);
    let missing = plan.run(&["task", "digest", "notes:no-such-task"]);
    assert_ne!(missing.status.code(), Some(0), "{}", stderr(&missing));
    let reserved = plan.run(&["task", "digest", &id, "--exclude", "onetaskgraph.origin"]);
    assert_ne!(reserved.status.code(), Some(0), "{}", stderr(&reserved));
    assert!(
        stderr(&reserved).contains("--exclude"),
        "{}",
        stderr(&reserved)
    );
}
