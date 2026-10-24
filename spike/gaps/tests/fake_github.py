"""A loopback stand-in for GitHub's GraphQL and REST answers to this spike's probes.

It holds issues and board items, places a created issue on the board only after
`placement_reads` reads of it (GitHub places asynchronously), answers an add of placed content
either with its item or with GitHub's "Content already exists" refusal, and sets
`x-ratelimit-*` headers whose `used` rises by one a request. The proxy under test forwards to
it exactly as it forwards to GitHub, so every probe runs through the real relay and guard.
"""

from __future__ import annotations

import datetime
import json
import re
import threading
from dataclasses import dataclass, field
from enum import Enum
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import graphql

from gaps.json_shape import Json, ShapeError, obj, parse, text

REPOSITORY = "R_scratch_repository"
PROJECT = "PVT_test_board"
FIELD = "PVTSSF_status"
OPTION = "option_todo"


class OddAdd(Enum):
    """How the stand-in answers the second aliased add, when told to answer it oddly."""

    REFUSED_OTHERWISE = "refused for a reason other than present content"
    OTHER_ITEM = "answered with an item the issue does not read"
    MALFORMED = "answered with no item at all"


class Chunking(Enum):
    """How the stand-in frames its answers."""

    NONE = "Content-Length"
    LONGER_THAN_SENT = "a Content-Length longer than what is sent"
    WHOLE = "chunked"
    CUT_SHORT = "chunked, without its terminating chunk"
    NO_FINAL_LINE = "chunked, its terminating chunk without the final empty line"


def _type_ref(node: graphql.TypeNode) -> dict[str, object]:
    match node:
        case graphql.NonNullTypeNode():
            return {"kind": "NON_NULL", "name": None, "ofType": _type_ref(node.type)}
        case graphql.ListTypeNode():
            return {"kind": "LIST", "name": None, "ofType": _type_ref(node.type)}
        case graphql.NamedTypeNode():
            return {"kind": "SCALAR", "name": node.name.value, "ofType": None}
        case _:
            raise TypeError(node)


def introspected_rate_limit() -> dict[str, object]:
    """The introspection answer GitHub gives, built from the extension the spike checks in."""
    extension = Path(__file__).resolve().parents[1] / "github-schema-extension.graphql"
    definitions = graphql.parse(extension.read_text()).definitions
    rate = next(d for d in definitions if isinstance(d, graphql.ObjectTypeDefinitionNode))
    query = next(d for d in definitions if isinstance(d, graphql.ObjectTypeExtensionNode))
    field_defs = [{"name": f.name.value, "type": _type_ref(f.type)} for f in rate.fields or ()]
    rate_limit = next(f for f in query.fields or () if f.name.value == "rateLimit")
    arguments = [
        {
            "name": a.name.value,
            "type": _type_ref(a.type),
            "defaultValue": None if a.default_value is None else graphql.print_ast(a.default_value),
        }
        for a in rate_limit.arguments or ()
    ]
    query_field = {"name": "rateLimit", "type": _type_ref(rate_limit.type), "args": arguments}
    return {
        "rateLimitType": {"fields": field_defs},
        "queryType": {"queryType": {"fields": [query_field]}},
    }


@dataclass
class Issue:
    """One created issue."""

    id: str
    title: str
    parent: str | None
    reads: int = 0
    item: str | None = None
    repository: str = REPOSITORY


