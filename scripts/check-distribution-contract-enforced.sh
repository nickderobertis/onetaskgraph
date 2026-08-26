#!/usr/bin/env bash
# Prove that scripts/check-distribution-contract.sh REFUSES release publication calls
# whose registry query or package operand does not meet the distribution contract.
#
# crates.io answers curl's default user agent with 403 — an answer that says nothing about
# whether a crate is published — so an unidentified query leaves publish-crates unable to
# reach either arm it knows how to act on. A pin nobody has watched fail is a pin nobody
# knows works: those greps would pass just as quietly if a pattern stopped matching what it
# describes, or if the query moved somewhere the pin does not read.
set -euo pipefail

# Every step that builds the scratch tree is fatal, and `set -e` alone would end the run on
# the underlying tool's diagnostic with nothing said about what to do about it.
fatal() {
  echo "check-distribution-contract-enforced: $1" >&2
  echo "check-distribution-contract-enforced: next: $2" >&2
  exit 1
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)" || fatal \
  "could not resolve this repository's root from ${BASH_SOURCE[0]}" \
  "run the check from a checkout of this repository, as 'just distribution-check' does"
readonly ROOT

scratch="$(mktemp -d)" || fatal \
  "could not create the scratch tree this check mutates" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
trap 'rm -rf "$scratch"' EXIT

# Git exports GIT_DIR to every hook and GIT_DIR overrides `git -C`; the gate runs from
# .githooks/pre-push, so `git ls-files` below has to be asked in a stripped environment or
# it answers about whatever repository the hook was invoked for.
#
# The path below is built from $ROOT at run time, so ShellCheck cannot follow it to decide
# whether scratch_clone_strip_git_env is defined; the directive names the file it resolves
# to so that check keeps working.
# shellcheck source=scripts/scratch-clone.sh
# Tested before it is sourced, not merely guarded after: bash 3.2 ends the shell where
# `source` cannot find its file, so the handler a later bash takes never runs there — and
# macos-latest is a 3.2 runner. Without this the reader gets bash's own "No such file or
# directory", which names the sourcing line rather than the file to put back.
if [ ! -r "$ROOT/scripts/scratch-clone.sh" ] || ! source "$ROOT/scripts/scratch-clone.sh"; then
  fatal \
    "could not load $ROOT/scripts/scratch-clone.sh, which strips the git environment" \
    "restore that file with 'git checkout -- scripts/scratch-clone.sh' and rerun"
fi
scratch_clone_strip_git_env

# The WORKING tree's tracked files, not HEAD's: what is under test is the pin as it is
# right now, so an author repairing it does not watch this check keep failing against the
# version they just replaced.
mkdir -p "$scratch/repo" || fatal \
  "could not create the scratch tree at $scratch/repo" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
(cd "$ROOT" && git ls-files -z | tar --null -T - -cf -) | tar -xf - -C "$scratch/repo" || fatal \
  "could not copy $ROOT's tracked files into $scratch/repo (see the tar or git output above)" \
  "confirm 'git ls-files' answers in $ROOT and 'df -h' for free space, then rerun"

failures=0
GUARD_OUTPUT=""
GUARD_STATUS=0

# Run the pin as it is written, from inside the scratch tree, so what is under test is the
# real script rather than a copy of its logic.
run_guard() {
  GUARD_OUTPUT="$(cd "$scratch/repo" && bash scripts/check-distribution-contract.sh 2>&1)" \
    && GUARD_STATUS=0 || GUARD_STATUS=$?
}

restore() {
  local path
  for path in "$@"; do
    mkdir -p "$(dirname "$scratch/repo/$path")" || fatal \
      "could not recreate the directory holding $path in $scratch/repo" \
      "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
    # An unrestored file leaves every later case mutating a tree that is already wrong, so
    # this stops rather than carrying on with cases that would prove nothing.
    cp "$ROOT/$path" "$scratch/repo/$path" || fatal \
      "could not restore $path in $scratch/repo, so the cases after this one would run against a tree still carrying the previous mutation" \
      "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
  done
}

