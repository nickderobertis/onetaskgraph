#!/usr/bin/env bash
# Enforce the two dependency-direction rules the crate split exists to establish.
#
#   1. No plugin crate depends on `onetaskgraph-core`, by any edge — normal, build or
#      dev, at any depth.
#   2. `onetaskgraph-plugin-api` depends on no other crate of this workspace.
#
# Both are read from the REAL dependency graph via `cargo metadata`, never from a list
# maintained beside it — a hand-maintained list is a rule that stops being true quietly.
# This runs inside `just check` so it fails in seconds locally; `deny.toml`'s wrapper
# restriction on `onetaskgraph-core` fails the same violation minutes later in CI, where
# `deny` is a required check. Two mechanisms because they fail at different moments.
#
# Why the rules matter: with the trait inside the engine crate, every plugin would depend
# on the engine, every engine change would mark every plugin affected, and affected
# selection would buy nothing for the six crates where it matters most.
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

readonly ENGINE_CRATE="onetaskgraph-core"

# The plugin set comes from scripts/plugin-crates.sh, so a crate added later cannot
# escape this check by not being listed here.
# llmlint: ignore[boundary_inputs_validated] these names are not external input:
# scripts/plugin-crates.sh reads them from this repository's own committed
# project.json files, scripts/check-workspace-config.sh reconciles those files on
# every `check`, and a name matching no package of this workspace is reported below by
# name — that report is the very failure the `tr` here exists to fix.
# `tr -d '\r'`: python opens stdout in text mode, so on Windows every "\n" it prints
# arrives as "\r\n". `mapfile -t` strips the newline but not the carriage return, and a
# crate name carrying a trailing CR matches no package in the graph — a failure no Linux
# or macOS run can reproduce.
mapfile -t PLUGINS < <(bash "$ROOT/scripts/plugin-crates.sh" | tr -d '\r')

metadata="$(cargo metadata --format-version 1 --no-deps --manifest-path Cargo.toml)"

violations="$(
  printf '%s' "$metadata" | PLUGINS="${PLUGINS[*]}" ENGINE="$ENGINE_CRATE" python3 -c '
import json
import os
import sys

PLUGINS = set(os.environ["PLUGINS"].split())
API = "onetaskgraph-plugin-api"
ENGINE = os.environ["ENGINE"]

metadata = json.load(sys.stdin)
workspace = {package["name"] for package in metadata["packages"]}

for package in metadata["packages"]:
    name = package["name"]
    for dependency in package["dependencies"]:
        target = dependency["name"]
        kind = dependency.get("kind") or "normal"
        if name in PLUGINS and target == ENGINE:
            print(f"{name} -> {target} ({kind}): a plugin crate may not depend on the engine")
        if name == API and target in workspace:
            print(f"{name} -> {target} ({kind}): the contract crate may depend on no other crate of this workspace")
'
)"

if [ -n "$violations" ]; then
  echo "check-plugin-isolation: the dependency direction the crate split establishes is broken." >&2
  printf '%s\n' "$violations" >&2
  echo "check-plugin-isolation: move the shared type into onetaskgraph-plugin-api, or copy" >&2
  echo "check-plugin-isolation: the helper into the plugin — the arrow only runs one way." >&2
  exit 1
fi

# Direct edges are only half the rule: an indirect path reaches the engine just as surely.
#
# One walk over the union of the edge kinds, not one per kind. A path to the engine need
# not be the same kind of edge the whole way down — a plugin dev-depending on a crate that
# normally depends on the engine reaches it at depth two — and asking for one kind alone
# cannot see that, because it stops following at the first normal edge below the dev one.
# Three separate queries therefore passed a tree that broke the rule, which is what
# scripts/check-isolation-enforced.sh caught the first time it ran; the union is the rule
# as AGENTS.md states it: any edge, at any depth.
#
# Read as data, from ONE `cargo metadata` for every plugin at once. Never render a tree
# per plugin again: `--no-dedupe` re-expands every shared subtree at every place it
# appears, so one plugin of this workspace came to 180 MB, and Windows spent 52.9 minutes
# of a 69-minute gate job pushing it through a command substitution. Never pipe a large
# rendering into a quiet `grep -q` either — it exits at the first match and SIGPIPEs the
# writer, so under `pipefail` the pipeline fails on exactly the runs that found a match.

