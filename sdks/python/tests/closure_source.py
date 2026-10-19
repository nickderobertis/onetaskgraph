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
from dataclasses import dataclass
from typing import Literal, NewType, NotRequired, TypedDict

KIND = "closure-source"
PROTOCOL_VERSION = 2

# The source's own id for an item, an issue reference as `owner/name#N`, and the id a request
# is answered under: three domains that are all strings on the wire and never interchangeable.
NativeId = NewType("NativeId", str)
IssueRef = NewType("IssueRef", str)
RequestId = NewType("RequestId", str)

type Support = Literal["native", "unsupported"]
type DependencySupport = Literal["both-directions", "forward-only"]
type ClosedAs = Literal["done", "cancelled"]


class Status(TypedDict):
    """An item's status, as the closure methods send it (§4.4)."""

    category: str
    name: str


class Project(TypedDict):
    """A project on the wire (§4.4), as `get_project` would read it."""

    id: NativeId
    title: str
    content: str | None
    status: Status
    labels: list[dict[str, str]]
    url: str | None
    created_at: str | None
    updated_at: str | None


class Task(Project):
    """A task on the wire (§4.4): a project's shape, filed under one."""

    project: NativeId | None


class Capabilities(TypedDict):
    """What this source declares once, at the handshake (§4.2)."""

    projects: Support
    documents: Support
    orphan_tasks: Support
    filter_by_label: Support
    filter_by_status: Support
    search_title: Support
    search_content: Support
    task_dependencies: DependencySupport
    project_dependencies: DependencySupport
    max_page_size: int
    reconcile_status: Support
    complete_by_issue: Support


class HeldTask(TypedDict):
    """The item one issue is held for (§4.24); `kind` is left out for a task, as 0.8.0 sent it."""

    id: NativeId
    kind: NotRequired[Literal["project"]]
    option: str


class ProjectContradiction(TypedDict):
    """A project item out of line with its closure (§4.23), under its kind's member."""

    project: Project
    option: str
    closed_as: ClosedAs
    to: str


class TaskContradiction(TypedDict):
    """A task item out of line with its closure (§4.23), under its kind's member."""

    task: Task
    option: str
    closed_as: ClosedAs
    to: str


class AbsentHolding(TypedDict):
    """An issue this source holds no item for (§4.24), echoed as it was asked."""

    issue: object
    completion: Literal["absent"]


class DueHolding(TypedDict):
    """An issue whose held item is owed its done option (§4.24)."""

    issue: IssueRef
    completion: Literal["due"]
    task: HeldTask
    to: str


class SourceError(TypedDict):
    """The contract's `SourceError`, in its serialized shape (§5)."""

    kind: Literal["malformed"]
    message: str


class Answered(TypedDict):
    """A response carrying a result (§2)."""

    result: object


class Refused(TypedDict):
    """A response carrying an error (§2)."""

    error: SourceError


@dataclass(frozen=True)
class Request:
    """One request line (§2): an `id` to echo, the method, and its object `params`."""

    id: RequestId
    method: str
    params: dict[str, object]


# What a write of each terminal category reads back as, as a board's options spell them.
NAMES: dict[str, str] = {"done": "Done", "cancelled": "Cancelled"}

HOME: Project = {
    "id": NativeId("home"),
    "title": "A plan's home",
    "content": None,
    "status": {"category": "in-progress", "name": "In Progress"},
    "labels": [],
    "url": None,
    "created_at": None,
    "updated_at": None,
}
TASK: Task = {
    **HOME,
    "id": NativeId("t1"),
    "title": "A task of the plan",
    "status": {"category": "todo", "name": "Todo"},
    "project": NativeId("home"),
}

CAPABILITIES: Capabilities = {
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
HOLDINGS: dict[IssueRef, HeldTask] = {
    IssueRef("acme/plan#1"): {"id": NativeId("home"), "kind": "project", "option": "In Progress"},
    IssueRef("acme/plan#2"): {"id": NativeId("t1"), "option": "Todo"},
}


def _malformed(message: str) -> Refused:
    return {"error": {"kind": "malformed", "message": message}}


def _project_contradiction(item: Project, closed_as: ClosedAs) -> ProjectContradiction:
    """The home out of line with its closure (§4.23)."""
    return {
        "project": item,
        "option": item["status"]["name"],
        "closed_as": closed_as,
        "to": NAMES[closed_as],
    }


def _task_contradiction(item: Task, closed_as: ClosedAs) -> TaskContradiction:
    """The task out of line with its closure (§4.23)."""
    return {
        "task": item,
        "option": item["status"]["name"],
        "closed_as": closed_as,
        "to": NAMES[closed_as],
    }


def _status_written(
    params: dict[str, object], held: NativeId, *, wrapped: bool
) -> Answered | Refused:
    """A status write of the one item of its kind this source holds, read back as written.

    A task's status is under a `status` member (§4.17); a project's is the whole result
    (§4.17a).
    """
    match params:
        case {"id": str(wanted), "category": str(category)}:
            status = (
                {"category": category, "name": NAMES.get(category, category)}
                if wanted == held
                else None
            )
            return {"result": {"status": status} if wrapped else status}
        case _:
            return _malformed("a status write names its item by `id` and its `category`")


def _holding(issue: object) -> AbsentHolding | DueHolding:
    """What this source holds for one issue it was asked about (§4.24)."""
    held = HOLDINGS.get(IssueRef(issue)) if isinstance(issue, str) else None
    if not isinstance(issue, str) or held is None:
        return {"issue": issue, "completion": "absent"}
    return {"issue": IssueRef(issue), "completion": "due", "task": held, "to": "Done"}


def _answer(request: Request) -> Answered | Refused:
    match request.method, request.params:
        case "initialize", _:
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
        case "health", _:
            return {"result": {"reachable": True, "detail": "one project and one task"}}
        case "status_contradictions", _:
            items = [
                _project_contradiction(HOME, "done"),
                _task_contradiction(TASK, "cancelled"),
            ]
            return {"result": {"items": items}}
        case "issue_holdings", {"issues": list(issues)}:
            return {"result": {"holdings": [_holding(issue) for issue in issues]}}
        case "issue_holdings", _:
            return _malformed("issue_holdings names its issues as a list")
        case "set_task_status", params:
            return _status_written(params, TASK["id"], wrapped=True)
        case "set_project_status", params:
            return _status_written(params, HOME["id"], wrapped=False)
        case method, _:
            return _malformed(
                f"protocol version {PROTOCOL_VERSION} has no method {method!r} this source serves"
            )


def request_of(line: str) -> Request | None:
    """One request line read into its shape, or `None` if it is not a request.

    A line without a string `id` has no address a response could echo, so it is set aside on
    stderr rather than answered.
    """
    try:
        decoded = json.loads(line)
    except ValueError:
        return None
    match decoded:
        case {"id": str(identifier), "method": str(method), "params": dict(params)}:
            return Request(RequestId(identifier), method, params)
        case _:
            return None


def _main() -> None:
    for line in sys.stdin:
        if not line.strip():
            continue
        request = request_of(line)
        if request is None:
            print(f"{KIND}: ignoring a line that is not a request", file=sys.stderr)
            continue
        sys.stdout.write(json.dumps({"id": request.id, **_answer(request)}) + "\n")
        sys.stdout.flush()


if __name__ == "__main__":
    _main()
