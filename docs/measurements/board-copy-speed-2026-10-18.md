# Board copy speed: before and after, measured on GitHub

The timing spike `spike-copy-timing` (report
`board-copy-speed-2026-10-18-spike-copy-timing-report` in `authoring:board-copy-speed-2026-10-18`)
measured how long a copy of a 10-item plan onto a GitHub Projects board, and an approval of
its design document, took on onetaskgraph 0.7.1. This file sets those figures beside the same
harness's measurement on the releases ai-orchestrator adopted afterwards, and beside each
agreed bar.

Every figure below is computed from the sanitized logs in
[`board-copy-speed-2026-10-18/`](board-copy-speed-2026-10-18/): `before/` is the spike's own
sanitized logs and derived JSON, and `after/` is this run's. No committed file holds a board
item's title, body, URL or node id.

## The bars

These are the manager's bars, each derived from the minimum requests its operation needs, at
the spike's measured latencies. At the `plans` board's real size, combined as the spike
combined its before figures:

| Operation | seconds bar | quota bar | before (spike, at `plans` size) |
| --- | --- | --- | --- |
| first copy of the 10-item plan (`project copy` then `document copy`) | 32 s | 12 points, 40 writes | ≈360 s |
| re-copy with one task's body changed | 8 s | 4 points | ≈441 s |
| unchanged re-copy | 7 s | 3 points | ≈440 s |
| one approval through `just approve-design` | 7 s | 5 points | ≈212 s |

The spike never measured a one-body re-copy. Its changed re-copy rewrote every body, which
sends at least as much as a one-body change. That every-body figure is the before beside the
8 s bar. This run measured both.

The quota bars come from the manager's second ruling below. On this host the seconds bars only
warn, and the quota bars are strict.

## What was measured

- **Versions.** onetaskgraph **0.11.0**: the `onetaskgraph-cli` 0.11.0 wheel. It is what
  `config/onetaskgraph.version` and `uv.lock` name at ai-orchestrator `origin/main`
  `46644351` ("perf(deps): adopt plan store 0.11.0 with warn-only approval budgets"), the
  landing of `aio-adopt-copy-speed`. onepipeline **0.86.0** and onepipeline-ui **0.48.1**,
  as `config/onepipeline.version` and `config/onepipeline-ui.version` name them at that
  commit. The clone's `onepipeline --version` answered 0.86.0. `onevcs` was 0.56.5, as the
  clone's environment installed it.
- **Where it ran.** The wheel came from a scratch clone of
  `github.com/nickderobertis/ai-orchestrator` at `46644351`, provisioned with
  `uv sync --frozen` outside this worktree. That clone also ran the host recipes.
- **Harness.** Branch `nick/board-copy-speed-2026-10-18/spike-copy-timing` at `bc49f888`.
  It ran from a scratch copy with only the changes the adopted releases forced; the next
  section lists them. The changed harness and its diff (sha256 `fd9fddc5…4c9b`) are kept on
  this host beside the raw logs, under
  `${XDG_CACHE_HOME:-$HOME/.cache}/board-copy-speed-2026-10-18/harness-after/`.
- **Span build.** Tag `v0.11.0` (`3759333d`) with the spike's `spans-v0.7.1.patch` ported
  onto it (sha256 `cb102049…eb9d4`). Four hunks conflicted, because 0.11.0 paces through one
  `pace()` call: `pacing_wait` now wraps the sleep inside `pace()`, and `retry_wait` wraps
  `sleep_because`. Nothing else changed. The span build is inert against the stock wheel. For
  each of the three test-board scenarios the stock run and the span run sent the same
  operations, in the same order and number, in both processes
  (`after/summary.json` `inertness`). Inside one aliased `WRITE_FIELD_VALUES` the stock
  first copy wrote two field values and the span run three. The extra value was a `Status`
  that the store writes only when the new board item does not already hold it. The span
  build's figures are used for every test-board scenario, as the spike used them.
- **When.** 2026-10-24, 09:38–10:23 UTC, on the test board (owner `nickderobertis`,
  project 1, issues in `nickderobertis/onetaskgraph-live-scratch`) and, for one read-only
  timing, ai-orchestrator's real `plans` board (project 2).
- **Workload.** The spike's seeded plan: one project of about 50 KB, eight tasks of 45–49 KB
  with 10 blocked-by edges, and one design document of about 35 KB (the size the manager
  ruled on for the spike) with a 104,072-byte PNG. Every record names the scratch
  repository. Sizes per run are in each run's `workload.json`.

### What the adopted releases forced

1. **Version and documents.** The harness expects `onetaskgraph 0.11.0`. The proxy's
   operation table (`documents.json`) was regenerated from v0.11.0 by the spike's
   `dump_documents.sh`, and every raw log was named again from it before sanitizing.
2. **Aliased requests.** 0.11.0 sends aliased batches, such as `CREATE_ISSUES`,
   `WRITE_FIELD_VALUES`, `ADD_BLOCKED_BYS`, `ORIGIN_LOOKUPS` and `CREATED_BOARD_ITEMS`. Each
   sanitized mutation record therefore carries `mutations`, the number of aliased
   mutations its answer holds. The analysis classes the new kinds as batched reads and
   aliased mutations.
3. **Write screening.** The host's `onetaskgraph.yaml` now switches the public write
   boundary on. Every write to `plans` is screened through `onevcs boundary inspect` and
   `onevcs boundary check`. The test board's scratch repository is public on GitHub but not
   registered with this host's `onevcs`, so its inspection was refused, and the copy with
   it. **In the scratch process alone, an inspection of the scratch repository inspected
   `nickderobertis/onetaskgraph`, a registered public repository of the same owner, in its
   place, through the same `onevcs`, which still ran its refresh from GitHub.** Every other
   request passed through unchanged. `onevcs` registered nothing. A small shim makes the
   substitution and logs each `onevcs` call's start and end (`screen.jsonl`). The shim adds
   a mean of 0.06 s per call outside the logged interval (10 calls measured: 0.715 s wall
   against 0.656 s inside). That is about 3 s of each host copy's 50 calls, and it falls in
   "unattributed".
