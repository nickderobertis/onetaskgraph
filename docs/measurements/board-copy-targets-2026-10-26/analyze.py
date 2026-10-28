# llmlint: ignore-file[contracts_have_one_source_or_a_drift_gate] The capture fields read here are the sanitizer's, shared by import (sanitize.REQUEST_KINDS, HOST_KINDS, SCREEN_NAMES, operation) and held by test_analyze.py, which builds every capture through sanitize.write and requires every loopback configuration; that suite sits outside every gate only because docs/measurements is no Nx project, which the new_code_lands_in_a_project note states.
# llmlint: ignore-file[new_code_lands_in_a_project] The report's own recomputation and its tests, run by hand beside the measurement record whose command the report names; docs/measurements is no Nx project, and making it one is a change to the project graph and its four guarded selections, outside this measurement.
"""Recompute every table of `board-copy-targets-2026-10-26.md` from the sanitized captures.

    python3 docs/measurements/board-copy-targets-2026-10-26/analyze.py           # check
    python3 docs/measurements/board-copy-targets-2026-10-26/analyze.py --write   # rewrite

Each table of the report sits between `<!-- analyze:begin NAME -->` and `<!-- analyze:end NAME -->`.
The default run recomputes every one from `captures/` and exits 1, naming the table, when the
report's copy differs from what the captures say; `--write` puts the recomputed tables in place,
and exits 1 when the report has no place for one or names one that does not exist. Both write
`captures/summary.json`, every figure the tables are made from.

A row is the latest capture of its scenario (`captures/<scenario>-<UTC stamp>/`); a capture moved
to `captures/superseded/` is never read as a row. Every capture is parsed into typed records and
refused, naming its file and line, when it is not what `harness/sanitize.py` writes. The targets
and the before figures are the brief's, stated once below, and no figure here is compared against
anything else.
"""

from __future__ import annotations

import argparse
import json
import math
import re
import sys
from collections import Counter, defaultdict
from dataclasses import asdict, dataclass, field
from enum import Enum
from pathlib import Path
from typing import Callable

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE / "harness"))

import sanitize  # the producer of every capture this reads

CAPTURES = HERE / "captures"
REPORT = HERE.parent / "board-copy-targets-2026-10-26.md"


class Verdict(str, Enum):
    WITHIN = "within"
    OVER = "over"


class Operation(str, Enum):
    """The four operations the user's targets name."""

    FIRST = "first"
    CHANGED1 = "changed1"
    UNCHANGED = "unchanged"
    APPROVE = "approve"


class RequestKind(str, Enum):
    QUERY = "query"
    MUTATION = "mutation"
    REST = "rest"
    TUNNEL = "tunnel"
    OTHER = "other"


class HostKind(str, Enum):
    STORE = "store"
    SUBPROCESS = "subprocess"
    INTERPRETER = "interpreter"
    STEP = "step"
    OTHER = "other"


class GateKind(str, Enum):
    READING = "reading"
    PAUSE = "pause"


@dataclass(frozen=True)
class Figures:
    """One operation's seconds, GraphQL points and writes; a figure not stated is `None`."""

    seconds: float | None = None
    points: int | None = None
    writes: int | None = None

    def of(self, name: str) -> float | None:
        return getattr(self, name)


@dataclass(frozen=True)
class Scenario:
    name: str
    operation: Operation
    label: str


# The user's targets, as the brief states them. Seconds warn on this host; points and writes
# are strict.
TARGETS: dict[Operation, Figures] = {
    Operation.FIRST: Figures(seconds=32, points=12, writes=40),
    Operation.CHANGED1: Figures(seconds=8, points=4),
    Operation.UNCHANGED: Figures(seconds=7, points=3),
    Operation.APPROVE: Figures(seconds=7, points=5),
}
# The brief's before figures: the host journey's copies on the loopback board at onetaskgraph
# 0.12.3. The brief gives none for an approval or for writes.
BEFORE: dict[Operation, Figures] = {
    Operation.FIRST: Figures(seconds=51, points=25),
    Operation.CHANGED1: Figures(seconds=20.8, points=18),
    Operation.UNCHANGED: Figures(seconds=17.4, points=14),
}
# The same operations measured on GitHub at onetaskgraph 0.11.0 on 2026-10-24
# (`board-copy-speed-2026-10-18.md`, combined table).
EARLIER: dict[str, Figures] = {
    "bare-first": Figures(seconds=29.8, points=20, writes=36),
    "bare-changed1": Figures(seconds=8.7, points=7, writes=1),
    "bare-unchanged": Figures(seconds=6.7, points=6, writes=0),
    "host-first": Figures(seconds=64.4, points=17, writes=25),
    "host-changed1": Figures(seconds=52.2, points=7, writes=2),
    "host-unchanged": Figures(seconds=44.7, points=4, writes=0),
    "host-approve": Figures(seconds=16.4, points=7, writes=1),
}
ROWS = (
    Scenario("bare-first", Operation.FIRST, "bare store: first copy"),
    Scenario(
        "bare-changed1",
        Operation.CHANGED1,
        "bare store: re-copy, one task's body changed",
    ),
    Scenario("bare-unchanged", Operation.UNCHANGED, "bare store: unchanged re-copy"),
    Scenario("host-first", Operation.FIRST, "host `just copy-plan`: first copy"),
    Scenario(
        "host-changed1",
        Operation.CHANGED1,
        "host `just copy-plan`: one task's body changed",
    ),
    Scenario(
        "host-unchanged",
        Operation.UNCHANGED,
        "host `just copy-plan`: unchanged re-copy",
    ),
    Scenario("host-approve", Operation.APPROVE, "host `just approve-design`"),
)
# The reference-size arithmetic of the timing spike (`board-copy-speed-2026-10-18.md`): every
# walk page the test board answered is replaced by the pages the same walk takes on a board of
# the `plans` board's size, at the latencies measured there.
WALK_PAGE_SECONDS = {"BOARD": 6.95, "SEARCH_ISSUES": 6.79}
WALK_PAGES_AT_REFERENCE = {"BOARD": 20, "SEARCH_ISSUES": 10}
COPY_VERBS = (("project", "copy"), ("document", "copy"))
# A mutation request's primary price, by the manager's ruling: GitHub's dry run cannot price a
# mutation, and the smallest rise of `x-ratelimit-used` observed before one is its evidence.
MUTATION_PRICE = 1
# The upload's request kind, spelled as the sanitizer spells it.
UPLOAD = sanitize.operation(
    {
        "kind": "rest",
        "method": "POST",
        "host": "uploads.github.com",
        "path": "/user-attachments/assets",
    }
)
CREATES = ("CREATE_ISSUE", "CREATE_ISSUES")


class CaptureError(SystemExit):
    """A capture that is not what `harness/sanitize.py` writes."""


def refuse(path: Path, number: int | None, why: str) -> CaptureError:
    where = f"{path}:{number}" if number else str(path)
    return CaptureError(
        f"analyze.py: {where}: {why}; take the capture again with measure.sh"
    )


def finite(value: object) -> bool:
    return (
        isinstance(value, (int, float))
        and not isinstance(value, bool)
        and math.isfinite(value)
    )


def whole(value: object) -> bool:
    return isinstance(value, int) and not isinstance(value, bool)


def count(value: object) -> bool:
    return whole(value) and value >= 0


def text(value: object) -> bool:
    return isinstance(value, str)


