"""The guarded proxy every live request of this spike leaves through.

`python -m gaps.guard --log L --port-file P [--allow ID]... [--allow-issue ID]... [--upstream URL]`

An async forwarder on loopback: it answers `POST /graphql` and `GET /rate_limit` by forwarding
them to GitHub (or, for the tests, to the loopback stand-in `--upstream` names) and refuses
everything else locally — every other path, every upload, every `CONNECT` tunnel. A mutation
passes only when every field it calls is one the probes send — the document parsed by
graphql-core, as GitHub parses it — every repository or project id it names is `--allow`ed,
and every issue it names is one a `createIssue` this proxy forwarded answered, or an
`--allow-issue` leftover of an interrupted probe; with no `--allow`, no mutation passes. A
board item a mutation names must be one an answer listed under such an issue. Each refusal is
one line of `L`: the reason, never the request.
"""

from __future__ import annotations

import argparse
import asyncio
import json
import ssl
from dataclasses import dataclass, field
from pathlib import Path
from urllib.parse import urlsplit

import graphql

from gaps.github import GitHubError, split_reply
from gaps.json_shape import Json, ShapeError, obj, parse, text

DESTINATION_KEYS = frozenset({"repositoryId", "projectId", "projectV2Ids"})
ISSUE_KEYS = frozenset(
    {"issueId", "blockingIssueId", "contentId", "subIssueId", "parentIssueId", "id"}
)
ITEM_KEYS = frozenset({"itemId"})
PROBE_MUTATIONS = frozenset(
    {
        "createIssue",
        "addProjectV2ItemById",
        "updateProjectV2ItemFieldValue",
        "addBlockedBy",
        "deleteIssue",
    }
)
GITHUB = "https://api.github.com"
LOOPBACK = frozenset({"127.0.0.1", "localhost"})
MAX_BODY = 4 * 1024 * 1024
TIMEOUT = 120.0


class Unreadable(ValueError):
    """A request the guard cannot read as one GraphQL operation."""


@dataclass(frozen=True)
class Operation:
    """What one GraphQL request would execute, read by GitHub's own grammar."""

    mutation: bool
    fields: tuple[tuple[str, str], ...]  # (alias, field) of every top-level field
    destinations: frozenset[str]
    issues: frozenset[str]
    items: frozenset[str]


def _keyed(value: object, keys: frozenset[str]) -> set[str]:
    """Every value held under one of `keys`, at any depth of a resolved argument value."""
    found: set[str] = set()
    match value:
        case dict():
            for key, inner in value.items():
                if key in keys:
                    found |= {str(v) for v in inner} if isinstance(inner, list) else {str(inner)}
                else:
                    found |= _keyed(inner, keys)
        case list():
            for inner in value:
                found |= _keyed(inner, keys)
        case _:
            pass
    return found


def operation(query: str, variables: dict[str, Json], name: str | None) -> Operation:
    """The operation a request executes: its kind, its fields and every id its arguments name.

    The document is parsed by graphql-core, so comments, strings and block strings read as
    GitHub reads them; a document defining several operations needs `operationName`; every
    argument is resolved with the variables sent and each variable's declared default.
    """
    try:
        document = graphql.parse(query)
    except graphql.GraphQLError as error:
        raise Unreadable(f"not a GraphQL document: {error.message}") from error
    operations = [d for d in document.definitions if isinstance(d, graphql.OperationDefinitionNode)]
    if name is not None:
        operations = [o for o in operations if o.name is not None and o.name.value == name]
    if len(operations) != 1:
        raise Unreadable(f"{len(operations)} operations to choose from")
    chosen = operations[0]
    resolved: dict[str, object] = {}
    for definition in chosen.variable_definitions or ():
        variable = definition.variable.name.value
        if variable in variables:
            resolved[variable] = variables[variable]
        elif definition.default_value is not None:
            resolved[variable] = graphql.value_from_ast_untyped(definition.default_value)
    fields: list[tuple[str, str]] = []
    arguments: list[object] = []
    for selection in chosen.selection_set.selections:
        if not isinstance(selection, graphql.FieldNode):
            raise Unreadable("a fragment at the root of an operation")
        alias = selection.alias.value if selection.alias else selection.name.value
        fields.append((alias, selection.name.value))
        for argument in selection.arguments or ():
            arguments.append(graphql.value_from_ast_untyped(argument.value, resolved))
    return Operation(
        mutation=chosen.operation is graphql.OperationType.MUTATION,
        fields=tuple(fields),
        destinations=frozenset(_keyed(arguments, DESTINATION_KEYS)),
        issues=frozenset(_keyed(arguments, ISSUE_KEYS)),
        items=frozenset(_keyed(arguments, ITEM_KEYS)),
    )


def read_payload(body: bytes) -> Operation:
    """A request body as the operation it executes."""
    try:
        payload = obj(parse(body.decode(errors="replace"), "request"), "request")
        query = text(payload.get("query"), "request.query")
        variables = obj(payload.get("variables") or {}, "request.variables")
        name = payload.get("operationName")
        if name is not None:
            name = text(name, "request.operationName")
    except ShapeError as error:
        raise Unreadable(str(error)) from error
    return operation(query, variables, name)


@dataclass
class Guard:
    """What this proxy lets through, and the issues and items it has seen this run own."""

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
            executed = read_payload(body)
        except Unreadable as error:
            return f"an unreadable request ({error})"
        if not executed.mutation:
            return None
        if not self.destinations:
            return "this run sends no writes"
        if {f for _, f in executed.fields} - PROBE_MUTATIONS:
            return "a mutation this spike's probes never send"
        if executed.destinations - self.destinations:
            return "it names a repository or project outside the test board"
        if executed.issues - self.issues:
            return "it names an issue this run did not create"
        if executed.items - self.items:
            return "it names a board item of an issue this run did not create"
        return None

    def learn(self, request: bytes, body: bytes) -> None:
        """Record what a forwarded answer proves this run owns.

        An issue is owned only when a `createIssue` this proxy forwarded answered it, never
        because a read returned it; a board item only when an answer lists it under an issue
        this run owns.
        """
        try:
            executed = read_payload(request)
            data = obj(parse(body.decode(errors="replace"), "answer"), "answer").get("data")
        except (Unreadable, ShapeError):
            return
        if not isinstance(data, dict):
            return
        if executed.mutation:
            for alias, name in executed.fields:
                answer = data.get(alias)
                issue = answer.get("issue") if isinstance(answer, dict) else None
                if name == "createIssue" and isinstance(issue, dict):
                    found = issue.get("id")
                    if isinstance(found, str):
                        self.issues.add(found)
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
