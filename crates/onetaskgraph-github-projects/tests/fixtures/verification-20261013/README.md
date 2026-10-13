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
