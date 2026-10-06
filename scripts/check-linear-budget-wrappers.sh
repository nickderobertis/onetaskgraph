#!/usr/bin/env bash
# Prove the two scripts the Linear request budgets run through compose with the real
# onebudgetspec: scripts/check-linear-budgets.sh, the Linear crate's `budget-check` target, and
# scripts/linear-budget.sh, the command each budget of crates/onetaskgraph-linear/budgets.yaml
# names.
#
# Both run from a scratch tree of their own, holding the real budgets.yaml and this
# workspace's own installed onebudgetspec, with a stand-in `cargo` first on PATH in place of the
# journey. The stand-in is the measurement's collaborator, not what is under test: it records
# which journey it was asked for and reports the figure the case gives it, so what is proven is
# that the budget id selects its journey, that the figure reaches onebudgetspec through
# ONEBUDGETSPEC_RESULT, and that each failure — a journey that fails, one that reports nothing,
# a figure over its threshold, a tool that is not installed — fails the check naming itself.
set -euo pipefail

fatal() {
  echo "check-linear-budget-wrappers: $1" >&2
  echo "check-linear-budget-wrappers: next: $2" >&2
  exit 1
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)" || fatal \
  "could not resolve this repository's root from ${BASH_SOURCE[0]}" \
  "run the check from a checkout of this repository, as 'just script-check' does"
readonly ROOT
readonly BUDGETS="crates/onetaskgraph-linear/budgets.yaml"

[ -x "$ROOT/node_modules/.bin/onebudgetspec" ] || fatal \
  "onebudgetspec is not installed in this worktree, so the wrappers cannot be driven against it" \
  "run 'just bootstrap', which installs the locked Node toolchain, then rerun"

scratch="$(mktemp -d)" || fatal \
  "could not create the scratch tree the wrappers run from" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
trap 'rm -rf "$scratch"' EXIT

tree="$scratch/tree"
mkdir -p "$tree/scripts" "$tree/crates/onetaskgraph-linear" "$scratch/bin" || fatal \
  "could not lay out the scratch tree under $scratch" \
  "check 'df -h' for free space, then rerun"
for script in scripts/check-linear-budgets.sh scripts/linear-budget.sh "$BUDGETS"; do
  cp "$ROOT/$script" "$tree/$script" || fatal \
    "could not copy $script into the scratch tree" \
    "restore it with 'git checkout -- $script', then rerun"
done
ln -s "$ROOT/node_modules" "$tree/node_modules" || fatal \
  "could not link this worktree's node_modules into the scratch tree" \
  "check 'df -h' for free space, then rerun"

# The journey's stand-in: it records each journey it was asked to run, then behaves as the
# case says — reports a figure, reports nothing, or fails.
cat > "$scratch/bin/cargo" <<'CARGO' || fatal "could not write the stand-in cargo under $scratch/bin" "check 'df -h' for free space, then rerun"
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$STANDIN_CALLS"
case "$STANDIN_MODE" in
  report) printf '{"value": %s}\n' "$STANDIN_VALUE" > "$ONEBUDGETSPEC_RESULT" ;;
  silent) ;;
  fail)
    echo "assertion failed: a status write sent a request it did not before"
    exit 101
    ;;
esac
CARGO
chmod +x "$scratch/bin/cargo" || fatal \
  "could not make the stand-in cargo under $scratch/bin executable" \
  "check the permissions of \$TMPDIR, then rerun"

failures=0
fail() {
  echo "check-linear-budget-wrappers: $1" >&2
  failures=$((failures + 1))
}

# Runs one wrapper in the scratch tree; leaves its exit status in `status`, its combined
# output in `output`, and the journeys the stand-in was asked for in `calls`.
run() {
  local mode="$1" value="$2"
  shift 2
  : > "$scratch/calls"
  status=0
  output="$(cd "$tree" && PATH="$scratch/bin:$PATH" STANDIN_CALLS="$scratch/calls" \
    STANDIN_MODE="$mode" STANDIN_VALUE="$value" "$@" 2>&1)" || status=$?
  calls="$(cat "$scratch/calls")"
}

