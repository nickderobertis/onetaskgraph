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
from typing import Any, TypedDict, cast

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
    # Caller-defined metadata holds any JSON value under each key (§4.4): `Any` is JSON here.
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
    # Each task's forward edges, as a write sent them (§4.7): decoded JSON, kept as sent.
    edges: dict[str, list[Any]]
    log: list[str]


def _task(state: State, identifier: object) -> Task | None:
    return next((task for task in state["tasks"] if task["id"] == identifier), None)


# A task on the wire is the JSON object §4.4 describes, its members of mixed JSON types: `Any`
# is a decoded JSON value, as every response this peer writes is.
def _shown(state: State, task: Task) -> dict[str, Any]:
    """`task` on the wire, as a read answers it (§4.4): without its version when the fixture
    says this source keeps none (`"unversioned": true` in its state)."""
    shown: dict[str, Any] = dict(task)
    if state.get("unversioned"):
        shown["version"] = None
    return shown


def _move(state: State, task: Task) -> None:
    """Another writer's edit, as the fixture names it for this task — a label added, the
    content replaced, a paragraph appended to it, or a comment added — and the version moved."""
    edit = state["edits"].get(task["id"], {})
    if "label" in edit:
        task["labels"] = [*task["labels"], {"id": edit["label"], "name": edit["label"], "color": None}]
    if "content" in edit:
        task["content"] = edit["content"]
    if "comment_target" in edit:
        for comment in state["comments"].get(task["id"], []):
            if comment["id"] == edit["comment_target"]:
                comment["body"] = edit.get("comment_body", comment["body"])
    if "comment" in edit:
        state["comments"].setdefault(task["id"], []).append(
            {"id": edit.get("comment_id", f"by-person-{task['version']}"), "body": edit["comment"]}
        )
    if "paragraph" in edit:
        task["content"] = (task.get("content") or "").rstrip("\n") + "\n\n" + edit["paragraph"]
    # A comment edited on its own moves no version of the task it is on.
    if "silent" not in edit:
        task["version"] = str(int(task["version"]) + 1)


def _malformed(message: str) -> dict[str, dict[str, str]]:
    return {"error": {"kind": "malformed", "message": message}}


# A request's params and a response are JSON objects whose members each method names (§2), so
# `Any` stands for a decoded JSON value; every member a method reads is checked where it is.
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
            if state.get("metering_fails"):
                return {"error": {"kind": "unavailable", "message": "the count could not be read"}}
            return {"result": {"metering": {"requests": len(state["log"]), "budgets": []}}}
        case "get_task":
            gone = state.get("gone", [])
            task = None if params.get("id") in gone else _task(state, params.get("id"))
            if task is not None:
                reads = state["reads"].get(task["id"], 0) + 1
                state["reads"][task["id"]] = reads
                if reads in state["moves"].get(task["id"], []):
                    _move(state, task)
            return {"result": {"task": None if task is None else _shown(state, task)}}
        case "query_tasks" if state.get("query_fails"):
            return {"error": {"kind": "unavailable", "message": "the tasks could not be listed"}}
        case "query_tasks":
            match params.get("query"):
                case {"text": {"terms": str(terms)}}:
                    wanted = terms.lower()
                case {"text": None} | dict():
                    wanted = None
                case _:
                    return _malformed("query_tasks carries its query as an object")
            items = [
                _shown(state, task)
                for task in state["tasks"]
                if wanted is None or wanted in state["index"].get(task["id"], "").lower()
            ]
            return {"result": {"items": items, "next": None}}
        case "task_comments" if state.get("comments_fail") or (
            state.get("later_comments_fail") and params.get("page", {}).get("cursor") is not None
        ):
            return {"error": {"kind": "unavailable", "message": "the comments could not be read"}}
        case "task_comments" | "edit_comment" if not isinstance(params.get("task"), str):
            return _malformed(f"{method} names its task by a string id")
        case "task_comments":
            held = state["comments"].get(params.get("task"))
            if held is None:
                return {"result": {"page": None}}
            match params.get("page"):
                case {"cursor": None, "limit": int(limit)}:
                    start = 0
                case {"cursor": str(cursor), "limit": int(limit)} if cursor.isdigit():
                    start = int(cursor)
                case _:
                    return _malformed("task_comments pages by this peer's own cursor and a limit")
            end = start + limit
            following = str(end) if end < len(held) else None
            return {"result": {"page": {"items": held[start:end], "next": following}}}
        case "edit_comment":
            if not isinstance(params.get("body"), str):
                return _malformed("edit_comment carries the comment's new body as a string")
            for comment in state["comments"].get(params.get("task"), []):
                if comment["id"] == params.get("comment"):
                    comment["body"] = params["body"]
                    task = _task(state, params.get("task"))
                    if task is not None:
                        task["version"] = str(int(task["version"]) + 1)
                    return {"result": {"comment": comment}}
            return {"result": {"comment": None}}
        case "task_dependencies":
            forward = params.get("direction") == "depends-on"
            held = state["edges"].get(str(params.get("id")), []) if forward else []
            return {"result": {"items": held, "next": None}}
        case "labels":
            return {"result": {"items": [], "next": None}}
        case "write_task" if state.get("refuse_writes"):
            return {"error": {"kind": "unavailable", "message": "the write could not be made"}}
        case "write_task":
            match params.get("write"):
                case {"item": {"id": str(), "title": str(), "status": dict()} as item, **write}:
                    target = write.get("target") or item["id"]
                    if isinstance(write.get("depends_on"), list):
                        state["edges"][target] = write["depends_on"]
                case _:
                    return _malformed("write_task carries a write of a task with an id and title")
            held = _task(state, target)
            answered = state.get("answer_ids", {}).get(target, target)
            if state.get("drop_comments_on_write"):
                state["comments"].pop(target, None)
            item["id"] = target
            item["version"] = str(int(held["version"]) + 1) if held else "1"
            state["tasks"] = [task for task in state["tasks"] if task["id"] != target] + [item]
            return {"result": {"id": answered}}
        case "set_task_priority":
            task = _task(state, params.get("id"))
            if task is None:
                return {"result": {"priority": None}}
            if not isinstance(params.get("priority"), str):
                return _malformed("set_task_priority carries a priority word")
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