def words(value: object) -> bool:
    return isinstance(value, list) and all(isinstance(w, str) for w in value)


def document(path: Path) -> dict:
    """One capture's JSON object, or a refusal naming the file."""
    try:
        value = json.loads(path.read_text())
    except ValueError as error:
        raise refuse(path, None, f"not JSON ({error})") from error
    if not isinstance(value, dict):
        raise refuse(path, None, "not a JSON object")
    return value


def lines(path: Path) -> list[tuple[int, dict]]:
    """Each JSON object of a JSON-lines capture with its line number."""
    if not path.exists():
        return []
    out = []
    for number, line in enumerate(path.read_text().splitlines(), 1):
        if not line.strip():
            continue
        try:
            record = json.loads(line)
        except ValueError as error:
            raise refuse(path, number, f"not JSON ({error})") from error
        if not isinstance(record, dict):
            raise refuse(path, number, "not a JSON object")
        out.append((number, record))
    return out


def interval(
    path: Path, number: int, record: dict, end_required: bool = True
) -> tuple[float, float]:
    start, end = (
        record.get("start"),
        record.get("end", None if end_required else record.get("start")),
    )
    if not finite(start) or not finite(end):
        raise refuse(path, number, "a start or end that is not a finite number")
    if end < start:
        raise refuse(path, number, "an end before its start")
    return float(start), float(end)


@dataclass(frozen=True)
class Request:
    """One request the proxy recorded, as sanitized."""

    seq: int
    start: float
    end: float
    ms: float
    host: str
    path: str
    kind: RequestKind
    op: str
    mutations: int | None
    limits: dict[str, object]

    @property
    def refused(self) -> bool:
        return self.host == "refused"

    @property
    def graphql(self) -> bool:
        return self.path == "/graphql"

    @property
    def carried(self) -> int:
        """How many mutations it carried: an aliased one's count, else one."""
        return 1 if self.mutations is None else self.mutations


@dataclass(frozen=True)
class Step:
    """One timed command of a row."""

    argv: tuple[str, ...]
    start: float
    end: float
    returncode: int


@dataclass(frozen=True)
class HostEvent:
    """One event of the host's own audit: an interpreter's start or exit, a store invocation, a
    host subprocess, or a named host step."""

    kind: HostKind
    start: float
    end: float
    argv: tuple[str, ...] = ()
    program: str = ""
    name: str = ""


@dataclass(frozen=True)
class ScreenCall:
    name: str
    start: float
    end: float


def requests_of(path: Path) -> list[Request]:
    if not path.exists():
        raise refuse(path, None, "no request log")
    out = []
    for number, r in lines(path):
        start, end = interval(path, number, r)
        if r.get("kind") not in (
            *sanitize.REQUEST_KINDS,
            RequestKind.OTHER.value,
        ) or not text(r.get("op")):
            raise refuse(path, number, "a request with no known kind or operation")
        if (
            not count(r.get("seq"))
            or not finite(r.get("ms"))
            or not isinstance(r.get("limits"), dict)
        ):
            raise refuse(
                path, number, "a request without its sequence, duration or limits"
            )
        if not text(r.get("host")) or not text(r.get("path")):
            raise refuse(path, number, "a request without its host or path")
        if "mutations" in r and not count(r["mutations"]):
            raise refuse(path, number, "a mutation count that is not a whole number")
        out.append(
            Request(
                r["seq"],
                start,
                end,
                float(r["ms"]),
                r["host"],
                r["path"],
                RequestKind(r["kind"]),
                r["op"],
                r.get("mutations"),
                r["limits"],
            )
        )
    return out


def host_of(path: Path) -> list[HostEvent]:
    out = []
    for number, r in lines(path):
        if r.get("kind") not in (*sanitize.HOST_KINDS, HostKind.OTHER.value):
            raise refuse(path, number, "a host event of no known kind")
        kind = HostKind(r["kind"])
        start, end = interval(
            path, number, r, end_required=kind is not HostKind.INTERPRETER
        )
        if "argv" in r and not words(r["argv"]):
            raise refuse(
                path, number, "a host event whose command line is not a list of words"
            )
        if kind is HostKind.STORE and not words(r.get("argv")):
            raise refuse(path, number, "a store invocation without its command line")
        if kind is HostKind.SUBPROCESS and not text(r.get("program")):
            raise refuse(path, number, "a host subprocess without its program")
        if kind in (HostKind.STEP, HostKind.INTERPRETER) and not text(r.get("name")):
            raise refuse(
                path, number, "a host step or interpreter event without its name"
            )
        out.append(
            HostEvent(
                kind,
                start,
                end,
                tuple(r.get("argv", ())),
                r.get("program", ""),
                r.get("name", ""),
            )
        )
    return out


def steps_of(path: Path) -> list[Step]:
    out = []
    for number, r in lines(path):
        start, end = interval(path, number, r)
        if not words(r.get("argv")) or not whole(r.get("returncode")):
            raise refuse(path, number, "a step without its command line or exit status")
        out.append(Step(tuple(r["argv"]), start, end, r["returncode"]))
    if not out:
        raise refuse(path, None, "no timed step")
    return out


def screen_of(path: Path) -> list[ScreenCall]:
    out = []
    for number, r in lines(path):
        start, end = interval(path, number, r)
        if r.get("name") not in (*sanitize.SCREEN_NAMES, "other"):
            raise refuse(
                path, number, "a screening call that is neither an inspect nor a check"
            )
        out.append(ScreenCall(r["name"], start, end))
    return out


def prices_of(path: Path) -> tuple[dict[str, int], dict[str, list[int]]]:
    """Every query's dry-run price by `<capture>/<seq>`, and every run's observed mutation rises."""
    pricing = document(path) if path.exists() else {}
    requests, rises = pricing.get("requests", {}), pricing.get("mutation_rises", {})
    if not isinstance(requests, dict) or not isinstance(rises, dict):
        raise refuse(path, None, "`requests` or `mutation_rises` is not an object")
    for key, value in requests.items():
        if not count(value):
            raise refuse(
                path, None, f"the price of {key} is not a whole number of points"
            )
    for key, value in rises.items():
        if not isinstance(value, list) or not all(whole(v) for v in value):
            raise refuse(
                path, None, f"the mutation rises of {key} are not whole numbers"
            )
    return dict(requests), dict(rises)


@dataclass(frozen=True)
class Bucket:
    limit: int
    used: int
    remaining: int
    reset: int
    readings: tuple[int, ...] = ()


@dataclass(frozen=True)
class Reading:
    at: float
    used: int
    reset: int


@dataclass(frozen=True)
class GateEvent:
    event: GateKind
    proceed: bool
    seconds: float


@dataclass(frozen=True)
class Quota:
    """The allowance around a row: the gate's events, both readings, and the idle samples."""

    before: dict[str, Bucket]
    after: dict[str, Bucket]
    gate: tuple[GateEvent, ...]
    burn_before: tuple[Reading, ...]
    burn_after: tuple[Reading, ...]


BUCKETS = ("core", "graphql", "graphql (GET /rate_limit)", "graphql (rateLimit)")


