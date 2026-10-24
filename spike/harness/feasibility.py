"""The feasibility spike's live probes on the test board: `run.py probe-alias-pricing` and
`run.py probe-placement`.

`probe-alias-pricing` (read-only) prices the plugin's own documents (`documents.json`) with
GitHub's `rateLimit(dryRun: true) { cost }`, at each alias count a copy can send, and prices
two coalesced documents — a listing and a lookup or a placement read merged into one request.
Then it runs each coalesced document once for real, asking `rateLimit { cost }` in the same
request, so the dry-run figure is checked against what GitHub charged.

`probe-placement` creates disposable issues in the scratch repository, every one deleted
before it returns (and recorded first, so `cleanup` can finish the job):

1. one parent created with `projectV2Ids`, then eight children in one aliased request with
   `projectV2Ids` and `parentIssueId`, as a coalesced first copy would create them — does
   either answer carry the board item, and when does each item appear (polled every
   `POLL` seconds, each poll one aliased read whose cost GitHub states)?
2. `addProjectV2ItemById` sent immediately after a create with `projectV2Ids`, before
   GitHub's own placement has landed, and again for an issue already placed — does either
   leave the issue on the board twice?
3. one request carrying every field value and every blocked-by edge the plan writes
   (9 + 10 aliased mutations of two kinds), as a coalesced write would send them.
4. one `subIssues` read of the parent: does it answer every child, so a document copy can
   resolve every task reference from it?

`feasibility.json` keeps counts, offsets, costs, error types and answer shapes: never a
title, a body or an id.
"""

from __future__ import annotations

import json
import time
from pathlib import Path

import pricing
import probes
import workload

HERE = Path(__file__).resolve().parent
DOCS = json.loads((HERE / "documents.json").read_text())
POLL = 0.5
PLACEMENT_TIMEOUT = 45.0
TITLE = "bcg-probe"
# The plan's ten blocked-by edges among its eight tasks, by index (the parent is 0).
EDGES = workload.TASK_EDGES


# --- documents ------------------------------------------------------------------------------


def split(document: str) -> tuple[list[str], str, dict[str, str]]:
    """A `query(<defs>){<body>} fragment ...` document as its variable definitions, its
    top-level selection and its fragments by name."""
    open_paren = document.index("(")
    depth, i = 0, open_paren
    while True:
        depth += {"(": 1, ")": -1}.get(document[i], 0)
        if depth == 0:
            break
        i += 1
    defs_text = document[open_paren + 1 : i]
    defs, current, brackets = [], "", 0
    for char in defs_text:
        brackets += {"[": 1, "]": -1}.get(char, 0)
        if char == "," and brackets == 0:
            defs.append(current.strip())
            current = ""
        else:
            current += char
    if current.strip():
        defs.append(current.strip())
    start = document.index("{", i)
    depth, j = 0, start
    while True:
        depth += {"{": 1, "}": -1}.get(document[j], 0)
        if depth == 0:
            break
        j += 1
    body = document[start + 1 : j]
    fragments: dict[str, str] = {}
    rest = document[j + 1 :]
    for chunk in rest.split("fragment ")[1:]:
        fragments[chunk.split(" ", 1)[0]] = "fragment " + chunk.strip()
    return defs, body, fragments


def merge(*documents: str) -> str:
    """One query selecting everything each of `documents` selects: variables with the same
    name and type shared, fragments with the same name kept once."""
    defs: dict[str, str] = {}
    bodies, fragments = [], {}
    for document in documents:
        d, body, f = split(document)
        for definition in d:
            name = definition.split(":", 1)[0].strip()
            if name in defs and defs[name].split("=")[0] != definition.split("=")[0]:
                raise ValueError(f"{name} is declared two ways")
            defs.setdefault(name, definition)
        bodies.append(body)
        fragments.update(f)
    return (
        "query("
        + ",".join(defs.values())
        + "){"
        + "\n".join(bodies)
        + "} "
        + " ".join(fragments.values())
    )


def lookup_variables(
    owner: str, number: int, repo: str, asks: int, first: int = 3
) -> dict:
    out = {
        "owner": owner,
        "number": number,
        "type": "ISSUE",
        "originFirst": first,
        "nestedFirst": 50,
        "boardItems": 3,
        "duplicates": True,
        "repositoryOwner": repo.split("/")[0],
        "repositoryName": repo.split("/")[1],
    }
    for k in range(10):
        origin = f"spike-md:{TITLE}/t{k}"
        out[f"filter{k}"] = f'onetaskgraph.origin:"{origin}"'
        out[f"search{k}"] = f'project:{owner}/{number} is:issue in:body "{origin}"'
        out[f"ask{k}"] = k < asks
    return out


