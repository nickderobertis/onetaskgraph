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
# `tr -d '\r'`: python opens stdout in text mode, so on Windows every "\n" it prints
# arrives as "\r\n". `mapfile -t` strips the newline but not the carriage return, and a
# crate name carrying a trailing CR is rejected by `cargo tree` as an invalid
# package-name character — a failure no Linux or macOS run can reproduce.
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
# One traversal over the union of the edge kinds, not one per kind. A path to the engine
# need not be the same kind of edge the whole way down — a plugin dev-depending on a crate
# that normally depends on the engine reaches it at depth two — and asking `--edges dev`
# alone cannot see that, because it stops following at the first normal edge below the dev
# one. Three separate queries therefore passed a tree that broke the rule, which is what
# scripts/check-isolation-enforced.sh caught the first time it ran; `--edges all` is the
# rule as AGENTS.md states it: any edge, at any depth.
for plugin in "${PLUGINS[@]}"; do
  # Capture rather than discard: a `cargo tree` that failed would otherwise look exactly
  # like a plugin with no edge to the engine, and this check would pass on a broken query.
  if ! tree="$(cargo tree --package "$plugin" --edges all --prefix none --no-dedupe 2>&1)"; then
    echo "check-plugin-isolation: could not read $plugin's dependency tree:" >&2
    printf '%s\n' "$tree" >&2
    echo "check-plugin-isolation: fix the workspace so 'cargo tree' resolves, then re-run." >&2
    exit 1
  fi
  # A here-string rather than a pipe into `grep -q`, and that is load-bearing here: with
  # `pipefail` set, `grep -q` exits at the first match and SIGPIPEs the `printf` still
  # writing the other 26,000 lines of this tree, so the PIPELINE reports failure on
  # exactly the runs where the violation was found. That inverted result is what let a
  # broken tree pass, and it gets quieter as the tree grows.
  if grep -qx "onetaskgraph-core v[0-9].*" <<<"$tree"; then
    echo "check-plugin-isolation: $plugin reaches onetaskgraph-core through a dependency edge." >&2
    echo "check-plugin-isolation: the path, innermost crate first:" >&2
    # Best-effort: the violation is already established by the tree above, so a failure to
    # render the path is worth reporting but must not turn this into a passing run.
    cargo tree --package "$plugin" --edges all --invert onetaskgraph-core 2>&1 \
      | sed 's/^/check-plugin-isolation:   /' >&2 \
      || echo "check-plugin-isolation:   (the path could not be rendered)" >&2
    echo "check-plugin-isolation: break that path — the arrow only runs one way." >&2
    exit 1
  fi
done
