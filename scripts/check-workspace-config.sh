#!/usr/bin/env bash
# Type-check the workspace's own configuration.
#
# Nx fans targets out by NAME: `nx affected -t check` reaches a project only if that
# project spells the target the same way every other one does. A typo there does not
# fail — it silently drops that project out of the gate. So the uniform target set is
# asserted here rather than trusted, alongside every workflow and project file parsing.
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

python3 - <<'PY'
import json
import re
import sys
from pathlib import Path

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path("scripts").resolve()))
from product_versions import read_reconciled_versions, unregistered_product_version_files

# The uniform set. Every project declares all of these, spelled identically, or one root
# command silently stops covering it.
UNIFORM = {"bootstrap", "check", "format", "format-check", "lint", "typecheck", "test",
           "coverage"}

problems = []

project_files = sorted(
    list(Path("crates").glob("*/project.json"))
    + list(Path("sdks").glob("*/project.json"))
    + [Path("workspace/project.json"), Path("scripts/project.json")]
)
if not project_files:
    problems.append("no project.json files found — Nx has nothing to orchestrate")

names = {}
projects_by_path = {}
targets_by_path = {}
for path in project_files:
    # Diagnostics are asserted by the cross-platform distribution journey. Keep one
    # repository-relative spelling instead of leaking pathlib's host separator on Windows.
    display_path = path.as_posix()
    try:
        project = json.loads(path.read_text())
    except json.JSONDecodeError as error:
        problems.append(f"{display_path}: is not valid JSON ({error})")
        continue
    if not isinstance(project, dict):
        problems.append(f"{display_path}: must contain a JSON object")
        continue
    projects_by_path[path] = project

    targets = project.get("targets", {})
    if not isinstance(targets, dict):
        problems.append(f'{display_path}: "targets" must contain a JSON object')
        targets = {}
    for target_name, target in targets.items():
        if not isinstance(target, dict):
            problems.append(
                f'{display_path}: target {target_name!r} must contain a JSON object'
            )
    targets_by_path[path] = {
        name: target for name, target in targets.items() if isinstance(target, dict)
    }

    name = project.get("name")
    if not isinstance(name, str) or not name:
        problems.append(f'{display_path}: "name" must be a non-empty string, so Nx can address it')
        continue
    if name in names:
        problems.append(
            f"{display_path}: reuses the project name {name!r}, already used by "
            f"{names[name].as_posix()}"
        )
    names[name] = path

    declared = set(targets)
    for missing in sorted(UNIFORM - declared):
        problems.append(
            f"{display_path}: is missing the {missing!r} target. Target names are uniform across "
            "projects because `nx affected` fans out by name — a project missing one is "
            "silently dropped from that root command."
        )

