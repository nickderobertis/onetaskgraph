# spike-gaps: the board-copy-gaps feasibility spike

Preserved on `nick/board-copy-gaps-2026-10-24/spike-feasibility`; nothing here lands. The report
is `board-copy-gaps-2026-10-24-spike-feasibility-report` in `authoring:board-copy-gaps-2026-10-24`.

It extends the timing spike's harness in `spike/harness/`, which stays exactly as that spike left
it (`bc49f888`): the recording proxy, the 40% quota rule and the workload generator are imported
through `gaps/harness.py`, never copied.

## One measurement

    spike/gaps.sh <scenario> [--build stock|head|closure]

`gaps/cli.py` documents every scenario, the environment and the exit statuses. The live probes
(`probe-alias-pricing`, `probe-placement`) write only to the scratch repository and board that
`crates/onetaskgraph-github-live/src/lib.rs` declares, through `gaps/guard.py`, and pause under 40%
of the GraphQL allowance. The offline cases (`reach-*`, `screen-union`, `proxy-refusal`) need no
credential. `closure` is built by `prototype/build.sh` from `prototype/reach-closure.patch`.

## The analysis and the report

    PYTHONPATH=spike/gaps:spike/harness python3 -m gaps.report --answers answers.json \
        [--ruling ruling.md] [--lint llmlint-run.json]

writes `logs/gaps-summary.json` from the committed logs under `logs/` and the plan's own budget
records, and the `spike-report` answers.

## Checks

`nx run spike-gaps:test` runs every test: the probes end to end against a loopback stand-in for
GitHub through the real proxy and guard, the offline cases against the real CLI and onevcs, the
drift gates (plugin documents, test board, boundary schema, baseline runs, plan budgets) and the
committed summary's freshness. `lint`, `format-check` and `typecheck` are the project's other
targets.
