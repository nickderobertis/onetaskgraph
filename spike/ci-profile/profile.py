"""Divide one CI test job's wall clock, from its step timings, its log and its Nx executions.

Every second of the job is assigned exactly once:
- steps outside the test step by kind: setup, cache restore, cache save, report;
- inside the test step, the Nx task timeline, each instant split equally among the tasks
  running then (so concurrent tasks share, never double count), and an instant with no task
  running is Nx planning between phases;
- inside each executed task, compile-and-link (task start until its first test starts) and
  test run (the rest); a replayed task's whole interval is cache restore (Nx remote read).
When the run carries the spike's sampler record, the test-run share is divided again by what
the process table shows at each tick (see `sampler.py`).
"""

from __future__ import annotations

import json
import re
from collections import defaultdict
from dataclasses import dataclass, field
from datetime import datetime, timezone

ANSI = re.compile(r"\x1b\[[0-9;]*[A-Za-z]")
LINE = re.compile(r"^(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d+)?)Z ?(.*)$")
HEADER = re.compile(r"^\[(?P<phase>[^\]]+)\] (?P<replay>🔁 )?> nx run (?P<task>[\w.@/-]+:[\w-]+)(?P<remote>\s+\[(?:remote|local) cache\])?")
TEST_START = re.compile(r"^(?:\[[^\]]+\] )?(?P<project>[\w.@/-]+): (?:running \d+ tests?|\s*Starting \d+ tests?)")
JSON_TEST = re.compile(r'^\[[^\]]+\] (?:(?P<project>[\w.@/-]+): )?\{"type":"(?:test|suite)".*?"name":"(?P<name>[^"]+)"')
NEXTEST_DONE = re.compile(r'^\[[^\]]+\] (?:[\w.@/-]+: )?\{"type":"test","event":"(?P<event>ok|failed|ignored)","name":"(?P<name>[^"]+)"(?:,"exec_time":(?P<secs>[0-9.]+))?')
LIBTEST_RESULT = re.compile(r"^(?:\[[^\]]+\] )?(?:(?P<project>[\w.@/-]+): )?(?:; )?.*?finished in (?P<secs>[0-9.]+)s")
STORED = re.compile(r"Nx cache: stored (?P<n>\d+) entries, (?P<bytes>\d+) bytes; replayed (?P<r>\d+) entries")


def ts(value: str) -> float:
    value = value.rstrip("Z")
    if "." in value:
        head, frac = value.split(".")
        value = f"{head}.{frac[:6]}"
    return datetime.fromisoformat(value).replace(tzinfo=timezone.utc).timestamp()


def parse_log(text: str) -> list[tuple[float, str]]:
    out = []
    for raw in text.splitlines():
        m = LINE.match(raw.lstrip("\ufeff"))
        if m:
            out.append((ts(m.group(1)), ANSI.sub("", m.group(2))))
    return out


@dataclass
class Task:
    name: str
    start: float
    end: float
    result: str  # executed | remote_hit | local_hit | ...
    compile_end: float | None = None
    builds: list = field(default_factory=list)  # (start, end, profile) of each cargo build the task printed
    lock_waits: list = field(default_factory=list)  # (start, end) cargo spent blocked on a file lock

    @property
    def project(self) -> str:
        return self.name.split(":")[0]

    @property
    def target(self) -> str:
        return self.name.split(":")[1]

    @property
    def instrumented(self) -> bool:
        return self.target in ("coverage", "coverage-build")


@dataclass
class JobProfile:
    name: str
    job_id: int
    wall: float
    start: float
    end: float
    buckets: dict[str, float] = field(default_factory=lambda: defaultdict(float))
    tasks: list[Task] = field(default_factory=list)
    task_share: dict[str, float] = field(default_factory=lambda: defaultdict(float))
    steps: list[tuple[str, float, float, str]] = field(default_factory=list)
    nextest: list[tuple[str, float]] = field(default_factory=list)
    stored: tuple[int, int, int] | None = None
    rust_cache: dict = field(default_factory=dict)
    critical: list[tuple[str, float, float]] = field(default_factory=list)
    test_step: tuple[float, float] | None = None
    sampler: dict | None = None


