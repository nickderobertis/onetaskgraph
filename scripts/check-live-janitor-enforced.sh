#!/usr/bin/env bash
# Watch scripts/check-live-janitor.sh hold CUTOVER_MICROS to the author second of the commit
# that introduced it, over real commits: a guard nobody has watched fail is not known to work.
#
# Each case commits on an orphan branch of a scratch clone, so the history the guard reads
# is exactly the history the case wrote, whatever this repository's own history holds.
set -euo pipefail
fatal() {
  echo "check-live-janitor-enforced: $1" >&2
  echo "check-live-janitor-enforced: next: $2" >&2
  exit 1
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)" ||
  fatal "could not resolve the repository root from ${BASH_SOURCE[0]}" "run it as 'bash scripts/check-live-janitor-enforced.sh' from a checkout"
readonly ROOT
readonly JANITOR="crates/onetaskgraph-live-janitor/src/lib.rs"
readonly SECOND=1791000000

scratch="$(mktemp -d)" || fatal "could not make a scratch directory" "check \$TMPDIR and 'df -h'"
trap 'rm -rf "$scratch"' EXIT
if [ ! -f "$ROOT/scripts/scratch-clone.sh" ]; then
  fatal "scripts/scratch-clone.sh is missing" "restore it from git, then rerun"
fi
# shellcheck source=scripts/scratch-clone.sh
source "$ROOT/scripts/scratch-clone.sh" ||
  fatal "scripts/scratch-clone.sh could not be loaded" "restore it from git, then rerun"
scratch_clone "$ROOT" "$scratch/repo" || fatal "could not clone this repository" "read the diagnostic above"
# The working tree's tracked files, so a guard or janitor not yet committed is what is proven.
(cd "$ROOT" && git ls-files -z | tar --null -T - -cf -) | tar -xf - -C "$scratch/repo" ||
  fatal "could not copy the working tree into $scratch/repo" "check 'df -h' for free space, then rerun"
repo="$scratch/repo"
failures=0

# set_cutover <micros>: the janitor's declaration, rewritten in the scratch tree only.
set_cutover() {
  python3 - "$repo/$JANITOR" "$1" <<'PY' || fatal "could not rewrite CUTOVER_MICROS" "check that $JANITOR still declares it"
import re
import sys
from pathlib import Path

path = Path(sys.argv[1])
text, count = re.subn(r'pub const CUTOVER_MICROS: u64 = [0-9_]+;',
                      f'pub const CUTOVER_MICROS: u64 = {sys.argv[2]};',
                      path.read_text(encoding='utf-8'))
if count != 1:
    sys.exit(1)
path.write_text(text, encoding='utf-8')
PY
}

# commit_at <second> <branch or ->: commit everything, on a fresh orphan branch when named.
commit_at() {
  if [ "$2" != "-" ]; then
    git -C "$repo" checkout --quiet --orphan "$2" ||
      fatal "could not start the orphan branch $2 in $repo" "read git's message above; if the scratch clone is damaged, delete \$TMPDIR's check-live-janitor scratch and rerun"
  fi
  git -C "$repo" add -A || fatal "could not stage the scratch tree" "check 'df -h', then rerun"
  GIT_AUTHOR_DATE="@$1 +0000" GIT_COMMITTER_DATE="@$1 +0000" git -C "$repo" \
    -c user.name=enforced -c user.email=enforced@invalid -c commit.gpgsign=false \
    commit --quiet --allow-empty -m "case at $1" ||
    fatal "could not commit the case dated $1 in $repo" "read git's message above; a commit hook or a full disk ('df -h') is the usual cause, then rerun"
}

# expect <case> <pass|refused> [diagnostic]
expect() {
  local output status=0
  output="$(cd "$repo" && bash scripts/check-live-janitor.sh 2>&1)" || status=$?
  if [ "$2" = pass ] && [ "$status" -ne 0 ]; then
    echo "check-live-janitor-enforced: $1 — the guard REFUSED it:" >&2
    printf '%s\n' "$output" | sed 's/^/    /' >&2
    failures=$((failures + 1))
  elif [ "$2" = refused ] && { [ "$status" -eq 0 ] || ! printf '%s' "$output" | grep -qF "$3"; }; then
    echo "check-live-janitor-enforced: $1 — the guard did not refuse it naming '$3':" >&2
    printf '%s\n' "$output" | sed 's/^/    /' >&2
    failures=$((failures + 1))
  fi
}

# Not yet introduced: HEAD has no declaration, so there is no author second to compare.
set_cutover 0
rename_cutover() {
  sed -i.bak "s/pub const $1:/pub const $2:/" "$repo/$JANITOR" && rm "$repo/$JANITOR.bak" ||
    fatal "could not rename $1 to $2 in the scratch copy of $JANITOR" "check 'df -h' and that $JANITOR still declares $1, then rerun"
}
rename_cutover CUTOVER_MICROS CUTOVER_PENDING
commit_at "$SECOND" pending
rename_cutover CUTOVER_PENDING CUTOVER_MICROS
set_cutover "$((SECOND + 7))000000"
expect "a declaration HEAD does not yet carry" pass

# Introduced at exactly its own author second.
set_cutover "${SECOND}000000"
commit_at "$SECOND" exact
expect "a constant equal to its introducing commit's author second" pass

# Edited after its introduction, uncommitted and then committed.
set_cutover "$((SECOND + 1))000000"
expect "a constant edited after its introduction" refused "changed after its introducing commit"
commit_at "$((SECOND + 1))" -
expect "a constant re-committed after its introduction" refused \
  "must equal its introducing commit author second"

# Introduced one second away from its own author time.
set_cutover "$((SECOND + 1))000000"
commit_at "$SECOND" off-by-one
expect "a constant one second from its introducing commit" refused \
  "must equal its introducing commit author second"

if [ "$failures" -ne 0 ]; then
  echo "check-live-janitor-enforced: $failures case(s) above did not go the way they must." >&2
  echo "check-live-janitor-enforced: next: fix the CUTOVER_MICROS provenance check in" >&2
  echo "check-live-janitor-enforced: scripts/check-live-janitor.sh; AGENTS.md says why it holds." >&2
  exit 1
fi
