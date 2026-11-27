#!/usr/bin/env bash
# Hold scripts/check-npm-publish.sh's cleanup to waiting for its registry before it removes
# the tree that registry runs in.
#
# On Windows a process that has not exited still holds its working directory, and removing
# the tree under one failed a passing check `Device or resource busy`. Waiting for that race
# would prove nothing off Windows, so this makes the order observable everywhere instead: the
# real check runs in a scratch clone whose registry launcher is the real registry behind a
# SIGTERM handler that takes two seconds to shut down and then records whether its working
# directory still exists. A cleanup that waits leaves it `present`; one that removes the tree
# first leaves it `removed`. Three cases: the cleanup as written on an ordinary exit and on a
# failure exit, and the cleanup without its wait, which has to read `removed` or this check
# could not tell the two apart.
set -euo pipefail

fatal() {
  echo "check-npm-publish-cleanup: $1" >&2
  echo "check-npm-publish-cleanup: next: $2" >&2
  exit 1
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)" || fatal \
  "could not resolve this repository's root from ${BASH_SOURCE[0]}" \
  "run it from a checkout of this repository, as 'just script-check' does"
readonly ROOT

scratch="$(mktemp -d)" || fatal "could not create a scratch directory" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
trap 'rm -rf "$scratch" || echo "check-npm-publish-cleanup: could not remove $scratch; next: delete it by hand" >&2' EXIT

# The path below is built from $ROOT at run time, so ShellCheck cannot follow it.
# shellcheck source=scripts/scratch-clone.sh
if [ ! -r "$ROOT/scripts/scratch-clone.sh" ] || ! source "$ROOT/scripts/scratch-clone.sh"; then
  fatal "could not load $ROOT/scripts/scratch-clone.sh, which strips the git environment" \
    "restore that file with 'git checkout -- scripts/scratch-clone.sh' and rerun"
fi

# A clone overlaid with the working tree's files, so what runs is the check as it is now.
# check-npm-publish.sh copies only tracked files into its own tree, so the real registry the
# stand-in runs sits outside the clone, named by NPM_CLEANUP_REGISTRY.
scratch_clone "$ROOT" "$scratch/repo" || fatal "could not clone $ROOT" "fix what is reported above, then rerun"
copy_tracked_files "$ROOT" "$scratch/repo" scripts npm sdks/typescript || fatal \
  "could not copy the working tree's scripts, npm and sdks/typescript into the clone" \
  "confirm 'git ls-files' answers in $ROOT and 'df -h' for free space, then rerun"
launcher="$scratch/repo/scripts/loopback-npm-registry.py"
cp "$launcher" "$scratch/registry.py" || fatal "could not keep the real registry" "check \$TMPDIR, then rerun"
cat > "$launcher" <<'PY' || fatal "could not write the stand-in registry launcher $launcher" "check \$TMPDIR permissions and 'df -h', then rerun"
"""The real registry, behind a shutdown that is slow on purpose and says what it found."""
import os
import runpy
import signal
import sys
import time

record = os.environ["NPM_CLEANUP_RECORD"]
registry = os.environ["NPM_CLEANUP_REGISTRY"]
here = os.getcwd()
with open(record, "w", encoding="utf-8") as handle:
    handle.write("started\n")


def stop(_signum, _frame):
    time.sleep(2)
    with open(record, "w", encoding="utf-8") as handle:
        handle.write("present\n" if os.path.isdir(here) else "removed\n")
    os._exit(0)


signal.signal(signal.SIGTERM, stop)
sys.argv[0] = registry
runpy.run_path(registry, run_name="__main__")
PY
# An npm that refuses everything: check-npm-publish.sh only asks whether npm exists before
# its registry starts, so the first `npm pack` after it is where the failure exit begins.
{ mkdir -p "$scratch/failing-npm" &&
  printf '#!/usr/bin/env bash\nexit 1\n' > "$scratch/failing-npm/npm" &&
  chmod +x "$scratch/failing-npm/npm"; } || fatal \
  "could not create the refusing npm in $scratch/failing-npm" "check \$TMPDIR permissions and 'df -h', then rerun"

# Run the clone's check: $1 names the case, $2 is `ordinary` or `failure`, $3 the record the
# registry must leave.
drive() {
  local case_name="$1" exit_path="$2" expected="$3" record="$scratch/$1.record" log="$scratch/$1.log"
  local path="$PATH" status=0
  [ "$exit_path" = ordinary ] || path="$scratch/failing-npm:$PATH"
  (cd "$scratch/repo" && PATH="$path" NPM_CLEANUP_RECORD="$record" \
    NPM_CLEANUP_REGISTRY="$scratch/registry.py" bash scripts/check-npm-publish.sh) > "$log" 2>&1 || status=$?
  if [ "$exit_path" = ordinary ] && [ "$status" -ne 0 ]; then
    cat "$log" >&2
    fatal "$case_name: scripts/check-npm-publish.sh failed (exit $status), so its ordinary cleanup was not reached" \
      "fix what it reports above, then rerun"
  fi
  if [ "$exit_path" = failure ] && ! grep -q 'npm pack refused' "$log"; then
    cat "$log" >&2
    fatal "$case_name: scripts/check-npm-publish.sh did not fail at its first npm pack (exit $status), so the failure cleanup was not reached after its registry started" \
      "fix what it reports above, then rerun"
  fi
  [ -s "$record" ] || fatal "$case_name: the registry never started, so the cleanup's order is unknown" \
    "read $log's diagnostic by rerunning with the case above, then rerun"
  # A cleanup that did not wait has returned while the registry is still shutting down, so its
  # answer arrives up to its two seconds later.
  local found tries=0
  found="$(tr -d '\r' < "$record")" || fatal "$case_name: could not read $record" "check \$TMPDIR permissions, then rerun"
  while [ "$found" = started ] && [ "$tries" -lt 50 ]; do
    sleep 0.2
    tries=$((tries + 1))
    found="$(tr -d '\r' < "$record")" || fatal "$case_name: could not read $record again" "check \$TMPDIR permissions, then rerun"
  done
  [ "$found" = "$expected" ] || fatal \
    "$case_name: the registry recorded '$found' where '$expected' belongs (present: its working directory outlived it; removed: the cleanup took the tree before it exited; started: it was never sent SIGTERM)" \
    "make cleanup() in scripts/check-npm-publish.sh kill the registry, wait for it, leave the tree and only then remove it"
}

drive ordinary-exit ordinary present
drive failure-exit failure present

# The cleanup without its wait, substituted in the clone's copy of the real check.
check="$scratch/repo/scripts/check-npm-publish.sh"
grep -q '^    wait "\$REGISTRY_PID" 2>/dev/null || true$' "$check" || fatal \
  "cleanup() in scripts/check-npm-publish.sh no longer waits with the line this check removes, so its control case cannot be built" \
  "update the line this check deletes to the cleanup's own wait, then rerun"
sed -i.bak '/^    wait "\$REGISTRY_PID" 2>\/dev\/null || true$/d' "$check" || fatal \
  "could not remove the wait from the clone's $check for the control case" "check \$TMPDIR permissions, then rerun"
drive cleanup-without-wait failure removed
