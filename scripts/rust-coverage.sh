#!/usr/bin/env bash
# The Rust line-coverage floor in two steps, over one LCOV report per coverage unit:
#
#   rust-coverage.sh <crate>     one crate's tests under nextest, instrumented, reduced by
#                                scripts/coverage-unit.sh to target/coverage-units/<crate>.lcov
#   rust-coverage.sh --report    those reports, every unit's, merged by scripts/coverage-merge.mjs
#                                and held to the floor
#   rust-coverage.sh --build-only <crate> [--test <target>]...
#                                that crate's tests built instrumented and none run, for a binary
#                                or suite another crate's run executes (no profile is written);
#                                only the named test targets when any are named
#
# Each unit's report is an Nx cache output, so a unit whose inputs did not change is replayed
# and its tests are not run. AGENTS.md records what the floor is over and why.
#
# On Windows every step is skipped with a printed notice: LLVM instrumentation there does
# not attribute coverage from the binary the e2e journeys spawn, so the number would
# understate the binary crate and mean nothing for the rest. The functional lanes (lint,
# typecheck, test) still run on all three platforms, so Windows is still gated. On macOS a
# unit runs uninstrumented and writes no report (scripts/coverage-unit.sh), so `--report` is
# skipped there too: coverage is measured on Linux.
set -euo pipefail

readonly REQUEST="${1:?usage: scripts/rust-coverage.sh <crate-name> | --report | --build-only <crate-name> [--test <target>]...}"
# On the LCOV basis. The user's 92 of 2026-10-30, over `cargo llvm-cov report`, plus the 0.8775
# points by which LCOV reads higher on the same run, rounded up, so no commit that fails the
# old floor at that measurement passes this one.
readonly MIN_LINES=92.88

# ONE live session per run of the gate, and this is where the second one would come from.
# `just check` runs `test` AND `coverage`, and `cargo llvm-cov --no-report nextest --package
# <crate>` below re-runs those same integration tests — so credentials left set here would open a
# second session against the shared external fixture the first may still be writing to.
# Neither session would delete the other's work — every artifact carries its writing run's
# own process id and the sweep that recovers an interrupted run's is decided by that stamp,
# in `onetaskgraph_live::artifact` — but a second session is a second session's worth of a
# third party's budget, spent to measure tests llvm-cov never measured anyway.
#
# The demand is cleared with them, or the skip that clearing produces would fail this phase
# for a session it is deliberately not running. Nothing is lost: llvm-cov never measured
# these tests. The other half of "one session per run" is the platform matrix, which hands
# the credentials to one of three `check` lanes. Changing the matrix or this target means
# keeping that pair true.
unset GH_PROJECTS_TOKEN LINEAR_API_KEY ONETASKGRAPH_LIVE_REQUIRED

case "${OS:-}${OSTYPE:-}" in
  *Windows_NT* | *msys* | *cygwin* | *win32*)
    echo "rust-coverage: $REQUEST skipped on Windows (instrumentation there does not attribute subprocess coverage); the functional lanes still gate this platform" >&2
    exit 0
    ;;
esac

# Coverage is measured on Linux: elsewhere the units wrote no report for the merge to read.
if [ "$REQUEST" = "--report" ] && [ "$(uname -s)" != Linux ]; then
  echo "rust-coverage: --report skipped on $(uname -s); coverage is measured on Linux" >&2
  exit 0
fi

if ! cargo llvm-cov --version >/dev/null 2>&1; then
  echo "rust-coverage: cargo-llvm-cov is not installed." >&2
  echo "rust-coverage: install it with 'cargo binstall cargo-llvm-cov' and re-run." >&2
  exit 1
fi
if ! cargo nextest --version >/dev/null 2>&1; then
  echo "rust-coverage: cargo-nextest is not installed, and a crate's instrumented run goes through it." >&2
  echo "rust-coverage: install it with 'cargo binstall cargo-nextest' and re-run." >&2
  exit 1
fi

# The merge reads exactly the units named below, so a report left behind under any other name
# refuses it rather than reaching the total. The unit list is workspace:coverage's own, and
# that target depends on each, so every file read was restored or written in this Nx run.
if [ "$REQUEST" = "--report" ]; then
  if ! units="$(python3 -c '
import json
import re
try:
    target = json.load(open("workspace/project.json", encoding="utf-8"))["targets"]["coverage"]
except (OSError, ValueError, KeyError, TypeError) as error:
    raise SystemExit(f"workspace/project.json: {error}")
depends_on = target.get("dependsOn") if isinstance(target, dict) else None
if not isinstance(depends_on, list):
    raise SystemExit("workspace/project.json: the coverage target has no dependsOn list")
names = []
for entry in depends_on:
    if not (isinstance(entry, dict) and entry.get("target") == "coverage"):
        continue
    projects = entry.get("projects")
    if not (isinstance(projects, list) and all(isinstance(name, str) for name in projects)):
        raise SystemExit("workspace/project.json: a coverage dependency has no list of project names")
    names.extend(projects)
unit_name = re.compile(r"[A-Za-z0-9_][A-Za-z0-9_-]*")
for name in names:
    if not unit_name.fullmatch(name):
        raise SystemExit(f"workspace/project.json: {name!r} is not a coverage unit name")