# target/debug/onetaskgraph is spawned by the Rust integration tests, both SDKs and the
# distribution journey, out of the one target directory .cargo/config.toml declares, and
# cargo replaces it whenever an invocation links a different unit of the package — which
# `cargo build` and `cargo test` of it are, dev-dependencies widening the features the
# dependencies are built with. A build from a concurrent target therefore lands between a
# test resolving CARGO_BIN_EXE_onetaskgraph and spawning it (observed on macOS). Held here:
#   1. onetaskgraph:build alone produces the file, as the test command with --no-run;
#   2. every target that spawns it depends on that build, directly or through a chain;
#   3. no other target invokes cargo on the package or names a target directory of its
#      own, and no source a spawner runs invokes cargo at all.
BINARY_PROJECT = "onetaskgraph"
BINARY_BUILD = (BINARY_PROJECT, "build")
BINARY_FILE = "target/debug/onetaskgraph"
# A reference to the debug directory, where onetaskgraph:build puts the binary: `target/debug`
# as a shell or TypeScript string, and `"target" / "debug"` as pathlib spells it, across the
# line break a formatter may put between the two. Under sdks/ and scripts/ that directory
# holds nothing else a source would reach for, so a reference to it is a spawner until it is
# registered.
DEBUG_DIRECTORY_PATTERN = re.compile(r"target\W{1,12}debug")
# A cargo invocation that links: as a shell command (`cargo build`, `cargo +stable test`)
# and as an argument list (`["cargo", "run", ...]`), which is how a test or a generator
# spells it.
CARGO_LINK_PATTERN = re.compile(
    r"\bcargo\b[\"',\s]+(?:\+\S+[\"',\s]+)?[\"']?(build|run|test|bench|rustc)\b"
)
# Every source under sdks/ and scripts/ that resolves the file, and the Nx targets that run
# it. Reconciled below against a scan of those trees for the path, both ways: a new spawner
# cannot land without naming its targets here, and an entry that no longer resolves the
# file cannot stand.
SDK_PYTHON_TESTS = [("sdk-python", "test"), ("sdk-python", "coverage")]
SDK_TYPESCRIPT_TESTS = [("sdk-typescript", "test"), ("sdk-typescript", "coverage")]
SPAWNERS = {
    "sdks/python/tests/conftest.py": SDK_PYTHON_TESTS,
    "sdks/python/generate.py": [("sdk-python", "generate-check")],
    "sdks/typescript/scripts/generate.ts": [("sdk-typescript", "generate-check")],
    "sdks/typescript/tests/client.test.ts": SDK_TYPESCRIPT_TESTS,
    "sdks/typescript/tests/generator.test.ts": SDK_TYPESCRIPT_TESTS,
    "sdks/typescript/scripts/test-packed.sh": [("sdk-typescript", "pack")],
    "scripts/test-distribution.sh": [("scripts", "distribution-test")],
}
SPAWNER_SUFFIXES = {".py", ".ts", ".sh", ".js", ".mjs"}
SPAWNER_SKIPPED_PARTS = {"node_modules", ".venv", "dist", "_generated"}
# This guard and the journey that watches it refuse both name the path in diagnostics.
GUARD_SOURCES = {
    Path("scripts/check-workspace-config.sh"),
    Path("scripts/check-workspace-config-enforced.sh"),
}

path_by_name = {name: path for name, path in names.items()}


def target_of(project: str, target: str) -> dict:
    path = path_by_name.get(project)
    return targets_by_path.get(path, {}).get(target, {}) if path else {}


def commands_of(target: dict) -> list[str]:
    options = target.get("options", {})
    if not isinstance(options, dict):
        return []
    found = []
    single = options.get("command")
    if isinstance(single, str):
        found.append(single)
    for entry in options.get("commands", []) if isinstance(options.get("commands"), list) else []:
        if isinstance(entry, str):
            found.append(entry)
        elif isinstance(entry, dict) and isinstance(entry.get("command"), str):
            found.append(entry["command"])
    return found


def dependencies_of(project: str, target: str) -> list[tuple[str, str]]:
    """The (project, target) pairs one target's dependsOn names outright.

    `^target` and `{"dependencies": true}` reach the project graph's dependencies, which
    an edge from a spawner to the binary's build never is; both are left unresolved here,
    so a spawner that reaches the build only that way is reported as not reaching it.
    """
    found = []
    for entry in target_of(project, target).get("dependsOn", []) or []:
        if isinstance(entry, str):
            if entry.startswith("^"):
                continue
            found.append(tuple(entry.split(":", 1)) if ":" in entry else (project, entry))
        elif isinstance(entry, dict) and isinstance(entry.get("target"), str):
            if entry.get("dependencies") is True:
                continue
            projects = entry.get("projects")
            if isinstance(projects, str):
                projects = [projects]
            for name in projects if isinstance(projects, list) else [project]:
                if isinstance(name, str):
                    found.append((name, entry["target"]))
    return found


def reaches_build(project: str, target: str) -> bool:
    seen = set()
    frontier = [(project, target)]
    while frontier:
        pair = frontier.pop()
        if pair in seen:
            continue
        seen.add(pair)
        if pair == BINARY_BUILD:
            return True
        frontier.extend(dependencies_of(*pair))
    return False


