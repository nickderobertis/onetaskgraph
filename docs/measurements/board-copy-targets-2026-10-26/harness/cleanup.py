# llmlint: ignore-file[new_code_lands_in_a_project] This is the measurement record beside the captures it produced: run by hand against GitHub through measure.sh, never by a target. docs/measurements is no Nx project (nor is coalesced-members-dry-run/dry_run.py beside it), and making it one would put a live-GitHub harness inside affected selection.
# llmlint: ignore-file[async_typed_clients_at_boundaries] These sequential urllib and http.client calls are the clients that took the committed captures (the 2026-10-18 timing harness, adapted only where the adopted releases required); one request at a time is deliberate so a row's timing is not perturbed, and rewriting them would make this no longer the code that measured.
# llmlint: ignore-file[boundary_inputs_validated] Every input here is this harness's own raw log, its own state folder or the plan it generated moments earlier; the committed captures are validated where they are read, by analyze.py, and what leaves the raw logs is held by the sanitizer's allowlist and leak scan.
# llmlint: ignore-file[changed_behavior_has_e2e] Its proof is the live run itself, whose captures are committed and reconciled by test_analyze.py; the GitHub-facing parts can be driven only against GitHub's test board, which measure.sh does, and the sanitizer produces every capture those tests read.
# llmlint: ignore-file[contracts_have_one_source_or_a_drift_gate] What this mirrors (the plugin's documents, the host's configuration, the scratch identities) is pinned to the releases measured, onetaskgraph v0.12.7 and ai-orchestrator 668dc496, each stated in the report and in every row's versions.json; the capture schema it shares with analyze.py is held by test_analyze.py, which builds every capture through sanitize.write.
# llmlint: ignore-file[modern_domain_modeling] These records are JSON lines passed by allowlist from the proxy's raw log to the sanitized captures; the typed reading of them is analyze.py, and remodelling the retained harness would make it no longer the code that took the captures.
"""Delete every GitHub issue this measurement created: each `createIssue` answer (aliased or
not) its runs' raw logs hold.

Each id is read back first and deleted only when its title is one the generated plan carries
and its repository is the test board's scratch repository, so nothing of ai-orchestrator's, or
anybody else's, can be touched whatever a log says. `only` (`bare` or `host`) limits it to the
issues one side's runs created, and retires only that side's folder.
"""

from __future__ import annotations

import json
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

REPOSITORIES = {"nickderobertis/onetaskgraph-live-scratch"}
TITLES = (
    "feat(spike): seeded task ",
    "Spike workload bcs-",
    "Design: bcs-",
    "DESIGN: Design: bcs-",
)
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
            headers={
                "Authorization": f"Bearer {self.token}",
                "Content-Type": "application/json",
            },
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


def run(
    run_dir: Path, endpoint: str, token: str, state: Path, only: str | None = None
) -> None:
    client = Client(endpoint, token)
    outcome = {"deleted": 0, "refused": 0, "gone": 0, "failed": 0}
    for issue_id in created_ids(run_dir.parent, only):
        body = client.send(
            "query CleanupRead($id: ID!) { node(id: $id) { ... on Issue { title "
            "repository { nameWithOwner } } } }",
            {"id": issue_id},
        )
        if body.get("errors") or not isinstance(body.get("data"), dict):
            # An unanswered read says nothing about whether the issue is still there.
            outcome["failed"] += 1
            continue
        node = body["data"].get("node")
        if not node:
            outcome["gone"] += 1
            continue
        repository = node.get("repository", {}).get("nameWithOwner", "").lower()
        if repository not in REPOSITORIES or not node.get("title", "").startswith(
            TITLES
        ):
            outcome["refused"] += 1
            continue
        body = client.send(
            "mutation CleanupDelete($id: ID!) { deleteIssue(input: {issueId: $id}) "
            "{ clientMutationId } }",
            {"id": issue_id},
            write=True,
        )
        answered = (
            isinstance(body.get("data"), dict)
            and body["data"].get("deleteIssue") is not None
        )
        outcome["deleted" if answered and not body.get("errors") else "failed"] += 1
    (run_dir / "cleanup.json").write_text(
        json.dumps(outcome | {"only": only or "all"}, indent=1)
    )
    print(
        f"  cleanup: {outcome}",
        file=sys.stderr if outcome["failed"] else sys.stdout,
        flush=True,
    )
    if outcome["failed"]:
        raise SystemExit(
            f"cleanup: {outcome['failed']} delete(s) failed; run `measure.sh cleanup` again once "
            "GitHub answers, and every issue already gone is skipped"
        )
    # Retire every cleaned plan's folder, so its next first copy starts afresh.
    stamp = run_dir.name.rsplit("-", 1)[-1]
    for plan in sorted(
        p for p in state.iterdir() if p.is_dir() and ".cleaned" not in p.name
    ):
        if only in (None, plan.name):
            plan.rename(plan.with_name(f"{plan.name}.cleaned-{stamp}"))
