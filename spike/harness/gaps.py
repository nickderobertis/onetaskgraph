"""The feasibility spike's analysis: `python3 spike/harness/gaps.py`.

Reads only committed, sanitized logs — the timing re-measurement's baseline under
`docs/measurements/board-copy-speed-2026-10-18/after/` and this spike's own runs under
`spike/logs/` — and writes `spike/logs/gaps-summary.json`:

- `prices`: what each request shape costs, from the latest `probe-alias-pricing` (dry runs,
  and the coalesced shapes charged for real) and from the `x-ratelimit-used` rise each probe
  mutation request carried over the request before it in the same window (least of all
  samples, because other sessions only ever add);
- `placement`: when GitHub put each created issue on the board, every `probe-placement`;
- `reach`, `union`, `refusal`: the latest offline result of each case and build;
- `candidates`: the 16 budgets, each with its baseline, the attainable figure and the
  arithmetic from the prices above (estimates labelled as such), and what limits it;
- `consumed`: every request this spike's live runs sent, what they cost and how long they ran.

No figure in it is typed in: each is read from a log, and each plan behind an attainable
figure names the priced shapes it is the sum of.
"""

from __future__ import annotations

import json
from pathlib import Path

import feasibility

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
LOGS = HERE.parent / "logs"
BASELINE = REPO / "docs/measurements/board-copy-speed-2026-10-18/after"
# The runs `docs/measurements/board-copy-speed-2026-10-18.md` draws each scenario from.
BASELINE_RUNS = {
    "bare first": "tb-first-span-20261024T095836",
    "bare changed": "tb-changed1-span-20261024T101530",
    "bare unchanged": "tb-unchanged-span-20261024T095930",
    "host first": "host-copy-stock-20261024T100745",
    "host changed": "host-changed1-stock-20261024T102149",
    "host unchanged": "host-recopy-stock-20261024T100859",
    "host approval": "host-approve-stock-20261024T100947",
}
PROBE_ALIASES = feasibility.PROBE_ALIASES
ONEVCS_CALL_SECONDS = (
    0.62,
    0.67,
)  # the host's measured mean per onevcs call, least and most


def lines(path: Path) -> list[dict]:
    if not path.exists():
        return []
    return [json.loads(line) for line in path.read_text().splitlines() if line.strip()]


def runs(prefix: str) -> list[Path]:
    return sorted(p for p in LOGS.iterdir() if p.is_dir() and p.name.startswith(prefix))


def latest(prefix: str) -> Path:
    found = runs(prefix)
    if not found:
        raise SystemExit(
            f"gaps: no run {prefix}* under {LOGS}; run spike/copy-timing.sh first"
        )
    return found[-1]


def mutation_rises() -> dict[int, list[int]]:
    """For each alias count, every `used` rise a probe mutation request carried over the last
    request before it in the same window."""
    rises: dict[int, list[int]] = {}
    for run in runs("probe-placement-"):
        last: dict[int, int] = {}
        for record in sorted(lines(run / "requests.jsonl"), key=lambda r: r["start"]):
            limits = record.get("limits") or {}
            if "used" not in limits or record["host"] == "refused":
                continue
            window, used = limits["reset"], limits["used"]
            aliases = PROBE_ALIASES.get(record["op"])
            if (
                record["kind"] == "mutation"
                and aliases
                and window in last
                and not record["errors"]
            ):
                rises.setdefault(aliases, []).append(used - last[window])
            last[window] = used
    return rises


