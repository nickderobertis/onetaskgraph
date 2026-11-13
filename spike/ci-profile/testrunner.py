"""A cargo target runner that records each libtest binary's wall time and every test's own time.

cargo runs `runner <test binary> <args>`. Under `cargo test` this asks libtest for its JSON
event stream with `--report-time` (RUSTC_BOOTSTRAP=1 in the binary's environment only, which
is what libtest reads to accept `-Zunstable-options`; cargo's own rustc invocations never see
it), appends every event to SPIKE_TEST_RECORD with the binary's name, and prints libtest's
terse progress and summary, plus each failing test's captured output, so the job log keeps
what it showed before. A `--list` probe, a nextest child, or a run with the record unset is
passed through untouched. The exit status is always the binary's own.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import time


def passthrough(argv: list[str]) -> int:
    return subprocess.call(argv)


def main() -> int:
    argv = sys.argv[1:]
    record = os.environ.get("SPIKE_TEST_RECORD")
    if not argv:
        return 2
    binary, args = argv[0], argv[1:]
    name = os.path.basename(binary)
    if (not record or "NEXTEST_RUN_ID" in os.environ or "NEXTEST_EXECUTION_MODE" in os.environ
            or "--list" in args or "--help" in args or any(a.startswith("--format") for a in args)):
        start = time.time()
        code = passthrough(argv)
        if record:
            with open(record + ".spawns", "a", encoding="utf-8") as out:
                out.write(json.dumps({"binary": name, "start": start, "end": time.time(), "mode": "passthrough",
                                      "code": code}) + "\n")
        return code
    args = [a for a in args if a not in ("-q", "--quiet")]
    env = dict(os.environ, RUSTC_BOOTSTRAP="1")
    command = [binary, *args, "-Zunstable-options", "--format=json", "--report-time"]
    start = time.time()
    proc = subprocess.Popen(command, stdout=subprocess.PIPE, env=env)
    counts = {"ok": 0, "failed": 0, "ignored": 0}
    failures: list[dict] = []
    assert proc.stdout is not None
    with open(record, "a", encoding="utf-8") as out:
        for raw in proc.stdout:
            line = raw.decode("utf-8", "replace").rstrip("\r\n")
            try:
                event = json.loads(line)
            except ValueError:
                print(line, flush=True)
                continue
            event["binary"] = name
            event["t"] = time.time()
            out.write(json.dumps(event) + "\n")
            if event.get("type") == "suite" and event.get("event") == "started":
                print(f"\nrunning {event.get('test_count', 0)} tests", flush=True)
            elif event.get("type") == "test" and event.get("event") in counts:
                counts[event["event"]] += 1
                sys.stdout.write({"ok": ".", "failed": "F", "ignored": "i"}[event["event"]])
                sys.stdout.flush()
                if event["event"] == "failed":
                    failures.append(event)
            elif event.get("type") == "suite" and event.get("event") in ("ok", "failed"):
                for failure in failures:
                    print(f"\n---- {failure.get('name')} stdout ----\n{failure.get('stdout', '')}")
                print(f"\ntest result: {event['event']}. {event.get('passed', 0)} passed; {event.get('failed', 0)} failed; "
                      f"{event.get('ignored', 0)} ignored; 0 measured; {event.get('filtered_out', 0)} filtered out; "
                      f"finished in {event.get('exec_time', 0):.2f}s", flush=True)
    code = proc.wait()
    with open(record + ".spawns", "a", encoding="utf-8") as out:
        out.write(json.dumps({"binary": name, "start": start, "end": time.time(), "mode": "json", "code": code}) + "\n")
    return code


if __name__ == "__main__":
    sys.exit(main())
