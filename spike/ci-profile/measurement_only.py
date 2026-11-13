#!/usr/bin/env python3
"""Apply the spike's measurement-only changes to a tree's `.github/workflows/ci.yml`.

    python3 spike/ci-profile/measurement_only.py <ci.yml> [--exclude=P,Q]
        [--step-env=[JOB:]NAME=VALUE]... [--job-exclude=JOB:P,Q]... [--marker-base] [--no-recorders]
        [--cold-rust] [--compile-only]

The same text edits apply to today's workflow and to the workflow of #3956 and its parent, so
both sides of a comparison carry byte-identical measurement changes:
- no cache write of any kind: every uv `save-cache` is false, setup-bun keeps no cache, every
  Rust `cache-save-if` is false, every `actions/cache/save` step is `if: false`, the Nx cache
  sidecar is read-only, and the Nx report is uploaded under a name no promotion reads;
- `lint` and `budgets` do not run (they measure nothing this spike reads);
- each test job may run 60 minutes, so a recorded job finishes and is measured whole;
- everything a measurement sets is exported inside the test step itself, after every cache
  restore and never through GITHUB_ENV, so no cache key (rust-cache folds CARGO_* into its
  key) and no later step sees it: the recorders (the process sampler, and the libtest
  recording runner as CARGO_TARGET_<triple>_RUNNER) unless `--no-recorders`, and each
  `--step-env` a prototype sets;
- `--exclude` passes projects to `ci-job.sh --exclude=` (the tasks only the workflow edit
  itself selected), so a demonstration runs the selection its source edit makes;
- `--job-exclude` adds projects to one job's `--exclude=` alone, so a prototype whose own diff
  widens the selection can be held to exactly the tasks its control runs in each job;
- `--marker-base` has each gate compare against a local commit without the demonstration's marker
  comment (`marker_base.py`), so a prototype and its control select the same tasks whatever else
  the prototype's commit changes;
- `--cold-rust` turns the Rust build cache off in each test job's toolchain step
  (`cache: false`: nothing is restored, and nothing could be saved), so the job builds cold;
- `--compile-only` replaces each test job's gate with one cold-comparable build,
  `cargo test --workspace --all-features --locked --no-run --timings`, and uploads cargo's
  timings report as `spike-profile-<job>`; it implies `--no-recorders`.
Exit status: 0 the file was rewritten; 2 a usage error, or an expected site of the workflow
is missing (named on standard error).
"""

from __future__ import annotations

import argparse
import re
import sys
from dataclasses import dataclass, field

TRIPLES = {
    "test-linux": "X86_64_UNKNOWN_LINUX_GNU",
    "test-macos": "AARCH64_APPLE_DARWIN",
    "test-windows": "X86_64_PC_WINDOWS_MSVC",
}
PROJECT_LIST = re.compile(r"[A-Za-z0-9_.@/-]+(,[A-Za-z0-9_.@/-]+)*")
STEP_ENV = re.compile(r"(?:(test-linux|test-macos|test-windows):)?([A-Z][A-Z0-9_]*)=([A-Za-z0-9_.:/+-]*)")
USAGE_EXIT = 2
# One cold-comparable build of every test target the jobs compile in the plain profile.
COMPILE_COMMAND = "cargo test --workspace --all-features --locked --no-run --timings"


class SiteMissing(Exception):
    pass


@dataclass(frozen=True)
class StepEnv:
    """One variable a measurement exports inside the test step, for one job or for every job."""

    job: str | None
    name: str
    value: str


@dataclass(frozen=True)
class Options:
    exclude: str = ""
    step_env: tuple[StepEnv, ...] = field(default_factory=tuple)
    job_exclude: tuple[tuple[str, str], ...] = field(default_factory=tuple)
    recorders: bool = True
    marker_base: bool = False
    cold_rust: bool = False
    compile_only: bool = False


