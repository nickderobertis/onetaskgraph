# Main journey accounting golden

`session-cost.txt` records the loopback journey's outgoing requests and worst-case node
counts. It is not a measured real-GitHub primary bill.

The initial-board lookup now selects item and field totals from two connections bounded
at one. The observed loopback total changed from 272,519 to 272,521 nodes, with the same
163 requests. Selecting actual prices and the reconciliation alias adds no dispatches.

Fresh token-bearing queries replace 15 full issue hydrations in the main token leg.
The observed loopback ledger remains 163 requests, with 267,181 worst-case nodes.
This payload change earns no point savings: the replaced ISSUE request already costs
one point. The lane still reads a fresh settled token after every mutation, and logs
comment count and both timestamps from those existing responses around deletion.

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
capabilities 42, comments 12, remaining token cases 60, cleanup 15, orphan recovery 2,
assets 85, observations/retries 18: 281 total, with 18 points to the 299 budget.
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
