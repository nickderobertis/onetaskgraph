"""A recording HTTP proxy in front of GitHub, for the board-copy-speed spike.

The GitHub Projects source is pointed at it through its `endpoint` override
(`http://127.0.0.1:<port>/graphql`). Every request it receives is forwarded with the
caller's own credential to `api.github.com`, or to `uploads.github.com` for an attachment
upload, over one kept-alive connection per thread, and one JSON line is appended to the raw
log for it. It also answers `CONNECT`, so a process given `HTTPS_PROXY` pointing here has
its direct HTTPS requests (an attachment's verification GET) tunnelled and logged by host.

The raw log holds whole request and response bodies, so it lives outside every tree (the
runner keeps it under the host cache directory); `sanitize.py` reduces it to the timings,
names, counts, costs and limit headers that are committed.

Run: `python3 proxy.py --log RAW.jsonl --port-file PORT [--upstream https://api.github.com]`.
"""

from __future__ import annotations

import argparse
import gzip
import http.client
import json
import re
import select
import socket
import threading
import time
import urllib.parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

HOP_BY_HOP = {
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailers",
    "transfer-encoding",
    "upgrade",
    "host",
    "content-length",
}

LIMIT_HEADERS = (
    "x-ratelimit-limit",
    "x-ratelimit-remaining",
    "x-ratelimit-used",
    "x-ratelimit-reset",
    "x-ratelimit-resource",
    "retry-after",
)

OPERATION = re.compile(r"^\s*(query|mutation)\s+([A-Za-z_][A-Za-z0-9_]*)")
ROOT_FIELD = re.compile(
    r"\{\s*(?:[A-Za-z_][A-Za-z0-9_]*\s*:\s*)?([A-Za-z_][A-Za-z0-9_]*)"
)
PRICE = "rateLimit{cost} "
DOCUMENTS = {
    text: name
    for name, text in json.loads(
        (Path(__file__).with_name("documents.json")).read_text()
    ).items()
}


def operation_of(document: str) -> tuple[str, str]:
    """The kind of a GraphQL document and its name: the name it declares, else the name of
    the plugin constant it was sent from (the plugin's documents are anonymous, and its
    ledger may have priced a query by inserting `rateLimit{cost}` after the first brace),
    else `<root field>?` naming its first root field."""
    stripped = document.lstrip()
    kind = "mutation" if stripped.startswith("mutation") else "query"
    match = OPERATION.search(document)
    if match:
        return match.group(1), match.group(2)
    unpriced = document.replace(PRICE, "", 1)
    if unpriced in DOCUMENTS:
        return kind, DOCUMENTS[unpriced]
    root = ROOT_FIELD.search(
        unpriced[unpriced.find(")") + 1 :]
        if "(" in unpriced.split("{", 1)[0]
        else unpriced
    )
    return kind, f"{root.group(1)}?" if root else "anonymous"


def reported_cost(body: bytes) -> int | None:
    """The `rateLimit { cost }` a GraphQL answer carries, when the query asked for it."""
    try:
        data = json.loads(body).get("data") or {}
    except (ValueError, AttributeError):
        return None
    if not isinstance(data, dict):
        return None
    for value in data.values():
        if isinstance(value, dict) and "cost" in value and "remaining" in value:
            cost = value.get("cost")
            return cost if isinstance(cost, int) else None
    return None


def error_types(body: bytes) -> list[str]:
    """The `type` of every GraphQL error, never its message (a message can name an item)."""
    try:
        errors = json.loads(body).get("errors") or []
    except (ValueError, AttributeError):
        return []
    return [str(e.get("type", "untyped")) for e in errors if isinstance(e, dict)]


GUARDED_KEYS = ("repositoryId", "projectId", "projectV2Ids")


def guarded_ids(value: object) -> list[str]:
    """Every repository and project id a request's variables name, at any depth."""
    found: list[str] = []
    if isinstance(value, dict):
        for key, inner in value.items():
            if key in GUARDED_KEYS:
                found += inner if isinstance(inner, list) else [inner]
            else:
                found += guarded_ids(inner)
    elif isinstance(value, list):
        for inner in value:
            found += guarded_ids(inner)
    return [str(v) for v in found]


