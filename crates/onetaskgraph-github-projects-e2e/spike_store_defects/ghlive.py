"""The one GitHub client every `spike-store-defects` measurement goes through.

Every request is recorded the same way and held to the spike's two limits:

- a points threshold per step (``SpikeLimits.stop_after_points``): no request is sent once
  the step's spend has reached it, so the one request in flight can take the spend past it by
  its own cost. Spend is counted from the ``cost`` GitHub itself reports, which is asked for by adding ``spikeRate:rateLimit{...}`` to the
  top-level selection of each GraphQL document and removed from the answer again;
- a remaining floor (``SpikeLimits.floor``, 3,000 by the task): once any reading says the
  GraphQL allowance is under it, nothing more is sent.

Decoded JSON is ``object`` until one of the ``expect_*`` helpers below narrows it, so every
third-party answer is validated where it is first read.
"""

from __future__ import annotations

import json
import os
import re
import time
import urllib.error
import urllib.request
from collections.abc import Mapping
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import NewType

Token = NewType("Token", str)
GRAPHQL_ENDPOINT = "https://api.github.com/graphql"
RATE_SELECTION = "spikeRate:rateLimit{cost limit remaining used resetAt}"
DEFAULT_FLOOR = 3000


class SpikeError(RuntimeError):
    """A measurement could not go on; the message says what and what to do next."""


class SpikeStop(SpikeError):
    """A limit of the spike refused a request before it was sent."""


class BoundaryError(SpikeError):
    """An answer or an input did not have the shape this harness reads."""


def expect_object(value: object, what: str) -> dict[str, object]:
    if not isinstance(value, dict):
        raise BoundaryError(f"{what} is not a JSON object: {str(value)[:200]}")
    return {str(key): item for key, item in value.items()}


def expect_list(value: object, what: str) -> list[object]:
    if not isinstance(value, list):
        raise BoundaryError(f"{what} is not a JSON array: {str(value)[:200]}")
    return list(value)


def expect_str(value: object, what: str) -> str:
    if not isinstance(value, str):
        raise BoundaryError(f"{what} is not a string: {value!r}")
    return value


def expect_int(value: object, what: str) -> int:
    # `bool` is an `int` in Python, and a JSON `true` is never a count.
    if isinstance(value, bool) or not isinstance(value, int):
        raise BoundaryError(f"{what} is not an integer: {value!r}")
    return value


def expect_bool(value: object, what: str) -> bool:
    if not isinstance(value, bool):
        raise BoundaryError(f"{what} is not a boolean: {value!r}")
    return value


def decode(payload: bytes, what: str) -> dict[str, object]:
    try:
        return expect_object(json.loads(payload or b"{}"), what)
    except json.JSONDecodeError as error:
        raise BoundaryError(
            f"{what} is not JSON ({error}): {payload[:200]!r}"
        ) from error


def positive(value: str) -> int:
    """An argparse type for a count that has to be at least one."""
    number = int(value)
    if number < 1:
        raise ValueError(f"{value} is not a positive integer")
    return number


def load_token(env_file: str | None) -> Token:
    """``GH_PROJECTS_TOKEN``, from the environment or the ``--env-file`` line of that name.

    Never printed and never written anywhere.
    """
    token = os.environ.get("GH_PROJECTS_TOKEN", "")
    if not token and env_file:
        try:
            lines = Path(env_file).read_text(encoding="utf-8").splitlines()
        except (OSError, UnicodeDecodeError) as error:
            raise SpikeError(f"cannot read --env-file {env_file}: {error}") from error
        for line in lines:
            name, _, value = line.partition("=")
            if name.strip() == "GH_PROJECTS_TOKEN":
                token = value.strip().strip('"').strip("'")
    if not token.strip():
        raise SpikeError(
            "no GH_PROJECTS_TOKEN in the environment or the --env-file; set one and retry"
        )
    return Token(token.strip())


def rest_base(graphql_endpoint: str) -> str:
    """The REST root beside a GraphQL endpoint: GitHub's, or a loopback board's."""
    return graphql_endpoint.removesuffix("/graphql")


def top_level_words(document: str) -> list[str]:
    """The words of ``document`` outside every selection set and argument list, with GraphQL
    comments and string literals removed: its definitions' keywords and names."""
    text = re.sub(r'"""[\s\S]*?"""|"(?:\\.|[^"\\])*"|#[^\n]*', " ", document)
    words: list[str] = []
    depth = 0
    for token in re.findall(r"[A-Za-z_][A-Za-z0-9_]*|[{}()\[\]]", text):
        match token:
            case "{" | "(" | "[":
                depth += 1
            case "}" | ")" | "]":
                depth -= 1
            case _ if depth == 0:
                words.append(token)
            case _:
                pass
    return words


def writes(document: str) -> bool:
    """Whether ``document`` defines any operation but a query: a mutation or a subscription."""
    return any(
        word in {"mutation", "subscription"} for word in top_level_words(document)
    )


def inject_rate(document: str) -> str:
    """``document`` with ``RATE_SELECTION`` added to its operation's top-level selection."""
    if writes(document):
        raise SpikeStop("only queries are metered and sent by this client")
    start = document.find("query")
    if start < 0:
        raise SpikeStop("the document names no query operation")
    depth = 0
    for index in range(start, len(document)):
        match document[index]:
            case "(":
                depth += 1
            case ")":
                depth -= 1
            case "{" if depth == 0:
                return (
                    document[: index + 1] + RATE_SELECTION + " " + document[index + 1 :]
                )
            case _:
                pass
    raise SpikeStop("could not find the operation's selection set")


