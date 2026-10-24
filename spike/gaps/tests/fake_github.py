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
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

from gaps.json_shape import Json, ShapeError, obj, parse, text

REPOSITORY = "R_scratch_repository"
PROJECT = "PVT_test_board"
FIELD = "PVTSSF_status"
OPTION = "option_todo"


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

    placement_reads: int = 1
    refuse_present_adds: bool = False
    fail_deletes: bool = False
    fail_ownership_read: bool = False
    answer_creates: int | None = None
    create_error: str | None = None
    remaining: int = 4000
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
            "x-ratelimit-reset": "4102444800",
            "x-ratelimit-resource": "graphql",
        }

    def items_of(self, issue: Issue) -> dict[str, object]:
        """An issue's `projectItems`, placing it once it has been read enough."""
        issue.reads += 1
        if issue.item is None and issue.reads > self.placement_reads:
            issue.item = f"PVTI_{issue.id}"
        nodes = [] if issue.item is None else [{"id": issue.item, "project": {"id": PROJECT}}]
        return {"nodes": nodes}

    def answer(self, query: str, variables: dict[str, Json]) -> dict[str, object]:
        """The GraphQL answer to one request."""
        name = re.search(r"^\s*(?:query|mutation)\s+(\w+)", query)
        op = name.group(1) if name else ""
        self.seen.append(op or ("dry run" if "dryRun" in query else "anonymous"))
        if "rateLimit(dryRun:true)" in query:
            return {"data": {"rateLimit": {"cost": 1}}}
        if op == "SpikeRateLimit":
            reset = datetime.datetime(2100, 1, 1, tzinfo=datetime.UTC).isoformat()
            rate = {"limit": 5000, "used": self.used, "remaining": self.remaining - self.used}
            return {"data": {"rateLimit": rate | {"resetAt": reset.replace("+00:00", "Z")}}}
        if op == "ProbeIds":
            options = [{"id": OPTION, "name": "Todo"}]
            project = {"id": PROJECT, "field": {"id": FIELD, "options": options}}
            return {"data": {"repository": {"id": REPOSITORY}, "user": {"projectV2": project}}}
        if op.startswith("ProbeCreate"):
            data = {}
            if self.create_error is not None:
                error = {"type": "UNPROCESSABLE", "path": ["c0"], "message": self.create_error}
                return {"data": None, "errors": [error]}
            answered = list(variables.items())[: self.answer_creates]
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
                data[alias] = {
                    "issue": {"id": issue.id, "parent": parent, "projectItems": {"nodes": []}}
                }
            if len(answered) < len(variables):
                error = {"type": "UNPROCESSABLE", "path": ["cN"], "message": "not created"}
                return {"data": data, "errors": [error]}
            return {"data": data}
        if op == "ProbePlaced":
            data: dict[str, object] = {"rateLimit": {"cost": 1}}
            for key, value in variables.items():
                issue = self.issues.get(str(value))
                alias = "i" + key.removeprefix("id")
                data[alias] = None if issue is None else {"projectItems": self.items_of(issue)}
            return {"data": data}
        if op == "ProbeAdd":
            data, errors = {}, []
            for alias in ("a", "b", "c"):
                issue = self.issues[str(variables[alias])]
                if issue.item is not None and self.refuse_present_adds:
                    data[alias] = None
                    errors.append(
                        {
                            "type": "UNPROCESSABLE",
                            "path": [alias],
                            "message": "Content already exists in this project",
                        }
                    )
                    continue
                issue.item = issue.item or f"PVTI_{issue.id}"
                data[alias] = {"item": {"id": issue.item}}
            return {"data": data, "errors": errors} if errors else {"data": data}
        if op in ("ProbeCoalesced", "ProbeNineteen"):
            aliases = re.findall(r"(\w+): (?:updateProjectV2ItemFieldValue|addBlockedBy)", query)
            return {"data": {alias: {"projectV2Item": {"id": "x"}} for alias in aliases}}
        if "subIssues(" in query:
            parent = str(variables.get("id"))
            children = [{"id": i.id} for i in self.issues.values() if i.parent == parent]
            node = {"subIssues": {"nodes": children}}
            return {"data": {"rateLimit": {"cost": 1}, "node": node}}
        if op == "ProbeOwn":
            if self.fail_ownership_read:
                return {"data": None, "errors": [{"type": "FORBIDDEN", "message": "no"}]}
            data, errors = {}, []
            for key, value in variables.items():
                alias = key
                issue = self.issues.get(str(value))
                if issue is None:
                    data[alias] = None
                    errors.append({"type": "NOT_FOUND", "path": [alias], "message": "gone"})
                else:
                    repository = {"id": issue.repository}
                    data[alias] = {"id": issue.id, "title": issue.title, "repository": repository}
            return {"data": data, "errors": errors} if errors else {"data": data}
        if op == "ProbeDelete":
            if self.fail_deletes:
                return {
                    "data": None,
                    "errors": [{"type": "FORBIDDEN", "path": ["d0"], "message": "no"}],
                }
            for value in variables.values():
                self.issues.pop(str(value), None)
            return {"data": {alias: {"clientMutationId": None} for alias in variables}}
        if "rateLimit{cost used" in query:
            return {
                "data": {
                    "rateLimit": {"cost": 1, "used": self.used, "remaining": 1, "resetAt": "x"}
                }
            }
        return {
            "data": None,
            "errors": [{"type": "UNKNOWN", "message": f"the stand-in has no {op}"}],
        }


def serve(board: Board) -> tuple[ThreadingHTTPServer, str]:
    """Serve `board` on a free loopback port in a daemon thread."""

    class Handler(BaseHTTPRequestHandler):
        def _send(self, status: int, payload: dict[str, object]) -> None:
            body = json.dumps(payload).encode()
            self.send_response(status)
            for name, value in board.headers().items():
                self.send_header(name, value)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def do_GET(self) -> None:
            with board.lock:
                board.tokens.append(self.headers.get("Authorization", ""))
                remaining = board.rest_remaining
                if remaining is None:
                    remaining = board.remaining - board.used
                bucket = {"limit": 5000, "used": 5000 - remaining, "remaining": remaining}
                bucket["reset"] = 4102444800
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