def quota_of(path: Path) -> Quota:
    q = document(path)

    def buckets(name: str) -> dict[str, Bucket]:
        envelope = q.get(name)
        if not isinstance(envelope, dict):
            raise refuse(path, None, f"`{name}` is not an object")
        raw = envelope.get("buckets")
        if not isinstance(raw, dict) or any(b not in raw for b in BUCKETS):
            raise refuse(
                path, None, f"`{name}` does not carry every bucket {', '.join(BUCKETS)}"
            )
        out = {}
        for b, v in raw.items():
            if not isinstance(v, dict) or not all(
                count(v.get(k)) for k in ("limit", "used", "remaining", "reset")
            ):
                raise refuse(
                    path,
                    None,
                    f"`{name}` bucket {b} without its limit, used, remaining and reset",
                )
            readings = v.get("readings", [])
            if not isinstance(readings, list) or not all(count(r) for r in readings):
                raise refuse(
                    path,
                    None,
                    f"`{name}` bucket {b} with readings that are not whole numbers",
                )
            out[b] = Bucket(
                v["limit"], v["used"], v["remaining"], v["reset"], tuple(readings)
            )
        if not out["graphql"].limit:
            raise refuse(path, None, f"`{name}` reports a GraphQL limit of 0")
        return out

    def burn(name: str) -> tuple[Reading, ...]:
        raw = q.get(name, [])
        if not isinstance(raw, list) or not all(
            isinstance(r, dict)
            and finite(r.get("at"))
            and count(r.get("used"))
            and count(r.get("reset"))
            for r in raw
        ):
            raise refuse(path, None, f"`{name}` is not a list of timed readings")
        return tuple(Reading(float(r["at"]), r["used"], r["reset"]) for r in raw)

    events = q.get("gate", [])
    if not isinstance(events, list):
        raise refuse(path, None, "the gate's events are not a list")
    gate = []
    for event in events:
        if not isinstance(event, dict) or event.get("event") not in (
            "reading",
            "pause",
        ):
            raise refuse(
                path, None, "a gate event that is neither a reading nor a pause"
            )
        verdicts = event.get("verdicts", {})
        if event["event"] == "reading" and (
            not isinstance(verdicts, dict)
            or not all(isinstance(v, bool) for v in verdicts.values())
        ):
            raise refuse(path, None, "a gate reading without its verdicts")
        if event["event"] == "pause" and not (
            finite(event.get("seconds")) and event["seconds"] >= 0
        ):
            raise refuse(path, None, "a gate pause without its seconds")
        seconds = float(event["seconds"]) if event["event"] == "pause" else 0.0
        gate.append(
            GateEvent(
                GateKind(event["event"]),
                all(verdicts.values()) if verdicts else False,
                seconds,
            )
        )
    return Quota(
        buckets("quota-before"),
        buckets("quota-after"),
        tuple(gate),
        burn("burn-before"),
        burn("burn-after"),
    )


@dataclass(frozen=True)
class Condition:
    value: object
    layer: str


# What each declared setting `config show` reports may hold, by the end of its key.
CONDITION_VALUES: dict[str, Callable[[object], bool]] = {
    "plugin": lambda v: isinstance(v, str) and bool(re.fullmatch(r"[a-z-]+", v)),
    "visibility": lambda v: v in ("public", "private", "unknown"),
    "enabled": lambda v: isinstance(v, bool),
    "visibility_command": lambda v: v in ("set", "unset"),
    "check_command": lambda v: v in ("set", "unset"),
}
CONDITION_KEY = re.compile(
    r"sources\.[a-z-]+\.(plugin|visibility|cache\.enabled)|write_policy\.(visibility|check)_command"
)


def conditions_of(path: Path) -> dict[str, Condition]:
    raw = document(path)
    out = {}
    for key, v in raw.items():
        if not CONDITION_KEY.fullmatch(key):
            raise refuse(
                path, None, f"the condition {key} is not one this measurement declares"
            )
        if isinstance(v, dict) and not CONDITION_VALUES[key.rsplit(".", 1)[-1]](
            v.get("value")
        ):
            raise refuse(
                path, None, f"the condition {key} holds a value it cannot hold"
            )
        if (
            not isinstance(v, dict)
            or not text(v.get("layer"))
            or not isinstance(v.get("value"), (str, bool))
        ):
            raise refuse(path, None, f"the condition {key} without its value and layer")
        out[key] = Condition(v["value"], v["layer"])
    return out


def strings_of(path: Path) -> dict[str, str]:
    raw = document(path)
    if not all(text(v) for v in raw.values()):
        raise refuse(path, None, "a version that is not a string")
    return dict(raw)


@dataclass(frozen=True)
class CacheState:
    files: int
    bytes: int


def cache_of(path: Path) -> CacheState:
    raw = document(path)
    if not count(raw.get("files")) or not count(raw.get("bytes")):
        raise refuse(path, None, "a cache state without its files and bytes")
    return CacheState(raw["files"], raw["bytes"])


@dataclass(frozen=True)
class Size:
    """A rendered body's size."""

    bytes: int
    characters: int


def workload_of(path: Path) -> dict[str, Size]:
    raw = document(path)
    out = {}
    for label, sizes in raw.items():
        if (
            not isinstance(sizes, list)
            or len(sizes) != 2
            or not all(count(s) for s in sizes)
        ):
            raise refuse(
                path, None, f"the size of {label} is not its bytes and characters"
            )
        out[label] = Size(sizes[0], sizes[1])
    return out


@dataclass(frozen=True)
class Capture:
    """One row's sanitized capture, parsed."""

    name: str
    requests: tuple[Request, ...]
    steps: tuple[Step, ...]
    host: tuple[HostEvent, ...]
    screen: tuple[ScreenCall, ...]
    quota: Quota
    versions: dict[str, str]
    conditions: dict[str, Condition]
    cache_before: CacheState
    cache_after: CacheState
    workload: dict[str, Size]


def load(path: Path) -> Capture:
    return Capture(
        name=path.name,
        requests=tuple(requests_of(path / "requests.jsonl")),
        steps=tuple(steps_of(path / "steps.jsonl")),
        host=tuple(host_of(path / "host.jsonl")),
        screen=tuple(screen_of(path / "screen.jsonl")),
        quota=quota_of(path / "quota.json"),
        versions=strings_of(path / "versions.json"),
        conditions=conditions_of(path / "conditions.json"),
        cache_before=cache_of(path / "cache-before.json"),
        cache_after=cache_of(path / "cache-after.json"),
        workload=workload_of(path / "workload.json"),
    )


def latest_captures(captures: Path) -> dict[str, Path]:
    """The latest capture of every scenario: `<scenario>-<YYYYmmddTHHMMSS>` directories."""
    found: dict[str, Path] = {}
    for path in sorted(captures.iterdir()) if captures.exists() else []:
        match = re.fullmatch(r"(.+)-(\d{8}T\d{6})", path.name)
        if path.is_dir() and match:
            found[match.group(1)] = path
    return found


def every_capture(captures: Path) -> list[tuple[Path, bool]]:
    """Every capture directory, rows and superseded alike, in the order they were taken, each with
    whether it was superseded."""
    named = [
        (p, False)
        for p in captures.iterdir()
        if p.is_dir() and re.fullmatch(r".+-\d{8}T\d{6}", p.name)
    ]
    superseded = captures / "superseded"
    named += (
        [(p, True) for p in superseded.iterdir() if p.is_dir()]
        if superseded.exists()
        else []
    )
    return sorted(named, key=lambda entry: entry[0].name.rsplit("-", 1)[-1])


