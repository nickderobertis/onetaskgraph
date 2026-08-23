#!/usr/bin/env bash
# Assert that every project still enforces a coverage floor.
#
# Coverage is the one gate that can be switched off without anything going red: drop the
# threshold flag and the target still passes, faster than before. This makes that edit
# fail, so a future change has to argue for lowering the bar rather than quietly deleting
# it. It checks the wiring, not the number — the numbers are measured by the targets.
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

python3 - <<'PY'
import json
import re
import sys
from pathlib import Path

MIN_LINES = 95
problems = []

# Every Rust crate routes coverage through one script, which carries the threshold once.
coverage_script = Path("scripts/rust-coverage.sh").read_text()
match = re.search(r"^readonly MIN_LINES=(\d+)$", coverage_script, re.MULTILINE)
if not match:
    problems.append("scripts/rust-coverage.sh: no MIN_LINES floor found")
elif int(match.group(1)) < MIN_LINES:
    problems.append(
        f"scripts/rust-coverage.sh: the line-coverage floor is {match.group(1)}%, below the "
        f"{MIN_LINES}% bar this repository commits to in AGENTS.md"
    )

for project_file in sorted(Path("crates").glob("*/project.json")):
    project = json.loads(project_file.read_text())
    command = project["targets"]["coverage"]["options"].get("command", "")
    if "scripts/rust-coverage.sh" not in command:
        problems.append(
            f"{project_file}: its coverage target no longer routes through "
            "scripts/rust-coverage.sh, so it no longer enforces the shared floor"
        )

# The Python SDK carries its own floor in pyproject.toml.
pyproject = Path("sdks/python/pyproject.toml").read_text()
match = re.search(r"--cov-fail-under=(\d+)", pyproject)
if not match:
    problems.append("sdks/python/pyproject.toml: pytest addopts set no --cov-fail-under")
elif int(match.group(1)) < MIN_LINES:
    problems.append(
        f"sdks/python/pyproject.toml: --cov-fail-under={match.group(1)} is below the "
        f"{MIN_LINES}% bar"
    )

# The TypeScript SDK carries its floor on the coverage command.
ts = json.loads(Path("sdks/typescript/project.json").read_text())
command = ts["targets"]["coverage"]["options"].get("command", "")
match = re.search(r"--coverage-threshold=([\d.]+)", command)
if not match:
    problems.append(
        "sdks/typescript/project.json: its coverage target sets no --coverage-threshold"
    )
elif float(match.group(1)) * 100 < MIN_LINES:
    problems.append(
        f"sdks/typescript/project.json: --coverage-threshold={match.group(1)} is below the "
        f"{MIN_LINES}% bar"
    )

if problems:
    print("check-coverage-enforced: a project stopped enforcing its coverage floor.", file=sys.stderr)
    for problem in problems:
        print(f"  {problem}", file=sys.stderr)
    print(
        f"check-coverage-enforced: restore the floor at each site named above to at least "
        f"{MIN_LINES}%, then re-run 'just coverage' to measure against it. Lowering the bar "
        "is a change to what this repository commits to in AGENTS.md, not a way to go green.",
        file=sys.stderr,
    )
    sys.exit(1)
PY