def lookup_one_variables(owner: str, number: int) -> dict:
    origin = f"spike-md:{TITLE}/t0"
    return {
        "owner": owner,
        "number": number,
        "type": "ISSUE",
        "originFirst": 3,
        "nestedFirst": 50,
        "boardItems": 3,
        "duplicates": True,
        "filter": f'onetaskgraph.origin:"{origin}"',
        "search": f'project:{owner}/{number} is:issue in:body "{origin}"',
        "itemsAfter": None,
        "searchAfter": None,
    }


def placement_variables(ids: list[str], reads: int) -> dict:
    out: dict = {"boardItems": 3, "statusName": "Status"}
    for k in range(21):
        out[f"id{k}"] = ids[k % len(ids)]
        out[f"read{k}"] = k < reads
    return out


def sub_issue_variables(parent: str, first: int = 23) -> dict:
    return {
        "id": parent,
        "memberFirst": first,
        "after": None,
        "nestedFirst": 50,
        "boardItems": 3,
        "duplicates": True,
    }


def issue_variables(issue: str) -> dict:
    return {
        "id": issue,
        "first": 100,
        "nestedFirst": 50,
        "boardItems": 3,
        "duplicates": True,
    }


def priced(client: probes.Client, label: str, document: str, variables: dict) -> dict:
    start = time.time()
    body, headers = client.send(pricing.dry_run(document), variables)
    cost = ((body.get("data") or {}).get("rateLimit") or {}).get("cost")
    return {
        "variant": label,
        "dry_run_cost": cost,
        "errors": probes.errors(body),
        "seconds": round(time.time() - start, 3),
        "limits": probes.limits(headers),
    }


def charged(client: probes.Client, label: str, document: str, variables: dict) -> dict:
    """Run `document` for real, asking GitHub what it charged in the same request."""
    root = document.index("{")
    asked = (
        document[: root + 1]
        + "rateLimit{cost used remaining resetAt} "
        + document[root + 1 :]
    )
    start = time.time()
    body, headers = client.send(asked, variables)
    rate = (body.get("data") or {}).get("rateLimit") or {}
    return {
        "variant": label,
        "charged_cost": rate.get("cost"),
        "errors": probes.errors(body),
        "seconds": round(time.time() - start, 3),
        "limits": probes.limits(headers),
        "answer_shape": probes.shape(
            {k: v for k, v in (body.get("data") or {}).items() if k != "rateLimit"}
        ),
    }


def board(client: probes.Client, owner: str, number: int, repo: str) -> dict:
    body, _ = client.send(
        "query ProbeIds($owner: String!, $name: String!, $number: Int!) {"
        " repository(owner: $owner, name: $name) { id }"
        " user(login: $owner) { projectV2(number: $number) { id"
        ' field(name: "Status") { ... on ProjectV2SingleSelectField { id options { id name } } } } } }',
        {"owner": owner, "name": repo.split("/")[1], "number": number},
    )
    project = body["data"]["user"]["projectV2"]
    return {
        "repo": body["data"]["repository"]["id"],
        "project": project["id"],
        "field": project["field"]["id"],
        "option": project["field"]["options"][0]["id"],
    }


# --- probe-alias-pricing --------------------------------------------------------------------


