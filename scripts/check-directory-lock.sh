#!/usr/bin/env bash
# Watch scripts/directory-lock.sh hold: two processes taking one lock never overlap, and a lock
# whose holder has died, or that names none and is old, is taken over rather than waited on.
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fail() {
  echo "check-directory-lock: $1" >&2
  echo "check-directory-lock: next: fix with_directory_lock in scripts/directory-lock.sh, then rerun" >&2
  exit 1
}
# shellcheck source=scripts/directory-lock.sh
if [ ! -r "$ROOT/scripts/directory-lock.sh" ] || ! source "$ROOT/scripts/directory-lock.sh"; then
  echo "check-directory-lock: could not load $ROOT/scripts/directory-lock.sh" >&2
  echo "check-directory-lock: next: restore it with 'git checkout -- scripts/directory-lock.sh', then rerun" >&2
  exit 1
fi

scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
lock="$scratch/lock"

# bounded <case> <command...>: the command, failed by name if it is still waiting on the lock
# after a minute — a lock that is never taken over hangs, and a hang must read as a failure.
bounded() {
  local case_name="$1" waiter waited=0
  shift
  ( trap - EXIT; "$@" ) &
  waiter=$!
  while kill -0 "$waiter" 2>/dev/null; do
    if [ "$waited" -ge 60 ]; then
      kill "$waiter" 2>/dev/null || true
      fail "$case_name: still waiting on the lock after 60 seconds"
    fi
    sleep 1
    waited=$((waited + 1))
  done
  wait "$waiter" || fail "$case_name: the command holding the lock failed"
}

# 1. Two holders at once: each writes where it starts and ends, and the record is never
#    interleaved, whichever takes the lock first.
section() {
  echo "start $1" >>"$scratch/record"
  sleep 2
  echo "end $1" >>"$scratch/record"
}
bash -c "source '$ROOT/scripts/directory-lock.sh'; $(declare -f section); scratch='$scratch'; with_directory_lock '$lock' section one" &
first=$!
bash -c "source '$ROOT/scripts/directory-lock.sh'; $(declare -f section); scratch='$scratch'; with_directory_lock '$lock' section two" &
second=$!
wait "$first" && wait "$second" || fail "a holder of the lock failed"
record="$(tr '\n' ' ' <"$scratch/record")"
case "$record" in
  "start one end one start two end two " | "start two end two start one end one ") ;;
  *) fail "two holders overlapped: $record" ;;
esac
[ ! -e "$lock" ] || fail "the lock outlived its last holder"

# 2. A command's status is the caller's, and a failing command releases the lock too.
if with_directory_lock "$lock" false; then fail "a failing command read as a success"; fi
[ ! -e "$lock" ] || fail "a failing command left the lock held"

# 3. A lock naming a holder that has died is taken over.
# A process that has already ended: its id names no one.
dead="$(bash -c 'echo "$$"')"
mkdir "$lock"
echo "$dead" >"$lock/pid"
bounded "a lock left by a dead holder" with_directory_lock "$lock" true

# 4. A lock naming no holder is taken over once it is old.
mkdir "$lock"
# `touch -t` reads local time, so the stamp is written in it.
touch -t "$(date -v-2M +%Y%m%d%H%M.%S 2>/dev/null || date -d '2 minutes ago' +%Y%m%d%H%M.%S)" "$lock"
bounded "an old lock naming no holder" with_directory_lock "$lock" true
[ ! -e "$lock" ] || fail "an old lock naming no holder was left behind once taken over"