def step_kind(name: str) -> str:
    n = name.lower()
    if "start-cache-server" in n:
        # `background: true`: it runs beside every later step, so it owns no second of its own.
        return "background"
    if n.startswith("post "):
        if "setup-uv" in n or "setup-bun" in n or "rust-cache" in n or "cache/save" in n:
            return "cache-save"
        return "report"
    if "cache/restore" in n:
        return "cache-restore"
    if "cache/save" in n:
        return "cache-save"
    if n.startswith("test the affected"):
        return "test"
    if "upload-artifact" in n or n.startswith("report cache") or n == "complete job" or "budget" in n:
        return "report"
    if "cancell" in n or "live artifacts" in n or "linear leases" in n:
        return "report"
    return "setup"


def _segments(log: list[tuple[float, str]]):
    """(start, end, title) of each top-level '##[group]Run' segment of the log."""
    marks = [(t, line[len("##[group]Run "):]) for t, line in log if line.startswith("##[group]Run ")]
    for i, (t, title) in enumerate(marks):
        end = marks[i + 1][0] if i + 1 < len(marks) else log[-1][0]
        yield t, end, title


def restore_inside(log, start, end) -> float:
    """Seconds of cache restore inside a setup action (setup-uv, setup-bun, rust-cache)."""
    lines = [(t, s) for t, s in log if start <= t <= end]
    hits = [t for t, s in lines if re.search(r"restor|cache key|Cache (hit|not found)|Received \d+ of", s, re.I)]
    return (max(hits) - min(hits)) if len(hits) >= 2 else 0.0


