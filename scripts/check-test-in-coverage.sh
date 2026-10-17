#!/usr/bin/env bash
# Hold every `test-in-coverage:<platforms>` tag to what it claims.
#
# scripts/ci-job.sh does not run a tagged project's `test` on those platforms, because its
# `coverage` target runs the same suite instrumented and `test-uncovered` runs the rest. That
# is only true while the three targets say so: a command added to `test` and nowhere else
# would stop running on Linux and macOS with every selection still green. So the tag is
# reconciled against the commands, rather than trusted:
#
#   unix  a Rust crate measured by scripts/rust-coverage.sh, whose every `test` command is
#         the all-features run that measurement repeats under nextest or a command
#         `test-uncovered` runs, and whose doctests, which nextest never runs, `test-uncovered`
#         runs;
#   all   an SDK whose `coverage` command is its `test` command with coverage switched on;
#
# and never a project with a live lane, whose credentials coverage clears.
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

python3 - <<'PY'
import json
import re
import shlex
import subprocess
import sys
from pathlib import Path

FENCE = re.compile(r"^\s*//[/!]\s*```\s*([\w,-]*)\s*$")
RUN = {"", "rust", "no_run", "should_panic", "compile_fail"}
problems = []


INCLUDED_DOC = re.compile(r"#!?\[doc\s*=\s*include_str!")


def has_doctest(text):
    """Whether a doc comment opens a code block rustdoc runs — one tagged rust, or untagged —
    or documentation is included from a file, whose blocks this cannot see and so counts."""
    if INCLUDED_DOC.search(text):
        return True
    opened = False
    for line in text.splitlines():
        fence = FENCE.match(line)
        if not fence:
            continue
        if not opened and all(tag in RUN or tag.startswith("edition") for tag in fence.group(1).split(",")):
            return True
        opened = not opened
    return False


def commands(project, target):
    options = project.get("targets", {}).get(target, {}).get("options", {})
    return list(options.get("commands") or ([options["command"]] if "command" in options else []))


# What a tagged crate's coverage run executes is scripts/rust-coverage.sh's own invocation, read
# rather than assumed: the whole crate, every feature, nothing filtered. A filter, a narrower
# feature set or a test selection there would leave the tags below claiming tests nobody runs.
MEASURED = ["--no-report", "nextest", "--package", '"$CRATE"', "--all-features", "--locked"]
coverage_script = Path("scripts/rust-coverage.sh").read_text(encoding="utf-8")
joined = re.sub(r"\\\n\s*", " ", coverage_script)
runs = [found.split() for found in re.findall(r"cargo llvm-cov (.*?) 2>&1", joined)]
runs = [words for words in runs if "nextest" in words]
measured = runs[0] if len(runs) == 1 else None
if measured != MEASURED:
    problems.append(
        "scripts/rust-coverage.sh: a crate's instrumented run is "
        f"{' '.join(measured) if measured else 'not found'}, not the whole crate with every "
        f"feature ({' '.join(MEASURED)}), so test-in-coverage:unix claims tests it no longer "
        "runs; restore the invocation, or take the tags off"
    )

paths = subprocess.check_output(["git", "ls-files", "*project.json"], text=True, encoding="utf-8")
for path in sorted(paths.split()):
    project = json.loads(Path(path).read_text(encoding="utf-8"))
    name = project.get("name")
    tags = project.get("tags", [])
    claimed = [tag.split(":", 1)[1] for tag in tags if tag.startswith("test-in-coverage:")]
    if not claimed:
        continue
    where = f"{path} ({name})"
    if len(claimed) > 1 or claimed[0] not in {"unix", "all"}:
        problems.append(f"{where}: carries {claimed}; the tag is one of test-in-coverage:unix or :all")
        continue
    if any(tag.startswith("live:") for tag in tags):
        problems.append(
            f"{where}: has a live lane, whose credentials scripts/rust-coverage.sh clears, so its "
            "live journeys never run under coverage; take test-in-coverage off it"
        )
    tested, covered, uncovered = (commands(project, t) for t in ("test", "coverage", "test-uncovered"))
    if claimed[0] == "unix":
        if covered != [f"bash scripts/rust-coverage.sh {name}"]:
            problems.append(
                f"{where}: test-in-coverage:unix names a Rust crate scripts/rust-coverage.sh "
                f"measures, and its coverage runs {covered}"
            )
        # The all-features run of the whole crate, which its nextest measurement repeats.
        repeated = ["cargo", "test", "--quiet", "-p", name, "--all-features", "--locked"]
        for command in tested:
            if shlex.split(command) != repeated and command not in uncovered:
                problems.append(
                    f"{where}: test runs `{command}`, which neither its coverage run repeats nor "
                    "test-uncovered runs, so it would stop running where coverage stands in for "
                    "test; add it to test-uncovered"
                )
        sources = Path(path).parent / "src"
        documented = any(has_doctest(file.read_text(encoding="utf-8")) for file in sources.rglob("*.rs"))
        if documented and not any("--doc" in shlex.split(command) for command in uncovered):
            problems.append(
                f"{where}: has doctests, which nextest never runs, and test-uncovered does not "
                "run them; add the crate's all-features test run with `--doc` to it"
            )
    else:
        if len(tested) != 1 or len(covered) != 1:
            problems.append(f"{where}: test-in-coverage:all needs one test and one coverage command")
            continue
        plain = [word for word in shlex.split(tested[0]) if word != "--no-cov"]
        measured = shlex.split(covered[0])
        extra = measured[len(plain):]
        options = lambda target: project["targets"][target].get("options", {}).get("cwd")
        if (
            measured[: len(plain)] != plain
            or not all(word.startswith(("--cov", "--coverage")) for word in extra)
            or options("test") != options("coverage")
        ):
            problems.append(
                f"{where}: test-in-coverage:all needs coverage to be test with coverage switched "
                f"on; test runs `{tested[0]}` and coverage `{covered[0]}`"
            )

if problems:
    print("check-test-in-coverage: a project's tests would stop running where coverage stands in for them.", file=sys.stderr)
    for problem in problems:
        print(f"  {problem}", file=sys.stderr)
    print("check-test-in-coverage: next: fix the targets each line names, or take the tag off; scripts/ci-job.sh says what the tag skips.", file=sys.stderr)
    raise SystemExit(1)
PY
