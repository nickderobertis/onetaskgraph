"""Shared plumbing for the `spike-store-defects` live measurements.

Throwaway spike code. One GitHub client that every live step goes through, so that every
request is recorded the same way and every request is held to the spike's two limits:

- a points cap for the whole step (``SpikeLimits.max_points``), counted from the ``cost``
  GitHub itself reports, which is asked for by adding ``spikeRate:rateLimit{...}`` to the
  top-level selection of every GraphQL document sent and removed from the answer again;
- a remaining floor (``SpikeLimits.floor``, 3,000 by the task): once a reading says the
  GraphQL allowance is under it, nothing more is sent.

The token is never printed or written: it is read from ``GH_PROJECTS_TOKEN`` or from the line
of that name in an ``--env-file``.
"""

from __future__ import annotations

import json
import os
import time
import urllib.error
import urllib.request
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

API = "https://api.github.com"
RATE_SELECTION = "spikeRate:rateLimit{cost limit remaining used resetAt}"


class SpikeStop(RuntimeError):
    """A limit of the spike refused a request before it was sent."""


def load_token(env_file: str | None) -> str:
    token = os.environ.get("GH_PROJECTS_TOKEN", "")
    if not token and env_file:
        for line in Path(env_file).read_text(encoding="utf-8").splitlines():
            name, _, value = line.partition("=")
            if name.strip() == "GH_PROJECTS_TOKEN":
                token = value.strip().strip('"').strip("'")
    if not token:
        raise SystemExit(
            "no GH_PROJECTS_TOKEN in the environment or the --env-file; set one and retry"
        )
    return token


def inject_rate(document: str) -> str:
    """``document`` with ``RATE_SELECTION`` added to its operation's top-level selection."""
    start = document.find("query")
    if start < 0 or document.lstrip().startswith("mutation"):
        raise SpikeStop("only queries are sent by this spike")
    depth = 0
    for index in range(start, len(document)):
        char = document[index]
        if char == "(":
            depth += 1
        elif char == ")":
            depth -= 1
        elif char == "{" and depth == 0:
            return document[: index + 1] + RATE_SELECTION + " " + document[index + 1 :]
    raise SpikeStop("could not find the operation's selection set")


def rate_headers(headers: Any) -> dict[str, str]:
    return {
        key.lower(): value
        for key, value in headers.items()
        if key.lower().startswith("x-ratelimit-")
    }


@dataclass
class SpikeLimits:
    max_points: int
    floor: int = 3000
    spent: int = 0
    last_remaining: int | None = None

    def admit(self) -> None:
        if self.spent >= self.max_points:
            raise SpikeStop(f"points cap reached: {self.spent} of {self.max_points}")
        if self.last_remaining is not None and self.last_remaining < self.floor:
            raise SpikeStop(
                f"GraphQL remaining {self.last_remaining} is under the floor {self.floor}"
            )


@dataclass
class Recorder:
    records: list[dict[str, Any]] = field(default_factory=list)

    def add(self, record: dict[str, Any]) -> None:
        self.records.append(record)


def send(
    method: str,
    url: str,
    token: str,
    body: bytes | None,
    extra_headers: dict[str, str] | None = None,
) -> tuple[int, dict[str, str], bytes, float]:
    headers = {
        "Authorization": f"Bearer {token}",
        "Accept": "application/vnd.github+json",
        "User-Agent": "onetaskgraph-spike-store-defects",
    }
    if body is not None:
        headers["Content-Type"] = "application/json"
    headers.update(extra_headers or {})
    request = urllib.request.Request(url, data=body, method=method, headers=headers)
    began = time.monotonic()
    try:
        with urllib.request.urlopen(request, timeout=120) as response:
            payload = response.read()
            return response.status, dict(response.headers), payload, time.monotonic() - began
    except urllib.error.HTTPError as error:
        return error.code, dict(error.headers), error.read(), time.monotonic() - began


def graphql(
    document: str,
    variables: dict[str, Any],
    token: str,
    limits: SpikeLimits,
    recorder: Recorder,
    phase: str,
) -> tuple[dict[str, Any], dict[str, Any]]:
    """Send one query, held to ``limits``; answer its decoded JSON and its record."""
    limits.admit()
    body = json.dumps({"query": inject_rate(document), "variables": variables}).encode()
    status, headers, payload, seconds = send("POST", f"{API}/graphql", token, body)
    answer = json.loads(payload or b"{}")
    rate = (answer.get("data") or {}).pop("spikeRate", None) or {}
    record = {
        "phase": phase,
        "status": status,
        "seconds": round(seconds, 3),
        "at": time.time(),
        "cost": rate.get("cost"),
        "remaining": rate.get("remaining"),
        "headers": rate_headers(headers),
        "errors": answer.get("errors"),
    }
    if isinstance(rate.get("cost"), int):
        limits.spent += rate["cost"]
    if isinstance(rate.get("remaining"), int):
        limits.last_remaining = rate["remaining"]
    recorder.add(record)
    return answer, record
