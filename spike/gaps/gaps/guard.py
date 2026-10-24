"""The guarded proxy every live request of this spike leaves through.

`python -m gaps.guard --log L --port-file P [--allow ID]... [--allow-issue ID]... [--upstream URL]`

An async forwarder on loopback: it answers `POST /graphql` and `GET /rate_limit` by forwarding
them to GitHub (or, for the tests, to the loopback stand-in `--upstream` names) and refuses
everything else locally — every other path, every upload, every `CONNECT` tunnel. A mutation
passes only when every field it calls is one the probes send, every repository or project id
it names is `--allow`ed, and every issue it names is one a `createIssue` this proxy forwarded
answered, or an `--allow-issue` leftover of an interrupted probe; with no `--allow`, no mutation
passes. A board item a mutation names must be one an answer listed under such an issue. Each
refusal is one line of `L`: the reason, never the request.
"""

from __future__ import annotations

import argparse
import asyncio
import json
import re
import ssl
from dataclasses import dataclass, field
from pathlib import Path
from urllib.parse import urlsplit

from gaps.github import GitHubError, split_reply
from gaps.json_shape import Json, ShapeError, obj, parse, text

GUARDED = re.compile(r"\b(repositoryId|projectId|projectV2Ids)\s*:\s*")
ISSUES = re.compile(r"\b(issueId|blockingIssueId|contentId|subIssueId|parentIssueId)\s*:\s*")
ITEMS = re.compile(r"\b(itemId)\s*:\s*")
COMMENT = re.compile(r"#[^\n]*")
STRING = re.compile(r'"(?:[^"\\]|\\.)*"')
MUTATION = re.compile(r"\bmutation\b")
LITERAL = re.compile(r'"((?:[^"\\]|\\.)*)"')
MUTATION_FIELD = re.compile(r"(?:\w+\s*:\s*)?(\w+)\s*\(\s*input\s*:")
PROBE_MUTATIONS = frozenset(
    {
        "createIssue",
        "addProjectV2ItemById",
        "updateProjectV2ItemFieldValue",
        "addBlockedBy",
        "deleteIssue",
    }
)
DESTINATION_KEYS = frozenset({"repositoryId", "projectId", "projectV2Ids"})
ISSUE_KEYS = frozenset({"issueId", "blockingIssueId", "contentId", "subIssueId", "parentIssueId"})
ITEM_KEYS = frozenset({"itemId"})
GITHUB = "https://api.github.com"
LOOPBACK = frozenset({"127.0.0.1", "localhost"})
MAX_BODY = 4 * 1024 * 1024
TIMEOUT = 120.0
UNREADABLE = "<unreadable>"


def _value_ids(value: Json) -> set[str]:
    match value:
        case list():
            return {str(v) for v in value}
        case None:
            return set()
        case _:
            return {str(value)}


def _named(pattern: re.Pattern[str], query: str, variables: dict[str, Json]) -> set[str]:
    """Every id the fields `pattern` matches name in `query`, however written."""
    names: set[str] = set()
    for match in pattern.finditer(query):
        rest = query[match.end() :]
        if rest.startswith("$"):
            variable = re.match(r"\$(\w+)", rest)
            names |= _value_ids(variables.get(variable.group(1)) if variable else None)
        elif rest.startswith("["):
            end = rest.find("]")
            if end < 0:
                names.add(UNREADABLE)
                continue
            for entry in (e.strip() for e in rest[1:end].split(",") if e.strip()):
                literal = LITERAL.fullmatch(entry)
                if entry.startswith("$"):
                    names |= _value_ids(variables.get(entry[1:]))
                elif literal is not None:
                    names.add(literal.group(1))
                else:
                    names.add(UNREADABLE)
        elif (literal := LITERAL.match(rest)) is not None:
            names.add(literal.group(1))
        else:
            names.add(UNREADABLE)
    return names


def guarded_values(value: Json, keys: frozenset[str]) -> set[str]:
    """Every value held under one of `keys`, at any depth of a variables object."""
    found: set[str] = set()
    match value:
        case dict():
            for key, inner in value.items():
                found |= _value_ids(inner) if key in keys else guarded_values(inner, keys)
        case list():
            for inner in value:
                found |= guarded_values(inner, keys)
        case _:
            pass
    return found


def named_ids(query: str, variables: dict[str, Json]) -> set[str]:
    """Every repository or project id a mutation names, however it is written."""
    return _named(GUARDED, query, variables) | guarded_values(variables, DESTINATION_KEYS)


