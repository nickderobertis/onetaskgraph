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
# plugin a version-only diff cannot reach. It narrows the phases that run `test` and
# `budgets`, exactly as it narrows `just test`; coverage is never narrowed by it.
#
# The tests run once. Coverage is measured on Linux alone: there a project tagged
# `test-in-coverage:<crate|sdk>` has a `coverage` target that runs its whole test suite
# instrumented, so its `test` is not run a second time uninstrumented, and its `test-uncovered`
# target runs whatever the instrumented run cannot (doctests, a default-feature build).
# test-macos and test-windows run every project's `test`, uninstrumented, and no coverage.
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

# One phase per line: the group it runs in, `affected|run`, the comma-separated targets or
# task, the projects that phase excludes (or `-`), and optionally Nx flags of its own,
# comma-separated. Groups run in order, and the phases of one group at once: they select
# disjoint work, and the instrumented build is cargo-llvm-cov's own target directory, so it
# and the plain build do not wait on one lock.
# llmlint: ignore-block[work_goes_through_command_surface] Each phase is Nx's own invocation because this script reads its task graph to select, stream and summarise the phase, and runs phases side by side; the just recipes for these tasks are single `nx run` wrappers (generate-check adds only output buffering), and this script is itself the body of the ci-* recipes.
case "$JOB" in
  lint)
    phases="1 affected format-check,lint,typecheck -
1 run sdk-python:generate-check -
1 run workspace:deny -"
    ;;
  test-linux)
    # And the live crates' `test` runs only their credentialed journeys, coverage running the
    # rest of them, as it does every tagged crate's (scripts/live-crate-test.sh).
    export ONETASKGRAPH_TESTS_UNDER_COVERAGE=1
    # Exercise the relative interpreter without paying for a second distribution run.
    # check-relative-interpreter.sh owns the call-site comparison.
    export ONETASKGRAPH_DISTRIBUTION_UNDER_RELATIVE_INTERPRETER=1
    # Work that would queue behind a long phase gets one of its own: the two Linear crates,
    # whose credentialed journeys take one seat and so run one at a time, and the SDKs'
    # coverage, which would otherwise wait for every crate's.
    #
    # The budgets are judged last, once coverage has run: the loopback journeys that record
    # their telemetry are coverage's here, and scripts/rust-coverage.sh writes it where
    # `budgets` reads it. Their `test` has already run in this job, so it is not run again,
    # and the cache cannot answer for telemetry its key never saw.
    phases="1 affected test tag:test-in-coverage:*,onetaskgraph-linear,onetaskgraph-linear-e2e
1 affected test *,!onetaskgraph-linear,!onetaskgraph-linear-e2e
1 affected test-uncovered,coverage tag:layer:sdk
1 affected coverage *,!tag:layer:sdk
1 run scripts:distribution-check -
1 run scripts:distribution-test -
2 affected budgets - --excludeTaskDependencies,--skipNxCache"
    ;;
  test-macos)
    # Every suite once, uninstrumented: coverage is Linux's. `budgets` reads what `test`
    # recorded, and depends on it. The two Linear crates have a phase of their own, as on
    # Linux: `onetaskgraph-linear:test` is `parallelism: false` for its seat, so in one Nx run
    # with everything else it waited for every other task to end. No credential reaches this
    # job, so no seat is taken here; that phase holds one of the runner's three slots.
    #
    # The workflow and workflow-policy suites check workflow files whose production runs are
    # Linux jobs, so test-linux runs them; of them only the real rust-cache action also runs on
    # this runner, and so does its journey (.github/policy/tests/inventory.test.mjs holds both).
    phases="1 affected test,budgets onetaskgraph-linear,onetaskgraph-linear-e2e,github-workflows,github-workflow-policy --parallel=2
1 affected test,budgets *,!onetaskgraph-linear,!onetaskgraph-linear-e2e
1 affected rust-cache-action -
1 run scripts:distribution-check -
1 run scripts:distribution-test -"
    ;;
  test-windows)
    # As test-macos, with a slot per core of the 4-core runner, one of them the Linear crates':
    # its suites spend most of their time waiting on the loopback servers and processes they
    # drive, not computing.
    # The workflow suites as on test-macos. The distribution journey names its slowest steps
    # here, where it is slowest.
    export ONETASKGRAPH_DISTRIBUTION_TIMINGS=1
    phases="1 affected test,budgets onetaskgraph-linear,onetaskgraph-linear-e2e,github-workflows,github-workflow-policy --parallel=3
1 affected test,budgets *,!onetaskgraph-linear,!onetaskgraph-linear-e2e
1 affected rust-cache-action -
1 run scripts:distribution-check -
1 run scripts:distribution-test -"
    ;;
  *) usage ;;
esac
# llmlint: ignore-end[work_goes_through_command_surface]

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
  local phase="$1" kind="$2" targets="$3" excluded="$4" flags="${5:-}" exclusions="" plan phase_status
  [ "$excluded" = "-" ] || exclusions="$excluded"
  if [ "$kind" = "affected" ] && [ -n "$live_exclusions" ] \
    && { [[ ",$targets," == *",test,"* ]] || [[ ",$targets," == *",budgets,"* ]]; }; then
    exclusions="${exclusions:+$exclusions,}$live_exclusions"
  fi
  if [ "$kind" = "affected" ]; then
    read -r -a target_words <<<"${targets//,/ }"
    arguments=(affected -t "${target_words[@]}")
    [ -z "$exclusions" ] || arguments+=("--exclude=$exclusions")
  else
    arguments=(run "$targets")
  fi
  if [ -n "$flags" ]; then
    read -r -a flag_words <<<"${flags//,/ }"
    arguments+=("${flag_words[@]}")
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

# The phases are waited for by their own process ids, so nothing else this script starts in the
# background is waited for with them.
phase=0
group=
running=""
wait_for_phases() {
  for pid in $running; do wait "$pid"; done
  running=""
}
while IFS=' ' read -r in_group kind targets excluded flags; do
  [ -n "$kind" ] || continue
  [ "$in_group" = "$group" ] || wait_for_phases
  group="$in_group"
  phase=$((phase + 1))
  record_phase "$phase" "$kind" "$targets" "$excluded" "$flags" &
  running="$running $!"
done <<EOF
$phases
EOF
wait_for_phases
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
