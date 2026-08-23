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

metadata="$(cargo metadata --format-version 1 --no-deps --manifest-path Cargo.toml)"

violations="$(
  printf '%s' "$metadata" | python3 -c '
import json
import sys

PLUGINS = {
    "onetaskgraph-in-memory",
    "onetaskgraph-local-md",
    "onetaskgraph-linear",
    "onetaskgraph-github-projects",
}
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
for plugin in onetaskgraph-in-memory onetaskgraph-local-md onetaskgraph-linear onetaskgraph-github-projects; do
  for kind in normal build dev; do
    if cargo tree --package "$plugin" --edges "$kind" --prefix none --no-dedupe 2>/dev/null \
      | grep -qx "onetaskgraph-core v[0-9].*"; then
      echo "check-plugin-isolation: $plugin reaches onetaskgraph-core through a $kind edge." >&2
      echo "check-plugin-isolation: run 'cargo tree -p $plugin --edges $kind -i onetaskgraph-core'" >&2
      echo "check-plugin-isolation: to see the path, then break it." >&2
      exit 1
    fi
  done
done