def named_issues(query: str, variables: dict[str, Json]) -> set[str]:
    """Every issue id a mutation names, however it is written.

    As the issue it writes, the issue blocking it, its parent, its sub-issue, or the content it
    adds to a board.
    """
    return _named(ISSUES, query, variables) | guarded_values(variables, ISSUE_KEYS)


def named_items(query: str, variables: dict[str, Json]) -> set[str]:
    """Every board item id a mutation names, however it is written."""
    return _named(ITEMS, query, variables) | guarded_values(variables, ITEM_KEYS)


def is_mutation(query: str) -> bool:
    """Whether a document defines a mutation anywhere, comments and string values aside."""
    return MUTATION.search(STRING.sub('""', COMMENT.sub("", query))) is not None


@dataclass
class Guard:
    """What this proxy lets through, and the issues it has seen created."""

    destinations: frozenset[str]
    issues: set[str] = field(default_factory=set)
    items: set[str] = field(default_factory=set)

    def refusal(self, method: str, path: str, body: bytes) -> str | None:
        """Why a request must not leave, or `None`."""
        if (method, path) == ("GET", "/rate_limit"):
            return None
        if (method, path) != ("POST", "/graphql"):
            return f"this run sends no {method} {path.split('?')[0]}"
        try:
            payload = obj(parse(body.decode(errors="replace"), "request"), "request")
            query = text(payload.get("query"), "request.query")
            variables = obj(payload.get("variables") or {}, "request.variables")
        except ShapeError:
            return "an unreadable request"
        if not is_mutation(query):
            return None
        if not self.destinations:
            return "this run sends no writes"
        fields = set(MUTATION_FIELD.findall(query))
        if not fields or fields - PROBE_MUTATIONS:
            return "a mutation this spike's probes never send"
        if named_ids(query, variables) - self.destinations:
            return "it names a repository or project outside the test board"
        if named_issues(query, variables) - self.issues:
            return "it names an issue this run did not create"
        if named_items(query, variables) - self.items:
            return "it names a board item of an issue this run did not create"
        return None

    def learn(self, request: bytes, body: bytes) -> None:
        """Record what a forwarded answer proves this run owns.

        An issue is owned only when a `createIssue` this proxy forwarded answered it, never
        because a read returned it; a board item only when an answer lists it under an issue
        this run owns.
        """
        try:
            query = text(
                obj(parse(request.decode(errors="replace"), "request"), "r").get("query"), "q"
            )
            answer = obj(parse(body.decode(errors="replace"), "answer"), "answer")
            data = answer.get("data")
        except ShapeError:
            return
        if not isinstance(data, dict):
            return
        if is_mutation(query) and "createIssue" in MUTATION_FIELD.findall(query):
            for value in data.values():
                issue = value.get("issue") if isinstance(value, dict) else None
                if isinstance(issue, dict) and isinstance(issue.get("id"), str):
                    self.issues.add(text(issue.get("id"), "issue.id"))
        for value in data.values():
            if isinstance(value, dict) and value.get("id") in self.issues:
                connection = value.get("projectItems")
                nodes = connection.get("nodes") if isinstance(connection, dict) else None
                for node in nodes if isinstance(nodes, list) else []:
                    if isinstance(node, dict) and isinstance(node.get("id"), str):
                        self.items.add(text(node.get("id"), "item.id"))


@dataclass(frozen=True)
class Request:
    """One request a client sent the proxy."""

    method: str
    path: str
    headers: dict[str, str]
    body: bytes


class BadRequest(ValueError):
    """A request the proxy cannot read."""


async def read_request(reader: asyncio.StreamReader) -> Request:
    """One HTTP/1.1 request, its length validated."""
    head = await asyncio.wait_for(reader.readuntil(b"\r\n\r\n"), TIMEOUT)
    lines = head.decode("latin-1").split("\r\n")
    parts = lines[0].split(" ")
    if len(parts) != 3:
        raise BadRequest(f"an unreadable request line: {lines[0]!r}")
    headers = {}
    for line in lines[1:]:
        name, sep, value = line.partition(":")
        if sep:
            headers[name.strip().lower()] = value.strip()
    length_text = headers.get("content-length", "0")
    if not length_text.isdigit() or int(length_text) > MAX_BODY:
        raise BadRequest(f"an unacceptable Content-Length: {length_text!r}")
    body = await asyncio.wait_for(reader.readexactly(int(length_text)), TIMEOUT)
    return Request(method=parts[0], path=parts[1], headers=headers, body=body)


