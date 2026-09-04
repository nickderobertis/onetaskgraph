//! The journeys hold whatever credentials the host that runs them happens to carry.
//!
//! Several of them turn on a configured source *not* being buildable: a `linear` block
//! with no `api_key_env` falls back to `LINEAR_API_KEY`, and the journey asserts the
//! source is reported unavailable, or that a verb against it costs exit 4. A host with
//! that variable set makes those sources build and answer, and every one of those
//! assertions then reads the opposite of what happened — which is exactly what this
//! repository's required check began doing when it started handing the live lanes their
//! credentials on one leg of its matrix.
//!
//! So the sandbox removes [`common::AMBIENT_CREDENTIALS`] from the child, and this is
//! that removal driven the way the host drives it: the variable is really set in this
//! process before the binary is spawned, so the child would inherit it if the sandbox
//! did not take it away.
//!
//! **A binary of its own, holding one test.** `std::env::set_var` is unsafe because
//! another thread may be reading the environment as it is written; a test binary with a
//! single test has no other test thread to race, which is what makes this the one place
//! the variable can be set for real rather than described.

// The same shared sandbox `tests/configuration.rs` and the e2e target use, and the same
// allowance those make: this target drives one journey and needs a handful of its helpers.
#[allow(
    dead_code,
    reason = "one shared sandbox, three test targets, a subset used by each"
)]
mod common;

use common::{AMBIENT_CREDENTIALS, Sandbox, stderr, stdout};
use serde_json::json;

#[test]
fn a_source_with_no_credential_is_unavailable_however_the_host_that_runs_this_is_set_up() {
    // The host under test: every name the sandbox is meant to take away, set to something
    // that would build a source if it reached one.
    for name in AMBIENT_CREDENTIALS {
        // SAFETY: this binary holds exactly one test, so no other thread of it is reading
        // the environment while this writes it. The module note above says why that is the
        // shape rather than a test beside the others.
        unsafe { std::env::set_var(name, "ambient-key-the-sandbox-must-not-pass-on") };
    }

    let sandbox = Sandbox::new();
    sandbox.project_document(
        &serde_json::to_string(&json!({
            "sources": {"gone": {"plugin": "linear", "config": {}}}
        }))
        .expect("a one-source document"),
    );

    let listing = sandbox
        .command()
        .args(["sources", "list"])
        .output()
        .expect("the binary runs");
    let rendered = stdout(&listing);
    assert!(
        rendered.contains("unavailable") && rendered.contains("LINEAR_API_KEY"),
        "the source must still be reported unavailable, naming the variable it wanted:\n\
         {rendered}{}",
        stderr(&listing)
    );

    // And the verb against it still costs exit 4 rather than reaching Linear and
    // answering, which is the half that would otherwise send a read to a real workspace.
    let listed = sandbox
        .command()
        .args(["task", "list"])
        .output()
        .expect("the binary runs");
    assert_eq!(
        listed.status.code(),
        Some(4),
        "a source that cannot be built must still cost exit 4:\n{}",
        stderr(&listed)
    );
    assert!(
        stderr(&listed).contains("gone"),
        "and still name the source:\n{}",
        stderr(&listed)
    );
}
