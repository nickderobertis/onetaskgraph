#!/usr/bin/env bash
# Prove the second half of the live lanes' edge, against real Nx and the real recipes.
#
# The first half is affected selection, and scripts/check-affected-selection.sh gates it.
# The second half is scripts/live-lane-selection.sh, which exists because a release commit
# defeats the first: it rewrites `Cargo.lock`, a `sharedGlobals` input, so every project in
# the workspace is selected and both hosted plugins open a session against a real API for a
# diff that reaches no plugin behaviour at all. That is not hypothetical — it is why the
# default branch has been red with the account's GraphQL budget exhausted.
#
# Reasoning about the recipes does not prove any of this, so every case below makes a real
# edit in a scratch clone, commits it, and puts it either to real Nx or to the real
# `just` recipes each path runs. Nothing here has a credential and nothing here reaches an
# API: what is under test is which targets would run, and the decision that says so.
#
#   1. Real Nx over real repository states: a core-only diff does not select the GitHub
#      Projects plugin's `test` target, a diff of that plugin's own source does, and a
#      version-only diff selects it until the decision's own `--exclude` is applied — which
#      is the whole reason the decision exists.
#   2. The decision's answers, each put to it as exactly that diff, so a manifest edit
#      cannot be mistaken for a version bump — and neither can the same version
#      substitution applied to a file the contract does not permit, which is the case that
#      holds the permitted set to being a set of PATHS rather than a shape a line has.
#   3. A question it cannot answer is answered `run`. It fails toward spending the budget.
#   4. Every path that can open a live session consults it. The paths are ENUMERATED from
#      the workflow and the hook rather than listed here, so one added or rewired later
#      cannot silently bypass the decision.
#   5. Each of those paths, driven: a diff the decision refuses leaves the lane reporting
#      it was not selected and the exclusion reaching Nx, and a diff it accepts leaves
#      neither.
set -euo pipefail

fatal() {
  echo "check-live-lane-selection: $1" >&2
  echo "check-live-lane-selection: next: $2" >&2
  exit 1
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)" || fatal \
  "could not resolve this repository's root from ${BASH_SOURCE[0]}" \
  "run the check from a checkout of this repository, as 'just script-check' does"
readonly ROOT

readonly PLUGIN="onetaskgraph-github-projects"
readonly PLUGIN_MANIFEST="crates/$PLUGIN/Cargo.toml"
readonly PLUGIN_CHANGELOG="crates/$PLUGIN/CHANGELOG.md"
readonly PLUGIN_SOURCE="crates/$PLUGIN/src/lib.rs"
readonly CORE_MANIFEST="crates/onetaskgraph-core/Cargo.toml"
readonly CORE_SOURCE="crates/onetaskgraph-core/src/registry.rs"
readonly CORE_CHANGELOG="crates/onetaskgraph-core/CHANGELOG.md"
# Outside the plugin, outside the permitted set, and version-bearing: this is where the
# Python SDK declares `__version__`, so a release bumps it with the very substitution a
# permitted manifest line gets. It is the case that tells the decision apart from one that
# reads a line's shape instead of the file it is in.
readonly OUTSIDE_SOURCE="sdks/python/src/onetaskgraph_sdk/__init__.py"
# Two files this repository does not have, planted in the base below, whose BASENAMES are
# the two the contract accepts and whose LOCATIONS are not. A test fixture called Cargo.toml
# and a package's own changelog are both ordinary things to find in a repository, and a
# decision that matched on the name alone would let a diff reach a plugin's behaviour
# through either.
readonly NESTED_MANIFEST="crates/onetaskgraph-core/tests/fixtures/Cargo.toml"
readonly OUTSIDE_CHANGELOG="sdks/python/CHANGELOG.md"

command -v just >/dev/null 2>&1 || fatal \
  "just is not installed, and case 5 drives the very recipes each path runs" \
  "install it (scripts/session-setup.sh does), then rerun"

# shellcheck source=scripts/scratch-clone.sh
if [ ! -r "$ROOT/scripts/scratch-clone.sh" ] || ! source "$ROOT/scripts/scratch-clone.sh"; then
  fatal \
    "could not load $ROOT/scripts/scratch-clone.sh, which is how every case below gets a tree it may commit to" \
    "restore it with 'git checkout -- scripts/scratch-clone.sh', then rerun"
fi

scratch="$(mktemp -d)" || fatal \
  "could not create the scratch tree these cases commit into" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
trap 'rm -rf "$scratch"' EXIT

readonly REPO="$scratch/repo"
readonly NX_LOG="$scratch/nx-invocations"

