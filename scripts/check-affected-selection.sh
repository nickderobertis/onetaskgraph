#!/usr/bin/env bash
# Prove the four selections the project graph exists to produce, against real Nx.
#
# Splitting `onetaskgraph-plugin-api` out of `onetaskgraph-core` buys exactly one thing,
# and it is a build-graph thing:
#
#   1. Editing the contract crate marks EVERY plugin affected — the interface moved.
#   2. Editing the engine outside it marks NO plugin affected. This is the return on the
#      split, and it is the one that fails silently: an over-broad implicitDependencies
#      entry, or a namedInputs glob reaching past its own crate, makes every engine commit
#      run every plugin's tests and nothing complains.
#   3. Editing one plugin marks that plugin and its dependents — never a sibling plugin.
#
# The fourth is the scripts project's own, and it fails the same silent way:
#
#   4. Editing a script marks `scripts` — and `workspace`, which invokes a dozen of them —
#      and NO crate. Before scripts/ was a project, Nx mapped none to it and a script-only
#      change selected nothing at all, so the guards it changed never ran; an edge added
#      the other way would swing it past that into re-testing every crate.
#
# A project graph that looks right and selects wrong is the expensive failure here, so
# this makes real edits in a scratch clone, commits them, and runs the real affected
# selection against them. Reading nx.json and reasoning about it does not prove anything.
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# What this proves is a property of the Nx PROJECT GRAPH — which edit selects which
# project — and that graph is read from nx.json and the project.json files, which are
# byte-identical on every platform. ci.yml keeps `deny` Linux-only for exactly this
# reason: "the graph is the same on every platform, so running it three times would buy
# nothing."
#
# On Windows it cannot run at all, and the obstacle is the runner rather than the graph:
# the severance below needs a REAL copy of node_modules, bun's layout there is hundreds of
# symlinks, and creating one on Windows needs a privilege the runner does not grant — so
# `cp -a` fails with "cannot create symbolic link". Dereferencing instead is not a fix,
# because those links form cycles and such a copy would not terminate.
#
# So it is skipped there with a notice, as scripts/rust-coverage.sh skips measurement on
# Windows for its own platform reason. The Linux and macOS lanes run it on every pull
# request, so the four selections AGENTS.md owes stay gated on every change.
case "${OS:-}${OSTYPE:-}" in
  *Windows_NT* | *msys* | *cygwin* | *win32*)
    echo "check-affected-selection: skipped on Windows (bun's node_modules is a symlink tree the runner cannot copy); the Linux and macOS lanes gate the project graph" >&2
    exit 0
    ;;
esac
# From scripts/plugin-crates.sh, so a plugin added later is covered without an edit here.
# llmlint: ignore[boundary_inputs_validated] these names are not external input:
# scripts/plugin-crates.sh reads them from this repository's own committed
# project.json files, scripts/check-workspace-config.sh reconciles those files on
# every `check`, and cargo refuses an invalid package name loudly — that refusal is
# the very failure the `tr` here exists to fix.
# tr: see scripts/check-plugin-isolation.sh — python's stdout is CRLF on Windows.
# The path is assembled from $ROOT at runtime, so shellcheck cannot resolve it. Naming
# the file has it follow and check read-lines.sh (SC1091) rather than skip it unread.
# shellcheck source=scripts/read-lines.sh
# Tested before it is sourced, not merely guarded after: bash 3.2 ends the shell where
# `source` cannot find its file, so the handler a later bash takes never runs there — and
# macos-latest is a 3.2 runner. Without this the reader gets bash's own "No such file or
# directory", which names the sourcing line rather than the file to put back.
if [ ! -r "$ROOT/scripts/read-lines.sh" ] || ! source "$ROOT/scripts/read-lines.sh"; then
  echo "check-affected-selection: could not load $ROOT/scripts/read-lines.sh, which reads the" >&2
  echo "check-affected-selection: plugin set into an array." >&2
  echo "check-affected-selection: restore it with 'git checkout -- scripts/read-lines.sh', then re-run." >&2
  exit 1
fi
read_lines PLUGINS < <(bash "$ROOT/scripts/plugin-crates.sh" | tr -d '\r')

scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT

