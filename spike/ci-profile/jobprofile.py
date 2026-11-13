"""Divide one CI test job's wall clock from its step timings, its log and its Nx executions.

Every second of the job is assigned exactly once:
- a step outside the test step by its kind: setup, cache restore, cache save or report; a
  background step (the Nx cache sidecar) runs beside the others and owns no second;
- inside the test step, the Nx task timeline: each instant is split equally among the tasks
  running then, and an instant with no task running is Nx planning between phases;
- inside an executed task: its cargo builds (`Compiling` … `Finished`), as compile-and-link by
  profile; its time blocked on cargo's file lock; until its first test starts, compile; then
  test run. A replayed task's whole interval is cache restore.
On a demonstration run the test step is divided again by the sampler's record (`by_process`).
"""

from __future__ import annotations

import re
from collections import defaultdict
from dataclasses import dataclass, field
from enum import StrEnum

from models import (
    ORCHESTRATION,
    Category,
    Exit,
    Job,
    NxExecution,
    Seconds,
    TaskId,
    TaskResult,
    TestTime,
    Tick,
    sampler_record,
    timestamp,
)

ANSI = re.compile(r"\x1b\[[0-9;]*[A-Za-z]")
LINE = re.compile(r"^(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d+)?)Z ?(.*)$")
HEADER = re.compile(
    r"^\[(?P<phase>[^\]]+)\] (?P<replay>🔁 )?> nx run (?P<task>[\w.@/-]+:[\w-]+)"
    r"(?P<remote>\s+\[(?:remote|local) cache\])?"
)
PREFIXED = re.compile(r"^\[[^\]]+\] (?P<project>[\w.@/-]+): +(?P<text>.*)$")
TEST_START = re.compile(r"^(?:\[[^\]]+\] )?(?P<project>[\w.@/-]+): (?:running \d+ tests?|\s*Starting \d+ tests?)")
JSON_TEST = re.compile(r'^\[[^\]]+\] (?:(?P<project>[\w.@/-]+): )?\{"type":"(?:test|suite)".*?"name":"(?P<name>[^"]+)"')
NEXTEST_DONE = re.compile(
    r'^\[[^\]]+\] (?:[\w.@/-]+: )?\{"type":"test","event":"(?:ok|failed|ignored)","name":"(?P<name>[^"]+)"'
    r'(?:,"exec_time":(?P<secs>[0-9.]+))?'
)
CARGO_BUILD = re.compile(r"^(?P<word>Compiling|Finished) (?:`(?P<profile>\w+)` profile)?")
STORED = re.compile(r"Nx cache: stored (?P<n>\d+) entries, (?P<bytes>\d+) bytes; replayed (?P<r>\d+) entries")
CACHE_SIZE = re.compile(r"Cache Size: ~?([0-9.]+ ?[MG]B)")
RESTORE_BOUNDS = 2  # a restore is measured between its first and last restore line
STEP_SLACK = 2.0  # seconds: the step API keeps whole seconds, the log's groups are exact
SHORTEST_LINK = 0.5  # seconds: shorter tasks are replays, which hold no slot worth chaining
# Below this share of one CPU, work processes in a tick are waiting rather than working.
IDLE_CPU_SHARE = 0.1


class Bucket(StrEnum):
    SETUP = "setup"
    CACHE_RESTORE = "cache-restore"
    CACHE_SAVE = "cache-save"
    REPORT = "report"
    NX_RESTORE = "cache-restore(nx)"
    NX_PLANNING = "nx-planning"
    COMPILE_PLAIN = "compile-link(plain)"
    COMPILE_INSTRUMENTED = "compile-link(instrumented)"
    COMPILE_RELEASE = "compile-link(release)"
    LOCK_WAIT = "wait:cargo-file-lock"
    SCRIPTS = "scripts-and-other-tasks"
    BUDGETS = "budget-checks"
    TEST_PLAIN = "test-run(plain)"
    TEST_INSTRUMENTED = "test-run(instrumented)"
    UNATTRIBUTED = "unattributed"


class StepKind(StrEnum):
    SETUP = "setup"
    CACHE_RESTORE = "cache-restore"
    CACHE_SAVE = "cache-save"
    REPORT = "report"
    TEST = "test"
    BACKGROUND = "background"


@dataclass(frozen=True)
class LogLine:
    t: float
    text: str


@dataclass(frozen=True)
class Interval:
    start: float
    end: float


@dataclass(frozen=True)
class Build(Interval):
    profile: str


