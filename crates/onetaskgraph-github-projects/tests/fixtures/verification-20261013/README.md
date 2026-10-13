# Offline verification of the completed GitHub lane

Verified on 2026-10-13: baseline `ca86536a98dfe5a412598e3b4adac1749da228af`
and finished executable tree `661d762d`. This evidence-only commit changes no executable
code. No real GitHub session was opened; all requests used local listeners and the
literal fixture credential `test-token`.

The actual main entry point `real_projects_v2_contract_writes_and_leaves_no_residue`
and asset entry point `disposable_task_and_document_asset_copies` were compiled with
`cargo test --locked --all-features --no-run` for both revisions. The baseline used a
separate detached scratch worktree. Each pair ran sequentially against its revision's
`live_pair_fixture_child`, with 90 initial items and 14 fields, then ran the actual
`budget_runner::report_live_session` executable. Both completed owned cleanup; the
fixture asserted that all 90 original items survived and no owned labels remained.

A scratch HTTP proxy captured requests at the boundary and forwarded them to the real
stateful loopback board. It rewrote only the fixture host in returned upload URLs to
keep authenticated attachment reads on the same local boundary. GraphQL documents
were hashed without modification. Variable shapes preserve every object key, list
position and scalar type; page-size values (`first`, `*First`, `last`, `limit`,
`pageSize`) are retained. Dynamic scalar values such as stamps and timestamps are
represented by their types. REST requests preserve method, path and body length; only
the label's generated timestamp suffix is replaced by `<stamp>`.

Both runs produced byte-identical normalized evidence:

- `sequence.json`: all 193 HTTP operations in order, including 174 GraphQL operations
  with document SHA-256, variable shape and page sizes, and 19 REST operations.
- `prices.json`: all 193 recorded calls in order, including phase, operation SHA-256,
  request kind, attempt, outcome and complete pricing basis.
- `loopback-report.json`: identical complete reporter output, **174 points = 108
  response-reported query points + 66 Modelled mutation points**.

These are offline comparison figures, not a new real-GitHub measurement. The fixture
reports one point per query, whereas the real server's costs and retry timing differ.
The finished reporter also read the retained completed real-session records directly,
using their historical identity: `historical-report.json` reports **212 points = 146
response-reported query points + 66 Modelled mutation points**, 87 below max 299.
The source measurement remains ca86536a, not the verification revision.

## Changed files since the measured revision

Paths below are relative to `crates/` unless explicitly described otherwise.

| File | Why it cannot move live requests |
| --- | --- |
| `onetaskgraph-github-live/src/quota.rs` | Defines `MODELLED_MUTATION_POINTS = 1`; validates that saved modelled prices use precisely that existing price. No transport or query change. |
| `onetaskgraph-github-projects/tests/journey/mod.rs` | Reconciliation compares against that constant instead of literal `1`; equal value, same branch and documents. |
| `onetaskgraph-github-projects/tests/journey/quota.rs` | Recording reads that constant instead of two literal `1` values; same recorded mutation prices. |
| `onetaskgraph-github-live/tests/fixtures/session-v3.json` | Accounting test golden formatting only; not read by live entry points. |
| `onetaskgraph-github-live/tests/quota.rs` | Adds rejection coverage for unapproved mutation prices; separate test executable. |
| `onetaskgraph-github-projects-e2e/tests/live_pair.rs` | Adds assertions on recorded mutation prices; separate offline test executable. |
| `onetaskgraph-github-projects-e2e/tests/live.rs` | This dispatch fixes only the separate admission test's expected error for `{}`; neither actual live entry point executes that assertion. |
| `onetaskgraph-github-projects/project.json` | Whitespace-only JSON formatting; parsed object equals baseline. |
| `onetaskgraph-github-projects-e2e/project.json` | Whitespace-only JSON formatting; parsed object equals baseline. |
| `onetaskgraph-github-projects/tests/fixtures/live-session-20261013/onebudgetspec-result.json` | Retained historical checker output; no live entry point reads it. |
| `onetaskgraph-github-projects/tests/fixtures/live-session-20261013/records/main.json` | Retained historical main record; no live entry point reads this fixture. |
| `onetaskgraph-github-projects/tests/fixtures/live-session-20261013/records/assets.json` | Retained historical asset record; no live entry point reads this fixture. |
| `onetaskgraph-github-projects/tests/fixtures/session-cost.md` | Measurement documentation only. |
| `onetaskgraph-github-projects/tests/fixtures/verification-20261013/README.md` | This report only. |
| `onetaskgraph-github-projects/tests/fixtures/verification-20261013/sequence.json` | Captured comparison evidence only. |
| `onetaskgraph-github-projects/tests/fixtures/verification-20261013/prices.json` | Captured comparison evidence only. |
| `onetaskgraph-github-projects/tests/fixtures/verification-20261013/loopback-report.json` | Offline reporter output only. |
| `onetaskgraph-github-projects/tests/fixtures/verification-20261013/historical-report.json` | Report-only reading of retained historical evidence. |

