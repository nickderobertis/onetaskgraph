"""The records the CI profile reads and writes, each validated where it is decoded.

Every JSON document that crosses into the harness — GitHub's run, job and artifact records,
Nx's executions report, the sampler's and the test runner's records — is turned into one of
these types by its `parse` function, which raises `Invalid` naming the field it refused.
"""

from __future__ import annotations

import math
from dataclasses import dataclass
from datetime import datetime, timezone
from enum import StrEnum
from typing import Any, NewType

RunId = NewType("RunId", int)
JobId = NewType("JobId", int)
ArtifactId = NewType("ArtifactId", int)
TaskId = NewType("TaskId", str)  # "<project>:<target>"
Seconds = float


class Invalid(ValueError):
    """A decoded record did not have the shape the harness reads."""


class JobName(StrEnum):
    LINUX = "test-linux"
    MACOS = "test-macos"
    WINDOWS = "test-windows"


class TaskResult(StrEnum):
    EXECUTED = "executed"
    REMOTE_HIT = "remote_hit"
    LOCAL_HIT = "local_hit"

    @classmethod
    def of(cls, value: str) -> TaskResult:
        # Nx's report says `failed` for a task that ran and failed: it was executed all the same.
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


def timestamp(value: Any, where: str) -> float:
    if not isinstance(value, str):
        raise Invalid(f"{where}: expected an ISO-8601 time, got {value!r}")
    text = value.rstrip("Z")
    if "." in text:
        head, fraction = text.split(".", 1)
        text = f"{head}.{fraction[:6]}"
    try:
        return datetime.fromisoformat(text).replace(tzinfo=timezone.utc).timestamp()
    except ValueError as error:
        raise Invalid(f"{where}: {value!r} is not an ISO-8601 time") from error


def items(value: Any, where: str) -> list[Any]:
    if not isinstance(value, list):
        raise Invalid(f"{where}: expected a list, got {type(value).__name__}")
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
        if record.get("conclusion") == "skipped" or not record.get("started_at") or not record.get("completed_at"):
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
        steps = tuple(s for i, raw_step in enumerate(items(record.get("steps", []), where))
                      if (s := Step.parse(raw_step, f"{where} step {i}")) is not None)
        return cls(JobId(take(record, "id", int, where)), take(record, "name", str, where),
                   record.get("conclusion"), start, end, steps)


@dataclass(frozen=True)
class PullRequestRef:
    number: int

    @classmethod
    def parse(cls, raw: Any, where: str) -> PullRequestRef:
        return cls(take(obj(raw, where), "number", int, where))


@dataclass(frozen=True)
class Run:
    id: RunId
    head_sha: str
    head_branch: str
    event: str
    created_at: str
    conclusion: str | None
    pull_requests: tuple[PullRequestRef, ...]

    @classmethod
    def parse(cls, raw: Any) -> Run:
        record = obj(raw, "run")
        where = f"run {record.get('id')}"
        created = take(record, "created_at", str, where)
        timestamp(created, where)
        return cls(RunId(take(record, "id", int, where)), take(record, "head_sha", str, where),
                   take(record, "head_branch", str, where), take(record, "event", str, where), created,
                   record.get("conclusion"),
                   tuple(PullRequestRef.parse(p, where) for p in items(record.get("pull_requests") or [], where)))


@dataclass(frozen=True)
class Artifact:
    id: ArtifactId
    name: str
    expired: bool

    @classmethod
    def parse(cls, raw: Any) -> Artifact:
        record = obj(raw, "artifact")
        where = f"artifact {record.get('id')}"
        return cls(ArtifactId(take(record, "id", int, where)), take(record, "name", str, where),
                   take(record, "expired", bool, where))


@dataclass(frozen=True)
class RateLimit:
    rest: int
    graphql: int

    @classmethod
    def parse(cls, raw: Any) -> RateLimit:
        resources = obj(obj(raw, "rate_limit").get("resources"), "rate_limit.resources")
        return cls(take(obj(resources.get("core"), "core"), "remaining", int, "core"),
                   take(obj(resources.get("graphql"), "graphql"), "remaining", int, "graphql"))


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


@dataclass(frozen=True)
class TestTime:
    binary: str
    name: str
    seconds: Seconds

    @property
    def qualified(self) -> str:
        return f"{self.binary}::{self.name}"


@dataclass(frozen=True)
class SuiteTime:
    binary: str
    seconds: Seconds


def test_record(raw: Any) -> TestTime | SuiteTime | None:
    """One event of the test runner's record: a finished test, a finished suite, or neither."""
    match raw:
        case {"type": "test", "event": "ok" | "failed", "binary": str(binary), "name": str(name), "exec_time": exec_time}:
            return TestTime(binary, name, seconds(exec_time, f"{binary}::{name}"))
        case {"type": "suite", "event": "ok" | "failed", "binary": str(binary), **rest}:
            return SuiteTime(binary, seconds(rest.get("exec_time", 0), binary))
        case _:
            return None


@dataclass(frozen=True)
class Tick:
    t: float
    dt: Seconds
    leaves: dict[Category, int]
    cpu: dict[Category, Seconds]


@dataclass(frozen=True)
class Exit:
    category: Category
    start: float
    end: float
    cpu: Seconds


def category(value: Any, where: str) -> Category:
    try:
        return Category(value)
    except ValueError as error:
        raise Invalid(f"{where}: unknown process category {value!r}") from error


def sampler_record(raw: Any) -> Tick | Exit | None:
    match raw:
        case {"kind": "tick", "t": t, "dt": dt, "leaves": dict(leaves), "cpu": dict(cpu)}:
            return Tick(seconds(t, "tick"), seconds(dt, "tick"),
                        {category(k, "tick leaves"): int(v) for k, v in leaves.items()},
                        {category(k, "tick cpu"): seconds(v, "tick cpu") for k, v in cpu.items()})
        case {"kind": "exit", "category": name, "start": start, "end": end, "cpu": cpu}:
            begin, finish = seconds(start, "exit"), seconds(end, "exit")
            if finish < begin:
                raise Invalid("exit: ends before it starts")
            return Exit(category(name, "exit"), begin, finish, seconds(cpu, "exit"))
        case {"kind": "start" | "stop"}:
            return None
        case _:
            raise Invalid(f"sampler record of unknown shape: {str(raw)[:80]}")