def _is_task(task: object) -> bool:
    """Whether `task` holds every member this peer reads or writes, of its type."""
    match task:
        case {
            "id": str(),
            "title": str(),
            "content": str() | None,
            "status": dict(),
            "priority": str(),
            "labels": list(labels),
            "metadata": dict(),
            "version": str(version),
        }:
            return version.isdigit() and all(isinstance(label, dict) for label in labels)
        case _:
            return False


def _is_edit(edit: object) -> bool:
    """Whether `edit` is one another writer's edit: each member it names a string."""
    return isinstance(edit, dict) and all(
        key
        in {"label", "content", "paragraph", "comment", "comment_id", "comment_body", "comment_target", "silent"}
        and isinstance(value, str)
        for key, value in edit.items()
    )


def _well_formed(held: object) -> bool:
    """Whether `held` is this peer's own state: every member `State` names, of its type, each
    task, comment and edit whole."""
    match held:
        case {
            "tasks": list(tasks),
            "comments": dict(comments),
            "reads": dict(reads),
            "moves": dict(moves),
            "edits": dict(edits),
            "index": dict(index),
            "edges": dict(edges),
            "log": list(),
        }:
            return (
                all(_is_task(task) for task in tasks)
                and all(isinstance(count, int) for count in reads.values())
                and all(
                    isinstance(at, list) and all(isinstance(read, int) for read in at)
                    for at in moves.values()
                )
                and all(_is_edit(edit) for edit in edits.values())
                and all(isinstance(text, str) for text in index.values())
                and all(isinstance(held_edges, list) for held_edges in edges.values())
                and all(
                    isinstance(held_comments, list)
                    and all(
                        isinstance(comment, dict)
                        and isinstance(comment.get("id"), str)
                        and isinstance(comment.get("body"), str)
                        for comment in held_comments
                    )
                    for held_comments in comments.values()
                )
            )
        case _:
            return False


def _state(store: Path) -> State:
    """The peer's own file, refused — ending the peer — unless it is well formed."""
    held = json.loads(store.read_text(encoding="utf-8"))
    if not _well_formed(held):
        sys.exit(f"{KIND}: {store} is not this peer's state")
    # Checked member by member above for everything this peer indexes or counts.
    return cast(State, held)


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
        match request:
            case {"id": str(identifier), "method": str(method), **rest}:
                params = rest.get("params")
            case _:
                print(f"{KIND}: ignoring a line that is not a request", file=sys.stderr)
                continue
        state = _state(store)
        if method != "initialize":
            state["log"].append(method)
        answer = _answer(state, method, params if isinstance(params, dict) else {})
        store.write_text(json.dumps(state), encoding="utf-8")
        sys.stdout.write(json.dumps({"id": identifier, **answer}) + "\n")
        sys.stdout.flush()


if __name__ == "__main__":
    _main()