# llmlint: ignore[changed_behavior_has_e2e] stands in for the refusal the per-plugin `cargo tree` gave, which no fixture reached either; the file that could add one, scripts/check-isolation-enforced.sh, is pinned unchanged by this node.
if ! graph="$(cargo metadata --format-version 1 --manifest-path Cargo.toml 2>&1)"; then
  echo "check-plugin-isolation: could not read the workspace dependency graph:" >&2
  printf '%s\n' "$graph" >&2
  echo "check-plugin-isolation: fix the workspace so 'cargo metadata' resolves, then re-run." >&2
  exit 1
fi

graph_violations="$(
  printf '%s' "$graph" | PLUGINS="${PLUGINS[*]}" ENGINE="$ENGINE_CRATE" python3 -c '
import json
import os
import sys
from collections import deque

PLUGINS = set(os.environ["PLUGINS"].split())
ENGINE = os.environ["ENGINE"]
PREFIX = "check-plugin-isolation:"

def path_to_engine(start):
    """The crates from `start` to the engine, innermost first, or None."""
    came_from = {start: None}
    queue = deque([start])
    while queue:
        current = queue.popleft()
        for dependency in nodes[current]["deps"]:
            target = dependency["pkg"]
            if target in came_from or target not in nodes:
                continue
            came_from[target] = current
            if names[target] == ENGINE:
                path, node = [target], target
                while came_from[node] is not None:
                    node = came_from[node]
                    path.append(node)
                return path
            queue.append(target)
    return None


# Reading the document is a boundary, and cargo is the only thing on the other side of it.
# Rather than restate the schema `--format-version 1` already fixes, every field cargo
# supplies is read inside this one block: a document that does not carry them raises, and
# the handler turns it into a refusal that names a next action rather than a traceback.
try:
    # llmlint: ignore[boundary_inputs_validated] every field cargo supplies is read inside this block, whose handler refuses with a next action; a schema check here would restate the --format-version 1 contract cargo already fixes.
    metadata = json.load(sys.stdin)
    names = {package["id"]: package["name"] for package in metadata["packages"]}
    labels = {
        package["id"]: package["name"] + " v" + package["version"]
        for package in metadata["packages"]
    }
    members = sorted(metadata["workspace_members"], key=lambda member: names[member])
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    workspace = {names[member] for member in members}

    # The engine is named in one place, and a rename there that no package answers to
    # would disarm the walk in silence — every plugin would come back clean.
    # llmlint: ignore[changed_behavior_has_e2e] it keeps the loud failure `cargo tree --package` gave on a name no package answers to; the file that could reach it is pinned unchanged by this node.
    for name in sorted(({ENGINE} | PLUGINS) - workspace):
        print(f"{PREFIX} {name} is no package of this workspace, so the rule cannot be")
        print(f"{PREFIX} checked for it. Fix that name where it is written — a crate this")
        print(f"{PREFIX} guard cannot find in the graph is a crate it stops checking.")

    for member in members:
        plugin = names[member]
        if plugin not in PLUGINS:
            continue
        path = path_to_engine(member)
        if path is None:
            continue
        print(f"{PREFIX} {plugin} reaches {ENGINE} through a dependency edge.")
        print(f"{PREFIX} the path, innermost crate first:")
        for package_id in path:
            print(f"{PREFIX}   {labels[package_id]}")
        print(f"{PREFIX} break that path — the arrow only runs one way.")
# llmlint: ignore[changed_behavior_has_e2e] the shape it refuses cannot come from any manifest, so only a stand-in for cargo could drive it, and the file that would hold one, scripts/check-isolation-enforced.sh, is pinned unchanged by this node.
except (KeyError, TypeError, ValueError) as error:
    print(f"{PREFIX} could not read the dependency graph cargo handed over: {error}",
          file=sys.stderr)
    print(f"{PREFIX} compare cargo metadata --format-version 1 against the fields this",
          file=sys.stderr)
    print(f"{PREFIX} script reads, and update it to the shape cargo now emits.",
          file=sys.stderr)
    raise SystemExit(1)
'
)"

if [ -n "$graph_violations" ]; then
  printf '%s\n' "$graph_violations" >&2
  exit 1
fi
