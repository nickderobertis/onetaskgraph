"""What GitHub does today with `createIssue`'s `projectV2Ids` and `parentIssueId`, and with
one request carrying several aliased mutations — on disposable test-board issues, every one
deleted before this returns (and recorded first, so `cleanup` can finish the job if a run
is interrupted).

Every request goes through the recording proxy; `probes.json` beside the raw log keeps what
each answer's *shape* was (keys, nulls, error types, item counts), never a title or an id.
Content-creating requests are spaced `SPACING` seconds apart, well inside GitHub's
documented 80-per-minute content-creation limit, so nothing here provokes the secondary
limiter that would hold the shared token for an hour.
"""

from __future__ import annotations

import json
import time
import urllib.error
import urllib.request
from pathlib import Path

SPACING = 2.0

ALIASED = """mutation Aliased($repo: ID!, $project: ID!, $field: ID!, $option: String!,
  $p1: ID!, $p2: ID!, $p3: ID!, $i1: ID!, $i2: ID!, $i3: ID!,
  $parent: ID!, $child: ID!, $blocked: ID!, $blocking: ID!) {
  c1: createIssue(input: {repositoryId: $repo, title: "bcs-probe aliased c1"}) { issue { id } }
  c2: createIssue(input: {repositoryId: $repo, title: "bcs-probe aliased c2"}) { issue { id } }
  c3: createIssue(input: {repositoryId: $repo, title: "bcs-probe aliased c3"}) { issue { id } }
  a1: addProjectV2ItemById(input: {projectId: $project, contentId: $p1}) { item { id } }
  a2: addProjectV2ItemById(input: {projectId: $project, contentId: $p2}) { item { id } }
  a3: addProjectV2ItemById(input: {projectId: $project, contentId: $p3}) { item { id } }
  u1: updateProjectV2ItemFieldValue(input: {projectId: $project, itemId: $i1, fieldId: $field,
    value: {singleSelectOptionId: $option}}) { projectV2Item { id } }
  u2: updateProjectV2ItemFieldValue(input: {projectId: $project, itemId: $i2, fieldId: $field,
    value: {singleSelectOptionId: $option}}) { projectV2Item { id } }
  u3: updateProjectV2ItemFieldValue(input: {projectId: $project, itemId: $i3, fieldId: $field,
    value: {singleSelectOptionId: $option}}) { projectV2Item { id } }
  s1: addSubIssue(input: {issueId: $parent, subIssueId: $child}) { issue { id } subIssue { id } }
  b1: addBlockedBy(input: {issueId: $blocked, blockingIssueId: $blocking}) {
    issue { id } blockingIssue { id } }
}"""


def shape(value: object) -> object:
    """`value` with every leaf replaced by its type: what an answer looked like, not said."""
    if isinstance(value, dict):
        return {k: shape(v) for k, v in value.items()}
    if isinstance(value, list):
        return [shape(v) for v in value[:3]] + (
            [f"... {len(value)} items"] if value else []
        )
    return None if value is None else type(value).__name__


class Client:
    def __init__(self, endpoint: str, token: str) -> None:
        self.endpoint = endpoint
        self.token = token
        self.last_write = 0.0

    def send(
        self, query: str, variables: dict, write: bool = False
    ) -> tuple[dict, dict]:
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
                body, headers = json.load(response), dict(response.headers)
        except urllib.error.HTTPError as error:
            body, headers = json.loads(error.read() or b"{}"), dict(error.headers)
        if write:
            self.last_write = time.time()
        return body, headers


def limits(headers: dict) -> dict:
    return {
        k.lower(): v
        for k, v in headers.items()
        if k.lower().startswith("x-ratelimit") or k.lower() == "retry-after"
    }


def errors(body: dict) -> list[dict]:
    return [
        {"type": e.get("type"), "path": e.get("path"), "message": e.get("message")}
        for e in body.get("errors") or []
    ]


def create(
    client: Client, record: dict, label: str, extra: str, variables: dict
) -> dict:
    query = (
        "mutation Create"
        + label
        + "($input: CreateIssueInput!) { createIssue(input: $input) "
        "{ issue { id number " + extra + " } } }"
    )
    body, headers = client.send(query, {"input": variables}, write=True)
    issue = ((body.get("data") or {}).get("createIssue") or {}).get("issue")
    if issue:
        record["created"].append(issue["id"])
        record["created_file"].write_text(json.dumps(record["created"]))
    record["findings"].append(
        {
            "probe": f"createIssue {label}",
            "shape": shape(body),
            "errors": errors(body),
            "limits": limits(headers),
        }
    )
    return issue or {}


