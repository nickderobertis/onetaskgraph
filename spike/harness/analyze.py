"""Turn the committed, sanitized logs into the spike's figures.

`python3 analyze.py [LOG_DIR...]` (default: every run under `spike/logs/`) writes
`spike/logs/summary.json` and prints a Markdown digest. Every figure is computed from the
committed logs alone, so anyone can re-derive the report from the branch.

Attribution: each instant of a scenario's wall time goes to exactly one category, the
highest-priority span active at that instant (`CATEGORIES`, highest first). A request the
proxy recorded is named by joining it to the `send_once` span it falls inside. Time inside a
measured process that no span covers — and, for the stock wheel, everything outside a
request — is `unattributed`, never folded into a neighbour.
"""

from __future__ import annotations

import json
import statistics
import sys
from collections import defaultdict
from pathlib import Path

HERE = Path(__file__).resolve().parent
import os

LOGS = Path(os.environ.get("SPIKE_LOGS") or HERE.parent / "logs")

WALK = {"BOARD", "SEARCH_ISSUES"}
REWRITING = {
    "copy.rewrite_references",
    "copy.counterparts",
    "copy.referents",
    "copy.substitute",
}
COMPOSING = {"compose_body", "with_slot", "copy.outgoing", "copy.carried"}
CATEGORIES = (
    "secondary-limit waits",
    "primary-limit waits",
    "pacing waits",
    "asset upload",
    "whole-board walk pages",
    "writes",
    "destination reads",
    "reference rewriting",
    "body composition and rendering",
    "engine and local-source work",
    "CLI (configuration, engine build, output)",
    "write-boundary screening (onevcs)",
    "unattributed inside the store process (stock wheel: no spans)",
    "host-side steps",
    "unattributed",
)
# Documented in GitHub's "Rate limits for the GraphQL API": secondary limits.
CONTENT_CREATING = {
    "CREATE_ISSUE",
    "ADD_COMMENT",
    "ADD_COMMENT_WHOLE",
    "POST /user-attachments/assets",
    "CREATE_ISSUES",
}
CREATES = {"CREATE_ISSUE", "CREATE_ISSUES"}
BATCHED = {
    "ORIGIN_LOOKUPS",
    "CREATED_BOARD_ITEMS",
    "ISSUE_DETAILS",
    "ISSUE_COPY_DETAILS",
    "ISSUES_BY_REFERENCE",
}
ALIASED = {"CREATE_ISSUES", "WRITE_FIELD_VALUES", "ADD_BLOCKED_BYS", "CLOSE_ISSUES"}
SCREENING_HOSTS = {"api.github.com", "cafe.github.com"}


def lines(path: Path) -> list[dict]:
    if not path.exists():
        return []
    return [json.loads(line) for line in path.read_text().splitlines() if line.strip()]


def request_kind(request: dict) -> str:
    if request["kind"] == "tunnel":
        if request["host"] in SCREENING_HOSTS:
            return "screening tunnel"
        return "upload"
    if request["path"] == "/user-attachments/assets" or request["op"].startswith(
        "GET /repos"
    ):
        return "upload"
    if request["op"] in WALK:
        return "board or search page"
    if request["kind"] == "mutation":
        if request["op"] in ALIASED or request.get("mutations", 1) > 1:
            return "aliased mutations"
        return "mutation"
    if request["op"] in BATCHED:
        return "batched read"
    return "item read"


def request_category(request: dict | None) -> str:
    if request is None:
        return "destination reads"
    kind = request_kind(request)
    return {
        "upload": "asset upload",
        "board or search page": "whole-board walk pages",
        "mutation": "writes",
        "aliased mutations": "writes",
    }.get(kind, "destination reads")


