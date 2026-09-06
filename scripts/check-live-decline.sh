#!/usr/bin/env bash
# Drive one plugin's live journey through all three of its outcomes, reaching no API.
#
# 1. **Declined** — a precondition refused a run that could have happened, so it tested
#    nothing. The run fails, which branch protection accepts neither as success nor in place
#    of it, and its first line says the tests DID NOT RUN so it is not read as a code defect.
#    This is the outcome the check exists for.
# 2. **Skipped** — no credential, and none expected: exit zero with the reason printed.
# 3. **Expected and absent** — `ONETASKGRAPH_LIVE_REQUIRED=1` makes that same skip a failure
#    naming the variable, so the required check cannot pass green for a missing credential.
#
# Every run below gets a placeholder credential and a scratch seat directory, so nothing
# here reaches a real API.
#
# ## Which precondition produces the decline, and why this reads the lane to find out
#
# A lane says at `Session::open` whether it wants `Exclusivity::OneAtATime`. For a lane that
# does, the seat is a precondition this can refuse offline — by pointing
# `ONETASKGRAPH_LIVE_SEAT_DIR` at something that cannot hold a seat at all — so case 1 below
# drives the real journey binary into a real decline. The seat file's own NAME is never
# spelled here: `onetaskgraph_live` decides it, and a check restating it would be a second
# spelling of one contract.
#
# A lane that opens `Exclusivity::Shared` has no seat to refuse, and its decline has to be
# driven by a check of its own. That is not taken on trust either: this refuses such a lane
# unless the crate's own `test` target really runs another `check-*-decline.sh`, so the
# decline case cannot be lost by a lane quietly giving up its seat.
#
# Usage: scripts/check-live-decline.sh <crate>
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

readonly CRATE="${1:?usage: scripts/check-live-decline.sh <crate>}"

# The argument names a crate of this workspace with a live journey in it; it is not a path
# the caller chooses. Held to Cargo's own package-name grammar first, so nothing that is not
# a package name can be spliced into a path or a `-p` selector, and then matched against the
# real tree.
case "$CRATE" in
  *[!a-z0-9_-]* | "" | -*)
    echo "check-live-decline: $CRATE is not a cargo package name (lowercase, digits, hyphens and underscores)." >&2
    echo "check-live-decline: pass the name of a crate of this workspace that has a live journey." >&2
    exit 1
    ;;
esac
readonly JOURNEY="crates/$CRATE/tests/live.rs"
if [ ! -f "$JOURNEY" ]; then
  echo "check-live-decline: $CRATE has no $JOURNEY, so it has no live journey." >&2
  echo "check-live-decline: pass the name of a crate that does." >&2
  exit 1
fi

scratch="$(mktemp -d)" || {
  echo "check-live-decline: could not create a scratch directory for the seats and logs." >&2
  echo "check-live-decline: free space in the temporary directory, or point TMPDIR at a writable one, then re-run." >&2
  exit 1
}
trap 'rm -rf "$scratch"' EXIT

failures=0
fail() {
  echo "check-live-decline: $1" >&2
  failures=$((failures + 1))
}

# Placeholders, so the journey's own nomination decision says "run" and the outcome under
# test is the one this case is about. None of them is a credential and none is ever sent:
# every case below stops before the journey's first request.
run_journey() {
  local seats="$1" demand="$2"
  shift 2
  # llmlint: ignore-block[work_goes_through_command_surface] This runs as a command OF the command surface — each hosted plugin's Nx `test` target — so `just test` here would re-enter the target running it. It needs one test binary under three environments, which no recipe offers.
  env \
    ONETASKGRAPH_LIVE_SEAT_DIR="$seats" \
    ONETASKGRAPH_LIVE_REQUIRED="$demand" \
    "$@" \
    cargo test -p "$CRATE" --test live --all-features --locked -- --nocapture 2>&1
  # llmlint: ignore-end[work_goes_through_command_surface]
}

# --nocapture: a skip is reported by the journey printing why, and the harness discards
# what a passing test printed. A skip nobody can read is indistinguishable from a journey
# that ran and asserted nothing, which is the one thing this check may not confuse.