def alias_pricing(
    run_dir: Path, endpoint: str, token: str, owner: str, number: int, repo: str
) -> dict:
    client = probes.Client(endpoint, token)
    ids = board(client, owner, number, repo)
    anchor = ids["repo"]  # any node id prices a document; nothing it names is read
    results: list[dict] = []
    for asks in (1, 2, 3, 4, 5, 6, 8, 10):
        results.append(
            priced(
                client,
                f"ORIGIN_LOOKUPS asks={asks}",
                DOCS["ORIGIN_LOOKUPS"],
                lookup_variables(owner, number, repo, asks),
            )
        )
    results.append(
        priced(
            client,
            "ORIGIN_LOOKUPS asks=10 originFirst=1",
            DOCS["ORIGIN_LOOKUPS"],
            lookup_variables(owner, number, repo, 10, first=1),
        )
    )
    results.append(
        priced(
            client,
            "ORIGIN_LOOKUP",
            DOCS["ORIGIN_LOOKUP"],
            lookup_one_variables(owner, number),
        )
    )
    for reads in (1, 2, 9, 10, 21):
        results.append(
            priced(
                client,
                f"CREATED_BOARD_ITEMS reads={reads}",
                DOCS["CREATED_BOARD_ITEMS"],
                placement_variables([anchor], reads),
            )
        )
    for first in (10, 23, 50, 100):
        results.append(
            priced(
                client,
                f"SUB_ISSUES memberFirst={first}",
                DOCS["SUB_ISSUES"],
                sub_issue_variables(anchor, first),
            )
        )
    results.append(priced(client, "ISSUE", DOCS["ISSUE"], issue_variables(anchor)))
    coalesced = {
        "SUB_ISSUES + ORIGIN_LOOKUPS asks=1": (
            merge(DOCS["SUB_ISSUES"], DOCS["ORIGIN_LOOKUPS"]),
            sub_issue_variables(anchor) | lookup_variables(owner, number, repo, 1),
        ),
        "SUB_ISSUES + ORIGIN_LOOKUPS asks=10": (
            merge(DOCS["SUB_ISSUES"], DOCS["ORIGIN_LOOKUPS"]),
            sub_issue_variables(anchor) | lookup_variables(owner, number, repo, 10),
        ),
        "SUB_ISSUES + CREATED_BOARD_ITEMS reads=1": (
            merge(DOCS["SUB_ISSUES"], DOCS["CREATED_BOARD_ITEMS"]),
            sub_issue_variables(anchor) | placement_variables([anchor], 1),
        ),
    }
    for label, (document, variables) in coalesced.items():
        results.append(priced(client, label, document, variables))
    executed = [
        charged(client, label, document, variables)
        for label, (document, variables) in coalesced.items()
    ]
    executed.append(
        charged(
            client,
            "ORIGIN_LOOKUPS asks=10",
            DOCS["ORIGIN_LOOKUPS"],
            lookup_variables(owner, number, repo, 10),
        )
    )
    out = {"dry_runs": results, "executed": executed}
    (run_dir / "feasibility.json").write_text(json.dumps(out, indent=1))
    return out


# --- probe-placement ------------------------------------------------------------------------

ITEMS = "projectItems(first: 5) { nodes { id project { id } } }"


def on_board(issue: dict | None, project: str) -> list[str]:
    nodes = ((issue or {}).get("projectItems") or {}).get("nodes") or []
    return [n["id"] for n in nodes if (n.get("project") or {}).get("id") == project]


def poll_items(
    client: probes.Client, issues: list[str], project: str
) -> tuple[dict, int]:
    """One aliased read of every issue's board items, with what GitHub charged for it."""
    aliases = " ".join(
        f"i{k}: node(id: $id{k}) {{ ... on Issue {{ {ITEMS} }} }}"
        for k in range(len(issues))
    )
    defs = ", ".join(f"$id{k}: ID!" for k in range(len(issues)))
    body, _ = client.send(
        f"query ProbePlaced({defs}) {{ rateLimit {{ cost }} {aliases} }}",
        {f"id{k}": issue for k, issue in enumerate(issues)},
    )
    data = body.get("data") or {}
    return (
        {issue: on_board(data.get(f"i{k}"), project) for k, issue in enumerate(issues)},
        (data.get("rateLimit") or {}).get("cost"),
    )


def aliased_create(
    client: probes.Client, record: dict, inputs: list[dict], label: str
) -> tuple[list[dict], dict, float]:
    defs = ", ".join(f"$c{k}: CreateIssueInput!" for k in range(len(inputs)))
    aliases = " ".join(
        f"c{k}: createIssue(input: $c{k}) {{ issue {{ id parent {{ id }} {ITEMS} }} }}"
        for k in range(len(inputs))
    )
    body, headers = client.send(
        f"mutation ProbeCreate{label}({defs}) {{ {aliases} }}",
        {f"c{k}": value for k, value in enumerate(inputs)},
        write=True,
    )
    end = time.time()
    data = body.get("data") or {}
    issues = [
        ((data.get(f"c{k}") or {}).get("issue") or {}) for k in range(len(inputs))
    ]
    for issue in issues:
        if issue.get("id"):
            record["created"].append(issue["id"])
    record["created_file"].write_text(json.dumps(record["created"]))
    finding = {
        "probe": f"aliased createIssue x{len(inputs)} ({label})",
        "errors": probes.errors(body),
        "limits": probes.limits(headers),
    }
    return issues, finding, end


