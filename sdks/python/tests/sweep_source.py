"""A writable task source that meters its requests and searches natively, over the stdio protocol.

`sources sanitize` searches a source rather than listing it only when that source spends a
quota, which it knows by the source answering `metering` (§3.4). Every credential-free source
this build ships spends none, so the SDK's sweep journeys speak to this peer, spoken to over
`docs/plugin-protocol.md` exactly as the engine speaks to any plugin. Its tasks live in the JSON
file named by its one argument, so what one invocation writes the next one reads.

It is spawned with a cleared environment (§3.1), so it imports only the standard library — and
none of the SDK's own models.
"""

import json
import sys
from pathlib import Path
from typing import Any, TypedDict, cast

KIND = "sweep-source"
PROTOCOL_VERSION = 2

CAPABILITIES = {
    "projects": "unsupported",
    "documents": "unsupported",
    "comments": "native",
    "orphan_tasks": "native",
    "filter_by_label": "unsupported",
    "filter_by_status": "unsupported",
    "search_title": "native",
    "search_content": "native",
    "task_dependencies": "forward-only",
    "project_dependencies": "forward-only",
    "max_page_size": 100,
}


class Status(TypedDict):
    """A task's status on the wire (§4.4)."""

    category: str
    name: str


class Task(TypedDict):
    """A task as this peer holds it and sends it (§4.4)."""

    id: str
    title: str
    content: str | None
    status: Status
    labels: list[dict[str, str]]
    project: str | None
    url: str | None
    location: dict[str, str] | None
    created_at: str | None
    updated_at: str | None


class Held(TypedDict):
    """The peer's own file: how many requests it has answered, and its tasks."""

    requests: int
    tasks: list[Task]


def _held(store: Path) -> Held:
    """Read the peer's own file: a count of requests beside a list of tasks.

    Anything else ends the peer, saying why.
    """
    held = json.loads(store.read_text(encoding="utf-8"))
    match held:
        case {"requests": int(), "tasks": list(tasks)} if all(
            isinstance(task, dict)
            and isinstance(task.get("id"), str)
            and isinstance(task.get("title"), str)
            and isinstance(task.get("content"), (str, type(None)))
            for task in tasks
        ):
            # Checked member by member above for what the peer reads; the rest of each task is
            # passed through as the engine sent it.
            return cast(Held, held)
        case _:
            sys.exit(f"{KIND}: {store} is not this peer's state")


# A query is a JSON object (§4.5) of which only `text` is read here, checked where it is read.
def _matches(task: Task, query: dict[str, Any]) -> bool:
    text = query.get("text")
    if not isinstance(text, dict):
        return True
    wanted = str(text.get("terms", "")).lower()
    title = task["title"].lower()
    content = (task["content"] or "").lower()
    match text.get("fields"):
        case "title":
            return wanted in title
        case "content":
            return wanted in content
        case _:
            return wanted in title or wanted in content


def _answer(store: Path, method: str, params: dict[str, object]) -> dict[str, object]:
    held = _held(store)
    tasks = held["tasks"]
    if method != "initialize":
        held["requests"] = held["requests"] + 1
        store.write_text(json.dumps(held), encoding="utf-8")
    match method:
        case "initialize":
            return {
                "result": {
                    "protocol_version": PROTOCOL_VERSION,
                    "kind": KIND,
                    "capabilities": CAPABILITIES,
                    "writes": "supported",
                    "meters": True,
                }
            }
        case "health":
            return {"result": {"reachable": True, "detail": f"{len(tasks)} task(s)"}}
        case "metering":
            return {"result": {"metering": {"requests": held["requests"], "budgets": []}}}
        case "get_task":
            found = [task for task in tasks if task["id"] == params.get("id")]
            return {"result": {"task": found[0] if found else None}}
        case "query_tasks":
            query = params.get("query")
            wanted = query if isinstance(query, dict) else {}
            return {
                "result": {
                    "items": [task for task in tasks if _matches(task, wanted)],
                    "next": None,
                }
            }
        case "task_comments":
            return {"result": {"page": {"items": [], "next": None}}}
        case "labels" | "task_dependencies":
            return {"result": {"items": [], "next": None}}
        case "write_task":
            match params.get("write"):
                case {
                    "item": {
                        "id": str(identifier),
                        "title": str(title),
                        "content": str() | None as content,
                        "status": {"category": str(category), "name": str(name)},
                        "labels": list(labels),
                    },
                    **write,
                }:
                    pass
                case _:
                    return {
                        "error": {
                            "kind": "malformed",
                            "message": "write_task carries a write of a whole task",
                        }
                    }
            match write.get("target"):
                case None:
                    target = identifier
                case str(named):
                    target = named
                case _:
                    return {"error": {"kind": "malformed", "message": "a write's target is an id"}}
            written: Task = {
                "id": target,
                "title": title,
                "content": content,
                "status": {"category": category, "name": name},
                "labels": [label for label in labels if isinstance(label, dict)],
                "project": None,
                "url": None,
                "location": None,
                "created_at": None,
                "updated_at": None,
            }
            held["tasks"] = [task for task in tasks if task["id"] != target] + [written]
            store.write_text(json.dumps(held), encoding="utf-8")
            return {"result": {"id": target}}
        case _:
            return {
                "error": {
                    "kind": "malformed",
                    "message": f"protocol version {PROTOCOL_VERSION} has no method {method!r}",
                }
            }


def _main() -> None:
    store = Path(sys.argv[1])
    for line in sys.stdin:
        if not line.strip():
            continue
        try:
            request = json.loads(line)
        except ValueError:
            print(f"{KIND}: ignoring a line that is not JSON", file=sys.stderr)
            continue
        if not isinstance(request, dict) or not isinstance(request.get("id"), str):
            print(f"{KIND}: ignoring a line that is not a request", file=sys.stderr)
            continue
        params = request.get("params")
        answer = _answer(
            store, str(request.get("method")), params if isinstance(params, dict) else {}
        )
        sys.stdout.write(json.dumps({"id": request["id"], **answer}) + "\n")
        sys.stdout.flush()


if __name__ == "__main__":
    _main()