# 1. A declined session, produced by whichever precondition this lane has.
if grep -q 'Exclusivity::OneAtATime' "$JOURNEY"; then
  # A seat directory that is a FILE: the seat cannot be created there, so the lane is
  # declined before it reaches its API. Nothing here names the seat file — the crate that
  # decides that name is the only place it is spelled.
  held="$scratch/not-a-directory"
  printf 'this is a file, so no seat may be taken inside it\n' > "$held"
  declined_output=""
  declined_status=0
  declined_output="$(run_journey "$held" "" \
    GH_PROJECTS_TOKEN=placeholder-not-a-credential \
    GH_PROJECTS_OWNER=placeholder-owner \
    GH_PROJECTS_NUMBER=1 \
    GH_PROJECTS_REPOSITORY=placeholder-owner/placeholder-repository \
    LINEAR_API_KEY=placeholder-not-a-credential \
    LINEAR_WRITE_TEAM=PLACEHOLDER)" || declined_status=$?
  if [ "$declined_status" -eq 0 ]; then
    fail "a declined session exited 0, so the required check would accept a run that never happened. Output: $declined_output"
  fi
  case "$declined_output" in
    *"DID NOT RUN"*) ;;
    *) fail "a declined session did not say it did not run, so a refusal reads as a code defect. Output: $declined_output" ;;
  esac
  case "$declined_output" in
    *"not a test failure in the code under test"*) ;;
    *) fail "a declined session did not distinguish itself from an ordinary test failure. Output: $declined_output" ;;
  esac
  case "$declined_output" in
    *"seat"*) ;;
    *) fail "a declined session did not say why it declined. Output: $declined_output" ;;
  esac
  case "$declined_output" in
    *ONETASKGRAPH_LIVE_SEAT_DIR*) ;;
    *) fail "a declined session did not name what a reader can change to make it run. Output: $declined_output" ;;
  esac
elif grep -q 'Exclusivity::Shared' "$JOURNEY"; then
  # No seat, so no decline this check can produce — but the lane must still HAVE one that
  # some check drives to a red result, or the outcome above stops being proven for it.
  targets="crates/$CRATE/project.json"
  driven="$(grep -o 'scripts/check-[a-z-]*-decline\.sh' "$targets" | grep -v 'check-live-decline\.sh' | head -1 || true)"
  if [ -z "$driven" ]; then
    fail "$JOURNEY opens a shared session, so no seat can decline it — and $targets runs no other check-*-decline.sh, so nothing drives this lane's decline through to a red result. Add one, or give the lane back a precondition this check can refuse."
  elif [ ! -f "$driven" ]; then
    fail "$targets names $driven, which is not in the tree."
  else
    echo "check-live-decline: $CRATE opens a shared session; its decline is driven by $driven."
  fi
else
  fail "$JOURNEY does not say which Exclusivity it opens its session with, so which precondition can decline it is unknown. Name one at Session::open."
fi

# llmlint: ignore-block[live_tier_compiles_and_requires_credential] Case 2 asserting a green
# exit is this repository's decision rather than an oversight: no credential was expected
# there, which is a contributor with no keys and a pull request from a fork, to which GitHub
# supplies no secrets at all. Case 3 immediately below is the other half — the same absent
# credential where one WAS expected — and it asserts the run is red and names what demanded
# it, which is the demand this rule exists for. Removing case 2 would not add a demand; it
# would only stop anybody proving that a fork pull request still reads honestly.
# 2. No credential and none expected: a skip, with the reason, and nothing red.
free="$scratch/free"
mkdir -p "$free"
skipped_status=0
skipped_output="$(run_journey "$free" "" \
  GH_PROJECTS_TOKEN= LINEAR_API_KEY=)" || skipped_status=$?
if [ "$skipped_status" -ne 0 ]; then
  fail "a run with no credential failed instead of skipping, which is a contributor with no keys and a fork pull request. Output: $skipped_output"
fi
case "$skipped_output" in
  *skipped*) ;;
  *) fail "a run with no credential did not print why it skipped. Output: $skipped_output" ;;
esac
if [ -n "$(ls -A "$free")" ]; then
  fail "a skipped run took a seat it was never going to use: $(ls -A "$free")"
fi

# 3. No credential where one was expected: the required check may not pass green for it.
demanded_status=0
demanded_output="$(run_journey "$free" 1 \
  GH_PROJECTS_TOKEN= LINEAR_API_KEY=)" || demanded_status=$?
if [ "$demanded_status" -eq 0 ]; then
  fail "a run demanded by ONETASKGRAPH_LIVE_REQUIRED=1 passed green with no credential. Output: $demanded_output"
fi
case "$demanded_output" in
  *ONETASKGRAPH_LIVE_REQUIRED*) ;;
  *) fail "a demanded run without a credential did not name what demanded it. Output: $demanded_output" ;;
esac
# llmlint: ignore-end[live_tier_compiles_and_requires_credential]

if [ "$failures" -ne 0 ]; then
  echo "check-live-decline: $failures expectation(s) failed for $CRATE." >&2
  echo "check-live-decline: the three outcomes are decided in crates/onetaskgraph-live and" >&2
  echo "check-live-decline: reported by $JOURNEY — fix whichever the" >&2
  echo "check-live-decline: failure above names, then re-run this check." >&2
  exit 1
fi