The only live-executed code changes are the shared one-point constant and its readers.
No serialized session contract changed in this dispatch; it remains schema 3.

## Focused checks

All commands removed `GH_PROJECTS_TOKEN`, `GH_TOKEN` and `GITHUB_TOKEN` from their
inherited environments. The loopback harness sets only its literal fixture credential.

Passed on the finished tree:

- `cargo test --locked -p onetaskgraph-github-live --test quota`: 14 tests.
- `cargo test --locked --all-features -p onetaskgraph-github-projects-e2e --test live_pair`:
  9 tests, including partial cleanup, malformed envelopes, missing board dimensions,
  unsupported/ignored pricing aliases and unread attachment allowances.
- Same package, `--test e2e live_session_report_boundary`: 1 reporter boundary test.
- Plugin package, `--test plugin`, each selected separately:
  `opted_in_actual_query_prices_reach_the_ledger_through_the_real_source`,
  `actual_execution_facts_distinguish_refusals_retries_and_unreadable_responses`,
  `fresh_token_polling_reads_again_after_a_server_date_inside_the_margin`: 3 tests.
- E2e package, `--test live`, selectors
  `raw_asset_queries_retain_actual_prices_and_failure_outcomes`, `a_rendering_`,
  `board_admission_rejects_non_success_even_with_valid_board_json`: 6 tests.
- `cargo fmt -p onetaskgraph-github-live -p onetaskgraph-github-projects
  -p onetaskgraph-github-projects-e2e -- --check`.
- `cargo clippy --quiet --locked --all-targets --all-features` over the same three
  packages, with `-- -D warnings`.

The admission assertion failed before this dispatch's fix at both HEAD and ca86536a,
but passed at merge-base `14ee32f799a31e6150154ea461487099e2442f41`.
Envelope validation introduced on this branch correctly refuses `{}` before board-id
validation. Commit `661d762d` changes the expected error to that envelope refusal;
the same real HTTP boundary test then passes. No production behavior was weakened.
The entry-point comparison was repeated after the fix and still matched.

Publication belongs to the dispatch harness; this dispatch did not push or open a PR.

SHA-256 of the byte-identical normalized captures:

- `sequence.json`: `195359274c46fab7a6e82451823094093041622692ba6a1bb69d9078e42049b3`.
- `prices.json`: `f13572f3c753c6377e9e4f658e1aded0e1f3285d7c6b77b2f9c01da26811adb3`.

## Merge-base source provenance recheck

An initial invocation immediately after creating the merge-base worktree accidentally
retained the ca86536a worktree as its working directory. Its failure at line 572 was
baseline evidence only; it was not used to classify the merge-base. The subsequent
correctly located merge-base invocation passed. This was repeated from a newly created
merge-base worktree after the provenance concern was raised, and passed again:

```text
working directory: $ONEPIPELINE_NODE_SCRATCH_DIR/merge-base
HEAD: 14ee32f799a31e6150154ea461487099e2442f41
source: crates/onetaskgraph-github-projects-e2e/tests/live.rs
source SHA-256: 5967dafde5b55e4367755eff26891164f3ee2627a80bc5563ee388671ec9e787
test definition line: 371
assertion line: 437
Cargo metadata live src_path: <working directory>/crates/onetaskgraph-github-projects-e2e/tests/live.rs
Cargo metadata target_directory: <working directory>/target
compiled package: onetaskgraph-github-projects-e2e (<working directory>/crates/onetaskgraph-github-projects-e2e)
executed test binary: target/debug/deps/live-3d8c5a197ad42c34
result: 1 passed; 0 failed; 5 filtered out
```

