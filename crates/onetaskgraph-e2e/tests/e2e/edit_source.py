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
import re
from datetime import datetime
import sys
from pathlib import Path
from typing import Any, Generic, Literal, TypeAlias, TypeVar, TypedDict, NotRequired, cast, get_args

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


TaskId: TypeAlias = str
CommentId: TypeAlias = str
VersionToken: TypeAlias = str
StatusCategory: TypeAlias = Literal["draft", "backlog", "todo", "queued", "in-progress", "done", "cancelled", "unknown"]
Priority: TypeAlias = Literal["none", "urgent", "high", "medium", "low"]
ItemKind: TypeAlias = Literal["task", "project"]
DependencyKind: TypeAlias = Literal["blocks", "related"]
TextFields: TypeAlias = Literal["title", "content", "title-or-content"]
Direction: TypeAlias = Literal["depends-on", "depended-on-by"]
Classification: TypeAlias = Literal["public", "private"]



class Status(TypedDict):
    """The protocol status category beside the backend's own name."""

    category: StatusCategory
    name: str


class Label(TypedDict):
    """A named label with the protocol's nullable colour."""

    id: str
    name: str
    color: str | None


class Edit(TypedDict, total=False):
    """The fixture's independent edits, applied together at a chosen read."""

    label: str
    content: str
    paragraph: str
    comment: str
    comment_id: CommentId
    comment_body: str
    comment_target: CommentId
    silent: str


T = TypeVar("T")


class PageRequest(TypedDict):
    """The cursor and positive limit validated before paging (§4.5)."""

    cursor: str | None
    limit: int


class Page(TypedDict, Generic[T]):
    """One typed page, with its continuation cursor (§4.5)."""

    items: list[T]
    next: str | None


class Task(TypedDict):
    """A task as this peer holds it and sends it (§4.4): every member the store reads, its
    `version` the token another writer's edit moves."""

    id: TaskId
    title: str
    content: str | None
    status: Status
    priority: str | None
    labels: list[Label]
    # Caller-defined metadata holds any JSON value under each key (§4.4): `Any` is JSON here.
    metadata: dict[str, Any]
    version: VersionToken
    key: NotRequired[str | None]
    project: str | None
    url: str | None
    location: dict[str, str] | None
    created_at: str | None
    updated_at: str | None
    repositories: NotRequired[list[str]]
    delivers: NotRequired[list[str]]
    delivered_by: NotRequired[list[str]]
    classification: NotRequired[Classification]


class Comment(TypedDict):
    """A comment as this peer holds it and sends it (§4.16)."""

    id: CommentId
    body: str


class State(TypedDict):
    """Everything the peer keeps between invocations: its tasks and their comments, how many
    times each task was read, the reads at which another writer's edit lands and what it is,
    the frozen search index, and every method it was asked."""

    tasks: list[Task]
    comments: dict[str, list[Comment]]
    reads: dict[str, int]
    moves: dict[str, list[int]]
    edits: dict[TaskId, Edit]
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


def _is_ordinal(value: object) -> bool:
    """Whether the peer's cursor or version can be consumed by its integer arithmetic."""
    if not isinstance(value, str) or not value.isdigit():
        return False
    try:
        int(value)
    except ValueError:
        return False
    return True


def _is_page(page: object) -> bool:
    match page:
        case {"cursor": cursor, "limit": limit}:
            return type(limit) is int and limit > 0 and (cursor is None or _is_ordinal(cursor))
        case _:
            return False


def _page(state: State, items: list[T], page: PageRequest) -> Page[T]:
    start = 0 if page["cursor"] is None else int(page["cursor"])
    end = start + min(page["limit"], state.get("page_size", page["limit"]))
    return {"items": items[start:end], "next": str(end) if end < len(items) else None}


def _is_priority(value: object) -> bool:
    return isinstance(value, str) and value in get_args(Priority)


def _is_label(value: object) -> bool:
    return (isinstance(value, dict) and isinstance(value.get("id"), str)
            and isinstance(value.get("name"), str) and isinstance(value.get("color"), (str, type(None))))


def _is_endpoint(value: object) -> bool:
    if isinstance(value, str):
        return bool(value)
    return (isinstance(value, dict) and isinstance(value.get("id"), str) and bool(value["id"])
            and value.get("kind") in get_args(ItemKind)
            and ("designator" not in value or isinstance(value["designator"], str)))