def refusal(kind: str, body: bytes, writes: str, allowed: set[str]) -> str | None:
    """Why a request must not leave, or `None`: under `none` no mutation and no upload leaves;
    under `allowed` a mutation naming a repository or project not in `allowed` does not."""
    if writes == "any" or kind not in ("mutation", "upload"):
        return None
    if writes == "none":
        return "this run sends no writes"
    if kind == "upload":
        return None
    try:
        payload = json.loads(body)
    except ValueError:
        return "an unparseable mutation"
    variables = payload.get("variables") or {}
    query = payload.get("query", "")
    names = set(guarded_ids(variables))
    names |= set(re.findall(r'(?:repositoryId|projectId):\s*"([^"]+)"', query))
    # A guarded input field given a variable of any name: `projectId: $p`.
    for variable in re.findall(
        r"(?:repositoryId|projectId|projectV2Ids):\s*\$(\w+)", query
    ):
        value = variables.get(variable) if isinstance(variables, dict) else None
        names |= {str(v) for v in (value if isinstance(value, list) else [value])}
    stray = names - allowed
    return (
        f"it names {len(stray)} repository or project id(s) outside the test board"
        if stray
        else None
    )


def upstream_for(path: str, api: str, uploads: str) -> str:
    return uploads if path.startswith("/user-attachments/") else api


class Recorder:
    """Appends one JSON line per request and keeps each body pair beside the log."""

    def __init__(self, log: Path) -> None:
        self.log = log
        self.bodies = log.with_suffix(".bodies")
        self.bodies.mkdir(parents=True, exist_ok=True)
        self.lock = threading.Lock()
        self.seq = 0

    def next_seq(self) -> int:
        with self.lock:
            self.seq += 1
            return self.seq

    def write(
        self, record: dict, request: bytes | None, response: bytes | None
    ) -> None:
        seq = record["seq"]
        if request is not None:
            (self.bodies / f"{seq:06d}.req").write_bytes(request)
        if response is not None:
            (self.bodies / f"{seq:06d}.resp").write_bytes(response)
        with self.lock, self.log.open("a") as out:
            out.write(json.dumps(record, sort_keys=True) + "\n")


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    server_version = "spike-proxy"
    recorder: Recorder
    api: str
    uploads: str
    writes: str = "none"
    allowed: frozenset[str] = frozenset()
    refused_ops: frozenset[str] = frozenset()
    local = threading.local()

    def log_message(self, format: str, *args: object) -> None:
        return

    def connection_to(self, base: str) -> http.client.HTTPConnection:
        pool = getattr(self.local, "pool", None)
        if pool is None:
            pool = self.local.pool = {}
        if base not in pool:
            parts = urllib.parse.urlsplit(base)
            # Plain HTTP only for a loopback stand-in, which is what the harness's tests use.
            factory = (
                http.client.HTTPConnection
                if parts.scheme == "http"
                else http.client.HTTPSConnection
            )
            pool[base] = factory(parts.netloc, timeout=300)
        return pool[base]

    def forward(self) -> None:
        seq = self.recorder.next_seq()
        length = int(self.headers.get("content-length") or 0)
        body = self.rfile.read(length) if length else b""
        base = upstream_for(self.path, self.api, self.uploads)
        headers = {k: v for k, v in self.headers.items() if k.lower() not in HOP_BY_HOP}
        kind, name = ("rest", f"{self.command} {urllib.parse.urlsplit(self.path).path}")
        if self.path.startswith("/graphql"):
            try:
                kind, name = operation_of(json.loads(body).get("query", ""))
            except ValueError:
                kind, name = "query", "unparsed"
        if self.path.startswith("/user-attachments/") or (
            not self.path.startswith("/graphql") and self.command != "GET"
        ):
            kind = "upload"
        why = refusal(kind, body, self.writes, set(self.allowed))
        if name in self.refused_ops:
            why = f"this run sends no {name}"
        if why:
            self.refuse(seq, kind, name, body, why)
            return
        if kind == "upload":
            kind = "rest"
        start = time.time()
        response = None
        for attempt in (1, 2):
            connection = self.connection_to(base)
            try:
                connection.request(
                    self.command, self.path, body=body or None, headers=headers
                )
                response = connection.getresponse()
                payload = response.read()
                break
            except (http.client.HTTPException, OSError):
                connection.close()
                self.local.pool.pop(base, None)
                if attempt == 2:
                    raise
        end = time.time()
        assert response is not None
        decoded = payload
        if response.getheader("content-encoding", "") == "gzip":
            decoded = gzip.decompress(payload)
        record = {
            "seq": seq,
            "start": start,
            "end": end,
            "ms": round((end - start) * 1000, 1),
            "method": self.command,
            "host": urllib.parse.urlsplit(base).netloc,
            "path": self.path,
            "kind": kind,
            "op": name,
            "req_bytes": len(body),
            "resp_bytes": len(payload),
            "status": response.status,
            "cost": reported_cost(decoded) if kind != "rest" else None,
            "errors": error_types(decoded) if kind != "rest" else [],
            "limits": {
                h: response.getheader(h) for h in LIMIT_HEADERS if response.getheader(h)
            },
        }
        self.recorder.write(record, body, decoded)
        self.send_response(response.status)
        for key, value in response.getheaders():
            if key.lower() not in HOP_BY_HOP:
                self.send_header(key, value)
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def refuse(self, seq: int, kind: str, name: str, body: bytes, why: str) -> None:
        """Answer locally, as GitHub answers a refusal, and send nothing upstream."""
        payload = json.dumps(
            {"errors": [{"type": "SPIKE_REFUSED", "message": f"spike proxy: {why}"}]}
        ).encode()
        now = time.time()
        record = {
            "seq": seq,
            "start": now,
            "end": now,
            "ms": 0.0,
            "method": self.command,
            "host": "refused",
            "path": self.path,
            "kind": kind,
            "op": name,
            "req_bytes": len(body),
            "resp_bytes": len(payload),
            "status": 403,
            "cost": None,
            "errors": ["SPIKE_REFUSED"],
            "limits": {},
        }
        self.recorder.write(record, body, payload)
        self.send_response(403)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    do_GET = do_POST = do_PATCH = do_PUT = do_DELETE = forward

    def do_CONNECT(self) -> None:
        seq = self.recorder.next_seq()
        host, _, port = self.path.partition(":")
        if self.writes == "none":
            self.refuse(
                seq, "tunnel", f"CONNECT {host}", b"", "this run opens no tunnel"
            )
            return
        start = time.time()
        up = down = 0
        try:
            upstream = socket.create_connection((host, int(port or 443)), timeout=60)
        except OSError:
            self.send_error(502)
            return
        self.send_response(200, "Connection Established")
        self.end_headers()
        client = self.connection
        try:
            while True:
                ready, _, _ = select.select([client, upstream], [], [], 60)
                if not ready:
                    break
                for sock in ready:
                    chunk = sock.recv(65536)
                    if not chunk:
                        raise EOFError
                    if sock is client:
                        upstream.sendall(chunk)
                        up += len(chunk)
                    else:
                        client.sendall(chunk)
                        down += len(chunk)
        except (EOFError, OSError):
            pass
        finally:
            upstream.close()
        end = time.time()
        self.close_connection = True
        record = {
            "seq": seq,
            "start": start,
            "end": end,
            "ms": round((end - start) * 1000, 1),
            "method": "CONNECT",
            "host": host,
            "path": "",
            "kind": "tunnel",
            "op": f"CONNECT {host}",
            "req_bytes": up,
            "resp_bytes": down,
            "status": 200,
            "cost": None,
            "errors": [],
            "limits": {},
        }
        self.recorder.write(record, None, None)


