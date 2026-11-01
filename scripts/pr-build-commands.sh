#!/usr/bin/env bash
# Print, one command per line, every build test-linux makes of this workspace's Rust units,
# read from where each is declared rather than restated: each crate's `test` target's cargo
# commands from its project.json, with --no-run, and what a scripts/live-crate-test.sh command
# there builds under ONETASKGRAPH_TESTS_UNDER_COVERAGE=1; the instrumented run scripts/rust-coverage.sh
# makes of each crate, with a filter selecting no test (cargo-llvm-cov refuses --no-report
# beside --no-run); the crate publication's packaging verification, which release-cd's test 23
# makes through `cargo publish` in the target directory scripts/release_cd.py names (that
# directory resolved as cargo resolves it here, and RUSTFLAGS the same -D warnings either way);
# and the distribution journey's release build.
#
#   scripts/pr-build-commands.sh
#
# scripts/check-warm-rust-cache.sh holds scripts/warm-rust-cache.sh to these shapes, and
# scripts/check-warm-rust-cache-reuse.sh runs them over what the warmer built.
set -euo pipefail
# Nothing here opens a live session, and its lines name whole live packages: the credentials
# go before anything runs, so a caller replaying them as printed opens none either.
unset GH_PROJECTS_TOKEN LINEAR_API_KEY ONETASKGRAPH_LIVE_REQUIRED

fail() {
  echo "pr-build-commands: $1" >&2
  echo "pr-build-commands: next: $2" >&2
  exit 1
}

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
python_bin="$(command -v python3)" || fail "python3 is not on PATH" "install python3 and re-run"
case $python_bin in
  /* | ?:[\\/]*) ;;
  *) python_bin=$(cd "$(dirname "$python_bin")" && pwd)/$(basename "$python_bin") ;;
esac

"$python_bin" - <<'PY' | tr -d '\r'
import json, os, pathlib, re, shlex, subprocess, sys

def refuse(path, why):
    sys.exit(f"pr-build-commands: {path}: {why}; next: give its `test` target a `cargo test` command, or teach scripts/pr-build-commands.sh the new shape")

out = []
# Named with forward slashes on every platform, as every other guard here names a path.
for found in sorted(pathlib.Path("crates").glob("*/project.json"), key=lambda found: found.as_posix()):
    path = found.as_posix()
    try:
        with open(found, encoding="utf-8") as handle:
            node = json.load(handle)
    except (OSError, ValueError) as error:
        sys.exit(f"pr-build-commands: {path}: could not be read as JSON ({error}); next: fix that file so it parses")
    tags = node.get("tags", []) if isinstance(node, dict) else []
    for key in ("targets", "test", "options"):
        node = node.get(key) if isinstance(node, dict) else None
    if not isinstance(node, dict):
        refuse(path, "has no `targets.test.options` object")
    options = node
    commands = options["commands"] if "commands" in options else [options.get("command")]
    if not isinstance(commands, list) or not commands:
        refuse(path, "its `test` target names no command")
    built = 0
    for command in commands:
        command = command.get("command") if isinstance(command, dict) else command
        if not isinstance(command, str):
            refuse(path, f"its `test` target has a command that is not a string: {command!r}")
        if command.startswith("cargo test "):
            out.append(command.replace(" --quiet", "") + " --no-run")
            built += 1
        # What scripts/live-crate-test.sh builds on Linux, where test-linux sets
        # ONETASKGRAPH_TESTS_UNDER_COVERAGE=1: a crate tagged coverage:uninstrumented whole, with
        # the package it also builds; any other live crate's credentialed journey alone, its
        # coverage running the rest.
        elif command.startswith("bash scripts/live-crate-test.sh "):
            words = shlex.split(command)[2:]
            if len(words) not in (3, 5) or (len(words) == 5 and words[3] != "--also-build"):
                refuse(path, f"its `test` target runs `{command}`, which is not scripts/live-crate-test.sh's <crate> <test-binary> <test> [--also-build <package>]")
            if "coverage:uninstrumented" not in tags:
                out.append(f"cargo test -p {words[0]} --all-features --locked --test {words[1]} --no-run")
            else:
                if len(words) == 5:
                    out.append(f"cargo test -p {words[4]} --all-features --locked --no-run")
                out.append(f"cargo test -p {words[0]} --all-features --locked --no-run")
            built += 1
        # A guard script of this repository drives the units the cargo commands built.
        elif not command.startswith("bash scripts/check-"):
            refuse(path, f"its `test` target runs `{command}`, which is neither `cargo test` nor a guard script")
    if not built:
        refuse(path, "its `test` target runs no `cargo test`")
