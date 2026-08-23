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

if problems:
    print("check-workspace-config: the workspace configuration is inconsistent.", file=sys.stderr)
    for problem in problems:
        print(f"  {problem}", file=sys.stderr)
    sys.exit(1)
PY
