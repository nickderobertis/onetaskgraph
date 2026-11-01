#!/usr/bin/env bash
# Hold scripts/check-npm-publish.sh to never writing the checkout it runs from.
#
# That check needs a TypeScript SDK `dist` to pack, and once wrote a stand-in into the
# checkout's sdks/typescript when there was none, removing it on exit. distribution-test runs
# it beside github-workflows:cd-journeys, whose publish build writes the real `dist` into the
# same directory and reads it back — so a build that landed between the two was deleted under
# the journey, which failed ENOENT on sdks/typescript/dist/index.js on the hosted Linux job.
# Whether that happens is a matter of timing, so this does not wait for the race: it runs the
# check in a scratch clone holding no `dist` and refuses any change to the clone's
# sdks/typescript at all, which the stand-in's create and removal always make.
set -euo pipefail

fatal() {
  echo "check-npm-publish-leaves-checkout: $1" >&2
  echo "check-npm-publish-leaves-checkout: next: $2" >&2
  exit 1
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)" || fatal \
  "could not resolve this repository's root from ${BASH_SOURCE[0]}" \
  "run it from a checkout of this repository, as 'just script-check' does"
readonly ROOT

PYTHON="$(command -v python3)" || fatal "python3 is not on PATH" "run 'just bootstrap', then rerun"
# Absolute, so a relative PATH entry still names it once this is inside the clone (AGENTS.md).
case "$PYTHON" in
  /*) ;;
  *) PYTHON="$(cd "$(dirname "$PYTHON")" && pwd)/$(basename "$PYTHON")" ;;
esac

scratch="$(mktemp -d)" || fatal "could not create a scratch directory" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
trap 'rm -rf "$scratch"' EXIT

# The path below is built from $ROOT at run time, so ShellCheck cannot follow it.
# shellcheck source=scripts/scratch-clone.sh
if [ ! -r "$ROOT/scripts/scratch-clone.sh" ] || ! source "$ROOT/scripts/scratch-clone.sh"; then
  fatal "could not load $ROOT/scripts/scratch-clone.sh, which strips the git environment" \
    "restore that file with 'git checkout -- scripts/scratch-clone.sh' and rerun"
fi

# A clone, because the check copies what its checkout tracks, overlaid with the working
# tree's own files so what is under test is the scripts as they are now.
scratch_clone "$ROOT" "$scratch/repo" || fatal "could not clone $ROOT" "fix what is reported above, then rerun"
copy_tracked_files "$ROOT" "$scratch/repo" scripts npm sdks/typescript || fatal \
  "could not copy the working tree's scripts, npm and sdks/typescript into the clone" \
  "confirm 'git ls-files' answers in $ROOT and 'df -h' for free space, then rerun"
sdk="$scratch/repo/sdks/typescript"
[ ! -e "$sdk/dist" ] || fatal "the clone already holds $sdk/dist, so the case below proves nothing" \
  "check that sdks/typescript/dist is not tracked ('git ls-files sdks/typescript/dist'), then rerun"

snapshot() {
  "$PYTHON" - "$sdk" <<'PY'
import os
import sys

root = sys.argv[1]
for directory, names, files in os.walk(root):
    names.sort()
    for name in [directory, *(os.path.join(directory, f) for f in sorted(files))]:
        info = os.stat(name)
        print(os.path.relpath(name, root).replace(os.sep, "/"), info.st_size, info.st_mtime_ns)
PY
}

snapshot > "$scratch/before" || fatal "could not read $sdk" "check \$TMPDIR permissions, then rerun"
if ! (cd "$scratch/repo" && bash scripts/check-npm-publish.sh) > "$scratch/check.log" 2>&1; then
  cat "$scratch/check.log" >&2
  fatal "scripts/check-npm-publish.sh failed in the clone, so whether it leaves its checkout alone is unknown" \
    "fix what it reports above, then rerun"
fi
snapshot > "$scratch/after" || fatal "could not read $sdk again" "check \$TMPDIR permissions, then rerun"

if ! diff -u "$scratch/before" "$scratch/after" > "$scratch/diff"; then
  sed 's/^/check-npm-publish-leaves-checkout:   /' "$scratch/diff" >&2
  fatal "scripts/check-npm-publish.sh changed its checkout's sdks/typescript (above: path, size, mtime), which github-workflows:cd-journeys builds and reads at the same moment" \
    "write the SDK's stand-in dist into the copy that check publishes from, never into the checkout"
fi