# The committed tree, through scripts/scratch-clone.sh, which strips the git environment
# first: under a hook GIT_DIR overrides `git -C`, and every git command below would run
# against the real repository instead. See that file.
# shellcheck source=scripts/scratch-clone.sh
source "$ROOT/scripts/scratch-clone.sh"
scratch_clone "$ROOT" "$scratch/repo"

# This check is an Nx target, so the invocation below nests inside the sweep that started
# it. That is safe only while the inner Nx shares no state with the outer one: the outer
# `run-many` holds the workspace's native state for the whole gate, so an inner Nx that
# resolves back to the outer worktree waits on a lock nothing will release, and no timeout
# exists in the target, in `just gate` or in the hook to end it.
#
# Three shared paths, all severed: the daemon, the cache and the workspace-data directory
# below, and a real copy of node_modules rather than a symlink here, because Nx locates its
# workspace root from where it is installed as well as from the working directory. The copy
# costs about half a second. If it ever looks wasteful, take this check out of `run-many`
# first — re-nesting is how the deadlock comes back.
cp -a "$ROOT/node_modules" "$scratch/repo/node_modules"

git -C "$scratch/repo" config user.email "check-affected-selection@invalid"
git -C "$scratch/repo" config user.name "check-affected-selection"

# `nx show projects` is a graph query, so keep it away from the daemon and the shared
# cache — a stale daemon answering for a different tree is precisely the failure mode
# this check exists to catch. NX_WORKSPACE_DATA_DIRECTORY is the third severance: it is
# the native workspace state the outer sweep locks, and the default puts it in .nx/ under
# whichever workspace root Nx picked.
export NX_DAEMON=false
export NX_CACHE_DIRECTORY="$scratch/nx-cache"
export NX_WORKSPACE_DATA_DIRECTORY="$scratch/nx-workspace-data"

failures=0

# Edit one file, commit it, and print the projects real Nx selects for that commit.
select_after_editing() {
  local file="$1"
  local base
  base="$(git -C "$scratch/repo" rev-parse HEAD)"

  printf '\n// touched by scripts/check-affected-selection.sh\n' >> "$scratch/repo/$file"
  git -C "$scratch/repo" add "$file"
  git -C "$scratch/repo" commit --quiet --no-verify -m "test: touch $file"

  # --json, then one name per line: `nx show projects` renders a JSON array outside a
  # TTY, and a shape-dependent parse is exactly the kind of quiet breakage this check
  # exists to catch.
  local raw stderr_file
  stderr_file="$scratch/nx-stderr"
  if ! raw="$(cd "$scratch/repo" && node_modules/.bin/nx show projects --affected --json \
    --base="$base" --head=HEAD 2>"$stderr_file")"; then
    echo "check-affected-selection: Nx could not compute the affected set for $file:" >&2
    printf '%s\n' "$raw" >&2
    cat "$stderr_file" >&2
    echo "check-affected-selection: fix the project graph so 'nx show projects' runs, then re-run." >&2
    exit 1
  fi
  # tr: the caller compares these names line for line, which a trailing CR defeats.
  if ! printf '%s' "$raw" \
    | python3 -c '
import json, sys
projects = json.load(sys.stdin)
if not isinstance(projects, list) or not all(isinstance(n, str) for n in projects):
    raise SystemExit("not a JSON array of project names")
print("\n".join(sorted(projects)))
' \
    | tr -d '\r'; then
    echo "check-affected-selection: 'nx show projects --affected --json' answered with something other than a JSON array of project names:" >&2
    printf '%s\n' "$raw" >&2
    echo "check-affected-selection: fix the project graph so that command returns JSON, then re-run." >&2
    exit 1
  fi
}

# Undo the scratch commit so each case starts from the same committed tree.
reset_scratch() {
  git -C "$scratch/repo" reset --quiet --hard HEAD~1
}

# Line-exact membership, decided in this shell rather than by `printf | grep -q`. Under
# `pipefail` that pipeline reports its writer's status as well as the matcher's, so a quiet
# grep's early exit — or any transient failure of the matcher itself — reads as "the
# project was not selected" and refuses a project graph that is correct. An exact-line
# match needs no subprocess, so the verdict below is a function of the selection alone.
# NL is what lets one `case` pattern anchor both ends: the selection has no trailing
# newline, so a first or last name would otherwise need a case of its own.
readonly NL='
'

