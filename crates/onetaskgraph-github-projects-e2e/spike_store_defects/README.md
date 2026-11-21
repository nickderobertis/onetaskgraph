# Spike `spike-store-defects` harness

Throwaway. It is kept on the spike branch so the `store-fixes` node can start from it, and is
never landed. It lives in `onetaskgraph-github-projects-e2e` because the journeys that drive
it are this project's tests. The scripts need Python 3.11 or later (`match`, `enum.StrEnum`)
and nothing outside the standard library.

## Defect 1: status set on an open issue under a terminal card

`tests/e2e/spike_status_set_close.rs` drives the real binary against the loopback board. It
runs with this project's other journeys in its `test` target.

## Defect 2: the open-item list, one measurement per command

    python3 measure_open_list.py --config-dir DIR --source NAME --mode today --out today.json \
        --binary PATH [--env-file PATH] [--max-points 120] [--cap-seconds 600]
    python3 measure_open_list.py --config-dir DIR --source NAME --mode capture --out capture.json \
        --binary PATH                                   # forwards nothing, costs nothing
    python3 measure_open_list.py --config-dir DIR --source NAME --mode prototype --out proto.json \
        --binary PATH --captured capture.json [--probe Q]... [--probe-only]

The source is the one the `onetaskgraph.yaml` in `DIR` configures, and requests go to its
configured `endpoint`. Pointed at the host's `followups` source, that is GitHub. Pointed at a
test's loopback board, nothing leaves the machine, and `tests/e2e/spike_open_list_harness.rs`
drives it that way. The module docstrings state each mode, the limits every step holds itself
to, and the exit codes.

## Defect 3 and the quota readings

    python3 duplicate_close.py --repository nickderobertis/onetaskgraph-live-scratch \
        [--env-file PATH] [--endpoint URL] [--readback-only SURVIVOR DUPLICATE]
    python3 quota_readings.py [--env-file PATH] [--endpoint URL]

`duplicate_close.py` refuses any repository but the scratch one, and closes both issues it
creates.