def cargo_selection_reaches_binary(command: str) -> bool:
    """Whether a cargo build/run/test/bench command links the onetaskgraph package."""
    tokens = command.split()
    selected = []
    for index, token in enumerate(tokens):
        if token in {"-p", "--package"} and index + 1 < len(tokens):
            selected.append(tokens[index + 1])
        elif token.startswith("--package=") or token.startswith("-p="):
            selected.append(token.split("=", 1)[1])
        elif token in {"--workspace", "--all"}:
            return True
    return not selected or BINARY_PROJECT in selected


binary_targets = targets_by_path.get(path_by_name.get(BINARY_PROJECT), {})
binary_project_display = "crates/onetaskgraph/project.json"
build_target = binary_targets.get("build")
test_target = binary_targets.get("test", {})
if not isinstance(build_target, dict):
    problems.append(
        f"{binary_project_display}: is missing the build target that produces {BINARY_FILE}; "
        "restore it as the test target's command with --no-run, so every target that spawns "
        "the binary can depend on one completed build"
    )
    build_target = {}
if build_target.get("cache") is True:
    problems.append(
        f"{binary_project_display}: build is cached; a replayed build restores no binary, so "
        "every spawner would start against a file nothing produced"
    )
test_dependencies = test_target.get("dependsOn", [])
if not isinstance(test_dependencies, list) or not all(isinstance(name, str) for name in test_dependencies):
    problems.append(f'{binary_project_display}: test "dependsOn" must contain a JSON list of target names')
    test_dependencies = []
if "build" not in test_dependencies:
    problems.append(
        f"{binary_project_display}: test does not depend on build; add that dependency so the "
        "integration tests start with the executable present and find it fresh"
    )
build_commands = commands_of(build_target)
test_commands = commands_of(test_target)
if len(build_commands) != 1 or len(test_commands) != 1:
    problems.append(
        f"{binary_project_display}: build and test must each run exactly one command, the "
        "test command and that command with --no-run"
    )
else:
    build_tokens = build_commands[0].split()
    test_tokens = test_commands[0].split()
    if "--no-run" not in build_tokens or sorted(
        token for token in build_tokens if token != "--no-run"
    ) != sorted(test_tokens):
        problems.append(
            f"{binary_project_display}: test and build resolve different units of the package "
            f"(build: {build_commands[0]!r}; test: {test_commands[0]!r}). Make build the test "
            "command plus --no-run, or the tests' own build step relinks "
            f"{BINARY_FILE} while another target that depends on build is spawning it"
        )

scanned_spawners = set()
for tree in (Path("sdks"), Path("scripts")):
    for candidate in sorted(tree.rglob("*")):
        if not candidate.is_file() or candidate.suffix not in SPAWNER_SUFFIXES:
            continue
        if SPAWNER_SKIPPED_PARTS & set(candidate.parts) or candidate in GUARD_SOURCES:
            continue
        if DEBUG_DIRECTORY_PATTERN.search(candidate.read_text(encoding="utf-8", errors="replace")):
            scanned_spawners.add(candidate.as_posix())
for unregistered in sorted(scanned_spawners - set(SPAWNERS)):
    problems.append(
        f"{unregistered}: reaches into target/debug, where {BINARY_FILE} is, but is not "
        "registered in SPAWNERS in scripts/check-workspace-config.sh; name the Nx targets "
        "that run it there and make each depend on onetaskgraph:build, or take the reference "
        "out"
    )
for stale in sorted(set(SPAWNERS) - scanned_spawners):
    problems.append(
        f"{stale}: is registered in SPAWNERS in scripts/check-workspace-config.sh but no "
        "longer reaches into target/debug; remove the entry, or restore the reference"
    )