scratch_clone "$ROOT" "$REPO" || fatal \
  "could not clone this repository into $REPO" \
  "read the scratch-clone diagnostic above; if it names GIT_*, run this through 'just script-check'"

# The working tree's tracked files over the clone, as scripts/check-live-lane-enforced.sh
# does: what is under test has to be the decision and the recipes as they are right now,
# not the last commit's copy of them.
(cd "$ROOT" && git ls-files -z | tar --null -T - -cf -) | tar -xf - -C "$REPO" || fatal \
  "could not overlay this working tree's files onto $REPO" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"

git -C "$REPO" config user.email "check-live-lane-selection@invalid"
git -C "$REPO" config user.name "check-live-lane-selection"

# The recorder that stands in for Nx while case 5 drives the real recipes. Committed into
# the base rather than written over each fixture, so it is never part of a diff the decision
# reads, and so `git checkout -- .` between cases cannot take it away. Every recipe reaches
# Nx through scripts/nx.sh, so this is the one seam that has to be replaced: the recipes,
# the decision and the git states below are all real.
cat > "$REPO/scripts/nx.sh" <<'RECORDER'
#!/usr/bin/env bash
# Stand-in for Nx, planted by scripts/check-live-lane-selection.sh. Records the arguments
# each recipe would hand to Nx and runs nothing.
set -euo pipefail
printf '%s\n' "$*" >> "${ONETASKGRAPH_NX_RECORD:?ONETASKGRAPH_NX_RECORD is unset}"
RECORDER
chmod +x "$REPO/scripts/nx.sh"

# The plugin's current version, and the one every version fixture below bumps it to. Read
# before the base is committed, because the two decoys planted into it carry the version
# they will later be bumped from.
VERSION="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$REPO/$PLUGIN_MANIFEST" | head -n1)"
[ -n "$VERSION" ] || fatal \
  "$PLUGIN_MANIFEST declares no [package] version, so no version fixture can be built" \
  "restore that manifest's version field, then rerun"
BUMPED="$(printf '%s' "$VERSION" | python3 -c '
import re
import sys

major, minor, patch = sys.stdin.read().strip().split(".")[:3]
digits = re.match(r"[0-9]+", patch)
print("{}.{}.{}".format(major, minor, int(digits.group(0)) + 1))
')" || fatal \
  "could not derive a bumped version from $VERSION" \
  "restore $PLUGIN_MANIFEST to an X.Y.Z version, then rerun"
readonly VERSION BUMPED

# The decoys, in the base so that a later case can MODIFY them: an added file answers `run`
# on its status alone, which would prove nothing about where the file lives. Each carries a
# line the release-shaped substitution below moves, so the only thing telling them from the
# manifests and changelogs beside them is their path.
mkdir -p "$REPO/$(dirname "$NESTED_MANIFEST")" "$REPO/$(dirname "$OUTSIDE_CHANGELOG")" || fatal \
  "could not make room for the decoy files in $REPO" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
cat > "$REPO/$NESTED_MANIFEST" <<DECOY || fatal "could not plant $NESTED_MANIFEST" "check 'df -h', then rerun"
# A fixture for a test, not a crate of this workspace.
[package]
name = "a-fixture-that-is-not-a-workspace-crate"
version = "$VERSION"
DECOY
cat > "$REPO/$OUTSIDE_CHANGELOG" <<DECOY || fatal "could not plant $OUTSIDE_CHANGELOG" "check 'df -h', then rerun"
# Changelog

## $VERSION

- the Python package's own log, which is not a workspace crate's
DECOY

git -C "$REPO" add -A || fatal \
  "could not stage the overlaid tree in $REPO, so no case could be undone after it ran" \
  "check 'df -h' for free space, then rerun"
git -C "$REPO" commit --quiet --no-verify -m "test: the tree every case below starts from" \
  || fatal \
    "could not commit the base state in $REPO" \
    "check 'df -h' for free space, then rerun"

BASE="$(git -C "$REPO" rev-parse HEAD)" || fatal \
  "could not read the base commit of $REPO" \
  "rerun; if it persists, check 'df -h' for a full disk"
readonly BASE

failures=0
fail() {
  echo "check-live-lane-selection: $1" >&2
  failures=$((failures + 1))
}