def placement(
    run_dir: Path, endpoint: str, token: str, owner: str, number: int, repo: str
) -> dict:
    client = probes.Client(endpoint, token)
    created_file = run_dir.parent.parent / "state" / "probe-issues.json"
    created_file.parent.mkdir(parents=True, exist_ok=True)
    # Whatever an interrupted run left recorded is deleted with this run's own issues.
    left = json.loads(created_file.read_text()) if created_file.exists() else []
    record: dict = {"created": list(left), "created_file": created_file}
    ids = board(client, owner, number, repo)
    project = ids["project"]
    findings: list[dict] = []
    out: dict = {"findings": findings}
    try:
        # 1. The parent, then eight children in one request, as a coalesced first copy sends them.
        [parent], finding, parent_end = aliased_create(
            client,
            record,
            [
                {
                    "repositoryId": ids["repo"],
                    "title": f"{TITLE} parent",
                    "projectV2Ids": [project],
                }
            ],
            "Parent",
        )
        finding["board_item_in_answer"] = bool(on_board(parent, project))
        findings.append(finding)
        children, finding, children_end = aliased_create(
            client,
            record,
            [
                {
                    "repositoryId": ids["repo"],
                    "title": f"{TITLE} child {k}",
                    "projectV2Ids": [project],
                    "parentIssueId": parent["id"],
                }
                for k in range(1, 9)
            ],
            "Children",
        )
        finding["board_items_in_answer"] = sum(
            1 for c in children if on_board(c, project)
        )
        finding["children_with_parent"] = sum(
            1 for c in children if (c.get("parent") or {}).get("id") == parent["id"]
        )
        findings.append(finding)
        issues = [parent["id"]] + [c["id"] for c in children]
        created_at = {parent["id"]: parent_end} | {
            c["id"]: children_end for c in children
        }
        seen: dict[str, float] = {}
        items: dict[str, list[str]] = {}
        polls, costs = 0, []
        deadline = time.time() + PLACEMENT_TIMEOUT
        while len(seen) < len(issues) and time.time() < deadline:
            answer, cost = poll_items(client, issues, project)
            polls += 1
            costs.append(cost)
            now = time.time()
            for issue, found in answer.items():
                if found and issue not in seen:
                    seen[issue] = now - created_at[issue]
                    items[issue] = found
            if len(seen) < len(issues):
                time.sleep(POLL)
        offsets = sorted(seen.values())
        out["placement"] = {
            "issues": len(issues),
            "placed": len(seen),
            "polls": polls,
            "poll_costs": sorted(set(costs)),
            "parent_seconds_after_its_answer": round(seen.get(parent["id"], -1), 3),
            "children_seconds_after_their_answer": [
                round(seen[c["id"]], 3) for c in children if c["id"] in seen
            ],
            "max_seconds": round(offsets[-1], 3) if offsets else None,
            "items_per_issue": sorted({len(v) for v in items.values()}),
        }
        # 2. addProjectV2ItemById before GitHub's own placement lands, and after it has.
        racing, finding, _ = aliased_create(
            client,
            record,
            [
                {
                    "repositoryId": ids["repo"],
                    "title": f"{TITLE} race {k}",
                    "projectV2Ids": [project],
                }
                for k in (1, 2)
            ],
            "Race",
        )
        findings.append(finding)
        add = (
            "mutation ProbeAdd($p: ID!, $a: ID!, $b: ID!, $c: ID!) { "
            + " ".join(
                f"{alias}: addProjectV2ItemById(input: {{projectId: $p, contentId: ${alias}}}) {{ item {{ id }} }}"
                for alias in ("a", "b", "c")
            )
            + " }"
        )
        client.last_write = (
            0.0  # not a content creation: sent at once, inside the race window
        )
        body, headers = client.send(
            add,
            {
                "p": project,
                "a": racing[0]["id"],
                "b": racing[1]["id"],
                "c": parent["id"],
            },
            write=True,
        )
        added = {
            alias: (((body.get("data") or {}).get(alias) or {}).get("item") or {}).get(
                "id"
            )
            for alias in ("a", "b", "c")
        }
        findings.append(
            {
                "probe": "addProjectV2ItemById x3 (2 racing, 1 placed)",
                "errors": probes.errors(body),
                "limits": probes.limits(headers),
            }
        )
        time.sleep(10)
        answer, _ = poll_items(
            client, [racing[0]["id"], racing[1]["id"], parent["id"]], project
        )
        out["idempotence"] = {
            "racing_items_after_10s": [
                len(answer[racing[0]["id"]]),
                len(answer[racing[1]["id"]]),
            ],
            "racing_item_is_the_added_one": [
                answer[racing[0]["id"]] == [added["a"]],
                answer[racing[1]["id"]] == [added["b"]],
            ],
            "placed_items_after_add": len(answer[parent["id"]]),
            "placed_add_returned_existing_item": added["c"]
            in items.get(parent["id"], []),
        }
        # 3. Every field value and every edge of the plan in one request.
        item_ids = [items[i][0] for i in issues if i in items]
        defs = ["$p: ID!", "$f: ID!", "$o: String!"] + [
            f"$i{k}: ID!" for k in range(len(item_ids))
        ]
        used = sorted({k for edge in EDGES for k in edge})
        defs += [f"$x{k}: ID!" for k in used]
        writes = [
            f"v{k}: updateProjectV2ItemFieldValue(input: {{projectId: $p, itemId: $i{k}, fieldId: $f, value: {{singleSelectOptionId: $o}}}}) {{ projectV2Item {{ id }} }}"
            for k in range(len(item_ids))
        ]
        writes += [
            f"e{n}: addBlockedBy(input: {{issueId: $x{t}, blockingIssueId: $x{b}}}) {{ issue {{ id }} }}"
            for n, (t, b) in enumerate(EDGES)
        ]
        variables = {"p": project, "f": ids["field"], "o": ids["option"]}
        variables |= {f"i{k}": v for k, v in enumerate(item_ids)} | {
            f"x{k}": issues[k] for k in used
        }
        start = time.time()
        body, headers = client.send(
            f"mutation ProbeCoalesced({', '.join(defs)}) {{ {' '.join(writes)} }}",
            variables,
            write=True,
        )
        data = body.get("data") or {}
        findings.append(
            {
                "probe": f"one request of {len(writes)} aliased mutations: {len(item_ids)} field values + {len(EDGES)} blocked-by edges",
                "answered": sum(1 for alias in data if data[alias]),
                "errors": probes.errors(body),
                "seconds": round(time.time() - start, 3),
                "limits": probes.limits(headers),
            }
        )
        # 4. One subIssues read of the parent resolves every child's destination id and URL.
        read = charged(
            client,
            "SUB_ISSUES of the parent",
            DOCS["SUB_ISSUES"],
            sub_issue_variables(parent["id"]),
        )
        out["reference_resolution"] = read
    finally:
        out["cleanup"] = delete(client, record, ids["repo"])
    (run_dir / "feasibility.json").write_text(json.dumps(out, indent=1))
    return out