for spawner, spawner_targets in sorted(SPAWNERS.items()):
    if spawner not in scanned_spawners:
        continue
    source = Path(spawner).read_text(encoding="utf-8", errors="replace")
    for line in source.splitlines():
        stripped = line.strip()
        if stripped.startswith("#") or stripped.startswith("//") or stripped.startswith("*"):
            continue
        if CARGO_LINK_PATTERN.search(line):
            problems.append(
                f"{spawner}: runs cargo itself ({stripped}); a spawner of {BINARY_FILE} depends "
                "on onetaskgraph:build through its Nx target and never builds, because its "
                "unit differs from the one the Rust integration tests link and cargo replaces "
                "the file on every switch"
            )
    for project, target in spawner_targets:
        display = path_by_name[project].as_posix() if project in path_by_name else project
        if project not in path_by_name or target not in targets_by_path.get(path_by_name[project], {}):
            problems.append(
                f"{display}: has no target {target!r}, which SPAWNERS in "
                f"scripts/check-workspace-config.sh names as running {spawner}; restore the "
                "target or correct the entry"
            )
        elif not reaches_build(project, target):
            problems.append(
                f"{display}: {target} spawns {BINARY_FILE} (through {spawner}) but does not "
                "depend on onetaskgraph:build; add that dependency, directly or through a "
                "target that carries it, so the binary is built once before it starts and is "
                "not being written while it runs"
            )
if not reaches_build(BINARY_PROJECT, "test"):
    problems.append(
        f"{binary_project_display}: test spawns {BINARY_FILE} as CARGO_BIN_EXE_onetaskgraph "
        "but does not depend on build"
    )

for path, targets in sorted(targets_by_path.items()):
    project = projects_by_path[path].get("name")
    for target_name, target in sorted(targets.items()):
        for command in commands_of(target):
            if "--target-dir" in command or "CARGO_TARGET_DIR" in command or "CARGO_LLVM_COV_TARGET_DIR" in command:
                problems.append(
                    f"{path.as_posix()}: {target_name} names a cargo target directory of its own "
                    f"({command!r}); every cargo invocation in this clone builds into the one "
                    "directory .cargo/config.toml declares, and what a private directory once "
                    "isolated is held by the dependencies this check asserts instead"
                )
            if (project, target_name) in {BINARY_BUILD, (BINARY_PROJECT, "test")}:
                continue
            if CARGO_LINK_PATTERN.search(command) and cargo_selection_reaches_binary(command):
                problems.append(
                    f"{path.as_posix()}: {target_name} invokes cargo on the {BINARY_PROJECT} "
                    f"package ({command!r}), which would relink {BINARY_FILE} while a target "
                    "that depends on onetaskgraph:build is spawning it; depend on that build "
                    "instead, and select a package by name if this command builds another"
                )

# The `workspace` project depends on every other project so the cross-cutting checks run
# whenever anything they check can change. That list is a hand-mirrored inventory of the
# projects discovered above, and nothing derives it — so add a project, forget the entry,
# and a change to it silently stops selecting the graph, coverage and live-lane checks.
# Reconciled here against the discovered set, both ways, for the same reason the Cargo and
# Nx graphs are reconciled in check-nx-graph.sh.
WORKSPACE_PROJECT = Path("workspace/project.json")
if WORKSPACE_PROJECT in project_files and "workspace" in names:
    inventory = projects_by_path[WORKSPACE_PROJECT].get("implicitDependencies", [])
    if not isinstance(inventory, list) or not all(isinstance(name, str) for name in inventory):
        problems.append(
            f'{WORKSPACE_PROJECT}: "implicitDependencies" must contain a JSON list of project names'
        )
        inventory = []
    expected = set(names) - {"workspace"}
    for absent in sorted(expected - set(inventory)):
        problems.append(
            f"{WORKSPACE_PROJECT}: does not depend on {absent!r}. Add it to "
            "implicitDependencies, or a change to that project will not select the "
            "cross-cutting checks the workspace project owns."
        )
    for unknown in sorted(set(inventory) - expected):
        problems.append(
            f"{WORKSPACE_PROJECT}: depends on {unknown!r}, which is not a project of this "
            "workspace. Remove it from implicitDependencies, or Nx cannot build the graph."
        )

