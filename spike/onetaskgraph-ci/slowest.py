#!/usr/bin/env python3
"""Print the N slowest tests a nextest run reported, from its own per-test timing lines.

Usage: python3 spike/onetaskgraph-ci/slowest.py <nextest-output.log> [N]

Reads only the final status line nextest prints for each test (PASS, FAIL, SIGTERM, ...),
so a test still running when the run was stopped appears with the time it had reached.
"""
import re
import sys

LINE = re.compile(r"^\s*(PASS|FAIL|FLAKY|LEAK|TIMEOUT|SIGTERM|SIGKILL|SIGSEGV|ABORT)\s+\[\s*([\d.]+)s\]\s+(?:\([^)]*\)\s+)?(\S+)\s+(\S+)")

results = {}
for line in open(sys.argv[1], encoding="utf-8", errors="replace"):
    m = LINE.match(line)
    if m:
        status, secs, binary, name = m.groups()
        results[f"{binary} {name}"] = (float(secs), status)
n = int(sys.argv[2]) if len(sys.argv) > 2 else 20
print(f"{len(results)} tests with a final status line\n")
print("| # | Test | Seconds | Status |\n| --- | --- | --- | --- |")
for i, (test, (secs, status)) in enumerate(sorted(results.items(), key=lambda kv: -kv[1][0])[:n], 1):
    print(f"| {i} | `{test}` | {secs:g} | {status} |")
print(f"\nSum of every listed test's time: {round(sum(s for s, _ in results.values()), 1)} s")
