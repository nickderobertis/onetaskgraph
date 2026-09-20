#!/usr/bin/env bash
# Assert that every project still enforces a coverage floor.
#
# Coverage is the one gate that can be switched off without anything going red: drop the
# threshold flag and the target still passes, faster than before; drop one crate from the
# aggregate's dependencies and the floor is enforced over a union missing that crate. This
# makes either edit fail, so a future change has to argue for lowering the bar rather than
# quietly deleting it. It checks the wiring, not the number — the number is measured by
# the report this runs beside.
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


def read_target(path: Path, target: str) -> dict:
    """One project file's target, as a mapping, or an empty one after naming what is wrong.

    A project file is configuration at this check's boundary: a file that is not JSON, not
    an object, or without the target as an object is reported with the edit that restores
    it, never as a traceback that says nothing about which file to open.
    """
    display = path.as_posix()
    try:
        project = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError) as error:
        problems.append(f"{display}: could not be read as JSON ({error}); restore the project file")
        return {}
    targets = project.get("targets") if isinstance(project, dict) else None
    found = targets.get(target) if isinstance(targets, dict) else None
    if not isinstance(found, dict):
        problems.append(
            f"{display}: has no {target!r} target object; restore it, or the floor it carries "
            "is no longer enforced"
        )
        return {}
    return found


def command_of(target: dict) -> str:
    options = target.get("options")
    command = options.get("command") if isinstance(options, dict) else None
    return command if isinstance(command, str) else ""


def commands_of(target: dict) -> list[str]:
    options = target.get("options")
    commands = options.get("commands") if isinstance(options, dict) else None
    return [entry for entry in commands if isinstance(entry, str)] if isinstance(commands, list) else []


def names_in(value: object) -> list[str]:
    return [name for name in value if isinstance(name, str)] if isinstance(value, list) else []

# Every Rust crate routes its instrumented run through one script, which carries the
# threshold once and enforces it once — in its `--report` step, over the union of every
# crate's run. So the floor is only as whole as the set of runs the report follows: the
# `workspace` project's coverage target has to depend on every crate's, the crates' runs
# have to keep their profiles for it and go through nextest, and the report has to pass
# the floor to llvm-cov.
# Comment lines out, so a flag a comment names is not read as one the invocation passes.
coverage_script = "\n".join(
    line
    for line in Path("scripts/rust-coverage.sh").read_text().splitlines()
    if not line.lstrip().startswith("#")
)
match = re.search(r"^readonly MIN_LINES=(\d+)$", coverage_script, re.MULTILINE)
if not match:
    problems.append("scripts/rust-coverage.sh: no MIN_LINES floor found")
elif int(match.group(1)) < MIN_LINES:
    problems.append(
        f"scripts/rust-coverage.sh: the line-coverage floor is {match.group(1)}%, below the "
        f"{MIN_LINES}% bar this repository commits to in AGENTS.md"
    )
if not re.search(r'cargo llvm-cov report\b[^\n]*(\\\n[^\n]*)*--fail-under-lines "\$MIN_LINES"', coverage_script):
    problems.append(
        "scripts/rust-coverage.sh: the --report step does not pass --fail-under-lines "
        '"$MIN_LINES" to `cargo llvm-cov report`, so the floor is declared but never enforced'
    )
if not re.search(r"cargo llvm-cov\b[^\n]*(\\\n[^\n]*)*--no-report", coverage_script):
    problems.append(
        "scripts/rust-coverage.sh: a crate's run no longer passes --no-report, so it clears "
        "every sibling's profiles before it starts and the report is over one crate at best"
    )
if not re.search(r"cargo llvm-cov\b[^\n]*(\\\n[^\n]*)*--no-report[^\n]*(\\\n[^\n]*)*\bnextest\b", coverage_script):
    problems.append(
        "scripts/rust-coverage.sh: a crate's run is no longer `cargo llvm-cov --no-report "
        "nextest`, the one shape the coverage runs share with every repository of this host"
    )

# A crate is named by its directory, which is what the coverage command and the
# aggregate's dependency list both spell; the project file's own name is held to it.
crate_names = set()
for project_file in sorted(Path("crates").glob("*/project.json")):
    crate = project_file.parent.name
    crate_names.add(crate)
    display = project_file.as_posix()
    coverage = read_target(project_file, "coverage")
    if not coverage:
        continue
    if command_of(coverage) != f"bash scripts/rust-coverage.sh {crate}":
        problems.append(
            f"{display}: its coverage target no longer runs "
            f"'bash scripts/rust-coverage.sh {crate}', so its run is not in the union the "
            "floor is enforced over"
        )
    if "workspace:coverage-clear" not in names_in(coverage.get("dependsOn")):
        problems.append(
            f"{display}: its coverage target does not depend on workspace:coverage-clear; "
            "without it a stale profile from an earlier run counts lines this tree no longer "
            "runs"
        )
    if coverage.get("cache") is not False:
        problems.append(
            f"{display}: its coverage target is cached; a cache hit restores every profile in "
            "the shared directory as it was, a sibling's stale ones included, beside the fresh "
            "ones the report then over-counts"
        )

workspace_file = Path("workspace/project.json")
workspace_display = workspace_file.as_posix()
aggregate = read_target(workspace_file, "coverage")
clear = read_target(workspace_file, "coverage-clear")
if clear and command_of(clear) != "bash scripts/rust-coverage.sh --clear":
    problems.append(
        f"{workspace_display}: coverage-clear does not run 'bash scripts/rust-coverage.sh "
        "--clear', which every crate's run depends on to start from an empty profile directory"
    )
if aggregate:
    if "bash scripts/rust-coverage.sh --report" not in commands_of(aggregate):
        problems.append(
            f"{workspace_display}: coverage does not run 'bash scripts/rust-coverage.sh "
            "--report', the one place the Rust floor is enforced"
        )
    if aggregate.get("cache") is not False:
        problems.append(
            f"{workspace_display}: coverage is cached, but what it reads is the profile "
            "directory the crates' runs just wrote, which no input describes"
        )
    depended = set()
    for entry in aggregate.get("dependsOn") if isinstance(aggregate.get("dependsOn"), list) else []:
        if isinstance(entry, dict) and entry.get("target") == "coverage":
            depended.update(names_in(entry.get("projects")))
    for absent in sorted(crate_names - depended):
        problems.append(
            f"{workspace_display}: coverage does not depend on {absent!r}'s coverage target, "
            "so that crate's run is missing from the union the floor is enforced over"
        )
    for unknown in sorted(depended - crate_names):
        problems.append(
            f"{workspace_display}: coverage depends on {unknown!r}'s coverage target, which "
            "is not a crate of this workspace"
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
command = command_of(read_target(Path("sdks/typescript/project.json"), "coverage"))
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
