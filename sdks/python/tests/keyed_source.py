"""A read-only task source whose backend shows people a short handle, over the stdio protocol.

The SDK tests need a source that reports a task's `key`, and none of the credential-free
sources this build ships can: `local-md` and `in-memory` have no handle of their own by
contract, and the two backends that do — Linear and GitHub — need a credential. So this
peer, spoken to over `docs/plugin-protocol.md` exactly as the engine speaks to any plugin,
holds two tasks: one whose backend gave it a handle, and one sent the way a plugin written
before `key` existed sends a task, with no `key` member at all (§4.13a).

It is spawned with a cleared environment (§3.1), so it imports only the standard library —
and none of the SDK's own models, so what the SDK decodes is not its own types read back.
"""

import json
import sys
from dataclasses import dataclass
from typing import Literal, NewType, NotRequired, TypedDict

KIND = "keyed-source"
PROTOCOL_VERSION = 2

# The source's own id for a task, and the id a request is answered under: two domains that
# are both strings on the wire and never interchangeable.
NativeId = NewType("NativeId", str)
RequestId = NewType("RequestId", str)

type Support = Literal["native", "unsupported"]
type DependencySupport = Literal["both-directions", "forward-only"]


class Status(TypedDict):
    """A task's status, as `get_task` and `query_tasks` send it (§4.4, §4.5)."""

    category: str
    name: str


class Task(TypedDict):
    """A task on the wire (§4.4); `key` is left out entirely where the backend has none."""

    id: NativeId
    key: NotRequired[str]
    title: str
    content: str | None
    status: Status
    labels: list[dict[str, str]]
    project: str | None
    url: str | None
    location: dict[str, str] | None
    created_at: str | None
    updated_at: str | None


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


def unfiled(identifier: NativeId, title: str) -> Task:
    """A task in no project, with nothing but its id and title to tell it apart."""
    return {
        "id": identifier,
        "title": title,
        "content": None,
        "status": {"category": "todo", "name": "Todo"},
        "labels": [],
        "project": None,
        "url": None,
        "location": None,
        "created_at": None,
        "updated_at": None,
    }


TASKS: list[Task] = [
    {**unfiled(NativeId("iss_8f2c"), "Engine handle"), "key": "ENG-7"},
    unfiled(NativeId("iss_91d0"), "Engine without a handle"),
]

# Everything a query could narrow by is declared unsupported, so this peer answers every
# `query_tasks` with the whole set and the engine applies the predicates itself (rule 2).
CAPABILITIES: Capabilities = {
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


def malformed(message: str) -> Refused:
    """A request this source cannot read, answered as the contract's `malformed`."""
    return {"error": {"kind": "malformed", "message": message}}


def answer(request: Request) -> Answered | Refused:
    """The result for one method, or a `SourceError` for one this source does not serve."""
    match request.method:
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
            wanted = request.params.get("id")
            if not isinstance(wanted, str):
                return malformed("get_task names its task by a string `id`")
            found = [task for task in TASKS if task["id"] == wanted]
            return {"result": {"task": found[0] if found else None}}
        case "query_tasks":
            return {"result": {"items": TASKS, "next": None}}
        case "labels" | "task_dependencies":
            return {"result": {"items": [], "next": None}}
        case method:
            return malformed(
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


def main() -> None:
    """Serve one connection until the engine closes its input."""
    for line in sys.stdin:
        if not line.strip():
            continue
        request = request_of(line)
        if request is None:
            print(f"{KIND}: ignoring a line that is not a request", file=sys.stderr)
            continue
        sys.stdout.write(json.dumps({"id": request.id, **answer(request)}) + "\n")
        sys.stdout.flush()


if __name__ == "__main__":
    main()