def _is_edge(value: object) -> bool:
    return (isinstance(value, dict) and _is_endpoint(value.get("from"))
            and _is_endpoint(value.get("to")) and value.get("kind") in get_args(DependencyKind))


# A request's params and a response are JSON objects whose members each method names (§2), so
# `Any` stands for a decoded JSON value; every member a method reads is checked where it is.
def _answer(state: State, method: str, params: dict[str, Any]) -> dict[str, Any]:
    if method in ("get_task", "set_task_priority") and not isinstance(params.get("id"), str):
        return _malformed("the task id is a string")
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
            shown = None if task is None else _shown(state, task)
            if shown is not None and state.get("wrong_read_id") and reads == state.get("wrong_read_at", 2):
                shown["id"] = state["wrong_read_id"]
            return {"result": {"task": shown}}
        case "query_tasks" if state.get("query_fails"):
            return {"error": {"kind": "unavailable", "message": "the tasks could not be listed"}}
        case "query_tasks":
            if not _is_page(params.get("page")):
                return _malformed("query_tasks carries a page with a positive limit")
            match params.get("query"):
                case {"text": {"terms": str(terms), "fields": str(fields)}} if fields in get_args(TextFields):
                    wanted = terms.lower()
                    index = state["index"] if fields == "title-or-content" else state[f"{fields}_index"]
                case dict(query) if query.get("text") is None:
                    wanted = None
                    index = state["index"]
                case _:
                    return _malformed("query_tasks carries its query as an object")
            items = [
                _shown(state, task)
                for task in sorted(state["tasks"], key=lambda task: task["id"])
                if wanted is None or wanted in index.get(task["id"], "").lower()
            ]
            state.setdefault("query_cursors", []).append(params["page"]["cursor"])
            return {"result": _page(state, items, params["page"])}
        case "task_comments" if not _is_page(params.get("page")):
            return _malformed("task_comments pages by this peer's own cursor and a positive limit")
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
            if not isinstance(params.get("comment"), str):
                return _malformed("edit_comment names its comment by a string id")
            if not isinstance(params.get("body"), str):
                return _malformed("edit_comment carries the comment's new body as a string")
            for comment in state["comments"].get(params.get("task"), []):
                if comment["id"] == params.get("comment"):
                    comment["body"] = params["body"]
                    task = _task(state, params.get("task"))
                    if task is not None:
                        task["version"] = str(int(task["version"]) + 1)
                    shown = dict(comment)
                    if state.get("wrong_comment_id"):
                        shown["id"] = state["wrong_comment_id"]
                    return {"result": {"comment": shown}}
            return {"result": {"comment": None}}
        case "task_dependencies":
            if (not isinstance(params.get("id"), str)
                    or params.get("direction") not in get_args(Direction)
                    or not _is_page(params.get("page"))):
                return _malformed("task_dependencies carries an id, direction and page")
            forward = params.get("direction") == "depends-on"
            held = state["edges"].get(params["id"], []) if forward else []
            state.setdefault("dependency_cursors", []).append(params["page"]["cursor"])
            return {"result": _page(state, held, params["page"])}
        case "labels":
            return {"result": {"items": [], "next": None}}
        case "write_task" if state.get("refuse_writes"):
            return {"error": {"kind": "unavailable", "message": "the write could not be made"}}
        case "write_task":
            match params.get("write"):
                case {"item": {"id": str(), "title": str(), "status": dict()} as item, **write}:
                    if not _is_task({**item, "version": "1"}):
                        return _malformed("write_task carries a whole task")
                    if write.get("target") is not None and not isinstance(write["target"], str):
                        return _malformed("write_task target is a string or null")
                    target = write.get("target") or item["id"]
                    edges = write.get("depends_on", [])
                    if not isinstance(edges, list) or not all(_is_edge(edge) for edge in edges):
                        return _malformed("write_task carries whole dependency edges")
                    state["edges"][target] = edges
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
            if not _is_priority(params.get("priority")):
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