@dataclass
class Board:
    """The stand-in's state and its knobs."""

    chunked: Chunking = Chunking.NONE
    placement_reads: int = 1
    refuse_present_adds: bool = False
    odd_add: OddAdd | None = None
    fail_deletes: bool = False
    null_deletes: bool = False
    fail_ownership_read: bool = False
    answer_creates: int | None = None
    malformed_create: int | None = None
    create_error: str | None = None
    remaining: int = 4000
    reset: int = 4102444800
    rest_remaining: int | None = None
    tokens: list[str] = field(default_factory=list)
    issues: dict[str, Issue] = field(default_factory=dict)
    used: int = 1000
    seen: list[str] = field(default_factory=list)
    lock: threading.Lock = field(default_factory=threading.Lock)

    def headers(self) -> dict[str, str]:
        """The rate-limit headers of the next answer."""
        self.used += 1
        return {
            "x-ratelimit-limit": "5000",
            "x-ratelimit-remaining": str(self.remaining - self.used),
            "x-ratelimit-used": str(self.used),
            "x-ratelimit-reset": str(self.reset),
            "x-ratelimit-resource": "graphql",
        }

    def items_of(self, issue: Issue) -> dict[str, object]:
        """An issue's `projectItems`, placing it once it has been read enough."""
        issue.reads += 1
        if issue.item is None and issue.reads > self.placement_reads:
            issue.item = f"PVTI_{issue.id}"
        nodes = [] if issue.item is None else [{"id": issue.item, "project": {"id": PROJECT}}]
        return {"nodes": nodes}

    def operation(self, query: str) -> str:
        """Which request this is: its declared name, else what it asks for."""
        name = re.search(r"^\s*(?:query|mutation)\s+(\w+)", query)
        if name:
            return name.group(1)
        if "rateLimit(dryRun:true)" in query:
            return "dry run"
        if "subIssues(" in query:
            return "SUB_ISSUES"
        if "rateLimit{cost used" in query:
            return "charged read"
        if "boardFields:repositoryOwner" in query and "repository(owner:" in query:
            return "CREATION_CONTEXT"
        return "anonymous"

    def answer(self, query: str, variables: dict[str, Json]) -> dict[str, object]:
        """The GraphQL answer to one request."""
        op = self.operation(query)
        self.seen.append(op)
        match op:
            case "dry run":
                return {"data": {"rateLimit": {"cost": 1}}}
            case "SpikeRateLimitSchema":
                return {"data": introspected_rate_limit()}
            case "SpikeRateLimit":
                reset = datetime.datetime.fromtimestamp(self.reset, datetime.UTC).isoformat()
                rate = {"limit": 5000, "used": self.used, "remaining": self.remaining - self.used}
                return {"data": {"rateLimit": rate | {"resetAt": reset.replace("+00:00", "Z")}}}
            case "CREATION_CONTEXT":
                status = {"__typename": "ProjectV2SingleSelectField", "id": FIELD, "name": "Status"}
                status["options"] = [{"id": OPTION, "name": "Todo"}]
                fields = {"nodes": [status], "pageInfo": {"hasNextPage": False}}
                owner = {"projectV2": {"id": PROJECT, "fields": fields}}
                repository = {"id": REPOSITORY, "nameWithOwner": "o/r"}
                return {"data": {"boardFields": owner, "repository": repository}}
            case create if create.startswith("ProbeCreate"):
                return self.create(variables)
            case "ProbePlaced":
                data: dict[str, object] = {"rateLimit": {"cost": 1}}
                for key, value in variables.items():
                    issue = self.issues.get(str(value))
                    placed = (
                        None
                        if issue is None
                        else {"id": issue.id} | {"projectItems": self.items_of(issue)}
                    )
                    data["i" + key.removeprefix("id")] = placed
                return {"data": data}
            case "ProbeAdd":
                return self.add(variables)
            case "ProbeCoalesced" | "ProbeNineteen":
                aliases = re.findall(
                    r"(\w+): (?:updateProjectV2ItemFieldValue|addBlockedBy)", query
                )
                return {"data": {alias: {"projectV2Item": {"id": "x"}} for alias in aliases}}
            case "SUB_ISSUES":
                parent = str(variables.get("id"))
                children = [{"id": i.id} for i in self.issues.values() if i.parent == parent]
                node = {"subIssues": {"nodes": children}}
                return {"data": {"rateLimit": {"cost": 1}, "node": node}}
            case "ProbeOwn":
                return self.own(variables)
            case "ProbeDelete":
                return self.delete(variables)
            case "charged read":
                rate = {"cost": 1, "used": self.used, "remaining": 1, "resetAt": "x"}
                return {"data": {"rateLimit": rate}}
            case _:
                error = {"type": "UNKNOWN", "message": f"the stand-in has no {op}"}
                return {"data": None, "errors": [error]}

    def create(self, variables: dict[str, Json]) -> dict[str, object]:
        """Create every aliased input, or as many as `answer_creates` allows."""
        if self.create_error is not None:
            error = {"type": "UNPROCESSABLE", "path": ["c0"], "message": self.create_error}
            return {"data": None, "errors": [error]}
        answered = list(variables.items())[: self.answer_creates]
        data: dict[str, object] = {}
        for alias, value in answered:
            created = value if isinstance(value, dict) else {}
            parent_id = created.get("parentIssueId")
            issue = Issue(
                id=f"I_{len(self.issues) + 1}",
                title=str(created.get("title")),
                parent=parent_id if isinstance(parent_id, str) else None,
            )
            self.issues[issue.id] = issue
            parent = None if issue.parent is None else {"id": issue.parent}
            answered_id: object = issue.id
            if self.malformed_create == len(data):
                answered_id = len(self.issues)  # created, but answered without a usable id
            answer = {"id": answered_id, "parent": parent, "projectItems": {"nodes": []}}
            data[alias] = {"issue": answer}
        if len(answered) < len(variables):
            error = {"type": "UNPROCESSABLE", "path": ["cN"], "message": "not created"}
            return {"data": data, "errors": [error]}
        return {"data": data}

    def add(self, variables: dict[str, Json]) -> dict[str, object]:
        """Add each aliased issue, refusing present content when told to."""
        data: dict[str, object] = {}
        errors = []
        for alias in ("a", "b", "c"):
            issue = self.issues[str(variables[alias])]
            if alias == "b" and self.odd_add is not None:
                match self.odd_add:
                    case OddAdd.REFUSED_OTHERWISE:
                        data[alias] = None
                        errors.append({"type": "FORBIDDEN", "path": [alias], "message": "no"})
                    case OddAdd.OTHER_ITEM:
                        issue.item = issue.item or f"PVTI_{issue.id}"
                        data[alias] = {"item": {"id": "PVTI_somewhere_else"}}
                    case OddAdd.MALFORMED:
                        issue.item = issue.item or f"PVTI_{issue.id}"
                        data[alias] = {"item": None}
                continue
            if issue.item is not None and self.refuse_present_adds:
                data[alias] = None
                message = "Content already exists in this project"
                errors.append({"type": "UNPROCESSABLE", "path": [alias], "message": message})
                continue
            issue.item = issue.item or f"PVTI_{issue.id}"
            data[alias] = {"item": {"id": issue.item}}
        return {"data": data, "errors": errors} if errors else {"data": data}

    def own(self, variables: dict[str, Json]) -> dict[str, object]:
        """Read each issue's title and repository, `NOT_FOUND` for one deleted."""
        if self.fail_ownership_read:
            return {"data": None, "errors": [{"type": "FORBIDDEN", "message": "no"}]}
        data: dict[str, object] = {}
        errors = []
        for alias, value in variables.items():
            issue = self.issues.get(str(value))
            if issue is None:
                data[alias] = None
                errors.append({"type": "NOT_FOUND", "path": [alias], "message": "gone"})
            else:
                repository = {"id": issue.repository}
                data[alias] = {"id": issue.id, "title": issue.title, "repository": repository}
        return {"data": data, "errors": errors} if errors else {"data": data}

    def delete(self, variables: dict[str, Json]) -> dict[str, object]:
        """Delete each aliased issue, or refuse every one when told to."""
        if self.fail_deletes:
            return {
                "data": None,
                "errors": [{"type": "FORBIDDEN", "path": ["d0"], "message": "no"}],
            }
        if self.null_deletes:
            return {"data": dict.fromkeys(variables)}
        for value in variables.values():
            self.issues.pop(str(value), None)
        return {"data": {alias: {"repository": {"id": REPOSITORY}} for alias in variables}}