def serve(
    log: Path,
    port_file: Path,
    api: str,
    uploads: str,
    port: int = 0,
    writes: str = "none",
    allowed: frozenset[str] = frozenset(),
    refused_ops: frozenset[str] = frozenset(),
) -> None:
    Handler.recorder = Recorder(log)
    Handler.api = api
    Handler.uploads = uploads
    Handler.writes = writes
    Handler.allowed = allowed
    Handler.refused_ops = refused_ops
    server = ThreadingHTTPServer(("127.0.0.1", port), Handler)
    server.daemon_threads = True
    port_file.write_text(str(server.server_address[1]))
    server.serve_forever()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--log", type=Path, required=True)
    parser.add_argument("--port-file", type=Path, required=True)
    parser.add_argument("--port", type=int, default=0)
    parser.add_argument("--upstream", default="https://api.github.com")
    parser.add_argument("--uploads", default="https://uploads.github.com")
    parser.add_argument(
        "--writes",
        choices=("any", "none", "allowed"),
        default="none",
        help="any: forward every write (stand-in upstreams only); none: refuse every mutation and upload; allowed: refuse a mutation naming a "
        "repository or project id not given with --allow",
    )
    parser.add_argument("--allow", action="append", default=[])
    parser.add_argument(
        "--refuse-op",
        action="append",
        default=[],
        help="an operation name never forwarded",
    )
    args = parser.parse_args()
    serve(
        args.log,
        args.port_file,
        args.upstream,
        args.uploads,
        args.port,
        args.writes,
        frozenset(args.allow),
        frozenset(args.refuse_op),
    )


if __name__ == "__main__":
    main()