selection_contains() {
  local selected="$1" project="$2"
  case "$NL$selected$NL" in
    *"$NL$project$NL"*) return 0 ;;
  esac
  return 1
}

expect_selected() {
  local case_name="$1" project="$2" selected="$3"
  if ! selection_contains "$selected" "$project"; then
    echo "check-affected-selection: $case_name — expected $project to be selected, but it was not." >&2
    failures=$((failures + 1))
  fi
}

expect_not_selected() {
  local case_name="$1" project="$2" selected="$3"
  if selection_contains "$selected" "$project"; then
    echo "check-affected-selection: $case_name — $project was selected but must not be." >&2
    failures=$((failures + 1))
  fi
}

# Quiet on success. The selections are worth seeing when an expectation breaks — they
# are the evidence for which way the graph went wrong — and are noise otherwise, so they
# are held and printed only if this case failed.
report_on_failure() {
  local case_name="$1" selected="$2" before="$3"
  if [ "$failures" -ne "$before" ]; then
    echo "check-affected-selection: $case_name selected: $(printf '%s' "$selected" | tr '\n' ' ')" >&2
  fi
}

# 1. The contract moved, so every implementation of it re-runs.
selection="$(select_after_editing crates/onetaskgraph-plugin-api/src/lib.rs)"
before=$failures
for plugin in "${PLUGINS[@]}"; do
  expect_selected "editing the contract crate" "$plugin" "$selection"
done
expect_selected "editing the contract crate" onetaskgraph-core "$selection"
expect_selected "editing the contract crate" onetaskgraph "$selection"
report_on_failure "editing onetaskgraph-plugin-api" "$selection" "$before"
reset_scratch

# 2. The engine changed, and no plugin can see it.
selection="$(select_after_editing crates/onetaskgraph-core/src/registry.rs)"
before=$failures
for plugin in "${PLUGINS[@]}"; do
  expect_not_selected "editing the engine" "$plugin" "$selection"
done
expect_selected "editing the engine" onetaskgraph-core "$selection"
expect_selected "editing the engine" onetaskgraph "$selection"
report_on_failure "editing onetaskgraph-core" "$selection" "$before"
reset_scratch

# 3. One plugin changed; its siblings are untouched.
selection="$(select_after_editing crates/onetaskgraph-linear/src/lib.rs)"
before=$failures
expect_selected "editing one plugin" onetaskgraph-linear "$selection"
expect_selected "editing one plugin" onetaskgraph "$selection"
for plugin in "${PLUGINS[@]}"; do
  [ "$plugin" = "onetaskgraph-linear" ] && continue
  expect_not_selected "editing one plugin" "$plugin" "$selection"
done
report_on_failure "editing onetaskgraph-linear" "$selection" "$before"
reset_scratch

# 4. A script changed, and the graph owns it. scripts/read-lines.sh is deliberately one no
#    other project names in its own inputs, so what this reads is the project root mapping
#    rather than an input glob that happens to mention it.
selection="$(select_after_editing scripts/read-lines.sh)"
before=$failures
expect_selected "editing a script" scripts "$selection"
expect_selected "editing a script" workspace "$selection"
for plugin in "${PLUGINS[@]}"; do
  expect_not_selected "editing a script" "$plugin" "$selection"
done
expect_not_selected "editing a script" onetaskgraph-core "$selection"
expect_not_selected "editing a script" onetaskgraph "$selection"
expect_not_selected "editing a script" sdk-python "$selection"
expect_not_selected "editing a script" sdk-typescript "$selection"
report_on_failure "editing scripts/read-lines.sh" "$selection" "$before"
reset_scratch

if [ "$failures" -ne 0 ]; then
  echo "check-affected-selection: $failures expectation(s) failed." >&2
  echo "check-affected-selection: the gate now silently under- or over-runs. Compare each" >&2
  echo "check-affected-selection: crate's implicitDependencies in project.json against its" >&2
  echo "check-affected-selection: Cargo dependencies, and check nx.json's namedInputs for a" >&2
  echo "check-affected-selection: glob reaching past its own crate." >&2
  exit 1
fi