4. **No user layer.** On 0.11.0 a host copy contacts every private source the host's user
   layer configures. The spike pointed those sources at an address that answers nothing,
   and 0.11.0 now fails the copy on that. **The host recipes ran with no onetaskgraph user
   layer.** That is the "no user layer" host state ai-orchestrator's own screened-host
   journeys drive. **A real host with private sources configured additionally spends their
   reachability checks, and this measurement does not include them.** The clone's private
   folders, which session setup would create, were created empty.
5. **Review records.** The host's review stamps are recomputed with ai-orchestrator's own
   `plan_review`, as in the spike. For the one-body re-copy, only the task whose review no
   longer matched its content was stamped again. That is what a review of that one task
   records.

### Manager rulings quoted

On the user layer and the visibility stand-in (2026-10-24):

> (a). Do not reach the private Linear sources or private boards, read-only or otherwise;
> this task never authorized reading them. Run the host recipes with no user layer as you
> describe. The visibility stand-in is acceptable: the scratch repository is public, the
> answer is the same, and onevcs still runs. But the evidence must state both deviations
> plainly: that the visibility call inspected nickderobertis/onetaskgraph in place of the
> unregistered scratch repository, and that a real host with private sources configured
> additionally spends their reachability checks, which this measurement does not include.
> Do not register the scratch repository with onevcs. Report every measured figure against
> its approved bar as it stands, overruns included, with the host conditions; under the
> user's CI-only rule a seconds overrun is reported, not optimised in this node.

On the combined figures over their bars (2026-10-24; the figures and dominant phases put to
the manager are those in the combined table below):

> Two more, then finish. (1) Run the one-body-changed re-copy on the test board, bare store
> and through the host recipe, since that is the scenario the 8 s / 4-point bar names; the
> every-body-changed figure stays recorded beside it as a heavier case. (2) Report the quota
> figures against their bars beside the seconds, for every scenario you measured: points,
> requests and writes for first copy (bars 12 points, 40 writes), one-change re-copy (4
> points), unchanged re-copy (3 points) and approval (5 points), bare and host. These quota
> bars are strict even though the seconds bars only warn on this host; say for each whether
> it is within or over. Keep the attribution you already have (screening share, destination
> reads, writes) and the two stated deviations. Optimise nothing.

The before figures rest on two rulings recorded in the spike's report. One kept the
document's body at about 35 KB. The other accepted, without a re-run, seven spike runs that
started under 40% of the GraphQL bucket by GitHub's own headers.

## Each scenario, before beside after

Each cell reads **before → after**. "Bare store" rows run `project copy` then `document
copy` from a `local-md` folder onto the test board, with the store's cache on, as
ai-orchestrator configures `plans`. "Host" rows run ai-orchestrator's own recipe in the
scratch clone, with its `plans` source overridden to the test board. An item is one of the
plan's 10 records: 1 project, 8 tasks and 1 document.

### Wall time by phase

