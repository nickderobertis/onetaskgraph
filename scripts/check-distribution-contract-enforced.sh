#!/usr/bin/env bash
# Prove that scripts/check-distribution-contract.sh REFUSES a release workflow whose
# crates.io existence query does not identify the caller to the registry.
#
# That pin exists because crates.io answers curl's default user agent with 403 — an answer
# that says nothing about whether a crate is published — so the publish-crates job could
# never reach either arm it knows how to act on, and a whole release cycle was spent
# discovering it. A pin nobody has watched fail is a pin nobody knows works: these greps
# would pass just as quietly if a pattern stopped matching what it describes, or if the
# query moved somewhere the pin does not read.
#
# So the unidentified query is reintroduced for real, three ways, in a scratch copy of the
# working tree, and each case asserts on the DIAGNOSTIC as well as the refusal — a pin that
# refuses without naming the user agent sends the next author hunting through a release
# workflow, which is most of what it is for.
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT

# Git exports GIT_DIR to every hook and GIT_DIR overrides `git -C`; the gate runs from
# .githooks/pre-push, so `git ls-files` below has to be asked in a stripped environment or
# it answers about whatever repository the hook was invoked for.
# shellcheck source=scripts/scratch-clone.sh
source "$ROOT/scripts/scratch-clone.sh"
scratch_clone_strip_git_env

# The WORKING tree's tracked files, not HEAD's: what is under test is the pin as it is
# right now, so an author repairing it does not watch this check keep failing against the
# version they just replaced.
mkdir -p "$scratch/repo"
(cd "$ROOT" && git ls-files -z | tar --null -T - -cf -) | tar -xf - -C "$scratch/repo"

failures=0
GUARD_OUTPUT=""
GUARD_STATUS=0

# Run the pin as it is written, from inside the scratch tree, so what is under test is the
# real script rather than a copy of its logic.
run_guard() {
  GUARD_OUTPUT="$(cd "$scratch/repo" && bash scripts/check-distribution-contract.sh 2>&1)" \
    && GUARD_STATUS=0 || GUARD_STATUS=$?
}

# Undo a case's edit by restoring that file from the working tree it was copied from.
restore() {
  local path
  for path in "$@"; do
    mkdir -p "$(dirname "$scratch/repo/$path")"
    cp "$ROOT/$path" "$scratch/repo/$path"
  done
}

# Replace one literal substring of a file in the scratch tree. python3 rather than `sed -i`,
# whose in-place spelling differs between GNU and BSD and so would fail on the macOS runner.
substitute() {
  local path="$1" before="$2" after="$3"
  python3 - "$scratch/repo/$path" "$before" "$after" <<'PY'
import pathlib
import sys

path, before, after = pathlib.Path(sys.argv[1]), sys.argv[2], sys.argv[3]
text = path.read_text(encoding="utf-8")
if before not in text:
    raise SystemExit(
        f"check-distribution-contract-enforced: {path} no longer contains the text this case\n"
        f"check-distribution-contract-enforced: rewrites, so the case would prove nothing:\n"
        f"    {before}\n"
        "check-distribution-contract-enforced: update the case to the text that replaced it."
    )
path.write_text(text.replace(before, after, 1), encoding="utf-8")
PY
}

report_guard_output() {
  printf '%s\n' "$GUARD_OUTPUT" | sed 's/^/    /' >&2
}