def run(
    run_dir: Path, endpoint: str, token: str, owner: str, number: int, repo: str
) -> None:
    client = Client(endpoint, token)
    created_file = run_dir.parent.parent / "state" / "probe-issues.json"
    created_file.parent.mkdir(parents=True, exist_ok=True)
    record: dict = {"created": [], "created_file": created_file, "findings": []}
    name = repo.split("/")[1]
    ids, _ = client.send(
        "query ProbeIds($owner: String!, $name: String!, $number: Int!) {"
        " repository(owner: $owner, name: $name) { id }"
        " user(login: $owner) { projectV2(number: $number) { id"
        ' field(name: "Status") { ... on ProjectV2SingleSelectField { id options { id } } } } } }',
        {"owner": owner, "name": name, "number": number},
    )
    repo_id = ids["data"]["repository"]["id"]
    project = ids["data"]["user"]["projectV2"]
    project_id, field = project["id"], project["field"]
    items = "projectItems(first: 10) { totalCount nodes { id project { id } } }"
    try:
        # 1. createIssue with projectV2Ids: is a board item returned, and does one exist?
        s1 = create(
            client,
            record,
            "WithProjectV2Ids",
            items,
            {
                "repositoryId": repo_id,
                "title": "bcs-probe projectV2Ids",
                "projectV2Ids": [project_id],
            },
        )
        time.sleep(3)
        later, headers = client.send(
            "query ProbeItems($id: ID!) { node(id: $id) { ... on Issue { "
            + items
            + " } } }",
            {"id": s1["id"]},
        )
        later_items = (later.get("data") or {}).get("node") or {}
        record["findings"].append(
            {
                "probe": "projectItems of the projectV2Ids issue, 3 s later",
                "shape": shape(later),
                "on_board": any(
                    n["project"]["id"] == project_id
                    for n in (later_items.get("projectItems") or {}).get("nodes") or []
                ),
                "limits": limits(headers),
            }
        )
        s1_item = next(
            (
                n["id"]
                for n in (later_items.get("projectItems") or {}).get("nodes") or []
                if n["project"]["id"] == project_id
            ),
            None,
        )
        if s1_item is None:
            body, headers = client.send(
                "mutation ProbeAdd($p: ID!, $c: ID!) { addProjectV2ItemById(input: "
                "{projectId: $p, contentId: $c}) { item { id } } }",
                {"p": project_id, "c": s1["id"]},
                write=True,
            )
            record["findings"].append(
                {
                    "probe": "addProjectV2ItemById of the projectV2Ids issue",
                    "shape": shape(body),
                    "errors": errors(body),
                    "limits": limits(headers),
                }
            )
            s1_item = (
                ((body.get("data") or {}).get("addProjectV2ItemById") or {}).get("item")
                or {}
            ).get("id")
        # 2. createIssue with parentIssueId.
        s2 = create(
            client,
            record,
            "WithParentIssueId",
            "parent { id } ",
            {
                "repositoryId": repo_id,
                "title": "bcs-probe parentIssueId",
                "parentIssueId": s1["id"],
            },
        )
        # 3. Prepare the aliased request's operands, one plain mutation each.
        plain = [
            create(
                client,
                record,
                "Plain",
                "",
                {"repositoryId": repo_id, "title": f"bcs-probe p{k}"},
            )
            for k in range(4)
        ]
        board_items = [s1_item]
        for issue in (s2, plain[0]):
            body, headers = client.send(
                "mutation ProbeAdd($p: ID!, $c: ID!) { addProjectV2ItemById(input: "
                "{projectId: $p, contentId: $c}) { item { id } } }",
                {"p": project_id, "c": issue["id"]},
                write=True,
            )
            board_items.append(body["data"]["addProjectV2ItemById"]["item"]["id"])
        # 4. One request, eleven aliased mutations.
        variables = {
            "repo": repo_id,
            "project": project_id,
            "field": field["id"],
            "option": field["options"][0]["id"],
            "p1": plain[1]["id"],
            "p2": plain[2]["id"],
            "p3": plain[3]["id"],
            "i1": board_items[0],
            "i2": board_items[1],
            "i3": board_items[2],
            "parent": plain[1]["id"],
            "child": plain[2]["id"],
            "blocked": plain[3]["id"],
            "blocking": plain[1]["id"],
        }
        body, headers = client.send(ALIASED, variables, write=True)
        for alias in ("c1", "c2", "c3"):
            issue = ((body.get("data") or {}).get(alias) or {}).get("issue")
            if issue:
                record["created"].append(issue["id"])
        created_file.write_text(json.dumps(record["created"]))
        record["findings"].append(
            {
                "probe": "one request of eleven aliased mutations",
                "shape": shape(body),
                "errors": errors(body),
                "limits": limits(headers),
            }
        )
    finally:
        deleted = []
        for issue_id in list(record["created"]):
            body, headers = client.send(
                "mutation ProbeDelete($id: ID!) { deleteIssue(input: {issueId: $id}) "
                "{ clientMutationId } }",
                {"id": issue_id},
                write=True,
            )
            ok = not body.get("errors")
            deleted.append(ok)
            if ok:
                record["created"].remove(issue_id)
        created_file.write_text(json.dumps(record["created"]))
        record["findings"].append(
            {
                "probe": "deleteIssue of every probe issue",
                "deleted": sum(deleted),
                "failed": len(deleted) - sum(deleted),
            }
        )
        out = {k: v for k, v in record.items() if k == "findings"}
        (run_dir / "probes.json").write_text(json.dumps(out, indent=1))