# Rewrite one file in the scratch tree with python, which every platform here spells the
# same way — `sed -i` differs between GNU and BSD and fails on the macOS runner. The
# payloads travel as files rather than as arguments, because MSYS2 rewrites an argument
# that looks like a path; scripts/check-live-lane-enforced.sh records what that cost.
rewrite() {
  local relative="$1" before="$2" after="$3"
  printf '%s' "$before" > "$scratch/before"
  printf '%s' "$after" > "$scratch/after"
  python3 - "$REPO/$relative" "$scratch/before" "$scratch/after" <<'PY' || fatal \
    "could not rewrite $relative in the scratch tree, so that fixture never landed" \
    "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
import sys
from pathlib import Path

target, before, after = (Path(argument) for argument in sys.argv[1:4])
text = target.read_text(encoding="utf-8")
needle = before.read_text(encoding="utf-8")
if needle not in text:
    raise SystemExit(f"{target} does not contain {needle!r}")
target.write_text(text.replace(needle, after.read_text(encoding="utf-8")), encoding="utf-8")
PY
}

append() {
  printf '%s' "$2" >> "$REPO/$1" || fatal \
    "could not append to $1 in the scratch tree, so that fixture never landed" \
    "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
}

# Every `version = "<old>"` line of a file becomes `version = "<new>"`. That is what a
# release does to Cargo.lock, and doing it line for line here is what makes the fixture the
# diff the decision is asked about rather than an approximation of it.
bump_version_lines() {
  local relative="$1"
  ONETASKGRAPH_OLD="$VERSION" ONETASKGRAPH_NEW="$BUMPED" python3 - "$REPO/$relative" <<'PY' \
    || fatal "could not bump the version lines of $relative" "rerun; if it persists, check 'df -h'"
import os
import re
import sys
from pathlib import Path

target = Path(sys.argv[1])
old = re.escape(os.environ["ONETASKGRAPH_OLD"])
new = os.environ["ONETASKGRAPH_NEW"]
text = target.read_text(encoding="utf-8")
target.write_text(
    re.sub(rf'(?m)^version = "{old}"$', f'version = "{new}"', text), encoding="utf-8"
)
PY
}

commit_fixture() {
  git -C "$REPO" add -A >/dev/null 2>&1 && \
    git -C "$REPO" commit --quiet --no-verify -m "test: $1" || fatal \
      "could not commit the fixture '$1' in $REPO" \
      "check 'df -h' for free space, then rerun"
}

reset_fixture() {
  git -C "$REPO" reset --quiet --hard "$BASE" || fatal \
    "could not restore $REPO to its base, so every case after this one would inherit a fixture" \
    "check 'df -h' for free space, then rerun"
}

# ---------------------------------------------------------------------------------------
# The fixtures. Each is one diff against $BASE, named for the answer it is owed.

# Only the plugin's own manifest version, its changelog, and the lockfile's version lines.
fixture_version_only() {
  rewrite "$PLUGIN_MANIFEST" "version = \"$VERSION\"" "version = \"$BUMPED\""
  append "$PLUGIN_CHANGELOG" "
## $BUMPED

- released
"
  bump_version_lines Cargo.lock
  commit_fixture "a version-only diff"
}

# That, and a dependency added to the plugin's own manifest.
fixture_manifest_dependency() {
  fixture_version_only
  append "$PLUGIN_MANIFEST" "
[dependencies.serde_json]
version = \"1\"
"
  commit_fixture "a dependency added to the plugin's own manifest"
}

# That, and a line of the plugin's own manifest CHANGED rather than added — the case a
# version bump is most easily mistaken for, since it is one line for one line.
fixture_manifest_edited_line() {
  fixture_version_only
  rewrite "$PLUGIN_MANIFEST" 'edition.workspace = true' 'edition = "2021"'
  commit_fixture "a line of the plugin's own manifest edited beside the bump"
}

# That, and the plugin's own source.
fixture_plugin_source() {
  fixture_version_only
  append "$PLUGIN_SOURCE" "
// touched by scripts/check-live-lane-selection.sh
"
  commit_fixture "the plugin's own source, beside the bump"
}

# That, and another crate's manifest beside its version line.
fixture_other_manifest() {
  fixture_version_only
  bump_version_lines "$CORE_MANIFEST"
  append "$CORE_MANIFEST" "
[dependencies.serde_json]
version = \"1\"
"
  commit_fixture "another crate's manifest beside its version line"
}

# Only version lines and changelogs, across the whole workspace.
fixture_other_versions_and_changelogs() {
  fixture_version_only
  bump_version_lines "$CORE_MANIFEST"
  bump_version_lines Cargo.toml
  append "$CORE_CHANGELOG" "
## $BUMPED

- released
"
  commit_fixture "other crates' version lines and changelogs beside the bump"
}

