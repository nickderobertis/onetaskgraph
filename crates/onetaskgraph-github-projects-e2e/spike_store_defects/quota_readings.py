#!/usr/bin/env python3
"""Read GitHub's GraphQL allowance three ways within seconds and say whether they agree.

    python3 quota_readings.py [--env-file PATH] [--endpoint URL] [--out FILE]

1. ``GET /rate_limit``, which GitHub documents as not counting against the primary limit;
2. GraphQL's ``rateLimit { limit remaining used resetAt cost }``;
3. the ``x-ratelimit-*`` headers on that same GraphQL call.

Each reading names its window by its reset time, because one token was seen answering from
two windows at once. ``--endpoint`` is the GraphQL URL, GitHub's unless given.

Exit status: ``0`` when all three were read; ``1`` when GitHub's answer to one of them was
not a reading (a refusal, or a missing or malformed figure); ``2`` when the invocation is wrong, its ``--out`` cannot be written, or no token is
configured. The readings are printed as JSON on stdout; diagnostics go
to stderr.
"""

from __future__ import annotations

import argparse
import json
import sys
import time
from dataclasses import asdict, dataclass
from datetime import datetime
from pathlib import Path

from ghlive import (
    GRAPHQL_ENDPOINT,
    BoundaryError,
    SpikeError,
    Token,
    check_out,
    decode,
    expect_int,
    expect_object,
    expect_str,
    header_int,
    load_token,
    rest_base,
    send,
    write_out,
)


@dataclass(frozen=True)
class Reading:
    """One reading of the GraphQL allowance, and the window it belongs to."""

    source: str
    limit: int
    remaining: int
    used: int
    reset: int


@dataclass(frozen=True)
class Comparison:
    started: float
    span_seconds: float
    readings: list[Reading]
    rateLimit_cost: int
    limits_agree: bool
    windows: list[int]
    remaining_spread: int


def from_rate_limit(token: Token, endpoint: str) -> Reading:
    response = send("GET", f"{rest_base(endpoint)}/rate_limit", token, None)
    if response.status != 200:
        raise BoundaryError(
            f"GET /rate_limit answered {response.status}: {response.body[:200]!r}"
        )
    resources = expect_object(
        decode(response.body, "/rate_limit").get("resources"), "resources"
    )
    graphql = expect_object(resources.get("graphql"), "resources.graphql")
    return Reading(
        source="GET /rate_limit",
        limit=expect_int(graphql.get("limit"), "graphql.limit"),
        remaining=expect_int(graphql.get("remaining"), "graphql.remaining"),
        used=expect_int(graphql.get("used"), "graphql.used"),
        reset=expect_int(graphql.get("reset"), "graphql.reset"),
    )


def from_graphql(token: Token, endpoint: str) -> tuple[Reading, Reading, int]:
    document = "query{rateLimit{limit remaining used resetAt cost}}"
    response = send("POST", endpoint, token, json.dumps({"query": document}).encode())
    if response.status != 200:
        raise BoundaryError(f"the rateLimit query answered {response.status}")
    answer = decode(response.body, "the rateLimit answer")
    if answer.get("errors"):
        raise BoundaryError(f"the rateLimit query answered errors: {answer['errors']}")
    rate = expect_object(
        expect_object(answer.get("data"), "data").get("rateLimit"), "rateLimit"
    )
    reset_at = expect_str(rate.get("resetAt"), "rateLimit.resetAt")
    try:
        reset_time = datetime.fromisoformat(reset_at)
    except ValueError as error:
        raise BoundaryError(f"rateLimit.resetAt is not a time: {reset_at!r}") from error
    if reset_time.tzinfo is None:
        # A naive time would be read in the host's zone and misplace the window.
        raise BoundaryError(f"rateLimit.resetAt names no time zone: {reset_at!r}")
    obj = Reading(
        source="GraphQL rateLimit",
        limit=expect_int(rate.get("limit"), "rateLimit.limit"),
        remaining=expect_int(rate.get("remaining"), "rateLimit.remaining"),
        used=expect_int(rate.get("used"), "rateLimit.used"),
        reset=int(reset_time.timestamp()),
    )
    figures: dict[str, int] = {}
    for name in ("limit", "remaining", "used", "reset"):
        value = header_int(response.headers, f"x-ratelimit-{name}")
        if value is None:
            raise BoundaryError(
                f"the rateLimit call carried no x-ratelimit-{name} header"
            )
        figures[name] = value
    headers = Reading(source="GraphQL call headers", **figures)
    return obj, headers, expect_int(rate.get("cost"), "rateLimit.cost")


def compare(token: Token, endpoint: str) -> Comparison:
    began = time.time()
    rest = from_rate_limit(token, endpoint)
    obj, headers, cost = from_graphql(token, endpoint)
    readings = [rest, obj, headers]
    remaining = [reading.remaining for reading in readings]
    return Comparison(
        started=began,
        span_seconds=round(time.time() - began, 3),
        readings=readings,
        rateLimit_cost=cost,
        limits_agree=len({reading.limit for reading in readings}) == 1,
        windows=sorted({reading.reset for reading in readings}),
        remaining_spread=max(remaining) - min(remaining),
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=(__doc__ or "").splitlines()[0])
    parser.add_argument("--env-file")
    parser.add_argument("--endpoint", default=GRAPHQL_ENDPOINT)
    parser.add_argument("--out", type=Path)
    args = parser.parse_args()
    if args.out:
        try:
            check_out(args.out)
        except SpikeError as error:
            print(f"quota_readings: {error}", file=sys.stderr)
            return 2
    try:
        token = load_token(args.env_file)
    except SpikeError as error:
        print(f"quota_readings: {error}", file=sys.stderr)
        return 2
    try:
        comparison = compare(token, args.endpoint)
    except BoundaryError as error:
        print(f"quota_readings: {error}", file=sys.stderr)
        return 1
    text = json.dumps(asdict(comparison), indent=2)
    print(text)
    if args.out:
        try:
            write_out(args.out, (text + "\n").encode())
        except SpikeError as error:
            print(f"quota_readings: {error}", file=sys.stderr)
            return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