@dataclass
class Points:
    """A row's GraphQL primary points: GitHub's own price of every GraphQL request it sent, with
    `rateLimit` read around it and the idle background that reading carries beside it."""

    figure: int
    queries: int
    mutation_requests: int
    at_five: int
    counter: int | None = None
    before_used: int | None = None
    after_used: int | None = None
    delta: int | None = None
    background_rate: float | None = None
    background: float | None = None
    residual: float | None = None


def price(request: Request, prices: dict[str, int], capture: str) -> int:
    """One GraphQL request's primary points: a query its own dry-run price, a mutation request
    `MUTATION_PRICE` (which the dry run cannot price)."""
    if request.kind is RequestKind.MUTATION:
        return MUTATION_PRICE
    key = f"{capture}/{request.seq}"
    if key not in prices:
        raise SystemExit(
            f"analyze.py: no dry-run price for {key} ({request.op}); run harness/pricing.py"
        )
    return prices[key]


def counter_of(before: tuple[Reading, ...], after: tuple[Reading, ...]) -> int | None:
    """The counter (by its reset) both idle samples read, the later-resetting if two did."""
    shared = {r.reset for r in before} & {r.reset for r in after}
    return max(shared) if shared else None


def slope(readings: list[Reading]) -> tuple[float, float]:
    """The rise of `used` across one counter's readings, and the seconds it took."""
    if len(readings) < 2:
        return 0.0, 0.0
    return float(readings[-1].used - readings[0].used), readings[-1].at - readings[0].at


def points_of(capture: Capture, prices: dict[str, int]) -> Points:
    graphql = [r for r in capture.requests if r.graphql and not r.refused]
    figure = sum(price(r, prices, capture.name) for r in graphql)
    mutations = sum(1 for r in graphql if r.kind is RequestKind.MUTATION)
    points = Points(figure, len(graphql) - mutations, mutations, figure + mutations * 4)
    counter = counter_of(capture.quota.burn_before, capture.quota.burn_after)
    if counter is None:
        return points
    b = [r for r in capture.quota.burn_before if r.reset == counter]
    a = [r for r in capture.quota.burn_after if r.reset == counter]
    rise_b, time_b = slope(b)
    rise_a, time_a = slope(a)
    rate = (rise_b + rise_a) / (time_b + time_a) if time_b + time_a else 0.0
    delta = a[0].used - b[-1].used
    background = rate * (a[0].at - b[-1].at)
    points.counter, points.before_used, points.after_used, points.delta = (
        counter,
        b[-1].used,
        a[0].used,
        delta,
    )
    points.background_rate = round(rate, 3)
    points.background = round(background, 1)
    points.residual = round(delta - figure - background, 1)
    return points


@dataclass(frozen=True)
class Writes:
    """Individual mutations (each aliased one counted) plus uploads, and the requests carrying them."""

    requests: int
    issues_created: int
    mutation_requests: int
    mutations: int
    uploads: int
    writes: int


def writes_of(requests: tuple[Request, ...]) -> Writes:
    sent = [r for r in requests if not r.refused]
    mutations = [r for r in sent if r.kind is RequestKind.MUTATION]
    uploads = sum(1 for r in sent if r.op == UPLOAD)
    carried = sum(r.carried for r in mutations)
    return Writes(
        requests=sum(1 for r in sent if r.kind is not RequestKind.TUNNEL),
        issues_created=sum(r.carried for r in mutations if r.op in CREATES),
        mutation_requests=len(mutations),
        mutations=carried,
        uploads=uploads,
        writes=carried + uploads,
    )


@dataclass
class KindFigures:
    """How many requests of one kind, and the points and writes they carried."""

    requests: int = 0
    points: int = 0
    writes: int = 0

    def add(self, request: Request, points: int) -> None:
        self.requests += 1
        self.points += points
        self.writes += (
            request.carried
            if request.kind is RequestKind.MUTATION
            else int(request.op == UPLOAD)
        )


def kinds_of(capture: Capture, prices: dict[str, int]) -> dict[str, KindFigures]:
    """Every request kind a row sent."""
    out: dict[str, KindFigures] = defaultdict(KindFigures)
    for r in capture.requests:
        if r.kind is not RequestKind.TUNNEL and not r.refused:
            out[r.op].add(r, price(r, prices, capture.name) if r.graphql else 0)
    return dict(sorted(out.items()))


@dataclass(frozen=True)
class Screening:
    inspect: int
    check: int
    seconds: float


def screening_of(screen: tuple[ScreenCall, ...]) -> Screening:
    counts = Counter(r.name for r in screen)
    return Screening(
        counts.get("inspect", 0),
        counts.get("check", 0),
        round(sum(r.end - r.start for r in screen), 3),
    )


# Causes in the order an instant is given to them when several are under way at once: a
# screening call runs inside a store invocation, and so does every round trip.
PRIORITY = (
    "write screening",
    "network",
    "store",
    "subprocess",
    "step",
    "python",
    "launch",
)
RANK = {name: index for index, name in enumerate(PRIORITY)}


@dataclass(frozen=True)
class Interval:
    start: float
    end: float
    priority: int
    label: str


def store_label(argv: tuple[str, ...]) -> str:
    named = [
        w
        for w in argv
        if not w.startswith("<") and not w.startswith("--") and ":" not in w
    ]
    return "store invocation: " + " ".join(named[:2] or ["(unnamed)"])


HOST_CAUSES: dict[HostKind, tuple[str, Callable[[HostEvent], str]]] = {
    HostKind.STORE: ("store", lambda h: store_label(h.argv)),
    HostKind.SUBPROCESS: ("subprocess", lambda h: f"host subprocess: {h.program}"),
    HostKind.STEP: ("step", lambda h: f"host step: {h.name}"),
}


def network_label(request: Request) -> str:
    if request.graphql:
        return "network round trips: GitHub GraphQL"
    if request.kind is RequestKind.TUNNEL:
        return "network round trips: tunnelled HTTPS (the image upload's storage and its check)"
    return "network round trips: GitHub REST and uploads"


def intervals_of(capture: Capture) -> tuple[list[Interval], float, float]:
    """Every cause's interval in a row, with the row's measured start and end."""
    start, end = min(s.start for s in capture.steps), max(s.end for s in capture.steps)
    out = [
        Interval(
            r.start,
            r.end,
            RANK["write screening"],
            f"write screening: onevcs boundary {r.name}",
        )
        for r in capture.screen
    ]
    out += [
        Interval(r.start, r.end, RANK["network"], network_label(r))
        for r in capture.requests
        if not r.refused
    ]
    if not capture.host:
        out += [
            Interval(
                s.start,
                s.end,
                RANK["store"],
                "store invocation: " + " ".join(s.argv[:2]),
            )
            for s in capture.steps
        ]
        return out, start, end
    first = min(
        (
            h.start
            for h in capture.host
            if h.kind is HostKind.INTERPRETER and h.name == "start"
        ),
        default=start,
    )
    last = max(
        (
            h.start
            for h in capture.host
            if h.kind is HostKind.INTERPRETER and h.name == "exit"
        ),
        default=end,
    )
    first_event = min(
        (h.start for h in capture.host if h.kind in HOST_CAUSES), default=first
    )
    out += [
        Interval(
            start,
            first,
            RANK["launch"],
            "recipe launch: just, plan-store.sh and uv run",
        ),
        Interval(first, first_event, RANK["launch"], "interpreter and SDK start-up"),
        Interval(
            first_event, last, RANK["python"], "host Python outside the named steps"
        ),
        Interval(last, end, RANK["launch"], "interpreter exit and recipe teardown"),
    ]
    for h in capture.host:
        if h.kind in HOST_CAUSES:
            cause, label = HOST_CAUSES[h.kind]
            out.append(Interval(h.start, h.end, RANK[cause], label(h)))
    return out, start, end


