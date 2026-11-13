# The spike's prototypes

Each hosted demonstration of the second prototype set is the session head plus one commit,
built from this directory alone:

    git checkout -b spike-otg-<side>-<n> <session head>
    git apply spike/ci-profile/prototypes/<prototype>.patch        # none for a control
    python3 spike/ci-profile/measurement_only.py .github/workflows/ci.yml --no-recorders \
        --exclude=github-cache-policy,github-nx-policy,github-workflow-policy,github-workflows,linear-documents-policy,report-job,screenshots,scripts \
        [--cold-rust --compile-only]
    printf '\n// spike-onetaskgraph-profile: a no-behaviour edit, so this demonstration selects what an engine change selects.\n' \
        >> crates/onetaskgraph-core/src/lib.rs
    git add -A && git commit -m "DO NOT MERGE: spike-otg-<side>-<n>"

Sides: `wctl` (warm control), `wpyr` (warm, `pyramid-e2e-journeys.patch`), `wsplit` (warm,
`split-template-crate.patch`), `cctl` (`--cold-rust --compile-only`), `csplit` (the split,
`--cold-rust --compile-only`); two of each.

- `pyramid-e2e-journeys.patch` marks every journey of `onetaskgraph-e2e`'s `section_links`,
  `assets` and `held_writes` modules `#[ignore]` (121 journeys), leaving the modules' helpers
  that other journeys import. Each module has a same-named in-process suite in
  `crates/onetaskgraph-core/tests/` that keeps running.
- `split-template-crate.patch` moves `onetaskgraph-core`'s `template` module, its 13 unit
  tests and its 42-test `tests/templates.rs` into a new crate `onetaskgraph-template`, which
  core re-exports as `onetaskgraph_core::template`, so no caller changes; the Nx graph,
  plugin-isolation, store-fixture and contract-inventory guards pass on it.
