#!/usr/bin/env bash
# Watch scripts/check-store-fixtures.sh refuse the fixtures that hid two shipped defects.
#
# A guard nobody has watched fail is a guard nobody knows works, and this one is checking a
# property whose absence is invisible: a fixture where every native id is its own title and
# every store holds one project passes every test it has, which is exactly how "write the
# project's id where its title belongs" and "discard the project query" both shipped.
#
# So each case below puts one of those fixtures back the way it was and asserts the check
# refuses, naming the fixture and which of the two properties it lacks.
set -euo pipefail

fatal() {
  echo "check-store-fixtures-enforced: $1" >&2
  echo "check-store-fixtures-enforced: next: $2" >&2
  exit 1
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)" || fatal \
  "could not resolve this repository's root from ${BASH_SOURCE[0]}" \
  "run the check from a checkout of this repository, as 'just distribution-check' does"
readonly ROOT

# The path below is built from $ROOT at run time, so ShellCheck cannot follow it; the
# directive names the file it resolves to. Tested before it is sourced rather than guarded
# after: bash 3.2 ends the shell where `source` cannot find its file, so the handler after
# `||` never runs there.
# shellcheck source=scripts/scratch-clone.sh
if [ ! -r "$ROOT/scripts/scratch-clone.sh" ] || ! source "$ROOT/scripts/scratch-clone.sh"; then
  fatal "could not load $ROOT/scripts/scratch-clone.sh, which strips the git environment" \
    "restore it with 'git checkout -- scripts/scratch-clone.sh' and rerun"
fi
scratch_clone_strip_git_env

scratch="$(mktemp -d)" || fatal \
  "could not create the scratch tree this check mutates" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
trap 'rm -rf "$scratch"' EXIT

# The WORKING tree's tracked files: what is under test is the check and the fixtures as
# they are right now, so an author repairing either does not watch this keep failing
# against the version they just replaced.
mkdir -p "$scratch/repo" || fatal \
  "could not create the scratch tree at $scratch/repo" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
(cd "$ROOT" && git ls-files -z | tar --null -T - -cf -) | tar -xf - -C "$scratch/repo" || fatal \
  "could not copy $ROOT's tracked files into $scratch/repo" \
  "confirm 'git ls-files' answers in $ROOT and 'df -h' for free space, then rerun"

failures=0
GUARD_OUTPUT=""
GUARD_STATUS=0

run_guard() {
  GUARD_OUTPUT="$(cd "$scratch/repo" && bash scripts/check-store-fixtures.sh 2>&1)" \
    && GUARD_STATUS=0 || GUARD_STATUS=$?
}

report_guard_output() {
  printf '%s\n' "$GUARD_OUTPUT" | sed 's/^/    /' >&2
}

expect_refused() {
  local case_name="$1"
  shift
  if [ "$GUARD_STATUS" -eq 0 ]; then
    echo "check-store-fixtures-enforced: $case_name — check-store-fixtures.sh PASSED a" >&2
    echo "check-store-fixtures-enforced: fixture under which the wrong answer and the right" >&2
    echo "check-store-fixtures-enforced: answer are the same bytes, which is how two defects" >&2
    echo "check-store-fixtures-enforced: shipped from this repository with a green suite." >&2
    failures=$((failures + 1))
    return
  fi
  local term
  for term in "$@"; do
    # A here-string rather than a pipe into a quiet grep: under `pipefail` an early exit
    # SIGPIPEs its writer, which can invert the pipeline's status on a match.
    if ! grep -qF -- "$term" <<<"$GUARD_OUTPUT"; then
      echo "check-store-fixtures-enforced: $case_name — the check refused, but never" >&2
      echo "check-store-fixtures-enforced: mentions '$term', so it does not say which fixture" >&2
      echo "check-store-fixtures-enforced: to go and enrich. It said:" >&2
      report_guard_output
      failures=$((failures + 1))
      return
    fi
  done
}

