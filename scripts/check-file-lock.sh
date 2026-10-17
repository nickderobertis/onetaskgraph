#!/usr/bin/env bash
# Watch scripts/file-lock.sh hold: two processes taking one lock never overlap, a holder killed
# while a process it started is still running keeps the lock until that process ends, a lock
# file left behind is no lock, and a lock that cannot be taken fails by name.
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fail() {
  echo "check-file-lock: $1" >&2
  echo "check-file-lock: next: fix with_file_lock in scripts/file-lock.sh, then rerun" >&2
  exit 1
}
# shellcheck source=scripts/file-lock.sh
if [ ! -r "$ROOT/scripts/file-lock.sh" ] || ! source "$ROOT/scripts/file-lock.sh"; then
  echo "check-file-lock: could not load $ROOT/scripts/file-lock.sh" >&2
  echo "check-file-lock: next: restore it with 'git checkout -- scripts/file-lock.sh', then rerun" >&2
  exit 1
fi

scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
lock="$scratch/lock"
record="$scratch/record"

# bounded <case> <command...>: the command, failed by name if it has not ended in a minute.
bounded() {
  local case_name="$1" waiter waited=0
  shift
  ( trap - EXIT; "$@" ) &
  waiter=$!
  while kill -0 "$waiter" 2>/dev/null; do
    if [ "$waited" -ge 60 ]; then
      kill "$waiter" 2>/dev/null || true
      fail "$case_name: still waiting after 60 seconds"
    fi
    sleep 1
    waited=$((waited + 1))
  done
  wait "$waiter" || fail "$case_name: the command failed"
}

# 1. Two holders at once never overlap, whichever takes the lock first.
section='echo "start $1" >>"$2"; sleep 2; echo "end $1" >>"$2"'
( trap - EXIT; with_file_lock "$lock" bash -c "$section" _ one "$record" ) &
first=$!
( trap - EXIT; with_file_lock "$lock" bash -c "$section" _ two "$record" ) &
second=$!
wait "$first" && wait "$second" || fail "a holder of the lock failed"
case "$(tr '\n' ' ' <"$record")" in
  "start one end one start two end two " | "start two end two start one end one ") ;;
  *) fail "two holders overlapped: $(tr '\n' ' ' <"$record")" ;;
esac

# 2. The program's status is the caller's, and the lock is free again after a failure.
if with_file_lock "$lock" false; then fail "a failing program read as a success"; fi
bounded "the lock after a failing holder" with_file_lock "$lock" true

# 3. A holder killed while a process it started still runs: that process keeps the lock, and
#    the next holder starts only once it has ended.
: >"$record"
( trap - EXIT; with_file_lock "$lock" bash -c \
  '( sleep 3; echo "child end" >>"$2" ) & echo "$!" >"$1.child"; echo "$$" >"$1.holder"; wait' \
  _ "$scratch/build" "$record" ) 2>"$scratch/killed-holder.err" &
holder_shell=$!
until [ -s "$scratch/build.holder" ] && [ -s "$scratch/build.child" ]; do sleep 0.1; done
kill "$(cat "$scratch/build.holder")"
wait "$holder_shell" 2>/dev/null || true
bounded "the lock a killed holder's child keeps" with_file_lock "$lock" bash -c 'echo "next start" >>"$1"' _ "$record"
[ "$(tr '\n' ' ' <"$record")" = "child end next start " ] \
  || fail "the next holder started before the killed holder's child ended: $(tr '\n' ' ' <"$record")"

# 4. A lock file left behind by holders long gone is no lock.
[ -e "$lock" ] || fail "the lock file the cases above held is not there to be left behind"
bounded "a lock file left behind" with_file_lock "$lock" true

# 5. A lock that cannot be opened fails, saying why, rather than waiting.
mkdir "$scratch/a-directory"
if output="$(with_file_lock "$scratch/a-directory" true 2>&1)"; then
  fail "a lock that could not be opened read as taken"
fi
grep -qF "file-lock: could not open $scratch/a-directory" <<<"$output" \
  || fail "a lock that could not be opened failed without saying so: $output"