def prices() -> dict:
    pricing = json.loads(
        (latest("probe-alias-pricing-") / "feasibility.json").read_text()
    )
    dry = {r["variant"]: r["dry_run_cost"] for r in pricing["dry_runs"]}
    charged = {r["variant"]: r["charged_cost"] for r in pricing["executed"]}
    rises = mutation_rises()
    least = {aliases: min(values) for aliases, values in sorted(rises.items())}
    polls = [
        json.loads((run / "feasibility.json").read_text())["placement"]["poll_costs"]
        for run in runs("probe-placement-")
    ]
    out = {
        "dry_run": dry,
        "charged": charged,
        "dry_run_agrees_with_charge": all(dry.get(v) == c for v, c in charged.items()),
        "mutation_request_least_rise_by_aliases": least,
        "mutation_request_rises_by_aliases": {k: v for k, v in sorted(rises.items())},
        "placement_poll": sorted({c for p in polls for c in p}),
    }
    # The shapes the plans below are sums of, each from one of the figures above.
    one_mutation = max(least.values()) if least else None
    out["shape"] = {
        "batched origin lookup, 10 asks, first 1": dry[
            "ORIGIN_LOOKUPS asks=10 originFirst=1"
        ],
        "batched origin lookup, 10 asks, first 3 (as sent today)": dry[
            "ORIGIN_LOOKUPS asks=10"
        ],
        "single origin lookup": dry["ORIGIN_LOOKUP"],
        "sub-issue listing (23 members)": dry["SUB_ISSUES memberFirst=23"],
        "item read": dry["ISSUE"],
        "placement read (up to 21 issues)": max(out["placement_poll"]),
        "listing through the item's parent, item trimmed": charged[
            "SUB_ISSUES through the item's parent, item trimmed"
        ],
        "mutation request (any alias count observed)": one_mutation,
    }
    out["seconds"] = {
        "listing through the item's parent, item trimmed": next(
            r["seconds"]
            for r in pricing["executed"]
            if r["variant"] == "SUB_ISSUES through the item's parent, item trimmed"
        )
    }
    return out


def baseline() -> dict:
    summary = json.loads((BASELINE / "summary.json").read_text())["runs"]
    out = {}
    for scenario, run in BASELINE_RUNS.items():
        figures = summary[run]
        screening = sum(v for k, v in figures["split"].items() if "screening" in k)
        uploads = sum(
            v["count"]
            for k, v in figures["by_op"].items()
            if k.startswith("POST /user-attachments")
        )
        out[scenario] = {
            "run": run,
            "points": figures["points"],
            "requests": figures["requests"],
            "writes": figures["mutation_fields"] + uploads,
            "wall_seconds": round(sum(figures["split"].values()), 1),
            "screening_seconds": round(screening, 2),
            "screening_calls": sum(figures.get("screening_calls", {}).values()),
            "by_op": {
                k: v["count"]
                for k, v in figures["by_op"].items()
                if v["kind"] != "upload" or k.startswith("POST")
            },
        }
    return out


def plan(shape: dict, steps: list[tuple[str, str, int]]) -> dict:
    """`steps`: (what, priced shape, how many). The sum and the arithmetic that makes it."""
    total = sum(shape[priced] * count for _, priced, count in steps)
    arithmetic = " + ".join(
        f"{what} {count}×{shape[priced]}" for what, priced, count in steps
    )
    return {"points": total, "arithmetic": f"{arithmetic} = {total}"}


M = "mutation request (any alias count observed)"
LOOKUP = "batched origin lookup, 10 asks, first 1"
ONE = "single origin lookup"
LIST = "sub-issue listing (23 members)"
PLACE = "placement read (up to 21 issues)"
PARENT = "listing through the item's parent, item trimmed"
READ = "item read"


def first_copy(shape: dict, polls: int = 1, write_requests: int = 1) -> dict:
    return plan(
        shape,
        [
            ("project copy: origin lookups of the project and 8 tasks", LOOKUP, 1),
            ("create the project issue", M, 1),
            (
                "create 8 task issues, aliased, with projectV2Ids and parentIssueId",
                M,
                1,
            ),
            ("placement read of 9 issues", PLACE, polls),
            ("field values and 10 blocked-by edges, aliased", M, write_requests),
            ("document copy: its origin lookup", ONE, 1),
            ("create the document issue", M, 1),
            ("placement read", PLACE, polls),
            ("its field value", M, 1),
        ],
    )


