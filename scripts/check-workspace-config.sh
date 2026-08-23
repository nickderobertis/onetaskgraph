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

# One product version spans three ecosystems and no manifest can derive it from another,
# so they are reconciled here. The release tool writes all three through one script; this
# is what catches a hand-edited one of N before it ships as a mismatched release.
versions = {
    "Cargo.toml": re.search(
        r'^\[workspace\.package\]\nversion\s*=\s*"([^"]+)"',
        Path("Cargo.toml").read_text(),
        re.M,
    ),
    "sdks/python/pyproject.toml": re.search(
        r'^version\s*=\s*"([^"]+)"', Path("sdks/python/pyproject.toml").read_text(), re.M
    ),
    "sdks/typescript/package.json": None,
}
package_json = json.loads(Path("sdks/typescript/package.json").read_text())

declared = {}
for path, match in versions.items():
    if path.endswith("package.json"):
        declared[path] = package_json.get("version")
    elif match:
        declared[path] = match.group(1)
    else:
        problems.append(f"{path}: no product version could be read")

if len(set(declared.values())) > 1:
    listed = ", ".join(f"{path} = {value}" for path, value in sorted(declared.items()))
    problems.append(
        "the three published distributions declare different versions "
        f"({listed}); one product version spans all three and the release tool writes "
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