def attribute(intervals: list[Interval], start: float, end: float) -> dict[str, float]:
    """Split [start, end] among the causes: each instant goes to the highest-priority cause under
    way then, the innermost (latest-starting) of equal ones; an instant none covers is
    `unattributed`. The shares therefore sum to the measured total exactly."""
    points = sorted(
        {
            start,
            end,
            *(t for i in intervals for t in (i.start, i.end) if start < t < end),
        }
    )
    shares: dict[str, float] = defaultdict(float)
    for a, b in zip(points, points[1:]):
        middle = (a + b) / 2
        live = [i for i in intervals if i.start <= middle < i.end]
        if live:
            shares[min(live, key=lambda i: (i.priority, -i.start)).label] += b - a
        else:
            shares["unattributed"] += b - a
    return dict(shares)


def rounded(
    shares: dict[str, float], total: float, places: int = 2
) -> dict[str, float]:
    """`shares` rounded so that they still sum to `total` rounded the same way (largest
    remainder), which is how a table of them stays a true decomposition of its total."""
    unit = 10**places
    target = round(total * unit)
    floors = {k: int(v * unit) for k, v in shares.items()}
    short = target - sum(floors.values())
    order = sorted(shares, key=lambda k: shares[k] * unit - floors[k], reverse=True)
    for key in order[: max(short, 0)]:
        floors[key] += 1
    # Causes that round to nothing are named together rather than listed at 0.00 s each.
    small = sorted(k for k, v in floors.items() if v == 0)
    out = {
        k: v / unit
        for k, v in sorted(floors.items(), key=lambda kv: (-kv[1], kv[0]))
        if v
    }
    if small:
        out[f"{len(small)} further causes under 0.005 s each ({', '.join(small)})"] = (
            0.0
        )
    return out


@dataclass(frozen=True, order=True)
class Sender:
    """Who sent a request: the store or the host, the store invocation it was sent inside, and
    its request kind."""

    owner: str
    invocation: str
    operation: str

    def __str__(self) -> str:
        return f"{self.owner} | {self.invocation} | {self.operation}"


def owners_of(capture: Capture, prices: dict[str, int]) -> dict[Sender, KindFigures]:
    """Each request kind's points and writes by who sent it: a request made inside the store's
    own `project copy` or `document copy` is the store's; one inside any other store invocation
    the host recipe made is the host's (it chose to make that call)."""
    calls = [h for h in capture.host if h.kind is HostKind.STORE] or [
        HostEvent(HostKind.STORE, s.start, s.end, s.argv) for s in capture.steps
    ]
    out: dict[Sender, KindFigures] = defaultdict(KindFigures)
    for r in capture.requests:
        if r.kind is RequestKind.TUNNEL or r.refused:
            continue
        inside = [c for c in calls if c.start <= r.start <= c.end]
        verb = (
            store_label(inside[-1].argv).removeprefix("store invocation: ")
            if inside
            else "(no store invocation)"
        )
        owner = "store" if tuple(verb.split()[:2]) in COPY_VERBS else "host"
        out[Sender(owner, verb, r.op)].add(
            r, price(r, prices, capture.name) if r.graphql else 0
        )
    return dict(sorted(out.items()))


@dataclass
class Row:
    scenario: Scenario
    capture: Capture
    seconds: float
    points: Points
    writes: Writes
    kinds: dict[str, KindFigures]
    screening: Screening
    attribution: dict[str, float]
    owners: dict[Sender, KindFigures]
    walk_pages: dict[str, int]
    walk_ms: float
    verdicts: dict[str, Verdict] = field(default_factory=dict)

    @property
    def label(self) -> str:
        return self.scenario.label

    @property
    def target(self) -> Figures:
        return TARGETS[self.scenario.operation]


def verdict(figure: float | None, bar: float | None) -> Verdict:
    return (
        Verdict.WITHIN
        if figure is not None and bar is not None and figure <= bar
        else Verdict.OVER
    )


def row_of(scenario: Scenario, path: Path, prices: dict[str, int]) -> Row:
    capture = load(path)
    seconds = round(sum(s.end - s.start for s in capture.steps), 3)
    intervals, start, end = intervals_of(capture)
    shares = attribute(intervals, start, end)
    # A bare row's two commands are timed back to back; the gap between them is the harness's.
    gaps = sum(b.start - a.end for a, b in zip(capture.steps, capture.steps[1:]))
    if gaps:
        shares["unattributed"] = shares.get("unattributed", 0.0) - gaps
        if abs(shares["unattributed"]) < 1e-6:
            del shares["unattributed"]
    walk = Counter(r.op for r in capture.requests if r.op in WALK_PAGE_SECONDS)
    row = Row(
        scenario=scenario,
        capture=capture,
        seconds=seconds,
        points=points_of(capture, prices),
        writes=writes_of(capture.requests),
        kinds=kinds_of(capture, prices),
        screening=screening_of(capture.screen),
        attribution=rounded(shares, seconds),
        owners=owners_of(capture, prices),
        walk_pages={op: walk.get(op, 0) for op in WALK_PAGE_SECONDS},
        walk_ms=sum(r.ms for r in capture.requests if r.op in WALK_PAGE_SECONDS),
    )
    measured = Figures(
        seconds=row.seconds, points=row.points.figure, writes=row.writes.writes
    )
    row.verdicts = {
        name: verdict(measured.of(name), row.target.of(name))
        for name in ("seconds", "points", "writes")
        if row.target.of(name) is not None
    }
    row.verdicts["points at 5 per mutation"] = verdict(
        row.points.at_five, row.target.points
    )
    return row


def reference_seconds(row: Row) -> float:
    """The row's seconds at the `plans` board's size: every walk page the test board answered
    replaced by the pages the same walk takes there. With no walk page it is the measured figure."""
    if not any(row.walk_pages.values()):
        return row.seconds
    extra = sum(
        WALK_PAGES_AT_REFERENCE[op] * WALK_PAGE_SECONDS[op]
        for op, n in row.walk_pages.items()
        if n
    )
    return round(row.seconds - row.walk_ms / 1000 + extra, 1)


@dataclass(frozen=True)
class Recording:
    """One configuration of one loopback budget recording: what it sent, by operation."""

    sent: dict[str, int]
    points: int | None


@dataclass(frozen=True)
class Loopback:
    """One loopback telemetry recording of the adopted release, by configuration."""

    budget: str
    recordings: dict[str, Recording]


def loopback(captures: Path) -> list[Loopback]:
    out = []
    for path in sorted((captures / "loopback").glob("*.json")):
        record = document(path)
        detail = record.get("detail")
        if isinstance(detail, str):
            try:
                detail = json.loads(detail)
            except ValueError as error:
                raise refuse(
                    path, None, f"a detail that is not JSON ({error})"
                ) from error
        if not text(record.get("budget")) or not isinstance(detail, dict):
            raise refuse(
                path, None, "a loopback recording without its budget or its detail"
            )
        recordings = {}
        for name, value in detail.items():
            if not isinstance(value, dict) or "requests by operation" not in value:
                continue
            sent, points = value["requests by operation"], value.get("points")
            if not isinstance(sent, dict) or not all(
                text(k) and count(v) for k, v in sent.items()
            ):
                raise refuse(
                    path, None, f"{name}: requests by operation that are not counts"
                )
            if points is not None and not count(points):
                raise refuse(path, None, f"{name}: points that are not a whole number")
            recordings[name] = Recording(dict(sent), points)
        out.append(Loopback(record["budget"], recordings))
    return out


