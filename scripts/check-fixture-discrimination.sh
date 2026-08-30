#!/usr/bin/env bash
# Watch this repository's own tests catch the defect its fixtures used to hide.
#
# scripts/check-store-fixtures.sh reads the two discriminating properties back off every
# store fixture, and scripts/check-store-fixtures-enforced.sh watches it refuse a fixture
# that lacks one. Neither of those runs a test: together they prove the *rule* is enforced,
# not that the fixtures obeying it actually separate the wrong answer from the right one.
#
# That gap is the whole failure this release repaired. `title: required_str(content, "id")`
# — a board item titled with its own identifier — shipped, and every assertion over it
# passed, because the fixture it was read through spelled the two the same string. The
# fixtures now differ; whether that difference reaches an assertion is a question only the
# assertions can answer.
#
# So this reintroduces exactly that substitution, in a scratch copy, and asserts the crate's
# real tests go red on it. Case 0 is the control: without it a suite that could not compile
# at all would look like the strictest suite in the repository.
set -euo pipefail

fatal() {
  echo "check-fixture-discrimination: $1" >&2
  echo "check-fixture-discrimination: next: $2" >&2
  exit 1
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)" || fatal \
  "could not resolve this repository's root from ${BASH_SOURCE[0]}" \
  "run the check from a checkout of this repository, as 'just distribution-test' does"
readonly ROOT
cd "$ROOT" || fatal \
  "could not enter $ROOT" \
  "check that directory's permissions, then rerun"

command -v cargo >/dev/null 2>&1 || fatal \
  "cargo is not installed, so the suite this check mutates cannot be run" \
  "install the Rust toolchain from https://rustup.rs, then rerun"

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

# The WORKING tree's tracked files, for the same reason check-store-fixtures-enforced.sh
# takes them: what is under test is the fixtures and the suite as they are right now, so an
# author repairing either does not watch this keep failing against the version they replaced.
mkdir -p "$scratch/repo" || fatal \
  "could not create the scratch tree at $scratch/repo" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
(cd "$ROOT" && git ls-files -z | tar --null -T - -cf -) | tar -xf - -C "$scratch/repo" || fatal \
  "could not copy $ROOT's tracked files into $scratch/repo" \
  "confirm 'git ls-files' answers in $ROOT and 'df -h' for free space, then rerun"

# Its own target directory, inside the scratch tree: sharing this repository's would make
# every later `just test` rebuild what this check displaced, and the crate's dependencies
# are already unpacked in the cargo registry, so a cold build here is under a minute.
readonly CRATE=onetaskgraph-github-projects
readonly SOURCE="crates/$CRATE/src/lib.rs"

SUITE_OUTPUT=""
SUITE_STATUS=0

run_suite() {
  SUITE_OUTPUT="$(cd "$scratch/repo" \
    && CARGO_TARGET_DIR="$scratch/target" cargo test -q -p "$CRATE" 2>&1)" \
    && SUITE_STATUS=0 || SUITE_STATUS=$?
}

report_suite_output() {
  printf '%s\n' "$SUITE_OUTPUT" | sed 's/^/    /' >&2
}

# 0. The control.
run_suite
if [ "$SUITE_STATUS" -ne 0 ]; then
  echo "check-fixture-discrimination: $CRATE's tests do not pass on the tree under test, so" >&2
  echo "check-fixture-discrimination: the case below would prove nothing — a suite that is" >&2
  echo "check-fixture-discrimination: already red goes red under any mutation. It said:" >&2
  report_suite_output
  echo "check-fixture-discrimination: next: run 'cargo test -p $CRATE' and fix what it reports" >&2
  echo "check-fixture-discrimination: first, then rerun." >&2
  exit 1
fi

# 1. The defect exactly as it shipped: the board item's own identifier written into the
#    field its title belongs in. One site, and the count is asserted rather than assumed —
#    a substitution that matched nothing would leave the suite green and read as the
#    fixtures having caught nothing.
readonly BEFORE='title: required_str(content, "title")?.to_owned(),'
readonly AFTER='title: required_str(content, "id")?.to_owned(),'
MUTATION_SOURCE="$scratch/repo/$SOURCE" MUTATION_BEFORE="$BEFORE" MUTATION_AFTER="$AFTER" \
  python3 - <<'PY' || fatal \
  "could not reintroduce the id-where-a-title-belongs substitution" \
  "the message above names what changed; update BEFORE in this check to the site's shape now"
import os
import pathlib
import sys

path = pathlib.Path(os.environ["MUTATION_SOURCE"])
before, after = os.environ["MUTATION_BEFORE"], os.environ["MUTATION_AFTER"]
text = path.read_text(encoding="utf-8")
found = text.count(before)
if found != 1:
    print(
        f"check-fixture-discrimination: the line this check substitutes appears {found} "
        f"times in {path.name}, not once:",
        file=sys.stderr,
    )
    print(f"check-fixture-discrimination:   {before}", file=sys.stderr)
    raise SystemExit(1)
path.write_text(text.replace(before, after), encoding="utf-8")
PY

run_suite
if [ "$SUITE_STATUS" -eq 0 ]; then
  echo "check-fixture-discrimination: $CRATE reads every board item's identifier into the" >&2
  echo "check-fixture-discrimination: field its title belongs in, and its tests still pass." >&2
  echo "check-fixture-discrimination: That is the defect this release repaired, and the" >&2
  echo "check-fixture-discrimination: fixtures exist to separate it from the correct answer." >&2
  echo "check-fixture-discrimination: next: add an assertion over a title whose value differs" >&2
  echo "check-fixture-discrimination: from its item's identifier — 'bash scripts/check-store-fixtures.sh'" >&2
  echo "check-fixture-discrimination: names the fixtures that already carry such an item." >&2
  exit 1
fi

# A suite that fails to BUILD fails too, and would satisfy the case above while proving
# nothing about any assertion. `test result: FAILED` is only ever printed by a harness that
# compiled, linked and ran.
case "$SUITE_OUTPUT" in
  *"test result: FAILED"*) ;;
  *)
    echo "check-fixture-discrimination: the substitution stopped $CRATE compiling rather than" >&2
    echo "check-fixture-discrimination: failing an assertion, so nothing here says the fixtures" >&2
    echo "check-fixture-discrimination: discriminate. It said:" >&2
    report_suite_output
    echo "check-fixture-discrimination: next: update AFTER in this check so the substitution is" >&2
    echo "check-fixture-discrimination: type-correct — it must change what a test reads, not the" >&2
    echo "check-fixture-discrimination: build." >&2
    exit 1
    ;;
esac
