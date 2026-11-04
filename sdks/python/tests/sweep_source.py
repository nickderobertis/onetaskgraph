"""A writable task source that meters its requests and searches natively, over the stdio protocol.

`sources sanitize` searches a source rather than listing it only when that source spends a
quota, which it knows by the source answering `metering` (§3.4). Every credential-free source
this build ships spends none, so the SDK's sweep journeys speak to this peer, spoken to over
`docs/plugin-protocol.md` exactly as the engine speaks to any plugin. Its tasks live in the JSON
file named by its one argument, so what one invocation writes the next one reads.

"""

import json
import sys
from pathlib import Path
from typing import Any, TypedDict, cast

from onetaskgraph_sdk import StatusCategory

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


type TaskId = str
type ProjectId = str


class Label(TypedDict):
    """A named label with the protocol's nullable colour."""

    id: str
    name: str
    color: str | None


class UrlLocation(TypedDict):
    """A hosted record's link (§4.4)."""

    url: str


class PathLocation(TypedDict):
    """A local record's absolute path (§4.4)."""

    path: str


type Location = UrlLocation | PathLocation


class Status(TypedDict):
    """A task's status on the wire (§4.4)."""

    category: StatusCategory
    name: str


class Task(TypedDict):
    """A task as this peer holds it and sends it (§4.4)."""

    id: TaskId
    title: str
    content: str | None
    status: Status
    labels: list[Label]
    project: ProjectId | None
    url: str | None
    location: Location | None
    created_at: str | None
    updated_at: str | None


class Held(TypedDict):
    """The peer's own file: how many requests it has answered, and its tasks."""

    requests: int
    tasks: list[Task]


def _valid_task(task: object) -> bool:
    """Validate every member the peer reads or returns before trusting its persisted state."""
    if not isinstance(task, dict):
        return False
    nullable = ("content", "project", "url", "created_at", "updated_at")
    if not (
        isinstance(task.get("id"), str)
        and isinstance(task.get("title"), str)
        and all(key in task and isinstance(task[key], (str, type(None))) for key in nullable)
    ):
        return False
    status = task.get("status")
    if not isinstance(status, dict) or not isinstance(status.get("name"), str):
        return False
    try:
        category = StatusCategory(status.get("category"))
    except (ValueError, TypeError):
        return False
    labels = task.get("labels")
    if not isinstance(labels, list) or not all(
        isinstance(label, dict)
        and isinstance(label.get("id"), str)
        and isinstance(label.get("name"), str)
        and isinstance(label.get("color"), (str, type(None)))
        for label in labels
    ):
        return False
    if "location" not in task:
        return False
    location = task["location"]
    if location is not None and not (
        isinstance(location, dict)
        and len(location) == 1
        and (
            (set(location) == {"url"} and isinstance(location["url"], str))
            or (
                set(location) == {"path"}
                and isinstance(location["path"], str)
                and Path(location["path"]).is_absolute()
            )
        )
    ):
        return False
    status["category"] = category
    return True


def _held(store: Path) -> Held:
    """Read the peer's own file: a count of requests beside a list of tasks.

    Anything else ends the peer, saying why.
    """
    held = json.loads(store.read_text(encoding="utf-8"))
    match held:
        case {"requests": int(requests), "tasks": list(tasks)} if (
            type(requests) is int and requests >= 0 and all(_valid_task(task) for task in tasks)
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
    wanted = text["terms"].lower()
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
            if not isinstance(params.get("id"), str):
                return {"error": {"kind": "malformed", "message": "the task id is a string"}}
            found = [task for task in tasks if task["id"] == params.get("id")]
            return {"result": {"task": found[0] if found else None}}
        case "query_tasks":
            query = params.get("query")
            if not isinstance(query, dict):
                return {"error": {"kind": "malformed", "message": "a query is an object"}}
            wanted = query
            text = wanted.get("text")
            if text is not None and (
                not isinstance(text, dict)
                or not isinstance(text.get("terms"), str)
                or text.get("fields") not in ("title", "content", "title-or-content")
            ):
                return {"error": {"kind": "malformed", "message": "search terms are a string"}}
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
            try:
                status_category = StatusCategory(category)
            except ValueError:
                return {
                    "error": {
                        "kind": "malformed",
                        "message": "status category is outside the protocol vocabulary",
                    }
                }
            match write.get("target"):
                case None:
                    target = identifier
                case str(named):
                    target = named
                case _:
                    return {"error": {"kind": "malformed", "message": "a write's target is an id"}}
            if not all(
                isinstance(label, dict)
                and isinstance(label.get("id"), str)
                and isinstance(label.get("name"), str)
                and isinstance(label.get("color"), (str, type(None)))
                for label in labels
            ):
                return {
                    "error": {"kind": "malformed", "message": "labels carry string ids and names"}
                }
            written: Task = {
                "id": target,
                "title": title,
                "content": content,
                "status": {"category": status_category, "name": name},
                "labels": labels,
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
        if (
            not isinstance(request, dict)
            or not isinstance(request.get("id"), str)
            or not isinstance(request.get("method"), str)
        ):
            print(f"{KIND}: ignoring a line that is not a request", file=sys.stderr)
            continue
        params = request.get("params")
        answer = _answer(store, request["method"], params if isinstance(params, dict) else {})
        sys.stdout.write(json.dumps({"id": request["id"], **answer}) + "\n")
        sys.stdout.flush()


if __name__ == "__main__":
    _main()