SIZES = (
    ("unscreened", "unscreened, on a board of 20,000 items"),
    ("screened", "screened, on a board of 20,000 items"),
    (
        "host configuration, private source",
        "host configuration, private source, on a board of 20,000 items",
    ),
    (
        "host configuration, public source",
        "host configuration, public source, on a board of 20,000 items",
    ),
)
# What the plugin calls a page of a whole-board walk or of an issue search, in the loopback
# recordings' operation names.
WALK_WORDS = ("page of the board", "board's items", "search")


@dataclass(frozen=True)
class SizePair:
    """One loopback recording on the board of 2,000 items beside its twin on 20,000."""

    budget: str
    recording: str
    same: bool  # the same requests by operation at both sizes
    requests: tuple[int, int]
    points: tuple[int | None, int | None]
    walk_pages: tuple[int, int]


def walk_pages(sent: dict[str, int]) -> int:
    return sum(n for op, n in sent.items() if any(w in op for w in WALK_WORDS))


def size_pairs(recordings: list[Loopback]) -> list[SizePair]:
    """Every loopback points recording on the board of 2,000 items beside its twin on 20,000:
    whether the two sent the same requests by operation, their points, and the walk pages."""
    pairs = []
    for record in recordings:
        if not record.budget.startswith("github-plan-copy-points-"):
            continue
        for small, large in SIZES:
            a, b = record.recordings.get(small), record.recordings.get(large)
            if a is None or b is None:
                continue
            pairs.append(
                SizePair(
                    budget=record.budget,
                    recording=small,
                    same=a.sent == b.sent,
                    requests=(sum(a.sent.values()), sum(b.sent.values())),
                    points=(a.points, b.points),
                    walk_pages=(walk_pages(a.sent), walk_pages(b.sent)),
                )
            )
    return pairs


def fmt(value: object) -> str:
    return f"{value:g}" if isinstance(value, (int, float)) else str(value)


def table(head: list[str], rows: list[list[str]]) -> str:
    out = [
        "| " + " | ".join(head) + " |",
        "| " + " | ".join("---" for _ in head) + " |",
    ]
    out += ["| " + " | ".join(r) + " |" for r in rows]
    return "\n".join(out)


def targets_table(rows: list[Row]) -> str:
    body = []
    for row in rows:
        before = BEFORE.get(row.scenario.operation, Figures())
        earlier = EARLIER[row.scenario.name]
        values = {
            "seconds": f"{row.seconds:.1f}",
            "points": str(row.points.figure),
            "writes": str(row.writes.writes),
        }
        cells = [row.label]
        for name in ("seconds", "points", "writes"):
            bar = row.target.of(name)
            mark = (
                f"target {fmt(bar)}: **{row.verdicts[name].value}**"
                if bar is not None
                else "no target"
            )
            was = (
                f"{fmt(before.of(name))} (loopback, 0.12.3)"
                if before.of(name) is not None
                else "none in the brief"
            )
            cells.append(
                f"**{values[name]}** — {mark}; before {was}; GitHub at 0.11.0 {fmt(earlier.of(name))}"
            )
        flips = (
            row.verdicts["points at 5 per mutation"] is Verdict.OVER
            and row.verdicts["points"] is Verdict.WITHIN
        )
        cells.append(
            f"{row.points.at_five} — {'**would flip to over**' if flips else 'no flip'}"
        )
        body.append(cells)
    head = [
        "row",
        "elapsed seconds",
        "GraphQL points",
        "writes (mutations + uploads)",
        "points if each mutation request cost 5",
    ]
    return table(head, body)


def detail_table(rows: list[Row]) -> str:
    body = []
    for row in rows:
        w, s, p = row.writes, row.screening, row.points
        kinds = ", ".join(
            f"`{op}` {k.requests}×/{k.points} pt"
            + (f"/{k.writes} w" if k.writes else "")
            for op, k in row.kinds.items()
        )
        body.append(
            [
                row.label,
                f"`{row.capture.name}`",
                str(w.requests),
                kinds,
                f"{p.queries} queries at their dry-run price + {p.mutation_requests} mutation requests at {MUTATION_PRICE} = {p.figure}",
                f"{w.mutation_requests} requests carrying {w.mutations} mutations + {w.uploads} upload(s) = {w.writes}; {w.issues_created} issues created",
                f"{s.inspect} inspect + {s.check} check ({s.seconds:.2f} s)",
                f"{row.capture.cache_before.files} → {row.capture.cache_after.files} files",
            ]
        )
    head = [
        "row",
        "capture",
        "requests",
        "request kinds (requests/points/writes)",
        "points",
        "writes",
        "screening calls",
        "store cache",
    ]
    return table(head, body)


def seconds_table(rows: list[Row]) -> str:
    body = [
        [
            row.label,
            f"{row.seconds:.2f} s",
            "; ".join(
                f"{label} {value:.2f} s" for label, value in row.attribution.items()
            ),
            f"{sum(row.attribution.values()):.2f} s",
        ]
        for row in rows
    ]
    return table(["row", "measured", "attributed to", "sum of shares"], body)


def owners_table(rows: list[Row]) -> str:
    body = []
    for row in rows:
        for key, value in row.owners.items():
            owner, verb, op = key.owner, key.invocation, key.operation
            body.append(
                [
                    row.label,
                    owner,
                    verb,
                    f"`{op}`",
                    str(value.requests),
                    str(value.points),
                    str(value.writes),
                ]
            )
    return table(
        ["row", "owner", "sent inside", "request kind", "requests", "points", "writes"],
        body,
    )


def over_table(rows: list[Row]) -> str:
    over = [
        f"{row.label}: {name}"
        for row in rows
        for name in ("points", "writes")
        if row.verdicts.get(name) is Verdict.OVER
    ]
    return (
        "Over its target: " + "; ".join(over) + "."
        if over
        else "No points or writes figure is over its target."
    )


def burn_table(rows: list[Row]) -> str:
    body = []
    for row in rows:
        pt = row.points
        if pt.counter is None:
            body.append(
                [
                    row.label,
                    "no counter read on both sides",
                    "—",
                    "—",
                    "—",
                    "—",
                    str(pt.figure),
                ]
            )
            continue
        body.append(
            [
                row.label,
                f"{pt.before_used} → {pt.after_used} (counter resetting {pt.counter})",
                str(pt.delta),
                f"{pt.background_rate:.2f} points/s",
                f"{pt.background:.1f}",
                str(pt.figure),
                f"{pt.residual:+.1f}",
            ]
        )
    head = [
        "row",
        "GraphQL `rateLimit` used, last idle read before → first after",
        "rise",
        "idle background burn bracketing the row",
        "background expected over the row",
        "the row's own priced points",
        "residual (rise − own − background)",
    ]
    return table(head, body)


