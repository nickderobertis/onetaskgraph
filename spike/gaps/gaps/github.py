"""A small async, typed GitHub client that speaks only to the guard on loopback.

Every live request of this spike leaves through `gaps.guard`, which forwards to GitHub and
refuses any write it does not allow, so this client opens plain HTTP/1.1
connections to `127.0.0.1` and nothing else; an endpoint that is not loopback is refused when
the client is made. Each answer is recorded as a `RequestRecord` — timings, the operation's
name, its stated cost, error types and the rate-limit headers — and never a body, so the log
it writes is already sanitized.
"""

from __future__ import annotations

import asyncio
import json
import time
from dataclasses import dataclass, field
from enum import StrEnum
from pathlib import Path
from typing import NewType
from urllib.parse import urlsplit

from gaps.json_shape import Json, ShapeError, arr, decode, encode, obj, parse, text

Token = NewType("Token", str)

LOOPBACK = frozenset({"127.0.0.1", "localhost"})
TIMEOUT = 120.0


class Kind(StrEnum):
    """What one request was."""

    QUERY = "query"
    MUTATION = "mutation"
    REST = "rest"


@dataclass(frozen=True)
class RateLimit:
    """The `x-ratelimit-*` headers of one answer."""

    limit: int
    remaining: int
    used: int
    reset: int
    resource: str

    @classmethod
    def from_headers(cls, headers: dict[str, str]) -> RateLimit | None:
        """The headers' reading, or `None` when an answer carried none."""
        try:
            return cls(
                limit=int(headers["x-ratelimit-limit"]),
                remaining=int(headers["x-ratelimit-remaining"]),
                used=int(headers["x-ratelimit-used"]),
                reset=int(headers["x-ratelimit-reset"]),
                resource=headers.get("x-ratelimit-resource", ""),
            )
        except (KeyError, ValueError):
            return None


@dataclass(frozen=True)
class GraphQLError:
    """One entry of an answer's `errors`."""

    type: str
    path: tuple[str, ...]
    message: str


@dataclass(frozen=True)
class RequestRecord:
    """What the log keeps of one request: no body, no id, no title."""

    seq: int
    kind: Kind
    op: str
    start: float
    end: float
    status: int
    cost: int | None
    errors: tuple[str, ...]
    limits: RateLimit | None
    aliases: int

    @classmethod
    def parse(cls, value: Json, where: str) -> RequestRecord:
        """A logged record, validated."""
        return decode(cls, value, where)

    def to_json(self) -> Json:
        """The record as one JSON line."""
        return encode(self)


@dataclass(frozen=True)
class Answer:
    """One GraphQL answer."""

    data: dict[str, Json]
    errors: tuple[GraphQLError, ...]
    record: RequestRecord

    @property
    def seconds(self) -> float:
        """How long the request took, end to end."""
        return self.record.end - self.record.start


@dataclass(frozen=True)
class HttpReply:
    """A raw HTTP answer."""

    status: int
    headers: dict[str, str]
    body: bytes


class GitHubError(RuntimeError):
    """GitHub, or the proxy in front of it, did not answer usably."""


