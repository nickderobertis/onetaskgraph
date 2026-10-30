#!/usr/bin/env bash
# Hold every `test-in-coverage:<crate|sdk>` tag to what it claims.
#
# scripts/ci-job.sh does not run a tagged project's `test` on Linux, where coverage is
# measured, because its `coverage` target runs the same suite instrumented and
# `test-uncovered` runs the rest. That is only true while the three targets say so: a command
# added to `test` and nowhere else would stop running on Linux with every selection still
# green. So the tag is reconciled against the commands, rather than trusted:
#
#   crate  a Rust crate measured by scripts/rust-coverage.sh, whose every `test` command is
#          the all-features run that measurement repeats under nextest or a command
#          `test-uncovered` runs, and whose library's doctests, which nextest never runs,
#          workspace:test-uncovered's one doctest run names;
#   sdk    an SDK whose `coverage` command is its `test` command with coverage switched on,
#          beside which `test` may run only commands `test-uncovered` runs;
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

problems = []


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
        f"feature ({' '.join(MEASURED)}), so test-in-coverage:crate claims tests it no longer "
        "runs; restore the invocation, or take the tags off"
    )

# The tagged libraries' doctests run in one invocation, workspace:test-uncovered's: every flag
# beside its -p list is the all-features doctest run itself, with nothing filtering it.
DOCTESTS = "cargo test --quiet --all-features --locked --doc"
workspace = json.loads(Path("workspace/project.json").read_text(encoding="utf-8"))
doc_words = shlex.split(
    workspace.get("targets", {}).get("test-uncovered", {}).get("options", {}).get("command", "")
)
documented = set()
if doc_words[: len(DOCTESTS.split())] == DOCTESTS.split():
    rest = doc_words[len(DOCTESTS.split()) :]
    if len(rest) % 2 == 0 and all(flag == "-p" for flag in rest[::2]):
        documented = set(rest[1::2])
    else:
        problems.append(
            f"workspace/project.json: test-uncovered runs `{' '.join(doc_words)}`; beside "
            f"`{DOCTESTS}` it may name packages with -p and nothing else"
        )