# Rewrite one fixture in the scratch tree with a python expression over the parsed JSON.
# python3 rather than sed, whose in-place spelling differs between GNU and BSD and so
# would fail on the macOS runner.
rewrite() {
  local path="$1" program="$2" status=0
  # Exit 3 is the helper saying it already printed the problem and the next action; any
  # other non-zero status is python3 itself failing, which would otherwise end the run on a
  # traceback alone.
  python3 - "$scratch/repo/$path" "$program" <<'PY' || status=$?
import json
import pathlib
import sys

path, program = pathlib.Path(sys.argv[1]), sys.argv[2]
try:
    document = json.loads(path.read_text(encoding="utf-8"))
except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
    print(
        f"check-store-fixtures-enforced: could not read {path}, the scratch copy this case\n"
        f"check-store-fixtures-enforced: rewrites: {error}\n"
        "check-store-fixtures-enforced: next: check the permissions of $TMPDIR and 'df -h'\n"
        "check-store-fixtures-enforced: for free space, then rerun.",
        file=sys.stderr,
    )
    raise SystemExit(3)
before = json.dumps(document, sort_keys=True)
exec(program, {"document": document})  # noqa: S102 — the program is this script's own
if json.dumps(document, sort_keys=True) == before:
    print(
        f"check-store-fixtures-enforced: rewriting {path} changed nothing, so the case would\n"
        "check-store-fixtures-enforced: prove nothing about the check.\n"
        "check-store-fixtures-enforced: next: update the case to the fixture's shape now.",
        file=sys.stderr,
    )
    raise SystemExit(3)
path.write_text(json.dumps(document, indent=2) + "\n", encoding="utf-8")
PY
  if [ "$status" -ne 0 ]; then
    [ "$status" -eq 3 ] || echo "check-store-fixtures-enforced: python3 failed rewriting $path" >&2
    exit 1
  fi
}

restore() {
  local path
  for path in "$@"; do
    cp "$ROOT/$path" "$scratch/repo/$path" || fatal \
      "could not restore $path in $scratch/repo, so the cases after this one would run against a tree still carrying the previous mutation" \
      "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
  done
}

# 0. The control. Without it, a check that refused every tree — including this one — would
#    satisfy every case below and look like the strictest check in the repository.
run_guard
if [ "$GUARD_STATUS" -ne 0 ]; then
  echo "check-store-fixtures-enforced: the check refuses the tree under test, so the cases" >&2
  echo "check-store-fixtures-enforced: below would prove nothing. Run" >&2
  echo "check-store-fixtures-enforced: 'bash scripts/check-store-fixtures.sh' and fix what it" >&2
  echo "check-store-fixtures-enforced: reports first. It said:" >&2
  report_guard_output
  exit 1
fi

# 1. The identity defect, exactly as it shipped: every project's *identifier* written where
#    its title belongs. Under a fixture that already spelled them the same, the defect and
#    the correct behaviour were byte-identical and no assertion could separate them.
readonly BOARD=crates/onetaskgraph-github-projects/tests/fixtures/project.json
rewrite "$BOARD" '
for item in document["data"]["owner"]["projectV2"]["items"]["nodes"]:
    content = item.get("content") or {}
    if "title" in content:
        content["title"] = content["id"]
'
run_guard
expect_refused "a board whose every item is titled with its own identifier" \
  "$BOARD" "identifier differs from its title"
restore "$BOARD"

# 2. The one-project store: a query that selects one project and a query that selects all
#    of them answer with the same row, so a source that discards the predicate outright
#    passes every test written over it. That is the second defect this release repaired.
readonly PROJECTS=crates/onetaskgraph-linear/tests/fixtures/projects.json
rewrite "$PROJECTS" '
nodes = document["data"]["projects"]["nodes"]
del nodes[1:]
'
run_guard
expect_refused "a project store holding a single project" \
  "$PROJECTS" "at least 2"
restore "$PROJECTS"

# 3. One value of a thing the code filters on. The rows a filter keeps and the rows it
#    would keep if it were never applied are the same rows.
readonly ISSUES=crates/onetaskgraph-linear/tests/fixtures/issues.json
rewrite "$ISSUES" '
nodes = document["data"]["issues"]["nodes"]
for node in nodes[1:]:
    node["state"] = nodes[0]["state"]
'
run_guard
expect_refused "an issue store whose every issue is in the same state" \
  "$ISSUES" "distinct status value"
restore "$ISSUES"

if [ "$failures" -ne 0 ]; then
  echo "check-store-fixtures-enforced: $failures case(s) failed." >&2
  echo "check-store-fixtures-enforced: repair scripts/check-store-fixtures.sh rather than" >&2
  echo "check-store-fixtures-enforced: relaxing the case above: a fixture that cannot tell the" >&2
  echo "check-store-fixtures-enforced: wrong answer from the right one is how both of this" >&2
  echo "check-store-fixtures-enforced: release's defects reached a version." >&2
  exit 1
fi