| Scenario | destination reads | whole-board walk pages | writes | pacing waits | secondary-limit waits | other retries (primary-limit waits) | reference rewriting | body composition | asset upload | host-side steps | write-boundary screening (`onevcs`) | engine, local source, CLI | unattributed | wall |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| test board: first copy | 6.07 s → 9.13 s | 13.72 s → 0.00 s | 40.57 s → 15.99 s | 35.30 s → 0.00 s | 0.00 s → 0.00 s | 0.00 s → 0.00 s | 0.01 s → 0.02 s | 0.00 s → 0.00 s | 2.55 s → 2.10 s | 0.00 s → 0.00 s | 0.00 s → 0.00 s | 0.21 s → 2.47 s | 0.03 s → 0.06 s | **98.5 s → 29.8 s** |
| test board: re-copy, every body changed | 9.66 s → 6.41 s | 24.34 s → 0.00 s | 10.50 s → 9.79 s | 6.00 s → 0.00 s | 0.00 s → 0.00 s | 0.00 s → 0.00 s | 0.02 s → 0.06 s | 0.00 s → 0.00 s | 0.00 s → 0.00 s | 0.00 s → 0.00 s | 0.00 s → 0.00 s | 0.72 s → 0.17 s | 0.03 s → 0.11 s | **51.3 s → 16.6 s** |
| test board: re-copy, one task's body changed § | 9.66 s → 7.26 s | 24.34 s → 0.00 s | 10.50 s → 1.09 s | 6.00 s → 0.00 s | 0.00 s → 0.00 s | 0.00 s → 0.00 s | 0.02 s → 0.02 s | 0.00 s → 0.01 s | 0.00 s → 0.00 s | 0.00 s → 0.00 s | 0.00 s → 0.00 s | 0.72 s → 0.19 s | 0.03 s → 0.09 s | **51.3 s → 8.7 s** |
| test board: unchanged re-copy | 6.51 s → 6.45 s | 20.01 s → 0.00 s | 8.71 s → 0.00 s | 6.00 s → 0.00 s | 0.00 s → 0.00 s | 0.00 s → 0.00 s | 0.03 s → 0.02 s | 0.00 s → 0.00 s | 0.00 s → 0.00 s | 0.00 s → 0.00 s | 0.00 s → 0.00 s | 0.30 s → 0.18 s | 0.03 s → 0.07 s | **41.6 s → 6.7 s** |
| host `copy-plan`: first copy | 6.29 s → 7.20 s | 13.13 s → 0.00 s | 38.52 s → 12.41 s | 0.00 s → 0.00 s | 0.00 s → 0.00 s | 0.00 s → 0.00 s | 0.00 s‡ → 0.00 s‡ | 0.00 s‡ → 0.00 s‡ | 1.38 s → 0.82 s | 1.29 s → 2.14 s | 0.00 s → 30.80 s | 0.00 s‡ → 0.00 s‡ | 38.31 s (38.31 s in a stock store process) → 11.04 s (11.04 s in a stock store process) | **98.9 s → 64.4 s** |
| host `copy-plan`: one task's body changed § | 9.66 s → 5.05 s | 24.34 s → 0.00 s | 10.50 s → 1.76 s | 6.00 s → 0.00 s | 0.00 s → 0.00 s | 0.00 s → 0.00 s | 0.02 s → 0.00 s‡ | 0.00 s → 0.00 s‡ | 0.00 s → 0.00 s | 0.00 s → 2.05 s | 0.00 s → 33.40 s | 0.72 s → 0.00 s‡ | 0.03 s → 9.98 s (9.98 s in a stock store process) | **51.3 s → 52.2 s** |
| host `copy-plan`: unchanged re-copy | 7.15 s → 3.89 s | 15.34 s → 0.00 s | 9.70 s → 0.00 s | 0.00 s → 0.00 s | 0.00 s → 0.00 s | 0.00 s → 0.00 s | 0.00 s‡ → 0.00 s‡ | 0.00 s‡ → 0.00 s‡ | 0.00 s → 0.00 s | 1.24 s → 2.17 s | 0.00 s → 30.93 s | 0.00 s‡ → 0.00 s‡ | 7.73 s (7.73 s in a stock store process) → 7.71 s (7.71 s in a stock store process) | **41.2 s → 44.7 s** |
| host `approve-design` | 5.07 s → 5.90 s | 4.20 s → 0.00 s | 1.43 s → 1.01 s | 0.00 s → 0.00 s | 0.00 s → 0.00 s | 0.00 s → 0.00 s | 0.00 s‡ → 0.00 s‡ | 0.00 s‡ → 0.00 s‡ | 0.00 s → 0.00 s | 1.10 s → 1.75 s | 0.00 s → 6.03 s | 0.00 s‡ → 0.00 s‡ | 0.67 s (0.67 s in a stock store process) → 1.73 s (1.73 s in a stock store process) | **12.5 s → 16.4 s** |
| `plans` board: the store's reads of a plan (read-only) ¶ | 0.00 s → 3.82 s | 206.87 s → 0.00 s | 0.00 s → 0.00 s | 0.00 s → 0.00 s | 0.00 s → 0.00 s | 0.00 s → 0.00 s | 0.00 s‡ → 0.00 s‡ | 0.00 s‡ → 0.00 s‡ | 0.00 s → 0.00 s | 0.00 s → 0.00 s | 0.00 s → 0.00 s | 0.00 s‡ → 0.00 s‡ | 3.31 s (3.31 s in a stock store process) → 0.35 s (0.35 s in a stock store process) | **210.2 s → 4.2 s** |

- ‡ A stock-wheel run has no spans, so these phases cannot be separated from the rest of
  the store process's time. They are inside "unattributed (… in a stock store process)",
  and the spike's report shows them as 0 in the same cells. In every span run, of either
  release, reference rewriting and body composition each took under 0.07 s.
- § No one-body before exists. The before is the spike's every-body-changed re-copy, the
  heavier case.
- ¶ Before: the spike's two read-only timings of `plans`. These are the reads 0.7.1's copy
  and approval made of that board: `project list --source plans`, a search walk of 69.2 s,
  and the BOARD half of `task list --source plans`, a board walk of 141.0 s. Together they
  are 210.2 s. After: `task list --project` (2.4 s) and `document list --project` (1.8 s) of
  this plan's own project on `plans`, with every write refused by the proxy. They answered
  8 tasks and 4 documents (`after/plans-read-stock-20261024T095948/answers.json`).
- No retry, primary-limit wait, secondary-limit wait or pacing wait ran in any after run.
  0.11.0's pacing reached no window in these runs, so no `pacing_wait` span was written.
- The phase that dominates each after figure: the bare first copy, writes (16.0 s, 54%).
  The bare changed and one-body re-copies, writes (59%) and destination reads (84%). The
  bare unchanged re-copy, destination reads (96%): its document copy's four single
  `ORIGIN_LOOKUP` reads take 2.8 s, and its two `SUB_ISSUES` listings most of the rest. In every host run, **write-boundary
  screening**: 48% of the first copy, 64% of the one-body re-copy, 69% of the unchanged
  re-copy and 37% of the approval. That is 50 `onevcs` calls in each copy (40 `inspect`,
  10 `check`) and 9 in the approval (8 `inspect`, 1 `check`), each a separate process of
  0.62–0.67 s on average, and none overlapping another (`after/<run>/screen.jsonl`). The unchanged host
  re-copy sends only 4 GraphQL requests and still screens 50 times.

### Requests, points and writes by operation

