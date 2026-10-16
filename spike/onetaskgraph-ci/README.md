# CI and CD time spike for onetaskgraph (throwaway, never lands)

The report is `authoring:spike-onetaskgraph-ci-report`. Everything here is read-only against GitHub.

## The one command

`measure.py figures` reads this repository's Actions run, job, pull-request and release
records through `gh` and writes `figures.json`. It logs every request to `requests.jsonl`
with the core quota GitHub's `/rate_limit` last reported (polled every 10 requests). It
refuses any request below 2,500 remaining or past 3,000 logged. A response kept under
`--raw-dir` is never requested again.

This is how the committed `figures.json` was produced, offline, from the committed raw
responses, run from the worktree root:

    d="$(mktemp -d)" && tar -xJf spike/onetaskgraph-ci/raw.tar.xz -C "$d"
    cp spike/onetaskgraph-ci/requests.jsonl "$d/log.jsonl"
    python3 -I spike/onetaskgraph-ci/measure.py figures --repo nickderobertis/onetaskgraph \
      --since 2026-09-15 --until 2026-10-15 --raw-dir "$d/raw" --log "$d/log.jsonl" \
      --out "$d/figures.json" --offline
    cmp "$d/figures.json" spike/onetaskgraph-ci/figures.json   # identical, under any PYTHONHASHSEED

`--offline` makes no request, so the log copy comes back unchanged. The online runs that
filled the raw store used the same arguments, without `--offline`. `raw.tar.xz` holds every
response they read: 1,283 requests, 160 `rate_limit` polls and 3 calls by hand, all in
`requests.jsonl`.

Example of reproducing one run and one PR from the raw store: PR #1145 has a checks phase
of 1,737 s and a merge-to-release time of 5,478 s, the latter ending at v0.2.32's last CD
job. Its head-commit `ci.yml` run, 35001678155, has 9 jobs, the longest being
`check (ubuntu-latest)` at 1,733 s. Both are in `figures.json` under `cycle.prs` and
`per_run.ci_pull_request_runs.detail`.

## Other files

- `tables.py figures.json [local-runs/<run>...]` renders the report's tables, including
  each local run's per-tier span.
- `slowest.py <nextest log> [N]` lists the N slowest tests from nextest's own timing lines.
- `loggaps.py <job log> [s]` lists a job log's steps and its quiet gaps.
- `local-profile.sh <out> <nx-home> <recipe...>` times one local recipe with a fresh or
  reused Nx home, with live credentials unset. Its runs are in `local-runs/`; see the
  report for which of them are valid.
- `required-status-checks.json` is the dated snapshot of `main`'s required checks.
