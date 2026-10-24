"""Price every GraphQL operation the spike's runs sent, in GitHub's primary-limit points.

The plugin does not ask GitHub for `rateLimit { cost }`, and the bucket the
`x-ratelimit-used` header reports is shared with every other session drawing on the token,
so neither the answers nor the bucket's movement say what one request cost. So:

- a **query** is priced by sending one recorded instance of it again with
  `rateLimit(dryRun: true) { cost }` — GitHub's own calculation, without running it;
- a **mutation** is priced by the smallest rise of `x-ratelimit-used` between two
  consecutive requests of one process (other sessions only ever add to a rise).

`python3 pricing.py` reads the raw runs under the cache and writes `spike/logs/pricing.json`.
"""

from __future__ import annotations

import json
import os
import re
import sys
import urllib.request
from collections import defaultdict
from pathlib import Path

HERE = Path(__file__).resolve().parent
CACHE = Path(os.environ.get("XDG_CACHE_HOME") or Path.home() / ".cache") / (
    "board-copy-speed-2026-10-18"
)
OUT = HERE.parent / "logs" / "pricing.json"


def dry_run(document: str) -> str:
    unpriced = document.replace("rateLimit{cost} ", "", 1)
    root = unpriced.index("{")
    return unpriced[: root + 1] + "rateLimit(dryRun:true){cost} " + unpriced[root + 1 :]


def samples(runs: Path) -> tuple[dict[str, dict], dict[str, list[int]]]:
    queries: dict[str, dict] = {}
    rises: dict[str, list[int]] = defaultdict(list)
    for log in sorted(runs.glob("*/proxy.jsonl")):
        bodies = log.with_suffix(".bodies")
        records = sorted(
            (json.loads(line) for line in log.read_text().splitlines()),
            key=lambda r: r["start"],
        )
        previous = None
        for record in records:
            if (
                record["kind"] in ("rest", "tunnel", "upload")
                or record["host"] == "refused"
            ):
                previous = None
                continue
            used = record["limits"].get("x-ratelimit-used")
            reset = record["limits"].get("x-ratelimit-reset")
            if record["kind"] == "query" and record["op"] not in queries:
                request = bodies / f"{record['seq']:06d}.req"
                if request.exists():
                    queries[record["op"]] = json.loads(request.read_text())
            if (
                previous is not None
                and used is not None
                and previous["reset"] == reset
                and previous["end"] <= record["start"]
            ):
                rises[record["op"]].append(int(used) - previous["used"])
            previous = (
                {"used": int(used), "reset": reset, "end": record["end"]}
                if used
                else None
            )
    return queries, rises


def price(document: dict, token: str) -> int | None:
    request = urllib.request.Request(
        "https://api.github.com/graphql",
        data=json.dumps(
            {"query": dry_run(document["query"]), "variables": document["variables"]}
        ).encode(),
        headers={
            "Authorization": f"Bearer {token}",
            "Content-Type": "application/json",
        },
    )
    with urllib.request.urlopen(request, timeout=60) as response:
        answer = json.load(response)
    return ((answer.get("data") or {}).get("rateLimit") or {}).get("cost")


def main() -> None:
    token = os.environ.get("GH_PROJECTS_TOKEN", "").strip()
    if not token:
        sys.exit("pricing.py: GH_PROJECTS_TOKEN is not set")
    queries, rises = samples(CACHE / "runs")
    prices = json.loads(OUT.read_text()) if OUT.exists() else {}
    for op, document in sorted(queries.items()):
        if op in prices and prices[op].get("method") == "dry run":
            continue
        cost = price(document, token)
        if cost is not None:
            sizes = {
                k: v
                for k, v in document["variables"].items()
                if re.fullmatch(r"first|nestedFirst|boardItems|[a-z]*First", k)
            }
            prices[op] = {"points": cost, "method": "dry run", "variables": sizes}
    for op, values in sorted(rises.items()):
        positive = [v for v in values if v > 0]
        if op not in prices and positive:
            prices[op] = {
                "points": min(positive),
                "method": "smallest header rise",
                "observations": len(positive),
            }
    OUT.write_text(json.dumps(prices, indent=1, sort_keys=True) + "\n")
    for op, entry in sorted(prices.items()):
        print(f"{op}: {entry['points']} ({entry['method']})")


if __name__ == "__main__":
    main()
