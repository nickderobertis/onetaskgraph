#!/usr/bin/env bash
# Check the Linear request budgets in crates/onetaskgraph-linear/budgets.yaml.
#
# A step of the Linear crate's own `test` target, so it runs whenever affected selection
# selects that crate. onebudgetspec is this workspace's own pinned devDependency, from the
# locked install `just bootstrap` makes.
#
# Quiet when every budget is within its threshold; otherwise onebudgetspec's own report,
# which names each budget's actual figure, its threshold and its headroom.
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if [ ! -x "node_modules/.bin/onebudgetspec" ]; then
  echo "check-linear-budgets: onebudgetspec is not installed in this worktree; it is a pinned devDependency of package.json" >&2
  echo "check-linear-budgets: next: run 'just bootstrap', which installs the locked Node toolchain, then re-run" >&2
  exit 1
fi

if ! report="$(node_modules/.bin/onebudgetspec check crates/onetaskgraph-linear/budgets.yaml 2>&1)"; then
  printf '%s\n' "$report" >&2
  echo "check-linear-budgets: a Linear request budget is over its threshold or could not be measured — see above" >&2
  echo "check-linear-budgets: next: find the request a status write sends that it did not before, in the journey the budget names" >&2
  exit 1
fi