| Scenario | requests and points by operation, before | requests and points by operation, after | requests (per item) | points (per item) | mutation requests (mutations they carried) | content-creating requests (issues created + uploads) |
| --- | --- | --- | --- | --- | --- | --- |
| test board: first copy | `ADD_BLOCKED_BY` 10×/10 pt, `ADD_SUB_ISSUE` 9×/9 pt, `ADD_TO_BOARD` 10×/10 pt, `BOARD` 1×/2 pt, `CREATE_ISSUE` 10×/10 pt, `CREATION_CONTEXT` 1×/1 pt, `GET /repos/{owner}/{repo}` 1×/0 pt, `ISSUE` 1×/1 pt, `ORIGIN_LOOKUP` 8×/8 pt, `POST /user-attachments/assets` 1×/0 pt, `REPOSITORY` 1×/1 pt, `SEARCH_ISSUES` 2×/10 pt, `UPDATE_FIELD` 1×/1 pt, `UPDATE_FIELDS` 9×/9 pt | `ADD_BLOCKED_BYS` 1×/1 pt, `CREATED_BOARD_ITEMS` 4×/4 pt, `CREATE_ISSUE` 2×/2 pt, `CREATE_ISSUES` 1×/1 pt, `ISSUE` 1×/1 pt, `ORIGIN_LOOKUP` 3×/3 pt, `ORIGIN_LOOKUPS` 2×/4 pt, `POST /user-attachments/assets` 1×/0 pt, `SUB_ISSUES` 1×/1 pt, `WRITE_FIELD_VALUES` 3×/3 pt | 65 (6.5) → 19 (1.9) | 72 (7.2) → 20 (2.0) | 49 (49) → 7 (35) | 11 → 4 (10 + 1) |
| test board: re-copy, every body changed | `BOARD` 2×/4 pt, `ISSUE` 11×/11 pt, `SEARCH_ISSUES` 2×/10 pt, `UPDATE_ISSUE` 10×/10 pt | `ORIGIN_LOOKUP` 4×/4 pt, `SUB_ISSUES` 2×/2 pt, `UPDATE_ISSUE` 10×/10 pt | 25 (2.5) → 16 (1.6) | 35 (3.5) → 16 (1.6) | 10 (10) → 10 (10) | 0 → 0 (0 + 0) |
| test board: re-copy, one task's body changed § | `BOARD` 2×/4 pt, `ISSUE` 11×/11 pt, `SEARCH_ISSUES` 2×/10 pt, `UPDATE_ISSUE` 10×/10 pt | `ORIGIN_LOOKUP` 4×/4 pt, `SUB_ISSUES` 2×/2 pt, `UPDATE_ISSUE` 1×/1 pt | 25 (2.5) → 7 (0.7) | 35 (3.5) → 7 (0.7) | 10 (10) → 1 (1) | 0 → 0 (0 + 0) |
| test board: unchanged re-copy | `BOARD` 2×/4 pt, `ISSUE` 11×/11 pt, `SEARCH_ISSUES` 2×/10 pt, `UPDATE_ISSUE` 9×/9 pt | `ORIGIN_LOOKUP` 4×/4 pt, `SUB_ISSUES` 2×/2 pt | 24 (2.4) → 6 (0.6) | 34 (3.4) → 6 (0.6) | 9 (9) → 0 (0) | 0 → 0 (0 + 0) |
| host `copy-plan`: first copy | `ADD_BLOCKED_BY` 10×/10 pt, `ADD_SUB_ISSUE` 9×/9 pt, `ADD_TO_BOARD` 10×/10 pt, `BOARD` 1×/2 pt, `CREATE_ISSUE` 10×/10 pt, `CREATION_CONTEXT` 1×/1 pt, `GET /repos/{owner}/{repo}` 1×/0 pt, `ISSUE` 2×/2 pt, `ORIGIN_LOOKUP` 8×/8 pt, `POST /user-attachments/assets` 1×/0 pt, `REPOSITORY` 1×/1 pt, `SEARCH_ISSUES` 2×/10 pt, `UPDATE_FIELD` 1×/1 pt, `UPDATE_FIELDS` 9×/9 pt | `ADD_BLOCKED_BYS` 1×/1 pt, `CREATED_BOARD_ITEMS` 4×/4 pt, `CREATE_ISSUE` 2×/2 pt, `CREATE_ISSUES` 1×/1 pt, `ISSUE` 2×/2 pt, `ORIGIN_LOOKUP` 1×/1 pt, `ORIGIN_LOOKUPS` 2×/4 pt, `POST /user-attachments/assets` 1×/0 pt, `SUB_ISSUES` 1×/1 pt, `WRITE_FIELD_VALUES` 1×/1 pt | 66 (6.6) → 16 (1.6) | 73 (7.3) → 17 (1.7) | 49 (49) → 5 (24) | 11 → 4 (10 + 1) |
| host `copy-plan`: one task's body changed § | `BOARD` 2×/4 pt, `ISSUE` 11×/11 pt, `SEARCH_ISSUES` 2×/10 pt, `UPDATE_ISSUE` 10×/10 pt | `BOARD_FIELDS` 1×/1 pt, `ISSUE` 1×/1 pt, `ORIGIN_LOOKUP` 1×/1 pt, `SUB_ISSUES` 2×/2 pt, `UPDATE_ISSUE` 2×/2 pt | 25 (2.5) → 7 (0.7) | 35 (3.5) → 7 (0.7) | 10 (10) → 2 (2) | 0 → 0 (0 + 0) |
| host `copy-plan`: unchanged re-copy | `BOARD` 2×/4 pt, `ISSUE` 13×/13 pt, `SEARCH_ISSUES` 2×/10 pt, `UPDATE_ISSUE` 9×/9 pt | `ISSUE` 1×/1 pt, `ORIGIN_LOOKUP` 1×/1 pt, `SUB_ISSUES` 2×/2 pt | 26 (2.6) → 4 (0.4) | 36 (3.6) → 4 (0.4) | 9 (9) → 0 (0) | 0 → 0 (0 + 0) |
| host `approve-design` | `ISSUE` 3×/3 pt, `SEARCH_ISSUES` 1×/5 pt, `SUB_ISSUES` 2×/2 pt, `UPDATE_ISSUE` 1×/1 pt | `ISSUE` 3×/3 pt, `SUB_ISSUES` 3×/3 pt, `UPDATE_ISSUE` 1×/1 pt | 7 (0.7) → 7 (0.7) | 11 (1.1) → 7 (0.7) | 1 (1) → 1 (1) | 0 → 0 (0 + 0) |
| `plans` board: the store's reads of a plan (read-only) ¶ | `plans-walk`: `SEARCH_ISSUES` 10×/50 pt; `plans-board-walk`: `BOARD` 20×/40 pt | `SUB_ISSUES` 2×/2 pt | 30 → 2 (0.17 per record answered, 12 records) | 90 → 2 | 0 (0) → 0 (0) | 0 → 0 (0 + 0) |

