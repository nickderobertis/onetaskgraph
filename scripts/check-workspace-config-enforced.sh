#!/usr/bin/env bash
# Watch the two guards over the shared target directory refuse what they are meant to.
#
# scripts/check-workspace-config.sh holds the structure that keeps target/debug/onetaskgraph
# from being replaced while a test spawns it, and scripts/check-coverage-enforced.sh holds
# the wiring that enforces the line floor over every crate's run. Both are text over
# project files, so each would keep passing after it stopped matching what it describes.
# Every shape they refuse is planted here, one at a time, in a scratch copy of the WORKING
# tree, and the refusal and its diagnostic are asserted; so is the refusal each spawner
# gives when the binary it depends on is not there.
set -euo pipefail

fatal() {
  echo "check-workspace-config-enforced: $1" >&2
  echo "check-workspace-config-enforced: next: $2" >&2
  exit 1
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)" || fatal \
  "could not resolve this repository's root from ${BASH_SOURCE[0]}" \
  "run the check from a checkout of this repository, as 'just script-check' does"
readonly ROOT

# Under a git hook GIT_DIR overrides `git -C`, so the environment is stripped before the
# tracked files are listed; the helper that does it is tested for before it is sourced,
# because a missing `source` ends a bash 3.2 shell outright.
# shellcheck source=scripts/scratch-clone.sh
if [ ! -r "$ROOT/scripts/scratch-clone.sh" ] || ! source "$ROOT/scripts/scratch-clone.sh"; then
  fatal "could not load $ROOT/scripts/scratch-clone.sh" \
    "restore it with 'git checkout -- scripts/scratch-clone.sh', then rerun"
fi
scratch_clone_strip_git_env

scratch="$(mktemp -d)" || fatal \
  "could not create the scratch tree this check mutates" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
trap 'rm -rf "$scratch"' EXIT

# The working tree's tracked files, as they are right now: what is under test is the guard
# as an author has it, and the guard reads the whole tree — every project file, every SDK
# source, the version inventory's manifests.
if ! (cd "$ROOT" && git ls-files -z | while IFS= read -r -d '' tracked; do
  mkdir -p "$scratch/$(dirname "$tracked")" && cp "$ROOT/$tracked" "$scratch/$tracked"
done); then
  fatal "could not copy the tracked files of $ROOT into $scratch" \
    "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
fi

failures=0

# expect <guard script> <case> <diagnostic fragment>: the guard, run over the scratch tree
# as mutated, must refuse and must name the reason.
expect() {
  local guard="$1" case="$2" fragment="$3" output status
  output="$(cd "$scratch" && bash "scripts/$guard" 2>&1)" && status=0 || status=$?
  if [ "$status" -eq 0 ]; then
    echo "check-workspace-config-enforced: $guard accepted $case" >&2
    failures=$((failures + 1))
  elif ! printf '%s\n' "$output" | grep -qF -- "$fragment"; then
    printf '%s\n' "$output" >&2
    echo "check-workspace-config-enforced: $guard refused $case without saying '$fragment'" >&2
    failures=$((failures + 1))
  elif printf '%s\n' "$output" | grep -q 'Traceback'; then
    printf '%s\n' "$output" >&2
    echo "check-workspace-config-enforced: $guard refused $case with a Python traceback instead of a diagnostic" >&2
    failures=$((failures + 1))
  fi
}

# One mutation of one scratch file, as a python expression over `path`, restored after the
# case so the next one starts from the working tree. python3 rather than `sed -i`, whose
# in-place spelling differs between GNU and BSD and so would fail on the macOS runner.
mutate() {
  python3 - "$scratch/$1" "$2" <<'PY' || fatal \
    "the helper that rewrites a scratch file did not finish, so that case was never put to the guard" \
    "run 'python3 --version' to confirm a working python3 is on PATH, then rerun"
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
code = sys.argv[2]
text = path.read_text(encoding="utf-8")
document = json.loads(text) if path.suffix == ".json" else None
namespace = {"path": path, "text": text, "document": document, "json": json}
exec(code, namespace)
if namespace["document"] is not None:
    path.write_text(json.dumps(namespace["document"], indent=2) + "\n", encoding="utf-8")
else:
    path.write_text(namespace["text"], encoding="utf-8")
PY
}

restore() {
  cp "$ROOT/$1" "$scratch/$1" || fatal \
    "could not restore $1 in the scratch copy, so every case after this one would run against a tree still carrying the previous mutation" \
    "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
}