# That, and the one line of an outside SOURCE file a release bumps: the Python SDK's
# `__version__`. Every changed line in this diff is the same line with the version
# substituted, so a decision that read a line's shape rather than the file it sits in would
# call this a version bump. It is not one the contract permits: outside the crate only the
# workspace lockfile, other crates' manifests and changelogs may move at all.
fixture_outside_source_version() {
  fixture_version_only
  bump_version_lines "$CORE_MANIFEST"
  rewrite "$OUTSIDE_SOURCE" "__version__ = \"$VERSION\"" "__version__ = \"$BUMPED\""
  commit_fixture "a version-looking edit in an outside source file"
}

# That, and the version line of a `Cargo.toml` that is a test fixture rather than a crate's
# own manifest. Same basename as a permitted path, same substitution, wrong location.
fixture_nested_manifest_basename() {
  fixture_version_only
  bump_version_lines "$NESTED_MANIFEST"
  commit_fixture "a Cargo.toml that is not a workspace crate's own manifest"
}

# That, and a changelog belonging to a package that is not a workspace crate. The changelog
# exception is the widest one the contract grants — any change, any status — so it is the
# one where a basename match would cost the most.
fixture_outside_changelog_basename() {
  fixture_version_only
  append "$OUTSIDE_CHANGELOG" "
## $BUMPED

- released
"
  commit_fixture "a CHANGELOG.md that is not a workspace crate's own"
}

# The engine alone, which reaches no plugin.
fixture_core_source() {
  append "$CORE_SOURCE" "
// touched by scripts/check-live-lane-selection.sh
"
  commit_fixture "the engine's own source"
}

# ---------------------------------------------------------------------------------------
# 2 and 3. The decision's answers, put to the real script over those real git states.

decide() {
  (cd "$REPO" && bash scripts/live-lane-selection.sh "$PLUGIN" "$BASE" 2>"$scratch/decide-stderr")
}

expect_answer() {
  local case_name="$1" expected="$2" answer
  answer="$(decide)" || answer="<the decision could not be run>"
  if [ "$answer" != "$expected" ]; then
    fail "$case_name — the decision answered '$answer', expected '$expected'. It said: $(cat "$scratch/decide-stderr")"
  fi
}

echo "check-live-lane-selection: putting the decision each diff it must tell apart" >&2

fixture_version_only
expect_answer "a version-only diff" not-selected
# The report is the lane's own words, and it must not read as a missing credential: that
# is a different outcome with a different repair, and ONETASKGRAPH_LIVE_REQUIRED keeps its
# own meaning so a defect here cannot turn an absent credential into an unselected lane.
if ! grep -q "NOT SELECTED" "$scratch/decide-stderr"; then
  fail "a version-only diff did not report that the lane was not selected: $(cat "$scratch/decide-stderr")"
fi
if ! grep -q "not a missing credential" "$scratch/decide-stderr"; then
  fail "a not-selected lane did not distinguish itself from a lane whose credential is missing: $(cat "$scratch/decide-stderr")"
fi
reset_fixture

fixture_manifest_dependency
expect_answer "a dependency added to the plugin's own manifest" run
reset_fixture

fixture_manifest_edited_line
expect_answer "a line of the plugin's own manifest edited beside the bump" run
reset_fixture

fixture_plugin_source
expect_answer "the plugin's own source" run
reset_fixture

fixture_other_manifest
expect_answer "another crate's manifest beside its version line" run
reset_fixture

fixture_other_versions_and_changelogs
expect_answer "other crates' version lines and changelogs beside the bump" not-selected
if ! grep -q "NOT SELECTED" "$scratch/decide-stderr"; then
  fail "a workspace-wide version bump did not report that the lane was not selected: $(cat "$scratch/decide-stderr")"
fi
reset_fixture

# The pair the case above exists to be told apart from: the same substitution, applied to a
# file the contract does not permit. Both diffs consist of nothing but version lines, so
# only the permitted SET distinguishes them.
fixture_outside_source_version
expect_answer "a version-looking edit in an outside source file" run
if grep -q "NOT SELECTED" "$scratch/decide-stderr"; then
  fail "a version-looking edit in an outside source file was reported as not selected, so the permitted set is being read from a line's shape rather than from the file: $(cat "$scratch/decide-stderr")"
fi
reset_fixture

