"""Print the evidence file's tables from the committed before and after logs alone.

Usage: python3 evidence.py <evidence dir>   (the directory holding `before/` and `after/`)
"""

from __future__ import annotations

import json
import sys
import time
from pathlib import Path

ROOT = Path(sys.argv[1])
BEFORE = json.loads((ROOT / "before/summary.json").read_text())["runs"]
AFTER = json.loads((ROOT / "after/summary.json").read_text())["runs"]
COMBINED = json.loads((ROOT / "before/combined.json").read_text())
ITEMS = 10  # 1 project, 8 tasks, 1 document

# (label, before runs, after run, note on the before)
ROWS = [
    ("test board: first copy", ["tb-first-span-20261018T203458"], "tb-first-span-20261024T095836", ""),
    ("test board: re-copy, every body changed", ["tb-changed-span-20261018T203717"], "tb-changed-span-20261024T095909", ""),
    ("test board: re-copy, one task's body changed", ["tb-changed-span-20261018T203717"], "tb-changed1-span-20261024T101530", "every-body"),
    ("test board: unchanged re-copy", ["tb-unchanged-span-20261018T203848"], "tb-unchanged-span-20261024T095930", ""),
    ("host `copy-plan`: first copy", ["host-copy-stock-20261018T205013"], "host-copy-stock-20261024T100745", ""),
    ("host `copy-plan`: one task's body changed", ["tb-changed-span-20261018T203717"], "host-changed1-stock-20261024T102149", "every-body"),
    ("host `copy-plan`: unchanged re-copy", ["host-recopy-stock-20261018T205156"], "host-recopy-stock-20261024T100859", ""),
    ("host `approve-design`", ["host-approve-stock-20261018T205238"], "host-approve-stock-20261024T100947", ""),
    ("`plans` board: the store's reads of a plan (read-only)", ["plans-walk-stock-20261018T205305", "plans-board-walk-stock-20261018T210253"], "plans-read-stock-20261024T095948", "walks"),
]

PHASES = [
    ("destination reads", ["destination reads"]),
    ("whole-board walk pages", ["whole-board walk pages"]),
    ("writes", ["writes"]),
    ("pacing waits", ["pacing waits"]),
    ("secondary-limit waits", ["secondary-limit waits"]),
    ("other retries (primary-limit waits)", ["primary-limit waits"]),
    ("reference rewriting", ["reference rewriting"]),
    ("body composition", ["body composition and rendering"]),
    ("asset upload", ["asset upload"]),
    ("host-side steps", ["host-side steps"]),
    ("write-boundary screening (`onevcs`)", ["write-boundary screening (onevcs)"]),
    ("engine, local source, CLI", ["engine and local-source work", "CLI (configuration, engine build, output)"]),
    ("unattributed", ["unattributed", "unattributed inside the store process (stock wheel: no spans)"]),
]
SPAN_ONLY = {"reference rewriting", "body composition", "engine, local source, CLI"}


def stock(name: str) -> bool:
    return "-stock-" in name


def seconds(runs: list[dict], keys: list[str]) -> float:
    return sum(r["split"].get(k, 0.0) for r in runs for k in keys)


def cell(runs: list[dict], names: list[str], label: str, keys: list[str]) -> str:
    value = seconds(runs, keys)
    if label in SPAN_ONLY and all(stock(n) for n in names):
        return f"{value:.2f} s‡"
    if label == "unattributed" and any(stock(n) for n in names):
        inside = sum(
            r["split"].get("unattributed inside the store process (stock wheel: no spans)", 0.0)
            for r in runs
        )
        return f"{value:.2f} s ({inside:.2f} s in a stock store process)"
    return f"{value:.2f} s"


def wall(runs: list[dict]) -> float:
    return sum(r["wall"] for r in runs)


def ops(run: dict) -> str:
    parts = []
    for op, entry in sorted(run["by_op"].items()):
        if entry["kind"] in ("screening tunnel",) or op.startswith("CONNECT"):
            continue
        parts.append(f"`{op}` {entry['count']}×/{entry['points']} pt")
    return ", ".join(parts)


