#!/usr/bin/env bash
# Prove the three selections the project graph exists to produce, against real Nx.
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
# A project graph that looks right and selects wrong is the expensive failure here, so
# this makes real edits in a scratch clone, commits them, and runs the real affected
# selection against them. Reading nx.json and reasoning about it does not prove anything.
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly PLUGINS=(
  onetaskgraph-in-memory
  onetaskgraph-local-md
  onetaskgraph-linear
  onetaskgraph-github-projects
)

scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT

# A shared clone of the committed tree: the selection must be provable from what is in
# git, not from whatever happens to be uncommitted in the working copy right now.
git clone --quiet --shared --no-checkout "$ROOT" "$scratch/repo"
git -C "$scratch/repo" checkout --quiet "$(git -C "$ROOT" rev-parse HEAD)"
# Nx resolves its own install from node_modules; borrowing the root's keeps this check
# from paying for a second `bun install` on every run.
ln -s "$ROOT/node_modules" "$scratch/repo/node_modules"

git -C "$scratch/repo" config user.email "check-affected-selection@invalid"
git -C "$scratch/repo" config user.name "check-affected-selection"

# `nx show projects` is a graph query, so keep it away from the daemon and the shared
# cache — a stale daemon answering for a different tree is precisely the failure mode
# this check exists to catch.
export NX_DAEMON=false
export NX_CACHE_DIRECTORY="$scratch/nx-cache"

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
  (cd "$scratch/repo" && node_modules/.bin/nx show projects --affected --json \
    --base="$base" --head=HEAD 2>/dev/null) \
    | python3 -c 'import json,sys; print("\n".join(sorted(json.load(sys.stdin))))'
}

# Undo the scratch commit so each case starts from the same committed tree.
reset_scratch() {
  git -C "$scratch/repo" reset --quiet --hard HEAD~1
}

expect_selected() {
  local case_name="$1" project="$2" selected="$3"
  if ! printf '%s\n' "$selected" | grep -qx "$project"; then
    echo "check-affected-selection: $case_name — expected $project to be selected, but it was not." >&2
    failures=$((failures + 1))
  fi
}

expect_not_selected() {
  local case_name="$1" project="$2" selected="$3"
  if printf '%s\n' "$selected" | grep -qx "$project"; then
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

# --- 1. the contract moved, so every implementation of it re-runs ------------------
selection="$(select_after_editing crates/onetaskgraph-plugin-api/src/lib.rs)"
before=$failures
for plugin in "${PLUGINS[@]}"; do
  expect_selected "editing the contract crate" "$plugin" "$selection"
done
expect_selected "editing the contract crate" onetaskgraph-core "$selection"
expect_selected "editing the contract crate" onetaskgraph "$selection"
report_on_failure "editing onetaskgraph-plugin-api" "$selection" "$before"
reset_scratch

# --- 2. the engine changed, and no plugin can see it -------------------------------
selection="$(select_after_editing crates/onetaskgraph-core/src/registry.rs)"
before=$failures
for plugin in "${PLUGINS[@]}"; do
  expect_not_selected "editing the engine" "$plugin" "$selection"
done
expect_selected "editing the engine" onetaskgraph-core "$selection"
expect_selected "editing the engine" onetaskgraph "$selection"
report_on_failure "editing onetaskgraph-core" "$selection" "$before"
reset_scratch

# --- 3. one plugin changed; its siblings are untouched -----------------------------
selection="$(select_after_editing crates/onetaskgraph-linear/src/lib.rs)"
before=$failures
expect_selected "editing one plugin" onetaskgraph-linear "$selection"
expect_selected "editing one plugin" onetaskgraph "$selection"
for plugin in onetaskgraph-in-memory onetaskgraph-local-md onetaskgraph-github-projects; do
  expect_not_selected "editing one plugin" "$plugin" "$selection"
done
report_on_failure "editing onetaskgraph-linear" "$selection" "$before"
reset_scratch

if [ "$failures" -ne 0 ]; then
  echo "check-affected-selection: $failures expectation(s) failed." >&2
  echo "check-affected-selection: the gate now silently under- or over-runs. Compare each" >&2
  echo "check-affected-selection: crate's implicitDependencies in project.json against its" >&2
  echo "check-affected-selection: Cargo dependencies, and check nx.json's namedInputs for a" >&2
  echo "check-affected-selection: glob reaching past its own crate." >&2
  exit 1
fi
