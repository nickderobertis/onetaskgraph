#!/usr/bin/env bash
# A live crate's tests, as its `test` target runs them.
#
#   scripts/live-crate-test.sh <crate> <test-binary> <test> [--also-build <package>]
#
# Every test of the crate, which is what every job runs: the live crates are tagged
# coverage:uninstrumented, so their whole suite, credentialed journey included, runs here.
# With ONETASKGRAPH_TESTS_UNDER_COVERAGE=1, which no job sets, only the crate's credentialed
# journey runs, the one `<test>` of `<test-binary>` that opens a live session, for a caller
# whose instrumented run covers the rest. Each crate names its journey in its own `test`
# target, and scripts/check-test-in-coverage.sh holds what each names to its live journeys
# and refuses a job that narrows an uninstrumented crate's `test` that way.
#
# `--also-build <package>` builds that package's test binaries first, for a crate whose whole
# suite runs them; the credentialed journey alone needs none of them.
set -euo pipefail

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
