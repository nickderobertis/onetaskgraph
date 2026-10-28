"""Recompute every table of `board-copy-targets-2026-10-26.md` from the sanitized captures.

    python3 docs/measurements/board-copy-targets-2026-10-26/analyze.py           # check
    python3 docs/measurements/board-copy-targets-2026-10-26/analyze.py --write   # rewrite

Each table of the report sits between `<!-- analyze:begin NAME -->` and `<!-- analyze:end NAME -->`.
The default run recomputes every one from `captures/` and exits 1, naming the table, when the
report's copy differs from what the captures say; `--write` puts the recomputed tables in place.
Both write `captures/summary.json`, every figure the tables are made from.

A row is the latest capture of its scenario (`captures/<scenario>-<UTC stamp>/`); a capture moved
to `captures/superseded/` is never read as a row. The targets and the before figures are the
brief's, stated once below, and no figure here is compared against anything else.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from collections import Counter, defaultdict
from dataclasses import dataclass, field
from pathlib import Path

HERE = Path(__file__).resolve().parent
CAPTURES = HERE / "captures"
REPORT = HERE.parent / "board-copy-targets-2026-10-26.md"

# The user's targets, as the brief states them. Seconds warn on this host; points and writes
# are strict.
TARGETS: dict[str, dict[str, float]] = {
    "first": {"points": 12, "writes": 40, "seconds": 32},
    "changed1": {"points": 4, "seconds": 8},
    "unchanged": {"points": 3, "seconds": 7},
    "approve": {"points": 5, "seconds": 7},
}
# The brief's before figures: the host journey's copies on the loopback board at onetaskgraph
# 0.12.3. The brief gives none for an approval or for writes.
BEFORE: dict[str, dict[str, float]] = {
    "first": {"seconds": 51, "points": 25},
    "changed1": {"seconds": 20.8, "points": 18},
    "unchanged": {"seconds": 17.4, "points": 14},
}
# The same operations measured on GitHub at onetaskgraph 0.11.0 on 2026-10-24
# (`board-copy-speed-2026-10-18.md`, combined table): seconds, points, writes.
EARLIER: dict[str, dict[str, float]] = {
    "bare-first": {"seconds": 29.8, "points": 20, "writes": 36},
    "bare-changed1": {"seconds": 8.7, "points": 7, "writes": 1},
    "bare-unchanged": {"seconds": 6.7, "points": 6, "writes": 0},
    "host-first": {"seconds": 64.4, "points": 17, "writes": 25},
    "host-changed1": {"seconds": 52.2, "points": 7, "writes": 2},
    "host-unchanged": {"seconds": 44.7, "points": 4, "writes": 0},
    "host-approve": {"seconds": 16.4, "points": 7, "writes": 1},
}
ROWS = (
    ("bare-first", "first", "bare store: first copy"),
    ("bare-changed1", "changed1", "bare store: re-copy, one task's body changed"),
    ("bare-unchanged", "unchanged", "bare store: unchanged re-copy"),
    ("host-first", "first", "host `just copy-plan`: first copy"),
    ("host-changed1", "changed1", "host `just copy-plan`: one task's body changed"),
    ("host-unchanged", "unchanged", "host `just copy-plan`: unchanged re-copy"),
    ("host-approve", "approve", "host `just approve-design`"),
)
# The reference-size arithmetic of the timing spike (`board-copy-speed-2026-10-18.md`): every
# walk page the test board answered is replaced by the pages the same walk takes on a board of
# the `plans` board's size, at the latencies measured there.
WALK_PAGE_SECONDS = {"BOARD": 6.95, "SEARCH_ISSUES": 6.79}
WALK_PAGES_AT_REFERENCE = {"BOARD": 20, "SEARCH_ISSUES": 10}
COPY_VERBS = (("project", "copy"), ("document", "copy"))
# A mutation request's primary price, by the manager's ruling: GitHub's dry run cannot price a
# mutation, and the smallest rise of `x-ratelimit-used` observed before one is its evidence.
MUTATION_PRICE = 1
UPLOAD = "POST /user-attachments/assets"


def lines(path: Path) -> list[dict]:
    if not path.exists():
        return []
    return [json.loads(line) for line in path.read_text().splitlines() if line.strip()]


def latest_captures(captures: Path) -> dict[str, Path]:
    """The latest capture of every scenario: `<scenario>-<YYYYmmddTHHMMSS>` directories."""
    found: dict[str, Path] = {}
    for path in sorted(captures.iterdir()) if captures.exists() else []:
        match = re.fullmatch(r"(.+)-(\d{8}T\d{6})", path.name)
        if path.is_dir() and match:
            found[match.group(1)] = path
    return found


# --- points ---------------------------------------------------------------------------------


@dataclass
class Points:
    """A row's GraphQL primary points: GitHub's own price of every GraphQL request it sent, with
    `rateLimit` read around it and the idle background that reading carries beside it."""

    figure: int
    queries: int
    mutation_requests: int
    at_five: int
    counter: int | None
    before_used: int | None
    after_used: int | None
    delta: int | None
    background_rate: float | None
    background: float | None
    residual: float | None


def price(request: dict, prices: dict[str, int], capture: str) -> int:
    """One GraphQL request's primary points: a query its own dry-run price, a mutation request
    `MUTATION_PRICE` (which the dry run cannot price)."""
    if request["kind"] == "mutation":
        return MUTATION_PRICE
    key = f"{capture}/{request['seq']}"
    if key not in prices:
        raise SystemExit(
            f"analyze.py: no dry-run price for {key} ({request['op']}); run harness/pricing.py"
        )
    return prices[key]


def counter_of(before: list[dict], after: list[dict]) -> int | None:
    """The counter (by its reset) both idle samples read, the later-resetting if two did."""
    shared = {r["reset"] for r in before} & {r["reset"] for r in after}
    return max(shared) if shared else None


def slope(readings: list[dict]) -> tuple[float, float]:
    """The rise of `used` across one counter's readings, and the seconds it took."""
    if len(readings) < 2:
        return 0.0, 0.0
    return float(readings[-1]["used"] - readings[0]["used"]), readings[-1][
        "at"
    ] - readings[0]["at"]


