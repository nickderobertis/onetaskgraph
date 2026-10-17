#!/usr/bin/env bash
# Drive the four pull-request job recipes, `just ci-<job>`, the way their ci.yml steps do.
#
# Each recipe is real, scripts/ci-job.sh is real, Nx and its affected selection are real and
# so is the live-lane decision the test recipes consult. What is not real is what a target
# DOES: in a scratch clone every Nx target's command is replaced with one that only succeeds
# (or, for one case, fails), so what the recipe ran is readable from its own output in
# seconds, without building or testing the workspace. Three commits are put to it — a change
# to one project, a change to nothing any project tests, and the one-project change with one
# of its targets failing — and it is held to which Nx targets ran, which did not, and its exit
# status. The Linux recipes run; the Windows and macOS ones are driven through their
# selection step (`--select-only`), which is all this host can run of them.
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# The scratch tree needs a real copy of node_modules, which the Windows runner cannot make
# (scripts/check-affected-selection.sh says why); the Linux and macOS lanes run this.
case "${OS:-}${OSTYPE:-}" in
  *Windows_NT* | *msys* | *cygwin* | *win32*)
    echo "check-ci-recipes: skipped on Windows (bun's node_modules is a symlink tree the runner cannot copy); the Linux and macOS lanes drive the recipes" >&2
    exit 0
    ;;
esac
if [ ! -d "$ROOT/node_modules" ]; then
  echo "check-ci-recipes: there is no node_modules to copy into the scratch tree." >&2
  echo "check-ci-recipes: next: run 'just bootstrap' (or 'bun install'), then re-run." >&2
  exit 1
fi

scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
# shellcheck source=scripts/scratch-clone.sh
source "$ROOT/scripts/scratch-clone.sh"
scratch_clone "$ROOT" "$scratch/repo"
readonly REPO="$scratch/repo"
cp -a "$ROOT/node_modules" "$REPO/node_modules"
git -C "$REPO" config user.email "check-ci-recipes@invalid"
git -C "$REPO" config user.name "check-ci-recipes"
# This runs inside an Nx target, so the Nx it drives shares no state with the outer one.
export NX_DAEMON=false
export NX_WORKSPACE_DATA_DIRECTORY="$scratch/nx-workspace-data"
# No credential reaches the recipes: the live decision still runs, and no session opens.
unset GH_PROJECTS_TOKEN LINEAR_API_KEY ONETASKGRAPH_LIVE_REQUIRED

# Every run-commands target becomes a recorder that succeeds, uncached; `FAILING` names the
# one task (`project:target`) that fails instead.
record_targets() {
  (cd "$REPO" && FAILING="${1:-}" python3 - <<'PY'
import json, os, subprocess
failing = os.environ["FAILING"]
for path in subprocess.check_output(["git", "ls-files", "*project.json"], text=True).split():
    with open(path, encoding="utf-8") as handle:
        project = json.load(handle)
    name = project.get("name")
    for target, definition in project.get("targets", {}).items():
        if definition.get("executor") != "nx:run-commands":
            continue
        task = f"{name}:{target}"
        definition["options"] = {"command": "exit 1" if task == failing else "true"}
        definition["cache"] = False
    with open(path, "w", encoding="utf-8") as handle:
        json.dump(project, handle, indent=2)
PY
  )
}

failures=0
fail() {
  echo "check-ci-recipes: $1" >&2
  failures=$((failures + 1))
}

# commit <file>: a committed change to that file, against which the recipe compares.
commit_change() {
  git -C "$REPO" checkout --quiet -- .
  git -C "$REPO" reset --quiet --hard "$BASE"
  printf '\n// touched by scripts/check-ci-recipes.sh\n' >>"$REPO/$1"
  git -C "$REPO" add "$1"
  git -C "$REPO" commit --quiet --no-verify -m "test: touch $1"
}

