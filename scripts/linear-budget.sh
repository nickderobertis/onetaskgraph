#!/usr/bin/env bash
# Measure one Linear request budget: the command each budget of
# crates/onetaskgraph-linear/budgets.yaml names.
#
# Each budget is one journey of crates/onetaskgraph-linear-e2e/tests/e2e/linear_budget.rs, the
# `e2e` test target of the Linear plugin's own e2e suite, named after it: `linear-requests-per-status-write-cold` is `measure_linear_requests_per_status_write_cold`.
# That journey measures against the loopback Linear workspace, holds the figure to the
# budget's threshold, and writes `{"value": <requests>}` to the file `ONEBUDGETSPEC_RESULT`
# names, which `onebudgetspec check` set before it ran this. Nothing here reaches Linear.
#
# The binary the journey drives is the one onetaskgraph:build makes, which the budget-check
# target depends on; this builds only the journey's own package, which never links the
# onetaskgraph package, so it cannot relink the binary another target may be spawning.
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
# llmlint: ignore[work_goes_through_command_surface] This IS the command surface's step: onebudgetspec runs it as a budget's measurement from the Linear crate's own Nx `budget-check` target, which depends on onetaskgraph:build; it runs one named journey of the Linear plugin's e2e suite against the binary that build made, which no recipe exposes and which a recipe here would only wrap.
if ! output="$(cargo test --quiet -p onetaskgraph-linear-e2e --all-features --locked --test e2e -- "$journey" --exact 2>&1)"; then
  printf '%s\n' "$output" >&2
  echo "linear-budget: $journey failed — see above" >&2
  echo "linear-budget: next: run 'cargo test -p onetaskgraph-linear-e2e --all-features --test e2e -- $journey --exact' and fix what it reports: an assertion over the budget names the request a status write now sends that it did not" >&2
  exit 1
fi
if [ ! -s "$ONEBUDGETSPEC_RESULT" ]; then
  printf '%s\n' "$output" >&2
  echo "linear-budget: $journey ran and reported nothing; it is no longer the measurement of $budget" >&2
  echo "linear-budget: next: restore that journey's call to report(\"$budget\", …) in crates/onetaskgraph-linear-e2e/tests/e2e/linear_budget.rs, which writes the figure to ONEBUDGETSPEC_RESULT" >&2
  exit 1
fi