@dataclass
class Client:
    """Async GraphQL and REST requests through the loopback proxy, each one logged."""

    endpoint: str
    token: Token
    log: Path | None = None
    records: list[RequestRecord] = field(default_factory=list)

    def __post_init__(self) -> None:
        """Refuse an endpoint that is not plain HTTP on loopback."""
        parts = urlsplit(self.endpoint)
        if parts.scheme != "http" or parts.hostname not in LOOPBACK or parts.port is None:
            raise ValueError(f"the client speaks only to the loopback proxy, not {self.endpoint}")
        if not self.token or any(c.isspace() or not c.isprintable() for c in self.token):
            raise ValueError("the credential is empty or carries whitespace or control characters")
        self._host = parts.hostname
        self._port = parts.port

    async def _send(self, method: str, path: str, body: bytes | None) -> HttpReply:
        reader, writer = await asyncio.wait_for(
            asyncio.open_connection(self._host, self._port), TIMEOUT
        )
        try:
            head = [
                f"{method} {path} HTTP/1.1",
                f"Host: {self._host}:{self._port}",
                f"Authorization: Bearer {self.token}",
                "User-Agent: board-copy-gaps-spike",
                "Connection: close",
            ]
            if body is not None:
                head += ["Content-Type: application/json", f"Content-Length: {len(body)}"]
            writer.write(("\r\n".join(head) + "\r\n\r\n").encode() + (body or b""))
            await writer.drain()
            raw = await asyncio.wait_for(reader.read(), TIMEOUT)
        finally:
            writer.close()
            await writer.wait_closed()
        return split_reply(raw)

    def _keep(self, record: RequestRecord) -> None:
        self.records.append(record)
        if self.log is not None:
            with self.log.open("a", encoding="utf-8") as out:
                out.write(json.dumps(record.to_json(), sort_keys=True) + "\n")

    def _record(
        self,
        kind: Kind,
        op: str,
        start: float,
        end: float,
        reply: HttpReply,
        cost: int | None,
        errors: tuple[str, ...],
        aliases: int,
    ) -> RequestRecord:
        return RequestRecord(
            seq=len(self.records) + 1,
            kind=kind,
            op=op,
            start=start,
            end=end,
            status=reply.status,
            cost=cost,
            errors=errors,
            limits=RateLimit.from_headers(reply.headers),
            aliases=aliases,
        )

    async def graphql(
        self, op: str, document: str, variables: dict[str, Json], aliases: int = 1
    ) -> Answer:
        """Send one GraphQL document; `op` names it in the log, `aliases` counts its fields."""
        kind = Kind.MUTATION if document.lstrip().startswith("mutation") else Kind.QUERY
        payload = json.dumps({"query": document, "variables": variables}).encode()
        start = time.time()
        reply = await self._send("POST", "/graphql", payload)
        end = time.time()
        # Every request is recorded before anything about its answer is refused, so a request
        # GitHub or the guard answered unusably still counts in what the run spent.
        try:
            if reply.status not in (200, 403):
                raise GitHubError(f"{op}: HTTP {reply.status}")
            body = obj(parse(reply.body.decode(errors="replace"), op), op)
            errors = tuple(
                graphql_error(e, op) for e in arr(body.get("errors", []), f"{op}.errors")
            )
            raw_data = body.get("data")
            if raw_data is None and not errors:
                raise GitHubError(f"{op}: an answer with neither data nor errors")
            data = {} if raw_data is None else obj(raw_data, f"{op}.data")
        except (GitHubError, ShapeError) as error:
            self._keep(
                self._record(kind, op, start, end, reply, None, (type(error).__name__,), aliases)
            )
            raise
        rate = data.get("rateLimit")
        cost = rate.get("cost") if isinstance(rate, dict) else None
        stated = cost if isinstance(cost, int) and not isinstance(cost, bool) else None
        record = self._record(
            kind, op, start, end, reply, stated, tuple(e.type for e in errors), aliases
        )
        self._keep(record)
        return Answer(data=data, errors=errors, record=record)

    async def rest_get(self, op: str, path: str) -> dict[str, Json]:
        """One REST `GET`, logged like a GraphQL request."""
        start = time.time()
        reply = await self._send("GET", path, None)
        record = RequestRecord(
            seq=len(self.records) + 1,
            kind=Kind.REST,
            op=op,
            start=start,
            end=time.time(),
            status=reply.status,
            cost=None,
            errors=(),
            limits=RateLimit.from_headers(reply.headers),
            aliases=1,
        )
        self._keep(record)
        if reply.status != 200:
            raise GitHubError(f"{op}: HTTP {reply.status}")
        return obj(parse(reply.body.decode(errors="replace"), op), op)


def split_reply(raw: bytes) -> HttpReply:
    """An HTTP/1.1 answer read to the end of its connection, its chunking undone."""
    head, sep, body = raw.partition(b"\r\n\r\n")
    if not sep:
        raise GitHubError("the proxy closed the connection without a complete answer")
    lines = head.decode("latin-1").split("\r\n")
    try:
        status = int(lines[0].split()[1])
    except (IndexError, ValueError) as error:
        raise GitHubError(f"an unreadable status line: {lines[0]!r}") from error
    headers: dict[str, str] = {}
    for line in lines[1:]:
        name, _, value = line.partition(":")
        headers[name.strip().lower()] = value.strip()
    if headers.get("transfer-encoding", "").lower() == "chunked":
        body = dechunk(body)
    elif "content-length" in headers:
        declared = headers["content-length"]
        if not declared.isdigit() or int(declared) != len(body):
            raise GitHubError(f"an answer declaring {declared} bytes carried {len(body)}")
    return HttpReply(status=status, headers=headers, body=body)


def dechunk(body: bytes) -> bytes:
    """A chunked body's payload, refusing framing that does not end in a terminating chunk."""
    out = b""
    while True:
        size_line, sep, body = body.partition(b"\r\n")
        size_text = size_line.split(b";")[0].strip()
        if not sep or not size_text or any(c not in b"0123456789abcdefABCDEF" for c in size_text):
            raise GitHubError("a chunked answer with unreadable framing")
        size = int(size_text, 16)
        if size == 0:
            # The last chunk is followed by optional trailers and one empty line.
            if not (body.startswith(b"\r\n") or b"\r\n\r\n" in body):
                raise GitHubError("a chunked answer without its final empty line")
            return out
        if len(body) < size + 2 or body[size : size + 2] != b"\r\n":
            raise GitHubError("a chunked answer cut short")
        out, body = out + body[:size], body[size + 2 :]


def graphql_error(value: Json, where: str) -> GraphQLError:
    """One `errors` entry, typed."""
    entry = obj(value, f"{where}.errors[]")
    path = entry.get("path", [])
    return GraphQLError(
        type=str(entry.get("type") or "untyped"),
        path=tuple(str(p) for p in arr(path, f"{where}.errors[].path")),
        message=text(entry.get("message") or "", f"{where}.errors[].message"),
    )


def require(answer: Answer, where: str) -> dict[str, Json]:
    """An answer's data, refusing one that carried any error."""
    if answer.errors:
        first = answer.errors[0]
        raise GitHubError(f"{where}: {first.type} at {'/'.join(first.path)}: {first.message}")
    return answer.data


__all__ = ["Answer", "Client", "GitHubError", "Kind", "RateLimit", "RequestRecord", "ShapeError"]