# recipe <name> <args...>: run `just <name>` against the commit before HEAD, keeping its
# output in $OUTPUT and its exit status in $STATUS.
recipe() {
  OUTPUT="$(cd "$REPO" && NX_CACHE_DIRECTORY="$scratch/nx-cache-$RANDOM" NX_BASE="$BASE" NX_HEAD=HEAD \
    just "$@" 2>&1)" && STATUS=0 || STATUS=$?
}

# expect <case> <ran|selects> <task> [absent]: the recipe's own output names (or never names)
# that task in that verb.
expect() {
  local case_name="$1" verb="$2" task="$3" absent="${4:-}" job line
  job="${case_name%% *}"
  line="ci-job: $job: $verb $task"
  if [ -z "$absent" ] && ! grep -qxF "$line" <<<"$OUTPUT"; then
    fail "$case_name: expected '$line' in the recipe's output, which was:"
    printf '%s\n' "$OUTPUT" >&2
  elif [ -n "$absent" ] && grep -qxF "$line" <<<"$OUTPUT"; then
    fail "$case_name: '$line' is in the recipe's output, but $task is not affected by the change"
  fi
}

expect_status() {
  local case_name="$1" expected="$2"
  if [ "$expected" = 0 ] && [ "$STATUS" -ne 0 ]; then
    fail "$case_name: the recipe exited $STATUS, expected 0. Its output:"
    printf '%s\n' "$OUTPUT" >&2
  elif [ "$expected" != 0 ] && [ "$STATUS" -eq 0 ]; then
    fail "$case_name: the recipe exited 0 although a target it ran failed. Its output:"
    printf '%s\n' "$OUTPUT" >&2
  fi
}

# Neither the changed project nor anything that depends on it.
readonly CHANGED=onetaskgraph-local-md
readonly CHANGED_FILE=crates/onetaskgraph-local-md/src/lib.rs
readonly DEPENDENT=onetaskgraph-core
readonly UNAFFECTED=onetaskgraph-plugin-api
readonly NOTHING_FILE=LICENSE

record_targets
git -C "$REPO" commit --quiet --no-verify -am "test: record every target"
BASE="$(git -C "$REPO" rev-parse HEAD)"

# 1. A change to one project.
commit_change "$CHANGED_FILE"
recipe ci-lint
expect_status "lint after a change to $CHANGED" 0
for target in format-check lint typecheck; do
  expect "lint after a change to $CHANGED" ran "$CHANGED:$target"
  expect "lint after a change to $CHANGED" ran "$DEPENDENT:$target"
  expect "lint after a change to $CHANGED" ran "$UNAFFECTED:$target" absent
done
expect "lint after a change to $CHANGED" ran "sdk-python:generate-check"
expect "lint after a change to $CHANGED" ran "workspace:deny"
expect "lint after a change to $CHANGED" ran "$CHANGED:test" absent

recipe ci-test-linux
expect_status "test-linux after a change to $CHANGED" 0
# The suite runs once: under coverage, plus what coverage cannot run, on Linux.
expect "test-linux after a change to $CHANGED" ran "$CHANGED:coverage"
expect "test-linux after a change to $CHANGED" ran "$CHANGED:test" absent
expect "test-linux after a change to $CHANGED" ran "$DEPENDENT:test-uncovered"
expect "test-linux after a change to $CHANGED" ran "onetaskgraph-linear-e2e:test"
expect "test-linux after a change to $CHANGED" ran "onetaskgraph-linear-e2e:budgets"
expect "test-linux after a change to $CHANGED" ran "workspace:test"
expect "test-linux after a change to $CHANGED" ran "workspace:coverage"
expect "test-linux after a change to $CHANGED" ran "scripts:distribution-check"
expect "test-linux after a change to $CHANGED" ran "scripts:distribution-test"
expect "test-linux after a change to $CHANGED" ran "$UNAFFECTED:test" absent
expect "test-linux after a change to $CHANGED" ran "github-workflows:test" absent
expect "test-linux after a change to $CHANGED" ran "$CHANGED:lint" absent