def sub(pattern: str, replacement: str, text: str, what: str, minimum: int = 1) -> str:
    new, count = re.subn(pattern, replacement, text)
    if count < minimum:
        raise SiteMissing(f"expected at least {minimum} site(s) for {what}, found {count}")
    return new


def install_step(job: str) -> str:
    py = "python" if job == "test-windows" else "python3"
    venv_py = (
        "$RUNNER_TEMP/spikevenv/Scripts/python.exe" if job == "test-windows" else "$RUNNER_TEMP/spikevenv/bin/python"
    )
    return (
        f'      - name: "spike: install the process sampler"\n        shell: bash\n        run: |\n'
        f'          {py} -m venv "$RUNNER_TEMP/spikevenv"\n'
        f'          "{venv_py}" -m pip install --quiet psutil==7.1.3\n'
        f'          mkdir -p "$RUNNER_TEMP/spike"\n'
    )


def upload_step(job: str) -> str:
    return (
        f'      - name: "spike: upload the profile records"\n        if: always()\n'
        f"        uses: actions/upload-artifact@v7\n        with:\n          name: spike-profile-{job}\n"
        f"          path: ${{{{ runner.temp }}}}/spike/\n"
        "          if-no-files-found: warn\n          retention-days: 14\n"
    )


def test_step(job: str, options: Options) -> str:
    py = "python" if job == "test-windows" else "python3"
    venv_py = (
        "$RUNNER_TEMP/spikevenv/Scripts/python.exe" if job == "test-windows" else "$RUNNER_TEMP/spikevenv/bin/python"
    )
    excluded = ",".join([options.exclude] * bool(options.exclude) + [p for j, p in options.job_exclude if j == job])
    command = f"just ci-{job}" + (f" --exclude={excluded}" if excluded else "")
    lines = [f"export {e.name}={e.value}" for e in options.step_env if e.job in (None, job)]
    if options.marker_base:
        lines += [f'marker="$({py} spike/ci-profile/marker_base.py)" || exit 1', 'eval "$marker"']
    if options.compile_only:
        lines += [
            'mkdir -p "$RUNNER_TEMP/spike"',
            "status=0",
            f"{COMPILE_COMMAND} || status=$?",
            'cp -R target/cargo-timings "$RUNNER_TEMP/spike/" || true',
            'exit "$status"',
        ]
        return "        shell: bash\n        run: |\n" + "".join(f"          {line}\n" for line in lines)
    if options.recorders:
        lines += [
            'mkdir -p "$RUNNER_TEMP/spike"',
            'export SPIKE_TEST_RECORD="$RUNNER_TEMP/spike/tests.jsonl"',
            f'export CARGO_TARGET_{TRIPLES[job]}_RUNNER="{py} $GITHUB_WORKSPACE/spike/ci-profile/testrunner.py"',
            f'"{venv_py}" spike/ci-profile/sampler.py "$RUNNER_TEMP/spike/samples.jsonl" --tick 1.0 &',
            "sampler=$!",
        ]
    lines += ["status=0", f"{command} || status=$?"]
    if options.recorders:
        lines += ['touch "$RUNNER_TEMP/spike/samples.jsonl.stop"', 'wait "$sampler" || true']
    lines.append('exit "$status"')
    return "        shell: bash\n        run: |\n" + "".join(f"          {line}\n" for line in lines)


