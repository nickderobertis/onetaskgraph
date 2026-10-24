"""What the spike itself consumed, from the committed logs alone: every request every run
sent through the proxy — measurements, the superseded runs under `logs/superseded/`,
cleanups and probes alike — priced as `analyze.py` prices them, plus every quota reading
(each `GET /rate_limit` and each `rateLimit` query its own request) and the pricing dry
runs, and the two direct requests no log recorded, which are counted by hand. The write
audit alone reads the raw request bodies in the cache.

`python3 consumption.py` writes `spike/logs/consumption.json` (numbers and run names only).
"""

from __future__ import annotations

import json
import os
from pathlib import Path

HERE = Path(__file__).resolve().parent
LOGS = HERE.parent / "logs"
CACHE = Path(os.environ.get("XDG_CACHE_HOME") or Path.home() / ".cache") / (
    "board-copy-speed-2026-10-18"
)
CONTENT_CREATING = {"CREATE_ISSUE", "CreateWithProjectV2Ids", "CreateWithParentIssueId"}
CONTENT_CREATING |= {"CreatePlain", "POST /user-attachments/assets"}
# `probe-body-limit`'s two `createIssue` attempts; GitHub refused the second
# (`UNPROCESSABLE`, body too long), but it was sent, and a content-creating request is
# counted by what was sent.
CONTENT_CREATING |= {"CreateWide77KB", "CreateAscii66000"}


ISSUE_KEYS = (
    "id",
    "issueId",
    "subIssueId",
    "contentId",
    "blockingIssueId",
    "subjectId",
)


def ids_under(value: object, keys: tuple[str, ...]) -> list[str]:
    found: list[str] = []
    if isinstance(value, dict):
        for key, inner in value.items():
            if key in keys and isinstance(inner, str):
                found.append(inner)
            elif key in keys and isinstance(inner, list):
                found += [v for v in inner if isinstance(v, str)]
            else:
                found += ids_under(inner, keys)
    elif isinstance(value, list):
        for inner in value:
            found += ids_under(inner, keys)
    return found


def audit_writes(runs: Path) -> dict:
    """Every mutation any run sent, by what it named: the test board's project, the scratch
    repository, the core repository (the first two runs only), anything else; and whether
    every issue a mutation named was one a `createIssue` of this spike answered with."""
    # `run.board_ids` keeps the scratch repository's id first and the board's second.
    scratch, board = json.loads(
        (runs.parent / "state" / "test-board-ids.json").read_text()
    )
    created: set[str] = set()
    items: set[str] = set()
    named: dict[str, int] = {"test board project": 0, "scratch repository": 0}
    named |= {"core repository": 0, "other repository or project": 0}
    issue_ids: list[str] = []
    for log in sorted(runs.glob("*/proxy.jsonl")):
        bodies = log.with_suffix(".bodies")
        for line in log.read_text().splitlines():
            record = json.loads(line)
            if record["kind"] != "mutation" or record["host"] == "refused":
                continue
            request = json.loads((bodies / f"{record['seq']:06d}.req").read_text())
            answer = json.loads(
                (bodies / f"{record['seq']:06d}.resp").read_text() or "{}"
            )
            for value in (answer.get("data") or {}).values():
                if isinstance(value, dict):
                    issue = value.get("issue")
                    if isinstance(issue, dict) and "createIssue" in request["query"]:
                        created.add(issue["id"])
                    item = value.get("item") or value.get("projectV2Item")
                    if isinstance(item, dict):
                        items.add(item["id"])
            for key_id in ids_under(
                request.get("variables"), ("projectId", "projectV2Ids")
            ):
                named[
                    "test board project"
                    if key_id == board
                    else "other repository or project"
                ] += 1
            for repo_id in ids_under(request.get("variables"), ("repositoryId",)):
                if repo_id == scratch:
                    named["scratch repository"] += 1
                elif repo_id == "R_kgDOUBBgxg":
                    named["core repository"] += 1
                else:
                    named["other repository or project"] += 1
            issue_ids += ids_under(request.get("variables"), ISSUE_KEYS + ("itemId",))
    foreign = [i for i in issue_ids if i not in created and i not in items]
    return {
        "mutations_naming": named,
        "issue_or_item_ids_named": len(issue_ids),
        "named_ids_this_spike_did_not_create": len(foreign),
    }


def rate_limit_queries(buckets: dict) -> int:
    """How many GraphQL `rateLimit` queries one quota reading sent: the gate's reads
    before `OWN_READS` kept no list and sent one; every later reading lists each one."""
    own = buckets.get("graphql (rateLimit)")
    if own is None:
        return 0
    return len(own.get("readings", [None]))