The command, run inside that directory, was:

```sh
env -u GH_PROJECTS_TOKEN -u GH_TOKEN -u GITHUB_TOKEN cargo test --locked --all-features \
  -p onetaskgraph-github-projects-e2e --test live \
  board_admission_rejects_non_success_even_with_valid_board_json
```

The same command was also repeated on the finished tree and passed (10 filtered out).
Thus this branch introduced the obsolete expectation; commit `661d762d` fixes it.
No pre-existing admission failure remains to draft as a follow-up.

## Merge-path repair verification

Commit `9c3e0895` formats `sequence.json` with the repository's Biome configuration
and inventories `LIVE_CHANGE_TOKEN`, including its one-point offline price pin.
The manager approved excluding this live-only document from reconciliation: its actual
response cost is already billed on every live-lane read. This preserves the measured
request workload while the schema, node-count and price inventories cover the document.
The loopback pair now checks each priced fresh-token document's recorded charge and
checks the complete bill's query subtotal. Injecting zero-point fresh-token charges
made the actual entry-point pair fail at that charge assertion; restoring the recorder
made all nine pair tests pass. No serialized contract changed.

Verification used a newly created detached worktree at `ca86536a` under the dispatch's
scratch directory, with Cargo run inside that worktree. The same capture harness
described above drove both actual entry points on the baseline and the repaired tree.
The final `sequence.json`, `prices.json` and reporter output were byte-identical across
both runs: 193 HTTP operations, 174 GraphQL requests, 19 REST requests, and 174 loopback
points. The sequence and price captures also match the retained evidence above.
The current reporter read the retained historical records and again reported 212 points,
87 below 299. No real GitHub session ran. The detached worktree was removed afterwards.

Passed on the repaired executable tree:

- `workspace:format-check`, invoked through Nx with its cache skipped.
- Plugin `schema` (10), `node_count` (4), `point_cost` (6), `budget_gate` (13),
  `lane_entry` (1), and `reconciliation_gate` (3) tests.
- Session accounting `quota` (14), the complete loopback pair (9), and the reporter
  boundary (1).
- The three plugin accounting/fresh-token selectors and six asset accounting,
  rendering and board-admission checks listed above.
- Rust formatting and clippy with warnings denied over `onetaskgraph-github-live`,
  `onetaskgraph-github-projects` and `onetaskgraph-github-projects-e2e`.

## Cleanup and Windows bridge repair

Executable repair `192d1a57` changes two offline test files only:

- `sweep_gate.rs` now expects the specific GraphQL envelope refusal for invalid JSON
  and the specific invalid-errors-envelope refusal for malformed errors. The remaining
  presence faults still assert their particular cleanup failures. The targeted test
  reproduced the invalid-JSON expectation failure before the edit; all 14 sweep tests
  pass afterwards.