DELETE_BATCH = (
    5  # 11 aliased deleteIssue in one request met RESOURCE_LIMITS_EXCEEDED at the 8th
)


def delete(client: probes.Client, record: dict, repo_id: str) -> dict:
    """Delete every issue this probe created (and any an interrupted run left recorded),
    `DELETE_BATCH` to a request, after reading each back and keeping only those in the
    scratch repository titled as this probe titles them; an issue already gone is dropped."""
    created = list(dict.fromkeys(record["created"]))
    if not created:
        return {"deleted": 0}
    reads = " ".join(
        f"r{k}: node(id: $r{k}) {{ ... on Issue {{ id title repository {{ id }} }} }}"
        for k in range(len(created))
    )
    body, _ = client.send(
        f"query ProbeOwn({', '.join(f'$r{k}: ID!' for k in range(len(created)))}) {{ {reads} }}",
        {f"r{k}": v for k, v in enumerate(created)},
    )
    data = body.get("data") or {}
    found = {k: data.get(f"r{k}") for k in range(len(created))}
    gone = [created[k] for k, issue in found.items() if not issue]
    own = [
        issue["id"]
        for issue in found.values()
        if issue
        and issue.get("title", "").startswith(TITLE)
        and (issue.get("repository") or {}).get("id") == repo_id
    ]
    refused = len(created) - len(gone) - len(own)
    deleted, errors, limits = [], [], {}
    for start in range(0, len(own), DELETE_BATCH):
        batch = own[start : start + DELETE_BATCH]
        aliases = " ".join(
            f"d{k}: deleteIssue(input: {{issueId: $d{k}}}) {{ clientMutationId }}"
            for k in range(len(batch))
        )
        body, headers = client.send(
            f"mutation ProbeDelete({', '.join(f'$d{k}: ID!' for k in range(len(batch)))}) {{ {aliases} }}",
            {f"d{k}": v for k, v in enumerate(batch)},
            write=True,
        )
        failed = {tuple(e.get("path") or [])[:1] for e in body.get("errors") or []}
        deleted += [
            v
            for k, v in enumerate(batch)
            if (f"d{k}",) not in failed
            and (body.get("data") or {}).get(f"d{k}") is not None
        ]
        errors += probes.errors(body)
        limits = probes.limits(headers)
    record["created"] = [i for i in created if i not in deleted and i not in gone]
    record["created_file"].write_text(json.dumps(record["created"]))
    return {
        "deleted": len(deleted),
        "already_gone": len(gone),
        "refused": refused,
        "left": len(record["created"]),
        "errors": errors,
        "limits": limits,
    }
