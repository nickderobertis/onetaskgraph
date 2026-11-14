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

Runs (each PR `DO NOT MERGE: spike-otg-<side>-<n>`, closed once read):
- pyramid, against the control it ran beside: control 43422783410 (#4010) and 43422844825 (#4015);
  pyramid 43422794958 (#4011) and 43422857812 (#4016). Both sides scheduled the same tasks.
- the split, matched: controls 43457659504 (#4024) and 43457450179 (#4026), built as above with
  `--marker-base --step-env=NX_SKIP_NX_CACHE=true` (each gate compares against a local commit
  without the marker comment, `marker_base.py`, and reads no Nx cache). The split, from the
  twice-repaired `split-template-crate.patch` (see `../data/prototypes/split-validation.txt`):
  macOS and Windows from 43504759508 (#4030) and 43504773180 (#4031); Linux from 43528710535
  (#4034) and 43528719773 (#4035), which also set `test-macos` and `test-windows` to `if: false`
  (manager ruling a19). Linux's split side also schedules its own crate's coverage unit,
  declared `--side-only`.
- discarded, kept as cited, because each changed the split side's workload: #4012/#4017
  (selection widened by the split's manifests), #4021/#4023 (two extra Linux units, ten one-sided
  replays), #4025/#4027 (the unrepaired patch failed its distribution journeys and Linux's
  coverage merge early), #4030/#4031's Linux jobs (no coverage read set for the new unit). Also kept:
  the cold builds 43422822720 (#4013), 43422893836 (#4018), 43422834977 (#4014), 43422905157 (#4019).

    CIPROFILE_OFFLINE=1 CIPROFILE_CACHE=<cache> python3 spike/ci-profile/ciprofile.py profile <run> --json   # profile-<run>.{txt,json}
    CIPROFILE_OFFLINE=1 CIPROFILE_CACHE=<cache> python3 spike/ci-profile/ciprofile.py compile <run>         # compile-<run>.txt
    D=spike/ci-profile/data; P=$D/prototypes; G=$D/taskgraphs
    python3 spike/ci-profile/taskgraphs.py history $D/history-first-runs.json --out $G      # pr-<n>.json
    python3 spike/ci-profile/taskgraphs.py runs $P <control run>... --out $G                # run-<id>.json
    python3 spike/ci-profile/prototype_replay.py deltas $P --control 43422783410,43422844825 \
        --side pyramid=43422794958,43422857812 --graphs $G --out $P/deltas-pyramid.json
    python3 spike/ci-profile/prototype_replay.py deltas $P --control 43457659504,43457450179 \
        --side split=43528710535,43528719773 --side-only test-linux:onetaskgraph-template:coverage \
        --jobs test-linux --graphs $G --out $P/deltas-split-linux.json
    python3 spike/ci-profile/prototype_replay.py deltas $P --control 43457659504,43457450179 \
        --side split=43504759508,43504773180 --jobs test-macos,test-windows --graphs $G \
        --out $P/deltas-split-macos-windows.json
    for d in pyramid split-linux split-macos-windows; do python3 spike/ci-profile/prototype_replay.py replay \
        $D/history-first-runs.json $D/prs30.json $P/deltas-$d.json --graphs $G; done > $P/replay-elapsed.txt
    python3 spike/ci-profile/replay.py $D/history-first-runs.json $D/prs30.json \
        $D/inputs-after-3956 $D/inputs-before-3956 --graphs $G --json $D/replay30.json > $D/replay30.txt

Each `deltas-*.json` keeps apart: the measured job figures (each run's wall and Nx gate span —
the evidence), `elapsed_saved_s` (each task's elapsed interval, control less side),
`model_on_control_s` (the model's gate change on the control runs' own timelines, to set beside
the measured one), and two figures that never predict job minutes: `share_saved_s` (wall-share
attribution) and `work_saved_s` (summed test-seconds, Linux's nextest records).
`replay-elapsed.txt` and `../replay30.txt`'s shortening are estimates of `critical_path.py`: each
PR's run re-timed over Nx's own task graph at that PR's commit (`../taskgraphs/`), a change moving
only along dependencies the run waited on.