# The same distinction one level in: a basename is not a location. Both files below are
# called exactly what a permitted path is called and neither is one, so a decision matching
# on the name would accept a diff that reaches whatever those files really are.
fixture_nested_manifest_basename
expect_answer "a Cargo.toml that is not a workspace crate's own manifest" run
if grep -q "NOT SELECTED" "$scratch/decide-stderr"; then
  fail "a nested Cargo.toml that is no crate's manifest was accepted, so the permitted set is being matched by basename rather than by path: $(cat "$scratch/decide-stderr")"
fi
reset_fixture

fixture_outside_changelog_basename
expect_answer "a CHANGELOG.md that is not a workspace crate's own" run
if grep -q "NOT SELECTED" "$scratch/decide-stderr"; then
  fail "a CHANGELOG.md outside every workspace crate was accepted, so the changelog exception is being matched by basename rather than by path: $(cat "$scratch/decide-stderr")"
fi
reset_fixture

# 3. A question it cannot answer. Two of them: a base that names no commit, and a crate
# this repository does not have. Both answer `run`, because failing toward the budget is
# the only direction a wrong answer is recoverable in.
for unanswerable in "no-such-base-ref-for-this-check" "$BASE"; do
  argument="$PLUGIN"
  [ "$unanswerable" = "$BASE" ] && argument="onetaskgraph-no-such-crate"
  answer="$(cd "$REPO" && bash scripts/live-lane-selection.sh "$argument" "$unanswerable" 2>"$scratch/decide-stderr")" \
    || answer="<the decision could not be run>"
  if [ "$answer" != "run" ]; then
    fail "an unanswerable question ($argument against $unanswerable) answered '$answer', expected 'run'. It said: $(cat "$scratch/decide-stderr")"
  fi
done

# ---------------------------------------------------------------------------------------
# 4. Every path that can open a live session consults the decision.
#
# Enumerated from the workflow, the hook and the justfile rather than listed here: a path
# added or rewired later has to appear in one of those three, and this fails when one of
# them reaches the live lane without reading the answer.

echo "check-live-lane-selection: enumerating the paths that can open a live session" >&2

if ! (cd "$REPO" && python3 - <<'PY'
import json
import re
import sys
from pathlib import Path

DECISION = "scripts/live-lane-selection.sh"
WORKFLOW = Path(".github/workflows/ci.yml")
HOOK = Path(".githooks/pre-push")
JUSTFILE = Path("justfile")

problems = []


def read(path, what):
    try:
        return path.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError) as problem:
        print(f"check-live-lane-selection: could not read {path.as_posix()}: {problem}.", file=sys.stderr)
        print(f"check-live-lane-selection: restore {what}, or point this check at where it moved to.", file=sys.stderr)
        raise SystemExit(1) from None


# Which variables a live lane reads, from the crates that declare they have one. The `live:`
# tag is how a crate says so — scripts/check-live-lane.sh reconciles those tags against the
# journeys both ways — and the `test` target's own `env` inputs are what that lane is keyed
# on, so a credential this check must watch is derived from the graph rather than restated.
live_variables = set()
for project_file in sorted(Path("crates").glob("*/project.json")):
    project = json.loads(read(project_file, "that project's configuration"))
    if not any(
        isinstance(tag, str) and tag.startswith("live:") for tag in project.get("tags", [])
    ):
        continue
    for entry in project.get("targets", {}).get("test", {}).get("inputs", []):
        if isinstance(entry, dict) and isinstance(entry.get("env"), str):
            live_variables.add(entry["env"])
if not live_variables:
    problems.append(
        "no crate declares a live session's variables as `env` inputs of its `test` target, "
        "so this check cannot tell which workflow steps can open one"
    )

# The justfile's recipe graph: which recipes reach the decision, directly or through a
# dependency. `just check` and `just gate` both have to, because those are what every path
# below runs.
recipes = {}
current = None
for line in read(JUSTFILE, "the command surface").splitlines():
    header = re.match(r"^([a-z][a-z0-9-]*)(\s+[^:]*)?:(.*)$", line)
    if header and not line.startswith((" ", "\t")):
        current = header.group(1)
        recipes[current] = {"deps": header.group(3).split(), "body": []}
        continue
    if current and (line.startswith((" ", "\t"))):
        recipes[current]["body"].append(line)
    elif not line.strip():
        continue
    elif not line.startswith(("#", " ", "\t")):
        current = None


def reaches_decision(recipe, seen=None):
    seen = seen or set()
    if recipe in seen or recipe not in recipes:
        return False
    seen.add(recipe)
    if any(DECISION in line for line in recipes[recipe]["body"]):
        return True
    return any(reaches_decision(dependency, seen) for dependency in recipes[recipe]["deps"])


