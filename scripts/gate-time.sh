#!/usr/bin/env bash
# `just gate`'s timing wrapper: runs the gate's whole body (the command given), records its
# start and end to .gate-time.json at the repository root (gitignored), then checks the
# `host-variable` budget onetaskgraph-gate-time against that record.
#
#   bash scripts/gate-time.sh <command> [args...]
#   bash scripts/gate-time.sh --check     # check the last recorded gate again, running nothing
#
# A development machine's timing varies too much to refuse on, so an over reading (exit 1 from
# onebudgetspec) prints a warning naming the budget and the gate still passes; an invalid
# budgets file (2) or a failed measurement (3) fails it. A body that fails fails the gate with
# its own status, and no budget is checked: a failed gate has no wall clock worth reporting.
# CI does not run this: its jobs are held to their own `timeout-minutes`.
set -uo pipefail

[ $# -gt 0 ] || { echo "usage: gate-time.sh <command> [args...] | --check" >&2; exit 2; }

root="$(cd "$(dirname "$0")/.." && pwd)"
record="$root/.gate-time.json"
stamp() { date -u +%Y-%m-%dT%H:%M:%SZ; }

check_budget() {
  local budget=0
  (cd "$root" && bun run --silent onebudgetspec check budgets.yaml --label host-variable) || budget=$?
  case "$budget" in
    0) ;;
    1)
      echo "gate-time: warning: onetaskgraph-gate-time is over its budget (reading above). The gate" \
        "still passes: a development machine's timing varies too much to refuse on." >&2
      ;;
    *)
      echo "gate-time: checking onetaskgraph-gate-time failed (onebudgetspec exit $budget); fix" \
        "budgets.yaml or the record and re-run \`just gate\`." >&2
      exit "$budget"
      ;;
  esac
}

if [ "$1" = --check ] && [ $# -eq 1 ]; then
  check_budget
  exit 0
fi

record_failed() {
  echo "gate-time: cannot $1 $record; make .gate-time.json at the repository root a writable" \
    "file (or remove whatever is there) and run \`just gate\` again." >&2
  exit 2
}

# A record left by an earlier gate never stands in for this one, even if this one is killed,
# so it goes before the body starts and the gate refuses to run when it cannot.
rm -f "$record" 2>/dev/null && [ ! -e "$record" ] || record_failed "remove the earlier record"
started="$(stamp)"
"$@"
status=$?
ended="$(stamp)"
printf '{"schema_version":1,"invocation":"%s-%s","started_at":"%s","ended_at":"%s","exit":%d}\n' \
  "$$" "$started" "$started" "$ended" "$status" >"$record.$$" 2>/dev/null \
  || record_failed "write the record beside"
mv "$record.$$" "$record" 2>/dev/null || { rm -f "$record.$$"; record_failed "replace"; }

if [ "$status" -ne 0 ]; then
  echo "gate-time: the gate failed (exit $status), so its wall clock is not checked; fix the" \
    "failure reported above and run \`just gate\` again." >&2
  exit "$status"
fi

check_budget