def hhmmss(epoch: float) -> str:
    return time.strftime("%H:%M:%S", time.gmtime(epoch))


def phase_table() -> None:
    print("| Scenario | " + " | ".join(p for p, _ in PHASES) + " | wall |")
    print("| --- " * (len(PHASES) + 2) + "|")
    for label, before, after, note in ROWS:
        b = [BEFORE[n] for n in before]
        a = [AFTER[after]]
        cells = []
        for phase, keys in PHASES:
            cells.append(f"{cell(b, before, phase, keys)} → {cell(a, [after], phase, keys)}")
        mark = " §" if note == "every-body" else (" ¶" if note == "walks" else "")
        cells.append(f"**{wall(b):.1f} s → {wall(a):.1f} s**")
        print(f"| {label}{mark} | " + " | ".join(cells) + " |")


def request_table() -> None:
    print("| Scenario | requests and points by operation, before | requests and points by operation, after | requests (per item) | points (per item) | mutation requests (mutations they carried) | content-creating requests (issues created + uploads) |")
    print("| --- | --- | --- | --- | --- | --- | --- |")
    for label, before, after, note in ROWS:
        b = [BEFORE[n] for n in before]
        a = AFTER[after]
        per = ITEMS
        if note == "walks":
            before_ops = "; ".join(f"`{n.split('-stock')[0]}`: " + ops(BEFORE[n]) for n in before)
        else:
            before_ops = ops(b[0])
        breq = sum(r["requests"] for r in b)
        bpts = sum(r["points"] for r in b)
        bmut = sum(r["mutations"] for r in b)
        bcc = sum(r["content_creating"] for r in b)
        if note == "walks":
            records = json.loads((ROOT / "after" / after / "answers.json").read_text())
            n_after = sum(v["records"] for v in records.values())
            reqs = f"{breq} → {a['requests']} ({a['requests'] / n_after:.2f} per record answered, {n_after} records)"
            pts = f"{bpts} → {a['points']}"
        else:
            reqs = f"{breq} ({breq / per:.1f}) → {a['requests']} ({a['requests'] / per:.1f})"
            pts = f"{bpts} ({bpts / per:.1f}) → {a['points']} ({a['points'] / per:.1f})"
        muts = f"{bmut} ({bmut}) → {a['mutations']} ({a['mutation_fields']})"
        cc = f"{bcc} → {a['content_creating']} ({a['issues_created']} + {a['uploads']})"
        mark = " §" if note == "every-body" else (" ¶" if note == "walks" else "")
        print(f"| {label}{mark} | {before_ops} | {ops(a)} | {reqs} | {pts} | {muts} | {cc} |")


def quota_table() -> None:
    print("| Scenario (after run) | GraphQL gate figure: least of the readings (remaining / limit, reset UTC) | `GET /rate_limit` graphql | GraphQL `rateLimit`, three reads | last real answer's `x-ratelimit-*` | core (`GET /rate_limit`) | pauses |")
    print("| --- | --- | --- | --- | --- | --- | --- |")
    for label, _, after, _ in ROWS:
        quota = json.loads((ROOT / "after" / after / "quota.json").read_text())
        readings = [e for e in quota["gate"] if e["event"] == "reading"]
        pauses = [e for e in quota["gate"] if e["event"] == "pause"]
        r = readings[-1]["buckets"]

        def fig(bucket: str) -> str:
            v = r.get(bucket)
            if v is None:
                return "none recorded"
            reads = f"; reads {', '.join(str(x) for x in v['readings'])}" if "readings" in v else ""
            return f"{v['remaining']} / {v['limit']} (reset {hhmmss(v['reset'])}){reads}"

        at = hhmmss(readings[-1]["at"])
        paused = "none recorded" if not pauses else "; ".join(
            f"{hhmmss(p['at'])}–{hhmmss(p['at'] + p['seconds'])} on {', '.join(p['buckets'])}" for p in pauses
        )
        print(f"| {label} (`{after}`, read {at}) | {fig('graphql')} | {fig('graphql (GET /rate_limit)')} | {fig('graphql (rateLimit)')} | {fig('graphql (last headers)')} | {fig('core')} | {paused} |")


