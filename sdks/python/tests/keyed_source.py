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
    match method:
        case "initialize":
            return {
                "result": {
                    "protocol_version": PROTOCOL_VERSION,
                    "kind": KIND,
                    "capabilities": CAPABILITIES,
                    "writes": "unsupported",
                }
            }
        case "health":
            return {"result": {"reachable": True, "detail": f"{len(TASKS)} task(s)"}}
        case "get_task":
            found = [task for task in TASKS if task["id"] == params.get("id")]
            return {"result": {"task": found[0] if found else None}}
        case "query_tasks":
            return {"result": {"items": TASKS, "next": None}}
        case "labels" | "task_dependencies":
            return {"result": {"items": [], "next": None}}
        case _:
            message = (
                f"protocol version {PROTOCOL_VERSION} has no method {method!r} this source serves"
            )
            return {"error": {"kind": "malformed", "message": message}}


def request_of(line: str) -> tuple[str, str, dict[str, object]] | None:
    """The `id`, `method` and `params` of one request line (§2), or `None` if it has none.

    A line without a string `id` has no address a response could echo, so it is set aside on
    stderr rather than answered.
    """
    try:
        request = json.loads(line)
    except ValueError:
        return None
    match request:
        case {"id": str(identifier), "method": str(method), "params": dict(params)}:
            return identifier, method, params
        case _:
            return None


def main() -> None:
    """Serve one connection until the engine closes its input."""
    for line in sys.stdin:
        if not line.strip():
            continue
        request = request_of(line)
        if request is None:
            print(f"{KIND}: ignoring a line that is not a request", file=sys.stderr)
            continue
        identifier, method, params = request
        reply = {"id": identifier, **answer(method, params)}
        sys.stdout.write(json.dumps(reply) + "\n")
        sys.stdout.flush()


if __name__ == "__main__":
    main()