try:
    with open("scripts/rust-coverage.sh", encoding="utf-8") as handle:
        coverage = handle.read()
except (OSError, ValueError) as error:
    sys.exit(f"pr-build-commands: scripts/rust-coverage.sh could not be read ({error}); next: restore it, which holds the instrumented run")
run = re.search(r'run="\$\((cargo llvm-cov\b.*?) 2>&1\)"', coverage, re.S)
if not run or '"$CRATE"' not in run.group(1):
    sys.exit("pr-build-commands: scripts/rust-coverage.sh has no `run=\"$(cargo llvm-cov ... \"$CRATE\" ... 2>&1)\"`; next: point this at its instrumented run")
instrumented = " ".join(run.group(1).replace("\\\n", " ").split())
uninstrumented = set()
narrowed_tests = {}
for found in pathlib.Path("crates").glob("*/project.json"):
    try:
        project = json.loads(found.read_text(encoding="utf-8"))
    except ValueError:
        continue
    coverage = project.get("targets", {}).get("coverage", {})
    if "coverage:uninstrumented" in project.get("tags", []) and coverage.get("executor") == "nx:noop":
        uninstrumented.add(found.parent.name)
    # A build-only coverage target narrowed to the test targets another crate's run executes
    # builds only those, so they are what is built here too.
    command = coverage.get("options", {}).get("command", "")
    prefix = f"bash scripts/rust-coverage.sh --build-only {found.parent.name}"
    if isinstance(command, str) and command.startswith(prefix):
        narrowed_tests[found.parent.name] = command[len(prefix):]
# The crates test-linux builds under cargo-llvm-cov: every one a test command builds but those
# whose suites run uninstrumented (coverage:uninstrumented) with a no-op coverage target. One
# whose coverage only builds its tests (`rust-coverage.sh --build-only`) is built exactly as
# below, with the filter selecting no test, and only the `--test` targets it names when any.
crates = sorted({m for c in out for m in re.findall(r"(?:-p|--package) (\S+)", c)} - uninstrumented)
if not crates:
    sys.exit("pr-build-commands: no crate's test target runs `cargo test`; next: restore the crates' test targets")
for crate in crates:
    out.append(instrumented.replace('"$CRATE"', crate) + narrowed_tests.get(crate, "") + " -E none() --no-tests=pass")
try:
    metadata = json.loads(subprocess.run(
        ["cargo", "metadata", "--no-deps", "--format-version", "1", "--locked"],
        check=True, capture_output=True, text=True, encoding="utf-8").stdout)
except (OSError, subprocess.CalledProcessError, ValueError) as error:
    sys.exit(f"pr-build-commands: cargo metadata failed ({error}); next: run 'cargo metadata --no-deps' and fix what it reports")
published = [package["name"] for package in metadata["packages"] if package.get("publish") != []]
if published:
    target = os.environ.get("CARGO_TARGET_DIR") or str(pathlib.Path.cwd() / "target")
    selection = " ".join(f"-p {crate}" for crate in published)
    out.append(f"cargo package {selection} --locked --allow-dirty --target-dir {target}/crate-publication")
out.append("bash scripts/test-distribution.sh --release-build-only")
# llmlint: ignore[tool_output_is_signal] Standard output is this script's machine-read interface: the two checks that call it read one build command per line.
print("\n".join(out))
PY