# `just` at a command position, so a recipe named in prose or inside a diagnostic is not
# mistaken for one that runs. The hook explains itself at length and says "just is not
# installed" in a message; a loose match read both as invocations.
INVOCATION = re.compile(r"(?m)^[ \t]*(?:if\s+!\s+|!\s+)?just\s+([a-z][a-z0-9-]*)")


def without_comments(text):
    """A file's instructions without its explanations, as check-live-lane.sh reads them."""
    return "\n".join(line.split("#", 1)[0] for line in text.splitlines())


def consults(command, where):
    """Whether a `just <recipe>` command line ends up reading the decision."""
    invoked = INVOCATION.findall(command)
    if not invoked:
        problems.append(
            f"{where}: can open a live session but runs no `just` recipe, so nothing here "
            "can tell whether it consults the decision"
        )
        return
    for recipe in invoked:
        if not reaches_decision(recipe):
            problems.append(
                f"{where}: runs `just {recipe}`, which never reaches {DECISION}. Every path "
                "that can open a live session has to read that answer, or a release's "
                "version-only diff opens a session against a real API again"
            )


# The workflow's steps, split the way scripts/check-live-lane.sh splits them.
workflow = read(WORKFLOW, "the workflow that runs the required check")
steps = re.split(r"(?m)^      - (?=name:|uses:)", workflow)
credentialed = [
    step
    for step in steps
    if any(f"secrets.{variable}" in step for variable in live_variables)
]
if not credentialed:
    problems.append(
        f"{WORKFLOW.as_posix()}: no step passes any of {sorted(live_variables)}, so either the "
        "live lanes reach no API from CI at all or this check is watching the wrong names"
    )
for step in credentialed:
    named = re.search(r"(?m)^\s*name:\s*(.+?)\s*$", step)
    where = f"{WORKFLOW.as_posix()} step {named.group(1)!r}" if named else WORKFLOW.as_posix()
    run = re.search(r"(?ms)^\s*run:\s*(.*?)(?=\n\s*(?:-\s|\w+:)|\Z)", step)
    consults(run.group(1) if run else "", where)
    # An implicit base is how affected selection quietly starts comparing against the wrong
    # commit, and on the default branch it compares against this very commit and selects
    # nothing at all. Every credentialed step names the base it derived.
    if not re.search(r"(?m)^\s*NX_BASE:", step):
        problems.append(
            f"{where}: can open a live session and sets no NX_BASE, so both Nx and the "
            "decision fall back to a default base rather than the commit this run is about"
        )

hook = read(HOOK, "the pre-push hook that runs the gate locally")
consults(without_comments(hook), HOOK.as_posix())
if "NX_BASE" not in hook:
    problems.append(
        f"{HOOK.as_posix()}: runs the gate without deriving a base, so it sweeps every project "
        "and opens a live session whatever the push contains"
    )

if problems:
    print("check-live-lane-selection: a path can reach a live session without consulting the decision.", file=sys.stderr)
    for problem in problems:
        print(f"  {problem}", file=sys.stderr)
    raise SystemExit(1)
PY
); then
  fail "the paths that can open a live session do not all consult the decision — see above."
fi

# ---------------------------------------------------------------------------------------
# 5. Each of those paths, driven over both answers.
#
# The recipes are real, the decision is real and the git states are real; Nx is the
# recorder planted above, so what a path would have asked Nx to run is readable without
# building the workspace. A path is named by the recipe the workflow or the hook runs it
# through, which is what case 4 has just held them to.

drive() {
  local recipe="$1" base="$2"
  : > "$NX_LOG"
  (cd "$REPO" && ONETASKGRAPH_NX_RECORD="$NX_LOG" NX_BASE="$base" just "$recipe") \
    >"$scratch/drive-output" 2>&1 || true
}

# The `affected -t test` line out of what the recipes handed the recorder — the one
# invocation that can run a live journey.
recorded_test_invocation() {
  grep -- "affected -t test" "$NX_LOG" || true
}

