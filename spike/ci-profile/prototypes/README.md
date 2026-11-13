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
- the split, with matched selection: control 43445538503 (#4020) and 43445570366 (#4022); split
  43445553591 (#4021) and 43445585382 (#4023), built as above with session head `f146009e` and
  `--job-exclude` holding each job to the control's selection:
  `test-linux:onetaskgraph-live-janitor,onetaskgraph-template,pool-maintenance` and, on macOS and
  Windows, `onetaskgraph-e2e-support,onetaskgraph-github-live,onetaskgraph-in-memory,onetaskgraph-live-janitor,onetaskgraph-live,onetaskgraph-local-md,onetaskgraph-plugin-api,onetaskgraph-status-options,onetaskgraph-template,pool-maintenance`.
  macOS and Windows then schedule exactly the control's tasks. Linux's coverage is never narrowed
  by an exclusion (`scripts/ci-job.sh`), so it still schedules `onetaskgraph-template:coverage`
  and `pool-maintenance:coverage`, declared with `--side-only`. Some tasks the control replays
  from the Nx cache the split executes, because its own manifest edit moves their keys; `deltas`
  records them under `executed_by_one_side` and compares nothing over them.
- superseded, kept as cited: the first split pair 43422809261 (#4012) and 43422873045 (#4017),
  which scheduled 47 / 34 / 34 tasks against 35 / 23 / 23, and the cold builds 43422822720
  (#4013), 43422893836 (#4018), 43422834977 (#4014) and 43422905157 (#4019).

    CIPROFILE_OFFLINE=1 CIPROFILE_CACHE=<cache> python3 spike/ci-profile/ciprofile.py profile <run> --json   # profile-<run>.{txt,json}
    CIPROFILE_OFFLINE=1 CIPROFILE_CACHE=<cache> python3 spike/ci-profile/ciprofile.py compile <run>         # compile-<run>.txt
    D=spike/ci-profile/data/prototypes
    python3 spike/ci-profile/prototype_replay.py deltas $D --control 43422783410,43422844825 \
        --side pyramid=43422794958,43422857812 --out $D/deltas-pyramid.json
    python3 spike/ci-profile/prototype_replay.py deltas $D --control 43445538503,43445570366 \
        --side split=43445553591,43445585382 --side-only test-linux:onetaskgraph-template:coverage \
        --side-only test-linux:pool-maintenance:coverage --out $D/deltas-split.json
    for d in pyramid split; do python3 spike/ci-profile/prototype_replay.py replay \
        spike/ci-profile/data/history-first-runs.json spike/ci-profile/data/prs30.json $D/deltas-$d.json; done \
        > $D/replay-elapsed.txt

Each `deltas-*.json` keeps three figures apart: `elapsed_saved_s` (each task's elapsed interval,
control less side), the measured job figures (wall, gate span, latest-finishing chain), and two
that are never a prediction of job minutes: `share_saved_s` (wall-share attribution) and
`work_saved_s` (summed test-seconds, from Linux's nextest records). `replay-elapsed.txt` is the
predicted job shortening: the measured elapsed change moved through each PR's own critical path,
from the task intervals `history --pin` added to `../history-first-runs.json`.
