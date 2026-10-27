#!/usr/bin/env bash
# Keep ordinary host budgets strict; only measured host-variable over readings warn.
# Direct onebudgetspec checks retain their strict verdict, including any CI invocation.
set -euo pipefail

budgets=${1:?usage: budgets-host.sh <budgets.yaml>}
strict_status=0
variable_status=0

bun run onebudgetspec check "$budgets" --label host --exclude-label host-variable \
  || strict_status=$?
bun run onebudgetspec check "$budgets" --label host-variable \
  || variable_status=$?

# onebudgetspec reserves 1 for over; a failed measurement or invalid input is an error.
if (( strict_status > 1 )); then
  exit "$strict_status"
fi
if (( variable_status > 1 )); then
  exit "$variable_status"
fi
if (( strict_status != 0 )); then
  exit "$strict_status"
fi
if (( variable_status == 1 )); then
  echo "budgets-host: warning: a host-variable budget is over; see the reading above. CI remains strict." >&2
fi
