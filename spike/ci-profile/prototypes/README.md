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

## The measurements, kept in `../data/prototypes/`

Runs (each PR `DO NOT MERGE: spike-otg-<side>-<n>`, closed once read): warm control
43422783410 (#4010) and 43422844825 (#4015); pyramid 43422794958 (#4011) and 43422857812
(#4016); split 43422809261 (#4012) and 43422873045 (#4017); cold control 43422822720 (#4013)
and 43422893836 (#4018); cold split 43422834977 (#4014) and 43422905157 (#4019).

    CIPROFILE_OFFLINE=1 CIPROFILE_CACHE=<cache> python3 spike/ci-profile/ciprofile.py profile <run> --json   # profile-<run>.{txt,json}
    CIPROFILE_OFFLINE=1 CIPROFILE_CACHE=<cache> python3 spike/ci-profile/ciprofile.py compile <run>         # compile-<run>.txt
    python3 spike/ci-profile/prototype_replay.py deltas spike/ci-profile/data/prototypes \
        --control 43422783410,43422844825 --side pyramid=43422794958,43422857812 \
        --side split=43422809261,43422873045 --out spike/ci-profile/data/prototypes/deltas-share.json
    python3 spike/ci-profile/prototype_replay.py replay spike/ci-profile/data/history-first-runs.json \
        spike/ci-profile/data/prs30.json spike/ci-profile/data/prototypes/deltas-share.json            # replay-share.txt

The split PR changes `Cargo.toml` and `Cargo.lock` and adds a project, so its own runs selected
47 tasks on Linux and 34 on macOS and Windows against the control's 35 and 23; wall shares, and
the share replay of the split, are confounded by that, and compile work moves between concurrent
tasks. `deltas-libtest-split.json` therefore prices the split by libtest's own seconds (its
`measure` names each figure and where it was read), replayed into `replay-libtest-split.txt`
with the same `replay` command.