def points_of(
    quota: dict, requests: list[dict], prices: dict[str, int], capture: str
) -> Points:
    graphql = [
        r for r in requests if r["path"] == "/graphql" and r["host"] != "refused"
    ]
    figure = sum(price(r, prices, capture) for r in graphql)
    mutations = sum(1 for r in graphql if r["kind"] == "mutation")
    before, after = quota.get("burn-before", []), quota.get("burn-after", [])
    counter = counter_of(before, after)
    if counter is None:
        return Points(
            figure,
            len(graphql) - mutations,
            mutations,
            figure + mutations * 4,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
    b = [r for r in before if r["reset"] == counter]
    a = [r for r in after if r["reset"] == counter]
    rise_b, time_b = slope(b)
    rise_a, time_a = slope(a)
    rate = (rise_b + rise_a) / (time_b + time_a) if time_b + time_a else 0.0
    delta = a[0]["used"] - b[-1]["used"]
    background = rate * (a[0]["at"] - b[-1]["at"])
    return Points(
        figure=figure,
        queries=len(graphql) - mutations,
        mutation_requests=mutations,
        at_five=figure + mutations * 4,
        counter=counter,
        before_used=b[-1]["used"],
        after_used=a[0]["used"],
        delta=delta,
        background_rate=round(rate, 3),
        background=round(background, 1),
        residual=round(delta - figure - background, 1),
    )


# --- writes, requests, screening --------------------------------------------------------------


def writes_of(requests: list[dict]) -> dict[str, int]:
    """Individual mutations (each aliased one counted) plus uploads, and the requests carrying them."""
    sent = [r for r in requests if r["host"] != "refused"]
    mutations = [r for r in sent if r["kind"] == "mutation"]
    uploads = [r for r in sent if r["op"] == UPLOAD]
    carried = sum(r.get("mutations", 1) for r in mutations)
    created = sum(
        r.get("mutations", 1)
        for r in mutations
        if r["op"] in ("CREATE_ISSUE", "CREATE_ISSUES")
    )
    return {
        "requests": len([r for r in sent if r["kind"] != "tunnel"]),
        "issues_created": created,
        "mutation_requests": len(mutations),
        "mutations": carried,
        "uploads": len(uploads),
        "writes": carried + len(uploads),
    }


def kinds_of(
    requests: list[dict], prices: dict[str, int], capture: str
) -> dict[str, dict[str, int]]:
    """Every request kind a row sent: how many, the points and the writes they carried."""
    out: dict[str, dict[str, int]] = defaultdict(
        lambda: {"requests": 0, "points": 0, "writes": 0}
    )
    for r in requests:
        if r["kind"] == "tunnel" or r["host"] == "refused":
            continue
        entry = out[r["op"]]
        entry["requests"] += 1
        if r["path"] == "/graphql":
            entry["points"] += price(r, prices, capture)
        if r["kind"] == "mutation":
            entry["writes"] += r.get("mutations", 1)
        if r["op"] == UPLOAD:
            entry["writes"] += 1
    return dict(sorted(out.items()))


def screening_of(screen: list[dict]) -> dict[str, float]:
    counts = Counter(r["name"] for r in screen)
    return {
        "inspect": counts.get("inspect", 0),
        "check": counts.get("check", 0),
        "seconds": round(sum(r["end"] - r["start"] for r in screen), 3),
    }


# --- seconds ---------------------------------------------------------------------------------

# Causes in the order an instant is given to them when several are under way at once: a
# screening call runs inside a store invocation, and so does every round trip.
PRIORITY = (
    "write screening",
    "network",
    "store",
    "subprocess",
    "step",
    "python",
    "launch",
)


@dataclass
class Interval:
    start: float
    end: float
    priority: int
    label: str


def store_label(argv: list[str]) -> str:
    words = [
        w
        for w in argv
        if not w.startswith("<") and not w.startswith("--") and ":" not in w
    ]
    return "store invocation: " + " ".join(words[:2] or ["(unnamed)"])


def intervals_of(capture: dict) -> tuple[list[Interval], float, float]:
    """Every cause's interval in a row, with the row's measured start and end."""
    steps = capture["steps"]
    start, end = min(s["start"] for s in steps), max(s["end"] for s in steps)
    rank = {name: index for index, name in enumerate(PRIORITY)}
    out: list[Interval] = []
    for r in capture["screen"]:
        out.append(
            Interval(
                r["start"],
                r["end"],
                rank["write screening"],
                f"write screening: onevcs boundary {r['name']}",
            )
        )
    for r in capture["requests"]:
        if r["host"] == "refused":
            continue
        label = (
            "network round trips: GitHub GraphQL"
            if r["path"] == "/graphql"
            else (
                "network round trips: tunnelled HTTPS (the image upload's storage and its check)"
                if r["kind"] == "tunnel"
                else "network round trips: GitHub REST and uploads"
            )
        )
        out.append(Interval(r["start"], r["end"], rank["network"], label))
    host = capture["host"]
    if host:
        interpreters = [h for h in host if h["kind"] == "interpreter"]
        first = min(
            (h["start"] for h in interpreters if h["name"] == "start"), default=start
        )
        last = max(
            (h["start"] for h in interpreters if h["name"] == "exit"), default=end
        )
        events = [h for h in host if h["kind"] in ("store", "subprocess", "step")]
        first_event = min((h["start"] for h in events), default=first)
        out.append(
            Interval(
                start,
                first,
                rank["launch"],
                "recipe launch: just, plan-store.sh and uv run",
            )
        )
        out.append(
            Interval(first, first_event, rank["launch"], "interpreter and SDK start-up")
        )
        out.append(
            Interval(
                first_event, last, rank["python"], "host Python outside the named steps"
            )
        )
        out.append(
            Interval(last, end, rank["launch"], "interpreter exit and recipe teardown")
        )
        for h in host:
            if h["kind"] == "store":
                out.append(
                    Interval(
                        h["start"], h["end"], rank["store"], store_label(h["argv"])
                    )
                )
            elif h["kind"] == "subprocess":
                out.append(
                    Interval(
                        h["start"],
                        h["end"],
                        rank["subprocess"],
                        f"host subprocess: {h['program']}",
                    )
                )
            elif h["kind"] == "step":
                out.append(
                    Interval(
                        h["start"], h["end"], rank["step"], f"host step: {h['name']}"
                    )
                )
    else:
        for s in steps:
            out.append(
                Interval(
                    s["start"],
                    s["end"],
                    rank["store"],
                    "store invocation: " + " ".join(s["argv"][:2]),
                )
            )
    return out, start, end


def attribute(intervals: list[Interval], start: float, end: float) -> dict[str, float]:
    """Split [start, end] among the causes: each instant goes to the highest-priority cause under
    way then, the innermost (latest-starting) of equal ones; an instant none covers is
    `unattributed`. The shares therefore sum to the measured total exactly."""
    points = sorted(
        {
            start,
            end,
            *(t for i in intervals for t in (i.start, i.end) if start < t < end),
        }
    )
    shares: dict[str, float] = defaultdict(float)
    for a, b in zip(points, points[1:]):
        middle = (a + b) / 2
        live = [i for i in intervals if i.start <= middle < i.end]
        if live:
            chosen = min(live, key=lambda i: (i.priority, -i.start))
            shares[chosen.label] += b - a
        else:
            shares["unattributed"] += b - a
    return dict(shares)


def rounded(
    shares: dict[str, float], total: float, places: int = 2
) -> dict[str, float]:
    """`shares` rounded so that they still sum to `total` rounded the same way (largest
    remainder), which is how a table of them stays a true decomposition of its total."""
    unit = 10**places
    target = round(total * unit)
    floors = {k: int(v * unit) for k, v in shares.items()}
    short = target - sum(floors.values())
    order = sorted(shares, key=lambda k: shares[k] * unit - floors[k], reverse=True)
    for key in order[: max(short, 0)]:
        floors[key] += 1
    # Causes that round to nothing are named together rather than listed at 0.00 s each.
    small = sorted(k for k, v in floors.items() if v == 0)
    out = {
        k: v / unit
        for k, v in sorted(floors.items(), key=lambda kv: (-kv[1], kv[0]))
        if v
    }
    if small:
        out[f"{len(small)} further causes under 0.005 s each ({', '.join(small)})"] = (
            0.0
        )
    return out


# --- a row ------------------------------------------------------------------------------------


@dataclass
class Row:
    scenario: str
    operation: str
    label: str
    capture: str
    seconds: float
    points: Points
    writes: dict[str, int]
    kinds: dict[str, dict[str, int]]
    screening: dict[str, float]
    attribution: dict[str, float]
    owners: dict[str, dict[str, int]]
    versions: dict
    conditions: dict
    cache: dict
    workload: dict
    quota: dict
    walk_pages: dict[str, int]
    walk_ms: float
    refused: int
    returncodes: list[int]
    verdicts: dict[str, str] = field(default_factory=dict)


def load(path: Path) -> dict:
    def optional(name: str) -> dict:
        target = path / name
        return json.loads(target.read_text()) if target.exists() else {}

    return {
        "requests": lines(path / "requests.jsonl"),
        "steps": lines(path / "steps.jsonl"),
        "host": lines(path / "host.jsonl"),
        "screen": lines(path / "screen.jsonl"),
        "quota": optional("quota.json"),
        "versions": optional("versions.json"),
        "conditions": optional("conditions.json"),
        "cache-before": optional("cache-before.json"),
        "cache-after": optional("cache-after.json"),
        "workload": optional("workload.json"),
    }


def owners_of(
    capture: dict, prices: dict[str, int], name: str
) -> dict[str, dict[str, int]]:
    """Each request kind's points and writes by who sent it: a request made inside the store's
    own `project copy` or `document copy` is the store's; one inside any other store invocation
    the host recipe made is the host's (it chose to make that call)."""
    calls = capture["host"] or [
        {"kind": "store", "argv": s["argv"], "start": s["start"], "end": s["end"]}
        for s in capture["steps"]
    ]
    stores = [c for c in calls if c["kind"] == "store"]
    out: dict[str, dict[str, int]] = defaultdict(
        lambda: {"points": 0, "writes": 0, "requests": 0}
    )
    for r in capture["requests"]:
        if r["kind"] == "tunnel" or r["host"] == "refused":
            continue
        inside = [c for c in stores if c["start"] <= r["start"] <= c["end"]]
        verb = (
            store_label(inside[-1]["argv"]).removeprefix("store invocation: ")
            if inside
            else "(no store invocation)"
        )
        owner = "store" if tuple(verb.split()[:2]) in COPY_VERBS else "host"
        key = f"{owner} | {verb} | {r['op']}"
        out[key]["requests"] += 1
        if r["path"] == "/graphql":
            out[key]["points"] += price(r, prices, name)
        if r["kind"] == "mutation":
            out[key]["writes"] += r.get("mutations", 1)
        if r["op"] == UPLOAD:
            out[key]["writes"] += 1
    return dict(sorted(out.items()))


def row_of(
    scenario: str, operation: str, label: str, path: Path, prices: dict[str, int]
) -> Row:
    capture = load(path)
    steps = capture["steps"]
    seconds = round(sum(s["end"] - s["start"] for s in steps), 3)
    intervals, start, end = intervals_of(capture)
    # A bare row's two commands are timed back to back; the gap between them is the harness's.
    total = seconds
    shares = attribute(intervals, start, end)
    if len(steps) > 1:
        gaps = sum(b["start"] - a["end"] for a, b in zip(steps, steps[1:]))
        shares["unattributed"] = shares.get("unattributed", 0.0) - gaps
        if abs(shares["unattributed"]) < 1e-6:
            del shares["unattributed"]
    walk = Counter(r["op"] for r in capture["requests"] if r["op"] in WALK_PAGE_SECONDS)
    row = Row(
        scenario=scenario,
        operation=operation,
        label=label,
        capture=path.name,
        seconds=seconds,
        points=points_of(capture["quota"], capture["requests"], prices, path.name),
        writes=writes_of(capture["requests"]),
        kinds=kinds_of(capture["requests"], prices, path.name),
        screening=screening_of(capture["screen"]),
        attribution=rounded(shares, total),
        owners=owners_of(capture, prices, path.name),
        versions=capture["versions"],
        conditions=capture["conditions"],
        cache={"before": capture["cache-before"], "after": capture["cache-after"]},
        workload=capture["workload"],
        quota=capture["quota"],
        walk_pages={op: walk.get(op, 0) for op in WALK_PAGE_SECONDS},
        walk_ms=sum(
            r["ms"] for r in capture["requests"] if r["op"] in WALK_PAGE_SECONDS
        ),
        refused=sum(1 for r in capture["requests"] if r["host"] == "refused"),
        returncodes=[s["returncode"] for s in steps],
    )
    target = TARGETS[operation]
    figures = {
        "points": row.points.figure,
        "seconds": row.seconds,
        "writes": row.writes["writes"],
    }
    row.verdicts = {
        name: ("within" if figures[name] <= bar else "over")
        for name, bar in target.items()
    }
    row.verdicts["points at 5 per mutation"] = (
        "within" if row.points.at_five <= target["points"] else "over"
    )
    return row


def reference_seconds(row: Row, walked_ms: float) -> float:
    """The row's seconds at the `plans` board's size: every walk page the test board answered
    replaced by the pages the same walk takes there. With no walk page it is the measured figure."""
    if not any(row.walk_pages.values()):
        return row.seconds
    extra = sum(
        WALK_PAGES_AT_REFERENCE[op] * WALK_PAGE_SECONDS[op]
        for op, n in row.walk_pages.items()
        if n
    )
    return round(row.seconds - walked_ms / 1000 + extra, 1)


# --- the board-size evidence ------------------------------------------------------------------


def loopback(captures: Path) -> list[dict]:
    """Each loopback telemetry recording of the adopted release: per scenario and board size, the
    requests by operation its detail carries."""
    out = []
    for path in sorted((captures / "loopback").glob("*.json")):
        record = json.loads(path.read_text())
        detail = record.get("detail")
        detail = json.loads(detail) if isinstance(detail, str) else detail or {}
        out.append(
            {
                "file": path.name,
                "budget": record.get("budget"),
                "value": record.get("value"),
                "detail": detail,
            }
        )
    return out


SIZES = (
    ("unscreened", "unscreened, on a board of 20,000 items"),
    ("screened", "screened, on a board of 20,000 items"),
    (
        "host configuration, private source",
        "host configuration, private source, on a board of 20,000 items",
    ),
    (
        "host configuration, public source",
        "host configuration, public source, on a board of 20,000 items",
    ),
)
# What the plugin calls a page of a whole-board walk or of an issue search, in the loopback
# recordings' operation names.
WALK_WORDS = ("page of the board", "board's items", "search")


def size_pairs(recordings: list[dict]) -> list[dict]:
    """Every loopback points recording on the board of 2,000 items beside its twin on 20,000:
    whether the two sent the same requests by operation, their points, and the walk pages."""
    pairs = []
    for record in recordings:
        if not str(record["budget"]).startswith("github-plan-copy-points-"):
            continue
        for small, large in SIZES:
            a, b = record["detail"].get(small), record["detail"].get(large)
            if not isinstance(a, dict) or not isinstance(b, dict):
                continue
            sent = a.get("requests by operation", {})
            pairs.append(
                {
                    "budget": record["budget"],
                    "recording": small,
                    "same": sent == b.get("requests by operation")
                    and a.get("points") == b.get("points"),
                    "requests": [
                        sum(sent.values()),
                        sum(b.get("requests by operation", {}).values()),
                    ],
                    "points": [a.get("points"), b.get("points")],
                    "walk_pages": sum(
                        n
                        for op, n in {
                            **sent,
                            **b.get("requests by operation", {}),
                        }.items()
                        if any(w in op for w in WALK_WORDS)
                    ),
                }
            )
    return pairs


# --- the tables --------------------------------------------------------------------------------


def fmt(value: object) -> str:
    return f"{value:g}" if isinstance(value, (int, float)) else str(value)


def table(head: list[str], rows: list[list[str]]) -> str:
    out = [
        "| " + " | ".join(head) + " |",
        "| " + " | ".join("---" for _ in head) + " |",
    ]
    out += ["| " + " | ".join(r) + " |" for r in rows]
    return "\n".join(out)


def targets_table(rows: list[Row]) -> str:
    body = []
    for row in rows:
        target, before, earlier = (
            TARGETS[row.operation],
            BEFORE.get(row.operation, {}),
            EARLIER[row.scenario],
        )
        cells = [row.label]
        values = {
            "seconds": f"{row.seconds:.1f}",
            "points": str(row.points.figure),
            "writes": str(row.writes["writes"]),
        }
        for name in ("seconds", "points", "writes"):
            bar = target.get(name)
            mark = (
                f"target {fmt(bar)}: **{row.verdicts[name]}**"
                if bar is not None
                else "no target"
            )
            was = (
                f"{fmt(before[name])} (loopback, 0.12.3)"
                if name in before
                else "none in the brief"
            )
            cells.append(
                f"**{values[name]}** — {mark}; before {was}; GitHub at 0.11.0 {fmt(earlier[name])}"
            )
        flip = row.verdicts.get("points at 5 per mutation")
        cells.append(
            f"{row.points.at_five} — {'**would flip to over**' if flip == 'over' and row.verdicts['points'] == 'within' else 'no flip'}"
        )
        body.append(cells)
    return table(
        [
            "row",
            "elapsed seconds",
            "GraphQL points",
            "writes (mutations + uploads)",
            "points if each mutation request cost 5",
        ],
        body,
    )


def detail_table(rows: list[Row]) -> str:
    body = []
    for row in rows:
        w = row.writes
        body.append(
            [
                row.label,
                f"`{row.capture}`",
                str(w["requests"]),
                ", ".join(
                    f"`{op}` {k['requests']}×/{k['points']} pt"
                    + (f"/{k['writes']} w" if k["writes"] else "")
                    for op, k in row.kinds.items()
                ),
                f"{row.points.queries} queries at their dry-run price + {row.points.mutation_requests} mutation requests at {MUTATION_PRICE} = {row.points.figure}",
                f"{w['mutation_requests']} requests carrying {w['mutations']} mutations + {w['uploads']} upload(s) = {w['writes']}; {w['issues_created']} issues created",
                f"{row.screening['inspect']} inspect + {row.screening['check']} check ({row.screening['seconds']:.2f} s)",
                f"{row.cache['before'].get('files', 0)} → {row.cache['after'].get('files', 0)} files",
            ]
        )
    return table(
        [
            "row",
            "capture",
            "requests",
            "request kinds (requests/points/writes)",
            "points",
            "writes",
            "screening calls",
            "store cache",
        ],
        body,
    )


def seconds_table(rows: list[Row]) -> str:
    body = []
    for row in rows:
        shares = "; ".join(
            f"{label} {value:.2f} s" for label, value in row.attribution.items()
        )
        body.append(
            [
                row.label,
                f"{row.seconds:.2f} s",
                shares,
                f"{sum(row.attribution.values()):.2f} s",
            ]
        )
    return table(["row", "measured", "attributed to", "sum of shares"], body)


def owners_table(rows: list[Row]) -> str:
    body = []
    for row in rows:
        for key, value in row.owners.items():
            owner, verb, op = key.split(" | ")
            body.append(
                [
                    row.label,
                    owner,
                    verb,
                    f"`{op}`",
                    str(value["requests"]),
                    str(value["points"]),
                    str(value["writes"]),
                ]
            )
    return table(
        ["row", "owner", "sent inside", "request kind", "requests", "points", "writes"],
        body,
    )


def over_table(rows: list[Row]) -> str:
    over = [
        f"{row.label}: {name}"
        for row in rows
        for name in ("points", "writes")
        if row.verdicts.get(name) == "over"
    ]
    return (
        "Over its target: " + "; ".join(over) + "."
        if over
        else "No points or writes figure is over its target."
    )


def burn_table(rows: list[Row]) -> str:
    body = []
    for row in rows:
        pt = row.points
        if pt.counter is None:
            body.append(
                [
                    row.label,
                    "no counter read on both sides",
                    "—",
                    "—",
                    "—",
                    "—",
                    str(pt.figure),
                ]
            )
            continue
        body.append(
            [
                row.label,
                f"{pt.before_used} → {pt.after_used} (counter resetting {pt.counter})",
                str(pt.delta),
                f"{pt.background_rate:.2f} points/s",
                f"{pt.background:.1f}",
                str(pt.figure),
                f"{pt.residual:+.1f}",
            ]
        )
    return table(
        [
            "row",
            "GraphQL `rateLimit` used, last idle read before → first after",
            "rise",
            "idle background burn bracketing the row",
            "background expected over the row",
            "the row's own priced points",
            "residual (rise − own − background)",
        ],
        body,
    )


def quota_table(rows: list[Row]) -> str:
    body = []
    for row in rows:
        q = row.quota
        gate = [e for e in q.get("gate", []) if e["event"] == "reading"][-1:]
        pauses = [e for e in q.get("gate", []) if e["event"] == "pause"]
        before = q["quota-before"]["buckets"]
        after = q["quota-after"]["buckets"]
        rl_b, rl_a = before["graphql (rateLimit)"], after["graphql (rateLimit)"]
        least = before["graphql"]
        body.append(
            [
                row.label,
                f"{least['remaining']} / {least['limit']} ({least['remaining'] / least['limit']:.0%})",
                f"remaining {', '.join(map(str, rl_b['readings']))}",
                f"remaining {', '.join(map(str, rl_a['readings']))}",
                f"{before['graphql (GET /rate_limit)']['remaining']} → {after['graphql (GET /rate_limit)']['remaining']}",
                f"{before['core']['remaining']} → {after['core']['remaining']}",
                "proceed (≥ 40%)"
                if gate and all(gate[0]["verdicts"].values())
                else "paused",
                f"{len(pauses)} pause(s), {sum(p['seconds'] for p in pauses):.0f} s"
                if pauses
                else "none",
            ]
        )
    return table(
        [
            "row",
            "gate figure (least reading)",
            "GraphQL `rateLimit` before (three reads)",
            "GraphQL `rateLimit` after (three reads)",
            "`GET /rate_limit` graphql before → after",
            "REST core before → after",
            "gate",
            "pauses",
        ],
        body,
    )


def conditions_table(rows: list[Row]) -> str:
    body = []
    for row in rows:
        c = row.conditions
        sources = sorted({k.split(".")[1] for k in c if k.startswith("sources.")})
        declared = "; ".join(
            f"`{s}` {c.get(f'sources.{s}.plugin', {}).get('value', '?')}, {c.get(f'sources.{s}.visibility', {}).get('value', 'undeclared')}"
            + (
                ", cache on"
                if c.get(f"sources.{s}.cache.enabled", {}).get("value")
                else ""
            )
            for s in sources
        )
        layers = sorted({v["layer"] for v in c.values()})
        policy = ", ".join(
            f"`{k.split('.')[1]}` {v['value']} ({v['layer']})"
            for k, v in sorted(c.items())
            if k.startswith("write_policy.")
        )
        v = row.versions
        body.append(
            [
                row.label,
                declared,
                policy,
                ", ".join(layers),
                f"{v.get('store')}; {v.get('engine')}; {v.get('sdk')}; {v.get('onevcs')}; host `{v.get('host_commit')}`; harness `{v.get('harness_commit')}`",
            ]
        )
    return table(
        [
            "row",
            "sources (plugin, visibility, cache)",
            "write_policy",
            "configuration layers present",
            "versions",
        ],
        body,
    )


def workload_table(rows: list[Row]) -> str:
    body = []
    for row in rows:
        w = row.workload
        tasks = [w[k][0] for k in sorted(w) if k.startswith("task")]
        body.append(
            [
                row.label,
                f"{w.get('project', [0])[0]:,}",
                f"{min(tasks):,}–{max(tasks):,} ({len(tasks)})" if tasks else "—",
                f"{w.get('document', [0])[0]:,}",
                f"{w.get('image', [0])[0]:,}",
            ]
        )
    return table(
        ["row", "project bytes", "task bytes (tasks)", "document bytes", "PNG bytes"],
        body,
    )


def size_table(pairs: list[dict]) -> str:
    body = [
        [
            f"`{p['budget']}`",
            p["recording"],
            f"{p['requests'][0]} / {p['requests'][1]}",
            "same" if p["same"] else "**differ**",
            f"{fmt(p['points'][0])} / {fmt(p['points'][1])}",
            str(p["walk_pages"]),
        ]
        for p in pairs
    ]
    return table(
        [
            "loopback recording",
            "configuration",
            "requests at 2,000 / 20,000 items",
            "requests by operation",
            "points at 2,000 / 20,000",
            "walk pages",
        ],
        body,
    )


def reference_table(rows: list[Row]) -> str:
    body = []
    for row in rows:
        pages = row.walk_pages
        arithmetic = (
            f"{row.seconds:.1f} s measured − 0 s of 0 walk pages + 0 × 6.95 s + 0 × 6.79 s = **{reference_seconds(row, 0):.1f} s**"
            if not any(pages.values())
            else f"**{reference_seconds(row, row.walk_ms):.1f} s**"
        )
        body.append(
            [
                row.label,
                f"{pages['BOARD']} BOARD + {pages['SEARCH_ISSUES']} SEARCH_ISSUES",
                arithmetic,
                str(row.points.figure),
            ]
        )
    return table(
        [
            "row",
            "walk pages sent",
            "seconds at the `plans` board's size",
            "points at that size",
        ],
        body,
    )


def writes_table(captures: Path) -> str:
    body = []
    named = [
        p
        for p in captures.iterdir()
        if p.is_dir() and re.fullmatch(r".+-\d{8}T\d{6}", p.name)
    ]
    superseded = captures / "superseded"
    named += (
        [p for p in superseded.iterdir() if p.is_dir()] if superseded.exists() else []
    )
    for path in sorted(named, key=lambda p: p.name.rsplit("-", 1)[-1]):
        requests = lines(path / "requests.jsonl")
        w = writes_of(requests)
        cleanup = (
            json.loads((path / "cleanup.json").read_text())
            if (path / "cleanup.json").exists()
            else {}
        )
        refused = sum(1 for r in requests if r["host"] == "refused")
        body.append(
            [
                f"`{path.name}`"
                + (" (superseded)" if path.parent == superseded else ""),
                str(w["mutation_requests"]),
                str(w["mutations"]),
                str(w["uploads"]),
                str(w["issues_created"]),
                f"{cleanup['deleted']} ({cleanup.get('only', 'all')})"
                if cleanup
                else "—",
                str(refused),
            ]
        )
    return table(
        [
            "capture",
            "mutation requests",
            "mutations",
            "uploads",
            "issues created",
            "issues deleted by cleanup",
            "requests the guard refused",
        ],
        body,
    )


def probes_table(captures: Path) -> str:
    """The two hand probes: the idle background on each counter, and what onevcs's inspect spends."""
    path = captures / "probes" / "background-and-onevcs.json"
    if not path.exists():
        return "No probe was recorded."
    probe = json.loads(path.read_text())
    body = []
    readings = probe["idle_background"]["readings"]
    for reset in sorted({r["reset"] for r in readings}):
        on = [r for r in readings if r["reset"] == reset]
        rise, seconds = slope(on)
        body.append(
            [
                f"idle background, counter resetting {reset}",
                f"{len(on)} reads over {seconds:.1f} s",
                f"{rise:g} points",
                f"{rise / seconds:.2f} points/s" if seconds else "—",
            ]
        )
    inspect = probe["onevcs_inspect"]
    core = inspect["after"][0]["core_used"] - inspect["before"][-1]["core_used"]
    graphql = (
        inspect["after"][0]["graphql_used"] - inspect["before"][-1]["graphql_used"]
    )
    body.append(
        [
            f"{inspect['inspects']} × `onevcs boundary inspect`",
            "`GET /rate_limit` last read before → first after",
            f"REST core {core:+d}, GraphQL {graphql:+d}",
            f"{core / inspect['inspects']:g} REST request and {graphql / inspect['inspects']:g} GraphQL points per inspect",
        ]
    )
    return table(["probe", "readings", "rise", "rate"], body)


def spend_table(captures: Path, priced: int) -> str:
    """What every capture sent, and the quota reads around it, with the total."""
    named = [
        p
        for p in captures.iterdir()
        if p.is_dir() and re.fullmatch(r".+-\d{8}T\d{6}", p.name)
    ]
    superseded = captures / "superseded"
    named += (
        [p for p in superseded.iterdir() if p.is_dir()] if superseded.exists() else []
    )
    body, totals = [], Counter()
    for path in sorted(named, key=lambda p: p.name.rsplit("-", 1)[-1]):
        requests = [r for r in lines(path / "requests.jsonl") if r["host"] != "refused"]
        q = (
            json.loads((path / "quota.json").read_text())
            if (path / "quota.json").exists()
            else {}
        )
        figures = Counter(
            queries=sum(1 for r in requests if r["kind"] == "query"),
            mutation_requests=sum(1 for r in requests if r["kind"] == "mutation"),
            rest=sum(1 for r in requests if r["kind"] == "rest"),
            tunnels=sum(1 for r in requests if r["kind"] == "tunnel"),
            screening=len(lines(path / "screen.jsonl")),
            rate_limit_reads=sum(
                len(
                    q.get(k, {})
                    .get("buckets", {})
                    .get("graphql (rateLimit)", {})
                    .get("readings", [])
                )
                for k in ("quota-before", "quota-after")
            )
            + len(q.get("burn-before", []))
            + len(q.get("burn-after", [])),
            rest_rate_limit_reads=sum(
                1 for k in ("quota-before", "quota-after") if k in q
            ),
        )
        totals += figures
        body.append(
            [f"`{path.name}`"]
            + [
                str(figures[k])
                for k in (
                    "queries",
                    "mutation_requests",
                    "rest",
                    "tunnels",
                    "screening",
                    "rate_limit_reads",
                    "rest_rate_limit_reads",
                )
            ]
        )
    body.append(
        ["**total**"]
        + [
            f"**{totals[k]}**"
            for k in (
                "queries",
                "mutation_requests",
                "rest",
                "tunnels",
                "screening",
                "rate_limit_reads",
                "rest_rate_limit_reads",
            )
        ]
    )
    table_text = table(
        [
            "capture",
            "GraphQL queries",
            "mutation requests",
            "REST requests",
            "tunnels (onevcs and the upload's storage)",
            "onevcs calls",
            "GraphQL `rateLimit` reads (free)",
            "`GET /rate_limit` reads (free)",
        ],
        body,
    )
    return (
        table_text
        + f"\n\nBeside these, `harness/pricing.py` sent {priced} dry-run pricings, one per GraphQL query of every run it read."
    )


def tables(captures: Path) -> tuple[dict[str, str], dict]:
    pricing = (
        json.loads((captures / "pricing.json").read_text())
        if (captures / "pricing.json").exists()
        else {}
    )
    prices = {key: int(value) for key, value in pricing.get("requests", {}).items()}
    latest = latest_captures(captures)
    missing = [s for s, _, _ in ROWS if s not in latest]
    if missing:
        raise SystemExit(
            f"analyze.py: no capture of {', '.join(missing)} under {captures}; take them with measure.sh"
        )
    rows = [row_of(s, op, label, latest[s], prices) for s, op, label in ROWS]
    pairs = size_pairs(loopback(captures))
    out = {
        "targets": targets_table(rows),
        "over": over_table(rows),
        "detail": detail_table(rows),
        "owners": owners_table(rows),
        "seconds": seconds_table(rows),
        "burn": burn_table(rows),
        "quota": quota_table(rows),
        "conditions": conditions_table(rows),
        "workload": workload_table(rows),
        "board-size": size_table(pairs),
        "reference-size": reference_table(rows),
        "writes": writes_table(captures),
        "spend": spend_table(captures, len(prices)),
        "probes": probes_table(captures),
    }
    summary = {
        "rows": [
            {
                "scenario": r.scenario,
                "capture": r.capture,
                "seconds": r.seconds,
                "points": r.points.__dict__,
                "writes": r.writes,
                "kinds": r.kinds,
                "screening": r.screening,
                "attribution": r.attribution,
                "owners": r.owners,
                "verdicts": r.verdicts,
                "walk_pages": r.walk_pages,
                "refused": r.refused,
                "returncodes": r.returncodes,
            }
            for r in rows
        ],
        "board_size_pairs": pairs,
        "mutation_rises": {
            k: v for k, v in pricing.get("mutation_rises", {}).items() if v
        },
    }
    return out, summary


BLOCK = re.compile(
    r"<!-- analyze:begin ([a-z-]+) -->\n(.*?)\n?<!-- analyze:end \1 -->", re.DOTALL
)


def reconcile(report: str, computed: dict[str, str]) -> tuple[str, list[str]]:
    """The report with every block replaced by its recomputed table, and the names of the blocks
    that differed (or that the report lacks)."""
    seen: set[str] = set()
    drifted: list[str] = []

    def replace(match: re.Match) -> str:
        name = match.group(1)
        seen.add(name)
        if name not in computed:
            drifted.append(f"{name} (no such table)")
            return match.group(0)
        if match.group(2).strip() != computed[name].strip():
            drifted.append(name)
        return f"<!-- analyze:begin {name} -->\n{computed[name]}\n<!-- analyze:end {name} -->"

    updated = BLOCK.sub(replace, report)
    drifted += [
        f"{name} (missing from the report)" for name in computed if name not in seen
    ]
    return updated, drifted


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--write", action="store_true", help="rewrite the report's tables"
    )
    parser.add_argument("--captures", type=Path, default=CAPTURES)
    parser.add_argument("--report", type=Path, default=REPORT)
    args = parser.parse_args(argv)
    computed, summary = tables(args.captures)
    (args.captures / "summary.json").write_text(
        json.dumps(summary, indent=1, sort_keys=True) + "\n"
    )
    updated, drifted = reconcile(args.report.read_text(), computed)
    if args.write:
        args.report.write_text(updated)
        return 0
    if drifted:
        print(
            f"analyze.py: {args.report.name} differs from its captures in: {', '.join(drifted)}; "
            "run with --write to put the recomputed tables in place",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