def serve(board: Board) -> tuple[ThreadingHTTPServer, str]:
    """Serve `board` on a free loopback port in a daemon thread."""

    class Handler(BaseHTTPRequestHandler):
        def _send(self, status: int, payload: dict[str, object]) -> None:
            body = json.dumps(payload).encode()
            self.send_response(status)
            for name, value in board.headers().items():
                self.send_header(name, value)
            self.send_header("Content-Type", "application/json")
            if board.chunked in (Chunking.NONE, Chunking.LONGER_THAN_SENT):
                extra = 10 if board.chunked is Chunking.LONGER_THAN_SENT else 0
                self.send_header("Content-Length", str(len(body) + extra))
                self.end_headers()
                self.wfile.write(body)
                self.close_connection = True
                return
            # GitHub answers chunked; the stand-in does too, in two chunks, and when told to
            # cuts the answer short before its terminating chunk.
            self.send_header("Transfer-Encoding", "chunked")
            self.end_headers()
            half = len(body) // 2
            for piece in (body[:half], body[half:]):
                self.wfile.write(f"{len(piece):x};ext=1\r\n".encode() + piece + b"\r\n")
            match board.chunked:
                case Chunking.WHOLE:
                    self.wfile.write(b"0\r\n\r\n")
                case Chunking.NO_FINAL_LINE:
                    self.wfile.write(b"0\r\n")
                case _:
                    pass
            self.close_connection = True

        def do_GET(self) -> None:
            with board.lock:
                board.tokens.append(self.headers.get("Authorization", ""))
                remaining = board.rest_remaining
                if remaining is None:
                    remaining = board.remaining - board.used
                bucket = {"limit": 5000, "used": 5000 - remaining, "remaining": remaining}
                bucket["reset"] = board.reset
                self._send(200, {"resources": {"core": bucket, "graphql": bucket}})

        def do_POST(self) -> None:
            length = int(self.headers.get("Content-Length") or 0)
            try:
                request = obj(parse(self.rfile.read(length).decode(), "request"), "request")
                query = text(request.get("query"), "request.query")
                variables = obj(request.get("variables") or {}, "request.variables")
            except ShapeError as error:
                self._send(400, {"errors": [{"type": "BAD_REQUEST", "message": str(error)}]})
                return
            with board.lock:
                board.tokens.append(self.headers.get("Authorization", ""))
                self._send(200, board.answer(query, variables))

        def log_message(self, format: str, *args: object) -> None:
            pass

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server, f"http://127.0.0.1:{server.server_address[1]}"
