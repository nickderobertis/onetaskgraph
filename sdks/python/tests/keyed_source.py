"""A read-only task source whose backend shows people a short handle, over the stdio protocol.

The SDK tests need a source that reports a task's `key`, and none of the credential-free
sources this build ships can: `local-md` and `in-memory` have no handle of their own by
contract, and the two backends that do — Linear and GitHub — need a credential. So this
peer, spoken to over `docs/plugin-protocol.md` exactly as the engine speaks to any plugin,
holds two tasks: one whose backend gave it a handle, and one sent the way a plugin written
before `key` existed sends a task, with no `key` member at all (§4.13a).

It is spawned with a cleared environment (§3.1), so it imports only the standard library.
"""

import json
import sys

KIND = "keyed-source"
PROTOCOL_VERSION = 2

TASKS = [
    {
        "id": "iss_8f2c",
        "key": "ENG-7",
        "title": "Engine handle",
        "content": None,
        "status": {"category": "todo", "name": "Todo"},
        "labels": [],
        "project": None,
        "url": None,
        "location": None,
        "created_at": None,
        "updated_at": None,
    },
    {
        "id": "iss_91d0",
        "title": "Engine without a handle",
        "content": None,
        "status": {"category": "todo", "name": "Todo"},
        "labels": [],
        "project": None,
        "url": None,
        "location": None,
        "created_at": None,
        "updated_at": None,
    },
]

# Everything a query could narrow by is declared unsupported, so this peer answers every
# `query_tasks` with the whole set and the engine applies the predicates itself (rule 2).
CAPABILITIES = {
    "projects": "unsupported",
    "documents": "unsupported",
    "orphan_tasks": "native",
    "filter_by_label": "unsupported",
    "filter_by_status": "unsupported",
    "search_title": "unsupported",
    "search_content": "unsupported",
    "task_dependencies": "forward-only",
    "project_dependencies": "forward-only",
    "max_page_size": 50,
}


def answer(method: str, params: dict[str, object]) -> dict[str, object]:
    """The result for one method, or a `SourceError` for one this source does not serve."""
    if method == "initialize":
        return {
            "result": {
                "protocol_version": PROTOCOL_VERSION,
                "kind": KIND,
                "capabilities": CAPABILITIES,
                "writes": "unsupported",
            }
        }
    if method == "health":
        return {"result": {"reachable": True, "detail": f"{len(TASKS)} task(s)"}}
    if method == "get_task":
        found = [task for task in TASKS if task["id"] == params.get("id")]
        return {"result": {"task": found[0] if found else None}}
    if method in ("query_tasks", "labels", "task_dependencies"):
        return {"result": {"items": TASKS if method == "query_tasks" else [], "next": None}}
    message = f"protocol version {PROTOCOL_VERSION} has no method {method!r} this source serves"
    return {"error": {"kind": "malformed", "message": message}}


def main() -> None:
    """Serve one connection until the engine closes its input."""
    for line in sys.stdin:
        if not line.strip():
            continue
        request = json.loads(line)
        reply = {"id": request["id"], **answer(request["method"], request["params"])}
        sys.stdout.write(json.dumps(reply) + "\n")
        sys.stdout.flush()


if __name__ == "__main__":
    main()