@dataclass
class Task:
    id: TaskId
    start: float
    end: float
    result: TaskResult
    compile_end: float | None = None
    builds: list[Build] = field(default_factory=list)
    lock_waits: list[Interval] = field(default_factory=list)

    @property
    def project(self) -> str:
        return self.id.split(":")[0]

    @property
    def target(self) -> str:
        return self.id.split(":")[1]

    @property
    def instrumented(self) -> bool:
        return self.target in ("coverage", "coverage-build")

    def bucket_at(self, instant: float) -> Bucket:  # noqa: PLR0911 — one return per case of a single match
        """The category this task's share of one instant belongs to; earlier cases win."""
        build = next((b for b in self.builds if b.start <= instant < b.end), None)
        waiting = any(w.start <= instant < w.end for w in self.lock_waits)
        compiling = self.compile_end is not None and instant < self.compile_end
        match (self.result.replayed, waiting, build, compiling, self.project, self.target):
            case (True, *_):
                return Bucket.NX_RESTORE
            case (_, True, *_):
                return Bucket.LOCK_WAIT
            case (_, _, Build(profile="release"), *_):
                return Bucket.COMPILE_RELEASE
            case (_, _, Build(), *_) | (_, _, None, True, *_):
                return Bucket.COMPILE_INSTRUMENTED if self.instrumented else Bucket.COMPILE_PLAIN
            case (*_, "workspace" | "scripts", _):
                return Bucket.SCRIPTS
            case (*_, "budgets"):
                return Bucket.BUDGETS
            case (*_, "test" | "coverage" | "test-uncovered"):
                return Bucket.TEST_INSTRUMENTED if self.instrumented else Bucket.TEST_PLAIN
            case _:
                return Bucket.SCRIPTS


@dataclass(frozen=True)
class RustCache:
    seconds: Seconds
    size: str | None
    hit: bool


@dataclass(frozen=True)
class Stored:
    entries: int
    bytes: int
    replayed: int


@dataclass
class JobProfile:
    job: Job
    buckets: dict[Bucket, Seconds] = field(default_factory=lambda: defaultdict(float))
    tasks: list[Task] = field(default_factory=list)
    task_share: dict[TaskId, Seconds] = field(default_factory=lambda: defaultdict(float))
    nextest: list[TestTime] = field(default_factory=list)
    stored: Stored | None = None
    rust_cache: RustCache | None = None
    latest_chain: list[Task] = field(default_factory=list)
    test_step: Interval | None = None

    @property
    def wall(self) -> Seconds:
        return self.job.wall


def parse_log(text: str) -> list[LogLine]:
    out = []
    for raw in text.splitlines():
        m = LINE.match(raw.lstrip("\ufeff"))
        if m:
            out.append(LogLine(timestamp(m.group(1), "log line"), ANSI.sub("", m.group(2))))
    return out


def step_kind(name: str) -> StepKind:  # noqa: PLR0911 — one return per case of a single match
    n = name.lower()
    match n:
        case _ if "start-cache-server" in n:
            return StepKind.BACKGROUND
        case _ if n.startswith("post ") and any(a in n for a in ("setup-uv", "setup-bun", "rust-cache", "cache/save")):
            return StepKind.CACHE_SAVE
        case _ if n.startswith("post "):
            return StepKind.REPORT
        case _ if "cache/restore" in n:
            return StepKind.CACHE_RESTORE
        case _ if "cache/save" in n:
            return StepKind.CACHE_SAVE
        case _ if n.startswith("test the affected"):
            return StepKind.TEST
        case _ if (
            any(
                w in n
                for w in ("upload-artifact", "report cache", "budget", "cancell", "live artifacts", "linear leases")
            )
            or n == "complete job"
        ):
            return StepKind.REPORT
        case _:
            return StepKind.SETUP


@dataclass(frozen=True)
class Section:
    """One top-level `##[group]Run` section of a job log: a step, or a step of a composite action."""

    span: Interval
    title: str


def groups(log: list[LogLine]) -> list[Section]:
    marks = [
        LogLine(line.t, line.text[len("##[group]Run ") :]) for line in log if line.text.startswith("##[group]Run ")
    ]
    return [
        Section(Interval(m.t, marks[i + 1].t if i + 1 < len(marks) else log[-1].t), m.text) for i, m in enumerate(marks)
    ]


def restore_inside(log: list[LogLine], span: Interval) -> Seconds:
    hits = [
        line.t
        for line in log
        if span.start <= line.t <= span.end
        and re.search(r"restor|cache key|Cache (hit|not found)|Received \d+ of", line.text, re.I)
    ]
    return (max(hits) - min(hits)) if len(hits) >= RESTORE_BOUNDS else 0.0