# Replace one literal substring of a file in the scratch tree. python3 rather than `sed -i`,
# whose in-place spelling differs between GNU and BSD and so would fail on the macOS runner.
substitute() {
  local path="$1" before="$2" after="$3" status=0
  # Exit 3 is the helper saying it already printed the problem and the next action. Any
  # other non-zero status is python3 itself failing — a missing interpreter, an unexpected
  # exception — which would otherwise end the run on a traceback alone.
  python3 - "$scratch/repo/$path" "$before" "$after" <<'PY' || status=$?
import pathlib
import sys

path, before, after = pathlib.Path(sys.argv[1]), sys.argv[2], sys.argv[3]
try:
    text = path.read_text(encoding="utf-8")
except OSError as error:
    print(
        f"check-distribution-contract-enforced: could not read {path}, the scratch copy\n"
        f"check-distribution-contract-enforced: this case rewrites: {error}\n"
        "check-distribution-contract-enforced: next: check the permissions of $TMPDIR and\n"
        "check-distribution-contract-enforced: 'df -h' for free space, then rerun.",
        file=sys.stderr,
    )
    raise SystemExit(3)
if before not in text:
    print(
        f"check-distribution-contract-enforced: {path} no longer contains the text this case\n"
        f"check-distribution-contract-enforced: rewrites, so the case would prove nothing:\n"
        f"    {before}\n"
        "check-distribution-contract-enforced: next: update the case to the text that replaced it.",
        file=sys.stderr,
    )
    raise SystemExit(3)
try:
    path.write_text(text.replace(before, after, 1), encoding="utf-8")
except OSError as error:
    print(
        f"check-distribution-contract-enforced: could not write the mutated {path}, so this\n"
        f"check-distribution-contract-enforced: case could not be put to the pin: {error}\n"
        "check-distribution-contract-enforced: next: check the permissions of $TMPDIR and\n"
        "check-distribution-contract-enforced: 'df -h' for free space, then rerun.",
        file=sys.stderr,
    )
    raise SystemExit(3)
PY
  case "$status" in
    0) ;;
    3) exit 1 ;;
    *) fatal \
      "the helper that rewrites $path in the scratch tree ended with status $status, so this case was never put to the pin" \
      "run 'python3 --version' to confirm a working python3 is on PATH, then rerun" ;;
  esac
}

report_guard_output() {
  printf '%s\n' "$GUARD_OUTPUT" | sed 's/^/    /' >&2
}

expect_refused() {
  local case_name="$1"
  shift
  if [ "$GUARD_STATUS" -eq 0 ]; then
    echo "check-distribution-contract-enforced: $case_name — check-distribution-contract.sh" >&2
    echo "check-distribution-contract-enforced: PASSED a release workflow that violates the" >&2
    echo "check-distribution-contract-enforced: publication contract, so the defect would reach a" >&2
    echo "check-distribution-contract-enforced: live registry before anyone discovered it." >&2
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
#    satisfy all four cases below and look like the strictest check in the repository.
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

# 5. The npm defect itself: without the explicit ./ prefix npm interprets npm/cli as the
#    GitHub shorthand for npm's own CLI repository and tries to publish that remote package.
substitute .github/workflows/release.yml \
  'publish_if_absent "@onetaskgraph/cli@$cli_version" ./npm/cli' \
  'publish_if_absent "@onetaskgraph/cli@$cli_version" npm/cli'
run_guard
expect_refused "a bare owner/name operand in place of the local CLI directory" \
  "installable npm packages must publish from explicit local directories"
restore .github/workflows/release.yml

if [ "$failures" -ne 0 ]; then
  echo "check-distribution-contract-enforced: $failures case(s) failed." >&2
  echo "check-distribution-contract-enforced: a release whose crates.io query the registry declines" >&2
  echo "check-distribution-contract-enforced: now merges unnoticed, and it fails a full release cycle" >&2
  echo "check-distribution-contract-enforced: later as what reads like a credentials problem. Repair" >&2
  echo "check-distribution-contract-enforced: the pin in scripts/check-distribution-contract.sh rather" >&2
  echo "check-distribution-contract-enforced: than relaxing the case above." >&2
  exit 1
fi