expect_path() {
  local path_name="$1" recipe="$2" base="$3" expectation="$4"
  drive "$recipe" "$base"
  local invocation
  invocation="$(recorded_test_invocation)"
  if [ -z "$invocation" ]; then
    fail "$path_name — \`just $recipe\` never ran the tests at all, so nothing here says whether the lane was selected. It printed: $(cat "$scratch/drive-output")"
    return
  fi
  case "$expectation" in
    not-selected)
      if ! grep -q "NOT SELECTED" "$scratch/drive-output"; then
        fail "$path_name — a diff the decision refuses left the lane saying nothing about not being selected. It printed: $(cat "$scratch/drive-output")"
      fi
      case "$invocation" in
        *"--exclude="*"$PLUGIN"*) ;;
        *) fail "$path_name — a diff the decision refuses still asked Nx to run the lane: $invocation" ;;
      esac
      ;;
    run)
      if grep -q "NOT SELECTED" "$scratch/drive-output"; then
        fail "$path_name — a diff the decision accepts reported the lane as not selected. It printed: $(cat "$scratch/drive-output")"
      fi
      case "$invocation" in
        *"--exclude="*) fail "$path_name — a diff the decision accepts still excluded a lane: $invocation" ;;
      esac
      ;;
  esac
}

# Nothing the sweep used to check may go unchecked now that the gate selects. The targets
# that deliberately sit outside affected selection are exactly the ones a recipe runs BY
# NAME rather than through `nx affected -t <phase>` — the supply chain and the two
# distribution stages, plus the generator on the Linux aggregate — so they are read out of
# the justfile and nx.json rather than listed here. Reading the whole command surface, not
# only what the gate reaches, is what makes this catch the regression it is for: a stage
# dropped from the gate's own dependencies still has its recipe, and so is still expected.
expect_unconditional_targets() {
  local named target
  named="$(cd "$REPO" && python3 - <<'NAMED'
import json
import re
from pathlib import Path

# The phases `check` fans out over, from nx.json itself. A target named by hand that is one
# of these is a by-hand entry point for a phase — `just script-check` is the one — rather
# than a stage outside selection, and the gate reaches it through `affected` instead.
fan_out = set(
    json.loads(Path("nx.json").read_text(encoding="utf-8"))
    .get("targetDefaults", {})
    .get("check", {})
    .get("dependsOn", [])
)
named = set()
for line in Path("justfile").read_text(encoding="utf-8").splitlines():
    if line.lstrip().startswith("#"):
        continue
    for reference in re.findall(r"run\s+([a-z][a-z0-9-]*):([a-z][a-z0-9-]*)", line):
        if reference[1] not in fan_out:
            named.add(":".join(reference))
print("\n".join(sorted(named)))
NAMED
)"
  if [ -z "$named" ]; then
    fail "no recipe runs a target by name any more, so the supply-chain and distribution stages no longer sit outside affected selection at all."
    return
  fi
  for target in $named; do
    if ! grep -qx -- "run $target" "$NX_LOG"; then
      fail "the gate did not run $target, which sits outside affected selection on purpose and has to run on every gate. It ran: $(tr '\n' ';' < "$NX_LOG")"
    fi
  done
}

# The pre-push path derives its base from the records git feeds the hook on stdin, so that
# derivation is driven with real refs before the recipe is.
pre_push_base() {
  local head
  head="$(git -C "$REPO" rev-parse HEAD)"
  printf 'refs/heads/checked %s refs/heads/checked %s\n' "$head" "$BASE" \
    | (cd "$REPO" && bash scripts/pre-push-base.sh)
}

echo "check-live-lane-selection: driving each path over both answers" >&2

for expectation in not-selected run; do
  if [ "$expectation" = "not-selected" ]; then
    fixture_version_only
  else
    fixture_plugin_source
  fi

  derived="$(pre_push_base)"
  if [ "$derived" != "$BASE" ]; then
    fail "the pre-push path derived '$derived' from the ref git would feed it, expected $BASE"
  fi

  # The change-request path runs `just check`; the default branch and the hook run
  # `just gate`. Case 4 holds the workflow and the hook to exactly those.
  expect_path "the change-request path" check "$BASE" "$expectation"
  expect_path "the default-branch path" gate "$BASE" "$expectation"
  expect_path "the local pre-push path" gate "$derived" "$expectation"

  reset_fixture
done

# Selecting must not have taken anything away from the gate. `gate-generated` is the Linux
# aggregate the default branch runs, and it is the recipe that reaches every stage held
# outside affected selection.
fixture_plugin_source
drive gate-generated "$BASE"
expect_unconditional_targets
reset_fixture

