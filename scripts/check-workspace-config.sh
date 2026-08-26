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
           "coverage", "test-live"}

problems = []

project_files = sorted(
    list(Path("crates").glob("*/project.json"))
    + list(Path("sdks").glob("*/project.json"))
    + [Path("workspace/project.json")]
)
if not project_files:
    problems.append("no project.json files found — Nx has nothing to orchestrate")

names = {}
for path in project_files:
    try:
        project = json.loads(path.read_text())
    except json.JSONDecodeError as error:
        problems.append(f"{path}: is not valid JSON ({error})")
        continue

    name = project.get("name")
    if not name:
        problems.append(f"{path}: has no \"name\", so Nx cannot address it")
        continue
    if name in names:
        problems.append(f"{path}: reuses the project name {name!r}, already used by {names[name]}")
    names[name] = path

    declared = set(project.get("targets", {}))
    for missing in sorted(UNIFORM - declared):
        problems.append(
            f"{path}: is missing the {missing!r} target. Target names are uniform across "
            "projects because `nx affected` fans out by name — a project missing one is "
            "silently dropped from that root command."
        )

# These two targets execute the same onetaskgraph binary. They must share one completed
# build: independent Cargo build/test processes can replace the executable between
# assert_cmd resolving CARGO_BIN_EXE_onetaskgraph and spawning it (observed on macOS).
# Keep this assertion beside the project-shape checks so a future consumer cannot quietly
# reintroduce that race by embedding another `cargo build` in its own command.
binary_project = json.loads(Path("crates/onetaskgraph/project.json").read_text())
typescript_project = json.loads(Path("sdks/typescript/project.json").read_text())
binary_targets = binary_project.get("targets", {})
typescript_targets = typescript_project.get("targets", {})
if "build" not in binary_targets:
    problems.append(
        "crates/onetaskgraph/project.json: is missing the shared binary build target; "
        "restore it so executable consumers cannot race independent Cargo builds"
    )
if "build" not in binary_targets.get("test", {}).get("dependsOn", []):
    problems.append(
        "crates/onetaskgraph/project.json: test does not depend on build; add that dependency "
        "so integration tests start with the executable present"
    )
generator = typescript_targets.get("generate-check", {})
if "onetaskgraph:build" not in generator.get("dependsOn", []):
    problems.append(
        "sdks/typescript/project.json: generate-check does not depend on onetaskgraph:build; "
        "share that build rather than relinking the binary beside integration tests"
    )
generator_command = generator.get("options", {}).get("command", "")
if "cargo build" in generator_command:
    problems.append(
        "sdks/typescript/project.json: generate-check runs its own cargo build; remove it and "
        "depend on onetaskgraph:build so the generator cannot replace a binary under test"
    )

# The `workspace` project depends on every other project so the cross-cutting checks run
# whenever anything they check can change. That list is a hand-mirrored inventory of the
# projects discovered above, and nothing derives it — so add a project, forget the entry,
# and a change to it silently stops selecting the graph, coverage and live-lane checks.
# Reconciled here against the discovered set, both ways, for the same reason the Cargo and
# Nx graphs are reconciled in check-nx-graph.sh.
WORKSPACE_PROJECT = Path("workspace/project.json")
if WORKSPACE_PROJECT in project_files and "workspace" in names:
    inventory = json.loads(WORKSPACE_PROJECT.read_text()).get("implicitDependencies", [])
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
