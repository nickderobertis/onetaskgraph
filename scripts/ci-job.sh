#!/usr/bin/env bash
# One pull-request job's checks, over the projects the diff against NX_BASE reaches.
#
#   scripts/ci-job.sh <lint|test-linux|test-windows|test-macos> [--select-only] [--exclude=<projects>]
#
# Each of ci.yml's four jobs runs exactly one `just ci-<job>` recipe, and each recipe is this
# script, so a contributor runs what CI runs. A job is a list of phases, each one Nx
# invocation. A phase that fails prints the whole of Nx's output, and the job ends with one
# line naming every task it ran. `--select-only` runs nothing and ends with one line naming
# every task the job selects, which is how a host that is not Windows or macOS proves those
# jobs' selection.
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
# Absolute, so a relative PATH entry such as a virtualenv's still names it (AGENTS.md).
case "$PYTHON" in
  /*) ;;
  *) PYTHON="$(cd "$(dirname "$PYTHON")" && pwd)/$(basename "$PYTHON")" ;;
esac

scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT

# The tasks an Nx invocation selects, one `project:target` per line, from its task graph.
selected_tasks() {
  if ! node_modules/.bin/nx "$@" --graph=stdout >"$scratch/graph.json" 2>"$scratch/graph.err"; then
    cat "$scratch/graph.err" >&2
    echo "ci-job: Nx could not plan '$*'; fix what it reports above, then re-run." >&2
    return 1
  fi
  # The envelope is held to its shape: a graph missing its task map is Nx answering something
  # else, never a phase with nothing to run.
  if ! "$PYTHON" -c '
import json, sys
try:
    with open(sys.argv[1], encoding="utf-8") as handle:
        document = json.load(handle)
    tasks = document["tasks"]["tasks"]
    if not isinstance(tasks, dict) or not all(isinstance(name, str) and ":" in name for name in tasks):
        raise TypeError("tasks.tasks is not a map of project:target names")
except (OSError, ValueError, KeyError, TypeError) as error:
    raise SystemExit(f"ci-job: Nx printed no task graph ({type(error).__name__}: {error})")
sys.stdout.reconfigure(newline="\n")
for name in sorted(tasks):
    print(name)
' "$scratch/graph.json"; then
    echo "ci-job: Nx answered '$*' with something other than a task graph; check the Nx version node_modules holds, then re-run." >&2
    return 1
  fi
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
  if "$select_only"; then
    [ -z "$plan" ] || printf '%s\n' "$plan" >>"$scratch/selected"
    continue
  fi
  [ -n "$plan" ] || continue
  if FORCE_COLOR=0 NO_COLOR=1 node_modules/.bin/nx "${arguments[@]}" --outputStyle=static >"$scratch/run.log" 2>&1; then
    sed -n 's/^.*> nx run \([^ ]*\).*$/\1/p' "$scratch/run.log" | tr -d '\r' >>"$scratch/ran"
  else
    cat "$scratch/run.log" >&2
    echo "ci-job: $JOB: nx ${arguments[*]} failed — the failing tasks are named above." >&2
    echo "ci-job: next: run 'just ci-$JOB' again after fixing them; every phase still runs, so one report names every failure." >&2
    status=1
  fi
done <<EOF
$phases
EOF
# One line for the whole job: which tasks it proved, which is what a reader of a green job
# needs to know of it.
if "$select_only"; then
  echo "ci-job: $JOB: selects $( { sort -u "$scratch/selected" 2>/dev/null || true; } | tr '\n' ' ' | sed 's/ $//')"
elif [ -s "$scratch/ran" ]; then
  echo "ci-job: $JOB: ran $(sort -u "$scratch/ran" | tr '\n' ' ' | sed 's/ $//')"
fi
exit "$status"
