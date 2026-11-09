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
from decimal import Decimal
from pathlib import Path

# The Rust floor on the LCOV basis: the user's 92 of 2026-10-30 over `cargo llvm-cov report`,
# plus the measured gap between the two reports, 0.8775 points, rounded up.
MIN_LINES = Decimal("92.88")
# The SDKs' own floors are measured by their own tools, so they stay at the user's 92.
SDK_MIN_LINES = 92
problems = []


def read_target(path: Path, target: str) -> dict:
    """One project file's target, as a mapping, or an empty one after naming what is wrong.

    A project file is configuration at this check's boundary: a file that is not JSON, not
    an object, or without the target as an object is reported with the edit that restores
    it, never as a traceback that says nothing about which file to open.
    """
    display = path.as_posix()
    try:
        project = json.loads(path.read_text(encoding="utf-8"))
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
# threshold once and enforces it once — in its `--report` step, over every unit's LCOV. So
# the floor is only as whole as the set of units the merge names: the `workspace` project's
# coverage target has to depend on every crate's, each crate's run has to go through
# scripts/coverage-unit.sh under nextest, and the report has to hand the floor to the merge.
# Comment lines out, so a flag a comment names is not read as one the invocation passes.
coverage_script = "\n".join(
    line
    for line in Path("scripts/rust-coverage.sh").read_text(encoding="utf-8").splitlines()
    if not line.lstrip().startswith("#")
)
match = re.search(r"^readonly MIN_LINES=(\d+(?:\.\d{1,2})?)$", coverage_script, re.MULTILINE)
if not match:
    problems.append("scripts/rust-coverage.sh: no MIN_LINES floor found")
elif Decimal(match.group(1)) < MIN_LINES:
    problems.append(
        f"scripts/rust-coverage.sh: the line-coverage floor is {match.group(1)}%, below the "
        f"{MIN_LINES}% bar this repository commits to in AGENTS.md"
    )
if not re.search(
    r'node scripts/coverage-merge\.mjs --fail-under-lines "\$MIN_LINES" --dir target/coverage-units --unit ',
    coverage_script,
):
    problems.append(
        "scripts/rust-coverage.sh: the --report step does not pass --fail-under-lines "
        '"$MIN_LINES" to scripts/coverage-merge.mjs, so the floor is declared but never enforced'
    )
# The crate run is the `run="$(...)"` one; the build-only one beside it would match it too.
if not re.search(r'run="\$\(bash scripts/coverage-unit\.sh "\$CRATE" -- ', coverage_script):
    problems.append(
        "scripts/rust-coverage.sh: a crate's run no longer goes through scripts/coverage-unit.sh, "
        "so it writes no report the merge can read"
    )
if not re.search(r'run="\$\(bash scripts/coverage-unit\.sh "\$CRATE" -- \\\n\s*nextest\b', coverage_script):
    problems.append(
        "scripts/rust-coverage.sh: a crate's run is no longer `coverage-unit.sh <crate> -- "
        "nextest`, the one shape the coverage runs share with every repository of this host"
    )

