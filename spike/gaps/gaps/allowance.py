"""The GraphQL allowance, read before and after every live probe, and the 40% gate.

A reading is the least of four: `GET /rate_limit`'s GraphQL bucket, three `rateLimit`
queries (GitHub has answered one bucket from two counters on consecutive requests), and the
`x-ratelimit-*` headers of the last answer any earlier run recorded in the same window. The
rule deciding whether to pause is the timing spike's own (`spike/harness/quota.py`
`verdict`): this module adapts its readings to that function rather than restating the rule.
"""

from __future__ import annotations

import asyncio
import datetime
import json
import time
from collections.abc import Awaitable, Callable
from dataclasses import asdict, dataclass
from pathlib import Path

from gaps.github import Client, RateLimit, require
from gaps.harness import quota
from gaps.json_shape import (
    Json,
    ShapeError,
    arr,
    encode,
    field,
    integer,
    load_lines,
    number,
    obj,
    text,
)

RATE_QUERY = "query SpikeRateLimit { rateLimit { limit used remaining resetAt } }"
MIN_PAUSE = 30.0


@dataclass(frozen=True)
class Bucket:
    """One reading of the GraphQL bucket, refused unless it is a possible allowance."""

    limit: int
    used: int
    remaining: int
    reset: int

    def __post_init__(self) -> None:
        """Refuse a bucket no allowance could be: the 40% rule divides by its limit."""
        if self.limit <= 0 or self.used < 0 or not 0 <= self.remaining <= self.limit:
            raise ShapeError(f"an impossible GraphQL bucket: {self}")


@dataclass(frozen=True)
class Reading:
    """Every figure one reading took, and the least of them, which the gate decides on."""

    at: float
    rest_endpoint: Bucket
    own: tuple[Bucket, ...]
    last_headers: Bucket | None
    # Which fields GitHub's own `rateLimit` answered: the evidence the schema extension the
    # spike validates its documents against is held to.
    answered_fields: tuple[str, ...] = ()

    @property
    def least(self) -> Bucket:
        """The least generous figure: a reading can only ever be too generous."""
        figures = [self.rest_endpoint, *self.own]
        if self.last_headers is not None and self.last_headers.reset > self.at:
            figures.append(self.last_headers)
        return min(figures, key=lambda bucket: bucket.remaining)

    def to_json(self) -> Json:
        """The reading as one log line, the figure it decides on stated beside it."""
        return obj(encode(self), "reading") | {"least": encode(self.least)}


def bucket_of(value: Json, where: str) -> Bucket:
    """A logged bucket, validated."""
    entry = obj(value, where)
    return Bucket(
        limit=integer(field(entry, "limit", where), where),
        used=integer(field(entry, "used", where), where),
        remaining=integer(field(entry, "remaining", where), where),
        reset=integer(field(entry, "reset", where), where),
    )


async def read(client: Client, last_headers: RateLimit | None) -> Reading:
    """Take one reading through the proxy."""
    at = time.time()
    answer = await client.rest_get("GET /rate_limit", "/rate_limit")
    resources = obj(answer.get("resources"), "rate_limit.resources")
    rest = bucket_of(resources.get("graphql"), "rate_limit.resources.graphql")
    own = []
    answered: set[str] = set()
    for _ in range(quota.OWN_READS):
        data = require(await client.graphql("rateLimit", RATE_QUERY, {}), "rateLimit")
        rate = obj(data.get("rateLimit"), "rateLimit")
        answered |= set(rate)
        reset_at = text(rate.get("resetAt"), "rateLimit.resetAt").replace("Z", "+00:00")
        own.append(
            Bucket(
                limit=integer(rate.get("limit"), "rateLimit.limit"),
                used=integer(rate.get("used"), "rateLimit.used"),
                remaining=integer(rate.get("remaining"), "rateLimit.remaining"),
                reset=int(datetime.datetime.fromisoformat(reset_at).timestamp()),
            )
        )
    headers = None
    if last_headers is not None and last_headers.resource in ("graphql", ""):
        headers = Bucket(
            last_headers.limit, last_headers.used, last_headers.remaining, last_headers.reset
        )
    return Reading(
        at=at,
        rest_endpoint=rest,
        own=tuple(own),
        last_headers=headers,
        answered_fields=tuple(sorted(answered)),
    )


@dataclass(frozen=True)
class Decision:
    """Whether the bucket may be drawn on now, and why."""

    proceed: bool
    why: str


def decide(now: Reading, previous: Reading | None) -> Decision:
    """The timing spike's 40% rule applied to the least figure of `now`."""

    def shaped(reading: Reading) -> dict[str, Json]:
        return {"graphql": asdict(reading.least)}

    earlier = None if previous is None else {"at": previous.at, "buckets": shaped(previous)}
    ok, why = quota.verdict("graphql", shaped(now), now.at, earlier)
    return Decision(proceed=ok, why=why)


def previous_reading(log: Path) -> Reading | None:
    """The last reading the log holds, validated, or `None`."""
    readings = [obj(line, f"{log}") for line in load_lines(log)]
    readings = [entry for entry in readings if entry.get("event") == "reading"]
    if not readings:
        return None
    last = obj(readings[-1].get("reading"), f"{log}: reading")
    headers = last.get("last_headers")
    return Reading(
        at=number(field(last, "at", "reading"), "reading.at"),
        rest_endpoint=bucket_of(field(last, "rest_endpoint", "reading"), "rest_endpoint"),
        own=tuple(bucket_of(b, "reading.own[]") for b in arr(field(last, "own", "reading"), "own")),
        last_headers=None if headers is None else bucket_of(headers, "last_headers"),
    )


@dataclass(frozen=True)
class Gated:
    """The reading the gate proceeded on, and how many pauses it took before it."""

    reading: Reading
    pauses: int


async def gate(
    client: Client,
    log: Path,
    scenario: str,
    last_headers: RateLimit | None,
    sleep: Callable[[float], Awaitable[None]] = asyncio.sleep,
) -> Gated:
    """Block until the bucket may be drawn on, every reading and pause logged.

    Answers the reading that let it, and how many pauses it took before it.
    """
    log.parent.mkdir(parents=True, exist_ok=True)
    pauses = 0
    while True:
        previous = previous_reading(log)
        now = await read(client, last_headers)
        decision = decide(now, previous)
        entry: dict[str, Json] = {
            "event": "reading",
            "scenario": scenario,
            "reading": now.to_json(),
            "proceed": decision.proceed,
            "why": decision.why,
        }
        with log.open("a", encoding="utf-8") as out:
            out.write(json.dumps(entry, sort_keys=True) + "\n")
        if decision.proceed:
            return Gated(reading=now, pauses=pauses)
        wait = max(now.least.reset - now.at + 5, MIN_PAUSE)
        with log.open("a", encoding="utf-8") as out:
            pause = {"event": "pause", "scenario": scenario, "at": now.at, "seconds": wait}
            out.write(json.dumps(pause, sort_keys=True) + "\n")
        pauses += 1
        await sleep(wait)