def candidates(shape: dict, base: dict, timing: dict) -> list[dict]:
    expected = first_copy(shape)
    worst = first_copy(shape, polls=2, write_requests=2)
    changed = plan(
        shape,
        [
            ("project copy: listing", LIST, 1),
            ("update the changed body", M, 1),
            ("document copy: listing", LIST, 1),
        ],
    )
    host_changed = plan(
        shape,
        [
            ("project copy: listing (field options from it)", LIST, 1),
            ("2 body updates, aliased", M, 1),
            ("document copy: listing", LIST, 1),
        ],
    )
    unchanged = plan(
        shape, [("project copy: listing", LIST, 1), ("document copy: listing", LIST, 1)]
    )
    approval = plan(
        shape,
        [
            ("document list --project", LIST, 1),
            ("project show", READ, 1),
            ("task list --project", LIST, 1),
            ("metadata set: document, project and members in one read", PARENT, 1),
            ("the write", M, 1),
        ],
    )
    first_writes = base["bare first"]["writes"]
    out = []

    def add(budget, bar, unit, scenario, attained, arithmetic, limits):
        out.append(
            {
                "budget": budget,
                "bar": bar,
                "unit": unit,
                "baseline": base[scenario][unit_key(unit)],
                "baseline_run": base[scenario]["run"],
                "attainable": attained,
                "within": attained <= bar
                if isinstance(attained, (int, float))
                else None,
                "arithmetic": arithmetic,
                "limits": limits,
            }
        )

    lim_points = (
        "Prices are GitHub's own (dry run, matched by the charged cost of the coalesced shapes); "
        "a mutation request was charged 1 whatever its alias count in every sample, up to 19 aliases."
    )
    add(
        "github-plan-copy-points-first",
        12,
        "points",
        "bare first",
        expected["points"],
        expected["arithmetic"]
        + f"; worst case two placement reads a process and two write requests: {worst['arithmetic']}",
        lim_points,
    )
    add(
        "plan-copy-points-first",
        12,
        "points",
        "host first",
        expected["points"],
        expected["arithmetic"],
        lim_points
        + " The host recipe sends the store's requests; screening sends none to GraphQL.",
    )
    add(
        "github-plan-copy-points-changed",
        4,
        "points",
        "bare changed",
        changed["points"],
        changed["arithmetic"],
        lim_points
        + " The document copy finds every counterpart from its recorded links and the listing.",
    )
    add(
        "plan-copy-points-changed",
        4,
        "points",
        "host changed",
        host_changed["points"],
        host_changed["arithmetic"],
        lim_points + " BOARD_FIELDS' options are in the listing's field values.",
    )
    add(
        "github-plan-copy-points-unchanged",
        3,
        "points",
        "bare unchanged",
        unchanged["points"],
        unchanged["arithmetic"],
        lim_points,
    )
    add(
        "plan-copy-points-unchanged",
        3,
        "points",
        "host unchanged",
        unchanged["points"],
        unchanged["arithmetic"],
        lim_points,
    )
    add(
        "plan-approval-points",
        5,
        "points",
        "host approval",
        approval["points"],
        approval["arithmetic"],
        lim_points
        + " The parent-scoped read is a new document of the plugin's own; no contract changes.",
    )
    add(
        "github-plan-copy-content-creations",
        40,
        "writes",
        "bare first",
        first_writes,
        f"unchanged by coalescing: aliases count one each ({first_writes} = 35 mutations + 1 upload); placement by projectV2Ids adds none, "
        "while an addProjectV2ItemById per issue would add 10",
        "Coalescing moves requests, not mutations; the 40 holds only while placement rides on createIssue.",
    )
    add(
        "plan-copy-writes-first",
        40,
        "writes",
        "host first",
        base["host first"]["writes"],
        f"{base['host first']['writes']} as measured; coalescing adds none",
        "As above.",
    )
    for budget, bar, scenario, estimate in timing:
        add(
            budget,
            bar,
            "seconds",
            scenario,
            estimate["seconds"],
            estimate["arithmetic"],
            estimate["limits"],
        )
    return out


def unit_key(unit: str) -> str:
    return {"points": "points", "writes": "writes", "seconds": "wall_seconds"}[unit]


def removed(scenario: str, picks: tuple[tuple[str, int], ...]) -> tuple[float, str]:
    """The seconds the last `n` requests of each named operation took in a baseline run: the
    reads a prototype plan does not send."""
    records = lines(BASELINE / BASELINE_RUNS[scenario] / "requests.jsonl")
    total, said = 0.0, []
    for op, n in picks:
        mine = [r["ms"] for r in records if r["op"] == op][-n:] if n else []
        total += sum(mine) / 1000
        said.append(f"{len(mine)} {op}")
    return round(total, 2), ", ".join(said)


