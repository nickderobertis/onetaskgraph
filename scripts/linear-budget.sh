#!/usr/bin/env bash
# Measure one Linear request budget: the command each budget of
# crates/onetaskgraph-linear/budgets.yaml names.
#
# Each budget is one journey of crates/onetaskgraph/tests/e2e/linear_budget.rs, named after
# it: `linear-requests-per-status-write-cold` is `measure_linear_requests_per_status_write_cold`.
# That journey measures against the loopback Linear workspace, holds the figure to the
# budget's threshold, and writes `{"value": <requests>}` to the file `ONEBUDGETSPEC_RESULT`
# names, which `onebudgetspec check` set before it ran this. Nothing here reaches Linear.
#
# The build is the one onetaskgraph:build makes — the same package, features and lock — so
# this runs the journey rather than relinking the binary another target may be spawning.
#
# Usage: scripts/linear-budget.sh <budget-id>
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

budget="${1:-}"
case "$budget" in
  linear-requests-per-status-write-cold | linear-requests-per-status-write-warm | \
    linear-requests-per-settlement-update-cold | linear-requests-per-settlement-update-warm) ;;
  *)
    echo "linear-budget: ${budget:-no budget} is not one of the Linear request budgets" >&2
    echo "linear-budget: next: name one of crates/onetaskgraph-linear/budgets.yaml's ids" >&2
    exit 2
    ;;
esac
if [ -z "${ONEBUDGETSPEC_RESULT:-}" ]; then
  echo "linear-budget: ONEBUDGETSPEC_RESULT is not set, so there is nowhere to report $budget" >&2
  echo "linear-budget: next: run it through 'bash scripts/check-linear-budgets.sh'" >&2
  exit 2
fi

journey="linear_budget::measure_$(printf '%s' "$budget" | tr '-' '_')"
if ! output="$(cargo test --quiet -p onetaskgraph --all-features --locked --test e2e -- "$journey" --exact 2>&1)"; then
  printf '%s\n' "$output" >&2
  echo "linear-budget: $journey failed — see above" >&2
  echo "linear-budget: next: run 'cargo test -p onetaskgraph --all-features --test e2e -- $journey --exact' and fix what it reports: an assertion over the budget names the request a status write now sends that it did not" >&2
  exit 1
fi
if [ ! -s "$ONEBUDGETSPEC_RESULT" ]; then
  printf '%s\n' "$output" >&2
  echo "linear-budget: $journey ran and reported nothing; it is no longer the measurement of $budget" >&2
  echo "linear-budget: next: restore that journey's call to report(\"$budget\", …) in crates/onetaskgraph/tests/e2e/linear_budget.rs, which writes the figure to ONEBUDGETSPEC_RESULT" >&2
  exit 1
fi