expect_refused() {
  local case_name="$1"
  shift
  if [ "$GUARD_STATUS" -eq 0 ]; then
    echo "check-distribution-contract-enforced: $case_name — check-distribution-contract.sh" >&2
    echo "check-distribution-contract-enforced: PASSED a release workflow whose crates.io query does" >&2
    echo "check-distribution-contract-enforced: not identify the caller. The registry answers that" >&2
    echo "check-distribution-contract-enforced: query with 403, so publish-crates would publish nothing." >&2
    failures=$((failures + 1))
    return
  fi
  local term
  for term in "$@"; do
    # A here-string rather than a pipe into `grep -q`: under `pipefail` a quiet grep's early
    # exit SIGPIPEs its writer, which can invert the pipeline's status on a match.
    if ! grep -qF -- "$term" <<<"$GUARD_OUTPUT"; then
      echo "check-distribution-contract-enforced: $case_name — the pin refused, but its diagnostic" >&2
      echo "check-distribution-contract-enforced: never mentions '$term', so it does not say what to go" >&2
      echo "check-distribution-contract-enforced: and fix. It said:" >&2
      report_guard_output
      failures=$((failures + 1))
      return
    fi
  done
}

# 0. The control. Without it, a pin that refused every tree — including this one — would
#    satisfy all three cases below and look like the strictest check in the repository.
run_guard
if [ "$GUARD_STATUS" -ne 0 ]; then
  echo "check-distribution-contract-enforced: the pin refuses the tree under test, so the cases" >&2
  echo "check-distribution-contract-enforced: below would prove nothing. Run" >&2
  echo "check-distribution-contract-enforced: 'bash scripts/check-distribution-contract.sh' and fix" >&2
  echo "check-distribution-contract-enforced: what it reports first. It said:" >&2
  report_guard_output
  exit 1
fi

# 1. The defect itself, restored: the job asks crates.io with curl's default agent, which
#    the registry answers 403 — so no crate is ever seen as absent and none is published.
substitute .github/workflows/release.yml \
  'publication=$(scripts/crate-publication-status.sh "$crate" "$version") || exit $?' \
  'publication=$(curl -sS -o /dev/null -w '"'"'%{http_code}'"'"' "https://crates.io/api/v1/crates/$crate/$version")'
run_guard
expect_refused "the workflow querying crates.io itself, unidentified" \
  "must decide from scripts/crate-publication-status.sh"
restore .github/workflows/release.yml

# 2. The decision still comes from the script, but an unidentified query is left beside it —
#    a probe, a retry, a warm-up — and whichever one the job then trusts, the 403 is back.
substitute .github/workflows/release.yml \
  'publication=$(scripts/crate-publication-status.sh "$crate" "$version") || exit $?' \
  'curl -sS -o /dev/null "https://crates.io/api/v1/crates/$crate/$version" || true
            publication=$(scripts/crate-publication-status.sh "$crate" "$version") || exit $?'
run_guard
expect_refused "an unidentified query left beside the one that decides" \
  "must stay in scripts/crate-publication-status.sh"
restore .github/workflows/release.yml

# 3. The query stays where the pin reads it, but stops naming a caller — the same 403, one
#    edit away, and the one a later author is most likely to make while tidying.
substitute scripts/crate-publication-status.sh '--user-agent "$agent" ' ''
run_guard
expect_refused "the query dropping its explicit user agent" \
  "must send an explicit user agent"
restore scripts/crate-publication-status.sh

# 4. An agent that is present but anonymous. crates.io's policy is that the caller be
#    identifiable and contactable, so a bare token satisfies --user-agent and not the policy.
substitute scripts/crate-publication-status.sh \
  'agent="onetaskgraph-release (https://github.com/nickderobertis/onetaskgraph)"' \
  'agent="release"'
run_guard
expect_refused "an agent naming no release and no contact URL" \
  "must name this release and a contact URL"
restore scripts/crate-publication-status.sh

if [ "$failures" -ne 0 ]; then
  echo "check-distribution-contract-enforced: $failures case(s) failed." >&2
  echo "check-distribution-contract-enforced: a release whose crates.io query the registry declines" >&2
  echo "check-distribution-contract-enforced: now merges unnoticed, and it fails a full release cycle" >&2
  echo "check-distribution-contract-enforced: later as what reads like a credentials problem. Repair" >&2
  echo "check-distribution-contract-enforced: the pin in scripts/check-distribution-contract.sh rather" >&2
  echo "check-distribution-contract-enforced: than relaxing the case above." >&2
  exit 1
fi