print(" ".join(names))
' | tr -d '\r')" || [ -z "$units" ]; then
    echo "rust-coverage: workspace/project.json's coverage target names no unit to merge." >&2
    echo "rust-coverage: next: restore its dependsOn list, which check-coverage-enforced.sh reconciles." >&2
    exit 1
  fi
  # shellcheck disable=SC2086 # $units is space-separated unit names, each one argument.
  merge_status=0
  node scripts/coverage-merge.mjs --fail-under-lines "$MIN_LINES" --dir target/coverage-units --unit $units || merge_status=$?
  if [ "$merge_status" -eq 1 ]; then
    echo "rust-coverage: the merge is below ${MIN_LINES}%; the uncovered lines are listed above." >&2
    echo "rust-coverage: cover them with a test that drives the real behaviour." >&2
  elif [ "$merge_status" -ne 0 ]; then
    echo "rust-coverage: the merge could not read the units' reports; its reason is above." >&2
    echo "rust-coverage: run 'just coverage' for fresh reports." >&2
  fi
  exit "$merge_status"
fi

build_only=false
# The test targets a build-only run is limited to: the ones another crate's measured run
# executes. Building the rest instrumented spends the job's CPU on binaries nothing runs.
only_tests=()
if [ "$REQUEST" = "--build-only" ]; then
  build_only=true
  readonly CRATE="${2:?usage: scripts/rust-coverage.sh --build-only <crate-name> [--test <target>]...}"
  shift 2
  while [ "$#" -gt 0 ]; do
    if [ "$1" != --test ] || [ "$#" -lt 2 ] || [ -z "$2" ] || [ "${2#-}" != "$2" ]; then
      echo "rust-coverage: --build-only takes a crate and then only '--test <target>' pairs, not '$*'." >&2
      echo "rust-coverage: next: name each test target another crate's run executes as --test <target>." >&2
      exit 2
    fi
    only_tests+=(--test "$2")
    shift 2
  done
else
  readonly CRATE="$REQUEST"
fi

# Validate the crate name against the real workspace before it is used as a Cargo
# selector. Unchecked, a typo becomes a silent "measured nothing".
if ! metadata="$(cargo metadata --format-version 1 --no-deps 2>&1)"; then
  echo "rust-coverage: could not read the workspace metadata:" >&2
  printf '%s\n' "$metadata" >&2
  echo "rust-coverage: fix the manifest so 'cargo metadata' resolves, then re-run." >&2
  exit 1
fi

# tr: $CRATE is matched against this list with `grep -qxF`, which a trailing CR defeats.
if ! members="$(printf '%s' "$metadata" \
  | python3 -c '
import json, sys
document = json.load(sys.stdin)
packages = document.get("packages") if isinstance(document, dict) else None
if not isinstance(packages, list) or not all(
    isinstance(package, dict) and isinstance(package.get("name"), str) for package in packages
):
    raise SystemExit("no packages array of named packages")
print("\n".join(package["name"] for package in packages))
' \
  | tr -d '\r')"; then
  echo "rust-coverage: 'cargo metadata' did not answer with a packages array of named packages." >&2
  echo "rust-coverage: fix the manifest so 'cargo metadata' resolves, then re-run." >&2
  exit 1
fi

if ! printf '%s\n' "$members" | grep -qxF -- "$CRATE"; then
  echo "rust-coverage: $CRATE is not a member of this Cargo workspace. Members:" >&2
  printf '  %s\n' $members >&2
  echo "rust-coverage: re-run with one of those names, or with --report." >&2
  exit 1
fi

# A crate whose suite runs uninstrumented but whose test binaries another crate's measured
# run drives (the GitHub Projects e2e pairs run the plugin's): built where that run finds
# them, with a filter selecting no test, so nothing of the crate's own suite is measured.
if "$build_only"; then
  if ! built="$(cargo llvm-cov --no-report nextest --package "$CRATE" --all-features --locked \
    ${only_tests[@]+"${only_tests[@]}"} -E 'none()' --no-tests=pass 2>&1)"; then
    printf '%s\n' "$built" >&2
    echo "rust-coverage: $CRATE's tests did not build under instrumentation — fix what cargo reports above, then re-run." >&2
    exit 1
  fi
  exit 0
fi

# The e2e journeys spawn the built binary; cargo-llvm-cov exports the profile path into
# that subprocess, so its coverage is attributed rather than lost. A failing test is the
# only failure here; the floor is the report's. Doctests are not run: nextest has none,
# and cargo-llvm-cov leaves them uninstrumented anyway.
# Keep each test's start and result visible while the suite runs, including when the
# job ends before nextest can print its summary. Preserve the captured failure diagnostic.
export NEXTEST_STATUS_LEVEL=all
export NEXTEST_EXPERIMENTAL_LIBTEST_JSON=1
export NEXTEST_MESSAGE_FORMAT=libtest-json-plus
if ! run="$(bash scripts/coverage-unit.sh "$CRATE" -- \
  nextest \
  --package "$CRATE" \
  --all-features \
  --locked 2>&1 | tee >(cat >&2))"; then
  printf '%s\n' "$run" >&2
  echo "rust-coverage: $CRATE's tests failed under instrumentation — fix the failures named above, then re-run." >&2
  exit 1
fi
