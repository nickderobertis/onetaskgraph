//! [`common::AMBIENT_CREDENTIALS`] removed for real, rather than described.
//!
//! **A binary of its own, holding one test.** `std::env::set_var` is unsafe because
//! another thread may be reading the environment as it is written, and a test binary with
//! a single test has none — which is what lets this one set the variables in its own
//! process, so the child would inherit them if the sandbox did not take them away.

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