def span_category(span: dict, joined: dict | None) -> str | None:
    name = span["span"]
    if name == "retry_wait":
        if span["attrs"].get("limiter") == "secondary":
            return "secondary-limit waits"
        return "primary-limit waits"
    if name == "pacing_wait":
        return "pacing waits"
    if name in ("asset_request", "upload_assets"):
        return "asset upload"
    if name == "send_once":
        return request_category(joined)
    if name in REWRITING:
        return "reference rewriting"
    if name in COMPOSING:
        return "body composition and rendering"
    if name.startswith("copy."):
        return "engine and local-source work"
    if name == "cli.run":
        return "CLI (configuration, engine build, output)"
    return None


def sweep(
    intervals: list[tuple[float, float, str]], start: float, end: float
) -> dict[str, float]:
    """Seconds of [start, end] per category, each instant to its highest-priority interval."""
    rank = {c: i for i, c in enumerate(CATEGORIES)}
    points = sorted(
        {start, end, *(t for a, b, _ in intervals for t in (a, b) if start < t < end)}
    )
    out: dict[str, float] = defaultdict(float)
    for left, right in zip(points, points[1:], strict=False):
        middle = (left + right) / 2
        active = [c for a, b, c in intervals if a <= middle < b]
        best = min(active, key=rank.__getitem__) if active else "unattributed"
        out[best] += right - left
    return dict(out)


def join(spans: list[dict], requests: list[dict]) -> dict[int, dict]:
    """The proxied request each `send_once` span sent, by the span's index."""
    joined: dict[int, dict] = {}
    pending = sorted(
        (r for r in requests if r["kind"] != "tunnel"), key=lambda r: r["start"]
    )
    for index, span in sorted(
        ((i, s) for i, s in enumerate(spans) if s["span"] == "send_once"),
        key=lambda p: p[1]["start"],
    ):
        for request in pending:
            if (
                span["start"] - 0.05 <= request["start"]
                and request["end"] <= span["end"] + 0.05
            ):
                joined[index] = request
                pending.remove(request)
                break
    return joined


def enclosing_phase(spans: list[dict], request: dict) -> str:
    inside = [
        s
        for s in spans
        if s["span"].startswith("copy.")
        and s["span"] not in ("copy.copy", "copy.copy_all")
        and s["start"] <= request["start"]
        and request["end"] <= s["end"]
    ]
    return (
        min(inside, key=lambda s: s["end"] - s["start"])["span"]
        if inside
        else "outside copy"
    )


def sent(run: Path) -> list[dict]:
    """Every request that left for GitHub: the proxy's own refusals never did."""
    return [r for r in lines(run / "requests.jsonl") if r["host"] != "refused"]


def attribute(run: Path) -> dict:
    steps = lines(run / "steps.jsonl")
    requests = sent(run)
    spans = lines(run / "spans.jsonl")
    host = lines(run / "host.jsonl")
    joined = join(spans, requests)
    intervals: list[tuple[float, float, str]] = []
    for index, span in enumerate(spans):
        category = span_category(span, joined.get(index))
        if category:
            intervals.append((span["start"], span["end"], category))
    if not spans:
        for request in requests:
            if request["kind"] == "tunnel":
                continue  # a tunnel's record spans the connection's life, not one request
            intervals.append(
                (request["start"], request["end"], request_category(request))
            )
        stores = [h for h in host if h["kind"] == "store"]
        for store in stores:
            intervals.append(
                (
                    store["start"],
                    store["end"],
                    "unattributed inside the store process (stock wheel: no spans)",
                )
            )
        if host and steps:
            for step in steps:
                intervals.append((step["start"], step["end"], "host-side steps"))
        elif steps:
            for step in steps:
                intervals.append(
                    (
                        step["start"],
                        step["end"],
                        "unattributed inside the store process (stock wheel: no spans)",
                    )
                )
    for screen in lines(run / "screen.jsonl"):
        intervals.append(
            (screen["start"], screen["end"], "write-boundary screening (onevcs)")
        )
    split: dict[str, float] = defaultdict(float)
    for step in steps:
        for category, seconds in sweep(intervals, step["start"], step["end"]).items():
            split[category] += seconds
    wall = sum(s["end"] - s["start"] for s in steps)
    return {
        "wall": wall,
        "split": dict(split),
        "joined": joined,
        "spans": spans,
        "requests": requests,
    }


