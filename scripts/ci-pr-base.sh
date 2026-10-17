#!/usr/bin/env bash
# The commit a pull request's affected selection compares against: the merge base of the
# commit under test with the pull request's base. Explicit, because an implicit base is how
# affected selection quietly starts comparing against the wrong commit (AGENTS.md).
#
#   scripts/ci-pr-base.sh <base-branch> <base-sha>
#
# Prints the merge base, and writes it to $GITHUB_OUTPUT as `base=` when Actions sets that.
set -euo pipefail

if [ "$#" -ne 2 ] || [ -z "$1" ] || [ -z "$2" ]; then
  echo "usage: scripts/ci-pr-base.sh <base-branch> <base-sha>" >&2
  echo "ci-pr-base: next: pass github.base_ref and github.event.pull_request.base.sha" >&2
  exit 2
fi
if ! fetched="$(git fetch --quiet --no-tags origin "$1" 2>&1)"; then
  printf '%s\n' "$fetched" >&2
  echo "ci-pr-base: could not fetch $1 from origin; check out with fetch-depth: 0 and re-run." >&2
  exit 1
fi
if ! merge_base="$(git merge-base "$2" HEAD 2>&1)"; then
  printf '%s\n' "$merge_base" >&2
  echo "ci-pr-base: $2 and HEAD share no merge base in this checkout; check out with fetch-depth: 0." >&2
  exit 1
fi
merge_base="$(printf '%s' "$merge_base" | tr -d '\r')"
[ -z "${GITHUB_OUTPUT:-}" ] || echo "base=$merge_base" >>"$GITHUB_OUTPUT"
echo "Affected against the merge base with $1: $merge_base"
