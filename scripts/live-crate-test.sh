#!/usr/bin/env bash
# A live crate's tests, as its `test` target runs them.
#
#   scripts/live-crate-test.sh <crate> <test-binary> <test> [--also-build <package>]
#
# Every test of the crate, ordinarily. With ONETASKGRAPH_TESTS_UNDER_COVERAGE=1 — which
# scripts/ci-job.sh sets on Linux, where the instrumented run stands in for `test` —
# only the crate's credentialed journey, the one `<test>` of `<test-binary>` that opens a live
# session: its `coverage` target runs the whole crate under nextest anyway (with the
# credentials cleared, so the journey skips there), and the journey is the one test that has to
# run here, credentialed. Every case still runs once, in the same job. Each crate names its
# journey in its own `test` target, and scripts/check-test-in-coverage.sh holds what each names
# to the crate's live journeys and its coverage run, so a non-live test cannot end up run by
# neither target. A crate tagged coverage:uninstrumented has no instrumented run to stand in
# (the user's decision of 2026-10-30), so it runs its whole suite whatever the variable says —
# under the variable, with the journey started before the rest is built (below).
#
# `--also-build <package>` builds that package's test binaries first, for a crate whose whole
# suite runs them; the credentialed journey alone needs none of them.
set -euo pipefail
# read_lines, because bash 3.2 has no mapfile; tested before it is sourced, as 3.2 ends the shell
# on a missing file.
if [ ! -f "$(dirname "$0")/read-lines.sh" ]; then
  echo "live-crate-test: scripts/read-lines.sh is missing." >&2
  echo "live-crate-test: next: restore it from git." >&2
  exit 1
fi
# shellcheck source=read-lines.sh
. "$(dirname "$0")/read-lines.sh"

usage() {
  echo "usage: scripts/live-crate-test.sh <crate> <test-binary> <test> [--also-build <package>]" >&2
  echo "live-crate-test: next: name the crate, the test binary holding its credentialed journey, and that journey, as its project.json's test target does." >&2
  exit 2
}

[ "$#" -eq 3 ] || [ "$#" -eq 5 ] || usage
readonly CRATE="$1" BINARY="$2" JOURNEY="$3"
also_build=""
if [ "$#" -eq 5 ]; then
  [ "$4" = --also-build ] || usage
  also_build="$5"
fi
for value in "$CRATE" "$BINARY" "$JOURNEY" ${also_build:+"$also_build"}; do
  case "$value" in
    "" | -*) usage ;;
  esac
done

run_journey() {
  # A crate tagged coverage:uninstrumented has no instrumented run standing in for the rest, so
  # it returns to the whole-suite run below.
  local project
  project="$(dirname "$0")/../crates/$CRATE/project.json"
  if [ ! -f "$project" ]; then
    echo "live-crate-test: $CRATE has no crates/$CRATE/project.json to say whether an instrumented run covers it." >&2
    echo "live-crate-test: next: name the crate as its directory under crates/ spells it." >&2
    exit 2
  fi
  if grep -qF '"coverage:uninstrumented"' "$project"; then
    run_whole_suite_journey_first
  fi
  # `--exact` with a name nothing holds runs no test and passes, so the journey is found first.
  local listed
  listed="$(cargo test --quiet -p "$CRATE" --all-features --locked --test "$BINARY" -- --exact "$JOURNEY" --list)" || exit
  if ! grep -qxF "$JOURNEY: test" <<<"$listed"; then
    echo "live-crate-test: $CRATE's \`$BINARY\` test binary holds no test named $JOURNEY, so nothing credentialed would run." >&2
    echo "live-crate-test: next: name the credentialed journey its project.json's test target runs as it is spelled in tests/." >&2
    exit 2
  fi
  exec cargo test --quiet -p "$CRATE" --all-features --locked --test "$BINARY" -- --exact "$JOURNEY"
}

