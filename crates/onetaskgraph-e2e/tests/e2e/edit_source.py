"""A writable task source another writer edits at fixed points, over the stdio protocol.

The sanitise-on-refusal and sweep journeys need a person's edit to land between the store's
read of an item and its write — and between its sanitising write and the write that set it off
— at a point a test names, every run. Nothing outside a process can land an edit there, so this
peer applies it itself: its fixture lists, per task, the read counts at which another writer's
edit lands — the edit applied to the stored record, its version moved, and only then the read
answered — and every write it takes is the store's own.

Its state — tasks, comments, read counts and every request — is the JSON file its one argument
names, so what one invocation writes the next reads. Its search answers from an index taken
when the fixture was laid and never refreshed, as a hosted index lags what its records hold, so
a sweep run again still finds what it already rewrote. It declares `meters`, so a sweep
searches it rather than reading it whole.

It is spawned with a cleared environment (§3.1), so it imports only the standard library.
"""

import json
import sys
from pathlib import Path
from typing import Any, TypedDict

KIND = "edit-source"
PROTOCOL_VERSION = 2

CAPABILITIES = {
    "projects": "unsupported",
    "documents": "unsupported",
    "comments": "native",
    "priority": "native",
    "orphan_tasks": "native",
    "filter_by_label": "unsupported",
    "filter_by_status": "unsupported",
    "search_title": "native",
    "search_content": "native",
    "task_dependencies": "forward-only",
    "project_dependencies": "forward-only",
    "max_page_size": 100,
}


class Task(TypedDict):
    """A task as this peer holds it and sends it (§4.4): every member the store reads, its
    `version` the token another writer's edit moves."""

    id: str
    title: str
    content: str | None
    status: dict[str, str]
    priority: str
    labels: list[dict[str, str | None]]
    metadata: dict[str, Any]
    version: str


class Comment(TypedDict):
    """A comment as this peer holds it and sends it (§4.16)."""

    id: str
    body: str


class State(TypedDict):
    """Everything the peer keeps between invocations: its tasks and their comments, how many
    times each task was read, the reads at which another writer's edit lands and what it is,
    the frozen search index, and every method it was asked."""

    tasks: list[Task]
    comments: dict[str, list[Comment]]
    reads: dict[str, int]
    moves: dict[str, list[int]]
    edits: dict[str, dict[str, str]]
    index: dict[str, str]
    log: list[str]


def _task(state: State, identifier: object) -> Task | None:
    return next((task for task in state["tasks"] if task["id"] == identifier), None)


def _move(state: State, task: Task) -> None:
    """Another writer's edit, as the fixture names it for this task: a label added, or the
    content's last paragraph replaced — and the version moved."""
    edit = state["edits"].get(task["id"], {})
    if "label" in edit:
        task["labels"] = [*task["labels"], {"id": edit["label"], "name": edit["label"], "color": None}]
    if "paragraph" in edit:
        task["content"] = (task.get("content") or "").rstrip("\n") + "\n\n" + edit["paragraph"]
    task["version"] = str(int(task["version"]) + 1)


def _answer(state: State, method: str, params: dict[str, Any]) -> dict[str, Any]:
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
            return {"result": {"reachable": True, "detail": f"{len(state['tasks'])} task(s)"}}
        case "metering":
            return {"result": {"metering": {"requests": len(state["log"]), "budgets": []}}}
        case "get_task":
            task = _task(state, params.get("id"))
            if task is not None:
                reads = state["reads"].get(task["id"], 0) + 1
                state["reads"][task["id"]] = reads
                if reads in state["moves"].get(task["id"], []):
                    _move(state, task)
            return {"result": {"task": task}}
        case "query_tasks":
            text = params.get("query", {}).get("text")
            wanted = str(text.get("terms", "")).lower() if isinstance(text, dict) else None
            items = [
                task
                for task in state["tasks"]
                if wanted is None or wanted in state["index"].get(task["id"], "").lower()
            ]
            return {"result": {"items": items, "next": None}}
        case "task_comments":
            held = state["comments"].get(params.get("task"))
            if held is None:
                return {"result": {"page": None}}
            page = params.get("page", {})
            start = int(page.get("cursor") or 0)
            end = start + int(page.get("limit") or 100)
            following = str(end) if end < len(held) else None
            return {"result": {"page": {"items": held[start:end], "next": following}}}
        case "edit_comment":
            for comment in state["comments"].get(params.get("task"), []):
                if comment["id"] == params.get("comment"):
                    comment["body"] = params["body"]
                    task = _task(state, params.get("task"))
                    if task is not None:
                        task["version"] = str(int(task["version"]) + 1)
                    return {"result": {"comment": comment}}
            return {"result": {"comment": None}}
        case "labels" | "task_dependencies":
            return {"result": {"items": [], "next": None}}
        case "write_task":
            item = params["write"]["item"]
            target = params["write"].get("target") or item["id"]
            held = _task(state, target)
            item["id"] = target
            item["version"] = str(int(held["version"]) + 1) if held else "1"
            state["tasks"] = [task for task in state["tasks"] if task["id"] != target] + [item]
            return {"result": {"id": target}}
        case "set_task_priority":
            task = _task(state, params.get("id"))
            if task is None:
                return {"result": {"priority": None}}
            task["priority"] = params["priority"]
            task["version"] = str(int(task["version"]) + 1)
            return {"result": {"priority": task["priority"]}}
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
        request = json.loads(line)
        state = json.loads(store.read_text(encoding="utf-8"))
        params = request.get("params") if isinstance(request.get("params"), dict) else {}
        if request.get("method") != "initialize":
            state["log"].append(request.get("method"))
        answer = _answer(state, str(request.get("method")), params)
        store.write_text(json.dumps(state), encoding="utf-8")
        sys.stdout.write(json.dumps({"id": request["id"], **answer}) + "\n")
        sys.stdout.flush()


if __name__ == "__main__":
    _main()