Points are GitHub's dry-run price for a query. A mutation request costs 1 point, the
smallest `x-ratelimit-used` rise, including an aliased request of 11. The prices are in
`after/pricing.json` and `before/pricing.json`. This run's single observation of
`CREATE_ISSUE` rose 4, because other sessions drew on the same bucket, so the spike's
40-observation price of 1 is kept. Content-creating requests are every `createIssue`
request, aliased or not, and every upload. An aliased `CREATE_ISSUES` creates 8 issues in
one request. The `onevcs` tunnels to `api.github.com` are never counted as requests: there are 40 in
each host copy and 8 in the approval (`by_op` in `after/summary.json`). A first host copy
also opened one tunnel to `cafe.github.com`, whose sender the proxy cannot see.

### Quota before each scenario

The corrected gate took the least of four readings: `GET /rate_limit`, three GraphQL
`rateLimit` reads, and the `x-ratelimit-*` headers of the last real answer any run recorded
in the current window. It paused while any bucket it draws on held under 40% of its limit,
unless the bucket's burn rate showed it resetting first. Every reading is in its run's
`quota.json` under `gate`. The figure gated on is the `graphql` bucket, which is the least.

| Scenario (after run) | GraphQL gate figure: least of the readings (remaining / limit, reset UTC) | `GET /rate_limit` graphql | GraphQL `rateLimit`, three reads | last real answer's `x-ratelimit-*` | core (`GET /rate_limit`) | pauses |
| --- | --- | --- | --- | --- | --- | --- |
| test board: first copy (`tb-first-span-20261024T095836`, read 09:58:36) | 4247 / 5000 (reset 10:26:48) | 4605 / 5000 (reset 10:05:02) | 4247 / 5000 (reset 10:26:48); reads 4605, 4605, 4247 | 4261 / 5000 (reset 10:26:48) | 4997 / 5000 (reset 10:58:06) | none recorded |
| test board: re-copy, every body changed (`tb-changed-span-20261024T095909`, read 09:59:09) | 4188 / 5000 (reset 10:26:48) | 4593 / 5000 (reset 10:05:02) | 4188 / 5000 (reset 10:26:48); reads 4592, 4189, 4188 | 4196 / 5000 (reset 10:26:48) | 4985 / 5000 (reset 10:58:06) | none recorded |
| test board: re-copy, one task's body changed (`tb-changed1-span-20261024T101530`, read 10:15:30) | 3661 / 5000 (reset 10:26:48) | 4864 / 5000 (reset 11:05:05) | 3661 / 5000 (reset 10:26:48); reads 3661, 3661, 3661 | 3661 / 5000 (reset 10:26:48) | 4770 / 5000 (reset 10:58:06) | none recorded |
| test board: unchanged re-copy (`tb-unchanged-span-20261024T095930`, read 09:59:30) | 4157 / 5000 (reset 10:26:48) | 4578 / 5000 (reset 10:05:02) | 4157 / 5000 (reset 10:26:48); reads 4157, 4578, 4157 | 4157 / 5000 (reset 10:26:48) | 4985 / 5000 (reset 10:58:06) | none recorded |
| host `copy-plan`: first copy (`host-copy-stock-20261024T100745`, read 10:07:45) | 4046 / 5000 (reset 10:26:48) | 4987 / 5000 (reset 11:05:05) | 4046 / 5000 (reset 10:26:48); reads 4987, 4046, 4046 | 4051 / 5000 (reset 10:26:48) | 4882 / 5000 (reset 10:58:06) | none recorded |
| host `copy-plan`: one task's body changed (`host-changed1-stock-20261024T102149`, read 10:21:49) | 3317 / 5000 (reset 10:26:48) | 4786 / 5000 (reset 11:05:05) | 3317 / 5000 (reset 10:26:48); reads 3317, 3317, 3317 | 3326 / 5000 (reset 10:26:48) | 4543 / 5000 (reset 10:58:06) | none recorded |
| host `copy-plan`: unchanged re-copy (`host-recopy-stock-20261024T100859`, read 10:08:59) | 4020 / 5000 (reset 10:26:48) | 4982 / 5000 (reset 11:05:05) | 4020 / 5000 (reset 10:26:48); reads 4023, 4021, 4020 | 4025 / 5000 (reset 10:26:48) | 4841 / 5000 (reset 10:58:06) | none recorded |
| host `approve-design` (`host-approve-stock-20261024T100947`, read 10:09:47) | 3961 / 5000 (reset 10:26:48) | 4963 / 5000 (reset 11:05:05) | 3961 / 5000 (reset 10:26:48); reads 4963, 3961, 4963 | 3963 / 5000 (reset 10:26:48) | 4801 / 5000 (reset 10:58:06) | none recorded |
| `plans` board: the store's reads of a plan (read-only) (`plans-read-stock-20261024T095948`, read 09:59:48) | 4154 / 5000 (reset 10:26:48) | 4574 / 5000 (reset 10:05:02) | 4154 / 5000 (reset 10:26:48); reads 4154, 4154, 4154 | 4154 / 5000 (reset 10:26:48) | 4985 / 5000 (reset 10:58:06) | none recorded |