def utf8_body(target_bytes: int, target_chars: int, seed: int = 20261018) -> str:
    """Seeded prose of exactly `target_chars` characters and at least `target_bytes` UTF-8
    bytes: ASCII words, with enough three-byte punctuation (em dashes, curly quotes) to
    carry the bytes the characters alone do not."""
    import random

    rng = random.Random(seed)
    words = ["board", "copy", "walk", "page", "token", "write", "read", "plan", "task"]
    wide = ["—", "’", "“", "”"]
    needed = max(0, (target_bytes - target_chars + 1) // 2)  # each adds two bytes
    out: list[str] = []
    length = 0
    while length < target_chars - needed:
        word = rng.choice(words) + " "
        out.append(word)
        length += len(word)
    text = "".join(out)[: target_chars - needed]
    marks = "".join(rng.choice(wide) for _ in range(needed))
    step = max(1, len(text) // max(1, needed))
    mixed = "".join(
        text[i * step : (i + 1) * step] + (marks[i] if i < needed else "")
        for i in range(needed)
    )
    return (mixed + text[needed * step :])[:target_chars]


def body_limit(run_dir: Path, endpoint: str, token: str, owner: str, repo: str) -> None:
    """Whether GitHub takes a 77 KB issue body written in fewer than 65,536 characters,
    and refuses 66,000 ASCII characters: the limit's unit. Both issues are deleted."""
    client = Client(endpoint, token)
    created_file = run_dir.parent.parent / "state" / "probe-issues.json"
    record: dict = {"created": [], "created_file": created_file, "findings": []}
    ids, _ = client.send(
        "query ProbeRepo($owner: String!, $name: String!) {"
        " repository(owner: $owner, name: $name) { id } }",
        {"owner": owner, "name": repo.split("/")[1]},
    )
    repo_id = ids["data"]["repository"]["id"]
    cases = {
        "Wide77KB": utf8_body(77_000, 63_500),
        "Ascii66000": "x" * 66_000,
    }
    try:
        for label, body in cases.items():
            create(
                client,
                record,
                label,
                "",
                {
                    "repositoryId": repo_id,
                    "title": f"bcs-probe body {label}",
                    "body": body,
                },
            )
            record["findings"][-1]["body"] = {
                "bytes": len(body.encode()),
                "chars": len(body),
            }
    finally:
        for issue_id in list(record["created"]):
            answer, _ = client.send(
                "mutation ProbeDelete($id: ID!) { deleteIssue(input: {issueId: $id}) "
                "{ clientMutationId } }",
                {"id": issue_id},
                write=True,
            )
            if not answer.get("errors"):
                record["created"].remove(issue_id)
        created_file.write_text(json.dumps(record["created"]))
        record["findings"].append({"probe": "deleted", "left": len(record["created"])})
        out = {"findings": record["findings"]}
        (run_dir / "probes.json").write_text(json.dumps(out, indent=1))
