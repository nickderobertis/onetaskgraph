"""The spike's quota rule: read `GET /rate_limit` (free) before every live scenario, and
pause while a bucket the scenario draws on holds under 40% of its limit, unless its burn
rate since the last reading shows it resetting before it runs out.

Every reading and every pause is appended to the readings log, which holds numbers only.
"""

from __future__ import annotations

import datetime
import json
import time
import urllib.request
from collections.abc import Callable
from pathlib import Path

FLOOR = 0.40
OWN_READS = 3
BUCKETS = ("core", "graphql")


def read(token: str, base: str = "https://api.github.com") -> dict:
    """Both readings of the token's buckets: `GET /rate_limit` (free), and GraphQL's own
    `rateLimit`, which the `x-ratelimit-*` headers of every GraphQL answer agree with.

    The two have been seen disagreeing by thousands of points about the same bucket in the
    same minute (`/rate_limit` reporting 4,799 GraphQL points left while `rateLimit` and the
    headers reported 883), so the gate holds the `graphql` bucket to the lower of the two
    and keeps the endpoint's own figure beside it as `graphql (GET /rate_limit)`.
    """
    headers = {
        "Authorization": f"Bearer {token}",
        "User-Agent": "board-copy-speed-spike",
    }
    request = urllib.request.Request(f"{base}/rate_limit", headers=headers)
    with urllib.request.urlopen(request, timeout=60) as response:
        resources = json.load(response)["resources"]
    fields = ("limit", "used", "remaining", "reset")
    out = {name: {k: resources[name][k] for k in fields} for name in BUCKETS}
    out["graphql (GET /rate_limit)"] = dict(out["graphql"])
    query = json.dumps(
        {"query": "query SpikeRateLimit { rateLimit { limit used remaining resetAt } }"}
    ).encode()
    # GitHub has answered one bucket from two counters minutes apart and even on consecutive
    # requests (216 used, then 4,458), so the bucket is read `OWN_READS` times and the least
    # remaining wins: a reading can only ever be too generous.
    readings = []
    for _ in range(OWN_READS):
        request = urllib.request.Request(
            f"{base}/graphql",
            data=query,
            headers=headers | {"Content-Type": "application/json"},
        )
        with urllib.request.urlopen(request, timeout=60) as response:
            readings.append(json.load(response)["data"]["rateLimit"])
    own = min(readings, key=lambda r: r["remaining"])
    reset = int(
        datetime.datetime.fromisoformat(
            own["resetAt"].replace("Z", "+00:00")
        ).timestamp()
    )
    out["graphql (rateLimit)"] = {
        "limit": own["limit"],
        "used": own["used"],
        "remaining": own["remaining"],
        "reset": reset,
        "readings": [r["remaining"] for r in readings],
        # Each read's own `used`, in order: the rise between two consecutive reads is what one
        # `rateLimit` read itself costs, which is subtracted from a scenario's own figure.
        "used_readings": [r["used"] for r in readings],
        "reset_readings": [r["resetAt"] for r in readings],
    }
    if own["remaining"] < out["graphql"]["remaining"]:
        out["graphql"] = {
            k: v for k, v in out["graphql (rateLimit)"].items() if not k.endswith("readings")
        }
    return out


def burn(token: str, reads: int = 5, spacing: float = 2.0, base: str = "https://api.github.com") -> list[dict]:
    """GraphQL's own `rateLimit`, read `reads` times `spacing` seconds apart while this harness
    sends nothing else: how fast every other session drawing on the same user's allowance is
    spending it, around a row. A `rateLimit` read costs nothing."""
    headers = {
        "Authorization": f"Bearer {token}",
        "User-Agent": "board-copy-targets-measurement",
        "Content-Type": "application/json",
    }
    query = json.dumps({"query": "query SpikeBurn { rateLimit { used remaining resetAt } }"}).encode()
    out = []
    for index in range(reads):
        if index:
            time.sleep(spacing)
        request = urllib.request.Request(f"{base}/graphql", data=query, headers=headers)
        with urllib.request.urlopen(request, timeout=60) as response:
            value = json.load(response)["data"]["rateLimit"]
        reset = int(
            datetime.datetime.fromisoformat(value["resetAt"].replace("Z", "+00:00")).timestamp()
        )
        out.append({"at": time.time(), "used": value["used"], "remaining": value["remaining"], "reset": reset})
    return out


def last_reading(log: Path) -> dict | None:
    if not log.exists():
        return None
    lines = [json.loads(line) for line in log.read_text().splitlines() if line.strip()]
    readings = [line for line in lines if line.get("event") == "reading"]
    return readings[-1] if readings else None


def verdict(
    bucket: str, now: dict, at: float, previous: dict | None
) -> tuple[bool, str]:
    """Whether `bucket` may be drawn on now, and why, under the 40% rule."""
    current = now[bucket]
    share = current["remaining"] / current["limit"]
    if share >= FLOOR:
        return True, f"{share:.0%} remaining"
    resets_in = current["reset"] - at
    if (
        previous is not None
        and previous["buckets"][bucket]["reset"] == current["reset"]
    ):
        spent = current["used"] - previous["buckets"][bucket]["used"]
        elapsed = at - previous["at"]
        if spent > 0 and elapsed > 0:
            lasts = current["remaining"] / (spent / elapsed)
            if resets_in < lasts:
                return True, (
                    f"{share:.0%} remaining, but at {spent / elapsed * 60:.1f}/min it lasts "
                    f"{lasts:.0f}s and resets in {resets_in:.0f}s"
                )
        elif spent == 0:
            return (
                False,
                f"{share:.0%} remaining and no burn measured since the last reading",
            )
    return False, f"{share:.0%} remaining, resets in {resets_in:.0f}s"


def gate(
    token: str,
    log: Path,
    scenario: str,
    buckets: tuple[str, ...],
    reader: Callable[[str], dict] = read,
    sleep: Callable[[float], None] = time.sleep,
    clock: Callable[[], float] = time.time,
) -> dict:
    """Block until every bucket in `buckets` may be drawn on; answer the reading that let it."""
    log.parent.mkdir(parents=True, exist_ok=True)
    while True:
        previous = last_reading(log)
        at = clock()
        now = reader(token)
        verdicts = {bucket: verdict(bucket, now, at, previous) for bucket in buckets}
        entry = {
            "event": "reading",
            "scenario": scenario,
            "at": at,
            "buckets": now,
            "verdicts": {
                b: {"proceed": ok, "why": why} for b, (ok, why) in verdicts.items()
            },
        }
        with log.open("a") as out:
            out.write(json.dumps(entry, sort_keys=True) + "\n")
        blocked = [b for b, (ok, _) in verdicts.items() if not ok]
        if not blocked:
            return entry
        wait = max(now[b]["reset"] - at for b in blocked) + 5
        wait = max(wait, 30)
        with log.open("a") as out:
            pause = {"event": "pause", "scenario": scenario, "at": at, "seconds": wait}
            out.write(json.dumps(pause | {"buckets": blocked}, sort_keys=True) + "\n")
        sleep(wait)