def run_summary(run: Path, prices: dict) -> dict:
    attributed = attribute(run)
    requests = attributed["requests"]
    steps = lines(run / "steps.jsonl")
    per_step = []
    for step in steps:
        inside = [r for r in requests if step["start"] <= r["start"] <= step["end"]]
        per_step.append(
            {
                "step": step["step"],
                "wall": round(step["end"] - step["start"], 2),
                "returncode": step["returncode"],
                "sequence": [r["op"] for r in sorted(inside, key=lambda r: r["start"])],
            }
        )
    by_op: dict[str, dict] = {}
    for request in requests:
        entry = by_op.setdefault(
            request["op"],
            {"count": 0, "seconds": 0.0, "points": 0, "kind": request_kind(request)},
        )
        entry["count"] += 1
        if request["kind"] != "tunnel":
            entry["seconds"] += request["ms"] / 1000
        entry["points"] += points(request, prices)
    walk_phases: dict[str, int] = defaultdict(int)
    for request in requests:
        if request["op"] in WALK:
            walk_phases[enclosing_phase(attributed["spans"], request)] += 1
    host = lines(run / "host.jsonl")
    host_steps: dict[str, dict] = {}
    for record in host:
        if record["kind"] == "step":
            entry = host_steps.setdefault(record["name"], {"calls": 0, "seconds": 0.0})
            entry["calls"] += 1
            entry["seconds"] += record["end"] - record["start"]
    stores = [
        {"argv": " ".join(r["argv"][:3]), "seconds": round(r["end"] - r["start"], 2)}
        for r in host
        if r["kind"] == "store"
    ]
    return {
        "wall": round(attributed["wall"], 2),
        "split": {
            c: round(attributed["split"].get(c, 0.0), 2)
            for c in CATEGORIES
            if c in attributed["split"]
        },
        "steps": per_step,
        "requests": sum(1 for r in requests if r["kind"] != "tunnel"),
        "tunnels": sum(
            1 for r in requests if request_kind(r) == "upload" and r["kind"] == "tunnel"
        ),
        "screening_tunnels": sum(
            1 for r in requests if request_kind(r) == "screening tunnel"
        ),
        "screening_calls": dict(
            sorted(
                (name, sum(1 for s in lines(run / "screen.jsonl") if s["name"] == name))
                for name in {s["name"] for s in lines(run / "screen.jsonl")}
            )
        ),
        "issues_created": sum(
            r.get("mutations", 1) for r in requests if r["op"] in CREATES
        ),
        "uploads": sum(1 for r in requests if r["op"] == "POST /user-attachments/assets"),
        "mutation_fields": sum(
            r.get("mutations", 1) for r in requests if r["kind"] == "mutation"
        ),
        "points": sum(points(r, prices) for r in requests),
        "content_creating": sum(1 for r in requests if r["op"] in CONTENT_CREATING),
        "mutations": sum(1 for r in requests if r["kind"] == "mutation"),
        "by_op": {
            k: {**v, "seconds": round(v["seconds"], 2)}
            for k, v in sorted(by_op.items())
        },
        "walk_pages_by_phase": dict(walk_phases),
        "host_steps": {
            k: {**v, "seconds": round(v["seconds"], 2)}
            for k, v in sorted(host_steps.items())
        },
        "store_calls": stores,
        "statuses": sorted({r["status"] for r in requests}),
        "refused_by_proxy": sum(
            1 for r in lines(run / "requests.jsonl") if r["host"] == "refused"
        ),
        "graphql_bucket_by_headers": bucket_by_headers(requests),
    }


def bucket_by_headers(requests: list[dict]) -> dict | None:
    """The GraphQL bucket as the run's own answers reported it: remaining at the first
    GraphQL answer and the least seen, with the limit, so a run's start can be held to the
    40% rule by the figure GitHub enforced rather than by `GET /rate_limit`."""
    seen = [
        r["limits"]
        for r in sorted(requests, key=lambda r: r["start"])
        if r["limits"].get("resource") == "graphql" and "remaining" in r["limits"]
    ]
    if not seen:
        return None
    return {
        "limit": seen[0]["limit"],
        "first_remaining": seen[0]["remaining"],
        "least_remaining": min(s["remaining"] for s in seen),
        "reset": seen[0].get("reset"),
    }


