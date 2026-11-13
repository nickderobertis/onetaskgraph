"""A cargo target runner that records each libtest binary's wall time and every test's own time.

cargo runs `testrunner.py <test binary> <args>`. With SPIKE_TEST_RECORD set it asks libtest for
its JSON event stream with `--report-time`, appends every event to that record with the
binary's name, and prints libtest's terse progress, each failure's captured output and the
summary, so the job log reads as before. A `--list` probe, a nextest child or an unset record
is passed through untouched. The exit status is always the binary's own.

libtest accepts `-Zunstable-options` only with RUSTC_BOOTSTRAP set, and a cargo that sees that
variable treats every crate as changed and rebuilds the workspace (measured: 17 units of an
up-to-date crate). So the binary gets RUSTC_BOOTSTRAP=1 together with a `cargo` shim first on
its PATH and in $CARGO, which removes the variable before it runs the real cargo: a test that
builds — such as the budget journeys that run `cargo test` through onebudgetspec — builds in
the job's own environment. The shim is compiled once per record directory with the job's
rustc.

Usage errors exit 2 with a message on standard error.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass
from pathlib import Path

USAGE_EXIT = 2
SHIM_SOURCE = r"""
use std::process::Command;
fn main() {
    let real = std::env::var_os("SPIKE_REAL_CARGO").expect("SPIKE_REAL_CARGO names the real cargo");
    let status = Command::new(real)
        .args(std::env::args_os().skip(1))
        .env_remove("RUSTC_BOOTSTRAP")
        .status()
        .expect("the real cargo starts");
    std::process::exit(status.code().unwrap_or(1));
}
"""


@dataclass(frozen=True)
class Invocation:
    binary: Path
    args: tuple[str, ...]

    @property
    def name(self) -> str:
        return self.binary.name

    @property
    def wants_passthrough(self) -> bool:
        return ("NEXTEST_RUN_ID" in os.environ or "NEXTEST_EXECUTION_MODE" in os.environ
                or any(a in ("--list", "--help") or a.startswith("--format") for a in self.args))


def parse(argv: list[str]) -> Invocation | None:
    if not argv or not argv[0]:
        return None
    return Invocation(Path(argv[0]), tuple(argv[1:]))


def shim(directory: Path) -> Path:
    """The compiled cargo shim in `directory`, built once and put in place atomically."""
    target = directory / ("cargo.exe" if os.name == "nt" else "cargo")
    if target.exists():
        return target
    directory.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=directory) as scratch:
        source = Path(scratch) / "shim.rs"
        source.write_text(SHIM_SOURCE, encoding="utf-8")
        built = Path(scratch) / target.name
        env = {k: v for k, v in os.environ.items() if k != "RUSTC_BOOTSTRAP"}
        subprocess.run(["rustc", "-O", "--edition", "2021", "-o", str(built), str(source)], check=True, env=env)
        try:
            os.replace(built, target)
        except OSError:
            if not target.exists():  # another runner's shim in use is as good as this one
                raise
    return target


def real_cargo() -> str:
    found = os.environ.get("CARGO") or shutil.which("cargo")
    if not found:
        raise SystemExit("testrunner: neither $CARGO nor a cargo on PATH names the real cargo")
    return found


def instrumented_env(record: Path) -> dict[str, str]:
    cargo = shim(record.parent / "cargo-shim")
    env = dict(os.environ)
    env["SPIKE_REAL_CARGO"] = env.get("SPIKE_REAL_CARGO") or real_cargo()
    env["RUSTC_BOOTSTRAP"] = "1"
    env["CARGO"] = str(cargo)
    env["PATH"] = str(cargo.parent) + os.pathsep + env.get("PATH", "")
    return env


def append(path: Path, entry: dict) -> None:
    with path.open("a", encoding="utf-8") as out:
        out.write(json.dumps(entry) + "\n")


def relay(event: dict, failures: list[dict]) -> None:
    """Print what libtest's terse format would have printed for one JSON event."""
    match event:
        case {"type": "suite", "event": "started", "test_count": int(count)}:
            print(f"\nrunning {count} tests", flush=True)
        case {"type": "test", "event": "ok" | "ignored" | "failed" as outcome}:
            sys.stdout.write({"ok": ".", "ignored": "i", "failed": "F"}[outcome])
            sys.stdout.flush()
            if outcome == "failed":
                failures.append(event)
        case {"type": "suite", "event": "ok" | "failed" as outcome}:
            for failure in failures:
                print(f"\n---- {failure.get('name')} stdout ----\n{failure.get('stdout', '')}")
            print(f"\ntest result: {outcome}. {event.get('passed', 0)} passed; {event.get('failed', 0)} failed; "
                  f"{event.get('ignored', 0)} ignored; 0 measured; {event.get('filtered_out', 0)} filtered out; "
                  f"finished in {float(event.get('exec_time', 0)):.2f}s", flush=True)


def run_recorded(invocation: Invocation, record: Path) -> int:
    args = [a for a in invocation.args if a not in ("-q", "--quiet")]
    command = [str(invocation.binary), *args, "-Zunstable-options", "--format=json", "--report-time"]
    start = time.time()
    proc = subprocess.Popen(command, stdout=subprocess.PIPE, env=instrumented_env(record))
    failures: list[dict] = []
    assert proc.stdout is not None
    for raw in proc.stdout:
        line = raw.decode("utf-8", "replace").rstrip("\r\n")
        try:
            event = json.loads(line)
        except ValueError:
            event = None
        if not isinstance(event, dict):
            print(line, flush=True)
            continue
        event["binary"] = invocation.name
        event["t"] = time.time()
        append(record, event)
        relay(event, failures)
    code = proc.wait()
    append(record.with_name(record.name + ".spawns"),
           {"binary": invocation.name, "start": start, "end": time.time(), "mode": "json", "code": code})
    return code


def main(argv: list[str]) -> int:
    invocation = parse(argv)
    if invocation is None:
        print("testrunner: cargo runs this as `testrunner.py <test binary> [args]`", file=sys.stderr)
        return USAGE_EXIT
    record = os.environ.get("SPIKE_TEST_RECORD")
    if not record or invocation.wants_passthrough:
        start = time.time()
        code = subprocess.call([str(invocation.binary), *invocation.args])
        if record:
            append(Path(record + ".spawns"), {"binary": invocation.name, "start": start, "end": time.time(),
                                               "mode": "passthrough", "code": code})
        return code
    return run_recorded(invocation, Path(record))


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