def apply(text: str, options: Options) -> str:
    text = sub(r"save-cache: \$\{\{[^}]*\}\}", "save-cache: false", text, "uv save-cache", 4)
    text = sub(r"no-cache: \$\{\{[^}]*\}\}", "no-cache: true", text, "setup-bun no-cache", 4)
    text = sub(r"cache-save-if: [^\n]+", "cache-save-if: false", text, "rust cache-save-if", 4)
    text = sub(r"(- uses: actions/cache/save@v\d+\n\s+)if: [^\n]+", r"\1if: false", text, "actions/cache/save", 2)
    text = sub(r"CACHE_READ_ONLY: [^\n]+", "CACHE_READ_ONLY: '1'", text, "Nx sidecar read-only", 4)
    text = sub(
        r"name: nx-cache-entries-\$\{\{ github\.job \}\}",
        "name: spike-nx-executions-${{ github.job }}",
        text,
        "Nx report artifact name",
        4,
    )
    text = sub(r"(\n  lint:\n    )if: [^\n]+", r"\1if: false", text, "lint off")
    text = sub(r"(\n  budgets:\n    needs: [^\n]*\n    )if: [^\n]+", r"\1if: false", text, "budgets off")
    for job in TRIPLES:
        block = re.search(rf"\n  {job}:\n(?:    [^\n]*\n|\n)+", text)
        if not block:
            raise SiteMissing(f"no {job} job")
        body = sub(rf"        run: just ci-{job}\n", test_step(job, options), block.group(0), f"{job} test step")
        body = sub(r"(\n    timeout-minutes: )\d+", r"\g<1>60", body, f"{job} timeout")
        if options.cold_rust:
            body = sub(
                r"(- uses: actions-rust-lang/setup-rust-toolchain@v\d+\n        with:\n)",
                r"\1          cache: false\n",
                body,
                f"{job} rust cache off",
            )
        if options.compile_only:
            body = sub(
                r"(\n      - name: Report cache entries and task executions\n)",
                "\n" + upload_step(job).rstrip("\n") + r"\1",
                body,
                f"{job} upload",
            )
        elif options.recorders:
            body = sub(
                r"(\n      - name: Test the affected projects\n)",
                "\n" + install_step(job).rstrip("\n") + r"\1",
                body,
                f"{job} recorder install",
            )
            body = sub(
                r"(\n      - name: Report cache entries and task executions\n)",
                "\n" + upload_step(job).rstrip("\n") + r"\1",
                body,
                f"{job} upload",
            )
        text = text.replace(block.group(0), body)
    return text


def parse(argv: list[str]) -> tuple[str, Options]:
    parser = argparse.ArgumentParser(
        prog="measurement_only.py", description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument("workflow")
    parser.add_argument("--exclude", default="")
    parser.add_argument("--step-env", action="append", default=[])
    parser.add_argument("--job-exclude", action="append", default=[])
    parser.add_argument("--marker-base", action="store_true")
    parser.add_argument("--no-recorders", action="store_true")
    parser.add_argument("--cold-rust", action="store_true")
    parser.add_argument("--compile-only", action="store_true")
    args = parser.parse_args(argv)
    # Both are written into a bash line of the workflow, so they are held to plain words.
    if args.exclude and not PROJECT_LIST.fullmatch(args.exclude):
        parser.error("--exclude takes comma-separated Nx project names only")
    pairs = []
    for item in args.step_env:
        match = STEP_ENV.fullmatch(item)
        if not match:
            parser.error(f"--step-env takes [JOB:]NAME=VALUE with an upper-case name and a plain value, not {item!r}")
        pairs.append(StepEnv(match.group(1), match.group(2), match.group(3)))
    job_exclude = []
    for item in args.job_exclude:
        job, _, projects = item.partition(":")
        if job not in TRIPLES or not PROJECT_LIST.fullmatch(projects):
            parser.error(f"--job-exclude takes JOB:P,Q with JOB one of {', '.join(TRIPLES)}, not {item!r}")
        job_exclude.append((job, projects))
    recorders = not (args.no_recorders or args.compile_only)
    return args.workflow, Options(
        args.exclude, tuple(pairs), tuple(job_exclude), recorders, args.marker_base, args.cold_rust, args.compile_only
    )


def main(argv: list[str]) -> int:
    path, options = parse(argv)
    with open(path, encoding="utf-8") as handle:
        text = handle.read()
    try:
        rewritten = apply(text, options)
    except SiteMissing as error:
        print(f"measurement_only: {error}; this workflow is not one the spike's edits know", file=sys.stderr)
        return USAGE_EXIT
    with open(path, "w", encoding="utf-8", newline="\n") as handle:
        handle.write(rewritten)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