# The whole crate where nothing instrumented stands in for it, with the credentialed journey
# started first: the journey's binary is built alone and set running, and every other target of
# the crate is built and run while it does, because the journey spends most of its quarter of an
# hour waiting on GitHub's settling margins and the rest of the suite needs nothing of it. Built
# all together, the journey would wait for every other binary to compile and the rest would wait
# for the journey. Every test runs once, the journey in its own `cargo test` — so one session —
# and the targets come from `cargo metadata`, so a target added later is run rather than missed.
run_whole_suite_journey_first() {
  local python_bin targets journey_pid status=0 journey_status=0
  python_bin="$(command -v python3)" || {
    echo "live-crate-test: python3 is not on PATH, and listing $CRATE's test targets needs it." >&2
    echo "live-crate-test: next: install python3 and re-run." >&2
    exit 1
  }
  case $python_bin in
    /* | ?:[\\/]*) ;;
    *) python_bin=$(cd "$(dirname "$python_bin")" && pwd)/$(basename "$python_bin") ;;
  esac
  # tr: a target name is passed to cargo as is, which a trailing CR would defeat.
  targets="$(cargo metadata --no-deps --format-version 1 --locked \
    | "$python_bin" -c '
import json, sys
crate, journey = sys.argv[1:]
packages = [p for p in json.load(sys.stdin)["packages"] if p["name"] == crate]
if len(packages) != 1:
    raise SystemExit(f"cargo metadata names {len(packages)} packages called {crate}")
kinds = [(t["kind"], t["name"]) for t in packages[0]["targets"]]
if not any(kind == ["test"] and name == journey for kind, name in kinds):
    raise SystemExit(f"{crate} has no test target named {journey}")
for kind, name in kinds:
    if kind == ["lib"]:
        print("--lib")
    elif kind == ["bin"]:
        print(f"--bin\n{name}")
    elif kind == ["test"] and name != journey:
        print(f"--test\n{name}")
    elif kind not in (["test"], ["custom-build"], ["example"], ["bench"]):
        raise SystemExit(f"{crate} has a {kind} target {name} this script does not run")
' "$CRATE" "$BINARY" | tr -d '\r')" || {
    echo "live-crate-test: could not list $CRATE's test targets from cargo metadata (above)." >&2
    echo "live-crate-test: next: fix what it reports, so every target of the crate is run." >&2
    exit 1
  }
  local others=()
  read_lines others <<<"$targets"
  if [ -n "$also_build" ]; then
    cargo test --quiet -p "$also_build" --all-features --locked --no-run
  fi
  cargo test --quiet -p "$CRATE" --all-features --locked --test "$BINARY" --no-run
  cargo test --quiet -p "$CRATE" --all-features --locked --test "$BINARY" &
  journey_pid=$!
  cargo test --quiet -p "$CRATE" --all-features --locked ${others[@]+"${others[@]}"} || status=$?
  if [ "$status" -eq 0 ] && [[ " ${others[*]-} " == *" --lib "* ]]; then
    cargo test --quiet -p "$CRATE" --all-features --locked --doc || status=$?
  fi
  # The journey is waited for whatever the rest did: stopping it would cut its own cleanup short.
  wait "$journey_pid" || journey_status=$?
  if [ "$journey_status" -ne 0 ]; then
    exit "$journey_status"
  fi
  exit "$status"
}

# The variable's other reader is telemetry::under_coverage in onetaskgraph-e2e-support, whose
# test holds these three arms to its reading.
case "${ONETASKGRAPH_TESTS_UNDER_COVERAGE:-}" in
  1) run_journey ;;
  "") ;;
  *)
    echo "live-crate-test: ONETASKGRAPH_TESTS_UNDER_COVERAGE is '$ONETASKGRAPH_TESTS_UNDER_COVERAGE'; it is 1 or unset." >&2
    echo "live-crate-test: next: unset it to run the whole crate, or set it to 1 where coverage runs the rest." >&2
    exit 2
    ;;
esac

if [ -n "$also_build" ]; then
  cargo test --quiet -p "$also_build" --all-features --locked --no-run
fi
exec cargo test --quiet -p "$CRATE" --all-features --locked
