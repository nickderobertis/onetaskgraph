#!/usr/bin/env python3
"""Read GitHub's GraphQL allowance three ways within seconds and say whether they agree.

    python3 spikes/store-defects/quota_readings.py [--env-file PATH] [--out FILE]

1. ``GET /rate_limit`` (documented as not counting against the primary limit);
2. GraphQL's ``rateLimit { limit remaining used resetAt cost }``;
3. the ``x-ratelimit-*`` headers on that same GraphQL call, and on one ordinary REST call.

Prints one JSON document; with ``--out`` writes it there too.
"""

from __future__ import annotations

import argparse
import json
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from ghlive import API, load_token, rate_headers, send  # noqa: E402


def read(token: str) -> dict:
    began = time.time()
    status, headers, payload, seconds = send("GET", f"{API}/rate_limit", token, None)
    rest = json.loads(payload)
    rest_reading = {
        "status": status,
        "seconds": round(seconds, 3),
        "graphql": rest["resources"]["graphql"],
        "core": rest["resources"]["core"],
        "headers": rate_headers(headers),
    }
    document = "query{rateLimit{limit remaining used resetAt cost}}"
    status, headers, payload, seconds = send(
        "POST", f"{API}/graphql", token, json.dumps({"query": document}).encode()
    )
    gql = json.loads(payload)
    graphql_reading = {
        "status": status,
        "seconds": round(seconds, 3),
        "rateLimit": gql.get("data", {}).get("rateLimit"),
        "headers": rate_headers(headers),
    }
    status, headers, _payload, seconds = send("GET", f"{API}/user", token, None)
    ordinary = {"status": status, "seconds": round(seconds, 3), "headers": rate_headers(headers)}
    finished = time.time()

    rl = rest_reading["graphql"]
    obj = graphql_reading["rateLimit"] or {}
    hdr = graphql_reading["headers"]
    figures = {
        "rate_limit_endpoint": {k: rl.get(k) for k in ("limit", "remaining", "used", "reset")},
        "rateLimit_object": obj,
        "graphql_call_headers": {
            "limit": int(hdr.get("x-ratelimit-limit", -1)),
            "remaining": int(hdr.get("x-ratelimit-remaining", -1)),
            "used": int(hdr.get("x-ratelimit-used", -1)),
            "reset": int(hdr.get("x-ratelimit-reset", -1)),
            "resource": hdr.get("x-ratelimit-resource"),
        },
    }
    limits = {
        figures["rate_limit_endpoint"]["limit"],
        obj.get("limit"),
        figures["graphql_call_headers"]["limit"],
    }
    remaining = [
        figures["rate_limit_endpoint"]["remaining"],
        obj.get("remaining"),
        figures["graphql_call_headers"]["remaining"],
    ]
    return {
        "started": began,
        "span_seconds": round(finished - began, 3),
        "rest_rate_limit": rest_reading,
        "graphql_rateLimit": graphql_reading,
        "ordinary_rest_call": ordinary,
        "figures": figures,
        "limits_agree": len(limits) == 1,
        "remaining_spread": max(remaining) - min(remaining),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--env-file")
    parser.add_argument("--out")
    args = parser.parse_args()
    answer = read(load_token(args.env_file))
    text = json.dumps(answer, indent=2)
    print(text)
    if args.out:
        Path(args.out).write_text(text + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