def combined_table() -> None:
    bars = {
        "first": (32, "12 points, 40 writes", "github-plan-copy-seconds-first"),
        "changed": (8, "4 points", "github-plan-copy-seconds-changed"),
        "unchanged": (7, "3 points", "github-plan-copy-seconds-unchanged"),
        "approval": (7, "5 points", "plan-approval-seconds"),
    }
    plan = COMBINED["plans"]
    rows = [
        ("first copy, bare store", "first", "tb-first-span-20261018T203458", "tb-first-span-20261024T095836", 1),
        ("first copy, host `copy-plan`", "first", "host-copy-stock-20261018T205013", "host-copy-stock-20261024T100745", 1),
        ("re-copy, one body changed, bare store", "changed", "tb-changed-span-20261018T203717", "tb-changed1-span-20261024T101530", 1),
        ("re-copy, one body changed, host `copy-plan`", "changed", "tb-changed-span-20261018T203717", "host-changed1-stock-20261024T102149", 1),
        ("re-copy, every body changed, bare store (heavier case)", "changed", "tb-changed-span-20261018T203717", "tb-changed-span-20261024T095909", 1),
        ("unchanged re-copy, bare store", "unchanged", "tb-unchanged-span-20261018T203848", "tb-unchanged-span-20261024T095930", 1),
        ("unchanged re-copy, host `copy-plan`", "unchanged", "host-recopy-stock-20261018T205156", "host-recopy-stock-20261024T100859", 1),
        ("approval, host `approve-design`", "approval", "host-approve-stock-20261018T205238", "host-approve-stock-20261024T100947", plan["approval_listing_processes"]),
    ]
    print("| Figure | before at `plans` size (spike's arithmetic) | after at `plans` size (same arithmetic) | seconds bar | within? | points / requests / writes after | quota bar | within? |")
    print("| --- | --- | --- | --- | --- | --- | --- | --- |")
    for label, kind, before, after, scale in rows:
        b = COMBINED["scenarios"][before]
        a = AFTER[after]
        reqs = json.loads(json.dumps(a["by_op"]))
        boards = reqs.get("BOARD", {}).get("count", 0)
        searches = reqs.get("SEARCH_ISSUES", {}).get("count", 0)
        walk = sum(reqs.get(op, {}).get("seconds", 0.0) for op in ("BOARD", "SEARCH_ISSUES"))
        bp = boards * plan["board_pages_per_walk"] * scale
        sp = searches * plan["search_pages_per_walk"] * scale
        total = a["wall"] - walk + bp * plan["board_page_seconds"] + sp * plan["search_page_seconds"]
        arithmetic = (
            f"{a['wall']:.1f} s measured − {walk:.1f} s of {boards} BOARD + {searches} SEARCH test-board pages "
            f"+ {bp} × {plan['board_page_seconds']:.2f} s + {sp} × {plan['search_page_seconds']:.2f} s = **{total:.1f} s**"
        )
        bar, quota_bar, _ = bars[kind]
        writes = a["mutation_fields"] + a["uploads"]
        pts_bar = int(quota_bar.split()[0])
        quota_ok = a["points"] <= pts_bar and (kind != "first" or writes <= 40)
        print(
            f"| {label} | {b['arithmetic']}; {b['points']} points, {b['requests']} requests (`{before}`) | {arithmetic} (`{after}`) | {bar} s | "
            f"{'within' if total <= bar else 'over'} ({total:.1f} s) | {a['points']} points / {a['requests']} requests / {writes} writes "
            f"({a['mutations']} mutation requests carrying {a['mutation_fields']} mutations, {a['uploads']} upload) | {quota_bar} | "
            f"{'within' if quota_ok else 'over'} |"
        )


