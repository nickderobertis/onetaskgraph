"""Command-scoped deduplicated inspection and union screening, against the real onevcs.

`run.py screen-union` copies the synthetic public plan (`reach.py`'s `reach-public`), keeping
every `write_policy` call the store made, and then replays those calls through the same onevcs
and onevcs home three ways:

- **per item**, as the store sends them today: every `inspect` and every `check`;
- **deduplicated and unioned**, as a command-scoped store would: one `inspect` per distinct
  repository and one `check` whose `text`, `paths` and `metadata` are the concatenation of every
  check's arrays, element by element (no string is joined to another), under the one `scope`
  every check carried;
- **planted**: the same two ways, after one item's title, then its content, then a path it
  names, carries the synthetic private identity's name — so a union that passes what one item
  refuses would show.

What it keeps (`union.json`): call counts, the verdict of each, whether the union agrees with
the per-item verdicts in every variant, and the seconds onevcs took each way.
"""

from __future__ import annotations

import json
import os
import subprocess
import time
from pathlib import Path

import reach

PLANTS = ("metadata", "text", "paths")


def calls(log: Path) -> list[dict]:
    return [json.loads(line) for line in log.read_text().splitlines() if line.strip()]


def ask(onevcs: Path, home: Path, kind: str, document: dict) -> tuple[int, float, str]:
    argv = [onevcs, "boundary", kind, "--input", "-"]
    if kind == "check":
        argv[3:3] = ["--destination", "public"]
    start = time.time()
    done = subprocess.run(
        argv,
        input=json.dumps(document).encode(),
        env=dict(os.environ, ONEVCS_HOME=str(home)),
        capture_output=True,
        check=False,
    )
    return done.returncode, time.time() - start, done.stdout.decode(errors="replace")[:200]


def union_of(checks: list[dict]) -> dict:
    scopes = {json.dumps(c.get("scope"), sort_keys=True) for c in checks}
    if len(scopes) != 1:
        raise ValueError(f"the checks carried {len(scopes)} different scopes; a union needs one")
    out: dict = {"destination": "public", "text": [], "paths": [], "metadata": []}
    for check in checks:
        for key in ("text", "paths", "metadata"):
            out[key] += list(check.get(key, []))
    out["scope"] = checks[0].get("scope")
    if out["scope"] is None:
        del out["scope"]
    return out


def planted(checks: list[dict], where: str, term: str) -> list[dict]:
    """The same checks with `term` written into the third item's `where` array."""
    out = json.loads(json.dumps(checks))
    target = out[min(2, len(out) - 1)]
    values = target.setdefault(where, [])
    if values:
        values[0] = f"{values[0]} see {term} for details"
    else:
        values.append(f"notes/{term}/plan.md" if where == "paths" else f"see {term}")
    return out


def replay(onevcs: Path, home: Path, inspects: list[dict], checks: list[dict]) -> dict:
    per_item = [ask(onevcs, home, "inspect", i) for i in inspects]
    per_item += [ask(onevcs, home, "check", c) for c in checks]
    distinct = {json.dumps(i, sort_keys=True) for i in inspects}
    unioned = [ask(onevcs, home, "inspect", json.loads(i)) for i in sorted(distinct)]
    unioned.append(ask(onevcs, home, "check", union_of(checks)))
    item_refused = any(code != 0 for code, _, _ in per_item[len(inspects):])
    union_refused = unioned[-1][0] != 0
    return {
        "per_item": {
            "calls": len(per_item),
            "inspect": len(inspects),
            "check": len(checks),
            "check_returncodes": sorted({code for code, _, _ in per_item[len(inspects):]}),
            "inspect_answers": sorted({answer for _, _, answer in per_item[: len(inspects)]}),
            "seconds": round(sum(s for _, s, _ in per_item), 3),
        },
        "unioned": {
            "calls": len(unioned),
            "inspect": len(unioned) - 1,
            "check": 1,
            "check_returncode": unioned[-1][0],
            "inspect_answers": sorted({answer for _, _, answer in unioned[:-1]}),
            "seconds": round(sum(s for _, s, _ in unioned), 3),
        },
        "agrees": item_refused == union_refused,
    }


def run(run_dir: Path, exe: Path, onevcs: Path, python: str) -> dict:
    copied = reach.run(run_dir, "reach-public", exe, onevcs, python)
    home = run_dir / "sandbox" / "onevcs-home"
    log = calls(run_dir / "screen.jsonl")
    commands = {}
    # Each process's calls are one command's: split at the gap between the two steps.
    boundary = copied["steps"][0]["seconds"]
    first_start = min(c["start"] for c in log)
    for call in log:
        step = "project-copy" if call["start"] - first_start <= boundary else "document-copy"
        commands.setdefault(step, []).append(call)
    term = f"{reach.PRIVATE[0]}/{reach.PRIVATE[1]}"
    result: dict = {"copy": {k: copied[k] for k in ("binary", "onevcs", "private_requests", "screens")}}
    result["copy"]["returncodes"] = [s["returncode"] for s in copied["steps"]]
    result["commands"] = {}
    for step, made in commands.items():
        inspects = [json.loads(c["input"]) for c in made if c["name"] == "inspect"]
        checks = [json.loads(c["input"]) for c in made if c["name"] == "check"]
        variants = {"as copied": replay(onevcs, home, inspects, checks)}
        for where in PLANTS:
            variants[f"planted in {where}"] = replay(
                onevcs, home, inspects, planted(checks, where, term)
            )
        result["commands"][step] = variants
    result["all_agree"] = all(
        v["agrees"] for variants in result["commands"].values() for v in variants.values()
    )
    (run_dir / "union.json").write_text(json.dumps(result, indent=1))
    return result
