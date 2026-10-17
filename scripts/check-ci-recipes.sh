#!/usr/bin/env bash
# Drive the four pull-request job recipes, `just ci-<job>`, the way their ci.yml steps do.
#
# The recipes, scripts/ci-job.sh, Nx's affected selection and the live-lane decision are all
# real; only what a target DOES is not — in a scratch clone each target's command is replaced
# with one that succeeds or fails on cue, so what a recipe ran is readable from its own output
# in seconds. The Windows and macOS recipes are driven through `--select-only`, which is all
# this host can run of them.
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# Absolute, because every case runs it from inside the scratch clone, where a relative PATH
# entry such as a virtualenv's resolves to nothing (AGENTS.md).
PYTHON="$(command -v python3)" || {
  echo "check-ci-recipes: python3 is not on PATH; next: install Python 3, then re-run." >&2
  exit 1
}
case "$PYTHON" in
  /*) ;;
  *) PYTHON="$(cd "$(dirname "$PYTHON")" && pwd)/$(basename "$PYTHON")" ;;
esac
readonly PYTHON

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
# This runs inside an Nx target, so the Nx it drives shares no state with the outer one —
# including the outer run's root process, under which Nx registers every task it runs to
# catch a recursive invocation: the recipes below run the same tasks again and again, and
# each phase of a job runs at once, so under that root each would read as a loop.
export NX_DAEMON=false
export NX_WORKSPACE_DATA_DIRECTORY="$scratch/nx-workspace-data"
unset NX_INVOCATION_ROOT_PID
# No credential reaches the recipes: the live decision still runs, and no session opens.
unset GH_PROJECTS_TOKEN LINEAR_API_KEY ONETASKGRAPH_LIVE_REQUIRED

# Every run-commands target becomes a recorder that succeeds, uncached; `FAILING` names the
# one task (`project:target`) that fails instead.
record_targets() {
  (cd "$REPO" && FAILING="${1:-}" "$PYTHON" - <<'PY'
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

# expect <case> <ran|selects> <task> [absent]: the recipe's own output names (or, given
# `absent`, never names) that task in that verb — a task the change does not reach, or one the
# job runs another way.
expect() {
  local case_name="$1" verb="$2" task="$3" absent="${4:-}" job line
  job="${case_name%% *}"
  # A phase that ran says so on one line, `ci-job: <job>: ran <task> <task>...`; a selection
  # is one task per line. Either way the task is one whole word after the verb.
  if grep -E "^ci-job: $job: $verb " <<<"$OUTPUT" | tr ' ' '\n' | grep -qxF "$task"; then
    line=present
  else
    line=absent
  fi
  if [ -z "$absent" ] && [ "$line" = absent ]; then
    fail "$case_name: expected the recipe to say it $verb $task; its output was:"
    printf '%s\n' "$OUTPUT" >&2
  elif [ -n "$absent" ] && [ "$line" = present ]; then
    fail "$case_name: the recipe $verb $task, which this job must not do for this change"
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

readonly CHANGED=onetaskgraph-local-md
readonly CHANGED_FILE=crates/onetaskgraph-local-md/src/lib.rs
readonly DEPENDENT=onetaskgraph-core
readonly UNAFFECTED=onetaskgraph-plugin-api
readonly NOTHING_FILE=LICENSE

# 0. The tag that makes a job skip a project's uninstrumented `test` is held to its claim, and
#    the check that holds it is watched refusing each way the claim can stop being true.
tag_refused() {
  local case_name="$1" expected="$2" guard
  guard="$(cd "$REPO" && bash scripts/check-test-in-coverage.sh 2>&1)" \
    && fail "$case_name: check-test-in-coverage passed it" \
    || grep -qF "$expected" <<<"$guard" \
    || fail "$case_name: check-test-in-coverage refused it without saying '$expected': $guard"
  git -C "$REPO" checkout --quiet -- .
}
(cd "$REPO" && bash scripts/check-test-in-coverage.sh) \
  || fail "check-test-in-coverage does not pass on the committed tree, so the cases below prove nothing"
"$PYTHON" - "$REPO/crates/$CHANGED/project.json" <<'PY'
import json, sys
project = json.load(open(sys.argv[1]))
project["targets"]["test"]["options"]["command"] += " && bash scripts/never-run-elsewhere.sh"
json.dump(project, open(sys.argv[1], "w"))
PY
tag_refused "a tagged crate's test gaining a command nothing else runs" "never-run-elsewhere.sh"
# The libraries' one doctest run leaving a library out, filtered to a name, and narrowed to a
# feature set.
for change in unnamed:"has a library" filtered:"nothing else" narrowed:"must run the tagged"; do
  "$PYTHON" - "$REPO/workspace/project.json" "${change%%:*}" "$DEPENDENT" <<'PY'
import json, sys
path, change, dependent = sys.argv[1:]
project = json.load(open(path))
options = project["targets"]["test-uncovered"]["options"]
words = options["command"].split()
if change == "unnamed":
    at = words.index(dependent)
    del words[at - 1 : at + 1]
elif change == "filtered":
    words.append("engine")
else:
    words[words.index("--all-features")] = "--no-default-features"
options["command"] = " ".join(words)
json.dump(project, open(path, "w"))
PY
  tag_refused "the libraries' doctest run ${change%%:*}" "${change#*:}"
done
"$PYTHON" - "$REPO/crates/onetaskgraph-github-projects/project.json" <<'PY'
import json, sys
project = json.load(open(sys.argv[1]))
project["tags"].append("test-in-coverage:unix")
json.dump(project, open(sys.argv[1], "w"))
PY
tag_refused "a crate with a live lane tagged" "has a live lane"
"$PYTHON" - "$REPO/sdks/python/project.json" <<'PY'
import json, sys
project = json.load(open(sys.argv[1]))
project["targets"]["coverage"]["options"]["command"] = "uv run --frozen pytest -q tests/smoke"
json.dump(project, open(sys.argv[1], "w"))
PY
tag_refused "an SDK whose coverage runs fewer tests than its test" "coverage switched on"
sed -i.bak 's/^  --all-features \\$/  --all-features -E "test(only_one)" \\/' "$REPO/scripts/rust-coverage.sh"
rm -f "$REPO/scripts/rust-coverage.sh.bak"
tag_refused "the coverage run filtering its tests" "not the whole crate with every feature"
sed -i.bak '/^  --all-features \\$/d' "$REPO/scripts/rust-coverage.sh"
rm -f "$REPO/scripts/rust-coverage.sh.bak"
tag_refused "the coverage run dropping --all-features" "not the whole crate with every feature"

record_targets
git -C "$REPO" commit --quiet --no-verify -am "test: record every target"
BASE="$(git -C "$REPO" rev-parse HEAD)"

# 1. A change to one project: its targets and its dependents' run, those of a project it never
#    reaches do not, and on Linux and macOS its suite runs once, under coverage.
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
# Both ways a job reports — streaming, as in Actions, and held until a phase ends, as anywhere
# else — because the two take different paths through a failing phase.
for actions in true ""; do
  mode="${actions:+streaming as in Actions}"
  mode="${mode:-held}"
  fail_one "$CHANGED:lint"
  GITHUB_ACTIONS="$actions" recipe ci-lint
  expect_status "lint with $CHANGED:lint failing, $mode" 1
  expect "lint with $CHANGED:lint failing, $mode" ran "workspace:deny"
  grep -qF "ci-job: lint: nx affected -t format-check lint typecheck failed" <<<"$OUTPUT" \
    || fail "lint with $CHANGED:lint failing, $mode: the recipe did not name the phase that failed"
  fail_one "$CHANGED:coverage"
  GITHUB_ACTIONS="$actions" recipe ci-test-linux
  expect_status "test-linux with $CHANGED:coverage failing, $mode" 1
  expect "test-linux with $CHANGED:coverage failing, $mode" ran "scripts:distribution-test"
  grep -qF "ci-job: test-linux: nx affected -t test-uncovered coverage failed" <<<"$OUTPUT" \
    || fail "test-linux with $CHANGED:coverage failing, $mode: the recipe did not name the phase that failed"
done

# 4. Nx answering with something other than a task graph fails the recipe rather than reading
#    as a phase with nothing to run. The stand-in is Nx's own executable, so everything the
#    recipe does around it is still real.
git -C "$REPO" checkout --quiet -- .
git -C "$REPO" reset --quiet --hard "$BASE"
commit_change "$CHANGED_FILE"
mv "$REPO/node_modules/.bin/nx" "$REPO/node_modules/.bin/nx.real"
printf '#!/usr/bin/env bash\necho "{\\"graph\\": {}}"\n' >"$REPO/node_modules/.bin/nx"
chmod +x "$REPO/node_modules/.bin/nx"
recipe ci-test-macos --select-only
expect_status "test-macos with Nx printing no task graph" 1
grep -qF "ci-job: Nx printed no task graph" <<<"$OUTPUT" \
  || fail "test-macos with Nx printing no task graph: the recipe did not say Nx printed no task graph: $OUTPUT"
mv "$REPO/node_modules/.bin/nx.real" "$REPO/node_modules/.bin/nx"

# 5. The tool pins ci.yml keys its caches on are read from where each tool is pinned, so a
#    pin that moves is a different key; a pin that is gone fails rather than keying on nothing.
pins() {
  PINS="$(cd "$REPO" && GITHUB_OUTPUT="$scratch/pins-output" bash scripts/ci-tool-pins.sh 2>&1)" \
    && PINS_STATUS=0 || PINS_STATUS=$?
}
: >"$scratch/pins-output"
sed -i.bak 's/^readonly RELEASE_PLZ_VERSION=.*/readonly RELEASE_PLZ_VERSION=9.8.7/' "$REPO/scripts/scoped-release-plz.sh"
sed -i.bak 's/^prune_superseded_version := .*/prune_superseded_version := "6.5.4"/' "$REPO/justfile"
pins
[ "$PINS_STATUS" -eq 0 ] && [ "$(cat "$scratch/pins-output")" = "release-plz=9.8.7
prune-superseded=6.5.4" ] \
  || fail "the tool pins did not follow the moved pins to \$GITHUB_OUTPUT: exit $PINS_STATUS, $PINS"
for broken in "" "9.8" "9.8.7 9.8.8" "--all" "9..8.7" "9.8.7."; do
  : >"$scratch/pins-output"
  if [ -z "$broken" ]; then
    sed -i.bak '/^readonly RELEASE_PLZ_VERSION=/d' "$REPO/scripts/scoped-release-plz.sh"
  else
    sed -i.bak "s/^readonly RELEASE_PLZ_VERSION=.*/readonly RELEASE_PLZ_VERSION=$broken/" "$REPO/scripts/scoped-release-plz.sh"
  fi
  pins
  [ "$PINS_STATUS" -ne 0 ] && grep -qF "no single pinned version for release-plz" <<<"$PINS" \
    && [ ! -s "$scratch/pins-output" ] \
    || fail "a release-plz pin of '$broken' did not fail the tool pins before writing a key: exit $PINS_STATUS, $PINS"
  git -C "$REPO" checkout --quiet -- scripts/scoped-release-plz.sh
done
rm -f "$REPO/scripts/scoped-release-plz.sh.bak" "$REPO/justfile.bak"
git -C "$REPO" checkout --quiet -- .

# 6. A pull request's comparison base is the merge base of the commit under test with its base
#    branch's tip, over a real remote whose base branch moved on after the branch was cut.
remote="$scratch/base-remote.git"
work="$scratch/base-work"
git init --quiet --bare "$remote"
git init --quiet -b main "$work"
git -C "$work" config user.email "check-ci-recipes@invalid"
git -C "$work" config user.name "check-ci-recipes"
git -C "$work" remote add origin "$remote"
git -C "$work" commit --quiet --allow-empty -m "the fork point"
fork="$(git -C "$work" rev-parse HEAD)"
git -C "$work" commit --quiet --allow-empty -m "main moves on"
tip="$(git -C "$work" rev-parse HEAD)"
git -C "$work" push --quiet origin main
git -C "$work" checkout --quiet -b feature "$fork"
git -C "$work" commit --quiet --allow-empty -m "the pull request"
: >"$scratch/base-output"
derived="$(cd "$work" && GITHUB_OUTPUT="$scratch/base-output" bash "$REPO/scripts/ci-pr-base.sh" main "$tip" 2>&1)" \
  && derived_status=0 || derived_status=$?
[ "$derived_status" -eq 0 ] && [ "$(cat "$scratch/base-output")" = "base=$fork" ] \
  || fail "the comparison base was not the fork point $fork: exit $derived_status, $derived, output $(cat "$scratch/base-output")"
derived="$(cd "$work" && bash "$REPO/scripts/ci-pr-base.sh" main "$(printf '%040d' 0)" 2>&1)" \
  && derived_status=0 || derived_status=$?
[ "$derived_status" -ne 0 ] && grep -qF "share no merge base" <<<"$derived" \
  || fail "a base sha the checkout does not hold did not fail the comparison base: exit $derived_status, $derived"
for arguments in "--all $tip" "main..feature $tip" "main ${tip:0:12}" "main HEAD" "main"; do
  # shellcheck disable=SC2086 # each case is two words, or deliberately one
  derived="$(cd "$work" && bash "$REPO/scripts/ci-pr-base.sh" $arguments 2>&1)" \
    && derived_status=0 || derived_status=$?
  [ "$derived_status" -eq 2 ] && grep -qF "usage: scripts/ci-pr-base.sh" <<<"$derived" \
    || fail "the comparison base accepted '$arguments' rather than refusing it before git: exit $derived_status, $derived"
done

if [ "$failures" -ne 0 ]; then
  echo "check-ci-recipes: $failures expectation(s) about the pull-request job recipes failed — see above." >&2
  echo "check-ci-recipes: next: fix scripts/ci-job.sh, the ci-* recipes in the justfile or the tags and targets they select, then re-run 'bash scripts/check-ci-recipes.sh'." >&2
  exit 1
fi
