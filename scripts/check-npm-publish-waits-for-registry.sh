#!/usr/bin/env bash
# Hold scripts/check-npm-publish.sh's exit cleanup to waiting for its registry before it
# removes the scratch tree that registry runs in.
#
# On test-windows the registry, signalled but not yet exited, held its working directory
# open, so the cleanup's rm failed 'Device or resource busy' and, under set -e, turned a
# passing check red. Whether that happens is a matter of timing, so this does not wait for
# the race: it runs the real check in a scratch clone whose registry takes two seconds to
# exit once signalled and records, as it exits, whether its checkout still stands. A cleanup
# that waits returns after that record is written and it reads `present`; one that does not
# returns before it exists.
set -euo pipefail

fatal() {
  echo "check-npm-publish-waits-for-registry: $1" >&2
  echo "check-npm-publish-waits-for-registry: next: $2" >&2
  exit 1
}

# The delayed exit is a Python SIGTERM handler, and MSYS bash's kill ends a native Windows
# process without running one, so there the case cannot be staged; it is the order of two
# steps in one shell function, which the Linux and macOS lanes hold for every platform.
case "${OS:-}${OSTYPE:-}" in
  *Windows_NT* | *msys* | *cygwin* | *win32*)
    echo "check-npm-publish-waits-for-registry: skipped on Windows (a signalled native process runs no handler there, so the delayed exit cannot be staged); the Linux and macOS lanes run it" >&2
    exit 0
    ;;
esac

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)" || fatal \
  "could not resolve this repository's root from ${BASH_SOURCE[0]}" \
  "run it from a checkout of this repository, as 'just script-check' does"
readonly ROOT

scratch="$(mktemp -d)" || fatal "could not create a scratch directory" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
trap 'rm -rf "$scratch"' EXIT

# The path below is built from $ROOT at run time, so ShellCheck cannot follow it.
# shellcheck source=scripts/scratch-clone.sh
if [ ! -r "$ROOT/scripts/scratch-clone.sh" ] || ! source "$ROOT/scripts/scratch-clone.sh"; then
  fatal "could not load $ROOT/scripts/scratch-clone.sh, which strips the git environment" \
    "restore that file with 'git checkout -- scripts/scratch-clone.sh' and rerun"
fi

# Overlaid with the working tree's own files, so what is under test is the cleanup as it is now.
scratch_clone "$ROOT" "$scratch/repo" || fatal "could not clone $ROOT" "fix what is reported above, then rerun"
copy_tracked_files "$ROOT" "$scratch/repo" scripts npm sdks/typescript || fatal \
  "could not copy the working tree's scripts, npm and sdks/typescript into the clone" \
  "confirm 'git ls-files' answers in $ROOT and 'df -h' for free space, then rerun"

# Prepended rather than wrapped: the check copies only tracked files into its own scratch
# tree, so a second launcher file would never reach the registry it starts. The checkout is
# the launcher's grandparent, `$scratch/checkout` in the check's own terms.
launcher="$scratch/repo/scripts/loopback-npm-registry.py"
marker="$scratch/registry-exit"
{
  cat <<'PY'
import os as _os
import signal as _signal
import time as _time


def _exit_slowly(_signum, _frame):
    _time.sleep(2)
    checkout = _os.path.dirname(_os.path.dirname(_os.path.abspath(__file__)))
    with open(_os.environ["NPM_REGISTRY_EXIT_MARKER"], "w", encoding="utf-8") as handle:
        handle.write("present" if _os.path.isdir(checkout) else "absent")
    _os._exit(0)


_signal.signal(_signal.SIGTERM, _exit_slowly)
PY
  cat "$ROOT/scripts/loopback-npm-registry.py"
} > "$launcher" || fatal "could not stage the slow-exiting registry at $launcher" \
  "check \$TMPDIR permissions and 'df -h' for free space, then rerun"

if ! (cd "$scratch/repo" && NPM_REGISTRY_EXIT_MARKER="$marker" bash scripts/check-npm-publish.sh) \
  > "$scratch/check.log" 2>&1; then
  cat "$scratch/check.log" >&2
  fatal "scripts/check-npm-publish.sh failed in the clone, so whether its cleanup waits for its registry is unknown" \
    "fix what it reports above, then rerun"
fi

[ -f "$marker" ] || fatal \
  "scripts/check-npm-publish.sh returned before the registry it signalled had exited, so its rm raced a process still holding the scratch checkout — the race that failed test-windows 'Device or resource busy'" \
  "in that script's cleanup, wait for \$REGISTRY_PID after killing it and before removing \$scratch"
[ "$(cat "$marker")" = present ] || fatal \
  "the registry found its scratch checkout already removed while it was still exiting (it recorded '$(cat "$marker")')" \
  "in scripts/check-npm-publish.sh's cleanup, remove \$scratch only after waiting for \$REGISTRY_PID"