def latency_table() -> None:
    after = json.loads((ROOT / "after/summary.json").read_text())["latency"]
    before = json.loads((ROOT / "before/summary.json").read_text())["latency"]
    print("| Request kind | before: n, median / mean ms | after: n, median / mean ms |")
    print("| --- | --- | --- |")
    for kind in sorted(set(before) | set(after)):
        def fmt(table: dict) -> str:
            v = table.get(kind)
            return f"{v['n']}, {v['median_ms']} / {v['mean_ms']}" if v else "0 sent"
        print(f"| {kind} | {fmt(before)} | {fmt(after)} |")


def writes_table() -> None:
    """Every write this run sent, by destination and by what sent it, from every committed
    after log (superseded runs included)."""
    paths = sorted((ROOT / "after").glob("*/requests.jsonl"))
    paths += sorted((ROOT / "after/superseded").glob("*/requests.jsonl"))
    rows: dict[tuple[str, str], dict] = {}
    created = rolled_back = 0
    for path in paths:
        name = path.parent.name
        superseded = path.parent.parent.name == "superseded"
        requests = [json.loads(line) for line in path.read_text().splitlines() if line.strip()]
        sent = [r for r in requests if r["host"] != "refused"]
        if name.startswith("plans-"):
            key = ("`plans` (ai-orchestrator's real board)", "the read-only timing")
        elif name.startswith("cleanup-"):
            key = ("onetaskgraph's test board", "cleanup (outside the copy scenarios)")
        elif superseded:
            key = ("onetaskgraph's test board", "copy scenarios, superseded runs (refused attempts and re-measured runs)")
        else:
            key = ("onetaskgraph's test board", "copy and approval scenarios the figures are computed from")
        entry = rows.setdefault(key, {"runs": 0, "mutation requests": 0, "mutations": 0, "uploads": 0, "refused": 0, "created": 0})
        entry["runs"] += 1
        entry["mutation requests"] += sum(1 for r in sent if r["kind"] == "mutation")
        entry["mutations"] += sum(r.get("mutations", 1) for r in sent if r["kind"] == "mutation")
        entry["uploads"] += sum(1 for r in sent if r["op"] == "POST /user-attachments/assets")
        entry["refused"] += sum(1 for r in requests if r["host"] == "refused")
        made = sum(r.get("mutations", 1) for r in sent if r["op"] in ("CREATE_ISSUE", "CREATE_ISSUES"))
        entry["created"] += made
        created += made
        rolled_back += sum(1 for r in sent if r["op"] == "DELETE_ISSUE")
    print("| Destination | sent by | runs | mutation requests | mutations they carried | uploads | issues created | requests the proxy refused |")
    print("| --- | --- | --- | --- | --- | --- | --- | --- |")
    for (destination, by), e in sorted(rows.items()):
        print(f"| {destination} | {by} | {e['runs']} | {e['mutation requests']} | {e['mutations']} | {e['uploads']} | {e['created']} | {e['refused']} |")
    print("| `followups` (ai-orchestrator) | nothing: its endpoint was an address that answers nothing | 0 | 0 | 0 | 0 | 0 | 0 |")
    deleted = sum(json.loads(p.read_text())["deleted"] for p in sorted((ROOT / "after").glob("cleanup-*/cleanup.json")))
    gone = [json.loads(p.read_text())["gone"] for p in sorted((ROOT / "after").glob("cleanup-*/cleanup.json"))]
    print()
    print(f"ISSUES: created {created}; taken back by the store inside its refused copies {rolled_back}; deleted by cleanup {deleted}; cleanup found already gone {gone}")


if __name__ == "__main__":
    for name, fn in [("PHASES", phase_table), ("REQUESTS", request_table), ("QUOTA", quota_table), ("COMBINED", combined_table), ("LATENCY", latency_table), ("WRITES", writes_table)]:
        print(f"<!-- {name} -->")
        fn()
        print()
