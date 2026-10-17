#!/usr/bin/env bash
# A live crate's tests, as its `test` target runs them.
#
#   scripts/live-crate-test.sh <crate>
#
# Every test of the crate, ordinarily. With ONETASKGRAPH_TESTS_UNDER_COVERAGE=1 — which
# scripts/ci-job.sh sets on the platforms where the instrumented run stands in for `test` —
# only the crate's credentialed journey: its `coverage` target runs the whole crate under
# nextest anyway (with the credentials cleared, so the journey skips there), and the journey is
# the one test that has to run here, credentialed. Every case still runs once, in the same
# job. scripts/check-test-in-coverage.sh holds the table below to the crates' live journeys
# and their coverage runs, so a non-live test cannot end up run by neither target.
set -euo pipefail

readonly CRATE="${1:?usage: scripts/live-crate-test.sh <crate>}"

# The crate's credentialed journey: the test binary that holds it, and the one test in it
# that opens a live session. Everything else of the binary is the coverage run's.
case "$CRATE" in
  onetaskgraph-github-projects) live=(live real_projects_v2_contract_writes_and_leaves_no_residue) ;;
  onetaskgraph-github-projects-e2e) live=(live disposable_task_and_document_asset_copies) ;;
  onetaskgraph-linear) live=(live real_linear_applies_every_declared_capability_and_leaves_no_residue) ;;
  onetaskgraph-linear-e2e) live=(e2e linear_assets::live_disposable_issue_and_document_assets_copy_and_recopy) ;;
  *)
    echo "live-crate-test: $CRATE is not a crate with a live journey this script knows." >&2
    echo "live-crate-test: next: add its live journey to the table here, which check-test-in-coverage.sh holds to the crate." >&2
    exit 2
    ;;
esac

case "${ONETASKGRAPH_TESTS_UNDER_COVERAGE:-}" in
  1) exec cargo test --quiet -p "$CRATE" --all-features --locked --test "${live[0]}" -- --exact "${live[1]}" ;;
  "") ;;
  *)
    echo "live-crate-test: ONETASKGRAPH_TESTS_UNDER_COVERAGE is '$ONETASKGRAPH_TESTS_UNDER_COVERAGE'; it is 1 or unset." >&2
    echo "live-crate-test: next: unset it to run the whole crate, or set it to 1 where coverage runs the rest." >&2
    exit 2
    ;;
esac

# The e2e suite's live_pair binary runs the plugin's own test binaries, which nothing else
# guarantees are built when this suite runs without the plugin's test target beside it.
if [ "$CRATE" = onetaskgraph-github-projects-e2e ]; then
  cargo test --quiet -p onetaskgraph-github-projects --all-features --locked --no-run
fi
exec cargo test --quiet -p "$CRATE" --all-features --locked