The two counters disagreed again, as the spike found: `GET /rate_limit` reported its GraphQL
bucket resetting at 10:05:02 and then at 11:05:05, while `rateLimit` and the answers'
headers reported one window resetting at 10:26:48, up to 1,513 points emptier. The lowest
gate figure of any after run was 3,317 / 5,000 (66%, host one-body re-copy). The lowest of
any reading, superseded runs and cleanups included, was 3,250 (65%), before the last cleanup at
10:22:49. No pause was
needed, and none is recorded.

## Combined at the `plans` board's real size, against the bars

The spike scaled each test-board figure to `plans` by replacing every test-board walk page
with the pages the same walk takes there: 20 BOARD pages × 6.95 s and 10 SEARCH_ISSUES pages
× 6.79 s per walk, and 3 listing processes for an approval. Everything else was kept as
measured (`before/combined.json`). The same arithmetic is applied to the after runs below.
**No after run sent a BOARD or SEARCH_ISSUES page**, so 0 pages are replaced and each figure
at real size is its measured figure. The read-only timing on `plans` itself supports that:
the store's reads of a plan there were one `SUB_ISSUES` request each, 2.4 s and 1.8 s, on a
board of about 2,000 items.

| Figure | before at `plans` size (spike's arithmetic) | after at `plans` size (same arithmetic) | seconds bar | within? | points / requests / writes after | quota bar | within? |
| --- | --- | --- | --- | --- | --- | --- | --- |
| first copy, bare store | 98.5 s measured - 13.6 s of 1 BOARD + 2 SEARCH test-board pages + 20 x 6.95 s + 20 x 6.79 s = 360 s; 200 points, 102 requests (`tb-first-span-20261018T203458`) | 29.8 s measured − 0.0 s of 0 BOARD + 0 SEARCH test-board pages + 0 × 6.95 s + 0 × 6.79 s = **29.8 s** (`tb-first-span-20261024T095836`) | 32 s | within (29.8 s) | 20 points / 19 requests / 36 writes (7 mutation requests carrying 35 mutations, 1 upload) | 12 points, 40 writes | over |
| first copy, host `copy-plan` | 98.9 s measured - 13.1 s of 1 BOARD + 2 SEARCH test-board pages + 20 x 6.95 s + 20 x 6.79 s = 361 s; 201 points, 103 requests (`host-copy-stock-20261018T205013`) | 64.4 s measured − 0.0 s of 0 BOARD + 0 SEARCH test-board pages + 0 × 6.95 s + 0 × 6.79 s = **64.4 s** (`host-copy-stock-20261024T100745`) | 32 s | over (64.4 s) | 17 points / 16 requests / 25 writes (5 mutation requests carrying 24 mutations, 1 upload) | 12 points, 40 writes | over |
| re-copy, one body changed, bare store | 51.3 s measured - 24.2 s of 2 BOARD + 2 SEARCH test-board pages + 40 x 6.95 s + 20 x 6.79 s = 441 s; 201 points, 81 requests (`tb-changed-span-20261018T203717`) | 8.7 s measured − 0.0 s of 0 BOARD + 0 SEARCH test-board pages + 0 × 6.95 s + 0 × 6.79 s = **8.7 s** (`tb-changed1-span-20261024T101530`) | 8 s | over (8.7 s) | 7 points / 7 requests / 1 writes (1 mutation requests carrying 1 mutations, 0 upload) | 4 points | over |
| re-copy, one body changed, host `copy-plan` | 51.3 s measured - 24.2 s of 2 BOARD + 2 SEARCH test-board pages + 40 x 6.95 s + 20 x 6.79 s = 441 s; 201 points, 81 requests (`tb-changed-span-20261018T203717`) | 52.2 s measured − 0.0 s of 0 BOARD + 0 SEARCH test-board pages + 0 × 6.95 s + 0 × 6.79 s = **52.2 s** (`host-changed1-stock-20261024T102149`) | 8 s | over (52.2 s) | 7 points / 7 requests / 2 writes (2 mutation requests carrying 2 mutations, 0 upload) | 4 points | over |
| re-copy, every body changed, bare store (heavier case) | 51.3 s measured - 24.2 s of 2 BOARD + 2 SEARCH test-board pages + 40 x 6.95 s + 20 x 6.79 s = 441 s; 201 points, 81 requests (`tb-changed-span-20261018T203717`) | 16.6 s measured − 0.0 s of 0 BOARD + 0 SEARCH test-board pages + 0 × 6.95 s + 0 × 6.79 s = **16.6 s** (`tb-changed-span-20261024T095909`) | 8 s | over (16.6 s) | 16 points / 16 requests / 10 writes (10 mutation requests carrying 10 mutations, 0 upload) | 4 points | over |
| unchanged re-copy, bare store | 41.6 s measured - 19.9 s of 2 BOARD + 2 SEARCH test-board pages + 40 x 6.95 s + 20 x 6.79 s = 435 s; 200 points, 80 requests (`tb-unchanged-span-20261018T203848`) | 6.7 s measured − 0.0 s of 0 BOARD + 0 SEARCH test-board pages + 0 × 6.95 s + 0 × 6.79 s = **6.7 s** (`tb-unchanged-span-20261024T095930`) | 7 s | within (6.7 s) | 6 points / 6 requests / 0 writes (0 mutation requests carrying 0 mutations, 0 upload) | 3 points | over |
| unchanged re-copy, host `copy-plan` | 41.2 s measured - 15.3 s of 2 BOARD + 2 SEARCH test-board pages + 40 x 6.95 s + 20 x 6.79 s = 440 s; 202 points, 82 requests (`host-recopy-stock-20261018T205156`) | 44.7 s measured − 0.0 s of 0 BOARD + 0 SEARCH test-board pages + 0 × 6.95 s + 0 × 6.79 s = **44.7 s** (`host-recopy-stock-20261024T100859`) | 7 s | over (44.7 s) | 4 points / 4 requests / 0 writes (0 mutation requests carrying 0 mutations, 0 upload) | 3 points | over |
| approval, host `approve-design` | 12.5 s measured - 4.2 s of 0 BOARD + 1 SEARCH test-board pages + 0 x 6.95 s + 30 x 6.79 s = 212 s; 156 points, 36 requests (`host-approve-stock-20261018T205238`) | 16.4 s measured − 0.0 s of 0 BOARD + 0 SEARCH test-board pages + 0 × 6.95 s + 0 × 6.79 s = **16.4 s** (`host-approve-stock-20261024T100947`) | 7 s | over (16.4 s) | 7 points / 7 requests / 1 writes (1 mutation requests carrying 1 mutations, 0 upload) | 5 points | over |

Reading the table: the bare store's first copy (29.8 s) and unchanged re-copy (6.7 s) are
within their seconds bars. Its one-body re-copy (8.7 s) is 0.7 s over, and its destination
reads are 7.3 s of it. Every host recipe is over its seconds bar, and write-boundary
screening is the largest phase in each. Every scenario, bare and host, is over its points
bar. The two first copies are within 40 writes (36 and 25). Against the before figures, a
copy at real size is 5.6–65× faster (first 360 → 29.8 s bare, 361 → 64.4 s host; unchanged
440 → 44.7 s host, 435 → 6.7 s bare). An approval is 13× faster (212 → 16.4 s), and points
fall from 200–202 to 4–20 per copy and from 156 to 7 per approval.

Why the points exceed the quota bars, from the request table: the bare unchanged re-copy's
6 points are 2 `SUB_ISSUES` listings and 4 single `ORIGIN_LOOKUP` reads by the document
copy. The loopback journey's recording answers the same copy with 1 batched read and 1
`ISSUE` read (2 points). The first copy's 20 points include 4 `CREATED_BOARD_ITEMS` polls,
because GitHub adds a created issue to the board asynchronously, and 3 `ORIGIN_LOOKUP`
reads. The loopback journey records 2 and 0 of those.

**What judges these figures.** The manager's reading of these combined figures against their
bars judges them, and so does any later re-run of this harness against GitHub. Whether they
meet their bars is not this file's to decide.

## The loopback budgets that hold the same bars

Each code node holds its bar on a loopback board inside its own suite. Each figure below is
what that budget's own command reported on the adopted releases: onetaskgraph tag `v0.11.0`
and ai-orchestrator `46644351`, run on this host on 2026-10-24. The results are in
`after/loopback-budgets/`.

| Budget | bar | figure its command reports | its breakdown | measured on GitHub here (at `plans` size) |
| --- | --- | --- | --- | --- |
| `github-plan-copy-seconds-first` (onetaskgraph) | 32 s | 30.877 s | `project` 24.984 s + `document` 5.893 s simulated, 14 requests | 29.8 s bare, 64.4 s host |
| `github-plan-copy-seconds-changed` (onetaskgraph) | 8 s | 6.187 s | `project` 5.633 s + `document` 0.554 s, 3 requests (one body changed) | 8.7 s bare, 52.2 s host (one body); 16.6 s bare (every body) |
| `github-plan-copy-seconds-unchanged` (onetaskgraph) | 7 s | 5.105 s | `project` 4.551 s + `document` 0.554 s, 2 requests | 6.7 s bare, 44.7 s host |
| `plan-approval-seconds` (ai-orchestrator) | 7 s | 8.407 s | 3.45 s real elapsed + 4.95 s charged for 8 requests (`document metadata` 3.30 s for 5 requests) | 16.4 s host |

The onetaskgraph figures were reported by
`cargo test -p onetaskgraph-github-projects-e2e --all-features --locked --test e2e --
budget_runner::report_plan_copy --exact --nocapture`, over the telemetry its `plan_copy::`
tests recorded. The ai-orchestrator figure was reported by `python -m tests.budget_telemetry`
over its `test-host` journey's telemetry. Those commands came from a clone outside this
host's registered checkouts, because the suite refuses to run inside one.

Every budget charges each request the latency of its kind. The two repositories' tables hold
the same nine values. They are set beside the spike report's measured means, and beside this
run's:

| Kind (onetaskgraph / ai-orchestrator name) | ms charged by both budgets | spike report's measured mean | this run's measured mean |
| --- | --- | --- | --- |
| `ItemRead` / item read | 554 | 554 (item read, n 176) | 576 (n 342) |
| `BoardPage` / board page | 6,948 | 6,948 (board page, `plans`, n 20) | none sent |
| `SearchPage` / search page | 6,791 | 6,791 (search page, `plans`, n 10) | none sent |
| `Mutation` / mutation | 1,082 | 1,082 (mutation, n 228) | 1,966 (n 104) |
| `Upload` / asset upload | 927 | 927 (upload, n 3) | 916 (n 5) |
| `Repository` / asset upload's repository read | 526 | 526 (n 3) | none sent |
| `AliasedMutations` / request of up to 11 aliased mutations | 3,700 | **not a mean**: the one aliased probe's 3.7 s answer | 2,929 (n 21) |
| `Verification` / asset upload's verification | 1,140 | **not a mean**: the one span the report gives (1.14 s) | not timed (tunnelled) |
| `BatchedRead` / read batching several items | 4,551 | **a different kind**: the test board's BOARD page mean (4,551); the spike sent no batched read | 736 (n 30) |

Differences between the tables: none between the two repositories' tables. Against the
spike's means, three charged values are not a mean of their kind, as marked. Against this
run, mutations took longer than the charge (the mean includes 50 KB `updateIssue` bodies
and cleanup's deletes), and batched reads and aliased mutations took less.

Every request kind, as the spike's report tabulated it, before and after (all runs but the probes, cleanups included; `before/summary.json` and `after/summary.json` `latency`):

| Request kind | before: n, median / mean ms | after: n, median / mean ms |
| --- | --- | --- |
| BOARD page (plans) | 20, 6425 / 6948 | 0 sent |
| BOARD page (test board) | 13, 4430 / 4551 | 0 sent |
| SEARCH_ISSUES page (plans) | 10, 6737 / 6791 | 0 sent |
| SEARCH_ISSUES page (test board) | 25, 4665 / 5036 | 0 sent |
| aliased mutations | 0 sent | 21, 2479 / 2929 |
| batched read | 0 sent | 30, 338 / 736 |
| item read | 176, 489 / 554 | 342, 458 / 576 |
| mutation | 228, 860 / 1082 | 104, 2023 / 1966 |
| upload: GET /repos/{owner}/{repo} | 3, 511 / 526 | 0 sent |
| upload: POST /user-attachments/assets | 3, 908 / 927 | 5, 919 / 916 |

## Writes

Counted from every committed after log, superseded runs included. `plans` received only the
read-only timing's two queries. The proxy refused every mutation and upload there, and none
was attempted. `followups` received nothing: its endpoint in every scratch process was an
address that answers nothing, and no layer of this run copied to it.

| Destination | sent by | runs | mutation requests | mutations they carried | uploads | issues created | requests the proxy refused |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `plans` (ai-orchestrator's real board) | the read-only timing | 1 | 0 | 0 | 0 | 0 | 0 |
| onetaskgraph's test board | cleanup (outside the copy scenarios) | 4 | 70 | 70 | 0 | 0 | 0 |
| onetaskgraph's test board | copy and approval scenarios the figures are computed from | 13 | 55 | 175 | 5 | 50 | 0 |
| onetaskgraph's test board | copy scenarios, superseded runs (refused attempts and re-measured runs) | 11 | 42 | 94 | 2 | 38 | 0 |
| `followups` (ai-orchestrator) | nothing: its endpoint was an address that answers nothing | 0 | 0 | 0 | 0 | 0 | 0 |

- **Copy scenarios:** 88 issues were created, all on the test board; every one is deleted. The store took back 18 in the two refused host attempts,
  before the user-layer and folder deviations above were in place. Each of those attempts
  created 9 issues and deleted the same 9 when the copy could not finish. Cleanup deleted the
  other 70: every issue of the five plans that landed, which is every issue of the seven plans that
  landed (10 each), the first copies the re-measured host runs replaced among them.
- **Disposable test-board items created outside the copy scenarios: 0, and 0 deleted.** This
  run sent no probe and created nothing outside a copy scenario. Cleanup's 70 deletions are
  of copy-scenario issues.
- Every cleanup read each issue back first and deleted it only when its title was one this
  harness generates and its repository the scratch repository. It refused none and failed
  none. Its last pass deleted the last 10 and found the other 78 already gone.

## What this run spent

From the committed after logs: 29 runs (8 copy and approval runs in the rows above, 5
supporting runs, 4 cleanups, 1 read-only timing and 11 superseded) sent 587 requests through the proxy, about 599 points by the prices above.
Separately, 391 tunnels to `api.github.com` were opened while
screening, and 5 to `cafe.github.com`; the proxy cannot see inside them. The quota gate took 29 readings, and each run read once more after it finished:
58 `GET /rate_limit` and 174 GraphQL `rateLimit` queries. Outside the logs, counted by hand:

- 1 gate reading of an attempt that crashed in the harness before sending anything;
- 7 pricing dry runs (`after/pricing.json`);
- 1 REST read of the scratch repository's visibility;
- 15 `onevcs` inspections run by hand to diagnose its refusal and time the shim;
- the loopback budget runs, which reach nothing.

## Supporting runs

These are second samples, recorded and not used in the rows above:

| Run | what it is | wall | requests | points |
| --- | --- | --- | --- | --- |
| `tb-first-stock-20261024T093842` | bare first copy, stock wheel (inertness pair) | 33.1 s | 19 | 20 |
| `tb-changed-stock-20261024T093934` | bare every-body re-copy, stock wheel (inertness pair) | 18.0 s | 16 | 16 |
| `tb-unchanged-stock-20261024T093958` | bare unchanged re-copy, stock wheel (inertness pair) | 7.7 s | 6 | 6 |
| `tb-first-span-20261024T101453` | bare first copy the one-body re-copy was made on | 31.3 s | 19 | 20 |
| `host-copy-stock-20261024T102033` | host first copy the host one-body re-copy was made on | 64.7 s | 16 | 17 |

`after/superseded/` holds every other run of this measurement: the refused host attempts, and
the first set of host runs, which was re-measured once the screening log existed (64.4 s →
76.7 s first copy, 44.7 s → 54.2 s unchanged, 16.4 s → 19.4 s approval). Its writes are
counted above.

## Limits on how far the numbers carry

- One sample per scenario per build, at one hour. Mutation latency was nearly twice the
  spike's mean this hour.
- The real-size figures assume nothing but the walk pages grows with the board. Every after
  request is item-scoped. `ORIGIN_LOOKUP` filters the board's items server-side, though, and
  its latency on a 2,000-item board was not measured here. The one read-only timing of
  `plans` covered `task list --project` and `document list --project` only.
- The host figures include the shim's overhead (about 0.06 s per `onevcs` call). They
  exclude a real host's reachability checks of its private sources.
- The bare store's runs had no `write_policy`, as the spike's had none. The host runs
  screened every write.