recipe ci-test-windows --select-only
expect_status "test-windows after a change to $CHANGED" 0
# Windows measures no Rust coverage, so the crate's own tests run there.
expect "test-windows after a change to $CHANGED" selects "$CHANGED:test"
expect "test-windows after a change to $CHANGED" selects "$DEPENDENT:test-uncovered" absent
expect "test-windows after a change to $CHANGED" selects "sdk-python:coverage"
expect "test-windows after a change to $CHANGED" selects "sdk-python:test" absent
expect "test-windows after a change to $CHANGED" selects "$UNAFFECTED:test" absent
expect "test-windows after a change to $CHANGED" ran "$CHANGED:test" absent

recipe ci-test-macos --select-only
expect_status "test-macos after a change to $CHANGED" 0
expect "test-macos after a change to $CHANGED" selects "$CHANGED:coverage"
expect "test-macos after a change to $CHANGED" selects "$CHANGED:test" absent
expect "test-macos after a change to $CHANGED" selects "$DEPENDENT:test-uncovered"
expect "test-macos after a change to $CHANGED" selects "$UNAFFECTED:test" absent
expect "test-macos after a change to $CHANGED" ran "$CHANGED:coverage" absent

# 2. A change to nothing any project tests: the affected phases select nothing, and the
#    phases every job runs by name still run.
commit_change "$NOTHING_FILE"
recipe ci-lint
expect_status "lint after a change to $NOTHING_FILE" 0
expect "lint after a change to $NOTHING_FILE" ran "$CHANGED:lint" absent
expect "lint after a change to $NOTHING_FILE" ran "workspace:lint" absent
expect "lint after a change to $NOTHING_FILE" ran "workspace:deny"
for job in test-linux; do
  recipe "ci-$job"
  expect_status "$job after a change to $NOTHING_FILE" 0
  expect "$job after a change to $NOTHING_FILE" ran "$CHANGED:coverage" absent
  expect "$job after a change to $NOTHING_FILE" ran "workspace:test" absent
  expect "$job after a change to $NOTHING_FILE" ran "scripts:distribution-test"
done
for job in test-windows test-macos; do
  recipe "ci-$job" --select-only
  expect_status "$job after a change to $NOTHING_FILE" 0
  expect "$job after a change to $NOTHING_FILE" selects "workspace:test" absent
  expect "$job after a change to $NOTHING_FILE" selects "$CHANGED:coverage" absent
  expect "$job after a change to $NOTHING_FILE" selects "scripts:distribution-test"
done

# 3. One of the changed project's targets failing fails the recipe that ran it, and every
#    later phase still runs. The commit is that project's own project.json, which is what
#    makes it the one project the change reaches.
fail_one() {
  git -C "$REPO" checkout --quiet -- .
  git -C "$REPO" reset --quiet --hard "$BASE"
  record_targets "$1"
  git -C "$REPO" commit --quiet --no-verify -am "test: fail $1"
}
fail_one "$CHANGED:lint"
recipe ci-lint
expect_status "lint with $CHANGED:lint failing" 1
expect "lint with $CHANGED:lint failing" ran "workspace:deny"
grep -qF "ci-job: lint: nx affected -t format-check lint typecheck failed" <<<"$OUTPUT" \
  || fail "lint with $CHANGED:lint failing: the recipe did not name the phase that failed"
fail_one "$CHANGED:coverage"
recipe ci-test-linux
expect_status "test-linux with $CHANGED:coverage failing" 1
expect "test-linux with $CHANGED:coverage failing" ran "scripts:distribution-test"
grep -qF "ci-job: test-linux: nx affected -t test-uncovered coverage failed" <<<"$OUTPUT" \
  || fail "test-linux with $CHANGED:coverage failing: the recipe did not name the phase that failed"

if [ "$failures" -ne 0 ]; then
  echo "check-ci-recipes: $failures expectation(s) about the pull-request job recipes failed — see above." >&2
  echo "check-ci-recipes: next: fix scripts/ci-job.sh, the ci-* recipes in the justfile or the tags and targets they select, then re-run 'bash scripts/check-ci-recipes.sh'." >&2
  exit 1
fi
