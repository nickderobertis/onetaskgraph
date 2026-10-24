"""The timing spike's recording proxy, run with a stricter write guard and no tunnels.

`python -m gaps.guard --log L --port-file P [--allow ID]... [--upstream URL --uploads URL]`

The proxy itself (`spike/harness/proxy.py`) is unchanged; this entry point serves a subclass of
its handler that screens every request first, and leaves the inherited guard in place behind
it. A mutation passes only when every repository or project id it names — as a literal, a
literal list, or a variable of any name — is one of the `--allow`ed ids; a request the guard
cannot read is refused. With no `--allow` every write is refused. Uploads and `CONNECT`
tunnels are always refused: none of this spike's requests uploads an asset, so it has no reason
to open an opaque channel the guard cannot read.
"""

from __future__ import annotations

import argparse
import io
import re
from http.server import ThreadingHTTPServer
from pathlib import Path

from gaps.harness import proxy
from gaps.json_shape import Json, ShapeError, obj, parse, text

GUARDED = re.compile(r"\b(repositoryId|projectId|projectV2Ids)\s*:\s*")
LITERAL = re.compile(r'"((?:[^"\\]|\\.)*)"')


def named_ids(query: str, variables: dict[str, Json]) -> set[str]:
    """Every repository or project id a mutation names, however it is written."""
    names: set[str] = set()
    for match in GUARDED.finditer(query):
        rest = query[match.end() :]
        if rest.startswith("$"):
            variable = re.match(r"\$(\w+)", rest)
            value = variables.get(variable.group(1)) if variable else None
            values = value if isinstance(value, list) else [value]
            names |= {str(v) for v in values}
        elif rest.startswith("["):
            names |= set(LITERAL.findall(rest[: rest.index("]") + 1]))
        elif rest.startswith('"'):
            literal = LITERAL.match(rest)
            names |= {literal.group(1)} if literal else {"<unreadable>"}
        else:
            names.add("<unreadable>")
    names |= guarded_values(variables)
    return names


def guarded_values(value: Json) -> set[str]:
    """Every value held under a guarded key, at any depth of a variables object."""
    found: set[str] = set()
    match value:
        case dict():
            for key, inner in value.items():
                if key in ("repositoryId", "projectId", "projectV2Ids"):
                    found |= {str(v) for v in (inner if isinstance(inner, list) else [inner])}
                else:
                    found |= guarded_values(inner)
        case list():
            for inner in value:
                found |= guarded_values(inner)
        case _:
            pass
    return found


def refusal(allowed: frozenset[str], path: str, method: str, body: bytes) -> str | None:
    """Why a request must not leave, or `None`.

    Refused: any write naming an id outside `allowed`, any request the guard cannot read, and
    any upload.
    """
    if not path.startswith("/graphql"):
        return None if method == "GET" else "this run uploads nothing"
    try:
        payload = obj(parse(body.decode(errors="replace"), "request"), "request")
        query = text(payload.get("query"), "request.query")
        variables = obj(payload.get("variables") or {}, "request.variables")
    except ShapeError:
        return "an unreadable request"
    if proxy.operation_of(query)[0] != "mutation":
        return None
    if not allowed:
        return "this run sends no writes"
    stray = named_ids(query, variables) - allowed
    if stray:
        return f"it names {len(stray)} repository or project id(s) outside the test board"
    return None


class GuardedHandler(proxy.Handler):
    """The timing spike's handler, every request screened by `refusal`, every tunnel refused."""

    guard_allows: frozenset[str] = frozenset()

    def forward(self) -> None:
        """Refuse a request the guard refuses; hand any other to the inherited handler."""
        length = int(self.headers.get("content-length") or 0)
        body = self.rfile.read(length) if length else b""
        why = refusal(self.guard_allows, self.path, self.command, body)
        if why:
            self.refuse(self.recorder.next_seq(), "mutation", "refused by the guard", body, why)
            return
        self.rfile = io.BytesIO(body)
        super().forward()

    do_GET = do_POST = do_PATCH = do_PUT = do_DELETE = forward

    def do_CONNECT(self) -> None:
        """Refuse every tunnel: none of this spike's requests uploads anything."""
        target = f"CONNECT {self.path.partition(':')[0]}"
        self.refuse(self.recorder.next_seq(), "tunnel", target, b"", "this run opens no tunnel")


def serve(log: Path, port_file: Path, api: str, uploads: str, allowed: frozenset[str]) -> None:
    """Serve the guarded proxy on a free loopback port, written to `port_file`."""
    GuardedHandler.recorder = proxy.Recorder(log)
    GuardedHandler.api = api
    GuardedHandler.uploads = uploads
    GuardedHandler.writes = "allowed"
    GuardedHandler.allowed = allowed
    GuardedHandler.guard_allows = allowed
    server = ThreadingHTTPServer(("127.0.0.1", 0), GuardedHandler)
    server.daemon_threads = True
    port_file.write_text(str(server.server_address[1]))
    server.serve_forever()


def main() -> None:
    """Start the guarded proxy."""
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--log", type=Path, required=True)
    parser.add_argument("--port-file", type=Path, required=True)
    parser.add_argument("--allow", action="append", default=[])
    parser.add_argument("--upstream", default="https://api.github.com")
    parser.add_argument("--uploads", default="https://uploads.github.com")
    args = parser.parse_args()
    serve(args.log, args.port_file, args.upstream, args.uploads, frozenset(args.allow))


if __name__ == "__main__":
    main()