def account_steps(p: JobProfile, log: list[LogLine]) -> None:
    sections = groups(log)
    restore = 0.0
    for section in sections:
        span, title = section.span, section.title
        if title.startswith("Swatinem/rust-cache"):
            text = "\n".join(line.text for line in log if span.start <= line.t <= span.end)
            size = CACHE_SIZE.search(text)
            p.rust_cache = RustCache(
                round(span.end - span.start, 1),
                size.group(1) if size else None,
                "restored from" in text.lower() and "No cache found" not in text,
            )
            restore += span.end - span.start
        elif title.startswith(("astral-sh/setup-uv", "oven-sh/setup-bun")):
            restore += restore_inside(log, span)
    for step in p.job.steps:
        kind = step_kind(step.name)
        if kind is StepKind.TEST:
            # The step API keeps whole seconds; the log's own group for the step is exact.
            near = [x.span for x in sections if abs(x.span.start - step.start) <= STEP_SLACK]
            p.test_step = (
                min(near, key=lambda span: abs(span.start - step.start)) if near else Interval(step.start, step.end)
            )
        elif kind is not StepKind.BACKGROUND:
            p.buckets[Bucket(kind.value)] += step.end - step.start
    p.buckets[Bucket.SETUP] -= restore
    p.buckets[Bucket.CACHE_RESTORE] += restore


def tasks_from_log(log: list[LogLine], window: Interval | None) -> list[Task]:
    """Tasks placed by the log alone: from a task's header to its project's last line before its next header."""
    inside = [line for line in log if window is None or window.start <= line.t <= window.end]
    out: list[Task] = []
    for line in inside:
        m = HEADER.match(line.text)
        if m:
            remote = m.group("remote") or ""
            result = (
                TaskResult.LOCAL_HIT
                if "local cache" in remote
                else TaskResult.REMOTE_HIT
                if remote or m.group("replay")
                else TaskResult.EXECUTED
            )
            out.append(Task(TaskId(m.group("task")), line.t, line.t, result))
    prefixed: dict[str, list[float]] = defaultdict(list)
    for line in inside:
        m = PREFIXED.match(line.text)
        if m:
            prefixed[m.group("project")].append(line.t)
    for task in out:
        if task.result.replayed:
            continue
        nxt = min((x.start for x in out if x.project == task.project and x.start > task.start), default=float("inf"))
        task.end = max((t for t in prefixed.get(task.project, []) if task.start <= t < nxt), default=task.start)
    return out


def place_tasks(p: JobProfile, log: list[LogLine], executions: tuple[NxExecution, ...] | None) -> None:
    window = p.test_step
    tasks: list[Task] = []
    if executions is not None:
        tasks = [
            Task(e.task, e.start, e.end, e.result)
            for e in executions
            if not e.task.endswith(":bootstrap") and (window is None or window.start - 2 <= e.start <= window.end + 2)
        ]
        # A cancelled job's unfinished Nx invocations never wrote their executions.
        recorded = {t.id for t in tasks}
        tasks += [t for t in tasks_from_log(log, window) if t.id not in recorded]
    else:
        tasks = tasks_from_log(log, window)
    starts: dict[str, list[float]] = defaultdict(list)
    lines: dict[str, list[LogLine]] = defaultdict(list)
    for line in log:
        if m := TEST_START.match(line.text):
            starts[m.group("project")].append(line.t)
        elif m := JSON_TEST.match(line.text):
            starts[m.group("name").split("::")[0]].append(line.t)
        if m := PREFIXED.match(line.text):
            lines[m.group("project")].append(LogLine(line.t, m.group("text")))
    for task in tasks:
        if task.result.replayed:
            continue
        mine = [line for line in lines.get(task.project, []) if task.start <= line.t <= task.end]
        read_builds(task, mine)
        first = [t for t in starts.get(task.project, []) if task.start <= t <= task.end]
        if first and task.target in ("test", "coverage", "test-uncovered"):
            task.compile_end = min(first)
        elif task.target in ("build", "coverage-build"):
            task.compile_end = task.end
    p.tasks = tasks


def read_builds(task: Task, lines: list[LogLine]) -> None:
    opened: float | None = None
    for i, line in enumerate(lines):
        if line.text.startswith("Blocking waiting for file lock"):
            later = next((x.t for x in lines[i + 1 :] if not x.text.startswith("Blocking waiting")), None)
            if later is not None and later > line.t and not (task.lock_waits and task.lock_waits[-1].end >= line.t):
                task.lock_waits.append(Interval(line.t, later))
            continue
        match CARGO_BUILD.match(line.text):
            case re.Match() as m if m.group("word") == "Compiling" and opened is None:
                opened = line.t
            case re.Match() as m if m.group("word") == "Finished" and opened is not None:
                task.builds.append(Build(opened, line.t, m.group("profile") or "dev"))
                opened = None
            case _:
                pass