- `quota_writer.rs` explicitly calls `set_nonblocking(false)` on each accepted stream
  before reading. The listener remains nonblocking so it can stop when the child exits.
  Winsock inherits the listener's socket properties on accept, unlike the blocking
  accepted sockets observed on Linux: the inherited nonblocking stream could return
  `WouldBlock` immediately at the request parser's read. A read timeout does not turn
  a nonblocking socket into a blocking one. This fixes the mode itself without adding
  retries or extending a wait. Both bridge tests pass on Linux. Windows execution was
  unavailable in this dispatch; the inheritance is documented by
  [Microsoft's accept contract](https://learn.microsoft.com/en-us/windows/win32/api/winsock2/nf-winsock2-accept).

The actual live main and asset entry points were rebuilt and driven through the capture
proxy and loopback board at both `192d1a57` and a newly created detached worktree at
`ca86536a98dfe5a412598e3b4adac1749da228af`. Cargo ran inside the detached worktree, under
`$ONEPIPELINE_NODE_SCRATCH_DIR/measured`, which was removed after comparison. Sequence,
prices and complete reporter output were byte-identical: 193 HTTP operations, 174
GraphQL requests, 19 REST requests, and 174 fixture points. The sequence and prices
SHA-256 values match those listed above. The repaired reporter also read the retained
historical records and reported 212 points (146 reported queries plus 66 modelled
mutations), 87 below 299. No real GitHub session ran; real credentials were removed
from check environments, and entry-point comparisons used only `test-token` locally.

Offline verification on the repaired tree used:

```sh
cargo test --locked --all-features -p onetaskgraph-github-live \
  -p onetaskgraph-github-projects -p onetaskgraph-github-projects-e2e -- \
  --skip real_projects_v2_contract_writes_and_leaves_no_residue \
  --skip disposable_task_and_document_asset_copies
```

That aggregate invocation passed 595 tests but failed two SDK e2e tests during
concurrent editable Python CLI builds. Both failures report maturin's rename of
`target/release/onetaskgraph` into `target/maturin/onetaskgraph` failing with ENOENT,
after successful release compilation. The manager authorized running those two tests
serially, with no code or configuration change (reply correlation
`c-60b5719f60d373c61f894ea56fd4d0a2`). Both passed individually with `--exact
--test-threads=1`:

- `cache::one_sources_cache_is_shared_by_the_command_line_both_sdks_and_a_rust_caller`
- `copy_cost::detail_and_record_only_reads_reuse_one_issue_resolution_through_cli_and_sdks`

The remaining e2e package `live` executable passed all 10 offline tests, and
`live_pair` passed all nine tests. Both library doctest suites passed (zero examples).
Thus every one of the 616 offline tests passed on this tree across these invocations;
the aggregate parallel invocation itself did not pass. Session accounting, budget
reporter, admission checks and loopback recovery cases are included. The independent
parallel build race was left unchanged and drafted as
`drafts:otg-live-github-under-300-plan/drafts/20261013T151201Z-concurrent-python-sdk-e2e-builds-race-staging-th`.
Logs are retained under this dispatch's scratch directory as `offline-tests.log`,
`sdk-cache-recheck.log`, `sdk-copy-recheck.log`, `remaining-tests.log` and `doc-tests.log`.

Also passed: `cargo fmt --all -- --check`; `workspace:format-check` with Nx cache
skipped; and clippy with all targets/features and `-D warnings` over the three changed
crates. This documentation changes no serialized contract or live request.


## Windows collection launch repair

Executable repair `6f219e54` changes the `test` recipe's launch from
`lane_runner.py select scripts/live-lane-selection.sh ./scripts/nx.sh …` to
`lane_runner.py select scripts/live-lane-selection.sh bash ./scripts/nx.sh …`.
The file Python's `asyncio.create_subprocess_exec` previously launched was
`./scripts/nx.sh`, a Bash script rather than a Windows executable. Windows process
creation does not interpret its shebang, so it refused that file with WinError 193.
The adapter now launches Bash explicitly, which reads the script. No Windows skip,
retry, live request, pricing rule or serialized contract changed.

The portable `test_recipe_launches_the_real_nx_script_through_bash` regression
reads `just --dry-run test` and drives its actual Python adapter and Nx wrapper,
substituting a graph query for target execution. It failed before the recipe fix
at the missing-interpreter assertion, then passed on Linux. It also checks that
Bash's missing-script failure propagates and produces no graph. It is an ordinary
unconditional test in the merge path; its Windows execution belongs to that path
and was not run in this Linux dispatch.

The actual main and asset entry points ran against the loopback board at
`757195dc` in a separate detached worktree under the dispatch scratch directory,
and on the repaired tree. Cargo ran inside the detached worktree. Sequence,
prices and the complete reporter output were byte-identical: 193 HTTP operations,
174 GraphQL requests, 19 REST requests, and 174 fixture points (108 reported query
points plus 66 modelled mutation points). These are offline figures; no real
GitHub session ran. The detached worktree was removed after comparison.

Passed on the repaired tree, with real GitHub credentials removed:

- All 620 offline tests across `onetaskgraph-github-live`,
  `onetaskgraph-github-projects` and `onetaskgraph-github-projects-e2e`, plus
  doctests, using the aggregate command above with `--test-threads=1` and only
  the two credentialed entry points excluded. Serial execution avoids the
  previously recorded independent concurrent SDK packaging race.
- The four `lane_wiring` tests after the final regression-test edit.
- `cargo fmt --all -- --check` and `workspace:format-check` with Nx cache skipped.
- Clippy over the same three crates with all targets/features and `-D warnings`.
- `bash scripts/check-live-lane-selection.sh`.

Dispatch evidence is retained under `$ONEPIPELINE_NODE_SCRATCH_DIR` in
`offline-tests.log`, `lane-wiring.log`, `format.log`, `clippy-final.log`,
`selection.log`, `before-{sequence,prices,reported}.json` and `after/`.

## Windows executable resolution repair

Commit `20562737` resolves every collection subprocess executable to an absolute
path before calling Python's process API. On Windows, Bash candidates come from
PATH and the Git installation identified by `git.exe`; a candidate must have
Git Bash's accompanying `msys-2.0.dll` runtime, either beside it in `usr/bin` or
under the installation's `usr/bin` when launching `bin/bash.exe`. Thus Windows
never receives the bare name `bash` and cannot substitute `System32\bash.exe`.
The policy and Nx wrapper run through that selected Git Bash executable. Git
and Bun also launch by their resolved absolute paths. Linux and macOS use the
absolute paths resolved from PATH (Linux here: `/usr/bin/bash`, `/usr/bin/git`
and the installed Bun binary). There is no Windows skip or retry.

The first attempt launched `scripts/nx.sh` directly: Windows does not interpret
a Bash shebang and returned WinError 193. The next supplied bare `bash`:
Windows preferred its System32 WSL launcher, which had no installed distribution.
The current resolution selects Git Bash by its MSYS runtime instead.

The unconditional `lane_wiring` regression drives the dry-run test recipe's
actual adapter and Nx wrapper with a graph query, and exercises missing-script
failure propagation. Its outer shell uses the same resolver. It failed against
the old adapter because that resolver was absent, then all four wiring tests
passed on Linux. Windows execution belongs to the merge path; no Windows result
is claimed here.

A detached worktree at `0bc6a76c` under this dispatch's scratch directory was
built with Cargo run inside it. The actual main and asset entry points ran
against the loopback board there and on the repaired tree. Captured sequence,
prices and complete reporter output were byte-identical: 193 HTTP operations,
174 GraphQL requests, 19 REST requests, and 174 fixture points. Sequence and
price hashes match the earlier evidence above. The worktree was removed.
No live GitHub session ran; real credentials were removed from check
environments, and the comparison used only the local `test-token`.

Workspace format-check (Nx cache skipped), `cargo fmt --all -- --check`, clippy
with all targets/features and warnings denied over the three changed crates,
and `scripts/check-live-lane-selection.sh` passed. Logs and before/after captures
are retained under this dispatch's `$ONEPIPELINE_NODE_SCRATCH_DIR`.

All 620 offline tests across the three changed crates, plus doctests, passed
with `--test-threads=1` and only the two credentialed entry points excluded,
using the aggregate command recorded above. The finished resolver also passed
all four `lane_wiring` tests in a separate invocation.

## Continuation verification at bc53590a

The continued dispatch started with the executable repair already committed in
`20562737` and its report in `bc53590a`; no additional launcher change was needed.
The four unconditional `lane_wiring` tests passed again on Linux. Copying the
finished regression into the detached `0bc6a76c` worktree made its recipe journey
fail with `KeyError: 'executable'`, at the missing resolver. Windows remains the
merge path's result, not a result obtained on this Linux host.

Cargo built the detached `0bc6a76c` worktree inside the dispatch scratch directory.
A local recording HTTP proxy then drove both actual live entry points against
its loopback board and against the finished tree's board. Both runs issued the
same 193 operations in order: HTTP methods, paths (ownership nonce normalized),
and complete GraphQL operation documents. Their operation-sequence SHA-256 was
`ac01993f647815a08a7d1cf4f2c7d87815fd8bbbba3051f2937de4400406d8e2`.
The raw request payloads contain wall-clock ownership nonces and generated asset
contents and are not claimed byte-identical. The recorded phase, operation hash,
attempt, outcome and charge tuples were byte-identical, as was the complete
reporter JSON: 174 points, 108 reported query points plus 66 modelled mutation
points, 174 GraphQL requests and 19 REST requests. The detached worktree was
removed. Only the local `test-token` was used; no live GitHub session ran.

Clippy over all targets/features of the three changed crates with warnings denied,
`cargo fmt --all -- --check`, the uncached workspace format check and
`scripts/check-live-lane-selection.sh` passed again. Evidence for this continuation
is retained under `$ONEPIPELINE_NODE_SCRATCH_DIR` as `*-current.log`,
`regression-before.log`, `before-build.log`, `before-capture/` and `after-capture/`.

The aggregate offline command recorded above passed all 620 tests and both
library doctest suites again, with `--test-threads=1` and only the two credentialed
entry points excluded. Real GitHub credentials and the live-required flag were
removed from its environment. The final workspace format check passed after this
verification note was added.

## Windows code-page repair at 03b0d228

The merge path at `2f23f8f6` refused `scripts:test` on Windows: the "GitHub live
collection" step (`lane_runner.py select`, which the `test` recipe runs) decoded the
selection helper's stderr as UTF-8, while the helper's python, writing to a pipe on
Windows, used cp1252, where its em dash is byte 0x97. The helper
(`scripts/live-lane-selection.sh`) now reconfigures both its streams to UTF-8; the step
captures and strictly decodes text from exactly two producers — that helper's stdout and stderr as
UTF-8, re-emitting the stderr as UTF-8 bytes — and `git rev-parse HEAD`'s stdout as an
ASCII hexadecimal object name, leaving git's stderr uncaptured. Forcing
`PYTHONIOENCODING=cp1252` on Linux reproduced the CI error exactly; the recipe-launch
test in `tests/lane_wiring.rs` now forces it on every platform and asserts the em dash
arrives as UTF-8, and fails with either half of the repair reverted.

With the same recording proxy, both actual live entry points ran against the loopback
board at `2f23f8f6` (a detached scratch worktree, since removed) and on the finished
tree: identical 193-operation sequences, byte-identical `prices.json`, and a
byte-identical bill of 174 points (108 reported + 66 modelled). Only `test-token` was
used; no live GitHub session ran.

## Repeat collection, decline follow-through and Windows pair lookup, to e5e9532b

The merge path at `615d7019` refused two `check` jobs. On Linux the GitHub live
collection created `GITHUB_LIVE_SESSION_DIR` (CI keys it on
`<run id>-<run attempt>` under the main checkout's `target/debug/telemetry/github-live-session/`)
exclusively, so `scripts/check-live-lane-selection.sh`, driving `just check` and
`just gate` again in the same job, refused with `File exists` before reaching Nx; its
missing `scripts:distribution-check` and `scripts:distribution-test` followed from that
refusal. A repeat now takes the first unused `<dir>.<n>` beside it, created exclusively
(`1f06bedd`). With that directory pre-created holding a sentinel and CI's absolute
`GITHUB_LIVE_SESSION_DIR` set, the check passed, its three repeats recorded in `.2`,
`.3` and `.4`, and the sentinel was untouched.

The Windows `recheck_gate` panic was not platform-dependent: it is the deliberate
decline follow-through of `scripts/check-budget-decline.sh`, which still required the
recheck's old reading name and failed on every platform (`cee6d552`). At `449b2f3d`
the merge path's Windows `onetaskgraph-github-projects:test` passed; only
`live_pair` failed, all nine tests with `compile ... before this pair`, because rustc
writes dep-info paths with the host separator (`crates\...\tests\plugin.rs`) and the
lookup matched forward slashes alone. `e5e9532b` matches either separator, with a
test that fails on the Windows spelling against the old match, and publishes the
fixture's readiness file by rename.

With the same recording proxy, both actual live entry points ran against the loopback
board at `615d7019` (a detached scratch worktree, since removed) and at `e5e9532b`:
identical 193-operation sequences (174 GraphQL, 19 REST; methods, paths with the
ownership nonce normalized, and GraphQL documents), byte-identical `prices.json`, and a
byte-identical bill of 174 points (108 reported + 66 modelled). Only `test-token` was
used; no live GitHub session ran.

## The 84-item board, and verification of the reverted tree at 4839f622

The merge path at `0f70890a` (CI run 38465257225) passed `scripts:test` on Linux and
refused only `onetaskgraph-github-projects:test`: the credentialed main journey stopped
before setup with "initial board workload is 84 items and 14 fields; registered workload
requires 90 items and 14 fields". `031f7650` briefly accepted any board no larger than the
registered one; by the planner's ruling the measured workload stays exactly 90 items and
14 fields, so `4839f622` and `2f41f536` revert it and its note. The tree at `4839f622` is
byte-identical to `0f70890a`; nothing about workload acceptance or preparation changed.

What the CI and janitor logs show, without any read of the board itself:

- At `615d7019` (CI run 38449060700, finished about 19:25 UTC on 2026-10-13) the live main
  journey passed, which its exact workload check permits only on 90 items and 14 fields.
- The scheduled janitor run 38463250113 at 20:29 UTC reported "4 REST reads, 1 GraphQL
  queries, 13 content writes". Every other janitor run after the 12:18 measurement either
  reported 0 content writes or failed on GitHub's rate limit before writing.
- At 20:59 UTC the main journey observed 84 items and 14 fields.

**Inference, not item-level evidence.** No log names the items deleted or the run that
owned them, and the account's other credentialed work is not visible here, so these logs do
not show that the janitor was the only writer to the board. The aggregate figures are
consistent with the janitor deleting six board items, their six scratch issues and one
label: thirteen writes, the shape of one main journey's fixture (two projects, three tasks,
one document) left behind by one run. The janitor deletes CI-stamped residue only once it is
24 hours old, and the 19:25 run wrote nothing while the 20:29 run wrote 13, which places the
residue's creation on 2026-10-12 between about 19:25 and 20:30 UTC. One run in that window
had the board credential and stopped without reaching end-of-run cleanup: CI run
38297505307 on `nick/3581/otg-source-cache`, inside `nx affected -t test budgets` from
19:38 until it was cancelled at 19:54. Its log stops at the cancellation and shows no
journey output, so its ownership is inferred from timing alone. If the inference holds,
the 90 recorded by the 12:18 measurement included that residue. Restoring the board is a
decision outside this branch.

Focused verification on `4839f622`, with `GH_PROJECTS_TOKEN`, `GH_TOKEN`, `GITHUB_TOKEN`,
`LINEAR_API_KEY` and `ONETASKGRAPH_LIVE_REQUIRED` removed (no live GitHub session ran):

- `cargo test --locked --all-features -p onetaskgraph-github-live -p onetaskgraph-github-projects
  -p onetaskgraph-github-projects-e2e`: 623 passed, 0 failed; the credentialed journey
  printed its skip for the missing token.
- `recheck_gate`: 9 passed, including
  `a_journey_whose_first_call_carries_less_than_the_endpoint_claimed_does_not_run`.
- `scripts/check-relative-interpreter.sh`, which drives `scripts/check-live-lane-selection.sh`.
  Its `expect_unconditional_targets` requires the gate to run every target the justfile runs
  by name, including `scripts:distribution-check` and `scripts:distribution-test`.
- `scripts/check-live-lane-selection.sh`, `scripts/check-budget-decline.sh`,
  `scripts/check-store-fixtures.sh`, `scripts/check-selection-line-endings.sh` and
  `scripts/check-selection-code-page.sh`.
- `cargo fmt --all -- --check`, `workspace:format-check` with the Nx cache skipped, and
  clippy over all targets and features of the three changed crates with `-D warnings`.

With the same recording proxy, both actual live entry points ran against the loopback board
in a detached scratch worktree at `615d7019`, with Cargo run inside it (the worktree has
since been removed), and on `4839f622`. The 193 operations (174 GraphQL, 19 REST) matched in
order: methods, paths with the ownership nonce normalized, and GraphQL documents (sequence
SHA-256 `add53edb35a58fccebbdeb8a7e4ada82139508144ac9b03244e3e74d2640bb4a`).
`prices.json` (`278db40e…e1a3`) and the reporter output (`8aad47d1…0d2f`) were byte-identical.
Both bills were 174 points: 108 reported query points plus 66 modelled mutation points.
These are loopback figures, not a real-GitHub measurement.
