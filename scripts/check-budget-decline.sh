#!/usr/bin/env bash
# Follow a budget decline through to the conclusion the required check reads.
#
# `tests/budget_gate.rs` asserts the *outcome* on every ordinary run: a journey whose
# account cannot afford it does not run, and says which budget was short, that budget's
# limit, what remained, the estimated cost, the retained buffer and when it resets. What an
# assertion cannot show is the other half — that such a run leaves the check red rather
# than green — because a test that asserts a panic passes.
#
# So this re-runs that one test with ONETASKGRAPH_BUDGET_DECLINE_FOLLOW_THROUGH set, which
# makes it re-raise the very panic the decline made instead of asserting it. Then the target
# fails, `cargo test` exits non-zero, and branch protection accepts that as neither success
# nor in place of success. A run that never happened must never be mergeable.
#
# It reaches no API and reads no credential: the allowance the decline is made on comes from
# a loopback stand-in the test starts.
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

readonly CRATE="onetaskgraph-github-projects"
readonly TARGET="budget_gate"
readonly TEST="a_journey_the_account_cannot_afford_does_not_run_and_says_which_budget_was_short"

failures=0
fail() {
  echo "check-budget-decline: $1" >&2
  failures=$((failures + 1))
}

status=0
# llmlint: ignore-block[work_goes_through_command_surface] This runs as a command OF the
# command surface — the plugin's Nx `test` target — so `just test` here would re-enter the
# target running it. It needs one test under one environment variable, which no recipe offers.
output="$(ONETASKGRAPH_BUDGET_DECLINE_FOLLOW_THROUGH=1 \
  cargo test -p "$CRATE" --test "$TARGET" --all-features --locked -- --exact "$TEST" 2>&1)" || status=$?
# llmlint: ignore-end[work_goes_through_command_surface]

if [ "$status" -eq 0 ]; then
  fail "a journey that declined for want of budget exited 0, so the required check would accept a run that never happened. Output: $output"
fi

# The message a person reads an hour later leads with the run not having happened, so a
# refusal is not sent to somebody debugging a defect in the code under test.
for phrase in "DID NOT RUN" "not a test failure in the code under test" "graphql"; do
  case "$output" in
    *"$phrase"*) ;;
    *) fail "a declined journey's output does not carry \"$phrase\", so a refusal reads as a code defect. Output: $output" ;;
  esac
done

# And every figure the decision was made on, so the reader can tell it from a failure
# without being asked to take the refusal on trust.
for phrase in "limit is" "remained" "estimated to spend" "retained buffer is" "resets at"; do
  case "$output" in
    *"$phrase"*) ;;
    *) fail "a declined journey's output does not report \"$phrase\". Output: $output" ;;
  esac
done

# Nothing waits for a budget to come back: a refusal naming a rate limit while the account's
# own reported budget still shows room is the secondary limiter, which nothing reports and
# every further attempt extends.
case "$output" in
  *"nothing here waits for it"*) ;;
  *) fail "a declined journey did not say it does not wait for the budget. Output: $output" ;;
esac

if [ "$failures" -ne 0 ]; then
  echo "check-budget-decline: $failures expectation(s) failed." >&2
  echo "check-budget-decline: the decision is in crates/onetaskgraph-live (affordable," >&2
  echo "check-budget-decline: RETAINED_BUFFER, Unaffordable) and the read and the cost model" >&2
  echo "check-budget-decline: are in crates/$CRATE/tests/journey/budget.rs — fix whichever" >&2
  echo "check-budget-decline: the failure above names, then re-run this check." >&2
  exit 1
fi
