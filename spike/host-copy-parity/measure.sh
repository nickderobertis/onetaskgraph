#!/usr/bin/env bash
# spike-host-copy-parity: one measurement of one variant of the plan copy on the loopback board.
#
#   spike/host-copy-parity/measure.sh <variant> [items] [--json]
#   SPIKE_FIXES=<fix,fix|all> spike/host-copy-parity/measure.sh <variant> [items] [--json]
#
# <variant> is `scenario`, `host`, or `+`-joined axes: private, cache, screened, host-shape,
# instant (see crates/onetaskgraph-github-projects-e2e/tests/e2e/plan_copy/parity.rs).
# [items] is the board's other items, 2000 by default. Prints, for the first copy, the re-copy
# after one task changed and the unchanged re-copy: points at the scenario's prices, writes,
# simulated seconds and the request tally; --json prints the whole measurement instead.
# SPIKE_FIXES switches on the prototyped fixes in the binary (see spike_fix in
# crates/onetaskgraph-core/src/engine/copy.rs); unset, the binary behaves as main does.
set -euo pipefail
if [ $# -lt 1 ]; then
  echo "usage: $0 <variant> [items] [--json]" >&2
  exit 2
fi
variant=$1
items=${2:-2000}
format=${3:-}
root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"
log=$(mktemp)
trap 'rm -f "$log"' EXIT
if ! cargo test --quiet -p onetaskgraph --all-features --locked --no-run >"$log" 2>&1; then
  cat "$log" >&2
  echo "building target/debug/onetaskgraph failed; fix the build above and run this again" >&2
  exit 1
fi
if ! SPIKE_PARITY_VARIANT=$variant SPIKE_PARITY_ITEMS=$items cargo test --quiet \
  -p onetaskgraph-github-projects-e2e --all-features --locked --test e2e -- \
  plan_copy::parity::one_variant --exact --nocapture >"$log" 2>&1; then
  cat "$log" >&2
  echo "the measurement of $variant at $items items failed; its output is above" >&2
  exit 1
fi
line=$(grep '^SPIKE-PARITY ' "$log" | tr -d '\r' | sed 's/^SPIKE-PARITY //')
if [ -z "$line" ]; then
  cat "$log" >&2
  echo "the measurement printed nothing; is plan_copy::parity::one_variant in the suite?" >&2
  exit 1
fi
if [ "$format" = "--json" ]; then
  printf '%s\n' "$line"
  exit 0
fi
printf '%s\n' "$line" | python3 -c '
import json, sys
measured = json.load(sys.stdin)
print("variant %s on a board of %s items, fixes: %s" % (
    measured["variant"], measured["items"], measured.get("fixes") or "none"))
for scenario in measured["scenarios"]:
    print("  %s: points=%s writes=%s simulated_seconds=%s requests=%s" % (
        scenario["scenario"], scenario["points"], scenario["writes"],
        scenario["simulated_seconds"], scenario["requests"]))
    for operation, sent in scenario["requests_by_operation"].items():
        print("    %2d x %s  (%s points)" % (sent, operation, scenario["points_by_operation"][operation]))
'