# A crate is named by its directory, which is what the coverage command and the
# aggregate's dependency list both spell; the project file's own name is held to it.
# A crate tagged `coverage:uninstrumented` runs its whole suite in `test` and contributes no
# coverage, by the user's decision of 2026-10-30: its coverage target is a no-op, or only builds
# its tests instrumented for another crate's measured run, and the aggregate leaves it out.
# Every other crate is measured and in the union.
crate_names = set()
UNINSTRUMENTED = "coverage:uninstrumented"
for project_file in sorted(Path("crates").glob("*/project.json")):
    crate = project_file.parent.name
    display = project_file.as_posix()
    coverage = read_target(project_file, "coverage")
    try:
        document = json.loads(project_file.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        document = {}  # read_target below names the file it cannot read
    tags = document.get("tags", []) if isinstance(document, dict) else []
    if UNINSTRUMENTED in tags:
        if any(str(tag).startswith("test-in-coverage:") for tag in tags):
            problems.append(f"{display}: tagged {UNINSTRUMENTED} and test-in-coverage, so its suite would run nowhere on Linux")
        build_only = f"bash scripts/rust-coverage.sh --build-only {crate}"
        # Optionally narrowed to the test targets another crate's measured run executes, each
        # one this crate really has.
        command = command_of(coverage) if coverage else None
        narrowed = (
            re.fullmatch(re.escape(build_only) + r"((?: --test [A-Za-z0-9_-]+)*)", command)
            if isinstance(command, str)
            else None
        )
        named_tests = narrowed.group(1).split()[1::2] if narrowed else []
        missing_tests = [
            name for name in named_tests if not (project_file.parent / "tests" / f"{name}.rs").is_file()
        ]
        if not coverage:
            pass  # read_target named the file it cannot read
        elif coverage.get("executor") == "nx:noop":
            pass
        elif missing_tests:
            problems.append(
                f"{display}: its build-only coverage target names the test targets {missing_tests}, "
                "which this crate does not have, so the measured run that drives them finds nothing"
            )
        elif not narrowed:
            problems.append(
                f"{display}: tagged {UNINSTRUMENTED}, but its coverage target still runs a "
                f"measurement; it is a no-op or '{build_only}' with any '--test <target>' pairs"
            )
        elif coverage.get("cache") is not False:
            problems.append(
                f"{display}: its build-only coverage target must be uncached, or a cache hit "
                "skips the build the run that drives these binaries needs"
            )
        continue
    crate_names.add(crate)
    if not coverage:
        continue
    if command_of(coverage) != f"bash scripts/rust-coverage.sh {crate}":
        problems.append(
            f"{display}: its coverage target no longer runs "
            f"'bash scripts/rust-coverage.sh {crate}', so its run is not in the union the "
            "floor is enforced over"
        )
    report = f"{{workspaceRoot}}/target/coverage-units/{crate}.lcov"
    if report not in names_in(coverage.get("outputs")):
        problems.append(
            f"{display}: its coverage target does not declare {report} as an output, so a cache "
            "hit restores no report and the merge refuses the run"
        )
    if coverage.get("cache") is not True:
        problems.append(
            f"{display}: its coverage target is not cached, so every pull request runs its "
            "instrumented tests again even when nothing it reads changed"
        )
    if "workspace:coverage-clear" in names_in(coverage.get("dependsOn")):
        problems.append(
            f"{display}: its coverage target still depends on workspace:coverage-clear, which "
            "was removed: a unit clears only its own outputs"
        )

workspace_file = Path("workspace/project.json")
workspace_display = workspace_file.as_posix()
aggregate = read_target(workspace_file, "coverage")
if aggregate:
    if "bash scripts/rust-coverage.sh --report" not in commands_of(aggregate):
        problems.append(
            f"{workspace_display}: coverage does not run 'bash scripts/rust-coverage.sh "
            "--report', the one place the Rust floor is enforced"
        )
    if aggregate.get("cache") is not False:
        problems.append(
            f"{workspace_display}: coverage is cached, but it must hold the floor over the "
            "units' reports on every run"
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
        reason = (
            f"which is tagged {UNINSTRUMENTED} and measures nothing"
            if Path("crates", unknown, "project.json").is_file()
            else "which is not a crate of this workspace"
        )
        problems.append(f"{workspace_display}: coverage depends on {unknown!r}'s coverage target, {reason}")

# The Python SDK carries its own floor in pyproject.toml.
pyproject = Path("sdks/python/pyproject.toml").read_text(encoding="utf-8")
match = re.search(r"--cov-fail-under=(\d+)", pyproject)
if not match:
    problems.append("sdks/python/pyproject.toml: pytest addopts set no --cov-fail-under")
elif int(match.group(1)) < SDK_MIN_LINES:
    problems.append(
        f"sdks/python/pyproject.toml: --cov-fail-under={match.group(1)} is below the "
        f"{SDK_MIN_LINES}% bar"
    )

# The TypeScript SDK carries its floor on the coverage command.
command = command_of(read_target(Path("sdks/typescript/project.json"), "coverage"))
match = re.search(r"--coverage-threshold=([\d.]+)", command)
if not match:
    problems.append(
        "sdks/typescript/project.json: its coverage target sets no --coverage-threshold"
    )
elif float(match.group(1)) * 100 < SDK_MIN_LINES:
    problems.append(
        f"sdks/typescript/project.json: --coverage-threshold={match.group(1)} is below the "
        f"{SDK_MIN_LINES}% bar"
    )

if problems:
    print("check-coverage-enforced: a project stopped enforcing its coverage floor.", file=sys.stderr)
    for problem in problems:
        print(f"  {problem}", file=sys.stderr)
    print(
        f"check-coverage-enforced: restore the floor at each site named above to at least "
        f"{MIN_LINES}% for Rust or {SDK_MIN_LINES}% for an SDK, then re-run 'just coverage' to "
        "measure against it. Lowering the bar "
        "is a change to what this repository commits to in AGENTS.md, not a way to go green.",
        file=sys.stderr,
    )
    sys.exit(1)
PY