def quota_table(rows: list[Row]) -> str:
    body = []
    for row in rows:
        q = row.capture.quota
        readings = [e for e in q.gate if e.event is GateKind.READING]
        pauses = [e for e in q.gate if e.event is GateKind.PAUSE]
        least = q.before["graphql"]
        body.append(
            [
                row.label,
                f"{least.remaining} / {least.limit} ({least.remaining / least.limit:.0%})",
                f"remaining {', '.join(map(str, q.before['graphql (rateLimit)'].readings))}",
                f"remaining {', '.join(map(str, q.after['graphql (rateLimit)'].readings))}",
                f"{q.before['graphql (GET /rate_limit)'].remaining} → {q.after['graphql (GET /rate_limit)'].remaining}",
                f"{q.before['core'].remaining} → {q.after['core'].remaining}",
                "proceed (≥ 40%)" if readings and readings[-1].proceed else "paused",
                f"{len(pauses)} pause(s), {sum(p.seconds for p in pauses):.0f} s"
                if pauses
                else "none",
            ]
        )
    head = [
        "row",
        "gate figure (least reading)",
        "GraphQL `rateLimit` before (three reads)",
        "GraphQL `rateLimit` after (three reads)",
        "`GET /rate_limit` graphql before → after",
        "REST core before → after",
        "gate",
        "pauses",
    ]
    return table(head, body)


def declared_value(
    conditions: dict[str, Condition], key: str, default: object
) -> object:
    return conditions[key].value if key in conditions else default


def conditions_table(rows: list[Row]) -> str:
    body = []
    for row in rows:
        c = row.capture.conditions
        sources = sorted({k.split(".")[1] for k in c if k.startswith("sources.")})
        declared = "; ".join(
            f"`{s}` {declared_value(c, f'sources.{s}.plugin', '?')}, {declared_value(c, f'sources.{s}.visibility', 'undeclared')}"
            + (
                ", cache on"
                if declared_value(c, f"sources.{s}.cache.enabled", False)
                else ""
            )
            for s in sources
        )
        policy = ", ".join(
            f"`{k.split('.')[1]}` {v.value} ({v.layer})"
            for k, v in sorted(c.items())
            if k.startswith("write_policy.")
        )
        v = row.capture.versions
        versions = f"{v.get('store')}; {v.get('engine')}; {v.get('sdk')}; {v.get('onevcs')}; host `{v.get('host_commit')}`; harness `{v.get('harness_commit')}`"
        body.append(
            [
                row.label,
                declared,
                policy,
                ", ".join(sorted({x.layer for x in c.values()})),
                versions,
            ]
        )
    head = [
        "row",
        "sources (plugin, visibility, cache)",
        "write_policy",
        "configuration layers present",
        "versions",
    ]
    return table(head, body)


def workload_table(rows: list[Row]) -> str:
    body = []
    for row in rows:
        w = row.capture.workload
        tasks = [w[k].bytes for k in sorted(w) if k.startswith("task")]
        body.append(
            [
                row.label,
                f"{w.get('project', Size(0, 0)).bytes:,}",
                f"{min(tasks):,}–{max(tasks):,} ({len(tasks)})" if tasks else "—",
                f"{w.get('document', Size(0, 0)).bytes:,}",
                f"{w.get('image', Size(0, 0)).bytes:,}",
            ]
        )
    return table(
        ["row", "project bytes", "task bytes (tasks)", "document bytes", "PNG bytes"],
        body,
    )


def size_table(pairs: list[SizePair]) -> str:
    body = [
        [
            f"`{p.budget}`",
            p.recording,
            f"{p.requests[0]} / {p.requests[1]}",
            "same" if p.same else "**differ**",
            f"{fmt(p.points[0])} / {fmt(p.points[1])}",
            f"{p.walk_pages[0]} / {p.walk_pages[1]}",
        ]
        for p in pairs
    ]
    head = [
        "loopback recording",
        "configuration",
        "requests at 2,000 / 20,000 items",
        "requests by operation",
        "points at 2,000 / 20,000",
        "walk pages at 2,000 / 20,000",
    ]
    return table(head, body)


def reference_table(rows: list[Row]) -> str:
    body = []
    for row in rows:
        pages = row.walk_pages
        arithmetic = (
            f"{row.seconds:.1f} s measured − 0 s of 0 walk pages + 0 × 6.95 s + 0 × 6.79 s = **{reference_seconds(row):.1f} s**"
            if not any(pages.values())
            else f"**{reference_seconds(row):.1f} s**"
        )
        body.append(
            [
                row.label,
                f"{pages['BOARD']} BOARD + {pages['SEARCH_ISSUES']} SEARCH_ISSUES",
                arithmetic,
                str(row.points.figure),
            ]
        )
    return table(
        [
            "row",
            "walk pages sent",
            "seconds at the `plans` board's size",
            "points at that size",
        ],
        body,
    )


@dataclass(frozen=True)
class Cleanup:
    deleted: int
    only: str


def cleanup_of(path: Path) -> Cleanup | None:
    if not path.exists():
        return None
    raw = document(path)
    if not count(raw.get("deleted")) or not text(raw.get("only", "all")):
        raise refuse(
            path, None, "a cleanup outcome without its deleted count and its scope"
        )
    return Cleanup(raw["deleted"], raw.get("only", "all"))


def writes_table(captures: Path) -> str:
    body = []
    for path, superseded in every_capture(captures):
        requests = tuple(requests_of(path / "requests.jsonl"))
        w = writes_of(requests)
        cleanup = cleanup_of(path / "cleanup.json")
        body.append(
            [
                f"`{path.name}`" + (" (superseded)" if superseded else ""),
                str(w.mutation_requests),
                str(w.mutations),
                str(w.uploads),
                str(w.issues_created),
                f"{cleanup.deleted} ({cleanup.only})" if cleanup else "—",
                str(sum(1 for r in requests if r.refused)),
            ]
        )
    head = [
        "capture",
        "mutation requests",
        "mutations",
        "uploads",
        "issues created",
        "issues deleted by cleanup",
        "requests the guard refused",
    ]
    return table(head, body)


def probes_table(captures: Path) -> str:
    """The two hand probes: the idle background on each counter, and what onevcs's inspect spends."""
    path = captures / "probes" / "background-and-onevcs.json"
    if not path.exists():
        return "No probe was recorded."
    probe = document(path)
    background, inspect = probe.get("idle_background"), probe.get("onevcs_inspect")
    raw = background.get("readings") if isinstance(background, dict) else None
    if (
        not isinstance(raw, list)
        or not raw
        or not all(
            isinstance(r, dict)
            and finite(r.get("at"))
            and count(r.get("used"))
            and count(r.get("reset"))
            for r in raw
        )
    ):
        raise refuse(
            path, None, "an idle-background probe without timed readings on a counter"
        )
    readings = [Reading(float(r["at"]), r["used"], r["reset"]) for r in raw]

    def sides(name: str) -> list[dict]:
        value = inspect.get(name) if isinstance(inspect, dict) else None
        if (
            not isinstance(value, list)
            or not value
            or not all(
                isinstance(r, dict)
                and count(r.get("core_used"))
                and count(r.get("graphql_used"))
                for r in value
            )
        ):
            raise refuse(
                path,
                None,
                f"an onevcs probe without its `{name}` readings of both buckets",
            )
        return value

    before, after = sides("before"), sides("after")
    inspects = inspect.get("inspects")
    if not count(inspects) or not inspects:
        raise refuse(path, None, "an onevcs probe without a positive count of inspects")
    body = []
    for reset in sorted({r.reset for r in readings}):
        on = [r for r in readings if r.reset == reset]
        rise, seconds = slope(on)
        rate = f"{rise / seconds:.2f} points/s" if seconds else "—"
        body.append(
            [
                f"idle background, counter resetting {reset}",
                f"{len(on)} reads over {seconds:.1f} s",
                f"{rise:g} points",
                rate,
            ]
        )
    core = after[0]["core_used"] - before[-1]["core_used"]
    graphql = after[0]["graphql_used"] - before[-1]["graphql_used"]
    body.append(
        [
            f"{inspects} × `onevcs boundary inspect`",
            "`GET /rate_limit` last read before → first after",
            f"REST core {core:+d}, GraphQL {graphql:+d}",
            f"{core / inspects:g} REST request and {graphql / inspects:g} GraphQL points per inspect",
        ]
    )
    return table(["probe", "readings", "rise", "rate"], body)


