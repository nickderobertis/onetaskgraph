"""The records the CI profile reads and writes, each validated where it is decoded.

Every JSON document that crosses into the harness — GitHub's run, job and artifact records,
Nx's executions report, libtest's event stream — is turned into one of these types by its
`parse`, which raises `Invalid` naming the field it refused. The documents the harness writes
itself — the sampler's records, the history the replay reads — are written by these types'
`to_json` and read back by their `parse`, so writer and reader share one definition.
"""

from __future__ import annotations

import math
import re
from dataclasses import dataclass, field
from datetime import UTC, datetime
from enum import StrEnum
from typing import Any, NewType

RunId = NewType("RunId", int)
JobId = NewType("JobId", int)
ArtifactId = NewType("ArtifactId", int)
TaskId = NewType("TaskId", str)  # "<project>:<target>"
Seconds = float

ISO_TIME = re.compile(r"\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.(\d+))?(Z|[+-]\d\d:\d\d)?")
CONCLUSIONS = frozenset(
    {
        "success",
        "failure",
        "cancelled",
        "skipped",
        "timed_out",
        "neutral",
        "action_required",
        "stale",
        "startup_failure",
    }
)


class Invalid(ValueError):
    """A decoded record did not have the shape the harness reads."""


class JobName(StrEnum):
    LINUX = "test-linux"
    MACOS = "test-macos"
    WINDOWS = "test-windows"


class TaskResult(StrEnum):
    """An Nx task's result as `scripts/nx-remote-cache.mjs report` writes it into executions.json."""

    EXECUTED = "executed"
    REMOTE_HIT = "remote_hit"
    LOCAL_HIT = "local_hit"

    @classmethod
    def of(cls, value: str) -> TaskResult:
        # The report says `failed` for a task that ran and failed: it was executed all the same.
        if value == "failed":
            return cls.EXECUTED
        try:
            return cls(value)
        except ValueError as error:
            raise Invalid(f"unknown Nx task result {value!r}") from error

    @property
    def replayed(self) -> bool:
        return self is not TaskResult.EXECUTED


class Category(StrEnum):
    """What a process is, as the sampler classifies it; the report reads the same enum."""

    COMPILE = "compile"
    DOCTEST = "doctest"
    LINK = "link"
    CARGO = "cargo"
    COVERAGE_REPORT = "coverage-report"
    CLI_UNDER_TEST = "cli-under-test"
    GIT = "git"
    NX = "nx"
    NX_CACHE_SIDECAR = "nx-cache-sidecar"
    NODE = "node"
    BUN = "bun"
    PYTHON = "python"
    ONEVCS = "onevcs"
    GH = "gh"
    NETWORK = "network"
    ARCHIVE = "archive"
    TEST_BINARY = "test-binary"
    SHELL = "shell"
    JUST = "just"
    OTHER = "other"


# Orchestration a leaf of real work waits under; these never own a sampled second themselves.
ORCHESTRATION = frozenset({Category.NX, Category.NX_CACHE_SIDECAR, Category.OTHER, Category.SHELL, Category.JUST})


def obj(value: Any, where: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise Invalid(f"{where}: expected an object, got {type(value).__name__}")
    return value


def take(record: dict[str, Any], key: str, kind: type | tuple[type, ...], where: str) -> Any:
    if key not in record:
        raise Invalid(f"{where}: missing {key!r}")
    value = record[key]
    if not isinstance(value, kind) or (isinstance(value, bool) and kind is not bool):
        raise Invalid(f"{where}: {key!r} is {type(value).__name__}, not {kind}")
    return value


def seconds(value: Any, where: str) -> Seconds:
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value) or value < 0:
        raise Invalid(f"{where}: expected a finite non-negative number of seconds, got {value!r}")
    return float(value)


