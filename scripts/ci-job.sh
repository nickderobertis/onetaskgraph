#!/usr/bin/env bash
# One pull-request job's checks, over the projects the diff against NX_BASE reaches.
#
#   scripts/ci-job.sh <lint|test-linux|test-windows|test-macos> [--select-only] [--exclude=<projects>]
#
# Each of ci.yml's four jobs runs exactly one `just ci-<job>` recipe, and each recipe is this
# script, so a contributor runs what CI runs. A job ends with one line naming every task it
# ran, or with every failing phase's Nx output; `--select-only` runs nothing and ends with one
# line naming every task the job selects.
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
concurrent=true
case "$JOB" in
  lint)
    phases="affected format-check,lint,typecheck -
run sdk-python:generate-check -
run workspace:deny -"
    ;;
  test-linux | test-macos)
    # And the live crates' `test` runs only their credentialed journeys, coverage running the
    # rest of them, as it does every tagged crate's (scripts/live-crate-test.sh).
    export ONETASKGRAPH_TESTS_UNDER_COVERAGE=1
    # Each phase runs beside the others, so work that would queue behind a long one gets a
    # phase of its own: the two Linear crates, whose credentialed journeys take one seat and so
    # run one at a time, and the SDKs' coverage, which would otherwise wait for every crate's.
    phases="affected test,budgets tag:test-in-coverage:*,onetaskgraph-linear,onetaskgraph-linear-e2e
affected test,budgets *,!onetaskgraph-linear,!onetaskgraph-linear-e2e
affected test-uncovered,coverage tag:layer:sdk
affected coverage *,!tag:layer:sdk
run scripts:distribution-check -
run scripts:distribution-test -"
    ;;
  test-windows)
    # In order rather than at once: on the Windows runner the phases' builds and journeys
    # beside one another took several times longer each, and timing-sensitive loopback
    # journeys failed under the load.
    concurrent=false
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

# The tasks an Nx invocation selects, one `project:target` per line, from its task graph:
# selected_tasks <phase> <nx arguments...>.
selected_tasks() {
  local phase="$1"
  shift
  if ! node_modules/.bin/nx "$@" --graph=stdout >"$scratch/graph.$phase.json" 2>"$scratch/graph.$phase.err"; then
    cat "$scratch/graph.$phase.err" >&2
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
 ' "$scratch/graph.$phase.json"; then
    echo "ci-job: Nx answered '$*' with something other than a task graph; check the Nx version node_modules holds, then re-run." >&2
    return 1
  fi
}

# One phase, in the background beside the job's others: its selection, then its run, each
# recorded under its index for the summary below. run_phase <index> <kind> <targets> <excluded>.
run_phase() {
  local phase="$1" kind="$2" targets="$3" excluded="$4" exclusions="" plan phase_status
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
  echo "nx ${arguments[*]}" >"$scratch/command.$phase"
  if ! plan="$(selected_tasks "$phase" "${arguments[@]}" | tr -d '\r')"; then
    return 1
  fi
  if "$select_only"; then
    [ -z "$plan" ] || printf '%s\n' "$plan" >"$scratch/selected.$phase"
    return 0
  fi
  [ -n "$plan" ] || return 0
  # In Actions the output streams as each task writes it (Nx's `stream` style), tagged with
  # its phase because the phases run at once, so a job the timeout cuts short still shows how
  # far each task got and when. Elsewhere it is held until the phase ends.
  if [ "${GITHUB_ACTIONS:-}" = true ]; then
    # Nx's status is written from inside the pipeline: under `pipefail` a failing Nx would
    # otherwise end this phase before it is recorded and the failure named.
    {
      FORCE_COLOR=0 NO_COLOR=1 node_modules/.bin/nx "${arguments[@]}" --outputStyle=stream 2>&1 \
        && echo 0 >"$scratch/status.$phase" || echo "$?" >"$scratch/status.$phase"
    } | tee "$scratch/run.$phase.log" \
      | awk -v tag="[$targets] " '/^::endgroup::/ { next } { sub(/^::group::/, ""); print tag $0; fflush() }'
    phase_status="$(cat "$scratch/status.$phase")"
  else
    FORCE_COLOR=0 NO_COLOR=1 node_modules/.bin/nx "${arguments[@]}" --outputStyle=static \
      >"$scratch/run.$phase.log" 2>&1 && phase_status=0 || phase_status=$?
  fi
  sed -n 's/^.*> nx run \([^ ]*\).*$/\1/p' "$scratch/run.$phase.log" | tr -d '\r' >"$scratch/ran.$phase"
  return "$phase_status"
}

record_phase() {
  if run_phase "$@"; then echo 0 >"$scratch/exit.$1"; else echo 1 >"$scratch/exit.$1"; fi
}

# Every phase of a job at once, unless the job says otherwise: they select disjoint work, and
# the instrumented build is cargo-llvm-cov's own target directory, so it and the plain build
# do not wait on one lock. Each phase's exit status is kept under its index.
phase=0
while IFS=' ' read -r kind targets excluded; do
  [ -n "$kind" ] || continue
  phase=$((phase + 1))
  if "$concurrent"; then
    record_phase "$phase" "$kind" "$targets" "$excluded" &
  else
    record_phase "$phase" "$kind" "$targets" "$excluded"
  fi
done <<EOF
$phases
EOF
wait
status=0
for index in $(seq 1 "$phase"); do
  phase="$index"
  [ "$(cat "$scratch/exit.$phase")" != 0 ] || continue
  status=1
  [ "${GITHUB_ACTIONS:-}" = true ] || cat "$scratch/run.$phase.log" >&2 2>/dev/null || true
  echo "ci-job: $JOB: $(cat "$scratch/command.$phase" 2>/dev/null || echo "phase $phase") failed — the failing tasks are named above." >&2
done
if [ "$status" -ne 0 ]; then
  echo "ci-job: next: run 'just ci-$JOB' again after fixing them; every phase runs to the end, so one report names every failure." >&2
fi
summary() {
  cat "$scratch"/"$1".* 2>/dev/null | sort -u | tr '\n' ' ' | sed 's/ $//'
}
if "$select_only"; then
  [ "$status" -ne 0 ] || echo "ci-job: $JOB: selects $(summary selected)"
else
  ran="$(summary ran)"
  [ -z "$ran" ] || echo "ci-job: $JOB: ran $ran"
fi
exit "$status"
