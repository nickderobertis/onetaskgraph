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
if ! plugin_names="$(bash "$ROOT/scripts/plugin-crates.sh" | tr -d '\r')" \
  || [ -z "${plugin_names//[[:space:]]/}" ]; then
  echo "check-plugin-isolation: could not read the plugin set, so nothing was checked." >&2
  echo "check-plugin-isolation: fix what scripts/plugin-crates.sh reported above — an empty" >&2
  echo "check-plugin-isolation: plugin set would pass this guard while checking no crate." >&2
  exit 1
fi
mapfile -t PLUGINS <<<"$plugin_names"

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

# `--format-version 1` is the contract cargo maintains for reading this document, but a
# contract is only a promise until something checks it. Every field this scan goes on to
# dereference is established below, and a document missing one is refused with the reason
# through a single path the caller turns into a diagnostic — a guard that could not read
# its input has not checked the rule, and must never be mistaken for one that checked and
# found nothing. Case 9 of scripts/check-isolation-enforced.sh drives these shapes through
# a shimmed cargo, because no manifest can produce them.
def refuse(problem):
    print("the document cargo handed over " + problem, file=sys.stderr)
    raise SystemExit(1)


def mapping(value, what):
    if not isinstance(value, dict):
        refuse("holds " + what + " that is not an object")
    return value


def array(value, what):
    if not isinstance(value, list):
        refuse("holds " + what + " that is not an array")
    return value


def text(value, what):
    if not isinstance(value, str):
        refuse("holds " + what + " that is not a string")
    return value


KINDS = ("dev", "build")


def kind_of(carrier, what):
    """The edge kind a dependency or a dep_kinds entry carries. Cargo writes null for a
    normal edge, so null is the one non-string this accepts, and the rest is a closed set
    — an unknown kind is an edge this guard cannot classify, which is not a thing to guess
    about when what it decides is whether a plugin reaches the engine."""
    kind = carrier.get("kind")
    if kind is None:
        return "normal"
    if text(kind, what) not in KINDS:
        refuse("holds a dependency kind that cargo does not define")
    return kind


try:
    metadata = json.loads(sys.stdin.read())
except ValueError as error:
    refuse(f"is not JSON: {error}")

mapping(metadata, "a document")
array(metadata.get("packages"), "a packages field")
array(metadata.get("workspace_members"), "a workspace_members field")

for package in metadata["packages"]:
    mapping(package, "a package")
    # Spelled out rather than looped, so the reconciliation in case 9 of
    # scripts/check-isolation-enforced.sh can read every field this establishes.
    text(package.get("id"), "a package id")
    text(package.get("name"), "a package name")
    text(package.get("version"), "a package version")
    for dependency in array(package.get("dependencies"), "a package dependencies field"):
        mapping(dependency, "a package dependency")
        text(dependency.get("name"), "a package dependency name")
        kind_of(dependency, "a package dependency kind")

names = {package["id"]: package["name"] for package in metadata["packages"]}
labels = {
    package["id"]: package["name"] + " v" + package["version"]
    for package in metadata["packages"]
}
members = set(metadata["workspace_members"])
for member in members:
    text(member, "a workspace member")
    if member not in names:
        refuse("names a workspace member that is no package of the same document")
workspace = {names[member] for member in members}

# `--no-deps` resolves nothing and so carries no resolve section: the caller runs this scan
# over that document first, and over the resolved one after. Where there is one, it is
# established here rather than at the walk below, so that a document this cannot read is
# refused as unreadable rather than by whichever rule trips on the readable part first.
resolve = metadata.get("resolve")
nodes = None
if resolve is not None:
    mapping(resolve, "a resolve section")
    resolved_ids = {
        node["id"]
        for node in array(resolve.get("nodes"), "a resolve nodes field")
        if isinstance(node, dict) and isinstance(node.get("id"), str)
    }
    for node in resolve["nodes"]:
        mapping(node, "a resolve node")
        text(node.get("id"), "a resolve node id")
        for dependency in array(node.get("deps"), "a resolve node deps field"):
            mapping(dependency, "a resolve dependency")
            if text(dependency.get("pkg"), "a resolve dependency pkg") not in names:
                refuse("resolves a dependency on no package of the same document")
            if dependency["pkg"] not in resolved_ids:
                refuse("resolves a dependency on a package with no node of its own")
            kinds = dependency.get("dep_kinds")
            if kinds is not None:
                for entry in array(kinds, "a dep_kinds field"):
                    mapping(entry, "a dep_kinds entry")
                    kind_of(entry, "a dep_kinds kind")
    nodes = {node["id"]: node for node in resolve["nodes"]}
    for member in members:
        if member not in nodes:
            refuse("resolves no node for a workspace member")

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
        kind = kind_of(dependency, "a package dependency kind")
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

if nodes is None:
    raise SystemExit(0)


def edge_kinds(dependency):
    """Every kind of edge this one dependency represents, as `--edges all` means it: a
    null kind is a normal dependency."""
    entries = dependency.get("dep_kinds")
    kinds = {kind_of(entry, "a dep_kinds kind") for entry in entries or []}
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
            if target in came_from:
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

# No manifest can produce a document the scan cannot read, so case 9 of
# scripts/check-isolation-enforced.sh replaces the boundary to reach this: a cargo earlier
# on PATH answering `metadata` from a table with one malformed shape per field.
scan() {
  local output status
  output="$(printf '%s' "$1" | PLUGINS="${PLUGINS[*]}" python3 -c "$ISOLATION_SCAN" 2>&1)" \
    && status=0 || status=$?
  if [ "$status" -ne 0 ]; then
    echo "check-plugin-isolation: could not read the document cargo handed over, so neither" >&2
    echo "check-plugin-isolation: half of the rule was checked. The scan said:" >&2
    printf '%s\n' "$output" | sed 's/^/check-plugin-isolation:   /' >&2
    echo "check-plugin-isolation: compare 'cargo metadata --format-version 1' against the keys" >&2
    echo "check-plugin-isolation: this script reads, and update the scan to the shape it found." >&2
    exit 1
  fi
  printf '%s' "$output"
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