else:
    problems.append(
        f"workspace/project.json: test-uncovered must run the tagged libraries' doctests as "
        f"`{DOCTESTS} -p <crate>...`; it runs `{' '.join(doc_words) or 'nothing'}`"
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
    if len(claimed) > 1 or claimed[0] not in {"crate", "sdk"}:
        problems.append(f"{where}: carries {claimed}; the tag is one of test-in-coverage:crate or :sdk")
        continue
    if any(tag.startswith("live:") for tag in tags):
        problems.append(
            f"{where}: has a live lane, whose credentials scripts/rust-coverage.sh clears, so its "
            "live journeys never run under coverage; take test-in-coverage off it"
        )
    tested, covered, uncovered = (commands(project, t) for t in ("test", "coverage", "test-uncovered"))
    if claimed[0] == "crate":
        if covered != [f"bash scripts/rust-coverage.sh {name}"]:
            problems.append(
                f"{where}: test-in-coverage:crate names a Rust crate scripts/rust-coverage.sh "
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
        # Every library's doctests, whether or not it has any today: nextest never runs them,
        # and which documentation rustdoc will execute is rustdoc's to say, not this check's.
        if (Path(path).parent / "src" / "lib.rs").is_file() and name not in documented:
            problems.append(
                f"{where}: has a library, whose doctests nextest never runs, and "
                f"workspace:test-uncovered's `{DOCTESTS}` run does not name it with -p"
            )
    else:
        options = lambda target: project["targets"].get(target, {}).get("options", {}).get("cwd")
        if uncovered and options("test-uncovered") != options("test"):
            problems.append(f"{where}: test-uncovered runs in another directory than test")
        suite = [command for command in tested if command not in uncovered]
        if len(suite) != 1 or len(covered) != 1:
            problems.append(
                f"{where}: test-in-coverage:sdk needs one test command, beside those "
                "test-uncovered runs, and one coverage command"
            )
            continue
        plain = [word for word in shlex.split(suite[0]) if word != "--no-cov"]
        measured = shlex.split(covered[0])
        extra = measured[len(plain):]
        if (
            measured[: len(plain)] != plain
            or not all(word.startswith(("--cov", "--coverage")) for word in extra)
            or options("test") != options("coverage")
        ):
            problems.append(
                f"{where}: test-in-coverage:sdk needs coverage to be test with coverage switched "
                f"on; test runs `{suite[0]}` and coverage `{covered[0]}`"
            )

# The live crates: on Linux their `test` runs only the credentialed journey each
# names to scripts/live-crate-test.sh, and `coverage` runs the rest. So exactly the crates whose
# `test` is keyed on a live credential name one, each a test that opens a live session, every
# cargo run of their `test` goes through that script, and their coverage is the whole-crate run
# — or a non-live test would be run by neither target.
CREDENTIALS = {"GH_PROJECTS_TOKEN", "LINEAR_API_KEY"}
table = {}
credentialed = set()
for path in sorted(Path("crates").glob("*/project.json")):
    project = json.loads(path.read_text(encoding="utf-8"))
    inputs = project.get("targets", {}).get("test", {}).get("inputs", [])
    if any(isinstance(entry, dict) and entry.get("env") in CREDENTIALS for entry in inputs):
        credentialed.add(project["name"])
    for command in commands(project, "test"):
        words = shlex.split(command)
        if words[:2] == ["bash", "scripts/live-crate-test.sh"]:
            table[project["name"]] = words[2:]
for crate in sorted(credentialed - set(table)):
    problems.append(
        f"crates/{crate}: its test target is keyed on a live credential, and names no "
        "credentialed journey to scripts/live-crate-test.sh"
    )
for crate in sorted(set(table) - credentialed):
    problems.append(
        f"crates/{crate}: its test target runs scripts/live-crate-test.sh, and is keyed on no "
        "live credential"
    )
for crate, arguments in sorted(table.items()):
    root = Path("crates") / crate
    project = json.loads((root / "project.json").read_text(encoding="utf-8"))
    tested = commands(project, "test")
    if (
        len(arguments) not in (3, 5)
        or arguments[0] != crate
        or (len(arguments) == 5 and arguments[3] != "--also-build")
        or any("cargo" in shlex.split(command) for command in tested)
    ):
        problems.append(
            f"crates/{crate}: its test target runs {tested}; every cargo run of it goes through "
            f"`bash scripts/live-crate-test.sh {crate} <test-binary> <test>`, or it runs tests "
            "coverage also runs"
        )
        continue
    binary, test = arguments[1], arguments[2]
    if "coverage:uninstrumented" in project.get("tags", []):
        # Its whole suite runs in `test`, which it does only while no job sets the variable
        # that narrows scripts/live-crate-test.sh to the journey.
        if re.search(r"^\s*export ONETASKGRAPH_TESTS_UNDER_COVERAGE=1", Path("scripts/ci-job.sh").read_text(encoding="utf-8"), re.M):
            problems.append(
                f"crates/{crate}: tagged coverage:uninstrumented, but scripts/ci-job.sh narrows its "
                "test to the credentialed journey, and its coverage runs nothing"
            )
    elif commands(project, "coverage") != [f"bash scripts/rust-coverage.sh {crate}"]:
        problems.append(
            f"crates/{crate}: its coverage is not the whole-crate run, so the tests "
            "its test target leaves to it would run nowhere"
        )
    *module, name = test.split("::")
    binary_root = root / "tests" / binary
    source = (binary_root / Path(*module)).with_suffix(".rs") if module else (
        binary_root / "main.rs" if binary_root.is_dir() else binary_root.with_suffix(".rs")
    )
    text = source.read_text(encoding="utf-8") if source.is_file() else ""
    body = re.search(rf"(?ms)^(?:async )?fn {re.escape(name)}\(\).*?^}}", text)
    if not body or "Session::open" not in body.group(0):
        problems.append(
            f"crates/{crate}: its test target names its `{binary}` test `{test}`, which "
            f"{'is not in ' + source.as_posix() if not body else 'opens no live session'}; "
            "only a credentialed journey is left out of the coverage run's share"
        )

if problems:
    print("check-test-in-coverage: a project's tests would stop running where coverage stands in for them.", file=sys.stderr)
    for problem in problems:
        print(f"  {problem}", file=sys.stderr)
    print("check-test-in-coverage: next: fix the targets each line names, or take the tag off; scripts/ci-job.sh says what the tag skips.", file=sys.stderr)
    raise SystemExit(1)
PY
