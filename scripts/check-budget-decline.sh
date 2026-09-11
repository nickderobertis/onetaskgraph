#!/usr/bin/env bash
# Follow a budget decline through to the conclusion the required check reads.
#
# `tests/budget_gate.rs` asserts the *outcome* on every ordinary run: a journey whose
# account cannot afford it does not run, and says which budget was short, that budget's
# limit, what remained, the estimated cost, the retained buffer and when it resets.
# `tests/recheck_gate.rs` asserts the same of the second reading — a journey the free read
# admitted and its own first real call's headers then refused does not run, and says what
# each reading answered. What neither assertion can show is the other half — that such a
# run leaves the check red rather than green — because a test that asserts a panic passes.
#
# So this re-runs one test of each with ONETASKGRAPH_BUDGET_DECLINE_FOLLOW_THROUGH set,
# which makes it re-raise the very panic the decline made instead of asserting it. Then the
# target fails, `cargo test` exits non-zero, and branch protection accepts that as neither
# success nor in place of success. A run that never happened must never be mergeable.
#
# It reaches no API and reads no credential: every allowance a decline is made on comes
# from a loopback stand-in the test starts.
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

readonly CRATE="onetaskgraph-github-projects"

failures=0
fail() {
  echo "check-budget-decline: $1" >&2
  failures=$((failures + 1))
}

# Assert that `$output`, from a run that exited `$status`, is a red run whose message a
# reader can tell from a code defect: it leads with the run not having happened, names the
# budget, and says it does not wait. `$@` are the figures that particular decline owes.
declined_red() {
  local reading="$1" status="$2" output="$3"
  shift 3
  if [ "$status" -eq 0 ]; then
    fail "a journey that declined on $reading exited 0, so the required check would accept a run that never happened. Output: $output"
  fi
  # The message a person reads an hour later leads with the run not having happened, so a
  # refusal is not sent to somebody debugging a defect in the code under test.
  for phrase in "DID NOT RUN" "not a test failure in the code under test" "graphql"; do
    case "$output" in
      *"$phrase"*) ;;
      *) fail "a journey declined on $reading does not carry \"$phrase\", so a refusal reads as a code defect. Output: $output" ;;
    esac
  done
  # And every figure the decision was made on, so the reader can tell it from a failure
  # without being asked to take the refusal on trust.
  for phrase in "$@"; do
    case "$output" in
      *"$phrase"*) ;;
      *) fail "a journey declined on $reading does not report \"$phrase\". Output: $output" ;;
    esac
  done
  # Nothing waits for a budget to come back: a refusal naming a rate limit while the
  # account's own reported budget still shows room is the secondary limiter, which nothing
  # reports and every further attempt extends.
  case "$output" in
    *"nothing here waits for it"*) ;;
    *) fail "a journey declined on $reading did not say it does not wait for the budget. Output: $output" ;;
  esac
}

follow_through() {
  local target="$1" test="$2"
  # llmlint: ignore-block[work_goes_through_command_surface] This runs as a command OF the
  # command surface — the plugin's Nx `test` target — so `just test` here would re-enter the
  # target running it. It needs one test under one environment variable, which no recipe offers.
  ONETASKGRAPH_BUDGET_DECLINE_FOLLOW_THROUGH=1 \
    cargo test -p "$CRATE" --test "$target" --all-features --locked -- --exact "$test" 2>&1
  # llmlint: ignore-end[work_goes_through_command_surface]
}

status=0
output="$(follow_through budget_gate a_journey_the_account_cannot_afford_does_not_run_and_says_which_budget_was_short)" || status=$?
declined_red "the allowance read" "$status" "$output" \
  "limit is" "remained" "estimated to spend" "retained buffer is" "resets at"

status=0
output="$(follow_through recheck_gate a_journey_whose_first_call_carries_less_than_the_endpoint_claimed_does_not_run)" || status=$?
declined_red "the headers of its first real call" "$status" "$output" \
  "GET /rate_limit" "claimed" "the headers of the session's first real call" "reported" \
  "estimated to spend" "retained buffer is" "resets at"

if [ "$failures" -ne 0 ]; then
  echo "check-budget-decline: $failures expectation(s) failed." >&2
  echo "check-budget-decline: the decision is in crates/onetaskgraph-live (affordable," >&2
  echo "check-budget-decline: still_affordable, RETAINED_BUFFER, Unaffordable) and the reads" >&2
  echo "check-budget-decline: and the cost model are in crates/$CRATE/tests/journey/budget.rs" >&2
  echo "check-budget-decline: — fix whichever the failure above names, then re-run this check." >&2
  exit 1
fi
