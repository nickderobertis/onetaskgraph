# Main journey accounting golden

`session-cost.txt` records the loopback journey's outgoing requests and worst-case node
counts. It is not a measured real-GitHub primary bill.

The initial-board lookup now selects item and field totals from two connections bounded
at one. The observed loopback total changed from 272,519 to 272,521 nodes, with the same
163 requests. Selecting actual prices and the reconciliation alias adds no dispatches.

The main token leg reads every token through the plugin's own read by id — a source built
afresh, sending `ISSUE` — since the token became `gh3`, whose facets (the item's kind, its
board field values and its labels) a separate token-spelling query could only spell by
restating the plugin's rules. Those 15 reads replace the 15 `LiveChangeToken` queries that
had replaced them before, so the ledger is again 163 requests, now with 272,521 worst-case
nodes. No point moves: `ISSUE` costs one point, as `LiveChangeToken` did. The lane still
reads a fresh settled token after every mutation.

The six `gh3` legs that follow the task-token leg — the run's label renamed and recoloured on a
project and a document, a stamped scratch single-select field's option renamed, the run's first
project deleted under its dependent project and its document, and the orphan task given a
sub-issue — add 27 requests, to 190, and 278,469 worst-case nodes: 13 more reads by id (one
point each) for the tokens they hold, 12 modelled one-point mutations (the scratch field's
creation, value, rename and removal, two label attachments, the deletion, the sub-issue's
creation, filing and board item, and one board item lookup's write path) beside one more board
item lookup, and two REST label changes, which spend no GraphQL point. Against GitHub, each
token read waits out the settling margin and may be asked again, so the session pays somewhat
more there than on the loopback; the credentialed lane's own session-points accounting is what
measures it.

## Whole credentialed lane

The budget is `github-live-session-points`, max 299 in the GitHub e2e budgets file.
One selected credentialed Linux job runs the main and asset entry points sequentially,
under one fresh session identity. Other platform and coverage runs remain credentialless.
The reporter reads both completed records and hands the attributed sum to onebudgetspec;
it makes no API request and applies no threshold. Admission and rechecks retain 20% of
the whole allowance, retire completed demand and reserve owned cleanup.

The spike's `spike-quota-report` and retained harness on
`nick/otg-live-github-under-300-plan/spike-quota-2` are partial evidence: 115 and 119
reported non-hypothetical points, each with 44 unpriced GraphQL calls and 18 hypothetical
probes. Neither reached assets. Its 24/32 observation calls are removed by recording
existing response facts. No candidate saving was measured; search page size 20 was
already in force, and ISSUE already costs one point. Fresh token queries reduce
hydration payload, not that call's point price. The main fixture, source and board
metadata are reused only where freshness does not matter. There is no board/schema cache.

Design allocations are admission 3, schema 10, reconciliation 20, setup 14,
capabilities 42, comments 12, remaining token cases 60, the six `gh3` facet legs 46, cleanup
15, orphan recovery 2, assets 85, observations/retries 18: 327 total. The facet legs measure 27
requests and about 26 points on the loopback; their 46 is the higher figure because, against
GitHub, every token read waits out the settling margin and may be asked again. So the admission
estimate sits above the 299 budget by design: it is a reservation against the shared allowance,
and the 299 is the measured ceiling `github-live-session-points` holds a session's actual spend
to, unchanged. Before those legs the total was 281.
These are admission allocations, not measured savings or phase gates. The spike's
schema 10 and setup 14 are reported figures; capability 42/39 and comments 12 exclude
unknown prices. Assets 85 is an unvalidated reachable-document envelope; token 60 and
cleanup 15 are estimates.

Each executed query selects actual `rateLimit.cost`. The 18 dryRun probes preserve
hypothetical node-count/pricing evidence separately from the `actualCost` alias; the
executed read-only fallback preserves the same reconciliation when aliases are refused.
Mutations use the explicitly Modelled one-point primary price, with count and
[GitHub pricing documentation](https://docs.github.com/en/graphql/overview/rate-limits-and-query-limits-for-the-graphql-api).
That page documents a minimum call price, not an exact primary mutation tariff; its
secondary mutation price belongs to a different allowance. Missing query prices refuse a bill, except for the single deliberately refused
page-size-101 probe under manager rulings `c-7408370fba3af9d6ed0546b26f210c7b` and
`c-f3f629ad2efff2cc3377ad030549f02d`: if that GraphQL refusal reports no price,
the telemetry labels its one-point minimum as Modelled and reports its count separately.
The reader pins the operation hash, main capability phase, GraphQL-error outcome,
documentation URL, one-point price and single occurrence. A reported price wins. Headers only corroborate it: any internal reset switch invalidates the movement,
and unexplained movement is flagged without subtracting presumed other-host use.

Schema 3 records actual uploaded PNG sizes (74,666 and 479,959 bytes for each item kind)
and the generator targets (75,000 and 480,000) separately. This follows manager ruling
`c-99c85599e4fc9fdb1f91beaf5d1c3bd9`; the actual entry point checks the generator's
output against the shared workload and its documented tolerance. The target list is
optional and omitted when empty; the golden and both writers/reader move together.
Initial orphan population stays unknown; a recovery-time zero does not measure it.

## Retained real-board coverage matrix

This maps every assertion group in the spike's existing live coverage inventory to
its retained entry point. It describes the assertions, not a claim that the spike
completed them. Main helpers below are in `tests/journey/mod.rs`; assets are in
`onetaskgraph-github-projects-e2e/tests/live.rs`.

| Spike assertion group | Retained real-board journey |
| --- | --- |
| Admission and stamps | `enter`, `run`, Session admission, stamp checks and first-response allowance recheck |
| Ten schema requests | `verify_contract_schema`: DraftIssue membership arguments/return, 16 mutation contracts, 35 type inventories and two enum inventories |
| Eighteen pricing contracts | `reconcile_node_counts_and_point_costs`: every pinned document, hypothetical nodeCount/cost and separate executed price |
| Health, source setup, cursor walks and by-id equality | `settled_fixture` and `run`: complete capability declaration, exhaustive project/task walks, normalized categories, no duplicates, existing by-id equality, absent project, unique labels and origin/status fields |
| Two parents, one orphan, one separate document | `drive_every_declared_capability`: two projects, three tasks and one document; all six written and later removed |
| Title/body/either search and negative predicates | `drive_every_declared_capability`: own titles, body marker positive, title marker negative and either-field positive |
| Parent/orphan selection | `drive_every_declared_capability`: each parent's distinct task and explicit no-parent task |
| Labels | `drive_every_declared_capability`: positive/negative selection, own label listing and terminating paging |
| Todo/done and terminal issue reasons | `drive_every_declared_capability` and `terminal_status_parts`: normalized states; Done/COMPLETED and Cancelled/NOT_PLANNED |
| Dependencies | `drive_every_declared_capability`: task/project edges in both directions with item kinds |
| Paging and ceiling | `drive_every_declared_capability`: limit-one equals whole-page order, caller clamp, GitHub accepts 100 and refuses 101 |
| Document separation | `drive_every_declared_capability`: id/title/content/project/location round trip; absent from task/project lists |
| Typed metadata | `drive_every_declared_capability`: nested JSON type preservation and title/status read-back |
| Comment activity | `an_edited_comment_moves_its_issue_and_is_selected_since`: earlier threshold negative, updatedAt advances, later selection exactly the two owned tasks |
| Token comment add/edit/delete | `every_change_kind_moves_a_tasks_token`: fresh settled token after each; GraphQL errors are failures even at HTTP 200; deletion response facts logged before any body update |
| Eleven other unconditional token transitions | `every_change_kind_moves_a_tasks_token`: body, title, label, metadata, board origin field, dependency add/remove/replacement (unchanged edge count), parent removal/addition and final status |
| Native priority token | Same helper, conditional on native priority; unsupported priority does not claim this proof |
| Pre-existing capability gaps | Explicit priority/metadata/origin query-predicate positive/negative assertions were absent before this change and remain unproved |
| Task and document assets | `disposable_task_and_document_asset_copies`: Engine copies, four real upload/verification pairs, persisted asset records, markdown alt-text and attachment URL rewrites |
| Authenticated rendering and recovery | Same entry point: four authenticated reads; bounded 5xx retry, immediate non-5xx failure, per-attempt telemetry |
| Unchanged copies | Same entry point: end-command boundary then two re-copies, neither uploads again |
| Owned cleanup and orphan recovery | Main `remove_live_state`/`sweep_orphans` and asset owning-plugin deletions: stamp safety, issue/item/label cleanup, failure reporting |

`tests/live_pair.rs` in the GitHub e2e crate drives the two compiled live entry points
against one stateful GitHub-only double and then the actual reporter executable. It
checks the complete pair, all upload bytes and authenticated reads, preserves the 90
initial items, and proves partial cleanup cannot emit numeric success. The accounting,
reconciliation, allowance, reporter and rendering boundary journeys cover refused/missing
prices, unsupported aliases, retries, resets, shared movement, stale/mismatched/duplicate
records and the onebudgetspec threshold on either side. The real settling margin remains
60 seconds plus two; only the loopback server's already-settled Date avoids waiting.

Attachment HTTP calls remain in the ledger and session record. Under manager ruling
`c-525806e5edcaf01944951bfb35a7b430`, headerless uploads, GitHub attachment reads and
CDN rendering do not contradict the api.github.com Core REST allowance. Rechecks use
the latest response belonging to that budget; a declared attachment rate resource is
honoured, and an unread actual Core response still declines. The loopback pair returns
no invented Core headers on attachments and also proves a declared-but-unread Core
response stops completion after owned cleanup.

## Completed implementation measurement, 2026-10-13

The real main and asset entry points both exited 0, then the report-only command handed
**212 attributed points** to onebudgetspec. Its configured max 299 returned `within`,
with **87 points headroom**. Retained machine-produced records are
[`live-session-20261013/records/main.json`](live-session-20261013/records/main.json)
and [`assets.json`](live-session-20261013/records/assets.json); the exact checker result is
[`onebudgetspec-result.json`](live-session-20261013/onebudgetspec-result.json).
These records are historical implementation evidence, never inputs substituting for
current-session CI telemetry.

- Measured source revision: `ca86536a98dfe5a412598e3b4adac1749da228af`.
- Workspace source version: **0.6.0**. Both entry points use the Rust plugin/Engine APIs;
  no workspace CLI was spawned for this measurement.
- Authoring/plan-store CLI, separately captured on PATH: **onetaskgraph 0.4.0**.
- Session id: `implementation-s-b0f666475f66-1791893895`; internal record schema **3**.
- Both records complete: 182 GraphQL requests (116 queries, 66 mutations), plus 19 REST
  requests retained as detail and excluded from GraphQL points.
- Pricing: **146 response-reported query points + 66 Modelled mutation points** at one
  point/request; zero documented mutation points and zero modelled ceiling-refusal points.
  GitHub reported the deliberately refused page-size probe's actual price.
- Four additional fresh-token reads were recorded as retry attempts; no query price was
  unavailable. All 10 schema requests and 18 pricing contracts completed; both pricing
  aliases were supported by real GitHub.
- Corroborating account movement is unavailable because the observed reset windows switch
  internally. No presumed other-host use was subtracted.

| Phase | Combined attributed points |
| --- | ---: |
| Admission | 0 (REST detail) |
| Schema | 10 |
| Reconciliation | 30 |
| Setup | 41 |
| Capabilities | 39 |
| Comment activity | 15 |
| Change token | 44 |
| Cleanup | 20 (main 14, assets 6) |
| Orphan recovery | 1 |
| Asset setup | 1 |
| Asset task | 6 |
| Asset document | 5 |
| **Total** | **212** |

The phase labels follow the current journey's boundaries: Setup includes source
initialization and the first five fixture writes; the document write belongs to
Capabilities. The design allocations above are not phase gates or a measured baseline
for subtraction. No before/after point saving is claimed.

The measured board began with 90 items and 14 fields. Main wrote its two projects, three
tasks (one orphan) and one document; all six owned items/issues and the owned label were
removed. Recovery found zero eligible artifacts and labels; initial orphan population
and board peak remain unmeasured. Assets wrote two issues and four PNGs, verified each
upload, read authenticated renderings, checked markdown/record rewriting, re-copied both
items without another upload, and removed both disposable items/issues. Both entry
points completed their owned cleanup before setting their completion flags.

All fourteen unconditional token changes passed with the real settling margin. Around
deleting the comment, the fresh responses showed count **2 → 1**, issue updatedAt
**12:21:59 → 12:23:07 UTC**, and item updatedAt **12:22:01 → 12:23:10 UTC**; two reads
still withheld the token before a later settled token was observed. Only afterward did
the body update run. This establishes the deletion/read-after-write behavior on this
source revision. The earlier spike's discarded inner error remains unknown; HTTP 200
from that earlier attempt is not a diagnosis of UPDATE_ISSUE or a complete bill.

This is one authorized implementation measurement, not a CI verdict or merge-path check.
The later evidence commit retains these records and prose. A subsequent refactor shares
`MODELLED_MUTATION_POINTS = 1` between writer, admission, billing and reconciliation,
and rejects an unapproved model in the reader. It changes no requests or emitted prices;
before/after actual entry-point loopback pairs produced identical request sequences
(operation hashes, phases, attempts, kinds, outcomes), prices and attributed bills.
The final entry-point test also reconciles every emitted model with that constant. The measurement above remains explicitly about `ca86536a`; no live measurement
of those later edits is claimed.

## A create safe to retry, 2026-10-30

`ISSUES_BY_CREATION` joined the source's documents: a repository's issues newest first,
which a retried copy lists to find the issue its earlier attempt created by its create
key. The session's reconciliation of every document's node count and point cost now
covers it too, so the record moved by that one reconciliation call — 217 requests and
100 more nodes — and by nothing a credentialed session sends.

## Verification and scoped suppressions

Focused checks passed on the measured code: all nine actual-entry-point loopback
journeys; the real policy/Nx/bun/onebudgetspec wiring journeys; shared-schema/accounting
and admission tests; source execution-price/error/retry journeys; writer/reader bridge;
comment refusal/deletion and fresh-token regressions; reconciliation and main request
count golden; rendering recovery/failure and raw asset query journeys; reporter integer
fixtures on both sides of the configured boundary; live-lane shape/selection scripts;
changed-crate formatting and clippy. Judged diff lint run `20261013T121400Z-9c7d9` passed
35 rules with zero failures (12 skipped, 9 not relevant). Publication checks were not run.

Negative evidence is labelled as such: disabling the budget adapter's live exclusion
made its boundary assertion fail; bypassing the remaining-demand refusal made the
actual entry-point refusal journey fail; a scalar response with Date reproduced the
old parser panic before the shared envelope validation. The restored implementations
passed afterward. The two-process loopback pair also refused missing actual prices,
partial cleanup, unsupported/ignored aliases and malformed errors rather than inventing
a numeric success.

The diff's scoped suppressions, including standing scopes over changed lines, are:

| Site | Rule and reason |
| --- | --- |
| `github-live/src/quota.rs`: session id, source revision, workspace version, completion flag and operation hash | `invalid_states_unrepresentable`: the agreed wire requires String/bool; the reader validates identity, provenance, completion and hash |
| Same file: separate kind/outcome/charge fields | Same rule: the frozen shared wire names separate fields; admission and billing validate combinations rather than change that agreement |
| Same file: documented/modelled mutation URLs and modelled page-refusal URL | Same rule: agreed String fields; the reader pins documentation and each permitted pricing basis, including the exact refusal identity |
| `github-projects-e2e/budgets.yaml`: live command | `budget_commands_measure_directly`: the task fixes a named report-only command using saved telemetry and the existing SDK; only onebudgetspec judges 299 |
| Same crate `tests/e2e/budget_runner.rs`: `report_live_session` and its selector check | `tests_assert_real_behavior` / `budget_commands_measure_directly`: a command entry, not a standalone journey; executable reporter tests assert its result/refusal and the selector prevents a foreign budget receiving the figure |
| Same crate `project.json`: test's main dependency and budget's main dependency | `external_service_suite_stays_out_of_the_affected_tier`: this task requires the ordered main/assets pair; the repository requires live tests under their own affected domain edge |
| Same file: coverage target | `expensive_tests_stay_behind_their_own_edge`: AGENTS.md requires aggregate instrumented coverage; credentials are cleared and the pair uses GitHub loopback only |
| `github-projects/tests/plugin.rs`: fresh-token child and pair fixture child | `tests_assert_real_behavior`: subprocess helpers whose parents assert fresh reads, real entry points, prices, refusal, uploads and cleanup; ordinary runs omit the child selector |
| Same crate `tests/quota_writer.rs`: bridge child | Same rule: its parent drives the source over HTTP and asserts persisted prices, failure cleanup and refusal of an inappropriate pricing exception |
| `justfile`: standing affected-test block | `external_service_suite_stays_out_of_the_affected_tier`: the existing plugin edge and version-only selection policy remain; the adapter propagates that decision and fresh session identity |

No enforcement threshold, shared lint rule, public plugin/Engine interface, hook,
bootstrap file, branch protection, Linear implementation or orchestration code changed.