# The guards accept the working tree before anything is planted, or the cases below prove
# nothing about the mutation.
for guard in check-workspace-config.sh check-coverage-enforced.sh; do
  if ! output="$(cd "$scratch" && bash "scripts/$guard" 2>&1)"; then
    printf '%s\n' "$output" >&2
    fatal "scripts/$guard refuses the working tree before any case is planted" \
      "fix what it names above, then rerun"
  fi
done

BINARY_PROJECT=crates/onetaskgraph/project.json
PYTHON_PROJECT=sdks/python/project.json
TYPESCRIPT_PROJECT=sdks/typescript/project.json
CORE_PROJECT=crates/onetaskgraph-core/project.json
WORKSPACE_PROJECT=workspace/project.json
COVERAGE_SCRIPT=scripts/rust-coverage.sh
CONFTEST=sdks/python/tests/conftest.py

# --- check-workspace-config.sh: the binary is built once and never relinked under a test.

mutate "$BINARY_PROJECT" 'document["targets"]["test"]["options"]["command"] = document["targets"]["test"]["options"]["command"].replace(" --all-features", "")'
expect check-workspace-config.sh "a test target linking a different unit from its build target" \
  "test and build resolve different units of the package"
restore "$BINARY_PROJECT"

mutate "$BINARY_PROJECT" 'document["targets"]["build"]["cache"] = True'
expect check-workspace-config.sh "a cached build target" "build is cached"
restore "$BINARY_PROJECT"

mutate "$BINARY_PROJECT" 'del document["targets"]["test"]["dependsOn"]'
expect check-workspace-config.sh "a test target that does not depend on build" \
  "test does not depend on build"
restore "$BINARY_PROJECT"

mutate "$PYTHON_PROJECT" 'del document["targets"]["test"]["dependsOn"]'
expect check-workspace-config.sh "an SDK test target that spawns the binary without depending on its build" \
  "sdks/python/project.json: test spawns target/debug/onetaskgraph (through sdks/python/tests/conftest.py) but does not depend on onetaskgraph:build"
restore "$PYTHON_PROJECT"

# test, coverage and pack reach the build only through generate-check, so its edge going
# is every one of them reported.
mutate "$TYPESCRIPT_PROJECT" 'del document["targets"]["generate-check"]["dependsOn"]'
expect check-workspace-config.sh "a chain of SDK targets losing the one edge that reached the build" \
  "sdks/typescript/project.json: pack spawns target/debug/onetaskgraph"
restore "$TYPESCRIPT_PROJECT"

mutate "$CORE_PROJECT" 'document["targets"]["test"]["options"]["commands"][0] += " --target-dir target/tests/core"'
expect check-workspace-config.sh "a target naming a cargo target directory of its own" \
  "names a cargo target directory of its own"
restore "$CORE_PROJECT"

mutate "$TYPESCRIPT_PROJECT" 'document["targets"]["generate-check"]["options"]["command"] = "cargo build -p onetaskgraph && " + document["targets"]["generate-check"]["options"]["command"]'
expect check-workspace-config.sh "another target building the binary package" \
  "invokes cargo on the onetaskgraph package"
restore "$TYPESCRIPT_PROJECT"

mutate "$CONFTEST" 'text += "\nsubprocess.run([\"cargo\", \"build\", \"-p\", \"onetaskgraph\"])\n"'
expect check-workspace-config.sh "a spawner building the binary itself" \
  "sdks/python/tests/conftest.py: runs cargo itself"
restore "$CONFTEST"

printf 'const binary = resolve(root, "target/debug/onetaskgraph");\n' > "$scratch/sdks/typescript/tests/unregistered.test.ts" \
  || fatal "could not plant the unregistered spawner in $scratch" \
    "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
expect check-workspace-config.sh "a new source reaching the binary without being registered" \
  "sdks/typescript/tests/unregistered.test.ts: reaches into target/debug"
rm -f "$scratch/sdks/typescript/tests/unregistered.test.ts"

# Every spelling in the file, the docstring's included: the scan reads the whole file.
mutate "$CONFTEST" 'text = text.replace("debug", "built")'
expect check-workspace-config.sh "a registered spawner that no longer reaches the binary" \
  "sdks/python/tests/conftest.py: is registered in SPAWNERS in scripts/check-workspace-config.sh but no longer reaches into target/debug"
restore "$CONFTEST"

# --- check-coverage-enforced.sh: the floor is one report over every crate's run.