def rate_headers(headers: Mapping[str, str]) -> dict[str, str]:
    return {
        key.lower(): value
        for key, value in headers.items()
        if key.lower().startswith("x-ratelimit-")
    }


@dataclass(frozen=True)
class HttpResponse:
    """One answer, its header names lowercased: HTTP header names are case-insensitive."""

    status: int
    headers: dict[str, str]
    body: bytes
    seconds: float


@dataclass(frozen=True)
class Metering:
    """What one GraphQL answer said about its own cost and the allowance after it."""

    cost: int | None
    remaining: int | None
    header_remaining: int | None
    header_reset: int | None


def lowercase(headers: Mapping[str, str]) -> dict[str, str]:
    return {key.lower(): value for key, value in headers.items()}


def header_int(headers: Mapping[str, str], name: str) -> int | None:
    value = headers.get(name)
    if value is None:
        return None
    if not value.strip().isdigit():
        raise BoundaryError(f"header {name} is not a number: {value!r}")
    return int(value)


def meter(answer: dict[str, object], headers: Mapping[str, str]) -> Metering:
    """Take ``spikeRate`` out of ``answer``'s data and read the allowance it reports."""
    data = answer.get("data")
    rate: dict[str, object] = {}
    if isinstance(data, dict):
        raw = data.pop("spikeRate", None)
        if raw is not None:
            rate = expect_object(raw, "rateLimit")
    cost = rate.get("cost")
    remaining = rate.get("remaining")
    return Metering(
        cost=None if cost is None else nonnegative(cost, "rateLimit.cost"),
        remaining=None
        if remaining is None
        else nonnegative(remaining, "rateLimit.remaining"),
        header_remaining=header_int(headers, "x-ratelimit-remaining"),
        header_reset=header_int(headers, "x-ratelimit-reset"),
    )


def nonnegative(value: object, what: str) -> int:
    number = expect_int(value, what)
    if number < 0:
        raise BoundaryError(f"{what} is negative: {number}")
    return number


@dataclass
class SpikeLimits:
    stop_after_points: int
    floor: int = DEFAULT_FLOOR
    spent: int = 0
    latest_remaining: int | None = None

    def admit(self) -> None:
        if self.spent >= self.stop_after_points:
            raise SpikeStop(
                f"spend reached the stop threshold: {self.spent} of {self.stop_after_points}"
            )
        if self.latest_remaining is not None and self.latest_remaining < self.floor:
            raise SpikeStop(
                f"GraphQL remaining {self.latest_remaining} is under the floor {self.floor}"
            )

    def charge(self, metering: Metering) -> None:
        """Count a request's cost, and keep the latest reading of the allowance.

        An answer that reports no cost (a loopback board's, or one GitHub refused) is charged
        GitHub's minimum of one point, so a missing figure never lets the cap stop counting.
        The latest reading rather than the least: a reading only ever falls within a window,
        and a window that resets has to be able to admit work again.
        """
        self.spent += 1 if metering.cost is None else metering.cost
        readings = [
            r for r in (metering.remaining, metering.header_remaining) if r is not None
        ]
        if readings:
            self.latest_remaining = min(readings)


@dataclass
class RequestRecord:
    """One request this spike sent, or refused to send."""

    phase: str
    at: float
    status: int | None = None
    seconds: float | None = None
    cost: int | None = None
    remaining: int | None = None
    items: int | None = None
    refused: str | None = None
    variables: dict[str, object] = field(default_factory=dict)
    headers: dict[str, str] = field(default_factory=dict)
    errors: object = None

    def to_json(self) -> dict[str, object]:
        return asdict(self)


def send(
    method: str,
    url: str,
    token: Token,
    body: bytes | None,
    timeout: float = 120,
) -> HttpResponse:
    headers = {
        "Authorization": f"Bearer {token}",
        "Accept": "application/vnd.github+json",
        "User-Agent": "onetaskgraph-spike-store-defects",
    }
    if body is not None:
        headers["Content-Type"] = "application/json"
    request = urllib.request.Request(url, data=body, method=method, headers=headers)
    began = time.monotonic()
    try:
        # llmlint: ignore[async_typed_clients_at_boundaries] Each step sends one request at a time on purpose: the per-request seconds and the order of the readings are the measurement, so concurrency would corrupt the figures. The client is typed (`HttpResponse`); stdlib `urllib` keeps the harness free of installs on any host.
        with urllib.request.urlopen(request, timeout=timeout) as response:
            return HttpResponse(
                response.status,
                lowercase(response.headers),
                response.read(),
                time.monotonic() - began,
            )
    except urllib.error.HTTPError as error:
        return HttpResponse(
            error.code, lowercase(error.headers), error.read(), time.monotonic() - began
        )


def graphql(
    endpoint: str,
    document: str,
    variables: dict[str, object],
    token: Token,
    limits: SpikeLimits,
    records: list[RequestRecord],
    phase: str,
    timeout: float = 120,
) -> tuple[dict[str, object], RequestRecord]:
    """Send one query, held to ``limits``; answer its validated envelope and its record."""
    limits.admit()
    body = json.dumps({"query": inject_rate(document), "variables": variables}).encode()
    response = send("POST", endpoint, token, body, timeout)
    answer = decode(response.body, f"the {phase} answer")
    metering = meter(answer, response.headers)
    limits.charge(metering)
    record = RequestRecord(
        phase=phase,
        at=time.time(),
        status=response.status,
        seconds=round(response.seconds, 3),
        cost=metering.cost,
        remaining=metering.remaining,
        headers=rate_headers(response.headers),
        errors=answer.get("errors"),
    )
    records.append(record)
    return answer, record