def attribute_test_step(p: JobProfile) -> None:
    assert p.test_step is not None
    edges = {p.test_step.start, p.test_step.end}
    for t in p.tasks:
        for x in (
            t.start,
            t.end,
            t.compile_end,
            *[e for b in t.builds for e in (b.start, b.end)],
            *[e for w in t.lock_waits for e in (w.start, w.end)],
        ):
            if x is not None and p.test_step.start < x < p.test_step.end:
                edges.add(x)
    ordered = sorted(edges)
    for lo, hi in zip(ordered, ordered[1:], strict=False):
        mid = (lo + hi) / 2
        active = [t for t in p.tasks if t.start <= mid < t.end]
        if not active:
            p.buckets[Bucket.NX_PLANNING] += hi - lo
            continue
        share = (hi - lo) / len(active)
        for t in active:
            p.task_share[t.id] += share
            p.buckets[t.bucket_at(mid)] += share


def latest_finishing_chain(tasks: list[Task], window: Interval) -> list[Task]:
    """Walking back from the step's end, the task finishing latest before each pointer.

    Nx's report carries no dependency edges, so this is a timeline chain, not a dependency
    critical path: each link is what was still running when the next one began."""
    pointer, chain = window.end, []
    pool = [t for t in tasks if t.end - t.start > SHORTEST_LINK]
    while before := [t for t in pool if t.end <= pointer + 1 and t.start < pointer - 0.5]:
        link = max(before, key=lambda x: (x.end, -x.start))
        chain.append(link)
        pointer = link.start
        pool.remove(link)
    return list(reversed(chain))


def profile_job(job: Job, log_text: str, executions: tuple[NxExecution, ...] | None) -> JobProfile:
    log = parse_log(log_text)
    p = JobProfile(job)
    account_steps(p, log)
    place_tasks(p, log, executions)
    for line in log:
        if (m := NEXTEST_DONE.match(line.text)) and m.group("secs"):
            binary, _, name = m.group("name").rpartition("$")
            p.nextest.append(TestTime(f"instrumented:{binary}", name, float(m.group("secs"))))
        if m := STORED.search(line.text):
            p.stored = Stored(int(m.group("n")), int(m.group("bytes")), int(m.group("r")))
    if p.test_step:
        attribute_test_step(p)
        p.latest_chain = latest_finishing_chain(p.tasks, p.test_step)
    p.buckets[Bucket.UNATTRIBUTED] += p.wall - sum(v for k, v in p.buckets.items() if k is not Bucket.UNATTRIBUTED)
    return p


@dataclass
class ExitTotals:
    """The processes of one category that ended in a window."""

    count: int = 0
    lifetime: Seconds = 0.0
    cpu: Seconds = 0.0


@dataclass(frozen=True)
class ProcessDivision:
    seconds: dict[str, Seconds]
    exits: dict[Category, ExitTotals]


def by_process(records: list, window: Interval) -> ProcessDivision:
    """Divide `window` by the sampler's leaves. A tick whose work processes spent under
    IDLE_CPU_SHARE of one CPU is a wait (sleep, poll, network or timeout), named by what waited."""
    buckets: dict[str, Seconds] = defaultdict(float)
    exits: dict[Category, ExitTotals] = defaultdict(ExitTotals)
    for record in records:
        match record:
            case Exit() if window.start <= record.end <= window.end:
                totals = exits[record.category]
                totals.count += 1
                totals.lifetime += record.end - record.start
                totals.cpu += record.cpu
            case Tick() if window.start <= record.t <= window.end:
                divide_tick(record, buckets)
            case _:
                pass
    return ProcessDivision(dict(buckets), dict(exits))


# A tick this much longer than the sampler's own interval means the recorder itself stalled: the
# excess is named unsampled rather than credited to whatever happened to be alive at its end.
LONGEST_TICK = 5.0


def divide_tick(tick: Tick, buckets: dict[str, Seconds]) -> None:
    dt = min(tick.dt, LONGEST_TICK)
    if tick.dt > LONGEST_TICK:
        buckets["unsampled(recorder stalled)"] += tick.dt - LONGEST_TICK
    work_cpu = sum(v for k, v in tick.cpu.items() if k not in ORCHESTRATION)
    leaves = {k for k, n in tick.leaves.items() if n and k not in ORCHESTRATION}
    if work_cpu < IDLE_CPU_SHARE * dt:
        waiting = {Category.TEST_BINARY, Category.CLI_UNDER_TEST} & leaves
        label = (
            "wait:in-tests(network,sleep,poll)"
            if waiting
            else "wait:subprocess(" + ",".join(sorted(k.value for k in leaves) or ["none"]) + ")"
        )
        buckets[label] += dt
    elif not leaves:
        buckets["orchestration(nx,shell)"] += dt
    else:
        for k in leaves:
            buckets[k.value] += dt / len(leaves)


def sampler_records(lines: list[str]) -> list:
    import json  # noqa: PLC0415 — only a demonstration run carries a sampler record

    return [r for line in lines if line.strip() and (r := sampler_record(json.loads(line))) is not None]