def count(value: Any, where: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        raise Invalid(f"{where}: expected a non-negative whole number, got {value!r}")
    return value


def timestamp(value: Any, where: str) -> float:
    """Seconds since the epoch of an ISO-8601 time; one with no offset is UTC, as GitHub's logs are."""
    if not isinstance(value, str) or not (m := ISO_TIME.fullmatch(value)):
        raise Invalid(f"{where}: {value!r} is not an ISO-8601 time")
    fraction, offset = m.group(1), m.group(2)
    text = value[: m.start(1) - 1] if fraction is not None else value[: len(value) - len(offset or "")]
    if fraction is not None:
        text = f"{text}.{fraction[:6].ljust(6, '0')}"
    zone = "+00:00" if offset in (None, "Z") else offset
    return datetime.fromisoformat(text + zone).astimezone(UTC).timestamp()


def items(value: Any, where: str) -> list[Any]:
    if not isinstance(value, list):
        raise Invalid(f"{where}: expected a list, got {type(value).__name__}")
    return value


def conclusion(value: Any, where: str) -> str | None:
    if value is not None and value not in CONCLUSIONS:
        raise Invalid(f"{where}: unknown conclusion {value!r}")
    return value


@dataclass(frozen=True)
class Step:
    name: str
    start: float
    end: float

    @classmethod
    def parse(cls, raw: Any, where: str) -> Step | None:
        record = obj(raw, where)
        name = take(record, "name", str, where)
        if (
            conclusion(record.get("conclusion"), where) == "skipped"
            or not record.get("started_at")
            or not record.get("completed_at")
        ):
            return None
        start, end = timestamp(record["started_at"], where), timestamp(record["completed_at"], where)
        if end < start:
            raise Invalid(f"{where}: step {name!r} ends before it starts")
        return cls(name, start, end)


@dataclass(frozen=True)
class Job:
    id: JobId
    name: str
    conclusion: str | None
    start: float | None
    end: float | None
    steps: tuple[Step, ...]

    @property
    def finished(self) -> bool:
        return self.start is not None and self.end is not None and self.conclusion not in (None, "skipped")

    @property
    def wall(self) -> Seconds:
        return (self.end or 0.0) - (self.start or 0.0)

    @classmethod
    def parse(cls, raw: Any) -> Job:
        record = obj(raw, "job")
        where = f"job {record.get('id')}"
        start = timestamp(record["started_at"], where) if record.get("started_at") else None
        end = timestamp(record["completed_at"], where) if record.get("completed_at") else None
        if start is not None and end is not None and end < start:
            raise Invalid(f"{where}: completes before it starts")
        steps = tuple(
            s
            for i, raw_step in enumerate(items(record.get("steps", []), where))
            if (s := Step.parse(raw_step, f"{where} step {i}")) is not None
        )
        return cls(
            JobId(take(record, "id", int, where)),
            take(record, "name", str, where),
            conclusion(record.get("conclusion"), where),
            start,
            end,
            steps,
        )


@dataclass(frozen=True)
class Run:
    id: RunId
    head_sha: str
    head_branch: str
    event: str
    created_at: str
    conclusion: str | None
    pull_requests: tuple[int, ...]

    @classmethod
    def parse(cls, raw: Any) -> Run:
        record = obj(raw, "run")
        where = f"run {record.get('id')}"
        created = take(record, "created_at", str, where)
        timestamp(created, where)
        numbers = tuple(
            take(obj(p, where), "number", int, where) for p in items(record.get("pull_requests") or [], where)
        )
        return cls(
            RunId(take(record, "id", int, where)),
            take(record, "head_sha", str, where),
            take(record, "head_branch", str, where),
            take(record, "event", str, where),
            created,
            conclusion(record.get("conclusion"), where),
            numbers,
        )


@dataclass(frozen=True)
class Artifact:
    id: ArtifactId
    name: str
    expired: bool

    @classmethod
    def parse(cls, raw: Any) -> Artifact:
        record = obj(raw, "artifact")
        where = f"artifact {record.get('id')}"
        return cls(
            ArtifactId(take(record, "id", int, where)),
            take(record, "name", str, where),
            take(record, "expired", bool, where),
        )


@dataclass(frozen=True)
class RateLimit:
    rest: int
    graphql: int

    @classmethod
    def parse(cls, raw: Any) -> RateLimit:
        resources = obj(obj(raw, "rate_limit").get("resources"), "rate_limit.resources")
        return cls(
            count(take(obj(resources.get("core"), "core"), "remaining", int, "core"), "core"),
            count(take(obj(resources.get("graphql"), "graphql"), "remaining", int, "graphql"), "graphql"),
        )


@dataclass(frozen=True)
class NxExecution:
    task: TaskId
    result: TaskResult
    start: float
    end: float

    @classmethod
    def parse(cls, raw: Any) -> NxExecution:
        record = obj(raw, "Nx execution")
        task = take(record, "task", str, "Nx execution")
        if ":" not in task:
            raise Invalid(f"Nx execution: {task!r} is not project:target")
        start = timestamp(take(record, "started_at", str, task), task)
        end = timestamp(take(record, "finished_at", str, task), task)
        if end < start:
            raise Invalid(f"Nx execution {task}: finishes before it starts")
        return cls(TaskId(task), TaskResult.of(take(record, "result", str, task)), start, end)

    @classmethod
    def report(cls, raw: Any) -> tuple[NxExecution, ...]:
        return tuple(cls.parse(t) for t in items(obj(raw, "executions.json").get("tasks"), "executions.json tasks"))


class LibtestKind(StrEnum):
    SUITE = "suite"
    TEST = "test"


class LibtestOutcome(StrEnum):
    STARTED = "started"
    OK = "ok"
    FAILED = "failed"
    IGNORED = "ignored"


@dataclass(frozen=True)
class LibtestEvent:
    """One event of libtest's `--format=json` stream, as the recording runner keeps it."""

    kind: LibtestKind
    outcome: LibtestOutcome
    binary: str
    name: str | None = None
    exec_time: Seconds | None = None
    test_count: int | None = None
    passed: int = 0
    failed: int = 0
    ignored: int = 0
    filtered_out: int = 0
    stdout: str = ""

    @classmethod
    def parse(cls, raw: Any, binary: str | None = None) -> LibtestEvent:
        record = obj(raw, "libtest event")
        where = f"libtest event of {binary or record.get('binary')}"
        try:
            kind, outcome = LibtestKind(record.get("type")), LibtestOutcome(record.get("event"))
        except ValueError as error:
            raise Invalid(f"{where}: {record.get('type')!r}/{record.get('event')!r} is no libtest event") from error
        name = record.get("name")
        if kind is LibtestKind.TEST and not isinstance(name, str):
            raise Invalid(f"{where}: a test event names no test")
        time_value = record.get("exec_time")
        stdout = record.get("stdout", "")
        if not isinstance(stdout, str):
            raise Invalid(f"{where}: stdout is not text")
        return cls(
            kind=kind,
            outcome=outcome,
            binary=binary if binary is not None else take(record, "binary", str, where),
            name=name,
            exec_time=None if time_value is None else seconds(time_value, where),
            test_count=None if record.get("test_count") is None else count(record["test_count"], where),
            passed=count(record.get("passed", 0), where),
            failed=count(record.get("failed", 0), where),
            ignored=count(record.get("ignored", 0), where),
            filtered_out=count(record.get("filtered_out", 0), where),
            stdout=stdout,
        )

    def to_json(self, at: float) -> dict[str, Any]:
        out: dict[str, Any] = {"type": self.kind.value, "event": self.outcome.value, "binary": self.binary, "t": at}
        for key in ("name", "exec_time", "test_count", "stdout"):
            if getattr(self, key) not in (None, ""):
                out[key] = getattr(self, key)
        if self.kind is LibtestKind.SUITE:
            out.update(passed=self.passed, failed=self.failed, ignored=self.ignored, filtered_out=self.filtered_out)
        return out


@dataclass(frozen=True)
class TestTime:
    binary: str
    name: str
    seconds: Seconds

    @property
    def qualified(self) -> str:
        return f"{self.binary}::{self.name}"


def category(value: Any, where: str) -> Category:
    try:
        return Category(value)
    except ValueError as error:
        raise Invalid(f"{where}: unknown process category {value!r}") from error


@dataclass(frozen=True)
class Tick:
    """One sample of the process table: which categories had a leaf alive, and their CPU since the last."""

    t: float
    dt: Seconds
    leaves: dict[Category, int]
    cpu: dict[Category, Seconds]

    def to_json(self) -> dict[str, Any]:
        return {
            "kind": "tick",
            "t": self.t,
            "dt": self.dt,
            "leaves": {k.value: v for k, v in self.leaves.items()},
            "cpu": {k.value: round(v, 4) for k, v in self.cpu.items() if v},
        }


@dataclass(frozen=True)
class Exit:
    """A process the sampler saw end: its category, its lifetime and its CPU."""

    category: Category
    start: float
    end: float
    cpu: Seconds

    def to_json(self) -> dict[str, Any]:
        return {
            "kind": "exit",
            "category": self.category.value,
            "start": self.start,
            "end": self.end,
            "cpu": round(self.cpu, 4),
        }


def sampler_record(raw: Any) -> Tick | Exit | None:
    match raw:
        case {"kind": "tick", "t": t, "dt": dt, "leaves": dict(leaves), "cpu": dict(cpu)}:
            return Tick(
                seconds(t, "tick"),
                seconds(dt, "tick"),
                {category(k, "tick leaves"): count(v, "tick leaves") for k, v in leaves.items()},
                {category(k, "tick cpu"): seconds(v, "tick cpu") for k, v in cpu.items()},
            )
        case {"kind": "exit", "category": name, "start": start, "end": end, "cpu": cpu}:
            begin, finish = seconds(start, "exit"), seconds(end, "exit")
            if finish < begin:
                raise Invalid("exit: ends before it starts")
            return Exit(category(name, "exit"), begin, finish, seconds(cpu, "exit"))
        case {"kind": "start" | "stop"}:
            return None
        case _:
            raise Invalid(f"sampler record of unknown shape: {str(raw)[:80]}")


@dataclass(frozen=True)
class TaskRecord:
    """One Nx task of one job of a PR's run, as `ciprofile.py history` writes it and `replay.py` reads it."""

    result: TaskResult
    seconds: Seconds
    share_s: Seconds

    def to_json(self) -> dict[str, Any]:
        return {"result": self.result.value, "seconds": round(self.seconds, 1), "share_s": round(self.share_s, 1)}

    @classmethod
    def parse(cls, raw: Any, where: str) -> TaskRecord:
        record = obj(raw, where)
        return cls(
            TaskResult.of(take(record, "result", str, where)),
            seconds(take(record, "seconds", (int, float), where), where),
            seconds(take(record, "share_s", (int, float), where), where),
        )


@dataclass(frozen=True)
class JobRecord:
    job_id: int
    conclusion: str | None
    wall_s: Seconds
    source: str
    buckets_s: dict[str, Seconds]
    tasks: dict[TaskId, TaskRecord]

    def to_json(self) -> dict[str, Any]:
        return {
            "job_id": self.job_id,
            "conclusion": self.conclusion,
            "wall_s": round(self.wall_s, 1),
            "source": self.source,
            "buckets_s": {k: round(v, 1) for k, v in self.buckets_s.items()},
            "tasks": {t: r.to_json() for t, r in self.tasks.items()},
        }

    @classmethod
    def parse(cls, raw: Any, where: str) -> JobRecord:
        record = obj(raw, where)
        return cls(
            take(record, "job_id", int, where),
            conclusion(record.get("conclusion"), where),
            seconds(take(record, "wall_s", (int, float), where), where),
            take(record, "source", str, where),
            {k: seconds(v, where) for k, v in obj(record.get("buckets_s", {}), where).items()},
            {
                TaskId(t): TaskRecord.parse(v, f"{where} {t}")
                for t, v in obj(take(record, "tasks", dict, where), where).items()
            },
        )


@dataclass(frozen=True)
class FirstRunRecord:
    run: int
    head_sha: str
    created_at: str
    conclusion: str | None
    runs_of_pr: int
    jobs: dict[JobName, JobRecord] = field(default_factory=dict)

    def to_json(self) -> dict[str, Any]:
        return {
            "run": self.run,
            "head_sha": self.head_sha,
            "created_at": self.created_at,
            "conclusion": self.conclusion,
            "runs_of_pr": self.runs_of_pr,
            "jobs": {j.value: r.to_json() for j, r in self.jobs.items()},
        }

    @classmethod
    def parse(cls, raw: Any, where: str) -> FirstRunRecord:
        record = obj(raw, where)
        jobs = {}
        for name, job in obj(record.get("jobs", {}), where).items():
            try:
                key = JobName(name)
            except ValueError as error:
                raise Invalid(f"{where}: unknown job {name!r}") from error
            jobs[key] = JobRecord.parse(job, f"{where} {name}")
        return cls(
            take(record, "run", int, where),
            take(record, "head_sha", str, where),
            take(record, "created_at", str, where),
            conclusion(record.get("conclusion"), where),
            take(record, "runs_of_pr", int, where),
            jobs,
        )


def history(raw: Any) -> dict[int, FirstRunRecord]:
    """A whole `ciprofile.py history` document, keyed by PR number."""
    out = {}
    for pr, entry in obj(raw, "history").items():
        if not str(pr).isdigit():
            raise Invalid(f"history: {pr!r} is not a pull request number")
        out[int(pr)] = FirstRunRecord.parse(entry, f"history #{pr}")
    return out
