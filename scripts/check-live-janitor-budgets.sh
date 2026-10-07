#!/usr/bin/env bash
# Check the janitor request budgets in crates/onetaskgraph-live-janitor/budgets.yaml.
#
# The janitor crate's `budget-check` target, run by `check` and `just test` whenever affected selection
# selects that crate. onebudgetspec is this workspace's own pinned devDependency, from the
# locked install `just bootstrap` makes.
#
# Quiet when every budget is within its threshold; otherwise onebudgetspec's own report,
# which names each budget's actual figure, its threshold and its headroom.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)" && cd "$ROOT" || {
  echo "check-live-janitor-budgets: cannot enter the repository root from ${BASH_SOURCE[0]}" >&2
  echo "check-live-janitor-budgets: next: run it as 'bash scripts/check-live-janitor-budgets.sh' from a checkout" >&2
  exit 1
}
readonly ROOT

# onebudgetspec publishes no build for win32-x64: its launcher there stops with "no build for
# win32-x64", which would read as a budget having failed. So the check is skipped there with a
# notice, by the convention scripts/check-affected-selection.sh follows for its own platform
# reason. The Linux and macOS lanes run every budget on every change that selects this crate.
case "${OS:-}${OSTYPE:-}" in
  *Windows_NT* | *msys* | *cygwin* | *win32*)
    echo "check-live-janitor-budgets: skipped on Windows (onebudgetspec publishes no win32-x64 build); the Linux and macOS lanes gate the janitor request budgets" >&2
    exit 0
    ;;
esac

if [ ! -x "node_modules/.bin/onebudgetspec" ]; then
  echo "check-live-janitor-budgets: onebudgetspec is not installed in this worktree; it is a pinned devDependency of package.json" >&2
  echo "check-live-janitor-budgets: next: run 'just bootstrap', which installs the locked Node toolchain, then re-run" >&2
  exit 1
fi

if ! report="$(node_modules/.bin/onebudgetspec check crates/onetaskgraph-live-janitor/budgets.yaml 2>&1)"; then
  printf '%s\n' "$report" >&2
  echo "check-live-janitor-budgets: a janitor request budget is over its threshold or could not be measured — see above" >&2
  echo "check-live-janitor-budgets: next: inspect the request count from the offline janitor journey the budget names" >&2
  exit 1
fi
