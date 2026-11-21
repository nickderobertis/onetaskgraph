# Spike `spike-store-defects` harness

Throwaway. It is kept on the spike branch so the `store-fixes` node can start from it. It is
never landed.

## Defect 1: status set on an open issue under a terminal card

`crates/onetaskgraph-github-projects-e2e/tests/e2e/spike_status_set_close.rs` is a loopback
journey through the real binary:

    cargo test --quiet -p onetaskgraph --all-features --locked --no-run
    cargo test -p onetaskgraph-github-projects-e2e --locked --test e2e spike_status_set_close

On `9b9520bd` (v0.14.20), the two terminal cases fail: the issue stays `OPEN` and no mutation
is sent. The open-column control passes, and so does the reconcile evidence case.

## Defect 2: the open-item list, one measurement per command

    python3 spikes/store-defects/measure_open_list.py --board OWNER/NUMBER --mode today \
        --env-file /path/to/.env [--max-points 100] [--cap-seconds 600] --out today.json
    python3 spikes/store-defects/measure_open_list.py --board OWNER/NUMBER --mode capture \
        --env-file /path/to/.env --out capture.json          # costs nothing
    python3 spikes/store-defects/measure_open_list.py --board OWNER/NUMBER --mode prototype \
        --captured capture.json --env-file /path/to/.env [--probe] --out prototype.json

- `today` runs `task list --all` of every open status through a recording proxy, which is set as
  the source's `endpoint`. The proxy refuses every mutation.
- `prototype` pages `ProjectV2.items(query: "is:open")` with the document the binary itself
  sends, captured by `capture`.
- `--probe` (with `--probe-query` and `--probe-only`) asks `totalCount` under candidate filters,
  at 1 point each.

Each mode prints requests, points, items and seconds per phase, from GitHub's own reported
`cost`. Every step stops itself at `--max-points`, and also once any reading says GraphQL
remaining is under 3,000.

## Defect 3 and the quota readings

    python3 spikes/store-defects/duplicate_close.py --repository nickderobertis/onetaskgraph-live-scratch \
        --env-file /path/to/.env [--readback-only SURVIVOR DUPLICATE]
    python3 spikes/store-defects/quota_readings.py --env-file /path/to/.env

`duplicate_close.py` refuses any repository but the scratch one. It closes both issues it
creates.
