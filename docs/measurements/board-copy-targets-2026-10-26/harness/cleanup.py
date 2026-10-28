"""Delete every GitHub issue this measurement created: each `createIssue` answer (aliased or
not) its runs' raw logs hold.

Each id is read back first and deleted only when its title is one the generated plan carries
and its repository is the test board's scratch repository, so nothing of ai-orchestrator's, or
anybody else's, can be touched whatever a log says. `only` (`bare` or `host`) limits it to the
issues one side's runs created, and retires only that side's folder.
"""

from __future__ import annotations

import json
import time
import urllib.error
import urllib.request
from pathlib import Path

REPOSITORIES = {"nickderobertis/onetaskgraph-live-scratch"}
TITLES = ("feat(spike): seeded task ", "Spike workload bcs-", "Design: bcs-", "DESIGN: Design: bcs-")
# Deletes are content-changing requests: spaced well inside GitHub's secondary limit.
SPACING = 2.0


class Client:
    def __init__(self, endpoint: str, token: str) -> None:
        self.endpoint = endpoint
        self.token = token
        self.last_write = 0.0

    def send(self, query: str, variables: dict, write: bool = False) -> dict:
        if write:
            time.sleep(max(0.0, self.last_write + SPACING - time.time()))
        request = urllib.request.Request(
            self.endpoint,
            data=json.dumps({"query": query, "variables": variables}).encode(),
            headers={"Authorization": f"Bearer {self.token}", "Content-Type": "application/json"},
        )
        try:
            with urllib.request.urlopen(request, timeout=120) as response:
                body = json.load(response)
        except urllib.error.HTTPError as error:
            body = json.loads(error.read() or b"{}")
        if write:
            self.last_write = time.time()
        return body


def created_ids(runs: Path, only: str | None = None) -> list[str]:
    """Every issue id a `createIssue` (or an aliased one) answered in the raw run logs."""
    ids: list[str] = []
    for log in sorted(runs.glob(f"{only + '-' if only else ''}*/proxy.jsonl")):
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
    return list(dict.fromkeys(ids))


def _creates(record: dict, bodies: Path) -> bool:
    request = bodies / f"{record['seq']:06d}.req"
    answer = bodies / f"{record['seq']:06d}.resp"
    if not request.exists() or not answer.exists():
        return False
    return "createIssue" in json.loads(request.read_text()).get("query", "")


def run(run_dir: Path, endpoint: str, token: str, state: Path, only: str | None = None) -> None:
    client = Client(endpoint, token)
    outcome = {"deleted": 0, "refused": 0, "gone": 0, "failed": 0}
    for issue_id in created_ids(run_dir.parent, only):
        body = client.send(
            "query CleanupRead($id: ID!) { node(id: $id) { ... on Issue { title "
            "repository { nameWithOwner } } } }",
            {"id": issue_id},
        )
        node = (body.get("data") or {}).get("node")
        if not node:
            outcome["gone"] += 1
            continue
        repository = node.get("repository", {}).get("nameWithOwner", "").lower()
        if repository not in REPOSITORIES or not node.get("title", "").startswith(TITLES):
            outcome["refused"] += 1
            continue
        body = client.send(
            "mutation CleanupDelete($id: ID!) { deleteIssue(input: {issueId: $id}) "
            "{ clientMutationId } }",
            {"id": issue_id},
            write=True,
        )
        outcome["failed" if body.get("errors") else "deleted"] += 1
    (run_dir / "cleanup.json").write_text(json.dumps(outcome | {"only": only or "all"}, indent=1))
    print(f"  cleanup: {outcome}", flush=True)
    if outcome["failed"] == 0:
        # Retire every cleaned plan's folder, so its next first copy starts afresh.
        stamp = run_dir.name.rsplit("-", 1)[-1]
        for plan in sorted(p for p in state.iterdir() if p.is_dir() and ".cleaned" not in p.name):
            if only in (None, plan.name):
                plan.rename(plan.with_name(f"{plan.name}.cleaned-{stamp}"))