def profile_job(job: dict, log_text: str, executions: dict | None) -> JobProfile:
    log = parse_log(log_text)
    start, end = ts(job["started_at"]), ts(job["completed_at"])
    p = JobProfile(job["name"], job["id"], end - start, start, end)
    steps = []
    for s in job["steps"]:
        if not s.get("started_at") or not s.get("completed_at") or s.get("conclusion") == "skipped":
            continue
        a, b = ts(s["started_at"]), ts(s["completed_at"])
        steps.append((s["name"], a, b, step_kind(s["name"])))
    # The step API has second resolution; the test step's own edges come from the log.
    segs = list(_segments(log))
    for title, a, b, kind in [(t, a, b, None) for a, b, t in segs]:
        pass
    for i, (name, a, b, kind) in enumerate(steps):
        if kind == "test":
            seg = [x for x in segs if x[2].startswith("just ci-test")]
            if seg:
                steps[i] = (name, seg[0][0], seg[0][1], kind)
                p.test_step = (seg[0][0], seg[0][1])
            else:
                p.test_step = (a, b)
    p.steps = steps
    # Restore portions of setup actions, read from the log's own groups.
    restore_extra = 0.0
    for a, b, title in segs:
        if title.startswith(("Swatinem/rust-cache", "astral-sh/setup-uv", "oven-sh/setup-bun")):
            seconds = (b - a) if title.startswith("Swatinem/rust-cache") else restore_inside(log, a, b)
            restore_extra += seconds
            if title.startswith("Swatinem/rust-cache"):
                text = "\n".join(s for t, s in log if a <= t <= b)
                m = re.search(r"Cache Size: ~?([0-9.]+ ?[MG]B)", text)
                hit = "restored from key" in text.lower() or "Restored from cache key" in text
                exact = re.search(r"Cache restored from key: (\S+)", text)
                p.rust_cache = {"seconds": round(b - a, 1), "size": m.group(1) if m else None,
                                "hit": bool(hit or exact), "key": exact.group(1) if exact else None,
                                "miss": "No cache found" in text}
    covered = 0.0
    for name, a, b, kind in steps:
        if kind in ("test", "background"):
            continue
        p.buckets[kind] += b - a
        covered += b - a
    p.buckets["setup"] -= restore_extra
    p.buckets["cache-restore"] += restore_extra
    # Nx tasks.
    tasks: list[Task] = []
    if executions:
        for t in executions["tasks"]:
            if t["task"].endswith(":bootstrap"):
                continue
            a, b = ts(t["started_at"]), ts(t["finished_at"])
            if p.test_step and not (p.test_step[0] - 2 <= a <= p.test_step[1] + 2):
                continue
            tasks.append(Task(t["task"], a, b, t["result"]))
    else:
        tasks = tasks_from_log(log, p.test_step)
    # compile end: first test start for that project after the task started
    starts = defaultdict(list)
    for t, line in log:
        m = TEST_START.match(line)
        if m:
            starts[m.group("project")].append(t)
            continue
        m = JSON_TEST.match(line)
        if m:
            starts[m.group("name").split("::")[0]].append(t)
    for task in tasks:
        if task.result != "executed":
            continue
        cands = [t for t in starts.get(task.project, []) if task.start <= t <= task.end]
        if cands and task.target in ("test", "coverage", "test-uncovered"):
            task.compile_end = min(cands)
        elif task.target in ("build", "coverage-build"):
            task.compile_end = task.end
    # Cargo builds a task printed itself (`Compiling` … `Finished \`<profile>\` profile`).
    per_project = defaultdict(list)
    project_lines = defaultdict(list)
    for t, line in log:
        m = re.match(r"^\[[^\]]+\] ([\w.@/-]+): +(.*)$", line)
        if m:
            project_lines[m.group(1)].append((t, m.group(2)))
    for t, line in log:
        m = re.match(r"^\[[^\]]+\] ([\w.@/-]+): +(Compiling|Finished) (?:`(\w+)` profile)?", line)
        if m:
            per_project[m.group(1)].append((t, m.group(2), m.group(3)))
    for task in tasks:
        if task.result != "executed":
            continue
        lines = [(t, x) for t, x in project_lines.get(task.project, []) if task.start <= t <= task.end]
        for i, (t, text) in enumerate(lines):
            if text.startswith("Blocking waiting for file lock"):
                later = next((u for u, y in lines[i + 1:] if not y.startswith("Blocking waiting")), None)
                if later is not None and later > t and not (task.lock_waits and task.lock_waits[-1][1] >= t):
                    task.lock_waits.append((t, later))
        opened = None
        for t, word, profile_name in per_project.get(task.project, []):
            if not (task.start <= t <= task.end):
                continue
            if word == "Compiling" and opened is None:
                opened = t
            elif word == "Finished" and opened is not None:
                task.builds.append((opened, t, profile_name or "dev"))
                opened = None
    p.tasks = tasks
    for t, line in log:
        m = NEXTEST_DONE.match(line)
        if m and m.group("secs"):
            p.nextest.append((m.group("name"), float(m.group("secs"))))
        m = STORED.search(line)
        if m:
            p.stored = (int(m.group("n")), int(m.group("bytes")), int(m.group("r")))
    if p.test_step:
        attribute_test_step(p)
        p.critical = critical_path(tasks, p.test_step)
    p.buckets["unattributed"] += p.wall - sum(v for k, v in p.buckets.items() if k != "unattributed")
    return p


def tasks_from_log(log, window) -> list[Task]:
    """Fallback without the executions artifact: a task runs from its header to its last prefixed line."""
    out: list[Task] = []
    for t, line in log:
        if window and not (window[0] <= t <= window[1]):
            continue
        m = HEADER.match(line)
        if m:
            result = "remote_hit" if m.group("remote") or m.group("replay") else "executed"
            out.append(Task(m.group("task"), t, t, result))
    # A task ends at its project's last prefixed line before that project's next header.
    prefixed: dict[str, list[float]] = {}
    for t, line in log:
        m = re.match(r"^\[[^\]]+\] ([\w.@/-]+): ", line)
        if m and (not window or window[0] <= t <= window[1]):
            prefixed.setdefault(m.group(1), []).append(t)
    for task in out:
        if task.result != "executed":
            task.end = task.start
            continue
        nxt = min((x.start for x in out if x.project == task.project and x.start > task.start), default=float("inf"))
        lines = [t for t in prefixed.get(task.project, []) if task.start <= t < nxt]
        task.end = max(lines, default=task.start)
    return out


