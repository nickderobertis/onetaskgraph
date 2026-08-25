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
  printf '%s' "$metadata" | PLUGINS="${PLUGINS[*]}" python3 -c '
import json
import os
import sys

PLUGINS = set(os.environ["PLUGINS"].split())
API = "onetaskgraph-plugin-api"
ENGINE = "onetaskgraph-core"

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
# Read as data, from ONE `cargo metadata` for every plugin at once, rather than rendered
# per plugin. `cargo tree --edges all --no-dedupe` re-expands every shared subtree at every
# place it appears, and reqwest’s graph is a wide diamond: one plugin’s tree measured
# 7,613,874 lines and 179,806,436 bytes, all of it crossing a command substitution and a
# here-string, and Windows spent 52.9 minutes of a 69-minute gate job pushing it through.
# Rendering it also invited a quiet `grep -q` to SIGPIPE the writer still pushing the rest,
# which under `pipefail` inverts the pipeline’s status on exactly the runs that found a
# match. Do not reintroduce either.

# Capture rather than discard: a `cargo metadata` that failed would otherwise look exactly
# like a workspace with no edge to the engine, and this check would pass on a broken query.
if ! graph="$(cargo metadata --format-version 1 --manifest-path Cargo.toml 2>&1)"; then
  echo "check-plugin-isolation: could not read the workspace dependency graph:" >&2
  printf '%s\n' "$graph" >&2
  echo "check-plugin-isolation: fix the workspace so 'cargo metadata' resolves, then re-run." >&2
  exit 1
fi

paths="$(
  printf '%s' "$graph" | PLUGINS="${PLUGINS[*]}" python3 -c '
import json
import os
import sys
from collections import deque

PLUGINS = set(os.environ["PLUGINS"].split())
ENGINE = "onetaskgraph-core"
PREFIX = "check-plugin-isolation:"

metadata = json.load(sys.stdin)
names = {package["id"]: package["name"] for package in metadata["packages"]}
labels = {
    package["id"]: package["name"] + " v" + package["version"]
    for package in metadata["packages"]
}
members = sorted(metadata["workspace_members"], key=lambda member: names[member])
nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}

# A name cargo has never heard of is a plugin this walk would skip in silence, where the
# per-plugin query it replaces failed loudly on the same name.
for name in sorted(PLUGINS - {names[member] for member in members}):
    print(f"{PREFIX} {name} is tagged layer:plugin but is no package of this workspace.")
    print(f"{PREFIX} fix that name, or add the crate to the workspace — isolation cannot")
    print(f"{PREFIX} be checked for a crate that is not in the graph.")


def edge_kinds(dependency):
    """Every kind of edge this one dependency represents. Cargo writes null for a normal
    edge."""
    kinds = {entry.get("kind") or "normal" for entry in dependency.get("dep_kinds") or []}
    return ",".join(sorted(kinds)) or "normal"


def path_to_engine(start):
    """The shortest path from `start` to the engine, innermost crate first, or None.
    Breadth-first, so the path reported is the shortest one there is — a long way round
    through a diamond says less about what to go and break."""
    came_from = {start: None}
    queue = deque([start])
    while queue:
        current = queue.popleft()
        for dependency in nodes[current]["deps"]:
            target = dependency["pkg"]
            if target in came_from or target not in nodes:
                continue
            came_from[target] = (current, edge_kinds(dependency))
            if names[target] == ENGINE:
                path, node = [(target, None)], target
                while came_from[node] is not None:
                    parent, kind = came_from[node]
                    path.append((parent, kind))
                    node = parent
                return path
            queue.append(target)
    return None


for member in members:
    plugin = names[member]
    if plugin not in PLUGINS:
        continue
    path = path_to_engine(member)
    if path is None:
        continue
    print(f"{PREFIX} {plugin} reaches {ENGINE} through a dependency edge.")
    print(f"{PREFIX} the path, innermost crate first:")
    for package_id, kind in path:
        suffix = f" ({kind})" if kind else ""
        print(f"{PREFIX}   {labels[package_id]}{suffix}")
    print(f"{PREFIX} break that path — the arrow only runs one way.")
'
)"

if [ -n "$paths" ]; then
  printf '%s\n' "$paths" >&2
  exit 1
fi