mutate "$WORKSPACE_PROJECT" 'document["targets"]["coverage"]["dependsOn"][0]["projects"].remove("onetaskgraph-linear")'
expect check-coverage-enforced.sh "a crate dropped from the aggregate's dependencies" \
  "coverage does not depend on 'onetaskgraph-linear''s coverage target"
restore "$WORKSPACE_PROJECT"

mutate "$WORKSPACE_PROJECT" 'document["targets"]["coverage"]["options"]["commands"] = ["bash scripts/check-coverage-enforced.sh"]'
expect check-coverage-enforced.sh "an aggregate that no longer reports" \
  "coverage does not run 'bash scripts/rust-coverage.sh --report'"
restore "$WORKSPACE_PROJECT"

mutate "$CORE_PROJECT" 'del document["targets"]["coverage"]["cache"]'
expect check-coverage-enforced.sh "a cached crate coverage target" "its coverage target is cached"
restore "$CORE_PROJECT"

mutate "$CORE_PROJECT" 'document["targets"]["coverage"]["dependsOn"] = []'
expect check-coverage-enforced.sh "a crate run that does not clear first" \
  "does not depend on workspace:coverage-clear"
restore "$CORE_PROJECT"

mutate "$COVERAGE_SCRIPT" 'text = text.replace("readonly MIN_LINES=95", "readonly MIN_LINES=80")'
expect check-coverage-enforced.sh "a lowered floor" "the line-coverage floor is 80%"
restore "$COVERAGE_SCRIPT"

mutate "$COVERAGE_SCRIPT" 'text = text.replace("    --fail-under-lines \"$MIN_LINES\" 2>&1", "    2>&1")'
expect check-coverage-enforced.sh "a report without the floor" \
  "does not pass --fail-under-lines"
restore "$COVERAGE_SCRIPT"

mutate "$COVERAGE_SCRIPT" 'text = text.replace("  --no-report \\\n", "")'
expect check-coverage-enforced.sh "a crate run without --no-report" \
  "no longer passes --no-report"
restore "$COVERAGE_SCRIPT"

printf '{\n' > "$scratch/$CORE_PROJECT"
expect check-coverage-enforced.sh "a project file that is not JSON" \
  "crates/onetaskgraph-core/project.json: could not be read as JSON"
restore "$CORE_PROJECT"

# --- Every spawner refuses, naming the build target, when the binary is not there. The
# scratch tree has no target directory, so each resolves a path nothing built.

output="$(cd "$scratch" && bash scripts/test-distribution.sh 2>&1)" && status=0 || status=$?
if [ "$status" -eq 0 ] || ! printf '%s\n' "$output" | grep -qF "run 'scripts/nx.sh run onetaskgraph:build'"; then
  printf '%s\n' "$output" >&2
  echo "check-workspace-config-enforced: scripts/test-distribution.sh did not refuse a missing target/debug/onetaskgraph by naming onetaskgraph:build (exit $status)" >&2
  failures=$((failures + 1))
fi

output="$(cd "$scratch/sdks/python" && uv run --frozen --project "$ROOT/sdks/python" python generate.py --check 2>&1)" && status=0 || status=$?
if [ "$status" -eq 0 ] || ! printf '%s\n' "$output" | grep -qF 'run `scripts/nx.sh run onetaskgraph:build`'; then
  printf '%s\n' "$output" >&2
  echo "check-workspace-config-enforced: sdks/python/generate.py did not refuse a missing target/debug/onetaskgraph by naming onetaskgraph:build (exit $status)" >&2
  failures=$((failures + 1))
fi

output="$(cd "$scratch/sdks/python" && uv run --frozen --project "$ROOT/sdks/python" pytest -q --no-cov -p no:cacheprovider tests/test_artifact.py -k schema_bundle 2>&1)" && status=0 || status=$?
if [ "$status" -eq 0 ] || ! printf '%s\n' "$output" | grep -qF 'run `scripts/nx.sh run onetaskgraph:build`'; then
  printf '%s\n' "$output" >&2
  echo "check-workspace-config-enforced: the Python SDK's binary fixture did not refuse a missing target/debug/onetaskgraph by naming onetaskgraph:build (exit $status)" >&2
  failures=$((failures + 1))
fi

if [ "$failures" -ne 0 ]; then
  fatal "$failures case(s) above were accepted, or refused without their reason" \
    "repair the guard the case names so it refuses that shape and says why, then rerun 'just script-check'"
fi