def attribute_test_step(p: JobProfile) -> None:
    a0, b0 = p.test_step
    edges = {a0, b0}
    for t in p.tasks:
        for x in (t.start, t.end, t.compile_end, *[e for b in t.builds for e in b[:2]], *[e for w in t.lock_waits for e in w]):
            if x is not None and a0 < x < b0:
                edges.add(x)
    edges = sorted(edges)
    for lo, hi in zip(edges, edges[1:]):
        mid = (lo + hi) / 2
        active = [t for t in p.tasks if t.start <= mid < t.end]
        dt = hi - lo
        if not active:
            p.buckets["nx-planning"] += dt
            continue
        share = dt / len(active)
        for t in active:
            p.task_share[t.name] += share
            build = next((b for b in t.builds if b[0] <= mid < b[1]), None)
            if t.result != "executed":
                p.buckets["cache-restore(nx)"] += share
            elif any(w[0] <= mid < w[1] for w in t.lock_waits):
                p.buckets["wait:cargo-file-lock"] += share
            elif build is not None:
                kind = "release" if build[2] == "release" else ("instrumented" if t.instrumented else "plain")
                p.buckets[f"compile-link({kind})"] += share
            elif t.compile_end is not None and mid < t.compile_end:
                p.buckets["compile-link(instrumented)" if t.instrumented else "compile-link(plain)"] += share
            elif t.project in ("workspace", "scripts") or t.target not in ("test", "coverage", "test-uncovered", "budgets"):
                p.buckets["scripts-and-other-tasks"] += share
            elif t.target == "budgets":
                p.buckets["budget-checks"] += share
            else:
                p.buckets["test-run(instrumented)" if t.instrumented else "test-run(plain)"] += share


def critical_path(tasks: list[Task], window) -> list[tuple[str, float, float]]:
    """Walk back from the step's end: each link is the task finishing latest before the pointer."""
    pointer = window[1]
    chain = []
    pool = [t for t in tasks if t.end - t.start > 0.5]
    while True:
        before = [t for t in pool if t.end <= pointer + 1 and t.start < pointer - 0.5]
        if not before:
            break
        t = max(before, key=lambda x: (x.end, -x.start))
        chain.append((t.name, t.start, t.end))
        pointer = t.start
        pool = [x for x in pool if x is not t]
    return list(reversed(chain))


def load_sampler(path_or_lines, window) -> dict:
    """Divide wall time inside `window` by sampler leaves; a tick where work processes spent
    under 10% of one CPU is a wait (sleep, poll, network, timeout)."""
    lines = path_or_lines if isinstance(path_or_lines, list) else open(path_or_lines, encoding="utf-8").read().splitlines()
    buckets: dict[str, float] = defaultdict(float)
    exits: dict[str, list[float]] = defaultdict(list)
    for raw in lines:
        try:
            e = json.loads(raw)
        except ValueError:
            continue
        if e["kind"] == "exit":
            if window[0] <= e["end"] <= window[1]:
                exits[e["category"]].append(e["end"] - e["start"])
            continue
        if e["kind"] != "tick" or not (window[0] <= e["t"] <= window[1]):
            continue
        dt = min(e["dt"], 5.0)
        work_cpu = sum(v for k, v in e["cpu"].items() if k not in ("nx", "nx-cache-sidecar", "other"))
        leaves = {k: v for k, v in e["leaves"].items() if k not in ("nx", "other", "shell", "just")}
        if work_cpu < 0.1 * dt:
            kinds = set(leaves) or {"none"}
            label = "wait:" + ("network-or-sleep(test)" if kinds & {"test-binary", "cli-under-test"} else
                               "subprocess(" + ",".join(sorted(kinds)) + ")")
            buckets[label] += dt
            continue
        if not leaves:
            buckets["orchestration(nx,shell)"] += dt
            continue
        share = dt / len(leaves)
        for k in leaves:
            buckets[k] += share
    return {"buckets": dict(buckets),
            "exits": {k: {"count": len(v), "seconds": round(sum(v), 1)} for k, v in exits.items()}}
