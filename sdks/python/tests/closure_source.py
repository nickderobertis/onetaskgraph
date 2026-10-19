"""A board-like source answering both closure methods, over the stdio protocol.

The SDK tests need a source that answers `task status reconcile` and `task status complete`
with items, and none of the credential-free sources this build ships can: `local-md` and
`in-memory` read their status as their closure and declare neither, and the one that does —
GitHub Projects — needs a credential and a board. So this peer, spoken to over
`docs/plugin-protocol.md` exactly as the engine speaks to any plugin, holds a plan's home
project `home` and a task `t1`, and answers each closure method with both: the home as a
project item, the task as a task item (§4.23, §4.24).

It is spawned with a cleared environment (§3.1), so it imports only the standard library —
and none of the SDK's own models, so what the SDK decodes is not its own types read back.
"""

import json
import sys

KIND = "closure-source"
PROTOCOL_VERSION = 2

# What a write of each terminal category reads back as, as a board's options spell them.
NAMES = {"done": "Done", "cancelled": "Cancelled"}

HOME = {
    "id": "home",
    "title": "A plan's home",
    "content": None,
    "status": {"category": "in-progress", "name": "In Progress"},
    "labels": [],
    "url": None,
    "created_at": None,
    "updated_at": None,
}
TASK = {
    "id": "t1",
    "title": "A task of the plan",
    "content": None,
    "status": {"category": "todo", "name": "Todo"},
    "labels": [],
    "project": "home",
    "url": None,
    "created_at": None,
    "updated_at": None,
}

CAPABILITIES = {
    "projects": "native",
    "documents": "unsupported",
    "orphan_tasks": "native",
    "filter_by_label": "unsupported",
    "filter_by_status": "unsupported",
    "search_title": "unsupported",
    "search_content": "unsupported",
    "task_dependencies": "forward-only",
    "project_dependencies": "forward-only",
    "max_page_size": 50,
    "reconcile_status": "native",
    "complete_by_issue": "native",
}

# The issue each item is held for: `acme/plan#1` is the home's, `acme/plan#2` the task's.
HOLDINGS = {
    "acme/plan#1": {"id": "home", "kind": "project", "option": "In Progress"},
    "acme/plan#2": {"id": "t1", "option": "Todo"},
}


def malformed(message: str) -> dict[str, object]:
    """The error envelope this peer answers a request it cannot read with (§5)."""
    return {"error": {"kind": "malformed", "message": message}}


def status_written(params: dict[str, object], held: str) -> dict[str, object]:
    """A status write of the one item of its kind this source holds, read back as written."""
    category = params.get("category")
    if not isinstance(category, str):
        return malformed("a status write names its category as a string")
    if params.get("id") != held:
        return {"result": {"status": None}}
    return {"result": {"status": {"category": category, "name": NAMES.get(category, category)}}}


def answer(method: str, params: dict[str, object]) -> dict[str, object]:
    """The `result` or `error` member answering one request (§2)."""
    if method == "initialize":
        return {
            "result": {
                "protocol_version": PROTOCOL_VERSION,
                "kind": KIND,
                "capabilities": CAPABILITIES,
                "writes": "supported",
                "task_updates": True,
                "project_status": True,
            }
        }
    if method == "health":
        return {"result": {"reachable": True, "detail": "one project and one task"}}
    if method == "status_contradictions":
        return {
            "result": {
                "items": [
                    {"project": HOME, "option": "In Progress", "closed_as": "done", "to": "Done"},
                    {"task": TASK, "option": "Todo", "closed_as": "cancelled", "to": "Cancelled"},
                ]
            }
        }
    if method == "issue_holdings":
        issues = params.get("issues")
        if not isinstance(issues, list):
            return malformed("issue_holdings names its issues as a list")
        holdings: list[dict[str, object]] = []
        for issue in issues:
            held = HOLDINGS.get(issue) if isinstance(issue, str) else None
            if held is None:
                holdings.append({"issue": issue, "completion": "absent"})
            else:
                holdings.append({"issue": issue, "completion": "due", "task": held, "to": "Done"})
        return {"result": {"holdings": holdings}}
    if method == "set_task_status":
        return status_written(params, "t1")
    if method == "set_project_status":
        return status_written(params, "home")
    return malformed(
        f"protocol version {PROTOCOL_VERSION} has no method {method} this source serves"
    )


def main() -> None:
    """Answer each request line on standard input with one response line (§1)."""
    for line in sys.stdin:
        if not line.strip():
            continue
        try:
            request = json.loads(line)
        except ValueError:
            print(f"{KIND}: ignoring a line that is not a request", file=sys.stderr)
            continue
        if not isinstance(request, dict) or not isinstance(request.get("id"), str):
            print(f"{KIND}: ignoring a line with no string id", file=sys.stderr)
            continue
        method = request.get("method")
        params = request.get("params")
        if not isinstance(method, str) or not isinstance(params, dict):
            reply = malformed("a request names its method as a string and carries object params")
        else:
            reply = answer(method, params)
        sys.stdout.write(json.dumps({"id": request["id"], **reply}) + "\n")
        sys.stdout.flush()


if __name__ == "__main__":
    main()
