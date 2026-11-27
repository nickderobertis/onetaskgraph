#!/usr/bin/env bash
# Stop a pool-maintenance measurement its job cancelled, and remove what it would have saved.
#
#   pool-maintenance/stop.sh SESSION_FILE RESULT_FILE
#
# ci.yml's test-linux starts `just pool-maintenance-budget` as the leader of a session of its
# own and writes that session's id (the leader's pid, which is its process group too) to
# SESSION_FILE. Cancelling the step stops the step's shell and nothing under it, so the
# measurement went on pruning target/ while the cancellation cleanup built into it. This
# signals the whole process group, waits for every member to exit, and only then removes
# RESULT_FILE, so nothing the group writes on its way out survives to be uploaded.
# A measurement that never started leaves no SESSION_FILE, and nothing is signalled.
set -euo pipefail

fail() {
  echo "pool-maintenance/stop.sh: $1" >&2
  echo "pool-maintenance/stop.sh: next: $2" >&2
  exit 1
}

[ "$#" -eq 2 ] || fail "expected SESSION_FILE and RESULT_FILE, received $# argument(s)" \
  "run it as ci.yml's cancellation step does"
session_file="$1"
result_file="$2"
# Seconds a member gets to leave on SIGTERM before the group is sent SIGKILL.
readonly GRACE_SECONDS=20

alive() { pgrep -g "$1" >/dev/null 2>&1; }

if [ -f "$session_file" ]; then
  group="$(tr -d '[:space:]' < "$session_file")"
  case "$group" in
    '' | *[!0-9]* | 0 | 1)
      fail "$session_file holds '$group', which is not a session's process group id" \
        "check the pool-maintenance step's setsid command, which writes it"
      ;;
  esac
  if alive "$group"; then
    kill -TERM -- "-$group" 2>/dev/null || true
    waited=0
    while alive "$group" && [ "$waited" -lt "$GRACE_SECONDS" ]; do
      sleep 1
      waited=$((waited + 1))
    done
    if alive "$group"; then
      kill -KILL -- "-$group" 2>/dev/null || true
      waited=0
      while alive "$group" && [ "$waited" -lt 10 ]; do
        sleep 1
        waited=$((waited + 1))
      done
      alive "$group" && fail "process group $group outlived SIGKILL" \
        "read the runner's process table; nothing may build into target/ while it runs"
    fi
    echo "pool-maintenance/stop.sh: stopped the cancelled measurement (process group $group)." >&2
  fi
fi
rm -f -- "$result_file"
