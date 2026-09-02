#!/usr/bin/env bash
# llmlint: ignore-file[new_code_lands_in_a_project] scripts/ is deliberately outside the
# project graph, as every other guard here is: Nx maps no project to it, which is why
# `just script-check` runs these outside Nx. This one is nevertheless a command of each
# hosted plugin's own `test` target, so it runs when that plugin is affected.
#
# Drive one plugin's live journey through all three of its outcomes, reaching no API.
#
# 1. **Skipped** — no credential, and none expected: exit zero with the reason printed.
# 2. **Expected and absent** — `ONETASKGRAPH_LIVE_REQUIRED=1` turns that skip into a
#    failure naming the variable, so the required check cannot pass green for a missing one.
# 3. **Declined** — it could have run and a precondition refused it, so it tested nothing.
#    The run fails, which is a conclusion branch protection accepts neither as success nor
#    in place of it; and its first line says the tests DID NOT RUN, so it is not read as a
#    code defect. This is the outcome this check exists for, and the precondition producing
#    it here — a seat another instance holds — needs nothing of GitHub's. A later one
#    declines by the same route, so what is proven is the wiring rather than one reason.
#
# Every run below gets a placeholder credential and a scratch seat directory, so nothing
# here reaches a real API.
#
# Usage: scripts/check-live-decline.sh <crate> <seat-file-name>
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

readonly CRATE="${1:?usage: scripts/check-live-decline.sh <crate> <seat-file-name>}"
readonly SEAT_FILE="${2:?usage: scripts/check-live-decline.sh <crate> <seat-file-name>}"

# The argument names a crate of this workspace with a live journey in it; it is not a path
# the caller chooses. Matched against the real tree before it reaches Cargo.
if [ ! -f "crates/$CRATE/tests/live.rs" ]; then
  echo "check-live-decline: $CRATE has no crates/$CRATE/tests/live.rs, so it has no live journey." >&2
  echo "check-live-decline: pass the name of a crate that does." >&2
  exit 1
fi
case "$SEAT_FILE" in
  *[!a-z0-9.-]* | "" | *..*)
    echo "check-live-decline: $SEAT_FILE is not a seat file name (lowercase, digits, dots and hyphens)." >&2
    echo "check-live-decline: pass the name onetaskgraph_live::Session uses for this session's seat." >&2
    exit 1
    ;;
esac

scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT

failures=0
fail() {
  echo "check-live-decline: $1" >&2
  failures=$((failures + 1))
}

# Placeholders, so the journey's own nomination decision says "run" and the outcome under
# test is the one this case is about. None of them is a credential and none is ever sent:
# every case below stops before the journey's first request.
#
# llmlint: ignore[work_goes_through_command_surface] This function is called BY the command
# surface: it is a command of each hosted plugin's own Nx `test` target, so routing it back
# through `just test` would re-enter the target that is running it. What it needs is one
# test binary run three times under three environments, which is a narrower thing than any
# recipe offers — and the recipe that comes closest, `just test`, would select every
# affected project rather than the one crate whose three outcomes are under test.
run_journey() {
  local seats="$1" demand="$2"
  shift 2
  env \
    ONETASKGRAPH_LIVE_SEAT_DIR="$seats" \
    ONETASKGRAPH_LIVE_REQUIRED="$demand" \
    "$@" \
    cargo test -p "$CRATE" --test live --all-features --locked -- --nocapture 2>&1
}

# --nocapture: a skip is reported by the journey printing why, and the harness discards
# what a passing test printed. A skip nobody can read is indistinguishable from a journey
# that ran and asserted nothing, which is the one thing this check may not confuse.

# 1. A declined session: the seat is already held, so the journey did not run.
held="$scratch/held"
mkdir -p "$held"
printf 'held by scripts/check-live-decline.sh\n' > "$held/$SEAT_FILE"
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
  *"already running"*) ;;
  *) fail "a declined session did not say why it declined. Output: $declined_output" ;;
esac
if [ ! -f "$held/$SEAT_FILE" ]; then
  fail "a declined session removed the seat another run holds, so the next instance would race it."
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
  echo "check-live-decline: reported by crates/$CRATE/tests/live.rs — fix whichever the" >&2
  echo "check-live-decline: failure above names, then re-run this check." >&2
  exit 1
fi
