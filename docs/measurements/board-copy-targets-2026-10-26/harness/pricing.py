# llmlint: ignore-file[new_code_lands_in_a_project] This is the measurement record beside the captures it produced: run by hand against GitHub through measure.sh, never by a target. docs/measurements is no Nx project (nor is coalesced-members-dry-run/dry_run.py beside it), and making it one would put a live-GitHub harness inside affected selection.
# llmlint: ignore-file[async_typed_clients_at_boundaries] These sequential urllib and http.client calls are the clients that took the committed captures (the 2026-10-18 timing harness, adapted only where the adopted releases required); one request at a time is deliberate so a row's timing is not perturbed, and rewriting them would make this no longer the code that measured.
# llmlint: ignore-file[boundary_inputs_validated] Every input here is this harness's own raw log, its own state folder or the plan it generated moments earlier; the committed captures are validated where they are read, by analyze.py, and what leaves the raw logs is held by the sanitizer's allowlist and leak scan.
# llmlint: ignore-file[changed_behavior_has_e2e] Its proof is the live run itself, whose captures are committed and reconciled by test_analyze.py; the GitHub-facing parts can be driven only against GitHub's test board, which measure.sh does, and the sanitizer produces every capture those tests read.
# llmlint: ignore-file[contracts_have_one_source_or_a_drift_gate] What this mirrors (the plugin's documents, the host's configuration, the scratch identities) is pinned to the releases measured, onetaskgraph v0.12.7 and ai-orchestrator 668dc496, each stated in the report and in every row's versions.json; the capture schema it shares with analyze.py is held by test_analyze.py, which builds every capture through sanitize.write.
# llmlint: ignore-file[modern_domain_modeling] These records are JSON lines passed by allowlist from the proxy's raw log to the sanitized captures; the typed reading of them is analyze.py, and remodelling the retained harness would make it no longer the code that took the captures.
"""Price every GraphQL request the measurement's runs sent, in GitHub's primary-limit points.

The plugin does not ask GitHub for `rateLimit { cost }`, and the bucket `rateLimit` and the
`x-ratelimit-used` header report is the user's, which every other session on this host draws
on at the same time, so neither says what one request cost. So, by the manager's ruling:

- every **query** is priced by re-sending its own recorded document and variables with
  `rateLimit(dryRun: true) { cost }` — GitHub's own calculation, without running it;
- every **mutation request** is priced at 1 point, which the dry run cannot verify; the smallest
  rise of `x-ratelimit-used` between two consecutive answers of one counter (another session
  only ever adds to a rise) is recorded beside it as that price's evidence.

`python3 pricing.py` reads the raw runs under the cache and writes `captures/pricing.json`:
`requests` maps `<capture>/<seq>` to each query's price, `operations` each operation's prices
seen, and `mutation_rises` every observed rise before a mutation's answer.
"""

from __future__ import annotations

import json
import os
import sys
import urllib.request
from collections import defaultdict
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

from run import CACHE  # the one definition of where raw runs live

OUT = Path(os.environ.get("SPIKE_LOGS") or HERE.parent / "captures") / "pricing.json"


def dry_run(document: str) -> str:
    root = document.index("{")
    # Aliased, so a document that already selects `rateLimit` does not conflict with it.
    return (
        document[: root + 1]
        + "spikePrice:rateLimit(dryRun:true){cost} "
        + document[root + 1 :]
    )


def records(log: Path) -> list[dict]:
    return sorted(
        (json.loads(line) for line in log.read_text().splitlines() if line.strip()),
        key=lambda r: r["end"],
    )


def mutation_rises(log: Path) -> list[int]:
    """Each rise of `x-ratelimit-used` to a mutation's answer from the answer before it on the
    same counter (the same reset), when that answer ended before the mutation was sent."""
    rises = []
    previous: dict[str, dict] = {}
    for record in records(log):
        limits = record.get("limits") or {}
        if (
            record["host"] == "refused"
            or limits.get("x-ratelimit-resource") != "graphql"
        ):
            continue
        used, reset = int(limits["x-ratelimit-used"]), limits["x-ratelimit-reset"]
        before = previous.get(reset)
        if record["kind"] == "mutation" and before and before["end"] <= record["start"]:
            rises.append(used - before["used"])
        previous[reset] = {"used": used, "end": record["end"]}
    return rises


def price(document: dict, token: str) -> int | None:
    request = urllib.request.Request(
        "https://api.github.com/graphql",
        data=json.dumps(
            {
                "query": dry_run(document["query"]),
                "variables": document.get("variables") or {},
            }
        ).encode(),
        headers={
            "Authorization": f"Bearer {token}",
            "Content-Type": "application/json",
        },
    )
    with urllib.request.urlopen(request, timeout=60) as response:
        answer = json.load(response)
    return ((answer.get("data") or {}).get("spikePrice") or {}).get("cost")


def main() -> None:
    token = os.environ.get("GH_PROJECTS_TOKEN", "").strip()
    if not token:
        sys.exit("pricing.py: GH_PROJECTS_TOKEN is not set")
    priced = json.loads(OUT.read_text()) if OUT.exists() else {}
    requests: dict[str, int] = priced.get("requests", {})
    rises: dict[str, list[int]] = {}
    operations: dict[str, set[int]] = defaultdict(set)
    for log in sorted(
        (CACHE / os.environ.get("SPIKE_RUNS_DIR", "runs")).glob("*/proxy.jsonl")
    ):
        run = log.parent.name
        bodies = log.with_suffix(".bodies")
        for record in records(log):
            if record["kind"] != "query" or record["host"] == "refused":
                continue
            key = f"{run}/{record['seq']}"
            if key not in requests:
                request = bodies / f"{record['seq']:06d}.req"
                cost = (
                    price(json.loads(request.read_text()), token)
                    if request.exists()
                    else None
                )
                if cost is None:
                    sys.exit(
                        f"pricing.py: GitHub returned no dry-run price for {key} ({record['op']})"
                    )
                requests[key] = cost
            operations[record["op"]].add(requests[key])
        rises[run] = mutation_rises(log)
    out = {
        "requests": dict(sorted(requests.items())),
        "operations": {op: sorted(prices) for op, prices in sorted(operations.items())},
        "mutation_rises": rises,
        "mutation_price": 1,
    }
    OUT.write_text(json.dumps(out, indent=1, sort_keys=True) + "\n")
    for op, prices in out["operations"].items():
        print(f"{op}: {prices}")


if __name__ == "__main__":
    main()
