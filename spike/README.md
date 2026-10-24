# Board copy: the timing spike's harness, extended by the feasibility spike

Throwaway measurement, kept as evidence and as the starting point the implementing tasks grow
into their budgets. Nothing here lands. It was written for `authoring:board-copy-speed-2026-10-18`
(branch `nick/board-copy-speed-2026-10-18/spike-copy-timing`, report
`board-copy-speed-2026-10-18-spike-copy-timing-report`), and the feasibility spike of
`authoring:board-copy-gaps-2026-10-24` extended it on `nick/board-copy-gaps-2026-10-24/spike-feasibility`
(report `board-copy-gaps-2026-10-24-spike-feasibility-report`). Runs and state now go under
`${XDG_CACHE_HOME:-$HOME/.cache}/board-copy-gaps-2026-10-24/` unless `SPIKE_CACHE_NAME` names
the timing spike's directory. The timing spike's own logs are in
`docs/measurements/board-copy-speed-2026-10-18/`.

## The feasibility spike's scenarios

| Scenario | What it measures |
| --- | --- |
| `probe-alias-pricing` | read-only: GitHub's dry-run price of the plugin's documents at each alias count, plus coalesced and parent-scoped shapes, each charged once for real |
| `probe-placement` | 9 disposable issues created as a coalesced first copy creates them: when each lands on the board, `addProjectV2ItemById` racing GitHub's placement, one 19-alias field-and-edge write, the charge of a 19-alias request, then deletes them |
| `reach-public`, `reach-private-item`, `reach-private-ref`, `reach-unresolvable-ref` | offline (`harness/reach.py`): the real CLI and real onevcs over synthetic identities, copying a public plan while two synthetic private sources answer 503 |
| `screen-union` | offline (`harness/union.py`): the public copy's onevcs calls replayed per item, then deduplicated and unioned, then with a private term planted |
| `proxy-refusal` | offline (`harness/refusal.py`): the proxy refusing every write naming a destination other than the test board |

`--build head` runs `target/debug/onetaskgraph` of this checkout and `--build closure` the
prototype that `spike/prototype/build.sh` builds from `spike/prototype/reach-closure.patch`.
`--build stock` offline needs `SPIKE_HOST_CLONE` and `SPIKE_VERSION` naming a 0.11.0 wheel.
`python3 harness/gaps.py` writes `logs/gaps-summary.json` from the committed logs, and
`python3 harness/report_answers.py <out.json> [<ruling>]` writes the report's answers.

## One command, one measurement

    spike/copy-timing.sh provision                       # once: clone ai-orchestrator, build the span build
    spike/copy-timing.sh <scenario> [--build stock|span] # one measurement of one scenario

| Scenario | What it measures |
| --- | --- |
| `tb-first` | `project copy` then `document copy` of the generated plan from a `local-md` folder onto onetaskgraph's test board (owner `nickderobertis`, project 1, repository `nickderobertis/onetaskgraph-live-scratch`), creating every item |
| `tb-changed` | the same copy after every body changed |
| `tb-unchanged` | the same copy with nothing changed |
| `host-copy` | ai-orchestrator's own `just copy-plan`, in a scratch clone, its `plans` source pointed at the test board; run again once the plan is on the board, it is recorded as `host-recopy` (unchanged) |
| `host-approve` | ai-orchestrator's `just approve-design` of that copy |
| `plans-walk` | one `project list --source plans --json` of ai-orchestrator's real `plans` board, every mutation refused |
| `plans-board-walk` | `task list --source plans --json`'s BOARD pages of the real `plans` board, its search half refused (allowed by the manager in addition to the one listing) |
| `tb-list-pages` | `project list` of the test board two a page, following `next` as ai-orchestrator's `every_page` does |
| `probes` | `createIssue` with `projectV2Ids` and with `parentIssueId`, and one request of eleven aliased mutations, on disposable test-board issues, all deleted |
| `probe-body-limit` | whether GitHub takes a 77,000-byte issue body in 63,500 characters and refuses 66,000 characters (the limit's unit), both issues deleted |
| `cleanup` | delete every issue this spike created (`--plan bcs-stock\|bcs-span\|bcs-host` for one plan) |

`--build stock` runs the `onetaskgraph-cli` 0.7.1 wheel ai-orchestrator locks; `--build span`
runs onetaskgraph's `v0.7.1` tag with `span-build/spans-v0.7.1.patch` applied (timing spans,
nothing else), which writes one JSON line per span only when `SPIKE_SPAN_LOG` names a file.
The test board's credential is `GH_PROJECTS_TOKEN`, read from ai-orchestrator's `.env` when
unset (`SPIKE_TOKEN_ENV` names another file).

Every request goes through `harness/proxy.py`, a recording proxy the source's `endpoint`
override points at (and `HTTPS_PROXY`, for attachment verification). It refuses any mutation
naming a repository or project other than the test board's, and every mutation in the
`plans` walks. Before each scenario `harness/quota.py` reads `GET /rate_limit` and GraphQL's
own `rateLimit`, and pauses while a bucket it draws on is under 40% unless the burn rate
shows it resetting first.

## Where things are

- Raw logs (whole bodies, every board page) stay under
  `${XDG_CACHE_HOME:-$HOME/.cache}/board-copy-speed-2026-10-18/runs/`, outside every tree.
- `logs/<scenario>-<build>-<time>/` holds what `harness/sanitize.py` lets through: timings,
  operation names, counts, costs and limit headers; no title, body, URL or node id.
  `logs/superseded/` holds, in the same form, every run the report does not draw on (runs
  re-measured, cleanups, an ad-hoc board check), so the spike's own consumption adds up
  from what is committed.
- `python3 harness/analyze.py` writes `logs/summary.json` (wall-time split, requests and
  points by operation, the inertness comparison, latency per request kind);
  `harness/combine.py` writes `logs/combined.json` (the real-size figures, with the
  arithmetic); `harness/pricing.py` writes `logs/pricing.json`; `harness/consumption.py`
  writes `logs/consumption.json` (what the spike itself sent, totalled from the committed
  logs — each `GET /rate_limit` and each `rateLimit` query of a quota reading, the gate's
  and the one after each run, its own request — and an audit of every write);
  `logs/quota-readings.jsonl` is every quota reading and pause the gate recorded.
- `.venv/bin/pytest spike/tests` runs the harness's own tests (no credential, no network).