def _task_fields_valid(task: dict[str, Any]) -> bool:
    """Validate every retained optional work field against the protocol's scalar shapes."""
    for key in ("key", "project", "url"):
        if task.get(key) is not None and not isinstance(task[key], str):
            return False
    if task.get("classification", "public") not in get_args(Classification):
        return False
    for key in ("repositories", "delivers", "delivered_by"):
        values = task.get(key, [])
        if not isinstance(values, list) or not all(isinstance(value, str) and value for value in values):
            return False
        if len(set(values)) != len(values):
            return False
        for value in values:
            if key == "repositories":
                if ("://" in value or value.endswith(".git") or any(c.isspace() for c in value)
                        or len(value.split("/")) < 3 or any(part in ("", ".", "..") for part in value.split("/"))):
                    return False
            elif ":" in value:
                source, native = value.split(":", 1)
                if not re.fullmatch(r"[a-z0-9][a-z0-9-]*", source) or not native:
                    return False
    location = task.get("location")
    if location is not None and (not isinstance(location, dict) or len(location) != 1
            or not any(key in location and isinstance(location[key], str) for key in ("url", "path"))):
        return False
    for key in ("created_at", "updated_at"):
        value = task.get(key)
        if value is None:
            continue
        if not isinstance(value, str) or not re.fullmatch(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})", value):
            return False
        try:
            datetime.fromisoformat(value.replace("Z", "+00:00"))
        except ValueError:
            return False
    return True


def _is_task(task: object) -> bool:
    """Whether `task` holds every member this peer reads or writes, of its type."""
    match task:
        case {
            "id": str(),
            "title": str(),
            "content": str() | None,
            "status": {"category": str(category), "name": str()},
            "priority": priority,
            "labels": list(labels),
            "metadata": dict(),
            "version": str(version),
        }:
            return (_is_ordinal(version) and _is_priority(priority)
                    and category in get_args(StatusCategory)
                    and all(_is_label(label) for label in labels)
                    and _task_fields_valid(task))
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
            "log": list(log),
        }:
            return (
                all(_is_task(task) for task in tasks)
                and all(type(count) is int and count >= 0 for count in reads.values())
                and all(
                    isinstance(at, list) and all(type(read) is int and read > 0 for read in at)
                    for at in moves.values()
                )
                and all(_is_edit(edit) for edit in edits.values())
                and all(isinstance(text, str) for text in index.values())
                and all(isinstance(held_edges, list) and all(_is_edge(edge) for edge in held_edges) for held_edges in edges.values())
                and all(isinstance(entry, str) for entry in log)
                and _controls_well_formed(held)
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


def _controls_well_formed(held: dict[str, object]) -> bool:
    for key in ("unversioned", "metering_fails", "query_fails", "comments_fail", "later_comments_fail", "refuse_writes", "drop_comments_on_write"):
        if key in held and not isinstance(held[key], bool):
            return False
    if any(key in held and not isinstance(held[key], str) for key in ("wrong_read_id", "wrong_comment_id")):
        return False
    if "wrong_read_at" in held and (type(held["wrong_read_at"]) is not int or held["wrong_read_at"] < 1):
        return False
    gone = held.get("gone", [])
    answers = held.get("answer_ids", {})
    if not isinstance(gone, list) or not all(isinstance(value, str) for value in gone):
        return False
    if not isinstance(answers, dict) or not all(isinstance(value, str) for value in answers.values()):
        return False
    if "page_size" in held and (type(held["page_size"]) is not int or held["page_size"] <= 0):
        return False
    for key in ("title_index", "content_index"):
        index = held.get(key, {})
        if not isinstance(index, dict) or not all(isinstance(value, str) for value in index.values()):
            return False
    return all(isinstance(held.get(key, []), list) and all(value is None or _is_ordinal(value) for value in held.get(key, []))
               for key in ("query_cursors", "dependency_cursors"))


def _state(store: Path) -> State:
    """The peer's own file, refused — ending the peer — unless it is well formed."""
    held = json.loads(store.read_text(encoding="utf-8"))
    if not _well_formed(held):
        sys.exit(f"{KIND}: {store} is not this peer's state")
    held.setdefault("title_index", {task["id"]: task["title"] for task in held["tasks"]})
    held.setdefault("content_index", {task["id"]: task["content"] or "" for task in held["tasks"]})
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
        answer = (_answer(state, method, params or {}) if params is None or isinstance(params, dict)
                  else _malformed("request params is an object"))
        store.write_text(json.dumps(state), encoding="utf-8")
        sys.stdout.write(json.dumps({"id": identifier, **answer}) + "\n")
        sys.stdout.flush()


if __name__ == "__main__":
    _main()
