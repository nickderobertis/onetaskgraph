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
#
# A test-only e2e crate (tagged `layer:e2e`) is reconciled by a rule of its own, because what
# it exercises is not all an edge. Its edge is the plugin it proves; the binary it spawns, the
# engine and the shared harness are named instead as `{workspaceRoot}/crates/<crate>/**/*`
# entries of its own `default` named input, which Nx treats as touching the project when they
# change — an edge through them would make every plugin's change select it. So for such a
# crate: every Cargo dependency is an edge or such an input, the binary is always such an
# input (a package with only binaries cannot be a Cargo dependency), and an edge names one of
# its Cargo dependencies or a crate the binary links — never anything else.
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# Captured rather than piped straight in: under `pipefail` a cargo failure would surface
# as a bare non-zero exit from a pipeline, with cargo's own reason swallowed.
if ! metadata="$(cargo metadata --format-version 1 --no-deps --manifest-path Cargo.toml 2>&1)"; then
  echo "check-nx-graph: could not read the Cargo workspace graph:" >&2
  printf '%s\n' "$metadata" >&2
  echo "check-nx-graph: fix the manifest error above, then re-run 'just check'." >&2
  exit 1
fi

if ! printf '%s' "$metadata" | python3 -c '
import json
import sys
from pathlib import Path

metadata = json.load(sys.stdin)
workspace = {package["name"] for package in metadata["packages"]}
BINARY = "onetaskgraph"
E2E_TAG = "layer:e2e"
directory_of = {
    package["name"]: Path(package["manifest_path"]).parent.name for package in metadata["packages"]
}
linked_by_binary = {
    dependency["name"]
    for package in metadata["packages"]
    if package["name"] == BINARY
    for dependency in package["dependencies"]
    if dependency["name"] in workspace and dependency.get("kind") in (None, "normal")
}


def exercised_inputs(project):
    """The crates a project names as `{workspaceRoot}/crates/<crate>/**/*` default inputs."""
    named = project.get("namedInputs", {})
    entries = named.get("default", []) if isinstance(named, dict) else []
    by_directory = {directory: name for name, directory in directory_of.items()}
    found = set()
    for entry in entries if isinstance(entries, list) else []:
        if not isinstance(entry, str):
            continue
        prefix, suffix = "{workspaceRoot}/crates/", "/**/*"
        if entry.startswith(prefix) and entry.endswith(suffix):
            directory = entry[len(prefix):-len(suffix)]
            found.add(by_directory.get(directory, directory))
    return found


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

    if E2E_TAG in project.get("tags", []):
        display = project_file.relative_to(Path.cwd())
        inputs = exercised_inputs(project)
        for unknown in sorted(inputs - workspace):
            problems.append(
                f"{name} names crates/{unknown} in its default input, which is no crate of "
                f"this workspace. Correct the entry in {display}, or a change to the crate it "
                "meant will not select this suite."
            )
        if BINARY not in inputs:
            problems.append(
                f"{name} -> {BINARY}: an e2e suite that does not name the binary it spawns. "
                f"Add \"{{workspaceRoot}}/crates/{directory_of.get(BINARY, BINARY)}/**/*\" to "
                f"namedInputs.default in {display}, or a change to the binary will not select "
                "the journeys that prove it."
            )
        for missing in sorted(actual - declared - inputs):
            problems.append(
                f"{name} -> {missing}: a Cargo dependency of an e2e suite that is neither an "
                f"Nx edge nor a default input. Add \"{missing}\" to implicitDependencies in "
                f"{display} if it is what this suite proves, or "
                f"\"{{workspaceRoot}}/crates/{directory_of.get(missing, missing)}/**/*\" to its "
                "namedInputs.default if it is what the suite only runs on, or affected "
                "selection will under-run and skip this suite."
            )
        for extra in sorted(declared - actual - linked_by_binary):
            problems.append(
                f"{name} -> {extra}: an Nx edge of an e2e suite to a crate it neither depends "
                f"on nor reaches through the binary. Remove \"{extra}\" from "
                f"implicitDependencies in {display}, or affected selection will over-run and "
                "re-test journeys the change cannot reach."
            )
        continue

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
'; then
  # The reader above names the disagreeing pair itself. This only has to add context for
  # the other way it can fail: dying before it could say anything.
  echo "check-nx-graph: if the output above is a traceback rather than a named pair, a" >&2
  echo "check-nx-graph: crates/*/project.json is malformed — check the one it names." >&2
  exit 1
fi