SPEND = (
    "queries",
    "mutation_requests",
    "rest",
    "tunnels",
    "screening",
    "rate_limit_reads",
    "rest_rate_limit_reads",
)


def spend_table(captures: Path, priced: int) -> str:
    """What every capture sent, and the quota reads around it, with the total."""
    body, totals = [], Counter()
    for path, _ in every_capture(captures):
        requests = [r for r in requests_of(path / "requests.jsonl") if not r.refused]
        q = quota_of(path / "quota.json")
        figures = Counter(
            queries=sum(1 for r in requests if r.kind is RequestKind.QUERY),
            mutation_requests=sum(
                1 for r in requests if r.kind is RequestKind.MUTATION
            ),
            rest=sum(1 for r in requests if r.kind is RequestKind.REST),
            tunnels=sum(1 for r in requests if r.kind is RequestKind.TUNNEL),
            screening=len(screen_of(path / "screen.jsonl")),
            rate_limit_reads=len(q.before["graphql (rateLimit)"].readings)
            + len(q.after["graphql (rateLimit)"].readings)
            + len(q.burn_before)
            + len(q.burn_after),
            rest_rate_limit_reads=2,
        )
        totals += figures
        body.append([f"`{path.name}`"] + [str(figures[k]) for k in SPEND])
    body.append(["**total**"] + [f"**{totals[k]}**" for k in SPEND])
    head = [
        "capture",
        "GraphQL queries",
        "mutation requests",
        "REST requests",
        "tunnels (onevcs and the upload's storage)",
        "onevcs calls",
        "GraphQL `rateLimit` reads (free)",
        "`GET /rate_limit` reads (free)",
    ]
    note = f"Beside these, `harness/pricing.py` sent {priced} dry-run pricings, one per GraphQL query of every run it read."
    return table(head, body) + "\n\n" + note


TABLES: dict[str, Callable[[list[Row], list[SizePair], Path, int], str]] = {
    "targets": lambda rows, pairs, captures, priced: targets_table(rows),
    "over": lambda rows, pairs, captures, priced: over_table(rows),
    "detail": lambda rows, pairs, captures, priced: detail_table(rows),
    "owners": lambda rows, pairs, captures, priced: owners_table(rows),
    "seconds": lambda rows, pairs, captures, priced: seconds_table(rows),
    "burn": lambda rows, pairs, captures, priced: burn_table(rows),
    "quota": lambda rows, pairs, captures, priced: quota_table(rows),
    "conditions": lambda rows, pairs, captures, priced: conditions_table(rows),
    "workload": lambda rows, pairs, captures, priced: workload_table(rows),
    "board-size": lambda rows, pairs, captures, priced: size_table(pairs),
    "reference-size": lambda rows, pairs, captures, priced: reference_table(rows),
    "writes": lambda rows, pairs, captures, priced: writes_table(captures),
    "spend": lambda rows, pairs, captures, priced: spend_table(captures, priced),
    "probes": lambda rows, pairs, captures, priced: probes_table(captures),
}


def tables(captures: Path) -> tuple[dict[str, str], dict]:
    prices, rises = prices_of(captures / "pricing.json")
    latest = latest_captures(captures)
    missing = [s.name for s in ROWS if s.name not in latest]
    if missing:
        raise SystemExit(
            f"analyze.py: no capture of {', '.join(missing)} under {captures}; take them with measure.sh"
        )
    rows = [row_of(s, latest[s.name], prices) for s in ROWS]
    pairs = size_pairs(loopback(captures))
    out = {
        name: make(rows, pairs, captures, len(prices)) for name, make in TABLES.items()
    }
    summary = {
        "rows": [
            {
                "scenario": r.scenario.name,
                "capture": r.capture.name,
                "seconds": r.seconds,
                "points": asdict(r.points),
                "writes": asdict(r.writes),
                "kinds": {op: asdict(k) for op, k in r.kinds.items()},
                "screening": asdict(r.screening),
                "attribution": r.attribution,
                "owners": {str(key): asdict(k) for key, k in r.owners.items()},
                "verdicts": {name: v.value for name, v in r.verdicts.items()},
                "walk_pages": r.walk_pages,
                "refused": sum(1 for q in r.capture.requests if q.refused),
                "returncodes": [s.returncode for s in r.capture.steps],
            }
            for r in rows
        ],
        "board_size_pairs": [asdict(p) for p in pairs],
        "mutation_rises": {k: v for k, v in rises.items() if v},
    }
    return out, summary


BLOCK = re.compile(
    r"<!-- analyze:begin ([a-z-]+) -->\n(.*?)\n?<!-- analyze:end \1 -->", re.DOTALL
)


def reconcile(report: str, computed: dict[str, str]) -> tuple[str, list[str]]:
    """The report with every block replaced by its recomputed table, and the names of the blocks
    that differed (or that the report lacks)."""
    seen: set[str] = set()
    drifted: list[str] = []

    def replace(match: re.Match) -> str:
        name = match.group(1)
        seen.add(name)
        if name not in computed:
            drifted.append(f"{name} (no such table)")
            return match.group(0)
        if match.group(2).strip() != computed[name].strip():
            drifted.append(name)
        return f"<!-- analyze:begin {name} -->\n{computed[name]}\n<!-- analyze:end {name} -->"

    updated = BLOCK.sub(replace, report)
    drifted += [
        f"{name} (missing from the report)" for name in computed if name not in seen
    ]
    return updated, drifted


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--write", action="store_true", help="rewrite the report's tables"
    )
    parser.add_argument("--captures", type=Path, default=CAPTURES)
    parser.add_argument("--report", type=Path, default=REPORT)
    args = parser.parse_args(argv)
    computed, summary = tables(args.captures)
    (args.captures / "summary.json").write_text(
        json.dumps(summary, indent=1, sort_keys=True) + "\n"
    )
    updated, drifted = reconcile(args.report.read_text(), computed)
    if args.write:
        args.report.write_text(updated)
        unresolved = [name for name in drifted if name.endswith(")")]
        if unresolved:
            print(
                f"analyze.py: {args.report.name} has no place for, or an unknown, table: "
                f"{', '.join(unresolved)}; add or rename its analyze:begin/end markers",
                file=sys.stderr,
            )
            return 1
        return 0
    if drifted:
        print(
            f"analyze.py: {args.report.name} differs from its captures in: {', '.join(drifted)}; "
            "run with --write to put the recomputed tables in place",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
