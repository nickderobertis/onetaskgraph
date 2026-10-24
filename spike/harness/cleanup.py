"""Delete every GitHub issue this spike created: each `createIssue` answer its runs' raw
logs hold (the generated plans' copies and the probes' issues), and any probe issue a run
recorded before it was interrupted.

Each id is read back first and deleted only when its title is one this spike generates
and its repository is one this spike has written to, so nothing of ai-orchestrator's, or
anybody else's, can be touched whatever a log says. (The core repository is on that list
because the first runs' generated records named it, and the plugin files an item's issue in
the repository the item names; those issues are this spike's own and are deleted too.)
"""

from __future__ import annotations

import json
import os
from pathlib import Path

from probes import Client

REPOSITORIES = {
    "nickderobertis/onetaskgraph-live-scratch",
    "nickderobertis/onetaskgraph",
}
TITLES = (
    "feat(spike): seeded task ",
    "Spike workload bcs-",
    "Design: bcs-",
    "DESIGN: Design: bcs-",
    "bcs-probe",
    "bcg-probe",  # the feasibility spike's probe-placement issues
)


def created_ids(runs: Path, state: Path) -> list[str]:
    """Every issue id a `createIssue` (or an aliased one) answered in any raw run log."""
    ids: list[str] = []
    extra = [Path(p) for p in os.environ.get("SPIKE_EXTRA_RUNS", "").split()]
    logs = sorted(runs.glob("*/proxy.jsonl")) + [l for e in extra for l in sorted(e.glob("*/proxy.jsonl"))]
    for log in logs:
        bodies = log.with_suffix(".bodies")
        for line in log.read_text().splitlines():
            record = json.loads(line)
            if record["kind"] != "mutation" or not _creates(record, bodies):
                continue
            answer = bodies / f"{record['seq']:06d}.resp"
            data = json.loads(answer.read_text() or "{}").get("data") or {}
            for value in data.values():
                issue = value.get("issue") if isinstance(value, dict) else None
                if isinstance(issue, dict) and "id" in issue:
                    ids.append(issue["id"])
    leftover = state / "probe-issues.json"
    if leftover.exists():
        ids += json.loads(leftover.read_text())
    return list(dict.fromkeys(ids))


def _creates(record: dict, bodies: Path) -> bool:
    request = bodies / f"{record['seq']:06d}.req"
    answer = bodies / f"{record['seq']:06d}.resp"
    if not request.exists() or not answer.exists():
        return False
    return "createIssue" in json.loads(request.read_text()).get("query", "")


def run(
    run_dir: Path, endpoint: str, token: str, state: Path, only: str | None = None
) -> None:
    client = Client(endpoint, token)
    outcome = {"deleted": 0, "refused": 0, "gone": 0, "skipped": 0, "failed": 0}
    for issue_id in created_ids(run_dir.parent, state):
        body, _ = client.send(
            "query CleanupRead($id: ID!) { node(id: $id) { ... on Issue { title "
            "repository { nameWithOwner } } } }",
            {"id": issue_id},
        )
        node = (body.get("data") or {}).get("node")
        if not node:
            outcome["gone"] += 1
            continue
        repository = node.get("repository", {}).get("nameWithOwner", "").lower()
        title = node.get("title", "")
        if repository not in REPOSITORIES or not title.startswith(TITLES):
            outcome["refused"] += 1
            continue
        if only and only not in title:
            outcome["skipped"] += 1
            continue
        body, _ = client.send(
            "mutation CleanupDelete($id: ID!) { deleteIssue(input: {issueId: $id}) "
            "{ clientMutationId } }",
            {"id": issue_id},
            write=True,
        )
        outcome["failed" if body.get("errors") else "deleted"] += 1
    (run_dir / "cleanup.json").write_text(json.dumps(outcome, indent=1))
    print(f"  cleanup: {outcome}", flush=True)
    if outcome["failed"] == 0:
        # Retire every cleaned plan, so its next `tb-first` starts from a fresh folder.
        stamp = run_dir.name.rsplit("-", 1)[-1]
        for plan in sorted(
            p for p in state.iterdir() if p.is_dir() and ".cleaned" not in p.name
        ):
            if only in (None, plan.name):
                plan.rename(plan.with_name(f"{plan.name}.cleaned-{stamp}"))
        leftover = state / "probe-issues.json"
        if leftover.exists():
            leftover.write_text("[]")