def points(request: dict, prices: dict) -> int:
    """GraphQL points: GitHub's own reported cost where the query asked, else the op's price
    measured by `pricing.py` (a dry run for a query, the header delta for a mutation)."""
    if request["kind"] in ("rest", "tunnel"):
        return 0
    if isinstance(request.get("cost"), int):
        return request["cost"]
    return int(prices.get(request["op"], {}).get("points", 1))


def latency(runs: dict[str, dict]) -> dict[str, dict]:
    samples: dict[str, list[float]] = defaultdict(list)
    for name, summary in runs.items():
        if name.startswith("probes"):
            continue
        board = "plans" if name.startswith("plans") else "test board"
        for request in summary["_requests"]:
            if request["kind"] == "tunnel" or request["status"] >= 400:
                continue
            kind = request_kind(request)
            if kind == "board or search page":
                kind = f"{request['op']} page ({board})"
            elif kind == "upload":
                kind = f"upload: {request['op']}"
            samples[kind].append(request["ms"])
    out = {}
    for kind, values in sorted(samples.items()):
        values.sort()
        out[kind] = {
            "n": len(values),
            "median_ms": round(statistics.median(values)),
            "p90_ms": round(values[int(0.9 * (len(values) - 1))]),
            "max_ms": round(values[-1]),
            "mean_ms": round(statistics.fmean(values)),
        }
    return out


def inertness(runs: dict[str, dict]) -> dict[str, dict]:
    """For each test-board scenario, the stock wheel's request sequence against the span
    build's, per process: operation names, order and count."""
    out = {}
    for scenario in ("tb-first", "tb-changed", "tb-unchanged"):
        stock = [n for n in runs if n.startswith(f"{scenario}-stock-")]
        span = [n for n in runs if n.startswith(f"{scenario}-span-")]
        if not stock or not span:
            continue
        a, b = runs[max(stock)], runs[max(span)]
        differences = []
        for x, y in zip(a["steps"], b["steps"], strict=False):
            if x["sequence"] != y["sequence"]:
                differences.append(
                    {"step": x["step"], "stock": x["sequence"], "span": y["sequence"]}
                )
        out[scenario] = {
            "stock": max(stock),
            "span": max(span),
            "equal": not differences and len(a["steps"]) == len(b["steps"]),
            "differences": differences,
        }
    return out


def main() -> None:
    dirs = [Path(a) for a in sys.argv[1:]] or sorted(
        p
        for p in LOGS.iterdir()
        if p.is_dir() and p.name != "superseded" and (p / "steps.jsonl").exists()
    )
    prices_path = LOGS / "pricing.json"
    prices = json.loads(prices_path.read_text()) if prices_path.exists() else {}
    runs = {}
    for run in dirs:
        summary = run_summary(run, prices)
        summary["_requests"] = sent(run)
        runs[run.name] = summary
    result = {
        "runs": {
            k: {kk: vv for kk, vv in v.items() if kk != "_requests"}
            for k, v in runs.items()
        },
        "latency": latency(runs),
        "inertness": inertness(runs),
    }
    (LOGS / "summary.json").write_text(
        json.dumps(result, indent=1, sort_keys=True) + "\n"
    )
    for name, summary in result["runs"].items():
        print(
            f"## {name}: {summary['wall']} s, {summary['requests']} requests, {summary['points']} points"
        )
        for category, seconds in summary["split"].items():
            share = 100 * seconds / summary["wall"] if summary["wall"] else 0
            print(f"- {category}: {seconds} s ({share:.1f}%)")
    print(json.dumps(result["inertness"], indent=1))
    print(json.dumps(result["latency"], indent=1))


if __name__ == "__main__":
    main()