# Within every threshold: quiet, and each of the four budgets ran its own journey.
run report 1 bash scripts/check-linear-budgets.sh
[ "$status" -eq 0 ] || fail "every figure within its threshold, but the check exited $status: $output"
[ -z "$output" ] || fail "every figure within its threshold, but the check was not quiet: $output"
for budget in linear-requests-per-status-write-cold linear-requests-per-status-write-warm \
  linear-requests-per-settlement-update-cold linear-requests-per-settlement-update-warm; do
  journey="linear_budget::measure_$(printf '%s' "$budget" | tr '-' '_')"
  case "$calls" in
    *"-- $journey --exact"*) ;;
    *) fail "budget $budget did not run its journey $journey; the stand-in was asked for: $calls" ;;
  esac
done

# A figure over a threshold reaches onebudgetspec and fails the check with its report.
run report 9 bash scripts/check-linear-budgets.sh
[ "$status" -ne 0 ] || fail "a figure of 9 is over every threshold, but the check passed"
case "$output" in
  *"is over its threshold or could not be measured"*) ;;
  *) fail "a figure over its threshold did not name the over-budget failure: $output" ;;
esac
case "$output" in
  *linear-requests-per-status-write-warm*) ;;
  *) fail "a figure over its threshold did not carry onebudgetspec's report naming the budget: $output" ;;
esac

# A journey that fails fails the measurement with the journey's own output and its name.
run fail 1 bash scripts/check-linear-budgets.sh
[ "$status" -ne 0 ] || fail "every journey failed, but the check passed"
case "$output" in
  *"a status write sent a request it did not before"*) ;;
  *) fail "a failing journey's own output did not reach the report: $output" ;;
esac

# Each wrapper case on its own, without onebudgetspec in front of it.
run report 1 env ONEBUDGETSPEC_RESULT="$scratch/result" bash scripts/linear-budget.sh \
  linear-requests-per-settlement-update-cold
[ "$status" -eq 0 ] || fail "a journey that reported, but the measurement exited $status: $output"
[ "$(cat "$scratch/result" 2>/dev/null)" = '{"value": 1}' ] \
  || fail "the journey's figure did not land in ONEBUDGETSPEC_RESULT"

: > "$scratch/result"
run silent 1 env ONEBUDGETSPEC_RESULT="$scratch/result" bash scripts/linear-budget.sh \
  linear-requests-per-status-write-cold
[ "$status" -eq 1 ] || fail "a journey that reported nothing, but the measurement exited $status"
case "$output" in
  *"ran and reported nothing"*) ;;
  *) fail "a journey that reported nothing was not named as no longer the measurement: $output" ;;
esac

run report 1 env ONEBUDGETSPEC_RESULT="$scratch/result" bash scripts/linear-budget.sh \
  linear-requests-per-task-status-write
[ "$status" -eq 2 ] || fail "an id no budget carries, but the measurement exited $status"
[ -z "$calls" ] || fail "an id no budget carries still ran a journey: $calls"

run report 1 env -u ONEBUDGETSPEC_RESULT bash scripts/linear-budget.sh \
  linear-requests-per-status-write-cold
[ "$status" -eq 2 ] || fail "no ONEBUDGETSPEC_RESULT, but the measurement exited $status"
[ -z "$calls" ] || fail "no ONEBUDGETSPEC_RESULT, but a journey still ran: $calls"

# A worktree with no onebudgetspec is refused naming the bootstrap that installs it.
rm "$tree/node_modules"
run report 1 bash scripts/check-linear-budgets.sh
[ "$status" -eq 1 ] || fail "onebudgetspec absent, but the check exited $status"
case "$output" in
  *"just bootstrap"*) ;;
  *) fail "onebudgetspec absent, but the check did not name 'just bootstrap': $output" ;;
esac

if [ "$failures" -ne 0 ]; then
  echo "check-linear-budget-wrappers: $failures case(s) failed — see above" >&2
  echo "check-linear-budget-wrappers: next: fix scripts/check-linear-budgets.sh or scripts/linear-budget.sh so each case reads as stated" >&2
  exit 1
fi
