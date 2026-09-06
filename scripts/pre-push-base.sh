#!/usr/bin/env bash
# The commit a pre-push gate compares against, read from what git is about to push.
#
# The hook used to sweep every project, so it needed no base at all. It selects now, for
# the reason scripts/live-lane-selection.sh gives: a plugin whose tests reach a real API
# must run them when that plugin changed and not otherwise, and a sweep runs them whatever
# the diff touched. An implicit base is how affected selection quietly starts comparing
# against the wrong commit, so the base is derived here rather than left to a default —
# the same approach .github/workflows/ci.yml takes for a pull request.
#
# git feeds a pre-push hook one record per ref on stdin:
#
#   <local ref> <local sha> <remote ref> <remote sha>
#
# and `<remote sha>` is exactly what this wants: the commit the remote already has, so the
# diff against it is what this push adds. Two records are not a branch's history and there
# is no one commit that answers for both, and an all-zero remote sha is a branch the remote
# has never seen; both fall back to the merge base with the default branch.
#
# Prints one ref, or NOTHING when it cannot derive one — which leaves NX_BASE unset and Nx
# comparing against its own `defaultBase`, the behaviour every other local invocation has.
# It never fails the push: a base it could not derive is not a reason to refuse work.
#
# Usage: scripts/pre-push-base.sh < <the pre-push ref records>
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

readonly ZERO="0000000000000000000000000000000000000000"

resolves() {
  git rev-parse --verify --quiet "$1^{commit}" >/dev/null 2>&1
}

# nx.json's own defaultBase, so the fallback below is the very ref Nx would have used
# rather than a second spelling of it that could drift.
default_base() {
  python3 -c '
import json, sys
try:
    base = json.load(open("nx.json")).get("defaultBase")
except (OSError, ValueError):
    raise SystemExit(0)
if isinstance(base, str) and base:
    sys.stdout.write(base)
' 2>/dev/null
}

fallback() {
  local base
  base="$(default_base)"
  [ -n "$base" ] || return 0
  resolves "$base" || return 0
  # The merge base rather than the branch tip: what this push adds is what it added since
  # it forked, and a base that has moved on since would mark everything else affected too.
  git merge-base "$base" HEAD 2>/dev/null || printf '%s' "$base"
}

records=0
remote_sha=""
while read -r _local_ref local_sha _remote_ref candidate; do
  # A ref being deleted pushes nothing to check.
  [ "$local_sha" = "$ZERO" ] && continue
  records=$((records + 1))
  remote_sha="$candidate"
done

if [ "$records" -eq 1 ] && [ -n "$remote_sha" ] && [ "$remote_sha" != "$ZERO" ] && resolves "$remote_sha"; then
  printf '%s\n' "$remote_sha"
  exit 0
fi

base="$(fallback)"
[ -n "$base" ] && printf '%s\n' "$base"
exit 0