def run_dirs(logs: Path) -> list[Path]:
    """Every committed run: the measurements beside the reports, and `superseded/`."""
    dirs = [p for p in logs.iterdir() if p.is_dir() and p.name != "superseded"]
    return sorted(dirs + [p for p in (logs / "superseded").iterdir() if p.is_dir()])


def run_figures(run: Path, prices: dict) -> dict:
    records = [
        json.loads(line)
        for line in (run / "requests.jsonl").read_text().splitlines()
        if line.strip()
    ]
    sent = [r for r in records if r["host"] != "refused" and r["kind"] != "tunnel"]
    wall = 0.0
    steps = run / "steps.jsonl"
    if steps.exists():
        for line in steps.read_text().splitlines():
            step = json.loads(line)
            wall += step["end"] - step["start"]
    points = 0
    creating = 0
    for record in sent:
        op = record["op"]
        if record["kind"] in ("query", "mutation"):
            points += int(prices.get(op, {}).get("points", 1))
        if op in CONTENT_CREATING:
            creating += 1
        if op == "Aliased":
            creating += 3  # its three aliased createIssue mutations
    return {
        "requests": len(sent),
        "refused_by_proxy": sum(1 for r in records if r["host"] == "refused"),
        "tunnels": sum(1 for r in records if r["kind"] == "tunnel"),
        "points": points,
        "content_creating": creating,
        "mutations": sum(1 for r in sent if r["kind"] == "mutation"),
        "wall_seconds": round(wall, 1),
    }


def quota_figures(logs: Path, dirs: list[Path]) -> dict:
    """Every quota reading: the gate's (`quota-readings.jsonl`, whose last reading per run
    is that run's `quota-before`) and each run's `quota-after`, which the gate never saw.
    A reading is one free `GET /rate_limit` and its `rateLimit` queries, one point each
    (GitHub's least cost of a GraphQL query; consecutive readings never moved the bucket,
    so this is an upper bound)."""
    gate = [
        json.loads(line)
        for line in (logs / "quota-readings.jsonl").read_text().splitlines()
        if line.strip()
    ]
    gate = [q for q in gate if q["event"] == "reading"]
    after = []
    for run in dirs:
        quota = json.loads((run / "quota.json").read_text())
        if "quota-after" in quota:
            after.append(quota["quota-after"])
    readings = gate + after
    return {
        "gate readings": len(gate),
        "after-run readings": len(after),
        "GET /rate_limit": len(readings),
        "GraphQL rateLimit": sum(rate_limit_queries(q["buckets"]) for q in readings),
    }


def main() -> None:
    prices = json.loads((LOGS / "pricing.json").read_text())
    dirs = run_dirs(LOGS)
    runs = {run.name: run_figures(run, prices) for run in dirs}
    quota = quota_figures(LOGS, dirs)
    dry_runs = sum(1 for v in prices.values() if v["method"] == "dry run")
    # Requests no proxy or quota log saw, so counted by hand; the four diagnosing reads'
    # split between REST and GraphQL was not recorded, so all four are priced as GraphQL.
    hand = {
        "GraphQL id lookup of the test board and its repository": 1,
        "GET /rate_limit and GraphQL rateLimit by hand while diagnosing": 4,
    }
    direct = {
        "GET /rate_limit (quota readings, free)": quota["GET /rate_limit"],
        "GraphQL rateLimit (quota readings, one request per query)": quota[
            "GraphQL rateLimit"
        ],
        "GraphQL dry runs (pricing.py), one per priced query": dry_runs,
    } | hand
    totals = {
        key: sum(run[key] for run in runs.values())
        for key in (
            "requests",
            "points",
            "content_creating",
            "mutations",
            "wall_seconds",
        )
    }
    totals["wall_seconds"] = round(totals["wall_seconds"], 1)
    totals["direct_requests"] = sum(direct.values())
    totals["direct_points"] = quota["GraphQL rateLimit"] + dry_runs + sum(hand.values())
    totals["hand_counted_requests"] = sum(hand.values())
    totals["all_requests"] = totals["requests"] + totals["direct_requests"]
    totals["all_points"] = totals["points"] + totals["direct_points"]
    out = {
        "runs": runs,
        "quota_readings": quota,
        "direct": direct,
        "totals": totals,
        "writes": audit_writes(CACHE / "runs"),
    }
    (LOGS / "consumption.json").write_text(
        json.dumps(out, indent=1, sort_keys=True) + "\n"
    )
    print(json.dumps({"totals": totals, "writes": out["writes"]}, indent=1))


if __name__ == "__main__":
    main()