def seconds(base: dict, union: dict, placement: list[dict], shape: dict) -> list[tuple]:
    """Each seconds candidate: the baseline less what the prototypes remove, as arithmetic."""
    per_call = ONEVCS_CALL_SECONDS[1]
    child = max(max(p["children_seconds_after_their_answer"]) for p in placement)
    calls_after = {
        step: v["as copied"]["unioned"]["calls"]
        for step, v in union["commands"].items()
    }
    per_copy = sum(calls_after.values())
    parent_read = shape["seconds"]["listing through the item's parent, item trimmed"]

    def host(scenario, picks, screened, added, limits):
        b = base[scenario]
        cut, which = removed(scenario, picks)
        after = round(
            b["wall_seconds"]
            - b["screening_seconds"]
            + screened * per_call
            - cut
            + added,
            1,
        )
        return {
            "seconds": after,
            "arithmetic": (
                f"{b['wall_seconds']} s measured − {b['screening_seconds']} s screening ({b['screening_calls']} onevcs calls)"
                f" + {screened} calls × {per_call} s − {cut} s ({which}) + {added} s added = {after} s"
            ),
            "limits": "Estimate; seconds warn on this host. " + limits,
        }

    def bare(scenario, picks):
        b = base[scenario]
        cut, which = removed(scenario, picks)
        after = round(b["wall_seconds"] - cut, 1)
        return {
            "seconds": after,
            "arithmetic": f"{b['wall_seconds']} s measured − {cut} s ({which}) = {after} s",
            "limits": "Estimate; seconds warn on this host; one baseline sample.",
        }

    note = f"union screening sends {per_copy} onevcs calls a copy ({calls_after}); every placement sample was read once, {child} s after creation at most"
    lookups = (("ORIGIN_LOOKUP", 4),)
    return [
        (
            "github-plan-copy-seconds-first",
            32,
            "bare first",
            bare(
                "bare first",
                (
                    ("ORIGIN_LOOKUP", 3),
                    ("ISSUE", 1),
                    ("SUB_ISSUES", 1),
                    ("CREATED_BOARD_ITEMS", 2),
                    ("WRITE_FIELD_VALUES", 1),
                ),
            ),
        ),
        (
            "plan-copy-seconds-first",
            32,
            "host first",
            host(
                "host first",
                (("ISSUE", 2), ("CREATED_BOARD_ITEMS", 2), ("SUB_ISSUES", 1)),
                per_copy,
                0.0,
                f"{note}. The stock-process time (11.0 s unattributed) is not removed.",
            ),
        ),
        (
            "github-plan-copy-seconds-changed",
            8,
            "bare changed",
            bare("bare changed", lookups),
        ),
        (
            "plan-copy-seconds-changed",
            8,
            "host changed",
            host(
                "host changed",
                (
                    ("BOARD_FIELDS", 1),
                    ("ORIGIN_LOOKUP", 1),
                    ("ISSUE", 1),
                    ("UPDATE_ISSUE", 1),
                ),
                4,
                0.0,
                f"Over the bar: 10.0 s of stock-process time and 2.1 s of host steps remain. Re-screening only the changed exposure is 4 calls. {note}.",
            ),
        ),
        (
            "github-plan-copy-seconds-unchanged",
            7,
            "bare unchanged",
            bare("bare unchanged", lookups),
        ),
        (
            "plan-copy-seconds-unchanged",
            7,
            "host unchanged",
            host(
                "host unchanged",
                (("ORIGIN_LOOKUP", 1), ("ISSUE", 1)),
                0,
                0.0,
                "Over the bar: an unchanged copy writes and so screens nothing, but 7.7 s of stock-process time and 2.2 s of host steps remain.",
            ),
        ),
        (
            "plan-approval-seconds",
            7,
            "host approval",
            host(
                "host approval",
                (("ISSUE", 2), ("SUB_ISSUES", 1)),
                2,
                parent_read,
                "Over the bar: one screened write is 1 inspect + 1 check; 1.7 s of stock-process time, 1.8 s of host steps and 4 store processes remain.",
            ),
        ),
    ]


