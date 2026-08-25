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

# Reachability is a property of the resolved graph, and `cargo metadata` hands that graph
# over as one document — so ONE invocation answers the "any depth" half of the rule for
# every plugin at once.
#
# Read that graph as data, never as rendered text. Two constraints, both left by the
# `cargo tree --edges all --no-dedupe` shape this replaced: never render a tree per plugin
# — that one cost 180 MB an invocation and 52.9 minutes of a 69-minute Windows gate job —
# and never pipe anything large into a quiet `grep -q`, which exits at the first match and
# SIGPIPEs its writer, so under `pipefail` the pipeline fails on exactly the runs that
# found a match.
readonly ISOLATION_SCAN='
import json
import os
import sys
from collections import deque

PLUGINS = set(os.environ["PLUGINS"].split())
API = "onetaskgraph-plugin-api"
ENGINE = "onetaskgraph-core"
PREFIX = "check-plugin-isolation:"

# llmlint: ignore[boundary_inputs_validated] this document is not external input and is
# not validated against a schema here: `--format-version 1` IS the versioned contract
# cargo maintains for reading it this way, and its producer is the pinned toolchain this
# workspace builds with. A document these keys cannot read raises, python exits non-zero,
# and the command substitution in the caller propagates that under `set -e` — so a graph
# this cannot read fails the guard closed rather than passing it.
metadata = json.load(sys.stdin)
names = {package["id"]: package["name"] for package in metadata["packages"]}
labels = {
    package["id"]: package["name"] + " v" + package["version"]
    for package in metadata["packages"]
}
members = set(metadata["workspace_members"])
workspace = {names[member] for member in members}

# The manifests as written. This is not redundant with the walk below: it sees an edge the
# resolver may leave out of the graph — an optional dependency behind a feature nothing
# turns on is still the plugin declaring the engine, and still marks that plugin affected
# by every engine change — and it names the edge KIND, which is what the next author needs
# in order to find the line.
direct = []
for package in metadata["packages"]:
    if package["id"] not in members:
        continue
    name = package["name"]
    for dependency in package["dependencies"]:
        target = dependency["name"]
        kind = dependency.get("kind") or "normal"
        if name in PLUGINS and target == ENGINE:
            direct.append(f"{name} -> {target} ({kind}): a plugin crate may not depend on the engine")
        if name == API and target in workspace:
            direct.append(f"{name} -> {target} ({kind}): the contract crate may depend on no other crate of this workspace")

if direct:
    print(f"{PREFIX} the dependency direction the crate split establishes is broken.")
    for line in direct:
        print(line)
    print(f"{PREFIX} move the shared type into onetaskgraph-plugin-api, or copy")
    print(f"{PREFIX} the helper into the plugin — the arrow only runs one way.")
    raise SystemExit(0)

# A crate tagged layer:plugin that is no package of this workspace is a rule that cannot
# be checked at all, so it is a refusal rather than a silent pass over an empty set.
missing = sorted(PLUGINS - workspace)
if missing:
    for name in missing:
        print(f"{PREFIX} {name} is tagged layer:plugin but is no package of this workspace.")
    print(f"{PREFIX} fix the name in that project.json, or add the crate to the workspace —")
    print(f"{PREFIX} isolation cannot be checked for a crate that is not in the graph.")
    raise SystemExit(0)

# `--no-deps` resolves nothing and so carries no graph. The caller runs this scan over
# that document first, and over the resolved one after; everything below needs the graph.
resolve = metadata.get("resolve")
if resolve is None:
    raise SystemExit(0)

nodes = {node["id"]: node for node in resolve["nodes"]}


def edge_kinds(dependency):
    """Every kind of edge this one dependency represents, as `--edges all` means it: a
    null kind is a normal dependency."""
    kinds = {entry.get("kind") or "normal" for entry in dependency.get("dep_kinds") or []}
    return ",".join(sorted(kinds)) or "normal"


def path_to_engine(start):
    """The shortest path from `start` to the engine, innermost crate first, or None.

    The walk follows the UNION of the edge kinds, because a path to the engine need not be
    the same kind of edge the whole way down: a plugin dev-depending on a crate that
    normally depends on the engine reaches it at depth two, and following one kind at a
    time stops at the first edge of another kind. Three separate `cargo tree --edges
    <kind>` queries therefore passed a tree that broke the rule, which is what
    scripts/check-isolation-enforced.sh caught the first time it ran.

    Breadth-first, so the reported path is the shortest one there is — a long way round
    through a diamond says less about what to go and break.
    """
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


for member in sorted(members, key=lambda member: names[member]):
    if names[member] not in PLUGINS:
        continue
    path = path_to_engine(member)
    if path is None:
        continue
    print(f"{PREFIX} {names[member]} reaches {ENGINE} through a dependency edge.")
    print(f"{PREFIX} the path, innermost crate first — each line is depended on by the one")
    print(f"{PREFIX} below it, by the kind of edge that line names:")
    for package_id, kind in path:
        suffix = f" ({kind})" if kind else ""
        print(f"{PREFIX}   {labels[package_id]}{suffix}")
    print(f"{PREFIX} break that path — the arrow only runs one way.")
'

scan() {
  printf '%s' "$1" | PLUGINS="${PLUGINS[*]}" python3 -c "$ISOLATION_SCAN"
}

# The manifests first, and they are read WITHOUT resolving anything, which is what makes
# this order load-bearing rather than incidental: the ordinary violation — a plugin naming
# the engine as a normal dependency — is a Cargo cycle, because the engine depends on
# every plugin. Cargo refuses to resolve a cycle, so the graph phase below cannot run on
# the very tree this guard exists to refuse.
if ! manifests="$(cargo metadata --format-version 1 --no-deps --manifest-path Cargo.toml 2>&1)"; then
  echo "check-plugin-isolation: could not read the workspace manifests, so neither half of" >&2
  echo "check-plugin-isolation: the rule could be checked. Cargo said:" >&2
  printf '%s\n' "$manifests" >&2
  echo "check-plugin-isolation: fix the Cargo.toml that error names, then re-run." >&2
  exit 1
fi
report="$(scan "$manifests")"

if [ -z "$report" ]; then
  # Capture rather than discard: a `cargo metadata` that failed would otherwise look
  # exactly like a workspace with no forbidden edge, and this check would pass on a
  # broken query.
  if ! resolved="$(cargo metadata --format-version 1 --manifest-path Cargo.toml 2>&1)"; then
    echo "check-plugin-isolation: the manifests declare no forbidden edge, but the workspace" >&2
    echo "check-plugin-isolation: dependency graph does not resolve, so the rule could not be" >&2
    echo "check-plugin-isolation: checked at depth. Cargo said:" >&2
    printf '%s\n' "$resolved" >&2
    echo "check-plugin-isolation: fix the workspace so 'cargo metadata' resolves, then re-run —" >&2
    echo "check-plugin-isolation: a cycle back into a plugin is itself the arrow running both ways." >&2
    exit 1
  fi
  report="$(scan "$resolved")"
fi

if [ -n "$report" ]; then
  printf '%s\n' "$report" >&2
  exit 1
fi