# Every workflow has to parse, and every workflow token has to be least-privilege.
for workflow in sorted(Path(".github/workflows").glob("*.yml")):
    text = workflow.read_text()
    if "\npermissions:" not in text and "\n  permissions:" not in text:
        problems.append(
            f"{workflow}: declares no `permissions:` block. Default the token to read-only "
            "and widen per job only where a job needs it."
        )

# The MSRV is written down in two files and the toolchain pin in a third. None can be
# derived from the others, so they are reconciled here instead of being trusted to stay
# in step — which is exactly how the `just` floor drifted before this gate existed.
def version_tuple(raw: str) -> tuple[int, ...]:
    return tuple(int(part) for part in raw.split("."))


msrv = re.search(r'^rust-version\s*=\s*"([\d.]+)"', Path("Cargo.toml").read_text(), re.M)
clippy_msrv = re.search(r'^msrv\s*=\s*"([\d.]+)"', Path("clippy.toml").read_text(), re.M)
channel = re.search(
    r'^channel\s*=\s*"([\d.]+)"', Path("rust-toolchain.toml").read_text(), re.M
)

if not (msrv and clippy_msrv and channel):
    problems.append(
        "the Rust version pins could not all be read from Cargo.toml, clippy.toml and "
        "rust-toolchain.toml"
    )
else:
    if msrv.group(1) != clippy_msrv.group(1):
        problems.append(
            f"clippy.toml msrv is {clippy_msrv.group(1)} but Cargo.toml rust-version is "
            f"{msrv.group(1)}; clippy would allow an API the declared floor forbids"
        )
    if version_tuple(channel.group(1)) < version_tuple(msrv.group(1)):
        problems.append(
            f"rust-toolchain.toml pins {channel.group(1)}, below the {msrv.group(1)} floor "
            "Cargo.toml promises; the workspace cannot build with its own toolchain"
        )

# One product version spans every publishable manifest, internal package pin and public SDK
# version constant. The release tool and this check read the same inventory, while structural
# discovery below refuses a newly added version-bearing surface until that inventory owns it.
try:
    declared = read_reconciled_versions()
    unregistered = unregistered_product_version_files()
except (OSError, ValueError, json.JSONDecodeError) as error:
    declared = {}
    unregistered = ()
    problems.append(
        f"the product version files could not be read ({error}); restore the named "
        "manifest and rerun this check"
    )
for path, version in declared.items():
    if version is None:
        problems.append(f"{path}: no product version could be read")

for path in unregistered:
    problems.append(
        f"{path.as_posix()}: carries a product version but is absent from "
        "RECONCILED_VERSION_FILES; register it in scripts/product_versions.py so release "
        "updates cannot leave it behind"
    )

if len(set(declared.values())) > 1:
    listed = ", ".join(f"{path} = {value}" for path, value in sorted(declared.items()))
    problems.append(
        "the published distributions and their public version constants disagree "
        f"({listed}); one product version spans them all and the release tool writes "
        "them together, so a mismatch here ships as a broken release"
    )

# The `just` floor is stated in .tool-versions; nothing may carry a second copy.
for path in (Path("scripts/session-setup.sh"), Path("justfile")):
    if re.search(r'JUST_MIN\s*=\s*"[\d.]+"', path.read_text()):
        problems.append(
            f"{path}: hard-codes a `just` floor. State it once in .tool-versions and read "
            "it from there."
        )

if problems:
    print("check-workspace-config: the workspace configuration is inconsistent.", file=sys.stderr)
    for problem in problems:
        print(f"  {problem}", file=sys.stderr)
    print(
        "check-workspace-config: apply the edit each line above names to the file it names, "
        "then re-run 'just typecheck'. A target Nx cannot address by name is a project that "
        "silently drops out of the gate.",
        file=sys.stderr,
    )
    sys.exit(1)
PY
