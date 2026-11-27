---
title: 'Spike report: where onetaskgraph''s test jobs spend their time'
project: ci-test-speed-2026-11-12
metadata:
  "onetaskgraph.template": {"template":"onepipeline:spike-report","digest":"sha256:f9de4090ff78d96a56b1f1d831f2d5d77c9c15e335379fefcb78a96b03caa354","body_digest":"sha256:36a844c88667de4ff8627270cb9046ad5a61849d705aaaa60997808098648508","answers_digest":"sha256:cf1833f4d1dd22957b2268766dd8eef33669594e39c96f2731282220a69919d9"}
---
# Spike `spike-onetaskgraph-profile`

- **Branch:** `nick/ci-test-speed-2026-11-12/spike-onetaskgraph-profile`

## Harness

`spike/ci-profile/` on the branch; finished commit `697233f3`. **One measurement is one command:**

    python3 spike/ci-profile/ciprofile.py profile <run id> [--jobs test-linux,test-macos,test-windows] [--json]

It reads one hosted run of `ci.yml` (its jobs, each test job's log, the Nx executions each job uploads and, on a demonstration run, the `spike-profile-<job>` recorder artifact), caching every read under `$CIPROFILE_CACHE` (default `target/ci-profile`) and refusing any read below 2,500 REST or 2,000 GraphQL remaining. Per test job it prints: the wall clock divided into categories that sum to it with an `unattributed` remainder; the rust-cache restore (seconds, size, hit); the Nx entries the job's sidecar stored; the Nx tasks by share of the wall clock; the latest-finishing task chain; on a demonstration run the test step divided by leaf-process category and the processes that ended per category; every test binary's seconds; and the 20 slowest tests. `--json` also writes the whole summary to `$CIPROFILE_CACHE/profile-<run>.json`. Exit 0 printed; 2 usage; 3 refused below the rate floor; 4 a job unfinished or a record of an unread shape.

The rest of the harness: `ciprofile.py compile <run id>` (a compile-only demonstration: the build step's minutes and cargo's own `--timings` report, its units, span, summed seconds and each workspace crate's seconds), `prototype_replay.py deltas|replay` (a prototype's measured elapsed task change against control runs of identical Nx selection, refused otherwise, with wall share and summed test-seconds kept as separate attribution and work figures, and the model checked on the control runs' own timelines; then the estimated job shortening over each population PR's own Nx task graph), `ciprofile.py history --pin H` (re-reads exactly the runs a history cites, adding each task's interval), `taskgraphs.py history|runs` (Nx's own task graph at each PR's or demonstration run's commit, read in a temporary worktree), `critical_path.py` (the dependency-aware re-timing both replays use), `marker_base.py` (the comparison a `--marker-base` gate uses), `prototypes/` (the pyramid and crate-split patches and how each demonstration is built from them), `ciprofile.py durations <since> <until>` (median job durations), `ciprofile.py history <prs.json> --out F` (each PR's first run, task by task), `replay.py` (which key inputs each re-run matched, and the finer-key options replayed; its inputs are committed under `spike/ci-profile/data/`), `measurement_only.py` (the demonstration edits: no cache write of any kind, the recorders, `--step-env` prototypes, `--job-exclude` to hold a job's selection, `--marker-base` to give both sides the same selection, and `--cold-rust --compile-only` for a cold build alone), `sampler.py` (psutil process-table recorder, one tick a second) and `testrunner.py` (a cargo target runner recording libtest's per-test `exec_time`). Tests: `python3 -m unittest discover -s spike/ci-profile -p 'test_*.py'` (114 tests, 1 skipped where psutil is absent: OK on `697233f3`), with `ruff format --check`, `ruff check` and `ty check` clean on `697233f3` (the `spike-ci-profile` Nx project's `format-check`, `lint`, `typecheck` and `test` targets).

**Its output on the finished commit** `697233f3`, offline from the read cache (`CIPROFILE_OFFLINE=1 CIPROFILE_CACHE=<cache> python3 spike/ci-profile/ciprofile.py profile 43390692001`, exit 0, `(GitHub reads this invocation: 0)`), first lines of the Linux and macOS sections (the full 346 lines are what every per-platform figure below for run 43390692001 is read from):

```
run 43390692001  head e63933bc8eab  branch spike-otg-base-2  created 2026-11-13T08:07:01Z

== test-linux (job 131769590009, success) wall 26.58 min; Nx tasks: 34 run, 11 replayed [executions artifact]
  category                          minutes   share
  test-run(instrumented)             8.63   32.5%
  test-run(plain)                    6.42   24.1%
  compile-link(plain)                3.19   12.0%
  wait:cargo-file-lock               1.97    7.4%
  compile-link(instrumented)         1.72    6.5%
  scripts-and-other-tasks            1.31    4.9%
  setup                              1.24    4.7%
  budget-checks                      1.02    3.8%
  cache-restore                      0.62    2.3%
  nx-planning                        0.28    1.1%
  report                             0.08    0.3%
  unattributed                       0.08    0.3%
  cache-save                         0.02    0.1%
  cache-restore(nx)                  0.00    0.0%
  (sum)                             26.58
  rust-cache restore: 34.3 s, 1362 MB, hit
  Nx sidecar: stored 0 entries (0 bytes), replayed 11
...
== test-macos (job 131769590459, failure) wall 29.97 min; Nx tasks: 23 run, 1 replayed [executions artifact]
  category                          minutes   share
  test-run(plain)                    7.89   26.3%
  compile-link(plain)                7.49   25.0%
  scripts-and-other-tasks            6.06   20.2%
  compile-link(release)              3.50   11.7%
  budget-checks                      2.25    7.5%
  setup                              1.98    6.6%
  cache-restore                      0.42    1.4%
  nx-planning                        0.25    0.8%
  report                             0.07    0.2%
  unattributed                       0.06    0.2%
  (sum)                             29.97
  rust-cache restore: 20.7 s, 615 MB, hit
  Nx sidecar: stored 0 entries (0 bytes), replayed 1
...
(GitHub reads this invocation: 0)
```

Figures read from it: test-linux wall 26.58 min with `unattributed` 0.08 min (0.3%); test-macos 29.97 min, compile-link(plain) 7.49 min (25.0%).

A cold build is read the same way, on the same commit (`CIPROFILE_OFFLINE=1 CIPROFILE_CACHE=<cache> python3 spike/ci-profile/ciprofile.py compile 43422893836`, exit 0), first lines:

```
run 43422893836  head 107be0519450  branch spike-otg-cctl-2  created 2026-11-13T12:10:09Z

== test-linux (job 131876998357, success) wall 7.82 min
  build step: 6.40 min
  cargo timings: 457 units, span 6.40 min, summed 23.49 min
    onetaskgraph-core                           188.4 s
    onetaskgraph-github-projects                105.6 s
    onetaskgraph-e2e                             43.5 s
```

Figure read from it: today's layout builds every test target cold on Linux in 6.40 min, `onetaskgraph-core` taking 188.4 s of it.

## Method

**Sources.** Hosted runs of `ci.yml`, read through REST only (`gh api repos/...`). For each run the harness reads the jobs listing, each test job's log once, and its `nx-cache-entries-<job>` (or `spike-nx-executions-<job>`) `executions.json`. Every read is cached under `$CIPROFILE_CACHE` and refused below 2,500 REST or 2,000 GraphQL remaining. Local work used only counts and resolved inputs (`nx show target inputs`); nothing was timed on this host.

**How a job's wall clock is divided** (`spike/ci-profile/jobprofile.py`).
- Every second of the job, from the jobs API's `started_at` to `completed_at`, is assigned once.
- Steps outside the test step are classed setup, cache-restore, cache-save or report. The restore inside setup-uv, setup-bun and Swatinem/rust-cache is read from the log's own groups. The background Nx sidecar step owns no second.
- Inside the test step, each instant is split equally among the Nx tasks running then, from `executions.json` (millisecond start and end) or else the log's `> nx run` headers. An instant with no task is `nx-planning`.
- Inside an executed task, time is classed as:
  - compile-link (dev/plain, instrumented for `coverage` and `coverage-build`, release for the distribution journeys' maturin build): the task's own `Compiling`…`Finished <profile>` windows, or the time until its first test starts;
  - `wait:cargo-file-lock`: `Blocking waiting for file lock` until the task's next line;
  - test-run: the rest.
- A replayed task is `cache-restore(nx)`.
- So concurrent tasks share and nothing is counted twice. `unattributed` is the remainder, at most 0.3% on every cited job.

**The recorder** for categories the logs cannot see: `sampler.py` reads psutil's process table each tick and records leaf-process categories, CPU and lifetimes. These cover the CLI under test, git, Python and uv, node, bun, onevcs, gh, curl/wget, tar, rustc, the linkers, llvm-cov and the test binaries. A tick where work processes used under 10% of one CPU is a wait (sleep, poll, network or timeout). `testrunner.py` is a cargo target runner that records each libtest binary's per-test `exec_time` through `--report-time`. Both run only on a demonstration (`measurement_only.py`).

**Population.**
- Durations: every successful test job of completed non-release PR runs created 2026-11-03 to 2026-11-09T23:14Z (before #3956 merged) against 2026-11-09T23:15Z to 2026-11-13. That is `ciprofile.py durations` over 128 runs.
- Reuse: the 30 most recent merged non-release PRs, #3881 to #3993, each PR's *first* CI run, with its Nx state as main left it.
- Typical PR: one whose squash diff changes a crate's `src/`, `tests/`, `build.rs` or `Cargo.toml`. That is 18 of the 30; the other 12 change only CI, scripts or docs.

**Runs this spike started.** Every one carries every cache write disabled by `measurement_only.py`: uv `save-cache: false`, setup-bun keeps no cache, every Rust `cache-save-if: false`, both `actions/cache/save` steps `if: false`, the Nx sidecar `CACHE_READ_ONLY: '1'`, and the Nx report uploaded under a name no promotion reads. Each test job's log shows it: in all 38 finished job logs of the 13 runs below there is no `Cache saved`/`Saving cache` line, `CACHE_READ_ONLY` is set, and the harness reads `Nx sidecar: stored 0 entries (0 bytes)` (counted over the cached logs; listing in the table). The only `Cache not found` lines are the `release-plz` and `prune-superseded` tool caches, whose save step is the disabled `actions/cache/save`. No cache entry was deleted. No run was cancelled by this spike.

- Session draft #3995: every commit carries `[skip ci]`, so it started no run (the Actions listing of runs created after 08:00Z on 2026-11-13, read 11:24Z, holds none on the session branch).
- Earlier demonstrations, already reported and closed: #3996 run 43369924587 (all three jobs hit the shipped 40-minute timeout; cited for recorder evidence only), #3997 run 43378506851 and #3998 run 43378508844 (closed 07:54Z). In 43378506851 the recorder leaked `RUSTC_BOOTSTRAP=1` into the cargo a test starts, so any cargo a test starts saw a different environment and rebuilt; that, rather than the sampler, is the likeliest cause of that run's several-fold slowdown of the loopback suites (an inference: the recorded runs after the fix, `eebb3129`, cost the measured amounts below). 43378508844 set `CARGO_PROFILE_RELEASE_INCREMENTAL` at job level, which rust-cache folds into its key, so every restore **missed**; `measurement_only.py --step-env` now exports a prototype's variable inside the test step only.
- **The prototype set, #3999 to #4008** (all read and closed with their branches deleted, #3999 to #4007 at 11:45Z and #4008 at 11:46Z; no DO NOT MERGE pull request of this spike is open), each `DO NOT MERGE: spike-otg-<side>-<n>`, based on the session branch, each one demonstration commit ahead of it that adds a comment line to `crates/onetaskgraph-core/src/lib.rs` (a core-touching PR, the typical PR's pattern) and the measurement-only `ci.yml`. Sides: *control* (no recorder), *dev opt-level 1* (`CARGO_PROFILE_DEV_OPT_LEVEL=1`), *RUST_TEST_THREADS=8* (control plus that variable), *recorder* (sampler and libtest runner), *recorder + incremental release* (`CARGO_PROFILE_RELEASE_INCREMENTAL=true`). Outside `spike/` and `ci.yml` every head's tree equals the control's (`git diff 1abf906b <head> -- ':!spike'` lists `.github/workflows/ci.yml` alone, or nothing); inside `ci.yml` each side differs from its control by its one exported variable (or the recorder), and #4007/#4008's first heads also passed `spike-ci-profile` to `--exclude` (their second heads, `25c57cb1`/`b9504265`, do not; that project has no Rust task). The executed `ci.yml` blob is the head's own, because the session branch does not touch `ci.yml`. Nx selection: what `NX_BASE=<session head>` selects for that lib.rs comment, less the workflow-only projects passed to `--exclude` (github-cache-policy, github-nx-policy, github-workflow-policy, github-workflows, linear-documents-policy, report-job, screenshots, scripts); per-job counts in the table. Every Rust cache restore hit. macOS and Windows end `failure` on every run because `github-workflow-policy:rust-cache-action` (20 s) refuses the demonstration's edited `ci.yml`; the job still ran every task, so the wall clock is whole. In 43394779047 one macOS `onetaskgraph-github-projects-e2e` test also failed (490 passed). Linux ends `failure` where `budgets` refused the live lane's figures, and in the two control runs because a credentialed live GitHub test failed: in 43390706286 the asset journey after 887 s (`HTTP 404` verifying an uploaded asset), in 43392805774 the contract journey after 11 s (`error sending request for url (https://api.github.com/graphql)`), which skipped the live e2e task behind it. **So Linux's wall clock in these runs depends on the live lane's outcome on the shared allowance, and no Linux difference below is evidence for or against an option.**

| PR | Run | Side | Head | `GITHUB_SHA` (merge) | ci.yml blob | Rust cache restore L / M / W (s, size, hit) | Nx entries stored L / M / W | Nx tasks run/replayed L / M / W |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| #4003 | 43390706286 | control (no recorder) | `1abf906b` | `c7a0387d` | `aeef8422` | 22.3 s 1362 MB hit / 8.9 s 615 MB hit / 18.7 s 613 MB hit | 0 / 0 / 0 | 34/11 / 23/1 / 23/1 |
| #4004 | 43392805774 | control (no recorder) | `954e2961` | `517afb6c` | `aeef8422` | 35.2 s 1362 MB hit / 12.0 s 615 MB hit / 16.9 s 613 MB hit | 0 / 0 / 0 | 35/10 / 23/1 / 23/1 |
| #4005 | 43392807274 | dev opt-level 1 | `2f5b60ec` | `fb953eb6` | `aff843af` | 29.9 s 1362 MB hit / 23.5 s 615 MB hit / 18.3 s 613 MB hit | 0 / 0 / 0 | 35/10 / 23/1 / 23/1 |
| #4006 | 43392811473 | dev opt-level 1 | `16629557` | `fcd96771` | `aff843af` | 32.6 s 1362 MB hit / 16.0 s 615 MB hit / 16.7 s 613 MB hit | 0 / 0 / 0 | 35/10 / 23/1 / 23/1 |
| #4007 | 43394779047 | RUST_TEST_THREADS=8 | `21734321` | `22a6f582` | `44cb9ec8` | 19.8 s 1362 MB hit / 21.7 s 615 MB hit / 17.5 s 613 MB hit | 0 / 0 / 0 | 35/10 / 22/1 / 23/1 |
| #4007 | 43394820753 | RUST_TEST_THREADS=8 | `25c57cb1` | `89646701` | `5b71a0f5` | 31.0 s 1362 MB hit / 26.5 s 615 MB hit / 13.4 s 613 MB hit | 0 / 0 / 0 | 35/10 / 22/1 / 23/1 |
| #4008 | 43394772150 | RUST_TEST_THREADS=8 | `6d741710` | `4c1562e2` | `44cb9ec8` | 22.8 s 1362 MB hit / 28.9 s 615 MB hit / 16.3 s 613 MB hit | 0 / 0 / 0 | 35/10 / 22/1 / 23/1 |
| #4008 | 43394820160 | RUST_TEST_THREADS=8 | `b9504265` | `e7fad71a` | `5b71a0f5` | 30.3 s 1362 MB hit / not finished / 16.0 s 613 MB hit | 0 / — / 0 | 35/10 / — / 22/1 |
| #3999 | 43390686747 | recorder (sampler + test runner) | `b5dddd51` | `76ea34b6` | `bf0c64e2` | 17.0 s 1362 MB hit / 21.2 s 615 MB hit / 14.8 s 613 MB hit | 0 / 0 / 0 | 34/11 / 23/1 / 23/1 |
| #3999 | 43394638992 | recorder (sampler + test runner) | `b5dddd51` | `51e0edb6` | `bf0c64e2` | 30.9 s 1362 MB hit / 18.2 s 615 MB hit / 15.4 s 613 MB hit | 0 / 0 / 0 | 35/10 / 23/1 / 23/1 |
| #4000 | 43390692001 | recorder (sampler + test runner) | `e63933bc` | `0f977a4b` | `bf0c64e2` | 34.3 s 1362 MB hit / 20.7 s 615 MB hit / 18.6 s 613 MB hit | 0 / 0 / 0 | 34/11 / 23/1 / 23/1 |
| #4001 | 43390696539 | recorder + incremental release | `67de2ee0` | `e12c99f0` | `dfcc6580` | 30.7 s 1362 MB hit / 13.4 s 615 MB hit / 19.0 s 613 MB hit | 0 / 0 / 0 | 34/11 / 23/1 / 23/1 |
| #4002 | 43390701482 | recorder + incremental release | `581b9d1f` | `42e0c0ca` | `dfcc6580` | 43.5 s 1362 MB hit / 21.7 s 615 MB hit / 16.7 s 613 MB hit | 0 / 0 / 0 | 34/11 / 23/1 / 23/1 |

**The pyramid and crate-split set, #4010 to #4019** (all read, and closed with their branches deleted by 14:35Z). Each is session head `18ad12b7` plus one commit built as `spike/ci-profile/prototypes/README.md` states: the measurement-only `ci.yml` without recorders (control blob `aeef8422`, byte-identical to #4003/#4004's control), the same lib.rs comment, and the prototype's patch. `git diff spike-otg-wctl-1 spike-otg-wpyr-1` is the three journey files alone (121 `#[ignore]` lines), `git diff spike-otg-wctl-1 spike-otg-wsplit-1` the split's 51 files alone, and each `-1`/`-2` pair is tree-identical. The cold sides add `--cold-rust --compile-only` (blob `c4a5501c`): the toolchain step's `cache: false` restores and saves no Rust cache, and the gate is one `cargo test --workspace --all-features --locked --no-run --timings`. Every job log shows no cache save, `CACHE_READ_ONLY` set and the sidecar storing 0 entries. The only `Cache not found` lines are the two tool caches, whose save is disabled. Nx selection is what `NX_BASE=18ad12b7` selects for each commit, less the same `--exclude` list; the split's own runs select more, because they change the workspace manifests.

| PR | Run | Side | Head | `GITHUB_SHA` (merge) | ci.yml blob | Rust cache restore L / M / W | Nx entries stored L / M / W | Nx tasks run/replayed L / M / W |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| #4010 | 43422783410 | warm control 1 | `e59361fe` | `bf580d77` | `aeef8422` | 36.6 s, hit / 25.9 s, hit / 15.9 s, hit | 0 / 0 / 0 | 35/10 / 23/1 / 23/1 |
| #4015 | 43422844825 | warm control 2 | `2e2676b4` | `4eed3240` | `aeef8422` | 30.8 s, hit / 13.6 s, hit / 17.4 s, hit | 0 / 0 / 0 | 35/10 / 23/1 / 23/1 |
| #4011 | 43422794958 | pyramid 1 | `8f14afd4` | `820326d0` | `aeef8422` | 31.1 s, hit / 17.6 s, hit / 17.5 s, hit | 0 / 0 / 0 | 35/10 / 23/1 / 23/1 |
| #4016 | 43422857812 | pyramid 2 | `0c48fdc5` | `6603393d` | `aeef8422` | 34.9 s, hit / 16.2 s, hit / 17.7 s, hit | 0 / 0 / 0 | 35/10 / 23/1 / 23/1 |
| #4012 | 43422809261 | split 1 | `f6a907a1` | `e8287b68` | `aeef8422` | 18.4 s, hit / 11.1 s, hit / 14.8 s, hit | 0 / 0 / 0 | 47/3 / 34/1 / 34/1 |
| #4017 | 43422873045 | split 2 | `d6a90891` | `ff64ab96` | `aeef8422` | 18.9 s, hit / 17.3 s, hit / 21.1 s, hit | 0 / 0 / 0 | 47/3 / 34/1 / 34/1 |
| #4013 | 43422822720 | cold control 1 | `21aae37a` | `f8bba987` | `c4a5501c` | off (`cache: false`) on all three | 0 / 0 / 0 | none: one `cargo test --workspace --no-run --timings` |
| #4018 | 43422893836 | cold control 2 | `107be051` | `bb237346` | `c4a5501c` | off (`cache: false`) on all three | 0 / 0 / 0 | none: one `cargo test --workspace --no-run --timings` |
| #4014 | 43422834977 | cold split 1 | `85161bce` | `9f25a76c` | `c4a5501c` | off (`cache: false`) on all three | 0 / 0 / 0 | none: one `cargo test --workspace --no-run --timings` |
| #4019 | 43422905157 | cold split 2 | `4230e262` | `8a286849` | `c4a5501c` | off (`cache: false`) on all three | 0 / 0 / 0 | none: one `cargo test --workspace --no-run --timings` |

**The `--job-exclude` split set, #4020 to #4023, superseded** (all read, and closed with their branches deleted at 15:38Z): its Linux coverage still scheduled two extra units, its split side executed ten tasks the control replayed, and its split side's distribution journeys failed early; it is kept as cited, not compared. Each is session head `f146009e` plus one commit: the control (#4020, #4022: ci.yml blob `aeef8422`, byte-identical to every earlier control) or the split patch with `--job-exclude` holding each job to the control's selection (#4021, #4023: blob `f701d81a`, which differs from the control's only on the three `--exclude=` lines). `git diff spike-otg-mctl-1 spike-otg-msplit-1` is the split's 51 files and those three lines; each `-1`/`-2` pair is tree-identical. Every Rust cache restore hit; every job log shows no cache save, `CACHE_READ_ONLY` set and the sidecar storing 0 entries.

| PR | Run | Side | Head | `GITHUB_SHA` (merge) | ci.yml blob | Rust cache restore L / M / W | Nx entries stored L / M / W | Nx tasks run/replayed L / M / W |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| #4020 | 43445538503 | matched control 1 | `43c02938` | `72b79404` | `aeef8422` | 29.7 s, hit / 24.9 s, hit / 16.9 s, hit | 0 / 0 / 0 | 35/10 / 23/1 / 23/1 |
| #4022 | 43445570366 | matched control 2 | `b9b8ae95` | `68ffe04e` | `aeef8422` | 17.5 s, hit / 13.5 s, hit / 17.3 s, hit | 0 / 0 / 0 | 35/10 / 23/1 / 23/1 |
| #4021 | 43445553591 | matched split 1 | `5f791781` | `ddbeb7be` | `f701d81a` | 33.7 s, hit / 15.6 s, hit / 15.9 s, hit | 0 / 0 / 0 | 45/2 / 24/0 / 24/0 |
| #4023 | 43445585382 | matched split 2 | `5fd41f22` | `4859edd8` | `f701d81a` | 33.8 s, hit / 16.0 s, hit / 18.9 s, hit | 0 / 0 / 0 | 45/2 / 24/0 / 24/0 |

**The matched split set, #4024 to #4027** (read and closed with their branches deleted: the controls at 17:47Z, the split runs at the 19:30Z cut-off of manager ruling a17). Each is session head `7029a947` plus one commit: the measurement-only `ci.yml` with `--marker-base --step-env=NX_SKIP_NX_CACHE=true` (blob `84938113` on all four, byte-identical) and the marker comment, plus, on the split side, `split-template-crate.patch`. `git diff spike-otg-nctl-1 spike-otg-nsplit-1` is the split's 51 files and nothing in `.github`; each `-1`/`-2` pair is tree-identical. Every Rust cache restore hit; every job stored 0 Nx entries and replayed none; no job log holds a cache save.

| PR | Run | Side | Head | ci.yml blob | Rust cache restore L / M / W | Nx entries stored L / M / W | Nx tasks run/replayed L / M / W |
| --- | --- | --- | --- | --- | --- | --- | --- |
| #4024 | 43457659504 | matched control 1 | `8635ef79` | `84938113` | 24.7 s, hit / 22.9 s, hit / 21.1 s, hit | 0 / 0 / 0 | 37/0 / 22/0 / 22/0 |
| #4026 | 43457450179 | matched control 2 | `89baeab0` | `84938113` | 30.2 s, hit / 13.6 s, hit / 16.6 s, hit | 0 / 0 / 0 | 38/0 / 22/0 / 22/0 |
| #4025 | 43457723269 | matched split 1 | `00548424` | `84938113` | 24.5 s, hit / not read / 19.4 s, hit | 0 / — / 0 | 38/0 / — / 22/0 |
| #4027 | 43457727138 | matched split 2 | `5dae86e5` | `84938113` | 29.0 s, hit / 7.8 s, hit / 14.9 s, hit | 0 / 0 / 0 | 38/0 / 22/0 / 22/0 |

Control 1's Linux job ran 37 tasks, not 38: its credentialed Linear journey failed against the real Linear API, so `onetaskgraph-linear-e2e:test` behind it did not run.

**The repaired split runs, #4030/#4031 and #4034/#4035.** All four were read and closed with their branches deleted, #4030/#4031 at 21:30Z and #4034/#4035 at 00:20Z, inside manager rulings a18 (cut-off 23:00Z) and a19 (cut-off 01:00Z). Each is session head `7029a947` plus one commit: the repaired `split-template-crate.patch`, the marker comment, and the same measurement-only `ci.yml` as the controls.
- #4030/#4031 carry blob `84938113`, byte-identical to the controls'. `git diff 00548424 6604aade` is the five release and coverage files of the first repair.
- #4034/#4035 also carry the coverage-read-set fix and `if: false` on `test-macos` and `test-windows`, blob `d935e7a1`; `git diff 6604aade 5bde5594` is `scripts/coverage-read-sets.json` and those two lines.
- Their pull-request merge refs join each head to the session branch as it then stood (`4370feaa`, `eefb19c3`), which differs from the controls' only under `spike/`, which no gate task reads.
- #4030/#4031's Linux jobs are discarded (their coverage-read-set check failed); their macOS and Windows jobs, and #4034/#4035's Linux jobs, are compared.
- Every Rust cache restore hit; every job stored 0 Nx entries, replayed none and logged no cache save.

| PR | Run | Head | ci.yml blob | Jobs read | Rust cache restore | Nx tasks run/replayed | Conclusion |
| --- | --- | --- | --- | --- | --- | --- | --- |
| #4030 | 43504759508 | `6604aade` | `84938113` | L (discarded) / M / W | 17.1 s / 13.3 s / 14.5 s, hits | 38/0 / 22/0 / 22/0 | L failed (prototype), M success, W failed in final cleanup |
| #4031 | 43504773180 | `0514e75a` | `84938113` | L (discarded) / M / W | 28.0 s / 18.1 s / 17.5 s, hits | 38/0 / 22/0 / 22/0 | L failed (prototype), M success, W success |
| #4034 | 43528710535 | `5bde5594` | `d935e7a1` | L | 20.5 s, hit | 39/0 | success |
| #4035 | 43528719773 | `7f79996c` | `d935e7a1` | L | 18.3 s, hit | 39/0 | success |

**GitHub API accounting (this dispatch, 11:24Z onward).** One run listing; 6 REST jobs reads of the two `spike-otg-threads-2` runs (at 11:32Z, 11:33Z and 11:44Z: the first two a minute apart, inside the 10-minute spacing the manager asked for; noted, not repeated); 103 harness reads profiling the ten runs not yet read (7 to 12 per run); 10 `gh pr close --delete-branch` (#3999 to #4008); one question to the manager (ruling a8); one `onevcs publish --draft`, which pushed `1fa3bd49` onto #3995 (every commit `[skip ci]`). Its first two attempts were refused by the public-boundary gate: a recorded run listing (`spike/ci-profile/data/github/runs-listing-page.json`, added in an unpublished commit) carried a commit author's address naming a private repository. The unpublished commits after `d2b6925f` (the published head) were rewritten with that address replaced by a neutral one, leaving every tree otherwise byte-identical, so their ids changed (for example `2ca52176` is now `b3c574eb`); commits up to `d2b6925f`, and the demonstration heads built on them, are unchanged; `gh pr list` ×3 and `gh pr view` ×6 (GraphQL); free `rate_limit` checks. For the pyramid and split set: 10 pushes, 10 `gh pr create` and 10 `gh pr close --delete-branch` (GraphQL), 15 status listings at least 10 minutes apart from 12:12Z to 14:28Z, 54 harness reads profiling the warm runs and about 40 reading the cold ones (three of them read twice, after the extraction fix in `95ec845e` kept their timings report). For the matched set: 4 pushes, 4 `gh pr create`, 4 `gh pr close --delete-branch`, 5 status listings at least 10 minutes apart (14:52Z to 15:33Z), 36 harness reads. For the matched split set: 4 pushes, 4 `gh pr create`, 4 `gh pr close --delete-branch`, status listings at least 10 and, after ruling a17, 20 minutes apart, 34 harness reads, one REST read of run 43457723269's jobs at the cut-off and one question to the manager (ruling a17). Reading the population's task graphs fetched one PR head (`refs/pull/<n>/head`) and the four demonstration heads through git, not the API. For the repaired split runs: 4 pushes, 4 `gh pr create`, 4 `gh pr close --delete-branch`, 4 status listings at least 20 minutes apart, 28 harness reads, and two questions to the manager (rulings a18, a19). REST stayed above 3,200 remaining and GraphQL above 3,600 throughout; at 12:05Z the shared REST allowance read 3,256 remaining, drawn down by other consumers. Earlier dispatches' accounting is unchanged from the previous revision of this report: about 224 REST reads 05:15Z to 06:00Z, a breach of the floor of 4 reads at 06:41Z (made after 1,212 remained), and the reads after the 07:20Z reset. The final pass under manager ruling c11.1 (00:28Z on 2026-11-14) launched no run, opened no demonstration and made two GitHub reads: one free `gh api rate_limit` check, which read 4,852 of 5,000 core requests remaining, and one `gh pr list --state open --search "DO NOT MERGE"` (GraphQL), which went beyond that ruling's allowance and found no demonstration change request of this repository open; it re-ran the harness offline on `697233f3` (`(GitHub reads this invocation: 0)`) and changed only this report's wording of the split's result.

## Candidate budgets

This spike measured no candidate budget itself.

## Design findings

- **Wall clock of a core-touching PR, divided, on complete jobs carrying the recorder** (run 43390692001, #4000, `GITHUB_SHA` `0f977a4b`; harness output on `697233f3`; a second recorded run, 43394638992, agrees within the spread below). Two exclusive views, each summing to the job's wall clock, so no minute is counted twice: *by phase* (every second of the job assigned once; concurrent Nx tasks share each instant equally) and, inside the test step, *by leaf process* (each one-second sampler tick shared among the leaf processes then running).

*By phase* (minutes, share):
- **test-linux 26.58 min**: test-run(instrumented) 8.63 (32.5%); test-run(plain) 6.42 (24.1%); compile-link(plain) 3.19 (12.0%); wait:cargo-file-lock 1.97 (7.4%); compile-link(instrumented) 1.72 (6.5%); scripts and other tasks 1.31 (4.9%); setup 1.24 (4.7%); budget checks 1.02 (3.8%); cache restore 0.62 (2.3%); Nx planning 0.28 (1.1%); report 0.08; cache save 0.02; Nx remote-cache restore 0.00 (11 replays, under 1 s each); **unattributed 0.08 (0.3%)**.
- **test-macos 29.97 min**: test-run(plain) 7.89 (26.3%); compile-link(plain) 7.49 (25.0%); scripts and other tasks 6.06 (20.2%; `scripts:distribution-test` alone 5.26); compile-link(release) 3.50 (11.7%); budget checks 2.25 (7.5%); setup 1.98 (6.6%); cache restore 0.42 (1.4%); Nx planning 0.25; report 0.07; cache save 0.00 (none); **unattributed 0.06 (0.2%)**.
- **test-windows 33.85 min**: test-run(plain) 17.28 (51.0%); compile-link(plain) 6.44 (19.0%); scripts and other tasks 3.74 (11.0%); setup 2.86 (8.4%); compile-link(release) 1.88 (5.5%); budget checks 0.82 (2.4%); cache restore 0.41 (1.2%); Nx planning 0.28; report 0.10; cache save 0.00 (none); **unattributed 0.05 (0.1%)**.
- *Coverage overhead* is Linux's alone: compile-link(instrumented) 1.72 + test-run(instrumented) 8.63 = 10.35 min (38.9%), of which `llvm-cov`/profdata reporting is 1.17 min by process. macOS and Windows: a measured zero (their selection holds no `coverage` task; `ci-job.sh` measures on Linux alone).
- *Per Nx task and the latest-finishing chain* (wall share; the chain is a timeline description, walking back from the gate's end through the task still running as each later one began, not a dependency path): Linux `onetaskgraph-github-projects-e2e:test` 4.38 min (1,302 s own), `onetaskgraph-github-projects:test` 3.88, `onetaskgraph-e2e:coverage` 2.75; latest-finishing chain `onetaskgraph:build` → `onetaskgraph-github-projects-e2e:test` (the live lane, 21.7 min) → its `budgets`, 24.13 min. macOS `scripts:distribution-test` 5.26 min (1,287 s own), `onetaskgraph-github-projects-e2e:test` 2.67, `onetaskgraph-linear-e2e:test` 2.59; chain `onetaskgraph-github-projects:test` → `onetaskgraph-github-projects-e2e:test` → `sdk-python:test` → `workspace:test`, 26.91 min. Windows `onetaskgraph-e2e:test` 6.22 min (1,681 s own), `scripts:distribution-test` 5.02, `onetaskgraph-linear-e2e:test` 4.19; chain `onetaskgraph-github-projects:test` → `sdk-typescript:generate-check` → `onetaskgraph:test` → `sdk-python:test` → `onetaskgraph-status-options-e2e:test` → `sdk-typescript:test`, 29.90 min.

*By leaf process, inside the test step* (minutes of the job; the step is 24.52 / 27.42 / 30.42 min on Linux / macOS / Windows and the categories sum to it, every tick assigned):
- rustc (compile): 3.98 / 4.96 / 3.00; linker: 0.27 / 0.24 / 0.47; cargo itself: 3.65 / 4.22 / 3.17.
- test binaries running their own code: 3.64 / 0.48 / 1.74; **sleeps, polls and timeouts inside tests** (ticks where the work processes used under 10% of one CPU): 1.48 / 0.02 / a measured zero (no such tick on Windows), almost all Linux's live session pausing on the shared allowance.
- **spawns of other CLIs**: the CLI under test 2.54 / 1.83 / 4.05 (3,013 / 969 / 5,716 processes); Python and uv 3.40 / 3.10 / 6.25 (2,413 / 721 / 1,961); node and npm 0.01 / 7.99 / 6.25 (1 / 547 / 154); bun 3.79 / 3.51 / 0.64; git 0.20 / 0.25 / 1.70 (40 / 62 / 919); onevcs 0.01 / 0.02 / 2.03 (4 / 6 / 1,167); network clients (curl, wget) 0.00 / 0.01 / 0.02; tar and zip 0.01 / 0.10 / 0.07; doctests 0.02 / 0.03 / 0.03.
- **fixtures**: the Python document-store peer is inside the Python figure; the loopback GitHub and Linear stand-ins are threads of the test binaries, so they are inside the test-binary figure and cannot be told apart from the test's own code by a process sampler.
- waits on a child (a parent whose only running children are named): Linux 0.33, macOS 0.68, Windows 1.01.

*Steps outside the test step* are the phase view's setup, cache restore, cache save and report rows. The rust-cache restore itself: 34.3 s of 1,362 MB (Linux), 20.7 s of 615 MB (macOS), 18.6 s of 613 MB (Windows), all hits; cache save is a measured zero here because the demonstration disables it, and on ordinary PR runs it is 0.0 to 0.1 min (run 43152201170: 0.00 / 0.07 / 0.02 min), because only `main` saves Rust caches.

*The recorder's own cost*, measured: recorder against control means +1.3 / +5.9 / +2.9 min on Linux / macOS / Windows; macOS's is inside its 13.9-minute control spread, Linux's inside 5.5; Windows' is under its 5.3 spread but all three recorded Windows runs sit at 33.1 to 33.9 min against the control's 27.8 and 33.1, so up to about 3 min of a Windows recorded job may be the recorder's (estimate). The divisions above are of a recorded job.
- **The 20 slowest tests per platform and every test binary's seconds** (run 43390692001, harness output on the finished commit; names are cut at the harness's print width; the full per-binary list is in its `--json`).

```
test-linux:
  slowest tests (of 3821):
      953.1s  live-57f9f5cdc3e78625::disposable_task_and_document_asset_copies
      912.3s  live-62ad363e434f2ff9::real_projects_v2_contract_writes_and_leaves_no_residue
      363.3s  instrumented:onetaskgraph-github-projects-e2e::e2e::cache_budget::the_cache_spends_no_more_board_points_than_n
      355.0s  instrumented:onetaskgraph-linear-e2e::e2e::cache_budget::the_cache_sends_fewer_of_the_linear_keys_requests_in_
      154.5s  instrumented:onetaskgraph-github-projects-e2e::e2e::plan_copy::a_plan_copy_reads_what_the_plan_holds_whatever_
       79.1s  instrumented:onetaskgraph-e2e::e2e::held_writes::every_task_write_verb_held_to_a_version_lands_at_it_and_is_re
       72.2s  instrumented:onetaskgraph-github-projects-e2e::lane_wiring::budget_adapter_uses_the_real_checker_and_never_sub
       61.4s  sweep_gate-5a3d182d0f5a038d::a_presence_read_that_settles_nothing_fails_the_cleanup
       60.2s  instrumented:onetaskgraph-github-projects-e2e::e2e::assets::asset_requests_take_mutation_slots_and_each_limite
       60.0s  instrumented:onetaskgraph-github-projects-e2e::e2e::human_cost::a_hundred_tasks_from_ten_managers_are_projecte
       53.4s  instrumented:onetaskgraph-github-projects-e2e::e2e::plan_pacing::filling_the_minutes_points_with_queries_alone
       48.3s  instrumented:onetaskgraph-e2e::e2e::section_links::public_copies_keep_sections_and_screen_private_record_links
       48.2s  instrumented:onetaskgraph-live-janitor::journeys::offline_linear_cleanup_journeys
       46.5s  instrumented:onetaskgraph-e2e::e2e::section_links::every_pair_of_kinds_writes_the_others_section_links_in_its_
       41.7s  run_board_gate-65f133c51362dacd::a_session_naming_a_board_outside_its_run_is_refused_before_sending
       38.8s  run_board_gate-65f133c51362dacd::concurrent_lifecycles_made_to_use_one_board_fail
       38.8s  sweep_gate-5a3d182d0f5a038d::an_issue_delete_github_never_answers_fails_the_cleanup_after_its_attempts
       38.2s  run_board_gate-65f133c51362dacd::eight_runs_at_once_stay_inside_the_hourly_points_hourly_creations_and_board_f
       37.9s  e2e-093be27dc4df3c09::linear_assets::live_disposable_issue_and_document_assets_copy_and_recopy
       37.6s  run_board_gate-65f133c51362dacd::a_copy_made_after_another_sessions_copy_was_deleted_never_reuses_its_number

test-macos:
  slowest tests (of 3278):
      243.0s  e2e-f21661ddefeb1e08::cache_budget::the_cache_spends_no_more_board_points_than_no_cache_over_every_kind_sequen
      158.2s  e2e-3338b72731d1100b::cache_budget::the_cache_sends_fewer_of_the_linear_keys_requests_in_an_hour_than_no_cache
      143.5s  e2e-f21661ddefeb1e08::plan_copy::a_plan_copy_reads_what_the_plan_holds_whatever_the_board_has_accumulated
       87.7s  e2e-f21661ddefeb1e08::human_cost::a_hundred_tasks_from_ten_managers_are_projected_recovered_and_resolved_once_
       55.1s  e2e-3338b72731d1100b::cache_parts::every_route_to_a_tasks_comments_is_served_warm_and_read_fresh_on_a_newer_or
       41.6s  e2e-3338b72731d1100b::assignees::two_hundred_fifty_two_unseen_names_are_resolved_in_one_lookup_that_answers_ea
       33.5s  e2e-f21661ddefeb1e08::plan_pacing::filling_the_minutes_points_with_queries_alone_holds_the_next_query
       32.2s  plugin-5e3e350e9fe048dc::the_change_kind_leg_asks_again_while_a_token_is_withheld
       29.5s  e2e-f21661ddefeb1e08::assets::measure_single_and_three_concurrent_project_copies_in_virtual_time
       29.2s  lane_wiring-3a81eb772e661f3a::budget_adapter_uses_the_real_checker_and_never_substitutes_missing_live_evidence
       28.6s  e2e-f21661ddefeb1e08::assets::asset_requests_take_mutation_slots_and_each_limiter_recovers_or_exhausts_its_bud
       28.4s  e2e-f21661ddefeb1e08::plan_pacing::filling_the_hours_creations_holds_the_next_until_the_first_has_left_the_hou
       28.1s  e2e-3338b72731d1100b::linear_assets::a_refused_concurrent_copy_is_named_in_what_the_budget_check_reports
       27.6s  e2e-f21661ddefeb1e08::budget_runner::report_sanitize_complete_sweep
       24.3s  sweep_gate-2ca887d576480bac::a_presence_read_that_settles_nothing_fails_the_cleanup
       23.7s  schema-04841b9538cbcb2f::a_schema_check_server_stops_once_idle_and_never_while_a_client_is_connected
       23.1s  sweep_gate-2ca887d576480bac::an_artifact_another_deleter_took_first_leaves_the_cleanup_successful
       22.3s  e2e-f21661ddefeb1e08::human_reporters::the_approval_reporter_hands_over_the_larger_sequence_of_the_journeys_ca
       22.1s  sweep_gate-2ca887d576480bac::an_issue_delete_answered_without_confirmation_is_settled_by_the_repository
       22.1s  e2e-5232741c9111c88a::copy_retry::a_project_or_document_copy_stopped_at_either_point_of_its_create_is_finished

test-windows:
  slowest tests (of 3236):
      604.6s  e2e-366086ca01a7cdab.exe::cache_budget::the_cache_sends_fewer_of_the_linear_keys_requests_in_an_hour_than_no_c
      436.6s  e2e-b037a591c8ad43e8.exe::cache_budget::the_cache_spends_no_more_board_points_than_no_cache_over_every_kind_se
      205.6s  e2e-b037a591c8ad43e8.exe::plan_copy::a_plan_copy_reads_what_the_plan_holds_whatever_the_board_has_accumulated
      157.7s  e2e-b037a591c8ad43e8.exe::human_cost::a_hundred_tasks_from_ten_managers_are_projected_recovered_and_resolved_o
      121.1s  run_board_gate-ef173e0d8e1d72dd.exe::concurrent_lifecycles_made_to_use_one_board_fail
      121.0s  run_board_gate-ef173e0d8e1d72dd.exe::eight_runs_at_once_stay_inside_the_hourly_points_hourly_creations_and_boa
      119.7s  run_board_gate-ef173e0d8e1d72dd.exe::a_session_naming_a_board_outside_its_run_is_refused_before_sending
      113.1s  run_board_gate-ef173e0d8e1d72dd.exe::a_copy_made_after_another_sessions_copy_was_deleted_never_reuses_its_numb
       97.2s  run_board_gate-ef173e0d8e1d72dd.exe::a_copy_that_cannot_be_deleted_says_so_beside_the_failure_that_ended_it
       94.8s  e2e-b037a591c8ad43e8.exe::narrow_write_cost::every_narrow_write_under_a_sanitize_command_costs_what_it_costs_w
       86.5s  e2e-b037a591c8ad43e8.exe::budget_runner::report_sanitize_complete_sweep
       83.8s  run_board_gate-ef173e0d8e1d72dd.exe::a_copy_that_settles_short_is_deleted_and_the_template_copied_once_more
       83.6s  e2e-e3f511a2d6f27f71.exe::boundary::projections::a_private_linear_canonicals_public_view_keeps_its_authored_se
       79.4s  e2e-b037a591c8ad43e8.exe::plan_pacing::filling_the_hours_creations_holds_the_next_until_the_first_has_left_the
       75.6s  run_board_gate-ef173e0d8e1d72dd.exe::a_copy_waits_for_a_last_draft_its_listing_shows_a_minute_after_the_copy
       75.2s  e2e-b037a591c8ad43e8.exe::plan_pacing::filling_the_minutes_points_with_queries_alone_holds_the_next_query
       60.3s  e2e-366086ca01a7cdab.exe::plan_copy::an_unchanged_private_plan_with_repositories_and_a_dependency_sends_no_wri
       58.0s  e2e-e3f511a2d6f27f71.exe::assets::a_copy_into_a_plugin_serving_assets_rewrites_references_and_reuploads_only_c
       56.4s  e2e-e3f511a2d6f27f71.exe::boundary::projections::an_anchor_carrying_anything_but_its_own_name_is_refused_namin
       47.5s  e2e-e3f511a2d6f27f71.exe::boundary::projections::an_unrecorded_task_is_taken_as_a_view_only_when_it_alone_says
```

Every end-to-end test binary, wall seconds and summed test seconds (from the same run's `spike-profile-<job>` records; crate named by matching each binary's test modules to `crates/*/tests/`):

```
== test-linux (run 43390692001)
  onetaskgraph-e2e,onetaskgraph-github-projects-e2e,onetaskgraph-linear-e2e e2e-7903b8d102129eef                          wall  1805.7s  test-s      0.0  tests 1
  onetaskgraph-e2e,onetaskgraph-github-projects-e2e,onetaskgraph-linear-e2e e2e-cbf38a7a5a51ecca                          wall  1665.4s  test-s     10.5  tests 28
  onetaskgraph,onetaskgraph-github-projects,onetaskgraph-github-projects-e2e,onetaskgraph-linear live-57f9f5cdc3e78625                         wall   960.9s  test-s    953.1  tests 1
  onetaskgraph,onetaskgraph-github-projects,onetaskgraph-github-projects-e2e,onetaskgraph-linear live-62ad363e434f2ff9                         wall   913.2s  test-s    913.0  tests 15
  onetaskgraph-linear-e2e                e2e-093be27dc4df3c09                          wall   848.1s  test-s     38.0  tests 22
  onetaskgraph-github-projects-e2e       live_pair-3737784b2c0474bf                    wall    99.6s  test-s      0.0  tests 0
  onetaskgraph-github-projects-e2e       lane_wiring-faa9ed6c3c5a72c9                  wall    92.2s  test-s      0.0  tests 0
  onetaskgraph-e2e,onetaskgraph-github-projects-e2e,onetaskgraph-linear-e2e,onetaskgraph-status-options-e2e e2e-356c145d8d54a8ec                          wall     6.9s  test-s      0.0  tests 0
  onetaskgraph-core,onetaskgraph-e2e     configuration-b00f1009affa81c6                wall     5.5s  test-s      0.0  tests 0
  onetaskgraph-e2e                       incomplete_sources-4367d1bb28a0f0eb           wall     4.9s  test-s      0.0  tests 0
  onetaskgraph,onetaskgraph-github-projects,onetaskgraph-github-projects-e2e,onetaskgraph-linear live-bf06b35555cdf34c                         wall     2.7s  test-s      0.0  tests 0
  onetaskgraph-core,onetaskgraph-e2e     configuration-a8f10eebebc6a1ec                wall     2.5s  test-s      0.0  tests 0
  onetaskgraph-e2e                       ambient_credentials-fcd41dacb302db7c          wall     0.1s  test-s      0.0  tests 0
== test-macos (run 43390692001)
  onetaskgraph-github-projects-e2e       e2e-f21661ddefeb1e08                          wall   401.5s  test-s   1126.9  tests 519
  onetaskgraph-e2e                       e2e-5232741c9111c88a                          wall   321.5s  test-s    962.2  tests 1124
  onetaskgraph-linear-e2e                e2e-3338b72731d1100b                          wall   165.7s  test-s    455.7  tests 297
  onetaskgraph-github-projects-e2e       lane_wiring-3a81eb772e661f3a                  wall    29.2s  test-s     55.7  tests 4
  onetaskgraph-github-projects-e2e       live_pair-84267f50fca1b037                    wall    21.9s  test-s     64.7  tests 14
  onetaskgraph,onetaskgraph-github-projects,onetaskgraph-github-projects-e2e,onetaskgraph-linear live-c79c887e08abbc0e                         wall     5.6s  test-s      6.4  tests 14
  onetaskgraph,onetaskgraph-github-projects,onetaskgraph-github-projects-e2e,onetaskgraph-linear live-fc5bc8444cf2e685                         wall     2.4s  test-s      2.3  tests 1
  onetaskgraph-core,onetaskgraph-e2e     configuration-fb540783f2f58b68                wall     1.4s  test-s      3.4  tests 70
  onetaskgraph-status-options-e2e        e2e-bafce794a28a3b31                          wall     1.3s  test-s      3.8  tests 12
  onetaskgraph-e2e                       incomplete_sources-6be9c63e3ecb99a0           wall     0.6s  test-s      1.5  tests 9
  onetaskgraph,onetaskgraph-github-projects,onetaskgraph-github-projects-e2e,onetaskgraph-linear live-95c694d6517aeebc                         wall     0.6s  test-s      1.1  tests 15
  onetaskgraph-e2e                       ambient_credentials-8387c6551c897682          wall     0.4s  test-s      0.2  tests 1
  onetaskgraph-core,onetaskgraph-e2e     configuration-e24a616daa918892                wall     0.1s  test-s      0.2  tests 72
== test-windows (run 43390692001)
  onetaskgraph-e2e                       e2e-e3f511a2d6f27f71.exe                      wall  1189.9s  test-s   4660.5  tests 1113
  onetaskgraph-github-projects-e2e       e2e-b037a591c8ad43e8.exe                      wall   734.7s  test-s   2548.0  tests 515
  onetaskgraph-linear-e2e                e2e-366086ca01a7cdab.exe                      wall   644.3s  test-s   1394.0  tests 293
  onetaskgraph-github-projects-e2e       live_pair-265d226b93f9b81b.exe                wall    61.8s  test-s    231.8  tests 14
  onetaskgraph-github-projects-e2e       lane_wiring-641def72719ebdf3.exe              wall    20.0s  test-s     51.5  tests 4
  onetaskgraph-core,onetaskgraph-e2e     configuration-ca1dd1eee257375d.exe            wall    14.3s  test-s     45.0  tests 67
  onetaskgraph,onetaskgraph-github-projects,onetaskgraph-github-projects-e2e,onetaskgraph-linear live-a0fdeba6235cecc8.exe                     wall    13.6s  test-s     13.4  tests 1
  onetaskgraph,onetaskgraph-github-projects,onetaskgraph-github-projects-e2e,onetaskgraph-linear live-88de5792da721c31.exe                     wall     8.2s  test-s     16.0  tests 13
  onetaskgraph,onetaskgraph-github-projects,onetaskgraph-github-projects-e2e,onetaskgraph-linear live-573c24dc8ed11c98.exe                     wall     6.1s  test-s      9.8  tests 15
  onetaskgraph-status-options-e2e        e2e-559b6ded479dad96.exe                      wall     1.1s  test-s      3.8  tests 12
  onetaskgraph-e2e                       incomplete_sources-92728eac239f7f02.exe       wall     1.0s  test-s      3.7  tests 9
  onetaskgraph-e2e                       ambient_credentials-6b95bc0af8b49ef3.exe      wall     0.6s  test-s      0.5  tests 1
  onetaskgraph-core,onetaskgraph-e2e     configuration-8801c60522594d3c.exe            wall     0.3s  test-s      0.4  tests 69
```
On Linux the e2e crates run under coverage through nextest, so their binaries' seconds are the harness's `instrumented:` rows: `onetaskgraph-e2e::e2e` 2,108.8 s, `onetaskgraph-github-projects-e2e::e2e` 1,790.1 s, `onetaskgraph-linear-e2e::e2e` 867.9 s, `live_pair` 101.2 s, `lane_wiring` 93.0 s; the rows above with 0 tests on Linux are the recording runner's listing passes for those same binaries.
- **Before and after #3956** (`ciprofile.py durations`, successful jobs).
- All PRs: test-linux 32.5 → 27.7 min (n 39 → 10), test-macos 24.1 → 33.3 (66 → 19), test-windows 21.9 → 24.7 (66 → 18).
- Rust-touching PRs only: macOS 24.9 → 33.3 (41 → 17), Windows 25.0 → 24.7 (39 → 16), Linux 31.2 → 28.5 (25 → 8).
- So the Windows rise in the medians is the mix of PRs: before #3956 there were more CI-only PRs, which run no Rust suite. The macOS rise is real.
- **Cause of the macOS rise: log-based attribution, so the historical A/B was not run.** First runs of the 10 Rust-touching PRs before #3956 against the 8 after, mean macOS wall 23.4 → 33.7 min (+10.3), divided by wall share:
- test-run +3.8 min;
- scripts and other tasks +5.2 min: `scripts:test` +2.9 (selected by more of the later PRs) and `scripts:distribution-check` +0.8 (own time 2.5 → 6.4 min);
- plain compile and link +1.5 min; release compile -0.6 min.

Per task, `onetaskgraph-e2e:test` own time went 4.3 → 7.9 min, `onetaskgraph-github-projects:test` 5.3 → 8.8, and `onetaskgraph-linear:test` 3.8 → 5.4. The suites grew: macOS e2e tests run 1,064 (#3931) → 1,204 (#3984), Linear e2e 209 → 297, core 600 → 651. These are tests added by #3969 to #3984.

#3956 changed no macOS phase in `scripts/ci-job.sh`, did not touch `ci.yml`, and macOS replay stayed as low after it as before (Rust PRs before ran 10 to 34 tasks and replayed 0 to 15; after, 24 to 32 and 0 to 4). The runner image was macos-26-arm64 throughout.

The caching change's own share of the macOS rise is therefore about 0 min; the rise is test growth and selection mix. Linux moved to ubuntu-26.04 at #3956's own run (42656859552), and Linux's fall is mostly `github-workflows:cd-journeys` (own time 26.4 → 16.3 min) and Linux's distribution journey moving to `lint` (#3971).
- **Reuse over the 30 PRs' first runs** (`replay.py`, committed with its inputs in `spike/ci-profile/data/`).

Tasks run / replayed: Linux 1,211 / 166, macOS 605 / 39, Windows 617 / 26. Mean wall 30.7 / 25.8 / 24.1 min.

Executed wall share (attribution: where the minutes went, not how much shorter a job would be) by the most specific changed input matched, in minutes over the 30 PRs; each option's predicted job shortening is under Options:
- Linux: dependency source 208, not keyed on the diff 183, scripts 106, own tests 90, own source 58, `.github` 57.
- macOS: scripts 160, not keyed 135, dependency source 114, own tests 78.
- Windows: own tests 115, dependency source 114, not keyed 106, scripts 100.

"Not keyed on the diff" is mostly `scripts:distribution-test` on every PR (macOS 60 min over 12 PRs, Windows 49) and the live `onetaskgraph-github-projects:test`. It also includes e2e coverage units whose key moved only through a runtime input (the read-set digest or the cargo closure), which `nx show target inputs` does not list.

Per PR row (first run, per job):

| PR | Run | Conclusion | test-linux | test-macos | test-windows |
| --- | --- | --- | --- | --- | --- |
| #3993 | 43349781263 | success | 1 run / 0 replayed, 3.3 min | 3 run / 0 replayed, 18.1 min | 3 run / 0 replayed, 13.0 min |
| #3984 | 43142127197 | failure | 41 run / 10 replayed, 28.8 min | 27 run / 0 replayed, 33.2 min | 27 run / 0 replayed, 34.7 min |
| #3982 | 43077192372 | failure | 42 run / 2 replayed, 42.9 min | 31 run / 0 replayed, 40.6 min | 31 run / 0 replayed, 31.2 min |
| #3981 | 43046436056 | failure | 46 run / 3 replayed, 43.5 min | 30 run / 0 replayed, 23.5 min | 30 run / 0 replayed, 39.2 min |
| #3979 | 43000055371 | success | 29 run / 41 replayed, 25.5 min | 31 run / 5 replayed, 34.9 min | 31 run / 5 replayed, 30.2 min |
| #3977 | 42993507127 | failure | 41 run / 7 replayed, 27.8 min | 30 run / 0 replayed, 34.4 min | 30 run / 0 replayed, 29.1 min |
| #3976 | 42967701358 | failure | 49 run / 18 replayed, 33.5 min | 32 run / 4 replayed, 33.7 min | 32 run / 4 replayed, 37.8 min |
| #3972 | 42812145716 | failure | 41 run / 7 replayed, 23.9 min | 29 run / 0 replayed, 28.3 min | 29 run / 0 replayed, 32.2 min |
| #3971 | 42793675570 | failure | 41 run / 11 replayed, 41.0 min | 33 run / 3 replayed, 39.4 min | 32 run / 3 replayed, 29.9 min |
| #3969 | 42778025118 | failure | 40 run / 7 replayed, 31.9 min | 28 run / 0 replayed, 36.8 min | 28 run / 0 replayed, 35.0 min |
| #3956 | 42656859552 | failure | 24 run / 0 replayed, 21.9 min | 5 run / 0 replayed, 19.6 min | 5 run / 0 replayed, 12.2 min |
| #3940 | 42486624315 | success | 36 run / 1 replayed, 37.2 min | 7 run / 0 replayed, 18.3 min | 7 run / 0 replayed, 13.6 min |
| #3938 | 42482531959 | cancelled | 43 run / 2 replayed, 41.8 min | 11 run / 0 replayed, 18.5 min | 11 run / 0 replayed, 10.0 min |
| #3937 | 42477887564 | success | 43 run / 1 replayed, 36.1 min | 7 run / 0 replayed, 23.1 min | 7 run / 0 replayed, 13.8 min |
| #3931 | 42435141466 | failure | 51 run / 15 replayed, 37.0 min | 27 run / 7 replayed, 25.0 min | 27 run / 7 replayed, 24.0 min |
| #3928 | 42412580213 | failure | 38 run / 3 replayed, 33.9 min | 9 run / 0 replayed, 15.4 min | 9 run / 0 replayed, 10.8 min |
| #3926 | 42396913357 | success | 39 run / 5 replayed, 27.4 min | 10 run / 15 replayed, 15.7 min | 25 run / 0 replayed, 26.6 min |
| #3922 | 42377282782 | failure | 65 run / 0 replayed, 35.3 min | 34 run / 0 replayed, 37.5 min | 34 run / 0 replayed, 28.1 min |
| #3919 | 42363139656 | failure | 65 run / 0 replayed, 31.3 min | 34 run / 0 replayed, 26.5 min | 33 run / 1 replayed, 32.3 min |
| #3917 | 42315017175 | failure | 65 run / 0 replayed, 33.0 min | 34 run / 0 replayed, 34.1 min | 34 run / 0 replayed, 30.4 min |
| #3910 | 42248762796 | success | 50 run / 1 replayed, 39.2 min | 11 run / 0 replayed, 22.2 min | 10 run / 1 replayed, 13.3 min |
| #3909 | 42244887415 | failure | 42 run / 2 replayed, 25.1 min | 18 run / 1 replayed, 22.4 min | 18 run / 1 replayed, 28.4 min |
| #3907 | 42238289872 | success | 35 run / 0 replayed, 18.2 min | 8 run / 0 replayed, 13.2 min | 8 run / 0 replayed, 9.0 min |
| #3903 | 42168986509 | failure | 47 run / 3 replayed, 22.8 min | 19 run / 1 replayed, 17.7 min | 19 run / 1 replayed, 21.9 min |
| #3901 | 42096194285 | success | 30 run / 11 replayed, 25.7 min | 17 run / 1 replayed, 24.9 min | 17 run / 1 replayed, 20.6 min |
| #3900 | 42094103599 | failure | 35 run / 6 replayed, 31.7 min | 18 run / 1 replayed, 27.0 min | 18 run / 1 replayed, 21.1 min |
| #3896 | 42036556197 | success | 34 run / 4 replayed, 32.7 min | 18 run / 0 replayed, 23.7 min | 18 run / 0 replayed, 26.1 min |
| #3894 | 42026208668 | success | 25 run / 0 replayed, 27.6 min | 5 run / 0 replayed, 20.9 min | 5 run / 0 replayed, 13.3 min |
| #3891 | 41985291875 | failure | 34 run / 6 replayed, 31.1 min | 16 run / 1 replayed, 21.9 min | 16 run / 1 replayed, 27.8 min |
| #3881 | 41847627149 | failure | 39 run / 0 replayed, 30.8 min | 23 run / 0 replayed, 23.9 min | 23 run / 0 replayed, 26.8 min |
- **Prototypes measured on hosted runs** (two to four runs per side, every side a core-touching PR on the same session tree; minutes are job wall clocks read by the harness from each run's jobs listing).

| Side | Runs | test-linux min | test-macos min | test-windows min |
| --- | --- | --- | --- | --- |
| control (no recorder) | 43390706286, 43392805774 | 27.8 / 22.4 (mean 25.1) | 18.6 / 32.5 (mean 25.5) | 27.8 / 33.1 (mean 30.5) |
| dev opt-level 1 | 43392807274, 43392811473 | 28.1 / 23.8 (mean 25.9) | 36.5 / 39.6 (mean 38.1) | 42.4 / 41.0 (mean 41.7) |
| RUST_TEST_THREADS=8 | 43394779047, 43394820753, 43394772150, 43394820160 | 27.6 / 23.6 / 27.4 / 25.1 (mean 25.9) | 31.4 / 34.6 / 34.0 (mean 33.3) | 34.0 / 23.0 / 26.7 / 34.4 (mean 29.5) |
| recorder (sampler + test runner) | 43390686747, 43394638992, 43390692001 | 28.4 / 24.3 / 26.6 (mean 26.4) | 34.1 / 30.3 / 30.0 (mean 31.4) | 33.2 / 33.1 / 33.9 (mean 33.4) |
| recorder + incremental release | 43390696539, 43390701482 | 25.6 / 26.9 (mean 26.3) | 26.9 / 37.9 (mean 32.4) | 30.8 / 33.7 (mean 32.2) |

| Comparison | Δ test-linux | Δ test-macos | Δ test-windows | control spread (max − min of identical trees) L / M / W |
| --- | --- | --- | --- | --- |
| dev opt-level 1 vs control | +0.8 min | +12.5 min | +11.2 min | 5.5 / 13.9 / 5.3 |
| RUST_TEST_THREADS=8 vs control | +0.8 min | +7.8 min | -0.9 min | 5.5 / 13.9 / 5.3 |
| recorder (sampler + test runner) vs control | +1.3 min | +5.9 min | +2.9 min | 5.5 / 13.9 / 5.3 |
| recorder + incremental release vs recorder | -0.2 min | +1.0 min | -1.1 min | 5.5 / 13.9 / 5.3 |

The control's own spread is the yardstick: two runs of an identical tree differed by 5.5 min on Linux, 13.9 on macOS and 5.3 on Windows (macOS 18.6 against 32.5 min on the same `macos-26-arm64` image, version 20261102.0473.1). A difference inside that spread is not a measured saving.
- **Faster runtime code: dev `opt-level=1`** does not pay. macOS +12.5 and Windows +11.2 min, outside the spread: compile-link(plain) rose to 13.2 to 15.4 min against the control's 3.6 to 8.1, because the restored cache was built at opt-level 0 and every dependency rebuilt, while test-run(plain) did not fall (macOS 10.5 and 12.2 min against 5.6 and 9.1; Windows 19.4 and 19.1 against 14.4 and 16.8). With a cache warmed at opt-level 1 the compile cost would return, but the tests measured no faster, so the saving is 0 min on every platform (estimate for the warmed case). Proves the same.
- **Faster tests that prove the same: `RUST_TEST_THREADS=8`** (8 test threads per binary on 3- and 4-vCPU runners; four runs on Linux and Windows, three on macOS): +0.8 / +7.8 / −0.9 min, every one inside the control's spread, and every macOS run (31.4 to 34.6 min) slower than the control's mean. No measured saving.
- **Incremental release build of the distribution journeys' wheel**: against the recorder control −0.2 / +1.0 / −1.1 min, inside the spread; compile-link(release) on macOS 3.2 and 4.3 min against 3.1 to 3.7. It would also prove less (the wheel the journey installs would be built with incremental codegen units, unlike the one release CD ships). Does not pay.
- **Cold compile and link at today's layout** (the one hosted run whose Rust cache missed, 43378508844, from #3998): plain compile-link 5.88 / 4.69 / 9.34 min of wall share on Linux / macOS / Windows, instrumented 2.65 (Linux), release 2.03 / 2.58 (macOS / Windows); macOS's job failed at 18.4 min, so its figure is partial. Against the warm control's plain compile-link (Linux 2.8 to 4.2, macOS 3.6 to 8.1, Windows 4.9 to 6.5), the restored cache is worth at most a few minutes per job, which is also the cost a layout change pays once when it invalidates the cache.
- **Pyramid and crate-split prototypes: measured on matched hosted runs; their per-PR prices are a model's estimates.** The split shows **no saving distinguishable from the controls' run-to-run spread** on any platform, and is not recommended (manager ruling c11.1; evidence under `spike/ci-profile/data/prototypes/`, commands in `spike/ci-profile/prototypes/README.md`).

Kept apart below, because they answer different questions:
- **Measured job shortening, the evidence:** each matched hosted run's job wall clock and Nx gate span (first task start to last task end), control against prototype.
- **Estimated job shortening, a model (`critical_path.py`):**
  - It re-times each population PR's own run over Nx's own task graph at that PR's commit (`taskgraphs.py`, `../taskgraphs/`; all 2,433 executed tasks are in their graphs).
  - A change moves only along a dependency the run waited on: the dependent started within 2 s of it ending. Two tasks that ran one after the other without an edge are independent.
  - A task that waited on anything else keeps its observed start, so a freed slot is never reassigned and contention is not modelled.
  - Its check against measurement, `model_on_control_s`, applies the measured task changes to each control's own timeline.
- **Attribution and work, never minutes saved:** wall share (`share_saved_s`) and Linux's summed nextest test-seconds (`work_saved_s`).

The timestamp-chain pricing of an earlier revision is withdrawn. Its regressions, `tasks_that_ran_one_after_another_without_an_edge_are_independent` and `shortening_one_path_leaves_a_competing_dependent_path_where_it_was` in `test_prototype_replay.py`, fail under it.

**The split comparison, and the runs discarded on the way to it.**
- *Discarded, invalid:*
  - #4012/#4017 scheduled 47 / 34 / 34 tasks against 35 / 23 / 23, through the split's own manifest edits.
  - #4021/#4023 still scheduled two extra Linux coverage units, and executed ten tasks its control replayed.
  - #4025/#4027 had matched selection, but the patch did not register its new crate for release or in the coverage merge, so its distribution journeys and Linux's coverage merge failed early.
  - #4030/#4031's Linux job lacked the new unit in `scripts/coverage-read-sets.json`, so `github-nx-policy:coverage-read-set` failed and `workspace:test`, which waits on it, did not run.
  - Every early failure changed the split side's workload and contention, so none of those runs is compared, and leaving the failed tasks out afterwards (an earlier revision's `--leave-out`) is not used.
- *The repaired patch.* It registers the crate for release (`release-plz.toml`, `release-targets.toml`, `scripts/product_versions.py`, `scripts/release_cd.py`), in the coverage merge (`workspace/project.json`) and in the coverage read sets.
  - It was validated locally before each launch (`split-validation.txt`): the distribution journeys, the uncached coverage merge (95.10% of lines, all 15 units, the floor 92.88%) and the Linux gate itself.
  - The gate's only local failures, the two SDK coverage units, fail identically on the unpatched head on this host and pass on every hosted run.
- *The matched runs compared:*
  - Controls #4024 and #4026 (runs 43457659504, 43457450179).
  - macOS and Windows: split #4030 and #4031 (43504759508, 43504773180).
  - Linux: split #4034 and #4035 (43528710535, 43528719773). These carry, as their only further change, the macOS and Windows test jobs set to `if: false` (manager ruling a19); their `ci.yml` blob is `d935e7a1` against `84938113`.
  - Every one of those jobs ran its whole workload. One Windows distribution journey (43504759508) failed only in its last cleanup (`rm: … Device or resource busy`), after its work had run.
- *How they are matched:*
  - Both sides compare against the demonstration without its marker comment (`--marker-base`) and read no Nx cache. macOS and Windows schedule exactly the same tasks.
  - Linux's split side schedules the control's 38 tasks plus its own crate's coverage unit `onetaskgraph-template:coverage`, declared `--side-only`. Linux coverage is never narrowed and the split adds a unit; in an ordinary PR that unit would replay from the Nx cache unless the PR changed it, whereas here, with no cache on either side, it executes (288 and 312 s).
  - Nothing executed on one side that the other replayed.
- *Within those runs:*
  - macOS's split runs started about five hours after its controls, and every task the split cannot reach ran 1.5 to 2 times faster on them (`onetaskgraph-github-projects:test` 269 and 361 s against 557 and 428; `onetaskgraph-linear:test` 406 and 503 against 787 and 613). The macOS difference measures the runner, not the split.
  - On Linux, the credentialed Linear journey that talks to the real Linear API took 746 and 727 s on the split runs against 556 and 524, and in control 43457659504 it failed outright.
  - Windows' unreachable tasks agree to within run noise.

| | test-linux | test-macos | test-windows |
| --- | --- | --- | --- |
| pyramid (controls 43422783410, 43422844825 → 43422794958, 43422857812): job wall (min) | 25.9, 25.0 → 25.0, 26.8 | 32.0, 29.0 → 29.5, 29.2 | 26.5, 34.0 → 33.6, 33.1 |
| pyramid: Nx gate span (min) | 24.0, 23.3 → 23.3, 24.8 | 29.1, 26.5 → 27.3, 26.7 | 23.5, 30.3 → 29.9, 29.2 |
| pyramid: measured gate shortening, control mean less pyramid mean | -25 s | +44 s | -160 s |
| pyramid: model's gate shortening on each control's own timeline | -20, -21 s | +19, +19 s | -66, -14 s |
| split: job wall (min) | 29.9, 24.7 → 31.7, 31.3 | 37.6, 30.1 → 20.8, 25.9 | 35.4, 32.5 → 33.0, 34.1 |
| split: Nx gate span (min) | 27.9, 22.9 → 29.8, 29.5 | 34.5, 27.6 → 18.8, 23.2 | 31.6, 28.8 → 29.7, 30.4 |
| split: measured gate shortening, control mean less split mean | -257 s | +602 s | +10 s |
| split: model's gate shortening on each control's own timeline | +5, -106 s | +74, +74 s | +8, +8 s |

Reading the split row platform by platform:
- **Windows:** measured +0.2 min (shorter) on the gate span, inside the controls' own 2.8-minute spread (31.6 against 28.8 min). No saving distinguishable from that spread; the model's estimate beside it is +0.00 min.
- **macOS:** measured +10.0 min shorter, outside the controls' 6.9-minute spread (34.5 against 27.6 min), but every task the split cannot reach ran 1.5 to 2 times faster on the split runs (below), so the difference is the runner's and none of it is attributable to the split. No saving distinguishable from the controls' spread; the model's estimate beside it is +0.00 min.
- **Linux:** measured 4.3 min longer, inside the controls' 5.0-minute spread (27.9 against 22.9 min). Part of that is its own coverage unit, which a cached PR would replay; part is the live Linear journey's own variance. No saving distinguishable from that spread; the model's estimate beside it is +0.00 min.

So the **split's measured result is decided, by manager ruling c11.1: no saving distinguishable from the controls' run-to-run spread** on every platform, the spread (Linux 5.0, macOS 6.9, Windows 2.8 min of gate span) being the bound on any saving it could hold, with the model's +0.00 min per PR beside it. The spike recommends against the split on that basis. No further run was taken to narrow it. The pyramid's measured gate changes are all inside their controls' spread too.

Work metric (Linux nextest summed test-seconds, not elapsed time): the pyramid removes 514 of the `onetaskgraph-e2e` binary's 1,830 test-seconds.

**Estimated shortening, replayed over the 30 PRs** (`replay-elapsed.txt`; typical = the 18 PRs whose diff changes a crate's `src/`, `tests/`, `build.rs` or `Cargo.toml`; all 90 PR jobs had intervals and graphs):

```
pyramid: runs [43422794958, 43422857812] against control [43422783410, 43422844825]
  test-linux   measured gate span (min) 24.0, 23.3 -> 23.3, 24.8, wall 25.9, 25.0 -> 25.0, 26.8; estimated shortening +0.00 min/PR over 30, +0.00 over the 18 typical (0 without intervals)
  test-macos   measured gate span (min) 29.1, 26.5 -> 27.3, 26.7, wall 32.0, 29.0 -> 29.5, 29.2; estimated shortening +0.00 min/PR over 30, +0.00 over the 18 typical (0 without intervals)
  test-windows measured gate span (min) 23.5, 30.3 -> 29.9, 29.2, wall 26.5, 34.0 -> 33.6, 33.1; estimated shortening -0.32 min/PR over 30, -0.40 over the 18 typical (0 without intervals)
split: runs [43528710535, 43528719773] against control [43457659504, 43457450179]
  test-linux   measured gate span (min) 27.9, 22.9 -> 29.8, 29.5, wall 29.9, 24.7 -> 31.7, 31.3; estimated shortening +0.00 min/PR over 30, +0.00 over the 18 typical (0 without intervals)
split: runs [43504759508, 43504773180] against control [43457659504, 43457450179]
  test-macos   measured gate span (min) 34.5, 27.6 -> 18.8, 23.2, wall 37.6, 30.1 -> 20.8, 25.9; estimated shortening +0.00 min/PR over 30, +0.00 over the 18 typical (0 without intervals)
  test-windows measured gate span (min) 31.6, 28.8 -> 29.7, 30.4, wall 35.4, 32.5 -> 33.0, 34.1; estimated shortening +0.00 min/PR over 30, +0.00 over the 18 typical (0 without intervals)
```

- **Pyramid:** an estimated **+0.00 / +0.00 / −0.40 min** per typical PR on Linux / macOS / Windows.
- **Split:** an estimated **+0.00 min** on every platform. No population PR's job ended waiting on a core task. One of the 30, #3972, changed a file the split moves, so the replay adds the new crate's tasks to its jobs, running beside the gate from its start (Linux's coverage unit at its measured 300 s; macOS and Windows scheduled no new-crate task, so none is added there); they end before that job's gate does.

These are a model's estimates, under its stated limits. For the split they agree with the measurement: nothing distinguishable from zero.

The cold build of every test target with no Rust cache (`ciprofile.py compile`) predates the repair: compile-only runs 43422822720 and 43422893836 against 43422834977 and 43422905157. Its build step ran the same whole-workspace build on both sides and completed on all: today's layout 4.12 / 6.40 min on Linux, 6.78 / 7.55 on macOS, 6.50 / 8.07 on Windows; split 6.27 / 4.28, 2.47 / 7.13, 8.58 / 9.20, every difference inside its own side's spread. The repair adds no source to that build.

- **Options, priced** (minutes per typical PR, i.e. Rust-touching, mean of 18, and over all 30. Each price is an **estimated job shortening** from a model: each PR's run re-timed over Nx's own task graph at its commit, with the tasks an option replays taking no time, or a prototype's tasks their measured elapsed change (`critical_path.py`; `spike/ci-profile/data/replay30.txt`, `prototypes/replay-elapsed.txt`). The measured evidence for the prototypes is the matched hosted runs. Wall-share attribution, the minutes an option's tasks held a slot, is given beside it, and is never the price.)
1. *Run the distribution journeys only when the publishable surface changes* (finer key).
   - 0 min on a typical PR, because it changes crate source. Over all 30 PRs, estimated shortening: Linux 0.05, macOS 0.37, Windows 0.28 min per PR (wall-share attribution 0.05 / 0.57 / 0.44). An earlier revision priced it at 1.14 / 1.68 / 1.18 min of wall share with the rule's first, wider publishable surface; the committed `replay.py` derives that surface from the scripts' own commands, and these figures are its output (`replay30.txt`, from the committed history and inputs).
   - Effort: `scripts/ci-job.sh` and the `scripts` project's inputs, 1 dispatch. Proves the same on the PRs it skips.
   - It deliberately reverses AGENTS.md's `distribution-check`/`distribution-test` rule, so it is for the user's decision.
2. *Dependents keyed on their dependencies' `src/` and manifests, not `tests/`* (finer key).
   - Over all 30 PRs, estimated shortening: Linux 0.21, macOS 0.00, Windows 0.04 min per PR (wall-share attribution 0.46 / 0.27 / 0.54).
   - Effort: `nx.json` named inputs and about 23 `project.json` files, with `scripts/check-nx-graph.sh` and `.github/nx-policy`, 1 dispatch. Proves the same.
3. *Take formatter, lint and prose files out of Rust test keys*: estimated shortening 0.04 min per PR on Linux and 0.00 elsewhere (wall-share attribution 0.19 on Linux, 0 elsewhere). Not worth a dispatch.
4. *Split `onetaskgraph-core`: its `template` module into its own crate* (modularization; **measured** on matched hosted runs, prototype `spike/ci-profile/prototypes/split-template-crate.patch`, repaired twice and validated; see "Pyramid and crate-split prototypes" above). It moves a module that depends on nothing else in core, with its 55 tests, behind a re-export.
   - **Net savings: no saving distinguishable from the controls' run-to-run spread** on any platform (manager ruling c11.1). The bound is that spread of gate span: Linux 5.0, macOS 6.9, Windows 2.8 min. Measured: Linux 4.3 min longer, Windows 0.2 min shorter, both inside it; macOS 10.0 min shorter, outside it but matched by tasks the split cannot reach running 1.5 to 2 times faster, so it is the runner's.
   - The model's estimate is **+0.00 min** per typical PR on every platform. The cold build is unchanged within its spread.
   - Cost, stated rather than measured: a landed split must also re-record `github-nx-policy`'s historical-hash fixture, which holds core's inputs to an earlier commit.
   - Effort: 57 files, 1 dispatch. Proves the same.
   - **Recommended against**: on the measurement and the model alike it saves nothing that can be told from noise, and it costs 57 files and a re-recorded fixture.
5. *Incremental release build for the distribution journeys*: measured above, inside the spread; 0 min on every platform. Effort: one variable exported in the distribution phase of `scripts/ci-job.sh`, 1 dispatch. Proves less (the journey's wheel is built unlike the shipped one). Not recommended.
6. *Faster runtime code*: dev `opt-level=1`, measured above, 0 min saved and +11 to +12.5 min on macOS and Windows while the cache is rebuilt. Effort: one `[profile.dev]` line in `Cargo.toml` and a new cache family, 1 dispatch. Proves the same. The ceiling for making the binary itself cheaper to start is its spawn time: 2.54 / 1.83 / 4.05 min of the recorded jobs (Linux / macOS / Windows), so a 20% faster start-up would be about 0.5 / 0.4 / 0.8 min (estimate). Not prototyped.
7. *Faster tests that prove the same*: `RUST_TEST_THREADS=8`, measured above, 0 min saved on every platform (inside the spread). Effort: one variable in `ci.yml`'s three test steps, 1 dispatch. Proves the same. The two `cache_budget` journeys (Linux 363 + 355 s, macOS 243 + 158 s) are the single largest test costs, but they are not what the platforms' jobs end waiting on, so shortening them alone saves under 1 min per job (estimate, from the timeline; the dependency-aware model of `critical_path.py` was not run for them). Not prototyped.
8. *Pyramid rebalance of `onetaskgraph-e2e`* (**measured** on a sample with identical selection, prototype `spike/ci-profile/prototypes/pyramid-e2e-journeys.patch`). The sample marks `#[ignore]` on the 121 journeys of `section_links`, `assets` and `held_writes`, whose same-named in-process `onetaskgraph-core` suites keep running. **Estimated job shortening per typical PR: +0.00 / +0.00 / -0.40 min** on Linux / macOS / Windows (a model's estimate); the matched hosted runs measured no shortening outside their noise. It removes 514 of the e2e binary's 1,830 Linux test-seconds (work, not elapsed), but no PR's Linux job waited on the e2e task at its end, and its macOS and Windows elapsed changes are inside noise. Extending it to all 19 such modules (378 journeys) scales a figure indistinguishable from zero, so it is not priced further. Effort: 3 files, 1 dispatch for the sample; 2 to 3 for all 19. **Proves less** for no measurable saving, per behaviour:
   - section links, assets, rendered and copy snapshots: an in-process test still proves the rewrite, the asset bytes and digests and the request counts against the loopback stand-ins; only the journey proves the command line's flags, the rendered output and exit code a user sees, and the files written through the process boundary.
   - held writes, update, priority, metadata, delivery and comments: the in-process test still proves the version guard and every field written; only the journey proves the `--if-version` and other flags parse, refusals reach stderr with their next action and the right exit status.
   - copy, copy links, locate, relations, plan graph, batch, templates, commented-since and human routes: the in-process test proves the engine's answer; only the journey proves the stdio plugin seam (§3.1's cleared environment) and the machine-readable output against the schema the binary emits.
   For the user's decision; the spike adopts none.
9. *Run the distribution journeys only when the publishable surface changes*, *dependents keyed on dependencies' `src/` only*, *formatter files out of Rust keys*: replayed above (items 1 to 3), 0 min on a typical PR because its diff changes crate source; over all 30 PRs an estimated shortening of at most 0.37 min (macOS) per PR.
- **Findings about the plan.**
- The live lanes take the account's rate limit, which the host's own agent reads share. While this host was draining the allowance, a demonstration's live asset journey paused 25 min by design and every job timed out. Measurement spikes and the live lane cannot both spend that allowance at once.
- Linux spends 1.7 to 3.4 min per Rust PR (mean 1.6 → 3.4 before → after) blocked on cargo's build-directory lock between concurrent Nx tasks. Parallelism above one cargo invocation buys less than its slot count.
- **What this spike still leaves unmeasured**, stated rather than papered over:
- a prototype of faster runtime code beyond `opt-level=1` (its ceiling is priced from spawn records, labelled an estimate);
- the pyramid beyond its three-module sample (its sample's estimated shortening is indistinguishable from zero, so no extrapolation is priced);
- summed test-seconds on macOS and Windows for the prototype runs (no recorder ran there, so the work metric is Linux's alone; elapsed task and job times are measured on all three);
- the model's limits: a freed slot is never reassigned and contention between concurrent tasks is not modelled, so its estimates are checked against, and subordinate to, the matched runs' measured gate spans;
- any split saving smaller than the controls' run-to-run spread (Linux 5.0, macOS 6.9, Windows 2.8 min): by manager ruling c11.1 the comparison is finished at "no saving distinguishable from that spread", and no runs matched in time were taken to narrow it. macOS's split runs started five hours after their controls and ran every unreachable task 1.5 to 2 times faster; Linux carries the live Linear lane's variance and the new crate's coverage unit executing without a cache. Every platform has two matched runs per side, Linux's split runs with the macOS and Windows jobs switched off (ruling a19);
- the hosted A/B of #3956 and its parent, not run because the log-based attribution decided the cause (finding above);
- run 43394820160's test-macos (#4008's second head `b9504265`): queued behind the account's macOS concurrency from 08:38Z and still queued at the last read (11:44Z). By manager ruling a8 it was not cancelled: #4008 was closed with that run left queued, and its macOS leg is not read; the side's two complete macOS runs (43394779047 and 43394820753) and a third (43394772150) are read.
- **Judged diff lint: an accepted gap for this throwaway harness** (manager rulings a4 and c4.5). `just lint-llm-diff origin/main` on the branch (llmlint history `20261113T064655Z-d7ece`, on `655b7bdb`) failed 12 rules over `spike/ci-profile/`: `async_typed_clients_at_boundaries`, `boundary_inputs_validated`, `changed_behavior_has_e2e`, `cli_output_contract`, `comments_earn_their_place`, `contracts_have_one_source_or_a_drift_gate`, `modern_domain_modeling`, `names_match_behavior`, `new_code_lands_in_a_project`, `no_injection_from_untrusted_input`, `structural_pattern_matching`, `suppressions_justified`. Later commits (`fb794565` to `b3c574eb`) answered several of them (shell text and artifact paths refused, typed records, a project of its own, commands driven through their entry points); later judged runs were made by the previous dispatch and their remaining findings were not re-listed before it ended. Per ruling c4.5 this dispatch did not re-run or rework it. Its deterministic lint over the diff — the `spike-ci-profile` project's `ruff format --check`, `ruff check` and `ty check` — is clean on the finished commit, after this dispatch wrapped three over-long test lines (`1fa3bd49`) and added the compile reader, the cold-build and per-job exclusion switches, task intervals in the history (with a fix for a gate whose log group opens beside another, which had made the 22 older log-read jobs read as task-less; every cited profile and all 2,664 history tasks read the same after it) and the elapsed-time prototype replay, each with tests driven through its entry point. A task that promotes any of this harness into the repository must meet all 12.


<!-- onetaskgraph:template-answers
branch: nick/ci-test-speed-2026-11-12/spike-onetaskgraph-profile
candidates: []
findings:
- |-
  **Wall clock of a core-touching PR, divided, on complete jobs carrying the recorder** (run 43390692001, #4000, `GITHUB_SHA` `0f977a4b`; harness output on `697233f3`; a second recorded run, 43394638992, agrees within the spread below). Two exclusive views, each summing to the job's wall clock, so no minute is counted twice: *by phase* (every second of the job assigned once; concurrent Nx tasks share each instant equally) and, inside the test step, *by leaf process* (each one-second sampler tick shared among the leaf processes then running).

  *By phase* (minutes, share):
  - **test-linux 26.58 min**: test-run(instrumented) 8.63 (32.5%); test-run(plain) 6.42 (24.1%); compile-link(plain) 3.19 (12.0%); wait:cargo-file-lock 1.97 (7.4%); compile-link(instrumented) 1.72 (6.5%); scripts and other tasks 1.31 (4.9%); setup 1.24 (4.7%); budget checks 1.02 (3.8%); cache restore 0.62 (2.3%); Nx planning 0.28 (1.1%); report 0.08; cache save 0.02; Nx remote-cache restore 0.00 (11 replays, under 1 s each); **unattributed 0.08 (0.3%)**.
  - **test-macos 29.97 min**: test-run(plain) 7.89 (26.3%); compile-link(plain) 7.49 (25.0%); scripts and other tasks 6.06 (20.2%; `scripts:distribution-test` alone 5.26); compile-link(release) 3.50 (11.7%); budget checks 2.25 (7.5%); setup 1.98 (6.6%); cache restore 0.42 (1.4%); Nx planning 0.25; report 0.07; cache save 0.00 (none); **unattributed 0.06 (0.2%)**.
  - **test-windows 33.85 min**: test-run(plain) 17.28 (51.0%); compile-link(plain) 6.44 (19.0%); scripts and other tasks 3.74 (11.0%); setup 2.86 (8.4%); compile-link(release) 1.88 (5.5%); budget checks 0.82 (2.4%); cache restore 0.41 (1.2%); Nx planning 0.28; report 0.10; cache save 0.00 (none); **unattributed 0.05 (0.1%)**.
  - *Coverage overhead* is Linux's alone: compile-link(instrumented) 1.72 + test-run(instrumented) 8.63 = 10.35 min (38.9%), of which `llvm-cov`/profdata reporting is 1.17 min by process. macOS and Windows: a measured zero (their selection holds no `coverage` task; `ci-job.sh` measures on Linux alone).
  - *Per Nx task and the latest-finishing chain* (wall share; the chain is a timeline description, walking back from the gate's end through the task still running as each later one began, not a dependency path): Linux `onetaskgraph-github-projects-e2e:test` 4.38 min (1,302 s own), `onetaskgraph-github-projects:test` 3.88, `onetaskgraph-e2e:coverage` 2.75; latest-finishing chain `onetaskgraph:build` → `onetaskgraph-github-projects-e2e:test` (the live lane, 21.7 min) → its `budgets`, 24.13 min. macOS `scripts:distribution-test` 5.26 min (1,287 s own), `onetaskgraph-github-projects-e2e:test` 2.67, `onetaskgraph-linear-e2e:test` 2.59; chain `onetaskgraph-github-projects:test` → `onetaskgraph-github-projects-e2e:test` → `sdk-python:test` → `workspace:test`, 26.91 min. Windows `onetaskgraph-e2e:test` 6.22 min (1,681 s own), `scripts:distribution-test` 5.02, `onetaskgraph-linear-e2e:test` 4.19; chain `onetaskgraph-github-projects:test` → `sdk-typescript:generate-check` → `onetaskgraph:test` → `sdk-python:test` → `onetaskgraph-status-options-e2e:test` → `sdk-typescript:test`, 29.90 min.

  *By leaf process, inside the test step* (minutes of the job; the step is 24.52 / 27.42 / 30.42 min on Linux / macOS / Windows and the categories sum to it, every tick assigned):
  - rustc (compile): 3.98 / 4.96 / 3.00; linker: 0.27 / 0.24 / 0.47; cargo itself: 3.65 / 4.22 / 3.17.
  - test binaries running their own code: 3.64 / 0.48 / 1.74; **sleeps, polls and timeouts inside tests** (ticks where the work processes used under 10% of one CPU): 1.48 / 0.02 / a measured zero (no such tick on Windows), almost all Linux's live session pausing on the shared allowance.
  - **spawns of other CLIs**: the CLI under test 2.54 / 1.83 / 4.05 (3,013 / 969 / 5,716 processes); Python and uv 3.40 / 3.10 / 6.25 (2,413 / 721 / 1,961); node and npm 0.01 / 7.99 / 6.25 (1 / 547 / 154); bun 3.79 / 3.51 / 0.64; git 0.20 / 0.25 / 1.70 (40 / 62 / 919); onevcs 0.01 / 0.02 / 2.03 (4 / 6 / 1,167); network clients (curl, wget) 0.00 / 0.01 / 0.02; tar and zip 0.01 / 0.10 / 0.07; doctests 0.02 / 0.03 / 0.03.
  - **fixtures**: the Python document-store peer is inside the Python figure; the loopback GitHub and Linear stand-ins are threads of the test binaries, so they are inside the test-binary figure and cannot be told apart from the test's own code by a process sampler.
  - waits on a child (a parent whose only running children are named): Linux 0.33, macOS 0.68, Windows 1.01.

  *Steps outside the test step* are the phase view's setup, cache restore, cache save and report rows. The rust-cache restore itself: 34.3 s of 1,362 MB (Linux), 20.7 s of 615 MB (macOS), 18.6 s of 613 MB (Windows), all hits; cache save is a measured zero here because the demonstration disables it, and on ordinary PR runs it is 0.0 to 0.1 min (run 43152201170: 0.00 / 0.07 / 0.02 min), because only `main` saves Rust caches.

  *The recorder's own cost*, measured: recorder against control means +1.3 / +5.9 / +2.9 min on Linux / macOS / Windows; macOS's is inside its 13.9-minute control spread, Linux's inside 5.5; Windows' is under its 5.3 spread but all three recorded Windows runs sit at 33.1 to 33.9 min against the control's 27.8 and 33.1, so up to about 3 min of a Windows recorded job may be the recorder's (estimate). The divisions above are of a recorded job.
- |-
  **The 20 slowest tests per platform and every test binary's seconds** (run 43390692001, harness output on the finished commit; names are cut at the harness's print width; the full per-binary list is in its `--json`).

  ```
  test-linux:
    slowest tests (of 3821):
        953.1s  live-57f9f5cdc3e78625::disposable_task_and_document_asset_copies
        912.3s  live-62ad363e434f2ff9::real_projects_v2_contract_writes_and_leaves_no_residue
        363.3s  instrumented:onetaskgraph-github-projects-e2e::e2e::cache_budget::the_cache_spends_no_more_board_points_than_n
        355.0s  instrumented:onetaskgraph-linear-e2e::e2e::cache_budget::the_cache_sends_fewer_of_the_linear_keys_requests_in_
        154.5s  instrumented:onetaskgraph-github-projects-e2e::e2e::plan_copy::a_plan_copy_reads_what_the_plan_holds_whatever_
         79.1s  instrumented:onetaskgraph-e2e::e2e::held_writes::every_task_write_verb_held_to_a_version_lands_at_it_and_is_re
         72.2s  instrumented:onetaskgraph-github-projects-e2e::lane_wiring::budget_adapter_uses_the_real_checker_and_never_sub
         61.4s  sweep_gate-5a3d182d0f5a038d::a_presence_read_that_settles_nothing_fails_the_cleanup
         60.2s  instrumented:onetaskgraph-github-projects-e2e::e2e::assets::asset_requests_take_mutation_slots_and_each_limite
         60.0s  instrumented:onetaskgraph-github-projects-e2e::e2e::human_cost::a_hundred_tasks_from_ten_managers_are_projecte
         53.4s  instrumented:onetaskgraph-github-projects-e2e::e2e::plan_pacing::filling_the_minutes_points_with_queries_alone
         48.3s  instrumented:onetaskgraph-e2e::e2e::section_links::public_copies_keep_sections_and_screen_private_record_links
         48.2s  instrumented:onetaskgraph-live-janitor::journeys::offline_linear_cleanup_journeys
         46.5s  instrumented:onetaskgraph-e2e::e2e::section_links::every_pair_of_kinds_writes_the_others_section_links_in_its_
         41.7s  run_board_gate-65f133c51362dacd::a_session_naming_a_board_outside_its_run_is_refused_before_sending
         38.8s  run_board_gate-65f133c51362dacd::concurrent_lifecycles_made_to_use_one_board_fail
         38.8s  sweep_gate-5a3d182d0f5a038d::an_issue_delete_github_never_answers_fails_the_cleanup_after_its_attempts
         38.2s  run_board_gate-65f133c51362dacd::eight_runs_at_once_stay_inside_the_hourly_points_hourly_creations_and_board_f
         37.9s  e2e-093be27dc4df3c09::linear_assets::live_disposable_issue_and_document_assets_copy_and_recopy
         37.6s  run_board_gate-65f133c51362dacd::a_copy_made_after_another_sessions_copy_was_deleted_never_reuses_its_number

  test-macos:
    slowest tests (of 3278):
        243.0s  e2e-f21661ddefeb1e08::cache_budget::the_cache_spends_no_more_board_points_than_no_cache_over_every_kind_sequen
        158.2s  e2e-3338b72731d1100b::cache_budget::the_cache_sends_fewer_of_the_linear_keys_requests_in_an_hour_than_no_cache
        143.5s  e2e-f21661ddefeb1e08::plan_copy::a_plan_copy_reads_what_the_plan_holds_whatever_the_board_has_accumulated
         87.7s  e2e-f21661ddefeb1e08::human_cost::a_hundred_tasks_from_ten_managers_are_projected_recovered_and_resolved_once_
         55.1s  e2e-3338b72731d1100b::cache_parts::every_route_to_a_tasks_comments_is_served_warm_and_read_fresh_on_a_newer_or
         41.6s  e2e-3338b72731d1100b::assignees::two_hundred_fifty_two_unseen_names_are_resolved_in_one_lookup_that_answers_ea
         33.5s  e2e-f21661ddefeb1e08::plan_pacing::filling_the_minutes_points_with_queries_alone_holds_the_next_query
         32.2s  plugin-5e3e350e9fe048dc::the_change_kind_leg_asks_again_while_a_token_is_withheld
         29.5s  e2e-f21661ddefeb1e08::assets::measure_single_and_three_concurrent_project_copies_in_virtual_time
         29.2s  lane_wiring-3a81eb772e661f3a::budget_adapter_uses_the_real_checker_and_never_substitutes_missing_live_evidence
         28.6s  e2e-f21661ddefeb1e08::assets::asset_requests_take_mutation_slots_and_each_limiter_recovers_or_exhausts_its_bud
         28.4s  e2e-f21661ddefeb1e08::plan_pacing::filling_the_hours_creations_holds_the_next_until_the_first_has_left_the_hou
         28.1s  e2e-3338b72731d1100b::linear_assets::a_refused_concurrent_copy_is_named_in_what_the_budget_check_reports
         27.6s  e2e-f21661ddefeb1e08::budget_runner::report_sanitize_complete_sweep
         24.3s  sweep_gate-2ca887d576480bac::a_presence_read_that_settles_nothing_fails_the_cleanup
         23.7s  schema-04841b9538cbcb2f::a_schema_check_server_stops_once_idle_and_never_while_a_client_is_connected
         23.1s  sweep_gate-2ca887d576480bac::an_artifact_another_deleter_took_first_leaves_the_cleanup_successful
         22.3s  e2e-f21661ddefeb1e08::human_reporters::the_approval_reporter_hands_over_the_larger_sequence_of_the_journeys_ca
         22.1s  sweep_gate-2ca887d576480bac::an_issue_delete_answered_without_confirmation_is_settled_by_the_repository
         22.1s  e2e-5232741c9111c88a::copy_retry::a_project_or_document_copy_stopped_at_either_point_of_its_create_is_finished

  test-windows:
    slowest tests (of 3236):
        604.6s  e2e-366086ca01a7cdab.exe::cache_budget::the_cache_sends_fewer_of_the_linear_keys_requests_in_an_hour_than_no_c
        436.6s  e2e-b037a591c8ad43e8.exe::cache_budget::the_cache_spends_no_more_board_points_than_no_cache_over_every_kind_se
        205.6s  e2e-b037a591c8ad43e8.exe::plan_copy::a_plan_copy_reads_what_the_plan_holds_whatever_the_board_has_accumulated
        157.7s  e2e-b037a591c8ad43e8.exe::human_cost::a_hundred_tasks_from_ten_managers_are_projected_recovered_and_resolved_o
        121.1s  run_board_gate-ef173e0d8e1d72dd.exe::concurrent_lifecycles_made_to_use_one_board_fail
        121.0s  run_board_gate-ef173e0d8e1d72dd.exe::eight_runs_at_once_stay_inside_the_hourly_points_hourly_creations_and_boa
        119.7s  run_board_gate-ef173e0d8e1d72dd.exe::a_session_naming_a_board_outside_its_run_is_refused_before_sending
        113.1s  run_board_gate-ef173e0d8e1d72dd.exe::a_copy_made_after_another_sessions_copy_was_deleted_never_reuses_its_numb
         97.2s  run_board_gate-ef173e0d8e1d72dd.exe::a_copy_that_cannot_be_deleted_says_so_beside_the_failure_that_ended_it
         94.8s  e2e-b037a591c8ad43e8.exe::narrow_write_cost::every_narrow_write_under_a_sanitize_command_costs_what_it_costs_w
         86.5s  e2e-b037a591c8ad43e8.exe::budget_runner::report_sanitize_complete_sweep
         83.8s  run_board_gate-ef173e0d8e1d72dd.exe::a_copy_that_settles_short_is_deleted_and_the_template_copied_once_more
         83.6s  e2e-e3f511a2d6f27f71.exe::boundary::projections::a_private_linear_canonicals_public_view_keeps_its_authored_se
         79.4s  e2e-b037a591c8ad43e8.exe::plan_pacing::filling_the_hours_creations_holds_the_next_until_the_first_has_left_the
         75.6s  run_board_gate-ef173e0d8e1d72dd.exe::a_copy_waits_for_a_last_draft_its_listing_shows_a_minute_after_the_copy
         75.2s  e2e-b037a591c8ad43e8.exe::plan_pacing::filling_the_minutes_points_with_queries_alone_holds_the_next_query
         60.3s  e2e-366086ca01a7cdab.exe::plan_copy::an_unchanged_private_plan_with_repositories_and_a_dependency_sends_no_wri
         58.0s  e2e-e3f511a2d6f27f71.exe::assets::a_copy_into_a_plugin_serving_assets_rewrites_references_and_reuploads_only_c
         56.4s  e2e-e3f511a2d6f27f71.exe::boundary::projections::an_anchor_carrying_anything_but_its_own_name_is_refused_namin
         47.5s  e2e-e3f511a2d6f27f71.exe::boundary::projections::an_unrecorded_task_is_taken_as_a_view_only_when_it_alone_says
  ```

  Every end-to-end test binary, wall seconds and summed test seconds (from the same run's `spike-profile-<job>` records; crate named by matching each binary's test modules to `crates/*/tests/`):

  ```
  == test-linux (run 43390692001)
    onetaskgraph-e2e,onetaskgraph-github-projects-e2e,onetaskgraph-linear-e2e e2e-7903b8d102129eef                          wall  1805.7s  test-s      0.0  tests 1
    onetaskgraph-e2e,onetaskgraph-github-projects-e2e,onetaskgraph-linear-e2e e2e-cbf38a7a5a51ecca                          wall  1665.4s  test-s     10.5  tests 28
    onetaskgraph,onetaskgraph-github-projects,onetaskgraph-github-projects-e2e,onetaskgraph-linear live-57f9f5cdc3e78625                         wall   960.9s  test-s    953.1  tests 1
    onetaskgraph,onetaskgraph-github-projects,onetaskgraph-github-projects-e2e,onetaskgraph-linear live-62ad363e434f2ff9                         wall   913.2s  test-s    913.0  tests 15
    onetaskgraph-linear-e2e                e2e-093be27dc4df3c09                          wall   848.1s  test-s     38.0  tests 22
    onetaskgraph-github-projects-e2e       live_pair-3737784b2c0474bf                    wall    99.6s  test-s      0.0  tests 0
    onetaskgraph-github-projects-e2e       lane_wiring-faa9ed6c3c5a72c9                  wall    92.2s  test-s      0.0  tests 0
    onetaskgraph-e2e,onetaskgraph-github-projects-e2e,onetaskgraph-linear-e2e,onetaskgraph-status-options-e2e e2e-356c145d8d54a8ec                          wall     6.9s  test-s      0.0  tests 0
    onetaskgraph-core,onetaskgraph-e2e     configuration-b00f1009affa81c6                wall     5.5s  test-s      0.0  tests 0
    onetaskgraph-e2e                       incomplete_sources-4367d1bb28a0f0eb           wall     4.9s  test-s      0.0  tests 0
    onetaskgraph,onetaskgraph-github-projects,onetaskgraph-github-projects-e2e,onetaskgraph-linear live-bf06b35555cdf34c                         wall     2.7s  test-s      0.0  tests 0
    onetaskgraph-core,onetaskgraph-e2e     configuration-a8f10eebebc6a1ec                wall     2.5s  test-s      0.0  tests 0
    onetaskgraph-e2e                       ambient_credentials-fcd41dacb302db7c          wall     0.1s  test-s      0.0  tests 0
  == test-macos (run 43390692001)
    onetaskgraph-github-projects-e2e       e2e-f21661ddefeb1e08                          wall   401.5s  test-s   1126.9  tests 519
    onetaskgraph-e2e                       e2e-5232741c9111c88a                          wall   321.5s  test-s    962.2  tests 1124
    onetaskgraph-linear-e2e                e2e-3338b72731d1100b                          wall   165.7s  test-s    455.7  tests 297
    onetaskgraph-github-projects-e2e       lane_wiring-3a81eb772e661f3a                  wall    29.2s  test-s     55.7  tests 4
    onetaskgraph-github-projects-e2e       live_pair-84267f50fca1b037                    wall    21.9s  test-s     64.7  tests 14
    onetaskgraph,onetaskgraph-github-projects,onetaskgraph-github-projects-e2e,onetaskgraph-linear live-c79c887e08abbc0e                         wall     5.6s  test-s      6.4  tests 14
    onetaskgraph,onetaskgraph-github-projects,onetaskgraph-github-projects-e2e,onetaskgraph-linear live-fc5bc8444cf2e685                         wall     2.4s  test-s      2.3  tests 1
    onetaskgraph-core,onetaskgraph-e2e     configuration-fb540783f2f58b68                wall     1.4s  test-s      3.4  tests 70
    onetaskgraph-status-options-e2e        e2e-bafce794a28a3b31                          wall     1.3s  test-s      3.8  tests 12
    onetaskgraph-e2e                       incomplete_sources-6be9c63e3ecb99a0           wall     0.6s  test-s      1.5  tests 9
    onetaskgraph,onetaskgraph-github-projects,onetaskgraph-github-projects-e2e,onetaskgraph-linear live-95c694d6517aeebc                         wall     0.6s  test-s      1.1  tests 15
    onetaskgraph-e2e                       ambient_credentials-8387c6551c897682          wall     0.4s  test-s      0.2  tests 1
    onetaskgraph-core,onetaskgraph-e2e     configuration-e24a616daa918892                wall     0.1s  test-s      0.2  tests 72
  == test-windows (run 43390692001)
    onetaskgraph-e2e                       e2e-e3f511a2d6f27f71.exe                      wall  1189.9s  test-s   4660.5  tests 1113
    onetaskgraph-github-projects-e2e       e2e-b037a591c8ad43e8.exe                      wall   734.7s  test-s   2548.0  tests 515
    onetaskgraph-linear-e2e                e2e-366086ca01a7cdab.exe                      wall   644.3s  test-s   1394.0  tests 293
    onetaskgraph-github-projects-e2e       live_pair-265d226b93f9b81b.exe                wall    61.8s  test-s    231.8  tests 14
    onetaskgraph-github-projects-e2e       lane_wiring-641def72719ebdf3.exe              wall    20.0s  test-s     51.5  tests 4
    onetaskgraph-core,onetaskgraph-e2e     configuration-ca1dd1eee257375d.exe            wall    14.3s  test-s     45.0  tests 67
    onetaskgraph,onetaskgraph-github-projects,onetaskgraph-github-projects-e2e,onetaskgraph-linear live-a0fdeba6235cecc8.exe                     wall    13.6s  test-s     13.4  tests 1
    onetaskgraph,onetaskgraph-github-projects,onetaskgraph-github-projects-e2e,onetaskgraph-linear live-88de5792da721c31.exe                     wall     8.2s  test-s     16.0  tests 13
    onetaskgraph,onetaskgraph-github-projects,onetaskgraph-github-projects-e2e,onetaskgraph-linear live-573c24dc8ed11c98.exe                     wall     6.1s  test-s      9.8  tests 15
    onetaskgraph-status-options-e2e        e2e-559b6ded479dad96.exe                      wall     1.1s  test-s      3.8  tests 12
    onetaskgraph-e2e                       incomplete_sources-92728eac239f7f02.exe       wall     1.0s  test-s      3.7  tests 9
    onetaskgraph-e2e                       ambient_credentials-6b95bc0af8b49ef3.exe      wall     0.6s  test-s      0.5  tests 1
    onetaskgraph-core,onetaskgraph-e2e     configuration-8801c60522594d3c.exe            wall     0.3s  test-s      0.4  tests 69
  ```
  On Linux the e2e crates run under coverage through nextest, so their binaries' seconds are the harness's `instrumented:` rows: `onetaskgraph-e2e::e2e` 2,108.8 s, `onetaskgraph-github-projects-e2e::e2e` 1,790.1 s, `onetaskgraph-linear-e2e::e2e` 867.9 s, `live_pair` 101.2 s, `lane_wiring` 93.0 s; the rows above with 0 tests on Linux are the recording runner's listing passes for those same binaries.
- |-
  **Before and after #3956** (`ciprofile.py durations`, successful jobs).
  - All PRs: test-linux 32.5 → 27.7 min (n 39 → 10), test-macos 24.1 → 33.3 (66 → 19), test-windows 21.9 → 24.7 (66 → 18).
  - Rust-touching PRs only: macOS 24.9 → 33.3 (41 → 17), Windows 25.0 → 24.7 (39 → 16), Linux 31.2 → 28.5 (25 → 8).
  - So the Windows rise in the medians is the mix of PRs: before #3956 there were more CI-only PRs, which run no Rust suite. The macOS rise is real.
- |-
  **Cause of the macOS rise: log-based attribution, so the historical A/B was not run.** First runs of the 10 Rust-touching PRs before #3956 against the 8 after, mean macOS wall 23.4 → 33.7 min (+10.3), divided by wall share:
  - test-run +3.8 min;
  - scripts and other tasks +5.2 min: `scripts:test` +2.9 (selected by more of the later PRs) and `scripts:distribution-check` +0.8 (own time 2.5 → 6.4 min);
  - plain compile and link +1.5 min; release compile -0.6 min.

  Per task, `onetaskgraph-e2e:test` own time went 4.3 → 7.9 min, `onetaskgraph-github-projects:test` 5.3 → 8.8, and `onetaskgraph-linear:test` 3.8 → 5.4. The suites grew: macOS e2e tests run 1,064 (#3931) → 1,204 (#3984), Linear e2e 209 → 297, core 600 → 651. These are tests added by #3969 to #3984.

  #3956 changed no macOS phase in `scripts/ci-job.sh`, did not touch `ci.yml`, and macOS replay stayed as low after it as before (Rust PRs before ran 10 to 34 tasks and replayed 0 to 15; after, 24 to 32 and 0 to 4). The runner image was macos-26-arm64 throughout.

  The caching change's own share of the macOS rise is therefore about 0 min; the rise is test growth and selection mix. Linux moved to ubuntu-26.04 at #3956's own run (42656859552), and Linux's fall is mostly `github-workflows:cd-journeys` (own time 26.4 → 16.3 min) and Linux's distribution journey moving to `lint` (#3971).
- |-
  **Reuse over the 30 PRs' first runs** (`replay.py`, committed with its inputs in `spike/ci-profile/data/`).

  Tasks run / replayed: Linux 1,211 / 166, macOS 605 / 39, Windows 617 / 26. Mean wall 30.7 / 25.8 / 24.1 min.

  Executed wall share (attribution: where the minutes went, not how much shorter a job would be) by the most specific changed input matched, in minutes over the 30 PRs; each option's predicted job shortening is under Options:
  - Linux: dependency source 208, not keyed on the diff 183, scripts 106, own tests 90, own source 58, `.github` 57.
  - macOS: scripts 160, not keyed 135, dependency source 114, own tests 78.
  - Windows: own tests 115, dependency source 114, not keyed 106, scripts 100.

  "Not keyed on the diff" is mostly `scripts:distribution-test` on every PR (macOS 60 min over 12 PRs, Windows 49) and the live `onetaskgraph-github-projects:test`. It also includes e2e coverage units whose key moved only through a runtime input (the read-set digest or the cargo closure), which `nx show target inputs` does not list.

  Per PR row (first run, per job):

  | PR | Run | Conclusion | test-linux | test-macos | test-windows |
  | --- | --- | --- | --- | --- | --- |
  | #3993 | 43349781263 | success | 1 run / 0 replayed, 3.3 min | 3 run / 0 replayed, 18.1 min | 3 run / 0 replayed, 13.0 min |
  | #3984 | 43142127197 | failure | 41 run / 10 replayed, 28.8 min | 27 run / 0 replayed, 33.2 min | 27 run / 0 replayed, 34.7 min |
  | #3982 | 43077192372 | failure | 42 run / 2 replayed, 42.9 min | 31 run / 0 replayed, 40.6 min | 31 run / 0 replayed, 31.2 min |
  | #3981 | 43046436056 | failure | 46 run / 3 replayed, 43.5 min | 30 run / 0 replayed, 23.5 min | 30 run / 0 replayed, 39.2 min |
  | #3979 | 43000055371 | success | 29 run / 41 replayed, 25.5 min | 31 run / 5 replayed, 34.9 min | 31 run / 5 replayed, 30.2 min |
  | #3977 | 42993507127 | failure | 41 run / 7 replayed, 27.8 min | 30 run / 0 replayed, 34.4 min | 30 run / 0 replayed, 29.1 min |
  | #3976 | 42967701358 | failure | 49 run / 18 replayed, 33.5 min | 32 run / 4 replayed, 33.7 min | 32 run / 4 replayed, 37.8 min |
  | #3972 | 42812145716 | failure | 41 run / 7 replayed, 23.9 min | 29 run / 0 replayed, 28.3 min | 29 run / 0 replayed, 32.2 min |
  | #3971 | 42793675570 | failure | 41 run / 11 replayed, 41.0 min | 33 run / 3 replayed, 39.4 min | 32 run / 3 replayed, 29.9 min |
  | #3969 | 42778025118 | failure | 40 run / 7 replayed, 31.9 min | 28 run / 0 replayed, 36.8 min | 28 run / 0 replayed, 35.0 min |
  | #3956 | 42656859552 | failure | 24 run / 0 replayed, 21.9 min | 5 run / 0 replayed, 19.6 min | 5 run / 0 replayed, 12.2 min |
  | #3940 | 42486624315 | success | 36 run / 1 replayed, 37.2 min | 7 run / 0 replayed, 18.3 min | 7 run / 0 replayed, 13.6 min |
  | #3938 | 42482531959 | cancelled | 43 run / 2 replayed, 41.8 min | 11 run / 0 replayed, 18.5 min | 11 run / 0 replayed, 10.0 min |
  | #3937 | 42477887564 | success | 43 run / 1 replayed, 36.1 min | 7 run / 0 replayed, 23.1 min | 7 run / 0 replayed, 13.8 min |
  | #3931 | 42435141466 | failure | 51 run / 15 replayed, 37.0 min | 27 run / 7 replayed, 25.0 min | 27 run / 7 replayed, 24.0 min |
  | #3928 | 42412580213 | failure | 38 run / 3 replayed, 33.9 min | 9 run / 0 replayed, 15.4 min | 9 run / 0 replayed, 10.8 min |
  | #3926 | 42396913357 | success | 39 run / 5 replayed, 27.4 min | 10 run / 15 replayed, 15.7 min | 25 run / 0 replayed, 26.6 min |
  | #3922 | 42377282782 | failure | 65 run / 0 replayed, 35.3 min | 34 run / 0 replayed, 37.5 min | 34 run / 0 replayed, 28.1 min |
  | #3919 | 42363139656 | failure | 65 run / 0 replayed, 31.3 min | 34 run / 0 replayed, 26.5 min | 33 run / 1 replayed, 32.3 min |
  | #3917 | 42315017175 | failure | 65 run / 0 replayed, 33.0 min | 34 run / 0 replayed, 34.1 min | 34 run / 0 replayed, 30.4 min |
  | #3910 | 42248762796 | success | 50 run / 1 replayed, 39.2 min | 11 run / 0 replayed, 22.2 min | 10 run / 1 replayed, 13.3 min |
  | #3909 | 42244887415 | failure | 42 run / 2 replayed, 25.1 min | 18 run / 1 replayed, 22.4 min | 18 run / 1 replayed, 28.4 min |
  | #3907 | 42238289872 | success | 35 run / 0 replayed, 18.2 min | 8 run / 0 replayed, 13.2 min | 8 run / 0 replayed, 9.0 min |
  | #3903 | 42168986509 | failure | 47 run / 3 replayed, 22.8 min | 19 run / 1 replayed, 17.7 min | 19 run / 1 replayed, 21.9 min |
  | #3901 | 42096194285 | success | 30 run / 11 replayed, 25.7 min | 17 run / 1 replayed, 24.9 min | 17 run / 1 replayed, 20.6 min |
  | #3900 | 42094103599 | failure | 35 run / 6 replayed, 31.7 min | 18 run / 1 replayed, 27.0 min | 18 run / 1 replayed, 21.1 min |
  | #3896 | 42036556197 | success | 34 run / 4 replayed, 32.7 min | 18 run / 0 replayed, 23.7 min | 18 run / 0 replayed, 26.1 min |
  | #3894 | 42026208668 | success | 25 run / 0 replayed, 27.6 min | 5 run / 0 replayed, 20.9 min | 5 run / 0 replayed, 13.3 min |
  | #3891 | 41985291875 | failure | 34 run / 6 replayed, 31.1 min | 16 run / 1 replayed, 21.9 min | 16 run / 1 replayed, 27.8 min |
  | #3881 | 41847627149 | failure | 39 run / 0 replayed, 30.8 min | 23 run / 0 replayed, 23.9 min | 23 run / 0 replayed, 26.8 min |
- |-
  **Prototypes measured on hosted runs** (two to four runs per side, every side a core-touching PR on the same session tree; minutes are job wall clocks read by the harness from each run's jobs listing).

  | Side | Runs | test-linux min | test-macos min | test-windows min |
  | --- | --- | --- | --- | --- |
  | control (no recorder) | 43390706286, 43392805774 | 27.8 / 22.4 (mean 25.1) | 18.6 / 32.5 (mean 25.5) | 27.8 / 33.1 (mean 30.5) |
  | dev opt-level 1 | 43392807274, 43392811473 | 28.1 / 23.8 (mean 25.9) | 36.5 / 39.6 (mean 38.1) | 42.4 / 41.0 (mean 41.7) |
  | RUST_TEST_THREADS=8 | 43394779047, 43394820753, 43394772150, 43394820160 | 27.6 / 23.6 / 27.4 / 25.1 (mean 25.9) | 31.4 / 34.6 / 34.0 (mean 33.3) | 34.0 / 23.0 / 26.7 / 34.4 (mean 29.5) |
  | recorder (sampler + test runner) | 43390686747, 43394638992, 43390692001 | 28.4 / 24.3 / 26.6 (mean 26.4) | 34.1 / 30.3 / 30.0 (mean 31.4) | 33.2 / 33.1 / 33.9 (mean 33.4) |
  | recorder + incremental release | 43390696539, 43390701482 | 25.6 / 26.9 (mean 26.3) | 26.9 / 37.9 (mean 32.4) | 30.8 / 33.7 (mean 32.2) |

  | Comparison | Δ test-linux | Δ test-macos | Δ test-windows | control spread (max − min of identical trees) L / M / W |
  | --- | --- | --- | --- | --- |
  | dev opt-level 1 vs control | +0.8 min | +12.5 min | +11.2 min | 5.5 / 13.9 / 5.3 |
  | RUST_TEST_THREADS=8 vs control | +0.8 min | +7.8 min | -0.9 min | 5.5 / 13.9 / 5.3 |
  | recorder (sampler + test runner) vs control | +1.3 min | +5.9 min | +2.9 min | 5.5 / 13.9 / 5.3 |
  | recorder + incremental release vs recorder | -0.2 min | +1.0 min | -1.1 min | 5.5 / 13.9 / 5.3 |

  The control's own spread is the yardstick: two runs of an identical tree differed by 5.5 min on Linux, 13.9 on macOS and 5.3 on Windows (macOS 18.6 against 32.5 min on the same `macos-26-arm64` image, version 20261102.0473.1). A difference inside that spread is not a measured saving.
  - **Faster runtime code: dev `opt-level=1`** does not pay. macOS +12.5 and Windows +11.2 min, outside the spread: compile-link(plain) rose to 13.2 to 15.4 min against the control's 3.6 to 8.1, because the restored cache was built at opt-level 0 and every dependency rebuilt, while test-run(plain) did not fall (macOS 10.5 and 12.2 min against 5.6 and 9.1; Windows 19.4 and 19.1 against 14.4 and 16.8). With a cache warmed at opt-level 1 the compile cost would return, but the tests measured no faster, so the saving is 0 min on every platform (estimate for the warmed case). Proves the same.
  - **Faster tests that prove the same: `RUST_TEST_THREADS=8`** (8 test threads per binary on 3- and 4-vCPU runners; four runs on Linux and Windows, three on macOS): +0.8 / +7.8 / −0.9 min, every one inside the control's spread, and every macOS run (31.4 to 34.6 min) slower than the control's mean. No measured saving.
  - **Incremental release build of the distribution journeys' wheel**: against the recorder control −0.2 / +1.0 / −1.1 min, inside the spread; compile-link(release) on macOS 3.2 and 4.3 min against 3.1 to 3.7. It would also prove less (the wheel the journey installs would be built with incremental codegen units, unlike the one release CD ships). Does not pay.
  - **Cold compile and link at today's layout** (the one hosted run whose Rust cache missed, 43378508844, from #3998): plain compile-link 5.88 / 4.69 / 9.34 min of wall share on Linux / macOS / Windows, instrumented 2.65 (Linux), release 2.03 / 2.58 (macOS / Windows); macOS's job failed at 18.4 min, so its figure is partial. Against the warm control's plain compile-link (Linux 2.8 to 4.2, macOS 3.6 to 8.1, Windows 4.9 to 6.5), the restored cache is worth at most a few minutes per job, which is also the cost a layout change pays once when it invalidates the cache.
- |
  **Pyramid and crate-split prototypes: measured on matched hosted runs; their per-PR prices are a model's estimates.** The split shows **no saving distinguishable from the controls' run-to-run spread** on any platform, and is not recommended (manager ruling c11.1; evidence under `spike/ci-profile/data/prototypes/`, commands in `spike/ci-profile/prototypes/README.md`).

  Kept apart below, because they answer different questions:
  - **Measured job shortening, the evidence:** each matched hosted run's job wall clock and Nx gate span (first task start to last task end), control against prototype.
  - **Estimated job shortening, a model (`critical_path.py`):**
    - It re-times each population PR's own run over Nx's own task graph at that PR's commit (`taskgraphs.py`, `../taskgraphs/`; all 2,433 executed tasks are in their graphs).
    - A change moves only along a dependency the run waited on: the dependent started within 2 s of it ending. Two tasks that ran one after the other without an edge are independent.
    - A task that waited on anything else keeps its observed start, so a freed slot is never reassigned and contention is not modelled.
    - Its check against measurement, `model_on_control_s`, applies the measured task changes to each control's own timeline.
  - **Attribution and work, never minutes saved:** wall share (`share_saved_s`) and Linux's summed nextest test-seconds (`work_saved_s`).

  The timestamp-chain pricing of an earlier revision is withdrawn. Its regressions, `tasks_that_ran_one_after_another_without_an_edge_are_independent` and `shortening_one_path_leaves_a_competing_dependent_path_where_it_was` in `test_prototype_replay.py`, fail under it.

  **The split comparison, and the runs discarded on the way to it.**
  - *Discarded, invalid:*
    - #4012/#4017 scheduled 47 / 34 / 34 tasks against 35 / 23 / 23, through the split's own manifest edits.
    - #4021/#4023 still scheduled two extra Linux coverage units, and executed ten tasks its control replayed.
    - #4025/#4027 had matched selection, but the patch did not register its new crate for release or in the coverage merge, so its distribution journeys and Linux's coverage merge failed early.
    - #4030/#4031's Linux job lacked the new unit in `scripts/coverage-read-sets.json`, so `github-nx-policy:coverage-read-set` failed and `workspace:test`, which waits on it, did not run.
    - Every early failure changed the split side's workload and contention, so none of those runs is compared, and leaving the failed tasks out afterwards (an earlier revision's `--leave-out`) is not used.
  - *The repaired patch.* It registers the crate for release (`release-plz.toml`, `release-targets.toml`, `scripts/product_versions.py`, `scripts/release_cd.py`), in the coverage merge (`workspace/project.json`) and in the coverage read sets.
    - It was validated locally before each launch (`split-validation.txt`): the distribution journeys, the uncached coverage merge (95.10% of lines, all 15 units, the floor 92.88%) and the Linux gate itself.
    - The gate's only local failures, the two SDK coverage units, fail identically on the unpatched head on this host and pass on every hosted run.
  - *The matched runs compared:*
    - Controls #4024 and #4026 (runs 43457659504, 43457450179).
    - macOS and Windows: split #4030 and #4031 (43504759508, 43504773180).
    - Linux: split #4034 and #4035 (43528710535, 43528719773). These carry, as their only further change, the macOS and Windows test jobs set to `if: false` (manager ruling a19); their `ci.yml` blob is `d935e7a1` against `84938113`.
    - Every one of those jobs ran its whole workload. One Windows distribution journey (43504759508) failed only in its last cleanup (`rm: … Device or resource busy`), after its work had run.
  - *How they are matched:*
    - Both sides compare against the demonstration without its marker comment (`--marker-base`) and read no Nx cache. macOS and Windows schedule exactly the same tasks.
    - Linux's split side schedules the control's 38 tasks plus its own crate's coverage unit `onetaskgraph-template:coverage`, declared `--side-only`. Linux coverage is never narrowed and the split adds a unit; in an ordinary PR that unit would replay from the Nx cache unless the PR changed it, whereas here, with no cache on either side, it executes (288 and 312 s).
    - Nothing executed on one side that the other replayed.
  - *Within those runs:*
    - macOS's split runs started about five hours after its controls, and every task the split cannot reach ran 1.5 to 2 times faster on them (`onetaskgraph-github-projects:test` 269 and 361 s against 557 and 428; `onetaskgraph-linear:test` 406 and 503 against 787 and 613). The macOS difference measures the runner, not the split.
    - On Linux, the credentialed Linear journey that talks to the real Linear API took 746 and 727 s on the split runs against 556 and 524, and in control 43457659504 it failed outright.
    - Windows' unreachable tasks agree to within run noise.

  | | test-linux | test-macos | test-windows |
  | --- | --- | --- | --- |
  | pyramid (controls 43422783410, 43422844825 → 43422794958, 43422857812): job wall (min) | 25.9, 25.0 → 25.0, 26.8 | 32.0, 29.0 → 29.5, 29.2 | 26.5, 34.0 → 33.6, 33.1 |
  | pyramid: Nx gate span (min) | 24.0, 23.3 → 23.3, 24.8 | 29.1, 26.5 → 27.3, 26.7 | 23.5, 30.3 → 29.9, 29.2 |
  | pyramid: measured gate shortening, control mean less pyramid mean | -25 s | +44 s | -160 s |
  | pyramid: model's gate shortening on each control's own timeline | -20, -21 s | +19, +19 s | -66, -14 s |
  | split: job wall (min) | 29.9, 24.7 → 31.7, 31.3 | 37.6, 30.1 → 20.8, 25.9 | 35.4, 32.5 → 33.0, 34.1 |
  | split: Nx gate span (min) | 27.9, 22.9 → 29.8, 29.5 | 34.5, 27.6 → 18.8, 23.2 | 31.6, 28.8 → 29.7, 30.4 |
  | split: measured gate shortening, control mean less split mean | -257 s | +602 s | +10 s |
  | split: model's gate shortening on each control's own timeline | +5, -106 s | +74, +74 s | +8, +8 s |

  Reading the split row platform by platform:
  - **Windows:** measured +0.2 min (shorter) on the gate span, inside the controls' own 2.8-minute spread (31.6 against 28.8 min). No saving distinguishable from that spread; the model's estimate beside it is +0.00 min.
  - **macOS:** measured +10.0 min shorter, outside the controls' 6.9-minute spread (34.5 against 27.6 min), but every task the split cannot reach ran 1.5 to 2 times faster on the split runs (below), so the difference is the runner's and none of it is attributable to the split. No saving distinguishable from the controls' spread; the model's estimate beside it is +0.00 min.
  - **Linux:** measured 4.3 min longer, inside the controls' 5.0-minute spread (27.9 against 22.9 min). Part of that is its own coverage unit, which a cached PR would replay; part is the live Linear journey's own variance. No saving distinguishable from that spread; the model's estimate beside it is +0.00 min.

  So the **split's measured result is decided, by manager ruling c11.1: no saving distinguishable from the controls' run-to-run spread** on every platform, the spread (Linux 5.0, macOS 6.9, Windows 2.8 min of gate span) being the bound on any saving it could hold, with the model's +0.00 min per PR beside it. The spike recommends against the split on that basis. No further run was taken to narrow it. The pyramid's measured gate changes are all inside their controls' spread too.

  Work metric (Linux nextest summed test-seconds, not elapsed time): the pyramid removes 514 of the `onetaskgraph-e2e` binary's 1,830 test-seconds.

  **Estimated shortening, replayed over the 30 PRs** (`replay-elapsed.txt`; typical = the 18 PRs whose diff changes a crate's `src/`, `tests/`, `build.rs` or `Cargo.toml`; all 90 PR jobs had intervals and graphs):

  ```
  pyramid: runs [43422794958, 43422857812] against control [43422783410, 43422844825]
    test-linux   measured gate span (min) 24.0, 23.3 -> 23.3, 24.8, wall 25.9, 25.0 -> 25.0, 26.8; estimated shortening +0.00 min/PR over 30, +0.00 over the 18 typical (0 without intervals)
    test-macos   measured gate span (min) 29.1, 26.5 -> 27.3, 26.7, wall 32.0, 29.0 -> 29.5, 29.2; estimated shortening +0.00 min/PR over 30, +0.00 over the 18 typical (0 without intervals)
    test-windows measured gate span (min) 23.5, 30.3 -> 29.9, 29.2, wall 26.5, 34.0 -> 33.6, 33.1; estimated shortening -0.32 min/PR over 30, -0.40 over the 18 typical (0 without intervals)
  split: runs [43528710535, 43528719773] against control [43457659504, 43457450179]
    test-linux   measured gate span (min) 27.9, 22.9 -> 29.8, 29.5, wall 29.9, 24.7 -> 31.7, 31.3; estimated shortening +0.00 min/PR over 30, +0.00 over the 18 typical (0 without intervals)
  split: runs [43504759508, 43504773180] against control [43457659504, 43457450179]
    test-macos   measured gate span (min) 34.5, 27.6 -> 18.8, 23.2, wall 37.6, 30.1 -> 20.8, 25.9; estimated shortening +0.00 min/PR over 30, +0.00 over the 18 typical (0 without intervals)
    test-windows measured gate span (min) 31.6, 28.8 -> 29.7, 30.4, wall 35.4, 32.5 -> 33.0, 34.1; estimated shortening +0.00 min/PR over 30, +0.00 over the 18 typical (0 without intervals)
  ```

  - **Pyramid:** an estimated **+0.00 / +0.00 / −0.40 min** per typical PR on Linux / macOS / Windows.
  - **Split:** an estimated **+0.00 min** on every platform. No population PR's job ended waiting on a core task. One of the 30, #3972, changed a file the split moves, so the replay adds the new crate's tasks to its jobs, running beside the gate from its start (Linux's coverage unit at its measured 300 s; macOS and Windows scheduled no new-crate task, so none is added there); they end before that job's gate does.

  These are a model's estimates, under its stated limits. For the split they agree with the measurement: nothing distinguishable from zero.

  The cold build of every test target with no Rust cache (`ciprofile.py compile`) predates the repair: compile-only runs 43422822720 and 43422893836 against 43422834977 and 43422905157. Its build step ran the same whole-workspace build on both sides and completed on all: today's layout 4.12 / 6.40 min on Linux, 6.78 / 7.55 on macOS, 6.50 / 8.07 on Windows; split 6.27 / 4.28, 2.47 / 7.13, 8.58 / 9.20, every difference inside its own side's spread. The repair adds no source to that build.
- |-
  **Options, priced** (minutes per typical PR, i.e. Rust-touching, mean of 18, and over all 30. Each price is an **estimated job shortening** from a model: each PR's run re-timed over Nx's own task graph at its commit, with the tasks an option replays taking no time, or a prototype's tasks their measured elapsed change (`critical_path.py`; `spike/ci-profile/data/replay30.txt`, `prototypes/replay-elapsed.txt`). The measured evidence for the prototypes is the matched hosted runs. Wall-share attribution, the minutes an option's tasks held a slot, is given beside it, and is never the price.)
  1. *Run the distribution journeys only when the publishable surface changes* (finer key).
     - 0 min on a typical PR, because it changes crate source. Over all 30 PRs, estimated shortening: Linux 0.05, macOS 0.37, Windows 0.28 min per PR (wall-share attribution 0.05 / 0.57 / 0.44). An earlier revision priced it at 1.14 / 1.68 / 1.18 min of wall share with the rule's first, wider publishable surface; the committed `replay.py` derives that surface from the scripts' own commands, and these figures are its output (`replay30.txt`, from the committed history and inputs).
     - Effort: `scripts/ci-job.sh` and the `scripts` project's inputs, 1 dispatch. Proves the same on the PRs it skips.
     - It deliberately reverses AGENTS.md's `distribution-check`/`distribution-test` rule, so it is for the user's decision.
  2. *Dependents keyed on their dependencies' `src/` and manifests, not `tests/`* (finer key).
     - Over all 30 PRs, estimated shortening: Linux 0.21, macOS 0.00, Windows 0.04 min per PR (wall-share attribution 0.46 / 0.27 / 0.54).
     - Effort: `nx.json` named inputs and about 23 `project.json` files, with `scripts/check-nx-graph.sh` and `.github/nx-policy`, 1 dispatch. Proves the same.
  3. *Take formatter, lint and prose files out of Rust test keys*: estimated shortening 0.04 min per PR on Linux and 0.00 elsewhere (wall-share attribution 0.19 on Linux, 0 elsewhere). Not worth a dispatch.
  4. *Split `onetaskgraph-core`: its `template` module into its own crate* (modularization; **measured** on matched hosted runs, prototype `spike/ci-profile/prototypes/split-template-crate.patch`, repaired twice and validated; see "Pyramid and crate-split prototypes" above). It moves a module that depends on nothing else in core, with its 55 tests, behind a re-export.
     - **Net savings: no saving distinguishable from the controls' run-to-run spread** on any platform (manager ruling c11.1). The bound is that spread of gate span: Linux 5.0, macOS 6.9, Windows 2.8 min. Measured: Linux 4.3 min longer, Windows 0.2 min shorter, both inside it; macOS 10.0 min shorter, outside it but matched by tasks the split cannot reach running 1.5 to 2 times faster, so it is the runner's.
     - The model's estimate is **+0.00 min** per typical PR on every platform. The cold build is unchanged within its spread.
     - Cost, stated rather than measured: a landed split must also re-record `github-nx-policy`'s historical-hash fixture, which holds core's inputs to an earlier commit.
     - Effort: 57 files, 1 dispatch. Proves the same.
     - **Recommended against**: on the measurement and the model alike it saves nothing that can be told from noise, and it costs 57 files and a re-recorded fixture.
  5. *Incremental release build for the distribution journeys*: measured above, inside the spread; 0 min on every platform. Effort: one variable exported in the distribution phase of `scripts/ci-job.sh`, 1 dispatch. Proves less (the journey's wheel is built unlike the shipped one). Not recommended.
  6. *Faster runtime code*: dev `opt-level=1`, measured above, 0 min saved and +11 to +12.5 min on macOS and Windows while the cache is rebuilt. Effort: one `[profile.dev]` line in `Cargo.toml` and a new cache family, 1 dispatch. Proves the same. The ceiling for making the binary itself cheaper to start is its spawn time: 2.54 / 1.83 / 4.05 min of the recorded jobs (Linux / macOS / Windows), so a 20% faster start-up would be about 0.5 / 0.4 / 0.8 min (estimate). Not prototyped.
  7. *Faster tests that prove the same*: `RUST_TEST_THREADS=8`, measured above, 0 min saved on every platform (inside the spread). Effort: one variable in `ci.yml`'s three test steps, 1 dispatch. Proves the same. The two `cache_budget` journeys (Linux 363 + 355 s, macOS 243 + 158 s) are the single largest test costs, but they are not what the platforms' jobs end waiting on, so shortening them alone saves under 1 min per job (estimate, from the timeline; the dependency-aware model of `critical_path.py` was not run for them). Not prototyped.
  8. *Pyramid rebalance of `onetaskgraph-e2e`* (**measured** on a sample with identical selection, prototype `spike/ci-profile/prototypes/pyramid-e2e-journeys.patch`). The sample marks `#[ignore]` on the 121 journeys of `section_links`, `assets` and `held_writes`, whose same-named in-process `onetaskgraph-core` suites keep running. **Estimated job shortening per typical PR: +0.00 / +0.00 / -0.40 min** on Linux / macOS / Windows (a model's estimate); the matched hosted runs measured no shortening outside their noise. It removes 514 of the e2e binary's 1,830 Linux test-seconds (work, not elapsed), but no PR's Linux job waited on the e2e task at its end, and its macOS and Windows elapsed changes are inside noise. Extending it to all 19 such modules (378 journeys) scales a figure indistinguishable from zero, so it is not priced further. Effort: 3 files, 1 dispatch for the sample; 2 to 3 for all 19. **Proves less** for no measurable saving, per behaviour:
     - section links, assets, rendered and copy snapshots: an in-process test still proves the rewrite, the asset bytes and digests and the request counts against the loopback stand-ins; only the journey proves the command line's flags, the rendered output and exit code a user sees, and the files written through the process boundary.
     - held writes, update, priority, metadata, delivery and comments: the in-process test still proves the version guard and every field written; only the journey proves the `--if-version` and other flags parse, refusals reach stderr with their next action and the right exit status.
     - copy, copy links, locate, relations, plan graph, batch, templates, commented-since and human routes: the in-process test proves the engine's answer; only the journey proves the stdio plugin seam (§3.1's cleared environment) and the machine-readable output against the schema the binary emits.
     For the user's decision; the spike adopts none.
  9. *Run the distribution journeys only when the publishable surface changes*, *dependents keyed on dependencies' `src/` only*, *formatter files out of Rust keys*: replayed above (items 1 to 3), 0 min on a typical PR because its diff changes crate source; over all 30 PRs an estimated shortening of at most 0.37 min (macOS) per PR.
- |-
  **Findings about the plan.**
  - The live lanes take the account's rate limit, which the host's own agent reads share. While this host was draining the allowance, a demonstration's live asset journey paused 25 min by design and every job timed out. Measurement spikes and the live lane cannot both spend that allowance at once.
  - Linux spends 1.7 to 3.4 min per Rust PR (mean 1.6 → 3.4 before → after) blocked on cargo's build-directory lock between concurrent Nx tasks. Parallelism above one cargo invocation buys less than its slot count.
- |-
  **What this spike still leaves unmeasured**, stated rather than papered over:
  - a prototype of faster runtime code beyond `opt-level=1` (its ceiling is priced from spawn records, labelled an estimate);
  - the pyramid beyond its three-module sample (its sample's estimated shortening is indistinguishable from zero, so no extrapolation is priced);
  - summed test-seconds on macOS and Windows for the prototype runs (no recorder ran there, so the work metric is Linux's alone; elapsed task and job times are measured on all three);
  - the model's limits: a freed slot is never reassigned and contention between concurrent tasks is not modelled, so its estimates are checked against, and subordinate to, the matched runs' measured gate spans;
  - any split saving smaller than the controls' run-to-run spread (Linux 5.0, macOS 6.9, Windows 2.8 min): by manager ruling c11.1 the comparison is finished at "no saving distinguishable from that spread", and no runs matched in time were taken to narrow it. macOS's split runs started five hours after their controls and ran every unreachable task 1.5 to 2 times faster; Linux carries the live Linear lane's variance and the new crate's coverage unit executing without a cache. Every platform has two matched runs per side, Linux's split runs with the macOS and Windows jobs switched off (ruling a19);
  - the hosted A/B of #3956 and its parent, not run because the log-based attribution decided the cause (finding above);
  - run 43394820160's test-macos (#4008's second head `b9504265`): queued behind the account's macOS concurrency from 08:38Z and still queued at the last read (11:44Z). By manager ruling a8 it was not cancelled: #4008 was closed with that run left queued, and its macOS leg is not read; the side's two complete macOS runs (43394779047 and 43394820753) and a third (43394772150) are read.
- '**Judged diff lint: an accepted gap for this throwaway harness** (manager rulings a4 and c4.5). `just lint-llm-diff origin/main` on the branch (llmlint history `20261113T064655Z-d7ece`, on `655b7bdb`) failed 12 rules over `spike/ci-profile/`: `async_typed_clients_at_boundaries`, `boundary_inputs_validated`, `changed_behavior_has_e2e`, `cli_output_contract`, `comments_earn_their_place`, `contracts_have_one_source_or_a_drift_gate`, `modern_domain_modeling`, `names_match_behavior`, `new_code_lands_in_a_project`, `no_injection_from_untrusted_input`, `structural_pattern_matching`, `suppressions_justified`. Later commits (`fb794565` to `b3c574eb`) answered several of them (shell text and artifact paths refused, typed records, a project of its own, commands driven through their entry points); later judged runs were made by the previous dispatch and their remaining findings were not re-listed before it ended. Per ruling c4.5 this dispatch did not re-run or rework it. Its deterministic lint over the diff — the `spike-ci-profile` project''s `ruff format --check`, `ruff check` and `ty check` — is clean on the finished commit, after this dispatch wrapped three over-long test lines (`1fa3bd49`) and added the compile reader, the cold-build and per-job exclusion switches, task intervals in the history (with a fix for a gate whose log group opens beside another, which had made the 22 older log-read jobs read as task-less; every cited profile and all 2,664 history tasks read the same after it) and the elapsed-time prototype replay, each with tests driven through its entry point. A task that promotes any of this harness into the repository must meet all 12.'
harness: |-
  `spike/ci-profile/` on the branch; finished commit `697233f3`. **One measurement is one command:**

      python3 spike/ci-profile/ciprofile.py profile <run id> [--jobs test-linux,test-macos,test-windows] [--json]

  It reads one hosted run of `ci.yml` (its jobs, each test job's log, the Nx executions each job uploads and, on a demonstration run, the `spike-profile-<job>` recorder artifact), caching every read under `$CIPROFILE_CACHE` (default `target/ci-profile`) and refusing any read below 2,500 REST or 2,000 GraphQL remaining. Per test job it prints: the wall clock divided into categories that sum to it with an `unattributed` remainder; the rust-cache restore (seconds, size, hit); the Nx entries the job's sidecar stored; the Nx tasks by share of the wall clock; the latest-finishing task chain; on a demonstration run the test step divided by leaf-process category and the processes that ended per category; every test binary's seconds; and the 20 slowest tests. `--json` also writes the whole summary to `$CIPROFILE_CACHE/profile-<run>.json`. Exit 0 printed; 2 usage; 3 refused below the rate floor; 4 a job unfinished or a record of an unread shape.

  The rest of the harness: `ciprofile.py compile <run id>` (a compile-only demonstration: the build step's minutes and cargo's own `--timings` report, its units, span, summed seconds and each workspace crate's seconds), `prototype_replay.py deltas|replay` (a prototype's measured elapsed task change against control runs of identical Nx selection, refused otherwise, with wall share and summed test-seconds kept as separate attribution and work figures, and the model checked on the control runs' own timelines; then the estimated job shortening over each population PR's own Nx task graph), `ciprofile.py history --pin H` (re-reads exactly the runs a history cites, adding each task's interval), `taskgraphs.py history|runs` (Nx's own task graph at each PR's or demonstration run's commit, read in a temporary worktree), `critical_path.py` (the dependency-aware re-timing both replays use), `marker_base.py` (the comparison a `--marker-base` gate uses), `prototypes/` (the pyramid and crate-split patches and how each demonstration is built from them), `ciprofile.py durations <since> <until>` (median job durations), `ciprofile.py history <prs.json> --out F` (each PR's first run, task by task), `replay.py` (which key inputs each re-run matched, and the finer-key options replayed; its inputs are committed under `spike/ci-profile/data/`), `measurement_only.py` (the demonstration edits: no cache write of any kind, the recorders, `--step-env` prototypes, `--job-exclude` to hold a job's selection, `--marker-base` to give both sides the same selection, and `--cold-rust --compile-only` for a cold build alone), `sampler.py` (psutil process-table recorder, one tick a second) and `testrunner.py` (a cargo target runner recording libtest's per-test `exec_time`). Tests: `python3 -m unittest discover -s spike/ci-profile -p 'test_*.py'` (114 tests, 1 skipped where psutil is absent: OK on `697233f3`), with `ruff format --check`, `ruff check` and `ty check` clean on `697233f3` (the `spike-ci-profile` Nx project's `format-check`, `lint`, `typecheck` and `test` targets).

  **Its output on the finished commit** `697233f3`, offline from the read cache (`CIPROFILE_OFFLINE=1 CIPROFILE_CACHE=<cache> python3 spike/ci-profile/ciprofile.py profile 43390692001`, exit 0, `(GitHub reads this invocation: 0)`), first lines of the Linux and macOS sections (the full 346 lines are what every per-platform figure below for run 43390692001 is read from):

  ```
  run 43390692001  head e63933bc8eab  branch spike-otg-base-2  created 2026-11-13T08:07:01Z

  == test-linux (job 131769590009, success) wall 26.58 min; Nx tasks: 34 run, 11 replayed [executions artifact]
    category                          minutes   share
    test-run(instrumented)             8.63   32.5%
    test-run(plain)                    6.42   24.1%
    compile-link(plain)                3.19   12.0%
    wait:cargo-file-lock               1.97    7.4%
    compile-link(instrumented)         1.72    6.5%
    scripts-and-other-tasks            1.31    4.9%
    setup                              1.24    4.7%
    budget-checks                      1.02    3.8%
    cache-restore                      0.62    2.3%
    nx-planning                        0.28    1.1%
    report                             0.08    0.3%
    unattributed                       0.08    0.3%
    cache-save                         0.02    0.1%
    cache-restore(nx)                  0.00    0.0%
    (sum)                             26.58
    rust-cache restore: 34.3 s, 1362 MB, hit
    Nx sidecar: stored 0 entries (0 bytes), replayed 11
  ...
  == test-macos (job 131769590459, failure) wall 29.97 min; Nx tasks: 23 run, 1 replayed [executions artifact]
    category                          minutes   share
    test-run(plain)                    7.89   26.3%
    compile-link(plain)                7.49   25.0%
    scripts-and-other-tasks            6.06   20.2%
    compile-link(release)              3.50   11.7%
    budget-checks                      2.25    7.5%
    setup                              1.98    6.6%
    cache-restore                      0.42    1.4%
    nx-planning                        0.25    0.8%
    report                             0.07    0.2%
    unattributed                       0.06    0.2%
    (sum)                             29.97
    rust-cache restore: 20.7 s, 615 MB, hit
    Nx sidecar: stored 0 entries (0 bytes), replayed 1
  ...
  (GitHub reads this invocation: 0)
  ```

  Figures read from it: test-linux wall 26.58 min with `unattributed` 0.08 min (0.3%); test-macos 29.97 min, compile-link(plain) 7.49 min (25.0%).

  A cold build is read the same way, on the same commit (`CIPROFILE_OFFLINE=1 CIPROFILE_CACHE=<cache> python3 spike/ci-profile/ciprofile.py compile 43422893836`, exit 0), first lines:

  ```
  run 43422893836  head 107be0519450  branch spike-otg-cctl-2  created 2026-11-13T12:10:09Z

  == test-linux (job 131876998357, success) wall 7.82 min
    build step: 6.40 min
    cargo timings: 457 units, span 6.40 min, summed 23.49 min
      onetaskgraph-core                           188.4 s
      onetaskgraph-github-projects                105.6 s
      onetaskgraph-e2e                             43.5 s
  ```

  Figure read from it: today's layout builds every test target cold on Linux in 6.40 min, `onetaskgraph-core` taking 188.4 s of it.
method: |-
  **Sources.** Hosted runs of `ci.yml`, read through REST only (`gh api repos/...`). For each run the harness reads the jobs listing, each test job's log once, and its `nx-cache-entries-<job>` (or `spike-nx-executions-<job>`) `executions.json`. Every read is cached under `$CIPROFILE_CACHE` and refused below 2,500 REST or 2,000 GraphQL remaining. Local work used only counts and resolved inputs (`nx show target inputs`); nothing was timed on this host.

  **How a job's wall clock is divided** (`spike/ci-profile/jobprofile.py`).
  - Every second of the job, from the jobs API's `started_at` to `completed_at`, is assigned once.
  - Steps outside the test step are classed setup, cache-restore, cache-save or report. The restore inside setup-uv, setup-bun and Swatinem/rust-cache is read from the log's own groups. The background Nx sidecar step owns no second.
  - Inside the test step, each instant is split equally among the Nx tasks running then, from `executions.json` (millisecond start and end) or else the log's `> nx run` headers. An instant with no task is `nx-planning`.
  - Inside an executed task, time is classed as:
    - compile-link (dev/plain, instrumented for `coverage` and `coverage-build`, release for the distribution journeys' maturin build): the task's own `Compiling`…`Finished <profile>` windows, or the time until its first test starts;
    - `wait:cargo-file-lock`: `Blocking waiting for file lock` until the task's next line;
    - test-run: the rest.
  - A replayed task is `cache-restore(nx)`.
  - So concurrent tasks share and nothing is counted twice. `unattributed` is the remainder, at most 0.3% on every cited job.

  **The recorder** for categories the logs cannot see: `sampler.py` reads psutil's process table each tick and records leaf-process categories, CPU and lifetimes. These cover the CLI under test, git, Python and uv, node, bun, onevcs, gh, curl/wget, tar, rustc, the linkers, llvm-cov and the test binaries. A tick where work processes used under 10% of one CPU is a wait (sleep, poll, network or timeout). `testrunner.py` is a cargo target runner that records each libtest binary's per-test `exec_time` through `--report-time`. Both run only on a demonstration (`measurement_only.py`).

  **Population.**
  - Durations: every successful test job of completed non-release PR runs created 2026-11-03 to 2026-11-09T23:14Z (before #3956 merged) against 2026-11-09T23:15Z to 2026-11-13. That is `ciprofile.py durations` over 128 runs.
  - Reuse: the 30 most recent merged non-release PRs, #3881 to #3993, each PR's *first* CI run, with its Nx state as main left it.
  - Typical PR: one whose squash diff changes a crate's `src/`, `tests/`, `build.rs` or `Cargo.toml`. That is 18 of the 30; the other 12 change only CI, scripts or docs.

  **Runs this spike started.** Every one carries every cache write disabled by `measurement_only.py`: uv `save-cache: false`, setup-bun keeps no cache, every Rust `cache-save-if: false`, both `actions/cache/save` steps `if: false`, the Nx sidecar `CACHE_READ_ONLY: '1'`, and the Nx report uploaded under a name no promotion reads. Each test job's log shows it: in all 38 finished job logs of the 13 runs below there is no `Cache saved`/`Saving cache` line, `CACHE_READ_ONLY` is set, and the harness reads `Nx sidecar: stored 0 entries (0 bytes)` (counted over the cached logs; listing in the table). The only `Cache not found` lines are the `release-plz` and `prune-superseded` tool caches, whose save step is the disabled `actions/cache/save`. No cache entry was deleted. No run was cancelled by this spike.

  - Session draft #3995: every commit carries `[skip ci]`, so it started no run (the Actions listing of runs created after 08:00Z on 2026-11-13, read 11:24Z, holds none on the session branch).
  - Earlier demonstrations, already reported and closed: #3996 run 43369924587 (all three jobs hit the shipped 40-minute timeout; cited for recorder evidence only), #3997 run 43378506851 and #3998 run 43378508844 (closed 07:54Z). In 43378506851 the recorder leaked `RUSTC_BOOTSTRAP=1` into the cargo a test starts, so any cargo a test starts saw a different environment and rebuilt; that, rather than the sampler, is the likeliest cause of that run's several-fold slowdown of the loopback suites (an inference: the recorded runs after the fix, `eebb3129`, cost the measured amounts below). 43378508844 set `CARGO_PROFILE_RELEASE_INCREMENTAL` at job level, which rust-cache folds into its key, so every restore **missed**; `measurement_only.py --step-env` now exports a prototype's variable inside the test step only.
  - **The prototype set, #3999 to #4008** (all read and closed with their branches deleted, #3999 to #4007 at 11:45Z and #4008 at 11:46Z; no DO NOT MERGE pull request of this spike is open), each `DO NOT MERGE: spike-otg-<side>-<n>`, based on the session branch, each one demonstration commit ahead of it that adds a comment line to `crates/onetaskgraph-core/src/lib.rs` (a core-touching PR, the typical PR's pattern) and the measurement-only `ci.yml`. Sides: *control* (no recorder), *dev opt-level 1* (`CARGO_PROFILE_DEV_OPT_LEVEL=1`), *RUST_TEST_THREADS=8* (control plus that variable), *recorder* (sampler and libtest runner), *recorder + incremental release* (`CARGO_PROFILE_RELEASE_INCREMENTAL=true`). Outside `spike/` and `ci.yml` every head's tree equals the control's (`git diff 1abf906b <head> -- ':!spike'` lists `.github/workflows/ci.yml` alone, or nothing); inside `ci.yml` each side differs from its control by its one exported variable (or the recorder), and #4007/#4008's first heads also passed `spike-ci-profile` to `--exclude` (their second heads, `25c57cb1`/`b9504265`, do not; that project has no Rust task). The executed `ci.yml` blob is the head's own, because the session branch does not touch `ci.yml`. Nx selection: what `NX_BASE=<session head>` selects for that lib.rs comment, less the workflow-only projects passed to `--exclude` (github-cache-policy, github-nx-policy, github-workflow-policy, github-workflows, linear-documents-policy, report-job, screenshots, scripts); per-job counts in the table. Every Rust cache restore hit. macOS and Windows end `failure` on every run because `github-workflow-policy:rust-cache-action` (20 s) refuses the demonstration's edited `ci.yml`; the job still ran every task, so the wall clock is whole. In 43394779047 one macOS `onetaskgraph-github-projects-e2e` test also failed (490 passed). Linux ends `failure` where `budgets` refused the live lane's figures, and in the two control runs because a credentialed live GitHub test failed: in 43390706286 the asset journey after 887 s (`HTTP 404` verifying an uploaded asset), in 43392805774 the contract journey after 11 s (`error sending request for url (https://api.github.com/graphql)`), which skipped the live e2e task behind it. **So Linux's wall clock in these runs depends on the live lane's outcome on the shared allowance, and no Linux difference below is evidence for or against an option.**

  | PR | Run | Side | Head | `GITHUB_SHA` (merge) | ci.yml blob | Rust cache restore L / M / W (s, size, hit) | Nx entries stored L / M / W | Nx tasks run/replayed L / M / W |
  | --- | --- | --- | --- | --- | --- | --- | --- | --- |
  | #4003 | 43390706286 | control (no recorder) | `1abf906b` | `c7a0387d` | `aeef8422` | 22.3 s 1362 MB hit / 8.9 s 615 MB hit / 18.7 s 613 MB hit | 0 / 0 / 0 | 34/11 / 23/1 / 23/1 |
  | #4004 | 43392805774 | control (no recorder) | `954e2961` | `517afb6c` | `aeef8422` | 35.2 s 1362 MB hit / 12.0 s 615 MB hit / 16.9 s 613 MB hit | 0 / 0 / 0 | 35/10 / 23/1 / 23/1 |
  | #4005 | 43392807274 | dev opt-level 1 | `2f5b60ec` | `fb953eb6` | `aff843af` | 29.9 s 1362 MB hit / 23.5 s 615 MB hit / 18.3 s 613 MB hit | 0 / 0 / 0 | 35/10 / 23/1 / 23/1 |
  | #4006 | 43392811473 | dev opt-level 1 | `16629557` | `fcd96771` | `aff843af` | 32.6 s 1362 MB hit / 16.0 s 615 MB hit / 16.7 s 613 MB hit | 0 / 0 / 0 | 35/10 / 23/1 / 23/1 |
  | #4007 | 43394779047 | RUST_TEST_THREADS=8 | `21734321` | `22a6f582` | `44cb9ec8` | 19.8 s 1362 MB hit / 21.7 s 615 MB hit / 17.5 s 613 MB hit | 0 / 0 / 0 | 35/10 / 22/1 / 23/1 |
  | #4007 | 43394820753 | RUST_TEST_THREADS=8 | `25c57cb1` | `89646701` | `5b71a0f5` | 31.0 s 1362 MB hit / 26.5 s 615 MB hit / 13.4 s 613 MB hit | 0 / 0 / 0 | 35/10 / 22/1 / 23/1 |
  | #4008 | 43394772150 | RUST_TEST_THREADS=8 | `6d741710` | `4c1562e2` | `44cb9ec8` | 22.8 s 1362 MB hit / 28.9 s 615 MB hit / 16.3 s 613 MB hit | 0 / 0 / 0 | 35/10 / 22/1 / 23/1 |
  | #4008 | 43394820160 | RUST_TEST_THREADS=8 | `b9504265` | `e7fad71a` | `5b71a0f5` | 30.3 s 1362 MB hit / not finished / 16.0 s 613 MB hit | 0 / — / 0 | 35/10 / — / 22/1 |
  | #3999 | 43390686747 | recorder (sampler + test runner) | `b5dddd51` | `76ea34b6` | `bf0c64e2` | 17.0 s 1362 MB hit / 21.2 s 615 MB hit / 14.8 s 613 MB hit | 0 / 0 / 0 | 34/11 / 23/1 / 23/1 |
  | #3999 | 43394638992 | recorder (sampler + test runner) | `b5dddd51` | `51e0edb6` | `bf0c64e2` | 30.9 s 1362 MB hit / 18.2 s 615 MB hit / 15.4 s 613 MB hit | 0 / 0 / 0 | 35/10 / 23/1 / 23/1 |
  | #4000 | 43390692001 | recorder (sampler + test runner) | `e63933bc` | `0f977a4b` | `bf0c64e2` | 34.3 s 1362 MB hit / 20.7 s 615 MB hit / 18.6 s 613 MB hit | 0 / 0 / 0 | 34/11 / 23/1 / 23/1 |
  | #4001 | 43390696539 | recorder + incremental release | `67de2ee0` | `e12c99f0` | `dfcc6580` | 30.7 s 1362 MB hit / 13.4 s 615 MB hit / 19.0 s 613 MB hit | 0 / 0 / 0 | 34/11 / 23/1 / 23/1 |
  | #4002 | 43390701482 | recorder + incremental release | `581b9d1f` | `42e0c0ca` | `dfcc6580` | 43.5 s 1362 MB hit / 21.7 s 615 MB hit / 16.7 s 613 MB hit | 0 / 0 / 0 | 34/11 / 23/1 / 23/1 |

  **The pyramid and crate-split set, #4010 to #4019** (all read, and closed with their branches deleted by 14:35Z). Each is session head `18ad12b7` plus one commit built as `spike/ci-profile/prototypes/README.md` states: the measurement-only `ci.yml` without recorders (control blob `aeef8422`, byte-identical to #4003/#4004's control), the same lib.rs comment, and the prototype's patch. `git diff spike-otg-wctl-1 spike-otg-wpyr-1` is the three journey files alone (121 `#[ignore]` lines), `git diff spike-otg-wctl-1 spike-otg-wsplit-1` the split's 51 files alone, and each `-1`/`-2` pair is tree-identical. The cold sides add `--cold-rust --compile-only` (blob `c4a5501c`): the toolchain step's `cache: false` restores and saves no Rust cache, and the gate is one `cargo test --workspace --all-features --locked --no-run --timings`. Every job log shows no cache save, `CACHE_READ_ONLY` set and the sidecar storing 0 entries. The only `Cache not found` lines are the two tool caches, whose save is disabled. Nx selection is what `NX_BASE=18ad12b7` selects for each commit, less the same `--exclude` list; the split's own runs select more, because they change the workspace manifests.

  | PR | Run | Side | Head | `GITHUB_SHA` (merge) | ci.yml blob | Rust cache restore L / M / W | Nx entries stored L / M / W | Nx tasks run/replayed L / M / W |
  | --- | --- | --- | --- | --- | --- | --- | --- | --- |
  | #4010 | 43422783410 | warm control 1 | `e59361fe` | `bf580d77` | `aeef8422` | 36.6 s, hit / 25.9 s, hit / 15.9 s, hit | 0 / 0 / 0 | 35/10 / 23/1 / 23/1 |
  | #4015 | 43422844825 | warm control 2 | `2e2676b4` | `4eed3240` | `aeef8422` | 30.8 s, hit / 13.6 s, hit / 17.4 s, hit | 0 / 0 / 0 | 35/10 / 23/1 / 23/1 |
  | #4011 | 43422794958 | pyramid 1 | `8f14afd4` | `820326d0` | `aeef8422` | 31.1 s, hit / 17.6 s, hit / 17.5 s, hit | 0 / 0 / 0 | 35/10 / 23/1 / 23/1 |
  | #4016 | 43422857812 | pyramid 2 | `0c48fdc5` | `6603393d` | `aeef8422` | 34.9 s, hit / 16.2 s, hit / 17.7 s, hit | 0 / 0 / 0 | 35/10 / 23/1 / 23/1 |
  | #4012 | 43422809261 | split 1 | `f6a907a1` | `e8287b68` | `aeef8422` | 18.4 s, hit / 11.1 s, hit / 14.8 s, hit | 0 / 0 / 0 | 47/3 / 34/1 / 34/1 |
  | #4017 | 43422873045 | split 2 | `d6a90891` | `ff64ab96` | `aeef8422` | 18.9 s, hit / 17.3 s, hit / 21.1 s, hit | 0 / 0 / 0 | 47/3 / 34/1 / 34/1 |
  | #4013 | 43422822720 | cold control 1 | `21aae37a` | `f8bba987` | `c4a5501c` | off (`cache: false`) on all three | 0 / 0 / 0 | none: one `cargo test --workspace --no-run --timings` |
  | #4018 | 43422893836 | cold control 2 | `107be051` | `bb237346` | `c4a5501c` | off (`cache: false`) on all three | 0 / 0 / 0 | none: one `cargo test --workspace --no-run --timings` |
  | #4014 | 43422834977 | cold split 1 | `85161bce` | `9f25a76c` | `c4a5501c` | off (`cache: false`) on all three | 0 / 0 / 0 | none: one `cargo test --workspace --no-run --timings` |
  | #4019 | 43422905157 | cold split 2 | `4230e262` | `8a286849` | `c4a5501c` | off (`cache: false`) on all three | 0 / 0 / 0 | none: one `cargo test --workspace --no-run --timings` |

  **The `--job-exclude` split set, #4020 to #4023, superseded** (all read, and closed with their branches deleted at 15:38Z): its Linux coverage still scheduled two extra units, its split side executed ten tasks the control replayed, and its split side's distribution journeys failed early; it is kept as cited, not compared. Each is session head `f146009e` plus one commit: the control (#4020, #4022: ci.yml blob `aeef8422`, byte-identical to every earlier control) or the split patch with `--job-exclude` holding each job to the control's selection (#4021, #4023: blob `f701d81a`, which differs from the control's only on the three `--exclude=` lines). `git diff spike-otg-mctl-1 spike-otg-msplit-1` is the split's 51 files and those three lines; each `-1`/`-2` pair is tree-identical. Every Rust cache restore hit; every job log shows no cache save, `CACHE_READ_ONLY` set and the sidecar storing 0 entries.

  | PR | Run | Side | Head | `GITHUB_SHA` (merge) | ci.yml blob | Rust cache restore L / M / W | Nx entries stored L / M / W | Nx tasks run/replayed L / M / W |
  | --- | --- | --- | --- | --- | --- | --- | --- | --- |
  | #4020 | 43445538503 | matched control 1 | `43c02938` | `72b79404` | `aeef8422` | 29.7 s, hit / 24.9 s, hit / 16.9 s, hit | 0 / 0 / 0 | 35/10 / 23/1 / 23/1 |
  | #4022 | 43445570366 | matched control 2 | `b9b8ae95` | `68ffe04e` | `aeef8422` | 17.5 s, hit / 13.5 s, hit / 17.3 s, hit | 0 / 0 / 0 | 35/10 / 23/1 / 23/1 |
  | #4021 | 43445553591 | matched split 1 | `5f791781` | `ddbeb7be` | `f701d81a` | 33.7 s, hit / 15.6 s, hit / 15.9 s, hit | 0 / 0 / 0 | 45/2 / 24/0 / 24/0 |
  | #4023 | 43445585382 | matched split 2 | `5fd41f22` | `4859edd8` | `f701d81a` | 33.8 s, hit / 16.0 s, hit / 18.9 s, hit | 0 / 0 / 0 | 45/2 / 24/0 / 24/0 |

  **The matched split set, #4024 to #4027** (read and closed with their branches deleted: the controls at 17:47Z, the split runs at the 19:30Z cut-off of manager ruling a17). Each is session head `7029a947` plus one commit: the measurement-only `ci.yml` with `--marker-base --step-env=NX_SKIP_NX_CACHE=true` (blob `84938113` on all four, byte-identical) and the marker comment, plus, on the split side, `split-template-crate.patch`. `git diff spike-otg-nctl-1 spike-otg-nsplit-1` is the split's 51 files and nothing in `.github`; each `-1`/`-2` pair is tree-identical. Every Rust cache restore hit; every job stored 0 Nx entries and replayed none; no job log holds a cache save.

  | PR | Run | Side | Head | ci.yml blob | Rust cache restore L / M / W | Nx entries stored L / M / W | Nx tasks run/replayed L / M / W |
  | --- | --- | --- | --- | --- | --- | --- | --- |
  | #4024 | 43457659504 | matched control 1 | `8635ef79` | `84938113` | 24.7 s, hit / 22.9 s, hit / 21.1 s, hit | 0 / 0 / 0 | 37/0 / 22/0 / 22/0 |
  | #4026 | 43457450179 | matched control 2 | `89baeab0` | `84938113` | 30.2 s, hit / 13.6 s, hit / 16.6 s, hit | 0 / 0 / 0 | 38/0 / 22/0 / 22/0 |
  | #4025 | 43457723269 | matched split 1 | `00548424` | `84938113` | 24.5 s, hit / not read / 19.4 s, hit | 0 / — / 0 | 38/0 / — / 22/0 |
  | #4027 | 43457727138 | matched split 2 | `5dae86e5` | `84938113` | 29.0 s, hit / 7.8 s, hit / 14.9 s, hit | 0 / 0 / 0 | 38/0 / 22/0 / 22/0 |

  Control 1's Linux job ran 37 tasks, not 38: its credentialed Linear journey failed against the real Linear API, so `onetaskgraph-linear-e2e:test` behind it did not run.

  **The repaired split runs, #4030/#4031 and #4034/#4035.** All four were read and closed with their branches deleted, #4030/#4031 at 21:30Z and #4034/#4035 at 00:20Z, inside manager rulings a18 (cut-off 23:00Z) and a19 (cut-off 01:00Z). Each is session head `7029a947` plus one commit: the repaired `split-template-crate.patch`, the marker comment, and the same measurement-only `ci.yml` as the controls.
  - #4030/#4031 carry blob `84938113`, byte-identical to the controls'. `git diff 00548424 6604aade` is the five release and coverage files of the first repair.
  - #4034/#4035 also carry the coverage-read-set fix and `if: false` on `test-macos` and `test-windows`, blob `d935e7a1`; `git diff 6604aade 5bde5594` is `scripts/coverage-read-sets.json` and those two lines.
  - Their pull-request merge refs join each head to the session branch as it then stood (`4370feaa`, `eefb19c3`), which differs from the controls' only under `spike/`, which no gate task reads.
  - #4030/#4031's Linux jobs are discarded (their coverage-read-set check failed); their macOS and Windows jobs, and #4034/#4035's Linux jobs, are compared.
  - Every Rust cache restore hit; every job stored 0 Nx entries, replayed none and logged no cache save.

  | PR | Run | Head | ci.yml blob | Jobs read | Rust cache restore | Nx tasks run/replayed | Conclusion |
  | --- | --- | --- | --- | --- | --- | --- | --- |
  | #4030 | 43504759508 | `6604aade` | `84938113` | L (discarded) / M / W | 17.1 s / 13.3 s / 14.5 s, hits | 38/0 / 22/0 / 22/0 | L failed (prototype), M success, W failed in final cleanup |
  | #4031 | 43504773180 | `0514e75a` | `84938113` | L (discarded) / M / W | 28.0 s / 18.1 s / 17.5 s, hits | 38/0 / 22/0 / 22/0 | L failed (prototype), M success, W success |
  | #4034 | 43528710535 | `5bde5594` | `d935e7a1` | L | 20.5 s, hit | 39/0 | success |
  | #4035 | 43528719773 | `7f79996c` | `d935e7a1` | L | 18.3 s, hit | 39/0 | success |

  **GitHub API accounting (this dispatch, 11:24Z onward).** One run listing; 6 REST jobs reads of the two `spike-otg-threads-2` runs (at 11:32Z, 11:33Z and 11:44Z: the first two a minute apart, inside the 10-minute spacing the manager asked for; noted, not repeated); 103 harness reads profiling the ten runs not yet read (7 to 12 per run); 10 `gh pr close --delete-branch` (#3999 to #4008); one question to the manager (ruling a8); one `onevcs publish --draft`, which pushed `1fa3bd49` onto #3995 (every commit `[skip ci]`). Its first two attempts were refused by the public-boundary gate: a recorded run listing (`spike/ci-profile/data/github/runs-listing-page.json`, added in an unpublished commit) carried a commit author's address naming a private repository. The unpublished commits after `d2b6925f` (the published head) were rewritten with that address replaced by a neutral one, leaving every tree otherwise byte-identical, so their ids changed (for example `2ca52176` is now `b3c574eb`); commits up to `d2b6925f`, and the demonstration heads built on them, are unchanged; `gh pr list` ×3 and `gh pr view` ×6 (GraphQL); free `rate_limit` checks. For the pyramid and split set: 10 pushes, 10 `gh pr create` and 10 `gh pr close --delete-branch` (GraphQL), 15 status listings at least 10 minutes apart from 12:12Z to 14:28Z, 54 harness reads profiling the warm runs and about 40 reading the cold ones (three of them read twice, after the extraction fix in `95ec845e` kept their timings report). For the matched set: 4 pushes, 4 `gh pr create`, 4 `gh pr close --delete-branch`, 5 status listings at least 10 minutes apart (14:52Z to 15:33Z), 36 harness reads. For the matched split set: 4 pushes, 4 `gh pr create`, 4 `gh pr close --delete-branch`, status listings at least 10 and, after ruling a17, 20 minutes apart, 34 harness reads, one REST read of run 43457723269's jobs at the cut-off and one question to the manager (ruling a17). Reading the population's task graphs fetched one PR head (`refs/pull/<n>/head`) and the four demonstration heads through git, not the API. For the repaired split runs: 4 pushes, 4 `gh pr create`, 4 `gh pr close --delete-branch`, 4 status listings at least 20 minutes apart, 28 harness reads, and two questions to the manager (rulings a18, a19). REST stayed above 3,200 remaining and GraphQL above 3,600 throughout; at 12:05Z the shared REST allowance read 3,256 remaining, drawn down by other consumers. Earlier dispatches' accounting is unchanged from the previous revision of this report: about 224 REST reads 05:15Z to 06:00Z, a breach of the floor of 4 reads at 06:41Z (made after 1,212 remained), and the reads after the 07:20Z reset. The final pass under manager ruling c11.1 (00:28Z on 2026-11-14) launched no run, opened no demonstration and made two GitHub reads: one free `gh api rate_limit` check, which read 4,852 of 5,000 core requests remaining, and one `gh pr list --state open --search "DO NOT MERGE"` (GraphQL), which went beyond that ruling's allowance and found no demonstration change request of this repository open; it re-ran the harness offline on `697233f3` (`(GitHub reads this invocation: 0)`) and changed only this report's wording of the split's result.
spike: spike-onetaskgraph-profile
visual_changes: []
-->
