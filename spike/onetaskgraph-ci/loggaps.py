#!/usr/bin/env python3
"""Print a GitHub Actions job log's step starts and every gap of >= N seconds between lines.

Usage: python3 spike/onetaskgraph-ci/loggaps.py <job-log.txt> [min-gap-seconds]
"""
import datetime as dt
import re
import sys

path = sys.argv[1]
gap = float(sys.argv[2]) if len(sys.argv) > 2 else 25
prev = None
for line in open(path, encoding="utf-8-sig", errors="replace"):
    m = re.match(r"^(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d)(\.\d+)?Z (.*)", line.lstrip("\ufeff"))
    if not m:
        continue
    t = dt.datetime.fromisoformat(m.group(1) + "+00:00") + dt.timedelta(seconds=float(m.group(2) or 0))
    text = m.group(3).rstrip()
    if prev and (t - prev[0]).total_seconds() >= gap:
        print(f"{(t - prev[0]).total_seconds():7.0f}s after: {prev[1][:140]}")
    if text.startswith("##[group]Run") or text.startswith("nx:"):
        print(f"  {m.group(1)[11:]}  {text[:120]}")
    prev = (t, text)
