#!/usr/bin/env bash
# One pull-request job's checks, over the projects the diff against NX_BASE reaches.
#
#   scripts/ci-job.sh <lint|test-linux|test-windows|test-macos> [--select-only] [--exclude=<projects>]
#
# Each of ci.yml's four jobs runs exactly one `just ci-<job>` recipe, and each recipe is this
# script, so a contributor runs what CI runs. A job is a list of phases, each one Nx
# invocation. Before running a phase it prints every task that phase selects; after, every
# task Nx ran; on failure, the whole of Nx's output. `--select-only` prints the selection and
# runs nothing, which is how a host that is not Windows or macOS proves those jobs' selection.
#
# `--exclude=` is what scripts/live-lane-selection.sh answers through lane_runner.py: a live
# plugin a version-only diff cannot reach. It narrows the phase that runs `test` and
# `budgets`, exactly as it narrows `just test`; coverage is never narrowed by it.
#
# The tests run once. A project tagged `test-in-coverage:<platforms>` has a `coverage` target
# that runs its whole test suite instrumented, so on those platforms its `test` is not run a
# second time uninstrumented; its `test-uncovered` target runs whatever the instrumented run
# cannot (doctests, a default-feature build). `unix` means every platform but Windows, where
# scripts/rust-coverage.sh measures nothing; `all` means every platform.
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

usage() {
  echo "usage: scripts/ci-job.sh <lint|test-linux|test-windows|test-macos> [--select-only] [--exclude=<projects>]" >&2
  echo "ci-job: next: name one of the four jobs ci.yml runs, e.g. 'just ci-test-linux'" >&2
  exit 2
}

[ "$#" -ge 1 ] || usage
readonly JOB="$1"
shift
select_only=false
live_exclusions=""
for argument in "$@"; do
  case "$argument" in
    --select-only) select_only=true ;;
    --exclude=*) live_exclusions="${argument#--exclude=}" ;;
    *) usage ;;
  esac
done

# One phase per line: `affected|run`, the comma-separated targets or task, and the projects
# that phase excludes (or `-`).
case "$JOB" in
  lint)
    phases="affected format-check,lint,typecheck -
run sdk-python:generate-check -
run workspace:deny -"
    ;;
  test-linux | test-macos)
    phases="affected test,budgets tag:test-in-coverage:*
affected test-uncovered,coverage -
run scripts:distribution-check -
run scripts:distribution-test -"
    ;;
  test-windows)
    phases="affected test,budgets tag:test-in-coverage:all
affected coverage -
run scripts:distribution-check -
run scripts:distribution-test -"
    ;;
  *) usage ;;
esac

if [ ! -x node_modules/.bin/nx ]; then
  # scripts/nx.sh heals a fresh checkout; ask it for nothing but the install.
  bash scripts/nx.sh --version >/dev/null
fi
PYTHON="$(command -v python3)" || {
  echo "ci-job: python3 is not on PATH, and it reads the task graph Nx prints." >&2
  echo "ci-job: next: install Python 3 (bootstrap provides it), then re-run." >&2
  exit 1
}

scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT

# The tasks an Nx invocation selects, one `project:target` per line, from its task graph.
selected_tasks() {
  if ! node_modules/.bin/nx "$@" --graph=stdout >"$scratch/graph.json" 2>"$scratch/graph.err"; then
    cat "$scratch/graph.err" >&2
    echo "ci-job: Nx could not plan '$*'; fix what it reports above, then re-run." >&2
    return 1
  fi
  "$PYTHON" -c '
import json, sys
with open(sys.argv[1], encoding="utf-8") as handle:
    document = json.load(handle)
tasks = document.get("tasks", {}).get("tasks", {})
sys.stdout.reconfigure(newline="\n")
for name in sorted(tasks):
    print(name)
' "$scratch/graph.json"
}

status=0
while IFS=' ' read -r kind targets excluded; do
  [ -n "$kind" ] || continue
  exclusions=""
  [ "$excluded" = "-" ] || exclusions="$excluded"
  if [ "$kind" = "affected" ] && [ -n "$live_exclusions" ] && [[ ",$targets," == *",test,"* ]]; then
    exclusions="${exclusions:+$exclusions,}$live_exclusions"
  fi
  if [ "$kind" = "affected" ]; then
    read -r -a target_words <<<"${targets//,/ }"
    arguments=(affected -t "${target_words[@]}")
    [ -z "$exclusions" ] || arguments+=("--exclude=$exclusions")
  else
    arguments=(run "$targets")
  fi
  if ! plan="$(selected_tasks "${arguments[@]}" | tr -d '\r')"; then
    exit 1
  fi
  if [ -z "$plan" ]; then
    echo "ci-job: $JOB: nx ${arguments[*]} selects nothing"
    continue
  fi
  printf '%s\n' "$plan" | sed "s/^/ci-job: $JOB: selects /"
  if "$select_only"; then
    continue
  fi
  if FORCE_COLOR=0 NO_COLOR=1 node_modules/.bin/nx "${arguments[@]}" --outputStyle=static >"$scratch/run.log" 2>&1; then
    sed -n 's/^.*> nx run \([^ ]*\).*$/\1/p' "$scratch/run.log" | tr -d '\r' | sort -u \
      | sed "s/^/ci-job: $JOB: ran /"
  else
    cat "$scratch/run.log" >&2
    echo "ci-job: $JOB: nx ${arguments[*]} failed — the failing tasks are named above." >&2
    echo "ci-job: next: run 'just ci-$JOB' again after fixing them; every phase still runs, so one report names every failure." >&2
    status=1
  fi
done <<EOF
$phases
EOF
exit "$status"