def offline() -> dict:
    out: dict = {"reach": {}, "union": {}, "refusal": None}
    for case in (
        "reach-public",
        "reach-private-item",
        "reach-private-ref",
        "reach-unresolvable-ref",
    ):
        for build in ("stock", "head", "closure"):
            found = runs(f"{case}-{build}-")
            if not found:
                continue
            r = json.loads((found[-1] / "result.json").read_text())
            out["reach"][f"{case} {build}"] = {
                "run": found[-1].name,
                "binary": r["binary"],
                "exit": [s["returncode"] for s in r["steps"]],
                "requests_to_private_doubles": r["private_requests"],
                "operations_they_asked": sorted(
                    {q["operation"] for s in r["steps"] for q in s["recorder_requests"]}
                ),
                "destination_changed": r["destination_changed"],
                "onevcs_calls": {
                    "inspect": r["screens"]["inspect"],
                    "check": r["screens"]["check"],
                },
                "output_names_the_reference": any(
                    s.get("output_names_the_reference", False) for s in r["steps"]
                ),
                "output_says_withheld": any(
                    s.get("output_says_withheld", False) for s in r["steps"]
                ),
                "refusal": next(
                    (
                        s["stderr_tail"][0]
                        for s in r["steps"]
                        if s["returncode"] not in (0, 4) and s["stderr_tail"]
                    ),
                    None,
                ),
            }
    for build in ("closure", "head"):
        found = runs(f"screen-union-{build}-")
        if found:
            r = json.loads((found[-1] / "result.json").read_text())
            out["union"][build] = {
                "run": found[-1].name,
                "all_agree": r["all_agree"],
                "commands": {
                    step: {
                        variant: {
                            "per_item_calls": v["per_item"]["calls"],
                            "unioned_calls": v["unioned"]["calls"],
                            "per_item_refused": 1 in v["per_item"]["check_returncodes"],
                            "union_refused": v["unioned"]["check_returncode"] == 1,
                        }
                        for variant, v in variants.items()
                    }
                    for step, variants in r["commands"].items()
                },
            }
    found = runs("proxy-refusal-")
    if found:
        out["refusal"] = json.loads((found[-1] / "result.json").read_text()) | {
            "run": found[-1].name
        }
    return out


def consumed() -> dict:
    total = {
        "runs": 0,
        "requests": 0,
        "queries": 0,
        "dry_runs": 0,
        "mutation_requests": 0,
        "issues_created": 0,
        "points": 0,
        "seconds": 0.0,
    }
    per_run = {}
    for run in runs("probe-"):
        records = [r for r in lines(run / "requests.jsonl") if r["host"] != "refused"]
        feasibility = json.loads((run / "feasibility.json").read_text())
        dry = len(feasibility.get("dry_runs", []))
        mutations = [r for r in records if r["kind"] == "mutation"]
        queries = [r for r in records if r["kind"] == "query"]
        # Dry runs (unnamed, no cost reported) and `rateLimit`-only reads moved `used` by
        # nothing in these logs; every other query is charged what it reported, or 1; a
        # mutation request 1.
        points = len(mutations) + sum(
            r["cost"]
            if r["cost"] is not None
            else (0 if r["op"] in ("unnamed", "ProbeRate") else 1)
            for r in queries
        )
        created = sum(
            PROBE_ALIASES.get(r["op"], 0)
            for r in mutations
            if r["op"].startswith("ProbeCreate")
        )
        quota = json.loads((run / "quota.json").read_text())
        figures = {
            "requests": len(records),
            "queries": len(queries),
            "dry_runs": dry,
            "mutation_requests": len(mutations),
            "issues_created": created,
            "points": points,
            "seconds": round(
                max(r["end"] for r in records) - min(r["start"] for r in records), 1
            ),
            "graphql_before": quota["quota-before"]["buckets"]["graphql"]["remaining"]
            if "quota-before" in quota
            else None,
            "graphql_after": quota["quota-after"]["buckets"]["graphql"]["remaining"],
        }
        per_run[run.name] = figures
        total["runs"] += 1
        for key in (
            "requests",
            "queries",
            "dry_runs",
            "mutation_requests",
            "issues_created",
            "points",
        ):
            total[key] += figures[key]
        total["seconds"] = round(total["seconds"] + figures["seconds"], 1)
    readings = len(runs("probe-"))
    # Each reading is 1 `GET /rate_limit` (free) and 3 `rateLimit` queries, which moved `used`
    # by nothing in these logs; they are requests all the same.
    total["requests_with_quota_readings"] = total["requests"] + 2 * readings * 4
    total["quota_readings"] = {
        "gate readings (1 GET /rate_limit + 3 rateLimit queries each)": readings,
        "after-run readings (the same)": readings,
    }
    return {"total": total, "runs": per_run}


def main() -> None:
    shape = prices()
    base = baseline()
    placement = [
        json.loads((run / "feasibility.json").read_text())["placement"]
        for run in runs("probe-placement-")
    ]
    results = offline()
    union = json.loads((latest("screen-union-closure-") / "result.json").read_text())
    summary = {
        "prices": shape,
        "placement": placement,
        "baseline": base,
        "candidates": candidates(
            shape["shape"], base, seconds(base, union, placement, shape)
        ),
        "offline": results,
        "consumed": consumed(),
    }
    (LOGS / "gaps-summary.json").write_text(
        json.dumps(summary, indent=1, sort_keys=True) + "\n"
    )
    print(f"wrote {LOGS / 'gaps-summary.json'}")


if __name__ == "__main__":
    main()
