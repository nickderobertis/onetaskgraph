#!/usr/bin/env bash
# The commit a pull request's affected selection compares against: the merge base of the
# commit under test with the pull request's base. Explicit, because an implicit base is how
# affected selection quietly starts comparing against the wrong commit (AGENTS.md).
#
#   scripts/ci-pr-base.sh <base-branch> <base-sha>
#
# Prints the merge base, and writes it to $GITHUB_OUTPUT as `base=` when Actions sets that.
set -euo pipefail

usage() {
  echo "usage: scripts/ci-pr-base.sh <base-branch> <base-sha>" >&2
  echo "ci-pr-base: $1; next: pass github.base_ref and github.event.pull_request.base.sha" >&2
  exit 2
}
[ "$#" -eq 2 ] || usage "expected two arguments, got $#"
# Both go to git, so neither may read as an option or a range: a plain branch name, and a
# full commit id.
case "$1" in
  "" | -* | *..* | *[!A-Za-z0-9._/-]*) usage "'$1' is not a plain branch name" ;;
esac
case "$2" in
  *[!0-9a-f]*) usage "'$2' is not a commit id" ;;
esac
[ "${#2}" -eq 40 ] || [ "${#2}" -eq 64 ] || usage "'$2' is not a full commit id"
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
