#!/usr/bin/env bash
# A live crate's tests, as its `test` target runs them.
#
#   scripts/live-crate-test.sh <crate> <test-binary> <test> [--also-build <package>]
#
# Every test of the crate, ordinarily. With ONETASKGRAPH_TESTS_UNDER_COVERAGE=1 — which
# scripts/ci-job.sh sets on the platforms where the instrumented run stands in for `test` —
# only the crate's credentialed journey, the one `<test>` of `<test-binary>` that opens a live
# session: its `coverage` target runs the whole crate under nextest anyway (with the
# credentials cleared, so the journey skips there), and the journey is the one test that has to
# run here, credentialed. Every case still runs once, in the same job. Each crate names its
# journey in its own `test` target, and scripts/check-test-in-coverage.sh holds what each names
# to the crate's live journeys and its coverage run, so a non-live test cannot end up run by
# neither target.
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

case "${ONETASKGRAPH_TESTS_UNDER_COVERAGE:-}" in
  1) exec cargo test --quiet -p "$CRATE" --all-features --locked --test "$BINARY" -- --exact "$JOURNEY" ;;
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
