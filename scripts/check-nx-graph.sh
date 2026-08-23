#!/usr/bin/env bash
# Fail when a Cargo dependency has no matching Nx edge, naming the pair.
#
# Nx cannot read a Cargo manifest, so each Rust project.json mirrors its crate's Cargo
# dependencies as `implicitDependencies`. Nothing keeps the two in step on its own: add a
# crate dependency, forget the Nx edge, and affected selection silently under-runs — the
# gate becomes a claim about a graph nobody maintains, and the first anyone learns of it
# is a regression that shipped. So the two are compared here, on every `just check`.
#
# The comparison runs both ways. A missing edge under-runs the gate; an extra edge
# over-runs it, which is how "editing the engine marks no plugin affected" fails silently.
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

cargo metadata --format-version 1 --no-deps --manifest-path Cargo.toml | python3 -c '
import json
import sys
from pathlib import Path

metadata = json.load(sys.stdin)
workspace = {package["name"] for package in metadata["packages"]}

problems = []
for package in metadata["packages"]:
    name = package["name"]
    manifest = Path(package["manifest_path"]).parent
    project_file = manifest / "project.json"
    if not project_file.exists():
        problems.append(
            f"{name}: has no project.json, so Nx cannot select it at all — "
            f"add {project_file.relative_to(Path.cwd())}"
        )
        continue

    project = json.loads(project_file.read_text())
    declared = set(project.get("implicitDependencies", []))
    # Every edge counts: a dev-dependency recompiles this crate just as a normal one does.
    actual = {
        dependency["name"]
        for dependency in package["dependencies"]
        if dependency["name"] in workspace
    }

    for missing in sorted(actual - declared):
        problems.append(
            f"{name} -> {missing}: a Cargo dependency with no Nx edge. Add "
            f"\"{missing}\" to implicitDependencies in {project_file.relative_to(Path.cwd())}, "
            "or affected selection will under-run and skip this crate."
        )
    for extra in sorted(declared - actual):
        problems.append(
            f"{name} -> {extra}: an Nx edge with no Cargo dependency. Remove "
            f"\"{extra}\" from implicitDependencies in {project_file.relative_to(Path.cwd())}, "
            "or affected selection will over-run and re-test crates the change cannot reach."
        )

if problems:
    print("check-nx-graph: the Nx project graph and the Cargo graph disagree.", file=sys.stderr)
    for problem in problems:
        print(f"  {problem}", file=sys.stderr)
    sys.exit(1)
'