# ---------------------------------------------------------------------------------------
# 1. Real Nx over real repository states.
#
# Windows cannot run this section, and the obstacle is the runner rather than the graph:
# the severance below needs a real copy of node_modules, bun's layout there is hundreds of
# symlinks, and creating one needs a privilege the runner does not grant.
# scripts/check-affected-selection.sh skips wholly for the same reason; the sections above
# need no node_modules and run on every platform.
case "${OS:-}${OSTYPE:-}" in
  *Windows_NT* | *msys* | *cygwin* | *win32*)
    echo "check-live-lane-selection: the real-Nx cases are skipped on Windows (bun's node_modules is a symlink tree the runner cannot copy); the Linux and macOS lanes gate them" >&2
    ;;
  *)
    if [ ! -d "$ROOT/node_modules" ]; then
      fatal \
        "there is no node_modules to copy into the scratch tree, so the real-Nx cases cannot run" \
        "run 'just bootstrap' (or 'bun install'), then rerun"
    fi
    # A real copy rather than a symlink, and the daemon, the cache and the workspace-data
    # directory all severed: this check is an Nx target, so the invocations below nest
    # inside the sweep that started it, and an inner Nx resolving back to the outer
    # worktree waits on a lock nothing will release. See scripts/check-affected-selection.sh.
    cp -a "$ROOT/node_modules" "$REPO/node_modules" || fatal \
      "could not copy node_modules into $REPO, so the real-Nx cases cannot run" \
      "check 'df -h' for free space, then rerun"
    export NX_DAEMON=false
    export NX_CACHE_DIRECTORY="$scratch/nx-cache"
    export NX_WORKSPACE_DATA_DIRECTORY="$scratch/nx-workspace-data"

    # Every project real Nx selects a `test` target for, for the diff now in the tree.
    selected_test_projects() {
      local raw
      if ! raw="$(cd "$REPO" && node_modules/.bin/nx show projects --affected --json \
        -t test --base="$BASE" --head=HEAD "$@" 2>"$scratch/nx-stderr")"; then
        fatal \
          "Nx could not compute the affected set with a test target: $(cat "$scratch/nx-stderr")" \
          "fix the project graph so 'nx show projects' runs, then rerun"
      fi
      printf '%s' "$raw" | python3 -c '
import json, sys
print("\n".join(sorted(json.load(sys.stdin))))
' | tr -d '\r'
    }

    selects() {
      case "
$1
" in
        *"
$2
"*) return 0 ;;
      esac
      return 1
    }

    echo "check-live-lane-selection: putting three diffs to real Nx" >&2

    fixture_core_source
    selection="$(selected_test_projects)"
    if selects "$selection" "$PLUGIN"; then
      fail "a diff of the engine's own source selected $PLUGIN's test target, which reaches a real API. Selected: $(printf '%s' "$selection" | tr '\n' ' ')"
    fi
    reset_fixture

    fixture_plugin_source
    selection="$(selected_test_projects)"
    if ! selects "$selection" "$PLUGIN"; then
      fail "a diff of $PLUGIN's own source did not select its test target, so narrowing what runs has narrowed what is checked. Selected: $(printf '%s' "$selection" | tr '\n' ' ')"
    fi
    reset_fixture

    # The version-only diff is why the decision exists: Cargo.lock is a `sharedGlobals`
    # input, so Nx alone selects every project — and the decision's own `--exclude` is what
    # takes the lane back out.
    fixture_version_only
    selection="$(selected_test_projects)"
    if ! selects "$selection" "$PLUGIN"; then
      fail "a version-only diff did not select $PLUGIN through Nx alone, which is the premise this decision exists on — if that has changed, say so here rather than leaving the case asserting nothing."
    fi
    exclusions="$(cd "$REPO" && bash scripts/live-lane-selection.sh --nx-exclusions "$BASE" 2>/dev/null)"
    if [ -z "$exclusions" ]; then
      fail "the decision produced no Nx exclusion for a version-only diff, so nothing would take the lane out of the selection."
    else
      # shellcheck disable=SC2086  # one Nx flag, produced by the decision, deliberately split.
      selection="$(selected_test_projects $exclusions)"
      if selects "$selection" "$PLUGIN"; then
        fail "with the decision's own exclusion applied, real Nx still selected $PLUGIN's test target. Selected: $(printf '%s' "$selection" | tr '\n' ' ')"
      fi
    fi
    reset_fixture
    ;;
esac

if [ "$failures" -ne 0 ]; then
  echo "check-live-lane-selection: $failures expectation(s) failed." >&2
  echo "check-live-lane-selection: the arrangement is: every path derives a base and runs a" >&2
  echo "check-live-lane-selection: recipe that consults scripts/live-lane-selection.sh, and that" >&2
  echo "check-live-lane-selection: decision answers 'not-selected' only for a version bump and its" >&2
  echo "check-live-lane-selection: changelogs. AGENTS.md records why. Fix whichever the failure" >&2
  echo "check-live-lane-selection: above names, then rerun 'just script-check'." >&2
  exit 1
fi