async def forward(upstream: str, request: Request) -> tuple[int, dict[str, str], bytes]:
    """Send `request` to `upstream`; answer its status, headers and body."""
    target = urlsplit(upstream)
    secure = target.scheme == "https"
    host = target.hostname or ""
    port = target.port or (443 if secure else 80)
    context = ssl.create_default_context() if secure else None
    reader, writer = await asyncio.wait_for(
        asyncio.open_connection(host, port, ssl=context), TIMEOUT
    )
    try:
        head = [f"{request.method} {request.path} HTTP/1.1", f"Host: {host}", "Connection: close"]
        for name in ("authorization", "content-type", "user-agent", "accept"):
            if name in request.headers:
                head.append(f"{name}: {request.headers[name]}")
        if request.body:
            head.append(f"Content-Length: {len(request.body)}")
        writer.write(("\r\n".join(head) + "\r\n\r\n").encode() + request.body)
        await writer.drain()
        reply = split_reply(await asyncio.wait_for(reader.read(), TIMEOUT))
    finally:
        writer.close()
    return reply.status, reply.headers, reply.body


async def respond(
    writer: asyncio.StreamWriter, status: int, headers: dict[str, str], body: bytes
) -> None:
    """Write one answer, keeping only its content type and rate-limit headers, and close."""
    kept = {k: v for k, v in headers.items() if k.startswith("x-ratelimit") or k == "content-type"}
    head = [f"HTTP/1.1 {status} {'OK' if status == 200 else 'Refused'}", "Connection: close"]
    head += [f"{k}: {v}" for k, v in kept.items()] + [f"Content-Length: {len(body)}"]
    writer.write(("\r\n".join(head) + "\r\n\r\n").encode() + body)
    await writer.drain()
    writer.close()


def refused_body(why: str) -> bytes:
    """GitHub's error shape, carrying the guard's reason."""
    error = {"type": "SPIKE_REFUSED", "message": f"spike guard: {why}"}
    return json.dumps({"errors": [error]}).encode()


JSON_TYPE = {"content-type": "application/json"}


@dataclass
class Proxy:
    """The serving half: one guard, one upstream, one refusal log."""

    guard: Guard
    upstream: str
    log: Path

    def refused(self, why: str) -> None:
        """Log one refusal."""
        with self.log.open("a", encoding="utf-8") as out:
            out.write(json.dumps({"refused": why}) + "\n")

    async def handle(self, reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
        """Serve one connection: refuse, or forward and pass the answer back."""
        try:
            request = await read_request(reader)
        except (BadRequest, asyncio.IncompleteReadError, asyncio.LimitOverrunError) as error:
            self.refused(f"an unreadable request ({type(error).__name__})")
            await respond(writer, 400, JSON_TYPE, refused_body("an unreadable request"))
            return
        why = (
            f"this run opens no tunnel to {request.path.split(':')[0]}"
            if request.method == "CONNECT"
            else self.guard.refusal(request.method, request.path, request.body)
        )
        if why is not None:
            self.refused(why)
            await respond(writer, 403, JSON_TYPE, refused_body(why))
            return
        try:
            status, headers, body = await forward(self.upstream, request)
        except (OSError, GitHubError, TimeoutError) as error:
            await respond(writer, 502, JSON_TYPE, refused_body(f"upstream: {error}"))
            return
        self.guard.learn(request.body, body)
        await respond(writer, status, headers, body)


async def serve(proxy: Proxy, port_file: Path) -> None:
    """Serve on a free loopback port, written to `port_file`, until stopped."""
    server = await asyncio.start_server(proxy.handle, "127.0.0.1", 0)
    port_file.write_text(str(server.sockets[0].getsockname()[1]))
    async with server:
        await server.serve_forever()


def main() -> None:
    """Start the guarded proxy."""
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--log", type=Path, required=True)
    parser.add_argument("--port-file", type=Path, required=True)
    parser.add_argument("--allow", action="append", default=[])
    parser.add_argument("--allow-issue", action="append", default=[])
    parser.add_argument("--upstream", default=GITHUB)
    args = parser.parse_args()
    target = urlsplit(args.upstream)
    loopback = target.scheme == "http" and target.hostname in LOOPBACK
    if args.upstream.rstrip("/") != GITHUB and not loopback:
        parser.error(f"--upstream is {GITHUB}, or a loopback stand-in over plain HTTP")
    guard = Guard(frozenset(args.allow), set(args.allow_issue))
    asyncio.run(serve(Proxy(guard, args.upstream, args.log), args.port_file))


if __name__ == "__main__":
    main()
